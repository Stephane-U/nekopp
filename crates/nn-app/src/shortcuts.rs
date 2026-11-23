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

//! The shortcut editor.
//!
//! A scrollable list of every command the keymap can bind, one chord each. The
//! dialog edits a *copy* of the keymap and hands it back, the same contract the
//! preferences dialog uses for the settings: the widget never touches the disk.
//!
//! Capture is a mode rather than a field. Pressing Enter on a row puts the row
//! into capture and the next chord that arrives is bound to it, whatever it is.
//! That is the only way to bind a chord the dialog itself would otherwise
//! swallow — `Ctrl+N` has to be capturable while the dialog is open, and it is
//! also the chord that would open a new tab.
//!
//! Like the function list and the preferences dialog, the rows are one text
//! area rather than one per row.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics};
use glyphon::{TextArea, TextBounds};

use crate::keymap::{Chord, Keymap};
use crate::menu::Action;
use crate::quad::QuadRenderer;
use crate::text::shaping_for;
use crate::theme::Theme;

/// The width of the dialog, in pixels.
///
/// Wide enough for the longest command label and the longest chord
/// (`Ctrl+Alt+Shift+F`) with a gap between them.
const WIDTH: f32 = 520.0;
/// Padding inside the frame.
const PADDING: f32 = 14.0;
/// The column the chords start at, in characters.
const VALUE_COLUMN: usize = 26;
/// How many rows fit before the list scrolls.
const MAX_VISIBLE: usize = 16;

/// The modal shortcut list.
pub struct Shortcuts {
    open: bool,
    /// Index into [`Action::BINDABLE`].
    selected: usize,
    /// The first visible index, so a long list can scroll.
    scroll: usize,
    /// Whether the next chord is bound to the selected row.
    capturing: bool,
    /// The keymap being edited. The caller reads this back after a change.
    keymap: Keymap,
    title: Buffer,
    title_shaped: bool,
    body: Buffer,
    body_shaped: String,
    hint: Buffer,
    hint_shaped: String,
    family: Family<'static>,
    char_width: f32,
    line_height: f32,
    /// Window size, so the dialog can centre itself.
    viewport: (f32, f32),
}

impl Shortcuts {
    /// Builds a closed dialog.
    pub fn new(metrics: Metrics, family: Family<'static>, char_width: f32) -> Self {
        Self {
            open: false,
            selected: 0,
            scroll: 0,
            capturing: false,
            keymap: Keymap::default(),
            title: Buffer::new_empty(metrics),
            title_shaped: false,
            body: Buffer::new_empty(metrics),
            body_shaped: String::new(),
            hint: Buffer::new_empty(metrics),
            hint_shaped: String::new(),
            family,
            char_width,
            line_height: metrics.line_height,
            viewport: (0.0, 0.0),
        }
    }

    /// Opens the dialog on a copy of `keymap`.
    pub fn open(&mut self, keymap: &Keymap) {
        self.keymap = keymap.clone();
        self.selected = 0;
        self.scroll = 0;
        self.capturing = false;
        self.open = true;
    }

    /// Closes the dialog, discarding nothing: the caller already has the edits.
    pub fn close(&mut self) {
        self.open = false;
        self.capturing = false;
    }

    /// Whether the dialog is showing.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The keymap as edited so far.
    pub fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    /// Whether the next chord will be bound to the selected row.
    pub fn is_capturing(&self) -> bool {
        self.capturing
    }

    /// The highlighted command.
    pub fn selected(&self) -> Action {
        Action::BINDABLE[self.selected.min(Action::BINDABLE.len() - 1)]
    }

