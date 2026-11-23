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

//! A directory tree, flattened to the rows a panel can draw.
//!
//! Notepad++ has two panels that show a tree of files: "Folder as Workspace",
//! which is one directory, and the three project panels, which are a named list
//! of directories. They differ in where the roots come from and in nothing else,
//! so they share this model: a `Tree` is a list of roots, a scan of everything
//! under them, and the expansion state that decides which of those entries are
//! currently visible.
//!
//! The tree is scanned once and flattened. Re-walking the filesystem on every
//! expand would be simpler, but it would also mean a panel that stutters on a
//! network mount and a scroll position that jumps when a directory changes
//! underneath it. A scan is a snapshot; `refresh` takes a new one.
//!
//! Rows are `Entry` values in depth-first order with a `depth`, not a nested
//! structure. A panel draws a flat list of rows with an indent, so a flat list
//! is what it should be given -- and a flat list is what a test can assert on
//! without walking a tree.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Directories that are never worth showing.
///
/// The same list find-in-files skips, for the same reason: a workspace panel
/// that opens onto `target/` is a panel the user scrolls past every time.
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "target",
    "node_modules",
    ".venv",
    "__pycache__",
    ".cache",
];

/// How deep a scan goes before it stops descending.
pub const MAX_DEPTH: u8 = 12;
/// How many entries a scan collects before it stops.
///
/// A workspace panel is a navigation aid, not a file manager. Ten thousand rows
/// is already more than anyone scrolls; past that the scan is spending time on
/// rows nobody will see.
pub const MAX_ENTRIES: usize = 10_000;

/// One row of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The absolute path this row stands for.
    pub path: PathBuf,
    /// The last path component, which is what the row displays.
    pub name: String,
    /// Whether the row is a directory.
    pub dir: bool,
    /// How far to indent it, zero for a root.
    pub depth: u8,
}

impl Entry {
    /// Whether this row can be expanded.
    pub fn is_dir(&self) -> bool {
        self.dir
    }
}

/// A scanned directory tree and its expansion state.
#[derive(Debug, Clone, Default)]
pub struct Tree {
    roots: Vec<PathBuf>,
    /// Every entry the scan found, in depth-first order.
    all: Vec<Entry>,
    /// Paths whose children are showing.
    expanded: HashSet<PathBuf>,
    /// The filter, lowercased. Empty means no filter.
    filter: String,
    /// Entries that match the filter, plus every ancestor of one.
    ///
    /// Precomputed when the filter changes rather than tested per row: a row is
    /// visible if it is in this set, and answering that by walking up to the
    /// root for every row would make filtering quadratic in tree depth.
    matched: HashSet<PathBuf>,
    /// Whether the scan hit `MAX_ENTRIES` and stopped early.
    truncated: bool,
}

impl Tree {
    /// Scans `roots` and returns the tree with every root expanded.
    ///
    /// Roots are expanded rather than collapsed because a panel that opens onto
    /// a single closed row has told the user nothing. One level of children is
    /// the useful default; deeper levels stay closed so the panel is a map
    /// rather than a dump.
    pub fn scan(roots: &[PathBuf]) -> Tree {
        let mut tree = Tree {
            roots: roots.to_vec(),
            ..Tree::default()
        };
        for root in &tree.roots {
            tree.expanded.insert(root.clone());
        }
        tree.rescan();
        tree
    }

