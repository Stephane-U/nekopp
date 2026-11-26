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

//! The theme catalogue.
//!
//! A [`crate::theme::Theme`] carries sixty-odd colours because every widget
//! needs one. Nobody picks sixty colours; a theme is designed as ten. So a
//! catalogue entry is a [`Palette`] -- the background, the text, the gutter, the
//! selection, the caret, one accent and the nine syntax styles -- and
//! [`crate::theme::Theme::from_palette`] derives the chrome from it.
//!
//! The values are the published ones, not approximations of them. Solarized's
//! base03 is `#002B36` and Tokyo Night's background is `#1A1B26`; a theme named
//! after a palette that does not match it is worse than no theme at all, because
//! the only reason to pick one by name is that you already know what it looks
//! like.
//!
//! Adding a theme is: one `Palette` constant, one entry in [`CATALOGUE`], one
//! variant in `ThemeChoice`. The last of those is what puts it in Preferences.

use crate::theme::Rgb;
use nn_shared::theme::ThemeChoice;

/// One theme, as the colours a designer actually chooses.
///
/// Not to be confused with [`crate::palette::Palette`], which is the set of
/// colours one *document* is drawn with: that one is derived per document from
/// the theme plus the user-defined language, and changes when the file does.
/// This one is the source a theme is built from, and changes only when someone
/// edits this file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// The name the Preferences row and the Settings menu show.
    pub name: &'static str,
    /// The keyword the settings file stores.
    ///
    /// Separate from [`Palette::name`] because a name is displayed and a key is
    /// parsed: "Rosé Pine" cannot be a bare word in a settings file without
    /// quoting it in every shell and every editor that reads the file.
    pub key: &'static str,
    /// Whether the background is light.
    ///
    /// Not derivable from the colours at the point of use -- the derivation in
    /// `Theme::from_palette` has to know which direction to mix, and the
    /// style configurator uses it to decide whether a swatch needs a light or a
    /// dark frame. A test checks it agrees with the background's luminance.
    pub light: bool,
    /// Behind the text.
    pub background: Rgb,
    /// Default text colour, and the anchor every neutral is mixed towards.
    pub foreground: Rgb,
    /// Behind the line numbers.
    pub gutter: Rgb,
    /// Behind selected text.
    pub selection: Rgb,
    /// The caret.
    pub caret: Rgb,
    /// Links, the enabled toggle, the bookmark dot, the icon accent.
    pub accent: Rgb,
    /// One colour per syntax style, in [`nn_core::Style`] order: default,
    /// keyword, type, string, comment, number, function, operator,
    /// preprocessor.
    pub syntax: [Rgb; 9],
}

