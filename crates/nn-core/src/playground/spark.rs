//! The page's light: the rainbow a jump leaves behind it, the confetti of a paw,
//! the ring a landing puts on the paper.
//!
//! None of this is physical. A spark has no mass and pushes nothing: it is a card
//! a cell or two across that lives for under a second and is drawn by adding its
//! colour to whatever is under it. What makes it worth having is that a page of
//! text has no sense of speed -- one line of code looks like the next -- so the
//! answer to a jump has to be drawn: a trail says where the cat has been, a ring
//! says she came down, and confetti says the paw took something.
//!
//! Nothing here is a particle system in the usual sense. There is no emitter and
//! no texture: the simulation pushes sparks, steps them, and forgets them, and the
//! renderer draws whatever is left with one additive shader. A mode whose effects
//! outlive their cause is a mode that looks the same in a still frame as in
//! motion.

use super::Rng;

/// How hard confetti is pulled down, in cells per second squared.
///
/// An eighth of the page's gravity: confetti is paper, not a word, and a spark that
/// fell as fast as a token would land before the reader saw it in the air. Dust is
/// lighter than paper and rises; light is pulled by nothing at all.
const SPARK_GRAVITY: f32 = 3.2;
/// How fast a spark loses its speed sideways, per second.
const SPARK_DRAG: f32 = 2.4;
/// The most sparks a page holds at once.
///
/// A cat in full flight holds a hundred or so between her trail, a ring coming off
/// a landing and the dust she kicked up; the cap is what stops a burst from a page
/// being rebuilt twice in a frame from filling the buffer. It went up with the
/// fireworks: a burst is around ninety sparks and there are twenty-four shells on a
/// fuse, several hundred of them alive at once, which is what a display looks like.
pub const MAX_SPARKS: usize = 960;

/// What a spark is drawn as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlowKind {
    /// A soft ball of light with a hot middle.
    Blob,
    /// A ring on the paper: the wave of a landing.
    Ring,
    /// A star: three rays through a bright middle.
    Star,
}

/// One spark: a colour, a place, and a clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spark {
    /// Centre, in cells.
    pub pos: [f32; 3],
    /// Speed, in cells per second.
    pub vel: [f32; 3],
    /// Where its colour sits on the wheel, 0 to 1. The renderer turns it into a
    /// colour, which is what makes a trail a rainbow rather than a smear.
    pub hue: f32,
    /// Radius, in cells.
    pub size: f32,
    /// How bright it is at its best, before the fade: a halo is not a firework.
    pub base: f32,
    /// How fast it grows, in cells per second: a ring widens, a blob does not.
    pub grow: f32,
    /// How hard the page pulls it down, in cells per second squared: confetti
    /// falls, dust rises, and light is pulled by nothing at all.
    pub pull: f32,
    /// Seconds it has been alive, and seconds it lasts.
    pub age: f32,
    pub life: f32,
    /// Turn in the page's plane, and how fast it turns.
    pub roll: f32,
    pub spin: f32,
    pub kind: GlowKind,
    /// A fixed number per spark, so two stars do not put their rays in the same
    /// place.
    pub seed: f32,
}

impl Spark {
    /// How solid it is: it arrives quickly and leaves over the whole of its life,
    /// squared, because a spark that fades linearly is a spark that is visible for
    /// too long at almost nothing.
    pub fn alpha(&self) -> f32 {
        let in_at = (self.age * 18.0).min(1.0);
        let out = (1.0 - self.age / self.life).clamp(0.0, 1.0);
        in_at * out * out * self.base
    }

    /// What a frame draws, given how much of the page is on screen.
    pub fn glow(&self, fade: f32) -> Glow {
        Glow {
            pos: self.pos,
            size: self.size,
            roll: self.roll,
            hue: self.hue,
            alpha: self.alpha() * fade,
            kind: self.kind,
            seed: self.seed,
        }
    }

