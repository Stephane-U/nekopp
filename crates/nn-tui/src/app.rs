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

//! One document, drawn in cells, with a caret and a status bar.
//!
//! Everything an editor does with text -- the caret, the motions, the undo
//! history, the auto-indent, the lexers -- belongs to `nn-core`, and this module
//! does not reimplement any of it. What it owns is the shape of the picture: how
//! many columns the gutter takes, which lines are on screen, where the caret
//! lands once a tab has been expanded, and what the bar at the bottom says.
//!
//! [`App::draw`] is a pure function of that state onto a [`Grid`], so the whole
//! of the layout can be tested without a terminal: the tests here open
//! documents, press keys and read cells back.

use std::path::Path;

use nn_core::{CLikeLexer, ConfigLexer, Document, Editor, Highlighter, Lexer, Motion};
use nn_shared::action::Action;
use nn_shared::keymap::Keymap;
use unicode_width::UnicodeWidthChar;

use crate::keys::Key;
use crate::menus::Menus;
use crate::palette::{self, Tone};
use crate::screen::Grid;

/// Columns per tab stop, while the tab character is still one character.
const TAB: usize = 4;

/// How the window is divided up, worked out once per frame.
///
/// The bar is the last row, so the number of rows of text and the row the bar is
/// drawn on are the same number: one field rather than two that would have to
/// agree.
#[derive(Debug, Clone, Copy)]
struct Layout {
    /// Columns the line numbers take.
    gutter: usize,
    /// Columns left for the text.
    text_cols: usize,
    /// The row the bar is drawn on, and therefore how many rows are above it.
    status_row: usize,
}

/// A message for the bar, and whether it is bad news.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Message {
    text: String,
    alert: bool,
}

/// The editor, as the terminal sees it.
pub struct App {
    /// The document and its caret, from the core.
    pub editor: Editor,
    highlighter: Highlighter,
    /// The first line on screen.
    scroll_line: usize,
    /// The first display column of the text area on screen.
    scroll_col: usize,
    /// The bar and its dropdowns, with the shared tree behind them.
    menus: Menus,
    message: Option<Message>,
    /// Set once `Ctrl+Q` has been pressed with unsaved changes, so the second
    /// press means what the first one warned about.
    quit_armed: bool,
    quit: bool,
}

impl App {
    /// Opens `path`, or a buffer named `[No Name]` when there is none.
    pub fn open(path: Option<&Path>) -> std::io::Result<Self> {
        let doc = match path {
            Some(path) => Document::from_file(path)?,
            None => Document::new(),
        };
        Ok(App::with_document(doc))
    }

    /// Wraps a document that is already in memory.
    pub fn with_document(doc: Document) -> Self {
        let mut editor = Editor::new(doc);
        let mut highlighter = Highlighter::new(lexer_for(editor.doc.path()));
        // A large file opens unhighlighted for the same reason it does in the
        // windowed editor: both passes are O(file) and the pause is not worth it.
        if !editor.doc.is_large() {
            highlighter.highlight_all(&mut editor.doc);
        }
        App {
            editor,
            highlighter,
            scroll_line: 0,
            scroll_col: 0,
            menus: Menus::new(&Keymap::default()),
            message: None,
            quit_armed: false,
            quit: false,
        }
    }

    /// Whether the loop should stop.
    pub fn take_quit(&mut self) -> bool {
        std::mem::take(&mut self.quit)
    }

    /// What the bottom bar would be saying. Only the tests ask.
    #[cfg(test)]
    pub fn message(&self) -> Option<&str> {
        self.message.as_ref().map(|one| one.text.as_str())
    }

