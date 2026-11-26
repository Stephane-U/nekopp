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

use nn_shared::i18n::{Catalog, Language};
use nn_shared::prefs::{self, Row, ROWS};
use nn_shared::settings::Settings;
use nn_shared::theme::ThemeChoice;

use crate::keys::Key;
use crate::palette::Tone;
use crate::screen::Grid;

/// The title on the box, and the keys along its bottom edge.
///
/// Both are the window's own sentences, so the catalogue carries them and a
/// translation covers this dialog as it covers that one.
const TITLE: &str = "Preferences";
const HINT: &str = "Up/Down  Left/Right  Enter open  Esc close";

/// How many entries of a list are shown at once, before it starts scrolling.
const PICKER_ROWS: usize = 12;

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

/// A list the dialog opened over itself.
///
/// The two rows that offer fifty values and twenty-four -- the theme and the
/// language -- step on Left and Right and open this on Enter, which is what the
/// window does with them too: a list you can read beats fifty presses of an
/// arrow.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Picker {
    /// The row it belongs to, which is what it is titled and what it writes to.
    row: Row,
    /// What it offers, top to bottom.
    items: Vec<String>,
    /// Which entry is lit.
    selected: usize,
}

impl Picker {
    /// The list `row` offers, with the entry in force lit.
    fn open(row: Row, settings: &Settings, languages: &[(String, String)]) -> Picker {
        let items: Vec<String> = match row {
            Row::Theme => ThemeChoice::ALL
                .iter()
                .map(|choice| choice.label().to_string())
                .collect(),
            _ => languages.iter().map(|(_, name)| name.clone()).collect(),
        };
        let selected = match row {
            Row::Theme => ThemeChoice::ALL
                .iter()
                .position(|choice| *choice == settings.theme)
                .unwrap_or(0),
            _ => languages
                .iter()
                .position(|(file, _)| {
                    file.as_str() == settings.language.as_deref().unwrap_or("")
                })
                .unwrap_or(0),
        };
        Picker {
            row,
            items,
            selected,
        }
    }

    /// The first entry drawn, chosen so the lit one is always in the window.
    fn first(&self) -> usize {
        if self.selected < PICKER_ROWS {
            0
        } else {
            self.selected + 1 - PICKER_ROWS
        }
    }

    /// Writes the lit entry into `settings`.
    fn choose(&self, settings: &mut Settings, languages: &[(String, String)]) {
        match self.row {
            Row::Theme => {
                if let Some(choice) = ThemeChoice::ALL.get(self.selected) {
                    settings.theme = *choice;
                }
            }
            _ => {
                if let Some((file, _)) = languages.get(self.selected) {
                    settings.language = if file.is_empty() {
                        None
                    } else {
                        Some(file.clone())
                    };
                }
            }
        }
    }
}

/// The dialog: whether it is up, which row is lit, and the words it reads them
/// through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefs {
    open: bool,
    selected: usize,
    /// The catalogue the labels and values are read through, or `None` for
    /// English.
    catalog: Option<Catalog>,
    /// The languages the Language row steps through, English first.
    languages: Vec<(String, String)>,
    /// The list a row opened, while it is up.
    picker: Option<Picker>,
}

impl Prefs {
    /// A closed dialog on its first row.
    pub fn new() -> Self {
        Prefs {
            open: false,
            selected: 0,
            catalog: None,
            languages: vec![(String::new(), "English".to_string())],
            picker: None,
        }
    }

    /// Swaps in a catalogue, or clears it back to English with `None`.
    pub fn set_catalog(&mut self, catalog: Option<Catalog>) {
        self.catalog = catalog;
    }

    /// Sets the languages the Language row offers, English first.
    ///
    /// The window's shape, built from the same list: `("", "English")` for the
    /// untranslated one, then the English name of each installed file, so a
    /// reader looking for Spanish finds it under `S` whatever it calls itself.
    pub fn set_languages(&mut self, list: &[Language]) {
        self.languages = std::iter::once((String::new(), "English".to_string()))
            .chain(list.iter().map(|one| (one.file.clone(), one.english.clone())))
            .collect();
    }

