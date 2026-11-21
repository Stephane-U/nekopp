//! Hover tooltips.
//!
//! A small label that appears under the pointer once it has rested on a control
//! for half a second. The toolbar is the only user today.
//!
//! Notepad++ has no toolbar tooltips. Its `TBBUTTON` array sets `iString = 0`
//! for every button, so `TBSTYLE_TOOLTIPS` is set on the bar and never fires.
//! This is therefore an addition rather than a parity item, and it is worth
//! having: the bar is thirty-two unlabelled icons, and the difference between
//! "Close" and "Close All" is a few pixels of drawing.
//!
//! The delay is the reason the widget exists rather than a bare `if hovered`
//! check. A tooltip that appears the instant the pointer crosses a button
//! flickers as the pointer travels along the bar to reach a distant one, and a
//! bar that flickers is worse than a bar with no labels at all. Half a second is
//! what the desktop's own tooltips use, so the bar feels like the rest of the
//! system rather than like a web page.
//!
//! The timer lives here, not in the application, because the widget owns the
//! pair "what is under the pointer, and since when". A pointer that leaves a
//! button and comes back has to restart the clock, and that is a property of the
//! tooltip rather than of the window.

use std::time::{Duration, Instant};

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics};
use glyphon::{TextArea, TextBounds};

use crate::quad::QuadRenderer;
use crate::text::shaping_for;
use crate::theme::Theme;

/// How long the pointer has to rest on a control before its tooltip appears.
pub const DELAY: Duration = Duration::from_millis(500);
/// The gap between the control and the tooltip below it.
const GAP: f32 = 4.0;
/// Padding inside the frame.
const PADDING: f32 = 6.0;
/// How close to the window edge a tooltip is allowed to come.
const MARGIN: f32 = 4.0;
/// The widest a tooltip gets before its label is cut.
///
/// Wider than any toolbar label, so in practice nothing is cut; the cap is here
/// so a future caller with a sentence to show cannot paper over the window.
const MAX_WIDTH: f32 = 320.0;

/// What the pointer is resting on, and since when.
///
/// `PartialEq` but not `Eq`: the anchor is in pixels, and two anchors that
/// differ by a fraction are the same button for the purpose of not restarting
/// the clock.
#[derive(Debug, Clone, PartialEq)]
struct Resting {
    label: String,
    /// The control's left edge and its bottom edge, in window pixels.
    anchor: (f32, f32),
    since: Instant,
}

/// A hover tooltip.
pub struct Tooltip {
    /// The control under the pointer, whether or not its label is showing yet.
    resting: Option<Resting>,
    /// The label being shown, empty when nothing is.
    text: String,
    /// The anchor the shown label hangs from.
    anchor: (f32, f32),
    /// The window size, so a label near the right edge is pushed back inside.
    viewport: (f32, f32),
    open: bool,
    body: Buffer,
    /// What the body was shaped for: (text, columns).
    shaped: (String, usize),
    family: Family<'static>,
    char_width: f32,
}

impl Tooltip {
    /// Builds a closed tooltip.
    pub fn new(metrics: Metrics, family: Family<'static>, char_width: f32) -> Self {
        Self {
            resting: None,
            text: String::new(),
            anchor: (0.0, 0.0),
            viewport: (0.0, 0.0),
            open: false,
            body: Buffer::new_empty(metrics),
            shaped: (String::new(), 0),
            family,
            char_width,
        }
    }

