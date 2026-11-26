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
//! A centred modal list, one row per setting, drawn in the overlay pass the
//! menu and the completion popup already share. It edits a *copy* of the
//! settings and hands it back on every change, so the caller decides when to
//! apply and when to write the file — the dialog never touches the disk.
//!
//! Like the function list, the rows are one text area rather than one per row.
//! The font is monospace, so padding the labels to a fixed column lines the
//! values up without a second layout.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics};
use glyphon::{TextArea, TextBounds};

use crate::i18n::Catalog;
use crate::quad::QuadRenderer;
use crate::settings::{
    FileDialog, Settings, ThemeChoice, UiScale, MAX_AUTOSAVE, MAX_FONT_SIZE,
    MAX_PLAYGROUND_MINUTES, MAX_TAB_WIDTH, MIN_AUTOSAVE, MIN_FONT_SIZE,
    MIN_PLAYGROUND_MINUTES, MIN_TAB_WIDTH,
};
use crate::text::shaping_for;
use crate::theme::Theme;

/// The narrowest the dialog gets, in pixels.
///
/// The actual width is measured from the widest line: a translation is longer
/// than the English it replaces often enough that a fixed one would clip it.
const WIDTH: f32 = 440.0;
/// Padding inside the frame.
const PADDING: f32 = 14.0;
/// The column the values start at, in characters.
const VALUE_COLUMN: usize = 18;
/// The fewest rows a dropdown shows before it gives up and scrolls inside them.
///
/// Only reached in a window too short to hold the list on either side of its
/// row: a list of fifty themes needs a thousand pixels, and a window that has
/// less than that above *and* below still has to show
/// something you can move through.
const PICKER_MIN_ROWS: usize = 4;
/// Padding above and below a dropdown's rows.
const PICKER_PADDING: f32 = 4.0;
/// The narrowest a dropdown gets, so a short list is not a sliver.
const PICKER_MIN_WIDTH: f32 = 150.0;
/// The keys a list takes, shown on the panel's own last line.
///
/// The dialog's hint sits under the rows, and a list with room for every entry
/// is long enough to cover it. Blanking the hint is what stops it drawing
/// through the panel, so the panel carries the keys instead of losing them.
const PICKER_FOOTER: &str = "Up/Down  Left/Right  Enter  Esc";
/// The dialog's title.
const TITLE: &str = "Preferences";
/// The hint line under the rows.
const HINT: &str = "Up/Down  Left/Right  Enter open  Esc close";
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

/// One row of the list a preferences row opens.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Entry {
    /// A rule between one group of choices and the next.
    Divider,
    /// A choice, and the index it stands for in the row's own list.
    Choice {
        label: String,
        /// Into `ThemeChoice::ALL` for the theme row, into `languages` for the
        /// language row.
        index: usize,
    },
}

/// The list a preferences row opens.
///
/// Drawn as a panel below the row's value rather than as a second modal: it is
/// the row's content, and a modal would hide the dialog it belongs to.
struct Picker {
    /// The row it belongs to, which is what its choice means.
    row: Row,
    /// Its entries, top to bottom, divider included.
    entries: Vec<Entry>,
    /// The highlighted entry.
    selected: usize,
    /// The first entry drawn, for a list longer than its panel.
    first: usize,
    /// The entry that was in force when the list opened.
    ///
    /// The highlight puts its value in force as it moves, so the list has to
    /// remember where it started: Escape is the key that undoes a preview, and
    /// it cannot undo it to anywhere but here.
    opened_with: usize,
}

impl Picker {
    /// Moves the highlight by `delta` entries, stepping over dividers.
    ///
    /// Stops at the ends rather than wrapping: a list you can see the end of is
    /// a list where wrapping is a surprise. Returns whether the highlight moved,
    /// which is what a preview has to report: the value it stands for is put in
    /// force on every move, and a press that went nowhere has nothing to show.
    fn step(&mut self, delta: isize, capacity: usize) -> bool {
        let mut index = self.selected as isize;
        loop {
            index += delta;
            if index < 0 || index as usize >= self.entries.len() {
                return false;
            }
            if matches!(self.entries[index as usize], Entry::Choice { .. }) {
                break;
            }
        }
        let moved = index as usize != self.selected;
        self.selected = index as usize;
        self.scroll_into_view(capacity);
        moved
    }

    /// Keeps the highlight inside the visible window.
    fn scroll_into_view(&mut self, capacity: usize) {
        if capacity == 0 {
            return;
        }
        if self.selected < self.first {
            self.first = self.selected;
        }
        if self.selected >= self.first + capacity {
            self.first = self.selected + 1 - capacity;
        }
    }

    /// The widest label, in characters.
    fn widest(&self) -> usize {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Choice { label, .. } => Some(label.chars().count()),
                Entry::Divider => None,
            })
            .max()
            .unwrap_or(0)
    }

    /// The highlighted entry, when it is a choice.
    fn chosen(&self) -> Option<usize> {
        match self.entries.get(self.selected) {
            Some(Entry::Choice { index, .. }) => Some(*index),
            _ => None,
        }
    }
}

/// Where an open list draws itself.
struct Panel {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    /// Entries shown, which is fewer than the list holds when it scrolls.
    rows: usize,
    /// Whether the panel carries the keys, because the dialog's own hint is
    /// underneath it.
    footer: bool,
}

/// The modal preferences list.
pub struct Preferences {
    open: bool,
    selected: usize,
    /// The settings being edited. The caller reads this back after a change.
    settings: Settings,
    /// The localizations the Language row can step through, as `(file, name)`
    /// pairs. The first entry is always English, spelled `("", "English")`, so
    /// the row has a value even when nothing is installed.
    languages: Vec<(String, String)>,
    /// The list the highlighted row has open, if any.
    picker: Option<Picker>,
    /// The localization the panel's own words come from.
    ///
    /// `None` draws English. Every label this dialog owns goes through it, so a
    /// translated language translates the dialog too: a panel that answered in
    /// English under a menu bar answering in French was the mix this exists to
    /// stop.
    catalog: Option<Catalog>,
    title: Buffer,
    title_shaped: String,
    body: Buffer,
    body_shaped: String,
    picker_body: Buffer,
    picker_shaped: String,
    family: Family<'static>,
    /// The metrics the widget was last built with, so a density change can
    /// be told from the size already in hand.
    metrics: Metrics,
    char_width: f32,
    /// Window size, so the dialog can centre itself.
    viewport: (f32, f32),
}

