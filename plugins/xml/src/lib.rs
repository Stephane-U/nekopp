//! Re-indent, flatten and check XML.
//!
//! Five commands over one tokenizer. Three of them are about structure -- the
//! two printers that move a document between the layout a person reads and the
//! layout a wire protocol sends, and the check that says what is wrong when one
//! of them refuses. The other two are about the five characters XML reserves,
//! which is the edit people reach for while pasting text into an element.
//!
//! The tokenizer is written from the XML 1.0 recommendation, with no crate
//! behind it. It keeps the bytes it was given: an entity reference stays as it
//! was written, a tag's attributes are copied as written, and a self-closing
//! tag keeps its spelling -- a formatter that rewrote `&#65;` as `A` or `<a />`
//! as `<a></a>` would be editing, not formatting.
//!
//! Written here rather than translated: the Notepad++ plugins with commands
//! like these are GPL, and this repository has no licence of its own yet. See
//! `plugins/README.md`.

use std::ops::Range;

use nn_plugin::host;

/// What a `&` has to be. Said once, because the checker and the unescaper
/// refuse the same thing and a second wording would only drift.
const REFERENCE: &str = concat!(
    "`&` must be `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;` ",
    "or a numeric reference",
);

/// Why an unescape is refused when its answer would still contain markup.
const BARE_AMPERSAND: &str = "the result would contain a bare &";
const BARE_LT: &str = "the result would contain a bare <";

/// One refusal: the byte offset of the offending character, and what was wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// The byte offset of the offending character, counted from the start of
    /// the document.
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

/// What a token is, with the parts of it a single byte cannot describe.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind<'a> {
    /// Character data, with its entity references left as written.
    Text,
    /// `<name ...>` or `<name .../>`, with the attribute names in order.
    Start {
        name: &'a str,
        attributes: Vec<&'a str>,
        self_closing: bool,
    },
    /// `</name>`.
    End { name: &'a str },
    /// `<!-- ... -->`.
    Comment,
    /// `<![CDATA[ ... ]]>`.
    CData,
    /// `<? ... ?>`, the XML declaration among them.
    Instruction,
    /// `<!DOCTYPE ...>`.
    DocType,
}

/// A token and the byte range of the document it covers.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Token<'a> {
    kind: Kind<'a>,
    start: usize,
    end: usize,
}

impl<'a> Token<'a> {
    /// The document's bytes this token covers, exactly as they were written.
    fn text(&self, text: &'a str) -> &'a str {
        text.get(self.start..self.end).unwrap_or_default()
    }
}

/// Splits `text` into tokens, refusing anything XML does not allow in one.
fn tokenize(text: &str) -> Result<Vec<Token<'_>>, Error> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let token = if bytes.get(at) == Some(&b'<') {
            markup(text, at)?
        } else {
            let mut end = at;
            while bytes.get(end).is_some_and(|&byte| byte != b'<') {
                end += 1;
            }
            check_text(text, at, end)?;
            Token {
                kind: Kind::Text,
                start: at,
                end,
            }
        };
        at = token.end;
        tokens.push(token);
    }
    Ok(tokens)
}

/// The markup token starting at `at`, where the byte there is `<`.
fn markup<'a>(text: &'a str, at: usize) -> Result<Token<'a>, Error> {
    let rest = text.get(at..).unwrap_or_default();
    if rest.starts_with("<!--") {
        let end = terminated(text, at, 4, "-->", "unterminated comment")?;
        let body = text.get(at + 4..end.saturating_sub(3)).unwrap_or_default();
        if let Some(dashes) = body.find("--") {
            return Err(error(
                at + 4 + dashes,
                "`--` is not allowed in a comment",
            ));
        }
        return Ok(Token {
            kind: Kind::Comment,
            start: at,
            end,
        });
    }
    if rest.starts_with("<![CDATA[") {
        let end = terminated(text, at, 9, "]]>", "unterminated CDATA section")?;
        return Ok(Token {
            kind: Kind::CData,
            start: at,
            end,
        });
    }
    if rest.starts_with("<!DOCTYPE") {
        let end = doctype_end(text, at)?;
        return Ok(Token {
            kind: Kind::DocType,
            start: at,
            end,
        });
    }
    if rest.starts_with("<!") {
        return Err(error(at, "unknown markup declaration"));
    }
    if rest.starts_with("<?") {
        return instruction(text, at);
    }
    if rest.starts_with("</") {
        return end_tag(text, at);
    }
    start_tag(text, at)
}

