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

//! The file browser.
//!
//! `Ctrl+O` used to open a text prompt and ask for a path. That is fine for a
//! user who knows where the file is and useless for one who does not, so the
//! prompt is replaced by a modal list of the directory's contents.
//!
//! It is drawn in the overlay pass the preferences dialog and the shortcut
//! editor already share, and it follows the same contract: the widget reads the
//! disk but never writes it, and the caller decides what a chosen path means.
//!
//! A native file chooser was the alternative. It would have meant a GTK or
//! portal dependency, which the AppImage cannot assume is present, and a
//! second look for the dialog on each platform. This is one widget that looks
//! the same everywhere and costs nothing at startup.

use std::path::{Path, PathBuf};

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use glyphon::{TextArea, TextBounds};

use crate::findbar::TextField;
use crate::quad::QuadRenderer;
use crate::theme::Theme;

/// The width of the dialog, in pixels.
///
/// Wide enough for a deep path and a long file name side by side.
const WIDTH: f32 = 620.0;
/// Padding inside the frame.
const PADDING: f32 = 14.0;
/// How many rows fit before the list scrolls.
const MAX_VISIBLE: usize = 18;
/// The label before the filter field.
const FILTER_LABEL: &str = "Filter:";

/// One line of the listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The file name, or `..` for the parent directory.
    pub name: String,
    /// Whether the entry is a directory.
    pub is_dir: bool,
}

impl Entry {
    /// The text drawn for this entry.
    ///
    /// Directories carry a trailing slash so they are distinguishable without
    /// a second colour: the font is monospace and the list is one text area, so
    /// a glyph is cheaper than a per-row style.
    pub fn label(&self) -> String {
        if self.is_dir {
            format!("{}/", self.name)
        } else {
            self.name.clone()
        }
    }
}

/// The modal file browser.
pub struct Browser {
    open: bool,
    /// The directory being listed.
    dir: PathBuf,
    /// The visible entries, parent first, then directories, then files.
    entries: Vec<Entry>,
    selected: usize,
    /// The rows marked for opening, as indices into `entries`, in the order
    /// they were marked.
    ///
    /// Indices rather than paths, because a mark is a statement about a row on
    /// screen. Every filter keystroke and every directory change rebuilds the
    /// listing, and [`Browser::refresh`] drops the marks with it: an index that
    /// outlived its listing would open whatever now sits at that number.
    ///
    /// Ordered rather than sorted, because the order is what the editor opens
    /// them in: the last row marked becomes the last document opened, and the
    /// last document opened is the one left in front. A set would answer in row
    /// order, so a user who Ctrl+clicked the top row last would be shown the
    /// bottom one.
    marked: Vec<usize>,
    /// The row a Shift+click measures its range from.
    anchor: usize,
    /// The first visible index, so a long list can scroll.
    scroll: usize,
    /// The filter, which doubles as a path field for an absolute path.
    filter: TextField,
    /// Why the directory could not be read, if it could not.
    error: Option<String>,
    title: Buffer,
    title_shaped: bool,
    label: Buffer,
    label_shaped: bool,
    path: Buffer,
    path_shaped: String,
    body: Buffer,
    body_shaped: String,
    hint: Buffer,
    hint_shaped: String,
    family: Family<'static>,
    /// The metrics the widget was last built with, so a density change can
    /// be told from the size already in hand.
    metrics: Metrics,
    char_width: f32,
    line_height: f32,
    /// Window size, so the dialog can centre itself.
    viewport: (f32, f32),
}

impl Browser {
    /// Builds a closed browser, listing the working directory when it opens.
    pub fn new(metrics: Metrics, family: Family<'static>, char_width: f32) -> Self {
        Self {
            open: false,
            dir: std::env::current_dir().unwrap_or_default(),
            entries: Vec::new(),
            selected: 0,
            marked: Vec::new(),
            anchor: 0,
            scroll: 0,
            filter: TextField::new(metrics, family),
            error: None,
            title: Buffer::new_empty(metrics),
            title_shaped: false,
            label: Buffer::new_empty(metrics),
            label_shaped: false,
            path: Buffer::new_empty(metrics),
            path_shaped: String::new(),
            body: Buffer::new_empty(metrics),
            body_shaped: String::new(),
            hint: Buffer::new_empty(metrics),
            hint_shaped: String::new(),
            family,
            metrics,
            char_width,
            line_height: metrics.line_height,
            viewport: (0.0, 0.0),
        }
    }