    /// Handles one key.
    ///
    /// A message is cleared first, so anything the bar said lasts until the next
    /// keystroke and no longer. The line the caret was on is remembered before
    /// the edit, because re-lexing starts there rather than at the top of the
    /// file.
    pub fn handle(&mut self, key: Key) {
        // While a dropdown is up it owns the keyboard: a keystroke means "move
        // the cursor" rather than "type a letter", which is the same rule the
        // window follows with its modal menus.
        if self.menus.is_open() {
            if let Some(action) = self.menus.handle(key) {
                self.run(action);
            }
            return;
        }
        // F9 opens the menus, as in Midnight Commander; F10 is for hands trained
        // on Notepad++. A menu opened by a keystroke is a menu whose keyboard
        // focus has already moved, so nothing else happens on that press.
        if matches!(key, Key::Function(9) | Key::Function(10)) {
            self.message = None;
            self.menus.toggle();
            return;
        }
        self.message = None;
        let line = self.editor.caret_line();
        match key {
            Key::Char(ch) => {
                self.editor.insert(&ch.to_string());
                self.refresh_from(line);
            }
            Key::Enter => {
                let indent = self.editor.auto_indent();
                self.editor.insert(&format!("\n{indent}"));
                self.refresh_from(line);
            }
            Key::Tab => {
                self.editor.insert("    ");
                self.refresh_from(line);
            }
            Key::Backspace => {
                self.editor.backspace();
                self.refresh_from(line);
            }
            Key::Delete => {
                self.editor.delete_forward();
                self.refresh_from(line);
            }
            Key::Up => self.editor.move_caret(Motion::LineUp, false),
            Key::Down => self.editor.move_caret(Motion::LineDown, false),
            Key::Left => self.editor.move_caret(Motion::CharLeft, false),
            Key::Right => self.editor.move_caret(Motion::CharRight, false),
            Key::ExtendUp => self.editor.move_caret(Motion::LineUp, true),
            Key::ExtendDown => self.editor.move_caret(Motion::LineDown, true),
            Key::ExtendLeft => self.editor.move_caret(Motion::CharLeft, true),
            Key::ExtendRight => self.editor.move_caret(Motion::CharRight, true),
            Key::WordLeft => self.editor.move_caret(Motion::WordLeft, false),
            Key::WordRight => self.editor.move_caret(Motion::WordRight, false),
            Key::LineStart => self.editor.move_caret(Motion::LineStart, false),
            Key::LineEnd => self.editor.move_caret(Motion::LineEnd, false),
            Key::DocStart => self.editor.move_caret(Motion::DocStart, false),
            Key::DocEnd => self.editor.move_caret(Motion::DocEnd, false),
            Key::PageUp => self.editor.move_caret(Motion::PageUp, false),
            Key::PageDown => self.editor.move_caret(Motion::PageDown, false),
            Key::Ctrl('s') => self.save(),
            Key::Ctrl('q') => self.ask_to_quit(),
            Key::Ctrl('z') => {
                if self.editor.undo() {
                    self.refresh_all();
                }
            }
            Key::Ctrl('y') => {
                if self.editor.redo() {
                    self.refresh_all();
                }
            }
            Key::Ctrl('a') => self.editor.select_all(),
            Key::Escape => {
                self.quit_armed = false;
                self.editor.clear_extra_carets();
            }
            Key::Ctrl(_) => {}
            Key::Function(_) => {}
        }
    }

    /// Runs a command chosen from a menu.
    ///
    /// The tree is the window's, so it offers all two hundred and forty-one
    /// commands and this shell has six of them. The rest say what they are
    /// rather than doing nothing: a menu that silently ignores a press teaches a
    /// user that the program is broken, and the list of what is missing is the
    /// shell's own work list.
    fn run(&mut self, action: Action) {
        self.message = None;
        match action {
            Action::Save => self.save(),
            Action::Undo => {
                if self.editor.undo() {
                    self.refresh_all();
                }
            }
            Action::Redo => {
                if self.editor.redo() {
                    self.refresh_all();
                }
            }
            Action::SelectAll => self.editor.select_all(),
            Action::Exit => self.ask_to_quit(),
            other => {
                let label = other.label();
                self.say(&format!("{label}: not in the terminal yet"), true);
            }
        }
    }

    /// Writes the document back, and says what happened.
    fn save(&mut self) {
        if self.editor.doc.path().is_none() {
            self.say("no file name: start with a path to save", true);
            return;
        }
        match self.editor.doc.save() {
            Ok(()) => self.say("saved", false),
            Err(error) => self.say(&format!("save failed: {error}"), true),
        }
    }

