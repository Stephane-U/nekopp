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

//! The keymap: which chord runs which command.
//!
//! Every command the menu can invoke is reachable from the keyboard, and the
//! chord for each lives in one table rather than in a `match` arm. The menu
//! reads its shortcut column from the same table, so a rebinding cannot leave
//! the menu lying about what a key does.
//!
//! Motions, typing, and the modal widgets are deliberately absent. They are not
//! commands with a name, and a user who rebinds `Ctrl+D` should not lose
//! `Ctrl+Left`.
//!
//! The table is a `Vec` scanned linearly. Forty-odd comparisons per keypress is
//! nothing next to shaping a line, and a `HashMap` would need `Action` to be
//! hashable for no measurable gain.

use crate::action::Action;

/// A key, without the modifiers held with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyName {
    /// A printable character, lowercased.
    Char(char),
    /// A function key, `F1` through `F24`.
    Function(u8),
    Space,
    Tab,
    Enter,
    Escape,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
}

impl KeyName {
    /// The name this key is written as in the settings file.
    pub fn render(self) -> String {
        match self {
            KeyName::Char(c) => c.to_ascii_uppercase().to_string(),
            KeyName::Function(n) => format!("F{n}"),
            KeyName::Space => "Space".into(),
            KeyName::Tab => "Tab".into(),
            KeyName::Enter => "Enter".into(),
            KeyName::Escape => "Escape".into(),
            KeyName::Backspace => "Backspace".into(),
            KeyName::Delete => "Delete".into(),
            KeyName::Up => "Up".into(),
            KeyName::Down => "Down".into(),
            KeyName::Left => "Left".into(),
            KeyName::Right => "Right".into(),
            KeyName::Home => "Home".into(),
            KeyName::End => "End".into(),
            KeyName::PageUp => "PageUp".into(),
            KeyName::PageDown => "PageDown".into(),
        }
    }

    /// Reads a Windows virtual-key code.
    ///
    /// Notepad++ writes its shortcuts as virtual-key codes, so importing them
    /// means reading the same numbers. The punctuation keys are named by the
    /// character they carry on a US layout; `VK_OEM_PLUS` and `VK_OEM_MINUS`
    /// are the exception, mapped to `+` and `-` rather than `=` and `-`, because
    /// that is the character this editor's own `Ctrl++` and `Ctrl+-` bindings
    /// hold and shift is dropped for punctuation anyway.
    pub fn from_vk(code: u32) -> Option<KeyName> {
        let named = match code {
            0x08 => KeyName::Backspace,
            0x09 => KeyName::Tab,
            0x0d => KeyName::Enter,
            0x1b => KeyName::Escape,
            0x20 => KeyName::Space,
            0x21 => KeyName::PageUp,
            0x22 => KeyName::PageDown,
            0x23 => KeyName::End,
            0x24 => KeyName::Home,
            0x25 => KeyName::Left,
            0x26 => KeyName::Up,
            0x27 => KeyName::Right,
            0x28 => KeyName::Down,
            0x2e => KeyName::Delete,
            0x30..=0x39 => KeyName::Char((b'0' + (code - 0x30) as u8) as char),
            0x41..=0x5a => KeyName::Char((b'a' + (code - 0x41) as u8) as char),
            0x70..=0x87 => KeyName::Function((code - 0x70 + 1) as u8),
            0xba => KeyName::Char(';'),
            0xbb => KeyName::Char('+'),
            0xbc => KeyName::Char(','),
            0xbd => KeyName::Char('-'),
            0xbe => KeyName::Char('.'),
            0xbf => KeyName::Char('/'),
            0xc0 => KeyName::Char('`'),
            0xdb => KeyName::Char('['),
            0xdc => KeyName::Char('\\'),
            0xdd => KeyName::Char(']'),
            0xde => KeyName::Char('\''),
            _ => return None,
        };
        Some(named)
    }

