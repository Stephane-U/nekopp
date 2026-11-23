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

//! Which shaper a line of text needs.
//!
//! cosmic-text offers two: `Shaping::Basic`, which picks the one font the
//! attributes name and draws a box for every glyph that font does not have, and
//! `Shaping::Advanced`, which also searches the installed fonts for the missing
//! ones. Basic does not merely shape worse -- it does no font fallback at all,
//! so a menu label in Japanese, a tab named in Korean or a path with CJK in it
//! comes out as a row of boxes whatever fonts are installed.
//!
//! Advanced costs more: it runs the complex-text shaper, with bidi reordering,
//! contextual forms and mark positioning. Monospace ASCII needs none of that,
//! and skipping it measured 2-5x faster per line -- the difference between a
//! 60 ms hitch and a 6 ms one when a whole screenful is reshaped at once. So
//! the cheap shaper keeps the ASCII case, which is what a source file mostly
//! is, and everything else takes the fallback.

use cosmic_text::Shaping;

/// Picks the cheapest shaper that is still correct for `text`.
pub fn shaping_for(text: &str) -> Shaping {
    if text.is_ascii() {
        Shaping::Basic
    } else {
        Shaping::Advanced
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_takes_the_cheap_shaper() {
        assert_eq!(shaping_for(""), Shaping::Basic);
        assert_eq!(shaping_for("File"), Shaping::Basic);
        // A menu label that happens to be ASCII: German, Spanish, Turkish.
        assert_eq!(shaping_for("Datei"), Shaping::Basic);
        assert_eq!(shaping_for("Arquivo"), Shaping::Basic);
    }

    #[test]
    fn anything_else_asks_for_the_fallback() {
        // Cyrillic and Greek have glyphs in most monospace fonts, but not in
        // all of them, and the box is not worth betting on.
        assert_eq!(shaping_for("Файл"), Shaping::Advanced);
        assert_eq!(shaping_for("Αρχείο"), Shaping::Advanced);
        assert_eq!(shaping_for("ファイル"), Shaping::Advanced);
        assert_eq!(shaping_for("파일"), Shaping::Advanced);
        // One symbol outside ASCII is enough.
        assert_eq!(shaping_for("Datei 10 €"), Shaping::Advanced);
    }
}
