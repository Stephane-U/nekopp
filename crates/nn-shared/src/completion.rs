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

//! What a completion list decides, on either front-end.
//!
//! The window draws the list as shaped glyphs in a floating box and the
//! terminal draws it as cells in a box of its own, but which words are offered,
//! what accepting one inserts, how far the caret steps back, how many rows show
//! at once, and which keys refilter are one set of decisions. They live here so
//! a fix to the plural in a label or to the wrapping at the top of the list
//! cannot land in one front-end and be forgotten in the other.
//!
//! The engine does the searching: `nn_core` collects the words and matches a
//! prefix against them. What is here is the composition -- take the prefix at
//! the caret, ask the engine for candidates, turn them into rows -- and the
//! list's own state, which is a cursor and a scroll offset.

use std::path::Path;

use nn_core::paths::{complete_path, resolve_dir, split_fragment};
use nn_core::{
    collect_words, collect_words_many, matching_words, path_prefix, prefix_range, scan_functions,
};

/// The most rows the list shows at once.
///
/// Ten, in both front-ends: a prefix that matches two hundred words still has to
/// give a box you can read, and the reader is choosing from it, not studying it.
pub const VISIBLE_ROWS: usize = 10;

/// Which completion the list is serving.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CompletionMode {
    /// Words already in the active document.
    Word,
    /// Words from every open document.
    Document,
    /// Function declarations in the active document.
    Function,
    /// Filesystem paths.
    Path,
}

impl CompletionMode {
    /// The word the status bar uses while the list is up.
    pub fn label(self) -> &'static str {
        match self {
            CompletionMode::Word => "Word completion",
            CompletionMode::Document => "Document completion",
            CompletionMode::Function => "Function completion",
            CompletionMode::Path => "Path completion",
        }
    }

    /// Whether typing `c` should refilter the list.
    ///
    /// Path completion refilters on every character, because a filename can hold
    /// dots and dashes that are not word characters. The other three only
    /// refilter on word characters: typing `(` after a half-typed word means you
    /// are done with the word, not that the word grew.
    pub fn refilters_on(self, c: char) -> bool {
        match self {
            CompletionMode::Path => !c.is_whitespace(),
            _ => c.is_alphanumeric() || c == '_',
        }
    }
}

/// One row of the completion list.
///
/// A row is not a string because the four completion modes disagree about what
/// accepting one means. Word completion inserts the word it shows. Function
/// completion shows `save(path: &Path)` but inserts `save()` and leaves the
/// caret between the parentheses, because the parameters are a reminder, not
/// text you asked for. Keeping the label and the insertion apart is what lets
/// one list serve all of them.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Item {
    /// What the row shows.
    pub label: String,
    /// What accepting the row inserts.
    pub insert: String,
    /// How far to move the caret back after inserting.
    pub back: usize,
}

impl Item {
    /// A row that inserts exactly what it shows.
    pub fn word(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            label: text.clone(),
            insert: text,
            back: 0,
        }
    }

    /// A row for a function: shows the signature, inserts the call.
    pub fn function(name: &str, params: &str) -> Self {
        Self {
            label: format!("{name}({params})"),
            insert: format!("{name}()"),
            back: 1,
        }
    }
}

/// The list itself: its rows, the cursor, and how far it is scrolled.
///
/// Drawn by whichever front-end owns it; this type never measures anything.
#[derive(Clone, Debug, Default)]
pub struct Candidates {
    items: Vec<Item>,
    /// Index into `items` of the highlighted row.
    selected: usize,
    /// Index of the first visible row.
    scroll: usize,
    open: bool,
}

impl Candidates {
    /// An empty, closed list.
    pub fn new() -> Self {
        Self::default()
    }

    /// Shows `items`. An empty list stays closed: a box with nothing in it says
    /// the wrong thing about what was found.
    pub fn show(&mut self, items: Vec<Item>) {
        self.open = !items.is_empty();
        self.items = items;
        self.selected = 0;
        self.scroll = 0;
    }

    /// Hides the list and drops the rows.
    pub fn close(&mut self) {
        self.open = false;
        self.items.clear();
        self.selected = 0;
        self.scroll = 0;
    }

    /// Whether the list is showing.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The rows, for a front-end that draws them.
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// The row at `index`, if the list is that long.
    pub fn item(&self, index: usize) -> Option<&Item> {
        self.items.get(index)
    }

    /// The highlighted row, for the accept handler.
    pub fn selected(&self) -> Option<&Item> {
        self.items.get(self.selected)
    }

