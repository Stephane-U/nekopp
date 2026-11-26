// Neko++ -- a GPU-rendered text editor.
// Copyright (C) 2026 Stéphane Urdy
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU General Public License as published by the Free
// Software Foundation, version 3 of the License.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or
// FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public License for
// more details.
//
// You should have received a copy of the GNU General Public License along with
// this program. If not, see <https://www.gnu.org/licenses/>.

//! The Style Configurator.
//!
//! Notepad++'s version is a two-pane dialog: a language list on the left, a style
//! list on the right, and a font and colour picker under both. This is the part
//! that matters — one row per colour the editor draws with, and a way to change
//! it — without the language dimension, because this editor has one lexer.
//!
//! Colours are chosen from a fixed palette rather than a full picker. A picker
//! means a hue wheel or three sliders and a hex field, which is a dialog of its
//! own; a palette is one row of swatches and covers every colour a syntax theme
//! actually uses. The palette is the one thing here that is a deliberate subset,
//! and it is written down rather than hidden.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use glyphon::{TextArea, TextBounds};

use crate::quad::QuadRenderer;
use crate::theme::{Rgb, Theme, ToGpu};

/// The width of the dialog, in pixels.
const WIDTH: f32 = 460.0;
/// Padding inside the frame.
const PADDING: f32 = 14.0;
/// The column the swatch starts at, in characters.
const SWATCH_COLUMN: usize = 16;
/// The column the hex value starts at, in characters. Far enough right that the
/// swatch sits in the gap rather than under the digits.
const VALUE_COLUMN: usize = 22;
/// The width of a swatch, in pixels.
const SWATCH_WIDTH: f32 = 34.0;

/// The colours the configurator offers.
///
/// Twenty-four, chosen to cover what a syntax theme needs: neutrals for text and
/// backgrounds, and a saturated and a muted version of each hue for keywords,
/// strings, and comments.
pub const PALETTE: [Rgb; 24] = [
    Rgb::new(0, 0, 0),
    Rgb::new(30, 30, 30),
    Rgb::new(60, 60, 60),
    Rgb::new(100, 100, 100),
    Rgb::new(140, 140, 140),
    Rgb::new(180, 180, 180),
    Rgb::new(212, 212, 212),
    Rgb::new(255, 255, 255),
    Rgb::new(163, 21, 21),
    Rgb::new(206, 145, 120),
    Rgb::new(214, 120, 40),
    Rgb::new(220, 180, 60),
    Rgb::new(181, 206, 168),
    Rgb::new(106, 153, 85),
    Rgb::new(0, 128, 0),
    Rgb::new(78, 201, 176),
    Rgb::new(38, 127, 110),
    Rgb::new(86, 156, 214),
    Rgb::new(0, 0, 200),
    Rgb::new(38, 79, 120),
    Rgb::new(197, 134, 192),
    Rgb::new(128, 0, 128),
    Rgb::new(220, 220, 170),
    Rgb::new(255, 240, 160),
];

// `Slot`, the record of overrides and the theme they build moved to `nn-shared`
// in the hundred-and-seventeenth pass: which colours a user may change, what each
// is called and how an override is written into a theme are the same questions in
// a terminal, and the answer is the same `Theme`. What is left in this file is the
// dialog -- glyphs, a swatch grid, a caret -- which is the window's alone.
pub use nn_shared::theme::{effective_theme, record, Slot};

/// The modal style list.
pub struct StyleConfigurator {
    open: bool,
    selected: usize,
    /// The theme being edited, so the dialog can show live colours.
    theme: Theme,
    title: Buffer,
    title_shaped: String,
    body: Buffer,
    body_shaped: String,
    family: Family<'static>,
    /// The metrics the widget was last built with, so a density change can
    /// be told from the size already in hand.
    metrics: Metrics,
    char_width: f32,
    /// Window size, so the dialog can centre itself.
    viewport: (f32, f32),
}