impl Preferences {
    /// Builds a closed dialog.
    pub fn new(metrics: Metrics, family: Family<'static>, char_width: f32) -> Self {
        Self {
            open: false,
            selected: 0,
            settings: Settings::default(),
            languages: vec![(String::new(), "English".to_string())],
            picker: None,
            catalog: None,
            title: Buffer::new_empty(metrics),
            title_shaped: String::new(),
            body: Buffer::new_empty(metrics),
            body_shaped: String::new(),
            picker_body: Buffer::new_empty(metrics),
            picker_shaped: String::new(),
            family,
            metrics,
            char_width,
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
        self.picker_body.set_metrics(metrics);
    }

    /// The line height the widget was built with.
    ///
    /// Read from the metrics rather than kept beside them: a second copy is what
    /// left the rows and their highlight at the size the window opened at while
    /// the text around them grew.
    fn line_height(&self) -> f32 {
        self.metrics.line_height
    }

    /// Opens the dialog on a copy of `settings`.
    pub fn open(&mut self, settings: &Settings) {
        self.settings = settings.clone();
        self.selected = 0;
        self.open = true;
        self.picker = None;
    }

    /// Closes the dialog, discarding nothing: the caller already has the edits.
    pub fn close(&mut self) {
        self.open = false;
        self.picker = None;
    }

    /// Whether the dialog is showing.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The settings as edited so far.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Replaces the settings being edited.
    ///
    /// The Notepad++ import writes through here, so the dialog and the widgets
    /// keep reading from one place.
    pub fn set_settings(&mut self, settings: Settings) {
        self.settings = settings;
    }

    /// Replaces the keymap in the settings being edited.
    ///
    /// The shortcut editor hands its result back through here, so the dialog
    /// stays the one place the caller reads settings from.
    pub fn set_keymap(&mut self, keymap: crate::keymap::Keymap) {
        self.settings.keymap = keymap;
    }

    /// Replaces the list the Language row steps through.
    ///
    /// English is prepended here rather than by the caller, so the row can never
    /// be left with no value to show. The labels are the English names of the
    /// languages: the dialog is otherwise English, and a list sorted by its own
    /// labels has to be labelled in a script the sorter understands.
    ///
    /// An open list keeps the entries it was opened with. Every move of its
    /// highlight puts a language in force and the window is retranslated before
    /// the next key, so rebuilding the labels here would move the entry under
    /// the user's finger and pin a new language on top of the list they are
    /// reading. The list re-reads this set when it is opened again.
    pub fn set_languages(&mut self, list: &[crate::i18n::Language]) {
        self.languages = std::iter::once((String::new(), "English".to_string()))
            .chain(list.iter().map(|l| (l.file.clone(), l.english.clone())))
            .collect();
    }

    /// Swaps in a localization, or clears it with `None`.
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

    /// The label drawn on the left of `row`, in the current language.
    fn label(&self, row: Row) -> &str {
        self.tr(row.label())
    }

    /// The column the values start in, in characters.
    ///
    /// Measured from the longest label rather than fixed: translations are
    /// longer than their English often enough -- "Tabulación" is three
    /// characters past "Tab Width" -- that a fixed column runs the label into
    /// the value.
    fn value_column(&self) -> usize {
        Row::ALL
            .iter()
            .map(|row| self.label(*row).chars().count() + 2)
            .max()
            .unwrap_or(VALUE_COLUMN)
            .max(VALUE_COLUMN)
    }

    /// The highlighted row.
    pub fn selected(&self) -> Row {
        Row::ALL[self.selected.min(ROWS - 1)]
    }

    /// Opens the highlighted row's list. Returns whether it opened.
    ///
    /// The highlight starts on the value in force, so a list opened by mistake
    /// is closed with Escape and changes nothing.
    pub fn open_picker(&mut self) -> bool {
        let row = self.selected();
        let (entries, selected) = match row {
            Row::Theme => {
                let entries = self.theme_entries();
                let at = entries
                    .iter()
                    .position(|entry| match entry {
                        Entry::Choice { index, .. } => {
                            ThemeChoice::ALL[*index] == self.settings.theme
                        }
                        Entry::Divider => false,
                    })
                    .unwrap_or(0);
                (entries, at)
            }
            Row::Language => {
                let entries = self.language_entries();
                (entries, 0)
            }
            _ => return false,
        };
        self.picker = Some(Picker {
            row,
            entries,
            selected,
            first: 0,
            opened_with: selected,
        });
        // Scroll is clamped against the rows the window actually holds, which
        // means the list has to exist before the panel can be measured.
        let rows = self.picker_rows();
        if let Some(picker) = &mut self.picker {
            picker.scroll_into_view(rows);
        }
        self.picker_shaped.clear();
        true
    }

    /// Whether a list is open.
    pub fn picker_open(&self) -> bool {
        self.picker.is_some()
    }

    /// Closes the list and puts back the value it opened on.
    ///
    /// The highlight is a preview, so a list dismissed with Escape has to take
    /// the preview with it: three steps through the palettes and then Escape
    /// leaves the theme the list started on, not the third one. Returns whether
    /// there was a preview to undo.
    pub fn close_picker(&mut self) -> bool {
        let Some(picker) = self.picker.take() else {
            return false;
        };
        if picker.selected == picker.opened_with {
            return false;
        }
        match picker.entries.get(picker.opened_with) {
            Some(Entry::Choice { index, .. }) => self.set_choice(picker.row, *index),
            _ => false,
        }
    }

    /// Moves the list's highlight by `delta`, stepping over dividers.
    ///
    /// The entry it lands on is put in force straight away, which is the whole
    /// point of a list of fifty palettes: you compare two of them by pressing
    /// Down once, not by choosing blind and looking at the result afterwards.
    /// Returns whether the value changed.
    pub fn picker_move(&mut self, delta: isize) -> bool {
        let capacity = self.picker_rows();
        let moved = match &mut self.picker {
            Some(picker) => picker.step(delta, capacity),
            None => false,
        };
        moved && self.apply_picker_choice()
    }

    /// Moves the list's highlight by a screenful, for Page Up and Page Down.
    pub fn picker_page(&mut self, pages: isize) -> bool {
        let rows = self.picker_rows().max(1) as isize;
        self.picker_move(pages * (rows - 1))
    }

    /// Highlights the first entry of the list.
    pub fn picker_home(&mut self) -> bool {
        self.picker_jump(false)
    }

    /// Highlights the last entry of the list.
    pub fn picker_end(&mut self) -> bool {
        self.picker_jump(true)
    }

    /// Highlights the first or last choice, stepping over a leading or
    /// trailing rule. Returns whether the value changed.
    fn picker_jump(&mut self, last: bool) -> bool {
        let capacity = self.picker_rows();
        let Some(picker) = &mut self.picker else {
            return false;
        };
        let index = if last {
            picker
                .entries
                .iter()
                .rposition(|entry| matches!(entry, Entry::Choice { .. }))
        } else {
            picker
                .entries
                .iter()
                .position(|entry| matches!(entry, Entry::Choice { .. }))
        };
        let Some(index) = index else {
            return false;
        };
        if index == picker.selected {
            return false;
        }
        picker.selected = index;
        picker.scroll_into_view(capacity);
        self.apply_picker_choice()
    }

    /// Highlights entry `index`. Returns whether it was a choice.
    pub fn picker_select(&mut self, index: usize) -> bool {
        let Some(picker) = &mut self.picker else {
            return false;
        };
        if index >= picker.entries.len() {
            return false;
        }
        picker.selected = index;
        matches!(picker.entries[index], Entry::Choice { .. })
    }

    /// Puts the highlighted entry in force and closes the list.
    pub fn picker_choose(&mut self) {
        let Some(picker) = self.picker.take() else {
            return;
        };
        if let Some(index) = picker.chosen() {
            self.set_choice(picker.row, index);
        }
    }

    /// Puts the highlighted entry in force, leaving the list open.
    ///
    /// Returns whether the value changed.
    fn apply_picker_choice(&mut self) -> bool {
        let Some(picker) = &self.picker else {
            return false;
        };
        let (row, Some(index)) = (picker.row, picker.chosen()) else {
            return false;
        };
        self.set_choice(row, index)
    }

    /// Writes one entry of a row's list into the settings.
    ///
    /// The one place a choice becomes a value, so the preview, the choice and
    /// the revert cannot disagree about what an entry means. Returns whether the
    /// value changed.
    fn set_choice(&mut self, row: Row, index: usize) -> bool {
        let before = self.settings.clone();
        match row {
            Row::Theme => match ThemeChoice::ALL.get(index) {
                Some(choice) => self.settings.theme = *choice,
                None => return false,
            },
            Row::Language => match self.languages.get(index) {
                Some((file, _)) => {
                    self.settings.language = if file.is_empty() {
                        None
                    } else {
                        Some(file.clone())
                    };
                }
                None => return false,
            },
            _ => return false,
        }
        self.settings != before
    }

    /// The entry under `(x, y)`, as an index into the list's entries.
    pub fn picker_at(&self, x: f32, y: f32) -> Option<usize> {
        let picker = self.picker.as_ref()?;
        let (left, top, width, height) = self.picker_panel()?;
        if x < left || x > left + width || y < top || y > top + height {
            return None;
        }
        let row = ((y - top - PICKER_PADDING) / self.line_height()).floor();
        if row < 0.0 {
            return None;
        }
        let index = picker.first + row as usize;
        if index >= picker.entries.len() || row as usize >= self.picker_rows() {
            return None;
        }
        Some(index)
    }

    /// The theme row's entries: the three built-ins, a rule, the catalogue.
    fn theme_entries(&self) -> Vec<Entry> {
        let mut entries = Vec::new();
        let mut ruled = false;
        for (index, choice) in ThemeChoice::ALL.iter().enumerate() {
            // The first named palette is where "follow the desktop or not" stops
            // and the list of palettes starts, so that is where the rule goes --
            // derived rather than hard-coded, so adding a built-in moves it.
            if !ruled && crate::themes::palette_of(*choice).is_some() {
                entries.push(Entry::Divider);
                ruled = true;
            }
            entries.push(Entry::Choice {
                label: self.tr(choice.label()).to_string(),
                index,
            });
        }
        entries
    }

    /// The language row's entries: the language in force, a rule, then the rest
    /// alphabetically.
    ///
    /// The one in force is pinned on top because that is the row's value, and a
    /// list that omits it would look like it had lost the current language. It is
    /// not repeated below the rule, which is why its index is skipped.
    fn language_entries(&self) -> Vec<Entry> {
        let current = self.current_language();
        let mut entries = vec![Entry::Choice {
            label: self.languages[current].1.clone(),
            index: current,
        }];
        // The rule separates the pinned entry from the rest, so a list with no
        // rest is one entry and no rule under it.
        if self.languages.len() > 1 {
            entries.push(Entry::Divider);
        }
        for (index, (_, label)) in self.languages.iter().enumerate() {
            if index == current {
                continue;
            }
            entries.push(Entry::Choice {
                label: label.clone(),
                index,
            });
        }
        entries
    }

    /// The index in `languages` of the language in force.
    fn current_language(&self) -> usize {
        let file = self.settings.language.as_deref().unwrap_or("");
        self.languages
            .iter()
            .position(|(name, _)| name.as_str() == file)
            .unwrap_or(0)
    }

    /// How many list rows fit in `height` pixels.
    fn picker_rows_in(&self, height: f32) -> usize {
        ((height - PICKER_PADDING * 2.0).max(0.0) / self.line_height()).floor() as usize
    }

    /// The panel: `(left, top, width, height, rows)` in window pixels.
    ///
    /// The list is as tall as its contents until the window says otherwise. A
    /// panel capped at a dozen rows made a list of fifty themes look like a list
    /// of twelve, and the scroll that hid the rest was invisible: the only
    /// thing saying there was more was a missing entry you could not name. So
    /// the panel asks for the height it needs, is given it, and drops to a
    /// scroll only when the list is taller than the window it would stand in.
    fn picker_layout(&self) -> Option<Panel> {
        let picker = self.picker.as_ref()?;
        let entries = picker.entries.len();
        if entries == 0 {
            return None;
        }
        let (left, top) = self.origin();
        let rows_top = top + PADDING + self.line_height() * 2.0;
        let row = Row::ALL.iter().position(|r| *r == picker.row)?;
        let anchor = rows_top + row as f32 * self.line_height() + self.line_height();
        let (viewport_width, viewport_height) = self.viewport;
        let wanted = entries as f32 * self.line_height() + PICKER_PADDING * 2.0;
        // Without a window there is nothing to fit into, which is the state a
        // test builds and a headless frame renders: show the list whole.
        let below = if viewport_height > 0.0 {
            (viewport_height - anchor).max(0.0)
        } else {
            wanted
        };
        let above = if viewport_height > 0.0 {
            (anchor - self.line_height()).max(0.0)
        } else {
            0.0
        };
        let (y, mut rows) = if wanted <= below {
            (anchor, entries)
        } else if wanted <= above {
            (anchor - self.line_height() - wanted, entries)
        } else if below >= above {
            (
                anchor,
                self.picker_rows_in(below).clamp(PICKER_MIN_ROWS, entries),
            )
        } else {
            let rows = self.picker_rows_in(above).clamp(PICKER_MIN_ROWS, entries);
            let height = rows as f32 * self.line_height() + PICKER_PADDING * 2.0;
            ((anchor - self.line_height() - height).max(0.0), rows)
        };
        let mut height = rows as f32 * self.line_height() + PICKER_PADDING * 2.0;
        // A list that does not fit on one side of its row but does fit in the
        // window is still shown whole: the panel slides to wherever it can
        // stand, over the dialog and its title if it has to, rather than hiding
        // entries behind a scroll that says nothing about them. Only a list
        // taller than the window drops to a scroll.
        if rows < entries && viewport_height > 0.0 && wanted <= viewport_height - top.max(0.0) {
            rows = entries;
            height = rows as f32 * self.line_height() + PICKER_PADDING * 2.0;
        }
        let mut y = self.clamp_panel(y.max(0.0), height);
        // A list long enough to cover the dialog's hint carries the keys on its
        // own last line, provided the line still fits in the window.
        let hint_top = rows_top + ROWS as f32 * self.line_height() + self.line_height();
        let mut footer = y + height > hint_top;
        if footer {
            if viewport_height <= 0.0 || y + height + self.line_height() <= viewport_height {
                height += self.line_height();
                // Only upwards: the extra line has to stay in the window, and a
                // panel that moves up still covers the hint it was covering.
                y = y.min((viewport_height - height).max(0.0));
            } else if rows > 1 {
                // No room for an extra line, so an entry gives its row up for
                // it: losing the keys to a window edge is how a list becomes a
                // puzzle.
                rows -= 1;
            } else {
                footer = false;
            }
        }
        let widest = picker.widest().max(if footer {
            self.tr(PICKER_FOOTER).chars().count()
        } else {
            0
        });
        let width = (widest as f32 * self.char_width + PADDING * 2.0)
            .max(PICKER_MIN_WIDTH * crate::renderer::scale_of(self.metrics))
            .min(self.width());
        // Under the value, which is where the eye already is.
        let mut x = left + PADDING + self.value_column() as f32 * self.char_width;
        if viewport_width > 0.0 && x + width > viewport_width {
            x = (viewport_width - width).max(0.0);
        }
        Some(Panel {
            left: x,
            top: y,
            width,
            height,
            rows,
            footer,
        })
    }

    /// The panel's rectangle, `(left, top, width, height)`.
    fn picker_panel(&self) -> Option<(f32, f32, f32, f32)> {
        self.picker_layout()
            .map(|panel| (panel.left, panel.top, panel.width, panel.height))
    }

    /// Keeps a panel inside the window and off the dialog's title.
    ///
    /// A panel that runs off the top edge loses entries to the edge of the
    /// screen, and one that covers the word "Preferences" reads as a broken
    /// panel rather than as a long list. So the title is the floor, unless the
    /// list is too tall to fit under it at all.
    fn clamp_panel(&self, y: f32, height: f32) -> f32 {
        let (_, viewport_height) = self.viewport;
        if viewport_height <= 0.0 {
            return y;
        }
        let top = self.origin().1;
        let title_bottom = top + PADDING + self.line_height();
        let max_y = (viewport_height - height).max(0.0);
        let min_y = if title_bottom + height <= viewport_height {
            title_bottom
        } else {
            top.max(0.0)
        };
        y.clamp(min_y.min(max_y), max_y)
    }

    /// How many entries the open list shows, given the window it is in.
    fn picker_rows(&self) -> usize {
        self.picker_layout().map(|panel| panel.rows).unwrap_or(0)
    }

    /// Records the window size, so the dialog can centre itself.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.viewport = (width, height);
    }

