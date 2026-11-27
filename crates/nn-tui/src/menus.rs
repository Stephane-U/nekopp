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

//! The menu bar and its dropdowns, in cells.
//!
//! The rows are not written here. They come from `nn_shared::menu`, which is the
//! same tree the windowed front-end shapes glyphs from, so a command added to a
//! menu is on both bars before either program is run -- that is the whole point
//! of that crate. What this module owns is the terminal's share: which menu is
//! open, which row is under the cursor, the chain of submenus, and the drawing of
//! a box with rows in it.
//!
//! The navigation is Midnight Commander's, since the terminal user already knows
//! it: `F9` opens the first title, Left and Right walk the titles while the bar
//! is open, Down opens a title's dropdown, Up and Down walk the rows, Right opens
//! a submenu and Left closes one, Enter runs the row, Esc backs out one level and
//! closes at the top. A key that runs a command the shell does not have yet is
//! not silently ignored: it says so, in the same words the window would use for
//! its label.

use nn_shared::action::Action;
use nn_shared::i18n::Catalog;
use nn_shared::keymap::Keymap;
use nn_shared::menu::{menus, Contents, Item, Menu};

use crate::keys::Key;
use crate::palette::Tone;
use crate::screen::Grid;

/// The character drawn on a row that opens a submenu.
const ARROW: char = '▸';

/// One open dropdown: which menu, and which row of it is highlighted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Open {
    /// The submenu's index in the level above it, and its own index in the bar
    /// for the outermost level.
    at: usize,
    /// The highlighted row, or `None` while a submenu is open beside it.
    row: Option<usize>,
}

/// The bar, the dropdowns, and where the cursor is.
pub struct Menus {
    menus: Vec<Menu>,
    /// The chain of open levels, outermost first. Empty when the bar is closed.
    open: Vec<Open>,
    /// Whether the bar itself has the focus, as opposed to a dropdown.
    bar: bool,
    /// The catalogue the titles and labels are read through, or `None` for
    /// English.
    ///
    /// The tree is the shared one and its labels are written in English; a
    /// translation is a lookup applied where a label is drawn and where it is
    /// measured, which is why this is a field rather than a second tree.
    catalog: Option<Catalog>,
}

impl Menus {
    /// The bar, built from the shared tree with the chords from `keymap` and the
    /// recent files from `recent`.
    ///
    /// The recent list is part of the tree rather than something the File menu
    /// adds when it is opened, because the shared tree is the one place that
    /// knows the File menu's rows are in the order Notepad++ puts them. A new
    /// recent file therefore means a new tree, which is what the shell rebuilds
    /// when its list changes.
    pub fn new(keymap: &Keymap, recent: &[String]) -> Self {
        let mut menus = menus(Contents {
            recent,
            ..Contents::default()
        });
        for menu in &mut menus {
            fill_shortcuts(keymap, &mut menu.items);
        }
        Menus {
            menus,
            open: Vec::new(),
            bar: true,
            catalog: None,
        }
    }

    /// Swaps in a catalogue, or clears it back to English with `None`.
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

    /// Whether a dropdown is open, and therefore whether keys belong to the menu
    /// rather than to the document.
    pub fn is_open(&self) -> bool {
        !self.open.is_empty()
    }

    /// Opens the bar with the first title highlighted, or closes everything if
    /// it is already up. `F9` and `F10` both land here.
    pub fn toggle(&mut self) {
        if self.open.is_empty() {
            self.open.push(Open {
                at: 0,
                row: None,
            });
            self.bar = true;
        } else {
            self.close();
        }
    }

    /// Closes every level.
    pub fn close(&mut self) {
        self.open.clear();
        self.bar = true;
    }

    /// The titles, left to right, with two spaces between them.
    ///
    /// Read by the drawing code and by the tests, which is why it is a slice of
    /// the tree rather than a string: a caller that wants the bar as text joins
    /// it, and a caller that wants to know how wide a title is asks the tree.
    pub fn titles(&self) -> Vec<&str> {
        self.menus
            .iter()
            .map(|menu| self.tr(menu.title.as_str()))
            .collect()
    }

