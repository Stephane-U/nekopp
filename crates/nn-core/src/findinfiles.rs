//! Searching a directory tree.
//!
//! Find-in-files is the one feature that reads files the editor has not opened,
//! so it is also the one place where a bug can hang the program on a directory
//! it should have skipped. Three limits keep that from happening: a depth cap, a
//! skip list for the directories that are never interesting, and a hit cap that
//! stops the walk as soon as the results panel is full.
//!
//! The walk is synchronous. A background thread would keep the UI responsive on
//! a huge tree, but it would also need a channel, a cancellation token, and a
//! way to merge partial results into the panel — and the hit cap means the
//! common case finishes in a few milliseconds anyway.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::search::{Search, SearchError, SearchOptions};

/// One matching line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// The file the match is in.
    pub path: PathBuf,
    /// Zero-based line number.
    pub line: usize,
    /// Zero-based column, in characters.
    pub column: usize,
    /// The line's text, trimmed for display.
    pub text: String,
}

/// What to search and where.
#[derive(Debug, Clone)]
pub struct FindInFiles {
    /// The pattern, literal or regex depending on `options`.
    pub pattern: String,
    /// The directory to walk.
    pub directory: PathBuf,
    /// Case, whole word, and how the pattern is read.
    pub options: SearchOptions,
    /// `*.rs;*.toml` style. Empty means every file.
    pub filters: Vec<String>,
    /// Stop after this many hits.
    pub max_hits: usize,
    /// Skip files larger than this.
    pub max_file_bytes: u64,
    /// How deep to recurse.
    pub max_depth: usize,
}

/// Directories that are never worth searching.
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

impl FindInFiles {
    /// A search of `directory` for `pattern`, with the defaults a menu item
    /// would use.
    pub fn new(pattern: impl Into<String>, directory: impl Into<PathBuf>) -> Self {
        Self {
            pattern: pattern.into(),
            directory: directory.into(),
            options: SearchOptions::default(),
            filters: Vec::new(),
            max_hits: 2_000,
            max_file_bytes: 8 * 1024 * 1024,
            max_depth: 24,
        }
    }

    /// Sets the `*.rs;*.toml` filter list.
    pub fn with_filters(mut self, filters: &str) -> Self {
        self.filters = filters
            .split([';', ','])
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .map(str::to_string)
            .collect();
        self
    }

    /// Runs the search.
    ///
    /// Returns the hits in walk order, which is directory order rather than
    /// sorted: sorting two thousand paths costs more than the search itself and
    /// the panel groups by file anyway.
    pub fn run(&self) -> Result<Vec<Hit>, SearchError> {
        self.run_roots(std::slice::from_ref(&self.directory))
    }

    /// Runs the search over several roots.
    ///
    /// This is find-in-projects: a project is a list of directories, and the
    /// user expects one result list, not one per root. Two roots that overlap
    /// -- a project listing both a repository and a directory inside it -- must
    /// not report the same file twice, so every file is visited once by
    /// canonical path. A root that is a file rather than a directory is
    /// searched directly, which is what makes a project that lists a single
    /// file behave the way it looks.
    pub fn run_roots(&self, roots: &[PathBuf]) -> Result<Vec<Hit>, SearchError> {
        let search = Search::new(&self.pattern, self.options)?;
        let mut hits = Vec::new();
        if search.is_empty() {
            return Ok(hits);
        }
        let mut seen: HashSet<PathBuf> = HashSet::new();
        for root in roots {
            if hits.len() >= self.max_hits {
                break;
            }
            if root.is_file() {
                self.scan_file(root, &search, &mut hits, &mut seen);
            } else {
                self.walk(root, 0, &search, &mut hits, &mut seen);
            }
        }
        Ok(hits)
    }