    /// The label of the highlighted row, for a test or a status line.
    pub fn selected_label(&self) -> Option<&str> {
        self.selected().map(|item| item.label.as_str())
    }

    /// Which row is highlighted.
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// The first visible row.
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// The number of candidates.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether there is nothing to show.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The most rows a list ever shows.
    pub fn visible_rows(&self) -> usize {
        VISIBLE_ROWS
    }

    /// Moves the highlight by `delta` rows, wrapping at both ends.
    ///
    /// Wrapping rather than clamping: `Up` on the first row should reach the
    /// last one, which is what every completion list does.
    pub fn move_selection(&mut self, delta: isize) {
        if self.items.is_empty() {
            return;
        }
        let n = self.items.len() as isize;
        self.selected = (((self.selected as isize + delta) % n + n) % n) as usize;
        self.scroll_to_selection();
    }

    /// Highlights `index`, for a click on the list.
    pub fn set_selected(&mut self, index: usize) {
        if index < self.items.len() {
            self.selected = index;
            self.scroll_to_selection();
        }
    }

    /// Keeps the highlighted row inside the visible window.
    fn scroll_to_selection(&mut self) {
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + VISIBLE_ROWS {
            self.scroll = self.selected + 1 - VISIBLE_ROWS;
        }
    }
}

/// The rows word completion offers for the word before `caret`.
///
/// Empty when the caret is not at the end of a word: completion replaces the run
/// of word characters that ends *at* the caret, so a caret in the middle of one
/// has nothing to complete.
pub fn word_items(text: &str, caret: usize, limit: usize) -> Vec<Item> {
    let Some(range) = prefix_range(text, caret) else {
        return Vec::new();
    };
    let prefix = &text[range];
    // The whole document is collected rather than only what precedes the caret:
    // a word used further down is still a word the reader has in mind, and the
    // engine's collection is one pass over a text the editor already holds.
    let words = collect_words(text, 20_000);
    matching_items(prefix, &words, limit)
}

/// The rows document completion offers: the same words, from every open
/// document.
///
/// `text` is the document being typed in, which is where the prefix comes from;
/// `pool` is the words of every open document, gathered by the front-end because
/// it is the one that knows which documents exist.
pub fn pooled_items(text: &str, caret: usize, pool: &[&str], limit: usize) -> Vec<Item> {
    let Some(range) = prefix_range(text, caret) else {
        return Vec::new();
    };
    let prefix = &text[range];
    let words = collect_words_many(pool, 20_000);
    matching_items(prefix, &words, limit)
}

/// The rows function completion offers: declarations in `text` whose names start
/// with the word before `caret`.
///
/// A function whose name is exactly what is typed is not offered: the reader has
/// finished that name, and `save(save(...))` is not what a completion is for.
pub fn function_items(text: &str, caret: usize, limit: usize) -> Vec<Item> {
    let Some(range) = prefix_range(text, caret) else {
        return Vec::new();
    };
    let prefix = text[range].to_lowercase();
    let mut seen: Vec<String> = Vec::new();
    let mut items: Vec<Item> = Vec::new();
    for function in scan_functions(text) {
        let name = function.name.to_lowercase();
        if name == prefix || !name.starts_with(&prefix) || seen.contains(&function.name) {
            continue;
        }
        seen.push(function.name.clone());
        items.push(Item::function(&function.name, &function.params));
        if items.len() >= limit {
            break;
        }
    }
    items
}

/// The rows path completion offers for the fragment before `caret`.
///
/// `base` is the directory a relative fragment is measured from, which only the
/// front-end knows: both pass the directory of the document in front, so `src/ma`
/// in `/home/me/project/main.rs` lists `/home/me/project/src`. A directory row
/// carries its separator in the label, so taking one leaves the caret where the
/// next step of the path starts rather than at the end of a word.
pub fn path_items(text: &str, caret: usize, base: Option<&Path>, limit: usize) -> Vec<Item> {
    let Some(range) = path_prefix(text, caret) else {
        return Vec::new();
    };
    let fragment = text.get(range).unwrap_or_default();
    let (dir, stem) = split_fragment(fragment);
    let resolved = resolve_dir(base, dir);
    complete_path(&resolved, stem, limit)
        .into_iter()
        .map(|name| Item {
            label: name.clone(),
            insert: format!("{dir}{name}"),
            back: 0,
        })
        .collect()
}

