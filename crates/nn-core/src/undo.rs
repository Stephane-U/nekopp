//! Undo and redo.
//!
//! Every edit is recorded as the range it touched, the text it removed, and the
//! text it inserted. Undo replays the inverse; redo replays the original. Because
//! the record is a plain range plus two strings, it survives any later change to
//! the document — the range is always interpreted against the state the edit
//! produced.
//!
//! Consecutive keystrokes coalesce into one undo step, the way every editor does
//! it. Anything that is not typing calls `break_group` so the next keystroke
//! starts a fresh step.

use std::ops::Range;

/// One reversible change to a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// The range that was replaced, in the document as it was before the edit.
    pub range: Range<usize>,
    /// The text that was there before.
    pub removed: String,
    /// The text that replaced it.
    pub inserted: String,
    /// Caret position before the edit.
    pub caret_before: usize,
    /// Caret position after the edit.
    pub caret_after: usize,
}

impl Edit {
    /// The range to replace when undoing.
    pub fn undo_range(&self) -> Range<usize> {
        self.range.start..self.range.start + self.inserted.chars().count()
    }

    /// The range to replace when redoing.
    pub fn redo_range(&self) -> Range<usize> {
        self.range.start..self.range.start + self.removed.chars().count()
    }
}

/// One undo step, with the context the change-history panel needs.
///
/// The line is recorded rather than derived because it cannot be derived later:
/// an `Edit` holds a character offset into the document as it was *before* the
/// edit, and by the time the panel draws, that document is gone. Recording the
/// line at the moment of the edit is the only way the panel can say where a
/// change happened without keeping a copy of every intermediate document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The reversible change.
    pub edit: Edit,
    /// Wall-clock second the step was recorded at, or 0 when the caller did not
    /// supply one. Zero is not a real timestamp, so the panel can tell the
    /// difference between "recorded at the epoch" and "no clock available".
    pub at: u64,
    /// The 1-based line the edit started on, as it was when recorded.
    pub line: usize,
}

impl Step {
    /// A step with no clock and no line, for callers that only want undo.
    pub fn bare(edit: Edit) -> Self {
        Step {
            edit,
            at: 0,
            line: 0,
        }
    }

    /// A short description of what the step did, for the panel.
    ///
    /// Deliberately terse: the panel is a list of a hundred rows, and a sentence
    /// per row is unreadable. The shape is "Insert 3 chars", "Delete 1 char",
    /// "Replace 2 with 5 chars", which is enough to recognise an edit by.
    pub fn describe(&self) -> String {
        let removed = self.edit.removed.chars().count();
        let inserted = self.edit.inserted.chars().count();
        match (removed, inserted) {
            (0, n) => format!("Insert {n} {}", plural(n)),
            (n, 0) => format!("Delete {n} {}", plural(n)),
            (r, i) => format!("Replace {r} with {i} {}", plural(i)),
        }
    }
}

/// `chars` or `char`, so a one-character edit does not read as a typo.
fn plural(n: usize) -> &'static str {
    if n == 1 {
        "char"
    } else {
        "chars"
    }
}

/// A bounded undo history.
#[derive(Debug)]
pub struct UndoStack {
    undo: Vec<Step>,
    redo: Vec<Step>,
    group_open: bool,
    group_is_delete: bool,
    limit: usize,
}

impl Default for UndoStack {
    fn default() -> Self {
        Self::new(1000)
    }
}

