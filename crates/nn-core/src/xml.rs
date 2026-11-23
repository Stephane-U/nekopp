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

//! A scanner for the subset of XML Notepad++ writes.
//!
//! This is not a parser. It does not build a tree, resolve namespaces, or
//! validate anything. It walks a string looking for `<name ...>` tags and
//! returns each one's attributes and the text between its tags, which is
//! exactly what `config.xml`, `shortcuts.xml` and `userDefineLang.xml` need.
//!
//! The reason for a scanner rather than a dependency: the three files are
//! written by one program, in one style, with quoted attributes and no CDATA,
//! no namespaces, and no mixed content. A real parser would be several thousand
//! lines and a supply-chain surface for a job this size.
//!
//! It never fails. A malformed tag ends the scan and returns what was found so
//! far, because a configuration file that is half-readable should import its
//! readable half rather than nothing.

/// One element: its name, its attributes, and the text between its tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element<'a> {
    /// The tag name, without the angle brackets.
    pub name: &'a str,
    /// The `name="value"` pairs, in file order.
    pub attrs: Vec<(&'a str, &'a str)>,
    /// The text between the open and close tags, empty for a self-closing tag.
    pub body: &'a str,
}

/// Every `<name ...>` element in `text`, in file order.
///
/// The body runs from the `>` that closes the open tag to the matching
/// `</name>`. A tag that closes itself with `/>` has an empty body. A tag with
/// no close tag has an empty body and does not consume the rest of the text,
/// which is what keeps a truncated file from swallowing its neighbours.
///
/// Elements of the same name do not nest in any file Notepad++ writes, so the
/// first `</name>` is always the right one. A file that did nest them would
/// read the outer element's body as ending at the inner close tag; that is a
/// limitation of a scanner, and it is documented rather than worked around.
pub fn elements<'a>(text: &'a str, name: &str) -> Vec<Element<'a>> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        rest = &rest[start + 1..];
        // Comments, declarations and doctypes carry no attributes worth having.
        if rest.starts_with('!') || rest.starts_with('?') {
            match rest.find('>') {
                Some(end) => rest = &rest[end + 1..],
                None => break,
            }
            continue;
        }
        let Some(end) = tag_end(rest) else {
            break;
        };
        let tag = &rest[..end];
        rest = &rest[end + 1..];
        let self_closing = tag.ends_with('/');
        let tag_name = tag
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("");
        if tag_name != name {
            continue;
        }
        let body = if self_closing {
            ""
        } else {
            let close = format!("</{name}");
            match rest.find(&close) {
                Some(at) => {
                    let body = &rest[..at];
                    let after = &rest[at..];
                    rest = match after.find('>') {
                        Some(gt) => &after[gt + 1..],
                        None => "",
                    };
                    body
                }
                None => "",
            }
        };
        out.push(Element {
            name: tag_name,
            attrs: attributes(tag),
            body,
        });
    }
    out
}

/// The value of `name`, if the element carries it.
pub fn attribute<'a>(element: &Element<'a>, name: &str) -> Option<&'a str> {
    element
        .attrs
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| *value)
}

/// The index of the `>` that closes the tag starting at `text[0]`, ignoring any
/// that sits inside a quoted value.
fn tag_end(text: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (index, c) in text.char_indices() {
        match quote {
            Some(open) if c == open => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '>' => return Some(index),
            None => {}
        }
    }
    None
}

/// The `name="value"` pairs in a tag body.
///
/// Values may contain spaces -- `fontName="Courier New"` -- so the scan is
/// driven by the quotes rather than by whitespace. An unquoted value ends at
/// the next space, which is what XML allows and what a hand-edited file
/// sometimes contains.
fn attributes(tag: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = tag;
    while let Some(eq) = rest.find('=') {
        let name = rest[..eq]
            .rsplit(|c: char| c.is_whitespace())
            .next()
            .unwrap_or("");
        rest = &rest[eq + 1..];
        let Some(quote) = rest.chars().next() else {
            break;
        };
        if quote != '"' && quote != '\'' {
            let end = rest
                .find(|c: char| c.is_whitespace())
                .unwrap_or(rest.len());
            out.push((name, &rest[..end]));
            rest = &rest[end..];
            continue;
        }
        let body = &rest[1..];
        let Some(end) = body.find(quote) else {
            break;
        };
        out.push((name, &body[..end]));
        rest = &body[end + 1..];
    }
    out
}

