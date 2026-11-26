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

//! The About box.
//!
//! Notepad++ puts its About text in a Win32 dialog. There is no dialog toolkit
//! here, so this is a centred box drawn in the overlay pass, the same way the
//! document peeker and the context menu are drawn -- the difference being that
//! this one has a button rather than rows to choose from.
//!
//! The content is a plain list of lines rather than a layout of labels and
//! values. An About box is read top to bottom and never interacted with, so the
//! only structure worth encoding is the order, and a `Vec<String>` says that
//! more directly than a tree of widgets would.
//!
//! The box also carries the notice the licence asks an interactive program to
//! show. A window has no terminal to print a copyright and a warranty
//! disclaimer in, so this is where they are read, and the Help menu's licence
//! row is where the terms themselves are one click away.
//!
//! The one exception is the LinkedIn address, which is a link: it is drawn in
//! the theme's link colour with a rule under it, and a click on it opens the
//! browser. The address is a constant rather than a field of the box because
//! there is exactly one, and a box that could hold any number of links would
//! need a layout to go with them.
//!
//! The version comes from `CARGO_PKG_VERSION` rather than from a constant, so it
//! cannot drift from the version the binary reports on the command line.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics};
use glyphon::{TextArea, TextBounds};

use crate::i18n::Catalog;
use crate::quad::QuadRenderer;
use crate::text::shaping_for;
use crate::theme::{Rgb, Theme, ToGpu};

/// Padding inside the frame, on all four sides.
const PADDING: f32 = 14.0;
/// The gap between the last line of text and the button.
const BUTTON_GAP: f32 = 14.0;
/// The button's size, in pixels.
const BUTTON_WIDTH: f32 = 88.0;
const BUTTON_HEIGHT: f32 = 26.0;
/// The narrowest the box gets, so a short About text still looks like a box.
const MIN_WIDTH: f32 = 380.0;
/// The label on the button.
const BUTTON_LABEL: &str = "Close";
/// The copyright line, which names the holder rather than the author: the two
/// are the same person here, and the licence speaks of a holder.
const COPYRIGHT: &str = "Copyright (C) 2026 Stéphane Urdy";
/// The licence, in the words the About box needs: version, not "or later",
/// because that is what `Cargo.toml` and every file's header say.
const LICENCE: &str = "GNU General Public License, version 3";
/// What the link line says before the address.
const LINK_PREFIX: &str = "LinkedIn: ";
/// The address the link opens.
///
/// A constant rather than a field of the box: the box has one link, and the
/// click handler needs the address without holding a borrow on the box.
const LINK_URL: &str = "https://fr.linkedin.com/in/stephane-urdy-05b2729";

/// The address the About box's link opens.
pub fn link_url() -> &'static str {
    LINK_URL
}

/// The lines the box shows, in order.
///
/// A blank line is a paragraph break. The author and the LinkedIn address are
/// separate lines rather than one, because the address is long enough that
/// joining them would push the box past the width of a small window.
pub fn lines() -> Vec<String> {
    vec![
        format!("Neko++ {}", env!("CARGO_PKG_VERSION")),
        String::new(),
        "A GPU-rendered text editor".to_string(),
        String::new(),
        COPYRIGHT.to_string(),
        LICENCE.to_string(),
        String::new(),
        "This program comes with ABSOLUTELY NO WARRANTY.".to_string(),
        "Free software: you are welcome to redistribute it".to_string(),
        "under certain conditions; Help > Neko++ License has the terms.".to_string(),
        String::new(),
        format!("{LINK_PREFIX}{LINK_URL}"),
    ]
}

/// The centred About box.
pub struct AboutBox {
    /// Window size, so the box can be centred.
    viewport: (f32, f32),
    open: bool,
    lines: Vec<String>,
    body: Buffer,
    button: Buffer,
    /// The colours the buffers were shaped with, or `None` before they were.
    ///
    /// The colours are part of the key rather than a separate flag because the
    /// desktop can change its colour scheme while the box is open: the frame is
    /// drawn from the theme every frame, so a body shaped against the old one
    /// would be dark text on a light box.
    shaped: Option<(Rgb, Rgb)>,
    family: Family<'static>,
    /// The metrics the widget was last built with, so a density change can
    /// be told from the size already in hand.
    metrics: Metrics,
    char_width: f32,
    /// The localization the box's words come from.
    catalog: Option<Catalog>,
}

