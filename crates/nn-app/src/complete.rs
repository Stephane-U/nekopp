//! The word-completion popup.
//!
//! A floating list under the caret, the way Notepad++'s `Ctrl+Space` completion
//! appears. It is drawn in the overlay pass, after the editor's text, because a
//! popup has to cover the glyphs it sits on — the same reason the menu dropdown
//! gets its own pass.
//!
//! The list is capped at ten rows and scrolls with the selection, so a prefix
//! that matches two hundred words still shows a box you can read.

use std::ops::Range;

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use glyphon::{TextArea, TextBounds};

use crate::quad::QuadRenderer;
use crate::theme::{Rgb, Theme};

/// The most rows the popup shows at once.
pub const VISIBLE_ROWS: usize = 10;
/// The narrowest and widest the popup gets, in pixels.
const MIN_WIDTH: f32 = 120.0;
/// Wide enough for a full signature: `set_caret_from_point(&mut self, x: f32,
/// y: f32, extend: bool)` is sixty characters, and a completion that cuts the
/// parameters off is not showing you the parameters.
const MAX_WIDTH: f32 = 560.0;
/// Padding inside the frame.
const PADDING: f32 = 6.0;

/// Which of the three completion modes the popup is serving.
///
/// The popup itself does not care: it shows rows and reports which one is
/// highlighted. The mode lives here rather than in `main.rs` because it decides
/// what a keystroke refilters against, and that decision is worth a test.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CompletionMode {
    /// Words already in the active document.
    Word,
    /// Words from every open document.
    Document,
    /// Function declarations in the active document.
    Function,
    /// Filesystem paths.
    Path,
}

impl CompletionMode {
    /// The word the status bar uses while the popup is up.
    pub fn label(self) -> &'static str {
        match self {
            CompletionMode::Word => "Word completion",
            CompletionMode::Document => "Document completion",
            CompletionMode::Function => "Function completion",
            CompletionMode::Path => "Path completion",
        }
    }

    /// Whether typing a word character should refilter the list.
    ///
    /// Path completion refilters on every character, because a filename can hold
    /// dots and dashes that are not word characters. The other three only
    /// refilter on word characters: typing `(` after a half-typed word means you
    /// are done with the word, not that the word grew.
    pub fn refilters_on(self, c: char) -> bool {
        match self {
            CompletionMode::Path => !c.is_whitespace(),
            _ => c.is_alphanumeric() || c == '_',
        }
    }
}

/// One row of the completion list.
///
/// A row is not a string because the three completion modes disagree about what
/// accepting one means. Word completion inserts the word it shows. Function
/// completion shows `save(path: &Path)` but inserts `save()` and leaves the
/// caret between the parentheses, because the parameters are a reminder, not
/// text you asked for. Keeping the label and the insertion apart is what lets
/// one popup serve all three.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Item {
    /// What the row shows.
    pub label: String,
    /// What accepting the row inserts.
    pub insert: String,
    /// How far to move the caret back after inserting.
    pub back: usize,
}

impl Item {
    /// A row that inserts exactly what it shows.
    pub fn word(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            label: text.clone(),
            insert: text,
            back: 0,
        }
    }

    /// A row for a function: shows the signature, inserts the call.
    pub fn function(name: &str, params: &str) -> Self {
        Self {
            label: format!("{name}({params})"),
            insert: format!("{name}()"),
            back: 1,
        }
    }
}

/// The floating completion list.
pub struct CompletionPopup {
    items: Vec<Item>,
    /// Index into `items` of the highlighted row.
    selected: usize,
    /// Index of the first visible row.
    scroll: usize,
    /// Top-left corner in window pixels, before the flip.
    anchor: (f32, f32),
    /// Window height, so the popup can flip above the caret near the bottom.
    viewport: f32,
    open: bool,
    body: Buffer,
    body_shaped: String,
    /// What the body was shaped for: (item count, scroll, width in chars).
    shaped_key: (usize, usize, usize),
    family: Family<'static>,
    char_width: f32,
}

