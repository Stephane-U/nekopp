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
//! the nine marks a terminal can honour -- match case, whole word, the three
//! ways to read the pattern (`N` normal, `E` extended, `.*` regular expression),
//! whether walking past an end wraps (`∞`), whether the search is confined to
//! the text selected when the mark was pressed (`▭`), and the window's two Mark
//! tab settings, `B` for a bookmark on the line a match lands on and `P` for
//! purging the previous search's marks first. The last two are the marks tab's
//! only switch-shaped settings; the tab's buttons are a highlight layer this
//! front-end does not have, so the two chips are read as the thing a terminal
//! can keep, which is a bookmark. Direction
//! is the arrows: on a terminal Up and Down *are* the two directions, where the
//! window needs a chip to say which way its button goes. The marks are the
//! window's own, in the
//! window's order, and the whole-word slot becomes `. matches newline` once the
//! pattern is a regex, which is the window's trick: a whole word is not a thing
//! a regular expression has. Tab moves between the things on the row, Space or
//! Enter presses one, Escape closes: a mark is a thing you stand on and press,
//! which is how Midnight Commander's dialogs answer the same question.
//!
//! `Ctrl+H` adds a second row: the replacement, and the two commands the window
//! puts beside it, `Replace` and `Replace All`. They are reached the same way the
//! marks are -- Tab stands on one, Space or Enter presses it -- because a
//! terminal has no buttons to click and a word you stand on is the nearest thing
//! to one. Enter in the replacement field replaces the match under the caret and
//! walks on, which is what it does in the window, and `Ctrl+R` does the same from
//! anywhere on the bar.
//!
//! The count is the window's own, `3/17` or `no match`, so that the two
//! front-ends say the same thing about the same pattern.

use std::ops::Range;

use nn_core::search::{Search, SearchMode, SearchOptions};
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

/// The things Tab moves between: the pattern and its two marks on the first row,
/// the replacement and its two commands on the second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    /// The pattern field.
    Query,
    /// Match case.
    Case,
    /// Whole word, or `.` matching a newline once the pattern is a regex.
    Word,
    /// Normal search: the pattern is literal text.
    Normal,
    /// Extended search: literal text with `\n`, `\t` and friends read.
    Extended,
    /// Regular expression.
    Regex,
    /// Continue at the other end when an end is reached.
    Wrap,
    /// Confine the search to the text that was selected.
    Selection,
    /// Put a bookmark on every line a match lands on.
    Bookmark,
    /// Drop the previous search's bookmarks before the first of this one's.
    Purge,
    /// The replacement field, on the second row.
    Replacement,
    /// The `Replace` command.
    Replace,
    /// The `Replace All` command.
    ReplaceAll,
}

