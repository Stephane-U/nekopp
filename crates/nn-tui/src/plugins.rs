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

//! The plugin manager, and the plugins behind it.
//!
//! The window's manager is a list of switches over the `.wasm` files in the
//! configuration folder, and so is this one: the same folder, the same file
//! names, the same two state words, and the same settings line recording which
//! plugins are on. What the terminal does not have is a second implementation of
//! anything else, because it does not need one -- the interpreter, the scan of
//! the folder, the switch and the bridge between a module and a document are all
//! `nn_core::plugin`'s, put there for exactly this reason: `npp` runs a command
//! against the document being edited and `npp-mcp` runs the same command against
//! text a client handed it, and a third front-end costs a list and a box.
//!
//! A modal that owns a file handle is a modal that has to be trusted with the
//! file, so this one does not: it reports which row was flipped and the shell
//! writes the settings, as the preferences and styles dialogs already do. The
//! plugin modules themselves are a different matter -- switching one on is
//! loading it -- and those live in `host` here, one per program rather than one
//! per keystroke, because a plugin instantiated twice would report different
//! command names in the menu than in the manager.

use std::path::PathBuf;

use nn_core::plugin::{Context, Inventory, Outcome, Plugins as Host};
use nn_shared::settings::Settings;

use crate::keys::Key;
use crate::palette::Tone;
use crate::screen::Grid;

/// The title on the box, and the keys along its bottom edge.
///
/// The window's own words, so a user who has read one manager has read both.
const TITLE: &str = "Plugin Manager";
const HINT: &str = "Space enables or disables, Esc closes";
/// The two words a row can end in.
const ENABLED: &str = "Enabled";
const DISABLED: &str = "Disabled";
/// What an empty folder says.
const EMPTY: &str = "No Plugins Installed";
/// The most rows shown at once, before the list scrolls.
const MAX_ROWS: usize = 14;

/// The plugins, the manager over them, and which row is lit.
pub struct Plugins {
    /// The folder and its modules: the only part of this that is not a widget.
    host: Host,
    /// The folder the modules were read from, kept so a reload reads it again.
    dir: Option<PathBuf>,
    open: bool,
    selected: usize,
    /// What the last flip came to, for the shell to say and clear.
    ///
    /// Held rather than returned because the two answers a flip has -- the
    /// plugin loaded, or the module refused to -- are one sentence each, and a
    /// failed load is worth saying even though nothing in the settings moved.
    notice: Option<String>,
}

impl Plugins {
    /// Reads the folder and switches on what the settings record.
    ///
    /// The order matters: the settings name plugins by the name each module
    /// gives itself, and nothing has run before this point, which is what
    /// "disabled until enabled" means at startup in both front-ends.
    pub fn new(dir: Option<PathBuf>, saved: &[String]) -> Plugins {
        let mut host = match &dir {
            Some(dir) => Host::discover(dir),
            None => Host::default(),
        };
        host.enable_saved(saved);
        Plugins {
            host,
            dir,
            open: false,
            selected: 0,
            notice: None,
        }
    }

    /// Reads the folder again, and switches on the plugins `saved` names.
    ///
    /// The names come from the settings rather than from what is running, so a
    /// reload after a plugin was rebuilt on disk brings the new build, and a
    /// reload after one was deleted leaves it simply absent.
    pub fn reload(&mut self, saved: &[String]) {
        self.host = match &self.dir {
            Some(dir) => Host::discover(dir),
            None => Host::default(),
        };
        self.host.enable_saved(saved);
        self.clamp_selection();
    }

    /// One row per installed plugin, in folder order.
    pub fn inventory(&self) -> Vec<Inventory> {
        self.host.inventory()
    }

    /// How many plugins are installed, and how many of them are running.
    pub fn counts(&self) -> (usize, usize) {
        (self.host.available(), self.host.len())
    }

