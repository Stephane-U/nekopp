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

//! The document change history panel.
//!
//! Notepad++'s "Document Change History" is a docked list of every change made
//! to the current document, newest at the bottom, with a click on a row reverting
//! the document to the state it was in after that change. It is the undo stack
//! made visible: the same steps, in the same order, with the context that makes
//! one recognisable — when it happened, where, and what it did.
//!
//! The panel does not own the history. It is handed a list of rows by the
//! application, which builds them from the editor's undo stack, and it reports
//! back which row was clicked. That keeps the panel free of any knowledge of
//! documents, which is what lets it be tested without one.
//!
//! Rows are shaped lazily and cached against `(row count, scroll, width)`, the
//! same way the results panel does it: a frame where nothing moved costs one
//! rectangle and no text work.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use glyphon::{TextArea, TextBounds};
use nn_core::clock_stamp;

use crate::quad::QuadRenderer;
use crate::theme::Theme;

/// How many rows the panel shows at once.
///
/// Eight, the same as the results panel, so the two docked lists are the same
/// height and swapping between them does not move the editor.
const VISIBLE_ROWS: usize = 8;

/// One line of the history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// How many undo steps remain if the document is reverted to this row.
    ///
    /// A count rather than a step index because the panel is rebuilt from the
    /// stack on every change, and an index would go stale the moment a new edit
    /// arrived. A count is stable: "revert to this row" means "undo until this
    /// many steps are left".
    pub keep: usize,
    /// The 1-based line the change happened on.
    pub line: usize,
    /// Wall-clock second the change was recorded at, or 0 when unknown.
    pub at: u64,
    /// What the change did, from `Step::describe`.
    pub what: String,
}

/// The docked change-history list.
pub struct ChangeHistory {
    rows: Vec<Row>,
    /// Index of the first visible row.
    scroll: usize,
    open: bool,
    /// One shaped buffer per visible row.
    buffers: Vec<Buffer>,
    /// The text currently shaped into each row, so a reshape is skipped when
    /// nothing changed.
    shaped: Vec<String>,
    /// What the rows were shaped for: (row count, scroll, width in chars).
    shaped_key: (usize, usize, usize),
    header: Buffer,
    header_text: String,
    header_shaped: String,
    metrics: Metrics,
    family: Family<'static>,
    char_width: f32,
}

impl ChangeHistory {
    /// Builds a closed panel.
    pub fn new(metrics: Metrics, family: Family<'static>, char_width: f32) -> Self {
        let mut header = Buffer::new_empty(metrics);
        header.set_size(None, None);
        header.set_text("", &Attrs::new().family(family), Shaping::Basic, None);
        Self {
            rows: Vec::new(),
            scroll: 0,
            open: false,
            buffers: Vec::new(),
            shaped: Vec::new(),
            shaped_key: (usize::MAX, usize::MAX, usize::MAX),
            header,
            header_text: String::new(),
            header_shaped: String::new(),
            metrics,
            family,
            char_width,
        }
    }

    /// Rebuilds the list at a new display density.
    pub fn set_metrics(&mut self, metrics: Metrics) {
        if metrics == self.metrics {
            return;
        }
        self.metrics = metrics;
        self.header.set_metrics(metrics);
        for row in &mut self.buffers {
            row.set_metrics(metrics);
        }
    }

    /// Replaces the rows and opens the panel.
    ///
    /// The list is scrolled to the bottom, because the newest change is the one
    /// the user just made and the one they are most likely to want to undo.
    pub fn set_rows(&mut self, rows: Vec<Row>) {
        self.header_text = match rows.len() {
            0 => "No changes".to_string(),
            1 => "1 change".to_string(),
            n => format!("{n} changes"),
        };
        self.rows = rows;
        self.scroll = self.rows.len().saturating_sub(VISIBLE_ROWS);
        self.open = true;
        self.shaped_key = (usize::MAX, usize::MAX, usize::MAX);
    }

    /// Closes the panel and drops the rows.
    pub fn close(&mut self) {
        self.open = false;
        self.rows.clear();
        self.scroll = 0;
        self.buffers.clear();
        self.shaped.clear();
        self.shaped_key = (usize::MAX, usize::MAX, usize::MAX);
    }

    /// Whether the panel is showing.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The rows, for the click handler.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The height the panel occupies.
    pub fn height(&self, line_height: f32) -> f32 {
        if !self.open {
            return 0.0;
        }
        let rows = self.rows.len().min(VISIBLE_ROWS) as f32;
        line_height * (rows + 1.0) + 6.0
    }

    /// Scrolls the list by `delta` rows.
    pub fn scroll_by(&mut self, delta: isize) {
        let max = self.rows.len().saturating_sub(VISIBLE_ROWS);
        let next = (self.scroll as isize + delta).clamp(0, max as isize);
        self.scroll = next as usize;
    }

