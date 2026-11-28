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

//! The themes a document can be drawn in: which ones exist, what they are called,
//! the keyword each is stored as, and the forty-nine colours a theme is.
//!
//! Three pieces. [`ThemeChoice`] is the identity -- the variant, the label, the
//! word in the settings file, the order the preference row steps through.
//! [`Theme`] is forty-nine colours, which a terminal draws in true colour and a
//! window feeds to a shader. And [`Slot`] is the sixteen of those a user may
//! change by hand, with `record` and `effective_theme` turning a list of such
//! changes into a theme. A terminal that offered a different list of themes, or
//! honoured a user's colour edits differently, would be a second editor.
//!
//! The named palettes are a table of their own, [`crate::catalogue`], aligned by
//! position with [`ThemeChoice::NAMES`]: entry `i` of the catalogue is the
//! colours of `NAMES[i]`, and a test there holds the two lists together. What
//! neither of them knows is how a colour becomes pixels: that is `ToGpu` in the
//! windowed crate, and escape sequences in the terminal.
//!
//! Three choices are not in the catalogue: `System` follows the desktop's
//! colour-scheme preference and solves to one of the two built-in palettes, and
//! `Light` and `Dark` are those palettes by name. They are choices because the
//! settings file has to record which of the three the user meant -- a user whose
//! desktop goes dark at sunset expects the editor to follow, and a user who
//! picked a named theme expects it to stay put whatever the desktop does.

use crate::color::{mix, Rgb};

/// One theme the user can pick.
///
/// The named themes are the catalogue in the windowed front-end's `themes`
/// module. They are variants rather than a `Named(&str)` payload so that a typo
/// is a compile error rather than a settings line that silently does nothing --
/// the same reason the languages are a table with typed keys and not a
/// directory of names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeChoice {
    /// Follow the desktop's colour-scheme preference, falling back to dark when
    /// the desktop has no opinion or no portal to ask.
    #[default]
    System,
    /// The built-in light palette, hand-tuned to Notepad++'s own colours.
    Light,
    /// The built-in dark palette, from the Visual Studio Code family.
    Dark,
    AyuDark,
    AyuLight,
    AyuMirage,
    CatppuccinFrappe,
    CatppuccinLatte,
    CatppuccinMacchiato,
    CatppuccinMocha,
    Darcula,
    Dracula,
    Everforest,
    EverforestLight,
    GitHubDark,
    GitHubDarkDimmed,
    GitHubLight,
    GruvboxDark,
    GruvboxLight,
    Horizon,
    Iceberg,
    Kanagawa,
    KanagawaDragon,
    KanagawaLotus,
    MaterialOcean,
    MaterialPalenight,
    Molokai,
    Monokai,
    MonokaiPro,
    Moonlight,
    NightOwl,
    Nightfox,
    Nord,
    OceanicNext,
    OneDark,
    OneLight,
    PandaSyntax,
    QuietLight,
    RosePine,
    RosePineDawn,
    RosePineMoon,
    ShadesOfPurple,
    SolarizedDark,
    SolarizedLight,
    Synthwave84,
    TokyoNight,
    TokyoNightDay,
    TokyoNightStorm,
    VitesseDark,
    Zenburn,
}

impl ThemeChoice {
    /// Every choice, in the order the preferences dialog cycles them.
    ///
    /// `System`, the two built-ins, then the catalogue in its own order. A
    /// cycling row is only navigable if the order is predictable, which is why
    /// the named themes are alphabetical and the two built-ins are not buried
    /// in the middle of them.
    pub const ALL: [ThemeChoice; 50] = [
        ThemeChoice::System,
        ThemeChoice::Light,
        ThemeChoice::Dark,
        ThemeChoice::AyuDark,
        ThemeChoice::AyuLight,
        ThemeChoice::AyuMirage,
        ThemeChoice::CatppuccinFrappe,
        ThemeChoice::CatppuccinLatte,
        ThemeChoice::CatppuccinMacchiato,
        ThemeChoice::CatppuccinMocha,
        ThemeChoice::Darcula,
        ThemeChoice::Dracula,
        ThemeChoice::Everforest,
        ThemeChoice::EverforestLight,
        ThemeChoice::GitHubDark,
        ThemeChoice::GitHubDarkDimmed,
        ThemeChoice::GitHubLight,
        ThemeChoice::GruvboxDark,
        ThemeChoice::GruvboxLight,
        ThemeChoice::Horizon,
        ThemeChoice::Iceberg,
        ThemeChoice::Kanagawa,
        ThemeChoice::KanagawaDragon,
        ThemeChoice::KanagawaLotus,
        ThemeChoice::MaterialOcean,
        ThemeChoice::MaterialPalenight,
        ThemeChoice::Molokai,
        ThemeChoice::Monokai,
        ThemeChoice::MonokaiPro,
        ThemeChoice::Moonlight,
        ThemeChoice::NightOwl,
        ThemeChoice::Nightfox,
        ThemeChoice::Nord,
        ThemeChoice::OceanicNext,
        ThemeChoice::OneDark,
        ThemeChoice::OneLight,
        ThemeChoice::PandaSyntax,
        ThemeChoice::QuietLight,
        ThemeChoice::RosePine,
        ThemeChoice::RosePineDawn,
        ThemeChoice::RosePineMoon,
        ThemeChoice::ShadesOfPurple,
        ThemeChoice::SolarizedDark,
        ThemeChoice::SolarizedLight,
        ThemeChoice::Synthwave84,
        ThemeChoice::TokyoNight,
        ThemeChoice::TokyoNightDay,
        ThemeChoice::TokyoNightStorm,
        ThemeChoice::VitesseDark,
        ThemeChoice::Zenburn,
    ];

    /// Every named theme's keyword and label, in [`ThemeChoice::ALL`] order.
    ///
    /// The keyword is what the settings file stores and the label is what the
    /// Preferences row and the Settings menu show. They are separate because a
    /// name is displayed and a key is parsed: "Rosé Pine" cannot be a bare word
    /// in a settings file without quoting it in every shell and every editor
    /// that reads the file.
    pub const NAMES: [(&'static str, &'static str); 47] = [
    ("ayu_dark", "Ayu Dark"),
    ("ayu_light", "Ayu Light"),
    ("ayu_mirage", "Ayu Mirage"),
    ("catppuccin_frappe", "Catppuccin Frappé"),
    ("catppuccin_latte", "Catppuccin Latte"),
    ("catppuccin_macchiato", "Catppuccin Macchiato"),
    ("catppuccin_mocha", "Catppuccin Mocha"),
    ("darcula", "Darcula"),
    ("dracula", "Dracula"),
    ("everforest", "Everforest"),
    ("everforest_light", "Everforest Light"),
    ("github_dark", "GitHub Dark"),
    ("github_dark_dimmed", "GitHub Dark Dimmed"),
    ("github_light", "GitHub Light"),
    ("gruvbox_dark", "Gruvbox Dark"),
    ("gruvbox_light", "Gruvbox Light"),
    ("horizon", "Horizon"),
    ("iceberg", "Iceberg"),
    ("kanagawa", "Kanagawa"),
    ("kanagawa_dragon", "Kanagawa Dragon"),
    ("kanagawa_lotus", "Kanagawa Lotus"),
    ("material_ocean", "Material Ocean"),
    ("material_palenight", "Material Palenight"),
    ("molokai", "Molokai"),
    ("monokai", "Monokai"),
    ("monokai_pro", "Monokai Pro"),
    ("moonlight", "Moonlight"),
    ("night_owl", "Night Owl"),
    ("nightfox", "Nightfox"),
    ("nord", "Nord"),
    ("oceanic_next", "Oceanic Next"),
    ("one_dark", "One Dark"),
    ("one_light", "One Light"),
    ("panda_syntax", "Panda Syntax"),
    ("quiet_light", "Quiet Light"),
    ("rose_pine", "Rosé Pine"),
    ("rose_pine_dawn", "Rosé Pine Dawn"),
    ("rose_pine_moon", "Rosé Pine Moon"),
    ("shades_of_purple", "Shades of Purple"),
    ("solarized_dark", "Solarized Dark"),
    ("solarized_light", "Solarized Light"),
    ("synthwave_84", "Synthwave '84"),
    ("tokyo_night", "Tokyo Night"),
    ("tokyo_night_day", "Tokyo Night Day"),
    ("tokyo_night_storm", "Tokyo Night Storm"),
    ("vitesse_dark", "Vitesse Dark"),
    ("zenburn", "Zenburn"),
    ];

