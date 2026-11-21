//! Carets and selections.
//!
//! A selection is an anchor plus a caret. The anchor is where the selection
//! started, the caret is where it ends — so `anchor > caret` means the user
//! dragged backwards, which matters for extending a selection with shift+arrow.
//!
//! `Selections` holds one or more of them. Column editing is the reason: a
//! rectangular block is one selection per line, and typing into it has to reach
//! every one of them.

use std::ops::Range;

/// A single caret, optionally with a selected span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection {
    /// Where the selection began.
    pub anchor: usize,
    /// Where the caret sits.
    pub caret: usize,
}

impl Selection {
    /// A collapsed selection at `caret`.
    pub const fn caret(caret: usize) -> Self {
        Self {
            anchor: caret,
            caret,
        }
    }

    /// A selection from `anchor` to `caret`.
    pub const fn new(anchor: usize, caret: usize) -> Self {
        Self { anchor, caret }
    }

    /// The lower bound of the selected span.
    pub fn start(&self) -> usize {
        self.anchor.min(self.caret)
    }

    /// The upper bound of the selected span.
    pub fn end(&self) -> usize {
        self.anchor.max(self.caret)
    }

    /// The selected span.
    pub fn range(&self) -> Range<usize> {
        self.start()..self.end()
    }

    /// Whether nothing is selected.
    pub fn is_empty(&self) -> bool {
        self.anchor == self.caret
    }

    /// The number of characters selected.
    pub fn len(&self) -> usize {
        self.end() - self.start()
    }

    /// Collapses the selection to `pos`.
    pub fn collapse_to(&mut self, pos: usize) {
        self.anchor = pos;
        self.caret = pos;
    }

    /// Moves the caret to `pos`, keeping the anchor so the span grows or shrinks.
    pub fn extend_to(&mut self, pos: usize) {
        self.caret = pos;
    }
}

/// An ordered, non-overlapping set of selections with one designated as primary.
#[derive(Debug, Clone, Default)]
pub struct Selections {
    items: Vec<Selection>,
    main: usize,
}

impl Selections {
    /// A single collapsed caret.
    pub fn single(caret: usize) -> Self {
        Self {
            items: vec![Selection::caret(caret)],
            main: 0,
        }
    }

    /// A single selection spanning `anchor..caret`.
    pub fn from_range(anchor: usize, caret: usize) -> Self {
        Self {
            items: vec![Selection::new(anchor, caret)],
            main: 0,
        }
    }

    /// The selection the UI draws a caret for.
    pub fn primary(&self) -> Selection {
        self.items[self.main]
    }

    /// Which of the selections is the primary one.
    ///
    /// Needed by anything that has to remove the primary from the set, which
    /// cannot be done through `primary_mut` because the index is what matters.
    pub fn primary_index(&self) -> usize {
        self.main
    }

    /// Mutable access to the primary selection.
    pub fn primary_mut(&mut self) -> &mut Selection {
        &mut self.items[self.main]
    }

    /// Replaces the whole set with one collapsed caret.
    pub fn set_caret(&mut self, caret: usize) {
        self.items.clear();
        self.items.push(Selection::caret(caret));
        self.main = 0;
    }

    /// Replaces the whole set with one selection.
    pub fn set_single(&mut self, sel: Selection) {
        self.items.clear();
        self.items.push(sel);
        self.main = 0;
    }

    /// Replaces the whole set, naming which one is primary.
    pub fn set_selections(&mut self, sels: Vec<Selection>, main: usize) {
        if sels.is_empty() {
            self.set_caret(0);
            return;
        }
        self.main = main.min(sels.len() - 1);
        self.items = sels;
    }

    /// Replaces the whole set with collapsed carets, naming which one is primary.
    pub fn set_carets(&mut self, carets: Vec<usize>, main: usize) {
        let sels = carets.into_iter().map(Selection::caret).collect();
        self.set_selections(sels, main);
    }

