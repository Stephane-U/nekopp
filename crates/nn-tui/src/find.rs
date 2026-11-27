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

//! The find bar: one row above the status bar, a pattern and its count.
//!
//! The windowed bar carries eleven chips on a row of their own, because it has
//! the width for them. A terminal row is a much smaller thing, so this bar keeps
//! the two marks that change what a search *finds* -- match case and whole word
//! -- and leaves the rest of the window's options to the passes that bring regex,
//! replace and find-in-files with them. Tab moves between the three things on the
//! row, Space or Enter flips a mark, Escape closes: a mark is a thing you stand
//! on and press, which is how Midnight Commander's dialogs answer the same
//! question.
//!
//! The count is the window's own, `3/17` or `no match`, so that the two
//! front-ends say the same thing about the same pattern.

use std::ops::Range;

use nn_core::Editor;
use nn_shared::i18n::Catalog;

use crate::keys::Key;
use crate::palette::Tone;
use crate::screen::Grid;

/// How many hits the bar counts before it stops.
///
/// A pattern of one letter in a big file matches everywhere, and the count is
/// there to tell a reader whether to keep typing, not to number them all.
const MAX_HITS: usize = 10_000;

/// The three things Tab moves between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    /// The pattern field.
    Query,
    /// Match case.
    Case,
    /// Whole word.
    Word,
}

/// The bar, its pattern, and the hits that pattern has in the document.
pub struct Find {
    open: bool,
    query: String,
    match_case: bool,
    whole_word: bool,
    focus: Focus,
    /// Where the pattern matches, as character ranges, in document order.
    hits: Vec<Range<usize>>,
    /// Which of `hits` the bar last stepped onto.
    current: Option<usize>,
    /// Set when `hits` no longer describe the pattern or the document.
    ///
    /// Typing in a large file only marks this, rather than searching again on
    /// every letter: a search is a pass over the whole document, and a keystroke
    /// that costs one is a keystroke a reader feels.
    stale: bool,
    catalog: Option<Catalog>,
}

impl Find {
    /// A closed bar with no pattern.
    pub fn new() -> Self {
        Find {
            open: false,
            query: String::new(),
            match_case: false,
            whole_word: false,
            focus: Focus::Query,
            hits: Vec::new(),
            current: None,
            stale: true,
            catalog: None,
        }
    }

    /// Whether the bar is up, and the text area owes it a row.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The words the bar draws, in the language the settings name.
    pub fn set_catalog(&mut self, catalog: Option<Catalog>) {
        self.catalog = catalog;
    }

    /// The translation of `english`, or `english` itself.
    fn tr<'a>(&'a self, english: &'a str) -> &'a str {
        match &self.catalog {
            Some(catalog) => catalog.tr(english),
            None => english,
        }
    }

    /// Opens the bar on the pattern it held last, and counts what that finds.
    ///
    /// The pattern is kept rather than cleared, as in the window: a search you
    /// have to retype is a search you were better off not dismissing.
    pub fn open(&mut self, editor: &Editor) {
        self.open = true;
        self.focus = Focus::Query;
        self.refresh(editor, true);
    }

    /// Closes the bar, keeping the pattern and leaving the match selected.
    ///
    /// Closing is not cancelling: the caret stays on the match the last Enter
    /// walked to, which is what the window's find mode does too. The hits are
    /// dropped so that the next `F3` searches the document as it is now rather
    /// than as it was when the bar closed.
    pub fn close(&mut self) {
        self.open = false;
        self.hits.clear();
        self.current = None;
        self.stale = true;
    }

