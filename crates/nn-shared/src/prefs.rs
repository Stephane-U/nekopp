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

// ---- the two questions a row answers ---------------------------------------

/// What a row shows on its right-hand side.
///
/// Shared because the two dialogs are one dialog: a terminal that wrote `x1.5`
/// where the window writes `x1.5x` would be a second editor with a typo in it.
/// The translation is passed in rather than looked up, because the catalogue of
/// words belongs to the front-end -- and it is asked for the *word* (`On`,
/// `Never`), not for the finished line, so `12 min` keeps its unit instead of
/// going through a dictionary as one string.
pub fn value_of(
    settings: &Settings,
    row: Row,
    languages: &[(String, String)],
    tr: &dyn Fn(&str) -> String,
) -> String {
    let on_off = |value: bool| tr(if value { "On" } else { "Off" });
    match row {
        Row::Theme => tr(settings.theme.label()).to_string(),
        Row::FontSize => format!("{}", settings.font_size),
        Row::UiScale => {
            if settings.ui_scale <= 0.0 {
                tr("Follow the display")
            } else {
                format!("x{}", UiScale(settings.ui_scale).label())
            }
        }
        Row::TabWidth => format!("{}", settings.tab_width),
        Row::Wrap => on_off(settings.wrap),
        Row::Minimap => on_off(settings.minimap),
        Row::Toolbar => on_off(settings.toolbar),
        Row::FunctionList => on_off(settings.function_list),
        Row::VolatileFind => on_off(settings.volatile_find),
        Row::FloatingSearch => on_off(settings.find_floating),
        Row::Playground => on_off(settings.playground),
        Row::PlaygroundAfter => {
            if settings.playground_after == 0 {
                tr("Never")
            } else {
                format!("{} min", settings.playground_after)
            }
        }
        Row::Backup => tr(settings.backup_mode.label()).to_string(),
        Row::Autosave => {
            if settings.autosave_seconds == 0 {
                tr("Off")
            } else {
                format!("{} s", settings.autosave_seconds)
            }
        }
        Row::Language => languages
            .iter()
            .find(|(file, _)| {
                file.as_str() == settings.language.as_deref().unwrap_or("")
            })
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| "English".to_string()),
        Row::FileDialog => tr(settings.file_dialog.label()).to_string(),
        Row::Shortcuts => tr("Edit..."),
    }
}

/// Steps a row, and says whether that changed anything.
///
/// Every number here has a floor and a ceiling and five of them also have a
/// step: five seconds for the autosave, one minute for the playground, one for
/// the tab width. They are in this function rather than in each dialog so that a
/// terminal's arrow key and a window's arrow key land on the same values -- the
/// bounds are what a settings file is written against, and two copies of them
/// would mean two settings files that disagree.
pub fn adjust(
    settings: &mut Settings,
    row: Row,
    delta: i32,
    languages: &[(String, String)],
) -> bool {
    let before = settings.clone();
    match row {
        // Fifty values, so it steps rather than toggles.
        Row::Theme => {
            settings.theme = if delta >= 0 {
                settings.theme.next()
            } else {
                settings.theme.previous()
            };
        }
        Row::FontSize => {
            let size = settings.font_size + delta as f32;
            settings.font_size = size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
        }
        Row::UiScale => {
            // A short list of choices rather than a number: "follow the
            // display" has no numeric form, and the point of the row is to
            // reach a readable size in one or two presses.
            let all = UiScale::ALL;
            let at = all
                .iter()
                .position(|choice| *choice == UiScale(settings.ui_scale))
                .unwrap_or(0) as i32;
            let next = (at + delta).clamp(0, all.len() as i32 - 1) as usize;
            settings.ui_scale = all[next].0;
        }
        Row::TabWidth => {
            let width = settings.tab_width as i32 + delta;
            settings.tab_width =
                width.clamp(MIN_TAB_WIDTH as i32, MAX_TAB_WIDTH as i32) as u16;
        }
        Row::Wrap => settings.wrap = !settings.wrap,
        Row::Minimap => settings.minimap = !settings.minimap,
        Row::Toolbar => settings.toolbar = !settings.toolbar,
        Row::FunctionList => settings.function_list = !settings.function_list,
        Row::VolatileFind => settings.volatile_find = !settings.volatile_find,
        Row::FloatingSearch => settings.find_floating = !settings.find_floating,
        Row::Playground => settings.playground = !settings.playground,
        Row::PlaygroundAfter => {
            // One minute a press: an idle time is not something anyone
            // tunes in five-second steps.
            let minutes = settings.playground_after as i32 + delta;
            settings.playground_after = minutes
                .clamp(MIN_PLAYGROUND_MINUTES as i32, MAX_PLAYGROUND_MINUTES as i32)
                as u32;
        }
        // Three values rather than two, so it steps rather than toggles.
        Row::Backup => {
            settings.backup_mode = if delta >= 0 {
                settings.backup_mode.next()
            } else {
                settings.backup_mode.previous()
            };
        }
        Row::Autosave => {
            // Five-second steps: one second at a time would take sixty
            // presses to reach a minute, and nobody wants a one-second
            // autosave anyway.
            let seconds = settings.autosave_seconds as i32 + delta * 5;
            settings.autosave_seconds =
                seconds.clamp(MIN_AUTOSAVE as i32, MAX_AUTOSAVE as i32) as u32;
        }
        // Two values, but a switch would read as on/off and neither choice
        // is the off one: both dialogs work.
        Row::FileDialog => {
            settings.file_dialog = if delta >= 0 {
                settings.file_dialog.next()
            } else {
                let all = FileDialog::ALL;
                let at = all
                    .iter()
                    .position(|mode| *mode == settings.file_dialog)
                    .unwrap_or(0);
                all[(at + all.len() - 1) % all.len()]
            };
        }
        Row::Language => {
            // Clamped, not wrapped: a language list is short and a wrap
            // from the last entry back to English reads as a mistake.
            let current = languages
                .iter()
                .position(|(file, _)| {
                    file.as_str() == settings.language.as_deref().unwrap_or("")
                })
                .unwrap_or(0) as i32;
            let last = languages.len() as i32 - 1;
            let next = (current + delta).clamp(0, last) as usize;
            let file = languages[next].0.clone();
            settings.language = if file.is_empty() { None } else { Some(file) };
        }
        // Nothing to step through: the row opens a dialog instead.
        Row::Shortcuts => {}
    }
    *settings != before
}

