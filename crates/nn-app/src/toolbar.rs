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

//! The toolbar.
//!
//! A row of icon buttons under the menu bar, one per command a user reaches for
//! without opening a menu. The set, the order and the nine groups are
//! Notepad++'s own, so a user who knows where Print sits knows where it sits
//! here.
//!
//! The icons are vector drawings — see [`crate::icons`] — rasterised at the size
//! the display asks for. Nothing here is a bitmap, which is what keeps the bar
//! sharp on a high-density screen and lets the icons follow the theme.
//!
//! The bar is a pure function of its button table: layout, hit testing, and the
//! draw list all walk the same list in the same order, so a button cannot be
//! drawn somewhere it cannot be clicked.

use crate::i18n::Catalog;
use crate::iconrender::IconRenderer;
use crate::icons;
use crate::menu::Action;
use crate::quad::QuadRenderer;
use crate::theme::Theme;

/// Horizontal padding inside a button, on each side of its icon.
///
/// Notepad++ sizes its buttons to the icon (`TB_SETBUTTONSIZE` with the icon
/// side), so the padding here is deliberately small: at 4px a 23px icon sits in
/// a 31px button, and the whole 32-button bar comes to 1032px, which fits the
/// default window. At 5px it came to 1137px and the last group was clipped.
const BUTTON_PADDING: f32 = 4.0;
/// Vertical padding above and below the icons.
const BAR_PADDING: f32 = 4.0;
/// The gap a group separator occupies.
const SEPARATOR_WIDTH: f32 = 8.0;
/// The smallest an icon is drawn, in pixels.
const MIN_ICON: f32 = 16.0;
/// The largest an icon is drawn, in pixels.
///
/// Past this the bar is taller than the text it sits above, and a toolbar that
/// costs more room than the document is a toolbar that gets turned off.
const MAX_ICON: f32 = 28.0;

/// One button on the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Button {
    /// The command's name, used for the accessible label and for tests.
    pub label: &'static str,
    /// The icon's key in [`crate::icons::ICONS`].
    pub icon: &'static str,
    /// What clicking it runs.
    pub action: Action,
    /// Whether a group separator is drawn before it.
    pub separator_before: bool,
}

impl Button {
    const fn new(label: &'static str, icon: &'static str, action: Action) -> Self {
        Self {
            label,
            icon,
            action,
            separator_before: false,
        }
    }

    const fn after_separator(label: &'static str, icon: &'static str, action: Action) -> Self {
        Self {
            label,
            icon,
            action,
            separator_before: true,
        }
    }
}

/// The buttons, left to right, in Notepad++'s order.
///
/// The nine groups are file, clipboard, history, search, zoom, scroll sync,
/// view toggles, panels, monitoring, and the macro recorder. They are separated
/// because they are reached for in different moods: the first three are editing,
/// the middle three are looking, and the last two are running something.
pub const BUTTONS: &[Button] = &[
    Button::new("New", "new", Action::New),
    Button::new("Open", "open", Action::Open),
    Button::new("Save", "save", Action::Save),
    Button::new("Save All", "saveall", Action::SaveAll),
    Button::new("Close", "close", Action::CloseTab),
    Button::new("Close All", "closeall", Action::CloseAll),
    Button::new("Print", "print", Action::Print),
    Button::after_separator("Cut", "cut", Action::Cut),
    Button::new("Copy", "copy", Action::Copy),
    Button::new("Paste", "paste", Action::Paste),
    Button::after_separator("Undo", "undo", Action::Undo),
    Button::new("Redo", "redo", Action::Redo),
    Button::after_separator("Find", "find", Action::Find),
    Button::new("Replace", "replace", Action::Replace),
    Button::after_separator("Zoom In", "zoom_in", Action::ZoomIn),
    Button::new("Zoom Out", "zoom_out", Action::ZoomOut),
    Button::after_separator("Sync Vertical Scroll", "sync_v", Action::SyncScrollV),
    Button::new("Sync Horizontal Scroll", "sync_h", Action::SyncScrollH),
    Button::after_separator("Word Wrap", "wrap", Action::ToggleWrap),
    Button::new("Show All Characters", "all_chars", Action::ShowAllChars),
    Button::new(
        "Show Indent Guide",
        "indent_guide",
        Action::ShowIndentGuides,
    ),
    Button::after_separator("User Defined Language", "udl", Action::DefineLanguage),
    Button::new("Document Map", "doc_map", Action::DocumentMap),
    Button::new("Document List", "doc_list", Action::DocumentList),
    Button::new("Function List", "func_list", Action::ToggleFunctionList),
    Button::new("Folder as Workspace", "file_browser", Action::FolderAsWorkspace),
    Button::after_separator("Monitoring (tail -f)", "monitoring", Action::Monitoring),
    Button::after_separator("Start Recording", "start_record", Action::ToggleRecording),
    Button::new("Stop Recording", "stop_record", Action::StopRecording),
    Button::new("Playback", "playback", Action::PlayMacro),
    Button::new("Run Multiple Macros", "run_multiple", Action::RunMacroMultiple),
    Button::new("Save Current Macro", "save_macro", Action::SaveMacro),
];