/// Every theme the editor ships, in the order Preferences cycles them.
///
/// Alphabetical by name, with the two built-in palettes left out: those are
/// `Theme::DARK` and `Theme::LIGHT`, which are hand-tuned against Notepad++'s
/// own colours rather than derived, and folding them into this table would mean
/// giving up the fidelity that makes them look like the thing they copy.
pub const CATALOGUE: [Palette; 47] = [
    Palette {
        name: "Ayu Dark",
        key: "ayu_dark",
        light: false,
        background: Rgb::new(11, 14, 20),
        foreground: Rgb::new(179, 177, 173),
        gutter: Rgb::new(13, 16, 23),
        selection: Rgb::new(29, 42, 58),
        caret: Rgb::new(230, 180, 80),
        accent: Rgb::new(230, 180, 80),
        syntax: [
            Rgb::new(179, 177, 173),    // default
            Rgb::new(255, 143, 64),     // keyword
            Rgb::new(89, 194, 255),     // type
            Rgb::new(170, 217, 76),     // string
            Rgb::new(98, 106, 115),     // comment
            Rgb::new(210, 166, 255),    // number
            Rgb::new(255, 180, 84),     // function
            Rgb::new(242, 150, 104),    // operator
            Rgb::new(255, 143, 64),     // preprocessor
        ],
    },
    Palette {
        name: "Ayu Light",
        key: "ayu_light",
        light: true,
        background: Rgb::new(250, 250, 250),
        foreground: Rgb::new(92, 103, 115),
        gutter: Rgb::new(240, 240, 240),
        selection: Rgb::new(209, 232, 255),
        caret: Rgb::new(255, 153, 64),
        accent: Rgb::new(255, 153, 64),
        syntax: [
            Rgb::new(92, 103, 115),     // default
            Rgb::new(250, 141, 62),     // keyword
            Rgb::new(85, 180, 212),     // type
            Rgb::new(134, 179, 0),      // string
            Rgb::new(171, 176, 182),    // comment
            Rgb::new(163, 122, 204),    // number
            Rgb::new(242, 174, 73),     // function
            Rgb::new(237, 147, 102),    // operator
            Rgb::new(250, 141, 62),     // preprocessor
        ],
    },
    Palette {
        name: "Ayu Mirage",
        key: "ayu_mirage",
        light: false,
        background: Rgb::new(31, 36, 48),
        foreground: Rgb::new(203, 204, 198),
        gutter: Rgb::new(27, 32, 42),
        selection: Rgb::new(51, 65, 94),
        caret: Rgb::new(230, 180, 80),
        accent: Rgb::new(255, 204, 102),
        syntax: [
            Rgb::new(203, 204, 198),    // default
            Rgb::new(255, 167, 89),     // keyword
            Rgb::new(115, 208, 255),    // type
            Rgb::new(186, 230, 126),    // string
            Rgb::new(92, 103, 115),     // comment
            Rgb::new(223, 191, 255),    // number
            Rgb::new(255, 213, 128),    // function
            Rgb::new(242, 158, 116),    // operator
            Rgb::new(255, 167, 89),     // preprocessor
        ],
    },
    Palette {
        name: "Catppuccin Frappé",
        key: "catppuccin_frappe",
        light: false,
        background: Rgb::new(48, 52, 70),
        foreground: Rgb::new(198, 208, 245),
        gutter: Rgb::new(41, 44, 60),
        selection: Rgb::new(81, 87, 109),
        caret: Rgb::new(242, 213, 207),
        accent: Rgb::new(140, 170, 238),
        syntax: [
            Rgb::new(198, 208, 245),    // default
            Rgb::new(202, 158, 230),    // keyword
            Rgb::new(229, 200, 144),    // type
            Rgb::new(166, 209, 137),    // string
            Rgb::new(131, 139, 167),    // comment
            Rgb::new(239, 159, 118),    // number
            Rgb::new(140, 170, 238),    // function
            Rgb::new(153, 209, 219),    // operator
            Rgb::new(244, 184, 228),    // preprocessor
        ],
    },
    Palette {
        name: "Catppuccin Latte",
        key: "catppuccin_latte",
        light: true,
        background: Rgb::new(239, 241, 245),
        foreground: Rgb::new(76, 79, 105),
        gutter: Rgb::new(230, 233, 239),
        selection: Rgb::new(204, 208, 218),
        caret: Rgb::new(220, 138, 120),
        accent: Rgb::new(30, 102, 245),
        syntax: [
            Rgb::new(76, 79, 105),    // default
            Rgb::new(136, 57, 239),   // keyword
            Rgb::new(223, 142, 29),   // type
            Rgb::new(64, 160, 43),    // string
            Rgb::new(156, 160, 176),  // comment
            Rgb::new(254, 100, 11),   // number
            Rgb::new(30, 102, 245),   // function
            Rgb::new(4, 165, 229),    // operator
            Rgb::new(234, 118, 203),  // preprocessor
        ],
    },
    Palette {
        name: "Catppuccin Macchiato",
        key: "catppuccin_macchiato",
        light: false,
        background: Rgb::new(36, 39, 58),
        foreground: Rgb::new(202, 211, 245),
        gutter: Rgb::new(30, 32, 48),
        selection: Rgb::new(91, 96, 120),
        caret: Rgb::new(244, 219, 214),
        accent: Rgb::new(138, 173, 244),
        syntax: [
            Rgb::new(202, 211, 245),    // default
            Rgb::new(198, 160, 246),    // keyword
            Rgb::new(238, 212, 159),    // type
            Rgb::new(166, 218, 149),    // string
            Rgb::new(128, 135, 162),    // comment
            Rgb::new(245, 169, 127),    // number
            Rgb::new(138, 173, 244),    // function
            Rgb::new(145, 215, 227),    // operator
            Rgb::new(245, 189, 230),    // preprocessor
        ],
    },
    Palette {
        name: "Catppuccin Mocha",
        key: "catppuccin_mocha",
        light: false,
        background: Rgb::new(30, 30, 46),
        foreground: Rgb::new(205, 214, 244),
        gutter: Rgb::new(24, 24, 37),
        selection: Rgb::new(88, 91, 112),
        caret: Rgb::new(245, 224, 220),
        accent: Rgb::new(137, 180, 250),
        syntax: [
            Rgb::new(205, 214, 244),  // default
            Rgb::new(203, 166, 247),  // keyword
            Rgb::new(249, 226, 175),  // type
            Rgb::new(166, 227, 161),  // string
            Rgb::new(108, 112, 134),  // comment
            Rgb::new(250, 179, 135),  // number
            Rgb::new(137, 180, 250),  // function
            Rgb::new(137, 220, 235),  // operator
            Rgb::new(245, 194, 231),  // preprocessor
        ],
    },
    Palette {
        name: "Darcula",
        key: "darcula",
        light: false,
        background: Rgb::new(43, 43, 43),
        foreground: Rgb::new(169, 183, 198),
        gutter: Rgb::new(49, 51, 53),
        selection: Rgb::new(33, 66, 131),
        caret: Rgb::new(169, 183, 198),
        accent: Rgb::new(152, 118, 170),
        syntax: [
            Rgb::new(169, 183, 198),    // default
            Rgb::new(204, 120, 50),     // keyword
            Rgb::new(169, 183, 198),    // type
            Rgb::new(106, 135, 89),     // string
            Rgb::new(128, 128, 128),    // comment
            Rgb::new(104, 151, 187),    // number
            Rgb::new(255, 198, 109),    // function
            Rgb::new(169, 183, 198),    // operator
            Rgb::new(204, 120, 50),     // preprocessor
        ],
    },
    Palette {
        name: "Dracula",
        key: "dracula",
        light: false,
        background: Rgb::new(40, 42, 54),
        foreground: Rgb::new(248, 248, 242),
        gutter: Rgb::new(33, 34, 44),
        selection: Rgb::new(68, 71, 90),
        caret: Rgb::new(248, 248, 242),
        accent: Rgb::new(189, 147, 249),
        syntax: [
            Rgb::new(248, 248, 242),  // default
            Rgb::new(255, 121, 198),  // keyword
            Rgb::new(139, 233, 253),  // type
            Rgb::new(241, 250, 140),  // string
            Rgb::new(98, 114, 164),   // comment
            Rgb::new(189, 147, 249),  // number
            Rgb::new(80, 250, 123),   // function
            Rgb::new(255, 121, 198),  // operator
            Rgb::new(255, 184, 108),  // preprocessor
        ],
    },
    Palette {
        name: "Everforest",
        key: "everforest",
        light: false,
        background: Rgb::new(45, 53, 59),
        foreground: Rgb::new(211, 198, 170),
        gutter: Rgb::new(39, 46, 51),
        selection: Rgb::new(71, 82, 88),
        caret: Rgb::new(211, 198, 170),
        accent: Rgb::new(167, 192, 128),
        syntax: [
            Rgb::new(211, 198, 170),  // default
            Rgb::new(230, 126, 128),  // keyword
            Rgb::new(219, 188, 127),  // type
            Rgb::new(167, 192, 128),  // string
            Rgb::new(133, 146, 137),  // comment
            Rgb::new(214, 153, 182),  // number
            Rgb::new(127, 187, 179),  // function
            Rgb::new(230, 152, 117),  // operator
            Rgb::new(230, 152, 117),  // preprocessor
        ],
    },
    Palette {
        name: "Everforest Light",
        key: "everforest_light",
        light: true,
        background: Rgb::new(253, 246, 227),
        foreground: Rgb::new(92, 106, 114),
        gutter: Rgb::new(244, 240, 217),
        selection: Rgb::new(230, 226, 204),
        caret: Rgb::new(245, 125, 38),
        accent: Rgb::new(141, 161, 1),
        syntax: [
            Rgb::new(92, 106, 114),     // default
            Rgb::new(141, 161, 1),      // keyword
            Rgb::new(58, 148, 197),     // type
            Rgb::new(53, 167, 124),     // string
            Rgb::new(122, 132, 120),    // comment
            Rgb::new(223, 105, 186),    // number
            Rgb::new(245, 125, 38),     // function
            Rgb::new(248, 85, 82),      // operator
            Rgb::new(223, 160, 0),      // preprocessor
        ],
    },
    Palette {
        name: "GitHub Dark",
        key: "github_dark",
        light: false,
        background: Rgb::new(13, 17, 23),
        foreground: Rgb::new(201, 209, 217),
        gutter: Rgb::new(1, 4, 9),
        selection: Rgb::new(38, 79, 120),
        caret: Rgb::new(201, 209, 217),
        accent: Rgb::new(88, 166, 255),
        syntax: [
            Rgb::new(201, 209, 217),  // default
            Rgb::new(255, 123, 114),  // keyword
            Rgb::new(255, 166, 87),   // type
            Rgb::new(165, 214, 255),  // string
            Rgb::new(139, 148, 158),  // comment
            Rgb::new(121, 192, 255),  // number
            Rgb::new(210, 168, 255),  // function
            Rgb::new(255, 123, 114),  // operator
            Rgb::new(255, 166, 87),   // preprocessor
        ],
    },
    Palette {
        name: "GitHub Dark Dimmed",
        key: "github_dark_dimmed",
        light: false,
        background: Rgb::new(34, 39, 46),
        foreground: Rgb::new(173, 186, 199),
        gutter: Rgb::new(28, 33, 40),
        selection: Rgb::new(61, 68, 77),
        caret: Rgb::new(173, 186, 199),
        accent: Rgb::new(83, 155, 245),
        syntax: [
            Rgb::new(173, 186, 199),    // default
            Rgb::new(244, 112, 103),    // keyword
            Rgb::new(108, 182, 255),    // type
            Rgb::new(150, 208, 255),    // string
            Rgb::new(118, 131, 144),    // comment
            Rgb::new(108, 182, 255),    // number
            Rgb::new(220, 189, 251),    // function
            Rgb::new(246, 157, 80),     // operator
            Rgb::new(141, 219, 224),    // preprocessor
        ],
    },
    Palette {
        name: "GitHub Light",
        key: "github_light",
        light: true,
        background: Rgb::new(255, 255, 255),
        foreground: Rgb::new(36, 41, 47),
        gutter: Rgb::new(246, 248, 250),
        selection: Rgb::new(187, 214, 248),
        caret: Rgb::new(36, 41, 47),
        accent: Rgb::new(9, 105, 218),
        syntax: [
            Rgb::new(36, 41, 47),     // default
            Rgb::new(207, 34, 46),    // keyword
            Rgb::new(149, 56, 0),     // type
            Rgb::new(10, 48, 105),    // string
            Rgb::new(110, 119, 129),  // comment
            Rgb::new(5, 80, 174),     // number
            Rgb::new(130, 80, 223),   // function
            Rgb::new(207, 34, 46),    // operator
            Rgb::new(149, 56, 0),     // preprocessor
        ],
    },
    Palette {
        name: "Gruvbox Dark",
        key: "gruvbox_dark",
        light: false,
        background: Rgb::new(40, 40, 40),
        foreground: Rgb::new(235, 219, 178),
        gutter: Rgb::new(29, 32, 33),
        selection: Rgb::new(80, 73, 69),
        caret: Rgb::new(235, 219, 178),
        accent: Rgb::new(250, 189, 47),
        syntax: [
            Rgb::new(235, 219, 178),  // default
            Rgb::new(251, 73, 52),    // keyword
            Rgb::new(250, 189, 47),   // type
            Rgb::new(184, 187, 38),   // string
            Rgb::new(146, 131, 116),  // comment
            Rgb::new(211, 134, 155),  // number
            Rgb::new(142, 192, 124),  // function
            Rgb::new(131, 165, 152),  // operator
            Rgb::new(254, 128, 25),   // preprocessor
        ],
    },
    Palette {
        name: "Gruvbox Light",
        key: "gruvbox_light",
        light: true,
        background: Rgb::new(251, 241, 199),
        foreground: Rgb::new(60, 56, 54),
        gutter: Rgb::new(242, 229, 188),
        selection: Rgb::new(235, 219, 178),
        caret: Rgb::new(60, 56, 54),
        accent: Rgb::new(181, 118, 20),
        syntax: [
            Rgb::new(60, 56, 54),     // default
            Rgb::new(157, 0, 6),      // keyword
            Rgb::new(181, 118, 20),   // type
            Rgb::new(121, 116, 14),   // string
            Rgb::new(146, 131, 116),  // comment
            Rgb::new(143, 63, 113),   // number
            Rgb::new(66, 123, 88),    // function
            Rgb::new(7, 102, 120),    // operator
            Rgb::new(175, 58, 3),     // preprocessor
        ],
    },
    Palette {
        name: "Horizon",
        key: "horizon",
        light: false,
        background: Rgb::new(28, 30, 38),
        foreground: Rgb::new(213, 216, 218),
        gutter: Rgb::new(22, 23, 29),
        selection: Rgb::new(46, 48, 62),
        caret: Rgb::new(233, 86, 120),
        accent: Rgb::new(37, 176, 188),
        syntax: [
            Rgb::new(213, 216, 218),    // default
            Rgb::new(184, 119, 219),    // keyword
            Rgb::new(38, 187, 217),     // type
            Rgb::new(41, 211, 152),     // string
            Rgb::new(108, 111, 147),    // comment
            Rgb::new(250, 183, 149),    // number
            Rgb::new(37, 176, 188),     // function
            Rgb::new(233, 86, 120),     // operator
            Rgb::new(250, 194, 154),    // preprocessor
        ],
    },
    Palette {
        name: "Iceberg",
        key: "iceberg",
        light: false,
        background: Rgb::new(22, 24, 33),
        foreground: Rgb::new(198, 200, 209),
        gutter: Rgb::new(30, 33, 50),
        selection: Rgb::new(46, 49, 63),
        caret: Rgb::new(226, 120, 120),
        accent: Rgb::new(132, 160, 198),
        syntax: [
            Rgb::new(198, 200, 209),    // default
            Rgb::new(160, 147, 199),    // keyword
            Rgb::new(137, 184, 194),    // type
            Rgb::new(180, 190, 130),    // string
            Rgb::new(107, 112, 137),    // comment
            Rgb::new(226, 164, 120),    // number
            Rgb::new(132, 160, 198),    // function
            Rgb::new(137, 184, 194),    // operator
            Rgb::new(226, 120, 120),    // preprocessor
        ],
    },
    Palette {
        name: "Kanagawa",
        key: "kanagawa",
        light: false,
        background: Rgb::new(31, 31, 40),
        foreground: Rgb::new(220, 215, 186),
        gutter: Rgb::new(22, 22, 29),
        selection: Rgb::new(45, 79, 103),
        caret: Rgb::new(200, 192, 147),
        accent: Rgb::new(126, 156, 216),
        syntax: [
            Rgb::new(220, 215, 186),  // default
            Rgb::new(149, 127, 184),  // keyword
            Rgb::new(127, 180, 202),  // type
            Rgb::new(152, 187, 108),  // string
            Rgb::new(114, 113, 105),  // comment
            Rgb::new(210, 126, 153),  // number
            Rgb::new(126, 156, 216),  // function
            Rgb::new(192, 163, 110),  // operator
            Rgb::new(255, 160, 102),  // preprocessor
        ],
    },
    Palette {
        name: "Kanagawa Dragon",
        key: "kanagawa_dragon",
        light: false,
        background: Rgb::new(24, 22, 22),
        foreground: Rgb::new(197, 201, 197),
        gutter: Rgb::new(29, 28, 25),
        selection: Rgb::new(40, 39, 39),
        caret: Rgb::new(196, 116, 110),
        accent: Rgb::new(139, 164, 176),
        syntax: [
            Rgb::new(197, 201, 197),    // default
            Rgb::new(162, 146, 163),    // keyword
            Rgb::new(142, 164, 162),    // type
            Rgb::new(135, 169, 135),    // string
            Rgb::new(98, 94, 90),       // comment
            Rgb::new(185, 141, 123),    // number
            Rgb::new(139, 164, 176),    // function
            Rgb::new(196, 116, 110),    // operator
            Rgb::new(196, 178, 138),    // preprocessor
        ],
    },
    Palette {
        name: "Kanagawa Lotus",
        key: "kanagawa_lotus",
        light: true,
        background: Rgb::new(242, 236, 188),
        foreground: Rgb::new(84, 84, 100),
        gutter: Rgb::new(231, 219, 160),
        selection: Rgb::new(220, 213, 172),
        caret: Rgb::new(200, 64, 83),
        accent: Rgb::new(77, 105, 155),
        syntax: [
            Rgb::new(84, 84, 100),      // default
            Rgb::new(98, 76, 131),      // keyword
            Rgb::new(77, 105, 155),     // type
            Rgb::new(111, 137, 78),     // string
            Rgb::new(138, 137, 128),    // comment
            Rgb::new(179, 91, 121),     // number
            Rgb::new(77, 105, 155),     // function
            Rgb::new(200, 64, 83),      // operator
            Rgb::new(119, 113, 63),     // preprocessor
        ],
    },
    Palette {
        name: "Material Ocean",
        key: "material_ocean",
        light: false,
        background: Rgb::new(15, 17, 26),
        foreground: Rgb::new(143, 147, 162),
        gutter: Rgb::new(10, 12, 18),
        selection: Rgb::new(31, 34, 51),
        caret: Rgb::new(132, 255, 255),
        accent: Rgb::new(132, 255, 255),
        syntax: [
            Rgb::new(143, 147, 162),    // default
            Rgb::new(199, 146, 234),    // keyword
            Rgb::new(255, 203, 107),    // type
            Rgb::new(195, 232, 141),    // string
            Rgb::new(70, 75, 93),       // comment
            Rgb::new(247, 140, 108),    // number
            Rgb::new(130, 170, 255),    // function
            Rgb::new(137, 221, 255),    // operator
            Rgb::new(255, 83, 112),     // preprocessor
        ],
    },
    Palette {
        name: "Material Palenight",
        key: "material_palenight",
        light: false,
        background: Rgb::new(41, 45, 62),
        foreground: Rgb::new(166, 172, 205),
        gutter: Rgb::new(35, 38, 53),
        selection: Rgb::new(58, 63, 88),
        caret: Rgb::new(255, 203, 107),
        accent: Rgb::new(130, 170, 255),
        syntax: [
            Rgb::new(166, 172, 205),    // default
            Rgb::new(199, 146, 234),    // keyword
            Rgb::new(255, 203, 107),    // type
            Rgb::new(195, 232, 141),    // string
            Rgb::new(103, 110, 149),    // comment
            Rgb::new(247, 140, 108),    // number
            Rgb::new(130, 170, 255),    // function
            Rgb::new(137, 221, 255),    // operator
            Rgb::new(255, 83, 112),     // preprocessor
        ],
    },
    Palette {
        name: "Molokai",
        key: "molokai",
        light: false,
        background: Rgb::new(27, 29, 30),
        foreground: Rgb::new(248, 248, 242),
        gutter: Rgb::new(35, 37, 38),
        selection: Rgb::new(73, 72, 62),
        caret: Rgb::new(248, 248, 240),
        accent: Rgb::new(174, 129, 255),
        syntax: [
            Rgb::new(248, 248, 242),    // default
            Rgb::new(249, 38, 114),     // keyword
            Rgb::new(102, 217, 239),    // type
            Rgb::new(230, 219, 116),    // string
            Rgb::new(117, 113, 94),     // comment
            Rgb::new(174, 129, 255),    // number
            Rgb::new(166, 226, 46),     // function
            Rgb::new(253, 151, 31),     // operator
            Rgb::new(174, 129, 255),    // preprocessor
        ],
    },
    Palette {
        name: "Monokai",
        key: "monokai",
        light: false,
        background: Rgb::new(39, 40, 34),
        foreground: Rgb::new(248, 248, 242),
        gutter: Rgb::new(30, 31, 28),
        selection: Rgb::new(73, 72, 62),
        caret: Rgb::new(248, 248, 240),
        accent: Rgb::new(102, 217, 239),
        syntax: [
            Rgb::new(248, 248, 242),  // default
            Rgb::new(249, 38, 114),   // keyword
            Rgb::new(102, 217, 239),  // type
            Rgb::new(230, 219, 116),  // string
            Rgb::new(117, 113, 94),   // comment
            Rgb::new(174, 129, 255),  // number
            Rgb::new(166, 226, 46),   // function
            Rgb::new(249, 38, 114),   // operator
            Rgb::new(174, 129, 255),  // preprocessor
        ],
    },
    Palette {
        name: "Monokai Pro",
        key: "monokai_pro",
        light: false,
        background: Rgb::new(45, 42, 46),
        foreground: Rgb::new(252, 252, 250),
        gutter: Rgb::new(34, 31, 34),
        selection: Rgb::new(64, 62, 65),
        caret: Rgb::new(255, 215, 106),
        accent: Rgb::new(255, 215, 106),
        syntax: [
            Rgb::new(252, 252, 250),    // default
            Rgb::new(255, 97, 136),     // keyword
            Rgb::new(120, 220, 232),    // type
            Rgb::new(255, 216, 102),    // string
            Rgb::new(114, 112, 114),    // comment
            Rgb::new(252, 152, 103),    // number
            Rgb::new(169, 220, 118),    // function
            Rgb::new(252, 152, 103),    // operator
            Rgb::new(171, 157, 242),    // preprocessor
        ],
    },
    Palette {
        name: "Moonlight",
        key: "moonlight",
        light: false,
        background: Rgb::new(30, 32, 48),
        foreground: Rgb::new(200, 211, 245),
        gutter: Rgb::new(25, 26, 42),
        selection: Rgb::new(45, 63, 118),
        caret: Rgb::new(130, 170, 255),
        accent: Rgb::new(130, 170, 255),
        syntax: [
            Rgb::new(200, 211, 245),    // default
            Rgb::new(192, 153, 255),    // keyword
            Rgb::new(255, 199, 119),    // type
            Rgb::new(195, 232, 141),    // string
            Rgb::new(99, 109, 166),     // comment
            Rgb::new(255, 150, 108),    // number
            Rgb::new(130, 170, 255),    // function
            Rgb::new(134, 225, 252),    // operator
            Rgb::new(255, 117, 127),    // preprocessor
        ],
    },
    Palette {
        name: "Night Owl",
        key: "night_owl",
        light: false,
        background: Rgb::new(1, 22, 39),
        foreground: Rgb::new(214, 222, 235),
        gutter: Rgb::new(1, 17, 29),
        selection: Rgb::new(29, 59, 83),
        caret: Rgb::new(128, 164, 194),
        accent: Rgb::new(130, 170, 255),
        syntax: [
            Rgb::new(214, 222, 235),  // default
            Rgb::new(199, 146, 234),  // keyword
            Rgb::new(255, 203, 139),  // type
            Rgb::new(236, 196, 141),  // string
            Rgb::new(99, 119, 119),   // comment
            Rgb::new(247, 140, 108),  // number
            Rgb::new(130, 170, 255),  // function
            Rgb::new(127, 219, 202),  // operator
            Rgb::new(255, 88, 116),   // preprocessor
        ],
    },
    Palette {
        name: "Nightfox",
        key: "nightfox",
        light: false,
        background: Rgb::new(25, 35, 48),
        foreground: Rgb::new(205, 206, 207),
        gutter: Rgb::new(19, 26, 36),
        selection: Rgb::new(43, 59, 81),
        caret: Rgb::new(174, 175, 176),
        accent: Rgb::new(113, 156, 214),
        syntax: [
            Rgb::new(205, 206, 207),    // default
            Rgb::new(157, 121, 214),    // keyword
            Rgb::new(99, 205, 207),     // type
            Rgb::new(129, 178, 154),    // string
            Rgb::new(82, 97, 117),      // comment
            Rgb::new(219, 192, 116),    // number
            Rgb::new(113, 156, 214),    // function
            Rgb::new(244, 162, 97),     // operator
            Rgb::new(201, 79, 109),     // preprocessor
        ],
    },
    Palette {
        name: "Nord",
        key: "nord",
        light: false,
        background: Rgb::new(46, 52, 64),
        foreground: Rgb::new(216, 222, 233),
        gutter: Rgb::new(39, 44, 54),
        selection: Rgb::new(67, 76, 94),
        caret: Rgb::new(216, 222, 233),
        accent: Rgb::new(136, 192, 208),
        syntax: [
            Rgb::new(216, 222, 233),  // default
            Rgb::new(129, 161, 193),  // keyword
            Rgb::new(143, 188, 187),  // type
            Rgb::new(163, 190, 140),  // string
            Rgb::new(97, 110, 136),   // comment
            Rgb::new(180, 142, 173),  // number
            Rgb::new(136, 192, 208),  // function
            Rgb::new(129, 161, 193),  // operator
            Rgb::new(208, 135, 112),  // preprocessor
        ],
    },
    Palette {
        name: "Oceanic Next",
        key: "oceanic_next",
        light: false,
        background: Rgb::new(27, 43, 52),
        foreground: Rgb::new(192, 197, 206),
        gutter: Rgb::new(22, 38, 46),
        selection: Rgb::new(79, 91, 102),
        caret: Rgb::new(192, 197, 206),
        accent: Rgb::new(102, 153, 204),
        syntax: [
            Rgb::new(192, 197, 206),    // default
            Rgb::new(197, 148, 197),    // keyword
            Rgb::new(250, 200, 99),     // type
            Rgb::new(153, 199, 148),    // string
            Rgb::new(101, 115, 126),    // comment
            Rgb::new(249, 145, 87),     // number
            Rgb::new(102, 153, 204),    // function
            Rgb::new(95, 179, 179),     // operator
            Rgb::new(197, 148, 197),    // preprocessor
        ],
    },
    Palette {
        name: "One Dark",
        key: "one_dark",
        light: false,
        background: Rgb::new(40, 44, 52),
        foreground: Rgb::new(171, 178, 191),
        gutter: Rgb::new(33, 37, 43),
        selection: Rgb::new(62, 68, 81),
        caret: Rgb::new(82, 139, 255),
        accent: Rgb::new(97, 175, 239),
        syntax: [
            Rgb::new(171, 178, 191),  // default
            Rgb::new(198, 120, 221),  // keyword
            Rgb::new(229, 192, 123),  // type
            Rgb::new(152, 195, 121),  // string
            Rgb::new(92, 99, 112),    // comment
            Rgb::new(209, 154, 102),  // number
            Rgb::new(97, 175, 239),   // function
            Rgb::new(86, 182, 194),   // operator
            Rgb::new(198, 120, 221),  // preprocessor
        ],
    },
    Palette {
        name: "One Light",
        key: "one_light",
        light: true,
        background: Rgb::new(250, 250, 250),
        foreground: Rgb::new(56, 58, 66),
        gutter: Rgb::new(234, 234, 235),
        selection: Rgb::new(229, 229, 230),
        caret: Rgb::new(82, 111, 255),
        accent: Rgb::new(64, 120, 242),
        syntax: [
            Rgb::new(56, 58, 66),     // default
            Rgb::new(166, 38, 164),   // keyword
            Rgb::new(193, 132, 1),    // type
            Rgb::new(80, 161, 79),    // string
            Rgb::new(160, 161, 167),  // comment
            Rgb::new(152, 104, 1),    // number
            Rgb::new(64, 120, 242),   // function
            Rgb::new(1, 132, 188),    // operator
            Rgb::new(166, 38, 164),   // preprocessor
        ],
    },
    Palette {
        name: "Panda Syntax",
        key: "panda_syntax",
        light: false,
        background: Rgb::new(41, 42, 43),
        foreground: Rgb::new(230, 230, 230),
        gutter: Rgb::new(30, 31, 32),
        selection: Rgb::new(62, 68, 81),
        caret: Rgb::new(255, 117, 181),
        accent: Rgb::new(111, 193, 255),
        syntax: [
            Rgb::new(230, 230, 230),    // default
            Rgb::new(255, 117, 181),    // keyword
            Rgb::new(69, 169, 249),     // type
            Rgb::new(25, 249, 216),     // string
            Rgb::new(103, 107, 121),    // comment
            Rgb::new(255, 184, 108),    // number
            Rgb::new(111, 193, 255),    // function
            Rgb::new(176, 132, 235),    // operator
            Rgb::new(255, 44, 109),     // preprocessor
        ],
    },
    Palette {
        name: "Quiet Light",
        key: "quiet_light",
        light: true,
        background: Rgb::new(245, 245, 245),
        foreground: Rgb::new(51, 51, 51),
        gutter: Rgb::new(232, 232, 232),
        selection: Rgb::new(199, 219, 252),
        caret: Rgb::new(122, 62, 157),
        accent: Rgb::new(0, 122, 204),
        syntax: [
            Rgb::new(51, 51, 51),       // default
            Rgb::new(112, 86, 151),     // keyword
            Rgb::new(75, 131, 205),     // type
            Rgb::new(68, 140, 39),      // string
            Rgb::new(170, 170, 170),    // comment
            Rgb::new(170, 55, 49),      // number
            Rgb::new(0, 122, 204),      // function
            Rgb::new(119, 119, 119),    // operator
            Rgb::new(112, 86, 151),     // preprocessor
        ],
    },
    Palette {
        name: "Rosé Pine",
        key: "rose_pine",
        light: false,
        background: Rgb::new(25, 23, 36),
        foreground: Rgb::new(224, 222, 244),
        gutter: Rgb::new(31, 29, 46),
        selection: Rgb::new(64, 61, 82),
        caret: Rgb::new(82, 79, 103),
        accent: Rgb::new(235, 188, 186),
        syntax: [
            Rgb::new(224, 222, 244),  // default
            Rgb::new(196, 167, 231),  // keyword
            Rgb::new(49, 116, 143),   // type
            Rgb::new(246, 193, 119),  // string
            Rgb::new(110, 106, 134),  // comment
            Rgb::new(235, 188, 186),  // number
            Rgb::new(156, 207, 216),  // function
            Rgb::new(235, 111, 146),  // operator
            Rgb::new(246, 193, 119),  // preprocessor
        ],
    },
    Palette {
        name: "Rosé Pine Dawn",
        key: "rose_pine_dawn",
        light: true,
        background: Rgb::new(250, 244, 237),
        foreground: Rgb::new(87, 82, 121),
        gutter: Rgb::new(242, 233, 225),
        selection: Rgb::new(223, 216, 208),
        caret: Rgb::new(180, 99, 122),
        accent: Rgb::new(144, 122, 169),
        syntax: [
            Rgb::new(87, 82, 121),      // default
            Rgb::new(144, 122, 169),    // keyword
            Rgb::new(40, 105, 131),     // type
            Rgb::new(86, 148, 159),     // string
            Rgb::new(152, 147, 165),    // comment
            Rgb::new(234, 157, 52),     // number
            Rgb::new(180, 99, 122),     // function
            Rgb::new(215, 130, 126),    // operator
            Rgb::new(144, 122, 169),    // preprocessor
        ],
    },
    Palette {
        name: "Rosé Pine Moon",
        key: "rose_pine_moon",
        light: false,
        background: Rgb::new(35, 33, 54),
        foreground: Rgb::new(224, 222, 244),
        gutter: Rgb::new(31, 29, 46),
        selection: Rgb::new(57, 53, 82),
        caret: Rgb::new(235, 111, 146),
        accent: Rgb::new(196, 167, 231),
        syntax: [
            Rgb::new(224, 222, 244),    // default
            Rgb::new(196, 167, 231),    // keyword
            Rgb::new(62, 143, 176),     // type
            Rgb::new(246, 193, 119),    // string
            Rgb::new(110, 106, 134),    // comment
            Rgb::new(234, 154, 151),    // number
            Rgb::new(156, 207, 216),    // function
            Rgb::new(235, 111, 146),    // operator
            Rgb::new(196, 167, 231),    // preprocessor
        ],
    },
    Palette {
        name: "Shades of Purple",
        key: "shades_of_purple",
        light: false,
        background: Rgb::new(30, 30, 63),
        foreground: Rgb::new(255, 255, 255),
        gutter: Rgb::new(24, 24, 51),
        selection: Rgb::new(45, 43, 85),
        caret: Rgb::new(250, 208, 0),
        accent: Rgb::new(250, 208, 0),
        syntax: [
            Rgb::new(255, 255, 255),    // default
            Rgb::new(250, 208, 0),      // keyword
            Rgb::new(158, 255, 255),    // type
            Rgb::new(165, 255, 144),    // string
            Rgb::new(179, 98, 255),     // comment
            Rgb::new(255, 98, 140),     // number
            Rgb::new(251, 148, 255),    // function
            Rgb::new(250, 208, 0),      // operator
            Rgb::new(251, 148, 255),    // preprocessor
        ],
    },
    Palette {
        name: "Solarized Dark",
        key: "solarized_dark",
        light: false,
        background: Rgb::new(0, 43, 54),
        foreground: Rgb::new(131, 148, 150),
        gutter: Rgb::new(7, 54, 66),
        selection: Rgb::new(7, 54, 66),
        caret: Rgb::new(147, 161, 161),
        accent: Rgb::new(38, 139, 210),
        syntax: [
            Rgb::new(131, 148, 150),  // default
            Rgb::new(133, 153, 0),    // keyword
            Rgb::new(181, 137, 0),    // type
            Rgb::new(42, 161, 152),   // string
            Rgb::new(88, 110, 117),   // comment
            Rgb::new(211, 54, 130),   // number
            Rgb::new(38, 139, 210),   // function
            Rgb::new(108, 113, 196),  // operator
            Rgb::new(203, 75, 22),    // preprocessor
        ],
    },
    Palette {
        name: "Solarized Light",
        key: "solarized_light",
        light: true,
        background: Rgb::new(253, 246, 227),
        foreground: Rgb::new(101, 123, 131),
        gutter: Rgb::new(238, 232, 213),
        selection: Rgb::new(238, 232, 213),
        caret: Rgb::new(88, 110, 117),
        accent: Rgb::new(38, 139, 210),
        syntax: [
            Rgb::new(101, 123, 131),  // default
            Rgb::new(133, 153, 0),    // keyword
            Rgb::new(181, 137, 0),    // type
            Rgb::new(42, 161, 152),   // string
            Rgb::new(147, 161, 161),  // comment
            Rgb::new(211, 54, 130),   // number
            Rgb::new(38, 139, 210),   // function
            Rgb::new(108, 113, 196),  // operator
            Rgb::new(203, 75, 22),    // preprocessor
        ],
    },
    Palette {
        name: "Synthwave '84",
        key: "synthwave_84",
        light: false,
        background: Rgb::new(42, 33, 57),
        foreground: Rgb::new(255, 255, 255),
        gutter: Rgb::new(36, 27, 48),
        selection: Rgb::new(70, 52, 101),
        caret: Rgb::new(54, 249, 246),
        accent: Rgb::new(54, 249, 246),
        syntax: [
            Rgb::new(255, 255, 255),    // default
            Rgb::new(249, 42, 173),     // keyword
            Rgb::new(54, 249, 246),     // type
            Rgb::new(255, 139, 57),     // string
            Rgb::new(132, 139, 189),    // comment
            Rgb::new(254, 222, 93),     // number
            Rgb::new(114, 241, 184),    // function
            Rgb::new(255, 139, 57),     // operator
            Rgb::new(249, 42, 173),     // preprocessor
        ],
    },
    Palette {
        name: "Tokyo Night",
        key: "tokyo_night",
        light: false,
        background: Rgb::new(26, 27, 38),
        foreground: Rgb::new(192, 202, 245),
        gutter: Rgb::new(22, 22, 30),
        selection: Rgb::new(40, 52, 87),
        caret: Rgb::new(192, 202, 245),
        accent: Rgb::new(122, 162, 247),
        syntax: [
            Rgb::new(192, 202, 245),  // default
            Rgb::new(187, 154, 247),  // keyword
            Rgb::new(42, 195, 222),   // type
            Rgb::new(158, 206, 106),  // string
            Rgb::new(86, 95, 137),    // comment
            Rgb::new(255, 158, 100),  // number
            Rgb::new(122, 162, 247),  // function
            Rgb::new(137, 221, 255),  // operator
            Rgb::new(224, 175, 104),  // preprocessor
        ],
    },
    Palette {
        name: "Tokyo Night Day",
        key: "tokyo_night_day",
        light: true,
        background: Rgb::new(225, 226, 231),
        foreground: Rgb::new(55, 96, 191),
        gutter: Rgb::new(208, 213, 227),
        selection: Rgb::new(182, 191, 226),
        caret: Rgb::new(55, 96, 191),
        accent: Rgb::new(46, 125, 233),
        syntax: [
            Rgb::new(55, 96, 191),      // default
            Rgb::new(152, 84, 241),     // keyword
            Rgb::new(0, 113, 151),      // type
            Rgb::new(88, 117, 57),      // string
            Rgb::new(132, 140, 181),    // comment
            Rgb::new(177, 92, 0),       // number
            Rgb::new(46, 125, 233),     // function
            Rgb::new(0, 113, 151),      // operator
            Rgb::new(245, 42, 101),     // preprocessor
        ],
    },
    Palette {
        name: "Tokyo Night Storm",
        key: "tokyo_night_storm",
        light: false,
        background: Rgb::new(36, 40, 59),
        foreground: Rgb::new(192, 202, 245),
        gutter: Rgb::new(31, 35, 53),
        selection: Rgb::new(54, 74, 130),
        caret: Rgb::new(192, 202, 245),
        accent: Rgb::new(122, 162, 247),
        syntax: [
            Rgb::new(192, 202, 245),    // default
            Rgb::new(187, 154, 247),    // keyword
            Rgb::new(42, 195, 222),     // type
            Rgb::new(158, 206, 106),    // string
            Rgb::new(86, 95, 137),      // comment
            Rgb::new(255, 158, 100),    // number
            Rgb::new(122, 162, 247),    // function
            Rgb::new(137, 221, 255),    // operator
            Rgb::new(224, 175, 104),    // preprocessor
        ],
    },
    Palette {
        name: "Vitesse Dark",
        key: "vitesse_dark",
        light: false,
        background: Rgb::new(18, 18, 18),
        foreground: Rgb::new(219, 215, 202),
        gutter: Rgb::new(24, 24, 24),
        selection: Rgb::new(40, 40, 40),
        caret: Rgb::new(219, 215, 202),
        accent: Rgb::new(77, 147, 117),
        syntax: [
            Rgb::new(219, 215, 202),    // default
            Rgb::new(203, 118, 118),    // keyword
            Rgb::new(99, 148, 191),     // type
            Rgb::new(77, 147, 117),     // string
            Rgb::new(117, 133, 117),    // comment
            Rgb::new(184, 169, 101),    // number
            Rgb::new(93, 169, 148),     // function
            Rgb::new(201, 138, 125),    // operator
            Rgb::new(203, 118, 118),    // preprocessor
        ],
    },
    Palette {
        name: "Zenburn",
        key: "zenburn",
        light: false,
        background: Rgb::new(63, 63, 63),
        foreground: Rgb::new(220, 220, 204),
        gutter: Rgb::new(56, 56, 56),
        selection: Rgb::new(87, 87, 87),
        caret: Rgb::new(220, 220, 204),
        accent: Rgb::new(140, 208, 211),
        syntax: [
            Rgb::new(220, 220, 204),    // default
            Rgb::new(240, 223, 175),    // keyword
            Rgb::new(140, 208, 211),    // type
            Rgb::new(204, 147, 147),    // string
            Rgb::new(127, 159, 127),    // comment
            Rgb::new(223, 175, 143),    // number
            Rgb::new(147, 224, 227),    // function
            Rgb::new(240, 223, 175),    // operator
            Rgb::new(204, 147, 147),    // preprocessor
        ],
    },
];

