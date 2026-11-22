//! The editing state machine.
//!
//! `Editor` owns a document, the carets, the scroll window, and the undo history,
//! and exposes the operations a keybinding or menu item maps onto. It has no
//! knowledge of rendering or input events, which is what makes it testable.

use std::ops::Range;

use crate::datetime::now_secs;
use crate::document::Document;
use crate::selection::{map_position, Selection, Selections};
use crate::undo::{Edit, Step, UndoStack};
use crate::viewport::Viewport;

/// The most carets a multi-select will create.
///
/// A cap rather than a limit on the feature: selecting every occurrence of `e` in
/// a large file would otherwise queue a rectangle per character and make the next
/// keystroke quadratic. Ten thousand is far past what anyone edits by hand and
/// far short of what would stall a frame.
const MAX_MULTI_SELECT: usize = 10_000;

/// How many mark styles the Search menu can paint with.
///
/// Notepad++ offers five, and five is what fits in a legend a user can hold in
/// their head: the point of the styles is to tell two kinds of hit apart at a
/// glance, and a sixth colour would stop being distinguishable from the other
/// five before it stopped being useful.
pub const MARK_STYLES: usize = 5;

/// A caret movement, independent of how it was triggered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    /// One character left.
    CharLeft,
    /// One character right.
    CharRight,
    /// One line up, keeping the column where possible.
    LineUp,
    /// One line down, keeping the column where possible.
    LineDown,
    /// To the start of the previous word.
    WordLeft,
    /// To the end of the next word.
    WordRight,
    /// To the first character of the line.
    LineStart,
    /// To just past the last character of the line.
    LineEnd,
    /// To the start of the document.
    DocStart,
    /// To the end of the document.
    DocEnd,
    /// Up by one screenful.
    PageUp,
    /// Down by one screenful.
    PageDown,
}

/// What typing a character should do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Typed {
    /// Insert it and nothing else.
    Plain,
    /// Insert it, then `close`, leaving the caret between the two.
    Pair(char),
    /// The closer is already there; move the caret over it instead.
    Skip,
}

/// Whether `c` closes something.
fn is_closer(c: char) -> bool {
    matches!(c, ')' | ']' | '}' | '"' | '\'' | '`')
}

/// The character that closes `open`, if `open` opens anything.
fn closer_for(open: char) -> Option<char> {
    match open {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '"' => Some('"'),
        '\'' => Some('\''),
        '`' => Some('`'),
        _ => None,
    }
}

/// Whether `c` counts as part of a word for word-wise movement.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The kind of character a double click treats as one unit.
///
/// Three classes rather than two: a word, a run of whitespace, and a run of
/// punctuation. Scintilla ranks characters the same way, which is why a double
/// click on the `.` of `foo.bar` takes the dot and a double click on the space
/// in `a = b` takes the space and not `= b`.
fn unit_class(c: char) -> u8 {
    if is_word_char(c) {
        0
    } else if c.is_whitespace() {
        1
    } else {
        2
    }
}

/// The start of the word before `caret`.
fn word_left(doc: &Document, caret: usize) -> usize {
    let mut pos = caret;
    while pos > 0 && !is_word_char(doc.char_at(pos - 1).unwrap_or(' ')) {
        pos -= 1;
    }
    while pos > 0 && is_word_char(doc.char_at(pos - 1).unwrap_or(' ')) {
        pos -= 1;
    }
    pos
}

/// The end of the word after `caret`.
fn word_right(doc: &Document, caret: usize) -> usize {
    let len = doc.len_chars();
    let mut pos = caret;
    while pos < len && is_word_char(doc.char_at(pos).unwrap_or(' ')) {
        pos += 1;
    }
    while pos < len && !is_word_char(doc.char_at(pos).unwrap_or(' ')) {
        pos += 1;
    }
    pos
}

/// Moves `caret` by `delta` lines, landing on the same column where possible.
///
/// `desired` remembers the column across a run of vertical moves so that passing
/// through a short line does not permanently pull the caret left.
fn vertical_target(
    doc: &Document,
    desired: &mut Option<usize>,
    caret: usize,
    delta: isize,
) -> usize {
    let line = doc.char_to_line(caret);
    let column = *desired.get_or_insert(caret - doc.line_to_char(line));
    let last = doc.len_lines().saturating_sub(1) as isize;
    let target = (line as isize + delta).clamp(0, last) as usize;
    doc.line_to_char(target) + column.min(doc.line_len(target))
}

/// Where `motion` takes a caret from `caret`.
///
/// Free rather than a method because the multi-caret path needs it with a
/// throwaway `desired` per caret, and borrowing `self` mutably once per caret
/// would not allow that.
fn target_of(
    doc: &Document,
    desired: &mut Option<usize>,
    page: isize,
    motion: Motion,
    caret: usize,
) -> usize {
    match motion {
        Motion::CharLeft => caret.saturating_sub(1),
        Motion::CharRight => (caret + 1).min(doc.len_chars()),
        Motion::LineUp => vertical_target(doc, desired, caret, -1),
        Motion::LineDown => vertical_target(doc, desired, caret, 1),
        Motion::PageUp => vertical_target(doc, desired, caret, -page),
        Motion::PageDown => vertical_target(doc, desired, caret, page),
        Motion::WordLeft => word_left(doc, caret),
        Motion::WordRight => word_right(doc, caret),
        Motion::LineStart => doc.line_to_char(doc.char_to_line(caret)),
        Motion::LineEnd => doc.line_range(doc.char_to_line(caret)).end,
        Motion::DocStart => 0,
        Motion::DocEnd => doc.len_chars(),
    }
}

/// A document plus everything the user is doing to it.
#[derive(Debug)]
pub struct Editor {
    /// The text and its per-line state.
    pub doc: Document,
    /// The carets.
    pub selections: Selections,
    /// The scroll window.
    pub viewport: Viewport,
    /// Whether typing replaces the character under the caret.
    ///
    /// Per document, not per window: Notepad++ keeps the mode with the buffer,
    /// so switching tabs does not silently change what the next key does.
    overwrite: bool,
    undo: UndoStack,
    desired_col: Option<usize>,
    /// The five sets of styled marks, as character ranges.
    ///
    /// Kept in the editor rather than the view because the commands that use them
    /// — jump to the next mark, copy the marked text — are document operations,
    /// and a view that owned them would have to be consulted by every one.
    marks: [Vec<Range<usize>>; MARK_STYLES],
}

impl Editor {
    /// Wraps a document in an editor with the caret at the start.
    pub fn new(doc: Document) -> Self {
        Self {
            doc,
            selections: Selections::single(0),
            viewport: Viewport::default(),
            overwrite: false,
            undo: UndoStack::default(),
            desired_col: None,
            marks: Default::default(),
        }
    }

    // ---- caret ----------------------------------------------------------

    /// The primary caret's character index.
    pub fn caret(&self) -> usize {
        self.selections.primary().caret
    }

    /// The line the primary caret is on.
    pub fn caret_line(&self) -> usize {
        self.doc.char_to_line(self.caret())
    }

    /// The primary caret's column, in characters.
    pub fn caret_column(&self) -> usize {
        self.caret() - self.doc.line_to_char(self.caret_line())
    }

    /// Places the caret at `pos`, collapsing any selection.
    pub fn set_caret(&mut self, pos: usize) {
        self.selections.set_caret(pos.min(self.doc.len_chars()));
        self.undo.break_group();
        self.desired_col = None;
    }

    /// Places the caret at the start of `line`, clamped to the document.
    pub fn goto_line(&mut self, line: usize) {
        self.goto_position(line, 0);
    }

    /// Places the caret at `column` of `line`, clamped to both.
    pub fn goto_position(&mut self, line: usize, column: usize) {
        let line = line.min(self.doc.len_lines().saturating_sub(1));
        let column = column.min(self.doc.line_len(line));
        let pos = self.doc.line_to_char(line) + column;
        self.set_caret(pos);
        self.ensure_caret_visible();
    }

    /// Moves the caret to the next or previous bookmark.
    ///
    /// Returns false when the document has no bookmarks, so the caller can say
    /// so rather than silently doing nothing.
    pub fn goto_bookmark(&mut self, forward: bool) -> bool {
        let line = self.caret_line();
        let hit = if forward {
            self.doc.next_bookmark(line)
        } else {
            self.doc.prev_bookmark(line)
        };
        match hit {
            Some(line) => {
                self.goto_line(line);
                true
            }
            None => false,
        }
    }

    /// The character index under a point in the text area, in pixels.
    ///
    /// Wrapping is resolved here rather than by the caller so that a plain
    /// click, a Ctrl+click and anything else that turns a point into a position
    /// all agree on where the point lands.
    pub fn char_at_point(&mut self, x: f32, y: f32) -> usize {
        let total = self.doc.visible_line_count();
        let row = self.viewport.y_to_line(y, total);
        let line = self.doc.line_of_row(row);
        let segment = self.doc.segment_of_row(row) as usize;
        let wrap = self.doc.wrap_cols();
        let column = if wrap == 0 {
            self.viewport.x_to_column(x).min(self.doc.line_len(line))
        } else {
            let within = self.viewport.x_to_column(x);
            (segment * wrap + within).min(self.doc.line_len(line))
        };
        self.doc.line_to_char(line) + column
    }

    /// Places the caret from a point in the text area, in pixels.
    pub fn set_caret_from_point(&mut self, x: f32, y: f32, extend: bool) {
        let pos = self.char_at_point(x, y);
        if extend {
            let mut sel = self.selections.primary();
            sel.extend_to(pos);
            self.selections.set_single(sel);
        } else {
            self.selections.set_caret(pos);
        }
        self.undo.break_group();
        self.desired_col = None;
    }

    // ---- multiple carets -------------------------------------------------

    /// How many carets are active.
    pub fn caret_count(&self) -> usize {
        self.selections.len()
    }

    /// Adds a collapsed caret one line above the topmost one.
    ///
    /// Returns false at the first line, where there is nothing to add. The new
    /// caret keeps the column of the caret it grew from, clamped to the target
    /// line, so a run of presses walks straight up a block of text instead of
    /// drifting left at every short line.
    pub fn add_caret_above(&mut self) -> bool {
        self.grow_carets(-1)
    }

    /// Adds a collapsed caret one line below the bottommost one.
    pub fn add_caret_below(&mut self) -> bool {
        self.grow_carets(1)
    }

    /// Grows the caret set by one line, upwards for a negative `delta`.
    ///
    /// The caret that grows is the outermost one in that direction rather than
    /// the most recently added: with carets already spread over several lines,
    /// "one more above" has to mean above all of them or the set stops being a
    /// contiguous block.
    fn grow_carets(&mut self, delta: isize) -> bool {
        let edge = if delta < 0 {
            self.selections.iter().min_by_key(|s| s.caret)
        } else {
            self.selections.iter().max_by_key(|s| s.caret)
        };
        let Some(edge) = edge else {
            return false;
        };
        let line = self.doc.char_to_line(edge.caret);
        let last = self.doc.len_lines().saturating_sub(1);
        let target = line as isize + delta;
        if target < 0 || target as usize > last {
            return false;
        }
        let target = target as usize;
        let column = edge.caret - self.doc.line_to_char(line);
        let pos = self.doc.line_to_char(target) + column.min(self.doc.line_len(target));

        let main = self.selections.main_index();
        let mut sels: Vec<Selection> = self.selections.iter().collect();
        sels.push(Selection::caret(pos));
        self.selections.set_selections(sels, main);
        self.selections.normalize();
        self.undo.break_group();
        self.desired_col = None;
        true
    }

    /// Adds a collapsed caret at `pos` and makes it the primary one.
    ///
    /// Ctrl+click lands here. The new caret becomes primary so the next
    /// keystroke anchors where the user just pointed, which is what makes
    /// "click, click, click, type" behave the way it looks like it should.
    pub fn add_caret_at(&mut self, pos: usize) {
        let pos = pos.min(self.doc.len_chars());
        let mut sels: Vec<Selection> = self.selections.iter().collect();
        sels.push(Selection::caret(pos));
        let main = sels.len() - 1;
        self.selections.set_selections(sels, main);
        self.selections.normalize();
        self.undo.break_group();
        self.desired_col = None;
    }

    /// Drops every caret but the primary one. Returns whether any were dropped.
    pub fn clear_extra_carets(&mut self) -> bool {
        if self.selections.len() <= 1 {
            return false;
        }
        let caret = self.selections.primary().caret;
        self.selections.set_caret(caret);
        self.desired_col = None;
        true
    }

    /// Selects the whole document.
    pub fn select_all(&mut self) {
        self.selections
            .set_single(Selection::new(0, self.doc.len_chars()));
        self.undo.break_group();
        self.desired_col = None;
    }

    /// How many characters are selected.
    pub fn selection_len(&self) -> usize {
        self.selections
            .iter()
            .map(|sel| sel.end().saturating_sub(sel.start()))
            .sum()
    }

