//! The kitten: what it does, and what it looks like doing it.
//!
//! The whole animal is a few dozen boxes, and every one of them is there to make
//! a kitten out of a silhouette: a head as wide as the body is long, a muzzle
//! short enough to stay under the eye, an eye a fifth of the face with a white
//! glint in it, ears wide at the base and narrow at the tip with the pink of the
//! inside showing, cream chin, bib and socks, and a tail on a bow that curls up
//! beside the rump. At two lines tall those are the cues that survive; whiskers
//! and fur are not. A smooth-skinned quadruped would be a rig, a weight table and
//! a mesh tool for something the size of a postage stamp.
//!
//! Two spaces, not one. The body is built facing `+z` and turned a quarter turn
//! to walk along the page -- `yaw` is always `±PI/2` -- so it is in profile, with
//! the length of the cat running left to right along the lines. The head is built
//! in its own space and turned by `head_yaw` about its own anchor, which is nearly
//! nothing, so the face looks out at the reader. A head in profile shows one eye,
//! half a muzzle and no face at all: that is a dog, or a rabbit, or a biscuit. The
//! cat moves in `x` and `y`, up and down lines of text, and never in `z`.
//!
//! Boxes that stack have to overlap, and a box must never be wider than the one
//! in front of it: the renderer lights every face, so a top face left showing
//! inside a silhouette is a bright band across the cat, and a band across a face
//! is a muzzle nobody drew.
//!
//! The behaviour is a small state machine with a timer per state. It sees where
//! the pointer is on the page, how long ago anything happened, the ledges the page
//! is made of, and the seeded generator. Nothing else, which is what makes it
//! testable without a window.
//!
//! Gravity is the page's, not hers. She stands on the highest ledge under her --
//! a line of code, a table top, the rug -- walks along it, drops to the one below
//! when she steps off the end, and jumps when what she wants is over her head.
//! That is the whole of the movement, and it is why the page's lines are load
//! bearing rather than decoration.

use super::scene::{surface_from, Mote, Prop, Shelf, Token, GLYPH_WIDTH, GRAVITY};
use super::{Block, Rng, Shade};

/// How long nothing has to happen before the kitten gives up and sleeps.
pub const SLEEP_AFTER: f32 = 180.0;

/// How fast it walks, in characters per second.
pub const WALK: f32 = 2.1;
/// How fast it runs.
pub const RUN: f32 = 4.4;
/// How fast it goes when something has its whole attention and the legs are
/// allowed to do what legs do.
///
/// Twice a run, which is not a number chosen for looks: at this speed a crossing of
/// the page takes under two seconds, so a dash reads as an event rather than as a
/// slightly hurried walk.
pub const DASH: f32 = RUN * 2.0;
/// How fast she goes on the zoomies, in characters per second.
///
/// Short of a dash, because a lap is a thing that goes on for seconds and turns,
/// while a dash is one straight line. Well over a run, because a lap at a run would
/// be a walk with a worse mood, and because at this speed she covers a line in about
/// a second and a half, which is how often the page has to see her.
pub const ZOOM: f32 = RUN * 1.7;
/// How far she travels between one rake and the next, in characters.
///
/// Shorter than the swathe is wide, so one rake overlaps the last and the furrow is
/// a line of damage rather than a row of separate bites: what a reader follows is
/// the damage going past. Wider apart and a run stops touching anything between its
/// bites; closer together and the page is taken apart by a single lap.
const RAKE_EVERY: f32 = 1.5;
/// How long a jump takes, in seconds.
///
/// Fixed rather than worked out from the height: a jump of one line and a jump of
/// four should leave the ground with the same hurry, and it is the arc that is
/// scaled to the height instead.
const LEAP_TIME: f32 = 0.42;
/// How far she has to rise between one pull of the claws and the next, in cells.
///
/// Smaller than a line, so that every row she passes is dug into and the furrow of an
/// ascent is a line of damage rather than a row of bites: a bound of five lines pulls
/// five times on the way up.
const CLAW_EVERY: f32 = 0.9;
/// How far to the side of her a foothold can be and still be one bound away, in cells.
///
/// A page of code is ragged: the line above her may end before her column, and a climb
/// that only took what was exactly overhead would stop a third of the way up. So a
/// foothold may lean, by most of a bound across the page. What it may not do is be
/// further over than that, because the rest of the way would be a walk along her own line
/// and then it is not a climb any more. Where the page above her column runs out, the
/// ascent has run out with it.
const REACH: f32 = 3.0;
/// How far across the page one leg of a swoop goes, in cells.
///
/// The page is sixty-six of them wide and the wall of furniture is against its right
/// edge, so a leg of thirty-four crosses the page and no further: the wall to the code,
/// and back again, which is the leap the wall is there to make possible. Longer than this
/// and one jump is the whole page and the trail behind it is a straight line rather than
/// an arc; much shorter and it is a hop with a rainbow on it.
const SWOOP_LEG: f32 = 34.0;
/// How far a super jump may gain, in cells.
///
/// The wall is climbed from the left as well as from the right. A cat standing in the
/// code with the furniture over her head has the whole width of the page between her and
/// a foothold, and a leap that could only go down would leave her in the code for ever;
/// five lines is a table's top over a shelf, and under what one bound climbs, so going up
/// this way is still a jump rather than a lift.
const SUPER_RISE: f32 = 5.0;
/// How far across a leg has to go to be worth taking, in cells.
///
/// A swoop that landed where it took off would be a cat bouncing. Under this there is
/// nothing left to the left worth leaving the ground for, and the mood has run out.
const SWOOP_MIN: f32 = 7.0;
/// How far below her a leg may land, in cells.
///
/// A swoop is the way down as well as across: every leg is a few lines lower than the one
/// before, so a cat at the top of the wall comes down the page in three bounds. Any
/// further than this and the leg is a fall with a jump drawn on it.
const SWOOP_DROP: f32 = 9.0;
/// How long a swoop is in the air for, in seconds, and how much each cell across adds.
///
/// A jump is timed by its height and takes four tenths of a second; a swoop is timed by
/// its length, because the thing worth looking at is the arc going past. A leg of
/// twenty-four cells lasts the better part of a second, which is slow enough to follow
/// the trail and fast enough to read as thrown.
const SWOOP_TIME: f32 = 0.45;
const SWOOP_PER_CELL: f32 = 0.02;

/// What the kitten is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mood {
    /// Walking to somewhere it picked itself.
    Wander,
    /// Walking or running toward the pointer.
    Follow,
    /// In the air, on its way to the pointer.
    Pounce,
    /// Pawing at a token.
    Bat,
    /// Sitting still, looking around.
    Sit,
    /// Licking a paw.
    Groom,
    /// Washing its face with a wet paw.
    Clean,
    /// Curled up.
    Sleep,
    /// Going for a piece of furniture with a paw.
    Swat,
    /// Running after a speck of dust.
    Chase,
    /// Up on a table or a shelf, sitting on the thing it climbed.
    Perch,
    /// A lap of the page at a speed nobody asked for.
    Zoomies,
    /// Going up: the next foothold above, and then the one above that, to the top
    /// of the page.
    ///
    /// Not a jump, which is one bound at a thing; a climb is a run of bounds with
    /// another one already chosen at the end of each, and what ends it is there being
    /// nothing left over her head. It takes the line it leaves with it as it goes,
    /// because her claws have to go somewhere.
    Climb,
    /// After something: flat out, in a straight line, taking what is in the way.
    ///
    /// Different from `Chase` in that she is not after a speck that is already
    /// there -- she picks the far end of the length of page she is standing on and
    /// covers it at the top of her speed, and the line she runs along comes apart
    /// behind her as she goes.
    Dash,
    /// Across the page: the long leaps between the wall of furniture on the right and
    /// the code on the left, and the ones after them.
    ///
    /// The wall is against the right edge and the code fills the rest, so a leap from
    /// one to the other is the only way across without walking. Out of the wall it is a
    /// leg at a time to the left, each a few lines lower than the last; back out of the
    /// code it is one leg to the right and up, because the furniture over her head is
    /// what there is to come down on. A run of legs is one direction -- a cat that
    /// turned round at every landing would be a cat bouncing -- and it ends where the
    /// page does, in the code or on the wall.
    Swoop,
}

impl Mood {
    /// The name a test or a log line uses.
    pub fn name(self) -> &'static str {
        match self {
            Mood::Wander => "wander",
            Mood::Follow => "follow",
            Mood::Pounce => "pounce",
            Mood::Bat => "bat",
            Mood::Sit => "sit",
            Mood::Groom => "groom",
            Mood::Clean => "clean",
            Mood::Sleep => "sleep",
            Mood::Swat => "swat",
            Mood::Chase => "chase",
            Mood::Perch => "perch",
            Mood::Zoomies => "zoomies",
            Mood::Climb => "climb",
            Mood::Dash => "dash",
            Mood::Swoop => "swoop",
        }
    }
}

/// What the kitten can see.
pub struct Senses<'a> {
    /// Where the pointer is on the page, if it is over the window.
    pub pointer: Option<[f32; 3]>,
    /// Seconds since the pointer last moved.
    pub pointer_age: f32,
    /// Seconds since any key or button.
    pub input_age: f32,
    /// The page's size, in cells: how far it may walk.
    pub page: [f32; 2],
    /// The furniture, which is what there is to climb on and knock about.
    pub props: &'a [Prop],
    /// Loose words, which are what there is to bat.
    pub tokens: &'a [Token],
    /// Dust, which is what there is to chase.
    pub motes: &'a [Mote],
    /// The ledges, which are what there is to jump onto.
    pub shelves: &'a [Shelf],
    /// The floor of the page: the top of the rug.
    pub floor: f32,
    pub rng: &'a mut Rng,
}

impl Senses<'_> {
    /// The ledge under a point, which is what a cat falls onto.
    pub fn surface(&self, x: f32, from: f32) -> f32 {
        surface_from(self.shelves, self.props, self.floor, x, from)
    }
}

/// Something the page should hold up for a moment because of what the cat is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Sleep: a Zzz off the top of her head.
    Zzz,
    /// Washing: a sparkle off the paw on her face.
    Lick,
}

/// The kitten.
#[derive(Debug, Clone)]
pub struct Kitten {
    /// Between the paws, on the page: `x` right, `y` up it, `z` unused and zero.
    pub pos: [f32; 3],
    pub yaw: f32,
    pub mood: Mood,
    /// Seconds left in this mood.
    pub timer: f32,
    /// Where it is heading.
    pub target: [f32; 3],
    /// Walk cycle, in radians.
    pub phase: f32,
    /// Current speed in characters per second.
    pub speed: f32,
    /// 0 upright, 1 crouched.
    pub crouch: f32,
    /// 0 standing, 1 sitting: blended, so she lowers herself instead of snapping.
    pub sit: f32,
    /// Which way the head looks, in radians, independent of which way she walks.
    ///
    /// The body is always in profile -- a cat is a long animal and a page is wide
    /// -- but a head in profile shows one eye and no muzzle, which is why she
    /// walks sideways and looks at the reader. Zero faces out of the page.
    pub head_yaw: f32,
    /// Height above the floor from a pounce.
    pub hop: f32,
    /// 0 open, 1 shut.
    pub blink: f32,
    /// Breathing phase.
    pub breath: f32,
    /// Tail wave phase.
    pub tail: f32,
    /// How long until the next blink.
    pub next_blink: f32,
    /// How much sitting she has left in her: 0 wants to play, 1 wants to
    /// settle. It fills while she rests and empties while she plays, which is
    /// what stops a cat from either running for ever or sleeping all minute.
    pub rest: f32,
    /// Whether she has reached what she set out for.
    pub arrived: bool,
    /// How long she is willing to walk to get there, in seconds.
    pub patience: f32,
    /// How long the mood was going to last once she got there.
    pub dwell: f32,
    /// Whether the paw has already landed in this window.
    pub struck: bool,
    /// Downward speed, in cells per second, while she is in the air without a
    /// jump under her.
    pub vy: f32,
    /// Whether she is in the air on a jump, which is not the same as falling.
    pub airborne: bool,
    /// How far through that jump she is, 0 to 1.
    pub jump: f32,
    /// Where the jump started and where it is aimed.
    pub jump_from: [f32; 3],
    pub jump_to: [f32; 3],
    /// Whether she touched down this frame, so the page can be told.
    pub landed: bool,
    /// Seconds before she is ready to leave the ground again.
    ///
    /// A cat lands, gathers itself, and only then goes up: without this a chase
    /// after dust is a cat on a spring, hopping off the same spot every half
    /// second.
    pub next_leap: f32,
    /// Which way the legs of this swoop go: -1 left, 1 right, 0 not swooping.
    ///
    /// One direction per run. A cat that picked the longest leap from wherever it
    /// happened to land would bounce between the wall and the code for ever without
    /// getting anywhere; a run is a crossing, and it ends where the page does.
    pub swoop_dir: f32,
    /// Washing phase, in radians.
    pub wash: f32,
    /// What the paw is aimed at, if anything: the thing she walked over to hit.
    ///
    /// A paw goes where she is looking, not where she happens to be facing. Landing
    /// beside a table turned away from it and then swatting the air is what a cat
    /// does not do.
    pub aim: Option<[f32; 3]>,
    /// Seconds until the next thing she raises off the page.
    next_raise: f32,
    /// What she is raising this frame, if anything.
    pub effect: Option<Effect>,
    /// Where a run took the page apart this frame, if it did.
    ///
    /// A raking is not a strike: it happens a dozen times in a dash and it is aimed
    /// at whatever is under her paws rather than at a thing she chose, so it is
    /// reported as a place rather than as a swing of the paw.
    pub rake: Option<[f32; 3]>,
    /// How far she has come since the last rake, in characters.
    dash_flown: f32,
    /// Where her claws are in the page this frame, if she is going up it.
    ///
    /// A climb damages the line it is leaving rather than the place it is going, so
    /// what a climb reports is the row her back paws are in, not a target.
    pub claw: Option<[f32; 3]>,
    /// How far she has risen since the last pull of the claws, in cells.
    claw_climbed: f32,
    /// Whether she got as high as the page goes this frame, so the page can say so.
    pub topped: bool,
    /// Whether this climb has already said it, so the top is an event and not a
    /// state a mood sits in for a second and a half.
    topped_said: bool,
}

impl Default for Kitten {
    fn default() -> Self {
        Self::new(1.0, 1.0)
    }
}

impl Kitten {
    /// A kitten sitting at `x`, `y` on the page.
    pub fn new(x: f32, y: f32) -> Self {
        let pos = [x, y, 0.0];
        Self {
            pos,
            yaw: std::f32::consts::FRAC_PI_2,
            mood: Mood::Sit,
            timer: 3.0,
            target: pos,
            phase: 0.0,
            speed: 0.0,
            crouch: 0.0,
            sit: 0.0,
            head_yaw: 0.0,
            hop: 0.0,
            blink: 0.0,
            breath: 0.0,
            tail: 0.0,
            next_blink: 2.5,
            rest: 0.35,
            arrived: true,
            patience: 0.0,
            dwell: 0.0,
            struck: false,
            vy: 0.0,
            airborne: false,
            jump: 0.0,
            jump_from: pos,
            jump_to: pos,
            landed: false,
            next_leap: 0.0,
            swoop_dir: 0.0,
            wash: 0.0,
            aim: None,
            next_raise: 0.0,
            effect: None,
            rake: None,
            dash_flown: 0.0,
            claw: None,
            claw_climbed: 0.0,
            topped: false,
            topped_said: false,
        }
    }

    /// Whether she is running flat out, which is a different thing from moving.
    ///
    /// Read by the page to decide how much to make of it: a cat on her way
    /// somewhere gets a walk cycle, a cat at speed gets a trail.
    pub fn dashing(&self) -> bool {
        self.mood == Mood::Dash && self.speed > RUN + 0.5
    }

    /// Whether she is going fast enough to be leaving a streak on the page.
    ///
    /// The trail is drawn for a dash and for a lap that has got up to speed, and not
    /// for the turns in between: a cat dragging a rainbow round a corner would be a
    /// cat with a comet rather than a cat with legs.
    pub fn streaking(&self) -> bool {
        self.dashing() || (self.mood == Mood::Zoomies && self.speed > RUN)
    }

    /// Whether she is in the middle of a swoop, which is the page's cue to make the
    /// rainbow of a jump into the rainbow of a leap across the page.
    ///
    /// The mood rather than the air, because the two ends of a swoop matter as much as
    /// its middle: it takes off with a crouch, and it comes down into the code with the
    /// whole of the page's weight behind it.
    pub fn swooping(&self) -> bool {
        self.mood == Mood::Swoop
    }

    /// Whether what she is doing rakes the line she runs along.
    ///
    /// Both of the fast moods do it. A lap is the same animal in the same hurry as a
    /// dash, only with the far end of the page rather than a straight line as its
    /// goal, so a lap that left the text alone would be the cat running round a room
    /// without touching anything in it.
    pub fn raking(&self) -> bool {
        matches!(self.mood, Mood::Dash | Mood::Zoomies)
    }

