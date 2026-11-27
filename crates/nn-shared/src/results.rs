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

//! What a list of search hits says about itself.
//!
//! Both front-ends show the results of Find in Files as a list, one row per
//! matching line. The list is drawn twice -- shaped glyphs in the window, cells
//! in the terminal -- but what a row *says*, and what the header above it
//! counts, is one decision and lives here. A second wording would drift the
//! first time a plural was fixed in one of them, and the drift would be
//! invisible until someone ran both programs side by side.

use nn_core::Hit;

/// `text` cut to `columns` characters, marking the cut with an ellipsis.
///
/// Characters rather than bytes, because this is what a row can show: a cut
/// made in the middle of a multi-byte character would corrupt the one thing the
/// row exists to say, which is what the file contains.
pub fn truncate(text: &str, columns: usize) -> String {
    if text.chars().count() <= columns {
        return text.to_string();
    }
    let mut out: String = text.chars().take(columns.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// The row `hit` is shown as: `name:line: text`, cut to `columns`.
///
/// The file's name rather than its path: a search over one tree returns the same
/// directories on every row, and the row is worth more spent on the line's text.
/// The number is counted from one, as every number a reader sees is.
pub fn row(hit: &Hit, columns: usize) -> String {
    let name = hit
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("?");
    let text = format!("{}:{}: {}", name, hit.line + 1, hit.text.trim());
    truncate(&text, columns)
}

/// "1 hit" or "3 hits", for a search results header.
pub fn hits_noun(count: usize) -> String {
    if count == 1 {
        "1 hit".to_string()
    } else {
        format!("{count} hits")
    }
}

/// "1 file" or "3 files", for a search results header.
pub fn files_noun(count: usize) -> String {
    if count == 1 {
        "1 file".to_string()
    } else {
        format!("{count} files")
    }
}

/// How many different files `hits` came from.
///
/// Sorted and deduplicated rather than collected into a set: the hits arrive in
/// walk order, which groups a file's lines together, so the sort is nearly free
/// and the answer does not depend on a hasher.
pub fn count_files(hits: &[Hit]) -> usize {
    let mut paths: Vec<&std::path::Path> = hits.iter().map(|hit| hit.path.as_path()).collect();
    paths.sort();
    paths.dedup();
    paths.len()
}

/// The plain header: how many results, in how many files.
///
/// "No results" rather than "0 results in 0 files", because an empty list is the
/// one case where the count is not what the reader is asking.
pub fn summary(hits: &[Hit]) -> String {
    match hits.len() {
        0 => "No results".to_string(),
        1 => format!("1 result in {}", files_noun(count_files(hits))),
        count => format!("{count} results in {}", files_noun(count_files(hits))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn hit(path: &str, line: usize, text: &str) -> Hit {
        Hit {
            path: PathBuf::from(path),
            line,
            column: 0,
            text: text.to_string(),
            tab: None,
        }
    }

    #[test]
    fn truncate_marks_the_cut() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("abc", 4), "abc");
        // A cut made without leaving room for the mark is the mark alone, not a
        // row one character wider than it was asked for.
        assert_eq!(truncate("abcdef", 1), "…");
        assert_eq!(truncate("abcdef", 0), "…");
    }

    #[test]
    fn truncating_counts_characters_not_bytes() {
        // Three characters that take six bytes: a row of four has room for
        // three of them and the mark.
        assert_eq!(truncate("ééé", 4), "ééé");
        assert_eq!(truncate("ééééé", 4), "ééé…");
    }

    #[test]
    fn a_row_names_the_file_and_counts_the_line_from_one() {
        let one = hit("/tmp/dir/a.rs", 4, "  let x = 1;\n");
        // The line's text is trimmed, as the window has always shown it: the row
        // is a pointer to the line, not a copy of its indentation.
        assert_eq!(row(&one, 80), "a.rs:5: let x = 1;");
        assert_eq!(row(&one, 10), "a.rs:5: l…");
    }

    #[test]
    fn the_nouns_agree_with_their_counts() {
        assert_eq!(hits_noun(0), "0 hits");
        assert_eq!(hits_noun(1), "1 hit");
        assert_eq!(hits_noun(3), "3 hits");
        assert_eq!(files_noun(1), "1 file");
        assert_eq!(files_noun(2), "2 files");
    }

    #[test]
    fn the_summary_counts_results_and_the_files_they_came_from() {
        assert_eq!(summary(&[]), "No results");
        assert_eq!(summary(&[hit("/tmp/a.rs", 0, "x")]), "1 result in 1 file");
        let two_files = [
            hit("/tmp/a.rs", 0, "x"),
            hit("/tmp/a.rs", 1, "y"),
            hit("/tmp/b.rs", 0, "z"),
        ];
        assert_eq!(summary(&two_files), "3 results in 2 files");
        // One file reached twice is one file.
        assert_eq!(
            summary(&[hit("/tmp/a.rs", 0, "x"), hit("/tmp/a.rs", 9, "y")]),
            "2 results in 1 file"
        );
    }
}
