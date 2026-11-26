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

//! The find and replace bar.
//!
//! A stack of rows along the bottom of the window: the pattern and its match
//! count, the replacement when Replace is open, a row of setting chips, and the
//! action buttons under them. It owns its own state and its own layout so that
//! the editor view only has to ask it for rectangles and text areas.
//!
//! The buttons are on their own rows because Notepad++'s Find dialog has ten of
//! them, and ten buttons carrying words cannot share a line with eleven chips in
//! a window of any width. They wrap instead: the bar grows a row when the next
//! button does not fit, which is also what keeps a translation -- usually longer
//! than the English -- from drawing one button on top of another.
//!
//! The bar is keyboard-first: `Ctrl+F` opens it, typing goes into the query,
//! `Enter` walks through the matches in the direction the bar is set to,
//! `Enter` in the replacement field replaces the match under the caret,
//! `Escape` closes. The settings and the buttons are clickable too, because a
//! setting you cannot click is a setting people will not find.

use std::ops::Range;

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use glyphon::{TextArea, TextBounds};
use nn_core::{Document, Search, SearchMode, SearchOptions};

use crate::i18n::Catalog;
use crate::quad::QuadRenderer;
use crate::theme::{mix, Rgb, Theme, ToGpu};

/// Which field has the caret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// The search pattern.
    Query,
    /// The replacement text.
    Replacement,
}

/// Space between a label and the field beside it.
const LABEL_GAP: f32 = 10.0;

/// Width of a setting chip, marks being at most two characters wide.
const CHIP_W: f32 = 26.0;

/// Space between two chips, and between a chip and the field above it.
const CHIP_GAP: f32 = 6.0;

/// Padding inside a button, left and right of its word.
const BUTTON_PAD: f32 = 10.0;

/// Width of the little arrow that opens a field's history.
const TOGGLE_W: f32 = 18.0;

/// How many remembered patterns a field keeps.
pub const MAX_HISTORY: usize = 20;

/// How many of them the open list shows at once.
const VISIBLE_HISTORY: usize = 8;

/// One of the settings in the bar's option row, drawn as a chip.
///
/// A chip carries a mark rather than a word. Notepad++ writes these out as
/// "Match case", "Wrap around" and so on, but a docked bar has one line to hold
/// nine of them, and a mark is the same in every language the app speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chip {
    /// Match case.
    Case,
    /// Whole word only, or `.` matching a newline once the pattern is a regex.
    Word,
    /// Normal search: the pattern is literal text.
    Normal,
    /// Extended search: literal text with `\n`, `\t` and friends read.
    Extended,
    /// Regular expression.
    Regex,
    /// Continue at the other end of the document when this one is reached.
    Wrap,
    /// Search only inside the selection.
    Selection,
    /// Search upwards.
    Up,
    /// Search downwards.
    Down,
    /// Drop a bookmark on every line a mark lands on.
    Bookmark,
    /// Clear the previous marks before marking again.
    Purge,
}

/// The chips, in the order they are drawn.
pub const CHIPS: [Chip; 11] = [
    Chip::Case,
    Chip::Word,
    Chip::Normal,
    Chip::Extended,
    Chip::Regex,
    Chip::Wrap,
    Chip::Selection,
    Chip::Up,
    Chip::Down,
    Chip::Bookmark,
    Chip::Purge,
];

impl Chip {
    /// The mark drawn on the chip.
    ///
    /// `Word` changes with the mode, which is Notepad++'s own trick: its
    /// "Match whole word only" box becomes ". matches newline" once the pattern
    /// is a regular expression, because a whole word is not a thing a regex
    /// has. Reusing the slot keeps the row from growing a tenth chip.
    pub fn mark(self, mode: SearchMode) -> &'static str {
        match self {
            Chip::Case => "Aa",
            // "\n" rather than a carriage-return arrow: the arrow is missing
            // from the interface font, and a box where a mark should be reads
            // as a broken button. The escape is also what the user types.
            Chip::Word if mode == SearchMode::Regex => "\\n",
            Chip::Word => "W",
            Chip::Normal => "N",
            Chip::Extended => "E",
            Chip::Regex => ".*",
            // "Infinity" rather than a circular arrow for the same reason: it
            // says the search keeps going, and the font has it.
            Chip::Wrap => "\u{221e}",
            Chip::Selection => "\u{25ad}",
            Chip::Up => "\u{2191}",
            Chip::Down => "\u{2193}",
            // Chips carry a mark, so the two Mark-tab settings get a letter
            // each: B for the bookmark, P for the purge.
            Chip::Bookmark => "B",
            Chip::Purge => "P",
        }
    }

    /// Whether the chip is lit.
    pub fn is_on(self, bar: &FindBar) -> bool {
        match self {
            Chip::Case => bar.options.case_sensitive,
            Chip::Word if bar.options.mode == SearchMode::Regex => bar.options.dot_matches_newline,
            Chip::Word => bar.options.whole_word,
            Chip::Normal => bar.options.mode == SearchMode::Normal,
            Chip::Extended => bar.options.mode == SearchMode::Extended,
            Chip::Regex => bar.options.mode == SearchMode::Regex,
            Chip::Wrap => bar.wrap,
            Chip::Selection => bar.in_selection,
            Chip::Up => bar.up,
            Chip::Down => !bar.up,
            Chip::Bookmark => bar.bookmark_lines,
            Chip::Purge => bar.purge_marks,
        }
    }
}

/// A button in the bar's action rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// Walk to the next match in the chosen direction.
    FindNext,
    /// Report how many matches there are, without moving.
    Count,
    /// List every match in the active document.
    FindAll,
    /// List every match in every open document.
    FindAllAll,
    /// Replace the current match and walk on.
    Replace,
    /// Replace every match in the active document.
    ReplaceAll,
    /// Replace every match in every open document.
    ReplaceAllAll,
    /// Highlight every match in the active document.
    MarkAll,
    /// Drop the highlights a mark left.
    ClearMarks,
    /// Close the bar.
    Close,
}

/// The buttons, in the order they are drawn, left to right.
///
/// Notepad++'s own order, "Find Next" through "Close", with the replacement's
/// three buttons in the middle where it puts them.
pub const BUTTONS: [Button; 10] = [
    Button::FindNext,
    Button::Count,
    Button::FindAll,
    Button::FindAllAll,
    Button::Replace,
    Button::ReplaceAll,
    Button::ReplaceAllAll,
    Button::MarkAll,
    Button::ClearMarks,
    Button::Close,
];

impl Button {
    /// The word on the button.
    pub fn label(self) -> &'static str {
        match self {
            Button::FindNext => "Find Next",
            Button::Count => "Count",
            Button::FindAll => "Find All in Current Document",
            Button::FindAllAll => "Find All in All Opened Documents",
            Button::Replace => "Replace",
            Button::ReplaceAll => "Replace All",
            Button::ReplaceAllAll => "Replace All in All Opened Documents",
            Button::MarkAll => "Mark All",
            Button::ClearMarks => "Clear Marks",
            Button::Close => "Close",
        }
    }

    /// Whether the button only makes sense once the replacement row is open.
    pub fn replace_only(self) -> bool {
        matches!(
            self,
            Button::Replace | Button::ReplaceAll | Button::ReplaceAllAll
        )
    }
}

/// One of the three things Notepad++'s swap button can do.
///
/// The dialog has one button and a pull-down to choose between the three; a
/// bar docked along the bottom has room for three small buttons and no room for
/// a menu, and three buttons are one click each rather than two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapChip {
    /// Exchange the two fields.
    Swap,
    /// Copy the pattern into the replacement, leaving the pattern alone.
    FindToReplace,
    /// Copy the replacement into the pattern, leaving the replacement alone.
    ReplaceToFind,
}

/// The swap buttons, in the order they are drawn.
pub const SWAP_CHIPS: [SwapChip; 3] = [
    SwapChip::Swap,
    SwapChip::FindToReplace,
    SwapChip::ReplaceToFind,
];

impl SwapChip {
    /// The mark drawn on the button.
    ///
    /// `<>` and the two arrows are ASCII: the interface font is a monospace
    /// face with no `⇅`, and a box where a mark should be reads as a fault.
    pub fn mark(self) -> &'static str {
        match self {
            SwapChip::Swap => "<>",
            SwapChip::FindToReplace => "->",
            SwapChip::ReplaceToFind => "<-",
        }
    }
}

/// What a click on the bar landed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// A setting chip.
    Chip(Chip),
    /// A button.
    Button(Button),
    /// One of the swap buttons.
    Swap(SwapChip),
    /// One of the two text fields.
    Field(Field),
    /// The arrow that opens a field's history.
    HistoryToggle(Field),
    /// One entry of an open history list, counted from the most recent.
    HistoryEntry(usize),
}

/// A rectangle in window coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

impl Rect {
    /// True if `(px, py)` is inside.
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// A single-line editable string.
pub struct TextField {
    text: String,
    /// Caret position in characters.
    caret: usize,
    /// The selected range, in characters, when one is in force.
    ///
    /// The field has no mouse and no Shift+arrow, so the only way to make one is
    /// `Ctrl+A`: select the whole thing and type over it. It is kept as a range
    /// rather than as a highlight flag because it is the range that decides what
    /// the next keystroke deletes.
    selection: Option<Range<usize>>,
    buffer: Buffer,
    stale: bool,
}

impl TextField {
    pub fn new(metrics: Metrics, family: Family<'static>) -> Self {
        let mut buffer = Buffer::new_empty(metrics);
        buffer.set_size(None, None);
        buffer.set_text("", &Attrs::new().family(family), Shaping::Basic, None);
        Self {
            text: String::new(),
            caret: 0,
            selection: None,
            buffer,
            stale: true,
        }
    }

    /// The current contents.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The caret position, in characters.
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// The selected range, in characters.
    pub fn selection(&self) -> Option<Range<usize>> {
        self.selection.clone()
    }

    /// Selects the whole field, leaving the caret at the end.
    pub fn select_all(&mut self) {
        let len = self.text.chars().count();
        self.caret = len;
        self.selection = if len == 0 { None } else { Some(0..len) };
    }

