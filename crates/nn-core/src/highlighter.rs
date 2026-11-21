//! Drives a [`Lexer`] over a [`Document`].
//!
//! The highlighter owns the policy the lexer deliberately does not: which lines
//! to re-lex after an edit, and when it is safe to stop.
//!
//! The stop rule is the whole point. Lexing line `n` produces the state that
//! line `n + 1` starts in. If that state is the one line `n + 1` already had,
//! then every line below it would be lexed identically, so the walk stops. In
//! ordinary code the state is `Normal` almost everywhere, so an edit re-lexes
//! one or two lines instead of the file. Only an edit that opens or closes a
//! block comment or a multi-line string walks further.

use crate::document::Document;
use crate::lexer::{LexState, Lexer};

/// Applies a lexer to a document.
pub struct Highlighter {
    lexer: Box<dyn Lexer>,
}

impl Highlighter {
    /// Wraps `lexer`.
    pub fn new(lexer: Box<dyn Lexer>) -> Self {
        Self { lexer }
    }

    /// Replaces the lexer, e.g. when the language is switched.
    pub fn set_lexer(&mut self, lexer: Box<dyn Lexer>) {
        self.lexer = lexer;
    }

    /// Lexes the whole document.
    ///
    /// Used on load and after a language change, where no previous state can be
    /// trusted.
    pub fn highlight_all(&mut self, doc: &mut Document) {
        let total = doc.len_lines();
        let mut state = LexState::Normal;
        for line in 0..total {
            doc.set_lex_state(line, state.to_u8());
            let text = doc.line(line);
            let (styles, next) = self.lexer.lex_line(&text, state);
            doc.set_line_styles(line, &styles);
            state = next;
        }
    }

    /// Re-lexes from `from` down, stopping as soon as the state converges.
    ///
    /// `from` is clamped to the document, so callers can pass a caret line
    /// without checking bounds.
    pub fn highlight_from(&mut self, doc: &mut Document, from: usize) {
        let total = doc.len_lines();
        if total == 0 {
            return;
        }
        let mut line = from.min(total - 1);
        let mut state = LexState::from_u8(doc.lex_state(line));

        while line < total {
            let text = doc.line(line);
            let (styles, next) = self.lexer.lex_line(&text, state);
            doc.set_line_styles(line, &styles);
            state = next;
            line += 1;

            if line >= total {
                break;
            }
            // The next line already starts in this state, so nothing below it
            // can change.
            if doc.lex_state(line) == state.to_u8() {
                break;
            }
            doc.set_lex_state(line, state.to_u8());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::{style, CLikeLexer};

    fn doc(text: &str) -> Document {
        Document::from_text(text)
    }

    fn highlighter() -> Highlighter {
        Highlighter::new(Box::new(CLikeLexer))
    }

    fn styles_of(doc: &Document, line: usize) -> Vec<u8> {
        doc.line_styles(line).to_vec()
    }

    #[test]
    fn highlights_every_line() {
        let mut d = doc("fn main() {\n    let x = 1;\n}\n");
        highlighter().highlight_all(&mut d);
        assert_eq!(styles_of(&d, 0)[0], style::KEYWORD);
        assert_eq!(styles_of(&d, 1)[4], style::KEYWORD);
    }

    #[test]
    fn records_state_at_start_of_each_line() {
        let mut d = doc("/* a\nb\nc */\nlet x = 1;\n");
        highlighter().highlight_all(&mut d);
        assert_eq!(d.lex_state(0), LexState::Normal.to_u8());
        assert_eq!(d.lex_state(1), LexState::BlockComment.to_u8());
        assert_eq!(d.lex_state(2), LexState::BlockComment.to_u8());
        assert_eq!(d.lex_state(3), LexState::Normal.to_u8());
    }

    #[test]
    fn edit_inside_code_stops_immediately() {
        let mut d = doc("let a = 1;\nlet b = 2;\nlet c = 3;\n");
        let mut h = highlighter();
        h.highlight_all(&mut d);
        d.insert(10, "x");
        h.highlight_from(&mut d, 0);
        assert_eq!(styles_of(&d, 0)[0], style::KEYWORD);
        assert_eq!(styles_of(&d, 2)[0], style::KEYWORD);
    }

    #[test]
    fn opening_a_block_comment_recolours_the_rest() {
        let mut d = doc("let a = 1;\nlet b = 2;\nlet c = 3;\n");
        let mut h = highlighter();
        h.highlight_all(&mut d);
        // Turn line 0 into the start of a block comment.
        d.insert(0, "/*");
        h.highlight_from(&mut d, 0);
        assert_eq!(styles_of(&d, 1)[0], style::COMMENT);
        assert_eq!(styles_of(&d, 2)[0], style::COMMENT);
    }

    #[test]
    fn closing_a_block_comment_restores_code() {
        let mut d = doc("/*\nlet a = 1;\n*/\nlet b = 2;\n");
        let mut h = highlighter();
        h.highlight_all(&mut d);
        assert_eq!(styles_of(&d, 1)[0], style::COMMENT);
        // Remove the closing marker.
        let close = d.text().find("*/").unwrap();
        d.remove(close..close + 2);
        h.highlight_from(&mut d, 2);
        assert_eq!(styles_of(&d, 3)[0], style::COMMENT);
    }

    #[test]
    fn highlight_from_clamps_past_the_end() {
        let mut d = doc("let a = 1;\n");
        let mut h = highlighter();
        h.highlight_all(&mut d);
        h.highlight_from(&mut d, 999);
        assert_eq!(styles_of(&d, 0)[0], style::KEYWORD);
    }

    #[test]
    fn empty_document_is_safe() {
        let mut d = doc("");
        let mut h = highlighter();
        h.highlight_all(&mut d);
        h.highlight_from(&mut d, 0);
    }

    #[test]
    fn state_survives_line_insertion() {
        let mut d = doc("/*\n*/\nlet a = 1;\n");
        let mut h = highlighter();
        h.highlight_all(&mut d);
        // Insert a line inside the comment.
        d.insert(3, "still comment\n");
        h.highlight_from(&mut d, 1);
        assert_eq!(styles_of(&d, 1)[0], style::COMMENT);
        assert_eq!(styles_of(&d, 3)[0], style::KEYWORD);
    }
}
