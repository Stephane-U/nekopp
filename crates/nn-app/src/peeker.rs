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

//! The document peeker.
//!
//! Notepad++'s `Ctrl+Tab` list: every open document in one column, the selection
//! moved with the arrow keys and committed with Enter. It is drawn in the overlay
//! pass and centred in the window, because it is a chooser rather than a menu
//! attached to anything -- there is no pointer position for it to hang off.
//!
//! The list is capped at twelve rows and scrolls with the selection, so a session
//! with forty documents still shows a box you can read.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use glyphon::{TextArea, TextBounds};

use crate::complete::truncate;
use crate::quad::QuadRenderer;
use crate::theme::Theme;

/// The most rows the peeker shows at once.
pub const VISIBLE_ROWS: usize = 12;
/// The narrowest and widest the box gets, in pixels.
const MIN_WIDTH: f32 = 240.0;
const MAX_WIDTH: f32 = 560.0;
/// Padding inside the frame.
const PADDING: f32 = 8.0;

/// The floating list of open documents.
pub struct DocumentPeeker {
    titles: Vec<String>,
    /// Index into `titles` of the highlighted row.
    selected: usize,
    /// Index of the first visible row.
    scroll: usize,
    /// Window size, so the box can be centred.
    viewport: (f32, f32),
    open: bool,
    body: Buffer,
    /// What the body was shaped for: (row count, scroll, columns).
    shaped_key: (usize, usize, usize),
    family: Family<'static>,
    char_width: f32,
}

impl DocumentPeeker {
    /// Builds a closed peeker.
    pub fn new(metrics: Metrics, family: Family<'static>, char_width: f32) -> Self {
        Self {
            titles: Vec::new(),
            selected: 0,
            scroll: 0,
            viewport: (0.0, 0.0),
            open: false,
            body: Buffer::new_empty(metrics),
            shaped_key: (usize::MAX, usize::MAX, usize::MAX),
            family,
            char_width,
        }
    }

