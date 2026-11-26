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

//! Playground mode: the window becomes a page.
//!
//! `Ctrl+Alt+K` hides the editor, the menu and every bar, and puts a page of code
//! on the screen at the editor's own line height: a line-number gutter, furniture
//! drawn in characters, loose punctuation lying about, and a kitten two and a
//! quarter lines tall who walks across the page in profile, follows the pointer,
//! bats tokens around and eventually falls asleep. `Escape` gives the window back.
//! The document is never read and never touched -- the material is a corpus
//! compiled into the binary -- so a screenshot of the mode cannot show anyone's
//! work.
//!
//! The simulation is in [`nn_core::playground`]: the page, its tokens, the
//! kitten's moods and the camera, all in cells and all testable without a GPU.
//! This module is the app side of it: when the mode is open, how long the pointer
//! has been still, how many cells the window holds, the glyph atlas, and the frame
//! the renderer draws.
//!
//! Two clocks matter and they are different. The mode's own clock decides when
//! the page is rebuilt -- five minutes -- and when the kitten falls asleep, and it
//! lives with the simulation because it is part of the rules. The idle clock here
//! is about the person: it decides when the mode may open by itself, which is a
//! property of the window rather than of the page.

mod glyphs;
mod mesh;
mod render;
mod xform;

use std::time::{Duration, Instant};

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use glyphon::{TextArea, TextBounds};
use nn_core::playground::{Frame, Input, Shade};
use nn_core::playground::Playground as Simulation;

use crate::theme::{mix, Rgb, Theme, ToGpu};

pub use nn_core::playground::scene::GLYPH_WIDTH;

/// The height of the glyph atlas cells, in pixels.
///
/// Twice the height a cell is drawn at, which is what keeps a glyph crisp when the
/// window's scale is not a round number and leaves room for a display with more
/// pixels than we were asked for.
const ATLAS_PX: u32 = 40;
/// The height of one line of the page, in pixels, at a window scale of one.
const LINE_PX: f32 = 20.0;
/// How tall the hint line is drawn, at a window scale of one.
const HINT_SIZE: f32 = 13.0;
/// How long a frame lasts while the mode is open.
///
/// Sixty a second. The simulation is a dozen microseconds of arithmetic and the
/// scene is three draw calls, so the ceiling here is the display's, not ours.
pub const FRAME: Duration = Duration::from_millis(16);
/// The gap between the hint and the window's edge.
const HINT_INSET: f32 = 14.0;

/// The height of one line of the page, in pixels, at `scale`.
///
/// The page is the editor's page: a line of code is the height it would be in the
/// document, so a window drawn at twice the density gets a page at twice the size
/// rather than a small one in a large window.
fn line_px(scale: f32) -> f32 {
    LINE_PX * scale
}

/// The height the atlas cells are rasterised at, in pixels.
///
/// Twice the line the glyphs are drawn on, which is what keeps them crisp when
/// the scale is not a whole number, and it follows the scale for the same reason.
fn atlas_px(scale: f32) -> u32 {
    (ATLAS_PX as f32 * scale).round() as u32
}

/// The hint's metrics at `scale`.
fn hint_metrics(scale: f32) -> Metrics {
    Metrics::new(HINT_SIZE * scale, HINT_SIZE * 1.35 * scale)
}

/// `scale`, or one when it is zero, negative or not a number.
fn sane(scale: f32) -> f32 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

/// How many cells a window of `width` by `height` holds at `line_px` a line.
///
/// The page is laid out to the line height rather than to a fixed grid, so a big
/// window holds more lines of code than a small one, exactly as the editor does.
fn cells(width: u32, height: u32, line_px: f32) -> [u32; 2] {
    let height = height.max(1) as f32;
    let rows = (height / line_px).round().max(8.0) as u32;
    let columns = (width as f32 / (line_px * GLYPH_WIDTH)).floor().max(24.0) as u32;
    [columns, rows]
}

/// The kitten's colours.
///
/// Fixed rather than taken from the theme: a cat is ginger whatever the syntax
/// highlighting is doing. They are adjusted for the background -- see
/// [`readable`] -- so they stay visible on a light theme.
const FUR: Rgb = Rgb(228, 172, 112);
const FUR_DARK: Rgb = Rgb(190, 134, 78);
const FUR_LIGHT: Rgb = Rgb(246, 236, 214);
const NOSE: Rgb = Rgb(226, 136, 148);
const EYE: Rgb = Rgb(40, 36, 46);

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
        },
        theme,
    )
}

