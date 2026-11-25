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

//! The plugin manager: every installed plugin, and whether it is running.
//!
//! A centred modal list drawn in the overlay pass the menu and the preferences
//! dialog already share. It is a list of switches, not a dialog of its own: one
//! row per `.wasm` file in the folder, a state word beside it, and the keys that
//! flip it.
//!
//! The dialog never touches a module. It reports which row was flipped and the
//! state it should now have; the window switches the plugin on or off, writes
//! the settings file, and hands the new rows back. That is the same split the
//! preferences dialog uses for its settings, and for the same reason: a modal
//! that owns a file handle is a modal that has to be trusted with the file.
//!
//! Every plugin starts switched off. A `.wasm` copied into the folder is
//! *installed*; the editor does not run it until somebody says so here, which is
//! the difference between a folder of plugins and a folder of code the editor
//! executes on sight.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics};
use glyphon::{TextArea, TextBounds};

use crate::i18n::Catalog;
use crate::quad::QuadRenderer;
use crate::text::shaping_for;
use crate::theme::Theme;

/// The narrowest the dialog gets, in pixels.
///
/// The width is measured from the widest row, because a plugin name is not
/// translatable and can be longer than any label the dialog owns.
const WIDTH: f32 = 460.0;
/// Padding inside the frame.
const PADDING: f32 = 14.0;
/// The column the state word starts at, in characters.
///
/// A column rather than a second layout: the font is monospace, so padding the
/// names lines the states up for free.
const STATE_COLUMN: usize = 24;
/// The most rows shown at once, before the list scrolls.
///
/// A folder can hold fifty plugins and a window cannot: past this the list keeps
/// the highlight in view rather than growing off the bottom of the screen.
const MAX_ROWS: usize = 14;
/// The keys, drawn under the list.
const HINT: &str = "Space enables or disables, Esc closes";
/// The title, drawn above it.
const TITLE: &str = "Plugin Manager";
/// The two words a row can end in.
const ENABLED: &str = "Enabled";
const DISABLED: &str = "Disabled";
/// What an empty folder says. The same words the menu uses, so a language that
/// translates one translates both.
const EMPTY: &str = "No Plugins Installed";

/// One row of the manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The name the module gives itself.
    pub name: String,
    /// The file it came from, without its directory.
    pub file: String,
    /// Whether the module is running.
    pub enabled: bool,
}

/// The modal list.
pub struct PluginList {
    open: bool,
    rows: Vec<Row>,
    selected: usize,
    /// The first row drawn, which is what a list longer than [`MAX_ROWS`]
    /// scrolls.
    first: usize,
    /// The localization the dialog's own words come from. `None` draws English.
    catalog: Option<Catalog>,
    title: Buffer,
    title_shaped: String,
    body: Buffer,
    body_shaped: String,
    family: Family<'static>,
    /// The metrics the widget was last built with, so a density change can
    /// be told from the size already in hand.
    metrics: Metrics,
    char_width: f32,
    line_height: f32,
    /// Window size, so the dialog can centre itself and know how many rows fit.
    viewport: (f32, f32),
}

impl PluginList {
    /// Builds a closed manager.
    pub fn new(metrics: Metrics, family: Family<'static>, char_width: f32) -> Self {
        Self {
            open: false,
            rows: Vec::new(),
            selected: 0,
            first: 0,
            catalog: None,
            title: Buffer::new_empty(metrics),
            title_shaped: String::new(),
            body: Buffer::new_empty(metrics),
            body_shaped: String::new(),
            family,
            metrics,
            char_width,
            line_height: metrics.line_height,
            viewport: (0.0, 0.0),
        }
    }

