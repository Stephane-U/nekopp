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

//! Word completion from the open document.
//!
//! Notepad++'s `Ctrl+Space` completes from the words already in the file. That
//! is the whole feature: no dictionary, no language server, no model. It is
//! useful precisely because it only offers words you have already typed, so it
//! never suggests something that does not belong in the file.
//!
//! The index is built on demand rather than maintained: a completion request is
//! a keystroke, not a frame, so one pass over the text is cheap enough and there
//! is no cache to invalidate on every edit.

use std::collections::HashSet;
use std::ops::Range;

use crate::symbols::{self, Kind};

/// The shortest word worth offering. Two-letter words are noise.
pub const MIN_LEN: usize = 3;

/// The most candidates a single request returns.
pub const MAX_CANDIDATES: usize = 200;

/// Collects the distinct words in `text`, sorted, capped at `limit`.
///
/// A word is a run of alphanumerics and underscores, which is the same rule the
/// editor uses for word-wise movement — so what completion offers and what
/// `Ctrl+Right` skips over are the same thing.
pub fn collect(text: &str, limit: usize) -> Vec<String> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    collect_into(text, &mut seen, &mut out, limit);
    out.sort_unstable();
    out.truncate(limit);
    out
}

/// Collects the distinct words across several documents.
///
/// Document completion is the same feature as word completion with a wider
/// haystack: Notepad++ offers words from every open file, not just the one you
/// are typing in. The words are pooled before sorting so the result is one
/// alphabetical list rather than one list per document.
pub fn collect_many<'a>(texts: &[&'a str], limit: usize) -> Vec<String> {
    let mut seen: HashSet<&'a str> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for text in texts {
        collect_into(text, &mut seen, &mut out, limit);
    }
    out.sort_unstable();
    out.truncate(limit);
    out
}

/// Adds every word in `text` to `out`, skipping duplicates and short words.
fn collect_into<'a>(
    text: &'a str,
    seen: &mut HashSet<&'a str>,
    out: &mut Vec<String>,
    limit: usize,
) {
    let mut start: Option<usize> = None;

    for (i, c) in text.char_indices() {
        if c.is_alphanumeric() || c == '_' {
            start.get_or_insert(i);
            continue;
        }
        if let Some(from) = start.take() {
            push_word(&text[from..i], seen, out, limit);
        }
    }
    if let Some(from) = start {
        push_word(&text[from..], seen, out, limit);
    }
}

/// Adds `word` to `out` unless it is too short or already there.
fn push_word<'a>(word: &'a str, seen: &mut HashSet<&'a str>, out: &mut Vec<String>, limit: usize) {
    if word.chars().count() < MIN_LEN || out.len() >= limit {
        return;
    }
    if seen.insert(word) {
        out.push(word.to_string());
    }
}

/// The words in `words` that start with `prefix`, best first.
///
/// Case-insensitive matches come before case-sensitive ones, so typing `doc`
/// offers `Document` before `docstring`. The prefix itself is dropped: offering
/// the word you have already finished typing is not a completion.
pub fn matching<'a>(prefix: &str, words: &'a [String], limit: usize) -> Vec<&'a str> {
    if prefix.is_empty() {
        return Vec::new();
    }
    let lower = prefix.to_lowercase();
    let mut exact: Vec<&str> = Vec::new();
    let mut loose: Vec<&str> = Vec::new();
    for word in words {
        if word == prefix || !word.to_lowercase().starts_with(&lower) {
            continue;
        }
        if word.starts_with(prefix) {
            exact.push(word);
        } else {
            loose.push(word);
        }
        if exact.len() + loose.len() >= limit {
            break;
        }
    }
    exact.extend(loose);
    exact.truncate(limit);
    exact
}

