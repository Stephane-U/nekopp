//! Read, re-emit and check JSON.
//!
//! Three commands over one parser: the two printers write a document in the
//! shape a person reads or the shape a machine gets, and the check answers the
//! question the printers only answer by refusing -- what is wrong, and where.
//!
//! The parser is recursive descent written from RFC 8259, with no crate behind
//! it: JSON is small enough that a dependency would be bigger than the parser.
//! It keeps the spelling the document was written with. Object keys stay in
//! order, `1e10` stays `1e10`, and a string keeps its escapes, because a
//! formatter that reorders keys or expands that exponent has changed the
//! document rather than formatted it. Strings and numbers are copied as the
//! bytes they were written with for exactly that reason, so every escape has to
//! be validated on the way past -- otherwise a string could be carried into the
//! output without anyone having read it.
//!
//! Written here rather than translated: the Notepad++ plugins with commands
//! like these are GPL, and this repository has no licence of its own yet. See
//! `plugins/README.md`.

use std::ops::Range;

use nn_plugin::host;

/// How deep a value may nest before the parser gives up.
///
/// Recursive descent costs one stack frame per level, and a plugin runs in a
/// module with a fixed stack, so the limit is the difference between a log line
/// and a trap. Deep enough for any document a person reads, shallow enough that
/// generated nonsense cannot exhaust the stack.
const MAX_DEPTH: usize = 512;

/// The refusals that more than one place can produce, said once so that the two
/// printers and the check agree on the wording.
const DEEP: &str = "nesting is deeper than this parser allows";
const TRAILING_COMMA: &str = "trailing commas are not JSON";
const LONE_SURROGATE: &str = "a lone surrogate is not a character";

/// One refusal: where the parser stopped, and what it wanted there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// The byte offset of the offending character, counted from the start of
    /// the text that was parsed.
    pub offset: usize,
    /// One line, lowercase, with no full stop: in the log it follows a colon.
    pub message: String,
}

/// An [`Error`] at `offset`.
fn error(at: usize, message: &str) -> Error {
    Error {
        offset: at,
        message: message.to_string(),
    }
}

/// A parsed JSON value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// A string literal, copied exactly as written, quotes included.
    ///
    /// The escapes are kept rather than decoded and re-encoded: `"\u00e9"` and
    /// `"é"` are the same string, and choosing one for the user is a change a
    /// formatter has no reason to make. The parser validates every escape on
    /// the way past, so a `Value` always holds a well-formed literal.
    String(String),
    /// A number, copied exactly as written: `1e10` does not become
    /// `10000000000`.
    Number(String),
    /// `true` or `false`.
    Bool(bool),
    /// `null`.
    Null,
    /// An object's members, in the order the document wrote them. Keys are
    /// stored the way [`Value::String`] is, quotes included.
    Object(Vec<(String, Value)>),
    /// An array's items, in document order.
    Array(Vec<Value>),
}

/// Parses `text` as one JSON value, with nothing but whitespace after it.
///
/// A document of a single scalar is a document: RFC 8259 allows any value at
/// the top level, so `42` and `"hi"` are parsed rather than refused.
pub fn parse(text: &str) -> Result<Value, Error> {
    let mut parser = Parser::new(text);
    parser.skip_whitespace();
    if parser.at == parser.bytes.len() {
        return Err(error(parser.at, "expected a value"));
    }
    let value = parser.value(0)?;
    parser.skip_whitespace();
    if parser.at < parser.bytes.len() {
        return Err(error(parser.at, "unexpected text after the value"));
    }
    Ok(value)
}