    /// Rebuilds the dialog at a new display density.
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
        self.title.set_metrics(metrics);
        self.label.set_metrics(metrics);
        self.path.set_metrics(metrics);
        self.body.set_metrics(metrics);
        self.hint.set_metrics(metrics);
    }

    /// Opens the browser on `dir`, or on the nearest directory above it.
    pub fn open(&mut self, dir: &Path) {
        self.dir = if dir.is_dir() {
            dir.to_path_buf()
        } else {
            dir.parent().map(|p| p.to_path_buf()).unwrap_or_default()
        };
        self.filter.set_text("");
        self.selected = 0;
        self.scroll = 0;
        self.refresh();
        self.open = true;
    }

    /// Closes the browser.
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Whether the browser is showing.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The directory being listed.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The visible entries.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The filter text.
    pub fn filter(&self) -> &str {
        self.filter.text()
    }

    /// The highlighted index.
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// The highlighted entry.
    pub fn selected_entry(&self) -> Option<&Entry> {
        self.entries.get(self.selected)
    }

    /// Whether the row at `index` is marked for opening.
    pub fn is_marked(&self, index: usize) -> bool {
        self.marked.contains(&index)
    }

    /// How many rows are marked.
    pub fn marked_count(&self) -> usize {
        self.marked.len()
    }

    /// Records the window size, so the dialog can centre itself.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.viewport = (width, height);
    }

    /// How many rows are on screen at once.
    pub fn visible_rows(&self) -> usize {
        MAX_VISIBLE
    }

    /// The first visible index.
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// Re-reads the directory and rebuilds the listing.
    ///
    /// A directory that cannot be read is not an error the caller has to
    /// handle: the message goes in the list, where the user is already looking.
    fn refresh(&mut self) {
        self.entries.clear();
        self.error = None;
        let filter = self.filter.text().to_lowercase();
        // A filter that starts with a dot is asking for the hidden files, which
        // is the only way to reach them without a separate toggle.
        let show_hidden = filter.starts_with('.');
        let mut dirs: Vec<String> = Vec::new();
        let mut files: Vec<String> = Vec::new();
        match std::fs::read_dir(&self.dir) {
            Ok(read) => {
                for entry in read.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') && !show_hidden {
                        continue;
                    }
                    if !filter.is_empty() && !name.to_lowercase().contains(&filter) {
                        continue;
                    }
                    let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    if is_dir {
                        dirs.push(name);
                    } else {
                        files.push(name);
                    }
                }
            }
            Err(err) => self.error = Some(err.to_string()),
        }
        dirs.sort_by_key(|name| name.to_lowercase());
        files.sort_by_key(|name| name.to_lowercase());
        // The parent is a navigation affordance rather than a file, so it is
        // never filtered out: a filter that matches nothing still has to leave
        // a way back up.
        if self.dir.parent().is_some() {
            self.entries.push(Entry {
                name: "..".to_string(),
                is_dir: true,
            });
        }
        self.entries.extend(dirs.into_iter().map(|name| Entry {
            name,
            is_dir: true,
        }));
        self.entries.extend(files.into_iter().map(|name| Entry {
            name,
            is_dir: false,
        }));
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
        // A rebuilt listing is a different set of rows, so nothing survives it
        // but the cursor: an index that outlived its listing would open whatever
        // now sits at that number.
        self.select_only(self.selected);
        self.scroll_to_selection();
    }

    /// Moves the highlight by `delta`, clamping at both ends and scrolling to
    /// keep it on screen.
    ///
    /// Clamping rather than wrapping, for the same reason the preferences list
    /// clamps: a list with a first and a last row should stop at them.
    pub fn move_selection(&mut self, delta: isize) {
        let last = self.entries.len().saturating_sub(1) as isize;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
        self.select_only(self.selected);
        self.scroll_to_selection();
    }

    /// Highlights `index`, for a click.
    pub fn set_selected(&mut self, index: usize) {
        if index < self.entries.len() {
            self.selected = index;
            self.select_only(index);
            self.scroll_to_selection();
        }
    }

    /// Marks `index`, or unmarks it when it is already marked, and leaves the
    /// cursor on it -- Ctrl+click.
    ///
    /// Unmarking is the half that makes a mistake recoverable without closing
    /// the dialog, which is the same reason every file manager toggles.
    pub fn toggle_mark(&mut self, index: usize) {
        if index >= self.entries.len() || self.is_parent(index) {
            return;
        }
        if let Some(at) = self.marked.iter().position(|row| *row == index) {
            self.marked.remove(at);
        } else {
            self.marked.push(index);
        }
        self.selected = index;
        self.anchor = index;
        self.scroll_to_selection();
    }

    /// Marks every row between the anchor and `index` -- Shift+click.
    ///
    /// The anchor is the row of the last mark, so repeated Shift+clicks grow one
    /// range rather than accumulating ranges nobody asked for. `extend` keeps
    /// what is already marked, which is what Ctrl+Shift+click does everywhere; a
    /// plain Shift+click replaces the selection.
    pub fn mark_range(&mut self, index: usize, extend: bool) {
        if index >= self.entries.len() {
            return;
        }
        if !extend {
            self.marked.clear();
        }
        let (low, high) = if self.anchor <= index {
            (self.anchor, index)
        } else {
            (index, self.anchor)
        };
        for row in low..=high {
            if !self.is_parent(row) && !self.marked.contains(&row) {
                self.marked.push(row);
            }
        }
        self.selected = index;
        self.scroll_to_selection();
    }

    /// Marks every file in the listing -- Ctrl+A.
    ///
    /// Directories and the parent row are left alone: a mark says "open this",
    /// and there is nothing in a directory to open.
    pub fn mark_all(&mut self) {
        self.marked = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| !entry.is_dir)
            .map(|(index, _)| index)
            .collect();
    }

    /// Makes `index` the whole selection.
    ///
    /// Every way of moving the cursor goes through here, because the row under
    /// the cursor is what a plain click or an arrow key selects: a highlight the
    /// user moved away from is not something to open, and Ctrl click and Shift
    /// click are how the selection grows past it. Clicking a file and
    /// Ctrl+clicking a second one therefore opens both, which is the gesture
    /// every file dialog in the world answers the same way.
    ///
    /// The parent row selects nothing: it is navigation, and a highlighted row
    /// means "this will open".
    fn select_only(&mut self, index: usize) {
        self.marked.clear();
        if index < self.entries.len() && !self.is_parent(index) {
            self.marked.push(index);
        }
        self.anchor = index;
    }

    /// Whether `index` is the parent row, which is a way out rather than a file.
    fn is_parent(&self, index: usize) -> bool {
        self.entries
            .get(index)
            .is_some_and(|entry| entry.name == "..")
    }

    /// Scrolls the minimum amount that puts the selection back on screen.
    fn scroll_to_selection(&mut self) {
        let visible = self.visible_rows();
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + visible {
            self.scroll = self.selected + 1 - visible;
        }
    }

    /// Descends into `path` and clears the filter.
    fn descend(&mut self, path: PathBuf) {
        self.dir = path;
        self.filter.set_text("");
        self.selected = 0;
        self.scroll = 0;
        self.refresh();
    }

    /// Goes up one directory, highlighting the one just left.
    ///
    /// Landing on the directory you came out of is what every file manager
    /// does, and it is the difference between walking back up a tree and
    /// hunting for your place at each step.
    pub fn up(&mut self) {
        let Some(parent) = self.dir.parent().map(|p| p.to_path_buf()) else {
            return;
        };
        let came_from = self.dir.file_name().map(|n| n.to_string_lossy().to_string());
        self.dir = parent;
        self.filter.set_text("");
        self.refresh();
        if let Some(name) = came_from {
            if let Some(index) = self.entries.iter().position(|entry| entry.name == name) {
                self.selected = index;
                self.select_only(index);
                self.scroll_to_selection();
            }
        }
    }

    /// Acts on the highlighted entry.
    ///
    /// Returns the files to open. That is one path for the ordinary case -- the
    /// highlighted file -- and every marked path when rows are marked, because a
    /// mark is the user naming the files they mean. A directory is descended
    /// into and `..` goes up rather than being returned, so a navigation answers
    /// with an empty list: the browser has already done the moving.
    ///
    /// A typed absolute path wins over the list, so a user who knows where they
    /// are going does not have to walk there.
    pub fn enter(&mut self) -> Vec<PathBuf> {
        let typed = self.filter.text().trim();
        if !typed.is_empty() {
            let path = PathBuf::from(typed);
            if path.is_absolute() && path.is_file() {
                return vec![path];
            }
        }
        // The selection is what opens. It normally holds the cursor row too, so
        // this is the ordinary case rather than a special one; a directory in it
        // is dropped, because there is nothing in one to open. When it holds no
        // file at all -- it is empty, or it is one directory -- the cursor below
        // decides, and that is what descends, walks up, or opens the row it is
        // sitting on.
        if !self.marked.is_empty() {
            let paths: Vec<PathBuf> = self
                .marked
                .iter()
                .filter_map(|index| self.entries.get(*index))
                .filter(|entry| !entry.is_dir)
                .map(|entry| self.dir.join(&entry.name))
                .collect();
            if !paths.is_empty() {
                return paths;
            }
        }
        let Some(entry) = self.entries.get(self.selected).cloned() else {
            return Vec::new();
        };
        // `..` goes through `up` rather than `dir.join("..")`: joining leaves
        // the path unnormalised, so the next `parent()` would walk back into
        // the directory just left.
        if entry.name == ".." {
            self.up();
            return Vec::new();
        }
        let path = self.dir.join(&entry.name);
        if entry.is_dir {
            self.descend(path);
            Vec::new()
        } else {
            vec![path]
        }
    }

    /// Inserts a character into the filter and re-reads the listing.
    pub fn insert_char(&mut self, c: char) {
        self.filter.insert(c);
        self.refresh();
        self.select_first_match();
    }

    /// Deletes the character before the filter's caret and re-reads the listing.
    pub fn backspace(&mut self) {
        self.filter.backspace();
        self.refresh();
        self.select_first_match();
    }

    /// Puts the highlight on the first entry that is not the parent.
    ///
    /// Typing a filter is a search, and the answer is never `..`: leaving the
    /// highlight there means Enter walks up the tree instead of opening the one
    /// thing the filter matched.
    fn select_first_match(&mut self) {
        self.selected = self
            .entries
            .iter()
            .position(|entry| entry.name != "..")
            .unwrap_or(0);
        self.select_only(self.selected);
        self.scroll = 0;
        self.scroll_to_selection();
    }

    /// Moves the filter's caret left.
    pub fn left(&mut self) {
        self.filter.left();
    }

    /// Moves the filter's caret right.
    pub fn right(&mut self) {
        self.filter.right();
    }

    /// Moves the filter's caret to the start.
    pub fn home(&mut self) {
        self.filter.home();
    }

    /// Moves the filter's caret to the end.
    pub fn end(&mut self) {
        self.filter.end();
    }

    /// The width the dialog occupies.
    ///
    /// `WIDTH` is the width at the plain density; a display that asks for twice
    /// the text asks for twice the box, or the path runs past its right edge.
    pub fn width(&self) -> f32 {
        WIDTH * crate::renderer::scale_of(self.metrics)
    }

    /// The height the dialog occupies.
    ///
    /// Title, path, filter, the rows, the hint, plus the padding above and
    /// below — the same arithmetic the shortcut editor uses, so the two dialogs
    /// sit at the same rhythm.
    pub fn height(&self) -> f32 {
        PADDING * 2.0 + (self.visible_rows() as f32 + 5.0) * self.line_height
    }

    /// The top-left corner, centred in the window.
    fn origin(&self) -> (f32, f32) {
        let (width, height) = self.viewport;
        let left = ((width - self.width()) / 2.0).max(0.0);
        let top = ((height - self.height()) / 2.0).max(0.0);
        (left, top)
    }

    /// The y pixel where the first row starts.
    fn rows_top(&self) -> f32 {
        self.origin().1 + PADDING + self.line_height * 3.0
    }

    /// The x pixel where the filter field begins.
    fn filter_left(&self) -> f32 {
        self.origin().0 + PADDING + (FILTER_LABEL.chars().count() as f32 + 1.0) * self.char_width
    }

    /// The row index under `(x, y)` in window pixels.
    pub fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        if !self.open {
            return None;
        }
        let (left, top) = self.origin();
        if x < left || x > left + self.width() {
            return None;
        }
        let rows_top = self.rows_top();
        if y < rows_top || y > top + self.height() {
            return None;
        }
        let row = ((y - rows_top) / self.line_height) as usize;
        if row >= self.visible_rows() {
            return None;
        }
        let index = self.scroll + row;
        (index < self.entries.len()).then_some(index)
    }

    /// Shapes the title, the path, the rows, and the hint if they changed.
    pub fn prepare(&mut self, font_system: &mut FontSystem) {
        if !self.open {
            return;
        }
        // The title is a constant, so it is shaped once and never again.
        if !self.title_shaped {
            self.title.set_text(
                self.title_text(),
                &Attrs::new().family(self.family),
                Shaping::Basic,
                None,
            );
            self.title.shape_until_scroll(font_system, false);
            self.title_shaped = true;
        }
        // The filter label is a constant too.
        if !self.label_shaped {
            self.label.set_text(
                self.filter_label(),
                &Attrs::new().family(self.family),
                Shaping::Basic,
                None,
            );
            self.label.shape_until_scroll(font_system, false);
            self.label_shaped = true;
        }

        let path = self.dir.display().to_string();
        if path != self.path_shaped {
            self.path.set_text(
                &path,
                &Attrs::new().family(self.family),
                Shaping::Basic,
                None,
            );
            self.path.shape_until_scroll(font_system, false);
            self.path_shaped = path;
        }

        let mut text = String::new();
        for index in self.scroll..self.scroll + self.visible_rows() {
            let Some(entry) = self.entries.get(index) else {
                break;
            };
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&entry.label());
        }
        if text.is_empty() {
            text.push_str(self.error.as_deref().unwrap_or("(nothing here)"));
        }
        if text != self.body_shaped {
            self.body.set_text(
                &text,
                &Attrs::new().family(self.family),
                Shaping::Basic,
                None,
            );
            self.body.shape_until_scroll(font_system, false);
            self.body_shaped = text;
        }

        if self.hint_shaped != self.hint() {
            self.hint.set_text(
                self.hint(),
                &Attrs::new().family(self.family),
                Shaping::Basic,
                None,
            );
            self.hint.shape_until_scroll(font_system, false);
            self.hint_shaped = self.hint().to_string();
        }

        self.filter.refresh(font_system, self.family);
    }

    /// Queues the frame, the selected row, and the filter caret.
    pub fn push_quads(&self, quads: &mut QuadRenderer, theme: &Theme) {
        if !self.open {
            return;
        }
        let (left, top) = self.origin();
        let width = self.width();
        let height = self.height();

        quads.push(left, top, width, height, theme.menu_background.to_linear());
        let border = theme.menu_border.to_linear();
        quads.push(left, top, width, 1.0, border);
        quads.push(left, top + height - 1.0, width, 1.0, border);
        quads.push(left, top, 1.0, height, border);
        quads.push(left + width - 1.0, top, 1.0, height, border);

        // Marked rows first, so the cursor's own highlight is still readable
        // when it lands on one of them.
        for index in &self.marked {
            if *index < self.scroll || *index >= self.scroll + self.visible_rows() {
                continue;
            }
            let row_top = self.rows_top() + (*index - self.scroll) as f32 * self.line_height;
            quads.push(
                left + 1.0,
                row_top,
                width - 2.0,
                self.line_height,
                theme.selection.to_linear(),
            );
        }

        let row = (self.selected - self.scroll) as f32;
        let row_top = self.rows_top() + row * self.line_height;
        // The cursor keeps its own band only when it is not on a marked row:
        // a band drawn over a mark would hide the mark, and every row the dialog
        // is about to open has to keep showing as chosen.
        if !self.marked.contains(&self.selected) {
            quads.push(
                left + 1.0,
                row_top,
                width - 2.0,
                self.line_height,
                theme.menu_hover.to_linear(),
            );
        }
        // The bar is what says "this is where the keys act", and it is the same
        // answer on a marked row and an unmarked one.
        quads.push(
            left + 1.0,
            row_top,
            2.0,
            self.line_height,
            theme.caret.to_linear(),
        );

        let caret_x = self.filter_left() + self.filter.caret() as f32 * self.char_width;
        quads.push(
            caret_x,
            top + PADDING + self.line_height * 2.0,
            2.0,
            self.line_height,
            theme.caret.to_linear(),
        );
    }

    /// The text areas: the title, the path, the filter, the rows, and the hint.
    pub fn text_areas<'a>(&'a self, theme: &Theme) -> Vec<TextArea<'a>> {
        if !self.open {
            return Vec::new();
        }
        let (left, top) = self.origin();
        let bounds = TextBounds {
            left: left as i32,
            top: top as i32,
            right: (left + self.width()) as i32,
            bottom: (top + self.height()) as i32,
        };
        vec![
            TextArea {
                buffer: &self.title,
                left: left + PADDING,
                top: top + PADDING,
                scale: 1.0,
                bounds,
                default_color: theme.bar_label.to_cosmic(),
                custom_glyphs: &[],
            },
            TextArea {
                buffer: &self.path,
                left: left + PADDING,
                top: top + PADDING + self.line_height,
                scale: 1.0,
                bounds,
                default_color: theme.bar_label.to_cosmic(),
                custom_glyphs: &[],
            },
            TextArea {
                buffer: &self.body,
                left: left + PADDING,
                top: top + PADDING + self.line_height * 3.0,
                scale: 1.0,
                bounds,
                default_color: theme.foreground.to_cosmic(),
                custom_glyphs: &[],
            },
            TextArea {
                buffer: &self.hint,
                left: left + PADDING,
                top: top + PADDING + self.line_height * (self.visible_rows() as f32 + 3.0),
                scale: 1.0,
                bounds,
                default_color: theme.bar_label.to_cosmic(),
                custom_glyphs: &[],
            },
        ]
    }

    /// The filter label and field, drawn as their own areas.
    ///
    /// They are separate from the rest because the field's buffer is owned by
    /// the `TextField`, and the label sits on the same line as it.
    pub fn filter_areas<'a>(&'a self, theme: &Theme) -> Vec<TextArea<'a>> {
        if !self.open {
            return Vec::new();
        }
        let (left, top) = self.origin();
        let bounds = TextBounds {
            left: left as i32,
            top: top as i32,
            right: (left + self.width()) as i32,
            bottom: (top + self.height()) as i32,
        };
        vec![
            TextArea {
                buffer: &self.label,
                left: left + PADDING,
                top: top + PADDING + self.line_height * 2.0,
                scale: 1.0,
                bounds,
                default_color: theme.bar_label.to_cosmic(),
                custom_glyphs: &[],
            },
            TextArea {
                buffer: self.filter.buffer(),
                left: self.filter_left(),
                top: top + PADDING + self.line_height * 2.0,
                scale: 1.0,
                bounds,
                default_color: theme.foreground.to_cosmic(),
                custom_glyphs: &[],
            },
        ]
    }

    /// The title, drawn separately so it can use the label colour.
    pub fn title_text(&self) -> &'static str {
        "Open File"
    }

    /// The filter label, drawn before the field.
    pub fn filter_label(&self) -> &'static str {
        FILTER_LABEL
    }

    /// The hint line under the rows.
    ///
    /// Two lines, because the keys that pick several files are the ones a user
    /// cannot guess: a one-line hint that named only Enter would leave Ctrl and
    /// Shift undiscovered.
    pub fn hint(&self) -> &'static str {
        "Up/Down select   Ctrl+click mark   Shift+click range   Ctrl+A all\n\
         Enter open   Backspace up   Esc cancel"
    }

    /// The character width the dialog was built with.
    pub fn char_width(&self) -> f32 {
        self.char_width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn browser() -> Browser {
        Browser::new(Metrics::new(15.0, 20.0), Family::Monospace, 8.0)
    }

    /// A directory with a known shape: two files, one subdirectory, one hidden
    /// file, and a name that sorts differently by case.
    fn fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("npp-browser-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).expect("fixture dir");
        std::fs::write(dir.join("beta.txt"), "b").expect("fixture file");
        std::fs::write(dir.join("Alpha.txt"), "a").expect("fixture file");
        std::fs::write(dir.join(".hidden"), "h").expect("fixture file");
        dir
    }

    #[test]
    fn a_new_browser_is_closed() {
        assert!(!browser().is_open());
    }

    #[test]
    fn opening_lists_the_directory() {
        let dir = fixture("lists");
        let mut browser = browser();
        browser.open(&dir);
        assert!(browser.is_open());
        assert_eq!(browser.dir(), dir);
        let names: Vec<&str> = browser.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["..", "sub", "Alpha.txt", "beta.txt"]);
    }

    #[test]
    fn directories_come_before_files_and_sort_without_case() {
        let dir = fixture("sort");
        let mut browser = browser();
        browser.open(&dir);
        let dirs: Vec<&Entry> = browser.entries().iter().filter(|e| e.is_dir).collect();
        assert_eq!(dirs.len(), 2);
        assert_eq!(dirs[1].name, "sub");
        let files: Vec<&Entry> = browser.entries().iter().filter(|e| !e.is_dir).collect();
        assert_eq!(files[0].name, "Alpha.txt");
        assert_eq!(files[1].name, "beta.txt");
    }

    #[test]
    fn hidden_files_are_listed_only_when_the_filter_asks() {
        let dir = fixture("hidden");
        let mut browser = browser();
        browser.open(&dir);
        assert!(!browser.entries().iter().any(|e| e.name == ".hidden"));
        browser.insert_char('.');
        assert!(browser.entries().iter().any(|e| e.name == ".hidden"));
    }

    #[test]
    fn the_filter_narrows_the_listing() {
        let dir = fixture("filter");
        let mut browser = browser();
        browser.open(&dir);
        for c in "beta".chars() {
            browser.insert_char(c);
        }
        let names: Vec<&str> = browser.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["..", "beta.txt"]);
    }

    #[test]
    fn the_parent_is_never_filtered_out() {
        let dir = fixture("parent");
        let mut browser = browser();
        browser.open(&dir);
        for c in "zzz".chars() {
            browser.insert_char(c);
        }
        let names: Vec<&str> = browser.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec![".."]);
    }

    #[test]
    fn a_filter_highlights_the_match_not_the_parent() {
        let dir = fixture("first-match");
        let mut browser = browser();
        browser.open(&dir);
        for c in "beta".chars() {
            browser.insert_char(c);
        }
        assert_eq!(browser.selected_entry().map(|e| e.name.as_str()), Some("beta.txt"));
    }

    #[test]
    fn a_filter_that_matches_nothing_falls_back_to_the_parent() {
        let dir = fixture("no-match");
        let mut browser = browser();
        browser.open(&dir);
        for c in "zzz".chars() {
            browser.insert_char(c);
        }
        assert_eq!(browser.selected_entry().map(|e| e.name.as_str()), Some(".."));
    }

    #[test]
    fn backspacing_widens_the_listing_again() {
        let dir = fixture("backspace");
        let mut browser = browser();
        browser.open(&dir);
        for c in "beta".chars() {
            browser.insert_char(c);
        }
        for _ in 0..4 {
            browser.backspace();
        }
        assert_eq!(browser.filter(), "");
        assert_eq!(browser.entries().len(), 4);
    }

    #[test]
    fn entering_a_directory_descends_into_it() {
        let dir = fixture("descend");
        let mut browser = browser();
        browser.open(&dir);
        browser.set_selected(1);
        assert_eq!(browser.selected_entry().map(|e| e.name.as_str()), Some("sub"));
        assert!(browser.enter().is_empty());
        assert_eq!(browser.dir(), dir.join("sub"));
        assert_eq!(browser.filter(), "");
    }

    #[test]
    fn entering_a_file_returns_its_path() {
        let dir = fixture("file");
        let mut browser = browser();
        browser.open(&dir);
        let index = browser
            .entries()
            .iter()
            .position(|e| e.name == "beta.txt")
            .expect("beta.txt listed");
        browser.set_selected(index);
        assert_eq!(browser.enter(), vec![dir.join("beta.txt")]);
    }

    /// The index of `name` in the listing.
    fn row_of(browser: &Browser, name: &str) -> usize {
        browser
            .entries()
            .iter()
            .position(|e| e.name == name)
            .unwrap_or_else(|| panic!("{name} is not listed"))
    }

    #[test]
    fn a_click_then_a_ctrl_click_opens_both_files() {
        let dir = fixture("mark-gesture");
        let mut browser = browser();
        browser.open(&dir);
        // What a user does: click the first file, Ctrl+click the second.
        browser.set_selected(row_of(&browser, "Alpha.txt"));
        browser.toggle_mark(row_of(&browser, "beta.txt"));
        assert_eq!(
            browser.enter(),
            vec![dir.join("Alpha.txt"), dir.join("beta.txt")]
        );
    }

    #[test]
    fn a_ctrl_click_marks_a_row_and_leaves_the_dialog_open() {
        let dir = fixture("mark-one");
        let mut browser = browser();
        browser.open(&dir);
        let row = row_of(&browser, "beta.txt");
        browser.toggle_mark(row);
        assert!(browser.is_marked(row));
        assert_eq!(browser.selected_index(), row);
        // Marking is not opening: the listing is still there.
        assert!(browser.is_open());
        assert_eq!(browser.marked_count(), 1);
    }

    #[test]
    fn a_second_ctrl_click_unmarks_the_row() {
        let dir = fixture("mark-twice");
        let mut browser = browser();
        browser.open(&dir);
        let row = row_of(&browser, "beta.txt");
        browser.toggle_mark(row);
        browser.toggle_mark(row);
        assert!(!browser.is_marked(row));
        assert_eq!(browser.marked_count(), 0);
    }

    #[test]
    fn entering_with_marks_returns_every_marked_file() {
        let dir = fixture("mark-many");
        let mut browser = browser();
        browser.open(&dir);
        let alpha = row_of(&browser, "Alpha.txt");
        let beta = row_of(&browser, "beta.txt");
        browser.toggle_mark(beta);
        browser.toggle_mark(alpha);
        // In the order they were marked, not in row order: `alpha` was marked
        // last, so it opens last and is the tab the editor leaves in front.
        assert_eq!(
            browser.enter(),
            vec![dir.join("beta.txt"), dir.join("Alpha.txt")]
        );
        // The directory has not moved: marks are a chooser, not a walk.
        assert_eq!(browser.dir(), dir);
    }

    #[test]
    fn the_last_row_marked_is_the_last_file_answered() {
        // The editor opens the list in order and activates the last one, so this
        // ordering is what puts the focus on the file the user clicked last.
        let dir = fixture("mark-order");

        // Downwards: the bottom row is last either way.
        let mut down = browser();
        down.open(&dir);
        down.toggle_mark(row_of(&down, "Alpha.txt"));
        down.toggle_mark(row_of(&down, "beta.txt"));
        assert_eq!(
            down.enter(),
            vec![dir.join("Alpha.txt"), dir.join("beta.txt")]
        );

        // Upwards: the case a sorted set got wrong, answering with the bottom
        // row because that is the one that sorts last.
        let mut up = browser();
        up.open(&dir);
        up.toggle_mark(row_of(&up, "beta.txt"));
        up.toggle_mark(row_of(&up, "Alpha.txt"));
        assert_eq!(up.enter(), vec![dir.join("beta.txt"), dir.join("Alpha.txt")]);
    }

    #[test]
    fn a_marked_directory_rides_along_without_being_opened() {
        let dir = fixture("mark-mixed");
        let mut browser = browser();
        browser.open(&dir);
        browser.toggle_mark(row_of(&browser, "sub"));
        browser.toggle_mark(row_of(&browser, "beta.txt"));
        // There is nothing to open in a directory, so a mark on one is skipped
        // rather than turned into a second descent the user did not ask for.
        assert_eq!(browser.enter(), vec![dir.join("beta.txt")]);
        assert_eq!(browser.dir(), dir);
    }

    #[test]
    fn a_shift_click_marks_the_range_from_the_anchor() {
        let dir = fixture("mark-range");
        let mut browser = browser();
        browser.open(&dir);
        let alpha = row_of(&browser, "Alpha.txt");
        let beta = row_of(&browser, "beta.txt");
        browser.toggle_mark(alpha);
        browser.mark_range(beta, false);
        assert!(browser.is_marked(alpha));
        assert!(browser.is_marked(beta));
        assert_eq!(browser.marked_count(), 2);
        // The parent row is above the anchor and stays out of it.
        assert!(!browser.is_marked(0));
    }

    #[test]
    fn a_shift_click_back_up_the_list_marks_the_same_rows() {
        let dir = fixture("mark-range-back");
        let mut browser = browser();
        browser.open(&dir);
        let alpha = row_of(&browser, "Alpha.txt");
        let beta = row_of(&browser, "beta.txt");
        browser.toggle_mark(beta);
        browser.mark_range(alpha, false);
        assert!(browser.is_marked(alpha));
        assert!(browser.is_marked(beta));
    }

    #[test]
    fn a_plain_shift_click_replaces_the_range() {
        let dir = fixture("mark-replace");
        let mut browser = browser();
        browser.open(&dir);
        let alpha = row_of(&browser, "Alpha.txt");
        let beta = row_of(&browser, "beta.txt");
        browser.mark_range(beta, false);
        assert!(browser.is_marked(beta));
        browser.mark_range(alpha, false);
        assert!(browser.is_marked(alpha));
        assert!(!browser.is_marked(beta));
    }

    #[test]
    fn a_ctrl_shift_click_adds_to_the_range() {
        let dir = fixture("mark-extend");
        let mut browser = browser();
        browser.open(&dir);
        let alpha = row_of(&browser, "Alpha.txt");
        let beta = row_of(&browser, "beta.txt");
        browser.toggle_mark(beta);
        browser.mark_range(alpha, true);
        assert!(browser.is_marked(beta));
        assert!(browser.is_marked(alpha));
        assert_eq!(browser.marked_count(), 2);
    }

    #[test]
    fn marking_everything_skips_the_directories() {
        let dir = fixture("mark-all");
        let mut browser = browser();
        browser.open(&dir);
        browser.mark_all();
        assert_eq!(browser.marked_count(), 2);
        assert_eq!(
            browser.enter(),
            vec![dir.join("Alpha.txt"), dir.join("beta.txt")]
        );
    }

    #[test]
    fn an_arrow_key_collapses_the_selection_onto_the_cursor() {
        let dir = fixture("mark-cursor");
        let mut browser = browser();
        browser.open(&dir);
        browser.set_selected(row_of(&browser, "beta.txt"));
        browser.mark_all();
        assert_eq!(browser.marked_count(), 2);
        browser.move_selection(-1);
        assert_eq!(browser.marked_count(), 1);
        assert!(browser.is_marked(browser.selected_index()));
        assert_eq!(browser.enter(), vec![dir.join("Alpha.txt")]);
    }

    #[test]
    fn a_new_listing_selects_the_cursor_and_nothing_else() {
        // Marks are row numbers, so a listing that changed under them would
        // open whatever now sits at that number: typing into the filter
        // replaces the selection with the row it highlights.
        let dir = fixture("mark-cleared");
        let mut browser = browser();
        browser.open(&dir);
        browser.mark_all();
        browser.insert_char('a');
        assert_eq!(browser.marked_count(), 1);
        assert!(browser.is_marked(browser.selected_index()));
    }

    #[test]
    fn going_up_selects_the_directory_just_left() {
        let dir = fixture("mark-descend");
        let mut browser = browser();
        browser.open(&dir);
        browser.mark_all();
        browser.up();
        assert_eq!(browser.marked_count(), 1);
        assert_eq!(
            browser
                .entries()
                .get(browser.selected_index())
                .map(|e| e.name.as_str()),
            dir.file_name().map(|name| name.to_str().unwrap_or_default())
        );
    }

    #[test]
    fn the_parent_row_is_never_part_of_the_selection() {
        let dir = fixture("mark-parent");
        let mut browser = browser();
        browser.open(&dir);
        assert_eq!(browser.selected_index(), 0);
        assert_eq!(browser.selected_entry().map(|e| e.name.as_str()), Some(".."));
        // The row that walks up the tree selects nothing, because a highlighted
        // row means "this will open".
        assert_eq!(browser.marked_count(), 0);
        browser.toggle_mark(0);
        assert_eq!(browser.marked_count(), 0);
        // And it is still Enter's way up.
        assert!(browser.enter().is_empty());
        assert_eq!(browser.dir(), dir.parent().expect("parent"));
    }

    #[test]
    fn a_marked_directory_is_not_opened() {
        let dir = fixture("mark-dir");
        let mut browser = browser();
        browser.open(&dir);
        browser.toggle_mark(row_of(&browser, "sub"));
        // Nothing to open in a directory, so Enter falls back to the cursor,
        // which is the same row, and descends into it.
        assert!(browser.enter().is_empty());
        assert_eq!(browser.dir(), dir.join("sub"));
    }

    #[test]
    fn a_mark_that_scrolls_out_of_view_is_still_opened() {
        let dir = std::env::temp_dir().join("npp-browser-mark-scroll");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("fixture dir");
        for i in 0..40 {
            std::fs::write(dir.join(format!("f{i:02}.txt")), "x").expect("fixture file");
        }
        let mut browser = browser();
        browser.open(&dir);
        // Ctrl+A marks rows the list is not showing, and Enter has to reach them
        // all: a mark is a choice, not a row number the widget can forget when
        // it scrolls.
        browser.mark_all();
        assert_eq!(browser.marked_count(), 40);
        let paths = browser.enter();
        assert_eq!(paths.len(), 40);
        assert_eq!(paths.first(), Some(&dir.join("f00.txt")));
    }

    #[test]
    fn entering_the_parent_goes_up() {
        let dir = fixture("up");
        let mut browser = browser();
        browser.open(&dir.join("sub"));
        assert_eq!(browser.selected_entry().map(|e| e.name.as_str()), Some(".."));
        assert!(browser.enter().is_empty());
        assert_eq!(browser.dir(), dir);
    }

    #[test]
    fn going_up_highlights_the_directory_just_left() {
        let dir = fixture("came-from");
        let mut browser = browser();
        browser.open(&dir.join("sub"));
        browser.up();
        assert_eq!(browser.dir(), dir);
        assert_eq!(browser.selected_entry().map(|e| e.name.as_str()), Some("sub"));
    }

    #[test]
    fn going_up_from_the_root_does_nothing() {
        let mut browser = browser();
        browser.open(Path::new("/"));
        browser.up();
        assert_eq!(browser.dir(), Path::new("/"));
    }

    #[test]
    fn a_typed_absolute_path_wins_over_the_list() {
        let dir = fixture("typed");
        let mut browser = browser();
        browser.open(&dir);
        let target = dir.join("beta.txt");
        for c in target.display().to_string().chars() {
            browser.insert_char(c);
        }
        assert_eq!(browser.enter(), vec![target]);
    }

    #[test]
    fn a_typed_path_that_is_not_a_file_falls_through_to_the_list() {
        let dir = fixture("typed-miss");
        let mut browser = browser();
        browser.open(&dir);
        for c in "/definitely/not/here".chars() {
            browser.insert_char(c);
        }
        // Only `..` survives the filter, and it is a directory.
        assert!(browser.enter().is_empty());
        assert_eq!(browser.dir(), dir.parent().expect("parent"));
    }

    #[test]
    fn selection_clamps_at_both_ends() {
        let dir = fixture("clamp");
        let mut browser = browser();
        browser.open(&dir);
        browser.move_selection(-5);
        assert_eq!(browser.selected_index(), 0);
        browser.move_selection(100);
        assert_eq!(browser.selected_index(), browser.entries().len() - 1);
    }

    #[test]
    fn the_selection_scrolls_into_view() {
        let dir = std::env::temp_dir().join("npp-browser-scroll");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("fixture dir");
        for i in 0..40 {
            std::fs::write(dir.join(format!("f{i:02}.txt")), "x").expect("fixture file");
        }
        let mut browser = browser();
        browser.open(&dir);
        let visible = browser.visible_rows();
        browser.move_selection(visible as isize + 2);
        assert_eq!(browser.scroll(), 3);
        browser.move_selection(-1000);
        assert_eq!(browser.scroll(), 0);
    }

    #[test]
    fn a_click_selects_a_row() {
        let dir = fixture("click");
        let mut browser = browser();
        browser.open(&dir);
        browser.set_viewport(1000.0, 800.0);
        let (left, _) = browser.origin();
        let rows_top = browser.rows_top();
        assert_eq!(browser.row_at(left + 20.0, rows_top + 5.0), Some(0));
        assert_eq!(
            browser.row_at(left + 20.0, rows_top + browser.line_height + 5.0),
            Some(1)
        );
        assert_eq!(browser.row_at(left - 5.0, rows_top + 5.0), None);
    }

    #[test]
    fn a_click_past_the_last_entry_is_none() {
        let dir = fixture("click-past");
        let mut browser = browser();
        browser.open(&dir);
        browser.set_viewport(1000.0, 800.0);
        let (left, _) = browser.origin();
        let y = browser.rows_top() + browser.visible_rows() as f32 * browser.line_height + 5.0;
        assert_eq!(browser.row_at(left + 20.0, y), None);
    }

    #[test]
    fn a_closed_browser_has_no_rows() {
        assert_eq!(browser().row_at(10.0, 10.0), None);
    }

    #[test]
    fn the_dialog_is_centred_in_the_window() {
        let mut browser = browser();
        browser.set_viewport(1000.0, 800.0);
        let (left, top) = browser.origin();
        assert_eq!(left, (1000.0 - WIDTH) / 2.0);
        assert_eq!(top, (800.0 - browser.height()) / 2.0);
    }

    #[test]
    fn a_window_smaller_than_the_dialog_pins_it_to_the_corner() {
        let mut browser = browser();
        browser.set_viewport(100.0, 100.0);
        assert_eq!(browser.origin(), (0.0, 0.0));
    }

    #[test]
    fn a_directory_that_cannot_be_read_says_so() {
        let mut browser = browser();
        browser.open(Path::new("/definitely/not/a/directory"));
        // The way out is still there: an unreadable directory is not a trap.
        assert_eq!(browser.entries().len(), 1);
        assert_eq!(browser.selected_entry().map(|e| e.name.as_str()), Some(".."));
        assert!(browser.error.is_some());
    }

    #[test]
    fn directories_carry_a_trailing_slash() {
        let entry = Entry {
            name: "src".to_string(),
            is_dir: true,
        };
        assert_eq!(entry.label(), "src/");
        let file = Entry {
            name: "main.rs".to_string(),
            is_dir: false,
        };
        assert_eq!(file.label(), "main.rs");
    }

    #[test]
    fn the_hint_names_the_keys() {
        let hint = browser().hint();
        assert!(hint.contains("Ctrl+click"));
        assert!(hint.contains("Shift+click"));
        assert!(hint.contains("Ctrl+A"));
        assert!(hint.contains("Enter"));
        assert!(hint.contains("Backspace"));
        assert!(hint.contains("Esc"));
    }

    #[test]
    fn the_hint_fits_the_dialog() {
        // The hint is drawn inside the dialog's own width and is not wrapped, so
        // a line longer than this is clipped in the middle of a word. It is two
        // lines for exactly that reason.
        let browser = browser();
        let room = ((WIDTH - PADDING * 2.0) / browser.char_width()) as usize;
        for line in browser.hint().lines() {
            assert!(
                line.chars().count() <= room,
                "{line:?} is longer than {room} characters"
            );
        }
    }

    #[test]
    fn opening_a_file_path_lists_its_directory() {
        let dir = fixture("from-file");
        let mut browser = browser();
        browser.open(&dir.join("beta.txt"));
        assert_eq!(browser.dir(), dir);
    }
}