/// Where the `needle` that closes a token ends, searching from `from` bytes
/// past `at`.
fn terminated(
    text: &str,
    at: usize,
    from: usize,
    needle: &str,
    message: &str,
) -> Result<usize, Error> {
    let rest = text.get(at + from..).unwrap_or_default();
    match rest.find(needle) {
        Some(found) => Ok(at + from + found + needle.len()),
        None => Err(error(at, message)),
    }
}

/// The end of a `<!DOCTYPE ...>` declaration, which may carry an internal
/// subset in square brackets that itself holds a `>`.
fn doctype_end(text: &str, at: usize) -> Result<usize, Error> {
    let bytes = text.as_bytes();
    let mut bracket = 0usize;
    let mut index = at + "<!DOCTYPE".len();
    while let Some(&byte) = bytes.get(index) {
        match byte {
            b'[' => bracket += 1,
            b']' => bracket = bracket.saturating_sub(1),
            b'>' if bracket == 0 => return Ok(index + 1),
            _ => {}
        }
        index += 1;
    }
    Err(error(at, "unterminated DOCTYPE declaration"))
}

/// A `<? ... ?>`, checked for the one target that has a rule of its own.
fn instruction<'a>(text: &'a str, at: usize) -> Result<Token<'a>, Error> {
    let target_at = at + 2;
    let (target, _) = name_at(text, target_at)
        .ok_or_else(|| error(target_at, "expected a target after `<?`"))?;
    if target.eq_ignore_ascii_case("xml") && at != 0 {
        return Err(error(
            at,
            "the XML declaration is only allowed at the start of the document",
        ));
    }
    let end = terminated(
        text,
        at,
        2,
        "?>",
        "unterminated processing instruction",
    )?;
    Ok(Token {
        kind: Kind::Instruction,
        start: at,
        end,
    })
}

/// A `</name>`, where XML allows whitespace before the closing `>`.
fn end_tag<'a>(text: &'a str, at: usize) -> Result<Token<'a>, Error> {
    let bytes = text.as_bytes();
    let start = at + 2;
    let (name, after_name) = name_at(text, start)
        .ok_or_else(|| error(start, "expected an element name after `</`"))?;
    let mut index = after_name;
    while bytes.get(index).is_some_and(|&byte| is_space_byte(byte)) {
        index += 1;
    }
    if bytes.get(index) != Some(&b'>') {
        return Err(error(index, "expected `>` to close the end tag"));
    }
    Ok(Token {
        kind: Kind::End { name },
        start: at,
        end: index + 1,
    })
}