    /// Drops the selection, leaving the text and the caret alone.
    fn deselect(&mut self) {
        self.selection = None;
    }

    /// Removes the selected characters, if any, and puts the caret where they
    /// were.
    fn take_selection(&mut self) -> bool {
        let Some(range) = self.selection.take() else {
            return false;
        };
        let start = self.byte_at(range.start);
        let end = self.byte_at(range.end);
        self.text.replace_range(start..end, "");
        self.caret = range.start;
        self.stale = true;
        true
    }

    /// Replaces the contents and puts the caret at the end.
    pub fn set_text(&mut self, text: &str) {
        if self.text != text {
            self.text.clear();
            self.text.push_str(text);
            self.caret = self.text.chars().count();
            self.selection = None;
            self.stale = true;
        } else {
            // Same text, but a selection left over would still be armed: a
            // keystroke that is meant to append would wipe the field.
            self.deselect();
        }
    }

    /// Inserts `c` at the caret, replacing the selection if there is one.
    pub fn insert(&mut self, c: char) {
        self.take_selection();
        let byte = self.byte_at(self.caret);
        self.text.insert(byte, c);
        self.caret += 1;
        self.stale = true;
    }

    /// Removes the selection, or the character before the caret.
    pub fn backspace(&mut self) {
        if self.take_selection() {
            return;
        }
        if self.caret == 0 {
            return;
        }
        let byte = self.byte_at(self.caret - 1);
        self.text.remove(byte);
        self.caret -= 1;
        self.stale = true;
    }

    /// Removes the selection, or the character after the caret.
    pub fn delete(&mut self) {
        if self.take_selection() {
            return;
        }
        if self.caret >= self.text.chars().count() {
            return;
        }
        let byte = self.byte_at(self.caret);
        self.text.remove(byte);
        self.stale = true;
    }

    /// Moves the caret one character left.
    pub fn left(&mut self) {
        self.deselect();
        self.caret = self.caret.saturating_sub(1);
    }

    /// Moves the caret one character right.
    pub fn right(&mut self) {
        self.deselect();
        self.caret = (self.caret + 1).min(self.text.chars().count());
    }

    /// Moves the caret to the start.
    pub fn home(&mut self) {
        self.deselect();
        self.caret = 0;
    }

    /// Moves the caret to the end.
    pub fn end(&mut self) {
        self.deselect();
        self.caret = self.text.chars().count();
    }

    /// The byte offset of character `index`.
    fn byte_at(&self, index: usize) -> usize {
        self.text
            .char_indices()
            .nth(index)
            .map_or(self.text.len(), |(b, _)| b)
    }

    /// Rebuilds the shaped buffer if the text changed.
    /// The shaped buffer, for drawing.
    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    /// Reshapes the buffer if the text changed.
    pub fn refresh(&mut self, font_system: &mut FontSystem, family: Family<'static>) {
        if !self.stale {
            return;
        }
        self.buffer.set_text(
            &self.text,
            &Attrs::new().family(family),
            Shaping::Basic,
            None,
        );
        self.buffer.shape_until_scroll(font_system, false);
        self.stale = false;
    }
}

/// The find/replace bar.
pub struct FindBar {
    /// Whether the bar is showing.
    pub open: bool,
    /// Whether the same search is drawn as a floating window instead of a bar.
    ///
    /// The search itself does not change: the dialog draws this struct's state,
    /// so the fields, the options, the matches and the history are the ones the
    /// bar would have used. What changes is the presentation and the space it
    /// takes -- a floating window covers the text for as long as it is up and
    /// reserves no band at the bottom, so every caller that asks the bar for its
    /// height has to get zero rather than a row count.
    pub floating: bool,
    /// Whether the replace row is showing.
    pub replace: bool,
    /// The search pattern.
    pub query: TextField,
    /// The replacement text.
    pub replacement: TextField,
    /// Which field has the caret.
    pub focus: Field,
    /// Whether the search wraps at the ends of the document.
    ///
    /// On by default, because a search that stops dead at the bottom of the
    /// file reads as a search that found nothing.
    pub wrap: bool,
    /// Whether the search is confined to the selection.
    pub in_selection: bool,
    /// Whether the search runs upwards.
    pub up: bool,
    /// Whether a mark also drops a bookmark on its line.
    pub bookmark_lines: bool,
    /// Whether a search drops the previous marks before marking again.
    pub purge_marks: bool,
    /// Patterns searched for before, most recent first.
    pub history: Vec<String>,
    /// Replacements used before, most recent first.
    pub replacement_history: Vec<String>,
    /// Which field's history list is open.
    history_open: Option<Field>,
    /// Which entry of it the pointer or the arrow keys are on.
    history_pick: Option<usize>,
    /// Whether the selection follows the pattern as it is typed.
    incremental: bool,
    /// How to match.
    pub options: SearchOptions,
    /// Every match in the document, in order.
    pub matches: Vec<Range<usize>>,
    /// Index into `matches` of the match the caret is on.
    pub current: Option<usize>,
    /// A compile error, shown in place of the match count.
    pub error: Option<String>,
    compiled: Option<Search>,
    stale: bool,
    family: Family<'static>,
    metrics: Metrics,
    char_width: f32,
    label_find: Buffer,
    label_replace: Buffer,
    /// Width of the widest label plus the gap before the fields, measured from
    /// the shaped text on the first frame.
    ///
    /// A constant wide enough for "Replace:" was not: the label is drawn in the
    /// proportional UI face, so its width depends on the font and on the
    /// language, and a translated label can be half again as long.
    label_w: f32,
    /// The mark on each chip, shaped, and the mark it was last shaped from.
    chips: [Buffer; 11],
    chip_marks: [&'static str; 11],
    /// The mark on each swap button, shaped once.
    swaps: [Buffer; 3],
    /// The little arrow that opens a history list, shaped once.
    history_glyph: Buffer,
    /// The entries of the open history list, shaped.
    history_rows: Vec<Buffer>,
    /// The entry text each row was shaped from.
    history_shaped: Vec<String>,
    /// The word on each button, shaped, and its width once shaped.
    buttons: [Buffer; 10],
    button_w: [f32; 10],
    buttons_shaped: bool,
    count: Buffer,
    count_text: String,
    labels_shaped: bool,
    /// The localization the bar's own words come from.
    catalog: Option<Catalog>,
}

impl FindBar {
    /// Builds a closed bar.
    pub fn new(metrics: Metrics, family: Family<'static>, char_width: f32) -> Self {
        let label = |text: &str| {
            let mut buffer = Buffer::new_empty(metrics);
            buffer.set_size(None, None);
            buffer.set_text(text, &Attrs::new().family(family), Shaping::Basic, None);
            buffer
        };
        Self {
            open: false,
            floating: false,
            replace: false,
            query: TextField::new(metrics, family),
            replacement: TextField::new(metrics, family),
            focus: Field::Query,
            wrap: true,
            in_selection: false,
            up: false,
            bookmark_lines: false,
            purge_marks: false,
            history: Vec::new(),
            replacement_history: Vec::new(),
            history_open: None,
            history_pick: None,
            incremental: false,
            options: SearchOptions::default(),
            matches: Vec::new(),
            current: None,
            error: None,
            compiled: None,
            stale: true,
            family,
            metrics,
            char_width,
            label_find: label("Find:"),
            label_replace: label("Replace:"),
            // Until the labels are shaped: the width the fixed column used to
            // have, which keeps the first layout sane on a machine whose fonts
            // are not there yet.
            label_w: 62.0,
            chips: std::array::from_fn(|i| label(CHIPS[i].mark(SearchMode::Normal))),
            // Empty, so the first frame shapes every chip rather than trusting
            // a mark that was never laid out.
            chip_marks: [""; 11],
            swaps: std::array::from_fn(|i| label(SWAP_CHIPS[i].mark())),
            history_glyph: label("\u{25be}"),
            history_rows: Vec::new(),
            history_shaped: Vec::new(),
            buttons: std::array::from_fn(|i| label(BUTTONS[i].label())),
            button_w: [0.0; 10],
            buttons_shaped: false,
            count: label(""),
            count_text: String::new(),
            labels_shaped: false,
            catalog: None,
        }
    }

    /// Rebuilds the bar at a new display density.
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
        self.label_find.set_metrics(metrics);
        self.label_replace.set_metrics(metrics);
        for chip in &mut self.chips {
            chip.set_metrics(metrics);
        }
        for swap in &mut self.swaps {
            swap.set_metrics(metrics);
        }
        self.history_glyph.set_metrics(metrics);
        for row in &mut self.history_rows {
            row.set_metrics(metrics);
        }
        for button in &mut self.buttons {
            button.set_metrics(metrics);
        }
        self.count.set_metrics(metrics);
    }

    /// Every label this bar can draw, for the coverage test.
    ///
    /// The chips are absent on purpose: `Aa`, `W` and `.*` are marks, not
    /// words, and they are the same marks in every language. The buttons are
    /// here because they carry words.
    pub fn labels() -> Vec<&'static str> {
        let mut labels = vec!["Find:", "Replace:", "no match", "bad regex"];
        for button in BUTTONS {
            labels.push(button.label());
        }
        labels
    }

    /// Swaps in a localization, or clears it with `None`.
    ///
    /// The two field labels are rebuilt here rather than at the next frame: they
    /// are shaped once and left alone, so a language picked while the bar is
    /// open would otherwise change the menus and leave the bar in the old one.
    pub fn set_catalog(&mut self, catalog: Option<Catalog>) {
        self.catalog = catalog;
        self.relabel();
        self.count_text.clear();
    }

    /// Rewrites the labels and the buttons from the catalog.
    fn relabel(&mut self) {
        let (find, replace) = self.labels_text();
        for (buffer, text) in [(&mut self.label_find, find), (&mut self.label_replace, replace)] {
            buffer.set_text(&text, &Attrs::new().family(self.family), Shaping::Basic, None);
        }
        // The buttons are words, so a language picked while the bar is open
        // relabels them too. Their widths are measured again because a
        // translation is usually longer than the English.
        let texts: Vec<String> = BUTTONS
            .iter()
            .map(|button| self.tr(button.label()).to_string())
            .collect();
        for (buffer, text) in self.buttons.iter_mut().zip(texts) {
            buffer.set_text(&text, &Attrs::new().family(self.family), Shaping::Basic, None);
        }
        self.buttons_shaped = false;
        self.labels_shaped = false;
    }

