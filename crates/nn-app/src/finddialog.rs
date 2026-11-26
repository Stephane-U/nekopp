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

//! The floating Find window.
//!
//! Notepad++ puts its search in a dialog: a sheet of tabs, two combo boxes, a
//! grid of checkboxes, two radio groups, a transparency slider and a row of
//! buttons. This is that sheet, drawn as a floating panel over the document
//! rather than as a band under it -- the alternative presentation of the same
//! search, chosen by the "Floating Search" preference.
//!
//! It owns no search state. The pattern, the replacement, the options, the
//! matches and the history all live in [`FindBar`], which is what makes the
//! preference a presentation switch rather than a second search: closing the
//! dialog and reopening the bar shows the pattern that was typed in it. What
//! this module owns is the geometry, the shaped labels and the tab that is
//! showing, and it answers clicks with [`Hit`] values so that the input handler
//! that already exists -- the one that knows what Mark All and Replace All do --
//! needs no second copy.
//!
//! A floating window is not modal here, and Notepad++'s dialog is not modal
//! either: you edit the text with it open, which is the point of Find Next. So a
//! click outside leaves the dialog up and lands in the document, and `Escape` is
//! what closes it.

use std::collections::BTreeMap;

use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping};
use glyphon::{TextArea, TextBounds};
use nn_core::SearchMode;

use crate::findbar::{Button, Chip, Field, FindBar, Hit, Rect, BUTTONS, SWAP_CHIPS};
use crate::i18n::Catalog;
use crate::quad::QuadRenderer;
use crate::theme::{mix, Rgb, Theme, ToGpu};

/// Padding inside the frame, on all four sides.
const PAD: f32 = 12.0;
/// Padding inside a button, left and right of its word.
const BUTTON_PAD: f32 = 10.0;
/// The narrowest a button is allowed to be.
const BUTTON_MIN: f32 = 56.0;
/// The checkbox square, and the gap between it and its word.
const BOX: f32 = 13.0;
const BOX_GAP: f32 = 7.0;
/// Width of a tab, and of the gap between two.
const TAB_W: f32 = 84.0;
const TAB_GAP: f32 = 2.0;
/// Width of the arrow that drops a field's history.
const ARROW_W: f32 = 18.0;
/// Width of one swap button.
const SWAP_W: f32 = 26.0;
/// The gap between two controls on a row.
const GAP: f32 = 6.0;
/// The dialog's width, and the narrowest it is squeezed to in a small window.
const WIDTH: f32 = 476.0;
const MIN_WIDTH: f32 = 360.0;
/// The slider's track.
const SLIDER_W: f32 = 150.0;
const SLIDER_H: f32 = 6.0;
/// The most transparent the frame gets: an invisible dialog is a lost dialog.
pub const MAX_TRANSPARENCY: f32 = 0.9;
/// How many entries of an open history list are drawn at once.
const VISIBLE_HISTORY: usize = 8;
/// The glyph on the arrow that drops a field's history.
const ARROW: &str = "\u{25be}";

/// One of the dialog's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// The pattern and the search options.
    Find,
    /// The same, plus the replacement row.
    Replace,
    /// The pattern, the two marking options and the marking buttons.
    Mark,
}

/// The tabs, in the order they are drawn.
pub const TABS: [Tab; 3] = [Tab::Find, Tab::Replace, Tab::Mark];

impl Tab {
    /// The word on the tab, and in the title bar.
    pub fn label(self) -> &'static str {
        match self {
            Tab::Find => "Find",
            Tab::Replace => "Replace",
            Tab::Mark => "Mark",
        }
    }

    /// Whether the tab shows the replacement row.
    pub fn replaces(self) -> bool {
        matches!(self, Tab::Replace)
    }

    /// The buttons the tab shows, in Notepad++'s order.
    pub fn buttons(self) -> Vec<Button> {
        match self {
            Tab::Find => vec![
                Button::FindNext,
                Button::Count,
                Button::FindAllAll,
                Button::FindAll,
                Button::Close,
            ],
            Tab::Replace => vec![
                Button::FindNext,
                Button::Replace,
                Button::ReplaceAll,
                Button::ReplaceAllAll,
                Button::Close,
            ],
            Tab::Mark => vec![Button::MarkAll, Button::ClearMarks, Button::Close],
        }
    }
}

/// One of the dialog's checkboxes.
///
/// Each is a switch the bar draws as a chip. A checkbox needs a word where a
/// chip carries a mark, which is why the two presentations name the same
/// settings separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    /// Match whole word only.
    WholeWord,
    /// Match case.
    MatchCase,
    /// Wrap around.
    Wrap,
    /// In selection.
    InSelection,
    /// Bookmark line, from the Mark tab.
    BookmarkLine,
    /// Purge for each search, from the Mark tab.
    PurgeMarks,
    /// ". matches newline", which only a regex mode can honour.
    Newline,
}

/// The checkboxes that belong to a tab, in the order they are drawn.
pub const FIND_CHECKS: [Check; 2] = [Check::WholeWord, Check::MatchCase];
pub const REPLACE_CHECKS: [Check; 4] = [
    Check::WholeWord,
    Check::MatchCase,
    Check::Wrap,
    Check::InSelection,
];
pub const MARK_CHECKS: [Check; 4] = [
    Check::WholeWord,
    Check::MatchCase,
    Check::BookmarkLine,
    Check::PurgeMarks,
];

impl Check {
    /// The word beside the box.
    pub fn label(self) -> &'static str {
        match self {
            Check::WholeWord => "Match whole word only",
            Check::MatchCase => "Match case",
            Check::Wrap => "Wrap around",
            Check::InSelection => "In selection",
            Check::BookmarkLine => "Bookmark line",
            Check::PurgeMarks => "Purge for each search",
            Check::Newline => ". matches newline",
        }
    }

    /// The chip this box stands for.
    ///
    /// "Match whole word only" and ". matches newline" share one chip in the
    /// bar, because a whole word is not a property a regular expression has and
    /// the two can never both be meaningful. The dialog shows both boxes and
    /// hands both to that one chip: in regex mode the chip toggles the newline
    /// option, and outside it the whole-word one, which is exactly what the two
    /// boxes claim.
    pub fn chip(self) -> Chip {
        match self {
            Check::WholeWord | Check::Newline => Chip::Word,
            Check::MatchCase => Chip::Case,
            Check::Wrap => Chip::Wrap,
            Check::InSelection => Chip::Selection,
            Check::BookmarkLine => Chip::Bookmark,
            Check::PurgeMarks => Chip::Purge,
        }
    }

    /// Whether the box is ticked.
    pub fn is_on(self, find: &FindBar) -> bool {
        match self {
            Check::WholeWord => find.options.whole_word,
            Check::MatchCase => find.options.case_sensitive,
            Check::Wrap => find.wrap,
            Check::InSelection => find.in_selection,
            Check::BookmarkLine => find.bookmark_lines,
            Check::PurgeMarks => find.purge_marks,
            Check::Newline => find.options.dot_matches_newline,
        }
    }

    /// Whether the box is greyed out in the current mode.
    ///
    /// Notepad++ disables "Match whole word only" once the pattern is a regular
    /// expression and ". matches newline" unless it is one. The boxes are drawn
    /// dimmed rather than removed, so the sheet does not reflow when the mode
    /// changes under the pointer.
    pub fn disabled(self, find: &FindBar) -> bool {
        let regex = find.options.mode == SearchMode::Regex;
        match self {
            Check::WholeWord => regex,
            Check::Newline => !regex,
            _ => false,
        }
    }
}

/// What a click on the dialog landed on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DialogHit {
    /// One of the controls the bar already knows how to act on.
    Find(Hit),
    /// A tab.
    Tab(Tab),
    /// The title bar, which drags the window.
    Title,
    /// The transparency slider, at this fraction of its track.
    Transparency(f32),
}

/// The direction radios, as `(up, word)`.
const DIRECTIONS: [(bool, &str); 2] = [(true, "Up"), (false, "Down")];
/// The search mode radios.
const MODES: [(SearchMode, &str); 3] = [
    (SearchMode::Normal, "Normal"),
    (SearchMode::Extended, "Extended"),
    (SearchMode::Regex, "Regular expression"),
];