/// A `<name ...>` or `<name .../>`, with its attributes read one by one.
fn start_tag<'a>(text: &'a str, at: usize) -> Result<Token<'a>, Error> {
    let bytes = text.as_bytes();
    let mut index = at + 1;
    let (name, after_name) = name_at(text, index)
        .ok_or_else(|| error(index, "expected an element name after `<`"))?;
    index = after_name;
    let mut attributes: Vec<&'a str> = Vec::new();
    loop {
        let mut spaced = false;
        while bytes.get(index).is_some_and(|&byte| is_space_byte(byte)) {
            index += 1;
            spaced = true;
        }
        match bytes.get(index).copied() {
            Some(b'>') => {
                return Ok(Token {
                    kind: Kind::Start {
                        name,
                        attributes,
                        self_closing: false,
                    },
                    start: at,
                    end: index + 1,
                });
            }
            Some(b'/') => {
                if bytes.get(index + 1) == Some(&b'>') {
                    return Ok(Token {
                        kind: Kind::Start {
                            name,
                            attributes,
                            self_closing: true,
                        },
                        start: at,
                        end: index + 2,
                    });
                }
                return Err(error(index + 1, "expected `>` after `/`"));
            }
            None => return Err(error(at, "unterminated start tag")),
            _ => {}
        }
        // Anything that starts another attribute has to be separated from what
        // came before it by whitespace: `<a href="x"src="y">` is not
        // well-formed, and neither is `<a="x">`.
        if !spaced {
            return Err(error(index, "expected whitespace before an attribute"));
        }
        let (attribute, after_attribute) = name_at(text, index)
            .ok_or_else(|| error(index, "expected an attribute name"))?;
        if attributes.contains(&attribute) {
            return Err(error(
                index,
                &format!("duplicate attribute `{attribute}` on one element"),
            ));
        }
        attributes.push(attribute);
        index = after_attribute;
        while bytes.get(index).is_some_and(|&byte| is_space_byte(byte)) {
            index += 1;
        }
        if bytes.get(index) != Some(&b'=') {
            return Err(error(index, "expected `=` after the attribute name"));
        }
        index += 1;
        while bytes.get(index).is_some_and(|&byte| is_space_byte(byte)) {
            index += 1;
        }
        let quote = match bytes.get(index).copied() {
            Some(quote @ (b'"' | b'\'')) => quote,
            _ => {
                return Err(error(index, "an attribute value has to be quoted"));
            }
        };
        index += 1;
        let value_start = index;
        loop {
            match bytes.get(index).copied() {
                None => {
                    return Err(error(
                        value_start,
                        "unterminated attribute value",
                    ));
                }
                Some(byte) if byte == quote => break,
                Some(b'<') => {
                    return Err(error(
                        index,
                        "`<` is not allowed in an attribute value",
                    ));
                }
                Some(_) => index += 1,
            }
        }
        check_data(text, value_start, index)?;
        index += 1; // past the closing quote
    }
}

/// The name starting at `at`, and where it ends.
fn name_at(text: &str, at: usize) -> Option<(&str, usize)> {
    let rest = text.get(at..)?;
    let first = rest.chars().next()?;
    if !is_name_start(first) {
        return None;
    }
    let mut end = first.len_utf8();
    for ch in rest.chars().skip(1) {
        if !is_name_byte(ch) {
            break;
        }
        end += ch.len_utf8();
    }
    Some((rest.get(..end)?, at + end))
}

/// Whether `ch` can start an XML name.
///
/// A name is a letter, an underscore or a colon first, then letters, digits,
/// `-`, `.` and `:`; "letter" is the Unicode notion, which `is_alphabetic`
/// already is, so a name outside ASCII is read rather than guessed at.
fn is_name_start(ch: char) -> bool {
    ch.is_alphabetic() || ch == '_' || ch == ':'
}

fn is_name_byte(ch: char) -> bool {
    is_name_start(ch) || ch.is_ascii_digit() || ch == '-' || ch == '.'
}

/// The four characters XML counts as whitespace.
fn is_space(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\r')
}

fn is_space_byte(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

/// Refuses what character data and an attribute value share: a character XML
/// allows nowhere, and a `&` that is not a reference this plugin accepts.
fn check_data(text: &str, start: usize, end: usize) -> Result<(), Error> {
    let bytes = text.as_bytes();
    let mut index = start;
    while index < end {
        match bytes.get(index).copied() {
            Some(b'&') => match reference_at(text, index) {
                Some((length, _)) => index += length,
                None => return Err(error(index, REFERENCE)),
            },
            Some(byte) if byte < 0x20 && !is_space_byte(byte) => {
                return Err(error(
                    index,
                    "a raw control character is not allowed",
                ));
            }
            Some(_) => index += 1,
            None => break,
        }
    }
    Ok(())
}

/// Character data, which has one rule of its own on top of [`check_data`].
fn check_text(text: &str, start: usize, end: usize) -> Result<(), Error> {
    check_data(text, start, end)?;
    let data = text.get(start..end).unwrap_or_default();
    if let Some(at) = data.find("]]>") {
        return Err(error(start + at, "`]]>` is not allowed in character data"));
    }
    Ok(())
}

/// A reference starting at `at`: how many bytes it spans, and the character it
/// stands for. `None` when the bytes there are not one this plugin accepts.
fn reference_at(text: &str, at: usize) -> Option<(usize, char)> {
    let rest = text.get(at..)?;
    let body = rest.strip_prefix('&')?;
    let close = body.find(';')?;
    let name = body.get(..close)?;
    let ch = match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        _ => {
            let digits = name.strip_prefix('#')?;
            let (digits, radix) = match digits.strip_prefix('x') {
                Some(hex) => (hex, 16),
                None => (digits, 10),
            };
            let numeric = !digits.is_empty()
                && digits.chars().all(|ch| ch.is_digit(radix));
            if !numeric {
                return None;
            }
            let value = u32::from_str_radix(digits, radix).ok()?;
            let ch = char::from_u32(value)?;
            if !is_xml_char(ch) {
                return None;
            }
            ch
        }
    };
    Some((1 + close + 1, ch))
}

