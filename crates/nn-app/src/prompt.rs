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

//! The path prompt.
//!
//! Open and Save As need a filename, and a desktop file chooser is a large
//! dependency for one text field. This is a single-line bar that appears under
//! the tab strip, takes a path, and reports it back. It is deliberately modal:
//! while it is open the editor does not receive keystrokes.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use glyphon::{TextArea, TextBounds};

use crate::findbar::TextField;
use crate::i18n::Catalog;
use crate::quad::QuadRenderer;
use crate::theme::Theme;

/// What the prompt is asking for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    /// Read a file into a new tab.
    Open,
    /// Write the active tab to a path.
    SaveAs,
    /// Move the caret to a line number.
    GotoLine,
    /// Split lines on a delimiter.
    SplitLines,
    /// Search a directory tree.
    FindInFiles,
    /// Search every root of every project.
    FindInProjects,
    /// Read a Notepad++ installation's configuration.
    ImportNotepad,
    /// Write the keymap as a Notepad++ `shortcuts.xml`.
    ExportShortcuts,
    /// Text to insert at the caret's column on every selected line.
    ColumnText,
    /// A command line to run.
    Run,
    /// Write a copy of the active tab to a path, leaving the tab where it is.
    SaveCopyAs,
    /// Rename the active tab's file on disk.
    Rename,
    /// Read a session file.
    LoadSession,
    /// Write a session file.
    SaveSession,
    /// Adopt a directory as the workspace root.
    OpenFolderAsWorkspace,
    /// A `strftime` pattern for "Date Time (customized)".
    DateTimeFormat,
    /// The URL template a "Search on Internet" uses.
    SearchEngine,
    /// A file to hash.
    HashFile,
    /// A `shortcuts.xml` to check.
    ValidateShortcuts,
    /// How many times to play the recorded macro.
    MacroRepeat,
    /// A code point to find.
    FindChar,
    /// A `stylers.xml` to take the colours from.
    ImportTheme,
    /// A plugin file to copy into the plugins directory.
    ImportPlugin,
}

impl PromptKind {
    /// Every kind, in the order the enum declares them.
    ///
    /// The coverage test walks this: a kind added without a translation is a
    /// prompt that asks its question in English.
    pub const ALL: [PromptKind; 23] = [
        PromptKind::Open,
        PromptKind::SaveAs,
        PromptKind::GotoLine,
        PromptKind::SplitLines,
        PromptKind::FindInFiles,
        PromptKind::FindInProjects,
        PromptKind::ImportNotepad,
        PromptKind::ExportShortcuts,
        PromptKind::ColumnText,
        PromptKind::Run,
        PromptKind::SaveCopyAs,
        PromptKind::Rename,
        PromptKind::LoadSession,
        PromptKind::SaveSession,
        PromptKind::OpenFolderAsWorkspace,
        PromptKind::DateTimeFormat,
        PromptKind::SearchEngine,
        PromptKind::HashFile,
        PromptKind::ValidateShortcuts,
        PromptKind::MacroRepeat,
        PromptKind::FindChar,
        PromptKind::ImportTheme,
        PromptKind::ImportPlugin,
    ];

    /// The label shown before the field.
    fn label(self) -> &'static str {
        match self {
            PromptKind::Open => "Open:",
            PromptKind::SaveAs => "Save as:",
            PromptKind::GotoLine => "Go to line:",
            PromptKind::SplitLines => "Split on:",
            PromptKind::FindInFiles => "Find in files:",
            PromptKind::FindInProjects => "Find in projects:",
            PromptKind::ImportNotepad => "Import from:",
            PromptKind::ExportShortcuts => "Export to:",
            PromptKind::ColumnText => "Insert at column:",
            PromptKind::Run => "Run:",
            PromptKind::SaveCopyAs => "Save a copy as:",
            PromptKind::Rename => "Rename to:",
            PromptKind::LoadSession => "Load session:",
            PromptKind::SaveSession => "Save session as:",
            PromptKind::OpenFolderAsWorkspace => "Folder:",
            PromptKind::DateTimeFormat => "Date time format:",
            PromptKind::SearchEngine => "Search URL:",
            PromptKind::HashFile => "Hash file:",
            PromptKind::ValidateShortcuts => "shortcuts.xml:",
            PromptKind::MacroRepeat => "Play macro times:",
            PromptKind::FindChar => "Character code:",
            PromptKind::ImportTheme => "Import theme from:",
            PromptKind::ImportPlugin => "Import plugin from:",
        }
    }
}