    /// Where this choice sits among the named themes, or `None` for the three
    /// built-ins -- which is also the index of its colours in the catalogue.
    pub fn named(self) -> Option<usize> {
        Self::ALL
            .iter()
            .position(|choice| *choice == self)
            .and_then(|at| at.checked_sub(3))
    }

    /// The keyword written to the settings file.
    pub fn key(self) -> &'static str {
        match self.named() {
            Some(at) => Self::NAMES[at].0,
            None => match self {
                ThemeChoice::System => "system",
                ThemeChoice::Light => "light",
                _ => "dark",
            },
        }
    }

    /// The label shown in the preferences dialog and the Settings menu.
    pub fn label(self) -> &'static str {
        match self.named() {
            Some(at) => Self::NAMES[at].1,
            None => match self {
                ThemeChoice::System => "System",
                ThemeChoice::Light => "Light",
                _ => "Dark",
            },
        }
    }

    /// Reads a keyword back, or `None` when it is not one of ours.
    ///
    /// `light` and `dark` are the two keywords the file used before this was a
    /// three-way choice, and they still name the two built-in palettes, so an
    /// existing settings file keeps working and keeps meaning what it meant.
    pub fn from_key(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|choice| choice.key() == text)
    }

    /// The next choice, wrapping.
    pub fn next(self) -> Self {
        let at = Self::ALL
            .iter()
            .position(|choice| *choice == self)
            .unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }

    /// The previous choice, wrapping.
    pub fn previous(self) -> Self {
        let at = Self::ALL
            .iter()
            .position(|choice| *choice == self)
            .unwrap_or(0);
        Self::ALL[(at + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_choice_has_a_key_and_a_label_and_no_two_share_one() {
        let mut keys: Vec<&str> = Vec::new();
        let mut labels: Vec<&str> = Vec::new();
        for choice in ThemeChoice::ALL {
            let key = choice.key();
            let label = choice.label();
            assert!(!key.is_empty());
            assert!(!label.is_empty(), "{key}");
            assert!(!keys.contains(&key), "{key} is used twice");
            assert!(!labels.contains(&label), "{label} is used twice");
            keys.push(key);
            labels.push(label);
        }
        assert_eq!(keys.len(), ThemeChoice::ALL.len());
        assert_eq!(ThemeChoice::NAMES.len(), ThemeChoice::ALL.len() - 3);
    }

    #[test]
    fn every_key_reads_back_as_its_choice() {
        for choice in ThemeChoice::ALL {
            assert_eq!(ThemeChoice::from_key(choice.key()), Some(choice));
        }
        assert_eq!(ThemeChoice::from_key("nonexistent"), None);
        // The two keywords an older settings file used still name the built-ins.
        assert_eq!(ThemeChoice::from_key("light"), Some(ThemeChoice::Light));
        assert_eq!(ThemeChoice::from_key("dark"), Some(ThemeChoice::Dark));
    }

    #[test]
    fn stepping_wraps_in_both_directions() {
        assert_eq!(ThemeChoice::System.next(), ThemeChoice::Light);
        assert_eq!(ThemeChoice::Dark.next(), ThemeChoice::AyuDark);
        assert_eq!(ThemeChoice::Zenburn.next(), ThemeChoice::System);
        assert_eq!(ThemeChoice::System.previous(), ThemeChoice::Zenburn);
        assert_eq!(
            ThemeChoice::Dark.next().previous(),
            ThemeChoice::Dark,
            "a step and a step back cancel"
        );
    }

    #[test]
    fn the_three_built_ins_are_not_in_the_named_table() {
        for (at, choice) in ThemeChoice::ALL.iter().enumerate() {
            let expected = if at < 3 { None } else { Some(at) };
            assert_eq!(choice.named().map(|named| named + 3), expected);
        }
    }
}

/// The colours the editor draws with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Behind the text.
    pub background: Rgb,
    /// Default text colour.
    pub foreground: Rgb,
    /// Behind the line numbers.
    pub gutter_background: Rgb,
    /// Line numbers.
    pub gutter_foreground: Rgb,
    /// The line number of the caret's line.
    pub gutter_active: Rgb,
    /// Behind the caret's line.
    pub current_line: Rgb,
    /// Background of the fold margin, right of the line numbers.
    pub fold_background: Rgb,
    /// The `+`/`-` box drawn on a foldable line.
    pub fold_marker: Rgb,
    /// The dot marking a bookmarked line.
    pub bookmark: Rgb,
    /// Background of the menu bar and its dropdowns.
    pub menu_background: Rgb,
    /// Background of the toolbar.
    pub toolbar_background: Rgb,
    /// The hairline under the toolbar and its group separators.
    pub toolbar_border: Rgb,
    /// Background of a hovered toolbar button.
    pub toolbar_hover: Rgb,
    /// A toolbar button's label.
    pub toolbar_text: Rgb,
    /// The body of a toolbar icon.
    ///
    /// Separate from `toolbar_text` because an icon is a drawing, not a glyph:
    /// it needs a little more contrast than a label to read at the same size,
    /// and it is the one place the two-tone scheme below is anchored.
    pub icon_ink: Rgb,
    /// The part of a toolbar icon that carries its meaning.
    ///
    /// Notepad++ draws its icons in two colours -- a grey body and a blue
    /// accent -- and the accent is what tells New from Close at a glance. It is
    /// a fixed hue rather than a theme colour so the icons stay recognisable
    /// across palettes, lightened on a dark background so it does not vibrate.
    pub icon_accent: Rgb,
    /// Menu label colour.
    pub menu_text: Rgb,
    /// A menu row that cannot be picked.
    ///
    /// Dimmer than `menu_text` but still readable: a greyed row is information
    /// -- it says the command exists and why it does not apply -- so it has to
    /// be legible rather than nearly invisible.
    pub menu_text_disabled: Rgb,
    /// Background of a highlighted menu row or title.
    pub menu_hover: Rgb,
    /// Dropdown frame and separator colour.
    pub menu_border: Rgb,
    /// The bar between two panes.
    ///
    /// Its own colour rather than the menu border's: the divider sits directly
    /// against the right pane's gutter, and a bar the same shade as the gutter
    /// reads as part of it rather than as something to grab.
    pub divider: Rgb,
    /// The shortcut column of a dropdown row.
    pub menu_shortcut: Rgb,
    /// A clickable address.
    ///
    /// Its own colour rather than [`Theme::icon_accent`], which happens to be
    /// the same blue today: the accent is chosen to read against a toolbar and
    /// this one has to read against the menu background, and a theme that
    /// retunes one should not silently retune the other.
    pub link: Rgb,
    /// Behind selected text.
    pub selection: Rgb,
    /// The caret itself.
    pub caret: Rgb,
    /// One colour per syntax style, indexed by the constants in
    /// [`nn_core::style`].
    pub syntax: [Rgb; 9],
    /// Behind the find bar.
    pub bar_background: Rgb,
    /// The hairline above the find bar.
    pub bar_border: Rgb,
    /// Labels and the match counter.
    pub bar_label: Rgb,
    /// Behind an enabled toggle.
    pub toggle_on: Rgb,
    /// The text of an enabled toggle.
    pub toggle_on_text: Rgb,
    /// Behind every search match.
    pub match_highlight: Rgb,
    /// Behind the match the caret is on.
    pub match_current: Rgb,
    /// Behind the five styled mark sets, in Search menu order.
    ///
    /// Five distinguishable hues rather than five shades of one: the whole point
    /// of the styles is telling two kinds of hit apart, and two shades of the same
    /// colour are not distinguishable at a glance. They are all dim enough to read
    /// text through, because a mark that hid its own text would be useless.
    pub mark_styles: [Rgb; 5],
    /// Behind every other occurrence of the word under the caret.
    ///
    /// Deliberately cooler and dimmer than `match_highlight`: smart highlighting
    /// is ambient, and it must not read as a search result.
    pub smart_highlight: Rgb,
    /// Behind the tab strip.
    pub tab_bar_background: Rgb,
    /// Behind an inactive tab.
    pub tab_inactive: Rgb,
    /// The bar along the leading edge of the active tab.
    ///
    /// Notepad++ paints this orange in both palettes — `#FAAA3C` when the pane
    /// has focus, `#FAD296` when it does not — and it is the single strongest
    /// cue for which document you are looking at. It is not derived from the
    /// accent colour for that reason: a theme that recoloured it would lose the
    /// one thing that makes a Notepad++ tab bar recognisable.
    pub tab_active_rule: Rgb,
    /// The title of the active tab.
    pub tab_active_text: Rgb,
    /// The title of an inactive tab.
    pub tab_inactive_text: Rgb,
    /// Behind an inactive tab the pointer is over.
    pub tab_hover: Rgb,
    /// The close cross and the pin on a tab.
    pub tab_button: Rgb,
    /// The pencil that marks a document with unsaved edits.
    pub tab_unsaved: Rgb,
    /// Behind a hover tooltip.
    ///
    /// Its own token rather than the menu's, because a tooltip is an annotation
    /// floating over the page rather than another menu: on the light palette it
    /// is deliberately dark, which is what makes it read as a label stuck on top
    /// of the window instead of as a panel that belongs to it.
    pub tooltip_background: Rgb,
    /// The text of a hover tooltip.
    pub tooltip_text: Rgb,
    /// Behind the minimap column.
    pub minimap_background: Rgb,
    /// The box marking the visible window on the minimap.
    pub minimap_viewport: Rgb,
    /// The dots and bars that stand in for spaces and tabs.
    pub whitespace: Rgb,
    /// The vertical rules marking each indent level.
    pub indent_guide: Rgb,
}

impl Theme {
    /// A dark theme in the Visual Studio Code family.
    pub const DARK: Theme = Theme {
        background: Rgb::new(30, 30, 30),
        foreground: Rgb::new(212, 212, 212),
        gutter_background: Rgb::new(37, 37, 38),
        gutter_foreground: Rgb::new(133, 133, 133),
        gutter_active: Rgb::new(198, 198, 198),
        current_line: Rgb::new(40, 40, 40),
        fold_background: Rgb::new(37, 37, 38),
        fold_marker: Rgb::new(120, 120, 120),
        bookmark: Rgb::new(90, 160, 230),
        menu_background: Rgb::new(45, 45, 48),
        toolbar_background: Rgb::new(37, 37, 40),
        toolbar_border: Rgb::new(60, 60, 64),
        toolbar_hover: Rgb::new(62, 62, 68),
        toolbar_text: Rgb::new(200, 200, 204),
        icon_ink: Rgb::new(206, 206, 210),
        icon_accent: Rgb::new(96, 165, 232),
        menu_text: Rgb::new(212, 212, 212),
        menu_text_disabled: Rgb::new(118, 118, 124),
        menu_hover: Rgb::new(62, 62, 68),
        menu_border: Rgb::new(70, 70, 74),
        divider: Rgb::new(110, 110, 118),
        menu_shortcut: Rgb::new(140, 140, 145),
        link: Rgb::new(96, 165, 232),
        selection: Rgb::new(38, 79, 120),
        caret: Rgb::new(174, 175, 173),
        syntax: [
            Rgb::new(212, 212, 212), // default
            Rgb::new(86, 156, 214),  // keyword
            Rgb::new(78, 201, 176),  // type
            Rgb::new(206, 145, 120), // string
            Rgb::new(106, 153, 85),  // comment
            Rgb::new(181, 206, 168), // number
            Rgb::new(220, 220, 170), // function
            Rgb::new(212, 212, 212), // operator
            Rgb::new(197, 134, 192), // preprocessor
        ],
        bar_background: Rgb::new(37, 37, 38),
        bar_border: Rgb::new(60, 60, 60),
        bar_label: Rgb::new(150, 150, 150),
        toggle_on: Rgb::new(14, 99, 156),
        toggle_on_text: Rgb::new(255, 255, 255),
        match_highlight: Rgb::new(70, 60, 20),
        match_current: Rgb::new(120, 90, 20),
        mark_styles: [
            Rgb::new(74, 52, 30),
            Rgb::new(30, 66, 44),
            Rgb::new(44, 40, 78),
            Rgb::new(78, 34, 44),
            Rgb::new(30, 62, 74),
        ],
        smart_highlight: Rgb::new(38, 56, 74),
        tab_bar_background: Rgb::new(24, 24, 24),
        tab_inactive: Rgb::new(45, 45, 45),
        tab_active_rule: Rgb::new(250, 170, 60),
        tab_active_text: Rgb::new(230, 230, 230),
        tab_inactive_text: Rgb::new(150, 150, 150),
        tab_hover: Rgb::new(58, 58, 58),
        tab_button: Rgb::new(190, 190, 194),
        tab_unsaved: Rgb::new(232, 176, 84),
        tooltip_background: Rgb::new(58, 58, 62),
        tooltip_text: Rgb::new(236, 236, 240),
        minimap_background: Rgb::new(26, 26, 26),
        minimap_viewport: Rgb::new(200, 200, 200),
        whitespace: Rgb::new(80, 80, 84),
        indent_guide: Rgb::new(52, 52, 56),
    };

    /// A light theme, for daylight and for projectors.
    ///
    /// The syntax colours are the same hues as the dark theme, darkened until
    /// they carry enough contrast against white. Reusing the hues keeps the
    /// document recognisable when the theme flips.
    pub const LIGHT: Theme = Theme {
        background: Rgb::new(255, 255, 255),
        foreground: Rgb::new(30, 30, 30),
        gutter_background: Rgb::new(240, 240, 240),
        gutter_foreground: Rgb::new(130, 130, 130),
        gutter_active: Rgb::new(40, 40, 40),
        current_line: Rgb::new(245, 245, 245),
        fold_background: Rgb::new(240, 240, 240),
        fold_marker: Rgb::new(120, 120, 120),
        bookmark: Rgb::new(30, 110, 200),
        menu_background: Rgb::new(245, 245, 245),
        toolbar_background: Rgb::new(238, 238, 238),
        toolbar_border: Rgb::new(205, 205, 205),
        toolbar_hover: Rgb::new(220, 230, 245),
        toolbar_text: Rgb::new(50, 50, 50),
        icon_ink: Rgb::new(64, 64, 68),
        icon_accent: Rgb::new(28, 108, 196),
        menu_text: Rgb::new(30, 30, 30),
        menu_text_disabled: Rgb::new(150, 150, 156),
        menu_hover: Rgb::new(220, 230, 245),
        menu_border: Rgb::new(200, 200, 200),
        divider: Rgb::new(150, 150, 156),
        menu_shortcut: Rgb::new(120, 120, 120),
        link: Rgb::new(28, 108, 196),
        selection: Rgb::new(180, 210, 245),
        caret: Rgb::new(30, 30, 30),
        syntax: [
            Rgb::new(30, 30, 30),    // default
            Rgb::new(0, 0, 200),     // keyword
            Rgb::new(38, 127, 110),  // type
            Rgb::new(163, 21, 21),   // string
            Rgb::new(0, 128, 0),     // comment
            Rgb::new(9, 134, 88),    // number
            Rgb::new(121, 94, 38),   // function
            Rgb::new(60, 60, 60),    // operator
            Rgb::new(128, 0, 128),   // preprocessor
        ],
        bar_background: Rgb::new(240, 240, 240),
        bar_border: Rgb::new(200, 200, 200),
        bar_label: Rgb::new(100, 100, 100),
        toggle_on: Rgb::new(0, 120, 215),
        toggle_on_text: Rgb::new(255, 255, 255),
        match_highlight: Rgb::new(255, 240, 160),
        match_current: Rgb::new(255, 200, 80),
        mark_styles: [
            Rgb::new(255, 226, 178),
            Rgb::new(198, 240, 198),
            Rgb::new(214, 208, 255),
            Rgb::new(255, 200, 214),
            Rgb::new(190, 232, 240),
        ],
        smart_highlight: Rgb::new(214, 232, 248),
        tab_bar_background: Rgb::new(228, 228, 228),
        tab_inactive: Rgb::new(240, 240, 240),
        tab_active_rule: Rgb::new(250, 170, 60),
        tab_active_text: Rgb::new(20, 20, 20),
        tab_inactive_text: Rgb::new(110, 110, 110),
        tab_hover: Rgb::new(232, 232, 232),
        tab_button: Rgb::new(70, 70, 74),
        tab_unsaved: Rgb::new(196, 96, 40),
        tooltip_background: Rgb::new(48, 48, 52),
        tooltip_text: Rgb::new(245, 245, 245),
        minimap_background: Rgb::new(248, 248, 248),
        minimap_viewport: Rgb::new(120, 120, 120),
        whitespace: Rgb::new(175, 175, 175),
        indent_guide: Rgb::new(224, 224, 224),
    };

    /// Builds a theme from a palette.
    ///
    /// Sixty-odd colours are more than anyone can pick well, and a constant per
    /// theme would be sixty lines of near-identical hex that drift apart at the
    /// first edit. So a theme in the catalogue is authored as the colours a
    /// designer actually chooses -- background, text, gutter, selection, caret,
    /// one accent and the nine syntax styles -- and the chrome is derived here.
    ///
    /// The derivation is deliberately boring: every neutral is the foreground
    /// mixed `t` of the way into the background, so a light theme darkens as
    /// `t` grows and a dark theme lightens, from one expression. The cost is
    /// that a palette cannot have, say, a green menu bar; the gain is that
    /// adding a theme is a fourteen-line table and not a directory.
    ///
    /// Three colours are not derived. The orange rule on the active tab and the
    /// pencil on an unsaved one are Notepad++'s own, and the tab bar is the one
    /// place a user recognises the editor by its colours; they stay whatever the
    /// palette says.
    pub fn from_palette(palette: &crate::catalogue::Palette) -> Theme {
        let background = palette.background;
        let foreground = palette.foreground;
        // A neutral: `t` of the way from the background towards the text.
        let wash = |t: f32| mix(background, foreground, t);
        // A hue, muted onto the background. The mark colours and the search
        // highlights are the only places a theme shows its own hue in the
        // chrome, and both have to stay readable through.
        let tint = |hue: Rgb, t: f32| mix(background, hue, t);
        let (tooltip_background, tooltip_text) = if palette.light {
            (wash(0.82), Rgb::new(255, 255, 255))
        } else {
            (wash(0.18), wash(0.94))
        };
        let highlight = if palette.light { 0.34 } else { 0.28 };
        Theme {
            background,
            foreground,
            gutter_background: palette.gutter,
            gutter_foreground: wash(0.55),
            gutter_active: wash(0.9),
            current_line: wash(0.05),
            fold_background: palette.gutter,
            fold_marker: wash(0.5),
            bookmark: palette.accent,
            menu_background: wash(0.07),
            toolbar_background: wash(0.04),
            toolbar_border: wash(0.14),
            toolbar_hover: wash(0.16),
            toolbar_text: wash(0.84),
            icon_ink: wash(0.88),
            icon_accent: palette.accent,
            menu_text: foreground,
            menu_text_disabled: wash(0.42),
            menu_hover: wash(0.16),
            menu_border: wash(0.16),
            divider: wash(0.25),
            menu_shortcut: wash(0.52),
            link: palette.accent,
            selection: palette.selection,
            caret: palette.caret,
            syntax: palette.syntax,
            bar_background: wash(0.06),
            bar_border: wash(0.14),
            bar_label: wash(0.58),
            toggle_on: palette.accent,
            toggle_on_text: if palette.light {
                Rgb::new(255, 255, 255)
            } else {
                mix(background, Rgb::new(0, 0, 0), 0.75)
            },
            match_highlight: tint(Rgb::new(220, 180, 60), highlight),
            match_current: tint(Rgb::new(220, 180, 60), highlight + 0.18),
            mark_styles: [
                tint(Rgb::new(204, 130, 60), 0.35),
                tint(Rgb::new(60, 160, 90), 0.35),
                tint(Rgb::new(120, 110, 200), 0.35),
                tint(Rgb::new(200, 80, 90), 0.35),
                tint(Rgb::new(60, 150, 170), 0.35),
            ],
            smart_highlight: wash(0.1),
            tab_bar_background: mix(background, Rgb::new(0, 0, 0), 0.1),
            tab_inactive: wash(0.07),
            tab_active_rule: Rgb::new(250, 170, 60),
            tab_active_text: foreground,
            tab_inactive_text: wash(0.58),
            tab_hover: wash(0.12),
            tab_button: wash(0.8),
            tab_unsaved: if palette.light {
                Rgb::new(196, 96, 40)
            } else {
                Rgb::new(232, 176, 84)
            },
            tooltip_background,
            tooltip_text,
            minimap_background: mix(background, Rgb::new(0, 0, 0), 0.03),
            minimap_viewport: wash(0.7),
            whitespace: wash(0.3),
            indent_guide: wash(0.12),
        }
    }
}

/// Reads a `#RRGGBB` or `RRGGBB` colour.
fn parse_hex(text: &str) -> Option<Rgb> {
    let text = text.trim().trim_start_matches('#');
    if text.len() != 6 || !text.is_ascii() {
        return None;
    }
    let byte = |at: usize| u8::from_str_radix(&text[at..at + 2], 16).ok();
    Some(Rgb::new(byte(0)?, byte(2)?, byte(4)?))
}

/// `c` moved `amount` of the way towards white (positive) or black (negative).
fn shade(c: Rgb, amount: f32) -> Rgb {
    if amount >= 0.0 {
        mix(c, Rgb::new(255, 255, 255), amount)
    } else {
        mix(c, Rgb::new(0, 0, 0), -amount)
    }
}

/// Builds a theme from a Notepad++ `stylers.xml`.
///
/// Notepad++'s theme file names the colours of its own widgets and of each
/// lexer's token classes. Only the parts this editor draws are read: the global
/// styles, and the `c` lexer's nine token classes, which are the nine styles
/// every built-in language here emits. A theme that colours `python` differently
/// from `c` is not honoured, because this editor has one palette per document
/// rather than one per language.
///
/// Anything the file does not name keeps the value from the base theme, which is
/// chosen by the background's brightness. A theme file is usually complete, but
/// a hand-edited one is not, and a missing colour should leave a readable editor
/// rather than a black rectangle.
///
/// The chrome -- menus, tabs, toolbar -- has no counterpart in the file, so it is
/// derived from the two colours that do: the background and the foreground. That
/// is what makes an imported theme look like one theme rather than a text area
/// dropped into someone else's window.
pub fn from_stylers(xml: &str) -> Option<Theme> {
    // The global styles first, because the background decides which base theme
    // the rest of the file is filled in against.
    let mut widgets: Vec<(&str, Option<Rgb>, Option<Rgb>)> = Vec::new();
    for element in nn_core::xml::elements(xml, "WidgetStyle") {
        let Some(name) = nn_core::xml::attribute(&element, "name") else {
            continue;
        };
        let fg = nn_core::xml::attribute(&element, "fgColor").and_then(parse_hex);
        let bg = nn_core::xml::attribute(&element, "bgColor").and_then(parse_hex);
        widgets.push((name, fg, bg));
    }
    let find = |name: &str| widgets.iter().find(|(n, _, _)| *n == name).copied();

    let (_, global_fg, global_bg) = find("GlobalStyles")?;
    let background = global_bg?;
    let foreground = global_fg?;

    let mut theme = if background.luminance() < 128 {
        Theme::DARK
    } else {
        Theme::LIGHT
    };
    theme.background = background;
    theme.foreground = foreground;

    // The widgets the file names, in the order they are read.
    if let Some((_, fg, bg)) = find("Current line background colour") {
        theme.current_line = bg.or(fg).unwrap_or(theme.current_line);
    }
    if let Some((_, fg, bg)) = find("Selected text colour") {
        theme.selection = bg.or(fg).unwrap_or(theme.selection);
    }
    if let Some((_, fg, _)) = find("Caret colour") {
        theme.caret = fg.unwrap_or(theme.caret);
    }
    if let Some((_, fg, bg)) = find("Line number margin") {
        theme.gutter_background = bg.unwrap_or(theme.gutter_background);
        theme.gutter_foreground = fg.unwrap_or(theme.gutter_foreground);
    }
    if let Some((_, fg, bg)) = find("Fold").or_else(|| find("Fold margin")) {
        theme.fold_background = bg.unwrap_or(theme.fold_background);
        theme.fold_marker = fg.unwrap_or(theme.fold_marker);
    }
    if let Some((_, fg, _)) = find("Bookmark margin") {
        theme.bookmark = fg.unwrap_or(theme.bookmark);
    }
    if let Some((_, fg, _)) = find("Indent guideline style") {
        theme.indent_guide = fg.unwrap_or(theme.indent_guide);
    }
    if let Some((_, fg, _)) = find("White space symbol") {
        theme.whitespace = fg.unwrap_or(theme.whitespace);
    }
    if let Some((_, _, bg)) = find("Smart HighLighting") {
        theme.smart_highlight = bg.unwrap_or(theme.smart_highlight);
    }
    if let Some((_, _, bg)) = find("Tags match highlighting") {
        theme.match_highlight = bg.unwrap_or(theme.match_highlight);
    }
    for (index, name) in [
        "Mark Style 1",
        "Mark Style 2",
        "Mark Style 3",
        "Mark Style 4",
        "Mark Style 5",
    ]
    .iter()
    .enumerate()
    {
        if let Some((_, _, bg)) = find(name) {
            theme.mark_styles[index] = bg.unwrap_or(theme.mark_styles[index]);
        }
    }

    // The token classes, from the `c` lexer. Notepad++ names them in capitals;
    // the mapping to this editor's nine styles is one-to-one.
    let lexer = nn_core::xml::elements(xml, "LexerType")
        .into_iter()
        .find(|element| nn_core::xml::attribute(element, "name") == Some("c"));
    if let Some(lexer) = lexer {
        for element in nn_core::xml::elements(lexer.body, "WordsStyle") {
            let Some(name) = nn_core::xml::attribute(&element, "name") else {
                continue;
            };
            let Some(fg) = nn_core::xml::attribute(&element, "fgColor").and_then(parse_hex) else {
                continue;
            };
            let slot = match name {
                "DEFAULT" => 0,
                "INSTRUCTION WORD" => 1,
                "TYPE WORD" => 2,
                "STRING" => 3,
                "COMMENT" => 4,
                "NUMBER" => 5,
                "FUNCTION" => 6,
                "OPERATOR" => 7,
                "PREPROCESSOR" => 8,
                _ => continue,
            };
            theme.syntax[slot] = fg;
        }
    }

    // The chrome, derived from the two colours the file does name.
    let raised = if background.luminance() < 128 {
        shade(background, 0.06)
    } else {
        shade(background, -0.04)
    };
    let sunken = if background.luminance() < 128 {
        shade(background, -0.03)
    } else {
        shade(background, -0.02)
    };
    let rule = mix(background, foreground, 0.18);
    let dim = mix(foreground, background, 0.35);
    let accent = Rgb::new(14, 99, 156);
    theme.menu_background = raised;
    theme.menu_text = foreground;
    theme.menu_text_disabled = mix(foreground, background, 0.45);
    theme.menu_hover = mix(raised, foreground, 0.12);
    theme.menu_border = rule;
    theme.menu_shortcut = dim;
    theme.toolbar_background = raised;
    theme.toolbar_border = rule;
    theme.toolbar_hover = theme.menu_hover;
    theme.toolbar_text = foreground;
    theme.divider = rule;
    theme.bar_background = raised;
    theme.bar_border = rule;
    theme.bar_label = dim;
    theme.toggle_on = accent;
    theme.toggle_on_text = Rgb::new(255, 255, 255);
    theme.match_current = shade(theme.match_highlight, 0.12);
    theme.tab_bar_background = sunken;
    theme.tab_inactive = raised;
    // The orange stays: it is the tab bar's identity, not a theme colour.
    theme.tab_active_text = foreground;
    theme.tab_inactive_text = dim;
    theme.tab_hover = mix(raised, foreground, 0.08);
    theme.tab_button = mix(foreground, background, 0.25);
    theme.tab_unsaved = mix(accent, Rgb::new(250, 170, 60), 0.5);
    theme.minimap_background = sunken;
    theme.minimap_viewport = mix(foreground, background, 0.5);
    theme.gutter_active = foreground;
    Some(theme)
}

impl Theme {
    /// The colour for a syntax style.
    ///
    /// Unknown styles fall back to the default text colour, so a lexer that
    /// invents a style index cannot make text invisible.
    pub fn style_color(&self, style: nn_core::Style) -> Rgb {
        self.syntax
            .get(style as usize)
            .copied()
            .unwrap_or(self.foreground)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::DARK
    }
}

#[cfg(test)]
mod theme_tests {
    use super::*;

    /// A `stylers.xml` with the elements the importer reads.
    const STYLERS: &str = r#"<?xml version="1.0" encoding="UTF-8" ?>
<NotepadPlus>
    <GlobalStyles>
        <WidgetStyle name="GlobalStyles" fgColor="C0C0C0" bgColor="101820" />
        <WidgetStyle name="Current line background colour" bgColor="1A2430" />
        <WidgetStyle name="Selected text colour" bgColor="264F78" />
        <WidgetStyle name="Caret colour" fgColor="FFCC00" />
        <WidgetStyle name="Line number margin" fgColor="808080" bgColor="0C1218" />
        <WidgetStyle name="Mark Style 1" bgColor="402010" />
    </GlobalStyles>
    <LexerStyles>
        <LexerType name="c" desc="C" ext="">
            <WordsStyle name="DEFAULT" fgColor="C0C0C0" bgColor="101820" />
            <WordsStyle name="COMMENT" fgColor="5F8F5F" bgColor="101820" />
            <WordsStyle name="KEYWORD" fgColor="000000" bgColor="101820" />
            <WordsStyle name="INSTRUCTION WORD" fgColor="569CD6" bgColor="101820" />
            <WordsStyle name="STRING" fgColor="CE9178" bgColor="101820" />
        </LexerType>
        <LexerType name="python" desc="Python" ext="py">
            <WordsStyle name="INSTRUCTION WORD" fgColor="FF0000" bgColor="101820" />
        </LexerType>
    </LexerStyles>
</NotepadPlus>"#;

    #[test]
    fn a_theme_file_supplies_the_colours_it_names() {
        let theme = from_stylers(STYLERS).expect("the file has a GlobalStyles element");
        assert_eq!(theme.background, Rgb::new(0x10, 0x18, 0x20));
        assert_eq!(theme.foreground, Rgb::new(0xC0, 0xC0, 0xC0));
        assert_eq!(theme.current_line, Rgb::new(0x1A, 0x24, 0x30));
        assert_eq!(theme.selection, Rgb::new(0x26, 0x4F, 0x78));
        assert_eq!(theme.caret, Rgb::new(0xFF, 0xCC, 0x00));
        assert_eq!(theme.gutter_background, Rgb::new(0x0C, 0x12, 0x18));
        assert_eq!(theme.mark_styles[0], Rgb::new(0x40, 0x20, 0x10));
    }

    #[test]
    fn the_token_classes_come_from_the_c_lexer_only() {
        // The file colours `python` differently from `c`. This editor has one
        // palette per document, so the `c` lexer is the one that is read and the
        // `python` block is ignored rather than merged.
        let theme = from_stylers(STYLERS).unwrap();
        assert_eq!(theme.syntax[1], Rgb::new(0x56, 0x9C, 0xD6));
        assert_eq!(theme.syntax[3], Rgb::new(0xCE, 0x91, 0x78));
        assert_eq!(theme.syntax[4], Rgb::new(0x5F, 0x8F, 0x5F));
        assert_ne!(theme.syntax[1], Rgb::new(0xFF, 0, 0));
    }

    #[test]
    fn a_colour_the_file_does_not_name_keeps_the_base_theme() {
        let theme = from_stylers(STYLERS).unwrap();
        // `NUMBER` is not in the file, so it keeps the dark theme's value.
        assert_eq!(theme.syntax[5], Theme::DARK.syntax[5]);
        // A dark background picks the dark base.
        assert_eq!(theme.syntax[2], Theme::DARK.syntax[2]);
    }

    #[test]
    fn a_light_background_picks_the_light_base() {
        let text = STYLERS.replace("bgColor=\"101820\"", "bgColor=\"FFFFFF\"");
        let theme = from_stylers(&text).unwrap();
        assert_eq!(theme.syntax[5], Theme::LIGHT.syntax[5]);
    }

    #[test]
    fn a_file_without_global_styles_is_refused() {
        // Half a theme is worse than none: the user would have no way to tell
        // which colours came from the file and which from the built-in theme.
        assert_eq!(from_stylers("<NotepadPlus></NotepadPlus>"), None);
        assert_eq!(from_stylers(""), None);
    }

    #[test]
    fn the_chrome_is_derived_from_the_two_colours_the_file_names() {
        let theme = from_stylers(STYLERS).unwrap();
        // The menu is a shade off the background rather than the background
        // itself, so a dropdown reads as a surface above the text.
        assert_ne!(theme.menu_background, theme.background);
        assert_eq!(theme.menu_text, theme.foreground);
        assert_eq!(theme.toolbar_text, theme.foreground);
        // A disabled row is dimmer than an enabled one but still legible.
        assert_ne!(theme.menu_text_disabled, theme.menu_text);
        assert!(theme.menu_text_disabled.luminance() > 40);
    }

    #[test]
    fn a_hex_colour_is_read_with_or_without_its_hash() {
        assert_eq!(parse_hex("#FF8000"), Some(Rgb::new(255, 128, 0)));
        assert_eq!(parse_hex("ff8000"), Some(Rgb::new(255, 128, 0)));
        assert_eq!(parse_hex("FF80"), None);
        assert_eq!(parse_hex("GGGGGG"), None);
        assert_eq!(parse_hex(""), None);
    }
}

/// One colour the configurator can change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Default,
    Keyword,
    Type,
    String,
    Comment,
    Number,
    Function,
    Operator,
    Preprocessor,
    Background,
    Gutter,
    Foreground,
    Selection,
    Caret,
    CurrentLine,
    LineNumber,
}

