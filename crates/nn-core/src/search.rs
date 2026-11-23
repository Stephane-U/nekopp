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

//! Finding and replacing.
//!
//! The document addresses text by character index, but every fast string search
//! in Rust works on bytes. Rather than convert the whole document up front — a
//! four-byte-per-character allocation on every keystroke in the find bar — the
//! matches are found in bytes and translated to characters in a single forward
//! pass, which is O(n) regardless of how many matches there are.
//!
//! A [`Search`] is compiled once and reused. Compiling a regex per keystroke
//! would dominate the cost of searching a small file.

use std::ops::Range;

use crate::document::Document;

/// How to match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SearchOptions {
    /// Distinguish upper from lower case.
    pub case_sensitive: bool,
    /// Only match when the hit is not part of a longer word.
    pub whole_word: bool,
    /// How the pattern itself is read.
    pub mode: SearchMode,
    /// In [`SearchMode::Regex`], let `.` match a newline.
    ///
    /// Off is the default, and it is Notepad++'s ". matches newline" checkbox:
    /// with it off a pattern cannot cross a line ending by accident, which is
    /// what a search almost always wants.
    pub dot_matches_newline: bool,
}

/// How a pattern is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchMode {
    /// Literal text, `*` and `\` being nothing but characters.
    #[default]
    Normal,
    /// Literal text with escapes: `\n`, `\r`, `\t`, `\0`, `\\`, `\xHH` and
    /// `\uHHHH`. Notepad++ calls this Extended, and it is how a pattern ever
    /// contains a tab or a line ending without being a regular expression.
    Extended,
    /// A regular expression.
    Regex,
}

/// Why a pattern could not be compiled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchError(pub String);

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SearchError {}

/// A compiled pattern.
pub struct Search {
    matcher: Matcher,
    whole_word: bool,
}

enum Matcher {
    Literal {
        needle: String,
        case_sensitive: bool,
    },
    Regex(regex::Regex),
}

impl Search {
    /// Compiles `pattern`.
    ///
    /// An empty pattern compiles to a matcher that never matches, which keeps
    /// callers from special-casing the empty find field.
    pub fn new(pattern: &str, options: SearchOptions) -> Result<Self, SearchError> {
        let matcher = match options.mode {
            SearchMode::Regex if !pattern.is_empty() => {
                let re = regex::RegexBuilder::new(pattern)
                    .case_insensitive(!options.case_sensitive)
                    .dot_matches_new_line(options.dot_matches_newline)
                    .build()
                    .map_err(|e| SearchError(e.to_string()))?;
                Matcher::Regex(re)
            }
            // An empty pattern compiles to a matcher that never matches, which
            // keeps callers from special-casing the empty find field.
            SearchMode::Regex => Matcher::Literal {
                needle: String::new(),
                case_sensitive: true,
            },
            SearchMode::Extended => Matcher::Literal {
                needle: unescape(pattern),
                case_sensitive: options.case_sensitive,
            },
            SearchMode::Normal => Matcher::Literal {
                needle: pattern.to_string(),
                case_sensitive: options.case_sensitive,
            },
        };
        Ok(Self {
            matcher,
            whole_word: options.whole_word,
        })
    }

    /// True if the pattern can never match.
    pub fn is_empty(&self) -> bool {
        match &self.matcher {
            Matcher::Literal { needle, .. } => needle.is_empty(),
            Matcher::Regex(_) => false,
        }
    }

    /// Every match in `doc`, in order, as character ranges.
    pub fn find_all(&self, doc: &Document) -> Vec<Range<usize>> {
        let text = doc.text();
        let mut mapper = ByteToChar::new(&text);
        self.find_in_text(&text)
            .into_iter()
            .map(|range| mapper.at(range.start)..mapper.at(range.end))
            .collect()
    }

    /// Every match in `text`, as byte ranges.
    ///
    /// Find-in-files searches files it has no `Document` for, so it needs the
    /// matcher without the character-index translation. `find_all` is this plus
    /// the translation, which keeps the two from drifting apart.
    pub fn find_in_text(&self, text: &str) -> Vec<Range<usize>> {
        if self.is_empty() {
            return Vec::new();
        }
        self.byte_matches(text)
            .into_iter()
            .filter(|&(start, end)| !self.whole_word || is_whole_word(text, start, end))
            .map(|(start, end)| start..end)
            .collect()
    }

    /// The first match starting at or after `from`.
    pub fn find_next(&self, doc: &Document, from: usize) -> Option<Range<usize>> {
        self.find_all(doc).into_iter().find(|m| m.start >= from)
    }

