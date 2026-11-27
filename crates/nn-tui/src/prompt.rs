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

//! The one-line prompt above the status bar.
//!
//! The window has one of these as well, and it is the same widget for every
//! command that wants a line typed before it runs: a label, the text after it,
//! and a caret. `Go to line:` is the one this pass brings, because it is the only
//! command the shell is missing whose answer is a single number; `Open`, `Save
//! As` and `Find in Files` will be rows of the same list rather than three more
//! widgets, which is why the label is a table and the field is not.
//!
//! It is modal while it is up, as the window's is: a letter belongs to the
//! prompt rather than to the document behind it, so a mistyped chord cannot edit
//! a file the reader is not looking at. Escape leaves without doing anything,
//! Enter runs what was typed, and a line number that is not a number runs
//! nothing -- the window's rule, kept rather than improved on, because two
//! front-ends that disagree about silence are two front-ends to learn twice.

use nn_core::Editor;
use nn_shared::i18n::Catalog;

use crate::keys::Key;
use crate::palette::Tone;
use crate::screen::Grid;

/// What the prompt is asking for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A line number, counted from one.
    GotoLine,
}

impl Kind {
    /// The label drawn in front of the field, in English.
    fn label(self) -> &'static str {
        match self {
            Kind::GotoLine => "Go to line:",
        }
    }
}

/// The prompt, what it is asking for, and what has been typed.
pub struct Prompt {
    open: Option<Kind>,
    text: String,
    /// Where the next character goes, counted in characters rather than bytes:
    /// the field holds what a reader typed, and a reader counts characters.
    caret: usize,
    catalog: Option<Catalog>,
}

impl Prompt {
    /// A closed prompt with nothing typed.
    pub fn new() -> Self {
        Prompt {
            open: None,
            text: String::new(),
            caret: 0,
            catalog: None,
        }
    }

    /// Whether the prompt is up, and the text area owes it a row.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Opens the prompt on `kind`, with an empty field.
    ///
    /// The field starts empty rather than holding the last answer: `Go to line`
    /// is asked again about a different line every time, and a number left over
    /// from the last time is a number the reader has to delete first.
    pub fn open(&mut self, kind: Kind) {
        self.open = Some(kind);
        self.text.clear();
        self.caret = 0;
    }

    /// Closes the prompt without running anything.
    pub fn close(&mut self) {
        self.open = None;
    }

    /// The words the prompt draws, in the language the settings name.
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

    /// Handles a key, answering whether it took it.
    ///
    /// Nothing goes back to the shell while the prompt is up: it is modal, and a
    /// key it does not want is dropped rather than handed on to whatever is
    /// behind it.
    pub fn handle(&mut self, key: Key, editor: &mut Editor) -> bool {
        let Some(kind) = self.open else {
            return false;
        };
        match key {
            Key::Escape => self.close(),
            Key::Enter => {
                self.close();
                self.run(kind, editor);
            }
            Key::Char(ch) => {
                let at = self.byte_at(self.caret);
                self.text.insert(at, ch);
                self.caret += 1;
            }
            Key::Backspace => {
                if self.caret > 0 {
                    let at = self.byte_at(self.caret - 1);
                    self.text.remove(at);
                    self.caret -= 1;
                }
            }
            Key::Delete => {
                if self.caret < self.text.chars().count() {
                    let at = self.byte_at(self.caret);
                    self.text.remove(at);
                }
            }
            Key::Left => self.caret = self.caret.saturating_sub(1),
            Key::Right => {
                self.caret = (self.caret + 1).min(self.text.chars().count());
            }
            Key::LineStart => self.caret = 0,
            Key::LineEnd => self.caret = self.text.chars().count(),
            _ => {}
        }
        true
    }

    /// Runs what was typed, for the kind the prompt was opened on.
    fn run(&mut self, kind: Kind, editor: &mut Editor) {
        match kind {
            // Counted from one, as the status bar counts, and clamped by the
            // editor: a line past the end of the file is the last line, which is
            // what the window's prompt does with the same number.
            Kind::GotoLine => {
                if let Ok(line) = self.text.trim().parse::<usize>() {
                    editor.goto_line(line.saturating_sub(1));
                }
            }
        }
    }

    /// The byte offset of the `index`th character of the field.
    fn byte_at(&self, index: usize) -> usize {
        self.text
            .char_indices()
            .nth(index)
            .map_or(self.text.len(), |(at, _)| at)
    }

