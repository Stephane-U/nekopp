//! The page the kitten walks on.
//!
//! Playground mode is a document, not a diorama. The window is filled with a page
//! of code at the editor's own line height -- a gutter of line numbers, then lines
//! of the chosen flavour -- and the kitten is two and a quarter lines tall, walking
//! across it. Everything that is not the kitten is made of characters: a table, a
//! plant, a lamp and a shelf are ASCII drawn into the page, the bottom rows are a
//! rug of punctuation, and the tokens the kitten bats are words lifted off the
//! text. There is nothing to install and nothing to render as a surface, which is
//! what keeps the mode a page rather than a place.
//!
//! Two kinds of thing on the page can be *done to*, and both are addressed by
//! position rather than by name. Furniture has a box the kitten can stand on or
//! swat, and it answers: a bump sets it shaking, and a shaking prop is drawn
//! swaying by its own phase until the wobble decays. Dust has a velocity and a
//! life: the ambient motes wander for ever, a swat throws short-lived ones, and a
//! paw that lands among them throws them away. Interaction is the page's own
//! bookkeeping, so it can be tested without a window: [`Page::strike`] is the whole
//! vocabulary of a paw landing.
//!
//! Underneath all of it the page has ledges. Every run of code on a row is a
//! [`Shelf`] with a top edge, the rug is the floor, and the furniture's top is a
//! ledge of its own; a word knocked off a line falls to the nearest ledge under it
//! and stays there. That is how the page has gravity without turning into a heap
//! of letters along the bottom: the lines catch what falls. And a paw that lands
//! on a line takes characters out of it -- [`Page::knock_out`] lifts them into
//! tokens -- so the cat leaves gaps in the code she has been playing in.
//!
//! Coordinates: one unit is one line of text. `x` runs right from the left edge,
//! `y` runs up the page from the bottom edge, and `z` comes out of the page towards
//! the reader, which is where the kitten's boxes and a lifted token live. Row 0 is
//! the top of the page, so a cell at row `r` has its centre at
//! `y = rows - r - 0.5`.

use std::collections::BTreeMap;

use super::corpus::Flavour;
use super::spark::{Glow, Spark, MAX_SPARKS};
use super::{Card, CardKind, Rng};

/// How wide one glyph card is, in line heights.
///
/// Monospace cells are about 0.55 of the line height in every font this editor
/// ships with; the app rasterises the atlas with the same ratio, so a card and a
/// cell are the same shape.
pub const GLYPH_WIDTH: f32 = 0.55;

/// The columns the line numbers take, including the gap after them.
const GUTTER: u32 = 4;
/// How many rows at the bottom of the page are rug rather than code.
const RUG_ROWS: u32 = 5;
/// The rows a piece of furniture needs under it, so nothing stands on the rug.
const FLOOR_MARGIN: u32 = RUG_ROWS + 1;
/// The height of the floor, in cells: the last line above the app's label.
///
/// The rug is the floor of the room, scattered with the punctuation the page is
/// not using, so the floor is under the rug rather than on top of it: the cat walks
/// in it and the words land in it.
const FLOOR: f32 = (LABEL_ROWS + 1) as f32;
/// How many rows at the bottom are left to the app's own label.
///
/// The label is drawn by the text renderer in screen space, so the page keeps the
/// last line clear: a hint printed over a line of code is a hint nobody reads.
const LABEL_ROWS: u32 = 1;

/// How far a token's centre may be from the page's edge, in cells.
pub const TOKEN_RADIUS: f32 = 0.4;
/// The longest word the kitten will bat: wider than that and it is a line, not a
/// token.
const UNDER_PAW: usize = 8;
/// The fastest a batted token may travel, in cells per second.
const MAX_SPEED: f32 = 14.0;
/// The fastest a batted token may spin.
const MAX_SPIN: f32 = 16.0;
/// How fast a token loses its speed to the page, per second.
const DRAG: f32 = 1.6;

/// How hard the page pulls, in cells per second squared.
///
/// Words and cat share the number: a page whose letters fall faster than its
/// kitten is a page drawn at two scales.
pub const GRAVITY: f32 = 26.0;
/// What a word keeps of its fall when it hits a ledge.
///
/// Small, because paper on paper does not bounce; enough that a word knocked off
/// a line gives a hop when it lands, which is how a fall reads as a fall.
const BOUNCE: f32 = 0.22;
/// The most loose words a page will carry. A cat left alone all afternoon takes
/// the page apart one glyph at a time; the oldest word goes rather than the
/// newest, so what is left is what she has just been doing.
const MAX_TOKENS: usize = 48;
/// The most glyphs a page will float at once: Zzz above a sleeping cat, and the
/// sparkles of one washing itself.
const MAX_FLOATING: usize = 32;
/// How close a paw has to be to a character to pull it out of the line, in cells.
const KNOCK: [f32; 2] = [2.2, 1.1];
/// How wide the swathe a body at a run takes is, in cells, and how many whole words
/// it can come away with at once.
///
/// Narrower than a paw's box and shallower, because a rake happens ten times in a
/// dash where a swing happens once: the point is that the page is damaged along the
/// route, not that the route is deleted.
const RAKE: [f32; 2] = [1.35, 0.8];
const RAKE_WORDS: usize = 3;
/// How wide the swathe a climbing cat takes out of the line it is leaving is, in
/// cells, and how many whole words come away with it.
///
/// Shallower than a rake, and that is the whole of it: a claw is fired on every bound
/// up the page, from the line under her back paws, and the one thing it must not touch
/// is the line she is going to land on. Half a line of height is exactly the row she
/// has left and nothing else, so an ascent leaves a furrow and still has somewhere to
/// put its feet down.
const CLAW: [f32; 2] = [1.5, 0.5];
const CLAW_WORDS: usize = 2;
/// How many characters the page reprints per second.
///
/// This is the number that lets the cat be as destructive as it likes. The page is
/// a playground and not a document, so the one thing it cannot do is run out: a run
/// takes three characters a rake and a rake every two cells, which is more than this
/// while she is at it, so a line empties under her and fills again behind her. The
/// two rates are close on purpose. Much faster and the damage is never seen; much
/// slower and a long afternoon leaves a page of gaps with a cat walking about in it.
const HEAL_RATE: f32 = 3.0;
/// How much faster than that the page prints when it is in pieces, as a multiple.
///
/// One rate cannot serve both ends of this. A run's furrow has to stay on the page for
/// a few seconds or the damage was never seen, which is [`HEAL_RATE`]; a page that has
/// half its characters in the air has to print them back faster than the cat can take
/// them or it ends its five minutes as gaps with a cat walking about in them. So the
/// rate is the first number plus this much of it, in proportion to how much of the page
/// is missing: barely damaged, a page heals at a reader's pace, and a wrecked one at
/// seven times that.
const HEAL_PANIC: f32 = 6.0;
/// How fast a character that is being printed again fades in, in alpha per second.
///
/// Not instant: a page that blinked characters back into place would read as a page
/// that was never damaged. A quarter of a second of fading is long enough to see and
/// short enough not to be waited for.
const HEAL_FADE: f32 = 2.5;
/// The most characters the page will hold in its memory of what it has lost.
///
/// A whole page, near enough: the two ends of a run and a climb's claws together can
/// put more into the air than an editor window holds characters, and a card forgotten
/// here is a card the page can never print again.
const MAX_MISSING: usize = 2400;
/// Two runs of glyphs with a gap no wider than this are one shelf, so a line with
/// a space in it is a line to stand on rather than two.
const SHELF_GAP: f32 = 2.6;

/// One loose token: a word the kitten can knock about.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    /// The characters of the token, in order.
    pub text: Vec<char>,
    /// One style per character, from the flavour's lexer.
    pub styles: Vec<u8>,
    /// Centre, in cells: `x` right, `y` up the page, `z` out of it.
    pub pos: [f32; 3],
    /// Velocity along the page, in cells per second.
    pub vel: [f32; 2],
    /// Turn in the page's own plane, in radians.
    pub roll: f32,
    /// Tipped off the page, in radians.
    pub pitch: f32,
    /// Spin, in radians per second.
    pub spin: f32,
    /// True once it has stopped moving.
    pub rest: bool,
}

impl Token {
    /// The token's own width, in cells.
    pub fn width(&self) -> f32 {
        self.text.len() as f32 * GLYPH_WIDTH
    }

    /// The cards that draw it.
    pub fn cards(&self, alpha: f32, out: &mut Vec<Card>) {
        let width = self.width();
        let (sin, cos) = self.roll.sin_cos();
        for (index, ch) in self.text.iter().copied().enumerate() {
            // Laid out along the token's own turn, so a word that has been
            // knocked sideways still spells itself.
            let along = index as f32 * GLYPH_WIDTH - width * 0.5 + GLYPH_WIDTH * 0.5;
            out.push(Card {
                ch,
                style: self.styles.get(index).copied().unwrap_or(0),
                pos: [
                    self.pos[0] + along * cos,
                    self.pos[1] + along * sin,
                    self.pos[2],
                ],
                yaw: 0.0,
                pitch: self.pitch,
                roll: self.roll,
                size: [GLYPH_WIDTH, 1.0],
                alpha,
                kind: CardKind::Glyph,
            });
        }
    }
}

/// A speck of dust, drifting in front of the page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mote {
    pub pos: [f32; 3],
    /// What a swat gave it, in cells per second. The wander below is the page's
    /// own air and is not a velocity: it is added on top of this one.
    pub vel: [f32; 2],
    /// How far it wanders from where it started.
    pub drift: f32,
    /// Where it is in its wander, so no two move together.
    pub phase: f32,
    /// Its size, in cells.
    pub size: f32,
    /// Seconds left. A mote that belongs to the page has `f32::MAX`, so the
    /// countdown never reaches it; one thrown by a paw lasts about a second.
    pub life: f32,
}