    /// Records the window size, so a label on the rightmost button is pushed
    /// back inside the frame instead of running off it.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.viewport = (width, height);
    }

    /// Records what the pointer is resting on.
    ///
    /// `target` is the control's label and the point the tooltip hangs from: its
    /// left edge and its bottom edge. `None` means the pointer is not on
    /// anything that has a tooltip.
    ///
    /// Returns true when the visible state changed, which is the caller's cue to
    /// repaint. Moving within one control keeps the clock running -- that is the
    /// whole point of the delay -- while moving to another control, or off the
    /// bar entirely, restarts it and hides whatever was showing.
    pub fn rest(&mut self, target: Option<(&str, (f32, f32))>, now: Instant) -> bool {
        let same = match (&self.resting, &target) {
            (Some(resting), Some((label, anchor))) => {
                resting.label == *label && resting.anchor == *anchor
            }
            (None, None) => true,
            _ => false,
        };
        if same {
            return false;
        }
        let was_open = self.open;
        self.open = false;
        self.text.clear();
        self.shaped.1 = 0;
        self.resting = target.map(|(label, anchor)| Resting {
            label: label.to_string(),
            anchor,
            since: now,
        });
        was_open
    }

    /// Opens the tooltip if the pointer has rested long enough.
    ///
    /// Returns true when it opened, which is the caller's cue to repaint.
    pub fn tick(&mut self, now: Instant) -> bool {
        if self.open {
            return false;
        }
        let Some(resting) = &self.resting else {
            return false;
        };
        if now.saturating_duration_since(resting.since) < DELAY {
            return false;
        }
        self.text = resting.label.clone();
        self.anchor = resting.anchor;
        self.open = true;
        self.shaped.1 = 0;
        true
    }

    /// How long until [`tick`](Self::tick) would open something.
    ///
    /// The event loop sleeps when nothing is pending, so a tooltip waiting out
    /// its delay has to be able to say when it wants waking. Without this the
    /// label would appear on the next unrelated event, which on an idle window
    /// is never.
    pub fn deadline(&self, now: Instant) -> Option<Duration> {
        if self.open {
            return None;
        }
        let resting = self.resting.as_ref()?;
        Some(DELAY.saturating_sub(now.saturating_duration_since(resting.since)))
    }

    /// Hides the tooltip and forgets what was under the pointer.
    ///
    /// Called when the pointer leaves the window, when a button is pressed, and
    /// when a key is typed: in all three cases the user has moved on, and a
    /// label that outlives the gesture is a label in the way.
    pub fn close(&mut self) {
        self.open = false;
        self.resting = None;
        self.text.clear();
        self.shaped.1 = 0;
    }

    /// Whether a label is showing.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The label being shown.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The width the tooltip occupies.
    pub fn width(&self) -> f32 {
        let columns = self.text.chars().count() as f32;
        (columns * self.char_width + PADDING * 2.0).min(MAX_WIDTH)
    }

    /// The height the tooltip occupies.
    pub fn height(&self, line_height: f32) -> f32 {
        if !self.open {
            return 0.0;
        }
        line_height + PADDING * 2.0
    }

    /// The top-left corner actually drawn at.
    ///
    /// Below the control, because the toolbar is at the top of the window and
    /// below it is always free. Pushed left when the label would run off the
    /// right edge, which is what happens on the last group of buttons.
    fn origin(&self) -> (f32, f32) {
        let (left, bottom) = self.anchor;
        // Before the first frame the window size is unknown, and clamping
        // against a zero width would pin every label to the left edge.
        if self.viewport.0 <= 0.0 {
            return (left, bottom + GAP);
        }
        let max_left = (self.viewport.0 - self.width() - MARGIN).max(MARGIN);
        (left.min(max_left).max(MARGIN), bottom + GAP)
    }

    /// Rebuilds the shaped line if the label or the width changed.
    pub fn prepare(&mut self, font_system: &mut FontSystem, theme: &Theme) {
        if !self.open {
            return;
        }
        // Rounded, not truncated: `width()` is `columns * char_width + padding`,
        // so dividing it back out lands a hair under the column count in f32 and
        // a cast would drop a column -- which shows up as an ellipsis on a label
        // that fits.
        let columns = ((self.width() - PADDING * 2.0) / self.char_width)
            .max(4.0)
            .round() as usize;
        let key = (self.text.clone(), columns);
        if key == self.shaped {
            return;
        }
        self.shaped = key;

        let shown = crate::complete::truncate(&self.text, columns);
        let attrs = Attrs::new()
            .family(self.family)
            .color(theme.tooltip_text.to_cosmic());
        self.body.set_text(&shown, &attrs, shaping_for(&shown), None);
        self.body.shape_until_scroll(font_system, false);
    }

    /// Queues the frame.
    pub fn push_quads(&self, quads: &mut QuadRenderer, line_height: f32, theme: &Theme) {
        if !self.open {
            return;
        }
        let (left, top) = self.origin();
        let width = self.width();
        let height = self.height(line_height);
        quads.push(
            left,
            top,
            width,
            height,
            theme.tooltip_background.to_linear(),
        );
        let border = theme.menu_border.to_linear();
        quads.push(left, top, width, 1.0, border);
        quads.push(left, top + height - 1.0, width, 1.0, border);
        quads.push(left, top, 1.0, height, border);
        quads.push(left + width - 1.0, top, 1.0, height, border);
    }

    /// The text area of the label.
    pub fn text_areas<'a>(&'a self, line_height: f32, theme: &Theme) -> Vec<TextArea<'a>> {
        if !self.open {
            return Vec::new();
        }
        let (left, top) = self.origin();
        let bounds = TextBounds {
            left: left as i32,
            top: top as i32,
            right: (left + self.width()) as i32,
            bottom: (top + self.height(line_height)) as i32,
        };
        vec![TextArea {
            buffer: &self.body,
            left: left + PADDING,
            top: top + PADDING,
            scale: 1.0,
            bounds,
            default_color: theme.tooltip_text.to_cosmic(),
            custom_glyphs: &[],
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tip() -> Tooltip {
        Tooltip::new(Metrics::new(15.0, 20.0), Family::Monospace, 8.0)
    }

    /// A fixed instant, so the tests do not depend on the wall clock.
    fn at() -> Instant {
        Instant::now()
    }

    #[test]
    fn a_fresh_tooltip_shows_nothing() {
        let tip = tip();
        assert!(!tip.is_open());
        assert_eq!(tip.height(20.0), 0.0);
        assert!(tip.text_areas(20.0, &Theme::DARK).is_empty());
    }

    #[test]
    fn resting_does_not_show_the_label_straight_away() {
        let mut tip = tip();
        let now = at();
        assert!(!tip.rest(Some(("Save", (10.0, 30.0))), now));
        assert!(!tip.is_open(), "the delay has not passed yet");
        assert!(!tip.tick(now + DELAY / 2));
        assert!(!tip.is_open());
    }

    #[test]
    fn the_label_appears_once_the_delay_has_passed() {
        let mut tip = tip();
        let now = at();
        tip.rest(Some(("Save", (10.0, 30.0))), now);
        assert!(tip.tick(now + DELAY));
        assert!(tip.is_open());
        assert_eq!(tip.text(), "Save");
    }

    #[test]
    fn moving_within_one_button_keeps_the_clock_running() {
        // The pointer jitters by a pixel while the user holds still; restarting
        // the delay on every move would mean the label never appears.
        let mut tip = tip();
        let now = at();
        tip.rest(Some(("Save", (10.0, 30.0))), now);
        assert!(!tip.rest(Some(("Save", (10.0, 30.0))), now + DELAY / 2));
        assert!(tip.tick(now + DELAY));
    }

    #[test]
    fn moving_to_another_button_restarts_the_clock() {
        let mut tip = tip();
        let now = at();
        tip.rest(Some(("Save", (10.0, 30.0))), now);
        tip.rest(Some(("Print", (40.0, 30.0))), now + DELAY / 2);
        assert!(!tip.tick(now + DELAY), "Print has only been hovered half as long");
        assert!(tip.tick(now + DELAY + DELAY / 2));
        assert_eq!(tip.text(), "Print");
    }

    #[test]
    fn moving_off_the_bar_hides_a_label_that_was_showing() {
        let mut tip = tip();
        let now = at();
        tip.rest(Some(("Save", (10.0, 30.0))), now);
        tip.tick(now + DELAY);
        assert!(tip.is_open());
        assert!(tip.rest(None, now + DELAY), "closing is a visible change");
        assert!(!tip.is_open());
        assert_eq!(tip.text(), "");
    }

    #[test]
    fn a_deadline_is_offered_only_while_a_label_is_pending() {
        let mut tip = tip();
        let now = at();
        assert_eq!(tip.deadline(now), None, "nothing is being hovered");
        tip.rest(Some(("Save", (10.0, 30.0))), now);
        assert_eq!(tip.deadline(now), Some(DELAY));
        assert_eq!(tip.deadline(now + DELAY / 2), Some(DELAY / 2));
        tip.tick(now + DELAY);
        assert_eq!(tip.deadline(now + DELAY), None, "already showing");
    }

    #[test]
    fn a_deadline_that_has_already_passed_is_zero_rather_than_negative() {
        let mut tip = tip();
        let now = at();
        tip.rest(Some(("Save", (10.0, 30.0))), now);
        assert_eq!(tip.deadline(now + DELAY * 4), Some(Duration::ZERO));
    }

    #[test]
    fn the_label_hangs_below_the_button() {
        let mut tip = tip();
        let now = at();
        tip.rest(Some(("Save", (10.0, 30.0))), now);
        tip.tick(now + DELAY);
        let (left, top) = tip.origin();
        assert_eq!(left, 10.0);
        assert_eq!(top, 30.0 + GAP);
    }

    #[test]
    fn a_label_on_the_rightmost_button_is_pushed_back_inside() {
        let mut tip = tip();
        tip.set_viewport(400.0, 300.0);
        let now = at();
        tip.rest(Some(("Save Current Macro", (380.0, 30.0))), now);
        tip.tick(now + DELAY);
        let (left, _) = tip.origin();
        assert!(
            left + tip.width() <= 400.0 - MARGIN,
            "the label runs off the right edge"
        );
    }

    #[test]
    fn the_width_grows_with_the_label_and_stops_at_the_cap() {
        let mut tip = tip();
        let now = at();
        tip.rest(Some(("Save", (0.0, 0.0))), now);
        tip.tick(now + DELAY);
        let narrow = tip.width();
        tip.close();
        tip.rest(Some(("Save Current Macro", (0.0, 0.0))), now);
        tip.tick(now + DELAY);
        assert!(tip.width() > narrow);
        tip.close();
        let long = "x".repeat(400);
        tip.rest(Some((&long, (0.0, 0.0))), now);
        tip.tick(now + DELAY);
        assert_eq!(tip.width(), MAX_WIDTH);
    }

    #[test]
    fn the_height_is_a_line_plus_padding() {
        let mut tip = tip();
        let now = at();
        tip.rest(Some(("Save", (0.0, 0.0))), now);
        tip.tick(now + DELAY);
        assert_eq!(tip.height(20.0), 20.0 + PADDING * 2.0);
    }

    #[test]
    fn closing_forgets_the_pending_label() {
        let mut tip = tip();
        let now = at();
        tip.rest(Some(("Save", (0.0, 0.0))), now);
        tip.close();
        assert!(!tip.tick(now + DELAY), "a closed tooltip has nothing pending");
        assert_eq!(tip.deadline(now), None);
    }
}