    /// The Windows virtual-key code for this key, when it has one.
    ///
    /// The inverse of [`KeyName::from_vk`], and the reason exporting a
    /// `shortcuts.xml` is possible at all: Notepad++ names keys by number, so a
    /// chord that cannot be written as a number cannot be exported. `None` is
    /// the honest answer for a key with no code -- `=` shares `VK_OEM_PLUS`
    /// with `+` on a US layout, and writing `0xbb` for it would export a
    /// different key than the one bound.
    pub fn to_vk(self) -> Option<u32> {
        let code = match self {
            KeyName::Backspace => 0x08,
            KeyName::Tab => 0x09,
            KeyName::Enter => 0x0d,
            KeyName::Escape => 0x1b,
            KeyName::Space => 0x20,
            KeyName::PageUp => 0x21,
            KeyName::PageDown => 0x22,
            KeyName::End => 0x23,
            KeyName::Home => 0x24,
            KeyName::Left => 0x25,
            KeyName::Up => 0x26,
            KeyName::Right => 0x27,
            KeyName::Down => 0x28,
            KeyName::Delete => 0x2e,
            KeyName::Char(c) => match c {
                '0'..='9' => 0x30 + (c as u32 - '0' as u32),
                'a'..='z' => 0x41 + (c as u32 - 'a' as u32),
                ';' => 0xba,
                '+' => 0xbb,
                ',' => 0xbc,
                '-' => 0xbd,
                '.' => 0xbe,
                '/' => 0xbf,
                '`' => 0xc0,
                '[' => 0xdb,
                '\\' => 0xdc,
                ']' => 0xdd,
                '\'' => 0xde,
                _ => return None,
            },
            KeyName::Function(n) if (1..=24).contains(&n) => 0x70 + (n as u32 - 1),
            KeyName::Function(_) => return None,
        };
        Some(code)
    }

    /// Reads a key name, case-insensitively.
    ///
    /// A single character is that character; anything longer is a name. That
    /// keeps `Ctrl+,` and `Ctrl+Space` in the same syntax without a quoting
    /// rule.
    pub fn parse(text: &str) -> Option<KeyName> {
        let mut chars = text.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            return Some(KeyName::Char(c.to_ascii_lowercase()));
        }
        let lower = text.to_ascii_lowercase();
        let named = match lower.as_str() {
            "space" => KeyName::Space,
            "tab" => KeyName::Tab,
            "enter" | "return" => KeyName::Enter,
            "escape" | "esc" => KeyName::Escape,
            "backspace" => KeyName::Backspace,
            "delete" | "del" => KeyName::Delete,
            "up" => KeyName::Up,
            "down" => KeyName::Down,
            "left" => KeyName::Left,
            "right" => KeyName::Right,
            "home" => KeyName::Home,
            "end" => KeyName::End,
            "pageup" | "pgup" => KeyName::PageUp,
            "pagedown" | "pgdn" => KeyName::PageDown,
            _ => {
                let digits = lower.strip_prefix('f')?;
                let number: u8 = digits.parse().ok()?;
                if !(1..=24).contains(&number) {
                    return None;
                }
                KeyName::Function(number)
            }
        };
        Some(named)
    }
}

/// A key together with the modifiers held with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: KeyName,
}

/// Folds a control character back to the key that produced it.
///
/// A platform that reports the *typed text* rather than the key gives `Ctrl+J`
/// as `U+000A`, not `j`. Windows does this through `ToUnicode`, and X11 does it
/// whenever the layout maps the control level. Left alone, `Ctrl+Shift+J` would
/// bind as a chord no key can ever produce again, so the caret notation is
/// undone here: `0x01..=0x1a` are `a..=z`, and the six that follow are the
/// punctuation keys that share the row.
fn uncontrol(c: char) -> char {
    match c {
        '\u{0}' => '@',
        '\u{1}'..='\u{1a}' => (b'a' + (c as u8 - 1)) as char,
        '\u{1b}' => '[',
        '\u{1c}' => '\\',
        '\u{1d}' => ']',
        '\u{1e}' => '^',
        '\u{1f}' => '_',
        '\u{7f}' => '?',
        _ => c,
    }
}