/// Where everything in the dialog sits.
pub struct DialogLayout {
    /// The frame.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// The title bar, which is the drag handle.
    pub title: Rect,
    /// The three tabs.
    pub tabs: [Rect; 3],
    /// Left edge of the field labels.
    pub label_x: f32,
    /// The pattern field and its history arrow.
    pub field_query: Rect,
    pub query_toggle: Rect,
    /// The replacement field and its arrow, on the Replace tab only.
    pub field_replace: Option<Rect>,
    pub replace_toggle: Option<Rect>,
    /// The three swap buttons, on the Replace tab only.
    pub swaps: [Rect; 3],
    /// Every checkbox with the rectangle it answers to, word included.
    pub checks: Vec<(Check, Rect)>,
    /// The Direction caption and its two radios.
    pub direction_caption: Rect,
    pub directions: [(bool, Rect); 2],
    /// The Search Mode caption and its three radios.
    pub mode_caption: Rect,
    pub modes: [(SearchMode, Rect); 3],
    /// ". matches newline".
    pub newline: Rect,
    /// The Transparency caption and its track.
    pub transparency_caption: Rect,
    pub transparency: Rect,
    /// The buttons, with the words they carry.
    pub buttons: Vec<(Button, Rect)>,
    /// The rows of an open history list, nearest the field first.
    pub history: Vec<Rect>,
}

impl DialogLayout {
    /// Every rectangle the frame holds, for the shift and for the tests.
    fn rects_mut(&mut self) -> Vec<&mut Rect> {
        let mut rects: Vec<&mut Rect> = vec![&mut self.title];
        rects.extend(self.tabs.iter_mut());
        rects.push(&mut self.field_query);
        rects.push(&mut self.query_toggle);
        if let Some(rect) = self.field_replace.as_mut() {
            rects.push(rect);
        }
        if let Some(rect) = self.replace_toggle.as_mut() {
            rects.push(rect);
        }
        rects.extend(self.swaps.iter_mut());
        rects.extend(self.checks.iter_mut().map(|(_, rect)| rect));
        rects.push(&mut self.direction_caption);
        rects.extend(self.directions.iter_mut().map(|(_, rect)| rect));
        rects.push(&mut self.mode_caption);
        rects.extend(self.modes.iter_mut().map(|(_, rect)| rect));
        rects.push(&mut self.newline);
        rects.push(&mut self.transparency_caption);
        rects.push(&mut self.transparency);
        rects.extend(self.buttons.iter_mut().map(|(_, rect)| rect));
        rects.extend(self.history.iter_mut());
        rects
    }

    /// Whether `(x, y)` is anywhere on the frame.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// An empty rectangle, for the slots a tab does not use.
fn zero() -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        w: 0.0,
        h: 0.0,
    }
}

/// The floating Find window.
///
/// One is built at startup and lives for the session, like the bar: the tab, the
/// position and the transparency are remembered between openings, which is what
/// a dialog does.
pub struct FindDialog {
    /// The window size, for centring and for keeping the frame on screen.
    viewport: (f32, f32),
    /// Which tab is showing.
    tab: Tab,
    /// The frame's top-left corner, once it has been dragged.
    pos: Option<(f32, f32)>,
    /// Where the pointer grabbed the title bar, relative to the frame's corner.
    drag: Option<(f32, f32)>,
    /// The transparency slider, from 0 (opaque) to the maximum.
    transparency: f32,
    family: Family<'static>,
    metrics: Metrics,
    char_width: f32,
    /// The shaped labels, keyed by their English text.
    labels: BTreeMap<&'static str, Buffer>,
    /// The width of the widest field label, measured once shaped.
    label_w: f32,
    /// The width of each button's word, measured once shaped.
    button_w: [f32; BUTTONS.len()],
    /// The rows of the open history list.
    history_rows: Vec<Buffer>,
    /// The entry text each row was shaped from.
    history_shaped: Vec<String>,
    /// Whether the labels have to be shaped again, which is when they are first
    /// drawn and whenever the language changes.
    dirty: bool,
    /// The localization the dialog's words come from.
    catalog: Option<Catalog>,
}

impl FindDialog {
    /// Builds a closed dialog on the Find tab.
    pub fn new(metrics: Metrics, family: Family<'static>, char_width: f32) -> Self {
        let mut labels: BTreeMap<&'static str, Buffer> = BTreeMap::new();
        let mut add = |word: &'static str| {
            let mut buffer = Buffer::new_empty(metrics);
            buffer.set_size(None, None);
            buffer.set_text(word, &Attrs::new().family(family), Shaping::Basic, None);
            labels.insert(word, buffer);
        };
        for word in Self::labels() {
            add(word);
        }
        // The buttons' words are the bar's, and `labels()` leaves them out so the
        // coverage test counts them once. They still need a buffer here: the
        // sheet draws them on its own row, and `prepare` fits the translation the
        // bar is currently using.
        for button in BUTTONS {
            add(button.label());
        }
        // The marks the dialog draws itself: the swap buttons' and the arrow
        // that drops a field's history. They are not translated, so they are not
        // in `labels()` -- `is_literal` is about single characters, and these are
        // marks rather than words.
        for chip in SWAP_CHIPS {
            add(chip.mark());
        }
        add("\u{25be}");
        Self {
            viewport: (0.0, 0.0),
            tab: Tab::Find,
            pos: None,
            drag: None,
            transparency: 0.0,
            family,
            metrics,
            char_width,
            labels,
            label_w: 70.0,
            button_w: [0.0; BUTTONS.len()],
            history_rows: Vec::new(),
            history_shaped: Vec::new(),
            dirty: true,
            catalog: None,
        }
    }

    /// Rebuilds the dialog at a new display density.
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
        for row in &mut self.history_rows {
            row.set_metrics(metrics);
        }
    }

    /// Every label this dialog can draw, for the coverage test.
    ///
    /// The buttons are absent: they are the bar's, and the bar counts them once.
    pub fn labels() -> Vec<&'static str> {
        let mut labels: Vec<&'static str> = TABS.iter().map(|tab| tab.label()).collect();
        labels.extend([
            "Find what:",
            "Replace with:",
            "Direction",
            "Search Mode",
            "Transparency",
        ]);
        labels.extend(
            FIND_CHECKS
                .iter()
                .chain(&REPLACE_CHECKS)
                .chain(&MARK_CHECKS)
                .map(|check| check.label()),
        );
        labels.push(Check::Newline.label());
        labels.extend(DIRECTIONS.map(|(_, word)| word));
        labels.extend(MODES.map(|(_, word)| word));
        labels.sort_unstable();
        labels.dedup();
        labels
    }

    /// Swaps in a localization, or clears it with `None`.
    pub fn set_catalog(&mut self, catalog: Option<Catalog>) {
        self.catalog = catalog;
        self.dirty = true;
    }

    /// Records the window size, so the frame can be centred and kept inside it.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.viewport = (width, height);
    }

    /// Whether the dialog is the thing on screen.
    ///
    /// The bar owns both flags: one search, one open state, two presentations.
    pub fn is_open(&self, find: &FindBar) -> bool {
        find.open && find.floating()
    }

    /// Which tab is showing.
    pub fn tab(&self) -> Tab {
        self.tab
    }

    /// Shows a tab.
    pub fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
    }

    /// How transparent the frame is, from 0 to the maximum.
    pub fn transparency(&self) -> f32 {
        self.transparency
    }

    /// Sets the transparency, clamped to the slider's range.
    pub fn set_transparency(&mut self, value: f32) {
        self.transparency = value.clamp(0.0, MAX_TRANSPARENCY);
    }

    /// Grabs the title bar at `(x, y)`.
    pub fn begin_drag(&mut self, x: f32, y: f32) {
        let (left, top) = self.corner();
        self.drag = Some((x - left, y - top));
    }

    /// Whether a drag is in progress.
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Moves the frame with the pointer.
    pub fn drag_to(&mut self, x: f32, y: f32) {
        let Some((dx, dy)) = self.drag else {
            return;
        };
        self.pos = Some((x - dx, y - dy));
    }

    /// Lets go of the title bar.
    pub fn end_drag(&mut self) {
        self.drag = None;
    }

    /// Where the frame's top-left corner is.
    ///
    /// Clamped every time it is read rather than once when it is set: a window
    /// that shrinks must not leave the frame hanging off the edge where it cannot
    /// be dragged back.
    fn corner(&self) -> (f32, f32) {
        let (width, height) = self.measure();
        match self.pos {
            Some((x, y)) => (
                x.min((self.viewport.0 - width).max(0.0)).max(0.0),
                y.min((self.viewport.1 - height - PAD).max(0.0))
                    .max(0.0),
            ),
            None => (
                ((self.viewport.0 - width) / 2.0).max(0.0),
                ((self.viewport.1 - height) / 2.5).max(0.0),
            ),
        }
    }

    /// The frame's width and height.
    ///
    /// Measured with the tallest tab laid out, so the frame does not change
    /// height under the pointer when a tab is switched: a dialog that jumps is a
    /// dialog that moved the button you were aiming at.
    fn measure(&self) -> (f32, f32) {
        // Every tab, not just Replace: the Find tab's five buttons wrap onto
        // three rows where Replace's take two, and measuring one tab would leave
        // the other's buttons hanging below the frame.
        let height = TABS.iter().map(|tab| self.rows(*tab)).fold(0.0, f32::max);
        (self.width(), height)
    }

    /// The height the sheet needs, from the top of the frame to its bottom edge.
    fn rows(&self, tab: Tab) -> f32 {
        let row = self.metrics.line_height;
        let title = row + 6.0;
        let tabs = row + 8.0;
        let mut y = title + tabs + PAD;
        // The pattern row, and the replacement row on the Replace tab.
        y += row + GAP;
        if tab.replaces() {
            y += row + GAP;
        }
        // Two checkboxes to a row.
        let checks = match tab {
            Tab::Find => FIND_CHECKS.len(),
            Tab::Replace => REPLACE_CHECKS.len(),
            Tab::Mark => MARK_CHECKS.len(),
        };
        y += checks.div_ceil(2) as f32 * (row + 2.0) + GAP;
        // Direction and Search Mode side by side, plus the newline box under
        // them: the taller of the two columns is the Search Mode one.
        y += row + 2.0 + 3.0 * (row + 2.0) + row + 2.0;
        y += GAP;
        // The transparency row.
        y += row + 4.0;
        // The buttons, in as many rows as the width needs.
        let rows = self.button_rows(tab).len() as f32;
        y += rows * (row + 4.0) + (rows - 1.0) * GAP + PAD;
        y
    }

    /// The buttons of `tab`, in the rows they wrap onto.
    ///
    /// The words are the bar's, so a translation decides how wide a button is,
    /// and the five buttons of the Find tab do not fit 476 pixels in English --
    /// let alone in a language that spells out more. Wrapping is what keeps
    /// Close inside the frame instead of past its right edge.
    fn button_rows(&self, tab: Tab) -> Vec<Vec<Button>> {
        let inner = self.width() - 2.0 * PAD;
        let mut rows: Vec<Vec<Button>> = Vec::new();
        let mut row: Vec<Button> = Vec::new();
        let mut used = 0.0;
        for button in tab.buttons() {
            let width = self.button_width(button);
            if !row.is_empty() && used + GAP + width > inner {
                rows.push(std::mem::take(&mut row));
                used = 0.0;
            }
            used = if row.is_empty() { width } else { used + GAP + width };
            row.push(button);
        }
        if !row.is_empty() {
            rows.push(row);
        }
        rows
    }

    /// The frame's width at this window size.
    fn width(&self) -> f32 {
        // Both numbers are scale-1: a dense display gets a frame wide enough for
        // the text it now draws, up to the window it has to fit in.
        let scale = crate::renderer::scale_of(self.metrics);
        (WIDTH * scale).min((self.viewport.0 - 2.0 * PAD).max(MIN_WIDTH * scale))
    }

    /// The width a button needs for the word on it.
    pub fn button_width(&self, button: Button) -> f32 {
        let index = BUTTONS.iter().position(|one| *one == button).unwrap_or(0);
        (self.button_w[index] + 2.0 * BUTTON_PAD).max(BUTTON_MIN)
    }

    /// The widest of the two field labels.
    pub fn label_width(&self) -> f32 {
        self.label_w
    }

    /// How many rows of an open history list are drawn.
    pub fn visible_history(&self) -> usize {
        VISIBLE_HISTORY
    }
}