/// One ledge: a run of code on a row, seen as something to stand on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shelf {
    /// The row whose top edge this is.
    pub row: u32,
    /// Where its top edge is, in cells.
    pub y: f32,
    /// Left and right edge across the page, in cells.
    pub span: [f32; 2],
}

impl Shelf {
    /// Whether `x` is over this ledge.
    pub fn covers(&self, x: f32) -> bool {
        x >= self.span[0] && x <= self.span[1]
    }

    /// The point on it closest to `x`, which is where a cat would land.
    pub fn nearest_point(&self, x: f32) -> f32 {
        x.clamp(self.span[0], self.span[1])
    }
}

/// A glyph the page is holding up for a moment: the Zzz of a sleeping cat, or a
/// sparkle off one washing itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Floating {
    pub ch: char,
    pub style: u8,
    /// Centre, in cells.
    pub pos: [f32; 3],
    /// Drift, in cells per second.
    pub vel: [f32; 2],
    /// Seconds it has been up, and seconds it lasts.
    pub age: f32,
    pub life: f32,
    /// Size, in cells.
    pub size: f32,
}

impl Floating {
    /// How solid it is: it arrives and leaves, so it is not a thing that pops.
    pub fn alpha(&self) -> f32 {
        let in_at = (self.age * 6.0).min(1.0);
        let out = (1.0 - self.age / self.life).clamp(0.0, 1.0);
        in_at * out
    }
}

/// What one page is made of.
#[derive(Debug, Clone)]
pub struct Page {
    pub flavour: Flavour,
    pub seed: u64,
    /// Which regeneration this is, counted from the start of the session.
    pub index: u32,
    /// Cells across and down.
    pub columns: u32,
    pub rows: u32,
    /// The document: one card per character, with the furniture's boxes left
    /// empty. The furniture is not in here, because it moves.
    pub text: Vec<Card>,
    /// The furniture, each with the box it stands in.
    pub props: Vec<Prop>,
    /// The ledges: every run of code on a row, highest row first.
    pub shelves: Vec<Shelf>,
    /// Loose tokens.
    pub tokens: Vec<Token>,
    /// Dust.
    pub motes: Vec<Mote>,
    /// Glyphs the kitten raises for a moment: Zzz, sparkles.
    pub floating: Vec<Floating>,
    /// The page's light: a jump's trail, a landing's ring, a paw's confetti.
    pub sparks: Vec<Spark>,
    /// Characters the page has lost, in no order, waiting to be printed again.
    missing: Vec<Card>,
    /// Characters on their way back: printed faint and filled in.
    pub mending: Vec<Card>,
    /// Characters the page owes its reader, as a fraction.
    heal_clock: f32,
}

