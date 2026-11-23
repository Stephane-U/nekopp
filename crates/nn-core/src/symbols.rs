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

//! A cheap symbol scan for the function list.
//!
//! This is not a parser. It reads one line at a time and matches the shapes that
//! declare a named thing in the languages people open in Notepad++: Rust, C and
//! C++, Python, JavaScript and TypeScript, Go, Java and C#. A real parser per
//! language is a dependency per language and a grammar to keep current; this is
//! a few hundred lines and gets the list right for code that is already indented
//! the way its language expects.
//!
//! The one structural rule it does use is indentation, and only to indent the
//! panel: a symbol declared four spaces in is drawn one level in. That is a
//! display hint, not a scope model — nothing here knows what a block is.

/// What a symbol is, for the label in the panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Function,
    Struct,
    Enum,
    Trait,
    Impl,
    Module,
    Class,
    Const,
    Macro,
}

impl Kind {
    /// The short tag shown next to the name.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Function => "fn",
            Kind::Struct => "struct",
            Kind::Enum => "enum",
            Kind::Trait => "trait",
            Kind::Impl => "impl",
            Kind::Module => "mod",
            Kind::Class => "class",
            Kind::Const => "const",
            Kind::Macro => "macro",
        }
    }
}

/// One entry in the function list.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Symbol {
    pub name: String,
    pub kind: Kind,
    /// Zero-based document line the declaration starts on.
    pub line: usize,
    /// Indentation level, capped, for drawing the tree.
    pub depth: u8,
}

/// Prefixes that decorate a declaration without changing what it declares.
///
/// Stripped repeatedly, so `pub(crate) async unsafe fn` reduces to `fn`.
const QUALIFIERS: &[&str] = &[
    "pub(crate) ",
    "pub(super) ",
    "pub(in crate) ",
    "pub ",
    "async ",
    "unsafe ",
    "extern ",
    "export ",
    "default ",
    "public ",
    "private ",
    "protected ",
    "abstract ",
    "final ",
    "override ",
    "virtual ",
    "inline ",
    "mut ",
];

/// Words that look like a call but open a block instead of declaring anything.
const NOT_A_NAME: &[&str] = &[
    "if", "for", "while", "switch", "return", "match", "catch", "sizeof", "else", "do", "new",
    "delete", "loop", "unless", "until", "with", "assert",
];

/// Scans `text` for declarations, in document order.
pub fn scan(text: &str) -> Vec<Symbol> {
    let mut out = Vec::new();
    for (line, raw) in text.lines().enumerate() {
        if let Some(symbol) = scan_line(raw, line) {
            out.push(symbol);
        }
    }
    out
}

/// Scans one line, returning the declaration it starts, if any.
pub(crate) fn scan_line(raw: &str, line: usize) -> Option<Symbol> {
    let trimmed = raw.trim_start();
    let first = trimmed.as_bytes().first()?;
    // Comments, attributes, preprocessor lines, and closing braces. `#` covers
    // both Rust attributes and Python decorators, neither of which declares
    // anything the list should show.
    if matches!(first, b'/' | b'*' | b'#' | b'-' | b'%' | b'}') {
        return None;
    }

    let body = strip_qualifiers(trimmed);
    // `const` and `static` are both a declaration keyword and a qualifier, so
    // they only declare a constant when what follows is a name and a value.
    // `const fn` and `static void helper(void)` fall through to the chain below.
    for prefix in ["const ", "static "] {
        if let Some(rest) = body.strip_prefix(prefix) {
            if is_const_decl(rest) {
                let name = ident_after(strip_qualifiers(rest))?;
                return Some(Symbol {
                    name: name.to_string(),
                    kind: Kind::Const,
                    line,
                    depth: depth_of(raw),
                });
            }
        }
    }
    let (kind, rest) = if let Some(rest) = body.strip_prefix("const fn ") {
        (Kind::Function, rest)
    } else if let Some(rest) = body.strip_prefix("fn ") {
        (Kind::Function, rest)
    } else if let Some(rest) = body.strip_prefix("def ") {
        (Kind::Function, rest)
    } else if let Some(rest) = body.strip_prefix("function ") {
        (Kind::Function, rest)
    } else if let Some(rest) = body.strip_prefix("struct ") {
        (Kind::Struct, rest)
    } else if let Some(rest) = body.strip_prefix("enum ") {
        (Kind::Enum, rest)
    } else if let Some(rest) = body.strip_prefix("trait ") {
        (Kind::Trait, rest)
    } else if let Some(rest) = body.strip_prefix("impl") {
        (Kind::Impl, rest)
    } else if let Some(rest) = body.strip_prefix("mod ") {
        (Kind::Module, rest)
    } else if let Some(rest) = body.strip_prefix("class ") {
        (Kind::Class, rest)
    } else if let Some(rest) = body.strip_prefix("macro_rules!") {
        (Kind::Macro, rest)
    } else {
        return scan_call(trimmed, raw, line);
    };

    let name = if kind == Kind::Impl {
        impl_name(rest)
    } else {
        ident_after(strip_qualifiers(rest)).map(str::to_string)
    }?;
    Some(Symbol {
        name,
        kind,
        line,
        depth: depth_of(raw),
    })
}