impl AboutBox {
    /// Builds a closed About box.
    pub fn new(metrics: Metrics, family: Family<'static>, char_width: f32) -> Self {
        Self {
            viewport: (0.0, 0.0),
            open: false,
            lines: Vec::new(),
            body: Buffer::new_empty(metrics),
            button: Buffer::new_empty(metrics),
            shaped: None,
            family,
            metrics,
            char_width,
            catalog: None,
        }
    }

    /// Rebuilds the box at a new display density.
    pub fn set_metrics(&mut self, metrics: Metrics) {
        if metrics == self.metrics {
            return;
        }
        // Lengths counted in characters -- a row's column, the box around a
        // label, the width a tab wants -- are measured with `char_width`, so the
        // width grows with the text instead of staying at the size the window
        // opened with.
        self.char_width *= metrics.line_height / self.metrics.line_height;
        self.metrics = metrics;
        self.body.set_metrics(metrics);
        self.button.set_metrics(metrics);
    }

    /// Every label this box can draw, for the coverage test.
    ///
    /// The version line, the author's name and the LinkedIn address are absent:
    /// a version number and a person's name are the same in every language, and
    /// an address is an address.
    pub fn labels() -> Vec<&'static str> {
        vec![BUTTON_LABEL, "A GPU-rendered text editor"]
    }

    /// Swaps in a localization, or clears it with `None`.
    ///
    /// A box that is open is rebuilt, so picking a language with the About box
    /// behind the dialog does not leave the box in the old one.
    pub fn set_catalog(&mut self, catalog: Option<Catalog>) {
        self.catalog = catalog;
        if self.open {
            self.lines = self.translated_lines();
            self.shaped = None;
        }
    }

    /// The box's lines, in the current language.
    fn translated_lines(&self) -> Vec<String> {
        let tr = |english: &str| match &self.catalog {
            Some(catalog) => catalog.tr(english).to_string(),
            None => english.to_string(),
        };
        lines()
            .into_iter()
            .map(|line| match line.as_str() {
                "A GPU-rendered text editor" => tr("A GPU-rendered text editor"),
                // The author line keeps the name and translates the word in
                // front of it: a name is not a label.
                "Author: Stéphane Urdy" => {
                    let prefix = tr("Author:");
                    format!("{prefix} Stéphane Urdy")
                }
                _ => line,
            })
            .collect()
    }

    /// The button's label, in the current language.
    fn button_label(&self) -> String {
        match &self.catalog {
            Some(catalog) => catalog.tr(BUTTON_LABEL).to_string(),
            None => BUTTON_LABEL.to_string(),
        }
    }

    /// Records the window size, so the box can be centred in it.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.viewport = (width, height);
    }

    /// Shows the box.
    ///
    /// The content is rebuilt on every open rather than at construction, so the
    /// version and the author are read at the moment the user asks for them.
    pub fn open(&mut self) {
        self.lines = self.translated_lines();
        self.open = true;
        self.shaped = None;
    }

    /// Hides the box.
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Whether the box is showing.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The lines the box is showing, for tests and for the accessibility tree.
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// The width the box occupies.
    ///
    /// Measured from the longest line rather than fixed, so the LinkedIn address
    /// is never clipped and a shorter About text does not leave a wide empty
    /// margin.
    pub fn width(&self) -> f32 {
        let longest = self
            .lines
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);
        (longest as f32 * self.char_width + PADDING * 2.0)
            .max(MIN_WIDTH * crate::renderer::scale_of(self.metrics))
    }

    /// The height the box occupies.
    pub fn height(&self, line_height: f32) -> f32 {
        if !self.open {
            return 0.0;
        }
        let rows = self.lines.len() as f32;
        let button = BUTTON_HEIGHT * crate::renderer::scale_of(self.metrics);
        PADDING + rows * line_height + BUTTON_GAP + button + PADDING
    }

    /// The left edge: centred, and pinned to zero when the window is narrower
    /// than the box.
    pub fn left(&self) -> f32 {
        ((self.viewport.0 - self.width()) / 2.0).max(0.0)
    }

    /// The top edge: centred, and pinned to zero when the window is shorter than
    /// the box.
    pub fn top(&self, line_height: f32) -> f32 {
        ((self.viewport.1 - self.height(line_height)) / 2.0).max(0.0)
    }

    /// The Close button as `(left, top, width, height)`.
    ///
    /// Right-aligned inside the padding, which is where a dialog's affirmative
    /// button sits on every platform this program runs on.
    pub fn button_rect(&self, line_height: f32) -> (f32, f32, f32, f32) {
        let width = self.button_width();
        let left = self.left() + self.width() - PADDING - width;
        let height = BUTTON_HEIGHT * crate::renderer::scale_of(self.metrics);
        let top = self.top(line_height) + self.height(line_height) - PADDING - height;
        (left, top, width, height)
    }

    /// The button's width.
    ///
    /// Wider than the English button when the translated label is longer, so a
    /// word such as the German "Schließen" still fits inside its border.
    fn button_width(&self) -> f32 {
        let text = self.button_label().chars().count() as f32 * self.char_width;
        (text + PADDING * 2.0).max(BUTTON_WIDTH * crate::renderer::scale_of(self.metrics))
    }

    /// Whether `(x, y)` is on the Close button.
    pub fn close_hit(&self, x: f32, y: f32, line_height: f32) -> bool {
        if !self.open {
            return false;
        }
        let (left, top, width, height) = self.button_rect(line_height);
        x >= left && x < left + width && y >= top && y < top + height
    }

    /// Which line carries the link, if the content has one.
    ///
    /// Found by prefix rather than by index, so a line added above the address
    /// moves the link instead of leaving the click target behind on whatever
    /// line now sits at the old position.
    fn link_line(&self) -> Option<usize> {
        self.lines
            .iter()
            .position(|line| line.starts_with(LINK_PREFIX))
    }

    /// The address itself as `(left, top, width, height)`.
    ///
    /// The address only, not the whole line: "LinkedIn: " is a label, and a
    /// click on a label that opens a browser is a click the user did not aim.
    /// Measured in characters because the family is monospaced, the same way
    /// [`AboutBox::width`] measures the box.
    pub fn link_rect(&self, line_height: f32) -> Option<(f32, f32, f32, f32)> {
        if !self.open {
            return None;
        }
        let index = self.link_line()?;
        let left = self.left() + PADDING + LINK_PREFIX.chars().count() as f32 * self.char_width;
        let top = self.top(line_height) + PADDING + index as f32 * line_height;
        let width = LINK_URL.chars().count() as f32 * self.char_width;
        Some((left, top, width, line_height))
    }

    /// Whether `(x, y)` is on the link.
    pub fn link_hit(&self, x: f32, y: f32, line_height: f32) -> bool {
        match self.link_rect(line_height) {
            Some((left, top, width, height)) => {
                x >= left && x < left + width && y >= top && y < top + height
            }
            None => false,
        }
    }

    /// Whether `(x, y)` is anywhere on the box.
    ///
    /// A click outside dismisses, which is what every dialog does and what a
    /// user tries first when they cannot find the button.
    pub fn contains(&self, x: f32, y: f32, line_height: f32) -> bool {
        if !self.open {
            return false;
        }
        let left = self.left();
        let top = self.top(line_height);
        x >= left && x < left + self.width() && y >= top && y < top + self.height(line_height)
    }

    /// Rebuilds the shaped buffers if the content changed.
    ///
    /// The body is shaped as rich text rather than as one run, because the link
    /// is the one line that is not the same colour as the rest. The spans are
    /// cut on the prefix so the label stays in the body colour and only the
    /// address is tinted -- colouring the whole line would make "LinkedIn: "
    /// look clickable too.
    pub fn prepare(&mut self, font_system: &mut FontSystem, theme: &Theme) {
        if !self.open {
            return;
        }
        let key = (theme.menu_text, theme.link);
        if self.shaped == Some(key) {
            return;
        }
        self.shaped = Some(key);
        let default = Attrs::new()
            .family(self.family)
            .color(theme.menu_text.to_cosmic());
        let link = Attrs::new()
            .family(self.family)
            .color(theme.link.to_cosmic());
        // One line outside ASCII -- a copyright sign, a translated label --
        // needs the fallback shaper for the whole body.
        let shaping = shaping_for(&self.lines.join("\n"));
        let mut spans: Vec<(&str, Attrs)> = Vec::with_capacity(self.lines.len() * 2);
        for (index, line) in self.lines.iter().enumerate() {
            if index > 0 {
                spans.push(("\n", default.clone()));
            }
            match line.strip_prefix(LINK_PREFIX) {
                Some(url) => {
                    spans.push((LINK_PREFIX, default.clone()));
                    spans.push((url, link.clone()));
                }
                None => spans.push((line, default.clone())),
            }
        }
        self.body.set_rich_text(spans, &default, shaping, None);
        self.body.shape_until_scroll(font_system, false);
        let button_label = self.button_label();
        self.button.set_text(
            &button_label,
            &Attrs::new().family(self.family),
            shaping_for(&button_label),
            None,
        );
        self.button.shape_until_scroll(font_system, false);
    }

    /// Queues the frame, the button, the button's border and the link's rule.
    pub fn push_quads(&self, quads: &mut QuadRenderer, line_height: f32, theme: &Theme) {
        if !self.open {
            return;
        }
        let left = self.left();
        let top = self.top(line_height);
        let width = self.width();
        let height = self.height(line_height);

        quads.push(
            left,
            top,
            width,
            height,
            theme.menu_background.to_linear(),
        );
        let border = theme.menu_border.to_linear();
        quads.push(left, top, width, 1.0, border);
        quads.push(left, top + height - 1.0, width, 1.0, border);
        quads.push(left, top, 1.0, height, border);
        quads.push(left + width - 1.0, top, 1.0, height, border);

        let (bx, by, bw, bh) = self.button_rect(line_height);
        quads.push(bx, by, bw, bh, theme.menu_hover.to_linear());
        quads.push(bx, by, bw, 1.0, border);
        quads.push(bx, by + bh - 1.0, bw, 1.0, border);
        quads.push(bx, by, 1.0, bh, border);
        quads.push(bx + bw - 1.0, by, 1.0, bh, border);

        // The rule under the address. Colour alone is a weak signal on a
        // background that is already tinted, and an underline is what every
        // browser and every dialog has taught people a link looks like. It sits
        // two pixels above the bottom of the line box, which is just under the
        // baseline at the sizes this box is drawn at.
        if let Some((lx, ly, lw, lh)) = self.link_rect(line_height) {
            quads.push(lx, ly + lh - 2.0, lw, 1.0, theme.link.to_linear());
        }
    }

    /// The text areas: the body, and the button's label centred in the button.
    pub fn text_areas<'a>(&'a self, line_height: f32, theme: &Theme) -> Vec<TextArea<'a>> {
        if !self.open {
            return Vec::new();
        }
        let left = self.left();
        let top = self.top(line_height);
        let bounds = TextBounds {
            left: left as i32,
            top: top as i32,
            right: (left + self.width()) as i32,
            bottom: (top + self.height(line_height)) as i32,
        };
        let (bx, by, bw, bh) = self.button_rect(line_height);
        // Centred by measurement rather than by padding the string with spaces:
        // the family is monospaced today, but a proportional one would make the
        // spaces the wrong width and the label would sit off-centre.
        let label_width = self.button_label().chars().count() as f32 * self.char_width;
        vec![
            TextArea {
                buffer: &self.body,
                left: left + PADDING,
                top: top + PADDING,
                scale: 1.0,
                bounds,
                default_color: theme.menu_text.to_cosmic(),
                custom_glyphs: &[],
            },
            TextArea {
                buffer: &self.button,
                left: bx + (bw - label_width) / 2.0,
                top: by + (bh - line_height) / 2.0,
                scale: 1.0,
                bounds,
                default_color: theme.menu_text.to_cosmic(),
                custom_glyphs: &[],
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: f32 = 20.0;

    fn about() -> AboutBox {
        let mut about = AboutBox::new(Metrics::new(15.0, LINE), Family::Monospace, 8.0);
        about.set_viewport(1000.0, 700.0);
        about
    }

    #[test]
    fn a_closed_box_has_no_size_and_takes_no_clicks() {
        let about = about();
        assert!(!about.is_open());
        assert_eq!(about.height(LINE), 0.0);
        assert!(!about.close_hit(500.0, 350.0, LINE));
        assert!(!about.contains(500.0, 350.0, LINE));
    }

    #[test]
    fn opening_shows_the_copyright_and_the_linkedin_address() {
        let mut about = about();
        about.open();
        assert!(about.is_open());
        let text = about.lines().join("\n");
        assert!(text.contains("Neko++"), "{text}");
        assert!(text.contains("Copyright (C) 2026 Stéphane Urdy"), "{text}");
        assert!(
            text.contains("https://fr.linkedin.com/in/stephane-urdy-05b2729"),
            "{text}"
        );
    }

    #[test]
    fn the_box_carries_the_warranty_disclaimer_and_the_licence() {
        // The licence asks an interactive program to show where the warranty
        // ends and the terms are; this box is the only place a window has.
        let mut about = about();
        about.open();
        let text = about.lines().join("\n");
        assert!(text.contains("GNU General Public License, version 3"), "{text}");
        assert!(text.contains("ABSOLUTELY NO WARRANTY"), "{text}");
        assert!(text.contains("Help > Neko++ License"), "{text}");
    }

    #[test]
    fn the_version_matches_the_crate() {
        let mut about = about();
        about.open();
        assert_eq!(
            about.lines()[0],
            format!("Neko++ {}", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn the_box_is_centred_in_the_window() {
        let mut about = about();
        about.open();
        assert_eq!(about.left(), (1000.0 - about.width()) / 2.0);
        assert_eq!(about.top(LINE), (700.0 - about.height(LINE)) / 2.0);
    }

    #[test]
    fn a_box_wider_than_the_window_sticks_to_the_left_edge() {
        let mut about = about();
        about.set_viewport(100.0, 700.0);
        about.open();
        assert_eq!(about.left(), 0.0);
    }

    #[test]
    fn the_width_follows_the_longest_line() {
        let mut about = about();
        about.open();
        let longest = about
            .lines()
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap();
        assert_eq!(about.width(), longest as f32 * 8.0 + PADDING * 2.0);
    }

    #[test]
    fn the_height_is_the_text_plus_the_button() {
        let mut about = about();
        about.open();
        let rows = about.lines().len() as f32;
        assert_eq!(
            about.height(LINE),
            PADDING + rows * LINE + BUTTON_GAP + BUTTON_HEIGHT + PADDING
        );
    }

    #[test]
    fn the_button_sits_inside_the_box_at_the_bottom_right() {
        let mut about = about();
        about.open();
        let (bx, by, bw, bh) = about.button_rect(LINE);
        assert_eq!(bw, BUTTON_WIDTH);
        assert_eq!(bh, BUTTON_HEIGHT);
        assert_eq!(bx + bw, about.left() + about.width() - PADDING);
        assert_eq!(by + bh, about.top(LINE) + about.height(LINE) - PADDING);
    }

    #[test]
    fn the_button_is_clickable_and_the_rest_of_the_box_is_not() {
        let mut about = about();
        about.open();
        let (bx, by, bw, bh) = about.button_rect(LINE);
        assert!(about.close_hit(bx + bw / 2.0, by + bh / 2.0, LINE));
        // The top-left corner of the box is inside it but not on the button.
        assert!(about.contains(about.left() + 2.0, about.top(LINE) + 2.0, LINE));
        assert!(!about.close_hit(about.left() + 2.0, about.top(LINE) + 2.0, LINE));
    }

    #[test]
    fn a_click_outside_the_box_misses_it() {
        let mut about = about();
        about.open();
        assert!(!about.contains(5.0, 5.0, LINE));
        assert!(!about.contains(995.0, 695.0, LINE));
    }

    #[test]
    fn closing_hides_the_box() {
        let mut about = about();
        about.open();
        about.close();
        assert!(!about.is_open());
        assert_eq!(about.height(LINE), 0.0);
    }

    #[test]
    fn reopening_after_a_close_shows_the_content_again() {
        let mut about = about();
        about.open();
        about.close();
        about.open();
        assert!(about.is_open());
        assert!(!about.lines().is_empty());
    }

    #[test]
    fn the_link_line_is_the_one_that_starts_with_the_label() {
        let mut about = about();
        about.open();
        let index = about.link_line().expect("the box has a link");
        assert_eq!(about.lines()[index], format!("{LINK_PREFIX}{LINK_URL}"));
    }

    #[test]
    fn the_link_opens_the_address_the_line_shows() {
        let mut about = about();
        about.open();
        let line = &about.lines()[about.link_line().unwrap()];
        assert!(line.ends_with(link_url()), "{line}");
        assert!(link_url().starts_with("https://"), "{}", link_url());
    }

    #[test]
    fn the_link_rect_covers_the_address_and_not_the_label() {
        let mut about = about();
        about.open();
        let (left, top, width, height) = about.link_rect(LINE).expect("the box has a link");
        let index = about.link_line().unwrap();
        assert_eq!(
            left,
            about.left() + PADDING + LINK_PREFIX.chars().count() as f32 * 8.0
        );
        assert_eq!(top, about.top(LINE) + PADDING + index as f32 * LINE);
        assert_eq!(width, LINK_URL.chars().count() as f32 * 8.0);
        assert_eq!(height, LINE);
    }

    #[test]
    fn a_click_on_the_address_is_a_link_click() {
        let mut about = about();
        about.open();
        let (left, top, width, height) = about.link_rect(LINE).unwrap();
        assert!(about.link_hit(left + width / 2.0, top + height / 2.0, LINE));
        // The first and last pixel of the address are on it, so a click on the
        // very edge of the text does what the text says.
        assert!(about.link_hit(left, top, LINE));
        assert!(about.link_hit(left + width - 0.5, top + height - 0.5, LINE));
    }

    #[test]
    fn a_click_on_the_label_is_not_a_link_click() {
        let mut about = about();
        about.open();
        let (left, top, _, height) = about.link_rect(LINE).unwrap();
        // Inside the line, to the left of the address: the "LinkedIn: " label.
        assert!(about.contains(left - 4.0, top + height / 2.0, LINE));
        assert!(!about.link_hit(left - 4.0, top + height / 2.0, LINE));
    }

    #[test]
    fn a_click_on_the_line_above_the_link_is_not_a_link_click() {
        let mut about = about();
        about.open();
        let (left, top, width, _) = about.link_rect(LINE).unwrap();
        assert!(!about.link_hit(left + width / 2.0, top - 1.0, LINE));
        assert!(!about.link_hit(left + width / 2.0, top + LINE, LINE));
    }

    #[test]
    fn a_click_past_the_end_of_the_address_is_not_a_link_click() {
        let mut about = about();
        about.open();
        let (left, top, width, height) = about.link_rect(LINE).unwrap();
        assert!(!about.link_hit(left + width, top + height / 2.0, LINE));
    }

    #[test]
    fn a_closed_box_has_no_link_to_click() {
        let about = about();
        assert!(about.link_rect(LINE).is_none());
        assert!(!about.link_hit(500.0, 350.0, LINE));
    }

    #[test]
    fn the_link_moves_with_the_box_when_the_window_resizes() {
        let mut about = about();
        about.open();
        let (before, _, _, _) = about.link_rect(LINE).unwrap();
        about.set_viewport(1400.0, 700.0);
        let (after, _, _, _) = about.link_rect(LINE).unwrap();
        assert!(after > before, "{after} should be right of {before}");
        assert_eq!(after - before, (1400.0 - 1000.0) / 2.0);
    }
}