impl Page {
    /// Lays out the page for one regeneration.
    pub fn new(flavour: Flavour, seed: u64, index: u32, columns: u32, rows: u32) -> Self {
        let columns = columns.max(24);
        let rows = rows.max(8);
        let mut rng = Rng::new(seed ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let lines = flavour.lines();

        // The last line belongs to the app's label, so the page stops above it.
        let body = rows.saturating_sub(LABEL_ROWS).max(8);

        let mut text = Vec::with_capacity((columns * rows) as usize);
        // A prop owns its whole box, not just the cells it draws ink in: a table
        // with code showing through it is a table drawn on a page, not furniture
        // standing on one. The furniture marks the cells it took as it places
        // itself, so the two never disagree about where its box is.
        let mut taken = vec![false; (columns * rows) as usize];
        let props = furniture(&mut rng, columns, rows, &mut taken);

        // The gutter: a line number per row, dim, right-aligned, the way the
        // editor draws it.
        for row in 0..body {
            let number = format!("{:>3} ", row + 1);
            let chars: Vec<char> = number.chars().take(GUTTER as usize).collect();
            for (index, ch) in chars.into_iter().enumerate() {
                text.push(cell(row, index as u32, rows, ch, 4, 0.55));
            }
        }

        // The code: every row above the rug, taken from the flavour's material in
        // a seeded order so two pages are two files.
        let mut cursor = rng.below(lines.len());
        for row in 0..body.saturating_sub(RUG_ROWS) {
            let line = lines[cursor % lines.len()];
            cursor = cursor.wrapping_add(1 + rng.below(2));
            let styles = flavour.styles(line);
            let chars: Vec<char> = line.chars().collect();
            // Occasionally indented, so the page is not a wall of flush-left text.
            let indent = if line.starts_with(' ') { 0 } else { rng.below(6) as u32 };
            for (index, ch) in chars.iter().copied().enumerate() {
                if ch == ' ' || ch == '\t' {
                    continue;
                }
                let column = GUTTER + indent + index as u32;
                if column >= columns {
                    break;
                }
                if taken[(row * columns + column) as usize] {
                    continue;
                }
                text.push(cell(
                    row,
                    column,
                    rows,
                    ch,
                    styles.get(index).copied().unwrap_or(0),
                    1.0,
                ));
            }
        }

        // The rug: punctuation lying about at the bottom of the page, the way a
        // floor is scattered with the small things a room is used for.
        let marks = ['.', ',', ';', '{', '}', '(', ')', ':', '=', '*', '+', '-', '~', '\''];
        for row in body.saturating_sub(RUG_ROWS)..body {
            for column in 0..columns {
                if taken[(row * columns + column) as usize] || rng.below(7) != 0 {
                    continue;
                }
                let mut card = cell(row, column, rows, marks[rng.below(marks.len())], 7, 0.3);
                card.roll = rng.range(-0.5, 0.5);
                card.size = [GLYPH_WIDTH * 0.9, 0.9];
                text.push(card);
            }
        }

        let shelves = shelves_from(&text, columns, rows, body);
        let mut tokens = loose_tokens(&mut rng, flavour, columns, rows);
        // A word starts where the page would have let it fall: on the ledge under
        // it. Words hanging in the air over a page with gravity on it would be a
        // page with the gravity switched off.
        for token in &mut tokens {
            token.pos[1] = surface_from(&shelves, &props, FLOOR, token.pos[0], token.pos[1]);
        }
        let motes = (0..40)
            .map(|index| Mote {
                pos: [
                    rng.range(0.0, columns as f32 * GLYPH_WIDTH),
                    rng.range(1.0, rows as f32),
                    rng.range(0.1, 1.6),
                ],
                vel: [0.0, 0.0],
                drift: rng.range(0.3, 1.4),
                phase: index as f32 * 0.37 + rng.range(0.0, std::f32::consts::TAU),
                size: rng.range(0.03, 0.08),
                life: f32::MAX,
            })
            .collect();

        Self {
            flavour,
            seed,
            index,
            columns,
            rows,
            text,
            props,
            shelves,
            tokens,
            motes,
            floating: Vec::new(),
            sparks: Vec::new(),
            missing: Vec::new(),
            mending: Vec::new(),
            heal_clock: 0.0,
        }
    }

    /// The width of the page, in line heights.
    pub fn width(&self) -> f32 {
        self.columns as f32 * GLYPH_WIDTH
    }

    /// The corner label: which flavour, which page.
    pub fn label(&self) -> String {
        format!("{} · page {}", self.flavour.name(), self.index + 1)
    }

    /// The floor of the page, which is where a word comes to rest when there is no
    /// line under it.
    pub fn floor(&self) -> f32 {
        FLOOR
    }

    /// The height of the highest ledge under `x` that is no higher than `from`:
    /// a line of code, the top of a table, or the rug.
    pub fn surface(&self, x: f32, from: f32) -> f32 {
        surface_from(&self.shelves, &self.props, self.floor(), x, from)
    }

    /// Moves every loose token by `dt` seconds.
    ///
    /// Fixed steps are the caller's job: one step of `1/120` and one of `1/60` are
    /// not the same integration, and a jump of half a second -- which a loaded
    /// machine will produce -- would send a token off the page.
    ///
    /// The page pulls sideways with friction and down with gravity, and a token
    /// stops falling when it reaches the ledge under it. That is the whole of the
    /// floor: no token ever comes to rest in the air, and none of them lands
    /// anywhere the page does not have a surface.
    pub fn step_tokens(&mut self, dt: f32) {
        let width = self.width();
        let top = self.rows as f32;
        let floor = self.floor();
        let shelves = &self.shelves;
        let props = &self.props;
        for token in &mut self.tokens {
            if token.rest {
                continue;
            }
            let ground = surface_from(shelves, props, floor, token.pos[0], token.pos[1]);
            token.pos[0] += token.vel[0] * dt;
            token.pos[1] += token.vel[1] * dt;
            token.roll += token.spin * dt;
            token.vel[1] -= GRAVITY * dt;
            // Paper drag sideways, and only sideways: the fall is the page's.
            token.vel[0] *= (1.0 - DRAG * dt).max(0.0);
            token.spin *= (1.0 - 2.4 * dt).max(0.0);
            // A word that has hit a ledge hops a little and stays there.
            if token.pos[1] <= ground {
                token.pos[1] = ground;
                if token.vel[1] < -0.4 {
                    token.vel[1] = -token.vel[1] * BOUNCE;
                    token.vel[0] *= 0.7;
                } else {
                    token.vel[1] = 0.0;
                }
            }
            // A sliding token settles flat as it slows: a card that stops standing
            // on its edge looks broken.
            token.pitch += (0.0 - token.pitch) * 3.0 * dt;
            let landed = (token.pos[1] - ground).abs() < 0.01 && token.vel[1].abs() < 0.5;
            if landed && token.vel[0].abs() < 0.08 && token.spin.abs() < 0.3 {
                token.vel = [0.0, 0.0];
                token.spin = 0.0;
                token.pitch = 0.0;
                token.rest = true;
                token.roll %= std::f32::consts::TAU;
            }
            // The edge of the page is the edge of the world.
            let half = token.width() * 0.5;
            if token.pos[0] < half {
                token.pos[0] = half;
                token.vel[0] = token.vel[0].abs() * 0.4;
            }
            if token.pos[0] > width - half {
                token.pos[0] = width - half;
                token.vel[0] = -token.vel[0].abs() * 0.4;
            }
            if token.pos[1] > top - TOKEN_RADIUS {
                token.pos[1] = top - TOKEN_RADIUS;
                token.vel[1] = -token.vel[1].abs() * 0.4;
            }
        }
    }

    /// Takes the characters a paw landed on out of the page, and lets them fall.
    ///
    /// They come back as tokens with the colours they had on the line, one per run
    /// of characters that stood next to each other, so a knocked-out piece still
    /// spells something. The page is left with a gap where they were, which is the
    /// only way a page can show that a cat has been on it.
    pub fn knock_out(&mut self, at: [f32; 3], rng: &mut Rng) -> usize {
        let mut lifted: Vec<Card> = Vec::new();
        self.text.retain(|card| {
            let near = card.alpha >= 0.9
                && card.pos[0] > GUTTER as f32 * GLYPH_WIDTH
                && (card.pos[0] - at[0]).abs() <= KNOCK[0]
                && (card.pos[1] - at[1]).abs() <= KNOCK[1];
            if near {
                lifted.push(*card);
            }
            !near
        });
        if lifted.is_empty() {
            return 0;
        }
        self.lost(&lifted);
        // The paw is taller than a line and lands between two of them, so what it
        // takes can be the ends of two different lines. A word knocked off a page
        // is still a word: the characters are gathered one line at a time and the
        // longest run under the paw is the one that comes loose.
        let best = runs(&lifted).into_iter().next();
        let made = usize::from(best.is_some());
        // One piece per paw, not the whole line: a page that lost a line every time
        // a cat looked at it would be a page with nothing left to read.
        if let Some(run) = best {
            let first = run[0].pos[0];
            let last = run[run.len() - 1].pos[0];
            self.tokens.push(Token {
                text: run.iter().map(|card| card.ch).collect(),
                styles: run.iter().map(|card| card.style).collect(),
                pos: [(first + last) * 0.5, run[0].pos[1], 0.35],
                vel: [rng.range(-1.4, 1.4), rng.range(1.0, 3.2)],
                roll: rng.range(-0.4, 0.4),
                pitch: 0.0,
                spin: rng.range(-3.0, 3.0),
                rest: false,
            });
        }
        // The page is not a shredder: past a few dozen loose words the oldest one
        // goes, so what is left is what the cat has just been doing.
        while self.tokens.len() > MAX_TOKENS {
            self.tokens.remove(0);
        }
        self.refresh_shelves();
        made
    }

    /// A cat at a run: it does not swing, it takes what it runs into.
    ///
    /// A paw takes the longest whole word under it, because a swing is a deliberate
    /// thing. A body at speed has no such manners -- every character in its way comes
    /// loose, a word among them -- and because the swathe is narrow, what it meets is
    /// a gap in a line rather than a line gone. `heading` is which way she is going,
    /// and it is what the words she takes are thrown along: what a cat runs into goes
    /// the way the cat was going.
    ///
    /// Returns how many characters came away, which is what a caller wants to know to
    /// decide whether the page noticed.
    pub fn rake(&mut self, at: [f32; 3], heading: f32, rng: &mut Rng) -> usize {
        let mut lifted: Vec<Card> = Vec::new();
        self.text.retain(|card| {
            let near = card.alpha >= 0.9
                && card.pos[0] > GUTTER as f32 * GLYPH_WIDTH
                && (card.pos[0] - at[0]).abs() <= RAKE[0]
                && (card.pos[1] - at[1]).abs() <= RAKE[1];
            if near {
                lifted.push(*card);
            }
            !near
        });
        if lifted.is_empty() {
            return 0;
        }
        self.lost(&lifted);
        // One row at a time again: a swathe across two lines that are not joined
        // would splice their ends into a word nobody wrote. The widest runs are the
        // ones that come loose, so what she takes reads as words rather than as
        // crumbs.
        for run in runs(&lifted).into_iter().take(RAKE_WORDS) {
            let first = run[0].pos[0];
            let last = run[run.len() - 1].pos[0];
            self.tokens.push(Token {
                text: run.iter().map(|card| card.ch).collect(),
                styles: run.iter().map(|card| card.style).collect(),
                pos: [(first + last) * 0.5, run[0].pos[1], 0.35],
                vel: [
                    heading * rng.range(1.8, 3.6),
                    rng.range(1.4, 3.4),
                ],
                roll: rng.range(-0.5, 0.5),
                pitch: 0.0,
                spin: heading * rng.range(1.5, 5.0),
                rest: false,
            });
        }
        while self.tokens.len() > MAX_TOKENS {
            self.tokens.remove(0);
        }
        let taken = lifted.len();
        self.refresh_shelves();
        taken
    }

    /// A cat going up the page: the claws are in the line it is leaving.
    ///
    /// A climb is not a run and it is not a swing. A paw takes one word and stops; a
    /// body at a run ploughs a furrow along a line; a cat scaling a page of text digs
    /// in where its back paws are and hauls itself up, and what comes away falls. So
    /// this is the rake's shape and a line's worth less of it: one row, two words, and
    /// thrown down and outwards rather than along the way she is going, because the way
    /// she is going is up and nothing falls upwards.
    ///
    /// Returns how many characters came away.
    pub fn claw(&mut self, at: [f32; 3], rng: &mut Rng) -> usize {
        let mut lifted: Vec<Card> = Vec::new();
        self.text.retain(|card| {
            let near = card.alpha >= 0.9
                && card.pos[0] > GUTTER as f32 * GLYPH_WIDTH
                && (card.pos[0] - at[0]).abs() <= CLAW[0]
                && (card.pos[1] - at[1]).abs() <= CLAW[1];
            if near {
                lifted.push(*card);
            }
            !near
        });
        if lifted.is_empty() {
            return 0;
        }
        self.lost(&lifted);
        for run in runs(&lifted).into_iter().take(CLAW_WORDS) {
            let first = run[0].pos[0];
            let last = run[run.len() - 1].pos[0];
            let middle = (first + last) * 0.5;
            self.tokens.push(Token {
                text: run.iter().map(|card| card.ch).collect(),
                styles: run.iter().map(|card| card.style).collect(),
                pos: [middle, run[0].pos[1], 0.35],
                // Away from her on both axes, and never upwards: the page is above
                // her and the line she took is where she has just been standing.
                vel: [
                    (middle - at[0]).signum() * rng.range(0.6, 2.2),
                    rng.range(-3.2, -1.2),
                ],
                roll: rng.range(-0.7, 0.7),
                pitch: 0.0,
                spin: rng.range(-6.0, 6.0),
                rest: false,
            });
        }
        while self.tokens.len() > MAX_TOKENS {
            self.tokens.remove(0);
        }
        let taken = lifted.len();
        self.refresh_shelves();
        taken
    }

    /// Everything within `reach` comes off the page at once, thrown away from `at`.
    ///
    /// This is what the end of a run does, where a paw would only take a word. A cat
    /// that has been running has the whole of its weight behind the stop, so what she
    /// lands among goes in every direction: each run of characters becomes one piece
    /// and each piece is thrown along the line from `at` to where it stood, which is
    /// why the middle of the hole is empty and the edges look like a burr.
    ///
    /// Returns how many characters came away.
    pub fn blast(&mut self, at: [f32; 3], reach: f32, power: f32, rng: &mut Rng) -> usize {
        let mut lifted: Vec<Card> = Vec::new();
        self.text.retain(|card| {
            let near = card.alpha >= 0.9
                && card.pos[0] > GUTTER as f32 * GLYPH_WIDTH
                && (card.pos[0] - at[0]).abs() <= reach
                && (card.pos[1] - at[1]).abs() <= reach * 0.8;
            if near {
                lifted.push(*card);
            }
            !near
        });
        if lifted.is_empty() {
            return 0;
        }
        self.lost(&lifted);
        let taken = lifted.len();
        for run in runs(&lifted) {
            let first = run[0].pos[0];
            let last = run[run.len() - 1].pos[0];
            let middle = [(first + last) * 0.5, run[0].pos[1]];
            let dx = middle[0] - at[0];
            let dy = middle[1] - at[1];
            let away = dx.hypot(dy).max(0.4);
            self.tokens.push(Token {
                text: run.iter().map(|card| card.ch).collect(),
                styles: run.iter().map(|card| card.style).collect(),
                pos: [middle[0], middle[1], 0.35],
                vel: [
                    dx / away * power * rng.range(0.6, 1.2),
                    dy / away * power * 0.6 + rng.range(1.0, 3.4),
                ],
                roll: rng.range(-0.7, 0.7),
                pitch: 0.0,
                spin: rng.range(-7.0, 7.0),
                rest: false,
            });
        }
        while self.tokens.len() > MAX_TOKENS {
            self.tokens.remove(0);
        }
        self.refresh_shelves();
        taken
    }

    /// Takes a piece off the furniture where a body hit it.
    ///
    /// The furniture is made of characters too, and a cat that can wreck a line but
    /// not a card is a cat playing in front of a backdrop. What comes off is thrown
    /// up and away from `at`, one card at a time, so a hit prop visibly loses its
    /// middle instead of sitting there being hit.
    ///
    /// Returns how many cards came off.
    pub fn chip(&mut self, at: [f32; 3], reach: f32, rng: &mut Rng) -> usize {
        let mut shed: Vec<Card> = Vec::new();
        for prop in &mut self.props {
            if prop.cards.is_empty() || prop.distance_to(at) > reach {
                continue;
            }
            let take = 1 + rng.below(3).min(prop.cards.len().saturating_sub(1));
            for _ in 0..take.min(prop.cards.len()) {
                let index = rng.below(prop.cards.len());
                shed.push(prop.cards.swap_remove(index));
            }
        }
        let taken = shed.len();
        for card in shed {
            if self.tokens.len() >= MAX_TOKENS {
                break;
            }
            let dx = card.pos[0] - at[0];
            let dy = card.pos[1] - at[1];
            let away = dx.hypot(dy).max(0.4);
            self.tokens.push(Token {
                text: vec![card.ch],
                styles: vec![card.style],
                pos: [card.pos[0], card.pos[1], 0.35],
                vel: [
                    dx / away * rng.range(1.6, 4.0),
                    dy / away * rng.range(0.8, 2.0) + rng.range(1.4, 3.6),
                ],
                roll: rng.range(-0.9, 0.9),
                pitch: 0.0,
                spin: rng.range(-8.0, 8.0),
                rest: false,
            });
        }
        while self.tokens.len() > MAX_TOKENS {
            self.tokens.remove(0);
        }
        taken
    }

    /// Lays the ledges out again, which a page needs after a paw took a line apart.
    pub fn refresh_shelves(&mut self) {
        let body = self.rows.saturating_sub(LABEL_ROWS).max(8);
        self.shelves = shelves_from(&self.text, self.columns, self.rows, body);
    }

    /// Keeps what a paw or a run took off the page, so that the page can print it
    /// again.
    ///
    /// It is the characters that are remembered, each with the cell it stood in, so a
    /// line that lost its middle comes back with the middle in the middle. Past
    /// `MAX_MISSING` the first loss is dropped: an afternoon of this must not grow a
    /// second page in memory, and a page that forgot the far past simply ends up with
    /// the lines the cat has not been at lately.
    fn lost(&mut self, cards: &[Card]) {
        for card in cards {
            if self.missing.len() >= MAX_MISSING {
                self.missing.remove(0);
            }
            self.missing.push(*card);
        }
    }

    /// Puts the page back together at a read's pace, and fades in what it prints.
    ///
    /// The mending is what makes the wrecking possible. Without it every run would
    /// cost the page a line for good, and the mode would spend its second minute on a
    /// blank page -- so the cat would have to be kept too calm to damage anything,
    /// which is exactly what a playground should not be. Printed characters go into
    /// `mending` at nothing and are filled in over `HEAL_FADE`, and only when a card is
    /// back to full does it rejoin the page and the ledges. That is also what keeps
    /// her honest: a character that has not finished coming back cannot be taken
    /// again.
    pub fn mend(&mut self, dt: f32, rng: &mut Rng) {
        if self.missing.is_empty() {
            self.heal_clock = 0.0;
        } else {
            let whole = (self.text.len() + self.missing.len()).max(1) as f32;
            let gone = self.missing.len() as f32 / whole;
            self.heal_clock += dt * HEAL_RATE * (1.0 + HEAL_PANIC * gone);
            while self.heal_clock >= 1.0 && !self.missing.is_empty() {
                self.heal_clock -= 1.0;
                let index = rng.below(self.missing.len());
                let card = self.missing.swap_remove(index);
                self.mending.push(Card { alpha: 0.0, ..card });
            }
        }
        if self.mending.is_empty() {
            return;
        }
        for card in &mut self.mending {
            card.alpha = (card.alpha + HEAL_FADE * dt).min(1.0);
        }
        let mut arrived: Vec<Card> = Vec::new();
        self.mending.retain(|card| {
            if card.alpha >= 1.0 {
                arrived.push(*card);
                false
            } else {
                true
            }
        });
        if !arrived.is_empty() {
            self.text.extend(arrived);
            self.refresh_shelves();
        }
    }

    /// Raises a glyph off the page for a moment: the Zzz of a sleeping cat, or a
    /// sparkle off one washing itself.
    pub fn float(&mut self, ch: char, style: u8, at: [f32; 3], rng: &mut Rng) {
        if self.floating.len() >= MAX_FLOATING {
            self.floating.remove(0);
        }
        self.floating.push(Floating {
            ch,
            style,
            pos: at,
            vel: [rng.range(-0.1, 0.5), rng.range(0.7, 1.3)],
            age: 0.0,
            life: rng.range(1.6, 2.4),
            size: 0.85,
        });
    }

    /// Ages the floating glyphs and lets them drift.
    pub fn step_floating(&mut self, dt: f32) {
        for glyph in &mut self.floating {
            glyph.age += dt;
            glyph.pos[0] += glyph.vel[0] * dt;
            glyph.pos[1] += glyph.vel[1] * dt;
            glyph.vel[1] *= (1.0 - 0.5 * dt).max(0.0);
        }
        self.floating.retain(|glyph| glyph.age < glyph.life);
    }

    /// A piece of a jump's trail: a soft blob of one hue where the cat was, drifting
    /// at `vel`.
    pub fn ribbon(&mut self, at: [f32; 3], vel: [f32; 3], hue: f32, size: f32) {
        self.push_spark(Spark::ribbon(at, vel, hue, size));
    }

    /// The light that travels with her while she is in the air.
    ///
    /// Its life is a tenth of a second and it is pushed again every frame, so what
    /// is drawn is one blob that follows her rather than a string of them.
    pub fn aura(&mut self, at: [f32; 3], hue: f32, size: f32) {
        self.push_spark(Spark::aura(at, hue, size));
    }

    /// A star in among the trail, turning as it dies.
    pub fn twinkle(&mut self, at: [f32; 3], hue: f32, size: f32) {
        self.push_spark(Spark::twinkle(at, hue, size));
    }

    /// A ring on the paper, widening as it fades.
    pub fn ring(&mut self, at: [f32; 3], hue: f32, size: f32) {
        self.push_spark(Spark::ring(at, hue, size));
    }

    /// A speck of the dust a run kicks up: dim, small, and rising.
    pub fn dust(&mut self, at: [f32; 3], hue: f32, size: f32, rng: &mut Rng) {
        self.push_spark(Spark::dust(at, hue, size, rng));
    }

    /// Confetti: `count` sparks thrown from `at`, their hues fanned out from `hue`
    /// across most of the wheel, so one burst is already a rainbow.
    pub fn confetti(&mut self, at: [f32; 3], hue: f32, count: usize, speed: f32, rng: &mut Rng) {
        for index in 0..count {
            let fan = index as f32 / (count.max(1) as f32 - 1.0).max(1.0);
            let hue = (hue + fan * 0.9) % 1.0;
            self.push_spark(Spark::confetti(at, hue, speed, rng));
        }
    }

    /// Moves the light: it flies, it is pulled down, it grows, and its clock runs
    /// out. A spark is gone the frame its life ends, which is what keeps the mode
    /// from turning into a haze.
    pub fn step_sparks(&mut self, dt: f32) {
        for spark in &mut self.sparks {
            spark.step(dt);
        }
        self.sparks.retain(|spark| spark.age < spark.life);
    }

    /// What one frame draws, `fade` being how much of the page is on screen.
    pub fn glows(&self, fade: f32) -> Vec<Glow> {
        self.sparks.iter().map(|spark| spark.glow(fade)).collect()
    }

    fn push_spark(&mut self, spark: Spark) {
        // The oldest goes rather than the newest, so a burst always shows: a page
        // that dropped its newest sparks would drop the paw that threw them.
        while self.sparks.len() >= MAX_SPARKS {
            self.sparks.remove(0);
        }
        self.sparks.push(spark);
    }

    /// Pushes a token near `at` away from the kitten, and the dust with it.
    ///
    /// The paw is not an engine: a token the kitten keeps its paw on for a dozen
    /// frames takes one shove, not a dozen, which is why the push is damped by
    /// paper drag rather than summed.
    pub fn bat(&mut self, at: [f32; 3], reach: f32, power: f32) {
        for token in &mut self.tokens {
            let dx = token.pos[0] - at[0];
            let dy = token.pos[1] - at[1];
            let distance = dx.hypot(dy);
            if distance > reach {
                continue;
            }
            let scale = power / distance.max(0.4);
            token.vel[0] = (token.vel[0] + dx * scale).clamp(-MAX_SPEED, MAX_SPEED);
            token.vel[1] = (token.vel[1] + dy * scale).clamp(-MAX_SPEED, MAX_SPEED);
            token.spin = (token.spin + power * 2.2).clamp(-MAX_SPIN, MAX_SPIN);
            token.rest = false;
        }
        // Dust is lighter than a word: the same paw sends it much further.
        for mote in &mut self.motes {
            let dx = mote.pos[0] - at[0];
            let dy = mote.pos[1] - at[1];
            let distance = dx.hypot(dy);
            if distance > reach {
                continue;
            }
            let scale = power * 1.6 / distance.max(0.3);
            mote.vel[0] = (mote.vel[0] + dx * scale).clamp(-MAX_SPEED, MAX_SPEED);
            mote.vel[1] = (mote.vel[1] + dy * scale).clamp(-MAX_SPEED, MAX_SPEED);
        }
    }

    /// Sets the furniture near `at` shaking, and says whether anything was hit.
    ///
    /// The reach is measured from the edge of a prop's box rather than from its
    /// middle, so a paw at a table's leg is as close to it as a paw at the same
    /// table's top edge.
    pub fn bump(&mut self, at: [f32; 3], reach: f32, power: f32) -> bool {
        let mut hit = false;
        for prop in &mut self.props {
            if prop.distance_to(at) <= reach {
                prop.wobble = prop.wobble.max(power);
                hit = true;
            }
        }
        hit
    }

    /// Throws `count` specks of dust out of `at`, which is what a paw landing on
    /// the page does.
    pub fn puff(&mut self, at: [f32; 3], count: usize, rng: &mut Rng) {
        // A page is not a snowstorm: the burst is capped, so a cat batting at one
        // spot for a minute cannot fill the screen with its own dust.
        let room = 240usize.saturating_sub(self.motes.len());
        for _ in 0..count.min(room) {
            let angle = rng.range(0.0, std::f32::consts::TAU);
            let speed = rng.range(0.9, 3.4);
            self.motes.push(Mote {
                pos: [
                    at[0] + rng.range(-0.3, 0.3),
                    at[1] + rng.range(-0.3, 0.3),
                    rng.range(0.3, 1.5),
                ],
                vel: [angle.cos() * speed, angle.sin() * speed],
                drift: rng.range(0.2, 0.7),
                phase: rng.range(0.0, std::f32::consts::TAU),
                size: rng.range(0.04, 0.09),
                life: rng.range(0.5, 1.3),
            });
        }
    }

    /// Everything a paw landing at `at` does to the page: a word goes flying, the
    /// furniture shakes, the dust is thrown, new dust is raised and the characters
    /// under the paw come out of the line.
    ///
    /// One method rather than five calls from the caller, so the consequences of a
    /// strike are in one place and a test can assert them together.
    pub fn strike(&mut self, at: [f32; 3], rng: &mut Rng) {
        self.bat(at, 1.7, 6.0);
        self.bump(at, 1.2, 1.0);
        self.puff(at, 4, rng);
        self.knock_out(at, rng);
    }

    /// A landing: the same dust and the same shake, and nothing taken.
    ///
    /// A paw that swings takes what is under it, but a cat that came down on a line
    /// would take the line out from under itself, and a page of ledges a cat cannot
    /// stand on is a page with no ledges. What falls off a line falls to a paw, not
    /// to a landing.
    pub fn land(&mut self, at: [f32; 3], rng: &mut Rng) {
        self.bump(at, 1.2, 1.0);
        self.puff(at, 3, rng);
    }

    /// Lets the furniture's shaking decay, and moves it on.
    ///
    /// A prop answers a bump by swaying on its own phase, amplitude falling with
    /// the wobble, so two props hit together do not sway together.
    pub fn step_props(&mut self, dt: f32) {
        for prop in &mut self.props {
            if prop.wobble <= 0.0 {
                prop.phase = 0.0;
                continue;
            }
            prop.phase += dt * 26.0;
            prop.wobble = (prop.wobble - dt * 1.7).max(0.0);
        }
    }

    /// Moves the dust: its own wander, whatever a paw gave it, and its clock.
    pub fn step_motes(&mut self, dt: f32) {
        let width = self.width();
        let top = self.rows as f32;
        for mote in &mut self.motes {
            mote.phase += dt * 0.9;
            let wander = [
                mote.drift * mote.phase.sin() * 0.6,
                mote.drift * (mote.phase * 1.7).cos() * 0.4,
            ];
            mote.pos[0] = (mote.pos[0] + (wander[0] + mote.vel[0]) * dt).clamp(0.2, width - 0.2);
            mote.pos[1] = (mote.pos[1] + (wander[1] + mote.vel[1]) * dt).clamp(0.2, top - 0.2);
            let keep = (1.0 - 2.2 * dt).max(0.0);
            mote.vel[0] *= keep;
            mote.vel[1] *= keep;
            mote.life -= dt;
        }
        self.motes.retain(|mote| mote.life > 0.0);
    }
}

/// What a piece of furniture is, which is what decides what a kitten does with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropKind {
    Table,
    Plant,
    Lamp,
    Shelf,
    Frame,
}