impl CompletionPopup {
    /// Builds a closed popup.
    pub fn new(metrics: Metrics, family: Family<'static>, char_width: f32) -> Self {
        Self {
            items: Vec::new(),
            selected: 0,
            scroll: 0,
            anchor: (0.0, 0.0),
            viewport: 0.0,
            open: false,
            body: Buffer::new_empty(metrics),
            body_shaped: String::new(),
            shaped_key: (usize::MAX, usize::MAX, usize::MAX),
            family,
            char_width,
        }
    }

    /// Records the window height, so a popup near the bottom edge flips above
    /// the caret instead of running off the screen.
    pub fn set_viewport(&mut self, height: f32) {
        self.viewport = height;
    }

    /// The top edge actually drawn at: `anchor.y`, or the anchor minus the
    /// popup's own height when it would not fit below.
    fn top(&self, line_height: f32) -> f32 {
        let (_, y) = self.anchor;
        let height = self.height(line_height);
        if self.viewport > 0.0 && y + height > self.viewport {
            (y - height).max(0.0)
        } else {
            y
        }
    }

    /// Shows `items` with the top-left corner at `anchor`. An empty list closes.
    pub fn show(&mut self, items: Vec<Item>, anchor: (f32, f32)) {
        self.open = !items.is_empty();
        self.items = items;
        self.selected = 0;
        self.scroll = 0;
        self.anchor = anchor;
        self.shaped_key = (usize::MAX, usize::MAX, usize::MAX);
    }

    /// Hides the popup and drops the list.
    pub fn close(&mut self) {
        self.open = false;
        self.items.clear();
        self.selected = 0;
        self.scroll = 0;
        self.body_shaped.clear();
        self.shaped_key = (usize::MAX, usize::MAX, usize::MAX);
    }

    /// Whether the popup is showing.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The highlighted row, for the accept handler.
    pub fn selected(&self) -> Option<&Item> {
        self.items.get(self.selected)
    }

    /// The label of the highlighted row, for tests and for the status bar.
    pub fn selected_label(&self) -> Option<&str> {
        self.selected().map(|item| item.label.as_str())
    }

    /// The number of candidates.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether there is nothing to show.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Moves the highlight by `delta` rows, wrapping at both ends.
    ///
    /// Wrapping rather than clamping: `Up` on the first row should reach the
    /// last one, which is what every completion list does.
    pub fn move_selection(&mut self, delta: isize) {
        if self.items.is_empty() {
            return;
        }
        let n = self.items.len() as isize;
        self.selected = (((self.selected as isize + delta) % n + n) % n) as usize;
        self.scroll_to_selection();
    }

    /// Highlights `index`, for a click on the list.
    pub fn set_selected(&mut self, index: usize) {
        if index < self.items.len() {
            self.selected = index;
            self.scroll_to_selection();
        }
    }

    /// Keeps the highlighted row inside the visible window.
    fn scroll_to_selection(&mut self) {
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + VISIBLE_ROWS {
            self.scroll = self.selected + 1 - VISIBLE_ROWS;
        }
    }

    /// The width the popup occupies.
    pub fn width(&self) -> f32 {
        let longest = self
            .items
            .iter()
            .map(|item| item.label.chars().count())
            .max()
            .unwrap_or(0);
        (longest as f32 * self.char_width + PADDING * 2.0).clamp(MIN_WIDTH, MAX_WIDTH)
    }

    /// The height the popup occupies.
    pub fn height(&self, line_height: f32) -> f32 {
        if !self.open {
            return 0.0;
        }
        let rows = self.items.len().min(VISIBLE_ROWS) as f32;
        rows * line_height + PADDING * 2.0
    }

    /// The item index under `(x, y)` in window pixels.
    pub fn item_at(&self, x: f32, y: f32, line_height: f32) -> Option<usize> {
        if !self.open {
            return None;
        }
        let (left, _) = self.anchor;
        let top = self.top(line_height);
        if x < left || x > left + self.width() || y < top || y > top + self.height(line_height) {
            return None;
        }
        let row = ((y - top - PADDING) / line_height) as usize;
        if row >= VISIBLE_ROWS {
            return None;
        }
        let index = self.scroll + row;
        (index < self.items.len()).then_some(index)
    }

