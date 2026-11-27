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

//! The completion list, as the terminal draws it.
//!
//! The window floats a box of shaped glyphs under the caret; this one draws a
//! border of cells and a row per candidate. What they agree on is everything
//! that decides what the reader sees and gets -- which words are offered, how
//! the cursor wraps, how far it scrolls, what accepting a row inserts and how
//! far the caret steps back -- and that lives in `nn_shared::completion`. The
//! box here is the terminal's half: a width that follows the longest label, a
//! flip above the caret when there is no room below, and a key rule that hands
//! anything it cannot use back to the document rather than swallowing it.
//!
//! The list is not modal. `Up` and `Down` walk it, `Enter` and `Tab` take the
//! row, `Escape` puts it away, and a character it would refilter on is typed
//! into the document first and then narrows the list -- which is what makes
//! typing past a match work without a chord to dismiss it first.

use nn_shared::completion::{Candidates, CompletionMode, Item, VISIBLE_ROWS};
use nn_shared::results::truncate;

use crate::keys::Key;
use crate::palette::Tone;
use crate::screen::Grid;

/// The narrowest the box gets, in cells.
///
/// Below this a candidate is a stub: the border and the padding already take
/// three cells, and a box narrower than its own frame reads as a mistake.
const MIN_WIDTH: usize = 8;

/// What the list made of a keystroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Not a key the list has a use for: the list is closed, and the key
    /// belongs to whatever is behind it.
    Passed,
    /// Escape: the list is closed, and the key is used up.
    Closed,
    /// The cursor moved, and the key is used up.
    Moved,
    /// Enter or Tab on the row under the cursor: the caller inserts it.
    Accepted,
    /// A character the mode refilters on: the caller types it and then shows
    /// the list it refills to.
    Typed(char),
}

/// The list, what it was filled from, and where the frame should put it.
pub struct Completion {
    list: Candidates,
    /// Which completion is being served, which is what decides whether a
    /// character narrows the list or ends it.
    mode: CompletionMode,
}

impl Completion {
    /// An empty list, closed.
    pub fn new() -> Self {
        Completion {
            list: Candidates::new(),
            mode: CompletionMode::Word,
        }
    }

    /// Whether the list is on screen.
    pub fn is_open(&self) -> bool {
        self.list.is_open()
    }

    /// Which completion the list is showing.
    pub fn mode(&self) -> CompletionMode {
        self.mode
    }

    /// The row under the cursor, for the accept handler.
    pub fn selected(&self) -> Option<&Item> {
        self.list.selected()
    }

    /// The candidates, top to bottom, which only the tests ask for.
    #[cfg(test)]
    pub fn items(&self) -> &[Item] {
        self.list.items()
    }

    /// The number of candidates, which only the tests ask for.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// Shows `items` for `mode`. Nothing found is a closed list rather than an
    /// empty box, which is the window's rule.
    pub fn show(&mut self, mode: CompletionMode, items: Vec<Item>) {
        self.mode = mode;
        self.list.show(items);
    }

    /// Hides the list.
    pub fn close(&mut self) {
        self.list.close();
    }

    /// Handles a key while the list is up.
    pub fn handle(&mut self, key: Key) -> Step {
        match key {
            Key::Up => {
                self.list.move_selection(-1);
                Step::Moved
            }
            Key::Down => {
                self.list.move_selection(1);
                Step::Moved
            }
            Key::Enter | Key::Tab if !self.list.is_empty() => Step::Accepted,
            Key::Escape => {
                self.close();
                Step::Closed
            }
            Key::Char(ch) if self.mode.refilters_on(ch) => Step::Typed(ch),
            // Every other key ends the list and belongs to the editor: an arrow
            // that moved the caret with a box still hanging under it would be
            // pointing at the wrong word.
            _ => {
                self.close();
                Step::Passed
            }
        }
    }