impl Chord {
    /// Builds a chord from a printable character.
    ///
    /// Shift is dropped for everything but letters. `Ctrl++` arrives as
    /// `Ctrl+Shift++` on any layout where `+` needs shift, and a binding that
    /// only fired on some layouts would be a trap. Letters keep it, because
    /// `Ctrl+U` and `Ctrl+Shift+U` are different commands.
    ///
    /// Digits are in the dropped group for the same reason as punctuation, and
    /// it took a bug report to notice. On AZERTY -- and on every layout where
    /// the digits sit on the shifted level -- `Ctrl+0` can only be pressed as
    /// `Ctrl+Shift+0`, because that is the only way to produce a `0`. Keeping
    /// shift for digits made `Restore Default Zoom` unreachable on those
    /// layouts: the chord that arrived was `Ctrl+Shift+0` and the binding was
    /// `Ctrl+0`. Nothing in the default keymap distinguishes `Ctrl+1` from
    /// `Ctrl+Shift+1`, so folding the two together costs no binding and buys
    /// back the shortcut.
    ///
    /// A control character is folded back to its key first, so a platform that
    /// reports typed text instead of the key still binds what was pressed.
    pub fn from_char(c: char, ctrl: bool, shift: bool, alt: bool) -> Chord {
        let c = uncontrol(c).to_ascii_lowercase();
        Chord {
            ctrl,
            shift: shift && c.is_ascii_alphabetic(),
            alt,
            key: KeyName::Char(c),
        }
    }

    /// Reads a chord such as `Ctrl+Shift+S`.
    ///
    /// Modifiers are stripped from the front, so `Ctrl++` parses: the trailing
    /// `+` is what is left once `Ctrl+` is gone.
    pub fn parse(text: &str) -> Option<Chord> {
        let mut rest = text.trim();
        let (mut ctrl, mut shift, mut alt) = (false, false, false);
        loop {
            if let Some(tail) = strip_prefix_ci(rest, "ctrl+") {
                ctrl = true;
                rest = tail;
            } else if let Some(tail) = strip_prefix_ci(rest, "shift+") {
                shift = true;
                rest = tail;
            } else if let Some(tail) = strip_prefix_ci(rest, "alt+") {
                alt = true;
                rest = tail;
            } else if strip_prefix_ci(rest, "super+").is_some()
                || strip_prefix_ci(rest, "meta+").is_some()
            {
                // A modifier this app does not use. Rejecting beats binding
                // something the user did not ask for.
                return None;
            } else {
                break;
            }
        }
        Some(Chord {
            ctrl,
            shift,
            alt,
            key: KeyName::parse(rest)?,
        })
    }

    /// The chord as it is written in the settings file and the menu.
    ///
    /// Modifiers come out in the order every desktop writes them, so a chord
    /// typed in any order renders the same way.
    pub fn render(self) -> String {
        let mut out = String::new();
        if self.ctrl {
            out.push_str("Ctrl+");
        }
        if self.alt {
            out.push_str("Alt+");
        }
        if self.shift {
            out.push_str("Shift+");
        }
        out.push_str(&self.key.render());
        out
    }
}

/// Strips `prefix` from `text`, ignoring ASCII case.
fn strip_prefix_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    if text.len() >= prefix.len()
        && text.is_char_boundary(prefix.len())
        && text[..prefix.len()].eq_ignore_ascii_case(prefix)
    {
        Some(&text[prefix.len()..])
    } else {
        None
    }
}