/// The cursor the parser reads through.
struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Parser {
            text,
            bytes: text.as_bytes(),
            at: 0,
        }
    }

    /// The byte at the cursor, or `None` at the end of the text.
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    /// The rest of the text from the cursor.
    fn rest(&self) -> &'a str {
        self.text.get(self.at..).unwrap_or_default()
    }

    /// Steps over the four characters JSON counts as whitespace.
    ///
    /// A form feed or a vertical tab is not one of them, so it is left where it
    /// stands for the value parser to refuse, which is what the format says.
    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    /// One value, with `depth` levels of brackets already open around it.
    fn value(&mut self, depth: usize) -> Result<Value, Error> {
        self.skip_whitespace();
        let at = self.at;
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Value::String),
            Some(b'-' | b'0'..=b'9') => self.number().map(Value::Number),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            // The tokens people write from memory and the format does not
            // have. Naming them is more use than "expected a value".
            Some(b'\'') => Err(error(at, "single quotes are not JSON strings")),
            Some(b'/') => Err(error(at, "JSON has no comments")),
            Some(b'N') => Err(error(at, "NaN is not a JSON value")),
            Some(b'I') => Err(error(at, "Infinity is not a JSON value")),
            Some(b'+') => Err(error(at, "a number may not start with `+`")),
            _ => Err(error(at, "expected a value")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, Error> {
        if depth >= MAX_DEPTH {
            return Err(error(self.at, DEEP));
        }
        self.at += 1; // the `{`
        let mut members: Vec<(String, Value)> = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Value::Object(members));
        }
        loop {
            self.skip_whitespace();
            let at = self.at;
            match self.peek() {
                Some(b'"') => {}
                Some(b'\'') => {
                    return Err(error(at, "single quotes are not JSON strings"));
                }
                None => return Err(error(at, "expected `}`")),
                _ => return Err(error(at, "expected a string key")),
            }
            let key = self.string()?;
            self.skip_whitespace();
            if self.peek() != Some(b':') {
                return Err(error(self.at, "expected a colon"));
            }
            self.at += 1;
            let value = self.value(depth + 1)?;
            members.push((key, value));
            self.skip_whitespace();
            let at = self.at;
            match self.peek() {
                Some(b',') => {
                    self.at += 1;
                    self.skip_whitespace();
                    if self.peek() == Some(b'}') {
                        return Err(error(self.at, TRAILING_COMMA));
                    }
                }
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Value::Object(members));
                }
                None => return Err(error(at, "expected `}`")),
                _ => return Err(error(at, "expected `,` or `}`")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, Error> {
        if depth >= MAX_DEPTH {
            return Err(error(self.at, DEEP));
        }
        self.at += 1; // the `[`
        let mut items: Vec<Value> = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.skip_whitespace();
            let at = self.at;
            match self.peek() {
                Some(b',') => {
                    self.at += 1;
                    self.skip_whitespace();
                    if self.peek() == Some(b']') {
                        return Err(error(self.at, TRAILING_COMMA));
                    }
                }
                Some(b']') => {
                    self.at += 1;
                    return Ok(Value::Array(items));
                }
                None => return Err(error(at, "expected `]`")),
                _ => return Err(error(at, "expected `,` or `]`")),
            }
        }
    }

    /// One string literal, from its opening quote to its closing one.
    fn string(&mut self) -> Result<String, Error> {
        let quote = self.at;
        self.at += 1;
        loop {
            match self.peek() {
                None => return Err(error(quote, "unterminated string")),
                Some(b'"') => {
                    self.at += 1;
                    // Sliced on a character boundary by construction: every
                    // step lands on an ASCII byte or steps a whole character.
                    let raw = self.text.get(quote..self.at).unwrap_or_default();
                    return Ok(raw.to_string());
                }
                Some(b'\\') => self.escape()?,
                Some(byte) if byte < 0x20 => {
                    return Err(error(
                        self.at,
                        "a raw control character is not allowed in a string",
                    ));
                }
                Some(_) => self.at += 1,
            }
        }
    }

    /// Steps over one escape sequence, starting at its backslash.
    fn escape(&mut self) -> Result<(), Error> {
        let at = self.at;
        self.at += 1; // the backslash
        match self.peek() {
            Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => {
                self.at += 1;
                Ok(())
            }
            Some(b'u') => {
                self.at += 1;
                let unit = self.hex4()?;
                if (0xDC00..=0xDFFF).contains(&unit) {
                    return Err(error(at, LONE_SURROGATE));
                }
                if (0xD800..=0xDBFF).contains(&unit) {
                    // A high surrogate is only half a character: the low half
                    // has to follow as its own escape, or the string has no
                    // character here at all.
                    if self.peek() != Some(b'\\') {
                        return Err(error(at, LONE_SURROGATE));
                    }
                    self.at += 1;
                    if self.peek() != Some(b'u') {
                        return Err(error(at, LONE_SURROGATE));
                    }
                    self.at += 1;
                    let low = self.hex4()?;
                    if !(0xDC00..=0xDFFF).contains(&low) {
                        return Err(error(at, LONE_SURROGATE));
                    }
                }
                Ok(())
            }
            Some(_) => Err(error(at, "unknown escape")),
            None => Err(error(at, "unterminated string")),
        }
    }

    /// The four hex digits of a `\u` escape, as the code unit they spell.
    fn hex4(&mut self) -> Result<u32, Error> {
        let mut unit = 0u32;
        for _ in 0..4 {
            let digit = match self.peek().and_then(hex_digit) {
                Some(digit) => digit,
                None => {
                    return Err(error(
                        self.at,
                        "expected four hex digits after `\\u`",
                    ));
                }
            };
            unit = unit * 16 + digit;
            self.at += 1;
        }
        Ok(unit)
    }

    /// One number, kept as the bytes the document used for it.
    fn number(&mut self) -> Result<String, Error> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(error(self.at, "expected a digit after `-`"));
            }
        }
        match self.peek() {
            Some(b'0') => {
                self.at += 1;
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    return Err(error(
                        self.at,
                        "a number may not start with a zero",
                    ));
                }
            }
            Some(b'1'..=b'9') => self.digits(),
            _ => return Err(error(self.at, "expected a digit")),
        }
        if self.peek() == Some(b'.') {
            self.at += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(error(
                    self.at,
                    "expected a digit after the decimal point",
                ));
            }
            self.digits();
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(error(self.at, "expected a digit in the exponent"));
            }
            self.digits();
        }
        Ok(self.text.get(start..self.at).unwrap_or_default().to_string())
    }

    /// Steps over the digits of a number's integer, fraction or exponent.
    fn digits(&mut self) {
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.at += 1;
        }
    }

    /// `true`, `false` or `null`, spelled out.
    fn literal(&mut self, word: &str, value: Value) -> Result<Value, Error> {
        if self.rest().starts_with(word) {
            self.at += word.len();
            Ok(value)
        } else {
            Err(error(self.at, &format!("expected `{word}`")))
        }
    }
}

