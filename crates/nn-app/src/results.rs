//! The find-in-files results panel.
//!
//! A docked list along the bottom edge, above the status bar, the way
//! Notepad++'s "Find result" window sits. It shows one row per matching line,
//! `path:line: text`, and clicking a row opens that file at that line.
//!
//! The panel is a fixed number of rows tall rather than resizable: a drag handle
//! is a whole interaction to build and test, and eight rows is enough to see
//! whether the search found the right thing. `PageUp`/`PageDown` scroll it.
//!
//! Shaping is the expensive part, so the row buffers are rebuilt only when the
//! hit list, the scroll offset, or the panel width changes. A frame where
//! nothing moved costs one rectangle and no text work.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use glyphon::{TextArea, TextBounds};
use nn_core::Hit;

use crate::quad::QuadRenderer;
use crate::theme::Theme;

/// How many rows the panel shows at once.
const VISIBLE_ROWS: usize = 8;

/// The docked results list.
pub struct ResultsPanel {
    hits: Vec<Hit>,
    /// Index of the first visible hit.
    scroll: usize,
    open: bool,
    /// One shaped buffer per visible row.
    rows: Vec<Buffer>,
    /// The text currently shaped into each row, so a reshape is skipped when
    /// nothing changed.
    shaped: Vec<String>,
    /// What the rows were shaped for: (hit count, scroll, width in chars).
    shaped_key: (usize, usize, usize),
    header: Buffer,
    header_text: String,
    header_shaped: String,
    metrics: Metrics,
    family: Family<'static>,
    char_width: f32,
}

impl ResultsPanel {
    /// Builds a closed panel.
    pub fn new(metrics: Metrics, family: Family<'static>, char_width: f32) -> Self {
        let mut header = Buffer::new_empty(metrics);
        header.set_size(None, None);
        header.set_text("", &Attrs::new().family(family), Shaping::Basic, None);
        Self {
            hits: Vec::new(),
            scroll: 0,
            open: false,
            rows: Vec::new(),
            shaped: Vec::new(),
            shaped_key: (usize::MAX, usize::MAX, usize::MAX),
            header,
            header_text: String::new(),
            header_shaped: String::new(),
            metrics,
            family,
            char_width,
        }
    }

    /// Replaces the hit list and opens the panel.
    pub fn set_hits(&mut self, hits: Vec<Hit>) {
        let files = self.count_files(&hits);
        let noun = if files == 1 { "file" } else { "files" };
        self.header_text = match hits.len() {
            0 => "No results".to_string(),
            1 => format!("1 result in {files} {noun}"),
            n => format!("{n} results in {files} {noun}"),
        };
        self.hits = hits;
        self.scroll = 0;
        self.open = true;
        self.shaped_key = (usize::MAX, usize::MAX, usize::MAX);
    }

    fn count_files(&self, hits: &[Hit]) -> usize {
        let mut paths: Vec<&std::path::Path> = hits.iter().map(|h| h.path.as_path()).collect();
        paths.sort();
        paths.dedup();
        paths.len()
    }

    /// Closes the panel and drops the results.
    pub fn close(&mut self) {
        self.open = false;
        self.hits.clear();
        self.scroll = 0;
        self.rows.clear();
        self.shaped.clear();
        self.shaped_key = (usize::MAX, usize::MAX, usize::MAX);
    }

    /// Whether the panel is showing.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The hits, for the click handler.
    pub fn hits(&self) -> &[Hit] {
        &self.hits
    }

    /// The height the panel occupies.
    pub fn height(&self, line_height: f32) -> f32 {
        if !self.open {
            return 0.0;
        }
        let rows = self.hits.len().min(VISIBLE_ROWS) as f32;
        line_height * (rows + 1.0) + 6.0
    }

