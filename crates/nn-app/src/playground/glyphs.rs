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

//! The glyphs the room is papered with, as one texture.
//!
//! The walls are code, so the cards the walls are built from are characters. A
//! character on the GPU is a quad and a lookup, and the lookup is into an atlas
//! built once: the same atlas the icons use, in the same single-channel format,
//! rasterised by the same shaper that draws the document.
//!
//! Two things make this different from drawing text. The first is that the cards
//! are a fixed shape — one line tall and 0.55 of a line wide — so each glyph is
//! rasterised into a cell of exactly that shape and the cell is stretched onto the
//! quad: no font metric has to be right for the proportions to come out right. The
//! second is that the material is ASCII, one hundred and ninety-five characters of
//! it, so the atlas is a grid of 16 by 6 and a lookup is a subtraction.
//!
//! The fit loop is what keeps this honest across fonts. Rather than trusting an
//! ascent, a descent or an advance, it rasterises the whole set, measures the ink,
//! and shrinks the font size until the ink is inside a cell. A font whose glyphs
//! are twice as wide as the monospace this program measures itself against comes
//! out smaller, not clipped.

use cosmic_text::{
    fontdb, Attrs, Buffer, CacheKey, CacheKeyFlags, Family, FontSystem, Metrics, Shaping,
    SwashCache, SwashContent,
};
use nn_core::playground::scene::GLYPH_WIDTH;

/// The first character in the atlas: a space.
pub const FIRST: u32 = 0x20;
/// The last: a tilde.
pub const LAST: u32 = 0x7E;
/// How many cells fit across the atlas.
const COLUMNS: u32 = 16;
/// How many times the fit loop may shrink the font before it gives up and clips.
const ATTEMPTS: u32 = 3;

/// The characters of the room, as one coverage texture.
pub struct GlyphAtlas {
    /// The size of one cell, in pixels.
    pub cell: [u32; 2],
    /// Cells across.
    pub cols: u32,
    /// Cells down.
    pub rows: u32,
    /// `cols * cell[0]` by `rows * cell[1]` coverage values, row-major.
    pub pixels: Vec<u8>,
}

impl GlyphAtlas {
    /// Rasterises every printable ASCII character into a cell `px` pixels tall.
    ///
    /// `px` is clamped to something a texture holds comfortably; the caller picks
    /// it from the display density and never has to care that a window may be
    /// resized afterwards, because the cards are scaled in world units.
    pub fn build(font_system: &mut FontSystem, px: u32) -> Self {
        let px = px.clamp(16, 256);
        let cell = [
            ((px as f32 * GLYPH_WIDTH).round() as u32).max(6),
            px,
        ];
        let count = LAST - FIRST + 1;
        let cols = COLUMNS;
        let rows = count.div_ceil(cols);
        let mut pixels = vec![0u8; (cols * cell[0] * rows * cell[1]) as usize];

        // A cell is wider than a monospace glyph is tall, so the font size that
        // fits is a little under the cell height; the loop settles it.
        let mut font_size = cell[1] as f32 * 0.9;
        let mut stamps = Vec::new();
        let mut ink = Ink::default();
        for _ in 0..ATTEMPTS {
            let drawn = rasterise(font_system, font_size, cell);
            stamps = drawn.0;
            ink = drawn.1;
            // A pixel of margin all round, so that the outer ring of every cell
            // stays blank and a linear sample along a card's edge cannot pick up
            // the character next door.
            if ink.width() + 2 <= cell[0] as i32 && ink.height() + 2 <= cell[1] as i32 {
                break;
            }
            let by_width = (cell[0] as f32 - 2.0) / ink.width().max(1) as f32;
            let by_height = (cell[1] as f32 - 2.0) / ink.height().max(1) as f32;
            font_size *= (by_width.min(by_height) * 0.98).min(1.0);
        }

        // One shift for the whole set, not one per glyph: a baseline that moved
        // from character to character would make the walls ripple.
        let dx = (cell[0] as i32 - ink.width()) / 2 - ink.left;
        let dy = (cell[1] as i32 - ink.height()) / 2 - ink.top;
        for (index, stamp) in stamps.iter().enumerate() {
            let Some(stamp) = stamp else { continue };
            let index = index as u32;
            let (cx, cy) = ((index % cols) * cell[0], (index / cols) * cell[1]);
            let (origin_x, origin_y) = (stamp.left + dx, stamp.top + dy);
            for row in 0..stamp.height {
                let y = origin_y + row;
                if y < 0 || y >= cell[1] as i32 {
                    continue;
                }
                let dst = ((cy + y as u32) * cols * cell[0] + cx) as usize;
                for column in 0..stamp.width {
                    let x = origin_x + column;
                    if x < 0 || x >= cell[0] as i32 {
                        continue;
                    }
                    let value = stamp.pixels[(row * stamp.width + column) as usize];
                    if value > 0 {
                        pixels[dst + x as usize] = value;
                    }
                }
            }
        }

        Self {
            cell,
            cols,
            rows,
            pixels,
        }
    }

