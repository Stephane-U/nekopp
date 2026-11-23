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

//! Line operations: sort, dedupe, trim, case, comment, indentation.
//!
//! Every function here takes the text of a block of whole lines and returns the
//! replacement text. Keeping them free of `Document` means they are tested with
//! string literals, and the editor only has to work out which characters the
//! block covers.
//!
//! A block is parsed into its lines plus two facts that a naive `split('\n')`
//! would lose: whether the block ended with a terminator, and whether that
//! terminator was CRLF. Both are restored on the way out, so a CRLF file stays
//! CRLF and the last line of a document does not silently gain a newline.

/// Which way to sort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    /// A to Z, 0 to 9.
    Ascending,
    /// Z to A, 9 to 0.
    Descending,
}

/// What to compare when sorting.
///
/// Notepad++ offers seven orders in each direction, and they are seven different
/// comparisons rather than one comparison with flags: sorting `10` before `9` is
/// a different question from sorting `a` before `B`, and a user who picks
/// "As Integers" means it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    /// Compare the whole line as text.
    Text,
    /// Compare the first number found in the line, falling back to text.
    Numeric,
    /// Compare the line as text, ignoring case.
    TextIgnoreCase,
    /// Compare the line in locale order.
    ///
    /// The locale is not consulted: this build has no locale database, so the
    /// order is the same as `Text` with case folded. It is a separate variant
    /// because the menu row exists and a user who picks it should get a sort
    /// rather than nothing, and because the day a collation table arrives this
    /// is the one place that changes.
    Locale,
    /// Compare the first integer in the line.
    Integer,
    /// Compare the first decimal number, where the decimal separator is a comma.
    DecimalComma,
    /// Compare the first decimal number, where the decimal separator is a dot.
    DecimalDot,
    /// Compare by line length.
    Length,
}

/// How to sort a block of lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortOptions {
    /// Direction.
    pub order: SortOrder,
    /// Comparison key.
    pub key: SortKey,
    /// Whether `A` and `a` are different.
    pub case_sensitive: bool,
}

impl Default for SortOptions {
    fn default() -> Self {
        Self {
            order: SortOrder::Ascending,
            key: SortKey::Text,
            case_sensitive: true,
        }
    }
}

/// Which case conversion to apply.
///
/// The "force" and "blend" pairs are Notepad++'s: a forced conversion rewrites
/// the letters it is not capitalising, a blended one leaves them as they are. A
/// user converting a heading wants the first; a user fixing a sentence that is
/// already half capitalised wants the second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Case {
    /// EVERYTHING SHOUTING.
    Upper,
    /// everything quiet.
    Lower,
    /// Every Word Capitalised, the rest lowercased.
    Title,
    /// Every Word Capitalised, the rest left alone.
    TitleBlend,
    /// First letter of the line capitalised, the rest lowercased.
    Sentence,
    /// First letter of the line capitalised, the rest left alone.
    SentenceBlend,
    /// sWAP tHE cASE oF eVERY lETTER.
    Invert,
    /// rAndOmLY cASE eVERY lETTER.
    Random,
}

/// A block of lines, split so it can be put back together unchanged.
struct Block<'a> {
    lines: Vec<&'a str>,
    /// The block ended with a line terminator.
    trailing: bool,
    /// The terminator was CRLF rather than LF.
    crlf: bool,
}

impl<'a> Block<'a> {
    fn parse(text: &'a str) -> Self {
        let crlf = text.contains("\r\n");
        let trailing = text.ends_with('\n');
        let body = if trailing {
            &text[..text.len() - 1]
        } else {
            text
        };
        let lines = body
            .split('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line))
            .collect();
        Self {
            lines,
            trailing,
            crlf,
        }
    }

    fn render(&self, lines: &[String]) -> String {
        let sep = if self.crlf { "\r\n" } else { "\n" };
        let mut out = lines.join(sep);
        if self.trailing {
            out.push_str(sep);
        }
        out
    }

    fn owned(&self) -> Vec<String> {
        self.lines.iter().map(|line| (*line).to_string()).collect()
    }
}

/// Sorts the block.
///
/// The sort is stable, so lines that compare equal keep their order. Descending
/// is the ascending result reversed, which keeps that property.
pub fn sort(text: &str, options: SortOptions) -> String {
    let block = Block::parse(text);
    let mut lines = block.owned();
    let key = |line: &str| {
        if options.case_sensitive {
            line.to_string()
        } else {
            line.to_lowercase()
        }
    };
    match options.key {
        SortKey::Text => lines.sort_by_key(|line| key(line)),
        SortKey::TextIgnoreCase | SortKey::Locale => {
            lines.sort_by_key(|line| line.to_lowercase())
        }
        SortKey::Numeric => lines.sort_by(|a, b| {
            leading_number(a)
                .partial_cmp(&leading_number(b))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| key(a).cmp(&key(b)))
        }),
        SortKey::Integer => lines.sort_by(|a, b| {
            leading_integer(a)
                .cmp(&leading_integer(b))
                .then_with(|| key(a).cmp(&key(b)))
        }),
        SortKey::DecimalComma => lines.sort_by(|a, b| {
            leading_decimal(a, ',')
                .partial_cmp(&leading_decimal(b, ','))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| key(a).cmp(&key(b)))
        }),
        SortKey::DecimalDot => lines.sort_by(|a, b| {
            leading_decimal(a, '.')
                .partial_cmp(&leading_decimal(b, '.'))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| key(a).cmp(&key(b)))
        }),
        SortKey::Length => lines.sort_by(|a, b| {
            a.chars()
                .count()
                .cmp(&b.chars().count())
                .then_with(|| key(a).cmp(&key(b)))
        }),
    }
    if options.order == SortOrder::Descending {
        lines.reverse();
    }
    block.render(&lines)
}

