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

//! Syntax highlighting.
//!
//! A lexer turns one line of text into one style byte per character. It is
//! stateful because constructs span lines: a block comment opened on line 3 is
//! still a comment on line 400. The state is carried in and out of `lex_line`,
//! which is what lets the highlighter resume from any line without re-reading
//! the file from the top.
//!
//! The lexer is deliberately not a parser. It does not build a tree, resolve
//! types, or understand scope. It recognises the handful of token classes that
//! carry colour, which is all highlighting needs and all it can afford at
//! keystroke latency.

use crate::document::Style;
use crate::lang::Apostrophe;

/// The style indices the default theme knows how to colour.
pub mod style {
    use crate::document::Style;

    /// Unstyled text.
    pub const DEFAULT: Style = 0;
    /// Language keywords: `fn`, `if`, `return`.
    pub const KEYWORD: Style = 1;
    /// Built-in type names: `u32`, `String`, `Vec`.
    pub const TYPE: Style = 2;
    /// String and character literals.
    pub const STRING: Style = 3;
    /// Line and block comments.
    pub const COMMENT: Style = 4;
    /// Numeric literals.
    pub const NUMBER: Style = 5;
    /// An identifier immediately followed by `(`.
    pub const FUNCTION: Style = 6;
    /// Punctuation and operators.
    pub const OPERATOR: Style = 7;
    /// A preprocessor directive.
    pub const PREPROCESSOR: Style = 8;

    /// The first of the eight keyword groups a user-defined language can
    /// declare. The groups are separate styles rather than one, because
    /// Notepad++ colours them separately and a language that uses two groups
    /// for two kinds of word relies on the difference.
    pub const UDL_KEYWORD1: Style = 16;
    pub const UDL_KEYWORD2: Style = 17;
    pub const UDL_KEYWORD3: Style = 18;
    pub const UDL_KEYWORD4: Style = 19;
    pub const UDL_KEYWORD5: Style = 20;
    pub const UDL_KEYWORD6: Style = 21;
    pub const UDL_KEYWORD7: Style = 22;
    pub const UDL_KEYWORD8: Style = 23;

    /// The first of the eight delimiter pairs a user-defined language can
    /// declare. Separate styles for the same reason as the keyword groups.
    pub const UDL_DELIMITER1: Style = 24;
    pub const UDL_DELIMITER2: Style = 25;
    pub const UDL_DELIMITER3: Style = 26;
    pub const UDL_DELIMITER4: Style = 27;
    pub const UDL_DELIMITER5: Style = 28;
    pub const UDL_DELIMITER6: Style = 29;
    pub const UDL_DELIMITER7: Style = 30;
    pub const UDL_DELIMITER8: Style = 31;

    /// One past the highest style index any lexer emits.
    pub const COUNT: usize = 32;
}

/// What the lexer was in the middle of when the previous line ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LexState {
    /// Ordinary code.
    #[default]
    Normal,
    /// Inside a `/* ... */` block.
    BlockComment,
    /// Inside a string opened with the given quote character.
    String(char),
    /// Inside construct `n` of a user-defined language.
    ///
    /// A user-defined language can open any of a dozen constructs -- two block
    /// comments and eight delimiter pairs -- and the state has to say which one
    /// is open, because the close marker differs. `String(char)` cannot: two
    /// delimiters can share an opening character and differ in their close.
    Udl(u8),
}

impl LexState {
    /// Encodes the state as the byte stored per line in the document.
    pub const fn to_u8(self) -> u8 {
        match self {
            LexState::Normal => 0,
            LexState::BlockComment => 1,
            LexState::String('"') => 2,
            LexState::String('\'') => 3,
            LexState::String(_) => 2,
            // Constructs are numbered from zero and there are at most a dozen,
            // so the byte has room for all of them.
            LexState::Udl(n) => 4u8.saturating_add(n),
        }
    }

    /// Decodes a byte written by [`LexState::to_u8`].
    pub const fn from_u8(byte: u8) -> Self {
        match byte {
            1 => LexState::BlockComment,
            2 => LexState::String('"'),
            3 => LexState::String('\''),
            byte if byte >= 4 => LexState::Udl(byte - 4),
            _ => LexState::Normal,
        }
    }
}

/// A line-at-a-time tokeniser.
pub trait Lexer {
    /// Styles `text`, continuing from `state`, and reports the state at the end.
    fn lex_line(&mut self, text: &str, state: LexState) -> (Vec<Style>, LexState);
}

/// A lexer that highlights nothing.
///
/// Notepad++'s Language menu offers "Plain Text", and it means exactly this:
/// every character in the default style. It is a lexer rather than a special
/// case in the highlighter so that switching to it costs the same as switching
/// to any other language, and so that a document that was highlighted before
/// loses its colours on the next pass rather than keeping stale ones.
#[derive(Debug, Default, Clone, Copy)]
pub struct PlainLexer;

impl Lexer for PlainLexer {
    fn lex_line(&mut self, text: &str, _state: LexState) -> (Vec<Style>, LexState) {
        (vec![style::DEFAULT; text.chars().count()], LexState::Normal)
    }
}

/// A lexer for the C family: Rust, C, C++, Java, JavaScript, Go, C#.
///
/// Also handles Python and shell well enough to be useful, since their comments
/// and strings follow the same rules.
#[derive(Debug, Default, Clone, Copy)]
pub struct CLikeLexer;