    /// How tall the kitten stands, in lines.
    ///
    /// Measured off the boxes rather than off a constant, because the ears are the
    /// top and a silhouette resized by hand is exactly what a stale constant lies
    /// about.
    pub fn height(&self) -> f32 {
        self.blocks()
            .iter()
            .map(|block| block.pos[1] + block.size[1] * 0.5 - self.pos[1])
            .fold(0.0f32, f32::max)
    }

    /// Where a paw lands when it strikes.
    ///
    /// In front of the nose along the page, at the height of the cat's own
    /// shoulder, because a paw that reached the top of its head would look like a
    /// cat striking a wasp. When there is something to hit, the paw goes to that
    /// side of her and no higher than her shoulders: the aim is the thing, the reach
    /// is the cat's.
    pub fn paw(&self) -> [f32; 3] {
        let (sin, _cos) = self.yaw.sin_cos();
        let forward = 1.2;
        let mut at = [
            self.pos[0] + forward * sin,
            self.pos[1] + 0.4 + self.hop,
            self.pos[2],
        ];
        if let Some(aim) = self.aim {
            at[0] = self.pos[0] + forward * (aim[0] - self.pos[0]).signum();
            at[1] = aim[1].clamp(self.pos[1] + 0.2, self.pos[1] + 1.0) + self.hop;
        }
        at
    }

    /// Keeps the cat on a page of this size, which is what a resize asks for.
    pub fn clamp_to(&mut self, width: f32, rows: f32) {
        let (left, right, bottom, top) = room_to_walk(width, rows);
        self.pos[0] = self.pos[0].clamp(left, right);
        self.pos[1] = self.pos[1].clamp(bottom, top);
        self.target[0] = self.target[0].clamp(left, right);
        self.target[1] = self.target[1].clamp(bottom, top);
    }

    /// One step of behaviour, then one step of movement.
    ///
    /// Returns true on the frame the kitten strikes, so the caller can knock a
    /// token about.
    pub fn think(&mut self, dt: f32, senses: &mut Senses<'_>) -> bool {
        self.breath += dt;
        self.tail += dt * if self.mood == Mood::Sleep { 0.6 } else { 2.4 };
        self.timer -= dt;
        self.landed = false;
        self.effect = None;
        self.rake = None;
        self.claw = None;
        self.topped = false;
        self.wash += dt
            * if matches!(self.mood, Mood::Clean | Mood::Groom) {
                1.0
            } else {
                0.0
            };
        // What she raises off the page: a Zzz while she sleeps, a sparkle while she
        // washes. Both are the page's own glyphs, so even what she feels is printed.
        if self.mood == Mood::Sleep {
            self.next_raise -= dt;
            if self.next_raise <= 0.0 {
                self.next_raise = 0.9 + senses.rng.range(0.0, 0.4);
                self.effect = Some(Effect::Zzz);
            }
        } else if matches!(self.mood, Mood::Clean | Mood::Groom) && self.sit > 0.6 {
            self.next_raise -= dt;
            if self.next_raise <= 0.0 {
                self.next_raise = 0.7;
                self.effect = Some(Effect::Lick);
            }
        } else {
            self.next_raise = 0.0;
        }

        // Blinking runs on its own clock: a cat that blinked only when it changed
        // its mind would look painted on.
        self.next_blink -= dt;
        if self.next_blink <= 0.0 {
            self.blink = 1.0;
            self.next_blink = 2.0 + senses.rng.range(0.0, 4.0);
        }
        self.blink = (self.blink - dt * 9.0).max(0.0);

        // Sleep wins over everything: nothing has happened for three minutes.
        if senses.input_age > SLEEP_AFTER {
            if self.mood != Mood::Sleep {
                self.mood = Mood::Sleep;
                self.timer = f32::MAX;
                self.speed = 0.0;
            }
        } else if self.mood == Mood::Sleep {
            self.mood = Mood::Sit;
            self.timer = 1.2;
        }

        if self.mood != Mood::Sleep {
            self.choose(dt, senses);
        }
        self.move_by(dt, senses);
        // How much sitting she has left in her: it fills while she rests and
        // empties while she plays, and it is one number rather than two because
        // a cat is not rested and restless at once.
        let change = match self.mood {
            Mood::Sit | Mood::Groom | Mood::Sleep => 0.05,
            // A leap across the page is the most tiring thing there is, and it is where
            // the mood goes afterwards that matters: a cat that has just flown the width
            // of the paper wants a wash, not another flight.
            Mood::Swoop => -0.18,
            Mood::Swat
            | Mood::Chase
            | Mood::Zoomies
            | Mood::Pounce
            | Mood::Bat
            | Mood::Climb => -0.11,
            // A dash is the most tiring thing she does, which is what makes the
            // wash afterwards land: a cat that has just crossed the page at a
            // gallop does not start another one.
            Mood::Dash => -0.2,
            _ => 0.0,
        };
        self.rest = (self.rest + change * dt).clamp(0.0, 1.0);
        // The head follows her intention, and comes back to the reader when she
        // has none: a face turned away is a cat with no face.
        let want = match self.mood {
            Mood::Follow | Mood::Pounce | Mood::Dash | Mood::Swoop
                if self.target[0] > self.pos[0] =>
            {
                0.34
            }
            Mood::Follow | Mood::Pounce | Mood::Dash | Mood::Swoop => -0.34,
            Mood::Sleep => 0.30,
            _ => 0.0,
        };
        self.head_yaw += (want - self.head_yaw) * (dt * 2.4).min(1.0);
        // One strike per window, not one per frame of it: eight frames of paw on
        // the table is one swat and thirty-two specks of dust.
        let strike = self.striking() && !self.struck;
        self.struck |= strike;
        strike
    }

    /// Puts the paw in the air: it comes down on the next frame that has the
    /// clock in the window, and only once.
    fn arm(&mut self) {
        self.timer = 0.41;
        self.struck = false;
    }

    /// Whether the paw is in the window to land.
    ///
    /// A strike is a window in the mood's own timer rather than an event with a
    /// clock of its own: the paw arrives a fixed time after the decision, which
    /// is what lines the swing up with the walk that brought her there.
    fn striking(&self) -> bool {
        !self.airborne
            && matches!(self.mood, Mood::Bat | Mood::Swat | Mood::Chase | Mood::Dash)
            && self.timer > 0.28
            && self.timer < 0.42
    }

    /// Picks what to do next.
    fn choose(&mut self, dt: f32, senses: &mut Senses<'_>) {
        let pointer = senses.pointer.filter(|_| senses.pointer_age < 1.6);
        if let Some(pointer) = pointer {
            let distance = self.distance_to(pointer);
            // A pointer that is still moving is the most interesting thing on the
            // page, so it overrides whatever the cat was doing. One that has been
            // sitting still does not take her off a word she is already batting.
            let fresh = senses.pointer_age < 0.5;
            if fresh || self.mood != Mood::Bat || distance < 1.4 {
                self.target = pointer;
                if distance > 3.2 && senses.pointer_age < 0.5 {
                    if self.mood != Mood::Pounce {
                        self.mood = Mood::Pounce;
                        self.timer = 0.55;
                        self.crouch = 1.0;
                    }
                } else if self.mood != Mood::Pounce && self.mood != Mood::Bat {
                    self.mood = Mood::Follow;
                }
                // The clock does not run out on a reader's pointer. Otherwise a cat on
                // its way to the mouse decides to go up the wall instead, and what the
                // reader sees is a cat that ignored the mouse.
                self.timer = self.timer.max(0.6);
            }
        }

        // Reaching the pointer: something happens, chosen from the generator so
        // two sessions do not do the same thing.
        if self.mood == Mood::Follow && self.distance_to(self.target) < 0.8 {
            let pick = senses.rng.below(3);
            self.start(
                [Mood::Bat, Mood::Sit, Mood::Groom][pick],
                senses,
            );
            self.timer = 0.9 + senses.rng.range(0.0, 1.6);
        }

        // A cat with somewhere to be goes there: the clock only starts running
        // out once she has arrived, and `patience` is the limit on that, so a
        // thing she cannot reach is given up on instead of walked at for ever.
        let reach = self.distance_to(self.target);
        match self.mood {
            // A dash is over when its own clock says so, not when it gets there: a
            // run that waited for its target would slow down at the end, and the
            // one thing a run cannot do is hesitate. Getting there early only means
            // the paw comes down on what she found.
            Mood::Dash if reach <= 1.0 && !self.struck => {
                // The leg has been run. A dash with time left on its clock takes
                // another, straight on, instead of stopping dead in the middle of the
                // page: what a reader sees is one run with several legs in it, and
                // every leg ends among characters rather than in the margin.
                if self.timer > 1.2 {
                    self.target = self.run_again(senses);
                } else {
                    self.arm();
                }
            }
            // A leg that was never reached still ends in a landing: the run is over
            // when the clock is, and a cat that stops at a gallop puts a paw down.
            Mood::Dash if self.timer < 0.5 && !self.struck && !self.airborne => self.arm(),
            // A climb is not one bound but the run of them: she is on her way to a
            // foothold, and the clock does not run out while she is between two of
            // them. A place she cannot get to -- because the claws took it as she came
            // -- is given up on by the patience, the same as anything else.
            Mood::Climb if !self.arrived => {
                if self.airborne {
                    self.timer = self.timer.max(1.6);
                } else if reach > 1.0 && self.patience > 0.0 {
                    self.patience -= dt;
                    self.timer = self.timer.max(1.6);
                }
            }
            // Her paws are down again, and the next foothold is the one above this
            // one. When there is none, she is as high as the page goes.
            Mood::Climb => match self.ascent(senses) {
                Some(next) => {
                    self.target = next;
                    self.arrived = false;
                    self.patience = 20.0;
                    self.timer = self.timer.max(1.6);
                }
                None => {
                    // A beat at the top rather than the rest of the clock: what is
                    // worth seeing is that she got there.
                    if self.timer > 1.2 {
                        self.timer = 1.2;
                    }
                    if !self.topped_said {
                        self.topped_said = true;
                        self.topped = true;
                    }
                    // And then off it. The top of a climb is the one place on the page
                    // where the whole of it is below her and to one side, so a cat that
                    // has just got up there leaves it the long way.
                    if self.swoop_target(senses).is_some() {
                        self.start(Mood::Swoop, senses);
                    }
                }
            },
            // A swoop is a run of legs, not one leap: she is on her way to the far side
            // of the page, and the clock does not run out between two of them.
            Mood::Swoop if !self.arrived => {
                if self.airborne {
                    self.timer = self.timer.max(0.9);
                } else if self.patience > 0.0 {
                    self.patience -= dt;
                    self.timer = self.timer.max(0.9);
                }
            }
            // Paws down on the far side: another leg, further left and lower, or the run
            // through what is left of the code.
            Mood::Swoop => match self.swoop_side(senses, self.swoop_dir) {
                Some((next, _)) => {
                    self.target = next;
                    self.arrived = false;
                    self.patience = 10.0;
                    self.timer = self.timer.max(1.2);
                }
                // Nowhere further to swoop to: the page has run out on that side. A
                // super jump that ended on the furniture was a way back up to the wall,
                // and what a cat does there is climb it; one that ended in the code is a
                // cat among the characters, and what it does there is run along them.
                None => {
                    let on_the_wall = self.pos[0] > senses.page[0] * 0.72;
                    if on_the_wall {
                        self.start(Mood::Climb, senses);
                    } else {
                        self.start(Mood::Dash, senses);
                    }
                }
            },
            Mood::Bat | Mood::Swat | Mood::Perch | Mood::Chase if !self.arrived => {
                if self.airborne {
                    // She is committed: the clock stops until she is down, and what
                    // she gets up to in the air is the leap's business.
                    self.timer = self.timer.max(0.6);
                } else if reach > 1.0 && self.patience > 0.0 {
                    self.patience -= dt;
                    self.timer = self.timer.max(0.6);
                } else if reach <= 1.0 {
                    self.arrived = true;
                    match self.mood {
                        // The paw lands on the thing as she gets there.
                        Mood::Bat | Mood::Swat => self.arm(),
                        // Sitting on the thing starts when she is on it.
                        _ => self.timer = self.dwell,
                    }
                }
            }
            // Dust that a paw has already thrown is worth another go while the
            // mood has time left in it.
            Mood::Chase if self.timer > 0.5 && reach < 1.0 => self.arm(),
            _ => {}
        }

        // A chase follows the dust it picked, live: dust does not sit still, and
        // a cat that ran to where a speck was is a cat pouncing on nothing.
        if self.mood == Mood::Chase {
            if let Some(mote) = nearest_mote(senses.motes, self.pos) {
                self.target = mote.pos;
            }
        }

        if self.timer > 0.0 {
            return;
        }
        // A mood ran out: choose another from what there is to do.
        let mood = self.next_mood(senses);
        self.start(mood, senses);
    }

    /// What every mood is worth right now, which is the whole character of the
    /// animal: a cat with plans wanders, a tired one sits, and a page with nothing
    /// on it is a page where nothing happens.
    ///
    /// The table is a function of its own so that it can be read without running
    /// the generator: what a cat wants after a dash is a fact about the cat, and a
    /// test should be able to ask about it rather than sample a distribution and
    /// hope.
    fn weights(&self, senses: &Senses<'_>) -> [(Mood, usize); 12] {
        let props = !senses.props.is_empty();
        let motes = !senses.motes.is_empty();
        let tokens = !senses.tokens.is_empty();
        // Anything over her head at all, which is what a climb is for: on a page
        // with nothing above the cat there is nothing to go up.
        let above = foothold_above(senses, self.pos).is_some();
        // Tired enough to sit down. It was lower, and what it cost was a cat that
        // sat through the interesting half of the page: a playground earns its name
        // from what she does when she is not tired, so the bar for resting is high
        // and the rests themselves are short.
        let idle = self.rest > 0.75;
        // A cat that has just crossed the page at a gallop has had its play. What
        // it wants next is a wash, and this is where that is said: the three
        // washing moods go up together because sitting still and licking a paw is
        // one behaviour in three lengths.
        let spent = self.mood == Mood::Dash;
        // A perch she is already on is not somewhere to go: without this, a cat
        // on a table keeps choosing the table and sits there until the page is
        // rebuilt.
        let on_a_perch = nearest_perch(senses.props, self.pos, f32::MAX)
            .map(|prop| (self.pos[1] - prop.top()).abs() < 1.2)
            .unwrap_or(false);
        // A cat wants up, always. The page's way out is over her head, the wall of
        // furniture runs the whole height of the right of it, and the floor is the one
        // place with nothing to recommend it -- so the moods that go up are worth more
        // the lower she is, half again as much on the rug as at the top of the page, and
        // what she does on the floor is leave it.
        let low = (1.0 - self.pos[1] / senses.page[1].max(1.0)).clamp(0.0, 1.0);
        let up = |weight: usize| (weight as f32 * (1.0 + low * 0.6)).round() as usize;
        let mut weights: [(Mood, usize); 12] = [
            (Mood::Wander, if idle { 5 } else { 3 }),
            (Mood::Sit, if spent { 8 } else if idle { 5 } else { 1 }),
            (Mood::Groom, if spent { 8 } else if idle { 2 } else { 1 }),
            (Mood::Clean, if spent { 10 } else if idle { 3 } else { 1 }),
            (Mood::Swat, usize::from(props) * if idle { 1 } else { 5 }),
            (
                Mood::Perch,
                up(usize::from(props && !on_a_perch) * if idle { 2 } else { 6 }),
            ),
            (Mood::Chase, usize::from(motes) * if idle { 1 } else { 5 }),
            // A lap is a serious thing: the whole page, at speed, taking what is
            // along the way. It used to be the quiet option among the running moods
            // and it read as a cat walking fast for no reason.
            (Mood::Zoomies, if idle { 1 } else { 8 }),
            // Going up. A page of text is a cliff -- a hundred and twenty columns of
            // it stacked to the top edge -- and a cat that can climb it and take it
            // apart on the way is the whole of what this mode is for. Tired, she would
            // rather not, which is why the weight falls with the rest of her.
            (Mood::Climb, up(usize::from(above) * if idle { 3 } else { 12 })),
            (Mood::Bat, usize::from(tokens) * 3),
            // Leaving the top of the page the long way. It is worth what the page below
            // her is worth: a cat with the whole width of it under her feet and one leap
            // after another in her is the middle of the circuit the wall is there for.
            (
                Mood::Swoop,
                up(usize::from(self.swoop_target(senses).is_some()) * if idle { 1 } else { 7 }),
            ),
            // A dash is worth what there is to run past, because crossing an empty
            // margin takes her nowhere in particular. It is the most likely thing
            // on a full page for a cat with energy left, which is what makes the
            // page feel like it is being played in rather than decorated.
            (
                Mood::Dash,
                if tokens {
                    if idle {
                        3
                    } else {
                        12
                    }
                } else {
                    usize::from(props) * if idle { 1 } else { 2 }
                },
            ),
        ];
        // A cat does not do the same thing twice in a row. Wandering, sitting and
        // grooming are the exceptions, because a cat does those all afternoon.
        if !matches!(
            self.mood,
            Mood::Wander | Mood::Sit | Mood::Groom | Mood::Clean
        ) {
            for (mood, weight) in &mut weights {
                if *mood == self.mood {
                    *weight = 0;
                }
            }
        }
        weights
    }

