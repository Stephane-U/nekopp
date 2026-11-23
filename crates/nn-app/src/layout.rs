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

//! How the window is divided between panes.
//!
//! Split view is the one feature that changes the shape of everything else: two
//! documents on screen at once, each with its own scroll position, its own tab
//! strip, and its own idea of where the caret is. The arithmetic that decides
//! which rectangle belongs to which pane is the part that can be wrong in a way
//! nobody notices until a click lands in the wrong document, so it lives here as
//! a pure function with tests rather than inline in the renderer.

/// How the window is divided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Split {
    /// One pane, filling the window.
    #[default]
    None,
    /// Two panes side by side, divided by a vertical bar.
    Vertical,
    /// Two panes stacked, divided by a horizontal bar.
    Horizontal,
}

impl Split {
    /// Whether there are two panes.
    pub fn is_split(self) -> bool {
        self != Split::None
    }

    /// The other orientation, for a command that flips the split.
    pub fn flipped(self) -> Split {
        match self {
            Split::None => Split::None,
            Split::Vertical => Split::Horizontal,
            Split::Horizontal => Split::Vertical,
        }
    }

    /// The name this split is written as in the settings file.
    ///
    /// Stable, like `Action::name`: a settings file has to keep meaning the
    /// same thing across versions.
    pub fn key(self) -> &'static str {
        match self {
            Split::None => "none",
            Split::Vertical => "vertical",
            Split::Horizontal => "horizontal",
        }
    }

    /// Reads a name written by [`Split::key`].
    pub fn from_key(text: &str) -> Option<Split> {
        Some(match text.trim() {
            "none" => Split::None,
            "vertical" => Split::Vertical,
            "horizontal" => Split::Horizontal,
            _ => return None,
        })
    }

    /// The label the View menu shows.
    pub fn label(self) -> &'static str {
        match self {
            Split::None => "Single View",
            Split::Vertical => "Split Vertically",
            Split::Horizontal => "Split Horizontally",
        }
    }
}

/// A rectangle in window pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    /// Whether `(x, y)` is inside.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// The width of the bar between two panes.
///
/// Wide enough to hit with a mouse and to read as a divider rather than a
/// border. Notepad++ uses a hairline; a hairline is a divider you cannot grab.
pub const DIVIDER: f32 = 6.0;

/// The smallest a pane may become, in pixels.
///
/// A drag that could collapse a pane to nothing would leave a document open with
/// no way to see it, so the ratio is clamped rather than the rectangle.
pub const MIN_PANE: f32 = 120.0;

/// The two pane rectangles for `split` inside `(x, y, width, height)`.
///
/// The first rectangle is the left or top pane. With `Split::None` the second is
/// `None` and the first is the whole area.
///
/// `ratio` is the first pane's share of the space, clamped so neither pane falls
/// below [`MIN_PANE`]. When the window is too narrow for two panes at the
/// minimum, the split is even and the panes are allowed to be small: a window
/// that cannot hold a split should still show both documents rather than one
/// pane of zero width.
pub fn panes(
    split: Split,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    ratio: f32,
) -> (Rect, Option<Rect>) {
    let whole = Rect {
        x,
        y,
        width: width.max(1.0),
        height: height.max(1.0),
    };
    if !split.is_split() {
        return (whole, None);
    }
    let (extent, other) = match split {
        Split::Vertical => (width, height),
        Split::Horizontal => (height, width),
        Split::None => unreachable!(),
    };
    let usable = (extent - DIVIDER).max(1.0);
    let first = clamp_extent(usable, ratio);
    let second = usable - first;
    let (a, b) = match split {
        Split::Vertical => (
            Rect {
                x,
                y,
                width: first,
                height: other,
            },
            Rect {
                x: x + first + DIVIDER,
                y,
                width: second,
                height: other,
            },
        ),
        Split::Horizontal => (
            Rect {
                x,
                y,
                width: other,
                height: first,
            },
            Rect {
                x,
                y: y + first + DIVIDER,
                width: other,
                height: second,
            },
        ),
        Split::None => unreachable!(),
    };
    (a, Some(b))
}