/// The first number in `line`, or zero when there is none.
///
/// Walks `char_indices` rather than bytes so a line starting with a multi-byte
/// character cannot produce a slice boundary in the middle of a code point.
fn leading_number(line: &str) -> f64 {
    let mut start = None;
    let mut end = 0;
    for (i, c) in line.char_indices() {
        let numeric = c.is_ascii_digit() || c == '.' || ((c == '-' || c == '+') && start.is_none());
        if numeric {
            if start.is_none() {
                start = Some(i);
            }
            end = i + c.len_utf8();
        } else if start.is_some() {
            break;
        }
    }
    match start {
        Some(from) => line[from..end].parse().unwrap_or(0.0),
        None => 0.0,
    }
}

/// The first integer in `line`, or zero when there is none.
///
/// Separate from [`leading_number`] because an integer sort must not treat the
/// dot in `1.5` as part of the number: `1.5` and `1.9` are the same integer, and
/// a user who asked for an integer sort expects them to stay in file order.
fn leading_integer(line: &str) -> i64 {
    let mut start = None;
    let mut end = 0;
    for (i, c) in line.char_indices() {
        let numeric = c.is_ascii_digit() || ((c == '-' || c == '+') && start.is_none());
        if numeric {
            if start.is_none() {
                start = Some(i);
            }
            end = i + c.len_utf8();
        } else if start.is_some() {
            break;
        }
    }
    match start {
        Some(from) => line[from..end].parse().unwrap_or(0),
        None => 0,
    }
}

/// The first decimal number in `line`, using `separator` as the decimal mark.
///
/// A separator that is not the one asked for is not a decimal mark, so `1,5`
/// sorted as a dot-decimal is the integer `1` followed by text. That is what
/// makes the two menu rows different from each other.
fn leading_decimal(line: &str, separator: char) -> f64 {
    let mut start = None;
    let mut end = 0;
    for (i, c) in line.char_indices() {
        let numeric = c.is_ascii_digit()
            || c == separator
            || ((c == '-' || c == '+') && start.is_none());
        if numeric {
            if start.is_none() {
                start = Some(i);
            }
            end = i + c.len_utf8();
        } else if start.is_some() {
            break;
        }
    }
    match start {
        Some(from) => line[from..end]
            .replace(separator, ".")
            .parse()
            .unwrap_or(0.0),
        None => 0.0,
    }
}

/// Removes lines that repeat the line immediately above them.
pub fn dedupe_consecutive(text: &str) -> String {
    let block = Block::parse(text);
    let mut out: Vec<String> = Vec::with_capacity(block.lines.len());
    for line in &block.lines {
        if out.last().map(String::as_str) != Some(*line) {
            out.push((*line).to_string());
        }
    }
    block.render(&out)
}

/// Removes every line that repeats an earlier one, keeping the first.
pub fn dedupe_all(text: &str) -> String {
    let block = Block::parse(text);
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(block.lines.len());
    for line in &block.lines {
        if seen.insert(*line) {
            out.push((*line).to_string());
        }
    }
    block.render(&out)
}

/// Strips whitespace from the end of every line.
pub fn trim_trailing(text: &str) -> String {
    let block = Block::parse(text);
    let out: Vec<String> = block
        .lines
        .iter()
        .map(|line| line.trim_end().to_string())
        .collect();
    block.render(&out)
}

/// Strips whitespace from the start of every line.
pub fn trim_leading(text: &str) -> String {
    let block = Block::parse(text);
    let out: Vec<String> = block
        .lines
        .iter()
        .map(|line| line.trim_start().to_string())
        .collect();
    block.render(&out)
}

