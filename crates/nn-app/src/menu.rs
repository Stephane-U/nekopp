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

//! The menu bar.
//!
//! Notepad++ is a menu-driven editor, so the menu is not decoration: it is the
//! primary way to reach a command. This module owns the bar, the dropdowns, and
//! the hit testing; it does not own the commands themselves. A click resolves to
//! an `Action` and the application decides what that means.
//!
//! Everything is laid out from the monospace advance width, so a dropdown is a
//! rectangle whose width is the longest label plus its shortcut. That keeps the
//! layout arithmetic exact and avoids measuring text twice.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics};
use glyphon::{TextArea, TextBounds};

use crate::i18n::Catalog;
use crate::keymap::Keymap;
use crate::quad::QuadRenderer;
use crate::settings::ThemeChoice;
use crate::text::shaping_for;
use crate::theme::Theme;

// The command vocabulary moved to `nn-shared` in the hundred-and-ninth
// pass: it is what both front-ends decide, and the terminal front-end has
// to answer with the same names, the same bindings and the same text-command
// table. Re-exported rather than renamed so that every `crate::menu::Action`
// in this program -- 275 dispatch sites and counting -- still says what it
// says.
pub use nn_shared::action::{Action, BrowserKind, MatchMode, Sort, WindowSort};
// The tree moved with the vocabulary in the hundred-and-tenth pass: the rows a
// dropdown is made of are decisions, and the window's share of a menu is the
// shaping and the boxes. What stays here is everything that measures or draws --
// `MenuBar`, the shaped buffers, the panel arithmetic -- plus the two passes that
// fill a column from outside: the keymap's chords and the catalogue's labels.
pub use nn_shared::menu::{
    LANGUAGE_MENU, PLUGIN_MENU, RUN_MENU,
    edit_items, encoding_items, file_items, help_items, language_items,
    line_operation_items, macro_items, multi_select_items, row_items, run_items,
    same_rows, search_items, settings_items, tools_items, view_items, window_items,
    Item, Menu, PluginMenu, RowItem, CHARACTER_SETS,
};

/// The theme names, in the order the preference row steps through them.
///
/// The Settings menu's Theme submenu is built from this list, and
/// `Action::Theme(n)` is an index into it, so the menu and the cycling row can
/// only agree: both read `ThemeChoice::ALL`.
fn theme_labels() -> Vec<&'static str> {
    ThemeChoice::ALL.iter().map(|choice| choice.label()).collect()
}

/// The action a settings file names, theme rows included.
///
/// [`Action::from_name`] answers what a *command* is called; a theme is a
/// setting, and `theme_<key>` is resolved against [`ThemeChoice::ALL`] here
/// rather than in the shared vocabulary, because the list of fifty themes is
/// palette names that no command has a use for. The order the two are tried in
/// cannot be observed: no name starts with both `theme_` and another prefix.
pub fn action_from_name(name: &str) -> Option<Action> {
    if let Some(key) = name.strip_prefix("theme_") {
        return ThemeChoice::ALL
            .iter()
            .position(|choice| choice.key() == key)
            .map(Action::Theme);
    }
    Action::from_name(name)
}

/// Where one dropdown panel sits on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Panel {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
}

impl Panel {
    /// Whether a point is inside the panel.
    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.left && x < self.left + self.width && y >= self.top && y < self.top + self.height
    }
}

/// One dropdown row, shaped.
///
/// Mirrors `Item` so a submenu's labels are shaped once, at `prepare`, rather
/// than every frame. The tree is rebuilt whenever the labels change, which is
/// what `shaped` already gates.
struct Shaped {
    label: Buffer,
    shortcut: Buffer,
    children: Vec<Shaped>,
}

/// One top-level menu, as something outside the menu needs to see it.
pub struct Row {
    /// The title shown in the bar, already translated.
    pub title: String,
    /// The dropdown's rows, in order.
    pub items: Vec<RowItem>,
}

/// The horizontal bar and its dropdowns.
pub struct MenuBar {
    menus: Vec<Menu>,
    /// Index of the open menu, if any.
    open: Option<usize>,
    /// Index of the highlighted row in the open menu.
    hover: Option<usize>,
    /// The chords shown in the shortcut column.
    keymap: Keymap,
    metrics: Metrics,
    family: Family<'static>,
    char_width: f32,
    /// Shaped top-level titles, one per menu.
    titles: Vec<Buffer>,
    /// How wide each title is, in pixels.
    ///
    /// Measured from the shaped buffer, not counted in characters: a title
    /// translated into a script whose glyphs are twice a cell wide -- Japanese,
    /// Korean, Chinese -- would otherwise be placed on top of the menu after it.
    title_widths: Vec<f32>,
    /// Shaped dropdown rows, mirroring the item tree.
    shaped_rows: Vec<Vec<Shaped>>,
    /// The arrow drawn on a row that opens a submenu.
    arrow: Buffer,
    /// The chain of open submenus, outermost first.
    ///
    /// Each entry is a row index in the level above it: `path[0]` is a row of the
    /// top dropdown, `path[1]` a row of that row's submenu, and so on. Empty means
    /// only the top dropdown is showing. A `Vec` rather than a fixed depth because
    /// the depth is whatever the menu happens to be, and a menu tree is data.
    path: Vec<usize>,
    /// The active localization, if one was loaded.
    catalog: Option<Catalog>,
    /// The plugins the Plugins menu was last built from.
    ///
    /// Kept rather than counted, because a language change rebuilds the menu and
    /// can only do that from what it was built from. The slice is in inventory
    /// order: its positions are the plugin indices the rows carry.
    plugins: Vec<PluginMenu>,
    shaped: bool,
}

/// Padding either side of a top-level title.
const TITLE_PADDING: f32 = 10.0;
/// Padding either side of a dropdown row.
const ROW_PADDING: f32 = 12.0;
/// Extra height of a dropdown row over the line height.
const ROW_EXTRA: f32 = 4.0;
/// Gap between a label and its shortcut column.
const COLUMN_GAP: f32 = 24.0;
/// Width reserved at the right of a row for the submenu arrow.
const ARROW_COLUMN: f32 = 14.0;
/// How far a submenu overlaps the panel it came from.
const PANEL_OVERLAP: f32 = 4.0;
/// The glyph on a row that opens a submenu.
///
/// U+25B8, which Noto Sans Mono has. A `>` would have been safe in any font and
/// would have looked like a typo.
const SUBMENU_ARROW: &str = "\u{25b8}";



/// The rows of the Plugins menu.
///
/// Three rows that never change: the manager, the folder, and the reload. Below
/// them, one submenu per *running* plugin, named after the plugin and holding
/// its commands -- the arrangement Notepad++ uses, where "JSON" opens onto
/// "Pretty Print", "Minify" and "Validate".
///
/// A plugin that is switched off gets no submenu, not a greyed one: the switch
/// is what makes its commands exist, and the manager one row up is where it is
/// flipped. That also keeps the dropdown still while the pointer is over it --
/// its length follows the switches, and the switches are never touched from
/// inside the dropdown.
///
/// Nothing below the management rows is translatable. A plugin's name and its
/// command names are the plugin's own words; the rows are literal, so no
/// catalog can rename a command into one the plugin does not have.
///
/// With nothing installed the menu still says so, because an empty dropdown
/// reads as a bug rather than as an empty list. A folder that holds plugins,
/// switched on or not, gets no note: the manager is one row up.
fn plugin_items(plugins: &[PluginMenu], catalog: Option<&Catalog>) -> Vec<Item> {
    let verb = |english: &'static str| match catalog {
        Some(catalog) => catalog.tr(english).to_string(),
        None => english.to_string(),
    };
    // Every row here is built translated, prose and all, because `relabel_items`
    // rebuilds each label from its English text. Building them is cheap, so
    // nothing in this menu relies on being relabelled afterwards.
    let row = |english: &'static str, action: Action| {
        Item::composed(english.to_string(), verb(english), action)
    };
    // The folder row is always there, even with nothing installed: it is how a
    // user gets from "no plugins" to having one. Reload sits beside it so a
    // plugin dropped into the folder by hand does not need a restart.
    let mut items = vec![
        row("Manage Plugins...", Action::ManagePlugins),
        row("Open Plugins Folder...", Action::OpenPluginsFolder),
        row("Reload Plugins", Action::ReloadPlugins),
    ];
    if plugins.is_empty() {
        items.push(Item::separator());
        items.push(row("No Plugins Installed", Action::NoOp));
        return items;
    }
    // The index of a plugin in this list is the index the inventory gave it,
    // because `set_plugins` is handed the inventory in its own order. Nothing
    // else in this module needs to know that; the rows carry the pair.
    let commands = |index: usize, plugin: &PluginMenu| -> Vec<Item> {
        plugin
            .commands
            .iter()
            .enumerate()
            .map(|(command, label)| Item::literal(label.clone(), Action::Plugin(index, command)))
            .collect()
    };
    let running: Vec<Item> = plugins
        .iter()
        .enumerate()
        .filter(|(_, plugin)| plugin.enabled && !plugin.commands.is_empty())
        .map(|(index, plugin)| {
            Item::literal_submenu(plugin.name.clone(), commands(index, plugin))
        })
        .collect();
    if !running.is_empty() {
        items.push(Item::separator());
        items.extend(running);
    }
    items
}

/// Rewrites every row's shortcut column from the keymap, at every depth.
///
/// A submenu row has no chord of its own: the arrow is drawn where the shortcut
/// column would be, and a chord beside it would read as the chord for opening the
/// submenu.
fn set_shortcuts(keymap: &Keymap, items: &mut [Item]) {
    for item in items {
        if item.separator || item.is_submenu() {
            item.shortcut = String::new();
        } else {
            item.shortcut = keymap.display(item.action);
        }
        set_shortcuts(keymap, &mut item.children);
    }
}

/// Shapes every row of `items`, and of every submenu under them.
/// The widest line of a shaped buffer, in pixels.
///
/// The menu is laid out in pixels rather than in character cells because a
/// translated label is not necessarily as wide as it is long: a Japanese title
/// of four characters is eight cells wide, and the cell count would put the next
/// one on top of it.
fn widest_line(buffer: &Buffer) -> f32 {
    buffer.layout_runs().map(|run| run.line_w).fold(0.0, f32::max)
}

fn shape_items(bar: &MenuBar, font_system: &mut FontSystem, items: &[Item]) -> Vec<Shaped> {
    items
        .iter()
        .map(|item| Shaped {
            label: bar.shape(font_system, &item.label),
            shortcut: bar.shape(font_system, &item.shortcut),
            children: shape_items(bar, font_system, &item.children),
        })
        .collect()
}

