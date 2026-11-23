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

//! The font the program carries with it.
//!
//! cosmic-text draws with whatever fontdb found installed. That is nearly
//! enough: Greek, Cyrillic, Japanese and Hebrew come out right because some
//! system font carries those scripts. Korean does not. The one face on this
//! machine that claims any Hangul at all claims U+AC00 and nothing beside it, so
//! every Korean menu label, tab title and status message drew as a row of boxes
//! -- and an editor that renders a source file as boxes is not usable for the
//! language, whatever the localization file says.
//!
//! Installing a package is not something a program may do on its own, so the
//! program ships the missing script instead. `assets/NekoHangul-Regular.ttf` is a
//! subset of NanumGothic Regular holding all 11172 syllables of U+AC00-U+D7A3,
//! both jamo blocks, Latin-1 and general punctuation, which is what Korean text
//! actually needs; the rest of the font -- Hanja, the fuller Latin, the hinted
//! outlines -- is dropped, and 1.3 MB is what remains of the original 2 MB.
//!
//! NanumGothic is under the SIL Open Font License 1.1, kept beside it in
//! `assets/OFL.txt`. The licence reserves the name `Nanum`, and a subset counts
//! as a modified version, so the face is renamed `Neko Hangul`; that is the name
//! it answers to in the fallback search, and the one a stack trace would show.
//!
//! Registering it before the `FontSystem` is built is what makes it work. The
//! fallback lists are computed from the faces present at construction, so a face
//! added to the database afterwards is never consulted for a missing glyph.

use std::sync::Arc;

use cosmic_text::{fontdb, FontSystem};

/// Hangul, as described above.
const HANGUL: &[u8] = include_bytes!("../assets/NekoHangul-Regular.ttf");

/// Creates a font system that knows the fonts this program brings.
///
/// Every `FontSystem::new()` in this crate should be one of these; the bare
/// constructor is for tests that only care about layout arithmetic and never
/// draw a Korean glyph.
pub fn font_system() -> FontSystem {
    FontSystem::new_with_fonts([fontdb::Source::Binary(Arc::new(HANGUL))])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::shaping_for;
    use cosmic_text::{Attrs, Buffer, Family, Metrics};

    /// The glyphs one layout produced: who drew each, and what it drew.
    fn shape(font_system: &mut FontSystem, text: &str) -> Vec<(String, u16)> {
        let mut buffer = Buffer::new_empty(Metrics::new(14.0, 18.0));
        buffer.set_size(None, None);
        buffer.set_text(
            text,
            &Attrs::new().family(Family::Name("Neko Hangul")),
            shaping_for(text),
            None,
        );
        buffer.shape_until_scroll(font_system, false);

        let mut glyphs = Vec::new();
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let family = font_system
                    .db()
                    .face(glyph.font_id)
                    .and_then(|face| face.families.first())
                    .map_or_else(String::new, |(name, _)| name.clone());
                glyphs.push((family, glyph.glyph_id));
            }
        }
        glyphs
    }

    /// A font system with nothing but the bundled face in it, so a glyph that is
    /// missing cannot be borrowed from the machine. Glyph id 0 is `.notdef`.
    fn bundled_only() -> FontSystem {
        let mut db = fontdb::Database::new();
        db.load_font_data(HANGUL.to_vec());
        FontSystem::new_with_locale_and_db("en-US".to_string(), db)
    }

    #[test]
    fn the_bundled_face_carries_hangul_and_ascii() {
        let mut font_system = bundled_only();
        // Four syllables, a capital, a digit -- what a Korean file name is made
        // of. The subset is easy to re-run with the wrong unicodes, and a face
        // that lost its Latin would draw `파일.rs` as boxes between the boxes.
        let glyphs = shape(&mut font_system, "한글파일 A1");
        assert_eq!(glyphs.len(), 7);
        for (family, glyph_id) in &glyphs {
            assert_eq!(family, "Neko Hangul");
            assert_ne!(*glyph_id, 0, "drew a box for a character in the subset");
        }
    }

    #[test]
    fn the_fallback_finds_the_bundled_face() {
        // The installed fonts are the ones cosmic-text would pick from, and none
        // of them has Hangul. So the glyphs can only come from the face the
        // program brought, which is the whole point of bringing it.
        let mut font_system = font_system();
        let mut buffer = Buffer::new_empty(Metrics::new(14.0, 18.0));
        buffer.set_size(None, None);
        buffer.set_text(
            "한글파일",
            &Attrs::new().family(Family::Monospace),
            shaping_for("한글파일"),
            None,
        );
        buffer.shape_until_scroll(&mut font_system, false);

        let mut families = Vec::new();
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                assert_ne!(glyph.glyph_id, 0, "drew a box");
                assert!(glyph.w > 0.0, "zero-width glyph");
                families.push(
                    font_system
                        .db()
                        .face(glyph.font_id)
                        .and_then(|face| face.families.first())
                        .map_or_else(String::new, |(name, _)| name.clone()),
                );
            }
        }
        assert_eq!(families.len(), 4);
        assert!(families.iter().all(|family| family == "Neko Hangul"));
    }
}
