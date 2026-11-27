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
use nn_shared::i18n::Catalog;
use nn_shared::keymap::{Chord, KeyName, Keymap};
use nn_shared::settings::Settings;
use unicode_width::UnicodeWidthChar;

use crate::find::Find;
use crate::keys::Key;
use crate::menus::Menus;
use crate::palette::{self, Palette, Tone};
use crate::prefs::Prefs;
use crate::prompt::{Kind as PromptKind, Prompt};
use crate::screen::Grid;

/// How the window is divided up, worked out once per frame.
///
/// The bar is the last row, and the find bar -- when it is up -- takes the one
/// above it. The row the text starts losing rows at is therefore one field: the
/// number of rows of text is the row the first strip is drawn on.
#[derive(Debug, Clone, Copy)]
struct Layout {
    /// Columns the line numbers take.
    gutter: usize,
    /// Columns left for the text.
    text_cols: usize,
    /// The first row of the strip above the status line, while anything is up:
    /// the prompt, or the find bar with its replacement row.
    bar_row: Option<usize>,
    /// The row the bar is drawn on.
    status_row: usize,
}

impl Layout {
    /// The rows the document has, which is every row above the first strip.
    fn text_rows(self) -> usize {
        self.bar_row.unwrap_or(self.status_row)
    }
}