    /// The last match ending at or before `from`.
    pub fn find_prev(&self, doc: &Document, from: usize) -> Option<Range<usize>> {
        self.find_all(doc).into_iter().rfind(|m| m.end <= from)
    }

    /// The match containing `pos`, if any.
    pub fn find_at(&self, doc: &Document, pos: usize) -> Option<Range<usize>> {
        self.find_all(doc)
            .into_iter()
            .find(|m| m.start <= pos && pos < m.end)
    }

    /// The byte ranges of every match in `text`.
    fn byte_matches(&self, text: &str) -> Vec<(usize, usize)> {
        match &self.matcher {
            Matcher::Literal {
                needle,
                case_sensitive,
            } => literal_matches(text, needle, *case_sensitive),
            Matcher::Regex(re) => re.find_iter(text).map(|m| (m.start(), m.end())).collect(),
        }
    }

    /// The replacement text for the match at `range`, expanding `$1` groups when
    /// the pattern is a regex.
    pub fn expand(&self, doc: &Document, range: Range<usize>, replacement: &str) -> String {
        let Matcher::Regex(re) = &self.matcher else {
            return replacement.to_string();
        };
        let text = doc.text();
        let mut mapper = ByteToChar::new(&text);
        let start = mapper.byte_at(range.start);
        let end = mapper.byte_at(range.end);
        let Some(caps) = re.captures_at(&text, start) else {
            return replacement.to_string();
        };
        // `captures_at` may find a later match; only expand if it is ours.
        if caps.get(0).map(|m| (m.start(), m.end())) != Some((start, end)) {
            return replacement.to_string();
        }
        let mut out = String::new();
        caps.expand(replacement, &mut out);
        out
    }
}

