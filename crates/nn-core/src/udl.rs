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

//! User-defined languages: Notepad++'s `userDefineLang.xml`.
//!
//! A user-defined language is a lexer described by data rather than by code.
//! Notepad++ ships a dialog that writes one, and the file it writes is the
//! format this module reads: a name, a list of extensions, up to two line
//! comments, up to two block comments, up to eight delimiter pairs, a set of
//! operator characters, eight keyword groups, and a colour per role.
//!
//! The point of reading it is that a user who has spent an afternoon defining a
//! language for their log format should not have to define it again. The point
//! of writing it is that the definition survives a reinstall.
//!
//! What is deliberately not implemented: the number formats (`Numbers, prefix1`
//! and friends), the code-folding keywords, and the nesting levels. Numbers are
//! recognised generically, which covers every language whose numbers look like
//! numbers; folding is driven by brackets here rather than by the language's own
//! open/close words; and nesting is a Scintilla feature this editor's lexer does
//! not have. Each is a real gap and each is named rather than approximated.

use std::collections::HashMap;
use std::path::Path;

use crate::document::Style;
use crate::lexer::{style, LexState, Lexer};
use crate::xml::{attribute, elements, unescape, Element};

/// A colour as a Notepad++ style writes it: six hex digits, `RRGGBB`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb8 {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Reads `RRGGBB`, ignoring case.
    ///
    /// Exactly six digits: Notepad++ writes no `#` and no alpha, and accepting
    /// a short form would mean guessing which end was truncated.
    pub fn from_hex(text: &str) -> Option<Rgb8> {
        let text = text.trim();
        if text.len() != 6 || !text.is_ascii() {
            return None;
        }
        let byte = |at: usize| u8::from_str_radix(&text[at..at + 2], 16).ok();
        Some(Rgb8 {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
        })
    }

    /// Perceived brightness, 0 for black and 255 for white.
    ///
    /// The Rec. 601 weights rather than a plain average, because the eye is far
    /// more sensitive to green than to blue: an average calls a saturated blue
    /// as bright as a yellow, and the whole point of this number is deciding
    /// whether a colour is readable on a background.
    pub fn luminance(self) -> u16 {
        // The weights are three digits, so the sum reaches 255_000 and needs
        // more room than the result does.
        let weighted =
            299 * self.r as u32 + 587 * self.g as u32 + 114 * self.b as u32;
        (weighted / 1000) as u16
    }
}

/// The style slots a user-defined language can colour.
///
/// These index [`Udl::colours`], not the editor's style space. The mapping from
/// a slot to a style index happens in the lexer, because a language with three
/// keyword groups should colour them with the first three keyword styles rather
/// than with whichever three the file happened to name.
pub mod slot {
    /// Ordinary text.
    pub const DEFAULT: usize = 0;
    /// Block comments.
    pub const COMMENT: usize = 1;
    /// Line comments.
    pub const LINE_COMMENT: usize = 2;
    /// Numeric literals.
    pub const NUMBER: usize = 3;
    /// The first of the eight keyword groups.
    pub const KEYWORD1: usize = 4;
    /// Operators and punctuation.
    pub const OPERATOR: usize = 12;
    /// The first of the eight delimiter pairs.
    pub const DELIMITER1: usize = 13;
    /// One past the last slot.
    pub const COUNT: usize = 21;
}

/// A language defined by data.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Udl {
    /// The name shown in the Language menu.
    pub name: String,
    /// The file extensions it claims, lowercased and without the dot.
    pub extensions: Vec<String>,
    /// Whether keywords are matched case-sensitively.
    pub case_sensitive: bool,
    /// Up to two markers that comment out the rest of a line.
    pub line_comments: Vec<String>,
    /// Up to two open/close pairs that comment out a span.
    pub block_comments: Vec<(String, String)>,
    /// Up to eight open/close pairs that mark a string-like span.
    pub delimiters: Vec<(String, String)>,
    /// The characters drawn as punctuation.
    pub operators: Vec<char>,
    /// The eight keyword groups.
    pub keywords: [Vec<String>; 8],
    /// One colour per [`slot`], `None` where the file named none.
    pub colours: [Option<Rgb8>; slot::COUNT],
}

impl Udl {
    /// Every `<UserLang>` in `text`, in file order.
    pub fn parse(text: &str) -> Vec<Udl> {
        elements(text, "UserLang")
            .iter()
            .map(Udl::parse_one)
            .collect()
    }

