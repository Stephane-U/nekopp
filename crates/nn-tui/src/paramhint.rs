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

//! The parameter hint, as the terminal draws it.
//!
//! The window floats the signature of the call under the caret as a line of
//! shaped glyphs with the argument the caret is inside lit in the accent colour;
//! this draws the same line as one row inside a border, with that argument in the
//! caret's tone. What it does not decide is anything about the text: which call
//! is being typed, what the document declares for it and which argument is lit
//! are `nn_shared::hint`'s, so `Ctrl+Shift+Down` here walks the same list it walks
//! in the window.
//!
//! The hint is not a mode either. It appears when a `(` is typed after a name the
//! document declares -- the one moment it can be useful unasked -- stays while the
//! caret is inside the call, and goes the moment it is not. A hint that appeared
//! while somebody was writing prose would be noise, and Notepad++'s own "on input"
//! setting exists because of exactly that.

use nn_shared::hint::Hint;
use nn_shared::results::truncate;

use crate::palette::Tone;
use crate::screen::Grid;

/// The narrowest the box gets, in cells.
///
/// The border and the padding already take three cells, and a box narrower than
/// its own frame reads as a mistake rather than as a hint.
const MIN_WIDTH: usize = 8;

/// One row of signature, between two of border.
const HEIGHT: usize = 3;

/// The signature of the call the caret is inside, when there is one.
pub struct ParamHint {
    hint: Hint,
}

impl ParamHint {
    /// An empty hint, closed.
    pub fn new() -> Self {
        ParamHint {
            hint: Hint::default(),
        }
    }

    /// Whether the hint is on screen.
    pub fn is_open(&self) -> bool {
        !self.hint.signature.is_empty()
    }

    /// The signature being shown, which only the tests ask for.
    #[cfg(test)]
    pub fn text(&self) -> &str {
        &self.hint.signature
    }

    /// The argument lit inside the signature, which only the tests ask for.
    #[cfg(test)]
    pub fn active(&self) -> Option<std::ops::Range<usize>> {
        self.hint.active.clone()
    }

    /// Shows `hint`, or closes when there is nothing to say.
    pub fn show(&mut self, hint: Hint) {
        self.hint = hint;
    }

    /// Hides the hint.
    pub fn close(&mut self) {
        self.hint = Hint::default();
    }

    /// Moves the highlight `delta` arguments along, wrapping at both ends.
    pub fn step(&mut self, delta: isize) {
        if !self.is_open() {
            return;
        }
        self.hint.step(delta);
    }

    /// Draws the box beside the cell the caret is drawn on.
    ///
    /// Below the caret when it fits, above it when it does not, and never over
    /// the status row -- the same three rules the completion box follows, because
    /// both are overlays hanging off the same caret.
    pub fn draw(&self, grid: &mut Grid, caret: (usize, usize)) {
        if !self.is_open() || grid.width() == 0 || grid.height() == 0 {
            return;
        }
        // The status row is not the hint's to cover.
        let limit = grid.height().saturating_sub(1);
        if limit < HEIGHT {
            return;
        }
        let width = self.width(grid.width());
        let mut top = caret.0 + 1;
        if top + HEIGHT > limit {
            top = caret.0.saturating_sub(HEIGHT);
        }
        let top = top.min(limit.saturating_sub(HEIGHT));
        let left = caret.1.min(grid.width().saturating_sub(width));

        for column in left..left + width {
            grid.put(top, column, '─', Tone::MenuBorder);
            grid.put(top + HEIGHT - 1, column, '─', Tone::MenuBorder);
        }
        for row in top..top + HEIGHT {
            grid.put(row, left, '│', Tone::MenuBorder);
            grid.put(row, left + width - 1, '│', Tone::MenuBorder);
        }
        grid.put(top, left, '┌', Tone::MenuBorder);
        grid.put(top, left + width - 1, '┐', Tone::MenuBorder);
        grid.put(top + HEIGHT - 1, left, '└', Tone::MenuBorder);
        grid.put(top + HEIGHT - 1, left + width - 1, '┘', Tone::MenuBorder);

        let at = top + 1;
        for column in left + 1..left + width - 1 {
            grid.put(at, column, ' ', Tone::Menu);
        }
        let columns = width.saturating_sub(3);
        let shown = truncate(&self.hint.signature, columns);
        let mut column = left + 2;
        // The lit argument is a byte range inside the signature, so it is only
        // drawn when the cut string still holds all of it.
        match self.hint.active.clone() {
            Some(range) if shown.get(range.clone()).is_some() => {
                column = grid.text(at, column, &shown[..range.start], Tone::Menu);
                column = grid.text(at, column, &shown[range.clone()], Tone::Caret);
                grid.text(at, column, &shown[range.end..], Tone::Menu);
            }
            _ => {
                grid.text(at, column, &shown, Tone::Menu);
            }
        }
    }