    /// Every selection, primary included.
    pub fn iter(&self) -> impl Iterator<Item = Selection> + '_ {
        self.items.iter().copied()
    }

    /// The index of the primary selection within `iter`.
    ///
    /// Callers that rebuild the set from `iter` need this to name the same
    /// selection as primary afterwards, since the primary is an index and not a
    /// property of the selection itself.
    pub fn main_index(&self) -> usize {
        self.main
    }

    /// Number of selections.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the set is empty. Never true in practice; present for clippy.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Sorts by position, merges overlaps, and keeps the primary selection
    /// pointing at the same caret it did before.
    pub fn normalize(&mut self) {
        if self.items.len() <= 1 {
            return;
        }
        let main_caret = self.primary().caret;
        self.items.sort_by_key(|s| (s.start(), s.end()));

        let mut merged: Vec<Selection> = Vec::with_capacity(self.items.len());
        for sel in self.items.drain(..) {
            match merged.last_mut() {
                Some(last) if sel.start() <= last.end() => {
                    if sel.end() > last.end() {
                        last.caret = sel.caret;
                    }
                }
                _ => merged.push(sel),
            }
        }

        self.main = merged
            .iter()
            .position(|s| s.caret == main_caret)
            .unwrap_or(merged.len() - 1);
        self.items = merged;
    }

    /// Remaps every position through `f`, then normalises.
    ///
    /// Used after an edit to keep carets attached to the text they were next to.
    pub fn map(&mut self, f: impl Fn(usize) -> usize) {
        for sel in &mut self.items {
            sel.anchor = f(sel.anchor);
            sel.caret = f(sel.caret);
        }
        self.normalize();
    }
}

/// Remaps a single position across an edit that replaced `range` with `new_len`
/// characters.
pub fn map_position(pos: usize, range: &Range<usize>, new_len: usize) -> usize {
    if pos <= range.start {
        pos
    } else if pos >= range.end {
        pos - (range.end - range.start) + new_len
    } else {
        range.start + new_len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_is_ordered() {
        let sel = Selection::new(9, 3);
        assert_eq!(sel.start(), 3);
        assert_eq!(sel.end(), 9);
        assert_eq!(sel.range(), 3..9);
        assert_eq!(sel.len(), 6);
        assert!(!sel.is_empty());
    }

    #[test]
    fn collapsed_selection_is_empty() {
        let sel = Selection::caret(5);
        assert!(sel.is_empty());
        assert_eq!(sel.len(), 0);
    }

    #[test]
    fn extend_keeps_anchor() {
        let mut sel = Selection::caret(5);
        sel.extend_to(9);
        assert_eq!(sel.anchor, 5);
        assert_eq!(sel.caret, 9);
        sel.extend_to(2);
        assert_eq!(sel.anchor, 5);
        assert_eq!(sel.caret, 2);
        assert_eq!(sel.range(), 2..5);
    }

    #[test]
    fn map_position_before_edit() {
        assert_eq!(map_position(2, &(5..8), 1), 2);
    }

    #[test]
    fn map_position_after_edit() {
        // Replacing 5..8 (3 chars) with 1 char shifts everything after by -2.
        assert_eq!(map_position(10, &(5..8), 1), 8);
    }

    #[test]
    fn map_position_inside_edit() {
        assert_eq!(map_position(6, &(5..8), 1), 6);
        assert_eq!(map_position(6, &(5..8), 4), 9);
    }

    #[test]
    fn normalize_merges_overlaps() {
        let mut sels = Selections {
            items: vec![
                Selection::new(0, 5),
                Selection::new(3, 9),
                Selection::new(20, 22),
            ],
            main: 1,
        };
        sels.normalize();
        assert_eq!(sels.len(), 2);
        assert_eq!(sels.iter().next().unwrap().range(), 0..9);
    }

    #[test]
    fn normalize_keeps_primary_caret() {
        let mut sels = Selections {
            items: vec![Selection::new(0, 5), Selection::new(3, 9)],
            main: 1,
        };
        sels.normalize();
        assert_eq!(sels.primary().caret, 9);
    }

    #[test]
    fn map_shifts_carets() {
        let mut sels = Selections::single(10);
        sels.map(|p| map_position(p, &(0..3), 0));
        assert_eq!(sels.primary().caret, 7);
    }
}