    /// Rebuilds the dialog at a new display density.
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
        self.title.set_metrics(metrics);
        self.body.set_metrics(metrics);
    }

    /// Opens the manager on `rows`, with the first row highlighted.
    pub fn open(&mut self, rows: Vec<Row>) {
        self.open = true;
        self.rows = rows;
        self.selected = 0;
        self.first = 0;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The rows, in the order the folder was scanned.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Replaces the rows without closing, after a flip.
    ///
    /// The highlight is kept where it was: flipping a switch should not move the
    /// list under the pointer, which is how a second Space lands on the wrong
    /// plugin.
    pub fn set_rows(&mut self, rows: Vec<Row>) {
        self.rows = rows;
        self.clamp();
    }

    /// Which row is highlighted.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Moves the highlight by `delta`, clamping at both ends.
    ///
    /// Clamping rather than wrapping: the list has a first and a last row, and
    /// wrapping off the end of a short list is disorienting.
    pub fn move_selection(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() - 1;
        self.selected = (self.selected as isize + delta).clamp(0, last as isize) as usize;
        self.clamp();
    }

    /// Moves the highlight by a screenful, for PageUp and PageDown.
    pub fn page(&mut self, pages: isize) {
        self.move_selection(pages * self.visible() as isize);
    }

    pub fn home(&mut self) {
        self.selected = 0;
        self.clamp();
    }

    pub fn end(&mut self) {
        self.selected = self.rows.len().saturating_sub(1);
        self.clamp();
    }

    /// Highlights `index`, for a click.
    pub fn set_selected(&mut self, index: usize) {
        if index < self.rows.len() {
            self.selected = index;
            self.clamp();
        }
    }

    /// Flips the highlighted row and answers its index and the state it should
    /// now have.
    ///
    /// The row is *not* flipped here. The caller switches the module off or on
    /// first and hands the rows back: a row that says `Enabled` over a module
    /// that refused to load would be a lie the dialog cannot detect.
    pub fn toggle(&self) -> Option<(usize, bool)> {
        let row = self.rows.get(self.selected)?;
        Some((self.selected, !row.enabled))
    }

    /// Records the row's new state, after the caller made it true.
    pub fn set_enabled(&mut self, index: usize, enabled: bool) {
        if let Some(row) = self.rows.get_mut(index) {
            row.enabled = enabled;
        }
    }

    /// Records the window size.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.viewport = (width, height);
        self.clamp();
    }

    /// Sets the localization the dialog's own words come from.
    pub fn set_catalog(&mut self, catalog: Option<Catalog>) {
        self.catalog = catalog;
    }

    /// How many rows are on screen at once.
    ///
    /// Bounded by the window: a manager that drew its whole list off the bottom
    /// edge would be a manager with unreachable switches.
    fn visible(&self) -> usize {
        if self.rows.is_empty() {
            return 0;
        }
        let screen = ((self.viewport.1 - 80.0) / self.line_height) as usize;
        self.rows.len().min(MAX_ROWS).min(screen.max(1))
    }

    /// Keeps the highlight inside the drawn window, and the window inside the
    /// rows.
    fn clamp(&mut self) {
        let visible = self.visible();
        if visible == 0 {
            self.first = 0;
            self.selected = 0;
            return;
        }
        if self.selected < self.first {
            self.first = self.selected;
        }
        if self.selected >= self.first + visible {
            self.first = self.selected + 1 - visible;
        }
        self.first = self.first.min(self.rows.len().saturating_sub(visible));
    }

    /// The dialog's own translation of `text`.
    fn tr<'a>(&'a self, text: &'a str) -> &'a str {
        match &self.catalog {
            Some(catalog) => catalog.tr(text),
            None => text,
        }
    }

    /// The title, drawn on its own so it can use the label colour.
    pub fn title(&self) -> String {
        self.tr(TITLE).to_string()
    }

    /// The hint under the list, which also names the highlighted file.
    pub fn hint(&self) -> String {
        match self.rows.get(self.selected) {
            Some(row) => format!("{}  {}", row.file, self.tr(HINT)),
            None => self.tr(EMPTY).to_string(),
        }
    }

    /// Every label this dialog can draw, for the coverage test.
    pub fn labels() -> Vec<&'static str> {
        vec![TITLE, HINT, ENABLED, DISABLED, EMPTY]
    }

    /// The width the dialog occupies.
    pub fn width(&self) -> f32 {
        let widest = self
            .rows
            .iter()
            .map(|_| STATE_COLUMN + self.tr(ENABLED).chars().count().max(8))
            .chain([
                self.title().chars().count(),
                self.hint().chars().count(),
                STATE_COLUMN,
            ])
            .max()
            .unwrap_or(0);
        (widest as f32 * self.char_width + PADDING * 2.0).max(WIDTH * crate::renderer::scale_of(self.metrics))
    }

    /// The height the dialog occupies: title, gap, rows, gap, hint.
    pub fn height(&self) -> f32 {
        let rows = self.visible().max(1) as f32;
        PADDING * 2.0 + (rows + 4.0) * self.line_height
    }

    /// The top-left corner, centred in the window.
    fn origin(&self) -> (f32, f32) {
        let (width, height) = self.viewport;
        let left = ((width - self.width()) / 2.0).max(0.0);
        let top = ((height - self.height()) / 2.0).max(0.0);
        (left, top)
    }

    /// The row under `(x, y)` in window pixels.
    pub fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        if !self.open {
            return None;
        }
        let (left, top) = self.origin();
        if x < left || x > left + self.width() {
            return None;
        }
        let rows_top = top + PADDING + self.line_height * 2.0;
        if y < rows_top {
            return None;
        }
        let drawn = ((y - rows_top) / self.line_height) as usize;
        if drawn >= self.visible() {
            return None;
        }
        let index = self.first + drawn;
        (index < self.rows.len()).then_some(index)
    }

    /// Shapes the title and the rows if they changed.
    pub fn prepare(&mut self, font_system: &mut FontSystem) {
        if !self.open {
            return;
        }
        self.clamp();

        let title = self.title();
        if self.title_shaped != title {
            self.title.set_text(
                &title,
                &Attrs::new().family(self.family),
                shaping_for(&title),
                None,
            );
            self.title.shape_until_scroll(font_system, false);
            self.title_shaped = title;
        }

        let mut text = String::new();
        let visible = self.visible();
        for (drawn, index) in (self.first..self.first + visible).enumerate() {
            if drawn > 0 {
                text.push('\n');
            }
            let Some(row) = self.rows.get(index) else {
                continue;
            };
            text.push_str(&row.name);
            for _ in row.name.chars().count()..STATE_COLUMN {
                text.push(' ');
            }
            text.push_str(self.tr(if row.enabled { ENABLED } else { DISABLED }));
        }
        if self.rows.is_empty() {
            text.push_str(self.tr(EMPTY));
        }
        // A blank line, then the hint, so the keys are always on screen.
        text.push_str("\n\n");
        text.push_str(&self.hint());
        if self.body_shaped != text {
            self.body.set_text(
                &text,
                &Attrs::new().family(self.family),
                shaping_for(&text),
                None,
            );
            self.body.shape_until_scroll(font_system, false);
            self.body_shaped = text;
        }
    }

    /// Queues the frame and the highlighted row.
    pub fn push_quads(&self, quads: &mut QuadRenderer, theme: &Theme) {
        if !self.open {
            return;
        }
        let (left, top) = self.origin();
        let width = self.width();
        let height = self.height();

        quads.push(left, top, width, height, theme.menu_background.to_linear());
        let border = theme.menu_border.to_linear();
        quads.push(left, top, width, 1.0, border);
        quads.push(left, top + height - 1.0, width, 1.0, border);
        quads.push(left, top, 1.0, height, border);
        quads.push(left + width - 1.0, top, 1.0, height, border);

        if !self.rows.is_empty() {
            let rows_top = top + PADDING + self.line_height * 2.0;
            let drawn = (self.selected.saturating_sub(self.first)) as f32;
            quads.push(
                left + 1.0,
                rows_top + drawn * self.line_height,
                width - 2.0,
                self.line_height,
                theme.menu_hover.to_linear(),
            );
        }
    }

    /// The text areas: the title, the rows and the hint.
    pub fn text_areas<'a>(&'a self, theme: &Theme) -> Vec<TextArea<'a>> {
        if !self.open {
            return Vec::new();
        }
        let (left, top) = self.origin();
        let bounds = TextBounds {
            left: left as i32,
            top: top as i32,
            right: (left + self.width()) as i32,
            bottom: (top + self.height()) as i32,
        };
        vec![
            TextArea {
                buffer: &self.title,
                left: left + PADDING,
                top: top + PADDING,
                scale: 1.0,
                bounds,
                default_color: theme.bar_label.to_cosmic(),
                custom_glyphs: &[],
            },
            TextArea {
                buffer: &self.body,
                left: left + PADDING,
                top: top + PADDING + self.line_height * 2.0,
                scale: 1.0,
                bounds,
                default_color: theme.foreground.to_cosmic(),
                custom_glyphs: &[],
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager() -> PluginList {
        PluginList::new(
            Metrics::new(15.0, 20.0),
            Family::Monospace,
            8.0,
        )
    }

    fn rows() -> Vec<Row> {
        vec![
            Row {
                name: "Align Columns".to_string(),
                file: "npp_align.wasm".to_string(),
                enabled: false,
            },
            Row {
                name: "Half Width".to_string(),
                file: "npp_halfwidth.wasm".to_string(),
                enabled: false,
            },
        ]
    }

    #[test]
    fn every_row_starts_switched_off() {
        let mut list = manager();
        list.open(rows());
        assert!(list.is_open());
        assert_eq!(list.rows().len(), 2);
        assert!(list.rows().iter().all(|row| !row.enabled));
    }

    #[test]
    fn toggling_answers_the_row_and_the_state_it_should_have() {
        let mut list = manager();
        list.open(rows());
        assert_eq!(list.toggle(), Some((0, true)));
        // The dialog does not flip its own row: the module has to load first.
        assert!(!list.rows()[0].enabled);
        list.set_enabled(0, true);
        assert_eq!(list.toggle(), Some((0, false)));
    }

    #[test]
    fn the_highlight_stops_at_both_ends() {
        let mut list = manager();
        list.open(rows());
        list.move_selection(-1);
        assert_eq!(list.selected(), 0);
        list.move_selection(1);
        assert_eq!(list.selected(), 1);
        list.move_selection(1);
        assert_eq!(list.selected(), 1);
        list.home();
        assert_eq!(list.selected(), 0);
        list.end();
        assert_eq!(list.selected(), 1);
    }

    #[test]
    fn a_list_longer_than_the_window_scrolls_the_highlight_into_view() {
        let mut list = manager();
        let many: Vec<Row> = (0..40)
            .map(|index| Row {
                name: format!("Plugin {index}"),
                file: format!("p{index}.wasm"),
                enabled: false,
            })
            .collect();
        list.open(many);
        list.set_viewport(1200.0, 800.0);
        list.end();
        assert_eq!(list.selected(), 39);
        // The last row is on screen, and the window did not run past the list.
        let visible = list.visible();
        assert_eq!(list.first, 40 - visible);
    }

    #[test]
    fn a_click_lands_on_the_row_it_is_over() {
        let mut list = manager();
        list.open(rows());
        list.set_viewport(1200.0, 800.0);
        let (left, top) = list.origin();
        let rows_top = top + PADDING + list.line_height * 2.0;
        let hit = list.row_at(left + 20.0, rows_top + list.line_height * 1.5);
        assert_eq!(hit, Some(1));
        // Outside the frame, and below the last row, there is nothing to hit.
        assert_eq!(list.row_at(left - 5.0, rows_top + 4.0), None);
        assert_eq!(list.row_at(left + 20.0, rows_top + list.line_height * 5.0), None);
    }

    #[test]
    fn an_empty_folder_says_so_instead_of_drawing_nothing() {
        let mut list = manager();
        list.open(Vec::new());
        assert_eq!(list.hint(), "No Plugins Installed");
        assert_eq!(list.toggle(), None);
        assert_eq!(list.visible(), 0);
    }

    #[test]
    fn the_state_word_follows_the_language() {
        let mut list = manager();
        list.open(rows());
        list.set_catalog(Some(crate::i18n::Catalog::load(None, "french.xml").expect("french")));
        assert_eq!(list.title(), "Gestionnaire de plugins");
        assert_eq!(
            list.hint(),
            format!("npp_align.wasm  {}", "Espace active ou désactive, Échap ferme")
        );
    }
}