/// The value of one hex digit.
fn hex_digit(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some(u32::from(byte - b'0')),
        b'a'..=b'f' => Some(u32::from(byte - b'a') + 10),
        b'A'..=b'F' => Some(u32::from(byte - b'A') + 10),
        _ => None,
    }
}

/// Re-emits `value` with two spaces of indentation per level.
pub fn pretty(value: &Value) -> String {
    let mut out = String::new();
    write_value(value, Some(0), &mut out);
    out
}

/// Re-emits `value` with no whitespace outside strings.
pub fn compact(value: &Value) -> String {
    let mut out = String::new();
    write_value(value, None, &mut out);
    out
}

/// Writes `value` out; `depth` is the level to indent to, `None` for compact.
///
/// One writer for both printers, because the difference between them is only
/// where the line breaks and the padding go -- two writers would be two places
/// to forget a comma.
fn write_value(value: &Value, depth: Option<usize>, out: &mut String) {
    match value {
        Value::String(raw) | Value::Number(raw) => out.push_str(raw),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Null => out.push_str("null"),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            let inner = depth.map(|level| level + 1);
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                break_line(inner, out);
                write_value(item, inner, out);
            }
            break_line(depth, out);
            out.push(']');
        }
        Value::Object(members) => {
            if members.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            let inner = depth.map(|level| level + 1);
            for (index, (key, member)) in members.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                break_line(inner, out);
                out.push_str(key);
                out.push(':');
                if depth.is_some() {
                    out.push(' ');
                }
                write_value(member, inner, out);
            }
            break_line(depth, out);
            out.push('}');
        }
    }
}

/// Breaks the line and indents to `depth`, or writes nothing when the output is
/// compact -- which is how the two printers share [`write_value`].
fn break_line(depth: Option<usize>, out: &mut String) {
    let Some(depth) = depth else {
        return;
    };
    out.push('\n');
    for _ in 0..depth {
        out.push_str("  ");
    }
}