    /// Chooses the next thing to do, from what the page holds and how much
    /// sitting she has left in her.
    fn next_mood(&mut self, senses: &mut Senses<'_>) -> Mood {
        let weights = self.weights(senses);
        let total: usize = weights.iter().map(|(_, weight)| weight).sum();
        let mut at = senses.rng.below(total.max(1));
        let mut picked = Mood::Wander;
        for (mood, weight) in weights {
            if at < weight {
                picked = mood;
                break;
            }
            at -= weight;
        }
        picked
    }

    /// Begins `mood`, with whatever that mood needs to know: where the thing it
    /// is interested in is, and how long the interest lasts.
    fn start(&mut self, mood: Mood, senses: &mut Senses<'_>) {
        let (left, right, bottom, top) = room_to_walk(senses.page[0], senses.page[1]);
        self.mood = mood;
        self.arrived = false;
        self.patience = if matches!(
            mood,
            Mood::Bat | Mood::Swat | Mood::Perch | Mood::Chase | Mood::Climb | Mood::Swoop
        ) {
            10.0
        } else {
            0.0
        };
        self.timer = match mood {
            Mood::Wander => 1.6 + senses.rng.range(0.0, 2.6),
            // Sitting still is the one thing a playground cannot afford much of.
            Mood::Sit | Mood::Groom | Mood::Clean => 1.4 + senses.rng.range(0.0, 2.2),
            Mood::Perch => 2.4 + senses.rng.range(0.0, 3.6),
            Mood::Bat => 1.0 + senses.rng.range(0.0, 1.4),
            Mood::Swat => 1.4 + senses.rng.range(0.0, 0.9),
            Mood::Chase => 3.0 + senses.rng.range(0.0, 2.5),
            // Long enough to cross the page and come back, which is what a lap is.
            Mood::Zoomies => 3.0 + senses.rng.range(0.0, 2.4),
            // Only a floor: the clock is refreshed at every foothold, so what ends a
            // climb is the page running out and not this.
            Mood::Climb => 5.0 + senses.rng.range(0.0, 3.0),
            // A run of legs, so only a floor: the clock is refreshed at every landing.
            Mood::Swoop => 3.6 + senses.rng.range(0.0, 2.0),
            Mood::Dash => 2.2 + senses.rng.range(0.0, 1.4),
            Mood::Follow | Mood::Pounce | Mood::Sleep => 1.2,
        };
        self.dwell = self.timer;
        self.struck = false;
        self.aim = None;
        self.topped_said = false;
        // A mood that goes nowhere has already arrived.
        self.arrived = !matches!(
            mood,
            Mood::Bat | Mood::Swat | Mood::Perch | Mood::Chase | Mood::Climb | Mood::Swoop
        );
        match mood {
            // Somewhere else on the page: a cat that walked to a point and stayed
            // there would be a cat on a short leash.
            Mood::Wander => {
                self.target = self.elsewhere(senses);
            }
            Mood::Zoomies => {
                self.target = self.lap_target(senses);
            }
            Mood::Swat => match nearest_prop(senses.props, self.pos) {
                Some(prop) => {
                    self.aim = Some(prop.pos);
                    // Beside it, at the foot of it, on the side she is already on:
                    // a cat swats what is next to it, not what is over its head, and
                    // a paw that has walked into a table is drawn through a table.
                    let side = if self.pos[0] < prop.pos[0] { -1.0 } else { 1.0 };
                    let reach = prop.size[0] * GLYPH_WIDTH * 0.5 + 1.2;
                    let base = prop.pos[1] - prop.size[1] * 0.5;
                    // The foot of it is where she stands to swat it: a plant on a
                    // table is reached from the table, and one on the rug from the
                    // rug, rather than from a point in the air beside its pot.
                    let at = prop.pos[0] + side * reach;
                    let stand = senses.surface(at, base + 0.2).max(senses.floor);
                    self.target = [at, stand, 0.0];
                }
                None => self.mood = Mood::Wander,
            },
            // A line above is something to jump on, and a table is something to
            // jump on: whichever is nearer is the thing she climbs.
            Mood::Perch => {
                if let Some(shelf) = nearest_shelf_above(senses.shelves, self.pos) {
                    self.target = [shelf.nearest_point(self.pos[0]), shelf.y, 0.0];
                } else if let Some(prop) = nearest_perch(senses.props, self.pos, CLIMB) {
                    self.target = [prop.pos[0], prop.top(), 0.0];
                } else {
                    self.mood = Mood::Wander;
                }
            }
            Mood::Bat => {
                if let Some(token) = nearest_token(senses.tokens, self.pos) {
                    self.target = token.pos;
                    self.aim = Some(token.pos);
                }
            }
            Mood::Chase => {
                if let Some(mote) = nearest_mote(senses.motes, self.pos) {
                    self.target = mote.pos;
                    self.aim = Some(mote.pos);
                }
            }
            Mood::Dash => {
                self.target = self.run_target(senses);
            }
            Mood::Climb => match self.ascent(senses) {
                Some(at) => self.target = at,
                // Nothing over her head after all: a cat with no cliff to climb
                // walks instead of gathering itself under the sky.
                None => self.mood = Mood::Wander,
            },
            Mood::Swoop => match self.swoop_target(senses) {
                Some((at, dir)) => {
                    self.target = at;
                    self.swoop_dir = dir;
                }
                // No far side to leave it for: a cat in the corner of the page has
                // nowhere to swoop to and wanders off instead.
                None => self.mood = Mood::Wander,
            },
            _ => {}
        }
        self.target[0] = self.target[0].clamp(left, right);
        self.target[1] = self.target[1].clamp(bottom, top);
    }

    /// Where a dash goes: the far end of the length of page she is standing on.
    ///
    /// A dash is a horizontal thing, and the point of it is the run, so what it aims
    /// at has to be on her own level. A cat that picked the farthest thing on the
    /// page would pick a shelf nine lines up, run to the point under it and stand
    /// there -- which is what the first version did, and it looked like a cat that
    /// had forgotten what it was doing.
    ///
    /// Everything on her level is a candidate: the two ends of the ledge under her,
    /// loose words at about her height, and furniture she could run past. The
    /// farthest one that she can actually stand at wins, and a page with nothing on
    /// her level gives her the run anyway, short and pointless, as a cat's is.
    fn run_target(&mut self, senses: &mut Senses<'_>) -> [f32; 3] {
        let (left, right, _bottom, _top) = room_to_walk(senses.page[0], senses.page[1]);
        let here = self.pos;
        let mut reachable: Vec<f32> = Vec::new();
        for shelf in senses.shelves {
            if (shelf.y - here[1]).abs() > 0.35 {
                continue;
            }
            reachable.push(shelf.span[0].clamp(left, right));
            reachable.push(shelf.span[1].clamp(left, right));
        }
        for token in senses.tokens.iter().filter(|token| token.rest) {
            if (token.pos[1] - here[1]).abs() < 2.2 {
                reachable.push(token.pos[0].clamp(left, right));
            }
        }
        for prop in senses.props {
            if (prop.pos[1] - here[1]).abs() < 3.0 {
                reachable.push(prop.pos[0].clamp(left, right));
            }
        }
        let mut best = [here[0], here[1], 0.0];
        let mut longest = 0.0;
        for x in reachable {
            // A ledge that is not under the place is not a place she can run to:
            // the surface at `x` is what her paws would land on.
            let stand = senses.surface(x, here[1] + 0.05);
            if (stand - here[1]).abs() > 0.6 {
                continue;
            }
            let far = (x - here[0]).abs();
            if far > longest {
                longest = far;
                best = [x, stand, 0.0];
            }
        }
        // The floor of the room is a rug of dots and not a line of code, and a run
        // along it takes nothing. Everything a run is for is over her head, so a dash
        // from the rug climbs first and runs there.
        if let Some(spring) = self.floor_spring(senses) {
            return spring;
        }
        if longest < 5.0 {
            let elsewhere = self.elsewhere(senses);
            return [elsewhere[0], elsewhere[1], 0.0];
        }
        best
    }

    /// A point on the nearest line over her head, when she is standing on the rug
    /// rather than on a line.
    ///
    /// The rug is drawn in characters like everything else, at a third of the
    /// opacity, and it is not part of the page's text: running along it is a cat
    /// crossing a room, which is motion without consequence. So the two fast moods
    /// ask this first, and a run that starts on the floor starts by going up.
    fn floor_spring(&self, senses: &Senses<'_>) -> Option<[f32; 3]> {
        if self.pos[1] > senses.floor + 0.5 {
            return None;
        }
        let shelf = nearest_shelf_above(senses.shelves, self.pos)?;
        if shelf.y - self.pos[1] > CLIMB {
            return None;
        }
        Some([shelf.nearest_point(self.pos[0]), shelf.y, 0.0])
    }

    /// Where a lap goes next.
    ///
    /// A lap is the same animal as a dash and gets the same manners about the page:
    /// from the floor it climbs, and from a line it goes somewhere else on the page,
    /// which is what makes a lap a circuit of the text rather than a circuit of the
    /// margin under it.
    fn lap_target(&mut self, senses: &mut Senses<'_>) -> [f32; 3] {
        if let Some(spring) = self.floor_spring(senses) {
            return spring;
        }
        // Her own line, if she is standing on one. A point on another piece of text
        // at the same height is a point she would run at and never reach: the piece
        // ends first, and a lap that spends half of itself falling into the gap
        // between two pieces of a line is a lap of the margin.
        let here = self.pos;
        let own = senses.shelves.iter().find(|shelf| {
            (shelf.y - here[1]).abs() <= 0.35
                && here[0] >= shelf.span[0] - 0.6
                && here[0] <= shelf.span[1] + 0.6
        });
        if let Some(shelf) = own {
            let (left, right, _bottom, _top) = room_to_walk(senses.page[0], senses.page[1]);
            let lo = shelf.span[0].max(left);
            let hi = shelf.span[1].min(right);
            if hi - lo > 2.0 {
                let x = senses.rng.range(lo, hi);
                return [x, shelf.y, 0.0];
            }
        }
        let elsewhere = self.elsewhere(senses);
        [elsewhere[0], elsewhere[1], 0.0]
    }

    /// Where the next leg of a run goes, when the one she was on has been run out
    /// with time to spare.
    ///
    /// Straight on, because a cat at a gallop does not turn round: the leg is the far
    /// end of what is on her own level in the direction she is already travelling. One
    /// leg in three is a line over her head instead, which is how a run up the page
    /// happens at all -- she springs into the line above and takes that apart instead
    /// of stopping under it.
    fn run_again(&mut self, senses: &mut Senses<'_>) -> [f32; 3] {
        let (left, right, _bottom, _top) = room_to_walk(senses.page[0], senses.page[1]);
        let here = self.pos;
        let forward = if self.yaw.sin() >= 0.0 { 1.0 } else { -1.0 };
        if senses.rng.below(3) == 0 {
            if let Some(shelf) = nearest_shelf_above(senses.shelves, here) {
                let rise = shelf.y - here[1];
                let span = shelf.span[1] - shelf.span[0];
                if rise <= CLIMB && span > 6.0 {
                    return [shelf.nearest_point(here[0]), shelf.y, 0.0];
                }
            }
        }
        let mut best = [here[0] + forward * 6.0, here[1], 0.0];
        let mut longest = 0.0;
        for shelf in senses.shelves {
            if (shelf.y - here[1]).abs() > 0.35 {
                continue;
            }
            for edge in [shelf.span[0], shelf.span[1]] {
                let x = edge.clamp(left, right);
                // Only what is ahead of her: a leg that went back the way she came
                // would be a cat turning a corner at a gallop, which is a collision.
                if (x - here[0]) * forward < 0.5 {
                    continue;
                }
                let stand = senses.surface(x, here[1] + 0.05);
                if (stand - here[1]).abs() > 0.6 {
                    continue;
                }
                let far = (x - here[0]).abs();
                if far > longest {
                    longest = far;
                    best = [x, stand, 0.0];
                }
            }
        }
        best
    }

    /// The next foothold of a climb: the highest one bound can reach, with a place on
    /// it to land that is not where she is standing.
    ///
    /// Highest and not nearest, because the point of a climb is height: the page is a
    /// cliff of lines and a cat that took the next one up would take thirty bounds to
    /// cross it and would stop at every one. The sideways offset is what keeps the
    /// ascent from being a lift -- she leaves the line she is on at an angle and lands a
    /// couple of cells along the one above it, so what a reader sees is a cat zigzagging
    /// up the page rather than a cat on a rope.
    fn ascent(&mut self, senses: &mut Senses<'_>) -> Option<[f32; 3]> {
        let [lo, hi, y] = foothold_above(senses, self.pos)?;
        let way = if senses.rng.below(2) == 0 { -1.0 } else { 1.0 };
        let step = senses.rng.range(1.0, 2.2) * way;
        // Not the last character of the ledge, if there is a choice: a cat that lands
        // on the very end of a line is a cat one step from the end of it.
        let (lo, hi) = if hi - lo > 2.4 {
            (lo + 1.2, hi - 1.2)
        } else {
            (lo, hi)
        };
        Some([(self.pos[0] + step).clamp(lo, hi), y, 0.0])
    }

    /// Where a swoop goes: the far side of the page, on something she can stand on.
    ///
    /// A super jump is a leap between the two halves of the page -- the wall of furniture
    /// against the right edge and the code to the left of it -- so both are candidates,
    /// and both directions are tried. What wins is the leg nearest `SWOOP_LEG` long:
    /// long enough to cross, short enough to be one bound, because a cat that aimed at the
    /// extreme far corner would be leaping off the page rather than across it.
    ///
    /// A leg may also gain ground now, by up to `SUPER_RISE`, which is what lets a cat
    /// standing in the code come back up to the furniture. Ties are broken by height: a
    /// rise beats a fall, because going up is what the whole page is for.
    fn swoop_target(&self, senses: &Senses<'_>) -> Option<([f32; 3], f32)> {
        let mut best: Option<([f32; 3], f32, f32)> = None;
        for dir in [-1.0, 1.0] {
            let Some((at, score)) = self.swoop_side(senses, dir) else {
                continue;
            };
            if best.is_some_and(|(_, _, been)| score <= been) {
                continue;
            }
            best = Some((at, dir, score));
        }
        best.map(|(at, dir, _)| (at, dir))
    }

    /// The best leg in one direction: a landing on a line of code or on the flat top of a
    /// piece of furniture, on the far side of the page from where she stands.
    fn swoop_side(&self, senses: &Senses<'_>, dir: f32) -> Option<([f32; 3], f32)> {
        let (left, right, _bottom, _top) = room_to_walk(senses.page[0], senses.page[1]);
        let mut best: Option<([f32; 3], f32)> = None;
        // The two halves of the page are the two ends of a super jump: a leap to the
        // right lands on the furniture against the wall, and a leap to the left comes
        // down in the code. A leap that stayed on its own side would be a jump along a
        // shelf, which is what a dash is for.
        let mut ledges: Vec<([f32; 2], f32)> = Vec::new();
        if dir > 0.0 {
            for prop in senses.props.iter().filter(|prop| prop.kind.is_perch()) {
                let half = prop.size[0] * GLYPH_WIDTH * 0.5;
                ledges.push(([prop.pos[0] - half, prop.pos[0] + half], prop.top()));
            }
        } else {
            ledges.extend(senses.shelves.iter().map(|shelf| (shelf.span, shelf.y)));
        }
        // A leap into the code is the way down -- leaving the top of the wall is leaving
        // it -- and a leap on to the furniture is the way back up, so that one may gain
        // what a super jump gains.
        let (least, most) = if dir > 0.0 {
            (-SUPER_RISE, SWOOP_DROP)
        } else {
            (0.8, SWOOP_DROP)
        };
        for (span, y) in ledges {
            let drop = self.pos[1] - y;
            if drop < least || drop > most {
                continue;
            }
            let lo = span[0].max(left) + 0.4;
            let hi = span[1].min(right) - 0.4;
            if hi <= lo {
                continue;
            }
            // As far as one leg reaches, or the end of the ledge if that is nearer. The end
            // of a line is where most legs come down, because that is where the code stops
            // and a swoop is a leap from one edge of the writing to the other.
            let x = (self.pos[0] + dir * SWOOP_LEG).clamp(lo, hi);
            let across = (x - self.pos[0]) * dir;
            if across < SWOOP_MIN {
                continue;
            }
            let score = -(across - SWOOP_LEG).abs() - drop * 0.2;
            if best.is_some_and(|(_, been)| score <= been) {
                continue;
            }
            best = Some(([x, y, 0.0], score));
        }
        best
    }