use crate::settings::{FileDialog, Settings, UiScale, MAX_AUTOSAVE, MAX_FONT_SIZE,
    MAX_PLAYGROUND_MINUTES, MAX_TAB_WIDTH, MIN_AUTOSAVE, MIN_FONT_SIZE,
    MIN_PLAYGROUND_MINUTES, MIN_TAB_WIDTH};

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

    /// A translator that does nothing: what the terminal passes.
    fn identity(text: &str) -> String {
        text.to_string()
    }

    #[test]
    fn a_value_reads_the_same_in_both_front_ends() {
        let mut settings = Settings::default();
        let languages = vec![(String::new(), "English".to_string())];
        assert_eq!(
            value_of(&settings, Row::FontSize, &languages, &identity),
            format!("{}", settings.font_size),
        );
        assert_eq!(value_of(&settings, Row::Wrap, &languages, &identity), "Off");
        settings.wrap = true;
        assert_eq!(value_of(&settings, Row::Wrap, &languages, &identity), "On");
        // The units stay outside the translated word.
        settings.autosave_seconds = 30;
        assert_eq!(
            value_of(&settings, Row::Autosave, &languages, &identity),
            "30 s",
        );
        // And a translation is asked for the word, not the finished line.
        let shouted = |text: &str| text.to_uppercase();
        assert_eq!(value_of(&settings, Row::Wrap, &languages, &shouted), "ON");
        assert_eq!(
            value_of(&settings, Row::Autosave, &languages, &shouted),
            "30 s",
        );
    }

    #[test]
    fn stepping_stops_at_the_bounds_whichever_end_it_starts_from() {
        let mut settings = Settings::default();
        let languages = vec![(String::new(), "English".to_string())];
        settings.tab_width = super::super::settings::MIN_TAB_WIDTH;
        assert!(!adjust(&mut settings, Row::TabWidth, -1, &languages));
        assert_eq!(settings.tab_width, super::super::settings::MIN_TAB_WIDTH);
        settings.tab_width = super::super::settings::MAX_TAB_WIDTH;
        assert!(!adjust(&mut settings, Row::TabWidth, 1, &languages));
        assert_eq!(settings.tab_width, super::super::settings::MAX_TAB_WIDTH);
        // A switch ignores the direction and reports the change.
        let before = settings.wrap;
        assert!(adjust(&mut settings, Row::Wrap, 1, &languages));
        assert_eq!(settings.wrap, !before);
        // An action row has nothing to step through.
        assert!(!adjust(&mut settings, Row::Shortcuts, 1, &languages));
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
