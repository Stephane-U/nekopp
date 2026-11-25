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

//! Right-click context menus.
//!
//! One widget serves both menus -- the editor's and the tab strip's -- because
//! they differ only in the list they are handed. It is drawn in the overlay
//! pass, after the text, for the same reason the menu dropdown is: a menu has to
//! cover what it sits on.
//!
//! Every row is one line high, separators included, with the rule drawn through
//! the middle of the row. That keeps a single shaped text buffer aligned with
//! the hit-test arithmetic; variable-height separators would mean a second
//! layout pass for the sake of a horizontal line.
//!
//! Disabled rows are shaped into a second buffer rather than coloured per span.
//! Two buffers with identical metrics line up exactly, and `set_text` takes one
//! attribute set for the whole buffer, so this is the cheap way to get two
//! colours in one column.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use glyphon::{TextArea, TextBounds};

use crate::complete::truncate;
use crate::layout::Split;
use crate::menu::Action;
use crate::tabbar::TabColour;
use crate::quad::QuadRenderer;
use crate::theme::Theme;

/// Horizontal padding inside the frame, in pixels.
const PADDING_X: f32 = 12.0;
/// Vertical padding above the first row and below the last.
const PADDING_Y: f32 = 4.0;
/// The narrowest and widest a menu gets.
const MIN_WIDTH: f32 = 150.0;
const MAX_WIDTH: f32 = 460.0;

/// One line of a context menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// A command. `enabled` false draws it dimmed and refuses the click.
    Item {
        /// What the row says.
        label: String,
        /// What picking it runs.
        action: Action,
        /// Whether it can be picked at all.
        enabled: bool,
    },
    /// A rule between two groups.
    Separator,
}

impl Entry {
    /// A command that can be picked.
    pub fn item(label: &str, action: Action, enabled: bool) -> Self {
        Self::Item {
            label: label.to_string(),
            action,
            enabled,
        }
    }

    /// An item whose label is built at the call site.
    ///
    /// Separate from `item` rather than taking `impl Into<String>`: the two
    /// callers differ in whether the label is a constant, and a signature that
    /// accepted both would hide which one a given row is.
    pub fn labelled(label: String, action: Action, enabled: bool) -> Self {
        Self::Item {
            label,
            action,
            enabled,
        }
    }

    /// A rule between two groups.
    pub fn separator() -> Self {
        Self::Separator
    }

    /// The label, or an empty string for a separator.
    fn text(&self) -> &str {
        match self {
            Self::Item { label, .. } => label,
            Self::Separator => "",
        }
    }

    /// Whether this entry is a command the user can pick.
    fn is_enabled(&self) -> bool {
        matches!(self, Self::Item { enabled: true, .. })
    }
}

/// The name a layout file uses for a rule between two groups.
pub const SEPARATOR: &str = "separator";

/// Reads a `contextMenu.xml` into the order it asks for.
///
/// Notepad++'s file lists `<Item id="..."/>` rows by numeric command id. This
/// editor has no such numbers -- they are Notepad++'s, and inventing a parallel
/// set would be a table to keep in step for no gain -- so the file names the
/// commands by the same keys the shortcut file uses. A file written here can be
/// read here; a file from Notepad++ is not read, and the menu keeps its built-in
/// order rather than showing an empty popup.
///
/// Returns `None` when the file names nothing, which is the signal to leave the
/// built-in order alone.
pub fn parse_layout(xml: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for element in nn_core::xml::elements(xml, "Item") {
        let Some(name) = nn_core::xml::attribute(&element, "name") else {
            continue;
        };
        out.push(name.trim().to_string());
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Writes `entries` as a `contextMenu.xml` this editor can read back.
///
/// The file is generated from the menu that is showing, so the first edit starts
/// from what the user already sees rather than from an empty file they would
/// have to fill in from memory.
pub fn render_layout(entries: &[Entry]) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n\
         <!-- The rows of the text area's right-click menu, in order.\n     \
         A name is a command key, as in shortcuts.xml; `separator` is a rule.\n     \
         A command that is not listed is not shown. -->\n\
         <NotepadPlus>\n    <ContextMenu>\n",
    );
    for entry in entries {
        match entry {
            Entry::Item { action, .. } => {
                out.push_str(&format!("        <Item name=\"{}\"/>\n", action.name()));
            }
            Entry::Separator => {
                out.push_str(&format!("        <Item name=\"{SEPARATOR}\"/>\n"));
            }
        }
    }
    out.push_str("    </ContextMenu>\n</NotepadPlus>\n");
    out
}

