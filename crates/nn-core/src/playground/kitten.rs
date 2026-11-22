//! The kitten: what it does, and what it looks like doing it.
//!
//! The whole animal is a handful of boxes: a body, a head, two ears, four legs,
//! four paws, a muzzle, a nose, two eyes and six tail segments. At two and a half
//! characters tall that is enough to read as a cat, and it means the renderer has
//! one mesh and one draw call whatever the pose is. A smooth-skinned quadruped
//! would be a rig, a weight table and a mesh tool for something the size of a
//! postage stamp.
//!
//! The behaviour is a small state machine with a timer per state. It sees three
//! things: where the pointer is on the floor, how long ago anything happened, and
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
    /// Where the pointer meets the floor, if it is over the room.
    pub pointer: Option<[f32; 3]>,
    /// Seconds since the pointer last moved.
    pub pointer_age: f32,
    /// Seconds since any key or button.
    pub input_age: f32,
    pub rng: &'a mut Rng,
}

/// The kitten.
#[derive(Debug, Clone)]
pub struct Kitten {
    /// Between the paws, on the floor.
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
        Self::new([0.0, 0.0, -2.0])
    }
}

impl Kitten {
    /// A kitten sitting at `pos`.
    pub fn new(pos: [f32; 3]) -> Self {
        Self {
            pos,
            yaw: 0.0,
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

    /// How tall the kitten stands, in characters.
    ///
    /// The ears are the top: the head's centre plus half its height plus the ear.
    pub fn height(&self) -> f32 {
        HEAD[1] + HEAD_SIZE[1] * 0.5 + EAR_SIZE[1]
    }

    /// Where a paw lands when it strikes.
    pub fn paw(&self) -> [f32; 3] {
        let (sin, cos) = self.yaw.sin_cos();
        let forward = 1.6;
        [
            self.pos[0] + forward * sin,
            self.pos[1] + 0.5 + self.hop,
            self.pos[2] + forward * cos,
        ]
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
            self.target = [
                senses.rng.range(-5.5, 5.5),
                0.0,
                senses.rng.range(-7.5, 1.5),
            ];
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
        let dz = goal[2] - self.pos[2];
        let distance = dx.hypot(dz);
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
            self.yaw = dz.atan2(dx);
            self.pos[0] += dx / distance * self.speed * dt;
            self.pos[2] += dz / distance * self.speed * dt;
        }
        // Paws stay on the floor, and the room has walls.
        self.pos[0] = self.pos[0].clamp(-6.6, 6.6);
        self.pos[2] = self.pos[2].clamp(-8.0, 2.0);

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
        (at[0] - self.pos[0]).hypot(at[2] - self.pos[2])
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
                pos: [self.pos[0] + x, self.pos[1] + local[1], self.pos[2] + z],
                size,
                yaw: self.yaw,
                shade,
            });
        };

        // Body: lower and longer when asleep, squashed when crouched.
        let (body_y, body_z, body_size) = if asleep {
            (0.52, 0.12, [1.15, 0.66, 1.9])
        } else {
            (
                1.0 + lift - settle,
                0.0,
                [
                    1.15,
                    (0.95 + breathing) * (1.0 - 0.18 * self.crouch),
                    1.75 * stretch,
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
                [0.0, body_y + 0.02, body_z + 0.72 * stretch],
                [1.02, 0.86, 0.52],
                Shade::FurLight,
            );
        }
        place(
            [0.0, body_y + if asleep { -0.02 } else { 0.05 }, body_z - 0.72],
            if asleep {
                [1.08, 0.66, 0.5]
            } else {
                [1.08, 0.92, 0.5]
            },
            Shade::Fur,
        );

        // Head: forward and up when awake, tucked down and forward when asleep.
        let head = if asleep {
            [0.0, 0.64, 1.26]
        } else {
            [HEAD[0], HEAD[1] + lift - settle * 0.7, HEAD[2] * stretch]
        };
        let head_size = if asleep {
            [0.95, 0.6, 0.82]
        } else {
            HEAD_SIZE
        };
        place(head, head_size, Shade::Fur);
        // Muzzle, nose, eyes.
        place(
            [head[0], head[1] - 0.16, head[2] + 0.42],
            [0.5, 0.34, 0.28],
            Shade::FurLight,
        );
        place(
            [head[0], head[1] - 0.14, head[2] + 0.56],
            [0.15, 0.12, 0.1],
            Shade::Nose,
        );
        let eye_y = if asleep { 0.04 } else { 0.2 * (1.0 - self.blink) + 0.03 };
        for side in [-1.0f32, 1.0] {
            place(
                [head[0] + side * 0.25, head[1] + 0.12, head[2] + 0.4],
                [0.16, eye_y, 0.08],
                Shade::Eye,
            );
            // Ears, which flatten when asleep. They ride the top of the head
            // they belong to, so a smaller sleeping head keeps them attached.
            place(
                [
                    head[0] + side * 0.3,
                    head[1] + head_size[1] * 0.5 - 0.03,
                    head[2] - 0.05,
                ],
                if asleep {
                    [0.26, 0.14, 0.3]
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
                    [side * 0.36, 0.45 + lift * 0.4, 0.62 + front],
                    [0.24, 0.8, 0.24],
                    Shade::Fur,
                );
                place(
                    [side * 0.36, 0.45 + lift * 0.4, -0.6 + back],
                    [0.26, 0.8, 0.26],
                    Shade::Fur,
                );
                place(
                    [side * 0.36, 0.1, 0.66 + front],
                    [0.3, 0.18, 0.36],
                    Shade::FurLight,
                );
                place(
                    [side * 0.36, 0.1, -0.56 + back],
                    [0.3, 0.18, 0.36],
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
                        0.85 + t * 0.42 + wave * 0.3
                    },
                    -0.95 - t * 0.3 + wave * 0.5,
                ],
                [0.2 - t * 0.05, 0.2 - t * 0.05, 0.3 - t * 0.04],
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

/// Where the head sits when the kitten is awake, in its own space.
const HEAD: [f32; 3] = [0.0, 1.58, 1.04];
/// The head's size.
const HEAD_SIZE: [f32; 3] = [0.98, 0.92, 0.86];
/// An ear's size.
const EAR_SIZE: [f32; 3] = [0.24, 0.34, 0.3];

#[cfg(test)]
mod tests {
    use super::*;

    fn senses<'a>(rng: &'a mut Rng, pointer: Option<[f32; 3]>, age: f32) -> Senses<'a> {
        Senses {
            pointer,
            pointer_age: 0.2,
            input_age: age,
            rng,
        }
    }

    #[test]
    fn the_kitten_is_two_to_three_characters_high() {
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
        assert!((top - kitten.height()).abs() < 0.35, "{top}");
    }

    #[test]
    fn it_walks_to_where_it_is_going() {
        let mut rng = Rng::new(1);
        let mut kitten = Kitten::new([0.0, 0.0, 0.0]);
        kitten.mood = Mood::Wander;
        kitten.target = [4.0, 0.0, 0.0];
        kitten.timer = 30.0;
        for _ in 0..(120 * 4) {
            kitten.think(1.0 / 120.0, &mut senses(&mut rng, None, 0.0));
        }
        assert!(kitten.pos[0] > 3.0, "{:?}", kitten.pos);
    }

    #[test]
    fn it_never_leaves_the_room() {
        let mut rng = Rng::new(2);
        let mut kitten = Kitten::new([0.0, 0.0, 0.0]);
        for step in 0..(120 * 40) {
            let pointer = Some([10.0 * (step as f32 * 0.01).sin(), 0.0, -10.0]);
            kitten.think(1.0 / 120.0, &mut senses(&mut rng, pointer, 0.0));
            assert!(kitten.pos[0].abs() <= 6.61, "{:?}", kitten.pos);
            assert!(kitten.pos[2] >= -8.01 && kitten.pos[2] <= 2.01, "{:?}", kitten.pos);
        }
    }

    #[test]
    fn it_follows_the_pointer_when_it_moves() {
        let mut rng = Rng::new(3);
        let mut kitten = Kitten::new([0.0, 0.0, 0.0]);
        for _ in 0..(120 * 3) {
            kitten.think(
                1.0 / 120.0,
                &mut senses(&mut rng, Some([5.0, 0.0, -1.0]), 0.0),
            );
        }
        assert!(kitten.pos[0] > 3.0, "{:?}", kitten.pos);
        assert!(matches!(kitten.mood, Mood::Follow | Mood::Bat | Mood::Sit | Mood::Groom));
    }

    #[test]
    fn it_sleeps_when_nothing_happens_and_wakes_when_something_does() {
        let mut rng = Rng::new(4);
        let mut kitten = Kitten::new([0.0, 0.0, 0.0]);
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
        assert!(top < 2.0, "a curled cat is lower: {top}");

        kitten.think(1.0 / 120.0, &mut senses(&mut rng, None, 0.0));
        assert_ne!(kitten.mood, Mood::Sleep);
    }

    #[test]
    fn it_pounces_when_the_pointer_appears_far_away() {
        let mut rng = Rng::new(5);
        let mut kitten = Kitten::new([0.0, 0.0, 0.0]);
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
        let mut kitten = Kitten::new([0.0, 0.0, 0.0]);
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
            pos: [0.0, 0.0, 0.0],
            yaw: 0.0,
            ..Kitten::default()
        };
        let paw = kitten.paw();
        assert!(paw[2] > 1.0, "{paw:?}");
        kitten.yaw = std::f32::consts::FRAC_PI_2;
        let paw = kitten.paw();
        assert!(paw[0] > 1.0, "{paw:?}");
    }

    #[test]
    fn the_same_seed_drives_the_same_kitten() {
        let run = || {
            let mut rng = Rng::new(9);
            let mut kitten = Kitten::new([0.0, 0.0, 0.0]);
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