    /// Quits, or warns once that there is unsaved work.
    fn ask_to_quit(&mut self) {
        if self.editor.doc.is_dirty() && !self.quit_armed {
            self.quit_armed = true;
            self.say("unsaved changes: Ctrl+Q again discards them", true);
            return;
        }
        self.quit = true;
    }

    fn say(&mut self, text: &str, alert: bool) {
        self.message = Some(Message {
            text: text.to_string(),
            alert,
        });
    }

    /// Re-lexes from `line` down, which is all an edit can have changed.
    fn refresh_from(&mut self, line: usize) {
        self.highlighter.highlight_from(&mut self.editor.doc, line);
    }

    /// Re-lexes everything: undo and redo can have changed any line.
    fn refresh_all(&mut self) {
        if self.editor.doc.is_large() {
            let line = self.editor.caret_line();
            self.refresh_from(line);
        } else {
            self.highlighter.highlight_all(&mut self.editor.doc);
        }
    }

    /// How many columns the line numbers take, with their trailing space.
    fn gutter_width(&self) -> usize {
        let digits = self.editor.doc.len_lines().to_string().len();
        digits + 1
    }

    /// The display column of character `column` of `line`, with tabs expanded.
    fn display_column(&self, line: usize, column: usize) -> usize {
        let mut display = 0;
        for (index, ch) in self.editor.doc.line_chars(line).enumerate() {
            if index >= column || ch == '\n' || ch == '\r' {
                break;
            }
            display = advance(display, ch);
        }
        display
    }

    /// Draws the whole window.
    pub fn draw(&mut self, grid: &mut Grid) {
        let width = grid.width();
        let height = grid.height();
        if width == 0 || height == 0 {
            return;
        }
        let gutter = self.gutter_width().min(width);
        let layout = Layout {
            gutter,
            text_cols: width - gutter,
            status_row: height - 1,
        };
        let text_rows = layout.status_row;
        let status_row = layout.status_row;

        // The core moves by a screenful on Page Up and Page Down, and it takes
        // "a screenful" from the viewport. In cells, a line is one unit high.
        let viewport = &mut self.editor.viewport;
        viewport.height = text_rows as f32;
        viewport.line_height = 1.0;
        viewport.char_width = 1.0;
        viewport.width = layout.text_cols as f32;

        let caret_line = self.editor.caret_line();
        let caret_col = self.editor.caret_column();
        let caret_display = self.display_column(caret_line, caret_col);

        // Keep the caret on the screen, scrolling the least that will do it.
        if caret_line < self.scroll_line {
            self.scroll_line = caret_line;
        }
        if text_rows > 0 && caret_line >= self.scroll_line + text_rows {
            self.scroll_line = caret_line + 1 - text_rows;
        }
        if caret_display < self.scroll_col {
            self.scroll_col = caret_display;
        }
        if layout.text_cols > 0 && caret_display >= self.scroll_col + layout.text_cols {
            self.scroll_col = caret_display + 1 - layout.text_cols;
        }

        let caret = self.editor.caret();
        let selection = self.editor.selection_start()..self.editor.selection_end();

        for row in 0..text_rows {
            // Every row is painted edge to edge, in the two tones the windowed
            // editor uses for the same strips. Leaving the cells past the end of
            // a line alone would let the terminal's own background through, and
            // it would leave a menu's box behind after a dropdown closed over
            // those cells: a frame that only writes what it has something to say
            // about cannot erase.
            for column in 0..layout.gutter.min(width) {
                grid.put(row, column, ' ', Tone::Gutter);
            }
            for column in layout.gutter..width {
                grid.put(row, column, ' ', Tone::Text);
            }
            let line = self.scroll_line + row;
            if line >= self.editor.doc.len_lines() {
                // Past the end of the document the gutter keeps its column, so
                // the strip reads as one thing rather than as a ragged edge.
                continue;
            }
            self.draw_gutter(grid, row, line, layout.gutter, caret_line);
            self.draw_line(grid, row, line, layout, caret, &selection);
            if line == caret_line && caret_col == self.line_length(line) && layout.text_cols > 0 {
                // The caret is past the last character: a block on the space
                // that follows it, which is where the next character goes.
                let column = gutter + caret_display.saturating_sub(self.scroll_col);
                if column < width {
                    grid.put(row, column, ' ', Tone::Caret);
                }
            }
        }
        self.draw_status(grid, status_row, width);
        // The menus last: a dropdown is an overlay, and the bar takes the top
        // row only while it is up. Midnight Commander's bar behaves the same
        // way, which is where a terminal user expects to find it.
        if self.menus.is_open() {
            self.menus.draw(grid);
        }
    }

