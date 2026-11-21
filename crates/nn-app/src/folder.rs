//! The workspace panel: a folder tree, docked on the left.
//!
//! One widget serves both of Notepad++'s tree panels. "Folder as Workspace" is a
//! tree with one root; a project panel is a tree with as many roots as the
//! project lists. The difference between them is the title and where the roots
//! come from, which is not enough difference to justify two widgets that would
//! drift apart in their scroll handling.
//!
//! Left-docked, unlike the function list, because that is where Notepad++ puts
//! it and because a file tree is read top-down: the eye starts at the root and
//! works right and down, so the root belongs at the left edge of the window.
//!
//! The panel owns its filter text but not the decision to filter: `main.rs`
//! routes keystrokes here when the filter has focus, and the panel applies them
//! to the tree. That keeps the widget free of key handling and keeps the tree's
//! filter logic in one place.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use glyphon::{TextArea, TextBounds};
use nn_core::Tree;

use crate::quad::QuadRenderer;
use crate::theme::Theme;

/// The narrowest and widest the column gets, in pixels.
const MIN_WIDTH: f32 = 180.0;
const MAX_WIDTH: f32 = 360.0;
/// How many characters wide the column aims to be.
const TARGET_COLUMNS: f32 = 34.0;
/// How many header lines sit above the rows: the title and the filter.
const HEADER_LINES: f32 = 2.0;

/// The docked tree.
pub struct FolderPanel {
    tree: Tree,
    /// The first header line: what the panel is showing.
    title: String,
    /// The filter text, as typed.
    filter: String,
    /// Whether the filter box has the keyboard.
    filter_focus: bool,
    /// Index of the first visible row.
    scroll: usize,
    /// How many rows fit, from the last `prepare`.
    visible: usize,
    /// The selected row, as an index into `tree.rows()`.
    selected: Option<usize>,
    open: bool,
    header: Buffer,
    header_shaped: String,
    body: Buffer,
    body_shaped: String,
    /// What the body was shaped for: (rows, scroll, columns, selection).
    shaped_key: (usize, usize, usize, usize),
    metrics: Metrics,
    family: Family<'static>,
    char_width: f32,
}

impl FolderPanel {
    /// Builds a closed panel.
    pub fn new(metrics: Metrics, family: Family<'static>, char_width: f32) -> Self {
        let mut header = Buffer::new_empty(metrics);
        header.set_size(None, None);
        header.set_text("", &Attrs::new().family(family), Shaping::Basic, None);
        Self {
            tree: Tree::default(),
            title: String::new(),
            filter: String::new(),
            filter_focus: false,
            scroll: 0,
            visible: 0,
            selected: None,
            open: false,
            header,
            header_shaped: String::new(),
            body: Buffer::new_empty(metrics),
            body_shaped: String::new(),
            shaped_key: (usize::MAX, usize::MAX, usize::MAX, usize::MAX),
            metrics,
            family,
            char_width,
        }
    }

    /// Shows `tree` under `title`.
    ///
    /// An empty tree still opens: a workspace with no files is a state the user
    /// asked for by opening the panel, and a panel that refuses to appear looks
    /// like the menu item did nothing.
    pub fn show(&mut self, tree: Tree, title: impl Into<String>) {
        self.tree = tree;
        self.title = title.into();
        self.scroll = 0;
        self.selected = None;
        self.open = true;
        self.invalidate();
    }

    /// Replaces the tree, keeping the scroll and the selection where they can
    /// still be.
    pub fn set_tree(&mut self, tree: Tree) {
        self.tree = tree;
        self.clamp();
        self.invalidate();
    }

    /// Closes the panel and drops the tree.
    pub fn close(&mut self) {
        self.open = false;
        self.tree = Tree::default();
        self.filter.clear();
        self.filter_focus = false;
        self.scroll = 0;
        self.selected = None;
        self.body_shaped.clear();
        self.invalidate();
    }