    /// One piece of a jump's trail: a soft blob where the cat was, left with `vel`
    /// so it streams out behind her while it swells and dies.
    ///
    /// The velocity is the caller's, because it is the cat's motion negated: light
    /// that hangs where it was dropped would gather into a ball under a cat who
    /// pounces in place, and a ball is not a trail.
    pub fn ribbon(at: [f32; 3], vel: [f32; 3], hue: f32, size: f32) -> Self {
        Self {
            pos: at,
            vel,
            hue,
            size,
            base: 0.9,
            grow: 1.4,
            pull: 0.8,
            age: 0.0,
            life: 0.8,
            roll: 0.0,
            spin: 0.0,
            kind: GlowKind::Blob,
            seed: 0.0,
        }
    }

    /// A star thrown in among a trail: the one piece of it that twinkles, because a
    /// ribbon of blobs is a ribbon and a ribbon of blobs with stars in it is a
    /// ribbon of light.
    pub fn twinkle(at: [f32; 3], hue: f32, size: f32) -> Self {
        Self {
            pos: at,
            vel: [0.0, 0.35, 0.0],
            hue,
            size,
            base: 1.0,
            grow: 0.6,
            pull: 0.5,
            age: 0.0,
            life: 0.45,
            roll: hue * std::f32::consts::TAU,
            spin: 2.6,
            kind: GlowKind::Star,
            // The hue again, folded: two stars of one colour do not put their rays
            // in the same place.
            seed: (hue * 7.0).fract(),
        }
    }

    /// A speck of the page's dust kicked up by a run: dim, small, and rising,
    /// because what a paw lifts off paper is lighter than the paper.
    pub fn dust(at: [f32; 3], hue: f32, size: f32, rng: &mut Rng) -> Self {
        let angle = rng.range(0.0, std::f32::consts::TAU);
        let out = rng.range(0.5, 1.8);
        Self {
            pos: at,
            vel: [angle.cos() * out, rng.range(0.6, 1.8), 0.0],
            hue,
            size,
            base: 0.4,
            grow: 1.1,
            pull: -0.7,
            age: 0.0,
            life: rng.range(0.4, 0.9),
            roll: 0.0,
            spin: 0.0,
            kind: GlowKind::Blob,
            seed: 0.0,
        }
    }

    /// The light that travels with her while she is in the air, so the trail reads
    /// as something attached to the cat rather than left behind by it.
    pub fn aura(at: [f32; 3], hue: f32, size: f32) -> Self {
        Self {
            pos: at,
            vel: [0.0; 3],
            hue,
            size,
            base: 0.12,
            grow: 0.0,
            pull: 0.0,
            age: 0.0,
            life: 0.1,
            roll: 0.0,
            spin: 0.0,
            kind: GlowKind::Blob,
            seed: 0.0,
        }
    }

    /// A ring on the paper, widening as it fades.
    pub fn ring(at: [f32; 3], hue: f32, size: f32) -> Self {
        Self {
            pos: at,
            vel: [0.0; 3],
            hue,
            size,
            base: 0.65,
            grow: 3.4,
            pull: 0.0,
            age: 0.0,
            life: 0.55,
            roll: 0.0,
            spin: 0.0,
            kind: GlowKind::Ring,
            seed: 0.0,
        }
    }

    /// A wave of light leaving a burst: a ring that widens across the page and dies
    /// as it goes.
    ///
    /// Four times a landing's ring and twice as long, because this one is not
    /// answering a paw on the paper -- it is the paper itself changing, and the wave
    /// of that has to reach the far side of it.
    pub fn shockwave(at: [f32; 3], hue: f32, size: f32) -> Self {
        Self {
            pos: at,
            vel: [0.0; 3],
            hue,
            size,
            base: 0.5,
            grow: 26.0,
            pull: 0.0,
            age: 0.0,
            life: 1.4,
            roll: 0.0,
            spin: 0.0,
            kind: GlowKind::Ring,
            seed: 0.0,
        }
    }

    /// A piece of a rocket: the streak a shell leaves behind it as it goes up.
    ///
    /// It rises rather than falls -- what it is following is going up -- and it is
    /// gone in half a second, so what a reader sees is a line of light with a shell
    /// on the end of it rather than a column of light on the page.
    pub fn rocket(at: [f32; 3], vel: [f32; 3], hue: f32, size: f32) -> Self {
        Self {
            pos: at,
            vel,
            hue,
            size,
            base: 0.85,
            grow: 1.2,
            pull: -1.4,
            age: 0.0,
            life: 0.5,
            roll: 0.0,
            spin: 0.0,
            kind: GlowKind::Blob,
            seed: 0.0,
        }
    }