    /// The highlighted index.
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// Records the window size, so the dialog can centre itself.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.viewport = (width, height);
    }

    /// How many rows are on screen at once.
    pub fn visible_rows(&self) -> usize {
        MAX_VISIBLE.min(Action::BINDABLE.len())
    }

    /// The first visible index.
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// Moves the highlight by `delta`, clamping at both ends and scrolling to
    /// keep it on screen.
    ///
    /// Clamping rather than wrapping, for the same reason the preferences list
    /// clamps: a list with a first and a last row should stop at them.
    pub fn move_selection(&mut self, delta: isize) {
        let last = Action::BINDABLE.len() as isize - 1;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
        self.scroll_to_selection();
    }

    /// Highlights `index`, for a click.
    pub fn set_selected(&mut self, index: usize) {
        if index < Action::BINDABLE.len() {
            self.selected = index;
            self.scroll_to_selection();
        }
    }

    /// Scrolls the minimum amount that puts the selection back on screen.
    fn scroll_to_selection(&mut self) {
        let visible = self.visible_rows();
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + visible {
            self.scroll = self.selected + 1 - visible;
        }
    }

    /// Starts waiting for a chord.
    pub fn begin_capture(&mut self) {
        self.capturing = true;
    }

    /// Stops waiting for a chord without binding anything.
    pub fn cancel_capture(&mut self) {
        self.capturing = false;
    }

    /// Binds `chord` to the selected command.
    ///
    /// Returns whether the keymap changed. A chord already held by another
    /// command moves to this one: a chord means one thing, and silently
    /// refusing the rebind would leave the user with no way to take a chord
    /// back from a command they no longer use.
    pub fn capture(&mut self, chord: Chord) -> bool {
        self.capturing = false;
        let action = self.selected();
        if self.keymap.lookup(&chord) == Some(action) {
            return false;
        }
        self.keymap.set(action, chord);
        true
    }

    /// Removes the selected command's chord.
    ///
    /// Returns whether the keymap changed, so a command that was already
    /// unbound does not trigger a write.
    pub fn unbind_selected(&mut self) -> bool {
        let action = self.selected();
        if self.keymap.display(action).is_empty() {
            return false;
        }
        self.keymap.unbind(action);
        true
    }

    /// The chord drawn on the right of `action`, or a dash when it has none.
    pub fn value(&self, action: Action) -> String {
        let chord = self.keymap.display(action);
        if chord.is_empty() {
            "-".to_string()
        } else {
            chord
        }
    }

    /// The width the dialog occupies.
    pub fn width(&self) -> f32 {
        WIDTH
    }

    /// The height the dialog occupies.
    ///
    /// Title, a gap, the column header, the rows, a gap, and the hint, plus the
    /// padding above and below.
    pub fn height(&self) -> f32 {
        PADDING * 2.0 + (self.visible_rows() as f32 + 5.0) * self.line_height
    }

    /// The top-left corner, centred in the window.
    fn origin(&self) -> (f32, f32) {
        let (width, height) = self.viewport;
        let left = ((width - self.width()) / 2.0).max(0.0);
        let top = ((height - self.height()) / 2.0).max(0.0);
        (left, top)
    }

    /// The y pixel where the first row starts.
    fn rows_top(&self) -> f32 {
        self.origin().1 + PADDING + self.line_height * 3.0
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
        let rows_top = self.rows_top();
        if y < rows_top || y > top + self.height() {
            return None;
        }
        let row = ((y - rows_top) / self.line_height) as usize;
        if row >= self.visible_rows() {
            return None;
        }
        let index = self.scroll + row;
        (index < Action::BINDABLE.len()).then_some(index)
    }

    /// Shapes the title, the rows, and the hint if they changed.
    pub fn prepare(&mut self, font_system: &mut FontSystem) {
        if !self.open {
            return;
        }
        // The title is a constant, so it is shaped once and never again.
        if !self.title_shaped {
            self.title.set_text(
                self.title_text(),
                &Attrs::new().family(self.family),
                shaping_for(self.title_text()),
                None,
            );
            self.title.shape_until_scroll(font_system, false);
            self.title_shaped = true;
        }

        let mut text = String::new();
        text.push_str(&pad("Command", VALUE_COLUMN));
        text.push_str("Shortcut");
        for index in self.scroll..self.scroll + self.visible_rows() {
            let Some(action) = Action::BINDABLE.get(index) else {
                break;
            };
            text.push('\n');
            text.push_str(&pad(&action.label(), VALUE_COLUMN));
            text.push_str(&self.value(*action));
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

        if self.hint_shaped != self.hint() {
            self.hint.set_text(
                self.hint(),
                &Attrs::new().family(self.family),
                shaping_for(self.hint()),
                None,
            );
            self.hint.shape_until_scroll(font_system, false);
            self.hint_shaped = self.hint().to_string();
        }
    }

    /// Queues the frame, the selected row, and the capture marker.
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

        let row = (self.selected - self.scroll) as f32;
        let row_top = self.rows_top() + row * self.line_height;
        quads.push(
            left + 1.0,
            row_top,
            width - 2.0,
            self.line_height,
            theme.menu_hover.to_linear(),
        );
        // A capture in progress gets a caret-coloured edge, so the row that is
        // waiting for a key is not just the row that happens to be highlighted.
        if self.capturing {
            quads.push(left + 1.0, row_top, 3.0, self.line_height, theme.caret.to_linear());
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
            TextArea {
                buffer: &self.hint,
                left: left + PADDING,
                top: top + PADDING + self.line_height * (self.visible_rows() as f32 + 3.0),
                scale: 1.0,
                bounds,
                default_color: theme.bar_label.to_cosmic(),
                custom_glyphs: &[],
            },
        ]
    }

    /// The title, drawn separately so it can use the label colour.
    pub fn title_text(&self) -> &'static str {
        "Shortcuts"
    }

    /// The hint line under the rows.
    pub fn hint(&self) -> &'static str {
        if self.capturing {
            "Press a key to bind it   Esc cancel"
        } else {
            "Up/Down select   Enter rebind   Del unbind   Esc close"
        }
    }

    /// The character width the dialog was built with.
    pub fn char_width(&self) -> f32 {
        self.char_width
    }
}

