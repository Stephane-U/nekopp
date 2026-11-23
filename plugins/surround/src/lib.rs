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

//! Wrap the selection in a pair, or find the pair that already encloses it.
//!
//! A port of the behaviour of the Notepad++ "Surround Selection" and
//! "SelectQuotedText" plugins: two gestures that are the same gesture read in
//! opposite directions. Wrapping is the one people reach for while writing --
//! select a word, turn it into a string literal -- and selecting the quoted text
//! is what they reach for when the quotes are already there and they meant to
//! grab what is inside them.
//!
//! The algorithm is written here rather than translated: the plugins that do
//! this in Notepad++ are GPL and this repository has no licence of its own yet,
//! so the behaviour was reimplemented from its description. See
//! `plugins/README.md`.

use std::ops::Range;

use nn_plugin::host;

/// A replacement of `range` with `text`, after which `selection` is selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<usize>,
    pub text: String,
    pub selection: Range<usize>,
}

/// Puts `open` and `close` around the selected text, or around the caret.
///
/// With nothing selected the pair is inserted and the caret is put between the
/// two halves, which is what makes the command usable as "start a string here".
/// With something selected the delimiters go outside it and the text stays
/// selected, so a second command -- or a second press of the same one -- can
/// wrap what was just wrapped.
pub fn wrap(text: &str, selection: Range<usize>, open: &str, close: &str) -> Edit {
    let start = selection.start.min(selection.end).min(text.len());
    let end = selection.end.max(selection.start).min(text.len());
    let inner = &text[start..end];
    let mut wrapped = String::with_capacity(open.len() + inner.len() + close.len());
    wrapped.push_str(open);
    wrapped.push_str(inner);
    wrapped.push_str(close);
    let inner_start = start + open.len();
    Edit {
        range: start..end,
        text: wrapped,
        selection: inner_start..inner_start + inner.len(),
    }
}

/// The delimiter pairs this plugin knows about, innermost candidates first.
const PAIRS: &[(char, char)] = &[
    ('"', '"'),
    ('\'', '\''),
    ('(', ')'),
    ('[', ']'),
    ('{', '}'),
    ('<', '>'),
];

/// The innermost pair of quotes or brackets containing `selection` on its line.
///
/// Answers the range *inside* the delimiters. The search is deliberately
/// local -- nearest opener to the left, nearest matching closer to the right, on
/// the same line -- because a document-wide search would happily jump into a
/// string three screens away from the cursor. A delimiter preceded by a
/// backslash is skipped: `"a\"b"` has three quotes and only two of them count.
pub fn enclosing(text: &str, selection: Range<usize>) -> Option<Range<usize>> {
    let start = selection.start.min(text.len());
    let end = selection.end.min(text.len());
    let line_start = text[..start].rfind('\n').map_or(0, |at| at + 1);
    let line_end = text[end..].find('\n').map_or(text.len(), |at| end + at);
    let before = &text[line_start..start];
    let after = &text[end..line_end];

    // The nearest opener wins, and on a tie an opener is only counted when its
    // own kind closes after the selection: `(a "b")` with the caret inside the
    // quotes has a `(` nearer than the quote, and that `(` does close after it,
    // so the parens are the answer.
    let mut best: Option<(usize, char, char)> = None;
    for (open, close) in PAIRS {
        if let Some(at) = last_unescaped(before, *open) {
            let distance = start - (line_start + at);
            let nearer = best.is_none_or(|(seen, _, _)| distance < seen);
            if nearer && first_unescaped(after, *close).is_some() {
                best = Some((distance, *open, *close));
            }
        }
    }
    let (distance, open, close) = best?;
    let inner_start = start - distance + open.len_utf8();
    let closer_at = first_unescaped(after, close)?;
    Some(inner_start..end + closer_at)
}

/// The distance from the end of `text` to the last unescaped `delimiter`.
fn last_unescaped(text: &str, delimiter: char) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut found = None;
    for (at, ch) in text.char_indices() {
        if ch != delimiter {
            continue;
        }
        let escaped = at > 0 && bytes[at - 1] == b'\\';
        if !escaped {
            found = Some(at);
        }
    }
    found
}

