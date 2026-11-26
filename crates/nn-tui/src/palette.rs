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

//! The colours, as the eight syntax styles and the eight kinds of furniture.
//!
//! A cell carries one [`Tone`] rather than a foreground and a background
//! separately: the grid compares cells to decide what to write, and two numbers
//! per cell would be two chances to write a frame that differs only in a colour
//! nobody can see. The tones are the default theme's colours from
//! `crates/nn-app/src/theme.rs`, so the two front-ends look the same, written as
//! 24-bit escapes because that is what a terminal made this century reads.
//!
//! The vocabulary is deliberately small. A theme with a colour of its own for
//! every token class is a later problem; what matters to a first shell is that a
//! keyword differs from a string and that the furniture differs from the text.

use nn_core::lexer::style;

/// What a cell is drawn as: one foreground, one background, fixed for the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// The document's plain text.
    Text,
    Keyword,
    Type,
    String,
    Comment,
    Number,
    Function,
    Operator,
    Preprocessor,
    /// A line number in the gutter.
    Gutter,
    /// The line number of the caret's line.
    GutterActive,
    /// Selected text.
    Selection,
    /// The character under the caret.
    Caret,
    /// The bar at the bottom.
    StatusBar,
    /// A message on the bar: saved, unsaved, refused.
    Alert,
}

impl Tone {
    /// The escape that paints this tone, foreground and background both.
    ///
    /// Every tone sets both, because a run of text written after a coloured run
    /// inherits whatever the previous escape left behind: half a pair is how a
    /// terminal ends up with black on black.
    pub fn sgr(self) -> &'static str {
        match self {
            Tone::Text => "\x1b[38;2;212;212;212;48;2;30;30;30m",
            Tone::Keyword => "\x1b[38;2;86;156;214;48;2;30;30;30m",
            Tone::Type => "\x1b[38;2;78;201;176;48;2;30;30;30m",
            Tone::String => "\x1b[38;2;206;145;120;48;2;30;30;30m",
            Tone::Comment => "\x1b[38;2;106;153;85;48;2;30;30;30m",
            Tone::Number => "\x1b[38;2;181;206;168;48;2;30;30;30m",
            Tone::Function => "\x1b[38;2;220;220;170;48;2;30;30;30m",
            Tone::Operator => "\x1b[38;2;212;212;212;48;2;30;30;30m",
            Tone::Preprocessor => "\x1b[38;2;197;134;192;48;2;30;30;30m",
            Tone::Gutter => "\x1b[38;2;133;133;133;48;2;37;37;38m",
            Tone::GutterActive => "\x1b[38;2;198;198;198;48;2;37;37;38m",
            Tone::Selection => "\x1b[38;2;230;230;230;48;2;38;79;120m",
            Tone::Caret => "\x1b[38;2;30;30;30;48;2;174;175;173m",
            Tone::StatusBar => "\x1b[38;2;150;150;150;48;2;37;37;38m",
            Tone::Alert => "\x1b[38;2;232;176;84;48;2;37;37;38m",
        }
    }
}

/// The tone a lexer's style is drawn in.
///
/// The user-defined styles (16 and above) are drawn as plain text for now: they
/// are a colour each in the windowed front-end and a later pass here.
pub fn tone(style: u8) -> Tone {
    match style {
        style::KEYWORD => Tone::Keyword,
        style::TYPE => Tone::Type,
        style::STRING => Tone::String,
        style::COMMENT => Tone::Comment,
        style::NUMBER => Tone::Number,
        style::FUNCTION => Tone::Function,
        style::OPERATOR => Tone::Operator,
        style::PREPROCESSOR => Tone::Preprocessor,
        _ => Tone::Text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_syntax_style_has_a_tone_that_is_not_the_plain_one() {
        let styles = [
            style::KEYWORD,
            style::TYPE,
            style::STRING,
            style::COMMENT,
            style::NUMBER,
            style::FUNCTION,
            style::OPERATOR,
            style::PREPROCESSOR,
        ];
        let mut seen: Vec<Tone> = Vec::new();
        for one in styles {
            let drawn = tone(one);
            assert_ne!(drawn, Tone::Text, "style {one} is drawn as text");
            assert!(!seen.contains(&drawn), "style {one} repeats a tone");
            seen.push(drawn);
        }
        assert_eq!(tone(style::DEFAULT), Tone::Text);
        // A user-defined style, which this pass draws as text on purpose.
        assert_eq!(tone(style::UDL_KEYWORD1), Tone::Text);
    }

    #[test]
    fn every_tone_sets_a_foreground_and_a_background_and_ends_cleanly() {
        let all = [
            Tone::Text,
            Tone::Keyword,
            Tone::Type,
            Tone::String,
            Tone::Comment,
            Tone::Number,
            Tone::Function,
            Tone::Operator,
            Tone::Preprocessor,
            Tone::Gutter,
            Tone::GutterActive,
            Tone::Selection,
            Tone::Caret,
            Tone::StatusBar,
            Tone::Alert,
        ];
        for one in all {
            let sgr = one.sgr();
            assert!(sgr.starts_with("\x1b["), "{one:?} is not an escape");
            assert!(sgr.ends_with('m'), "{one:?} does not end in `m`");
            assert!(sgr.contains("38;2;"), "{one:?} sets no foreground");
            assert!(sgr.contains("48;2;"), "{one:?} sets no background");
        }
        // `Operator` is drawn in the plain colour on purpose -- the theme has no
        // colour of its own for punctuation -- so the escapes are not all
        // distinct, and a test that demanded they were would be asking for a
        // palette nobody chose.
    }
}