/// Reads the escapes Notepad++ reads in Extended mode.
///
/// `\n`, `\r`, `\t`, `\0`, `\\`, `\xHH` and `\uHHHH` are translated; anything
/// else is left as the two characters it was written with, because a pattern is
/// a user's text and guessing at an escape it did not mean is worse than leaving
/// it to match literally. `\xHH` is a character code, not a byte: `\xE9` is `é`,
/// which is what a user typing a Latin-1 code point expects.
fn unescape(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('0') => out.push('\0'),
            Some('\\') => out.push('\\'),
            Some(letter @ ('x' | 'u')) => {
                let digits = if letter == 'x' { 2 } else { 4 };
                let hex: String = chars.by_ref().take(digits).collect();
                let code = if hex.chars().count() == digits
                    && hex.chars().all(|c| c.is_ascii_hexdigit())
                {
                    u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
                } else {
                    None
                };
                match code {
                    Some(c) => out.push(c),
                    None => push_literal(&mut out, letter, &hex),
                }
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Puts an escape that did not parse back the way it was written.
fn push_literal(out: &mut String, letter: char, digits: &str) {
    out.push('\\');
    out.push(letter);
    out.push_str(digits);
}

/// Finds `needle` in `haystack`, returning byte ranges.
fn literal_matches(haystack: &str, needle: &str, case_sensitive: bool) -> Vec<(usize, usize)> {
    if needle.is_empty() {
        return Vec::new();
    }
    if case_sensitive {
        return haystack
            .match_indices(needle)
            .map(|(i, m)| (i, i + m.len()))
            .collect();
    }

    // Lowercasing can change a string's byte length, which would invalidate
    // every offset. ASCII lowering never does, so take that path whenever it is
    // available and fall back to an exact match otherwise.
    if haystack.is_ascii() && needle.is_ascii() {
        let hay = haystack.to_ascii_lowercase();
        let pin = needle.to_ascii_lowercase();
        return hay
            .match_indices(&pin)
            .map(|(i, m)| (i, i + m.len()))
            .collect();
    }

    let hay = haystack.to_lowercase();
    let pin = needle.to_lowercase();
    if hay.len() != haystack.len() {
        return literal_matches(haystack, needle, true);
    }
    hay.match_indices(&pin)
        .map(|(i, m)| (i, i + m.len()))
        .collect()
}

/// True if the match at `start..end` is not part of a longer word.
fn is_whole_word(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Translates byte offsets to character offsets in one forward pass.
///
/// Offsets must be requested in non-decreasing order, which holds because
/// matches come out of the search sorted.
struct ByteToChar<'a> {
    iter: std::str::CharIndices<'a>,
    byte: usize,
    char_idx: usize,
}

impl<'a> ByteToChar<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            iter: text.char_indices(),
            byte: 0,
            char_idx: 0,
        }
    }

    /// The character index of the character starting at `target`.
    fn at(&mut self, target: usize) -> usize {
        while self.byte < target {
            match self.iter.next() {
                Some((b, c)) => {
                    self.byte = b + c.len_utf8();
                    self.char_idx += 1;
                }
                None => break,
            }
        }
        self.char_idx
    }

    /// The byte offset of the character at `target`.
    fn byte_at(&mut self, target: usize) -> usize {
        while self.char_idx < target {
            match self.iter.next() {
                Some((b, c)) => {
                    self.byte = b + c.len_utf8();
                    self.char_idx += 1;
                }
                None => break,
            }
        }
        self.byte
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Document {
        Document::from_text(text)
    }

    fn search(pattern: &str, options: SearchOptions) -> Search {
        Search::new(pattern, options).expect("compile")
    }

    fn plain(pattern: &str) -> Search {
        search(
            pattern,
            SearchOptions {
                case_sensitive: true,
                ..Default::default()
            },
        )
    }

    #[test]
    fn finds_every_occurrence() {
        let d = doc("foo bar foo baz foo");
        let hits = plain("foo").find_all(&d);
        assert_eq!(hits, vec![0..3, 8..11, 16..19]);
    }

    #[test]
    fn empty_pattern_matches_nothing() {
        let d = doc("anything");
        assert!(plain("").find_all(&d).is_empty());
        assert!(plain("").is_empty());
    }

    #[test]
    fn case_insensitive_finds_both_cases() {
        let d = doc("Foo foo FOO");
        let s = search(
            "foo",
            SearchOptions {
                case_sensitive: false,
                ..Default::default()
            },
        );
        assert_eq!(s.find_all(&d).len(), 3);
    }

    #[test]
    fn case_sensitive_skips_other_cases() {
        let d = doc("Foo foo FOO");
        assert_eq!(plain("foo").find_all(&d), vec![4..7]);
    }

    #[test]
    fn whole_word_rejects_substrings() {
        let d = doc("foo foobar barfoo foo");
        let s = search(
            "foo",
            SearchOptions {
                case_sensitive: true,
                whole_word: true,
                ..Default::default()
            },
        );
        assert_eq!(s.find_all(&d), vec![0..3, 18..21]);
    }

    #[test]
    fn whole_word_treats_underscore_as_a_word_char() {
        let d = doc("foo foo_bar");
        let s = search(
            "foo",
            SearchOptions {
                case_sensitive: true,
                whole_word: true,
                ..Default::default()
            },
        );
        assert_eq!(s.find_all(&d), vec![0..3]);
    }

    #[test]
    fn regex_matches() {
        let d = doc("let x = 1;\nlet y = 22;");
        let s = search(
            r"\d+",
            SearchOptions {
                mode: SearchMode::Regex,
                ..Default::default()
            },
        );
        assert_eq!(s.find_all(&d), vec![8..9, 19..21]);
    }

    #[test]
    fn regex_is_case_insensitive_when_asked() {
        let d = doc("Foo foo");
        let s = search(
            "foo",
            SearchOptions {
                mode: SearchMode::Regex,
                case_sensitive: false,
                ..Default::default()
            },
        );
        assert_eq!(s.find_all(&d).len(), 2);
    }

    #[test]
    fn invalid_regex_is_an_error() {
        assert!(Search::new(
            "(",
            SearchOptions {
                mode: SearchMode::Regex,
                ..Default::default()
            }
        )
        .is_err());
    }

    fn extended(pattern: &str) -> Search {
        search(
            pattern,
            SearchOptions {
                case_sensitive: true,
                mode: SearchMode::Extended,
                ..Default::default()
            },
        )
    }

    #[test]
    fn extended_reads_tab_and_newline_escapes() {
        let d = doc("a\tb\ncd");
        assert_eq!(extended(r"\t").find_all(&d), vec![1..2]);
        assert_eq!(extended(r"b\nc").find_all(&d), vec![2..5]);
    }

    #[test]
    fn extended_reads_a_hex_code_point() {
        let d = doc("caf\u{e9} \u{20ac}");
        assert_eq!(extended(r"caf\xE9").find_all(&d), vec![0..4]);
        assert_eq!(extended(r"\u20AC").find_all(&d), vec![5..6]);
    }

    #[test]
    fn extended_leaves_unknown_escapes_alone() {
        // A backslash a user did not mean as an escape has to stay one, or a
        // Windows path would be found only under some other name.
        for pattern in [r"\d", r"a\xZZ", r"a\u12", "trailing \\"] {
            assert_eq!(unescape(pattern), pattern);
        }
        assert_eq!(unescape(r"\x41"), "A");
        assert_eq!(unescape(r"\\"), "\\");
    }

    #[test]
    fn extended_still_honours_whole_word() {
        let d = doc("foo foobar foo");
        let s = search(
            "foo",
            SearchOptions {
                case_sensitive: true,
                whole_word: true,
                mode: SearchMode::Extended,
                ..Default::default()
            },
        );
        assert_eq!(s.find_all(&d), vec![0..3, 11..14]);
    }

    #[test]
    fn normal_mode_treats_escapes_as_text() {
        // Notepad++'s Normal mode is literal: a pattern with a backslash finds
        // the backslash, which is what makes it the safe default.
        let d = doc(r"a\n b");
        assert_eq!(plain(r"\n").find_all(&d), vec![1..3]);
    }

    #[test]
    fn a_dot_does_not_cross_a_line_break_by_default() {
        let d = doc("ab\ncd");
        let s = search(
            "a.+d",
            SearchOptions {
                mode: SearchMode::Regex,
                ..Default::default()
            },
        );
        assert!(s.find_all(&d).is_empty());
    }

    #[test]
    fn a_dot_crosses_a_line_break_when_asked() {
        let d = doc("ab\ncd");
        let s = search(
            "a.+d",
            SearchOptions {
                mode: SearchMode::Regex,
                dot_matches_newline: true,
                ..Default::default()
            },
        );
        assert_eq!(s.find_all(&d), vec![0..5]);
    }

    #[test]
    fn find_next_starts_at_the_offset() {
        let d = doc("foo foo foo");
        assert_eq!(plain("foo").find_next(&d, 1), Some(4..7));
        assert_eq!(plain("foo").find_next(&d, 4), Some(4..7));
        assert_eq!(plain("foo").find_next(&d, 5), Some(8..11));
        assert_eq!(plain("foo").find_next(&d, 9), None);
    }

    #[test]
    fn find_prev_ends_at_the_offset() {
        let d = doc("foo foo foo");
        assert_eq!(plain("foo").find_prev(&d, 11), Some(8..11));
        assert_eq!(plain("foo").find_prev(&d, 10), Some(4..7));
        assert_eq!(plain("foo").find_prev(&d, 3), Some(0..3));
        assert_eq!(plain("foo").find_prev(&d, 2), None);
    }

    #[test]
    fn find_at_covers_the_position() {
        let d = doc("foo foo");
        assert_eq!(plain("foo").find_at(&d, 0), Some(0..3));
        assert_eq!(plain("foo").find_at(&d, 2), Some(0..3));
        assert_eq!(plain("foo").find_at(&d, 3), None);
    }

    #[test]
    fn offsets_are_characters_not_bytes() {
        // Each of these is multi-byte in UTF-8, so a byte offset would be wrong.
        let d = doc("héllo wörld héllo");
        let hits = plain("héllo").find_all(&d);
        assert_eq!(hits, vec![0..5, 12..17]);
    }

    #[test]
    fn matches_across_a_line_break() {
        let d = doc("one\ntwo");
        assert_eq!(plain("e\nt").find_all(&d), vec![2..5]);
    }

    #[test]
    fn expand_substitutes_capture_groups() {
        let d = doc("let x = 1;");
        let s = search(
            r"(\w+) = (\d+)",
            SearchOptions {
                mode: SearchMode::Regex,
                ..Default::default()
            },
        );
        let hit = s.find_all(&d)[0].clone();
        assert_eq!(s.expand(&d, hit, "$2 = $1"), "1 = x");
    }

    #[test]
    fn expand_without_regex_returns_the_literal() {
        let d = doc("abc");
        let hit = plain("abc").find_all(&d)[0].clone();
        assert_eq!(plain("abc").expand(&d, hit, "$1"), "$1");
    }

    #[test]
    fn byte_to_char_handles_multibyte() {
        let text = "aé€𝄞b";
        let mut m = ByteToChar::new(text);
        assert_eq!(m.at(0), 0);
        assert_eq!(m.at(1), 1); // é
        assert_eq!(m.at(3), 2); // €
        assert_eq!(m.at(6), 3); // 𝄞
        assert_eq!(m.at(10), 4); // b
    }
}