/// Where a cell of a line landed, once the wrapping and the scrolling had their
/// say about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CellAt {
    /// Drawn at this row and column.
    Put(usize, usize),
    /// Off the top of the screen, or scrolled off to the left.
    Hidden,
    /// Past the bottom of the screen, or past its right-hand edge: so is
    /// everything after it on this line.
    Past,
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
    ///
    /// Always zero while wrapping is on: a wrapped line has no second half off
    /// to the right, so there is nothing to scroll towards.
    scroll_col: usize,
    /// With wrapping on, the segments of [`App::scroll_line`] above the screen.
    ///
    /// A line taller than the window is the one case the line number cannot
    /// describe the top of the screen on its own, and the caret can sit deeper
    /// in such a line than the window is tall.
    scroll_seg: usize,
    /// The first line the last frame did not have room for.
    ///
    /// The next frame uses it to decide how far to follow a caret that has left
    /// the screen, which is a question the line number alone cannot answer once
    /// one line can take several rows.
    visible_until: usize,
    /// The columns the text area had in the last frame.
    ///
    /// Kept because a vertical move is a move between rows while wrapping is on,
    /// and the rows depend on the column count -- which a keystroke needs before
    /// there is a frame to ask.
    view_cols: usize,
    /// The rows of text the last frame had, which is what a page is.
    view_rows: usize,
    /// The column within a wrapped row that a run of vertical moves aims at.
    ///
    /// The core remembers a character column for the moves it makes, which is the
    /// right unit while a line is a row. With wrapping on the column that has to
    /// survive is the position inside the row -- `desired_col` in the core's
    /// sense would be a different place on every row -- so the shell keeps its
    /// own, and keeps it only across a run of vertical moves.
    desired_row: Option<usize>,
    /// The bar and its dropdowns, with the shared tree behind them.
    menus: Menus,
    /// The preferences dialog, and the settings it edits.
    ///
    /// Loading them at startup rather than when the dialog opens is what makes a
    /// terminal started twice behave the same way: what the dialog wrote last
    /// time is already in force when the first frame is drawn.
    prefs: Prefs,
    /// The find bar and its pattern, which outlives the bar so `F3` works after
    /// it closes.
    find: Find,
    /// The one-line prompt, when a command needs a line typed before it runs.
    prompt: Prompt,
    settings: Settings,
    /// The chords, read out of the settings.
    ///
    /// A field rather than a lookup into `settings` because a keystroke asks for
    /// an action on every press and a linear scan of a hundred bindings per
    /// press is work the reader would feel as latency.
    keymap: Keymap,
    /// The escapes the screen is painted with, built from the theme the settings
    /// name.
    palette: Palette,
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
        let mut app = App {
            editor,
            highlighter,
            scroll_line: 0,
            scroll_col: 0,
            scroll_seg: 0,
            visible_until: 0,
            view_cols: 0,
            view_rows: 0,
            desired_row: None,
            menus: Menus::new(&Keymap::default()),
            prefs: Prefs::new(),
            find: Find::new(),
            prompt: Prompt::new(),
            settings: Settings::load(),
            keymap: Keymap::default(),
            palette: Palette::of(&nn_shared::theme::Theme::DARK),
            message: None,
            quit_armed: false,
            quit: false,
        };
        // Before the first frame, so a terminal started twice reads the same way,
        // in the same language and the same colours.
        app.apply_settings();
        app
    }

    /// The escapes the screen is painted with.
    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    /// The settings, for a test that wants to set one before the first frame.
    #[cfg(test)]
    pub fn settings_mut(&mut self) -> &mut Settings {
        &mut self.settings
    }

    /// Puts the settings that change how the program looks into force.
    ///
    /// Everything else a setting can change -- a backup, an autosave, a run
    /// command -- happens where it is used; these three are the ones the screen
    /// is drawn with, so they are built once here rather than looked up per cell.
    fn apply_settings(&mut self) {
        self.apply_theme();
        self.apply_language();
        self.keymap = self.settings.keymap.clone();
    }

    /// Resolves the theme the settings name into the escapes the screen uses.
    ///
    /// `None` for the desktop's light or dark preference, because a terminal has
    /// no such setting to ask: it is told its colours by the terminal, which
    /// reports none back. `System` therefore means dark here, which is what the
    /// terminals that draw a light page are in the minority about.
    fn apply_theme(&mut self) {
        let theme = nn_shared::theme::effective_theme(
            nn_shared::catalogue::theme_of(self.settings.theme, None),
            &self.settings.styles,
        );
        self.palette = Palette::of(&theme);
    }

    /// Loads the localization the settings name, and the list the dialog offers.
    ///
    /// A file that will not parse leaves the bar and the dialog in English and
    /// says so: refusing to start over a translation would cost the user their
    /// editor for a cosmetic fault. The window answers the same way for the same
    /// reason.
    fn apply_language(&mut self) {
        let dir = nn_core::paths::config_file("localization");
        let installed = Catalog::languages(dir.as_deref());
        let catalog = match self.settings.language.as_deref() {
            Some(file) => match Catalog::load(dir.as_deref(), file) {
                Ok(catalog) => Some(catalog),
                Err(error) => {
                    self.say(&format!("language: {error}"), true);
                    None
                }
            },
            None => None,
        };
        self.prefs.set_languages(&installed);
        self.prefs.set_catalog(catalog.clone());
        self.find.set_catalog(catalog.clone());
        self.prompt.set_catalog(catalog.clone());
        self.menus.set_catalog(catalog);
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
        // The preferences dialog is modal in both front-ends: while it is up,
        // every key is either one it understands or is dropped.
        if self.prefs.is_open() {
            self.prefs.handle(key, &mut self.settings);
            if !self.prefs.is_open() {
                // Closing writes: the dialog is the only thing that changes
                // settings, so leaving is the moment they are worth keeping. The
                // language is re-read here rather than live, because a list that
                // retranslates itself under the user's finger moves the entry
                // they are stepping towards.
                self.settings.save();
                self.apply_settings();
                self.say("preferences saved", false);
            }
            return;
        }
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
        // The prompt is modal, as the window's is: a letter belongs to the field
        // rather than to the document behind it, and a key the field has no use
        // for is dropped rather than handed on.
        if self.prompt.is_open() {
            self.prompt.handle(key, &mut self.editor);
            return;
        }
        self.message = None;
        // The remembered row survives a run of vertical moves and nothing else.
        if !matches!(
            key,
            Key::Up | Key::Down | Key::ExtendUp | Key::ExtendDown | Key::PageUp | Key::PageDown
        ) {
            self.desired_row = None;
        }
        if self.move_by_rows(key) {
            return;
        }
        // The find bar owns the keys it understands while it is up and hands the
        // rest back, so `Ctrl+S` saves and `Ctrl+Q` quits with a pattern half
        // typed: a bar that swallowed every chord would be a mode, and this one
        // is a row.
        if self.find.is_open() && self.find.handle(key, &mut self.editor) {
            // The bar replaces through the editor rather than through the shell,
            // so this is the only place that hears the document changed.
            if self.find.take_replaced() {
                self.refresh_all();
            }
            return;
        }
        // Ctrl+Q leaves, and it has to: the window binds `exit` to Ctrl+Shift+Q,
        // which a terminal cannot send -- Ctrl and Shift and a letter is not one
        // of the bytes a keyboard puts on the wire. A shell that could only be
        // left with a chord the terminal cannot produce would have no way out,
        // so the key every terminal editor uses for this is the shell's own,
        // beside F9 for the bar.
        if key == Key::Ctrl('q') {
            self.ask_to_quit();
            return;
        }
        // A chord the settings bind is a command, and the command table is the
        // shared one: `Ctrl+D` duplicates a line here because it is
        // `Action::DuplicateLine` there, and a user who rebinds it in the
        // preferences gets the new key in both front-ends.
        if let Some(action) = self.chord_action(key) {
            self.run(action);
            return;
        }
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
            Key::Escape => {
                self.quit_armed = false;
                self.editor.clear_extra_carets();
            }
            // Every other control chord and function key is either bound in the
            // keymap, which was asked above, or bound to nothing. A chord bound
            // to nothing does nothing: typing the control character it stands
            // for would be the one answer nobody wants.
            Key::Ctrl(_) => {}
            Key::Function(_) => {}
        }
    }

    /// The action a keystroke is bound to, if it is bound to one.
    ///
    /// Only the chords and the function keys are asked about. A plain character
    /// is text, and the arrows, Home, End and the paging keys are navigation
    /// this shell answers itself -- a binding on one of those would take typing
    /// away from a user who had bound it by accident.
    fn chord_action(&self, key: Key) -> Option<Action> {
        let chord = match key {
            Key::Ctrl(ch) => Chord::from_char(ch, true, false, false),
            Key::Function(number) => Chord {
                ctrl: false,
                shift: false,
                alt: false,
                key: KeyName::Function(number),
            },
            _ => return None,
        };
        self.keymap.lookup(&chord)
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
            Action::SelectLine => self.editor.select_line(),
            Action::DuplicateLine => self.edit(|ed| ed.duplicate_lines()),
            Action::MoveLineUp => self.edit(|ed| ed.move_lines(true)),
            Action::MoveLineDown => self.edit(|ed| ed.move_lines(false)),
            Action::DeleteLine => self.edit(|ed| ed.delete_lines()),
            Action::Preferences => self.prefs.open(),
            Action::ToggleWrap => self.toggle_wrap(),
            Action::Find => self.find.open(&self.editor),
            Action::Replace => self.find.open_replace(&self.editor),
            Action::GotoLine => self.prompt.open(PromptKind::GotoLine),
            Action::FindNext => self.find.step(&mut self.editor, true),
            Action::FindPrev => self.find.step(&mut self.editor, false),
            Action::Exit => self.ask_to_quit(),
            // The rest of the editing commands are one table over in the engine:
            // upper case, indent, sort, comment, and thirty more. They take the
            // selected lines as text and give them back changed, so the shell
            // answers them with one arm.
            other if other.is_text_command() => self.text_command(other),
            other => {
                let label = other.label();
                self.say(&format!("{label}: not in the terminal yet"), true);
            }
        }
    }

    /// Turns wrapping on or off, and says which it now is.
    ///
    /// The setting is written as well as applied, because the terminal has no
    /// other way to remember it: closing the program is the only thing that
    /// happens between one run and the next. The scroll is put back to the start
    /// of the caret's line, which is where a reader who has just changed the
    /// shape of every row expects to find themself.
    fn toggle_wrap(&mut self) {
        self.settings.wrap = !self.settings.wrap;
        self.settings.save();
        self.scroll_col = 0;
        self.scroll_seg = 0;
        self.visible_until = 0;
        self.scroll_line = self.editor.caret_line();
        self.say(
            if self.settings.wrap {
                "long lines wrap"
            } else {
                "long lines are cut at the edge"
            },
            false,
        );
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

    /// Runs an edit that can have changed any line, and re-lexes what it did.
    ///
    /// The closure answers whether it changed anything, which is what the engine
    /// returns: an edit that was a no-op -- indent on a line that is already
    /// indented to the stop -- costs no undo step and no refresh.
    fn edit(&mut self, edit: impl FnOnce(&mut Editor) -> bool) {
        if edit(&mut self.editor) {
            self.refresh_all();
        }
    }

    /// Runs one of the text commands the engine owns over the selected lines.
    ///
    /// The options are the ones the window passes: the tab width from the
    /// settings, a fresh seed for the two shuffling commands, and the sort's own
    /// direction. The comment marker stays the default `//` here as it does
    /// there -- a per-language marker is a pass of its own, and the two
    /// front-ends should answer the same command the same way in the meantime.
    fn text_command(&mut self, action: Action) {
        let name = action.name();
        let options = nn_core::ops::Options {
            width: usize::from(self.settings.tab_width),
            seed: nn_core::now_secs() as u64,
            sort: match action {
                Action::SortLines(sort) => sort.options(),
                _ => Default::default(),
            },
            ..Default::default()
        };
        self.edit(|editor| {
            editor.transform_lines(|text| {
                nn_core::ops::run(name, text, &options).unwrap_or_else(|_| text.to_string())
            })
        });
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

    /// Moves the caret by rows instead of by lines, while wrapping is on.
    ///
    /// Answers whether it took the key. With wrapping off, or with several
    /// carets, the core's own motion is the right one and this says so by
    /// declining: a stack of carets walks a block in parallel there, and a row
    /// motion written for one caret would have to answer what happens to the
    /// others.
    ///
    /// A row is `view_cols` cells of a line, so a line taller than the window is
    /// walked a row at a time, and stepping past its last row arrives on the
    /// first row of the line below. What survives a run of these moves is the
    /// column *inside* the row, which is the only column that means the same
    /// thing on the next row: the remembered one is clamped to a short row and
    /// not lost, so passing through a short line does not pull the caret left for
    /// good.
    fn move_by_rows(&mut self, key: Key) -> bool {
        // The find bar takes Up and Down as "the match before" and "the match
        // after", and it is asked *after* this move, so a bar that is up is a
        // reason to decline rather than a reason to move the caret by a row.
        if self.find.is_open()
            || !self.settings.wrap
            || self.view_cols == 0
            || self.editor.selections.len() > 1
        {
            return false;
        }
        let (delta, extend) = match key {
            Key::Up => (-1isize, false),
            Key::Down => (1, false),
            Key::ExtendUp => (-1, true),
            Key::ExtendDown => (1, true),
            Key::PageUp => (-(self.view_rows as isize), false),
            Key::PageDown => (self.view_rows as isize, false),
            _ => return false,
        };
        let cols = self.view_cols;
        let line = self.editor.caret_line();
        let display = self.display_column(line, self.editor.caret_column());
        let row_col = *self.desired_row.get_or_insert(display % cols);
        let mut at = (line, display / cols);
        // How many rows the line the caret is on takes, counted once per line it
        // crosses: a page walks every row of a tall line, and counting a line of
        // a hundred thousand characters again for each of those rows is the kind
        // of cost a keypress should not have.
        let mut rows = self.row_count(at.0);
        for _ in 0..delta.unsigned_abs() {
            match self.row_neighbour(at.0, at.1, rows, delta > 0) {
                Some(next) => {
                    if next.0 != at.0 {
                        rows = self.row_count(next.0);
                    }
                    at = next;
                }
                // The first or the last row of the document: the move stays
                // where it is rather than wrapping round.
                None => break,
            }
        }
        let width = self.display_column(at.0, self.line_length(at.0));
        let target = (at.1 * cols + row_col).min(width);
        let position = self.editor.doc.line_to_char(at.0) + self.char_at_display(at.0, target);
        if extend {
            self.editor.selections.primary_mut().extend_to(position);
        } else {
            self.editor.selections.set_caret(position);
        }
        true
    }

    /// The row above or below `segment` of a line of `rows` rows, or `None` at
    /// the document's ends.
    fn row_neighbour(
        &self,
        line: usize,
        segment: usize,
        rows: usize,
        down: bool,
    ) -> Option<(usize, usize)> {
        if down {
            if segment + 1 < rows {
                Some((line, segment + 1))
            } else {
                (line + 1 < self.editor.doc.len_lines()).then_some((line + 1, 0))
            }
        } else if segment > 0 {
            Some((line, segment - 1))
        } else if line > 0 {
            Some((line - 1, self.row_count(line - 1) - 1))
        } else {
            None
        }
    }

    /// How many rows `line` takes once wrapped, which is at least one.
    ///
    /// Counted in cells, not characters: a tab stands for the stop it jumps to
    /// and a wide glyph for two columns, which is what the drawing does and
    /// therefore what the rows are.
    fn row_count(&self, line: usize) -> usize {
        let width = self.display_column(line, self.line_length(line));
        width.div_ceil(self.view_cols.max(1)).max(1)
    }

    /// The character of `line` that stands at display column `target`, or the
    /// one past its last character.
    fn char_at_display(&self, line: usize, target: usize) -> usize {
        let mut display = 0usize;
        for (index, ch) in self.editor.doc.line_chars(line).enumerate() {
            if ch == '\n' || ch == '\r' || display >= target {
                return index;
            }
            display = advance(display, ch, self.settings.tab_width);
        }
        self.line_length(line)
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
            display = advance(display, ch, self.settings.tab_width);
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
            bar_row: {
                // The strip above the status row belongs to one widget at a time:
                // the prompt while it is up, otherwise the find bar, which takes
                // one row for the pattern and two once the replacement is up.
                let rows = if self.prompt.is_open() {
                    1
                } else {
                    self.find.rows()
                };
                (rows > 0).then(|| height.saturating_sub(1 + rows))
            },
            status_row: height - 1,
        };
        let text_rows = layout.text_rows();
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

        let wrap = self.settings.wrap && layout.text_cols > 0;
        if !wrap {
            self.scroll_seg = 0;
        }
        if wrap {
            // Wrapping means the caret is somewhere on a line that may take
            // several rows, so the question is not which column is on screen but
            // which rows the last frame had.
            self.scroll_col = 0;
            self.reveal_wrapped(caret_line, caret_display, layout);
        } else {
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
            self.visible_until = self.scroll_line + text_rows;
        }

        self.view_cols = layout.text_cols;
        self.view_rows = text_rows;
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
        }

        // The lines, one after another, each taking as many rows as it needs:
        // one while wrapping is off, a screenful or more for a long line while
        // it is on.
        let mut row = 0usize;
        let mut line = self.scroll_line;
        let mut skip = if wrap { self.scroll_seg } else { 0 };
        while row < text_rows && line < self.editor.doc.len_lines() {
            if skip == 0 {
                // A line wears its number on its first row, and a continuation
                // row has none: the strip reads as one line rather than as
                // several with the same number.
                self.draw_gutter(grid, row, line, layout.gutter, caret_line);
            }
            row += self.draw_line(grid, row, line, layout, caret, &selection, skip, wrap);
            line += 1;
            skip = 0;
        }
        self.visible_until = line;
        if let Some(row) = layout.bar_row {
            if self.prompt.is_open() {
                self.prompt.draw(grid, row, width);
            } else {
                self.find.draw(grid, row, width);
            }
        }
        self.draw_status(grid, status_row, width);
        // The menus last: a dropdown is an overlay, and the bar takes the top
        // row only while it is up. Midnight Commander's bar behaves the same
        // way, which is where a terminal user expects to find it.
        if self.menus.is_open() {
            self.menus.draw(grid);
        }
        // And the preferences dialog over everything: it is the most modal thing
        // the shell has, and a dropdown cannot be open at the same time.
        self.prefs.draw(grid, &self.settings);
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

    /// Where the caret has to be for the view to hold it, while wrapping is on.
    ///
    /// A wrapped line is several rows of one line, so `scroll_line` alone cannot
    /// say whether the caret is on the screen: `scroll_seg` names the segments of
    /// the top line that are above it, and `visible_until` -- the first line the
    /// last frame had no room for -- says whether the caret's line was one of
    /// them at all. The near case moves the view by one line, so that a caret
    /// walked off the bottom of the screen scrolls it as it always did; the far
    /// case, a jump or a first frame, puts the caret's line at the top.
    fn reveal_wrapped(&mut self, caret_line: usize, caret_display: usize, layout: Layout) {
        let rows = layout.text_rows();
        let columns = layout.text_cols.max(1);
        let segment = caret_display / columns;
        if caret_line < self.scroll_line {
            self.scroll_line = caret_line;
            self.scroll_seg = 0;
        }
        if self.visible_until <= self.scroll_line {
            // Nothing is known about what fits -- a first frame, or a window that
            // has just changed size -- so the caret's line is given the benefit.
            self.visible_until = self.scroll_line + rows;
        }
        if caret_line >= self.visible_until {
            self.scroll_line = if rows > 0 && caret_line < self.visible_until + rows {
                self.visible_until
            } else {
                (caret_line + 1).saturating_sub(rows)
            };
            self.scroll_seg = 0;
        }
        if caret_line == self.scroll_line && rows > 0 {
            if segment < self.scroll_seg {
                self.scroll_seg = segment;
            }
            if segment >= self.scroll_seg + rows {
                self.scroll_seg = segment + 1 - rows;
            }
        }
    }

    /// Where the cell at display column `display` of a line starting on `row`
    /// goes.
    fn cell_of(
        &self,
        row: usize,
        display: usize,
        layout: Layout,
        wrap: bool,
        skip: usize,
    ) -> CellAt {
        if layout.text_cols == 0 {
            return CellAt::Past;
        }
        if wrap {
            let segment = display / layout.text_cols;
            if segment < skip {
                // Above the top of the screen: the caret can still be in it.
                return CellAt::Hidden;
            }
            let at_row = row + segment - skip;
            if at_row >= layout.text_rows() {
                return CellAt::Past;
            }
            CellAt::Put(at_row, layout.gutter + display % layout.text_cols)
        } else {
            if display < self.scroll_col {
                return CellAt::Hidden;
            }
            let column = layout.gutter + display - self.scroll_col;
            if column >= layout.gutter + layout.text_cols {
                return CellAt::Past;
            }
            CellAt::Put(row, column)
        }
    }

    /// Puts the line on the screen, from its first visible row, and answers how
    /// many rows it took.
    ///
    /// With wrapping on a row holds a segment of the line, and a line too long
    /// for the rows that are left is drawn as far as they go: the rest of it is
    /// below the screen and the frame after this one starts where this one ran
    /// out.
    #[allow(clippy::too_many_arguments)]
    fn draw_line(
        &self,
        grid: &mut Grid,
        row: usize,
        line: usize,
        layout: Layout,
        caret: usize,
        selection: &std::ops::Range<usize>,
        skip: usize,
        wrap: bool,
    ) -> usize {
        let line_start = self.editor.doc.line_to_char(line);
        let mut display = 0usize;
        let mut used = 1usize;
        for (index, ch) in self.editor.doc.line_chars(line).enumerate() {
            if ch == '\n' || ch == '\r' {
                break;
            }
            let width = ch.width().unwrap_or(1);
            let at = display;
            display = advance(display, ch, self.settings.tab_width);
            let char_index = line_start + index;
            let tone = if char_index == caret {
                Tone::Caret
            } else if selection.contains(&char_index) {
                Tone::Selection
            } else {
                palette::tone(self.editor.doc.style_at(char_index))
            };
            // A tab is a run of cells rather than a character, and a run can
            // straddle the column a wrapped row ends on: each cell is placed on
            // its own. Any other character is one cell -- two for a wide glyph,
            // which the grid writes itself.
            let (from, cells, drawn) = match ch {
                '\t' => (at, display.saturating_sub(at).max(1), ' '),
                _ if width == 0 => {
                    // A combining mark: this shell draws the base character only.
                    continue;
                }
                _ => (at, 1, ch),
            };
            for step in 0..cells {
                match self.cell_of(row, from + step, layout, wrap, skip) {
                    CellAt::Hidden => {}
                    CellAt::Put(at_row, column) => {
                        grid.put(at_row, column, drawn, tone);
                        used = used.max(at_row - row + 1);
                    }
                    CellAt::Past => {
                        // Nothing after this fits. With wrapping on that is the
                        // bottom of the screen, and every row left belongs to
                        // this line; with it off it is the right-hand edge, and
                        // the line is as tall as it ever was.
                        return if wrap {
                            layout.text_rows().saturating_sub(row).max(1)
                        } else {
                            used
                        };
                    }
                }
            }
        }
        // The caret past the last character takes a cell of its own, and a line
        // that fills its rows exactly gets one more for it: the caret is a thing
        // the line is a cell wider for, not an ornament on the last character.
        if caret == line_start + self.line_length(line) {
            match self.cell_of(row, display, layout, wrap, skip) {
                CellAt::Put(at_row, column) => {
                    grid.put(at_row, column, ' ', Tone::Caret);
                    used = used.max(at_row - row + 1);
                }
                CellAt::Hidden => {}
                CellAt::Past => {
                    return if wrap {
                        layout.text_rows().saturating_sub(row).max(1)
                    } else {
                        used
                    };
                }
            }
        }
        used
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
///
/// A tab is a jump to the next tab stop rather than a character, so the width it
/// jumps by is the one the preferences dialog holds -- the same setting the
/// windowed editor passes to its own view, and the same default of four.
fn advance(column: usize, ch: char, tab_width: u16) -> usize {
    if ch == '\t' {
        let stop = usize::from(tab_width).max(1);
        (column / stop + 1) * stop
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
    use nn_shared::theme::ThemeChoice;

    /// An app on `text`, with the default settings rather than the ones on the
    /// machine that runs the tests.
    ///
    /// The settings file of whoever runs the suite decides the keymap and the
    /// theme, and a test that read them would pass where it was written and fail
    /// on the next machine. `Ctrl+Q` in this crate's tests means what it means in
    /// a fresh installation.
    fn app(text: &str) -> App {
        let mut app = App::with_document(Document::from_text(text));
        *app.settings_mut() = Settings::default();
        app.apply_settings();
        app
    }

    /// Where the caret is drawn, if it is drawn at all.
    fn caret_cell(grid: &Grid) -> Option<(usize, usize)> {
        for row in 0..grid.height() {
            for column in 0..grid.width() {
                if grid.get(row, column).tone == Tone::Caret {
                    return Some((row, column));
                }
            }
        }
        None
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
    fn a_long_line_wraps_onto_the_row_below() {
        // Twelve columns and a gutter of two leave ten of text, so the sixteen
        // characters of the first line take two rows -- and the second of them
        // carries no line number, because it is not a line.
        let mut app = app("0123456789ABCDEF\nab\n");
        app.settings_mut().wrap = true;
        let rows = drawn(&mut app, 12, 5);
        assert_eq!(rows[0], "1 0123456789", "{:?}", rows);
        assert_eq!(rows[1], "  ABCDEF    ", "{:?}", rows);
        assert_eq!(rows[2], "2 ab        ", "{:?}", rows);
        // With wrapping off the same line is cut at the edge instead, and the
        // second line of the document starts one row higher.
        app.settings_mut().wrap = false;
        let rows = drawn(&mut app, 12, 5);
        assert_eq!(rows[0], "1 0123456789", "{:?}", rows);
        assert_eq!(rows[1], "2 ab        ", "{:?}", rows);
    }

    #[test]
    fn wrapped_rows_carry_the_caret_and_the_tone_of_their_text() {
        let mut app = app("fn main() { let value = 1; }\n");
        app.settings_mut().wrap = true;
        app.editor.goto_position(0, 14);
        let mut grid = Grid::new(12, 4);
        app.draw(&mut grid);
        // The caret is at display column fourteen, which is the fifth cell of
        // the second row: two of gutter and three more.
        assert_eq!(grid.get(0, 2).ch, 'f', "{:?}", (0, 2));
        assert_eq!(grid.get(0, 2).tone, Tone::Keyword);
        assert_eq!(grid.get(1, 6).ch, 't', "{:?}", (1, 6));
        assert_eq!(grid.get(1, 6).tone, Tone::Caret);
        // The row a line continues on is text, not gutter: it starts with the
        // character the previous row ran out of room for, and the cells past the
        // end of it keep the text background.
        assert_eq!(grid.get(1, 2).ch, '{', "{:?}", (1, 2));
        assert_eq!(grid.get(1, 2).tone, Tone::Operator);
        assert_eq!(grid.get(1, 11).tone, Tone::Text);
    }

    #[test]
    fn a_line_taller_than_the_window_scrolls_to_hold_the_caret() {
        // Fifty characters on one line and two rows of text: the caret at the
        // end of it is five segments below the start, which no line number can
        // bring onto the screen. The segment offset does.
        let line: String = "abcdefghij".repeat(5);
        let mut app = app(&format!("{line}\n"));
        app.settings_mut().wrap = true;
        app.editor.goto_position(0, line.chars().count());
        let rows = drawn(&mut app, 12, 3);
        // Segments four and five: the last ten characters, then the caret on the
        // blank cell after them.
        assert_eq!(rows[0], "  abcdefghij", "{:?}", rows);
        assert_eq!(&rows[1][..3], "   ", "{:?}", rows);
        let mut grid = Grid::new(12, 3);
        app.draw(&mut grid);
        assert_eq!(grid.get(1, 2).tone, Tone::Caret, "{:?}", rows);
    }

    #[test]
    fn down_walks_the_rows_of_one_wrapped_line() {
        // Twelve columns and a one-line document: a gutter of two, ten columns of
        // text, and twenty-five characters in three rows.
        let mut app = app("0123456789ABCDEFGHIJKLMNO");
        app.settings_mut().wrap = true;
        let mut grid = Grid::new(12, 4);
        app.draw(&mut grid);
        app.handle(Key::Down);
        assert_eq!(app.editor.caret_column(), 10);
        app.handle(Key::Down);
        assert_eq!(app.editor.caret_column(), 20);
        // The third row is five characters long, and the caret arrives at the
        // start of it: a row move keeps the column inside the row, which is zero.
        app.handle(Key::Down);
        assert_eq!(app.editor.caret_column(), 20);
        // Which is also the last row of the document: one more Down stays put.
        app.handle(Key::Down);
        assert_eq!(app.editor.caret_column(), 20);
        // And Up walks back, arriving on the same two columns.
        app.handle(Key::Up);
        assert_eq!(app.editor.caret_column(), 10);
        app.handle(Key::Up);
        assert_eq!(app.editor.caret_column(), 0);
        app.handle(Key::Up);
        assert_eq!(app.editor.caret_column(), 0);
    }

    #[test]
    fn the_column_inside_the_row_survives_a_short_line() {
        let text = "0123456789ABCDEFGHIJKLMNO\nab\n0123456789ABCDEFGHIJKLMNO\n";
        let mut app = app(text);
        app.settings_mut().wrap = true;
        let mut grid = Grid::new(12, 4);
        app.draw(&mut grid);
        app.editor.goto_position(0, 3);
        app.draw(&mut grid);
        // Down the rows of the same line: three more cells along, ten in.
        app.handle(Key::Down);
        assert_eq!((app.editor.caret_line(), app.editor.caret_column()), (0, 13));
        app.handle(Key::Down);
        assert_eq!((app.editor.caret_line(), app.editor.caret_column()), (0, 23));
        // Down onto `ab`: the remembered column is three, the line has two.
        app.handle(Key::Down);
        assert_eq!((app.editor.caret_line(), app.editor.caret_column()), (1, 2));
        // And the memory is not the clamp: the next line gets its own row read
        // from the column that was asked for, not from the two cells it gave.
        app.handle(Key::Down);
        assert_eq!((app.editor.caret_line(), app.editor.caret_column()), (2, 3));
    }

    #[test]
    fn up_from_the_first_row_arrives_on_the_last_row_of_the_line_above() {
        let mut app = app("0123456789ABCDEF\nabcdefghij\n");
        app.settings_mut().wrap = true;
        let mut grid = Grid::new(12, 4);
        app.draw(&mut grid);
        app.editor.goto_position(1, 2);
        app.draw(&mut grid);
        // The line above takes two rows, so Up lands on its second row, two cells
        // in -- not at the end of its first.
        app.handle(Key::Up);
        assert_eq!((app.editor.caret_line(), app.editor.caret_column()), (0, 12));
    }

    #[test]
    fn a_page_moves_a_screenful_of_rows_and_a_tab_covers_its_cells() {
        // Four rows of text, so a page is four rows: three tabs' worth of cells
        // is six, and the document is five lines of four tabs.
        let mut app = app("\t\t\t\t\n\t\t\t\t\n\t\t\t\t\n\t\t\t\t\n\t\t\t\t\n");
        app.settings_mut().wrap = true;
        let mut grid = Grid::new(12, 5);
        app.draw(&mut grid);
        // A tab reaches the next stop of four, so a line of four is sixteen cells
        // and two rows of ten.
        app.handle(Key::PageDown);
        assert_eq!(app.editor.caret_line(), 2, "a page of four rows is two lines");
        app.handle(Key::PageDown);
        assert_eq!(app.editor.caret_line(), 4);
        app.handle(Key::PageUp);
        assert_eq!(app.editor.caret_line(), 2);
    }

    #[test]
    fn with_wrapping_off_a_vertical_move_is_still_a_line() {
        let mut app = app("0123456789ABCDEFGHIJ\nab\n0123456789ABCDEFGHIJ\n");
        let mut grid = Grid::new(12, 4);
        app.draw(&mut grid);
        app.editor.goto_position(0, 3);
        app.draw(&mut grid);
        // The second line holds two characters, the third none of the first
        // line's tail: the core's motion clamps to each line and remembers the
        // column, and nothing here interfered with it.
        app.handle(Key::Down);
        assert_eq!((app.editor.caret_line(), app.editor.caret_column()), (1, 2));
        app.handle(Key::Down);
        assert_eq!((app.editor.caret_line(), app.editor.caret_column()), (2, 3));
    }

    #[test]
    fn walking_down_a_wrapped_document_brings_the_caret_with_it() {
        // Twelve columns leave ten of text, and every line is sixty characters:
        // each one is six rows tall, so the screen -- four rows of text -- does
        // not even hold one whole line. Following the caret through that is the
        // case a line number alone cannot describe.
        let line: String = "x".repeat(60);
        let text: String = (0..30).map(|_| format!("{line}\n")).collect();
        let mut app = app(&text);
        app.settings_mut().wrap = true;
        let mut grid = Grid::new(12, 5);
        app.draw(&mut grid);
        for _ in 0..12 {
            app.handle(Key::Down);
            app.draw(&mut grid);
            assert!(caret_cell(&grid).is_some(), "the caret left the screen");
        }
        // Twelve rows down is one and a half lines of six rows, so the caret is
        // on the second line -- where a line motion would have put it on the
        // thirteenth.
        assert_eq!(app.editor.caret_line(), 1);
        // And the same walk back up returns it to where it started, which is what
        // makes the two motions inverses of one another.
        for _ in 0..12 {
            app.handle(Key::Up);
            app.draw(&mut grid);
            assert!(caret_cell(&grid).is_some(), "the caret left the screen");
        }
        assert_eq!(app.editor.caret(), 0);
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
    fn the_tab_width_is_the_one_the_dialog_holds() {
        let mut app = App::with_document(Document::from_text("\tx\n"));
        app.settings_mut().tab_width = 8;
        let mut grid = Grid::new(20, 3);
        app.draw(&mut grid);
        let row: Vec<char> = (0..20).map(|column| grid.get(0, column).ch).collect();
        // The gutter is `1 ` -- two columns -- and the tab jumps from there to
        // the eighth stop, so `x` stands at index ten.
        let expected: String = std::iter::once('1')
            .chain(std::iter::repeat_n(' ', 9))
            .collect();
        assert_eq!(row[..10].iter().collect::<String>(), expected, "{row:?}");
        assert_eq!(row[10], 'x', "{row:?}");
    }

    #[test]
    fn the_theme_the_settings_name_is_the_one_the_screen_uses() {
        let mut app = app("");
        // Both choices are set rather than assumed: the settings file of whoever
        // runs the tests is not this test's business, and a default read from it
        // would make the assertions depend on the machine.
        app.settings_mut().theme = ThemeChoice::Dark;
        app.apply_settings();
        let dark = app.palette().sgr(Tone::Text).to_string();
        app.settings_mut().theme = ThemeChoice::Light;
        app.apply_settings();
        let light = app.palette().sgr(Tone::Text);
        assert_ne!(dark, light, "the theme changed and the colours did not");
        // And back: the same choice twice is the same escapes.
        app.settings_mut().theme = ThemeChoice::Dark;
        app.apply_settings();
        assert_eq!(app.palette().sgr(Tone::Text), dark);
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
    fn a_chord_runs_the_command_the_settings_bind_it_to() {
        // Ctrl+D is `duplicate_line` in the default keymap and Ctrl+L is
        // `delete_line`; both act on the caret's line when nothing is selected.
        let mut app = app("one\ntwo\n");
        app.handle(Key::Ctrl('d'));
        assert_eq!(app.editor.doc.text(), "one\none\ntwo\n");
        app.editor.goto_line(1);
        app.handle(Key::Ctrl('l'));
        assert_eq!(app.editor.doc.text(), "one\ntwo\n");
    }

    #[test]
    fn a_chord_the_settings_do_not_bind_does_nothing() {
        let mut app = app("one\n");
        app.settings_mut().keymap.unbind(Action::DuplicateLine);
        app.apply_settings();
        app.handle(Key::Ctrl('d'));
        assert_eq!(app.editor.doc.text(), "one\n", "an unbound chord edited the file");
    }

    #[test]
    fn the_text_commands_run_over_the_selected_lines() {
        let mut app = app("b\na\n");
        // A caret alone means "the caret's line", which is what the window does
        // too: these commands are about a selection, and the shell does not
        // invent one for them.
        app.run(Action::SelectAll);
        app.run(Action::UpperCase);
        assert_eq!(app.editor.doc.text(), "B\nA\n");
        // An edit replaces what it was given, so the selection is gone after it:
        // the next command needs one of its own.
        app.run(Action::SelectAll);
        app.run(Action::SortAsc);
        assert_eq!(app.editor.doc.text(), "A\nB\n");
        app.run(Action::SelectAll);
        // The engine indents with a tab, not with `tab_width` spaces: the
        // column a tab stands for is a display question, and the file keeps the
        // character that is one keystroke away from the one it replaces.
        app.run(Action::Indent);
        assert_eq!(app.editor.doc.text(), "\tA\n\tB\n");
        app.run(Action::SelectAll);
        app.run(Action::Unindent);
        assert_eq!(app.editor.doc.text(), "A\nB\n");
    }

    #[test]
    fn the_line_commands_move_and_delete() {
        let mut app = app("one\ntwo\n");
        app.run(Action::MoveLineDown);
        assert_eq!(app.editor.doc.text(), "two\none\n");
        app.editor.goto_line(1);
        app.run(Action::MoveLineUp);
        assert_eq!(app.editor.doc.text(), "one\ntwo\n");
        // The caret is left at the end of the block the move touched, which is
        // the empty line after the last newline; a delete needs it somewhere
        // with a line under it.
        app.editor.goto_line(0);
        app.run(Action::DeleteLine);
        assert_eq!(app.editor.doc.text(), "two\n");
        // The text commands leave a message when they cannot run, and these do
        // not: they answered.
        assert_eq!(app.message(), None);
    }

    #[test]
    fn the_find_bar_takes_a_row_and_walks_the_matches() {
        let mut app = app("one two one\n");
        // Ctrl+F is `find` in the default keymap, as it is in the window.
        app.handle(Key::Ctrl('f'));
        for ch in "one".chars() {
            app.handle(Key::Char(ch));
        }
        // Typing went into the bar, not into the document.
        assert_eq!(app.editor.doc.text(), "one two one\n");
        let rows = drawn(&mut app, 40, 5);
        // The bar is the row above the status bar, and the text lost the row.
        assert!(rows[3].starts_with("Find: one"), "{:?}", rows);
        assert!(rows[3].ends_with("0/2 "), "{:?}", rows);
        assert!(rows[4].contains("Ln 1, Col 1"), "{:?}", rows);
        app.handle(Key::Enter);
        let rows = drawn(&mut app, 40, 5);
        assert!(rows[3].ends_with("1/2 "), "{:?}", rows);
        // Escape closes and leaves the match selected, and the text gets its row
        // back: the row the bar was on is a line of the document again.
        app.handle(Key::Escape);
        let rows = drawn(&mut app, 40, 5);
        assert!(!rows[3].contains("Find:"), "{:?}", rows);
        assert!(rows[4].contains("Ln 1, Col 4"), "{:?}", rows);
    }

    #[test]
    fn f3_walks_the_matches_after_the_bar_has_closed() {
        let mut app = app("one two one\n");
        app.run(Action::Find);
        for ch in "one".chars() {
            app.handle(Key::Char(ch));
        }
        app.handle(Key::Escape);
        assert!(!app.find.is_open());
        // The caret is still where the search left it: at the start, having
        // never stepped anywhere.
        app.handle(Key::Function(3));
        assert_eq!(app.editor.selection_start(), 0);
        app.handle(Key::Function(3));
        assert_eq!(app.editor.selection_start(), 8);
        // A chord the bar does not want is the shell's, even with the bar up.
        app.run(Action::Find);
        app.handle(Key::Ctrl('s'));
        assert!(app.message().unwrap_or_default().contains("no file name"));
    }

    #[test]
    fn ctrl_g_asks_for_a_line_and_moves_the_caret_there() {
        let text: String = (1..=30).map(|n| format!("line {n}\n")).collect();
        let mut app = app(&text);
        app.handle(Key::Ctrl('g'));
        let rows = drawn(&mut app, 40, 8);
        // The prompt takes the row above the status line, as the window's does.
        assert!(rows[6].starts_with("Go to line:"), "{:?}", rows);
        assert!(rows[7].contains("Ln 1, Col 1"), "{:?}", rows);
        // A chord dropped on the prompt does not leave: the field is modal.
        app.handle(Key::Ctrl('q'));
        assert!(!app.take_quit(), "a chord left with the prompt up");
        for ch in "12".chars() {
            app.handle(Key::Char(ch));
        }
        let rows = drawn(&mut app, 40, 8);
        assert!(rows[6].starts_with("Go to line: 12"), "{:?}", rows);
        app.handle(Key::Enter);
        let rows = drawn(&mut app, 40, 8);
        assert!(!rows[6].starts_with("Go to line:"), "{:?}", rows);
        assert!(rows[7].contains("Ln 12, Col 1"), "{:?}", rows);
        // And the line the caret arrived on is on the screen, the view having
        // scrolled the least that would show it. The prompt's row went back to
        // the document when it closed, so the frame holds seven lines again.
        assert!(rows[6].contains("line 12"), "{:?}", rows);
    }

    #[test]
    fn ctrl_h_adds_the_replacement_row_and_enter_replaces() {
        let mut app = app("one two one\n");
        let rows = drawn(&mut app, 40, 6);
        assert!(!rows[3].contains("Find:"), "{:?}", rows);
        app.run(Action::Replace);
        let rows = drawn(&mut app, 40, 6);
        // The bar takes the two rows above the status line, and the document
        // gives up one to make room for the second.
        assert!(rows[3].contains("Find:"), "{:?}", rows);
        assert!(rows[4].starts_with("Replace:"), "{:?}", rows);
        assert!(rows[5].contains("Ln 1, Col 1"), "{:?}", rows);
        for ch in "one".chars() {
            app.handle(Key::Char(ch));
        }
        // Tab three times: the two marks, then the replacement field.
        for _ in 0..3 {
            app.handle(Key::Tab);
        }
        app.handle(Key::Char('1'));
        app.handle(Key::Enter);
        assert_eq!(app.editor.doc.text().as_str(), "1 two one\n");
        // The shell heard the bar's edit: the line is drawn again with the new
        // text on it, not with the one it replaced.
        let rows = drawn(&mut app, 40, 6);
        assert!(rows[0].contains("1 two one"), "{:?}", rows);
        // And the whole replacement is one press of undo.
        app.handle(Key::Escape);
        app.handle(Key::Ctrl('z'));
        let rows = drawn(&mut app, 40, 6);
        assert!(rows[0].contains("one two one"), "{:?}", rows);
    }

    #[test]
    fn the_find_bar_keeps_its_arrows_while_lines_wrap() {
        // A pattern with two matches on one long line: Down in the bar means the
        // match after, not the row below, even though the document wraps.
        let text = format!("{} needle {} needle\n", "x".repeat(200), "y".repeat(200));
        let mut app = App::with_document(Document::from_text(&text));
        *app.settings_mut() = Settings {
            wrap: true,
            ..Settings::default()
        };
        app.apply_settings();
        // A frame first: the row motion works in the cells the last frame drew,
        // so without one it has no width to divide by and declines for that
        // reason instead of this one.
        drawn(&mut app, 40, 6);
        app.run(Action::Find);
        for ch in "needle".chars() {
            app.handle(Key::Char(ch));
        }
        app.handle(Key::Down);
        assert_eq!(app.editor.selection_start(), 201);
        app.handle(Key::Down);
        assert_eq!(app.editor.selection_start(), 409);
    }

    #[test]
    fn a_command_the_shell_has_not_got_says_so() {
        let mut app = app("x\n");
        // Find and replace have arrived; find in files has not.
        app.run(Action::FindInFiles);
        let message = app.message().unwrap_or_default();
        assert!(message.contains("not in the terminal yet"), "{message}");
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

        // With wrapping on a vertical move walks rows, and counting the rows a
        // line takes is a pass over that line. That is the same order as the
        // line the move used to walk, but it is a pass per line crossed, so the
        // number is measured rather than assumed to have stayed put.
        let long: String = (0..100).map(|_| "0123456789".repeat(30) + "\n").collect();
        let mut wrapped = App::with_document(Document::from_text(&long));
        *wrapped.settings_mut() = Settings {
            wrap: true,
            ..Settings::default()
        };
        wrapped.apply_settings();
        let mut frame = Frame::new(120, 40);
        wrapped.draw(frame.grid());
        let mut moves = Vec::new();
        for _ in 0..100 {
            let start = std::time::Instant::now();
            wrapped.handle(Key::Down);
            moves.push(start.elapsed().as_nanos() as u64);
        }
        moves.sort_unstable();
        let wrapped_micros = moves[moves.len() / 2] as f64 / 1000.0;
        println!("a Down across wrapped rows takes {wrapped_micros:.1} us");
        assert!(wrapped_micros < 1_000.0, "a wrapped Down took {wrapped_micros} us");
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