    /// One `<UserLang>` element.
    fn parse_one(element: &Element) -> Udl {
        let mut udl = Udl {
            name: attribute(element, "name").unwrap_or("").to_string(),
            extensions: attribute(element, "ext")
                .map(|ext| {
                    ext.split_whitespace()
                        .map(|one| one.trim_start_matches('.').to_ascii_lowercase())
                        .filter(|one| !one.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            // Notepad++'s own default is to respect case, and a language that
            // wants otherwise says so.
            case_sensitive: true,
            ..Udl::default()
        };

        for global in elements(element.body, "Global") {
            if let Some(ignored) = attribute(&global, "caseIgnored") {
                udl.case_sensitive = !ignored.eq_ignore_ascii_case("yes");
            }
        }

        for list in elements(element.body, "Keywords") {
            let Some(name) = attribute(&list, "name") else {
                continue;
            };
            let body = unescape(list.body);
            match name {
                "Comments" => udl.read_comments(&body),
                "Operators" => {
                    udl.operators = body.chars().filter(|c| !c.is_whitespace()).collect();
                }
                "Delimiters" => udl.read_delimiters(&body),
                _ => {
                    if let Some(group) = keyword_group(name) {
                        udl.keywords[group] = body
                            .split_whitespace()
                            .map(|word| word.to_string())
                            .collect();
                    }
                }
            }
        }

        for words_style in elements(element.body, "WordsStyle") {
            let Some(name) = attribute(&words_style, "name") else {
                continue;
            };
            let Some(slot) = style_slot(name) else {
                continue;
            };
            if let Some(colour) = attribute(&words_style, "fgColor").and_then(Rgb8::from_hex) {
                udl.colours[slot] = Some(colour);
            }
        }

        udl
    }

    /// Reads the six `Comments` slots.
    ///
    /// The encoding is two decimal digits naming the slot, then the text:
    /// `00// 01 02/* 03*/`. Slots 0 and 1 are line comments, 2 and 3 are the
    /// first block comment's open and close, 4 and 5 the second's.
    fn read_comments(&mut self, body: &str) {
        let slots = slot_list(body);
        for index in [0, 1] {
            let text = slot_text(&slots, index);
            if !text.is_empty() {
                self.line_comments.push(text);
            }
        }
        for index in [2, 4] {
            let open = slot_text(&slots, index);
            if !open.is_empty() {
                self.block_comments.push((open, slot_text(&slots, index + 1)));
            }
        }
    }

    /// Reads the `Delimiters` slots.
    ///
    /// Twelve pairs of slots, of which Notepad++'s dialog offers eight. Reading
    /// all twelve costs nothing and means a file written by a build that offers
    /// more still loads.
    fn read_delimiters(&mut self, body: &str) {
        let slots = slot_list(body);
        for pair in 0..12 {
            let open = slot_text(&slots, pair * 2);
            if !open.is_empty() {
                self.delimiters
                    .push((open, slot_text(&slots, pair * 2 + 1)));
            }
        }
    }

    /// A lexer driven by this language.
    pub fn lexer(&self) -> UdlLexer {
        UdlLexer::new(self.clone())
    }

    /// The colour for a slot, if the file named one.
    pub fn colour(&self, slot: usize) -> Option<Rgb8> {
        self.colours.get(slot).copied().flatten()
    }

    /// Whether `extension` (without the dot) is one of this language's.
    pub fn claims(&self, extension: &str) -> bool {
        let extension = extension.trim_start_matches('.').to_ascii_lowercase();
        self.extensions.contains(&extension)
    }
}

/// The text of one numbered slot, or an empty string.
fn slot_text(slots: &[(usize, String)], index: usize) -> String {
    slots
        .iter()
        .find(|(at, _)| *at == index)
        .map(|(_, text)| text.clone())
        .unwrap_or_default()
}

/// The `NNtext` entries in a keyword list.
///
/// The two-digit prefix is the slot number. A token shorter than that, or one
/// whose prefix is not a number, is skipped rather than guessed at: a
/// mis-numbered slot would put a comment marker in the wrong place, which is
/// worse than dropping it.
fn slot_list(body: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for token in body.split_whitespace() {
        if token.len() < 2 || !token.is_char_boundary(2) {
            continue;
        }
        let (head, tail) = token.split_at(2);
        let Ok(index) = head.parse::<usize>() else {
            continue;
        };
        out.push((index, tail.to_string()));
    }
    out
}

/// The keyword group a `KeywordsN` list names, zero-based.
fn keyword_group(name: &str) -> Option<usize> {
    let rest = name.strip_prefix("Keywords")?;
    let number: usize = rest.parse().ok()?;
    if !(1..=8).contains(&number) {
        return None;
    }
    Some(number - 1)
}

/// The colour slot a `WordsStyle` name fills.
fn style_slot(name: &str) -> Option<usize> {
    let upper = name.to_ascii_uppercase();
    let slot = match upper.as_str() {
        "DEFAULT" => slot::DEFAULT,
        "COMMENTS" => slot::COMMENT,
        "LINE COMMENTS" => slot::LINE_COMMENT,
        "NUMBERS" => slot::NUMBER,
        "OPERATORS" => slot::OPERATOR,
        _ => {
            let (rest, first) = upper
                .strip_prefix("KEYWORDS")
                .map(|rest| (rest, slot::KEYWORD1))
                .or_else(|| {
                    upper
                        .strip_prefix("DELIMITERS")
                        .map(|rest| (rest, slot::DELIMITER1))
                })?;
            let number: usize = rest.parse().ok().filter(|n| (1..=8).contains(n))?;
            first + number - 1
        }
    };
    Some(slot)
}

/// One construct a language can open: a block comment or a delimiter pair.
///
/// Held as characters rather than as a string because the lexer compares it
/// against a character slice at every position of every line, and converting on
/// each comparison would allocate once per character.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Construct {
    pub open: Vec<char>,
    pub close: Vec<char>,
    pub style: Style,
}

/// A lexer driven by a [`Udl`].
///
/// The language is owned rather than borrowed because a [`crate::Highlighter`]
/// holds a boxed lexer with no lifetime, and a language is a few hundred words:
/// cloning it once when the language is chosen is cheaper than threading a
/// lifetime through the highlighter, the editor, and every document.
pub struct UdlLexer {
    udl: Udl,
    /// Block comments first, then delimiter pairs. The index is the number the
    /// lexer state carries, so this order must not change after construction.
    constructs: Vec<Construct>,
    /// Indices into `constructs`, longest opening marker first, so `"""` wins
    /// over `"` at the same position.
    order: Vec<usize>,
    line_comments: Vec<Vec<char>>,
    /// Every keyword, mapped to the style of the group it belongs to.
    keywords: HashMap<String, Style>,
}

impl UdlLexer {
    /// Builds a lexer for `udl`.
    pub fn new(udl: Udl) -> Self {
        let mut constructs = Vec::new();
        for (open, close) in &udl.block_comments {
            constructs.push(Construct {
                open: open.chars().collect(),
                close: close.chars().collect(),
                style: style::COMMENT,
            });
        }
        for (index, (open, close)) in udl.delimiters.iter().enumerate() {
            constructs.push(Construct {
                open: open.chars().collect(),
                close: close.chars().collect(),
                style: style::UDL_DELIMITER1 + index.min(7) as Style,
            });
        }
        let mut order: Vec<usize> = (0..constructs.len()).collect();
        order.sort_by_key(|&index| std::cmp::Reverse(constructs[index].open.len()));

        let mut keywords = HashMap::new();
        for (group, words) in udl.keywords.iter().enumerate() {
            for word in words {
                let key = if udl.case_sensitive {
                    word.clone()
                } else {
                    word.to_lowercase()
                };
                keywords.insert(key, style::UDL_KEYWORD1 + group as Style);
            }
        }

        Self {
            line_comments: udl
                .line_comments
                .iter()
                .map(|marker| marker.chars().collect())
                .collect(),
            udl,
            constructs,
            order,
            keywords,
        }
    }

    /// The language this lexer was built from.
    pub fn udl(&self) -> &Udl {
        &self.udl
    }

    /// The construct opening at `at`, with the length of its opening marker.
    fn construct_at(&self, chars: &[char], at: usize) -> Option<(usize, usize)> {
        for &index in &self.order {
            let open = &self.constructs[index].open;
            if !open.is_empty() && chars[at..].starts_with(open) {
                return Some((index, open.len()));
            }
        }
        None
    }

    /// Whether a line comment starts at `at`.
    fn line_comment_at(&self, chars: &[char], at: usize) -> bool {
        self.line_comments
            .iter()
            .any(|marker| !marker.is_empty() && chars[at..].starts_with(marker))
    }

    /// The style for `word`, if it is one of the language's keywords.
    fn keyword_style(&self, word: &str) -> Option<Style> {
        if self.udl.case_sensitive {
            self.keywords.get(word).copied()
        } else {
            self.keywords.get(&word.to_lowercase()).copied()
        }
    }
}

impl Lexer for UdlLexer {
    fn lex_line(&mut self, text: &str, state: LexState) -> (Vec<Style>, LexState) {
        let chars: Vec<char> = text.chars().collect();
        let mut styles = vec![style::DEFAULT; chars.len()];
        let mut i = 0;

        // Finish whatever the previous line left open.
        if let LexState::Udl(number) = state {
            if let Some(construct) = self.constructs.get(number as usize) {
                let style = construct.style;
                let close = construct.close.clone();
                match find_close(&chars, 0, &close) {
                    Some(end) => {
                        styles[..end].fill(style);
                        i = end;
                    }
                    None => {
                        styles.fill(style);
                        return (styles, LexState::Udl(number));
                    }
                }
            }
        }

        while i < chars.len() {
            if let Some((index, open_len)) = self.construct_at(&chars, i) {
                let style = self.constructs[index].style;
                let close = self.constructs[index].close.clone();
                // A construct with no close marker runs to the end of the line,
                // which is what Notepad++ means by an empty close.
                if close.is_empty() {
                    styles[i..].fill(style);
                    return (styles, LexState::Normal);
                }
                match find_close(&chars, i + open_len, &close) {
                    Some(end) => {
                        styles[i..end].fill(style);
                        i = end;
                    }
                    None => {
                        styles[i..].fill(style);
                        return (styles, LexState::Udl(index as u8));
                    }
                }
                continue;
            }

            if self.line_comment_at(&chars, i) {
                styles[i..].fill(style::COMMENT);
                return (styles, LexState::Normal);
            }

            let c = chars[i];
            if c.is_ascii_digit() {
                let mut j = i;
                while j < chars.len()
                    && (chars[j].is_alphanumeric() || chars[j] == '.' || chars[j] == '_')
                {
                    j += 1;
                }
                styles[i..j].fill(style::NUMBER);
                i = j;
                continue;
            }
            if c.is_alphabetic() || c == '_' {
                let mut j = i;
                while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                let word: String = chars[i..j].iter().collect();
                if let Some(style) = self.keyword_style(&word) {
                    styles[i..j].fill(style);
                }
                i = j;
                continue;
            }
            if self.udl.operators.contains(&c) {
                styles[i] = style::OPERATOR;
            }
            i += 1;
        }

        (styles, LexState::Normal)
    }
}

/// The index just past `close`, searching from `from`.
///
/// A backslash escapes the character after it, so a delimiter can contain its
/// own close marker. An empty `close` never matches: the caller treats that as
/// "to the end of the line" rather than as a zero-length match, which would
/// loop forever.
fn find_close(chars: &[char], from: usize, close: &[char]) -> Option<usize> {
    if close.is_empty() {
        return None;
    }
    let mut j = from;
    while j < chars.len() {
        if chars[j] == '\\' {
            j += 2;
            continue;
        }
        if chars[j..].starts_with(close) {
            return Some(j + close.len());
        }
        j += 1;
    }
    None
}

/// The user-defined languages an installation has.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UdlSet {
    langs: Vec<Udl>,
}

impl UdlSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.langs.is_empty()
    }

    pub fn len(&self) -> usize {
        self.langs.len()
    }

    pub fn get(&self, index: usize) -> Option<&Udl> {
        self.langs.get(index)
    }

    /// Every language, in file order.
    pub fn langs(&self) -> &[Udl] {
        &self.langs
    }

    /// The names, in file order.
    pub fn names(&self) -> Vec<&str> {
        self.langs.iter().map(|udl| udl.name.as_str()).collect()
    }

    /// The language claiming `path`'s extension, if any.
    ///
    /// The first match wins, so a language defined earlier in the file takes
    /// precedence over one defined later. That is the order the user sees in
    /// Notepad++'s own list, and picking the last would make the visible order
    /// a lie.
    pub fn by_extension(&self, path: &Path) -> Option<usize> {
        let extension = path.extension()?.to_str()?;
        self.langs.iter().position(|udl| udl.claims(extension))
    }

    /// The language with this name, ignoring case.
    pub fn by_name(&self, name: &str) -> Option<usize> {
        self.langs
            .iter()
            .position(|udl| udl.name.eq_ignore_ascii_case(name))
    }

    /// Adds `udl`, replacing a language of the same name in place.
    ///
    /// Replacing rather than appending is what makes re-importing a language
    /// after editing it in Notepad++ update it instead of leaving two copies
    /// with the same name in the menu.
    pub fn add(&mut self, udl: Udl) -> usize {
        if let Some(index) = self.by_name(&udl.name) {
            self.langs[index] = udl;
            return index;
        }
        self.langs.push(udl);
        self.langs.len() - 1
    }

    /// Reads every `<UserLang>` in `text`.
    pub fn parse(text: &str) -> UdlSet {
        UdlSet {
            langs: Udl::parse(text),
        }
    }

    /// Writes the set back as a `userDefineLang.xml`.
    ///
    /// The output is the format Notepad++ reads, so an imported language can be
    /// edited here and taken back. Only what this module models is written: the
    /// number formats and folding keywords are absent because they were never
    /// read, and writing an empty element for them would claim otherwise.
    pub fn render(&self) -> String {
        let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n");
        out.push_str("<NotepadPlus>\n");
        for udl in &self.langs {
            out.push_str(&format!(
                "    <UserLang name=\"{}\" ext=\"{}\" udlVersion=\"2.1\">\n",
                crate::xml::escape(&udl.name),
                crate::xml::escape(&udl.extensions.join(" ")),
            ));
            out.push_str("        <Settings>\n");
            out.push_str(&format!(
                "            <Global caseIgnored=\"{}\" />\n",
                if udl.case_sensitive { "no" } else { "yes" }
            ));
            out.push_str("        </Settings>\n");
            out.push_str("        <KeywordLists>\n");

            let mut comments = String::new();
            for (index, marker) in udl.line_comments.iter().take(2).enumerate() {
                comments.push_str(&format!("{index:02}{marker} "));
            }
            for (index, (open, close)) in udl.block_comments.iter().take(2).enumerate() {
                comments.push_str(&format!("{:02}{open} {:02}{close} ", 2 + index * 2, 3 + index * 2));
            }
            out.push_str(&format!(
                "            <Keywords name=\"Comments\">{}</Keywords>\n",
                crate::xml::escape(comments.trim_end())
            ));

            let operators: String = udl
                .operators
                .iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(" ");
            out.push_str(&format!(
                "            <Keywords name=\"Operators\">{}</Keywords>\n",
                crate::xml::escape(&operators)
            ));

            let mut delimiters = String::new();
            for (index, (open, close)) in udl.delimiters.iter().take(12).enumerate() {
                delimiters.push_str(&format!("{:02}{open} {:02}{close} ", index * 2, index * 2 + 1));
            }
            out.push_str(&format!(
                "            <Keywords name=\"Delimiters\">{}</Keywords>\n",
                crate::xml::escape(delimiters.trim_end())
            ));

            for (group, words) in udl.keywords.iter().enumerate() {
                if words.is_empty() {
                    continue;
                }
                out.push_str(&format!(
                    "            <Keywords name=\"Keywords{}\">{}</Keywords>\n",
                    group + 1,
                    crate::xml::escape(&words.join(" "))
                ));
            }
            out.push_str("        </KeywordLists>\n");

            out.push_str("        <Styles>\n");
            for (index, colour) in udl.colours.iter().enumerate() {
                let Some(colour) = colour else {
                    continue;
                };
                out.push_str(&format!(
                    "            <WordsStyle name=\"{}\" fgColor=\"{:02X}{:02X}{:02X}\" />\n",
                    slot_name(index),
                    colour.r,
                    colour.g,
                    colour.b,
                ));
            }
            out.push_str("        </Styles>\n");
            out.push_str("    </UserLang>\n");
        }
        out.push_str("</NotepadPlus>\n");
        out
    }
}