    /// The translation of `english`, or `english` itself.
    fn tr<'a>(&'a self, english: &'a str) -> &'a str {
        match &self.catalog {
            Some(catalog) => catalog.tr(english),
            None => english,
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
        // A list that is up owns the keys: the dialog behind it is not what the
        // user is looking at, and an arrow that moved both would move the row
        // under a list they are choosing from.
        if self.picker.is_some() {
            self.handle_picker(key, settings);
            return true;
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
                prefs::adjust(settings, row, -1, &self.languages);
            }
            // Enter opens the list for the two rows that have one, because
            // stepping through fifty themes is not a way to find one.
            Key::Enter if row.opens_picker() && unhonoured(row).is_none() => {
                self.picker = Some(Picker::open(row, settings, &self.languages));
            }
            Key::Right | Key::Enter if unhonoured(row).is_none() => {
                prefs::adjust(settings, row, 1, &self.languages);
            }
            _ => return true,
        }
        true
    }

    /// Handles a key while a list is up.
    ///
    /// Escape closes the list and leaves the setting as it was, which is what
    /// makes it safe to open one to look: nothing is written until Enter.
    fn handle_picker(&mut self, key: Key, settings: &mut Settings) {
        let Some(picker) = &mut self.picker else {
            return;
        };
        match key {
            Key::Escape => self.picker = None,
            Key::Up => picker.selected = picker.selected.saturating_sub(1),
            Key::Down => {
                picker.selected = (picker.selected + 1).min(picker.items.len().saturating_sub(1))
            }
            Key::PageUp => picker.selected = 0,
            Key::PageDown => picker.selected = picker.items.len().saturating_sub(1),
            Key::Enter => {
                let chosen = picker.clone();
                chosen.choose(settings, &self.languages);
                self.picker = None;
            }
            _ => {}
        }
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
            .map(|row| self.tr(row.label()).chars().count())
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

        draw_box(grid, left, top, box_width, box_height, self.tr(TITLE));
        let hint_at = top + box_height - 1;
        let hint = self.tr(HINT);
        let hint_column = left + box_width.saturating_sub(2 + hint.chars().count());
        grid.text(hint_at, hint_column, hint, Tone::MenuHover);

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
            grid.text(at, left + 2, self.tr(row.label()), tone);
            let value = self.value(settings, *row);
            let value_column = left + box_width - 2 - value.chars().count();
            grid.text(at, value_column, &value, tone);
        }

        // And the list over everything, because it is what the user is looking
        // at: the rows behind it are context, not a second place to press keys.
        if let Some(picker) = &self.picker {
            self.draw_picker(grid, picker);
        }
    }

    /// Draws the list a row opened, centred over the dialog.
    fn draw_picker(&self, grid: &mut Grid, picker: &Picker) {
        let width = grid.width();
        let height = grid.height();
        let inner = picker
            .items
            .iter()
            .map(|item| item.chars().count())
            .max()
            .unwrap_or(0);
        // One column of padding each side and one of border: the same shape as
        // the dialog's own box, so a list reads as part of it.
        let box_width = (inner + 4).min(width.saturating_sub(4)).max(10.min(width));
        let shown = picker.items.len().min(PICKER_ROWS);
        let box_height = shown + 2;
        if box_height > height.saturating_sub(2) || box_width < 8 {
            return;
        }
        let left = (width - box_width) / 2;
        let top = (height - box_height) / 2;
        draw_box(grid, left, top, box_width, box_height, self.tr(picker.row.label()));
        let first = picker.first();
        for (offset, item) in picker.items.iter().skip(first).take(shown).enumerate() {
            let at = top + 1 + offset;
            let lit = first + offset == picker.selected;
            let tone = if lit { Tone::MenuHover } else { Tone::Menu };
            for column in left + 1..left + box_width - 1 {
                grid.put(at, column, ' ', tone);
            }
            grid.text(at, left + 2, item, tone);
        }
    }

    /// What a row reads on the right: the shared answer, or why there is none.
    ///
    /// The words come through the catalogue -- `On`, `Never`, `Edit...` -- and
    /// the numbers keep their units, because [`prefs::value_of`] is asked for the
    /// word and not for the finished line.
    fn value(&self, settings: &Settings, row: Row) -> String {
        match unhonoured(row) {
            Some(reason) => self.tr(reason).to_string(),
            None => prefs::value_of(settings, row, &self.languages, &|text| {
                self.tr(text).to_string()
            }),
        }
    }
}