/// The catalogue entry whose key is `key`, if there is one.
pub fn by_key(key: &str) -> Option<&'static Palette> {
    CATALOGUE.iter().find(|palette| palette.key == key)
}

/// The catalogue entry a theme choice names, or `None` for the three built-ins.
///
/// The choice and the catalogue are two lists of fifty and forty-seven that have
/// to agree about which is which, and they are written apart because the choice
/// belongs to both front-ends while the colours belong to the screen. The index
/// is [`ThemeChoice::named`], so a palette inserted in the middle of
/// [`CATALOGUE`] renumbers nothing: the position is the choice's, not the
/// table's, and the test below is what says the two still line up.
pub fn palette_of(choice: ThemeChoice) -> Option<&'static Palette> {
    choice.named().and_then(|at| CATALOGUE.get(at))
}

/// The theme a choice resolves to.
///
/// `system` is what the desktop said, or `None` when it said nothing -- no
/// portal, no session bus, or a desktop with no preference. The fallback is
/// dark, which is the palette this editor has always opened with, so a machine
/// that cannot answer looks exactly as it did before.
pub fn theme_of(choice: ThemeChoice, system: Option<bool>) -> crate::theme::Theme {
    match choice {
        ThemeChoice::Light => crate::theme::Theme::LIGHT,
        ThemeChoice::Dark => crate::theme::Theme::DARK,
        ThemeChoice::System => {
            if system.unwrap_or(false) {
                crate::theme::Theme::LIGHT
            } else {
                crate::theme::Theme::DARK
            }
        }
        named => palette_of(named).map_or(crate::theme::Theme::DARK, |palette| {
            crate::theme::Theme::from_palette(palette)
        }),
    }
}

