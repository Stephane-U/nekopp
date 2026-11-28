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

//! How the window is divided, and which cells each pane gets.
//!
//! The window answers the same question in `nn_app::layout`, with pixels and a
//! draggable divider; a terminal answers it in cells and a fixed one. The
//! arithmetic lives here rather than in the frame for the reason the window
//! gives for its own copy: a division that is wrong puts a document in a column
//! nobody can see, and that is the kind of mistake that is invisible until
//! somebody reports it. `panes` is a pure function of the split and the cell
//! area, so it is tested by reading numbers rather than by reading a screen.

/// How the window is divided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Split {
    /// One pane, filling the window.
    #[default]
    None,
    /// Two panes side by side, divided by a column.
    Vertical,
    /// Two panes stacked, divided by a row.
    Horizontal,
}

impl Split {
    /// Whether there are two panes.
    pub fn is_split(self) -> bool {
        self != Split::None
    }

    /// The other orientation, for the command that turns the split a quarter.
    pub fn flipped(self) -> Split {
        match self {
            Split::None => Split::None,
            Split::Vertical => Split::Horizontal,
            Split::Horizontal => Split::Vertical,
        }
    }
}

/// A rectangle of cells, as one pane is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// The first row.
    pub top: usize,
    /// The first column.
    pub left: usize,
    /// How many rows it is tall.
    pub rows: usize,
    /// How many columns it is wide.
    pub cols: usize,
}

impl Rect {
    /// Whether there is room to draw in it at all.
    pub fn is_empty(self) -> bool {
        self.rows == 0 || self.cols == 0
    }
}

/// The fewest columns a pane may be left in a vertical split.
///
/// The window's floor is a hundred and twenty pixels, which is a dozen
/// characters: narrower than that and a pane is a margin rather than a document.
/// A terminal divides columns into character cells, so the same rule is a
/// handful of them -- enough for a gutter, a caret and a word.
pub const MIN_PANE_COLS: usize = 8;

/// The fewest rows a pane may be left in a horizontal split.
///
/// One: a line number and a line to read it against. A terminal with four rows
/// of text still has room for two documents, and refusing the split there would
/// be refusing the one thing the rows are good for.
const MIN_PANE_ROWS: usize = 1;

/// The two panes inside `area`, or `None` for a single pane.
///
/// The first rectangle is the left or top pane. The divider between them is one
/// cell of `area`, which is the cell neither pane may use: on a terminal a
/// divider glyph is a character, where the window has six pixels of its own. A
/// window too small to hold two panes at [`MIN_PANE`] gives the whole area to the
/// first and answers `None` -- a split that showed half a pane would be worse
/// than one that declined, and the command that asked has a status line to say
/// so on.
pub fn panes(split: Split, area: Rect) -> Option<(Rect, Rect)> {
    if !split.is_split() || area.is_empty() {
        return None;
    }
    match split {
        Split::None => None,
        Split::Vertical => {
            // One column goes to the divider, and the rest is shared: the first
            // pane takes the larger half when the count is odd, so the divider
            // sits one cell right of centre and both panes are as alike as whole
            // cells allow.
            let share = area.cols.saturating_sub(1);
            let first = share / 2;
            if first < MIN_PANE_COLS || share - first < MIN_PANE_COLS {
                return None;
            }
            Some((
                Rect {
                    top: area.top,
                    left: area.left,
                    rows: area.rows,
                    cols: first,
                },
                Rect {
                    top: area.top,
                    left: area.left + first + 1,
                    rows: area.rows,
                    cols: share - first,
                },
            ))
        }
        Split::Horizontal => {
            let share = area.rows.saturating_sub(1);
            let first = share / 2;
            if first < MIN_PANE_ROWS || share - first < MIN_PANE_ROWS {
                return None;
            }
            Some((
                Rect {
                    top: area.top,
                    left: area.left,
                    rows: first,
                    cols: area.cols,
                },
                Rect {
                    top: area.top + first + 1,
                    left: area.left,
                    rows: share - first,
                    cols: area.cols,
                },
            ))
        }
    }
}