/// The word immediately before `caret` in `text`, as a byte range.
///
/// Completion replaces this range, so it has to be the run of word characters
/// that ends exactly at the caret — not the word the caret is merely inside.
pub fn prefix_range(text: &str, caret: usize) -> Option<std::ops::Range<usize>> {
    let caret = caret.min(text.len());
    if !text.is_char_boundary(caret) {
        return None;
    }
    let mut start = caret;
    for (i, c) in text[..caret].char_indices().rev() {
        if c.is_alphanumeric() || c == '_' {
            start = i;
        } else {
            break;
        }
    }
    (start < caret).then_some(start..caret)
}

/// A function declaration, for function completion and parameter hints.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Function {
    /// The declared name.
    pub name: String,
    /// The text between the parentheses, as written, without the parentheses.
    pub params: String,
    /// Zero-based line the declaration starts on.
    pub line: usize,
}

/// Scans `text` for function declarations, in document order.
///
/// This reuses the function list's scanner rather than growing a second one: the
/// list and the completion popup have to agree about what a function is, and two
/// scanners would drift. What this adds is the parameter list, which the list
/// panel does not need but a hint does.
///
/// The parameter text is taken from the whole document rather than the line, so
/// a signature broken across lines still reads as one list. A declaration whose
/// parentheses never close is dropped: a hint that shows half a signature is
/// worse than no hint.
pub fn functions(text: &str) -> Vec<Function> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    for (line, raw) in text.split_inclusive('\n').enumerate() {
        let body = raw.strip_suffix('\n').unwrap_or(raw);
        let body = body.strip_suffix('\r').unwrap_or(body);
        if let Some(symbol) = symbols::scan_line(body, line) {
            if symbol.kind == Kind::Function {
                if let Some(params) = params_after(text, offset, &symbol.name) {
                    out.push(Function {
                        name: symbol.name,
                        params,
                        line,
                    });
                }
            }
        }
        offset += raw.len();
    }
    out
}

/// The parameter list of the declaration of `name`, if the document has one.
///
/// The first declaration wins. Overloads are a real thing in C++ and Java, but
/// picking the first is what Notepad++ does and it is right far more often than
/// it is wrong: the first declaration in a file is usually the one being called.
pub fn params_of(text: &str, name: &str) -> Option<String> {
    functions(text)
        .into_iter()
        .find(|f| f.name == name)
        .map(|f| f.params)
}

/// The parameter list that follows `name` on the line starting at `from`.
fn params_after(text: &str, from: usize, name: &str) -> Option<String> {
    let open = find_call_paren(text, from, name)?;
    balanced(text, open).map(|(params, _)| params)
}

/// The index of the `(` that opens `name`'s parameter list, at or after `from`.
///
/// The name has to be a whole word and the parenthesis has to follow it, which
/// is what keeps `pub` from matching inside `pub fn publish(`.
fn find_call_paren(text: &str, from: usize, name: &str) -> Option<usize> {
    let hay = &text[from..];
    let mut search = 0usize;
    while let Some(rel) = hay[search..].find(name) {
        let at = search + rel;
        let whole = hay[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        let after = at + name.len();
        let rest = &hay[after..];
        let trimmed = rest.trim_start();
        if whole && trimmed.starts_with('(') {
            return Some(from + after + (rest.len() - trimmed.len()));
        }
        search = after;
    }
    None
}

/// The text between the parenthesis at `open` and its partner, plus the index of
/// the partner.
fn balanced(text: &str, open: usize) -> Option<(String, usize)> {
    let mut depth = 0usize;
    for (i, c) in text[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some((text[open + 1..open + i].to_string(), open + i));
                }
            }
            _ => {}
        }
    }
    None
}

/// A call the caret is sitting inside.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Call {
    /// The name being called.
    pub name: String,
    /// Zero-based index of the argument the caret is in.
    pub arg: usize,
}

