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

//! The print preview, and the handoff to the system printer.
//!
//! Printing is two halves. The layout and the PDF are in `nn-core` and are
//! pure; this module is the part that needs a window: a sheet drawn on screen,
//! a page stepper, and the page-setup controls that would otherwise need a
//! dialog of their own.
//!
//! The preview doubles as the page-setup dialog. Notepad++ has a separate Page
//! Setup window with paper, orientation, margins and header/footer fields; here
//! the same choices are buttons along the bottom of the preview, where the
//! effect of pressing them is visible immediately. A dialog that changes
//! something you cannot see until you close it is the worse design.
//!
//! The sheet is drawn white on the dark chrome whatever the theme, because a
//! preview of a printed page should look like the paper it will come out on.

use std::path::Path;

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use glyphon::{TextArea, TextBounds};

use nn_core::pdf;
use nn_core::print::{self, Fields, Layout, PageSetup, Paper};

use crate::quad::QuadRenderer;
use crate::theme::Theme;

/// The margin between the window edge and the sheet, in pixels.
const PAD: f32 = 20.0;
/// The height of the title bar and of the control bar.
const BAR: f32 = 26.0;
/// The gap between the sheet and the bars.
const GAP: f32 = 8.0;

/// A control along the bottom of the preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Previous,
    Next,
    Paper,
    Orientation,
    LineNumbers,
    Print,
    Close,
}

impl Control {
    /// Every control, left to right.
    pub const ALL: [Control; 7] = [
        Control::Previous,
        Control::Next,
        Control::Paper,
        Control::Orientation,
        Control::LineNumbers,
        Control::Print,
        Control::Close,
    ];

    /// The label drawn on the button.
    ///
    /// The paper and orientation buttons show their current value rather than
    /// their name, so the bar reads as a summary of the setup as well as a row
    /// of buttons.
    pub fn label(self, setup: &PageSetup) -> String {
        match self {
            Control::Previous => "<".to_string(),
            Control::Next => ">".to_string(),
            Control::Paper => setup.paper.name.to_string(),
            Control::Orientation => {
                if setup.landscape {
                    "Landscape".to_string()
                } else {
                    "Portrait".to_string()
                }
            }
            Control::LineNumbers => {
                if setup.line_numbers {
                    "Numbers: On".to_string()
                } else {
                    "Numbers: Off".to_string()
                }
            }
            Control::Print => "Print".to_string(),
            Control::Close => "Close".to_string(),
        }
    }
}

/// The modal print preview.
pub struct PrintPreview {
    open: bool,
    /// The page being shown, zero-based.
    page: usize,
    setup: PageSetup,
    layout: Layout,
    /// The document, kept so a setup change can lay it out again.
    text: String,
    /// The name shown in the title bar and used as the print job's title.
    name: String,
    fields: Fields,
    /// The width of one character cell at the setup's font size, in points.
    char_width: f32,
    /// The width of one character cell at the interface's font size, in pixels.
    ///
    /// The control bar is drawn at the interface size, not the print size, so
    /// its buttons are laid out from this rather than from `char_width`.
    ui_char_width: f32,
    /// The scale the sheet is drawn at, recomputed from the window size.
    scale: f32,
    title: Buffer,
    title_shaped: String,
    body: Buffer,
    body_shaped: String,
    /// One shaped buffer per control, so each label can be placed inside its
    /// own button rather than relying on tab stops lining up with the layout.
    controls: Vec<Buffer>,
    controls_shaped: Vec<String>,
    family: Family<'static>,
    line_height: f32,
    viewport: (f32, f32),
}

impl PrintPreview {
    /// Builds a closed preview.
    pub fn new(metrics: Metrics, family: Family<'static>, char_width: f32) -> Self {
        Self {
            open: false,
            page: 0,
            setup: PageSetup::default(),
            layout: Layout {
                pages: Vec::new(),
                columns: 1,
                rows: 1,
            },
            text: String::new(),
            name: String::new(),
            fields: Fields::default(),
            char_width,
            ui_char_width: char_width,
            scale: 1.0,
            title: Buffer::new_empty(metrics),
            title_shaped: String::new(),
            body: Buffer::new_empty(metrics),
            body_shaped: String::new(),
            controls: (0..Control::ALL.len())
                .map(|_| Buffer::new_empty(metrics))
                .collect(),
            controls_shaped: vec![String::new(); Control::ALL.len()],
            family,
            line_height: metrics.line_height,
            viewport: (0.0, 0.0),
        }
    }

