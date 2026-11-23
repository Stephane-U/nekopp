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
pub mod spark;

use camera::Camera;
use kitten::{Effect, Kitten, Mood, Senses};
use scene::{Page, GLYPH_WIDTH};

pub use corpus::Flavour;
pub use scene::{Floating, Mote, Shelf, Token};
pub use spark::{Glow, GlowKind, Spark};

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
/// How far she flies between two pieces of her trail, in cells.
///
/// Laid by distance rather than by time, because a trail is a line in space: a
/// piece every few hundredths of a second would pile into a white column when she
/// pounces in place and thin into a dotted line when she falls off a ledge. At this
/// spacing the pieces overlap enough to be a band and the hue of one is still a
/// colour.
const TRAIL_STEP: f32 = 0.42;
/// How far the trail's hue moves from one piece to the next.
///
/// A jump is a dozen pieces, so this is one turn of the wheel per jump: red at the
/// tail, violet at her back, in the order a rainbow has them.
const TRAIL_HUE: f32 = 0.09;
/// How fast the colour wheel turns for the whole mode, in turns per second.
///
/// Slow, because the point of the wheel is that two jumps in a row are different
/// rainbows, not that the page strobes.
const HUE_RATE: f32 = 0.11;

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
    /// The page's light: trail, rings, confetti, drawn additively.
    pub glows: Vec<Glow>,
    /// Seconds since the mode opened, for anything the renderer shimmers with the
    /// clock -- a rainbow that drifts rather than one that is painted on.
    pub time: f32,
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
    /// Where the trail's colour is on the wheel, and how many cells she has flown
    /// since its last piece.
    trail_hue: f32,
    trail_clock: f32,
    /// Whether she was in the air last frame, so taking off can be seen as an
    /// event rather than as a state.
    was_air: bool,
    /// The same for running flat out, so a dash has a beginning and an end.
    was_dash: bool,
    /// The same for raking, which is either of the fast moods: the frames a run
    /// begins and ends are the frames the page is hit as a whole.
    was_raking: bool,
    /// Pieces of trail dropped since she left the page, so every fourth one can be
    /// a star.
    trail_count: u32,
    /// Where her middle was last frame, so this frame knows the path she took.
    last_at: [f32; 2],
    /// How fast her middle is moving, in cells per second.
    vel: [f32; 2],
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
        // Each visit starts somewhere else on the colour wheel, so two cats on two
        // days do not jump in the same rainbow.
        let trail_hue = rng.above() as f32;
        // Her middle as it stands, so the first frame does not read the whole page
        // as a jump from the corner.
        let last_at = [kitten.pos[0], kitten.pos[1] + kitten.hop + 1.0];
        Self {
            page,
            previous: None,
            blend: 1.0,
            kitten,
            camera,
            rng,
            time: 0.0,
            seed,
            trail_hue,
            trail_clock: 0.0,
            was_air: false,
            was_dash: false,
            was_raking: false,
            trail_count: 0,
            last_at,
            vel: [0.0; 2],
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
            props: &self.page.props,
            tokens: &self.page.tokens,
            motes: &self.page.motes,
            shelves: &self.page.shelves,
            floor: self.page.floor(),
            rng: &mut self.rng,
        };
        let strike = self.kitten.think(dt, &mut senses);
        // A paw that swings takes what is under it; a cat that lands only shakes the
        // page and raises dust. Landing on a line and taking the line out from under
        // herself is a page a cat cannot stand on.
        if strike {
            let paw = self.kitten.paw();
            self.page.strike(paw, &mut self.rng);
        } else if self.kitten.landed {
            let paw = self.kitten.paw();
            self.page.land(paw, &mut self.rng);
        }
        // A swoop comes down the way something thrown does. Every other landing on the
        // page only shakes it, because a cat that took the line it stood on would have
        // nowhere to stand -- but a swoop is the end of a flight the wall was climbed for,
        // and what it is for is the damage. The line under her paws gives way, the words
        // in it are in the air, and the furniture within reach of her sheds.
        if self.kitten.landed && self.kitten.swooping() {
            let feet = [self.kitten.pos[0], self.kitten.pos[1] - 0.5, 0.0];
            let torn = self.page.blast(feet, 4.0, 16.0, &mut self.rng);
            self.page.puff(feet, 12, &mut self.rng);
            self.page.ring(feet, self.trail_hue, 0.8);
            self.page.ring(feet, (self.trail_hue + 0.5) % 1.0, 0.55);
            self.page
                .confetti(feet, self.trail_hue, 16 + torn, 6.2, &mut self.rng);
            self.page.chip(feet, 3.0, &mut self.rng);
        }
        // A cat at a gallop does not swing at one thing, it takes everything it
        // passes: the page is asked about a place rather than about a paw, and the
        // place moves with her, which is what draws a line of damage across the
        // page instead of a hole in it.
        if let Some(at) = self.kitten.rake {
            let heading = if self.kitten.yaw < 0.0 { -1.0 } else { 1.0 };
            if self.page.rake(at, heading, &mut self.rng) > 0 {
                // Two specks, not one: what a run kicks up is a spray along the
                // furrow rather than a puff at the end of it.
                self.page.dust(at, self.trail_hue, 0.3, &mut self.rng);
                self.page.dust(at, self.trail_hue, 0.24, &mut self.rng);
                // A little confetti in the furrow, often enough to read as debris
                // and not so often that the page is fireworks.
                if self.rng.below(2) == 0 {
                    self.page.confetti(at, self.trail_hue, 3, 3.4, &mut self.rng);
                }
                // Furniture in the way of a run is worth the page saying so: a
                // table that only shakes for a paw slamming into it at eight cells
                // a second is a table nobody believes in. It sheds, too -- the box
                // keeps its shape and loses the characters it was made of.
                if self.page.bump(at, 2.0, 1.9) {
                    let shed = self.page.chip(at, 1.8, &mut self.rng);
                    self.page.ring(at, (self.trail_hue + 0.5) % 1.0, 0.55);
                    self.page.confetti(
                        at,
                        self.trail_hue,
                        10 + shed * 2,
                        4.6,
                        &mut self.rng,
                    );
                }
            }
        }
        // A cat going up the page takes the page with it. The claws are in the line it
        // is leaving -- the one she is going to has to stay, or the climb has nowhere to
        // land -- and what is in that line comes away, a word at a time, falling. The
        // furniture does not survive a wall of them either.
        if let Some(at) = self.kitten.claw {
            if self.page.claw(at, &mut self.rng) > 0 {
                self.page.dust(at, self.trail_hue, 0.3, &mut self.rng);
                self.page
                    .dust(at, (self.trail_hue + 0.5) % 1.0, 0.22, &mut self.rng);
                if self.rng.below(2) == 0 {
                    self.page.confetti(at, self.trail_hue, 4, 3.0, &mut self.rng);
                }
            }
            if self.page.bump(at, 2.4, 1.9) {
                let shed = self.page.chip(at, 2.4, &mut self.rng);
                self.page.ring(at, (self.trail_hue + 0.5) % 1.0, 0.55);
                self.page
                    .confetti(at, self.trail_hue, 8 + shed * 2, 4.6, &mut self.rng);
            }
        }
        // The top of the page is the end of a climb: she has got as high as there is to
        // go, and it is worth a beat of the page's own light. Nothing is taken from the
        // line she is standing on -- the only ledge left up here is the one holding her.
        if self.kitten.topped {
            let feet = [self.kitten.pos[0], self.kitten.pos[1] - 0.5, 0.0];
            self.page.ring(feet, self.trail_hue, 0.8);
            self.page
                .ring(feet, (self.trail_hue + 0.5) % 1.0, 0.55);
            self.page.puff(feet, 10, &mut self.rng);
            self.page
                .confetti(feet, (self.trail_hue + 0.5) % 1.0, 24, 6.5, &mut self.rng);
            self.page.chip(feet, 4.0, &mut self.rng);
        }
        match self.kitten.effect {
            Some(Effect::Zzz) => {
                let at = self.kitten.crown();
                self.page.float('z', 7, at, &mut self.rng);
            }
            Some(Effect::Lick) => {
                let at = self.kitten.muzzle();
                self.page.float('*', 5, at, &mut self.rng);
            }
            None => {}
        }
        self.emote(dt, strike);

        // Fixed steps, so the same sequence of frames always gives the same
        // tokens, the same dust, and the same furniture shaking itself still.
        let mut left = dt;
        while left > 1e-5 {
            let step = left.min(1.0 / 120.0);
            self.page.step_tokens(step);
            self.page.step_motes(step);
            self.page.step_props(step);
            self.page.step_floating(step);
            self.page.step_sparks(step);
            // The page mends itself at the same cadence as everything else that
            // moves on it: what a run takes away comes back, so she can take it
            // away again.
            self.page.mend(step, &mut self.rng);
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
            // A new page is a new rainbow: the wheel moves on with the material so
            // the fifth minute does not look like the first.
            self.trail_hue = (self.trail_hue + 0.37) % 1.0;
            self.was_air = false;
            self.was_dash = false;
            self.trail_clock = 0.0;
            self.trail_count = 0;
            self.last_at = [
                self.kitten.pos[0],
                self.kitten.pos[1] + self.kitten.hop + 1.0,
            ];
        }
        if self.blend < 1.0 {
            self.blend = (self.blend + dt / BLEND).min(1.0);
            if self.blend >= 1.0 {
                self.previous = None;
            }
        }
    }

    /// How many pieces of trail this frame owes, given how far she flew, and where
    /// along the frame's path each of them belongs.
    ///
    /// The clock is a distance and it carries over: one piece every [`TRAIL_STEP`]
    /// cells, so a frame that flew three steps lays three pieces instead of one.
    /// Reset to a step each time, a trail thins out as frames get longer, and the
    /// light behind a jump would depend on the renderer rather than on the jump.
    ///
    /// Each answer is a fraction of the path this frame, 0 at the start of it and 1
    /// at the end, which is where the piece is placed.
    fn due_trail(&mut self, flown: f32) -> Vec<f32> {
        let mut due = Vec::new();
        self.trail_clock -= flown;
        while self.trail_clock <= 0.0 && due.len() < 8 {
            due.push(1.0 - (-self.trail_clock / flown.max(1e-4)).clamp(0.0, 1.0));
            self.trail_clock += TRAIL_STEP;
        }
        due
    }

    /// What the page answers, per frame: a rainbow behind a jump, a ring where she
    /// comes down, confetti where a paw takes something.
    ///
    /// The effects are driven from the simulation's own events rather than from the
    /// renderer, so a still frame at the same time has the same sparks as the run
    /// that produced it, and a test can count them without a window.
    fn emote(&mut self, dt: f32, strike: bool) {
        let air = self.kitten.airborne;
        // The cat's middle, which is where a trail comes from: her paws are on the
        // page and what the reader follows across a jump is the whole animal, so the
        // light is at her body and not at the shadow she left.
        let middle = self.kitten.pos[1] + self.kitten.hop + 1.0;
        let at = [self.kitten.pos[0], middle, 0.5];
        let paw = self.kitten.paw();
        self.trail_hue = (self.trail_hue + dt * HUE_RATE) % 1.0;
        // How far she moved this frame, and how fast: the trail is laid along the
        // path she took, so a walk, a pounce and a fall off a ledge all put their
        // light in the same order.
        let moved = [at[0] - self.last_at[0], at[1] - self.last_at[1]];
        self.last_at = [at[0], at[1]];
        self.vel = if dt > 0.0 {
            [moved[0] / dt, moved[1] / dt]
        } else {
            [0.0, 0.0]
        };
        let flown = (moved[0] * moved[0] + moved[1] * moved[1]).sqrt();

        // Taking off and touching down are the two moments worth punctuating: the
        // page says where she left and where she arrived.
        if air && !self.was_air {
            self.page.confetti(paw, self.trail_hue, 10, 4.5, &mut self.rng);
        }
        if self.kitten.landed {
            self.page.ring(paw, self.trail_hue, 0.45);
            self.page
                .confetti(paw, (self.trail_hue + 0.5) % 1.0, 8, 3.2, &mut self.rng);
        }
        if strike {
            self.page
                .ring(paw, (self.trail_hue + 0.35) % 1.0, 0.3);
            self.page
                .confetti(paw, (self.trail_hue + 0.2) % 1.0, 18, 5.0, &mut self.rng);
        }
        // The two ends of a run. A cat that leaves the ground at a gallop pushes off
        // with everything it has, and a cat that stops from one puts the whole of
        // itself down; either way the page gives way in every direction at once
        // instead of losing the one word a paw would take. This is what makes a run an
        // event rather than a line drawn quietly across a page, and it happens twice
        // because a run has two ends.
        let raking = self.kitten.raking();
        if raking != self.was_raking {
            let feet = [self.kitten.pos[0], self.kitten.pos[1] - 0.5, 0.0];
            let torn = self.page.blast(feet, 2.8, 10.0, &mut self.rng);
            if torn > 0 {
                self.page.puff(feet, 12, &mut self.rng);
                self.page.ring(feet, (self.trail_hue + 0.5) % 1.0, 0.7);
                self.page
                    .confetti(feet, self.trail_hue, 18, 6.0, &mut self.rng);
            }
            self.page.chip(feet, 2.2, &mut self.rng);
        }
        self.was_raking = raking;

        // The trail: a piece of light every part of a cell she flies, each a step
        // around the wheel from the last, so a jump is a band of colour with its
        // bands in order rather than a cloud of one.
        if air {
            // A swoop is the long one, and the page says so. Its band is half again as
            // wide as a jump's and doubled, so what crosses the page is a ribbon rather
            // than a line; its colour is a whole turn of the wheel laid along the arc
            // instead of a step per piece, so one leg is one rainbow from the tail of it
            // to the landing; and it puts a star in every third piece rather than every
            // fourth. A jump is a hop with light behind it and a swoop is the light.
            let swoop = self.kitten.swooping();
            for fraction in self.due_trail(flown) {
                let hue = if swoop {
                    (self.trail_hue + self.kitten.jump * 1.15) % 1.0
                } else {
                    self.trail_hue = (self.trail_hue + TRAIL_HUE) % 1.0;
                    self.trail_hue
                };
                // A little scatter, so the band has a thickness: two sparks of one
                // colour exactly on the line read as a drawn stroke, the same two
                // off it read as light.
                let size = if swoop {
                    self.rng.range(1.0, 1.4)
                } else {
                    self.rng.range(0.7, 1.0)
                };
                let at = [
                    at[0] - moved[0] * (1.0 - fraction) + self.rng.range(-0.3, 0.3),
                    at[1] - moved[1] * (1.0 - fraction) + self.rng.range(-0.35, 0.35),
                    at[2],
                ];
                // Left going the other way to her: up while she drops, down while
                // she rises, so the band is a tail and not a ball.
                let vel = [
                    self.rng.range(-0.6, 0.6) - self.vel[0] * 0.45,
                    self.rng.range(0.0, 0.6) - self.vel[1] * 0.45,
                    0.0,
                ];
                self.page.ribbon(at, vel, hue, size);
                if swoop {
                    // A second band over the first, smaller and dimmer: a bow is thick
                    // in the middle and thin at its edges, and one row of blobs is a
                    // stroke rather than a bow.
                    self.page.ribbon(
                        [at[0], at[1] + 0.5, at[2]],
                        [vel[0] * 0.6, vel[1] * 0.6 + 0.2, 0.0],
                        (hue + 0.035) % 1.0,
                        size * 0.55,
                    );
                }
                // Every fourth piece puts a star in the ribbon, so the trail has
                // something in it that turns rather than only something that glows.
                self.trail_count = self.trail_count.wrapping_add(1);
                let stars = if swoop { 3 } else { 4 };
                if self.trail_count % stars == 0 {
                    self.page.twinkle(at, (hue + 0.02) % 1.0, 0.45);
                }
            }
            // The halo comes with her: its life is a tenth of a second and it is
            // pushed again every frame, so it is one light that follows the cat. It
            // is dim on purpose -- it is there to say she is lit, not to cover the
            // colours she has left behind.
            let halo = if swoop {
                (self.trail_hue + self.kitten.jump * 1.15) % 1.0
            } else {
                self.trail_hue
            };
            self.page.aura(at, halo, if swoop { 1.9 } else { 1.2 });
        } else if self.kitten.streaking() {
            // A run gets a trail of its own, on the paper rather than in the air, and
            // it is a different animal from the jump's rainbow -- a lap drags the same
            // one. The pieces are laid at her feet, thrown hard the way she came, and
            // the wheel turns per piece instead of per jump, so what follows her is a
            // streak. Laid a third again as far apart as a jump's pieces and smaller,
            // so the streak is a line: a run has twenty of these on screen at once,
            // and twenty blobs of the size of a leg is a cloud, not a speed.
            for fraction in self.due_trail(flown * 1.35) {
                self.trail_hue = (self.trail_hue + TRAIL_HUE) % 1.0;
                // At her paws and not above them: the light of a run belongs on the
                // line she is taking apart, and a streak at the height of her shoulders
                // would be drawn across the line above her.
                let feet = [
                    self.kitten.pos[0] - moved[0] * (1.0 - fraction),
                    self.kitten.pos[1] - 0.15,
                    0.35,
                ];
                // Nothing here is drawn at random: a streak is laid often enough
                // that a generator would only make it look noisy, and a page that
                // takes a number out of the generator per piece is a page whose
                // sparks depend on how many frames were rendered.
                self.trail_count = self.trail_count.wrapping_add(1);
                let lift = 0.12 + 0.24 * (self.trail_count % 5) as f32 * 0.25;
                self.page.ribbon(
                    feet,
                    [-self.vel[0] * 0.5, lift, 0.0],
                    self.trail_hue,
                    0.4 + (self.trail_count % 4) as f32 * 0.05,
                );
                // Dust off the paper, which is most of what a run looks like at
                // this size: the ribbons are the speed, the dust is the floor.
                if self.trail_count % 3 == 0 {
                    self.page.dust(feet, (self.trail_hue + 0.5) % 1.0, 0.28, &mut self.rng);
                }
            }
        }
        // A dash is bookended: something at the moment she goes and something at
        // the moment she stops, so the run has edges rather than fading in and out.
        let dash = self.kitten.dashing();
        if dash && !self.was_dash {
            self.page.ring(at, self.trail_hue, 0.55);
            self.page
                .confetti(at, (self.trail_hue + 0.3) % 1.0, 8, 5.4, &mut self.rng);
        }
        if !dash && self.was_dash {
            self.page.ring(at, (self.trail_hue + 0.5) % 1.0, 0.5);
        }
        self.was_dash = dash;
        self.was_air = air;
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
            for prop in &previous.props {
                prop.cards(self.blend * 1.5, fade, &mut cards);
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
        for prop in &self.page.props {
            prop.cards((1.0 - arrive) * -1.5, arrive, &mut cards);
        }
        // Characters the page is printing again, faint and in the cell they stood
        // in: a wrecked line is seen filling rather than seen being replaced.
        for card in &self.page.mending {
            cards.push(lift(card, (1.0 - arrive) * -1.5, card.alpha * arrive));
        }
        // Zzz and sparkles: the page's own glyphs, held up for a moment.
        for glyph in &self.page.floating {
            cards.push(Card {
                ch: glyph.ch,
                style: glyph.style,
                pos: glyph.pos,
                yaw: 0.0,
                pitch: 0.0,
                roll: 0.0,
                size: [GLYPH_WIDTH * glyph.size, glyph.size],
                alpha: glyph.alpha() * arrive,
                kind: CardKind::Glyph,
            });
        }
        for token in &self.page.tokens {
            token.cards(arrive, &mut cards);
            // A drop shadow, so a loose word sits on the paper rather than being
            // printed on it.
            cards.push(Card {
                ch: ' ',
                style: 0,
                pos: [token.pos[0], token.pos[1], 0.02],
                yaw: 0.0,
                pitch: 0.0,
                roll: 0.0,
                size: [token.width() * 1.3, 0.3],
                alpha: 0.20,
                kind: CardKind::Dot,
            });
        }
        for mote in &self.page.motes {
            // The simulation owns where a mote is, the page does not wobble it:
            // the kitten chases what is drawn, and a speck in a different place
            // from the one on screen is a cat pouncing at nothing.
            cards.push(Card {
                ch: ' ',
                style: 0,
                pos: mote.pos,
                yaw: 0.0,
                pitch: 0.0,
                roll: 0.0,
                size: [mote.size, mote.size],
                // Dust that a paw threw fades as its second runs out; the page's
                // own dust does not.
                alpha: 0.28 * arrive * (mote.life / 1.3).min(1.0),
                kind: CardKind::Dot,
            });
        }

        // A soft shadow under the kitten, which is what makes a box look like it
        // is standing on the page rather than floating over it. It falls on the
        // ledge under her rather than on her paws, so a jump reads as a jump: the
        // shadow stays down where the ground is while the cat goes up.
        let ground = self
            .page
            .surface(self.kitten.pos[0], self.kitten.pos[1] + self.kitten.hop);
        let height = (self.kitten.pos[1] + self.kitten.hop - ground).max(0.0);
        let squash = (1.0 - height * 0.12).max(0.35);
        cards.push(Card {
            ch: ' ',
            style: 0,
            pos: [self.kitten.pos[0], ground - 0.05, 0.02],
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            size: [1.9 * squash, 0.5 * squash],
            alpha: (0.3 - height * 0.05).max(0.05),
            kind: CardKind::Dot,
        });
        blocks.extend(self.kitten.blocks());

        Frame {
            cards,
            blocks,
            glows: self.page.glows(arrive),
            time: self.time,
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
    fn a_jump_leaves_a_rainbow_behind_it() {
        let mut playground = open();
        let input = Input::default();
        let mut colours = [false; 6];
        let mut trail = 0;
        let mut rings = 0;
        let mut air = false;
        for _ in 0..(120.0 / 0.05) as usize {
            playground.step(0.05, &input);
            if playground.kitten().airborne {
                air = true;
            }
            for glow in playground.frame(&input).glows {
                match glow.kind {
                    GlowKind::Ring => rings += 1,
                    // The halo is two cells across; a piece of trail is one.
                    GlowKind::Blob if glow.size < 1.2 => {
                        trail += 1;
                        let bucket = (glow.hue * 6.0) as usize % 6;
                        colours[bucket] = true;
                    }
                    _ => {}
                }
            }
        }
        assert!(air, "she never left the ground");
        assert!(trail > 30, "{trail} pieces of trail");
        assert!(rings > 0, "no landing rang the page");
        let spread = colours.iter().filter(|seen| **seen).count();
        assert!(spread >= 4, "{spread} colours on the wheel: {colours:?}");
    }

    #[test]
    fn a_swoop_lays_a_rainbow_across_the_page() {
        // What the wall is climbed for: leaving the top of it lays a band a whole turn of
        // the wheel long, across the page rather than along it, and the landing at the end
        // of a leg is the one landing on the page that takes the line with it.
        let mut playground = open();
        let input = Input::default();
        let mut swoops = 0;
        let mut in_one = false;
        let mut left = f32::MAX;
        let mut right = f32::MIN;
        let mut widest = 0.0f32;
        let mut colours = [false; 6];
        let mut spread = [false; 6];
        let mut torn = 0;
        let mut chars = playground.page().text.len();
        for _ in 0..(120 * 300) {
            let swoop = playground.kitten().swooping();
            if swoop && !in_one {
                in_one = true;
                swoops += 1;
                left = f32::MAX;
                right = f32::MIN;
            }
            if !swoop && in_one {
                in_one = false;
                widest = widest.max(right - left);
                for (seen, here) in colours.iter_mut().zip(spread) {
                    *seen |= here;
                }
                spread = [false; 6];
            }
            playground.step(1.0 / 120.0, &input);
            let now = playground.page().text.len();
            if playground.kitten().swooping() {
                torn += chars.saturating_sub(now);
            }
            chars = now;
            if playground.kitten().swooping() {
                for glow in playground.frame(&input).glows {
                    // The band of a swoop is a cell wide or wider; the halo that follows
                    // her is a light rather than a colour, so it is not counted.
                    if glow.kind != GlowKind::Blob || !(1.0..1.8).contains(&glow.size) {
                        continue;
                    }
                    left = left.min(glow.pos[0]);
                    right = right.max(glow.pos[0]);
                    let bucket = (glow.hue * 6.0) as usize % 6;
                    spread[bucket] = true;
                }
            }
        }
        assert!(swoops > 0, "she never left the top of anything");
        // A jump is two or three cells of light. This is a band the width of the page.
        assert!(
            widest > 30.0,
            "the longest rainbow laid was {widest} cells of a page {}",
            playground.page().width()
        );
        let wheel = colours.iter().filter(|seen| **seen).count();
        assert!(wheel >= 5, "{wheel} of six colours on one leg");
        assert!(torn > 10, "{torn} characters came off under a landing");
    }

    #[test]
    fn the_trail_lies_along_the_path_and_not_along_the_clock() {
        // The same light at two frame rates: a piece every part of a cell she flies,
        // so the pieces a jump leaves are the distance divided by the step, whatever
        // the frame rate. Laid per frame instead, a slow renderer would draw a dotted
        // line and a fast one a smear. The measure is density rather than a count,
        // because two runs of the same seed do not have the same jumps: what has to
        // hold is the rule, not the run.
        let density = |dt: f32| {
            let mut playground = open();
            let input = Input::default();
            let mut flown = 0.0f32;
            let mut pieces = 0;
            for _ in 0..(60.0 / dt) as usize {
                let was = playground.kitten().pos;
                let was_hop = playground.kitten().hop;
                let air = playground.kitten().airborne;
                playground.step(dt, &input);
                // A swoop lays two bands where a jump lays one, on purpose: what this
                // measures is the rule a jump follows, so the long leaps are left out of
                // it rather than averaged in.
                if !air || playground.kitten().swooping() {
                    continue;
                }
                let now = playground.kitten().pos;
                flown += (now[0] - was[0]).hypot(now[1] + playground.kitten().hop - was[1] - was_hop);
                // A piece made this frame is the one with no age on it beyond the
                // frame itself: the page steps its sparks after the frame they were
                // made in, and the list is capped, so an index taken before the step
                // would be a lie.
                pieces += playground
                    .page()
                    .sparks
                    .iter()
                    .filter(|spark| {
                        spark.age <= dt * 1.05
                            && spark.kind == GlowKind::Blob
                            && spark.grow > 0.0
                            && spark.pull > 0.0
                    })
                    .count();
            }
            assert!(flown > 4.0, "{flown} cells in the air");
            pieces as f32 / flown
        };
        let quick = density(1.0 / 120.0);
        let slow = density(1.0 / 30.0);
        let wanted = 1.0 / TRAIL_STEP;
        // Scatter lands two pieces of a step in the same place, so the density is
        // allowed to be under the exact one, never over it by much.
        assert!((0.75..=1.15).contains(&(quick / wanted)), "{quick} pieces a cell at a hundred and twenty frames a second");
        assert!((0.75..=1.15).contains(&(slow / wanted)), "{slow} pieces a cell at thirty");
        assert!(
            (quick - slow).abs() <= quick.max(slow) * 0.2,
            "{quick} pieces a cell against {slow}"
        );
    }

    #[test]
    fn a_dash_leaves_a_trail_and_a_line_full_of_holes() {
        // A dash is the one mood that damages the page on its own: it takes the
        // line it runs along, a rake at a time, and it drags a streak of light the
        // length of the run. Both are the page answering her, so both are counted
        // here rather than looked at.
        let mut playground = open();
        let input = Input::default();
        let mut dashing = 0;
        let mut trail = 0;
        let mut rings = 0;
        let mut taken = 0;
        for _ in 0..(120 * 240) {
            let before = playground.page().text.len();
            playground.step(1.0 / 120.0, &input);
            let after = playground.page().text.len();
            if !playground.kitten().dashing() {
                continue;
            }
            dashing += 1;
            taken += before.saturating_sub(after);
            for glow in playground.frame(&input).glows {
                match glow.kind {
                    GlowKind::Ring => rings += 1,
                    GlowKind::Blob if glow.size < 1.0 => trail += 1,
                    _ => {}
                }
            }
        }
        assert!(dashing > 60, "{dashing} frames at a run");
        assert!(trail > 20, "{trail} pieces of trail behind a run");
        assert!(rings > 0, "a run began or ended without a ring");
        assert!(taken >= 4, "{taken} characters came off the page");
    }

    #[test]
    fn a_page_under_a_cat_that_never_stops_is_never_emptied() {
        // The mode is a playground and not a document, and the one thing a playground
        // cannot do is run out. Five minutes of the busiest animal this mode has is what
        // this costs: the page is taken apart and printed again, and at no point is it a
        // page of gaps with a cat walking about in it.
        let mut playground = open();
        let input = Input::default();
        let floor = playground.page().rows as usize * 10;
        let mut least = playground.page().text.len();
        let mut printed = false;
        for _ in 0..(120 * 300) {
            playground.step(1.0 / 120.0, &input);
            assert!(
                playground.page().text.len() > floor,
                "the page was down to {} characters",
                playground.page().text.len()
            );
            least = least.min(playground.page().text.len());
            printed |= !playground.page().mending.is_empty();
        }
        assert!(printed, "nothing was ever coming back");
        assert!(least > floor);
    }

    #[test]
    fn the_end_of_a_run_blasts_a_hole_in_the_page() {
        // A run does not stop, it arrives: the last thing a dash or a lap does is put
        // the whole animal down among the characters it was chasing. A rake takes two or
        // three at once, so a frame that loses ten is a landing and not a furrow.
        let mut playground = open();
        let input = Input::default();
        let mut biggest = 0;
        let mut holes = 0;
        for _ in 0..(120 * 240) {
            let before = playground.page().text.len();
            playground.step(1.0 / 120.0, &input);
            let lost = before.saturating_sub(playground.page().text.len());
            biggest = biggest.max(lost);
            if lost >= 8 {
                holes += 1;
            }
        }
        assert!(holes > 0, "no run ever ended in a hole, biggest loss {biggest}");
    }

    #[test]
    fn a_climb_goes_up_the_page_and_takes_it_apart() {
        // Through the whole simulation rather than the animal on its own, because the
        // claws are the page's business. What an ascent should leave behind is a cat
        // near the top of the page and a furrow of characters missing on the way up it.
        let mut playground = open();
        let input = Input::default();
        let rows = playground.page().rows as f32;
        let mut highest = 0.0f32;
        let mut clawed = 0;
        let mut chars = playground.page().text.len();
        // Five minutes rather than two, because the page now has a wall of furniture up
        // the right of it and climbs of that take nothing off the code: an ascent of the
        // text happens when she is over the text, and that is a matter of where her
        // wandering has put her.
        for _ in 0..(120 * 300) {
            playground.step(1.0 / 120.0, &input);
            highest = highest.max(playground.kitten().pos[1]);
            let now = playground.page().text.len();
            if playground.kitten().mood == Mood::Climb {
                clawed += chars.saturating_sub(now);
            }
            chars = now;
        }
        assert!(
            highest > rows * 0.6,
            "she got to {highest} of a page {rows} lines high"
        );
        assert!(clawed > 10, "{clawed} characters came off the page as she climbed");
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
    fn a_long_frame_keeps_a_batted_token_on_the_page() {
        let mut playground = open();
        for token in &mut playground.page.tokens {
            token.vel = [14.0, 14.0];
            token.rest = false;
        }
        // A second and a half of wall clock, which is a stalled window.
        playground.step(1.5, &Input::default());
        for token in &playground.page().tokens {
            assert!(token.pos[0] > 0.0, "{token:?}");
            assert!(token.pos[0] < playground.page().width(), "{token:?}");
            assert!(token.pos[1] > 0.0, "{token:?}");
            assert!(token.pos[1] < playground.page().rows as f32, "{token:?}");
        }
    }

    #[test]
    fn the_pointer_reaches_the_kitten_as_a_place_on_the_page() {
        let mut playground = open();
        // A pointer that arrived and then held still, which is what a reader does with
        // one: a pointer that reports itself as still moving is a cat pouncing at it for
        // ever, and what this asks about is the walk to it.
        let target = Input {
            pointer: Some([0.5, -0.8]),
            pointer_age: 1.0,
            input_age: 0.0,
            ..Input::default()
        };
        let place = Camera::default()
            .page(Some([0.5, -0.8]))
            .expect("the page");
        let before = playground.kitten().pos[0];
        for _ in 0..(120 * 8) {
            playground.step(1.0 / 120.0, &target);
        }
        let kitten = playground.kitten();
        assert!(
            (kitten.pos[0] - place[0]).abs() < 3.0,
            "{:?} toward {place:?} from {before}",
            kitten.pos
        );
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
                assert!(token.pos[0] > 0.0);
                assert!(token.pos[0] < playground.page().width());
                assert!(token.pos[1] > 0.0);
                assert!(token.pos[1] < playground.page().rows as f32);
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
