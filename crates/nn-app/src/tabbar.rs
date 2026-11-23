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

//! The tab strip.
//!
//! Document titles, each a rectangle plus a shaped title; the active one is
//! drawn in the editor's own background colour so it reads as continuous with
//! the text beside it.
//!
//! The strip has three arrangements, matching Notepad++'s three: a single row
//! across the top that clips when it runs out of room, a multi-line block that
//! wraps onto further rows, and a column down the left edge of the pane. All
//! three share one geometry function, so a click, a rectangle and a title can
//! never disagree about where a tab is.
//!
//! Titles are shaped once per change rather than per frame, and the strip is
//! only as wide as its tabs, so a window with one document open costs one
//! rectangle and one text area.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics};
use glyphon::{TextArea, TextBounds};

use crate::iconrender::IconRenderer;
use crate::icons;
use crate::quad::QuadRenderer;
use crate::text::shaping_for;
use crate::theme::{mix, Rgb, Theme};

/// The minimum width of a tab, in pixels.
///
/// Wide enough for the two buttons at the trailing edge plus a few characters
/// of title: a tab narrower than its own controls would put the cross over the
/// filename.
const MIN_TAB_WIDTH: f32 = 128.0;
/// The maximum width of a tab, in pixels.
const MAX_TAB_WIDTH: f32 = 260.0;
/// Horizontal padding inside a tab.
const TAB_PADDING: f32 = 12.0;
/// The side of the close and pin buttons, in pixels.
///
/// Notepad++ uses 16 in dark mode and 11 in light; one size for both keeps the
/// hit target the same whichever palette is on.
const BUTTON_SIZE: f32 = 16.0;
/// The gap between the close button and the pin button.
const BUTTON_GAP: f32 = 4.0;
/// The width the two buttons and their gap take at a tab's trailing edge.
const BUTTON_ZONE: f32 = BUTTON_SIZE * 2.0 + BUTTON_GAP;
/// The space between the end of a title and the first button it must not reach.
///
/// A tab reserves [`BUTTON_ZONE`] for its buttons; this is the margin inside
/// that reservation, so a long name stops a word-gap short of the pin instead of
/// running underneath it.
const LABEL_GAP: f32 = 8.0;
/// The side of the pencil that marks an unsaved document.
const MARK_SIZE: f32 = 14.0;
/// The gap between the pencil and the title.
const MARK_GAP: f32 = 5.0;
/// The height of the bar along the leading edge of the active tab.
///
/// Notepad++ uses `scale(4)`; four pixels is what makes the bar read as a bar
/// rather than as a border.
const TOP_BAR: f32 = 4.0;
/// How many rows a multi-line strip will grow to.
///
/// Notepad++ grows without bound, which on a small window leaves a strip taller
/// than the text it is labelling. Three rows is the point past which the strip
/// costs more than it gives; beyond that the tabs clip, the same way they do in
/// single-row mode.
const MAX_ROWS: usize = 3;
/// The width of the vertical strip's column, in pixels.
///
/// Fixed rather than proportional: a column that resized with the window would
/// move every tab under the pointer whenever the window was dragged, and the
/// point of the column is that a tab stays where it was put.
const COLUMN_WIDTH: f32 = 168.0;

/// How far the tab colour is blended toward the theme's own background.
///
/// A saturated yellow that reads well on a dark background is unreadable on a
/// light one, so the colour is muted rather than used raw. Halfway keeps the
/// hue recognisable while leaving the title legible in both palettes.
const COLOUR_MIX: f32 = 0.5;

/// A colour the user can paint a tab with.
///
/// Fixed rather than themeable: the point of colouring a tab is to recognise it
/// at a glance, and a colour that changed with the theme would stop being the
/// one the user picked. The five are Notepad++'s own set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabColour {
    #[default]
    None,
    Yellow,
    Green,
    Blue,
    Red,
    Purple,
}

impl TabColour {
    /// Every colour, in the order the menu cycles them.
    pub const ALL: [TabColour; 6] = [
        TabColour::None,
        TabColour::Yellow,
        TabColour::Green,
        TabColour::Blue,
        TabColour::Red,
        TabColour::Purple,
    ];

    /// The keyword written to the session file.
    pub fn key(self) -> &'static str {
        match self {
            TabColour::None => "none",
            TabColour::Yellow => "yellow",
            TabColour::Green => "green",
            TabColour::Blue => "blue",
            TabColour::Red => "red",
            TabColour::Purple => "purple",
        }
    }

    /// The label shown in the context menu.
    pub fn label(self) -> &'static str {
        match self {
            TabColour::None => "No Colour",
            TabColour::Yellow => "Yellow",
            TabColour::Green => "Green",
            TabColour::Blue => "Blue",
            TabColour::Red => "Red",
            TabColour::Purple => "Purple",
        }
    }

    /// Reads a keyword back, or `None` when it is not one of ours.
    pub fn from_key(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|colour| colour.key() == text)
    }

    /// The next colour, wrapping. What the menu entry does when picked.
    pub fn next(self) -> Self {
        let at = Self::ALL.iter().position(|c| *c == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }

    /// The colour itself, or `None` for an unpainted tab.
    pub fn rgb(self) -> Option<Rgb> {
        match self {
            TabColour::None => None,
            TabColour::Yellow => Some(Rgb::new(0xC8, 0xA8, 0x2A)),
            TabColour::Green => Some(Rgb::new(0x4C, 0xA0, 0x50)),
            TabColour::Blue => Some(Rgb::new(0x3D, 0x7E, 0xC4)),
            TabColour::Red => Some(Rgb::new(0xC0, 0x4A, 0x44)),
            TabColour::Purple => Some(Rgb::new(0x8A, 0x5C, 0xB8)),
        }
    }
}

/// How the strip arranges its tabs.
///
/// The three arrangements Notepad++ offers under Settings > Preferences >
/// General > Tab Bar. Persisted by key, so the names are stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabLayout {
    /// One row across the top, clipped at the right edge.
    #[default]
    Horizontal,
    /// As many rows as the tabs need, up to [`MAX_ROWS`], then clipped.
    MultiLine,
    /// A column down the left edge of the pane.
    Vertical,
}

impl TabLayout {
    /// Every arrangement, in the order the menu cycles them.
    pub const ALL: [TabLayout; 3] = [
        TabLayout::Horizontal,
        TabLayout::MultiLine,
        TabLayout::Vertical,
    ];

    /// The keyword written to the settings file.
    pub fn key(self) -> &'static str {
        match self {
            TabLayout::Horizontal => "horizontal",
            TabLayout::MultiLine => "multiline",
            TabLayout::Vertical => "vertical",
        }
    }

    /// Reads a keyword back, or `None` when it is not one of ours.
    pub fn from_key(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|layout| layout.key() == text)
    }

    /// The label shown in the menu.
    pub fn label(self) -> &'static str {
        match self {
            TabLayout::Horizontal => "Single Row",
            TabLayout::MultiLine => "Multi-line",
            TabLayout::Vertical => "Vertical",
        }
    }

    /// The next arrangement, wrapping. What the menu entry does when picked.
    pub fn next(self) -> Self {
        let at = Self::ALL.iter().position(|l| *l == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }

    /// Whether the strip runs down the side rather than across the top.
    pub fn is_vertical(self) -> bool {
        self == TabLayout::Vertical
    }
}