    /// Puts the line number, right-aligned, at the left of `row`.
    fn draw_gutter(&self, grid: &mut Grid, row: usize, line: usize, gutter: usize, caret: usize) {
        if gutter == 0 {
            return;
        }
        let tone = if line == caret {
            Tone::GutterActive
        } else {
            Tone::Gutter
        };
        let number = format!("{:>width$}", line + 1, width = gutter - 1);
        grid.text(row, 0, &number, tone);
        grid.put(row, gutter - 1, ' ', tone);
    }

    /// Puts one line of the document, from the scroll position, on `row`.
    fn draw_line(
        &self,
        grid: &mut Grid,
        row: usize,
        line: usize,
        layout: Layout,
        caret: usize,
        selection: &std::ops::Range<usize>,
    ) {
        let gutter = layout.gutter;
        let text_cols = layout.text_cols;
        let line_start = self.editor.doc.line_to_char(line);
        let mut display = 0usize;
        for (index, ch) in self.editor.doc.line_chars(line).enumerate() {
            if ch == '\n' || ch == '\r' {
                break;
            }
            let width = ch.width().unwrap_or(1);
            let at = display;
            display = advance(display, ch);
            if at < self.scroll_col {
                // Scrolled off to the left, but the caret can still be in it.
                continue;
            }
            let column = gutter + at - self.scroll_col;
            if column >= gutter + text_cols {
                break;
            }
            let char_index = line_start + index;
            let tone = if char_index == caret {
                Tone::Caret
            } else if selection.contains(&char_index) {
                Tone::Selection
            } else {
                palette::tone(self.editor.doc.style_at(char_index))
            };
            if ch == '\t' {
                // A tab is drawn as the spaces it stands for, in the tone of the
                // text around it: a terminal has no tab stops to hand the job to.
                for step in 0..(display - at) {
                    let column = gutter + at + step - self.scroll_col;
                    if column < gutter + text_cols {
                        grid.put(row, column, ' ', tone);
                    }
                }
                continue;
            }
            if width == 0 {
                // A combining mark: this shell draws the base character only.
                continue;
            }
            grid.put(row, column, ch, tone);
        }
    }

    /// How many characters `line` holds, without its terminator.
    fn line_length(&self, line: usize) -> usize {
        self.editor.doc.line_len(line)
    }

    /// The bar at the bottom: where the file is, and where the caret is in it.
    fn draw_status(&self, grid: &mut Grid, row: usize, width: usize) {
        for column in 0..width {
            grid.put(row, column, ' ', Tone::StatusBar);
        }
        let mut left = self
            .editor
            .doc
            .path()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "[No Name]".to_string());
        if self.editor.doc.is_dirty() {
            left.push_str(" [+]");
        }
        let tone = match &self.message {
            Some(message) => {
                left = message.text.clone();
                if message.alert {
                    Tone::Alert
                } else {
                    Tone::StatusBar
                }
            }
            None => Tone::StatusBar,
        };
        let line = self.editor.caret_line() + 1;
        let column = self.editor.caret_column() + 1;
        let right = format!(
            "Ln {}, Col {}  {}  {} ",
            line,
            column,
            self.editor.doc.encoding().name(),
            self.editor.doc.eol().name()
        );
        let start = width.saturating_sub(right.chars().count());
        // The half on the left gives way to the half on the right: a narrow
        // window cuts the path rather than the line and column, which are the
        // two numbers a reader is looking at while typing.
        if left.chars().count() >= start {
            left = left.chars().take(start.saturating_sub(1)).collect();
        }
        grid.text(row, 0, &left, tone);
        grid.text(row, start, &right, Tone::StatusBar);
    }
}