impl FindDialog {
    /// Lays the sheet out with its top-left corner at the origin.
    fn sheet(&self, find: &FindBar, line_height: f32) -> DialogLayout {
        let tab = self.tab;
        let w = self.width();
        let row = line_height;
        let title_h = row + 6.0;
        let tabs_h = row + 8.0;
        let mut y = title_h + tabs_h;

        let title = Rect {
            x: 0.0,
            y: 0.0,
            w,
            h: title_h,
        };
        let mut tabs = [zero(); 3];
        let mut tab_x = PAD * 0.5;
        for rect in tabs.iter_mut() {
            *rect = Rect {
                x: tab_x,
                y: title_h,
                w: TAB_W,
                h: tabs_h,
            };
            tab_x += TAB_W + TAB_GAP;
        }

        y += PAD;
        let label_x = PAD;
        let field_x = PAD + self.label_w;
        let body_right = w - PAD;

        // The swap buttons sit at the right end of the replacement row, the way
        // the dialog's pull-down does.
        let swap_total = 3.0 * SWAP_W + 2.0 * 4.0;
        let swaps_right = body_right - swap_total - GAP;
        let mut swaps = [zero(); 3];
        if tab.replaces() {
            let mut swap_x = swaps_right;
            for rect in swaps.iter_mut() {
                *rect = Rect {
                    x: swap_x,
                    y: y + row + GAP + 2.0,
                    w: SWAP_W,
                    h: row - 4.0,
                };
                swap_x += SWAP_W + 4.0;
            }
        }

        // Each row ends at the little arrow that drops its history.
        let query_right = body_right - ARROW_W - GAP;
        let field_query = Rect {
            x: field_x,
            y,
            w: (query_right - field_x).max(60.0),
            h: row,
        };
        let query_toggle = Rect {
            x: field_query.x + field_query.w + GAP,
            y: y + 2.0,
            w: ARROW_W,
            h: row - 4.0,
        };
        y += row + GAP;

        let mut field_replace = None;
        let mut replace_toggle = None;
        if tab.replaces() {
            let right = swaps_right - GAP;
            let field = Rect {
                x: field_x,
                y,
                w: (right - ARROW_W - GAP - field_x).max(60.0),
                h: row,
            };
            field_replace = Some(field);
            replace_toggle = Some(Rect {
                x: field.x + field.w + GAP,
                y: y + 2.0,
                w: ARROW_W,
                h: row - 4.0,
            });
            y += row + GAP;
        }

        // The checkboxes fill two columns, left to right then down.
        let checks_for: &[Check] = match tab {
            Tab::Find => &FIND_CHECKS,
            Tab::Replace => &REPLACE_CHECKS,
            Tab::Mark => &MARK_CHECKS,
        };
        let column = (body_right - PAD) / 2.0;
        let mut checks = Vec::with_capacity(checks_for.len());
        for (index, check) in checks_for.iter().enumerate() {
            let column_x = PAD + (index % 2) as f32 * column;
            let column_y = y + (index / 2) as f32 * (row + 2.0);
            let width = match self.labels.get(check.label()) {
                Some(buffer) => widest_line(buffer) + BOX + BOX_GAP + BOX_GAP,
                None => column - 4.0,
            };
            checks.push((
                *check,
                Rect {
                    x: column_x,
                    y: column_y,
                    w: width.min(column - 4.0),
                    h: row,
                },
            ));
        }
        y += checks_for.len().div_ceil(2) as f32 * (row + 2.0) + GAP;

        // The two radio groups share the row: Direction on the left, Search Mode
        // on the right, which is the dialog's own arrangement.
        let direction_caption = Rect {
            x: PAD,
            y,
            w: column - 8.0,
            h: row,
        };
        let mut directions = [(true, zero()); 2];
        let mut direction_y = y + row + 2.0;
        for (index, (up, _)) in DIRECTIONS.iter().enumerate() {
            directions[index] = (
                *up,
                Rect {
                    x: PAD,
                    y: direction_y,
                    w: (column - 8.0).min(90.0),
                    h: row,
                },
            );
            direction_y += row + 2.0;
        }
        let mode_caption = Rect {
            x: PAD + column,
            y,
            w: column - 8.0,
            h: row,
        };
        let mut modes = [(SearchMode::Normal, zero()); 3];
        let mut mode_y = y + row + 2.0;
        for (index, (mode, _)) in MODES.iter().enumerate() {
            modes[index] = (
                *mode,
                Rect {
                    x: PAD + column,
                    y: mode_y,
                    w: column - 8.0,
                    h: row,
                },
            );
            mode_y += row + 2.0;
        }
        let newline = Rect {
            x: PAD + column,
            y: mode_y,
            w: column - 8.0,
            h: row,
        };
        y = mode_y + row + 2.0 + GAP;

        let transparency_caption = Rect {
            x: PAD,
            y,
            w: self.label_w.max(90.0),
            h: row,
        };
        let track_x = PAD + transparency_caption.w + GAP;
        let transparency = Rect {
            x: track_x,
            y: y + (row - SLIDER_H) / 2.0,
            w: SLIDER_W.min((body_right - track_x).max(60.0)),
            h: SLIDER_H,
        };
        y += row + 4.0;

        // The buttons along the bottom, each row right-aligned so Close stays in
        // the corner however long a translation makes the others.
        let mut buttons = Vec::new();
        let button_rows = self.button_rows(tab);
        let last_row = button_rows.len().saturating_sub(1);
        for (index, row_buttons) in button_rows.iter().enumerate() {
            let widths: Vec<f32> = row_buttons.iter().map(|b| self.button_width(*b)).collect();
            let mut total: f32 = widths.iter().sum();
            total += GAP * (widths.len().saturating_sub(1)) as f32;
            let mut button_x = (body_right - total).max(PAD);
            for (button, width) in row_buttons.iter().zip(widths) {
                buttons.push((
                    *button,
                    Rect {
                        x: button_x,
                        y,
                        w: width,
                        h: row + 4.0,
                    },
                ));
                button_x += width + GAP;
            }
            y += row + 4.0;
            if index != last_row {
                y += GAP;
            }
        }
        y += PAD;

        // The open history list, dropping from its field over the sheet.
        let mut history = Vec::new();
        if let Some(field) = find.history_open() {
            let anchor = match field {
                Field::Query => field_query,
                Field::Replacement => field_replace.unwrap_or(field_query),
            };
            let count = find.history_entries(field).len().min(VISIBLE_HISTORY);
            for index in 0..count {
                history.push(Rect {
                    x: anchor.x,
                    y: anchor.y + (index as f32 + 1.0) * row,
                    w: anchor.w,
                    h: row,
                });
            }
        }

        DialogLayout {
            x: 0.0,
            y: 0.0,
            w,
            h: y,
            title,
            tabs,
            label_x,
            field_query,
            query_toggle,
            field_replace,
            replace_toggle,
            swaps,
            checks,
            direction_caption,
            directions,
            mode_caption,
            modes,
            newline,
            transparency_caption,
            transparency,
            buttons,
            history,
        }
    }