impl Slot {
    /// Every slot, in the order the dialog lists them.
    pub const ALL: [Slot; 16] = [
        Slot::Default,
        Slot::Keyword,
        Slot::Type,
        Slot::String,
        Slot::Comment,
        Slot::Number,
        Slot::Function,
        Slot::Operator,
        Slot::Preprocessor,
        Slot::Background,
        Slot::Gutter,
        Slot::Foreground,
        Slot::Selection,
        Slot::Caret,
        Slot::CurrentLine,
        Slot::LineNumber,
    ];

    /// The label drawn on the left.
    pub fn label(self) -> &'static str {
        match self {
            Slot::Default => "Default text",
            Slot::Keyword => "Keyword",
            Slot::Type => "Type",
            Slot::String => "String",
            Slot::Comment => "Comment",
            Slot::Number => "Number",
            Slot::Function => "Function",
            Slot::Operator => "Operator",
            Slot::Preprocessor => "Preprocessor",
            Slot::Background => "Background",
            Slot::Gutter => "Gutter",
            Slot::Foreground => "Text",
            Slot::Selection => "Selection",
            Slot::Caret => "Caret",
            Slot::CurrentLine => "Current line",
            Slot::LineNumber => "Line number",
        }
    }

    /// The keyword this slot is written as in the settings file.
    ///
    /// Stable: a user's `style` lines have to keep meaning the same thing across
    /// versions, so these are not derived from the label.
    pub fn key(self) -> &'static str {
        match self {
            Slot::Default => "default",
            Slot::Keyword => "keyword",
            Slot::Type => "type",
            Slot::String => "string",
            Slot::Comment => "comment",
            Slot::Number => "number",
            Slot::Function => "function",
            Slot::Operator => "operator",
            Slot::Preprocessor => "preprocessor",
            Slot::Background => "background",
            Slot::Gutter => "gutter",
            Slot::Foreground => "foreground",
            Slot::Selection => "selection",
            Slot::Caret => "caret",
            Slot::CurrentLine => "current_line",
            Slot::LineNumber => "line_number",
        }
    }

    /// Reads a key written by [`Slot::key`].
    pub fn from_key(key: &str) -> Option<Slot> {
        Slot::ALL.into_iter().find(|slot| slot.key() == key)
    }

    /// The colour this slot currently holds in `theme`.
    pub fn get(self, theme: &Theme) -> Rgb {
        match self {
            Slot::Default => theme.syntax[0],
            Slot::Keyword => theme.syntax[1],
            Slot::Type => theme.syntax[2],
            Slot::String => theme.syntax[3],
            Slot::Comment => theme.syntax[4],
            Slot::Number => theme.syntax[5],
            Slot::Function => theme.syntax[6],
            Slot::Operator => theme.syntax[7],
            Slot::Preprocessor => theme.syntax[8],
            Slot::Background => theme.background,
            Slot::Gutter => theme.gutter_background,
            Slot::Foreground => theme.foreground,
            Slot::Selection => theme.selection,
            Slot::Caret => theme.caret,
            Slot::CurrentLine => theme.current_line,
            Slot::LineNumber => theme.gutter_foreground,
        }
    }

    /// Writes `color` into this slot of `theme`.
    pub fn set(self, theme: &mut Theme, color: Rgb) {
        match self {
            Slot::Default => theme.syntax[0] = color,
            Slot::Keyword => theme.syntax[1] = color,
            Slot::Type => theme.syntax[2] = color,
            Slot::String => theme.syntax[3] = color,
            Slot::Comment => theme.syntax[4] = color,
            Slot::Number => theme.syntax[5] = color,
            Slot::Function => theme.syntax[6] = color,
            Slot::Operator => theme.syntax[7] = color,
            Slot::Preprocessor => theme.syntax[8] = color,
            Slot::Background => theme.background = color,
            Slot::Gutter => theme.gutter_background = color,
            Slot::Foreground => theme.foreground = color,
            Slot::Selection => theme.selection = color,
            Slot::Caret => theme.caret = color,
            Slot::CurrentLine => theme.current_line = color,
            Slot::LineNumber => theme.gutter_foreground = color,
        }
    }
}