/// The bar, its pattern, and the hits that pattern has in the document.
pub struct Find {
    open: bool,
    /// Whether the replacement row is up, which is what `Ctrl+H` opens.
    replace: bool,
    query: String,
    /// The text a replacement writes, kept across openings like the pattern.
    replacement: String,
    match_case: bool,
    whole_word: bool,
    /// How the pattern itself is read.
    mode: SearchMode,
    /// In [`SearchMode::Regex`], whether `.` crosses a line ending.
    dot_matches_newline: bool,
    /// Why the pattern would not compile, when it would not.
    error: Option<String>,
    /// Whether walking past the last match comes round to the first.
    wrap: bool,
    /// Whether the search is confined to [`Find::within`].
    in_selection: bool,
    /// The text `in_selection` confines to, in character indices.
    ///
    /// Taken once, when the mark is pressed, because on a terminal the search
    /// *is* the document's selection: by the second match, reading the selection
    /// back would confine the search to the match it just made. Clamped to the
    /// document as it is now, so a delete above it shortens what it covers
    /// rather than pointing past the end.
    within: Option<Range<usize>>,
    /// Whether a match bookmarks the line it lands on.
    bookmark_lines: bool,
    /// Whether the previous search's marks go before this one's.
    purge_marks: bool,
    /// Set by a fresh count, cleared by the first step that marks.
    ///
    /// Purging happens once per search, not once per match: a purge on every
    /// step would delete the mark the step before had just put down.
    fresh: bool,
    /// Something the bar cannot say itself: it has no line of its own.
    notice: Option<String>,
    focus: Focus,
    /// Where the pattern matches, as character ranges, in document order.
    hits: Vec<Range<usize>>,
    /// Which of `hits` the bar last stepped onto.
    current: Option<usize>,
    /// Set when the last key handled replaced something.
    ///
    /// The bar edits through the editor rather than through the shell, so nothing
    /// else would re-lex the lines it changed; the shell reads this and does.
    replaced: bool,
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
            replace: false,
            query: String::new(),
            replacement: String::new(),
            match_case: false,
            whole_word: false,
            mode: SearchMode::Normal,
            dot_matches_newline: false,
            error: None,
            wrap: true,
            in_selection: false,
            within: None,
            bookmark_lines: false,
            purge_marks: false,
            fresh: false,
            notice: None,
            focus: Focus::Query,
            hits: Vec::new(),
            current: None,
            replaced: false,
            stale: true,
            catalog: None,
        }
    }

    /// Whether the bar is up, and the text area owes it a row.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// How many rows the bar takes: the pattern, and the replacement when the
    /// replace row is up.
    pub fn rows(&self) -> usize {
        if !self.open {
            0
        } else if self.replace {
            2
        } else {
            1
        }
    }

    /// What the bar wanted to say but could not, once.
    ///
    /// A mark that does nothing when it is pressed is a mark that looks broken,
    /// so the one case that cannot be honoured -- searching inside a selection
    /// there is none of -- is said on the status bar, which is the shell's.
    pub fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
    }

    /// Whether the key just handled edited the document. Reading clears it.
    ///
    /// The bar replaces through the editor rather than through the shell, so the
    /// shell would otherwise never hear that the lines under it changed and would
    /// leave the frame painted with the colours of the text it replaced.
    pub fn take_replaced(&mut self) -> bool {
        std::mem::take(&mut self.replaced)
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
        self.replace = false;
        self.focus = Focus::Query;
        self.refresh(editor, true);
    }

    /// Opens the bar with the replacement row up, keeping both texts.
    ///
    /// `Ctrl+H` is the window's chord for the same thing, and it opens a row here
    /// rather than the window's dialog because a terminal has no dialog to open:
    /// the two front-ends offer the same command, each in the shape it has.
    pub fn open_replace(&mut self, editor: &Editor) {
        self.open = true;
        self.replace = true;
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
        // Escape, Tab, the arrows and `Ctrl+R` mean the same thing wherever the
        // focus is, so they are answered before it is looked at.
        self.replaced = false;
        match key {
            Key::Escape => {
                self.close();
                return true;
            }
            Key::Tab => {
                self.focus = self.next_focus();
                return true;
            }
            // The window answers this chord inside its bar with "replace the
            // match under the caret", and a terminal can send it: one letter,
            // and the keymap binds nothing to it.
            Key::Ctrl('r') => {
                self.replace_current(editor);
                return true;
            }
            // The arrows walk the matches wherever the focus stands, because a
            // bar that is up owns the caret: leaving one to the shell would move
            // the caret out from under the bar.
            Key::Down => {
                self.step(editor, true);
                return true;
            }
            Key::Up => {
                self.step(editor, false);
                return true;
            }
            // Enter acts on what the focus is standing on, which is the window's
            // rule too: in the pattern field it walks on, in the replacement
            // field it replaces, and on a mark or a command it presses it.
            Key::Enter if self.focus == Focus::Query => {
                self.step(editor, true);
                return true;
            }
            // Standing on a mark or a command, Enter and Space are the same
            // press. A field is not one: there a space is a space.
            Key::Enter | Key::Char(' ')
                if matches!(
                    self.focus,
                    Focus::Case
                        | Focus::Word
                        | Focus::Normal
                        | Focus::Extended
                        | Focus::Regex
                        | Focus::Wrap
                        | Focus::Selection
                        | Focus::Bookmark
                        | Focus::Purge
                ) =>
            {
                self.flip(editor);
                return true;
            }
            Key::Enter | Key::Char(' ') if self.focus == Focus::Replace => {
                self.replace_current(editor);
                return true;
            }
            Key::Enter | Key::Char(' ') if self.focus == Focus::ReplaceAll => {
                self.replace_all(editor);
                return true;
            }
            Key::Enter if self.focus == Focus::Replacement => {
                self.replace_current(editor);
                return true;
            }
            _ => {}
        }
        match self.focus {
            Focus::Query => match key {
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
            },
            Focus::Replacement => match key {
                Key::Char(ch) => {
                    self.replacement.push(ch);
                    true
                }
                Key::Backspace => {
                    self.replacement.pop();
                    true
                }
                _ => false,
            },
            // A mark and a command are not fields: every other key is the
            // shell's again.
            _ => false,
        }
    }

    /// Where Tab goes next.
    ///
    /// The order is the order the controls are drawn in -- the pattern and its
    /// seven marks along the first row, then the replacement and its two
    /// commands along the second -- so Tab reads down the bar rather than
    /// jumping about.
    fn next_focus(&self) -> Focus {
        match self.focus {
            Focus::Query => Focus::Case,
            Focus::Case => Focus::Word,
            Focus::Word => Focus::Normal,
            Focus::Normal => Focus::Extended,
            Focus::Extended => Focus::Regex,
            Focus::Regex => Focus::Wrap,
            Focus::Wrap => Focus::Selection,
            Focus::Selection => Focus::Bookmark,
            Focus::Bookmark => Focus::Purge,
            Focus::Purge if self.replace => Focus::Replacement,
            Focus::Purge => Focus::Query,
            Focus::Replacement => Focus::Replace,
            Focus::Replace => Focus::ReplaceAll,
            Focus::ReplaceAll => Focus::Query,
        }
    }

    /// Replaces the match the bar is standing on, and walks on to the next one.
    ///
    /// The window's Replace button does the same: the edit is one undo step, and
    /// the search carries on from where the replacement left the caret rather
    /// than from the match that no longer exists. A bar the reader typed into and
    /// reached straight for Replace has stepped to nothing yet, so this steps
    /// first -- the match the first Enter would have chosen.
    fn replace_current(&mut self, editor: &mut Editor) {
        if self.current.is_none() {
            self.step(editor, true);
        }
        let Some(hit) = self.current.and_then(|index| self.hits.get(index).cloned()) else {
            return;
        };
        let replacement = self.expand(editor, hit.clone());
        editor.select_range(hit);
        editor.insert(&replacement);
        self.replaced = true;
        self.refresh(editor, true);
        self.step(editor, true);
    }

    /// Replaces every match in the document, as one undo step.
    ///
    /// Every match rather than the count's ten thousand: a count may stop where
    /// the reader has seen enough, but a command that says it replaced everything
    /// and stopped at the ten-thousandth would be lying.
    fn replace_all(&mut self, editor: &mut Editor) {
        let Some(search) = self.search() else {
            return;
        };
        let hits = search.find_all(&editor.doc);
        if hits.is_empty() {
            return;
        }
        let edits: Vec<(Range<usize>, String)> = hits
            .into_iter()
            .map(|hit| {
                let text = search.expand(&editor.doc, hit.clone(), &self.replacement);
                (hit, text)
            })
            .collect();
        editor.replace_ranges(&edits);
        self.replaced = true;
        self.refresh(editor, true);
    }

    /// The pattern as a search, or `None` when there is no pattern yet.
    ///
    /// One matcher for the replacement, so that the text written can never be
    /// expanded against a pattern that matched something else -- which is what
    /// makes this the place regex arrives: the modes above are the literal ones,
    /// and a mode added here is a mode the count and the replacement both get.
    fn search(&self) -> Option<Search> {
        if self.query.is_empty() {
            return None;
        }
        Search::new(&self.query, self.options()).ok()
    }

    /// The options the marks on the row stand for.
    fn options(&self) -> SearchOptions {
        SearchOptions {
            case_sensitive: self.match_case,
            whole_word: self.whole_word,
            mode: self.mode,
            dot_matches_newline: self.dot_matches_newline,
        }
    }

    /// The text a replacement writes in place of `hit`.
    fn expand(&self, editor: &Editor, hit: Range<usize>) -> String {
        match self.search() {
            Some(search) => search.expand(&editor.doc, hit, &self.replacement),
            None => self.replacement.clone(),
        }
    }

    /// Flips the mark the focus is on, and counts what the new rule finds.
    fn flip(&mut self, editor: &Editor) {
        match self.focus {
            Focus::Case => self.match_case = !self.match_case,
            // The window's whole-word slot becomes `. matches newline` once the
            // pattern is a regular expression, because a whole word is not a
            // thing a regex has. Same slot here, same rule, and whole word keeps
            // the value it had for the modes that use it.
            Focus::Word if self.mode == SearchMode::Regex => {
                self.dot_matches_newline = !self.dot_matches_newline;
            }
            Focus::Word => self.whole_word = !self.whole_word,
            // The three modes are radios, not switches: a pattern is read one
            // way, and pressing the reading it already has is not a request to
            // read it as nothing.
            Focus::Normal => self.mode = SearchMode::Normal,
            Focus::Extended => self.mode = SearchMode::Extended,
            Focus::Regex => self.mode = SearchMode::Regex,
            Focus::Wrap => self.wrap = !self.wrap,
            Focus::Selection => self.flip_selection(editor),
            Focus::Bookmark => self.bookmark_lines = !self.bookmark_lines,
            Focus::Purge => self.purge_marks = !self.purge_marks,
            _ => return,
        }
        // The marks decide what matches, so the count in the bar is now wrong.
        self.retyped(editor);
    }

    /// Takes the selection as the search's region, or gives it up.
    ///
    /// The region is read once and kept while the mark is lit. Turning the mark
    /// on with nothing selected is refused and said out loud, rather than lit
    /// over an empty range that would find nothing.
    fn flip_selection(&mut self, editor: &Editor) {
        if self.in_selection {
            self.in_selection = false;
            self.within = None;
            return;
        }
        let range = editor.selection_start()..editor.selection_end();
        if range.is_empty() {
            let notice = self.tr("select the text to search in").to_string();
            self.notice = Some(notice);
            return;
        }
        self.in_selection = true;
        self.within = Some(range);
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
        self.error = None;
        if self.query.is_empty() || (!force && editor.doc.is_large()) {
            return;
        }
        // Through `Search` rather than the editor's literal finder, because the
        // pattern may be read three ways and the count has to be the count for
        // the reading the row is showing. One matcher for the count and the
        // replacement both, so what is written can never be expanded against a
        // pattern that matched something else.
        match Search::new(&self.query, self.options()) {
            Ok(search) => {
                self.hits = search.find_all(&editor.doc);
                // A new count is a new search, which is what the purge chip is
                // waiting for.
                self.fresh = true;
                if let Some(within) = &self.within {
                    // A match has to lie entirely inside the region: one that
                    // starts before the selection and ends in it is not inside
                    // anything the reader looked at, and replacing it would
                    // write outside the text they meant to touch.
                    let end = within.end.min(editor.doc.len_chars());
                    let start = within.start.min(end);
                    self.hits
                        .retain(|hit| hit.start >= start && hit.end <= end);
                }
                self.hits.truncate(MAX_HITS);
                self.stale = false;
            }
            Err(err) => self.error = Some(err.to_string()),
        }
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
        let stepped = match self.current {
            // From where the bar already is: one along, either way.
            Some(index) if forward => Some(index + 1),
            Some(index) => index.checked_sub(1),
            // The first step starts from the selection, and falls off whichever
            // end of the document that leaves.
            None if forward => {
                let from = editor.selection_end();
                self.hits.iter().position(|hit| hit.start >= from)
            }
            None => {
                let from = editor.selection_start();
                self.hits.iter().rposition(|hit| hit.end <= from)
            }
        };
        let index = match stepped.filter(|&index| index < self.hits.len()) {
            Some(index) => index,
            // Past an end: the other end with wrap on, and with wrap off a step
            // that does not happen -- jumping to the top would be the wrap it
            // was told not to do.
            None if self.wrap && forward => 0,
            None if self.wrap => self.hits.len() - 1,
            None => return,
        };
        self.current = Some(index);
        if let Some(hit) = self.hits.get(index) {
            let hit = hit.clone();
            editor.select_range(hit.clone());
            self.mark(editor, hit.start);
        }
    }

    /// Marks the line a match landed on, which is what `B` and `P` are about.
    ///
    /// The window's marks are highlights, and its Mark tab has a button that
    /// lays them down; a terminal has no highlight layer, so the two chips are
    /// read as the thing a terminal can keep -- a bookmark on the line, which
    /// the gutter already draws a dot for. `P` clears the previous search's
    /// bookmarks before the first match of a new one, so the dots left in the
    /// margin are the ones this pattern found.
    fn mark(&mut self, editor: &mut Editor, at: usize) {
        let fresh = std::mem::take(&mut self.fresh);
        if self.purge_marks && fresh {
            editor.doc.clear_bookmarks();
        }
        if !self.bookmark_lines {
            return;
        }
        let line = editor.doc.char_to_line(at);
        // Only when the line carries no mark already: the window marks a line
        // once however many matches it holds, and a toggle here would erase the
        // dot a second match on the same line had just put down.
        if !editor.doc.is_bookmarked(line) {
            editor.doc.toggle_bookmark(line);
        }
    }

    /// What the match counter says, and whether it is bad news.
    fn count(&self) -> (String, bool) {
        if self.query.is_empty() {
            return (String::new(), false);
        }
        // A pattern that will not compile found nothing because it is not a
        // pattern. The window's rule for the words: a short complaint is the
        // compiler's own, and a long one is `bad regex`, because a count slot is
        // not a place to read a paragraph.
        if let Some(err) = &self.error {
            return if err.len() > 12 {
                (self.tr("bad regex").to_string(), true)
            } else {
                (err.clone(), true)
            };
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

    /// Draws the bar's rows from `row` down: the pattern, and the replacement
    /// when the replace row is up.
    pub fn draw(&self, grid: &mut Grid, row: usize, width: usize) {
        self.draw_find_row(grid, row, width);
        if self.replace {
            self.draw_replace_row(grid, row + 1, width);
        }
    }

    /// Draws the replacement row: its field, and its two commands on the right.
    ///
    /// The commands are measured rather than assumed, because their words are
    /// translated: a language whose `Replace All` is twice as long still ends on
    /// the last column but one, and the field gives up the room instead.
    fn draw_replace_row(&self, grid: &mut Grid, row: usize, width: usize) {
        for column in 0..width {
            grid.put(row, column, ' ', Tone::StatusBar);
        }
        let label = self.tr("Replace:");
        let at = grid.text(row, 0, label, Tone::StatusBar) + 1;
        let all_label = self.tr("Replace All");
        let replace_label = self.tr("Replace");
        let all_at = width.saturating_sub(1 + all_label.chars().count());
        let replace_at = all_at.saturating_sub(BUTTON_GAP + replace_label.chars().count());
        let field_end = replace_at.saturating_sub(1).max(at);
        let room = field_end.saturating_sub(at);
        let field: String = self.replacement.chars().take(room).collect();
        grid.text(row, at, &field, Tone::StatusBar);
        if self.focus == Focus::Replacement {
            let column = at + field.chars().count();
            if column < field_end {
                grid.put(row, column, ' ', Tone::Caret);
            }
        }
        // A window too narrow for the labels keeps the field and drops the two
        // commands rather than writing them over the text they belong beside.
        if replace_at >= at && all_at >= replace_at + replace_label.chars().count() {
            self.draw_button(grid, row, replace_at, replace_label, self.focus == Focus::Replace);
            self.draw_button(grid, row, all_at, all_label, self.focus == Focus::ReplaceAll);
        }
    }

    /// One of the replacement row's commands: the whole word in the caret's
    /// colours while Tab is standing on it.
    fn draw_button(&self, grid: &mut Grid, row: usize, at: usize, label: &str, focused: bool) {
        let tone = if focused { Tone::Caret } else { Tone::MenuHover };
        grid.text(row, at, label, tone);
    }

    /// Draws the pattern's row on `row`, edge to edge.
    ///
    /// The pattern takes what is left after the seven marks and the count, which
    /// are pinned to the right: a long pattern loses its tail rather than the
    /// number that says whether the search found anything.
    fn draw_find_row(&self, grid: &mut Grid, row: usize, width: usize) {
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
        let columns = mark_columns(marks_at);
        // The window's third trick: the whole-word slot is `. matches newline`
        // when the pattern is a regex, which is why it has two marks and one
        // place to stand.
        let regex = self.mode == SearchMode::Regex;
        let marks = [
            ("Aa", self.match_case, Focus::Case),
            (
                if regex { "\\n" } else { "W" },
                if regex {
                    self.dot_matches_newline
                } else {
                    self.whole_word
                },
                Focus::Word,
            ),
            ("N", self.mode == SearchMode::Normal, Focus::Normal),
            ("E", self.mode == SearchMode::Extended, Focus::Extended),
            (".*", regex, Focus::Regex),
            // The window's own marks: an infinity for a search that keeps
            // going, and the white rectangle for the shape a selection makes.
            ("∞", self.wrap, Focus::Wrap),
            ("▭", self.in_selection, Focus::Selection),
            // The Mark tab in two cells: `B` bookmarks the line a match lands
            // on, `P` purges the previous search's bookmarks first.
            ("B", self.bookmark_lines, Focus::Bookmark),
            ("P", self.purge_marks, Focus::Purge),
        ];
        for (at, (mark, on, focus)) in columns.into_iter().zip(marks) {
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

/// The columns between the two commands on the replacement row.
const BUTTON_GAP: usize = 2;

/// The cells the nine marks take up: `Aa W  N E .* ∞ ▭  B P`, and
/// the space that separates them from the pattern.
///
/// The word slot keeps two columns of its own because its regex mark, `\n`, is
/// two characters: a row whose marks moved by one when the pattern changed
/// reading would be a row that shuffles under the reader's hand.
const MARKS_W: usize = 21 + 1;

/// The cells the counter keeps: the longest word it can hold, `no match`, and
/// the column after it.
const COUNT_W: usize = 8 + 1;

/// Where the nine marks start, given the column the marks begin at.
///
/// `Aa` and `W` say what matches; `N`, `E` and `.*` say how the pattern is read;
/// `∞` and `▭` say where the search may go; `B` and `P` say what a match leaves
/// behind. The wider gaps between the groups are where the word slot's second
/// column falls in a literal reading.
fn mark_columns(marks_at: usize) -> [usize; 9] {
    [
        marks_at,
        marks_at + 3,
        marks_at + 6,
        marks_at + 8,
        marks_at + 10,
        marks_at + 13,
        marks_at + 15,
        marks_at + 18,
        marks_at + 20,
    ]
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
        // And back round to the field with Tab -- past the three mode marks and
        // the two chips of the marks tab -- where a space is a space.
        for _ in 0..8 {
            bar.handle(Key::Tab, &mut editor);
        }
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
        let row = drawn(&bar, 48);
        assert!(row.starts_with("Find: one"), "{row:?}");
        // `Aa W  N E .* ∞ ▭  B P`, then the counter, which ends at the
        // last column but one so nothing is written where a terminal would wrap.
        // Counted in characters: three of the marks are not ASCII.
        let marks: String = row.chars().skip(17).take(21).collect();
        assert_eq!(marks, "Aa W  N E .* ∞ ▭  B P", "{row:?}");
        assert!(row.ends_with(" 0/2 "), "{row:?}");
        // A pattern longer than the room keeps its head and loses its tail, and
        // neither the marks nor the counter move to make space for it.
        type_in(&mut bar, &mut editor, " abcdefghijklmnop");
        let row = drawn(&bar, 48);
        assert!(row.starts_with("Find: one abc"), "{row:?}");
        let marks: String = row.chars().skip(17).take(21).collect();
        assert_eq!(marks, "Aa W  N E .* ∞ ▭  B P", "{row:?}");
        assert!(row.ends_with(" no match "), "{row:?}");
    }

    #[test]
    fn ctrl_h_opens_the_replace_row_and_tab_walks_its_controls() {
        let editor = editor("one\n");
        let mut bar = Find::new();
        assert_eq!(bar.rows(), 0, "a closed bar takes no rows");
        bar.open_replace(&editor);
        assert_eq!(bar.rows(), 2, "the replacement is a row of its own");
        // Tab reads down the bar: the pattern's row, then the replacement's.
        let mut order = Vec::new();
        for _ in 0..13 {
            bar.focus = bar.next_focus();
            order.push(bar.focus);
        }
        assert_eq!(
            order,
            vec![
                Focus::Case,
                Focus::Word,
                Focus::Normal,
                Focus::Extended,
                Focus::Regex,
                Focus::Wrap,
                Focus::Selection,
                Focus::Bookmark,
                Focus::Purge,
                Focus::Replacement,
                Focus::Replace,
                Focus::ReplaceAll,
                Focus::Query,
            ]
        );
        // Find mode closes the row again, and keeps what was typed in it.
        bar.open(&editor);
        assert_eq!(bar.rows(), 1);
        assert_eq!(bar.next_focus(), Focus::Case);
    }

    #[test]
    fn enter_in_the_replacement_field_replaces_the_match_and_walks_on() {
        let mut editor = editor("one two one\n");
        let mut bar = Find::new();
        bar.open_replace(&editor);
        type_in(&mut bar, &mut editor, "one");
        // Nothing has been stepped to yet, so the first press replaces the match
        // the first Enter would have chosen.
        bar.focus = Focus::Replacement;
        type_in(&mut bar, &mut editor, "1");
        assert!(bar.handle(Key::Enter, &mut editor));
        assert_eq!(editor.doc.text().as_str(), "1 two one\n");
        assert!(bar.replaced, "the shell was not told the document changed");
        // And it has walked on to the match that is left.
        assert_eq!(editor.selection_start(), 6);
        assert!(bar.handle(Key::Enter, &mut editor));
        assert_eq!(editor.doc.text().as_str(), "1 two 1\n");
        // A space is a space in this field, a press in the pattern's marks.
        bar.handle(Key::Char(' '), &mut editor);
        assert_eq!(bar.replacement, "1 ");
        assert_eq!(bar.count().0, "no match");
    }

    #[test]
    fn the_replace_command_and_ctrl_r_replace_one_match() {
        let mut editor = editor("a b a b\n");
        let mut bar = Find::new();
        bar.open_replace(&editor);
        type_in(&mut bar, &mut editor, "a");
        bar.focus = Focus::Replacement;
        type_in(&mut bar, &mut editor, "x");
        bar.focus = Focus::Replace;
        assert!(bar.handle(Key::Char(' '), &mut editor), "Space presses it");
        assert_eq!(editor.doc.text().as_str(), "x b a b\n");
        // Ctrl+R is the window's own chord for the same thing, from any focus.
        assert!(bar.handle(Key::Ctrl('r'), &mut editor));
        assert_eq!(editor.doc.text().as_str(), "x b x b\n");
    }

    #[test]
    fn replace_all_replaces_every_match_as_one_undo_step() {
        let mut editor = editor("one two one\nthree one\n");
        let mut bar = Find::new();
        bar.open_replace(&editor);
        type_in(&mut bar, &mut editor, "one");
        bar.focus = Focus::Replacement;
        type_in(&mut bar, &mut editor, "1");
        bar.handle(Key::Tab, &mut editor);
        assert_eq!(bar.focus, Focus::Replace);
        bar.handle(Key::Tab, &mut editor);
        assert_eq!(bar.focus, Focus::ReplaceAll);
        assert!(bar.handle(Key::Enter, &mut editor));
        assert_eq!(editor.doc.text().as_str(), "1 two 1\nthree 1\n");
        assert_eq!(bar.count().0, "no match", "the pattern is gone now");
        assert!(editor.undo());
        assert_eq!(editor.doc.text().as_str(), "one two one\nthree one\n");
    }

    #[test]
    fn the_replacement_row_is_drawn_under_the_find_row() {
        let mut editor = editor("one one\n");
        let mut bar = Find::new();
        bar.open_replace(&editor);
        type_in(&mut bar, &mut editor, "one");
        bar.focus = Focus::Replacement;
        type_in(&mut bar, &mut editor, "abc");
        // Wide enough for the pattern's field as well as the nine marks: the
        // marks take twenty-two cells of the row's right-hand end.
        let rows = drawn_rows(&bar, 46, 2);
        assert!(rows[0].starts_with("Find: one"), "{:?}", rows);
        assert!(rows[1].starts_with("Replace: abc"), "{:?}", rows);
        // The two commands are pinned to the right, one column from the edge,
        // and the field gives up its room to them rather than the other way
        // round.
        assert_eq!(&rows[1][25..32], "Replace", "{:?}", rows[1]);
        assert_eq!(&rows[1][34..45], "Replace All", "{:?}", rows[1]);
        // Standing on a command lights the whole word, and nothing else moves.
        bar.focus = Focus::ReplaceAll;
        let rows = drawn_rows(&bar, 46, 2);
        assert_eq!(&rows[1][34..45], "Replace All", "{:?}", rows[1]);
        // A window too narrow for the labels keeps the field's label alone.
        let narrow = drawn_rows(&bar, 20, 2);
        assert!(narrow[1].starts_with("Replace:"), "{:?}", narrow);
        assert!(!narrow[1].contains("Replace All"), "{:?}", narrow);
    }

    #[test]
    fn the_three_mode_marks_change_how_the_pattern_is_read() {
        // `a.c` is three characters to a literal search and three to a regex,
        // but only the regex reads the dot as any one character.
        let mut editor = editor("a.c abc\n");
        let mut bar = Find::new();
        bar.open(&editor);
        type_in(&mut bar, &mut editor, "a.c");
        assert_eq!(bar.count().0, "0/1", "{:?}", bar.hits);
        // Standing on `.*` and pressing it reads the pattern as a regex.
        bar.focus = Focus::Regex;
        bar.handle(Key::Char(' '), &mut editor);
        assert_eq!(bar.mode, SearchMode::Regex);
        assert_eq!(bar.count().0, "0/2", "{:?}", bar.hits);
        // Pressing the mark that is already lit leaves it lit: the three are
        // radios, not switches.
        bar.handle(Key::Char(' '), &mut editor);
        assert_eq!(bar.mode, SearchMode::Regex);
        // Normal is the literal reading again.
        bar.focus = Focus::Normal;
        bar.handle(Key::Enter, &mut editor);
        assert_eq!(bar.mode, SearchMode::Normal);
        assert_eq!(bar.count().0, "0/1", "{:?}", bar.hits);
    }

    #[test]
    fn an_extended_pattern_reads_an_escape_the_normal_one_does_not() {
        // The document holds a tab; the pattern holds a backslash and a `t`.
        let mut editor = editor("a\tb\n");
        let mut bar = Find::new();
        bar.open(&editor);
        type_in(&mut bar, &mut editor, "a\\tb");
        assert_eq!(bar.count().0, "no match", "{:?}", bar.hits);
        bar.focus = Focus::Extended;
        bar.handle(Key::Char(' '), &mut editor);
        assert_eq!(bar.mode, SearchMode::Extended);
        assert_eq!(bar.hits.len(), 1, "{:?}", bar.hits);
    }

    #[test]
    fn the_word_mark_becomes_dot_matches_newline_once_a_regex_is_read() {
        let mut editor = editor("a\nb\n");
        let mut bar = Find::new();
        bar.open(&editor);
        type_in(&mut bar, &mut editor, "a.b");
        // In the literal readings the mark is whole word, and `a.b` is not in
        // the document at all.
        bar.focus = Focus::Word;
        bar.handle(Key::Char(' '), &mut editor);
        assert!(bar.whole_word);
        assert!(bar.hits.is_empty());
        bar.focus = Focus::Regex;
        bar.handle(Key::Char(' '), &mut editor);
        // Now the same slot is `. matches newline`, still off: a dot stops at
        // the line ending, which is what the default is for.
        assert!(bar.whole_word, "whole word lost its value for the other modes");
        assert!(!bar.dot_matches_newline);
        assert!(bar.hits.is_empty(), "{:?}", bar.hits);
        bar.focus = Focus::Word;
        bar.handle(Key::Char(' '), &mut editor);
        assert!(bar.dot_matches_newline);
        assert_eq!(bar.hits.len(), 1, "{:?}", bar.hits);
    }

    #[test]
    fn a_pattern_that_will_not_compile_says_so_and_finds_nothing() {
        let mut editor = editor("abc\n");
        let mut bar = Find::new();
        bar.open(&editor);
        type_in(&mut bar, &mut editor, "a[");
        bar.focus = Focus::Regex;
        bar.handle(Key::Char(' '), &mut editor);
        assert_eq!(bar.mode, SearchMode::Regex);
        assert!(bar.hits.is_empty());
        // The compiler's message is long, so the count slot says the short word
        // the window uses, and says it as bad news.
        assert_eq!(bar.count(), ("bad regex".to_string(), true));
        // A pattern that compiles again clears it, and counting comes back.
        bar.focus = Focus::Query;
        bar.handle(Key::Backspace, &mut editor);
        assert_eq!(bar.count().0, "0/1");
    }

    #[test]
    fn walking_past_the_end_wraps_or_stops_where_the_mark_says() {
        let mut editor = editor("one two one\n");
        let mut bar = Find::new();
        bar.open(&editor);
        type_in(&mut bar, &mut editor, "one");
        // Wrap is on by default: the third Enter comes round to the first match.
        bar.handle(Key::Enter, &mut editor);
        bar.handle(Key::Enter, &mut editor);
        assert_eq!(editor.selection_start(), 8);
        bar.handle(Key::Enter, &mut editor);
        assert_eq!(editor.selection_start(), 0);
        // With the mark off the search stops at the last match instead, because
        // jumping to the first would be the wrap it was told not to do.
        bar.focus = Focus::Wrap;
        bar.handle(Key::Char(' '), &mut editor);
        assert!(!bar.wrap);
        // The arrows walk from any focus, where Enter would press the mark the
        // focus is still standing on.
        for _ in 0..3 {
            bar.handle(Key::Down, &mut editor);
        }
        assert_eq!(editor.selection_start(), 8, "the search wrapped anyway");
        assert_eq!(bar.count().0, "2/2", "the count moved off the last match");
    }

    #[test]
    fn the_selection_mark_confines_the_search_to_what_was_selected() {
        let mut editor = editor("one two one\nthree one\n");
        let mut bar = Find::new();
        bar.open(&editor);
        type_in(&mut bar, &mut editor, "one");
        assert_eq!(bar.count().0, "0/3");
        // Three matches, one of them inside `two one`.
        editor.select_range(4..11);
        bar.focus = Focus::Selection;
        bar.handle(Key::Char(' '), &mut editor);
        assert!(bar.in_selection);
        assert_eq!(bar.count().0, "0/1", "{:?}", bar.hits);
        assert_eq!(bar.hits[0], 8..11, "{:?}", bar.hits);
        // The mark is a switch: pressing it again gives the whole document back.
        bar.handle(Key::Char(' '), &mut editor);
        assert!(!bar.in_selection);
        assert_eq!(bar.count().0, "0/3", "{:?}", bar.hits);
    }

    #[test]
    fn the_selection_mark_refuses_to_light_over_nothing_and_says_why() {
        let mut editor = editor("one\n");
        let mut bar = Find::new();
        bar.open(&editor);
        type_in(&mut bar, &mut editor, "one");
        bar.focus = Focus::Selection;
        bar.handle(Key::Char(' '), &mut editor);
        assert!(!bar.in_selection, "the mark lit over an empty range");
        // A mark that does nothing when it is pressed looks broken, so the bar
        // hands the shell a sentence to put on its own line.
        assert_eq!(
            bar.take_notice().as_deref(),
            Some("select the text to search in")
        );
        assert_eq!(bar.take_notice(), None, "the notice came twice");
        // And pressing it again with a selection does light it.
        editor.select_range(0..3);
        bar.handle(Key::Char(' '), &mut editor);
        assert!(bar.in_selection);
    }

    #[test]
    fn the_regex_reading_puts_its_own_mark_in_the_word_slot() {
        let mut editor = editor("a.b\n");
        let mut bar = Find::new();
        bar.open(&editor);
        let row = drawn(&bar, 40);
        let word: String = row.chars().skip(12).take(2).collect();
        assert_eq!(word, "W ", "{row:?}");
        bar.focus = Focus::Regex;
        bar.handle(Key::Char(' '), &mut editor);
        let row = drawn(&bar, 40);
        let word: String = row.chars().skip(12).take(2).collect();
        assert_eq!(word, "\\n", "{row:?}");
        let regex: String = row.chars().skip(19).take(2).collect();
        assert_eq!(regex, ".*", "{row:?}");
    }

    #[test]
    fn the_bookmark_chip_marks_the_line_a_match_lands_on() {
        let mut editor = editor("one one\none\n");
        let mut bar = Find::new();
        bar.open(&editor);
        type_in(&mut bar, &mut editor, "one");
        // Off until it is pressed, as the window's Mark tab is: a search that
        // filled the margin without being asked would be a search tidying up
        // after itself.
        assert!(!bar.bookmark_lines);
        bar.focus = Focus::Bookmark;
        bar.handle(Key::Char(' '), &mut editor);
        assert!(bar.bookmark_lines);
        bar.step(&mut editor, true);
        assert!(editor.doc.is_bookmarked(0), "the first match marked nothing");
        assert_eq!(editor.doc.bookmark_count(), 1);
        // Two matches on one line are one dot, not a toggle that erases it.
        bar.step(&mut editor, true);
        assert!(editor.doc.is_bookmarked(0), "the dot was toggled off again");
        assert_eq!(editor.doc.bookmark_count(), 1);
        // The third match is on the line below, which gets a dot of its own.
        bar.step(&mut editor, true);
        assert!(editor.doc.is_bookmarked(1));
        assert_eq!(editor.doc.bookmark_count(), 2);
    }

    #[test]
    fn the_purge_chip_drops_the_previous_searchs_marks_before_this_one() {
        let mut editor = editor("one\ntwo\nthree\none\n");
        // A dot the search before this one left behind.
        editor.doc.toggle_bookmark(2);
        let mut bar = Find::new();
        bar.open(&editor);
        type_in(&mut bar, &mut editor, "one");
        bar.focus = Focus::Purge;
        bar.handle(Key::Char(' '), &mut editor);
        bar.focus = Focus::Bookmark;
        bar.handle(Key::Char(' '), &mut editor);
        assert!(bar.purge_marks && bar.bookmark_lines);
        bar.step(&mut editor, true);
        // The old dot is gone and this search's first is in its place.
        assert!(!editor.doc.is_bookmarked(2), "the old mark survived");
        assert!(editor.doc.is_bookmarked(0));
        assert_eq!(editor.doc.bookmark_count(), 1);
        // And the purge does not run again: the dot the last step made stays.
        bar.step(&mut editor, true);
        assert!(editor.doc.is_bookmarked(0), "the purge ran a second time");
        assert!(editor.doc.is_bookmarked(3));
        assert_eq!(editor.doc.bookmark_count(), 2);
    }

    /// The bar's row, as a string.
    fn drawn(bar: &Find, width: usize) -> String {
        let mut grid = Grid::new(width, 1);
        bar.draw(&mut grid, 0, width);
        (0..width).map(|column| grid.get(0, column).ch).collect()
    }

    /// The bar's rows, as strings.
    fn drawn_rows(bar: &Find, width: usize, rows: usize) -> Vec<String> {
        let mut grid = Grid::new(width, rows);
        bar.draw(&mut grid, 0, width);
        (0..rows)
            .map(|row| (0..width).map(|column| grid.get(row, column).ch).collect())
            .collect()
    }
}