/// Every label the bar can show, in English.
///
/// Read by the translation-coverage test, so a button added without a
/// translation fails the suite rather than showing an English word in an
/// otherwise translated window.
pub fn labels() -> Vec<&'static str> {
    BUTTONS.iter().map(|button| button.label).collect()
}

/// The strip of command buttons.
pub struct Toolbar {
    enabled: bool,
    /// The button the pointer is over, if any.
    hover: Option<usize>,
    /// The language's catalog, when one is set.
    ///
    /// Only the labels under the pointer go through it: the icons are
    /// drawings, so nothing on the bar itself is text.
    catalog: Option<Catalog>,
}

impl Toolbar {
    /// Builds a bar with the default button set.
    pub fn new() -> Self {
        Self {
            enabled: true,
            hover: None,
            catalog: None,
        }
    }

    /// Sets the language whose translations the bar's labels come from.
    pub fn set_catalog(&mut self, catalog: Option<Catalog>) {
        self.catalog = catalog;
    }

    /// The label of every button, in the current language.
    fn translate(&self, english: &'static str) -> String {
        match &self.catalog {
            Some(catalog) => catalog.tr(english).to_string(),
            None => english.to_string(),
        }
    }

    /// Shows or hides the bar.
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.enabled = enabled;
            self.hover = None;
        }
    }

    /// Whether the bar is shown.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// The side of one icon, in pixels.
    ///
    /// Tied to the line height rather than fixed, so the bar grows with the
    /// font: an icon smaller than the text beside it reads as a bullet point.
    /// The line height it is given is the chrome's, not the document's, so the
    /// bar does not grow when the text is zoomed — Notepad++'s toolbar is a row
    /// of fixed bitmaps for the same reason, and a bar that resized on every
    /// notch would rebuild the icon atlas each time.
    pub fn icon_size(&self, line_height: f32) -> f32 {
        (line_height * 1.15).round().clamp(MIN_ICON, MAX_ICON)
    }

    /// The height the bar occupies, zero when it is hidden.
    pub fn height(&self, line_height: f32) -> f32 {
        if self.enabled {
            self.icon_size(line_height) + 2.0 * BAR_PADDING
        } else {
            0.0
        }
    }

    /// The width of button `index`.
    fn button_width(&self, index: usize, line_height: f32) -> f32 {
        let _ = index;
        self.icon_size(line_height) + 2.0 * BUTTON_PADDING
    }

    /// The x where button `index` starts.
    fn button_left(&self, index: usize, line_height: f32) -> f32 {
        let mut x = 0.0;
        for (i, button) in BUTTONS.iter().enumerate().take(index) {
            if button.separator_before {
                x += SEPARATOR_WIDTH;
            }
            x += self.button_width(i, line_height);
        }
        if BUTTONS[index].separator_before {
            x += SEPARATOR_WIDTH;
        }
        x
    }

    /// Whether button `index` fits inside a bar `width` wide.
    ///
    /// Notepad++ sets `TBSTYLE_EX_HIDECLIPPEDBUTTONS` on its toolbar, so a
    /// button that would be cut in half by the right edge is dropped whole
    /// rather than drawn sliced. We do the same: a half icon reads as a
    /// rendering fault, and the command is still reachable from the menu.
    fn fits(&self, index: usize, width: f32, line_height: f32) -> bool {
        self.button_left(index, line_height) + self.button_width(index, line_height) <= width
    }

    /// The button under `(x, y)`, if any.
    ///
    /// `top` is the bar's own top edge, because the bar sits below the menu and
    /// does not own its own vertical position. `width` is the window's, so that
    /// buttons hidden by the right edge cannot be hit either.
    pub fn button_at(
        &self,
        x: f32,
        y: f32,
        top: f32,
        width: f32,
        line_height: f32,
    ) -> Option<usize> {
        if !self.enabled || y < top || y >= top + self.height(line_height) {
            return None;
        }
        for (index, _) in BUTTONS.iter().enumerate() {
            if !self.fits(index, width, line_height) {
                break;
            }
            let left = self.button_left(index, line_height);
            if x >= left && x < left + self.button_width(index, line_height) {
                return Some(index);
            }
        }

        None
    }

    /// Handles a pointer move. Returns true when the bar consumed it.
    pub fn hover_at(
        &mut self,
        x: f32,
        y: f32,
        top: f32,
        width: f32,
        line_height: f32,
    ) -> bool {
        if !self.enabled || y < top || y >= top + self.height(line_height) {
            if self.hover.is_some() {
                self.hover = None;
            }
            return false;
        }
        self.hover = self.button_at(x, y, top, width, line_height);
        true
    }

    /// Handles a click. Returns the action to run, if a button was hit.
    pub fn click(
        &mut self,
        x: f32,
        y: f32,
        top: f32,
        width: f32,
        line_height: f32,
    ) -> Option<Action> {
        let index = self.button_at(x, y, top, width, line_height)?;
        Some(BUTTONS[index].action)
    }

    /// The label of the button under the pointer, for the status bar.
    pub fn hovered_label(&self) -> Option<String> {
        self.hover.map(|index| self.translate(BUTTONS[index].label))
    }

    /// The button under `(x, y)`, with its label and the point a tooltip hangs
    /// from.
    ///
    /// The anchor is the button's left edge and the bar's bottom edge, so the
    /// label sits under the button it describes. A pair rather than a rectangle
    /// because that is all a tooltip needs, and handing it a rectangle would
    /// invite it to draw inside the bar.
    ///
    /// Separate from [`button_at`](Self::button_at) because it must not disturb
    /// the hover state: the tooltip is asked before the bar is, so that a
    /// pointer resting on a button keeps its dwell timer running while the bar
    /// repaints the highlight underneath it.
    pub fn tip_at(
        &self,
        x: f32,
        y: f32,
        top: f32,
        width: f32,
        line_height: f32,
    ) -> Option<(String, (f32, f32))> {
        let index = self.button_at(x, y, top, width, line_height)?;
        let left = self.button_left(index, line_height);
        Some((
            self.translate(BUTTONS[index].label),
            (left, top + self.height(line_height)),
        ))
    }

    /// Queues the bar's background, the separators, and the hovered button.
    pub fn push_quads(
        &self,
        quads: &mut QuadRenderer,
        width: f32,
        top: f32,
        line_height: f32,
        theme: &Theme,
    ) {
        let height = self.height(line_height);
        if height == 0.0 {
            return;
        }
        quads.push(
            0.0,
            top,
            width,
            height,
            theme.toolbar_background.to_linear(),
        );
        quads.push(
            0.0,
            top + height - 1.0,
            width,
            1.0,
            theme.toolbar_border.to_linear(),
        );

        for (index, button) in BUTTONS.iter().enumerate() {
            if !self.fits(index, width, line_height) {
                break;
            }
            let left = self.button_left(index, line_height);
            if button.separator_before {
                quads.push(
                    left - SEPARATOR_WIDTH * 0.5,
                    top + 4.0,
                    1.0,
                    height - 8.0,
                    theme.toolbar_border.to_linear(),
                );
            }
            if self.hover == Some(index) {
                quads.push(
                    left,
                    top + 1.0,
                    self.button_width(index, line_height),
                    height - 3.0,
                    theme.toolbar_hover.to_linear(),
                );
            }
        }
    }

    /// Queues every icon.
    ///
    /// Two rectangles per button, one per colour, because the body and the
    /// accent are separate cells in the atlas. A button with no accent — the
    /// clipboard, the printer — still queues an empty one; the cost is a
    /// degenerate quad that the shader discards, and skipping it would mean the
    /// toolbar had to know which icons are two-tone.
    pub fn push_icons(
        &self,
        icons: &mut IconRenderer,
        width: f32,
        top: f32,
        line_height: f32,
        theme: &Theme,
    ) {
        let height = self.height(line_height);
        if height == 0.0 {
            return;
        }
        let size = self.icon_size(line_height);
        let y = top + (height - size) * 0.5;
        for (index, button) in BUTTONS.iter().enumerate() {
            if !self.fits(index, width, line_height) {
                break;
            }
            let Some(icon) = icons::index(button.icon) else {
                continue;
            };
            let left = self.button_left(index, line_height);
            let x = left + (self.button_width(index, line_height) - size) * 0.5;
            icons.push(
                icon,
                false,
                x,
                y,
                size,
                size,
                theme.icon_ink.to_linear(),
            );
            icons.push(
                icon,
                true,
                x,
                y,
                size,
                size,
                theme.icon_accent.to_linear(),
            );
        }
    }
}

