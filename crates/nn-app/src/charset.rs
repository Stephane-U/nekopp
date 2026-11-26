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

//! The character panel.
//!
//! Notepad++'s "ASCII Insertion Panel" is a grid of the 256 code points of
//! Latin-1, one cell each, with a click inserting the character at the caret. It
//! exists for the characters a keyboard cannot produce: the box-drawing set, the
//! accented letters of a language the layout is not set to, the currency signs.
//!
//! The grid is the whole range rather than a curated set, because the point of
//! the panel is that you can find a character you cannot type. A curated set
//! would be a different feature -- a favourites list -- and would still leave the
//! user without the character they came for.
//!
//! The panel floats rather than docking. Notepad++ docks it, but this editor's
//! right edge already belongs to the function list, and stacking a second docked
//! panel there would make the character grid's height depend on how many
//! functions the current document happens to have. A palette of fixed size never
//! moves, which is what a grid you are aiming at needs.
//!
//! The panel owns no document. It reports which byte was clicked and the
//! application inserts it, which is what lets every rule here be tested without
//! an editor.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use glyphon::{TextArea, TextBounds};

use crate::quad::QuadRenderer;
use crate::theme::{Theme, ToGpu};

/// Columns in the grid.
const COLUMNS: usize = 16;
/// Rows in the grid.
const ROWS: usize = 16;
/// Side of one cell, in pixels, when the window has room for it.
///
/// Fixed rather than derived from the line height. The grid is a table, and a
/// table whose cells resize with the font is harder to aim at, not easier. At
/// twenty pixels a cell the panel is 336 wide, which fits beside the text on any
/// window this editor is usable in.
const CELL: f32 = 20.0;
/// The smallest a cell is allowed to get.
///
/// Below this the glyphs stop being distinguishable and the grid stops being a
/// grid. A window smaller than the panel at this size simply clips it, which is
/// the honest outcome: there is no room.
const MIN_CELL: f32 = 8.0;
/// Padding inside the frame.
const PADDING: f32 = 8.0;
/// The glyph drawn in a cell whose character would otherwise be invisible.
///
/// A space is a real character and has to be clickable, but a blank cell is
/// indistinguishable from a cell that failed to draw. A middle dot says "there is
/// something here and it is not visible", which is exactly what a space is.
const BLANK: char = '\u{b7}';
/// The caption in the top-left corner.
const TITLE: &str = "Insert Character";

/// Whether `byte` can be inserted into a document.
///
/// The control characters are shown but not insertable. Notepad++ inserts them,
/// which puts a NUL or a BEL into a text file: invisible in the editor, and a
/// surprise for every tool that reads the file afterwards. The grid still shows
/// them, dimmed, because a table with holes in it reads as a bug rather than as a
/// decision.
pub fn insertable(byte: u8) -> bool {
    !char::from(byte).is_control()
}

/// The glyph drawn in `byte`'s cell.
fn cell_text(byte: u8) -> String {
    let c = char::from(byte);
    if c.is_control() {
        String::new()
    } else if c.is_whitespace() {
        BLANK.to_string()
    } else {
        c.to_string()
    }
}

/// The floating character grid.
pub struct CharPanel {
    open: bool,
    /// The byte the pointer is over, for the readout and the highlight.
    hover: Option<u8>,
    /// The window size, so the panel can centre itself.
    viewport: (f32, f32),
    /// One shaped buffer per cell, allocated up front so `text_areas` cannot
    /// index past the end of a half-built panel.
    buffers: Vec<Buffer>,
    /// Whether the cells have been shaped. They never change, so this is a
    /// one-way latch: the 256 shapes happen on the first frame the panel is open
    /// and never again.
    cells_shaped: bool,
    header: Buffer,
    title_shaped: bool,
    readout: Buffer,
    readout_shaped: String,
    metrics: Metrics,
    family: Family<'static>,
    char_width: f32,
}

