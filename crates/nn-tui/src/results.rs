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

//! The find-in-files results list, as the terminal draws it.
//!
//! The window docks the same list along the bottom edge, and the rows say the
//! same thing -- `name:line: text`, under a header counting results and files --
//! because both come from `nn_shared::results`. What differs is what a terminal
//! can afford: the list is a picker there, not a fixture, so `Enter` opens the
//! hit and puts the list away and `Search Results Window` brings it back from the
//! menu. The hits are kept while it is down, which is what makes the second half
//! of that true.
//!
//! The cursor is one row and the keys are the four arrows plus the two paging
//! ones. `j` and `k` are not bound here on purpose: the list is not modal -- a
//! letter it does not want belongs to the document behind it, as it does for the
//! find bar -- so binding a letter would mean swallowing it.

use nn_core::Hit;
use nn_shared::results::{row, summary, truncate};

use crate::keys::Key;
use crate::palette::Tone;
use crate::screen::Grid;

/// How many hits the list shows at once, before it starts scrolling.
///
/// The window's panel is this tall for the same reason: eight rows is enough to
/// see whether the search found the right thing, and the document underneath is
/// what the reader came for.
const VISIBLE_ROWS: usize = 8;

/// What the list made of a keystroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Not a key the list has a use for.
    No,
    /// The cursor moved.
    Moved,
    /// Enter was pressed on the hit at this index.
    Chosen(usize),
    /// Escape: the list is closed, and the hits are kept.
    Closed,
}

/// The list of hits, its cursor, and whether it is on screen.
pub struct Results {
    hits: Vec<Hit>,
    /// Index of the first visible hit.
    scroll: usize,
    /// Index of the hit the cursor is standing on.
    selected: usize,
    /// Whether a hit has been stepped to yet.
    ///
    /// What the first `Next Search Result` means depends on it: the window opens
    /// the first hit going forward and the last going back, and only then starts
    /// counting from where it landed.
    walked: bool,
    open: bool,
    /// The hit rows the last frame had, which is what a page is worth here.
    visible: usize,
}

impl Results {
    /// An empty list, closed.
    pub fn new() -> Self {
        Results {
            hits: Vec::new(),
            scroll: 0,
            selected: 0,
            walked: false,
            open: false,
            visible: VISIBLE_ROWS,
        }
    }

    /// Whether the list is on screen.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The hits, in the order the search found them.
    pub fn hits(&self) -> &[Hit] {
        &self.hits
    }

    /// The hit at `index`, if the list is that long.
    pub fn hit(&self, index: usize) -> Option<&Hit> {
        self.hits.get(index)
    }

    /// Where the cursor is standing, counted from one for a reader.
    pub fn position(&self) -> usize {
        self.selected + 1
    }

    /// Replaces the list with a new search, and opens it.
    pub fn set_hits(&mut self, hits: Vec<Hit>) {
        self.hits = hits;
        self.scroll = 0;
        self.selected = 0;
        self.walked = false;
        self.open = true;
    }

    /// Puts the list away, keeping its hits.
    ///
    /// Keeping them is the whole reason `Search Results Window` has something to
    /// show: the list leaves the screen when a hit is opened, and the reader who
    /// wants it back wants the same search, not a new one.
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Opens the list again, answering whether there was anything to show.
    pub fn reopen(&mut self) -> bool {
        if self.hits.is_empty() {
            return false;
        }
        self.open = true;
        true
    }

    /// The rows the list wants, given how many the screen can spare.
    ///
    /// A header plus the hits it can show, and never a row of the document's
    /// last: the reader asked to see the matches, not to lose the file.
    pub fn screen_rows(&self, room: usize) -> usize {
        if !self.open {
            return 0;
        }
        (self.hits.len().min(VISIBLE_ROWS) + 1).min(room)
    }

    /// Handles a key, answering what it meant.
    pub fn handle(&mut self, key: Key) -> Step {
        match key {
            Key::Up => self.step(-1),
            Key::Down => self.step(1),
            Key::PageUp => self.step(-(self.visible as isize)),
            Key::PageDown => self.step(self.visible as isize),
            Key::LineStart => {
                self.selected = 0;
                self.reveal();
                Step::Moved
            }
            Key::LineEnd => {
                self.selected = self.hits.len().saturating_sub(1);
                self.reveal();
                Step::Moved
            }
            Key::Enter if !self.hits.is_empty() => {
                self.walked = true;
                Step::Chosen(self.selected)
            }
            Key::Escape => {
                self.close();
                Step::Closed
            }
            _ => Step::No,
        }
    }