/// Reorders and filters `entries` to match `layout`.
///
/// A name in the layout that no entry matches is skipped rather than shown as a
/// dead row: the layout is a list of commands, and a command this build does not
/// have is not one of them. A layout that matches nothing at all falls back to
/// the built-in order, because an empty right-click menu is a broken one.
pub fn apply_layout(entries: Vec<Entry>, layout: &[String]) -> Vec<Entry> {
    let mut out = Vec::with_capacity(layout.len());
    for name in layout {
        if name == SEPARATOR {
            // A rule at either end, or two in a row, is a drawing artefact
            // rather than a grouping; drop it.
            if !out.is_empty() && !matches!(out.last(), Some(Entry::Separator)) {
                out.push(Entry::Separator);
            }
            continue;
        }
        if let Some(entry) = entries
            .iter()
            .find(|entry| matches!(entry, Entry::Item { action, .. } if action.name() == name))
        {
            out.push(entry.clone());
        }
    }
    while matches!(out.last(), Some(Entry::Separator)) {
        out.pop();
    }
    if out.iter().all(|entry| matches!(entry, Entry::Separator)) {
        return entries;
    }
    out
}

/// The commands offered by a right-click in the text area.
///
/// Takes plain state rather than the editor so the list can be tested without
/// building one, and so the bench draws the same menu the application does
/// instead of a copy of it that can drift.
pub fn editor_entries(can_undo: bool, can_redo: bool, selected: bool, empty: bool) -> Vec<Entry> {
    vec![
        Entry::item("Undo", Action::Undo, can_undo),
        Entry::item("Redo", Action::Redo, can_redo),
        Entry::separator(),
        Entry::item("Cut", Action::Cut, selected),
        Entry::item("Copy", Action::Copy, selected),
        // Always offered: whether the clipboard holds text is a question for the
        // clipboard, and asking it on every right-click to grey one row is not
        // worth the round trip. Pasting nothing does nothing.
        Entry::item("Paste", Action::Paste, true),
        Entry::item("Delete", Action::Delete, selected),
        Entry::separator(),
        Entry::item("Select All", Action::SelectAll, !empty),
        Entry::separator(),
        Entry::item("Add Caret Above", Action::AddCaretAbove, true),
        Entry::item("Add Caret Below", Action::AddCaretBelow, true),
    ]
}

/// The commands offered by a right-click on the bar between two views.
///
/// Notepad++ has one: the splitter's own menu, with a single row that turns the
/// split a quarter turn. The wording names what the click will produce — the
/// other orientation — because a row reading "Vertical" beside a vertical split
/// would read as a statement about the current layout rather than a command.
///
/// `None` when the window is not split: there is no bar to click, and a caller
/// that asked anyway gets nothing to draw rather than a menu about nothing.
pub fn divider_entries(split: Split) -> Vec<Entry> {
    match split {
        Split::None => Vec::new(),
        Split::Vertical => vec![Entry::item(
            "Rotate to Horizontal",
            Action::RotateSplit,
            true,
        )],
        Split::Horizontal => vec![Entry::item("Rotate to Vertical", Action::RotateSplit, true)],
    }
}