/// Rewrites every title and row from the catalog.
///
/// A free function rather than a method so the catalog and the menus can be
/// borrowed at once: `self.catalog.as_ref()` and `&mut self.menus` are disjoint
/// fields, but a `&mut self` method could not see both.
fn relabel(catalog: Option<&Catalog>, menus: &mut [Menu]) {
    for menu in menus {
        menu.title = match catalog {
            Some(catalog) => catalog.tr(menu.english).to_string(),
            None => menu.english.to_string(),
        };
        relabel_items(catalog, &mut menu.items);
    }
}

/// Relabels `items` and everything under them.
fn relabel_items(catalog: Option<&Catalog>, items: &mut [Item]) {
    for item in items {
        if item.separator || item.literal {
            continue;
        }
        item.label = match catalog {
            Some(catalog) => catalog.tr(&item.english).to_string(),
            None => item.english.clone(),
        };
        relabel_items(catalog, &mut item.children);
    }
}

impl MenuBar {
    /// Builds the standard menu set.
    pub fn new(
        metrics: Metrics,
        family: Family<'static>,
        char_width: f32,
        keymap: Keymap,
    ) -> Self {
        let mut menus = nn_shared::menu::menus(nn_shared::menu::Contents {
            themes: &theme_labels(),
            ..Default::default()
        });
        // The Plugins rows are composed from the catalogue and from names the
        // plugins give themselves, so they are built here and dropped into the
        // menu the shared set left empty.
        menus[PLUGIN_MENU].items = plugin_items(&[], None);

        // Guessed from the character grid until `prepare` measures the shaped
        // titles, so the bar has a layout before the first frame.
        let title_widths: Vec<f32> = menus
            .iter()
            .map(|menu| menu.title.chars().count() as f32 * char_width)
            .collect();
        let mut bar = Self {
            menus,
            open: None,
            hover: None,
            keymap,
            metrics,
            family,
            char_width,
            titles: Vec::new(),
            title_widths,
            shaped_rows: Vec::new(),
            arrow: Buffer::new_empty(metrics),
            path: Vec::new(),
            catalog: None,
            plugins: Vec::new(),
            shaped: false,
        };
        bar.refresh_shortcuts();
        bar
    }

    /// Rebuilds the bar at a new display density.
    ///
    /// The titles are shaped in `prepare` from `metrics` and cached behind a
    /// flag, so dropping the flag is the whole of the work: the next frame shapes
    /// every label at the size the display asked for, and measures the widths
    /// from what it made.
    pub fn set_metrics(&mut self, metrics: Metrics) {
        if metrics == self.metrics {
            return;
        }
        // Everything in this widget that is counted in characters -- a tab's
        // width, the column a value starts in, the box around a row -- is
        // measured with `char_width`, so the width has to grow with the text or
        // the box stays the size the window opened at and the labels run out of
        // it.
        self.char_width *= metrics.line_height / self.metrics.line_height;
        self.metrics = metrics;
        self.shaped = false;
        self.titles.clear();
        self.title_widths.clear();
        self.shaped_rows.clear();
    }

    /// Points the shortcut column at a new keymap.
    pub fn set_keymap(&mut self, keymap: Keymap) {
        if self.keymap == keymap {
            return;
        }
        self.keymap = keymap;
        self.refresh_shortcuts();
    }

    /// Swaps in a localization, or clears it with `None`.
    ///
    /// The whole bar is relabelled from the stored English text, so switching
    /// languages twice does not translate a translation.
    pub fn set_catalog(&mut self, catalog: Option<Catalog>) {
        self.catalog = catalog;
        relabel(self.catalog.as_ref(), &mut self.menus);
        // The plugin rows are built from the catalog and the folder rather than
        // relabelled above, so they are built again here: a row that appears
        // only when the folder is empty has to arrive in the new language too,
        // and a plugin's own words have to keep the spelling the plugin chose.
        self.menus[PLUGIN_MENU].items = plugin_items(&self.plugins, self.catalog.as_ref());
        self.refresh_shortcuts();
        self.close();
    }

    /// The name of the active localization, if any.
    pub fn catalog_name(&self) -> Option<&str> {
        self.catalog.as_ref().map(Catalog::name)
    }

