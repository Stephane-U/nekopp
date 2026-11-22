//! What the room is papered with.
//!
//! A playground room is made of real code, and the code is not the document the
//! user had open: a room that showed their file would be a screenshot of their
//! work in a place they cannot edit it, and it would make the mode look like a
//! view of the document rather than a place beside it. The material is a small
//! corpus instead, written for this purpose and compiled into the binary, so
//! there is nothing to install and no licence to carry.
//!
//! Eight flavours, because a room should look different from the last one: the
//! six that have a lexer in [`crate::lang`] are coloured by the editor's own
//! `ConfigLexer`, and EDI and CSV -- which Notepad++ has no language for -- are
//! split on their own delimiters. One code path colours all eight: a string and
//! one style per character.

use crate::document::Style;
use crate::lexer::{style, ConfigLexer, Lexer, LexState};

/// One kind of material a room can be built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavour {
    Rust,
    C,
    Java,
    Python,
    Json,
    Xml,
    /// UN/EDIFACT, the fixed-width world of orders and invoices.
    Edifact,
    /// Comma-separated values.
    Csv,
}

impl Flavour {
    /// Every flavour, in the order a room is chosen from.
    pub const ALL: [Flavour; 8] = [
        Flavour::Rust,
        Flavour::C,
        Flavour::Java,
        Flavour::Python,
        Flavour::Json,
        Flavour::Xml,
        Flavour::Edifact,
        Flavour::Csv,
    ];

    /// The name the corner of the room shows.
    pub fn name(self) -> &'static str {
        match self {
            Flavour::Rust => "rust",
            Flavour::C => "c",
            Flavour::Java => "java",
            Flavour::Python => "python",
            Flavour::Json => "json",
            Flavour::Xml => "xml",
            Flavour::Edifact => "edifact",
            Flavour::Csv => "csv",
        }
    }

    /// The built-in language that colours this flavour, when there is one.
    ///
    /// EDI and CSV have no entry in the Language menu, and inventing one would
    /// put two languages in that list that the editor cannot edit.
    pub fn language(self) -> Option<&'static str> {
        Some(match self {
            Flavour::Rust => "Rust",
            Flavour::C => "C",
            Flavour::Java => "Java",
            Flavour::Python => "Python",
            Flavour::Json => "JSON",
            Flavour::Xml => "XML",
            Flavour::Edifact | Flavour::Csv => return None,
        })
    }

    /// The lines this flavour is written in.
    pub fn lines(self) -> &'static [&'static str] {
        match self {
            Flavour::Rust => &[
                "fn main() {",
                "    let mut k = Kitten::new();",
                "    k.chase(&mouse);",
                "    println!(\"purr\");",
                "}",
            ],
            Flavour::C => &[
                "#include <stdio.h>",
                "int main(void) {",
                "    int laps = 9;",
                "    printf(\"%d\\n\", laps);",
                "    return 0;",
                "}",
            ],
            Flavour::Java => &[
                "public class Nap {",
                "    private int hours = 8;",
                "    public void sleep() {",
                "        System.out.println(hours);",
                "    }",
                "}",
            ],
            Flavour::Python => &[
                "def purr(times=3):",
                "    for n in range(times):",
                "        print(\"purr\", n)",
                "    return True",
            ],
            Flavour::Json => &[
                "{",
                "  \"name\": \"Mochi\",",
                "  \"lives\": 9,",
                "  \"naps\": [true, false, null]",
                "}",
            ],
            Flavour::Xml => &[
                "<room name=\"study\">",
                "  <!-- a kitten was here -->",
                "  <shelf units=\"3\" />",
                "  <lamp on=\"true\"/>",
                "</room>",
            ],
            Flavour::Edifact => &[
                "UNA:+.? '",
                "UNB+UNOC:3+SUPPLIER:14+BUYER:14+240101:1200+1'",
                "NAD+BY+SUPPLIER+++ACME LTD'",
                "LIN+1++WHISKAS:SA'",
                "QTY+21:9'",
                "UNT+4+1'",
            ],
            Flavour::Csv => &[
                "id,name,lives,naps",
                "1,Mochi,9,3",
                "2,Biscuit,7,5",
                "3,\"Sir Pounce\",8,2",
            ],
        }
    }

    /// Colours one line: a style per character.
    pub fn styles(self, line: &str) -> Vec<Style> {
        match self.language().and_then(spec) {
            Some(spec) => ConfigLexer::new(spec).lex_line(line, LexState::Normal).0,
            None => split_delimited(self, line),
        }
    }
}

/// The `LangSpec` a language name names, if this build has it.
fn spec(name: &str) -> Option<&'static crate::lang::LangSpec> {
    crate::lang::LANGS.iter().find(|spec| spec.name == name)
}

/// Colours a line of EDI or CSV, which have no lexer to colour them.
///
/// Both are delimiter languages, and the only distinction worth drawing is the
/// one a reader makes: quoted text is a string, digits are numbers, the
/// delimiters are operators and the rest is plain.
fn split_delimited(flavour: Flavour, line: &str) -> Vec<Style> {
    let chars: Vec<char> = line.chars().collect();
    let mut styles = vec![style::DEFAULT; chars.len()];
    let mut quoted = false;
    for (index, ch) in chars.iter().copied().enumerate() {
        if flavour == Flavour::Csv && ch == '"' {
            styles[index] = style::STRING;
            quoted = !quoted;
        } else if quoted {
            styles[index] = style::STRING;
        } else if ch.is_ascii_digit() {
            styles[index] = style::NUMBER;
        } else if matches!(ch, ',' | '+' | ':' | '\'' | '.') {
            styles[index] = style::OPERATOR;
        } else if line.len() - line.trim_start().len() > index {
            // Leading whitespace stays plain, as it does in the lexers.
            styles[index] = style::DEFAULT;
        }
    }
    if flavour == Flavour::Edifact && styles.len() > 3 {
        // A segment tag is the three letters before the first `+`: `UNB`, `NAD`.
        for index in 0..3 {
            if chars[index].is_ascii_uppercase() {
                styles[index] = style::KEYWORD;
            }
        }
    }
    styles
}