impl PropKind {
    /// The name a test or a log line uses.
    pub fn name(self) -> &'static str {
        match self {
            PropKind::Table => "table",
            PropKind::Plant => "plant",
            PropKind::Lamp => "lamp",
            PropKind::Shelf => "shelf",
            PropKind::Frame => "frame",
        }
    }

    /// Whether its top is flat and wide enough to sit on.
    ///
    /// A cat sits on tables and shelves; a plant, a lamp and a picture are things
    /// to look at and knock about, and a cat standing on top of a lamp would be a
    /// cat floating over a lamp.
    pub fn is_perch(self) -> bool {
        matches!(self, PropKind::Table | PropKind::Shelf)
    }
}

/// One piece of furniture: its box, the characters in it, and how hard it is
/// still shaking.
#[derive(Debug, Clone, PartialEq)]
pub struct Prop {
    pub kind: PropKind,
    /// The centre of its box, in cells.
    pub pos: [f32; 3],
    /// Across and down, in cells.
    pub size: [f32; 2],
    /// Its characters, printed where they stand.
    pub cards: Vec<Card>,
    /// 0 still, 1 the hardest a paw can shake it.
    pub wobble: f32,
    /// Where it is in its sway. Only advances while it is shaking.
    pub phase: f32,
}

impl Prop {
    /// How far a point is from the box, in cells: zero inside it.
    pub fn distance_to(&self, at: [f32; 3]) -> f32 {
        let half = [self.size[0] * GLYPH_WIDTH * 0.5, self.size[1] * 0.5];
        let dx = ((at[0] - self.pos[0]).abs() - half[0]).max(0.0);
        let dy = ((at[1] - self.pos[1]).abs() - half[1]).max(0.0);
        dx.hypot(dy)
    }