/// Shrinks a pane's left edge by `amount`, keeping it at least a pixel wide.
///
/// The vertical tab strip lives inside the pane it belongs to, so the text has to
/// start to the right of it. Applied to the rectangle rather than to the view's
/// origin so the gutter, the caret, the selection rectangles and the hit testing
/// all move together: a view that knew about the column separately could disagree
/// with the rectangle about where the text begins.
pub fn inset_left(rect: Rect, amount: f32) -> Rect {
    let amount = amount.max(0.0).min(rect.width - 1.0).max(0.0);
    Rect {
        x: rect.x + amount,
        width: rect.width - amount,
        ..rect
    }
}

/// Orders two pane rectangles by focus: the focused pane's first.
///
/// Every caller wants "the focused pane" and "the other one", never "the left
/// one" and "the right one": a hit test that acted on the wrong side of the
/// divider would edit a document the user is not looking at. `focused_second`
/// says which rectangle the focused pane is drawn in, which is how a click in
/// the right-hand view moves the focus without moving either document.
///
/// A single view returns its rectangle unchanged whatever the flag says: a split
/// that has just collapsed leaves the flag set for a frame, and there is no
/// second rectangle to swap with.
pub fn by_focus(first: Rect, second: Option<Rect>, focused_second: bool) -> (Rect, Option<Rect>) {
    match (second, focused_second) {
        (Some(second), true) => (second, Some(first)),
        _ => (first, second),
    }
}

/// The bar between the panes, or `None` when there is one pane.
pub fn divider(split: Split, x: f32, y: f32, width: f32, height: f32, ratio: f32) -> Option<Rect> {
    if !split.is_split() {
        return None;
    }
    let (first, _) = panes(split, x, y, width, height, ratio);
    Some(match split {
        Split::Vertical => Rect {
            x: first.x + first.width,
            y,
            width: DIVIDER,
            height: height.max(1.0),
        },
        Split::Horizontal => Rect {
            x,
            y: first.y + first.height,
            width: width.max(1.0),
            height: DIVIDER,
        },
        Split::None => unreachable!(),
    })
}

/// The ratio a drag to `(x, y)` implies.
///
/// Returns the ratio unchanged when the pointer is nowhere near the axis the
/// divider moves along, so a drag that wanders off the bar does not jump.
pub fn ratio_at(
    split: Split,
    x: f32,
    y: f32,
    origin_x: f32,
    origin_y: f32,
    width: f32,
    height: f32,
) -> f32 {
    let (extent, at, origin) = match split {
        Split::Vertical => (width, x, origin_x),
        Split::Horizontal => (height, y, origin_y),
        Split::None => return 0.5,
    };
    let usable = (extent - DIVIDER).max(1.0);
    let first = (at - origin).clamp(0.0, usable);
    clamp_extent(usable, first / usable) / usable
}