impl Lexer for CLikeLexer {
    fn lex_line(&mut self, text: &str, state: LexState) -> (Vec<Style>, LexState) {
        let chars: Vec<char> = text.chars().collect();
        let mut styles = vec![style::DEFAULT; chars.len()];
        let mut i = 0;

        // Finish whatever the previous line left open.
        match state {
            LexState::BlockComment => match find_block_end(&chars, 0) {
                Some(end) => {
                    styles[..end].fill(style::COMMENT);
                    i = end;
                }
                None => {
                    styles.fill(style::COMMENT);
                    return (styles, LexState::BlockComment);
                }
            },
            LexState::String(quote) => match find_string_end(&chars, 0, quote) {
                Some(end) => {
                    styles[..end].fill(style::STRING);
                    i = end;
                }
                None => {
                    styles.fill(style::STRING);
                    return (styles, LexState::String(quote));
                }
            },
            // A user-defined language's construct is not this lexer's to
            // finish; treat it as ordinary text rather than guessing.
            LexState::Normal | LexState::Udl(_) => {}
        }

        while i < chars.len() {
            let c = chars[i];
            match c {
                '/' if chars.get(i + 1) == Some(&'/') => {
                    styles[i..].fill(style::COMMENT);
                    return (styles, LexState::Normal);
                }
                '/' if chars.get(i + 1) == Some(&'*') => match find_block_end(&chars, i) {
                    Some(end) => {
                        styles[i..end].fill(style::COMMENT);
                        i = end;
                    }
                    None => {
                        styles[i..].fill(style::COMMENT);
                        return (styles, LexState::BlockComment);
                    }
                },
                '"' => match find_string_end(&chars, i, '"') {
                    Some(end) => {
                        styles[i..end].fill(style::STRING);
                        i = end;
                    }
                    None => {
                        styles[i..].fill(style::STRING);
                        return (styles, LexState::String('"'));
                    }
                },
                '\'' => {
                    // A character literal closes within a few characters. If it
                    // does not, this is a lifetime or a lone quote, not a string.
                    match find_char_literal_end(&chars, i) {
                        Some(end) => {
                            styles[i..end].fill(style::STRING);
                            i = end;
                        }
                        None => {
                            styles[i] = style::OPERATOR;
                            i += 1;
                        }
                    }
                }
                '#' if chars[..i].iter().all(|c| c.is_whitespace()) => {
                    styles[i..].fill(style::PREPROCESSOR);
                    return (styles, LexState::Normal);
                }
                c if c.is_ascii_digit() => {
                    let j = number_end(&chars, i);
                    styles[i..j].fill(style::NUMBER);
                    i = j;
                }
                c if c.is_alphabetic() || c == '_' => {
                    let mut j = i;
                    while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                        j += 1;
                    }
                    let word: String = chars[i..j].iter().collect();
                    let call = declares_a_function(&chars, i) || is_called(&chars, j);
                    let style = classify(&word, call);
                    styles[i..j].fill(style);
                    i = j;
                }
                c if is_operator(c) => {
                    styles[i] = style::OPERATOR;
                    i += 1;
                }
                _ => i += 1,
            }
        }

        (styles, LexState::Normal)
    }
}

/// A lexer driven by a [`LangSpec`](crate::lang::LangSpec).
///
/// Ninety-odd languages differ in four things a line-at-a-time tokeniser can
/// see: their comment markers, their quote characters, how an apostrophe ends,
/// and their keyword list. Writing ninety lexers would produce ninety files that
/// differ only in their tables, so the tables live in `lang` and this one lexer
/// reads them.
///
/// The order of the checks is the whole design:
///
/// 1. A block-comment opener is tested before a line-comment marker, because
///    several languages make one a prefix of the other. Lua's `--[[` must win
///    over `--`, Nim's `#[` over `#`, CoffeeScript's `###` over `#`, Matlab's
///    `%{` over `%`. Testing the line marker first would swallow the block
///    opener and colour the rest of the file as a comment.
/// 2. Quotes are tested after comments, so a `"` inside a comment stays a
///    comment and a `//` inside a string stays a string.
/// 3. An apostrophe is only a string where the language says so, and only
///    while it still closes on the line. `&'a str` is a lifetime, `x'` is an
///    identifier and `don't` is prose; colouring any of them as the start of a
///    string would carry that string into every line below.
/// 4. Numbers before identifiers, so `0x1f` is one number rather than a number
///    and a word.
///
/// What it cannot do is context. `*` is a comment in SAS only at the start of a
/// statement, and an apostrophe in HTML prose is not an attribute quote; those
/// languages declare no marker rather than a wrong one.
pub struct ConfigLexer {
    spec: &'static crate::lang::LangSpec,
}

impl ConfigLexer {
    /// A lexer for `spec`.
    pub fn new(spec: &'static crate::lang::LangSpec) -> Self {
        Self { spec }
    }

    /// Whether `marker` occurs at `at`.
    fn starts_with(&self, chars: &[char], at: usize, marker: &str) -> bool {
        marker
            .chars()
            .enumerate()
            .all(|(offset, m)| chars.get(at + offset) == Some(&m))
    }

