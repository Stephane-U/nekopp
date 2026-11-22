//! The find and replace bar.
//!
//! A single-line text field, three option toggles, and a match counter, laid out
//! along the bottom of the window. It owns its own state and its own layout so
//! that the editor view only has to ask it for rectangles and text areas.
//!
//! The bar is keyboard-first: `Ctrl+F` opens it, typing goes into the query,
//! `Enter` walks forward through matches, `Escape` closes. The toggles are also
//! clickable, because a toggle you cannot click is a toggle people will not find.

use std::ops::Range;

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use glyphon::{TextArea, TextBounds};
use nn_core::{Document, Search, SearchOptions};

use crate::i18n::Catalog;
use crate::quad::QuadRenderer;
use crate::theme::{Rgb, Theme};

/// Which field has the caret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// The search pattern.
    Query,
    /// The replacement text.
    Replacement,
}

/// One of the three option toggles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    /// Match case.
    Case,
    /// Whole word only.
    Word,
    /// Regular expression.
    Regex,
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

    /// Replaces the contents and puts the caret at the end.
    pub fn set_text(&mut self, text: &str) {
        if self.text != text {
            self.text.clear();
            self.text.push_str(text);
            self.caret = self.text.chars().count();
            self.stale = true;
        }
    }

    /// Inserts `c` at the caret.
    pub fn insert(&mut self, c: char) {
        let byte = self.byte_at(self.caret);
        self.text.insert(byte, c);
        self.caret += 1;
        self.stale = true;
    }

    /// Removes the character before the caret.
    pub fn backspace(&mut self) {
        if self.caret == 0 {
            return;
        }
        let byte = self.byte_at(self.caret - 1);
        self.text.remove(byte);
        self.caret -= 1;
        self.stale = true;
    }

    /// Removes the character after the caret.
    pub fn delete(&mut self) {
        if self.caret >= self.text.chars().count() {
            return;
        }
        let byte = self.byte_at(self.caret);
        self.text.remove(byte);
        self.stale = true;
    }

    /// Moves the caret one character left.
    pub fn left(&mut self) {
        self.caret = self.caret.saturating_sub(1);
    }

    /// Moves the caret one character right.
    pub fn right(&mut self) {
        self.caret = (self.caret + 1).min(self.text.chars().count());
    }

    /// Moves the caret to the start.
    pub fn home(&mut self) {
        self.caret = 0;
    }

    /// Moves the caret to the end.
    pub fn end(&mut self) {
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
    toggle_labels: [Buffer; 3],
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
            toggle_labels: [label("Aa"), label("W"), label(".*")],
            count: label(""),
            count_text: String::new(),
            labels_shaped: false,
            catalog: None,
        }
    }

    /// Every label this bar can draw, for the coverage test.
    ///
    /// The three toggle buffers are absent on purpose: `Aa`, `W` and `.*` are
    /// the marks on the buttons, not words, and they are the same marks in
    /// every language.
    pub fn labels() -> Vec<&'static str> {
        vec!["Find:", "Replace:", "no match", "bad regex"]
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

    /// Rewrites the two field labels from the catalog.
    fn relabel(&mut self) {
        let (find, replace) = self.labels_text();
        for (buffer, text) in [(&mut self.label_find, find), (&mut self.label_replace, replace)] {
            buffer.set_text(&text, &Attrs::new().family(self.family), Shaping::Basic, None);
        }
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
        self.focus = match self.focus {
            Field::Query => Field::Replacement,
            Field::Replacement => Field::Query,
        };
    }

    /// Recompiles the pattern and recomputes matches if anything changed.
    pub fn refresh(&mut self, doc: &Document) {
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

    /// Advances to the next match at or after `from`, wrapping.
    pub fn next_from(&mut self, from: usize) -> Option<Range<usize>> {
        if self.matches.is_empty() {
            return None;
        }
        let index = self
            .matches
            .iter()
            .position(|m| m.start >= from)
            .unwrap_or(0);
        self.current = Some(index);
        self.matches.get(index).cloned()
    }

    /// Steps back to the previous match before `from`, wrapping.
    pub fn prev_from(&mut self, from: usize) -> Option<Range<usize>> {
        if self.matches.is_empty() {
            return None;
        }
        let index = self
            .matches
            .iter()
            .rposition(|m| m.end <= from)
            .unwrap_or(self.matches.len() - 1);
        self.current = Some(index);
        self.matches.get(index).cloned()
    }

    /// Flips one option and marks the search stale.
    pub fn toggle(&mut self, which: Toggle) {
        match which {
            Toggle::Case => self.options.case_sensitive = !self.options.case_sensitive,
            Toggle::Word => self.options.whole_word = !self.options.whole_word,
            Toggle::Regex => self.options.regex = !self.options.regex,
        }
        self.stale = true;
    }

    /// The compiled search, if the pattern is valid.
    pub fn search(&self) -> Option<&Search> {
        self.compiled.as_ref()
    }

    /// The height the bar occupies, in pixels.
    pub fn height(&self, line_height: f32, padding: f32) -> f32 {
        if !self.open {
            return 0.0;
        }
        let rows = if self.replace { 2.0 } else { 1.0 };
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
            for buffer in &mut self.toggle_labels {
                buffer.shape_until_scroll(font_system, false);
            }
            self.labels_shaped = true;
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

        let toggle_w = 26.0;
        let count_w = 72.0;
        let label_w = 62.0;
        let right = width - padding;

        let toggles = [
            Toggle::Case,
            Toggle::Word,
            Toggle::Regex,
        ];
        let mut toggle_rects = [Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        }; 3];
        let gap = 14.0;
        for (i, _) in toggles.iter().enumerate() {
            let x = right - count_w - gap - (3 - i) as f32 * toggle_w;
            toggle_rects[i] = Rect {
                x,
                y: find_y,
                w: toggle_w,
                h: row,
            };
        }

        let field_x = padding + label_w;
        let field_w = (right - count_w - gap - 3.0 * toggle_w - field_x).max(40.0);

        BarLayout {
            height: self.height(line_height, padding),
            find_y,
            replace_y,
            label_x: padding,
            field_x,
            field_w,
            toggles: toggle_rects,
            count_x: right - count_w,
            count_w,
            row,
        }
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

        for (i, rect) in layout.toggles.iter().enumerate() {
            let on = match i {
                0 => self.options.case_sensitive,
                1 => self.options.whole_word,
                _ => self.options.regex,
            };
            if on {
                quads.push(
                    rect.x,
                    rect.y + 2.0,
                    rect.w,
                    rect.h - 4.0,
                    theme.toggle_on.to_linear(),
                );
            }
        }

        // The caret in the focused field.
        let (field, x) = match self.focus {
            Field::Query => (&self.query, layout.field_x),
            Field::Replacement => (&self.replacement, layout.field_x),
        };
        let y = match self.focus {
            Field::Query => layout.find_y,
            Field::Replacement => layout.replace_y,
        };
        let caret_x = x + field.caret() as f32 * self.char_width;
        quads.push(caret_x, y + 2.0, 1.5, layout.row - 4.0, theme.caret.to_linear());
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
        for (i, rect) in layout.toggles.iter().enumerate() {
            let on = match i {
                0 => self.options.case_sensitive,
                1 => self.options.whole_word,
                _ => self.options.regex,
            };
            areas.push(TextArea {
                buffer: &self.toggle_labels[i],
                left: rect.x + 4.0,
                top: rect.y,
                scale: 1.0,
                bounds,
                default_color: toggle_color(on, theme).to_cosmic(),
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
    /// Left edge of the labels.
    pub label_x: f32,
    /// Left edge of the text fields.
    pub field_x: f32,
    /// Width of the text fields.
    pub field_w: f32,
    /// The three toggle rectangles.
    pub toggles: [Rect; 3],
    /// Left edge of the match counter.
    pub count_x: f32,
    /// Width of the match counter.
    pub count_w: f32,
    /// Height of one row.
    pub row: f32,
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
}