/// The `WordsStyle` name for a slot, the inverse of [`style_slot`].
fn slot_name(slot: usize) -> String {
    match slot {
        slot::DEFAULT => "DEFAULT".into(),
        slot::COMMENT => "COMMENTS".into(),
        slot::LINE_COMMENT => "LINE COMMENTS".into(),
        slot::NUMBER => "NUMBERS".into(),
        slot::OPERATOR => "OPERATORS".into(),
        other if (slot::KEYWORD1..slot::KEYWORD1 + 8).contains(&other) => {
            format!("KEYWORDS{}", other - slot::KEYWORD1 + 1)
        }
        other if (slot::DELIMITER1..slot::DELIMITER1 + 8).contains(&other) => {
            format!("DELIMITERS{}", other - slot::DELIMITER1 + 1)
        }
        _ => "DEFAULT".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8" ?>
<NotepadPlus>
    <UserLang name="Logfile" ext="log txt" udlVersion="2.1">
        <Settings>
            <Global caseIgnored="yes" />
        </Settings>
        <KeywordLists>
            <Keywords name="Comments">00# 01 02/* 03*/ 04 05</Keywords>
            <Keywords name="Operators">- + = &gt; &lt; [ ]</Keywords>
            <Keywords name="Delimiters">00&quot; 01&quot; 02' 03' 04 05 06 07</Keywords>
            <Keywords name="Keywords1">ERROR WARN</Keywords>
            <Keywords name="Keywords2">started stopped</Keywords>
        </KeywordLists>
        <Styles>
            <WordsStyle name="DEFAULT" fgColor="D4D4D4" />
            <WordsStyle name="COMMENTS" fgColor="6A9955" />
            <WordsStyle name="KEYWORDS1" fgColor="F44747" />
            <WordsStyle name="DELIMITERS1" fgColor="CE9178" />
        </Styles>
    </UserLang>
</NotepadPlus>
"#;

    fn sample() -> Udl {
        Udl::parse(SAMPLE).remove(0)
    }

    fn lex(udl: &Udl, text: &str) -> Vec<Style> {
        udl.lexer().lex_line(text, LexState::Normal).0
    }

    #[test]
    fn a_language_carries_its_name_and_extensions() {
        let udl = sample();
        assert_eq!(udl.name, "Logfile");
        assert_eq!(udl.extensions, vec!["log", "txt"]);
        assert!(udl.claims("LOG"));
        assert!(udl.claims(".txt"));
        assert!(!udl.claims("rs"));
    }

    #[test]
    fn case_ignored_flips_the_case_sensitivity() {
        assert!(!sample().case_sensitive);
        let text = SAMPLE.replace(r#"caseIgnored="yes""#, r#"caseIgnored="no""#);
        assert!(Udl::parse(&text).remove(0).case_sensitive);
    }

    #[test]
    fn a_language_with_no_global_element_respects_case() {
        let text = r#"<UserLang name="X" ext="x"><KeywordLists /></UserLang>"#;
        assert!(Udl::parse(text).remove(0).case_sensitive);
    }

    #[test]
    fn the_comment_slots_split_into_line_and_block_markers() {
        let udl = sample();
        assert_eq!(udl.line_comments, vec!["#"]);
        assert_eq!(udl.block_comments, vec![("/*".to_string(), "*/".to_string())]);
    }

    #[test]
    fn a_second_line_comment_is_read() {
        let text = r#"<UserLang name="X" ext="x"><KeywordLists>
            <Keywords name="Comments">00// 01-- 02 03 04 05</Keywords>
        </KeywordLists></UserLang>"#;
        let udl = Udl::parse(text).remove(0);
        assert_eq!(udl.line_comments, vec!["//", "--"]);
        assert!(udl.block_comments.is_empty());
    }

    #[test]
    fn a_second_block_comment_is_read() {
        let text = r#"<UserLang name="X" ext="x"><KeywordLists>
            <Keywords name="Comments">00 01 02/* 03*/ 04&lt;!-- 05--&gt;</Keywords>
        </KeywordLists></UserLang>"#;
        let udl = Udl::parse(text).remove(0);
        assert_eq!(
            udl.block_comments,
            vec![
                ("/*".to_string(), "*/".to_string()),
                ("<!--".to_string(), "-->".to_string()),
            ]
        );
    }

    #[test]
    fn the_operator_list_is_unescaped() {
        let udl = sample();
        assert!(udl.operators.contains(&'>'));
        assert!(udl.operators.contains(&'<'));
        assert!(udl.operators.contains(&'='));
        assert!(!udl.operators.contains(&' '));
    }

    #[test]
    fn the_delimiter_slots_pair_up() {
        let udl = sample();
        assert_eq!(
            udl.delimiters,
            vec![
                ("\"".to_string(), "\"".to_string()),
                ("'".to_string(), "'".to_string()),
            ]
        );
    }

    #[test]
    fn the_keyword_groups_are_separate() {
        let udl = sample();
        assert_eq!(udl.keywords[0], vec!["ERROR", "WARN"]);
        assert_eq!(udl.keywords[1], vec!["started", "stopped"]);
        assert!(udl.keywords[2].is_empty());
    }

    #[test]
    fn the_colours_land_in_their_slots() {
        let udl = sample();
        assert_eq!(udl.colour(slot::DEFAULT), Some(Rgb8::new(0xd4, 0xd4, 0xd4)));
        assert_eq!(udl.colour(slot::COMMENT), Some(Rgb8::new(0x6a, 0x99, 0x55)));
        assert_eq!(udl.colour(slot::KEYWORD1), Some(Rgb8::new(0xf4, 0x47, 0x47)));
        assert_eq!(udl.colour(slot::DELIMITER1), Some(Rgb8::new(0xce, 0x91, 0x78)));
        assert_eq!(udl.colour(slot::NUMBER), None);
    }

    #[test]
    fn a_hex_colour_needs_exactly_six_digits() {
        assert_eq!(Rgb8::from_hex("FFFFFF"), Some(Rgb8::new(255, 255, 255)));
        assert_eq!(Rgb8::from_hex("00ff00"), Some(Rgb8::new(0, 255, 0)));
        assert_eq!(Rgb8::from_hex("FFF"), None);
        assert_eq!(Rgb8::from_hex("#FFFFFF"), None);
        assert_eq!(Rgb8::from_hex("GGGGGG"), None);
        assert_eq!(Rgb8::from_hex(""), None);
    }

    #[test]
    fn luminance_weights_green_above_blue() {
        assert_eq!(Rgb8::new(0, 0, 0).luminance(), 0);
        assert_eq!(Rgb8::new(255, 255, 255).luminance(), 255);
        assert!(Rgb8::new(0, 255, 0).luminance() > Rgb8::new(0, 0, 255).luminance());
    }

    #[test]
    fn keywords_are_coloured_by_their_group() {
        let udl = sample();
        let styles = lex(&udl, "ERROR started");
        assert_eq!(styles[0], style::UDL_KEYWORD1);
        assert_eq!(styles[6], style::UDL_KEYWORD2);
    }

    #[test]
    fn keywords_ignore_case_when_the_language_does() {
        let udl = sample();
        assert_eq!(lex(&udl, "error")[0], style::UDL_KEYWORD1);
        let text = SAMPLE.replace(r#"caseIgnored="yes""#, r#"caseIgnored="no""#);
        let strict = Udl::parse(&text).remove(0);
        assert_eq!(lex(&strict, "error")[0], style::DEFAULT);
        assert_eq!(lex(&strict, "ERROR")[0], style::UDL_KEYWORD1);
    }

    #[test]
    fn a_line_comment_runs_to_the_end() {
        let udl = sample();
        let styles = lex(&udl, "x = 1 # ERROR");
        assert_eq!(styles[0], style::DEFAULT);
        assert!(styles[6..].iter().all(|&s| s == style::COMMENT));
    }

    #[test]
    fn a_block_comment_closes_on_the_same_line() {
        let udl = sample();
        let styles = lex(&udl, "a /* b */ c");
        assert_eq!(styles[0], style::DEFAULT);
        assert_eq!(styles[2], style::COMMENT);
        assert_eq!(styles[8], style::COMMENT);
        assert_eq!(styles[10], style::DEFAULT);
    }

    #[test]
    fn a_block_comment_carries_across_lines() {
        let udl = sample();
        let mut lexer = udl.lexer();
        let (styles, state) = lexer.lex_line("a /* b", LexState::Normal);
        assert_eq!(state, LexState::Udl(0));
        assert_eq!(styles[2], style::COMMENT);

        let (styles, state) = lexer.lex_line("still", state);
        assert_eq!(state, LexState::Udl(0));
        assert!(styles.iter().all(|&s| s == style::COMMENT));

        let (styles, state) = lexer.lex_line("end */ x", state);
        assert_eq!(state, LexState::Normal);
        assert_eq!(styles[5], style::COMMENT);
        assert_eq!(styles[7], style::DEFAULT);
    }

    #[test]
    fn a_delimiter_is_coloured_by_its_pair() {
        let udl = sample();
        let styles = lex(&udl, r#"a "b" c"#);
        assert_eq!(styles[2], style::UDL_DELIMITER1);
        assert_eq!(styles[4], style::UDL_DELIMITER1);
        let styles = lex(&udl, "a 'b' c");
        assert_eq!(styles[2], style::UDL_DELIMITER2);
    }

    #[test]
    fn a_delimiter_carries_across_lines() {
        let udl = sample();
        let mut lexer = udl.lexer();
        let (styles, state) = lexer.lex_line(r#"s = "abc"#, LexState::Normal);
        // Construct 0 is the block comment, so the first delimiter pair is 1.
        assert_eq!(state, LexState::Udl(1));
        assert_eq!(styles[4], style::UDL_DELIMITER1);

        let (styles, state) = lexer.lex_line(r#"def""#, state);
        assert_eq!(state, LexState::Normal);
        assert!(styles.iter().all(|&s| s == style::UDL_DELIMITER1));
    }

    #[test]
    fn a_delimiter_beats_a_line_comment_at_the_same_position() {
        // `#` is the comment marker and `"` a delimiter, so a `#` inside a
        // string is part of the string.
        let udl = sample();
        let styles = lex(&udl, r#""a # b" c"#);
        assert!(styles[..7].iter().all(|&s| s == style::UDL_DELIMITER1));
        assert_eq!(styles[8], style::DEFAULT);
    }

    #[test]
    fn a_longer_opening_marker_wins() {
        let text = r#"<UserLang name="X" ext="x"><KeywordLists>
            <Keywords name="Delimiters">00""" 01""" 02" 03"</Keywords>
        </KeywordLists></UserLang>"#;
        let udl = Udl::parse(text).remove(0);
        let styles = lex(&udl, r#"a """b""" c"#);
        // The triple quote is construct 0, the single quote construct 1.
        assert_eq!(styles[2], style::UDL_DELIMITER1);
        assert_eq!(styles[4], style::UDL_DELIMITER1);
        assert_eq!(styles[7], style::UDL_DELIMITER1);
    }

    #[test]
    fn a_delimiter_with_no_close_runs_to_the_end_of_the_line() {
        let text = r#"<UserLang name="X" ext="x"><KeywordLists>
            <Keywords name="Delimiters">00; 01</Keywords>
        </KeywordLists></UserLang>"#;
        let udl = Udl::parse(text).remove(0);
        let mut lexer = udl.lexer();
        let (styles, state) = lexer.lex_line("a ; b", LexState::Normal);
        assert_eq!(state, LexState::Normal);
        assert!(styles[2..].iter().all(|&s| s == style::UDL_DELIMITER1));
    }

    #[test]
    fn a_backslash_escapes_the_close_marker() {
        let udl = sample();
        let styles = lex(&udl, r#""a\"b" c"#);
        assert_eq!(styles[5], style::UDL_DELIMITER1);
        assert_eq!(styles[7], style::DEFAULT);
    }

    #[test]
    fn numbers_are_recognised() {
        let udl = sample();
        let styles = lex(&udl, "x = 42 + 3.5");
        assert_eq!(styles[4], style::NUMBER);
        assert_eq!(styles[5], style::NUMBER);
        assert_eq!(styles[10], style::NUMBER);
    }

    #[test]
    fn operators_are_recognised() {
        let udl = sample();
        let styles = lex(&udl, "a+b");
        assert_eq!(styles[1], style::OPERATOR);
        assert_eq!(styles[0], style::DEFAULT);
    }

    #[test]
    fn a_character_outside_every_list_is_plain() {
        let udl = sample();
        let styles = lex(&udl, "a~b");
        assert_eq!(styles[1], style::DEFAULT);
    }

    #[test]
    fn the_style_count_matches_the_character_count() {
        let udl = sample();
        for text in ["", "a", "héllo", "ERROR # x", r#""a b" c"#] {
            assert_eq!(lex(&udl, text).len(), text.chars().count(), "for {text:?}");
        }
    }

    #[test]
    fn an_empty_line_is_normal_state() {
        let udl = sample();
        let (styles, state) = udl.lexer().lex_line("", LexState::Normal);
        assert!(styles.is_empty());
        assert_eq!(state, LexState::Normal);
    }

    #[test]
    fn a_language_with_nothing_defined_styles_nothing() {
        let udl = Udl::parse(r#"<UserLang name="X" ext="x" />"#).remove(0);
        let styles = lex(&udl, "fn main() { x }");
        assert!(styles.iter().all(|&s| s == style::DEFAULT));
    }

    #[test]
    fn several_languages_are_read_in_file_order() {
        let text = r#"<NotepadPlus>
            <UserLang name="A" ext="a" />
            <UserLang name="B" ext="b" />
        </NotepadPlus>"#;
        let set = UdlSet::parse(text);
        assert_eq!(set.len(), 2);
        assert_eq!(set.names(), vec!["A", "B"]);
    }

    #[test]
    fn a_language_is_found_by_its_extension() {
        let set = UdlSet::parse(SAMPLE);
        assert_eq!(set.by_extension(Path::new("/tmp/a.log")), Some(0));
        assert_eq!(set.by_extension(Path::new("/tmp/a.TXT")), Some(0));
        assert_eq!(set.by_extension(Path::new("/tmp/a.rs")), None);
        assert_eq!(set.by_extension(Path::new("/tmp/a")), None);
    }

    #[test]
    fn the_first_language_claiming_an_extension_wins() {
        let text = r#"<NotepadPlus>
            <UserLang name="A" ext="x" />
            <UserLang name="B" ext="x" />
        </NotepadPlus>"#;
        let set = UdlSet::parse(text);
        assert_eq!(set.by_extension(Path::new("f.x")), Some(0));
    }

    #[test]
    fn a_language_is_found_by_its_name_ignoring_case() {
        let set = UdlSet::parse(SAMPLE);
        assert_eq!(set.by_name("logfile"), Some(0));
        assert_eq!(set.by_name("nope"), None);
    }

    #[test]
    fn adding_a_language_of_the_same_name_replaces_it() {
        let mut set = UdlSet::parse(SAMPLE);
        let mut replacement = sample();
        replacement.name = "Logfile".into();
        replacement.extensions = vec!["lg".into()];
        let index = set.add(replacement);
        assert_eq!(index, 0);
        assert_eq!(set.len(), 1);
        assert_eq!(set.get(0).map(|udl| udl.extensions.clone()), Some(vec!["lg".to_string()]));
    }

    #[test]
    fn adding_a_new_language_appends_it() {
        let mut set = UdlSet::parse(SAMPLE);
        let mut other = sample();
        other.name = "Other".into();
        assert_eq!(set.add(other), 1);
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn a_language_round_trips_through_the_written_file() {
        let set = UdlSet::parse(SAMPLE);
        let text = set.render();
        let back = UdlSet::parse(&text);
        assert_eq!(back, set);
    }

    #[test]
    fn the_written_file_is_what_notepad_plus_expects() {
        let set = UdlSet::parse(SAMPLE);
        let text = set.render();
        assert!(text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n"));
        assert!(text.contains(r#"<UserLang name="Logfile" ext="log txt" udlVersion="2.1">"#));
        assert!(text.contains(r#"<Global caseIgnored="yes" />"#));
        assert!(text.contains(r#"<Keywords name="Comments">00# 02/* 03*/</Keywords>"#));
        assert!(text.contains(r#"<Keywords name="Keywords1">ERROR WARN</Keywords>"#));
        assert!(text.contains(r#"<WordsStyle name="KEYWORDS1" fgColor="F44747" />"#));
        assert!(text.ends_with("</NotepadPlus>\n"));
    }

    #[test]
    fn an_empty_set_writes_an_empty_file() {
        let text = UdlSet::new().render();
        assert!(text.contains("<NotepadPlus>"));
        assert!(!text.contains("<UserLang"));
        assert!(UdlSet::parse(&text).is_empty());
    }

    #[test]
    fn a_slot_name_reads_back_as_its_slot() {
        for index in 0..slot::COUNT {
            assert_eq!(style_slot(&slot_name(index)), Some(index), "for {index}");
        }
    }

    #[test]
    fn a_malformed_slot_entry_is_skipped() {
        let text = r#"<UserLang name="X" ext="x"><KeywordLists>
            <Keywords name="Comments">xx// 0 00#</Keywords>
        </KeywordLists></UserLang>"#;
        let udl = Udl::parse(text).remove(0);
        assert_eq!(udl.line_comments, vec!["#"]);
    }

    #[test]
    fn a_keyword_group_outside_the_range_is_ignored() {
        let text = r#"<UserLang name="X" ext="x"><KeywordLists>
            <Keywords name="Keywords9">a</Keywords>
            <Keywords name="Keywords0">b</Keywords>
        </KeywordLists></UserLang>"#;
        let udl = Udl::parse(text).remove(0);
        assert!(udl.keywords.iter().all(|group| group.is_empty()));
    }
}
