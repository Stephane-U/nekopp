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

//! The colours, and the one conversion only a window can make.
//!
//! Everything is authored as sRGB bytes because that is how designers and colour
//! pickers think. The GPU wants linear values, so conversion happens once, here,
//! at the boundary. The colours themselves -- the type, the blend, the theme and
//! its catalogue -- live in `nn-shared`, because a terminal draws them too; what
//! is left in this file is the part that needs a shader to mean anything.

// The colour type moved to `nn-shared` in the hundred-and-fifteenth pass, and
// the theme with its catalogue in the hundred-and-sixteenth. What is left here is
// the half of the story that needs a GPU: sRGB bytes are authored, linear values
// are what a shader is given.
pub use nn_shared::color::{mix, Rgb};
pub use nn_shared::theme::{from_stylers, Theme};

/// What the GPU needs a colour turned into.
///
/// A trait rather than inherent methods because the type is no longer this
/// crate's to extend, and it is a trait here rather than in `nn-shared` because
/// none of these three answers means anything without a shader: linear values
/// for a vertex buffer, `cosmic_text`'s packed colour, and `wgpu`'s clear value.
pub trait ToGpu {
    /// The colour as linear RGBA, ready for a shader.
    fn to_linear(self) -> [f32; 4];
    /// The colour as linear RGBA with an explicit alpha.
    fn to_linear_alpha(self, alpha: f32) -> [f32; 4];
    /// The colour in the form glyphon expects.
    fn to_cosmic(self) -> cosmic_text::Color;
    /// The colour in the form `wgpu` expects for a clear value.
    fn to_wgpu(self) -> wgpu::Color;
}

impl ToGpu for Rgb {
    fn to_linear(self) -> [f32; 4] {
        [
            srgb_to_linear(self.0),
            srgb_to_linear(self.1),
            srgb_to_linear(self.2),
            1.0,
        ]
    }

    fn to_linear_alpha(self, alpha: f32) -> [f32; 4] {
        let [r, g, b, _] = self.to_linear();
        [r, g, b, alpha]
    }

    fn to_cosmic(self) -> cosmic_text::Color {
        cosmic_text::Color::rgb(self.0, self.1, self.2)
    }

    fn to_wgpu(self) -> wgpu::Color {
        let [r, g, b, a] = self.to_linear();
        wgpu::Color {
            r: r as f64,
            g: g as f64,
            b: b as f64,
            a: a as f64,
        }
    }
}

/// Converts one sRGB channel to linear.
fn srgb_to_linear(c: u8) -> f32 {
    let s = c as f32 / 255.0;
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_and_white_are_exact() {
        assert_eq!(srgb_to_linear(0), 0.0);
        assert!((srgb_to_linear(255) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn mid_grey_is_darker_in_linear() {
        // 128 sRGB is about 0.216 linear.
        assert!((srgb_to_linear(128) - 0.2158).abs() < 0.001);
    }

    #[test]
    fn linear_conversion_is_monotonic() {
        let mut prev = -1.0;
        for c in 0..=255u8 {
            let v = srgb_to_linear(c);
            assert!(v > prev);
            prev = v;
        }
    }
}
