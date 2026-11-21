//! Where per-user files live, per platform.
//!
//! Both the session and the settings need this, and getting it wrong means the
//! file is invisible to the tools that back it up, so it lives in one place.

use std::path::{Path, PathBuf};

/// The per-user configuration directory, per platform.
pub fn config_dir() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        // %APPDATA% is the roaming profile, which is what a user expects to
        // follow them between machines.
        return std::env::var_os("APPDATA").map(PathBuf::from);
    }
    if cfg!(target_os = "macos") {
        return std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join("Library").join("Application Support"));
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

/// The name of the directory the application keeps its files in.
///
/// It is the program name, not the product name: a path is typed, and `Neko++`
/// in a path would mean quoting it in every shell and escaping it in every
/// desktop file.
pub const APP_DIR: &str = "npp";

/// The directory name this program used before it was renamed.
///
/// Kept only so [`migrate_legacy`] can find the old files. Nothing writes here.
pub const LEGACY_APP_DIR: &str = "npp-rs";

/// The path of `name` inside the application's configuration directory.
pub fn config_file(name: &str) -> Option<PathBuf> {
    Some(config_dir()?.join(APP_DIR).join(name))
}

/// Moves a pre-rename configuration directory into place, once.
///
/// The rename from `npp-rs` to `npp` would otherwise present every existing user
/// with a fresh install: their theme, font size, key bindings, saved run
/// commands and session would all be gone, and nothing would say why. Renaming
/// the directory is the whole migration -- the file names inside it did not
/// change -- so this is a `rename` and not a copy.
///
/// It refuses to act when the new directory already exists, because that means
/// the migration has already run or the user has started using the new name, and
/// in both cases the old directory is the stale one. A failure is ignored: a
/// read-only home directory should not stop the editor from starting, and the
/// worst case is the fresh install the migration was meant to avoid.
///
/// Returns whether anything moved, which is what the test asserts on.
pub fn migrate_legacy(base: &Path) -> bool {
    let old = base.join(LEGACY_APP_DIR);
    let new = base.join(APP_DIR);
    if new.exists() || !old.is_dir() {
        return false;
    }
    std::fs::rename(&old, &new).is_ok()
}

/// [`migrate_legacy`] against the real configuration directory.
pub fn migrate() {
    if let Some(base) = config_dir() {
        migrate_legacy(&base);
    }
}

/// Splits a path fragment into the directory to list and the stem to match.
///
/// The directory keeps its trailing separator, so the completion can be spliced
/// back in front of the chosen entry without re-deriving it. A fragment with no
/// separator at all is all stem, which is what makes `src/ma` list `src` and
/// match `ma`.
pub fn split_fragment(fragment: &str) -> (&str, &str) {
    match fragment.rfind(['/', '\\']) {
        Some(i) => fragment.split_at(i + 1),
        None => ("", fragment),
    }
}

/// The directory a fragment's directory part names, resolved against `base`.
///
/// A relative fragment is relative to the document being edited, not to the
/// process's working directory: `src/ma` in `/home/me/project/main.rs` means
/// `/home/me/project/src`, which is the only reading that makes the completion
/// useful.
pub fn resolve_dir(base: Option<&Path>, dir: &str) -> PathBuf {
    let path = Path::new(dir);
    if path.is_absolute() {
        return path.to_path_buf();
    }
    match base {
        Some(base) => base.join(path),
        None => path.to_path_buf(),
    }
}

/// The entries of `dir` whose names start with `stem`, directories first.
///
/// Directories come first and carry a trailing separator, because a directory is
/// a step on the way to a file and a file is the end of the path. Hidden entries
/// are offered only when the stem itself starts with a dot, so a completion in
/// the middle of a path does not fill the list with `.git`.
pub fn complete_path(dir: &Path, stem: &str, limit: usize) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(stem) || name == stem {
            continue;
        }
        if name.starts_with('.') && !stem.starts_with('.') {
            continue;
        }
        if entry.path().is_dir() {
            dirs.push(format!("{name}/"));
        } else {
            files.push(name);
        }
    }
    dirs.sort();
    files.sort();
    dirs.extend(files);
    dirs.truncate(limit);
    dirs
}