    /// Opens the preview on a document.
    ///
    /// `char_width` is the width of one character at the setup's font size, in
    /// points. The caller measures it from the font it will draw with, so the
    /// preview and the PDF agree about how many characters fit on a line.
    pub fn open(
        &mut self,
        text: &str,
        name: &str,
        path: Option<&Path>,
        setup: PageSetup,
        char_width: f32,
    ) {
        self.text = text.to_string();
        self.name = name.to_string();
        self.fields = Fields::now();
        self.fields.file_name = name.to_string();
        self.fields.path = path
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.setup = setup;
        self.char_width = char_width;
        self.page = 0;
        self.open = true;
        self.relayout();
    }

    /// Closes the preview.
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Whether the preview is showing.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The page being shown, zero-based.
    pub fn page(&self) -> usize {
        self.page
    }

    /// How many pages the document makes.
    pub fn pages(&self) -> usize {
        self.layout.len()
    }

    /// The setup in force, including any change made in the preview.
    pub fn setup(&self) -> &PageSetup {
        &self.setup
    }

    /// The document's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The laid-out pages.
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Records the window size, so the sheet can be scaled to fit.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        if self.viewport == (width, height) {
            return;
        }
        self.viewport = (width, height);
        self.rescale();
    }

    /// Steps to the next page, stopping at the last.
    pub fn next_page(&mut self) {
        if self.page + 1 < self.pages() {
            self.page += 1;
            self.body_shaped.clear();
        }
    }

    /// Steps to the previous page, stopping at the first.
    pub fn previous_page(&mut self) {
        if self.page > 0 {
            self.page -= 1;
            self.body_shaped.clear();
        }
    }

    /// Steps to page `index`, if it exists.
    pub fn set_page(&mut self, index: usize) {
        if index < self.pages() {
            self.page = index;
            self.body_shaped.clear();
        }
    }

    /// Cycles the paper size.
    pub fn cycle_paper(&mut self) {
        let index = Paper::ALL
            .iter()
            .position(|paper| *paper == self.setup.paper)
            .unwrap_or(0);
        self.setup.paper = Paper::ALL[(index + 1) % Paper::ALL.len()];
        self.relayout();
    }

    /// Flips between portrait and landscape.
    pub fn toggle_orientation(&mut self) {
        self.setup.landscape = !self.setup.landscape;
        self.relayout();
    }

    /// Turns the line-number gutter on or off.
    pub fn toggle_line_numbers(&mut self) {
        self.setup.line_numbers = !self.setup.line_numbers;
        self.relayout();
    }

    /// Lays the document out again after a setup change.
    ///
    /// The page index is kept where it can be: changing the paper size should
    /// not throw the reader back to page one, but a smaller page may not have
    /// as many pages as the old one did.
    fn relayout(&mut self) {
        self.layout = print::layout(&self.text, &self.setup, self.char_width, &self.fields);
        self.page = self.page.min(self.pages().saturating_sub(1));
        self.body_shaped.clear();
        // Reset rather than clear: the vector is indexed by control, so an
        // empty one would be out of bounds on the next frame.
        for shaped in &mut self.controls_shaped {
            shaped.clear();
        }
        self.rescale();
    }

    /// Recomputes the scale that fits the sheet in the window.
    fn rescale(&mut self) {
        let (width, height) = self.viewport;
        let (page_width, page_height) = self.setup.paper.size(self.setup.landscape);
        let available_width = (width - 2.0 * PAD).max(1.0);
        let available_height = (height - 2.0 * (PAD + BAR + GAP)).max(1.0);
        self.scale = (available_width / page_width)
            .min(available_height / page_height)
            .max(0.05);
    }

    /// The sheet's rectangle in window pixels: left, top, width, height.
    pub fn sheet(&self) -> (f32, f32, f32, f32) {
        let (width, height) = self.viewport;
        let (page_width, page_height) = self.setup.paper.size(self.setup.landscape);
        let sheet_width = page_width * self.scale;
        let sheet_height = page_height * self.scale;
        let left = (width - sheet_width) / 2.0;
        let top = PAD + BAR + GAP + ((height - 2.0 * (PAD + BAR + GAP) - sheet_height) / 2.0).max(0.0);
        (left, top, sheet_width, sheet_height)
    }

    /// The rectangle of control `index` along the bottom bar.
    ///
    /// The buttons are laid out from the monospace advance width, like the menu,
    /// so the hit test and the drawing cannot disagree.
    pub fn control_rect(&self, index: usize) -> (f32, f32, f32, f32) {
        let (width, height) = self.viewport;
        let top = height - PAD - BAR;
        let mut left = PAD;
        for (position, control) in Control::ALL.iter().enumerate() {
            let label = control.label(&self.setup);
            let button_width = (label.chars().count() as f32 + 2.0) * self.ui_char_width;
            if position == index {
                return (left, top, button_width, BAR);
            }
            left += button_width + 6.0;
        }
        (width, top, 0.0, BAR)
    }

    /// The control under `(x, y)`, if any.
    pub fn control_at(&self, x: f32, y: f32) -> Option<Control> {
        if !self.open {
            return None;
        }
        for (index, control) in Control::ALL.iter().enumerate() {
            let (left, top, width, height) = self.control_rect(index);
            if x >= left && x <= left + width && y >= top && y <= top + height {
                return Some(*control);
            }
        }
        None
    }

    /// The PDF for the current setup.
    pub fn pdf(&self) -> Vec<u8> {
        pdf::render(&self.layout, &self.setup, self.char_width)
    }

    /// The title bar's text.
    pub fn title(&self) -> String {
        format!("Print - {}", self.name)
    }

    /// Shapes the title, the page and the control bar.
    pub fn prepare(&mut self, font_system: &mut FontSystem) {
        if !self.open {
            return;
        }
        let title = self.title();
        if self.title_shaped != title {
            self.title.set_text(
                &title,
                &Attrs::new().family(self.family),
                Shaping::Basic,
                None,
            );
            self.title.shape_until_scroll(font_system, false);
            self.title_shaped = title;
        }

        for (index, control) in Control::ALL.iter().enumerate() {
            let label = control.label(&self.setup);
            if self.controls_shaped[index] == label {
                continue;
            }
            let buffer = &mut self.controls[index];
            buffer.set_text(
                &label,
                &Attrs::new().family(self.family),
                Shaping::Basic,
                None,
            );
            buffer.shape_until_scroll(font_system, false);
            self.controls_shaped[index] = label;
        }

        let body = self.page_text();
        if self.body_shaped != body {
            // The page is shaped at the print size and drawn scaled, so the
            // cell width the layout used and the cell width on screen are the
            // same number of points.
            self.body.set_metrics(Metrics::new(
                self.setup.font_size,
                self.setup.line_height,
            ));
            self.body.set_text(
                &body,
                &Attrs::new().family(self.family),
                Shaping::Basic,
                None,
            );
            self.body.shape_until_scroll(font_system, false);
            self.body_shaped = body;
        }
    }

    /// The current page as text, one row per line, padded to the page width.
    ///
    /// Padding matters: the header and footer zones are placed by column, and a
    /// monospace row of exactly `columns` characters is what makes the right
    /// zone land at the right margin.
    pub fn page_text(&self) -> String {
        let Some(page) = self.layout.pages.get(self.page) else {
            return String::new();
        };
        let columns = self.layout.columns;
        let mut rows: Vec<String> = Vec::new();
        if page.header.iter().any(|zone| !zone.is_empty()) {
            rows.push(compose(&page.header, columns));
        }
        let digits = self
            .layout
            .pages
            .iter()
            .flat_map(|page| page.lines.iter())
            .filter_map(|line| line.number)
            .max()
            .unwrap_or(1)
            .to_string()
            .len();
        for line in &page.lines {
            if self.setup.line_numbers {
                let number = match line.number {
                    Some(number) => format!("{number:>digits$}"),
                    None => " ".repeat(digits),
                };
                rows.push(format!("{number}  {}", line.text));
            } else {
                rows.push(line.text.clone());
            }
        }
        if page.footer.iter().any(|zone| !zone.is_empty()) {
            rows.push(compose(&page.footer, columns));
        }
        rows.join("\n")
    }

    /// Queues the backdrop, the sheet, and the buttons.
    pub fn push_quads(&self, quads: &mut QuadRenderer, theme: &Theme) {
        if !self.open {
            return;
        }
        let (width, height) = self.viewport;
        // The backdrop covers the window, so the editor behind it cannot be
        // mistaken for part of the preview.
        quads.push(0.0, 0.0, width, height, theme.background.to_linear());
        let (left, top, sheet_width, sheet_height) = self.sheet();
        // A one-pixel shadow, so the sheet reads as a page on a desk rather
        // than a hole in the window.
        quads.push(
            left + 2.0,
            top + 2.0,
            sheet_width,
            sheet_height,
            theme.menu_border.to_linear(),
        );
        quads.push(left, top, sheet_width, sheet_height, PAPER.to_linear());
        for (index, _) in Control::ALL.iter().enumerate() {
            let (x, y, button_width, button_height) = self.control_rect(index);
            quads.push(
                x,
                y,
                button_width,
                button_height,
                theme.menu_background.to_linear(),
            );
            let border = theme.menu_border.to_linear();
            quads.push(x, y, button_width, 1.0, border);
            quads.push(x, y + button_height - 1.0, button_width, 1.0, border);
            quads.push(x, y, 1.0, button_height, border);
            quads.push(x + button_width - 1.0, y, 1.0, button_height, border);
        }
    }

    /// The text areas: the title, the page, and one per control.
    pub fn text_areas<'a>(&'a self, theme: &Theme) -> Vec<TextArea<'a>> {
        if !self.open {
            return Vec::new();
        }
        let (width, height) = self.viewport;
        let window = TextBounds {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        let (left, top, _, _) = self.sheet();
        let (page_width, page_height) = self.setup.paper.size(self.setup.landscape);
        let mut areas = vec![
            TextArea {
                buffer: &self.title,
                left: PAD,
                top: PAD,
                scale: 1.0,
                bounds: window,
                default_color: theme.bar_label.to_cosmic(),
                custom_glyphs: &[],
            },
            TextArea {
                buffer: &self.body,
                // The text starts at the page's margin, scaled with the sheet.
                left: left + self.setup.margins.left * self.scale,
                top: top + self.setup.margins.top * self.scale,
                scale: self.scale,
                // Clipped to the sheet: a row that somehow overflows the page
                // must not be drawn on the desk around it.
                bounds: TextBounds {
                    left: left as i32,
                    top: top as i32,
                    right: (left + page_width * self.scale) as i32,
                    bottom: (top + page_height * self.scale) as i32,
                },
                default_color: INK.to_cosmic(),
                custom_glyphs: &[],
            },
        ];
        for (index, buffer) in self.controls.iter().enumerate() {
            let (button_left, button_top, _, _) = self.control_rect(index);
            areas.push(TextArea {
                buffer,
                left: button_left + self.ui_char_width,
                top: button_top + (BAR - self.line_height) / 2.0,
                scale: 1.0,
                bounds: window,
                default_color: theme.bar_label.to_cosmic(),
                custom_glyphs: &[],
            });
        }
        areas
    }
}

