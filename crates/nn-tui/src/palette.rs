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
//! nobody can see.
//!
//! The escapes are built once per theme, in [`Palette`], and a tone's escape is
//! then a slice of that: the renderer writes a run of cells in one tone, so the
//! hot path must not be formatting a colour per cell, and a palette is eighteen
//! strings that change when the user picks Tokyo Night and never otherwise.
//!
//! The vocabulary is deliberately small. A theme with a colour of its own for
//! every token class is a later problem; what matters to a first shell is that a
//! keyword differs from a string and that the furniture differs from the text.

use nn_core::lexer::style;
use nn_shared::color::Rgb;
use nn_shared::theme::Theme;

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

    /// The strip of menu titles along the top, and the rows of a dropdown.
    ///
    /// The same two colours for both, taken from the windowed theme's own menu
    /// colours, because a menu is a menu in either front-end.
    Menu,
    /// A dropdown row under the pointer, or the title of the open menu.
    MenuHover,
    /// The box drawn round a dropdown.
    MenuBorder,
}

impl Tone {
    /// Where this tone's escape lives in a [`Palette`].
    fn at(self) -> usize {
        self as usize
    }

    /// Every tone, in the order the escapes are built.
    #[cfg(test)]
    const ALL: [Tone; 18] = [
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
        Tone::Menu,
        Tone::MenuHover,
        Tone::MenuBorder,
    ];
}

/// Every tone's escape for one theme, built in one go.
///
/// Built rather than formatted per cell because the renderer writes a tone's
/// escape once per run: eighteen strings per theme change, and none per frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    escapes: Vec<String>,
}

impl Palette {
    /// The escapes a theme's colours make.
    pub fn of(theme: &Theme) -> Self {
        let syntax = |index: u8| -> (Rgb, Rgb) {
            let colour = theme
                .syntax
                .get(usize::from(index))
                .copied()
                .unwrap_or(theme.foreground);
            (colour, theme.background)
        };
        let pairs = [
            (theme.foreground, theme.background),
            syntax(style::KEYWORD),
            syntax(style::TYPE),
            syntax(style::STRING),
            syntax(style::COMMENT),
            syntax(style::NUMBER),
            syntax(style::FUNCTION),
            syntax(style::OPERATOR),
            syntax(style::PREPROCESSOR),
            (theme.gutter_foreground, theme.gutter_background),
            (theme.gutter_active, theme.gutter_background),
            (theme.foreground, theme.selection),
            // The caret is drawn as the background showing through it, which is
            // the one place the two colours swap.
            (theme.background, theme.caret),
            (theme.bar_label, theme.bar_background),
            // No theme colour means "this is bad news"; the orange a Notepad++
            // tab draws its rule in is the one warm colour every palette has.
            (theme.tab_active_rule, theme.bar_background),
            (theme.menu_text, theme.menu_background),
            (theme.menu_text, theme.menu_hover),
            (theme.menu_border, theme.menu_background),
        ];
        Palette {
            escapes: pairs
                .into_iter()
                .map(|(foreground, background)| {
                    format!(
                        "\x1b[38;2;{};{};{};48;2;{};{};{}m",
                        foreground.0, foreground.1, foreground.2,
                        background.0, background.1, background.2
                    )
                })
                .collect(),
        }
    }

    /// The escape that paints `tone`, foreground and background both.
    ///
    /// Every tone sets both, because a run of text written after a coloured run
    /// inherits whatever the previous escape left behind: half a pair is how a
    /// terminal ends up with black on black.
    pub fn sgr(&self, tone: Tone) -> &str {
        self.escapes.get(tone.at()).map_or("", |one| one.as_str())
    }

    /// How many tones it paints, which is what the tests count.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.escapes.len()
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
    fn every_tone_gets_an_escape_that_sets_both_colours() {
        let palette = Palette::of(&Theme::DARK);
        assert_eq!(palette.len(), 18);
        for one in Tone::ALL {
            let sgr = palette.sgr(one);
            assert!(sgr.starts_with("\x1b["), "{one:?} is not an escape");
            assert!(sgr.ends_with('m'), "{one:?} does not end in `m`");
            assert!(sgr.contains("38;2;"), "{one:?} sets no foreground");
            assert!(sgr.contains("48;2;"), "{one:?} sets no background");
        }
    }

    #[test]
    fn the_palette_is_the_themes_colours_not_a_copy_of_one() {
        let dark = Palette::of(&Theme::DARK);
        let light = Palette::of(&Theme::LIGHT);
        assert_ne!(dark.sgr(Tone::Text), light.sgr(Tone::Text));
        // A light theme puts dark ink on a light page, so the plain text escape
        // and the dark one cannot be the same string.
        assert_ne!(dark.sgr(Tone::Caret), light.sgr(Tone::Caret));
        // The same theme twice is the same escapes: the renderer compares them
        // to decide whether the screen it wrote is still current.
        assert_eq!(Palette::of(&Theme::LIGHT), light);
    }
}