    /// Moves the highlight by `delta`, clamping at both ends.
    ///
    /// Clamping rather than wrapping: a settings list has a first and a last
    /// row, and wrapping past them is disorienting when the list is short.
    pub fn move_selection(&mut self, delta: isize) {
        let last = ROWS as isize - 1;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
    }

    /// Highlights `index`, for a click.
    pub fn set_selected(&mut self, index: usize) {
        if index < ROWS {
            self.selected = index;
        }
    }

    /// Changes the highlighted row's value by `delta`.
    ///
    /// Returns whether anything changed, so the caller can skip a redraw and a
    /// disk write when a value is already at its limit.
    pub fn adjust(&mut self, delta: i32) -> bool {
        let before = self.settings.clone();
        match self.selected() {
            // Fifty values, so it steps rather than toggles.
            Row::Theme => {
                self.settings.theme = if delta >= 0 {
                    self.settings.theme.next()
                } else {
                    self.settings.theme.previous()
                };
            }
            Row::FontSize => {
                let size = self.settings.font_size + delta as f32;
                self.settings.font_size = size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
            }
            Row::UiScale => {
                // A short list of choices rather than a number: "follow the
                // display" has no numeric form, and the point of the row is to
                // reach a readable size in one or two presses.
                let all = UiScale::ALL;
                let at = all
                    .iter()
                    .position(|choice| *choice == UiScale(self.settings.ui_scale))
                    .unwrap_or(0) as i32;
                let next = (at + delta).clamp(0, all.len() as i32 - 1) as usize;
                self.settings.ui_scale = all[next].0;
            }
            Row::TabWidth => {
                let width = self.settings.tab_width as i32 + delta;
                self.settings.tab_width =
                    width.clamp(MIN_TAB_WIDTH as i32, MAX_TAB_WIDTH as i32) as u16;
            }
            Row::Wrap => self.settings.wrap = !self.settings.wrap,
            Row::Minimap => self.settings.minimap = !self.settings.minimap,
            Row::Toolbar => self.settings.toolbar = !self.settings.toolbar,
            Row::FunctionList => self.settings.function_list = !self.settings.function_list,
            Row::VolatileFind => self.settings.volatile_find = !self.settings.volatile_find,
            Row::FloatingSearch => self.settings.find_floating = !self.settings.find_floating,
            Row::Playground => self.settings.playground = !self.settings.playground,
            Row::PlaygroundAfter => {
                // One minute a press: an idle time is not something anyone
                // tunes in five-second steps.
                let minutes = self.settings.playground_after as i32 + delta;
                self.settings.playground_after = minutes
                    .clamp(MIN_PLAYGROUND_MINUTES as i32, MAX_PLAYGROUND_MINUTES as i32)
                    as u32;
            }
            // Three values rather than two, so it steps rather than toggles.
            Row::Backup => {
                self.settings.backup_mode = if delta >= 0 {
                    self.settings.backup_mode.next()
                } else {
                    self.settings.backup_mode.previous()
                };
            }
            Row::Autosave => {
                // Five-second steps: one second at a time would take sixty
                // presses to reach a minute, and nobody wants a one-second
                // autosave anyway.
                let seconds = self.settings.autosave_seconds as i32 + delta * 5;
                self.settings.autosave_seconds =
                    seconds.clamp(MIN_AUTOSAVE as i32, MAX_AUTOSAVE as i32) as u32;
            }
            // Two values, but a switch would read as on/off and neither choice
            // is the off one: both dialogs work.
            Row::FileDialog => {
                self.settings.file_dialog = if delta >= 0 {
                    self.settings.file_dialog.next()
                } else {
                    let all = FileDialog::ALL;
                    let at = all
                        .iter()
                        .position(|mode| *mode == self.settings.file_dialog)
                        .unwrap_or(0);
                    all[(at + all.len() - 1) % all.len()]
                };
            }
            Row::Language => {
                // Clamped, not wrapped: a language list is short and a wrap
                // from the last entry back to English reads as a mistake.
                let current = self
                    .languages
                    .iter()
                    .position(|(file, _)| {
                        file.as_str() == self.settings.language.as_deref().unwrap_or("")
                    })
                    .unwrap_or(0) as i32;
                let last = self.languages.len() as i32 - 1;
                let next = (current + delta).clamp(0, last) as usize;
                let file = self.languages[next].0.clone();
                self.settings.language = if file.is_empty() { None } else { Some(file) };
            }
            // Nothing to step through: the row opens a dialog instead.
            Row::Shortcuts => {}
        }
        self.settings != before
    }

