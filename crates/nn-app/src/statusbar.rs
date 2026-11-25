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

//! The status bar.
//!
//! Notepad++ puts the caret position, the selection size, the document length,
//! the encoding, and the line-ending mode along the bottom edge. It is the one
//! place where the editor tells you what it thinks the file is, so it is worth
//! having even though nothing on it is clickable yet.
//!
//! Two shaped buffers, one left-aligned and one right-aligned. Rebuilding them
//! only when the text changes keeps the bar free on the frames where nothing
//! moved, which is most of them.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use glyphon::{TextArea, TextBounds};
use nn_core::Editor;

use crate::i18n::Catalog;
use crate::quad::QuadRenderer;
use crate::text::shaping_for;
use crate::theme::Theme;

/// The bottom status strip.
pub struct StatusBar {
    left: Buffer,
    right: Buffer,
    left_text: String,
    right_text: String,
    /// The text currently shaped into each buffer, so a reshape is skipped when
    /// nothing changed.
    left_shaped: String,
    right_shaped: String,
    metrics: Metrics,
    family: Family<'static>,
    char_width: f32,
    /// Steps recorded so far, while a macro is being recorded.
    recording: Option<usize>,
    /// A one-off message that takes the place of the right-hand labels.
    message: Option<String>,
    /// The localization the labels are drawn from.
    ///
    /// `None` draws English. The labels are words -- "Ln", "length", "LARGE" --
    /// so a bar left in English under a translated menu bar is a second
    /// language on screen, which is the thing this prevents.
    catalog: Option<Catalog>,
}

impl StatusBar {
    /// Builds an empty bar.
    pub fn new(metrics: Metrics, family: Family<'static>, char_width: f32) -> Self {
        let mut left = Buffer::new_empty(metrics);
        left.set_size(None, None);
        left.set_text("", &Attrs::new().family(family), Shaping::Basic, None);
        let mut right = Buffer::new_empty(metrics);
        right.set_size(None, None);
        right.set_text("", &Attrs::new().family(family), Shaping::Basic, None);
        Self {
            left,
            right,
            left_text: String::new(),
            right_text: String::new(),
            left_shaped: String::new(),
            right_shaped: String::new(),
            metrics,
            family,
            char_width,
            recording: None,
            message: None,
            catalog: None,
        }
    }

    /// Rebuilds the bar at a new display density.
    pub fn set_metrics(&mut self, metrics: Metrics) {
        if metrics == self.metrics {
            return;
        }
        self.metrics = metrics;
        self.left.set_metrics(metrics);
        self.right.set_metrics(metrics);
        // A label is shaped only when its text changes, so the texts these
        // buffers are holding have to look like they changed.
        self.left_shaped.clear();
        self.right_shaped.clear();
    }

    /// Swaps in a localization, or clears it with `None`.
    pub fn set_catalog(&mut self, catalog: Option<Catalog>) {
        self.catalog = catalog;
    }

    /// The translation of `english`, or `english` itself.
    fn tr<'a>(&'a self, english: &'a str) -> &'a str {
        match &self.catalog {
            Some(catalog) => catalog.tr(english),
            None => english,
        }
    }

    /// Shows a one-off message in place of the right-hand labels.
    ///
    /// The caller clears it; there is no timer here, so a message stays until
    /// the next keystroke rather than disappearing mid-read.
    pub fn set_message(&mut self, message: Option<String>) {
        self.message = message;
    }

    /// Shows or hides the recording indicator.
    ///
    /// `None` hides it; `Some(n)` shows the step count, which is the only
    /// feedback a recorder can give while it is running.
    pub fn set_recording(&mut self, steps: Option<usize>) {
        self.recording = steps;
    }