/// The bindings every install starts with.
///
/// One chord per command. A command with two chords would have to survive a
/// round trip through a file that names each command once, and the first
/// hand-edit would silently drop one of them.
const DEFAULTS: &[(Action, &str)] = &[
    (Action::New, "Ctrl+N"),
    (Action::Open, "Ctrl+O"),
    (Action::Save, "Ctrl+S"),
    (Action::SaveAs, "Ctrl+Shift+S"),
    (Action::CloseTab, "Ctrl+W"),
    (Action::Exit, "Ctrl+Shift+Q"),
    (Action::Undo, "Ctrl+Z"),
    (Action::Redo, "Ctrl+Y"),
    (Action::Cut, "Ctrl+X"),
    (Action::Copy, "Ctrl+C"),
    (Action::Paste, "Ctrl+V"),
    (Action::SelectAll, "Ctrl+A"),
    (Action::CompleteWord, "Ctrl+Space"),
    (Action::CompleteFunction, "Ctrl+Shift+Space"),
    (Action::ParamHint, "Ctrl+Shift+Alt+Space"),
    (Action::CompletePath, "Ctrl+Alt+Space"),
    (Action::Find, "Ctrl+F"),
    (Action::Replace, "Ctrl+H"),
    (Action::FindNext, "F3"),
    (Action::FindPrev, "Shift+F3"),
    (Action::FindInFiles, "Ctrl+Shift+F"),
    (Action::GotoLine, "Ctrl+G"),
    (Action::ToggleBookmark, "Ctrl+F2"),
    (Action::NextBookmark, "F2"),
    (Action::PrevBookmark, "Shift+F2"),
    (Action::FoldToggle, "Ctrl+Alt+F"),
    (Action::FoldAll, "Ctrl+Alt+A"),
    (Action::UnfoldAll, "Ctrl+Alt+Shift+F"),
    (Action::ZoomIn, "Ctrl++"),
    (Action::ZoomOut, "Ctrl+-"),
    (Action::ZoomReset, "Ctrl+0"),
    (Action::ToggleMinimap, "Ctrl+Alt+M"),
    (Action::ToggleFunctionList, "Ctrl+Alt+L"),
    (Action::ToggleWrap, "Ctrl+Alt+W"),
    (Action::ToggleTheme, "Ctrl+Alt+T"),
    (Action::Preferences, "Ctrl+,"),
    (Action::Run, "F5"),
    (Action::ToggleRecording, "Ctrl+Shift+R"),
    (Action::PlayMacro, "Ctrl+Shift+P"),
    (Action::UpperCase, "Ctrl+Shift+U"),
    (Action::LowerCase, "Ctrl+U"),
    (Action::ToggleComment, "Ctrl+Q"),
    (Action::DuplicateLine, "Ctrl+D"),
    (Action::MoveLineUp, "Ctrl+Shift+Up"),
    (Action::MoveLineDown, "Ctrl+Shift+Down"),
    // Notepad++ puts the peeker on Ctrl+Tab and leaves the plain cycling to
    // Ctrl+PageDown/PageUp, which is what these follow.
    (Action::DocumentPeeker, "Ctrl+Tab"),
    (Action::NextTab, "Ctrl+PageDown"),
    (Action::PrevTab, "Ctrl+PageUp"),
    (Action::SaveAll, "Ctrl+Alt+S"),
    (Action::Print, "Ctrl+P"),
    (Action::PrintNow, "Ctrl+Alt+Shift+P"),
    (Action::CloseAll, "Ctrl+Shift+W"),
    (Action::Reload, "Ctrl+Alt+R"),
    (Action::DeleteLine, "Ctrl+L"),
    (Action::SelectLine, "Ctrl+Shift+L"),
    (Action::Indent, "Ctrl+]"),
    (Action::Unindent, "Ctrl+["),
    (Action::InsertDateTime, "Ctrl+Alt+D"),
    // Notepad++ reaches multi-caret through Ctrl+click and the mouse only. The
    // keyboard pair is the one every other editor settled on, and it is the
    // only way to grow a caret stack without a pointer.
    (Action::AddCaretAbove, "Ctrl+Alt+Up"),
    (Action::AddCaretBelow, "Ctrl+Alt+Down"),
    (Action::FullScreen, "F11"),
    // Notepad++ ships distraction-free mode unbound. F11 is taken by full
    // screen, so the pair reads as "bigger" and "quieter" on adjacent keys.
    (Action::DistractionFree, "F12"),
    (Action::TabLayout, "Ctrl+Alt+B"),
    // Notepad++ leaves the change history unbound as well.
    (Action::ChangeHistory, "Ctrl+Alt+H"),
    // Notepad++ leaves the character panel unbound too. C is for character and
    // is free: Ctrl+Alt+C is not used by anything else here.
    (Action::CharPanel, "Ctrl+Alt+C"),
    // K for kitten. Ctrl+Alt+K is free, and the mode is meant to be reached
    // without thinking about it.
    (Action::Playground, "Ctrl+Alt+K"),
    // Notepad++'s own chord for incremental search.
    (Action::IncrementalSearch, "Ctrl+Alt+I"),
    (Action::MarkAll, "Ctrl+Shift+M"),
    (Action::GotoBrace, "Ctrl+B"),
    // Notepad++ leaves the split commands unbound. These follow the convention
    // its own "Move to Other View" users adopt, and stay out of the way of the
    // Ctrl+Alt+Shift+arrow block-selection chords.
    (Action::SplitVertical, "Ctrl+Alt+Shift+V"),
    (Action::SplitHorizontal, "Ctrl+Alt+Shift+H"),
    (Action::Unsplit, "Ctrl+Alt+Shift+U"),
    (Action::MoveToOtherView, "Ctrl+Alt+Shift+Right"),
    (Action::CloneToOtherView, "Ctrl+Alt+Shift+Left"),
];