/// Whether the theme a choice resolves to is a light one.
///
/// Read off the resolved theme rather than the variant, so a named palette and a
/// `System` answer are described the same way.
pub fn is_light_of(choice: ThemeChoice, system: Option<bool>) -> bool {
    theme_of(choice, system).background.luminance() >= 128
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The relative luminance of a colour, as WCAG defines it.
    fn relative(rgb: Rgb) -> f32 {
        let channel = |c: u8| {
            let s = c as f32 / 255.0;
            if s <= 0.03928 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(rgb.0) + 0.7152 * channel(rgb.1) + 0.0722 * channel(rgb.2)
    }

    /// WCAG's contrast ratio between two colours.
    fn contrast(a: Rgb, b: Rgb) -> f32 {
        let (a, b) = (relative(a), relative(b));
        let (light, dark) = if a > b { (a, b) } else { (b, a) };
        (light + 0.05) / (dark + 0.05)
    }

    #[test]
    fn every_key_is_unique_and_parseable() {
        for (index, palette) in CATALOGUE.iter().enumerate() {
            assert!(!palette.name.is_empty());
            assert!(!palette.key.is_empty(), "{}", palette.name);
            // A key is written to a settings file and read back by splitting on
            // whitespace, so it has to be a bare word.
            assert!(
                palette
                    .key
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{}: {}",
                palette.name,
                palette.key
            );
            for other in &CATALOGUE[index + 1..] {
                assert_ne!(palette.key, other.key);
                assert_ne!(palette.name, other.name);
            }
        }
    }

    #[test]
    fn a_palette_agrees_with_its_own_background() {
        // `light` is what `from_palette` mixes towards, so a palette that says
        // the wrong one derives a theme whose every border is invisible.
        for palette in CATALOGUE {
            let light = palette.background.luminance() >= 128;
            assert_eq!(light, palette.light, "{}", palette.name);
        }
    }

    #[test]
    fn every_palette_is_readable() {
        // 4.0 rather than WCAG AA's 4.5, because Solarized Light is 4.13 by its
        // own design: base0 on base3 is the published pairing and darkening it
        // to pass AA would make it a different theme with the same name. The
        // other seventeen clear 4.5 on their own; this is a floor that catches a
        // palette whose text has gone the wrong way, not a certification.
        for palette in CATALOGUE {
            let ratio = contrast(palette.foreground, palette.background);
            assert!(
                ratio >= 4.0,
                "{}: text contrast {ratio:.2}",
                palette.name
            );
        }
        // The gap is real and worth naming: if a palette ever lands between 4.0
        // and 4.5 the assertion above stops meaning anything, so the count of
        // ones below AA is pinned.
        let below_aa = CATALOGUE
            .iter()
            .filter(|palette| contrast(palette.foreground, palette.background) < 4.5)
            .count();
        assert_eq!(below_aa, 1, "only Solarized Light is designed this dim");
    }

    #[test]
    fn no_syntax_colour_disappears_into_the_background() {
        // The palette module's threshold, applied to the static table: a style
        // within forty of the background's brightness is unreadable at any size.
        for palette in CATALOGUE {
            let background = palette.background.luminance() as i32;
            for (style, colour) in palette.syntax.iter().enumerate() {
                let distance = (colour.luminance() as i32 - background).abs();
                assert!(
                    distance >= 40,
                    "{}: style {style} is {distance} from the background",
                    palette.name
                );
            }
        }
    }

    #[test]
    fn the_catalogue_is_alphabetical() {
        // The Preferences row cycles it, and a list that is alphabetical is the
        // only one a user can find an entry in without reading all of it.
        let mut names: Vec<&str> = CATALOGUE.iter().map(|palette| palette.name).collect();
        names.sort_unstable();
        let listed: Vec<&str> = CATALOGUE.iter().map(|palette| palette.name).collect();
        assert_eq!(names, listed);
    }

    #[test]
    fn the_choice_and_the_catalogue_name_the_same_themes() {
        // Two lists written in two crates: the choice says which themes exist,
        // the catalogue holds their colours. They agree only by construction, so
        // this is what fails when someone adds a palette and forgets the choice,
        // or adds a choice in the wrong place and shifts every colour after it.
        assert_eq!(CATALOGUE.len(), ThemeChoice::NAMES.len());
        let mut named = 0;
        for choice in ThemeChoice::ALL {
            match palette_of(choice) {
                Some(palette) => {
                    named += 1;
                    assert_eq!(palette.name, choice.label(), "{choice:?}");
                    assert_eq!(palette.key, choice.key(), "{choice:?}");
                }
                None => assert!(
                    matches!(
                        choice,
                        ThemeChoice::System | ThemeChoice::Light | ThemeChoice::Dark
                    ),
                    "{choice:?} has no colours",
                ),
            }
        }
        assert_eq!(named, CATALOGUE.len());
    }

    #[test]
    fn a_key_finds_its_palette() {
        for palette in CATALOGUE {
            assert_eq!(by_key(palette.key), Some(&palette));
        }
        assert_eq!(by_key("solarized"), None);
        assert_eq!(by_key(""), None);
    }
}
