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

//! The function list, as the terminal draws it.
//!
//! The window docks these rows along the right edge, where a press can pick one.
//! A terminal has no pointer, so the same rows -- `nn_shared::symbols`, the same
//! header and the same indent -- are a list you walk: `Enter` puts the caret on
//! the symbol's line and takes the list away, `Escape` puts it away, and
//! `View > Function List` opens it again.
//!
//! What is listed is the document as it was when the command ran. That is the
//! results list's rule as well, and it is the performance rule: scanning every
//! keystroke to keep a box up to date would spend the typing's time on the box,
//! and re-running the command is one visit to the menu.
//!
//! A keystroke is answered in the four ways the results list answers one --
//! [`Step`] is that list's -- because the shell handles both lists the same way
//! and two enums saying the same four things would drift apart.

use nn_core::Symbol;
use nn_shared::symbols::{header, row};

use crate::keys::Key;
use crate::palette::Tone;
use crate::results::Step;
use crate::screen::Grid;

/// How many symbols the list shows at once, before it starts scrolling.
///
/// The same eight the results list shows: enough to see whether the command
/// found the right part of the document, and never the whole screen.
const VISIBLE_ROWS: usize = 8;

/// The document's symbols, the cursor, and whether the list is on screen.
pub struct Symbols {
    symbols: Vec<Symbol>,
    /// Index of the first visible symbol.
    scroll: usize,
    /// Index of the symbol the cursor is standing on.
    selected: usize,
    open: bool,
    /// The symbol rows the last frame had, which is what a page is worth here.
    visible: usize,
}

impl Symbols {
    /// An empty list, closed.
    pub fn new() -> Self {
        Symbols {
            symbols: Vec::new(),
            scroll: 0,
            selected: 0,
            open: false,
            visible: VISIBLE_ROWS,
        }
    }

    /// Whether the list is on screen.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The symbols, in document order.
    pub fn symbols(&self) -> &[Symbol] {
        &self.symbols
    }

    /// The symbol at `index`, if the list is that long.
    pub fn symbol(&self, index: usize) -> Option<&Symbol> {
        self.symbols.get(index)
    }

    /// Where the cursor is standing, counted from one for a reader.
    pub fn position(&self) -> usize {
        self.selected + 1
    }

    /// Replaces the list with a fresh scan.
    ///
    /// An empty scan leaves the list shut, which is the window's rule as well: a
    /// document with nothing to list has no panel, and the shell says so in the
    /// bar rather than drawing a header over an empty box.
    pub fn set_symbols(&mut self, symbols: Vec<Symbol>) {
        self.open = !symbols.is_empty();
        self.symbols = symbols;
        self.scroll = 0;
        self.selected = 0;
    }

    /// Puts the list away, keeping the rows it had.
    pub fn close(&mut self) {
        self.open = false;
    }

    /// The rows the list wants, given how many the screen can spare.
    ///
    /// A header plus the symbols it can show, and never a row of the document's
    /// last: the reader asked to see the declarations, not to lose the file.
    pub fn screen_rows(&self, room: usize) -> usize {
        if !self.open {
            return 0;
        }
        (self.symbols.len().min(VISIBLE_ROWS) + 1).min(room)
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
                self.selected = self.symbols.len().saturating_sub(1);
                self.reveal();
                Step::Moved
            }
            Key::Enter if !self.symbols.is_empty() => Step::Chosen(self.selected),
            Key::Escape => {
                self.close();
                Step::Closed
            }
            _ => Step::No,
        }
    }

    /// Moves the cursor by `delta` rows, and keeps it on the screen.
    fn step(&mut self, delta: isize) -> Step {
        if self.symbols.is_empty() {
            return Step::No;
        }
        let last = self.symbols.len() - 1;
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
        let most = self.symbols.len().saturating_sub(rows);
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
        let header = format!(" {}", header(self.symbols.len()));
        grid.text(top, 0, &header, Tone::Menu);
        for offset in 0..rows.saturating_sub(1) {
            let index = self.scroll + offset;
            if index >= self.symbols.len() {
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
            let text = row(&self.symbols[index], width.saturating_sub(1));
            grid.text(at, 0, &format!(" {text}"), tone);
        }
    }
}

impl Default for Symbols {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::Grid;
    use nn_core::SymbolKind;

