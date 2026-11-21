//! Keeping the previous version of a file when it is overwritten.
//!
//! A save is the one operation in an editor that destroys data on purpose: the
//! bytes on disk are replaced by whatever is in the buffer. Everything else here
//! is recoverable — an undo, a reload, a re-open — but a save that overwrote
//! something the user wanted is gone unless a copy was taken first.
//!
//! Notepad++ offers two shapes of that copy, and they answer different worries:
//!
//! - **Simple** puts `foo.txt.bak` next to `foo.txt`. It is the one you find
//!   without knowing the editor exists, and it travels with the file. It also
//!   litters the source tree, which is why it is off by default here.
//! - **Verbose** puts a timestamped copy in the application's own directory. It
//!   keeps several generations, so a save made an hour ago is still reachable,
//!   and it works on read-only media where a sibling file cannot be written.
//!
//! The policy is here rather than in the binary because it is all path
//! arithmetic and file copying, which is exactly the sort of thing that is
//! wrong in a way nobody notices until they need the backup.

use std::path::{Path, PathBuf};

/// The longest file name a verbose backup will produce before it gives up on
/// being readable and falls back to a hash.
///
/// Most filesystems cap a single name at 255 bytes. A deep path encoded into one
/// name can reach that, and a backup that fails to write because the name was
/// too long is worse than one with an opaque name.
pub const MAX_NAME: usize = 120;

/// What to do with a file's previous contents when it is saved over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackupMode {
    /// Overwrite. The default: writing `.bak` files into someone's source tree
    /// without being asked is a surprise, and version control already exists.
    #[default]
    None,
    /// `foo.txt` becomes `foo.txt.bak` beside it.
    Simple,
    /// A timestamped copy in the application's backup directory.
    Verbose,
}

impl BackupMode {
    /// Every mode, in the order the preferences dialog cycles them.
    pub const ALL: [BackupMode; 3] = [BackupMode::None, BackupMode::Simple, BackupMode::Verbose];

    /// The keyword written to the settings file.
    pub fn key(self) -> &'static str {
        match self {
            BackupMode::None => "none",
            BackupMode::Simple => "simple",
            BackupMode::Verbose => "verbose",
        }
    }

    /// The label shown in the preferences dialog.
    pub fn label(self) -> &'static str {
        match self {
            BackupMode::None => "None",
            BackupMode::Simple => "Simple",
            BackupMode::Verbose => "Verbose",
        }
    }

    /// Reads a keyword back, or `None` when it is not one of ours.
    pub fn from_key(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.key() == text)
    }

    /// The next mode, wrapping. What a switch row does when stepped.
    pub fn next(self) -> Self {
        let at = Self::ALL.iter().position(|mode| *mode == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }

    /// The previous mode, wrapping.
    pub fn previous(self) -> Self {
        let at = Self::ALL.iter().position(|mode| *mode == self).unwrap_or(0);
        Self::ALL[(at + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// Where a simple backup of `path` goes: the same name with `.bak` appended.
///
/// Appended rather than substituted, so `foo.txt` and `foo.rs` in one directory
/// do not both become `foo.bak`. Notepad++ does the same, which means a user
/// arriving from it finds the file where they expect.
pub fn simple_backup_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".bak");
    path.with_file_name(name)
}

/// Encodes a full path into a single file name component.
///
/// Separators become `@` because a backup directory is flat: two files called
/// `main.rs` in different projects must not share one backup. The drive colon on
/// Windows goes the same way, and a leading separator is dropped so the name
/// does not start with the separator character.
pub fn encode_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '/' | '\\' | ':' => out.push('@'),
            _ => out.push(ch),
        }
    }
    out.trim_start_matches('@').to_string()
}

/// A short, stable hash of a string, as eight lowercase hex digits.
///
/// FNV-1a: four lines, no dependency, and stable across runs and machines, which
/// is what a file name needs. It is not a security hash and does not need to be.
pub fn short_hash(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08x}", hash & 0xffff_ffff)
}

