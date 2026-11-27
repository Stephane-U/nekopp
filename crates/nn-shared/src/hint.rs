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

//! What a parameter hint decides, on either front-end.
//!
//! The window draws the signature as a line of shaped glyphs floating under the
//! caret, and the terminal draws it as a bordered row of cells; both say the same
//! thing. Which call the caret is inside, what that call declares, where a
//! signature starts and ends, and which argument the highlight is on are one set
//! of decisions, and they live here so a fix to the comma rule cannot land in one
//! front-end and be forgotten in the other.
//!
//! The engine does the reading: `nn_core::call_at` finds the call the caret is
//! inside, `params_of` finds the declaration, and `arg_range` splits the
//! parameter list on the commas that are not inside brackets. What is here is the
//! composition -- one string to show, one range to light -- and the stepping that
//! lets the highlight disagree with the caret until the next keystroke.

use std::ops::Range;

use nn_core::{arg_range, call_at, params_of};

/// The signature of the call the caret is inside, and the argument lit in it.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Hint {
    /// What is shown: `name(param: Type, other: Type)`, as the document declares
    /// it and not as it was typed.
    pub signature: String,
    /// The argument the highlight is on, as a byte range *inside* `signature`.
    pub active: Option<Range<usize>>,
}

impl Hint {
    /// Moves the highlight `delta` arguments along, wrapping at both ends.
    ///
    /// The boundaries are recovered from the signature rather than stored, so a
    /// hint that was stepped away from the caret and then re-shown starts again
    /// from what the document says. A signature with no arguments has nothing to
    /// step through, and a highlight that has not been placed starts from the
    /// first argument, which is what makes the first press mean "the next one".
    pub fn step(&mut self, delta: isize) {
        if delta == 0 {
            return;
        }
        let Some(open) = self.signature.find('(') else {
            return;
        };
        let Some(close) = self.signature.rfind(')') else {
            return;
        };
        if close <= open {
            return;
        }
        let params = &self.signature[open + 1..close];
        if params.trim().is_empty() {
            return;
        }
        let mut ranges = Vec::new();
        for index in 0.. {
            match arg_range(params, index) {
                Some(range) => ranges.push(range),
                None => break,
            }
        }
        if ranges.is_empty() {
            return;
        }
        let current = self
            .active
            .as_ref()
            .and_then(|active| {
                let at = active.start.checked_sub(open + 1)?;
                ranges.iter().position(|range| at >= range.start && at <= range.end)
            })
            .unwrap_or(0);
        let count = ranges.len() as isize;
        let next = (current as isize + delta).rem_euclid(count) as usize;
        let range = ranges[next].clone();
        self.active = Some(open + 1 + range.start..open + 1 + range.end);
    }
}

/// The hint for the call `caret` is inside, or `None` when there is nothing to
/// say.
///
/// `None` covers both cases the hint has to stay quiet for: a caret that is not
/// inside a call at all, and a call to something the document does not declare,
/// where a signature would be a guess.
pub fn hint_at(text: &str, caret: usize) -> Option<Hint> {
    let call = call_at(text, caret)?;
    let params = params_of(text, &call.name)?;
    let signature = format!("{}({params})", call.name);
    // The highlight is a range inside the signature, so it shifts by the name and
    // the opening parenthesis.
    let offset = call.name.len() + 1;
    let active = arg_range(&params, call.arg).map(|range| offset + range.start..offset + range.end);
    Some(Hint { signature, active })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_under_the_caret_gives_the_signature_the_document_declares() {
        let text = concat!(
            "fn save(path: &Path, force: bool) {}\n",
            "save(\"a\", t",
        );
        let hint = hint_at(text, text.len()).expect("a call to a declared name");
        assert_eq!(hint.signature, "save(path: &Path, force: bool)");
        // The caret is in the second argument, so the second is what is lit: the
        // highlight is measured inside the signature, and the argument is trimmed
        // of the space after the comma.
        assert_eq!(hint.active, Some("save(".len() + 13.."save(".len() + 24));
    }

    #[test]
    fn a_name_the_document_does_not_declare_says_nothing() {
        let text = "nothing_declared(1, 2";
        assert!(hint_at(text, text.len()).is_none());
        // And a caret outside any call says nothing either.
        let text = "fn save(a: u8) {}\nlet x = 1;\n";
        assert!(hint_at(text, text.len()).is_none());
    }

    #[test]
    fn the_highlight_walks_the_arguments_and_wraps() {
        let mut hint = Hint {
            signature: "foo(a, b, c)".to_string(),
            active: Some(4..5),
        };
        hint.step(1);
        assert_eq!(hint.active, Some(7..8));
        hint.step(1);
        assert_eq!(hint.active, Some(10..11));
        // Wraps at the end, and back again at the start.
        hint.step(1);
        assert_eq!(hint.active, Some(4..5));
        hint.step(-1);
        assert_eq!(hint.active, Some(10..11));
        // A highlight that was never placed starts from the first argument, so one
        // press means the second.
        let mut unplaced = Hint {
            signature: "foo(a, b)".to_string(),
            active: None,
        };
        unplaced.step(1);
        assert_eq!(unplaced.active, Some(7..8));
    }

    #[test]
    fn a_nested_comma_does_not_split_an_argument() {
        let mut hint = Hint {
            // The comma inside `f(x, y)` belongs to the first argument.
            signature: "g(f(x, y), z)".to_string(),
            active: Some(3..10),
        };
        hint.step(1);
        assert_eq!(hint.active, Some(11..12));
    }

    #[test]
    fn a_signature_with_no_arguments_has_nothing_to_step_through() {
        let mut hint = Hint {
            signature: "now()".to_string(),
            active: None,
        };
        hint.step(1);
        assert_eq!(hint.active, None);
        // A hint that was never shown is the same nothing.
        let mut empty = Hint::default();
        empty.step(-1);
        assert_eq!(empty.active, None);
    }
}
