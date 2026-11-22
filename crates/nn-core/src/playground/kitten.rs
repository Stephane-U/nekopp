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
//! The behaviour is a small state machine with a timer per state. It sees three
//! things: where the pointer is on the page, how long ago anything happened, and
//! the seeded generator. Nothing else, which is what makes it testable without a
//! window.

use super::{Block, Rng, Shade};

/// How long nothing has to happen before the kitten gives up and sleeps.
pub const SLEEP_AFTER: f32 = 180.0;

/// How fast it walks, in characters per second.
pub const WALK: f32 = 2.1;
/// How fast it runs.
pub const RUN: f32 = 4.4;

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
    /// Curled up.
    Sleep,
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
            Mood::Sleep => "sleep",
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
    pub rng: &'a mut Rng,
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
        }
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
    /// cat striking a wasp.
    pub fn paw(&self) -> [f32; 3] {
        let (sin, _cos) = self.yaw.sin_cos();
        let forward = 1.2;
        [
            self.pos[0] + forward * sin,
            self.pos[1] + 0.4 + self.hop,
            self.pos[2],
        ]
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
        // The head follows her intention, and comes back to the reader when she
        // has none: a face turned away is a cat with no face.
        let want = match self.mood {
            Mood::Follow | Mood::Pounce if self.target[0] > self.pos[0] => 0.34,
            Mood::Follow | Mood::Pounce => -0.34,
            Mood::Sleep => 0.30,
            _ => 0.0,
        };
        self.head_yaw += (want - self.head_yaw) * (dt * 2.4).min(1.0);
        self.mood == Mood::Bat && self.timer > 0.28 && self.timer < 0.42
    }

    /// Picks what to do next.
    fn choose(&mut self, _dt: f32, senses: &mut Senses<'_>) {
        let pointer = senses.pointer.filter(|_| senses.pointer_age < 1.6);
        if let Some(pointer) = pointer {
            let distance = self.distance_to(pointer);
            // A pointer that has just stopped is the most interesting thing in
            // the room, so it overrides whatever the cat was doing.
            if self.mood != Mood::Bat || distance < 1.4 {
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
            }
        }

        // Reaching the pointer: something happens, chosen from the generator so
        // two sessions do not do the same thing.
        if self.mood == Mood::Follow && self.distance_to(self.target) < 0.8 {
            let pick = senses.rng.below(3);
            self.mood = [Mood::Bat, Mood::Sit, Mood::Groom][pick];
            self.timer = 0.9 + senses.rng.range(0.0, 1.6);
            self.crouch = if self.mood == Mood::Bat { 0.5 } else { 0.0 };
        }

        if self.timer > 0.0 {
            return;
        }
        // A mood ran out: pick the next one, weighted toward sitting about.
        let pick = senses.rng.below(10);
        self.mood = match pick {
            0..=3 => Mood::Wander,
            4..=5 => Mood::Sit,
            6 => Mood::Groom,
            7 => Mood::Bat,
            _ => Mood::Follow,
        };
        self.timer = 1.4 + senses.rng.range(0.0, 2.6);
        self.crouch = if self.mood == Mood::Bat { 0.5 } else { 0.0 };
        if self.mood == Mood::Follow && senses.pointer.is_none() {
            self.mood = Mood::Wander;
        }
        if matches!(self.mood, Mood::Wander | Mood::Follow) && self.distance_to(self.target) < 1.0
        {
            // Somewhere else on the page, a comfortable distance away, so the cat
            // walks across the text instead of circling on the spot.
            let (left, right, bottom, top) = room_to_walk(senses.page[0], senses.page[1]);
            let mut tries = 0;
            self.target = [
                senses.rng.range(left, right),
                senses.rng.range(bottom, top),
                0.0,
            ];
            while self.distance_to(self.target) < 4.0 && tries < 8 {
                self.target[0] = senses.rng.range(left, right);
                self.target[1] = senses.rng.range(bottom, top);
                tries += 1;
            }
        }
    }

    /// Walks, runs or leaps toward the target.
    fn move_by(&mut self, dt: f32, senses: &mut Senses<'_>) {
        let goal = match self.mood {
            Mood::Follow | Mood::Pounce => self.target,
            Mood::Wander => self.target,
            // Sitting, grooming, batting: stay put.
            _ => self.pos,
        };
        let dx = goal[0] - self.pos[0];
        let dy = goal[1] - self.pos[1];
        let distance = dx.hypot(dy);
        let wanted = match self.mood {
            Mood::Pounce => RUN * 1.25,
            Mood::Follow if distance > 3.0 => RUN,
            Mood::Follow | Mood::Wander => WALK,
            _ => 0.0,
        };
        let speed = if distance < 0.35 { 0.0 } else { wanted };
        // Approach the wanted speed smoothly, so the first frame of a run is not
        // an instant jump.
        self.speed += (speed - self.speed) * (dt * 6.0).min(1.0);
        if distance > 0.05 {
            // Facing is horizontal whatever way it is going: a cat that turned to
            // face up the page would be a cat seen end on, which is a blob.
            if dx.abs() > 0.02 {
                self.yaw = if dx > 0.0 {
                    std::f32::consts::FRAC_PI_2
                } else {
                    -std::f32::consts::FRAC_PI_2
                };
            }
            self.pos[0] += dx / distance * self.speed * dt;
            self.pos[1] += dy / distance * self.speed * dt;
        }
        // Paws stay on the page, and the page has edges. The kitten is kept far
        // enough from them that its ears and its tail stay on the paper: an ear cut
        // off by the window frame stops the animal from reading as an animal.
        let (left, right, bottom, top) = room_to_walk(senses.page[0], senses.page[1]);
        self.pos[0] = self.pos[0].clamp(left, right);
        self.pos[1] = self.pos[1].clamp(bottom, top);

        self.phase += self.speed * dt * 2.4;
        // The leap: up at the start, down before the landing.
        if self.mood == Mood::Pounce {
            let t = (0.55 - self.timer.max(0.0)) / 0.55;
            self.hop = (t * std::f32::consts::PI).sin() * 1.5;
        } else {
            self.hop = (self.hop - dt * 4.0).max(0.0);
        }
        let crouch = if self.mood == Mood::Sleep { 0.0 } else { self.crouch };
        self.crouch += (crouch - self.crouch) * (dt * 8.0).min(1.0);
        // A cat sits down when it has nothing to do, and gets up before it moves.
        let sit = match self.mood {
            Mood::Sit | Mood::Groom => 1.0,
            Mood::Sleep => 0.0,
            _ => 0.0,
        };
        self.sit += (sit - self.sit) * (dt * 3.4).min(1.0);
        let _ = senses;
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

    fn senses<'a>(rng: &'a mut Rng, pointer: Option<[f32; 3]>, age: f32) -> Senses<'a> {
        Senses {
            pointer,
            pointer_age: 0.2,
            input_age: age,
            page: [80.0, 40.0],
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
        for _ in 0..(120 * 3) {
            kitten.think(
                1.0 / 120.0,
                &mut senses(&mut rng, Some([12.5, 20.0, 0.0]), 0.0),
            );
        }
        assert!(kitten.pos[0] > 11.0, "{:?}", kitten.pos);
        assert!(matches!(kitten.mood, Mood::Follow | Mood::Bat | Mood::Sit | Mood::Groom));
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
