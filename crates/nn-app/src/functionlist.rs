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

//! The function list panel.
//!
//! A docked column on the right edge, the way Notepad++'s Function List sits.
//! It lists what `nn_core::symbols::scan` found in the active document, indented
//! by declaration depth, and clicking a row jumps to it.
//!
//! Right-docked rather than bottom-docked because the results panel already owns
//! the bottom edge and a symbol list wants height, not width. The editor gives up
//! the column: `EditorView::set_right_dock` shrinks the text area and slides the
//! minimap left, so nothing is drawn underneath.
//!
//! Shaping is keyed by `(symbol count, scroll, width)`, so a frame where nothing
//! moved costs one rectangle and no text work.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use glyphon::{TextArea, TextBounds};
use nn_core::Symbol;

use crate::quad::QuadRenderer;
use crate::theme::Theme;

/// The narrowest and widest the column gets, in pixels.
const MIN_WIDTH: f32 = 170.0;
const MAX_WIDTH: f32 = 320.0;
/// How many characters wide the column aims to be.
const TARGET_COLUMNS: f32 = 30.0;

/// The docked symbol list.
pub struct FunctionList {
    symbols: Vec<Symbol>,
    /// Index of the first visible symbol.
    scroll: usize,
    /// How many rows fit, from the last `prepare`.
    visible: usize,
    open: bool,
    header: Buffer,
    header_text: String,
    header_shaped: String,
    /// Every visible row in one buffer, newline separated.
    ///
    /// One buffer rather than one per row: glyphon lays out each text area it is
    /// handed, and a hundred areas cost about 2 ms a frame at 4K — the whole
    /// panel's budget. A single multi-line buffer is one area and one layout.
    body: Buffer,
    body_shaped: String,
    /// What the body was shaped for: (symbol count, scroll, width in chars).
    shaped_key: (usize, usize, usize),
    metrics: Metrics,
    family: Family<'static>,
    char_width: f32,
}

impl FunctionList {
    /// Builds a closed panel.
    pub fn new(metrics: Metrics, family: Family<'static>, char_width: f32) -> Self {
        let mut header = Buffer::new_empty(metrics);
        header.set_size(None, None);
        header.set_text("", &Attrs::new().family(family), Shaping::Basic, None);
        Self {
            symbols: Vec::new(),
            scroll: 0,
            visible: 0,
            open: false,
            header,
            header_text: String::new(),
            header_shaped: String::new(),
            body: Buffer::new_empty(metrics),
            body_shaped: String::new(),
            shaped_key: (usize::MAX, usize::MAX, usize::MAX),
            metrics,
            family,
            char_width,
        }
    }

    /// Replaces the symbol list. An empty list closes the panel.
    pub fn set_symbols(&mut self, symbols: Vec<Symbol>) {
        self.header_text = match symbols.len() {
            0 => "No symbols".to_string(),
            1 => "1 symbol".to_string(),
            n => format!("{n} symbols"),
        };
        self.symbols = symbols;
        self.scroll = 0;
        self.open = !self.symbols.is_empty();
        self.shaped_key = (usize::MAX, usize::MAX, usize::MAX);
    }

    /// Closes the panel and drops the list.
    pub fn close(&mut self) {
        self.open = false;
        self.symbols.clear();
        self.scroll = 0;
        self.body_shaped.clear();
        self.shaped_key = (usize::MAX, usize::MAX, usize::MAX);
    }

    /// Whether the panel is showing.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The symbols, for the click handler.
    pub fn symbols(&self) -> &[Symbol] {
        &self.symbols
    }

    /// The width the column occupies, zero when closed.
    pub fn width(&self, char_width: f32) -> f32 {
        if !self.open {
            return 0.0;
        }
        (char_width * TARGET_COLUMNS).clamp(MIN_WIDTH, MAX_WIDTH)
    }

    /// Scrolls the list by `delta` rows.
    pub fn scroll_by(&mut self, delta: isize) {
        let max = self.symbols.len().saturating_sub(self.visible);
        let next = (self.scroll as isize + delta).clamp(0, max as isize);
        self.scroll = next as usize;
    }

