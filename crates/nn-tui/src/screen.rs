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

//! The screen: a grid of cells, and the bytes that turn the old one into the new.
//!
//! A terminal is not a framebuffer. It is a stream of instructions, and what it
//! costs is the number of bytes in that stream, so this module is built around
//! one idea: keep the frame that is on screen, compare it to the frame that
//! should be on screen, and write only what differs. A keystroke then costs one
//! cursor move and a few characters rather than a screenful, which is the
//! difference between a usable editor and a slideshow over a slow link.
//!
//! Two rules make the difference small. A row is written from column zero to the
//! last column that has anything in it, in *either* frame, so erasing the tail
//! of a line costs spaces rather than a blank repaint; and adjacent cells of the
//! same [`Tone`] share one escape, because a line of code has six or seven runs
//! in it, not eighty.

use unicode_width::UnicodeWidthChar;

use crate::palette::Tone;

/// One cell: a character and the tone it is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub tone: Tone,
}

impl Cell {
    /// A space in the document's own colours, which is what an empty cell is.
    pub const BLANK: Cell = Cell {
        ch: ' ',
        tone: Tone::Text,
    };

    /// The second half of a wide character.
    ///
    /// Wide characters are one glyph in two cells, and a terminal cannot be told
    /// to paint only one of them: the pair is written as the character and the
    /// cell beside it is left alone, which is what this marker says.
    pub const CONT: Cell = Cell {
        ch: '\0',
        tone: Tone::Text,
    };

}

/// A rectangle of cells, the thing that is drawn into and the thing that is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    width: usize,
    height: usize,
    cells: Vec<Cell>,
}