    /// The height of its top edge, in cells, which is where a cat sits.
    pub fn top(&self) -> f32 {
        self.pos[1] + self.size[1] * 0.5
    }

    /// Its characters, swaying by however much it is still shaking.
    pub fn cards(&self, lift: f32, alpha: f32, out: &mut Vec<Card>) {
        let sway = self.phase.sin() * self.wobble * 0.45;
        for card in &self.cards {
            out.push(Card {
                pos: [
                    card.pos[0] + sway,
                    card.pos[1] + lift - sway.abs() * 0.2,
                    card.pos[2] + self.wobble * 0.3,
                ],
                roll: card.roll + sway * 0.12,
                alpha: (card.alpha * alpha).clamp(0.0, 1.0),
                ..*card
            });
        }
    }
}

/// The height of the highest ledge under a point at `x`, no higher than `from`.
///
/// A free function rather than a method because the caller is usually holding a
/// mutable borrow of something else on the page: a falling word is asking what is
/// under it while it is being moved.
pub(crate) fn surface_from(shelves: &[Shelf], props: &[Prop], floor: f32, x: f32, from: f32) -> f32 {
    let mut best = floor;
    for shelf in shelves {
        if shelf.covers(x) && shelf.y <= from + 0.05 && shelf.y > best {
            best = shelf.y;
        }
    }
    for prop in props {
        // Only a flat top catches anything: a word does not come to rest on a
        // lamp's shade.
        if !prop.kind.is_perch() {
            continue;
        }
        let half = prop.size[0] * GLYPH_WIDTH * 0.5;
        let top = prop.top();
        if (x - prop.pos[0]).abs() <= half && top <= from + 0.05 && top > best {
            best = top;
        }
    }
    best
}

/// Every run of code on the page, seen as a ledge.
///
/// The gutter is a margin and the rug is the floor, so neither is a ledge: what a
/// cat can stand on is what the code printed. Runs are merged across the spaces
/// inside a line, because a line with a gap in it is still a line to walk along.
/// The runs of characters that stood next to each other, widest first.
///
/// One row at a time: a swathe across two lines that are not joined would splice their
/// ends into a word nobody wrote. Within a row, two characters are in the same run when
/// the gap between them is no wider than a word space, so a word keeps its punctuation
/// and loses the space before the next one.
fn runs(cards: &[Card]) -> Vec<Vec<Card>> {
    let mut rows: BTreeMap<i32, Vec<Card>> = BTreeMap::new();
    for card in cards {
        rows.entry(card.pos[1].round() as i32).or_default().push(*card);
    }
    let mut out: Vec<Vec<Card>> = Vec::new();
    for (_, mut row) in rows {
        row.sort_by(|a, b| a.pos[0].total_cmp(&b.pos[0]));
        let mut run: Vec<Card> = Vec::new();
        for card in row {
            let joined = run
                .last()
                .is_some_and(|last: &Card| card.pos[0] - last.pos[0] <= GLYPH_WIDTH * 1.5);
            if !joined && !run.is_empty() {
                out.push(std::mem::take(&mut run));
            }
            run.push(card);
        }
        if !run.is_empty() {
            out.push(run);
        }
    }
    out.sort_by_key(|run| std::cmp::Reverse(run.len()));
    out
}