    /// The box width for a screen `columns` wide: the signature, a space of
    /// padding on each side and the two borders.
    fn width(&self, columns: usize) -> usize {
        let longest = self.hint.signature.chars().count();
        (longest + 4).clamp(MIN_WIDTH, columns.max(MIN_WIDTH))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(signature: &str, active: Option<std::ops::Range<usize>>) -> Hint {
        Hint {
            signature: signature.to_string(),
            active,
        }
    }

    /// The frame as text, one string per row.
    fn drawn(hint: &ParamHint, caret: (usize, usize), width: usize, height: usize) -> Vec<String> {
        let mut grid = Grid::new(width, height);
        hint.draw(&mut grid, caret);
        (0..height)
            .map(|row| {
                (0..width)
                    .map(|column| grid.get(row, column).ch)
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn a_closed_hint_draws_nothing() {
        let hint = ParamHint::new();
        assert!(!hint.is_open());
        let rows = drawn(&hint, (0, 0), 40, 10);
        assert!(rows.iter().all(|row| row.trim().is_empty()), "{rows:?}");
    }

    #[test]
    fn showing_nothing_leaves_the_hint_closed() {
        let mut hint = ParamHint::new();
        hint.show(Hint::default());
        assert!(!hint.is_open());
        // And closing drops what was there.
        hint.show(value("save(a: u8)", None));
        assert!(hint.is_open());
        hint.close();
        assert!(!hint.is_open());
        assert_eq!(hint.text(), "");
    }

    #[test]
    fn the_box_is_one_row_of_signature_between_two_of_border() {
        let mut hint = ParamHint::new();
        hint.show(value("save(a: u8)", None));
        let rows = drawn(&hint, (0, 0), 40, 10);
        // Below the caret's row, which is row nought.
        assert!(rows[1].starts_with('┌'), "{rows:?}");
        assert!(rows[2].contains("save(a: u8)"), "{rows:?}");
        assert!(rows[3].starts_with('└'), "{rows:?}");
        // Eleven characters, a space of padding on each side and the two borders.
        assert_eq!(rows[1].chars().position(|ch| ch == '┐'), Some(14));
    }

    #[test]
    fn the_argument_the_caret_is_in_is_the_lit_one() {
        let mut hint = ParamHint::new();
        // `save(a: u8, b: u8)`: the second argument is bytes 12 to 17.
        hint.show(value("save(a: u8, b: u8)", Some(12..17)));
        let mut grid = Grid::new(40, 10);
        hint.draw(&mut grid, (0, 0));
        // Row two: the border, a space of padding, then the signature from byte
        // nought -- so byte twelve is the fourteenth column.
        assert_eq!(grid.get(2, 2).tone, Tone::Menu);
        assert_eq!(grid.get(2, 14).tone, Tone::Caret);
        assert_eq!(grid.get(2, 14).ch, 'b');
        // One past the lit run, which ends on the `8` at byte sixteen.
        assert_eq!(grid.get(2, 19).tone, Tone::Menu);
    }

    #[test]
    fn a_highlight_past_the_cut_is_not_drawn() {
        let mut hint = ParamHint::new();
        // A range the cut signature no longer holds: the row is drawn whole in
        // the body tone rather than half-lit.
        hint.show(value(&format!("save({})", "x".repeat(60)), Some(70..80)));
        let mut grid = Grid::new(20, 10);
        hint.draw(&mut grid, (0, 0));
        for column in 2..17 {
            assert_eq!(grid.get(2, column).tone, Tone::Menu, "{column}");
        }
    }

    #[test]
    fn the_box_flips_above_the_caret_at_the_bottom_of_the_screen() {
        let mut hint = ParamHint::new();
        hint.show(value("save()", None));
        // Ten rows of screen minus the status row: three do not fit below row
        // seven, so they hang above it.
        let rows = drawn(&hint, (7, 0), 40, 10);
        assert!(rows[4].starts_with('┌'), "{rows:?}");
        assert!(rows[6].starts_with('└'), "{rows:?}");
        // And the status row is never covered.
        let rows = drawn(&hint, (9, 0), 40, 10);
        assert_eq!(rows[9].trim(), "");
    }

    #[test]
    fn the_box_stays_inside_the_screen_and_cuts_what_does_not_fit() {
        let mut hint = ParamHint::new();
        hint.show(value("save(a: u8, b: u8, c: u8)", None));
        // A caret at the right edge: the box is pulled back rather than drawn off
        // the end, and it is as wide as the signature.
        let rows = drawn(&hint, (0, 35), 40, 10);
        for row in &rows {
            assert_eq!(row.chars().count(), 40, "{row:?}");
        }
        assert!(rows[2].contains("save(a: u8, b: u8, c: u8)"), "{:?}", rows[2]);
        // A screen too narrow for the signature cuts it with a mark.
        let rows = drawn(&hint, (0, 18), 20, 10);
        assert!(rows[2].contains('…'), "{:?}", rows[2]);
        assert_eq!(rows[2].chars().count(), 20);
    }

    #[test]
    fn stepping_walks_the_arguments_and_a_closed_hint_ignores_it() {
        let mut hint = ParamHint::new();
        hint.step(1);
        assert!(!hint.is_open());
        hint.show(value("foo(a, b)", Some(4..5)));
        hint.step(1);
        let mut grid = Grid::new(40, 10);
        hint.draw(&mut grid, (0, 0));
        // The second argument is the lit one now: `b` is the seventh byte of the
        // signature, which starts two cells inside the border.
        assert_eq!(grid.get(2, 9).tone, Tone::Caret);
        assert_eq!(grid.get(2, 9).ch, 'b');
    }
}