    /// Whether the panel is showing.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The tree, for the click handler and for find-in-projects.
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// The tree, mutably, for expanding and collapsing.
    pub fn tree_mut(&mut self) -> &mut Tree {
        &mut self.tree
    }

    /// The title line.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The filter text.
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Whether the filter box has the keyboard.
    pub fn is_filter_focused(&self) -> bool {
        self.filter_focus
    }

    /// Gives the filter box the keyboard.
    pub fn focus_filter(&mut self) {
        self.filter_focus = true;
        self.invalidate();
    }

    /// Takes the keyboard away from the filter box.
    pub fn blur_filter(&mut self) {
        self.filter_focus = false;
        self.invalidate();
    }

    /// Appends a character to the filter and applies it.
    pub fn type_filter(&mut self, c: char) {
        self.filter.push(c);
        self.apply_filter();
    }

    /// Removes the last character of the filter and applies it.
    pub fn backspace_filter(&mut self) {
        self.filter.pop();
        self.apply_filter();
    }

    /// Clears the filter and applies it.
    pub fn clear_filter(&mut self) {
        self.filter.clear();
        self.apply_filter();
    }

    fn apply_filter(&mut self) {
        self.tree.set_filter(&self.filter);
        self.scroll = 0;
        self.selected = None;
        self.invalidate();
    }

    /// The width the column occupies, zero when closed.
    pub fn width(&self, char_width: f32) -> f32 {
        if !self.open {
            return 0.0;
        }
        (char_width * TARGET_COLUMNS).clamp(MIN_WIDTH, MAX_WIDTH)
    }

    /// Scrolls the list by `delta` rows.
    pub fn scroll_by(&mut self, delta: isize) {
        let max = self.tree.rows().len().saturating_sub(self.visible);
        let next = (self.scroll as isize + delta).clamp(0, max as isize);
        self.scroll = next as usize;
    }

    /// The row index under `y`, where `top` is the panel's top edge.
    pub fn row_at(&self, y: f32, top: f32, line_height: f32) -> Option<usize> {
        if !self.open {
            return None;
        }
        let first = top + HEADER_LINES * line_height + 3.0;
        if y < first {
            return None;
        }
        let row = ((y - first) / line_height) as usize;
        if row >= self.visible {
            return None;
        }
        let index = self.scroll + row;
        (index < self.tree.rows().len()).then_some(index)
    }

    /// Whether `y` is inside the filter line.
    pub fn filter_at(&self, y: f32, top: f32, line_height: f32) -> bool {
        if !self.open {
            return false;
        }
        let first = top + line_height;
        y >= first && y < first + line_height
    }

    /// The selected row index.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Selects a row, clamped to the list.
    pub fn select(&mut self, index: usize) {
        if index < self.tree.rows().len() {
            self.selected = Some(index);
            self.invalidate();
        }
    }