    /// The value drawn on the right of `row`.
    pub fn value(&self, row: Row) -> String {
        let on_off = |value: bool| self.tr(if value { "On" } else { "Off" }).to_string();
        match row {
            Row::Theme => self.tr(self.settings.theme.label()).to_string(),
            Row::FontSize => format!("{}", self.settings.font_size),
            Row::UiScale => {
                if self.settings.ui_scale <= 0.0 {
                    self.tr("Follow the display").to_string()
                } else {
                    format!("x{}", UiScale(self.settings.ui_scale).label())
                }
            }
            Row::TabWidth => format!("{}", self.settings.tab_width),
            Row::Wrap => on_off(self.settings.wrap),
            Row::Minimap => on_off(self.settings.minimap),
            Row::Toolbar => on_off(self.settings.toolbar),
            Row::FunctionList => on_off(self.settings.function_list),
            Row::VolatileFind => on_off(self.settings.volatile_find),
            Row::FloatingSearch => on_off(self.settings.find_floating),
            Row::Playground => on_off(self.settings.playground),
            Row::PlaygroundAfter => {
                if self.settings.playground_after == 0 {
                    self.tr("Never").to_string()
                } else {
                    format!("{} min", self.settings.playground_after)
                }
            }
            Row::Backup => self.tr(self.settings.backup_mode.label()).to_string(),
            Row::Autosave => {
                if self.settings.autosave_seconds == 0 {
                    self.tr("Off").to_string()
                } else {
                    format!("{} s", self.settings.autosave_seconds)
                }
            }
            Row::Language => self
                .languages
                .iter()
                .find(|(file, _)| {
                    file.as_str() == self.settings.language.as_deref().unwrap_or("")
                })
                .map(|(_, name)| name.clone())
                .unwrap_or_else(|| "English".to_string()),
            Row::FileDialog => self.tr(self.settings.file_dialog.label()).to_string(),
            Row::Shortcuts => self.tr("Edit...").to_string(),
        }
    }

    /// The width the dialog occupies.
    ///
    /// Measured from the widest line, which is the hint in most languages and a
    /// long row in the others, and never narrower than [`WIDTH`].
    pub fn width(&self) -> f32 {
        let column = self.value_column();
        let widest = Row::ALL
            .iter()
            .map(|row| column + self.value(*row).chars().count())
            .chain([self.title().chars().count(), self.hint().chars().count()])
            .max()
            .unwrap_or(0);
        // The floor is a scale-1 number, so it doubles with the text rather
        // than pinning a dense display to a box measured for a sparse one.
        let floor = WIDTH * crate::renderer::scale_of(self.metrics);
        (widest as f32 * self.char_width + PADDING * 2.0).max(floor)
    }

    /// The height the dialog occupies.
    ///
    /// Title, a gap, the rows, a gap, and the hint, plus the padding above and
    /// below.
    pub fn height(&self) -> f32 {
        PADDING * 2.0 + (ROWS as f32 + 4.0) * self.line_height()
    }

    /// The top-left corner, centred in the window.
    fn origin(&self) -> (f32, f32) {
        let (width, height) = self.viewport;
        let left = ((width - self.width()) / 2.0).max(0.0);
        let top = ((height - self.height()) / 2.0).max(0.0);
        (left, top)
    }