/// The rows for `prefix` out of `words`, as list rows.
fn matching_items(prefix: &str, words: &[String], limit: usize) -> Vec<Item> {
    matching_words(prefix, words, limit)
        .into_iter()
        .map(Item::word)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(names: &[&str]) -> Vec<Item> {
        names.iter().map(|name| Item::word(*name)).collect()
    }

    #[test]
    fn a_new_list_is_closed_and_showing_items_opens_it() {
        let mut list = Candidates::new();
        assert!(!list.is_open());
        list.show(words(&["alpha", "beta"]));
        assert!(list.is_open());
        assert_eq!(list.len(), 2);
        assert_eq!(list.selected_label(), Some("alpha"));
        // Nothing found is a closed list rather than an empty box.
        list.show(Vec::new());
        assert!(!list.is_open());
    }

    #[test]
    fn the_selection_wraps_at_both_ends() {
        let mut list = Candidates::new();
        list.show(words(&["a", "b", "c"]));
        list.move_selection(-1);
        assert_eq!(list.selected_label(), Some("c"));
        list.move_selection(1);
        assert_eq!(list.selected_label(), Some("a"));
        list.move_selection(2);
        assert_eq!(list.selected_label(), Some("c"));
        // A closed list has nothing to move through.
        let mut closed = Candidates::new();
        closed.move_selection(1);
        assert_eq!(closed.selected_label(), None);
    }

    #[test]
    fn the_selection_scrolls_into_view() {
        let names: Vec<String> = (0..20).map(|n| format!("word{n:02}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut list = Candidates::new();
        list.show(words(&refs));
        assert_eq!(list.scroll(), 0);
        for _ in 0..VISIBLE_ROWS {
            list.move_selection(1);
        }
        // The eleventh row is one past the window, so the window moves by one.
        assert_eq!(list.selected_index(), VISIBLE_ROWS);
        assert_eq!(list.scroll(), 1);
        // And back up again follows it.
        for _ in 0..VISIBLE_ROWS {
            list.move_selection(-1);
        }
        assert_eq!(list.scroll(), 0);
    }

    #[test]
    fn set_selected_highlights_a_row_within_the_list() {
        let mut list = Candidates::new();
        list.show(words(&["a", "b", "c"]));
        list.set_selected(2);
        assert_eq!(list.selected_label(), Some("c"));
        // An index past the end is not a selection.
        list.set_selected(9);
        assert_eq!(list.selected_label(), Some("c"));
        assert_eq!(list.item(1).map(|item| item.label.as_str()), Some("b"));
        assert!(list.item(9).is_none());
    }

    #[test]
    fn closing_drops_the_rows() {
        let mut list = Candidates::new();
        list.show(words(&["a"]));
        list.close();
        assert!(!list.is_open());
        assert!(list.is_empty());
        assert_eq!(list.selected(), None);
    }

    #[test]
    fn a_word_row_inserts_what_it_shows_and_a_function_row_inserts_the_call() {
        let word = Item::word("document");
        assert_eq!(word.label, "document");
        assert_eq!(word.insert, "document");
        assert_eq!(word.back, 0);
        let call = Item::function("save", "path: &Path");
        assert_eq!(call.label, "save(path: &Path)");
        assert_eq!(call.insert, "save()");
        assert_eq!(call.back, 1);
        // A function with no parameters still inserts its parentheses, because
        // that is where the arguments would go.
        let bare = Item::function("now", "");
        assert_eq!(bare.label, "now()");
        assert_eq!(bare.insert, "now()");
    }

    #[test]
    fn every_mode_has_a_label_and_its_own_refilter_rule() {
        assert_eq!(CompletionMode::Word.label(), "Word completion");
        assert_eq!(CompletionMode::Document.label(), "Document completion");
        assert_eq!(CompletionMode::Function.label(), "Function completion");
        assert_eq!(CompletionMode::Path.label(), "Path completion");
        // The three word modes refilter on word characters only: a `(` ends the
        // word rather than growing it.
        for mode in [
            CompletionMode::Word,
            CompletionMode::Document,
            CompletionMode::Function,
        ] {
            assert!(mode.refilters_on('a'));
            assert!(mode.refilters_on('_'));
            assert!(mode.refilters_on('7'));
            assert!(!mode.refilters_on('('));
            assert!(!mode.refilters_on(' '));
        }
        // Path completion refilters on the punctuation a filename can hold.
        assert!(CompletionMode::Path.refilters_on('.'));
        assert!(CompletionMode::Path.refilters_on('-'));
        assert!(!CompletionMode::Path.refilters_on(' '));
    }

    #[test]
    fn word_completion_offers_the_words_that_extend_the_prefix() {
        let text = "document the documenter, Document and doc\n";
        // The caret is just after `doc`.
        let caret = text.find("doc\n").expect("the word") + 3;
        let items = word_items(text, caret, 200);
        let labels: Vec<&str> = items.iter().map(|item| item.label.as_str()).collect();
        // Case-exact matches first, then the case-insensitive ones, and the word
        // being typed is never offered as its own completion.
        assert_eq!(labels, vec!["document", "documenter", "Document"]);
    }

    #[test]
    fn completion_extends_the_run_of_word_characters_that_ends_at_the_caret() {
        let text = "document\n";
        // A caret inside a word extends the part before it, which is what the
        // reader sees in front of them.
        let items = word_items(text, 3, 200);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "document");
        // After the newline there is no word to extend, and a field of spaces is
        // not a prefix either.
        assert!(word_items(text, text.len(), 200).is_empty());
        assert!(word_items("a \nb\n", 3, 200).is_empty());
    }

    #[test]
    fn document_completion_pools_the_words_of_every_document() {
        let active = "sav\n";
        let pool = ["save\n", "saved = 1\n"];
        let items = pooled_items(active, 3, &pool, 200);
        let labels: Vec<&str> = items.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(labels, vec!["save", "saved"]);
    }

    #[test]
    fn function_completion_shows_the_signature_and_skips_a_name_already_typed() {
        let text = concat!(
            "fn save(path: &Path) {}\n",
            "fn saved(path: &Path) {}\n",
            "fn other() {}\n",
            "save\n",
        );
        let caret = text.len() - 1;
        let items = function_items(text, caret, 200);
        let labels: Vec<&str> = items.iter().map(|item| item.label.as_str()).collect();
        // `save` is what is typed, so it is not offered; `saved` is.
        assert_eq!(labels, vec!["saved(path: &Path)"]);
        assert_eq!(items[0].insert, "saved()");
    }

    /// A scratch directory that removes itself, so the path test does not leak.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("npp-completion-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch directory");
            Self(dir)
        }

        fn file(&self, name: &str) {
            std::fs::write(self.0.join(name), b"x").expect("scratch file");
        }

        fn dir(&self, name: &str) {
            std::fs::create_dir_all(self.0.join(name)).expect("scratch subdirectory");
        }

        /// A file inside a subdirectory, which is what a path fragment with a
        /// directory part in it is listing.
        fn file_in(&self, dir: &str, name: &str) {
            let dir = self.0.join(dir);
            std::fs::create_dir_all(&dir).expect("scratch subdirectory");
            std::fs::write(dir.join(name), b"x").expect("scratch file");
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn path_completion_lists_the_directory_and_splices_the_step_back_in() {
        let scratch = Scratch::new("paths");
        scratch.file("main.rs");
        scratch.file("other.txt");
        scratch.dir("src");
        scratch.file_in("src", "mod.rs");

        // A fragment has to carry a separator to be a path at all -- a bare word
        // is a word, and word completion is what offers those.
        assert!(path_items("open ma", 7, Some(&scratch.0), 200).is_empty());

        // With one, the row is the name and the insert is the name behind the
        // directory part the fragment already had.
        let text = "open ./ma";
        let items = path_items(text, text.len(), Some(&scratch.0), 200);
        let labels: Vec<&str> = items.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(labels, vec!["main.rs"]);
        assert_eq!(items[0].insert, "./main.rs");
        assert_eq!(items[0].back, 0);

        // A directory carries its separator in the label and in the insert, so
        // taking it leaves the caret ready for the next step of the path.
        let text = "open ./sr";
        let items = path_items(text, text.len(), Some(&scratch.0), 200);
        assert_eq!(items[0].label, "src/");
        assert_eq!(items[0].insert, "./src/");

        // And a fragment with a directory part in it is resolved against the
        // document rather than against the working directory of the process.
        let text = "open src/m";
        let items = path_items(text, text.len(), Some(&scratch.0), 200);
        assert_eq!(items[0].label, "mod.rs");
        assert_eq!(items[0].insert, "src/mod.rs");

        // A directory that is not there is a closed list rather than a failure.
        let missing = scratch.0.join("nowhere/");
        assert!(path_items("open nowhere/x", 14, Some(&missing), 200).is_empty());
    }

    #[test]
    fn a_list_of_candidates_is_capped_by_the_caller() {
        let text: String = (0..50).map(|n| format!("word{n:02} ")).collect();
        let items = word_items(&format!("{text}word"), text.len() + 4, 3);
        assert_eq!(items.len(), 3);
    }
}