    /// Whether a line comment starts at `at`.
    fn line_comment_at(&self, chars: &[char], at: usize) -> bool {
        self.spec
            .line
            .iter()
            .any(|marker| self.starts_with(chars, at, marker))
    }

    /// The index just past the literal opened by `quote` at `at`.
    ///
    /// `None` means the literal does not end on this line, which is a string
    /// carried into the next one for `"` and for an apostrophe the language
    /// lets run on.
    fn literal_end(&self, chars: &[char], at: usize, quote: char) -> Option<usize> {
        match self.spec.apostrophe {
            Apostrophe::Char if quote == '\'' => find_char_literal_end(chars, at),
            _ => find_string_end(chars, at, quote),
        }
    }

    /// Whether a literal opened by an apostrophe may cross into the next line.
    fn apostrophe_runs_on(&self) -> bool {
        self.spec.quotes.contains(&'\'') && self.spec.apostrophe == Apostrophe::Multiline
    }

    /// The index just past a block-comment opener at `at`, if one starts there.
    fn block_open_at(&self, chars: &[char], at: usize) -> Option<usize> {
        let (open, _) = self.spec.block?;
        if self.starts_with(chars, at, open) {
            Some(at + open.chars().count())
        } else {
            None
        }
    }

    /// The index just past the block-comment closer at or after `from`.
    fn find_block_close(&self, chars: &[char], from: usize) -> Option<usize> {
        let (_, close) = self.spec.block?;
        let close: Vec<char> = close.chars().collect();
        let mut j = from;
        while j + close.len() <= chars.len() {
            if chars[j..j + close.len()] == close[..] {
                return Some(j + close.len());
            }
            j += 1;
        }
        None
    }
}

impl Lexer for ConfigLexer {
    fn lex_line(&mut self, text: &str, state: LexState) -> (Vec<Style>, LexState) {
        let chars: Vec<char> = text.chars().collect();
        let mut styles = vec![style::DEFAULT; chars.len()];
        let mut i = 0;

        // Finish whatever the previous line left open.
        match state {
            LexState::BlockComment => match self.find_block_close(&chars, 0) {
                Some(end) => {
                    styles[..end].fill(style::COMMENT);
                    i = end;
                }
                None => {
                    styles.fill(style::COMMENT);
                    return (styles, LexState::BlockComment);
                }
            },
            // A carried apostrophe is only resumable in the languages that let
            // one run on; anywhere else it was never a string to begin with,
            // and falls through below.
            LexState::String(quote) if quote != '\'' || self.apostrophe_runs_on() => {
                match find_string_end(&chars, 0, quote) {
                    Some(end) => {
                        styles[..end].fill(style::STRING);
                        i = end;
                    }
                    None => {
                        styles.fill(style::STRING);
                        return (styles, LexState::String(quote));
                    }
                }
            }
            // A construct of a user-defined language is not this lexer's to
            // finish, and an apostrophe it never opened is not a literal:
            // both are ordinary text here.
            LexState::Normal | LexState::String(_) | LexState::Udl(_) => {}
        }

        while i < chars.len() {
            if let Some(open_end) = self.block_open_at(&chars, i) {
                match self.find_block_close(&chars, open_end) {
                    Some(end) => {
                        styles[i..end].fill(style::COMMENT);
                        i = end;
                    }
                    None => {
                        styles[i..].fill(style::COMMENT);
                        return (styles, LexState::BlockComment);
                    }
                }
                continue;
            }
            if self.line_comment_at(&chars, i) {
                styles[i..].fill(style::COMMENT);
                return (styles, LexState::Normal);
            }

            let c = chars[i];
            if self.spec.quotes.contains(&c) {
                match self.literal_end(&chars, i, c) {
                    Some(end) => {
                        styles[i..end].fill(style::STRING);
                        i = end;
                    }
                    // An apostrophe that does not close on its own line is
                    // only a string where the language lets one run on. The
                    // rest of the time it is the quote in `don't`, a lifetime,
                    // or a transpose, and drawing it as punctuation keeps it
                    // out of the lines below.
                    None if c == '\'' && !self.apostrophe_runs_on() => {
                        styles[i] = style::OPERATOR;
                        i += 1;
                    }
                    None => {
                        styles[i..].fill(style::STRING);
                        return (styles, LexState::String(c));
                    }
                }
                continue;
            }
            if c.is_ascii_digit() {
                let j = number_end(&chars, i);
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
                let call = declares_a_function(&chars, i) || is_called(&chars, j);
                let style = if self.spec.is_keyword(&word) {
                    style::KEYWORD
                } else if call {
                    style::FUNCTION
                } else {
                    style::DEFAULT
                };
                styles[i..j].fill(style);
                i = j;
                continue;
            }
            if is_operator(c) {
                styles[i] = style::OPERATOR;
            }
            i += 1;
        }

        (styles, LexState::Normal)
    }
}

/// The index just past the `*/` that closes a block comment starting at `from`.
fn find_block_end(chars: &[char], from: usize) -> Option<usize> {
    let mut j = from;
    while j + 1 < chars.len() {
        if chars[j] == '*' && chars[j + 1] == '/' {
            return Some(j + 2);
        }
        j += 1;
    }
    None
}

/// The index just past the quote that closes a string starting at `from`.
fn find_string_end(chars: &[char], from: usize, quote: char) -> Option<usize> {
    let mut j = from + 1;
    while j < chars.len() {
        match chars[j] {
            '\\' => j += 2,
            c if c == quote => return Some(j + 1),
            _ => j += 1,
        }
    }
    None
}