    /// The two field labels, translated.
    fn labels_text(&self) -> (String, String) {
        let tr = |english: &str| match &self.catalog {
            Some(catalog) => catalog.tr(english).to_string(),
            None => english.to_string(),
        };
        (tr("Find:"), tr("Replace:"))
    }

    /// The translation of `english`, or `english` itself.
    fn tr<'a>(&'a self, english: &'a str) -> &'a str {
        match &self.catalog {
            Some(catalog) => catalog.tr(english),
            None => english,
        }
    }

    /// Opens the bar in find mode, keeping any existing query.
    pub fn open_find(&mut self) {
        self.open = true;
        self.replace = false;
        self.incremental = false;
        self.focus = Field::Query;
    }

    /// Opens the bar in replace mode.
    pub fn open_replace(&mut self) {
        self.open = true;
        self.replace = true;
        self.incremental = false;
        self.focus = Field::Query;
    }

    /// Opens the bar in incremental mode.
    ///
    /// Incremental mode differs from find mode in what happens as the pattern is
    /// typed: the selection follows the first match instead of waiting for
    /// Enter. The bar itself is the same, because the toggles matter just as
    /// much while typing -- case sensitivity decides which match is first.
    pub fn open_incremental(&mut self) {
        self.open = true;
        self.replace = false;
        self.incremental = true;
        self.focus = Field::Query;
    }

    /// Whether the selection follows the pattern as it is typed.
    pub fn is_incremental(&self) -> bool {
        self.incremental
    }

    /// Closes the bar.
    ///
    /// `volatile` decides what happens to the highlights. Volatile find -- the
    /// default, and what Notepad++ does -- drops them, because a search you have
    /// finished with should not leave the document speckled. Non-volatile keeps
    /// them, which is what you want when the point of the search was to see
    /// every occurrence at once.
    pub fn close(&mut self, volatile: bool) {
        self.open = false;
        self.incremental = false;
        self.remember();
        self.close_history();
        if volatile {
            self.matches.clear();
            self.current = None;
            self.error = None;
        }
    }

    /// Whether the bar is showing, or closed but still holding highlights.
    ///
    /// A closed bar with retained matches is still doing work: the highlights
    /// have to follow the text as it is edited, so `refresh` keeps running.
    pub fn is_active(&self) -> bool {
        self.open || !self.matches.is_empty()
    }

    /// Marks the pattern or options as changed.
    pub fn invalidate(&mut self) {
        self.stale = true;
    }

    /// The field with the caret.
    fn focused(&mut self) -> &mut TextField {
        match self.focus {
            Field::Query => &mut self.query,
            Field::Replacement => &mut self.replacement,
        }
    }

    /// Inserts `c` into the focused field.
    pub fn insert_char(&mut self, c: char) {
        self.focused().insert(c);
        if self.focus == Field::Query {
            self.stale = true;
        }
    }

    /// Inserts a pasted string at the caret, and reports whether anything went
    /// in.
    ///
    /// Control characters are dropped: a pattern copied from a file arrives
    /// with the newline that ended its line, and the field is one line with no
    /// way to show one. Flattened text can still match; a pattern carrying a
    /// newline never would. A paste holding only white space returns false, so
    /// the caller can leave the matches alone.
    pub fn insert_text(&mut self, text: &str) -> bool {
        let mut inserted = false;
        for c in text.chars().filter(|c| !c.is_control()) {
            self.insert_char(c);
            inserted = true;
        }
        inserted
    }

    /// Deletes before the caret in the focused field.
    pub fn backspace(&mut self) {
        self.focused().backspace();
        if self.focus == Field::Query {
            self.stale = true;
        }
    }

    /// Deletes after the caret in the focused field.
    pub fn delete(&mut self) {
        self.focused().delete();
        if self.focus == Field::Query {
            self.stale = true;
        }
    }

    /// Moves the caret left in the focused field.
    pub fn left(&mut self) {
        self.focused().left();
    }

    /// Moves the caret right in the focused field.
    pub fn right(&mut self) {
        self.focused().right();
    }

    /// Moves the caret to the start of the focused field.
    pub fn home(&mut self) {
        self.focused().home();
    }

    /// Moves the caret to the end of the focused field.
    pub fn end(&mut self) {
        self.focused().end();
    }

    /// Moves the caret between the two fields.
    pub fn toggle_focus(&mut self) {
        if !self.replace {
            return;
        }
        // The selection belongs to the field it was made in: leaving it armed in
        // the background would let the next keystroke wipe text the user cannot
        // see.
        self.query.deselect();
        self.replacement.deselect();
        self.focus = match self.focus {
            Field::Query => Field::Replacement,
            Field::Replacement => Field::Query,
        };
    }

    /// Selects the whole of the focused field, so the next keystroke replaces it.
    pub fn select_all(&mut self) {
        self.focused().select_all();
    }

    /// Recompiles the pattern and recomputes matches if anything changed.
    pub fn refresh(&mut self, doc: &Document) {
        self.refresh_within(doc, None);
    }

    /// Recomputes the matches, keeping only those inside `within`.
    ///
    /// A match has to lie entirely inside the range to count. One that starts
    /// before the selection and ends inside it is not "in the selection"
    /// anywhere the user looked, and replacing it would write outside the text
    /// they meant to touch.
    pub fn refresh_within(&mut self, doc: &Document, within: Option<Range<usize>>) {
        if !self.is_active() {
            return;
        }
        if self.stale {
            self.stale = false;
            match Search::new(self.query.text(), self.options) {
                Ok(search) => {
                    self.error = None;
                    self.compiled = Some(search);
                }
                Err(err) => {
                    self.error = Some(err.to_string());
                    self.compiled = None;
                }
            }
        }
        let Some(search) = &self.compiled else {
            self.matches.clear();
            self.current = None;
            return;
        };
        self.matches = search.find_all(doc);
        if let Some(range) = within {
            self.matches
                .retain(|m| m.start >= range.start && m.end <= range.end);
        }
        if self.matches.is_empty() {
            self.current = None;
        } else if self.current.is_none_or(|i| i >= self.matches.len()) {
            self.current = Some(0);
        }
    }

    /// The match the caret is on.
    pub fn current_match(&self) -> Option<Range<usize>> {
        self.current.and_then(|i| self.matches.get(i).cloned())
    }

    /// Advances to the next match at or after `from`, wrapping if the bar says
    /// to.
    pub fn next_from(&mut self, from: usize) -> Option<Range<usize>> {
        if self.matches.is_empty() {
            return None;
        }
        let index = match self.matches.iter().position(|m| m.start >= from) {
            Some(index) => index,
            // Past the last match: start again at the top, or stay put. Staying
            // put is what a search with wrap off has to do -- jumping to the
            // first match would be the wrap it was told not to do.
            None if self.wrap => 0,
            None => return None,
        };
        self.current = Some(index);
        self.matches.get(index).cloned()
    }

    /// Steps back to the previous match before `from`, wrapping if the bar says
    /// to.
    pub fn prev_from(&mut self, from: usize) -> Option<Range<usize>> {
        if self.matches.is_empty() {
            return None;
        }
        let index = match self.matches.iter().rposition(|m| m.end <= from) {
            Some(index) => index,
            None if self.wrap => self.matches.len() - 1,
            None => return None,
        };
        self.current = Some(index);
        self.matches.get(index).cloned()
    }

    /// Flips one setting, or chooses one of the mutually exclusive ones.
    pub fn activate(&mut self, chip: Chip) {
        match chip {
            Chip::Case => self.options.case_sensitive = !self.options.case_sensitive,
            Chip::Word if self.options.mode == SearchMode::Regex => {
                self.options.dot_matches_newline = !self.options.dot_matches_newline;
            }
            Chip::Word => self.options.whole_word = !self.options.whole_word,
            Chip::Normal => self.options.mode = SearchMode::Normal,
            Chip::Extended => self.options.mode = SearchMode::Extended,
            Chip::Regex => self.options.mode = SearchMode::Regex,
            Chip::Wrap => self.wrap = !self.wrap,
            Chip::Selection => self.in_selection = !self.in_selection,
            Chip::Up => self.up = true,
            Chip::Down => self.up = false,
            Chip::Bookmark => self.bookmark_lines = !self.bookmark_lines,
            Chip::Purge => self.purge_marks = !self.purge_marks,
        }
        self.stale = true;
    }

    /// Puts the caret in one of the two fields.
    ///
    /// The other field's selection is dropped: it belongs to the field it was
    /// made in, and leaving it armed in the background would let the next
    /// keystroke wipe text nobody can see.
    pub fn focus_field(&mut self, field: Field) {
        if field == Field::Replacement && !self.replace {
            return;
        }
        self.query.deselect();
        self.replacement.deselect();
        self.focus = field;
    }

    /// The compiled search, if the pattern is valid.
    pub fn search(&self) -> Option<&Search> {
        self.compiled.as_ref()
    }

    /// Puts remembered history in place, from the file the app keeps it in.
    pub fn set_history(&mut self, patterns: Vec<String>, replacements: Vec<String>) {
        self.history = patterns;
        self.history.truncate(MAX_HISTORY);
        self.replacement_history = replacements;
        self.replacement_history.truncate(MAX_HISTORY);
    }

    /// Files both fields' text away at the front of their history.
    ///
    /// Called when the bar closes and whenever a search runs, so a pattern used
    /// twice in one session is remembered whether or not the bar was closed.
    /// Empty text is not a search and is not remembered.
    pub fn remember(&mut self) {
        remember_into(&mut self.history, self.query.text());
        remember_into(&mut self.replacement_history, self.replacement.text());
    }

    /// The entries a field's history holds.
    pub fn history_entries(&self, field: Field) -> &[String] {
        match field {
            Field::Query => &self.history,
            Field::Replacement => &self.replacement_history,
        }
    }

