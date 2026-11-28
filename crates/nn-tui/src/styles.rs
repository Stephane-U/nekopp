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

//! The Style Configurator, in cells.
//!
//! The window's dialog is a list of sixteen colours with a swatch beside each
//! one. The list, the palette and the step between two colours are all in
//! `nn-shared`, so this file is the drawing: a box, a lit row, a sample of each
//! colour, and the keys the window's own hint line names.
//!
//! A terminal shows a colour by drawing with it, and there are two ways a colour
//! can be drawn: as glyphs, or as the cell behind them. A slot that paints text
//! gets four blocks in its colour; one that paints a background -- the page, the
//! selection, the caret, the gutter -- gets four cells of it. That is the same
//! distinction the window's swatch makes by filling a rectangle, and it is why
//! the sample column is not one shape for every row.
//!
//! Two of the sixteen are the window's alone. The terminal paints plain text
//! with the slot called `Text` rather than a default style of its own, and it does
//! not paint the caret's line at all, so `Default text` and `Current line` say so
//! instead of offering a colour that would change nothing. The rows are still
//! there, in the same order, for the reason the preferences dialog keeps its two
//! unusable rows: one list, either dialog, nothing silently missing.

use nn_shared::color::Rgb;
use nn_shared::settings::Settings;
use nn_shared::theme::{record, step, Slot, Theme};

use crate::keys::Key;
use crate::palette::Tone;
use crate::screen::Grid;

/// The title on the box, and the keys along its bottom edge.
///
/// Both are the window's own sentences, word for word: the dialog is the same
/// dialog, and a user who has read one hint line has read the other.
const TITLE: &str = "Style Configurator";
const HINT: &str = "Up/Down select   Left/Right change   Esc close";

/// How many rows the list has, and so how many slots the dialog offers.
const ROWS: usize = Slot::ALL.len();

/// The characters a slot's sample is made of.
const GLYPH: &str = "████";
/// The four cells of a slot that paints a background.
const BLOCK: &str = "    ";
/// How many cells either sample covers, counted in characters rather than bytes:
/// a block is three bytes and one cell.
const SAMPLE: usize = 4;

/// What a row shows instead of a colour the terminal cannot draw.
///
/// A reason, not a value: a hex triplet beside a row whose colour changes nothing
/// would be the one dishonest thing in the box.
fn undrawn(slot: Slot) -> Option<&'static str> {
    match slot {
        Slot::Default => Some("follows Text"),
        Slot::CurrentLine => Some("not painted here"),
        _ => None,
    }
}

/// Whether `slot` paints the cells behind the text rather than the text itself.
fn paints_behind(slot: Slot) -> bool {
    matches!(
        slot,
        Slot::Background | Slot::Selection | Slot::Caret | Slot::Gutter
    )
}

/// The tone a slot's sample is drawn in.
///
/// Eight of the sixteen are syntax colours and have a tone each; the rest are the
/// page, the gutter, the caret and the selection, which the terminal draws with
/// the tones of those names. `Default text` borrows the plain tone because that is
/// what the terminal paints plain text with, and `Current line` gives the gutter's,
/// which is the nearest thing the terminal has to a line's own colour.
fn tone_of(slot: Slot) -> Tone {
    match slot {
        Slot::Default | Slot::Foreground => Tone::Text,
        Slot::Keyword => Tone::Keyword,
        Slot::Type => Tone::Type,
        Slot::String => Tone::String,
        Slot::Comment => Tone::Comment,
        Slot::Number => Tone::Number,
        Slot::Function => Tone::Function,
        Slot::Operator => Tone::Operator,
        Slot::Preprocessor => Tone::Preprocessor,
        Slot::Gutter | Slot::LineNumber => Tone::Gutter,
        Slot::Selection => Tone::Selection,
        Slot::Caret => Tone::Caret,
        Slot::Background => Tone::Text,
        Slot::CurrentLine => Tone::Gutter,
    }
}

/// The colours the editor draws with, and which of them is lit.
pub struct Styles {
    open: bool,
    /// The lit row, as an index into [`Slot::ALL`].
    selected: usize,
}