    /// Records the window size, so the box can be centred in it.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.viewport = (width, height);
    }

    /// Shows `titles` with `selected` highlighted. An empty list closes.
    pub fn show(&mut self, titles: Vec<String>, selected: usize) {
        self.open = !titles.is_empty();
        self.selected = selected.min(titles.len().saturating_sub(1));
        self.titles = titles;
        self.scroll = 0;
        self.scroll_to_selection();
        self.shaped_key = (usize::MAX, usize::MAX, usize::MAX);
    }

    /// Hides the peeker and drops the list.
    pub fn close(&mut self) {
        self.open = false;
        self.titles.clear();
        self.selected = 0;
        self.scroll = 0;
        self.shaped_key = (usize::MAX, usize::MAX, usize::MAX);
    }

    /// Whether the peeker is showing.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The highlighted document, for the commit handler.
    pub fn selected(&self) -> Option<usize> {
        self.open.then_some(self.selected)
    }

    /// The number of documents listed.
    pub fn len(&self) -> usize {
        self.titles.len()
    }

    /// Whether there is nothing to show.
    pub fn is_empty(&self) -> bool {
        self.titles.is_empty()
    }

    /// Moves the highlight by `delta` rows, wrapping at both ends.
    ///
    /// Wrapping rather than clamping: `Ctrl+Tab` held down should walk the whole
    /// list and come back round, which is the only way to reach the document
    /// above the current one without knowing how many there are.
    pub fn move_selection(&mut self, delta: isize) {
        if self.titles.is_empty() {
            return;
        }
        let n = self.titles.len() as isize;
        self.selected = (((self.selected as isize + delta) % n + n) % n) as usize;
        self.scroll_to_selection();
    }

    /// Highlights `index`, for a click on the list.
    pub fn set_selected(&mut self, index: usize) {
        if index < self.titles.len() {
            self.selected = index;
            self.scroll_to_selection();
        }
    }

    /// Keeps the highlighted row inside the visible window.
    fn scroll_to_selection(&mut self) {
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + VISIBLE_ROWS {
            self.scroll = self.selected + 1 - VISIBLE_ROWS;
        }
    }

    /// The width the box occupies.
    pub fn width(&self) -> f32 {
        let longest = self
            .titles
            .iter()
            .map(|title| title.chars().count())
            .max()
            .unwrap_or(0);
        (longest as f32 * self.char_width + PADDING * 2.0).clamp(MIN_WIDTH, MAX_WIDTH)
    }

    /// The height the box occupies.
    pub fn height(&self, line_height: f32) -> f32 {
        if !self.open {
            return 0.0;
        }
        let rows = self.titles.len().min(VISIBLE_ROWS) as f32;
        rows * line_height + PADDING * 2.0
    }

    /// The left edge: centred, and pinned to zero when the window is narrower
    /// than the box.
    pub fn left(&self) -> f32 {
        ((self.viewport.0 - self.width()) / 2.0).max(0.0)
    }

    /// The top edge: centred, and pinned to zero when the window is shorter than
    /// the box.
    pub fn top(&self, line_height: f32) -> f32 {
        ((self.viewport.1 - self.height(line_height)) / 2.0).max(0.0)
    }

    /// The document index under `(x, y)` in window pixels.
    pub fn item_at(&self, x: f32, y: f32, line_height: f32) -> Option<usize> {
        if !self.open {
            return None;
        }
        let left = self.left();
        let top = self.top(line_height);
        if x < left || x >= left + self.width() {
            return None;
        }
        if y < top + PADDING || y >= top + self.height(line_height) - PADDING {
            return None;
        }
        let row = ((y - top - PADDING) / line_height) as usize;
        if row >= VISIBLE_ROWS {
            return None;
        }
        let index = self.scroll + row;
        (index < self.titles.len()).then_some(index)
    }

    /// Highlights the document under `(x, y)`, if any.
    pub fn hover(&mut self, x: f32, y: f32, line_height: f32) {
        if let Some(index) = self.item_at(x, y, line_height) {
            self.selected = index;
        }
    }

    /// Rebuilds the shaped rows if anything they depend on changed.
    pub fn prepare(&mut self, font_system: &mut FontSystem) {
        if !self.open {
            return;
        }
        let columns = ((self.width() - PADDING * 2.0) / self.char_width).max(4.0) as usize;
        let key = (self.titles.len(), self.scroll, columns);
        if key == self.shaped_key {
            return;
        }
        self.shaped_key = key;

        let visible = self.titles.len().min(VISIBLE_ROWS);
        let mut text = String::with_capacity(visible * (columns + 1));
        for row in 0..visible {
            if row > 0 {
                text.push('\n');
            }
            text.push_str(&truncate(&self.titles[self.scroll + row], columns));
        }
        self.body.set_text(
            &text,
            &Attrs::new().family(self.family),
            Shaping::Basic,
            None,
        );
        self.body.shape_until_scroll(font_system, false);
    }

    /// Queues the frame and the highlighted row.
    pub fn push_quads(&self, quads: &mut QuadRenderer, line_height: f32, theme: &Theme) {
        if !self.open {
            return;
        }
        let left = self.left();
        let top = self.top(line_height);
        let width = self.width();
        let height = self.height(line_height);

        quads.push(
            left,
            top,
            width,
            height,
            theme.menu_background.to_linear(),
        );
        let border = theme.menu_border.to_linear();
        quads.push(left, top, width, 1.0, border);
        quads.push(left, top + height - 1.0, width, 1.0, border);
        quads.push(left, top, 1.0, height, border);
        quads.push(left + width - 1.0, top, 1.0, height, border);

        let row = self.selected.wrapping_sub(self.scroll);
        if row < VISIBLE_ROWS && self.selected < self.titles.len() {
            quads.push(
                left + 1.0,
                top + PADDING + row as f32 * line_height,
                width - 2.0,
                line_height,
                theme.menu_hover.to_linear(),
            );
        }
    }

    /// The text areas of the visible rows.
    pub fn text_areas<'a>(&'a self, line_height: f32, theme: &Theme) -> Vec<TextArea<'a>> {
        if !self.open {
            return Vec::new();
        }
        let left = self.left();
        let top = self.top(line_height);
        let bounds = TextBounds {
            left: left as i32,
            top: top as i32,
            right: (left + self.width()) as i32,
            bottom: (top + self.height(line_height)) as i32,
        };
        vec![TextArea {
            buffer: &self.body,
            left: left + PADDING,
            top: top + PADDING,
            scale: 1.0,
            bounds,
            default_color: theme.menu_text.to_cosmic(),
            custom_glyphs: &[],
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: f32 = 20.0;

    fn peeker() -> DocumentPeeker {
        let mut peeker = DocumentPeeker::new(Metrics::new(15.0, LINE), Family::Monospace, 8.0);
        peeker.set_viewport(800.0, 600.0);
        peeker
    }

    fn titles(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("file{i}.rs")).collect()
    }

    #[test]
    fn a_closed_peeker_has_no_size() {
        let peeker = peeker();
        assert!(!peeker.is_open());
        assert_eq!(peeker.height(LINE), 0.0);
        assert_eq!(peeker.selected(), None);
        assert_eq!(peeker.item_at(400.0, 300.0, LINE), None);
    }

    #[test]
    fn showing_an_empty_list_leaves_it_closed() {
        let mut peeker = peeker();
        peeker.show(Vec::new(), 0);
        assert!(!peeker.is_open());
    }

    #[test]
    fn the_list_opens_on_the_document_it_was_given() {
        let mut peeker = peeker();
        peeker.show(titles(4), 2);
        assert!(peeker.is_open());
        assert_eq!(peeker.selected(), Some(2));
    }

    #[test]
    fn a_selection_past_the_end_is_clamped() {
        let mut peeker = peeker();
        peeker.show(titles(3), 9);
        assert_eq!(peeker.selected(), Some(2));
    }

    #[test]
    fn the_selection_wraps_at_both_ends() {
        let mut peeker = peeker();
        peeker.show(titles(3), 0);
        peeker.move_selection(-1);
        assert_eq!(peeker.selected(), Some(2));
        peeker.move_selection(1);
        assert_eq!(peeker.selected(), Some(0));
    }

    #[test]
    fn the_box_is_centred_in_the_window() {
        let mut peeker = peeker();
        peeker.show(titles(3), 0);
        assert_eq!(peeker.left(), (800.0 - peeker.width()) / 2.0);
        assert_eq!(peeker.top(LINE), (600.0 - peeker.height(LINE)) / 2.0);
    }

    #[test]
    fn a_box_wider_than_the_window_sticks_to_the_left_edge() {
        let mut peeker = peeker();
        peeker.set_viewport(100.0, 600.0);
        peeker.show(titles(3), 0);
        assert_eq!(peeker.left(), 0.0);
    }

    #[test]
    fn a_box_taller_than_the_window_sticks_to_the_top_edge() {
        let mut peeker = peeker();
        peeker.set_viewport(800.0, 40.0);
        peeker.show(titles(3), 0);
        assert_eq!(peeker.top(LINE), 0.0);
    }

    #[test]
    fn the_height_caps_at_the_visible_row_count() {
        let mut peeker = peeker();
        peeker.show(titles(40), 0);
        assert_eq!(
            peeker.height(LINE),
            VISIBLE_ROWS as f32 * LINE + PADDING * 2.0
        );
    }

    #[test]
    fn the_width_follows_the_longest_title() {
        let mut peeker = peeker();
        peeker.show(vec!["a".to_string(), "x".repeat(40)], 0);
        assert_eq!(peeker.width(), 40.0 * 8.0 + PADDING * 2.0);
    }

    #[test]
    fn a_short_title_still_gets_the_minimum_width() {
        let mut peeker = peeker();
        peeker.show(vec!["a".to_string(), "b".to_string()], 0);
        assert_eq!(peeker.width(), MIN_WIDTH);
    }

    #[test]
    fn a_row_maps_back_to_its_document() {
        let mut peeker = peeker();
        peeker.show(titles(3), 0);
        let top = peeker.top(LINE) + PADDING;
        assert_eq!(peeker.item_at(400.0, top + 5.0, LINE), Some(0));
        assert_eq!(peeker.item_at(400.0, top + 2.0 * LINE + 5.0, LINE), Some(2));
    }

    #[test]
    fn a_click_outside_the_box_misses() {
        let mut peeker = peeker();
        peeker.show(titles(3), 0);
        assert_eq!(peeker.item_at(5.0, 300.0, LINE), None);
        assert_eq!(peeker.item_at(400.0, 5.0, LINE), None);
    }

    #[test]
    fn a_click_in_the_padding_misses() {
        let mut peeker = peeker();
        peeker.show(titles(3), 0);
        let top = peeker.top(LINE);
        assert_eq!(peeker.item_at(400.0, top + 1.0, LINE), None);
        assert_eq!(
            peeker.item_at(400.0, top + peeker.height(LINE) - 1.0, LINE),
            None
        );
    }

    #[test]
    fn scrolling_keeps_the_selection_visible() {
        let mut peeker = peeker();
        peeker.show(titles(40), 0);
        for _ in 0..VISIBLE_ROWS {
            peeker.move_selection(1);
        }
        assert_eq!(peeker.selected(), Some(VISIBLE_ROWS));
        assert_eq!(peeker.scroll, 1);
        // The row the selection is on is the last visible one.
        let top = peeker.top(LINE) + PADDING;
        let last = top + (VISIBLE_ROWS as f32 - 1.0) * LINE + 5.0;
        assert_eq!(peeker.item_at(400.0, last, LINE), Some(VISIBLE_ROWS));
    }

    #[test]
    fn scrolling_back_up_follows_the_selection() {
        let mut peeker = peeker();
        peeker.show(titles(40), 20);
        assert_eq!(peeker.scroll, 20 + 1 - VISIBLE_ROWS);
        peeker.set_selected(0);
        assert_eq!(peeker.scroll, 0);
    }

    #[test]
    fn hovering_marks_the_row_and_ignores_the_rest() {
        let mut peeker = peeker();
        peeker.show(titles(3), 0);
        let top = peeker.top(LINE) + PADDING;
        peeker.hover(400.0, top + LINE + 5.0, LINE);
        assert_eq!(peeker.selected(), Some(1));
        peeker.hover(5.0, top + 5.0, LINE);
        assert_eq!(peeker.selected(), Some(1));
    }

    #[test]
    fn closing_drops_the_list() {
        let mut peeker = peeker();
        peeker.show(titles(3), 1);
        peeker.close();
        assert!(!peeker.is_open());
        assert!(peeker.is_empty());
        assert_eq!(peeker.selected(), None);
    }
}
