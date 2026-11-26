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

//! The colour type, and the one measurement made of it.
//!
//! A colour is three bytes, and that is the whole of it: the settings file
//! writes them as hex, a theme is a list of them, and the two front-ends both
//! carry them -- a window into a shader, a terminal into as close a pair of
//! escape sequences as it can manage. What is *not* here is the conversion to
//! linear values, because that is a fact about a GPU rather than about a colour,
//! and the terminal has no shader to feed.

/// An sRGB colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Builds a colour from its sRGB components.
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self(r, g, b)
    }

    /// Perceived brightness, 0 for black and 255 for white.
    ///
    /// The Rec. 601 weights, matching [`nn_core::Rgb8::luminance`] so a colour
    /// read from a language file and the same colour in the theme compare the
    /// same way.
    pub fn luminance(self) -> u16 {
        let Rgb(r, g, b) = self;
        let weighted = 299 * r as u32 + 587 * g as u32 + 114 * b as u32;
        (weighted / 1000) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ends_of_the_scale_are_exact() {
        // The weights sum to a thousand, which is what makes white exact.
        assert_eq!(Rgb::new(0, 0, 0).luminance(), 0);
        assert_eq!(Rgb::new(255, 255, 255).luminance(), 255);
    }

    #[test]
    fn brightness_weighs_green_most() {
        // The same count in each channel, so only the weights can order them.
        let red = Rgb::new(100, 0, 0).luminance();
        let green = Rgb::new(0, 100, 0).luminance();
        let blue = Rgb::new(0, 0, 100).luminance();
        assert!(green > red && red > blue, "{green} {red} {blue}");
    }
}
