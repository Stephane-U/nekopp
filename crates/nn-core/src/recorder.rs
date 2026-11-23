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

//! Macro recording.
//!
//! Notepad++ records keystrokes and replays them. This records the *editor calls*
//! those keystrokes produce, which is the same thing with the ambiguity removed:
//! a recorded `Insert("a")` means "insert an a at every caret", not "press the
//! key labelled A", so a macro recorded on a US layout replays correctly on an
//! AZERTY one.
//!
//! Replay goes through the same `Editor` methods the keyboard does, so a macro
//! cannot do anything a person could not, and it inherits every guarantee those
//! methods make — undo grouping, caret clamping, selection normalisation.

use crate::editor::{Editor, Motion};

/// One recorded editor call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Insert this text at every caret.
    Insert(String),
    /// Replace the character under the caret with this one.
    ///
    /// Distinct from `Insert` because overwrite mode is a mode: a macro
    /// recorded while it was on has to replay the same way, or the recording
    /// is a lie about what happened.
    Overwrite(char),
    /// Delete the character before the caret.
    Backspace,
    /// Delete the character after the caret.
    Delete,
    /// Delete every selection.
    DeleteSelection,
    /// Move the caret, optionally extending the selection.
    Move(Motion, bool),
    /// Select the whole document.
    SelectAll,
    /// Undo one step.
    Undo,
    /// Redo one step.
    Redo,
}

/// A recorded sequence of steps.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Macro {
    steps: Vec<Step>,
    recording: bool,
}

impl Macro {
    /// An empty, idle macro.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a recording is in progress.
    pub fn is_recording(&self) -> bool {
        self.recording
    }

    /// Starts recording, discarding whatever was recorded before.
    ///
    /// Notepad++ replaces the macro on every recording, and so does this: a
    /// recorder that silently appended would be impossible to reason about.
    pub fn start(&mut self) {
        self.steps.clear();
        self.recording = true;
    }

    /// Stops recording, keeping the steps.
    pub fn stop(&mut self) {
        self.recording = false;
    }

    /// Records `step` if a recording is in progress.
    pub fn record(&mut self, step: Step) {
        if self.recording {
            self.steps.push(step);
        }
    }

    /// The recorded steps.
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// How many steps are recorded.
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Whether nothing is recorded.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Replays every step against `editor`.
    ///
    /// Returns how many steps ran. A macro that is still recording is not
    /// replayed: playing it back into itself would never terminate.
    pub fn play(&self, editor: &mut Editor) -> usize {
        if self.recording {
            return 0;
        }
        for step in &self.steps {
            match step {
                Step::Insert(text) => editor.insert(text),
                Step::Overwrite(c) => editor.overwrite_char(*c),
                Step::Backspace => editor.backspace(),
                Step::Delete => editor.delete_forward(),
                Step::DeleteSelection => {
                    editor.delete_all_selections();
                }
                Step::Move(motion, extend) => editor.move_caret(*motion, *extend),
                Step::SelectAll => editor.select_all(),
                Step::Undo => {
                    editor.undo();
                }
                Step::Redo => {
                    editor.redo();
                }
            }
        }
        self.steps.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    fn editor(text: &str) -> Editor {
        Editor::new(Document::from_text(text))
    }

    #[test]
    fn nothing_is_recorded_while_idle() {
        let mut m = Macro::new();
        m.record(Step::Insert("a".into()));
        assert!(m.is_empty());
    }

    #[test]
    fn recording_collects_steps() {
        let mut m = Macro::new();
        m.start();
        m.record(Step::Insert("a".into()));
        m.record(Step::Backspace);
        m.stop();
        assert_eq!(m.len(), 2);
        assert!(!m.is_recording());
    }

    #[test]
    fn starting_again_discards_the_old_macro() {
        let mut m = Macro::new();
        m.start();
        m.record(Step::Insert("a".into()));
        m.start();
        assert!(m.is_empty());
    }

    #[test]
    fn replay_reproduces_the_typing() {
        let mut m = Macro::new();
        m.start();
        m.record(Step::Insert("ab".into()));
        m.record(Step::Move(Motion::LineEnd, false));
        m.record(Step::Insert("!".into()));
        m.stop();

        let mut ed = editor("xy");
        assert_eq!(m.play(&mut ed), 3);
        assert_eq!(ed.doc.text(), "abxy!");
    }

    #[test]
    fn replay_is_repeatable() {
        let mut m = Macro::new();
        m.start();
        m.record(Step::Insert("z".into()));
        m.stop();

        let mut ed = editor("");
        m.play(&mut ed);
        m.play(&mut ed);
        assert_eq!(ed.doc.text(), "zz");
    }

    #[test]
    fn a_recording_macro_does_not_play() {
        let mut m = Macro::new();
        m.start();
        m.record(Step::Insert("a".into()));
        let mut ed = editor("");
        assert_eq!(m.play(&mut ed), 0);
        assert_eq!(ed.doc.text(), "");
    }

    #[test]
    fn replay_undoes_as_one_step_per_recorded_edit() {
        let mut m = Macro::new();
        m.start();
        m.record(Step::Insert("a".into()));
        m.record(Step::Insert("b".into()));
        m.stop();

        let mut ed = editor("");
        m.play(&mut ed);
        assert_eq!(ed.doc.text(), "ab");
        ed.undo();
        assert_eq!(ed.doc.text(), "");
    }
}
