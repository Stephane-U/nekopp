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

//! The playground, in characters.
//!
//! The simulation is `nn_core::playground`'s and is not touched: it was written
//! in cells, with an orthographic camera and one unit to a line of text, so what
//! this file owns is the flattening -- a card becomes the character it was
//! carrying at the cell its centre names, a box of the kitten becomes a run of
//! cells in the shade it was given, and a glow becomes one punctuation mark.
//!
//! This is the plain tier: the page is drawn in the theme's own tones rather than
//! in per-cell colour, so it costs what any other screen costs and reads the same
//! on a terminal that has sixteen colours as on one that has sixteen million. The
//! colour tier the plan describes is a later pass behind a preference; what it
//! would add is a hue per cell and a faster clock, not a different page.
//!
//! Nothing here holds a pointer: a terminal has no mouse of its own to follow, so
//! the kitten follows no pointer and every key that is not `Esc` counts as the
//! user being there.

use nn_core::playground::{Card, Frame, Input, Playground as World, Shade};

use crate::keys::Key;
use crate::palette::{self, Tone};
use crate::screen::Grid;

/// How solid a card has to be before one of its cells is worth a character.
///
/// A card fades as it is knocked about, and a third of a glyph is not a glyph:
/// below this the cell is left as the page had it.
const VISIBLE: f32 = 0.34;

/// How much bigger the kitten is drawn than the page.
///
/// One unit is one line of text, and she is two and a quarter of them tall: three
/// cells, which is a cat-shaped smudge and nothing more. The window can spend
/// pixels on her; a screen of characters has to spend cells, so she is drawn at
/// twice her size -- five cells, where a body, a head and a tail can be told
/// apart -- and the page keeps its own scale.
const KITTEN_SCALE: f32 = 2.0;

/// The character a shade of the kitten is drawn with.
///
/// Fur is a solid block, the dark side a darker one, and the light side a lighter
/// one, which is the whole of the three-dimensional shear a terminal can keep: the
/// window's boxes are turned in the page, and a screen of characters has no room
/// to show it.
fn shade_char(shade: Shade) -> char {
    match shade {
        Shade::Fur => '█',
        Shade::FurDark => '▓',
        Shade::FurLight => '▒',
        Shade::Nose => '●',
        Shade::Eye => 'o',
    }
}

/// The tone a shade is drawn in, chosen so the kitten reads as one animal rather
/// than as a pattern: a bright block, a dimmer one, and the two marks that are
/// meant to be found.
fn shade_tone(shade: Shade) -> Tone {
    match shade {
        Shade::Fur => Tone::MenuHover,
        Shade::FurDark => Tone::MenuBorder,
        Shade::FurLight => Tone::Gutter,
        Shade::Nose => Tone::String,
        Shade::Eye => Tone::Text,
    }
}

/// The playground, and the frame it is on.
pub struct Playground {
    world: World,
    /// What the simulation is told about the world: the grid, and how long since
    /// anyone touched anything.
    input: Input,
    /// Seconds since the mode opened, which is the clock the page's shimmer reads.
    elapsed: f32,
}

impl Playground {
    /// Opens the mode on `grid`, from `seed`.
    pub fn new(seed: u64, grid: [u32; 2]) -> Playground {
        let input = Input {
            pointer: None,
            pointer_age: f32::MAX,
            input_age: 0.0,
            grid,
        };
        Playground {
            world: World::new(seed),
            input,
            elapsed: 0.0,
        }
    }

    /// Advances the simulation by `dt` seconds.
    pub fn step(&mut self, dt: f32, grid: [u32; 2]) {
        self.input.grid = grid;
        self.input.input_age += dt;
        self.elapsed += dt;
        self.world.step(dt, &self.input);
    }

    /// Counts a keystroke as the user being at the keyboard.
    ///
    /// The window resets the same clock on a mouse move, which is a thing a
    /// terminal has no way to report and no need for: a key is the whole of the
    /// evidence that somebody is there.
    pub fn touch(&mut self) {
        self.input.input_age = 0.0;
    }

    /// The page as characters. `true` when the key was the mode's.
    ///
    /// `Esc` leaves, which is what the window's own hint line says; every other
    /// key is somebody being present and is swallowed, because the document is
    /// behind this screen and not on it.
    pub fn handle(&mut self, key: Key) -> bool {
        if key == Key::Escape {
            return true;
        }
        self.touch();
        false
    }

