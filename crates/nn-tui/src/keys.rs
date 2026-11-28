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

//! What the terminal sends, turned into something the editor can act on.
//!
//! A raw-mode terminal sends a keystroke as bytes: a letter as itself, a control
//! chord as one byte below 0x20, and everything else -- the arrows, Home, Delete
//! -- as a sequence beginning with `ESC [`. This module is the whole of that
//! translation, as a function from bytes to a key and a length, so it can be
//! tested without a terminal.
//!
//! Only the keys this shell uses are here. A key it does not know is reported as
//! unknown and its bytes are skipped, which is how a terminal that sends a
//! sequence for a key nobody has bound costs nothing.

/// A key, with the modifiers that this shell acts on folded in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// A printable character, as typed.
    Char(char),
    Enter,
    Tab,
    Backspace,
    Delete,
    Escape,
    Up,
    Down,
    Left,
    Right,
    /// `Home`: the first column of the line.
    LineStart,
    /// `End`: just past the last character of the line.
    LineEnd,
    /// `Ctrl+Home`: the start of the document.
    DocStart,
    /// `Ctrl+End`: the end of the document.
    DocEnd,
    PageUp,
    PageDown,
    /// `Ctrl+PageUp` and `Ctrl+PageDown`: the previous and the next document.
    ///
    /// Sent as `ESC [ 5 ; 5 ~` and `ESC [ 6 ; 5 ~`, which is what a terminal
    /// that understands the modifier does with them. `Ctrl+Tab` would be the
    /// chord a window user reaches for and it cannot arrive at all: a terminal
    /// sends the same `0x09` for it as for `Tab`, which in an editor is the
    /// indent. The window binds these two as well, so the hands that know them
    /// need not learn anything new.
    CtrlPageUp,
    CtrlPageDown,
    /// `Ctrl+Left` and `Ctrl+Right`: by the word.
    WordLeft,
    WordRight,
    /// An arrow with `Shift` held: the same motion, selecting.
    ExtendUp,
    ExtendDown,
    ExtendLeft,
    ExtendRight,
    /// `Ctrl` and a letter, lowercased.
    Ctrl(char),
    /// `F1` through `F12`. `F9` opens the menus, as it does in Midnight
    /// Commander, and `F10` does the same for hands trained on Notepad++.
    Function(u8),
    /// A function key with `Shift` held, which a terminal that understands the
    /// modifier sends as `ESC [ 12 ; 2 ~`. `Shift+F2` is the previous bookmark.
    ShiftFunction(u8),
    /// A function key with `Ctrl` held, `ESC [ 12 ; 5 ~`: `Ctrl+F2` puts a
    /// bookmark on the caret's line.
    CtrlFunction(u8),
}

/// Reads one key from the front of `bytes`, and how many bytes it took.
///
/// `None` when the bytes there are not a key this shell knows: the caller drops
/// one byte and tries again, so a sequence from a keyboard nobody designed for
/// this editor cannot wedge the loop.
pub fn parse(bytes: &[u8]) -> Option<(Key, usize)> {
    let first = *bytes.first()?;
    match first {
        // The escape that is a key of its own rather than the start of one.
        0x1b if bytes.len() == 1 => Some((Key::Escape, 1)),
        0x1b => sequence(bytes),
        b'\r' | b'\n' => Some((Key::Enter, 1)),
        b'\t' => Some((Key::Tab, 1)),
        // A keyboard sends `Ctrl+Space` as NUL, which is why the range below
        // starts at one: it is `Ctrl+@` to a terminal and the completion chord to
        // every editor that has one.
        0x00 => Some((Key::Ctrl(' '), 1)),
        0x7f | 0x08 => Some((Key::Backspace, 1)),
        0x01..=0x1a => {
            let letter = (b'a' + first - 1) as char;
            Some((Key::Ctrl(letter), 1))
        }
        // Everything else that is not a control byte is text, in UTF-8.
        _ => {
            let text = std::str::from_utf8(bytes.get(..4.min(bytes.len()))?).ok()?;
            let ch = text.chars().next()?;
            if ch.is_control() {
                return None;
            }
            Some((Key::Char(ch), ch.len_utf8()))
        }
    }
}