    /// Handles a key, answering whether the bar took it.
    ///
    /// A key it does not want goes back to the shell, so `Ctrl+S` saves and
    /// `Ctrl+Q` quits with the bar up: a bar that swallowed every chord would be
    /// a mode, and this one is a row.
    pub fn handle(&mut self, key: Key, editor: &mut Editor) -> bool {
        // Escape, Tab and the stepping keys mean the same thing wherever the
        // focus is, so they are answered before it is looked at.
        match key {
            Key::Escape => {
                self.close();
                return true;
            }
            Key::Tab => {
                self.focus = match self.focus {
                    Focus::Query => Focus::Case,
                    Focus::Case => Focus::Word,
                    Focus::Word => Focus::Query,
                };
                return true;
            }
            Key::Enter | Key::Down if self.focus == Focus::Query => {
                self.step(editor, true);
                return true;
            }
            Key::Up if self.focus == Focus::Query => {
                self.step(editor, false);
                return true;
            }
            // Standing on a mark, Enter and Space are the same press.
            Key::Enter | Key::Char(' ') if self.focus != Focus::Query => {
                self.flip(editor);
                return true;
            }
            _ => {}
        }
        if self.focus != Focus::Query {
            // A letter is not a mark: on a mark, the keys that are not one of
            // the above are the shell's again.
            return false;
        }
        match key {
            Key::Char(ch) => {
                self.query.push(ch);
                self.retyped(editor);
                true
            }
            Key::Backspace => {
                self.query.pop();
                self.retyped(editor);
                true
            }
            _ => false,
        }
    }

    /// Flips the mark the focus is on, and counts what the new rule finds.
    fn flip(&mut self, editor: &Editor) {
        match self.focus {
            Focus::Case => self.match_case = !self.match_case,
            Focus::Word => self.whole_word = !self.whole_word,
            Focus::Query => return,
        }
        // The marks decide what matches, so the count in the bar is now wrong.
        self.retyped(editor);
    }

    /// The pattern changed: recount, where recounting is cheap.
    fn retyped(&mut self, editor: &Editor) {
        self.refresh(editor, false);
    }

    /// Recomputes the hits, unless the document is large and this is not asked
    /// for outright.
    ///
    /// `force` is what Enter and `F3` pass: the user asked for an answer, so it
    /// is worth a pass over the file. Typing in a large file is not.
    fn refresh(&mut self, editor: &Editor, force: bool) {
        self.hits.clear();
        self.current = None;
        self.stale = true;
        if self.query.is_empty() || (!force && editor.doc.is_large()) {
            return;
        }
        self.hits =
            editor.find_all_matching(&self.query, self.match_case, self.whole_word, MAX_HITS);
        self.stale = false;
    }

    /// Moves to the next hit, or the one before it, wrapping at the ends.
    ///
    /// With no hit stepped to yet the search starts from the selection, and
    /// falls off whichever end of the document that leaves: `F3` on a caret at
    /// the bottom of the file finds the one at the top, which is what a reader
    /// expects from a search that says `no match` nowhere else.
    pub fn step(&mut self, editor: &mut Editor, forward: bool) {
        if self.stale {
            self.refresh(editor, true);
        }
        if self.hits.is_empty() {
            return;
        }
        let index = match (self.current, forward) {
            (Some(index), true) => (index + 1) % self.hits.len(),
            (Some(index), false) => (index + self.hits.len() - 1) % self.hits.len(),
            (None, true) => {
                let from = editor.selection_end();
                self.hits
                    .iter()
                    .position(|hit| hit.start >= from)
                    .unwrap_or(0)
            }
            (None, false) => {
                let from = editor.selection_start();
                self.hits
                    .iter()
                    .rposition(|hit| hit.end <= from)
                    .unwrap_or(self.hits.len() - 1)
            }
        };
        self.current = Some(index);
        if let Some(hit) = self.hits.get(index) {
            editor.select_range(hit.clone());
        }
    }

    /// What the match counter says, and whether it is bad news.
    fn count(&self) -> (String, bool) {
        if self.query.is_empty() {
            return (String::new(), false);
        }
        if self.stale && self.hits.is_empty() {
            // A large file has not been searched yet: saying `no match` would be
            // a claim the bar has not earned.
            return (self.tr("press Enter").to_string(), false);
        }
        if self.hits.is_empty() {
            return (self.tr("no match").to_string(), true);
        }
        let n = self.current.map_or(0, |index| index + 1);
        (format!("{n}/{}", self.hits.len()), false)
    }

