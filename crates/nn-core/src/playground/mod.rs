//! A page of code, a kitten, and five minutes of nothing to do.
//!
//! Playground mode replaces the window with a page of code and puts a kitten two
//! and a quarter lines tall on it. The cat walks across the text, follows the
//! pointer, bats loose characters about and eventually falls asleep; `Escape`
//! returns to the document. It exists because an editor is a place people spend
//! their day in, and the alternative to a five-minute kitten is a five-minute
//! phone.
//!
//! This module is the whole simulation and none of the drawing: pages are laid
//! out, tokens are knocked about, the kitten decides what to do next, and the
//! camera maps cells to the window. Nothing here knows what a pixel is, which is
//! what makes it testable -- `cargo test` drives it for minutes of simulated time
//! in milliseconds, and the renderer receives a list of cards and boxes to draw.
//!
//! The rules of the place:
//!
//! - One unit is one line of editor text, and the page is `columns` by `rows` of
//!   them. A card is [`scene::GLYPH_WIDTH`] wide and one unit tall, the kitten is
//!   two and a quarter tall, and nothing is in pixels, so the mode looks the same
//!   at any font size and on any display.
//! - The material is a compiled-in corpus, never the open document: see
//!   [`corpus`]. A screenshot of the playground cannot show anyone's work.
//! - The page is rebuilt every five minutes from the next seed.
//! - Three minutes with no input and the kitten curls up; four and the page dims.
//!
//! The pieces are [`scene`] (the page and its tokens), [`kitten`] (behaviour and
//! poses), [`camera`] (the grid and the pointer) and [`corpus`] (the code).

pub mod camera;
pub mod corpus;
pub mod kitten;
pub mod scene;

use camera::Camera;
use kitten::{Kitten, Mood, Senses};
use scene::Page;

pub use corpus::Flavour;
pub use scene::{Mote, Token};

/// One unit: the height of a line of the editor's text.
pub const CELL: f32 = 1.0;
/// How long a page stands before it is rebuilt.
pub const REBUILD_AFTER: f32 = 300.0;
/// How long the rebuild takes, during which the old page fades out and the new
/// one rises into place.
pub const BLEND: f32 = 2.5;
/// After this long with no input the page starts to dim, as a screensaver does.
pub const DIM_AFTER: f32 = 240.0;
/// How far it dims, at most.
pub const DIM: f32 = 0.45;

/// What a card is drawn as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardKind {
    /// A glyph from the atlas.
    Glyph,
    /// A soft dot: dust, or the shadow under the kitten.
    Dot,
}

/// One character, or one dot, placed on the page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Card {
    /// The glyph. Ignored by a dot.
    pub ch: char,
    /// The syntax style, as `nn_core::Style`, for the colour.
    pub style: u8,
    /// Where its centre is, in cells: `x` right, `y` up the page, `z` out of it.
    pub pos: [f32; 3],
    /// Turn about the page's vertical axis, in radians.
    pub yaw: f32,
    /// Tip about the horizontal axis: a glyph tipping off the page.
    pub pitch: f32,
    /// Turn in the page's own plane, in radians, for scattered punctuation.
    pub roll: f32,
    /// Width and height, in cells.
    pub size: [f32; 2],
    /// 0 gone, 1 solid.
    pub alpha: f32,
    pub kind: CardKind,
}

/// What the kitten is made of.
///
/// The renderer owns the actual colours, because they come from the theme; these
/// are the roles a shade plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shade {
    /// The main fur.
    Fur,
    /// The back and the ears: a shade darker.
    FurDark,
    /// Chest, paws, muzzle: a shade lighter.
    FurLight,
    /// The nose.
    Nose,
    /// The eyes.
    Eye,
}

/// One box of the kitten.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Block {
    /// Centre of the box, in cells.
    pub pos: [f32; 3],
    /// Full size on each axis, in cells.
    pub size: [f32; 3],
    /// Turn about the vertical axis.
    pub yaw: f32,
    pub shade: Shade,
}

/// What the world is doing, as the simulation needs to know it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Input {
    /// The pointer, in `-1..1` on both axes, y up.
    pub pointer: Option<[f32; 2]>,
    /// Seconds since the pointer last moved.
    pub pointer_age: f32,
    /// Seconds since any key, button or wheel.
    pub input_age: f32,
    /// The window as a grid of cells: columns, rows.
    pub grid: [u32; 2],
}