/// A sequence that starts with `ESC`, which in practice is always `ESC [`.
fn sequence(bytes: &[u8]) -> Option<(Key, usize)> {
    // `ESC O H` and friends: the keys that move the cursor in "application
    // cursor" mode, which some terminals use for Home and End.
    if bytes.get(1) == Some(&b'O') {
        let key = match bytes.get(2)? {
            b'H' => Key::LineStart,
            b'F' => Key::LineEnd,
            // `ESC O P` through `ESC O S` are the first four function keys on
            // the terminals that send this form.
            b'P' => Key::Function(1),
            b'Q' => Key::Function(2),
            b'R' => Key::Function(3),
            b'S' => Key::Function(4),
            _ => return None,
        };
        return Some((key, 3));
    }
    if bytes.get(1) != Some(&b'[') {
        return None;
    }
    // A CSI sequence is parameters (digits and semicolons) up to one final byte
    // in the range 0x40..=0x7e.
    let mut at = 2;
    while let Some(&byte) = bytes.get(at) {
        if (0x40..=0x7e).contains(&byte) {
            break;
        }
        at += 1;
    }
    let final_byte = *bytes.get(at)?;
    let params = std::str::from_utf8(bytes.get(2..at)?).ok()?;
    // The function keys first, because they have two spellings and three
    // modifiers apiece and a table row per combination would be a table of
    // thirty-six rows saying the same thing.
    if let Some(key) = function_key(final_byte, params) {
        return Some((key, at + 1));
    }
    let key = match (final_byte, params) {
        (b'A', "") => Key::Up,
        (b'B', "") => Key::Down,
        (b'C', "") => Key::Right,
        (b'D', "") => Key::Left,
        (b'A', "1;2") => Key::ExtendUp,
        (b'B', "1;2") => Key::ExtendDown,
        (b'C', "1;2") => Key::ExtendRight,
        (b'D', "1;2") => Key::ExtendLeft,
        (b'C', "1;5") => Key::WordRight,
        (b'D', "1;5") => Key::WordLeft,
        (b'H', "") => Key::LineStart,
        (b'F', "") => Key::LineEnd,
        (b'H', "1;5") => Key::DocStart,
        (b'F', "1;5") => Key::DocEnd,
        (b'~', "3") => Key::Delete,
        (b'~', "5") => Key::PageUp,
        (b'~', "6") => Key::PageDown,
        (b'~', "5;5") => Key::CtrlPageUp,
        (b'~', "6;5") => Key::CtrlPageDown,
        (b'~', "1") | (b'~', "7") => Key::LineStart,
        (b'~', "4") | (b'~', "8") => Key::LineEnd,
        _ => return None,
    };
    Some((key, at + 1))
}