    /// Highlights the item under `(x, y)`, if any.
    pub fn hover(&mut self, x: f32, y: f32, line_height: f32) {
        if let Some(index) = self.item_at(x, y, line_height) {
            self.selected = index;
        }
    }

    /// Rebuilds the shaped rows if anything they depend on changed.
    pub fn prepare(&mut self, font_system: &mut FontSystem) {
        if !self.open {
            return;
        }
        // Rounded, not truncated: `width()` is `columns * char_width + padding`,
        // so dividing it back out lands on 46.9999 in f32 and a cast would drop
        // a column -- which shows up as an ellipsis on a label that fits.
        let columns = ((self.width() - PADDING * 2.0) / self.char_width)
            .max(4.0)
            .round() as usize;
        let key = (self.items.len(), self.scroll, columns);
        if key == self.shaped_key {
            return;
        }
        self.shaped_key = key;

        let visible = self.items.len().min(VISIBLE_ROWS);
        let mut text = String::with_capacity(visible * (columns + 1));
        for row in 0..visible {
            if row > 0 {
                text.push('\n');
            }
            let item = &self.items[self.scroll + row];
            text.push_str(&truncate(&item.label, columns));
        }
        if text == self.body_shaped {
            return;
        }
        self.body.set_text(
            &text,
            &Attrs::new().family(self.family),
            Shaping::Basic,
            None,
        );
        self.body.shape_until_scroll(font_system, false);
        self.body_shaped = text;
    }

    /// Queues the frame and the highlighted row.
    pub fn push_quads(&self, quads: &mut QuadRenderer, line_height: f32, theme: &Theme) {
        if !self.open {
            return;
        }
        let (left, _) = self.anchor;
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

        let row = self.selected.wrapping_sub(self.scroll);
        if row < VISIBLE_ROWS && self.selected < self.items.len() {
            quads.push(
                left + 1.0,
                top + PADDING + row as f32 * line_height,
                width - 2.0,
                line_height,
                theme.menu_hover.to_linear(),
            );
        }
    }

    /// The text areas of the visible rows.
    pub fn text_areas<'a>(&'a self, line_height: f32, theme: &Theme) -> Vec<TextArea<'a>> {
        if !self.open {
            return Vec::new();
        }
        let (left, _) = self.anchor;
        let top = self.top(line_height);
        let bounds = TextBounds {
            left: left as i32,
            top: top as i32,
            right: (left + self.width()) as i32,
            bottom: (top + self.height(line_height)) as i32,
        };
        vec![TextArea {
            buffer: &self.body,
            left: left + PADDING,
            top: top + PADDING,
            scale: 1.0,
            bounds,
            default_color: theme.menu_text.to_cosmic(),
            custom_glyphs: &[],
        }]
    }
}

/// The parameter hint: a one-line box under the caret naming the signature of
/// the call the caret is inside.
///
/// It is deliberately not a `CompletionPopup`. A popup is a list you choose from;
/// a hint is a label you read. Giving it a selection would invite the arrow keys
/// to move a highlight that means nothing, and giving it rows would make it
/// compete with the popup for the same screen space when both are up.
///
/// The argument being typed is drawn in the accent colour, which is the whole
/// point: on `save(path, |)` the hint says which of the two arguments you are
/// on without you counting commas.
pub struct ParamHint {
    text: String,
    /// Byte range of the active argument inside `text`.
    active: Option<Range<usize>>,
    anchor: (f32, f32),
    viewport: f32,
    open: bool,
    body: Buffer,
    /// What the body was shaped for: (text, columns, accent colour).
    shaped: (String, usize, Rgb),
    family: Family<'static>,
    char_width: f32,
}

impl ParamHint {
    /// Builds a closed hint.
    pub fn new(metrics: Metrics, family: Family<'static>, char_width: f32) -> Self {
        Self {
            text: String::new(),
            active: None,
            anchor: (0.0, 0.0),
            viewport: 0.0,
            open: false,
            body: Buffer::new_empty(metrics),
            shaped: (String::new(), 0, Rgb(0, 0, 0)),
            family,
            char_width,
        }
    }

    /// Records the window height, so a hint near the bottom edge flips above the
    /// caret instead of running off the screen.
    pub fn set_viewport(&mut self, height: f32) {
        self.viewport = height;
    }