/// Drops lines that are empty.
///
/// A line of spaces is not empty, which is the distinction Notepad++ draws
/// between this and [`remove_empty_with_blank`]: a file indented with trailing
/// whitespace keeps its shape under this one.
pub fn remove_empty(text: &str) -> String {
    let block = Block::parse(text);
    let out: Vec<String> = block
        .lines
        .iter()
        .filter(|line| !line.is_empty())
        .map(|line| (*line).to_string())
        .collect();
    block.render(&out)
}

/// Drops lines that are empty or contain only blank characters.
pub fn remove_empty_with_blank(text: &str) -> String {
    let block = Block::parse(text);
    let out: Vec<String> = block
        .lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| (*line).to_string())
        .collect();
    block.render(&out)
}

/// Reverses the order of the lines.
pub fn reverse_order(text: &str) -> String {
    let block = Block::parse(text);
    let mut lines = block.owned();
    lines.reverse();
    block.render(&lines)
}

/// Shuffles the lines, deterministically for a given `seed`.
///
/// The seed is passed in rather than read from the clock so the shuffle can be
/// tested: a random order that cannot be reproduced is a random order that
/// cannot be checked. The generator is xorshift64, which is four lines and good
/// enough for shuffling a text file.
pub fn randomize_order(text: &str, seed: u64) -> String {
    let block = Block::parse(text);
    let mut lines = block.owned();
    let mut state = seed | 1;
    for i in (1..lines.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let j = (state % (i as u64 + 1)) as usize;
        lines.swap(i, j);
    }
    block.render(&lines)
}

/// Joins every line into one, separated by a single space.
pub fn join(text: &str) -> String {
    let block = Block::parse(text);
    let joined = block
        .lines
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    block.render(&[joined])
}

/// Splits every line at `delimiter`, one trimmed piece per line.
pub fn split(text: &str, delimiter: &str) -> String {
    if delimiter.is_empty() {
        return text.to_string();
    }
    let block = Block::parse(text);
    let mut out = Vec::with_capacity(block.lines.len());
    for line in &block.lines {
        for piece in line.split(delimiter) {
            out.push(piece.trim().to_string());
        }
    }
    block.render(&out)
}

/// Converts the case of every line.
///
/// `Case::Random` needs a seed, so it goes through [`convert_case_seeded`]; this
/// entry point uses a fixed one, which makes the "random" case stable for a given
/// line rather than different on every redraw.
pub fn convert_case(text: &str, case: Case) -> String {
    convert_case_seeded(text, case, 0x9E37_79B9_7F4A_7C15)
}

/// Converts the case of every line, seeding the random conversion.
pub fn convert_case_seeded(text: &str, case: Case, seed: u64) -> String {
    let block = Block::parse(text);
    let mut state = seed | 1;
    let out: Vec<String> = block
        .lines
        .iter()
        .map(|line| match case {
            Case::Upper => line.to_uppercase(),
            Case::Lower => line.to_lowercase(),
            Case::Title => title_case(line),
            Case::TitleBlend => title_case_blend(line),
            Case::Sentence => sentence_case(line, true),
            Case::SentenceBlend => sentence_case(line, false),
            Case::Invert => invert_case(line),
            Case::Random => {
                let mut out = String::with_capacity(line.len());
                for c in line.chars() {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    if state & 1 == 0 {
                        out.extend(c.to_uppercase());
                    } else {
                        out.extend(c.to_lowercase());
                    }
                }
                out
            }
        })
        .collect();
    block.render(&out)
}

/// Capitalises the first letter of every word, leaving the rest alone.
fn title_case_blend(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut at_word_start = true;
    for c in line.chars() {
        if c.is_alphanumeric() {
            if at_word_start {
                out.extend(c.to_uppercase());
            } else {
                out.push(c);
            }
            at_word_start = false;
        } else {
            out.push(c);
            at_word_start = true;
        }
    }
    out
}