/// Writes `text` to `path`, creating the directory first.
///
/// Failures are ignored on purpose: a read-only home directory should not stop
/// the editor from working.
pub fn write(path: &PathBuf, text: &str) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_config_file_sits_under_the_app_directory() {
        let Some(path) = config_file("settings.txt") else {
            return;
        };
        assert!(path.ends_with("npp/settings.txt"));
    }

    #[test]
    fn the_old_directory_is_moved_into_place() {
        let scratch = Scratch::new("migrate");
        scratch.dir(LEGACY_APP_DIR);
        std::fs::write(scratch.0.join(LEGACY_APP_DIR).join("settings.txt"), b"theme dark")
            .expect("old settings");
        assert!(migrate_legacy(&scratch.0));
        assert!(!scratch.0.join(LEGACY_APP_DIR).exists());
        assert_eq!(
            std::fs::read_to_string(scratch.0.join(APP_DIR).join("settings.txt")).unwrap(),
            "theme dark"
        );
    }

    #[test]
    fn a_second_migration_does_nothing() {
        let scratch = Scratch::new("migrate-twice");
        scratch.dir(LEGACY_APP_DIR);
        assert!(migrate_legacy(&scratch.0));
        assert!(!migrate_legacy(&scratch.0));
    }

    #[test]
    fn an_existing_new_directory_wins_over_the_old_one() {
        let scratch = Scratch::new("migrate-both");
        scratch.dir(LEGACY_APP_DIR);
        scratch.dir(APP_DIR);
        std::fs::write(scratch.0.join(APP_DIR).join("settings.txt"), b"theme light")
            .expect("new settings");
        assert!(!migrate_legacy(&scratch.0));
        assert_eq!(
            std::fs::read_to_string(scratch.0.join(APP_DIR).join("settings.txt")).unwrap(),
            "theme light"
        );
    }

    #[test]
    fn a_fresh_install_has_nothing_to_migrate() {
        let scratch = Scratch::new("migrate-fresh");
        assert!(!migrate_legacy(&scratch.0));
    }

    #[test]
    fn a_fragment_splits_at_its_last_separator() {
        assert_eq!(split_fragment("src/main/ma"), ("src/main/", "ma"));
    }

    #[test]
    fn a_fragment_with_no_separator_is_all_stem() {
        assert_eq!(split_fragment("main"), ("", "main"));
    }

    #[test]
    fn a_fragment_ending_in_a_separator_has_an_empty_stem() {
        assert_eq!(split_fragment("src/"), ("src/", ""));
    }

    #[test]
    fn an_absolute_directory_is_left_alone() {
        assert_eq!(resolve_dir(Some(Path::new("/base")), "/usr/"), Path::new("/usr/"));
    }

    #[test]
    fn a_relative_directory_resolves_against_the_base() {
        assert_eq!(
            resolve_dir(Some(Path::new("/base")), "src/"),
            Path::new("/base/src/")
        );
    }

    #[test]
    fn an_empty_directory_resolves_to_the_base() {
        assert_eq!(resolve_dir(Some(Path::new("/base")), ""), Path::new("/base"));
    }

    #[test]
    fn a_missing_base_leaves_the_directory_relative() {
        assert_eq!(resolve_dir(None, "src/"), Path::new("src/"));
    }

    /// A scratch directory that removes itself, so the path tests do not leak.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("npp-paths-{name}"));
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
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn completion_lists_matching_entries() {
        let scratch = Scratch::new("matching");
        scratch.file("main.rs");
        scratch.file("mod.rs");
        scratch.file("other.txt");
        assert_eq!(
            complete_path(&scratch.0, "m", 10),
            vec!["main.rs".to_string(), "mod.rs".to_string()]
        );
    }

    #[test]
    fn directories_come_first_and_carry_a_separator() {
        let scratch = Scratch::new("dirs");
        scratch.file("main.rs");
        scratch.dir("mod");
        assert_eq!(
            complete_path(&scratch.0, "m", 10),
            vec!["mod/".to_string(), "main.rs".to_string()]
        );
    }

    #[test]
    fn hidden_entries_need_a_dot_in_the_stem() {
        let scratch = Scratch::new("hidden");
        scratch.file(".gitignore");
        scratch.file("main.rs");
        assert_eq!(complete_path(&scratch.0, "", 10), vec!["main.rs".to_string()]);
        assert_eq!(
            complete_path(&scratch.0, ".", 10),
            vec![".gitignore".to_string()]
        );
    }

    #[test]
    fn the_exact_name_is_not_offered_again() {
        let scratch = Scratch::new("exact");
        scratch.file("main.rs");
        assert!(complete_path(&scratch.0, "main.rs", 10).is_empty());
    }

    #[test]
    fn completion_is_capped() {
        let scratch = Scratch::new("capped");
        for i in 0..20 {
            scratch.file(&format!("file{i}.rs"));
        }
        assert_eq!(complete_path(&scratch.0, "file", 5).len(), 5);
    }

    #[test]
    fn a_missing_directory_completes_to_nothing() {
        assert!(complete_path(Path::new("/nonexistent-npp"), "", 10).is_empty());
    }
}