    /// Shows `text` with `active` picked out, top-left at `anchor`.
    pub fn show(&mut self, text: String, active: Option<Range<usize>>, anchor: (f32, f32)) {
        self.open = !text.is_empty();
        self.text = text;
        self.active = active;
        self.anchor = anchor;
        self.shaped.1 = 0;
    }

    /// Hides the hint.
    pub fn close(&mut self) {
        self.open = false;
        self.text.clear();
        self.active = None;
        self.shaped.1 = 0;
    }

    /// Whether the hint is showing.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The signature being shown.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Moves the highlight `delta` arguments along, wrapping at the ends.
    ///
    /// Notepad++'s Ctrl+Shift+Down/Up walk the parameter list without moving the
    /// caret, so the highlight has to be able to disagree with the caret until
    /// the next keystroke recomputes it. The argument boundaries are recovered
    /// from the signature rather than stored, because the signature is the only
    /// thing that survives a re-show.
    pub fn move_selection(&mut self, delta: isize) {
        if !self.open || delta == 0 {
            return;
        }
        let Some(open) = self.text.find('(') else {
            return;
        };
        let Some(close) = self.text.rfind(')') else {
            return;
        };
        if close <= open {
            return;
        }
        let params = &self.text[open + 1..close];
        let mut ranges = Vec::new();
        for index in 0.. {
            match nn_core::arg_range(params, index) {
                Some(range) => ranges.push(range),
                None => break,
            }
        }
        if ranges.is_empty() {
            return;
        }
        // Which argument the highlight is on now. A signature with no highlight
        // yet starts from the first, so the first press moves to the second.
        let current = self
            .active
            .as_ref()
            .and_then(|active| {
                let at = active.start.checked_sub(open + 1)?;
                ranges.iter().position(|r| at >= r.start && at <= r.end)
            })
            .unwrap_or(0);
        let count = ranges.len() as isize;
        let next = (current as isize + delta).rem_euclid(count) as usize;
        let range = ranges[next].clone();
        self.active = Some(open + 1 + range.start..open + 1 + range.end);
        self.shaped.1 = 0;
    }

    /// The top edge actually drawn at, flipped when it would not fit below.
    fn top(&self, line_height: f32) -> f32 {
        let (_, y) = self.anchor;
        let height = self.height(line_height);
        if self.viewport > 0.0 && y + height > self.viewport {
            (y - height).max(0.0)
        } else {
            y
        }
    }

    /// The width the hint occupies.
    pub fn width(&self) -> f32 {
        let columns = self.text.chars().count() as f32;
        (columns * self.char_width + PADDING * 2.0).clamp(MIN_WIDTH, MAX_WIDTH)
    }

    /// The height the hint occupies.
    pub fn height(&self, line_height: f32) -> f32 {
        if !self.open {
            return 0.0;
        }
        line_height + PADDING * 2.0
    }

    /// Rebuilds the shaped line if the text or the theme changed.
    ///
    /// The theme is part of the key because the active argument is coloured, and
    /// a theme switch has to recolour it without the text changing.
    pub fn prepare(&mut self, font_system: &mut FontSystem, theme: &Theme) {
        if !self.open {
            return;
        }
        // Rounded, not truncated: `width()` is `columns * char_width + padding`,
        // so dividing it back out lands on 46.9999 in f32 and a cast would drop
        // a column -- which shows up as an ellipsis on a label that fits.
        let columns = ((self.width() - PADDING * 2.0) / self.char_width)
            .max(4.0)
            .round() as usize;
        let key = (self.text.clone(), columns, theme.toggle_on);
        if key == self.shaped {
            return;
        }
        self.shaped = key;

        let shown = truncate(&self.text, columns);
        let default = Attrs::new()
            .family(self.family)
            .color(theme.menu_text.to_cosmic());
        let active = Attrs::new()
            .family(self.family)
            .color(theme.toggle_on.to_cosmic());
        let mut spans: Vec<(&str, Attrs)> = Vec::with_capacity(3);
        match &self.active {
            Some(range) if range.end <= shown.len() => {
                spans.push((&shown[..range.start], default.clone()));
                spans.push((&shown[range.clone()], active));
                spans.push((&shown[range.end..], default.clone()));
            }
            _ => spans.push((&shown, default.clone())),
        }
        self.body.set_rich_text(spans, &default, Shaping::Basic, None);
        self.body.shape_until_scroll(font_system, false);
    }