    /// Draws the box beside the cell the caret is drawn on.
    ///
    /// Below the caret when it fits, above it when it does not, and never over
    /// the status row: the bar is the line that says what the file is, and a
    /// list covering it would hide the one row the reader cannot scroll.
    pub fn draw(&self, grid: &mut Grid, caret: (usize, usize)) {
        if !self.list.is_open() || grid.width() == 0 || grid.height() == 0 {
            return;
        }
        // The status row is not the list's to cover: the bar is the line that
        // says what the file is, and nothing the reader can scroll hides it.
        let limit = grid.height().saturating_sub(1);
        let rows = self.list.len().min(VISIBLE_ROWS).min(limit.saturating_sub(2));
        if rows == 0 {
            return;
        }
        let width = self.width(grid.width());
        let height = rows + 2;
        let top = self.top(caret.0, height, limit);
        let left = caret.1.min(grid.width().saturating_sub(width));

        // The frame, as one rectangle: a list is an overlay, and an overlay
        // needs an edge or it reads as text that happens to be indented.
        for column in left..left + width {
            grid.put(top, column, '─', Tone::MenuBorder);
            grid.put(top + height - 1, column, '─', Tone::MenuBorder);
        }
        for row in top..top + height {
            grid.put(row, left, '│', Tone::MenuBorder);
            grid.put(row, left + width - 1, '│', Tone::MenuBorder);
        }
        grid.put(top, left, '┌', Tone::MenuBorder);
        grid.put(top, left + width - 1, '┐', Tone::MenuBorder);
        grid.put(top + height - 1, left, '└', Tone::MenuBorder);
        grid.put(top + height - 1, left + width - 1, '┘', Tone::MenuBorder);

        let selected = self.list.selected_index();
        let columns = width.saturating_sub(3);
        for offset in 0..rows {
            let index = self.list.scroll() + offset;
            let Some(item) = self.list.item(index) else {
                break;
            };
            let tone = if index == selected {
                Tone::MenuHover
            } else {
                Tone::Menu
            };
            let at = top + 1 + offset;
            for column in left + 1..left + width - 1 {
                grid.put(at, column, ' ', tone);
            }
            grid.text(at, left + 2, &truncate(&item.label, columns), tone);
        }
    }

    /// The box width for a screen `columns` wide: the longest label, plus a
    /// space of padding on each side and the two borders.
    fn width(&self, columns: usize) -> usize {
        let longest = self
            .list
            .items()
            .iter()
            .map(|item| item.label.chars().count())
            .max()
            .unwrap_or(0);
        (longest + 4).clamp(MIN_WIDTH, columns.max(MIN_WIDTH))
    }