impl StyleConfigurator {
    /// Builds a closed dialog.
    pub fn new(metrics: Metrics, family: Family<'static>, char_width: f32) -> Self {
        Self {
            open: false,
            selected: 0,
            theme: Theme::DARK,
            title: Buffer::new_empty(metrics),
            title_shaped: String::new(),
            body: Buffer::new_empty(metrics),
            body_shaped: String::new(),
            family,
            metrics,
            char_width,
            viewport: (0.0, 0.0),
        }
    }

    /// Rebuilds the dialog at a new display density.
    pub fn set_metrics(&mut self, metrics: Metrics) {
        if metrics == self.metrics {
            return;
        }
        // Lengths counted in characters -- a row's column, the box around a
        // label, the width a tab wants -- are measured with `char_width`, so the
        // width grows with the text instead of staying at the size the window
        // opened with.
        self.char_width *= metrics.line_height / self.metrics.line_height;
        self.metrics = metrics;
        self.title.set_metrics(metrics);
        self.body.set_metrics(metrics);
    }

    /// The line height the widget was built with.
    ///
    /// Read from the metrics rather than kept beside them: a second copy is what
    /// left the rows and their highlight at the size the window opened at while
    /// the text around them grew.
    fn line_height(&self) -> f32 {
        self.metrics.line_height
    }

    /// Opens the dialog on a copy of `theme`.
    pub fn open(&mut self, theme: Theme) {
        self.theme = theme;
        self.selected = 0;
        self.open = true;
    }

    /// Closes the dialog.
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Whether the dialog is showing.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The theme as edited so far.
    pub fn theme(&self) -> Theme {
        self.theme
    }

    /// The highlighted row.
    pub fn selected(&self) -> Slot {
        Slot::ALL[self.selected.min(Slot::ALL.len() - 1)]
    }