    /// The cell a character is drawn from: space for anything outside ASCII, and
    /// for the space itself, which is blank.
    pub fn index(ch: char) -> u32 {
        let code = ch as u32;
        if (FIRST..=LAST).contains(&code) {
            code - FIRST
        } else {
            0
        }
    }

    /// The texture coordinates of one character's cell.
    ///
    /// The vertical pair is bottom-up, because the quad the atlas is stretched
    /// onto has its own texture coordinates that way; getting this wrong stands
    /// every character on its head, which is at least obvious.
    pub fn uv(&self, ch: char) -> [f32; 4] {
        let index = Self::index(ch);
        let (cx, cy) = (index % self.cols, index / self.cols);
        let (width, height) = (self.width() as f32, self.height() as f32);
        let inset = 0.5;
        let (left, top) = (cx as f32 * self.cell[0] as f32, cy as f32 * self.cell[1] as f32);
        [
            (left + inset) / width,
            (top + self.cell[1] as f32 - inset) / height,
            (left + self.cell[0] as f32 - inset) / width,
            (top + inset) / height,
        ]
    }

    /// The width of the atlas texture, in pixels.
    pub fn width(&self) -> u32 {
        self.cols * self.cell[0]
    }

    /// The height of the atlas texture, in pixels.
    pub fn height(&self) -> u32 {
        self.rows * self.cell[1]
    }

    /// How much ink a character has, for the tests.
    #[cfg(test)]
    pub fn coverage(&self, ch: char) -> u32 {
        let index = Self::index(ch);
        let (cx, cy) = (
            (index % self.cols) * self.cell[0],
            (index / self.cols) * self.cell[1],
        );
        let mut total = 0;
        for y in 0..self.cell[1] {
            for x in 0..self.cell[0] {
                let at = ((cy + y) * self.width() + cx + x) as usize;
                if self.pixels[at] > 8 {
                    total += 1;
                }
            }
        }
        total
    }
}

/// The ink box of a whole set of glyphs, in cell pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ink {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl Default for Ink {
    fn default() -> Self {
        Self {
            left: i32::MAX,
            top: i32::MAX,
            right: i32::MIN,
            bottom: i32::MIN,
        }
    }
}

impl Ink {
    fn width(&self) -> i32 {
        (self.right - self.left + 1).max(0)
    }

    fn height(&self) -> i32 {
        (self.bottom - self.top + 1).max(0)
    }

    fn grow(&mut self, x: i32, y: i32) {
        self.left = self.left.min(x);
        self.top = self.top.min(y);
        self.right = self.right.max(x);
        self.bottom = self.bottom.max(y);
    }
}

/// One character, rasterised at the pen and not yet placed in its cell.
struct Stamp {
    /// `width * height` coverage values, row-major from the bitmap's top left.
    pixels: Vec<u8>,
    width: i32,
    height: i32,
    /// Where the bitmap's top left corner sits in cell pixels.
    left: i32,
    top: i32,
}