impl Grid {
    /// An empty grid of `width` by `height`.
    pub fn new(width: usize, height: usize) -> Self {
        Grid {
            width,
            height,
            cells: vec![Cell::BLANK; width * height],
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// The cell at `row` and `column`, or a blank outside the grid.
    pub fn get(&self, row: usize, column: usize) -> Cell {
        if row >= self.height || column >= self.width {
            return Cell::BLANK;
        }
        self.cells[row * self.width + column]
    }

    /// Puts `ch` at `row` and `column`, and says how many cells it took.
    ///
    /// Nothing outside the grid is written and a character that does not fit in
    /// the last column is dropped, because a wide glyph in the last column makes
    /// a terminal wrap and scroll the whole screen up by one.
    pub fn put(&mut self, row: usize, column: usize, ch: char, tone: Tone) -> usize {
        if row >= self.height || column >= self.width {
            return 0;
        }
        let wide = ch.width().unwrap_or(1);
        match wide {
            0 => 0,
            2 => {
                if column + 1 >= self.width {
                    return 0;
                }
                let index = row * self.width + column;
                self.cells[index] = Cell { ch, tone };
                self.cells[index + 1] = Cell::CONT;
                2
            }
            _ => {
                self.cells[row * self.width + column] = Cell { ch, tone };
                1
            }
        }
    }

    /// Writes `text` from `column`, and says where it stopped.
    ///
    /// Characters that are wider than the space left are dropped rather than
    /// split, so a line is cut short by the edge of the window instead of
    /// wrapping the row into the next one.
    pub fn text(&mut self, row: usize, column: usize, text: &str, tone: Tone) -> usize {
        let mut at = column;
        for ch in text.chars() {
            let used = self.put(row, at, ch, tone);
            if used == 0 {
                break;
            }
            at += used;
        }
        at
    }
}

/// Two grids and the bytes between them.
///
/// The old frame is kept so the next one can be compared against it, and the
/// buffer of bytes is kept so a frame is one allocation for the life of the
/// program rather than one per keystroke.
pub struct Frame {
    front: Grid,
    back: Grid,
    out: Vec<u8>,
    /// Whether the whole screen has to be written on the next frame.
    ///
    /// A blank cell and a cell painted in the document's own colours are the
    /// same cell, so a diff against an empty grid writes nothing where the
    /// picture is empty -- and the terminal keeps showing the background it had
    /// before the program started, which does not match the background the
    /// windowed editor uses. The first frame, and the first after a resize, are
    /// therefore written edge to edge: after that the picture is the program's
    /// own and a diff is again the right question.
    fresh: bool,
}

impl Frame {
    /// A frame for a window of `width` by `height`.
    pub fn new(width: usize, height: usize) -> Self {
        Frame {
            front: Grid::new(width, height),
            back: Grid::new(width, height),
            out: Vec::new(),
            fresh: true,
        }
    }

    /// The grid to draw the next frame into, empty.
    pub fn grid(&mut self) -> &mut Grid {
        self.back.cells.fill(Cell::BLANK);
        &mut self.back
    }

    /// Fits the frame to a window that changed size.
    ///
    /// Both grids are replaced rather than stretched: the old frame is gone from
    /// the screen anyway, since the terminal redrew everything when it resized,
    /// so the honest thing to compare against is an empty one.
    pub fn resize(&mut self, width: usize, height: usize) {
        self.front = Grid::new(width, height);
        self.back = Grid::new(width, height);
        self.fresh = true;
    }

    pub fn width(&self) -> usize {
        self.front.width()
    }

    pub fn height(&self) -> usize {
        self.front.height()
    }

    /// The bytes that take the screen from the old frame to the new one, and
    /// the new frame becomes the old one.
    pub fn render(&mut self) -> &[u8] {
        self.out.clear();
        if self.front.width != self.back.width || self.front.height != self.back.height {
            // A resized frame is drawn in full: everything on the screen belongs
            // to somebody else now.
            self.front = Grid::new(self.back.width, self.back.height);
            self.out.extend_from_slice(b"\x1b[2J");
        }
        let fresh = std::mem::replace(&mut self.fresh, false);
        for row in 0..self.back.height {
            // A fresh frame writes every column of every row; after that, only
            // the columns that differ.
            let span = if fresh {
                self.back.width.checked_sub(1).map(|last| (0, last))
            } else {
                match (self.first_difference(row), self.last_difference(row)) {
                    (Some(from), Some(last)) => Some((from, last)),
                    _ => None,
                }
            };
            let Some((from, last)) = span else {
                continue;
            };
            // Cursor to the first column that differs, not to the start of the
            // row: a line number that went from 9 to 10 and a careted line that
            // moved one column are the two cases where a cell is cheaper than a
            // row. Rows and columns in an escape are one-based, which is the one
            // place this module translates.
            self.out
                .extend_from_slice(format!("\x1b[{};{}H", row + 1, from + 1).as_bytes());
            let mut tone: Option<Tone> = None;
            for column in from..=last {
                let cell = self.back.get(row, column);
                if cell.ch == Cell::CONT.ch {
                    continue;
                }
                if tone != Some(cell.tone) {
                    self.out.extend_from_slice(cell.tone.sgr().as_bytes());
                    tone = Some(cell.tone);
                }
                let mut utf8 = [0u8; 4];
                self.out
                    .extend_from_slice(cell.ch.encode_utf8(&mut utf8).as_bytes());
            }
            if tone.is_some() {
                self.out.extend_from_slice(b"\x1b[0m");
            }
        }
        std::mem::swap(&mut self.front, &mut self.back);
        &self.out
    }

    /// The first column of `row` that differs, or `None` when the row is the same.
    fn first_difference(&self, row: usize) -> Option<usize> {
        if self.front.width != self.back.width {
            return Some(0);
        }
        (0..self.back.width)
            .find(|&column| self.front.get(row, column) != self.back.get(row, column))
    }

    /// The last column of `row` that differs.
    ///
    /// A cell that held a character and now holds a space *differs*, so this
    /// covers the end of a line that got shorter without needing to know where
    /// the line used to end: the spaces are written because they are not what
    /// was there.
    fn last_difference(&self, row: usize) -> Option<usize> {
        if self.front.width != self.back.width {
            return Some(self.back.width.saturating_sub(1));
        }
        (0..self.back.width)
            .rev()
            .find(|&column| self.front.get(row, column) != self.back.get(row, column))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: usize, height: usize, rows: &[(usize, &str)]) -> Frame {
        let mut frame = Frame::new(width, height);
        {
            let grid = frame.grid();
            for (row, text) in rows {
                grid.text(*row, 0, text, Tone::Text);
            }
        }
        frame
    }

    #[test]
    fn the_same_content_twice_writes_nothing_the_second_time() {
        let mut frame = Frame::new(20, 3);
        {
            let grid = frame.grid();
            grid.text(0, 0, "fn main() {", Tone::Text);
        }
        let first = frame.render().len();
        assert!(first > 0, "the first frame wrote nothing");
        {
            let grid = frame.grid();
            grid.text(0, 0, "fn main() {", Tone::Text);
        }
        assert!(frame.render().is_empty(), "an unchanged screen was repainted");
    }

    #[test]
    fn one_changed_cell_stays_inside_its_row() {
        let mut frame = frame(20, 4, &[(0, "aaaaaaaa"), (1, "bbbbbbbb"), (2, "cccccccc")]);
        frame.render();
        {
            let grid = frame.grid();
            grid.text(0, 0, "aaaaaaaa", Tone::Text);
            grid.text(1, 0, "bbbbbbbc", Tone::Text);
            grid.text(2, 0, "cccccccc", Tone::Text);
        }
        let bytes = frame.render();
        let text = String::from_utf8_lossy(bytes);
        // One row's worth of instructions: a cursor move, the tone, and the
        // reset. The other two rows are not in the stream at all.
        assert_eq!(text.matches("\x1b[").count(), 3, "{text:?}");
        // The cursor goes to the last cell of the row, not to its start.
        assert!(text.starts_with("\x1b[2;8H"), "{text:?}");
        assert!(!text.contains("\x1b[1;"), "{text:?}");
        assert!(!text.contains("\x1b[3;"), "{text:?}");
    }

    #[test]
    fn a_line_that_got_shorter_is_erased_to_the_end() {
        let mut frame = frame(20, 2, &[(0, "a long line of text")]);
        frame.render();
        {
            let grid = frame.grid();
            grid.text(0, 0, "a long", Tone::Text);
        }
        let bytes = frame.render();
        let text = String::from_utf8_lossy(bytes);
        // The tail is written as spaces rather than left on the screen, and the
        // part of the line that did not change is not written at all.
        assert!(!text.contains("a long"), "{text:?}");
        assert!(text.contains("          "), "{text:?}");
    }

    #[test]
    fn adjacent_cells_of_one_tone_share_an_escape() {
        let mut frame = frame(20, 1, &[(0, "abcdef")]);
        let bytes = frame.render();
        let text = String::from_utf8_lossy(bytes);
        assert_eq!(text.matches('m').count(), 2, "{text:?}");
    }

    #[test]
    fn a_wide_character_takes_two_cells_and_never_splits_at_the_edge() {
        let mut grid = Grid::new(4, 1);
        assert_eq!(grid.put(0, 0, '中', Tone::Text), 2);
        assert_eq!(grid.get(0, 1).ch, Cell::CONT.ch);
        // The last column is one cell wide, so a wide character there is dropped
        // rather than wrapped.
        assert_eq!(grid.put(0, 3, '中', Tone::Text), 0);
        assert_eq!(grid.get(0, 3).ch, ' ');
    }

    #[test]
    fn the_frame_follows_a_window_that_changed_size() {
        let mut frame = frame(20, 3, &[(0, "hello")]);
        frame.render();
        frame.resize(30, 5);
        assert_eq!((frame.width(), frame.height()), (30, 5));
        {
            let grid = frame.grid();
            assert_eq!((grid.width(), grid.height()), (30, 5));
            grid.text(4, 0, "last", Tone::Text);
        }
        let bytes = frame.render();
        let text = String::from_utf8_lossy(bytes);
        assert!(text.contains("\x1b[5;1H"), "{text:?}");
    }
}