/// Where a verbose backup of `path` goes, inside `dir`.
///
/// The name carries the encoded path so a human can find it, and the stamp so
/// successive saves do not destroy each other. When the encoded path is too long
/// to be a file name the readable part is dropped and the hash keeps the name
/// unique — an unreadable backup is still a backup.
pub fn verbose_backup_path(dir: &Path, path: &Path, stamp: &str) -> PathBuf {
    let encoded = encode_path(path);
    let name = if encoded.len() <= MAX_NAME {
        format!("{encoded}@{stamp}.bak")
    } else {
        let file = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_string());
        format!("{file}@{}@{stamp}.bak", short_hash(&encoded))
    };
    dir.join(name)
}

/// Copies `path` to its backup location before it is overwritten.
///
/// Returns the backup's path, or `None` when there was nothing to do: the mode
/// is off, or the file does not exist yet because this is its first save. A
/// missing file is not an error — there is no previous version to keep.
///
/// The copy is made before the write, not after, because the point is to hold
/// the bytes that are about to be replaced.
pub fn backup_before_save(
    mode: BackupMode,
    path: &Path,
    dir: &Path,
    stamp: &str,
) -> std::io::Result<Option<PathBuf>> {
    if mode == BackupMode::None || !path.is_file() {
        return Ok(None);
    }
    let target = match mode {
        BackupMode::None => return Ok(None),
        BackupMode::Simple => simple_backup_path(path),
        BackupMode::Verbose => verbose_backup_path(dir, path, stamp),
    };
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(path, &target)?;
    Ok(Some(target))
}

/// The part of a verbose backup's name before the stamp, which identifies the
/// file it came from.
fn backup_stem(name: &str) -> Option<&str> {
    let name = name.strip_suffix(".bak")?;
    let (stem, stamp) = name.rsplit_once('@')?;
    // A stamp is `YYYYMMDD-HHMMSS`. Anything else is not one of ours, and
    // deleting a file we did not write would be a poor way to save space.
    let looks_like_stamp = stamp.len() == 15
        && stamp.as_bytes()[8] == b'-'
        && stamp
            .chars()
            .enumerate()
            .all(|(at, ch)| at == 8 || ch.is_ascii_digit());
    looks_like_stamp.then_some(stem)
}