/// Which chord runs which command.
#[derive(Debug, Clone, PartialEq)]
pub struct Keymap {
    bindings: Vec<(Chord, Action)>,
}

impl Default for Keymap {
    fn default() -> Self {
        let mut keymap = Keymap {
            bindings: Vec::with_capacity(DEFAULTS.len()),
        };
        for (action, text) in DEFAULTS {
            // A malformed default is a bug, not a runtime condition: the test
            // below fails on one, and skipping it here keeps a typo from
            // taking the window down at startup.
            if let Some(chord) = Chord::parse(text) {
                keymap.bind(*action, chord);
            }
        }
        keymap
    }
}

impl Keymap {
    /// Binds `chord` to `action`, taking it from whatever held it before.
    ///
    /// A chord means one thing. Private because a command holds one chord:
    /// `set` is the way in, and it clears the old one first.
    fn bind(&mut self, action: Action, chord: Chord) {
        if self.bindings.iter().any(|(c, a)| *c == chord && *a == action) {
            return;
        }
        self.bindings.retain(|(c, _)| *c != chord);
        self.bindings.push((chord, action));
    }

    /// Removes every chord bound to `action`.
    pub fn unbind(&mut self, action: Action) {
        self.bindings.retain(|(_, a)| *a != action);
    }

    /// Points `action` at `chord` and nothing else.
    pub fn set(&mut self, action: Action, chord: Chord) {
        self.unbind(action);
        self.bind(action, chord);
    }

    /// The command `chord` runs, if any.
    pub fn lookup(&self, chord: &Chord) -> Option<Action> {
        self.bindings
            .iter()
            .find(|(c, _)| c == chord)
            .map(|(_, action)| *action)
    }

    /// The chord shown beside `action` in the menu, empty when it has none.
    pub fn display(&self, action: Action) -> String {
        self.bindings
            .iter()
            .find(|(_, a)| *a == action)
            .map(|(chord, _)| chord.render())
            .unwrap_or_default()
    }

    /// Every binding, in the order they were made.
    pub fn bindings(&self) -> &[(Chord, Action)] {
        &self.bindings
    }

    /// Applies one `key <action> <chord>` line.
    ///
    /// Returns false when the line names something this build does not know, so
    /// a file written by a newer version still loads the rest.
    ///
    /// `resolve` turns the name into a command. It is the caller's because not
    /// every name in a settings file is a command: `theme_dracula` names a
    /// setting, and the list of themes belongs to the front-end that draws them.
    /// Passing the resolver keeps that list out of this crate without keeping the
    /// knowledge of it in two places -- there is one resolver per front-end, and
    /// it is the same one that reads the rest of the settings file.
    pub fn parse_line(
        &mut self,
        action: &str,
        chord: &str,
        resolve: impl Fn(&str) -> Option<Action>,
    ) -> bool {
        let Some(action) = resolve(action) else {
            return false;
        };
        let Some(chord) = Chord::parse(chord) else {
            return false;
        };
        self.set(action, chord);
        true
    }