    /// A point on a ledge, a few cells away: a cat crosses the text along the
    /// lines, and steps off the end of one into the line below.
    ///
    /// Every walk is on a surface, because a page whose cat could stand anywhere
    /// would be a page with nothing to stand on.
    fn elsewhere(&mut self, senses: &mut Senses<'_>) -> [f32; 3] {
        let (left, right, bottom, top) = room_to_walk(senses.page[0], senses.page[1]);
        let floor = senses.floor.clamp(bottom, top);
        // Her own line or one below it: going up is what the jump is for.
        let mut places: Vec<[f32; 3]> = vec![[0.0, floor, 0.0]];
        for shelf in senses.shelves {
            if shelf.y > self.pos[1] + 0.3 || shelf.y < bottom {
                continue;
            }
            let lo = shelf.span[0].max(left);
            let hi = shelf.span[1].min(right);
            if hi > lo {
                places.push([(lo + hi) * 0.5, shelf.y, 0.0]);
            }
        }
        // Under the text when there is text over her: a cat that wanders out into
        // the blank half of the page has nothing above its head to jump at, and a
        // page is mostly margin.
        let mut over: Vec<[f32; 2]> = Vec::new();
        for shelf in senses.shelves {
            if shelf.y <= self.pos[1] + 0.6 {
                continue;
            }
            let span = [shelf.span[0].max(left), shelf.span[1].min(right)];
            if span[1] > span[0] {
                over.push(span);
            }
        }
        let mut point = places[0];
        for _ in 0..9 {
            let pick = places[senses.rng.below(places.len())];
            let x = if pick[1] <= senses.floor + 0.01 && !over.is_empty() {
                let span = over[senses.rng.below(over.len())];
                senses.rng.range(span[0], span[1])
            } else {
                senses.rng.range(left, right)
            };
            point = [
                if pick[0] == 0.0 { x } else { (pick[0] + x) * 0.5 },
                pick[1],
                0.0,
            ];
            if self.distance_to(point) > 4.0 {
                break;
            }
        }
        point
    }

    /// Walks, runs, jumps or falls toward the target.
    ///
    /// Two motions share the paws. Along the page she goes where the mood says;
    /// down the page the page itself decides, because the ledges are what she
    /// stands on. A cat that walks off the end of a line drops to the one under it,
    /// and a cat that wants the line above has to jump: that is the whole
    /// difference between a page with lines on it and a picture of a page.
    fn move_by(&mut self, dt: f32, senses: &mut Senses<'_>) {
        let goal = match self.mood {
            // The moods that are going somewhere.
            Mood::Follow
            | Mood::Pounce
            | Mood::Wander
            | Mood::Bat
            | Mood::Swat
            | Mood::Chase
            | Mood::Perch
            | Mood::Zoomies
            | Mood::Climb
            | Mood::Dash
            | Mood::Swoop => self.target,
            // Sitting, grooming and washing: stay put.
            _ => self.pos,
        };
        // What is below her is not reached by standing over it. She walks to the end
        // of the line she is on and steps off, and the page does the rest: without
        // this a cat that wants a table under her shelf waits out its patience
        // standing exactly on top of it.
        let mut goal = goal;
        if goal[1] < self.pos[1] - 1.2 && (goal[0] - self.pos[0]).abs() < 1.2 {
            if let Some(edge) = ledge_end(senses, self.pos) {
                goal = [edge, self.pos[1], 0.0];
            }
        }
        // A foothold the claws have already taken is not a foothold: a climb re-aims
        // at whatever is over her head now, instead of gathering itself under a hole
        // and spending its patience there.
        if self.mood == Mood::Climb
            && !self.airborne
            && senses.surface(goal[0], goal[1] + 0.05) < goal[1] - 0.05
        {
            if let Some(next) = self.ascent(senses) {
                self.target = next;
                goal = next;
            }
        }
        let dx = goal[0] - self.pos[0];
        let dy = goal[1] - self.pos[1];
        let across = dx.hypot(dy);
        let (left, right, bottom, top) = room_to_walk(senses.page[0], senses.page[1]);
        let floor = senses.floor.clamp(bottom, top);

        if self.airborne {
            self.step_leap(dt);
            self.blend_crouch(dt);
            self.clamp_to_room(left, right, top, floor);
            return;
        }
        self.next_leap = (self.next_leap - dt).max(0.0);

        let wanted = match self.mood {
            // Nothing else is allowed to be faster than a dash: it is the top of
            // the range, and a pounce that outran it would read as the wrong event.
            Mood::Dash => DASH,
            Mood::Pounce => RUN * 1.25,
            Mood::Zoomies => ZOOM,
            Mood::Chase if across > 2.0 => RUN,
            Mood::Chase => WALK * 1.6,
            Mood::Follow if across > 3.0 => RUN,
            Mood::Swat | Mood::Perch if across > 5.0 => RUN,
            // A climb walks and bounds: the sideways part is a step and a half, and
            // the way up is the jump's business. There is no such thing as a cat that
            // runs up a wall.
            Mood::Follow
            | Mood::Wander
            | Mood::Bat
            | Mood::Swat
            | Mood::Perch
            | Mood::Climb
            | Mood::Swoop => WALK,
            _ => 0.0,
        };
        // Arriving is not the end of a walk: a cat that stops dead where it was
        // going stands in the middle of the page for the rest of its mood. A lap
        // or a patrol picks the next somewhere instead.
        if self.mood == Mood::Zoomies && across < 0.8 {
            self.target = self.lap_target(senses);
        } else if self.mood == Mood::Wander && across < 0.8 {
            self.target = self.elsewhere(senses);
        }
        // A swoop is not a walk to somewhere and then a jump: the leap is the whole of
        // it. She gathers herself where she stands and goes, and what is under her at
        // the far end is where she lands -- which is why the place is asked about again
        // here rather than trusted from the moment the mood began. A page whose code is
        // still being printed back has holes in it, and a leg aimed at one is a leg into
        // the middle of the page.
        if self.mood == Mood::Swoop && across > SWOOP_MIN * 0.5 && self.patience > 0.0 {
            let holds = senses.surface(goal[0], goal[1] + 0.05) >= goal[1] - 0.05;
            if self.next_leap > 0.0 || !holds {
                self.crouch = 1.0;
                self.step_gravity(dt, senses);
                self.blend_crouch(dt);
                self.clamp_to_room(left, right, top, floor);
                return;
            }
            self.leap(goal);
            self.next_leap = 0.12 + senses.rng.range(0.0, 0.1);
            self.step_leap(dt);
            self.blend_crouch(dt);
            self.clamp_to_room(left, right, top, floor);
            return;
        }
        // A line above is not walked up. When she is under what she wants and it is
        // over her head, she crouches and leaves the ground.
        //
        // A jump needs something to come down on, and the paw takes a line apart
        // where it lands: the place she was going can be a hole by the time she
        // stands under it. Jumping at a hole again and again is bouncing in place,
        // so a place is only leapt at while the page still holds it up. Dust and a
        // word in the air are the exception: those are caught on the way up.
        let holds = senses.surface(goal[0], goal[1] + 0.05) >= goal[1] - 0.05;
        let overhead = matches!(self.mood, Mood::Bat | Mood::Chase | Mood::Dash);
        // A climb is allowed the whole of its reach: its foothold is picked for height
        // and can lean most of a bound across the page, and a cat that walked that
        // difference would be a cat walking off the end of a line.
        let bound = if self.mood == Mood::Climb { REACH } else { 2.4 };
        if dy > 0.6
            && dy <= CLIMB
            && dx.abs() < bound
            && (holds || overhead)
            && self.mood != Mood::Sleep
        {
            if self.next_leap > 0.0 {
                // Gathering herself under it: the crouch before the spring is
                // most of what tells a reader that a jump is being aimed.
                self.crouch = 1.0;
                self.step_gravity(dt, senses);
                self.blend_crouch(dt);
                self.clamp_to_room(left, right, top, floor);
                return;
            }
            self.leap(goal);
            self.next_leap = 0.3 + senses.rng.range(0.0, 0.2);
            self.step_leap(dt);
            self.blend_crouch(dt);
            self.clamp_to_room(left, right, top, floor);
            return;
        }
        let speed = if dx.abs() < 0.35 { 0.0 } else { wanted };
        // Approach the wanted speed smoothly, so the first frame of a run is not
        // an instant jump.
        self.speed += (speed - self.speed) * (dt * 6.0).min(1.0);
        let mut travel = 0.0;
        if dx.abs() > 0.02 {
            // Facing is horizontal whatever way it is going: a cat that turned to
            // face up the page would be a cat seen end on, which is a blob.
            self.yaw = if dx > 0.0 {
                std::f32::consts::FRAC_PI_2
            } else {
                -std::f32::consts::FRAC_PI_2
            };
            travel = (self.speed * dt).min(dx.abs());
            self.pos[0] += dx.signum() * travel;
        }
        // A dash does not swing: it takes the line apart as it goes, a rake every
        // few characters. Reported behind her rather than under her, because a rake
        // under her own paws would cut the ledge out from under her and drop her
        // mid-dash -- what she leaves is a furrow, and a furrow is behind.
        //
        // The line she runs along is half a line below her feet: her paws are on
        // top of the glyphs, which is where the cat stands, so the place a rake is
        // reported at is the row of text and not the surface she is held up by.
        //
        // Far enough behind her that the swathe ends under her tail and not under
        // her paws. A rake reaches further than a paw, and a furrow that took the
        // glyphs she is standing on would drop her through the line she is running
        // along, which reads as a bug in the page rather than as a cat at speed.
        if self.raking() {
            self.dash_flown += travel;
            if self.dash_flown >= RAKE_EVERY {
                self.dash_flown -= RAKE_EVERY;
                self.rake = Some([
                    self.pos[0] - dx.signum() * 1.6,
                    self.pos[1] - 0.5,
                    0.0,
                ]);
            }
        } else {
            self.dash_flown = 0.0;
        }

        self.step_gravity(dt, senses);

        self.phase += self.speed * dt * 2.4;
        // A pounce is a hop she talks herself into: the leap above is for ledges,
        // this one is for a pointer on the page.
        if self.mood == Mood::Pounce {
            let t = (0.55 - self.timer.max(0.0)) / 0.55;
            self.hop = (t * std::f32::consts::PI).sin() * 1.5;
        } else {
            self.hop = (self.hop - dt * 6.0).max(0.0);
        }
        self.blend_crouch(dt);
        self.clamp_to_room(left, right, top, floor);
    }

    /// Lets the page hold her up: she stands on the highest ledge under her, and a
    /// step past the end of it is a step into the air.
    fn step_gravity(&mut self, dt: f32, senses: &Senses<'_>) {
        let ground = senses.surface(self.pos[0], self.pos[1]);
        if self.pos[1] > ground + 0.02 {
            self.vy -= GRAVITY * dt;
            self.pos[1] += self.vy * dt;
            if self.pos[1] <= ground {
                self.pos[1] = ground;
                self.vy = 0.0;
                self.crouch = 0.7;
                self.landed = true;
            }
        } else {
            self.pos[1] = ground;
            self.vy = 0.0;
        }
    }

    /// Leaves the ground for `to`, on an arc.
    ///
    /// The arc is the whole of it: a cat that slid up a line of code would be an
    /// elevator with ears. What is animated is `hop`, and `pos` is the ground under
    /// the jump, so the shadow can be dropped to the page while she is in the air.
    fn leap(&mut self, to: [f32; 3]) {
        self.airborne = true;
        // A climb digs in as it leaves: without this the claws of a one-line bound
        // would never come due, because she cannot rise a line and a half.
        self.claw_climbed = CLAW_EVERY;
        self.jump = 0.0;
        self.jump_from = self.pos;
        self.jump_to = to;
        self.speed = 0.0;
        self.vy = 0.0;
    }

    /// One step of a jump in the air.
    ///
    /// The hump is a fixed size and the time grows with the height. A cat scaling a
    /// table does not fly over it by the height of the table: she goes up the side
    /// of it in one bound, and what the reader sees is the bound, not a parabola
    /// scaled to the furniture.
    fn step_leap(&mut self, dt: f32) {
        let was = self.pos[1];
        let rise = (self.jump_to[1] - self.jump_from[1]).max(0.0);
        let across = (self.jump_to[0] - self.jump_from[0])
            .hypot(self.jump_to[1] - self.jump_from[1]);
        // A jump is timed by how high it goes and a swoop by how far: the one thing a
        // cat crossing the page cannot do is arrive in the time a hop takes.
        let time = if self.mood == Mood::Swoop {
            SWOOP_TIME + across * SWOOP_PER_CELL
        } else {
            LEAP_TIME + rise * 0.05
        };
        self.jump = (self.jump + dt / time).min(1.0);
        let t = self.jump;
        self.pos[0] = self.jump_from[0] + (self.jump_to[0] - self.jump_from[0]) * t;
        self.pos[1] = self.jump_from[1] + (self.jump_to[1] - self.jump_from[1]) * t;
        // The hump of a jump is a hump; the hump of a swoop is a flight, and it grows
        // with the length of the leg so that the longest of them is the highest.
        self.hop = if self.mood == Mood::Swoop {
            (t * std::f32::consts::PI).sin() * (1.2 + across * 0.055).min(3.2)
        } else {
            (t * std::f32::consts::PI).sin() * (1.0 + rise * 0.1).min(2.6)
        };
        if (self.jump_to[0] - self.jump_from[0]).abs() > 0.02 {
            self.yaw = if self.jump_to[0] > self.jump_from[0] {
                std::f32::consts::FRAC_PI_2
            } else {
                -std::f32::consts::FRAC_PI_2
            };
        }
        // The claws, while she is between two footholds. They are in the line her
        // back paws have just left and never in the one she is going to: a cat that
        // took the row it was landing on would have nothing to land on, which is the
        // one thing an ascent cannot afford.
        if self.mood == Mood::Climb {
            self.claw_climbed += (self.pos[1] - was).max(0.0);
            if self.claw_climbed >= CLAW_EVERY && self.pos[1] < self.jump_to[1] - 0.75 {
                self.claw_climbed -= CLAW_EVERY;
                // Half a line under her feet, because the characters of a row sit in
                // the middle of the cell the line's top edge bounds.
                self.claw = Some([self.pos[0], self.pos[1] - 0.5, 0.0]);
            }
        } else {
            self.claw_climbed = 0.0;
        }
        if t >= 1.0 {
            self.airborne = false;
            self.hop = 0.0;
            self.arrived = true;
            self.crouch = 0.7;
            self.landed = true;
            // A landing on a thing is still a paw landing on it: the swing comes
            // after touchdown, when the cat is where it meant to be.
            if matches!(self.mood, Mood::Bat | Mood::Swat | Mood::Chase) {
                self.arm();
            }
        }
    }

    /// How deep a crouch this mood asks for, eased into.
    ///
    /// Landing folds her up for a frame as well, which is the only thing a squash
    /// needs to read as weight.
    fn blend_crouch(&mut self, dt: f32) {
        let crouch = match self.mood {
            Mood::Bat | Mood::Swat => 0.5,
            _ => 0.0,
        };
        self.crouch += (crouch - self.crouch) * (dt * 8.0).min(1.0);
        // A cat sits down when it has nothing to do, and gets up before it moves.
        // Sitting includes sitting on something: a cat on a table folds itself up
        // the same way as a cat on the page.
        let sit = match self.mood {
            Mood::Sit | Mood::Groom | Mood::Clean | Mood::Perch => 1.0,
            Mood::Sleep => 0.0,
            _ => 0.0,
        };
        self.sit += (sit - self.sit) * (dt * 3.4).min(1.0);
    }

    /// Keeps her on the paper and on the ledges it has.
    fn clamp_to_room(&mut self, left: f32, right: f32, top: f32, floor: f32) {
        self.pos[0] = self.pos[0].clamp(left, right);
        self.pos[1] = self.pos[1].clamp(floor, top);
        if self.airborne {
            self.jump_to[0] = self.jump_to[0].clamp(left, right);
            self.jump_to[1] = self.jump_to[1].clamp(floor, top);
        }
    }

    /// The top of her head, where a Zzz starts.
    pub fn crown(&self) -> [f32; 3] {
        [
            self.pos[0] + 0.18,
            self.pos[1] + self.height() - 0.2 + self.hop,
            self.pos[2] + STAND + 0.15,
        ]
    }

    /// The paw she washes with, which is where a sparkle starts.
    pub fn muzzle(&self) -> [f32; 3] {
        [
            self.pos[0],
            self.pos[1] + 0.94 + self.hop,
            self.pos[2] + STAND + 0.4,
        ]
    }

    fn distance_to(&self, at: [f32; 3]) -> f32 {
        (at[0] - self.pos[0]).hypot(at[1] - self.pos[1])
    }

