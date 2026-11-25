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

//! Line-ending modes.
//!
//! Documents are normalised to `\n` internally so the rope never has to deal
//! with multi-byte terminators. The original mode is remembered and restored on
//! save.

use std::borrow::Cow;

/// The line terminator a document is written with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EolMode {
    /// `\n` — Unix, macOS 10+.
    #[default]
    Lf,
    /// `\r\n` — Windows.
    CrLf,
    /// `\r` — classic Mac OS.
    Cr,
}

impl EolMode {
    /// The literal bytes this mode writes.
    pub const fn as_str(self) -> &'static str {
        match self {
            EolMode::Lf => "\n",
            EolMode::CrLf => "\r\n",
            EolMode::Cr => "\r",
        }
    }

    /// Human-readable name, as shown in the status bar.
    pub const fn name(self) -> &'static str {
        match self {
            EolMode::Lf => "LF",
            EolMode::CrLf => "CRLF",
            EolMode::Cr => "CR",
        }
    }

    /// Picks the mode that terminates the most lines in `text`.
    ///
    /// Ties resolve towards `Lf`, and text with no terminator at all reports
    /// `Lf` — the same default Notepad++ uses on a fresh document.
    pub fn detect(text: &str) -> Self {
        let bytes = text.as_bytes();
        let mut crlf = 0usize;
        let mut lf = 0usize;
        let mut cr = 0usize;

        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\r' => {
                    if bytes.get(i + 1) == Some(&b'\n') {
                        crlf += 1;
                        i += 2;
                        continue;
                    }
                    cr += 1;
                }
                b'\n' => lf += 1,
                _ => {}
            }
            i += 1;
        }

        if crlf >= lf && crlf >= cr && crlf > 0 {
            EolMode::CrLf
        } else if cr > lf && cr > 0 {
            EolMode::Cr
        } else {
            EolMode::Lf
        }
    }

    /// Rewrites every terminator in `text` to `\n`.
    ///
    /// Returns the normalised text and the mode that was detected on the way in.
    /// Text that needs no rewriting -- no carriage return anywhere, which is
    /// every file written on this side of the last thirty years -- comes back
    /// borrowed rather than copied, so opening a large file does not duplicate
    /// it a second time.
    pub fn normalise(text: &str) -> (Cow<'_, str>, EolMode) {
        let mode = Self::detect(text);
        if !text.as_bytes().contains(&b'\r') {
            return (Cow::Borrowed(text), mode);
        }

        let mut out = String::with_capacity(text.len());
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\r' {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            } else {
                out.push(c);
            }
        }
        (Cow::Owned(out), mode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_lf() {
        assert_eq!(EolMode::detect("a\nb\nc"), EolMode::Lf);
    }

    #[test]
    fn detects_crlf() {
        assert_eq!(EolMode::detect("a\r\nb\r\nc"), EolMode::CrLf);
    }

    #[test]
    fn detects_cr() {
        assert_eq!(EolMode::detect("a\rb\rc"), EolMode::Cr);
    }

    #[test]
    fn empty_text_is_lf() {
        assert_eq!(EolMode::detect(""), EolMode::Lf);
        assert_eq!(EolMode::detect("no terminator"), EolMode::Lf);
    }

    #[test]
    fn majority_wins() {
        assert_eq!(EolMode::detect("a\r\nb\r\nc\nd"), EolMode::CrLf);
        assert_eq!(EolMode::detect("a\nb\nc\r\nd"), EolMode::Lf);
    }

    #[test]
    fn normalise_collapses_all_terminators() {
        let (text, mode) = EolMode::normalise("a\r\nb\rc\nd");
        assert_eq!(text, "a\nb\nc\nd");
        // One of each terminator: the tie resolves to CRLF.
        assert_eq!(mode, EolMode::CrLf);
    }

    #[test]
    fn normalise_leaves_lf_untouched() {
        let (text, mode) = EolMode::normalise("a\nb");
        assert_eq!(text, "a\nb");
        assert_eq!(mode, EolMode::Lf);
    }

    #[test]
    fn normalise_borrows_text_it_does_not_have_to_rewrite() {
        // A file with no carriage return anywhere is handed back as the slice
        // it came in as; copying it would double a large document on the way to
        // the rope for no change at all.
        let (text, _) = EolMode::normalise("a\nb\nc");
        assert!(matches!(text, Cow::Borrowed(_)), "{text:?}");
        // One carriage return is enough to need a new string.
        let (text, mode) = EolMode::normalise("a\r\nb");
        assert!(matches!(text, Cow::Owned(_)), "{text:?}");
        assert_eq!(mode, EolMode::CrLf);
    }
}