/// The colour of the sheet: white, whatever the theme.
const PAPER: crate::theme::Rgb = crate::theme::Rgb(255, 255, 255);
/// The colour of the text on the sheet.
const INK: crate::theme::Rgb = crate::theme::Rgb(24, 24, 24);

/// Places three zones on a row of `columns` characters.
///
/// Left at the start, right at the end, centre in the middle. A zone that would
/// overlap another is truncated rather than allowed to run over it, because a
/// header that collides with itself is worse than one that is cut.
fn compose(zones: &[String; 3], columns: usize) -> String {
    let mut row: Vec<char> = vec![' '; columns];
    let place = |row: &mut Vec<char>, text: &str, at: usize| {
        for (offset, ch) in text.chars().enumerate() {
            if at + offset < row.len() {
                row[at + offset] = ch;
            }
        }
    };
    place(&mut row, &zones[0], 0);
    let centre = zones[1].chars().count();
    if centre > 0 {
        place(&mut row, &zones[1], columns.saturating_sub(centre) / 2);
    }
    let right = zones[2].chars().count();
    if right > 0 {
        place(&mut row, &zones[2], columns.saturating_sub(right));
    }
    row.into_iter().collect()
}

/// Sends a PDF to the system printer.
///
/// The file is written to the temporary directory and handed to `lp`, which is
/// the CUPS client on every distribution that has one. The temporary file is
/// left behind: `lp` may still be reading it when this returns, and a print
/// queue that loses its input is worse than a file in `/tmp`.
pub fn spool(pdf: &[u8], title: &str) -> Result<String, String> {
    spool_with("lp", pdf, title)
}

