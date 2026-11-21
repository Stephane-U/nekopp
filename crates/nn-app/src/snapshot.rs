//! The list of unsaved buffers, written while the editor is running.
//!
//! A backup on save protects the file you had. It does nothing for the work you
//! have not saved yet, which is the work a crash actually costs you: an hour of
//! typing in a buffer that was never written anywhere.
//!
//! So every few seconds the editor writes the contents of each modified buffer
//! into its own directory and records what it wrote here. On the next launch the
//! manifest is read back and the buffers reappear, still marked as modified, so
//! the user can save them or throw them away. Nothing is written over a file the
//! user owns — the snapshot lives entirely inside the configuration directory.
//!
//! The manifest is a separate file from the buffers because it is the index: the
//! buffer files are named by position, and only the manifest knows which path
//! and which caret each one belongs to.
//!
//! Format, one entry per line, fixed-width fields first so a path with spaces
//! needs no quoting:
//!
//! ```text
//! entry <line> <column> <file> <path>
//! ```
//!
//! `path` is the rest of the line and is empty for a buffer that was never
//! saved. `file` is a name inside the snapshot directory.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// One buffer that had unsaved changes when the snapshot was taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotEntry {
    /// Where the buffer came from, or `None` for one that was never saved.
    pub path: Option<PathBuf>,
    /// The caret's line, zero-based.
    pub line: usize,
    /// The caret's column, in characters.
    pub column: usize,
    /// The file inside the snapshot directory holding the text.
    pub file: String,
}

/// Every buffer that had unsaved changes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// In tab order, so a restore puts them back where they were.
    pub entries: Vec<SnapshotEntry>,
}

impl Snapshot {
    /// The file the manifest is stored in.
    pub fn path() -> Option<PathBuf> {
        crate::paths::config_file("snapshot.txt")
    }

    /// The directory the buffer files live in.
    ///
    /// A subdirectory of the configuration directory rather than the directory
    /// itself, so a restore can clear it wholesale without touching the settings
    /// or the session.
    pub fn dir() -> Option<PathBuf> {
        crate::paths::config_file("snapshot")
    }