/// The index just past a character literal starting at `from`, if it is one.
///
/// A character literal holds one character or one escape: `'a'`, `'\n'`,
/// `'\''`, `'\u{1F600}'`. A longer run is not a literal -- it is a lifetime, a
/// type parameter, a quote macro or a transpose -- and pairing its opening
/// quote with the next apostrophe on the line would colour code as a string.
fn find_char_literal_end(chars: &[char], from: usize) -> Option<usize> {
    if chars.get(from + 1) == Some(&'\\') {
        // An escape: scan a few characters for the closing quote, stepping
        // over the backslash pairs on the way.
        let mut j = from + 1;
        while j < chars.len() && j <= from + 12 {
            match chars[j] {
                '\\' => j += 2,
                '\'' if j > from + 1 => return Some(j + 1),
                _ => j += 1,
            }
        }
        return None;
    }
    // One character, then the closing quote, and nothing else.
    (chars.get(from + 2) == Some(&'\'')).then_some(from + 3)
}

/// The words that name the word after them as a function.
///
/// A definition is the one place a function name is not followed by `(`, so
/// without these `fn f<'a>(` and `def f` would lose their colour. The list
/// holds no language's name: the declaration words are the same word in a dozen
/// languages, shortened or not.
const DECLARES_A_FUNCTION: &[&str] = &[
    "def",
    "defmacro",
    "defn",
    "defp",
    "defun",
    "fn",
    "func",
    "fun",
    "function",
    "procedure",
    "proc",
    "sub",
    "subroutine",
];

/// The index just past a numeric literal starting at `at`.
///
/// The scan is fine-grained for the stop rather than for the start: `4..end` is
/// a range, and a literal that swallows what follows it takes the identifier
/// after the dots with it, which is how a method call lost its colour.
fn number_end(chars: &[char], at: usize) -> usize {
    let digit = |c: char| c.is_ascii_digit() || c == '_';
    let mut j = at;
    while j < chars.len() && digit(chars[j]) {
        j += 1;
    }
    // A fraction only where a digit follows the dot: `1.5` is one number,
    // `4..end` and `1.foo()` are a number and punctuation.
    if chars.get(j) == Some(&'.') && chars.get(j + 1).is_some_and(|c| c.is_ascii_digit()) {
        j += 1;
        while j < chars.len() && digit(chars[j]) {
            j += 1;
        }
    }
    // An exponent, which is a sign and digits.
    if matches!(chars.get(j), Some('e' | 'E')) {
        let mut k = j + 1;
        if matches!(chars.get(k), Some('+' | '-')) {
            k += 1;
        }
        if chars.get(k).is_some_and(|c| c.is_ascii_digit()) {
            j = k;
            while j < chars.len() && digit(chars[j]) {
                j += 1;
            }
        }
    }
    // A suffix or a radix marker: `0x1f`, `0b1010`, `1u8`, `1.5f32`, `10L`.
    while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
        j += 1;
    }
    j
}

/// Whether the word ending at `end` is used as a function: followed by `(`,
/// or by a list of type arguments and then `(`.
fn is_called(chars: &[char], end: usize) -> bool {
    let mut j = end;
    while chars.get(j).is_some_and(|c| c.is_whitespace()) {
        j += 1;
    }
    if chars.get(j) == Some(&'(') {
        return true;
    }
    // A turbofish: `collect::<Vec<_>>()`.
    if chars.get(j) == Some(&':') && chars.get(j + 1) == Some(&':') {
        j += 2;
    }
    chars.get(j) == Some(&'<')
        && generic_end(chars, j).is_some_and(|after| chars.get(after) == Some(&'('))
}

/// The index just past the `>` that closes the `<` at `open`, when what stands
/// between them can be a list of type arguments.
///
/// The test is deliberately narrow. `<` is a comparison in every language here,
/// so `a < b && c > (d)` must not read as a call: a type argument list holds
/// letters, digits, spaces, commas, `:`, `'`, `*`, `.`, brackets, `!`, `+`, `-`
/// and parentheses, and it is short.
fn generic_end(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut j = open;
    while j < chars.len() && j - open <= 60 {
        match chars[j] {
            '<' => depth += 1,
            '>' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(j + 1);
                }
            }
            // `&&` is a comparison in C and the family; a single `&` is a
            // reference in the type arguments of Rust and C++.
            '&' if chars.get(j + 1) != Some(&'&') => {}
            c if c.is_alphanumeric() || c == ' ' || "_:,'*.[]()!#+-".contains(c) => {}
            _ => return None,
        }
        j += 1;
    }
    None
}

/// Whether the word before `at` declares the word at `at` as a function.
fn declares_a_function(chars: &[char], at: usize) -> bool {
    let mut back = at;
    while back > 0 && chars[back - 1].is_whitespace() {
        back -= 1;
    }
    let mut begin = back;
    while begin > 0 && (chars[begin - 1].is_alphanumeric() || chars[begin - 1] == '_') {
        begin -= 1;
    }
    if begin == back {
        return false;
    }
    let word: String = chars[begin..back].iter().collect();
    DECLARES_A_FUNCTION
        .iter()
        .any(|declares| declares.eq_ignore_ascii_case(&word))
}