    /// Records the window size, so the dialog can centre itself.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.viewport = (width, height);
    }

    /// Moves the highlight by `delta`, clamping at both ends.
    pub fn move_selection(&mut self, delta: isize) {
        let last = Slot::ALL.len() as isize - 1;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
    }

    /// Highlights `index`, for a click.
    pub fn set_selected(&mut self, index: usize) {
        if index < Slot::ALL.len() {
            self.selected = index;
        }
    }

    /// Steps the highlighted slot's colour through the palette.
    ///
    /// Returns whether anything changed, so the caller can skip a redraw and a
    /// disk write when the colour is already at the end of the palette.
    pub fn adjust(&mut self, delta: i32) -> bool {
        let slot = self.selected();
        let current = slot.get(&self.theme);
        let at = PALETTE.iter().position(|&c| c == current);
        let next = match at {
            Some(at) => {
                let last = PALETTE.len() as isize - 1;
                (at as isize + delta as isize).clamp(0, last) as usize
            }
            // A colour that is not in the palette — the theme's own default —
            // steps to the nearest end rather than jumping to the middle.
            None => {
                if delta >= 0 {
                    0
                } else {
                    PALETTE.len() - 1
                }
            }
        };
        if PALETTE[next] == current {
            return false;
        }
        slot.set(&mut self.theme, PALETTE[next]);
        true
    }

    /// The value drawn on the right of `slot`.
    pub fn value(&self, slot: Slot) -> String {
        let color = slot.get(&self.theme);
        format!("#{:02X}{:02X}{:02X}", color.0, color.1, color.2)
    }

    /// The width the dialog occupies.
    ///
    /// `WIDTH` is the width at the plain density; a display that asks for twice
    /// the text asks for twice the box, or the swatches and the hex values run
    /// past its right edge.
    pub fn width(&self) -> f32 {
        WIDTH * crate::renderer::scale_of(self.metrics)
    }

    /// The height the dialog occupies.
    pub fn height(&self) -> f32 {
        PADDING * 2.0 + (Slot::ALL.len() as f32 + 4.0) * self.line_height()
    }

    /// The top-left corner, centred in the window.
    fn origin(&self) -> (f32, f32) {
        let (width, height) = self.viewport;
        let left = ((width - self.width()) / 2.0).max(0.0);
        let top = ((height - self.height()) / 2.0).max(0.0);
        (left, top)
    }

    /// The row index under `(x, y)` in window pixels.
    pub fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        if !self.open {
            return None;
        }
        let (left, top) = self.origin();
        if x < left || x > left + self.width() {
            return None;
        }
        let rows_top = top + PADDING + self.line_height() * 2.0;
        if y < rows_top {
            return None;
        }
        let row = ((y - rows_top) / self.line_height()) as usize;
        (row < Slot::ALL.len()).then_some(row)
    }

    /// Shapes the title and the body if they changed.
    pub fn prepare(&mut self, font_system: &mut FontSystem) {
        if !self.open {
            return;
        }
        if self.title_shaped != self.title_text() {
            self.title.set_text(
                self.title_text(),
                &Attrs::new().family(self.family),
                Shaping::Basic,
                None,
            );
            self.title.shape_until_scroll(font_system, false);
            self.title_shaped = self.title_text().to_string();
        }

        let mut text = String::new();
        for (index, slot) in Slot::ALL.iter().enumerate() {
            if index > 0 {
                text.push('\n');
            }
            let label = slot.label();
            text.push_str(label);
            for _ in label.chars().count()..VALUE_COLUMN {
                text.push(' ');
            }
            text.push_str(&self.value(*slot));
        }
        text.push_str("\n\n");
        text.push_str(self.hint());
        if text == self.body_shaped {
            return;
        }
        self.body.set_text(
            &text,
            &Attrs::new().family(self.family),
            Shaping::Basic,
            None,
        );
        self.body.shape_until_scroll(font_system, false);
        self.body_shaped = text;
    }

    /// Queues the frame, the selected row, and a swatch per row.
    pub fn push_quads(&self, quads: &mut QuadRenderer, theme: &Theme) {
        if !self.open {
            return;
        }
        let (left, top) = self.origin();
        let width = self.width();
        let height = self.height();

        quads.push(left, top, width, height, theme.menu_background.to_linear());
        let border = theme.menu_border.to_linear();
        quads.push(left, top, width, 1.0, border);
        quads.push(left, top + height - 1.0, width, 1.0, border);
        quads.push(left, top, 1.0, height, border);
        quads.push(left + width - 1.0, top, 1.0, height, border);

        let rows_top = top + PADDING + self.line_height() * 2.0;
        quads.push(
            left + 1.0,
            rows_top + self.selected as f32 * self.line_height(),
            width - 2.0,
            self.line_height(),
            theme.menu_hover.to_linear(),
        );

        // A swatch of the colour itself, left of the hex value. The hex is what
        // you copy; the swatch is what you read.
        let swatch_left = left + PADDING + SWATCH_COLUMN as f32 * self.char_width();
        for (index, slot) in Slot::ALL.iter().enumerate() {
            let y = rows_top + index as f32 * self.line_height() + 3.0;
            quads.push(
                swatch_left,
                y,
                SWATCH_WIDTH,
                self.line_height() - 6.0,
                slot.get(&self.theme).to_linear(),
            );
        }
    }

    /// The text areas: the title, the rows, and the hint.
    pub fn text_areas<'a>(&'a self, theme: &Theme) -> Vec<TextArea<'a>> {
        if !self.open {
            return Vec::new();
        }
        let (left, top) = self.origin();
        let bounds = TextBounds {
            left: left as i32,
            top: top as i32,
            right: (left + self.width()) as i32,
            bottom: (top + self.height()) as i32,
        };
        vec![
            TextArea {
                buffer: &self.title,
                left: left + PADDING,
                top: top + PADDING,
                scale: 1.0,
                bounds,
                default_color: theme.bar_label.to_cosmic(),
                custom_glyphs: &[],
            },
            TextArea {
                buffer: &self.body,
                left: left + PADDING,
                top: top + PADDING + self.line_height() * 2.0,
                scale: 1.0,
                bounds,
                default_color: theme.foreground.to_cosmic(),
                custom_glyphs: &[],
            },
        ]
    }

    /// The title, drawn separately so it can use the label colour.
    pub fn title_text(&self) -> &'static str {
        "Style Configurator"
    }

    /// The hint line under the rows.
    pub fn hint(&self) -> &'static str {
        "Up/Down select   Left/Right change   Esc close"
    }

    /// The character width the dialog was built with.
    pub fn char_width(&self) -> f32 {
        self.char_width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog() -> StyleConfigurator {
        StyleConfigurator::new(Metrics::new(15.0, 20.0), Family::Monospace, 8.0)
    }

    #[test]
    fn a_new_dialog_is_closed() {
        assert!(!dialog().is_open());
    }

    #[test]
    fn opening_copies_the_theme() {
        let mut d = dialog();
        d.open(Theme::LIGHT);
        assert!(d.is_open());
        assert_eq!(d.theme().background, Theme::LIGHT.background);
    }

    #[test]
    fn the_first_row_is_the_default_text_colour() {
        assert_eq!(dialog().selected(), Slot::Default);
    }

    #[test]
    fn selection_clamps_at_both_ends() {
        let mut d = dialog();
        d.move_selection(-1);
        assert_eq!(d.selected(), Slot::Default);
        d.move_selection(100);
        assert_eq!(d.selected(), Slot::LineNumber);
    }

    #[test]
    fn adjusting_steps_through_the_palette() {
        let mut d = dialog();
        d.open(Theme::DARK);
        d.set_selected(1); // Keyword
        // The dark theme's keyword colour is itself a palette entry, so the step
        // is measured from wherever it sits rather than assumed to be off-palette.
        let start = PALETTE
            .iter()
            .position(|&c| c == Slot::Keyword.get(&Theme::DARK))
            .expect("the dark keyword colour is in the palette");
        assert!(d.adjust(1));
        assert_eq!(Slot::Keyword.get(&d.theme()), PALETTE[start + 1]);
        assert!(d.adjust(1));
        assert_eq!(Slot::Keyword.get(&d.theme()), PALETTE[start + 2]);
    }

    #[test]
    fn adjusting_clamps_at_the_ends_of_the_palette() {
        let mut d = dialog();
        d.open(Theme::DARK);
        d.set_selected(1);
        for _ in 0..PALETTE.len() {
            d.adjust(1);
        }
        assert_eq!(Slot::Keyword.get(&d.theme()), PALETTE[PALETTE.len() - 1]);
        assert!(!d.adjust(1));
    }

    #[test]
    fn a_colour_outside_the_palette_steps_to_an_end() {
        let mut d = dialog();
        let mut theme = Theme::DARK;
        // A colour no palette entry holds.
        Slot::Keyword.set(&mut theme, Rgb::new(1, 1, 1));
        d.open(theme);
        d.set_selected(1);
        assert!(d.adjust(1));
        assert_eq!(Slot::Keyword.get(&d.theme()), PALETTE[0]);
    }

    #[test]
    fn the_value_is_a_hex_triplet() {
        let mut d = dialog();
        let mut theme = Theme::DARK;
        Slot::Keyword.set(&mut theme, Rgb::new(0x56, 0x9C, 0xD6));
        d.open(theme);
        assert_eq!(d.value(Slot::Keyword), "#569CD6");
    }

    #[test]
    fn the_dialog_is_centred_in_the_window() {
        let mut d = dialog();
        d.set_viewport(1000.0, 800.0);
        let (left, top) = d.origin();
        assert_eq!(left, (1000.0 - WIDTH) / 2.0);
        assert_eq!(top, (800.0 - d.height()) / 2.0);
    }

    #[test]
    fn row_at_maps_a_click_to_a_row() {
        let mut d = dialog();
        d.open(Theme::DARK);
        d.set_viewport(1000.0, 800.0);
        let (left, top) = d.origin();
        let rows_top = top + PADDING + d.line_height() * 2.0;
        assert_eq!(d.row_at(left + 20.0, rows_top + 5.0), Some(0));
        assert_eq!(d.row_at(left + 20.0, rows_top + d.line_height() + 5.0), Some(1));
        assert_eq!(d.row_at(left + 20.0, rows_top - 5.0), None);
    }

    #[test]
    fn a_closed_dialog_has_no_rows() {
        assert_eq!(dialog().row_at(10.0, 10.0), None);
    }
}
