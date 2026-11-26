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

//! The rows of the preferences dialog.
//!
//! The dialog is the same dialog in both front-ends: same seventeen settings in
//! the same order, because a user who has learned where Tab Width sits should
//! find it there whichever one they opened. So the order, the labels and -- the
//! part that matters more -- which rows are switches, which step through a value
//! and which open another dialog are decisions made once, here.
//!
//! What a row *holds* is not here. That is the settings themselves, which are
//! still the window's while the shape of what the terminal needs is being
//! learned from the terminal.

/// How many rows the dialog has.
pub const ROWS: usize = 17;

/// The settings the dialog edits, in row order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Theme,
    FontSize,
    UiScale,
    TabWidth,
    Wrap,
    Minimap,
    Toolbar,
    FunctionList,
    VolatileFind,
    FloatingSearch,
    Playground,
    PlaygroundAfter,
    Backup,
    Autosave,
    FileDialog,
    Language,
    Shortcuts,
}

impl Row {
    /// Every row, top to bottom.
    pub const ALL: [Row; ROWS] = [
        Row::Theme,
        Row::FontSize,
        Row::UiScale,
        Row::TabWidth,
        Row::Wrap,
        Row::Minimap,
        Row::Toolbar,
        Row::FunctionList,
        Row::VolatileFind,
        Row::FloatingSearch,
        Row::Playground,
        Row::PlaygroundAfter,
        Row::Backup,
        Row::Autosave,
        Row::FileDialog,
        Row::Language,
        Row::Shortcuts,
    ];

    /// The label drawn on the left.
    pub fn label(self) -> &'static str {
        match self {
            Row::Theme => "Theme",
            Row::FontSize => "Font Size",
            Row::UiScale => "Window Scale",
            Row::TabWidth => "Tab Width",
            Row::Wrap => "Word Wrap",
            Row::Minimap => "Minimap",
            Row::Toolbar => "Toolbar",
            Row::FunctionList => "Function List",
            Row::VolatileFind => "Volatile Find",
            Row::FloatingSearch => "Floating Search",
            Row::Playground => "Playground",
            Row::PlaygroundAfter => "Playground After",
            Row::Backup => "Backup on Save",
            Row::Autosave => "Autosave",
            Row::FileDialog => "File Dialog",
            Row::Language => "Language",
            Row::Shortcuts => "Shortcuts",
        }
    }

    /// Whether the row is a switch rather than a number.
    ///
    /// The theme is not one: it has fifty values, and a switch would have to
    /// pretend that "follow the desktop" is the off position.
    pub fn is_switch(self) -> bool {
        matches!(
            self,
            Row::Wrap
                | Row::Minimap
                | Row::Toolbar
                | Row::FunctionList
                | Row::VolatileFind
                | Row::FloatingSearch
                | Row::Playground
                | Row::Backup
                | Row::FileDialog
        )
    }

    /// Whether the row opens another dialog rather than holding a value.
    ///
    /// An action row has nothing to step through, so Left and Right do nothing
    /// on it and Enter is the only way in.
    pub fn is_action(self) -> bool {
        matches!(self, Row::Shortcuts)
    }

    /// Whether the row opens a list, rather than stepping through its values.
    ///
    /// The two rows that do are the two with more values than a user can hold in
    /// their head: fifty themes and however many languages are installed.
    /// Stepping through fifty values to find Tokyo Night is a dialog you cannot
    /// use, and the same row with a list is a dialog you can.
    pub fn opens_picker(self) -> bool {
        matches!(self, Row::Theme | Row::Language)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_appears_once() {
        assert_eq!(Row::ALL.len(), ROWS);
        for (at, row) in Row::ALL.iter().enumerate() {
            assert!(
                !Row::ALL[..at].contains(row),
                "{row:?} is in the list twice",
            );
        }
    }

    #[test]
    fn no_two_rows_are_called_the_same_thing() {
        // The label is how a user finds a setting, and two rows reading alike
        // would be two rows to try before you found the one you wanted.
        for (at, row) in Row::ALL.iter().enumerate() {
            assert!(!row.label().is_empty(), "{row:?} has no label");
            assert!(
                !Row::ALL[..at].iter().any(|other| other.label() == row.label()),
                "{row:?} shares its label with an earlier row",
            );
        }
    }

    #[test]
    fn a_row_is_a_switch_or_a_step_or_an_action_but_only_one() {
        // The dialog reads these three to decide what Left and Right and Enter
        // do, so a row that answered yes to two of them would be a key pressed
        // to no effect.
        for row in Row::ALL {
            assert!(!(row.is_switch() && row.is_action()), "{row:?}");
            assert!(!(row.is_switch() && row.opens_picker()), "{row:?}");
            assert!(!(row.is_action() && row.opens_picker()), "{row:?}");
        }
    }
}