    /// Draws the prompt on `row`, edge to edge.
    ///
    /// The field runs to the right-hand edge: a line number is three or four
    /// cells, and the room past it is there for the paths and patterns the same
    /// field will carry later.
    pub fn draw(&self, grid: &mut Grid, row: usize, width: usize) {
        for column in 0..width {
            grid.put(row, column, ' ', Tone::StatusBar);
        }
        let label = self.open.map_or("", |kind| kind.label());
        let at = grid.text(row, 0, self.tr(label), Tone::StatusBar) + 1;
        let room = width.saturating_sub(at + 1);
        let field: String = self.text.chars().take(room).collect();
        grid.text(row, at, &field, Tone::StatusBar);
        // The caret, where the next character goes, drawn the way the text area
        // and the find bar draw theirs: one shell, one block.
        let column = at + self.caret.min(room);
        if column < width {
            grid.put(row, column, ' ', Tone::Caret);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nn_core::Document;

    /// An editor on `text`, with the caret at the start.
    fn editor(text: &str) -> Editor {
        Editor::new(Document::from_text(text))
    }

    /// Types `text` into the open prompt.
    fn type_in(prompt: &mut Prompt, editor: &mut Editor, text: &str) {
        for ch in text.chars() {
            prompt.handle(Key::Char(ch), editor);
        }
    }

    #[test]
    fn a_line_number_moves_the_caret_and_is_counted_from_one() {
        let mut editor = editor("one\ntwo\nthree\n");
        let mut prompt = Prompt::new();
        prompt.open(Kind::GotoLine);
        type_in(&mut prompt, &mut editor, "2");
        assert!(prompt.handle(Key::Enter, &mut editor));
        assert!(!prompt.is_open(), "the prompt stayed up after Enter");
        assert_eq!(editor.caret_line(), 1);
        assert_eq!(editor.caret_column(), 0);
    }

    #[test]
    fn a_line_past_the_end_arrives_on_the_last_line() {
        let mut editor = editor("one\ntwo\n");
        let mut prompt = Prompt::new();
        prompt.open(Kind::GotoLine);
        type_in(&mut prompt, &mut editor, "99");
        prompt.handle(Key::Enter, &mut editor);
        // `one\ntwo\n` has three lines, the last of them empty: that is where a
        // number past the end arrives.
        assert_eq!(editor.caret_line(), 2);
        // And a number below one is the first line rather than a wrap round.
        prompt.open(Kind::GotoLine);
        type_in(&mut prompt, &mut editor, "0");
        prompt.handle(Key::Enter, &mut editor);
        assert_eq!(editor.caret_line(), 0);
    }

    #[test]
    fn what_is_not_a_number_moves_nothing_and_escape_runs_nothing() {
        let mut editor = editor("one\ntwo\n");
        let mut prompt = Prompt::new();
        prompt.open(Kind::GotoLine);
        type_in(&mut prompt, &mut editor, "2");
        assert!(prompt.handle(Key::Escape, &mut editor));
        assert_eq!(editor.caret_line(), 0, "Escape ran the command anyway");
        prompt.open(Kind::GotoLine);
        type_in(&mut prompt, &mut editor, "two");
        prompt.handle(Key::Enter, &mut editor);
        assert_eq!(editor.caret_line(), 0);
    }

    #[test]
    fn the_field_takes_the_editing_keys_and_nothing_goes_back_to_the_shell() {
        let mut editor = editor("one\ntwo\nthree\n");
        let mut prompt = Prompt::new();
        prompt.open(Kind::GotoLine);
        // A chord is dropped rather than handed on: the prompt is modal.
        assert!(prompt.handle(Key::Ctrl('q'), &mut editor));
        assert!(prompt.is_open(), "a chord closed the prompt");
        type_in(&mut prompt, &mut editor, "123");
        assert_eq!(prompt.text, "123");
        // A character is inserted where the caret is, not always at the end.
        prompt.handle(Key::Left, &mut editor);
        prompt.handle(Key::Left, &mut editor);
        type_in(&mut prompt, &mut editor, "9");
        assert_eq!(prompt.text, "1923");
        prompt.handle(Key::LineStart, &mut editor);
        prompt.handle(Key::Delete, &mut editor);
        assert_eq!(prompt.text, "923");
        prompt.handle(Key::LineEnd, &mut editor);
        prompt.handle(Key::Backspace, &mut editor);
        assert_eq!(prompt.text, "92");
        // A key the field has no use for is swallowed rather than passed on.
        assert!(prompt.handle(Key::PageDown, &mut editor));
        assert_eq!(editor.caret_line(), 0);
    }

    #[test]
    fn the_label_and_the_caret_hold_their_places_on_the_row() {
        let mut editor = editor("one\n");
        let mut prompt = Prompt::new();
        prompt.open(Kind::GotoLine);
        type_in(&mut prompt, &mut editor, "42");
        let row = drawn(&prompt, 20);
        assert!(row.starts_with("Go to line: 42"), "{row:?}");
        assert_eq!(row.len(), 20, "{row:?}");
        // The caret is a space of its own, one past the number, and the row is
        // painted to its last cell so nothing of the frame behind shows through.
        assert_eq!(&row[12..], "42      ", "{row:?}");
    }

    /// The prompt's row, as a string.
    fn drawn(prompt: &Prompt, width: usize) -> String {
        let mut grid = Grid::new(width, 1);
        prompt.draw(&mut grid, 0, width);
        (0..width).map(|column| grid.get(0, column).ch).collect()
    }
}