/// Where a character leaves the display column.
fn advance(column: usize, ch: char) -> usize {
    if ch == '\t' {
        (column / TAB + 1) * TAB
    } else {
        column + ch.width().unwrap_or(1)
    }
}

/// The lexer for a path, chosen the way the windowed editor chooses one.
fn lexer_for(path: Option<&Path>) -> Box<dyn Lexer> {
    let index = path
        .and_then(|path| path.extension())
        .and_then(|ext| ext.to_str())
        .and_then(nn_core::lang_for_extension);
    match index.and_then(|index| nn_core::LANGS.get(index)) {
        Some(spec) => Box::new(ConfigLexer::new(spec)),
        // An unknown extension is highlighted as code, which is what the
        // windowed editor does with it too.
        None => Box::new(CLikeLexer),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::Frame;

    fn app(text: &str) -> App {
        App::with_document(Document::from_text(text))
    }

    fn drawn(app: &mut App, width: usize, height: usize) -> Vec<String> {
        let mut grid = Grid::new(width, height);
        app.draw(&mut grid);
        (0..height)
            .map(|row| {
                (0..width)
                    .map(|column| grid.get(row, column).ch)
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn typing_lands_in_the_document_and_on_the_screen() {
        let mut app = app("");
        for ch in "hello".chars() {
            app.handle(Key::Char(ch));
        }
        assert_eq!(app.editor.doc.text(), "hello");
        let rows = drawn(&mut app, 60, 4);
        assert!(rows[0].contains("hello"), "{:?}", rows);
        // The line number is there, in the gutter.
        assert!(rows[0].starts_with("1 "), "{:?}", rows);
        assert!(rows[0].trim_end().ends_with("hello"), "{:?}", rows);
    }

    #[test]
    fn the_screen_scrolls_to_keep_the_caret_visible() {
        let text: String = (0..200).map(|n| format!("line {n}\n")).collect();
        let mut app = app(&text);
        for _ in 0..50 {
            app.handle(Key::Down);
        }
        let rows = drawn(&mut app, 60, 6);
        // Five rows of text and the bar; the caret's line is the last of them.
        assert!(rows[4].trim_start().starts_with("51 line 50"), "{:?}", rows);
        assert!(rows[0].trim_start().starts_with("47 line 46"), "{:?}", rows);
        assert!(rows[5].contains("Ln 51"), "{:?}", rows);
    }

    #[test]
    fn a_tab_takes_the_next_stop_and_a_wide_character_two_cells() {
        let mut app = App::with_document(Document::from_text("\tx中y\n"));
        let mut grid = Grid::new(20, 3);
        app.draw(&mut grid);
        let row: Vec<char> = (0..20).map(|column| grid.get(0, column).ch).collect();
        // The gutter is two columns, the tab reaches the fourth stop, so `x` is
        // the sixth character of the row.
        assert_eq!(row[..6].iter().collect::<String>(), "1     ", "{row:?}");
        assert_eq!(row[6], 'x', "{row:?}");
        // A wide character covers two cells: the glyph and its marker.
        assert_eq!(row[7], '中', "{row:?}");
        assert_eq!(row[8], '\0', "{row:?}");
        assert_eq!(row[9], 'y', "{row:?}");
    }

    #[test]
    fn return_keeps_the_indent_of_the_line_above() {
        let mut app = app("    let x = 1;\n");
        app.handle(Key::LineEnd);
        app.handle(Key::Enter);
        app.handle(Key::Char('a'));
        assert_eq!(app.editor.doc.text(), "    let x = 1;\n    a\n");
    }

    #[test]
    fn unsaved_changes_take_two_presses_of_ctrl_q() {
        let mut app = app("text");
        app.handle(Key::Char('!'));
        assert!(app.editor.doc.is_dirty());
        app.handle(Key::Ctrl('q'));
        assert!(!app.take_quit(), "the first press left without asking");
        assert!(app.message().unwrap_or_default().contains("unsaved"));
        app.handle(Key::Ctrl('q'));
        assert!(app.take_quit(), "the second press did not leave");
    }

    #[test]
    fn saving_writes_the_file_and_clears_the_mark() {
        let path = std::env::temp_dir().join(format!("npp-tui-save-{}.txt", std::process::id()));
        std::fs::write(&path, "one\n").expect("write");
        let mut app = App::open(Some(&path)).expect("open");
        app.handle(Key::LineEnd);
        app.handle(Key::Char('!'));
        assert!(app.editor.doc.is_dirty());
        app.handle(Key::Ctrl('s'));
        assert!(!app.editor.doc.is_dirty(), "{}", app.message().unwrap_or(""));
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "one!\n");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_buffer_with_no_file_refuses_to_save_and_says_why() {
        let mut app = app("text");
        app.handle(Key::Ctrl('s'));
        assert!(app.message().unwrap_or_default().contains("no file name"));
    }

    #[test]
    fn the_status_bar_counts_the_caret_from_one() {
        let mut app = app("one\ntwo\n");
        app.handle(Key::Down);
        app.handle(Key::Char('x'));
        let rows = drawn(&mut app, 60, 4);
        assert!(rows[3].contains("Ln 2, Col 2"), "{:?}", rows);
        assert!(rows[3].trim_start().starts_with("[No Name] [+]"), "{:?}", rows);
    }

    #[test]
    fn a_keystroke_costs_a_fraction_of_a_screen_and_of_a_millisecond() {
        // The number this shell lives or dies by. A terminal is paid for in
        // bytes, and a frame that repaints the screen to move a caret is what
        // makes a TUI feel slow over ssh. The bounds are loose enough to survive
        // a busy machine; the printed numbers are the ones to compare across
        // passes.
        let text: String = (0..500).map(|n| format!("let value_{n} = {n};\n")).collect();
        let mut app = app(&text);
        let mut frame = Frame::new(120, 40);
        app.draw(frame.grid());
        let paint = frame.render().len();

        let mut bytes = Vec::new();
        let mut nanos = Vec::new();
        for _ in 0..100 {
            app.handle(Key::Char('x'));
            let start = std::time::Instant::now();
            app.draw(frame.grid());
            let written = frame.render().len();
            nanos.push(start.elapsed().as_nanos() as u64);
            bytes.push(written);
        }
        bytes.sort_unstable();
        nanos.sort_unstable();
        let median = bytes[bytes.len() / 2];
        let micros = nanos[nanos.len() / 2] as f64 / 1000.0;
        println!("first paint {paint} bytes; a keystroke {median} bytes in {micros:.1} us");

        assert!(paint > 5_000, "the first paint was only {paint} bytes");
        assert!(
            median * 10 < paint,
            "a keystroke cost {median} bytes of a {paint}-byte screen"
        );
        assert!(median < 600, "a keystroke cost {median} bytes");
        assert!(micros < 5_000.0, "a keystroke took {micros} us");
    }

    #[test]
    fn a_keyword_is_drawn_in_its_own_tone() {
        let mut app = app("fn main() {}\n");
        // The caret is on the `f` when the document opens, and the caret is
        // drawn over the character it stands on, so move it out of the way first.
        app.handle(Key::LineEnd);
        let mut grid = Grid::new(40, 3);
        app.draw(&mut grid);
        // `fn` starts after the gutter, which is two columns for two lines.
        assert_eq!(grid.get(0, 2).ch, 'f');
        assert_eq!(grid.get(0, 2).tone, Tone::Keyword);
        assert_eq!(grid.get(0, 3).tone, Tone::Keyword);
        // The space after the word, then the name of the function, then a
        // bracket: three tones in seven columns, which is what a line of code
        // costs to draw.
        assert_eq!(grid.get(0, 4).tone, Tone::Text);
        assert_eq!(grid.get(0, 5).tone, Tone::Function);
        assert_eq!(grid.get(0, 9).tone, Tone::Operator);
    }
}