/// The 1-based line and column of a byte offset.
///
/// The column counts characters, not bytes, because it is a number for a reader
/// looking at the line; the offset itself is a byte offset, because that is the
/// unit the caret takes.
pub fn line_column(text: &str, offset: usize) -> (usize, usize) {
    let at = boundary(text, offset);
    let before = text.get(..at).unwrap_or_default();
    let line = before.bytes().filter(|&byte| byte == b'\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |newline| newline + 1);
    let column = text
        .get(line_start..at)
        .map_or(1, |column| column.chars().count() + 1);
    (line, column)
}

/// `at`, moved back to the nearest character boundary of `text`.
fn boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

// ---- commands -------------------------------------------------------------

/// The byte range a command works on: the selection when there is one, the
/// whole document otherwise.
fn target(text: &str) -> Range<usize> {
    let selection = host::selection();
    if selection.is_empty() {
        return 0..text.len();
    }
    boundary(text, selection.start)..boundary(text, selection.end)
}

/// Re-emits the target text in the shape `print` gives it, and keeps it
/// selected: the two spellings are different lengths, so the old offsets would
/// land inside what the printer just wrote.
fn reformat(print: fn(&Value) -> String) {
    let text = host::text();
    let range = target(&text);
    let input = text.get(range.clone()).unwrap_or_default();
    match parse(input) {
        Ok(value) => {
            let out = print(&value);
            if !host::replace_range(range.start, range.len(), &out) {
                host::log("the document is read-only");
                return;
            }
            host::select(range.start, range.start + out.len());
        }
        Err(error) => report(&text, range.start, &error),
    }
}

/// Puts the caret on the offending byte and says where that byte is.
///
/// The error's offset counts from the start of the parsed text, which is the
/// selection when there was one; the log line and the caret are both in the
/// document's coordinates, because that is what the user is looking at.
fn report(text: &str, start: usize, error: &Error) {
    let at = boundary(text, start + error.offset);
    host::set_caret(at);
    let (line, column) = line_column(text, at);
    host::log(&format!(
        "invalid JSON at line {line}, column {column}: {}",
        error.message
    ));
}

fn pretty_print() {
    reformat(pretty);
}

fn minify() {
    reformat(compact);
}

fn validate() {
    let text = host::text();
    let range = target(&text);
    let input = text.get(range.clone()).unwrap_or_default();
    match parse(input) {
        Ok(_) => host::log("valid JSON"),
        Err(error) => report(&text, range.start, &error),
    }
}

nn_plugin::plugin! {
    name: "JSON",
    commands: [
        ("Pretty Print", pretty_print),
        ("Minify", minify),
        ("Validate", validate),
    ],
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A document with every shape the printers care about, already compact.
    const COMPACT: &str = concat!(
        "{\"name\":\"pocket\",\"items\":[1,2.5e3,{\"deep\":true}],",
        "\"empty\":{},\"none\":null}"
    );

    #[test]
    fn pretty_printing_indents_two_spaces_per_level() {
        let value = parse(COMPACT).expect("the document is valid");
        let expected = concat!(
            "{\n",
            "  \"name\": \"pocket\",\n",
            "  \"items\": [\n",
            "    1,\n",
            "    2.5e3,\n",
            "    {\n",
            "      \"deep\": true\n",
            "    }\n",
            "  ],\n",
            "  \"empty\": {},\n",
            "  \"none\": null\n",
            "}",
        );
        assert_eq!(pretty(&value), expected);
    }

    #[test]
    fn both_printers_round_trip_a_nested_document() {
        let once = parse(COMPACT).expect("the document is valid");
        let pretty_text = pretty(&once);
        let compact_text = compact(&once);
        assert_eq!(compact_text, COMPACT);
        let twice = parse(&pretty_text).expect("the pretty output is valid");
        assert_eq!(compact(&twice), compact_text);
        let thrice = parse(&compact_text).expect("the compact output is valid");
        assert_eq!(pretty(&thrice), pretty_text);
        // The pretty text is a fixed point: printing it again changes nothing.
        assert_eq!(pretty(&parse(&pretty_text).unwrap()), pretty_text);
    }

    #[test]
    fn a_number_keeps_the_spelling_it_was_written_with() {
        let text = concat!(
            "{\"exponent\":1e10,\"zero\":-0.0,\"plus\":1E+3,",
            "\"fraction\":0.5000,\"big\":1234567890123456789012}"
        );
        let value = parse(text).expect("the document is valid");
        assert_eq!(compact(&value), text);
        // The exponent and the digits survive the other printer too.
        let printed = pretty(&value);
        assert!(printed.contains("1e10"), "{printed}");
        assert!(printed.contains("1234567890123456789012"), "{printed}");
    }

    #[test]
    fn an_object_keeps_the_order_of_its_keys() {
        let text = r#"{"z":1,"a":2,"m":3}"#;
        assert_eq!(compact(&parse(text).expect("valid")), text);
    }

    #[test]
    fn a_string_keeps_its_escapes_and_a_surrogate_pair() {
        let text = r#"{"s":"\u00e9 \ud83d\ude00 \" \\ \/ \n \t"}"#;
        assert_eq!(compact(&parse(text).expect("valid")), text);
    }

    #[test]
    fn a_broken_escape_or_a_raw_control_character_is_refused() {
        let unknown = parse(r#""a\qb""#).expect_err("`\\q` is not an escape");
        assert_eq!(unknown.offset, 2);
        assert_eq!(unknown.message, "unknown escape");
        let short = parse(r#""a\u12""#).expect_err("`\\u` needs four digits");
        assert_eq!(short.offset, 6);
        let control = parse("\"a\tb\"").expect_err("a tab is raw");
        assert_eq!(control.offset, 2);
        let lone = parse(r#""\ud83d""#).expect_err("a lone high surrogate");
        assert_eq!(lone.offset, 1);
        assert_eq!(lone.message, "a lone surrogate is not a character");
    }

    #[test]
    fn a_trailing_comma_is_refused_where_it_stands() {
        let array = parse("[1, 2, ]").expect_err("trailing comma");
        assert_eq!(array.offset, 7);
        assert_eq!(array.message, "trailing commas are not JSON");
        let object = parse(r#"{"a": 1,}"#).expect_err("same in an object");
        assert_eq!(object.offset, 8);
    }

    #[test]
    fn comments_and_single_quotes_are_not_json() {
        let leading = parse("// note\n1").expect_err("no comments");
        assert_eq!(leading.offset, 0);
        assert_eq!(leading.message, "JSON has no comments");
        let trailing = parse("[1] // note").expect_err("no comment here");
        assert_eq!(trailing.offset, 4);
        let quotes = parse("{'a': 1}").expect_err("single quotes");
        assert_eq!(quotes.offset, 1);
        assert_eq!(quotes.message, "single quotes are not JSON strings");
    }

    #[test]
    fn nan_and_infinity_are_not_values() {
        let nan = parse("NaN").expect_err("there is no NaN");
        assert_eq!(nan.message, "NaN is not a JSON value");
        let infinity = parse("-Infinity").expect_err("there is no Infinity");
        assert_eq!(infinity.message, "expected a digit after `-`");
    }

    #[test]
    fn trailing_garbage_after_the_value_is_refused() {
        let error = parse("{} {}").expect_err("two values are not a document");
        assert_eq!(error.offset, 3);
        assert_eq!(error.message, "unexpected text after the value");
        let empty = parse("  ").expect_err("whitespace is not a value");
        assert_eq!(empty.message, "expected a value");
    }

    #[test]
    fn an_error_names_the_line_and_column_of_the_offending_byte() {
        let text = "{\n  \"a\": 1,\n  \"b\" 2\n}";
        let error = parse(text).expect_err("the second member has no colon");
        assert_eq!(error.message, "expected a colon");
        assert_eq!(error.offset, 18);
        assert_eq!(line_column(text, error.offset), (3, 7));
        // A command that parsed a selection adds its start back on before it
        // reports, so the line and the column stay the document's.
        let whole = format!("prefix {text}");
        assert_eq!(line_column(&whole, 7 + error.offset), (3, 7));
    }

    #[test]
    fn a_byte_offset_counts_bytes_not_characters() {
        // Two two-byte characters before the mistake, so the byte offset and
        // the column disagree -- and the column is the one a person counts.
        let text = "{\"éé\": 1 2}";
        let error = parse(text).expect_err("the members are not separated");
        assert_eq!(error.message, "expected `,` or `}`");
        assert_eq!(error.offset, 11);
        assert_eq!(line_column(text, error.offset), (1, 10));
    }
}