/// The call whose argument list contains `caret`, if any.
///
/// Walking backwards from the caret and counting parentheses is enough to find
/// the innermost open call, and it is what makes the hint follow the caret into
/// nested calls: `outer(inner(|))` reports `inner`. A statement boundary at depth
/// zero means the caret is not inside a call at all, which is why `;`, `{` and
/// `}` stop the walk.
pub fn call_at(text: &str, caret: usize) -> Option<Call> {
    let caret = caret.min(text.len());
    if !text.is_char_boundary(caret) {
        return None;
    }
    let mut depth = 0usize;
    let mut open = None;
    for (i, c) in text[..caret].char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' => {
                if depth == 0 {
                    open = Some(i);
                    break;
                }
                depth -= 1;
            }
            ';' | '{' | '}' if depth == 0 => return None,
            _ => {}
        }
    }
    let open = open?;
    let name = ident_before(text, open)?;
    Some(Call {
        name,
        arg: count_commas(text, open + 1, caret),
    })
}

/// The identifier that ends just before `at`, skipping trailing whitespace.
fn ident_before(text: &str, at: usize) -> Option<String> {
    let head = text[..at].trim_end();
    let start = head
        .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
        .map_or(0, |i| i + 1);
    let name = &head[start..];
    (!name.is_empty() && !name.starts_with(|c: char| c.is_ascii_digit())).then(|| name.to_string())
}

/// The number of top-level commas in `text[from..to]`.
///
/// Brackets are tracked so a comma inside a nested call, an array literal or a
/// block does not count as an argument separator. Angle brackets are not
/// tracked: `a < b` is a comparison far more often than it is a generic, and
/// treating it as one would swallow every comma after it.
fn count_commas(text: &str, from: usize, to: usize) -> usize {
    let mut depth = 0usize;
    let mut count = 0usize;
    for c in text[from..to].chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => count += 1,
            _ => {}
        }
    }
    count
}

/// The byte range of argument `index` inside a parameter list.
///
/// The hint picks out the argument the caret is in, so it needs the range of
/// that argument's text rather than its index. Surrounding whitespace is
/// trimmed: highlighting the space after a comma reads as a mistake.
pub fn arg_range(params: &str, index: usize) -> Option<Range<usize>> {
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut current = 0usize;
    for (i, c) in params.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                if current == index {
                    return Some(trimmed(params, start..i));
                }
                current += 1;
                start = i + 1;
            }
            _ => {}
        }
    }
    (current == index).then(|| trimmed(params, start..params.len()))
}

/// `range` with surrounding whitespace removed.
fn trimmed(text: &str, range: Range<usize>) -> Range<usize> {
    let slice = &text[range.clone()];
    let lead = slice.len() - slice.trim_start().len();
    let trail = slice.len() - slice.trim_end().len();
    if lead + trail >= slice.len() {
        return range.start..range.start;
    }
    range.start + lead..range.end - trail
}