    /// The row index under window pixel `y`, if any.
    pub fn row_at(&self, y: f32, top: f32, line_height: f32) -> Option<usize> {
        if !self.open {
            return None;
        }
        // The header occupies the first line; rows start below it.
        let offset = y - top - line_height;
        if offset < 0.0 {
            return None;
        }
        let row = (offset / line_height) as usize;
        let index = self.scroll + row;
        (index < self.rows.len()).then_some(index)
    }

    /// Rebuilds the shaped rows if anything they depend on changed.
    pub fn prepare(&mut self, font_system: &mut FontSystem, width: f32) {
        if !self.open {
            return;
        }
        let columns = ((width - 16.0) / self.char_width).max(8.0) as usize;
        let key = (self.rows.len(), self.scroll, columns);
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

        let visible = self.rows.len().min(VISIBLE_ROWS);
        self.buffers.resize_with(visible, || {
            let mut buffer = Buffer::new_empty(self.metrics);
            buffer.set_size(None, None);
            buffer
        });
        self.shaped.resize(visible, String::new());
        for row in 0..visible {
            let text = row_text(&self.rows[self.scroll + row], columns);
            if self.shaped[row] == text {
                continue;
            }
            self.buffers[row].set_text(
                &text,
                &Attrs::new().family(self.family),
                Shaping::Basic,
                None,
            );
            self.buffers[row].shape_until_scroll(font_system, false);
            self.shaped[row].clone_from(&text);
        }
    }

    /// Queues the panel background and its top border.
    pub fn push_quads(
        &self,
        quads: &mut QuadRenderer,
        width: f32,
        top: f32,
        line_height: f32,
        theme: &Theme,
    ) {
        if !self.open {
            return;
        }
        let height = self.height(line_height);
        quads.push(
            0.0,
            top,
            width,
            height,
            theme.bar_background.to_linear(),
        );
        quads.push(0.0, top, width, 1.0, theme.bar_border.to_linear());
    }

    /// The text areas of the header and the visible rows.
    pub fn text_areas<'a>(
        &'a self,
        width: f32,
        top: f32,
        line_height: f32,
        padding: f32,
        theme: &Theme,
    ) -> Vec<TextArea<'a>> {
        if !self.open {
            return Vec::new();
        }
        let height = self.height(line_height);
        let bounds = TextBounds {
            left: 0,
            top: top as i32,
            right: width as i32,
            bottom: (top + height) as i32,
        };
        let mut areas = Vec::with_capacity(self.buffers.len() + 1);
        areas.push(TextArea {
            buffer: &self.header,
            left: padding,
            top: top + 3.0,
            scale: 1.0,
            bounds,
            default_color: theme.bar_label.to_cosmic(),
            custom_glyphs: &[],
        });
        for (row, buffer) in self.buffers.iter().enumerate() {
            areas.push(TextArea {
                buffer,
                left: padding,
                top: top + line_height + 3.0 + row as f32 * line_height,
                scale: 1.0,
                bounds,
                default_color: theme.foreground.to_cosmic(),
                custom_glyphs: &[],
            });
        }
        areas
    }

    /// The metrics the panel was built with.
    pub fn metrics(&self) -> Metrics {
        self.metrics
    }
}