impl CharPanel {
    /// Builds a closed panel.
    pub fn new(metrics: Metrics, family: Family<'static>, char_width: f32) -> Self {
        let mut header = Buffer::new_empty(metrics);
        header.set_size(None, None);
        let mut readout = Buffer::new_empty(metrics);
        readout.set_size(None, None);
        let mut buffers = Vec::with_capacity(COLUMNS * ROWS);
        for _ in 0..COLUMNS * ROWS {
            let mut buffer = Buffer::new_empty(metrics);
            buffer.set_size(None, None);
            buffers.push(buffer);
        }
        Self {
            open: false,
            hover: None,
            viewport: (0.0, 0.0),
            buffers,
            cells_shaped: false,
            header,
            title_shaped: false,
            readout,
            readout_shaped: String::new(),
            metrics,
            family,
            char_width,
        }
    }

    /// Rebuilds the grid at a new display density.
    pub fn set_metrics(&mut self, metrics: Metrics) {
        if metrics == self.metrics {
            return;
        }
        // Everything in this widget that is counted in characters -- a tab's
        // width, the column a value starts in, the box around a row -- is
        // measured with `char_width`, so the width has to grow with the text or
        // the box stays the size the window opened at and the labels run out of
        // it.
        self.char_width *= metrics.line_height / self.metrics.line_height;
        self.metrics = metrics;
        self.header.set_metrics(metrics);
        self.readout.set_metrics(metrics);
        for cell in &mut self.buffers {
            cell.set_metrics(metrics);
        }
    }

    /// Whether the panel is showing.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens the panel with nothing hovered.
    ///
    /// The hover is cleared rather than kept: the pointer is wherever it was when
    /// the panel was last closed, which is not over the panel now.
    pub fn open(&mut self) {
        self.open = true;
        self.hover = None;
    }

    /// Closes the panel.
    pub fn close(&mut self) {
        self.open = false;
        self.hover = None;
    }

    /// Opens the panel if it is closed and closes it if it is open.
    pub fn toggle(&mut self) {
        if self.open {
            self.close();
        } else {
            self.open();
        }
    }

    /// The byte under the pointer, if any.
    pub fn hover(&self) -> Option<u8> {
        self.hover
    }

    /// Sets the hovered byte, or clears it.
    pub fn set_hover(&mut self, byte: Option<u8>) {
        self.hover = byte;
    }

    /// The character a click on the hovered cell would insert.
    ///
    /// `None` for a control character, so the caller cannot insert one by
    /// forgetting to ask `insertable`.
    pub fn hovered_char(&self) -> Option<char> {
        self.hover
            .filter(|byte| insertable(*byte))
            .map(char::from)
    }

    /// Steps the hover by `(dx, dy)` cells, wrapping at the edges.
    ///
    /// Wrapping rather than clamping because the grid is a torus in the only
    /// sense that matters: the arrow keys are how you reach a cell without a
    /// mouse, and stopping dead at the last column would make the far side of the
    /// grid unreachable from the near side.
    pub fn move_hover(&mut self, dx: isize, dy: isize) {
        let current = self.hover.unwrap_or(0) as isize;
        let col = current % COLUMNS as isize;
        let row = current / COLUMNS as isize;
        let col = (col + dx).rem_euclid(COLUMNS as isize);
        let row = (row + dy).rem_euclid(ROWS as isize);
        self.hover = Some((row * COLUMNS as isize + col) as u8);
    }