    /// Draws one frame over the whole grid.
    pub fn draw(&self, grid: &mut Grid) {
        let width = grid.width();
        let height = grid.height();
        if width == 0 || height == 0 {
            return;
        }
        let frame = self.world.frame(&self.input);
        for cell in 0..width * height {
            grid.put(cell / width, cell % width, ' ', Tone::Text);
        }
        // The page first, then the kitten over it, then what the page threw in the
        // air: the same order the window draws them in, and the reason a token the
        // kitten is sitting on is behind her rather than on her.
        for card in &frame.cards {
            self.card(grid, card, frame.dim);
        }
        self.kitten(grid, &frame);
        for glow in &frame.glows {
            self.glow(grid, &frame, glow.pos, glow.alpha);
        }
        self.label(grid, &frame);
    }

    /// One card, at the cell its centre names.
    ///
    /// A card is [`nn_core::playground::GLYPH_WIDTH`] of a cell wide, so several
    /// of them can share a column; the last one to land there is the one a screen
    /// of characters can show, and the simulation has already decided which of
    /// them is in front.
    fn card(&self, grid: &mut Grid, card: &Card, dim: f32) {
        if card.alpha < VISIBLE {
            return;
        }
        let Some((row, column)) = cell(grid, card.pos[0], card.pos[1]) else {
            return;
        };
        let tone = if dim > 0.5 {
            Tone::Comment
        } else {
            palette::tone(card.style)
        };
        grid.put(row, column, card.ch, tone);
    }

    /// The kitten, as the cells her boxes cover.
    ///
    /// The boxes are turned in the page and a screen of characters cannot show
    /// that, so what is left is the silhouette: every cell she reaches is hers, and
    /// when two boxes reach the same cell the one nearer the front wins -- the nose
    /// over the muzzle, the chest over the body -- because that is the order a
    /// reader would see if the page had depth to it. She is drawn at
    /// [`KITTEN_SCALE`], since a cat three cells tall is a smudge.
    fn kitten(&self, grid: &mut Grid, frame: &Frame) {
        // Grown about her own middle rather than about the page's corner: the
        // simulation has already said which cell she stands on, and scaling that
        // too would walk her off the top of the screen.
        let middle = match frame.blocks.split_first() {
            Some((first, rest)) => {
                let sum = rest.iter().fold((first.pos[0], first.pos[1]), |sum, block| {
                    (sum.0 + block.pos[0], sum.1 + block.pos[1])
                });
                let count = frame.blocks.len() as f32;
                (sum.0 / count, sum.1 / count)
            }
            None => return,
        };
        let mut cells: Vec<(i32, i32, f32, char, Tone)> = Vec::new();
        for block in &frame.blocks {
            let ch = shade_char(block.shade);
            let tone = if frame.dim > 0.5 {
                Tone::Comment
            } else {
                shade_tone(block.shade)
            };
            // Each box is drawn at its own size, and never smaller than one cell:
            // a nose is a cell because a cell is the smallest mark there is, and
            // the ears are one each, which is what tells a cat from a rectangle.
            let half = (block.size[0] * KITTEN_SCALE / 2.0).max(0.5);
            let tall = (block.size[1] * KITTEN_SCALE / 2.0).max(0.5);
            let centre_x = middle.0 + (block.pos[0] - middle.0) * KITTEN_SCALE;
            let centre_y = middle.1 + (block.pos[1] - middle.1) * KITTEN_SCALE;
            let first = (centre_x - half).floor() as i32;
            let last = (centre_x + half).ceil() as i32;
            let bottom = (centre_y - tall).floor() as i32;
            let top = (centre_y + tall).ceil() as i32;
            for y in bottom..top {
                for x in first..last {
                    match cells.iter_mut().find(|one| one.0 == x && one.1 == y) {
                        // Nearer the front wins; a tie keeps what was drawn first,
                        // which is the darker fur and reads as an outline.
                        Some(one) if block.pos[2] > one.2 => {
                            *one = (x, y, block.pos[2], ch, tone);
                        }
                        Some(_) => {}
                        None => cells.push((x, y, block.pos[2], ch, tone)),
                    }
                }
            }
        }
        for (x, y, _, ch, tone) in cells {
            if let Some((row, column)) = cell(grid, x as f32, y as f32) {
                grid.put(row, column, ch, tone);
            }
        }
    }