    fn symbol(name: &str, line: usize, depth: u8) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind: SymbolKind::Function,
            line,
            depth,
        }
    }

    fn list(count: usize) -> Symbols {
        let mut symbols = Symbols::new();
        symbols.set_symbols((0..count).map(|line| symbol("f", line, 0)).collect());
        symbols
    }

    /// The list's cells as text, one string per row.
    fn drawn(symbols: &mut Symbols, rows: usize, width: usize) -> Vec<String> {
        let mut grid = Grid::new(width, rows);
        symbols.draw(&mut grid, 0, rows, width);
        (0..rows)
            .map(|row| {
                (0..width)
                    .map(|column| grid.get(row, column).ch)
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn a_fresh_scan_opens_the_list_and_no_symbols_leaves_it_shut() {
        let mut symbols = Symbols::new();
        assert_eq!(symbols.screen_rows(20), 0);
        symbols.set_symbols(vec![symbol("main", 0, 0)]);
        assert!(symbols.is_open());
        // A header and the one row.
        assert_eq!(symbols.screen_rows(20), 2);
        // A document with nothing to list is the window's closed panel too.
        let mut empty = Symbols::new();
        empty.set_symbols(Vec::new());
        assert!(!empty.is_open());
        assert_eq!(empty.screen_rows(20), 0);
    }

    #[test]
    fn the_header_counts_the_symbols() {
        let mut one = list(1);
        assert_eq!(drawn(&mut one, 4, 40)[0].trim(), "1 symbol");
        let mut two = list(2);
        assert_eq!(drawn(&mut two, 4, 40)[0].trim(), "2 symbols");
    }

    #[test]
    fn a_nested_symbol_is_drawn_one_step_in() {
        let mut symbols = Symbols::new();
        symbols.set_symbols(vec![
            symbol("outer", 0, 0),
            symbol("inner", 1, 1),
            symbol("deeper", 2, 3),
        ]);
        let rows = drawn(&mut symbols, 5, 40);
        assert_eq!(rows[1].trim_end(), " outer");
        assert_eq!(rows[2].trim_end(), "   inner");
        assert_eq!(rows[3].trim_end(), "   deeper");
    }

    #[test]
    fn the_cursor_walks_the_list_and_enter_chooses_escape_closes() {
        let mut symbols = list(3);
        assert_eq!(symbols.handle(Key::Down), Step::Moved);
        assert_eq!(symbols.position(), 2);
        assert_eq!(symbols.handle(Key::Enter), Step::Chosen(1));
        assert_eq!(symbols.symbol(1).map(|symbol| symbol.line), Some(1));
        assert_eq!(symbols.handle(Key::Escape), Step::Closed);
        assert!(!symbols.is_open());
        // The top and the bottom are the ends, not one past them.
        let mut symbols = list(20);
        assert_eq!(symbols.handle(Key::Up), Step::Moved);
        assert_eq!(symbols.position(), 1);
        symbols.handle(Key::LineEnd);
        assert_eq!(symbols.position(), 20);
        symbols.handle(Key::Down);
        assert_eq!(symbols.position(), 20);
        symbols.handle(Key::LineStart);
        assert_eq!(symbols.position(), 1);
        // A letter belongs to the document behind the list.
        assert_eq!(symbols.handle(Key::Char('j')), Step::No);
    }

    #[test]
    fn enter_on_an_empty_list_is_not_a_choice() {
        let mut symbols = Symbols::new();
        assert_eq!(symbols.handle(Key::Enter), Step::No);
        assert_eq!(symbols.handle(Key::Down), Step::No);
    }

    #[test]
    fn the_list_never_takes_more_than_the_screen_can_spare() {
        let symbols = list(30);
        assert_eq!(symbols.screen_rows(4), 4, "a short screen gets four rows");
        assert_eq!(symbols.screen_rows(20), VISIBLE_ROWS + 1);
    }

    #[test]
    fn the_cursor_stays_on_screen_while_the_list_scrolls_under_it() {
        let mut symbols = list(20);
        for _ in 0..VISIBLE_ROWS {
            symbols.handle(Key::Down);
        }
        assert_eq!(symbols.position(), VISIBLE_ROWS + 1);
        assert_eq!(symbols.scroll, 1);
        symbols.handle(Key::LineEnd);
        assert_eq!(symbols.scroll, 20 - VISIBLE_ROWS);
        symbols.handle(Key::LineStart);
        assert_eq!(symbols.scroll, 0);
    }

    #[test]
    fn a_page_moves_by_what_the_screen_showed() {
        let mut symbols = list(30);
        // The frame is what says how tall the list is: three rows of symbols.
        let mut grid = Grid::new(40, 4);
        symbols.draw(&mut grid, 0, 4, 40);
        symbols.handle(Key::PageDown);
        assert_eq!(symbols.position(), 4);
        symbols.handle(Key::PageDown);
        assert_eq!(symbols.position(), 7);
    }

    #[test]
    fn the_cursor_row_is_the_lit_one() {
        let mut symbols = list(3);
        symbols.handle(Key::Down);
        let mut grid = Grid::new(30, 6);
        symbols.draw(&mut grid, 0, 6, 30);
        // Row nought is the header, so the lit row is the second symbol.
        assert_eq!(grid.get(2, 5).tone, Tone::MenuHover);
        assert_eq!(grid.get(1, 5).tone, Tone::Menu);
    }

    #[test]
    fn a_long_row_is_cut_with_a_mark_rather_than_spilling() {
        let mut symbols = Symbols::new();
        symbols.set_symbols(vec![symbol(&"x".repeat(200), 0, 0)]);
        let rows = drawn(&mut symbols, 4, 20);
        assert_eq!(rows[1].chars().count(), 20);
        assert!(rows[1].contains('…'), "{:?}", rows[1]);
    }
}