/// A modal single-line path prompt.
pub struct PromptBar {
    kind: PromptKind,
    field: TextField,
    label: Buffer,
    /// The text currently shaped into `label`, so a reshape is skipped when
    /// neither the kind nor the language changed.
    label_shaped: String,
    /// The localization the label is drawn from.
    catalog: Option<Catalog>,
    open: bool,
    metrics: Metrics,
    family: Family<'static>,
    char_width: f32,
}

impl PromptBar {
    /// Every label a prompt can draw, for the coverage test.
    pub fn labels() -> Vec<&'static str> {
        PromptKind::ALL.iter().map(|kind| kind.label()).collect()
    }

    /// Builds a closed prompt.
    pub fn new(metrics: Metrics, family: Family<'static>, char_width: f32) -> Self {
        let mut label = Buffer::new_empty(metrics);
        label.set_size(None, None);
        label.set_text("", &Attrs::new().family(family), Shaping::Basic, None);
        Self {
            kind: PromptKind::Open,
            field: TextField::new(metrics, family),
            label,
            label_shaped: String::new(),
            catalog: None,
            open: false,
            metrics,
            family,
            char_width,
        }
    }

    /// Swaps in a localization, or clears it with `None`.
    pub fn set_catalog(&mut self, catalog: Option<Catalog>) {
        self.catalog = catalog;
    }

    /// The label this prompt asks with, in the current language.
    fn label_text(&self) -> String {
        match &self.catalog {
            Some(catalog) => catalog.tr(self.kind.label()).to_string(),
            None => self.kind.label().to_string(),
        }
    }

    /// Opens the prompt with `initial` in the field.
    pub fn open(&mut self, kind: PromptKind, initial: &str) {
        self.kind = kind;
        self.field.set_text(initial);
        self.open = true;
    }

    /// Closes the prompt.
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Whether the prompt is showing.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// What the prompt is asking for.
    pub fn kind(&self) -> PromptKind {
        self.kind
    }

    /// The path typed so far.
    pub fn text(&self) -> &str {
        self.field.text()
    }

    /// The height the prompt occupies.
    pub fn height(&self, line_height: f32, padding: f32) -> f32 {
        if !self.open {
            return 0.0;
        }
        line_height + 2.0 * padding
    }

    /// Rebuilds the shaped label and field.
    pub fn prepare(&mut self, font_system: &mut FontSystem) {
        if !self.open {
            return;
        }
        let text = self.label_text();
        if text != self.label_shaped {
            self.label.set_text(
                &text,
                &Attrs::new().family(self.family),
                Shaping::Basic,
                None,
            );
            self.label.shape_until_scroll(font_system, false);
            self.label_shaped = text;
        }
        self.field.refresh(font_system, self.family);
    }

    /// The x pixel where the field begins.
    fn field_left(&self, padding: f32) -> f32 {
        padding + self.label_text().chars().count() as f32 * self.char_width + self.char_width
    }

    /// Queues the bar background, its border, and the caret.
    pub fn push_quads(
        &self,
        quads: &mut QuadRenderer,
        width: f32,
        top: f32,
        line_height: f32,
        padding: f32,
        theme: &Theme,
    ) {
        if !self.open {
            return;
        }
        let height = self.height(line_height, padding);
        quads.push(
            0.0,
            top,
            width,
            height,
            theme.bar_background.to_linear(),
        );
        quads.push(0.0, top, width, 1.0, theme.bar_border.to_linear());
        quads.push(
            0.0,
            top + height - 1.0,
            width,
            1.0,
            theme.bar_border.to_linear(),
        );

        let caret_x = self.field_left(padding) + self.field.caret() as f32 * self.char_width;
        quads.push(
            caret_x,
            top + padding,
            2.0,
            line_height,
            theme.caret.to_linear(),
        );
    }

    /// The text areas of the label and the field.
    pub fn text_areas<'a>(
        &'a self,
        width: f32,
        top: f32,
        line_height: f32,
        padding: f32,
        theme: &Theme,
    ) -> Vec<TextArea<'a>> {
        if !self.open {
            return Vec::new();
        }
        let height = self.height(line_height, padding);
        let bounds = TextBounds {
            left: 0,
            top: top as i32,
            right: width as i32,
            bottom: (top + height) as i32,
        };
        vec![
            TextArea {
                buffer: &self.label,
                left: padding,
                top: top + padding,
                scale: 1.0,
                bounds,
                default_color: theme.bar_label.to_cosmic(),
                custom_glyphs: &[],
            },
            TextArea {
                buffer: self.field.buffer(),
                left: self.field_left(padding),
                top: top + padding,
                scale: 1.0,
                bounds,
                default_color: theme.foreground.to_cosmic(),
                custom_glyphs: &[],
            },
        ]
    }

    /// Inserts a character at the caret.
    pub fn insert_char(&mut self, c: char) {
        self.field.insert(c);
    }

    /// Deletes the character before the caret.
    pub fn backspace(&mut self) {
        self.field.backspace();
    }

    /// Deletes the character after the caret.
    pub fn delete(&mut self) {
        self.field.delete();
    }

    /// Moves the caret left.
    pub fn left(&mut self) {
        self.field.left();
    }

    /// Moves the caret right.
    pub fn right(&mut self) {
        self.field.right();
    }

    /// Moves the caret to the start.
    pub fn home(&mut self) {
        self.field.home();
    }

    /// Moves the caret to the end.
    pub fn end(&mut self) {
        self.field.end();
    }

    /// The metrics the prompt was built with.
    pub fn metrics(&self) -> Metrics {
        self.metrics
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt() -> PromptBar {
        PromptBar::new(Metrics::new(15.0, 20.0), Family::Monospace, 8.0)
    }

    #[test]
    fn a_closed_prompt_takes_no_space() {
        assert_eq!(prompt().height(20.0, 8.0), 0.0);
    }

    #[test]
    fn an_open_prompt_is_one_row_tall() {
        let mut p = prompt();
        p.open(PromptKind::Open, "");
        assert_eq!(p.height(20.0, 8.0), 36.0);
    }

    #[test]
    fn opening_prefills_the_field() {
        let mut p = prompt();
        p.open(PromptKind::SaveAs, "/tmp/a.txt");
        assert_eq!(p.text(), "/tmp/a.txt");
        assert_eq!(p.kind(), PromptKind::SaveAs);
    }

    #[test]
    fn typing_appends_at_the_caret() {
        let mut p = prompt();
        p.open(PromptKind::Open, "/tmp/");
        p.insert_char('x');
        assert_eq!(p.text(), "/tmp/x");
    }

    #[test]
    fn backspace_removes_the_last_character() {
        let mut p = prompt();
        p.open(PromptKind::Open, "abc");
        p.backspace();
        assert_eq!(p.text(), "ab");
    }

    #[test]
    fn closing_hides_the_prompt() {
        let mut p = prompt();
        p.open(PromptKind::Open, "");
        p.close();
        assert!(!p.is_open());
        assert_eq!(p.height(20.0, 8.0), 0.0);
    }

    #[test]
    fn prepare_shapes_the_label_for_the_current_kind() {
        let mut p = prompt();
        assert_eq!(p.label_shaped, "");
        p.open(PromptKind::Open, "");
        p.prepare(&mut FontSystem::new());
        assert_eq!(p.label_shaped, "Open:");
        assert_eq!(p.kind.label(), "Open:");
    }

    #[test]
    fn prepare_reshapes_when_the_kind_changes() {
        let mut p = prompt();
        p.open(PromptKind::Open, "");
        p.prepare(&mut FontSystem::new());
        p.open(PromptKind::GotoLine, "");
        p.prepare(&mut FontSystem::new());
        assert_eq!(p.label_shaped, "Go to line:");
        assert_eq!(p.kind.label(), "Go to line:");
    }

    #[test]
    fn every_kind_has_a_label_ending_in_a_colon() {
        // The label is the only thing that says what the field wants, so a kind
        // added without one would open a blank bar.
        for kind in PromptKind::ALL {
            let label = kind.label();
            assert!(!label.is_empty(), "for {kind:?}");
            assert!(label.ends_with(':'), "for {kind:?}: {label:?}");
        }
    }
}