    /// The caret as a `(line, column)` pair.
    pub fn caret_point(&self) -> (usize, usize) {
        (self.caret_line(), self.caret_column())
    }

    /// Selects the rectangular block between two `(line, column)` points.
    ///
    /// Columns are clamped per line, so a block dragged past the end of a short
    /// line stops at that line's end rather than spilling onto the next one.
    pub fn select_block(&mut self, anchor: (usize, usize), caret: (usize, usize)) {
        let first = anchor.0.min(caret.0);
        let last = anchor.0.max(caret.0);
        let left = anchor.1.min(caret.1);
        let right = anchor.1.max(caret.1);

        let mut sels = Vec::with_capacity(last - first + 1);
        for line in first..=last {
            let start = self.doc.line_to_char(line);
            let len = self.doc.line_len(line);
            sels.push(Selection::new(
                start + left.min(len),
                start + right.min(len),
            ));
        }
        let main = caret.0.saturating_sub(first);
        self.selections.set_selections(sels, main);
        self.desired_col = None;
    }

    /// Deletes every selection as one undo group.
    ///
    /// Returns false when nothing is selected.
    pub fn delete_all_selections(&mut self) -> bool {
        let mut ranges: Vec<Range<usize>> = self
            .selections
            .iter()
            .filter(|sel| !sel.is_empty())
            .map(|sel| sel.range())
            .collect();
        if ranges.is_empty() {
            return false;
        }
        ranges.sort_by_key(|r| r.start);
        let edits: Vec<(Range<usize>, String)> =
            ranges.into_iter().map(|r| (r, String::new())).collect();
        self.replace_ranges(&edits);
        true
    }