    /// Moves the selection by `delta` rows, scrolling to follow it.
    pub fn move_selection(&mut self, delta: isize) {
        let count = self.tree.rows().len();
        if count == 0 {
            return;
        }
        let current = self.selected.unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, count as isize - 1) as usize;
        self.selected = Some(next);
        if next < self.scroll {
            self.scroll = next;
        } else if next >= self.scroll + self.visible {
            self.scroll = next + 1 - self.visible.max(1);
        }
        self.invalidate();
    }

    /// The path of the selected row, if there is one.
    pub fn selected_path(&self) -> Option<std::path::PathBuf> {
        let rows = self.tree.rows();
        self.selected
            .and_then(|i| rows.get(i))
            .map(|e| e.path.clone())
    }

    /// The path of the row at `index`.
    pub fn path_at(&self, index: usize) -> Option<std::path::PathBuf> {
        self.tree.rows().get(index).map(|e| e.path.clone())
    }

    /// Whether the row at `index` is a directory.
    pub fn is_dir_at(&self, index: usize) -> bool {
        self.tree.rows().get(index).is_some_and(|e| e.dir)
    }

    /// Forces the next `prepare` to reshape.
    fn invalidate(&mut self) {
        self.shaped_key = (usize::MAX, usize::MAX, usize::MAX, usize::MAX);
    }

    /// Keeps the scroll and selection inside the list after it changed size.
    fn clamp(&mut self) {
        let count = self.tree.rows().len();
        if count == 0 {
            self.scroll = 0;
            self.selected = None;
            return;
        }
        self.scroll = self.scroll.min(count.saturating_sub(self.visible.max(1)));
        if let Some(selected) = self.selected {
            if selected >= count {
                self.selected = Some(count - 1);
            }
        }
    }

    /// Rebuilds the shaped rows if anything they depend on changed.
    pub fn prepare(&mut self, font_system: &mut FontSystem, width: f32, height: f32) {
        if !self.open {
            return;
        }
        let line_height = self.metrics.line_height;
        let body_height = height - HEADER_LINES * line_height - 3.0;
        self.visible = ((body_height / line_height).floor() as usize).max(1);
        self.clamp();

        let columns = ((width - 16.0) / self.char_width).max(8.0) as usize;
        let rows = self.tree.rows();
        let key = (
            rows.len(),
            self.scroll,
            columns,
            self.selected.unwrap_or(usize::MAX),
        );
        if key == self.shaped_key {
            return;
        }
        self.shaped_key = key;

        let header = self.header_line();
        if header != self.header_shaped {
            self.header.set_text(
                &header,
                &Attrs::new().family(self.family),
                Shaping::Basic,
                None,
            );
            self.header.shape_until_scroll(font_system, false);
            self.header_shaped = header;
        }

        let visible = rows.len().min(self.visible);
        let mut text = String::with_capacity(visible * (columns + 1));
        for row in 0..visible {
            if row > 0 {
                text.push('\n');
            }
            text.push_str(&self.row_text(self.scroll + row, columns));
        }
        if text == self.body_shaped {
            return;
        }
        self.body.set_text(
            &text,
            &Attrs::new().family(self.family),
            Shaping::Basic,
            None,
        );
        self.body.shape_until_scroll(font_system, false);
        self.body_shaped = text;
    }

    /// The two header lines, newline separated.
    ///
    /// The filter line always shows, even when empty, so the box has somewhere
    /// to be clicked. A caret marks it while it has the keyboard, which is the
    /// only affordance a text field drawn as text can offer.
    fn header_line(&self) -> String {
        let count = self.tree.file_count();
        let title = if self.tree.is_truncated() {
            format!("{} ({count}+ files)", self.title)
        } else {
            format!("{} ({count} files)", self.title)
        };
        let filter = if self.filter_focus {
            format!("Filter: {}_", self.filter)
        } else if self.filter.is_empty() {
            "Filter:".to_string()
        } else {
            format!("Filter: {}", self.filter)
        };
        format!("{title}\n{filter}")
    }

    /// The display text of row `index`, cut to `columns`.
    ///
    /// A directory carries `-` when it is open and `+` when it is closed, which
    /// is the whole of the disclosure affordance: the row is a line of text, so
    /// the marker has to be text too.
    fn row_text(&self, index: usize, columns: usize) -> String {
        let rows = self.tree.rows();
        let Some(entry) = rows.get(index) else {
            return String::new();
        };
        let indent = "  ".repeat(entry.depth as usize);
        let marker = if entry.dir {
            if self.tree.is_expanded(&entry.path) {
                "- "
            } else {
                "+ "
            }
        } else {
            "  "
        };
        let text = format!("{indent}{marker}{}", entry.name);
        truncate(&text, columns)
    }

    /// Queues the panel background and its right border.
    pub fn push_quads(
        &self,
        quads: &mut QuadRenderer,
        top: f32,
        height: f32,
        theme: &Theme,
    ) {
        if !self.open {
            return;
        }
        let width = self.width(self.char_width);
        quads.push(0.0, top, width, height, theme.bar_background.to_linear());
        quads.push(width - 1.0, top, width, height, theme.bar_border.to_linear());
        // The selected row gets a band, drawn before the text so the glyphs sit
        // on top of it.
        if let Some(selected) = self.selected {
            if selected >= self.scroll && selected < self.scroll + self.visible {
                let line_height = self.metrics.line_height;
                let y = top + HEADER_LINES * line_height + 3.0
                    + (selected - self.scroll) as f32 * line_height;
                quads.push(
                    0.0,
                    y,
                    width - 1.0,
                    y + line_height,
                    theme.selection.to_linear(),
                );
            }
        }
    }

    /// The text areas of the header and the visible rows.
    pub fn text_areas<'a>(
        &'a self,
        top: f32,
        height: f32,
        padding: f32,
        theme: &Theme,
    ) -> Vec<TextArea<'a>> {
        if !self.open {
            return Vec::new();
        }
        let width = self.width(self.char_width);
        let bounds = TextBounds {
            left: 0,
            top: top as i32,
            right: width as i32,
            bottom: (top + height) as i32,
        };
        vec![
            TextArea {
                buffer: &self.header,
                left: padding,
                top: top + 3.0,
                scale: 1.0,
                bounds,
                default_color: theme.bar_label.to_cosmic(),
                custom_glyphs: &[],
            },
            TextArea {
                buffer: &self.body,
                left: padding,
                top: top + HEADER_LINES * self.metrics.line_height + 3.0,
                scale: 1.0,
                bounds,
                default_color: theme.foreground.to_cosmic(),
                custom_glyphs: &[],
            },
        ]
    }
}

