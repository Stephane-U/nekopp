//! The find and replace bar.
//!
//! Up to three rows along the bottom of the window: the pattern and its match
//! count, the replacement when Replace is open, and a row of settings and
//! buttons under them. It owns its own state and its own layout so that the
//! editor view only has to ask it for rectangles and text areas.
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
use crate::theme::{mix, Rgb, Theme};

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
}

/// The chips, in the order they are drawn.
pub const CHIPS: [Chip; 9] = [
    Chip::Case,
    Chip::Word,
    Chip::Normal,
    Chip::Extended,
    Chip::Regex,
    Chip::Wrap,
    Chip::Selection,
    Chip::Up,
    Chip::Down,
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
        }
    }
}

/// A button in the bar's option row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// Walk to the next match in the chosen direction.
    FindNext,
    /// Replace the current match and walk on.
    Replace,
    /// Replace every match.
    ReplaceAll,
    /// Close the bar.
    Close,
}

/// The buttons, in the order they are drawn, left to right.
pub const BUTTONS: [Button; 4] = [
    Button::FindNext,
    Button::Replace,
    Button::ReplaceAll,
    Button::Close,
];

impl Button {
    /// The word on the button.
    pub fn label(self) -> &'static str {
        match self {
            Button::FindNext => "Find Next",
            Button::Replace => "Replace",
            Button::ReplaceAll => "Replace All",
            Button::Close => "Close",
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
    /// One of the two text fields.
    Field(Field),
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
    chips: [Buffer; 9],
    chip_marks: [&'static str; 9],
    /// The word on each button, shaped, and its width once shaped.
    buttons: [Buffer; 4],
    button_w: [f32; 4],
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
            replace: false,
            query: TextField::new(metrics, family),
            replacement: TextField::new(metrics, family),
            focus: Field::Query,
            wrap: true,
            in_selection: false,
            up: false,
            incremental: false,
            options: SearchOptions::default(),
            matches: Vec::new(),
            current: None,
            error: None,
            compiled: None,
            stale: true,
            family,
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
            chip_marks: [""; 9],
            buttons: std::array::from_fn(|i| label(BUTTONS[i].label())),
            button_w: [0.0; 4],
            buttons_shaped: false,
            count: label(""),
            count_text: String::new(),
            labels_shaped: false,
            catalog: None,
        }
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

    /// The height the bar occupies, in pixels.
    ///
    /// Two rows in find mode -- the pattern and the settings -- and three with
    /// the replacement open.
    pub fn height(&self, line_height: f32, padding: f32) -> f32 {
        if !self.open {
            return 0.0;
        }
        let rows = if self.replace { 3.0 } else { 2.0 };
        rows * line_height + 2.0 * padding
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
        let options_y = find_y + if self.replace { 2.0 } else { 1.0 } * row;

        let count_w = 72.0;
        let right = width - padding;
        let label_x = padding;
        // The fields run to the match count and no further: with the settings
        // moved to their own row there is nothing left to dodge on this one.
        let field_x = padding + self.label_w;
        let count_x = right - count_w;
        let field_w = (count_x - CHIP_GAP - field_x).max(40.0);
        let field = |y: f32| Rect {
            x: field_x,
            y,
            w: field_w,
            h: row,
        };

        // The buttons take the right end of the option row, laid out right to
        // left in reverse order, so Close is always the furthest one out: a
        // button that shifts when a translation gets longer is a button people
        // click wrong.
        let mut buttons = [Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        }; 4];
        let mut x = right;
        for index in (0..BUTTONS.len()).rev() {
            let w = self.button_w[index].max(44.0);
            x -= w;
            buttons[index] = Rect {
                x,
                y: options_y + 2.0,
                w,
                h: row - 4.0,
            };
            x -= CHIP_GAP;
        }

        // The chips start where the fields do, so the settings read as a column
        // under the pattern rather than floating in the middle of the bar.
        let mut chips = [Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        }; 9];
        let mut x = field_x;
        for rect in chips.iter_mut() {
            *rect = Rect {
                x,
                y: options_y + 2.0,
                w: CHIP_W,
                h: row - 4.0,
            };
            x += CHIP_W + 4.0;
        }

        BarLayout {
            height: self.height(line_height, padding),
            find_y,
            replace_y,
            options_y,
            label_x,
            field_x,
            field_w,
            field_query: field(find_y),
            field_replace: field(replace_y),
            chips,
            buttons,
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
        if !self.open {
            return None;
        }
        let layout = self.layout(width, top, line_height, padding);
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
        if !self.open {
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
        for rect in layout.buttons.iter() {
            quads.push(
                rect.x,
                rect.y,
                rect.w,
                rect.h,
                mix(theme.bar_background, theme.foreground, 0.12).to_linear(),
            );
            quads.push(rect.x, rect.y, rect.w, 1.0, theme.bar_border.to_linear());
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
        if !self.open {
            return Vec::new();
        }
        let layout = self.layout(width, top, line_height, padding);
        let bounds = TextBounds {
            left: 0,
            top: top as i32,
            right: width as i32,
            bottom: (top + layout.height) as i32,
        };
        let mut areas = Vec::with_capacity(8);

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
            areas.push(TextArea {
                buffer: &self.buttons[index],
                left: rect.x + BUTTON_PAD,
                top: layout.options_y,
                scale: 1.0,
                bounds,
                default_color: theme.foreground.to_cosmic(),
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
    /// Top of the settings and buttons row.
    pub options_y: f32,
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
    /// The nine setting chips.
    pub chips: [Rect; 9],
    /// The four buttons.
    pub buttons: [Rect; 4],
    /// Left edge of the match counter.
    pub count_x: f32,
    /// Width of the match counter.
    pub count_w: f32,
    /// Height of one row.
    pub row: f32,
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
        let find = bar.height(20.0, 8.0);
        bar.open_replace();
        assert_eq!(bar.height(20.0, 8.0), find + 20.0);
    }

    #[test]
    fn a_closed_bar_has_no_height_and_takes_no_clicks() {
        let bar = bar();
        assert_eq!(bar.height(20.0, 8.0), 0.0);
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
        bar.open_find();
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
    fn the_buttons_leave_room_for_the_chips() {
        let mut bar = bar();
        bar.open_find();
        bar.prepare(&mut FontSystem::new());
        let layout = bar.layout(900.0, 0.0, 20.0, 8.0);
        let last_chip = layout.chips[8];
        assert!(
            last_chip.x + last_chip.w < layout.buttons[0].x,
            "chips end at {} and the first button starts at {}",
            last_chip.x + last_chip.w,
            layout.buttons[0].x
        );
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
}
