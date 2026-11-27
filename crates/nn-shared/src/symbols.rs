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

//! What a function list says about itself.
//!
//! Both front-ends list what `nn_core::symbols::scan` found in the document in
//! front: a column down the right edge in the window, a list over the bottom of
//! the screen in the terminal. The shapes differ -- one is clicked, the other
//! is walked -- but what a row says and what the header above the rows counts
//! is one decision, and it lives here. A second wording would drift the first
//! time a plural was fixed in one of them, and the drift would be invisible
//! until someone ran both programs side by side.

use nn_core::Symbol;

use crate::results::truncate;

/// How far a nested symbol is indented, in spaces.
const INDENT: &str = "  ";

/// `symbol` as one row of the list, cut to `columns`.
///
/// Indented one step when the scan found it nested, and no further. The scan
/// knows indentation, not scope, so a method inside an `impl` and a closure
/// inside that method both read as "nested" -- which is true, and a tree that
/// claimed more than that would be lying.
pub fn row(symbol: &Symbol, columns: usize) -> String {
    let indent = if symbol.depth > 0 { INDENT } else { "" };
    truncate(&format!("{indent}{}", symbol.name), columns)
}

/// "No symbols", "1 symbol" or "3 symbols", for the header above the list.
///
/// Nothing found is said in words rather than counted as nought: the list is
/// opened by a command, and a header reading "0 symbols" under a command that
/// was just run looks like the command failed.
pub fn header(count: usize) -> String {
    match count {
        0 => "No symbols".to_string(),
        1 => "1 symbol".to_string(),
        n => format!("{n} symbols"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nn_core::SymbolKind;

    fn symbol(name: &str, depth: u8) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind: SymbolKind::Function,
            line: 0,
            depth,
        }
    }

    #[test]
    fn a_nested_symbol_is_indented_one_step_and_no_more() {
        assert_eq!(row(&symbol("outer", 0), 40), "outer");
        assert_eq!(row(&symbol("inner", 1), 40), "  inner");
        // A deeper declaration is drawn where a nested one is: the scan knows
        // indentation, not scope.
        assert_eq!(row(&symbol("deeper", 3), 40), "  deeper");
    }

    #[test]
    fn a_long_name_is_cut_with_a_mark() {
        assert_eq!(row(&symbol("a_very_long_function_name", 0), 10), "a_very_lo…");
        // The cut counts the indent, because the indent is on the row.
        assert_eq!(row(&symbol("a_very_long_function_name", 1), 10), "  a_very_…");
    }

    #[test]
    fn the_header_counts_symbols_and_names_none() {
        assert_eq!(header(0), "No symbols");
        assert_eq!(header(1), "1 symbol");
        assert_eq!(header(2), "2 symbols");
        assert_eq!(header(11), "11 symbols");
    }
}
