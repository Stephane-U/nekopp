//! Playground mode: the window becomes a room.
//!
//! `Ctrl+Alt+K` hides the editor, the menu and every bar, and puts a room on the
//! screen: code on the walls, a rug of punctuation, a few pieces of furniture, and
//! a kitten two and a half characters tall who walks about, follows the pointer,
//! bats loose tokens around and eventually falls asleep. `Escape` gives the
//! window back. The document is never read and never touched -- the material is a
//! corpus compiled into the binary -- so a screenshot of the mode cannot show
//! anyone's work.
//!
//! The simulation is in [`nn_core::playground`]: rooms, tokens, the kitten's
//! moods and the camera, all in character units and all testable without a GPU.
//! This module is the app side of it: when the mode is open, how long the pointer
//! has been still, the glyph atlas, and the frame the renderer draws.
//!
//! Two clocks matter and they are different. The mode's own clock decides when
//! the room is rebuilt -- five minutes -- and when the kitten falls asleep, and it
//! lives with the simulation because it is part of the rules. The idle clock here
//! is about the person: it decides when the mode may open by itself, which is a
//! property of the window rather than of the room.

mod glyphs;
mod mesh;
mod render;
mod xform;

use std::time::{Duration, Instant};

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use glyphon::{TextArea, TextBounds};
use nn_core::playground::{Frame, Input, Shade};
use nn_core::playground::Playground as Room;

use crate::theme::{mix, Rgb, Theme};

pub use nn_core::playground::scene::GLYPH_WIDTH;

/// The height of the glyph atlas cells, in pixels.
const ATLAS_PX: u32 = 96;
/// How tall the hint line is drawn.
const HINT_SIZE: f32 = 13.0;
/// How long a frame lasts while the mode is open.
///
/// Sixty a second. The simulation is a dozen microseconds of arithmetic and the
/// scene is three draw calls, so the ceiling here is the display's, not ours.
pub const FRAME: Duration = Duration::from_millis(16);
/// The gap between the hint and the window's edge.
const HINT_INSET: f32 = 14.0;

/// The kitten's colours.
///
/// Fixed rather than taken from the theme: a cat is ginger whatever the syntax
/// highlighting is doing. They are adjusted for the background -- see
/// [`readable`] -- so they stay visible on a light theme.
const FUR: Rgb = Rgb(214, 158, 96);
const FUR_DARK: Rgb = Rgb(174, 120, 66);
const FUR_LIGHT: Rgb = Rgb(246, 236, 214);
const NOSE: Rgb = Rgb(226, 136, 148);
const EYE: Rgb = Rgb(40, 36, 46);
const WOOD: Rgb = Rgb(152, 112, 76);
const METAL: Rgb = Rgb(124, 132, 144);
const LEAF: Rgb = Rgb(110, 168, 98);
const POT: Rgb = Rgb(178, 116, 90);
const GLOW: Rgb = Rgb(255, 228, 156);

/// A colour moved away from the window's background until it reads.
///
/// A ginger kitten on a paper-white theme is a pale smudge, and a cream chest on
/// a near-black one is a bright smear. One rule covers both: darken on a light
/// background, lighten on a dark one, by enough to see and not enough to look
/// like a different colour.
pub fn readable(colour: Rgb, theme: &Theme) -> Rgb {
    let background = theme.background;
    let luma = 0.299 * background.0 as f32
        + 0.587 * background.1 as f32
        + 0.114 * background.2 as f32;
    if luma > 140.0 {
        mix(colour, Rgb(0, 0, 0), 0.24)
    } else {
        mix(colour, Rgb(255, 255, 255), 0.06)
    }
}

/// The colour a [`Shade`] plays.
pub fn shade_colour(shade: Shade, theme: &Theme) -> Rgb {
    readable(
        match shade {
            Shade::Fur => FUR,
            Shade::FurDark => FUR_DARK,
            Shade::FurLight => FUR_LIGHT,
            Shade::Nose => NOSE,
            Shade::Eye => EYE,
            Shade::Wood => WOOD,
            Shade::Metal => METAL,
            Shade::Leaf => LEAF,
            Shade::Pot => POT,
            Shade::Glow => GLOW,
        },
        theme,
    )
}

