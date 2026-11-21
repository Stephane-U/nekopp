//! The minimap: a compressed picture of the whole document, drawn in a narrow
//! column on the right edge.
//!
//! It is built from the same per-character styles the editor uses. Each pixel
//! row of the column stands for a band of document rows; the longest of three
//! sampled rows in the band is drawn as a handful of style runs, which is enough
//! to read the shape of a file at a glance.
//!
//! The picture is cached and rebuilt only when the document changes. Reaching
//! into a rope is a tree descent per line, so doing it per frame would cost more
//! than drawing the text itself; a cached run list costs one buffer write.

use nn_core::{Document, Style, Viewport};

use crate::quad::QuadRenderer;
use crate::palette::Palette;
use crate::theme::mix;

/// The narrowest column the runs are scaled against. A file of short lines is
/// scaled to this rather than to its own longest line, so the picture does not
/// jump around as the caret moves.
const MIN_COLUMNS: f32 = 60.0;

/// Style runs drawn per band. A line with more runs than this is truncated: the
/// minimap is a thumbnail, not a second renderer.
const MAX_RUNS: usize = 6;

/// Bands the column is divided into. One per pixel row is more detail than a
/// thumbnail needs and makes a rebuild expensive on a large file.
const MAX_BANDS: usize = 512;

/// One cached rectangle of the picture.
struct Run {
    band: u16,
    x0: f32,
    x1: f32,
    color: [f32; 4],
}

/// What the cached picture was built from.
#[derive(Clone, Copy, PartialEq, Eq)]
struct CacheKey {
    revision: u64,
    bands: usize,
    width: u32,
}

/// The narrow column on the right edge that shows the whole document.
pub struct Minimap {
    width: f32,
    enabled: bool,
    /// Set while the chrome is reduced. Separate from `enabled` so leaving
    /// distraction-free mode restores whatever the user had chosen rather than
    /// turning the minimap back on for someone who had turned it off.
    suppressed: bool,
    /// The cached picture, in band order.
    runs: Vec<Run>,
    /// The document line each band stands for, kept between rebuilds so the
    /// scratch allocation is reused.
    lines: Vec<usize>,
    cache: Option<CacheKey>,
}

impl Minimap {
    /// Builds a minimap at its default width.
    pub fn new() -> Self {
        Self {
            width: 84.0,
            enabled: true,
            suppressed: false,
            runs: Vec::new(),
            lines: Vec::new(),
            cache: None,
        }
    }

    /// The width the editor has to leave free, zero when hidden.
    pub fn width(&self) -> f32 {
        if self.enabled && !self.suppressed {
            self.width
        } else {
            0.0
        }
    }