/// The distance from the start of `text` to the first unescaped `delimiter`.
fn first_unescaped(text: &str, delimiter: char) -> Option<usize> {
    let bytes = text.as_bytes();
    for (at, ch) in text.char_indices() {
        if ch == delimiter && (at == 0 || bytes[at - 1] != b'\\') {
            return Some(at);
        }
    }
    None
}

// ---- commands -------------------------------------------------------------

fn surround(open: &str, close: &str) {
    let text = host::text();
    let selection = host::selection();
    let edit = wrap(&text, selection.clone(), open, close);
    if !host::replace_range(edit.range.start, edit.range.end - edit.range.start, &edit.text) {
        host::log("the document is read-only");
        return;
    }
    host::select(edit.selection.start, edit.selection.end);
}

fn quotes() {
    surround("\"", "\"");
}

fn brackets() {
    surround("[", "]");
}

fn parentheses() {
    surround("(", ")");
}

fn select_quoted() {
    let text = host::text();
    let selection = host::selection();
    match enclosing(&text, selection.clone()) {
        Some(inner) if inner != selection => {
            host::select(inner.start, inner.end);
        }
        Some(_) => host::log("already exactly the quoted text"),
        None => host::log("nothing quoted or bracketed around the selection"),
    }
}

nn_plugin::plugin! {
    name: "Surround",
    commands: [
        ("Surround with Quotes", quotes),
        ("Surround with Brackets", brackets),
        ("Surround with Parentheses", parentheses),
        ("Select Quoted Text", select_quoted),
    ],
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: usize, end: usize) -> Range<usize> {
        start..end
    }

    #[test]
    fn wrapping_a_selection_keeps_it_selected() {
        let edit = wrap("say hello", range(4, 9), "\"", "\"");
        assert_eq!(edit.text, "\"hello\"");
        assert_eq!(edit.range, range(4, 9));
        // The inner text, not the delimiters: wrapping twice nests.
        assert_eq!(edit.selection, range(5, 10));
        // The selection `wrap` answers is in the document's coordinates, so a
        // second pass over the same document nests rather than replacing.
        assert_eq!(edit.selection, range(5, 10));
        let again = wrap("say \"hello\"", edit.selection, "\"", "\"");
        assert_eq!(again.text, "\"hello\"");
        assert_eq!(again.range, range(5, 10));
    }

    #[test]
    fn wrapping_nothing_inserts_a_pair_and_leaves_the_caret_between() {
        let edit = wrap("hello", range(5, 5), "(", ")");
        assert_eq!(edit.text, "()");
        assert_eq!(edit.range, range(5, 5));
        assert_eq!(edit.selection, range(6, 6));
    }

    #[test]
    fn a_backward_selection_wraps_the_same_way() {
        let forward = wrap("say hello", range(4, 9), "[", "]");
        let backward = wrap("say hello", range(9, 4), "[", "]");
        assert_eq!(forward, backward);
    }

    #[test]
    fn the_innermost_quotes_are_found() {
        let text = "let x = \"hello world\";";
        // The caret is on "world".
        assert_eq!(enclosing(text, range(17, 17)), Some(range(9, 20)));
    }

    #[test]
    fn brackets_are_found_around_a_quoted_string() {
        let text = "call(\"inner\", other)";
        // Inside the quotes: the quotes are nearer than the parens.
        assert_eq!(enclosing(text, range(7, 7)), Some(range(6, 11)));
        // Between the arguments: nothing but the call's parens.
        assert_eq!(enclosing(text, range(12, 12)), Some(range(5, 19)));
    }

    #[test]
    fn an_escaped_quote_does_not_open() {
        let text = "\"a\\\"b\"";
        // The caret sits after the escaped quote, inside the string.
        let found = enclosing(text, range(5, 5));
        assert_eq!(found, Some(range(1, 5)));
        // Between the two real quotes on the left there is nothing to find.
        assert_eq!(enclosing("a \" b", range(4, 4)), None);
    }

    #[test]
    fn nothing_enclosing_answers_none() {
        assert_eq!(enclosing("plain words here", range(3, 3)), None);
        // An opener on another line does not reach into this one.
        assert_eq!(enclosing("(\nwords", range(3, 3)), None);
    }

    #[test]
    fn an_unclosed_opener_is_not_an_answer() {
        assert_eq!(enclosing("\"unclosed", range(4, 4)), None);
    }

}