/// The mode: a room, the GPU side of it, and how long the pointer has been still.
pub struct Playground {
    room: Room,
    scene: render::Scene,
    /// The corner label, shaped once per room.
    hint: Buffer,
    hint_text: String,
    active: bool,
    /// The pointer, in `-1..1` on both axes, y up, while it is in the window.
    pointer: Option<[f32; 2]>,
    /// Seconds since the pointer last moved.
    pointer_age: f32,
    /// Seconds since any key, button or wheel.
    input_age: f32,
    /// When the last input arrived, for the idle clock.
    last_input: Instant,
    /// The window's aspect ratio, from the last frame.
    aspect: f32,
    /// When the next frame of the room is due.
    next: Instant,
    /// When the last one was drawn, so the step is the time that passed.
    last_frame: Instant,
}

impl Playground {
    /// Builds the mode, closed, with the atlas rasterised now.
    ///
    /// The atlas costs a few milliseconds and a frame inside the mode costs
    /// nothing, so it is built at startup rather than the first time the shortcut
    /// is pressed: a mode whose whole purpose is to be a rest should not stall the
    /// window on the way in.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        font_system: &mut FontSystem,
        seed: u64,
        now: Instant,
    ) -> Self {
        let atlas = glyphs::GlyphAtlas::build(font_system, ATLAS_PX);
        Self {
            room: Room::new(seed),
            scene: render::Scene::new(device, queue, format, &atlas),
            hint: Buffer::new_empty(Metrics::new(HINT_SIZE, HINT_SIZE * 1.35)),
            hint_text: String::new(),
            active: false,
            pointer: None,
            pointer_age: f32::MAX,
            input_age: 0.0,
            last_input: now,
            aspect: 1.6,
            next: now,
            last_frame: now,
        }
    }

    /// Whether the room is on screen instead of the editor.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Opens the room.
    ///
    /// A fresh seed each time, so two visits are two different rooms and the
    /// kitten is awake.
    pub fn enter(&mut self, seed: u64, now: Instant) {
        self.room = Room::new(seed);
        self.active = true;
        self.input_age = 0.0;
        self.pointer_age = f32::MAX;
        self.pointer = None;
        self.last_input = now;
        self.next = now;
        self.last_frame = now;
    }

    /// Closes the room.
    pub fn leave(&mut self) {
        self.active = false;
    }

    /// The pointer moved, or left the window.
    pub fn pointer(&mut self, ndc: Option<[f32; 2]>, now: Instant) {
        if self.pointer != ndc {
            self.pointer = ndc;
            self.pointer_age = 0.0;
        }
        self.last_input = now;
    }

    /// A key, a button or a wheel: anything that means someone is here.
    pub fn activity(&mut self, now: Instant) {
        self.input_age = 0.0;
        self.last_input = now;
    }

    /// How long nothing has happened, which is the screensaver's question.
    pub fn idle(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.last_input)
    }

    /// How long until the next frame of the room is due.
    ///
    /// `None` while the mode is closed, which is what lets the event loop sleep
    /// through an idle window: a closed playground is not a timer.
    pub fn deadline(&self, now: Instant) -> Option<Duration> {
        if !self.active {
            return None;
        }
        Some(self.next.saturating_duration_since(now))
    }

    /// Runs one frame if one is due, and says whether the window has to repaint.
    ///
    /// The frame rate is decided here rather than by asking for a redraw on
    /// every pass through the event loop: a redraw request made from
    /// `about_to_wait` is answered immediately, so a loop that always asks for
    /// one spins at whatever the display allows and burns a core on a kitten
    /// that is asleep.
    pub fn tick(&mut self, now: Instant) -> bool {
        if !self.active || now < self.next {
            return false;
        }
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.next = now + FRAME;
        self.step(dt.min(0.25), self.aspect);
        true
    }

    /// Advances the clocks, and the room if it is open.
    ///
    /// Returns true when the room moved, which is every step while the mode is
    /// open and no step while it is closed.
    pub fn step(&mut self, dt: f32, aspect: f32) -> bool {
        if !self.active {
            return false;
        }
        self.aspect = aspect.max(0.2);
        self.input_age += dt;
        self.pointer_age = (self.pointer_age + dt).min(f32::MAX);
        let input = Input {
            pointer: self.pointer,
            pointer_age: self.pointer_age,
            input_age: self.input_age,
            aspect: self.aspect,
        };
        self.room.step(dt, &input);
        true
    }

    /// Rebuilds the frame's instances and the hint line.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        font_system: &mut FontSystem,
        theme: &Theme,
        width: u32,
        height: u32,
    ) {
        // The frame is built with the aspect the window has now, not the one it
        // had when the last step ran: a window resized while the room is open
        // must not be stretched for a frame.
        self.aspect = width as f32 / height.max(1) as f32;
        let input = Input {
            pointer: self.pointer,
            pointer_age: self.pointer_age,
            input_age: self.input_age,
            aspect: self.aspect,
        };
        let frame = self.room.frame(&input);
        self.scene
            .prepare(device, queue, &frame, theme, width, height);
        self.set_hint(font_system, theme, &frame);
    }

    /// Shapes the corner label, if it changed.
    fn set_hint(&mut self, font_system: &mut FontSystem, theme: &Theme, frame: &Frame) {
        let text = if frame.asleep {
            format!("{} · the kitten is asleep · Esc leaves", frame.label)
        } else {
            format!("{} · Esc leaves", frame.label)
        };
        if text == self.hint_text {
            return;
        }
        self.hint_text = text;
        let attrs = Attrs::new()
            .family(Family::Monospace)
            .color(theme.foreground.to_cosmic());
        self.hint
            .set_text(&self.hint_text, &attrs, Shaping::Basic, None);
        self.hint.shape_until_scroll(font_system, false);
    }

    /// The depth view the pass attaches.
    pub fn depth_view(&self) -> &wgpu::TextureView {
        self.scene.depth_view()
    }

    /// Draws the room.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.scene.draw(pass);
    }

    /// The corner label, at the bottom left.
    pub fn text_areas<'a>(&'a self, theme: &Theme, height: f32) -> Vec<TextArea<'a>> {
        if !self.active {
            return Vec::new();
        }
        let top = height - HINT_SIZE * 1.35 - HINT_INSET;
        vec![TextArea {
            buffer: &self.hint,
            left: HINT_INSET,
            top,
            scale: 1.0,
            bounds: TextBounds {
                left: HINT_INSET as i32,
                top: top as i32,
                right: (HINT_INSET + self.hint_text.chars().count() as f32 * HINT_SIZE * 0.62) as i32,
                bottom: (top + HINT_SIZE * 1.35) as i32,
            },
            default_color: theme.foreground.to_cosmic(),
            custom_glyphs: &[],
        }]
    }

    /// The room's own clock, for the window's next wake.
    pub fn time(&self) -> f32 {
        self.room.time()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ginger_cat_is_darkened_on_a_light_theme_and_left_alone_on_a_dark_one() {
        let paper = Theme {
            background: Rgb(255, 255, 255),
            ..Theme::default()
        };
        let night = Theme {
            background: Rgb(18, 18, 22),
            ..Theme::default()
        };
        let on_paper = readable(FUR, &paper);
        let on_night = readable(FUR, &night);
        assert!(on_paper.0 < FUR.0, "{on_paper:?}");
        assert!(on_night.0 >= FUR.0, "{on_night:?}");
        assert!(on_paper.1 < on_night.1, "{on_paper:?} {on_night:?}");
    }

    #[test]
    fn every_shade_has_a_colour_and_no_two_are_the_same() {
        let theme = Theme::default();
        let shades = [
            Shade::Fur,
            Shade::FurDark,
            Shade::FurLight,
            Shade::Nose,
            Shade::Eye,
            Shade::Wood,
            Shade::Metal,
            Shade::Leaf,
            Shade::Pot,
            Shade::Glow,
        ];
        let colours: Vec<Rgb> = shades
            .iter()
            .map(|shade| shade_colour(*shade, &theme))
            .collect();
        for (index, colour) in colours.iter().enumerate() {
            assert!(
                !colours[index + 1..].contains(colour),
                "{:?} is the colour of another shade",
                shades[index]
            );
        }
    }
}