/// A function key from its xterm parameters: which one, and which modifier.
///
/// Two spellings reach a terminal. `ESC [ 12 ~` is F2, counting 11 for F1 and
/// skipping 16; `ESC [ 1 ; 2 P` is F1 with Shift, the first four function keys
/// having a letter of their own. A modifier is a second parameter: 2 for Shift,
/// 5 for Ctrl. The two that a terminal cannot send at all are worth naming --
/// `Ctrl+Shift` and `Alt` -- so that their absence here reads as a fact rather
/// than as an oversight.
fn function_key(final_byte: u8, params: &str) -> Option<Key> {
    let (base, modifier) = match params.split_once(';') {
        Some((base, modifier)) => (base, modifier.parse::<u8>().ok()?),
        None => (params, 1),
    };
    let number = match final_byte {
        b'~' => match base {
            "11" => 1,
            "12" => 2,
            "13" => 3,
            "14" => 4,
            "15" => 5,
            "17" => 6,
            "18" => 7,
            "19" => 8,
            "20" => 9,
            "21" => 10,
            "23" => 11,
            "24" => 12,
            _ => return None,
        },
        b'P' | b'Q' | b'R' | b'S' if base == "1" => match final_byte {
            b'P' => 1,
            b'Q' => 2,
            b'R' => 3,
            _ => 4,
        },
        _ => return None,
    };
    Some(match modifier {
        1 => Key::Function(number),
        2 => Key::ShiftFunction(number),
        5 => Key::CtrlFunction(number),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(bytes: &[u8]) -> (Key, usize) {
        parse(bytes).expect("a key")
    }

    #[test]
    fn text_and_the_control_bytes_are_read_as_themselves() {
        assert_eq!(one(b"a"), (Key::Char('a'), 1));
        // Any UTF-8 character, at the number of bytes it takes.
        assert_eq!(one("é".as_bytes()), (Key::Char('é'), 2));
        assert_eq!(one("中".as_bytes()), (Key::Char('中'), 3));
        assert_eq!(one(b"\r"), (Key::Enter, 1));
        assert_eq!(one(b"\t"), (Key::Tab, 1));
        assert_eq!(one(b"\x7f"), (Key::Backspace, 1));
        // Ctrl+S is 0x13, which is where the control bytes come from.
        assert_eq!(one(b"\x13"), (Key::Ctrl('s'), 1));
        assert_eq!(one(b"\x11"), (Key::Ctrl('q'), 1));
    }

    #[test]
    fn the_arrows_and_the_editing_keys_read_as_themselves() {
        assert_eq!(one(b"\x1b[A"), (Key::Up, 3));
        assert_eq!(one(b"\x1b[B"), (Key::Down, 3));
        assert_eq!(one(b"\x1b[C"), (Key::Right, 3));
        assert_eq!(one(b"\x1b[D"), (Key::Left, 3));
        assert_eq!(one(b"\x1b[3~"), (Key::Delete, 4));
        assert_eq!(one(b"\x1b[5~"), (Key::PageUp, 4));
        assert_eq!(one(b"\x1b[6~"), (Key::PageDown, 4));
        assert_eq!(one(b"\x1b[H"), (Key::LineStart, 3));
        assert_eq!(one(b"\x1b[F"), (Key::LineEnd, 3));
        assert_eq!(one(b"\x1bOH"), (Key::LineStart, 3));
    }

    #[test]
    fn a_function_key_carries_the_modifier_that_was_held() {
        assert_eq!(one(b"\x1b[12~"), (Key::Function(2), 5));
        // Shift+F2 and Ctrl+F2 are the two bookmark chords the window binds,
        // and a terminal that understands the modifier sends them like this.
        assert_eq!(one(b"\x1b[12;2~"), (Key::ShiftFunction(2), 7));
        assert_eq!(one(b"\x1b[12;5~"), (Key::CtrlFunction(2), 7));
        // The first four have a letter of their own, and the modifier comes
        // before it rather than after.
        assert_eq!(one(b"\x1b[1;2P"), (Key::ShiftFunction(1), 6));
        assert_eq!(one(b"\x1b[1;5Q"), (Key::CtrlFunction(2), 6));
        // A modifier this shell does not act on is a key it does not know,
        // rather than a key it guesses at.
        assert!(parse(b"\x1b[12;3~").is_none());
    }

    #[test]
    fn a_held_modifier_is_folded_into_the_key() {
        // Shift and the arrows select; Ctrl and the arrows go by the word.
        assert_eq!(one(b"\x1b[1;2A"), (Key::ExtendUp, 6));
        assert_eq!(one(b"\x1b[1;2B"), (Key::ExtendDown, 6));
        assert_eq!(one(b"\x1b[1;2C"), (Key::ExtendRight, 6));
        assert_eq!(one(b"\x1b[1;2D"), (Key::ExtendLeft, 6));
        assert_eq!(one(b"\x1b[1;5C"), (Key::WordRight, 6));
        assert_eq!(one(b"\x1b[1;5D"), (Key::WordLeft, 6));
        assert_eq!(one(b"\x1b[1;5H"), (Key::DocStart, 6));
        assert_eq!(one(b"\x1b[1;5F"), (Key::DocEnd, 6));
        // The two the window uses to move between documents.
        assert_eq!(one(b"\x1b[5;5~"), (Key::CtrlPageUp, 6));
        assert_eq!(one(b"\x1b[6;5~"), (Key::CtrlPageDown, 6));
    }

    #[test]
    fn a_sequence_nobody_bound_is_refused_rather_than_guessed_at() {
        assert_eq!(parse(b"\x1b[Z"), None);
        assert_eq!(parse(b"\x1b[99~"), None);
        assert_eq!(parse(b"\x1bQ"), None);
        // An escape on its own is the Escape key, which is what closes a dialog.
        assert_eq!(one(b"\x1b"), (Key::Escape, 1));
        // A truncated sequence is not a key either: the caller skips its first
        // byte rather than waiting for bytes that are not coming.
        assert_eq!(parse(b"\x1b["), None);
    }
}