    /// Handles a key while the menus are open, answering the command to run if
    /// one was chosen.
    pub fn handle(&mut self, key: Key) -> Option<Action> {
        match key {
            Key::Escape => {
                if self.open.len() > 1 {
                    self.open.pop();
                } else {
                    self.close();
                }
                None
            }
            Key::Left => {
                self.step_title(-1);
                None
            }
            Key::Right => {
                self.step_title(1);
                None
            }
            Key::Up => {
                self.step_row(-1);
                None
            }
            Key::Down => {
                if self.bar {
                    self.open_rows();
                } else {
                    self.step_row(1);
                }
                None
            }
            Key::Enter => self.choose(),
            _ => None,
        }
    }

    /// Moves the highlight along the titles. Only the outermost level is a title
    /// row; from inside a submenu, Left and Right close and open levels.
    fn step_title(&mut self, delta: isize) {
        if !self.bar {
            if delta < 0 {
                self.open.pop();
                if self.open.len() == 1 {
                    self.bar = true;
                }
            } else {
                self.open_children();
            }
            return;
        }
        let at = self.open[0].at;
        let count = self.menus.len();
        if count == 0 {
            return;
        }
        let next = (at as isize + delta).rem_euclid(count as isize) as usize;
        self.open[0] = Open {
            at: next,
            row: None,
        };
    }

    /// Opens the highlighted title's dropdown.
    fn open_rows(&mut self) {
        if self.open.is_empty() {
            return;
        }
        self.bar = false;
        let items = &self.menus[self.open[0].at].items;
        let first = items.iter().position(|item| !item.separator);
        self.open[0].row = first;
    }

    /// Moves the highlight through the rows of the innermost open level, skipping
    /// separators, which are not rows a cursor can stand on.
    fn step_row(&mut self, delta: isize) {
        let Some(depth) = self.open.len().checked_sub(1) else {
            return;
        };
        let Some(index) = self.open[depth].row else {
            return;
        };
        let count = self.items_at(depth).len();
        if count == 0 {
            return;
        }
        let mut at = index as isize;
        let mut next = None;
        for _ in 0..count {
            at = (at + delta).rem_euclid(count as isize);
            if !self.items_at(depth)[at as usize].separator {
                next = Some(at as usize);
                break;
            }
        }
        if let Some(next) = next {
            self.open[depth].row = Some(next);
        }
    }

    /// Opens the submenu on the highlighted row, if it has one.
    fn open_children(&mut self) {
        let Some(&Open { row: Some(row), .. }) = self.open.last() else {
            return;
        };
        let depth = self.open.len() - 1;
        let items = self.items_at(depth);
        let Some(item) = items.get(row) else {
            return;
        };
        if item.children.is_empty() {
            return;
        }
        let first = item.children.iter().position(|child| !child.separator);
        // The parent keeps its highlight but stops being the innermost level, so
        // Enter on it opens rather than runs.
        self.open.push(Open { at: row, row: first });
        self.bar = false;
    }

    /// Runs the highlighted row: a command answer, a submenu, or nothing when the
    /// row is a separator or a heading.
    fn choose(&mut self) -> Option<Action> {
        if self.bar {
            self.open_rows();
            return None;
        }
        let Some(&Open { row: Some(row), .. }) = self.open.last() else {
            return None;
        };
        let depth = self.open.len() - 1;
        let items = self.items_at(depth);
        let item = items.get(row)?;
        if item.separator {
            return None;
        }
        if !item.children.is_empty() {
            self.open_children();
            return None;
        }
        let action = item.action;
        self.close();
        Some(action)
    }

    /// The rows at `depth`, where depth 0 is the dropdown under the bar.
    fn items_at(&self, depth: usize) -> &[Item] {
        let Some(&Open { at, .. }) = self.open.first() else {
            return &[];
        };
        let mut items: &[Item] = match self.menus.get(at) {
            Some(menu) => &menu.items,
            None => return &[],
        };
        // Each level names a row in the level above it, not a menu in the bar:
        // depth one is a row of the open dropdown, depth two a row of the
        // submenu that row opened, and so on.
        for level in 1..=depth {
            let Some(&Open { at, .. }) = self.open.get(level) else {
                return &[];
            };
            let Some(item) = items.get(at) else {
                return &[];
            };
            items = &item.children;
        }
        items
    }

    /// Draws the bar on row 0 and every open dropdown below it.
    pub fn draw(&self, grid: &mut Grid) {
        if grid.width() == 0 {
            return;
        }
        self.draw_bar(grid);
        for depth in 0..self.open.len() {
            self.draw_level(grid, depth);
        }
    }

