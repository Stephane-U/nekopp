//! The colours a document's text is drawn in.
//!
//! A [`Theme`] colours the chrome and the nine built-in syntax styles. A
//! user-defined language needs more than nine: it can declare eight keyword
//! groups and eight delimiter pairs, and Notepad++ colours each separately. It
//! also brings its own colours, chosen by whoever wrote the language.
//!
//! So the palette is the theme plus a table of one colour per style index,
//! built once per document rather than once per frame. The renderer asks the
//! palette, not the theme, and a document with no user-defined language gets a
//! palette that is exactly the theme's nine colours.
//!
//! The one judgement call is [`MIN_CONTRAST`]. A language file is written
//! against *some* theme -- usually Notepad++'s light default -- and applying its
//! colours verbatim in a dark theme produces black text on a near-black
//! background. Refusing them all would make the import pointless. So a colour
//! within [`MIN_CONTRAST`] of the background is replaced by the theme's colour
//! for that role, and everything else is honoured.

use nn_core::style;
use nn_core::{Rgb8, Style, Udl};

use crate::theme::{Rgb, Theme};

/// How far a colour's brightness must be from the background's before it is
/// used as written.
///
/// Forty of 255 is about a sixth of the range: enough that a mid-grey on a
/// near-black background is rejected, small enough that a deliberately dim
/// comment colour survives.
pub const MIN_CONTRAST: i32 = 40;

/// One colour per style index, plus the theme the chrome is drawn with.
#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    /// The theme in use. The chrome reads its colours from here.
    pub theme: Theme,
    syntax: [Rgb; style::COUNT],
}

impl Palette {
    /// The palette for a document with no user-defined language.
    pub fn new(theme: Theme) -> Palette {
        let mut syntax = [theme.foreground; style::COUNT];
        syntax[..theme.syntax.len()].copy_from_slice(&theme.syntax);
        // A user-defined language that names no colours still has to be
        // readable, so its slots start at the theme's own keyword and string
        // colours rather than at the foreground.
        for group in 0..8 {
            syntax[style::UDL_KEYWORD1 as usize + group] = theme.syntax[style::KEYWORD as usize];
            syntax[style::UDL_DELIMITER1 as usize + group] = theme.syntax[style::STRING as usize];
        }
        Palette { theme, syntax }
    }

    /// The palette for a document highlighted by `udl`.
    ///
    /// A colour the language names is used unless it is too close to the
    /// background to read, in which case the theme's colour for that role
    /// stands.
    pub fn with_udl(theme: Theme, udl: &Udl) -> Palette {
        let mut palette = Palette::new(theme);
        let background = theme.background.luminance() as i32;
        for (slot, colour) in udl.colours.iter().enumerate() {
            let Some(colour) = colour else {
                continue;
            };
            let Some(index) = style_of_slot(slot) else {
                continue;
            };
            if (colour.luminance() as i32 - background).abs() < MIN_CONTRAST {
                continue;
            }
            palette.syntax[index as usize] = Rgb::new(colour.r, colour.g, colour.b);
        }
        palette
    }

    /// The colour for a style index.
    ///
    /// An index past the table falls back to the default text colour, so a
    /// lexer that invents one cannot make text invisible.
    pub fn color(&self, style: Style) -> Rgb {
        self.syntax
            .get(style as usize)
            .copied()
            .unwrap_or(self.theme.foreground)
    }
}

/// The style index a user-defined language's colour slot paints.
///
/// The two comment slots share one style because the editor has one comment
/// colour: a language that colours its line and block comments differently is
/// asking for a distinction the theme does not make.
fn style_of_slot(slot: usize) -> Option<Style> {
    use nn_core::udl::slot;
    let style = match slot {
        slot::DEFAULT => style::DEFAULT,
        slot::COMMENT | slot::LINE_COMMENT => style::COMMENT,
        slot::NUMBER => style::NUMBER,
        slot::OPERATOR => style::OPERATOR,
        other if (slot::KEYWORD1..slot::KEYWORD1 + 8).contains(&other) => {
            style::UDL_KEYWORD1 + (other - slot::KEYWORD1) as Style
        }
        other if (slot::DELIMITER1..slot::DELIMITER1 + 8).contains(&other) => {
            style::UDL_DELIMITER1 + (other - slot::DELIMITER1) as Style
        }
        _ => return None,
    };
    Some(style)
}