/// The tokens a line is made of: a card the kitten can bat, once it is loose.
///
/// A token is a run of characters between two delimiters, with the delimiters
/// kept as tokens of their own, which is what makes `"Sir Pounce"` one card and
/// `{` another.
pub fn tokens(flavour: Flavour, line: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut word = String::new();
    let delimiters: &[char] = match flavour {
        Flavour::Edifact => &['+', ':', '\'', '\'', ' '],
        Flavour::Csv => &[',', ' '],
        Flavour::Json => &['{', '}', '[', ']', ':', ',', ' '],
        Flavour::Xml => &['<', '>', '/', '=', ' ', '"'],
        _ => &[
            '(', ')', '{', '}', ';', ',', '.', '=', '+', '-', '*', '/', '<', '>', ' ', '"', '!',
            '[', ']', '&', '|', '\'', ':',
        ],
    };
    for ch in line.chars() {
        if delimiters.contains(&ch) {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            if !ch.is_whitespace() {
                out.push(ch.to_string());
            }
        } else {
            word.push(ch);
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_flavour_writes_something() {
        for flavour in Flavour::ALL {
            assert!(!flavour.lines().is_empty(), "{}", flavour.name());
            assert!(!flavour.name().is_empty());
        }
    }

    #[test]
    fn a_code_flavour_names_a_language_the_editor_has() {
        for flavour in Flavour::ALL {
            let Some(name) = flavour.language() else {
                continue;
            };
            assert!(spec(name).is_some(), "{} names {name}", flavour.name());
        }
        assert!(Flavour::Edifact.language().is_none());
        assert!(Flavour::Csv.language().is_none());
    }

    #[test]
    fn a_line_is_coloured_one_style_per_character() {
        for flavour in Flavour::ALL {
            for line in flavour.lines() {
                let styles = flavour.styles(line);
                assert_eq!(
                    styles.len(),
                    line.chars().count(),
                    "{}: {line}",
                    flavour.name()
                );
            }
        }
    }

    #[test]
    fn the_lexer_finds_a_keyword_and_a_string_in_the_rust_lines() {
        let line = "    let mut k = Kitten::new();";
        let styles = Flavour::Rust.styles(line);
        let chars: Vec<char> = line.chars().collect();
        // `let` is a keyword, and `mut` with it.
        let keyword = chars
            .iter()
            .zip(&styles)
            .position(|(ch, style)| *ch == 'l' && *style == style::KEYWORD);
        assert!(keyword.is_some(), "{styles:?}");

        let quoted = "\"purr\"";
        let styles = Flavour::Rust.styles(quoted);
        assert!(styles.iter().all(|style| *style == style::STRING), "{styles:?}");
    }

    #[test]
    fn edifact_tags_are_the_three_letters_before_the_first_plus() {
        let styles = Flavour::Edifact.styles("NAD+BY+SUPPLIER+++ACME LTD'");
        assert_eq!(&styles[..3], &[style::KEYWORD; 3]);
        // The `+` after NAD is an operator, and the tag's letters are not.
        assert_eq!(styles[3], style::OPERATOR);
    }

    #[test]
    fn csv_quotes_are_strings_and_numbers_are_numbers() {
        let line = "3,\"Sir Pounce\",8,2";
        let styles = Flavour::Csv.styles(line);
        assert_eq!(styles[0], style::NUMBER);
        assert_eq!(styles[1], style::OPERATOR);
        assert_eq!(styles[2], style::STRING);
        // The last two fields are numbers, not strings.
        let last = styles.len() - 1;
        assert_eq!(styles[last], style::NUMBER);
    }

    #[test]
    fn whitespace_and_punctuation_are_not_tokens() {
        let found = tokens(Flavour::Rust, "    let mut k = Kitten::new();");
        assert_eq!(
            found,
            vec!["let", "mut", "k", "=", "Kitten", ":", ":", "new", "(", ")", ";"]
        );
        assert!(tokens(Flavour::Csv, "a, b").iter().all(|t| t != " "));
    }

    #[test]
    fn tokens_cover_the_line_they_came_from() {
        // Every non-space character of a line is in some token, so a wall of
        // cards really is the code and not a paraphrase of it.
        for flavour in Flavour::ALL {
            for line in flavour.lines() {
                let joined: String = tokens(flavour, line).concat();
                let expected: String = line.chars().filter(|ch| !ch.is_whitespace()).collect();
                assert_eq!(joined, expected, "{}: {line}", flavour.name());
            }
        }
    }

    #[test]
    fn every_flavour_yields_enough_tokens_to_furnish_a_room() {
        for flavour in Flavour::ALL {
            let count: usize = flavour
                .lines()
                .iter()
                .map(|line| tokens(flavour, line).len())
                .sum();
            assert!(count >= 12, "{}: {count} tokens", flavour.name());
        }
    }

    #[test]
    fn no_line_is_wider_than_a_wall_card_grid_of_seventy_eight() {
        for flavour in Flavour::ALL {
            for line in flavour.lines() {
                assert!(line.chars().count() <= 78, "{}: {line}", flavour.name());
            }
        }
    }
}