/// [`spool`] with the program named, so the failure paths can be tested.
pub fn spool_with(program: &str, pdf: &[u8], title: &str) -> Result<String, String> {
    let stamp = nn_core::datetime::file_stamp(nn_core::datetime::now_secs());
    let path = std::env::temp_dir().join(format!("npp-{stamp}.pdf"));
    std::fs::write(&path, pdf).map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    let output = std::process::Command::new(program)
        .arg("-t")
        .arg(title)
        .arg(&path)
        .output()
        .map_err(|error| format!("cannot run {program}: {error}"))?;
    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(if text.is_empty() {
            format!("sent {} to the printer", path.display())
        } else {
            text
        })
    } else {
        let text = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if text.is_empty() {
            format!("{program} failed")
        } else {
            text
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nn_core::print::Margins;

    fn preview() -> PrintPreview {
        let mut preview = PrintPreview::new(
            Metrics::new(15.0, 20.0),
            Family::Monospace,
            6.0,
        );
        preview.set_viewport(900.0, 700.0);
        preview
    }

    /// A setup with no header or footer, so a page holds exactly `rows` lines.
    fn bare() -> PageSetup {
        PageSetup {
            header: String::new(),
            footer: String::new(),
            ..PageSetup::default()
        }
    }

    fn opened(text: &str) -> PrintPreview {
        let mut preview = preview();
        preview.open(text, "notes.txt", Some(Path::new("/tmp/notes.txt")), PageSetup::default(), 6.0);
        preview
    }

    #[test]
    fn a_new_preview_is_closed() {
        assert!(!preview().is_open());
    }

    #[test]
    fn opening_shows_the_first_page() {
        let preview = opened("one\ntwo");
        assert!(preview.is_open());
        assert_eq!(preview.page(), 0);
        assert_eq!(preview.pages(), 1);
        assert_eq!(preview.name(), "notes.txt");
    }

    #[test]
    fn the_title_names_the_document() {
        assert_eq!(opened("x").title(), "Print - notes.txt");
    }

    #[test]
    fn the_page_text_holds_the_document() {
        let preview = opened("one\ntwo");
        let text = preview.page_text();
        assert!(text.starts_with("one\ntwo"), "{text}");
    }

    #[test]
    fn the_footer_is_the_last_row_of_the_page() {
        let preview = opened("one");
        let text = preview.page_text();
        // The row is padded to the page width, so the number is at its start.
        let last = text.lines().last().unwrap().trim_end();
        assert_eq!(last, "1");
    }

    #[test]
    fn a_header_is_the_first_row_of_the_page() {
        let mut preview = preview();
        let setup = PageSetup {
            header: "$(FILE_NAME)".to_string(),
            ..PageSetup::default()
        };
        preview.open("one", "notes.txt", None, setup, 6.0);
        let text = preview.page_text();
        assert!(text.starts_with("notes.txt"), "{text}");
    }

    #[test]
    fn the_header_and_footer_zones_land_at_the_margins() {
        let mut preview = preview();
        let setup = PageSetup {
            header: "L\tC\tR".to_string(),
            ..PageSetup::default()
        };
        preview.open("one", "notes.txt", None, setup, 6.0);
        let text = preview.page_text();
        let first = text.lines().next().unwrap();
        assert!(first.starts_with('L'), "{first}");
        assert!(first.ends_with('R'), "{first}");
        let centre = first.chars().position(|ch| ch == 'C').unwrap();
        let columns = preview.layout().columns;
        assert!((centre as i32 - (columns / 2) as i32).abs() <= 1, "{first}");
    }

    #[test]
    fn stepping_forward_stops_at_the_last_page() {
        let mut preview = preview();
        let setup = bare();
        let rows = setup.rows();
        let text = (0..rows * 2).map(|i| format!("{i}")).collect::<Vec<_>>().join("\n");
        preview.open(&text, "notes.txt", None, setup, 6.0);
        assert_eq!(preview.pages(), 2);
        preview.next_page();
        assert_eq!(preview.page(), 1);
        preview.next_page();
        assert_eq!(preview.page(), 1);
    }

    #[test]
    fn stepping_back_stops_at_the_first_page() {
        let mut preview = opened("one");
        preview.previous_page();
        assert_eq!(preview.page(), 0);
    }

    #[test]
    fn a_page_can_be_chosen_directly() {
        let mut preview = preview();
        let setup = bare();
        let rows = setup.rows();
        let text = (0..rows * 2).map(|i| format!("{i}")).collect::<Vec<_>>().join("\n");
        preview.open(&text, "notes.txt", None, setup, 6.0);
        preview.set_page(1);
        assert_eq!(preview.page(), 1);
        preview.set_page(9);
        assert_eq!(preview.page(), 1);
    }

    #[test]
    fn cycling_the_paper_walks_the_list_and_wraps() {
        let mut preview = opened("one");
        assert_eq!(preview.setup().paper, Paper::A4);
        preview.cycle_paper();
        assert_eq!(preview.setup().paper, Paper::LETTER);
        for _ in 0..Paper::ALL.len() - 1 {
            preview.cycle_paper();
        }
        assert_eq!(preview.setup().paper, Paper::A4);
    }

    #[test]
    fn a_paper_change_relays_the_document() {
        let mut preview = opened("one");
        let before = preview.layout().columns;
        // A5 is narrower than A4, so fewer columns fit.
        preview.cycle_paper();
        preview.cycle_paper();
        preview.cycle_paper();
        preview.cycle_paper();
        assert_eq!(preview.setup().paper, Paper::A5);
        assert!(preview.layout().columns < before);
    }

    #[test]
    fn a_paper_change_keeps_the_page_when_it_can() {
        let mut preview = preview();
        let setup = bare();
        let rows = setup.rows();
        let text = (0..rows * 3).map(|i| format!("{i}")).collect::<Vec<_>>().join("\n");
        preview.open(&text, "notes.txt", None, setup, 6.0);
        preview.set_page(2);
        preview.toggle_line_numbers();
        assert_eq!(preview.page(), 2);
    }

    #[test]
    fn a_paper_change_clamps_the_page_when_the_document_shrinks() {
        let mut preview = preview();
        let setup = bare();
        let rows = setup.rows();
        // Two pages on A4, one on Tabloid.
        let text = (0..rows + 1).map(|i| format!("{i}")).collect::<Vec<_>>().join("\n");
        preview.open(&text, "notes.txt", None, setup, 6.0);
        assert_eq!(preview.pages(), 2);
        preview.set_page(1);
        while preview.setup().paper != Paper::TABLOID {
            preview.cycle_paper();
        }
        assert_eq!(preview.pages(), 1);
        assert_eq!(preview.page(), 0);
    }

    #[test]
    fn the_orientation_toggles() {
        let mut preview = opened("one");
        assert!(!preview.setup().landscape);
        preview.toggle_orientation();
        assert!(preview.setup().landscape);
        preview.toggle_orientation();
        assert!(!preview.setup().landscape);
    }

    #[test]
    fn landscape_fits_more_columns() {
        let mut preview = opened("one");
        let portrait = preview.layout().columns;
        preview.toggle_orientation();
        assert!(preview.layout().columns > portrait);
    }

    #[test]
    fn the_line_number_gutter_toggles() {
        let mut preview = opened("one");
        assert!(!preview.setup().line_numbers);
        preview.toggle_line_numbers();
        assert!(preview.setup().line_numbers);
        assert!(preview.page_text().starts_with("1  one"), "{}", preview.page_text());
    }

    #[test]
    fn the_sheet_fits_inside_the_window() {
        let preview = opened("one");
        let (left, top, width, height) = preview.sheet();
        assert!(left >= 0.0);
        assert!(top >= 0.0);
        assert!(left + width <= 900.0);
        assert!(top + height <= 700.0);
    }

    #[test]
    fn the_sheet_keeps_the_paper_aspect_ratio() {
        let preview = opened("one");
        let (_, _, width, height) = preview.sheet();
        let (page_width, page_height) = Paper::A4.size(false);
        assert!((width / height - page_width / page_height).abs() < 0.01);
    }

    #[test]
    fn a_landscape_sheet_is_wider_than_it_is_tall() {
        let mut preview = opened("one");
        preview.toggle_orientation();
        let (_, _, width, height) = preview.sheet();
        assert!(width > height);
    }

    #[test]
    fn the_controls_are_laid_out_left_to_right_without_overlapping() {
        let preview = opened("one");
        let mut right = 0.0;
        for index in 0..Control::ALL.len() {
            let (left, _, width, _) = preview.control_rect(index);
            assert!(left >= right, "control {index} overlaps the one before it");
            assert!(width > 0.0);
            right = left + width;
        }
        assert!(right <= 900.0, "the controls run off the window");
    }

    #[test]
    fn a_click_on_a_control_finds_it() {
        let preview = opened("one");
        for (index, control) in Control::ALL.iter().enumerate() {
            let (left, top, width, height) = preview.control_rect(index);
            let hit = preview.control_at(left + width / 2.0, top + height / 2.0);
            assert_eq!(hit, Some(*control));
        }
    }

    #[test]
    fn a_click_away_from_the_controls_finds_nothing() {
        let preview = opened("one");
        assert_eq!(preview.control_at(400.0, 200.0), None);
    }

    #[test]
    fn a_closed_preview_has_no_controls() {
        let preview = preview();
        assert_eq!(preview.control_at(20.0, 660.0), None);
    }

    #[test]
    fn the_control_labels_show_the_current_setup() {
        let mut preview = opened("one");
        assert_eq!(Control::Paper.label(preview.setup()), "A4");
        assert_eq!(Control::Orientation.label(preview.setup()), "Portrait");
        assert_eq!(Control::LineNumbers.label(preview.setup()), "Numbers: Off");
        preview.toggle_orientation();
        preview.toggle_line_numbers();
        assert_eq!(Control::Orientation.label(preview.setup()), "Landscape");
        assert_eq!(Control::LineNumbers.label(preview.setup()), "Numbers: On");
    }

    #[test]
    fn the_pdf_is_a_pdf() {
        let preview = opened("one\ntwo");
        let bytes = preview.pdf();
        assert!(bytes.starts_with(b"%PDF-1.4"));
        assert!(bytes.ends_with(b"%%EOF\n"));
    }

    #[test]
    fn the_pdf_has_one_page_per_page() {
        let mut preview = preview();
        let setup = bare();
        let rows = setup.rows();
        let text = (0..rows * 2).map(|i| format!("{i}")).collect::<Vec<_>>().join("\n");
        preview.open(&text, "notes.txt", None, setup, 6.0);
        let bytes = preview.pdf();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/Count 2"), "{text}");
    }

    #[test]
    fn a_zone_row_is_exactly_the_page_width() {
        let zones = [
            "left".to_string(),
            "mid".to_string(),
            "right".to_string(),
        ];
        let row = compose(&zones, 40);
        assert_eq!(row.chars().count(), 40);
        assert!(row.starts_with("left"));
        assert!(row.ends_with("right"));
    }

    #[test]
    fn a_zone_too_long_for_the_page_is_cut_rather_than_wrapped() {
        let zones = ["x".repeat(50), String::new(), String::new()];
        let row = compose(&zones, 10);
        assert_eq!(row, "xxxxxxxxxx");
    }

    #[test]
    fn an_empty_zone_leaves_spaces() {
        let zones = [String::new(), String::new(), String::new()];
        assert_eq!(compose(&zones, 5), "     ");
    }

    #[test]
    fn the_margins_scale_with_the_sheet() {
        let preview = opened("one");
        let (left, top, _, _) = preview.sheet();
        let areas = preview.text_areas(&Theme::DARK);
        let body = &areas[1];
        assert!((body.left - (left + Margins::NORMAL.left * preview.scale)).abs() < 0.01);
        assert!((body.top - (top + Margins::NORMAL.top * preview.scale)).abs() < 0.01);
        assert!((body.scale - preview.scale).abs() < 0.001);
    }

    #[test]
    fn a_closed_preview_draws_nothing() {
        let preview = preview();
        let theme = Theme::DARK;
        assert!(preview.text_areas(&theme).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_printer_that_succeeds_reports_the_job() {
        // `true` is a real program that exits 0, which is the shape of a
        // successful `lp`.
        let result = spool_with("true", b"%PDF-1.4\n", "notes.txt");
        assert!(result.is_ok(), "{result:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_printer_that_fails_reports_its_error() {
        let result = spool_with("false", b"%PDF-1.4\n", "notes.txt");
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_printer_is_an_error_rather_than_a_panic() {
        let result = spool_with("/nonexistent/lp", b"%PDF-1.4\n", "notes.txt");
        let error = result.expect_err("a missing program");
        assert!(error.contains("cannot run"), "{error}");
    }
}