/// Expands the five predefined entities.
///
/// Notepad++ escapes `&`, `<`, `>` and the quotes in attribute values, and a
/// user-defined language's operator list is full of `<` and `>`. Anything that
/// is not one of the five is left as it was written, including a bare `&`,
/// because inventing a replacement for a malformed entity would corrupt the
/// text rather than preserve it.
pub fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        // The longest entity is `&quot;`, so a semicolon further away than this
        // is not one.
        let entity = tail.find(';').filter(|semi| *semi <= 6).and_then(|semi| {
            let replacement = match &tail[1..semi] {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                _ => return None,
            };
            Some((replacement, semi + 1))
        });
        match entity {
            Some((c, len)) => {
                out.push(c);
                rest = &tail[len..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Escapes the five predefined entities, the inverse of [`unescape`].
///
/// Every character that would end an attribute value or start an entity is
/// replaced, so a language whose operator list contains `<` or `&` writes a
/// file that reads back as the same list.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_element_carries_its_attributes() {
        let found = elements(r#"<ScintKey menuCmdID="41005" Ctrl="yes" />"#, "ScintKey");
        assert_eq!(found.len(), 1);
        assert_eq!(attribute(&found[0], "menuCmdID"), Some("41005"));
        assert_eq!(attribute(&found[0], "Ctrl"), Some("yes"));
        assert_eq!(attribute(&found[0], "Alt"), None);
    }

    #[test]
    fn a_self_closing_tag_has_an_empty_body() {
        let found = elements(r#"<Global caseIgnored="no" />"#, "Global");
        assert_eq!(found[0].body, "");
    }

    #[test]
    fn a_body_runs_to_the_close_tag() {
        let found = elements(
            r#"<Keywords name="Keywords1">fn let mut</Keywords>"#,
            "Keywords",
        );
        assert_eq!(found.len(), 1);
        assert_eq!(attribute(&found[0], "name"), Some("Keywords1"));
        assert_eq!(found[0].body, "fn let mut");
    }

    #[test]
    fn several_elements_come_back_in_file_order() {
        let found = elements(
            r#"<Keywords name="a">1</Keywords><Keywords name="b">2</Keywords>"#,
            "Keywords",
        );
        assert_eq!(found.len(), 2);
        assert_eq!(attribute(&found[0], "name"), Some("a"));
        assert_eq!(found[1].body, "2");
    }

    #[test]
    fn a_value_may_contain_spaces() {
        let found = elements(r#"<WidgetStyle name="Default Style" fontSize="12" />"#, "WidgetStyle");
        assert_eq!(attribute(&found[0], "name"), Some("Default Style"));
        assert_eq!(attribute(&found[0], "fontSize"), Some("12"));
    }

    #[test]
    fn a_quote_inside_a_value_does_not_end_the_tag() {
        let found = elements(r#"<X a="he said > hi" b="2" />"#, "X");
        assert_eq!(attribute(&found[0], "a"), Some("he said > hi"));
        assert_eq!(attribute(&found[0], "b"), Some("2"));
    }

    #[test]
    fn comments_and_declarations_are_skipped() {
        let text = r#"<?xml version="1.0" ?><!-- <X a="1" /> --><X a="2" />"#;
        let found = elements(text, "X");
        assert_eq!(found.len(), 1);
        assert_eq!(attribute(&found[0], "a"), Some("2"));
    }

    #[test]
    fn a_truncated_tag_ends_the_scan_without_panicking() {
        assert!(elements(r#"<X a="1""#, "X").is_empty());
        assert!(elements("<", "X").is_empty());
        assert!(elements("", "X").is_empty());
    }

    #[test]
    fn a_tag_with_no_close_tag_does_not_swallow_its_neighbour() {
        let found = elements(r#"<X a="1"><Y b="2" />"#, "X");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].body, "");
    }

    #[test]
    fn the_five_entities_expand() {
        assert_eq!(unescape("a &amp; b"), "a & b");
        assert_eq!(unescape("&lt;&gt;"), "<>");
        assert_eq!(unescape("&quot;x&quot;"), "\"x\"");
        assert_eq!(unescape("&apos;"), "'");
    }

    #[test]
    fn a_bare_ampersand_is_left_alone() {
        assert_eq!(unescape("a & b"), "a & b");
        assert_eq!(unescape("&notanentity;"), "&notanentity;");
        assert_eq!(unescape("100% &"), "100% &");
    }

    #[test]
    fn escaping_and_unescaping_are_inverses() {
        for text in ["a & b", "<>", "\"x\"", "'y'", "plain", "", "a&amp;b"] {
            assert_eq!(unescape(&escape(text)), text, "for {text:?}");
        }
    }

    #[test]
    fn escaping_replaces_every_markup_character() {
        assert_eq!(escape("<&>"), "&lt;&amp;&gt;");
        assert_eq!(escape("\"q\""), "&quot;q&quot;");
    }

    #[test]
    fn text_without_an_ampersand_is_returned_unchanged() {
        assert_eq!(unescape("plain"), "plain");
        assert_eq!(unescape(""), "");
    }
}