    /// Whether the search is drawn as a floating window.
    pub fn floating(&self) -> bool {
        self.floating
    }

    /// Draws the search as a floating window, or as the bar.
    ///
    /// The search state is untouched, so switching mid-search keeps the pattern,
    /// the options and the matches: what moves is the presentation.
    pub fn set_floating(&mut self, on: bool) {
        self.floating = on;
    }

    /// A button's word in the current language.
    ///
    /// The floating window draws the same buttons from its own shaped buffers,
    /// and a second copy of the translation would be a second thing to keep in
    /// step -- so it asks here.
    pub fn button_text(&self, button: Button) -> &str {
        self.tr(button.label())
    }

    /// Which field's history list is open, if any.
    pub fn history_open(&self) -> Option<Field> {
        self.history_open
    }

    /// Which entry the pointer or the arrow keys are on.
    pub fn history_pick(&self) -> Option<usize> {
        self.history_pick
    }

    /// Opens a field's history list, or closes it when it is already open.
    pub fn toggle_history(&mut self, field: Field) {
        self.history_open = if self.history_open == Some(field) {
            None
        } else {
            Some(field)
        };
        self.history_pick = None;
    }

    /// Closes the history list.
    pub fn close_history(&mut self) {
        self.history_open = None;
        self.history_pick = None;
    }

    /// Puts the text of history entry `index` into its field.
    pub fn pick_history(&mut self, field: Field, index: usize) {
        let Some(text) = self.history_entries(field).get(index).cloned() else {
            return;
        };
        self.set_field_text(field, &text);
        self.close_history();
    }

    /// Moves the highlight through the open list, wrapping at both ends.
    pub fn move_history(&mut self, delta: isize) {
        let Some(field) = self.history_open else {
            return;
        };
        let count = self.history_entries(field).len().min(VISIBLE_HISTORY);
        if count == 0 {
            return;
        }
        let next = match self.history_pick {
            // The first arrow press lands on the nearest entry rather than on
            // the one after it: the list opens with nothing picked, and Down
            // has to mean "the most recent one".
            None if delta > 0 => 0,
            None => count as isize - 1,
            Some(current) => (current as isize + delta).rem_euclid(count as isize),
        };
        self.history_pick = Some(next as usize);
    }

    /// Replaces the text of one field, wholesale.
    pub fn set_field_text(&mut self, field: Field, text: &str) {
        let target = match field {
            Field::Query => &mut self.query,
            Field::Replacement => &mut self.replacement,
        };
        target.set_text(text);
        if field == Field::Query {
            self.stale = true;
        }
    }

    /// Runs one of the swap buttons.
    ///
    /// The pattern is marked stale by `set_field_text`, so the matches follow
    /// the new pattern rather than the one the bar was holding.
    pub fn swap(&mut self, chip: SwapChip) {
        match chip {
            SwapChip::Swap => {
                let query = self.query.text().to_string();
                let replacement = self.replacement.text().to_string();
                self.set_field_text(Field::Query, &replacement);
                self.set_field_text(Field::Replacement, &query);
            }
            SwapChip::FindToReplace => {
                let query = self.query.text().to_string();
                self.set_field_text(Field::Replacement, &query);
            }
            SwapChip::ReplaceToFind => {
                let replacement = self.replacement.text().to_string();
                self.set_field_text(Field::Query, &replacement);
            }
        }
    }

    /// The height the bar occupies, in pixels.
    ///
    /// The two fields, the chips, and however many rows the buttons need at this
    /// width: a narrow window wraps them rather than clipping the last one.
    pub fn height(&self, width: f32, line_height: f32, padding: f32) -> f32 {
        // A floating window reserves no band: the text runs under it.
        if !self.open || self.floating {
            return 0.0;
        }
        let fields = if self.replace { 2.0 } else { 1.0 };
        let buttons = self.button_rows(width, padding).len().max(1) as f32;
        (fields + 1.0 + buttons) * line_height + 2.0 * padding
    }

    /// The buttons that fit on each row, as indices into [`BUTTONS`].
    ///
    /// Greedy, left to right, in the order they are drawn. A button wider than
    /// the whole row is squeezed to the row rather than dropped: it is still the
    /// button that does the thing, and a missing button is a missing feature.
    fn button_rows(&self, width: f32, padding: f32) -> Vec<Vec<usize>> {
        let left = padding + self.label_w;
        let available = ((width - padding) - left).max(80.0);
        let mut rows: Vec<Vec<usize>> = Vec::new();
        let mut current: Vec<usize> = Vec::new();
        let mut used = 0.0;
        for (index, button) in BUTTONS.iter().enumerate() {
            if button.replace_only() && !self.replace {
                continue;
            }
            let w = self.button_w[index].max(44.0).min(available);
            let extra = if current.is_empty() {
                w
            } else {
                CHIP_GAP + w
            };
            if !current.is_empty() && used + extra > available {
                rows.push(std::mem::take(&mut current));
                used = w;
            } else {
                used += extra;
            }
            current.push(index);
        }
        if !current.is_empty() {
            rows.push(current);
        }
        // A greedy fill can leave one button alone on the last row, under a row
        // that is packed to the edge. Moving the last button of the row above
        // down evens the two out, which is the difference between a bar that
        // looks laid out and one that looks wrapped.
        while rows.len() > 1 {
            let last = rows[rows.len() - 1].len();
            let above = rows[rows.len() - 2].len();
            if last != 1 || above <= 2 {
                break;
            }
            let count = rows.len();
            let moved = rows[count - 2].pop().expect("a row of buttons");
            rows[count - 1].insert(0, moved);
        }
        rows
    }

    /// Rebuilds the shaped buffers.
    pub fn prepare(&mut self, font_system: &mut FontSystem) {
        if !self.open {
            return;
        }
        if !self.labels_shaped {
            // The static labels are built without a font system, so they are
            // shaped on the first frame rather than in the constructor.
            self.label_find.shape_until_scroll(font_system, false);
            self.label_replace.shape_until_scroll(font_system, false);
            // The column is as wide as the widest label, measured rather than
            // guessed: "Replace:" in the proportional UI face is wider than the
            // fixed sixty-two pixels it used to be given, and the field text
            // then started on top of it.
            self.label_w = widest_line(&self.label_find)
                .max(widest_line(&self.label_replace))
                + LABEL_GAP;
            // The swap marks and the history arrow never change, so they are
            // shaped with the labels rather than on every frame.
            self.history_glyph.shape_until_scroll(font_system, false);
            for swap in self.swaps.iter_mut() {
                swap.shape_until_scroll(font_system, false);
            }
            self.labels_shaped = true;
        }

        // A chip's mark changes with the mode, so it is shaped from the mark it
        // is showing rather than once for good.
        for (index, chip) in CHIPS.iter().enumerate() {
            let mark = chip.mark(self.options.mode);
            if mark != self.chip_marks[index] {
                self.chips[index].set_text(
                    mark,
                    &Attrs::new().family(self.family),
                    Shaping::Basic,
                    None,
                );
                self.chips[index].shape_until_scroll(font_system, false);
                self.chip_marks[index] = mark;
            }
        }
        if !self.buttons_shaped {
            // Measured rather than assumed: the buttons are words in a
            // proportional face, and a translation is usually longer than the
            // English it replaces.
            for (index, button) in self.buttons.iter_mut().enumerate() {
                button.shape_until_scroll(font_system, false);
                self.button_w[index] = widest_line(button) + 2.0 * BUTTON_PAD;
            }
            self.buttons_shaped = true;
        }

        self.query.refresh(font_system, self.family);
        self.replacement.refresh(font_system, self.family);

        // The open history list, reshaped only when its entries changed: it is
        // drawn over the document, and reshaping it on every frame would shape
        // text nobody can see change.
        let entries: Vec<String> = match self.history_open {
            Some(field) => self
                .history_entries(field)
                .iter()
                .take(VISIBLE_HISTORY)
                .cloned()
                .collect(),
            None => Vec::new(),
        };
        if entries != self.history_shaped {
            self.history_rows.resize_with(entries.len(), || {
                Buffer::new_empty(self.metrics)
            });
            for (buffer, text) in self.history_rows.iter_mut().zip(&entries) {
                buffer.set_size(None, None);
                buffer.set_text(text, &Attrs::new().family(self.family), Shaping::Basic, None);
                buffer.shape_until_scroll(font_system, false);
            }
            self.history_shaped = entries;
        }

        let text = self.count_label();
        if text != self.count_text {
            self.count.set_text(
                &text,
                &Attrs::new().family(self.family),
                Shaping::Basic,
                None,
            );
            self.count.shape_until_scroll(font_system, false);
            self.count_text = text;
        }
    }

    /// The text shown in the match counter.
    fn count_label(&self) -> String {
        if let Some(err) = &self.error {
            return if err.len() > 12 {
                self.tr("bad regex").to_string()
            } else {
                err.clone()
            };
        }
        if self.query.text().is_empty() {
            return String::new();
        }
        if self.matches.is_empty() {
            return self.tr("no match").to_string();
        }
        let n = self.current.map_or(0, |i| i + 1);
        format!("{n}/{}", self.matches.len())
    }

