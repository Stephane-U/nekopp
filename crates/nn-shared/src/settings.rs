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

//! The values a setting can hold.
//!
//! A setting is a word in a file, a row in a dialog and a thing a program acts
//! on, and all three have to agree between the two front-ends. The module starts
//! with the one value that had to leave the window to be shared; the settings
//! themselves follow.

/// How the strip arranges its tabs.
///
/// The three arrangements Notepad++ offers under Settings > Preferences >
/// General > Tab Bar. Persisted by key, so the names are stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabLayout {
    /// One row across the top, clipped at the right edge.
    #[default]
    Horizontal,
    /// As many rows as the tabs need, up to three rows, then clipped.
    MultiLine,
    /// A column down the left edge of the pane.
    Vertical,
}

impl TabLayout {
    /// Every arrangement, in the order the menu cycles them.
    pub const ALL: [TabLayout; 3] = [
        TabLayout::Horizontal,
        TabLayout::MultiLine,
        TabLayout::Vertical,
    ];

    /// The keyword written to the settings file.
    pub fn key(self) -> &'static str {
        match self {
            TabLayout::Horizontal => "horizontal",
            TabLayout::MultiLine => "multiline",
            TabLayout::Vertical => "vertical",
        }
    }

    /// Reads a keyword back, or `None` when it is not one of ours.
    pub fn from_key(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|layout| layout.key() == text)
    }

    /// The label shown in the menu.
    pub fn label(self) -> &'static str {
        match self {
            TabLayout::Horizontal => "Single Row",
            TabLayout::MultiLine => "Multi-line",
            TabLayout::Vertical => "Vertical",
        }
    }

    /// The next arrangement, wrapping. What the menu entry does when picked.
    pub fn next(self) -> Self {
        let at = Self::ALL.iter().position(|l| *l == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }

    /// Whether the strip runs down the side rather than across the top.
    pub fn is_vertical(self) -> bool {
        self == TabLayout::Vertical
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_layout_round_trips_through_its_key() {
        for layout in TabLayout::ALL {
            assert_eq!(TabLayout::from_key(layout.key()), Some(layout));
        }
        assert_eq!(TabLayout::from_key("diagonal"), None);
    }

    #[test]
    fn stepping_a_layout_wraps_back_to_the_first() {
        assert_eq!(TabLayout::Horizontal.next(), TabLayout::MultiLine);
        assert_eq!(TabLayout::Vertical.next(), TabLayout::Horizontal);
    }

    #[test]
    fn every_layout_has_a_distinct_label() {
        let mut seen = Vec::new();
        for layout in TabLayout::ALL {
            assert!(!layout.label().is_empty());
            assert!(!seen.contains(&layout.label()), "{layout:?} repeats");
            seen.push(layout.label());
        }
    }

    #[test]
    fn only_the_vertical_layout_is_vertical() {
        assert!(TabLayout::Vertical.is_vertical());
        assert!(!TabLayout::Horizontal.is_vertical());
        assert!(!TabLayout::MultiLine.is_vertical());
    }
}