/// Records `slot`'s colour in `overrides`, replacing any earlier entry.
///
/// Kept sorted in dialog order so the settings file is stable and diffable, and
/// so a slot can never appear twice — the last write wins, which is what a user
/// stepping a colour expects.
pub fn record(overrides: &mut Vec<(Slot, Rgb)>, slot: Slot, color: Rgb) {
    overrides.retain(|(s, _)| *s != slot);
    overrides.push((slot, color));
    overrides.sort_by_key(|(s, _)| Slot::ALL.iter().position(|x| x == s).unwrap_or(0));
}

/// The colours the style configurator offers, in the order it steps them.
///
/// Twenty-four, chosen to cover what a syntax theme needs: neutrals for text and
/// backgrounds, and a saturated and a muted version of each hue for keywords,
/// strings, and comments. It lives here rather than in either front-end because
/// both dialogs offer it: a window user stepping a colour and a terminal user
/// stepping the same colour have to land on the same twenty-four, in the same
/// order, or the two dialogs disagree about what a colour is.
pub const PALETTE: [Rgb; 24] = [
    Rgb::new(0, 0, 0),
    Rgb::new(30, 30, 30),
    Rgb::new(60, 60, 60),
    Rgb::new(100, 100, 100),
    Rgb::new(140, 140, 140),
    Rgb::new(180, 180, 180),
    Rgb::new(212, 212, 212),
    Rgb::new(255, 255, 255),
    Rgb::new(163, 21, 21),
    Rgb::new(206, 145, 120),
    Rgb::new(214, 120, 40),
    Rgb::new(220, 180, 60),
    Rgb::new(181, 206, 168),
    Rgb::new(106, 153, 85),
    Rgb::new(0, 128, 0),
    Rgb::new(78, 201, 176),
    Rgb::new(38, 127, 110),
    Rgb::new(86, 156, 214),
    Rgb::new(0, 0, 200),
    Rgb::new(38, 79, 120),
    Rgb::new(197, 134, 192),
    Rgb::new(128, 0, 128),
    Rgb::new(220, 220, 170),
    Rgb::new(255, 240, 160),
];