    /// Re-reads the filesystem, keeping the expansion state.
    ///
    /// Expansion is keyed by path, so a refresh keeps every directory the user
    /// opened open -- including ones that were re-created, which is the
    /// behaviour a build that deletes and rewrites a directory needs.
    pub fn rescan(&mut self) {
        self.all.clear();
        self.truncated = false;
        let roots = self.roots.clone();
        for root in &roots {
            let name = root
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| root.display().to_string());
            self.all.push(Entry {
                path: root.clone(),
                name,
                dir: true,
                depth: 0,
            });
            self.walk(root, 1);
            if self.truncated {
                break;
            }
        }
        self.rebuild_matched();
    }

    fn walk(&mut self, dir: &Path, depth: u8) {
        // Too deep is not the same as too big: a tree that stops descending is
        // complete as far as the panel is concerned, while one that stops
        // collecting has rows missing and says so in its header.
        if depth > MAX_DEPTH {
            return;
        }
        if self.all.len() >= MAX_ENTRIES {
            self.truncated = true;
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut children: Vec<(PathBuf, bool)> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let path = e.path();
                let meta = e.metadata().ok()?;
                Some((path, meta.is_dir()))
            })
            .collect();
        // Directories first, then by name, case-insensitively. The same order a
        // file manager uses, and the order that makes a tree readable: the
        // folders are the structure, the files are the leaves.
        children.sort_by(|a, b| {
            b.1.cmp(&a.1).then_with(|| {
                let an = a.0.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let bn = b.0.file_name().and_then(|n| n.to_str()).unwrap_or("");
                an.to_lowercase().cmp(&bn.to_lowercase())
            })
        });
        for (path, is_dir) in children {
            if self.all.len() >= MAX_ENTRIES {
                self.truncated = true;
                return;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if is_dir && SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            self.all.push(Entry {
                path: path.clone(),
                name,
                dir: is_dir,
                depth,
            });
            if is_dir {
                self.walk(&path, depth + 1);
            }
        }
    }

    /// The roots the tree was scanned from.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Replaces the roots and rescans.
    pub fn set_roots(&mut self, roots: &[PathBuf]) {
        self.roots = roots.to_vec();
        self.expanded.clear();
        for root in &self.roots {
            self.expanded.insert(root.clone());
        }
        self.rescan();
    }

    /// Whether the last scan stopped at `MAX_ENTRIES`.
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }

    /// Every entry the scan found, visible or not.
    pub fn entries(&self) -> &[Entry] {
        &self.all
    }

    /// The rows to draw, in order.
    ///
    /// An entry is visible when every directory above it is expanded. The walk
    /// tracks that with a single "hide everything deeper than this" marker
    /// rather than a stack, because depth-first order guarantees that once a
    /// collapsed directory's children start, they are contiguous.
    pub fn rows(&self) -> Vec<&Entry> {
        let filtering = !self.filter.is_empty();
        let mut rows = Vec::new();
        let mut hide_below: Option<u8> = None;
        for entry in &self.all {
            if let Some(depth) = hide_below {
                if entry.depth > depth {
                    continue;
                }
                hide_below = None;
            }
            if filtering && !self.matched.contains(&entry.path) {
                continue;
            }
            rows.push(entry);
            // A filter shows the path to every match, so directories are
            // implicitly open while one is active. Collapsing a directory
            // mid-filter would hide the match the user is looking for.
            if entry.dir && !filtering && !self.expanded.contains(&entry.path) {
                hide_below = Some(entry.depth);
            }
        }
        rows
    }

    /// Whether `path` is expanded.
    pub fn is_expanded(&self, path: &Path) -> bool {
        self.expanded.contains(path)
    }

    /// Opens or closes `path`, returning whether it is now expanded.
    ///
    /// A file cannot be expanded, so toggling one is a no-op that reports
    /// `false` -- the caller uses the return value to decide whether the click
    /// was a disclosure or an open.
    pub fn toggle(&mut self, path: &Path) -> bool {
        if !self.expanded.remove(path) {
            let is_dir = self
                .all
                .iter()
                .any(|e| e.path == path && e.dir);
            if !is_dir {
                return false;
            }
            self.expanded.insert(path.to_path_buf());
            return true;
        }
        false
    }

    /// Opens every directory in the scan.
    pub fn expand_all(&mut self) {
        for entry in &self.all {
            if entry.dir {
                self.expanded.insert(entry.path.clone());
            }
        }
    }

    /// Closes every directory except the roots.
    pub fn collapse_all(&mut self) {
        let roots = self.roots.clone();
        self.expanded.clear();
        for root in roots {
            self.expanded.insert(root);
        }
    }

    /// Sets the filter, or clears it when `text` is empty.
    ///
    /// Matching is a case-insensitive substring on the file name, not a glob.
    /// The panel's filter field is for finding a file you can half-remember,
    /// and a half-remembered name is a fragment, not a pattern.
    pub fn set_filter(&mut self, text: &str) {
        self.filter = text.trim().to_lowercase();
        self.rebuild_matched();
    }

    /// The current filter, as typed.
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Recomputes which entries survive the filter.
    fn rebuild_matched(&mut self) {
        self.matched.clear();
        if self.filter.is_empty() {
            return;
        }
        // The roots always survive. A panel that goes completely blank when a
        // filter matches nothing looks broken; one that keeps its root row
        // looks like a tree with no matches, which is what it is.
        for root in &self.roots {
            self.matched.insert(root.clone());
        }
        // A directory survives when it matches itself or contains a match, so
        // the walk goes backwards: children are always after their parent in
        // depth-first order, so by the time a parent is examined its children
        // have already been marked.
        let mut keep: HashSet<PathBuf> = HashSet::new();
        for entry in self.all.iter().rev() {
            let hit = entry.name.to_lowercase().contains(&self.filter);
            if hit || (entry.dir && keep.contains(&entry.path)) {
                keep.insert(entry.path.clone());
                self.matched.insert(entry.path.clone());
                if let Some(parent) = entry.path.parent() {
                    keep.insert(parent.to_path_buf());
                    self.matched.insert(parent.to_path_buf());
                }
            }
        }
        // A directory that matched by name shows everything under it. Filtering
        // to `src` means "show me src", not "show me the row called src and
        // nothing inside it", so the subtree comes along.
        let mut open: Option<u8> = None;
        for entry in &self.all {
            if let Some(depth) = open {
                if entry.depth > depth {
                    self.matched.insert(entry.path.clone());
                    continue;
                }
                open = None;
            }
            if entry.dir && entry.name.to_lowercase().contains(&self.filter) {
                open = Some(entry.depth);
            }
        }
    }

    /// The number of files in the scan, which is what the panel's header counts.
    pub fn file_count(&self) -> usize {
        self.all.iter().filter(|e| !e.dir).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory that deletes itself, so a failing test does not leave
    /// rubbish in `/tmp` for the next run to trip over.
    ///
    /// Both paths are held: the root is what the tests scan, and the base is
    /// what has to go. Removing only the root left an empty
    /// `npp-workspace-<name>-<pid>` behind on every run, which is a hundred
    /// directories a run and was most of what filled `/tmp`.
    struct Scratch(PathBuf, PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let base = std::env::temp_dir().join(format!(
                "npp-workspace-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&base);
            // The scanned directory is a fixed child of the scratch base, so
            // the root row's name is the same in every test and the assertions
            // do not have to spell out a process id.
            let dir = base.join("root");
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir, base)
        }

        /// The directory the tree is scanned from.
        fn root(&self) -> PathBuf {
            self.0.clone()
        }

        fn file(&self, rel: &str) {
            let path = self.0.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, "x").unwrap();
        }

        fn dir(&self, rel: &str) {
            std::fs::create_dir_all(self.0.join(rel)).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }

    fn names(rows: &[&Entry]) -> Vec<String> {
        rows.iter().map(|e| e.name.clone()).collect()
    }

    #[test]
    fn a_scan_lists_directories_before_files() {
        let scratch = Scratch::new("order");
        scratch.file("b.txt");
        scratch.file("a.txt");
        scratch.dir("zdir");
        let tree = Tree::scan(std::slice::from_ref(&scratch.root()));
        let rows = tree.rows();
        // The root, then zdir, then the two files alphabetically.
        assert_eq!(names(&rows), vec!["root", "zdir", "a.txt", "b.txt"]);
    }

    #[test]
    fn roots_start_expanded_and_children_do_not() {
        let scratch = Scratch::new("expand");
        scratch.file("sub/deep.txt");
        let tree = Tree::scan(std::slice::from_ref(&scratch.root()));
        let rows = tree.rows();
        // The root is open, so `sub` shows; `sub` is closed, so `deep.txt`
        // does not.
        assert_eq!(names(&rows), vec!["root", "sub"]);
        assert!(tree.is_expanded(&scratch.0));
        assert!(!tree.is_expanded(&scratch.0.join("sub")));
    }

    #[test]
    fn toggling_a_directory_shows_its_children() {
        let scratch = Scratch::new("toggle");
        scratch.file("sub/deep.txt");
        let mut tree = Tree::scan(&[scratch.root()]);
        assert!(tree.toggle(&scratch.0.join("sub")));
        assert_eq!(names(&tree.rows()), vec!["root", "sub", "deep.txt"]);
        assert!(!tree.toggle(&scratch.0.join("sub")));
        assert_eq!(names(&tree.rows()), vec!["root", "sub"]);
    }

    #[test]
    fn toggling_a_file_reports_no_change() {
        let scratch = Scratch::new("togglefile");
        scratch.file("a.txt");
        let mut tree = Tree::scan(&[scratch.root()]);
        assert!(!tree.toggle(&scratch.0.join("a.txt")));
        assert_eq!(names(&tree.rows()), vec!["root", "a.txt"]);
    }

    #[test]
    fn collapsing_a_root_hides_everything_under_it() {
        let scratch = Scratch::new("collapse");
        scratch.file("sub/deep.txt");
        let mut tree = Tree::scan(&[scratch.root()]);
        tree.expand_all();
        assert_eq!(names(&tree.rows()), vec!["root", "sub", "deep.txt"]);
        assert!(!tree.toggle(&scratch.root()));
        assert_eq!(names(&tree.rows()), vec!["root"]);
    }

    #[test]
    fn collapse_all_leaves_the_roots_open() {
        let scratch = Scratch::new("collapseall");
        scratch.file("sub/deep.txt");
        let mut tree = Tree::scan(&[scratch.root()]);
        tree.expand_all();
        tree.collapse_all();
        // The root stays open so the panel still shows its top level; only the
        // directories below it close.
        assert_eq!(names(&tree.rows()), vec!["root", "sub"]);
    }

    #[test]
    fn skipped_directories_are_not_scanned() {
        let scratch = Scratch::new("skip");
        scratch.file("target/junk.txt");
        scratch.file("src/real.txt");
        let tree = Tree::scan(std::slice::from_ref(&scratch.root()));
        let all: Vec<String> = tree.entries().iter().map(|e| e.name.clone()).collect();
        assert!(!all.contains(&"target".to_string()));
        assert!(all.contains(&"src".to_string()));
    }

    #[test]
    fn a_filter_keeps_matches_and_their_ancestors() {
        let scratch = Scratch::new("filter");
        scratch.file("src/main.rs");
        scratch.file("src/other.txt");
        scratch.file("docs/readme.md");
        let mut tree = Tree::scan(&[scratch.root()]);
        tree.set_filter("MAIN");
        let rows = names(&tree.rows());
        // The root and `src` survive because they lead to the match; `docs`
        // and the two non-matching files do not.
        assert_eq!(rows, vec!["root", "src", "main.rs"]);
    }

    #[test]
    fn a_filter_matches_a_directory_name_too() {
        let scratch = Scratch::new("filterdir");
        scratch.file("src/main.rs");
        scratch.file("docs/readme.md");
        let mut tree = Tree::scan(&[scratch.root()]);
        tree.set_filter("doc");
        let rows = names(&tree.rows());
        assert_eq!(rows, vec!["root", "docs", "readme.md"]);
    }

    #[test]
    fn a_filter_shows_children_of_a_collapsed_directory() {
        let scratch = Scratch::new("filteropen");
        scratch.file("sub/deep.txt");
        let mut tree = Tree::scan(&[scratch.root()]);
        tree.collapse_all();
        tree.set_filter("deep");
        // `sub` is closed, but the filter opens the path to the match.
        assert_eq!(names(&tree.rows()), vec!["root", "sub", "deep.txt"]);
    }

    #[test]
    fn clearing_the_filter_restores_the_collapsed_view() {
        let scratch = Scratch::new("filterclear");
        scratch.file("sub/deep.txt");
        let mut tree = Tree::scan(&[scratch.root()]);
        tree.collapse_all();
        tree.set_filter("deep");
        tree.set_filter("");
        assert_eq!(names(&tree.rows()), vec!["root", "sub"]);
    }

    #[test]
    fn a_filter_that_matches_nothing_leaves_only_the_root() {
        let scratch = Scratch::new("filternone");
        scratch.file("a.txt");
        let mut tree = Tree::scan(&[scratch.root()]);
        tree.set_filter("zzz");
        // The root stays so the panel does not look broken, but nothing under
        // it survives.
        assert_eq!(names(&tree.rows()), vec!["root"]);
    }

    #[test]
    fn several_roots_are_scanned_in_order() {
        let one = Scratch::new("rootone");
        one.file("a.txt");
        let two = Scratch::new("roottwo");
        two.file("b.txt");
        let tree = Tree::scan(&[one.root(), two.root()]);
        let rows = names(&tree.rows());
        assert_eq!(rows, vec!["root", "a.txt", "root", "b.txt"]);
        assert_eq!(tree.roots().len(), 2);
    }

    #[test]
    fn a_rescan_keeps_the_expansion_state() {
        let scratch = Scratch::new("rescan");
        scratch.file("sub/deep.txt");
        let mut tree = Tree::scan(&[scratch.root()]);
        tree.toggle(&scratch.0.join("sub"));
        scratch.file("sub/added.txt");
        tree.rescan();
        let rows = names(&tree.rows());
        assert!(rows.contains(&"added.txt".to_string()));
        assert!(tree.is_expanded(&scratch.0.join("sub")));
    }

    #[test]
    fn a_missing_root_is_a_row_with_no_children() {
        let missing = std::env::temp_dir().join("npp-workspace-does-not-exist");
        let tree = Tree::scan(std::slice::from_ref(&missing));
        assert_eq!(tree.rows().len(), 1);
        assert_eq!(tree.file_count(), 0);
    }

    #[test]
    fn the_file_count_ignores_directories() {
        let scratch = Scratch::new("count");
        scratch.file("a.txt");
        scratch.file("b.txt");
        scratch.dir("sub");
        let tree = Tree::scan(std::slice::from_ref(&scratch.root()));
        assert_eq!(tree.file_count(), 2);
    }

    #[test]
    fn setting_roots_replaces_the_scan() {
        let one = Scratch::new("setone");
        one.file("a.txt");
        let two = Scratch::new("settwo");
        two.file("b.txt");
        let mut tree = Tree::scan(&[one.root()]);
        tree.set_roots(&[two.root()]);
        assert_eq!(names(&tree.rows()), vec!["root", "b.txt"]);
    }

    #[test]
    fn an_empty_tree_has_no_rows() {
        let tree = Tree::default();
        assert!(tree.rows().is_empty());
        assert_eq!(tree.file_count(), 0);
        assert!(!tree.is_truncated());
    }
}
