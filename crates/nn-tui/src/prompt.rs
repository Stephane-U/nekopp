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
//! As` and `Find in Files` turned out to be rows of the same list rather than
//! three more widgets, which is why the label is a table and the field is not.
//!
//! It is modal while it is up, as the window's is: a letter belongs to the
//! prompt rather than to the document behind it, so a mistyped chord cannot edit
//! a file the reader is not looking at. Escape leaves without doing anything and
//! Enter hands what was typed back to the shell, which is the piece that can open
//! a file and the piece with a status line to answer on. A path that cannot be
//! read, and a line number that is not a number, are the shell's to answer too;
//! keeping that out of here is what lets the field be tested without an editor.

use nn_shared::i18n::Catalog;

use crate::keys::Key;
use crate::palette::Tone;
use crate::screen::Grid;

/// What the prompt is asking for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A file to read.
    Open,
    /// A path to write the document to.
    SaveAs,
    /// A line number, counted from one.
    GotoLine,
    /// A pattern to search the files around the document for.
    FindInFiles,
    /// A folder to show as the workspace tree.
    Workspace,
    /// How many times to play the recorded macro.
    MacroRepeat,
}

impl Kind {
    /// The label drawn in front of the field, in English.
    fn label(self) -> &'static str {
        match self {
            // The window's own words, down to the lowercased `as`: a reader who
            // has used both front-ends should not have to notice they differ.
            Kind::Open => "Open:",
            Kind::SaveAs => "Save as:",
            Kind::GotoLine => "Go to line:",
            Kind::FindInFiles => "Find in files:",
            Kind::Workspace => "Folder:",
            Kind::MacroRepeat => "Play macro times:",
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
    /// What was answered, waiting to be read by the shell.
    answer: Option<(Kind, String)>,
    catalog: Option<Catalog>,
}

impl Prompt {
    /// A closed prompt with nothing typed.
    pub fn new() -> Self {
        Prompt {
            open: None,
            text: String::new(),
            caret: 0,
            answer: None,
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
        self.answer = None;
    }

    /// The answer to a prompt that was just answered, once.
    ///
    /// Reading clears it, so a command that opened a prompt and left it alone
    /// cannot pick up the answer to the one before it.
    pub fn take_answer(&mut self) -> Option<(Kind, String)> {
        self.answer.take()
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
    pub fn handle(&mut self, key: Key) -> bool {
        let Some(kind) = self.open else {
            return false;
        };
        match key {
            Key::Escape => self.close(),
            Key::Enter => {
                // The row goes before the command runs, so a command that says
                // something on the bar is not read with the question still above
                // it, and the answer is left for the shell to collect.
                self.close();
                self.answer = Some((kind, self.text.clone()));
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

    /// Types `text` into the open prompt.
    fn type_in(prompt: &mut Prompt, text: &str) {
        for ch in text.chars() {
            prompt.handle(Key::Char(ch));
        }
    }

    #[test]
    fn an_answer_is_handed_back_once_and_keeps_the_kind() {
        let mut prompt = Prompt::new();
        prompt.open(Kind::GotoLine);
        type_in(&mut prompt, "12");
        assert!(prompt.handle(Key::Enter));
        assert!(!prompt.is_open(), "the prompt stayed up after Enter");
        assert_eq!(prompt.take_answer(), Some((Kind::GotoLine, "12".to_string())));
        // Read once: a command that opens a prompt and walks away cannot pick up
        // the answer to the one before it.
        assert_eq!(prompt.take_answer(), None);
        // And opening the next prompt drops any answer left unread.
        prompt.open(Kind::Open);
        type_in(&mut prompt, "notes.txt");
        prompt.handle(Key::Enter);
        prompt.open(Kind::Open);
        assert_eq!(prompt.take_answer(), None);
    }

    #[test]
    fn escape_answers_nothing_at_all() {
        let mut prompt = Prompt::new();
        prompt.open(Kind::SaveAs);
        type_in(&mut prompt, "/tmp/x.txt");
        assert!(prompt.handle(Key::Escape));
        assert!(!prompt.is_open());
        assert_eq!(prompt.take_answer(), None);
    }

    #[test]
    fn the_field_takes_the_editing_keys_and_nothing_goes_back_to_the_shell() {
        let mut prompt = Prompt::new();
        prompt.open(Kind::Open);
        // A chord is dropped rather than handed on: the prompt is modal.
        assert!(prompt.handle(Key::Ctrl('q')));
        assert!(prompt.is_open(), "a chord closed the prompt");
        type_in(&mut prompt, "123");
        assert_eq!(prompt.text, "123");
        // A character is inserted where the caret is, not always at the end.
        prompt.handle(Key::Left);
        prompt.handle(Key::Left);
        type_in(&mut prompt, "9");
        assert_eq!(prompt.text, "1923");
        prompt.handle(Key::LineStart);
        prompt.handle(Key::Delete);
        assert_eq!(prompt.text, "923");
        prompt.handle(Key::LineEnd);
        prompt.handle(Key::Backspace);
        assert_eq!(prompt.text, "92");
        // A key the field has no use for is swallowed rather than passed on.
        assert!(prompt.handle(Key::PageDown));
        assert_eq!(prompt.text, "92");
    }

    #[test]
    fn the_label_and_the_caret_hold_their_places_on_the_row() {
        let mut prompt = Prompt::new();
        prompt.open(Kind::GotoLine);
        type_in(&mut prompt, "42");
        let row = drawn(&prompt, 20);
        assert!(row.starts_with("Go to line: 42"), "{row:?}");
        assert_eq!(row.len(), 20, "{row:?}");
        // The caret is a space of its own, one past the number, and the row is
        // painted to its last cell so nothing of the frame behind shows through.
        assert_eq!(&row[12..], "42      ", "{row:?}");
        // Each kind asks its own question, in the window's words.
        for (kind, label) in [
            (Kind::Open, "Open:"),
            (Kind::SaveAs, "Save as:"),
            (Kind::GotoLine, "Go to line:"),
            (Kind::Workspace, "Folder:"),
            (Kind::MacroRepeat, "Play macro times:"),
        ] {
            prompt.open(kind);
            assert!(drawn(&prompt, 20).starts_with(label));
        }
    }

    /// The prompt's row, as a string.
    fn drawn(prompt: &Prompt, width: usize) -> String {
        let mut grid = Grid::new(width, 1);
        prompt.draw(&mut grid, 0, width);
        (0..width).map(|column| grid.get(0, column).ch).collect()
    }
}