    /// The clickable rectangles, in window coordinates.
    pub fn layout(&self, width: f32, top: f32, line_height: f32, padding: f32) -> BarLayout {
        let row = line_height;
        let find_y = top + padding;
        let replace_y = find_y + row;
        let chips_y = find_y + if self.replace { 2.0 } else { 1.0 } * row;
        let buttons_y = chips_y + row;

        let count_w = 72.0;
        let right = width - padding;
        let label_x = padding;
        let field_x = padding + self.label_w;

        // Each row ends at a little history arrow; the pattern's row leaves the
        // match counter beyond it, the replacement's row the three swap buttons.
        let swap_w = 3.0 * CHIP_W + 2.0 * 4.0;
        let count_x = right - count_w;
        let query_right = count_x - CHIP_GAP - TOGGLE_W - CHIP_GAP;
        let replace_right = right - swap_w - CHIP_GAP - TOGGLE_W - CHIP_GAP;
        let query_field = Rect {
            x: field_x,
            y: find_y,
            w: (query_right - field_x).max(40.0),
            h: row,
        };
        let replace_field = Rect {
            x: field_x,
            y: replace_y,
            w: (replace_right - field_x).max(40.0),
            h: row,
        };

        // The arrows sit at the right edge of their own row's field, so each
        // one is beside the text it drops down.
        let toggle = |rect: Rect| Rect {
            x: rect.x + rect.w + CHIP_GAP,
            y: rect.y + 2.0,
            w: TOGGLE_W,
            h: row - 4.0,
        };
        let history_toggle = [toggle(query_field), toggle(replace_field)];

        let mut swaps = [Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        }; 3];
        let mut x = right - swap_w;
        for rect in swaps.iter_mut() {
            *rect = Rect {
                x,
                y: replace_y + 2.0,
                w: CHIP_W,
                h: row - 4.0,
            };
            x += CHIP_W + 4.0;
        }

        // The buttons take rows of their own under the chips, each row packed
        // right to left so Close ends at the window's edge. A button that
        // shifted when a translation got longer would be a button people click
        // wrong.
        let mut buttons = vec![
            Rect {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            };
            BUTTONS.len()
        ];
        let mut y = buttons_y + 2.0;
        for indices in self.button_rows(width, padding) {
            let mut widths = Vec::with_capacity(indices.len());
            let mut total = 0.0;
            for &index in &indices {
                let w = self.button_w[index].max(44.0);
                total += w;
                widths.push(w);
            }
            total += CHIP_GAP * (indices.len().saturating_sub(1)) as f32;
            let mut x = (right - total).max(label_x);
            for (&index, w) in indices.iter().zip(widths) {
                buttons[index] = Rect {
                    x,
                    y,
                    w,
                    h: row - 4.0,
                };
                x += w + CHIP_GAP;
            }
            y += row;
        }

        // The chips start where the fields do, so the settings read as a column
        // under the pattern rather than floating in the middle of the bar.
        let mut chips = [Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        }; 11];
        let mut x = field_x;
        for rect in chips.iter_mut() {
            *rect = Rect {
                x,
                y: chips_y + 2.0,
                w: CHIP_W,
                h: row - 4.0,
            };
            x += CHIP_W + 4.0;
        }

        // The open history list, drawn upwards from its field so the most recent
        // entry is the one nearest the text it would fill in.
        let mut history = Vec::new();
        let mut dropdown = None;
        if let Some(field) = self.history_open {
            let anchor = match field {
                Field::Query => query_field,
                Field::Replacement => replace_field,
            };
            let count = self.history_entries(field).len().min(VISIBLE_HISTORY);
            let w = anchor.w.clamp(160.0, 460.0);
            for index in 0..count {
                history.push(Rect {
                    x: anchor.x,
                    y: anchor.y - (index as f32 + 1.0) * row,
                    w,
                    h: row,
                });
            }
            if count > 0 {
                dropdown = Some(Rect {
                    x: anchor.x,
                    y: anchor.y - count as f32 * row,
                    w,
                    h: count as f32 * row,
                });
            }
        }

        BarLayout {
            height: self.height(width, line_height, padding),
            find_y,
            replace_y,
            options_y: chips_y,
            buttons_y,
            label_x,
            field_x,
            field_w: query_field.w,
            field_query: query_field,
            field_replace: replace_field,
            chips,
            buttons,
            swaps,
            history_toggle,
            history,
            dropdown,
            count_x,
            count_w,
            row,
        }
    }

    /// What a click at `(x, y)` lands on, if anything.
    ///
    /// The geometry comes from `layout`, which is also what draws the bar, so a
    /// chip cannot end up clickable somewhere it is not.
    pub fn hit_at(
        &self,
        x: f32,
        y: f32,
        width: f32,
        top: f32,
        line_height: f32,
        padding: f32,
    ) -> Option<Hit> {
        // Floating, the bar is not on screen and the floating window answers
        // clicks instead.
        if !self.open || self.floating {
            return None;
        }
        let layout = self.layout(width, top, line_height, padding);
        // The open list is over the document, so it is tested first: a click on
        // an entry must not also move the caret in the text underneath.
        for (index, rect) in layout.history.iter().enumerate() {
            if rect.contains(x, y) {
                return Some(Hit::HistoryEntry(index));
            }
        }
        for (index, rect) in layout.chips.iter().enumerate() {
            if rect.contains(x, y) {
                return Some(Hit::Chip(CHIPS[index]));
            }
        }
        for (index, rect) in layout.buttons.iter().enumerate() {
            if rect.contains(x, y) {
                return Some(Hit::Button(BUTTONS[index]));
            }
        }
        if self.replace {
            for (index, rect) in layout.swaps.iter().enumerate() {
                if rect.contains(x, y) {
                    return Some(Hit::Swap(SWAP_CHIPS[index]));
                }
            }
        }
        for (index, rect) in layout.history_toggle.iter().enumerate() {
            if rect.contains(x, y) {
                let field = if index == 0 {
                    Field::Query
                } else {
                    Field::Replacement
                };
                if field == Field::Replacement && !self.replace {
                    continue;
                }
                return Some(Hit::HistoryToggle(field));
            }
        }
        if layout.field_query.contains(x, y) {
            return Some(Hit::Field(Field::Query));
        }
        if self.replace && layout.field_replace.contains(x, y) {
            return Some(Hit::Field(Field::Replacement));
        }
        None
    }

    /// Queues the bar's background and toggle rectangles.
    pub fn push_quads(
        &self,
        quads: &mut QuadRenderer,
        width: f32,
        top: f32,
        line_height: f32,
        padding: f32,
        theme: &Theme,
    ) {
        if !self.open || self.floating {
            return;
        }
        let layout = self.layout(width, top, line_height, padding);
        quads.push(
            0.0,
            top,
            width,
            layout.height,
            theme.bar_background.to_linear(),
        );
        quads.push(
            0.0,
            top,
            width,
            1.0,
            theme.bar_border.to_linear(),
        );

        for (index, rect) in layout.chips.iter().enumerate() {
            if CHIPS[index].is_on(self) {
                quads.push(rect.x, rect.y, rect.w, rect.h, theme.toggle_on.to_linear());
            }
        }
        // A button is a raised rectangle with its word inside it, drawn even
        // when it is not hovered: a word floating on the bar does not read as
        // something to press.
        let raised = mix(theme.bar_background, theme.foreground, 0.12).to_linear();
        for rect in layout
            .buttons
            .iter()
            .chain(layout.swaps.iter())
            .chain(layout.history_toggle.iter())
        {
            quads.push(rect.x, rect.y, rect.w, rect.h, raised);
            quads.push(rect.x, rect.y, rect.w, 1.0, theme.bar_border.to_linear());
        }

        // The open history list, above the bar and over the document. It is
        // drawn last so that it covers whatever the bar has already queued.
        if let Some(rect) = layout.dropdown {
            quads.push(rect.x, rect.y, rect.w, rect.h, theme.menu_background.to_linear());
            quads.push(rect.x, rect.y, rect.w, 1.0, theme.bar_border.to_linear());
            quads.push(
                rect.x,
                rect.y + rect.h - 1.0,
                rect.w,
                1.0,
                theme.bar_border.to_linear(),
            );
            quads.push(rect.x, rect.y, 1.0, rect.h, theme.bar_border.to_linear());
            quads.push(
                rect.x + rect.w - 1.0,
                rect.y,
                1.0,
                rect.h,
                theme.bar_border.to_linear(),
            );
        }
        if let Some(pick) = self.history_pick {
            if let Some(rect) = layout.history.get(pick) {
                quads.push(rect.x, rect.y, rect.w, rect.h, theme.selection.to_linear());
            }
        }

        // The caret in the focused field, over the selection it is about to
        // replace: a select-all that drew nothing would look like the field had
        // simply been ignored.
        let (field, rect) = match self.focus {
            Field::Query => (&self.query, layout.field_query),
            Field::Replacement => (&self.replacement, layout.field_replace),
        };
        if let Some(range) = field.selection() {
            quads.push(
                rect.x + range.start as f32 * self.char_width,
                rect.y + 2.0,
                (range.end - range.start) as f32 * self.char_width,
                layout.row - 4.0,
                theme.selection.to_linear(),
            );
        }
        let caret_x = rect.x + field.caret() as f32 * self.char_width;
        quads.push(
            caret_x,
            rect.y + 2.0,
            1.5,
            layout.row - 4.0,
            theme.caret.to_linear(),
        );
    }

    /// The text areas to hand to glyphon.
    pub fn text_areas<'a>(
        &'a self,
        width: f32,
        top: f32,
        line_height: f32,
        padding: f32,
        theme: &Theme,
    ) -> Vec<TextArea<'a>> {
        if !self.open || self.floating {
            return Vec::new();
        }
        let layout = self.layout(width, top, line_height, padding);
        // The open history list reaches above the bar, so the bounds have to
        // start where the list does or its top rows would be clipped away.
        let list_top = layout
            .dropdown
            .map_or(top, |rect| top.min(rect.y));
        let bounds = TextBounds {
            left: 0,
            top: list_top as i32,
            right: width as i32,
            bottom: (top + layout.height) as i32,
        };
        let mut areas = Vec::with_capacity(16);

        areas.push(TextArea {
            buffer: &self.label_find,
            left: layout.label_x,
            top: layout.find_y,
            scale: 1.0,
            bounds,
            default_color: theme.bar_label.to_cosmic(),
            custom_glyphs: &[],
        });
        if self.replace {
            areas.push(TextArea {
                buffer: &self.label_replace,
                left: layout.label_x,
                top: layout.replace_y,
                scale: 1.0,
                bounds,
                default_color: theme.bar_label.to_cosmic(),
                custom_glyphs: &[],
            });
        }
        for (index, rect) in layout.chips.iter().enumerate() {
            areas.push(TextArea {
                buffer: &self.chips[index],
                left: rect.x + 4.0,
                // The row's own top, not the chip's: the chip rectangle is
                // inset so the lit background does not touch the row above, and
                // text drawn from its edge would sit a couple of pixels low.
                top: layout.options_y,
                scale: 1.0,
                bounds,
                default_color: toggle_color(CHIPS[index].is_on(self), theme).to_cosmic(),
                custom_glyphs: &[],
            });
        }
        for (index, rect) in layout.buttons.iter().enumerate() {
            // A button that does not fit in this row at this width was never
            // placed, and drawing its word at zero would paste it in the corner.
            if rect.w == 0.0 {
                continue;
            }
            areas.push(TextArea {
                buffer: &self.buttons[index],
                left: rect.x + BUTTON_PAD,
                top: rect.y - 2.0,
                scale: 1.0,
                bounds,
                default_color: theme.foreground.to_cosmic(),
                custom_glyphs: &[],
            });
        }
        if self.replace {
            for (index, rect) in layout.swaps.iter().enumerate() {
                areas.push(TextArea {
                    buffer: &self.swaps[index],
                    left: rect.x + 3.0,
                    top: rect.y - 2.0,
                    scale: 1.0,
                    bounds,
                    default_color: theme.foreground.to_cosmic(),
                    custom_glyphs: &[],
                });
            }
        }
        for (index, rect) in layout.history_toggle.iter().enumerate() {
            if index == 1 && !self.replace {
                continue;
            }
            areas.push(TextArea {
                buffer: &self.history_glyph,
                left: rect.x + 5.0,
                top: rect.y - 2.0,
                scale: 1.0,
                bounds,
                default_color: theme.bar_label.to_cosmic(),
                custom_glyphs: &[],
            });
        }
        for (index, rect) in layout.history.iter().enumerate() {
            let Some(buffer) = self.history_rows.get(index) else {
                continue;
            };
            areas.push(TextArea {
                buffer,
                left: rect.x + 4.0,
                top: rect.y,
                scale: 1.0,
                bounds,
                default_color: if self.history_pick == Some(index) {
                    theme.toggle_on_text.to_cosmic()
                } else {
                    theme.foreground.to_cosmic()
                },
                custom_glyphs: &[],
            });
        }
        areas.push(TextArea {
            buffer: &self.count,
            left: layout.count_x,
            top: layout.find_y,
            scale: 1.0,
            bounds,
            default_color: theme.bar_label.to_cosmic(),
            custom_glyphs: &[],
        });

        areas.push(TextArea {
            buffer: &self.query.buffer,
            left: layout.field_x,
            top: layout.find_y,
            scale: 1.0,
            bounds,
            default_color: theme.foreground.to_cosmic(),
            custom_glyphs: &[],
        });
        if self.replace {
            areas.push(TextArea {
                buffer: &self.replacement.buffer,
                left: layout.field_x,
                top: layout.replace_y,
                scale: 1.0,
                bounds,
                default_color: theme.foreground.to_cosmic(),
                custom_glyphs: &[],
            });
        }
        areas
    }
}