/// Draws every character at one pen position and one baseline, and reports where
/// the ink of the whole set ended up.
///
/// The pen is the cell's left edge and the baseline its floor, so a character is
/// drawn where its own bearings put it and the caller shifts the finished set
/// into the middle afterwards. Measuring the ink from the placements rather than
/// from the pixels that survived is what lets the fit loop see a character that
/// is too big: a box measured after the clipping always fits.
fn rasterise(
    font_system: &mut FontSystem,
    font_size: f32,
    cell: [u32; 2],
) -> (Vec<Option<Stamp>>, Ink) {
    let text: String = (FIRST..=LAST).filter_map(char::from_u32).collect();
    let count = (LAST - FIRST + 1) as usize;
    let mut stamps: Vec<Option<Stamp>> = (0..count).map(|_| None).collect();
    let mut ink = Ink::default();

    let mut buffer = Buffer::new_empty(Metrics::new(font_size, cell[1] as f32));
    buffer.set_size(None, None);
    buffer.set_text(
        &text,
        &Attrs::new().family(Family::Monospace),
        Shaping::Basic,
        None,
    );
    buffer.shape_until_scroll(font_system, false);

    let pen_x = 0.0;
    let baseline = cell[1] as f32;
    let mut swash = SwashCache::new();

    for run in buffer.layout_runs() {
        for glyph in run.glyphs {
            let index = glyph.start as u32;
            if index as usize >= count {
                continue;
            }
            let key = CacheKey::new(
                glyph.font_id,
                glyph.glyph_id,
                font_size,
                (0.0, 0.0),
                fontdb::Weight::NORMAL,
                CacheKeyFlags::empty(),
            )
            .0;
            let Some(image) = swash.get_image(font_system, key) else {
                continue;
            };
            if image.content != SwashContent::Mask {
                continue;
            }
            let placement = image.placement;
            let (width, height) = (placement.width as i32, placement.height as i32);
            if width == 0 || height == 0 {
                continue;
            }
            let left = (pen_x + placement.left as f32).round() as i32;
            let top = (baseline - placement.top as f32).round() as i32;
            ink.grow(left, top);
            ink.grow(left + width - 1, top + height - 1);
            stamps[index as usize] = Some(Stamp {
                pixels: image.data.to_vec(),
                width,
                height,
                left,
                top,
            });
        }
    }

    if ink.width() == 0 || ink.height() == 0 {
        ink = Ink {
            left: 0,
            top: 0,
            right: cell[0] as i32 - 1,
            bottom: cell[1] as i32 - 1,
        };
    }
    (stamps, ink)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atlas() -> GlyphAtlas {
        let mut font_system = crate::fonts::font_system();
        GlyphAtlas::build(&mut font_system, 64)
    }

    #[test]
    fn the_atlas_has_a_cell_for_every_printable_character() {
        let atlas = atlas();
        assert_eq!(atlas.cols, COLUMNS);
        assert_eq!(atlas.rows, (LAST - FIRST + 1).div_ceil(COLUMNS));
        assert_eq!(
            atlas.pixels.len(),
            (atlas.width() * atlas.height()) as usize
        );
        assert_eq!(atlas.cell[1], 64);
        // A cell is a card's shape, not a square.
        assert!(atlas.cell[0] < atlas.cell[1], "{:?}", atlas.cell);
    }

    #[test]
    fn letters_have_ink_and_a_space_has_none() {
        let atlas = atlas();
        assert_eq!(atlas.coverage(' '), 0);
        for ch in ['a', 'K', '#', '{', '0', ';', '~'] {
            assert!(atlas.coverage(ch) > 12, "{ch} drew nothing");
        }
    }

    #[test]
    fn every_character_stays_inside_its_own_cell() {
        let atlas = atlas();
        // The outer ring of every cell is blank, so a linear sample along an edge
        // cannot pick up the character next door.
        for index in 0..(LAST - FIRST + 1) {
            let (cx, cy) = (
                (index % atlas.cols) * atlas.cell[0],
                (index / atlas.cols) * atlas.cell[1],
            );
            for y in 0..atlas.cell[1] {
                for x in 0..atlas.cell[0] {
                    let edge = x == 0 || y == 0 || x == atlas.cell[0] - 1 || y == atlas.cell[1] - 1;
                    if !edge {
                        continue;
                    }
                    let at = ((cy + y) * atlas.width() + cx + x) as usize;
                    assert_eq!(atlas.pixels[at], 0, "cell {index} touches its edge at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn a_character_outside_ascii_falls_back_to_the_first_cell() {
        assert_eq!(GlyphAtlas::index(' '), 0);
        assert_eq!(GlyphAtlas::index('~'), LAST - FIRST);
        assert_eq!(GlyphAtlas::index('é'), 0);
        assert_eq!(GlyphAtlas::index('\u{4e00}'), 0);
    }

    #[test]
    fn the_texture_coordinates_run_bottom_up_inside_the_cell() {
        let atlas = atlas();
        let space = atlas.uv(' ');
        let next = atlas.uv('!');
        // Adjacent cells are a texel apart, not touching: the half-texel inset
        // at each edge is what keeps a linear sample off the neighbour.
        let gap = next[0] - space[2];
        assert!(gap > 0.0 && gap < 0.01, "a cell apart by {gap}");
        // The quad's own coordinates run bottom-up, so the pair the shader uses
        // for its bottom edge is the larger of the two texture coordinates.
        for ch in [' ', '!', 'a', 'z', '0', '~'] {
            let uv = atlas.uv(ch);
            assert!(uv[1] > uv[3], "{ch}: {uv:?}");
            assert!(uv[0] < uv[2], "{ch}: {uv:?}");
        }
        // A cell on the second row sits below the first, both edges further in.
        let second_row = atlas.uv('0');
        assert!(second_row[1] > space[1], "{space:?} {second_row:?}");
        assert!(second_row[3] > space[3], "{space:?} {second_row:?}");
        // Every cell's coordinates are inside the texture.
        for ch in ['a', 'z', '0', '~'] {
            let uv = atlas.uv(ch);
            for value in uv {
                assert!((0.0..=1.0).contains(&value), "{ch}: {uv:?}");
            }
        }
    }

    #[test]
    fn the_ink_of_the_whole_set_fits_one_cell() {
        let mut font_system = crate::fonts::font_system();
        let atlas = GlyphAtlas::build(&mut font_system, 48);
        // The uppercase letters share a top edge and the descenders a bottom, so
        // the set spans most of a cell without spilling out of it.
        let tallest = ['A', 'g', 'j', 'y', '|'].map(|ch| atlas.coverage(ch));
        assert!(tallest.iter().all(|value| *value > 12), "{tallest:?}");
        assert!(atlas.coverage('W') > atlas.coverage('i'), "proportional ink");
    }
}

