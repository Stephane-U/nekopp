//! Convert between full-width and half-width characters.
//!
//! A port of the behaviour of the Notepad++ "Zenkaku/Hankaku" plugin: the two
//! commands switch text between the forms East Asian typography uses for the
//! same characters. Half-width ASCII is what source code, identifiers and file
//! names have to be; full-width ASCII is what a document written for a Chinese
//! or Japanese reader often carries, and the two are indistinguishable on screen
//! in some fonts -- which is exactly why a conversion command exists: a
//! full-width `＝` in a config file is a parse error that reads as a correct
//! line.
//!
//! Written here rather than translated; see `plugins/README.md`.

use nn_plugin::host;

/// The first and last code points of the full-width ASCII block, U+FF01 to
/// U+FF5E. They map onto U+0021 to U+007E -- every printable ASCII character
/// except the space -- by subtracting a constant, which is what makes the
/// conversion a range check rather than a table.
const FULLWIDTH_FIRST: u32 = 0xFF01;
const FULLWIDTH_LAST: u32 = 0xFF5E;
const FULLWIDTH_TO_ASCII: u32 = 0xFEE0;
/// U+3000 IDEOGRAPHIC SPACE, the full-width form of the space.
const IDEOGRAPHIC_SPACE: char = '\u{3000}';

/// Full-width characters to their ASCII forms.
///
/// Only the full-width ASCII block and the ideographic space are converted.
/// Half-width katakana are left alone: they are a different conversion with a
/// different answer, and folding them into this one would silently change
/// Japanese text a user only meant to make ASCII again.
pub fn to_half_width(text: &str) -> String {
    text.chars()
        .map(|ch| {
            let code = ch as u32;
            if (FULLWIDTH_FIRST..=FULLWIDTH_LAST).contains(&code) {
                char::from_u32(code - FULLWIDTH_TO_ASCII).unwrap_or(ch)
            } else if ch == IDEOGRAPHIC_SPACE {
                ' '
            } else {
                ch
            }
        })
        .collect()
}

/// Printable ASCII to its full-width forms, and the space to an ideographic one.
///
/// Control characters stay half-width: a tab or a newline has no full-width
/// form, and writing one where the other was meant would break the file rather
/// than decorate it.
pub fn to_full_width(text: &str) -> String {
    text.chars()
        .map(|ch| {
            let code = ch as u32;
            if (0x21..=0x7E).contains(&code) {
                char::from_u32(code + FULLWIDTH_TO_ASCII).unwrap_or(ch)
            } else if ch == ' ' {
                IDEOGRAPHIC_SPACE
            } else {
                ch
            }
        })
        .collect()
}

// ---- commands -------------------------------------------------------------

/// Runs `convert` over the selection, or over the whole document when nothing
/// is selected.
///
/// The selection is restored over the converted text rather than kept where it
/// was: the two forms are different byte lengths, so the old offsets would land
/// in the middle of a character the conversion had just rewritten.
fn apply(convert: fn(&str) -> String) {
    let selection = host::selection();
    if selection.is_empty() {
        let text = host::text();
        let converted = convert(&text);
        if !host::replace(&converted) {
            host::log("the document is read-only");
        }
        return;
    }
    let original = host::range(selection.clone());
    let converted = convert(&original);
    if !host::replace_range(selection.start, selection.end - selection.start, &converted) {
        host::log("the document is read-only");
        return;
    }
    host::select(selection.start, selection.start + converted.len());
}

fn half() {
    apply(to_half_width);
}

fn full() {
    apply(to_full_width);
}

nn_plugin::plugin! {
    name: "Half Width",
    commands: [
        ("To Half Width", half),
        ("To Full Width", full),
    ],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_width_ascii_becomes_ascii() {
        // "ＡＢＣ１２３" -- what a CJK input method produces for ABC123.
        assert_eq!(to_half_width("ＡＢＣ１２３"), "ABC123");
        assert_eq!(to_half_width("＝"), "=");
    }

    #[test]
    fn the_ideographic_space_becomes_a_space() {
        assert_eq!(to_half_width("a\u{3000}b"), "a b");
    }

    #[test]
    fn half_width_ascii_becomes_full_width() {
        assert_eq!(to_full_width("ABC123"), "ＡＢＣ１２３");
        assert_eq!(to_full_width("a b"), "\u{ff41}\u{3000}\u{ff42}");
    }

    #[test]
    fn control_characters_keep_their_width() {
        // No full-width form exists for a tab or a newline.
        assert_eq!(to_full_width("a\tb\nc"), "ａ\tｂ\nｃ");
        assert_eq!(to_half_width("ａ\tｂ"), "a\tb");
    }

    #[test]
    fn cjk_text_is_left_alone() {
        // Korean and Hebrew are not in the block being converted, which is the
        // property that makes the command safe on a mixed document. The space
        // between them is a space like any other, so the full-width direction
        // does widen it -- which is why the words are compared on their own.
        assert_eq!(to_half_width("한국어 שלום"), "한국어 שלום");
        assert_eq!(to_full_width("한국어"), "한국어");
        assert_eq!(to_full_width("שלום"), "שלום");
    }

    #[test]
    fn the_two_directions_are_inverses_on_the_block() {
        let text = "!\"#$%&'()*+,-./09:;<=>?@AZ[\\]^_`az{|}~";
        assert_eq!(to_half_width(&to_full_width(text)), text);
    }

    #[test]
    fn a_round_trip_leaves_an_ascii_space_full_width() {
        // The one asymmetry, stated rather than hidden: a space becomes an
        // ideographic space on the way out and comes back as an ordinary one,
        // so the pair of commands is a conversion and not an undo.
        assert_eq!(to_half_width(&to_full_width("a b")), "a b");
        assert_eq!(to_full_width("a b"), "\u{ff41}\u{3000}\u{ff42}");
    }
}