/// The commands offered by a right-click on a tab.
pub fn tab_entries(
    tabs: usize,
    active: usize,
    read_only: bool,
    pinned: bool,
    colour: TabColour,
) -> Vec<Entry> {
    vec![
        // A pinned tab cannot be closed, so the row says so rather than
        // offering a click that would be refused.
        Entry::item("Close", Action::CloseTab, !pinned),
        Entry::item("Close Others", Action::CloseOthers, tabs > 1),
        Entry::item("Close to the Left", Action::CloseToLeft, active > 0),
        Entry::item("Close to the Right", Action::CloseToRight, active + 1 < tabs),
        Entry::separator(),
        // The label says what the click will do, so a pinned tab offers Unpin.
        Entry::item(
            if pinned { "Unpin Tab" } else { "Pin Tab" },
            Action::PinTab,
            true,
        ),
        // One row that cycles rather than six that each set a colour: the menu
        // is already long, and a submenu is a second widget this does not have.
        Entry::labelled(
            format!("Tab Colour: {}", colour.label()),
            Action::TabColour,
            true,
        ),
        Entry::separator(),
        Entry::item("Save", Action::Save, true),
        Entry::item("Save As...", Action::SaveAs, true),
        Entry::item("Reload from Disk", Action::Reload, true),
        Entry::separator(),
        Entry::item("Copy Path", Action::CopyPath, true),
        Entry::item("Open Containing Folder", Action::OpenContainingFolder, true),
        Entry::separator(),
        // Always live. With one view these rows are how a user makes the second
        // one -- Notepad++ works the same way -- so greying them out until the
        // window is already split leaves the command reachable only by knowing
        // the keyboard chord, which is not a way to find a menu row.
        Entry::item("Move to Other View", Action::MoveToOtherView, true),
        Entry::item("Clone to Other View", Action::CloneToOtherView, true),
        Entry::separator(),
        // The label says what the click will do, not what the state is: a row
        // reading "Read-Only" on a read-only buffer would be a statement of fact
        // where the user is looking for an action.
        Entry::item(
            if read_only { "Make Writable" } else { "Read-Only" },
            Action::ReadOnly,
            true,
        ),
    ]
}

/// A floating list of commands, opened at the pointer.
pub struct ContextMenu {
    entries: Vec<Entry>,
    /// Where the pointer was, before the menu is nudged back inside the window.
    anchor: (f32, f32),
    /// Window size, so a menu opened near an edge stays on screen.
    viewport: (f32, f32),
    open: bool,
    /// The row under the pointer, if any.
    hovered: Option<usize>,
    /// Labels of the rows that can be picked.
    body: Buffer,
    /// Labels of the rows that cannot.
    dim: Buffer,
    /// What the buffers were shaped for: (row count, columns).
    shaped_key: (usize, usize),
    family: Family<'static>,
    /// The metrics the widget was last built with, so a density change can
    /// be told from the size already in hand.
    metrics: Metrics,
    char_width: f32,
}

impl ContextMenu {
    /// Builds a closed menu.
    pub fn new(metrics: Metrics, family: Family<'static>, char_width: f32) -> Self {
        Self {
            entries: Vec::new(),
            anchor: (0.0, 0.0),
            viewport: (0.0, 0.0),
            open: false,
            hovered: None,
            body: Buffer::new_empty(metrics),
            dim: Buffer::new_empty(metrics),
            shaped_key: (usize::MAX, usize::MAX),
            family,
            metrics,
            char_width,
        }
    }

    /// Rebuilds the menu at a new display density.
    pub fn set_metrics(&mut self, metrics: Metrics) {
        if metrics == self.metrics {
            return;
        }
        // Lengths counted in characters -- a row's column, the box around a
        // label, the width a tab wants -- are measured with `char_width`, so the
        // width grows with the text instead of staying at the size the window
        // opened with.
        self.char_width *= metrics.line_height / self.metrics.line_height;
        self.metrics = metrics;
        self.body.set_metrics(metrics);
        self.dim.set_metrics(metrics);
    }