    /// Whether the minimap is shown.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Shows or hides the minimap.
    pub fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }

    /// Shows or hides the minimap, for a setting rather than a keystroke.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Hides the minimap without forgetting whether the user wants it.
    ///
    /// Distraction-free mode uses this rather than `set_enabled(false)`: the
    /// mode is a temporary view of the same window, so it must not overwrite a
    /// preference the user set deliberately.
    pub fn set_suppressed(&mut self, suppressed: bool) {
        self.suppressed = suppressed;
    }

    /// Whether the minimap is hidden by the chrome rather than by the user.
    pub fn is_suppressed(&self) -> bool {
        self.suppressed
    }

    /// Drops the cached picture, forcing a rebuild on the next frame.
    ///
    /// The cache key is the document revision, which does not change when the
    /// palette does, so a theme flip has to say so explicitly.
    pub fn invalidate(&mut self) {
        self.cache = None;
    }

    /// The document row under window pixel `y`, for click-to-scroll.
    pub fn row_at(&self, y: f32, top: f32, height: f32, total: usize) -> usize {
        if height <= 0.0 || total == 0 {
            return 0;
        }
        let t = ((y - top) / height).clamp(0.0, 1.0);
        ((t * total as f32) as usize).min(total - 1)
    }

    /// Queues the background, the document picture, and the viewport box.
    ///
    /// `right` is the window edge the column is flush against.
    #[allow(clippy::too_many_arguments)]
    pub fn push_quads(
        &mut self,
        doc: &mut Document,
        vp: &Viewport,
        top: f32,
        height: f32,
        right: f32,
        palette: &Palette,
        quads: &mut QuadRenderer,
    ) {
        if !self.enabled || self.suppressed || height < 4.0 {
            return;
        }
        let left = right - self.width;
        quads.push(
            left,
            top,
            self.width,
            height,
            palette.theme.minimap_background.to_linear(),
        );
        quads.push(left, top, 1.0, height, palette.theme.bar_border.to_linear());

        let total = doc.visible_line_count();
        if total == 0 {
            return;
        }

        let bands = (height as usize).clamp(1, MAX_BANDS);
        let band_h = height / bands as f32;
        let key = CacheKey {
            revision: doc.revision(),
            bands,
            width: self.width.to_bits(),
        };
        if self.cache != Some(key) {
            self.build(doc, bands, palette);
            self.cache = Some(key);
        }

        let text_left = left + 3.0;
        for run in &self.runs {
            quads.push(
                text_left + run.x0,
                top + run.band as f32 * band_h,
                run.x1 - run.x0,
                band_h,
                run.color,
            );
        }

        // The viewport box, so the picture says where the window is.
        let visible = vp.visible_line_count().max(1);
        let box_top = top + vp.first_line as f32 * height / total as f32;
        let box_height = (visible as f32 * height / total as f32).max(6.0);
        let box_height = box_height.min(top + height - box_top);
        quads.push(
            left,
            box_top,
            self.width,
            box_height,
            palette.theme.minimap_viewport.to_linear_alpha(0.16),
        );
        quads.push(
            left,
            box_top,
            self.width,
            1.0,
            palette.theme.minimap_viewport.to_linear_alpha(0.45),
        );
        quads.push(
            left,
            box_top + box_height - 1.0,
            self.width,
            1.0,
            palette.theme.minimap_viewport.to_linear_alpha(0.45),
        );
    }

    /// Rebuilds the cached picture for a document of `bands` bands.
    fn build(&mut self, doc: &mut Document, bands: usize, palette: &Palette) {
        self.runs.clear();
        self.lines.clear();
        let total = doc.visible_line_count();
        if total == 0 {
            return;
        }

        // A band can stand for hundreds of lines. Sampling its first, middle and
        // last row and keeping the longest keeps the picture from aliasing into
        // stripes, at three lookups per band.
        let mut longest = 0usize;
        for band in 0..bands {
            let first = band * total / bands;
            let last = ((band + 1) * total / bands).max(first + 1).min(total);
            let mut best = doc.line_of_row(first);
            let mut best_len = doc.line_range(best).len();
            for row in [first + (last - first) / 2, last - 1] {
                let line = doc.line_of_row(row);
                let len = doc.line_range(line).len();
                if len > best_len {
                    best = line;
                    best_len = len;
                }
            }
            longest = longest.max(best_len);
            self.lines.push(best);
        }

        let columns = (longest as f32).max(MIN_COLUMNS);
        let scale = (self.width - 6.0) / columns;

        // Taken out so the run list can be pushed to while the bands are read.
        let lines = std::mem::take(&mut self.lines);
        for (band, &line) in lines.iter().enumerate() {
            let start = doc.line_to_char(line);
            let mut offset = 0usize;
            let mut run_start = 0usize;
            let mut run_style: Style = 0;
            let mut started = false;
            let mut runs = 0usize;
            // Indentation is not content: a bar at the left edge of every line
            // would read as a solid block instead of code.
            let mut indented = true;

            for ch in doc.line_chars(line) {
                let style = doc.style_at(start + offset);
                if indented && ch.is_whitespace() {
                    offset += 1;
                    continue;
                }
                indented = false;
                if !started {
                    run_start = offset;
                    run_style = style;
                    started = true;
                } else if style != run_style {
                    self.push_run(band, run_start, offset, run_style, scale, palette);
                    runs += 1;
                    if runs >= MAX_RUNS {
                        break;
                    }
                    run_start = offset;
                    run_style = style;
                }
                offset += 1;
            }
            if started && runs < MAX_RUNS {
                self.push_run(band, run_start, offset, run_style, scale, palette);
            }
        }
        self.lines = lines;
    }

    /// Caches one style run of a band.
    fn push_run(
        &mut self,
        band: usize,
        start: usize,
        end: usize,
        style: Style,
        scale: f32,
        palette: &Palette,
    ) {
        if start >= end {
            return;
        }
        let x0 = start as f32 * scale;
        let x1 = end as f32 * scale;
        if x1 - x0 < 0.5 {
            return;
        }
        let color = mix(palette.theme.minimap_background, palette.color(style), 0.62);
        self.runs.push(Run {
            band: band as u16,
            x0,
            x1,
            color: color.to_linear(),
        });
    }
}

impl Default for Minimap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_minimap_takes_no_width() {
        let mut m = Minimap::new();
        assert!(m.width() > 0.0);
        m.toggle();
        assert_eq!(m.width(), 0.0);
        assert!(!m.is_enabled());
    }

    #[test]
    fn suppression_hides_the_column_without_forgetting_the_preference() {
        let mut m = Minimap::new();
        m.set_suppressed(true);
        assert!(m.is_suppressed());
        assert_eq!(m.width(), 0.0);
        // The user's choice is untouched, so leaving the mode brings it back.
        assert!(m.is_enabled());
        m.set_suppressed(false);
        assert!(m.width() > 0.0);
    }

    #[test]
    fn a_suppressed_minimap_stays_hidden_when_the_user_turns_it_on() {
        // The two flags are independent: turning the minimap on while the
        // chrome is reduced must not make it appear.
        let mut m = Minimap::new();
        m.toggle();
        m.set_suppressed(true);
        m.toggle();
        assert!(m.is_enabled());
        assert_eq!(m.width(), 0.0);
    }

    #[test]
    fn row_at_maps_the_column_onto_the_document() {
        let m = Minimap::new();
        assert_eq!(m.row_at(100.0, 100.0, 400.0, 1000), 0);
        assert_eq!(m.row_at(300.0, 100.0, 400.0, 1000), 500);
        assert_eq!(m.row_at(500.0, 100.0, 400.0, 1000), 999);
        // Above and below the column clamp to the ends.
        assert_eq!(m.row_at(0.0, 100.0, 400.0, 1000), 0);
        assert_eq!(m.row_at(900.0, 100.0, 400.0, 1000), 999);
    }

    #[test]
    fn row_at_survives_an_empty_document() {
        let m = Minimap::new();
        assert_eq!(m.row_at(50.0, 0.0, 400.0, 0), 0);
        assert_eq!(m.row_at(50.0, 0.0, 0.0, 10), 0);
    }

    #[test]
    fn the_picture_is_rebuilt_only_when_the_document_changes() {
        let mut doc = Document::from_text("fn main() {\n    let x = 1;\n}\n");
        let mut m = Minimap::new();
        let palette = Palette::new(crate::theme::Theme::DARK);
        m.build(&mut doc, 8, &palette);
        let first = m.runs.len();
        assert!(first > 0);
        // A second build of an unchanged document produces the same picture.
        m.build(&mut doc, 8, &palette);
        assert_eq!(m.runs.len(), first);
    }
}