impl Default for Input {
    fn default() -> Self {
        Self {
            pointer: None,
            pointer_age: f32::MAX,
            input_age: 0.0,
            grid: [120, 40],
        }
    }
}

/// Everything one frame draws.
#[derive(Debug, Clone, Default)]
pub struct Frame {
    pub cards: Vec<Card>,
    pub blocks: Vec<Block>,
    pub camera: Camera,
    /// 0 bright, [`DIM`] dark, for the screensaver dimming.
    pub dim: f32,
    /// The corner label: which material, which page.
    pub label: String,
    /// Whether the kitten is asleep, for the hint line.
    pub asleep: bool,
}

/// The mode's state: the page, the kitten, and the clock.
#[derive(Debug, Clone)]
pub struct Playground {
    page: Page,
    /// The page being replaced, while the rebuild crosses over.
    previous: Option<Page>,
    /// 0 at the start of a rebuild, 1 when it has finished.
    blend: f32,
    kitten: Kitten,
    camera: Camera,
    rng: Rng,
    /// Seconds since the mode opened.
    time: f32,
    seed: u64,
}

impl Playground {
    /// Opens the mode with `seed`, which is what makes a run reproducible.
    pub fn new(seed: u64) -> Self {
        let mut rng = Rng::new(seed | 1);
        let camera = Camera::default();
        let flavour = Flavour::ALL[rng.below(Flavour::ALL.len())];
        let page = Page::new(
            flavour,
            seed,
            0,
            camera.columns,
            camera.rows,
        );
        let kitten = Kitten::new(page.width() * 0.5, page.rows as f32 * 0.5);
        Self {
            page,
            previous: None,
            blend: 1.0,
            kitten,
            camera,
            rng,
            time: 0.0,
            seed,
        }
    }

    /// The page on screen.
    pub fn page(&self) -> &Page {
        &self.page
    }

    /// The kitten.
    pub fn kitten(&self) -> &Kitten {
        &self.kitten
    }

    /// The seed this session was opened with.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Seconds since the mode opened.
    pub fn time(&self) -> f32 {
        self.time
    }

    /// Whether the kitten has fallen asleep.
    pub fn asleep(&self) -> bool {
        self.kitten.mood == Mood::Sleep
    }

    /// One frame of the world, `dt` seconds long.
    ///
    /// `dt` is clamped: a frame that took half a second -- a stalled process, a
    /// window dragged across the screen -- must not teleport a token off the page.
    /// Physics runs in fixed steps of `1/120` inside that budget.
    pub fn step(&mut self, dt: f32, input: &Input) {
        let dt = dt.clamp(0.0, 1.0 / 20.0);
        self.time += dt;

        // A resized window is a page of a different shape, so the page is laid out
        // again from the same seed. Nothing else moves: the kitten stays where it
        // was standing, in cells, and the tokens are put back on the page.
        let camera = Camera::new(input.grid[0], input.grid[1]);
        if camera != self.camera && self.previous.is_none() {
            self.camera = camera;
            let flavour = self.page.flavour;
            let (seed, index) = (self.page.seed, self.page.index);
            self.page = Page::new(flavour, seed, index, camera.columns, camera.rows);
            self.kitten.clamp_to(camera.width(), camera.rows as f32);
        }
        self.camera = camera;

        let page = [
            self.page.width(),
            self.page.rows as f32,
        ];
        let mut senses = Senses {
            pointer: self.camera.page(input.pointer),
            pointer_age: input.pointer_age,
            input_age: input.input_age,
            page,
            rng: &mut self.rng,
        };
        let strike = self.kitten.think(dt, &mut senses);
        if strike {
            let paw = self.kitten.paw();
            self.page.bat(paw, 1.7, 6.0);
        }

        // Fixed steps, so the same sequence of frames always gives the same tokens.
        let mut left = dt;
        while left > 1e-5 {
            let step = left.min(1.0 / 120.0);
            self.page.step_tokens(step);
            left -= step;
        }

        if self.time >= REBUILD_AFTER * (self.page.index + 1) as f32 {
            let index = self.page.index + 1;
            let flavour = Flavour::ALL[self.rng.below(Flavour::ALL.len())];
            let seed = self.seed ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let fresh = Page::new(flavour, seed, index, self.camera.columns, self.camera.rows);
            self.previous = Some(std::mem::replace(&mut self.page, fresh));
            self.blend = 0.0;
            // The tokens went with the last page, so the kitten gets a new place
            // to stand as well as new material to stand on.
            self.kitten = Kitten::new(self.page.width() * 0.5, self.page.rows as f32 * 0.5);
        }
        if self.blend < 1.0 {
            self.blend = (self.blend + dt / BLEND).min(1.0);
            if self.blend >= 1.0 {
                self.previous = None;
            }
        }
    }

