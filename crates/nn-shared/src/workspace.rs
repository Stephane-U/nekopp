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

//! What the workspace panel says about itself.
//!
//! Both front-ends show a `nn_core::workspace::Tree` down the left edge: a
//! column of shaped glyphs in the window, a column of cells in the terminal.
//! The width and the font differ; what a row says does not. A second wording
//! would drift the first time an indent or a count was changed in one of them,
//! and the drift would be invisible until someone ran both programs side by
//! side.

use nn_core::{Entry, Tree};

use crate::results::truncate;

/// How far a nested row is indented, in spaces.
const INDENT: &str = "  ";

/// The line above the rows: the panel's name and how many files it found.
///
/// A scan that stopped at the entry cap says `+` after the count, because a
/// number that is a floor rather than a total has to look like one. The count
/// is of files: a directory is a row you open, not a thing the panel found.
pub fn title(name: &str, tree: &Tree) -> String {
    let files = tree.file_count();
    let more = if tree.is_truncated() { "+" } else { "" };
    let noun = if files == 1 { "file" } else { "files" };
    format!("{name} ({files}{more} {noun})")
}

/// `entry` as one row of the panel, cut to `columns`.
///
/// A directory carries `-` when it is open and `+` when it is closed, which is
/// the whole of the disclosure affordance: a row is a line of text, so the
/// marker has to be text too. A file gets a matching pair of spaces, so a name
/// starts in the same column whichever kind of row it is on.
pub fn row(entry: &Entry, expanded: bool, columns: usize) -> String {
    let indent = INDENT.repeat(entry.depth as usize);
    let marker = if entry.dir {
        if expanded {
            "- "
        } else {
            "+ "
        }
    } else {
        "  "
    };
    truncate(&format!("{indent}{marker}{}", entry.name), columns)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(name: &str, dir: bool, depth: u8) -> Entry {
        Entry {
            path: PathBuf::from("/tmp").join(name),
            name: name.to_string(),
            dir,
            depth,
        }
    }

    /// A tree with one root and the `count` files it found under it.
    fn tree(count: usize) -> Tree {
        let root = std::env::temp_dir().join(format!("nn-workspace-shared-{count}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("mkdir");
        for index in 0..count {
            std::fs::write(root.join(format!("file{index}.txt")), "x").expect("write");
        }
        let tree = Tree::scan(std::slice::from_ref(&root));
        let _ = std::fs::remove_dir_all(&root);
        tree
    }

    #[test]
    fn a_directory_says_whether_it_is_open_and_a_file_does_not() {
        assert_eq!(row(&entry("src", true, 0), true, 40), "- src");
        assert_eq!(row(&entry("src", true, 0), false, 40), "+ src");
        assert_eq!(row(&entry("main.rs", false, 1), false, 40), "    main.rs");
        // A deeper row is indented once per level, and the marker sits after
        // the indent rather than at either end.
        assert_eq!(row(&entry("mod.rs", false, 2), false, 40), "      mod.rs");
    }

    #[test]
    fn a_long_name_is_cut_with_a_mark() {
        let long = entry("a_very_long_file_name.rs", false, 0);
        assert_eq!(row(&long, false, 8), "  a_ver…");
    }

    #[test]
    fn the_title_counts_the_files() {
        let empty = Tree::default();
        assert_eq!(title("Folder as Workspace", &empty), "Folder as Workspace (0 files)");
        // One file is a file, not one files: the count and the noun agree.
        assert_eq!(title("Project", &tree(1)), "Project (1 file)");
        assert_eq!(title("Project", &tree(3)), "Project (3 files)");
        // The `+` after a truncated count is not tested here: a tree is
        // truncated only by a scan that found ten thousand entries, and a test
        // that made them would spend its time in the filesystem to read one
        // character.
    }
}