/// The mode: a page, the GPU side of it, and how long the pointer has been still.
pub struct Playground {
    sim: Simulation,
    scene: render::Scene,
    /// The corner label, shaped once per page.
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
    /// How much larger than the plain page this one is drawn: the window's
    /// furniture scale, so the mode and the editor agree about how big a line is.
    scale: f32,
    /// The window as a grid of cells, from the last frame or step.
    grid: [u32; 2],
    /// When the next frame of the page is due.
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
        scale: f32,
    ) -> Self {
        let scale = sane(scale);
        let atlas = glyphs::GlyphAtlas::build(font_system, atlas_px(scale));
        Self {
            sim: Simulation::new(seed),
            scene: render::Scene::new(device, queue, format, &atlas),
            hint: Buffer::new_empty(hint_metrics(scale)),
            hint_text: String::new(),
            active: false,
            pointer: None,
            pointer_age: f32::MAX,
            input_age: 0.0,
            last_input: now,
            scale,
            grid: [120, 40],
            next: now,
            last_frame: now,
        }
    }

    /// Whether the page is on screen instead of the editor.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Opens the page.
    ///
    /// A fresh seed each time, so two visits are two different pages and the
    /// kitten is awake.
    pub fn enter(&mut self, seed: u64, now: Instant) {
        self.sim = Simulation::new(seed);
        self.active = true;
        self.input_age = 0.0;
        self.pointer_age = f32::MAX;
        self.pointer = None;
        self.last_input = now;
        self.next = now;
        self.last_frame = now;
    }

    /// Closes the page.
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

    /// How long until the next frame of the page is due.
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
        self.step(dt.min(0.25));
        true
    }

    /// Tells the mode how many pixels the window has, and so how many cells.
    ///
    /// The page is laid out to the editor's line height rather than to a fixed
    /// grid, so a big window holds more lines of code than a small one, exactly as
    /// the editor does.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.grid = cells(width, height, line_px(self.scale));
    }

    /// How much larger than the plain page this one is drawn.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Draws the page, its glyphs and its label at a new window scale.
    ///
    /// The atlas is rasterised at the size the cells are drawn at, so it has to be
    /// built again: keeping it would draw forty-pixel glyphs on an eighty-pixel
    /// page, which is the mode looking half its age on the display that asked for
    /// the larger window. The grid follows on the next frame, which is the one
    /// that resizes against the line height measured here.
    pub fn set_scale(
        &mut self,
        scale: f32,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        font_system: &mut FontSystem,
    ) {
        let scale = sane(scale);
        if (scale - self.scale).abs() < f32::EPSILON {
            return;
        }
        self.scale = scale;
        self.hint.set_metrics(hint_metrics(scale));
        // A label is shaped once per text, so it has to look like it changed.
        self.hint_text.clear();
        let atlas = glyphs::GlyphAtlas::build(font_system, atlas_px(scale));
        self.scene.set_atlas(device, queue, &atlas);
    }

    /// Advances the clocks, and the page if it is open.
    ///
    /// Returns true when the page moved, which is every step while the mode is
    /// open and no step while it is closed.
    pub fn step(&mut self, dt: f32) -> bool {
        if !self.active {
            return false;
        }
        self.input_age += dt;
        self.pointer_age = (self.pointer_age + dt).min(f32::MAX);
        let input = Input {
            pointer: self.pointer,
            pointer_age: self.pointer_age,
            input_age: self.input_age,
            grid: self.grid,
        };
        self.sim.step(dt, &input);
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
        // The frame is built for the grid the window has now, not the one it had
        // when the last step ran: a window resized while the page is open must not
        // be stretched for a frame.
        self.resize(width, height);
        let input = Input {
            pointer: self.pointer,
            pointer_age: self.pointer_age,
            input_age: self.input_age,
            grid: self.grid,
        };
        let frame = self.sim.frame(&input);
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

    /// Draws the page.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.scene.draw(pass);
    }

    /// The corner label, at the bottom left.
    pub fn text_areas<'a>(&'a self, theme: &Theme, height: f32) -> Vec<TextArea<'a>> {
        if !self.active {
            return Vec::new();
        }
        // The label is drawn at the page's scale too, and its gap to the window's
        // edge with it, so a page at twice the size does not carry a stamp meant
        // for a small window in its corner.
        let size = HINT_SIZE * self.scale;
        let line = size * 1.35;
        let inset = HINT_INSET * self.scale;
        let top = height - line - inset;
        vec![TextArea {
            buffer: &self.hint,
            left: inset,
            top,
            scale: 1.0,
            bounds: TextBounds {
                left: inset as i32,
                top: top as i32,
                right: (inset + self.hint_text.chars().count() as f32 * size * 0.62) as i32,
                bottom: (top + line) as i32,
            },
            default_color: theme.foreground.to_cosmic(),
            custom_glyphs: &[],
        }]
    }

    /// The page's own clock, for the window's next wake.
    pub fn time(&self) -> f32 {
        self.sim.time()
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
    fn the_page_holds_half_the_cells_at_twice_the_line_height() {
        // A dense display gets a page at twice the size, which is half as many
        // lines and half as many columns in the same window. The grid is what the
        // simulation walks around in, so it has to follow or the kitten is drawn on
        // a page that is not the one behind her.
        let window = (1280, 800);
        let sparse = cells(window.0, window.1, line_px(1.0));
        let dense = cells(window.0, window.1, line_px(2.0));
        assert_eq!(sparse, [1280 / 11, 40]);
        assert_eq!(dense, [sparse[0] / 2, sparse[1] / 2]);
        // The floor holds: a window smaller than the page still has a page.
        assert_eq!(cells(1, 1, line_px(2.0)), [24, 8]);
    }

    #[test]
    fn the_glyphs_and_the_label_are_rasterised_at_the_scale_they_draw_at() {
        // Twice the window is twice the cell, and twice the atlas that fills it;
        // the label and its gap from the edge go the same way.
        assert_eq!(atlas_px(1.0), 40);
        assert_eq!(atlas_px(2.0), 80);
        assert_eq!(atlas_px(1.5), 60);
        assert_eq!(hint_metrics(2.0).font_size, 26.0);
        assert_eq!(hint_metrics(2.0).line_height, HINT_SIZE * 1.35 * 2.0);
        assert_eq!(line_px(1.75), 35.0);
    }

    #[test]
    fn a_nonsense_scale_draws_the_plain_page() {
        assert_eq!(sane(0.0), 1.0);
        assert_eq!(sane(-2.0), 1.0);
        assert_eq!(sane(f32::NAN), 1.0);
        assert_eq!(sane(1.5), 1.5);
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