    fn walk(
        &self,
        dir: &Path,
        depth: usize,
        search: &Search,
        hits: &mut Vec<Hit>,
        seen: &mut HashSet<PathBuf>,
    ) {
        if depth > self.max_depth || hits.len() >= self.max_hits {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        // Collect and sort so the result order is stable between runs; a
        // filesystem's own order is arbitrary and makes the panel jump around.
        let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            if hits.len() >= self.max_hits {
                return;
            }
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if SKIP_DIRS.contains(&name) || name.starts_with('.') {
                    continue;
                }
                self.walk(&path, depth + 1, search, hits, seen);
            } else if meta.is_file() {
                if meta.len() > self.max_file_bytes || !self.matches_filter(&path) {
                    continue;
                }
                self.scan_file(&path, search, hits, seen);
            }
        }
    }

    /// Whether `path` passes the filter list.
    fn matches_filter(&self, path: &Path) -> bool {
        if self.filters.is_empty() {
            return true;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        self.filters.iter().any(|f| glob_match(f, name))
    }

    fn scan_file(
        &self,
        path: &Path,
        search: &Search,
        hits: &mut Vec<Hit>,
        seen: &mut HashSet<PathBuf>,
    ) {
        // Canonicalising costs a syscall per file and buys the guarantee that
        // overlapping roots do not double-report. A path that cannot be
        // canonicalised is still searched, keyed by itself.
        let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if !seen.insert(key) {
            return;
        }
        let Ok(bytes) = std::fs::read(path) else {
            return;
        };
        // A file that is not UTF-8 is not text this editor can show, so it is
        // not text it should report matches in.
        let Ok(text) = String::from_utf8(bytes) else {
            return;
        };
        if text.contains('\0') {
            return;
        }
        let matches = search.find_in_text(&text);
        if matches.is_empty() {
            return;
        }
        let starts = line_starts(&text);
        for range in matches {
            if hits.len() >= self.max_hits {
                return;
            }
            let line = match starts.binary_search(&range.start) {
                Ok(index) => index,
                Err(index) => index - 1,
            };
            let line_start = starts[line];
            let line_end = text[line_start..]
                .find('\n')
                .map(|i| line_start + i)
                .unwrap_or(text.len());
            let column = text[line_start..range.start].chars().count();
            hits.push(Hit {
                path: path.to_path_buf(),
                line,
                column,
                text: text[line_start..line_end].trim_end().to_string(),
            });
        }
    }
}

/// Splits `pattern in directory` into its two halves.
///
/// The directory half is only taken when it names a real directory, so a pattern
/// that happens to contain " in " still searches for itself. This is the whole
/// of the find-in-files prompt's syntax: one field, one optional suffix, no
/// second dialog.
pub fn split_query(typed: &str, default_dir: PathBuf) -> (String, PathBuf) {
    if let Some((pattern, dir)) = typed.rsplit_once(" in ") {
        let dir = dir.trim();
        if !dir.is_empty() && Path::new(dir).is_dir() {
            return (pattern.trim().to_string(), PathBuf::from(dir));
        }
    }
    (typed.trim().to_string(), default_dir)
}

/// The byte offset of the start of every line in `text`.
fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    starts
}