    /// Fills row 0 with the titles, the open one lit.
    fn draw_bar(&self, grid: &mut Grid) {
        let width = grid.width();
        for column in 0..width {
            grid.put(0, column, ' ', Tone::Menu);
        }
        let mut column = 1usize;
        for (index, title) in self.titles().iter().enumerate() {
            let tone = if !self.open.is_empty() && self.open[0].at == index {
                Tone::MenuHover
            } else {
                Tone::Menu
            };
            // One space of padding on each side, which is what makes the
            // highlighted title read as a button rather than as a word.
            grid.put(0, column, ' ', tone);
            column += 1;
            let end = grid.text(0, column, title, tone);
            column = end + 1;
            if column < width {
                grid.put(0, column - 1, ' ', tone);
            }
            column += 1;
            if column >= width {
                break;
            }
        }
    }

    /// Draws the dropdown at `depth`: a box, its rows, and the shortcut column.
    fn draw_level(&self, grid: &mut Grid, depth: usize) {
        let Some(&Open { row, .. }) = self.open.get(depth) else {
            return;
        };
        let items = self.items_at(depth);
        if items.is_empty() {
            return;
        }
        let width = grid.width();
        let height = grid.height();
        let label_width = items
            .iter()
            .map(|item| {
                self.tr(&item.label).chars().count() + usize::from(!item.children.is_empty()) * 2
            })
            .max()
            .unwrap_or(0);
        let shortcut_width = items
            .iter()
            .map(|item| item.shortcut.chars().count())
            .max()
            .unwrap_or(0);
        // Two columns of padding inside the box, one of border on each side, and
        // a gap between the labels and the chords.
        let inner = label_width + if shortcut_width > 0 { shortcut_width + 3 } else { 0 };
        let box_width = (inner + 4).min(width);
        let left = self.left_of(depth).min(width.saturating_sub(box_width));
        let top = match depth {
            // The first dropdown hangs off the bar; every one after it hangs off
            // the row that opened it.
            0 => 1,
            _ => self.top_of(depth),
        };
        let rows = items.len() + 2;
        let box_height = rows.min(height.saturating_sub(top));
        if box_height < 3 {
            return;
        }

        // The border, as one rectangle.
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

        for (index, item) in items.iter().take(box_height - 2).enumerate() {
            let at = top + 1 + index;
            let lit = row == Some(index);
            if item.separator {
                for column in left + 1..left + box_width - 1 {
                    grid.put(at, column, '─', Tone::MenuBorder);
                }
                continue;
            }
            let tone = if lit { Tone::MenuHover } else { Tone::Menu };
            let background = if lit { Tone::MenuHover } else { Tone::Menu };
            for column in left + 1..left + box_width - 1 {
                grid.put(at, column, ' ', background);
            }
            grid.text(at, left + 2, self.tr(&item.label), tone);
            if !item.children.is_empty() {
                grid.put(at, left + 2 + label_width + 1, ARROW, tone);
            }
            if !item.shortcut.is_empty() {
                let at_column = left + box_width - 2 - item.shortcut.chars().count();
                grid.text(at, at_column, &item.shortcut, tone);
            }
        }
    }

    /// The column the box at `depth` starts at: under its title for the first
    /// level, and to the right of the row that opened it for the others.
    fn left_of(&self, depth: usize) -> usize {
        if depth == 0 {
            // Under the title, which is one space of padding plus every title
            // before it and its padding.
            let mut column = 1usize;
            for menu in self.menus.iter().take(self.open[0].at) {
                column += self.tr(&menu.title).chars().count() + 3;
            }
            return column;
        }
        let parent = self.left_of(depth - 1);
        let items = self.items_at(depth - 1);
        let width = items
            .iter()
            .map(|item| self.tr(&item.label).chars().count() + usize::from(!item.children.is_empty()) * 2)
            .max()
            .unwrap_or(0);
        let shortcuts = items
            .iter()
            .map(|item| item.shortcut.chars().count())
            .max()
            .unwrap_or(0);
        // The parent's box width, less one column so the child overlaps the
        // border rather than standing a gap away from it.
        parent + width + if shortcuts > 0 { shortcuts + 3 } else { 0 } + 3
    }

    /// The row the box at `depth` starts at, which is the row that opened it.
    fn top_of(&self, depth: usize) -> usize {
        let mut top = 1usize;
        for level in 0..depth {
            let Some(&Open { row: Some(row), .. }) = self.open.get(level) else {
                return top;
            };
            // One for the top border, and one for each row above it.
            top += 1 + row;
        }
        top
    }
}

