//! Pad a block of lines so one column of text lines up.
//!
//! A port of the behaviour of the Notepad++ column plugins: given lines that
//! each carry one separator -- `=` in a config, `:` in a YAML block, `,` in a
//! table -- make the separators start in the same column by padding what comes
//! before them with spaces. The separator is the first one on the line, which is
//! the one people mean: a value may well contain the character being aligned on,
//! and the first one is the only position that exists on every line.
//!
//! The block is the selection when there is one and the whole document when
//! there is not: a column command with nothing marked is about the table in
//! front of you, not about the one line the caret is on.
//!
//! Written here rather than translated; see `plugins/README.md`.

use nn_plugin::host;

/// Aligns `lines` on their first `separator`, leaving the others alone.
///
/// A line without the separator is a line that has nothing to align, so it is
/// left exactly as it was rather than pushed to a column of its own. Trailing
/// whitespace before the separator is dropped first, so a block that was already
/// aligned and then spaced by hand ends up aligned again instead of drifting
/// one space per run: running the command twice has to be the same as running it
/// once.
pub fn align_lines(lines: &[String], separator: char) -> Vec<String> {
    let mut columns: Vec<Option<usize>> = Vec::with_capacity(lines.len());
    let mut widest = 0usize;
    for line in lines {
        let column = line.find(separator).map(|at| {
            let left = line[..at].trim_end().len();
            widest = widest.max(left);
            left
        });
        columns.push(column);
    }
    if widest == 0 {
        return lines.to_vec();
    }
    lines
        .iter()
        .zip(&columns)
        .map(|(line, column)| match column {
            Some(left) => {
                let at = line.find(separator).expect("checked above");
                let pad = widest - left;
                let mut out = String::with_capacity(line.len() + pad + 1);
                out.push_str(&line[..*left]);
                out.extend(std::iter::repeat_n(' ', pad + 1));
                out.push_str(&line[at..]);
                out
            }
            None => line.clone(),
        })
        .collect()
}

/// The lines the selection touches, as a pair of inclusive line numbers.
///
/// The command aligns the whole document when nothing is selected, so this is
/// only asked when there *is* a selection. It takes offsets rather than a
/// boolean because "nothing selected" and "the caret is on line 3" are different
/// questions and the answer to the second one is not an empty range.
pub fn touched_lines(text: &str, start: usize, end: usize) -> (usize, usize) {
    let first = text[..start.min(text.len())]
        .bytes()
        .filter(|&b| b == b'\n')
        .count();
    let last = text[..end.min(text.len())]
        .bytes()
        .filter(|&b| b == b'\n')
        .count();
    (first, last)
}

// ---- commands -------------------------------------------------------------

fn align(separator: char) {
    let text = host::text();
    let selection = host::selection();
    let mut lines: Vec<String> = host::lines();
    // A selection means the block the user marked; no selection means the whole
    // document, because a column command with nothing marked is about the table
    // in front of you, not about the single line the caret happens to be on.
    let (first, last) = if selection.is_empty() {
        (0, lines.len().saturating_sub(1))
    } else {
        touched_lines(&text, selection.start, selection.end)
    };
    let last = last.min(lines.len().saturating_sub(1));
    if first > last || lines.is_empty() {
        return;
    }
    let block = align_lines(&lines[first..=last], separator);
    let changed = block.iter().zip(&lines[first..=last]).any(|(a, b)| a != b);
    if !changed {
        host::log("nothing to align: no line in the block has that separator");
        return;
    }
    lines.splice(first..=last, block);
    let joined = lines.join("\n");
    // The document is replaced whole because the padding changes lengths, and a
    // plugin has no way to hand back a list of edits. It is one undo step either
    // way.
    if !host::replace(&joined) {
        host::log("the document is read-only");
    }
}

fn equals() {
    align('=');
}

fn colon() {
    align(':');
}

fn comma() {
    align(',');
}

nn_plugin::plugin! {
    name: "Align Columns",
    commands: [
        ("Align on Equals", equals),
        ("Align on Colon", colon),
        ("Align on Comma", comma),
    ],
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.split('\n').map(str::to_owned).collect()
    }

    #[test]
    fn the_separator_column_lines_up() {
        let input = lines("a = 1\nlong_name = 2\nmid = 3");
        let out = align_lines(&input, '=');
        assert_eq!(out, ["a         = 1", "long_name = 2", "mid       = 3"]);
    }

    #[test]
    fn a_line_without_the_separator_is_left_alone() {
        let input = lines("a = 1\njust text\nbb = 2");
        let out = align_lines(&input, '=');
        assert_eq!(out, ["a  = 1", "just text", "bb = 2"]);
    }

    #[test]
    fn only_the_first_separator_is_used() {
        let input = lines("a = b = c\ndd = e");
        let out = align_lines(&input, '=');
        assert_eq!(out, ["a  = b = c", "dd = e"]);
    }

    #[test]
    fn aligning_twice_changes_nothing_the_second_time() {
        let once = align_lines(&lines("a = 1\nbbbbb = 2"), '=');
        let twice = align_lines(&once, '=');
        assert_eq!(once, twice);
        assert_eq!(once, ["a     = 1", "bbbbb = 2"]);
    }

    #[test]
    fn a_block_with_no_separator_is_unchanged() {
        let input = lines("one\ntwo");
        assert_eq!(align_lines(&input, '='), input);
    }

    #[test]
    fn the_touched_lines_are_the_ones_the_selection_covers() {
        let text = "one\ntwo\nthree\nfour";
        // A caret on "two" touches only that line.
        assert_eq!(touched_lines(text, 4, 4), (1, 1));
        // A selection from the middle of "two" into "three" touches both.
        assert_eq!(touched_lines(text, 5, 10), (1, 2));
        // Off the end, the last line is still the last line.
        assert_eq!(touched_lines(text, 99, 99), (3, 3));
    }
}