    /// One glow: the trail, a ring, a piece of confetti.
    ///
    /// A terminal cannot blend, so the brightness decides the mark rather than the
    /// distance from the middle.
    fn glow(&self, grid: &mut Grid, frame: &Frame, pos: [f32; 3], alpha: f32) {
        let ch = if alpha > 0.66 {
            '*'
        } else if alpha > 0.4 {
            '+'
        } else if alpha > 0.2 {
            '.'
        } else {
            return;
        };
        let tone = if frame.dim > 0.5 {
            Tone::Comment
        } else {
            Tone::Function
        };
        if let Some((row, column)) = cell(grid, pos[0], pos[1]) {
            grid.put(row, column, ch, tone);
        }
    }

    /// The corner label and the way out, along the bottom row.
    ///
    /// The window's own sentence, in the window's own order: the material and the
    /// page on the left, the hint on the right when the kitten has given up.
    fn label(&self, grid: &mut Grid, frame: &Frame) {
        let row = grid.height().saturating_sub(1);
        let mut text = frame.label.clone();
        if frame.asleep {
            text.push_str(" · the kitten is asleep");
        }
        text.push_str(" · Esc leaves");
        grid.text(row, 1, &text, Tone::Comment);
    }
}

/// The cell a point of the page falls on, with the page's own axes: `x` right,
/// `y` up from the bottom.
fn cell(grid: &Grid, x: f32, y: f32) -> Option<(usize, usize)> {
    if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 {
        return None;
    }
    let column = x.round() as usize;
    let row = grid.height().checked_sub(1 + y.round() as usize)?;
    (column < grid.width()).then_some((row, column))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playground() -> Playground {
        Playground::new(7, [80, 24])
    }

    fn screen(play: &Playground) -> String {
        let mut grid = Grid::new(80, 24);
        play.draw(&mut grid);
        (0..24)
            .map(|row| (0..80).map(|c| grid.get(row, c).ch).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_page_is_drawn_as_characters_on_the_whole_screen() {
        let mut play = playground();
        play.step(0.1, [80, 24]);
        let text = screen(&play);
        let glyphs = text.chars().filter(|ch| *ch != ' ' && *ch != '\n').count();
        assert!(glyphs > 100, "the page drew {glyphs} characters");
        // And the way out is on the last row, in the window's own words.
        assert!(text.lines().last().unwrap_or_default().contains("Esc leaves"));
    }

    #[test]
    fn the_kitten_is_something_bigger_than_one_cell() {
        let mut play = playground();
        play.step(0.1, [80, 24]);

        let text = screen(&play);
        // She is blocks and marks, and a cat is more than one cell of either: the
        // shades are told apart by character, which is all a plain screen has.
        let fur = text
            .chars()
            .filter(|ch| "█▓▒●".contains(*ch))
            .count();
        assert!(fur > 8, "{fur} cells of fur\n{text}");
        // And she is on the page rather than off its edge.
        assert!(text.contains("●"), "no nose\n{text}");
    }

    #[test]
    fn escape_is_the_mode_s_only_key_and_the_rest_only_wake_it() {
        let mut play = playground();
        play.step(4.0, [80, 24]);
        let before = play.input.input_age;
        assert!(!play.handle(Key::Char('x')));
        assert!(play.input.input_age < before, "a key did not wake the page");
        assert!(play.handle(Key::Escape));
    }

    #[test]
    fn a_grid_with_no_room_draws_nothing_and_does_not_panic() {
        let mut play = playground();
        play.step(0.1, [0, 0]);
        let mut grid = Grid::new(0, 0);
        play.draw(&mut grid);
        // And a point off the page is dropped rather than wrapped round to the
        // other side, which is what an unsigned cast would do.
        let grid = Grid::new(10, 5);
        assert!(cell(&grid, -1.0, 2.0).is_none());
        assert!(cell(&grid, 11.0, 2.0).is_none());
        assert!(cell(&grid, 1.0, 9.0).is_none());
        assert_eq!(cell(&grid, 1.0, 0.0), Some((4, 1)));
    }
}