    /// How dark the page is, from how long nothing has happened.
    pub fn dim(&self, input_age: f32) -> f32 {
        let over = (input_age - DIM_AFTER).max(0.0);
        (over / 60.0).min(1.0) * DIM
    }

    /// Everything to draw.
    pub fn frame(&self, input: &Input) -> Frame {
        let mut cards = Vec::with_capacity(8192);
        let mut blocks = Vec::with_capacity(32);

        // The old page rises and fades while the new one arrives from below.
        if let Some(previous) = &self.previous {
            let fade = 1.0 - self.blend;
            for card in &previous.text {
                cards.push(lift(card, self.blend * 1.5, card.alpha * fade));
            }
        }
        let arrive = if self.previous.is_some() {
            self.blend
        } else {
            1.0
        };
        for card in &self.page.text {
            cards.push(lift(card, (1.0 - arrive) * -1.5, card.alpha * arrive));
        }
        for token in &self.page.tokens {
            token.cards(arrive, &mut cards);
            // The shadow keeps a lifted token from looking pasted on.
            cards.push(Card {
                ch: ' ',
                style: 0,
                pos: [token.pos[0], token.pos[1], 0.02],
                yaw: 0.0,
                pitch: 0.0,
                roll: 0.0,
                size: [token.width() * 1.3, 0.3],
                alpha: 0.22 * (1.0 - token.pos[2] * 0.5).clamp(0.0, 1.0),
                kind: CardKind::Dot,
            });
        }
        for mote in &self.page.motes {
            let wobble = (self.time * 0.6 + mote.phase).sin();
            cards.push(Card {
                ch: ' ',
                style: 0,
                pos: [
                    mote.pos[0] + wobble * mote.drift * 0.3,
                    mote.pos[1] + (self.time * 0.4 + mote.phase).cos() * mote.drift * 0.2,
                    mote.pos[2],
                ],
                yaw: 0.0,
                pitch: 0.0,
                roll: 0.0,
                size: [mote.size, mote.size],
                alpha: 0.28 * arrive,
                kind: CardKind::Dot,
            });
        }

        // A soft shadow under the kitten, which is what makes a box look like it
        // is standing on the page rather than floating over it.
        let squash = 1.0 - self.kitten.hop * 0.12;
        cards.push(Card {
            ch: ' ',
            style: 0,
            pos: [self.kitten.pos[0], self.kitten.pos[1] - 0.05, 0.02],
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            size: [1.9 * squash, 0.5 * squash],
            alpha: 0.3 - self.kitten.hop * 0.1,
            kind: CardKind::Dot,
        });
        blocks.extend(self.kitten.blocks());

        Frame {
            cards,
            blocks,
            camera: self.camera,
            dim: self.dim(input.input_age),
            label: self.page.label(),
            asleep: self.asleep(),
        }
    }
}

/// A card moved up by `lift` and drawn at `alpha`.
fn lift(card: &Card, lift: f32, alpha: f32) -> Card {
    Card {
        pos: [card.pos[0], card.pos[1] + lift, card.pos[2]],
        alpha: alpha.clamp(0.0, 1.0),
        ..*card
    }
}