    /// The sheet in window coordinates.
    pub fn layout(&self, find: &FindBar, line_height: f32) -> DialogLayout {
        let mut layout = self.sheet(find, line_height);
        let (left, top) = self.corner();
        layout.x = left;
        layout.y = top;
        for rect in layout.rects_mut() {
            rect.x += left;
            rect.y += top;
        }
        // The captions are positions rather than rectangles, so they are moved
        // here or they stay at the window's left edge, outside the frame.
        layout.label_x += left;
        layout
    }

    /// What a click at `(x, y)` lands on, if anything.
    ///
    /// The geometry is the one `push_quads` draws, so a box cannot be clickable
    /// somewhere it is not ticked.
    pub fn hit_at(&self, find: &FindBar, x: f32, y: f32, line_height: f32) -> Option<DialogHit> {
        if !self.is_open(find) {
            return None;
        }
        let layout = self.layout(find, line_height);
        // The open list is over the sheet, so it is tested first.
        for (index, rect) in layout.history.iter().enumerate() {
            if rect.contains(x, y) {
                return Some(DialogHit::Find(Hit::HistoryEntry(index)));
            }
        }
        for (index, rect) in layout.tabs.iter().enumerate() {
            if rect.contains(x, y) {
                return Some(DialogHit::Tab(TABS[index]));
            }
        }
        for (button, rect) in &layout.buttons {
            if rect.contains(x, y) {
                return Some(DialogHit::Find(Hit::Button(*button)));
            }
        }
        if let Some(rect) = layout.field_replace {
            if rect.contains(x, y) {
                return Some(DialogHit::Find(Hit::Field(Field::Replacement)));
            }
        }
        if layout.field_replace.is_some() {
            for (index, rect) in layout.swaps.iter().enumerate() {
                if rect.contains(x, y) {
                    return Some(DialogHit::Find(Hit::Swap(SWAP_CHIPS[index])));
                }
            }
        }
        if layout.field_query.contains(x, y) {
            return Some(DialogHit::Find(Hit::Field(Field::Query)));
        }
        if layout.query_toggle.contains(x, y) {
            return Some(DialogHit::Find(Hit::HistoryToggle(Field::Query)));
        }
        if let Some(rect) = layout.replace_toggle {
            if rect.contains(x, y) {
                return Some(DialogHit::Find(Hit::HistoryToggle(Field::Replacement)));
            }
        }
        for (check, rect) in &layout.checks {
            if rect.contains(x, y) {
                return Some(DialogHit::Find(Hit::Chip(check.chip())));
            }
        }
        if layout.newline.contains(x, y) {
            return Some(DialogHit::Find(Hit::Chip(Chip::Word)));
        }
        for (up, rect) in &layout.directions {
            if rect.contains(x, y) {
                let chip = if *up { Chip::Up } else { Chip::Down };
                return Some(DialogHit::Find(Hit::Chip(chip)));
            }
        }
        for (mode, rect) in &layout.modes {
            if rect.contains(x, y) {
                let chip = match mode {
                    SearchMode::Normal => Chip::Normal,
                    SearchMode::Extended => Chip::Extended,
                    SearchMode::Regex => Chip::Regex,
                };
                return Some(DialogHit::Find(Hit::Chip(chip)));
            }
        }
        // The slider is grabbable a few pixels above and below its track, which
        // is what makes a six-pixel line usable with a mouse.
        let track = layout.transparency;
        if x >= track.x
            && x < track.x + track.w
            && y >= track.y - GAP
            && y < track.y + track.h + GAP
        {
            let fraction = ((x - track.x) / track.w).clamp(0.0, 1.0);
            return Some(DialogHit::Transparency(fraction));
        }
        if layout.title.contains(x, y) {
            return Some(DialogHit::Title);
        }
        None
    }

    /// Whether `(x, y)` is on the frame.
    pub fn contains(&self, find: &FindBar, x: f32, y: f32, line_height: f32) -> bool {
        if !self.is_open(find) {
            return false;
        }
        self.layout(find, line_height).contains(x, y)
    }
}

impl FindDialog {
    /// Rebuilds the shaped buffers.
    ///
    /// The button words come from the bar, which already translates them: a
    /// second copy of that table here would be a second thing to keep in step.
    pub fn prepare(&mut self, font_system: &mut FontSystem, find: &FindBar) {
        // The open history list first: it changes as the list is opened and
        // closed, which is more often than the labels do.
        let entries: Vec<String> = match find.history_open() {
            Some(field) => find
                .history_entries(field)
                .iter()
                .take(VISIBLE_HISTORY)
                .cloned()
                .collect(),
            None => Vec::new(),
        };
        if entries != self.history_shaped {
            self.history_rows
                .resize_with(entries.len(), || Buffer::new_empty(self.metrics));
            for (buffer, text) in self.history_rows.iter_mut().zip(&entries) {
                buffer.set_size(None, None);
                buffer.set_text(text, &Attrs::new().family(self.family), Shaping::Basic, None);
                buffer.shape_until_scroll(font_system, false);
            }
            self.history_shaped = entries;
        }
        if !self.dirty {
            return;
        }
        self.dirty = false;
        for (english, buffer) in self.labels.iter_mut() {
            let word = match &self.catalog {
                Some(catalog) => catalog.tr(english).to_string(),
                None => (*english).to_string(),
            };
            buffer.set_text(&word, &Attrs::new().family(self.family), Shaping::Basic, None);
            buffer.shape_until_scroll(font_system, false);
        }
        // The button words are the bar's, which is where the translation lives.
        for (index, button) in BUTTONS.iter().enumerate() {
            let word = find.button_text(*button).to_string();
            if let Some(buffer) = self.labels.get_mut(button.label()) {
                buffer.set_text(&word, &Attrs::new().family(self.family), Shaping::Basic, None);
                buffer.shape_until_scroll(font_system, false);
                self.button_w[index] = widest_line(buffer);
            }
        }
        self.label_w = widest_line(&self.labels["Find what:"])
            .max(widest_line(&self.labels["Replace with:"]))
            + GAP
            + 4.0;
    }

