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

//! The workspace panel: a folder tree, docked on the left.
//!
//! The window docks the same tree on the same edge, for the same reason: a file
//! tree is read top-down, so the root belongs at the left edge and the rows
//! under it. A tree read top-down wants height, which the bottom edge cannot
//! give it -- the results list and the function list live there, and both are
//! bands of eight rows.
//!
//! What differs is how a row is acted on. The window has a pointer: a click
//! opens a folder or opens a file, and the keyboard stays with the text. A
//! terminal has no click on the text to hand the keyboard back, so the tree
//! takes the walking keys while it is up -- as every list in this shell does --
//! and `Enter` opens the row under the cursor: a folder opens or closes where it
//! stands, and a file is opened and the tree put away, because opening a file is
//! arriving somewhere and the reader wants their arrows back.
//!
//! The rows and the line above them come from `nn-shared`, which is where the
//! window's column reads them too.

use nn_core::{Entry, Tree};
use nn_shared::results::truncate;
use nn_shared::workspace;

use crate::keys::Key;
use crate::palette::Tone;
use crate::results::Step;
use crate::screen::Grid;

/// The workspace tree, its cursor, and whether the dock is up.
pub struct Workspace {
    tree: Tree,
    /// The line above the rows: what the panel is showing.
    title: String,
    /// Index of the first visible row.
    scroll: usize,
    /// The row the cursor is standing on, as an index into `tree.rows()`.
    selected: usize,
    open: bool,
    /// The rows the last frame had, which is what a page is worth here.
    visible: usize,
}

impl Workspace {
    /// An empty tree, closed.
    pub fn new() -> Self {
        Workspace {
            tree: Tree::default(),
            title: String::new(),
            scroll: 0,
            selected: 0,
            open: false,
            visible: 1,
        }
    }

    /// Whether the dock is up.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Shows `tree` under `title`, with the cursor on its first row.
    pub fn show(&mut self, tree: Tree, title: impl Into<String>) {
        self.tree = tree;
        self.title = title.into();
        self.scroll = 0;
        self.selected = 0;
        self.open = true;
    }

    /// Takes the dock down and drops the tree.
    ///
    /// The tree is dropped rather than kept: the command that opens the dock
    /// scans the folder every time, so a tree held here would be a second copy
    /// of something nobody is looking at.
    pub fn close(&mut self) {
        self.open = false;
        self.tree = Tree::default();
        self.title.clear();
        self.scroll = 0;
        self.selected = 0;
    }

    /// The tree, for a command that re-reads the disk.
    pub fn tree_mut(&mut self) -> &mut Tree {
        &mut self.tree
    }

    /// How many rows the tree is showing.
    pub fn row_count(&self) -> usize {
        self.tree.rows().len()
    }

    /// The path of the row at `index`.
    pub fn path_at(&self, index: usize) -> Option<std::path::PathBuf> {
        self.tree.rows().get(index).map(|entry| entry.path.clone())
    }

    /// Whether the row at `index` is a folder.
    pub fn is_dir_at(&self, index: usize) -> bool {
        self.tree.rows().get(index).is_some_and(|entry| entry.dir)
    }

    /// Handles a key, answering what it meant.
    pub fn handle(&mut self, key: Key) -> Step {
        match key {
            Key::Up => self.step(-1),
            Key::Down => self.step(1),
            Key::PageUp => self.step(-(self.visible as isize)),
            Key::PageDown => self.step(self.visible as isize),
            Key::LineStart => {
                self.selected = 0;
                self.reveal();
                Step::Moved
            }
            Key::LineEnd => {
                self.selected = self.row_count().saturating_sub(1);
                self.reveal();
                Step::Moved
            }
            Key::Enter if self.row_count() > 0 => Step::Chosen(self.selected),
            Key::Escape => {
                self.close();
                Step::Closed
            }
            _ => Step::No,
        }
    }