    /// The symbol index under `y`, where `top` is the panel's top edge.
    pub fn symbol_at(&self, y: f32, top: f32, line_height: f32) -> Option<usize> {
        if !self.open {
            return None;
        }
        let first = top + line_height + 3.0;
        if y < first {
            return None;
        }
        let row = ((y - first) / line_height) as usize;
        if row >= self.visible {
            return None;
        }
        let index = self.scroll + row;
        (index < self.symbols.len()).then_some(index)
    }

    /// Rebuilds the shaped rows if anything they depend on changed.
    pub fn prepare(&mut self, font_system: &mut FontSystem, width: f32, height: f32) {
        if !self.open {
            return;
        }
        let line_height = self.metrics.line_height;
        self.visible = (((height - line_height - 3.0) / line_height).floor() as usize).max(1);
        let max = self.symbols.len().saturating_sub(self.visible);
        self.scroll = self.scroll.min(max);

        let columns = ((width - 16.0) / self.char_width).max(8.0) as usize;
        let key = (self.symbols.len(), self.scroll, columns);
        if key == self.shaped_key {
            return;
        }
        self.shaped_key = key;

        if self.header_text != self.header_shaped {
            self.header.set_text(
                &self.header_text,
                &Attrs::new().family(self.family),
                Shaping::Basic,
                None,
            );
            self.header.shape_until_scroll(font_system, false);
            self.header_shaped.clone_from(&self.header_text);
        }

        let visible = self.symbols.len().min(self.visible);
        let mut text = String::with_capacity(visible * (columns + 1));
        for row in 0..visible {
            if row > 0 {
                text.push('\n');
            }
            text.push_str(&self.row_text(self.scroll + row, columns));
        }
        if text == self.body_shaped {
            return;
        }
        self.body.set_text(
            &text,
            &Attrs::new().family(self.family),
            Shaping::Basic,
            None,
        );
        self.body.shape_until_scroll(font_system, false);
        self.body_shaped = text;
    }

    /// The display text of symbol `index`, cut to `columns`.
    ///
    /// Indented one step for anything nested, and no further. The scan knows
    /// indentation, not scope, so a method inside an `impl` and a closure inside
    /// that method both read as "nested" — which is true, and a two-level tree
    /// that claims more than that would be lying.
    fn row_text(&self, index: usize, columns: usize) -> String {
        let Some(symbol) = self.symbols.get(index) else {
            return String::new();
        };
        let indent = if symbol.depth > 0 { "  " } else { "" };
        let text = format!("{indent}{}", symbol.name);
        truncate(&text, columns)
    }

    /// Queues the panel background and its left border.
    pub fn push_quads(
        &self,
        quads: &mut QuadRenderer,
        width: f32,
        top: f32,
        height: f32,
        theme: &Theme,
    ) {
        if !self.open {
            return;
        }
        let left = width - self.width(self.char_width);
        quads.push(left, top, width, height, theme.bar_background.to_linear());
        quads.push(left, top, left + 1.0, height, theme.bar_border.to_linear());
    }

    /// The text areas of the header and the visible rows.
    pub fn text_areas<'a>(
        &'a self,
        width: f32,
        top: f32,
        height: f32,
        padding: f32,
        theme: &Theme,
    ) -> Vec<TextArea<'a>> {
        if !self.open {
            return Vec::new();
        }
        let left = width - self.width(self.char_width);
        let bounds = TextBounds {
            left: left as i32,
            top: top as i32,
            right: width as i32,
            bottom: (top + height) as i32,
        };
        vec![
            TextArea {
                buffer: &self.header,
                left: left + padding,
                top: top + 3.0,
                scale: 1.0,
                bounds,
                default_color: theme.bar_label.to_cosmic(),
                custom_glyphs: &[],
            },
            TextArea {
                buffer: &self.body,
                left: left + padding,
                top: top + self.metrics.line_height + 3.0,
                scale: 1.0,
                bounds,
                default_color: theme.foreground.to_cosmic(),
                custom_glyphs: &[],
            },
        ]
    }
}

