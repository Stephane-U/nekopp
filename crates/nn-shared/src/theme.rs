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
//! and the keyword each is stored as.
//!
//! The colours are not here. A theme's identity -- the variant, the label, the
//! word in the settings file, the order the preference row steps through -- is
//! what both front-ends have to agree about, and a terminal that offered a
//! different list of themes from the window would be a second editor. The
//! colours are what only the front-end with a screen can do anything with, so
//! they stay there, in `themes::CATALOGUE`, aligned by position with
//! [`ThemeChoice::NAMES`]: entry `i` of the catalogue is the colours of
//! `NAMES[i]`, and a test in the windowed crate holds the two lists together.
//!
//! Three choices are not in the catalogue: `System` follows the desktop's
//! colour-scheme preference and solves to one of the two built-in palettes, and
//! `Light` and `Dark` are those palettes by name. They are choices because the
//! settings file has to record which of the three the user meant -- a user whose
//! desktop goes dark at sunset expects the editor to follow, and a user who
//! picked a named theme expects it to stay put whatever the desktop does.

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