    /// Moves the cursor by `delta` rows, and keeps it on the screen.
    fn step(&mut self, delta: isize) -> Step {
        let count = self.row_count();
        if count == 0 {
            return Step::No;
        }
        let next = (self.selected as isize + delta).clamp(0, count as isize - 1);
        self.selected = next as usize;
        self.reveal();
        Step::Moved
    }

    /// Scrolls the least that puts the cursor back on the screen.
    fn reveal(&mut self) {
        let rows = self.visible.max(1);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        }
        if self.selected >= self.scroll + rows {
            self.scroll = self.selected + 1 - rows;
        }
        let most = self.row_count().saturating_sub(rows);
        self.scroll = self.scroll.min(most);
    }

    /// Keeps the cursor and the scroll inside a tree that changed size.
    fn clamp(&mut self) {
        let count = self.row_count();
        if count == 0 {
            self.scroll = 0;
            self.selected = 0;
            return;
        }
        if self.selected >= count {
            self.selected = count - 1;
        }
        self.reveal();
    }

    /// Draws the dock over the `rows` starting at `top`, `width` columns wide.
    ///
    /// The right-hand column is a border rather than a row of the tree: it is
    /// what separates the panel from the text without a frame around the panel,
    /// which on a terminal would be two columns spent on a box nobody asked for.
    pub fn draw(&mut self, grid: &mut Grid, top: usize, rows: usize, width: usize) {
        if !self.open || rows == 0 || width == 0 {
            return;
        }
        self.visible = rows.saturating_sub(1).max(1);
        self.clamp();
        let body = width.saturating_sub(1);
        for row in top..top + rows {
            for column in 0..width {
                grid.put(row, column, ' ', Tone::Menu);
            }
            if width > 1 {
                grid.put(row, width - 1, '│', Tone::Menu);
            }
        }
        // One column of the body is left to the border, which the header must
        // not write over: a title cut to the full width would draw its ellipsis
        // where the border belongs.
        let header = workspace::title(&self.title, &self.tree);
        let room = body.saturating_sub(1);
        grid.text(top, 0, &format!(" {}", truncate(&header, room)), Tone::Menu);
        let entries: Vec<&Entry> = self.tree.rows();
        for offset in 0..rows.saturating_sub(1) {
            let index = self.scroll + offset;
            if index >= entries.len() {
                break;
            }
            let tone = if index == self.selected {
                Tone::MenuHover
            } else {
                Tone::Menu
            };
            let at = top + 1 + offset;
            for column in 0..body {
                grid.put(at, column, ' ', tone);
            }
            let entry = entries[index];
            let expanded = self.tree.is_expanded(&entry.path);
            let text = workspace::row(entry, expanded, body.saturating_sub(1));
            grid.text(at, 0, &format!(" {text}"), tone);
        }
    }
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::Grid;
    use std::path::PathBuf;

    /// A folder with `files` files and a subfolder holding one more.
    fn fixture(name: &str, files: usize) -> PathBuf {
        let root = std::env::temp_dir().join(format!("nn-tui-workspace-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).expect("mkdir");
        for index in 0..files {
            std::fs::write(root.join(format!("f{index}.txt")), "x").expect("write");
        }
        std::fs::write(root.join("sub/deep.txt"), "x").expect("write");
        root
    }

    /// A dock showing `root`, drawn once so the rows are the frame's.
    fn dock(name: &str, files: usize, rows: usize, width: usize) -> (Workspace, PathBuf) {
        let root = fixture(name, files);
        let mut workspace = Workspace::new();
        workspace.show(Tree::scan(std::slice::from_ref(&root)), "Folder as Workspace");
        draw(&mut workspace, rows, width);
        (workspace, root)
    }

    fn draw(workspace: &mut Workspace, rows: usize, width: usize) {
        let mut grid = Grid::new(width, rows);
        workspace.draw(&mut grid, 0, rows, width);
    }

    /// The dock's cells as text, one string per row.
    fn drawn(workspace: &mut Workspace, rows: usize, width: usize) -> Vec<String> {
        let mut grid = Grid::new(width, rows);
        workspace.draw(&mut grid, 0, rows, width);
        (0..rows)
            .map(|row| {
                (0..width)
                    .map(|column| grid.get(row, column).ch)
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn a_closed_dock_is_not_drawn_at_all() {
        let mut workspace = Workspace::new();
        let rows = drawn(&mut workspace, 3, 20);
        assert!(rows.iter().all(|row| row.trim().is_empty()), "{rows:?}");
    }

    /// A dock row without the border column and its padding.
    fn body(row: &str) -> &str {
        row.trim_end_matches('│').trim_end()
    }

    #[test]
    fn the_header_counts_the_files_and_the_rows_are_the_tree() {
        let root = fixture("header", 2);
        let mut workspace = Workspace::new();
        workspace.show(Tree::scan(std::slice::from_ref(&root)), "Folder as Workspace");
        let rows = drawn(&mut workspace, 8, 40);
        // The line above the rows is the panel's title, with the count in it.
        assert_eq!(body(&rows[0]), " Folder as Workspace (3 files)");
        // Then the root, open, and its children one step in: folders first, as
        // the scan sorts them.
        let name = root.file_name().unwrap_or_default().to_string_lossy();
        assert_eq!(body(&rows[1]), format!(" - {name}"));
        assert_eq!(body(&rows[2]), "   + sub");
        assert_eq!(body(&rows[3]), "     f0.txt");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_row_carries_its_marker_and_a_file_row_does_not() {
        let (mut workspace, root) = dock("markers", 1, 10, 40);
        let rows = drawn(&mut workspace, 10, 40);
        // A closed folder is `+`, and opening it where it stands is one
        // keystroke and one call on the tree.
        assert_eq!(body(&rows[2]), "   + sub");
        assert_eq!(body(&rows[3]), "     f0.txt");
        // The cursor lands on the folder, which is the row under the root.
        workspace.handle(Key::Down);
        assert!(workspace.is_dir_at(1));
        let path = workspace.path_at(1).expect("a path");
        workspace.tree_mut().toggle(&path);
        let rows = drawn(&mut workspace, 10, 40);
        assert_eq!(body(&rows[2]), "   - sub");
        assert_eq!(body(&rows[3]), "       deep.txt", "the folder opened");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn escape_takes_the_dock_down_and_enter_on_a_file_is_a_choice() {
        let (mut workspace, root) = dock("escape", 2, 10, 26);
        assert_eq!(workspace.handle(Key::Enter), Step::Chosen(0));
        assert_eq!(workspace.handle(Key::Escape), Step::Closed);
        assert!(!workspace.is_open());
        assert_eq!(workspace.row_count(), 0, "the tree was kept");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_cursor_walks_the_rows_and_stops_at_both_ends() {
        let (mut workspace, root) = dock("walk", 4, 10, 26);
        let count = workspace.row_count();
        assert_eq!(workspace.handle(Key::Up), Step::Moved);
        assert_eq!(workspace.selected + 1, 1);
        workspace.handle(Key::LineEnd);
        assert_eq!(workspace.selected + 1, count);
        workspace.handle(Key::Down);
        assert_eq!(workspace.selected + 1, count);
        workspace.handle(Key::LineStart);
        assert_eq!(workspace.selected + 1, 1);
        // A letter belongs to the document behind the dock.
        assert_eq!(workspace.handle(Key::Char('j')), Step::No);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_cursor_stays_on_screen_while_the_tree_scrolls_under_it() {
        let (mut workspace, root) = dock("scroll", 20, 6, 26);
        // Five rows of tree under a header, so the sixth move scrolls by one.
        for _ in 0..5 {
            workspace.handle(Key::Down);
        }
        assert_eq!(workspace.selected + 1, 6);
        assert_eq!(workspace.scroll, 1);
        workspace.handle(Key::LineEnd);
        assert_eq!(workspace.scroll, workspace.row_count() - 5);
        let _ = std::fs::remove_dir_all(&root);
    }
}