/// Writes each row's chord from the keymap, at every depth.
///
/// A submenu row has no chord of its own: the arrow stands where the shortcut
/// column would be, and a chord beside it would read as the chord for opening
/// the submenu -- the windowed front-end draws it the same way, from the same
/// table.
fn fill_shortcuts(keymap: &Keymap, items: &mut [Item]) {
    for item in items {
        if item.separator || !item.children.is_empty() {
            item.shortcut.clear();
        } else {
            item.shortcut = keymap.display(item.action);
        }
        fill_shortcuts(keymap, &mut item.children);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::Grid;

    fn bar() -> Menus {
        Menus::new(&Keymap::default(), &[])
    }

    /// A catalogue written in this editor's own schema: one file, no ids.
    fn french() -> Catalog {
        let xml = concat!(
            "<NotepadPlus><Native-Langue name=\"Francais\" english=\"French\" ",
            "filename=\"french.xml\"><Menu><Main>",
            "<Item id=\"File\" name=\"Fichier\"/>",
            "<Item id=\"New\" name=\"Nouveau\"/>",
            "</Main></Menu></Native-Langue></NotepadPlus>",
        );
        Catalog::parse(None, xml).expect("a catalog")
    }

    #[test]
    fn the_bar_holds_the_thirteen_titles_in_order() {
        let menus = bar();
        assert_eq!(
            menus.titles(),
            [
                "File", "Edit", "Search", "View", "Encoding", "Language", "Settings", "Tools",
                "Macro", "Run", "Plugins", "Window", "Help",
            ]
        );
    }

    #[test]
    fn the_chords_come_from_the_shared_keymap() {
        let menus = bar();
        let file = &menus.menus[0].items;
        let save = file
            .iter()
            .find(|item| item.action == Action::Save)
            .expect("File has Save");
        assert_eq!(save.shortcut, "Ctrl+S");
        // A submenu row shows no chord even when its command has one.
        let submenu = file
            .iter()
            .find(|item| !item.children.is_empty())
            .expect("File has a submenu");
        assert_eq!(submenu.shortcut, "");
    }

    #[test]
    fn f9_opens_the_bar_and_escape_closes_it() {
        let mut menus = bar();
        menus.toggle();
        assert!(menus.is_open());
        assert_eq!(menus.handle(Key::Escape), None, "escape runs nothing");
        assert!(!menus.is_open());
    }

    #[test]
    fn the_titles_walk_and_wrap() {
        let mut menus = bar();
        menus.toggle();
        menus.handle(Key::Right);
        assert_eq!(menus.open[0].at, 1);
        menus.handle(Key::Left);
        menus.handle(Key::Left);
        assert_eq!(menus.open[0].at, 12, "Left from File wraps to Help");
    }

    #[test]
    fn down_opens_the_dropdown_on_its_first_row() {
        let mut menus = bar();
        menus.toggle();
        menus.handle(Key::Down);
        assert_eq!(menus.open[0].row, Some(0));
        // The first row of File is New, and it is not a separator.
        assert!(!menus.items_at(0)[0].separator);
    }

    #[test]
    fn the_cursor_skips_separators() {
        let mut menus = bar();
        menus.toggle();
        // Walk to the Edit menu, whose second row is a separator after Undo and
        // Redo.
        menus.handle(Key::Right);
        menus.handle(Key::Down);
        let count = menus.items_at(0).len();
        for _ in 0..count {
            menus.handle(Key::Down);
            let row = menus.open[0].row.expect("a row");
            assert!(
                !menus.items_at(0)[row].separator,
                "the cursor stood on a separator"
            );
        }
    }

    #[test]
    fn enter_on_a_command_answers_it_and_closes() {
        let mut menus = bar();
        menus.toggle();
        menus.handle(Key::Down);
        let first = menus.items_at(0)[0].action;
        assert_eq!(menus.handle(Key::Enter), Some(first));
        assert!(!menus.is_open());
    }

    #[test]
    fn enter_on_a_submenu_opens_it_instead_of_running_anything() {
        let mut menus = bar();
        menus.toggle();
        // The Encoding menu's Character sets row opens a submenu.
        for _ in 0..4 {
            menus.handle(Key::Right);
        }
        assert_eq!(menus.titles()[menus.open[0].at], "Encoding");
        menus.handle(Key::Down);
        // Walk to the submenu row.
        loop {
            let row = menus.open[0].row.expect("a row");
            if !menus.items_at(0)[row].children.is_empty() {
                break;
            }
            menus.handle(Key::Down);
        }
        assert_eq!(menus.handle(Key::Enter), None);
        assert_eq!(menus.open.len(), 2, "the submenu is open beside its parent");
        // Escape backs out one level rather than closing everything.
        menus.handle(Key::Escape);
        assert_eq!(menus.open.len(), 1);
        assert!(menus.is_open());
    }

    #[test]
    fn a_command_three_levels_down_is_reachable() {
        // Encoding, Character sets, a script, and then a code page: the deepest
        // chain the menus have, and the one that shows a level names a row of the
        // level above rather than a menu of the bar.
        let mut menus = bar();
        menus.toggle();
        let at = menus
            .menus
            .iter()
            .position(|menu| menu.title == "Encoding")
            .expect("Encoding");
        menus.open[0] = Open { at, row: None };
        menus.handle(Key::Down);
        // The Character sets row is the dropdown's first submenu.
        let sets = (0..menus.items_at(0).len())
            .find(|&index| !menus.items_at(0)[index].children.is_empty())
            .expect("Character sets");
        assert_eq!(menus.items_at(0)[sets].label, "Character sets");
        menus.open[0].row = Some(sets);
        menus.handle(Key::Enter);
        assert_eq!(menus.open.len(), 2);
        // The scripts are the submenus inside Character sets.
        let script = (0..menus.items_at(1).len())
            .find(|&index| !menus.items_at(1)[index].children.is_empty())
            .expect("a script");
        let name = menus.items_at(1)[script].label.clone();
        menus.open[1].row = Some(script);
        menus.handle(Key::Enter);
        assert_eq!(menus.open.len(), 3, "{name} did not open");
        // Every row of a script is a code page, and picking one answers it.
        let first = menus.items_at(2)[0].label.clone();
        let action = menus.handle(Key::Enter).expect("a command");
        assert!(
            matches!(action, Action::EncodingSet(nn_core::Encoding::CodePage(_))),
            "{first} gave {action:?}"
        );
        assert!(!menus.is_open(), "choosing closes the whole chain");
    }

    #[test]
    fn the_dropdown_is_drawn_as_a_box_round_its_rows() {
        let mut menus = bar();
        let mut grid = Grid::new(60, 12);
        menus.toggle();
        menus.handle(Key::Down);
        menus.draw(&mut grid);
        // The bar is row 0 and holds the titles.
        assert_eq!(grid.get(0, 1).ch, ' ');
        assert_eq!(grid.get(0, 2).ch, 'F', "the File title starts at column 2");
        // The box starts on the next row, one column in from the left.
        assert_eq!(grid.get(1, 1).ch, '┌');
        assert_eq!(grid.get(1, 2).ch, '─');
        // The first row inside it is New, with its chord on the right.
        let text = grid_row(&grid, 2);
        assert!(text.starts_with(" │ New"), "{text}");
        assert!(text.contains("Ctrl+N"), "{text}");
    }

    #[test]
    fn a_box_wider_than_the_screen_is_cut_rather_than_wrapped() {
        let mut menus = bar();
        let mut grid = Grid::new(20, 12);
        menus.toggle();
        menus.handle(Key::Down);
        menus.draw(&mut grid);
        // Nothing is written past the last column: a box that wrapped would
        // scroll the whole screen on a real terminal.
        for row in 0..12 {
            for column in 0..20 {
                assert!(grid.get(row, column).ch != '\0');
            }
        }
    }

    fn grid_row(grid: &Grid, row: usize) -> String {
        let mut out = String::new();
        for column in 0..grid.width() {
            out.push(grid.get(row, column).ch);
        }
        out
    }

    #[test]
    fn a_catalogue_translates_the_titles_and_the_rows() {
        let mut menus = bar();
        menus.set_catalog(Some(french()));
        assert_eq!(menus.titles()[0], "Fichier");
        // A title the catalogue does not cover stays English.
        assert_eq!(menus.titles()[1], "Edit");
        menus.toggle();
        menus.handle(Key::Down);
        let mut grid = Grid::new(80, 24);
        menus.draw(&mut grid);
        let drawn: String = (0..24)
            .map(|row| (0..80).map(|c| grid.get(row, c).ch).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(drawn.contains("Fichier"), "{drawn}");
        assert!(drawn.contains("Nouveau"), "{drawn}");
    }
}
