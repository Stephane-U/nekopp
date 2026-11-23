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

//! Text encodings.
//!
//! Two families. The Unicode ones -- UTF-8 with and without a BOM, UTF-16 in
//! both byte orders -- are variable-width and carry their own byte-order mark.
//! The legacy ones are single-byte code pages, and there are forty-one of them
//! because that is how many Notepad++ lists under Encoding > Character sets.
//!
//! ISO 8859-1 is not a special case here: it is one of the forty-one, and it is
//! also the fallback `detect` reaches for, because every byte maps to a character
//! and so no file is ever unreadable.

use crate::codepage::CodePage;

/// How the bytes of a document map to characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Encoding {
    /// UTF-8, no byte-order mark.
    #[default]
    Utf8,
    /// UTF-8 preceded by `EF BB BF`.
    Utf8Bom,
    /// UTF-16 little-endian, preceded by `FF FE`.
    Utf16Le,
    /// UTF-16 big-endian, preceded by `FE FF`.
    Utf16Be,
    /// A single-byte code page.
    ///
    /// One variant rather than forty-one because they differ only in their
    /// table: `bom`, `detect` and the shape of `decode` are the same for all of
    /// them, and forty-one variants would mean forty-one arms in every match.
    CodePage(CodePage),
}

impl Encoding {
    /// The byte-order mark this encoding writes, if any.
    pub const fn bom(self) -> &'static [u8] {
        match self {
            Encoding::Utf8 => &[],
            Encoding::Utf8Bom => &[0xEF, 0xBB, 0xBF],
            Encoding::Utf16Le => &[0xFF, 0xFE],
            Encoding::Utf16Be => &[0xFE, 0xFF],
            Encoding::CodePage(_) => &[],
        }
    }

    /// Name as shown in the status bar.
    pub const fn name(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf8Bom => "UTF-8 BOM",
            Encoding::Utf16Le => "UTF-16 LE",
            Encoding::Utf16Be => "UTF-16 BE",
            Encoding::CodePage(page) => page.name(),
        }
    }

    /// Identifies the encoding of `bytes`.
    ///
    /// A BOM is authoritative. Without one we accept UTF-8 if it validates and
    /// fall back to Latin-1, which can represent any byte sequence without loss.
    pub fn detect(bytes: &[u8]) -> Self {
        if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            return Encoding::Utf8Bom;
        }
        if bytes.starts_with(&[0xFF, 0xFE]) {
            return Encoding::Utf16Le;
        }
        if bytes.starts_with(&[0xFE, 0xFF]) {
            return Encoding::Utf16Be;
        }
        if std::str::from_utf8(bytes).is_ok() {
            return Encoding::Utf8;
        }
        // ISO 8859-1 rather than a guess: every byte maps to a character, so the
        // file opens and the user can then pick the right set from the menu. A
        // guess that is wrong is worse than a fallback that is obviously wrong.
        Encoding::CodePage(CodePage::Iso8859_1)
    }

    /// Decodes `bytes` into a `String`.
    ///
    /// Invalid sequences become `U+FFFD` rather than failing: an editor must
    /// always be able to open a file.
    pub fn decode(self, bytes: &[u8]) -> String {
        let body = bytes.strip_prefix(self.bom()).unwrap_or(bytes);
        match self {
            Encoding::Utf8 | Encoding::Utf8Bom => String::from_utf8_lossy(body).into_owned(),
            Encoding::CodePage(page) => {
                let high = page.high();
                body.iter()
                    .map(|&b| {
                        if b < 0x80 {
                            b as char
                        } else {
                            high[(b - 0x80) as usize]
                        }
                    })
                    .collect()
            }
            Encoding::Utf16Le | Encoding::Utf16Be => {
                let units = body.chunks_exact(2).map(|pair| {
                    if self == Encoding::Utf16Le {
                        u16::from_le_bytes([pair[0], pair[1]])
                    } else {
                        u16::from_be_bytes([pair[0], pair[1]])
                    }
                });
                char::decode_utf16(units)
                    .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
                    .collect()
            }
        }
    }

    /// Encodes `text`, prefixing the BOM when this encoding has one.
    pub fn encode(self, text: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(text.len() + self.bom().len());
        out.extend_from_slice(self.bom());
        match self {
            Encoding::Utf8 | Encoding::Utf8Bom => out.extend_from_slice(text.as_bytes()),
            Encoding::CodePage(page) => {
                // A reverse table built once per save rather than a scan of the
                // forward one per character: 128 inserts against a linear search
                // of 128 for every character in the file.
                let high = page.high();
                let mut reverse = std::collections::HashMap::with_capacity(128);
                for (index, &c) in high.iter().enumerate() {
                    // A byte the mapping leaves undefined decodes to U+FFFD, and
                    // encoding that back would be a lie: it is not in the set.
                    if c != char::REPLACEMENT_CHARACTER {
                        reverse.entry(c).or_insert(0x80 + index as u8);
                    }
                }
                for c in text.chars() {
                    if (c as u32) < 0x80 {
                        out.push(c as u8);
                    } else {
                        out.push(reverse.get(&c).copied().unwrap_or(b'?'));
                    }
                }
            }
            Encoding::Utf16Le | Encoding::Utf16Be => {
                for unit in text.encode_utf16() {
                    out.extend_from_slice(&if self == Encoding::Utf16Le {
                        unit.to_le_bytes()
                    } else {
                        unit.to_be_bytes()
                    });
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_boms() {
        assert_eq!(Encoding::detect(b"\xEF\xBB\xBFhi"), Encoding::Utf8Bom);
        assert_eq!(Encoding::detect(b"\xFF\xFEh\x00"), Encoding::Utf16Le);
        assert_eq!(Encoding::detect(b"\xFE\xFF\x00h"), Encoding::Utf16Be);
    }

    #[test]
    fn detects_plain_utf8() {
        assert_eq!(Encoding::detect("héllo".as_bytes()), Encoding::Utf8);
    }

    #[test]
    fn falls_back_to_latin1() {
        assert_eq!(
            Encoding::detect(&[0xE9, 0xFF]),
            Encoding::CodePage(CodePage::Iso8859_1)
        );
    }

    #[test]
    fn utf8_roundtrip() {
        let text = "héllo\nwörld";
        assert_eq!(Encoding::Utf8.decode(&Encoding::Utf8.encode(text)), text);
    }

    #[test]
    fn utf8_bom_roundtrip() {
        let text = "héllo";
        let bytes = Encoding::Utf8Bom.encode(text);
        assert_eq!(&bytes[..3], &[0xEF, 0xBB, 0xBF]);
        assert_eq!(Encoding::Utf8Bom.decode(&bytes), text);
    }

    #[test]
    fn utf16_roundtrip_both_orders() {
        let text = "héllo\nwörld 🎉";
        for enc in [Encoding::Utf16Le, Encoding::Utf16Be] {
            assert_eq!(enc.decode(&enc.encode(text)), text);
        }
    }

    #[test]
    fn latin1_roundtrip() {
        let latin1 = Encoding::CodePage(CodePage::Iso8859_1);
        let text = "café";
        let bytes = latin1.encode(text);
        assert_eq!(bytes, vec![b'c', b'a', b'f', 0xE9]);
        assert_eq!(latin1.decode(&bytes), text);
    }

    #[test]
    fn latin1_replaces_unrepresentable() {
        assert_eq!(
            Encoding::CodePage(CodePage::Iso8859_1).encode("a→b"),
            b"a?b".to_vec()
        );
    }

    #[test]
    fn latin1_preserves_arbitrary_bytes() {
        let latin1 = Encoding::CodePage(CodePage::Iso8859_1);
        let bytes: Vec<u8> = (0u8..=255).collect();
        assert_eq!(latin1.encode(&latin1.decode(&bytes)), bytes);
    }

    #[test]
    fn a_code_page_reads_its_high_half() {
        // 0xE9 is `é` in Windows-1252 and `И` in KOI8-R, which is the whole
        // reason the menu has to exist.
        let west = Encoding::CodePage(CodePage::Windows1252);
        assert_eq!(west.decode(&[0xE9]), "é");
        let cyrillic = Encoding::CodePage(CodePage::Koi8R);
        assert_eq!(cyrillic.decode(&[0xE9]), "И");
    }

    #[test]
    fn a_code_page_leaves_ascii_alone() {
        for page in CodePage::ALL {
            let encoding = Encoding::CodePage(page);
            assert_eq!(encoding.decode(b"hello"), "hello", "{}", page.name());
            assert_eq!(encoding.encode("hello"), b"hello".to_vec(), "{}", page.name());
        }
    }

    #[test]
    fn every_code_page_round_trips_its_own_table() {
        for page in CodePage::ALL {
            let encoding = Encoding::CodePage(page);
            let bytes: Vec<u8> = (0x80u8..=0xFF).collect();
            let text = encoding.decode(&bytes);
            // A byte the mapping leaves undefined decodes to U+FFFD and encodes
            // back to `?`, so the round trip is only exact where the table is.
            let expected: Vec<u8> = page
                .high()
                .iter()
                .map(|&c| {
                    if c == char::REPLACEMENT_CHARACTER {
                        b'?'
                    } else {
                        encoding.encode(&c.to_string())[0]
                    }
                })
                .collect();
            assert_eq!(encoding.encode(&text), expected, "{}", page.name());
        }
    }

    #[test]
    fn every_code_page_has_a_name_and_a_key() {
        for page in CodePage::ALL {
            assert!(!page.name().is_empty());
            assert!(!page.key().is_empty());
            assert!(page.key().is_ascii());
        }
    }
}