/// Converts a colour read from a language file.
pub fn from_rgb8(colour: Rgb8) -> Rgb {
    Rgb::new(colour.r, colour.g, colour.b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn udl_with(colours: &[(usize, Rgb8)]) -> Udl {
        let mut udl = Udl::default();
        for (slot, colour) in colours {
            udl.colours[*slot] = Some(*colour);
        }
        udl
    }

    #[test]
    fn a_plain_palette_is_the_theme() {
        let palette = Palette::new(Theme::DARK);
        for index in 0..9 {
            assert_eq!(palette.color(index), Theme::DARK.syntax[index as usize]);
        }
    }

    #[test]
    fn a_plain_palette_colours_the_udl_slots_from_the_theme() {
        let palette = Palette::new(Theme::DARK);
        assert_eq!(
            palette.color(style::UDL_KEYWORD1),
            Theme::DARK.syntax[style::KEYWORD as usize]
        );
        assert_eq!(
            palette.color(style::UDL_DELIMITER8),
            Theme::DARK.syntax[style::STRING as usize]
        );
    }

    #[test]
    fn a_style_past_the_table_is_the_foreground() {
        let palette = Palette::new(Theme::DARK);
        assert_eq!(palette.color(200), Theme::DARK.foreground);
    }

    #[test]
    fn a_legible_language_colour_is_used_as_written() {
        // A bright green on the dark theme's near-black background.
        let udl = udl_with(&[(nn_core::udl::slot::KEYWORD1, Rgb8::new(0x4e, 0xc9, 0x4e))]);
        let palette = Palette::with_udl(Theme::DARK, &udl);
        assert_eq!(palette.color(style::UDL_KEYWORD1), Rgb::new(0x4e, 0xc9, 0x4e));
    }

    #[test]
    fn a_colour_too_close_to_the_background_is_refused() {
        // Notepad++'s default black text on this editor's dark background.
        let udl = udl_with(&[(nn_core::udl::slot::KEYWORD1, Rgb8::new(0, 0, 0))]);
        let palette = Palette::with_udl(Theme::DARK, &udl);
        assert_eq!(
            palette.color(style::UDL_KEYWORD1),
            Theme::DARK.syntax[style::KEYWORD as usize]
        );
    }

    #[test]
    fn the_same_colour_is_accepted_on_the_light_theme() {
        let udl = udl_with(&[(nn_core::udl::slot::KEYWORD1, Rgb8::new(0, 0, 0))]);
        let palette = Palette::with_udl(Theme::LIGHT, &udl);
        assert_eq!(palette.color(style::UDL_KEYWORD1), Rgb::new(0, 0, 0));
    }

    #[test]
    fn every_slot_paints_the_style_it_names() {
        use nn_core::udl::slot;
        assert_eq!(style_of_slot(slot::DEFAULT), Some(style::DEFAULT));
        assert_eq!(style_of_slot(slot::COMMENT), Some(style::COMMENT));
        assert_eq!(style_of_slot(slot::LINE_COMMENT), Some(style::COMMENT));
        assert_eq!(style_of_slot(slot::NUMBER), Some(style::NUMBER));
        assert_eq!(style_of_slot(slot::OPERATOR), Some(style::OPERATOR));
        for group in 0..8 {
            assert_eq!(
                style_of_slot(slot::KEYWORD1 + group),
                Some(style::UDL_KEYWORD1 + group as Style)
            );
            assert_eq!(
                style_of_slot(slot::DELIMITER1 + group),
                Some(style::UDL_DELIMITER1 + group as Style)
            );
        }
        assert_eq!(style_of_slot(slot::COUNT), None);
    }

    #[test]
    fn the_eight_keyword_groups_are_eight_styles() {
        let mut udl = Udl::default();
        for group in 0..8 {
            let shade = 0x40 + group as u8 * 0x10;
            udl.colours[nn_core::udl::slot::KEYWORD1 + group] =
                Some(Rgb8::new(shade, shade, shade));
        }
        let palette = Palette::with_udl(Theme::DARK, &udl);
        let mut seen = Vec::new();
        for group in 0..8 {
            let colour = palette.color(style::UDL_KEYWORD1 + group as Style);
            assert!(!seen.contains(&colour), "group {group} repeats a colour");
            seen.push(colour);
        }
    }

    #[test]
    fn a_language_with_no_colours_leaves_the_theme_alone() {
        let palette = Palette::with_udl(Theme::DARK, &Udl::default());
        assert_eq!(palette, Palette::new(Theme::DARK));
    }
}