/// Whether `rest`, the text after `const` or `static`, declares a constant.
///
/// A constant has a name and a value; a function has a parameter list.
fn is_const_decl(rest: &str) -> bool {
    !rest.contains('(') && (rest.contains(':') || rest.contains('='))
}

/// Removes every leading qualifier from `text`.
fn strip_qualifiers(mut text: &str) -> &str {
    loop {
        let before = text;
        for prefix in QUALIFIERS {
            if let Some(rest) = text.strip_prefix(prefix) {
                text = rest;
            }
        }
        if text.len() == before.len() {
            return text;
        }
    }
}

/// The first identifier in `text`, skipping a leading generic parameter list.
fn ident_after(text: &str) -> Option<&str> {
    let text = text.trim_start();
    let text = if let Some(rest) = text.strip_prefix('<') {
        let close = rest.find('>')?;
        rest[close + 1..].trim_start()
    } else {
        text
    };
    let end = text
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(text.len());
    (end > 0).then(|| &text[..end])
}

/// The name of an `impl` block: the type, plus the trait when there is one.
///
/// `impl Default for Document` reads better as `Default for Document` than as
/// `Default`, and the panel has room for it.
fn impl_name(text: &str) -> Option<String> {
    let text = text.trim_start();
    let text = if let Some(rest) = text.strip_prefix('<') {
        let close = rest.find('>')?;
        rest[close + 1..].trim_start()
    } else {
        text
    };
    let end = text.find(['{', ';']).unwrap_or(text.len());
    let name: String = text[..end].split_whitespace().collect::<Vec<_>>().join(" ");
    (!name.is_empty()).then_some(name)
}

/// The indentation level of `raw`, four columns per level, capped at eight.
fn depth_of(raw: &str) -> u8 {
    let mut columns = 0usize;
    for c in raw.chars() {
        match c {
            ' ' => columns += 1,
            '\t' => columns += 4,
            _ => break,
        }
    }
    (columns / 4).min(8) as u8
}

