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
                    let mut j = i;
                    while j < chars.len()
                        && (chars[j].is_alphanumeric() || chars[j] == '.' || chars[j] == '_')
                    {
                        j += 1;
                    }
                    styles[i..j].fill(style::NUMBER);
                    i = j;
                }
                c if c.is_alphabetic() || c == '_' => {
                    let mut j = i;
                    while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                        j += 1;
                    }
                    let word: String = chars[i..j].iter().collect();
                    let style = classify(&word, chars.get(j) == Some(&'('));
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
/// Ninety-odd languages differ in three things a line-at-a-time tokeniser can
/// see: their comment markers, their quote characters, and their keyword list.
/// Writing ninety lexers would produce ninety files that differ only in their
/// tables, so the tables live in `lang` and this one lexer reads them.
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
/// 3. Numbers before identifiers, so `0x1f` is one number rather than a number
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
                match find_string_end(&chars, i, c) {
                    Some(end) => {
                        styles[i..end].fill(style::STRING);
                        i = end;
                    }
                    None => {
                        styles[i..].fill(style::STRING);
                        return (styles, LexState::String(c));
                    }
                }
                continue;
            }
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
                let style = if self.spec.is_keyword(&word) {
                    style::KEYWORD
                } else if chars.get(j) == Some(&'(') {
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
fn find_char_literal_end(chars: &[char], from: usize) -> Option<usize> {
    let mut j = from + 1;
    let mut escaped = false;
    while j < chars.len() && j <= from + 4 {
        if escaped {
            escaped = false;
            j += 1;
            continue;
        }
        match chars[j] {
            '\\' => escaped = true,
            '\'' => return Some(j + 1),
            _ => {}
        }
        j += 1;
    }
    None
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
        assert_eq!(styles[3], style::DEFAULT);
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