    /// Queues the frame.
    pub fn push_quads(&self, quads: &mut QuadRenderer, line_height: f32, theme: &Theme) {
        if !self.open {
            return;
        }
        let (left, _) = self.anchor;
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
    }

    /// The text area of the signature.
    pub fn text_areas<'a>(&'a self, line_height: f32, theme: &Theme) -> Vec<TextArea<'a>> {
        if !self.open {
            return Vec::new();
        }
        let (left, _) = self.anchor;
        let top = self.top(line_height);
        let bounds = TextBounds {
            left: left as i32,
            top: top as i32,
            right: (left + self.width()) as i32,
            bottom: (top + self.height(line_height)) as i32,
        };
        vec![TextArea {
            buffer: &self.body,
            left: left + PADDING,
            top: top + PADDING,
            scale: 1.0,
            bounds,
            default_color: theme.menu_text.to_cosmic(),
            custom_glyphs: &[],
        }]
    }
}

/// Cuts `text` to `columns` characters, marking the cut with an ellipsis.
///
/// Shared with the context menu, which has the same problem: a label longer than
/// the box has to be cut somewhere, and cutting it without a mark would read as
/// a shorter label rather than a truncated one.
pub(crate) fn truncate(text: &str, columns: usize) -> String {
    if text.chars().count() <= columns {
        return text.to_string();
    }
    let mut out: String = text.chars().take(columns.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn popup() -> CompletionPopup {
        CompletionPopup::new(Metrics::new(15.0, 20.0), Family::Monospace, 8.0)
    }

    fn items(n: usize) -> Vec<Item> {
        (0..n).map(|i| Item::word(format!("word{i}"))).collect()
    }

    fn strs(words: &[&str]) -> Vec<Item> {
        words.iter().map(|w| Item::word(*w)).collect()
    }

    #[test]
    fn the_parameter_highlight_walks_the_argument_list() {
        let mut h = hint();
        // "foo(a, b, c)": the arguments start at byte 4, 7 and 10.
        h.show("foo(a, b, c)".into(), Some(4..5), (0.0, 0.0));
        h.move_selection(1);
        assert_eq!(h.active, Some(7..8));
        h.move_selection(1);
        assert_eq!(h.active, Some(10..11));
        // Wraps at the end, and back again at the start.
        h.move_selection(1);
        assert_eq!(h.active, Some(4..5));
        h.move_selection(-1);
        assert_eq!(h.active, Some(10..11));
    }

    #[test]
    fn a_nested_comma_does_not_split_an_argument() {
        let mut h = hint();
        // The comma inside `f(x, y)` belongs to the first argument.
        h.show("g(f(x, y), z)".into(), Some(3..10), (0.0, 0.0));
        h.move_selection(1);
        assert_eq!(h.active, Some(11..12));
    }

    #[test]
    fn moving_the_highlight_on_a_closed_hint_does_nothing() {
        let mut h = hint();
        h.move_selection(1);
        assert_eq!(h.active, None);
    }

    #[test]
    fn a_new_popup_is_closed() {
        let p = popup();
        assert!(!p.is_open());
        assert_eq!(p.height(20.0), 0.0);
    }

    #[test]
    fn showing_items_opens_it() {
        let mut p = popup();
        p.show(items(3), (10.0, 20.0));
        assert!(p.is_open());
        assert_eq!(p.len(), 3);
        assert_eq!(p.selected_label(), Some("word0"));
    }

    #[test]
    fn showing_nothing_leaves_it_closed() {
        let mut p = popup();
        p.show(Vec::new(), (0.0, 0.0));
        assert!(!p.is_open());
    }

    #[test]
    fn the_height_caps_at_the_visible_row_count() {
        let mut p = popup();
        p.show(items(100), (0.0, 0.0));
        assert_eq!(p.height(20.0), 20.0 * VISIBLE_ROWS as f32 + PADDING * 2.0);
    }

    #[test]
    fn the_width_follows_the_longest_item() {
        let mut p = popup();
        p.show(vec![Item::word("a")], (0.0, 0.0));
        assert_eq!(p.width(), MIN_WIDTH);
        p.show(vec![Item::word("x".repeat(200))], (0.0, 0.0));
        assert_eq!(p.width(), MAX_WIDTH);
    }

    #[test]
    fn the_selection_wraps_at_both_ends() {
        let mut p = popup();
        p.show(items(3), (0.0, 0.0));
        p.move_selection(-1);
        assert_eq!(p.selected_label(), Some("word2"));
        p.move_selection(1);
        assert_eq!(p.selected_label(), Some("word0"));
    }

    #[test]
    fn the_selection_scrolls_into_view() {
        let mut p = popup();
        p.show(items(30), (0.0, 0.0));
        for _ in 0..VISIBLE_ROWS {
            p.move_selection(1);
        }
        assert_eq!(p.selected, VISIBLE_ROWS);
        assert_eq!(p.scroll, 1);
    }

    #[test]
    fn scrolling_back_up_follows_the_selection() {
        let mut p = popup();
        p.show(items(30), (0.0, 0.0));
        for _ in 0..15 {
            p.move_selection(1);
        }
        for _ in 0..15 {
            p.move_selection(-1);
        }
        assert_eq!(p.selected, 0);
        assert_eq!(p.scroll, 0);
    }

    #[test]
    fn item_at_maps_a_row_to_an_item() {
        let mut p = popup();
        p.show(items(5), (100.0, 50.0));
        assert_eq!(p.item_at(110.0, 50.0 + PADDING + 5.0, 20.0), Some(0));
        assert_eq!(p.item_at(110.0, 50.0 + PADDING + 25.0, 20.0), Some(1));
    }

    #[test]
    fn item_at_outside_the_box_is_none() {
        let mut p = popup();
        p.show(items(5), (100.0, 50.0));
        assert_eq!(p.item_at(10.0, 60.0, 20.0), None);
        assert_eq!(p.item_at(110.0, 10.0, 20.0), None);
    }

    #[test]
    fn closing_drops_the_list() {
        let mut p = popup();
        p.show(items(3), (0.0, 0.0));
        p.close();
        assert!(!p.is_open());
        assert!(p.is_empty());
        assert_eq!(p.selected_label(), None);
    }

    #[test]
    fn truncate_marks_the_cut() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("abc", 4), "abc");
    }

    #[test]
    fn set_selected_highlights_a_row() {
        let mut popup = popup();
        popup.show(strs(&["alpha", "beta", "gamma"]), (0.0, 0.0));
        popup.set_selected(2);
        assert_eq!(popup.selected_label(), Some("gamma"));
    }

    #[test]
    fn set_selected_ignores_an_out_of_range_index() {
        let mut popup = popup();
        popup.show(strs(&["alpha", "beta"]), (0.0, 0.0));
        popup.set_selected(9);
        assert_eq!(popup.selected_label(), Some("alpha"));
    }

    #[test]
    fn set_selected_scrolls_the_window() {
        let mut popup = popup();
        let many: Vec<Item> = (0..20).map(|i| Item::word(format!("item{i}"))).collect();
        popup.show(many, (0.0, 0.0));
        popup.set_selected(15);
        assert_eq!(popup.selected_label(), Some("item15"));
        // Row 15 must be inside the ten-row window, so the first visible row
        // is 6: the top row of the box maps to item 6.
        assert_eq!(popup.item_at(5.0, 6.0 + 5.0, 20.0), Some(6));
    }

    #[test]
    fn popup_flips_above_the_caret_when_it_would_not_fit_below() {
        let mut popup = popup();
        popup.set_viewport(500.0);
        popup.show(strs(&["alpha", "beta"]), (10.0, 470.0));
        // Two rows plus padding is 52 px, so 470 + 52 > 500 and the box moves
        // up to 470 - 52 = 418.
        assert_eq!(popup.item_at(20.0, 418.0 + 6.0 + 5.0, 20.0), Some(0));
    }

    #[test]
    fn a_word_row_inserts_what_it_shows() {
        let item = Item::word("document");
        assert_eq!(item.label, "document");
        assert_eq!(item.insert, "document");
        assert_eq!(item.back, 0);
    }

    #[test]
    fn a_function_row_shows_the_signature_and_inserts_the_call() {
        let item = Item::function("save", "path: &Path");
        assert_eq!(item.label, "save(path: &Path)");
        assert_eq!(item.insert, "save()");
        assert_eq!(item.back, 1);
    }

    #[test]
    fn a_function_with_no_parameters_still_inserts_parentheses() {
        let item = Item::function("new", "");
        assert_eq!(item.label, "new()");
        assert_eq!(item.insert, "new()");
    }

    #[test]
    fn every_mode_has_a_label() {
        for mode in [
            CompletionMode::Word,
            CompletionMode::Document,
            CompletionMode::Function,
            CompletionMode::Path,
        ] {
            assert!(!mode.label().is_empty());
        }
    }

    #[test]
    fn word_modes_refilter_on_word_characters_only() {
        assert!(CompletionMode::Word.refilters_on('a'));
        assert!(CompletionMode::Word.refilters_on('_'));
        assert!(!CompletionMode::Word.refilters_on('('));
        assert!(!CompletionMode::Word.refilters_on(' '));
    }

    #[test]
    fn path_mode_refilters_on_punctuation() {
        assert!(CompletionMode::Path.refilters_on('.'));
        assert!(CompletionMode::Path.refilters_on('-'));
        assert!(!CompletionMode::Path.refilters_on(' '));
    }

    fn hint() -> ParamHint {
        ParamHint::new(Metrics::new(15.0, 20.0), Family::Monospace, 8.0)
    }

    #[test]
    fn a_new_hint_is_closed() {
        let h = hint();
        assert!(!h.is_open());
        assert_eq!(h.height(20.0), 0.0);
    }

    #[test]
    fn showing_a_signature_opens_the_hint() {
        let mut h = hint();
        h.show("save(path: &Path)".to_string(), Some(5..9), (10.0, 20.0));
        assert!(h.is_open());
        assert_eq!(h.text(), "save(path: &Path)");
    }

    #[test]
    fn showing_nothing_leaves_the_hint_closed() {
        let mut h = hint();
        h.show(String::new(), None, (0.0, 0.0));
        assert!(!h.is_open());
    }

    #[test]
    fn closing_hides_the_hint() {
        let mut h = hint();
        h.show("save()".to_string(), None, (0.0, 0.0));
        h.close();
        assert!(!h.is_open());
        assert_eq!(h.text(), "");
    }

    #[test]
    fn the_hint_is_one_line_plus_its_padding() {
        let mut h = hint();
        h.show("save()".to_string(), None, (0.0, 0.0));
        assert_eq!(h.height(20.0), 20.0 + PADDING * 2.0);
    }

    #[test]
    fn the_hint_width_follows_the_signature() {
        let mut h = hint();
        h.show("a".to_string(), None, (0.0, 0.0));
        assert_eq!(h.width(), MIN_WIDTH);
        h.show("x".repeat(200), None, (0.0, 0.0));
        assert_eq!(h.width(), MAX_WIDTH);
    }

    #[test]
    fn the_hint_flips_above_the_caret_when_it_would_not_fit_below() {
        let mut h = hint();
        h.set_viewport(500.0);
        h.show("save()".to_string(), None, (10.0, 490.0));
        // One row plus padding is 32 px, so 490 + 32 > 500 and the box moves up
        // to 490 - 32 = 458. The frame's top edge is what the quad covers.
        assert_eq!(h.top(20.0), 458.0);
    }

    #[test]
    fn the_hint_stays_below_the_caret_when_it_fits() {
        let mut h = hint();
        h.set_viewport(500.0);
        h.show("save()".to_string(), None, (10.0, 100.0));
        assert_eq!(h.top(20.0), 100.0);
    }

    #[test]
    fn popup_stays_below_the_caret_when_it_fits() {
        let mut popup = popup();
        popup.set_viewport(500.0);
        popup.show(strs(&["alpha", "beta"]), (10.0, 100.0));
        assert_eq!(popup.item_at(20.0, 100.0 + 6.0 + 5.0, 20.0), Some(0));
    }
}
