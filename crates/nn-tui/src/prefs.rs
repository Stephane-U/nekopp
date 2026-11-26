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

//! The preferences dialog.
//!
//! The same seventeen rows, in the same order, offering the same values as the
//! window's -- because the rows, the values and the steps between them are all
//! in `nn-shared`, and this file is only what is left when the drawing is a grid
//! of cells instead of a shader: a box, a lit row, and four keys.
//!
//! Two of the seventeen are the window's alone and are shown as such rather than
//! hidden. A terminal has no font to size and no window to scale, but a user who
//! opens both dialogs should find the same list in the same order, and a row
//! that quietly disappeared from one of them would be a worse answer than a row
//! that says it does not apply.

use nn_shared::prefs::{self, Row, ROWS};
use nn_shared::settings::Settings;

use crate::keys::Key;
use crate::palette::Tone;
use crate::screen::Grid;

/// The title on the box, and the keys along its bottom edge.
const TITLE: &str = " Preferences ";
const HINT: &str = " Up/Down  Left/Right  Enter  Esc ";

/// The rows a terminal cannot honour, and what it says instead of a value.
///
/// The font size is pixels and the window scale is a multiplier on a font; a
/// terminal is told its cell size by the terminal. The row is still drawn, with
/// its reason on the right, so the list is the same length in both dialogs.
fn unhonoured(row: Row) -> Option<&'static str> {
    match row {
        Row::FontSize => Some("the terminal's font"),
        Row::UiScale => Some("follows the display"),
        _ => None,
    }
}

/// The dialog: whether it is up, and which row is lit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prefs {
    open: bool,
    selected: usize,
}