/// The colour `slot` steps to in `theme`, or `None` when it is already at the
/// end of the palette.
///
/// A step is measured from where the slot currently sits, not from an index the
/// dialog remembers: the theme's own colour is usually in the palette, and when
/// it is not -- a hand-written `style` line, or a theme colour the palette does
/// not carry -- the step goes to the nearer end rather than jumping to the
/// middle. Stepping past either end stays put, so a held arrow does not wrap a
/// colour round to the other side of the palette under the user's finger.
pub fn step(slot: Slot, theme: &Theme, delta: i32) -> Option<Rgb> {
    let current = slot.get(theme);
    let next = match PALETTE.iter().position(|&color| color == current) {
        Some(at) => {
            let last = PALETTE.len() as isize - 1;
            (at as isize + delta as isize).clamp(0, last) as usize
        }
        None if delta >= 0 => 0,
        None => PALETTE.len() - 1,
    };
    let color = PALETTE[next];
    (color != current).then_some(color)
}

/// Builds the theme the editor should draw with.
///
/// The base palette is chosen first and the user's overrides go on top, so
/// switching to another one of the twenty-one themes keeps their edits instead
/// of discarding them. That is the whole reason the overrides are stored
/// sparsely: they are a diff against a theme, not a theme.
pub fn effective_theme(base: Theme, overrides: &[(Slot, Rgb)]) -> Theme {
    let mut theme = base;
    for (slot, color) in overrides {
        slot.set(&mut theme, *color);
    }
    theme
}

