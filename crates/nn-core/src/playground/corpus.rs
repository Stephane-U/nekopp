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
    ///
    /// Twenty-odd lines each, and most of them long: a page holds a hundred and
    /// twenty columns and a file of five-word lines would leave two thirds of the
    /// mode empty. Blank lines are part of the material, because a file without
    /// them is a file nobody wrote.
    pub fn lines(self) -> &'static [&'static str] {
        match self {
            Flavour::Rust => &[
                "// playground: a page of code, a kitten, and five minutes of nothing to do",
                "use std::time::{Duration, Instant};   // the nap timer, not the cat's own clock",
                "use crate::lines::Rng;                // seeded, so a run can be repeated",
                "",
                "pub fn main() {",
                "    let mut k = Kitten::new(40.0, 20.0);   // halfway down the page, mid-line",
                "    k.chase(&mouse);",
                "    println!(\"purr\");",
                "}",
                "",
                "impl Kitten {",
                "    pub fn new(x: f32, y: f32) -> Self {",
                "        Self { pos: [x, y, 0.0], naps: 9, mood: Mood::Curious, speed: 0.0 }",
                "    }",
                "",
                "    fn chase(&mut self, at: &Point) {",
                "        self.target = *at;                 // a place on the page, in cells",
                "        self.speed = WALK;",
                "        self.purr();",
                "    }",
                "",
                "    fn purr(&self) -> bool {",
                "        self.mood == Mood::Happy && self.naps > 0   // and the mouse has stopped",
                "    }",
                "}",
                "",
                "#[derive(Debug, Clone, Copy)]",
                "enum Mood { Curious, Happy, Asleep }",
            ],
            Flavour::C => &[
                "#include <stdio.h>",
                "#include <stdlib.h>",
                "",
                "/* a room made out of text: the walls are code and the floor is punctuation */",
                "struct room {",
                "    int chairs;",
                "    int lamps;",
                "    char name[16];",
                "};",
                "",
                "static int laps(int n) { return n * 2 + 1; }   /* two laps, and one back */",
                "",
                "int main(void) {",
                "    int laps = 9;",
                "    struct room study = { 2, 1, \"study\" };",
                "    printf(\"%d laps, %d chairs\\n\", laps, study.chairs);",
                "    if (study.chairs > 0 && study.lamps > 0) { puts(study.name); }",
                "    return 0;                                  /* and the cat sleeps on */",
                "}",
            ],
            Flavour::Java => &[
                "package playground;",
                "",
                "import java.util.List;",
                "import java.util.ArrayList;",
                "",
                "public class Nap {",
                "    private int hours = 8;",
                "    private final String name;",
                "    private final List<String> dreams = new ArrayList<>();",
                "",
                "    public Nap(String name) { this.name = name; }",
                "",
                "    public void sleep() {",
                "        for (int i = 0; i < hours; i++) { System.out.println(hours); }",
                "    }",
                "",
                "    public String getName() { return this.name; }",
                "",
                "    public List<String> getDreams() { return this.dreams; }",
                "",
                "    public static void main(String[] args) { new Nap(\"Mochi\").sleep(); }",
                "}",
            ],
            Flavour::Python => &[
                "import time",
                "",
                "",
                "class Kitten:",
                "    \"\"\"A small animal, on a page of code, with nothing to do all afternoon.\"\"\"",
                "",
                "    def __init__(self, name=\"Mochi\", naps=9):",
                "        self.name = name",
                "        self.naps = naps",
                "        self.mood = \"curious\"",
                "",
                "    def purr(self, times=3):",
                "        for n in range(times):",
                "            print(\"purr\", n)        # three is enough to be going on with",
                "        return True",
                "",
                "    def sleep(self, seconds=30):",
                "        time.sleep(seconds)",
                "        self.naps += 1",
                "        self.mood = \"asleep\"",
                "",
                "",
                "if __name__ == \"__main__\":",
                "    Kitten().purr()",
            ],
            Flavour::Json => &[
                "{",
                "  \"name\": \"Mochi\",",
                "  \"lives\": 9,",
                "  \"naps\": [true, false, null],",
                "  \"room\": {",
                "    \"walls\": \"code\",",
                "    \"floor\": \"punctuation\",",
                "    \"cats\": 1,",
                "    \"window\": { \"open\": true, \"sill\": \"books\" }",
                "  },",
                "  \"shelf\": [",
                "    { \"title\": \"Nap\", \"pages\": 12 },",
                "    { \"title\": \"Purr\", \"pages\": 9 },",
                "    { \"title\": \"A Room of One's Own\", \"pages\": 172 }",
                "  ],",
                "  \"awake\": false,",
                "  \"weight\": 3.4,",
                "  \"notes\": \"a kitten was here, and left a pawprint on the page\"",
                "}",
            ],
            Flavour::Xml => &[
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
                "<room name=\"study\" walls=\"code\" floor=\"punctuation\">",
                "  <!-- a kitten was here, and the page says so -->",
                "  <shelf units=\"3\">",
                "    <book title=\"Nap\" pages=\"12\"/>",
                "    <book title=\"Purr\" pages=\"9\"/>",
                "    <book title=\"A Room of One's Own\" pages=\"172\"/>",
                "  </shelf>",
                "  <lamp on=\"true\" watt=\"40\"/>",
                "  <desk chairs=\"2\">",
                "    <drawer closed=\"false\"/>",
                "    <drawer closed=\"true\"/>",
                "  </desk>",
                "  <cat name=\"Mochi\" asleep=\"false\" naps=\"9\"/>",
                "</room>",
            ],
            Flavour::Edifact => &[
                "UNA:+.? '",
                "UNB+UNOC:3+SUPPLIER:14+BUYER:14+240101:1200+1'",
                "UNH+1+ORDERS:D:96A:UN'",
                "BGM+220+PO-4471+9'",
                "DTM+137:20240101:102'",
                "NAD+BY+SUPPLIER+++ACME LTD'",
                "NAD+SU+BUYER+++MOCHI AND CO'",
                "LIN+1++WHISKAS:SA'",
                "QTY+21:9'",
                "PRI+AAA:12.40'",
                "LIN+2++CATNIP:SA'",
                "QTY+21:3'",
                "PRI+AAA:4.15'",
                "UNT+13+1'",
                "UNZ+1+1'",
            ],
            Flavour::Csv => &[
                "id,name,lives,naps,notes",
                "1,Mochi,9,3,\"a kitten was here\"",
                "2,Biscuit,7,5,\"found asleep on the shelf\"",
                "3,\"Sir Pounce\",8,2,\"pounces first, asks later\"",
                "4,Pepper,9,1,\"sits on the keyboard\"",
                "5,\"Marmalade, Jr\",7,4,\"learns the window sill\"",
                "6,Whiskers,9,2,\"counts the birds in the margin\"",
                "7,Sox,8,3,\"black and white, and mostly white\"",
                "8,\"Mochi II\",9,0,\"still awake, somehow\"",
                "9,Ash,7,6,\"sleeps through everything\"",
                "10,Bramble,8,4,\"into the brambles, and back\"",
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
    fn no_line_is_wider_than_a_page_column() {
        // A hundred and twenty columns at most, so a long line of code fits on one
        // row of the widest page the mode lays out instead of wrapping mid-token.
        for flavour in Flavour::ALL {
            for line in flavour.lines() {
                assert!(line.chars().count() <= 120, "{}: {line}", flavour.name());
            }
        }
    }
}