    /// The boxes that draw it, in world space.
    ///
    /// Standing and sitting are two layouts of the same parts, blended by `sit`,
    /// so a cat that decides to sit down lowers itself over a few frames instead
    /// of being replaced by a different cat. Asleep is a layout of its own: a
    /// curled cat shares no useful coordinates with a standing one.
    ///
    /// The body turns with `yaw`; the head turns with `head_yaw` about its own
    /// anchor, which is what puts a face on a profile.
    pub fn blocks(&self) -> Vec<Block> {
        let asleep = self.mood == Mood::Sleep;
        let (body, anchor) = if asleep {
            (self.asleep_body(), self.head_anchor(true))
        } else {
            let sit = self.sit.clamp(0.0, 1.0);
            let standing = self.awake_body(false);
            let sitting = self.awake_body(true);
            let body = standing
                .iter()
                .zip(sitting.iter())
                .map(|(up, down)| up.blend(down, sit))
                .collect();
            let stand = self.head_anchor_at(false);
            let seat = self.head_anchor_at(true);
            let anchor = [
                stand[0] + (seat[0] - stand[0]) * sit,
                stand[1] + (seat[1] - stand[1]) * sit,
                stand[2] + (seat[2] - stand[2]) * sit,
            ];
            (body, anchor)
        };

        let mut out = Vec::with_capacity(body.len() + 16);
        for part in &body {
            out.push(self.place(part, self.yaw, part.at));
        }
        // The head anchor is in the cat's own space; its parts are in the head's.
        let (sin, cos) = self.yaw.sin_cos();
        let centre = [
            self.pos[0] + anchor[0] * cos + anchor[2] * sin,
            self.pos[1] + anchor[1],
            self.pos[2] - anchor[0] * sin + anchor[2] * cos + STAND,
        ];
        for part in self.head_parts(asleep) {
            let (hsin, hcos) = self.head_yaw.sin_cos();
            let at = [
                part.at[0] * hcos + part.at[2] * hsin,
                part.at[1],
                -part.at[0] * hsin + part.at[2] * hcos,
            ];
            out.push(Block {
                pos: [centre[0] + at[0], centre[1] + at[1], centre[2] + at[2]],
                size: part.size,
                yaw: self.head_yaw,
                shade: part.shade,
            });
        }
        // A paw up at the face. A cat washing itself is a cat with one front paw
        // over its muzzle: at this size that is a box in front of the nose that
        // moves while the head rocks behind it, and it is the difference between
        // washing and staring.
        if !asleep && self.sit > 0.35 && matches!(self.mood, Mood::Clean | Mood::Groom) {
            let lick = (self.wash * 7.0).sin();
            let at = self.muzzle();
            out.push(Block {
                pos: [at[0] + lick * 0.05, at[1] - 0.28, at[2]],
                size: [0.19, 0.46, 0.19],
                yaw: self.head_yaw,
                shade: Shade::Fur,
            });
            out.push(Block {
                pos: [at[0] + lick * 0.07, at[1] - 0.02 + lick * 0.04, at[2] + 0.01],
                size: [0.22, 0.15, 0.22],
                yaw: self.head_yaw,
                shade: Shade::FurLight,
            });
        }
        out
    }

    /// One part, turned for the page and put where the cat is.
    fn place(&self, part: &Part, yaw: f32, at: [f32; 3]) -> Block {
        let (sin, cos) = yaw.sin_cos();
        Block {
            pos: [
                self.pos[0] + at[0] * cos + at[2] * sin,
                self.pos[1] + at[1],
                self.pos[2] - at[0] * sin + at[2] * cos + STAND,
            ],
            size: part.size,
            yaw,
            shade: part.shade,
        }
    }

    /// Where the head sits, standing or sitting, before this frame's motion.
    fn head_anchor_at(&self, sitting: bool) -> [f32; 3] {
        let motion = self.motion();
        if sitting {
            [0.0, 1.08, 0.10]
        } else {
            [
                0.0,
                HEAD[1] + motion.lift - motion.settle * 0.7,
                HEAD[2],
            ]
        }
    }

    /// Where the head sits this frame.
    fn head_anchor(&self, asleep: bool) -> [f32; 3] {
        if asleep {
            [0.0, 0.52, 0.50]
        } else {
            let sit = self.sit.clamp(0.0, 1.0);
            let stand = self.head_anchor_at(false);
            let seat = self.head_anchor_at(true);
            [
                stand[0] + (seat[0] - stand[0]) * sit,
                stand[1] + (seat[1] - stand[1]) * sit,
                stand[2] + (seat[2] - stand[2]) * sit,
            ]
        }
    }

    /// The face, in its own space: across `x`, up `y`, out of the nose `z`.
    ///
    /// A face is symmetric, so it is built once and not per pose; the only
    /// difference between awake and asleep is that the eyes shut and the ears
    /// fold.
    fn head_parts(&self, asleep: bool) -> Vec<Part> {
        let eye = EYE[1] * (1.0 - self.blink).max(0.12);
        let mut out = Vec::with_capacity(16);
        if asleep {
            out.push(Part::new([0.0, 0.0, 0.0], [0.62, 0.54, 0.50], Shade::Fur));
            out.push(Part::new([0.0, 0.06, 0.06], [0.52, 0.32, 0.40], Shade::Fur));
        } else {
            out.push(Part::new([0.0, 0.02, 0.0], HEAD_SIZE, Shade::Fur));
            // The crown: a narrower box set back on top of the skull, because a
            // square forehead on a cat reads as a bear. Set back, so that it does
            // not stand in front of the ears and eat them.
            out.push(Part::new(
                [0.0, 0.30, -0.10],
                [0.68, 0.24, 0.44],
                Shade::Fur,
            ));
        }
        // The chin: narrower than the skull and tucked under it, so the jaw tapers
        // instead of ending on a straight line. Narrower, because a box that is
        // wider than the one in front of it shows its own top face across the
        // face, and a bright band across a face is a muzzle that nobody drew.
        out.push(Part::new(
            [0.0, -0.40, 0.00],
            [0.66, 0.16, 0.50],
            Shade::Fur,
        ));
        out.push(Part::new(
            [0.0, -0.16, 0.30],
            [0.36, 0.22, 0.12],
            Shade::FurLight,
        ));
        out.push(Part::new(
            [0.0, -0.10, 0.37],
            [0.15, 0.07, 0.05],
            Shade::Nose,
        ));
        out.push(Part::new(
            [0.0, -0.24, 0.35],
            [0.08, 0.03, 0.05],
            Shade::FurDark,
        ));

        for side in [-1.0f32, 1.0] {
            if asleep {
                // A shut eye is a line, and the glint is hidden inside it so the
                // two faces keep the same parts.
                out.push(Part::new(
                    [side * 0.18, -0.02, 0.24],
                    [0.15, 0.04, 0.08],
                    Shade::Eye,
                ));
                out.push(Part::new(
                    [side * 0.18, -0.02, 0.25],
                    [0.01, 0.01, 0.01],
                    Shade::FurLight,
                ));
                out.push(Part::new(
                    [side * 0.22, 0.28, -0.06],
                    [0.24, 0.14, 0.18],
                    Shade::FurDark,
                ));
                out.push(Part::new(
                    [side * 0.24, 0.36, -0.08],
                    [0.16, 0.12, 0.12],
                    Shade::FurDark,
                ));
                out.push(Part::new(
                    [side * 0.22, 0.30, 0.03],
                    [0.11, 0.07, 0.03],
                    Shade::Nose,
                ));
            } else {
                // Eyes: a fifth of the head wide and a third tall, set either side
                // of the muzzle, each with a white glint in its top corner.
                out.push(Part::new(
                    [side * 0.21, 0.00, 0.26],
                    [EYE[0], eye, EYE[2]],
                    Shade::Eye,
                ));
                out.push(Part::new(
                    [side * 0.24, 0.09, 0.30],
                    [0.06, (eye * 0.30).max(0.03), 0.04],
                    Shade::FurLight,
                ));
                // Ears: a wide base and a narrower tip, both tall enough to stand
                // above the crown and in front of it, dark against the fur, with
                // the pink of the inside on the face of the base.
                out.push(Part::new(
                    [side * 0.28, 0.44, 0.08],
                    EAR_SIZE,
                    Shade::FurDark,
                ));
                out.push(Part::new(
                    [side * 0.30, 0.62, 0.06],
                    [0.20, 0.24, 0.16],
                    Shade::FurDark,
                ));
                out.push(Part::new(
                    [side * 0.28, 0.52, 0.20],
                    [0.13, 0.12, 0.05],
                    Shade::Nose,
                ));
            }
        }
        out
    }

    /// What the walk and the breath do to the pose.
    fn motion(&self) -> Motion {
        let breath = (self.breath * 1.9).sin() * 0.02;
        let bob = if self.speed > 0.2 {
            (self.phase * 2.0).sin() * 0.05
        } else {
            0.0
        };
        Motion {
            lift: self.hop + bob,
            settle: self.crouch * 0.35,
            stretch: 1.0 + self.hop * 0.06,
            breath,
        }
    }

    /// The body, standing or sitting, part for part in the same order.
    ///
    /// The silhouette is decided before the cat: a short barrel, a haunch behind
    /// it, a cream bib at the chest, four legs and their socks on the page, and a
    /// tail on an arc outside the rump instead of growing through it. The two
    /// poses are laid out box for box so that one can be blended into the other.
    fn awake_body(&self, sitting: bool) -> Vec<Part> {
        let motion = self.motion();
        // A sitting cat does not bob with a walk it is not doing, and it is not
        // stretched by a pounce it is not making.
        let moving = if sitting { 0.0 } else { 1.0 };
        let lift = motion.lift * moving;
        let settle = motion.settle * moving;
        let stretch = 1.0 + (motion.stretch - 1.0) * moving;
        let breath = motion.breath;
        let crouch = self.crouch * moving;
        let mut out = Vec::with_capacity(20);

        if sitting {
            // Sitting down drops the rear onto the haunch and stands the chest up
            // over it, which is the whole difference between the two poses.
            out.push(Part::new([0.0, 0.33, -0.06], [0.72, 0.56, 0.62], Shade::Fur));
            out.push(Part::new([0.0, 0.28, -0.32], [0.76, 0.44, 0.26], Shade::Fur));
            out.push(Part::new([0.0, 0.52, -0.04], [0.58, 0.10, 0.62], Shade::FurDark));
            out.push(Part::new([0.0, 0.56, 0.30], [0.30, 0.30, 0.12], Shade::FurLight));
            out.push(Part::new([0.0, 0.50, 0.06], [0.60, 0.46, 0.46], Shade::Fur));
        } else {
            out.push(Part::new(
                [0.0, 0.66 + lift - settle, -0.02],
                [
                    BODY[0],
                    (BODY[1] + breath) * (1.0 - 0.16 * crouch),
                    BODY[2] * stretch,
                ],
                Shade::Fur,
            ));
            out.push(Part::new(
                [0.0, 0.68 + lift - settle, -0.32],
                [0.72, 0.54, 0.36],
                Shade::Fur,
            ));
            out.push(Part::new(
                [0.0, 0.88 + lift - settle, -0.02],
                [0.54, 0.10, 0.72],
                Shade::FurDark,
            ));
            out.push(Part::new(
                [0.0, 0.58 + lift - settle, 0.34],
                [0.60, 0.30, 0.12],
                Shade::FurLight,
            ));
            out.push(Part::new(
                [0.0, 0.84 + lift - settle, 0.14],
                [0.46, 0.22, 0.34],
                Shade::Fur,
            ));
        }

        // Two front legs and two back ones, each with a cream sock: four thin
        // columns are what says "quadruped" at this size, and the socks are what
        // say where the page is.
        for side in [-1.0f32, 1.0] {
            if sitting {
                out.push(Part::new(
                    [side * 0.17, 0.36, 0.30],
                    [0.17, 0.60, 0.17],
                    Shade::Fur,
                ));
                out.push(Part::new(
                    [side * 0.30, 0.26, -0.24],
                    [0.20, 0.36, 0.34],
                    Shade::Fur,
                ));
                out.push(Part::new(
                    [side * 0.17, 0.06, 0.42],
                    [0.21, 0.13, 0.24],
                    Shade::FurLight,
                ));
                out.push(Part::new(
                    [side * 0.30, 0.06, 0.04],
                    [0.21, 0.13, 0.24],
                    Shade::FurLight,
                ));
            } else {
                let step = lift * 0.4;
                out.push(Part::new(
                    [side * 0.19, 0.24 + step, 0.22],
                    [0.18, 0.40, 0.19],
                    Shade::Fur,
                ));
                out.push(Part::new(
                    [side * 0.19, 0.24 + step, -0.34],
                    [0.20, 0.40, 0.21],
                    Shade::Fur,
                ));
                out.push(Part::new(
                    [side * 0.20, 0.06, 0.28],
                    [0.22, 0.13, 0.25],
                    Shade::FurLight,
                ));
                out.push(Part::new(
                    [side * 0.20, 0.06, -0.32],
                    [0.22, 0.13, 0.25],
                    Shade::FurLight,
                ));
            }
        }

        // Tail: six overlapping boxes along a bow that starts inside the rump and
        // ends in the air beside it. A circle centred on the body buries half the
        // tail in it; the bow keeps all of it outside the silhouette it grows
        // from, and each box is longer than the gap to the next so the six read as
        // one tail.
        let wave = (self.tail * 0.9).sin() * 0.04;
        let (root, bow, tip) = if sitting {
            ([-0.26, 0.30], [-0.70, 0.52], [-0.50, 0.90])
        } else {
            ([-0.30, 0.62], [-0.74, 0.95], [-0.60, 1.35])
        };
        for segment in 0..6 {
            let t = segment as f32 / 5.0;
            let back = (1.0 - t) * (1.0 - t);
            let across = 2.0 * (1.0 - t) * t;
            let forward = t * t;
            let thick = 0.19 - t * 0.05;
            out.push(Part::new(
                [
                    t * 0.10 + wave,
                    root[1] * back + bow[1] * across + tip[1] * forward + lift * t,
                    root[0] * back + bow[0] * across + tip[0] * forward,
                ],
                [thick, thick - 0.01, 0.30 - t * 0.05],
                if segment >= 5 {
                    Shade::FurLight
                } else {
                    Shade::Fur
                },
            ));
        }
        out
    }

    /// The curled body: a loaf with a head tucked against the front of it.
    fn asleep_body(&self) -> Vec<Part> {
        let wave = (self.tail * 0.6).sin() * 0.02;
        let mut out = Vec::with_capacity(16);
        out.push(Part::new([0.0, 0.36, -0.04], [0.84, 0.58, 1.06], Shade::Fur));
        out.push(Part::new([0.0, 0.36, -0.42], [0.88, 0.60, 0.44], Shade::Fur));
        out.push(Part::new([0.0, 0.60, -0.04], [0.62, 0.10, 0.82], Shade::FurDark));
        out.push(Part::new([0.0, 0.26, 0.44], [0.48, 0.28, 0.20], Shade::FurLight));

        for side in [-1.0f32, 1.0] {
            out.push(Part::new(
                [side * 0.20, 0.16, 0.30],
                [0.20, 0.18, 0.26],
                Shade::FurLight,
            ));
            out.push(Part::new(
                [side * 0.28, 0.16, -0.28],
                [0.24, 0.18, 0.30],
                Shade::FurLight,
            ));
            out.push(Part::new(
                [side * 0.20, 0.06, 0.40],
                [0.20, 0.10, 0.20],
                Shade::FurLight,
            ));
            out.push(Part::new(
                [side * 0.28, 0.06, -0.20],
                [0.20, 0.10, 0.20],
                Shade::FurLight,
            ));
        }

        // The tail lies along her flank and curls up by her nose.
        for segment in 0..5 {
            let t = segment as f32 / 4.0;
            let thick = 0.17 - t * 0.04;
            out.push(Part::new(
                [0.44 + wave * t, 0.24 + t * 0.10, -0.30 + t * 0.78],
                [thick, thick - 0.01, 0.32 - t * 0.03],
                if segment >= 3 {
                    Shade::FurLight
                } else {
                    Shade::Fur
                },
            ));
        }
        out
    }
}

/// One box of the animal in its own space, before it is turned for the page.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Part {
    at: [f32; 3],
    size: [f32; 3],
    shade: Shade,
}

impl Part {
    fn new(at: [f32; 3], size: [f32; 3], shade: Shade) -> Self {
        Self { at, size, shade }
    }

    /// A part partway between two poses, keeping the shade of the first.
    fn blend(&self, other: &Part, t: f32) -> Part {
        let mix = |a: f32, b: f32| a + (b - a) * t;
        Part {
            at: [
                mix(self.at[0], other.at[0]),
                mix(self.at[1], other.at[1]),
                mix(self.at[2], other.at[2]),
            ],
            size: [
                mix(self.size[0], other.size[0]),
                mix(self.size[1], other.size[1]),
                mix(self.size[2], other.size[2]),
            ],
            shade: self.shade,
        }
    }
}

/// What the walk and the breath do to a pose, in cells.
struct Motion {
    lift: f32,
    settle: f32,
    stretch: f32,
    breath: f32,
}

/// How far in front of the page the cat stands, in cells.
///
/// Half its own width plus a margin: the far side of a box that straddled the page
/// would be cut by the letters it stands on, which reads as a hole in the cat.
const STAND: f32 = 0.62;

