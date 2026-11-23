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

//! The scroll window.
//!
//! The viewport is the only thing that changes when the user scrolls. Keeping it
//! separate from the document is what lets scrolling be a uniform update instead
//! of a scene rebuild: the renderer asks for a line range and a pixel offset, and
//! nothing else in the editor moves.
//!
//! Vertical position is `first_line` plus `top_offset` pixels, so a trackpad can
//! scroll smoothly through a line rather than snapping. Horizontal position is a
//! pixel offset because proportional fonts make character columns meaningless.

/// The visible window onto a document.
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    /// Index of the topmost line, partially or fully visible.
    pub first_line: usize,
    /// How many pixels of `first_line` are scrolled off the top.
    pub top_offset: f32,
    /// Horizontal scroll in pixels.
    pub scroll_x: f32,
    /// Width of the text area in pixels.
    pub width: f32,
    /// Height of the text area in pixels.
    pub height: f32,
    /// Height of one line in pixels.
    pub line_height: f32,
    /// Width of one character in pixels. Monospace assumption, for now.
    pub char_width: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            first_line: 0,
            top_offset: 0.0,
            scroll_x: 0.0,
            width: 800.0,
            height: 600.0,
            line_height: 20.0,
            char_width: 8.0,
        }
    }
}

impl Viewport {
    /// Resizes the text area.
    pub fn set_size(&mut self, width: f32, height: f32) {
        self.width = width.max(1.0);
        self.height = height.max(1.0);
    }

    /// Sets the font metrics.
    pub fn set_metrics(&mut self, line_height: f32, char_width: f32) {
        self.line_height = line_height.max(1.0);
        self.char_width = char_width.max(0.1);
    }

    /// How many lines fit fully in the window.
    pub fn full_line_count(&self) -> usize {
        ((self.height / self.line_height).floor() as usize).max(1)
    }

    /// How many lines are at least partly visible, including a partial last line.
    pub fn visible_line_count(&self) -> usize {
        let usable = self.height - self.top_offset;
        if usable <= 0.0 {
            return 1;
        }
        ((usable / self.line_height).ceil() as usize).max(1) + 1
    }

    /// The line range to draw, clamped to `total_lines`.
    pub fn visible_lines(&self, total_lines: usize) -> std::ops::Range<usize> {
        if total_lines == 0 {
            return 0..0;
        }
        let start = self.first_line.min(total_lines - 1);
        let end = (start + self.visible_line_count()).min(total_lines);
        start..end
    }

    /// The y pixel of `line`'s top edge, relative to the text area.
    pub fn line_y(&self, line: usize) -> f32 {
        (line as f32 - self.first_line as f32) * self.line_height - self.top_offset
    }

    /// The line under pixel `y`.
    pub fn y_to_line(&self, y: f32, total_lines: usize) -> usize {
        let raw = self.first_line as f32 + (y + self.top_offset) / self.line_height;
        (raw.floor().max(0.0) as usize).min(total_lines.saturating_sub(1))
    }

    /// The x pixel of character column `col`.
    pub fn column_x(&self, col: usize) -> f32 {
        col as f32 * self.char_width - self.scroll_x
    }

    /// The character column under pixel `x`.
    pub fn x_to_column(&self, x: f32) -> usize {
        (((x + self.scroll_x) / self.char_width).round().max(0.0)) as usize
    }

    /// Scrolls by `dy` pixels, positive meaning down.
    ///
    /// The far end is `total_lines - full_line_count`, not `total_lines - 1`:
    /// the last line belongs at the bottom of the window, and stopping one line
    /// short of that leaves a screenful of blank space under the text. That is
    /// what a restored session looked like -- the caret was at the end of the
    /// file, the viewport was scrolled while its height was still zero, and the
    /// clamp let the last line sit at the top with nothing below it.
    pub fn scroll_pixels(&mut self, dy: f32, total_lines: usize) {
        if total_lines == 0 {
            return;
        }
        let max = self.max_first_line(total_lines) as f32 * self.line_height;
        let pos =
            (self.first_line as f32 * self.line_height + self.top_offset + dy).clamp(0.0, max);
        let lines = (pos / self.line_height).floor();
        self.first_line = lines as usize;
        self.top_offset = pos - lines * self.line_height;
    }

