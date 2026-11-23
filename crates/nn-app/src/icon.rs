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

//! The window icon.
//!
//! The icon is the same drawing the desktop entry and the packages use, so the
//! taskbar, the window list and the launcher all show one mark. It is embedded
//! with `include_bytes!` rather than looked up at run time: an installed binary
//! has no `packaging/` directory beside it, and a missing icon is not worth a
//! search path, a fallback and a warning.
//!
//! The mark is a kitten on an orange tile, and it is ours. Notepad++'s is a
//! chameleon on a pencil over a page with grass along the bottom; that
//! arrangement is their trademark, so none of it is reproduced here — not the
//! page, not the pencil, not the grass, and obviously not the animal. See
//! `packaging/npp.svg` for the drawing and the reasoning behind it.
//!
//! The PNG is decoded here rather than shipped as raw RGBA because the drawing
//! is edited as an SVG and rendered by Inkscape; keeping one source of truth
//! means the icon cannot drift from the packages. Decoding 256x256 costs about a
//! millisecond, once, before the first frame.

use winit::window::Icon;

/// The 256x256 rendering of `packaging/npp.svg`.
///
/// The path is relative to this file, which is `crates/nn-app/src/icon.rs`, so
/// it climbs three levels to the workspace root. That is a build-time path, not
/// a run-time one: the bytes end up inside the binary.
pub const PNG: &[u8] = include_bytes!("../../../packaging/npp-256.png");

/// Decodes a PNG into straight RGBA8, row-major, top row first.
///
/// Returns `None` for anything that is not a single 8-bit image the decoder can
/// normalise to RGBA. The caller has no useful recovery, so the error is not
/// carried: a window without an icon is a cosmetic loss, not a failure.
pub fn decode(png: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    // `normalize_to_color8` expands palettes and sub-byte greyscale and strips
    // 16-bit samples, which leaves RGB or RGBA. The icon is RGBA, but a future
    // re-export that drops the alpha channel should still work.
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width, info.height);
    let pixels = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => buf[..info.buffer_size()]
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 0xff])
            .collect(),
        _ => return None,
    };
    Some((pixels, w, h))
}

/// The icon to hand to `WindowAttributes::with_window_icon`.
///
/// `None` when the embedded PNG will not decode, which can only happen if the
/// file was replaced with something that is not a PNG.
pub fn window_icon() -> Option<Icon> {
    let (rgba, w, h) = decode(PNG)?;
    Icon::from_rgba(rgba, w, h).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_png_decodes_to_a_square_rgba_image() {
        let (rgba, w, h) = decode(PNG).expect("embedded icon decodes");
        assert_eq!((w, h), (256, 256));
        assert_eq!(rgba.len(), 256 * 256 * 4);
    }

    #[test]
    fn the_corners_are_transparent_so_the_tile_shape_shows() {
        let (rgba, w, _) = decode(PNG).expect("embedded icon decodes");
        let alpha_at = |x: usize, y: usize| rgba[(y * w as usize + x) * 4 + 3];
        assert_eq!(alpha_at(0, 0), 0, "top-left corner is outside the tile");
        assert_eq!(alpha_at(255, 255), 0, "bottom-right corner is outside");
    }

    #[test]
    fn the_tile_is_orange_and_the_kitten_is_cream() {
        let (rgba, w, _) = decode(PNG).expect("embedded icon decodes");
        let px = |x: usize, y: usize| {
            let i = (y * w as usize + x) * 4;
            (rgba[i], rgba[i + 1], rgba[i + 2])
        };
        // The tile, above the ears and clear of the rounded corner.
        let (r, g, b) = px(128, 24);
        assert!(r > 200 && r > g && g > b, "tile is orange, got {r},{g},{b}");
        // The kitten's cheek, below the eyes and inside the head.
        let (r, g, b) = px(128, 200);
        assert!(
            r > 200 && g > 200 && b > 190,
            "fur is cream, got {r},{g},{b}"
        );
    }

    #[test]
    fn the_face_has_an_eye_and_a_nose_where_the_drawing_puts_them() {
        // The face is the part that has to survive being scaled to 16 pixels,
        // so it is the part worth pinning: an eye that drifted off the head or a
        // nose that lost its colour would still pass a "the file decodes" test.
        let (rgba, w, _) = decode(PNG).expect("embedded icon decodes");
        let px = |x: usize, y: usize| {
            let i = (y * w as usize + x) * 4;
            (rgba[i], rgba[i + 1], rgba[i + 2])
        };
        let (r, g, b) = px(100, 140);
        assert!(r < 80 && g < 80 && b < 80, "eye is dark, got {r},{g},{b}");
        let (r, g, b) = px(128, 164);
        assert!(r > 200 && g < 190 && b < 190, "nose is pink, got {r},{g},{b}");
    }

    #[test]
    fn a_non_png_is_rejected_rather_than_panicking() {
        assert!(decode(b"not a png at all").is_none());
        assert!(decode(&[]).is_none());
    }

    #[test]
    fn the_window_icon_builds() {
        assert!(window_icon().is_some());
    }
}