/// Deletes all but the newest `keep` verbose backups of each file in `dir`.
///
/// Returns how many were removed. Timestamps sort lexicographically because they
/// are fixed-width and most-significant-first, so "newest" needs no parsing.
///
/// Pruning is what makes timestamped backups usable: without it the directory
/// grows by one file per save forever, and a feature that fills the disk is one
/// the user turns off.
pub fn prune_backups(dir: &Path, keep: usize) -> std::io::Result<usize> {
    let keep = keep.max(1);
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        // No directory means no backups, which is not a failure.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err),
    };
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = backup_stem(&name) else {
            continue;
        };
        match groups.iter_mut().find(|(key, _)| key == stem) {
            Some((_, names)) => names.push(name),
            None => groups.push((stem.to_string(), vec![name])),
        }
    }
    let mut removed = 0;
    for (_, mut names) in groups {
        if names.len() <= keep {
            continue;
        }
        names.sort();
        let doomed = names.len() - keep;
        for name in names.into_iter().take(doomed) {
            if std::fs::remove_file(dir.join(name)).is_ok() {
                removed += 1;
            }
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("npp-backup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn the_default_mode_writes_nothing() {
        assert_eq!(BackupMode::default(), BackupMode::None);
    }

    #[test]
    fn every_mode_round_trips_through_its_key() {
        for mode in BackupMode::ALL {
            assert_eq!(BackupMode::from_key(mode.key()), Some(mode));
        }
        assert_eq!(BackupMode::from_key("sometimes"), None);
    }

    #[test]
    fn stepping_a_mode_wraps_both_ways() {
        assert_eq!(BackupMode::None.next(), BackupMode::Simple);
        assert_eq!(BackupMode::Simple.next(), BackupMode::Verbose);
        assert_eq!(BackupMode::Verbose.next(), BackupMode::None);
        assert_eq!(BackupMode::None.previous(), BackupMode::Verbose);
        assert_eq!(BackupMode::Verbose.previous(), BackupMode::Simple);
    }

    #[test]
    fn a_simple_backup_appends_rather_than_replaces_the_extension() {
        let path = simple_backup_path(Path::new("/src/foo.txt"));
        assert_eq!(path, PathBuf::from("/src/foo.txt.bak"));
        // Two files that share a stem must not share a backup.
        assert_ne!(
            simple_backup_path(Path::new("/src/foo.txt")),
            simple_backup_path(Path::new("/src/foo.rs"))
        );
    }

    #[test]
    fn encoding_a_path_flattens_the_separators() {
        assert_eq!(
            encode_path(Path::new("/home/me/src/main.rs")),
            "home@me@src@main.rs"
        );
        assert_eq!(
            encode_path(Path::new("C:\\work\\main.rs")),
            "C@@work@main.rs"
        );
        assert_eq!(encode_path(Path::new("main.rs")), "main.rs");
    }

    #[test]
    fn the_hash_is_stable_and_eight_digits() {
        assert_eq!(short_hash("hello"), short_hash("hello"));
        assert_ne!(short_hash("hello"), short_hash("hellp"));
        assert_eq!(short_hash("hello").len(), 8);
        assert!(short_hash("hello").chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn a_verbose_backup_carries_the_path_and_the_stamp() {
        let path = verbose_backup_path(
            Path::new("/backup"),
            Path::new("/home/me/main.rs"),
            "20240102-030405",
        );
        assert_eq!(
            path,
            PathBuf::from("/backup/home@me@main.rs@20240102-030405.bak")
        );
    }

    #[test]
    fn two_files_with_one_name_get_different_backups() {
        let a = verbose_backup_path(
            Path::new("/b"),
            Path::new("/one/main.rs"),
            "20240102-030405",
        );
        let b = verbose_backup_path(
            Path::new("/b"),
            Path::new("/two/main.rs"),
            "20240102-030405",
        );
        assert_ne!(a, b);
    }

    #[test]
    fn a_path_too_long_for_a_name_falls_back_to_a_hash() {
        let deep = format!("/{}/main.rs", "d".repeat(MAX_NAME * 2));
        let path = verbose_backup_path(Path::new("/b"), Path::new(&deep), "20240102-030405");
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.len() < MAX_NAME + 40, "{name}");
        assert!(name.starts_with("main.rs@"), "{name}");
        assert!(name.ends_with("@20240102-030405.bak"), "{name}");
    }

    #[test]
    fn a_long_path_still_gets_a_unique_name() {
        let one = format!("/{}/main.rs", "d".repeat(MAX_NAME * 2));
        let two = format!("/{}/main.rs", "e".repeat(MAX_NAME * 2));
        let a = verbose_backup_path(Path::new("/b"), Path::new(&one), "20240102-030405");
        let b = verbose_backup_path(Path::new("/b"), Path::new(&two), "20240102-030405");
        assert_ne!(a, b);
    }

    #[test]
    fn no_mode_copies_nothing() {
        let dir = temp_dir("none");
        let file = dir.join("a.txt");
        std::fs::write(&file, "one").unwrap();
        let made = backup_before_save(BackupMode::None, &file, &dir, "20240102-030405").unwrap();
        assert_eq!(made, None);
        assert!(!dir.join("a.txt.bak").exists());
    }

    #[test]
    fn a_simple_backup_holds_the_bytes_that_were_replaced() {
        let dir = temp_dir("simple");
        let file = dir.join("a.txt");
        std::fs::write(&file, "before").unwrap();
        let made = backup_before_save(BackupMode::Simple, &file, &dir, "20240102-030405")
            .unwrap()
            .expect("a backup");
        assert_eq!(made, dir.join("a.txt.bak"));
        std::fs::write(&file, "after").unwrap();
        assert_eq!(std::fs::read_to_string(&made).unwrap(), "before");
    }

    #[test]
    fn a_verbose_backup_lands_in_the_backup_directory() {
        let dir = temp_dir("verbose");
        let work = dir.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let file = work.join("a.txt");
        std::fs::write(&file, "before").unwrap();
        let made = backup_before_save(
            BackupMode::Verbose,
            &file,
            &dir.join("backup"),
            "20240102-030405",
        )
        .unwrap()
        .expect("a backup");
        assert!(made.starts_with(dir.join("backup")), "{made:?}");
        assert_eq!(std::fs::read_to_string(&made).unwrap(), "before");
        // The source tree is left alone.
        assert!(!work.join("a.txt.bak").exists());
    }

    #[test]
    fn a_file_that_does_not_exist_yet_has_nothing_to_back_up() {
        let dir = temp_dir("missing");
        let file = dir.join("new.txt");
        let made = backup_before_save(BackupMode::Simple, &file, &dir, "20240102-030405").unwrap();
        assert_eq!(made, None);
    }

    #[test]
    fn a_stamp_is_recognised_only_in_our_own_shape() {
        assert_eq!(backup_stem("a@20240102-030405.bak"), Some("a"));
        assert_eq!(backup_stem("a@20240102-030405.txt"), None);
        assert_eq!(backup_stem("a.bak"), None);
        assert_eq!(backup_stem("a@notastamp.bak"), None);
        assert_eq!(backup_stem("a@20240102x030405.bak"), None);
    }

    #[test]
    fn pruning_keeps_the_newest_and_drops_the_rest() {
        let dir = temp_dir("prune");
        for stamp in ["20240101-000000", "20240102-000000", "20240103-000000"] {
            std::fs::write(dir.join(format!("a@{stamp}.bak")), stamp).unwrap();
        }
        let removed = prune_backups(&dir, 2).unwrap();
        assert_eq!(removed, 1);
        assert!(!dir.join("a@20240101-000000.bak").exists());
        assert!(dir.join("a@20240102-000000.bak").exists());
        assert!(dir.join("a@20240103-000000.bak").exists());
    }

    #[test]
    fn pruning_counts_each_file_separately() {
        let dir = temp_dir("prune-groups");
        for (stem, stamp) in [
            ("a", "20240101-000000"),
            ("a", "20240102-000000"),
            ("b", "20240101-000000"),
            ("b", "20240102-000000"),
        ] {
            std::fs::write(dir.join(format!("{stem}@{stamp}.bak")), "").unwrap();
        }
        // Two files, two generations each, keeping one: two go.
        assert_eq!(prune_backups(&dir, 1).unwrap(), 2);
        assert!(dir.join("a@20240102-000000.bak").exists());
        assert!(dir.join("b@20240102-000000.bak").exists());
    }

    #[test]
    fn pruning_leaves_files_it_did_not_write() {
        let dir = temp_dir("prune-foreign");
        std::fs::write(dir.join("notes.txt"), "mine").unwrap();
        std::fs::write(dir.join("a@20240101-000000.bak"), "").unwrap();
        std::fs::write(dir.join("a@20240102-000000.bak"), "").unwrap();
        assert_eq!(prune_backups(&dir, 1).unwrap(), 1);
        assert!(dir.join("notes.txt").exists());
    }

    #[test]
    fn pruning_a_directory_that_is_not_there_is_not_an_error() {
        let dir = std::env::temp_dir().join("npp-backup-absent-does-not-exist");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(prune_backups(&dir, 3).unwrap(), 0);
    }

    #[test]
    fn pruning_keeps_at_least_one() {
        let dir = temp_dir("prune-zero");
        std::fs::write(dir.join("a@20240101-000000.bak"), "").unwrap();
        std::fs::write(dir.join("a@20240102-000000.bak"), "").unwrap();
        // A keep of zero would delete the backup that was just made.
        assert_eq!(prune_backups(&dir, 0).unwrap(), 1);
        assert!(dir.join("a@20240102-000000.bak").exists());
    }
}