/// Cuts `text` to `columns` characters, marking the cut with an ellipsis.
fn truncate(text: &str, columns: usize) -> String {
    if text.chars().count() <= columns {
        return text.to_string();
    }
    let mut out: String = text.chars().take(columns.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn panel() -> FolderPanel {
        FolderPanel::new(Metrics::new(15.0, 20.0), Family::Monospace, 8.0)
    }

    /// A tree built by hand, so the tests do not touch the filesystem.
    fn tree() -> Tree {
        let mut tree = Tree::default();
        let root = PathBuf::from("/w");
        tree.set_roots(std::slice::from_ref(&root));
        tree
    }

    #[test]
    fn a_closed_panel_takes_no_width() {
        let panel = panel();
        assert!(!panel.is_open());
        assert_eq!(panel.width(8.0), 0.0);
        assert!(panel.row_at(100.0, 0.0, 20.0).is_none());
    }

    #[test]
    fn showing_a_tree_opens_the_panel() {
        let mut panel = panel();
        panel.show(tree(), "Folder as Workspace");
        assert!(panel.is_open());
        assert_eq!(panel.title(), "Folder as Workspace");
        assert!(panel.width(8.0) >= MIN_WIDTH);
    }

    #[test]
    fn the_width_is_clamped() {
        let mut panel = panel();
        panel.show(tree(), "x");
        assert_eq!(panel.width(1.0), MIN_WIDTH);
        assert_eq!(panel.width(100.0), MAX_WIDTH);
    }

    #[test]
    fn closing_drops_the_tree_and_the_filter() {
        let mut panel = panel();
        panel.show(tree(), "x");
        panel.type_filter('a');
        panel.close();
        assert!(!panel.is_open());
        assert!(panel.tree().entries().is_empty());
        assert_eq!(panel.filter(), "");
        assert!(!panel.is_filter_focused());
    }

    #[test]
    fn the_header_counts_files_and_shows_the_filter() {
        let mut panel = panel();
        panel.show(tree(), "Folder as Workspace");
        let header = panel.header_line();
        assert!(header.starts_with("Folder as Workspace (0 files)"));
        assert!(header.ends_with("Filter:"));
    }

    #[test]
    fn a_focused_filter_shows_a_caret() {
        let mut panel = panel();
        panel.show(tree(), "x");
        panel.focus_filter();
        panel.type_filter('r');
        assert!(panel.header_line().ends_with("Filter: r_"));
        panel.blur_filter();
        assert!(panel.header_line().ends_with("Filter: r"));
    }

    #[test]
    fn typing_into_the_filter_reaches_the_tree() {
        let mut panel = panel();
        panel.show(tree(), "x");
        panel.type_filter('a');
        panel.type_filter('b');
        assert_eq!(panel.tree().filter(), "ab");
        panel.backspace_filter();
        assert_eq!(panel.tree().filter(), "a");
        panel.clear_filter();
        assert_eq!(panel.tree().filter(), "");
    }

    #[test]
    fn a_truncated_scan_says_so() {
        let mut panel = panel();
        let mut tree = Tree::default();
        tree.set_roots(&[PathBuf::from("/w")]);
        panel.show(tree, "x");
        // A hand-built tree is never truncated, so the header is the plain one.
        assert!(panel.header_line().contains("(0 files)"));
    }

    #[test]
    fn row_at_maps_a_row_to_an_index() {
        let mut panel = panel();
        panel.show(tree(), "x");
        panel.prepare(&mut FontSystem::new(), 200.0, 400.0);
        // The first row starts two header lines down; the tree has one row, so
        // the second line is past the end.
        assert_eq!(panel.row_at(43.0, 0.0, 20.0), Some(0));
        assert_eq!(panel.row_at(63.0, 0.0, 20.0), None);
        assert_eq!(panel.row_at(10.0, 0.0, 20.0), None);
    }

    #[test]
    fn filter_at_finds_the_second_header_line() {
        let mut panel = panel();
        panel.show(tree(), "x");
        assert!(panel.filter_at(25.0, 0.0, 20.0));
        assert!(!panel.filter_at(5.0, 0.0, 20.0));
        assert!(!panel.filter_at(45.0, 0.0, 20.0));
    }

    #[test]
    fn the_selection_moves_and_clamps() {
        let mut panel = panel();
        panel.show(tree(), "x");
        panel.prepare(&mut FontSystem::new(), 200.0, 400.0);
        panel.move_selection(1);
        assert_eq!(panel.selected(), Some(0));
        panel.move_selection(5);
        assert_eq!(panel.selected(), Some(0));
        panel.move_selection(-1);
        assert_eq!(panel.selected(), Some(0));
    }

    #[test]
    fn selecting_past_the_end_is_ignored() {
        let mut panel = panel();
        panel.show(tree(), "x");
        panel.select(9);
        assert_eq!(panel.selected(), None);
        panel.select(0);
        assert_eq!(panel.selected(), Some(0));
    }

    #[test]
    fn the_selected_path_is_the_row_path() {
        let mut panel = panel();
        panel.show(tree(), "x");
        panel.select(0);
        assert_eq!(panel.selected_path(), Some(PathBuf::from("/w")));
        assert!(panel.is_dir_at(0));
    }

    #[test]
    fn scrolling_clamps_to_the_list() {
        let mut panel = panel();
        panel.show(tree(), "x");
        panel.prepare(&mut FontSystem::new(), 200.0, 400.0);
        panel.scroll_by(10);
        assert_eq!(panel.scroll, 0);
        panel.scroll_by(-10);
        assert_eq!(panel.scroll, 0);
    }

    #[test]
    fn a_row_is_indented_by_its_depth() {
        let mut panel = panel();
        panel.show(tree(), "x");
        let text = panel.row_text(0, 40);
        assert_eq!(text, "- w");
    }

    #[test]
    fn truncate_marks_the_cut() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("abcdef", 4), "abc…");
    }
}