impl UndoStack {
    /// A stack that keeps at most `limit` undo steps.
    pub fn new(limit: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            group_open: false,
            group_is_delete: false,
            limit: limit.max(1),
        }
    }

    /// Records an edit with no clock and no line.
    pub fn push(&mut self, edit: Edit) {
        self.push_step(Step::bare(edit));
    }

    /// Records a step, merging it into the previous one when it continues a run
    /// of typing or backspacing.
    ///
    /// A merged step keeps the timestamp and line it was first recorded with.
    /// That is what a user means by "when I typed this": the start of the run,
    /// not the last keystroke of it.
    pub fn push_step(&mut self, step: Step) {
        self.redo.clear();

        if self.group_open {
            if let Some(last) = self.undo.last_mut() {
                if Self::coalesces(&last.edit, &step.edit, self.group_is_delete) {
                    Self::merge(&mut last.edit, step.edit);
                    return;
                }
            }
        }

        self.group_is_delete =
            step.edit.inserted.is_empty() && step.edit.removed.chars().count() == 1;
        self.undo.push(step);
        self.group_open = true;

        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
    }

    /// Whether `next` continues the run that produced `last`.
    fn coalesces(last: &Edit, next: &Edit, group_is_delete: bool) -> bool {
        if !last.inserted.is_empty() && !next.inserted.is_empty() {
            // Typing: the new text must start exactly where the old text ended.
            return last.range.start + last.inserted.chars().count() == next.range.start;
        }
        if group_is_delete
            && last.inserted.is_empty()
            && next.inserted.is_empty()
            && next.removed.chars().count() == 1
        {
            // Backspace: the new range ends where the old one started.
            return next.range.end == last.range.start;
        }
        false
    }

    /// Folds `next` into `last`.
    fn merge(last: &mut Edit, next: Edit) {
        if !next.inserted.is_empty() {
            last.inserted.push_str(&next.inserted);
            // Overwrite typing consumes a character per keystroke, so the text
            // it removed grows with it. Plain typing removes nothing and this
            // branch is skipped, leaving the range empty as before.
            if !next.removed.is_empty() {
                last.removed.push_str(&next.removed);
                last.range.end = next.range.end;
            }
        } else {
            let mut removed = next.removed;
            removed.push_str(&last.removed);
            last.removed = removed;
            last.range.start = next.range.start;
        }
        last.caret_after = next.caret_after;
    }

    /// Ends the current coalescing group.
    pub fn break_group(&mut self) {
        self.group_open = false;
    }

    /// Pops the most recent edit so the caller can revert it.
    pub fn undo(&mut self) -> Option<Edit> {
        self.undo_step().map(|step| step.edit)
    }

    /// Pops the most recent step, with its context.
    pub fn undo_step(&mut self) -> Option<Step> {
        let step = self.undo.pop()?;
        self.redo.push(step.clone());
        self.group_open = false;
        Some(step)
    }

    /// Pops the most recently undone edit so the caller can reapply it.
    pub fn redo(&mut self) -> Option<Edit> {
        self.redo_step().map(|step| step.edit)
    }

    /// Pops the most recently undone step, with its context.
    pub fn redo_step(&mut self) -> Option<Step> {
        let step = self.redo.pop()?;
        self.undo.push(step.clone());
        self.group_open = false;
        Some(step)
    }

    /// The steps that can be undone, oldest first.
    pub fn steps(&self) -> &[Step] {
        &self.undo
    }

    /// The steps that can be redone, newest first.
    ///
    /// The opposite order to [`UndoStack::steps`], because the two stacks are
    /// popped from opposite ends: `redo` is pushed in the order steps were
    /// undone, so its last element is the one `redo` will return next.
    pub fn redo_steps(&self) -> &[Step] {
        &self.redo
    }

    /// Undoes steps until `keep` remain, returning them newest first.
    ///
    /// The popped steps go onto the redo stack, so a revert to an earlier point
    /// is itself reversible: Ctrl+Y walks forward again. Returning them rather
    /// than applying them here keeps the stack free of any knowledge of the
    /// document, which is what lets it be tested on its own.
    pub fn truncate_to(&mut self, keep: usize) -> Vec<Step> {
        let mut out = Vec::new();
        while self.undo.len() > keep {
            match self.undo_step() {
                Some(step) => out.push(step),
                None => break,
            }
        }
        out
    }

    /// Whether there is anything to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether there is anything to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Discards all history.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.group_open = false;
    }

    /// Number of undo steps currently held.
    pub fn depth(&self) -> usize {
        self.undo.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert_edit(at: usize, text: &str) -> Edit {
        Edit {
            range: at..at,
            removed: String::new(),
            inserted: text.to_string(),
            caret_before: at,
            caret_after: at + text.chars().count(),
        }
    }

    fn delete_edit(at: usize, text: &str) -> Edit {
        Edit {
            range: at..at + text.chars().count(),
            removed: text.to_string(),
            inserted: String::new(),
            caret_before: at + text.chars().count(),
            caret_after: at,
        }
    }

    #[test]
    fn typing_coalesces_into_one_step() {
        let mut stack = UndoStack::default();
        for (i, c) in "hello".chars().enumerate() {
            stack.push(insert_edit(i, &c.to_string()));
        }
        assert_eq!(stack.depth(), 1);
        let edit = stack.undo().unwrap();
        assert_eq!(edit.inserted, "hello");
        assert_eq!(edit.range, 0..0);
    }

    #[test]
    fn non_adjacent_typing_starts_a_new_step() {
        let mut stack = UndoStack::default();
        stack.push(insert_edit(0, "a"));
        stack.push(insert_edit(5, "b"));
        assert_eq!(stack.depth(), 2);
    }

    #[test]
    fn break_group_splits_typing() {
        let mut stack = UndoStack::default();
        stack.push(insert_edit(0, "a"));
        stack.break_group();
        stack.push(insert_edit(1, "b"));
        assert_eq!(stack.depth(), 2);
    }

    #[test]
    fn backspaces_coalesce() {
        let mut stack = UndoStack::default();
        stack.push(delete_edit(4, "o"));
        stack.push(delete_edit(3, "l"));
        stack.push(delete_edit(2, "l"));
        assert_eq!(stack.depth(), 1);
        let edit = stack.undo().unwrap();
        assert_eq!(edit.removed, "llo");
        assert_eq!(edit.range, 2..5);
    }

    #[test]
    fn deleting_a_selection_does_not_coalesce_with_backspace() {
        let mut stack = UndoStack::default();
        stack.push(delete_edit(5, "world"));
        stack.push(delete_edit(4, "o"));
        assert_eq!(stack.depth(), 2);
    }

    #[test]
    fn undo_then_redo_roundtrips() {
        let mut stack = UndoStack::default();
        stack.push(insert_edit(0, "abc"));
        let undone = stack.undo().unwrap();
        assert_eq!(undone.inserted, "abc");
        assert!(!stack.can_undo());
        assert!(stack.can_redo());
        let redone = stack.redo().unwrap();
        assert_eq!(redone.inserted, "abc");
        assert!(stack.can_undo());
        assert!(!stack.can_redo());
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut stack = UndoStack::default();
        stack.push(insert_edit(0, "a"));
        stack.undo();
        assert!(stack.can_redo());
        stack.push(insert_edit(0, "b"));
        assert!(!stack.can_redo());
    }

    #[test]
    fn limit_drops_oldest_steps() {
        let mut stack = UndoStack::new(2);
        stack.push(insert_edit(0, "a"));
        stack.break_group();
        stack.push(insert_edit(1, "b"));
        stack.break_group();
        stack.push(insert_edit(2, "c"));
        assert_eq!(stack.depth(), 2);
        assert_eq!(stack.undo().unwrap().inserted, "c");
        assert_eq!(stack.undo().unwrap().inserted, "b");
        assert!(!stack.can_undo());
    }

    fn step(at: usize, text: &str, line: usize, when: u64) -> Step {
        Step {
            edit: insert_edit(at, text),
            at: when,
            line,
        }
    }

    #[test]
    fn a_step_keeps_the_context_it_was_recorded_with() {
        let mut stack = UndoStack::default();
        stack.push_step(step(0, "abc", 7, 1700000000));
        let steps = stack.steps();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].line, 7);
        assert_eq!(steps[0].at, 1_700_000_000);
    }

    #[test]
    fn a_merged_step_keeps_the_first_timestamp_and_line() {
        // A run of typing is one step, and the step is when the run started.
        let mut stack = UndoStack::default();
        stack.push_step(step(0, "a", 3, 100));
        stack.push_step(step(1, "b", 9, 200));
        assert_eq!(stack.depth(), 1);
        assert_eq!(stack.steps()[0].at, 100);
        assert_eq!(stack.steps()[0].line, 3);
        assert_eq!(stack.steps()[0].edit.inserted, "ab");
    }

    #[test]
    fn a_bare_step_has_no_clock_and_no_line() {
        let step = Step::bare(insert_edit(0, "x"));
        assert_eq!(step.at, 0);
        assert_eq!(step.line, 0);
    }

    #[test]
    fn a_step_describes_what_it_did() {
        assert_eq!(
            Step::bare(insert_edit(0, "abc")).describe(),
            "Insert 3 chars"
        );
        assert_eq!(Step::bare(insert_edit(0, "a")).describe(), "Insert 1 char");
        assert_eq!(
            Step::bare(delete_edit(0, "ab")).describe(),
            "Delete 2 chars"
        );
        let replace = Step::bare(Edit {
            range: 0..2,
            removed: "ab".to_string(),
            inserted: "xyz".to_string(),
            caret_before: 2,
            caret_after: 3,
        });
        assert_eq!(replace.describe(), "Replace 2 with 3 chars");
    }

    #[test]
    fn truncate_to_undoes_down_to_the_requested_depth() {
        let mut stack = UndoStack::default();
        for i in 0..4 {
            stack.push_step(step(i, "x", i + 1, 0));
            stack.break_group();
        }
        assert_eq!(stack.depth(), 4);
        let popped = stack.truncate_to(1);
        assert_eq!(popped.len(), 3);
        // Newest first, so the caller can apply the inverses in order.
        assert_eq!(popped[0].line, 4);
        assert_eq!(popped[2].line, 2);
        assert_eq!(stack.depth(), 1);
    }

    #[test]
    fn a_truncated_step_can_be_redone() {
        let mut stack = UndoStack::default();
        for i in 0..3 {
            stack.push_step(step(i, "x", i + 1, 0));
            stack.break_group();
        }
        stack.truncate_to(1);
        assert_eq!(stack.redo_steps().len(), 2);
        // The redo side is newest first, so the step `redo` returns next is the
        // last element, which is the oldest of the two that were reverted.
        assert_eq!(stack.redo_steps()[0].line, 3);
        assert_eq!(stack.redo_steps()[1].line, 2);
        assert_eq!(stack.redo().unwrap().inserted, "x");
        assert_eq!(stack.depth(), 2);
    }

    #[test]
    fn truncating_past_the_end_does_nothing() {
        let mut stack = UndoStack::default();
        stack.push(insert_edit(0, "a"));
        assert!(stack.truncate_to(10).is_empty());
        assert_eq!(stack.depth(), 1);
    }

    #[test]
    fn undo_range_covers_inserted_text() {
        let edit = Edit {
            range: 10..12,
            removed: "xy".to_string(),
            inserted: "abcd".to_string(),
            caret_before: 12,
            caret_after: 14,
        };
        assert_eq!(edit.undo_range(), 10..14);
        assert_eq!(edit.redo_range(), 10..12);
    }
}
