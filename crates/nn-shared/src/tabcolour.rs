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

//! The five colours a user can paint a tab with.

use crate::color::Rgb;

/// A colour the user can paint a tab with.
///
/// Fixed rather than themeable: the point of colouring a tab is to recognise it
/// at a glance, and a colour that changed with the theme would stop being the
/// one the user picked. The five are Notepad++'s own set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabColour {
    #[default]
    None,
    Yellow,
    Green,
    Blue,
    Red,
    Purple,
}

impl TabColour {
    /// Every colour, in the order the menu cycles them.
    pub const ALL: [TabColour; 6] = [
        TabColour::None,
        TabColour::Yellow,
        TabColour::Green,
        TabColour::Blue,
        TabColour::Red,
        TabColour::Purple,
    ];

    /// The keyword written to the session file.
    pub fn key(self) -> &'static str {
        match self {
            TabColour::None => "none",
            TabColour::Yellow => "yellow",
            TabColour::Green => "green",
            TabColour::Blue => "blue",
            TabColour::Red => "red",
            TabColour::Purple => "purple",
        }
    }

    /// The label shown in the context menu.
    pub fn label(self) -> &'static str {
        match self {
            TabColour::None => "No Colour",
            TabColour::Yellow => "Yellow",
            TabColour::Green => "Green",
            TabColour::Blue => "Blue",
            TabColour::Red => "Red",
            TabColour::Purple => "Purple",
        }
    }

    /// Reads a keyword back, or `None` when it is not one of ours.
    pub fn from_key(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|colour| colour.key() == text)
    }

    /// The next colour, wrapping. What the menu entry does when picked.
    pub fn next(self) -> Self {
        let at = Self::ALL.iter().position(|c| *c == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }

    /// The colour itself, or `None` for an unpainted tab.
    pub fn rgb(self) -> Option<Rgb> {
        match self {
            TabColour::None => None,
            TabColour::Yellow => Some(Rgb::new(0xC8, 0xA8, 0x2A)),
            TabColour::Green => Some(Rgb::new(0x4C, 0xA0, 0x50)),
            TabColour::Blue => Some(Rgb::new(0x3D, 0x7E, 0xC4)),
            TabColour::Red => Some(Rgb::new(0xC0, 0x4A, 0x44)),
            TabColour::Purple => Some(Rgb::new(0x8A, 0x5C, 0xB8)),
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_colour_round_trips_through_its_key() {
        for colour in TabColour::ALL {
            assert_eq!(TabColour::from_key(colour.key()), Some(colour));
        }
        assert_eq!(TabColour::from_key("chartreuse"), None);
    }

    #[test]
    fn stepping_a_colour_wraps_back_to_none() {
        assert_eq!(TabColour::None.next(), TabColour::Yellow);
        assert_eq!(TabColour::Purple.next(), TabColour::None);
    }

    #[test]
    fn only_none_has_no_colour_of_its_own() {
        assert_eq!(TabColour::None.rgb(), None);
        for colour in TabColour::ALL.into_iter().filter(|c| *c != TabColour::None) {
            assert!(colour.rgb().is_some(), "{colour:?}");
        }
    }

    #[test]
    fn the_colours_are_distinguishable_from_each_other() {
        let mut seen = Vec::new();
        for colour in TabColour::ALL.into_iter().filter(|c| *c != TabColour::None) {
            let rgb = colour.rgb().expect("a colour");
            assert!(!seen.contains(&rgb), "{colour:?} repeats a colour");
            seen.push(rgb);
        }
    }
}