/// Whether `c` is drawn as punctuation.
fn is_operator(c: char) -> bool {
    "(){}[]<>+-*/%=!&|^~?:;,.@$".contains(c)
}

/// The style for an identifier, given whether a `(` follows it.
fn classify(word: &str, is_call: bool) -> Style {
    if let Some(style) = keyword_style(word) {
        return style;
    }
    if is_call {
        return style::FUNCTION;
    }
    style::DEFAULT
}

/// The style for a keyword or built-in type, if `word` is one.
fn keyword_style(word: &str) -> Option<Style> {
    let style = match word {
        // Rust
        "fn" | "let" | "mut" | "if" | "else" | "for" | "while" | "loop" | "match" | "return"
        | "break" | "continue" | "struct" | "enum" | "impl" | "trait" | "pub" | "use" | "mod"
        | "const" | "static" | "unsafe" | "async" | "await" | "move" | "ref" | "where" | "as"
        | "in" | "dyn" | "crate" | "super" | "self" | "Self" | "type" | "extern" | "true"
        | "false" | "macro_rules" | "union" | "yield" | "box" => style::KEYWORD,
        // C family
        "class" | "public" | "private" | "protected" | "void" | "int" | "char" | "float"
        | "double" | "new" | "delete" | "this" | "namespace" | "template" | "virtual"
        | "override" | "final" | "nullptr" | "null" | "switch" | "case" | "default" | "do"
        | "goto" | "sizeof" | "typedef" | "using" | "operator" | "friend" | "explicit"
        | "inline" | "volatile" | "register" | "auto" | "long" | "short" | "signed"
        | "unsigned" | "throw" | "try" | "catch" | "finally" | "extends" | "implements"
        | "interface" | "package" | "import" | "abstract" | "instanceof" | "var" | "function"
        | "typeof" | "of" | "get" | "set" => style::KEYWORD,
        // Python
        "def" | "from" | "elif" | "with" | "lambda" | "global" | "nonlocal" | "pass" | "raise"
        | "assert" | "del" | "and" | "or" | "not" | "is" | "None" | "True" | "False" | "except" => {
            style::KEYWORD
        }
        // Built-in types
        "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64" | "u128"
        | "usize" | "f32" | "f64" | "bool" | "str" | "String" | "Vec" | "Option" | "Result"
        | "Box" | "Rc" | "Arc" | "HashMap" | "HashSet" | "BTreeMap" | "VecDeque" | "Cow"
        | "Path" | "PathBuf" | "Duration" | "Instant" => style::TYPE,
        _ => return None,
    };
    Some(style)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(text: &str) -> Vec<Style> {
        CLikeLexer.lex_line(text, LexState::Normal).0
    }

    fn lex_from(text: &str, state: LexState) -> (Vec<Style>, LexState) {
        CLikeLexer.lex_line(text, state)
    }

    #[test]
    fn keywords_are_recognised() {
        let styles = lex("fn main");
        assert_eq!(styles[0], style::KEYWORD);
        assert_eq!(styles[1], style::KEYWORD);
        // The name after a declaration keyword is a function, not plain text.
        assert_eq!(styles[3], style::FUNCTION);
    }

    #[test]
    fn built_in_types_are_recognised() {
        let styles = lex("let x: u32");
        assert_eq!(styles[0], style::KEYWORD);
        assert_eq!(styles[7], style::TYPE);
    }

    #[test]
    fn calls_are_recognised() {
        let styles = lex("foo(1)");
        assert_eq!(styles[0], style::FUNCTION);
        assert_eq!(styles[4], style::NUMBER);
    }

    #[test]
    fn line_comments_run_to_the_end() {
        let styles = lex("x // fn");
        assert_eq!(styles[0], style::DEFAULT);
        assert!(styles[2..].iter().all(|&s| s == style::COMMENT));
    }

    #[test]
    fn block_comments_close_on_the_same_line() {
        let styles = lex("a /* b */ c");
        assert_eq!(styles[0], style::DEFAULT);
        assert_eq!(styles[2], style::COMMENT);
        assert_eq!(styles[6], style::COMMENT);
        assert_eq!(styles[10], style::DEFAULT);
    }

    #[test]
    fn block_comments_carry_across_lines() {
        let (styles, state) = lex_from("a /* b", LexState::Normal);
        assert_eq!(state, LexState::BlockComment);
        assert_eq!(styles[2], style::COMMENT);

        let (styles, state) = lex_from("still comment", state);
        assert_eq!(state, LexState::BlockComment);
        assert!(styles.iter().all(|&s| s == style::COMMENT));

        let (styles, state) = lex_from("end */ x", state);
        assert_eq!(state, LexState::Normal);
        assert_eq!(styles[5], style::COMMENT);
        assert_eq!(styles[7], style::DEFAULT);
    }

    #[test]
    fn strings_are_recognised() {
        let styles = lex(r#"let s = "hi";"#);
        assert_eq!(styles[8], style::STRING);
        assert_eq!(styles[11], style::STRING);
        assert_eq!(styles[12], style::OPERATOR);
    }

    #[test]
    fn escaped_quotes_do_not_close_a_string() {
        let styles = lex(r#""a\"b""#);
        assert!(styles.iter().all(|&s| s == style::STRING));
    }

    #[test]
    fn unterminated_strings_carry_across_lines() {
        let (styles, state) = lex_from(r#"s = "abc"#, LexState::Normal);
        assert_eq!(state, LexState::String('"'));
        assert_eq!(styles[4], style::STRING);

        let (styles, state) = lex_from(r#"def""#, state);
        assert_eq!(state, LexState::Normal);
        assert!(styles.iter().all(|&s| s == style::STRING));
    }

    #[test]
    fn char_literals_are_strings() {
        let styles = lex("let c = 'x';");
        assert_eq!(styles[8], style::STRING);
        assert_eq!(styles[10], style::STRING);
    }

    #[test]
    fn lifetimes_are_not_strings() {
        let styles = lex("fn f<'a>(x: &'a str)");
        assert_eq!(styles[5], style::OPERATOR);
        assert_eq!(styles[6], style::DEFAULT);
        assert_eq!(styles[7], style::OPERATOR);
    }

    #[test]
    fn numbers_are_recognised() {
        let styles = lex("x = 42 + 3.5");
        assert_eq!(styles[4], style::NUMBER);
        assert_eq!(styles[5], style::NUMBER);
        assert_eq!(styles[10], style::NUMBER);
    }

    #[test]
    fn preprocessor_directives_are_recognised() {
        let styles = lex("#include <stdio.h>");
        assert!(styles.iter().all(|&s| s == style::PREPROCESSOR));
    }

    #[test]
    fn a_hash_mid_line_is_not_a_directive() {
        let styles = lex("x = a # b");
        assert_eq!(styles[6], style::DEFAULT);
    }

    #[test]
    fn operators_are_recognised() {
        let styles = lex("a+b");
        assert_eq!(styles[1], style::OPERATOR);
    }

    #[test]
    fn style_count_matches_char_count() {
        for text in ["", "a", "héllo", "fn main() {}", "// ünïcode"] {
            assert_eq!(lex(text).len(), text.chars().count(), "for {text:?}");
        }
    }

    #[test]
    fn the_plain_lexer_styles_nothing() {
        let text = "fn main() { /* x */ }";
        let (styles, state) = PlainLexer.lex_line(text, LexState::BlockComment);
        assert_eq!(styles.len(), text.chars().count());
        assert!(styles.iter().all(|&s| s == style::DEFAULT));
        // A plain document has no constructs, so it never carries state.
        assert_eq!(state, LexState::Normal);
    }

    #[test]
    fn a_user_defined_state_byte_round_trips() {
        for number in 0..12u8 {
            assert_eq!(LexState::from_u8(LexState::Udl(number).to_u8()), LexState::Udl(number));
        }
        // The three built-in states keep their bytes, so a document highlighted
        // before this existed still reads back the same.
        assert_eq!(LexState::Normal.to_u8(), 0);
        assert_eq!(LexState::BlockComment.to_u8(), 1);
        assert_eq!(LexState::String('"').to_u8(), 2);
        assert_eq!(LexState::String('\'').to_u8(), 3);
    }

    #[test]
    fn empty_line_is_normal_state() {
        let (styles, state) = lex_from("", LexState::Normal);
        assert!(styles.is_empty());
        assert_eq!(state, LexState::Normal);
    }

    /// The spec for a language the tests below name.
    fn spec(name: &str) -> &'static crate::lang::LangSpec {
        let index = crate::lang::by_name(name).unwrap_or_else(|| panic!("no language named {name}"));
        &crate::lang::LANGS[index]
    }

    fn config(name: &str, text: &str) -> Vec<Style> {
        ConfigLexer::new(spec(name)).lex_line(text, LexState::Normal).0
    }

    #[test]
    fn a_block_opener_wins_over_a_line_marker_that_prefixes_it() {
        // Lua's `--[[` starts with `--`. Testing the line marker first would
        // colour the rest of the file as a comment instead of the block.
        let text = "--[[ a block ]] code";
        let styles = config("Lua", text);
        let chars: Vec<char> = text.chars().collect();
        let close = chars.windows(2).position(|w| w == [']', ']']).unwrap() + 2;
        assert!(styles[..close].iter().all(|&s| s == style::COMMENT));
        assert_eq!(styles[close], style::DEFAULT);

        // The plain line comment still works.
        let styles = config("Lua", "-- just a line");
        assert!(styles.iter().all(|&s| s == style::COMMENT));
    }

    #[test]
    fn a_block_comment_carries_across_lines() {
        let mut lexer = ConfigLexer::new(spec("Lua"));
        let (styles, state) = lexer.lex_line("--[[ opened", LexState::Normal);
        assert!(styles.iter().all(|&s| s == style::COMMENT));
        assert_eq!(state, LexState::BlockComment);

        let (styles, state) = lexer.lex_line("still ]] after", state);
        let chars: Vec<char> = "still ]] after".chars().collect();
        let close = chars.windows(2).position(|w| w == [']', ']']).unwrap() + 2;
        assert!(styles[..close].iter().all(|&s| s == style::COMMENT));
        assert_eq!(styles[close + 1], style::DEFAULT);
        assert_eq!(state, LexState::Normal);
    }

    #[test]
    fn a_multi_character_block_close_is_matched_whole() {
        // Nim closes with `]#`, not `]`. A single-character close would end the
        // comment one character early and leave the `#` unstyled.
        let mut lexer = ConfigLexer::new(spec("Nim"));
        let (styles, state) = lexer.lex_line("#[ open", LexState::Normal);
        assert!(styles.iter().all(|&s| s == style::COMMENT));
        assert_eq!(state, LexState::BlockComment);

        let (styles, state) = lexer.lex_line("x ]# y", state);
        assert_eq!(styles[0], style::COMMENT);
        assert_eq!(styles[3], style::COMMENT);
        assert_eq!(styles[4], style::DEFAULT);
        assert_eq!(state, LexState::Normal);
    }

    #[test]
    fn keywords_fold_case_only_where_the_spec_says_so() {
        // Pascal was designed before case mattered.
        let styles = config("Pascal", "BEGIN");
        assert!(styles.iter().all(|&s| s == style::KEYWORD));
        // C was not: `BEGIN` is an identifier there.
        let styles = config("C", "BEGIN");
        assert!(styles.iter().all(|&s| s == style::DEFAULT));
    }

    #[test]
    fn a_string_continues_to_the_next_line() {
        let mut lexer = ConfigLexer::new(spec("Python"));
        let (styles, state) = lexer.lex_line("x = \"open", LexState::Normal);
        assert_eq!(styles[0], style::DEFAULT);
        assert_eq!(styles[4], style::STRING);
        assert_eq!(state, LexState::String('"'));

        let (styles, state) = lexer.lex_line("close\" y", state);
        assert_eq!(styles[0], style::STRING);
        assert_eq!(styles[6], style::DEFAULT);
        assert_eq!(state, LexState::Normal);
    }

    #[test]
    fn an_apostrophe_that_does_not_close_is_not_a_string() {
        // YAML's apostrophe is the one in `don't`. Calling it the start of a
        // string coloured every line below the value as a string until the next
        // apostrophe in the file.
        let mut lexer = ConfigLexer::new(spec("YAML"));
        let (styles, state) = lexer.lex_line("name: don't panic", LexState::Normal);
        assert!(!styles.contains(&style::STRING));
        assert_eq!(state, LexState::Normal);

        let (styles, state) = lexer.lex_line("version: 3", state);
        assert_eq!(styles[0], style::DEFAULT);
        assert_eq!(state, LexState::Normal);
    }

    #[test]
    fn a_single_quoted_string_still_closes_on_its_own_line() {
        let styles = config("YAML", "note: 'quoted here'");
        assert_eq!(styles.len(), 19);
        assert_eq!(styles[6], style::STRING);
        assert_eq!(styles[18], style::STRING);
    }

    #[test]
    fn a_rust_lifetime_is_not_a_string() {
        // `'` is a quote in Rust, for the character literal. A lifetime is the
        // other use of the same character, and it must not open anything.
        for text in [
            "fn f<'a>(x: &'a str)",
            "fn f<'a, 'b>(a: &'a str, b: &'b str)",
        ] {
            let mut lexer = ConfigLexer::new(spec("Rust"));
            let (styles, state) = lexer.lex_line(text, LexState::Normal);
            assert!(!styles.contains(&style::STRING), "for {text:?}");
            assert_eq!(state, LexState::Normal, "for {text:?}");
        }
    }

    #[test]
    fn a_rust_character_literal_is_a_string() {
        let styles = config("Rust", "let c = 'x';");
        assert_eq!(styles[8], style::STRING);
        assert_eq!(styles[10], style::STRING);
        // An escape holds more than one character and still closes.
        let styles = config("Rust", "let c = '\\n';");
        assert_eq!(styles[8], style::STRING);
        assert_eq!(styles[11], style::STRING);
    }

    #[test]
    fn a_prime_belongs_to_the_word() {
        // Haskell's `x'` is an identifier, and `'` quotes nothing there.
        let mut lexer = ConfigLexer::new(spec("Haskell"));
        let (styles, state) = lexer.lex_line("let x' = 1 in x'", LexState::Normal);
        assert!(!styles.contains(&style::STRING));
        assert_eq!(state, LexState::Normal);
    }

    #[test]
    fn a_transpose_is_not_a_string() {
        let mut lexer = ConfigLexer::new(spec("Julia"));
        let (styles, state) = lexer.lex_line("b = a'", LexState::Normal);
        assert!(!styles.contains(&style::STRING));
        assert_eq!(state, LexState::Normal);
    }

    #[test]
    fn a_shell_string_still_runs_across_lines() {
        // The rule is per language, not one rule for all of them: a shell
        // string opens with an apostrophe and legitimately ends on a later
        // line, and must keep doing so.
        let mut lexer = ConfigLexer::new(spec("Shell"));
        let (styles, state) = lexer.lex_line("msg='line one", LexState::Normal);
        assert_eq!(styles[4], style::STRING);
        assert_eq!(state, LexState::String('\''));

        let (styles, state) = lexer.lex_line("line two' end", state);
        assert_eq!(styles[8], style::STRING);
        assert_eq!(styles[10], style::DEFAULT);
        assert_eq!(state, LexState::Normal);
    }

    #[test]
    fn a_stray_apostrophe_never_reaches_the_next_line() {
        // The rules live in the tables, and the tables are data. This walks all
        // of them: a word holding one apostrophe either opens a string the
        // language says may run on, or opens nothing at all.
        for spec in crate::lang::LANGS.iter() {
            let allowed = spec.quotes.contains(&'\'')
                && spec.apostrophe == crate::lang::Apostrophe::Multiline;
            let (styles, state) = ConfigLexer::new(spec).lex_line("don't", LexState::Normal);
            assert_eq!(styles.len(), 5, "{}", spec.name);
            if allowed {
                assert_eq!(state, LexState::String('\''), "{}", spec.name);
            } else {
                assert_eq!(state, LexState::Normal, "{}", spec.name);
                assert!(!styles.contains(&style::STRING), "{}", spec.name);
            }
        }
    }

    #[test]
    fn a_language_that_does_not_quote_with_an_apostrophe_does_not_list_it() {
        // The table promises that `quotes` is what can open a literal, so the
        // languages where an apostrophe is a prime or an operator leave it out.
        for name in ["Haskell", "Julia", "Makefile", "Dockerfile", "NSIS"] {
            assert!(!spec(name).quotes.contains(&'\''), "{name}");
        }
    }

    #[test]
    fn a_generic_definition_is_a_function() {
        // A definition is the one place a function name is not followed by
        // `(`, and a lifetime or a type parameter can stand between the two.
        let styles = config("Rust", "fn markup<'a>(text: &'a str)");
        assert_eq!(styles[3], style::FUNCTION);
        let styles = config("Rust", "fn f<T>(x: T)");
        assert_eq!(styles[3], style::FUNCTION);
        // A language that declares with a word of its own.
        let styles = config("Perl", "sub foo {");
        assert_eq!(styles[4], style::FUNCTION);
        let styles = config("TCL", "proc foo {}");
        assert_eq!(styles[5], style::FUNCTION);
    }

    #[test]
    fn a_turbofish_call_is_a_function() {
        let text = "let v: Vec<_> = names.collect::<Vec<_>>();";
        let styles = config("Rust", text);
        let at = text.find("collect").unwrap();
        assert_eq!(styles[at], style::FUNCTION);
    }

    #[test]
    fn a_type_argument_list_is_not_a_call() {
        // `Vec<u8>` is a type: nothing follows the `>` but a name.
        let styles = config("Rust", "let v: Vec<u8> = value;");
        assert_eq!(styles[7], style::DEFAULT);
        assert!(!styles.contains(&style::FUNCTION));
    }

    #[test]
    fn a_comparison_is_not_a_call() {
        // `<` and `>` are operators here, and a rule that reads them as type
        // arguments colours `a` as a function.
        let styles = config("C", "if (a < b && c > (d)) {");
        assert!(!styles.contains(&style::FUNCTION));
    }

    #[test]
    fn a_number_stops_where_the_number_stops() {
        // `4..end.len()` is a range and a call, not one long number; the old
        // rule took everything alphanumeric and dotted with it, so `len` was
        // painted as a number and the call lost its colour.
        let text = "let tail = &s[4..end.len()];";
        let styles = config("Rust", text);
        let four = text.find('4').unwrap();
        assert_eq!(styles[four], style::NUMBER);
        assert_eq!(styles[four + 1], style::OPERATOR);
        let end = text.find("end").unwrap();
        assert_eq!(styles[end], style::DEFAULT);
        assert_eq!(styles[end + 4], style::FUNCTION);
        // The forms that are one number stay whole.
        for (text, number) in [
            ("x = 1.5f32;", "1.5f32"),
            ("x = 0x1f;", "0x1f"),
            ("x = 1e-9;", "1e-9"),
            ("x = 1_000u64;", "1_000u64"),
        ] {
            let styles = config("Rust", text);
            let start = text.find(number).unwrap();
            let painted = &styles[start..start + number.len()];
            assert!(painted.iter().all(|&s| s == style::NUMBER), "for {text:?}");
        }
    }

    #[test]
    fn an_identifier_before_a_paren_is_a_function() {
        let styles = config("Python", "foo(1)");
        assert_eq!(styles[0], style::FUNCTION);
        assert_eq!(styles[3], style::OPERATOR);
        assert_eq!(styles[4], style::NUMBER);
    }

    #[test]
    fn a_comment_hides_the_quotes_inside_it() {
        // The quote is inside a comment, so it must not open a string that
        // swallows the next line.
        let (styles, state) = ConfigLexer::new(spec("C")).lex_line("// \"unclosed", LexState::Normal);
        assert!(styles.iter().all(|&s| s == style::COMMENT));
        assert_eq!(state, LexState::Normal);
    }

    #[test]
    fn a_language_with_no_comment_markers_styles_none() {
        let plain = crate::lang::LANGS
            .iter()
            .find(|s| s.line.is_empty() && s.block.is_none())
            .expect("some language declares no comments");
        let text = "// not a comment";
        let styles = ConfigLexer::new(plain).lex_line(text, LexState::Normal).0;
        assert!(styles.iter().all(|&s| s != style::COMMENT));
    }

    #[test]
    fn every_language_lexes_a_line_without_panicking() {
        // The tables are data, and data can be wrong. This walks all of them
        // over text that exercises every branch: a comment, a quote, a digit,
        // a word and an operator.
        let text = "x = 1 // \"s\" /* b */";
        for spec in crate::lang::LANGS.iter() {
            let (styles, _) = ConfigLexer::new(spec).lex_line(text, LexState::Normal);
            assert_eq!(styles.len(), text.chars().count(), "{}", spec.name);
        }
    }
}
