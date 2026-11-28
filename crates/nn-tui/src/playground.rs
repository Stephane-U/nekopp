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
//! Two tiers, decided when the mode opens. The plain one draws the page in the
//! theme's own tones, so it costs what any other screen costs and reads the same
//! on a terminal with sixteen colours as on one with sixteen million. The colour
//! one gives every cell a colour of its own -- the kitten her ginger, a card the
//! colour its syntax would have, a glow the hue the simulation gave it -- and
//! asks the loop for a faster clock, because a page in colour that steps five
//! times a second is a slideshow. Which tier runs is the terminal's answer, not a
//! setting: `crate::term::per_cell_colour` reads it off the environment.
//!
//! Nothing here holds a pointer: a terminal has no mouse of its own to follow, so
//! the kitten follows no pointer and every key that is not `Esc` counts as the
//! user being there.

use nn_core::playground::{Card, Frame, Glow, Input, Playground as World, Shade};
use nn_shared::color::{mix, rainbow, Rgb};
use nn_shared::theme::{shade_colour, Theme};

use crate::keys::Key;
use crate::palette::{self, Tone};
use crate::screen::Grid;

/// How long the loop waits for a key while the plain page is up.
///
/// The editor's own tick is two hundred milliseconds: a shell that redraws five
/// times a second with nothing happening has decided nobody is reading. A page of
/// walking text is the exception, so the mode asks for this one instead, and the
/// editor's rate is untouched by it being there.
const PLAIN_TICK_MS: i32 = 40;

/// The same, in colour: the same page, drawn in a way that earns more frames.
const COLOUR_TICK_MS: i32 = 25;

/// The clock the glows' hues drift with, per second.
///
/// The window's spark shader adds the mode's clock to a spark's hue and then a
/// little more for how far it is from the middle of the screen. The numbers are
/// the shader's, because the two tiers are the same mode.
const HUE_DRIFT: f32 = 0.12;
const HUE_ACROSS: f32 = 0.10;

/// How dim the page has to be before the plain tier gives it up.
///
/// `nn_core` fades a page to a little under half brightness over the minute that
/// follows four without input, and a screen of characters has no shades to fade
/// with: it keeps its own colours while the page is mostly itself, and gives them
/// up at the end of the fade rather than stepping through three greys on the way
/// down. The colour tier needs none of this, because a colour can be mixed.
const QUIET: f32 = 0.4;

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
    /// The colours the tier with an ink per cell draws from. A terminal has no
    /// shader to ask, so the page's colours have to be here rather than on the
    /// GPU.
    theme: Theme,
    /// Whether a cell may carry a colour of its own.
    colour: bool,
}