    /// Reads the manifest, or an empty one when there is none.
    pub fn load() -> Snapshot {
        Self::path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    /// Writes the manifest, then deletes the buffer files it no longer names.
    ///
    /// Failures are ignored on purpose: a read-only home directory should not
    /// stop the editor from working, and the next tick will try again.
    ///
    /// The pruning is not housekeeping. Buffer files are named by position, so a
    /// buffer that becomes clean -- saved, or reloaded from disk -- drops out of
    /// the manifest and every later buffer shifts down one index. Without the
    /// prune, the file that used to hold the last buffer stays on disk under a
    /// name nothing refers to, holding a copy of a document the user believes is
    /// only on their own disk. It is never read again, so it is not a correctness
    /// bug, but it is a copy of their text left in a configuration directory
    /// forever, and that is worth one `read_dir`.
    pub fn save(&self) {
        let Some(path) = Self::path() else {
            return;
        };
        crate::paths::write(&path, &self.render());
        if let Some(dir) = Self::dir() {
            let keep: Vec<&str> = self.entries.iter().map(|e| e.file.as_str()).collect();
            let _ = Self::prune_in(&dir, &keep);
        }
    }

    /// Deletes every `*.txt` in `dir` that `keep` does not name.
    ///
    /// Split out from `save` for the same reason `write_buffer_in` is: the
    /// behaviour is worth a test, and a test cannot be allowed to touch the
    /// user's real snapshot directory.
    ///
    /// Only `.txt` is considered, so a stray file of another kind in the
    /// directory is left alone rather than being deleted on a guess.
    pub fn prune_in(dir: &Path, keep: &[&str]) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)?.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !name.ends_with(".txt") || keep.contains(&name) {
                continue;
            }
            let _ = std::fs::remove_file(entry.path());
        }
        Ok(())
    }

    /// Whether there is nothing to restore.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many buffers the snapshot holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Parses the on-disk format.
    ///
    /// Malformed lines are skipped rather than rejected: a manifest written by a
    /// newer version should still restore the buffers this one understands, and
    /// a half-written manifest from a crash mid-save should restore the entries
    /// that made it to disk.
    pub fn parse(text: &str) -> Snapshot {
        let mut snapshot = Snapshot::default();
        for line in text.lines() {
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some(rest) = line.strip_prefix("entry ") else {
                continue;
            };
            // Four fields, the last of which may be empty and may contain spaces.
            let mut parts = rest.splitn(4, ' ');
            let (Some(line_no), Some(column), Some(file)) =
                (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let (Ok(line_no), Ok(column)) = (line_no.parse::<usize>(), column.parse::<usize>())
            else {
                continue;
            };
            if file.is_empty() {
                continue;
            }
            let path = parts.next().unwrap_or_default();
            snapshot.entries.push(SnapshotEntry {
                path: (!path.is_empty()).then(|| PathBuf::from(path)),
                line: line_no,
                column,
                file: file.to_string(),
            });
        }
        snapshot
    }

    /// Renders the on-disk format.
    pub fn render(&self) -> String {
        let mut out = String::from("# Neko++ snapshot\n");
        for entry in &self.entries {
            out.push_str(&format!(
                "entry {} {} {} {}\n",
                entry.line,
                entry.column,
                entry.file,
                entry
                    .path
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ));
        }
        out
    }

    /// The name of the file holding the text of the buffer at `index`.
    ///
    /// Positional rather than derived from the path: an untitled buffer has no
    /// path to derive one from, and a buffer that is renamed by Save As would
    /// otherwise leave its snapshot behind under the old name.
    pub fn file_name(index: usize) -> String {
        format!("{index}.txt")
    }

    /// Writes `text` as the snapshot of the buffer at `index`.
    pub fn write_buffer(index: usize, text: &str) -> std::io::Result<()> {
        let Some(dir) = Self::dir() else {
            return Ok(());
        };
        Self::write_buffer_in(&dir, &Self::file_name(index), text)
    }

    /// Reads back the text written by `write_buffer`.
    pub fn read_buffer(file: &str) -> std::io::Result<String> {
        let dir = Self::dir().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "no configuration directory")
        })?;
        Self::read_buffer_in(&dir, file)
    }

    /// Writes one buffer file into `dir`.
    ///
    /// Split out from `write_buffer` so the round trip can be tested against a
    /// temporary directory rather than the user's real one.
    pub fn write_buffer_in(dir: &Path, file: &str, text: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join(file), text)
    }

    /// Reads one buffer file back out of `dir`.
    pub fn read_buffer_in(dir: &Path, file: &str) -> std::io::Result<String> {
        std::fs::read_to_string(dir.join(file))
    }

    /// Deletes the manifest and every buffer file.
    ///
    /// Called once a snapshot has been restored, and when the user turns the
    /// feature off. A snapshot that has been read is stale: leaving it would
    /// restore the same buffers again on the next launch, on top of whatever the
    /// user has done since.
    pub fn clear() {
        if let Some(path) = Self::path() {
            let _ = std::fs::remove_file(path);
        }
        if let Some(dir) = Self::dir() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// Whether `path` is already one of the entries.
    pub fn contains(&self, path: &Path) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.path.as_deref() == Some(path))
    }
}

/// How long until the next snapshot, or `None` when none is wanted.
///
/// `None` covers both "the feature is off" and "there is nothing unsaved", which
/// are the same answer to the event loop: do not wake for this. A zero result
/// means the interval has already elapsed and the snapshot is due now.
pub fn autosave_wait(seconds: u32, dirty: bool, elapsed: Duration) -> Option<Duration> {
    if seconds == 0 || !dirty {
        return None;
    }
    Some(Duration::from_secs(u64::from(seconds)).saturating_sub(elapsed))
}