fn shelves_from(text: &[Card], columns: u32, rows: u32, body: u32) -> Vec<Shelf> {
    let mut solid = vec![false; (columns * rows) as usize];
    for card in text {
        if card.alpha < 0.9 || card.pos[0] < GUTTER as f32 * GLYPH_WIDTH {
            continue;
        }
        let column = (card.pos[0] / GLYPH_WIDTH).floor();
        let row = (rows as f32 - card.pos[1] - 0.5).round();
        if column < 0.0 || row < 0.0 || column >= columns as f32 || row >= rows as f32 {
            continue;
        }
        solid[(row as u32 * columns + column as u32) as usize] = true;
    }
    let code = body.saturating_sub(RUG_ROWS);
    let mut out = Vec::new();
    for row in 0..code {
        let mut run: Option<[u32; 2]> = None;
        for column in 0..columns {
            if solid[(row * columns + column) as usize] {
                match &mut run {
                    Some(span) if (column - span[1]) as f32 * GLYPH_WIDTH <= SHELF_GAP => {
                        span[1] = column;
                    }
                    Some(span) => {
                        push_shelf(&mut out, row, rows, span[0], span[1]);
                        run = Some([column, column]);
                    }
                    None => run = Some([column, column]),
                }
                continue;
            }
            if let Some(span) = run {
                if (column - span[1]) as f32 * GLYPH_WIDTH > SHELF_GAP {
                    push_shelf(&mut out, row, rows, span[0], span[1]);
                    run = None;
                }
            }
        }
        if let Some(span) = run {
            push_shelf(&mut out, row, rows, span[0], span[1]);
        }
    }
    out
}

/// Adds the ledge a run of cells makes, if it is wide enough to land on.
fn push_shelf(out: &mut Vec<Shelf>, row: u32, rows: u32, from: u32, to: u32) {
    // One character is not a ledge: a cat that landed on it would be a cat
    // standing on a comma.
    if to <= from {
        return;
    }
    out.push(Shelf {
        row,
        y: rows as f32 - row as f32,
        span: [from as f32 * GLYPH_WIDTH, (to + 1) as f32 * GLYPH_WIDTH],
    });
}

/// One character of the page, placed on the grid.
fn cell(row: u32, column: u32, rows: u32, ch: char, style: u8, alpha: f32) -> Card {
    Card {
        ch,
        style,
        pos: [
            (column as f32 + 0.5) * GLYPH_WIDTH,
            rows as f32 - row as f32 - 0.5,
            0.0,
        ],
        yaw: 0.0,
        pitch: 0.0,
        roll: 0.0,
        size: [GLYPH_WIDTH, 1.0],
        alpha,
        kind: CardKind::Glyph,
    }
}

/// Two to four pieces of furniture, standing in the lower half of the page.
///
/// It marks the cells it took in `taken` as it goes, so the code and the rug are
/// generated around the boxes rather than over them.
fn furniture(rng: &mut Rng, columns: u32, rows: u32, taken: &mut [bool]) -> Vec<Prop> {
    let designs: [(&[&str], PropKind); 5] = [
        (&TABLE, PropKind::Table),
        (&PLANT, PropKind::Plant),
        (&LAMP, PropKind::Lamp),
        (&SHELF, PropKind::Shelf),
        (&FRAME, PropKind::Frame),
    ];
    let mut order: Vec<usize> = (0..designs.len()).collect();
    // Shuffle, then take the first few: no page has two tables.
    for index in (1..order.len()).rev() {
        let other = rng.below(index + 1);
        order.swap(index, other);
    }
    let count = 2 + rng.below(3);
    let mut out = Vec::new();
    let mut occupied = vec![false; (columns * rows) as usize];
    // The last line is the app's label, so nothing stands on it.
    let body = rows.saturating_sub(LABEL_ROWS).max(8);
    let usable = body.saturating_sub(FLOOR_MARGIN);
    for (place, pick) in order.into_iter().take(count).enumerate() {
        let (design, kind) = designs[pick];
        let height = design.len() as u32;
        let width = design.iter().map(|line| line.chars().count()).max().unwrap_or(0) as u32;
        if width == 0 || height == 0 || usable < height + 2 || columns < width + 6 {
            continue;
        }
        // Spread them across the page rather than stacking them on one side.
        let slot = (place as f32 + 0.5) / count as f32;
        let centre = slot * (columns - width) as f32;
        // In the lower part of the page, where furniture stands in a document
        // rather than across the first lines of it.
        let lowest = usable.saturating_sub(height);
        let highest = (body / 3).min(lowest);
        let row = highest + rng.below((lowest - highest) as usize + 1) as u32;
        let column = (centre as u32).min(columns - width - 1).max(GUTTER + 1);
        // Two pieces of furniture in the same cells would be one piece of
        // furniture drawn twice.
        let free = (0..height).all(|down| {
            (0..width).all(|across| {
                let (row, column) = (row + down, column + across);
                row < rows && column < columns && !occupied[(row * columns + column) as usize]
            })
        });
        if !free {
            continue;
        }
        let mut cards = Vec::with_capacity((width * height) as usize);
        for down in 0..height {
            for across in 0..width {
                let (row, column) = (row + down, column + across);
                if row < rows && column < columns {
                    occupied[(row * columns + column) as usize] = true;
                    taken[(row * columns + column) as usize] = true;
                }
            }
        }
        for (down, line) in design.iter().enumerate() {
            for (across, ch) in line.chars().enumerate() {
                if ch == ' ' {
                    continue;
                }
                let (row, column) = (row + down as u32, column + across as u32);
                if row >= rows || column >= columns {
                    continue;
                }
                cards.push(cell(row, column, rows, ch, 2, 0.95));
            }
        }
        out.push(Prop {
            kind,
            pos: [
                (column as f32 + width as f32 * 0.5) * GLYPH_WIDTH,
                rows as f32 - row as f32 - height as f32 * 0.5,
                0.0,
            ],
            size: [width as f32, height as f32],
            cards,
            wobble: 0.0,
            phase: place as f32 * 0.7,
        });
    }
    out
}

/// A table: a top, two legs, and a line under it.
const TABLE: [&str; 5] = [
    " ___________ ",
    "|           |",
    "|___________|",
    "  ||     ||  ",
    "  ||     ||  ",
];

/// A plant: leaves, a stem and a pot.
const PLANT: [&str; 5] = [
    "  \\ | /  ",
    " \\ \\|/ / ",
    "   \\|/   ",
    "    |    ",
    "  [___]  ",
];

/// A lamp: a shade, a stem and a foot.
const LAMP: [&str; 5] = [
    "  ,---.  ",
    " (     ) ",
    "  `---'  ",
    "    |    ",
    "  =====  ",
];

/// A shelf: two boards and three books.
const SHELF: [&str; 4] = [
    "+---------+",
    "| [] [] [ |",
    "+---------+",
    "|_________|",
];

/// A framed picture on the page.
const FRAME: [&str; 4] = [
    "+------+",
    "| .  . |",
    "|  --  |",
    "+------+",
];