    /// Renders every binding as a `key` line.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for (chord, action) in &self.bindings {
            out.push_str("key ");
            out.push_str(action.name());
            out.push(' ');
            out.push_str(&chord.render());
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(text: &str) -> Chord {
        Chord::parse(text).expect("chord parses")
    }

    #[test]
    fn every_default_chord_parses() {
        let keymap = Keymap::default();
        assert_eq!(keymap.bindings().len(), DEFAULTS.len());
    }

    #[test]
    fn every_default_action_has_a_name_that_reads_back() {
        for (action, _) in DEFAULTS {
            assert_eq!(Action::from_name(action.name()), Some(*action));
        }
    }

    #[test]
    fn a_chord_round_trips_through_its_text() {
        for text in [
            "Ctrl+S",
            "Ctrl+Shift+S",
            "Ctrl+Alt+Shift+F",
            "Ctrl+Space",
            "Ctrl+,",
            "Ctrl++",
            "F2",
            "Shift+F3",
            "Ctrl+Shift+Up",
            "Ctrl+Tab",
        ] {
            assert_eq!(chord(text).render(), text);
        }
    }

    #[test]
    fn modifiers_are_read_in_any_order() {
        assert_eq!(chord("Shift+Ctrl+S"), chord("Ctrl+Shift+S"));
        assert_eq!(chord("alt+ctrl+f"), chord("Ctrl+Alt+F"));
    }

    #[test]
    fn a_trailing_plus_is_the_key() {
        let parsed = chord("Ctrl++");
        assert!(parsed.ctrl);
        assert_eq!(parsed.key, KeyName::Char('+'));
    }

    #[test]
    fn a_bare_letter_is_a_character_key() {
        assert_eq!(chord("f").key, KeyName::Char('f'));
        assert_eq!(chord("F").key, KeyName::Char('f'));
    }

    #[test]
    fn a_function_key_is_not_a_character() {
        assert_eq!(chord("F12").key, KeyName::Function(12));
        assert_eq!(chord("f24").key, KeyName::Function(24));
    }

    #[test]
    fn an_out_of_range_function_key_is_rejected() {
        assert_eq!(Chord::parse("F0"), None);
        assert_eq!(Chord::parse("F25"), None);
    }

    #[test]
    fn an_unknown_key_name_is_rejected() {
        assert_eq!(Chord::parse("Ctrl+Banana"), None);
        assert_eq!(Chord::parse(""), None);
    }

    #[test]
    fn an_unsupported_modifier_is_rejected() {
        assert_eq!(Chord::parse("Super+S"), None);
    }

    #[test]
    fn shift_is_dropped_for_punctuation() {
        let plus = Chord::from_char('+', true, true, false);
        assert!(!plus.shift);
        assert_eq!(plus, chord("Ctrl++"));
    }

    #[test]
    fn a_virtual_key_code_reads_as_the_key_it_names() {
        assert_eq!(KeyName::from_vk(0x53), Some(KeyName::Char('s')));
        assert_eq!(KeyName::from_vk(0x70), Some(KeyName::Function(1)));
        assert_eq!(KeyName::from_vk(0x87), Some(KeyName::Function(24)));
        assert_eq!(KeyName::from_vk(0x20), Some(KeyName::Space));
        assert_eq!(KeyName::from_vk(0x2e), Some(KeyName::Delete));
        assert_eq!(KeyName::from_vk(0xbb), Some(KeyName::Char('+')));
        assert_eq!(KeyName::from_vk(0xbd), Some(KeyName::Char('-')));
        assert_eq!(KeyName::from_vk(0xff), None);
    }