    /// Moves to the hit `Next Search Result` means, or `Previous`, and answers
    /// where it landed.
    ///
    /// The window's arithmetic, one for one: a list nobody has stepped through
    /// answers with its first hit going forward and its last going back, and the
    /// walk wraps at both ends. Stepping also moves the cursor the list draws, so
    /// reopening it shows where the reader is rather than where they started.
    pub fn advance(&mut self, forward: bool) -> Option<usize> {
        let count = self.hits.len();
        if count == 0 {
            return None;
        }
        let next = if !self.walked {
            if forward {
                0
            } else {
                count - 1
            }
        } else if forward {
            (self.selected + 1) % count
        } else {
            (self.selected + count - 1) % count
        };
        self.walked = true;
        self.selected = next;
        self.reveal();
        self.open = false;
        Some(next)
    }

    /// Moves the cursor by `delta` rows, and keeps it on the screen.
    fn step(&mut self, delta: isize) -> Step {
        if self.hits.is_empty() {
            return Step::No;
        }
        let last = self.hits.len() - 1;
        let next = (self.selected as isize + delta).clamp(0, last as isize);
        self.selected = next as usize;
        self.reveal();
        Step::Moved
    }

    /// Scrolls the least that puts the cursor back on the screen.
    fn reveal(&mut self) {
        let rows = self.visible.max(1);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        }
        if self.selected >= self.scroll + rows {
            self.scroll = self.selected + 1 - rows;
        }
        let most = self.hits.len().saturating_sub(rows);
        self.scroll = self.scroll.min(most);
    }

    /// Draws the list over the `rows` starting at `top`.
    ///
    /// The rows the frame gives are also what a page is worth, so they are kept:
    /// `Page Down` has to move by what the reader can see, and only the frame
    /// knows what that was.
    pub fn draw(&mut self, grid: &mut Grid, top: usize, rows: usize, width: usize) {
        if !self.open || rows == 0 {
            return;
        }
        self.visible = rows.saturating_sub(1).max(1);
        for row in top..top + rows {
            for column in 0..width {
                grid.put(row, column, ' ', Tone::Menu);
            }
        }
        let header = format!(" {}", summary(&self.hits));
        grid.text(top, 0, &truncate(&header, width), Tone::Menu);
        for offset in 0..rows.saturating_sub(1) {
            let index = self.scroll + offset;
            if index >= self.hits.len() {
                break;
            }
            let tone = if index == self.selected {
                Tone::MenuHover
            } else {
                Tone::Menu
            };
            let at = top + 1 + offset;
            for column in 0..width {
                grid.put(at, column, ' ', tone);
            }
            let text = row(&self.hits[index], width.saturating_sub(1));
            grid.text(at, 0, &format!(" {text}"), tone);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::Grid;
    use std::path::PathBuf;

    fn hit(line: usize) -> Hit {
        Hit {
            path: PathBuf::from("/tmp/notes.txt"),
            line,
            column: 0,
            text: format!("line {line}"),
            tab: None,
        }
    }

    fn list(count: usize) -> Results {
        let mut results = Results::new();
        results.set_hits((0..count).map(hit).collect());
        results
    }

    /// The panel's cells as text, one string per row.
    fn drawn(results: &mut Results, rows: usize, width: usize) -> Vec<String> {
        let mut grid = Grid::new(width, rows);
        results.draw(&mut grid, 0, rows, width);
        (0..rows)
            .map(|row| {
                (0..width)
                    .map(|column| grid.get(row, column).ch)
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn a_closed_list_takes_no_room_and_opens_on_a_search() {
        let mut results = Results::new();
        assert_eq!(results.screen_rows(20), 0);
        results.set_hits(vec![hit(1), hit(2)]);
        assert!(results.is_open());
        // A header and the two rows.
        assert_eq!(results.screen_rows(20), 3);
    }

    #[test]
    fn the_header_counts_the_results_and_the_files() {
        let mut results = list(2);
        assert_eq!(drawn(&mut results, 4, 40)[0].trim(), "2 results in 1 file");
        // No hits is a sentence rather than a list of nothing.
        let mut empty = Results::new();
        empty.set_hits(Vec::new());
        assert_eq!(drawn(&mut empty, 4, 40)[0].trim(), "No results");
    }

    #[test]
    fn a_row_names_the_file_and_counts_the_line_from_one() {
        let mut results = list(1);
        assert_eq!(drawn(&mut results, 4, 40)[1].trim(), "notes.txt:1: line 0");
    }

    #[test]
    fn the_list_never_takes_more_than_the_screen_can_spare() {
        let results = list(30);
        assert_eq!(results.screen_rows(4), 4, "a short screen gets four rows");
        assert_eq!(results.screen_rows(20), VISIBLE_ROWS + 1);
        // An empty list is its header and nothing else.
        let mut empty = Results::new();
        empty.set_hits(Vec::new());
        assert_eq!(empty.screen_rows(20), 1);
    }

    #[test]
    fn the_cursor_walks_the_list_and_takes_a_page() {
        let mut results = list(20);
        assert_eq!(results.handle(Key::Down), Step::Moved);
        assert_eq!(results.position(), 2);
        // The top and the bottom are the ends, not one past them.
        assert_eq!(results.handle(Key::Up), Step::Moved);
        assert_eq!(results.handle(Key::Up), Step::Moved);
        assert_eq!(results.position(), 1);
        results.handle(Key::LineEnd);
        assert_eq!(results.position(), 20);
        results.handle(Key::Down);
        assert_eq!(results.position(), 20);
        results.handle(Key::LineStart);
        assert_eq!(results.position(), 1);
        assert_eq!(results.handle(Key::Char('j')), Step::No);
    }

    #[test]
    fn the_cursor_stays_on_screen_while_the_list_scrolls_under_it() {
        let mut results = list(20);
        // Eight rows of hits show at once, so the ninth move scrolls by one.
        for _ in 0..VISIBLE_ROWS {
            results.handle(Key::Down);
        }
        assert_eq!(results.position(), VISIBLE_ROWS + 1);
        assert_eq!(results.scroll, 1);
        results.handle(Key::LineEnd);
        assert_eq!(results.scroll, 20 - VISIBLE_ROWS);
        results.handle(Key::LineStart);
        assert_eq!(results.scroll, 0);
    }

    #[test]
    fn a_page_moves_by_what_the_screen_showed() {
        let mut results = list(30);
        // The frame is what says how tall the list is: three rows of hits.
        let mut grid = Grid::new(40, 4);
        results.draw(&mut grid, 0, 4, 40);
        results.handle(Key::PageDown);
        assert_eq!(results.position(), 4);
        results.handle(Key::PageDown);
        assert_eq!(results.position(), 7);
    }

    #[test]
    fn enter_chooses_the_row_under_the_cursor_and_escape_closes() {
        let mut results = list(3);
        results.handle(Key::Down);
        assert_eq!(results.handle(Key::Enter), Step::Chosen(1));
        assert_eq!(results.handle(Key::Escape), Step::Closed);
        assert!(!results.is_open());
        // The hits outlive the close, so the menu can bring the list back.
        assert_eq!(results.hits().len(), 3);
        assert!(results.reopen());
        assert!(results.is_open());
        // With nothing to show there is nothing to reopen, and Enter on an empty
        // list is not a choice.
        let mut empty = Results::new();
        assert_eq!(empty.handle(Key::Enter), Step::No);
        empty.close();
        assert!(!empty.reopen());
    }

    #[test]
    fn the_cursor_row_is_the_lit_one() {
        let mut results = list(3);
        results.handle(Key::Down);
        let rows = drawn(&mut results, 6, 30);
        let mut grid = Grid::new(30, 6);
        results.draw(&mut grid, 0, 6, 30);
        // Row one is the header, so the lit row is the second hit.
        assert_eq!(grid.get(2, 5).tone, Tone::MenuHover);
        assert_eq!(grid.get(1, 5).tone, Tone::Menu);
        assert!(rows[2].starts_with(" notes.txt:2:"), "{:?}", rows[2]);
    }

    #[test]
    fn a_long_row_is_cut_with_a_mark_rather_than_spilling() {
        let mut results = Results::new();
        results.set_hits(vec![Hit {
            path: PathBuf::from("/tmp/a.txt"),
            line: 0,
            column: 0,
            text: "x".repeat(200),
            tab: None,
        }]);
        let rows = drawn(&mut results, 4, 20);
        assert_eq!(rows[1].chars().count(), 20);
        assert!(rows[1].contains('…'), "{:?}", rows[1]);
    }
}