impl Prefs {
    /// A closed dialog on its first row.
    pub fn new() -> Self {
        Prefs {
            open: false,
            selected: 0,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self) {
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// The row that is lit. Only the tests ask: the shell draws the lit row
    /// from the field itself.
    #[cfg(test)]
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Which row is lit, clamped to the list. Only the tests ask.
    #[cfg(test)]
    pub fn set_selected(&mut self, index: usize) {
        self.selected = index.min(ROWS - 1);
    }

    /// Handles one key. `true` when the dialog took it.
    ///
    /// Everything it took, it took: an arrow that ran off the end of the list
    /// still counts as the dialog's key rather than the document's, which is the
    /// rule a modal has to follow or the text underneath starts moving.
    pub fn handle(&mut self, key: Key, settings: &mut Settings) -> bool {
        if !self.open {
            return false;
        }
        let row = Row::ALL[self.selected];
        match key {
            Key::Escape => self.close(),
            Key::Up => self.selected = self.selected.saturating_sub(1),
            Key::Down => self.selected = (self.selected + 1).min(ROWS - 1),
            Key::PageUp => self.selected = 0,
            Key::PageDown => self.selected = ROWS - 1,
            // A row the terminal cannot honour takes the key and does nothing
            // with it. Stepping it would write a font size this program cannot
            // use into the file the window reads, so a key pressed here would
            // resize text somewhere else.
            Key::Left if unhonoured(row).is_none() => {
                prefs::adjust(settings, row, -1, &languages());
            }
            Key::Right | Key::Enter if unhonoured(row).is_none() => {
                prefs::adjust(settings, row, 1, &languages());
            }
            _ => return true,
        }
        true
    }

    /// Draws the box over whatever is on the grid.
    pub fn draw(&self, grid: &mut Grid, settings: &Settings) {
        if !self.open {
            return;
        }
        let width = grid.width();
        let height = grid.height();
        let labels = Row::ALL
            .iter()
            .map(|row| row.label().chars().count())
            .max()
            .unwrap_or(0);
        let widest = Row::ALL
            .iter()
            .map(|row| self.value(settings, *row).chars().count())
            .max()
            .unwrap_or(0);
        // One column of padding each side, one of border, and two between the
        // label and its value -- the same proportions the dropdowns use.
        let inner = labels + widest + 4;
        let box_width = (inner + 2).min(width.saturating_sub(2)).max(24.min(width));
        let box_height = (ROWS + 4).min(height.saturating_sub(2));
        if box_height < 5 || box_width < 12 {
            return;
        }
        let left = (width - box_width) / 2;
        let top = (height - box_height) / 2;

        for column in left..left + box_width {
            grid.put(top, column, '─', Tone::MenuBorder);
            grid.put(top + box_height - 1, column, '─', Tone::MenuBorder);
        }
        for row_at in top..top + box_height {
            grid.put(row_at, left, '│', Tone::MenuBorder);
            grid.put(row_at, left + box_width - 1, '│', Tone::MenuBorder);
        }
        grid.put(top, left, '┌', Tone::MenuBorder);
        grid.put(top, left + box_width - 1, '┐', Tone::MenuBorder);
        grid.put(top + box_height - 1, left, '└', Tone::MenuBorder);
        grid.put(top + box_height - 1, left + box_width - 1, '┘', Tone::MenuBorder);

        // The title, written over the top edge.
        grid.text(top, left + 2, TITLE, Tone::MenuHover);
        let hint_at = top + box_height - 1;
        let hint_column = left + box_width.saturating_sub(2 + HINT.chars().count());
        grid.text(hint_at, hint_column, HINT, Tone::MenuHover);

        for (index, row) in Row::ALL.iter().enumerate() {
            let at = top + 2 + index;
            if at >= hint_at {
                break;
            }
            let lit = index == self.selected;
            let tone = if lit { Tone::MenuHover } else { Tone::Menu };
            for column in left + 1..left + box_width - 1 {
                grid.put(at, column, ' ', tone);
            }
            grid.text(at, left + 2, row.label(), tone);
            let value = self.value(settings, *row);
            let value_column = left + box_width - 2 - value.chars().count();
            grid.text(at, value_column, &value, tone);
        }
    }

    /// What a row reads on the right: the shared answer, or why there is none.
    fn value(&self, settings: &Settings, row: Row) -> String {
        match unhonoured(row) {
            Some(reason) => reason.to_string(),
            None => prefs::value_of(settings, row, &languages(), &|text| text.to_string()),
        }
    }
}

/// The languages the dialog can offer.
///
/// One, because the terminal has no language files yet: the list is the window's
/// shape with the window's default in it, so the row reads `English` rather than
/// blank and the list is not a special case when the reader arrives.
fn languages() -> Vec<(String, String)> {
    vec![(String::new(), "English".to_string())]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog() -> Prefs {
        let mut prefs = Prefs::new();
        prefs.open();
        prefs
    }

    #[test]
    fn a_closed_dialog_takes_no_keys() {
        let mut prefs = Prefs::new();
        let mut settings = Settings::default();
        assert!(!prefs.handle(Key::Down, &mut settings));
        assert_eq!(prefs.selected(), 0);
    }

    #[test]
    fn the_arrows_move_through_every_row_and_stop_at_the_ends() {
        let mut prefs = dialog();
        let mut settings = Settings::default();
        prefs.handle(Key::Up, &mut settings);
        assert_eq!(prefs.selected(), 0);
        for _ in 0..ROWS + 5 {
            prefs.handle(Key::Down, &mut settings);
        }
        assert_eq!(prefs.selected(), ROWS - 1);
        prefs.handle(Key::PageUp, &mut settings);
        assert_eq!(prefs.selected(), 0);
    }

    #[test]
    fn right_and_left_step_the_lit_row_and_nothing_else() {
        let mut prefs = dialog();
        let mut settings = Settings::default();
        // The third row is the tab width.
        prefs.set_selected(3);
        let before = settings.tab_width;
        prefs.handle(Key::Right, &mut settings);
        assert_eq!(settings.tab_width, before + 1);
        prefs.handle(Key::Left, &mut settings);
        assert_eq!(settings.tab_width, before);
        // A row it cannot honour leaves the setting alone.
        prefs.set_selected(1);
        prefs.handle(Key::Right, &mut settings);
        assert_eq!(settings.font_size, Settings::default().font_size);
    }

    #[test]
    fn escape_closes_the_dialog() {
        let mut prefs = dialog();
        let mut settings = Settings::default();
        assert!(prefs.handle(Key::Escape, &mut settings));
        assert!(!prefs.is_open());
    }

    #[test]
    fn the_box_is_centred_and_says_which_row_is_lit() {
        let prefs = dialog();
        let settings = Settings::default();
        let mut grid = Grid::new(80, 24);
        prefs.draw(&mut grid, &settings);
        let text: String = (0..24)
            .map(|row| (0..80).map(|c| grid.get(row, c).ch).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Preferences"), "{text}");
        assert!(text.contains("Tab Width"), "{text}");
        assert!(text.contains("the terminal's font"), "{text}");
        // Centred: the box does not touch either edge.
        let first = text.lines().position(|line| line.contains('┌')).unwrap();
        assert!(first > 0, "{first}");
    }

    #[test]
    fn every_row_is_drawn_with_its_value() {
        let prefs = dialog();
        let settings = Settings::default();
        for row in Row::ALL {
            assert!(!prefs.value(&settings, row).is_empty(), "{row:?}");
        }
    }
}