#[cfg(test)]
mod slot_tests {
    use super::*;

    #[test]
    fn every_slot_reads_back_from_its_key() {
        for slot in Slot::ALL {
            assert_eq!(Slot::from_key(slot.key()), Some(slot), "{}", slot.key());
        }
    }

    #[test]
    fn the_keys_are_unique() {
        let mut seen: Vec<&str> = Vec::new();
        for slot in Slot::ALL {
            assert!(!seen.contains(&slot.key()), "{} twice", slot.key());
            seen.push(slot.key());
        }
    }

    #[test]
    fn every_slot_has_a_label() {
        for slot in Slot::ALL {
            assert!(!slot.label().is_empty());
        }
    }

    #[test]
    fn setting_a_slot_changes_only_that_slot() {
        let mut theme = Theme::DARK;
        let before = theme;
        Slot::Keyword.set(&mut theme, Rgb::new(1, 2, 3));
        assert_eq!(Slot::Keyword.get(&theme), Rgb::new(1, 2, 3));
        assert_eq!(theme.background, before.background);
        assert_eq!(theme.syntax[3], before.syntax[3]);
    }

    #[test]
    fn every_slot_round_trips_through_the_theme() {
        for slot in Slot::ALL {
            let mut theme = Theme::DARK;
            slot.set(&mut theme, Rgb::new(9, 8, 7));
            assert_eq!(slot.get(&theme), Rgb::new(9, 8, 7), "{}", slot.key());
        }
    }