    /// The files that could not be read as modules.
    pub fn failures(&self) -> usize {
        self.host.failures().len()
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self) {
        self.open = true;
        self.clamp_selection();
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// The row that is lit. Only the tests ask: the shell draws the lit row from
    /// the field itself, and a flip reports its own row.
    #[cfg(test)]
    pub fn selected_row(&self) -> usize {
        self.selected
    }

    /// Keeps the lit row inside the list, which is one place because a plugin
    /// appearing or disappearing moves the rows under a reader's finger.
    fn clamp_selection(&mut self) {
        let rows = self.host.inventory().len();
        self.selected = self.selected.min(rows.saturating_sub(1));
    }

    /// Handles one key, and says whether the settings changed.
    ///
    /// Every key that arrives while the box is up stops here: a modal that let
    /// one through would type into the document behind it, and the box would then
    /// be describing a document the reader cannot see. The return value is
    /// narrower -- whether the list of enabled plugins moved -- because that is
    /// what the caller has to write down and put back on the menu.
    pub fn handle(&mut self, key: Key, settings: &mut Settings) -> bool {
        if !self.open {
            return false;
        }
        let rows = self.host.inventory().len();
        match key {
            Key::Escape => {
                self.close();
                false
            }
            Key::Up => {
                self.selected = self.selected.saturating_sub(1);
                false
            }
            Key::Down => {
                self.selected = (self.selected + 1).min(rows.saturating_sub(1));
                false
            }
            Key::PageUp => {
                self.selected = 0;
                false
            }
            Key::PageDown => {
                self.selected = rows.saturating_sub(1);
                false
            }
            Key::Char(' ') => {
                if rows == 0 {
                    return false;
                }
                let changed = self.flip(self.selected);
                // Rewritten from what is actually running rather than edited in
                // place: a plugin whose file was deleted while the program was
                // open leaves a name in the file that matches nothing, and this
                // is the one moment where the truth is in memory.
                settings.enabled_plugins = self.host.names();
                changed
            }
            _ => false,
        }
    }

    /// Switches one plugin on or off, and says whether that moved the list.
    ///
    /// A plugin that will not load is a failure of its own file, and the answer is
    /// the settings saying it is still off: the row stays `Disabled` and the shell
    /// says what the module said. That is why the sentence is kept beside the
    /// answer rather than derived from the row afterwards -- `Disabled` reads the
    /// same whether a plugin was switched off or never came on.
    fn flip(&mut self, index: usize) -> bool {
        let Some(row) = self.host.inventory().into_iter().nth(index) else {
            return false;
        };
        if row.enabled {
            let changed = self.host.disable(index).is_some();
            if changed {
                self.notice = Some(format!("Unloaded {}", row.name));
            }
            return changed;
        }
        match self.host.enable(index) {
            Ok(name) => {
                self.notice = Some(format!("Loaded {name}"));
                true
            }
            Err(error) => {
                self.notice = Some(format!("Cannot load the plugin: {error}"));
                false
            }
        }
    }

    /// The sentence the last flip earned, taken so it is said once.
    pub fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
    }

    /// Runs one command of one plugin over a document.
    ///
    /// The crossing is where the units change: the editor counts characters and a
    /// plugin counts bytes, so the caller converts before handing the context in,
    /// once per command. A command that traps leaves the document alone -- the
    /// bridge it worked on is a copy -- and comes back as a sentence rather than
    /// an error type this front-end would have to know how to read.
    pub fn run(
        &mut self,
        plugin: usize,
        command: usize,
        context: &Context,
    ) -> Result<Outcome, String> {
        self.host
            .run(plugin, command, context)
            .map_err(|error| format!("Plugin failed: {error}"))
    }