/// Where one tab sits inside the strip, relative to the strip's own corner.
///
/// One struct rather than four parallel vectors: the four travel together, and
/// a caller that read three of them would draw a tab in the wrong place.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Slot {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
}

/// What the pointer is over inside the strip.
///
/// An enum rather than an index with sentinels: the close and pin buttons
/// belong to the active tab, so "the pointer is over the cross" is a different
/// fact from "the pointer is over tab 3", and a sentinel index would make the
/// two indistinguishable in the drawing code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hover {
    Tab(usize),
    Close,
    Pin,
    /// The arrow that scrolls the strip towards its first tab.
    ScrollLeft,
    /// The arrow that scrolls it towards its last.
    ScrollRight,
}

/// What the strip needs to know about one tab.
///
/// A struct rather than three parallel vectors: the three travel together, and
/// a caller that updated two of them would draw a title with the wrong colour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabInfo {
    pub title: String,
    pub pinned: bool,
    pub colour: TabColour,
    /// Whether the document has edits that are not on disk.
    ///
    /// Notepad++ marks this with a pencil in dark mode and a red floppy in
    /// light. We use the pencil in both: a floppy on every *saved* tab is
    /// noise, and the pencil only appears when there is something to say.
    pub modified: bool,
}

impl TabInfo {
    /// An ordinary tab.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            pinned: false,
            colour: TabColour::None,
            modified: false,
        }
    }
}

/// Moves the item at `from` so that it sits at `to`.
///
/// Notepad++ lets a tab be dragged along its strip, and the drag is live: the
/// tab takes the place of the one it passes rather than being reordered on the
/// drop. Both ends are ignored rather than clamped here, because the index comes
/// from a hit test that ran against the previous frame's slots.
pub fn move_tab<T>(tabs: &mut Vec<T>, from: usize, to: usize) {
    if from == to || from >= tabs.len() || to >= tabs.len() {
        return;
    }
    let tab = tabs.remove(from);
    tabs.insert(to, tab);
}

/// The strip of document tabs.
pub struct TabBar {
    tabs: Vec<TabInfo>,
    active: usize,
    buffers: Vec<Buffer>,
    metrics: Metrics,
    family: Family<'static>,
    char_width: f32,
    /// Where the strip starts in the window, and how wide it is.
    ///
    /// Zero and the window width for a single pane. A split view gives each pane
    /// its own strip, so the tabs of one document never appear over the other.
    left: f32,
    top: f32,
    width: f32,
    /// How the tabs are arranged.
    layout: TabLayout,
    /// How far the strip is scrolled towards its last tab, in pixels.
    ///
    /// Zero while the tabs fit, which is the usual case. Once they do not, the
    /// single row scrolls under the strip's own arrows rather than running off
    /// the edge with no way to reach the tabs past it. Only the single-row
    /// arrangement scrolls: a column has its own height and the multi-line strip
    /// wraps.
    offset: f32,
    /// What the pointer is over, if anything.
    hover: Option<Hover>,
    stale: bool,
}