    /// The alpha the sheet is drawn at.
    fn alpha(&self) -> f32 {
        1.0 - self.transparency
    }

    /// Queues the frame, the tabs, the boxes, the radios and the slider.
    pub fn push_quads(
        &self,
        quads: &mut QuadRenderer,
        find: &FindBar,
        line_height: f32,
        theme: &Theme,
    ) {
        if !self.is_open(find) {
            return;
        }
        let layout = self.layout(find, line_height);
        let alpha = self.alpha();
        let border = theme.menu_border.to_linear_alpha(alpha);
        // A drop shadow, so the frame reads as floating over the text rather
        // than as another band of it.
        quads.push(
            layout.x + 3.0,
            layout.y + 3.0,
            layout.w,
            layout.h,
            [0.0, 0.0, 0.0, 0.35 * alpha],
        );
        quads.push(
            layout.x,
            layout.y,
            layout.w,
            layout.h,
            theme.menu_background.to_linear_alpha(alpha),
        );
        for (x, y, w, h) in [
            (layout.x, layout.y, layout.w, 1.0),
            (layout.x, layout.y + layout.h - 1.0, layout.w, 1.0),
            (layout.x, layout.y, 1.0, layout.h),
            (layout.x + layout.w - 1.0, layout.y, 1.0, layout.h),
        ] {
            quads.push(x, y, w, h, border);
        }

        // The title bar, and the lit tab under it.
        let caption = mix(theme.menu_background, theme.foreground, 0.10).to_linear_alpha(alpha);
        quads.push(
            layout.title.x,
            layout.title.y,
            layout.title.w,
            layout.title.h,
            caption,
        );
        let strip = mix(theme.menu_background, theme.foreground, 0.04).to_linear_alpha(alpha);
        let first = layout.tabs[0];
        let last = layout.tabs[2];
        quads.push(
            first.x,
            first.y,
            last.x + last.w - first.x,
            first.h,
            strip,
        );
        let active = layout.tabs[TABS.iter().position(|tab| *tab == self.tab).unwrap_or(0)];
        quads.push(
            active.x,
            active.y,
            active.w,
            active.h,
            theme.bar_background.to_linear_alpha(alpha),
        );
        quads.push(
            active.x,
            active.y + active.h - 2.0,
            active.w,
            2.0,
            theme.toggle_on.to_linear_alpha(alpha),
        );

        // The two fields, their arrows and the swap buttons are raised, the way
        // the bar's are: a word drawn on a panel does not read as a control.
        let raised = mix(theme.menu_background, theme.foreground, 0.10).to_linear_alpha(alpha);
        let mut raised_rects = vec![layout.field_query, layout.query_toggle];
        if let Some(rect) = layout.field_replace {
            raised_rects.push(rect);
        }
        if let Some(rect) = layout.replace_toggle {
            raised_rects.push(rect);
        }
        if layout.field_replace.is_some() {
            raised_rects.extend(layout.swaps.iter().copied());
        }
        for rect in raised_rects {
            quads.push(rect.x, rect.y, rect.w, rect.h, raised);
            quads.push(rect.x, rect.y + rect.h - 1.0, rect.w, 1.0, border);
        }

        // A checkbox is a square with a word beside it; a radio is the same
        // square with a dot instead of a fill, because a rounded corner is not
        // something a rectangle can draw.
        for (check, rect) in &layout.checks {
            let faded = if check.disabled(find) { 0.5 } else { alpha };
            mark_box(
                quads,
                Rect {
                    x: rect.x,
                    y: rect.y + 3.0,
                    w: BOX,
                    h: BOX,
                },
                check.is_on(find),
                faded,
                theme,
            );
        }
        for (mode, rect) in &layout.modes {
            radio(
                quads,
                Rect {
                    x: rect.x,
                    y: rect.y + 3.0,
                    w: BOX,
                    h: BOX,
                },
                find.options.mode == *mode,
                alpha,
                theme,
            );
        }
        for (up, rect) in &layout.directions {
            radio(
                quads,
                Rect {
                    x: rect.x,
                    y: rect.y + 3.0,
                    w: BOX,
                    h: BOX,
                },
                find.up == *up,
                alpha,
                theme,
            );
        }
        mark_box(
            quads,
            Rect {
                x: layout.newline.x,
                y: layout.newline.y + 3.0,
                w: BOX,
                h: BOX,
            },
            Check::Newline.is_on(find),
            if Check::Newline.disabled(find) { 0.5 } else { alpha },
            theme,
        );

        // The slider: a track, the part of it that is filled, and a knob.
        let track = layout.transparency;
        let filled = track.w * (self.transparency / MAX_TRANSPARENCY).clamp(0.0, 1.0);
        quads.push(
            track.x,
            track.y,
            track.w,
            track.h,
            mix(theme.menu_background, theme.foreground, 0.22).to_linear_alpha(alpha),
        );
        quads.push(
            track.x,
            track.y,
            filled,
            track.h,
            theme.toggle_on.to_linear_alpha(alpha),
        );
        quads.push(
            track.x + filled - 2.0,
            track.y - 4.0,
            4.0,
            track.h + 8.0,
            theme.foreground.to_linear_alpha(alpha),
        );

        for (_, rect) in &layout.buttons {
            quads.push(rect.x, rect.y, rect.w, rect.h, raised);
            for (x, y, w, h) in [
                (rect.x, rect.y, rect.w, 1.0),
                (rect.x, rect.y + rect.h - 1.0, rect.w, 1.0),
                (rect.x, rect.y, 1.0, rect.h),
                (rect.x + rect.w - 1.0, rect.y, 1.0, rect.h),
            ] {
                quads.push(x, y, w, h, border);
            }
        }

        // The open history list goes last, over the sheet it belongs to.
        if let Some(first) = layout.history.first() {
            let bottom = layout
                .history
                .last()
                .map_or(first.y + first.h, |rect| rect.y + rect.h);
            quads.push(
                first.x,
                first.y,
                first.w,
                bottom - first.y,
                theme.menu_background.to_linear_alpha(alpha),
            );
            for rect in &layout.history {
                quads.push(rect.x, rect.y + rect.h - 1.0, rect.w, 1.0, border);
            }
            if let Some(pick) = find.history_pick() {
                if let Some(rect) = layout.history.get(pick) {
                    quads.push(
                        rect.x,
                        rect.y,
                        rect.w,
                        rect.h,
                        theme.selection.to_linear_alpha(alpha),
                    );
                }
            }
        }

        // The caret and the selection in the focused field.
        let (field, rect) = match find.focus {
            Field::Query => (&find.query, layout.field_query),
            Field::Replacement => (
                &find.replacement,
                layout.field_replace.unwrap_or(layout.field_query),
            ),
        };
        if let Some(range) = field.selection() {
            quads.push(
                rect.x + range.start as f32 * self.char_width,
                rect.y + 2.0,
                (range.end - range.start) as f32 * self.char_width,
                rect.h - 4.0,
                theme.selection.to_linear_alpha(alpha),
            );
        }
        quads.push(
            rect.x + field.caret() as f32 * self.char_width,
            rect.y + 2.0,
            1.5,
            rect.h - 4.0,
            theme.caret.to_linear_alpha(alpha),
        );
    }
}

/// Draws a checkbox: the square, its border, and a fill when it is ticked.
fn mark_box(quads: &mut QuadRenderer, rect: Rect, on: bool, alpha: f32, theme: &Theme) {
    let border = theme.menu_border.to_linear_alpha(alpha);
    let background = if on {
        theme.toggle_on.to_linear_alpha(alpha)
    } else {
        mix(theme.menu_background, theme.foreground, 0.16).to_linear_alpha(alpha)
    };
    quads.push(rect.x, rect.y, rect.w, rect.h, background);
    for (x, y, w, h) in [
        (rect.x, rect.y, rect.w, 1.0),
        (rect.x, rect.y + rect.h - 1.0, rect.w, 1.0),
        (rect.x, rect.y, 1.0, rect.h),
        (rect.x + rect.w - 1.0, rect.y, 1.0, rect.h),
    ] {
        quads.push(x, y, w, h, border);
    }
    if on {
        quads.push(
            rect.x + 3.0,
            rect.y + 3.0,
            rect.w - 6.0,
            rect.h - 6.0,
            theme.toggle_on_text.to_linear_alpha(alpha),
        );
    }
}