/// Matches `name` against a glob with `*` and `?`.
///
/// Hand-rolled rather than pulled in as a dependency: the filter field needs
/// exactly these two wildcards and nothing else, and a glob crate would be a
/// third-party parser in the path of every file in the tree.
fn glob_match(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0, 0);
    let mut star: Option<usize> = None;
    let mut resume = 0;
    while ni < name.len() {
        if pi < pattern.len() && (pattern[pi] == '?' || pattern[pi] == name[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < pattern.len() && pattern[pi] == '*' {
            star = Some(pi);
            resume = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            resume += 1;
            ni = resume;
        } else {
            return false;
        }
    }
    while pi < pattern.len() && pattern[pi] == '*' {
        pi += 1;
    }
    pi == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::SearchMode;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("npp-fif-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn glob_star_matches_any_suffix() {
        assert!(glob_match("*.rs", "main.rs"));
        assert!(!glob_match("*.rs", "main.toml"));
        assert!(glob_match("*", "anything"));
    }

    #[test]
    fn glob_question_matches_one_character() {
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "ac"));
    }

    #[test]
    fn glob_star_in_the_middle() {
        assert!(glob_match("a*b*c", "axxbyyc"));
        assert!(!glob_match("a*b*c", "axxbyy"));
    }

    #[test]
    fn line_starts_are_offsets_after_each_newline() {
        assert_eq!(line_starts("ab\ncd\n"), vec![0, 3, 6]);
        assert_eq!(line_starts(""), vec![0]);
    }

    #[test]
    fn finds_a_line_and_column() {
        let dir = temp_dir("basic");
        std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        let hits = FindInFiles::new("two", &dir).run().unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, 1);
        assert_eq!(hits[0].column, 0);
        assert_eq!(hits[0].text, "two");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reports_every_match_on_a_line() {
        let dir = temp_dir("multi");
        std::fs::write(dir.join("a.txt"), "x x x\n").unwrap();
        let hits = FindInFiles::new("x", &dir).run().unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[2].column, 4);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recurses_into_subdirectories() {
        let dir = temp_dir("recurse");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/deep.txt"), "needle\n").unwrap();
        let hits = FindInFiles::new("needle", &dir).run().unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].path.ends_with("sub/deep.txt"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn skips_target_and_git() {
        let dir = temp_dir("skip");
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join("target/x.txt"), "needle\n").unwrap();
        std::fs::write(dir.join(".git/x.txt"), "needle\n").unwrap();
        std::fs::write(dir.join("keep.txt"), "needle\n").unwrap();
        let hits = FindInFiles::new("needle", &dir).run().unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].path.ends_with("keep.txt"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn filters_by_extension() {
        let dir = temp_dir("filter");
        std::fs::write(dir.join("a.rs"), "needle\n").unwrap();
        std::fs::write(dir.join("b.txt"), "needle\n").unwrap();
        let hits = FindInFiles::new("needle", &dir)
            .with_filters("*.rs")
            .run()
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].path.ends_with("a.rs"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stops_at_the_hit_cap() {
        let dir = temp_dir("cap");
        let body: String = (0..100).map(|_| "needle\n").collect();
        std::fs::write(dir.join("a.txt"), body).unwrap();
        let mut search = FindInFiles::new("needle", &dir);
        search.max_hits = 10;
        assert_eq!(search.run().unwrap().len(), 10);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn skips_binary_files() {
        let dir = temp_dir("binary");
        std::fs::write(dir.join("a.bin"), b"needle\0needle").unwrap();
        assert!(FindInFiles::new("needle", &dir).run().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_pattern_finds_nothing() {
        let dir = temp_dir("empty");
        std::fs::write(dir.join("a.txt"), "needle\n").unwrap();
        assert!(FindInFiles::new("", &dir).run().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn several_roots_are_searched_into_one_list() {
        let one = temp_dir("roots-one");
        let two = temp_dir("roots-two");
        std::fs::write(one.join("a.txt"), "needle\n").unwrap();
        std::fs::write(two.join("b.txt"), "needle\n").unwrap();
        let search = FindInFiles::new("needle", &one);
        let hits = search.run_roots(&[one.clone(), two.clone()]).unwrap();
        assert_eq!(hits.len(), 2);
        let _ = std::fs::remove_dir_all(&one);
        let _ = std::fs::remove_dir_all(&two);
    }

    #[test]
    fn overlapping_roots_do_not_report_a_file_twice() {
        let dir = temp_dir("overlap");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/a.txt"), "needle\n").unwrap();
        let search = FindInFiles::new("needle", &dir);
        // The parent and the child both contain the file.
        let hits = search
            .run_roots(&[dir.clone(), dir.join("sub")])
            .unwrap();
        assert_eq!(hits.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_root_that_is_a_file_is_searched_directly() {
        let dir = temp_dir("fileroot");
        std::fs::write(dir.join("a.txt"), "needle\n").unwrap();
        std::fs::write(dir.join("b.txt"), "needle\n").unwrap();
        let search = FindInFiles::new("needle", &dir);
        let hits = search.run_roots(&[dir.join("a.txt")]).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].path.ends_with("a.txt"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_hit_cap_is_shared_across_roots() {
        let one = temp_dir("cap-one");
        let two = temp_dir("cap-two");
        let body: String = (0..50).map(|_| "needle\n").collect();
        std::fs::write(one.join("a.txt"), &body).unwrap();
        std::fs::write(two.join("b.txt"), &body).unwrap();
        let mut search = FindInFiles::new("needle", &one);
        search.max_hits = 10;
        assert_eq!(search.run_roots(&[one.clone(), two.clone()]).unwrap().len(), 10);
        let _ = std::fs::remove_dir_all(&one);
        let _ = std::fs::remove_dir_all(&two);
    }

    #[test]
    fn no_roots_finds_nothing() {
        let search = FindInFiles::new("needle", "/nonexistent-npp-dir");
        assert!(search.run_roots(&[]).unwrap().is_empty());
    }

    #[test]
    fn a_missing_directory_is_not_an_error() {
        let hits = FindInFiles::new("x", "/nonexistent-npp-dir").run().unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn regex_mode_works() {
        let dir = temp_dir("regex");
        std::fs::write(dir.join("a.txt"), "fn main() {}\nlet x = 1;\n").unwrap();
        let mut search = FindInFiles::new(r"fn\s+\w+", &dir);
        search.options.mode = SearchMode::Regex;
        let hits = search.run().unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_query_without_a_directory_uses_the_default() {
        let (pattern, dir) = split_query("needle", PathBuf::from("/tmp"));
        assert_eq!(pattern, "needle");
        assert_eq!(dir, PathBuf::from("/tmp"));
    }

    #[test]
    fn a_query_with_a_real_directory_splits() {
        let (pattern, dir) = split_query("needle in /tmp", PathBuf::from("/elsewhere"));
        assert_eq!(pattern, "needle");
        assert_eq!(dir, PathBuf::from("/tmp"));
    }

    #[test]
    fn a_query_with_a_missing_directory_keeps_the_whole_pattern() {
        let (pattern, dir) = split_query("needle in /nope-npp", PathBuf::from("/tmp"));
        assert_eq!(pattern, "needle in /nope-npp");
        assert_eq!(dir, PathBuf::from("/tmp"));
    }

    #[test]
    fn a_query_is_trimmed() {
        let (pattern, _) = split_query("  needle  ", PathBuf::from("/tmp"));
        assert_eq!(pattern, "needle");
    }

    #[test]
    fn an_invalid_regex_is_an_error() {
        let mut search = FindInFiles::new("(", "/tmp");
        search.options.mode = SearchMode::Regex;
        assert!(search.run().is_err());
    }
}