/// Matches a C-like declaration: `int main(int argc, char **argv) {`.
///
/// The line has to open a block, which is what separates a declaration from a
/// call: `foo(1);` has a semicolon before any brace, `void foo() {` does not.
fn scan_call(trimmed: &str, raw: &str, line: usize) -> Option<Symbol> {
    let open = trimmed.find('(')?;
    let head = trimmed[..open].trim_end();
    let start = head
        .rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '~'))
        .map_or(0, |i| i + 1);
    let name = &head[start..];
    if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    if NOT_A_NAME.contains(&name) {
        return None;
    }
    // A declaration opens a block, or ends at the parameter list with the brace
    // on the next line. A call ends in a semicolon. The Allman form needs a
    // return type in front of the name, which is what keeps `print(x)` and
    // `self.assertEqual(a, b)` out of the list.
    let opens_block = match trimmed.find('{') {
        Some(brace) => !trimmed[..brace].contains(';'),
        None => head.contains(' ') && trimmed.ends_with(')') && !trimmed.contains(';'),
    };
    if !opens_block {
        return None;
    }
    Some(Symbol {
        name: name.to_string(),
        kind: Kind::Function,
        line,
        depth: depth_of(raw),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(text: &str) -> Vec<(String, Kind, usize)> {
        scan(text)
            .into_iter()
            .map(|s| (s.name, s.kind, s.line))
            .collect()
    }

    #[test]
    fn rust_functions_and_types() {
        let text = "\
pub struct Document {
    rope: Rope,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) async fn save(path: &Path) -> io::Result<()> {
    Ok(())
}
";
        assert_eq!(
            names(text),
            vec![
                ("Document".to_string(), Kind::Struct, 0),
                ("Default for Document".to_string(), Kind::Impl, 4),
                ("default".to_string(), Kind::Function, 5),
                ("save".to_string(), Kind::Function, 10),
            ]
        );
    }

    #[test]
    fn generic_parameters_are_skipped() {
        let text = "impl<T> From<T> for Wrapper<T> {\nfn get<T: Clone>(&self) -> T {\n";
        assert_eq!(
            names(text),
            vec![
                ("From<T> for Wrapper<T>".to_string(), Kind::Impl, 0),
                ("get".to_string(), Kind::Function, 1),
            ]
        );
    }

    #[test]
    fn python_defs_and_classes() {
        let text = "\
class Parser:
    def parse(self, text):
        pass

def main():
    pass
";
        assert_eq!(
            names(text),
            vec![
                ("Parser".to_string(), Kind::Class, 0),
                ("parse".to_string(), Kind::Function, 1),
                ("main".to_string(), Kind::Function, 4),
            ]
        );
    }

    #[test]
    fn c_declarations_are_functions_but_calls_are_not() {
        let text = "\
int main(int argc, char **argv) {
    printf(\"hi\");
    return 0;
}

static void helper(void)
{
}
";
        assert_eq!(
            names(text),
            vec![
                ("main".to_string(), Kind::Function, 0),
                ("helper".to_string(), Kind::Function, 5),
            ]
        );
    }

    #[test]
    fn control_flow_is_not_a_declaration() {
        let text = "if (x) {\nwhile (y) {\nfor (;;) {\nswitch (z) {\n";
        assert!(names(text).is_empty());
    }

    #[test]
    fn comments_and_attributes_are_skipped() {
        let text = "\
// fn commented_out() {
/// fn documented() {
#[derive(Debug)]
# def decorated(self):
* fn in_a_block_comment() {
";
        assert!(names(text).is_empty());
    }

    #[test]
    fn consts_and_statics_are_listed() {
        let text = "\
pub const FOLD_LEVEL_BASE: FoldLevel = 0x400;
static mut COUNTER: usize = 0;
const fn cheap() -> u8 { 0 }
";
        assert_eq!(
            names(text),
            vec![
                ("FOLD_LEVEL_BASE".to_string(), Kind::Const, 0),
                ("COUNTER".to_string(), Kind::Const, 1),
                ("cheap".to_string(), Kind::Function, 2),
            ]
        );
    }

    #[test]
    fn modules_traits_enums_and_macros() {
        let text = "\
mod tests {
trait Renderer {
enum Kind {
macro_rules! square {
";
        assert_eq!(
            names(text),
            vec![
                ("tests".to_string(), Kind::Module, 0),
                ("Renderer".to_string(), Kind::Trait, 1),
                ("Kind".to_string(), Kind::Enum, 2),
                ("square".to_string(), Kind::Macro, 3),
            ]
        );
    }

    #[test]
    fn depth_follows_indentation() {
        let text = "fn a() {\n    fn b() {\n        fn c() {\n";
        let depths: Vec<u8> = scan(text).iter().map(|s| s.depth).collect();
        assert_eq!(depths, vec![0, 1, 2]);
    }

    #[test]
    fn depth_counts_tabs_as_four_columns() {
        assert_eq!(depth_of("\tfn a()"), 1);
        assert_eq!(depth_of("\t\tfn a()"), 2);
        assert_eq!(depth_of("  fn a()"), 0);
    }

    #[test]
    fn depth_is_capped() {
        let deep = " ".repeat(200);
        assert_eq!(depth_of(&format!("{deep}fn a()")), 8);
    }

    #[test]
    fn a_qualified_name_keeps_only_the_last_segment() {
        let text = "void Foo::bar(int x) {\n";
        assert_eq!(names(text), vec![("bar".to_string(), Kind::Function, 0)]);
    }

    #[test]
    fn an_empty_document_has_no_symbols() {
        assert!(scan("").is_empty());
        assert!(scan("\n\n\n").is_empty());
    }

    #[test]
    fn labels_are_short() {
        assert_eq!(Kind::Function.label(), "fn");
        assert_eq!(Kind::Impl.label(), "impl");
    }

    #[test]
    fn line_numbers_are_zero_based_and_count_blank_lines() {
        let text = "\n\nfn third() {\n";
        assert_eq!(scan(text)[0].line, 2);
    }
}