    /// The selected text, with selections joined by newlines.
    pub fn selected_text(&self) -> String {
        let mut out = String::new();
        for sel in self.selections.iter() {
            if sel.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&self.doc.slice(sel.range()));
        }
        out
    }

    /// Moves the primary caret. `extend` keeps the anchor, growing the selection.
    pub fn move_caret(&mut self, motion: Motion, extend: bool) {
        if self.selections.len() > 1 {
            self.move_all_carets(motion, extend);
            return;
        }
        let sel = self.selections.primary();

        // A horizontal move with a selection collapses to its edge instead of
        // stepping one character past it.
        if !extend && !sel.is_empty() {
            let edge = match motion {
                Motion::CharLeft | Motion::WordLeft | Motion::LineStart => Some(sel.start()),
                Motion::CharRight | Motion::WordRight | Motion::LineEnd => Some(sel.end()),
                _ => None,
            };
            if let Some(pos) = edge {
                self.selections.set_caret(pos);
                self.undo.break_group();
                self.desired_col = None;
                return;
            }
        }

        let target = self.compute_target(motion, sel.caret);
        let mut next = sel;
        if extend {
            next.extend_to(target);
        } else {
            next.collapse_to(target);
        }
        self.selections.set_single(next);
        self.undo.break_group();

        if !matches!(
            motion,
            Motion::LineUp | Motion::LineDown | Motion::PageUp | Motion::PageDown
        ) {
            self.desired_col = None;
        }
    }

    /// Moves every caret by `motion`, keeping the primary one primary.
    ///
    /// Each caret keeps its own column, so a stack of carets walks down a block
    /// of text in parallel. The remembered column is deliberately not shared:
    /// one `desired_col` for several carets would drag them all onto the same
    /// column the moment one of them crossed a short line, which is exactly the
    /// thing the remembered column exists to prevent.
    fn move_all_carets(&mut self, motion: Motion, extend: bool) {
        let page = self.viewport.full_line_count() as isize;
        let main = self.selections.main_index();
        let mut out = Vec::with_capacity(self.selections.len());
        for sel in self.selections.iter() {
            // Same rule as the single-caret path: a horizontal move with a
            // selection collapses to its edge instead of stepping past it.
            if !extend && !sel.is_empty() {
                let edge = match motion {
                    Motion::CharLeft | Motion::WordLeft | Motion::LineStart => Some(sel.start()),
                    Motion::CharRight | Motion::WordRight | Motion::LineEnd => Some(sel.end()),
                    _ => None,
                };
                if let Some(pos) = edge {
                    out.push(Selection::caret(pos));
                    continue;
                }
            }
            let mut desired = None;
            let target = target_of(&self.doc, &mut desired, page, motion, sel.caret);
            let mut next = sel;
            if extend {
                next.extend_to(target);
            } else {
                next.collapse_to(target);
            }
            out.push(next);
        }
        self.selections.set_selections(out, main);
        self.selections.normalize();
        self.undo.break_group();
        self.desired_col = None;
    }

    /// Where `motion` takes the caret from `caret`.
    fn compute_target(&mut self, motion: Motion, caret: usize) -> usize {
        let page = self.viewport.full_line_count() as isize;
        target_of(&self.doc, &mut self.desired_col, page, motion, caret)
    }

    // ---- editing --------------------------------------------------------

    /// Inserts `text` at every caret, replacing any selection.
    ///
    /// Every caret survives the edit, which is what makes typing into a column
    /// block work: one keystroke lands on every line of the block.
    ///
    /// A multi-caret edit is recorded as **one** undo step. The step is the whole
    /// span the carets cover, with the untouched text between them carried in
    /// both the removed and the inserted string — so undo restores every line at
    /// once instead of one line per press.
    pub fn insert(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let primary = self.selections.primary();
        let mut sels: Vec<Selection> = self.selections.iter().collect();
        // Apply back to front so earlier ranges stay valid.
        sels.sort_by_key(|s| std::cmp::Reverse(s.start()));
        let main = sels.iter().position(|s| *s == primary).unwrap_or(0);
        let text_len = text.chars().count();

        let span = (sels.len() > 1).then(|| {
            let lo = sels.last().map_or(0, |s| s.start());
            let hi = sels.first().map_or(0, |s| s.end());
            lo..hi
        });
        let removed_span = span.clone().map(|range| self.doc.slice(range));

        let mut removed = Vec::with_capacity(sels.len());
        let mut carets = Vec::with_capacity(sels.len());
        for sel in &sels {
            let range = sel.range();
            removed.push(self.doc.slice(range.clone()));
            self.replace_doc(range.clone(), text);
            carets.push(range.start + text_len);
        }

        // The carets were recorded against the document as it was before the
        // edit, so each one shifts by the net change of every edit below it.
        let mut order: Vec<usize> = (0..sels.len()).collect();
        order.sort_by_key(|&i| sels[i].start());
        let mut delta = 0isize;
        for &i in &order {
            carets[i] = (carets[i] as isize + delta) as usize;
            delta += text_len as isize - sels[i].len() as isize;
        }

        match (span, removed_span) {
            (Some(span), Some(removed)) => {
                let new_len = (span.end - span.start) as isize + delta;
                let inserted = self.doc.slice(span.start..span.start + new_len as usize);
                self.undo.break_group();
                self.record(Edit {
                    range: span,
                    removed,
                    inserted,
                    caret_before: primary.caret,
                    caret_after: carets[main],
                });
            }
            _ => {
                let sel = sels[0];
                self.record(Edit {
                    range: sel.range(),
                    removed: removed.pop().unwrap_or_default(),
                    inserted: text.to_string(),
                    caret_before: sel.caret,
                    caret_after: carets[0],
                });
            }
        }

        // The carets were built in the order the edits were applied, which is
        // back to front. Normalising restores the sorted, non-overlapping
        // invariant the rest of `Selections` relies on.
        self.selections.set_carets(carets, main);
        self.selections.normalize();
        self.desired_col = None;
    }

    /// Whether typing replaces the character under the caret.
    pub fn overwrite(&self) -> bool {
        self.overwrite
    }

    /// Turns overwrite mode on or off.
    pub fn set_overwrite(&mut self, on: bool) {
        self.overwrite = on;
    }

    /// Flips overwrite mode and returns the new state.
    pub fn toggle_overwrite(&mut self) -> bool {
        self.overwrite = !self.overwrite;
        self.overwrite
    }

    /// Types `c` at the caret, replacing the character under it in overwrite
    /// mode.
    ///
    /// Three cases fall back to a plain insert, and each is a decision rather
    /// than an oversight:
    ///
    /// - A selection is replaced, not overwritten. Overwriting a selection would
    ///   delete text the user pointed at and then some.
    /// - At the end of a line there is nothing to replace, so the character is
    ///   appended. Without this the caret would stick at a line end.
    /// - A newline is never overwritten. Joining two lines because the user
    ///   typed a letter is not a trade anyone would accept.
    pub fn overwrite_char(&mut self, c: char) {
        let sel = self.selections.primary();
        if !sel.is_empty() {
            self.insert(&c.to_string());
            return;
        }
        let at_end = sel.caret >= self.doc.len_chars()
            || matches!(self.doc.char_at(sel.caret), Some('\n') | Some('\r'));
        if at_end {
            self.insert(&c.to_string());
            return;
        }
        let range = sel.caret..sel.caret + 1;
        let removed = self.doc.slice(range.clone());
        let inserted = c.to_string();
        self.replace_doc(range.clone(), &inserted);
        // Pushed without breaking the group, so a run of overwritten characters
        // undoes as one step the way a run of typed ones does.
        self.record(Edit {
            range,
            removed,
            inserted,
            caret_before: sel.caret,
            caret_after: sel.caret + 1,
        });
        self.selections.set_caret(sel.caret + 1);
        self.desired_col = None;
    }

    /// Replaces several ranges at once, as a single undo group.
    ///
    /// Ranges must be sorted and non-overlapping. They are applied back to front
    /// so that each one's offsets are still valid when it is reached.
    pub fn replace_ranges(&mut self, edits: &[(Range<usize>, String)]) {
        if edits.is_empty() {
            return;
        }
        let span = edits[0].0.start..edits[edits.len() - 1].0.end;
        let removed = self.doc.slice(span.clone());
        let mut delta = 0isize;
        for (range, text) in edits.iter().rev() {
            self.replace_doc(range.clone(), text);
            delta += text.chars().count() as isize - (range.end - range.start) as isize;
        }
        let new_len = (span.end - span.start) as isize + delta;
        let inserted = self.doc.slice(span.start..span.start + new_len as usize);
        let caret = edits[0].0.start + edits[0].1.chars().count();

        self.undo.break_group();
        self.record(Edit {
            range: span,
            removed,
            inserted,
            caret_before: self.selections.primary().caret,
            caret_after: caret,
        });
        self.undo.break_group();
        self.selections.set_caret(caret);
        self.desired_col = None;
    }

    /// The lower bound of the primary selection.
    pub fn selection_start(&self) -> usize {
        self.selections.primary().start()
    }

    /// The upper bound of the primary selection.
    pub fn selection_end(&self) -> usize {
        self.selections.primary().end()
    }

    /// Selects `range` and puts the caret at its end.
    pub fn select_range(&mut self, range: Range<usize>) {
        self.selections
            .set_single(Selection::new(range.start, range.end));
        self.desired_col = None;
    }

    /// Deletes the selection, if there is one.
    ///
    /// Returns false when the selection is empty, so callers can fall back to
    /// whatever "nothing selected" means for them.
    pub fn delete_selection(&mut self) -> bool {
        let sel = self.selections.primary();
        if sel.is_empty() {
            return false;
        }
        self.delete_range(sel.range());
        true
    }

    /// The text of `line`, including its terminator.
    ///
    /// Used by copy-with-no-selection, which takes the whole line the way
    /// Notepad++ does.
    pub fn line_with_break(&self, line: usize) -> String {
        let start = self.doc.line_to_char(line);
        let end = if line + 1 < self.doc.len_lines() {
            self.doc.line_to_char(line + 1)
        } else {
            self.doc.len_chars()
        };
        self.doc.slice(start..end)
    }

    /// The indent the next line should start with.
    ///
    /// The current line's leading whitespace, plus one level when the caret sits
    /// just after an opening bracket. Notepad++'s behaviour, minus the part
    /// where it guesses the language's indent width: four spaces is what the Tab
    /// key inserts, so it is what Enter inserts too.
    pub fn auto_indent(&self) -> String {
        let line = self.caret_line();
        let start = self.doc.line_to_char(line);
        let before = self.doc.slice(start..self.caret());
        let indent: String = before
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let trimmed = before.trim_end();
        if trimmed.ends_with('{') || trimmed.ends_with('(') || trimmed.ends_with('[') {
            format!("{indent}    ")
        } else {
            indent
        }
    }

    /// The word the caret is on, or the selection when there is one.
    ///
    /// This is what smart highlighting marks: Notepad++ lights up every other
    /// occurrence of the word you are standing on. A selection wins, so selecting
    /// a phrase highlights the phrase rather than the word around it.
    ///
    /// The word ending at the caret is preferred over the one starting at it, so
    /// that standing just after `foo` highlights `foo` and not the next token.
    pub fn highlight_word(&self) -> Option<Range<usize>> {
        let primary = self.selections.primary();
        if primary.start() != primary.end() {
            return Some(primary.range());
        }
        let len = self.doc.len_chars();
        let caret = self.caret();
        let mut start = caret;
        while start > 0 && is_word_char(self.doc.char_at(start - 1).unwrap_or(' ')) {
            start -= 1;
        }
        let mut end = caret;
        while end < len && is_word_char(self.doc.char_at(end).unwrap_or(' ')) {
            end += 1;
        }
        (end > start).then_some(start..end)
    }

    /// Every whole-word occurrence of `word`, as character ranges.
    ///
    /// Whole-word because `foo` lighting up inside `foobar` is noise, and capped
    /// because a one-letter word in a large file would otherwise queue a
    /// rectangle per character.
    pub fn occurrences(&self, word: &str, limit: usize) -> Vec<Range<usize>> {
        if word.is_empty() || limit == 0 {
            return Vec::new();
        }
        let text = self.doc.text();
        let mut out = Vec::new();
        let mut from = 0;
        while let Some(rel) = text[from..].find(word) {
            let start = from + rel;
            let end = start + word.len();
            let before_ok = text[..start]
                .chars()
                .next_back()
                .is_none_or(|c| !is_word_char(c));
            let after_ok = text[end..].chars().next().is_none_or(|c| !is_word_char(c));
            if before_ok && after_ok {
                out.push(self.doc.byte_to_char(start)..self.doc.byte_to_char(end));
                if out.len() >= limit {
                    break;
                }
            }
            from = start + word.chars().next().map_or(1, char::len_utf8);
        }
        out
    }

    /// Every occurrence of `needle`, word boundaries ignored.
    ///
    /// `Mark All` marks what the user selected, and a selection is often not a
    /// word — `foo(` or `->` are perfectly reasonable things to mark.
    pub fn find_all(&self, needle: &str, limit: usize) -> Vec<Range<usize>> {
        if needle.is_empty() || limit == 0 {
            return Vec::new();
        }
        let text = self.doc.text();
        let mut out = Vec::new();
        let mut from = 0;
        while let Some(rel) = text[from..].find(needle) {
            let start = from + rel;
            let end = start + needle.len();
            out.push(self.doc.byte_to_char(start)..self.doc.byte_to_char(end));
            if out.len() >= limit {
                break;
            }
            from = start + needle.chars().next().map_or(1, char::len_utf8);
        }
        out
    }

    /// Every occurrence of `needle` under the given matching rules.
    ///
    /// `whole_word` asks for word boundaries on both sides; `match_case` asks for
    /// an exact byte match rather than a case-folded one. Both are Notepad++'s
    /// four combinations, and they are separate flags rather than an enum because
    /// the menu offers the four combinations and nothing else.
    pub fn find_all_matching(
        &self,
        needle: &str,
        match_case: bool,
        whole_word: bool,
        limit: usize,
    ) -> Vec<Range<usize>> {
        if needle.is_empty() || limit == 0 {
            return Vec::new();
        }
        let text = self.doc.text();
        let haystack = if match_case {
            text.clone()
        } else {
            text.to_lowercase()
        };
        let needle_cmp = if match_case {
            needle.to_string()
        } else {
            needle.to_lowercase()
        };
        let mut out = Vec::new();
        let mut from = 0;
        while let Some(rel) = haystack[from..].find(&needle_cmp) {
            let start = from + rel;
            let end = start + needle_cmp.len();
            let before_ok = !whole_word
                || text[..start]
                    .chars()
                    .next_back()
                    .is_none_or(|c| !is_word_char(c));
            let after_ok = !whole_word
                || text[end..].chars().next().is_none_or(|c| !is_word_char(c));
            if before_ok && after_ok {
                out.push(self.doc.byte_to_char(start)..self.doc.byte_to_char(end));
                if out.len() >= limit {
                    break;
                }
            }
            from = start + needle_cmp.chars().next().map_or(1, char::len_utf8);
        }
        out
    }

    /// The text a multi-select should look for: the selection, or the word under
    /// the caret when nothing is selected.
    pub fn multi_select_needle(&self) -> Option<String> {
        let primary = self.selections.primary();
        if primary.anchor != primary.caret {
            let (from, to) = (
                primary.anchor.min(primary.caret),
                primary.anchor.max(primary.caret),
            );
            return Some(self.doc.slice(from..to));
        }
        self.highlight_word()
            .map(|range| self.doc.slice(range.start..range.end))
    }

    /// Puts a caret on every occurrence of the current word or selection.
    ///
    /// Returns how many carets there now are. One occurrence is the one the user
    /// is already on, so a word that appears once leaves a single caret and the
    /// command has visibly done nothing -- which is the honest answer.
    pub fn select_all_occurrences(&mut self, match_case: bool, whole_word: bool) -> usize {
        let Some(needle) = self.multi_select_needle() else {
            return self.selections.len();
        };
        let found = self.find_all_matching(&needle, match_case, whole_word, MAX_MULTI_SELECT);
        if found.len() < 2 {
            return self.selections.len();
        }
        let sels: Vec<Selection> = found
            .into_iter()
            .map(|range| Selection::new(range.start, range.end))
            .collect();
        let main = sels.len() - 1;
        self.selections.set_selections(sels, main);
        self.undo.break_group();
        self.desired_col = None;
        self.selections.len()
    }

    /// Adds a caret on the next occurrence after the last one selected.
    ///
    /// Returns false when there is no next occurrence, which is the caller's cue
    /// to say so rather than to silently do nothing.
    pub fn select_next_occurrence(&mut self, match_case: bool, whole_word: bool) -> bool {
        let Some(needle) = self.multi_select_needle() else {
            return false;
        };
        let found = self.find_all_matching(&needle, match_case, whole_word, MAX_MULTI_SELECT);
        if found.is_empty() {
            return false;
        }
        let last = self
            .selections
            .iter()
            .map(|sel| sel.caret.max(sel.anchor))
            .max()
            .unwrap_or(0);
        let Some(next) = found.iter().find(|range| range.start >= last) else {
            return false;
        };
        let mut sels: Vec<Selection> = self.selections.iter().collect();
        sels.push(Selection::new(next.start, next.end));
        let main = sels.len() - 1;
        self.selections.set_selections(sels, main);
        self.undo.break_group();
        self.desired_col = None;
        true
    }

    /// Drops the current occurrence and moves on to the next one.
    ///
    /// Notepad++'s "Skip Current and Get Next": the occurrence you did not want
    /// stops being selected, and the caret lands on the following one so the next
    /// Select Next adds that instead. The remaining selections are left alone,
    /// which is what makes skipping useful in the middle of a long run.
    pub fn skip_occurrence(&mut self, match_case: bool, whole_word: bool) -> bool {
        let Some(needle) = self.multi_select_needle() else {
            return false;
        };
        let found = self.find_all_matching(&needle, match_case, whole_word, MAX_MULTI_SELECT);
        if found.is_empty() {
            return false;
        }
        let mut sels: Vec<Selection> = self.selections.iter().collect();
        // The occurrence being skipped is the primary one; with a single caret
        // there is nothing to drop, so only the move happens.
        if sels.len() > 1 {
            let primary = self.selections.primary_index();
            sels.remove(primary);
        }
        let last = sels
            .iter()
            .map(|sel| sel.caret.max(sel.anchor))
            .max()
            .unwrap_or(0);
        let Some(next) = found.iter().find(|range| range.start >= last) else {
            return false;
        };
        let main = sels.len();
        sels.push(Selection::new(next.start, next.start));
        self.selections.set_selections(sels, main);
        self.desired_col = None;
        true
    }

    /// Drops the most recently added caret, keeping at least one.
    pub fn undo_multi_select(&mut self) -> bool {
        if self.selections.len() <= 1 {
            return false;
        }
        let mut sels: Vec<Selection> = self.selections.iter().collect();
        sels.pop();
        let main = sels.len() - 1;
        self.selections.set_selections(sels, main);
        self.desired_col = None;
        true
    }

    /// What typing `c` should do, given the text around the caret.
    ///
    /// Notepad++ closes brackets and quotes as you type and steps over a closer
    /// that is already there. The rules that keep it from being annoying:
    ///
    /// - A selection is wrapped rather than replaced.
    /// - Nothing is closed in front of a word, so `(` before `foo` does not
    ///   produce `(foo)`.
    /// - A quote is not closed after a word, so `don't` stays `don't`.
    pub fn typed(&self, c: char) -> Typed {
        let len = self.doc.len_chars();
        let caret = self.caret();
        let next = (caret < len).then(|| self.doc.char_at(caret)).flatten();
        let prev = caret.checked_sub(1).and_then(|p| self.doc.char_at(p));

        // Typing a closer that is already there steps over it rather than
        // doubling it, which is what makes typing `()` by hand feel right.
        if is_closer(c) && next == Some(c) {
            return Typed::Skip;
        }
        let Some(close) = closer_for(c) else {
            return Typed::Plain;
        };
        let primary = self.selections.primary();
        if primary.start() != primary.end() {
            return Typed::Pair(close);
        }
        if next == Some(close) {
            return Typed::Skip;
        }
        if next.is_some_and(is_word_char) {
            return Typed::Plain;
        }
        let quote = c == close;
        if quote && (prev.is_some_and(is_word_char) || next.is_some_and(is_word_char)) {
            return Typed::Plain;
        }
        Typed::Pair(close)
    }

    /// The bracket pair the caret is next to, as `(open, close)`.
    ///
    /// The character before the caret is tried first, then the one at it, so
    /// both "just typed a brace" and "just moved past one" highlight.
    ///
    /// The scan is bounded: a file with an unbalanced brace would otherwise walk
    /// to the end of the document on every keystroke. It also does not know about
    /// strings or comments, which is the same trade Notepad++ makes — a brace
    /// inside a string still matches, and the alternative is running the lexer
    /// from the top of the file on every caret move.
    pub fn matching_brace(&self) -> Option<(usize, usize)> {
        let len = self.doc.len_chars();
        let caret = self.caret();
        for pos in [caret.checked_sub(1), Some(caret)].into_iter().flatten() {
            if pos >= len {
                continue;
            }
            let Some(c) = self.doc.char_at(pos) else {
                continue;
            };
            let (open, close, forward) = match c {
                '(' => ('(', ')', true),
                '[' => ('[', ']', true),
                '{' => ('{', '}', true),
                ')' => ('(', ')', false),
                ']' => ('[', ']', false),
                '}' => ('{', '}', false),
                _ => continue,
            };
            if let Some(partner) = self.scan_brace(pos, open, close, forward) {
                return Some(if forward {
                    (pos, partner)
                } else {
                    (partner, pos)
                });
            }
        }
        None
    }

    /// Walks from `pos` looking for the bracket that balances it.
    fn scan_brace(&self, pos: usize, open: char, close: char, forward: bool) -> Option<usize> {
        /// How far to look before giving up, in characters.
        const WINDOW: usize = 200_000;
        let len = self.doc.len_chars();
        let (from, to) = if forward {
            (pos, (pos + WINDOW).min(len))
        } else {
            (pos.saturating_sub(WINDOW), pos + 1)
        };
        let text = self.doc.slice(from..to);
        let mut depth = 0i32;
        if forward {
            for (byte, ch) in text.char_indices().skip(1) {
                if ch == open {
                    depth += 1;
                } else if ch == close {
                    if depth == 0 {
                        return Some(from + text[..byte].chars().count());
                    }
                    depth -= 1;
                }
            }
        } else {
            for (byte, ch) in text.char_indices().rev().skip(1) {
                if ch == close {
                    depth += 1;
                } else if ch == open {
                    if depth == 0 {
                        return Some(from + text[..byte].chars().count());
                    }
                    depth -= 1;
                }
            }
        }
        None
    }

    /// The first and last line the selection covers, or the caret line.
    ///
    /// A selection that ends exactly at the start of a line does not include
    /// that line: dragging down to column zero of line 5 and then sorting should
    /// not drag line 5 into the sort.
    pub fn selected_lines(&self) -> (usize, usize) {
        let sel = self.selections.primary();
        let first = self.doc.char_to_line(sel.start());
        let mut last = self.doc.char_to_line(sel.end());
        if last > first && self.doc.line_to_char(last) == sel.end() {
            last -= 1;
        }
        (first, last)
    }

    /// The character range covering whole lines `first..=last`, terminators
    /// included.
    pub fn line_block(&self, first: usize, last: usize) -> Range<usize> {
        let start = self.doc.line_to_char(first);
        let end = if last + 1 < self.doc.len_lines() {
            self.doc.line_to_char(last + 1)
        } else {
            self.doc.len_chars()
        };
        start..end
    }

    /// Applies `f` to the whole lines the selection covers, as one undo step.
    ///
    /// Returns false when `f` leaves the text unchanged, so the caller can skip
    /// the undo entry and the redraw.
    pub fn transform_lines(&mut self, f: impl Fn(&str) -> String) -> bool {
        let (first, last) = self.selected_lines();
        self.transform_span(first, last, f)
    }

    /// Applies `f` to lines `first..=last`, as one undo step.
    pub fn transform_span(
        &mut self,
        first: usize,
        last: usize,
        f: impl Fn(&str) -> String,
    ) -> bool {
        let range = self.line_block(first, last);
        let text = self.doc.slice(range.clone());
        let new = f(&text);
        if new == text {
            return false;
        }
        self.replace_ranges(&[(range, new)]);
        true
    }

    /// Repeats the selected lines below themselves.
    pub fn duplicate_lines(&mut self) -> bool {
        self.transform_lines(crate::lines::duplicate)
    }

    /// Moves the selected lines one line up or down.
    ///
    /// The block is widened by the neighbouring line so the transform can rotate
    /// within it; at the top or bottom of the document there is nothing to swap
    /// with and the call is a no-op.
    pub fn move_lines(&mut self, up: bool) -> bool {
        let (first, last) = self.selected_lines();
        if up {
            if first == 0 {
                return false;
            }
            self.transform_span(first - 1, last, |text| crate::lines::shift(text, true))
        } else {
            if last + 1 >= self.doc.len_lines() {
                return false;
            }
            self.transform_span(first, last + 1, |text| crate::lines::shift(text, false))
        }
    }

    /// Deletes every line the selection touches, terminators included.
    ///
    /// The last line of the document has no terminator to take with it, so the
    /// block is widened backwards instead: deleting the final line leaves the
    /// caret at the end of the line above rather than on an empty one.
    pub fn delete_lines(&mut self) -> bool {
        let (first, last) = self.selected_lines();
        let mut range = self.line_block(first, last);
        if range.is_empty() {
            return false;
        }
        if range.end == self.doc.len_chars() && first > 0 {
            // Take the terminator before the block instead of the one after it,
            // which does not exist.
            range.start = self.doc.line_to_char(first) - 1;
        }
        self.replace_ranges(&[(range, String::new())]);
        true
    }

    /// Selects the whole line the caret is on, terminator excluded.
    pub fn select_line(&mut self) {
        let line = self.caret_line();
        let range = self.doc.line_range(line);
        self.select_range(range);
    }

    // ---- the pointer's units ---------------------------------------------

    /// The unit of text at `pos`, for a double click.
    ///
    /// A word, or the run of whitespace or punctuation the position sits in --
    /// see [`unit_class`]. The run never crosses a line ending, so a double
    /// click on the space before a newline takes that space rather than the
    /// indentation of the line below.
    ///
    /// A position on a line ending, or past the end of the document, counts as
    /// a press on the last character of the line it is on: that is where the
    /// pointer actually is when it is beyond the text, and selecting the newline
    /// is never what the second press of a double click meant. An empty line
    /// gives `None`, because there is nothing there to select.
    pub fn word_range_at(&self, pos: usize) -> Option<Range<usize>> {
        let len = self.doc.len_chars();
        if len == 0 {
            return None;
        }
        let line = self.doc.char_to_line(pos.min(len));
        let bounds = self.doc.line_range(line);
        if bounds.is_empty() {
            return None;
        }
        let seed = pos.clamp(bounds.start, bounds.end - 1);
        let class = unit_class(self.doc.char_at(seed)?);
        let same = |doc: &Document, i: usize| {
            doc.char_at(i).is_some_and(|c| unit_class(c) == class)
        };
        let mut start = seed;
        while start > bounds.start && same(&self.doc, start - 1) {
            start -= 1;
        }
        let mut end = seed + 1;
        while end < bounds.end && same(&self.doc, end) {
            end += 1;
        }
        Some(start..end)
    }

    /// The line at `pos`, terminator included, for a triple click.
    ///
    /// Including the terminator is what makes the result useful: copy a line
    /// selected this way and pasting it gives a line, not a line and half of the
    /// next one glued together.
    pub fn line_range_at(&self, pos: usize) -> Range<usize> {
        let line = self.doc.char_to_line(pos.min(self.doc.len_chars()));
        let bounds = self.doc.line_range(line);
        let mut end = bounds.end;
        while self
            .doc
            .char_at(end)
            .is_some_and(|c| c == '\n' || c == '\r')
        {
            end += 1;
        }
        bounds.start..end
    }

    /// Selects the word, or the run of whitespace or punctuation, under `pos`.
    ///
    /// Nothing to select -- an empty line, an empty document -- leaves the
    /// selection where it was rather than clearing it, so a double click on
    /// blank space past the text does not throw away what the user had.
    pub fn select_word_at(&mut self, pos: usize) {
        if let Some(range) = self.word_range_at(pos) {
            self.select_range(range);
        }
    }

    /// Selects the line under `pos`, terminator included.
    pub fn select_line_at(&mut self, pos: usize) {
        let range = self.line_range_at(pos);
        self.select_range(range);
    }

    /// Deletes the selection, or the character before the caret.
    ///
    /// With several carets every one of them deletes, as a single undo step.
    pub fn backspace(&mut self) {
        if self.selections.len() > 1 {
            self.delete_at_carets(false);
            return;
        }
        let sel = self.selections.primary();
        if !sel.is_empty() {
            self.delete_range(sel.range());
        } else if sel.caret > 0 {
            self.delete_range(sel.caret - 1..sel.caret);
        }
    }

    /// Deletes the selection, or the character after the caret.
    ///
    /// With several carets every one of them deletes, as a single undo step.
    pub fn delete_forward(&mut self) {
        if self.selections.len() > 1 {
            self.delete_at_carets(true);
            return;
        }
        let sel = self.selections.primary();
        if !sel.is_empty() {
            self.delete_range(sel.range());
        } else if sel.caret < self.doc.len_chars() {
            self.delete_range(sel.caret..sel.caret + 1);
        }
    }

    /// Deletes one character at every caret, backwards or forwards.
    ///
    /// The whole run is one undo step spanning from the first deletion to the
    /// last, with the untouched text between them carried in both strings --
    /// the same shape `insert` uses, so undo puts every line back at once
    /// instead of one line per press.
    ///
    /// A caret with nothing to delete in that direction (at the start of the
    /// document going backwards, at the end going forwards) is skipped rather
    /// than clamped, so it stays where it is while its neighbours eat.
    fn delete_at_carets(&mut self, forward: bool) {
        let primary = self.selections.primary();
        let main = self.selections.main_index();
        let mut ranges: Vec<(Range<usize>, bool)> = Vec::with_capacity(self.selections.len());
        for (i, sel) in self.selections.iter().enumerate() {
            let range = if !sel.is_empty() {
                sel.range()
            } else if forward {
                if sel.caret >= self.doc.len_chars() {
                    continue;
                }
                sel.caret..sel.caret + 1
            } else {
                if sel.caret == 0 {
                    continue;
                }
                sel.caret - 1..sel.caret
            };
            ranges.push((range, i == main));
        }
        if ranges.is_empty() {
            return;
        }
        ranges.sort_by_key(|(r, _)| r.start);
        // Two carets on adjacent characters can name the same character; the
        // second deletion would then take text the first one already took.
        ranges.dedup_by(|a, b| a.0.start < b.0.end);
        let main = ranges.iter().position(|(_, p)| *p).unwrap_or(0);

        let span = ranges[0].0.start..ranges[ranges.len() - 1].0.end;
        let removed = self.doc.slice(span.clone());
        for (range, _) in ranges.iter().rev() {
            self.doc.remove(range.clone());
        }

        // Each caret lands at the start of the range it deleted, shifted by the
        // net change of every deletion below it.
        let mut carets = Vec::with_capacity(ranges.len());
        let mut delta = 0isize;
        for (range, _) in &ranges {
            carets.push((range.start as isize + delta) as usize);
            delta -= (range.end - range.start) as isize;
        }
        let new_len = (span.end - span.start) as isize + delta;
        let inserted = self.doc.slice(span.start..span.start + new_len as usize);

        self.undo.break_group();
        self.record(Edit {
            range: span,
            removed,
            inserted,
            caret_before: primary.caret,
            caret_after: carets[main],
        });
        self.selections.set_carets(carets, main);
        self.desired_col = None;
    }

    /// Removes `range` and records it for undo.
    fn delete_range(&mut self, range: Range<usize>) {
        let removed = self.doc.slice(range.clone());
        let caret_before = self.selections.primary().caret;
        self.doc.remove(range.clone());
        self.record(Edit {
            range: range.clone(),
            removed,
            inserted: String::new(),
            caret_before,
            caret_after: range.start,
        });
        self.selections.set_caret(range.start);
        self.desired_col = None;
    }

    /// Records `edit` with the context the change-history panel needs.
    ///
    /// The line is read from the document *after* the edit, which is correct
    /// because an edit begins at `range.start` and nothing before that offset
    /// moved. Reading it here rather than in the panel is what lets the panel
    /// show where an old change happened without keeping old documents around.
    fn record(&mut self, edit: Edit) {
        let at = edit.range.start.min(self.doc.len_chars());
        let line = self.doc.char_to_line(at) + 1;
        self.undo.push_step(Step {
            edit,
            at: now_secs().max(0) as u64,
            line,
        });
    }

    /// The undo steps, oldest first, for the change-history panel.
    pub fn history(&self) -> &[Step] {
        self.undo.steps()
    }

    /// Drops the whole undo history.
    ///
    /// The document is left as it is: this is "forget how we got here", not
    /// "revert". Notepad++'s Clear Change History means the same thing, and a
    /// command that silently reverted the file would be a trap.
    pub fn clear_history(&mut self) {
        self.undo.clear();
    }

    /// The steps that can be redone, newest first.
    pub fn redo_history(&self) -> &[Step] {
        self.undo.redo_steps()
    }

    /// Reverts the document to the state after `keep` undo steps.
    ///
    /// Returns how many steps were reverted. The reverted steps land on the redo
    /// stack, so the revert is itself undoable and Ctrl+Y walks forward again.
    pub fn revert_to(&mut self, keep: usize) -> usize {
        let steps = self.undo.truncate_to(keep);
        for step in &steps {
            self.replace_doc(step.edit.undo_range(), &step.edit.removed);
        }
        if let Some(last) = steps.last() {
            self.selections.set_caret(last.edit.caret_before);
        }
        self.desired_col = None;
        steps.len()
    }

    /// Reverts the last edit. Returns whether anything happened.
    pub fn undo(&mut self) -> bool {
        let Some(edit) = self.undo.undo() else {
            return false;
        };
        self.replace_doc(edit.undo_range(), &edit.removed);
        self.selections.set_caret(edit.caret_before);
        self.desired_col = None;
        true
    }

    /// Reapplies the last undone edit. Returns whether anything happened.
    pub fn redo(&mut self) -> bool {
        let Some(edit) = self.undo.redo() else {
            return false;
        };
        self.replace_doc(edit.redo_range(), &edit.inserted);
        self.selections.set_caret(edit.caret_after);
        self.desired_col = None;
        true
    }

    /// Ends the current coalescing group, so the next edit starts a new step.
    ///
    /// Typing coalesces into one undo step; anything that is not typing calls
    /// this so the next keystroke starts fresh. Exposed because a caller that
    /// drives the editor programmatically — a macro replay, a test — needs the
    /// same control over step boundaries that the key handler has.
    pub fn break_undo_group(&mut self) {
        self.undo.break_group();
    }

    /// Replaces the text wholesale, keeping the caret and dropping the history.
    ///
    /// The second view on a cloned document calls this: it is the moment the
    /// text it was showing is no longer its own. The caret is clamped to the new
    /// text rather than moved to the top, because a clone is opened to watch a
    /// second place in a file and a caret that jumped home on every keystroke in
    /// the other view would take that place away.
    ///
    /// The twin's styling and fold levels come along, so the clone keeps the
    /// colours instead of re-lexing the file on each keystroke. See
    /// [`Document::mirror_from`]. The dirty flag is taken from the twin rather
    /// than derived: a view that decided for itself would show a pencil the
    /// other one does not.
    pub fn adopt(&mut self, source: &Document) {
        let caret = self.caret();
        self.doc.mirror_from(source);
        self.forget_history();
        self.set_caret(caret);
        // After `set_caret`, which breaks the undo group but does not touch the
        // flag; `mirror_from` marks the document changed, so the flag goes on
        // last.
        self.doc.set_dirty(source.is_dirty());
    }

    /// Discards the undo and redo history, keeping the text.
    ///
    /// The second view on a cloned document needs this. Its text is replaced
    /// wholesale by the twin's, so every range its history holds describes a
    /// document that no longer exists; replaying one would cut at an offset that
    /// may now be past the end. Forgetting is the only safe state, and it costs
    /// the user nothing they were promised: the clone's history was about text
    /// the clone no longer holds.
    pub fn forget_history(&mut self) {
        self.undo.clear();
        self.desired_col = None;
    }

    /// Whether there is anything to undo.
    pub fn can_undo(&self) -> bool {
        self.undo.can_undo()
    }

    /// Whether there is anything to redo.
    pub fn can_redo(&self) -> bool {
        self.undo.can_redo()
    }

    // ---- scrolling ------------------------------------------------------

    /// Scrolls by `dy` pixels, positive meaning down.
    pub fn scroll_pixels(&mut self, dy: f32) {
        let total = self.doc.visible_line_count();
        self.viewport.scroll_pixels(dy, total);
    }

    /// Scrolls by `delta` lines, positive meaning down.
    pub fn scroll_lines(&mut self, delta: i64) {
        let total = self.doc.visible_line_count();
        self.viewport.scroll_lines(delta, total);
    }

    /// Scrolls horizontally by `dx` pixels.
    pub fn scroll_x(&mut self, dx: f32) {
        let widest = self.widest_visible_line() as f32 * self.viewport.char_width;
        self.viewport.scroll_x(dx, widest);
    }

    /// The longest visible line, in characters.
    fn widest_visible_line(&mut self) -> usize {
        if self.doc.wrap_cols() != 0 {
            return 0;
        }
        let total = self.doc.visible_line_count();
        let range = self.viewport.visible_lines(total);
        range
            .map(|row| {
                let line = self.doc.line_of_row(row);
                self.doc.line_len(line)
            })
            .max()
            .unwrap_or(0)
    }

    /// Scrolls the minimum amount needed to show the caret.
    pub fn ensure_caret_visible(&mut self) {
        let caret = self.caret();
        let line = self.doc.char_to_line(caret);
        // Moving into a collapsed block expands it, which is what Notepad++ does
        // and what keeps the caret from disappearing.
        self.doc.reveal_line(line);
        let first = self.doc.row_of_line(line).unwrap_or(0);
        let column = caret - self.doc.line_to_char(line);
        let wrap = self.doc.wrap_cols();
        // A wrapped line occupies several rows, so the caret's row is its first
        // row plus the segment its column falls in.
        // `checked_div` is the honest spelling: with no wrapping there is no
        // segment to divide by, and the row is the line's first.
        let row = first + column.checked_div(wrap).unwrap_or(0);
        let total = self.doc.visible_line_count();
        self.viewport.ensure_line_visible(row, total);
        if wrap == 0 {
            let x = column as f32 * self.viewport.char_width;
            let content = self.doc.line_len(line) as f32 * self.viewport.char_width;
            self.viewport.ensure_x_visible(x, content);
        } else {
            // Wrapped text never scrolls sideways: every row fits the window.
            self.viewport.scroll_x = 0.0;
        }
    }

    /// Remaps carets across an edit that happened outside the editor's control.
    pub fn remap_selections(&mut self, range: &Range<usize>, new_len: usize) {
        self.selections.map(|pos| map_position(pos, range, new_len));
    }

    // ---- styled marks ----------------------------------------------------

    /// The marks painted in `style`.
    pub fn marks(&self, style: usize) -> &[Range<usize>] {
        self.marks.get(style).map(Vec::as_slice).unwrap_or(&[])
    }

    /// How many marks every style holds, for a status line.
    pub fn mark_counts(&self) -> [usize; MARK_STYLES] {
        let mut out = [0usize; MARK_STYLES];
        for (slot, marks) in out.iter_mut().zip(&self.marks) {
            *slot = marks.len();
        }
        out
    }

    /// Paints every occurrence of the current word or selection in `style`.
    ///
    /// Returns how many marks there now are. The needle is the same one
    /// multi-select uses, so "style all occurrences" and "select all occurrences"
    /// always agree about what an occurrence is.
    pub fn style_all_occurrences(&mut self, style: usize, match_case: bool, whole_word: bool) -> usize {
        let Some(needle) = self.multi_select_needle() else {
            return self.marks(style).len();
        };
        let found = self.find_all_matching(&needle, match_case, whole_word, MAX_MULTI_SELECT);
        if let Some(slot) = self.marks.get_mut(style) {
            *slot = found;
        }
        self.marks(style).len()
    }

    /// Paints just the occurrence the caret is on, or the selection.
    ///
    /// Returns false when there is nothing to paint, which is the caller's cue to
    /// say so rather than to leave the user wondering.
    pub fn style_one_token(&mut self, style: usize) -> bool {
        let Some(needle) = self.multi_select_needle() else {
            return false;
        };
        let primary = self.selections.primary();
        let range = if primary.is_empty() {
            match self.highlight_word() {
                Some(range) => range,
                None => return false,
            }
        } else {
            primary.range()
        };
        if self.doc.slice(range.clone()) != needle {
            return false;
        }
        let Some(slot) = self.marks.get_mut(style) else {
            return false;
        };
        if slot.contains(&range) {
            return true;
        }
        slot.push(range);
        slot.sort_by_key(|mark| mark.start);
        true
    }

    /// Drops every mark in `style`.
    pub fn clear_style(&mut self, style: usize) {
        if let Some(slot) = self.marks.get_mut(style) {
            slot.clear();
        }
    }

    /// Drops every mark in every style.
    pub fn clear_all_styles(&mut self) {
        for slot in &mut self.marks {
            slot.clear();
        }
    }

    /// Moves the caret to the next or previous mark in `style`.
    ///
    /// Wraps at both ends, because a jump that refused to wrap would leave the
    /// user pressing the key and getting nothing at the last mark.
    pub fn jump_mark(&mut self, style: usize, forward: bool) -> bool {
        let marks = self.marks(style);
        if marks.is_empty() {
            return false;
        }
        let caret = self.caret();
        let target = if forward {
            marks
                .iter()
                .find(|mark| mark.start > caret)
                .or_else(|| marks.first())
        } else {
            marks
                .iter()
                .rev()
                .find(|mark| mark.end < caret)
                .or_else(|| marks.last())
        };
        let Some(target) = target else {
            return false;
        };
        let start = target.start;
        self.select_range(start..target.end);
        self.ensure_caret_visible();
        true
    }

    /// The text of every mark in `style`, one per line.
    ///
    /// One per line rather than concatenated: the marks are hits, and a list of
    /// hits pasted into a document is only readable if each one starts on its own
    /// line.
    pub fn styled_text(&self, style: usize) -> String {
        self.marks(style)
            .iter()
            .map(|mark| self.doc.slice(mark.clone()))
            .collect::<Vec<String>>()
            .join("\n")
    }

    /// Replaces `range` with `text`, keeping the marks in step.
    ///
    /// Every document mutation goes through here rather than calling
    /// `doc.replace` directly. The marks are character offsets, so an edit below
    /// one would otherwise leave it pointing at the wrong text — and an edit
    /// inside one would leave it pointing at text that no longer exists.
    fn replace_doc(&mut self, range: Range<usize>, text: &str) {
        let new_len = text.chars().count();
        self.doc.replace(range.clone(), text);
        for marks in &mut self.marks {
            for mark in marks.iter_mut() {
                let start = map_mark(mark.start, &range, new_len);
                let end = map_mark(mark.end, &range, new_len);
                *mark = start..end.max(start);
            }
            // A mark the edit swallowed whole is no longer a mark.
            marks.retain(|mark| !mark.is_empty());
        }
    }
}