    /// One spark of a burst: thrown hard in every direction, pulled back down under
    /// its own weight, a second and a half of life.
    ///
    /// Twice confetti's speed and twice its life, because a firework has to reach
    /// across the page before it is worth looking at, and a firework of paw-sized
    /// sparks would be a sneeze.
    pub fn firework(at: [f32; 3], hue: f32, speed: f32, rng: &mut Rng) -> Self {
        let angle = rng.range(0.0, std::f32::consts::TAU);
        let out = rng.range(speed * 0.45, speed);
        let star = rng.above() < 0.3;
        Self {
            pos: [
                at[0] + rng.range(-0.3, 0.3),
                at[1] + rng.range(-0.3, 0.3),
                at[2],
            ],
            vel: [angle.cos() * out, angle.sin() * out, rng.range(-0.5, 0.5)],
            hue,
            size: rng.range(0.22, 0.42),
            base: 1.0,
            grow: 0.12,
            pull: SPARK_GRAVITY,
            age: 0.0,
            life: rng.range(0.9, 1.6),
            roll: rng.range(0.0, std::f32::consts::TAU),
            spin: rng.range(-5.0, 5.0),
            kind: if star { GlowKind::Star } else { GlowKind::Blob },
            seed: rng.above() as f32,
        }
    }

    /// One piece of confetti: thrown out and up, pulled down gently, half a second
    /// to a second of life.
    pub fn confetti(at: [f32; 3], hue: f32, speed: f32, rng: &mut Rng) -> Self {
        let angle = rng.range(0.0, std::f32::consts::TAU);
        let out = rng.range(speed * 0.35, speed);
        // Upwards more than sideways: confetti thrown from a paw arcing up off the
        // page is a shape the reader already knows.
        let vel = [
            angle.cos() * out,
            angle.sin() * out * 0.6 + speed * 0.55,
            rng.range(-0.2, 0.2),
        ];
        let star = rng.above() < 0.35;
        Self {
            pos: [at[0] + rng.range(-0.2, 0.2), at[1] + rng.range(-0.15, 0.15), at[2]],
            vel,
            hue,
            size: rng.range(0.22, 0.40),
            base: 1.0,
            grow: 0.0,
            pull: SPARK_GRAVITY,
            age: 0.0,
            life: rng.range(0.5, 1.0),
            roll: rng.range(0.0, std::f32::consts::TAU),
            spin: rng.range(-7.0, 7.0),
            kind: if star { GlowKind::Star } else { GlowKind::Blob },
            seed: rng.above() as f32,
        }
    }

    /// One step of a spark: it moves, it is pulled, it slows, it grows.
    ///
    /// Fixed steps are the caller's job, as everywhere in this mode: a half-second
    /// frame must not throw confetti off the page.
    pub fn step(&mut self, dt: f32) {
        self.age += dt;
        self.pos[0] += self.vel[0] * dt;
        self.pos[1] += self.vel[1] * dt;
        self.vel[1] -= self.pull * dt;
        let keep = (1.0 - SPARK_DRAG * dt).max(0.0);
        self.vel[0] *= keep;
        self.vel[1] *= keep;
        self.size += self.grow * dt;
        self.roll += self.spin * dt;
    }
}

/// One glowing thing, ready to draw: no state left in it but the colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glow {
    /// Centre, in cells.
    pub pos: [f32; 3],
    /// Radius, in cells.
    pub size: f32,
    /// Turn in the page's plane, in radians.
    pub roll: f32,
    /// 0 to 1 around the colour wheel. The renderer reads it as a hue, so a trail
    /// is a rainbow without the simulation knowing what a rainbow is.
    pub hue: f32,
    /// 0 gone, 1 solid.
    pub alpha: f32,
    pub kind: GlowKind,
    /// A fixed number, so two stars do not put their rays in the same place.
    pub seed: f32,
}