/// The display text of one row, truncated to `columns`.
///
/// `line`, then the clock, then what happened. The line comes first because it
/// is the column the eye scans when looking for "the change I made in that
/// function"; the clock is second because it is only useful once the row has been
/// found.
fn row_text(row: &Row, columns: usize) -> String {
    let clock = if row.at == 0 {
        "--:--:--".to_string()
    } else {
        clock_stamp(row.at as i64)
    };
    let text = format!("{:>6}  {}  {}", row.line, clock, row.what);
    truncate(&text, columns)
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

    fn panel() -> ChangeHistory {
        let mut font_system = FontSystem::new();
        let metrics = Metrics::new(15.0, 20.0);
        let family = Family::Monospace;
        let mut panel = ChangeHistory::new(metrics, family, 8.0);
        panel.prepare(&mut font_system, 800.0);
        panel
    }

    fn row(keep: usize, line: usize, at: u64, what: &str) -> Row {
        Row {
            keep,
            line,
            at,
            what: what.to_string(),
        }
    }

    #[test]
    fn a_closed_panel_takes_no_space() {
        let panel = panel();
        assert!(!panel.is_open());
        assert_eq!(panel.height(20.0), 0.0);
        assert!(panel.rows().is_empty());
    }

    #[test]
    fn an_open_panel_is_a_header_plus_rows() {
        let mut panel = panel();
        panel.set_rows(vec![row(1, 3, 0, "Insert 1 char")]);
        assert!(panel.is_open());
        // One row plus the header, plus the border padding.
        assert_eq!(panel.height(20.0), 20.0 * 2.0 + 6.0);
    }

    #[test]
    fn the_panel_caps_at_the_visible_row_count() {
        let mut panel = panel();
        let rows: Vec<Row> = (0..40).map(|i| row(i, i + 1, 0, "Insert 1 char")).collect();
        panel.set_rows(rows);
        assert_eq!(panel.height(20.0), 20.0 * (VISIBLE_ROWS as f32 + 1.0) + 6.0);
    }

    #[test]
    fn opening_scrolls_to_the_newest_change() {
        let mut panel = panel();
        let rows: Vec<Row> = (0..40).map(|i| row(i, i + 1, 0, "Insert 1 char")).collect();
        panel.set_rows(rows);
        // The last row is visible without scrolling.
        let top = 100.0;
        let last = top + 20.0 + 7.0 * 20.0;
        assert_eq!(panel.row_at(last, top, 20.0), Some(39));
    }

    #[test]
    fn a_short_list_does_not_scroll() {
        let mut panel = panel();
        panel.set_rows(vec![
            row(1, 1, 0, "Insert 1 char"),
            row(2, 2, 0, "Delete 1 char"),
        ]);
        panel.scroll_by(-5);
        assert_eq!(panel.row_at(100.0 + 20.0, 100.0, 20.0), Some(0));
    }

    #[test]
    fn scrolling_moves_the_window() {
        let mut panel = panel();
        let rows: Vec<Row> = (0..20).map(|i| row(i, i + 1, 0, "Insert 1 char")).collect();
        panel.set_rows(rows);
        panel.scroll_by(-4);
        // Row 0 of the window is now index 8.
        assert_eq!(panel.row_at(100.0 + 20.0, 100.0, 20.0), Some(8));
    }

    #[test]
    fn scrolling_clamps_to_the_list() {
        let mut panel = panel();
        let rows: Vec<Row> = (0..20).map(|i| row(i, i + 1, 0, "Insert 1 char")).collect();
        panel.set_rows(rows);
        panel.scroll_by(-1000);
        assert_eq!(panel.row_at(100.0 + 20.0, 100.0, 20.0), Some(0));
        panel.scroll_by(1000);
        assert_eq!(panel.row_at(100.0 + 20.0, 100.0, 20.0), Some(12));
    }

    #[test]
    fn the_header_is_not_a_row() {
        let mut panel = panel();
        panel.set_rows(vec![row(1, 1, 0, "Insert 1 char")]);
        // A click on the header line selects nothing.
        assert_eq!(panel.row_at(100.0, 100.0, 20.0), None);
        assert_eq!(panel.row_at(100.0 + 19.0, 100.0, 20.0), None);
    }

    #[test]
    fn a_click_past_the_last_row_is_none() {
        let mut panel = panel();
        panel.set_rows(vec![row(1, 1, 0, "Insert 1 char")]);
        assert_eq!(panel.row_at(100.0 + 20.0 + 20.0, 100.0, 20.0), None);
    }

    #[test]
    fn a_closed_panel_has_no_rows_to_click() {
        let panel = panel();
        assert_eq!(panel.row_at(120.0, 100.0, 20.0), None);
    }

    #[test]
    fn closing_drops_the_rows() {
        let mut panel = panel();
        panel.set_rows(vec![row(1, 1, 0, "Insert 1 char")]);
        panel.close();
        assert!(!panel.is_open());
        assert!(panel.rows().is_empty());
    }

    #[test]
    fn a_row_shows_the_line_the_clock_and_the_change() {
        let text = row_text(&row(1, 42, 0, "Insert 3 chars"), 80);
        assert!(text.contains("42"), "{text}");
        assert!(text.contains("--:--:--"), "{text}");
        assert!(text.contains("Insert 3 chars"), "{text}");
    }

    #[test]
    fn a_row_with_a_clock_shows_the_time() {
        let text = row_text(&row(1, 7, 1_700_000_000, "Delete 1 char"), 80);
        let clock = clock_stamp(1_700_000_000);
        assert!(text.contains(&clock), "{text} should contain {clock}");
    }

    #[test]
    fn a_long_row_is_truncated() {
        let text = row_text(&row(1, 1, 0, &"x".repeat(200)), 20);
        assert_eq!(text.chars().count(), 20);
        assert!(text.ends_with('…'));
    }

    #[test]
    fn the_header_counts_the_changes() {
        let mut panel = panel();
        panel.set_rows(Vec::new());
        assert_eq!(panel.header_text, "No changes");
        panel.set_rows(vec![row(1, 1, 0, "Insert 1 char")]);
        assert_eq!(panel.header_text, "1 change");
        panel.set_rows(vec![
            row(1, 1, 0, "Insert 1 char"),
            row(2, 2, 0, "Delete 1 char"),
        ]);
        assert_eq!(panel.header_text, "2 changes");
    }
}