/// Cuts `text` to `columns` characters, marking the cut with an ellipsis.
fn truncate(text: &str, columns: usize) -> String {
    if text.chars().count() <= columns {
        return text.to_string();
    }
    let mut out: String = text.chars().take(columns.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use nn_core::SymbolKind;

    fn panel() -> FunctionList {
        FunctionList::new(Metrics::new(15.0, 20.0), Family::Monospace, 8.0)
    }

    fn symbol(name: &str, line: usize, depth: u8) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind: SymbolKind::Function,
            line,
            depth,
        }
    }

    #[test]
    fn a_closed_panel_takes_no_width() {
        assert_eq!(panel().width(8.0), 0.0);
    }

    #[test]
    fn setting_symbols_opens_the_panel() {
        let mut p = panel();
        p.set_symbols(vec![symbol("main", 0, 0)]);
        assert!(p.is_open());
        assert!(p.width(8.0) >= MIN_WIDTH);
    }

    #[test]
    fn an_empty_scan_leaves_the_panel_closed() {
        let mut p = panel();
        p.set_symbols(Vec::new());
        assert!(!p.is_open());
        assert_eq!(p.width(8.0), 0.0);
    }

    #[test]
    fn the_width_is_clamped() {
        let mut p = panel();
        p.set_symbols(vec![symbol("main", 0, 0)]);
        assert_eq!(p.width(1.0), MIN_WIDTH);
        assert_eq!(p.width(100.0), MAX_WIDTH);
    }

    #[test]
    fn the_header_counts_symbols() {
        let mut p = panel();
        p.set_symbols(vec![symbol("a", 0, 0)]);
        assert_eq!(p.header_text, "1 symbol");
        p.set_symbols(vec![symbol("a", 0, 0), symbol("b", 1, 0)]);
        assert_eq!(p.header_text, "2 symbols");
    }

    #[test]
    fn nested_symbols_are_indented_one_step() {
        let mut p = panel();
        p.set_symbols(vec![
            symbol("outer", 0, 0),
            symbol("inner", 1, 1),
            symbol("deeper", 2, 3),
        ]);
        assert_eq!(p.row_text(0, 40), "outer");
        assert_eq!(p.row_text(1, 40), "  inner");
        assert_eq!(p.row_text(2, 40), "  deeper");
    }

    #[test]
    fn a_long_name_is_cut() {
        let mut p = panel();
        p.set_symbols(vec![symbol("a_very_long_function_name", 0, 0)]);
        assert_eq!(p.row_text(0, 10), "a_very_lo…");
    }

    #[test]
    fn symbol_at_maps_a_row_to_a_symbol() {
        let mut p = panel();
        p.set_symbols((0..20).map(|i| symbol("f", i, 0)).collect());
        p.prepare(&mut FontSystem::new(), 200.0, 200.0);
        assert_eq!(p.symbol_at(23.0, 0.0, 20.0), Some(0));
        assert_eq!(p.symbol_at(23.0 + 20.0 * 2.0, 0.0, 20.0), Some(2));
        assert_eq!(p.symbol_at(5.0, 0.0, 20.0), None);
    }

    #[test]
    fn symbol_at_follows_the_scroll() {
        let mut p = panel();
        p.set_symbols((0..40).map(|i| symbol("f", i, 0)).collect());
        p.prepare(&mut FontSystem::new(), 200.0, 200.0);
        p.scroll_by(4);
        assert_eq!(p.symbol_at(23.0, 0.0, 20.0), Some(4));
    }

    #[test]
    fn scrolling_clamps_to_the_list() {
        let mut p = panel();
        p.set_symbols((0..40).map(|i| symbol("f", i, 0)).collect());
        p.prepare(&mut FontSystem::new(), 200.0, 200.0);
        p.scroll_by(-5);
        assert_eq!(p.scroll, 0);
        p.scroll_by(1000);
        assert_eq!(p.scroll, 40 - p.visible);
    }

    #[test]
    fn closing_drops_the_list() {
        let mut p = panel();
        p.set_symbols(vec![symbol("main", 0, 0)]);
        p.close();
        assert!(!p.is_open());
        assert!(p.symbols().is_empty());
        assert_eq!(p.width(8.0), 0.0);
    }

    #[test]
    fn truncate_marks_the_cut() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("abc", 4), "abc");
    }
}
