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

//! The one place a command becomes an operation.
//!
//! Both front-ends draw the same two hundred and forty-one commands, from the same
//! tree in [`crate::menu`], with the same labels and the same shortcuts. Each of
//! them also has to *answer* a press the same way: `Save` writes the document,
//! `NextTab` puts the next one in front, `Exit` asks before it drops unsaved work.
//! The drawing cannot be shared -- one writes pixels through a GPU, the other cells
//! through a terminal -- but the answer can be, and until it is, two matches have to
//! be kept in step by hand. That is the drift this module exists to end.
//!
//! [`Host`] is what a front-end has to be able to do for the commands here to run,
//! and [`run`] is the match. It covers the document commands first -- the ones that
//! open, save, close and walk between documents -- because both front-ends have had
//! all twelve for a long time, and because those are the ones where the two drifting
//! apart costs a user work rather than a moment's surprise.
//!
//! The rest of the vocabulary follows, a group a pass, until the last arm makes a
//! new command a compile error rather than a row that does nothing in one of the two
//! programs. Until then a front-end calls [`run`] first and falls through to its own
//! match for what is left.

use crate::action::Action;

/// What a front-end has to be able to do, as the commands here ask for it.
///
/// Every method is an *operation* rather than a command, and deliberately: two
/// commands may end up at one operation later (`Reopen with Encoding` and `Reload`
/// both read a file again), and one command may need more than a method here. What
/// a method may not be is a way of drawing: a name in this trait is a thing that
/// happens to the editor, not a thing that appears on a screen.
pub trait Host {
    /// Opens an empty document in a new tab and puts it in front.
    fn new_document(&mut self);

    /// Asks the person for a file and opens it.
    fn open_document(&mut self);

    /// Writes the document in front back to its file, and asks where when it has
    /// never been saved.
    fn save_document(&mut self);

    /// Writes the document in front to the file the person names.
    fn save_document_as(&mut self);

    /// Closes the document in front, asking about unsaved changes first.
    fn close_document(&mut self);

    /// Closes every document, asking once about all of the unsaved ones.
    fn close_all_documents(&mut self);

    /// Reads the document in front from its file again.
    fn reload_document(&mut self);

    /// Puts the next document in front, wrapping round at the end.
    fn next_document(&mut self);

    /// The same, the other way.
    fn previous_document(&mut self);

    /// Opens the `index`th entry of the recent list, counting from zero.
    ///
    /// An index that is not in the list is not an error: the list can be cleared
    /// between a menu being built and a row being chosen, and a row that opened
    /// nothing is honest about a list that has moved on.
    fn open_recent(&mut self, index: usize);

    /// Empties the recent list.
    fn clear_recent(&mut self);

    /// Leaves the program, writing and asking whatever this front-end has to.
    fn quit(&mut self);
}

/// Runs `action` when it is one of the commands here, and says whether it was.
///
/// A front-end calls this before its own match. `false` means "not mine", and the
/// front-end answers the action the way it always has; `true` means the command has
/// already run, and answering it twice would save a file twice or close two tabs.
pub fn run(action: Action, host: &mut impl Host) -> bool {
    match action {
        Action::New => host.new_document(),
        Action::Open => host.open_document(),
        Action::Save => host.save_document(),
        Action::SaveAs => host.save_document_as(),
        Action::CloseTab => host.close_document(),
        Action::CloseAll => host.close_all_documents(),
        Action::Reload => host.reload_document(),
        Action::NextTab => host.next_document(),
        Action::PrevTab => host.previous_document(),
        Action::OpenRecent(index) => host.open_recent(index),
        Action::ClearRecent => host.clear_recent(),
        Action::Exit => host.quit(),
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host that writes down what it was asked to do.
    #[derive(Debug, Default, PartialEq, Eq)]
    struct Recorder {
        calls: Vec<String>,
    }

    impl Recorder {
        fn note(&mut self, what: &str) {
            self.calls.push(what.to_string());
        }
    }

    impl Host for Recorder {
        fn new_document(&mut self) {
            self.note("new");
        }
        fn open_document(&mut self) {
            self.note("open");
        }
        fn save_document(&mut self) {
            self.note("save");
        }
        fn save_document_as(&mut self) {
            self.note("save as");
        }
        fn close_document(&mut self) {
            self.note("close");
        }
        fn close_all_documents(&mut self) {
            self.note("close all");
        }
        fn reload_document(&mut self) {
            self.note("reload");
        }
        fn next_document(&mut self) {
            self.note("next");
        }
        fn previous_document(&mut self) {
            self.note("previous");
        }
        fn open_recent(&mut self, index: usize) {
            self.note(&format!("recent {index}"));
        }
        fn clear_recent(&mut self) {
            self.note("clear recent");
        }
        fn quit(&mut self) {
            self.note("quit");
        }
    }

    #[test]
    fn every_document_command_reaches_its_own_operation() {
        // One row per command, in the order the answer is checked, so a command that
        // was wired to the wrong operation is a failure here rather than a save that
        // opens a file.
        let cases = [
            (Action::New, "new"),
            (Action::Open, "open"),
            (Action::Save, "save"),
            (Action::SaveAs, "save as"),
            (Action::CloseTab, "close"),
            (Action::CloseAll, "close all"),
            (Action::Reload, "reload"),
            (Action::NextTab, "next"),
            (Action::PrevTab, "previous"),
            (Action::OpenRecent(3), "recent 3"),
            (Action::ClearRecent, "clear recent"),
            (Action::Exit, "quit"),
        ];
        for (action, expected) in cases {
            let mut host = Recorder::default();
            assert!(run(action, &mut host), "{action:?} was not answered");
            assert_eq!(host.calls, [expected.to_string()], "{action:?}");
        }
    }

    #[test]
    fn a_command_from_another_group_is_left_to_the_front_end() {
        // The rest of the vocabulary is still each front-end's own match, and this
        // answers "not mine" for all of it: the alternative is a menu row that runs
        // twice.
        for action in [
            Action::Undo,
            Action::Find,
            Action::Preferences,
            Action::Print,
            Action::Theme(0),
            Action::Submenu,
        ] {
            let mut host = Recorder::default();
            assert!(!run(action, &mut host), "{action:?} was answered here");
            assert!(host.calls.is_empty(), "{action:?} ran something");
        }
    }

    #[test]
    fn the_recent_rows_are_the_only_ones_that_carry_a_number() {
        // A row of the recent list carries its position, so two rows of the same
        // menu are different commands; the operation is told which one was chosen
        // rather than being asked to work it out again.
        let mut host = Recorder::default();
        run(Action::OpenRecent(0), &mut host);
        run(Action::OpenRecent(7), &mut host);
        assert_eq!(host.calls, ["recent 0".to_string(), "recent 7".to_string()]);
    }
}