    /// Records the window size, so a menu opened near an edge is nudged inside.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.viewport = (width, height);
    }

    /// Opens the menu with its top-left corner at `(x, y)`. An empty list
    /// closes it instead, so a caller that builds no entries does not have to
    /// check first.
    pub fn show(&mut self, x: f32, y: f32, entries: Vec<Entry>) {
        self.open = !entries.is_empty();
        self.entries = entries;
        self.anchor = (x, y);
        self.hovered = None;
        self.shaped_key = (usize::MAX, usize::MAX);
    }

    /// Hides the menu and drops the list.
    pub fn close(&mut self) {
        self.open = false;
        self.entries.clear();
        self.hovered = None;
        self.shaped_key = (usize::MAX, usize::MAX);
    }

    /// Whether the menu is showing.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The entries, for tests and for the caller that wants to know what it
    /// just opened.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The width the menu occupies.
    pub fn width(&self) -> f32 {
        let longest = self
            .entries
            .iter()
            .map(|entry| entry.text().chars().count())
            .max()
            .unwrap_or(0);
        let scale = crate::renderer::scale_of(self.metrics);
        (longest as f32 * self.char_width + PADDING_X * 2.0).clamp(MIN_WIDTH * scale, MAX_WIDTH * scale)
    }

    /// The height the menu occupies.
    pub fn height(&self, line_height: f32) -> f32 {
        if !self.open {
            return 0.0;
        }
        self.entries.len() as f32 * line_height + PADDING_Y * 2.0
    }

    /// The left edge actually drawn at, nudged left when the pointer was too
    /// close to the right edge.
    pub fn left(&self) -> f32 {
        let left = self.anchor.0;
        if self.viewport.0 <= 0.0 {
            return left;
        }
        left.min(self.viewport.0 - self.width()).max(0.0)
    }

    /// The top edge actually drawn at, nudged up when the pointer was too close
    /// to the bottom edge.
    pub fn top(&self, line_height: f32) -> f32 {
        let top = self.anchor.1;
        if self.viewport.1 <= 0.0 {
            return top;
        }
        top.min(self.viewport.1 - self.height(line_height)).max(0.0)
    }

    /// The row index under `(x, y)` in window pixels, separators included.
    pub fn row_at(&self, x: f32, y: f32, line_height: f32) -> Option<usize> {
        if !self.open {
            return None;
        }
        let left = self.left();
        let top = self.top(line_height);
        if x < left || x >= left + self.width() {
            return None;
        }
        if y < top + PADDING_Y || y >= top + self.height(line_height) - PADDING_Y {
            return None;
        }
        let row = ((y - top - PADDING_Y) / line_height) as usize;
        (row < self.entries.len()).then_some(row)
    }

    /// The command under `(x, y)`, if the row there is one that can be picked.
    pub fn action_at(&self, x: f32, y: f32, line_height: f32) -> Option<Action> {
        let row = self.row_at(x, y, line_height)?;
        match &self.entries[row] {
            Entry::Item {
                action,
                enabled: true,
                ..
            } => Some(*action),
            _ => None,
        }
    }

    /// Highlights the row under `(x, y)`, if any.
    pub fn hover(&mut self, x: f32, y: f32, line_height: f32) {
        self.hovered = self.row_at(x, y, line_height);
    }

    /// Rebuilds the shaped rows if anything they depend on changed.
    pub fn prepare(&mut self, font_system: &mut FontSystem) {
        if !self.open {
            return;
        }
        let columns = ((self.width() - PADDING_X * 2.0) / self.char_width).max(4.0) as usize;
        let key = (self.entries.len(), columns);
        if key == self.shaped_key {
            return;
        }
        self.shaped_key = key;

        let mut bright = String::new();
        let mut dim = String::new();
        for (index, entry) in self.entries.iter().enumerate() {
            if index > 0 {
                bright.push('\n');
                dim.push('\n');
            }
            let text = truncate(entry.text(), columns);
            match entry {
                Entry::Item { enabled: true, .. } => bright.push_str(&text),
                Entry::Item { enabled: false, .. } => dim.push_str(&text),
                Entry::Separator => {}
            }
        }

        let attrs = Attrs::new().family(self.family);
        self.body
            .set_text(&bright, &attrs, Shaping::Basic, None);
        self.body.shape_until_scroll(font_system, false);
        self.dim.set_text(&dim, &attrs, Shaping::Basic, None);
        self.dim.shape_until_scroll(font_system, false);
    }

    /// Queues the frame, the separator rules and the hovered row.
    pub fn push_quads(&self, quads: &mut QuadRenderer, line_height: f32, theme: &Theme) {
        if !self.open {
            return;
        }
        let left = self.left();
        let top = self.top(line_height);
        let width = self.width();
        let height = self.height(line_height);

        quads.push(
            left,
            top,
            width,
            height,
            theme.menu_background.to_linear(),
        );
        let border = theme.menu_border.to_linear();
        quads.push(left, top, width, 1.0, border);
        quads.push(left, top + height - 1.0, width, 1.0, border);
        quads.push(left, top, 1.0, height, border);
        quads.push(left + width - 1.0, top, 1.0, height, border);

        for (index, entry) in self.entries.iter().enumerate() {
            if !matches!(entry, Entry::Separator) {
                continue;
            }
            let y = top + PADDING_Y + index as f32 * line_height + line_height / 2.0;
            quads.push(left + PADDING_X / 2.0, y, width - PADDING_X, 1.0, border);
        }

        if let Some(row) = self.hovered {
            if self.entries.get(row).is_some_and(Entry::is_enabled) {
                quads.push(
                    left + 1.0,
                    top + PADDING_Y + row as f32 * line_height,
                    width - 2.0,
                    line_height,
                    theme.menu_hover.to_linear(),
                );
            }
        }
    }

    /// The text areas of the rows, one per colour.
    pub fn text_areas<'a>(&'a self, line_height: f32, theme: &Theme) -> Vec<TextArea<'a>> {
        if !self.open {
            return Vec::new();
        }
        let left = self.left();
        let top = self.top(line_height);
        let bounds = TextBounds {
            left: left as i32,
            top: top as i32,
            right: (left + self.width()) as i32,
            bottom: (top + self.height(line_height)) as i32,
        };
        let mut areas = Vec::with_capacity(2);
        for (buffer, color) in [
            (&self.body, theme.menu_text),
            (&self.dim, theme.menu_text_disabled),
        ] {
            areas.push(TextArea {
                buffer,
                left: left + PADDING_X,
                top: top + PADDING_Y,
                scale: 1.0,
                bounds,
                default_color: color.to_cosmic(),
                custom_glyphs: &[],
            });
        }
        areas
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    const LINE: f32 = 20.0;

    fn menu() -> ContextMenu {
        let mut menu = ContextMenu::new(
            Metrics::new(15.0, LINE),
            Family::Monospace,
            8.0,
        );
        menu.set_viewport(800.0, 600.0);
        menu
    }

    fn sample() -> Vec<Entry> {
        vec![
            Entry::item("Undo", Action::Undo, true),
            Entry::item("Redo", Action::Redo, false),
            Entry::separator(),
            Entry::item("Select All", Action::SelectAll, true),
        ]
    }

    #[test]
    fn a_closed_menu_has_no_size() {
        let menu = menu();
        assert!(!menu.is_open());
        assert_eq!(menu.height(LINE), 0.0);
        assert_eq!(menu.action_at(10.0, 10.0, LINE), None);
    }

    #[test]
    fn showing_an_empty_list_leaves_it_closed() {
        let mut menu = menu();
        menu.show(10.0, 10.0, Vec::new());
        assert!(!menu.is_open());
    }

    #[test]
    fn every_row_is_one_line_high() {
        let mut menu = menu();
        menu.show(10.0, 10.0, sample());
        assert_eq!(menu.height(LINE), 4.0 * LINE + PADDING_Y * 2.0);
    }

    #[test]
    fn the_width_follows_the_longest_label() {
        let mut menu = menu();
        menu.show(
            10.0,
            10.0,
            vec![Entry::item("Open Containing Folder", Action::OpenContainingFolder, true)],
        );
        assert_eq!(menu.width(), 22.0 * 8.0 + PADDING_X * 2.0);
    }

    #[test]
    fn a_short_label_still_gets_the_minimum_width() {
        let mut menu = menu();
        menu.show(10.0, 10.0, vec![Entry::item("Cut", Action::Cut, true)]);
        assert_eq!(menu.width(), MIN_WIDTH);
    }

    #[test]
    fn a_very_long_label_is_capped() {
        let mut menu = menu();
        let long = "x".repeat(200);
        menu.show(10.0, 10.0, vec![Entry::item(&long, Action::Cut, true)]);
        assert_eq!(menu.width(), MAX_WIDTH);
    }

    #[test]
    fn a_row_maps_back_to_its_command() {
        let mut menu = menu();
        menu.show(10.0, 10.0, sample());
        let top = menu.top(LINE) + PADDING_Y;
        assert_eq!(menu.action_at(20.0, top + 5.0, LINE), Some(Action::Undo));
        assert_eq!(
            menu.action_at(20.0, top + 3.0 * LINE + 5.0, LINE),
            Some(Action::SelectAll)
        );
    }

    #[test]
    fn a_disabled_row_offers_nothing() {
        let mut menu = menu();
        menu.show(10.0, 10.0, sample());
        let top = menu.top(LINE) + PADDING_Y;
        assert_eq!(menu.action_at(20.0, top + LINE + 5.0, LINE), None);
    }

    #[test]
    fn a_separator_offers_nothing() {
        let mut menu = menu();
        menu.show(10.0, 10.0, sample());
        let top = menu.top(LINE) + PADDING_Y;
        assert_eq!(menu.action_at(20.0, top + 2.0 * LINE + 5.0, LINE), None);
    }

    #[test]
    fn a_click_outside_the_frame_misses() {
        let mut menu = menu();
        menu.show(10.0, 10.0, sample());
        assert_eq!(menu.action_at(5.0, 20.0, LINE), None);
        assert_eq!(menu.action_at(400.0, 20.0, LINE), None);
        assert_eq!(menu.action_at(20.0, 5.0, LINE), None);
        assert_eq!(menu.action_at(20.0, 400.0, LINE), None);
    }

    #[test]
    fn a_click_in_the_padding_misses() {
        let mut menu = menu();
        menu.show(10.0, 10.0, sample());
        let top = menu.top(LINE);
        assert_eq!(menu.action_at(20.0, top + 1.0, LINE), None);
        assert_eq!(
            menu.action_at(20.0, top + menu.height(LINE) - 1.0, LINE),
            None
        );
    }

    #[test]
    fn a_menu_near_the_right_edge_is_nudged_left() {
        let mut menu = menu();
        menu.show(790.0, 10.0, sample());
        assert_eq!(menu.left(), 800.0 - menu.width());
    }

    #[test]
    fn a_menu_near_the_bottom_edge_is_nudged_up() {
        let mut menu = menu();
        menu.show(10.0, 590.0, sample());
        assert_eq!(menu.top(LINE), 600.0 - menu.height(LINE));
    }

    #[test]
    fn a_menu_that_fits_is_not_moved() {
        let mut menu = menu();
        menu.show(100.0, 100.0, sample());
        assert_eq!(menu.left(), 100.0);
        assert_eq!(menu.top(LINE), 100.0);
    }

    #[test]
    fn a_menu_wider_than_the_window_sticks_to_the_left_edge() {
        let mut menu = menu();
        menu.set_viewport(100.0, 600.0);
        menu.show(50.0, 10.0, sample());
        assert_eq!(menu.left(), 0.0);
    }

    #[test]
    fn hovering_marks_the_row_and_ignores_the_rest() {
        let mut menu = menu();
        menu.show(10.0, 10.0, sample());
        let top = menu.top(LINE) + PADDING_Y;
        menu.hover(20.0, top + 5.0, LINE);
        assert_eq!(menu.hovered, Some(0));
        menu.hover(20.0, top + LINE + 5.0, LINE);
        assert_eq!(menu.hovered, Some(1));
        menu.hover(5.0, top + 5.0, LINE);
        assert_eq!(menu.hovered, None);
    }

    #[test]
    fn closing_drops_the_entries() {
        let mut menu = menu();
        menu.show(10.0, 10.0, sample());
        menu.close();
        assert!(!menu.is_open());
        assert!(menu.entries().is_empty());
        assert_eq!(menu.hovered, None);
    }

    /// Whether the row labelled `label` can be picked, or `None` if there is no
    /// such row.
    fn enabled(entries: &[Entry], label: &str) -> Option<bool> {
        entries.iter().find_map(|entry| match entry {
            Entry::Item {
                label: text,
                enabled,
                ..
            } if text == label => Some(*enabled),
            _ => None,
        })
    }

    #[test]
    fn the_editor_menu_greys_what_cannot_run() {
        let entries = editor_entries(false, false, false, true);
        assert_eq!(enabled(&entries, "Undo"), Some(false));
        assert_eq!(enabled(&entries, "Redo"), Some(false));
        assert_eq!(enabled(&entries, "Cut"), Some(false));
        assert_eq!(enabled(&entries, "Copy"), Some(false));
        assert_eq!(enabled(&entries, "Delete"), Some(false));
        assert_eq!(enabled(&entries, "Select All"), Some(false));
        // Pasting into an empty buffer is the one thing that always applies.
        assert_eq!(enabled(&entries, "Paste"), Some(true));
    }

    #[test]
    fn the_editor_menu_offers_what_can_run() {
        let entries = editor_entries(true, true, true, false);
        assert_eq!(enabled(&entries, "Undo"), Some(true));
        assert_eq!(enabled(&entries, "Redo"), Some(true));
        assert_eq!(enabled(&entries, "Cut"), Some(true));
        assert_eq!(enabled(&entries, "Select All"), Some(true));
    }

    #[test]
    fn the_editor_menu_offers_the_caret_commands() {
        let entries = editor_entries(false, false, false, false);
        assert_eq!(enabled(&entries, "Add Caret Above"), Some(true));
        assert_eq!(enabled(&entries, "Add Caret Below"), Some(true));
    }

    #[test]
    fn the_tab_menu_greys_close_others_with_one_tab() {
        let entries = tab_entries(1, 0, false, false, TabColour::None);
        assert_eq!(enabled(&entries, "Close Others"), Some(false));
        assert_eq!(enabled(&entries, "Close"), Some(true));
    }

    #[test]
    fn the_tab_menu_greys_close_to_the_left_on_the_first_tab() {
        let entries = tab_entries(3, 0, false, false, TabColour::None);
        assert_eq!(enabled(&entries, "Close to the Left"), Some(false));
        assert_eq!(enabled(&entries, "Close to the Right"), Some(true));
    }

    #[test]
    fn the_tab_menu_greys_close_to_the_right_on_the_last_tab() {
        let entries = tab_entries(3, 2, false, false, TabColour::None);
        assert_eq!(enabled(&entries, "Close to the Left"), Some(true));
        assert_eq!(enabled(&entries, "Close to the Right"), Some(false));
    }

    #[test]
    fn the_tab_menu_offers_the_split_commands_without_a_split() {
        // These rows make the second view, so they are live on a window that
        // has none. A user with one view who right-clicks a tab is exactly the
        // user looking for this command.
        let entries = tab_entries(2, 0, false, false, TabColour::None);
        assert_eq!(enabled(&entries, "Move to Other View"), Some(true));
        assert_eq!(enabled(&entries, "Clone to Other View"), Some(true));
    }

    #[test]
    fn the_divider_menu_offers_the_other_orientation() {
        // The row names what the click produces, not what the window shows: a
        // row reading "Rotate to Vertical" beside a vertical split would say
        // the opposite of what it does.
        let vertical = divider_entries(Split::Vertical);
        assert_eq!(vertical.len(), 1);
        assert_eq!(vertical[0].text(), "Rotate to Horizontal");
        assert!(matches!(
            vertical[0],
            Entry::Item {
                action: Action::RotateSplit,
                enabled: true,
                ..
            }
        ));

        let horizontal = divider_entries(Split::Horizontal);
        assert_eq!(horizontal.len(), 1);
        assert_eq!(horizontal[0].text(), "Rotate to Vertical");
        assert!(matches!(
            horizontal[0],
            Entry::Item {
                action: Action::RotateSplit,
                enabled: true,
                ..
            }
        ));
    }

    #[test]
    fn an_unsplit_window_has_no_divider_menu() {
        // There is no bar to right-click, so there is nothing to offer. An empty
        // list is what keeps the caller from drawing a menu at the window edge
        // where no divider exists.
        assert!(divider_entries(Split::None).is_empty());
    }

    #[test]
    fn the_read_only_row_says_what_the_click_will_do() {
        let writable = tab_entries(1, 0, false, false, TabColour::None);
        assert_eq!(enabled(&writable, "Read-Only"), Some(true));
        assert_eq!(enabled(&writable, "Make Writable"), None);
        let locked = tab_entries(1, 0, true, false, TabColour::None);
        assert_eq!(enabled(&locked, "Read-Only"), None);
        assert_eq!(enabled(&locked, "Make Writable"), Some(true));
    }

    #[test]
    fn a_pinned_tab_cannot_be_closed_from_the_menu() {
        let entries = tab_entries(2, 0, false, true, TabColour::None);
        assert_eq!(enabled(&entries, "Close"), Some(false));
        // And the row offers the way out instead.
        assert_eq!(enabled(&entries, "Unpin Tab"), Some(true));
        assert_eq!(enabled(&entries, "Pin Tab"), None);
    }

    #[test]
    fn an_unpinned_tab_offers_the_pin() {
        let entries = tab_entries(2, 0, false, false, TabColour::None);
        assert_eq!(enabled(&entries, "Pin Tab"), Some(true));
        assert_eq!(enabled(&entries, "Unpin Tab"), None);
    }

    #[test]
    fn the_colour_row_names_the_colour_the_tab_has() {
        let entries = tab_entries(1, 0, false, false, TabColour::Green);
        assert_eq!(enabled(&entries, "Tab Colour: Green"), Some(true));
        let entries = tab_entries(1, 0, false, false, TabColour::None);
        assert_eq!(enabled(&entries, "Tab Colour: No Colour"), Some(true));
    }

    #[test]
    fn the_theme_has_a_colour_for_a_disabled_row() {
        // Guards the field the dim buffer reads; a theme without it would not
        // compile, but a theme that set it equal to the normal text would make
        // the distinction invisible.
        assert_ne!(Theme::DARK.menu_text_disabled, Theme::DARK.menu_text);
        assert_ne!(Theme::LIGHT.menu_text_disabled, Theme::LIGHT.menu_text);
    }

    #[test]
    fn a_layout_file_round_trips_through_the_menu_it_describes() {
        let entries = editor_entries(true, false, true, false);
        let text = render_layout(&entries);
        let layout = parse_layout(&text).expect("the file names rows");
        let back = apply_layout(entries.clone(), &layout);
        assert_eq!(back, entries);
    }

    #[test]
    fn a_layout_reorders_and_drops_rows() {
        let entries = editor_entries(true, true, true, false);
        let layout = vec!["select_all".to_string(), SEPARATOR.to_string(), "undo".to_string()];
        let out = apply_layout(entries, &layout);
        let labels: Vec<&str> = out
            .iter()
            .filter_map(|entry| match entry {
                Entry::Item { label, .. } => Some(label.as_str()),
                Entry::Separator => None,
            })
            .collect();
        assert_eq!(labels, ["Select All", "Undo"]);
        assert!(matches!(out[1], Entry::Separator));
    }

    #[test]
    fn a_layout_that_names_nothing_known_keeps_the_built_in_menu() {
        // An empty right-click menu is a broken one, so a layout that matches
        // nothing falls back rather than showing nothing.
        let entries = editor_entries(true, true, true, false);
        let out = apply_layout(entries.clone(), &["no_such_command".to_string()]);
        assert_eq!(out, entries);
    }

    #[test]
    fn a_layout_drops_rules_at_the_ends_and_in_a_row() {
        let entries = editor_entries(true, true, true, false);
        let layout = vec![
            SEPARATOR.to_string(),
            "undo".to_string(),
            SEPARATOR.to_string(),
            SEPARATOR.to_string(),
            "redo".to_string(),
            SEPARATOR.to_string(),
        ];
        let out = apply_layout(entries, &layout);
        assert_eq!(out.len(), 3);
        assert!(matches!(out[0], Entry::Item { .. }));
        assert!(matches!(out[1], Entry::Separator));
        assert!(matches!(out[2], Entry::Item { .. }));
    }

    #[test]
    fn a_file_with_no_rows_is_not_a_layout() {
        assert_eq!(parse_layout("<NotepadPlus></NotepadPlus>"), None);
        assert_eq!(parse_layout(""), None);
    }

    #[test]
    fn a_layout_keeps_the_enabled_state_of_each_row() {
        // The layout says which commands to show, not whether they apply: a
        // greyed Undo is still Undo, and the file must not be able to make it
        // clickable.
        let entries = editor_entries(false, false, false, true);
        let layout = vec!["undo".to_string(), "cut".to_string()];
        let out = apply_layout(entries, &layout);
        for entry in &out {
            match entry {
                Entry::Item { enabled, .. } => assert!(!enabled),
                Entry::Separator => panic!("no rule was asked for"),
            }
        }
    }
}