/// A small deterministic generator: xorshift64*, seeded.
///
/// The same reasoning as `nn_core::lines`'s seeded shuffles: a generator whose
/// seed is passed in makes a run reproducible, and one that reads the clock
/// inside cannot be tested.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// A generator for `seed`.
    pub fn new(seed: u64) -> Self {
        Self {
            state: (seed ^ 0x2545_F491_4F6C_DD1D) | 1,
        }
    }

    /// The next 64 bits.
    pub fn bits(&mut self) -> u64 {
        let mut state = self.state;
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        self.state = state;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number in `0..1`.
    pub fn above(&mut self) -> f64 {
        // 53 bits is what an f64 can hold exactly.
        (self.bits() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A number in `low..high`.
    pub fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.above() as f32
    }

    /// A number in `0..up_to`, never `up_to`.
    pub fn below(&mut self, up_to: usize) -> usize {
        if up_to == 0 {
            return 0;
        }
        (self.bits() % up_to as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> Playground {
        Playground::new(0x5EED)
    }

    fn input(age: f32) -> Input {
        Input {
            input_age: age,
            ..Input::default()
        }
    }

    #[test]
    fn a_frame_draws_a_page_of_code_and_a_kitten() {
        let playground = open();
        let frame = playground.frame(&Input::default());
        assert!(frame.cards.len() > 700, "{}", frame.cards.len());
        assert!(frame.blocks.len() > 12, "{}", frame.blocks.len());
        assert!(frame.label.starts_with(&playground.page().flavour.name().to_string()));
        assert_eq!(frame.dim, 0.0);
        assert!(!frame.asleep);
        // The page is the whole window: a card at every row of it.
        assert!(playground.page().text.len() > playground.page().rows as usize * 10);
    }

    #[test]
    fn the_kitten_is_two_to_three_lines_tall() {
        let playground = open();
        let height = playground.kitten().height();
        assert!((2.0..3.0).contains(&height), "{height} lines");
    }

    #[test]
    fn the_same_seed_opens_the_same_page() {
        let a = open();
        let b = open();
        assert_eq!(a.page().flavour, b.page().flavour);
        assert_eq!(a.page().seed, b.page().seed);
        assert_eq!(
            a.frame(&Input::default()).cards.len(),
            b.frame(&Input::default()).cards.len()
        );
    }

    #[test]
    fn two_seeds_open_different_pages() {
        let a = Playground::new(1);
        let b = Playground::new(2);
        assert!(a.page().flavour != b.page().flavour || a.page().seed != b.page().seed);
    }

    #[test]
    fn the_page_is_rebuilt_every_five_minutes() {
        let mut playground = open();
        let mut rebuilt = 0;
        for _ in 0..(60 * (REBUILD_AFTER as usize + 2)) {
            playground.step(1.0 / 60.0, &Input::default());
            if playground.page().index as usize > rebuilt {
                rebuilt = playground.page().index as usize;
            }
        }
        assert_eq!(rebuilt, 1, "one rebuild in five minutes");
        assert!(playground.time() >= REBUILD_AFTER);
        // The label counts from one, and the material is chosen again.
        assert!(playground.page().label().ends_with("page 2"));
    }

    #[test]
    fn a_rebuild_crosses_over_instead_of_cutting() {
        let mut playground = open();
        // Just past the rebuild.
        for _ in 0..(60 * (REBUILD_AFTER as usize + 1)) {
            playground.step(1.0 / 60.0, &Input::default());
        }
        let frame = playground.frame(&Input::default());
        assert!(frame.cards.len() > 1200, "both pages are drawn");
        // Then it finishes and the old page is gone.
        for _ in 0..(60 * 3) {
            playground.step(1.0 / 60.0, &Input::default());
        }
        let after = playground.frame(&Input::default());
        assert!(after.cards.len() < frame.cards.len());
    }

    #[test]
    fn a_new_window_shape_lays_the_page_out_again() {
        let mut playground = open();
        let before = playground.page().text.len();
        let wide = Input {
            grid: [200, 60],
            ..Input::default()
        };
        playground.step(1.0 / 60.0, &wide);
        assert_eq!(playground.page().columns, 200);
        assert_eq!(playground.page().rows, 60);
        assert!(playground.page().text.len() > before / 2, "a page, not a stub");
        // The kitten is still on it.
        let kitten = playground.kitten();
        assert!(kitten.pos[0] > 0.0 && kitten.pos[0] < playground.page().width());
        assert!(kitten.pos[1] > 0.0 && kitten.pos[1] < 60.0);
    }

    #[test]
    fn the_kitten_falls_asleep_after_three_minutes_and_the_page_dims_after_four() {
        let mut playground = open();
        for _ in 0..(60 * 200) {
            playground.step(1.0 / 60.0, &input(kitten::SLEEP_AFTER + 1.0));
        }
        assert!(playground.asleep());
        assert!(playground.dim(0.0) == 0.0);
        assert!(playground.dim(DIM_AFTER + 30.0) > 0.1);
        assert!(playground.dim(DIM_AFTER + 300.0) <= DIM + 1e-3);
    }

    #[test]
    fn a_long_frame_is_clamped_so_nothing_tunnels_through_the_page() {
        let mut playground = open();
        for token in &mut playground.page.tokens {
            token.pos[2] = 6.0;
            token.rest = false;
        }
        // A second and a half of wall clock, which is a stalled window.
        playground.step(1.5, &Input::default());
        for token in &playground.page().tokens {
            assert!(token.pos[2] >= 0.0, "fell through: {token:?}");
        }
    }

    #[test]
    fn the_pointer_reaches_the_kitten_as_a_place_on_the_page() {
        let mut playground = open();
        let target = Input {
            pointer: Some([0.5, -0.8]),
            pointer_age: 0.0,
            input_age: 0.0,
            ..Input::default()
        };
        let place = Camera::default()
            .page(Some([0.5, -0.8]))
            .expect("the page");
        let before = playground.kitten().pos[0];
        for _ in 0..(120 * 4) {
            playground.step(1.0 / 120.0, &target);
        }
        let kitten = playground.kitten();
        assert!(kitten.pos[0] > before, "{:?} toward {place:?}", kitten.pos);
    }

    #[test]
    fn the_same_frames_give_the_same_world() {
        let run = || {
            let mut playground = open();
            for frame in 0..(60 * 8) {
                let input = Input {
                    pointer: Some([(frame as f32 * 0.03).sin(), -0.7]),
                    pointer_age: 0.0,
                    input_age: 0.0,
                    ..Input::default()
                };
                playground.step(1.0 / 60.0, &input);
            }
            playground.frame(&Input::default())
        };
        let a = run();
        let b = run();
        assert_eq!(a.cards.len(), b.cards.len());
        assert_eq!(a.blocks, b.blocks);
        assert_eq!(a.cards[100], b.cards[100]);
    }

    #[test]
    fn a_token_never_leaves_the_page_after_an_afternoon_of_batting() {
        let mut playground = open();
        let mut rng = Rng::new(77);
        for _ in 0..(60 * 120) {
            let pointer = Some([rng.range(-0.9, 0.9), rng.range(-0.95, -0.5)]);
            playground.step(
                1.0 / 60.0,
                &Input {
                    pointer,
                    pointer_age: 0.0,
                    input_age: 0.0,
                    ..Input::default()
                },
            );
            for token in &playground.page().tokens {
                assert!(token.pos[2] >= 0.0, "{token:?}");
                assert!(token.pos[0] > 0.0);
                assert!(token.pos[0] < playground.page().width());
                assert!(token.pos[1] > 0.0);
                assert!(token.pos[1] < playground.page().rows as f32);
                assert!(token.pos[2] < 6.0);
            }
        }
    }

    #[test]
    fn the_random_generator_is_reproducible_and_bounded() {
        let mut a = Rng::new(5);
        let mut b = Rng::new(5);
        for _ in 0..100 {
            assert_eq!(a.bits(), b.bits());
        }
        let mut rng = Rng::new(6);
        for _ in 0..1000 {
            let value = rng.range(-2.0, 3.0);
            assert!((-2.0..3.0).contains(&value), "{value}");
            assert!(rng.below(4) < 4);
        }
        // Zero is a fixed point of a plain xorshift, and a seed of zero must not
        // produce an unending zero.
        let mut zero = Rng::new(0);
        assert!(zero.bits() != 0);
    }

    #[test]
    fn the_dim_never_exceeds_its_bound_and_never_goes_negative() {
        let playground = open();
        assert_eq!(playground.dim(0.0), 0.0);
        assert_eq!(playground.dim(DIM_AFTER), 0.0);
        assert!(playground.dim(DIM_AFTER + 40.0) > 0.0);
        assert!(playground.dim(1e6) <= DIM);
    }

    #[test]
    fn a_token_that_comes_to_rest_has_a_shadow_under_it() {
        let playground = open();
        let frame = playground.frame(&Input::default());
        let shadows = frame
            .cards
            .iter()
            .filter(|card| card.kind == CardKind::Dot)
            .count();
        assert!(shadows >= playground.page().tokens.len());
    }
}