    /// Scrolls by `delta` whole lines, positive meaning down.
    pub fn scroll_lines(&mut self, delta: i64, total_lines: usize) {
        if total_lines == 0 {
            return;
        }
        let max = self.max_first_line(total_lines) as i64;
        self.first_line = (self.first_line as i64 + delta).clamp(0, max) as usize;
        self.top_offset = 0.0;
    }

    /// The largest `first_line` that still fills the window with text.
    ///
    /// Shared by every scroll path so they cannot disagree about where the
    /// document ends. `scroll_to_bottom` already used this value; the others
    /// used `total_lines - 1` and could scroll the text off the top of the
    /// window.
    fn max_first_line(&self, total_lines: usize) -> usize {
        total_lines.saturating_sub(self.full_line_count())
    }

    /// Scrolls horizontally by `dx` pixels, clamped to `content_width`.
    pub fn scroll_x(&mut self, dx: f32, content_width: f32) {
        let max = (content_width - self.width).max(0.0);
        self.scroll_x = (self.scroll_x + dx).clamp(0.0, max);
    }

    /// Scrolls the minimum amount needed to bring `line` fully into view.
    pub fn ensure_line_visible(&mut self, line: usize, total_lines: usize) {
        let full = self.full_line_count();
        if line < self.first_line {
            self.first_line = line;
            self.top_offset = 0.0;
        } else if line >= self.first_line + full {
            self.first_line = line + 1 - full;
            self.top_offset = 0.0;
        }
        // Clamped to the same far end as the scroll paths, so a viewport that
        // was scrolled while its height was unknown is pulled back into the
        // document the first time it is asked to show anything.
        self.first_line = self.first_line.min(self.max_first_line(total_lines));
    }

    /// Scrolls the minimum amount needed to bring pixel `x` into view.
    pub fn ensure_x_visible(&mut self, x: f32, content_width: f32) {
        if x < self.scroll_x {
            self.scroll_x = x;
        } else if x + self.char_width > self.scroll_x + self.width {
            self.scroll_x = x + self.char_width - self.width;
        }
        let max = (content_width - self.width).max(0.0);
        self.scroll_x = self.scroll_x.clamp(0.0, max);
    }

    /// Jumps to the top of the document.
    pub fn scroll_to_top(&mut self) {
        self.first_line = 0;
        self.top_offset = 0.0;
    }

