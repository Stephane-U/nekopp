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

//! Fold level computation.
//!
//! Fold levels are derived, not stored in the file: a folder walks the document
//! once and writes a level per line. The levels use Scintilla's bit layout so the
//! renderer can ask the same questions it would ask of Scintilla — is this line a
//! block header, is it blank, how deep is it.
//!
//! The brace folder reads the style array the highlighter already produced, so a
//! `{` inside a string or a comment does not open a block. That is the whole
//! reason folding runs after highlighting rather than during lexing.

use crate::document::{
    fold_number, Document, FoldLevel, FOLD_LEVEL_BASE, FOLD_LEVEL_HEADERFLAG, FOLD_LEVEL_WHITEFLAG,
};
use crate::lexer::style;

/// Computes fold levels for brace-delimited languages.
#[derive(Debug, Default, Clone, Copy)]
pub struct BraceFolder;

impl BraceFolder {
    /// Rewrites every fold level in `doc`.
    ///
    /// Styles must already be up to date; run the highlighter first.
    pub fn apply(&self, doc: &mut Document) {
        self.run(doc, 0, false);
    }

    /// Rewrites fold levels from `start` down, stopping once the levels converge
    /// with what is already stored.
    ///
    /// This is the same trick the highlighter uses: a line's fold level depends
    /// only on the depth it starts at and its own text, so once a line comes out
    /// unchanged everything below it is still correct. Typing inside a function
    /// therefore costs a handful of lines, not the whole file.
    pub fn apply_from(&self, doc: &mut Document, start: usize) {
        self.run(doc, start, true);
    }

    /// The shared walk. `converge` is off for a full pass, because a fresh
    /// document already looks converged: every level is the base level and every
    /// depth is zero.
    fn run(&self, doc: &mut Document, start: usize, converge: bool) {
        let lines = doc.len_lines();
        if lines == 0 {
            return;
        }
        let mut line = start.min(lines - 1);
        let mut depth = doc.fold_depth(line);

        while line < lines {
            let (level, end_depth) = self.scan_line(doc, line, depth);
            let converged = converge
                && line > start
                && doc.fold_depth(line) == depth
                && doc.fold_level(line) == level;
            doc.set_fold_level(line, level);
            doc.set_fold_depth(line, depth);
            depth = end_depth;
            if converged {
                break;
            }
            line += 1;
        }
    }

