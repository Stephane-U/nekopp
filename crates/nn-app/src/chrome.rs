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

//! Which parts of the window furniture are drawn.
//!
//! Distraction-free mode is not a separate window: it is the same window with
//! most of its furniture switched off. That makes it a question of policy rather
//! than of layout, and policy is the part that has to be testable, so it lives
//! here instead of being a handful of `if` statements scattered through the
//! renderer.
//!
//! The mode also caps the width of the text column and centres it. A 34-inch
//! monitor with a 200-character line is not restful to read, and hiding the
//! chrome while leaving the text stretched edge to edge only makes the emptiness
//! more obvious. The cap is a deliberate addition to what Notepad++ does; it is
//! recorded here so it is not mistaken for a bug.

/// How much of the window furniture is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Everything: menu, toolbar, tabs, status bar, panels.
    #[default]
    Normal,
    /// Text only, with the menu bar revealed when the pointer reaches the top.
    DistractionFree,
}

impl Mode {
    /// The name this mode is written as in the settings file.
    ///
    /// Stable, like every other key in this program: a settings file has to keep
    /// meaning the same thing across versions.
    pub fn key(self) -> &'static str {
        match self {
            Mode::Normal => "normal",
            Mode::DistractionFree => "distraction_free",
        }
    }

    /// Reads a name written by [`Mode::key`].
    pub fn from_key(text: &str) -> Option<Mode> {
        Some(match text.trim() {
            "normal" => Mode::Normal,
            "distraction_free" => Mode::DistractionFree,
            _ => return None,
        })
    }

    /// The label the View menu shows.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "Normal",
            Mode::DistractionFree => "Distraction Free",
        }
    }
}

/// The widest the text column is allowed to get, in characters.
///
/// 100 is around the point where a line stops being one eye movement. It is a
/// character count rather than a pixel count so the cap follows the font size:
/// zooming in should not silently widen the column.
pub const MAX_COLUMN: f32 = 100.0;

/// How close to the top edge the pointer has to be for the menu bar to appear.
///
/// Expressed in line heights rather than pixels so it scales with the font. Two
/// lines is enough to reach without aiming and small enough that the bar is gone
/// again as soon as the pointer goes back to the text.
pub const REVEAL_BAND: f32 = 2.0;

/// Which bars are drawn, and how wide the text column is.
///
/// A value rather than a set of flags on `State` because the renderer needs the
/// whole answer at once and the answer has to be consistent: a menu bar that is
/// laid out but not painted leaves a gap, and one that is painted but not laid
/// out covers the first line of text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chrome {
    mode: Mode,
    /// Set while the user is holding the menu open by hand, which keeps it on
    /// screen even when the pointer has left the reveal band.
    menu_pinned: bool,
}

impl Default for Chrome {
    fn default() -> Self {
        Chrome::new(Mode::Normal)
    }
}

impl Chrome {
    /// A chrome policy for `mode`.
    pub fn new(mode: Mode) -> Self {
        Chrome {
            mode,
            menu_pinned: false,
        }
    }