    /// The row index under `(x, y)` in window pixels.
    pub fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        if !self.open {
            return None;
        }
        let (left, top) = self.origin();
        if x < left || x > left + self.width() {
            return None;
        }
        let rows_top = top + PADDING + self.line_height() * 2.0;
        if y < rows_top {
            return None;
        }
        let row = ((y - rows_top) / self.line_height()) as usize;
        (row < ROWS).then_some(row)
    }

    /// Shapes the title and the body if they changed.
    pub fn prepare(&mut self, font_system: &mut FontSystem) {
        if !self.open {
            return;
        }
        // The window can change while the list is open, so the scroll is
        // re-clamped against the rows that actually fit before anything is
        // drawn from it.
        let rows = self.picker_rows();
        if let Some(picker) = &mut self.picker {
            picker.scroll_into_view(rows);
        }

        let rows_top = self.origin().1 + PADDING + self.line_height() * 2.0;
        let title_top = self.origin().1 + PADDING;
        let covered = self
            .picker_panel()
            .map(|(_, top, _, height)| (top, top + height));
        // The title is drawn from its own buffer, so a list flipped above its
        // row covers it and the same blanking applies.
        let title_hidden = covered.is_some_and(|(panel_top, panel_bottom)| {
            panel_top < title_top + self.line_height() && panel_bottom > title_top
        });
        let title = if title_hidden {
            " ".repeat(self.title().len())
        } else {
            self.title().to_string()
        };
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
        // Where an open list covers a row's value, the value is not drawn.
        // Glyph passes here run after every rectangle, so a panel background
        // cannot hide text queued beside it -- blanking the covered cells is what
        // stops the values showing through the list.
        for (index, row) in Row::ALL.iter().enumerate() {
            if index > 0 {
                text.push('\n');
            }
            let label = self.label(*row);
            text.push_str(label);
            let column = self.value_column();
            for _ in label.chars().count()..column {
                text.push(' ');
            }
            let value = self.value(*row);
            let hidden = covered.is_some_and(|(panel_top, panel_bottom)| {
                let row_top = rows_top + index as f32 * self.line_height();
                let row_bottom = row_top + self.line_height();
                panel_top < row_bottom && panel_bottom > row_top
            });
            if hidden {
                text.push_str(&" ".repeat(value.chars().count()));
            } else {
                text.push_str(&value);
            }
        }
        // A blank line, then the hint, so the keys are always on screen. The
        // hint is blanked like a covered value when the list reaches it: the
        // last rows sit right above the hint, so a list opened on one of them
        // hangs past the bottom of the dialog and over the keys.
        text.push_str("\n\n");
        let hint_top = rows_top + ROWS as f32 * self.line_height() + self.line_height();
        let hint_hidden = covered.is_some_and(|(panel_top, panel_bottom)| {
            panel_top < hint_top + self.line_height() && panel_bottom > hint_top
        });
        if hint_hidden {
            text.push_str(&" ".repeat(self.hint().chars().count()));
        } else {
            text.push_str(&self.hint());
        }
        if text != self.body_shaped {
            self.body.set_text(
                &text,
                &Attrs::new().family(self.family),
                shaping_for(&text),
                None,
            );
            self.body.shape_until_scroll(font_system, false);
            self.body_shaped = text;
        }

        // Always: scrolling the list changes its text without changing the
        // dialog's, so the early return above cannot be allowed to skip it.
        self.prepare_picker(font_system);
    }

    /// Shapes the open list, one entry per line.
    ///
    /// A divider is an empty line: the line keeps the rows below it at the
    /// slots the hit test expects, and the rule itself is a quad.
    fn prepare_picker(&mut self, font_system: &mut FontSystem) {
        let Some(picker) = &self.picker else {
            return;
        };
        let capacity = self.picker_rows();
        let mut text = String::new();
        for (row, entry) in picker
            .entries
            .iter()
            .enumerate()
            .skip(picker.first)
            .take(capacity)
        {
            if row > picker.first {
                text.push('\n');
            }
            if let Entry::Choice { label, .. } = entry {
                text.push_str(label);
            }
        }
        if self.picker_layout().is_some_and(|panel| panel.footer) {
            text.push('\n');
            let footer = self.tr(PICKER_FOOTER).to_string();
            text.push_str(&footer);
        }
        if text == self.picker_shaped {
            return;
        }
        self.picker_body.set_text(
            &text,
            &Attrs::new().family(self.family),
            shaping_for(&text),
            None,
        );
        self.picker_body.shape_until_scroll(font_system, false);
        self.picker_shaped = text;
    }

    /// Queues the frame, the title, the selected row, and the hint.
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

        // The title sits above the rows, the hint below them.
        let rows_top = top + PADDING + self.line_height() * 2.0;
        quads.push(
            left + 1.0,
            rows_top + self.selected as f32 * self.line_height(),
            width - 2.0,
            self.line_height(),
            theme.menu_hover.to_linear(),
        );

        self.push_picker_quads(quads, theme);
    }

    /// Queues the open list: its panel, its rule, and the highlighted row.
    fn push_picker_quads(&self, quads: &mut QuadRenderer, theme: &Theme) {
        let (Some(picker), Some((left, top, width, height))) =
            (self.picker.as_ref(), self.picker_panel())
        else {
            return;
        };
        let border = theme.menu_border.to_linear();
        quads.push(left, top, width, height, theme.menu_background.to_linear());
        quads.push(left, top, width, 1.0, border);
        quads.push(left, top + height - 1.0, width, 1.0, border);
        quads.push(left, top, 1.0, height, border);
        quads.push(left + width - 1.0, top, 1.0, height, border);

        let capacity = self.picker_rows();
        for (row, entry) in picker
            .entries
            .iter()
            .enumerate()
            .skip(picker.first)
            .take(capacity)
        {
            let y = top + PICKER_PADDING + (row - picker.first) as f32 * self.line_height();
            match entry {
                Entry::Divider => {
                    // Halfway down the line the divider owns, so it reads as a
                    // rule between two rows rather than as an underline.
                    quads.push(
                        left + 1.0,
                        y + self.line_height() / 2.0,
                        width - 2.0,
                        1.0,
                        border,
                    );
                }
                Entry::Choice { .. } if row == picker.selected => {
                    quads.push(
                        left + 1.0,
                        y,
                        width - 2.0,
                        self.line_height(),
                        theme.menu_hover.to_linear(),
                    );
                }
                Entry::Choice { .. } => {}
            }
        }
        // A rule over the keys, so they read as the panel's footer rather than
        // as an entry that cannot be chosen.
        if self.picker_layout().is_some_and(|panel| panel.footer) {
            quads.push(
                left + 1.0,
                top + PICKER_PADDING + capacity as f32 * self.line_height(),
                width - 2.0,
                1.0,
                border,
            );
        }
    }

    /// The text areas: the title, the rows, and the hint.
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
        let mut areas = vec![
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
                top: top + PADDING + self.line_height() * 2.0,
                scale: 1.0,
                bounds,
                default_color: theme.foreground.to_cosmic(),
                custom_glyphs: &[],
            },
        ];
        if let Some((panel_left, panel_top, panel_width, panel_height)) = self.picker_panel() {
            areas.push(TextArea {
                buffer: &self.picker_body,
                left: panel_left + PADDING,
                top: panel_top + PICKER_PADDING,
                scale: 1.0,
                // Clipped to the panel, so a long list cannot draw over the
                // dialog that owns it.
                bounds: TextBounds {
                    left: panel_left as i32,
                    top: panel_top as i32,
                    right: (panel_left + panel_width) as i32,
                    bottom: (panel_top + panel_height) as i32,
                },
                default_color: theme.foreground.to_cosmic(),
                custom_glyphs: &[],
            });
        }
        areas
    }

    /// Every label this dialog can draw, for the coverage test.
    ///
    /// Derived from the rows and from the settings enums rather than written out
    /// twice: a new row or a new value is a label the test asks every language
    /// for, without anyone remembering to add it here.
    pub fn labels() -> Vec<&'static str> {
        let mut labels: Vec<&'static str> = Row::ALL.iter().map(|row| row.label()).collect();
        labels.extend([
            "Preferences",
            "On",
            "Off",
            "Never",
            "Follow the display",
            "Edit...",
            PICKER_FOOTER,
            HINT,
        ]);
        labels.extend(FileDialog::ALL.iter().map(|mode| mode.label()));
        labels.extend(
            nn_core::backup::BackupMode::ALL
                .iter()
                .map(|mode| mode.label()),
        );
        labels.extend(ThemeChoice::ALL.iter().map(|choice| choice.label()));
        labels
    }

    /// The title, drawn separately so it can use the label colour.
    pub fn title(&self) -> String {
        self.tr(TITLE).to_string()
    }

    /// The hint line under the rows.
    ///
    /// Short enough to fit in `WIDTH`: the row values and the title are far
    /// narrower, and a hint that runs past the frame is drawn clipped mid-word.
    pub fn hint(&self) -> String {
        self.tr(HINT).to_string()
    }

    /// The character width the dialog was built with.
    pub fn char_width(&self) -> f32 {
        self.char_width
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::Action;

    fn dialog() -> Preferences {
        Preferences::new(Metrics::new(15.0, 20.0), Family::Monospace, 8.0)
    }

    #[test]
    fn a_new_dialog_is_closed() {
        assert!(!dialog().is_open());
    }

    #[test]
    fn opening_copies_the_settings() {
        let mut prefs = dialog();
        let settings = Settings {
            theme: crate::settings::ThemeChoice::Light,
            font_size: 20.0,
            ..Settings::default()
        };
        prefs.open(&settings);
        assert!(prefs.is_open());
        assert_eq!(prefs.settings(), &settings);
    }

    #[test]
    fn the_first_row_is_the_theme() {
        assert_eq!(dialog().selected(), Row::Theme);
    }

    #[test]
    fn the_dialog_widens_with_the_text_it_holds() {
        let sparse = dialog();
        // The dense dialog of a display at twice the density: same box, twice
        // the font, so the character width doubles with the row height.
        let dense = Preferences::new(Metrics::new(30.0, 40.0), Family::Monospace, 16.0);
        // Both boxes are measured in characters, so the dense one is the same
        // number of columns and twice the pixels -- which is the whole point of
        // the floor passing through `scale_of`.
        assert_eq!(dense.width(), sparse.width() * 2.0);
    }

    #[test]
    fn selection_clamps_at_both_ends() {
        let mut prefs = dialog();
        prefs.move_selection(-1);
        assert_eq!(prefs.selected(), Row::Theme);
        prefs.move_selection(100);
        assert_eq!(prefs.selected(), Row::Shortcuts);
    }

    #[test]
    fn adjusting_the_theme_steps_through_every_choice() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        assert_eq!(prefs.value(Row::Theme), "System");
        assert!(prefs.adjust(1));
        assert_eq!(prefs.settings().theme, crate::settings::ThemeChoice::Light);
        assert!(prefs.adjust(1));
        assert_eq!(prefs.settings().theme, crate::settings::ThemeChoice::Dark);
        assert!(prefs.adjust(1));
        // The catalogue follows the two built-ins, alphabetically.
        assert_eq!(
            prefs.value(Row::Theme),
            crate::settings::ThemeChoice::ALL[3].label()
        );
        // One full lap comes back to where it started, which is what makes the
        // row usable as the only way to reach a theme in a dialog with no list.
        let count = crate::settings::ThemeChoice::ALL.len();
        for _ in 0..count - 3 {
            assert!(prefs.adjust(1));
        }
        assert_eq!(prefs.settings().theme, crate::settings::ThemeChoice::System);
        // And back the other way, wrapping past the first choice.
        assert!(prefs.adjust(-1));
        assert_eq!(
            prefs.settings().theme,
            crate::settings::ThemeChoice::ALL[count - 1]
        );
    }

    #[test]
    fn the_file_dialog_row_steps_between_the_two_dialogs() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        prefs.set_selected(Row::ALL.iter().position(|r| *r == Row::FileDialog).unwrap());
        assert_eq!(prefs.value(Row::FileDialog), "System dialog");
        assert!(prefs.adjust(1));
        assert_eq!(prefs.settings().file_dialog, FileDialog::Builtin);
        assert_eq!(prefs.value(Row::FileDialog), "Built-in browser");
        // Wraps, so the row never gets stuck.
        assert!(prefs.adjust(1));
        assert_eq!(prefs.settings().file_dialog, FileDialog::Portal);
        // And back the other way.
        assert!(prefs.adjust(-1));
        assert_eq!(prefs.settings().file_dialog, FileDialog::Builtin);
    }

    #[test]
    fn the_backup_row_steps_through_three_modes() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        prefs.set_selected(Row::ALL.iter().position(|r| *r == Row::Backup).unwrap());
        assert_eq!(prefs.value(Row::Backup), "None");
        assert!(prefs.adjust(1));
        assert_eq!(prefs.settings().backup_mode, nn_core::backup::BackupMode::Simple);
        assert_eq!(prefs.value(Row::Backup), "Simple");
        assert!(prefs.adjust(1));
        assert_eq!(prefs.value(Row::Backup), "Verbose");
        // Wraps, so the row never gets stuck.
        assert!(prefs.adjust(1));
        assert_eq!(prefs.value(Row::Backup), "None");
        assert!(prefs.adjust(-1));
        assert_eq!(prefs.value(Row::Backup), "Verbose");
    }

    #[test]
    fn the_autosave_row_steps_by_five_seconds_and_reaches_off() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        prefs.set_selected(Row::ALL.iter().position(|r| *r == Row::Autosave).unwrap());
        assert_eq!(prefs.value(Row::Autosave), "7 s");
        assert!(prefs.adjust(1));
        assert_eq!(prefs.settings().autosave_seconds, 12);
        // Down from twelve is seven, then two, then zero, which reads as Off.
        assert!(prefs.adjust(-1));
        assert_eq!(prefs.settings().autosave_seconds, 7);
        assert!(prefs.adjust(-1));
        assert_eq!(prefs.settings().autosave_seconds, 2);
        assert!(prefs.adjust(-1));
        assert_eq!(prefs.settings().autosave_seconds, 0);
        assert_eq!(prefs.value(Row::Autosave), "Off");
        // And stops there rather than wrapping to five minutes.
        assert!(!prefs.adjust(-1));
        assert_eq!(prefs.settings().autosave_seconds, 0);
    }

    #[test]
    fn adjusting_the_font_size_steps_by_one() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        prefs.set_selected(1);
        assert!(prefs.adjust(1));
        assert_eq!(prefs.settings().font_size, 16.0);
        assert!(prefs.adjust(-1));
        assert_eq!(prefs.settings().font_size, 15.0);
    }

    #[test]
    fn the_font_size_clamps_and_reports_no_change() {
        let mut prefs = dialog();
        prefs.open(&Settings {
            font_size: MAX_FONT_SIZE,
            ..Settings::default()
        });
        prefs.set_selected(1);
        assert!(!prefs.adjust(1));
        assert_eq!(prefs.settings().font_size, MAX_FONT_SIZE);
    }

    #[test]
    fn the_tab_width_clamps() {
        let mut prefs = dialog();
        prefs.open(&Settings {
            tab_width: MIN_TAB_WIDTH,
            ..Settings::default()
        });
        prefs.set_selected(2);
        assert!(!prefs.adjust(-1));
        assert_eq!(prefs.settings().tab_width, MIN_TAB_WIDTH);
    }

    #[test]
    fn a_switch_row_ignores_the_direction() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        let index = Row::ALL.iter().position(|r| *r == Row::Wrap).unwrap();
        prefs.set_selected(index);
        assert!(prefs.adjust(1));
        assert!(prefs.settings().wrap);
        assert!(prefs.adjust(-1));
        assert!(!prefs.settings().wrap);
    }

    #[test]
    fn the_volatile_find_row_toggles() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        let index = Row::ALL.iter().position(|r| *r == Row::VolatileFind).unwrap();
        prefs.set_selected(index);
        assert_eq!(prefs.value(Row::VolatileFind), "On");
        assert!(prefs.adjust(1));
        assert!(!prefs.settings().volatile_find);
        assert_eq!(prefs.value(Row::VolatileFind), "Off");
    }

    #[test]
    fn the_window_scale_row_steps_through_its_choices() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        let index = Row::ALL.iter().position(|r| *r == Row::UiScale).unwrap();
        prefs.set_selected(index);
        assert_eq!(prefs.value(Row::UiScale), "Follow the display");
        // Four choices in all: the display's own density, then three
        // multipliers for the screens that report one on a dense panel.
        assert!(prefs.adjust(1));
        assert_eq!(prefs.settings().ui_scale, 1.5);
        assert_eq!(prefs.value(Row::UiScale), "x1.5");
        assert!(prefs.adjust(1));
        assert_eq!(prefs.value(Row::UiScale), "x1.75");
        assert!(prefs.adjust(1));
        assert_eq!(prefs.value(Row::UiScale), "x2");
        // Both ends hold rather than wrapping, so holding the key cannot
        // walk the window past a size the furniture still fits in.
        assert!(!prefs.adjust(1));
        assert_eq!(prefs.settings().ui_scale, 2.0);
        prefs.adjust(-1);
        prefs.adjust(-1);
        assert!(prefs.adjust(-1));
        assert_eq!(prefs.settings().ui_scale, 0.0);
        assert_eq!(prefs.value(Row::UiScale), "Follow the display");
        assert!(!prefs.adjust(-1));
    }

    #[test]
    fn every_row_has_a_label_and_a_value() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        for row in Row::ALL {
            assert!(!row.label().is_empty(), "{row:?} has no label");
            assert!(!prefs.value(row).is_empty(), "{row:?} has no value");
        }
    }

    #[test]
    fn values_are_rendered_for_each_row() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        assert_eq!(prefs.value(Row::Theme), "System");
        assert_eq!(prefs.value(Row::FontSize), "15");
        assert_eq!(prefs.value(Row::TabWidth), "4");
        assert_eq!(prefs.value(Row::Wrap), "Off");
        assert_eq!(prefs.value(Row::Minimap), "On");
        assert_eq!(prefs.value(Row::Toolbar), "On");
        assert_eq!(prefs.value(Row::FunctionList), "Off");
    }

    #[test]
    fn the_dialog_is_centred_in_the_window() {
        let mut prefs = dialog();
        prefs.set_viewport(1000.0, 800.0);
        let (left, top) = prefs.origin();
        assert_eq!(left, (1000.0 - WIDTH) / 2.0);
        assert_eq!(top, (800.0 - prefs.height()) / 2.0);
    }

    #[test]
    fn a_window_smaller_than_the_dialog_pins_it_to_the_corner() {
        let mut prefs = dialog();
        prefs.set_viewport(100.0, 100.0);
        assert_eq!(prefs.origin(), (0.0, 0.0));
    }

    #[test]
    fn row_at_maps_a_click_to_a_row() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        prefs.set_viewport(1000.0, 800.0);
        let (left, top) = prefs.origin();
        let rows_top = top + PADDING + prefs.line_height() * 2.0;
        let x = left + 20.0;
        assert_eq!(prefs.row_at(x, rows_top + 5.0), Some(0));
        assert_eq!(prefs.row_at(x, rows_top + prefs.line_height() + 5.0), Some(1));
        assert_eq!(prefs.row_at(x, rows_top - 5.0), None);
        assert_eq!(prefs.row_at(left - 5.0, rows_top + 5.0), None);
    }

    #[test]
    fn a_new_density_moves_the_rows_and_their_highlight() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        prefs.set_viewport(1000.0, 900.0);
        // The row pitch and the highlight over it are the metrics' line height.
        // A copy kept beside the metrics is what left both at the size the
        // window opened at while the labels grew, which is the fault this test
        // exists to keep from coming back: the preference that changes the
        // scale is edited in this very dialog.
        prefs.set_metrics(Metrics::new(30.0, 40.0));
        assert_eq!(prefs.line_height(), 40.0);
        let (left, top) = prefs.origin();
        let rows_top = top + PADDING + 40.0 * 2.0;
        assert_eq!(prefs.row_at(left + 20.0, rows_top + 45.0), Some(1));
        assert_eq!(prefs.row_at(left + 20.0, rows_top + 85.0), Some(2));
        // The box grows with them: the rows are the height's largest term.
        assert_eq!(prefs.height(), PADDING * 2.0 + (ROWS as f32 + 4.0) * 40.0);
    }

    #[test]
    fn row_at_past_the_last_row_is_none() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        prefs.set_viewport(1000.0, 800.0);
        let (left, top) = prefs.origin();
        let rows_top = top + PADDING + prefs.line_height() * 2.0;
        let y = rows_top + ROWS as f32 * prefs.line_height() + 5.0;
        assert_eq!(prefs.row_at(left + 20.0, y), None);
    }

    #[test]
    fn a_closed_dialog_has_no_rows() {
        let prefs = dialog();
        assert_eq!(prefs.row_at(10.0, 10.0), None);
    }

    #[test]
    fn every_row_has_a_label() {
        for row in Row::ALL {
            assert!(!row.label().is_empty());
        }
    }

    #[test]
    fn the_switch_rows_are_the_ones_without_numbers() {
        // The theme has fifty values, so it is not a switch.
        assert!(!Row::Theme.is_switch());
        assert!(!Row::FontSize.is_switch());
        assert!(!Row::TabWidth.is_switch());
        assert!(Row::Wrap.is_switch());
        assert!(Row::Minimap.is_switch());
        assert!(Row::Toolbar.is_switch());
        assert!(Row::FunctionList.is_switch());
        assert!(!Row::Shortcuts.is_switch());
    }

    #[test]
    fn the_shortcuts_row_is_the_only_action_row() {
        for row in Row::ALL {
            assert_eq!(row.is_action(), row == Row::Shortcuts);
        }
    }

    #[test]
    fn the_shortcuts_row_has_nothing_to_step_through() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        prefs.set_selected(ROWS - 1);
        assert!(!prefs.adjust(1));
        assert!(!prefs.adjust(-1));
        assert_eq!(prefs.value(Row::Shortcuts), "Edit...");
    }

    /// Opens the list of `row`, which the caller has already opened.
    fn open_row(prefs: &mut Preferences, row: Row) {
        prefs.set_selected(Row::ALL.iter().position(|r| *r == row).expect("the row"));
        assert!(prefs.open_picker());
    }

    /// The labels an open list shows, with a rule as an empty string.
    fn labels(prefs: &Preferences) -> Vec<String> {
        prefs
            .picker
            .as_ref()
            .expect("an open list")
            .entries
            .iter()
            .map(|entry| match entry {
                Entry::Choice { label, .. } => label.clone(),
                Entry::Divider => String::new(),
            })
            .collect()
    }

    /// A localization as the catalogue reports it.
    fn language(file: &str, english: &str) -> crate::i18n::Language {
        crate::i18n::Language {
            name: english.to_string(),
            english: english.to_string(),
            file: file.to_string(),
        }
    }

    #[test]
    fn the_theme_row_opens_every_palette_under_one_rule() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Theme);
        let labels = labels(&prefs);
        // The three built-ins, a rule, then the catalogue: the palette list
        // starts where "follow the desktop" stops.
        assert_eq!(&labels[..4], ["System", "Light", "Dark", ""]);
        assert_eq!(labels.len(), crate::settings::ThemeChoice::ALL.len() + 1);
    }

    #[test]
    fn the_language_row_pins_what_is_in_force_above_the_rule() {
        let mut prefs = dialog();
        prefs.set_languages(&[
            language("danish.xml", "Danish"),
            language("german.xml", "German"),
            language("italian.xml", "Italian"),
        ]);
        prefs.open(&Settings {
            language: Some("german.xml".to_string()),
            ..Settings::default()
        });
        open_row(&mut prefs, Row::Language);
        // The one in force on top, a rule, then the rest -- and not the one in
        // force a second time below it.
        assert_eq!(labels(&prefs), ["German", "", "English", "Danish", "Italian"]);
    }

    #[test]
    fn a_list_with_no_other_language_has_no_rule() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Language);
        assert_eq!(labels(&prefs), ["English"]);
    }

    #[test]
    fn the_list_opens_on_the_value_in_force() {
        let mut prefs = dialog();
        let last = crate::settings::ThemeChoice::ALL[crate::settings::ThemeChoice::ALL.len() - 1];
        prefs.open(&Settings {
            theme: last,
            ..Settings::default()
        });
        open_row(&mut prefs, Row::Theme);
        let picker = prefs.picker.as_ref().expect("an open list");
        assert_eq!(
            picker.chosen(),
            Some(crate::settings::ThemeChoice::ALL.len() - 1)
        );
        // And scrolled to it, so the highlight is drawn somewhere in the panel.
        assert!(picker.selected >= picker.first);
        assert!(picker.selected < picker.first + prefs.picker_rows());
    }

    /// The entries the panel shows, as opposed to the entries the list holds.
    fn visible(prefs: &Preferences) -> usize {
        prefs.picker_rows()
    }

    #[test]
    fn a_list_with_room_shows_every_entry() {
        // Five installed languages, the English one on top and the rule under
        // it: seven rows, all of them drawn. A panel that stopped at a fixed
        // dozen would be indistinguishable here, which is why the theme list --
        // fifty rows, taller than the window -- has its own test.
        let mut prefs = dialog();
        prefs.set_viewport(1000.0, 800.0);
        let mut installed: Vec<crate::i18n::Language> = Vec::new();
        for (file, english) in [
            ("danish.xml", "Danish"),
            ("dutch.xml", "Dutch"),
            ("finnish.xml", "Finnish"),
            ("german.xml", "German"),
            ("greek.xml", "Greek"),
        ] {
            installed.push(language(file, english));
        }
        prefs.set_languages(&installed);
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Language);
        assert_eq!(visible(&prefs), 7);
    }

    #[test]
    fn a_list_taller_than_its_row_reaches_the_top_of_the_window() {
        // Every language the build carries plus English and the rule is
        // twenty-five rows; there are not twenty-five rows of room below the
        // Language row, and hiding the last two behind a scroll with no
        // scrollbar is how a translation looks missing when it is not.
        let mut prefs = dialog();
        prefs.set_viewport(1000.0, 800.0);
        prefs.set_languages(&crate::i18n::Catalog::languages(None));
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Language);
        let entries = crate::i18n::Catalog::languages(None).len() + 2;
        assert_eq!(visible(&prefs), entries);
        let panel = prefs.picker_layout().expect("a panel");
        assert!(panel.top >= 0.0);
        assert!(panel.top + panel.height <= 800.0);
    }

    #[test]
    fn the_theme_list_scrolls_rather_than_running_off_the_window() {
        // Fifty themes are more than eight hundred pixels of rows, so the panel
        // shows what fits and keeps the rest a scroll away instead of a clip
        // away. The point of asking for the height the list needs is that the
        // only case that scrolls is the one that has to.
        let mut prefs = dialog();
        prefs.set_viewport(1000.0, 800.0);
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Theme);
        let rows = visible(&prefs);
        assert!(rows < crate::settings::ThemeChoice::ALL.len() + 1);
        assert!(rows >= 20, "a panel of {rows} rows is not a list you can read");
        let panel = prefs.picker_layout().expect("a panel");
        assert!(panel.top >= 0.0);
        assert!(panel.top + panel.height <= 800.0);
        // And the last theme comes into view rather than staying below the fold.
        prefs.picker_end();
        let picker = prefs.picker.as_ref().expect("an open list");
        assert_eq!(
            picker.chosen(),
            Some(crate::settings::ThemeChoice::ALL.len() - 1)
        );
        assert!(picker.selected >= picker.first);
        assert!(picker.selected < picker.first + rows);
    }

    #[test]
    fn the_language_list_moves_up_and_still_shows_every_entry() {
        let mut prefs = dialog();
        prefs.set_viewport(1000.0, 800.0);
        let mut installed: Vec<crate::i18n::Language> = Vec::new();
        for index in 0..20 {
            installed.push(language(
                &format!("lang{index}.xml"),
                &format!("Language {index:02}"),
            ));
        }
        prefs.set_languages(&installed);
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Language);
        // Twenty-two entries do not fit under the second-to-last row, so the
        // panel slides up over the dialog's own rows rather than clipping the
        // list or running off the top of the window.
        assert_eq!(visible(&prefs), 22);
        let panel = prefs.picker_layout().expect("a panel");
        assert!(panel.top >= 0.0);
        assert!(panel.top + panel.height <= 800.0);
        // And it still starts below the title, so the dialog keeps its name.
        let title_bottom = prefs.origin().1 + PADDING + prefs.line_height();
        assert!(panel.top >= title_bottom);
    }

    #[test]
    fn a_window_too_short_for_the_list_scrolls_it() {
        let mut prefs = dialog();
        prefs.set_viewport(1000.0, 300.0);
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Theme);
        let rows = visible(&prefs);
        assert!(rows < crate::settings::ThemeChoice::ALL.len() + 1);
        assert!(rows >= PICKER_MIN_ROWS);
        // Stepping past the bottom scrolls rather than running off the panel.
        for _ in 0..rows {
            prefs.picker_move(1);
        }
        let picker = prefs.picker.as_ref().expect("an open list");
        assert!(picker.selected < picker.first + rows);
        assert!(picker.first > 0);
    }

    #[test]
    fn end_and_home_reach_both_ends_of_the_list() {
        let mut prefs = dialog();
        prefs.set_viewport(1000.0, 300.0);
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Theme);
        prefs.picker_end();
        let picker = prefs.picker.as_ref().expect("an open list");
        assert_eq!(
            picker.chosen(),
            Some(crate::settings::ThemeChoice::ALL.len() - 1)
        );
        prefs.picker_home();
        let picker = prefs.picker.as_ref().expect("an open list");
        // Straight to System, over the rule above the catalogue.
        assert_eq!(picker.chosen(), Some(0));
        assert_eq!(picker.first, 0);
    }

    #[test]
    fn a_panel_too_tall_for_the_title_blanks_it() {
        let mut prefs = dialog();
        // A list this long cannot fit under the title in a window this short,
        // so the panel takes the whole dialog and the title is blanked rather
        // than drawn through it.
        prefs.set_viewport(1000.0, 300.0);
        let mut installed: Vec<crate::i18n::Language> = Vec::new();
        for index in 0..30 {
            installed.push(language(
                &format!("lang{index}.xml"),
                &format!("Language {index:02}"),
            ));
        }
        prefs.set_languages(&installed);
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Language);
        prefs.prepare(&mut FontSystem::new());
        assert_eq!(prefs.title_shaped.trim(), "");
        prefs.close_picker();
        prefs.prepare(&mut FontSystem::new());
        assert_eq!(prefs.title_shaped, prefs.title());
    }

    #[test]
    fn moving_the_highlight_steps_over_the_rule() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Theme);
        prefs.picker_move(1);
        prefs.picker_move(1);
        prefs.picker_move(1);
        // Past Dark, which is the last built-in: the rule is not a stop.
        let picker = prefs.picker.as_ref().expect("an open list");
        assert_eq!(picker.chosen(), Some(3));
    }

    #[test]
    fn the_highlight_stops_at_both_ends() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Theme);
        prefs.picker_move(-1);
        assert_eq!(
            prefs.picker.as_ref().expect("an open list").chosen(),
            Some(0)
        );
        // Repeated moves stop on the last entry rather than wrapping.
        for _ in 0..crate::settings::ThemeChoice::ALL.len() + 4 {
            prefs.picker_move(1);
        }
        assert_eq!(
            prefs.picker.as_ref().expect("an open list").chosen(),
            Some(crate::settings::ThemeChoice::ALL.len() - 1)
        );
        // A press that went nowhere says so, so the caller does not repaint a
        // window for a value that did not change.
        assert!(!prefs.picker_move(1));
    }

    #[test]
    fn moving_the_highlight_puts_the_theme_in_force() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Theme);
        // System is in force and the list opens on it; one step down is Light,
        // and Light is on the window before the list closes.
        assert!(prefs.picker_move(1));
        assert_eq!(prefs.settings().theme, crate::settings::ThemeChoice::Light);
        assert!(prefs.picker_move(1));
        assert_eq!(prefs.settings().theme, crate::settings::ThemeChoice::Dark);
        // And back up, one palette at a time.
        assert!(prefs.picker_move(-1));
        assert_eq!(prefs.settings().theme, crate::settings::ThemeChoice::Light);
    }

    #[test]
    fn choosing_a_theme_applies_it_and_closes_the_list() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Theme);
        prefs.picker_move(1);
        // The move already previewed Light, so choosing it changes nothing: the
        // list is the thing that closes.
        assert_eq!(prefs.settings().theme, crate::settings::ThemeChoice::Light);
        prefs.picker_choose();
        assert_eq!(prefs.settings().theme, crate::settings::ThemeChoice::Light);
        assert!(!prefs.picker_open());
        // The dialog stays up: choosing a theme is not closing the dialog.
        assert!(prefs.is_open());
    }

    #[test]
    fn choosing_what_is_already_in_force_changes_nothing() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Theme);
        prefs.picker_choose();
        assert_eq!(prefs.settings().theme, crate::settings::ThemeChoice::System);
        assert!(!prefs.picker_open());
    }

    #[test]
    fn escape_puts_back_the_theme_the_list_opened_on() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Theme);
        prefs.picker_move(1);
        prefs.picker_move(1);
        assert_eq!(prefs.settings().theme, crate::settings::ThemeChoice::Dark);
        // Escape is the undo of the preview: the theme the list opened on comes
        // back, not the one the highlight happened to stop on.
        assert!(prefs.close_picker());
        assert!(!prefs.picker_open());
        assert_eq!(prefs.settings().theme, crate::settings::ThemeChoice::System);
    }

    #[test]
    fn escape_without_moving_reports_no_preview_to_undo() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Theme);
        assert!(!prefs.close_picker());
        assert_eq!(prefs.settings().theme, crate::settings::ThemeChoice::System);
    }

    #[test]
    fn choosing_a_language_stores_its_file() {
        let mut prefs = dialog();
        prefs.set_languages(&[language("german.xml", "German")]);
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Language);
        // English is pinned first, the rule second, German under it.
        assert!(prefs.picker_select(2));
        prefs.picker_choose();
        assert_eq!(prefs.settings().language.as_deref(), Some("german.xml"));
    }

    #[test]
    fn moving_the_highlight_puts_the_language_in_force() {
        let mut prefs = dialog();
        prefs.set_languages(&[
            language("german.xml", "German"),
            language("italian.xml", "Italian"),
        ]);
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Language);
        // English is pinned on top with the rule under it, so one step down is
        // German -- and German is in force while the list is still open, which
        // is the point: you read the menus before you settle on them.
        assert!(prefs.picker_move(1));
        assert_eq!(prefs.settings().language.as_deref(), Some("german.xml"));
        assert_eq!(prefs.value(Row::Language), "German");
        assert!(prefs.picker_move(1));
        assert_eq!(prefs.settings().language.as_deref(), Some("italian.xml"));
        // Escape puts back the language the list opened on.
        assert!(prefs.close_picker());
        assert_eq!(prefs.settings().language, None);
        assert_eq!(prefs.value(Row::Language), "English");
    }

    #[test]
    fn choosing_english_clears_the_language() {
        let mut prefs = dialog();
        prefs.set_languages(&[language("italian.xml", "Italian")]);
        prefs.open(&Settings {
            language: Some("italian.xml".to_string()),
            ..Settings::default()
        });
        open_row(&mut prefs, Row::Language);
        assert_eq!(labels(&prefs), ["Italian", "", "English"]);
        assert!(prefs.picker_select(2));
        prefs.picker_choose();
        assert_eq!(prefs.settings().language, None);
        assert_eq!(prefs.value(Row::Language), "English");
    }

    #[test]
    fn a_click_inside_the_panel_picks_the_entry_under_it() {
        let mut prefs = dialog();
        prefs.set_viewport(1000.0, 800.0);
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Theme);
        let (left, top, _, _) = prefs.picker_panel().expect("a panel");
        let x = left + 10.0;
        let y = top + PICKER_PADDING + 1.5 * prefs.line_height();
        assert_eq!(prefs.picker_at(x, y), Some(1));
        assert!(prefs.picker_select(1));
        prefs.picker_choose();
        assert_eq!(prefs.settings().theme, crate::settings::ThemeChoice::Light);
    }

    #[test]
    fn a_click_outside_the_rows_hits_nothing() {
        let mut prefs = dialog();
        prefs.set_viewport(1000.0, 800.0);
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Theme);
        let (left, top, width, _) = prefs.picker_panel().expect("a panel");
        assert_eq!(prefs.picker_at(left - 5.0, top + 10.0), None);
        assert_eq!(prefs.picker_at(left + width + 5.0, top + 10.0), None);
        // The padding above the first row is the panel, but not an entry.
        assert_eq!(prefs.picker_at(left + 10.0, top + 1.0), None);
        assert_eq!(prefs.picker_at(left + 10.0, top + 10.0), Some(0));
    }

    #[test]
    fn a_list_with_no_room_below_opens_above_its_row() {
        let mut prefs = dialog();
        prefs.set_viewport(1000.0, 300.0);
        prefs.open(&Settings::default());
        // The row's own bottom edge: two lines of title, the rows above it, and
        // the row itself.
        let bottom_of = |prefs: &Preferences, row: Row| {
            let above = Row::ALL.iter().position(|each| *each == row).expect("a row");
            prefs.origin().1 + PADDING + prefs.line_height() * (2.0 + above as f32 + 1.0)
        };
        // The language row is near the bottom, so nothing fits under it in a
        // window this short and the panel goes above.
        open_row(&mut prefs, Row::Language);
        let (_, top, _, height) = prefs.picker_panel().expect("a panel");
        let anchor = bottom_of(&prefs, Row::Language);
        assert!(top + height <= anchor);
        assert!(top + height <= 300.0);
        // And the theme row, with room below, stays under its row instead.
        prefs.close_picker();
        open_row(&mut prefs, Row::Theme);
        let (_, top, _, height) = prefs.picker_panel().expect("a panel");
        let anchor = bottom_of(&prefs, Row::Theme);
        assert!(top >= anchor);
        assert!(top + height <= 300.0);
    }

    #[test]
    fn closing_the_list_leaves_the_dialog_open() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Theme);
        prefs.close_picker();
        assert!(!prefs.picker_open());
        assert!(prefs.is_open());
        // And a row that holds a value rather than a list opens nothing.
        prefs.set_selected(1);
        assert!(!prefs.open_picker());
    }

    #[test]
    fn only_the_theme_and_language_rows_open_lists() {
        for row in Row::ALL {
            assert_eq!(
                row.opens_picker(),
                matches!(row, Row::Theme | Row::Language)
            );
        }
    }

    #[test]
    fn the_hint_fits_inside_the_frame() {
        // The frame is sized for its widest line, so a hint that outgrew it
        // would be drawn clipped mid-word.
        let prefs = Preferences::new(Metrics::new(15.0, 20.0), Family::Monospace, 9.0);
        let width = prefs.hint().chars().count() as f32 * prefs.char_width();
        assert!(width + PADDING * 2.0 <= prefs.width());
    }

    #[test]
    fn a_list_over_the_hint_blanks_it() {
        let mut prefs = dialog();
        // Ten rows fit in a short window, and they end well above the hint, so
        // the keys stay on screen for as long as the list is short enough.
        prefs.set_viewport(1000.0, 300.0);
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Theme);
        prefs.prepare(&mut FontSystem::new());
        assert!(prefs.body_shaped.ends_with(&prefs.hint()));
        // Give the window room for the whole list and the panel reaches the
        // hint, which is blanked rather than drawn through the panel.
        prefs.set_viewport(1000.0, 800.0);
        prefs.prepare(&mut FontSystem::new());
        assert!(!prefs.body_shaped.contains("Esc close"));
        assert!(prefs
            .body_shaped
            .ends_with(&" ".repeat(prefs.hint().chars().count())));
    }

    #[test]
    fn a_list_over_the_hint_carries_the_keys_on_its_own_line() {
        let mut prefs = dialog();
        prefs.set_viewport(1000.0, 800.0);
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Theme);
        prefs.prepare(&mut FontSystem::new());
        let panel = prefs.picker_layout().expect("a panel");
        assert!(panel.footer);
        assert!(prefs.picker_shaped.ends_with(PICKER_FOOTER));
        // The footer is not an entry: a click on its line chooses nothing, and
        // the last entry is still the last entry.
        let (_, top, width, _) = prefs.picker_panel().expect("a panel");
        let y = top + PICKER_PADDING + panel.rows as f32 * prefs.line_height() + 1.5;
        assert_eq!(prefs.picker_at(prefs.origin().0 + width, y), None);
    }

    #[test]
    fn a_short_list_leaves_the_dialog_hint_on_screen() {
        let mut prefs = dialog();
        prefs.set_viewport(1000.0, 300.0);
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Theme);
        prefs.prepare(&mut FontSystem::new());
        // Ten rows end above the hint, so the panel does not repeat the keys.
        let panel = prefs.picker_layout().expect("a panel");
        assert!(!panel.footer);
        assert!(!prefs.picker_shaped.contains("Enter"));
    }

    #[test]
    fn a_covered_value_is_not_drawn() {
        let mut prefs = dialog();
        prefs.set_viewport(1000.0, 800.0);
        prefs.open(&Settings::default());
        open_row(&mut prefs, Row::Theme);
        prefs.prepare(&mut FontSystem::new());
        // The panel hangs below the theme row, so the rows under it have their
        // values blanked rather than drawn through the list.
        let second = |prefs: &Preferences| {
            prefs
                .body_shaped
                .lines()
                .nth(1)
                .expect("a second line")
                .to_string()
        };
        let value = prefs.value(Row::FontSize);
        let blanked = format!(
            "{:<18}{}",
            "Font Size",
            " ".repeat(value.chars().count())
        );
        assert_eq!(second(&prefs), blanked);
        prefs.close_picker();
        prefs.prepare(&mut FontSystem::new());
        assert_eq!(second(&prefs), format!("{:<18}{value}", "Font Size"));
    }

    #[test]
    fn the_keymap_can_be_replaced_through_the_dialog() {
        let mut prefs = dialog();
        prefs.open(&Settings::default());
        let mut keymap = crate::keymap::Keymap::default();
        keymap.set(Action::Save, crate::keymap::Chord::parse("F5").unwrap());
        prefs.set_keymap(keymap.clone());
        assert_eq!(prefs.settings().keymap, keymap);
    }
}