    /// Draws the bar on `row`, edge to edge.
    ///
    /// The pattern takes what is left after the two marks and the count, which
    /// are pinned to the right: a long pattern loses its tail rather than the
    /// number that says whether the search found anything.
    pub fn draw(&self, grid: &mut Grid, row: usize, width: usize) {
        for column in 0..width {
            grid.put(row, column, ' ', Tone::StatusBar);
        }
        let label = self.tr("Find:");
        let at = grid.text(row, 0, label, Tone::StatusBar) + 1;
        let (count, alert) = self.count();
        // The counter keeps a fixed width so that the marks do not jump about as
        // a count changes from `no match` to `1/2` and back while typing.
        let count_w = COUNT_W.max(count.chars().count() + 1);
        let marks_at = width.saturating_sub(count_w + MARKS_W);
        let field_end = marks_at.saturating_sub(1).max(at);
        // The field runs from `at` to the mark before the marks, so the count
        // and the marks keep their places however long the pattern is.
        let room = field_end.saturating_sub(at);
        let query: String = self.query.chars().take(room).collect();
        grid.text(row, at, &query, Tone::StatusBar);
        if self.focus == Focus::Query {
            // The caret, where the next character goes: the same block the text
            // area draws, so the two read as one editor.
            let column = at + query.chars().count();
            if column < field_end {
                grid.put(row, column, ' ', Tone::Caret);
            }
        }
        let (case_at, word_at) = mark_columns(marks_at);
        let marks = [
            (case_at, "Aa", self.match_case, Focus::Case),
            (word_at, "W", self.whole_word, Focus::Word),
        ];
        for (at, mark, on, focus) in marks {
            self.draw_mark(grid, row, at, mark, on, self.focus == focus);
        }
        let tone = if alert { Tone::Alert } else { Tone::StatusBar };
        grid.text(row, width - count.chars().count() - 1, &count, tone);
    }

    /// One mark, lit when it is on, greyed when it is off, and underlined by the
    /// caret's colours when the focus is standing on it.
    fn draw_mark(
        &self,
        grid: &mut Grid,
        row: usize,
        at: usize,
        mark: &str,
        on: bool,
        focused: bool,
    ) {
        if focused {
            grid.put(row, at.saturating_sub(1), ' ', Tone::Caret);
        }
        let tone = if focused {
            Tone::Caret
        } else if on {
            Tone::MenuHover
        } else {
            Tone::Gutter
        };
        grid.text(row, at, mark, tone);
    }
}

/// The cells the two marks take up: `Aa`, a space, `W`, and the space that
/// separates them from the pattern.
const MARKS_W: usize = 4 + 1;

/// The cells the counter keeps: the longest word it can hold, `no match`, and
/// the column after it.
const COUNT_W: usize = 8 + 1;