/// The characters XML 1.0 allows in a document, as the recommendation lists
/// them: tab, newline, carriage return, then the three printable ranges.
fn is_xml_char(ch: char) -> bool {
    matches!(ch, '\t' | '\n' | '\r')
        || matches!(
            ch as u32,
            0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF
        )
}

/// Checks the parts of well-formedness a token cannot answer alone: matching
/// tags in order, one root element, and where a DOCTYPE and a CDATA may stand.
fn check_structure(tokens: &[Token<'_>], text: &str) -> Result<(), Error> {
    let mut open: Vec<(&str, usize)> = Vec::new();
    let mut roots = 0usize;
    let mut doctype = false;
    for token in tokens {
        match &token.kind {
            Kind::Text => {
                let data = text.get(token.start..token.end).unwrap_or_default();
                if open.is_empty() && !data.chars().all(is_space) {
                    return Err(error(
                        token.start,
                        "text outside the root element",
                    ));
                }
            }
            Kind::Start {
                name,
                self_closing,
                ..
            } => {
                if open.is_empty() {
                    roots += 1;
                    if roots > 1 {
                        return Err(error(
                            token.start,
                            "only one root element is allowed",
                        ));
                    }
                }
                if !*self_closing {
                    open.push((name, token.start));
                }
            }
            Kind::End { name } => match open.pop() {
                Some((open_name, _)) if open_name == *name => {}
                Some((open_name, _)) => {
                    let message =
                        format!("</{name}> does not close <{open_name}>");
                    return Err(error(token.start, &message));
                }
                None => {
                    let message = format!("</{name}> closes nothing");
                    return Err(error(token.start, &message));
                }
            },
            Kind::CData => {
                if open.is_empty() {
                    return Err(error(
                        token.start,
                        "a CDATA section is only allowed inside an element",
                    ));
                }
            }
            Kind::DocType => {
                if !open.is_empty() || roots > 0 || doctype {
                    return Err(error(
                        token.start,
                        "a DOCTYPE is allowed once, before the root element",
                    ));
                }
                doctype = true;
            }
            Kind::Comment | Kind::Instruction => {}
        }
    }
    if let Some((name, start)) = open.first() {
        return Err(error(*start, &format!("element <{name}> is never closed")));
    }
    if roots == 0 {
        return Err(error(0, "the document has no root element"));
    }
    Ok(())
}

/// Tokenizes `text` and checks everything no single token can decide.
fn parse(text: &str) -> Result<Vec<Token<'_>>, Error> {
    let tokens = tokenize(text)?;
    check_structure(&tokens, text)?;
    Ok(tokens)
}

/// Checks that `text` is a well-formed XML document.
pub fn well_formed(text: &str) -> Result<(), Error> {
    parse(text).map(|_| ())
}

/// What the printers need to know about each element.
struct Structure {
    /// For a start tag, the index of its matching end tag -- itself, for a tag
    /// that closes where it opens.
    close_of: Vec<Option<usize>>,
    /// For a start tag, whether it directly holds character data or a CDATA
    /// section.
    has_text: Vec<bool>,
    /// For a start tag, whether it directly holds another element.
    has_element: Vec<bool>,
}