    /// The mode in force.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Switches mode, dropping any pinned menu.
    ///
    /// Dropping the pin matters: leaving distraction-free mode with the menu
    /// pinned would keep it pinned in normal mode, where it is always drawn
    /// anyway, and then pin it again on the way back in.
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        self.menu_pinned = false;
    }

    /// Flips between the two modes.
    pub fn toggle(&mut self) {
        self.set_mode(match self.mode {
            Mode::Normal => Mode::DistractionFree,
            Mode::DistractionFree => Mode::Normal,
        });
    }

    /// Whether the chrome is currently reduced.
    pub fn is_distraction_free(&self) -> bool {
        self.mode == Mode::DistractionFree
    }

    /// Keeps the menu bar on screen regardless of the pointer.
    pub fn pin_menu(&mut self, pinned: bool) {
        self.menu_pinned = pinned;
    }

    /// Whether the menu bar is pinned open.
    pub fn menu_pinned(&self) -> bool {
        self.menu_pinned
    }

    /// Whether the menu bar is drawn, given where the pointer is.
    ///
    /// In normal mode it always is. In distraction-free mode it appears when the
    /// pointer is within [`REVEAL_BAND`] line heights of the top edge, which is
    /// the only way back to the menu without knowing a keyboard shortcut, and it
    /// stays while a dropdown is open so the dropdown is not orphaned.
    pub fn shows_menu(&self, cursor_y: f32, line_height: f32) -> bool {
        if !self.is_distraction_free() {
            return true;
        }
        self.menu_pinned || cursor_y <= line_height * REVEAL_BAND
    }

    /// Whether the toolbar is drawn.
    pub fn shows_toolbar(&self) -> bool {
        !self.is_distraction_free()
    }

    /// Whether the tab strips are drawn.
    pub fn shows_tabs(&self) -> bool {
        !self.is_distraction_free()
    }

    /// Whether the status bar is drawn.
    pub fn shows_status(&self) -> bool {
        !self.is_distraction_free()
    }

    /// Whether the prompt bar is drawn.
    ///
    /// The prompt is a modal question ("Save changes?"), so it is drawn even in
    /// distraction-free mode: hiding it would leave the program waiting for an
    /// answer the user cannot see.
    pub fn shows_prompt(&self) -> bool {
        true
    }

    /// Whether the results panel is drawn.
    ///
    /// Hidden with the rest of the chrome, but only while it is empty: a search
    /// that has just finished has results the user asked for, and hiding them
    /// would make the search look like it did nothing.
    pub fn shows_results(&self, has_results: bool) -> bool {
        !self.is_distraction_free() || has_results
    }

    /// Whether the change-history panel is drawn.
    ///
    /// The same rule as the results panel: hidden with the rest of the chrome,
    /// but only while it is empty. A history the user just opened is content
    /// they asked for.
    pub fn shows_history(&self, has_rows: bool) -> bool {
        !self.is_distraction_free() || has_rows
    }

    /// Whether the function list is drawn.
    pub fn shows_functions(&self) -> bool {
        !self.is_distraction_free()
    }

    /// Whether the workspace panel is drawn.
    ///
    /// Hidden with the rest of the chrome. A file tree is navigation, and
    /// distraction-free mode is the one mode whose whole point is that there is
    /// nothing to navigate to.
    pub fn shows_folder(&self) -> bool {
        !self.is_distraction_free()
    }

    /// Whether the minimap is drawn.
    pub fn shows_minimap(&self) -> bool {
        !self.is_distraction_free()
    }

    /// The blank margin on each side of the text column, in pixels.
    ///
    /// Zero unless the column would otherwise be wider than [`MAX_COLUMN`]
    /// characters, so a narrow window keeps every pixel it has. The result is
    /// floored to a whole character so the column edge lands on a cell boundary
    /// and the caret does not sit half a glyph off the last column.
    pub fn inset(&self, width: f32, char_width: f32) -> f32 {
        if !self.is_distraction_free() || char_width <= 0.0 {
            return 0.0;
        }
        let cap = MAX_COLUMN * char_width;
        if width <= cap {
            return 0.0;
        }
        let half = (width - cap) / 2.0;
        (half / char_width).floor() * char_width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_mode_shows_everything() {
        let chrome = Chrome::new(Mode::Normal);
        assert!(chrome.shows_menu(9999.0, 20.0));
        assert!(chrome.shows_toolbar());
        assert!(chrome.shows_tabs());
        assert!(chrome.shows_status());
        assert!(chrome.shows_functions());
        assert!(chrome.shows_minimap());
        assert!(!chrome.is_distraction_free());
    }

    #[test]
    fn distraction_free_hides_the_furniture() {
        let chrome = Chrome::new(Mode::DistractionFree);
        assert!(chrome.is_distraction_free());
        assert!(!chrome.shows_toolbar());
        assert!(!chrome.shows_tabs());
        assert!(!chrome.shows_status());
        assert!(!chrome.shows_functions());
        assert!(!chrome.shows_minimap());
    }

    #[test]
    fn the_menu_appears_near_the_top_edge() {
        let chrome = Chrome::new(Mode::DistractionFree);
        // Two line heights is the band, so the top edge and one line in are
        // inside it and three lines down is not.
        assert!(chrome.shows_menu(0.0, 20.0));
        assert!(chrome.shows_menu(20.0, 20.0));
        assert!(chrome.shows_menu(40.0, 20.0));
        assert!(!chrome.shows_menu(41.0, 20.0));
        assert!(!chrome.shows_menu(400.0, 20.0));
    }

    #[test]
    fn a_pinned_menu_stays_put() {
        let mut chrome = Chrome::new(Mode::DistractionFree);
        chrome.pin_menu(true);
        assert!(chrome.menu_pinned());
        assert!(chrome.shows_menu(400.0, 20.0));
        chrome.pin_menu(false);
        assert!(!chrome.shows_menu(400.0, 20.0));
    }

    #[test]
    fn toggling_round_trips_and_drops_the_pin() {
        let mut chrome = Chrome::new(Mode::Normal);
        chrome.toggle();
        assert_eq!(chrome.mode(), Mode::DistractionFree);
        chrome.pin_menu(true);
        chrome.toggle();
        assert_eq!(chrome.mode(), Mode::Normal);
        assert!(!chrome.menu_pinned());
    }

    #[test]
    fn the_prompt_is_never_hidden() {
        // A modal question the user cannot see is a hang.
        assert!(Chrome::new(Mode::DistractionFree).shows_prompt());
        assert!(Chrome::new(Mode::Normal).shows_prompt());
    }

    #[test]
    fn results_survive_when_they_exist() {
        let chrome = Chrome::new(Mode::DistractionFree);
        assert!(!chrome.shows_results(false));
        assert!(chrome.shows_results(true));
        assert!(Chrome::new(Mode::Normal).shows_results(false));
    }

    #[test]
    fn the_history_survives_when_it_has_rows() {
        let chrome = Chrome::new(Mode::DistractionFree);
        assert!(!chrome.shows_history(false));
        assert!(chrome.shows_history(true));
        assert!(Chrome::new(Mode::Normal).shows_history(false));
    }

    #[test]
    fn a_narrow_window_gets_no_inset() {
        let chrome = Chrome::new(Mode::DistractionFree);
        // 50 columns at 8px is 400px, so a 400px window is exactly at the cap.
        assert_eq!(chrome.inset(400.0, 8.0), 0.0);
        assert_eq!(chrome.inset(300.0, 8.0), 0.0);
    }

    #[test]
    fn a_wide_window_centres_the_column() {
        let chrome = Chrome::new(Mode::DistractionFree);
        // 2000px at 8px is 250 columns; the cap is 100, so 800px of text and
        // 600px of margin on each side.
        assert_eq!(chrome.inset(2000.0, 8.0), 600.0);
        // The column that is left is the cap, not more.
        assert_eq!(2000.0 - 2.0 * chrome.inset(2000.0, 8.0), 800.0);
    }

    #[test]
    fn the_inset_lands_on_a_character_boundary() {
        let chrome = Chrome::new(Mode::DistractionFree);
        // 2001px leaves 600.5px a side, which is 75.06 characters: the floor
        // keeps the column edge on a cell so the caret is not half a glyph off.
        let inset = chrome.inset(2001.0, 8.0);
        assert_eq!(inset, 600.0);
        assert_eq!(inset % 8.0, 0.0);
    }

    #[test]
    fn normal_mode_never_insets() {
        assert_eq!(Chrome::new(Mode::Normal).inset(4000.0, 8.0), 0.0);
    }

    #[test]
    fn a_zero_char_width_does_not_divide_by_zero() {
        let chrome = Chrome::new(Mode::DistractionFree);
        assert_eq!(chrome.inset(2000.0, 0.0), 0.0);
    }

    #[test]
    fn the_mode_round_trips_through_its_key() {
        for mode in [Mode::Normal, Mode::DistractionFree] {
            assert_eq!(Mode::from_key(mode.key()), Some(mode));
        }
        assert_eq!(Mode::from_key("nonsense"), None);
        assert_eq!(Mode::from_key("  normal  "), Some(Mode::Normal));
    }

    #[test]
    fn every_mode_has_a_distinct_label() {
        let labels: Vec<&str> = [Mode::Normal, Mode::DistractionFree]
            .iter()
            .map(|m| m.label())
            .collect();
        assert_eq!(labels.len(), 2);
        assert_ne!(labels[0], labels[1]);
    }
}