    #[test]
    fn every_virtual_key_code_this_editor_reads_writes_back_the_same() {
        // The export is only worth having if it is the exact inverse of the
        // import. Every code `from_vk` accepts has to come back out of `to_vk`
        // unchanged, or a round trip through Notepad++ would move a binding.
        for code in 0u32..=0xff {
            let Some(key) = KeyName::from_vk(code) else {
                continue;
            };
            assert_eq!(key.to_vk(), Some(code), "for VK {code:#04x} ({key:?})");
        }
    }

    #[test]
    fn a_key_with_no_virtual_code_has_no_export() {
        // `=` shares `VK_OEM_PLUS` with `+` on a US layout, so writing a code
        // for it would export a different key than the one bound.
        assert_eq!(KeyName::Char('=').to_vk(), None);
        assert_eq!(KeyName::Char('!').to_vk(), None);
        assert_eq!(KeyName::Function(0).to_vk(), None);
        assert_eq!(KeyName::Function(25).to_vk(), None);
    }

    #[test]
    fn a_virtual_key_code_round_trips_through_the_chord_it_builds() {
        // `Ctrl+S` as Notepad++ writes it: menuCmdID 41005, Key 83.
        let imported = Chord {
            ctrl: true,
            shift: false,
            alt: false,
            key: KeyName::from_vk(83).expect("VK 83 is S"),
        };
        assert_eq!(imported, chord("Ctrl+S"));
    }

    #[test]
    fn shift_is_kept_for_letters() {
        let upper = Chord::from_char('U', true, true, false);
        assert!(upper.shift);
        assert_eq!(upper, chord("Ctrl+Shift+U"));
        assert_ne!(upper, chord("Ctrl+U"));
    }

    #[test]
    fn a_shifted_digit_is_the_digit() {
        // AZERTY puts the digits on the shifted level, so `Ctrl+0` can only be
        // pressed as `Ctrl+Shift+0`. If shift survived, `Restore Default Zoom`
        // would be unreachable on that layout.
        assert_eq!(Chord::from_char('0', true, true, false), chord("Ctrl+0"));
        assert!(!Chord::from_char('0', true, true, false).shift);
        // Letters are unaffected by the same rule.
        assert_eq!(Chord::from_char('S', true, true, false), chord("Ctrl+Shift+S"));
        // And the binding the fix exists for is still reachable.
        let keymap = Keymap::default();
        assert_eq!(
            keymap.lookup(&Chord::from_char('0', true, true, false)),
            Some(Action::ZoomReset)
        );
    }

    #[test]
    fn a_control_character_binds_the_key_that_produced_it() {
        // Windows reports `Ctrl+Shift+J` as the typed text `U+000A`. Binding
        // that would store a chord no keypress can ever match again.
        let newline = Chord::from_char('\u{a}', true, true, false);
        assert_eq!(newline, chord("Ctrl+Shift+J"));
        assert_eq!(Chord::from_char('\u{1}', true, false, false), chord("Ctrl+A"));
        assert_eq!(Chord::from_char('\u{1b}', true, false, false), chord("Ctrl+["));
        assert_eq!(Chord::from_char('\u{7f}', false, false, false), chord("?"));
    }

    #[test]
    fn the_defaults_bind_the_commands_the_menu_advertises() {
        let keymap = Keymap::default();
        assert_eq!(keymap.lookup(&chord("Ctrl+S")), Some(Action::Save));
        assert_eq!(keymap.lookup(&chord("Ctrl+Space")), Some(Action::CompleteWord));
        assert_eq!(
            keymap.lookup(&chord("Ctrl+Shift+Space")),
            Some(Action::CompleteFunction)
        );
        assert_eq!(
            keymap.lookup(&chord("Ctrl+Shift+Alt+Space")),
            Some(Action::ParamHint)
        );
        assert_eq!(
            keymap.lookup(&chord("Ctrl+Alt+Space")),
            Some(Action::CompletePath)
        );
        assert_eq!(keymap.lookup(&chord("Ctrl+,")), Some(Action::Preferences));
        assert_eq!(keymap.lookup(&chord("F3")), Some(Action::FindNext));
        assert_eq!(keymap.lookup(&chord("F5")), Some(Action::Run));
        assert_eq!(keymap.lookup(&chord("Ctrl+PageUp")), Some(Action::PrevTab));
        assert_eq!(
            keymap.lookup(&chord("Ctrl+Tab")),
            Some(Action::DocumentPeeker)
        );
        assert_eq!(keymap.lookup(&chord("Ctrl+Alt+Up")), Some(Action::AddCaretAbove));
        assert_eq!(
            keymap.lookup(&chord("Ctrl+Alt+Down")),
            Some(Action::AddCaretBelow)
        );
    }