    /// Every label this bar can draw, for the coverage test.
    ///
    /// `INS` and `OVR` are absent on purpose: they are abbreviations rather than
    /// words, the same four glyphs in every language.
    pub fn labels() -> Vec<&'static str> {
        vec!["Ln:", "Col:", "Sel:", "length:", "lines:", "LARGE", "REC"]
    }

    /// The bar's text, as one line.
    ///
    /// The accessibility tree is the caller: a screen reader reads the status
    /// bar as a single label, and the two halves are only separate because they
    /// are drawn at opposite ends of the window.
    pub fn text(&self) -> String {
        match (&self.left_text, &self.right_text) {
            (left, right) if left.is_empty() => right.clone(),
            (left, right) if right.is_empty() => left.clone(),
            (left, right) => format!("{left}  {right}"),
        }
    }

    /// The height the bar occupies.
    pub fn height(&self, line_height: f32) -> f32 {
        line_height + 4.0
    }

    /// Recomputes the labels from the editor.
    pub fn update(&mut self, editor: &Editor) {
        let doc = &editor.doc;
        let caret = editor.caret();
        let line = doc.char_to_line(caret);
        let column = caret - doc.line_to_char(line);
        let selection = editor.selection_len();

        let (ln, col, sel, length, lines) = (
            self.tr("Ln:").to_string(),
            self.tr("Col:").to_string(),
            self.tr("Sel:").to_string(),
            self.tr("length:").to_string(),
            self.tr("lines:").to_string(),
        );
        self.left_text = format!(
            "{ln} {}, {col} {}    {sel} {}    {length} {}    {lines} {}",
            line + 1,
            column + 1,
            selection,
            doc.len_chars(),
            doc.len_lines(),
        );
        // The mode label is the document's, not the window's: Notepad++ keeps
        // insert/overwrite with the buffer, and a status bar that disagreed with
        // what the next keystroke does would be worse than no label at all.
        // Two-and-three-letter abbreviations, not words: `INS` and `OVR` are the
        // same four glyphs in every language the editor speaks, and translating
        // them would be inventing abbreviations rather than translating.
        let mode = if editor.overwrite() { "OVR" } else { "INS" };
        // A large file is unhighlighted and unfolded, and a user who does not
        // know that will think the editor is broken rather than thrifty.
        let large = if doc.is_large() {
            format!("    {}", self.tr("LARGE"))
        } else {
            String::new()
        };
        let rec = self.tr("REC").to_string();
        self.right_text = match &self.message {
            Some(message) => message.clone(),
            None => match self.recording {
                Some(steps) => format!(
                    "{rec} {}    {}    {}    {}{}",
                    steps,
                    doc.encoding().name(),
                    doc.eol().name(),
                    mode,
                    large,
                ),
                None => format!(
                    "{}    {}    {}{}",
                    doc.encoding().name(),
                    doc.eol().name(),
                    mode,
                    large,
                ),
            },
        };
    }

    /// Reshapes the labels if they changed.
    pub fn prepare(&mut self, font_system: &mut FontSystem) {
        if self.left_text != self.left_shaped {
            self.left.set_text(
                &self.left_text,
                &Attrs::new().family(self.family),
                shaping_for(&self.left_text),
                None,
            );
            self.left.shape_until_scroll(font_system, false);
            self.left_shaped.clone_from(&self.left_text);
        }
        if self.right_text != self.right_shaped {
            self.right.set_text(
                &self.right_text,
                &Attrs::new().family(self.family),
                shaping_for(&self.right_text),
                None,
            );
            self.right.shape_until_scroll(font_system, false);
            self.right_shaped.clone_from(&self.right_text);
        }
    }

    /// Queues the bar background and its top border.
    pub fn push_quads(
        &self,
        quads: &mut QuadRenderer,
        width: f32,
        top: f32,
        line_height: f32,
        theme: &Theme,
    ) {
        let height = self.height(line_height);
        quads.push(
            0.0,
            top,
            width,
            height,
            theme.bar_background.to_linear(),
        );
        quads.push(0.0, top, width, 1.0, theme.bar_border.to_linear());
    }

    /// The text areas of the two labels.
    pub fn text_areas<'a>(
        &'a self,
        width: f32,
        top: f32,
        line_height: f32,
        padding: f32,
        theme: &Theme,
    ) -> Vec<TextArea<'a>> {
        let height = self.height(line_height);
        let bounds = TextBounds {
            left: 0,
            top: top as i32,
            right: width as i32,
            bottom: (top + height) as i32,
        };
        let right_width = self.right_text.chars().count() as f32 * self.char_width;
        vec![
            TextArea {
                buffer: &self.left,
                left: padding,
                top: top + 2.0,
                scale: 1.0,
                bounds,
                default_color: theme.bar_label.to_cosmic(),
                custom_glyphs: &[],
            },
            TextArea {
                buffer: &self.right,
                left: (width - padding - right_width).max(padding),
                top: top + 2.0,
                scale: 1.0,
                bounds,
                default_color: theme.bar_label.to_cosmic(),
                custom_glyphs: &[],
            },
        ]
    }

    /// The metrics the bar was built with.
    pub fn metrics(&self) -> Metrics {
        self.metrics
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nn_core::Document;

    fn bar() -> StatusBar {
        StatusBar::new(Metrics::new(15.0, 20.0), Family::Monospace, 8.0)
    }

    fn editor() -> Editor {
        Editor::new(Document::from_text("hello\n"))
    }

    #[test]
    fn the_right_side_reports_the_encoding_and_line_ending() {
        let mut bar = bar();
        bar.update(&editor());
        assert!(bar.right_text.contains("UTF-8"));
        assert!(bar.right_text.contains("INS"));
    }

    #[test]
    fn the_mode_label_follows_the_document() {
        let mut bar = bar();
        let mut ed = editor();
        bar.update(&ed);
        assert!(bar.right_text.ends_with("INS"));
        ed.set_overwrite(true);
        bar.update(&ed);
        assert!(bar.right_text.ends_with("OVR"));
    }

    #[test]
    fn a_large_file_says_so() {
        let mut bar = bar();
        let text = "a".repeat(nn_core::LARGE_FILE_CHARS + 1);
        let ed = Editor::new(Document::from_text(&text));
        bar.update(&ed);
        assert!(bar.right_text.ends_with("LARGE"), "{}", bar.right_text);
    }

    #[test]
    fn a_message_takes_the_place_of_the_right_side() {
        let mut bar = bar();
        bar.set_message(Some("Imported 3 shortcuts".into()));
        bar.update(&editor());
        assert_eq!(bar.right_text, "Imported 3 shortcuts");
    }

    #[test]
    fn clearing_the_message_brings_the_labels_back() {
        let mut bar = bar();
        bar.set_message(Some("Imported 3 shortcuts".into()));
        bar.update(&editor());
        bar.set_message(None);
        bar.update(&editor());
        assert!(bar.right_text.contains("UTF-8"));
    }

    #[test]
    fn the_recording_indicator_outranks_the_labels_but_not_a_message() {
        let mut bar = bar();
        bar.set_recording(Some(4));
        bar.update(&editor());
        assert!(bar.right_text.starts_with("REC 4"));
        bar.set_message(Some("Imported".into()));
        bar.update(&editor());
        assert_eq!(bar.right_text, "Imported");
    }
}