impl Playground {
    /// Opens the mode on `grid`, from `seed`, in colour or in the theme's tones.
    pub fn new(
        seed: u64,
        grid: [u32; 2],
        theme: &Theme,
        colour: bool,
    ) -> Playground {
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
            theme: *theme,
            colour,
        }
    }

    /// How long the loop should wait for a key while the page is up.
    ///
    /// A page in colour is the same page with more to say, so it is given more
    /// frames; a page in characters has nothing to show between two positions of a
    /// card, and forty milliseconds is already smooth for that.
    pub fn tick_ms(&self) -> i32 {
        if self.colour {
            COLOUR_TICK_MS
        } else {
            PLAIN_TICK_MS
        }
    }

    /// `colour` faded into the page by `dim`.
    ///
    /// The window dims by multiplying a colour, which on a paper-white theme
    /// makes the page darker than the paper it is printed on. Mixing towards the
    /// page is the same idea with the right answer on both themes, and it is the
    /// one thing a cell can be told to do that a character cannot: a colour is a
    /// number, so it can be half way between two of them.
    fn fade(&self, colour: Rgb, dim: f32) -> Rgb {
        mix(colour, self.theme.background, dim)
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
            self.glow(grid, &frame, glow);
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
        if self.colour {
            // The colour its syntax would have had in the window, which is what the
            // same card is tinted with there.
            let colour = self.theme.style_color(card.style);
            grid.paint(row, column, card.ch, self.fade(colour, dim));
            return;
        }
        let tone = if dim > QUIET {
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
        let mut cells: Vec<(i32, i32, f32, Shade)> = Vec::new();
        for block in &frame.blocks {
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
                            *one = (x, y, block.pos[2], block.shade);
                        }
                        Some(_) => {}
                        None => cells.push((x, y, block.pos[2], block.shade)),
                    }
                }
            }
        }
        for (x, y, _, shade) in cells {
            if let Some((row, column)) = cell(grid, x as f32, y as f32) {
                let ch = shade_char(shade);
                if self.colour {
                    // The same five colours the window lights her with, from the
                    // definition both front-ends share.
                    let colour = shade_colour(shade, &self.theme);
                    grid.paint(row, column, ch, self.fade(colour, frame.dim));
                } else {
                    let tone = if frame.dim > QUIET {
                        Tone::Comment
                    } else {
                        shade_tone(shade)
                    };
                    grid.put(row, column, ch, tone);
                }
            }
        }
    }

    /// One glow: the trail, a ring, a piece of confetti.
    ///
    /// A terminal cannot blend, so the brightness decides the mark rather than the
    /// distance from the middle. In colour it decides the hue as well, which is the
    /// one thing a cell can do that a lit pixel cannot be told twice about: the
    /// window draws a spark in the hue the simulation sent, and so does this, with
    /// the shader's own drift -- the clock, and how far from the middle it is.
    fn glow(&self, grid: &mut Grid, frame: &Frame, glow: &Glow) {
        let ch = if glow.alpha > 0.66 {
            '*'
        } else if glow.alpha > 0.4 {
            '+'
        } else if glow.alpha > 0.2 {
            '.'
        } else {
            return;
        };
        let Some((row, column)) = cell(grid, glow.pos[0], glow.pos[1]) else {
            return;
        };
        if self.colour {
            let x = glow.pos[0] / grid.width().max(1) as f32 * 2.0 - 1.0;
            let y = glow.pos[1] / grid.height().max(1) as f32 * 2.0 - 1.0;
            let across = (x * x + y * y).sqrt();
            let hue = glow.hue + frame.time * HUE_DRIFT + across * HUE_ACROSS;
            grid.paint(row, column, ch, self.fade(rainbow(hue), frame.dim));
            return;
        }
        let tone = if frame.dim > QUIET {
            Tone::Comment
        } else {
            Tone::Function
        };
        grid.put(row, column, ch, tone);
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
    use nn_shared::color::Rgb;
    use crate::screen::Cell;

    fn playground() -> Playground {
        Playground::new(7, [80, 24], &Theme::DARK, false)
    }

    fn in_colour() -> Playground {
        Playground::new(7, [80, 24], &Theme::DARK, true)
    }

    fn screen(play: &Playground) -> String {
        let mut grid = Grid::new(80, 24);
        play.draw(&mut grid);
        (0..24)
            .map(|row| (0..80).map(|c| grid.get(row, c).ch).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Every colour the page put on the screen, in the order the cells are read.
    fn inks(play: &Playground) -> Vec<Rgb> {
        let mut grid = Grid::new(80, 24);
        play.draw(&mut grid);
        let mut found = Vec::new();
        for row in 0..24 {
            for column in 0..80 {
                if let Some(ink) = grid.get(row, column).ink {
                    found.push(ink);
                }
            }
        }
        found
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
    fn the_colour_tier_gives_the_kitten_her_own_colours_and_the_plain_one_none() {
        let mut plain = playground();
        plain.step(0.1, [80, 24]);
        assert!(inks(&plain).is_empty(), "the plain tier painted a colour");

        let mut colour = in_colour();
        colour.step(0.1, [80, 24]);
        let inks = inks(&colour);
        assert!(inks.len() > 8, "{} cells in colour", inks.len());
        // The kitten is the same five colours the window lights her with, which
        // is the point of the definition living in `nn-shared`.
        let fur = shade_colour(Shade::Fur, &Theme::DARK);
        let nose = shade_colour(Shade::Nose, &Theme::DARK);
        assert!(inks.contains(&fur), "no fur in {}", inks.len());
        assert!(inks.contains(&nose), "no nose");
        // And a coloured page is characters too: it is the same page, not another.
        assert!(screen(&colour).contains("●"));
    }

    #[test]
    fn a_coloured_second_costs_about_what_a_plain_one_does() {
        // The number a terminal mode is paid for in. A frame is written by
        // difference and the page drifts, so what a second of it costs is a few
        // kilobytes either way: the cost of a colour per cell is one escape where
        // the tone was one escape, and the cards the page moves are the same cards.
        let second = |colour: bool| {
            let mut play = Playground::new(7, [80, 24], &Theme::DARK, colour);
            let mut frame = crate::screen::Frame::new(80, 24);
            play.step(0.1, [80, 24]);
            play.draw(frame.grid());
            let paint = frame.render().len();
            let tick = play.tick_ms() as f32 / 1000.0;
            let mut total = 0;
            for _ in 0..(1.0 / tick) as usize {
                play.step(tick, [80, 24]);
                play.draw(frame.grid());
                total += frame.render().len();
            }
            (paint, total)
        };
        let (plain_paint, plain) = second(false);
        let (colour_paint, colour) = second(true);
        println!(
            "a plain page: {plain_paint} bytes, then {plain} a second; \
             a coloured one: {colour_paint}, then {colour}"
        );
        // The first frame writes the screen; a second of moving writes a fraction
        // of one, and the colour tier is not the more expensive of the two by more
        // than the extra frames it asks for.
        assert!(plain < plain_paint, "{plain} against {plain_paint}");
        assert!(colour < colour_paint, "{colour} against {colour_paint}");
        assert!(colour < plain * 2, "{colour} against {plain}");
    }

    #[test]
    fn a_quiet_page_fades_in_colour_and_changes_tone_in_characters() {
        // Five minutes of nobody touching anything, which is what fades the page:
        // `nn_core` walks it down to a little under half brightness over the minute
        // that follows four.
        let mut fresh = in_colour();
        fresh.step(0.1, [80, 24]);
        let fur = shade_colour(Shade::Fur, &Theme::DARK);
        assert!(inks(&fresh).contains(&fur), "no fur at rest");

        let mut quiet = in_colour();
        quiet.step(300.0, [80, 24]);
        let dim = quiet.world.frame(&quiet.input).dim;
        assert!(dim > QUIET, "the page did not fade: {dim}");
        // A colour fades rather than going out, because a colour is a number and
        // can be half way between two of them: the fur ends up part way to the
        // page, which is what the window's dimming amounts to.
        assert_eq!(quiet.fade(fur, 0.0), fur);
        assert_eq!(quiet.fade(fur, 1.0), Theme::DARK.background);
        assert_eq!(
            quiet.fade(fur, dim),
            mix(fur, Theme::DARK.background, dim)
        );
        // And the page is still drawn: a faded page is quieter, not empty.
        assert!(!inks(&quiet).is_empty(), "the faded page drew nothing");

        // The plain tier has no shades to fade with, so it gives the page up to one
        // tone at the end of the fade. Every character it draws is that tone.
        let mut plain = playground();
        plain.step(300.0, [80, 24]);
        assert!(inks(&plain).is_empty());
        let mut grid = Grid::new(80, 24);
        plain.draw(&mut grid);
        for row in 0..24 {
            for column in 0..80 {
                let cell = grid.get(row, column);
                if cell.ch != ' ' && cell.ch != Cell::CONT.ch {
                    let at = (row, column);
                    assert_eq!(cell.tone, Tone::Comment, "{cell:?} at {at:?}");
                }
            }
        }
    }

    #[test]
    fn the_colour_tier_asks_for_the_faster_clock() {
        assert_eq!(playground().tick_ms(), PLAIN_TICK_MS);
        assert_eq!(in_colour().tick_ms(), COLOUR_TICK_MS);
        assert!(!playground().colour);
        assert!(in_colour().colour);
        // The colour tier is the faster of the two, which is the whole of why it
        // asks for a number rather than sharing one.
        assert!(in_colour().tick_ms() < playground().tick_ms());
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