    #[test]
    fn recording_a_slot_replaces_the_earlier_value() {
        let mut overrides = Vec::new();
        record(&mut overrides, Slot::Caret, Rgb::new(1, 1, 1));
        record(&mut overrides, Slot::Caret, Rgb::new(2, 2, 2));
        assert_eq!(overrides, vec![(Slot::Caret, Rgb::new(2, 2, 2))]);
    }

    #[test]
    fn recording_keeps_the_list_in_dialog_order() {
        let mut overrides = Vec::new();
        record(&mut overrides, Slot::LineNumber, Rgb::new(1, 1, 1));
        record(&mut overrides, Slot::Keyword, Rgb::new(2, 2, 2));
        record(&mut overrides, Slot::Background, Rgb::new(3, 3, 3));
        assert_eq!(
            overrides.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
            vec![Slot::Keyword, Slot::Background, Slot::LineNumber]
        );
    }

    #[test]
    fn overrides_apply_on_top_of_each_base() {
        let overrides = [(Slot::Keyword, Rgb::new(1, 2, 3))];
        let mut bases = vec![Theme::DARK, Theme::LIGHT];
        bases.extend(crate::catalogue::CATALOGUE.iter().map(Theme::from_palette));
        for base in bases {
            let theme = effective_theme(base, &overrides);
            assert_eq!(Slot::Keyword.get(&theme), Rgb::new(1, 2, 3));
            assert_eq!(theme.background, base.background);
        }
    }