/// Where `Aa` and `W` start, given the column the marks begin at.
fn mark_columns(marks_at: usize) -> (usize, usize) {
    (marks_at, marks_at + 3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nn_core::Document;

    /// An editor on `text`, with the caret at the start.
    fn editor(text: &str) -> Editor {
        Editor::new(Document::from_text(text))
    }

    /// Types `text` into the open bar.
    fn type_in(bar: &mut Find, editor: &mut Editor, text: &str) {
        for ch in text.chars() {
            bar.handle(Key::Char(ch), editor);
        }
    }

    #[test]
    fn a_pattern_counts_its_matches_and_enter_walks_them() {
        let mut editor = editor("one two one\ntwo one\n");
        let mut bar = Find::new();
        bar.open(&editor);
        type_in(&mut bar, &mut editor, "one");
        assert_eq!(bar.count().0, "0/3", "{:?}", bar.hits);
        bar.handle(Key::Enter, &mut editor);
        assert_eq!(editor.selection_start(), 0);
        assert_eq!(bar.count().0, "1/3");
        bar.handle(Key::Enter, &mut editor);
        assert_eq!(editor.selection_start(), 8);
        bar.handle(Key::Enter, &mut editor);
        assert_eq!(editor.selection_start(), 16);
        // Past the last one the search wraps to the first: a count that says
        // `no match` nowhere else has to mean what it says.
        bar.handle(Key::Enter, &mut editor);
        assert_eq!(editor.selection_start(), 0);
        assert_eq!(bar.count().0, "1/3");
        // And Up walks back, wrapping the same way.
        bar.handle(Key::Up, &mut editor);
        assert_eq!(editor.selection_start(), 16);
        assert_eq!(bar.count().0, "3/3");
    }

    #[test]
    fn the_two_marks_change_what_is_found() {
        let mut editor = editor("Foo foo FOO food\n");
        let mut bar = Find::new();
        bar.open(&editor);
        type_in(&mut bar, &mut editor, "foo");
        assert_eq!(bar.hits.len(), 4, "{:?}", bar.hits);
        // Tab stands on the match-case mark, and Space flips it.
        bar.handle(Key::Tab, &mut editor);
        assert_eq!(bar.focus, Focus::Case);
        bar.handle(Key::Char(' '), &mut editor);
        assert!(bar.match_case);
        assert_eq!(bar.hits.len(), 2, "case still folded: {:?}", bar.hits);
        // The same for whole word, which is what `food` was failing.
        bar.handle(Key::Tab, &mut editor);
        bar.handle(Key::Char(' '), &mut editor);
        assert!(bar.whole_word);
        assert_eq!(bar.hits.len(), 1);
        assert_eq!(bar.hits[0].start, 4);
        // And back round to the field with Tab, where a space is a space.
        bar.handle(Key::Tab, &mut editor);
        assert_eq!(bar.focus, Focus::Query);
        bar.handle(Key::Char(' '), &mut editor);
        assert_eq!(bar.query, "foo ");
    }

    #[test]
    fn escape_closes_and_leaves_the_match_selected() {
        let mut editor = editor("alpha beta\nalpha\n");
        let mut bar = Find::new();
        bar.open(&editor);
        type_in(&mut bar, &mut editor, "alpha");
        // Typing counts but does not move the selection, as in the window. The
        // caret stands on the first match, so the first Enter is that match.
        assert_eq!(editor.selection_start(), 0);
        bar.handle(Key::Enter, &mut editor);
        assert_eq!(editor.selection_start(), 0);
        bar.handle(Key::Enter, &mut editor);
        assert_eq!(editor.selection_start(), 11);
        assert!(bar.handle(Key::Escape, &mut editor));
        assert!(!bar.is_open());
        assert_eq!(editor.selection_start(), 11);
        // The pattern is kept, and the hits are not: F3 asks the document as it
        // is now rather than as it was.
        assert_eq!(bar.query, "alpha");
        assert!(bar.hits.is_empty());
        bar.step(&mut editor, true);
        // The caret is at the end of the second match, so the next one is the
        // first: a search that wraps.
        assert_eq!(editor.selection_start(), 0);
    }

    #[test]
    fn no_match_says_so_and_a_key_the_bar_does_not_want_goes_back() {
        let mut editor = editor("alpha\n");
        let mut bar = Find::new();
        bar.open(&editor);
        type_in(&mut bar, &mut editor, "zulu");
        assert_eq!(bar.count(), ("no match".to_string(), true));
        // Enter with nothing to step to leaves the caret alone.
        let caret = editor.caret();
        bar.handle(Key::Enter, &mut editor);
        assert_eq!(editor.caret(), caret);
        // A chord is the shell's: the bar reports that it did not take it.
        assert!(!bar.handle(Key::Ctrl('s'), &mut editor));
        assert!(!bar.handle(Key::Left, &mut editor));
        // An empty pattern counts nothing at all rather than zero.
        let mut bar = Find::new();
        bar.open(&editor);
        assert_eq!(bar.count().0, "");
    }

    #[test]
    fn the_marks_and_the_count_hold_their_places_on_the_row() {
        let mut editor = editor("one one\n");
        let mut bar = Find::new();
        bar.open(&editor);
        type_in(&mut bar, &mut editor, "one");
        let row = drawn(&bar, 30);
        assert!(row.starts_with("Find: one"), "{row:?}");
        // `Aa` and `W`, then a space, then the counter, which ends at the last
        // column but one so nothing is written where a terminal would wrap.
        assert_eq!(&row[16..19], "Aa ", "{row:?}");
        assert_eq!(row.chars().nth(19), Some('W'), "{row:?}");
        assert!(row.ends_with(" 0/2 "), "{row:?}");
        // A pattern longer than the room keeps its head and loses its tail, and
        // neither the marks nor the counter move to make space for it.
        type_in(&mut bar, &mut editor, " abcdefghijklmnop");
        let row = drawn(&bar, 30);
        assert!(row.starts_with("Find: one abc"), "{row:?}");
        assert_eq!(&row[16..19], "Aa ", "{row:?}");
        assert!(row.ends_with(" no match "), "{row:?}");
    }

    /// The bar's row, as a string.
    fn drawn(bar: &Find, width: usize) -> String {
        let mut grid = Grid::new(width, 1);
        bar.draw(&mut grid, 0, width);
        (0..width).map(|column| grid.get(0, column).ch).collect()
    }
}