/// The path fragment ending at `caret`, as a byte range.
///
/// Path completion needs the run of characters that could be part of a filename:
/// everything back to the last space, quote or bracket. The fragment has to
/// contain a separator, because completing a bare word is word completion and
/// offering directory listings for every word would be unusable.
pub fn path_prefix(text: &str, caret: usize) -> Option<Range<usize>> {
    let caret = caret.min(text.len());
    if !text.is_char_boundary(caret) {
        return None;
    }
    let mut start = caret;
    for (i, c) in text[..caret].char_indices().rev() {
        if c.is_whitespace() || "\"'`()[]{}<>,;=".contains(c) {
            break;
        }
        start = i;
    }
    let fragment = &text[start..caret];
    let separated = fragment.contains('/') || fragment.contains('\\');
    (separated && !fragment.starts_with("//")).then_some(start..caret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_collected_and_deduplicated() {
        let words = collect("let document = Document::new(); document.save()", 100);
        assert_eq!(words, vec!["Document", "document", "let", "new", "save"]);
    }

    #[test]
    fn short_words_are_dropped() {
        let words = collect("a ab abc abcd", 100);
        assert_eq!(words, vec!["abc", "abcd"]);
    }

    #[test]
    fn underscores_join_words() {
        let words = collect("foo_bar foo", 100);
        assert_eq!(words, vec!["foo", "foo_bar"]);
    }

    #[test]
    fn the_collection_is_capped() {
        let text: String = (0..500).map(|i| format!("word{i} ")).collect();
        assert_eq!(collect(&text, 10).len(), 10);
    }

    #[test]
    fn a_word_at_the_end_of_the_text_is_collected() {
        assert_eq!(collect("alpha beta", 100), vec!["alpha", "beta"]);
    }

    #[test]
    fn matching_is_prefix_only() {
        let words = collect("document docstring doctor dogma", 100);
        assert_eq!(
            matching("doc", &words, 10),
            vec!["docstring", "doctor", "document"]
        );
    }

    #[test]
    fn case_insensitive_matches_come_last() {
        let words = collect("Document docstring", 100);
        assert_eq!(matching("doc", &words, 10), vec!["docstring", "Document"]);
    }

    #[test]
    fn the_prefix_itself_is_not_offered() {
        let words = collect("document documental", 100);
        assert_eq!(matching("document", &words, 10), vec!["documental"]);
    }

    #[test]
    fn an_empty_prefix_offers_nothing() {
        let words = collect("document", 100);
        assert!(matching("", &words, 10).is_empty());
    }

    #[test]
    fn matching_is_capped() {
        let text: String = (0..100).map(|i| format!("doc{i} ")).collect();
        let words = collect(&text, 1000);
        assert_eq!(matching("doc", &words, 5).len(), 5);
    }

    #[test]
    fn the_prefix_range_is_the_word_ending_at_the_caret() {
        let text = "let document = 1";
        assert_eq!(prefix_range(text, 12), Some(4..12));
    }

    #[test]
    fn the_prefix_range_is_none_after_a_separator() {
        let text = "let document = ";
        assert_eq!(prefix_range(text, 15), None);
    }

    #[test]
    fn the_prefix_range_stops_at_a_non_word_character() {
        let text = "foo.bar";
        assert_eq!(prefix_range(text, 7), Some(4..7));
    }

    #[test]
    fn the_prefix_range_handles_multibyte_text() {
        let text = "héllo";
        assert_eq!(prefix_range(text, text.len()), Some(0..text.len()));
    }

    #[test]
    fn words_are_pooled_across_documents() {
        let words = collect_many(&["alpha beta", "beta gamma"], 100);
        assert_eq!(words, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn pooling_is_capped() {
        let a: String = (0..50).map(|i| format!("a{i} ")).collect();
        let b: String = (0..50).map(|i| format!("b{i} ")).collect();
        assert_eq!(collect_many(&[&a, &b], 10).len(), 10);
    }

    #[test]
    fn pooling_nothing_is_empty() {
        assert!(collect_many(&[], 10).is_empty());
    }

    #[test]
    fn functions_carry_their_parameters() {
        let text = "fn save(path: &Path) -> io::Result<()> {\n}\n";
        assert_eq!(
            functions(text),
            vec![Function {
                name: "save".to_string(),
                params: "path: &Path".to_string(),
                line: 0,
            }]
        );
    }

    #[test]
    fn a_function_with_no_parameters_has_an_empty_list() {
        let text = "fn new() -> Self {\n}\n";
        assert_eq!(functions(text)[0].params, "");
    }

    #[test]
    fn a_signature_broken_across_lines_is_one_list() {
        let text = "fn save(\n    path: &Path,\n    mode: Mode,\n) -> Result<()> {\n}\n";
        assert_eq!(
            functions(text)[0].params,
            "\n    path: &Path,\n    mode: Mode,\n"
        );
    }

    #[test]
    fn a_declaration_that_never_closes_is_dropped() {
        assert!(functions("fn broken(a: u8\n").is_empty());
    }

    #[test]
    fn non_functions_are_not_offered() {
        let text = "struct Document {\n}\nconst MAX: usize = 4;\n";
        assert!(functions(text).is_empty());
    }

    #[test]
    fn c_declarations_are_found() {
        let text = "int main(int argc, char **argv) {\n    return 0;\n}\n";
        assert_eq!(functions(text)[0].name, "main");
        assert_eq!(functions(text)[0].params, "int argc, char **argv");
    }

    #[test]
    fn params_of_finds_the_declaration() {
        let text = "fn alpha() {}\nfn beta(x: u8, y: u8) {}\n";
        assert_eq!(params_of(text, "beta"), Some("x: u8, y: u8".to_string()));
    }

    #[test]
    fn params_of_an_unknown_name_is_none() {
        assert_eq!(params_of("fn alpha() {}", "beta"), None);
    }

    #[test]
    fn the_first_declaration_wins() {
        let text = "fn alpha(a: u8) {}\nfn alpha(a: u8, b: u8) {}\n";
        assert_eq!(params_of(text, "alpha"), Some("a: u8".to_string()));
    }

    #[test]
    fn a_call_is_found_from_inside_its_arguments() {
        let text = "save(path, mode)";
        assert_eq!(
            call_at(text, 6),
            Some(Call {
                name: "save".to_string(),
                arg: 0
            })
        );
    }

    #[test]
    fn the_argument_index_follows_the_commas() {
        let text = "save(path, mode)";
        assert_eq!(call_at(text, 12).map(|c| c.arg), Some(1));
    }

    #[test]
    fn a_nested_call_reports_the_inner_one() {
        let text = "outer(inner(x))";
        assert_eq!(call_at(text, 12).map(|c| c.name), Some("inner".to_string()));
    }

    #[test]
    fn a_comma_inside_a_nested_call_does_not_count() {
        let text = "outer(inner(a, b), c)";
        assert_eq!(call_at(text, 19).map(|c| c.arg), Some(1));
    }

    #[test]
    fn a_comma_inside_an_array_does_not_count() {
        let text = "outer([1, 2], c)";
        assert_eq!(call_at(text, 14).map(|c| c.arg), Some(1));
    }

    #[test]
    fn outside_a_call_there_is_nothing() {
        assert_eq!(call_at("let x = 1;", 10), None);
    }

    #[test]
    fn a_statement_boundary_stops_the_walk() {
        assert_eq!(call_at("foo(); bar", 10), None);
    }

    #[test]
    fn a_call_with_no_name_is_not_a_call() {
        assert_eq!(call_at("(1 + 2)", 6), None);
    }

    #[test]
    fn an_argument_range_is_the_text_between_commas() {
        assert_eq!(arg_range("a: u8, b: u8", 1), Some(7..12));
    }

    #[test]
    fn the_first_argument_starts_at_zero() {
        assert_eq!(arg_range("a: u8, b: u8", 0), Some(0..5));
    }

    #[test]
    fn an_argument_range_is_trimmed() {
        assert_eq!(arg_range("a ,  b", 1), Some(5..6));
    }

    #[test]
    fn a_comma_inside_a_nested_call_does_not_split() {
        assert_eq!(arg_range("f(a, b), c", 1), Some(9..10));
    }

    #[test]
    fn an_argument_past_the_end_is_none() {
        assert_eq!(arg_range("a, b", 2), None);
    }

    #[test]
    fn an_empty_parameter_list_has_an_empty_first_argument() {
        assert_eq!(arg_range("", 0), Some(0..0));
    }

    #[test]
    fn a_path_fragment_is_found() {
        let text = "open(\"src/main.rs\")";
        assert_eq!(path_prefix(text, 14), Some(6..14));
    }

    #[test]
    fn a_bare_word_is_not_a_path() {
        assert_eq!(path_prefix("open(main", 9), None);
    }

    #[test]
    fn a_comment_is_not_a_path() {
        assert_eq!(path_prefix("//src/main", 10), None);
    }

    #[test]
    fn a_path_stops_at_a_quote() {
        let text = "open(\"a/b\"";
        assert_eq!(path_prefix(text, 9), Some(6..9));
    }
}