/// Maps a mark's offset across an edit.
///
/// Differs from [`map_position`] in one case: a position exactly at the start of
/// the edit moves with it. A caret at that point should stay put — typing there
/// pushes the caret forward explicitly — but a mark covers text, and text that
/// was at the insertion point has been pushed along.
fn map_mark(pos: usize, range: &Range<usize>, new_len: usize) -> usize {
    if pos < range.start {
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

    fn editor(text: &str) -> Editor {
        Editor::new(Document::from_text(text))
    }

    #[test]
    fn styling_all_occurrences_marks_every_one() {
        let mut ed = editor("foo bar foo baz foo");
        ed.set_caret(1);
        assert_eq!(ed.style_all_occurrences(0, true, true), 3);
        assert_eq!(ed.mark_counts(), [3, 0, 0, 0, 0]);
        assert_eq!(ed.styled_text(0), "foo\nfoo\nfoo");
    }

    #[test]
    fn the_five_styles_are_independent() {
        let mut ed = editor("foo bar foo");
        ed.set_caret(1);
        ed.style_all_occurrences(0, true, true);
        ed.style_all_occurrences(3, true, true);
        assert_eq!(ed.mark_counts(), [2, 0, 0, 2, 0]);
        ed.clear_style(0);
        assert_eq!(ed.mark_counts(), [0, 0, 0, 2, 0]);
        ed.clear_all_styles();
        assert_eq!(ed.mark_counts(), [0, 0, 0, 0, 0]);
    }

    #[test]
    fn styling_one_token_marks_only_the_caret_word() {
        let mut ed = editor("foo bar foo");
        ed.set_caret(1);
        assert!(ed.style_one_token(1));
        assert_eq!(ed.marks(1).len(), 1);
        assert_eq!(ed.marks(1)[0], 0..3);
        // The same token twice does not double up.
        assert!(ed.style_one_token(1));
        assert_eq!(ed.marks(1).len(), 1);
    }

    #[test]
    fn jumping_walks_the_marks_and_wraps() {
        let mut ed = editor("foo bar foo baz foo");
        ed.set_caret(0);
        ed.style_all_occurrences(0, true, true);
        // The marks are 0..3, 8..11 and 16..19, and a jump selects one, so the
        // caret lands at its end.
        assert!(ed.jump_mark(0, true));
        assert_eq!(ed.caret(), 11);
        assert!(ed.jump_mark(0, true));
        assert_eq!(ed.caret(), 19);
        // Past the last mark it wraps to the first.
        assert!(ed.jump_mark(0, true));
        assert_eq!(ed.caret(), 3);
        // And backwards from the first wraps to the last.
        assert!(ed.jump_mark(0, false));
        assert_eq!(ed.caret(), 19);
    }

    #[test]
    fn jumping_with_no_marks_does_nothing() {
        let mut ed = editor("foo");
        assert!(!ed.jump_mark(2, true));
    }

    #[test]
    fn an_edit_below_a_mark_moves_it() {
        let mut ed = editor("foo bar foo");
        ed.set_caret(1);
        ed.style_all_occurrences(0, true, true);
        // Insert at the very start: both marks shift by four.
        ed.set_caret(0);
        ed.insert("xxxx");
        assert_eq!(ed.marks(0), [4..7, 12..15]);
    }

    #[test]
    fn an_edit_inside_a_mark_drops_it() {
        let mut ed = editor("foo bar foo");
        ed.set_caret(1);
        ed.style_all_occurrences(0, true, true);
        // Overwrite the first `foo` entirely; that mark has nothing left to point
        // at, so it goes rather than collapsing to an empty range.
        ed.select_range(0..3);
        ed.insert("quux");
        // "quux bar foo": the surviving `foo` moved from 8..11 to 9..12.
        assert_eq!(ed.marks(0).to_vec(), vec![9..12]);
    }

    #[test]
    fn undo_puts_the_marks_back_in_step() {
        let mut ed = editor("foo bar foo");
        ed.set_caret(1);
        ed.style_all_occurrences(0, true, true);
        ed.set_caret(0);
        ed.insert("xxxx");
        assert_eq!(ed.marks(0), [4..7, 12..15]);
        ed.undo();
        assert_eq!(ed.marks(0), [0..3, 8..11]);
    }

    #[test]
    fn multi_select_all_takes_every_occurrence_of_the_word() {
        let mut ed = editor("foo bar foo baz foo");
        ed.set_caret(1);
        assert_eq!(ed.select_all_occurrences(true, true), 3);
        assert_eq!(ed.caret_count(), 3);
    }

    #[test]
    fn multi_select_all_needs_more_than_one_occurrence() {
        let mut ed = editor("foo bar baz");
        ed.set_caret(1);
        assert_eq!(ed.select_all_occurrences(true, true), 1);
        assert_eq!(ed.caret_count(), 1);
    }

    #[test]
    fn multi_select_all_can_ignore_case() {
        let mut loose = editor("Foo foo FOO");
        loose.set_caret(1);
        assert_eq!(loose.select_all_occurrences(false, true), 3);

        // The same text with case sensitivity on: `Foo` appears once, so the
        // command leaves the single caret where it was.
        let mut exact = editor("Foo foo FOO");
        exact.set_caret(1);
        assert_eq!(exact.select_all_occurrences(true, true), 1);
    }

    #[test]
    fn multi_select_all_can_ignore_word_boundaries() {
        let mut ed = editor("foo foobar foo");
        ed.set_caret(1);
        // Whole word finds two; without the boundary it also finds the prefix.
        assert_eq!(ed.select_all_occurrences(true, true), 2);
        assert_eq!(ed.select_all_occurrences(true, false), 3);
    }

    #[test]
    fn multi_select_all_uses_the_selection_when_there_is_one() {
        let mut ed = editor("ab ab ab");
        ed.selections.set_single(Selection::new(0, 2));
        assert_eq!(ed.select_all_occurrences(true, true), 3);
    }

    #[test]
    fn multi_select_next_adds_one_caret_at_a_time() {
        let mut ed = editor("foo foo foo");
        ed.set_caret(1);
        assert!(ed.select_next_occurrence(true, true));
        assert_eq!(ed.caret_count(), 2);
        assert!(ed.select_next_occurrence(true, true));
        assert_eq!(ed.caret_count(), 3);
        // Nothing left after the last one.
        assert!(!ed.select_next_occurrence(true, true));
    }

    #[test]
    fn multi_select_next_with_nothing_to_find_is_refused() {
        let mut ed = editor("foo bar");
        ed.set_caret(1);
        assert!(!ed.select_next_occurrence(true, true));
    }

    #[test]
    fn undo_multi_select_drops_the_last_caret() {
        let mut ed = editor("foo foo foo");
        ed.set_caret(1);
        ed.select_all_occurrences(true, true);
        assert_eq!(ed.caret_count(), 3);
        assert!(ed.undo_multi_select());
        assert_eq!(ed.caret_count(), 2);
        assert!(ed.undo_multi_select());
        assert_eq!(ed.caret_count(), 1);
        assert!(!ed.undo_multi_select());
    }

    #[test]
    fn insert_at_caret() {
        let mut ed = editor("ac");
        ed.set_caret(1);
        ed.insert("b");
        assert_eq!(ed.doc.text(), "abc");
        assert_eq!(ed.caret(), 2);
    }

    #[test]
    fn insert_replaces_selection() {
        let mut ed = editor("hello world");
        ed.selections.set_single(Selection::new(0, 5));
        ed.insert("goodbye");
        assert_eq!(ed.doc.text(), "goodbye world");
        assert_eq!(ed.caret(), 7);
    }

    #[test]
    fn backspace_removes_previous_char() {
        let mut ed = editor("abc");
        ed.set_caret(2);
        ed.backspace();
        assert_eq!(ed.doc.text(), "ac");
        assert_eq!(ed.caret(), 1);
    }

    #[test]
    fn backspace_at_start_is_a_noop() {
        let mut ed = editor("abc");
        ed.backspace();
        assert_eq!(ed.doc.text(), "abc");
        assert_eq!(ed.caret(), 0);
    }

    #[test]
    fn backspace_deletes_selection() {
        let mut ed = editor("abcdef");
        ed.selections.set_single(Selection::new(1, 4));
        ed.backspace();
        assert_eq!(ed.doc.text(), "aef");
        assert_eq!(ed.caret(), 1);
    }

    #[test]
    fn delete_forward_removes_next_char() {
        let mut ed = editor("abc");
        ed.delete_forward();
        assert_eq!(ed.doc.text(), "bc");
        assert_eq!(ed.caret(), 0);
    }

    #[test]
    fn delete_forward_at_end_is_a_noop() {
        let mut ed = editor("abc");
        ed.set_caret(3);
        ed.delete_forward();
        assert_eq!(ed.doc.text(), "abc");
    }

    #[test]
    fn undo_restores_text_and_caret() {
        let mut ed = editor("ac");
        ed.set_caret(1);
        ed.insert("b");
        assert!(ed.undo());
        assert_eq!(ed.doc.text(), "ac");
        assert_eq!(ed.caret(), 1);
    }

    #[test]
    fn adopting_a_twin_keeps_the_caret_and_drops_the_history() {
        let mut ed = editor("alpha\nbeta\n");
        ed.set_caret(6);
        ed.insert("X");
        assert!(ed.can_undo());
        let mut twin = editor("one\ntwo\n");
        twin.doc.set_dirty(true);
        ed.adopt(&twin.doc);
        assert_eq!(ed.doc.text(), "one\ntwo\n");
        assert_eq!(ed.caret(), 7, "the caret stays where the user left it");
        assert!(ed.doc.is_dirty(), "the twin has unsaved edits, so this view does too");
        assert!(!ed.can_undo(), "the old ranges describe text that is gone");
    }

    #[test]
    fn adopting_a_twin_copies_its_styling() {
        let mut ed = editor("plain\n");
        let mut twin = editor("fn main() {}\n");
        twin.doc.set_style_range(0..2, 3);
        ed.adopt(&twin.doc);
        assert_eq!(ed.doc.style_at(0), 3, "the clone is styled without a re-lex");
        assert_eq!(ed.doc.style_at(2), 0);
    }

    #[test]
    fn adopting_a_shorter_text_pulls_the_caret_back_inside() {
        let mut ed = editor("alpha\nbeta\n");
        ed.set_caret(10);
        ed.adopt(&editor("ab").doc);
        assert_eq!(ed.caret(), 2);
        assert!(!ed.doc.is_dirty());
    }

    #[test]
    fn forgetting_the_history_keeps_the_text() {
        let mut ed = editor("ac");
        ed.set_caret(1);
        ed.insert("b");
        assert!(ed.can_undo());
        ed.forget_history();
        assert!(!ed.can_undo());
        assert!(!ed.can_redo());
        assert_eq!(ed.doc.text(), "abc");
        assert!(!ed.undo(), "there is nothing left to undo");
        assert_eq!(ed.doc.text(), "abc");
    }

    #[test]
    fn the_history_records_where_each_change_happened() {
        let mut ed = editor("one\ntwo\nthree\n");
        ed.set_caret(8);
        ed.insert("X");
        let history = ed.history();
        assert_eq!(history.len(), 1);
        // "one\ntwo\n" is eight characters, so offset 8 is the first character
        // of the third line.
        assert_eq!(history[0].line, 3);
        assert!(history[0].at > 0, "a recorded step carries a clock");
        assert_eq!(history[0].describe(), "Insert 1 char");
    }

    #[test]
    fn a_typing_run_is_one_history_entry() {
        let mut ed = editor("");
        for c in "hello".chars() {
            ed.insert(&c.to_string());
        }
        assert_eq!(ed.history().len(), 1);
        assert_eq!(ed.history()[0].describe(), "Insert 5 chars");
    }

    #[test]
    fn reverting_to_a_point_undoes_everything_after_it() {
        let mut ed = editor("");
        ed.insert("a");
        ed.break_undo_group();
        ed.insert("b");
        ed.break_undo_group();
        ed.insert("c");
        assert_eq!(ed.doc.text(), "abc");
        assert_eq!(ed.history().len(), 3);
        // Keep the first step: "b" and "c" are reverted.
        assert_eq!(ed.revert_to(1), 2);
        assert_eq!(ed.doc.text(), "a");
        assert_eq!(ed.history().len(), 1);
    }

    #[test]
    fn a_revert_can_be_walked_forward_again() {
        let mut ed = editor("");
        ed.insert("a");
        ed.break_undo_group();
        ed.insert("b");
        ed.revert_to(1);
        assert_eq!(ed.doc.text(), "a");
        assert_eq!(ed.redo_history().len(), 1);
        assert!(ed.redo());
        assert_eq!(ed.doc.text(), "ab");
    }

    #[test]
    fn reverting_to_the_current_depth_changes_nothing() {
        let mut ed = editor("abc");
        ed.set_caret(3);
        ed.insert("d");
        assert_eq!(ed.revert_to(1), 0);
        assert_eq!(ed.doc.text(), "abcd");
    }

    #[test]
    fn reverting_to_zero_empties_the_document() {
        let mut ed = editor("");
        ed.insert("a");
        ed.break_undo_group();
        ed.insert("b");
        assert_eq!(ed.revert_to(0), 2);
        assert_eq!(ed.doc.text(), "");
        assert!(!ed.can_undo());
    }

    #[test]
    fn redo_reapplies() {
        let mut ed = editor("ac");
        ed.set_caret(1);
        ed.insert("b");
        ed.undo();
        assert!(ed.redo());
        assert_eq!(ed.doc.text(), "abc");
        assert_eq!(ed.caret(), 2);
    }

    #[test]
    fn undo_of_a_delete_restores_the_text() {
        let mut ed = editor("abcdef");
        ed.selections.set_single(Selection::new(1, 4));
        ed.backspace();
        assert_eq!(ed.doc.text(), "aef");
        assert!(ed.undo());
        assert_eq!(ed.doc.text(), "abcdef");
    }

    #[test]
    fn typing_run_undoes_as_one_step() {
        let mut ed = editor("");
        for c in "hello".chars() {
            ed.insert(&c.to_string());
        }
        assert!(ed.undo());
        assert_eq!(ed.doc.text(), "");
    }

    #[test]
    fn move_char_left_and_right() {
        let mut ed = editor("abc");
        ed.move_caret(Motion::CharRight, false);
        assert_eq!(ed.caret(), 1);
        ed.move_caret(Motion::CharRight, false);
        assert_eq!(ed.caret(), 2);
        ed.move_caret(Motion::CharLeft, false);
        assert_eq!(ed.caret(), 1);
    }

    #[test]
    fn move_char_clamps_at_document_ends() {
        let mut ed = editor("ab");
        ed.move_caret(Motion::CharLeft, false);
        assert_eq!(ed.caret(), 0);
        ed.move_caret(Motion::DocEnd, false);
        ed.move_caret(Motion::CharRight, false);
        assert_eq!(ed.caret(), 2);
    }

    #[test]
    fn extend_grows_the_selection() {
        let mut ed = editor("abcdef");
        ed.move_caret(Motion::CharRight, true);
        ed.move_caret(Motion::CharRight, true);
        assert_eq!(ed.selections.primary().range(), 0..2);
        assert_eq!(ed.selected_text(), "ab");
    }

    #[test]
    fn horizontal_move_collapses_selection_to_edge() {
        let mut ed = editor("abcdef");
        ed.selections.set_single(Selection::new(1, 4));
        ed.move_caret(Motion::CharLeft, false);
        assert_eq!(ed.caret(), 1);
        assert!(ed.selections.primary().is_empty());

        ed.selections.set_single(Selection::new(1, 4));
        ed.move_caret(Motion::CharRight, false);
        assert_eq!(ed.caret(), 4);
    }

    #[test]
    fn line_up_and_down_clamp_to_short_lines() {
        let mut ed = editor("long line\nab\nanother long line");
        ed.set_caret(5);
        ed.move_caret(Motion::LineDown, false);
        assert_eq!(ed.caret_line(), 1);
        assert_eq!(ed.caret_column(), 2);
        ed.move_caret(Motion::LineDown, false);
        assert_eq!(ed.caret_line(), 2);
        // The desired column survives the short line.
        assert_eq!(ed.caret_column(), 5);
    }

    #[test]
    fn vertical_moves_remember_the_desired_column() {
        let mut ed = editor("0123456789\nab\n0123456789");
        ed.set_caret(8);
        ed.move_caret(Motion::LineDown, false);
        assert_eq!(ed.caret_column(), 2);
        ed.move_caret(Motion::LineDown, false);
        assert_eq!(ed.caret_column(), 8);
    }

    #[test]
    fn line_up_at_first_line_stays_put() {
        let mut ed = editor("abc\ndef");
        ed.set_caret(1);
        ed.move_caret(Motion::LineUp, false);
        assert_eq!(ed.caret(), 1);
    }

    #[test]
    fn word_movement() {
        let mut ed = editor("foo bar_baz qux");
        ed.move_caret(Motion::WordRight, false);
        assert_eq!(ed.caret(), 4);
        ed.move_caret(Motion::WordRight, false);
        assert_eq!(ed.caret(), 12);
        ed.move_caret(Motion::WordLeft, false);
        assert_eq!(ed.caret(), 4);
        ed.move_caret(Motion::WordLeft, false);
        assert_eq!(ed.caret(), 0);
    }

    #[test]
    fn line_start_and_end() {
        let mut ed = editor("abc\ndefg\nhi");
        ed.set_caret(6);
        ed.move_caret(Motion::LineStart, false);
        assert_eq!(ed.caret(), 4);
        ed.move_caret(Motion::LineEnd, false);
        assert_eq!(ed.caret(), 8);
    }

    #[test]
    fn doc_start_and_end() {
        let mut ed = editor("abc\ndef");
        ed.move_caret(Motion::DocEnd, false);
        assert_eq!(ed.caret(), 7);
        ed.move_caret(Motion::DocStart, false);
        assert_eq!(ed.caret(), 0);
    }

    #[test]
    fn select_all_covers_the_document() {
        let mut ed = editor("abc\ndef");
        ed.select_all();
        assert_eq!(ed.selections.primary().range(), 0..7);
        assert_eq!(ed.selected_text(), "abc\ndef");
    }

    #[test]
    fn ensure_caret_visible_scrolls_down() {
        let text = (0..200)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut ed = editor(&text);
        ed.viewport.set_size(800.0, 400.0);
        ed.viewport.set_metrics(20.0, 8.0);
        ed.move_caret(Motion::DocEnd, false);
        ed.ensure_caret_visible();
        assert!(ed.viewport.first_line > 0);
        let line = ed.caret_line();
        let visible = ed.viewport.visible_lines(ed.doc.len_lines());
        assert!(visible.contains(&line));
    }

    #[test]
    fn caret_from_point_maps_to_a_position() {
        let mut ed = editor("abc\ndef");
        ed.viewport.set_metrics(20.0, 8.0);
        ed.set_caret_from_point(16.0, 25.0, false);
        assert_eq!(ed.caret_line(), 1);
        assert_eq!(ed.caret_column(), 2);
        assert_eq!(ed.caret(), 6);
    }

    #[test]
    fn caret_from_point_clamps_past_line_end() {
        let mut ed = editor("ab\nlonger line");
        ed.viewport.set_metrics(20.0, 8.0);
        ed.set_caret_from_point(500.0, 5.0, false);
        assert_eq!(ed.caret(), 2);
    }

    #[test]
    fn scroll_pixels_moves_the_viewport() {
        let text = (0..100)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut ed = editor(&text);
        ed.viewport.set_metrics(20.0, 8.0);
        ed.scroll_pixels(45.0);
        assert_eq!(ed.viewport.first_line, 2);
        assert_eq!(ed.viewport.top_offset, 5.0);
    }

    #[test]
    fn page_down_moves_by_a_screenful() {
        let text = (0..200)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut ed = editor(&text);
        ed.viewport.set_size(800.0, 400.0);
        ed.viewport.set_metrics(20.0, 8.0);
        let page = ed.viewport.full_line_count();
        ed.move_caret(Motion::PageDown, false);
        assert_eq!(ed.caret_line(), page);
    }

    #[test]
    fn a_block_selects_one_span_per_line() {
        let mut ed = editor("abcd\nefgh\nijkl");
        ed.select_block((0, 1), (2, 3));
        assert_eq!(ed.selections.len(), 3);
        assert_eq!(ed.selected_text(), "bc\nfg\njk");
    }

    #[test]
    fn a_block_clamps_to_short_lines() {
        let mut ed = editor("abcdef\nab\nxyz");
        ed.select_block((0, 2), (2, 5));
        assert_eq!(ed.selected_text(), "cde\nz");
    }

    #[test]
    fn typing_into_a_block_reaches_every_line() {
        let mut ed = editor("abcd\nefgh\nijkl");
        ed.select_block((0, 1), (2, 3));
        ed.insert("X");
        assert_eq!(ed.doc.text(), "aXd\neXh\niXl");
    }

    #[test]
    fn typing_into_a_block_keeps_the_carets() {
        let mut ed = editor("abcd\nefgh\nijkl");
        ed.select_block((0, 1), (2, 3));
        ed.insert("X");
        assert_eq!(ed.selections.len(), 3);
        ed.insert("Y");
        assert_eq!(ed.doc.text(), "aXYd\neXYh\niXYl");
        assert_eq!(ed.selections.len(), 3);
    }

    #[test]
    fn deleting_a_block_empties_every_line() {
        let mut ed = editor("abcd\nefgh\nijkl");
        ed.select_block((0, 1), (2, 3));
        assert!(ed.delete_all_selections());
        assert_eq!(ed.doc.text(), "ad\neh\nil");
    }

    #[test]
    fn deleting_with_nothing_selected_does_nothing() {
        let mut ed = editor("abc");
        assert!(!ed.delete_all_selections());
        assert_eq!(ed.doc.text(), "abc");
    }

    #[test]
    fn a_block_undoes_as_one_step() {
        let mut ed = editor("abcd\nefgh");
        ed.select_block((0, 1), (1, 3));
        ed.insert("X");
        assert_eq!(ed.doc.text(), "aXd\neXh");
        ed.undo();
        assert_eq!(ed.doc.text(), "abcd\nefgh");
    }

    #[test]
    fn goto_position_clamps_to_the_line() {
        let mut ed = editor("ab\ncdef");
        ed.goto_position(1, 99);
        assert_eq!(ed.caret(), 7);
        ed.goto_position(99, 0);
        assert_eq!(ed.caret_line(), 1);
    }

    #[test]
    fn transform_lines_touches_only_the_caret_line() {
        let mut ed = editor("a  \nb  ");
        ed.set_caret(0);
        assert!(ed.transform_lines(crate::lines::trim_trailing));
        assert_eq!(ed.doc.text(), "a\nb  ");
    }

    #[test]
    fn transform_lines_covers_the_whole_selection() {
        let mut ed = editor("c\na\nb");
        ed.selections.set_single(Selection::new(0, 3));
        assert!(ed.transform_lines(|t| crate::lines::sort(t, Default::default())));
        assert_eq!(ed.doc.text(), "a\nc\nb");
    }

    #[test]
    fn transform_lines_ignores_the_line_a_selection_stops_at() {
        let mut ed = editor("c\na\nb\n");
        // Selects "c\na\n" and stops at the start of line 2.
        ed.selections.set_single(Selection::new(0, 4));
        assert!(ed.transform_lines(|t| crate::lines::sort(t, Default::default())));
        assert_eq!(ed.doc.text(), "a\nc\nb\n");
    }

    #[test]
    fn transform_lines_is_one_undo_step() {
        let mut ed = editor("c\na\nb");
        ed.selections.set_single(Selection::new(0, 5));
        ed.transform_lines(|t| crate::lines::sort(t, Default::default()));
        assert_eq!(ed.doc.text(), "a\nb\nc");
        assert!(ed.undo());
        assert_eq!(ed.doc.text(), "c\na\nb");
    }

    #[test]
    fn transform_lines_reports_no_change() {
        let mut ed = editor("a\nb");
        assert!(!ed.transform_lines(|t| crate::lines::sort(t, Default::default())));
        assert!(!ed.can_undo());
    }

    #[test]
    fn duplicate_lines_repeats_the_caret_line() {
        let mut ed = editor("a\nb");
        ed.set_caret(0);
        assert!(ed.duplicate_lines());
        assert_eq!(ed.doc.text(), "a\na\nb");
    }

    #[test]
    fn duplicate_lines_repeats_a_block() {
        let mut ed = editor("a\nb\nc");
        ed.selections.set_single(Selection::new(0, 3));
        assert!(ed.duplicate_lines());
        assert_eq!(ed.doc.text(), "a\nb\na\nb\nc");
    }

    #[test]
    fn move_lines_up_swaps_with_the_line_above() {
        let mut ed = editor("a\nb\nc");
        ed.set_caret(2);
        assert!(ed.move_lines(true));
        assert_eq!(ed.doc.text(), "b\na\nc");
    }

    #[test]
    fn move_lines_down_swaps_with_the_line_below() {
        let mut ed = editor("a\nb\nc");
        ed.set_caret(0);
        assert!(ed.move_lines(false));
        assert_eq!(ed.doc.text(), "b\na\nc");
    }

    #[test]
    fn move_lines_at_the_top_is_a_no_op() {
        let mut ed = editor("a\nb");
        ed.set_caret(0);
        assert!(!ed.move_lines(true));
        assert_eq!(ed.doc.text(), "a\nb");
    }

    #[test]
    fn move_lines_at_the_bottom_is_a_no_op() {
        let mut ed = editor("a\nb");
        ed.set_caret(2);
        assert!(!ed.move_lines(false));
        assert_eq!(ed.doc.text(), "a\nb");
    }

    #[test]
    fn move_lines_keeps_a_missing_final_newline() {
        let mut ed = editor("a\nb");
        ed.set_caret(0);
        assert!(ed.move_lines(false));
        assert_eq!(ed.doc.text(), "b\na");
    }

    #[test]
    fn auto_indent_keeps_the_current_indent() {
        let mut ed = editor("    abc");
        ed.set_caret(7);
        assert_eq!(ed.auto_indent(), "    ");
    }

    #[test]
    fn auto_indent_opens_a_level_after_a_brace() {
        let mut ed = editor("    if x {");
        ed.set_caret(10);
        assert_eq!(ed.auto_indent(), "        ");
    }

    #[test]
    fn auto_indent_ignores_trailing_space_after_a_brace() {
        let mut ed = editor("fn f() {   ");
        ed.set_caret(11);
        assert_eq!(ed.auto_indent(), "    ");
    }

    #[test]
    fn auto_indent_of_an_unindented_line_is_empty() {
        let mut ed = editor("abc");
        ed.set_caret(3);
        assert_eq!(ed.auto_indent(), "");
    }

    #[test]
    fn auto_indent_uses_the_line_start_not_the_caret_column() {
        let mut ed = editor("  abc");
        ed.set_caret(4);
        assert_eq!(ed.auto_indent(), "  ");
    }

    #[test]
    fn matching_brace_finds_the_partner_after_the_caret() {
        let mut ed = editor("f(a)");
        ed.set_caret(2);
        assert_eq!(ed.matching_brace(), Some((1, 3)));
    }

    #[test]
    fn matching_brace_finds_the_partner_before_the_caret() {
        let mut ed = editor("f(a)");
        ed.set_caret(4);
        assert_eq!(ed.matching_brace(), Some((1, 3)));
    }

    #[test]
    fn matching_brace_counts_nesting() {
        let mut ed = editor("{a{b}c}");
        ed.set_caret(0);
        assert_eq!(ed.matching_brace(), Some((0, 6)));
    }

    #[test]
    fn matching_brace_handles_each_bracket_kind() {
        let mut ed = editor("[x]");
        ed.set_caret(0);
        assert_eq!(ed.matching_brace(), Some((0, 2)));
        let mut ed = editor("(x)");
        ed.set_caret(0);
        assert_eq!(ed.matching_brace(), Some((0, 2)));
    }

    #[test]
    fn matching_brace_is_none_away_from_a_bracket() {
        let mut ed = editor("abc");
        ed.set_caret(1);
        assert_eq!(ed.matching_brace(), None);
    }

    #[test]
    fn matching_brace_is_none_when_unbalanced() {
        let mut ed = editor("(abc");
        ed.set_caret(0);
        assert_eq!(ed.matching_brace(), None);
    }

    #[test]
    fn matching_brace_skips_a_mismatched_kind() {
        let mut ed = editor("(a]");
        ed.set_caret(0);
        assert_eq!(ed.matching_brace(), None);
    }

    #[test]
    fn selected_lines_of_a_caret_is_its_line() {
        let ed = editor("a\nb\nc");
        assert_eq!(ed.selected_lines(), (0, 0));
    }

    #[test]
    fn highlight_word_covers_the_word_under_the_caret() {
        let mut ed = editor("let total = 1;");
        ed.set_caret(6);
        assert_eq!(ed.highlight_word(), Some(4..9));
    }

    #[test]
    fn highlight_word_at_the_end_of_a_word_takes_that_word() {
        let mut ed = editor("let total = 1;");
        ed.set_caret(9);
        assert_eq!(ed.highlight_word(), Some(4..9));
    }

    #[test]
    fn highlight_word_at_the_start_of_a_word_takes_that_word() {
        let mut ed = editor("let total = 1;");
        ed.set_caret(4);
        assert_eq!(ed.highlight_word(), Some(4..9));
    }

    #[test]
    fn highlight_word_prefers_the_selection() {
        let mut ed = editor("let total = 1;");
        ed.selections.set_single(Selection::new(0, 3));
        assert_eq!(ed.highlight_word(), Some(0..3));
    }

    #[test]
    fn highlight_word_is_none_between_words() {
        let mut ed = editor("a  b");
        ed.set_caret(2);
        assert_eq!(ed.highlight_word(), None);
    }

    #[test]
    fn occurrences_finds_whole_words_only() {
        let ed = editor("cat concat cat scatter cat");
        assert_eq!(ed.occurrences("cat", 10), vec![0..3, 11..14, 23..26]);
    }

    #[test]
    fn occurrences_respects_the_limit() {
        let ed = editor("a a a a a");
        assert_eq!(ed.occurrences("a", 2), vec![0..1, 2..3]);
    }

    #[test]
    fn occurrences_of_an_empty_word_is_empty() {
        let ed = editor("abc");
        assert!(ed.occurrences("", 10).is_empty());
    }

    #[test]
    fn occurrences_returns_char_offsets_for_multibyte_text() {
        let ed = editor("éé x éé");
        assert_eq!(ed.occurrences("éé", 10), vec![0..2, 5..7]);
    }

    #[test]
    fn typed_opens_a_pair_for_a_bracket() {
        let mut ed = editor("");
        ed.set_caret(0);
        assert_eq!(ed.typed('('), Typed::Pair(')'));
    }

    #[test]
    fn typed_skips_a_closer_that_is_already_there() {
        let mut ed = editor("()");
        ed.set_caret(1);
        assert_eq!(ed.typed(')'), Typed::Skip);
    }

    #[test]
    fn typed_does_not_pair_a_bracket_before_a_word() {
        let mut ed = editor("abc");
        ed.set_caret(0);
        assert_eq!(ed.typed('('), Typed::Plain);
    }

    #[test]
    fn typed_does_not_pair_a_quote_inside_a_word() {
        let mut ed = editor("dont");
        ed.set_caret(2);
        assert_eq!(ed.typed('\''), Typed::Plain);
    }

    #[test]
    fn typed_steps_over_a_quote_that_is_already_there() {
        let mut ed = editor("don't");
        ed.set_caret(3);
        assert_eq!(ed.typed('\''), Typed::Skip);
    }

    #[test]
    fn typed_pairs_a_quote_at_a_word_boundary() {
        let mut ed = editor("x ");
        ed.set_caret(2);
        assert_eq!(ed.typed('"'), Typed::Pair('"'));
    }

    #[test]
    fn typed_wraps_a_selection_in_a_pair() {
        let mut ed = editor("abc");
        ed.selections.set_single(Selection::new(0, 3));
        assert_eq!(ed.typed('('), Typed::Pair(')'));
    }

    #[test]
    fn typed_is_plain_for_an_ordinary_letter() {
        let mut ed = editor("");
        ed.set_caret(0);
        assert_eq!(ed.typed('a'), Typed::Plain);
    }

    #[test]
    fn delete_lines_removes_the_caret_line() {
        let mut ed = editor("a\nb\nc");
        ed.set_caret(2);
        assert!(ed.delete_lines());
        assert_eq!(ed.doc.text(), "a\nc");
    }

    #[test]
    fn delete_lines_takes_the_whole_selection() {
        let mut ed = editor("a\nb\nc\nd");
        ed.select_range(0..4);
        assert!(ed.delete_lines());
        assert_eq!(ed.doc.text(), "c\nd");
    }

    #[test]
    fn delete_lines_on_the_last_line_keeps_the_document_whole() {
        let mut ed = editor("a\nb");
        ed.set_caret(2);
        assert!(ed.delete_lines());
        assert_eq!(ed.doc.text(), "a");
    }

    #[test]
    fn find_all_ignores_word_boundaries() {
        let ed = editor("foo( bar( baz");
        assert_eq!(ed.find_all("(", 10).len(), 2);
    }

    #[test]
    fn find_all_stops_at_the_limit() {
        let ed = editor("aaaa");
        assert_eq!(ed.find_all("a", 3).len(), 3);
    }

    #[test]
    fn find_all_of_nothing_is_empty() {
        let ed = editor("abc");
        assert!(ed.find_all("", 10).is_empty());
    }

    #[test]
    fn select_line_covers_the_line_without_its_terminator() {
        let mut ed = editor("ab\ncd");
        ed.set_caret(4);
        ed.select_line();
        assert_eq!(ed.selected_text(), "cd");
    }

    #[test]
    fn a_double_click_takes_the_word_under_the_pointer() {
        let mut ed = editor("foo bar");
        ed.select_word_at(6);
        assert_eq!(ed.selected_text(), "bar");
        assert_eq!(ed.word_range_at(6), Some(4..7));
    }

    #[test]
    fn a_double_click_on_punctuation_takes_only_the_punctuation() {
        let mut ed = editor("foo.bar");
        ed.select_word_at(3);
        assert_eq!(ed.selected_text(), ".");
    }

    #[test]
    fn a_double_click_on_whitespace_takes_the_run() {
        let mut ed = editor("a   b");
        ed.select_word_at(2);
        assert_eq!(ed.selected_text(), "   ");
    }

    #[test]
    fn a_whitespace_run_stops_at_the_line_ending() {
        let ed = editor("a \nb");
        assert_eq!(ed.word_range_at(1), Some(1..2));
    }

    #[test]
    fn a_double_click_past_the_end_takes_the_last_word_of_the_line() {
        let ed = editor("foo\nbar");
        // On the terminator, and one past the last character, are both a press
        // on the text the pointer is next to.
        assert_eq!(ed.word_range_at(3), Some(0..3));
        assert_eq!(ed.word_range_at(7), Some(4..7));
    }

    #[test]
    fn a_double_click_on_an_empty_line_selects_nothing() {
        let mut ed = editor("a\n\nb");
        assert_eq!(ed.word_range_at(2), None);
        ed.set_caret(3);
        ed.select_word_at(2);
        // The selection it already had is left alone rather than cleared.
        assert_eq!(ed.selected_text(), "");
        assert_eq!(ed.caret(), 3);
    }

    #[test]
    fn a_double_click_reads_non_ascii_words_whole() {
        let mut ed = editor("héllo wörld");
        ed.select_word_at(1);
        assert_eq!(ed.selected_text(), "héllo");
    }

    #[test]
    fn a_triple_click_takes_the_line_and_its_terminator() {
        let mut ed = editor("ab\ncd\nef");
        ed.select_line_at(4);
        assert_eq!(ed.selected_text(), "cd\n");
    }

    #[test]
    fn a_triple_click_on_the_last_line_takes_what_there_is() {
        let mut ed = editor("ab\ncd");
        ed.select_line_at(4);
        assert_eq!(ed.selected_text(), "cd");
    }

    #[test]
    fn a_triple_click_takes_both_halves_of_a_crlf_terminator() {
        let mut ed = editor("ab\r\ncd");
        ed.select_line_at(0);
        assert_eq!(ed.selected_text(), "ab\r\n");
    }

    #[test]
    fn overwrite_is_off_until_it_is_asked_for() {
        let mut ed = editor("abc");
        assert!(!ed.overwrite());
        assert!(ed.toggle_overwrite());
        assert!(ed.overwrite());
        assert!(!ed.toggle_overwrite());
    }

    #[test]
    fn overwrite_replaces_the_character_under_the_caret() {
        let mut ed = editor("abc");
        ed.set_caret(1);
        ed.overwrite_char('X');
        assert_eq!(ed.doc.text(), "aXc");
        assert_eq!(ed.caret(), 2);
    }

    #[test]
    fn overwrite_at_the_end_of_a_line_appends() {
        let mut ed = editor("ab");
        ed.set_caret(2);
        ed.overwrite_char('c');
        assert_eq!(ed.doc.text(), "abc");
        assert_eq!(ed.caret(), 3);
    }

    #[test]
    fn overwrite_never_eats_a_newline() {
        let mut ed = editor("ab\ncd");
        ed.set_caret(2);
        ed.overwrite_char('X');
        assert_eq!(ed.doc.text(), "abX\ncd");
    }

    #[test]
    fn overwrite_replaces_a_selection_rather_than_one_character() {
        let mut ed = editor("abcdef");
        ed.select_range(1..4);
        ed.overwrite_char('X');
        assert_eq!(ed.doc.text(), "aXef");
    }

    #[test]
    fn a_run_of_overwrites_undoes_as_one_step() {
        let mut ed = editor("abcde");
        ed.set_caret(0);
        ed.overwrite_char('X');
        ed.overwrite_char('Y');
        ed.overwrite_char('Z');
        assert_eq!(ed.doc.text(), "XYZde");
        assert!(ed.undo());
        assert_eq!(ed.doc.text(), "abcde");
    }

    #[test]
    fn redoing_a_run_of_overwrites_restores_all_of_it() {
        let mut ed = editor("abcde");
        ed.set_caret(0);
        ed.overwrite_char('X');
        ed.overwrite_char('Y');
        ed.undo();
        assert!(ed.redo());
        assert_eq!(ed.doc.text(), "XYcde");
    }

    #[test]
    fn overwrite_does_not_join_two_lines() {
        let mut ed = editor("a\nb");
        ed.set_caret(1);
        ed.overwrite_char('X');
        assert_eq!(ed.doc.text(), "aX\nb");
    }

    // ---- multiple carets ------------------------------------------------

    /// Every caret, in document order.
    fn carets(ed: &Editor) -> Vec<usize> {
        ed.selections.iter().map(|s| s.caret).collect()
    }

    #[test]
    fn add_caret_below_puts_a_second_caret_on_the_next_line() {
        let mut ed = editor("aa\nbb\ncc");
        assert!(ed.add_caret_below());
        assert_eq!(carets(&ed), vec![0, 3]);
        assert_eq!(ed.caret_count(), 2);
    }

    #[test]
    fn add_caret_below_keeps_the_column() {
        let mut ed = editor("abc\nx\nabc");
        ed.set_caret(2);
        assert!(ed.add_caret_below());
        // Column 2 does not exist on the one-character line, so it clamps.
        assert_eq!(carets(&ed), vec![2, 5]);
    }

    #[test]
    fn add_caret_below_stops_at_the_last_line() {
        let mut ed = editor("aa\nbb");
        ed.set_caret(3);
        assert!(!ed.add_caret_below());
        assert_eq!(ed.caret_count(), 1);
    }

    #[test]
    fn add_caret_above_stops_at_the_first_line() {
        let mut ed = editor("aa\nbb");
        assert!(!ed.add_caret_above());
        assert_eq!(ed.caret_count(), 1);
    }

    #[test]
    fn add_caret_above_grows_from_the_topmost_caret() {
        let mut ed = editor("aa\nbb\ncc");
        ed.set_caret(6);
        assert!(ed.add_caret_above());
        assert!(ed.add_caret_above());
        assert_eq!(carets(&ed), vec![0, 3, 6]);
    }

    #[test]
    fn add_caret_at_makes_the_new_caret_primary() {
        let mut ed = editor("hello");
        ed.add_caret_at(4);
        assert_eq!(ed.caret_count(), 2);
        assert_eq!(ed.caret(), 4);
    }

    #[test]
    fn add_caret_at_the_same_place_does_not_duplicate_it() {
        let mut ed = editor("hello");
        ed.add_caret_at(0);
        assert_eq!(ed.caret_count(), 1);
    }

    #[test]
    fn clear_extra_carets_keeps_the_primary_one() {
        let mut ed = editor("aa\nbb");
        ed.selections.set_carets(vec![0, 3], 1);
        assert!(ed.clear_extra_carets());
        assert_eq!(ed.caret_count(), 1);
        assert_eq!(ed.caret(), 3);
    }

    #[test]
    fn clearing_a_single_caret_reports_nothing_to_do() {
        let mut ed = editor("aa");
        assert!(!ed.clear_extra_carets());
    }

    #[test]
    fn typing_lands_on_every_caret() {
        let mut ed = editor("aa\nbb");
        ed.selections.set_carets(vec![0, 3], 0);
        ed.insert("X");
        assert_eq!(ed.doc.text(), "Xaa\nXbb");
        assert_eq!(carets(&ed), vec![1, 5]);
        assert_eq!(ed.caret(), 1);
    }

    #[test]
    fn a_multi_caret_insert_undoes_as_one_step() {
        let mut ed = editor("aa\nbb");
        ed.selections.set_carets(vec![0, 3], 0);
        ed.insert("X");
        assert!(ed.undo());
        assert_eq!(ed.doc.text(), "aa\nbb");
    }

    #[test]
    fn backspace_deletes_at_every_caret() {
        let mut ed = editor("aa\nbb");
        ed.selections.set_carets(vec![2, 5], 0);
        ed.backspace();
        assert_eq!(ed.doc.text(), "a\nb");
        assert_eq!(carets(&ed), vec![1, 3]);
    }

    #[test]
    fn delete_forward_deletes_at_every_caret() {
        let mut ed = editor("aa\nbb");
        ed.selections.set_carets(vec![0, 3], 0);
        ed.delete_forward();
        assert_eq!(ed.doc.text(), "a\nb");
        assert_eq!(carets(&ed), vec![0, 2]);
    }

    #[test]
    fn backspace_skips_a_caret_with_nothing_before_it() {
        let mut ed = editor("aa\nbb");
        ed.selections.set_carets(vec![0, 5], 0);
        ed.backspace();
        assert_eq!(ed.doc.text(), "aa\nb");
        assert_eq!(carets(&ed), vec![4]);
    }

    #[test]
    fn a_multi_caret_backspace_undoes_as_one_step() {
        let mut ed = editor("aa\nbb");
        ed.selections.set_carets(vec![2, 5], 0);
        ed.backspace();
        assert!(ed.undo());
        assert_eq!(ed.doc.text(), "aa\nbb");
    }

    #[test]
    fn arrows_move_every_caret() {
        let mut ed = editor("aa\nbb\ncc");
        ed.selections.set_carets(vec![0, 3], 0);
        ed.move_caret(Motion::LineDown, false);
        assert_eq!(carets(&ed), vec![3, 6]);
    }

    #[test]
    fn shift_arrow_extends_every_caret() {
        let mut ed = editor("aa\nbb");
        ed.selections.set_carets(vec![0, 3], 0);
        ed.move_caret(Motion::CharRight, true);
        let ranges: Vec<_> = ed.selections.iter().map(|s| s.range()).collect();
        assert_eq!(ranges, vec![0..1, 3..4]);
    }

    #[test]
    fn a_horizontal_move_collapses_every_selection_to_its_edge() {
        let mut ed = editor("aa\nbb");
        ed.selections
            .set_selections(vec![Selection::new(0, 2), Selection::new(3, 5)], 0);
        ed.move_caret(Motion::CharLeft, false);
        assert_eq!(carets(&ed), vec![0, 3]);
    }

    #[test]
    fn moving_every_caret_to_the_document_end_merges_them() {
        let mut ed = editor("aa\nbb");
        ed.selections.set_carets(vec![0, 3], 0);
        ed.move_caret(Motion::DocEnd, false);
        assert_eq!(ed.caret_count(), 1);
        assert_eq!(ed.caret(), 5);
    }
}