impl Default for Toolbar {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wide enough that every button fits, so the layout tests are not
    /// accidentally exercising the clipped-button rule.
    const WIDE: f32 = 4000.0;

    fn bar() -> Toolbar {
        Toolbar::new()
    }

    #[test]
    fn the_bar_is_taller_than_a_line() {
        // 20px line, 23px icon, 4px padding above and below.
        assert_eq!(bar().height(20.0), 31.0);
    }

    #[test]
    fn a_hidden_bar_takes_no_space() {
        let mut b = bar();
        b.set_enabled(false);
        assert_eq!(b.height(20.0), 0.0);
    }

    #[test]
    fn a_hidden_bar_swallows_no_clicks() {
        let mut b = bar();
        b.set_enabled(false);
        assert_eq!(b.click(4.0, 30.0, 26.0, WIDE, 20.0), None);
        assert!(!b.hover_at(4.0, 30.0, 26.0, WIDE, 20.0));
    }

    #[test]
    fn the_first_button_is_new() {
        let mut b = bar();
        assert_eq!(b.click(4.0, 30.0, 26.0, WIDE, 20.0), Some(Action::New));
    }

    #[test]
    fn buttons_are_hit_tested_left_to_right() {
        let b = bar();
        for (index, button) in BUTTONS.iter().enumerate() {
            let left = b.button_left(index, 20.0);
            let x = left + b.button_width(index, 20.0) * 0.5;
            assert_eq!(
                b.button_at(x, 30.0, 26.0, WIDE, 20.0),
                Some(index),
                "{} is not where it is drawn",
                button.label
            );
        }
    }