impl Structure {
    /// Reads the spans out of a token list that has already been checked.
    fn of(tokens: &[Token<'_>], text: &str) -> Self {
        let mut close_of = vec![None; tokens.len()];
        let mut has_text = vec![false; tokens.len()];
        let mut has_element = vec![false; tokens.len()];
        let mut stack: Vec<usize> = Vec::new();
        for (index, token) in tokens.iter().enumerate() {
            match &token.kind {
                Kind::Text => {
                    let data = token.text(text);
                    if let Some(&top) = stack.last() {
                        if !data.chars().all(is_space) {
                            has_text[top] = true;
                        }
                    }
                }
                Kind::CData => {
                    // A CDATA section is character data, so an element holding
                    // one is mixed content: indenting around it would change
                    // what the document says, whatever the section holds.
                    if let Some(&top) = stack.last() {
                        has_text[top] = true;
                    }
                }
                Kind::Start { self_closing, .. } => {
                    if let Some(&top) = stack.last() {
                        has_element[top] = true;
                    }
                    if *self_closing {
                        close_of[index] = Some(index);
                    } else {
                        stack.push(index);
                    }
                }
                Kind::End { .. } => {
                    if let Some(open) = stack.pop() {
                        close_of[open] = Some(index);
                    }
                }
                Kind::Comment | Kind::Instruction | Kind::DocType => {}
            }
        }
        Structure {
            close_of,
            has_text,
            has_element,
        }
    }

    /// Whether the element with this start tag is written out as one unit,
    /// because indentation inside it would change its content: it holds text,
    /// or it holds no element at all.
    fn is_unit(&self, index: usize) -> bool {
        let has_element = self.has_element.get(index).copied().unwrap_or(false);
        let has_text = self.has_text.get(index).copied().unwrap_or(false);
        has_text || !has_element
    }
}

/// The document's bytes from `start` to `end`.
fn slice(text: &str, start: usize, end: usize) -> &str {
    text.get(start..end).unwrap_or_default()
}

/// `body` behind `depth` levels of two-space indentation.
fn indent(depth: usize, body: &str) -> String {
    let mut line = String::with_capacity(depth * 2 + body.len());
    for _ in 0..depth {
        line.push_str("  ");
    }
    line.push_str(body);
    line
}

/// Re-indents the element structure of `text`, two spaces per level.
///
/// Text-only elements, mixed content, CDATA, comments and processing
/// instructions are written back exactly as they were, because those are the
/// places where a newline is data and not layout. A self-closing tag keeps its
/// spelling: `<a />` does not become `<a></a>`. The output ends where the last
/// element ends -- no newline is appended -- so that running the command twice
/// adds nothing the first run did not.
pub fn pretty(text: &str) -> Result<String, Error> {
    let tokens = parse(text)?;
    let structure = Structure::of(&tokens, text);
    let mut lines: Vec<String> = Vec::new();
    let mut depth = 0usize;
    let mut index = 0;
    while let Some(token) = tokens.get(index) {
        match &token.kind {
            Kind::Text => {
                // Whitespace between tags is the thing this command rewrites.
                // A text node that carries data cannot arrive here: the checker
                // refuses text outside the root element, and an element holding
                // text is a unit, written out whole by the arm above.
                index += 1;
            }
            Kind::Start { .. } => {
                let close = structure.close_of.get(index).copied().flatten();
                if structure.is_unit(index) {
                    let end = close
                        .and_then(|close| tokens.get(close))
                        .map_or(token.end, |last| last.end);
                    lines.push(indent(depth, slice(text, token.start, end)));
                    index = close.map_or(index + 1, |close| close + 1);
                } else {
                    lines.push(indent(depth, token.text(text)));
                    depth += 1;
                    index += 1;
                }
            }
            Kind::End { .. } => {
                depth = depth.saturating_sub(1);
                lines.push(indent(depth, token.text(text)));
                index += 1;
            }
            Kind::Comment | Kind::CData | Kind::Instruction | Kind::DocType => {
                lines.push(indent(depth, token.text(text)));
                index += 1;
            }
        }
    }
    Ok(lines.join("\n"))
}

/// Strips the whitespace-only text between tags, and nothing else.
///
/// A text node with a non-whitespace character in it is left byte for byte,
/// because there its whitespace is the document's content: `<a> x </a>` keeps
/// both spaces. `xml:space` is ignored on purpose -- this command exists to
/// undo pretty printing, and demanding an annotation on every element first
/// would make it the one thing nobody runs.
pub fn linearize(text: &str) -> Result<String, Error> {
    let tokens = parse(text)?;
    let mut out = String::with_capacity(text.len());
    for token in &tokens {
        let data = token.text(text);
        match &token.kind {
            Kind::Text if data.chars().all(is_space) => {}
            _ => out.push_str(data),
        }
    }
    Ok(out)
}

/// Escapes the five characters XML reserves, one occurrence at a time.
///
/// An `&` that already starts a reference is escaped again, because the command
/// is asked to escape the text it is given and only the caller knows whether
/// the text has been through this before.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

/// The reverse: every reference in `text` becomes the character it stands for.
///
/// Refused when the answer would not be character data a document can hold --
/// a bare `&` that starts no reference, or a `<` that would open markup where
/// text was meant. That includes the two references whose answer is exactly
/// those bytes, `&amp;` and `&lt;`, and it includes a `&` or `<` the input
/// already carried, so what comes back can go straight into a document.
///
/// The refusal is a sentence rather than an [`Error`] because there is no
/// offending byte to name: what is wrong is in the answer, not in the text.
pub fn unescape(text: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < text.len() {
        let rest = text.get(index..).unwrap_or_default();
        let Some(ch) = rest.chars().next() else {
            break;
        };
        if ch == '<' {
            return Err(BARE_LT.to_string());
        }
        if ch == '&' {
            match reference_at(text, index) {
                // A reference whose answer is one of the two bytes markup owns
                // is refused for what it produces, not for what it is.
                Some((_, '&')) => return Err(BARE_AMPERSAND.to_string()),
                Some((_, '<')) => return Err(BARE_LT.to_string()),
                Some((length, ch)) => {
                    out.push(ch);
                    index += length;
                    continue;
                }
                None => return Err(BARE_AMPERSAND.to_string()),
            }
        }
        out.push(ch);
        index += ch.len_utf8();
    }
    Ok(out)
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

/// Re-indents or flattens the whole document, and keeps it selected.
///
/// The whole document rather than the selection, because both printers are
/// answered against the nesting that surrounds the text: a fragment's own
/// whitespace is not something either one can call insignificant.
fn reformat(format: fn(&str) -> Result<String, Error>) {
    let text = host::text();
    match format(&text) {
        Ok(out) => {
            if !host::replace_range(0, text.len(), &out) {
                host::log("the document is read-only");
                return;
            }
            host::select(0, out.len());
        }
        Err(error) => report(&text, &error),
    }
}

/// Puts the caret on the offending byte and says where that byte is.
fn report(text: &str, error: &Error) {
    let at = boundary(text, error.offset);
    host::set_caret(at);
    let (line, column) = line_column(text, at);
    host::log(&format!(
        "not well-formed at line {line}, column {column}: {}",
        error.message
    ));
}

/// The selection as byte offsets inside `text`, clamped to its bounds.
fn selection_in(text: &str) -> Range<usize> {
    let selection = host::selection();
    boundary(text, selection.start)..boundary(text, selection.end)
}

fn pretty_print() {
    reformat(pretty);
}

fn linearize_document() {
    reformat(linearize);
}

fn check_document() {
    let text = host::text();
    match well_formed(&text) {
        Ok(()) => host::log("well-formed XML"),
        Err(error) => report(&text, &error),
    }
}

fn escape_selection() {
    let text = host::text();
    let range = selection_in(&text);
    if range.is_empty() {
        host::log("select the text to escape");
        return;
    }
    let escaped = escape(slice(&text, range.start, range.end));
    if !host::replace_range(range.start, range.len(), &escaped) {
        host::log("the document is read-only");
        return;
    }
    host::select(range.start, range.start + escaped.len());
}

fn unescape_selection() {
    let text = host::text();
    let range = selection_in(&text);
    if range.is_empty() {
        host::log("select the text to unescape");
        return;
    }
    let plain = match unescape(slice(&text, range.start, range.end)) {
        Ok(plain) => plain,
        Err(reason) => {
            host::log(&format!("not unescaped: {reason}"));
            return;
        }
    };
    if !host::replace_range(range.start, range.len(), &plain) {
        host::log("the document is read-only");
        return;
    }
    host::select(range.start, range.start + plain.len());
}

nn_plugin::plugin! {
    name: "XML",
    commands: [
        ("Pretty Print", pretty_print),
        ("Linearize", linearize_document),
        ("Check Well-Formedness", check_document),
        ("Escape Entities", escape_selection),
        ("Unescape Entities", unescape_selection),
    ],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_printing_indents_the_element_structure() {
        let text = "<doc><a>one</a><b><c/></b></doc>";
        let expected = concat!(
            "<doc>\n",
            "  <a>one</a>\n",
            "  <b>\n",
            "    <c/>\n",
            "  </b>\n",
            "</doc>",
        );
        assert_eq!(pretty(text).expect("well-formed"), expected);
        // The declaration, the DOCTYPE and the root each get a line of their
        // own, and the DOCTYPE keeps the internal subset it came with.
        let prolog = concat!(
            "<?xml version=\"1.0\"?>",
            "<!DOCTYPE a [ <!ENTITY x \"y\"> ]><a/>",
        );
        let expected = concat!(
            "<?xml version=\"1.0\"?>\n",
            "<!DOCTYPE a [ <!ENTITY x \"y\"> ]>\n",
            "<a/>",
        );
        assert_eq!(pretty(prolog).expect("well-formed"), expected);
    }

    #[test]
    fn mixed_content_is_left_exactly_as_written() {
        let text = "<doc><p>Hello <b>world</b>, and\nmore.</p><i/></doc>";
        let expected = concat!(
            "<doc>\n",
            "  <p>Hello <b>world</b>, and\nmore.</p>\n",
            "  <i/>\n",
            "</doc>",
        );
        assert_eq!(pretty(text).expect("well-formed"), expected);
        // Linearizing leaves the same text node alone: it carries data.
        assert_eq!(linearize(text).expect("well-formed"), text);
    }

    #[test]
    fn cdata_comments_and_instructions_keep_their_insides() {
        let text = concat!(
            "<doc><!-- keep <this> -->",
            "<?pi a > b?>",
            "<data><![CDATA[a < b\n  && c]]></data>",
            "<item/></doc>",
        );
        let expected = concat!(
            "<doc>\n",
            "  <!-- keep <this> -->\n",
            "  <?pi a > b?>\n",
            "  <data><![CDATA[a < b\n  && c]]></data>\n",
            "  <item/>\n",
            "</doc>",
        );
        assert_eq!(pretty(text).expect("well-formed"), expected);
        // Neither printer can strip inside a CDATA run.
        let linear = linearize(text).expect("well-formed");
        assert!(linear.contains("<![CDATA[a < b\n  && c]]>"), "{linear}");
    }

    #[test]
    fn a_self_closing_tag_stays_as_written() {
        let text = "<doc><a/><b /><c  /></doc>";
        let expected = "<doc>\n  <a/>\n  <b />\n  <c  />\n</doc>";
        assert_eq!(pretty(text).expect("well-formed"), expected);
        // Running it again is the same document, not a second rewrite.
        assert_eq!(pretty(expected).expect("well-formed"), expected);
    }

    #[test]
    fn linearize_strips_only_the_whitespace_between_tags() {
        let text = "<doc>\n  <a> x </a>\n  <b>text\nkept</b>\n</doc>";
        assert_eq!(
            linearize(text).expect("well-formed"),
            "<doc><a> x </a><b>text\nkept</b></doc>"
        );
    }

    #[test]
    fn a_mismatched_tag_is_reported_where_it_stands() {
        let text = "<a>\n  <b>text</a></b>";
        let error = well_formed(text).expect_err("the tags cross");
        assert_eq!(error.offset, 13);
        assert_eq!(line_column(text, error.offset), (2, 10));
        assert!(error.message.contains("</a>"), "{}", error.message);
        assert!(error.message.contains("<b>"), "{}", error.message);
    }

    #[test]
    fn an_attribute_value_has_to_be_quoted() {
        let unquoted = well_formed("<a href=x>").expect_err("no quotes");
        assert_eq!(unquoted.offset, 8);
        assert_eq!(unquoted.message, "an attribute value has to be quoted");
        let separated = well_formed("<a href=\"x\"src=\"y\">")
            .expect_err("the attributes run together");
        assert_eq!(separated.offset, 11);
        let bracket = well_formed("<a href=\"a<b\">").expect_err("a `<`");
        assert_eq!(bracket.offset, 10);
        assert!(bracket.message.contains("`<`"), "{}", bracket.message);
    }

    #[test]
    fn a_duplicate_attribute_is_refused() {
        let error = well_formed("<a x=\"1\" x=\"2\"/>").expect_err("`x` twice");
        assert_eq!(error.offset, 9);
        assert!(error.message.contains("duplicate"), "{}", error.message);
    }

    #[test]
    fn only_predefined_entities_and_numeric_references_are_accepted() {
        let good = "<a>&amp;&lt;&gt;&quot;&apos;&#65;&#x42;</a>";
        assert!(well_formed(good).is_ok());
        let unknown = well_formed("<a>&nbsp;</a>").expect_err("no nbsp");
        assert_eq!(unknown.offset, 3);
        assert_eq!(unknown.message, REFERENCE);
        let digits = well_formed("<a>&#xZZ;</a>").expect_err("not a number");
        assert_eq!(digits.offset, 3);
        // A `&` that starts nothing at all is the same refusal.
        let bare = well_formed("<a>& </a>").expect_err("bare ampersand");
        assert_eq!(bare.offset, 3);
    }

    #[test]
    fn a_document_needs_exactly_one_root_element() {
        let two = well_formed("<a/><b/>").expect_err("two roots");
        assert_eq!(two.offset, 4);
        let outside = well_formed("<a></a>text").expect_err("text after it");
        assert_eq!(outside.offset, 7);
        let empty = well_formed("").expect_err("no root at all");
        assert_eq!(empty.offset, 0);
        // An element that is never closed is reported where it was opened.
        let unclosed = well_formed("<a><b/>").expect_err("`<a>` stays open");
        assert_eq!(unclosed.offset, 0);
        assert!(unclosed.message.contains("<a>"), "{}", unclosed.message);
    }

    #[test]
    fn escaping_round_trips_through_unescaping() {
        let text = "\"quoted\" and 'single' > bare";
        let escaped = escape(text);
        assert_eq!(
            escaped,
            "&quot;quoted&quot; and &apos;single&apos; &gt; bare"
        );
        assert_eq!(unescape(&escaped), Ok(text.to_string()));
        // Numeric references come back as the characters they stand for.
        assert_eq!(unescape("A &#65; &#x42;"), Ok("A A B".to_string()));
    }

    #[test]
    fn unescaping_refuses_a_result_with_a_bare_ampersand_or_angle_bracket() {
        let ampersand = unescape("a &amp; b").expect_err("`&` is markup");
        assert!(ampersand.contains("bare &"), "{ampersand}");
        let bracket = unescape("a &lt; b").expect_err("`<` is markup");
        assert!(bracket.contains("bare <"), "{bracket}");
        // A `&` or `<` the text already carried is refused for the same reason.
        assert!(unescape("a & b").is_err());
        assert!(unescape("a < b").is_err());
        // Which is why escaping and then unescaping cannot come back to `&`.
        assert!(unescape(&escape("a & b")).is_err());
        assert_eq!(unescape("plain"), Ok("plain".to_string()));
    }
}
