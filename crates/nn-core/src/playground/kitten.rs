//! The kitten: what it does, and what it looks like doing it.
//!
//! The whole animal is a handful of boxes: a body, a head, two ears, four legs,
//! four paws, a muzzle, a nose, two eyes and six tail segments. At two and a
//! quarter lines tall that is enough to read as a cat, and it means the renderer
//! has one mesh and one draw call whatever the pose is. A smooth-skinned quadruped
//! would be a rig, a weight table and a mesh tool for something the size of a
//! postage stamp.
//!
//! The cat is built facing `+z` in its own space, and `+z` on the page comes out
//! towards the reader. So it is turned a quarter turn to walk along the page --
//! `yaw` is always `±PI/2` -- which is also what puts it in profile, the way a
//! small animal on a page reads best. It moves in `x` and `y`, up and down lines
//! of text, and never in `z`.
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
            hop: 0.0,
            blink: 0.0,
            breath: 0.0,
            tail: 0.0,
            next_blink: 2.5,
        }
    }

    /// How tall the kitten stands, in lines.
    ///
    /// The ears are the top: the head's centre plus half its height plus the ear.
    pub fn height(&self) -> f32 {
        HEAD[1] + HEAD_SIZE[1] * 0.5 + EAR_SIZE[1]
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
        let _ = senses;
    }

    fn distance_to(&self, at: [f32; 3]) -> f32 {
        (at[0] - self.pos[0]).hypot(at[1] - self.pos[1])
    }

    /// The boxes that draw it, in world space.
    pub fn blocks(&self) -> Vec<Block> {
        let mut out = Vec::with_capacity(24);
        let (sin, cos) = self.yaw.sin_cos();
        let asleep = self.mood == Mood::Sleep;
        let breathing = (self.breath * 1.9).sin() * 0.02;
        // Walking lifts and drops the body, and a pounce stretches it out.
        let bob = if self.speed > 0.2 {
            (self.phase * 2.0).sin() * 0.05
        } else {
            0.0
        };
        let lift = self.hop + bob;
        let stretch = 1.0 + self.hop * 0.06;
        let settle = self.crouch * 0.35;

        let mut place = |local: [f32; 3], size: [f32; 3], shade: Shade| {
            let x = local[0] * cos + local[2] * sin;
            let z = -local[0] * sin + local[2] * cos;
            out.push(Block {
                pos: [self.pos[0] + x, self.pos[1] + local[1], self.pos[2] + z + STAND],
                size,
                yaw: self.yaw,
                shade,
            });
        };

        // Body: lower and longer when asleep, squashed when crouched. Short,
        // because a kitten is mostly head and the head is the thing that makes it
        // read as a kitten rather than as a small dog.
        let (body_y, body_z, body_size) = if asleep {
            (0.5, 0.1, [1.05, 0.6, 1.6])
        } else {
            (
                0.92 + lift - settle,
                0.0,
                [
                    1.02,
                    (0.82 + breathing) * (1.0 - 0.18 * self.crouch),
                    1.24 * stretch,
                ],
            )
        };
        place([0.0, body_y, body_z], body_size, Shade::Fur);
        // The back, a shade darker, which is what makes it read as round.
        place(
            [0.0, body_y + body_size[1] * 0.28, body_z - 0.15],
            [body_size[0] * 0.86, body_size[1] * 0.34, body_size[2] * 0.8],
            Shade::FurDark,
        );
        // Chest and rump, so the silhouette is not one slab. Asleep they sink
        // into the body rather than standing proud of it, or the cat reads as a
        // crate with a lid.
        if !asleep {
            place(
                [0.0, body_y + 0.02, body_z + 0.54 * stretch],
                [0.8, 0.62, 0.4],
                Shade::FurLight,
            );
        }
        place(
            [0.0, body_y + if asleep { -0.02 } else { 0.05 }, body_z - 0.56],
            if asleep {
                [1.0, 0.6, 0.46]
            } else {
                [1.0, 0.84, 0.46]
            },
            Shade::Fur,
        );

        // Head: forward and up when awake, tucked down and forward when asleep.
        let head = if asleep {
            [0.0, 0.56, 1.02]
        } else {
            [HEAD[0], HEAD[1] + lift - settle * 0.7, HEAD[2] * stretch]
        };
        let head_size = if asleep {
            [1.0, 0.56, 0.8]
        } else {
            HEAD_SIZE
        };
        place(head, head_size, Shade::Fur);
        // Muzzle and nose, at the front of the head, which is the side the cat
        // walks towards.
        place(
            [head[0], head[1] - 0.22, head[2] + 0.46],
            [0.5, 0.3, 0.26],
            Shade::FurLight,
        );
        place(
            [head[0], head[1] - 0.2, head[2] + 0.6],
            [0.14, 0.12, 0.11],
            Shade::Nose,
        );
        // Eyes and ears are laid out for the view the reader has: the cat is
        // always in profile, so its eyes belong on the sides of its head and its
        // two ears side by side along the body. Put them on the front face, which
        // is where they go on a real cat, and the reader sees one ear and no eye.
        let eye_y = if asleep { 0.04 } else { 0.18 * (1.0 - self.blink) + 0.04 };
        for side in [-1.0f32, 1.0] {
            place(
                [
                    head[0] + side * (head_size[0] * 0.5 - 0.02),
                    head[1] + 0.14,
                    head[2] + 0.2,
                ],
                [0.09, eye_y, 0.16],
                Shade::Eye,
            );
            place(
                [
                    head[0] + side * 0.16,
                    head[1] + head_size[1] * 0.5 + EAR_SIZE[1] * 0.18,
                    head[2] - 0.26 + side * 0.26,
                ],
                if asleep {
                    [0.24, 0.14, 0.3]
                } else {
                    EAR_SIZE
                },
                Shade::FurDark,
            );
        }

        // Legs: the front pair swings with the walk, the back pair against it.
        let swing = |offset: f32| {
            if asleep {
                0.0
            } else if self.speed > 0.2 {
                (self.phase * 2.0 + offset).sin() * 0.16
            } else {
                0.0
            }
        };
        for side in [-1.0f32, 1.0] {
            let front = swing(0.0);
            let back = swing(std::f32::consts::PI);
            if asleep {
                place([side * 0.4, 0.2, 0.5], [0.3, 0.3, 0.5], Shade::FurLight);
                place([side * 0.4, 0.2, -0.4], [0.3, 0.3, 0.5], Shade::FurLight);
            } else {
                place(
                    [side * 0.34, 0.38 + lift * 0.4, 0.46 + front],
                    [0.22, 0.68, 0.22],
                    Shade::Fur,
                );
                place(
                    [side * 0.34, 0.38 + lift * 0.4, -0.44 + back],
                    [0.24, 0.68, 0.24],
                    Shade::Fur,
                );
                place(
                    [side * 0.34, 0.09, 0.5 + front],
                    [0.28, 0.16, 0.34],
                    Shade::FurLight,
                );
                place(
                    [side * 0.34, 0.09, -0.4 + back],
                    [0.28, 0.16, 0.34],
                    Shade::FurLight,
                );
            }
        }

        // Tail: six segments on a travelling wave, wrapped round when asleep.
        for segment in 0..6 {
            let t = segment as f32 / 5.0;
            let wave = if asleep {
                (t * 2.4 - self.tail * 0.4).sin() * 0.5 * t
            } else {
                (self.tail - t * 1.5).sin() * 0.34 * t
            };
            place(
                [
                    wave * 0.9,
                    if asleep {
                        0.25 + t * 0.1
                    } else {
                        0.86 + t * 0.44 + wave * 0.3
                    },
                    -0.66 - t * 0.26 + wave * 0.5,
                ],
                [0.17 - t * 0.04, 0.17 - t * 0.04, 0.26 - t * 0.03],
                if segment % 2 == 0 {
                    Shade::Fur
                } else {
                    Shade::FurLight
                },
            );
        }
        out
    }
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

/// Where the head sits when the kitten is awake, in its own space.
const HEAD: [f32; 3] = [0.0, 1.38, 0.74];
/// The head's size: as tall as the body and deeper than it is wide, which is the
/// whole trick of a kitten seen from the side.
const HEAD_SIZE: [f32; 3] = [0.9, 1.0, 1.06];
/// An ear's size: tall, narrow, and separated from its twin, or the pair reads as
/// a hat.
const EAR_SIZE: [f32; 3] = [0.22, 0.46, 0.24];

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
        assert!(blocks.len() >= 22, "{}", blocks.len());
        let top = blocks
            .iter()
            .map(|block| block.pos[1] + block.size[1] * 0.5)
            .fold(0.0f32, f32::max);
        // Measured from the cat's own feet, which are wherever it is standing.
        assert!((top - kitten.pos[1] - kitten.height()).abs() < 0.35, "{top}");
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