    #[test]
    fn an_unbound_chord_runs_nothing() {
        assert_eq!(Keymap::default().lookup(&chord("Ctrl+Alt+Z")), None);
    }

    #[test]
    fn rebinding_frees_the_old_chord() {
        let mut keymap = Keymap::default();
        keymap.set(Action::Save, chord("F5"));
        assert_eq!(keymap.lookup(&chord("F5")), Some(Action::Save));
        assert_eq!(keymap.lookup(&chord("Ctrl+S")), None);
        assert_eq!(keymap.display(Action::Save), "F5");
    }

    #[test]
    fn rebinding_a_chord_takes_it_from_its_old_command() {
        let mut keymap = Keymap::default();
        keymap.set(Action::Save, chord("Ctrl+D"));
        assert_eq!(keymap.lookup(&chord("Ctrl+D")), Some(Action::Save));
        assert_eq!(keymap.display(Action::DuplicateLine), "");
    }

    #[test]
    fn a_command_holds_one_chord() {
        let mut keymap = Keymap::default();
        keymap.set(Action::Save, chord("F5"));
        keymap.set(Action::Save, chord("F6"));
        assert_eq!(keymap.display(Action::Save), "F6");
        assert_eq!(keymap.lookup(&chord("F5")), None);
    }

    #[test]
    fn an_unbound_command_shows_no_shortcut() {
        assert_eq!(Keymap::default().display(Action::SortAsc), "");
    }

    #[test]
    fn the_keymap_round_trips_through_the_file_format() {
        let keymap = Keymap::default();
        let mut parsed = Keymap {
            bindings: Vec::new(),
        };
        for line in keymap.render().lines() {
            let mut parts = line.split(' ');
            assert_eq!(parts.next(), Some("key"));
            let action = parts.next().expect("action");
            let chord = parts.next().expect("chord");
            assert!(parsed.parse_line(action, chord, Action::from_name));
        }
        assert_eq!(parsed, keymap);
    }

    #[test]
    fn an_unknown_action_in_the_file_is_skipped() {
        let mut keymap = Keymap::default();
        assert!(!keymap.parse_line("teleport", "Ctrl+T", Action::from_name));
        assert!(!keymap.parse_line("save", "Ctrl+Banana", Action::from_name));
        assert_eq!(keymap, Keymap::default());
    }

    #[test]
    fn a_file_line_rebinds() {
        let mut keymap = Keymap::default();
        assert!(keymap.parse_line("save", "Ctrl+Shift+S", Action::from_name));
        assert_eq!(keymap.lookup(&chord("Ctrl+Shift+S")), Some(Action::Save));
        assert_eq!(keymap.lookup(&chord("Ctrl+S")), None);
    }

    #[test]
    fn no_two_default_chords_collide() {
        let keymap = Keymap::default();
        let mut seen: Vec<Chord> = Vec::new();
        for (chord, _) in keymap.bindings() {
            assert!(!seen.contains(chord), "{} is bound twice", chord.render());
            seen.push(*chord);
        }
    }

    #[test]
    fn every_default_command_is_offered_by_the_shortcut_editor() {
        for (action, _) in DEFAULTS {
            assert!(
                Action::BINDABLE.contains(action),
                "{} is bound but not listed",
                action.name()
            );
        }
    }
}