    /// Draws the box over whatever is on the grid.
    pub fn draw(&self, grid: &mut Grid) {
        if !self.open {
            return;
        }
        let width = grid.width();
        let height = grid.height();
        let rows = self.host.inventory();
        let labels = rows
            .iter()
            .map(|row| row.name.chars().count())
            .chain(std::iter::once(EMPTY.chars().count()))
            .max()
            .unwrap_or(0);
        // The name, one blank column of padding each side, and the state word at
        // a fixed column so the two words line up whatever the names are.
        let content = labels + 2 + ENABLED.chars().count() + 2;
        let widest = content.max(HINT.chars().count() + 4);
        let box_width = widest.min(width.saturating_sub(2)).max(24.min(width));
        // Room for the title, the hint and one row at least, so an empty folder
        // draws a box saying it is empty rather than nothing at all.
        let shown = rows.len().clamp(1, MAX_ROWS);
        let box_height = (shown + 4).min(height.saturating_sub(2));
        if box_height < 5 || box_width < 12 {
            return;
        }
        let left = (width - box_width) / 2;
        let top = (height - box_height) / 2;
        draw_box(grid, left, top, box_width, box_height, TITLE);
        let hint_at = top + box_height - 1;
        let hint_column = left + box_width.saturating_sub(2 + HINT.chars().count());
        grid.text(hint_at, hint_column, HINT, Tone::MenuHover);

        let fits = hint_at.saturating_sub(top + 2);
        if rows.is_empty() {
            // An empty dropdown reads as a bug rather than as an empty list, so
            // the box says which of the two it is.
            grid.text(top + 2, left + 2, EMPTY, Tone::Menu);
            return;
        }
        let first = if self.selected < fits {
            0
        } else {
            self.selected + 1 - fits.max(1)
        };
        for (offset, row) in rows.iter().enumerate().skip(first).take(fits) {
            let at = top + 2 + offset - first;
            let lit = offset == self.selected;
            let tone = if lit { Tone::MenuHover } else { Tone::Menu };
            for column in left + 1..left + box_width - 1 {
                grid.put(at, column, ' ', tone);
            }
            grid.text(at, left + 2, &row.name, tone);
            let state = if row.enabled { ENABLED } else { DISABLED };
            let column = left + box_width - 2 - state.chars().count();
            grid.text(at, column, state, tone);
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

    /// A folder with one file in it that is not a module, so the manager has a
    /// row to draw without a wasm build being involved.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("npp-tui-plugins-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a folder");
        dir
    }

    #[test]
    fn an_empty_folder_is_an_empty_list() {
        let dir = scratch("empty");
        let plugins = Plugins::new(Some(dir), &[]);
        assert!(plugins.inventory().is_empty());
        assert_eq!(plugins.counts(), (0, 0));
    }

    #[test]
    fn a_folder_that_is_not_there_is_the_same_as_an_empty_one() {
        let plugins = Plugins::new(Some(PathBuf::from("/nonexistent/plugins")), &[]);
        assert!(plugins.inventory().is_empty());
        // And a program with no configuration directory at all does not panic.
        let none = Plugins::new(None, &[]);
        assert!(none.inventory().is_empty());
    }

    #[test]
    fn a_file_that_is_not_a_module_is_a_failure_rather_than_a_row() {
        let dir = scratch("broken");
        std::fs::write(dir.join("broken.wasm"), b"not a module").expect("a file");
        let plugins = Plugins::new(Some(dir), &[]);
        assert!(plugins.inventory().is_empty());
        assert_eq!(plugins.failures(), 1);
    }

    #[test]
    fn a_closed_box_takes_no_keys() {
        let dir = scratch("closed");
        let mut plugins = Plugins::new(Some(dir), &[]);
        let mut settings = Settings::default();
        assert!(!plugins.handle(Key::Down, &mut settings));
        assert!(!plugins.handle(Key::Char(' '), &mut settings));
        assert!(settings.enabled_plugins.is_empty());
    }

    #[test]
    fn walking_an_empty_list_stays_on_the_first_row() {
        let dir = scratch("walk");
        let mut plugins = Plugins::new(Some(dir), &[]);
        plugins.open();
        let mut settings = Settings::default();
        for _ in 0..3 {
            plugins.handle(Key::Down, &mut settings);
        }
        assert_eq!(plugins.selected_row(), 0);
        plugins.handle(Key::PageDown, &mut settings);
        assert_eq!(plugins.selected_row(), 0);
        // Space on nothing writes nothing.
        assert!(!plugins.handle(Key::Char(' '), &mut settings));
        assert!(settings.enabled_plugins.is_empty());
    }

    #[test]
    fn escape_closes_the_box_and_nothing_else_does() {
        let dir = scratch("escape");
        let mut plugins = Plugins::new(Some(dir), &[]);
        plugins.open();
        let mut settings = Settings::default();
        assert!(!plugins.handle(Key::Up, &mut settings));
        assert!(plugins.is_open());
        assert!(!plugins.handle(Key::Escape, &mut settings));
        assert!(!plugins.is_open());
    }

    /// What the box puts on a screen, as text.
    fn screen(plugins: &Plugins, width: usize, height: usize) -> String {
        let mut grid = Grid::new(width, height);
        plugins.draw(&mut grid);
        (0..height)
            .map(|row| (0..width).map(|c| grid.get(row, c).ch).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn an_empty_manager_says_so_rather_than_drawing_an_empty_box() {
        let dir = scratch("draw-empty");
        let mut plugins = Plugins::new(Some(dir), &[]);
        plugins.open();
        let text = screen(&plugins, 80, 24);
        assert!(text.contains("Plugin Manager"), "{text}");
        assert!(text.contains(EMPTY), "{text}");
        assert!(text.contains("Esc closes"), "{text}");
    }

    #[test]
    fn a_folder_too_small_for_the_box_is_left_alone() {
        let dir = scratch("tiny");
        let mut plugins = Plugins::new(Some(dir), &[]);
        plugins.open();
        let mut grid = Grid::new(10, 4);
        plugins.draw(&mut grid);
        for row in 0..4 {
            for column in 0..10 {
                assert_eq!(grid.get(row, column).ch, ' ', "{row},{column}");
            }
        }
    }
}