impl TabBar {
    /// Builds an empty strip.
    pub fn new(metrics: Metrics, family: Family<'static>, char_width: f32) -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
            buffers: Vec::new(),
            metrics,
            family,
            char_width,
            left: 0.0,
            top: 0.0,
            width: 0.0,
            layout: TabLayout::Horizontal,
            offset: 0.0,
            hover: None,
            stale: true,
        }
    }

    /// Replaces the tabs and the active index.
    pub fn set_tabs(&mut self, tabs: Vec<TabInfo>, active: usize) {
        if self.tabs != tabs || self.active != active {
            self.tabs = tabs;
            self.active = active;
            // The buttons belong to the active tab, so a change of active tab
            // moves them out from under the pointer.
            self.hover = None;
            self.stale = true;
        }
        // The set of tabs decides how far the strip may scroll, and the active
        // one decides where it has to be: a document opened from the keyboard
        // must arrive in view rather than past the last arrow.
        self.clamp_offset();
        self.ensure_active_visible();
    }

    /// The tabs the strip is showing.
    pub fn tabs(&self) -> &[TabInfo] {
        &self.tabs
    }

    /// How the tabs are arranged.
    pub fn layout(&self) -> TabLayout {
        self.layout
    }

    /// Changes the arrangement.
    ///
    /// The titles do not need reshaping: the same buffer is drawn at a different
    /// place, and the clip rectangle is what makes a long title fit a narrow
    /// column.
    pub fn set_layout(&mut self, layout: TabLayout) {
        self.layout = layout;
    }

    /// The height of one row of tabs, in pixels.
    fn row_height(&self, line_height: f32) -> f32 {
        line_height + 8.0
    }

    /// Whether the strip is drawn at all.
    ///
    /// Any document at all, not two: Notepad++ keeps the tab bar up for a lone
    /// document, and the tab is where the close and pin buttons live, so hiding
    /// it would take those away from the one document that has them. The strip
    /// therefore appears when the first document opens and stays until the pane
    /// is empty, which also means the text never shifts under the pointer as
    /// documents come and go.
    fn is_shown(&self) -> bool {
        !self.tabs.is_empty()
    }

    /// The height the strip occupies at the top of the pane, in pixels.
    ///
    /// Zero for the vertical arrangement, which takes width instead.
    pub fn height(&self, line_height: f32) -> f32 {
        if !self.is_shown() || self.layout.is_vertical() {
            return 0.0;
        }
        self.rows() as f32 * self.row_height(line_height)
    }

    /// The width the strip occupies at the left of the pane, in pixels.
    ///
    /// Zero for the two horizontal arrangements, which take height instead.
    pub fn width(&self) -> f32 {
        if !self.is_shown() || !self.layout.is_vertical() {
            return 0.0;
        }
        // A column wider than the pane would leave no text at all, so it gives
        // way rather than pushing the document off the edge.
        COLUMN_WIDTH.min(self.width * 0.5).max(0.0)
    }

    /// How many rows the strip uses.
    ///
    /// One for the single-row arrangement, one per tab for the vertical one, and
    /// however many the widths need for multi-line, capped at [`MAX_ROWS`].
    fn rows(&self) -> usize {
        if !self.is_shown() {
            return 0;
        }
        match self.layout {
            TabLayout::Horizontal => 1,
            TabLayout::Vertical => self.tabs.len(),
            TabLayout::MultiLine => {
                let mut rows = 1;
                let mut used = 0.0;
                for i in 0..self.tabs.len() {
                    let w = self.tab_width(i);
                    if used > 0.0 && used + w > self.width && rows < MAX_ROWS {
                        rows += 1;
                        used = w;
                    } else {
                        used += w;
                    }
                }
                rows
            }
        }
    }

    /// Places the strip in the window.
    pub fn set_bounds(&mut self, left: f32, top: f32, width: f32) {
        let previous = self.width;
        self.left = left;
        self.top = top;
        self.width = width;
        // A narrower window can make an existing scroll impossible, and a wider
        // one can leave the strip parked past its own end.
        self.clamp_offset();
        // A tab set that arrived before the strip had a width could not be
        // scrolled into view, and every tab past the new edge would be stranded.
        // Only a real change of width re-anchors the strip: doing it every frame
        // would undo the arrows as fast as they are pressed.
        if (self.width - previous).abs() > f32::EPSILON {
            self.ensure_active_visible();
        }
    }

    /// The width of tab `index`.
    ///
    /// Every tab reserves the button zone whether or not its buttons are drawn,
    /// so the strip does not reflow when the active tab changes. The pencil is
    /// only reserved on a tab that has one, because a dirty marker appearing
    /// should widen its own tab and nothing else.
    fn tab_width(&self, index: usize) -> f32 {
        let Some(tab) = self.tabs.get(index) else {
            return MIN_TAB_WIDTH;
        };
        let chars = tab.title.chars().count();
        let mark = if tab.modified {
            MARK_SIZE + MARK_GAP
        } else {
            0.0
        };
        (chars as f32 * self.char_width + 2.0 * TAB_PADDING + BUTTON_ZONE + mark)
            .clamp(MIN_TAB_WIDTH, MAX_TAB_WIDTH)
    }

    /// The width the tabs need if they are laid out in one row.
    fn content_width(&self) -> f32 {
        (0..self.tabs.len()).map(|index| self.tab_width(index)).sum()
    }

    /// Whether the tabs do not fit in one row.
    ///
    /// Only the single-row arrangement can overflow: a column takes its own
    /// height, and the multi-line strip wraps instead.
    fn overflows(&self) -> bool {
        self.layout == TabLayout::Horizontal
            && self.width > 0.0
            && self.tabs.len() > 1
            && self.content_width() > self.width
    }

    /// How much of the strip the tabs may use.
    ///
    /// Once the arrows exist they take a zone at the trailing edge, so the tabs
    /// stop before them rather than sliding underneath: a tab half-covered by an
    /// arrow is a tab whose buttons cannot be clicked.
    fn usable_width(&self) -> f32 {
        if self.overflows() {
            (self.width - BUTTON_ZONE).max(MIN_TAB_WIDTH)
        } else {
            self.width
        }
    }

    /// How far the strip may scroll before its last tab is flush with the edge.
    fn max_offset(&self) -> f32 {
        // A strip that has not been placed yet has no edge to scroll against,
        // and scrolling against a width of zero is what parked the row past its
        // first tab at startup.
        if self.layout != TabLayout::Horizontal || self.width <= 0.0 {
            return 0.0;
        }
        (self.content_width() - self.usable_width()).max(0.0)
    }

    /// Pulls the scroll back inside the range the current tabs allow.
    fn clamp_offset(&mut self) {
        let max = self.max_offset();
        self.offset = if max > 0.0 {
            self.offset.clamp(0.0, max)
        } else {
            0.0
        };
    }

    /// The left edge of tab `index` with the strip unscrolled.
    fn left_of(&self, index: usize) -> f32 {
        (0..index).map(|earlier| self.tab_width(earlier)).sum()
    }

    /// Scrolls the strip far enough to show the active tab.
    ///
    /// Opening or cycling to a document that sits past the last visible tab has
    /// to bring it in, or the keyboard reaches tabs the pointer cannot.
    fn ensure_active_visible(&mut self) {
        if self.layout != TabLayout::Horizontal || self.tabs.is_empty() || self.width <= 0.0 {
            return;
        }
        let left = self.left_of(self.active);
        let right = left + self.tab_width(self.active);
        let usable = self.usable_width();
        if left < self.offset {
            self.offset = left;
        }
        if right > self.offset + usable {
            self.offset = right - usable;
        }
        self.clamp_offset();
    }

    /// Scrolls by one press of an arrow. `direction` is `-1` for left, `1` for
    /// right, and the return value says whether anything moved.
    ///
    /// A press moves three quarters of the visible width, so the tab that was at
    /// the edge stays in view as a landmark rather than being scrolled away.
    pub fn scroll_step(&mut self, direction: i32) -> bool {
        if direction == 0 || !self.overflows() {
            return false;
        }
        let step = (self.usable_width() * 0.75).max(MIN_TAB_WIDTH * 0.5);
        let before = self.offset;
        self.offset += direction.signum() as f32 * step;
        self.clamp_offset();
        self.offset != before
    }

    /// The two arrow buttons at the trailing edge, when the strip has them.
    ///
    /// `None` while the tabs fit: an arrow that scrolls nothing is a button that
    /// looks broken, so it only exists when there is somewhere to go.
    fn arrow_rects(&self, line_height: f32) -> Option<[(f32, f32, f32, f32); 2]> {
        if !self.overflows() {
            return None;
        }
        let row = self.row_height(line_height);
        let inset = ((row - BUTTON_SIZE) * 0.5).max(0.0);
        let left = self.width - BUTTON_ZONE;
        Some([
            (left, inset, BUTTON_SIZE, BUTTON_SIZE),
            (
                left + BUTTON_SIZE + BUTTON_GAP,
                inset,
                BUTTON_SIZE,
                BUTTON_SIZE,
            ),
        ])
    }

    /// The scroll arrow under `(x, y)`: `-1` left, `1` right.
    pub fn arrow_at(&self, x: f32, y: f32, line_height: f32) -> Option<i32> {
        let rects = self.arrow_rects(line_height)?;
        let x = x - self.left;
        let y = y - self.top;
        for (index, (left, top, width, height)) in rects.into_iter().enumerate() {
            if x >= left && x < left + width && y >= top && y < top + height {
                return Some(if index == 0 { -1 } else { 1 });
            }
        }
        None
    }

    /// Where a tab dragged past the end of its row has to land.
    ///
    /// [`TabBar::tab_at`] answers nothing over the empty space to the right of
    /// the last tab, so a tab dragged past its neighbours would stop one short of
    /// the end. `None` for a pointer that is not on the row at all, so a drag
    /// that wanders off the strip moves nothing.
    pub fn reorder_end(&self, x: f32, y: f32, line_height: f32) -> Option<usize> {
        if !self.is_shown() || self.tabs.is_empty() {
            return None;
        }
        let slots = self.slots(line_height);
        let last = slots.last()?;
        let x = x - self.left;
        let y = y - self.top;
        if y < last.top || y >= last.top + last.height {
            return None;
        }
        (x >= last.left + last.width).then(|| slots.len() - 1)
    }

    /// Where a tab's title has to stop so the buttons stay legible.
    fn label_right(&self, slot: &Slot) -> f32 {
        self.pin_rect(slot).0 - LABEL_GAP
    }

    /// The rectangle of the close button inside a tab slot.
    ///
    /// Notepad++ centres the button vertically and hangs it off the trailing
    /// edge by the same inset, which is why the cross sits a little in from the
    /// corner rather than flush against it.
    fn close_rect(&self, slot: &Slot) -> (f32, f32, f32, f32) {
        let inset = ((slot.height - BUTTON_SIZE) * 0.5).max(0.0);
        (
            slot.left + slot.width - inset - BUTTON_SIZE,
            slot.top + inset,
            BUTTON_SIZE,
            BUTTON_SIZE,
        )
    }

    /// The rectangle of the pin button inside a tab slot.
    fn pin_rect(&self, slot: &Slot) -> (f32, f32, f32, f32) {
        let (close_x, y, w, h) = self.close_rect(slot);
        (close_x - BUTTON_GAP - BUTTON_SIZE, y, w, h)
    }

    /// The rectangle of the unsaved pencil inside a tab slot.
    fn mark_rect(&self, slot: &Slot) -> (f32, f32, f32, f32) {
        (
            slot.left + TAB_PADDING,
            slot.top + (slot.height - MARK_SIZE) * 0.5,
            MARK_SIZE,
            MARK_SIZE,
        )
    }

    /// Where a tab's title starts, relative to the strip's corner.
    fn title_left(&self, index: usize, slot: &Slot) -> f32 {
        let mark = if self.tabs.get(index).is_some_and(|tab| tab.modified) {
            MARK_SIZE + MARK_GAP
        } else {
            0.0
        };
        slot.left + TAB_PADDING + mark
    }

    /// Where every tab sits, relative to the strip's own corner.
    ///
    /// The single source of truth for the strip's geometry: the rectangles, the
    /// titles and the hit test all read this, so none of them can drift from the
    /// others.
    fn slots(&self, line_height: f32) -> Vec<Slot> {
        let row_height = self.row_height(line_height);
        let mut slots = Vec::with_capacity(self.tabs.len());
        match self.layout {
            TabLayout::Horizontal => {
                // The row starts under the left edge until the strip is
                // scrolled, then the tabs that scrolled off keep their places
                // to the left of it and are clipped by the strip.
                let mut left = -self.offset;
                for i in 0..self.tabs.len() {
                    let width = self.tab_width(i);
                    slots.push(Slot {
                        left,
                        top: 0.0,
                        width,
                        height: row_height,
                    });
                    left += width;
                }
            }
            TabLayout::MultiLine => {
                let mut left = 0.0;
                let mut row = 0;
                for i in 0..self.tabs.len() {
                    let width = self.tab_width(i);
                    // Wrap only when the tab would overflow and there is another
                    // row to wrap onto. Past the cap the tabs keep going on the
                    // last row and are clipped, which is what single-row mode
                    // does with every tab that does not fit.
                    if left > 0.0 && left + width > self.width && row + 1 < MAX_ROWS {
                        row += 1;
                        left = 0.0;
                    }
                    slots.push(Slot {
                        left,
                        top: row as f32 * row_height,
                        width,
                        height: row_height,
                    });
                    left += width;
                }
            }
            TabLayout::Vertical => {
                let width = self.width();
                for i in 0..self.tabs.len() {
                    slots.push(Slot {
                        left: 0.0,
                        top: i as f32 * row_height,
                        width,
                        height: row_height,
                    });
                }
            }
        }
        slots
    }

    /// The tab under `(x, y)`, if any.
    ///
    /// Both coordinates, because a multi-line strip has rows and a vertical one
    /// has a column: a hit test on `x` alone would report a tab in the wrong row.
    pub fn tab_at(&self, x: f32, y: f32, line_height: f32) -> Option<usize> {
        let x = x - self.left;
        let y = y - self.top;
        // Under the arrows there is no tab to hit, however far the row runs
        // underneath them.
        if self.overflows() && (x < 0.0 || x >= self.usable_width()) {
            return None;
        }
        self.slots(line_height)
            .into_iter()
            .position(|slot| {
                x >= slot.left
                    && x < slot.left + slot.width
                    && y >= slot.top
                    && y < slot.top + slot.height
            })
    }

    /// The tab whose close button is under `(x, y)`, if any.
    ///
    /// Only the active tab carries buttons, which is Notepad++'s default: its
    /// `TAB_INACTIVETABSHOWBUTTON` flag is off, so an inactive tab draws an
    /// empty image where the cross would be. Hovering an inactive tab is
    /// therefore a plain tab click, not a close.
    pub fn close_at(&self, x: f32, y: f32, line_height: f32) -> Option<usize> {
        self.button_at(x, y, line_height, Self::close_rect)
    }

    /// The tab whose pin button is under `(x, y)`, if any.
    pub fn pin_at(&self, x: f32, y: f32, line_height: f32) -> Option<usize> {
        self.button_at(x, y, line_height, Self::pin_rect)
    }

    /// Shared hit test for the two buttons on the active tab.
    fn button_at(
        &self,
        x: f32,
        y: f32,
        line_height: f32,
        rect: fn(&Self, &Slot) -> (f32, f32, f32, f32),
    ) -> Option<usize> {
        if !self.is_shown() {
            return None;
        }
        let slot = self.slots(line_height).into_iter().nth(self.active)?;
        let (left, top, width, height) = rect(self, &slot);
        let x = x - self.left;
        let y = y - self.top;
        if x >= left && x < left + width && y >= top && y < top + height {
            Some(self.active)
        } else {
            None
        }
    }

    /// Handles a pointer move over the strip. Returns true when it landed on a
    /// tab or one of its buttons.
    pub fn hover_at(&mut self, x: f32, y: f32, line_height: f32) -> bool {
        let hover = if let Some(direction) = self.arrow_at(x, y, line_height) {
            Some(if direction < 0 {
                Hover::ScrollLeft
            } else {
                Hover::ScrollRight
            })
        } else if self.close_at(x, y, line_height).is_some() {
            Some(Hover::Close)
        } else if self.pin_at(x, y, line_height).is_some() {
            Some(Hover::Pin)
        } else {
            self.tab_at(x, y, line_height).map(Hover::Tab)
        };
        let hit = hover.is_some();
        self.hover = hover;
        hit
    }

    /// Clears the highlight, for when the pointer leaves the strip.
    pub fn clear_hover(&mut self) {
        self.hover = None;
    }

    /// Rebuilds the shaped titles if they changed.
    pub fn prepare(&mut self, font_system: &mut FontSystem) {
        if !self.stale {
            return;
        }
        self.stale = false;
        self.buffers.clear();
        for tab in &self.tabs {
            let mut buffer = Buffer::new_empty(self.metrics);
            buffer.set_size(None, None);
            buffer.set_text(
                &tab.title,
                &Attrs::new().family(self.family),
                shaping_for(&tab.title),
                None,
            );
            buffer.shape_until_scroll(font_system, false);
            self.buffers.push(buffer);
        }
    }

    /// The width of the strip's own background.
    ///
    /// A row of tabs spans the pane, so a multi-line strip's rows are all the
    /// same length even when the last one holds fewer tabs. A column is only as
    /// wide as the column.
    fn block_width(&self) -> f32 {
        if self.layout.is_vertical() {
            self.width()
        } else {
            self.width
        }
    }

    /// The height of the strip's own background.
    ///
    /// The rows for the two horizontal arrangements, and the tabs' own extent for
    /// the column, which has no fixed height.
    fn block_height(&self, line_height: f32) -> f32 {
        if self.layout.is_vertical() {
            self.tabs.len() as f32 * self.row_height(line_height)
        } else {
            self.height(line_height)
        }
    }

    /// Queues the strip's background and one rectangle per tab.
    pub fn push_quads(&self, quads: &mut QuadRenderer, line_height: f32, theme: &Theme) {
        if !self.is_shown() {
            return;
        }
        let slots = self.slots(line_height);
        let block_height = self.block_height(line_height);
        let block_width = self.block_width();
        quads.push(
            self.left,
            self.top,
            block_width,
            block_height,
            theme.tab_bar_background.to_linear(),
        );

        for (i, tab) in self.tabs.iter().enumerate() {
            let Some(slot) = slots.get(i) else { continue };
            // A scrolled-out tab keeps its place in the row but is not painted
            // past the strip's own ends, where it would sit under the arrows or
            // bleed over the pane beside it.
            let (start, span) = if self.overflows() {
                let right = (slot.left + slot.width).min(self.usable_width());
                let start = slot.left.max(0.0);
                (start, (right - start).max(0.0))
            } else {
                (slot.left, slot.width)
            };
            if span <= 0.0 {
                continue;
            }
            let left = self.left + start;
            let top = self.top + slot.top;
            let base = if i == self.active {
                theme.background
            } else {
                theme.tab_inactive
            };
            // A painted tab keeps its hue but is pulled toward the palette, so
            // the same colour is legible on both backgrounds.
            let color = match tab.colour.rgb() {
                Some(paint) => mix(paint, base, COLOUR_MIX),
                None => base,
            };
            quads.push(left, top, span, slot.height, color.to_linear());
            if i != self.active {
                // An inactive tab under the pointer lifts a shade, which is the
                // only feedback the strip gives before a click.
                if self.hover == Some(Hover::Tab(i)) {
                    quads.push(left, top, span, slot.height, theme.tab_hover.to_linear());
                }
                continue;
            }
            // The orange bar along the leading edge of the active tab: the top
            // for a row, the left for a column, so the bar always faces the text
            // the tab belongs to.
            if self.layout.is_vertical() {
                quads.push(
                    left,
                    top,
                    TOP_BAR,
                    slot.height,
                    theme.tab_active_rule.to_linear(),
                );
            } else {
                quads.push(
                    left,
                    top,
                    span,
                    TOP_BAR,
                    theme.tab_active_rule.to_linear(),
                );
            }
            // A pressed-looking well behind whichever button the pointer is on.
            for (rect, hovered) in [
                (self.close_rect(slot), self.hover == Some(Hover::Close)),
                (self.pin_rect(slot), self.hover == Some(Hover::Pin)),
            ] {
                if hovered {
                    quads.push(
                        self.left + rect.0,
                        self.top + rect.1,
                        rect.2,
                        rect.3,
                        theme.tab_hover.to_linear(),
                    );
                }
            }
        }
        // A pressed-looking well behind whichever arrow the pointer is on, so
        // the strip answers before the click rather than after it.
        if let Some(rects) = self.arrow_rects(line_height) {
            for (index, rect) in rects.into_iter().enumerate() {
                let hovered = matches!(
                    (index, self.hover),
                    (0, Some(Hover::ScrollLeft)) | (1, Some(Hover::ScrollRight))
                );
                if hovered {
                    quads.push(
                        self.left + rect.0,
                        self.top + rect.1,
                        rect.2,
                        rect.3,
                        theme.tab_hover.to_linear(),
                    );
                }
            }
        }
    }

    /// Whether a mark `width` wide at `x` falls inside the part of the row the
    /// tabs occupy.
    ///
    /// Marks are glyphs, not quads, so they cannot be clipped to the strip the
    /// way a tab's fill is. A pencil on a tab that scrolled off would otherwise
    /// appear to belong to whatever sits beside the strip, so it is dropped.
    fn mark_is_visible(&self, x: f32, width: f32) -> bool {
        if !self.overflows() {
            return true;
        }
        x >= 0.0 && x + width <= self.usable_width()
    }

    /// Queues the marks on the tabs: the pencil on a dirty document, and the
    /// close cross and pin on the active one.
    ///
    /// Drawn through the icon renderer rather than as rectangles because a
    /// cross and a pin are diagonals, and the quad renderer only draws
    /// axis-aligned boxes. The atlas is already built for the toolbar, so the
    /// four extra glyphs cost nothing at draw time.
    pub fn push_icons(
        &self,
        icons: &mut IconRenderer,
        line_height: f32,
        theme: &Theme,
    ) {
        if !self.is_shown() {
            return;
        }
        let slots = self.slots(line_height);
        for (i, tab) in self.tabs.iter().enumerate() {
            let Some(slot) = slots.get(i) else { continue };
            if tab.modified {
                let Some(icon) = icons::index("tab_unsaved") else {
                    continue;
                };
                let (x, y, w, h) = self.mark_rect(slot);
                if !self.mark_is_visible(x, w) {
                    continue;
                }
                icons.push(
                    icon,
                    true,
                    self.left + x,
                    self.top + y,
                    w,
                    h,
                    theme.tab_unsaved.to_linear(),
                );
            }
            if i != self.active {
                continue;
            }
            let Some(close) = icons::index("tab_close") else {
                continue;
            };
            let (x, y, w, h) = self.close_rect(slot);
            if !self.mark_is_visible(x, w) {
                continue;
            }
            icons.push(
                close,
                false,
                self.left + x,
                self.top + y,
                w,
                h,
                theme.tab_button.to_linear(),
            );
            let pin = if tab.pinned {
                icons::index("tab_pin_on")
            } else {
                icons::index("tab_pin")
            };
            let Some(pin) = pin else { continue };
            let (x, y, w, h) = self.pin_rect(slot);
            if !self.mark_is_visible(x, w) {
                continue;
            }
            icons.push(
                pin,
                tab.pinned,
                self.left + x,
                self.top + y,
                w,
                h,
                theme.tab_button.to_linear(),
            );
        }
        // The two scroll arrows, last so they sit over the strip's own edge
        // rather than under a tab that scrolled past them.
        if let Some(rects) = self.arrow_rects(line_height) {
            for (index, name) in ["tab_left", "tab_right"].into_iter().enumerate() {
                let Some(icon) = icons::index(name) else {
                    continue;
                };
                let (x, y, w, h) = rects[index];
                icons.push(
                    icon,
                    false,
                    self.left + x,
                    self.top + y,
                    w,
                    h,
                    theme.tab_button.to_linear(),
                );
            }
        }
    }

    /// The text areas for the tab titles.
    ///
    /// Each title is clipped to its own tab, and inside that tab to the space
    /// left of the two buttons. A single clip for the whole strip -- which is
    /// what this was -- let a long name run under the pin and the cross, and
    /// under the next tab as well.
    pub fn text_areas<'a>(&'a self, line_height: f32, theme: &Theme) -> Vec<TextArea<'a>> {
        if !self.is_shown() {
            return Vec::new();
        }
        let slots = self.slots(line_height);
        let mut areas = Vec::with_capacity(self.buffers.len());
        for (i, buffer) in self.buffers.iter().enumerate() {
            let Some(slot) = slots.get(i) else { continue };
            let left = self.left + self.title_left(i, slot);
            // The arrows take the strip's trailing edge, so a tab that runs
            // under them is clipped there as well as at its own buttons.
            let right = (self.left + self.label_right(slot)).min(self.left + self.usable_width());
            let top = self.top + slot.top;
            if right <= left {
                continue;
            }
            areas.push(TextArea {
                buffer,
                left,
                top: top + 4.0,
                scale: 1.0,
                bounds: TextBounds {
                    left: left as i32,
                    top: top as i32,
                    right: right as i32,
                    bottom: (top + slot.height) as i32,
                },
                default_color: if i == self.active {
                    theme.tab_active_text.to_cosmic()
                } else {
                    theme.tab_inactive_text.to_cosmic()
                },
                custom_glyphs: &[],
            });
        }
        areas
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar() -> TabBar {
        TabBar::new(Metrics::new(15.0, 20.0), Family::Monospace, 8.0)
    }

    fn strip(titles: &[&str]) -> Vec<TabInfo> {
        titles.iter().map(|t| TabInfo::new(*t)).collect()
    }

    #[test]
    fn every_colour_round_trips_through_its_key() {
        for colour in TabColour::ALL {
            assert_eq!(TabColour::from_key(colour.key()), Some(colour));
        }
        assert_eq!(TabColour::from_key("chartreuse"), None);
    }

    #[test]
    fn stepping_a_colour_wraps_back_to_none() {
        assert_eq!(TabColour::None.next(), TabColour::Yellow);
        assert_eq!(TabColour::Purple.next(), TabColour::None);
    }

    #[test]
    fn only_none_has_no_colour_of_its_own() {
        assert_eq!(TabColour::None.rgb(), None);
        for colour in TabColour::ALL.into_iter().filter(|c| *c != TabColour::None) {
            assert!(colour.rgb().is_some(), "{colour:?}");
        }
    }

    #[test]
    fn the_colours_are_distinguishable_from_each_other() {
        let mut seen = Vec::new();
        for colour in TabColour::ALL.into_iter().filter(|c| *c != TabColour::None) {
            let rgb = colour.rgb().expect("a colour");
            assert!(!seen.contains(&rgb), "{colour:?} repeats a colour");
            seen.push(rgb);
        }
    }

    #[test]
    fn a_single_tab_still_has_a_strip() {
        // Notepad++ keeps the tab bar up for one document, and the close and pin
        // buttons live on the tab, so a lone document would lose them.
        let mut bar = bar();
        bar.set_tabs(strip(&["a.rs"]), 0);
        assert!(bar.height(20.0) > 0.0);
    }

    #[test]
    fn an_empty_strip_has_no_height() {
        let mut bar = bar();
        bar.set_tabs(Vec::new(), 0);
        assert_eq!(bar.height(20.0), 0.0);
    }

    #[test]
    fn two_tabs_have_a_strip() {
        let mut bar = bar();
        bar.set_tabs(strip(&["a.rs", "b.rs"]), 0);
        assert!(bar.height(20.0) > 0.0);
    }

    #[test]
    fn pinning_a_tab_does_not_resize_it() {
        // The pin lives in the button zone every tab already reserves, so
        // pinning must not reflow the strip under the pointer.
        let long = "document.rs";
        let mut bar = bar();
        bar.set_tabs(strip(&[long, "b.rs"]), 0);
        let plain = bar.tab_width(0);
        let mut pinned = strip(&[long, "b.rs"]);
        pinned[0].pinned = true;
        bar.set_tabs(pinned, 0);
        assert_eq!(bar.tab_width(0), plain);
    }

    #[test]
    fn a_dirty_tab_is_wider_than_a_clean_one() {
        // The pencil needs room of its own, or it lands on the first letter.
        let mut bar = bar();
        bar.set_tabs(strip(&["document.rs", "b.rs"]), 0);
        let clean = bar.tab_width(0);
        let mut dirty = strip(&["document.rs", "b.rs"]);
        dirty[0].modified = true;
        bar.set_tabs(dirty, 0);
        assert_eq!(bar.tab_width(0), clean + MARK_SIZE + MARK_GAP);
    }

    #[test]
    fn a_short_title_still_fits_beside_its_buttons() {
        // The minimum width leaves room for the two buttons and the padding, so
        // the cross cannot land on top of a short filename.
        let mut bar = bar();
        bar.set_tabs(strip(&["a.rs", "b.rs"]), 0);
        let needed = 4.0 * 8.0 + 2.0 * TAB_PADDING + BUTTON_ZONE;
        assert!(bar.tab_width(0) >= needed);
    }

    #[test]
    fn the_buttons_sit_inside_the_active_tab() {
        let mut bar = bar();
        bar.set_tabs(strip(&["a.rs", "b.rs"]), 0);
        bar.set_bounds(0.0, 0.0, 900.0);
        let slot = bar.slots(20.0)[0];
        for (left, top, width, height) in [bar.close_rect(&slot), bar.pin_rect(&slot)] {
            assert!(left >= slot.left, "a button starts left of its tab");
            assert!(left + width <= slot.left + slot.width, "a button overflows");
            assert!(top >= slot.top && top + height <= slot.top + slot.height);
        }
    }

    #[test]
    fn the_close_button_is_right_of_the_pin() {
        let mut bar = bar();
        bar.set_tabs(strip(&["a.rs", "b.rs"]), 0);
        let slot = bar.slots(20.0)[0];
        let (close, _, _, _) = bar.close_rect(&slot);
        let (pin, _, _, _) = bar.pin_rect(&slot);
        assert!(close > pin);
        assert_eq!(close - pin, BUTTON_SIZE + BUTTON_GAP);
    }

    #[test]
    fn a_click_on_the_cross_closes_the_active_tab() {
        let mut bar = bar();
        bar.set_tabs(strip(&["a.rs", "b.rs", "c.rs"]), 1);
        bar.set_bounds(0.0, 0.0, 900.0);
        let slot = bar.slots(20.0)[1];
        let (x, y, w, h) = bar.close_rect(&slot);
        assert_eq!(bar.close_at(x + w * 0.5, y + h * 0.5, 20.0), Some(1));
        assert_eq!(bar.pin_at(x + w * 0.5, y + h * 0.5, 20.0), None);
    }

    #[test]
    fn a_click_on_the_pin_pins_the_active_tab() {
        let mut bar = bar();
        bar.set_tabs(strip(&["a.rs", "b.rs"]), 0);
        bar.set_bounds(0.0, 0.0, 900.0);
        let slot = bar.slots(20.0)[0];
        let (x, y, w, h) = bar.pin_rect(&slot);
        assert_eq!(bar.pin_at(x + w * 0.5, y + h * 0.5, 20.0), Some(0));
        assert_eq!(bar.close_at(x + w * 0.5, y + h * 0.5, 20.0), None);
    }

    #[test]
    fn an_inactive_tab_has_no_buttons_to_click() {
        // Notepad++ leaves the buttons off inactive tabs by default, so a click
        // where the cross would be is a plain tab click.
        let mut bar = bar();
        bar.set_tabs(strip(&["a.rs", "b.rs"]), 0);
        bar.set_bounds(0.0, 0.0, 900.0);
        let slot = bar.slots(20.0)[1];
        let (x, y, w, h) = bar.close_rect(&slot);
        assert_eq!(bar.close_at(x + w * 0.5, y + h * 0.5, 20.0), None);
        assert_eq!(bar.tab_at(x + w * 0.5, y + h * 0.5, 20.0), Some(1));
    }

    #[test]
    fn hovering_the_cross_is_not_hovering_the_tab() {
        let mut bar = bar();
        bar.set_tabs(strip(&["a.rs", "b.rs"]), 0);
        bar.set_bounds(0.0, 0.0, 900.0);
        let slot = bar.slots(20.0)[0];
        let (x, y, w, h) = bar.close_rect(&slot);
        assert!(bar.hover_at(x + w * 0.5, y + h * 0.5, 20.0));
        assert_eq!(bar.hover, Some(Hover::Close));
        let (x, y, w, h) = bar.pin_rect(&slot);
        assert!(bar.hover_at(x + w * 0.5, y + h * 0.5, 20.0));
        assert_eq!(bar.hover, Some(Hover::Pin));
        bar.clear_hover();
        assert_eq!(bar.hover, None);
    }

    #[test]
    fn the_title_starts_after_the_pencil() {
        let mut bar = bar();
        bar.set_tabs(strip(&["a.rs", "b.rs"]), 0);
        let slot = bar.slots(20.0)[0];
        assert_eq!(bar.title_left(0, &slot), slot.left + TAB_PADDING);
        let mut dirty = strip(&["a.rs", "b.rs"]);
        dirty[0].modified = true;
        bar.set_tabs(dirty, 0);
        let slot = bar.slots(20.0)[0];
        assert_eq!(
            bar.title_left(0, &slot),
            slot.left + TAB_PADDING + MARK_SIZE + MARK_GAP
        );
    }

    #[test]
    fn a_click_maps_to_the_tab_under_it() {
        let mut bar = bar();
        bar.set_tabs(strip(&["a.rs", "b.rs", "c.rs"]), 0);
        bar.set_bounds(0.0, 0.0, 900.0);
        let first = bar.tab_width(0);
        assert_eq!(bar.tab_at(1.0, 1.0, 20.0), Some(0));
        assert_eq!(bar.tab_at(first + 1.0, 1.0, 20.0), Some(1));
        assert_eq!(bar.tab_at(first * 3.0 + 1.0, 1.0, 20.0), None);
    }

    #[test]
    fn the_strip_is_offset_by_its_bounds() {
        let mut bar = bar();
        bar.set_tabs(strip(&["a.rs", "b.rs"]), 0);
        bar.set_bounds(400.0, 60.0, 500.0);
        // Left of the pane is not a tab.
        assert_eq!(bar.tab_at(10.0, 61.0, 20.0), None);
        assert_eq!(bar.tab_at(401.0, 61.0, 20.0), Some(0));
    }

    #[test]
    fn the_strip_reports_the_tabs_it_holds() {
        let mut bar = bar();
        let mut tabs = strip(&["a.rs", "b.rs"]);
        tabs[1].colour = TabColour::Red;
        bar.set_tabs(tabs.clone(), 1);
        assert_eq!(bar.tabs(), tabs.as_slice());
    }

    #[test]
    fn an_ordinary_tab_starts_unpinned_and_unpainted() {
        let tab = TabInfo::new("a.rs");
        assert!(!tab.pinned);
        assert_eq!(tab.colour, TabColour::None);
        assert_eq!(tab.title, "a.rs");
    }

    #[test]
    fn every_layout_round_trips_through_its_key() {
        for layout in TabLayout::ALL {
            assert_eq!(TabLayout::from_key(layout.key()), Some(layout));
        }
        assert_eq!(TabLayout::from_key("diagonal"), None);
    }

    #[test]
    fn stepping_a_layout_wraps_back_to_the_first() {
        assert_eq!(TabLayout::Horizontal.next(), TabLayout::MultiLine);
        assert_eq!(TabLayout::Vertical.next(), TabLayout::Horizontal);
    }

    #[test]
    fn every_layout_has_a_distinct_label() {
        let mut seen = Vec::new();
        for layout in TabLayout::ALL {
            assert!(!layout.label().is_empty());
            assert!(!seen.contains(&layout.label()), "{layout:?} repeats");
            seen.push(layout.label());
        }
    }

    #[test]
    fn only_the_vertical_layout_is_vertical() {
        assert!(TabLayout::Vertical.is_vertical());
        assert!(!TabLayout::Horizontal.is_vertical());
        assert!(!TabLayout::MultiLine.is_vertical());
    }

    #[test]
    fn a_single_tab_still_has_a_column() {
        let mut bar = bar();
        bar.set_layout(TabLayout::Vertical);
        bar.set_tabs(strip(&["a.rs"]), 0);
        bar.set_bounds(0.0, 0.0, 900.0);
        assert!(bar.width() > 0.0);
        assert_eq!(bar.height(20.0), 0.0);
    }

    #[test]
    fn the_vertical_layout_takes_width_and_no_height() {
        let mut bar = bar();
        bar.set_layout(TabLayout::Vertical);
        bar.set_tabs(strip(&["a.rs", "b.rs", "c.rs"]), 0);
        bar.set_bounds(0.0, 0.0, 900.0);
        assert_eq!(bar.height(20.0), 0.0);
        assert!(bar.width() > 0.0);
    }

    #[test]
    fn the_horizontal_layouts_take_height_and_no_width() {
        for layout in [TabLayout::Horizontal, TabLayout::MultiLine] {
            let mut bar = bar();
            bar.set_layout(layout);
            bar.set_tabs(strip(&["a.rs", "b.rs"]), 0);
            bar.set_bounds(0.0, 0.0, 900.0);
            assert!(bar.height(20.0) > 0.0, "{layout:?}");
            assert_eq!(bar.width(), 0.0, "{layout:?}");
        }
    }

    #[test]
    fn a_vertical_column_never_takes_more_than_half_the_pane() {
        let mut bar = bar();
        bar.set_layout(TabLayout::Vertical);
        bar.set_tabs(strip(&["a.rs", "b.rs"]), 0);
        bar.set_bounds(0.0, 0.0, 200.0);
        assert!(bar.width() <= 100.0, "{}", bar.width());
    }

    #[test]
    fn a_vertical_tab_is_a_row_of_the_column() {
        let mut bar = bar();
        bar.set_layout(TabLayout::Vertical);
        bar.set_tabs(strip(&["a.rs", "b.rs", "c.rs"]), 0);
        bar.set_bounds(0.0, 0.0, 900.0);
        let row = 20.0 + 8.0;
        // Every tab starts at the column's left edge, one row apart.
        assert_eq!(bar.tab_at(1.0, 1.0, 20.0), Some(0));
        assert_eq!(bar.tab_at(1.0, row + 1.0, 20.0), Some(1));
        assert_eq!(bar.tab_at(1.0, 2.0 * row + 1.0, 20.0), Some(2));
        // Past the last row is not a tab.
        assert_eq!(bar.tab_at(1.0, 3.0 * row + 1.0, 20.0), None);
    }

    #[test]
    fn a_vertical_tab_does_not_extend_past_the_column() {
        let mut bar = bar();
        bar.set_layout(TabLayout::Vertical);
        bar.set_tabs(strip(&["a.rs", "b.rs"]), 0);
        bar.set_bounds(0.0, 0.0, 900.0);
        let column = bar.width();
        assert_eq!(bar.tab_at(column + 1.0, 1.0, 20.0), None);
    }

    #[test]
    fn a_multi_line_strip_wraps_when_it_runs_out_of_room() {
        let mut bar = bar();
        bar.set_layout(TabLayout::MultiLine);
        bar.set_tabs(strip(&["a.rs", "b.rs", "c.rs", "d.rs"]), 0);
        // Two tabs fit per row at this width, so four tabs need two rows.
        bar.set_bounds(0.0, 0.0, 2.0 * MIN_TAB_WIDTH);
        let row = 20.0 + 8.0;
        assert_eq!(bar.height(20.0), 2.0 * row);
        assert_eq!(bar.tab_at(1.0, 1.0, 20.0), Some(0));
        assert_eq!(bar.tab_at(1.0, row + 1.0, 20.0), Some(2));
    }

    #[test]
    fn a_multi_line_strip_stops_growing_at_the_cap() {
        let mut bar = bar();
        bar.set_layout(TabLayout::MultiLine);
        let titles: Vec<String> = (0..40).map(|i| format!("doc{i}.rs")).collect();
        let tabs: Vec<TabInfo> = titles.iter().map(TabInfo::new).collect();
        bar.set_tabs(tabs, 0);
        bar.set_bounds(0.0, 0.0, MIN_TAB_WIDTH);
        let row = 20.0 + 8.0;
        assert_eq!(bar.height(20.0), MAX_ROWS as f32 * row);
    }

    #[test]
    fn a_single_row_strip_never_wraps() {
        let mut bar = bar();
        bar.set_tabs(strip(&["a.rs", "b.rs", "c.rs", "d.rs"]), 0);
        bar.set_bounds(0.0, 0.0, MIN_TAB_WIDTH);
        let row = 20.0 + 8.0;
        assert_eq!(bar.height(20.0), row);
        // The tabs past the edge are still placed, just clipped.
        assert_eq!(bar.slots(20.0).len(), 4);
        assert!(bar.slots(20.0)[3].left >= MIN_TAB_WIDTH);
    }

    #[test]
    fn a_tab_can_be_moved_along_its_strip() {
        let mut tabs = strip(&["a.rs", "b.rs", "c.rs", "d.rs"]);
        move_tab(&mut tabs, 0, 2);
        let order: Vec<&str> = tabs.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(order, ["b.rs", "c.rs", "a.rs", "d.rs"]);
        move_tab(&mut tabs, 3, 0);
        let order: Vec<&str> = tabs.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(order, ["d.rs", "b.rs", "c.rs", "a.rs"]);
    }

    #[test]
    fn moving_a_tab_nowhere_leaves_it_alone() {
        let mut tabs = strip(&["a.rs", "b.rs"]);
        move_tab(&mut tabs, 1, 1);
        move_tab(&mut tabs, 0, 7);
        move_tab(&mut tabs, 9, 0);
        let order: Vec<&str> = tabs.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(order, ["a.rs", "b.rs"]);
    }

    #[test]
    fn a_drag_past_the_last_tab_lands_on_the_end() {
        let mut bar = bar();
        bar.set_tabs(strip(&["a.rs", "b.rs", "c.rs", "d.rs"]), 0);
        bar.set_bounds(0.0, 0.0, 900.0);
        let last = *bar.slots(20.0).last().expect("four slots");
        let beyond = last.left + last.width + 20.0;
        assert_eq!(bar.reorder_end(beyond, 10.0, 20.0), Some(3));
        // Inside the last tab there is a tab to hit, so the end rule abstains.
        assert_eq!(bar.reorder_end(last.left + 2.0, 10.0, 20.0), None);
        // Below the row there is nothing to drag onto.
        assert_eq!(bar.reorder_end(beyond, 60.0, 20.0), None);
    }

    #[test]
    fn a_long_title_stops_before_the_pin() {
        // The titles used to be clipped to the strip, not to their own tab, so a
        // long name ran under the pin and the cross.
        let mut bar = bar();
        let long = "a_really_rather_long_document_name.rs";
        bar.set_tabs(strip(&[long, "b.rs"]), 0);
        bar.set_bounds(0.0, 0.0, 900.0);
        let slot = bar.slots(20.0)[0];
        let (pin, _, _, _) = bar.pin_rect(&slot);
        assert_eq!(bar.label_right(&slot), pin - LABEL_GAP);
        assert!(bar.label_right(&slot) < slot.left + slot.width);
        // The name is wider than the room left for it, so the clip is doing work
        // rather than merely enclosing a title that already fitted.
        let span = bar.label_right(&slot) - bar.title_left(0, &slot);
        assert!(long.len() as f32 * bar.char_width > span);
    }

    #[test]
    fn the_arrows_appear_only_when_the_tabs_do_not_fit() {
        let mut bar = bar();
        bar.set_tabs(strip(&["a.rs", "b.rs"]), 0);
        bar.set_bounds(0.0, 0.0, 900.0);
        assert!(bar.arrow_rects(20.0).is_none());
        assert_eq!(bar.arrow_at(892.0, 10.0, 20.0), None);
        bar.set_tabs(strip(&["a.rs", "b.rs", "c.rs", "d.rs", "e.rs", "f.rs"]), 0);
        bar.set_bounds(0.0, 0.0, 200.0);
        assert!(bar.arrow_rects(20.0).is_some());
    }

    #[test]
    fn scrolling_stops_at_both_ends() {
        let mut bar = bar();
        bar.set_tabs(strip(&["a.rs", "b.rs", "c.rs", "d.rs", "e.rs", "f.rs"]), 0);
        bar.set_bounds(0.0, 0.0, 200.0);
        assert!(!bar.scroll_step(-1));
        assert_eq!(bar.offset, 0.0);
        assert!(bar.scroll_step(1));
        assert!(bar.offset > 0.0);
        let mut presses = 0;
        while bar.scroll_step(1) {
            presses += 1;
            assert!(presses < 20, "the strip never reached its end");
        }
        assert_eq!(bar.offset, bar.max_offset());
        assert!(!bar.scroll_step(1));
        assert!(bar.scroll_step(-1));
    }

    #[test]
    fn activating_a_far_tab_scrolls_it_into_view() {
        let mut bar = bar();
        bar.set_tabs(strip(&["a.rs", "b.rs", "c.rs", "d.rs", "e.rs", "f.rs"]), 5);
        bar.set_bounds(0.0, 0.0, 200.0);
        let left = bar.left_of(5);
        assert!(left >= bar.offset, "the active tab starts left of the strip");
        assert!(left + bar.tab_width(5) <= bar.offset + bar.usable_width() + 0.01);
    }

    #[test]
    fn a_click_on_an_arrow_says_which_way() {
        let mut bar = bar();
        bar.set_tabs(strip(&["a.rs", "b.rs", "c.rs", "d.rs", "e.rs", "f.rs"]), 0);
        bar.set_bounds(10.0, 5.0, 200.0);
        let rects = bar.arrow_rects(20.0).expect("two arrows");
        for (index, expected) in [-1, 1].into_iter().enumerate() {
            let (x, y, w, h) = rects[index];
            let hit = bar.arrow_at(10.0 + x + w * 0.5, 5.0 + y + h * 0.5, 20.0);
            assert_eq!(hit, Some(expected));
        }
    }

    #[test]
    fn a_tab_under_the_arrows_cannot_be_clicked() {
        // The row runs on under the arrows; a click there is a scroll, so the
        // hit test must not also report the tab behind them.
        let mut bar = bar();
        bar.set_tabs(strip(&["a.rs", "b.rs", "c.rs", "d.rs", "e.rs", "f.rs"]), 0);
        bar.set_bounds(0.0, 0.0, 200.0);
        assert_eq!(bar.tab_at(170.0, 10.0, 20.0), None);
        assert_eq!(bar.tab_at(195.0, 10.0, 20.0), None);
        assert!(bar.tab_at(100.0, 10.0, 20.0).is_some());
    }

    #[test]
    fn a_multi_line_strip_that_fits_is_one_row() {
        let mut bar = bar();
        bar.set_layout(TabLayout::MultiLine);
        bar.set_tabs(strip(&["a.rs", "b.rs"]), 0);
        bar.set_bounds(0.0, 0.0, 900.0);
        assert_eq!(bar.height(20.0), 20.0 + 8.0);
    }

    #[test]
    fn the_layout_survives_a_change_of_tabs() {
        let mut bar = bar();
        bar.set_layout(TabLayout::Vertical);
        bar.set_tabs(strip(&["a.rs", "b.rs"]), 0);
        bar.set_tabs(strip(&["c.rs", "d.rs", "e.rs"]), 1);
        assert_eq!(bar.layout(), TabLayout::Vertical);
    }
}