    /// Jumps to the bottom of the document.
    pub fn scroll_to_bottom(&mut self, total_lines: usize) {
        self.first_line = total_lines.saturating_sub(self.full_line_count());
        self.top_offset = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp() -> Viewport {
        Viewport {
            width: 800.0,
            height: 600.0,
            line_height: 20.0,
            char_width: 8.0,
            ..Default::default()
        }
    }

    #[test]
    fn full_line_count_fits_the_window() {
        assert_eq!(vp().full_line_count(), 30);
    }

    #[test]
    fn visible_lines_cover_a_partial_last_line() {
        let v = vp();
        assert_eq!(v.visible_lines(1000), 0..31);
    }

    #[test]
    fn visible_lines_clamp_to_document() {
        let v = vp();
        assert_eq!(v.visible_lines(10), 0..10);
        assert_eq!(v.visible_lines(0), 0..0);
    }

    #[test]
    fn line_y_tracks_scroll() {
        let mut v = vp();
        assert_eq!(v.line_y(0), 0.0);
        assert_eq!(v.line_y(3), 60.0);
        v.first_line = 2;
        assert_eq!(v.line_y(2), 0.0);
        assert_eq!(v.line_y(4), 40.0);
    }

    #[test]
    fn y_to_line_inverts_line_y() {
        let mut v = vp();
        v.first_line = 5;
        v.top_offset = 7.0;
        for line in 5..20 {
            let y = v.line_y(line) + 1.0;
            assert_eq!(v.y_to_line(y, 100), line);
        }
    }

    #[test]
    fn scroll_pixels_accumulates_sub_line_offset() {
        let mut v = vp();
        v.scroll_pixels(30.0, 100);
        assert_eq!(v.first_line, 1);
        assert_eq!(v.top_offset, 10.0);
        v.scroll_pixels(10.0, 100);
        assert_eq!(v.first_line, 2);
        assert_eq!(v.top_offset, 0.0);
    }

    #[test]
    fn scroll_pixels_clamps_at_both_ends() {
        let mut v = vp();
        v.scroll_pixels(-500.0, 100);
        assert_eq!(v.first_line, 0);
        assert_eq!(v.top_offset, 0.0);
        // 100 lines, 30 fit: the last line belongs at the bottom, so the
        // furthest the text may scroll is line 70 at the top.
        v.scroll_pixels(1e9, 100);
        assert_eq!(v.first_line, 70);
    }

    #[test]
    fn scroll_lines_clamps() {
        let mut v = vp();
        v.scroll_lines(5, 100);
        assert_eq!(v.first_line, 5);
        v.scroll_lines(-100, 100);
        assert_eq!(v.first_line, 0);
        v.scroll_lines(1000, 100);
        assert_eq!(v.first_line, 70);
    }

    #[test]
    fn a_short_document_cannot_scroll_at_all() {
        // Ten lines in a window that holds thirty: there is nowhere to go, and
        // the old clamp let the tenth line sit at the top of a blank window.
        let mut v = vp();
        v.scroll_lines(1000, 10);
        assert_eq!(v.first_line, 0);
        v.scroll_pixels(1e9, 10);
        assert_eq!(v.first_line, 0);
        v.ensure_line_visible(9, 10);
        assert_eq!(v.first_line, 0);
    }

    #[test]
    fn ensure_line_visible_pulls_a_stale_scroll_back_into_the_document() {
        // The restored-session case: the viewport was scrolled while its height
        // was still zero, so `full_line_count` was 1 and the last line ended up
        // at the top. Once the window has a real height, asking for the caret
        // must bring the text back.
        let mut v = vp();
        v.first_line = 99;
        v.ensure_line_visible(99, 100);
        assert_eq!(v.first_line, 70);
    }

    #[test]
    fn ensure_line_visible_scrolls_down() {
        let mut v = vp();
        v.ensure_line_visible(45, 100);
        assert_eq!(v.first_line, 16);
        assert_eq!(v.top_offset, 0.0);
    }

    #[test]
    fn ensure_line_visible_scrolls_up() {
        let mut v = vp();
        v.first_line = 40;
        v.ensure_line_visible(10, 100);
        assert_eq!(v.first_line, 10);
    }

    #[test]
    fn ensure_line_visible_is_a_noop_when_already_visible() {
        let mut v = vp();
        v.first_line = 10;
        v.ensure_line_visible(20, 100);
        assert_eq!(v.first_line, 10);
    }

    #[test]
    fn ensure_x_visible_scrolls_right() {
        let mut v = vp();
        v.ensure_x_visible(900.0, 2000.0);
        assert_eq!(v.scroll_x, 900.0 + 8.0 - 800.0);
    }

    #[test]
    fn ensure_x_visible_scrolls_left() {
        let mut v = vp();
        v.scroll_x = 200.0;
        v.ensure_x_visible(50.0, 2000.0);
        assert_eq!(v.scroll_x, 50.0);
    }

    #[test]
    fn scroll_x_clamps_to_content() {
        let mut v = vp();
        v.scroll_x(1e6, 1000.0);
        assert_eq!(v.scroll_x, 200.0);
        v.scroll_x(-1e6, 1000.0);
        assert_eq!(v.scroll_x, 0.0);
    }

    #[test]
    fn column_and_x_roundtrip() {
        let mut v = vp();
        v.scroll_x = 40.0;
        assert_eq!(v.column_x(10), 80.0 - 40.0);
        assert_eq!(v.x_to_column(40.0), 10);
    }
}