/// Where everything in the bar sits.
pub struct BarLayout {
    /// Total height of the bar.
    pub height: f32,
    /// Top of the find row.
    pub find_y: f32,
    /// Top of the replace row.
    pub replace_y: f32,
    /// Top of the chips row.
    pub options_y: f32,
    /// Top of the first button row.
    pub buttons_y: f32,
    /// Left edge of the labels.
    pub label_x: f32,
    /// Left edge of the text fields.
    pub field_x: f32,
    /// Width of the text fields.
    pub field_w: f32,
    /// The pattern field.
    pub field_query: Rect,
    /// The replacement field, on the row a click would find it.
    pub field_replace: Rect,
    /// The eleven setting chips.
    pub chips: [Rect; 11],
    /// The buttons, one rectangle each, in [`BUTTONS`] order.
    pub buttons: Vec<Rect>,
    /// The three swap buttons, on the replacement row.
    pub swaps: [Rect; 3],
    /// The two history arrows, pattern first.
    pub history_toggle: [Rect; 2],
    /// The rows of the open history list, most recent first and each above the
    /// one before it.
    pub history: Vec<Rect>,
    /// The background of the open history list.
    pub dropdown: Option<Rect>,
    /// Left edge of the match counter.
    pub count_x: f32,
    /// Width of the match counter.
    pub count_w: f32,
    /// Height of one row.
    pub row: f32,
}

/// Puts `text` at the front of a history list, once.
///
/// The list is most-recent-first and holds each pattern once, so using the same
/// pattern twice does not fill it with a run of one entry.
fn remember_into(list: &mut Vec<String>, text: &str) {
    if text.is_empty() {
        return;
    }
    list.retain(|entry| entry != text);
    list.insert(0, text.to_string());
    list.truncate(MAX_HISTORY);
}

/// The widest line of a shaped buffer, in pixels.
fn widest_line(buffer: &Buffer) -> f32 {
    buffer.layout_runs().map(|run| run.line_w).fold(0.0, f32::max)
}

/// Whether a keypress with these modifiers is text for the field rather than a
/// command.
///
/// A chord the bar does not handle itself -- `Ctrl+H` for replace, `Ctrl+F` to
/// reopen the bar -- has to reach the keymap. It arrives carrying text: on
/// this platform Ctrl and a letter still produce the letter, so a handler that
/// trusts the text handed to it types into the field instead of running the
/// command. Ctrl and Alt together are the exception, because that is how AltGr
/// arrives, and AltGr is how a good part of the world types a `@` or a `€`.
pub fn types_text(ctrl: bool, alt: bool) -> bool {
    ctrl == alt
}