    /// The row the box starts on: below the caret when it fits, above it when
    /// it does not, and never past the last row the text can use.
    fn top(&self, caret_row: usize, height: usize, limit: usize) -> usize {
        let mut top = caret_row + 1;
        if top + height > limit {
            top = caret_row.saturating_sub(height);
        }
        top.min(limit.saturating_sub(height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(names: &[&str]) -> Vec<Item> {
        names.iter().map(|name| Item::word(*name)).collect()
    }

    fn list(names: &[&str]) -> Completion {
        let mut completion = Completion::new();
        completion.show(CompletionMode::Word, items(names));
        completion
    }

    /// The frame as text, one string per row.
    fn drawn(
        completion: &Completion,
        caret: (usize, usize),
        width: usize,
        height: usize,
    ) -> Vec<String> {
        let mut grid = Grid::new(width, height);
        completion.draw(&mut grid, caret);
        (0..height)
            .map(|row| {
                (0..width)
                    .map(|column| grid.get(row, column).ch)
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn a_closed_list_draws_nothing() {
        let completion = Completion::new();
        let rows = drawn(&completion, (0, 0), 40, 10);
        assert!(rows.iter().all(|row| row.trim().is_empty()), "{rows:?}");
    }

    #[test]
    fn showing_nothing_leaves_the_list_closed() {
        let mut completion = Completion::new();
        completion.show(CompletionMode::Word, Vec::new());
        assert!(!completion.is_open());
    }

    #[test]
    fn the_box_is_a_border_with_a_row_per_candidate() {
        let completion = list(&["alpha", "beta"]);
        let rows = drawn(&completion, (0, 0), 40, 10);
        // The caret is on row nought, so the box starts on row one.
        assert!(rows[1].starts_with('┌'), "{:?}", rows[1]);
        assert_eq!(rows[1].chars().position(|ch| ch == '┐'), Some(8));
        assert_eq!(rows[2].chars().nth(2), Some('a'));
        assert!(rows[2].contains("alpha"), "{:?}", rows[2]);
        assert!(rows[3].contains("beta"), "{:?}", rows[3]);
        assert!(rows[4].starts_with('└'), "{:?}", rows[4]);
    }

    #[test]
    fn the_box_is_as_wide_as_its_longest_label_and_no_wider() {
        let completion = list(&["alpha"]);
        let rows = drawn(&completion, (0, 0), 40, 10);
        // Five characters, a space of padding on each side and two borders.
        assert_eq!(rows[1].chars().position(|ch| ch == '┐'), Some(8));
        // A label shorter than the narrowest box gets the narrowest box.
        let narrow = list(&["a"]);
        let rows = drawn(&narrow, (0, 0), 40, 10);
        assert_eq!(
            rows[1].chars().position(|ch| ch == '┐'),
            Some(MIN_WIDTH - 1)
        );
    }

    #[test]
    fn the_cursor_row_is_the_lit_one() {
        let mut completion = list(&["alpha", "beta"]);
        completion.handle(Key::Down);
        let mut grid = Grid::new(40, 10);
        completion.draw(&mut grid, (0, 0));
        assert_eq!(grid.get(3, 5).tone, Tone::MenuHover);
        assert_eq!(grid.get(2, 5).tone, Tone::Menu);
    }

    #[test]
    fn the_box_flips_above_the_caret_when_it_would_run_off_the_screen() {
        // Ten rows of screen minus the status row: a box of four rows does not
        // fit below row seven, so it hangs above it.
        let completion = list(&["alpha", "beta"]);
        let rows = drawn(&completion, (7, 0), 40, 10);
        assert!(rows[3].starts_with('┌'), "{rows:?}");
        assert!(rows[6].starts_with('└'), "{rows:?}");
        // And it never covers the status row.
        let many: Vec<String> = (0..30).map(|n| format!("word{n}")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        let completion = list(&refs);
        let rows = drawn(&completion, (9, 0), 40, 10);
        assert_eq!(rows[9].trim(), "", "the status row stays clear");
    }

    #[test]
    fn the_box_stays_inside_the_screen_when_the_caret_is_at_the_right_edge() {
        let completion = list(&["a_very_long_candidate"]);
        let rows = drawn(&completion, (0, 35), 40, 10);
        for row in &rows {
            assert_eq!(row.chars().count(), 40, "{row:?}");
        }
        // The right-hand border is the last column of the screen, not past it.
        assert_eq!(rows[1].chars().position(|ch| ch == '┐'), Some(39));
    }

    #[test]
    fn a_label_longer_than_the_box_is_cut_with_a_mark() {
        let long = "x".repeat(120);
        let completion = list(&[long.as_str()]);
        let rows = drawn(&completion, (0, 0), 40, 10);
        assert_eq!(rows[2].chars().count(), 40);
        assert!(rows[2].contains('…'), "{:?}", rows[2]);
    }

    #[test]
    fn only_the_visible_rows_are_drawn() {
        let many: Vec<String> = (0..30).map(|n| format!("word{n:02}")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        let mut completion = list(&refs);
        assert_eq!(completion.len(), 30);
        for _ in 0..VISIBLE_ROWS {
            completion.handle(Key::Down);
        }
        let rows = drawn(&completion, (0, 0), 40, 20);
        assert!(rows[2].contains("word01"), "{:?}", rows[2]);
        assert!(rows[1 + VISIBLE_ROWS].contains("word10"), "{rows:?}");
        assert!(
            rows.iter().all(|row| !row.contains("word00")),
            "{rows:?}"
        );
    }

    #[test]
    fn the_cursor_wraps_and_every_other_key_is_handed_back() {
        let mut completion = list(&["alpha", "beta"]);
        assert_eq!(completion.handle(Key::Up), Step::Moved);
        assert_eq!(
            completion.selected().map(|item| item.label.as_str()),
            Some("beta")
        );
        assert_eq!(completion.handle(Key::Enter), Step::Accepted);
        assert!(completion.is_open());
        // A word character narrows the list and is the caller's to type.
        assert_eq!(completion.handle(Key::Char('a')), Step::Typed('a'));
        // A space ends the word, so the list goes and the space is typed.
        assert_eq!(completion.handle(Key::Char(' ')), Step::Passed);
        assert!(!completion.is_open());
        // Escape closes without handing the key on.
        let mut completion = list(&["alpha"]);
        assert_eq!(completion.handle(Key::Escape), Step::Closed);
        assert!(!completion.is_open());
        // So does an arrow, which is what keeps the box pointing at the word the
        // caret is actually inside.
        let mut completion = list(&["alpha"]);
        assert_eq!(completion.handle(Key::Left), Step::Passed);
        assert!(!completion.is_open());
    }

    #[test]
    fn a_refilter_puts_the_cursor_back_at_the_top() {
        // The window does the same: narrowing a list to three rows leaves a
        // cursor on the fourth one nowhere to stand.
        let mut completion = list(&["alpha", "beta", "gamma"]);
        completion.handle(Key::Down);
        completion.handle(Key::Down);
        completion.show(CompletionMode::Word, items(&["gamma"]));
        assert_eq!(
            completion.selected().map(|item| item.label.as_str()),
            Some("gamma")
        );
    }
}