/// Draws a radio button, which is a box with a dot rather than a circle.
fn radio(quads: &mut QuadRenderer, rect: Rect, on: bool, alpha: f32, theme: &Theme) {
    let border = theme.menu_border.to_linear_alpha(alpha);
    quads.push(
        rect.x,
        rect.y,
        rect.w,
        rect.h,
        mix(theme.menu_background, theme.foreground, 0.16).to_linear_alpha(alpha),
    );
    for (x, y, w, h) in [
        (rect.x, rect.y, rect.w, 1.0),
        (rect.x, rect.y + rect.h - 1.0, rect.w, 1.0),
        (rect.x, rect.y, 1.0, rect.h),
        (rect.x + rect.w - 1.0, rect.y, 1.0, rect.h),
    ] {
        quads.push(x, y, w, h, border);
    }
    if on {
        quads.push(
            rect.x + 4.0,
            rect.y + 4.0,
            rect.w - 8.0,
            rect.h - 8.0,
            theme.toggle_on.to_linear_alpha(alpha),
        );
    }
}

/// The widest line of a shaped buffer, in pixels.
fn widest_line(buffer: &Buffer) -> f32 {
    buffer.layout_runs().map(|run| run.line_w).fold(0.0, f32::max)
}

impl FindDialog {
    /// The text areas to hand to glyphon.
    pub fn text_areas<'a>(
        &'a self,
        find: &'a FindBar,
        line_height: f32,
        theme: &Theme,
    ) -> Vec<TextArea<'a>> {
        if !self.is_open(find) {
            return Vec::new();
        }
        let layout = self.layout(find, line_height);
        let alpha = self.alpha();
        let bounds = TextBounds {
            left: layout.x as i32,
            top: layout.y as i32,
            right: (layout.x + layout.w) as i32,
            bottom: (layout.y + layout.h) as i32,
        };
        let tint = |rgb: Rgb, fade: f32| {
            let a = (fade * 255.0).round() as u8;
            Color::rgba(rgb.0, rgb.1, rgb.2, a)
        };
        let mut areas: Vec<TextArea<'a>> = Vec::with_capacity(24);

        areas.push(self.area(
            self.tab.label(),
            layout.title.x + PAD,
            layout.title.y + 3.0,
            bounds,
            tint(theme.menu_text, alpha),
        ));
        for (index, tab) in TABS.iter().enumerate() {
            let rect = layout.tabs[index];
            let colour = if *tab == self.tab {
                tint(theme.foreground, alpha)
            } else {
                tint(theme.menu_text, alpha)
            };
            areas.push(self.area(tab.label(), rect.x + 8.0, rect.y + 4.0, bounds, colour));
        }
        areas.push(self.area(
            "Find what:",
            layout.label_x,
            layout.field_query.y,
            bounds,
            tint(theme.bar_label, alpha),
        ));
        if let Some(rect) = layout.field_replace {
            areas.push(self.area(
                "Replace with:",
                layout.label_x,
                rect.y,
                bounds,
                tint(theme.bar_label, alpha),
            ));
        }
        for (check, rect) in &layout.checks {
            let colour = if check.disabled(find) {
                tint(theme.menu_text_disabled, 0.5)
            } else {
                tint(theme.menu_text, alpha)
            };
            areas.push(self.area(
                check.label(),
                rect.x + BOX + BOX_GAP,
                rect.y,
                bounds,
                colour,
            ));
        }
        areas.push(self.area(
            "Direction",
            layout.direction_caption.x,
            layout.direction_caption.y,
            bounds,
            tint(theme.menu_text_disabled, alpha),
        ));
        for (up, rect) in &layout.directions {
            let word = DIRECTIONS
                .iter()
                .find(|(direction, _)| *direction == *up)
                .map_or("Up", |(_, word)| *word);
            let colour = if find.up == *up {
                tint(theme.foreground, alpha)
            } else {
                tint(theme.menu_text, alpha)
            };
            areas.push(self.area(word, rect.x + BOX + BOX_GAP, rect.y, bounds, colour));
        }
        areas.push(self.area(
            "Search Mode",
            layout.mode_caption.x,
            layout.mode_caption.y,
            bounds,
            tint(theme.menu_text_disabled, alpha),
        ));
        for (mode, rect) in &layout.modes {
            let word = MODES
                .iter()
                .find(|(other, _)| other == mode)
                .map_or("Normal", |(_, word)| *word);
            let colour = if find.options.mode == *mode {
                tint(theme.foreground, alpha)
            } else {
                tint(theme.menu_text, alpha)
            };
            areas.push(self.area(word, rect.x + BOX + BOX_GAP, rect.y, bounds, colour));
        }
        let newline = if Check::Newline.disabled(find) {
            tint(theme.menu_text_disabled, 0.5)
        } else {
            tint(theme.menu_text, alpha)
        };
        areas.push(self.area(
            Check::Newline.label(),
            layout.newline.x + BOX + BOX_GAP,
            layout.newline.y,
            bounds,
            newline,
        ));
        areas.push(self.area(
            "Transparency",
            layout.transparency_caption.x,
            layout.transparency_caption.y,
            bounds,
            tint(theme.bar_label, alpha),
        ));
        for (button, rect) in &layout.buttons {
            // Centred by measurement: a translation is rarely the same width as
            // the English it replaces, and a word pinned to the left edge of its
            // button looks like a mistake.
            let word_w = self.button_width(*button) - 2.0 * BUTTON_PAD;
            let left = rect.x + (rect.w - word_w) / 2.0;
            areas.push(self.area(
                button.label(),
                left.max(rect.x + 4.0),
                rect.y + 3.0,
                bounds,
                tint(theme.foreground, alpha),
            ));
        }
        if layout.field_replace.is_some() {
            for (index, rect) in layout.swaps.iter().enumerate() {
                areas.push(self.area(
                    SWAP_CHIPS[index].mark(),
                    rect.x + 3.0,
                    rect.y,
                    bounds,
                    tint(theme.foreground, alpha),
                ));
            }
        }
        for rect in std::iter::once(layout.query_toggle).chain(layout.replace_toggle) {
            if layout.field_replace.is_none() && Some(rect) == layout.replace_toggle {
                continue;
            }
            let arrow = ARROW;
            if let Some(buffer) = self.labels.get(arrow) {
                areas.push(TextArea {
                    buffer,
                    left: rect.x + 5.0,
                    top: rect.y,
                    scale: 1.0,
                    bounds,
                    default_color: tint(theme.bar_label, alpha),
                    custom_glyphs: &[],
                });
            }
        }

        // The two fields' own text, laid out by the bar's text fields.
        areas.push(TextArea {
            buffer: find.query.buffer(),
            left: layout.field_query.x,
            top: layout.field_query.y,
            scale: 1.0,
            bounds,
            default_color: tint(theme.foreground, alpha),
            custom_glyphs: &[],
        });
        if let Some(rect) = layout.field_replace {
            areas.push(TextArea {
                buffer: find.replacement.buffer(),
                left: rect.x,
                top: rect.y,
                scale: 1.0,
                bounds,
                default_color: tint(theme.foreground, alpha),
                custom_glyphs: &[],
            });
        }

        // The open history list, clipped to its own rows.
        if let Some(first) = layout.history.first() {
            let bottom = layout
                .history
                .last()
                .map_or(first.y + first.h, |rect| rect.y + rect.h);
            let list_bounds = TextBounds {
                left: first.x as i32,
                top: first.y as i32,
                right: (first.x + first.w) as i32,
                bottom: bottom as i32,
            };
            for (index, rect) in layout.history.iter().enumerate() {
                let Some(buffer) = self.history_rows.get(index) else {
                    continue;
                };
                areas.push(TextArea {
                    buffer,
                    left: rect.x + 4.0,
                    top: rect.y,
                    scale: 1.0,
                    bounds: list_bounds,
                    default_color: if find.history_pick() == Some(index) {
                        tint(theme.toggle_on_text, alpha)
                    } else {
                        tint(theme.foreground, alpha)
                    },
                    custom_glyphs: &[],
                });
            }
        }
        areas
    }

    /// One label as a text area.
    fn area<'a>(
        &'a self,
        english: &'static str,
        left: f32,
        top: f32,
        bounds: TextBounds,
        colour: Color,
    ) -> TextArea<'a> {
        TextArea {
            buffer: self
                .labels
                .get(english)
                .expect("every drawn label is in the map"),
            left,
            top,
            scale: 1.0,
            bounds,
            default_color: colour,
            custom_glyphs: &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nn_core::Document;

    const LINE: f32 = 20.0;

    fn metrics() -> Metrics {
        Metrics::new(15.0, LINE)
    }

    fn dialog() -> FindDialog {
        let mut dialog = FindDialog::new(metrics(), Family::SansSerif, 8.0);
        dialog.set_viewport(1100.0, 750.0);
        dialog
    }

    fn bar() -> FindBar {
        let mut bar = FindBar::new(metrics(), Family::SansSerif, 8.0);
        bar.open_find();
        bar.set_floating(true);
        bar.query.set_text("needle");
        bar
    }

    #[test]
    fn a_closed_search_has_no_dialog() {
        let dialog = dialog();
        let mut find = bar();
        find.open = false;
        assert!(!dialog.is_open(&find));
        assert_eq!(dialog.hit_at(&find, 550.0, 375.0, LINE), None);
        assert!(!dialog.contains(&find, 550.0, 375.0, LINE));
    }

    #[test]
    fn a_docked_search_has_no_dialog() {
        let dialog = dialog();
        let mut find = bar();
        find.set_floating(false);
        assert!(!dialog.is_open(&find));
        assert!(!dialog.contains(&find, 550.0, 375.0, LINE));
    }

    #[test]
    fn a_floating_search_reserves_no_band() {
        let mut find = bar();
        assert_eq!(find.height(1100.0, LINE, 8.0), 0.0);
        find.set_floating(false);
        assert!(find.height(1100.0, LINE, 8.0) > 0.0);
    }

    #[test]
    fn a_floating_search_takes_no_clicks_for_the_bar() {
        let find = bar();
        assert_eq!(find.hit_at(550.0, 740.0, 1100.0, 712.0, LINE, 8.0), None);
    }

    #[test]
    fn the_frame_sits_inside_the_window() {
        let dialog = dialog();
        let find = bar();
        let layout = dialog.layout(&find, LINE);
        assert!(layout.x >= 0.0 && layout.y >= 0.0);
        assert!(layout.x + layout.w <= 1100.0);
        assert!(layout.y + layout.h <= 750.0);
        assert!(layout.w >= MIN_WIDTH && layout.w <= WIDTH);
    }

    #[test]
    fn every_control_sits_inside_the_frame() {
        for tab in TABS {
            let mut dialog = dialog();
            dialog.set_tab(tab);
            let find = bar();
            let layout = dialog.layout(&find, LINE);
            let mut rects = vec![
                layout.title,
                layout.field_query,
                layout.query_toggle,
                layout.newline,
                layout.transparency,
                layout.direction_caption,
                layout.mode_caption,
            ];
            rects.extend(layout.checks.iter().map(|(_, rect)| *rect));
            rects.extend(layout.buttons.iter().map(|(_, rect)| *rect));
            rects.extend(layout.tabs);
            for rect in rects {
                assert!(
                    rect.x >= layout.x
                        && rect.x + rect.w <= layout.x + layout.w + 0.5
                        && rect.y >= layout.y
                        && rect.y + rect.h <= layout.y + layout.h + 0.5,
                    "{tab:?}: {rect:?} is outside the frame"
                );
            }
        }
    }

    #[test]
    fn every_tab_is_clickable_where_it_is_drawn() {
        for tab in TABS {
            let mut dialog = dialog();
            dialog.set_tab(tab);
            let find = bar();
            let layout = dialog.layout(&find, LINE);
            let index = TABS.iter().position(|other| *other == tab).unwrap();
            let rect = layout.tabs[index];
            assert_eq!(
                dialog.hit_at(&find, rect.x + 2.0, rect.y + 2.0, LINE),
                Some(DialogHit::Tab(tab)),
                "{tab:?}"
            );
        }
    }

    #[test]
    fn every_checkbox_is_clickable_where_it_is_drawn() {
        for tab in TABS {
            let mut dialog = dialog();
            dialog.set_tab(tab);
            let find = bar();
            let layout = dialog.layout(&find, LINE);
            assert!(!layout.checks.is_empty());
            for (check, rect) in &layout.checks {
                assert_eq!(
                    dialog.hit_at(&find, rect.x + 2.0, rect.y + 2.0, LINE),
                    Some(DialogHit::Find(Hit::Chip(check.chip()))),
                    "{check:?} on {tab:?}"
                );
            }
            let rect = layout.newline;
            assert_eq!(
                dialog.hit_at(&find, rect.x + 2.0, rect.y + 2.0, LINE),
                Some(DialogHit::Find(Hit::Chip(Chip::Word)))
            );
        }
    }

    #[test]
    fn every_button_is_clickable_where_it_is_drawn() {
        for tab in TABS {
            let mut dialog = dialog();
            dialog.set_tab(tab);
            let find = bar();
            let layout = dialog.layout(&find, LINE);
            assert_eq!(layout.buttons.len(), tab.buttons().len());
            for (button, rect) in &layout.buttons {
                assert_eq!(
                    dialog.hit_at(&find, rect.x + 2.0, rect.y + 2.0, LINE),
                    Some(DialogHit::Find(Hit::Button(*button))),
                    "{button:?} on {tab:?}"
                );
            }
        }
    }

    #[test]
    fn the_two_fields_and_their_arrows_are_clickable() {
        let mut dialog = dialog();
        dialog.set_tab(Tab::Replace);
        let find = bar();
        let layout = dialog.layout(&find, LINE);
        assert_eq!(
            dialog.hit_at(
                &find,
                layout.field_query.x + 4.0,
                layout.field_query.y + 2.0,
                LINE
            ),
            Some(DialogHit::Find(Hit::Field(Field::Query)))
        );
        assert_eq!(
            dialog.hit_at(
                &find,
                layout.query_toggle.x + 2.0,
                layout.query_toggle.y + 2.0,
                LINE
            ),
            Some(DialogHit::Find(Hit::HistoryToggle(Field::Query)))
        );
        let replace = layout
            .field_replace
            .expect("the Replace tab has a replacement row");
        assert_eq!(
            dialog.hit_at(&find, replace.x + 4.0, replace.y + 2.0, LINE),
            Some(DialogHit::Find(Hit::Field(Field::Replacement)))
        );
        let toggle = layout.replace_toggle.expect("and its arrow");
        assert_eq!(
            dialog.hit_at(&find, toggle.x + 2.0, toggle.y + 2.0, LINE),
            Some(DialogHit::Find(Hit::HistoryToggle(Field::Replacement)))
        );
        for (index, rect) in layout.swaps.iter().enumerate() {
            assert_eq!(
                dialog.hit_at(&find, rect.x + 2.0, rect.y + 2.0, LINE),
                Some(DialogHit::Find(Hit::Swap(SWAP_CHIPS[index])))
            );
        }
    }

    #[test]
    fn the_find_tab_has_no_replacement_row() {
        let dialog = dialog();
        let find = bar();
        let layout = dialog.layout(&find, LINE);
        assert!(layout.field_replace.is_none());
        assert!(layout.replace_toggle.is_none());
        // The swap buttons keep their slots but are zero-sized, so they are not
        // clickable: a control a tab does not show is not there.
        for rect in layout.swaps {
            assert_eq!(rect.w, 0.0);
        }
    }

    #[test]
    fn every_mode_radio_is_clickable_where_it_is_drawn() {
        let dialog = dialog();
        let find = bar();
        let layout = dialog.layout(&find, LINE);
        for (mode, rect) in &layout.modes {
            let chip = match mode {
                SearchMode::Normal => Chip::Normal,
                SearchMode::Extended => Chip::Extended,
                SearchMode::Regex => Chip::Regex,
            };
            assert_eq!(
                dialog.hit_at(&find, rect.x + 2.0, rect.y + 2.0, LINE),
                Some(DialogHit::Find(Hit::Chip(chip)))
            );
        }
    }

    #[test]
    fn both_direction_radios_are_clickable() {
        let dialog = dialog();
        let find = bar();
        let layout = dialog.layout(&find, LINE);
        for (up, rect) in &layout.directions {
            let chip = if *up { Chip::Up } else { Chip::Down };
            assert_eq!(
                dialog.hit_at(&find, rect.x + 2.0, rect.y + 2.0, LINE),
                Some(DialogHit::Find(Hit::Chip(chip)))
            );
        }
    }

    #[test]
    fn the_slider_answers_with_the_fraction_it_was_clicked_at() {
        let dialog = dialog();
        let find = bar();
        let layout = dialog.layout(&find, LINE);
        let track = layout.transparency;
        match dialog.hit_at(&find, track.x + track.w / 2.0, track.y + 2.0, LINE) {
            Some(DialogHit::Transparency(fraction)) => {
                assert!((fraction - 0.5).abs() < 0.05, "{fraction}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            dialog.hit_at(&find, track.x + track.w + 40.0, track.y + 2.0, LINE),
            None
        );
    }

    #[test]
    fn the_title_bar_drags_the_frame() {
        let mut dialog = dialog();
        let find = bar();
        let before = dialog.layout(&find, LINE);
        dialog.begin_drag(before.title.x + 20.0, before.title.y + 4.0);
        assert!(dialog.dragging());
        dialog.drag_to(before.title.x + 60.0, before.title.y + 40.0);
        dialog.end_drag();
        let after = dialog.layout(&find, LINE);
        assert!(
            (after.x - (before.x + 40.0)).abs() < 0.5,
            "{} -> {}",
            before.x,
            after.x
        );
        assert!(
            (after.y - (before.y + 36.0)).abs() < 0.5,
            "{} -> {}",
            before.y,
            after.y
        );
    }

    #[test]
    fn a_dragged_frame_stops_at_the_window_edge() {
        let mut dialog = dialog();
        let find = bar();
        dialog.begin_drag(20.0, 4.0);
        dialog.drag_to(-400.0, -400.0);
        dialog.end_drag();
        let layout = dialog.layout(&find, LINE);
        assert_eq!(layout.x, 0.0);
        assert_eq!(layout.y, 0.0);
        dialog.begin_drag(20.0, 4.0);
        dialog.drag_to(2000.0, 2000.0);
        dialog.end_drag();
        let layout = dialog.layout(&find, LINE);
        assert!(layout.x + layout.w <= 1100.0);
        assert!(layout.y + layout.h <= 750.0);
    }

    #[test]
    fn the_frame_does_not_change_height_between_tabs() {
        let mut dialog = dialog();
        let find = bar();
        let mut heights = Vec::new();
        for tab in TABS {
            dialog.set_tab(tab);
            heights.push(dialog.layout(&find, LINE).h);
        }
        // The frame is measured with the tallest tab, so no tab can be taller
        // than it: a dialog that grew on a tab switch would move its own buttons.
        assert!(
            heights.iter().all(|height| *height <= heights[1] + 0.5),
            "{heights:?}"
        );
    }

    #[test]
    fn transparency_is_clamped_to_the_slider() {
        let mut dialog = dialog();
        dialog.set_transparency(-1.0);
        assert_eq!(dialog.transparency(), 0.0);
        dialog.set_transparency(0.5);
        assert!((dialog.transparency() - 0.5).abs() < f32::EPSILON);
        dialog.set_transparency(4.0);
        assert!((dialog.transparency() - MAX_TRANSPARENCY).abs() < f32::EPSILON);
    }

    #[test]
    fn the_mark_tab_draws_the_marking_buttons() {
        let mut dialog = dialog();
        dialog.set_tab(Tab::Mark);
        let find = bar();
        let layout = dialog.layout(&find, LINE);
        let labels: Vec<Button> = layout.buttons.iter().map(|(button, _)| *button).collect();
        assert_eq!(
            labels,
            vec![Button::MarkAll, Button::ClearMarks, Button::Close]
        );
        let checks: Vec<Check> = layout.checks.iter().map(|(check, _)| *check).collect();
        assert!(checks.contains(&Check::BookmarkLine));
        assert!(checks.contains(&Check::PurgeMarks));
    }

    #[test]
    fn the_replace_tab_draws_the_replacement_buttons() {
        let mut dialog = dialog();
        dialog.set_tab(Tab::Replace);
        let find = bar();
        let layout = dialog.layout(&find, LINE);
        let labels: Vec<Button> = layout.buttons.iter().map(|(button, _)| *button).collect();
        assert!(labels.contains(&Button::Replace));
        assert!(labels.contains(&Button::ReplaceAll));
        assert!(labels.contains(&Button::ReplaceAllAll));
        assert_eq!(labels.last(), Some(&Button::Close));
    }

    #[test]
    fn the_checkbox_stands_for_the_option_it_names() {
        let mut find = bar();
        assert!(!Check::WholeWord.is_on(&find));
        find.activate(Chip::Word);
        assert!(Check::WholeWord.is_on(&find));
        assert!(!Check::Newline.is_on(&find));
        // In regex mode the same box means the newline option, and the whole
        // word box is the one greyed out.
        find.activate(Chip::Regex);
        assert!(Check::WholeWord.disabled(&find));
        assert!(!Check::Newline.disabled(&find));
        assert!(!Check::Newline.is_on(&find));
        find.activate(Chip::Word);
        assert!(Check::Newline.is_on(&find));
        // The whole-word value the box still holds is not consulted while it is
        // greyed out: the search reads the newline option instead.
        assert!(Check::WholeWord.disabled(&find));
    }

    #[test]
    fn the_dialog_names_every_label_it_draws() {
        let labels = FindDialog::labels();
        assert!(labels.contains(&"Match case"));
        assert!(labels.contains(&". matches newline"));
        assert!(labels.contains(&"Regular expression"));
        assert!(labels.contains(&"Find what:"));
        assert!(labels.contains(&"Purge for each search"));
    }

    #[test]
    fn the_field_captions_sit_inside_the_frame() {
        // The captions are positions, not rectangles, so nothing in the layout
        // moves them unless `layout` does: they were drawn at the window's left
        // edge and clipped away by the frame's own text bounds.
        for tab in TABS {
            let mut dialog = dialog();
            dialog.set_tab(tab);
            let find = bar();
            let layout = dialog.layout(&find, LINE);
            assert!(
                layout.label_x >= layout.x,
                "{tab:?}: the caption at {} is left of the frame at {}",
                layout.label_x,
                layout.x
            );
            assert!(
                layout.label_x < layout.field_query.x,
                "{tab:?}: the caption overlays the field"
            );
            assert!(layout.label_x + dialog.label_width() <= layout.field_query.x + 0.5);
        }
    }

    #[test]
    fn every_drawn_label_is_shaped() {
        // `prepare` shapes the words it knows about and `text_areas` looks each
        // one up: a label the sheet draws but the map does not hold panics on the
        // frame. The buttons were the ones missing, so this walks every tab.
        let mut font_system = FontSystem::new();
        for tab in TABS {
            let mut dialog = dialog();
            dialog.set_tab(tab);
            let find = bar();
            dialog.prepare(&mut font_system, &find);
            let areas = dialog.text_areas(&find, LINE, &Theme::default());
            assert!(!areas.is_empty(), "{tab:?}: nothing drawn");
        }
    }

    #[test]
    fn every_button_row_fits_the_frame_width() {
        // The Find tab's five buttons do not fit one row at this width, so they
        // wrap: without that they were laid out past the right edge and Close
        // fell outside the frame.
        for tab in TABS {
            let mut dialog = dialog();
            dialog.set_tab(tab);
            let find = bar();
            let layout = dialog.layout(&find, LINE);
            let rows = dialog.button_rows(tab);
            assert_eq!(
                rows.iter().map(Vec::len).sum::<usize>(),
                tab.buttons().len(),
                "{tab:?}: a button was dropped while wrapping"
            );
            let inner = layout.w - 2.0 * PAD;
            for row in &rows {
                let used: f32 = row
                    .iter()
                    .map(|button| dialog.button_width(*button))
                    .sum::<f32>()
                    + GAP * (row.len() - 1) as f32;
                assert!(used <= inner + 0.5, "{tab:?}: a row is {used} wide");
            }
        }
    }

    #[test]
    fn a_floating_search_finds_the_same_matches() {
        // The dialog is a view, so the search behind it is the bar's: a pattern
        // typed into it is found by the code that was already there.
        let mut find = bar();
        let doc = Document::from_text("a needle here\n");
        find.refresh(&doc);
        assert_eq!(find.matches.len(), 1);
        assert!(find.floating());
    }
}