/// Where the divider is, as the cells it covers.
///
/// One column for a vertical split and one row for a horizontal one, inside
/// `area` and between the two panes [`panes`] answered.
pub fn divider(split: Split, area: Rect) -> Option<Rect> {
    let (first, _) = panes(split, area)?;
    Some(match split {
        Split::None => return None,
        Split::Vertical => Rect {
            top: area.top,
            left: first.left + first.cols,
            rows: area.rows,
            cols: 1,
        },
        Split::Horizontal => Rect {
            top: first.top + first.rows,
            left: area.left,
            rows: 1,
            cols: area.cols,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole grid, forty rows of a hundred and forty columns.
    fn area() -> Rect {
        Rect {
            top: 1,
            left: 0,
            rows: 40,
            cols: 140,
        }
    }

    #[test]
    fn one_pane_is_the_whole_area() {
        assert_eq!(panes(Split::None, area()), None);
        assert_eq!(divider(Split::None, area()), None);
    }

    #[test]
    fn a_vertical_split_shares_the_columns_but_for_the_divider() {
        let (first, second) = panes(Split::Vertical, area()).expect("room enough");
        // A hundred and forty columns, one of them the divider, half of what is
        // left in each: sixty-nine and seventy, with the divider between them.
        assert_eq!(first, Rect { top: 1, left: 0, rows: 40, cols: 69 });
        assert_eq!(second, Rect { top: 1, left: 70, rows: 40, cols: 70 });
        assert_eq!(first.cols + 1 + second.cols, 140);
        // Both panes are the full height.
        assert_eq!(first.rows, 40);
        assert_eq!(second.rows, 40);
        assert_eq!(
            divider(Split::Vertical, area()),
            Some(Rect { top: 1, left: 69, rows: 40, cols: 1 })
        );
    }

    #[test]
    fn a_horizontal_split_shares_the_rows_and_keeps_the_columns() {
        let (first, second) = panes(Split::Horizontal, area()).expect("room enough");
        // Thirty-nine rows to share: nineteen above the divider, twenty below.
        assert_eq!(first, Rect { top: 1, left: 0, rows: 19, cols: 140 });
        assert_eq!(second, Rect { top: 21, left: 0, rows: 20, cols: 140 });
        assert_eq!(first.rows + 1 + second.rows, 40);
        assert_eq!(
            divider(Split::Horizontal, area()),
            Some(Rect { top: 20, left: 0, rows: 1, cols: 140 })
        );
    }

    #[test]
    fn an_area_inside_the_dock_is_divided_where_it_stands() {
        let docked = Rect {
            top: 2,
            left: 20,
            rows: 10,
            cols: 60,
        };
        let (first, second) = panes(Split::Vertical, docked).expect("room enough");
        assert_eq!(first.left, 20);
        assert_eq!(first.top, 2);
        assert_eq!(second.left, 20 + 29 + 1);
        assert_eq!(second.cols, 30);
        // The divider is between them, not at the window's edge.
        let bar = divider(Split::Vertical, docked).expect("a split has one");
        assert_eq!(bar.left, 20 + 29);
        assert_eq!(bar.top, 2);
        assert_eq!(bar.rows, 10);
    }

    #[test]
    fn a_window_too_narrow_to_split_shows_one_pane() {
        // Sixteen columns shared by two panes is eight each, which is the floor:
        // one column fewer and the split is refused rather than drawn as two
        // slivers.
        let narrow = Rect { top: 0, left: 0, rows: 10, cols: 17 };
        assert!(panes(Split::Vertical, narrow).is_some());
        let narrower = Rect { top: 0, left: 0, rows: 10, cols: 16 };
        assert_eq!(panes(Split::Vertical, narrower), None);
        assert_eq!(divider(Split::Vertical, narrower), None);
        // A single row has no room for two of anything.
        let flat = Rect { top: 0, left: 0, rows: 2, cols: 100 };
        assert_eq!(panes(Split::Horizontal, flat), None);
        // An empty area is not a pane.
        let empty = Rect { top: 0, left: 0, rows: 0, cols: 0 };
        assert_eq!(panes(Split::Vertical, empty), None);
        assert!(panes(Split::None, empty).is_none());
    }

    #[test]
    fn turning_the_split_a_quarter_swaps_the_two_axes() {
        assert_eq!(Split::Vertical.flipped(), Split::Horizontal);
        assert_eq!(Split::Horizontal.flipped(), Split::Vertical);
        assert_eq!(Split::None.flipped(), Split::None);
        assert!(Split::Vertical.is_split());
        assert!(!Split::None.is_split());
    }
}