    /// Scrolls the list by `delta` rows.
    pub fn scroll_by(&mut self, delta: isize) {
        let max = self.hits.len().saturating_sub(VISIBLE_ROWS);
        let next = (self.scroll as isize + delta).clamp(0, max as isize);
        self.scroll = next as usize;
    }

    /// The hit index under `y`, where `top` is the panel's top edge.
    pub fn hit_at(&self, y: f32, top: f32, line_height: f32) -> Option<usize> {
        if !self.open {
            return None;
        }
        let first = top + line_height + 3.0;
        if y < first {
            return None;
        }
        let row = ((y - first) / line_height) as usize;
        if row >= VISIBLE_ROWS {
            return None;
        }
        let index = self.scroll + row;
        (index < self.hits.len()).then_some(index)
    }

    /// Rebuilds the shaped rows if anything they depend on changed.
    pub fn prepare(&mut self, font_system: &mut FontSystem, width: f32) {
        if !self.open {
            return;
        }
        let columns = ((width - 16.0) / self.char_width).max(8.0) as usize;
        let key = (self.hits.len(), self.scroll, columns);
        if key == self.shaped_key {
            return;
        }
        self.shaped_key = key;

        if self.header_text != self.header_shaped {
            self.header.set_text(
                &self.header_text,
                &Attrs::new().family(self.family),
                Shaping::Basic,
                None,
            );
            self.header.shape_until_scroll(font_system, false);
            self.header_shaped.clone_from(&self.header_text);
        }

        let visible = self.hits.len().min(VISIBLE_ROWS);
        self.rows.resize_with(visible, || {
            let mut buffer = Buffer::new_empty(self.metrics);
            buffer.set_size(None, None);
            buffer
        });
        self.shaped.resize(visible, String::new());
        for row in 0..visible {
            let text = self.row_text(self.scroll + row, columns);
            if self.shaped[row] == text {
                continue;
            }
            self.rows[row].set_text(
                &text,
                &Attrs::new().family(self.family),
                Shaping::Basic,
                None,
            );
            self.rows[row].shape_until_scroll(font_system, false);
            self.shaped[row].clone_from(&text);
        }
    }

    /// The display text of hit `index`, truncated to `columns`.
    fn row_text(&self, index: usize, columns: usize) -> String {
        let Some(hit) = self.hits.get(index) else {
            return String::new();
        };
        let name = hit
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");
        let text = format!("{}:{}: {}", name, hit.line + 1, hit.text.trim());
        truncate(&text, columns)
    }

    /// Queues the panel background, its top border, and the row highlight.
    pub fn push_quads(
        &self,
        quads: &mut QuadRenderer,
        width: f32,
        top: f32,
        line_height: f32,
        theme: &Theme,
    ) {
        if !self.open {
            return;
        }
        let height = self.height(line_height);
        quads.push(
            0.0,
            top,
            width,
            height,
            theme.bar_background.to_linear(),
        );
        quads.push(0.0, top, width, 1.0, theme.bar_border.to_linear());
    }

    /// The text areas of the header and the visible rows.
    pub fn text_areas<'a>(
        &'a self,
        width: f32,
        top: f32,
        line_height: f32,
        padding: f32,
        theme: &Theme,
    ) -> Vec<TextArea<'a>> {
        if !self.open {
            return Vec::new();
        }
        let height = self.height(line_height);
        let bounds = TextBounds {
            left: 0,
            top: top as i32,
            right: width as i32,
            bottom: (top + height) as i32,
        };
        let mut areas = Vec::with_capacity(self.rows.len() + 1);
        areas.push(TextArea {
            buffer: &self.header,
            left: padding,
            top: top + 3.0,
            scale: 1.0,
            bounds,
            default_color: theme.bar_label.to_cosmic(),
            custom_glyphs: &[],
        });
        for (row, buffer) in self.rows.iter().enumerate() {
            areas.push(TextArea {
                buffer,
                left: padding,
                top: top + line_height + 3.0 + row as f32 * line_height,
                scale: 1.0,
                bounds,
                default_color: theme.foreground.to_cosmic(),
                custom_glyphs: &[],
            });
        }
        areas
    }

    /// The metrics the panel was built with.
    pub fn metrics(&self) -> Metrics {
        self.metrics
    }
}