    #[test]
    fn a_click_above_or_below_the_bar_misses() {
        let b = bar();
        assert_eq!(b.button_at(4.0, 25.0, 26.0, WIDE, 20.0), None);
        assert_eq!(b.button_at(4.0, 60.0, 26.0, WIDE, 20.0), None);
    }

    #[test]
    fn a_click_past_the_last_button_misses() {
        let b = bar();
        assert_eq!(b.button_at(1e6, 30.0, 26.0, WIDE, 20.0), None);
    }

    #[test]
    fn hovering_a_button_highlights_it() {
        let mut b = bar();
        assert!(b.hover_at(4.0, 30.0, 26.0, WIDE, 20.0));
        assert_eq!(b.hover, Some(0));
        assert_eq!(b.hovered_label().as_deref(), Some("New"));
    }

    #[test]
    fn moving_off_the_bar_clears_the_highlight() {
        let mut b = bar();
        b.hover_at(4.0, 30.0, 26.0, WIDE, 20.0);
        assert!(!b.hover_at(4.0, 500.0, 26.0, WIDE, 20.0));
        assert_eq!(b.hover, None);
        assert_eq!(b.hovered_label(), None);
    }

    #[test]
    fn every_button_carries_a_label_and_a_command() {
        for button in BUTTONS {
            assert!(!button.label.is_empty());
        }
    }

    #[test]
    fn every_button_names_an_icon_that_exists() {
        for button in BUTTONS {
            assert!(
                icons::index(button.icon).is_some(),
                "{} names a missing icon {}",
                button.label,
                button.icon
            );
        }
    }

    #[test]
    fn no_button_is_listed_twice() {
        let mut seen: Vec<Action> = Vec::new();
        for button in BUTTONS {
            assert!(!seen.contains(&button.action), "{} twice", button.label);
            seen.push(button.action);
        }
    }

    #[test]
    fn the_first_button_has_no_separator_before_it() {
        assert!(!BUTTONS[0].separator_before);
    }

    #[test]
    fn a_separator_pushes_the_next_button_right() {
        let b = bar();
        let index = BUTTONS
            .iter()
            .position(|button| button.separator_before)
            .expect("a group separator");
        let previous_end = b.button_left(index - 1, 20.0) + b.button_width(index - 1, 20.0);
        assert_eq!(b.button_left(index, 20.0), previous_end + SEPARATOR_WIDTH);
    }