/// Clamps a first-pane extent so neither pane falls below [`MIN_PANE`].
fn clamp_extent(usable: f32, ratio: f32) -> f32 {
    let ratio = ratio.clamp(0.0, 1.0);
    let wanted = usable * ratio;
    if usable < MIN_PANE * 2.0 {
        // Too small to honour the minimum: split evenly rather than show one
        // pane and a sliver.
        return usable * 0.5;
    }
    wanted.clamp(MIN_PANE, usable - MIN_PANE)
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f32 = 1000.0;
    const H: f32 = 800.0;

    #[test]
    fn every_split_round_trips_through_its_key() {
        for split in [Split::None, Split::Vertical, Split::Horizontal] {
            assert_eq!(Split::from_key(split.key()), Some(split));
        }
    }

    #[test]
    fn an_unknown_split_key_is_rejected() {
        assert_eq!(Split::from_key("diagonal"), None);
        assert_eq!(Split::from_key(""), None);
    }

    #[test]
    fn one_pane_fills_the_window() {
        let (a, b) = panes(Split::None, 0.0, 0.0, W, H, 0.5);
        assert_eq!(
            a,
            Rect {
                x: 0.0,
                y: 0.0,
                width: W,
                height: H
            }
        );
        assert!(b.is_none());
    }

    #[test]
    fn a_vertical_split_is_side_by_side() {
        let (a, b) = panes(Split::Vertical, 0.0, 0.0, W, H, 0.5);
        let b = b.unwrap();
        assert_eq!(a.height, H);
        assert_eq!(b.height, H);
        assert_eq!(a.y, 0.0);
        assert_eq!(b.y, 0.0);
        assert_eq!(a.width + DIVIDER + b.width, W);
        assert_eq!(b.x, a.x + a.width + DIVIDER);
    }

    #[test]
    fn a_horizontal_split_is_stacked() {
        let (a, b) = panes(Split::Horizontal, 0.0, 0.0, W, H, 0.5);
        let b = b.unwrap();
        assert_eq!(a.width, W);
        assert_eq!(b.width, W);
        assert_eq!(a.x, 0.0);
        assert_eq!(b.x, 0.0);
        assert_eq!(a.height + DIVIDER + b.height, H);
        assert_eq!(b.y, a.y + a.height + DIVIDER);
    }

    #[test]
    fn the_panes_do_not_overlap() {
        for split in [Split::Vertical, Split::Horizontal] {
            let (a, b) = panes(split, 0.0, 0.0, W, H, 0.5);
            let b = b.unwrap();
            // The far corner of the first pane is not inside the second.
            assert!(!b.contains(a.x + a.width - 1.0, a.y + a.height - 1.0));
            assert!(!a.contains(b.x, b.y));
        }
    }

    #[test]
    fn the_ratio_moves_the_divider() {
        let (a, _) = panes(Split::Vertical, 0.0, 0.0, W, H, 0.25);
        assert!((a.width - (W - DIVIDER) * 0.25).abs() < 0.01);
    }

    #[test]
    fn a_pane_never_falls_below_the_minimum() {
        for ratio in [0.0, 0.001, 0.5, 0.999, 1.0] {
            let (a, b) = panes(Split::Vertical, 0.0, 0.0, W, H, ratio);
            let b = b.unwrap();
            assert!(a.width >= MIN_PANE, "ratio {ratio}: {}", a.width);
            assert!(b.width >= MIN_PANE, "ratio {ratio}: {}", b.width);
        }
    }

    #[test]
    fn a_window_too_narrow_to_split_splits_evenly() {
        let (a, b) = panes(Split::Vertical, 0.0, 0.0, 200.0, H, 0.9);
        let b = b.unwrap();
        assert!((a.width - b.width).abs() < 0.01);
    }

    #[test]
    fn the_origin_is_respected() {
        let (a, b) = panes(Split::Vertical, 40.0, 60.0, W, H, 0.5);
        let b = b.unwrap();
        assert_eq!(a.x, 40.0);
        assert_eq!(a.y, 60.0);
        assert_eq!(b.y, 60.0);
        assert_eq!(b.x + b.width, 40.0 + W);
    }

    #[test]
    fn the_divider_sits_between_the_panes() {
        let (a, b) = panes(Split::Vertical, 0.0, 0.0, W, H, 0.5);
        let b = b.unwrap();
        let d = divider(Split::Vertical, 0.0, 0.0, W, H, 0.5).unwrap();
        assert_eq!(d.x, a.x + a.width);
        assert_eq!(d.width, DIVIDER);
        assert_eq!(d.x + d.width, b.x);
        assert_eq!(d.height, H);
    }

    #[test]
    fn a_horizontal_divider_spans_the_width() {
        let d = divider(Split::Horizontal, 0.0, 0.0, W, H, 0.5).unwrap();
        assert_eq!(d.x, 0.0);
        assert_eq!(d.width, W);
        assert_eq!(d.height, DIVIDER);
    }

    #[test]
    fn there_is_no_divider_without_a_split() {
        assert!(divider(Split::None, 0.0, 0.0, W, H, 0.5).is_none());
    }

    #[test]
    fn a_drag_on_the_divider_round_trips() {
        let (a, _) = panes(Split::Vertical, 0.0, 0.0, W, H, 0.5);
        let at = a.x + a.width;
        let ratio = ratio_at(Split::Vertical, at, 0.0, 0.0, 0.0, W, H);
        assert!((ratio - 0.5).abs() < 0.01, "{ratio}");
    }

    #[test]
    fn a_drag_past_the_edge_clamps() {
        let left = ratio_at(Split::Vertical, -500.0, 0.0, 0.0, 0.0, W, H);
        let right = ratio_at(Split::Vertical, 5000.0, 0.0, 0.0, 0.0, W, H);
        assert!(left < 0.2, "{left}");
        assert!(right > 0.8, "{right}");
        // And the clamped ratio still produces two legal panes.
        let (a, b) = panes(Split::Vertical, 0.0, 0.0, W, H, left);
        assert!(a.width >= MIN_PANE);
        assert!(b.unwrap().width >= MIN_PANE);
    }

    #[test]
    fn a_drag_ignores_the_other_axis() {
        let a = ratio_at(Split::Vertical, 500.0, 0.0, 0.0, 0.0, W, H);
        let b = ratio_at(Split::Vertical, 500.0, 700.0, 0.0, 0.0, W, H);
        assert_eq!(a, b);
    }

    #[test]
    fn a_horizontal_drag_measures_from_the_pane_area_top() {
        // The pane area starts below the menu and toolbar, so a drag at y = 500
        // in a window whose pane area starts at 60 is 440 pixels in, not 500.
        let top = 60.0;
        let at = top + (H - DIVIDER) * 0.25;
        let ratio = ratio_at(Split::Horizontal, 0.0, at, 0.0, top, W, H);
        assert!((ratio - 0.25).abs() < 0.01, "{ratio}");
    }

    #[test]
    fn a_click_lands_in_at_most_one_pane() {
        for split in [Split::Vertical, Split::Horizontal] {
            for ratio in [0.2, 0.5, 0.8] {
                let (a, b) = panes(split, 0.0, 0.0, W, H, ratio);
                let b = b.unwrap();
                for (x, y) in [(5.0, 5.0), (W - 5.0, H - 5.0), (W / 2.0, H / 2.0)] {
                    assert!(
                        !(a.contains(x, y) && b.contains(x, y)),
                        "{split:?} at {ratio}: ({x}, {y}) is in both"
                    );
                }
            }
        }
    }

    #[test]
    fn flipping_swaps_the_orientation() {
        assert_eq!(Split::Vertical.flipped(), Split::Horizontal);
        assert_eq!(Split::Horizontal.flipped(), Split::Vertical);
        assert_eq!(Split::None.flipped(), Split::None);
    }

    #[test]
    fn only_a_split_has_two_panes() {
        assert!(!Split::None.is_split());
        assert!(Split::Vertical.is_split());
        assert!(Split::Horizontal.is_split());
    }

    #[test]
    fn the_focused_rectangle_comes_first() {
        let a = Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
        };
        let b = Rect {
            x: 110.0,
            y: 0.0,
            width: 200.0,
            height: 50.0,
        };
        assert_eq!(by_focus(a, Some(b), false), (a, Some(b)));
        assert_eq!(by_focus(a, Some(b), true), (b, Some(a)));
    }

    #[test]
    fn a_single_view_ignores_the_focus_order() {
        // The flag outlives the split by a frame when the last tab of one view
        // closes, and a stale `true` must not make the geometry look for a
        // second pane that is no longer there.
        let a = Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
        };
        assert_eq!(by_focus(a, None, false), (a, None));
        assert_eq!(by_focus(a, None, true), (a, None));
    }

    #[test]
    fn an_inset_moves_the_left_edge_and_keeps_the_right_one() {
        let rect = Rect {
            x: 100.0,
            y: 20.0,
            width: 400.0,
            height: 300.0,
        };
        let inset = inset_left(rect, 168.0);
        assert_eq!(inset.x, 268.0);
        assert_eq!(inset.width, 232.0);
        assert_eq!(inset.x + inset.width, rect.x + rect.width);
        assert_eq!(inset.y, rect.y);
        assert_eq!(inset.height, rect.height);
    }

    #[test]
    fn an_inset_never_swallows_the_whole_pane() {
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 300.0,
        };
        let inset = inset_left(rect, 500.0);
        assert!(inset.width >= 1.0, "{}", inset.width);
    }

    #[test]
    fn a_zero_inset_changes_nothing() {
        let rect = Rect {
            x: 10.0,
            y: 20.0,
            width: 400.0,
            height: 300.0,
        };
        assert_eq!(inset_left(rect, 0.0), rect);
    }

    #[test]
    fn every_split_has_a_label() {
        for split in [Split::None, Split::Vertical, Split::Horizontal] {
            assert!(!split.label().is_empty());
        }
    }
}