    /// Tells the panel the window size, so it can centre itself.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.viewport = (width, height);
    }

    /// The side of one cell.
    ///
    /// Twenty pixels unless the window cannot show the panel at that size, in
    /// which case the cells shrink just enough to fit. The shrink is a fallback
    /// rather than a policy: a grid whose cells resize with the window moves
    /// every character under the pointer whenever the window is dragged, and the
    /// point of a grid is that a cell stays where it was put. It only gives way
    /// when the alternative is a row the user cannot reach at all.
    pub fn cell(&self) -> f32 {
        let (width, height) = self.viewport;
        let by_width = (width - 2.0 * PADDING) / COLUMNS as f32;
        let by_height = (height - 3.0 * PADDING - self.line_height()) / ROWS as f32;
        CELL.min(by_width).min(by_height).max(MIN_CELL)
    }

    /// The width the panel occupies.
    pub fn width(&self) -> f32 {
        COLUMNS as f32 * self.cell() + 2.0 * PADDING
    }

    /// The height the panel occupies.
    pub fn height(&self) -> f32 {
        PADDING + self.line_height() + PADDING + ROWS as f32 * self.cell() + PADDING
    }

    /// The line height the panel was built with.
    fn line_height(&self) -> f32 {
        self.metrics.line_height
    }

    /// The top-left corner, centred in the window.
    fn origin(&self) -> (f32, f32) {
        let (width, height) = self.viewport;
        let left = ((width - self.width()) / 2.0).max(0.0);
        let top = ((height - self.height()) / 2.0).max(0.0);
        (left, top)
    }

    /// The y of the grid's top edge, below the header.
    fn grid_top(&self) -> f32 {
        self.origin().1 + PADDING + self.line_height() + PADDING
    }

    /// The top-left corner of `byte`'s cell, given the panel's own corner.
    fn cell_origin(&self, byte: u8, left: f32, grid_top: f32) -> (f32, f32) {
        let index = byte as usize;
        let col = index % COLUMNS;
        let row = index / COLUMNS;
        let cell = self.cell();
        (
            left + PADDING + col as f32 * cell,
            grid_top + row as f32 * cell,
        )
    }

    /// The byte under `(x, y)` in window pixels, if any.
    ///
    /// The header is not a cell: a click on the caption does nothing rather than
    /// inserting whatever happens to be at index zero.
    pub fn cell_at(&self, x: f32, y: f32) -> Option<u8> {
        if !self.open {
            return None;
        }
        let (left, top) = self.origin();
        let gx = x - left - PADDING;
        let gy = y - (top + PADDING + self.line_height() + PADDING);
        if gx < 0.0 || gy < 0.0 {
            return None;
        }
        let cell = self.cell();
        let col = (gx / cell) as usize;
        let row = (gy / cell) as usize;
        if col >= COLUMNS || row >= ROWS {
            return None;
        }
        Some((row * COLUMNS + col) as u8)
    }

    /// The readout in the top-right corner.
    ///
    /// The code point in hex and in decimal, because the two are used for
    /// different things: hex is what a source file or a font tool wants, decimal
    /// is what a character-map reference lists.
    fn readout(&self) -> String {
        match self.hover {
            None => String::new(),
            Some(byte) => {
                let mut out = format!("U+{byte:04X} ({byte})");
                if !insertable(byte) {
                    out.push_str("  control");
                }
                out
            }
        }
    }

    /// Shapes whatever changed since the last frame.
    pub fn prepare(&mut self, font_system: &mut FontSystem) {
        if !self.open {
            return;
        }
        let attrs = Attrs::new().family(self.family);
        if !self.title_shaped {
            self.header.set_text(TITLE, &attrs, Shaping::Basic, None);
            self.header.shape_until_scroll(font_system, false);
            self.title_shaped = true;
        }
        if !self.cells_shaped {
            for byte in 0..=255u8 {
                let text = cell_text(byte);
                let buffer = &mut self.buffers[byte as usize];
                buffer.set_text(&text, &attrs, Shaping::Basic, None);
                buffer.shape_until_scroll(font_system, false);
            }
            self.cells_shaped = true;
        }
        let readout = self.readout();
        if readout != self.readout_shaped {
            self.readout.set_text(&readout, &attrs, Shaping::Basic, None);
            self.readout.shape_until_scroll(font_system, false);
            self.readout_shaped = readout;
        }
    }

    /// Queues the frame, the header rule, and the cell highlights.
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
        // A rule under the header, so the readout reads as a caption rather than
        // as the first row of the grid.
        quads.push(
            left + 1.0,
            top + PADDING + self.line_height() + PADDING * 0.5,
            width - 2.0,
            1.0,
            border,
        );

        let grid_top = self.grid_top();
        let cell = self.cell();
        for byte in 0..=255u8 {
            let (x, y) = self.cell_origin(byte, left, grid_top);
            if !insertable(byte) {
                quads.push(x, y, cell, cell, theme.menu_hover.to_linear());
            }
            if self.hover == Some(byte) {
                quads.push(x, y, cell, cell, theme.selection.to_linear());
            }
        }
    }

    /// The text areas: the caption, the readout, and every cell.
    pub fn text_areas<'a>(&'a self, theme: &Theme) -> Vec<TextArea<'a>> {
        if !self.open {
            return Vec::new();
        }
        let (left, top) = self.origin();
        let width = self.width();
        let height = self.height();
        let bounds = TextBounds {
            left: left as i32,
            top: top as i32,
            right: (left + width) as i32,
            bottom: (top + height) as i32,
        };
        let mut areas = Vec::with_capacity(COLUMNS * ROWS + 2);
        areas.push(TextArea {
            buffer: &self.header,
            left: left + PADDING,
            top: top + PADDING,
            scale: 1.0,
            bounds,
            default_color: theme.menu_text.to_cosmic(),
            custom_glyphs: &[],
        });
        // The readout is right-aligned by its own width. The font is monospace,
        // so counting characters is exact.
        let readout_chars = self.readout_shaped.chars().count() as f32;
        if readout_chars > 0.0 {
            areas.push(TextArea {
                buffer: &self.readout,
                left: left + width - PADDING - readout_chars * self.char_width,
                top: top + PADDING,
                scale: 1.0,
                bounds,
                default_color: theme.menu_shortcut.to_cosmic(),
                custom_glyphs: &[],
            });
        }
        let grid_top = top + PADDING + self.line_height() + PADDING;
        let cell = self.cell();
        for byte in 0..=255u8 {
            let (x, y) = self.cell_origin(byte, left, grid_top);
            areas.push(TextArea {
                buffer: &self.buffers[byte as usize],
                left: x + (cell - self.char_width) / 2.0,
                top: y + (cell - self.line_height()) / 2.0,
                scale: 1.0,
                bounds,
                default_color: if insertable(byte) {
                    theme.menu_text.to_cosmic()
                } else {
                    theme.menu_text_disabled.to_cosmic()
                },
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

#[cfg(test)]
mod tests {
    use super::*;

    fn panel() -> CharPanel {
        let metrics = Metrics::new(14.0, 20.0);
        let mut panel = CharPanel::new(metrics, Family::Monospace, 8.0);
        panel.set_viewport(900.0, 500.0);
        panel.open();
        panel
    }

    #[test]
    fn the_grid_covers_the_whole_range() {
        let panel = panel();
        let (left, top) = panel.origin();
        let grid_top = top + PADDING + panel.line_height() + PADDING;
        for byte in 0..=255u8 {
            let (x, y) = panel.cell_origin(byte, left, grid_top);
            // The centre of the cell, so rounding cannot land on a boundary.
            let hit = panel.cell_at(x + panel.cell() / 2.0, y + panel.cell() / 2.0);
            assert_eq!(hit, Some(byte), "cell {byte} did not hit itself");
        }
    }

    #[test]
    fn a_click_outside_the_grid_hits_nothing() {
        let panel = panel();
        let (left, top) = panel.origin();
        // Left of the panel.
        assert_eq!(panel.cell_at(left - 1.0, top + 100.0), None);
        // Above the panel.
        assert_eq!(panel.cell_at(left + 100.0, top - 1.0), None);
        // Right of the last column.
        assert_eq!(panel.cell_at(left + panel.width() + 1.0, top + 100.0), None);
        // Below the last row.
        assert_eq!(panel.cell_at(left + 100.0, top + panel.height() + 1.0), None);
    }

    #[test]
    fn the_header_is_not_a_cell() {
        let panel = panel();
        let (left, top) = panel.origin();
        // The caption line, which is inside the frame but above the grid.
        assert_eq!(panel.cell_at(left + PADDING + 2.0, top + PADDING + 2.0), None);
    }

    #[test]
    fn a_closed_panel_hits_nothing() {
        let mut panel = panel();
        let (left, top) = panel.origin();
        let grid_top = top + PADDING + panel.line_height() + PADDING;
        let (x, y) = panel.cell_origin(65, left, grid_top);
        panel.close();
        assert_eq!(panel.cell_at(x + panel.cell() / 2.0, y + panel.cell() / 2.0), None);
    }

    #[test]
    fn control_characters_are_not_insertable() {
        for byte in 0..=31u8 {
            assert!(!insertable(byte), "{byte} should not be insertable");
        }
        assert!(!insertable(127));
        // The C1 range is control too, even though it sits inside Latin-1.
        for byte in 128..=159u8 {
            assert!(!insertable(byte), "{byte} should not be insertable");
        }
    }

    #[test]
    fn printable_ascii_is_insertable() {
        for byte in 32..=126u8 {
            assert!(insertable(byte), "{byte} should be insertable");
        }
    }

    #[test]
    fn the_latin_1_supplement_is_insertable() {
        // The accented letters and the currency signs, which are the reason the
        // panel exists.
        for byte in [0xA0u8, 0xA3, 0xA9, 0xE9, 0xFC, 0xFF] {
            assert!(insertable(byte), "{byte} should be insertable");
        }
    }

    #[test]
    fn a_control_cell_draws_nothing() {
        assert_eq!(cell_text(0), "");
        assert_eq!(cell_text(7), "");
        assert_eq!(cell_text(127), "");
    }

    #[test]
    fn a_space_cell_draws_a_placeholder() {
        assert_eq!(cell_text(32), BLANK.to_string());
        // A no-break space is whitespace too, and equally invisible.
        assert_eq!(cell_text(0xA0), BLANK.to_string());
    }

    #[test]
    fn a_letter_cell_draws_the_letter() {
        assert_eq!(cell_text(b'A'), "A");
        assert_eq!(cell_text(0xE9), "\u{e9}");
    }

    #[test]
    fn the_panel_is_closed_until_opened() {
        let metrics = Metrics::new(14.0, 20.0);
        let panel = CharPanel::new(metrics, Family::Monospace, 8.0);
        assert!(!panel.is_open());
    }

    #[test]
    fn toggling_opens_and_closes() {
        let mut panel = panel();
        assert!(panel.is_open());
        panel.toggle();
        assert!(!panel.is_open());
        panel.toggle();
        assert!(panel.is_open());
    }

    #[test]
    fn closing_forgets_the_hover() {
        let mut panel = panel();
        panel.set_hover(Some(65));
        panel.close();
        assert_eq!(panel.hover(), None);
        panel.open();
        assert_eq!(panel.hover(), None);
    }

    #[test]
    fn the_panel_is_centred_in_the_window() {
        let panel = panel();
        let (left, top) = panel.origin();
        let expected_left = (900.0 - panel.width()) / 2.0;
        let expected_top = (500.0 - panel.height()) / 2.0;
        assert!((left - expected_left).abs() < 0.01);
        assert!((top - expected_top).abs() < 0.01);
    }

    #[test]
    fn the_panel_never_leaves_the_window() {
        let metrics = Metrics::new(14.0, 20.0);
        let mut panel = CharPanel::new(metrics, Family::Monospace, 8.0);
        // Smaller than the panel in both directions.
        panel.set_viewport(100.0, 100.0);
        panel.open();
        assert_eq!(panel.origin(), (0.0, 0.0));
    }

    #[test]
    fn the_panel_is_wider_than_the_grid_by_its_padding() {
        let panel = panel();
        assert_eq!(panel.width(), COLUMNS as f32 * panel.cell() + 2.0 * PADDING);
    }

    #[test]
    fn the_panel_is_taller_than_the_grid_by_its_header() {
        let panel = panel();
        let grid = ROWS as f32 * panel.cell();
        assert_eq!(
            panel.height(),
            grid + 2.0 * PADDING + panel.line_height() + PADDING
        );
    }

    #[test]
    fn a_roomy_window_gets_full_size_cells() {
        let panel = panel();
        assert_eq!(panel.cell(), CELL);
    }

    #[test]
    fn a_small_window_shrinks_the_cells_to_fit() {
        let metrics = Metrics::new(14.0, 20.0);
        let mut panel = CharPanel::new(metrics, Family::Monospace, 8.0);
        panel.set_viewport(420.0, 300.0);
        panel.open();
        assert!(panel.cell() < CELL);
        // The panel now fits the window it was given.
        assert!(panel.height() <= 300.0);
        assert!(panel.width() <= 420.0);
    }

    #[test]
    fn the_cells_never_shrink_past_the_minimum() {
        let metrics = Metrics::new(14.0, 20.0);
        let mut panel = CharPanel::new(metrics, Family::Monospace, 8.0);
        panel.set_viewport(60.0, 60.0);
        panel.open();
        assert_eq!(panel.cell(), MIN_CELL);
    }

    #[test]
    fn a_shrunk_grid_still_hits_every_cell() {
        let metrics = Metrics::new(14.0, 20.0);
        let mut panel = CharPanel::new(metrics, Family::Monospace, 8.0);
        panel.set_viewport(420.0, 300.0);
        panel.open();
        let (left, top) = panel.origin();
        let grid_top = top + PADDING + panel.line_height() + PADDING;
        let cell = panel.cell();
        for byte in 0..=255u8 {
            let (x, y) = panel.cell_origin(byte, left, grid_top);
            assert_eq!(
                panel.cell_at(x + cell / 2.0, y + cell / 2.0),
                Some(byte),
                "cell {byte} did not hit itself at {cell}px"
            );
        }
    }

    #[test]
    fn the_readout_is_empty_when_nothing_is_hovered() {
        let panel = panel();
        assert_eq!(panel.readout(), "");
    }

    #[test]
    fn the_readout_names_the_code_point() {
        let mut panel = panel();
        panel.set_hover(Some(b'A'));
        assert_eq!(panel.readout(), "U+0041 (65)");
        panel.set_hover(Some(0xE9));
        assert_eq!(panel.readout(), "U+00E9 (233)");
    }

    #[test]
    fn the_readout_says_control_for_a_control_character() {
        let mut panel = panel();
        panel.set_hover(Some(7));
        assert_eq!(panel.readout(), "U+0007 (7)  control");
    }

    #[test]
    fn a_control_character_has_no_character_to_insert() {
        let mut panel = panel();
        panel.set_hover(Some(7));
        assert_eq!(panel.hovered_char(), None);
        panel.set_hover(Some(b'A'));
        assert_eq!(panel.hovered_char(), Some('A'));
    }

    #[test]
    fn stepping_the_hover_wraps_at_the_edges() {
        let mut panel = panel();
        // Nothing hovered starts at the first cell.
        panel.move_hover(0, 0);
        assert_eq!(panel.hover(), Some(0));
        // Left from the first column wraps to the last.
        panel.move_hover(-1, 0);
        assert_eq!(panel.hover(), Some(15));
        // Up from the first row wraps to the last.
        panel.move_hover(0, -1);
        assert_eq!(panel.hover(), Some(255));
        // Right from the last column wraps to the first.
        panel.move_hover(1, 0);
        assert_eq!(panel.hover(), Some(240));
        // Down from the last row wraps to the first.
        panel.move_hover(0, 1);
        assert_eq!(panel.hover(), Some(0));
    }

    #[test]
    fn stepping_the_hover_moves_one_cell() {
        let mut panel = panel();
        panel.set_hover(Some(0x41));
        panel.move_hover(1, 0);
        assert_eq!(panel.hover(), Some(0x42));
        panel.move_hover(0, 1);
        assert_eq!(panel.hover(), Some(0x52));
        panel.move_hover(-1, -1);
        assert_eq!(panel.hover(), Some(0x41));
    }
}