    #[test]
    fn the_bar_is_notepad_plus_plus_s_in_order_and_grouping() {
        let shape: Vec<(&str, bool)> = BUTTONS
            .iter()
            .map(|b| (b.label, b.separator_before))
            .collect();
        assert_eq!(
            shape,
            vec![
                ("New", false),
                ("Open", false),
                ("Save", false),
                ("Save All", false),
                ("Close", false),
                ("Close All", false),
                ("Print", false),
                ("Cut", true),
                ("Copy", false),
                ("Paste", false),
                ("Undo", true),
                ("Redo", false),
                ("Find", true),
                ("Replace", false),
                ("Zoom In", true),
                ("Zoom Out", false),
                ("Sync Vertical Scroll", true),
                ("Sync Horizontal Scroll", false),
                ("Word Wrap", true),
                ("Show All Characters", false),
                ("Show Indent Guide", false),
                ("User Defined Language", true),
                ("Document Map", false),
                ("Document List", false),
                ("Function List", false),
                ("Folder as Workspace", false),
                ("Monitoring (tail -f)", true),
                ("Start Recording", true),
                ("Stop Recording", false),
                ("Playback", false),
                ("Run Multiple Macros", false),
                ("Save Current Macro", false),
            ]
        );
    }

    #[test]
    fn the_icon_grows_with_the_font_but_stays_within_bounds() {
        let b = bar();
        assert_eq!(b.icon_size(8.0), MIN_ICON);
        assert_eq!(b.icon_size(20.0), 23.0);
        assert_eq!(b.icon_size(80.0), MAX_ICON);
    }

    #[test]
    fn every_button_is_wide_enough_for_its_icon() {
        let b = bar();
        for index in 0..BUTTONS.len() {
            assert!(b.button_width(index, 20.0) > b.icon_size(20.0));
        }
    }

    #[test]
    fn a_button_cut_by_the_right_edge_is_hidden_whole() {
        // Notepad++ sets TBSTYLE_EX_HIDECLIPPEDBUTTONS, so a button that would
        // be sliced by the window edge is dropped rather than drawn in part.
        let b = bar();
        let last = BUTTONS.len() - 1;
        let end = b.button_left(last, 20.0) + b.button_width(last, 20.0);
        assert!(b.fits(last, end, 20.0));
        assert!(!b.fits(last, end - 1.0, 20.0));
    }

    #[test]
    fn a_hidden_button_cannot_be_clicked() {
        let mut b = bar();
        let last = BUTTONS.len() - 1;
        let left = b.button_left(last, 20.0);
        let x = left + b.button_width(last, 20.0) * 0.5;
        assert_eq!(b.click(x, 30.0, 26.0, WIDE, 20.0), Some(BUTTONS[last].action));
        assert_eq!(b.click(x, 30.0, 26.0, left, 20.0), None);
    }

    #[test]
    fn a_tooltip_hangs_under_the_button_it_describes() {
        let b = bar();
        let left = b.button_left(2, 20.0);
        let x = left + b.button_width(2, 20.0) * 0.5;
        let (label, anchor) = b.tip_at(x, 30.0, 26.0, WIDE, 20.0).expect("a button");
        assert_eq!(label, BUTTONS[2].label);
        assert_eq!(anchor, (left, 26.0 + b.height(20.0)));
    }

    #[test]
    fn a_hidden_button_has_no_tooltip() {
        // The same rule as the click: a button the right edge cut away is not
        // there, so it cannot be described either.
        let b = bar();
        let last = BUTTONS.len() - 1;
        let left = b.button_left(last, 20.0);
        let x = left + b.button_width(last, 20.0) * 0.5;
        assert!(b.tip_at(x, 30.0, 26.0, WIDE, 20.0).is_some());
        assert!(b.tip_at(x, 30.0, 26.0, left, 20.0).is_none());
    }

    #[test]
    fn a_hidden_bar_has_no_tooltip() {
        let mut b = bar();
        b.set_enabled(false);
        assert!(b.tip_at(10.0, 30.0, 26.0, WIDE, 20.0).is_none());
    }

    #[test]
    fn the_bar_fits_a_typical_window() {
        // 1100px is the width the app is screenshotted at; the whole bar has to
        // be reachable there or the last group is dead.
        let b = bar();
        let last = BUTTONS.len() - 1;
        let end = b.button_left(last, 20.0) + b.button_width(last, 20.0);
        assert!(
            end <= 1100.0,
            "the bar needs {end}px, more than a 1100px window"
        );
    }
}

