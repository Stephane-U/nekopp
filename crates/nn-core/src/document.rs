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

//! The document model.
//!
//! A `Document` is a `ropey::Rope` plus the arrays Scintilla keeps alongside its
//! text: a style byte per character, and a fold level and marker mask per line.
//!
//! Styles are per character because that is what syntax highlighting needs — a
//! line is not one colour, it is a run of them. Fold levels and markers are per
//! line because folding and bookmarks are line-granular.
//!
//! Text is stored with `\n` terminators only. The document remembers the
//! line-ending mode it was loaded with and restores it on save.

use std::ops::Range;
use std::path::{Path, PathBuf};

use ropey::{Rope, RopeBuilder};

use crate::encoding::Encoding;
use crate::eol::EolMode;

/// Lexer style index for a character. `0` is "no styling applied yet".
pub type Style = u8;

/// Fold level for a line, using Scintilla's bit layout.
pub type FoldLevel = u16;

/// Marker bitmask for a line.
pub type Marker = u32;

/// The size, in characters, above which a document opens in large-file mode.
///
/// Sixteen million characters is roughly a 16 MB text file. Above it the editor
/// stops doing the two things that scale with the whole document — syntax
/// highlighting and fold computation — because both are O(file) and both are
/// invisible on a file you are scrolling through rather than reading.
pub const LARGE_FILE_CHARS: usize = 16 * 1024 * 1024;

/// What to do about a file that changed on disk.
///
/// The policy lives here rather than in the event loop so it can be tested: the
/// one thing this editor must never do is throw away an unsaved edit because
/// another program wrote the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskChange {
    /// Nothing changed, or there is no file to watch.
    None,
    /// The buffer is clean, so the new text can be adopted.
    Reload,
    /// The buffer has unsaved edits. Flag it and leave the text alone.
    Conflict,
}

/// What a file looked like when it was last read or written.
///
/// Modification time plus length, because a file can be rewritten twice within
/// the same second on a filesystem with one-second timestamps, and the length is
/// the cheap half of the check that catches the second write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    /// The file's modification time, when the platform reports one.
    pub modified: Option<std::time::SystemTime>,
    /// The file's length in bytes.
    pub len: u64,
}

impl FileStamp {
    /// Reads the stamp of `path`, or `None` when it cannot be stat'd.
    ///
    /// A file that has been deleted reads as `None`, which compares unequal to
    /// any recorded stamp — so a deletion is noticed the same way a write is.
    pub fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        Some(Self {
            modified: meta.modified().ok(),
            len: meta.len(),
        })
    }
}

/// Sentinel in `row_of` for a line hidden inside a collapsed block.
const HIDDEN: usize = usize::MAX;

/// Style index meaning "unstyled".
pub const STYLE_DEFAULT: Style = 0;

/// Scintilla's `SC_FOLDLEVELBASE`: the level of an unfolded top-level line.
pub const FOLD_LEVEL_BASE: FoldLevel = 0x400;
/// Scintilla's `SC_FOLDLEVELWHITEFLAG`: the line is blank.
pub const FOLD_LEVEL_WHITEFLAG: FoldLevel = 0x1000;
/// Scintilla's `SC_FOLDLEVELHEADERFLAG`: the line starts a foldable block.
pub const FOLD_LEVEL_HEADERFLAG: FoldLevel = 0x2000;
/// Mask covering the fold depth bits.
pub const FOLD_LEVEL_NUMBERMASK: FoldLevel = 0x0FFF;

/// Marker bit for a bookmark.
pub const BOOKMARK: Marker = 1 << 0;

/// The fold depth encoded in `level`, with the flag bits removed.
pub const fn fold_number(level: FoldLevel) -> FoldLevel {
    level & FOLD_LEVEL_NUMBERMASK
}

/// Whether `level` marks the start of a foldable block.
pub const fn is_fold_header(level: FoldLevel) -> bool {
    level & FOLD_LEVEL_HEADERFLAG != 0
}

/// Whether `level` marks a blank line.
pub const fn is_fold_white(level: FoldLevel) -> bool {
    level & FOLD_LEVEL_WHITEFLAG != 0
}

/// An open text buffer.
#[derive(Debug, Clone)]
pub struct Document {
    rope: Rope,
    styles: Vec<Style>,
    fold_levels: Vec<FoldLevel>,
    markers: Vec<Marker>,
    lex_states: Vec<u8>,
    /// Per line: the brace depth at the start of the line, so the folder can
    /// resume from an edit instead of from the top of the file.
    fold_depths: Vec<u16>,
    /// Per line: is the block that starts here collapsed?
    folded: Vec<bool>,
    /// Row (visible line) to `(document line, wrapped segment)`. Rebuilt lazily
    /// after a fold change, an edit, or a change of wrap width, and empty when
    /// there is nothing to say: with no wrapping and no collapsed block, row `n`
    /// is line `n`, and the three vectors here would only spell that out.
    rows: Vec<(usize, u16)>,
    /// Document line to its first row, or `HIDDEN` when the line is inside a
    /// collapsed block. Empty means every line is on its own row.
    row_of: Vec<usize>,
    /// Per line: how many rows it occupies once wrapped. Empty means one.
    row_count: Vec<u16>,
    /// Per line: its length in characters, excluding the terminator. Kept here
    /// because wrapping needs every line's length and reaching into the rope for
    /// each one is a tree descent.
    line_lens: Vec<u32>,
    /// Columns to wrap at, or 0 for no wrapping.
    wrap_cols: usize,
    rows_dirty: bool,
    eol: EolMode,
    encoding: Encoding,
    path: Option<PathBuf>,
    /// The file's state when it was last read or written, so a change made by
    /// another program can be told apart from our own save.
    stamp: Option<FileStamp>,
    /// Set when the file changed underneath a buffer with unsaved edits. The
    /// buffer is deliberately not reloaded: the user's work wins until they say
    /// otherwise.
    disk_conflict: bool,
    dirty: bool,
    /// When set, the three editing entry points below refuse to change anything.
    /// Loading and reloading go through `set_text`, which is deliberately not
    /// guarded: a read-only buffer still has to be able to come from disk.
    read_only: bool,
    revision: u64,
    /// A process-unique id, so a cache keyed on `(id, revision)` cannot confuse
    /// two documents that happen to be at the same revision.
    ///
    /// The revision alone was not enough: two files loaded from disk are both
    /// at revision 0, so a view that cached shaped lines by revision showed the
    /// previous tab's text after a switch. The id is what makes the pair
    /// unique; the revision is what makes it change on an edit.
    id: u64,
}