impl Styles {
    pub fn new() -> Self {
        Styles {
            open: false,
            selected: 0,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self) {
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// The row that is lit. Only the tests ask: the shell draws the lit row from
    /// the field itself.
    #[cfg(test)]
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Which row is lit, clamped to the list. Only the tests ask.
    #[cfg(test)]
    pub fn set_selected(&mut self, index: usize) {
        self.selected = index.min(ROWS - 1);
    }

    /// Handles one key, and says whether it changed the settings.
    ///
    /// Every key that arrives while the box is up stops here, whether it did
    /// anything or not: a modal that let an arrow through would move the text
    /// behind the box, and the box would then be describing a caret the reader
    /// cannot see. The return value is narrower than that -- it is the answer the
    /// caller needs to know whether to resolve the theme again -- so a key that
    /// walked the list is `false` and one that stepped a colour is `true`.
    pub fn handle(&mut self, key: Key, settings: &mut Settings, theme: &Theme) -> bool {
        if !self.open {
            return false;
        }
        match key {
            // The window's hint says Esc, and Esc is what a modal box in a
            // terminal is dismissed with; Enter is what a reader presses next, so
            // it closes too rather than being the one key that does nothing.
            Key::Escape | Key::Enter => {
                self.close();
                false
            }
            Key::Up => {
                self.selected = self.selected.saturating_sub(1);
                false
            }
            Key::Down => {
                self.selected = (self.selected + 1).min(ROWS - 1);
                false
            }
            Key::PageUp => {
                self.selected = 0;
                false
            }
            Key::PageDown => {
                self.selected = ROWS - 1;
                false
            }
            Key::Left => self.step(settings, theme, -1),
            Key::Right => self.step(settings, theme, 1),
            _ => false,
        }
    }

    /// Steps the lit slot's colour through the palette, writing the override into
    /// `settings`.
    ///
    /// The colour a step starts from is the one on screen -- `theme` is the base
    /// palette with every override applied -- so a slot changed by hand keeps its
    /// place in the palette rather than jumping to the first entry.
    ///
    /// The row the terminal cannot draw takes the key and writes nothing, which is
    /// the rule the preferences dialog follows for the rows it cannot honour: the
    /// settings file is shared with the window, so a press here must never write a
    /// colour this front-end would then ignore.
    fn step(&mut self, settings: &mut Settings, theme: &Theme, delta: i32) -> bool {
        let slot = Slot::ALL[self.selected];
        if undrawn(slot).is_some() {
            return false;
        }
        match step(slot, theme, delta) {
            Some(color) => {
                record(&mut settings.styles, slot, color);
                true
            }
            None => false,
        }
    }

    /// The first row drawn, chosen so the lit one is always inside the box.
    fn first(&self, shown: usize) -> usize {
        if shown == 0 || self.selected < shown {
            0
        } else {
            self.selected + 1 - shown
        }
    }

    /// Draws the box over whatever is on the grid.
    pub fn draw(&self, grid: &mut Grid, theme: &Theme) {
        if !self.open {
            return;
        }
        let width = grid.width();
        let height = grid.height();
        let labels = Slot::ALL
            .iter()
            .map(|slot| slot.label().chars().count())
            .max()
            .unwrap_or(0);
        let reasons = Slot::ALL
            .iter()
            .filter_map(|slot| undrawn(*slot))
            .map(|reason| reason.chars().count())
            .max()
            .unwrap_or(0);
        // One column of padding, the label, two between the label and its sample,
        // the sample, two more, then the hex value or the reason -- and one column
        // of padding and one of border at the far end. The hint line is drawn along
        // the bottom inside the same box, so it sets a floor on the width: a box
        // narrower than its own keys would say what to press over the border.
        let content = labels + SAMPLE + reasons.max(7) + 8;
        let widest = content.max(HINT.chars().count() + 4);
        let box_width = widest.min(width.saturating_sub(2)).max(24.min(width));
        let box_height = (ROWS + 4).min(height.saturating_sub(2));
        if box_height < 5 || box_width < 12 {
            return;
        }
        let left = (width - box_width) / 2;
        let top = (height - box_height) / 2;
        draw_box(grid, left, top, box_width, box_height, TITLE);
        let hint_at = top + box_height - 1;
        let hint_column = left + box_width.saturating_sub(2 + HINT.chars().count());
        grid.text(hint_at, hint_column, HINT, Tone::MenuHover);

        // The rows that fit between the top border and the hint line, and the
        // window of the list that keeps the lit row in it.
        let shown = hint_at.saturating_sub(top + 2);
        let first = self.first(shown);
        let sample_at = left + 2 + labels + 2;
        for (offset, slot) in Slot::ALL.iter().enumerate().skip(first).take(shown) {
            let at = top + 2 + offset - first;
            let lit = offset == self.selected;
            let row_tone = if lit { Tone::MenuHover } else { Tone::Menu };
            for column in left + 1..left + box_width - 1 {
                grid.put(at, column, ' ', row_tone);
            }
            grid.text(at, left + 2, slot.label(), row_tone);
            match undrawn(*slot) {
                // No sample and no hex: the reason takes the value column.
                Some(reason) => {
                    let column = left + box_width - 2 - reason.chars().count();
                    grid.text(at, column, reason, row_tone);
                }
                None => {
                    let tone = tone_of(*slot);
                    let sample = if paints_behind(*slot) { BLOCK } else { GLYPH };
                    grid.text(at, sample_at, sample, tone);
                    let value = hex(slot.get(theme));
                    let column = left + box_width - 2 - value.chars().count();
                    grid.text(at, column, &value, row_tone);
                }
            }
        }
    }
}

impl Default for Styles {
    fn default() -> Self {
        Styles::new()
    }
}

/// A colour the way the window writes it, so the two dialogs read alike.
fn hex(color: Rgb) -> String {
    format!("#{:02X}{:02X}{:02X}", color.0, color.1, color.2)
}

/// The border of a box with `title` written over its top edge.
fn draw_box(grid: &mut Grid, left: usize, top: usize, width: usize, height: usize, title: &str) {
    for column in left..left + width {
        grid.put(top, column, '─', Tone::MenuBorder);
        grid.put(top + height - 1, column, '─', Tone::MenuBorder);
    }
    for row_at in top..top + height {
        grid.put(row_at, left, '│', Tone::MenuBorder);
        grid.put(row_at, left + width - 1, '│', Tone::MenuBorder);
    }
    grid.put(top, left, '┌', Tone::MenuBorder);
    grid.put(top, left + width - 1, '┐', Tone::MenuBorder);
    grid.put(top + height - 1, left, '└', Tone::MenuBorder);
    grid.put(top + height - 1, left + width - 1, '┘', Tone::MenuBorder);
    // One space of padding each side, which is what keeps a title from running
    // into the corner.
    grid.text(top, left + 1, &format!(" {title} "), Tone::MenuHover);
}

#[cfg(test)]
mod tests {
    use super::*;
    use nn_shared::theme::{effective_theme, PALETTE};

    /// The theme the shell hands the dialog: the base palette with the overrides
    /// already applied, re-resolved after every press exactly as the shell does it.
    fn shown(settings: &Settings) -> Theme {
        effective_theme(Theme::DARK, &settings.styles)
    }

    fn dialog() -> Styles {
        let mut styles = Styles::new();
        styles.open();
        styles
    }

    /// What the dialog puts on an 80 by 24 grid, as text.
    fn screen(styles: &Styles, theme: &Theme) -> String {
        let mut grid = Grid::new(80, 24);
        styles.draw(&mut grid, theme);
        (0..24)
            .map(|row| (0..80).map(|c| grid.get(row, c).ch).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_closed_dialog_takes_no_keys() {
        let mut styles = Styles::new();
        let mut settings = Settings::default();
        assert!(!styles.handle(Key::Down, &mut settings, &Theme::DARK));
        assert_eq!(styles.selected(), 0);
        assert!(settings.styles.is_empty());
    }

    #[test]
    fn the_arrows_walk_the_sixteen_slots() {
        let mut styles = dialog();
        let mut settings = Settings::default();
        for _ in 0..ROWS + 3 {
            styles.handle(Key::Down, &mut settings, &Theme::DARK);
        }
        assert_eq!(styles.selected(), ROWS - 1);
        for _ in 0..ROWS + 3 {
            styles.handle(Key::Up, &mut settings, &Theme::DARK);
        }
        assert_eq!(styles.selected(), 0);
        // Page keys go to the ends rather than by a screenful, which is what the
        // preferences dialog does and what a list of sixteen wants.
        styles.handle(Key::PageDown, &mut settings, &Theme::DARK);
        assert_eq!(styles.selected(), ROWS - 1);
        styles.handle(Key::PageUp, &mut settings, &Theme::DARK);
        assert_eq!(styles.selected(), 0);
    }

    #[test]
    fn stepping_a_row_writes_the_style_line_the_window_writes() {
        let mut styles = dialog();
        let mut settings = Settings::default();
        styles.set_selected(1); // Keyword
        let at = PALETTE
            .iter()
            .position(|&color| color == Slot::Keyword.get(&Theme::DARK))
            .expect("the dark keyword colour is in the palette");
        let theme = shown(&settings);
        assert!(styles.handle(Key::Right, &mut settings, &theme));
        assert_eq!(
            settings.styles,
            vec![(
                Slot::Keyword,
                PALETTE[at + 1]
            )]
        );
        // And back again, which replaces the entry rather than adding a second.
        let theme = shown(&settings);
        assert!(styles.handle(Key::Left, &mut settings, &theme));
        assert_eq!(
            settings.styles,
            vec![(
                Slot::Keyword,
                PALETTE[at]
            )]
        );
    }

    #[test]
    fn a_colour_at_the_end_of_the_palette_reports_no_change() {
        let mut styles = dialog();
        let mut settings = Settings::default();
        styles.set_selected(1);
        settings.styles = vec![(
            Slot::Keyword,
            PALETTE[PALETTE.len() - 1],
        )];
        let theme = shown(&settings);
        assert!(!styles.handle(Key::Right, &mut settings, &theme));
    }

    #[test]
    fn the_two_rows_the_terminal_cannot_draw_write_nothing() {
        let mut styles = dialog();
        let mut settings = Settings::default();
        for row in [0, 14] {
            styles.set_selected(row);
            let slot = Slot::ALL[row];
            assert!(undrawn(slot).is_some(), "{} is drawable", slot.key());
            assert!(!styles.handle(Key::Right, &mut settings, &Theme::DARK));
            assert!(!styles.handle(Key::Left, &mut settings, &Theme::DARK));
            assert!(settings.styles.is_empty(), "{} was written", slot.key());
        }
    }

    #[test]
    fn every_drawable_row_says_which_colour_it_holds() {
        let styles = dialog();
        let theme = Theme::DARK;
        let text = screen(&styles, &theme);
        for slot in Slot::ALL {
            match undrawn(slot) {
                Some(reason) => {
                    assert!(text.contains(reason), "{}: {reason}", slot.key())
                }
                None => {
                    let value = hex(slot.get(&theme));
                    assert!(text.contains(&value), "{}: {value}", slot.key());
                }
            }
            assert!(text.contains(slot.label()), "{}", slot.key());
        }
    }

    #[test]
    fn escaping_closes_the_box_and_keeps_what_was_stepped() {
        let mut styles = dialog();
        let mut settings = Settings::default();
        styles.set_selected(1);
        assert!(styles.handle(Key::Right, &mut settings, &Theme::DARK));
        let kept = settings.styles.clone();
        assert!(!styles.handle(Key::Escape, &mut settings, &Theme::DARK));
        assert!(!styles.is_open());
        assert_eq!(settings.styles, kept);
    }

    #[test]
    fn a_box_shorter_than_the_list_scrolls_to_the_lit_row() {
        let mut styles = dialog();
        styles.set_selected(ROWS - 1);
        let mut grid = Grid::new(80, 14);
        styles.draw(&mut grid, &Theme::DARK);
        let text: String = (0..14)
            .map(|row| (0..80).map(|c| grid.get(row, c).ch).collect::<String>() + "\n")
            .collect();
        let last = Slot::ALL[ROWS - 1];
        assert!(text.contains(last.label()), "{text}");
        // The hint line is still on the last row of the box, not pushed off by
        // the rows above it.
        assert!(text.contains("Esc close"), "{text}");
    }

    #[test]
    fn a_grid_too_small_for_the_box_is_left_alone() {
        let styles = dialog();
        let mut grid = Grid::new(10, 4);
        styles.draw(&mut grid, &Theme::DARK);
        for row in 0..4 {
            for column in 0..10 {
                assert_eq!(grid.get(row, column).ch, ' ', "{row},{column}");
            }
        }
    }
}