/// The nearest piece of furniture to `at`, other than what she is standing on.
fn nearest_prop(props: &[Prop], at: [f32; 3]) -> Option<&Prop> {
    props
        .iter()
        .filter(|prop| !standing_on(prop, at))
        .min_by(|a, b| a.distance_to(at).total_cmp(&b.distance_to(at)))
}

/// The nearest end of what she is standing on, which is the way down.
///
/// A line of code is a ledge and a table top is a ledge, so both are asked: a cat
/// that can only get off a line and not off a table is a cat stuck on a table.
fn ledge_end(senses: &Senses<'_>, at: [f32; 3]) -> Option<f32> {
    let shelf = senses
        .shelves
        .iter()
        .filter(|shelf| shelf.covers(at[0]) && (shelf.y - at[1]).abs() < 0.1)
        .min_by(|a, b| a.span[0].total_cmp(&b.span[0]))
        .map(|shelf| shelf.span);
    let table = senses
        .props
        .iter()
        .filter(|prop| prop.kind.is_perch() && standing_on(prop, at))
        .map(|prop| {
            let half = prop.size[0] * GLYPH_WIDTH * 0.5;
            [prop.pos[0] - half, prop.pos[0] + half]
        })
        .fold(None, |best: Option<[f32; 2]>, span| match best {
            Some(best) => Some([best[0].min(span[0]), best[1].max(span[1])]),
            None => Some(span),
        });
    let span = shelf.or(table)?;
    let left = span[0] - 0.6;
    let right = span[1] + 0.6;
    Some(if (left - at[0]).abs() < (right - at[0]).abs() {
        left
    } else {
        right
    })
}

/// Whether she is standing on that piece of furniture.
fn standing_on(prop: &Prop, at: [f32; 3]) -> bool {
    let half = prop.size[0] * GLYPH_WIDTH * 0.5;
    (at[1] - prop.top()).abs() < 0.35 && (at[0] - prop.pos[0]).abs() <= half
}

/// The ledge above her that is nearest to being under her.
///
/// Usually the next line up: a cat that climbed from the rug to the top of the
/// page in one jump would be a cat on a lift.
fn nearest_shelf_above(shelves: &[Shelf], at: [f32; 3]) -> Option<&Shelf> {
    let near = |shelf: &Shelf| (shelf.nearest_point(at[0]) - at[0]).abs() + (shelf.y - at[1]) * 0.5;
    shelves
        .iter()
        .filter(|shelf| shelf.y > at[1] + 0.5 && shelf.y < at[1] + CLIMB)
        .filter(|shelf| shelf.span[0] < at[0] + 8.0 && shelf.span[1] > at[0] - 8.0)
        .min_by(|a, b| near(a).total_cmp(&near(b)))
}

/// How high a bound can take her, in cells.
///
/// A cat clears a chair, not a bookshelf: the height is what makes the arc read
/// as a jump rather than as a cat being lifted up the page.
const CLIMB: f32 = 6.0;

/// The highest thing over her head that one bound can reach, as the stretch of it she
/// could stand on and how high it is: left edge, right edge, top.
///
/// A line of code and the top of a table are the same thing to a climbing cat, so both
/// are asked. What is not a foothold is anything above the highest place a cat may
/// stand -- the top two rows of the page are where its ears go, not its paws -- or
/// anything further to the side than [`REACH`], which is a walk and then a jump and not
/// a climb.
fn foothold_above(senses: &Senses<'_>, at: [f32; 3]) -> Option<[f32; 3]> {
    let (left, right, _bottom, top) = room_to_walk(senses.page[0], senses.page[1]);
    let mut places: Vec<[f32; 3]> = Vec::with_capacity(senses.shelves.len() + senses.props.len());
    for shelf in senses.shelves {
        places.push([shelf.span[0], shelf.span[1], shelf.y]);
    }
    for prop in senses.props.iter().filter(|prop| prop.kind.is_perch()) {
        let half = prop.size[0] * GLYPH_WIDTH * 0.5;
        places.push([prop.pos[0] - half, prop.pos[0] + half, prop.top()]);
    }
    let climbable: Vec<[f32; 3]> = places
        .into_iter()
        .map(|place| [place[0].max(left), place[1].min(right), place[2]])
        .filter(|place| place[1] - place[0] >= 1.2)
        .filter(|place| {
            // Above the height a leap is even attempted at, and no higher than one can
            // go: a foothold in between the two would leave her standing under it
            // waiting for a jump that never comes.
            place[2] - at[1] >= 0.75 && place[2] - at[1] <= CLIMB && place[2] <= top + 0.05
        })
        .collect();
    highest_foothold(
        climbable
            .into_iter()
            .filter(|place| (at[0].clamp(place[0], place[1]) - at[0]).abs() <= REACH),
        at,
    )
}

/// The highest of a set of footholds, and the nearest across the page when two of them
/// are at the same height.
fn highest_foothold(places: impl Iterator<Item = [f32; 3]>, at: [f32; 3]) -> Option<[f32; 3]> {
    places.max_by(|a, b| {
        a[2].total_cmp(&b[2]).then(
            (at[0].clamp(b[0], b[1]) - at[0])
                .abs()
                .total_cmp(&(at[0].clamp(a[0], a[1]) - at[0]).abs()),
        )
    })
}

/// The nearest piece of furniture that can be climbed onto, no higher than
/// `highest` and no lower than her own paws.
fn nearest_perch(props: &[Prop], at: [f32; 3], highest: f32) -> Option<&Prop> {
    props
        .iter()
        .filter(|prop| prop.kind.is_perch())
        .filter(|prop| prop.top() - at[1] > 0.6 && prop.top() - at[1] <= highest)
        .min_by(|a, b| a.distance_to(at).total_cmp(&b.distance_to(at)))
}

/// The nearest loose word that has stopped moving.
///
/// A word still in the air is not something to walk up to: by the time the cat
/// got there it would have landed somewhere else.
fn nearest_token(tokens: &[Token], at: [f32; 3]) -> Option<&Token> {
    tokens
        .iter()
        .filter(|token| token.rest)
        .min_by(|a, b| apart(&a.pos, at).total_cmp(&apart(&b.pos, at)))
}

/// The nearest speck of dust.
fn nearest_mote(motes: &[Mote], at: [f32; 3]) -> Option<&Mote> {
    motes
        .iter()
        .min_by(|a, b| apart(&a.pos, at).total_cmp(&apart(&b.pos, at)))
}