    #[test]
    fn a_step_walks_the_palette_from_where_the_colour_sits() {
        // The dark theme's keyword colour is itself a palette entry, so the walk
        // is measured from wherever it sits rather than assumed to start at 0.
        let at = PALETTE
            .iter()
            .position(|&color| color == Slot::Keyword.get(&Theme::DARK))
            .expect("the dark keyword colour is in the palette");
        assert_eq!(
            step(Slot::Keyword, &Theme::DARK, 1),
            Some(PALETTE[at + 1])
        );
        assert_eq!(
            step(Slot::Keyword, &Theme::DARK, -1),
            Some(PALETTE[at - 1])
        );
    }

    #[test]
    fn a_step_past_either_end_stays_put() {
        let mut theme = Theme::DARK;
        Slot::Keyword.set(&mut theme, PALETTE[0]);
        assert_eq!(step(Slot::Keyword, &theme, -1), None);
        Slot::Keyword.set(&mut theme, PALETTE[PALETTE.len() - 1]);
        assert_eq!(step(Slot::Keyword, &theme, 1), None);
    }

    #[test]
    fn a_colour_outside_the_palette_steps_to_an_end() {
        let mut theme = Theme::DARK;
        Slot::Keyword.set(&mut theme, Rgb::new(1, 1, 1));
        assert_eq!(step(Slot::Keyword, &theme, 1), Some(PALETTE[0]));
        assert_eq!(
            step(Slot::Keyword, &theme, -1),
            Some(PALETTE[PALETTE.len() - 1])
        );
    }

    #[test]
    fn every_slot_can_be_stepped_from_the_dark_theme() {
        for slot in Slot::ALL {
            let next = step(slot, &Theme::DARK, 1);
            assert!(next.is_some(), "{} does not step", slot.key());
            // And the answer is a palette colour, not the one it came from.
            let next = next.expect("a step");
            assert!(PALETTE.contains(&next), "{}", slot.key());
            assert_ne!(next, slot.get(&Theme::DARK), "{}", slot.key());
        }
    }

    #[test]
    fn no_overrides_is_the_base_theme() {
        assert_eq!(
            effective_theme(Theme::DARK, &[]).background,
            Theme::DARK.background
        );
        assert_eq!(
            effective_theme(Theme::LIGHT, &[]).background,
            Theme::LIGHT.background
        );
    }
}

/// The kitten's colours.
///
/// Fixed rather than taken from the theme: a cat is ginger whatever the syntax
/// highlighting is doing. Both front-ends draw her from these five -- the window
/// as lit geometry, the terminal as one cell per character -- so a shade cannot
/// be one colour on a GPU and another in a cell. They are adjusted for the
/// background by [`readable`], which is what keeps a ginger cat from being a
/// pale smudge on a paper-white theme.
const FUR: Rgb = Rgb::new(228, 172, 112);
const FUR_DARK: Rgb = Rgb::new(190, 134, 78);
const FUR_LIGHT: Rgb = Rgb::new(246, 236, 214);
const NOSE: Rgb = Rgb::new(226, 136, 148);
const EYE: Rgb = Rgb::new(40, 36, 46);

/// A colour moved away from the theme's background until it reads.
///
/// A ginger kitten on a paper-white theme is a pale smudge, and a cream chest on
/// a near-black one is a bright smear. One rule covers both: darken on a light
/// background, lighten on a dark one, by enough to see and not enough to look
/// like a different colour.
pub fn readable(colour: Rgb, theme: &Theme) -> Rgb {
    let background = theme.background;
    let luma = 0.299 * background.0 as f32
        + 0.587 * background.1 as f32
        + 0.114 * background.2 as f32;
    if luma > 140.0 {
        mix(colour, Rgb::new(0, 0, 0), 0.24)
    } else {
        mix(colour, Rgb::new(255, 255, 255), 0.06)
    }
}

/// The colour a [`nn_core::playground::Shade`] plays on `theme`.
pub fn shade_colour(shade: nn_core::playground::Shade, theme: &Theme) -> Rgb {
    readable(
        match shade {
            nn_core::playground::Shade::Fur => FUR,
            nn_core::playground::Shade::FurDark => FUR_DARK,
            nn_core::playground::Shade::FurLight => FUR_LIGHT,
            nn_core::playground::Shade::Nose => NOSE,
            nn_core::playground::Shade::Eye => EYE,
        },
        theme,
    )
}

#[cfg(test)]
mod kitten_tests {
    use super::*;
    use nn_core::playground::Shade;

    #[test]
    fn a_ginger_cat_is_darkened_on_a_light_theme_and_left_alone_on_a_dark_one() {
        let paper = Theme {
            background: Rgb::new(255, 255, 255),
            ..Theme::default()
        };
        let night = Theme {
            background: Rgb::new(18, 18, 22),
            ..Theme::default()
        };
        let on_paper = readable(FUR, &paper);
        let on_night = readable(FUR, &night);
        assert!(on_paper.0 < FUR.0, "{on_paper:?}");
        assert!(on_night.0 >= FUR.0, "{on_night:?}");
        assert!(on_paper.1 < on_night.1, "{on_paper:?} {on_night:?}");
    }

    #[test]
    fn every_shade_has_a_colour_and_no_two_are_the_same() {
        let theme = Theme::default();
        let shades = [
            Shade::Fur,
            Shade::FurDark,
            Shade::FurLight,
            Shade::Nose,
            Shade::Eye,
        ];
        let colours: Vec<Rgb> = shades
            .iter()
            .map(|shade| shade_colour(*shade, &theme))
            .collect();
        for (index, colour) in colours.iter().enumerate() {
            assert!(
                !colours[index + 1..].contains(colour),
                "{:?} is the colour of another shade",
                shades[index]
            );
        }
    }
}