/// The border of a box with `title` written over its top edge.
fn draw_box(grid: &mut Grid, left: usize, top: usize, width: usize, height: usize, title: &str) {
    for column in left..left + width {
        grid.put(top, column, '─', Tone::MenuBorder);
        grid.put(top + height - 1, column, '─', Tone::MenuBorder);
    }
    for row_at in top..top + height {
        grid.put(row_at, left, '│', Tone::MenuBorder);
        grid.put(row_at, left + width - 1, '│', Tone::MenuBorder);
    }
    grid.put(top, left, '┌', Tone::MenuBorder);
    grid.put(top, left + width - 1, '┐', Tone::MenuBorder);
    grid.put(top + height - 1, left, '└', Tone::MenuBorder);
    grid.put(top + height - 1, left + width - 1, '┘', Tone::MenuBorder);
    // One space of padding each side, which is what keeps a title from running
    // into the corner.
    grid.text(top, left + 1, &format!(" {title} "), Tone::MenuHover);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog() -> Prefs {
        let mut prefs = Prefs::new();
        prefs.open();
        prefs
    }

    /// What the dialog puts on an 80 by 24 grid, as text.
    fn screen(prefs: &Prefs, settings: &Settings) -> String {
        let mut grid = Grid::new(80, 24);
        prefs.draw(&mut grid, settings);
        (0..24)
            .map(|row| (0..80).map(|c| grid.get(row, c).ch).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A catalogue written in this editor's own schema: one file, no ids.
    fn french() -> Catalog {
        let xml = concat!(
            "<NotepadPlus><Native-Langue name=\"Francais\" english=\"French\" ",
            "filename=\"french.xml\"><Menu><Main>",
            "<Item id=\"Theme\" name=\"Theme\"/>",
            "<Item id=\"Tab Width\" name=\"Tabulacion\"/>",
            "<Item id=\"On\" name=\"Oui\"/>",
            "<Item id=\"Off\" name=\"Non\"/>",
            "<Item id=\"Preferences\" name=\"Preferences (fr)\"/>",
            "</Main></Menu></Native-Langue></NotepadPlus>",
        );
        Catalog::parse(None, xml).expect("a catalog")
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

    #[test]
    fn a_catalogue_translates_the_labels_and_the_words() {
        let mut prefs = dialog();
        prefs.set_catalog(Some(french()));
        let settings = Settings::default();
        let text = screen(&prefs, &settings);
        // The box's title, the row's own label, and the word a switch offers.
        assert!(text.contains("Preferences (fr)"), "{text}");
        assert!(text.contains("Tabulacion"), "{text}");
        assert!(text.contains("Oui"), "{text}");
        // What the catalogue does not cover stays English rather than blank.
        assert!(text.contains("Autosave"), "{text}");
    }

    #[test]
    fn enter_on_a_list_row_opens_the_list() {
        let mut prefs = dialog();
        let mut settings = Settings::default();
        // The first row is the theme, which has fifty values and a list.
        prefs.handle(Key::Enter, &mut settings);
        let picker = prefs.picker.as_ref().expect("a list");
        assert_eq!(picker.row, Row::Theme);
        assert_eq!(picker.items.len(), ThemeChoice::ALL.len());
        // It opens on the value in force rather than at the top.
        assert_eq!(picker.selected, 0);
        // And the dialog is still up behind it.
        assert!(prefs.is_open());
    }

    #[test]
    fn choosing_from_the_list_sets_the_value_and_closes_it() {
        let mut prefs = dialog();
        let mut settings = Settings::default();
        prefs.handle(Key::Enter, &mut settings);
        prefs.handle(Key::Down, &mut settings);
        prefs.handle(Key::Enter, &mut settings);
        assert!(prefs.picker.is_none(), "the list stayed up");
        assert_eq!(settings.theme, ThemeChoice::ALL[1]);
        assert!(prefs.is_open(), "choosing a theme closed the dialog too");
    }

    #[test]
    fn escape_leaves_the_list_without_changing_anything() {
        let mut prefs = dialog();
        let mut settings = Settings::default();
        let before = settings.theme;
        prefs.handle(Key::Enter, &mut settings);
        prefs.handle(Key::Down, &mut settings);
        prefs.handle(Key::Escape, &mut settings);
        assert!(prefs.picker.is_none());
        assert_eq!(settings.theme, before, "an escape wrote a value");
        // The dialog is still up: one escape closes the list, not both.
        assert!(prefs.is_open());
    }

    #[test]
    fn the_language_list_offers_english_first_and_chooses_none() {
        let mut prefs = dialog();
        prefs.set_languages(&[Language {
            name: "Francais".to_string(),
            english: "French".to_string(),
            file: "french.xml".to_string(),
        }]);
        let mut settings = Settings {
            language: Some("french.xml".to_string()),
            ..Settings::default()
        };
        prefs.set_selected(15); // the Language row
        prefs.handle(Key::Enter, &mut settings);
        let picker = prefs.picker.as_ref().expect("a list");
        assert_eq!(picker.items, ["English", "French"]);
        // It opened on the language in force, which is the second entry.
        assert_eq!(picker.selected, 1);
        prefs.handle(Key::PageUp, &mut settings);
        prefs.handle(Key::Enter, &mut settings);
        assert_eq!(settings.language, None, "English is the empty file name");
    }

    #[test]
    fn the_list_is_drawn_over_the_dialog_with_its_entries() {
        let mut prefs = dialog();
        let mut settings = Settings::default();
        prefs.handle(Key::Enter, &mut settings);
        let text = screen(&prefs, &settings);
        // The list's title is the row's label, and its entries are the themes.
        assert!(text.contains("Theme"), "{text}");
        assert!(text.contains("System"), "{text}");
        assert!(text.contains("Dark"), "{text}");
    }

    #[test]
    fn enter_still_steps_a_row_that_has_no_list() {
        let mut prefs = dialog();
        let mut settings = Settings::default();
        prefs.set_selected(4); // word wrap, a switch
        let before = settings.wrap;
        prefs.handle(Key::Enter, &mut settings);
        assert_ne!(settings.wrap, before);
        assert!(prefs.picker.is_none(), "a switch opened a list");
    }

    #[test]
    fn the_language_row_steps_through_what_is_installed() {
        let mut prefs = dialog();
        prefs.set_languages(&[
            Language {
                name: "Francais".to_string(),
                english: "French".to_string(),
                file: "french.xml".to_string(),
            },
            Language {
                name: "Espanol".to_string(),
                english: "Spanish".to_string(),
                file: "spanish.xml".to_string(),
            },
        ]);
        let mut settings = Settings::default();
        assert_eq!(prefs.value(&settings, Row::Language), "English");
        prefs.set_selected(15); // the Language row
        prefs.handle(Key::Right, &mut settings);
        assert_eq!(settings.language.as_deref(), Some("french.xml"));
        assert_eq!(prefs.value(&settings, Row::Language), "French");
        // And it stops at the end of the list rather than wrapping to English.
        prefs.handle(Key::Right, &mut settings);
        prefs.handle(Key::Right, &mut settings);
        assert_eq!(settings.language.as_deref(), Some("spanish.xml"));
    }
}