/// The colour a toggle label is drawn in.
pub fn toggle_color(on: bool, theme: &Theme) -> Rgb {
    if on {
        theme.toggle_on_text
    } else {
        theme.bar_label
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar() -> FindBar {
        FindBar::new(Metrics::new(14.0, 20.0), Family::Monospace, 8.0)
    }

    fn doc(text: &str) -> Document {
        let mut doc = Document::new();
        doc.set_text(text);
        doc
    }

    #[test]
    fn incremental_mode_is_off_until_asked_for() {
        let mut bar = bar();
        bar.open_find();
        assert!(!bar.is_incremental());
        bar.open_incremental();
        assert!(bar.is_incremental());
    }

    #[test]
    fn opening_find_leaves_incremental_mode() {
        let mut bar = bar();
        bar.open_incremental();
        bar.open_find();
        assert!(!bar.is_incremental());
    }

    #[test]
    fn opening_replace_leaves_incremental_mode() {
        let mut bar = bar();
        bar.open_incremental();
        bar.open_replace();
        assert!(!bar.is_incremental());
        assert!(bar.replace);
    }

    #[test]
    fn closing_always_leaves_incremental_mode() {
        let mut bar = bar();
        bar.open_incremental();
        bar.close(true);
        assert!(!bar.is_incremental());
    }

    #[test]
    fn closing_volatile_drops_the_highlights() {
        let mut bar = bar();
        bar.open_find();
        bar.query.set_text("fn");
        bar.refresh(&doc("fn a\nfn b\n"));
        assert_eq!(bar.matches.len(), 2);
        bar.close(true);
        assert!(bar.matches.is_empty());
        assert_eq!(bar.current, None);
    }

    #[test]
    fn closing_non_volatile_keeps_the_highlights() {
        let mut bar = bar();
        bar.open_find();
        bar.query.set_text("fn");
        bar.refresh(&doc("fn a\nfn b\n"));
        bar.close(false);
        assert_eq!(bar.matches.len(), 2);
        assert!(!bar.open);
    }

    #[test]
    fn a_closed_bar_with_highlights_is_still_active() {
        let mut bar = bar();
        bar.open_find();
        bar.query.set_text("fn");
        bar.refresh(&doc("fn a\n"));
        bar.close(false);
        assert!(bar.is_active());
    }

    #[test]
    fn a_closed_bar_without_highlights_is_not_active() {
        let mut bar = bar();
        bar.open_find();
        bar.query.set_text("fn");
        bar.refresh(&doc("fn a\n"));
        bar.close(true);
        assert!(!bar.is_active());
    }

    #[test]
    fn a_retained_search_follows_the_text() {
        let mut bar = bar();
        bar.open_find();
        bar.query.set_text("fn");
        bar.refresh(&doc("fn a\n"));
        bar.close(false);
        assert_eq!(bar.matches.len(), 1);
        // The document grows while the bar is closed; the highlights have to
        // keep up, which is why `refresh` runs on a closed but active bar.
        bar.refresh(&doc("fn a\nfn b\n"));
        assert_eq!(bar.matches.len(), 2);
    }

    #[test]
    fn a_closed_inactive_bar_does_no_work() {
        let mut bar = bar();
        bar.open_find();
        bar.query.set_text("fn");
        bar.refresh(&doc("fn a\n"));
        bar.close(true);
        // Nothing to recompute, so the match list stays empty even though the
        // document has matches.
        bar.refresh(&doc("fn a\nfn b\n"));
        assert!(bar.matches.is_empty());
    }

    #[test]
    fn a_paste_fills_the_query_and_searches_for_it() {
        let mut bar = bar();
        bar.open_find();
        assert!(bar.insert_text("fn main"));
        assert_eq!(bar.query.text(), "fn main");
        assert_eq!(bar.query.caret(), 7);
        bar.refresh(&doc("fn main() {}\nfn main_loop() {}\n"));
        assert_eq!(bar.matches.len(), 2);
    }

    #[test]
    fn a_paste_drops_the_control_characters() {
        let mut bar = bar();
        bar.open_find();
        // What a copied line looks like once the newline and the indent tab
        // come along for the ride.
        assert!(bar.insert_text("\talpha\n"));
        assert_eq!(bar.query.text(), "alpha");
    }

    #[test]
    fn a_paste_at_the_caret_keeps_what_is_already_typed() {
        let mut bar = bar();
        bar.open_find();
        bar.query.set_text("main");
        bar.query.home();
        bar.insert_text("fn ");
        assert_eq!(bar.query.text(), "fn main");
        assert_eq!(bar.query.caret(), 3);
    }

    #[test]
    fn a_paste_goes_into_the_replacement_field_when_it_has_the_caret() {
        let mut bar = bar();
        bar.open_replace();
        bar.toggle_focus();
        assert!(bar.insert_text("bravo"));
        assert_eq!(bar.replacement.text(), "bravo");
        assert_eq!(bar.query.text(), "");
    }

    #[test]
    fn an_empty_paste_reports_nothing_inserted() {
        let mut bar = bar();
        bar.open_find();
        assert!(!bar.insert_text(""));
        assert!(!bar.insert_text("\n\t"));
        assert_eq!(bar.query.text(), "");
    }

    #[test]
    fn select_all_then_typing_replaces_the_pattern() {
        let mut bar = bar();
        bar.open_find();
        bar.query.set_text("alpha");
        bar.select_all();
        assert_eq!(bar.query.selection(), Some(0..5));
        bar.insert_char('b');
        assert_eq!(bar.query.text(), "b");
        assert_eq!(bar.query.caret(), 1);
        assert_eq!(bar.query.selection(), None);
    }

    #[test]
    fn select_all_then_typing_searches_the_new_pattern() {
        let mut bar = bar();
        bar.open_find();
        bar.query.set_text("alpha");
        bar.refresh(&doc("alpha alpha\nbravo\n"));
        assert_eq!(bar.matches.len(), 2);
        bar.select_all();
        bar.insert_char('b');
        bar.refresh(&doc("alpha alpha\nbravo\n"));
        assert_eq!(bar.matches.len(), 1);
    }

    #[test]
    fn select_all_then_pasting_replaces_the_pattern() {
        let mut bar = bar();
        bar.open_find();
        bar.query.set_text("alpha");
        bar.select_all();
        bar.insert_text("bravo");
        assert_eq!(bar.query.text(), "bravo");
    }

    #[test]
    fn select_all_then_backspace_empties_the_field() {
        let mut bar = bar();
        bar.open_find();
        bar.query.set_text("alpha");
        bar.select_all();
        bar.backspace();
        assert_eq!(bar.query.text(), "");
        // Only the selection went: the next backspace has nothing left to eat.
        bar.backspace();
        assert_eq!(bar.query.text(), "");
    }

    #[test]
    fn select_all_then_delete_empties_the_field() {
        let mut bar = bar();
        bar.open_find();
        bar.query.set_text("alpha");
        bar.select_all();
        bar.delete();
        assert_eq!(bar.query.text(), "");
    }

    #[test]
    fn select_all_takes_the_field_that_has_the_caret() {
        let mut bar = bar();
        bar.open_replace();
        bar.query.set_text("alpha");
        bar.replacement.set_text("bravo");
        bar.toggle_focus();
        bar.select_all();
        assert_eq!(bar.replacement.selection(), Some(0..5));
        assert_eq!(bar.query.selection(), None);
        bar.insert_char('c');
        assert_eq!(bar.replacement.text(), "c");
        assert_eq!(bar.query.text(), "alpha");
    }

    #[test]
    fn an_arrow_key_drops_the_selection() {
        let mut bar = bar();
        bar.open_find();
        bar.query.set_text("alpha");
        bar.select_all();
        bar.left();
        assert_eq!(bar.query.selection(), None);
        bar.insert_char('b');
        // Appended where the caret is, rather than replacing the field.
        assert_eq!(bar.query.text(), "alphba");
    }

    #[test]
    fn switching_fields_drops_the_selection() {
        let mut bar = bar();
        bar.open_replace();
        bar.query.set_text("alpha");
        bar.select_all();
        bar.toggle_focus();
        assert_eq!(bar.query.selection(), None);
    }

    #[test]
    fn selecting_all_an_empty_field_selects_nothing() {
        let mut bar = bar();
        bar.open_find();
        bar.select_all();
        assert_eq!(bar.query.selection(), None);
    }

    #[test]
    fn only_an_unmodified_key_types_into_the_field() {
        // A plain letter, and AltGr, which arrives as Ctrl plus Alt on this
        // platform and is how a layout puts a euro sign on a key.
        assert!(types_text(false, false));
        assert!(types_text(true, true));
        // A command chord, and an Alt menu letter.
        assert!(!types_text(true, false));
        assert!(!types_text(false, true));
    }

    #[test]
    fn the_field_starts_after_the_widest_label() {
        let mut bar = bar();
        bar.open_replace();
        bar.prepare(&mut FontSystem::new());
        let layout = bar.layout(900.0, 0.0, 20.0, 8.0);
        let widest = widest_line(&bar.label_find).max(widest_line(&bar.label_replace));
        assert!(
            layout.field_x >= 8.0 + widest,
            "field at {} but the label ends at {}",
            layout.field_x,
            8.0 + widest
        );
    }

    #[test]
    fn the_bar_grows_a_row_for_the_replacement() {
        let mut bar = bar();
        bar.open_find();
        let find = bar.height(900.0, 20.0, 8.0);
        bar.open_replace();
        assert_eq!(bar.height(900.0, 20.0, 8.0), find + 20.0);
    }

    #[test]
    fn a_closed_bar_has_no_height_and_takes_no_clicks() {
        let bar = bar();
        assert_eq!(bar.height(900.0, 20.0, 8.0), 0.0);
        assert_eq!(bar.hit_at(10.0, 10.0, 900.0, 0.0, 20.0, 8.0), None);
    }

    #[test]
    fn every_chip_is_clickable_where_it_is_drawn() {
        let mut bar = bar();
        bar.open_replace();
        bar.prepare(&mut FontSystem::new());
        let layout = bar.layout(900.0, 700.0, 20.0, 8.0);
        for (index, rect) in layout.chips.iter().enumerate() {
            let hit = bar.hit_at(
                rect.x + 2.0,
                rect.y + 2.0,
                900.0,
                700.0,
                20.0,
                8.0,
            );
            assert_eq!(hit, Some(Hit::Chip(CHIPS[index])), "chip {index}");
            // The row below the options row belongs to nothing, so a press
            // there must not be mistaken for a chip click.
            let below = bar.hit_at(
                rect.x + 2.0,
                rect.y + rect.h + 2.0,
                900.0,
                700.0,
                20.0,
                8.0,
            );
            assert!(below.is_none(), "chip {index} reached past its row");
        }
    }

    #[test]
    fn every_button_is_clickable_where_it_is_drawn() {
        let mut bar = bar();
        bar.open_replace();
        bar.prepare(&mut FontSystem::new());
        let layout = bar.layout(900.0, 700.0, 20.0, 8.0);
        for (index, rect) in layout.buttons.iter().enumerate() {
            let hit = bar.hit_at(
                rect.x + 2.0,
                rect.y + 2.0,
                900.0,
                700.0,
                20.0,
                8.0,
            );
            assert_eq!(hit, Some(Hit::Button(BUTTONS[index])), "button {index}");
        }
    }

    #[test]
    fn the_buttons_sit_on_their_own_rows_under_the_chips() {
        let mut bar = bar();
        bar.open_replace();
        bar.prepare(&mut FontSystem::new());
        let layout = bar.layout(900.0, 0.0, 20.0, 8.0);
        let chip = layout.chips[0];
        for (index, rect) in layout.buttons.iter().enumerate() {
            assert!(
                rect.y >= chip.y + chip.h,
                "button {index} at {} overlaps the chips at {}",
                rect.y,
                chip.y
            );
        }
        assert!(layout.buttons_y > layout.options_y);
    }

    #[test]
    fn a_click_in_a_field_puts_the_caret_there() {
        let mut bar = bar();
        bar.open_replace();
        bar.prepare(&mut FontSystem::new());
        let layout = bar.layout(900.0, 0.0, 20.0, 8.0);
        let query = bar.hit_at(
            layout.field_query.x + 40.0,
            layout.field_query.y + 4.0,
            900.0,
            0.0,
            20.0,
            8.0,
        );
        assert_eq!(query, Some(Hit::Field(Field::Query)));
        let replace = bar.hit_at(
            layout.field_replace.x + 40.0,
            layout.field_replace.y + 4.0,
            900.0,
            0.0,
            20.0,
            8.0,
        );
        assert_eq!(replace, Some(Hit::Field(Field::Replacement)));
    }

    #[test]
    fn the_replacement_field_is_not_reachable_when_it_is_not_shown() {
        let mut bar = bar();
        bar.open_find();
        bar.prepare(&mut FontSystem::new());
        bar.focus_field(Field::Replacement);
        assert_eq!(bar.focus, Field::Query);
    }

    #[test]
    fn the_word_chip_means_a_newline_option_in_regex_mode() {
        let mut bar = bar();
        assert_eq!(Chip::Word.mark(SearchMode::Normal), "W");
        assert_eq!(Chip::Word.mark(SearchMode::Regex), "\\n");
        bar.activate(Chip::Regex);
        bar.activate(Chip::Word);
        assert!(bar.options.dot_matches_newline);
        assert!(!bar.options.whole_word);
        bar.activate(Chip::Normal);
        bar.activate(Chip::Word);
        assert!(bar.options.whole_word);
        assert!(bar.options.dot_matches_newline);
    }

    #[test]
    fn the_mode_chips_are_mutually_exclusive() {
        let mut bar = bar();
        for chip in [Chip::Extended, Chip::Regex, Chip::Normal] {
            bar.activate(chip);
            let lit: Vec<Chip> = CHIPS
                .iter()
                .copied()
                .filter(|&c| matches!(c, Chip::Normal | Chip::Extended | Chip::Regex))
                .filter(|c| c.is_on(&bar))
                .collect();
            assert_eq!(lit.len(), 1, "{chip:?} left {lit:?} lit");
        }
    }

    #[test]
    fn only_one_direction_chip_is_lit() {
        let mut bar = bar();
        assert!(Chip::Down.is_on(&bar));
        assert!(!Chip::Up.is_on(&bar));
        bar.activate(Chip::Up);
        assert!(Chip::Up.is_on(&bar));
        assert!(!Chip::Down.is_on(&bar));
    }

    #[test]
    fn wrap_is_on_until_it_is_turned_off() {
        let mut bar = bar();
        assert!(bar.wrap);
        bar.activate(Chip::Wrap);
        assert!(!bar.wrap);
        assert!(!Chip::Wrap.is_on(&bar));
    }

    #[test]
    fn wrap_off_stops_at_the_last_match() {
        let mut bar = bar();
        bar.open_find();
        bar.query.set_text("fn");
        bar.refresh(&doc("fn a\nfn b\n"));
        bar.wrap = false;
        assert_eq!(bar.next_from(4), Some(5..7));
        assert_eq!(bar.next_from(7), None);
        // The caret stays on the match it was on rather than jumping home.
        assert_eq!(bar.current, Some(1));
    }

    #[test]
    fn wrap_on_starts_again_at_the_first_match() {
        let mut bar = bar();
        bar.open_find();
        bar.query.set_text("fn");
        bar.refresh(&doc("fn a\nfn b\n"));
        assert_eq!(bar.next_from(7), Some(0..2));
    }

    #[test]
    fn wrap_off_stops_going_backwards_at_the_first_match() {
        let mut bar = bar();
        bar.open_find();
        bar.query.set_text("fn");
        bar.refresh(&doc("fn a\nfn b\n"));
        bar.wrap = false;
        assert_eq!(bar.prev_from(5), Some(0..2));
        assert_eq!(bar.prev_from(0), None);
        bar.wrap = true;
        assert_eq!(bar.prev_from(0), Some(5..7));
    }

    #[test]
    fn in_selection_keeps_only_matches_inside_the_range() {
        let mut bar = bar();
        bar.open_find();
        bar.query.set_text("fn");
        let document = doc("fn a\nfn b\nfn c\n");
        bar.refresh_within(&document, Some(5..10));
        assert_eq!(bar.matches, vec![5..7]);
    }

    #[test]
    fn a_match_that_starts_before_the_selection_is_not_in_it() {
        let mut bar = bar();
        bar.open_find();
        bar.query.set_text("abc");
        // The selection starts inside the only match.
        bar.refresh_within(&doc("abcdef"), Some(1..4));
        assert!(bar.matches.is_empty());
    }

    #[test]
    fn the_selection_restriction_is_dropped_when_it_is_cleared() {
        let mut bar = bar();
        bar.open_find();
        bar.query.set_text("fn");
        let document = doc("fn a\nfn b\n");
        bar.refresh_within(&document, Some(5..10));
        assert_eq!(bar.matches.len(), 1);
        bar.refresh(&document);
        assert_eq!(bar.matches.len(), 2);
    }

    #[test]
    fn a_pattern_is_remembered_once_and_most_recent_first() {
        let mut bar = bar();
        bar.open_find();
        for pattern in ["alpha", "beta", "alpha"] {
            bar.query.set_text(pattern);
            bar.remember();
        }
        assert_eq!(bar.history, vec!["alpha", "beta"]);
        // An empty pattern is not a search and is not remembered.
        bar.query.set_text("");
        bar.remember();
        assert_eq!(bar.history, vec!["alpha", "beta"]);
    }

    #[test]
    fn closing_the_bar_files_both_fields_away() {
        let mut bar = bar();
        bar.open_replace();
        bar.query.set_text("needle");
        bar.replacement.set_text("haystack");
        bar.close(true);
        assert_eq!(bar.history, vec!["needle"]);
        assert_eq!(bar.replacement_history, vec!["haystack"]);
    }

    #[test]
    fn an_entry_of_the_history_fills_its_field() {
        let mut bar = bar();
        bar.open_find();
        bar.set_history(vec!["one".into(), "two".into()], Vec::new());
        bar.toggle_history(Field::Query);
        assert_eq!(bar.history_open(), Some(Field::Query));
        bar.pick_history(Field::Query, 1);
        assert_eq!(bar.query.text(), "two");
        assert_eq!(bar.history_open(), None);
    }

    #[test]
    fn the_arrows_walk_the_open_history_and_wrap() {
        let mut bar = bar();
        bar.open_find();
        bar.set_history(
            vec!["one".into(), "two".into(), "three".into()],
            Vec::new(),
        );
        bar.toggle_history(Field::Query);
        bar.move_history(1);
        assert_eq!(bar.history_pick(), Some(0));
        bar.move_history(1);
        assert_eq!(bar.history_pick(), Some(1));
        bar.move_history(-1);
        bar.move_history(-1);
        assert_eq!(bar.history_pick(), Some(2));
        bar.toggle_history(Field::Query);
        assert_eq!(bar.history_open(), None);
    }

    #[test]
    fn the_swap_button_exchanges_the_two_fields() {
        let mut bar = bar();
        bar.open_replace();
        bar.query.set_text("left");
        bar.replacement.set_text("right");
        bar.swap(SwapChip::Swap);
        assert_eq!(bar.query.text(), "right");
        assert_eq!(bar.replacement.text(), "left");
    }

    #[test]
    fn the_two_copy_buttons_leave_one_field_alone() {
        let mut bar = bar();
        bar.open_replace();
        bar.query.set_text("left");
        bar.replacement.set_text("right");
        bar.swap(SwapChip::FindToReplace);
        assert_eq!(bar.query.text(), "left");
        assert_eq!(bar.replacement.text(), "left");
        bar.swap(SwapChip::ReplaceToFind);
        assert_eq!(bar.query.text(), "left");
    }

    #[test]
    fn the_swap_buttons_are_only_there_with_the_replacement() {
        let mut bar = bar();
        bar.open_find();
        let layout = bar.layout(900.0, 700.0, 20.0, 8.0);
        let swap = layout.swaps[0];
        assert_eq!(
            bar.hit_at(swap.x + 2.0, swap.y + 2.0, 900.0, 700.0, 20.0, 8.0),
            None
        );
        bar.open_replace();
        let layout = bar.layout(900.0, 700.0, 20.0, 8.0);
        let swap = layout.swaps[0];
        assert_eq!(
            bar.hit_at(swap.x + 2.0, swap.y + 2.0, 900.0, 700.0, 20.0, 8.0),
            Some(Hit::Swap(SwapChip::Swap))
        );
    }

    #[test]
    fn the_history_arrow_and_its_entries_are_clickable() {
        let mut bar = bar();
        bar.open_find();
        bar.set_history(vec!["one".into(), "two".into()], Vec::new());
        bar.toggle_history(Field::Query);
        bar.prepare(&mut FontSystem::new());
        let layout = bar.layout(900.0, 700.0, 20.0, 8.0);
        let toggle = layout.history_toggle[0];
        assert_eq!(
            bar.hit_at(toggle.x + 2.0, toggle.y + 2.0, 900.0, 700.0, 20.0, 8.0),
            Some(Hit::HistoryToggle(Field::Query))
        );
        // The list grows upwards, above the field it belongs to.
        assert_eq!(layout.history.len(), 2);
        assert!(layout.history[0].y + layout.history[0].h <= layout.field_query.y);
        assert!(layout.history[1].y < layout.history[0].y);
        let row = layout.history[1];
        assert_eq!(
            bar.hit_at(row.x + 2.0, row.y + 2.0, 900.0, 700.0, 20.0, 8.0),
            Some(Hit::HistoryEntry(1))
        );
    }

    #[test]
    fn a_narrow_bar_wraps_the_buttons_onto_another_row() {
        let mut bar = bar();
        bar.open_replace();
        bar.prepare(&mut FontSystem::new());
        let wide = bar.layout(1200.0, 0.0, 20.0, 8.0);
        let narrow = bar.layout(520.0, 0.0, 20.0, 8.0);
        let wide_rows: Vec<f32> = wide.buttons.iter().map(|rect| rect.y).collect();
        let narrow_rows: Vec<f32> = narrow.buttons.iter().map(|rect| rect.y).collect();
        let distinct = |rows: &[f32]| {
            let mut seen: Vec<f32> = rows.to_vec();
            seen.sort_by(|a, b| a.partial_cmp(b).unwrap());
            seen.dedup();
            seen.len()
        };
        assert!(
            distinct(&narrow_rows) > distinct(&wide_rows),
            "narrow {narrow_rows:?} vs wide {wide_rows:?}"
        );
        assert!(narrow.height > wide.height);
    }

    #[test]
    fn the_buttons_stay_inside_the_window_when_it_is_narrow() {
        let mut bar = bar();
        bar.open_replace();
        bar.prepare(&mut FontSystem::new());
        let layout = bar.layout(520.0, 0.0, 20.0, 8.0);
        for rect in layout.buttons.iter().filter(|rect| rect.w > 0.0) {
            assert!(
                rect.x >= 0.0 && rect.x + rect.w <= 520.0,
                "button at {} is off the edge",
                rect.x
            );
        }
    }

    #[test]
    fn the_history_list_starts_at_the_field_it_belongs_to() {
        let mut bar = bar();
        bar.open_replace();
        bar.set_history(Vec::new(), vec!["x".into()]);
        bar.toggle_history(Field::Replacement);
        let layout = bar.layout(900.0, 700.0, 20.0, 8.0);
        let row = layout.history[0];
        assert_eq!(row.x, layout.field_replace.x);
        assert!(row.y + row.h <= layout.field_replace.y);
    }

    #[test]
    fn every_control_fits_inside_the_height_the_bar_reserves() {
        // The renderer reserves `height` pixels for the bar and draws the rows
        // from the top of that band, so a row that reaches past it is drawn over
        // the status bar -- which is exactly what a mistyped row count does.
        for replace in [false, true] {
            for width in [520.0, 900.0, 1100.0, 1600.0] {
                let mut bar = bar();
                if replace {
                    bar.open_replace();
                } else {
                    bar.open_find();
                }
                bar.prepare(&mut FontSystem::new());
                let top = 600.0;
                let layout = bar.layout(width, top, 20.0, 8.0);
                let bottom = top + layout.height;
                for rect in layout.buttons.iter().filter(|rect| rect.w > 0.0) {
                    assert!(
                        rect.y + rect.h <= bottom,
                        "replace {replace} width {width}: button bottom {} past {bottom}",
                        rect.y + rect.h
                    );
                }
                assert!(layout.options_y + 20.0 <= bottom);
            }
        }
    }
}