/// Capitalises the first letter of the line, optionally lowercasing the rest.
///
/// The first letter is the first alphanumeric character, not the first character:
/// a line starting with a quote or a bullet still has a first letter, and
/// capitalising the quote would do nothing.
fn sentence_case(line: &str, force: bool) -> String {
    let mut out = String::with_capacity(line.len());
    let mut seen_letter = false;
    for c in line.chars() {
        if !seen_letter && c.is_alphanumeric() {
            out.extend(c.to_uppercase());
            seen_letter = true;
        } else if force {
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Swaps the case of every letter.
fn invert_case(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for c in line.chars() {
        if c.is_uppercase() {
            out.extend(c.to_lowercase());
        } else if c.is_lowercase() {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Capitalises the first letter of every word and lowercases the rest.
fn title_case(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut at_word_start = true;
    for c in line.chars() {
        if c.is_alphanumeric() {
            if at_word_start {
                out.extend(c.to_uppercase());
            } else {
                out.extend(c.to_lowercase());
            }
            at_word_start = false;
        } else {
            out.push(c);
            at_word_start = true;
        }
    }
    out
}

/// Adds `prefix` to every line, or removes it when every line already has one.
///
/// Blank lines are left alone in both directions, so a comment block with a gap
/// in it still toggles as a unit.
pub fn toggle_comment(text: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        return text.to_string();
    }
    let block = Block::parse(text);
    let commented = block
        .lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .all(|line| line.trim_start().starts_with(prefix));
    let out: Vec<String> = block
        .lines
        .iter()
        .map(|line| {
            let indent = line.len() - line.trim_start().len();
            if commented {
                match line[indent..].strip_prefix(prefix) {
                    Some(rest) => {
                        let mut s = String::with_capacity(line.len());
                        s.push_str(&line[..indent]);
                        s.push_str(rest.strip_prefix(' ').unwrap_or(rest));
                        s
                    }
                    None => (*line).to_string(),
                }
            } else if line.trim().is_empty() {
                (*line).to_string()
            } else {
                let mut s = String::with_capacity(line.len() + prefix.len() + 1);
                s.push_str(&line[..indent]);
                s.push_str(prefix);
                s.push(' ');
                s.push_str(&line[indent..]);
                s
            }
        })
        .collect();
    block.render(&out)
}

/// Adds `prefix` to every non-blank line, whether or not it is already there.
pub fn single_line_comment(text: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        return text.to_string();
    }
    let block = Block::parse(text);
    let out: Vec<String> = block
        .lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() || line.trim_start().starts_with(prefix) {
                (*line).to_string()
            } else {
                let indent = line.len() - line.trim_start().len();
                format!("{}{prefix} {}", &line[..indent], &line[indent..])
            }
        })
        .collect();
    block.render(&out)
}

/// Removes `prefix` from every line that has one.
pub fn single_line_uncomment(text: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        return text.to_string();
    }
    let block = Block::parse(text);
    let out: Vec<String> = block
        .lines
        .iter()
        .map(|line| {
            let indent = line.len() - line.trim_start().len();
            match line[indent..].strip_prefix(prefix) {
                Some(rest) => {
                    let mut s = String::with_capacity(line.len());
                    s.push_str(&line[..indent]);
                    s.push_str(rest.strip_prefix(' ').unwrap_or(rest));
                    s
                }
                None => (*line).to_string(),
            }
        })
        .collect();
    block.render(&out)
}

/// Wraps the whole block in `open` and `close`.
///
/// The markers go on their own lines around the block rather than at its ends,
/// which is what makes the result a block comment in every language that has
/// one: `/* a */` on one line is a comment, but `/*\na\n*/` is a comment that
/// can be extended by typing.
pub fn block_comment(text: &str, open: &str, close: &str) -> String {
    if open.is_empty() || close.is_empty() || text.is_empty() {
        return text.to_string();
    }
    let block = Block::parse(text);
    let mut lines = block.owned();
    lines.insert(0, open.to_string());
    lines.push(close.to_string());
    block.render(&lines)
}

/// Removes a `open` / `close` pair that wraps the block.
///
/// Only the outermost pair is removed, and only when it is the first and last
/// thing in the block: a block that merely contains a comment is not a commented
/// block, and unwrapping it would delete code.
pub fn block_uncomment(text: &str, open: &str, close: &str) -> String {
    if open.is_empty() || close.is_empty() {
        return text.to_string();
    }
    let block = Block::parse(text);
    let mut lines = block.owned();
    let first = lines.first().map(|line| line.trim().to_string());
    let last = lines.last().map(|line| line.trim().to_string());
    if first.as_deref() == Some(open) && last.as_deref() == Some(close) && lines.len() >= 2 {
        lines.remove(0);
        lines.pop();
    }
    block.render(&lines)
}

/// Strips whitespace from both ends of every line.
pub fn trim_both(text: &str) -> String {
    let block = Block::parse(text);
    let out: Vec<String> = block
        .lines
        .iter()
        .map(|line| line.trim().to_string())
        .collect();
    block.render(&out)
}

/// Replaces the line terminator at the end of every line with a space.
///
/// The block is joined into one line, because that is what removing the
/// terminators means: the text that was on separate lines is now on one.
pub fn eol_to_space(text: &str) -> String {
    let block = Block::parse(text);
    let joined = block.lines.join(" ");
    block.render(&[joined])
}

/// Trims both ends of every line and joins them with spaces.
pub fn trim_all(text: &str) -> String {
    let block = Block::parse(text);
    let joined = block
        .lines
        .iter()
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join(" ");
    block.render(&[joined])
}

/// Replaces every run of `width` spaces with a tab, anywhere in the line.
///
/// The "All" variant of [`spaces_to_tabs`]: it converts alignment inside a line
/// as well as the indent, which is what a user converting a space-indented file
/// to tabs wants and what a user aligning a table does not.
pub fn spaces_to_tabs_all(text: &str, width: usize) -> String {
    if width == 0 {
        return text.to_string();
    }
    let block = Block::parse(text);
    let out: Vec<String> = block
        .lines
        .iter()
        .map(|line| {
            let mut out = String::with_capacity(line.len());
            let mut run = 0;
            for c in line.chars() {
                if c == ' ' {
                    run += 1;
                    if run == width {
                        out.push('\t');
                        run = 0;
                    }
                } else {
                    for _ in 0..run {
                        out.push(' ');
                    }
                    run = 0;
                    out.push(c);
                }
            }
            for _ in 0..run {
                out.push(' ');
            }
            out
        })
        .collect();
    block.render(&out)
}

/// Replaces every tab with `width` spaces.
pub fn tabs_to_spaces(text: &str, width: usize) -> String {
    if width == 0 {
        return text.to_string();
    }
    let block = Block::parse(text);
    let pad = " ".repeat(width);
    let out: Vec<String> = block
        .lines
        .iter()
        .map(|line| line.replace('\t', &pad))
        .collect();
    block.render(&out)
}

/// Replaces leading runs of `width` spaces with a tab.
///
/// Only the indent is converted: turning spaces inside a line into tabs would
/// shift text that was aligned by hand.
pub fn spaces_to_tabs(text: &str, width: usize) -> String {
    if width == 0 {
        return text.to_string();
    }
    let block = Block::parse(text);
    let out: Vec<String> = block
        .lines
        .iter()
        .map(|line| {
            let indent = line.len() - line.trim_start_matches(' ').len();
            let mut s = String::with_capacity(line.len());
            for _ in 0..indent / width {
                s.push('\t');
            }
            for _ in 0..indent % width {
                s.push(' ');
            }
            s.push_str(&line[indent..]);
            s
        })
        .collect();
    block.render(&out)
}

/// Adds one tab of indentation to every non-empty line.
///
/// Blank lines are left alone: indenting them would leave trailing whitespace
/// that the next `Trim Trailing Space` would only have to remove again.
pub fn indent(text: &str) -> String {
    let block = Block::parse(text);
    let out: Vec<String> = block
        .lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                (*line).to_string()
            } else {
                format!("\t{line}")
            }
        })
        .collect();
    block.render(&out)
}