/// Cuts `text` to `columns` characters, marking the cut with an ellipsis.
fn truncate(text: &str, columns: usize) -> String {
    if text.chars().count() <= columns {
        return text.to_string();
    }
    let mut out: String = text.chars().take(columns.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn panel() -> ResultsPanel {
        ResultsPanel::new(Metrics::new(15.0, 20.0), Family::Monospace, 8.0)
    }

    fn hit(line: usize) -> Hit {
        Hit {
            path: PathBuf::from("/tmp/a.rs"),
            line,
            column: 0,
            text: format!("line {line}"),
        }
    }

    #[test]
    fn a_closed_panel_takes_no_space() {
        assert_eq!(panel().height(20.0), 0.0);
    }

    #[test]
    fn an_open_panel_is_a_header_plus_rows() {
        let mut p = panel();
        p.set_hits(vec![hit(0), hit(1)]);
        assert_eq!(p.height(20.0), 20.0 * 3.0 + 6.0);
    }

    #[test]
    fn the_panel_caps_at_the_visible_row_count() {
        let mut p = panel();
        p.set_hits((0..100).map(hit).collect());
        assert_eq!(p.height(20.0), 20.0 * (VISIBLE_ROWS as f32 + 1.0) + 6.0);
    }

    #[test]
    fn the_header_counts_results_and_files() {
        let mut p = panel();
        p.set_hits(vec![hit(0), hit(1)]);
        assert_eq!(p.header_text, "2 results in 1 file");
    }

    #[test]
    fn an_empty_result_set_says_so() {
        let mut p = panel();
        p.set_hits(Vec::new());
        assert_eq!(p.header_text, "No results");
        assert!(p.is_open());
    }

    #[test]
    fn scrolling_clamps_to_the_list() {
        let mut p = panel();
        p.set_hits((0..20).map(hit).collect());
        p.scroll_by(-5);
        assert_eq!(p.scroll, 0);
        p.scroll_by(100);
        assert_eq!(p.scroll, 20 - VISIBLE_ROWS);
    }

    #[test]
    fn hit_at_maps_a_row_to_a_hit() {
        let mut p = panel();
        p.set_hits((0..20).map(hit).collect());
        assert_eq!(p.hit_at(23.0 + 5.0, 0.0, 20.0), Some(0));
        assert_eq!(p.hit_at(23.0 + 20.0 * 2.0, 0.0, 20.0), Some(2));
        assert_eq!(p.hit_at(5.0, 0.0, 20.0), None);
    }

    #[test]
    fn hit_at_follows_the_scroll() {
        let mut p = panel();
        p.set_hits((0..20).map(hit).collect());
        p.scroll_by(4);
        assert_eq!(p.hit_at(23.0, 0.0, 20.0), Some(4));
    }

    #[test]
    fn hit_at_past_the_end_is_none() {
        let mut p = panel();
        p.set_hits(vec![hit(0)]);
        assert_eq!(p.hit_at(23.0 + 20.0 * 3.0, 0.0, 20.0), None);
    }

    #[test]
    fn closing_drops_the_results() {
        let mut p = panel();
        p.set_hits(vec![hit(0)]);
        p.close();
        assert!(!p.is_open());
        assert!(p.hits().is_empty());
        assert_eq!(p.height(20.0), 0.0);
    }

    #[test]
    fn truncate_marks_the_cut() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("abc", 4), "abc");
    }

    #[test]
    fn a_row_shows_the_file_name_and_line_number() {
        let mut p = panel();
        p.set_hits(vec![hit(4)]);
        assert_eq!(p.row_text(0, 80), "a.rs:5: line 4");
    }
}