    /// The fold level of `line` and the depth the next line starts at.
    fn scan_line(
        &self,
        doc: &Document,
        line: usize,
        start_depth: FoldLevel,
    ) -> (FoldLevel, FoldLevel) {
        let styles = doc.line_styles(line);
        let mut depth = start_depth;
        let mut first_code: Option<char> = None;
        let mut blank = true;

        for (i, ch) in doc.line_chars(line).enumerate() {
            if !ch.is_whitespace() {
                blank = false;
            }
            let style = styles.get(i).copied().unwrap_or(style::DEFAULT);
            if style == style::STRING || style == style::COMMENT {
                continue;
            }
            if first_code.is_none() && !ch.is_whitespace() {
                first_code = Some(ch);
            }
            match ch {
                '{' => depth = depth.saturating_add(1),
                '}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }

        // A line that closes a block belongs to the outer level, so the closing
        // brace lines up with the line that opened it.
        let base = if first_code == Some('}') {
            start_depth.saturating_sub(1)
        } else {
            start_depth
        };
        let mut level = FOLD_LEVEL_BASE + base;
        if depth > start_depth {
            level |= FOLD_LEVEL_HEADERFLAG;
        }
        if blank {
            level |= FOLD_LEVEL_WHITEFLAG;
        }
        (level, depth)
    }
}

/// A folder that never folds anything, for plain text.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoFolder;

impl NoFolder {
    /// Resets every fold level to the base.
    pub fn apply(&self, doc: &mut Document) {
        let levels = vec![FOLD_LEVEL_BASE; doc.len_lines()];
        doc.set_fold_levels(&levels);
    }
}

/// The depth a line sits at, ignoring the flag bits.
pub fn depth_of(level: FoldLevel) -> FoldLevel {
    fold_number(level).saturating_sub(FOLD_LEVEL_BASE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::is_fold_header;
    use crate::highlighter::Highlighter;
    use crate::lexer::CLikeLexer;

    fn folded(text: &str) -> Document {
        let mut doc = Document::from_text(text);
        let mut hl = Highlighter::new(Box::new(CLikeLexer));
        hl.highlight_all(&mut doc);
        BraceFolder.apply(&mut doc);
        doc
    }

    #[test]
    fn a_brace_block_is_a_header() {
        let doc = folded("fn main() {\n    body();\n}\n");
        assert!(is_fold_header(doc.fold_level(0)));
        assert!(!is_fold_header(doc.fold_level(1)));
        assert_eq!(doc.fold_end(0), 1);
    }

    #[test]
    fn nested_blocks_nest() {
        let doc = folded("a {\nb {\nc\n}\n}\n");
        assert_eq!(depth_of(doc.fold_level(0)), 0);
        assert_eq!(depth_of(doc.fold_level(1)), 1);
        assert_eq!(depth_of(doc.fold_level(2)), 2);
        assert_eq!(depth_of(doc.fold_level(3)), 1);
        assert_eq!(depth_of(doc.fold_level(4)), 0);
        assert_eq!(doc.fold_end(0), 3);
        assert_eq!(doc.fold_end(1), 2);
    }

    #[test]
    fn braces_in_strings_and_comments_do_not_count() {
        let doc = folded("let s = \"{\";\n// {\nlet t = 1;\n");
        assert!(!is_fold_header(doc.fold_level(0)));
        assert!(!is_fold_header(doc.fold_level(1)));
        assert_eq!(depth_of(doc.fold_level(2)), 0);
    }

    #[test]
    fn a_closing_line_sits_at_the_outer_level() {
        let doc = folded("a {\nb\n}\n");
        assert_eq!(depth_of(doc.fold_level(2)), 0);
    }

    #[test]
    fn blank_lines_are_flagged() {
        let doc = folded("a {\n\n}\n");
        assert!(crate::document::is_fold_white(doc.fold_level(1)));
    }

    #[test]
    fn folding_hides_the_block_body() {
        let mut doc = folded("a {\nb\nc\n}\nd\n");
        assert_eq!(doc.visible_line_count(), 6);
        doc.fold(0);
        // The closing brace sits at the outer level, so it stays visible.
        assert_eq!(doc.visible_line_count(), 4);
        assert_eq!(doc.line_of_row(0), 0);
        assert_eq!(doc.line_of_row(1), 3);
        assert!(doc.is_hidden(1));
        assert!(!doc.is_hidden(0));
    }

    #[test]
    fn unfolding_restores_every_line() {
        let mut doc = folded("a {\nb\n}\n");
        doc.fold(0);
        doc.unfold(0);
        assert_eq!(doc.visible_line_count(), 4);
        assert_eq!(doc.row_of_line(1), Some(1));
    }

    #[test]
    fn reveal_line_expands_the_hiding_block() {
        let mut doc = folded("a {\nb\nc\n}\n");
        doc.fold(0);
        assert!(doc.is_hidden(2));
        doc.reveal_line(2);
        assert!(!doc.is_hidden(2));
    }

    #[test]
    fn fold_all_and_unfold_all_round_trip() {
        let mut doc = folded("a {\nb {\nc\n}\n}\n");
        doc.fold_all();
        assert_eq!(doc.visible_line_count(), 3);
        doc.unfold_all();
        assert_eq!(doc.visible_line_count(), 6);
    }

    #[test]
    fn editing_keeps_the_row_map_consistent() {
        let mut doc = folded("a {\nb\n}\n");
        doc.fold(0);
        assert_eq!(doc.visible_line_count(), 3);
        doc.insert(doc.len_chars(), "x\n");
        assert_eq!(doc.visible_line_count(), 4);
    }

    #[test]
    fn incremental_folding_matches_a_full_pass() {
        let mut doc = folded("a {\nb {\nc\n}\n}\nd {\ne\n}\n");
        let full: Vec<FoldLevel> = doc.fold_levels().to_vec();

        // Insert a line inside the first block and fold from there.
        let at = doc.line_to_char(2);
        doc.insert(at, "x {\n}\n");
        let mut hl = Highlighter::new(Box::new(CLikeLexer));
        hl.highlight_from(&mut doc, 2);
        BraceFolder.apply_from(&mut doc, 2);

        let mut reference = doc.clone();
        BraceFolder.apply(&mut reference);
        assert_eq!(doc.fold_levels(), reference.fold_levels());
        assert_ne!(doc.fold_levels(), full.as_slice());
    }

    #[test]
    fn no_folder_clears_every_level() {
        let mut doc = folded("a {\nb\n}\n");
        NoFolder.apply(&mut doc);
        assert!(!is_fold_header(doc.fold_level(0)));
    }
}