/// The words lying about for the kitten to bat.
///
/// A token is a whole word off the page, not a slice of one: a fragment the kitten
/// knocks about reads as a rendering fault rather than as something a cat did.
fn loose_tokens(rng: &mut Rng, flavour: Flavour, columns: u32, rows: u32) -> Vec<Token> {
    let lines = flavour.lines();
    let count = 4 + rng.below(4);
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let line = lines[rng.below(lines.len())];
        let styles = flavour.styles(line);
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() {
            continue;
        }
        // Every run of non-space characters short enough to be a thing the kitten
        // can carry, picked at random.
        let mut words = Vec::new();
        let mut start = None;
        for (index, ch) in chars.iter().chain([' '].iter()).enumerate() {
            match (ch.is_whitespace(), start) {
                (false, None) => start = Some(index),
                (true, Some(from)) => {
                    let length = index - from;
                    if (1..=UNDER_PAW).contains(&length) {
                        words.push((from, length));
                    }
                    start = None;
                }
                _ => {}
            }
        }
        if words.is_empty() {
            continue;
        }
        let (from, length) = words[rng.below(words.len())];
        let text: Vec<char> = chars[from..from + length].to_vec();
        let token_styles: Vec<u8> = (from..from + length)
            .map(|at| styles.get(at).copied().unwrap_or(0))
            .collect();
        let half = text.len() as f32 * GLYPH_WIDTH * 0.5;
        out.push(Token {
            text,
            styles: token_styles,
            pos: [
                rng.range(half + 1.0, (columns as f32 * GLYPH_WIDTH - half - 1.0).max(half + 1.0)),
                rng.range(1.0, (rows.saturating_sub(FLOOR_MARGIN) as f32).max(2.0)),
                0.0,
            ],
            vel: [0.0, 0.0],
            roll: rng.range(-0.5, 0.5),
            pitch: 0.0,
            spin: 0.0,
            rest: true,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playground::GlowKind;

    fn page() -> Page {
        Page::new(Flavour::Rust, 0x5EED, 0, 120, 40)
    }

    /// A long line with ink on it, and a character on that line to aim at: a swathe
    /// in a gap would take nothing, which is right of the page and wrong of a test.
    fn inked(page: &Page) -> (Shelf, f32) {
        let shelf = *page
            .shelves
            .iter()
            .find(|shelf| shelf.row > 10 && shelf.span[1] - shelf.span[0] > 8.0)
            .expect("a long line");
        let x = page
            .text
            .iter()
            .filter(|card| {
                card.pos[0] >= shelf.span[0]
                    && card.pos[0] <= shelf.span[1]
                    && (page.rows as f32 - card.pos[1] - 0.5).round() as u32 == shelf.row
            })
            .map(|card| card.pos[0])
            .next()
            .expect("a character on the line");
        (shelf, x)
    }

    #[test]
    fn a_ring_on_the_paper_widens_as_it_fades() {
        let mut page = page();
        page.ring([10.0, 5.0, 0.5], 0.3, 0.4);
        let first = page.glows(1.0);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].kind, GlowKind::Ring);
        for _ in 0..36 {
            page.step_sparks(1.0 / 120.0);
        }
        let later = page.glows(1.0);
        assert_eq!(later.len(), 1);
        assert!(
            later[0].size > first[0].size + 0.3,
            "{} then {}",
            first[0].size,
            later[0].size
        );
        // Bright while it is small, gone by the time it is wide: the ring arrives
        // over a few frames, so compare a tenth of a second with a third.
        page.ring([10.0, 5.0, 0.5], 0.3, 0.4);
        for _ in 0..12 {
            page.step_sparks(1.0 / 120.0);
        }
        let fresh = page.glows(1.0);
        let bright = fresh.last().expect("the ring just pushed").alpha;
        assert!(later[0].alpha < bright, "a ring dims as it widens");
    }

    #[test]
    fn confetti_fans_out_across_the_wheel() {
        let mut page = page();
        let mut rng = Rng::new(7);
        page.confetti([10.0, 10.0, 0.5], 0.2, 12, 5.0, &mut rng);
        let glows = page.glows(1.0);
        assert_eq!(glows.len(), 12);
        let low = glows.iter().fold(f32::MAX, |m, g| m.min(g.hue));
        let high = glows.iter().fold(f32::MIN, |m, g| m.max(g.hue));
        assert!(high - low > 0.6, "{low} to {high}");
        // Thrown, not dropped: some of them are going up.
        assert!(page.sparks.iter().any(|spark| spark.vel[1] > 1.0));
    }

    #[test]
    fn a_spark_is_gone_when_its_second_is_up() {
        let mut page = page();
        let mut rng = Rng::new(11);
        page.confetti([10.0, 10.0, 0.5], 0.5, 8, 4.0, &mut rng);
        page.ribbon([10.0, 10.0, 0.5], [0.0; 3], 0.1, 0.5);
        page.aura([10.0, 10.0, 0.5], 0.2, 2.0);
        assert_eq!(page.glows(1.0).len(), 10);
        for _ in 0..(2.0 / (1.0 / 120.0)) as usize {
            page.step_sparks(1.0 / 120.0);
        }
        assert!(page.glows(1.0).is_empty(), "{} left", page.sparks.len());
    }

    #[test]
    fn the_page_never_holds_more_than_its_share_of_light() {
        let mut page = page();
        let mut rng = Rng::new(3);
        for _ in 0..(MAX_SPARKS * 3) {
            page.confetti([10.0, 10.0, 0.5], 0.5, 1, 4.0, &mut rng);
        }
        assert_eq!(page.sparks.len(), MAX_SPARKS);
    }

    #[test]
    fn a_page_is_lines_of_code_in_a_gutter() {
        let page = page();
        assert!(page.text.len() > 700, "{}", page.text.len());
        // One line number, three digits and a space, per row.
        let gutter = page
            .text
            .iter()
            .filter(|card| card.style == 4 && card.ch.is_ascii_digit())
            .count();
        assert!(gutter > 40, "{}", gutter);
        // Every card is inside the page, and on the grid.
        let width = page.width();
        for card in &page.text {
            assert!(card.pos[0] > 0.0 && card.pos[0] < width, "{:?}", card.pos);
            assert!(card.pos[1] > 0.0 && card.pos[1] <= page.rows as f32, "{:?}", card.pos);
            assert_eq!(card.pos[2], 0.0);
        }
    }

    #[test]
    fn the_same_seed_lays_out_the_same_page() {
        let a = page();
        let b = page();
        assert_eq!(a.text.len(), b.text.len());
        assert_eq!(a.text[10], b.text[10]);
        assert_eq!(a.tokens, b.tokens);
    }

    #[test]
    fn two_seeds_lay_out_two_pages() {
        let a = Page::new(Flavour::Rust, 1, 0, 120, 40);
        let b = Page::new(Flavour::Rust, 2, 0, 120, 40);
        assert!(a.text.iter().map(|c| c.ch).ne(b.text.iter().map(|c| c.ch)));
    }

    #[test]
    fn the_furniture_stands_on_the_page_and_not_on_the_rug() {
        let page = page();
        assert!(!page.props.is_empty(), "a page with furniture");
        let rug = page.rows - RUG_ROWS;
        for prop in &page.props {
            // The whole box, not just its ink: the bottom edge of the box is what
            // has to stay above the rug.
            let low = prop.pos[1] - prop.size[1] * 0.5;
            let row = page.rows as f32 - low;
            assert!(row <= rug as f32, "{} on the rug: row {row}", prop.kind.name());
            assert!(!prop.cards.is_empty(), "{}", prop.kind.name());
        }
    }

    #[test]
    fn the_furniture_leaves_its_box_empty_on_the_page() {
        // A table with code showing through it is a table drawn on a page.
        let page = page();
        for prop in &page.props {
            let left = prop.pos[0] - prop.size[0] * GLYPH_WIDTH * 0.5;
            let right = prop.pos[0] + prop.size[0] * GLYPH_WIDTH * 0.5;
            let low = prop.pos[1] - prop.size[1] * 0.5;
            let high = prop.pos[1] + prop.size[1] * 0.5;
            for card in &page.text {
                let inside = card.pos[0] > left
                    && card.pos[0] < right
                    && card.pos[1] > low
                    && card.pos[1] < high;
                assert!(!inside, "{} has {:?} in its box", prop.kind.name(), card);
            }
        }
    }

    #[test]
    fn a_paw_on_the_furniture_shakes_it_and_the_shaking_stops() {
        let mut page = page();
        let at = page.props[0].pos;
        assert!(page.bump(at, 0.6, 1.0), "a paw on the table hit nothing");
        let shaking = page.props[0].wobble;
        assert!(shaking > 0.0, "{shaking}");
        // It sways while it shakes, and it settles exactly where it stood.
        let mut still = Vec::new();
        page.props[0].cards(0.0, 1.0, &mut still);
        page.step_props(0.05);
        let mut moved = Vec::new();
        page.props[0].cards(0.0, 1.0, &mut moved);
        assert_ne!(still[0].pos, moved[0].pos, "the table did not move");
        for _ in 0..600 {
            page.step_props(1.0 / 120.0);
        }
        assert_eq!(page.props[0].wobble, 0.0);
        let mut settled = Vec::new();
        page.props[0].cards(0.0, 1.0, &mut settled);
        assert_eq!(settled[0].pos, page.props[0].cards[0].pos);
    }

    #[test]
    fn a_strike_throws_dust_that_dies_and_leaves_the_page_as_it_was() {
        let mut page = page();
        let mut rng = Rng::new(0xC0FFEE);
        let before = page.motes.len();
        page.strike([page.width() * 0.5, 20.0, 0.0], &mut rng);
        assert!(page.motes.len() > before, "no dust was raised");
        // The thrown dust is short-lived; the page's own dust is not.
        for _ in 0..(120 * 3) {
            page.step_motes(1.0 / 120.0);
        }
        assert_eq!(page.motes.len(), before);
    }

    #[test]
    fn a_paw_on_a_line_takes_characters_out_of_it() {
        let mut page = page();
        let mut rng = Rng::new(0xBEEF);
        let shelf = *page.shelves.iter().find(|shelf| shelf.row > 10).expect("a line");
        let before = page.text.len();
        let made = page.knock_out([shelf.nearest_point(40.0), shelf.y + 0.4, 0.0], &mut rng);
        assert!(made > 0, "nothing came out of the line");
        assert!(page.text.len() < before, "the page kept its characters");
        // What came out is a word, in the air, on its way down.
        let lifted = page.tokens.last().expect("a word");
        assert!(!lifted.rest, "{lifted:?}");
        assert!(!lifted.text.is_empty(), "{lifted:?}");
    }

    #[test]
    fn a_run_takes_characters_out_of_a_line_and_leaves_it_standing() {
        // A rake is to a swing what a shoulder is to a paw: narrower and without
        // manners. What matters is that the line is still a ledge afterwards -- the
        // gap a rake leaves is bridged, so a cat does not run into a hole it dug
        // itself -- and that what came away went the way she was going.
        let mut page = page();
        let mut rng = Rng::new(0x7A4E);
        let (shelf, x) = inked(&page);
        let before = page.text.len();
        let ledges = page.shelves.len();
        let taken = page.rake([x, shelf.y - 0.5, 0.0], 1.0, &mut rng);
        assert!(taken >= 2, "{taken} characters came away");
        assert_eq!(page.text.len(), before - taken);
        assert_eq!(
            page.shelves.len(),
            ledges,
            "the rake split the line it ran along"
        );
        let thrown = page.tokens.last().expect("a word in the air");
        assert!(thrown.vel[0] > 0.0, "{thrown:?} went the wrong way");
    }

    #[test]
    fn the_page_prints_back_what_a_run_took() {
        // A playground that could be emptied would have to keep the cat too calm to
        // empty it, which is the version of this mode that was boring. The page mends
        // instead: a character at a time, a quarter of a second behind the paw, so a
        // furrow is a furrow and not a hole that never closes.
        let mut page = page();
        let mut rng = Rng::new(0x4EE7);
        let (shelf, x) = inked(&page);
        let before = page.text.len();
        let taken = page.rake([x, shelf.y - 0.5, 0.0], 1.0, &mut rng);
        assert!(taken >= 2, "{taken} characters came away");
        // Not at once: a page that blinked its characters back would read as a page
        // that had never been damaged at all.
        page.mend(1.0 / 120.0, &mut rng);
        assert_eq!(page.text.len(), before - taken);
        // One character per heal, at the page's own rate: long enough for the first one
        // to be printed and still be faint.
        for _ in 0..(120.0 / HEAL_RATE) as usize + 6 {
            page.mend(1.0 / 120.0, &mut rng);
        }
        assert!(!page.mending.is_empty(), "nothing was ever on its way back");
        // And what is back is whole: the ghosts are the ones still fading.
        for _ in 0..(120 * 4) {
            page.mend(1.0 / 120.0, &mut rng);
        }
        assert_eq!(page.text.len(), before, "the line never came back");
        assert!(page.mending.is_empty(), "{} still ghosting", page.mending.len());
    }

    #[test]
    fn a_blast_takes_a_radius_where_a_paw_takes_a_word() {
        // What ends a run: a cat stopping from a gallop puts all of itself down, so
        // the page gives way in every direction at once instead of losing the one word
        // a swinging paw would have taken.
        let (shelf, _) = inked(&page());
        let middle = (shelf.span[0] + shelf.span[1]) * 0.5;
        let at = [middle, shelf.y - 0.5, 0.0];
        let mut struck = page();
        let mut blasted = page();
        let words = struck.text.len();
        struck.strike(at, &mut Rng::new(0xB1A5));
        let paw = words - struck.text.len();
        let thrown = blasted.tokens.len();
        let taken = blasted.blast(at, 2.6, 10.0, &mut Rng::new(0xB1A5));
        assert_eq!(blasted.text.len(), words - taken);
        assert!(paw > 0, "the paw took nothing to compare with");
        assert!(taken > paw, "{taken} against a paw's {paw}");
        assert!(blasted.tokens.len() > thrown, "nothing was thrown clear");
        let near = |page: &Page| {
            page.text
                .iter()
                .filter(|card| {
                    (card.pos[0] - at[0]).abs() <= 2.6 && (card.pos[1] - at[1]).abs() <= 2.0
                })
                .count()
        };
        assert!(near(&blasted) == 0, "{} characters stayed in the hole", near(&blasted));
        assert!(near(&struck) > 0, "a paw took the whole radius too");
    }

    #[test]
    fn a_claw_takes_the_line_she_has_left_and_nothing_over_her_head() {
        // A climbing cat digs in where its back paws are: the row it has just left,
        // and nothing above it. The line she is going to has to still be there, or the
        // climb has nowhere to land, and that is what the height of the swathe buys.
        let page = page();
        let (shelf, _) = inked(&page);
        let x = (shelf.span[0] + shelf.span[1]) * 0.5;
        let at = [x, shelf.y - 0.5, 0.0];
        let mut climbed = page.clone();
        let loose = climbed.tokens.len();
        let taken = climbed.claw(at, &mut Rng::new(0xC1A3));
        assert!(taken > 0, "the claws took nothing");
        let in_a_line = |page: &Page, y: f32| {
            page.text
                .iter()
                .filter(|card| {
                    (card.pos[0] - x).abs() <= CLAW[0] && (card.pos[1] - y).abs() <= CLAW[1]
                })
                .count()
        };
        assert_eq!(in_a_line(&climbed, at[1]), 0, "the row she left kept its characters");
        let over = |page: &Page| {
            page.text
                .iter()
                .filter(|card| card.pos[1] > at[1] + CLAW[1])
                .count()
        };
        assert_eq!(over(&climbed), over(&page), "the claws reached over her head");
        // What comes off falls: the page is above her, so nothing on it goes up.
        let shed = &climbed.tokens[loose..];
        assert!(!shed.is_empty(), "nothing was thrown");
        assert!(
            shed.iter().all(|token| token.vel[1] < 0.0),
            "{:?}",
            shed.iter().map(|token| token.vel).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_furniture_sheds_what_it_is_made_of_when_it_is_hit_hard() {
        // A cat that can wreck a line but not a card is a cat playing in front of a
        // backdrop. What comes off a prop is thrown up and away from the hit, so a
        // table loses its middle rather than sitting there being hit.
        let mut page = page();
        let mut rng = Rng::new(0xC41F);
        let index = page
            .props
            .iter()
            .position(|prop| prop.cards.len() > 3)
            .expect("a prop with ink in it");
        let at = page.props[index].pos;
        let before = page.props[index].cards.len();
        let shed = page.chip(at, 1.0, &mut rng);
        let after = page.props[index].cards.len();
        assert!(after < before, "the prop kept every character");
        assert!(shed >= before - after, "{shed} came off a prop of {before}");
    }

    #[test]
    fn the_page_remembers_only_so_much_of_what_it_lost() {
        // An afternoon of this must not grow a second page in memory. Past the cap the
        // oldest loss is forgotten, which leaves the page with the lines the cat has
        // not been at lately and not with an unbounded list of everything she has.
        let mut page = page();
        let cards: Vec<Card> = std::mem::take(&mut page.text);
        let mut lost = 0;
        while lost <= MAX_MISSING {
            page.lost(&cards);
            lost += cards.len();
        }
        assert_eq!(page.missing.len(), MAX_MISSING);
    }

    #[test]
    fn a_paw_between_two_lines_takes_one_word_not_two() {
        // The paw is taller than a line, so it comes down on the gap and reaches
        // into both of them. What comes loose is one run off one line: two lines
        // shuffled together would not be a word.
        let mut page = page();
        let mut rng = Rng::new(0x5A17);
        let mut pair = None;
        for above in &page.shelves {
            for below in &page.shelves {
                let overlap =
                    above.span[1].min(below.span[1]) - above.span[0].max(below.span[0]);
                if (above.y - below.y - 1.0).abs() < 0.01 && overlap > 4.0 {
                    pair = Some((*above, *below));
                }
            }
        }
        let (above, below) = pair.expect("two lines a row apart");
        let x = (above.span[0].max(below.span[0]) + above.span[1].min(below.span[1])) * 0.5;
        let mut lines: Vec<String> = Vec::new();
        for row in [above.row, below.row] {
            let mut cards: Vec<&Card> = page
                .text
                .iter()
                .filter(|card| {
                    (card.pos[0] - x).abs() <= KNOCK[0]
                        && (page.rows as f32 - card.pos[1] - 0.5).round() as u32 == row
                })
                .collect();
            cards.sort_by(|a, b| a.pos[0].total_cmp(&b.pos[0]));
            lines.push(cards.iter().map(|card| card.ch).collect());
        }
        assert!(page.knock_out([x, (above.y + below.y) * 0.5, 0.0], &mut rng) > 0);
        let word = page.tokens.last().expect("a word");
        let text: String = word.text.iter().collect();
        assert!(
            lines.iter().any(|line| line.contains(&text)),
            "{text:?} came off {lines:?}"
        );
    }

    #[test]
    fn a_word_knocked_off_a_line_falls_to_the_one_below() {
        let mut page = page();
        let mut rng = Rng::new(0x1234);
        let shelf = *page
            .shelves
            .iter()
            .find(|shelf| shelf.y > 12.0)
            .expect("a line high enough to fall from");
        let at = [shelf.nearest_point(40.0), shelf.y + 0.4, 0.0];
        page.knock_out(at, &mut rng);
        let start = page.tokens.last().expect("a word").pos[1];
        assert!(start > 10.0, "{start}");
        for _ in 0..(120 * 4) {
            page.step_tokens(1.0 / 120.0);
        }
        let word = page.tokens.last().expect("a word");
        let ground = page.surface(word.pos[0], word.pos[1] + 0.01);
        assert!(word.pos[1] < start, "it did not fall: {} then {}", start, word.pos[1]);
        assert!(
            (word.pos[1] - ground).abs() < 0.05,
            "it came to rest in the air at {} with the page at {ground}",
            word.pos[1]
        );
    }

    #[test]
    fn dust_is_thrown_further_than_words_are() {
        let mut page = page();
        let mut rng = Rng::new(7);
        let at = [page.width() * 0.5, 20.0, 0.0];
        page.strike(at, &mut rng);
        let fastest = page
            .motes
            .iter()
            .map(|mote| mote.vel[0].hypot(mote.vel[1]))
            .fold(0.0f32, f32::max);
        assert!(fastest > 0.9, "{fastest}");
    }

    #[test]
    fn a_batted_token_stays_on_the_page() {
        let mut page = page();
        page.bat([page.width() * 0.5, 20.0, 0.0], 6.0, 8.0);
        for _ in 0..(120 * 20) {
            page.step_tokens(1.0 / 120.0);
        }
        for token in &page.tokens {
            assert!(token.pos[0] > 0.0, "{token:?}");
            assert!(token.pos[0] < page.width(), "{token:?}");
            assert!(token.pos[1] > 0.0, "{token:?}");
            assert!(token.pos[1] < page.rows as f32, "{token:?}");
        }
    }

    #[test]
    fn a_batted_token_stops_instead_of_drifting_for_ever() {
        let mut page = page();
        page.bat([page.width() * 0.5, 20.0, 0.0], 8.0, 6.0);
        for _ in 0..(120 * 40) {
            page.step_tokens(1.0 / 120.0);
        }
        assert!(
            page.tokens.iter().all(|token| token.rest),
            "every token came to rest"
        );
    }

    #[test]
    fn the_label_names_the_flavour_and_the_page() {
        let page = Page::new(Flavour::Csv, 9, 2, 100, 30);
        assert_eq!(page.label(), "csv · page 3");
    }
}