/// Removes one tab, or up to one tab width of spaces, from every line.
pub fn unindent(text: &str, width: usize) -> String {
    let block = Block::parse(text);
    let out: Vec<String> = block
        .lines
        .iter()
        .map(|line| {
            if let Some(rest) = line.strip_prefix('\t') {
                return rest.to_string();
            }
            let spaces = line.chars().take(width).take_while(|&c| c == ' ').count();
            line[spaces..].to_string()
        })
        .collect();
    block.render(&out)
}

/// Repeats the whole block below itself.
pub fn duplicate(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let block = Block::parse(text);
    let mut lines = block.owned();
    let copy = lines.clone();
    lines.extend(copy);
    block.render(&lines)
}

/// Moves the first line of the block to the end, or the last line to the front.
///
/// The caller widens the block by one line in the direction of travel, so this
/// only has to rotate.
pub fn shift(text: &str, up: bool) -> String {
    let block = Block::parse(text);
    let mut lines = block.owned();
    if lines.len() < 2 {
        return text.to_string();
    }
    if up {
        let head = lines.remove(0);
        lines.push(head);
    } else {
        let tail = lines.pop().unwrap_or_default();
        lines.insert(0, tail);
    }
    block.render(&lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asc() -> SortOptions {
        SortOptions::default()
    }

    #[test]
    fn sort_ascending_text() {
        assert_eq!(sort("c\na\nb", asc()), "a\nb\nc");
    }

    #[test]
    fn sort_descending_text() {
        let opts = SortOptions {
            order: SortOrder::Descending,
            ..asc()
        };
        assert_eq!(sort("c\na\nb", opts), "c\nb\na");
    }

    #[test]
    fn sort_is_case_insensitive_when_asked() {
        let opts = SortOptions {
            case_sensitive: false,
            ..asc()
        };
        assert_eq!(sort("b\nA\nc", opts), "A\nb\nc");
    }

    #[test]
    fn sort_numeric_reads_the_first_number() {
        let opts = SortOptions {
            key: SortKey::Numeric,
            ..asc()
        };
        assert_eq!(
            sort("item 10\nitem 2\nitem 1", opts),
            "item 1\nitem 2\nitem 10"
        );
    }

    #[test]
    fn sort_numeric_handles_negative_and_missing() {
        let opts = SortOptions {
            key: SortKey::Numeric,
            ..asc()
        };
        assert_eq!(sort("x\n-3\n5", opts), "-3\nx\n5");
    }

    #[test]
    fn sort_keeps_the_trailing_newline() {
        assert_eq!(sort("b\na\n", asc()), "a\nb\n");
    }

    #[test]
    fn sort_keeps_crlf() {
        assert_eq!(sort("b\r\na\r\n", asc()), "a\r\nb\r\n");
    }

    #[test]
    fn sort_is_stable() {
        let opts = SortOptions {
            case_sensitive: false,
            ..asc()
        };
        assert_eq!(sort("b\nB\na", opts), "a\nb\nB");
    }

    #[test]
    fn dedupe_consecutive_only_touches_neighbours() {
        assert_eq!(dedupe_consecutive("a\na\nb\na"), "a\nb\na");
    }

    #[test]
    fn dedupe_all_keeps_the_first() {
        assert_eq!(dedupe_all("a\nb\na\nb\nc"), "a\nb\nc");
    }

    #[test]
    fn trim_trailing_strips_spaces_and_tabs() {
        assert_eq!(trim_trailing("a  \nb\t\nc"), "a\nb\nc");
    }

    #[test]
    fn trim_leading_strips_indent() {
        assert_eq!(trim_leading("  a\n\tb"), "a\nb");
    }

    #[test]
    fn remove_empty_drops_only_truly_empty_lines() {
        assert_eq!(remove_empty("a\n\n  \nb"), "a\n  \nb");
    }

    #[test]
    fn remove_empty_with_blank_drops_whitespace_lines_too() {
        assert_eq!(remove_empty_with_blank("a\n\n  \nb"), "a\nb");
    }

    #[test]
    fn reverse_order_turns_the_block_around() {
        assert_eq!(reverse_order("a\nb\nc\n"), "c\nb\na\n");
    }

    #[test]
    fn reverse_order_keeps_a_missing_final_newline_missing() {
        assert_eq!(reverse_order("a\nb"), "b\na");
    }

    #[test]
    fn randomize_order_keeps_every_line() {
        let text = "a\nb\nc\nd\ne\nf\ng\nh\n";
        let shuffled = randomize_order(text, 42);
        let mut before: Vec<&str> = text.lines().collect();
        let mut after: Vec<&str> = shuffled.lines().collect();
        before.sort_unstable();
        after.sort_unstable();
        assert_eq!(before, after);
    }

    #[test]
    fn randomize_order_is_reproducible_for_a_seed() {
        let text = "a\nb\nc\nd\ne\nf\ng\nh\n";
        assert_eq!(randomize_order(text, 7), randomize_order(text, 7));
    }

    #[test]
    fn randomize_order_actually_moves_something() {
        let text = "a\nb\nc\nd\ne\nf\ng\nh\n";
        assert_ne!(randomize_order(text, 7), text);
    }

    #[test]
    fn sorting_as_integers_puts_nine_before_ten() {
        let options = SortOptions {
            order: SortOrder::Ascending,
            key: SortKey::Integer,
            case_sensitive: true,
        };
        assert_eq!(sort("10\n9\n1\n", options), "1\n9\n10\n");
    }

    #[test]
    fn sorting_as_integers_ignores_a_decimal_point() {
        // Both lines are the integer 1, so both sort before the integer 2 even
        // though `1.9` is the larger number.
        let options = SortOptions {
            order: SortOrder::Ascending,
            key: SortKey::Integer,
            case_sensitive: true,
        };
        assert_eq!(sort("2\n1.9\n1.5\n", options), "1.5\n1.9\n2\n");
    }

    #[test]
    fn sorting_as_dot_decimals_reads_the_fraction() {
        let options = SortOptions {
            order: SortOrder::Ascending,
            key: SortKey::DecimalDot,
            case_sensitive: true,
        };
        assert_eq!(sort("1.9\n1.5\n", options), "1.5\n1.9\n");
    }

    #[test]
    fn sorting_as_comma_decimals_reads_the_fraction() {
        let options = SortOptions {
            order: SortOrder::Ascending,
            key: SortKey::DecimalComma,
            case_sensitive: true,
        };
        assert_eq!(sort("1,9\n1,5\n", options), "1,5\n1,9\n");
    }

    #[test]
    fn a_comma_decimal_is_not_a_dot_decimal() {
        // Under a dot sort `1,9` is the integer 1, so it sorts before `1.5`.
        let options = SortOptions {
            order: SortOrder::Ascending,
            key: SortKey::DecimalDot,
            case_sensitive: true,
        };
        assert_eq!(sort("1.5\n1,9\n", options), "1,9\n1.5\n");
    }

    #[test]
    fn sorting_by_length_puts_the_shortest_first() {
        let options = SortOptions {
            order: SortOrder::Ascending,
            key: SortKey::Length,
            case_sensitive: true,
        };
        assert_eq!(sort("ccc\na\nbb\n", options), "a\nbb\nccc\n");
    }

    #[test]
    fn sorting_ignoring_case_puts_upper_and_lower_together() {
        let options = SortOptions {
            order: SortOrder::Ascending,
            key: SortKey::TextIgnoreCase,
            case_sensitive: true,
        };
        assert_eq!(sort("b\nA\na\nB\n", options), "A\na\nb\nB\n");
    }

    #[test]
    fn sorting_descending_reverses_the_ascending_result() {
        let options = SortOptions {
            order: SortOrder::Descending,
            key: SortKey::Integer,
            case_sensitive: true,
        };
        assert_eq!(sort("1\n10\n9\n", options), "10\n9\n1\n");
    }

    #[test]
    fn proper_case_blend_leaves_the_rest_of_the_word_alone() {
        assert_eq!(
            convert_case("hello WORLD", Case::TitleBlend),
            "Hello WORLD"
        );
    }

    #[test]
    fn proper_case_force_lowercases_the_rest_of_the_word() {
        assert_eq!(convert_case("hello WORLD", Case::Title), "Hello World");
    }

    #[test]
    fn sentence_case_capitalises_the_first_letter_only() {
        assert_eq!(
            convert_case("the QUICK brown fox", Case::Sentence),
            "The quick brown fox"
        );
    }

    #[test]
    fn sentence_case_blend_leaves_the_rest_alone() {
        assert_eq!(
            convert_case("the QUICK brown fox", Case::SentenceBlend),
            "The QUICK brown fox"
        );
    }

    #[test]
    fn sentence_case_skips_leading_punctuation() {
        assert_eq!(convert_case("\"quoted\"", Case::Sentence), "\"Quoted\"");
    }

    #[test]
    fn invert_case_swaps_every_letter() {
        assert_eq!(convert_case("Hello World", Case::Invert), "hELLO wORLD");
    }

    #[test]
    fn random_case_keeps_the_letters_and_their_count() {
        let out = convert_case("hello world", Case::Random);
        assert_eq!(out.to_lowercase(), "hello world");
        assert_eq!(out.chars().count(), 11);
    }

    #[test]
    fn random_case_is_reproducible_for_a_seed() {
        let a = convert_case_seeded("hello world", Case::Random, 5);
        let b = convert_case_seeded("hello world", Case::Random, 5);
        assert_eq!(a, b);
    }

    #[test]
    fn single_line_comment_adds_the_prefix_once() {
        assert_eq!(single_line_comment("a\nb", "//"), "// a\n// b");
        assert_eq!(single_line_comment("// a", "//"), "// a");
    }

    #[test]
    fn single_line_comment_keeps_the_indent() {
        assert_eq!(single_line_comment("  a", "//"), "  // a");
    }

    #[test]
    fn single_line_uncomment_removes_the_prefix_and_one_space() {
        assert_eq!(single_line_uncomment("// a\n//b", "//"), "a\nb");
    }

    #[test]
    fn single_line_uncomment_leaves_a_plain_line_alone() {
        assert_eq!(single_line_uncomment("a", "//"), "a");
    }

    #[test]
    fn block_comment_wraps_the_block_on_its_own_lines() {
        assert_eq!(block_comment("a\nb", "/*", "*/"), "/*\na\nb\n*/");
    }

    #[test]
    fn block_uncomment_unwraps_a_wrapped_block() {
        assert_eq!(block_uncomment("/*\na\nb\n*/", "/*", "*/"), "a\nb");
    }

    #[test]
    fn block_uncomment_leaves_a_block_that_is_not_wrapped_alone() {
        assert_eq!(block_uncomment("a\nb", "/*", "*/"), "a\nb");
    }

    #[test]
    fn block_uncomment_does_not_eat_a_single_line() {
        // One line that happens to be both markers is not a wrapped block.
        assert_eq!(block_uncomment("/*", "/*", "*/"), "/*");
    }

    #[test]
    fn trim_both_strips_each_end() {
        assert_eq!(trim_both("  a  \n\tb\t"), "a\nb");
    }

    #[test]
    fn eol_to_space_joins_the_lines() {
        assert_eq!(eol_to_space("a\nb\nc"), "a b c");
    }

    #[test]
    fn trim_all_trims_and_joins() {
        assert_eq!(trim_all("  a  \n  b  "), "a b");
    }

    #[test]
    fn spaces_to_tabs_all_converts_inside_the_line() {
        assert_eq!(spaces_to_tabs_all("a    b", 4), "a\tb");
    }

    #[test]
    fn spaces_to_tabs_all_leaves_a_short_run_alone() {
        assert_eq!(spaces_to_tabs_all("a  b", 4), "a  b");
    }

    #[test]
    fn spaces_to_tabs_leading_leaves_the_middle_alone() {
        assert_eq!(spaces_to_tabs("    a    b", 4), "\ta    b");
    }

    #[test]
    fn join_collapses_to_one_line() {
        assert_eq!(join("a\n  b  \n\nc"), "a b c");
    }

    #[test]
    fn split_breaks_on_the_delimiter() {
        assert_eq!(split("a,b\nc,d", ","), "a\nb\nc\nd");
    }

    #[test]
    fn split_with_empty_delimiter_is_a_no_op() {
        assert_eq!(split("a,b", ""), "a,b");
    }

    #[test]
    fn case_conversions() {
        assert_eq!(convert_case("aB", Case::Upper), "AB");
        assert_eq!(convert_case("aB", Case::Lower), "ab");
        assert_eq!(convert_case("hello world", Case::Title), "Hello World");
    }

    #[test]
    fn title_case_keeps_punctuation() {
        assert_eq!(convert_case("o'neill mc-x", Case::Title), "O'Neill Mc-X");
    }

    #[test]
    fn comment_adds_then_removes() {
        let added = toggle_comment("a\n  b", "//");
        assert_eq!(added, "// a\n  // b");
        assert_eq!(toggle_comment(&added, "//"), "a\n  b");
    }

    #[test]
    fn comment_leaves_blank_lines_alone() {
        assert_eq!(toggle_comment("a\n\nb", "//"), "// a\n\n// b");
    }

    #[test]
    fn comment_removes_from_a_mixed_block() {
        assert_eq!(toggle_comment("// a\n// b", "//"), "a\nb");
    }

    #[test]
    fn tabs_to_spaces_expands_every_tab() {
        assert_eq!(tabs_to_spaces("\ta\tb", 4), "    a    b");
    }

    #[test]
    fn spaces_to_tabs_only_touches_the_indent() {
        assert_eq!(spaces_to_tabs("        a  b", 4), "\t\ta  b");
    }

    #[test]
    fn spaces_to_tabs_keeps_a_partial_indent() {
        assert_eq!(spaces_to_tabs("      a", 4), "\t  a");
    }

    #[test]
    fn duplicate_repeats_the_block() {
        assert_eq!(duplicate("a\nb"), "a\nb\na\nb");
    }

    #[test]
    fn duplicate_keeps_the_trailing_newline() {
        assert_eq!(duplicate("a\n"), "a\na\n");
    }

    #[test]
    fn shift_up_rotates_the_first_line_to_the_end() {
        assert_eq!(shift("a\nb\nc", true), "b\nc\na");
    }

    #[test]
    fn shift_down_rotates_the_last_line_to_the_front() {
        assert_eq!(shift("a\nb\nc", false), "c\na\nb");
    }

    #[test]
    fn shift_of_one_line_is_a_no_op() {
        assert_eq!(shift("a", true), "a");
    }

    #[test]
    fn empty_block_round_trips() {
        assert_eq!(sort("", asc()), "");
        assert_eq!(trim_trailing(""), "");
        assert_eq!(duplicate(""), "");
    }

    #[test]
    fn single_line_without_terminator_round_trips() {
        assert_eq!(trim_trailing("a  "), "a");
        assert_eq!(convert_case("a", Case::Upper), "A");
    }

    #[test]
    fn indent_adds_one_tab_per_line() {
        assert_eq!(indent("a\nb"), "\ta\n\tb");
    }

    #[test]
    fn indent_leaves_blank_lines_alone() {
        assert_eq!(indent("a\n\nb"), "\ta\n\n\tb");
    }

    #[test]
    fn indent_keeps_a_trailing_terminator() {
        assert_eq!(indent("a\n"), "\ta\n");
    }

    #[test]
    fn unindent_removes_one_tab() {
        assert_eq!(unindent("\ta\n\tb", 4), "a\nb");
    }

    #[test]
    fn unindent_removes_up_to_a_tab_width_of_spaces() {
        assert_eq!(unindent("    a\n  b\nc", 4), "a\nb\nc");
    }

    #[test]
    fn unindent_stops_at_the_first_non_space() {
        assert_eq!(unindent("  x  ", 4), "x  ");
    }
}