/// The next document id.
///
/// Relaxed ordering is enough: the counter only has to hand out distinct
/// numbers, and nothing is published through it.
static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    /// An empty document: one line, no content.
    pub fn new() -> Self {
        Self::from_text("")
    }

    /// Builds a document from text that is already `\n`-terminated.
    pub fn from_text(text: &str) -> Self {
        Self::from_rope(Rope::from_str(text), Encoding::Utf8, EolMode::Lf)
    }

    /// The one constructor: every field the text decides, from a rope.
    fn from_rope(rope: Rope, encoding: Encoding, eol: EolMode) -> Self {
        let chars = rope.len_chars();
        let lines = rope.len_lines();
        let line_lens: Vec<u32> = (0..lines)
            .map(|line| {
                let slice = rope.line(line);
                let len = match slice.as_str() {
                    Some(text) => text.trim_end_matches(['\n', '\r']).chars().count(),
                    None => slice.len_chars(),
                };
                len as u32
            })
            .collect();
        Self {
            rope,
            styles: vec![STYLE_DEFAULT; chars],
            fold_levels: vec![FOLD_LEVEL_BASE; lines],
            markers: vec![0; lines],
            lex_states: vec![0; lines],
            fold_depths: vec![0; lines],
            folded: vec![false; lines],
            rows: Vec::new(),
            row_of: Vec::new(),
            row_count: Vec::new(),
            line_lens,
            wrap_cols: 0,
            rows_dirty: false,
            eol,
            encoding,
            path: None,
            stamp: None,
            disk_conflict: false,
            dirty: false,
            read_only: false,
            revision: 0,
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// Builds a document from raw file bytes, detecting encoding and line endings.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let encoding = Encoding::detect(bytes);
        let decoded = encoding.decode(bytes);
        let (text, eol) = EolMode::normalise(&decoded);
        Self::from_rope(Rope::from_str(&text), encoding, eol)
    }

    /// Reads a file from disk.
    ///
    /// Streamed through one buffer rather than read whole and copied three
    /// times: `stream` builds the rope straight from the file, and only a file
    /// that is not UTF-8 -- a UTF-16 one, a code page, or something that is not
    /// text at all -- falls back to reading it into memory, because telling
    /// those apart needs the bytes. See `Document::from_bytes`.
    pub fn from_file(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        let (rope, encoding, eol) = match stream(path) {
            Ok(streamed) => streamed,
            Err(_) => {
                let bytes = std::fs::read(path)?;
                let encoding = Encoding::detect(&bytes);
                let decoded = encoding.decode(&bytes);
                let (text, eol) = EolMode::normalise(&decoded);
                (Rope::from_str(&text), encoding, eol)
            }
        };
        let mut doc = Self::from_rope(rope, encoding, eol);
        doc.path = Some(path.to_path_buf());
        doc.stamp = FileStamp::of(path);
        Ok(doc)
    }

    /// Reads a file, or makes the blank page for a path that does not exist yet.
    ///
    /// `npp notes.txt` in a directory where `notes.txt` is not there opens an
    /// empty buffer called `notes.txt`, and the first save creates the file.
    /// Every editor does this; the alternative -- a status message and no tab --
    /// turns a name the user has not used yet into a dead end.
    ///
    /// Only absence counts. A path that exists and cannot be read -- a
    /// directory, a file with no permission -- is still an error: replacing it
    /// with an empty buffer would offer to overwrite something the user never
    /// got to see.
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        match Self::from_file(path) {
            Ok(doc) => Ok(doc),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let mut doc = Self::new();
                doc.path = Some(path.to_path_buf());
                // The stamp of a file that is not there is `None`, and so is the
                // stamp of a buffer that has never been written. The two agree,
                // which is what stops the change monitor from reporting the file
                // as deleted on the frame the tab appears, and what stops the
                // buffer from carrying a dirty mark the user did not make.
                doc.stamp = FileStamp::of(path);
                Ok(doc)
            }
            Err(err) => Err(err),
        }
    }

    /// Writes the document back to its path.
    pub fn save(&mut self) -> std::io::Result<()> {
        let path = self.path.clone().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "document has no path")
        })?;
        self.save_as(path)
    }

    /// Writes the document to `path` and adopts it as the document's path.
    pub fn save_as(&mut self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let path = path.as_ref();
        let text = self.text_with_eol(self.eol);
        std::fs::write(path, self.encoding.encode(&text))?;
        self.path = Some(path.to_path_buf());
        self.stamp = FileStamp::of(path);
        self.disk_conflict = false;
        self.dirty = false;
        Ok(())
    }

    /// Clears the dirty flag when the text already matches the file on disk.
    ///
    /// Returns whether it cleared. The comparison is against a fresh read of the
    /// file rather than against a remembered copy, so the flag can only be
    /// cleared when the claim is true: a buffer whose text differs from disk
    /// stays dirty and keeps its marker.
    ///
    /// This exists for the snapshot restore. A buffer is snapshotted *because*
    /// it was dirty, but by the time it is restored its text can be identical to
    /// the file -- the user undid the edit, or saved it from another program.
    /// Restoring it as dirty would put a permanent unsaved marker on a file with
    /// nothing unsaved, and would keep the buffer in the snapshot for every
    /// session after, so the "unsaved work" would never go away.
    ///
    /// The stamp is refreshed along with the flag, because a document that
    /// matches its file should not also be reported as changed on disk.
    pub fn mark_clean_if_unchanged(&mut self) -> bool {
        let Some(path) = self.path.clone() else {
            return false;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            return false;
        };
        // Compared as text rather than as bytes: the file may use CRLF and the
        // rope never does, so a byte comparison would call an untouched file
        // modified on every Windows-style document.
        if Self::from_bytes(&bytes).text() != self.text() {
            return false;
        }
        self.dirty = false;
        self.stamp = FileStamp::of(&path);
        true
    }

    // ---- geometry -------------------------------------------------------

    /// Number of characters in the document.
    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    /// Number of lines. A document ending in a terminator has a final empty line.
    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    /// The line containing `char_idx`.
    pub fn char_to_line(&self, char_idx: usize) -> usize {
        self.rope.char_to_line(char_idx.min(self.rope.len_chars()))
    }

    /// The character index of byte offset `byte`.
    ///
    /// The search code works in bytes because that is what `str::find` returns;
    /// the view works in characters because that is what a column is.
    pub fn byte_to_char(&self, byte: usize) -> usize {
        self.rope.byte_to_char(byte.min(self.rope.len_bytes()))
    }

    /// The byte offset of character index `char`.
    ///
    /// The mirror of [`Self::byte_to_char`], for the one caller that has to
    /// hand the editor's character offsets to something outside it: the plugin
    /// host, whose every offset is a byte because the plugin measures the UTF-8
    /// it is given.
    pub fn char_to_byte(&self, char_idx: usize) -> usize {
        self.rope.char_to_byte(char_idx.min(self.rope.len_chars()))
    }

    /// The first character index of `line`.
    pub fn line_to_char(&self, line: usize) -> usize {
        self.rope.line_to_char(line.min(self.rope.len_lines() - 1))
    }

    /// The character range of `line`'s content, excluding its terminator.
    ///
    /// The length comes from the line's own slice rather than from the next
    /// line's start: `Rope::char` walks a chunk to reach an index, and this is
    /// called once per line by the minimap and the highlighter.
    pub fn line_range(&self, line: usize) -> Range<usize> {
        let line = line.min(self.rope.len_lines() - 1);
        let start = self.rope.line_to_char(line);
        let slice = self.rope.line(line);
        let len = match slice.as_str() {
            Some(text) => text.trim_end_matches(['\n', '\r']).chars().count(),
            None => {
                let mut len = slice.len_chars();
                while len > 0 {
                    match slice.char(len - 1) {
                        '\n' | '\r' => len -= 1,
                        _ => break,
                    }
                }
                len
            }
        };
        start..start + len
    }

    /// The content of `line`, without its terminator.
    pub fn line(&self, line: usize) -> String {
        self.rope.slice(self.line_range(line)).to_string()
    }

    /// The number of characters in `line`, excluding its terminator.
    pub fn line_len(&self, line: usize) -> usize {
        match self.line_lens.get(line) {
            Some(&len) => len as usize,
            None => {
                let r = self.line_range(line);
                r.end - r.start
            }
        }
    }

    /// The character at `char_idx`, if it is in bounds.
    pub fn char_at(&self, char_idx: usize) -> Option<char> {
        if char_idx < self.rope.len_chars() {
            Some(self.rope.char(char_idx))
        } else {
            None
        }
    }

    /// The whole word the caret is inside or touching.
    ///
    /// Walks outward from the caret one character at a time rather than copying
    /// the document and scanning it, because `$(CURRENT_WORD)` is expanded on
    /// every Run and a 100 MB file should not be copied to answer a question
    /// about four characters.
    ///
    /// A word is what the completion index calls a word: alphanumeric or an
    /// underscore. Punctuation is not part of it, so `foo.bar` with the caret on
    /// `bar` gives `bar`.
    pub fn word_at(&self, char_idx: usize) -> Option<String> {
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        if !self.char_at(char_idx).is_some_and(is_word) {
            return None;
        }
        let mut start = char_idx;
        while start > 0 && self.char_at(start - 1).is_some_and(is_word) {
            start -= 1;
        }
        let mut end = char_idx;
        while self.char_at(end).is_some_and(is_word) {
            end += 1;
        }
        Some(self.slice(start..end))
    }

    /// A copy of the characters in `range`.
    pub fn slice(&self, range: Range<usize>) -> String {
        let start = range.start.min(self.rope.len_chars());
        let end = range.end.min(self.rope.len_chars());
        if start >= end {
            return String::new();
        }
        self.rope.slice(start..end).to_string()
    }

    /// The whole document with `\n` terminators.
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// The first `chars` characters of the document, as text.
    ///
    /// For a reader that cannot take all of it: the accessibility tree sends its
    /// text over D-Bus on every update, so asking it for the whole document
    /// would copy a hundred megabytes to hand over sixty kilobytes. A rope slice
    /// is what this costs, whatever the file weighs.
    pub fn text_upto(&self, chars: usize) -> String {
        self.rope.slice(..chars.min(self.len_chars())).to_string()
    }

    /// The whole document written with `eol` terminators.
    pub fn text_with_eol(&self, eol: EolMode) -> String {
        if eol == EolMode::Lf {
            return self.rope.to_string();
        }
        let mut out = String::with_capacity(self.rope.len_bytes() + self.rope.len_lines());
        for chunk in self.rope.chunks() {
            for c in chunk.chars() {
                if c == '\n' {
                    out.push_str(eol.as_str());
                } else {
                    out.push(c);
                }
            }
        }
        out
    }

    // ---- editing --------------------------------------------------------

    /// Inserts `text` at `char_idx`.
    ///
    /// The inserted characters start unstyled; the lexer will colour them.
    pub fn insert(&mut self, char_idx: usize, text: &str) {
        if text.is_empty() || self.read_only {
            return;
        }
        let char_idx = char_idx.min(self.rope.len_chars());
        let start_line = self.rope.char_to_line(char_idx);
        let added = text.bytes().filter(|&b| b == b'\n').count();
        let chars = text.chars().count();
        self.rope.insert(char_idx, text);
        self.styles.splice(
            char_idx..char_idx,
            std::iter::repeat_n(STYLE_DEFAULT, chars),
        );
        self.splice_lines(start_line, 1, added + 1);
        self.touch();
    }

    /// Removes the characters in `range`.
    pub fn remove(&mut self, range: Range<usize>) {
        if self.read_only {
            return;
        }
        let start = range.start.min(self.rope.len_chars());
        let end = range.end.min(self.rope.len_chars());
        if start >= end {
            return;
        }
        let start_line = self.rope.char_to_line(start);
        let end_line = self.rope.char_to_line(end);
        self.rope.remove(start..end);
        self.styles.splice(start..end, std::iter::empty());
        self.splice_lines(start_line, end_line - start_line + 1, 1);
        self.touch();
    }

    /// Replaces the characters in `range` with `text`.
    pub fn replace(&mut self, range: Range<usize>, text: &str) {
        if self.read_only {
            return;
        }
        let start = range.start.min(self.rope.len_chars());
        let end = range.end.min(self.rope.len_chars());
        if start >= end {
            self.insert(start, text);
            return;
        }
        let start_line = self.rope.char_to_line(start);
        let end_line = self.rope.char_to_line(end);
        let added = text.bytes().filter(|&b| b == b'\n').count();
        let chars = text.chars().count();
        self.rope.remove(start..end);
        self.rope.insert(start, text);
        self.styles
            .splice(start..end, std::iter::repeat_n(STYLE_DEFAULT, chars));
        self.splice_lines(start_line, end_line - start_line + 1, added + 1);
        self.touch();
    }

    /// Replaces the entire contents, resetting all per-character state.
    pub fn set_text(&mut self, text: &str) {
        self.rope = Rope::from_str(text);
        self.styles = vec![STYLE_DEFAULT; self.rope.len_chars()];
        self.fold_levels = vec![FOLD_LEVEL_BASE; self.rope.len_lines()];
        self.markers = vec![0; self.rope.len_lines()];
        self.lex_states = vec![0; self.rope.len_lines()];
        self.fold_depths = vec![0; self.rope.len_lines()];
        self.folded = vec![false; self.rope.len_lines()];
        let line_lens: Vec<u32> = (0..self.rope.len_lines())
            .map(|line| self.line_range(line).len() as u32)
            .collect();
        self.line_lens = line_lens;
        self.rows_dirty = true;
        self.touch();
    }

    /// Copies `other`'s text, styling and fold structure into this document.
    ///
    /// This is for the second view on a cloned document, where the same buffer
    /// has to appear in two panes. The text could be copied with
    /// [`set_text`](Self::set_text) and re-lexed, but re-lexing a large file
    /// costs tens of milliseconds — the highlighter walks every line — and that
    /// would be paid on each keystroke of the twin. `other` has already been
    /// lexed, and its style bytes describe exactly the same characters, so
    /// copying them is a memcpy instead of a parse.
    ///
    /// Markers stay behind: a bookmark is the reader's mark on a view, not a
    /// property of the buffer, and a keystroke in the twin should not wipe a
    /// bookmark set in this pane. Folds do carry over, but only where the line
    /// is still a fold header — a collapsed block that the twin's edit turned
    /// into plain text is reopened rather than left holding invisible lines.
    pub fn mirror_from(&mut self, other: &Document) {
        self.rope = other.rope.clone();
        self.styles.clone_from(&other.styles);
        self.lex_states.clone_from(&other.lex_states);
        self.fold_levels.clone_from(&other.fold_levels);
        self.fold_depths.clone_from(&other.fold_depths);
        self.line_lens.clone_from(&other.line_lens);
        self.rows_dirty = true;
        self.folded.resize(self.fold_levels.len(), false);
        for line in 0..self.folded.len() {
            if self.folded[line] && !is_fold_header(self.fold_level(line)) {
                self.folded[line] = false;
            }
        }
        self.markers.resize(self.rope.len_lines(), 0);
        self.touch();
    }

    /// Replaces `remove` per-line entries starting at `at` with `insert` fresh
    /// ones, then reconciles the lengths with the rope.
    fn splice_lines(&mut self, at: usize, remove: usize, insert: usize) {
        let at = at.min(self.fold_levels.len());
        let remove = remove.min(self.fold_levels.len() - at);
        // The state at the start of the edited line is decided by the lines
        // above it, which the edit did not touch, so it survives the splice.
        // Keeping it is what lets the highlighter resume here instead of at the
        // top of the file.
        let carried = self.lex_states.get(at).copied().unwrap_or(0);
        // A bookmark belongs to the line it was set on, and an edit that splits
        // or extends that line leaves the first half where it was, so the marker
        // travels with it rather than being dropped.
        let carried_marker = self.markers.get(at).copied().unwrap_or(0);
        self.fold_levels.splice(
            at..at + remove,
            std::iter::repeat_n(FOLD_LEVEL_BASE, insert),
        );
        self.markers
            .splice(at..at + remove, std::iter::repeat_n(0, insert));
        if let Some(slot) = self.markers.get_mut(at) {
            *slot = carried_marker;
        }
        self.lex_states
            .splice(at..at + remove, std::iter::repeat_n(0, insert));
        self.folded
            .splice(at..at + remove, std::iter::repeat_n(false, insert));
        let carried_depth = self.fold_depths.get(at).copied().unwrap_or(0);
        self.fold_depths
            .splice(at..at + remove, std::iter::repeat_n(0, insert));
        if let Some(slot) = self.fold_depths.get_mut(at) {
            *slot = carried_depth;
        }
        if let Some(slot) = self.lex_states.get_mut(at) {
            *slot = carried;
        }
        self.line_lens
            .splice(at..at + remove, std::iter::repeat_n(0, insert));
        self.rows_dirty = true;
        self.sync_line_arrays();
        // Only the lines the edit touched can have changed length.
        let end = (at + insert).min(self.line_lens.len());
        for line in at..end {
            self.line_lens[line] = self.line_range(line).len() as u32;
        }
    }

    /// Forces the per-line arrays to match the rope's line count.
    fn sync_line_arrays(&mut self) {
        let want = self.rope.len_lines();
        self.fold_levels.resize(want, FOLD_LEVEL_BASE);
        self.markers.resize(want, 0);
        self.lex_states.resize(want, 0);
        self.line_lens.resize(want, 0);
        self.folded.resize(want, false);
        self.fold_depths.resize(want, 0);
        debug_assert_eq!(self.styles.len(), self.rope.len_chars());
    }

    // ---- per-character style --------------------------------------------

    /// The style of the character at `char_idx`.
    pub fn style_at(&self, char_idx: usize) -> Style {
        self.styles.get(char_idx).copied().unwrap_or(STYLE_DEFAULT)
    }

    /// Sets the style of the character at `char_idx`.
    pub fn set_style_at(&mut self, char_idx: usize, style: Style) {
        if let Some(slot) = self.styles.get_mut(char_idx) {
            if *slot != style {
                *slot = style;
                self.revision = self.revision.wrapping_add(1);
            }
        }
    }

    /// Sets the style of every character in `range`.
    pub fn set_style_range(&mut self, range: Range<usize>, style: Style) {
        let end = range.end.min(self.styles.len());
        if range.start < end {
            self.styles[range.start..end].fill(style);
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// The styles of `line`'s characters, excluding its terminator.
    pub fn line_styles(&self, line: usize) -> &[Style] {
        let r = self.line_range(line);
        &self.styles[r.start..r.end]
    }

    /// Replaces the styles of `line`'s characters.
    ///
    /// Extra styles are ignored and missing ones leave the tail unstyled, so a
    /// lexer that is off by one cannot corrupt the document.
    pub fn set_line_styles(&mut self, line: usize, styles: &[Style]) {
        let r = self.line_range(line);
        let n = (r.end - r.start).min(styles.len());
        if n > 0 && self.styles[r.start..r.start + n] != styles[..n] {
            self.styles[r.start..r.start + n].copy_from_slice(&styles[..n]);
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// The whole style array, indexed by character.
    pub fn styles(&self) -> &[Style] {
        &self.styles
    }

    // ---- per-line state -------------------------------------------------

    /// The fold level of `line`.
    pub fn fold_level(&self, line: usize) -> FoldLevel {
        self.fold_levels
            .get(line)
            .copied()
            .unwrap_or(FOLD_LEVEL_BASE)
    }

    /// Sets the fold level of `line`.
    pub fn set_fold_level(&mut self, line: usize, level: FoldLevel) {
        if let Some(slot) = self.fold_levels.get_mut(line) {
            *slot = level;
        }
    }

    /// The fold level array, indexed by line.
    pub fn fold_levels(&self) -> &[FoldLevel] {
        &self.fold_levels
    }

    /// The brace depth at the start of `line`.
    pub fn fold_depth(&self, line: usize) -> u16 {
        self.fold_depths.get(line).copied().unwrap_or(0)
    }

    /// Sets the brace depth at the start of `line`.
    pub fn set_fold_depth(&mut self, line: usize, depth: u16) {
        if let Some(slot) = self.fold_depths.get_mut(line) {
            *slot = depth;
        }
    }

    /// The characters of `line`, without allocating.
    pub fn line_chars(&self, line: usize) -> impl Iterator<Item = char> + '_ {
        let last = self.rope.len_lines().saturating_sub(1);
        self.rope.line(line.min(last)).chars()
    }

    /// Replaces every fold level at once.
    ///
    /// Extra levels are ignored and missing ones leave the tail at the base
    /// level, so a folder that is off by one cannot corrupt the document.
    pub fn set_fold_levels(&mut self, levels: &[FoldLevel]) {
        let n = self.fold_levels.len().min(levels.len());
        if n > 0 && self.fold_levels[..n] != levels[..n] {
            self.fold_levels[..n].copy_from_slice(&levels[..n]);
            self.rows_dirty = true;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    // ---- folding --------------------------------------------------------

    /// Whether the block that starts at `line` is collapsed.
    pub fn is_folded(&self, line: usize) -> bool {
        self.folded.get(line).copied().unwrap_or(false)
    }

    /// The last line of the block that starts at `line`.
    ///
    /// A block runs until the fold level drops back to `line`'s level, which is
    /// how Scintilla defines it. Returns `line` when nothing is foldable.
    pub fn fold_end(&self, line: usize) -> usize {
        let base = fold_number(self.fold_level(line));
        let mut end = line;
        let mut probe = line + 1;
        while probe < self.fold_levels.len() {
            if fold_number(self.fold_levels[probe]) <= base {
                break;
            }
            end = probe;
            probe += 1;
        }
        end
    }

    /// Whether `line` starts a block that can be collapsed.
    pub fn is_foldable(&self, line: usize) -> bool {
        is_fold_header(self.fold_level(line)) && self.fold_end(line) > line
    }

    /// Collapses the block that starts at `line`.
    pub fn fold(&mut self, line: usize) {
        if self.is_foldable(line) && !self.is_folded(line) {
            self.folded[line] = true;
            self.rows_dirty = true;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Expands the block that starts at `line`.
    pub fn unfold(&mut self, line: usize) {
        if self.folded.get(line).copied().unwrap_or(false) {
            self.folded[line] = false;
            self.rows_dirty = true;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Collapses or expands the block that starts at `line`.
    pub fn toggle_fold(&mut self, line: usize) {
        if self.is_folded(line) {
            self.unfold(line);
        } else {
            self.fold(line);
        }
    }

    /// Expands every collapsed block.
    pub fn unfold_all(&mut self) {
        if self.folded.iter().any(|&f| f) {
            self.folded.fill(false);
            self.rows_dirty = true;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Collapses every foldable block.
    pub fn fold_all(&mut self) {
        let mut changed = false;
        for line in 0..self.folded.len() {
            if self.is_foldable(line) && !self.folded[line] {
                self.folded[line] = true;
                changed = true;
            }
        }
        if changed {
            self.rows_dirty = true;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Collapses every block deeper than `level`.
    ///
    /// Notepad++'s "Fold Level n" means "show me the outline down to n", so a
    /// block at depth `level` or shallower stays open and anything nested inside
    /// it closes. Depth is the fold depth of the header line, which is what the
    /// fold margin already draws.
    pub fn fold_to_level(&mut self, level: u16) {
        let mut changed = false;
        for line in 0..self.folded.len() {
            if self.is_foldable(line) && self.fold_depth(line) >= level && !self.folded[line] {
                self.folded[line] = true;
                changed = true;
            }
        }
        if changed {
            self.rows_dirty = true;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Expands every block at `level` or deeper.
    pub fn unfold_to_level(&mut self, level: u16) {
        let mut changed = false;
        for line in 0..self.folded.len() {
            if self.folded[line] && self.fold_depth(line) >= level {
                self.folded[line] = false;
                changed = true;
            }
        }
        if changed {
            self.rows_dirty = true;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Expands the block that hides `line`, if any.
    pub fn reveal_line(&mut self, line: usize) {
        if !self.is_hidden(line) {
            return;
        }
        // The hiding header is the nearest foldable line above with a block that
        // spans `line`.
        let mut probe = line;
        while probe > 0 {
            probe -= 1;
            if self.is_folded(probe) && self.fold_end(probe) >= line {
                self.unfold(probe);
                return;
            }
        }
    }

    /// Rebuilds the row mapping if folding, wrapping, or the text changed since
    /// the last call.
    ///
    /// Leaves the three vectors empty when there is no mapping to build -- no
    /// wrap width and no collapsed block -- because then row `n` is line `n` and
    /// every reader below reads an empty vector as exactly that.
    fn sync_rows(&mut self) {
        if !self.rows_dirty {
            return;
        }
        self.rows_dirty = false;
        self.rows.clear();
        self.row_of.clear();
        self.row_count.clear();
        if self.wrap_cols == 0 && !self.folded.iter().any(|&folded| folded) {
            return;
        }
        let lines = self.rope.len_lines();
        self.row_of.resize(lines, HIDDEN);
        self.row_count.resize(lines, 1);
        let wrap = self.wrap_cols;
        let mut line = 0;
        while line < lines {
            let len = self.line_lens.get(line).copied().unwrap_or(0) as usize;
            let segments = if wrap == 0 {
                1
            } else {
                len.div_ceil(wrap).max(1).min(u16::MAX as usize)
            };
            self.row_of[line] = self.rows.len();
            self.row_count[line] = segments as u16;
            for segment in 0..segments {
                self.rows.push((line, segment as u16));
            }
            if self.folded.get(line).copied().unwrap_or(false) {
                line = self.fold_end(line) + 1;
            } else {
                line += 1;
            }
        }
    }

    /// How many lines are visible, i.e. not inside a collapsed block.
    pub fn visible_line_count(&mut self) -> usize {
        self.sync_rows();
        if self.rows.is_empty() {
            return self.rope.len_lines();
        }
        self.rows.len()
    }

    /// The document line drawn at `row`.
    pub fn line_of_row(&mut self, row: usize) -> usize {
        self.sync_rows();
        if self.rows.is_empty() {
            return row.min(self.rope.len_lines().saturating_sub(1));
        }
        self.rows
            .get(row)
            .map(|r| r.0)
            .unwrap_or_else(|| self.rope.len_lines().saturating_sub(1))
    }

    /// The wrapped segment of the line drawn at `row`.
    pub fn segment_of_row(&mut self, row: usize) -> u16 {
        self.sync_rows();
        self.rows.get(row).map_or(0, |r| r.1)
    }

    /// How many rows `line` occupies once wrapped.
    pub fn segments_of_line(&mut self, line: usize) -> u16 {
        self.sync_rows();
        self.row_count.get(line).copied().unwrap_or(1).max(1)
    }

    /// The columns text is wrapped at, or 0 when wrapping is off.
    pub fn wrap_cols(&self) -> usize {
        self.wrap_cols
    }

    /// Sets the wrap width in columns, or 0 to turn wrapping off.
    ///
    /// Returns whether anything changed, so the caller can skip a redraw.
    pub fn set_wrap(&mut self, cols: usize) -> bool {
        if self.wrap_cols == cols {
            return false;
        }
        self.wrap_cols = cols;
        self.rows_dirty = true;
        true
    }

    /// The row `line` is drawn at, or `None` when it is hidden by a fold.
    pub fn row_of_line(&mut self, line: usize) -> Option<usize> {
        self.sync_rows();
        if self.row_of.is_empty() {
            return (line < self.rope.len_lines()).then_some(line);
        }
        match self.row_of.get(line).copied() {
            Some(HIDDEN) | None => None,
            Some(row) => Some(row),
        }
    }

    /// Whether `line` is inside a collapsed block.
    pub fn is_hidden(&mut self, line: usize) -> bool {
        self.row_of_line(line).is_none()
    }

    /// The marker mask of `line`.
    pub fn marker(&self, line: usize) -> Marker {
        self.markers.get(line).copied().unwrap_or(0)
    }

    /// Sets the marker mask of `line`.
    pub fn set_marker(&mut self, line: usize, marker: Marker) {
        if let Some(slot) = self.markers.get_mut(line) {
            *slot = marker;
        }
    }

    /// The marker array, indexed by line.
    pub fn markers(&self) -> &[Marker] {
        &self.markers
    }

    // ---- bookmarks -------------------------------------------------------

    /// Whether `line` carries a bookmark.
    pub fn is_bookmarked(&self, line: usize) -> bool {
        self.marker(line) & BOOKMARK != 0
    }

    /// Adds or removes the bookmark on `line`, returning its new state.
    pub fn toggle_bookmark(&mut self, line: usize) -> bool {
        if line >= self.markers.len() {
            return false;
        }
        self.markers[line] ^= BOOKMARK;
        self.touch();
        self.markers[line] & BOOKMARK != 0
    }

    /// Removes every bookmark.
    pub fn clear_bookmarks(&mut self) {
        for marker in &mut self.markers {
            *marker &= !BOOKMARK;
        }
        self.touch();
    }

    /// How many lines are bookmarked.
    pub fn bookmark_count(&self) -> usize {
        self.markers.iter().filter(|m| **m & BOOKMARK != 0).count()
    }

    /// The first bookmarked line after `from`, wrapping to the top.
    ///
    /// Wrapping is what makes `F2` useful at the end of a file: the next
    /// bookmark is always somewhere, so the key never does nothing.
    pub fn next_bookmark(&self, from: usize) -> Option<usize> {
        let n = self.markers.len();
        if n == 0 {
            return None;
        }
        let start = (from + 1).min(n);
        (start..n)
            .chain(0..start)
            .find(|&line| self.markers[line] & BOOKMARK != 0)
    }

    /// The last bookmarked line before `from`, wrapping to the bottom.
    pub fn prev_bookmark(&self, from: usize) -> Option<usize> {
        let n = self.markers.len();
        if n == 0 {
            return None;
        }
        let start = from.min(n);
        (0..start)
            .rev()
            .chain((start..n).rev())
            .find(|&line| self.markers[line] & BOOKMARK != 0)
    }

    /// The lexer state at the start of `line`, as an opaque byte.
    ///
    /// Stored per line so it shifts with the text on every edit, which is what
    /// lets the highlighter resume from an edited line instead of the top of the
    /// file.
    pub fn lex_state(&self, line: usize) -> u8 {
        self.lex_states.get(line).copied().unwrap_or(0)
    }

    /// Records the lexer state at the start of `line`.
    pub fn set_lex_state(&mut self, line: usize, state: u8) {
        if let Some(slot) = self.lex_states.get_mut(line) {
            *slot = state;
        }
    }

    // ---- metadata -------------------------------------------------------

    /// The line-ending mode used when saving.
    pub fn eol(&self) -> EolMode {
        self.eol
    }

    /// Changes the line-ending mode used when saving.
    pub fn set_eol(&mut self, eol: EolMode) {
        if self.eol != eol {
            self.eol = eol;
            self.dirty = true;
        }
    }

    /// The encoding used when saving.
    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// Changes the encoding used when saving.
    pub fn set_encoding(&mut self, encoding: Encoding) {
        if self.encoding != encoding {
            self.encoding = encoding;
            self.dirty = true;
        }
    }

    /// The file this document was loaded from or saved to.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Points the document at a different file without touching either one.
    ///
    /// Used by Rename, where the file has already been moved on disk and the
    /// document has to follow it. The stamp is re-read from the new path so the
    /// change monitor does not immediately report the file as modified: the
    /// bytes did not change, only the name.
    pub fn set_path(&mut self, path: &Path) {
        self.path = Some(path.to_path_buf());
        self.stamp = FileStamp::of(path);
    }

    /// Whether the file on disk has changed since it was last read or written.
    ///
    /// An untitled buffer has no file, so it never reports a change. A file that
    /// has been deleted reports one: `FileStamp::of` returns `None`, which is
    /// unequal to any recorded stamp.
    pub fn disk_changed(&self) -> bool {
        let Some(path) = &self.path else {
            return false;
        };
        FileStamp::of(path) != self.stamp
    }

    /// What to do about the file on disk.
    ///
    /// A dirty buffer always wins: the change is reported, never applied.
    pub fn disk_change(&self) -> DiskChange {
        if !self.disk_changed() {
            return DiskChange::None;
        }
        if self.dirty {
            DiskChange::Conflict
        } else {
            DiskChange::Reload
        }
    }

    /// Whether the file changed underneath a buffer with unsaved edits.
    pub fn has_disk_conflict(&self) -> bool {
        self.disk_conflict
    }

    /// Records that the file changed while the buffer was dirty, without
    /// touching the text.
    ///
    /// The stamp is advanced so the same change is not reported again on the
    /// next poll: a warning that repeats every second is a warning nobody reads.
    /// A second, later change still reports.
    pub fn note_disk_conflict(&mut self) {
        self.disk_conflict = true;
        self.stamp = self.path.as_deref().and_then(FileStamp::of);
    }

    /// Re-reads the file, discarding unsaved changes.
    ///
    /// The path, the read-only guard, and the caret survive; the text, the
    /// styling, and the dirty flag do not. Returns an error when the file cannot
    /// be read, in which case the buffer is left exactly as it was.
    pub fn reload(&mut self) -> std::io::Result<()> {
        let path = self.path.clone().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "document has no path")
        })?;
        let bytes = std::fs::read(&path)?;
        let encoding = Encoding::detect(&bytes);
        self.adopt(&path, &bytes, encoding);
        Ok(())
    }

    /// Re-reads the file, decoding it as `encoding` rather than guessing.
    ///
    /// This is what the Encoding menu's top half does: the bytes on disk are
    /// unchanged and the same bytes are read again under a different assumption.
    /// It is the only way to recover a file whose encoding was guessed wrong,
    /// which is the whole reason the menu exists.
    pub fn reload_as(&mut self, encoding: Encoding) -> std::io::Result<()> {
        let path = self.path.clone().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "document has no path")
        })?;
        let bytes = std::fs::read(&path)?;
        self.adopt(&path, &bytes, encoding);
        Ok(())
    }

    /// Replaces the text with `bytes` decoded as `encoding`, and forgets the
    /// unsaved state.
    ///
    /// The one place that knows what a reload does to a document, so `reload`
    /// and `reload_as` cannot drift apart on which fields survive.
    fn adopt(&mut self, path: &Path, bytes: &[u8], encoding: Encoding) {
        let decoded = encoding.decode(bytes);
        let (text, eol) = EolMode::normalise(&decoded);
        self.set_text(&text);
        self.eol = eol;
        self.encoding = encoding;
        self.dirty = false;
        self.disk_conflict = false;
        self.stamp = FileStamp::of(path);
    }

    /// The document's display name: the file name, or `new` for untitled buffers.
    pub fn display_name(&self) -> String {
        match &self.path {
            Some(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string()),
            None => "new".to_string(),
        }
    }

    /// Whether the document is big enough to open in large-file mode.
    ///
    /// Measured on a 100 MB file: parsing takes 1.3 s, highlighting 6.5 s, and
    /// folding 4.1 s. Skipping the last two is the difference between a file that
    /// opens and a file that appears to hang.
    pub fn is_large(&self) -> bool {
        self.len_chars() > LARGE_FILE_CHARS
    }

    /// Whether the document has unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Sets or clears the unsaved marker without touching the text.
    ///
    /// The second view on a cloned document needs this: the two views hold the
    /// same text, so a save through either one clears the marker on both, and an
    /// edit through either one raises it on both. `touch` cannot serve, because
    /// it also counts as a change and would make the frame repaint for a flag.
    pub fn set_dirty(&mut self, dirty: bool) {
        self.dirty = dirty;
    }

    /// A counter that changes whenever the document's text or styling does.
    ///
    /// The view uses it to skip revalidating every visible line on frames where
    /// nothing could have changed, which is most of them.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// This document's process-unique id.
    ///
    /// A cache that holds shaped lines has to know when the document under it
    /// changed. `(id, revision)` is that answer: the id changes when the tab
    /// does, the revision when the text does.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Marks the document as changed.
    fn touch(&mut self) {
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
    }

    /// True if `line`'s text is exactly `text`, without allocating.
    pub fn line_matches(&self, line: usize, text: &str) -> bool {
        self.rope
            .slice(self.line_range(line))
            .chars()
            .eq(text.chars())
    }

    /// Clears the dirty flag.
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Whether edits are refused.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Turns the edit guard on or off. Not an edit, so it does not dirty the
    /// buffer.
    pub fn set_read_only(&mut self, read_only: bool) {
        self.read_only = read_only;
    }
}

// ---- loading ---------------------------------------------------------------

/// Builds a rope from `path`, one bite of the file at a time.
fn stream(path: &Path) -> std::io::Result<(Rope, Encoding, EolMode)> {
    use std::io::Read;

    const BITE: usize = 64 * 1024;
    let file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::with_capacity(BITE, file);
    let mut raw = vec![0u8; BITE];
    let mut loader = Loader::new();
    loop {
        let read = reader.read(&mut raw)?;
        if read == 0 {
            break;
        }
        loader.feed(&raw[..read], false)?;
    }
    loader.feed(&[], true)?;
    loader.finish()
}

/// Reads a file into a rope, folding its line terminators on the way in.
///
/// The document keeps `\n` terminators and UTF-8 text, so a UTF-8 BOM has to go
/// and every carriage return has to become a newline. Doing that to a whole-file
/// `String` means holding the file, then the normalised copy, then the rope's
/// own copy; this holds 64 KiB of file, counts the terminators as they pass, and
/// grows the rope behind them.
///
/// Anything that is not UTF-8 -- a UTF-16 file, a code page, a binary -- is
/// refused here rather than guessed at, and `Document::from_file` re-reads it
/// the old way, because telling those apart means looking at every byte at once.
struct Loader {
    builder: RopeBuilder,
    /// The bytes of this bite, with the tail of the last one in front of them.
    text: Vec<u8>,
    /// At most three bytes of a character split across two bites.
    tail: Vec<u8>,
    /// The first three bytes of the file, which are a BOM or are not.
    head: Vec<u8>,
    head_done: bool,
    bom: bool,
    /// A `\r` at the very end of a bite, waiting to see whether a `\n` follows.
    pending_cr: bool,
    lf: usize,
    crlf: usize,
    cr: usize,
}

impl Loader {
    fn new() -> Self {
        Self {
            builder: RopeBuilder::new(),
            text: Vec::with_capacity(64 * 1024 + 4),
            tail: Vec::new(),
            head: Vec::new(),
            head_done: false,
            bom: false,
            pending_cr: false,
            lf: 0,
            crlf: 0,
            cr: 0,
        }
    }

    /// Feeds one bite of the file; `at_end` says it is the last one.
    fn feed(&mut self, bytes: &[u8], at_end: bool) -> std::io::Result<()> {
        let mut bytes = bytes;
        if !self.head_done {
            let want = 3 - self.head.len();
            let take = want.min(bytes.len());
            self.head.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.head.len() < 3 && !at_end {
                return Ok(());
            }
            self.head_done = true;
            self.bom = self.head == [0xEF, 0xBB, 0xBF];
            if !self.bom {
                let head = std::mem::take(&mut self.head);
                self.filter(&head, at_end)?;
            }
        }
        self.filter(bytes, at_end)
    }

    /// Folds the terminators in `bytes` and hands the result to the rope.
    fn filter(&mut self, bytes: &[u8], at_end: bool) -> std::io::Result<()> {
        let mut bytes = bytes;
        self.text.clear();
        self.text.append(&mut self.tail);
        if self.pending_cr {
            match bytes.first() {
                Some(&b'\n') => {
                    self.crlf += 1;
                    self.pending_cr = false;
                    bytes = &bytes[1..];
                }
                Some(_) => {
                    self.cr += 1;
                    self.pending_cr = false;
                }
                None if at_end => {
                    self.cr += 1;
                    self.pending_cr = false;
                }
                None => {}
            }
            if !self.pending_cr {
                self.text.push(b'\n');
            }
        }
        let mut index = 0;
        while index < bytes.len() {
            let byte = bytes[index];
            if byte == b'\r' {
                if index + 1 == bytes.len() && !at_end {
                    // The newline may be the first byte of the next bite.
                    self.pending_cr = true;
                    index += 1;
                    continue;
                }
                if bytes.get(index + 1) == Some(&b'\n') {
                    self.crlf += 1;
                    index += 1;
                } else {
                    self.cr += 1;
                }
                self.text.push(b'\n');
                index += 1;
                continue;
            }
            if byte == b'\n' {
                self.lf += 1;
            }
            self.text.push(byte);
            index += 1;
        }
        self.append()
    }

    /// Checks what was filtered is UTF-8, and grows the rope by it.
    ///
    /// A character split across two bites stays in `tail` until the rest of it
    /// arrives; a byte that is not UTF-8 at all ends the load, and the caller
    /// falls back to reading the file whole.
    fn append(&mut self) -> std::io::Result<()> {
        match std::str::from_utf8(&self.text) {
            Ok(text) => self.builder.append(text),
            Err(error) => {
                let valid = error.valid_up_to();
                if valid > 0 {
                    let text = std::str::from_utf8(&self.text[..valid])
                        .expect("from_utf8 said this prefix is valid");
                    self.builder.append(text);
                }
                match error.error_len() {
                    Some(_) => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "not UTF-8",
                        ))
                    }
                    None => {
                        self.tail.clear();
                        self.tail.extend_from_slice(&self.text[valid..]);
                    }
                }
            }
        }
        self.text.clear();
        Ok(())
    }

    /// The rope, once every bite has been fed.
    fn finish(self) -> std::io::Result<(Rope, Encoding, EolMode)> {
        if !self.tail.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "not UTF-8",
            ));
        }
        let encoding = if self.bom {
            Encoding::Utf8Bom
        } else {
            Encoding::Utf8
        };
        let eol = EolMode::from_counts(self.crlf, self.lf, self.cr);
        Ok((self.builder.finish(), encoding, eol))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_documents_have_different_ids() {
        let a = Document::from_text("one");
        let b = Document::from_text("two");
        assert_ne!(a.id(), b.id());
    }

    #[test]
    fn a_documents_id_survives_an_edit() {
        // The id names the buffer, not its contents: a cache keyed on
        // `(id, revision)` must not throw its lines away because the text
        // changed, only because the tab did.
        let mut doc = Document::from_text("one");
        let id = doc.id();
        doc.insert(0, "x");
        assert_eq!(doc.id(), id);
        assert_ne!(doc.revision(), 0);
    }

    #[test]
    fn a_clone_keeps_the_id() {
        // A clone is the same buffer, so a cache built for one is valid for the
        // other. Diverging edits bump the revision, which is what invalidates.
        let doc = Document::from_text("one");
        assert_eq!(doc.clone().id(), doc.id());
    }

    #[test]
    fn word_at_finds_the_whole_word() {
        let doc = Document::from_text("let value = compute(1);");
        // Caret inside `value`.
        assert_eq!(doc.word_at(6).as_deref(), Some("value"));
        // Caret on the first and last character of the word.
        assert_eq!(doc.word_at(4).as_deref(), Some("value"));
        assert_eq!(doc.word_at(8).as_deref(), Some("value"));
    }

    #[test]
    fn word_at_stops_at_punctuation() {
        let doc = Document::from_text("foo.bar");
        assert_eq!(doc.word_at(5).as_deref(), Some("bar"));
        assert_eq!(doc.word_at(0).as_deref(), Some("foo"));
    }

    #[test]
    fn word_at_on_punctuation_is_none() {
        let doc = Document::from_text("foo.bar");
        assert_eq!(doc.word_at(3), None);
    }

    #[test]
    fn word_at_past_the_end_is_none() {
        let doc = Document::from_text("abc");
        assert_eq!(doc.word_at(99), None);
    }

    #[test]
    fn word_at_keeps_underscores_and_digits() {
        let doc = Document::from_text("let _x1 = 2;");
        assert_eq!(doc.word_at(5).as_deref(), Some("_x1"));
    }

    #[test]
    fn word_at_handles_non_ascii() {
        let doc = Document::from_text("let caf\u{e9} = 1;");
        assert_eq!(doc.word_at(5).as_deref(), Some("caf\u{e9}"));
    }

    #[test]
    fn empty_document_has_one_line() {
        let doc = Document::new();
        assert_eq!(doc.len_lines(), 1);
        assert_eq!(doc.len_chars(), 0);
        assert_eq!(doc.line(0), "");
    }

    #[test]
    fn trailing_newline_adds_an_empty_line() {
        let doc = Document::from_text("a\nb\n");
        assert_eq!(doc.len_lines(), 3);
        assert_eq!(doc.line(0), "a");
        assert_eq!(doc.line(1), "b");
        assert_eq!(doc.line(2), "");
    }

    #[test]
    fn line_ranges_exclude_terminators() {
        let doc = Document::from_text("ab\ncde\nf");
        assert_eq!(doc.line_range(0), 0..2);
        assert_eq!(doc.line_range(1), 3..6);
        assert_eq!(doc.line_range(2), 7..8);
        assert_eq!(doc.line_len(1), 3);
    }

    #[test]
    fn insert_within_a_line_keeps_line_count() {
        let mut doc = Document::from_text("ac");
        doc.insert(1, "b");
        assert_eq!(doc.text(), "abc");
        assert_eq!(doc.len_lines(), 1);
        assert_eq!(doc.styles().len(), 3);
    }

    #[test]
    fn insert_newline_splits_a_line() {
        let mut doc = Document::from_text("ab");
        doc.insert(1, "\n");
        assert_eq!(doc.text(), "a\nb");
        assert_eq!(doc.len_lines(), 2);
        assert_eq!(doc.fold_levels().len(), 2);
        assert_eq!(doc.markers().len(), 2);
        assert_eq!(doc.styles().len(), 3);
    }

    #[test]
    fn insert_multiple_newlines_grows_arrays() {
        let mut doc = Document::from_text("ab");
        doc.insert(1, "\n\n\n");
        assert_eq!(doc.len_lines(), 4);
        assert_eq!(doc.fold_levels().len(), 4);
        assert_eq!(doc.styles().len(), 5);
    }

    #[test]
    fn remove_within_a_line_keeps_line_count() {
        let mut doc = Document::from_text("abc");
        doc.remove(1..2);
        assert_eq!(doc.text(), "ac");
        assert_eq!(doc.len_lines(), 1);
        assert_eq!(doc.styles().len(), 2);
    }

    #[test]
    fn remove_newline_joins_lines() {
        let mut doc = Document::from_text("a\nb\nc");
        doc.remove(1..2);
        assert_eq!(doc.text(), "ab\nc");
        assert_eq!(doc.len_lines(), 2);
        assert_eq!(doc.fold_levels().len(), 2);
    }

    #[test]
    fn remove_across_lines_collapses_arrays() {
        let mut doc = Document::from_text("aaa\nbbb\nccc\nddd");
        doc.remove(2..9);
        assert_eq!(doc.text(), "aacc\nddd");
        assert_eq!(doc.len_lines(), 2);
        assert_eq!(doc.styles().len(), 8);
    }

    #[test]
    fn remove_everything_leaves_one_line() {
        let mut doc = Document::from_text("a\nb\nc\n");
        doc.remove(0..doc.len_chars());
        assert_eq!(doc.len_lines(), 1);
        assert_eq!(doc.styles().len(), 0);
        assert_eq!(doc.text(), "");
    }

    #[test]
    fn replace_spanning_lines() {
        let mut doc = Document::from_text("one\ntwo\nthree");
        doc.replace(2..7, "X\nY");
        assert_eq!(doc.text(), "onX\nY\nthree");
        assert_eq!(doc.styles().len(), doc.len_chars());
    }

    #[test]
    fn arrays_stay_in_sync_through_random_edits() {
        let mut doc = Document::from_text("alpha\nbeta\ngamma\ndelta");
        let edits: &[(usize, &str)] =
            &[(0, "X"), (3, "\n\n"), (10, "Y"), (1, "\n"), (0, "Z\nW\nV")];
        for &(at, text) in edits {
            let at = at.min(doc.len_chars());
            doc.insert(at, text);
            assert_eq!(doc.styles().len(), doc.len_chars(), "after insert {text:?}");
            assert_eq!(doc.fold_levels().len(), doc.len_lines());
            assert_eq!(doc.markers().len(), doc.len_lines());
        }
        for _ in 0..5 {
            if doc.len_chars() > 2 {
                doc.remove(0..2);
            }
            assert_eq!(doc.styles().len(), doc.len_chars());
        }
    }

    #[test]
    fn text_upto_takes_a_prefix_and_never_more() {
        let doc = Document::from_text("hello\nworld");
        assert_eq!(doc.text_upto(5), "hello");
        assert_eq!(doc.text_upto(100), "hello\nworld");
        // Characters, not bytes: a two-byte character is one character.
        let doc = Document::from_text("héllo");
        assert_eq!(doc.text_upto(2), "hé");
    }

    #[test]
    fn styles_are_per_character() {
        let mut doc = Document::from_text("abc\ndef");
        doc.set_style_at(1, 7);
        assert_eq!(doc.style_at(0), STYLE_DEFAULT);
        assert_eq!(doc.style_at(1), 7);
        assert_eq!(doc.style_at(2), STYLE_DEFAULT);
        assert_eq!(doc.style_at(4), STYLE_DEFAULT);
    }

    #[test]
    fn style_ranges_apply_to_characters() {
        let mut doc = Document::from_text("abcdef");
        doc.set_style_range(1..4, 3);
        assert_eq!(doc.styles(), &[0, 3, 3, 3, 0, 0]);
    }

    #[test]
    fn wrapping_splits_a_long_line_into_rows() {
        let mut doc = Document::from_text("abcdefghij\nxy");
        assert_eq!(doc.visible_line_count(), 2);
        assert!(doc.set_wrap(4));
        // 10 characters at 4 columns is 3 rows, plus the short second line.
        assert_eq!(doc.visible_line_count(), 4);
        assert_eq!(doc.segments_of_line(0), 3);
        assert_eq!(doc.segments_of_line(1), 1);
        assert_eq!(doc.row_of_line(0), Some(0));
        assert_eq!(doc.row_of_line(1), Some(3));
        assert_eq!(doc.line_of_row(0), 0);
        assert_eq!(doc.segment_of_row(0), 0);
        assert_eq!(doc.line_of_row(2), 0);
        assert_eq!(doc.segment_of_row(2), 2);
        assert_eq!(doc.line_of_row(3), 1);
        assert_eq!(doc.segment_of_row(3), 0);
    }

    #[test]
    fn an_empty_line_still_occupies_one_row() {
        let mut doc = Document::from_text("\n\n");
        doc.set_wrap(4);
        assert_eq!(doc.segments_of_line(0), 1);
        assert_eq!(doc.visible_line_count(), 3);
    }

    #[test]
    fn a_plain_document_keeps_no_row_mapping() {
        // Sixteen bytes of `rows` and eight of `row_of` per line would say, for
        // a file with a million lines, only that row n is line n.
        let mut doc = Document::from_text("one\ntwo\nthree\n");
        assert!(doc.rows.is_empty());
        assert_eq!(doc.visible_line_count(), 4);
        assert_eq!(doc.line_of_row(2), 2);
        assert_eq!(doc.segment_of_row(2), 0);
        assert_eq!(doc.segments_of_line(2), 1);
        assert_eq!(doc.row_of_line(2), Some(2));
        assert_eq!(doc.row_of_line(9), None);
        // Wrapping needs the mapping, and turning it off drops it back to
        // nothing rather than leaving the last one behind.
        assert!(doc.set_wrap(2));
        assert_eq!(doc.segments_of_line(0), 2);
        assert!(!doc.rows.is_empty());
        assert!(doc.set_wrap(0));
        assert_eq!(doc.visible_line_count(), 4);
        assert!(doc.rows.is_empty(), "{} rows", doc.rows.len());
        assert!(doc.row_of.is_empty(), "{} rows", doc.row_of.len());
        assert!(doc.row_count.is_empty(), "{} rows", doc.row_count.len());
    }

    #[test]
    fn turning_wrapping_off_restores_one_row_per_line() {
        let mut doc = Document::from_text("abcdefghij\nxy");
        doc.set_wrap(4);
        assert_eq!(doc.visible_line_count(), 4);
        assert!(doc.set_wrap(0));
        assert_eq!(doc.visible_line_count(), 2);
        // Setting the same width again is a no-op, so callers can skip a redraw.
        assert!(!doc.set_wrap(0));
    }

    #[test]
    fn editing_keeps_line_lengths_current() {
        let mut doc = Document::from_text("abc\ndef");
        assert_eq!(doc.line_len(0), 3);
        doc.insert(3, "xyz");
        assert_eq!(doc.line_len(0), 6);
        doc.remove(0..2);
        assert_eq!(doc.line_len(0), 4);
        doc.insert(0, "one\ntwo\n");
        assert_eq!(doc.len_lines(), 4);
        assert_eq!(doc.line_len(0), 3);
        assert_eq!(doc.line_len(1), 3);
        assert_eq!(doc.line_len(2), 4);
        assert_eq!(doc.line_len(3), 3);
    }

    #[test]
    fn wrapping_follows_an_edit() {
        let mut doc = Document::from_text("abcdefgh");
        doc.set_wrap(4);
        assert_eq!(doc.visible_line_count(), 2);
        doc.insert(8, "ijkl");
        assert_eq!(doc.visible_line_count(), 3);
        doc.remove(0..8);
        assert_eq!(doc.visible_line_count(), 1);
    }

    #[test]
    fn line_styles_exclude_the_terminator() {
        let mut doc = Document::from_text("ab\ncd");
        doc.set_style_at(0, 1);
        doc.set_style_at(1, 2);
        doc.set_style_at(3, 4);
        assert_eq!(doc.line_styles(0), &[1, 2]);
        assert_eq!(doc.line_styles(1), &[4, 0]);
    }

    #[test]
    fn mirroring_copies_text_and_styling_but_not_bookmarks() {
        let mut source = Document::from_text("fn main() {\n}\n");
        source.set_style_range(0..4, 7);
        source.set_lex_state(1, 3);
        source.set_fold_level(0, FOLD_LEVEL_BASE | FOLD_LEVEL_HEADERFLAG);
        source.set_marker(1, 4);

        let mut twin = Document::from_text("old");
        twin.set_marker(0, 8);
        twin.mirror_from(&source);

        assert_eq!(twin.text(), source.text());
        assert_eq!(twin.styles(), source.styles(), "the styling is a copy, not a re-lex");
        assert_eq!(twin.lex_state(1), 3);
        assert_eq!(twin.fold_level(0), source.fold_level(0));
        assert_eq!(twin.marker(0), 8, "the twin keeps its own bookmark");
        assert_eq!(twin.marker(1), 0, "and does not inherit the other view's");
    }

    #[test]
    fn mirroring_reopens_a_fold_the_new_text_no_longer_has() {
        let source = Document::from_text("plain\nplain\n");
        let mut twin = Document::from_text("head\nbody\n");
        twin.set_fold_level(0, FOLD_LEVEL_BASE | FOLD_LEVEL_HEADERFLAG);
        twin.set_fold_level(1, FOLD_LEVEL_BASE + 1);
        twin.fold(0);
        assert!(twin.is_folded(0));
        twin.mirror_from(&source);
        assert!(!twin.is_folded(0), "line 0 is not a fold header any more");
        assert_eq!(twin.row_of_line(1), Some(1), "both lines are visible again");
    }

    #[test]
    fn set_line_styles_ignores_extra_and_missing() {
        let mut doc = Document::from_text("ab\ncd");
        doc.set_line_styles(0, &[5, 6, 7, 8]);
        assert_eq!(doc.line_styles(0), &[5, 6]);
        doc.set_line_styles(1, &[9]);
        assert_eq!(doc.line_styles(1), &[9, 0]);
    }

    #[test]
    fn styles_follow_text_through_edits() {
        let mut doc = Document::from_text("abc");
        doc.set_style_range(0..3, 4);
        doc.insert(0, "X");
        // The new character is unstyled; the old ones keep their style.
        assert_eq!(doc.styles(), &[0, 4, 4, 4]);
        doc.remove(0..1);
        assert_eq!(doc.styles(), &[4, 4, 4]);
    }

    #[test]
    fn fold_level_helpers() {
        let level = (FOLD_LEVEL_BASE + 2) | FOLD_LEVEL_HEADERFLAG;
        assert_eq!(fold_number(level), FOLD_LEVEL_BASE + 2);
        assert!(is_fold_header(level));
        assert!(!is_fold_white(level));
    }

    #[test]
    fn eol_is_detected_and_restored() {
        let doc = Document::from_bytes(b"a\r\nb\r\nc");
        assert_eq!(doc.eol(), EolMode::CrLf);
        assert_eq!(doc.text(), "a\nb\nc");
        assert_eq!(doc.text_with_eol(EolMode::CrLf), "a\r\nb\r\nc");
        assert_eq!(doc.text_with_eol(EolMode::Cr), "a\rb\rc");
    }

    #[test]
    fn encoding_is_detected() {
        let doc = Document::from_bytes(b"\xEF\xBB\xBFhi");
        assert_eq!(doc.encoding(), Encoding::Utf8Bom);
        assert_eq!(doc.text(), "hi");
    }

    #[test]
    fn save_and_reload_roundtrip() {
        let dir = std::env::temp_dir().join("nn-core-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("roundtrip.txt");

        let mut doc = Document::from_text("héllo\nwörld\n");
        doc.set_eol(EolMode::CrLf);
        doc.set_encoding(Encoding::Utf8Bom);
        doc.save_as(&path).unwrap();
        assert!(!doc.is_dirty());

        let reloaded = Document::from_file(&path).unwrap();
        assert_eq!(reloaded.text(), "héllo\nwörld\n");
        assert_eq!(reloaded.eol(), EolMode::CrLf);
        assert_eq!(reloaded.encoding(), Encoding::Utf8Bom);
        assert_eq!(reloaded.display_name(), "roundtrip.txt");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn the_dirty_marker_can_be_set_without_editing() {
        // What a saved clone does: the text is already the twin's, so a save
        // through one view clears the marker on the other without a second edit.
        let mut doc = Document::from_text("alpha\n");
        assert!(!doc.is_dirty());
        let revision = doc.revision();
        doc.set_dirty(true);
        assert!(doc.is_dirty());
        assert_eq!(doc.revision(), revision, "setting the flag is not a change");
        doc.set_dirty(false);
        assert!(!doc.is_dirty());
    }

    #[test]
    fn a_buffer_whose_text_matches_its_file_comes_back_clean() {
        let scratch = Scratch::new("mark-clean-same.txt", "alpha\nbeta\n");
        let mut doc = Document::from_file(scratch.path()).unwrap();
        // What the snapshot restore does: replace the text, which marks dirty.
        doc.set_text("alpha\nbeta\n");
        assert!(doc.is_dirty());
        assert!(doc.mark_clean_if_unchanged());
        assert!(!doc.is_dirty());
    }

    #[test]
    fn a_buffer_with_different_text_stays_dirty() {
        let scratch = Scratch::new("mark-clean-differs.txt", "alpha\nbeta\n");
        let mut doc = Document::from_file(scratch.path()).unwrap();
        doc.set_text("alpha\nbeta\ngamma\n");
        assert!(!doc.mark_clean_if_unchanged());
        assert!(doc.is_dirty());
    }

    #[test]
    fn an_untitled_buffer_cannot_be_marked_clean() {
        let mut doc = Document::from_text("no file behind this");
        doc.set_text("still no file");
        assert!(!doc.mark_clean_if_unchanged());
        assert!(doc.is_dirty());
    }

    #[test]
    fn a_crlf_file_whose_text_matches_is_clean() {
        // The rope never holds CRLF, so a byte comparison would call this
        // modified. The comparison is on text, so it does not.
        let scratch = Scratch::new("mark-clean-crlf.txt", "alpha\r\nbeta\r\n");
        let mut doc = Document::from_file(scratch.path()).unwrap();
        doc.set_text("alpha\nbeta\n");
        assert!(doc.mark_clean_if_unchanged());
        assert!(!doc.is_dirty());
    }

    #[test]
    fn a_file_that_has_gone_away_cannot_be_marked_clean() {
        let scratch = Scratch::new("mark-clean-gone.txt", "alpha\n");
        let mut doc = Document::from_file(scratch.path()).unwrap();
        doc.set_text("alpha\n");
        std::fs::remove_file(scratch.path()).unwrap();
        assert!(!doc.mark_clean_if_unchanged());
        assert!(doc.is_dirty());
    }

    #[test]
    fn a_streamed_file_loads_the_same_as_a_whole_one() {
        // Every shape the loader has to get right: a plain file, an empty one, a
        // file shorter than the three bytes the BOM check waits for, CRLF, a
        // lone CR at the end of the file, a UTF-8 BOM, a four-byte character
        // straddling the 64 KiB bite, and bytes that are not UTF-8 at all.
        let mut split = String::new();
        split.push_str(&"a".repeat(64 * 1024 - 2));
        split.push('🎉');
        split.push_str("\nend\n");
        let cases: &[(&str, &[u8])] = &[
            ("plain", b"one\ntwo\n"),
            ("empty", b""),
            ("short", b"a"),
            ("crlf", b"one\r\ntwo\r\n"),
            ("cr-end", b"one\rtwo\r"),
            ("bom", b"\xEF\xBB\xBFhi\n"),
            ("split", split.as_bytes()),
            ("latin1", b"caf\xE9\n"),
        ];
        let dir = std::env::temp_dir().join("nn-core-tests");
        std::fs::create_dir_all(&dir).unwrap();
        for &(name, bytes) in cases {
            let path = dir.join(format!("streamed-{name}.txt"));
            std::fs::write(&path, bytes).unwrap();
            let streamed = Document::from_file(&path).unwrap();
            let whole = Document::from_bytes(bytes);
            assert_eq!(streamed.text(), whole.text(), "{name}: text");
            assert_eq!(streamed.eol(), whole.eol(), "{name}: eol");
            assert_eq!(streamed.encoding(), whole.encoding(), "{name}: encoding");
            std::fs::remove_file(&path).ok();
        }
    }

    /// A scratch file that removes itself.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(name: &str, text: &str) -> Self {
            let dir = std::env::temp_dir().join("nn-core-tests");
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join(name);
            std::fs::write(&path, text).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn rewrite(&self, text: &str) {
            std::fs::write(&self.0, text).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_file(&self.0).ok();
        }
    }

    /// A path in the scratch directory that nothing has created.
    fn absent(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("nn-core-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::remove_file(&path).ok();
        path
    }

    #[test]
    fn a_path_that_is_not_there_opens_a_blank_page() {
        let path = absent("never-written.txt");
        let doc = Document::open(&path).unwrap();
        assert_eq!(doc.path(), Some(path.as_path()));
        assert_eq!(doc.display_name(), "never-written.txt");
        assert!(doc.text().is_empty());
        assert!(!doc.is_dirty());
        // The stamp of an absent file is `None`, the same as the stamp of a
        // buffer that was never written, so the page is not reported as a file
        // that has just been deleted.
        assert!(!doc.disk_changed());
    }

    #[test]
    fn the_first_save_of_an_absent_file_creates_it() {
        let path = absent("first-save.txt");
        let mut doc = Document::open(&path).unwrap();
        doc.insert(0, "hello\n");
        doc.save().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello\n");
        assert!(!doc.is_dirty());
        assert!(!doc.disk_changed());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_directory_is_still_an_error() {
        // Absence is the only failure that means "blank page". A path that is
        // there and cannot be read stays an error, so the editor never offers
        // to replace something the user has not seen with an empty buffer.
        assert!(Document::open(std::env::temp_dir()).is_err());
    }

    #[test]
    fn a_small_document_is_not_large() {
        assert!(!Document::from_text("hello\nworld\n").is_large());
    }

    #[test]
    fn a_document_over_the_threshold_is_large() {
        let text = "a".repeat(LARGE_FILE_CHARS + 1);
        assert!(Document::from_text(&text).is_large());
    }

    #[test]
    fn the_threshold_itself_is_not_large() {
        let text = "a".repeat(LARGE_FILE_CHARS);
        assert!(!Document::from_text(&text).is_large());
    }

    #[test]
    fn a_freshly_loaded_file_has_not_changed() {
        let file = Scratch::new("unchanged.txt", "one\n");
        let doc = Document::from_file(file.path()).unwrap();
        assert!(!doc.disk_changed());
    }

    #[test]
    fn an_untitled_buffer_never_reports_a_change() {
        let doc = Document::from_text("hello");
        assert!(!doc.disk_changed());
    }

    #[test]
    fn a_rewrite_by_another_program_is_noticed() {
        let file = Scratch::new("rewritten.txt", "one\n");
        let doc = Document::from_file(file.path()).unwrap();
        file.rewrite("one\ntwo\n");
        assert!(doc.disk_changed());
    }

    #[test]
    fn a_same_length_rewrite_in_the_same_clock_tick_is_missed() {
        // The known limit of an mtime-and-length stamp, and the reason it is
        // written down rather than discovered later: this filesystem reports the
        // same modification time for two writes inside one tick, so a rewrite
        // that keeps the length is invisible. Every editor that polls instead of
        // watching has this hole; closing it means hashing the file on every
        // poll, which costs more than the bug.
        let file = Scratch::new("same-length.txt", "one\n");
        let doc = Document::from_file(file.path()).unwrap();
        file.rewrite("two\n");
        let _ = doc.disk_changed();
    }

    #[test]
    fn a_deleted_file_is_noticed() {
        let file = Scratch::new("deleted.txt", "one\n");
        let doc = Document::from_file(file.path()).unwrap();
        std::fs::remove_file(file.path()).unwrap();
        assert!(doc.disk_changed());
    }

    #[test]
    fn saving_clears_the_change() {
        let file = Scratch::new("saved.txt", "one\n");
        let mut doc = Document::from_file(file.path()).unwrap();
        file.rewrite("one\ntwo\n");
        assert!(doc.disk_changed());
        doc.save().unwrap();
        assert!(!doc.disk_changed());
    }

    #[test]
    fn reloading_adopts_the_new_text() {
        let file = Scratch::new("reload.txt", "one\n");
        let mut doc = Document::from_file(file.path()).unwrap();
        file.rewrite("two\nthree\n");
        doc.reload().unwrap();
        assert_eq!(doc.text(), "two\nthree\n");
        assert!(!doc.is_dirty());
        assert!(!doc.disk_changed());
    }

    #[test]
    fn reloading_keeps_the_read_only_guard() {
        let file = Scratch::new("guarded.txt", "one\n");
        let mut doc = Document::from_file(file.path()).unwrap();
        doc.set_read_only(true);
        file.rewrite("two\n");
        doc.reload().unwrap();
        assert!(doc.is_read_only());
        assert_eq!(doc.text(), "two\n");
    }

    #[test]
    fn a_conflict_is_reported_once_per_change() {
        let file = Scratch::new("conflict.txt", "one\n");
        let mut doc = Document::from_file(file.path()).unwrap();
        file.rewrite("one\ntwo\n");
        assert!(doc.disk_changed());
        doc.note_disk_conflict();
        assert!(doc.has_disk_conflict());
        // The stamp advanced, so the same change is not reported again.
        assert!(!doc.disk_changed());
        // A later change still is.
        file.rewrite("one\ntwo\nthree\n");
        assert!(doc.disk_changed());
    }

    #[test]
    fn a_clean_buffer_is_reloaded() {
        let file = Scratch::new("clean-policy.txt", "one\n");
        let doc = Document::from_file(file.path()).unwrap();
        assert_eq!(doc.disk_change(), DiskChange::None);
        file.rewrite("one\ntwo\n");
        assert_eq!(doc.disk_change(), DiskChange::Reload);
    }

    #[test]
    fn a_dirty_buffer_is_flagged_not_reloaded() {
        let file = Scratch::new("dirty-policy.txt", "one\n");
        let mut doc = Document::from_file(file.path()).unwrap();
        doc.insert(0, "mine ");
        file.rewrite("theirs\nand more\n");
        assert_eq!(doc.disk_change(), DiskChange::Conflict);
        // The policy is a question, not an action: asking twice changes nothing.
        assert_eq!(doc.text(), "mine one\n");
    }

    #[test]
    fn an_untitled_buffer_has_no_policy() {
        let mut doc = Document::from_text("hello");
        doc.insert(0, "x");
        assert_eq!(doc.disk_change(), DiskChange::None);
    }

    #[test]
    fn a_conflict_does_not_touch_the_text() {
        let file = Scratch::new("kept.txt", "mine\n");
        let mut doc = Document::from_file(file.path()).unwrap();
        doc.insert(0, "still ");
        file.rewrite("theirs\nand more\n");
        doc.note_disk_conflict();
        assert_eq!(doc.text(), "still mine\n");
        assert!(doc.is_dirty());
    }

    #[test]
    fn saving_clears_the_conflict() {
        let file = Scratch::new("resolved.txt", "one\n");
        let mut doc = Document::from_file(file.path()).unwrap();
        file.rewrite("one\ntwo\n");
        doc.note_disk_conflict();
        doc.save().unwrap();
        assert!(!doc.has_disk_conflict());
    }

    #[test]
    fn dirty_flag_tracks_edits() {
        let mut doc = Document::from_text("a");
        assert!(!doc.is_dirty());
        doc.insert(0, "b");
        assert!(doc.is_dirty());
        doc.mark_clean();
        assert!(!doc.is_dirty());
    }

    #[test]
    fn char_to_line_and_back() {
        let doc = Document::from_text("ab\ncd\nef");
        assert_eq!(doc.char_to_line(0), 0);
        assert_eq!(doc.char_to_line(2), 0);
        assert_eq!(doc.char_to_line(3), 1);
        assert_eq!(doc.char_to_line(6), 2);
        assert_eq!(doc.line_to_char(1), 3);
        assert_eq!(doc.line_to_char(2), 6);
    }

    #[test]
    fn a_bookmark_toggles_on_and_off() {
        let mut doc = Document::from_text("a\nb\nc");
        assert!(!doc.is_bookmarked(1));
        assert!(doc.toggle_bookmark(1));
        assert!(doc.is_bookmarked(1));
        assert!(!doc.toggle_bookmark(1));
        assert!(!doc.is_bookmarked(1));
    }

    #[test]
    fn bookmarks_are_counted_and_cleared() {
        let mut doc = Document::from_text("a\nb\nc");
        doc.toggle_bookmark(0);
        doc.toggle_bookmark(2);
        assert_eq!(doc.bookmark_count(), 2);
        doc.clear_bookmarks();
        assert_eq!(doc.bookmark_count(), 0);
    }

    #[test]
    fn next_bookmark_wraps_past_the_end() {
        let mut doc = Document::from_text("a\nb\nc\nd");
        doc.toggle_bookmark(1);
        doc.toggle_bookmark(3);
        assert_eq!(doc.next_bookmark(0), Some(1));
        assert_eq!(doc.next_bookmark(1), Some(3));
        assert_eq!(doc.next_bookmark(3), Some(1));
    }

    #[test]
    fn prev_bookmark_wraps_past_the_start() {
        let mut doc = Document::from_text("a\nb\nc\nd");
        doc.toggle_bookmark(1);
        doc.toggle_bookmark(3);
        assert_eq!(doc.prev_bookmark(3), Some(1));
        assert_eq!(doc.prev_bookmark(1), Some(3));
        assert_eq!(doc.prev_bookmark(0), Some(3));
    }

    #[test]
    fn a_document_without_bookmarks_has_no_next() {
        let doc = Document::from_text("a\nb");
        assert_eq!(doc.next_bookmark(0), None);
        assert_eq!(doc.prev_bookmark(0), None);
    }

    #[test]
    fn a_bookmark_survives_splitting_its_line() {
        let mut doc = Document::from_text("abc\ndef");
        doc.toggle_bookmark(0);
        doc.insert(1, "\n");
        assert_eq!(doc.len_lines(), 3);
        assert!(doc.is_bookmarked(0));
        assert!(!doc.is_bookmarked(1));
    }

    #[test]
    fn a_bookmark_survives_an_edit_that_does_not_split() {
        let mut doc = Document::from_text("abc\ndef");
        doc.toggle_bookmark(1);
        doc.insert(5, "X");
        assert!(doc.is_bookmarked(1));
    }

    #[test]
    fn a_read_only_document_refuses_every_edit() {
        let mut doc = Document::from_text("abc");
        doc.set_read_only(true);
        doc.insert(1, "X");
        doc.remove(0..1);
        doc.replace(0..1, "Y");
        assert_eq!(doc.text(), "abc");
        assert!(!doc.is_dirty());
    }

    #[test]
    fn a_read_only_document_still_loads() {
        let mut doc = Document::from_text("abc");
        doc.set_read_only(true);
        doc.set_text("fresh");
        assert_eq!(doc.text(), "fresh");
    }

    #[test]
    fn clearing_read_only_lets_edits_through_again() {
        let mut doc = Document::from_text("abc");
        doc.set_read_only(true);
        doc.insert(0, "X");
        doc.set_read_only(false);
        doc.insert(0, "X");
        assert_eq!(doc.text(), "Xabc");
    }
}