/// How far two points on the page are apart, in cells.
fn apart(a: &[f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// The rectangle the kitten may walk in, in cells: inset far enough from the edges
/// of the page that no part of it is ever cut off.
fn room_to_walk(width: f32, rows: f32) -> (f32, f32, f32, f32) {
    // Right of the gutter: the line numbers are the page's furniture, not a place
    // for a cat to stand.
    let left = 3.2;
    let right = (width - 1.6).max(left);
    let bottom = 1.6;
    let top = (rows - 2.4).max(bottom);
    (left, right, bottom, top)
}

/// Where the head sits when the kitten is standing, in its own space.
const HEAD: [f32; 3] = [0.0, 1.30, 0.10];
/// The head's size: as wide as the body is long, which is the whole trick of a
/// kitten. A head smaller than that is a cat, and a cat is not what we drew.
const HEAD_SIZE: [f32; 3] = [0.86, 0.72, 0.58];
/// An ear's size at its base. The tip is narrower and sits above it: wide at the
/// bottom and narrow at the top is what makes the pair read as ears, not a hat.
const EAR_SIZE: [f32; 3] = [0.34, 0.24, 0.20];
/// The body's size, standing: short, because a kitten is mostly head.
const BODY: [f32; 3] = [0.66, 0.50, 0.76];
/// An eye's size when open: a fifth of the head wide, which is the difference
/// between a face and a dot.
const EYE: [f32; 3] = [0.18, 0.23, 0.08];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playground::corpus::Flavour;
    use crate::playground::scene::Page;

    /// A page with code, furniture, loose words and dust on it.
    fn page() -> Page {
        Page::new(Flavour::Rust, 0x5EED, 0, 120, 40)
    }

    fn senses<'a>(rng: &'a mut Rng, pointer: Option<[f32; 3]>, age: f32) -> Senses<'a> {
        Senses {
            pointer,
            pointer_age: 0.2,
            input_age: age,
            page: [80.0, 40.0],
            props: &[],
            tokens: &[],
            motes: &[],
            shelves: &[],
            floor: 1.6,
            rng,
        }
    }

    /// The same senses, on a page that has things on it.
    fn senses_on<'a>(
        rng: &'a mut Rng,
        page: &'a Page,
        pointer: Option<[f32; 3]>,
        age: f32,
    ) -> Senses<'a> {
        Senses {
            pointer,
            pointer_age: 0.2,
            input_age: age,
            page: [page.width(), page.rows as f32],
            props: &page.props,
            shelves: &page.shelves,
            floor: page.floor(),
            tokens: &page.tokens,
            motes: &page.motes,
            rng,
        }
    }

    #[test]
    fn the_kitten_is_two_to_three_lines_high() {
        let kitten = Kitten::default();
        let height = kitten.height();
        assert!((2.0..3.0).contains(&height), "{height}");
    }

    #[test]
    fn it_is_made_of_boxes_and_the_ears_are_the_top() {
        let kitten = Kitten::default();
        let blocks = kitten.blocks();
        assert!(blocks.len() >= 30, "{}", blocks.len());
        let top = blocks
            .iter()
            .copied()
            .max_by(|a, b| (a.pos[1] + a.size[1] * 0.5).total_cmp(&(b.pos[1] + b.size[1] * 0.5)))
            .expect("a cat is not empty");
        // The ears are the top of the cat: dark against the fur, and narrower
        // than the skull, so that the pair reads as ears and not as a hat.
        assert_eq!(top.shade, Shade::FurDark);
        assert!(top.size[0] < HEAD_SIZE[0], "{:?}", top.size);
        assert!(top.pos[1] > kitten.pos[1] + HEAD[1], "{:?}", top.pos);
    }

    #[test]
    fn the_face_is_what_makes_it_a_kitten() {
        // A head as wide as the body is long, an eye a fifth of the face, and the
        // two of them with a nose between: below these numbers it is a cat, and
        // at the size a cat is drawn here a cat is a dog.
        let head = HEAD_SIZE[0];
        let body = BODY[2];
        assert!(head >= body * 0.95, "{head} wide on {body} long is a cat");
        let fifth = EYE[0] / HEAD_SIZE[0];
        assert!((0.18..0.24).contains(&fifth), "{fifth}");
        let blocks = Kitten::default().blocks();
        let eyes = blocks.iter().filter(|b| b.shade == Shade::Eye).count();
        let noses = blocks.iter().filter(|b| b.shade == Shade::Nose).count();
        assert_eq!(eyes, 2);
        // One nose on the muzzle and a pink inside to each ear.
        assert_eq!(noses, 3);
    }

    #[test]
    fn the_head_looks_at_the_reader_and_the_body_does_not() {
        let mut rng = Rng::new(11);
        let mut kitten = Kitten::new(10.0, 20.0);
        for _ in 0..(120 * 2) {
            kitten.think(
                1.0 / 120.0,
                &mut senses(&mut rng, Some([30.0, 20.0, 0.0]), 0.0),
            );
        }
        // The body walks along the page; the face stays turned to the reader,
        // within a few degrees either way.
        assert!((kitten.yaw.abs() - std::f32::consts::FRAC_PI_2).abs() < 0.01);
        assert!(kitten.head_yaw.abs() < 0.5, "{}", kitten.head_yaw);
        let eyes: Vec<_> = kitten
            .blocks()
            .into_iter()
            .filter(|b| b.shade == Shade::Eye)
            .collect();
        assert_eq!(eyes.len(), 2);
        // Two eyes at the same height, one either side of the middle.
        assert!((eyes[0].pos[1] - eyes[1].pos[1]).abs() < 0.01);
        assert!(eyes[0].pos[0] < eyes[1].pos[0]);
    }

    #[test]
    fn it_walks_to_where_it_is_going() {
        let mut rng = Rng::new(1);
        let mut kitten = Kitten::new(10.0, 20.0);
        kitten.mood = Mood::Wander;
        kitten.target = [4.0, 0.0, 0.0];
        kitten.timer = 30.0;
        for _ in 0..(120 * 4) {
            kitten.think(1.0 / 120.0, &mut senses(&mut rng, None, 0.0));
        }
        assert!(kitten.pos[0] > 3.0, "{:?}", kitten.pos);
    }

    #[test]
    fn it_never_leaves_the_page() {
        let mut rng = Rng::new(2);
        let mut kitten = Kitten::new(10.0, 20.0);
        for step in 0..(120 * 40) {
            let pointer = Some([10.0 * (step as f32 * 0.01).sin(), 20.0, 0.0]);
            kitten.think(1.0 / 120.0, &mut senses(&mut rng, pointer, 0.0));
            assert!(kitten.pos[0] >= 0.99 && kitten.pos[0] <= 79.01, "{:?}", kitten.pos);
            assert!(kitten.pos[1] >= 1.49 && kitten.pos[1] <= 38.01, "{:?}", kitten.pos);
            assert_eq!(kitten.pos[2], 0.0, "the cat never leaves the page");
        }
    }

    #[test]
    fn it_follows_the_pointer_when_it_moves() {
        let mut rng = Rng::new(3);
        let mut kitten = Kitten::new(10.0, 20.0);
        // Up to the moment she gets there: what she does next is another mood, and
        // a test that read the mood at an arbitrary second would be reading the
        // generator, not the following.
        let mut closest = f32::MAX;
        for _ in 0..(120 * 3) {
            kitten.think(
                1.0 / 120.0,
                &mut senses(&mut rng, Some([12.5, 20.0, 0.0]), 0.0),
            );
            closest = closest.min((kitten.pos[0] - 12.5).abs());
        }
        assert!(closest < 1.0, "she never came within a cell of it: {closest}");
    }

    #[test]
    fn it_sleeps_when_nothing_happens_and_wakes_when_something_does() {
        let mut rng = Rng::new(4);
        let mut kitten = Kitten::new(10.0, 20.0);
        for _ in 0..120 {
            kitten.think(1.0 / 120.0, &mut senses(&mut rng, None, SLEEP_AFTER + 1.0));
        }
        assert_eq!(kitten.mood, Mood::Sleep);
        assert_eq!(kitten.speed, 0.0);
        let asleep = kitten.blocks();
        let top = asleep
            .iter()
            .map(|block| block.pos[1] + block.size[1] * 0.5)
            .fold(0.0f32, f32::max);
        assert!(
            top - kitten.pos[1] < 2.0,
            "a curled cat is lower: {top}"
        );

        kitten.think(1.0 / 120.0, &mut senses(&mut rng, None, 0.0));
        assert_ne!(kitten.mood, Mood::Sleep);
    }

    #[test]
    fn it_pounces_when_the_pointer_appears_far_away() {
        let mut rng = Rng::new(5);
        let mut kitten = Kitten::new(10.0, 20.0);
        let mut found = false;
        for _ in 0..240 {
            kitten.think(
                1.0 / 120.0,
                &mut senses(&mut rng, Some([6.0, 0.0, -6.0]), 0.0),
            );
            found |= kitten.mood == Mood::Pounce;
        }
        assert!(found, "never pounced: {:?}", kitten.mood);
    }

    #[test]
    fn it_strikes_while_batting() {
        let mut rng = Rng::new(6);
        let mut kitten = Kitten::new(10.0, 20.0);
        kitten.mood = Mood::Bat;
        kitten.timer = 0.4;
        let mut strikes = 0;
        for _ in 0..30 {
            if kitten.think(1.0 / 120.0, &mut senses(&mut rng, None, 0.0)) {
                strikes += 1;
            }
        }
        assert!(strikes > 0);
    }

    #[test]
    fn the_paw_is_in_front_of_the_cat() {
        let mut kitten = Kitten {
            pos: [4.0, 8.0, 0.0],
            ..Kitten::default()
        };
        // Facing right: the paw is to the right of the cat and on the page.
        let paw = kitten.paw();
        assert!(paw[0] > 4.5, "{paw:?}");
        assert_eq!(paw[2], 0.0, "{paw:?}");
        kitten.yaw = -std::f32::consts::FRAC_PI_2;
        let paw = kitten.paw();
        assert!(paw[0] < 3.5, "{paw:?}");
    }

    #[test]
    fn the_same_seed_drives_the_same_kitten() {
        let run = || {
            let mut rng = Rng::new(9);
            let mut kitten = Kitten::new(10.0, 20.0);
            for _ in 0..(120 * 6) {
                kitten.think(1.0 / 120.0, &mut senses(&mut rng, None, 0.0));
            }
            kitten
        };
        let a = run();
        let b = run();
        assert_eq!(a.pos, b.pos);
        assert_eq!(a.mood, b.mood);
    }

    /// What a cat did with a minute of the page: which moods it was in, and
    /// whether the page answered.
    #[derive(Default)]
    struct Show {
        moods: std::collections::BTreeMap<&'static str, usize>,
        shook: bool,
        words_moved: bool,
        dust_thrown: bool,
        jumped: bool,
        knocked: bool,
        clawed: bool,
        washed: bool,
    }

    /// Runs the cat on a page for `seconds`, doing what the app does to it.
    fn live_for(page: &mut Page, kitten: &mut Kitten, rng: &mut Rng, seconds: f32) -> Show {
        let mut show = Show::default();
        let dt = 1.0 / 60.0;
        let mut characters = page.text.len();
        for _ in 0..(seconds / dt) as usize {
            let strike = {
                let mut senses = senses_on(rng, page, None, 0.0);
                kitten.think(dt, &mut senses)
            };
            // A swing takes what is under it, a landing only shakes the page.
            if strike {
                page.strike(kitten.paw(), rng);
            } else if kitten.landed {
                page.land(kitten.paw(), rng);
            }
            // And a climb takes the line it is leaving, which is the page's business
            // rather than the animal's.
            if let Some(at) = kitten.claw {
                show.clawed |= page.claw(at, rng) > 0;
            }
            show.washed |= kitten.effect == Some(Effect::Lick);
            match kitten.effect {
                Some(Effect::Zzz) => page.float('z', 7, kitten.crown(), rng),
                Some(Effect::Lick) => page.float('*', 5, kitten.muzzle(), rng),
                None => {}
            }
            page.step_tokens(dt);
            page.step_motes(dt);
            page.step_props(dt);
            page.step_floating(dt);
            *show.moods.entry(kitten.mood.name()).or_insert(0) += 1;
            show.jumped |= kitten.airborne;
            show.shook |= page.props.iter().any(|prop| prop.wobble > 0.0);
            show.words_moved |= page.tokens.iter().any(|token| !token.rest);
            show.dust_thrown |= page
                .motes
                .iter()
                .any(|mote| mote.vel[0].hypot(mote.vel[1]) > 0.5);
            show.knocked |= page.text.len() < characters;
            characters = page.text.len();
        }
        show
    }

    #[test]
    fn she_gathers_herself_before_the_next_jump() {
        // Land, crouch, spring: a cat that left the ground on the frame it landed
        // would be a cat bouncing.
        let page = page();
        let mut rng = Rng::new(55);
        let shelf = *page
            .shelves
            .iter()
            .find(|shelf| {
                shelf.y > page.floor() + 1.0 && shelf.y < page.floor() + CLIMB
            })
            .expect("a line within a bound");
        let x = shelf.nearest_point(page.width() * 0.5);
        let mut kitten = Kitten::new(x, page.floor());
        kitten.mood = Mood::Perch;
        kitten.arrived = false;
        kitten.patience = 8.0;
        kitten.target = [x, shelf.y, 0.0];
        kitten.next_leap = 0.25;
        for _ in 0..6 {
            let mut senses = senses_on(&mut rng, &page, None, 0.0);
            kitten.think(1.0 / 120.0, &mut senses);
        }
        assert!(!kitten.airborne, "she left the ground while gathering herself");
        for _ in 0..(120 * 2) {
            let mut senses = senses_on(&mut rng, &page, None, 0.0);
            kitten.think(1.0 / 120.0, &mut senses);
            if kitten.airborne {
                return;
            }
        }
        panic!("she never jumped once she was ready");
    }

    #[test]
    fn she_does_not_jump_at_a_line_that_is_not_there_any_more() {
        // The paw takes a line apart where it lands, so the place she was going can
        // be a hole by the time she stands under it. Jumping at the hole again is
        // bouncing in place, which is what this is here to stop.
        let mut page = page();
        let mut rng = Rng::new(56);
        let mut hole = None;
        for y in [18.0, 22.0, 26.0, 30.0] {
            for x in [8.0, page.width() * 0.5, page.width() - 6.0] {
                if page.surface(x, y + 0.05) < y - 1.0 {
                    hole = Some([x, y, 0.0]);
                }
            }
        }
        let hole = hole.expect("a stretch of the page with nothing on it");
        let mut kitten = Kitten::new(hole[0], page.floor());
        kitten.mood = Mood::Perch;
        kitten.arrived = false;
        kitten.patience = 20.0;
        kitten.target = hole;
        for _ in 0..(120 * 4) {
            let mut senses = senses_on(&mut rng, &page, None, 0.0);
            kitten.think(1.0 / 120.0, &mut senses);
            assert!(!kitten.airborne, "she jumped at a hole at {hole:?}");
            page.step_tokens(1.0 / 120.0);
        }
    }

    #[test]
    fn the_lower_she_is_the_more_she_wants_up() {
        // The whole point of a page of code with a wall of furniture on the right of it:
        // whatever else she is doing, the way on is over her head. What that is worth is
        // a function of how far up the page she already is, so the two moods that go up,
        // and the furniture she goes up on, are worth more on the rug than in the middle.
        let page = page();
        let mut rng = Rng::new(0x5EED);
        let up = |rng: &mut Rng, y: f32| {
            let kitten = Kitten::new(30.0, y);
            let senses = senses_on(rng, &page, None, 1.0);
            kitten
                .weights(&senses)
                .iter()
                .filter(|(mood, _)| matches!(mood, Mood::Climb | Mood::Swoop | Mood::Perch))
                .map(|(_, weight)| *weight)
                .sum::<usize>()
        };
        let (floor, middle) = (up(&mut rng, 2.0), up(&mut rng, page.rows as f32 * 0.5));
        assert!(floor > 0, "nothing to go up from the floor");
        assert!(
            floor > middle,
            "{floor} on the floor against {middle} in the middle of the page"
        );
    }

    #[test]
    fn left_alone_it_finds_something_to_do_on_the_page() {
        // The whole point of the mode: nothing happens, and the cat makes
        // something happen. A minute of it has to contain play, not just sitting.
        let mut page = page();
        let mut rng = Rng::new(0xCA7);
        let mut kitten = Kitten::new(40.0, 20.0);
        let show = live_for(&mut page, &mut kitten, &mut rng, 180.0);
        for mood in ["swat", "perch", "chase", "zoomies", "climb"] {
            assert!(
                show.moods.get(mood).copied().unwrap_or(0) > 0,
                "never {mood}: {:?}",
                show.moods
            );
        }
        assert!(show.shook, "the furniture was never touched");
        assert!(show.words_moved, "no word was ever batted");
        assert!(show.dust_thrown, "no dust was ever thrown");
        assert!(show.jumped, "she never left the floor");
        assert!(show.knocked, "the page lost nothing to her");
        assert!(show.clawed, "she climbed the page without a claw in it");
        assert!(show.washed, "she never washed her face");
    }

    #[test]
    fn a_swat_is_aimed_at_the_furniture_and_lands_on_it() {
        let mut page = page();
        let mut rng = Rng::new(31);
        let mut kitten = Kitten::new(40.0, page.floor());
        {
            let mut senses = senses_on(&mut rng, &page, None, 0.0);
            kitten.start(Mood::Swat, &mut senses);
        }
        assert_eq!(kitten.mood, Mood::Swat);
        // The walk goes to the thing, not to where it happened to be standing: to
        // the side of it, because the paw is what reaches and the cat is not a paw.
        let prop = nearest_prop(&page.props, kitten.pos).expect("furniture");
        let half = prop.size[0] * GLYPH_WIDTH * 0.5;
        assert!(
            (kitten.target[0] - prop.pos[0]).abs() > half
                && (kitten.target[0] - prop.pos[0]).abs() < half + 1.6,
            "{:?} against a box of half width {half}",
            kitten.target
        );
        let mut struck = false;
        for _ in 0..(120 * 6) {
            let strike = {
                let mut senses = senses_on(&mut rng, &page, None, 0.0);
                kitten.think(1.0 / 120.0, &mut senses)
            };
            if strike {
                struck = true;
                page.strike(kitten.paw(), &mut rng);
                break;
            }
        }
        assert!(struck, "the paw never came down");
        let shaken = page.props.iter().filter(|prop| prop.wobble > 0.0).count();
        assert!(shaken > 0, "the paw missed the furniture");
    }

    #[test]
    fn it_sits_on_the_table_it_climbed() {
        let page = page();
        let mut rng = Rng::new(32);
        let mut kitten = Kitten::new(10.0, 8.0);
        let perch = nearest_perch(&page.props, kitten.pos, f32::MAX)
            .expect("somewhere to sit")
            .clone();
        // Her paws one bound under the top: what this test is about is the climb,
        // not how she crossed the whole page to get under it.
        kitten.pos = [perch.pos[0], perch.top() - 4.0, 0.0];
        {
            // Nothing to jump on but the furniture: on a page with lines in reach
            // she would take the next line up instead, which is the other test.
            let mut senses = Senses {
                shelves: &[],
                ..senses_on(&mut rng, &page, None, 0.0)
            };
            kitten.start(Mood::Perch, &mut senses);
        }
        let mut jumped = false;
        for _ in 0..(120 * 8) {
            let mut senses = senses_on(&mut rng, &page, None, 0.0);
            kitten.think(1.0 / 120.0, &mut senses);
            jumped |= kitten.airborne;
            if kitten.mood != Mood::Perch {
                break;
            }
        }
        assert!(jumped, "she got onto the table without leaving the floor");
        // She is on the top edge of it, and folded up the way a sitting cat is.
        assert!(
            (kitten.pos[1] - perch.top()).abs() < 0.35,
            "{:?} against a top of {}",
            kitten.pos,
            perch.top()
        );
        assert!(kitten.sit > 0.5, "{}", kitten.sit);
    }

    #[test]
    fn it_jumps_onto_a_line_of_code_and_sits_on_it() {
        let page = page();
        let mut rng = Rng::new(41);
        // Started on the lowest long line, because a cat on the rug is under the
        // first line of code by more than a bound, and this is about the bound.
        let lowest = page
            .shelves
            .iter()
            .filter(|shelf| shelf.span[1] - shelf.span[0] > 2.0)
            .min_by(|a, b| a.y.total_cmp(&b.y))
            .expect("a line of code");
        let mut kitten = Kitten::new((lowest.span[0] + lowest.span[1]) * 0.5, lowest.y);
        {
            let mut senses = senses_on(&mut rng, &page, None, 0.0);
            kitten.start(Mood::Perch, &mut senses);
        }
        let aim = kitten.target;
        assert!(aim[1] > lowest.y + 0.5, "{aim:?} was not above her");
        let mut jumped = false;
        for _ in 0..(120 * 8) {
            let mut senses = senses_on(&mut rng, &page, None, 0.0);
            kitten.think(1.0 / 120.0, &mut senses);
            jumped |= kitten.airborne;
            if kitten.mood != Mood::Perch {
                break;
            }
        }
        assert!(jumped, "she never left the floor");
        // On the line, not through it, and looking like a cat that got up there.
        assert!(
            (kitten.pos[1] - aim[1]).abs() < 0.2,
            "{:?} against {aim:?}",
            kitten.pos
        );
        assert!(kitten.sit > 0.5, "{}", kitten.sit);
    }

    #[test]
    fn it_steps_off_the_end_of_a_line_and_drops_to_the_one_below() {
        let page = page();
        let mut rng = Rng::new(42);
        let shelf = *page
            .shelves
            .iter()
            .find(|shelf| shelf.y > page.floor() + 3.0 && shelf.span[1] < page.width() - 6.0)
            .expect("a line with an edge");
        let mut kitten = Kitten::new(shelf.span[0] + 1.0, shelf.y);
        // Something to walk to beyond the end of the line she is on: gravity is
        // what takes her down, not a decision.
        kitten.mood = Mood::Wander;
        kitten.timer = 3.0;
        kitten.target = [shelf.span[1] + 4.0, page.floor(), 0.0];
        let mut fell = false;
        for _ in 0..(120 * 4) {
            let mut senses = senses_on(&mut rng, &page, None, 0.0);
            kitten.think(1.0 / 120.0, &mut senses);
            fell |= kitten.landed;
        }
        assert!(fell, "she walked off the line and stayed in the air");
        assert!(
            kitten.pos[1] < shelf.y - 0.5,
            "{:?} never left a line at {}",
            kitten.pos,
            shelf.y
        );
        let surface = page.surface(kitten.pos[0], kitten.pos[1] + 0.01);
        assert!(
            (kitten.pos[1] - surface).abs() < 0.05,
            "{:?} is not standing on the page at {surface}",
            kitten.pos
        );
    }

    #[test]
    fn a_sleeping_cat_puts_a_zzz_over_its_head() {
        let page = page();
        let mut rng = Rng::new(43);
        let mut kitten = Kitten::new(20.0, page.floor());
        let mut seen = false;
        for _ in 0..(120 * 6) {
            let mut senses = senses_on(&mut rng, &page, None, SLEEP_AFTER + 1.0);
            kitten.think(1.0 / 120.0, &mut senses);
            seen |= kitten.effect == Some(Effect::Zzz);
        }
        assert_eq!(kitten.mood, Mood::Sleep);
        assert!(seen, "she slept without a Zzz");
        // A sleeping cat is a loaf, so the glyph comes off the top of the loaf
        // rather than off the top of the cat she was standing up.
        assert!(
            kitten.crown()[1] > kitten.pos[1] + 0.5,
            "{:?} off {:?}",
            kitten.crown(),
            kitten.pos
        );
    }

    #[test]
    fn washing_puts_a_paw_in_front_of_its_face() {
        let page = page();
        let mut rng = Rng::new(44);
        let mut kitten = Kitten::new(20.0, page.floor());
        {
            let mut senses = senses_on(&mut rng, &page, None, 0.0);
            kitten.start(Mood::Clean, &mut senses);
        }
        let mut licked = false;
        for _ in 0..(120 * 3) {
            let mut senses = senses_on(&mut rng, &page, None, 0.0);
            kitten.think(1.0 / 120.0, &mut senses);
            licked |= kitten.effect == Some(Effect::Lick);
        }
        assert!(kitten.sit > 0.5, "she washed standing up: {}", kitten.sit);
        assert!(licked, "nothing came off the paw");
        // A box in front of the muzzle, which is the paw.
        let muzzle = kitten.muzzle();
        let paw = kitten
            .blocks()
            .into_iter()
            .filter(|block| block.shade == Shade::FurLight)
            .any(|block| {
                block.pos[2] > muzzle[2] - 0.2
                    && (block.pos[1] - muzzle[1]).abs() < 0.4
                    && (block.pos[0] - muzzle[0]).abs() < 0.3
            });
        assert!(paw, "no paw at the face");
    }

    #[test]
    fn it_does_not_swat_the_furniture_it_is_standing_on() {
        let page = page();
        let mut rng = Rng::new(45);
        let perch = page
            .props
            .iter()
            .find(|prop| prop.kind.is_perch())
            .expect("a table")
            .clone();
        let mut kitten = Kitten::new(perch.pos[0], perch.top());
        {
            let mut senses = senses_on(&mut rng, &page, None, 0.0);
            kitten.start(Mood::Swat, &mut senses);
        }
        // What matters is the thing she picked, not where she is standing to reach it: on
        // a page with a wall of furniture, everything above the sofa is swatted from the
        // sofa, so a paw on the sofa is the rule working rather than the rule broken.
        let aim = kitten.aim.expect("something to swat from a table");
        assert_ne!(aim, perch.pos, "she went for the furniture she was on");
        let her_own = (aim[1] - perch.top()).abs() < 0.2
            && (aim[0] - perch.pos[0]).abs() <= perch.size[0] * GLYPH_WIDTH * 0.5;
        assert!(!her_own, "the thing under her paws: {aim:?}");
    }

    #[test]
    fn a_chase_follows_dust_that_keeps_moving() {
        // Dust does not sit still, so the target is read again every frame. The speck it
        // follows is put on her own line, because a chase after one over her head is a
        // cat jumping at it again and again, and that is the leap's business rather than
        // the chase's.
        let mut page = page();
        // Its own dust is put aside, so that the speck she chases is the one this test
        // put there rather than whichever mote the page's seed happened to leave beside
        // her paw.
        page.motes.clear();
        page.motes.insert(
            0,
            Mote {
                pos: [13.0, 20.0, 0.0],
                vel: [0.8, 0.0],
                drift: 0.2,
                phase: 0.0,
                size: 0.05,
                life: f32::MAX,
            },
        );
        let mut rng = Rng::new(33);
        let mut kitten = Kitten::new(10.0, 20.0);
        {
            let mut senses = senses_on(&mut rng, &page, None, 0.0);
            kitten.start(Mood::Chase, &mut senses);
        }
        let first = kitten.target;
        let mut fastest = 0.0f32;
        for _ in 0..(120 * 4) {
            let mut senses = senses_on(&mut rng, &page, None, 0.0);
            kitten.think(1.0 / 120.0, &mut senses);
            page.step_motes(1.0 / 120.0);
            fastest = fastest.max(kitten.speed);
            if kitten.mood != Mood::Chase {
                break;
            }
        }
        assert_ne!(kitten.target, first, "the chase never moved on");
        // And it is a run, not a stroll, and it stays on the page.
        assert!(fastest > WALK, "{fastest} was the quickest she went");
        assert!(kitten.pos[0] > 0.0 && kitten.pos[0] < page.width());
    }

    #[test]
    fn a_lap_is_a_run_that_rakes_the_text() {
        // The zoomies used to be a cat going quickly for no reason: a lap of the margin
        // leaves the page as it was. A lap is the same animal as a dash, so it rakes,
        // and it starts on a line rather than on the rug.
        let page = page();
        let mut rng = Rng::new(0x2041);
        let shelf = page
            .shelves
            .iter()
            .find(|shelf| shelf.row > 10 && shelf.span[1] - shelf.span[0] > 10.0)
            .copied()
            .expect("a long line");
        let mut kitten = Kitten::new(shelf.span[0] + 1.0, shelf.y);
        {
            let mut senses = senses_on(&mut rng, &page, None, 1.0);
            kitten.start(Mood::Zoomies, &mut senses);
        }
        assert!(kitten.raking(), "a lap does not rake");
        assert!(
            (kitten.target[1] - shelf.y).abs() < 0.01,
            "the lap left the line for {:?}",
            kitten.target
        );
        let mut rakes = 0;
        let mut quickest = 0.0f32;
        for _ in 0..(120 * 2) {
            let mut senses = senses_on(&mut rng, &page, None, 1.0);
            kitten.think(1.0 / 120.0, &mut senses);
            quickest = quickest.max(kitten.speed);
            if kitten.rake.is_some() {
                rakes += 1;
            }
        }
        assert!(rakes >= 2, "{rakes} rakes in a lap");
        assert!(quickest > RUN, "{quickest} cells a second is not a lap");
    }

    #[test]
    fn a_dash_with_time_left_on_its_clock_runs_another_leg() {
        // A run that stopped the moment it arrived would be a cat crossing a short
        // line and standing at the end of it. What a dash has is time, so the legs go
        // on while there is any: this is what makes a run an event rather than a step.
        let page = page();
        let mut rng = Rng::new(0x1E65);
        let shelf = page
            .shelves
            .iter()
            .find(|shelf| shelf.row > 5 && (4.0..9.5).contains(&(shelf.span[1] - shelf.span[0])))
            .copied()
            .expect("a short line");
        let mut kitten = Kitten::new(shelf.span[0] + 0.5, shelf.y);
        let mut senses = senses_on(&mut rng, &page, None, 1.0);
        kitten.start(Mood::Dash, &mut senses);
        let first = kitten.target;
        // The end of the first leg, with the clock still on it. A run that stopped here
        // would be a cat crossing a line and standing at the end of it.
        kitten.pos = first;
        let mut senses = senses_on(&mut rng, &page, None, 1.0);
        kitten.think(1.0 / 120.0, &mut senses);
        assert_ne!(kitten.target, first, "the run stood at the end of its first leg");
        assert_eq!(kitten.mood, Mood::Dash, "the run ended early");
    }

    /// The highest thing on the page she can stand on: the top of the wall.
    fn highest_perch(page: &Page) -> &Prop {
        page.props
            .iter()
            .filter(|prop| prop.kind.is_perch())
            .max_by(|a, b| a.top().total_cmp(&b.top()))
            .expect("a perch")
    }

    #[test]
    fn a_swoop_heads_for_the_far_side_and_down() {
        // The wall is climbed for this: its top is high and off to one side, so the whole
        // width of the page is below her on one hand and nothing is above her. What a leg
        // of a swoop is, then, is a super jump of a leg's length to the left and a few
        // lines down -- not the crossing of the page in one bound, which is a fall with a
        // cat on it.
        let page = page();
        let mut rng = Rng::new(0x5EED);
        let perch = highest_perch(&page);
        let kitten = Kitten::new(perch.pos[0], perch.top());
        let senses = senses_on(&mut rng, &page, None, 0.0);
        let (to, dir) = kitten
            .swoop_target(&senses)
            .expect("somewhere to swoop to");
        assert_eq!(dir, -1.0, "the far side of the wall is the code");
        let across = (to[0] - kitten.pos[0]) * dir;
        let drop = kitten.pos[1] - to[1];
        assert!(
            (SWOOP_MIN..=SWOOP_LEG).contains(&across),
            "a leg of {across} cells"
        );
        assert!(
            (-SUPER_RISE..=SWOOP_DROP).contains(&drop),
            "a drop of {drop} lines: {:?} to {to:?}",
            kitten.pos
        );
        // And it is a landing rather than a point in the air: what holds her up at the
        // far end of the leg is the ledge she can see from where she stands.
        let stand = senses.surface(to[0], to[1] + 0.05);
        assert!(
            (stand - to[1]).abs() < 0.01,
            "nothing to land on at {to:?}: the surface there is {stand}"
        );
    }

    #[test]
    fn a_super_jump_goes_back_up_to_the_furniture() {
        // The other half of the crossing. A cat in the code with the furniture over her
        // head has the whole width of the page between her and the next foothold, and a
        // leap that could only go down would leave her down there for good. What a super
        // jump gains it gains on the wall: the landing is a flat top, and it is higher
        // than the line she left.
        let page = page();
        let mut rng = Rng::new(0x5EED);
        let standable: Vec<&Prop> = page
            .props
            .iter()
            .filter(|prop| prop.kind.is_perch())
            .collect();
        let mut gains = Vec::new();
        for shelf in page.shelves.iter().filter(|shelf| shelf.span[1] > 20.0) {
            let kitten = Kitten::new(shelf.span[1] - 1.0, shelf.y);
            let senses = senses_on(&mut rng, &page, None, 0.0);
            let Some((to, dir)) = kitten.swoop_target(&senses) else {
                continue;
            };
            let rise = to[1] - kitten.pos[1];
            if dir == 1.0 && rise > 0.0 && rise <= SUPER_RISE {
                gains.push((shelf.y, to, rise));
            }
        }
        let (from, to, rise) = *gains
            .first()
            .expect("no super jump off the code gained any ground");
        assert!(rise <= SUPER_RISE, "a rise of {rise} lines from {from}");
        // And it comes down on the flat top of a piece of furniture, which is the whole
        // point: a landing in the middle of a fern would be a cat in a plant.
        assert!(
            standable
                .iter()
                .any(|prop| (prop.top() - to[1]).abs() < 0.01),
            "nothing to land on at {to:?}"
        );
    }

    #[test]
    fn the_top_of_a_climb_is_where_the_swoop_begins() {
        // Climbing and swooping are one circuit, and this is the joint: the wall is
        // climbed for the top of it, and a cat that has got up there leaves it the long
        // way instead of standing on the highest shelf until the clock runs out.
        let page = page();
        let mut rng = Rng::new(0x5EED);
        let perch = highest_perch(&page);
        let mut kitten = Kitten::new(perch.pos[0], perch.top());
        kitten.mood = Mood::Climb;
        kitten.arrived = true;
        kitten.timer = 5.0;
        let mut senses = senses_on(&mut rng, &page, None, 1.0);
        kitten.think(1.0 / 120.0, &mut senses);
        assert_eq!(kitten.mood, Mood::Swoop, "the climb stopped at the top");
        assert!(
            kitten.target[1] < kitten.pos[1],
            "the swoop went up: {:?}",
            kitten.target
        );
        assert!(
            kitten.target[0] < kitten.pos[0] - SWOOP_MIN,
            "the swoop went right: {:?}",
            kitten.target
        );
    }

    #[test]
    fn a_run_from_the_rug_climbs_before_it_goes() {
        // The rug is drawn in characters like everything else, at a third of the
        // opacity, and it is not text: a run along it takes nothing. So the fast moods
        // leave the floor first, and the page's own lines are what a run is for.
        let page = page();
        let mut rng = Rng::new(0x2071);
        let shelf = page
            .shelves
            .iter()
            .filter(|shelf| shelf.y > page.floor() && shelf.y - page.floor() <= CLIMB)
            .min_by(|a, b| a.y.total_cmp(&b.y))
            .copied()
            .expect("a line over the rug");
        let mut kitten = Kitten::new(shelf.nearest_point(page.width() * 0.5), page.floor());
        let mut senses = senses_on(&mut rng, &page, None, 1.0);
        kitten.start(Mood::Dash, &mut senses);
        assert!(
            kitten.target[1] > page.floor() + 0.6,
            "the run stayed on the rug at {:?}",
            kitten.target
        );
        // And the lap does the same, because it is the same animal.
        kitten.mood = Mood::Wander;
        let mut senses = senses_on(&mut rng, &page, None, 1.0);
        kitten.start(Mood::Zoomies, &mut senses);
        assert!(kitten.target[1] > page.floor() + 0.6, "{:?}", kitten.target);
    }

    #[test]
    fn a_climb_aims_at_the_highest_foothold_one_bound_can_reach() {
        // A cat that took the next line up would take thirty bounds to cross the page
        // upwards and stop at every one of them. A climb is a height, not a hop: what
        // it goes for is the top of what it can reach.
        let page = page();
        let mut rng = Rng::new(0x41C);
        let mut kitten = Kitten::new(page.width() * 0.5, page.floor());
        let mut senses = senses_on(&mut rng, &page, None, 0.0);
        kitten.start(Mood::Climb, &mut senses);
        assert_eq!(kitten.mood, Mood::Climb);
        let aim = kitten.target;
        let rise = aim[1] - kitten.pos[1];
        assert!(rise > 0.6 && rise <= CLIMB, "{aim:?} is {rise} above her");
        // Nothing standable that one bound could have reached is above the place she
        // picked: not a line, and not the top of the furniture either.
        let mut above: Vec<[f32; 2]> = page
            .shelves
            .iter()
            .map(|shelf| [shelf.nearest_point(kitten.pos[0]), shelf.y])
            .collect();
        for prop in page.props.iter().filter(|prop| prop.kind.is_perch()) {
            above.push([prop.pos[0], prop.top()]);
        }
        for [x, y] in above {
            let rise = y - kitten.pos[1];
            let near = (x - kitten.pos[0]).abs() <= REACH;
            assert!(
                !(rise > 0.75 && rise <= CLIMB && near && y > aim[1] + 0.05),
                "she left [{x}, {y}] behind for {aim:?}"
            );
        }
    }

    #[test]
    fn a_climb_ends_where_the_page_does_and_nowhere_else() {
        // The one thing an ascent must not do is give up with something still over its
        // head: she takes a foothold and then the one above it, and what ends the mood
        // is the page running out. A page of code is ragged, so where that happens
        // depends on the column she is in and not on the seed.
        let page = page();
        let mut rng = Rng::new(0x41D);
        let mut kitten = Kitten::new(page.width() * 0.5, page.floor());
        {
            let mut senses = senses_on(&mut rng, &page, None, 0.0);
            kitten.start(Mood::Climb, &mut senses);
        }
        let start = kitten.pos[1];
        let mut bounds = 0;
        let mut was_air = false;
        let mut highest = start;
        for _ in 0..(120 * 30) {
            let mut senses = senses_on(&mut rng, &page, None, 0.0);
            kitten.think(1.0 / 120.0, &mut senses);
            bounds += usize::from(kitten.airborne && !was_air);
            was_air = kitten.airborne;
            highest = highest.max(kitten.pos[1]);
            if kitten.mood != Mood::Climb {
                let senses = senses_on(&mut rng, &page, None, 0.0);
                assert!(
                    foothold_above(&senses, kitten.pos).is_none(),
                    "she gave up at {:?} with a foothold still over her",
                    kitten.pos
                );
                break;
            }
        }
        assert!(bounds >= 1, "she never left the ground");
        assert!(highest > start + 1.0, "{} then {highest}", start);
    }

    #[test]
    fn the_claws_of_a_climb_are_in_the_line_it_has_left() {
        // The row she is going to has to still be there, or the climb has nowhere to
        // put its paws. The claws are fired from under her back paws, which is a line
        // below her feet, and never from the row she is on her way to.
        let page = page();
        let mut rng = Rng::new(0x41E);
        let mut kitten = Kitten::new(page.width() * 0.5, page.floor());
        {
            let mut senses = senses_on(&mut rng, &page, None, 0.0);
            kitten.start(Mood::Climb, &mut senses);
        }
        let mut claws = 0;
        for _ in 0..(120 * 12) {
            let mut senses = senses_on(&mut rng, &page, None, 0.0);
            kitten.think(1.0 / 120.0, &mut senses);
            let Some(at) = kitten.claw else {
                continue;
            };
            assert_eq!(kitten.mood, Mood::Climb, "claws outside a climb");
            claws += 1;
            assert_eq!(kitten.mood, Mood::Climb);
            assert!(at[1] < kitten.pos[1], "{at:?} is not under her feet");
            assert!(
                at[1] < kitten.jump_to[1] - 0.5,
                "{at:?} is the row she is landing on"
            );
        }
        assert!(claws >= 4, "{claws} claws in an ascent");
    }

    #[test]
    fn the_sitting_clock_fills_while_she_rests_and_empties_while_she_plays() {
        let mut rng = Rng::new(34);
        let mut kitten = Kitten::new(20.0, 20.0);
        let resting = kitten.rest;
        kitten.mood = Mood::Sit;
        kitten.timer = 30.0;
        for _ in 0..(120 * 2) {
            kitten.think(1.0 / 120.0, &mut senses(&mut rng, None, 0.0));
        }
        assert!(kitten.rest > resting, "{} then {}", kitten.rest, resting);
        let rested = kitten.rest;
        kitten.mood = Mood::Zoomies;
        kitten.timer = 30.0;
        kitten.target = [30.0, 25.0, 0.0];
        for _ in 0..(120 * 2) {
            kitten.think(1.0 / 120.0, &mut senses(&mut rng, None, 0.0));
        }
        assert!(kitten.rest < rested, "{} then {}", kitten.rest, rested);
    }

    #[test]
    fn a_dash_is_faster_than_a_run() {
        // The name of the mood is a promise about speed, and this is the test that
        // holds it to it: twice a run, and she gets there without being told twice.
        const { assert!(DASH > RUN * 1.5) };
        let page = page();
        let mut rng = Rng::new(0x0D15);
        let shelf = page
            .shelves
            .iter()
            .max_by(|a, b| {
                let (a, b) = (a.span[1] - a.span[0], b.span[1] - b.span[0]);
                a.total_cmp(&b)
            })
            .copied()
            .expect("a line to run along");
        let mut kitten = Kitten::new(shelf.span[0] + 2.0, shelf.y);
        let mut senses = senses_on(&mut rng, &page, None, 1.0);
        kitten.start(Mood::Dash, &mut senses);
        let furthest = (kitten.target[0] - kitten.pos[0]).abs();
        assert!(furthest > 4.0, "the run is {furthest} cells long");
        let mut quickest = 0.0f32;
        for _ in 0..120 {
            kitten.think(1.0 / 120.0, &mut senses);
            quickest = quickest.max(kitten.speed);
        }
        assert!(quickest > RUN, "{quickest} cells a second is a run");
        assert!(quickest <= DASH + 0.01, "{quickest} is faster than a dash");
    }

    #[test]
    fn after_a_dash_the_cat_wants_to_wash_itself() {
        // A run is the most tiring thing she does, so what follows it is a wash
        // rather than another lap: without that she is a cat made of moods, and the
        // mode looks like a generator instead of an animal.
        let page = page();
        let mut rng = Rng::new(0x11CE);
        let mut kitten = Kitten::new(20.0, 20.0);
        let mut weigh = |kitten: &Kitten, mood: Mood| {
            let senses = senses_on(&mut rng, &page, None, 1.0);
            let table = kitten.weights(&senses);
            table
                .iter()
                .find(|(what, _)| *what == mood)
                .expect("every mood is weighed")
                .1
        };
        kitten.mood = Mood::Bat;
        let (sit, groom, clean, dash) = (
            weigh(&kitten, Mood::Sit),
            weigh(&kitten, Mood::Groom),
            weigh(&kitten, Mood::Clean),
            weigh(&kitten, Mood::Dash),
        );
        kitten.mood = Mood::Dash;
        assert!(weigh(&kitten, Mood::Sit) > sit);
        assert!(weigh(&kitten, Mood::Groom) > groom);
        assert!(weigh(&kitten, Mood::Clean) > clean);
        // And what she does not want, having just done it, is another run.
        assert_eq!(weigh(&kitten, Mood::Dash), 0);
        assert!(dash > 0, "she never wants to run at all");
    }

    #[test]
    fn every_mood_has_a_name_and_a_pose() {
        for mood in [
            Mood::Wander,
            Mood::Follow,
            Mood::Pounce,
            Mood::Bat,
            Mood::Sit,
            Mood::Groom,
            Mood::Sleep,
            Mood::Swat,
            Mood::Chase,
            Mood::Perch,
            Mood::Zoomies,
            Mood::Climb,
            Mood::Dash,
            Mood::Swoop,
        ] {
            assert!(!mood.name().is_empty());
            let kitten = Kitten {
                mood,
                ..Kitten::default()
            };
            assert!(kitten.blocks().len() >= 20, "{}", mood.name());
        }
    }
}