/// Pads `text` with spaces to `column` characters.
///
/// The font is monospace, so padding to a fixed column lines the values up
/// without a second layout.
fn pad(text: &str, column: usize) -> String {
    let mut out = String::with_capacity(column);
    out.push_str(text);
    for _ in text.chars().count()..column {
        out.push(' ');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog() -> Shortcuts {
        Shortcuts::new(Metrics::new(15.0, 20.0), Family::Monospace, 8.0)
    }

    fn chord(text: &str) -> Chord {
        Chord::parse(text).expect("chord parses")
    }

    #[test]
    fn a_new_dialog_is_closed() {
        assert!(!dialog().is_open());
    }

    #[test]
    fn opening_copies_the_keymap() {
        let mut shortcuts = dialog();
        let keymap = Keymap::default();
        shortcuts.open(&keymap);
        assert!(shortcuts.is_open());
        assert_eq!(shortcuts.keymap(), &keymap);
    }

    #[test]
    fn the_first_row_is_the_first_bindable_command() {
        assert_eq!(dialog().selected(), Action::BINDABLE[0]);
    }

    #[test]
    fn selection_clamps_at_both_ends() {
        let mut shortcuts = dialog();
        shortcuts.open(&Keymap::default());
        shortcuts.move_selection(-1);
        assert_eq!(shortcuts.selected_index(), 0);
        shortcuts.move_selection(1000);
        assert_eq!(shortcuts.selected_index(), Action::BINDABLE.len() - 1);
    }

    #[test]
    fn the_selection_scrolls_into_view() {
        let mut shortcuts = dialog();
        shortcuts.open(&Keymap::default());
        let visible = shortcuts.visible_rows();
        shortcuts.move_selection(visible as isize + 2);
        assert_eq!(shortcuts.scroll(), 3);
        shortcuts.move_selection(-1000);
        assert_eq!(shortcuts.scroll(), 0);
    }

    #[test]
    fn a_click_selects_a_row() {
        let mut shortcuts = dialog();
        shortcuts.open(&Keymap::default());
        shortcuts.set_viewport(1000.0, 800.0);
        let (left, _) = shortcuts.origin();
        let rows_top = shortcuts.rows_top();
        assert_eq!(shortcuts.row_at(left + 20.0, rows_top + 5.0), Some(0));
        assert_eq!(
            shortcuts.row_at(left + 20.0, rows_top + shortcuts.line_height + 5.0),
            Some(1)
        );
        assert_eq!(shortcuts.row_at(left - 5.0, rows_top + 5.0), None);
    }

    #[test]
    fn a_click_past_the_last_visible_row_is_none() {
        let mut shortcuts = dialog();
        shortcuts.open(&Keymap::default());
        shortcuts.set_viewport(1000.0, 800.0);
        let (left, _) = shortcuts.origin();
        let y = shortcuts.rows_top() + shortcuts.visible_rows() as f32 * shortcuts.line_height + 5.0;
        assert_eq!(shortcuts.row_at(left + 20.0, y), None);
    }

    #[test]
    fn a_closed_dialog_has_no_rows() {
        assert_eq!(dialog().row_at(10.0, 10.0), None);
    }

    #[test]
    fn capturing_binds_the_next_chord() {
        let mut shortcuts = dialog();
        shortcuts.open(&Keymap::default());
        shortcuts.set_selected(0);
        shortcuts.begin_capture();
        assert!(shortcuts.is_capturing());
        assert!(shortcuts.capture(chord("F7")));
        assert!(!shortcuts.is_capturing());
        assert_eq!(shortcuts.keymap().lookup(&chord("F7")), Some(Action::BINDABLE[0]));
    }

    #[test]
    fn capturing_the_chord_a_command_already_holds_changes_nothing() {
        let mut shortcuts = dialog();
        shortcuts.open(&Keymap::default());
        shortcuts.set_selected(0);
        let held = shortcuts.keymap().display(Action::BINDABLE[0]);
        assert!(!shortcuts.capture(chord(&held)));
    }

    #[test]
    fn capturing_a_chord_takes_it_from_its_old_command() {
        let mut shortcuts = dialog();
        shortcuts.open(&Keymap::default());
        shortcuts.set_selected(0);
        assert!(shortcuts.capture(chord("Ctrl+D")));
        assert_eq!(shortcuts.keymap().lookup(&chord("Ctrl+D")), Some(Action::BINDABLE[0]));
        assert_eq!(shortcuts.keymap().display(Action::DuplicateLine), "");
    }

    #[test]
    fn cancelling_capture_binds_nothing() {
        let mut shortcuts = dialog();
        shortcuts.open(&Keymap::default());
        let before = shortcuts.keymap().clone();
        shortcuts.begin_capture();
        shortcuts.cancel_capture();
        assert!(!shortcuts.is_capturing());
        assert_eq!(shortcuts.keymap(), &before);
    }

    #[test]
    fn unbinding_removes_the_chord() {
        let mut shortcuts = dialog();
        shortcuts.open(&Keymap::default());
        shortcuts.set_selected(0);
        assert!(shortcuts.unbind_selected());
        assert_eq!(shortcuts.keymap().display(Action::BINDABLE[0]), "");
        assert_eq!(shortcuts.value(Action::BINDABLE[0]), "-");
    }

    #[test]
    fn unbinding_an_unbound_command_reports_no_change() {
        let mut shortcuts = dialog();
        shortcuts.open(&Keymap::default());
        shortcuts.set_selected(0);
        assert!(shortcuts.unbind_selected());
        assert!(!shortcuts.unbind_selected());
    }

    #[test]
    fn the_dialog_is_centred_in_the_window() {
        let mut shortcuts = dialog();
        shortcuts.set_viewport(1000.0, 800.0);
        let (left, top) = shortcuts.origin();
        assert_eq!(left, (1000.0 - WIDTH) / 2.0);
        assert_eq!(top, (800.0 - shortcuts.height()) / 2.0);
    }

    #[test]
    fn a_window_smaller_than_the_dialog_pins_it_to_the_corner() {
        let mut shortcuts = dialog();
        shortcuts.set_viewport(100.0, 100.0);
        assert_eq!(shortcuts.origin(), (0.0, 0.0));
    }

    #[test]
    fn the_hint_changes_while_capturing() {
        let mut shortcuts = dialog();
        shortcuts.open(&Keymap::default());
        assert!(shortcuts.hint().contains("rebind"));
        shortcuts.begin_capture();
        assert!(shortcuts.hint().contains("Press a key"));
    }

    #[test]
    fn padding_fills_to_the_column() {
        assert_eq!(pad("abc", 6), "abc   ");
        assert_eq!(pad("abcdef", 3), "abcdef");
    }

    #[test]
    fn every_bindable_command_has_a_label() {
        for action in Action::BINDABLE {
            assert!(!action.label().is_empty());
            assert!(!action.label().contains('_'));
        }
    }
}