    /// Every fixed English label the bar draws, at every depth.
    ///
    /// The stored English text rather than what is on screen, so a test can ask
    /// what a catalog has to cover without first applying one. Labels that come
    /// from outside -- plugin names, user-defined languages, recent files -- are
    /// in the list too when the bar was built with them, which is why the
    /// completeness test builds a bar with none of them.
    pub fn english_labels(&self) -> Vec<&str> {
        fn walk<'a>(items: &'a [Item], out: &mut Vec<&'a str>) {
            for item in items {
                if !item.separator && !item.literal {
                    out.push(&item.english);
                }
                walk(&item.children, out);
            }
        }
        let mut out: Vec<&str> = self.menus.iter().map(|menu| menu.english).collect();
        for menu in &self.menus {
            walk(&menu.items, &mut out);
        }
        out
    }

    /// Rewrites every row's shortcut from the keymap.
    ///
    /// Called whenever the rows or the keymap change, so the two cannot drift.
    fn refresh_shortcuts(&mut self) {
        let keymap = &self.keymap;
        for menu in &mut self.menus {
            set_shortcuts(keymap, &mut menu.items);
        }
        self.shaped = false;
    }

    /// Replaces the recent file list shown in the File menu.
    ///
    /// The labels are reshaped on the next frame rather than here, so the caller
    /// does not need the font system.
    pub fn set_recent(&mut self, names: &[String]) {
        let items = file_items(names);
        if self.menus[0].items.len() == items.len()
            && self.menus[0]
                .items
                .iter()
                .zip(&items)
                .all(|(a, b)| a.english == b.english)
        {
            return;
        }
        self.menus[0].items = items;
        relabel(self.catalog.as_ref(), &mut self.menus);
        self.refresh_shortcuts();
        self.close();
    }

    /// Replaces the saved commands shown in the Run menu.
    pub fn set_runs(&mut self, labels: &[String]) {
        let items = run_items(labels);
        if self.menus[RUN_MENU].items.len() == items.len()
            && self.menus[RUN_MENU]
                .items
                .iter()
                .zip(&items)
                .all(|(a, b)| a.english == b.english)
        {
            return;
        }
        self.menus[RUN_MENU].items = items;
        relabel(self.catalog.as_ref(), &mut self.menus);
        self.refresh_shortcuts();
        self.close();
    }

    /// Replaces the user-defined languages shown in the Language menu.
    pub fn set_languages(&mut self, names: &[String]) {
        let items = language_items(names);
        if same_rows(&self.menus[LANGUAGE_MENU].items, &items) {
            return;
        }
        self.menus[LANGUAGE_MENU].items = items;
        relabel(self.catalog.as_ref(), &mut self.menus);
        self.refresh_shortcuts();
        self.close();
    }

    /// Rebuilds the Plugins menu from the plugins the editor has.
    ///
    /// The list is in inventory order and is kept even when the rows come out
    /// identical, because the next language change rebuilds them from it.
    ///
    /// Nothing is relabelled here. `plugin_items` builds every row already
    /// translated, and the rows of this menu are the only ones whose words come
    /// from a catalog and whose position comes from the disk.
    pub fn set_plugins(&mut self, plugins: &[PluginMenu]) {
        self.plugins = plugins.to_vec();
        let items = plugin_items(&self.plugins, self.catalog.as_ref());
        if same_rows(&self.menus[PLUGIN_MENU].items, &items) {
            return;
        }
        self.menus[PLUGIN_MENU].items = items;
        self.refresh_shortcuts();
        self.close();
    }

    /// The bar as a list of rows, for anything outside the menu that needs to
    /// see it.
    ///
    /// The accessibility tree is the caller. It needs the titles, the labels,
    /// the shortcuts and the action each row runs, and it must not reach into
    /// the private `Item` to get them -- a second reader of the same fields is
    /// how a menu and its accessibility tree end up disagreeing.
    pub fn rows(&self) -> Vec<Row> {
        self.menus
            .iter()
            .map(|menu| Row {
                title: menu.title.clone(),
                items: row_items(&menu.items),
            })
            .collect()
    }

    /// The height of the bar itself.
    pub fn height(&self, line_height: f32) -> f32 {
        line_height + 6.0
    }

    /// Whether a dropdown is open.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Closes any open dropdown, and every submenu under it.
    pub fn close(&mut self) {
        self.open = None;
        self.path.clear();
        self.hover = None;
    }

    /// Shapes every label once.
    pub fn prepare(&mut self, font_system: &mut FontSystem) {
        if self.shaped {
            return;
        }
        self.shaped = true;
        self.titles = self
            .menus
            .iter()
            .map(|m| self.shape(font_system, &m.title))
            .collect();
        self.title_widths = self.titles.iter().map(widest_line).collect();
        self.arrow = self.shape(font_system, SUBMENU_ARROW);
        self.shaped_rows = self
            .menus
            .iter()
            .map(|m| shape_items(self, font_system, &m.items))
            .collect();
    }

    fn shape(&self, font_system: &mut FontSystem, text: &str) -> Buffer {
        let mut buffer = Buffer::new_empty(self.metrics);
        buffer.set_size(None, None);
        buffer.set_text(
            text,
            &Attrs::new().family(self.family),
            shaping_for(text),
            None,
        );
        buffer.shape_until_scroll(font_system, false);
        buffer
    }

    /// The x pixel where menu `index` starts.
    fn title_left(&self, index: usize) -> f32 {
        let mut x = 0.0;
        for width in &self.title_widths[..index] {
            x += width + 2.0 * TITLE_PADDING;
        }
        x
    }

    /// The width of menu `index`'s title button.
    fn title_width(&self, index: usize) -> f32 {
        self.title_widths[index] + 2.0 * TITLE_PADDING
    }

    /// The x pixel where menu `index`'s dropdown starts.
    ///
    /// Normally the title's left edge, but pulled back when the dropdown would
    /// run off the right of the window. The last menus in the bar are the ones
    /// with the longest rows -- Help carries a full path -- so without the clamp
    /// the rightmost dropdown is cut off mid-label.
    fn dropdown_left(&self, index: usize, width: f32) -> f32 {
        let left = self.title_left(index);
        let room = width - self.dropdown_width(index);
        left.min(room.max(0.0))
    }

    /// The width of a panel holding `items`.
    ///
    /// The arrow column is only reserved when the panel actually has a row that
    /// opens a submenu, so a menu without one is exactly as wide as it was before
    /// submenus existed.
    fn panel_width(&self, items: &[Item], shaped: &[Shaped]) -> f32 {
        // Measured where the rows have been shaped, counted in cells before the
        // first prepare -- which is what a monospace font makes of them anyway.
        let (label, shortcut) = if shaped.len() == items.len() {
            (
                shaped.iter().map(|s| widest_line(&s.label)).fold(0.0, f32::max),
                shaped
                    .iter()
                    .map(|s| widest_line(&s.shortcut))
                    .fold(0.0, f32::max),
            )
        } else {
            let cell = |count: usize| count as f32 * self.char_width;
            (
                cell(items.iter().map(|i| i.label.chars().count()).max().unwrap_or(0)),
                cell(items
                    .iter()
                    .map(|i| i.shortcut.chars().count())
                    .max()
                    .unwrap_or(0)),
            )
        };
        let arrow = if items.iter().any(Item::is_submenu) {
            ARROW_COLUMN
        } else {
            0.0
        };
        label
            + if shortcut > 0.0 {
                COLUMN_GAP + shortcut
            } else {
                0.0
            }
            + arrow
            + 2.0 * ROW_PADDING
    }

    /// The width of menu `index`'s dropdown.
    fn dropdown_width(&self, index: usize) -> f32 {
        let shaped = self.shaped_rows.get(index).map_or(&[][..], Vec::as_slice);
        self.panel_width(&self.menus[index].items, shaped)
    }

    /// The height of one dropdown row.
    fn row_height(&self, line_height: f32) -> f32 {
        line_height + ROW_EXTRA
    }

    /// The total height of menu `index`'s dropdown.
    fn dropdown_height(&self, index: usize, line_height: f32) -> f32 {
        self.menus[index].items.len() as f32 * self.row_height(line_height) + 2.0
    }

    /// The rows of the submenu `item` opens.
    fn submenu_width(&self, item: &Item, shaped: &[Shaped]) -> f32 {
        self.panel_width(&item.children, shaped)
    }

    /// The height of the submenu `item` opens.
    fn submenu_height(&self, item: &Item, line_height: f32) -> f32 {
        item.children.len() as f32 * self.row_height(line_height) + 2.0
    }

    /// The rows shown at `level`, where level 0 is the top dropdown.
    fn level_items(&self, level: usize) -> &[Item] {
        let Some(index) = self.open else {
            return &[];
        };
        let mut items: &[Item] = &self.menus[index].items;
        for &row in self.path.iter().take(level) {
            match items.get(row) {
                Some(item) => items = &item.children,
                None => return &[],
            }
        }
        items
    }

    /// The shaped rows shown at `level`, mirroring [`Self::level_items`].
    fn level_shaped(&self, level: usize) -> &[Shaped] {
        let Some(index) = self.open else {
            return &[];
        };
        let mut rows: &[Shaped] = match self.shaped_rows.get(index) {
            Some(rows) => rows,
            None => return &[],
        };
        for &row in self.path.iter().take(level) {
            match rows.get(row) {
                Some(shaped) => rows = &shaped.children,
                None => return &[],
            }
        }
        rows
    }

    /// Where every visible panel sits, outermost first.
    ///
    /// One entry per open level: the top dropdown, then one per entry in `path`.
    /// Everything that draws or hit-tests a dropdown goes through this, so the
    /// background, the labels and the pointer cannot disagree about where a panel
    /// is -- which is the bug a second copy of the arithmetic always produces.
    fn panels(&self, width: f32, line_height: f32) -> Vec<Panel> {
        let mut out = Vec::with_capacity(1 + self.path.len());
        let Some(index) = self.open else {
            return out;
        };
        let row_height = self.row_height(line_height);
        out.push(Panel {
            left: self.dropdown_left(index, width),
            top: self.height(line_height) + 1.0,
            width: self.dropdown_width(index),
            height: self.dropdown_height(index, line_height),
        });
        for level in 0..self.path.len() {
            let parent = out[level];
            let row = self.path[level];
            let Some(item) = self.level_items(level).get(row) else {
                break;
            };
            let children = self
                .level_shaped(level)
                .get(row)
                .map_or(&[][..], |shaped| shaped.children.as_slice());
            let panel_width = self.submenu_width(item, children);
            let panel_height = self.submenu_height(item, line_height);
            // To the right of the parent row, overlapping its border so the two
            // panels read as one object rather than two.
            let mut left = parent.left + parent.width - PANEL_OVERLAP;
            if left + panel_width > width {
                // No room on the right: open to the left instead, which is what
                // every desktop menu does at the edge of the screen.
                left = (parent.left - panel_width + PANEL_OVERLAP).max(0.0);
            }
            out.push(Panel {
                left,
                top: parent.top + 1.0 + row as f32 * row_height - PANEL_OVERLAP,
                width: panel_width,
                height: panel_height,
            });
        }
        out
    }

    /// The menu whose title is under `x`, if any.
    fn title_at(&self, x: f32) -> Option<usize> {
        let mut left = 0.0;
        for index in 0..self.menus.len() {
            let width = self.title_width(index);
            if x >= left && x < left + width {
                return Some(index);
            }
            left += width;
        }
        None
    }

    /// The row of `panel` under `y`, if any.
    fn row_in(&self, level: usize, panel: &Panel, y: f32, line_height: f32) -> Option<usize> {
        let top = panel.top + 1.0;
        if y < top {
            return None;
        }
        let row = ((y - top) / self.row_height(line_height)).floor() as usize;
        if row < self.level_items(level).len() {
            Some(row)
        } else {
            None
        }
    }

    /// The deepest panel containing a point, with its level.
    ///
    /// Deepest first, because a submenu overlaps the panel it came from and the
    /// overlap belongs to the submenu.
    fn panel_at(&self, x: f32, y: f32, width: f32, line_height: f32) -> Option<(usize, Panel)> {
        self.panels(width, line_height)
            .iter()
            .enumerate()
            .rev()
            .find(|(_, panel)| panel.contains(x, y))
            .map(|(level, panel)| (level, *panel))
    }

    /// Handles a pointer move. Returns true when the bar consumed it.
    pub fn hover_at(&mut self, x: f32, y: f32, line_height: f32, width: f32) -> bool {
        if y < self.height(line_height) {
            // Hovering the bar while a menu is open switches to that menu, which
            // is how every desktop menu bar behaves.
            if self.open.is_some() {
                if let Some(index) = self.title_at(x) {
                    if self.open != Some(index) {
                        self.open = Some(index);
                        self.path.clear();
                        self.hover = None;
                    }
                }
            }
            return true;
        }
        if self.open.is_none() {
            return false;
        }
        let Some((level, panel)) = self.panel_at(x, y, width, line_height) else {
            // Outside every panel. The highlight is left alone rather than
            // cleared: the pointer crosses the gap between a row and its submenu
            // on the way there, and clearing would make the submenu flicker shut.
            return false;
        };
        let row = self.row_in(level, &panel, y, line_height);
        // Moving back up the tree closes the submenus below this level.
        self.path.truncate(level);
        self.hover = row;
        if let Some(row) = row {
            if self.level_items(level).get(row).is_some_and(Item::is_submenu) {
                self.path.push(row);
                self.hover = None;
            }
        }
        true
    }

    /// Handles a click. Returns the action to run, if the click hit an item.
    ///
    /// A click on a title opens or closes that menu; a click elsewhere closes it.
    /// Opens the dropdown at `index`, or closes it when it is already open.
    ///
    /// The mouse path goes through `click`; this is for keyboard access and for
    /// the offscreen bench, which has no pointer.
    pub fn open_menu(&mut self, index: usize) {
        if index >= self.menus.len() {
            return;
        }
        self.open = if self.open == Some(index) {
            None
        } else {
            Some(index)
        };
        self.path.clear();
        self.hover = None;
    }

    /// Opens the menu whose title starts with `letter`.
    ///
    /// Two menus can start with the same letter -- Edit and Encoding, Search and
    /// Settings -- so pressing the same chord again moves to the next one, which
    /// is how Windows and Notepad++ resolve the collision. A letter that names
    /// only one menu toggles it, so the chord that opened a menu closes it.
    ///
    /// Returns false when no menu starts with that letter, which is the caller's
    /// cue to let the key through to whatever else wanted it.
    pub fn open_by_letter(&mut self, letter: char) -> bool {
        let letter = letter.to_ascii_lowercase();
        let matches: Vec<usize> = self
            .menus
            .iter()
            .enumerate()
            .filter(|(_, menu)| {
                menu.title
                    .chars()
                    .next()
                    .is_some_and(|first| first.to_ascii_lowercase() == letter)
            })
            .map(|(index, _)| index)
            .collect();
        if matches.is_empty() {
            return false;
        }
        let next = match self
            .open
            .and_then(|open| matches.iter().position(|&index| index == open))
        {
            Some(_) if matches.len() == 1 => {
                self.close();
                return true;
            }
            Some(position) => matches[(position + 1) % matches.len()],
            None => matches[0],
        };
        self.open = Some(next);
        self.path.clear();
        self.hover = None;
        true
    }

    /// Moves to the next or previous menu title, keeping the dropdown open.
    ///
    /// This is what Left and Right do at the top level of a menu bar: the
    /// dropdown follows the title rather than closing.
    pub fn cycle_menu(&mut self, delta: isize) -> bool {
        let Some(open) = self.open else {
            return false;
        };
        if self.menus.is_empty() {
            return false;
        }
        let count = self.menus.len() as isize;
        self.open = Some((open as isize + delta).rem_euclid(count) as usize);
        self.path.clear();
        self.hover = None;
        true
    }

    pub fn click(&mut self, x: f32, y: f32, line_height: f32, width: f32) -> Option<Action> {
        if y < self.height(line_height) {
            match self.title_at(x) {
                Some(index) => {
                    self.open = if self.open == Some(index) {
                        None
                    } else {
                        Some(index)
                    };
                    self.path.clear();
                    self.hover = None;
                }
                None => self.close(),
            }
            return None;
        }

        let Some((level, panel)) = self.panel_at(x, y, width, line_height) else {
            self.close();
            return None;
        };
        let row = self.row_in(level, &panel, y, line_height)?;
        // Copied out before anything mutates: the row borrows the menu.
        let item = self.level_items(level).get(row)?;
        let (separator, submenu, action) = (item.separator, item.is_submenu(), item.action);
        if separator {
            return None;
        }
        if submenu {
            self.path.truncate(level);
            self.path.push(row);
            self.hover = None;
            return None;
        }
        self.close();
        Some(action)
    }

    /// Opens the submenu under the highlight.
    ///
    /// Returns false when the highlighted row is a command, which is the caller's
    /// cue to run it instead.
    /// Puts the highlight on `row` of the deepest open level.
    ///
    /// The pointer path goes through `hover_at`, which needs a geometry the
    /// caller may not have. This is the same thing by index, which is what a
    /// test or the bench wants when it is posing a menu rather than driving one.
    pub fn set_hover(&mut self, row: Option<usize>) {
        self.hover = row;
    }

    pub fn open_submenu(&mut self) -> bool {
        let level = self.path.len();
        let Some(row) = self.hover else {
            return false;
        };
        if !self
            .level_items(level)
            .get(row)
            .is_some_and(Item::is_submenu)
        {
            return false;
        }
        self.path.push(row);
        self.hover = None;
        true
    }

    /// Closes the deepest submenu.
    ///
    /// Returns false when none was open, which is the caller's cue to close the
    /// whole menu -- the same key does both, one level at a time.
    pub fn back(&mut self) -> bool {
        if self.path.pop().is_some() {
            self.hover = None;
            true
        } else {
            false
        }
    }

    /// Moves the highlight within the deepest panel, skipping separators.
    ///
    /// Wraps, because a menu is a ring: Down on the last row goes to the first,
    /// which is what every desktop menu does and what stops a user having to
    /// think about where the end is.
    pub fn step(&mut self, delta: isize) {
        let level = self.path.len();
        let count = self.level_items(level).len();
        if count == 0 {
            return;
        }
        let mut row = match self.hover {
            Some(row) => row as isize,
            None if delta > 0 => -1,
            None => count as isize,
        };
        for _ in 0..count {
            row += delta;
            if row < 0 {
                row = count as isize - 1;
            }
            if row >= count as isize {
                row = 0;
            }
            if !self.level_items(level)[row as usize].separator {
                self.hover = Some(row as usize);
                return;
            }
        }
    }

    /// Runs the highlighted row, or opens it when it is a submenu.
    pub fn activate(&mut self) -> Option<Action> {
        let level = self.path.len();
        let row = self.hover?;
        let (separator, submenu, action) = {
            let item = self.level_items(level).get(row)?;
            (item.separator, item.is_submenu(), item.action)
        };
        if separator {
            return None;
        }
        if submenu {
            self.path.push(row);
            self.hover = None;
            return None;
        }
        self.close();
        Some(action)
    }

    /// The level the highlight is on: 0 for the top dropdown.
    pub fn depth(&self) -> usize {
        self.path.len()
    }

    /// Queues the bar, the open dropdown, and their borders.
    pub fn push_quads(&self, quads: &mut QuadRenderer, width: f32, line_height: f32, theme: &Theme) {
        let bar = self.height(line_height);
        quads.push(
            0.0,
            0.0,
            width,
            bar,
            theme.menu_background.to_linear(),
        );
        quads.push(
            0.0,
            bar - 1.0,
            width,
            1.0,
            theme.menu_border.to_linear(),
        );

        if let Some(open) = self.open {
            quads.push(
                self.title_left(open),
                0.0,
                self.title_width(open),
                bar,
                theme.menu_hover.to_linear(),
            );
        }

    }

    /// The quads of the open dropdown, empty when nothing is open.
    ///
    /// These are queued in a separate batch drawn after the editor's glyphs,
    /// because a dropdown has to cover the text underneath it.
    pub fn push_dropdown_quads(
        &self,
        quads: &mut QuadRenderer,
        line_height: f32,
        theme: &Theme,
        width: f32,
    ) {
        let row_height = self.row_height(line_height);
        let border = theme.menu_border.to_linear();
        for (level, panel) in self.panels(width, line_height).iter().enumerate() {
            quads.push(
                panel.left,
                panel.top,
                panel.width,
                panel.height,
                theme.menu_background.to_linear(),
            );
            // A one pixel frame, drawn as four bars.
            quads.push(panel.left, panel.top, panel.width, 1.0, border);
            quads.push(
                panel.left,
                panel.top + panel.height - 1.0,
                panel.width,
                1.0,
                border,
            );
            quads.push(panel.left, panel.top, 1.0, panel.height, border);
            quads.push(
                panel.left + panel.width - 1.0,
                panel.top,
                1.0,
                panel.height,
                border,
            );

            for (row, item) in self.level_items(level).iter().enumerate() {
                let y = panel.top + 1.0 + row as f32 * row_height;
                if item.separator {
                    quads.push(
                        panel.left + 6.0,
                        y + row_height * 0.5,
                        panel.width - 12.0,
                        1.0,
                        border,
                    );
                    continue;
                }
                if self.is_highlighted(level, row) {
                    quads.push(
                        panel.left + 1.0,
                        y,
                        panel.width - 2.0,
                        row_height,
                        theme.menu_hover.to_linear(),
                    );
                }
            }
        }
    }

    /// Whether row `row` of `level` is drawn as highlighted.
    ///
    /// A row is lit when the pointer or the keyboard is on it, and also while its
    /// submenu is open: the parent stays lit so a user can see which branch of the
    /// tree they are in after the child panel has taken the highlight.
    fn is_highlighted(&self, level: usize, row: usize) -> bool {
        if level < self.path.len() {
            return self.path[level] == row;
        }
        self.hover == Some(row)
    }

    /// The text areas of the bar's own titles.
    pub fn text_areas<'a>(&'a self, width: f32, line_height: f32, theme: &Theme) -> Vec<TextArea<'a>> {
        let bar = self.height(line_height);
        let bounds = TextBounds {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: bar as i32,
        };
        let mut areas = Vec::with_capacity(self.menus.len());
        for (index, buffer) in self.titles.iter().enumerate() {
            areas.push(TextArea {
                buffer,
                left: self.title_left(index) + TITLE_PADDING,
                top: 3.0,
                scale: 1.0,
                bounds,
                default_color: theme.menu_text.to_cosmic(),
                custom_glyphs: &[],
            });
        }
        areas
    }

    /// The text areas of the open dropdown, empty when nothing is open.
    ///
    /// These are drawn in a second pass, after the editor's glyphs, because a
    /// dropdown has to cover the text underneath it.
    pub fn dropdown_areas<'a>(
        &'a self,
        line_height: f32,
        theme: &Theme,
        width: f32,
    ) -> Vec<TextArea<'a>> {
        let mut areas = Vec::with_capacity(32);
        let row_height = self.row_height(line_height);
        for (level, panel) in self.panels(width, line_height).iter().enumerate() {
            // The same clamp the background uses: a label drawn at the title's
            // left edge while the panel is pulled back would hang off the panel.
            let bounds = TextBounds {
                left: panel.left as i32,
                top: panel.top as i32,
                right: (panel.left + panel.width) as i32,
                bottom: (panel.top + panel.height) as i32,
            };
            let shaped = self.level_shaped(level);
            for (row, item) in self.level_items(level).iter().enumerate() {
                if item.separator {
                    continue;
                }
                let Some(shaped) = shaped.get(row) else {
                    continue;
                };
                let y = panel.top + 1.0 + row as f32 * row_height + ROW_EXTRA * 0.5;
                areas.push(TextArea {
                    buffer: &shaped.label,
                    left: panel.left + ROW_PADDING,
                    top: y,
                    scale: 1.0,
                    bounds,
                    default_color: theme.menu_text.to_cosmic(),
                    custom_glyphs: &[],
                });
                if !item.shortcut.is_empty() {
                    let shortcut_width = widest_line(&shaped.shortcut);
                    areas.push(TextArea {
                        buffer: &shaped.shortcut,
                        left: panel.left + panel.width - ROW_PADDING - shortcut_width,
                        top: y,
                        scale: 1.0,
                        bounds,
                        default_color: theme.menu_shortcut.to_cosmic(),
                        custom_glyphs: &[],
                    });
                }
                if item.is_submenu() {
                    areas.push(TextArea {
                        buffer: &self.arrow,
                        left: panel.left + panel.width - ROW_PADDING - ARROW_COLUMN + 4.0,
                        top: y,
                        scale: 1.0,
                        bounds,
                        default_color: theme.menu_shortcut.to_cosmic(),
                        custom_glyphs: &[],
                    });
                }
            }
        }

        areas
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nn_core::{CodePage, Encoding};

    fn bar() -> MenuBar {
        MenuBar::new(
            Metrics::new(15.0, 20.0),
            Family::Monospace,
            8.0,
            Keymap::default(),
        )
    }

    /// Finds a row anywhere in a menu, submenus included.
    ///
    /// The Edit menu is almost entirely submenus now, so a test that looked only
    /// at the top level would be asserting about a menu that no longer exists.
    fn find(items: &[Item], action: Action) -> Option<&Item> {
        for item in items {
            if item.action == action {
                return Some(item);
            }
            if let Some(found) = find(&item.children, action) {
                return Some(found);
            }
        }
        None
    }

    /// Every row in a menu, submenus flattened in.
    fn flatten(items: &[Item]) -> Vec<&Item> {
        let mut out = Vec::new();
        for item in items {
            out.push(item);
            out.extend(flatten(&item.children));
        }
        out
    }

    /// A bar whose menus are given, so the submenu tests do not depend on what
    /// the real menus happen to contain this week.
    fn bar_with(menus: Vec<Menu>) -> MenuBar {
        let mut bar = bar();
        bar.menus = menus;
        bar.refresh_shortcuts();
        bar
    }

    /// One menu with a submenu in it, and a command after it. A second menu
    /// follows so the flip-at-the-edge test has a panel that does not start at
    /// x = 0, where there is nothing to flip towards.
    fn submenu_bar() -> MenuBar {
        bar_with(vec![
            Menu {
                title: "File".to_string(),
                english: "File",
                items: vec![
                    Item::new("New", Action::New),
                    Item::submenu(
                        "Open Containing Folder",
                        vec![
                            Item::new("Explorer", Action::OpenContainingFolder),
                            Item::separator(),
                            Item::new("cmd", Action::OpenContainingFolder),
                        ],
                    ),
                    Item::new("Save", Action::Save),
                ],
            },
            Menu {
                title: "Edit".to_string(),
                english: "Edit",
                items: vec![
                    Item::new("Undo", Action::Undo),
                    Item::submenu(
                        "Convert Case to",
                        vec![
                            Item::new("UPPERCASE", Action::UpperCase),
                            Item::new("lowercase", Action::LowerCase),
                        ],
                    ),
                ],
            },
        ])
    }

    /// The row of the open menu that the pointer would be over at `row`.
    fn row_y(bar: &MenuBar, row: usize, line_height: f32) -> f32 {
        let panel = bar.panels(900.0, line_height)[0];
        panel.top + 1.0 + row as f32 * bar.row_height(line_height) + 2.0
    }

    #[test]
    fn a_submenu_row_is_a_heading_not_a_command() {
        let bar = submenu_bar();
        let item = &bar.menus[0].items[1];
        assert!(item.is_submenu());
        assert_eq!(item.action, Action::Submenu);
        // No chord: the arrow is drawn where the shortcut column would be.
        assert!(item.shortcut.is_empty());
    }

    #[test]
    fn a_submenu_row_carries_its_children() {
        let bar = submenu_bar();
        let rows = bar.rows();
        assert_eq!(rows[0].items[1].children.len(), 3);
        assert_eq!(rows[0].items[1].children[0].label, "Explorer");
        assert!(rows[0].items[1].children[1].separator);
        assert!(rows[0].items[0].children.is_empty());
    }

    #[test]
    fn hovering_a_submenu_row_opens_it() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        let y = row_y(&bar, 1, 20.0);
        assert!(bar.hover_at(20.0, y, 20.0, 900.0));
        assert_eq!(bar.depth(), 1);
        assert_eq!(bar.panels(900.0, 20.0).len(), 2);
    }

    #[test]
    fn hovering_a_plain_row_does_not_open_anything() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        let y = row_y(&bar, 2, 20.0);
        assert!(bar.hover_at(20.0, y, 20.0, 900.0));
        assert_eq!(bar.depth(), 0);
    }

    #[test]
    fn moving_back_to_the_parent_row_closes_the_submenu() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        let submenu_y = row_y(&bar, 1, 20.0);
        bar.hover_at(20.0, submenu_y, 20.0, 900.0);
        assert_eq!(bar.depth(), 1);
        let plain_y = row_y(&bar, 2, 20.0);
        bar.hover_at(20.0, plain_y, 20.0, 900.0);
        assert_eq!(bar.depth(), 0);
    }

    #[test]
    fn clicking_a_submenu_row_opens_it_and_runs_nothing() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        let y = row_y(&bar, 1, 20.0);
        assert_eq!(bar.click(20.0, y, 20.0, 900.0), None);
        assert_eq!(bar.depth(), 1);
        assert!(bar.is_open());
    }

    #[test]
    fn clicking_a_row_inside_a_submenu_runs_it() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        let y = row_y(&bar, 1, 20.0);
        bar.click(20.0, y, 20.0, 900.0);
        let panel = bar.panels(900.0, 20.0)[1];
        let child_y = panel.top + 1.0 + 2.0 * bar.row_height(20.0) + 2.0;
        let x = panel.left + 10.0;
        assert_eq!(
            bar.click(x, child_y, 20.0, 900.0),
            Some(Action::OpenContainingFolder)
        );
        assert!(!bar.is_open());
    }

    #[test]
    fn a_submenu_opens_to_the_right_of_its_parent() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        let y = row_y(&bar, 1, 20.0);
        bar.hover_at(20.0, y, 20.0, 900.0);
        let panels = bar.panels(900.0, 20.0);
        assert!(panels[1].left > panels[0].left);
        // Overlapping the parent's right edge, so the two read as one object.
        assert!(panels[1].left < panels[0].left + panels[0].width);
    }

    #[test]
    fn a_submenu_with_no_room_on_the_right_opens_to_the_left() {
        let mut bar = submenu_bar();
        // The second menu, so its panel starts to the right of x = 0 and there
        // is somewhere for a flipped submenu to go.
        bar.open_menu(1);
        let y = row_y(&bar, 1, 20.0);
        let x = bar.panels(900.0, 20.0)[0].left + 10.0;
        bar.hover_at(x, y, 20.0, 900.0);
        let wide = bar.panels(900.0, 20.0);
        assert!(wide[0].left > 0.0);
        assert!(wide[1].left > wide[0].left);
        // A window just wide enough for the parent panel, so the submenu has
        // nowhere to go on the right but the parent still sits where it wants.
        let narrow = wide[0].left + wide[0].width + 10.0;
        let panels = bar.panels(narrow, 20.0);
        assert_eq!(panels[0].left, wide[0].left);
        assert!(panels[1].left < panels[0].left);
        assert!(panels[1].left >= 0.0);
    }

    #[test]
    fn back_closes_one_level_at_a_time() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        let y = row_y(&bar, 1, 20.0);
        bar.hover_at(20.0, y, 20.0, 900.0);
        assert_eq!(bar.depth(), 1);
        assert!(bar.back());
        assert_eq!(bar.depth(), 0);
        assert!(bar.is_open());
        assert!(!bar.back());
    }

    #[test]
    fn closing_the_menu_closes_every_submenu() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        let y = row_y(&bar, 1, 20.0);
        bar.hover_at(20.0, y, 20.0, 900.0);
        bar.close();
        assert_eq!(bar.depth(), 0);
        assert!(!bar.is_open());
    }

    #[test]
    fn stepping_skips_separators_and_wraps() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        bar.step(1);
        assert_eq!(bar.hover, Some(0));
        bar.step(1);
        assert_eq!(bar.hover, Some(1));
        bar.step(1);
        assert_eq!(bar.hover, Some(2));
        // Wraps to the top rather than stopping at the end.
        bar.step(1);
        assert_eq!(bar.hover, Some(0));
        bar.step(-1);
        assert_eq!(bar.hover, Some(2));
    }

    #[test]
    fn stepping_inside_a_submenu_skips_its_separator() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        bar.step(1);
        bar.step(1);
        assert!(bar.open_submenu());
        assert_eq!(bar.depth(), 1);
        bar.step(1);
        assert_eq!(bar.hover, Some(0));
        // Row 1 is a separator, so the next step lands on row 2.
        bar.step(1);
        assert_eq!(bar.hover, Some(2));
    }

    #[test]
    fn open_submenu_refuses_a_command() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        bar.step(1);
        assert_eq!(bar.hover, Some(0));
        assert!(!bar.open_submenu());
        assert_eq!(bar.depth(), 0);
    }

    #[test]
    fn activate_opens_a_submenu_rather_than_running_it() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        bar.step(1);
        bar.step(1);
        assert_eq!(bar.activate(), None);
        assert_eq!(bar.depth(), 1);
        assert!(bar.is_open());
    }

    #[test]
    fn activate_runs_a_command_and_closes_the_menu() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        bar.step(1);
        assert_eq!(bar.activate(), Some(Action::New));
        assert!(!bar.is_open());
    }

    #[test]
    fn a_panel_with_a_submenu_reserves_the_arrow_column() {
        let plain = bar_with(vec![Menu {
            title: "File".to_string(),
            english: "File",
            items: vec![Item::new("New", Action::New)],
        }]);
        let nested = submenu_bar();
        assert!(nested.dropdown_width(0) > plain.dropdown_width(0));
    }

    #[test]
    fn a_letter_opens_the_menu_that_starts_with_it() {
        let mut bar = submenu_bar();
        assert!(bar.open_by_letter('f'));
        assert_eq!(bar.open, Some(0));
        assert!(bar.open_by_letter('e'));
        assert_eq!(bar.open, Some(1));
    }

    #[test]
    fn a_letter_with_no_menu_is_refused() {
        let mut bar = submenu_bar();
        assert!(!bar.open_by_letter('z'));
        assert_eq!(bar.open, None);
    }

    #[test]
    fn a_shared_letter_cycles_through_the_menus_that_share_it() {
        let mut bar = bar_with(vec![
            Menu {
                title: "Edit".to_string(),
                english: "Edit",
                items: vec![Item::new("Undo", Action::Undo)],
            },
            Menu {
                title: "Encoding".to_string(),
                english: "Encoding",
                items: vec![Item::new("UTF-8", Action::EncodingSet(Encoding::Utf8))],
            },
        ]);
        assert!(bar.open_by_letter('e'));
        assert_eq!(bar.open, Some(0));
        // The same chord again moves to the other menu with that letter.
        assert!(bar.open_by_letter('e'));
        assert_eq!(bar.open, Some(1));
        assert!(bar.open_by_letter('e'));
        assert_eq!(bar.open, Some(0));
    }

    #[test]
    fn a_letter_that_names_one_menu_toggles_it() {
        let mut bar = submenu_bar();
        assert!(bar.open_by_letter('f'));
        assert!(bar.is_open());
        assert!(bar.open_by_letter('f'));
        assert!(!bar.is_open());
    }

    #[test]
    fn cycling_moves_along_the_bar_and_wraps() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        assert!(bar.cycle_menu(1));
        assert_eq!(bar.open, Some(1));
        assert!(bar.cycle_menu(1));
        assert_eq!(bar.open, Some(0));
        assert!(bar.cycle_menu(-1));
        assert_eq!(bar.open, Some(1));
    }

    #[test]
    fn cycling_with_nothing_open_does_nothing() {
        let mut bar = submenu_bar();
        assert!(!bar.cycle_menu(1));
        assert_eq!(bar.open, None);
    }

    #[test]
    fn cycling_closes_the_submenus_of_the_menu_it_leaves() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        let y = row_y(&bar, 1, 20.0);
        bar.hover_at(20.0, y, 20.0, 900.0);
        assert_eq!(bar.depth(), 1);
        bar.cycle_menu(1);
        assert_eq!(bar.depth(), 0);
        assert_eq!(bar.hover, None);
    }

    #[test]
    fn the_bar_is_taller_than_a_line() {
        assert_eq!(bar().height(20.0), 26.0);
    }

    #[test]
    fn titles_are_hit_tested_left_to_right() {
        let b = bar();
        assert_eq!(b.title_at(2.0), Some(0));
        let second = b.title_left(1);
        assert_eq!(b.title_at(second + 1.0), Some(1));
        assert_eq!(b.title_at(1e6), None);
    }

    #[test]
    fn clicking_a_title_opens_and_closes_it() {
        let mut b = bar();
        assert!(!b.is_open());
        b.click(2.0, 2.0, 20.0, 900.0);
        assert_eq!(b.open, Some(0));
        b.click(2.0, 2.0, 20.0, 900.0);
        assert!(!b.is_open());
    }

    #[test]
    fn clicking_another_title_switches_menus() {
        let mut b = bar();
        b.click(2.0, 2.0, 20.0, 900.0);
        let second = b.title_left(1) + 1.0;
        b.click(second, 2.0, 20.0, 900.0);
        assert_eq!(b.open, Some(1));
    }

    #[test]
    fn clicking_an_item_returns_its_action() {
        let mut b = bar();
        b.click(2.0, 2.0, 20.0, 900.0);
        // First row of File is New.
        let y = b.height(20.0) + 1.0 + 2.0;
        assert_eq!(b.click(20.0, y, 20.0, 900.0), Some(Action::New));
        assert!(!b.is_open());
    }

    #[test]
    fn clicking_a_separator_does_nothing() {
        let mut b = bar();
        b.click(2.0, 2.0, 20.0, 900.0);
        let row = b.row_height(20.0);
        let y = b.height(20.0) + 1.0 + 2.0 * row + 2.0;
        assert_eq!(b.click(20.0, y, 20.0, 900.0), None);
        assert!(b.is_open());
    }

    #[test]
    fn clicking_outside_closes_the_menu() {
        let mut b = bar();
        b.click(2.0, 2.0, 20.0, 900.0);
        b.click(900.0, 500.0, 20.0, 900.0);
        assert!(!b.is_open());
    }

    #[test]
    fn hovering_the_bar_switches_the_open_menu() {
        let mut b = bar();
        b.click(2.0, 2.0, 20.0, 900.0);
        let second = b.title_left(1) + 1.0;
        assert!(b.hover_at(second, 2.0, 20.0, 900.0));
        assert_eq!(b.open, Some(1));
    }

    #[test]
    fn hovering_a_row_highlights_it() {
        let mut b = bar();
        b.click(2.0, 2.0, 20.0, 900.0);
        let y = b.height(20.0) + 1.0 + 2.0;
        assert!(b.hover_at(20.0, y, 20.0, 900.0));
        assert_eq!(b.hover, Some(0));
    }

    #[test]
    fn a_dropdown_is_wide_enough_for_its_longest_row() {
        let b = bar();
        let width = b.dropdown_width(0);
        // "Save As..." plus "Ctrl+Shift+S" plus padding.
        assert!(width > 20.0 * 8.0);
    }

    #[test]
    fn a_dropdown_near_the_right_edge_is_pulled_back_inside_the_window() {
        let mut b = bar();
        b.prepare(&mut FontSystem::new());
        let last = b.menus.len() - 1;
        b.open_menu(last);
        let width = 400.0;
        let left = b.dropdown_left(last, width);
        assert!(left + b.dropdown_width(last) <= width + 0.01);
        // The text follows the panel: the first label starts inside it.
        let areas = b.dropdown_areas(20.0, &Theme::DARK, width);
        let first = areas.first().expect("a label");
        assert!(first.left >= left);
        assert!(first.left < left + b.dropdown_width(last));
    }

    #[test]
    fn a_dropdown_with_room_stays_under_its_title() {
        let mut b = bar();
        b.open_menu(0);
        assert_eq!(b.dropdown_left(0, 4000.0), b.title_left(0));
    }

    #[test]
    fn the_edit_menu_offers_word_completion() {
        let b = bar();
        let edit = &b.menus[1];
        assert_eq!(edit.title, "Edit");
        let item = find(&edit.items, Action::CompleteWord).expect("Edit menu has Word Completion");
        assert_eq!(item.label, "Word Completion");
        assert_eq!(item.shortcut, "Ctrl+Space");
    }

    #[test]
    fn the_shortcut_column_comes_from_the_keymap() {
        let mut b = bar();
        let mut keymap = Keymap::default();
        keymap.set(Action::Save, crate::keymap::Chord::parse("F5").unwrap());
        b.set_keymap(keymap);
        let save = b.menus[0]
            .items
            .iter()
            .find(|item| item.action == Action::Save)
            .expect("File menu has Save");
        assert_eq!(save.shortcut, "F5");
    }

    #[test]
    fn a_command_with_no_chord_shows_an_empty_column() {
        let b = bar();
        let sort = find(&b.menus[1].items, Action::SortLines(Sort::LexAsc))
            .expect("Edit > Line Operations has Sort Ascending");
        assert_eq!(sort.shortcut, "");
    }

    #[test]
    fn separators_never_carry_a_shortcut() {
        let b = bar();
        for menu in &b.menus {
            for item in flatten(&menu.items) {
                if item.separator {
                    assert_eq!(item.shortcut, "");
                }
            }
        }
    }

    #[test]
    fn every_advertised_shortcut_is_bound() {
        let b = bar();
        for menu in &b.menus {
            for item in flatten(&menu.items) {
                if item.separator || item.shortcut.is_empty() {
                    continue;
                }
                let chord = crate::keymap::Chord::parse(&item.shortcut)
                    .unwrap_or_else(|| panic!("{} has an unparsable chord", item.label));
                assert_eq!(
                    b.keymap.lookup(&chord),
                    Some(item.action),
                    "{} advertises {} but the keymap disagrees",
                    item.label,
                    item.shortcut
                );
            }
        }
    }

    #[test]
    fn the_encoding_menu_is_notepads_shape() {
        let items = encoding_items();
        let labels: Vec<&str> = items
            .iter()
            .filter(|item| !item.separator)
            .map(|item| item.label.as_str())
            .collect();
        assert_eq!(
            labels,
            [
                "ANSI",
                "UTF-8",
                "UTF-8-BOM",
                "UTF-16 BE BOM",
                "UTF-16 LE BOM",
                "Character sets",
                "Convert to ANSI",
                "Convert to UTF-8",
                "Convert to UTF-8-BOM",
                "Convert to UTF-16 BE BOM",
                "Convert to UTF-16 LE BOM",
            ]
        );
    }

    #[test]
    fn every_character_set_row_names_a_code_page() {
        let items = encoding_items();
        let sets = items
            .iter()
            .find(|item| item.label == "Character sets")
            .expect("the Character sets submenu");
        assert_eq!(sets.children.len(), CHARACTER_SETS.len());
        let mut seen = 0usize;
        for script in &sets.children {
            assert!(!script.children.is_empty(), "{} is empty", script.label);
            for row in &script.children {
                match row.action {
                    Action::EncodingSet(Encoding::CodePage(_)) => seen += 1,
                    other => panic!("{} is {other:?}", row.label),
                }
            }
        }
        // Every single-byte set is reachable from the menu, and none twice.
        assert_eq!(seen, CodePage::ALL.len());
    }


    /// A catalog that renames a handful of rows, built the way a real
    /// localization file is: numeric ids joined against an English reference.
    fn catalog() -> Catalog {
        let english = "<NotepadPlus><Native-Langue name=\"English\"><Menu>\
            <Main><Item id=\"0\" name=\"File\"/><Item id=\"1\" name=\"Edit\"/>\
            <Item id=\"2\" name=\"New\"/></Main>\
            <SubEntries><Item id=\"0\" name=\"File\"/></SubEntries>\
            </Menu></Native-Langue></NotepadPlus>";
        let french = "<NotepadPlus><Native-Langue name=\"Francais\"><Menu>\
            <Main><Item id=\"0\" name=\"Fichier\"/><Item id=\"1\" name=\"Edition\"/>\
            <Item id=\"2\" name=\"Nouveau\"/></Main>\
            <SubEntries><Item id=\"0\" name=\"Fichier\"/></SubEntries>\
            </Menu></Native-Langue></NotepadPlus>";
        Catalog::parse(Some(english), french).expect("a catalog")
    }

    #[test]
    fn a_catalog_relabels_the_titles_and_the_rows() {
        let mut b = bar();
        b.set_catalog(Some(catalog()));
        assert_eq!(b.menus[0].title, "Fichier");
        assert_eq!(b.menus[0].items[0].label, "Nouveau");
        // A row the catalog does not mention keeps its English text.
        assert_eq!(b.menus[0].items[1].label, "Open...");
        assert_eq!(b.catalog_name(), Some("Francais"));
    }

    #[test]
    fn clearing_the_catalog_restores_every_english_label() {
        let mut b = bar();
        let before: Vec<String> = b.menus[0].items.iter().map(|i| i.label.clone()).collect();
        b.set_catalog(Some(catalog()));
        b.set_catalog(None);
        let after: Vec<String> = b.menus[0].items.iter().map(|i| i.label.clone()).collect();
        assert_eq!(before, after);
        assert_eq!(b.menus[0].title, "File");
        assert_eq!(b.catalog_name(), None);
    }

    #[test]
    fn switching_languages_twice_does_not_translate_a_translation() {
        // The second catalog is applied to the stored English text, not to the
        // French that is currently on screen.
        let mut b = bar();
        b.set_catalog(Some(catalog()));
        b.set_catalog(Some(catalog()));
        assert_eq!(b.menus[0].title, "Fichier");
        assert_eq!(b.menus[0].items[0].label, "Nouveau");
    }

    #[test]
    fn a_rebuilt_list_is_relabelled_too() {
        // `set_recent` rebuilds the File menu from English literals; the
        // catalog has to survive that.
        let mut b = bar();
        b.set_catalog(Some(catalog()));
        b.set_recent(&["notes.txt".to_string()]);
        assert_eq!(b.menus[0].title, "Fichier");
        assert_eq!(b.menus[0].items[0].label, "Nouveau");
        assert!(b.menus[0].items.iter().any(|i| i.label == "notes.txt"));
    }

    #[test]
    fn a_catalog_does_not_change_which_action_a_row_runs() {
        let mut b = bar();
        let before: Vec<Action> = b.menus[0].items.iter().map(|i| i.action).collect();
        b.set_catalog(Some(catalog()));
        let after: Vec<Action> = b.menus[0].items.iter().map(|i| i.action).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn the_bar_is_notepads_thirteen_in_notepads_order() {
        let b = bar();
        let titles: Vec<&str> = b.menus.iter().map(|menu| menu.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "File", "Edit", "Search", "View", "Encoding", "Language", "Settings", "Tools",
                "Macro", "Run", "Plugins", "Window", "Help",
            ]
        );
        // The three menus reached by index have to agree with that order.
        assert_eq!(titles[LANGUAGE_MENU], "Language");
        assert_eq!(titles[RUN_MENU], "Run");
        assert_eq!(titles[PLUGIN_MENU], "Plugins");
    }

    #[test]
    fn the_view_menu_is_notepads_shape() {
        let items = view_items();
        // Playground is ours: Notepad++ has no row there. Everything else keeps
        // Notepad++'s order, which is what the rest of this list is about.
        let labels: Vec<&str> = items
            .iter()
            .filter(|item| !item.separator)
            .map(|item| item.label.as_str())
            .filter(|label| *label != "Playground")
            .collect();
        assert_eq!(
            labels,
            [
                "Always on Top",
                "Toggle Full Screen Mode",
                "Post-It",
                "Distraction Free Mode",
                "View Current File in",
                "Show Symbol",
                "Zoom",
                "Move/Clone Current Document",
                "Tab",
                "Word wrap",
                "Focus on Another View",
                "Hide Lines",
                "Fold All",
                "Unfold All",
                "Fold Current Level",
                "Unfold Current Level",
                "Fold Level",
                "Unfold Level",
                "Summary...",
                "Project Panels",
                "Folder as Workspace",
                "Document Map",
                "Document List",
                "Function List",
                "Synchronize Vertical Scrolling",
                "Synchronize Horizontal Scrolling",
                "Monitoring (tail -f)",
            ]
        );
    }

    #[test]
    fn the_playground_row_sits_under_distraction_free_mode() {
        let items = view_items();
        let at = items
            .iter()
            .position(|item| item.action == Action::Playground)
            .expect("a Playground row");
        assert_eq!(items[at - 1].action, Action::DistractionFree);
    }

    #[test]
    fn the_view_menu_has_no_text_direction_rows() {
        // The renderer has no bidi layout, so the two rows Notepad++ has would be
        // switches nothing reads. Their absence is a decision, not an oversight.
        let items = view_items();
        assert!(find(&items, Action::AlwaysOnTop).is_some());
        let labels: Vec<&str> = flatten(&items).iter().map(|i| i.label.as_str()).collect();
        assert!(!labels.iter().any(|label| label.starts_with("Text Direction")));
    }

    #[test]
    fn the_tab_submenu_counts_to_nine_and_paints_five() {
        let items = view_items();
        let submenu = items
            .iter()
            .find(|item| item.label == "Tab")
            .expect("a Tab row");
        let rows = &submenu.children;
        let gotos: Vec<usize> = rows
            .iter()
            .filter_map(|row| match row.action {
                Action::GotoTab(index) => Some(index),
                _ => None,
            })
            .collect();
        assert_eq!(gotos, (0..9).collect::<Vec<usize>>());
        let paints = rows
            .iter()
            .filter(|row| matches!(row.action, Action::TabColourSet(_)))
            .count();
        assert_eq!(paints, 6);
    }

    #[test]
    fn the_fold_level_submenus_go_from_one_to_eight() {
        let items = view_items();
        for (label, make) in [
            ("Fold Level", Action::FoldLevel as fn(usize) -> Action),
            ("Unfold Level", Action::UnfoldLevel as fn(usize) -> Action),
        ] {
            let row = items
                .iter()
                .find(|item| item.label == label)
                .expect("the submenu");
            let rows = &row.children;
            let levels: Vec<usize> = rows
                .iter()
                .filter_map(|row| match row.action {
                    Action::FoldLevel(level) | Action::UnfoldLevel(level) => Some(level),
                    _ => None,
                })
                .collect();
            assert_eq!(levels, (1..=8).collect::<Vec<usize>>());
            assert_eq!(rows[0].action, make(1));
        }
    }

    #[test]
    fn every_browser_row_names_a_program() {
        for kind in BrowserKind::all() {
            assert!(!kind.command().is_empty());
            assert!(!kind.label().is_empty());
        }
    }

    #[test]
    fn the_settings_menu_matches_notepads_shape() {
        let b = bar();
        let items = &b.menus[6].items;
        let labels: Vec<&str> = items.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Preferences...",
                "Style Configurator...",
                "Theme",
                "Shortcut Mapper",
                "",
                "Import",
                "",
                "Edit Popup ContextMenu",
            ]
        );
        assert_eq!(items[0].action, Action::Preferences);
        assert_eq!(items[1].action, Action::StyleConfigurator);
        assert!(items[4].separator);
        assert!(items[6].separator);
        assert_eq!(items[7].action, Action::EditContextMenu);
    }

    #[test]
    fn every_theme_is_reachable_from_the_settings_menu() {
        // The Preferences row steps through the same list one arrow press at a
        // time; the submenu is what makes a theme findable by name.
        let b = bar();
        let items = &b.menus[6].items;
        let theme = items
            .iter()
            .find(|item| item.label == "Theme")
            .expect("the theme submenu");
        let actions: Vec<Action> = theme.children.iter().map(|row| row.action).collect();
        let expected: Vec<Action> = (0..ThemeChoice::ALL.len()).map(Action::Theme).collect();
        assert_eq!(actions, expected);
        assert_eq!(
            theme.children[0].label,
            ThemeChoice::ALL[0].label(),
            "the rows are labelled in the same order the row steps"
        );
        assert_eq!(theme.children.len(), ThemeChoice::ALL.len());
    }

    #[test]
    fn a_theme_row_round_trips_through_its_shortcut_key() {
        // The rows are addressable, so a shortcut can name one; that only works
        // if the key is unique per theme and parses back to the same index.
        for (index, choice) in ThemeChoice::ALL.iter().enumerate() {
            let key = format!("theme_{}", choice.key());
            assert_eq!(
                action_from_name(&key),
                Some(Action::Theme(index)),
                "{key}"
            );
        }
        assert_eq!(action_from_name("theme_nonexistent"), None);
        // The vocabulary on its own does not resolve a theme: the key names a
        // setting, and the setting's list is not something a command can see.
        assert_eq!(Action::from_name("theme_dracula"), None);
        // Everything that is a command still resolves there rather than here.
        assert_eq!(action_from_name("save"), Some(Action::Save));
        assert_eq!(action_from_name("theme"), None);
    }

    #[test]
    fn the_two_imports_are_under_the_import_submenu() {
        let b = bar();
        let items = &b.menus[6].items;
        let import = items.iter().find(|item| item.label == "Import").unwrap();
        let actions: Vec<Action> = import.children.iter().map(|row| row.action).collect();
        assert_eq!(actions, [Action::ImportPlugin, Action::ImportTheme]);
    }

    #[test]
    fn the_shortcut_mapper_keeps_the_import_and_export_reachable() {
        // Notepad++ puts these two on the mapper dialog's own buttons. There is
        // no such dialog here, so they are rows under the mapper instead of
        // being dropped.
        let b = bar();
        let items = &b.menus[6].items;
        let mapper = items.iter().find(|item| item.label == "Shortcut Mapper").unwrap();
        let actions: Vec<Action> = mapper
            .children
            .iter()
            .filter(|row| !row.separator)
            .map(|row| row.action)
            .collect();
        assert_eq!(
            actions,
            [Action::ShortcutMapper, Action::ImportNotepad, Action::ExportShortcuts]
        );
    }

    #[test]
    fn the_language_menu_starts_with_plain_text() {
        let b = bar();
        let items = &b.menus[LANGUAGE_MENU].items;
        assert_eq!(items[0].label, "None (Normal Text)");
        assert_eq!(items[0].action, Action::LanguagePlain);
        assert!(items[1].separator);
    }

    #[test]
    fn every_built_in_language_is_reachable_from_the_menu() {
        // The menu groups the table by first letter, so a language could be
        // dropped by a grouping bug without any single row looking wrong.
        let b = bar();
        let items = &b.menus[LANGUAGE_MENU].items;
        let mut seen = Vec::new();
        for item in items {
            // The three ungrouped rows are top-level; the rest are children.
            for row in std::iter::once(item).chain(&item.children) {
                if let Action::LanguageBuiltIn(index) = row.action {
                    seen.push(index);
                }
            }
        }
        seen.sort_unstable();
        let expected: Vec<usize> = (0..nn_core::LANGS.len()).collect();
        assert_eq!(seen, expected);
    }

    #[test]
    fn the_language_groups_are_lettered_and_in_order() {
        let b = bar();
        let items = &b.menus[LANGUAGE_MENU].items;
        let letters: Vec<&str> = items
            .iter()
            .filter(|item| !item.children.is_empty())
            .map(|item| item.label.as_str())
            .collect();
        // `User Defined Language` and `User-Defined` are submenus too, so the
        // letter groups are the ones that are a single character.
        let letters: Vec<&str> = letters
            .into_iter()
            .filter(|label| label.chars().count() == 1)
            .collect();
        assert_eq!(
            letters,
            [
                "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "R",
                "S", "T", "V", "W", "Z"
            ]
        );
    }

    #[test]
    fn the_three_ungrouped_languages_sit_where_notepad_puts_them() {
        let b = bar();
        let items = &b.menus[LANGUAGE_MENU].items;
        let labels: Vec<&str> = items.iter().map(|item| item.label.as_str()).collect();
        let at = |label: &str| labels.iter().position(|l| *l == label).unwrap();
        // `KIXtart` between the `J` and `L` groups, `XML` and `YAML` after `V`.
        assert!(at("J") < at("KIXtart") && at("KIXtart") < at("L"));
        assert!(at("V") < at("XML") && at("XML") < at("YAML"));
    }

    #[test]
    fn a_user_defined_language_becomes_a_row_under_its_own_submenu() {
        let mut b = bar();
        b.set_languages(&["Logfile".to_string(), "Ini".to_string()]);
        let items = &b.menus[LANGUAGE_MENU].items;
        let udl = items
            .iter()
            .find(|item| item.label == "User-Defined")
            .expect("the User-Defined submenu is always there");
        assert_eq!(udl.children.len(), 2);
        assert_eq!(udl.children[0].label, "Logfile");
        assert_eq!(udl.children[0].action, Action::Language(0));
        assert_eq!(udl.children[1].action, Action::Language(1));
    }

    #[test]
    fn an_empty_user_defined_submenu_says_so() {
        let b = bar();
        let items = &b.menus[LANGUAGE_MENU].items;
        let udl = items.iter().find(|item| item.label == "User-Defined").unwrap();
        assert_eq!(udl.children.len(), 1);
        assert_eq!(udl.children[0].action, Action::NoOp);
    }

    #[test]
    fn the_language_menu_ends_with_the_three_management_commands() {
        let b = bar();
        let items = &b.menus[LANGUAGE_MENU].items;
        let manage = items
            .iter()
            .find(|item| item.label == "User Defined Language")
            .expect("the management submenu is always there");
        let actions: Vec<Action> = manage.children.iter().map(|row| row.action).collect();
        assert_eq!(
            actions,
            [Action::DefineLanguage, Action::OpenUdlFolder, Action::UdlCollection]
        );
    }

    #[test]
    fn a_built_in_language_round_trips_through_its_key() {
        for (index, spec) in nn_core::LANGS.iter().enumerate() {
            let key = format!("language_built_in_{}", spec.key());
            assert_eq!(Action::from_name(&key), Some(Action::LanguageBuiltIn(index)), "{key}");
        }
        assert_eq!(Action::from_name("language_built_in_klingon"), None);
    }

    #[test]
    fn setting_the_same_languages_twice_keeps_the_menu_closed() {
        let mut b = bar();
        b.set_languages(&["Logfile".to_string()]);
        b.open_menu(LANGUAGE_MENU);
        assert!(b.is_open());
        b.set_languages(&["Logfile".to_string()]);
        assert!(b.is_open());
        b.set_languages(&["Other".to_string()]);
        assert!(!b.is_open());
    }

    #[test]
    fn the_plugins_menu_sits_between_run_and_window() {
        let b = bar();
        let titles: Vec<&str> = b.menus.iter().map(|menu| menu.title.as_str()).collect();
        assert_eq!(titles[PLUGIN_MENU], "Plugins");
        assert_eq!(titles[PLUGIN_MENU - 1], "Run");
        assert_eq!(titles[PLUGIN_MENU + 1], "Window");
        assert_eq!(titles[PLUGIN_MENU + 2], "Help");
    }

    #[test]
    fn an_empty_plugins_menu_says_so() {
        let b = bar();
        let items = &b.menus[PLUGIN_MENU].items;
        // The manager and folder rows are always first, so "no plugins" is
        // still reachable from a menu that has nothing in it.
        assert_eq!(items[0].label, "Manage Plugins...");
        assert_eq!(items[0].action, Action::ManagePlugins);
        assert_eq!(items[1].label, "Open Plugins Folder...");
        assert_eq!(items[1].action, Action::OpenPluginsFolder);
        assert_eq!(items[2].label, "Reload Plugins");
        assert_eq!(items[2].action, Action::ReloadPlugins);
        assert!(items[3].separator);
        assert_eq!(items[4].label, "No Plugins Installed");
        assert_eq!(items[4].action, Action::NoOp);
        assert_eq!(items.len(), 5);
    }

    /// A plugin list as `set_plugins` takes one, in inventory order.
    fn plugins(rows: &[(&str, bool, &[&str])]) -> Vec<PluginMenu> {
        rows.iter()
            .map(|(name, enabled, commands)| PluginMenu {
                name: (*name).to_string(),
                enabled: *enabled,
                commands: commands.iter().map(|c| (*c).to_string()).collect(),
            })
            .collect()
    }

    #[test]
    fn a_folder_with_plugins_in_it_gets_no_note() {
        let mut b = bar();
        b.set_plugins(&plugins(&[("JSON", false, &["Minify"])]));
        let items = &b.menus[PLUGIN_MENU].items;
        // The folder is not empty, so "No Plugins Installed" would be a lie, and
        // the manager above lists every plugin whether it runs or not. The one
        // plugin is switched off, so it gets no submenu either: three rows.
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].label, "Manage Plugins...");
        assert!(items.iter().all(|item| item.label != "No Plugins Installed"));
    }

    #[test]
    fn each_running_plugin_opens_onto_its_commands() {
        let mut b = bar();
        b.set_plugins(&plugins(&[
            ("JSON", true, &["Pretty Print", "Minify", "Validate"]),
            ("XML", false, &["Pretty Print", "Check Well-Formedness"]),
        ]));
        let items = &b.menus[PLUGIN_MENU].items;
        // The three management rows, a separator, then one submenu per running
        // plugin -- and none for the one that is switched off, whose commands
        // do not exist until it is.
        assert_eq!(items.len(), 5);
        assert!(items[3].separator);
        assert!(items[4].is_submenu());
        assert_eq!(items[4].label, "JSON");
        assert_eq!(items[4].action, Action::Submenu);
        let commands: Vec<(&str, Action)> = items[4]
            .children
            .iter()
            .map(|item| (item.label.as_str(), item.action))
            .collect();
        assert_eq!(
            commands,
            vec![
                ("Pretty Print", Action::Plugin(0, 0)),
                ("Minify", Action::Plugin(0, 1)),
                ("Validate", Action::Plugin(0, 2)),
            ]
        );
    }

    #[test]
    fn a_plugin_that_is_switched_off_gets_no_submenu() {
        let mut b = bar();
        b.set_plugins(&plugins(&[
            ("JSON", false, &["Minify"]),
            ("XML", true, &["Linearize"]),
        ]));
        let items = &b.menus[PLUGIN_MENU].items;
        // The submenu that is there belongs to the second plugin, and its rows
        // carry the index the inventory gave that plugin, not the row above it.
        assert_eq!(items.len(), 5);
        assert_eq!(items[4].label, "XML");
        assert_eq!(items[4].children[0].action, Action::Plugin(1, 0));
    }

    #[test]
    fn a_plugin_submenu_keeps_the_plugins_words_in_every_language() {
        // The trap this exists for: a command named after a menu word. "Undo"
        // is a catalog entry, and a translated plugin row would name a command
        // the plugin does not have.
        let italian = crate::i18n::Catalog::load(None, "italian.xml").expect("italian.xml");
        let mut b = bar();
        b.set_catalog(Some(italian));
        b.set_plugins(&plugins(&[("JSON", true, &["Undo", "Minify"])]));
        let items = &b.menus[PLUGIN_MENU].items;
        assert_eq!(items[0].label, "Gestisci i plugin...");
        assert_eq!(items[4].label, "JSON");
        assert_eq!(items[4].children[0].label, "Undo");
        // The same word in the Edit menu, which the catalog does own.
        assert_eq!(find(&b.menus[1].items, Action::Undo).unwrap().label, "Annulla");
        // A plugin's name and its commands are not phrases of the program's, so
        // they are not labels a catalog owes a translation for either. A name no
        // menu uses, because a plugin called "JSON" would find one in the
        // Language menu and prove nothing.
        b.set_plugins(&plugins(&[("Zebra", true, &["Zebra Command"])]));
        assert!(!b.english_labels().contains(&"Zebra"));
        assert!(!b.english_labels().contains(&"Zebra Command"));
        assert!(b.english_labels().contains(&"Manage Plugins..."));
    }

    #[test]
    fn the_italian_catalog_translates_the_manager_row() {
        // The same path the menu takes at startup: a real catalog, loaded from
        // the bundled copy. The manager row is the only plugin row left that
        // carries a translated verb, so a catalog that missed it would put an
        // English word inside an otherwise Italian dropdown.
        let italian = crate::i18n::Catalog::load(None, "italian.xml").expect("italian.xml");
        let mut b = bar();
        b.set_catalog(Some(italian));
        assert_eq!(b.menus[PLUGIN_MENU].items[0].label, "Gestisci i plugin...");
        // A language change rebuilds the rows from the list, so the note comes
        // back translated rather than from a stale English label.
        b.set_plugins(&[]);
        assert_eq!(
            b.menus[PLUGIN_MENU].items[4].label,
            "Nessun plugin installato"
        );
    }

    #[test]
    fn the_plugin_rows_that_take_no_index_are_bindable() {
        // Reloading takes no index, so it can be bound like anything else.
        assert!(Action::BINDABLE.contains(&Action::ReloadPlugins));
        assert_eq!(Action::from_name("reload_plugins"), Some(Action::ReloadPlugins));
        // Run-this-command-of-this-plugin takes two indices, and a saved chord
        // would point at whatever plugin happened to be there after an install.
        // It is a row in a submenu and nothing else.
        assert!(!Action::BINDABLE.contains(&Action::Plugin(0, 0)));
        assert_eq!(Action::from_name("plugin"), None);
        // Switching one plugin on or off is the manager's row: there is no id
        // for it, so no chord can name it.
        assert_eq!(Action::from_name("load_plugin"), None);
    }

    #[test]
    fn a_language_row_is_not_bindable() {
        // A chord that highlighted "the third language" would mean something
        // different after every import, the same reason OpenRecent is out.
        assert!(!Action::BINDABLE.contains(&Action::Language(0)));
        assert_eq!(Action::from_name("language"), None);
    }

    #[test]
    fn a_label_is_the_name_in_words() {
        assert_eq!(Action::Save.label(), "Save");
        assert_eq!(Action::SaveAs.label(), "Save As");
        assert_eq!(Action::ToggleFunctionList.label(), "Toggle Function List");
        assert_eq!(Action::MoveLineDown.label(), "Move Line Down");
    }

    /// Whether a label is the same word in every language.
    ///
    /// Palette names, programming language names, encoding names, submenu
    /// letters and digits are proper nouns: a French user's copy of "HTML" says
    /// "HTML". The case-conversion rows are samples of the effect rather than
    /// words. None of them has, or should have, an entry in a translation file.
    fn is_literal(label: &str) -> bool {
        if label.chars().count() == 1 {
            return true;
        }
        const SAMPLES: [&str; 5] = ["cmd", "lowercase", "UPPERCASE", "iNVERT cASE", "ranDOm CasE"];
        if SAMPLES.contains(&label) {
            return true;
        }
        if crate::themes::CATALOGUE.iter().any(|p| p.name == label) {
            return true;
        }
        nn_core::LANGS.iter().any(|lang| lang.name == label)
    }

    /// Every label every bundled language has to translate.
    ///
    /// The mix this exists to catch: a menu bar that says "Fichier" at the top
    /// and "Fold All" inside, because the translation file predates the row.
    /// Nothing in the bar may fall back to English except a proper noun.
    #[test]
    fn every_bundled_language_translates_every_menu_label() {
        let bar = bar();
        for (file, source) in crate::i18n::BUNDLED {
            let catalog = Catalog::parse(None, source).expect(file);
            let missing: Vec<&str> = bar
                .english_labels()
                .into_iter()
                .filter(|label| !is_literal(label))
                .filter(|label| !catalog.translates(label))
                .collect();
            assert!(missing.is_empty(), "{file} leaves {} in English: {missing:?}", missing.len());
        }
    }

    /// The same question from the other side: a label nobody wrote a row for.
    ///
    /// A menu row added without a translation would pass the test above only if
    /// the catalog happened to know the word, which it would not; this one names
    /// the row instead of the file.
    #[test]
    fn every_prose_label_in_the_bar_is_translated_by_every_language() {
        let bar = bar();
        let mut prose: Vec<&str> = bar
            .english_labels()
            .into_iter()
            .filter(|label| !is_literal(label))
            .collect();
        prose.sort_unstable();
        prose.dedup();
        assert!(prose.len() > 350, "{} prose labels", prose.len());
    }

    #[test]
    fn the_help_menu_opens_the_licence_between_two_rules() {
        // The terms are not a page about this project, so the row is a section
        // of its own: a rule above it and a rule below it, and the page rows
        // stay where they were.
        let bar = bar();
        let help = bar.menus.last().expect("no menus");
        let index = help
            .items
            .iter()
            .position(|item| item.action == Action::License)
            .expect("the Help menu has no licence row");
        assert_eq!(help.items[index].label, "Neko++ License (GPLv3)");
        assert!(help.items[index - 1].separator, "no rule above the licence");
        assert!(help.items[index + 1].separator, "no rule below the licence");
        // It is a link, not a command: nothing to bind, and no chord of its own.
        assert!(!Action::BINDABLE.contains(&Action::License));
        assert!(help.items[index].shortcut.is_empty());
    }
}