/// The earlier of two optional waits.
///
/// Written out rather than `a.min(b)` because `Option`'s ordering puts `None`
/// below `Some`, which would read "no timer" as "wake immediately" and spin the
/// event loop at full speed. Here `None` means "nothing wants this timer", so it
/// loses to any real wait.
pub fn soonest(a: Option<Duration>, b: Option<Duration>) -> Option<Duration> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: Option<&str>, line: usize, column: usize, file: &str) -> SnapshotEntry {
        SnapshotEntry {
            path: path.map(PathBuf::from),
            line,
            column,
            file: file.to_string(),
        }
    }

    #[test]
    fn an_empty_snapshot_round_trips() {
        let snapshot = Snapshot::default();
        assert!(snapshot.is_empty());
        assert_eq!(Snapshot::parse(&snapshot.render()), snapshot);
    }

    #[test]
    fn a_saved_buffer_round_trips() {
        let snapshot = Snapshot {
            entries: vec![entry(Some("/src/main.rs"), 12, 4, "0.txt")],
        };
        assert_eq!(Snapshot::parse(&snapshot.render()), snapshot);
    }

    #[test]
    fn an_untitled_buffer_round_trips_with_no_path() {
        let snapshot = Snapshot {
            entries: vec![entry(None, 0, 0, "1.txt")],
        };
        let text = snapshot.render();
        assert!(text.contains("entry 0 0 1.txt \n"), "{text}");
        assert_eq!(Snapshot::parse(&text), snapshot);
    }

    #[test]
    fn a_path_with_spaces_survives() {
        let snapshot = Snapshot {
            entries: vec![entry(Some("/home/me/my notes/a file.txt"), 3, 7, "2.txt")],
        };
        assert_eq!(Snapshot::parse(&snapshot.render()), snapshot);
    }

    #[test]
    fn tab_order_is_kept() {
        let snapshot = Snapshot {
            entries: vec![
                entry(Some("/a"), 0, 0, "0.txt"),
                entry(None, 1, 1, "1.txt"),
                entry(Some("/c"), 2, 2, "2.txt"),
            ],
        };
        let parsed = Snapshot::parse(&snapshot.render());
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed.entries[1].path, None);
        assert_eq!(parsed.entries[2].path, Some(PathBuf::from("/c")));
    }

    #[test]
    fn a_truncated_line_is_skipped_and_the_rest_kept() {
        // What a crash mid-write leaves behind.
        let text = "entry 0 0 0.txt /a\nentry 1 1\nentry 2 2 2.txt /c\n";
        let parsed = Snapshot::parse(text);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed.entries[1].path, Some(PathBuf::from("/c")));
    }

    #[test]
    fn a_line_with_a_bad_number_is_skipped() {
        let text = "entry x 0 0.txt /a\nentry 1 y 1.txt /b\nentry 2 2 2.txt /c\n";
        assert_eq!(Snapshot::parse(text).len(), 1);
    }

    #[test]
    fn an_entry_with_no_file_is_skipped() {
        // Nothing to read the text from, so restoring it would produce an empty
        // buffer where the user had work.
        assert!(Snapshot::parse("entry 0 0  /a\n").is_empty());
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let text = "# Neko++ snapshot\n\nentry 0 0 0.txt /a\n";
        assert_eq!(Snapshot::parse(text).len(), 1);
    }

    #[test]
    fn an_unknown_keyword_is_ignored() {
        assert!(Snapshot::parse("buffer 0 0 0.txt /a\n").is_empty());
    }

    #[test]
    fn buffer_names_are_positional() {
        assert_eq!(Snapshot::file_name(0), "0.txt");
        assert_eq!(Snapshot::file_name(11), "11.txt");
    }

    #[test]
    fn autosave_waits_the_interval_then_is_due() {
        let interval = Duration::from_secs(7);
        assert_eq!(autosave_wait(7, true, Duration::ZERO), Some(interval));
        assert_eq!(
            autosave_wait(7, true, Duration::from_secs(2)),
            Some(Duration::from_secs(5))
        );
        // Past the interval the wait is zero, not negative and not a wrap.
        assert_eq!(autosave_wait(7, true, Duration::from_secs(30)), Some(Duration::ZERO));
    }

    #[test]
    fn autosave_wants_nothing_when_off_or_clean() {
        assert_eq!(autosave_wait(0, true, Duration::ZERO), None);
        assert_eq!(autosave_wait(7, false, Duration::ZERO), None);
        assert_eq!(autosave_wait(0, false, Duration::ZERO), None);
    }

    #[test]
    fn the_earlier_of_two_waits_wins() {
        let short = Duration::from_secs(1);
        let long = Duration::from_secs(7);
        assert_eq!(soonest(Some(long), Some(short)), Some(short));
        assert_eq!(soonest(Some(short), Some(long)), Some(short));
    }

    #[test]
    fn a_missing_timer_loses_rather_than_winning() {
        // `Option::min` would return `None` here and the loop would never sleep.
        let wait = Duration::from_secs(7);
        assert_eq!(soonest(None, Some(wait)), Some(wait));
        assert_eq!(soonest(Some(wait), None), Some(wait));
        assert_eq!(soonest(None, None), None);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("npp-snapshot-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn a_buffer_round_trips_through_the_directory() {
        let dir = temp_dir("roundtrip");
        let text = "fn main() {\n    println!(\"hi\");\n}\n";
        Snapshot::write_buffer_in(&dir, &Snapshot::file_name(3), text).unwrap();
        assert_eq!(Snapshot::read_buffer_in(&dir, &Snapshot::file_name(3)).unwrap(), text);
    }

    #[test]
    fn writing_a_buffer_creates_the_directory() {
        let dir = temp_dir("nested").join("a").join("b");
        Snapshot::write_buffer_in(&dir, "0.txt", "x").unwrap();
        assert!(dir.join("0.txt").exists());
    }

    #[test]
    fn reading_a_buffer_that_is_not_there_is_an_error() {
        let dir = temp_dir("absent");
        assert!(Snapshot::read_buffer_in(&dir, "9.txt").is_err());
    }

    #[test]
    fn a_snapshot_knows_which_paths_it_holds() {
        let snapshot = Snapshot {
            entries: vec![entry(Some("/a"), 0, 0, "0.txt"), entry(None, 0, 0, "1.txt")],
        };
        assert!(snapshot.contains(Path::new("/a")));
        assert!(!snapshot.contains(Path::new("/b")));
        // An untitled buffer matches no path, including the empty one.
        assert!(!snapshot.contains(Path::new("")));
    }

    #[test]
    fn pruning_removes_the_buffers_the_manifest_no_longer_names() {
        let dir = temp_dir("prune");
        for name in ["0.txt", "1.txt", "2.txt"] {
            Snapshot::write_buffer_in(&dir, name, "text").unwrap();
        }
        // A buffer became clean, so the manifest now names only the first two.
        Snapshot::prune_in(&dir, &["0.txt", "1.txt"]).unwrap();
        assert!(dir.join("0.txt").exists());
        assert!(dir.join("1.txt").exists());
        assert!(!dir.join("2.txt").exists());
    }

    #[test]
    fn pruning_leaves_files_of_other_kinds_alone() {
        let dir = temp_dir("prune-other");
        Snapshot::write_buffer_in(&dir, "0.txt", "text").unwrap();
        std::fs::write(dir.join("notes.md"), "keep me").unwrap();
        Snapshot::prune_in(&dir, &[]).unwrap();
        assert!(!dir.join("0.txt").exists());
        assert!(dir.join("notes.md").exists());
    }

    #[test]
    fn pruning_a_directory_that_is_not_there_is_an_error_not_a_panic() {
        let dir = temp_dir("prune-absent");
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(Snapshot::prune_in(&dir, &[]).is_err());
    }
}
