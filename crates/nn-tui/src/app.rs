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

use std::path::{Path, PathBuf};

use nn_core::paths::absolute;
use nn_core::symbols::scan;
use nn_core::{
    path_prefix, prefix_range, split_query, CLikeLexer, ConfigLexer, Document, Editor, FindInFiles,
    Highlighter, Lexer, Macro, Motion, Step as Recorded, Tree, MAX_CANDIDATES, MAX_REPEAT,
};
use nn_shared::action::Action;
use nn_shared::completion::{
    function_items, path_items, pooled_items, word_items, CompletionMode,
};
use nn_shared::hint::hint_at;
use nn_shared::i18n::Catalog;
use nn_shared::keymap::{Chord, KeyName, Keymap};
use nn_shared::results::hits_noun;
use nn_shared::session::{Session, SessionTab};
use nn_shared::settings::Settings;
use nn_shared::tabcolour::TabColour;
use unicode_width::UnicodeWidthChar;

use crate::complete::{Completion, Step as CompleteStep};
use crate::find::Find;
use crate::keys::Key;
use crate::menus::Menus;
use crate::paramhint::ParamHint;
use crate::palette::{self, Palette, Tone};
use crate::prefs::Prefs;
use crate::prompt::{Kind as PromptKind, Prompt};
use crate::results::{Results, Step};
use crate::screen::Grid;
use crate::symbols::Symbols;
use crate::workspace::Workspace;

/// How the window is divided up, worked out once per frame.
///
/// The bar is the last row, and the find bar -- when it is up -- takes the one
/// above it. The row the text starts losing rows at is therefore one field: the
/// number of rows of text is the row the first strip is drawn on.
#[derive(Debug, Clone, Copy)]
struct Layout {
    /// The first row of text: the tab strip's row plus one while the strip is
    /// drawn, and nought the rest of the time.
    top: usize,
    /// Columns the line numbers take.
    gutter: usize,
    /// Columns left for the text.
    text_cols: usize,
    /// Columns the workspace panel takes at the left edge, nought when it is
    /// down. The gutter is counted after it: `gutter` is where the text starts.
    dock: usize,
    /// The list above the status bar -- the search results, or the function
    /// list, whichever is up, and never both: the row it starts on, and how many
    /// rows it has, its header included.
    list: Option<(usize, usize)>,
    /// The first row of the strip above the status line, while anything is up:
    /// the prompt, or the find bar with its replacement row.
    bar_row: Option<usize>,
    /// The row the bar is drawn on.
    status_row: usize,
}

/// What drawing one line gave the frame: how many rows it took, and the cell
/// the caret was drawn on, when the caret is on that line at all.
#[derive(Debug, Clone, Copy)]
struct Drawn {
    rows: usize,
    caret: Option<(usize, usize)>,
}

impl Layout {
    /// The first row below the text: the find bar, the list, or the bar.
    ///
    /// Whichever of them starts highest, because the three stack upwards from the
    /// status row and the text stops at the first of them.
    fn bottom(self) -> usize {
        let bar = self.bar_row.unwrap_or(self.status_row);
        match self.list {
            Some((top, _)) => top.min(bar),
            None => bar,
        }
    }

    /// The rows the document has, which is every row between the two.
    fn text_rows(self) -> usize {
        self.bottom().saturating_sub(self.top)
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

/// One open document and everything that belongs to it.
///
/// The window keeps the same thing per tab: an editor, its highlighter and where
/// the view was scrolled to. The shell keeps the list and the index of the one in
/// front, and every command that reads a document reads it through `tab`, so a
/// second document is a row in `tabs` rather than a second shell.
pub struct Tab {
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
    /// With wrapping on, the segments of [`Tab::scroll_line`] above the screen.
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
    /// The column within a wrapped row that a run of vertical moves aims at.
    ///
    /// The core remembers a character column for the moves it makes, which is the
    /// right unit while a line is a row. With wrapping on the column that has to
    /// survive is the position inside the row -- `desired_col` in the core's
    /// sense would be a different place on every row -- so a tab keeps its own,
    /// and keeps it only across a run of vertical moves.
    desired_row: Option<usize>,
}

impl Tab {
    /// A tab on `doc`, highlighted unless the file is too large to be worth it.
    fn new(doc: Document) -> Self {
        let mut editor = Editor::new(doc);
        let mut highlighter = Highlighter::new(lexer_for(editor.doc.path()));
        // A large file opens unhighlighted for the same reason it does in the
        // windowed editor: the pass is O(file) and the pause is not worth it.
        if !editor.doc.is_large() {
            highlighter.highlight_all(&mut editor.doc);
        }
        Tab {
            editor,
            highlighter,
            scroll_line: 0,
            scroll_col: 0,
            scroll_seg: 0,
            visible_until: 0,
            desired_row: None,
        }
    }
}

/// The shell around the open documents, as the terminal sees it.
pub struct App {
    /// The open documents, in the order the window would show their tabs.
    tabs: Vec<Tab>,
    /// Which of them is in front.
    active: usize,
    /// The columns the text area had in the last frame.
    ///
    /// Kept because a vertical move is a move between rows while wrapping is on,
    /// and the rows depend on the column count -- which a keystroke needs before
    /// there is a frame to ask.
    view_cols: usize,
    /// The rows of text the last frame had, which is what a page is.
    view_rows: usize,
    /// The bar and its dropdowns, with the shared tree behind them.
    menus: Menus,
    /// The catalogue the labels are drawn through, kept so the bar can be
    /// rebuilt for a new recent list without reading the language off the disk
    /// again.
    catalog: Option<Catalog>,
    /// The recent files and the open tabs, which is the file both front-ends
    /// share.
    ///
    /// Read at startup and written at exit, which is when the window writes it
    /// too. The copy read at startup is where the tabs come from, and where the
    /// pins and the colours come from on the way out: a tab the window left
    /// pinned or painted goes back to it as it was.
    session: Session,
    /// The preferences dialog, and the settings it edits.
    ///
    /// Loading them at startup rather than when the dialog opens is what makes a
    /// terminal started twice behave the same way: what the dialog wrote last
    /// time is already in force when the first frame is drawn.
    prefs: Prefs,
    /// The find bar and its pattern, which outlives the bar so `F3` works after
    /// it closes.
    find: Find,
    /// The results of the last Find in Files, and where its cursor stands.
    ///
    /// Outlives the list itself, which is what lets `Search Results Window`
    /// bring back the search that was made rather than an empty panel.
    results: Results,
    /// What the document in front declares, and where its cursor stands.
    ///
    /// A list of its own rather than a second use of `results`: the two answer
    /// the same keys but hold different things, and the frame gives the slot
    /// above the status bar to whichever is open.
    symbols: Symbols,
    /// The recorded keystrokes, and whether one is being recorded.
    ///
    /// `nn_core`'s, and the same type the window holds: a recording is a list of
    /// editor calls, so a macro made here replays there and the other way round.
    recorder: Macro,
    /// The folder tree down the left edge.
    ///
    /// A dock of its own rather than a third list in the slot above the status
    /// bar: a tree wants height, and it wants the left edge, which is where the
    /// window puts it.
    workspace: Workspace,
    /// The one-line prompt, when a command needs a line typed before it runs.
    prompt: Prompt,
    /// The completion list, which shares its rows and its cursor with the
    /// window's through `nn-shared`.
    complete: Completion,
    /// The parameter hint, which shares the signature and the lit argument with
    /// the window through `nn-shared`.
    hint: ParamHint,
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
        // `Document::open` rather than `from_file`, because the name on the
        // command line means what it means in the File menu: a file that is not
        // there is a blank page for it, and its first save creates the file.
        // Absolute for the reason the typed path is: `notes.txt` and
        // `./notes.txt` are one file, and a second tab on it would be a second
        // buffer writing over the first.
        let doc = match path {
            Some(path) => Document::open(absolute(path))?,
            None => Document::new(),
        };
        let mut app = App::with_document(doc);
        // The recent list and the tabs, read once. Writing happens at exit,
        // which is when the window writes it too -- so a shell that is killed
        // rather than quit loses the same thing the window would.
        app.session = Session::load();
        match path {
            // The file the command line named is a recent one, as it is in the
            // window: it is the last file this shell was asked to open. A name
            // that is not on the disk is skipped by `record_recent`, because a
            // buffer for a file that has never been written is not somewhere the
            // user has been.
            Some(path) => app.record_recent(&absolute(path)),
            // Naming a file means that file and nothing else, which is the
            // window's rule: the session's tabs are reopened only when the
            // command line asked for none.
            None => app.restore_tabs(),
        }
        app.rebuild_menus();
        Ok(app)
    }

    /// Writes the session, for the loop that is leaving.
    ///
    /// Not called on every open: the file belongs to both front-ends, so it is
    /// written once, when this shell is done, out of the copy it read at startup
    /// with the tabs it has now.
    pub fn save_session(&self) {
        self.session_now().save();
    }

    /// Wraps a document that is already in memory.
    pub fn with_document(doc: Document) -> Self {
        let mut app = App {
            tabs: vec![Tab::new(doc)],
            active: 0,
            view_cols: 0,
            view_rows: 0,
            menus: Menus::new(&Keymap::default(), &[]),
            catalog: None,
            session: Session::default(),
            prefs: Prefs::new(),
            find: Find::new(),
            results: Results::new(),
            symbols: Symbols::new(),
            recorder: Macro::new(),
            workspace: Workspace::new(),
            prompt: Prompt::new(),
            complete: Completion::new(),
            hint: ParamHint::new(),
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

    /// The tab in front, which is what a command acts on.
    fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    /// The tab in front, for the parts of it a command writes.
    fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    /// The editor of the tab in front, for an edit.
    ///
    /// Apart from `tab_mut`, because most of the shell only ever wants the
    /// document, and `self.editor().insert(..)` says what the line does without
    /// spelling out which tab it does it to.
    fn editor(&mut self) -> &mut Editor {
        &mut self.tabs[self.active].editor
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
        self.menus.set_catalog(catalog.clone());
        // Kept as well, because the bar is rebuilt whenever a recent file is
        // added and the catalogue has to go back into the new tree.
        self.catalog = catalog;
    }

    /// Rebuilds the bar with the recent files it now has.
    ///
    /// The recent list is part of the shared tree rather than a list the File
    /// menu reads when it opens, so a new entry means a new tree. That is a few
    /// hundred rows of text built once per open command -- not on a keystroke --
    /// and nothing else in the shell reads the tree between one open and the
    /// next.
    fn rebuild_menus(&mut self) {
        let recent: Vec<String> = self
            .session
            .recent
            .iter()
            .map(|path| match path.file_name() {
                Some(name) => name.to_string_lossy().into_owned(),
                None => path.display().to_string(),
            })
            .collect();
        let mut menus = Menus::new(&self.keymap, &recent);
        menus.set_catalog(self.catalog.clone());
        self.menus = menus;
    }

    /// Records `path` as a recent file, and puts the bar's File menu back in step.
    ///
    /// Only files that are on the disk are worth remembering: a buffer for a name
    /// that has never been written is not somewhere the user has been, which is
    /// the window's rule and the reason `Save As` records the name it just made.
    fn record_recent(&mut self, path: &Path) {
        if !path.exists() {
            return;
        }
        self.session.push_recent(path);
        self.rebuild_menus();
    }

    /// Reopens the tabs the session remembers, and puts the one that was in
    /// front back in front.
    ///
    /// The window's rule, one for one. A file that has been moved or deleted
    /// since the last run is skipped rather than reported -- a session is a
    /// convenience, and refusing to start over a stale entry would cost more
    /// than the tab it lost -- and a session that restores nothing leaves the
    /// blank page the shell was built with.
    fn restore_tabs(&mut self) {
        let mut tabs: Vec<Tab> = Vec::new();
        for saved in &self.session.tabs {
            // Two tabs on one file would be two buffers writing over each other,
            // which is what `open_path` refuses; a session written by hand, or by
            // two front-ends at once, should not get past that either.
            if tabs
                .iter()
                .any(|tab| tab.editor.doc.path() == Some(saved.path.as_path()))
            {
                continue;
            }
            // `from_file` rather than `open`: a file that is gone is skipped, and
            // a name that was never written is not a tab to reopen.
            let Ok(doc) = Document::from_file(&saved.path) else {
                continue;
            };
            let mut tab = Tab::new(doc);
            // The caret goes back where it was, in the units the core counts in:
            // a zero-based line, and a column of characters.
            tab.editor.goto_position(saved.line, saved.column);
            tabs.push(tab);
        }
        if tabs.is_empty() {
            return;
        }
        self.active = self.session.active.min(tabs.len() - 1);
        self.tabs = tabs;
    }

    /// The session as it stands: every tab with a file, and the index that was
    /// in front.
    ///
    /// A buffer that has never been saved has no file to reopen, so it is not
    /// written; the window skips the same ones for the same reason. The pins and
    /// the colours are taken from the copy read at startup, because this shell
    /// has no way to set them and dropping them would repaint the window's tabs
    /// the next time it starts.
    fn session_now(&self) -> Session {
        let mut session = self.session.clone();
        session.tabs.clear();
        let mut active = 0;
        let mut kept = 0;
        for (index, tab) in self.tabs.iter().enumerate() {
            let Some(path) = tab.editor.doc.path() else {
                continue;
            };
            if index <= self.active {
                active = kept;
            }
            kept += 1;
            let before = self.session.tabs.iter().find(|saved| saved.path == path);
            session.tabs.push(SessionTab {
                path: path.to_path_buf(),
                line: tab.editor.caret_line(),
                column: tab.editor.caret_column(),
                pinned: before.is_some_and(|saved| saved.pinned),
                colour: before.map_or(TabColour::None, |saved| saved.colour),
            });
        }
        session.active = active.min(session.tabs.len().saturating_sub(1));
        session
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
            self.complete.close();
            self.menus.toggle();
            return;
        }
        // The prompt is modal, as the window's is: a letter belongs to the field
        // rather than to the document behind it, and a key the field has no use
        // for is dropped rather than handed on.
        if self.prompt.is_open() {
            self.prompt.handle(key);
            // Enter closes the row and leaves the answer behind, so the command
            // runs on the keystroke that answered it rather than on the next one.
            if let Some((kind, text)) = self.prompt.take_answer() {
                self.run_prompt(kind, text);
            }
            return;
        }
        self.message = None;
        // The remembered row survives a run of vertical moves and nothing else.
        if !matches!(
            key,
            Key::Up | Key::Down | Key::ExtendUp | Key::ExtendDown | Key::PageUp | Key::PageDown
        ) {
            self.tab_mut().desired_row = None;
        }
        if self.move_by_rows(key) {
            return;
        }
        // The find bar owns the keys it understands while it is up and hands the
        // rest back, so `Ctrl+S` saves and `Ctrl+Q` quits with a pattern half
        // typed: a bar that swallowed every chord would be a mode, and this one
        // is a row.
        // Two fields, not two helpers: the bar borrows the editor while it is
        // asked, so both have to come straight out of `self`.
        if self.find.is_open()
            && self.find.handle(key, &mut self.tabs[self.active].editor)
        {
            // The bar replaces through the editor rather than through the shell,
            // so this is the only place that hears the document changed.
            if self.find.take_replaced() {
                self.refresh_all();
            }
            // A mark the bar could not light is said here: the bar is one row and
            // has no room to explain itself, and the status bar is where the
            // shell already says what it could not do.
            if let Some(notice) = self.find.take_notice() {
                self.say(&notice, false);
            }
            return;
        }
        // The completion list takes the five keys that walk or take a row and
        // hands the rest back, which is the window's rule: a list that swallowed
        // every chord would be a mode, and this one is a box over the text. A
        // letter it would refilter on is typed first, so the box narrows on the
        // keystroke rather than a keystroke late.
        if self.complete.is_open() {
            match self.complete.handle(key) {
                CompleteStep::Moved | CompleteStep::Closed => return,
                CompleteStep::Accepted => {
                    self.accept_completion();
                    return;
                }
                CompleteStep::Typed(ch) => {
                    let line = self.editor().caret_line();
                    self.editor().insert(&ch.to_string());
                    self.refresh_from(line);
                    let mode = self.complete.mode();
                    self.open_completion(mode);
                    return;
                }
                CompleteStep::Passed => {}
            }
        }
        // The results list takes the keys that walk a list and hands the rest
        // back, for the reason the find bar does: a chord has to keep working
        // with the panel up.
        if self.results.is_open() {
            match self.results.handle(key) {
                Step::No => {}
                Step::Moved => return,
                Step::Closed => return,
                Step::Chosen(index) => {
                    self.open_hit(index);
                    self.results.close();
                    return;
                }
            }
        }
        // The function list answers the same keys the results list does, and
        // hands back the ones it has no use for, for the same reason. `Enter`
        // goes to the symbol and takes the list away: in a terminal a list of
        // this kind is a picker, and the reader is done with it.
        if self.symbols.is_open() {
            match self.symbols.handle(key) {
                Step::No => {}
                Step::Moved | Step::Closed => return,
                Step::Chosen(index) => {
                    self.goto_symbol(index);
                    return;
                }
            }
        }
        // The workspace tree walks itself while it is up, and it goes last: a
        // list over the bottom of the screen is a picker with the keyboard, and
        // the tree is a dock that can wait. `Enter` opens the row under the
        // cursor -- a folder where it stands, a file in a tab of its own.
        //
        // Letters go to the dock's filter line before its own keys, so that
        // typing in a tree narrowed by a click in the window narrows it here
        // too. `Escape` is the filter's while there is text to take back, and
        // the dock's once the line is empty.
        if self.workspace.filter_key(key) {
            return;
        }
        if self.workspace.is_open() {
            match self.workspace.handle(key) {
                Step::No => {}
                Step::Moved | Step::Closed => return,
                Step::Chosen(index) => {
                    self.choose_workspace_row(index);
                    return;
                }
            }
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
        let line = self.editor().caret_line();
        match key {
            Key::Char(ch) => {
                let text = ch.to_string();
                self.record(Recorded::Insert(text.clone()));
                self.editor().insert(&text);
                self.refresh_from(line);
            }
            Key::Enter => {
                // The indent is part of what was typed, because that is what
                // the editor inserted: replaying the newline alone would replay
                // a different edit than the one that happened.
                let text = format!("\n{}", self.editor().auto_indent());
                self.record(Recorded::Insert(text.clone()));
                self.editor().insert(&text);
                self.refresh_from(line);
            }
            Key::Tab => {
                let text = "    ".to_string();
                self.record(Recorded::Insert(text.clone()));
                self.editor().insert(&text);
                self.refresh_from(line);
            }
            Key::Backspace => {
                self.record(Recorded::Backspace);
                self.editor().backspace();
                self.refresh_from(line);
            }
            Key::Delete => {
                self.record(Recorded::Delete);
                self.editor().delete_forward();
                self.refresh_from(line);
            }
            Key::Up => self.moved(Motion::LineUp, false),
            Key::Down => self.moved(Motion::LineDown, false),
            Key::Left => self.moved(Motion::CharLeft, false),
            Key::Right => self.moved(Motion::CharRight, false),
            Key::ExtendUp => self.moved(Motion::LineUp, true),
            Key::ExtendDown => self.moved(Motion::LineDown, true),
            Key::ExtendLeft => self.moved(Motion::CharLeft, true),
            Key::ExtendRight => self.moved(Motion::CharRight, true),
            Key::WordLeft => self.moved(Motion::WordLeft, false),
            Key::WordRight => self.moved(Motion::WordRight, false),
            Key::LineStart => self.moved(Motion::LineStart, false),
            Key::LineEnd => self.moved(Motion::LineEnd, false),
            Key::DocStart => self.moved(Motion::DocStart, false),
            Key::DocEnd => self.moved(Motion::DocEnd, false),
            Key::PageUp => self.moved(Motion::PageUp, false),
            Key::PageDown => self.moved(Motion::PageDown, false),
            Key::Escape => {
                self.quit_armed = false;
                self.editor().clear_extra_carets();
            }
            // Every other control chord and function key is either bound in the
            // keymap, which was asked above, or bound to nothing. A chord bound
            // to nothing does nothing: typing the control character it stands
            // for would be the one answer nobody wants.
            Key::Ctrl(_) => {}
            Key::CtrlPageUp | Key::CtrlPageDown => {}
            Key::Function(_) | Key::ShiftFunction(_) | Key::CtrlFunction(_) => {}
        }
        // Typing the opening parenthesis of a call is the moment the signature
        // becomes useful, and it is the only moment the hint appears unasked; an
        // open one follows the caret so it cannot outlive the call.
        if key == Key::Char('(') || self.hint.is_open() {
            self.follow_hint();
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
            // A space is the one key whose chord is not spelled as a character:
            // the keymap says `Space`, and folding the two spellings together
            // here is what lets `Ctrl+Space` reach word completion.
            Key::Ctrl(' ') => Chord {
                ctrl: true,
                shift: false,
                alt: false,
                key: KeyName::Space,
            },
            Key::Ctrl(ch) => Chord::from_char(ch, true, false, false),
            // The two chords that carry a modifier a letter cannot: a terminal
            // sends `Ctrl+PageDown` as a sequence, so the pair is named here and
            // looked up like any other binding.
            Key::CtrlPageUp | Key::CtrlPageDown => Chord {
                ctrl: true,
                shift: false,
                alt: false,
                key: if key == Key::CtrlPageUp {
                    KeyName::PageUp
                } else {
                    KeyName::PageDown
                },
            },
            Key::Function(number) => Chord {
                ctrl: false,
                shift: false,
                alt: false,
                key: KeyName::Function(number),
            },
            // The two function-key chords a terminal can send with the modifier
            // held: `Shift+F2` is the previous bookmark and `Ctrl+F2` puts one
            // on the caret's line, both of them the window's own bindings.
            Key::ShiftFunction(number) => Chord {
                ctrl: false,
                shift: true,
                alt: false,
                key: KeyName::Function(number),
            },
            Key::CtrlFunction(number) => Chord {
                ctrl: true,
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
        // A command can change the document under the box, move to another tab
        // or open a dialog that owns the keyboard, so the list goes first; the
        // three completion commands open a fresh one on the line after.
        self.complete.close();
        match action {
            Action::Save => self.save(),
            // The three commands the recorder has a step for are recorded where
            // the keys that type are, so a macro recorded with the chords
            // replays what happened rather than what the keys spelled.
            Action::Undo => {
                self.record(Recorded::Undo);
                if self.editor().undo() {
                    self.refresh_all();
                }
            }
            Action::Redo => {
                self.record(Recorded::Redo);
                if self.editor().redo() {
                    self.refresh_all();
                }
            }
            Action::SelectAll => {
                self.record(Recorded::SelectAll);
                self.editor().select_all();
            }
            Action::ToggleRecording => self.toggle_recording(),
            Action::StopRecording => self.stop_recording(),
            Action::PlayMacro => self.play_macro(),
            Action::RunMacroMultiple => self.prompt.open(PromptKind::MacroRepeat),
            Action::SelectLine => self.editor().select_line(),
            Action::DuplicateLine => self.edit(|ed| ed.duplicate_lines()),
            Action::MoveLineUp => self.edit(|ed| ed.move_lines(true)),
            Action::MoveLineDown => self.edit(|ed| ed.move_lines(false)),
            Action::DeleteLine => self.edit(|ed| ed.delete_lines()),
            Action::CompleteWord => self.open_completion(CompletionMode::Word),
            Action::CompleteDocument => self.open_completion(CompletionMode::Document),
            Action::CompleteFunction => self.open_completion(CompletionMode::Function),
            Action::CompletePath => self.open_completion(CompletionMode::Path),
            Action::ParamHint => {
                if self.hint.is_open() {
                    self.hint.close();
                } else {
                    self.follow_hint();
                }
            }
            Action::ParamHintPrev => self.hint.step(-1),
            Action::ParamHintNext => self.hint.step(1),
            Action::Preferences => self.prefs.open(),
            Action::ToggleWrap => self.toggle_wrap(),
            Action::Find => self.find.open(&self.tabs[self.active].editor),
            Action::FindInFiles => self.prompt.open(PromptKind::FindInFiles),
            Action::SearchResultsWindow => self.search_results_window(),
            Action::ToggleFunctionList => self.toggle_function_list(),
            Action::FolderAsWorkspace => self.toggle_workspace(),
            Action::OpenFolderAsWorkspace => self.prompt.open(PromptKind::Workspace),
            Action::NextSearchResult => self.step_hit(true),
            Action::PrevSearchResult => self.step_hit(false),
            Action::Replace => self.find.open_replace(&self.tabs[self.active].editor),
            Action::GotoLine => self.prompt.open(PromptKind::GotoLine),
            Action::Open => self.prompt.open(PromptKind::Open),
            Action::OpenRecent(index) => {
                if let Some(path) = self.session.recent.get(index).cloned() {
                    self.open_path(&path.display().to_string());
                }
            }
            Action::ClearRecent => {
                self.session.recent.clear();
                self.rebuild_menus();
                self.say("the recent list is empty", false);
            }
            Action::New => self.new_document(),
            Action::CloseTab => self.close_document(),
            Action::CloseAll => self.close_all(),
            Action::NextTab => self.cycle_document(true),
            Action::PrevTab => self.cycle_document(false),
            Action::Reload => self.reload(),
            Action::SaveAs => self.prompt.open(PromptKind::SaveAs),
            // Two fields, for the reason the find bar's own handling gives: the
            // step borrows the editor while it walks the matches.
            Action::FindNext => {
                self.find.step(&mut self.tabs[self.active].editor, true)
            }
            Action::FindPrev => {
                self.find.step(&mut self.tabs[self.active].editor, false)
            }
            // Bookmarks are the engine's: `Document` has kept a marker array
            // beside the text since the window gave it one, so these four
            // commands only have to say where the caret is and where to look.
            Action::ToggleBookmark => {
                let line = self.editor().caret_line();
                self.tab_mut().editor.doc.toggle_bookmark(line);
            }
            Action::NextBookmark => self.step_bookmark(true),
            Action::PrevBookmark => self.step_bookmark(false),
            Action::ClearBookmarks => self.tab_mut().editor.doc.clear_bookmarks(),
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
        // An open hint follows whatever the command did to the caret; a closed
        // one stays closed, so it never appears on its own while you are typing.
        // The three rows that work the hint keep the highlight they were given:
        // they are the one place the lit argument is allowed to disagree with the
        // caret, and recomputing here would put it straight back.
        let stepped = matches!(
            action,
            Action::ParamHint | Action::ParamHintPrev | Action::ParamHintNext
        );
        if !stepped && self.hint.is_open() {
            self.follow_hint();
        }
    }

    /// Shows the signature of the call the caret is inside, or hides the hint.
    ///
    /// A large file is refused before the document is copied: the hint is a
    /// convenience, and reading a hundred megabytes per keystroke is not.
    fn follow_hint(&mut self) {
        if self.editor().doc.is_large() {
            self.hint.close();
            return;
        }
        let (text, caret) = {
            let editor = &self.tabs[self.active].editor;
            (editor.doc.text(), editor.caret())
        };
        match hint_at(&text, caret) {
            Some(hint) => self.hint.show(hint),
            None => self.hint.close(),
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
        self.tab_mut().scroll_col = 0;
        self.tab_mut().scroll_seg = 0;
        self.tab_mut().visible_until = 0;
        self.tab_mut().scroll_line = self.editor().caret_line();
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
        if self.editor().doc.path().is_none() {
            self.say("no file name: start with a path to save", true);
            return;
        }
        match self.editor().doc.save() {
            Ok(()) => self.say("saved", false),
            Err(error) => self.say(&format!("save failed: {error}"), true),
        }
    }

    /// Runs the command a prompt was asking for.
    ///
    /// The prompt is a field and nothing else: what a path means, what a line
    /// number means, and what the bar says afterwards all belong here, because
    /// this is the piece that can open a file and the piece with a status line to
    /// answer on.
    fn run_prompt(&mut self, kind: PromptKind, text: String) {
        match kind {
            // Counted from one, as the status bar counts, and clamped by the
            // editor: a line past the end of the file is its last line, which is
            // what the window does with the same number.
            PromptKind::GotoLine => {
                if let Ok(line) = text.trim().parse::<usize>() {
                    self.editor().goto_line(line.saturating_sub(1));
                }
            }
            PromptKind::Open => self.open_path(text.trim()),
            PromptKind::SaveAs => self.save_as(text.trim()),
            PromptKind::FindInFiles => self.find_in_files(text.trim()),
            PromptKind::Workspace => self.adopt_workspace(text.trim()),
            PromptKind::MacroRepeat => self.run_macro_multiple(text.trim()),
        }
    }

    /// Starts a blank document with no name, in a tab of its own.
    ///
    /// The window pushes a tab here, and now so does the shell: the buffer in
    /// front keeps its unsaved changes, which is what tab 135 was for. Nothing is
    /// said about it because the bar already is -- the path becomes `[No Name]`
    /// and the count beside it goes up by one.
    fn new_document(&mut self) {
        self.tabs.push(Tab::new(Document::new()));
        self.active = self.tabs.len() - 1;
    }

    /// Moves to the next or the previous document, wrapping around.
    ///
    /// The window says nothing here either, and it can afford to: the strip is
    /// on screen and the new tab is the one drawn in front. The terminal's answer
    /// is the bar's, which names the file that is now in front and the count it
    /// sits in, so a message would only repeat it. With one document there is
    /// nowhere to go and saying so is better than a chord that looks broken.
    fn cycle_document(&mut self, forward: bool) {
        let count = self.tabs.len();
        if count <= 1 {
            self.say("only one document is open", true);
            return;
        }
        self.active = if forward {
            (self.active + 1) % count
        } else {
            (self.active + count - 1) % count
        };
    }

    /// Closes the document in front, keeping at least one open.
    ///
    /// The window's `close_tab`, one document at a time. A dirty buffer stops it
    /// with the sentence every other command here gives -- there is no save
    /// prompt to fall back on -- and closing the last tab leaves a blank page
    /// rather than an empty strip: an editor with no document has nowhere to
    /// type.
    fn close_document(&mut self) {
        if self.editor().doc.is_dirty() {
            self.say("unsaved changes: Ctrl+S saves them first", true);
            return;
        }
        if self.tabs.len() == 1 {
            self.load(Document::new());
            self.say("closed the last document", false);
            return;
        }
        let closed = self
            .editor()
            .doc
            .path()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "[No Name]".to_string());
        self.tabs.remove(self.active);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
        self.say(&format!("closed {closed}"), false);
    }

    /// Closes every document, and says how many stopped it if any did.
    ///
    /// The window refuses the whole command when one document is dirty and names
    /// the count that is; that is the only answer that cannot lose an edit
    /// quietly, and it is what this says too.
    fn close_all(&mut self) {
        let dirty = self
            .tabs
            .iter()
            .filter(|tab| tab.editor.doc.is_dirty())
            .count();
        if dirty > 0 {
            let documents = if dirty == 1 { "document" } else { "documents" };
            self.say(
                &format!(
                    "unsaved changes in {dirty} {documents}: \
                     Ctrl+S saves them first"
                ),
                true,
            );
            return;
        }
        let closed = self.tabs.len();
        self.tabs.truncate(1);
        self.active = 0;
        self.load(Document::new());
        self.say(&format!("closed {closed} documents"), false);
    }

    /// Reads the document's file again, dropping what was typed since.
    ///
    /// The window's Reload does the same, and the command's whole meaning is
    /// "throw away what I have and read the file". The one liberty taken here is
    /// saying so: the window's tab loses its `[+]` mark where a reader can see it,
    /// and the terminal's bar is the only place the fact can land.
    fn reload(&mut self) {
        let Some(path) = self.editor().doc.path().map(|path| path.to_path_buf()) else {
            self.say("nothing to reload: this buffer has no file", true);
            return;
        };
        // `from_file` rather than `open`: a file that has been deleted since it
        // was opened is an error to report, not a blank page to hand back.
        match Document::from_file(&path) {
            Ok(doc) => {
                let dropped = self.editor().doc.is_dirty();
                let read_only = self.editor().doc.is_read_only();
                self.load(doc);
                if read_only {
                    self.editor().doc.set_read_only(true);
                }
                let at = path.display();
                if dropped {
                    self.say(
                        &format!("reloaded {at} (unsaved changes were dropped)"),
                        true,
                    );
                } else {
                    self.say(&format!("reloaded {at}"), false);
                }
            }
            Err(error) => self.say(&format!("cannot reload: {error}"), true),
        }
    }

    /// Opens `typed` in a tab of its own, or moves to the tab that has it.
    ///
    /// Made absolute, because a relative name means a different file the next
    /// time the shell is started from somewhere else -- the window learned that
    /// and `nn_core::paths::absolute` is what it learned it into. Nothing is
    /// refused on account of the buffer in front: that is the window's rule too,
    /// and it is what the tabs are for.
    ///
    /// A file that is already open is switched to rather than opened twice, of
    /// course: two tabs on one file would be two buffers writing over each other,
    /// with the last save winning and nothing said about it.
    ///
    /// A name that is not there opens a blank page for it rather than an error,
    /// which is `Document::open`'s rule and the window's: `npp notes.txt` in a
    /// directory without `notes.txt` gives a buffer whose first save creates the
    /// file.
    fn open_path(&mut self, typed: &str) {
        if typed.is_empty() {
            self.say("open: type a path", true);
            return;
        }
        let path = absolute(Path::new(typed));
        let already = self.has_tab(&path);
        if !self.show_file(&path) {
            return;
        }
        let message = if already {
            format!("already open: {}", path.display())
        } else {
            format!("opened {}", path.display())
        };
        self.say(&message, false);
    }

    /// Whether a tab already has `path`.
    fn has_tab(&self, path: &Path) -> bool {
        self.tabs
            .iter()
            .any(|tab| tab.editor.doc.path() == Some(path))
    }

    /// Puts the tab holding `path` in front, opening it when no tab has it.
    ///
    /// The half of `Open` that a hit in the results list needs too: it moves to
    /// the document and says nothing when it works, because what the bar says
    /// afterwards belongs to the command that asked. A file that cannot be read
    /// is reported here rather than by the caller, so the two commands cannot
    /// word the same failure differently.
    fn show_file(&mut self, path: &Path) -> bool {
        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.editor.doc.path() == Some(path))
        {
            self.active = index;
            return true;
        }
        match Document::open(path) {
            Ok(doc) => {
                self.tabs.push(Tab::new(doc));
                self.active = self.tabs.len() - 1;
                self.record_recent(path);
                true
            }
            Err(error) => {
                self.say(&format!("cannot open {}: {error}", path.display()), true);
                false
            }
        }
    }

    /// The folder a Find in Files search runs over by default.
    ///
    /// The one holding the file in front, because "find in files" means "find in
    /// the files around this one", and the working directory for a buffer that
    /// has never been saved -- the window's rule, word for word.
    fn default_search_dir(&self) -> PathBuf {
        self.tab()
            .editor
            .doc
            .path()
            .and_then(|path| path.parent())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }

    /// Searches the files around the document, and lists what it found.
    ///
    /// One field and one optional suffix: `pattern in directory` is the whole of
    /// the prompt's syntax, because it is `nn_core::split_query`'s and therefore
    /// the window's. The walk is synchronous here as it is there: the hit cap
    /// means the common case is over in a few milliseconds, and the alternative
    /// is a thread, a channel and a way to cancel, for the case where it is not.
    fn find_in_files(&mut self, typed: &str) {
        let (pattern, directory) = split_query(typed, self.default_search_dir());
        if pattern.is_empty() {
            self.say("find in files: type a pattern", true);
            return;
        }
        match FindInFiles::new(&pattern, &directory).run() {
            Ok(hits) => {
                let message = format!(
                    "searched {}: {}",
                    directory.display(),
                    hits_noun(hits.len())
                );
                self.results.set_hits(hits);
                self.symbols.close();
                self.say(&message, false);
            }
            // The window says the same thing, on the desk it prints to.
            Err(error) => self.say(&format!("bad pattern: {error}"), true),
        }
    }

    /// Brings the results list back, or says there is nothing to show.
    ///
    /// The list leaves the screen when a hit is chosen, because in a terminal it
    /// is a picker; this is how it comes back, with the search that was made
    /// rather than a new one.
    fn search_results_window(&mut self) {
        if !self.results.reopen() {
            self.say("no search results yet", true);
            return;
        }
        let message = format!("{} results in the list", self.results.hits().len());
        self.say(&message, false);
    }

    /// Steps to the next or the previous hit, wrapping, and opens it.
    ///
    /// Stepping does not open the list: reading the matches in the document is
    /// one thing, and a row of results over it would be in the way of it.
    fn step_hit(&mut self, forward: bool) {
        match self.results.advance(forward) {
            Some(index) => self.open_hit(index),
            None => self.say("no search results yet", true),
        }
    }

    /// Opens the file a hit points at, and puts the caret where the match is.
    ///
    /// The caret goes to the column as well as the line, which is what makes a
    /// hit a place to stand rather than a line to search again.
    fn open_hit(&mut self, index: usize) {
        let Some(hit) = self.results.hit(index).cloned() else {
            return;
        };
        match hit.tab {
            // A hit from a buffer names its tab and may have no path at all, so
            // the switch is what "open" means; this shell makes no such hits yet,
            // but `Hit` says a search may, and a second tab on one file would be
            // the bug `open_path` exists to prevent.
            Some(tab) if tab < self.tabs.len() => self.active = tab,
            _ => {
                if !self.show_file(&hit.path) {
                    return;
                }
            }
        }
        self.editor().goto_position(hit.line, hit.column);
        let name = match hit.path.file_name() {
            Some(name) => name.to_string_lossy().into_owned(),
            None => hit.path.display().to_string(),
        };
        let message = format!(
            "{name}:{} ({}/{})",
            hit.line + 1,
            self.results.position(),
            self.results.hits().len()
        );
        self.say(&message, false);
    }

    /// Records `step`, if a recording is running.
    ///
    /// One line at each place the shell calls the editor, which is the same
    /// place the window records: a step is an editor call and not a keypress, so
    /// a macro means what it did whoever pressed it.
    fn record(&mut self, step: Recorded) {
        self.recorder.record(step);
    }

    /// Moves the caret, recording the motion on the way.
    fn moved(&mut self, motion: Motion, extend: bool) {
        self.record(Recorded::Move(motion, extend));
        self.editor().move_caret(motion, extend);
    }

    /// Starts a recording, or stops the one running.
    ///
    /// The window's toggle, one for one: the Macro menu's `Start Recording`
    /// stops a recording that is already running, and `Stop Recording` says
    /// there is nothing to stop when there is not.
    fn toggle_recording(&mut self) {
        if self.recorder.is_recording() {
            self.stop_recording();
            return;
        }
        self.recorder.start();
        self.say("recording a macro", false);
    }

    /// Stops the recorder, saying how much it caught.
    fn stop_recording(&mut self) {
        if !self.recorder.is_recording() {
            self.say("not recording", true);
            return;
        }
        self.recorder.stop();
        let message = format!("macro recorded, {} steps", self.recorder.len());
        self.say(&message, false);
    }

    /// Replays the recorded macro against the document in front.
    ///
    /// The steps go through the same `Editor` calls the keyboard does, so a
    /// macro cannot do anything a person could not, and the highlighter is
    /// re-run from the first of the two lines the caret stood on -- a playback
    /// can move it either way.
    fn play_macro(&mut self) {
        if self.recorder.is_empty() {
            self.say("no macro recorded", true);
            return;
        }
        let steps = self.recorder.len();
        let from = {
            let tab = &mut self.tabs[self.active];
            let before = tab.editor.caret_line();
            self.recorder.play(&mut tab.editor);
            let after = tab.editor.caret_line();
            tab.editor.ensure_caret_visible();
            before.min(after)
        };
        self.refresh_from(from);
        let message = format!("macro played, {steps} steps");
        self.say(&message, false);
    }

    /// Plays the recorded macro a number of times.
    ///
    /// The count is clamped rather than refused, and to `nn_core`'s cap rather
    /// than a number of this shell's: the window makes the same promise about
    /// the same recording.
    fn run_macro_multiple(&mut self, typed: &str) {
        let Ok(times) = typed.parse::<usize>() else {
            self.say("play macro times: type a number", true);
            return;
        };
        if self.recorder.is_empty() {
            self.say("no macro recorded", true);
            return;
        }
        let times = times.clamp(1, MAX_REPEAT);
        for _ in 0..times {
            self.play_macro();
        }
        self.say(&format!("macro played {times} time(s)"), false);
    }

    /// Shows the folder the document is in as a tree, or takes the dock down.
    ///
    /// The window's command and the window's starting folder: the one the
    /// document in front lives in, and this shell's working directory for a
    /// buffer that has no file yet.
    fn toggle_workspace(&mut self) {
        if self.workspace.is_open() {
            self.workspace.close();
            return;
        }
        let root = self.workspace_root();
        let tree = Tree::scan(&[root]);
        self.workspace.show(tree, "Folder as Workspace");
    }

    /// The folder a workspace command starts from.
    fn workspace_root(&self) -> PathBuf {
        self.tab()
            .editor
            .doc
            .path()
            .and_then(|path| path.parent())
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("/"))
    }

    /// Shows a typed folder as the workspace tree.
    ///
    /// A path that is not a folder is refused in the bar rather than shown as an
    /// empty tree, which is what the window says and what keeps an empty panel
    /// from looking like an empty folder.
    fn adopt_workspace(&mut self, typed: &str) {
        if typed.is_empty() {
            self.say("folder: type a path", true);
            return;
        }
        let path = absolute(Path::new(typed));
        if !path.is_dir() {
            let message = format!("{} is not a folder", path.display());
            self.say(&message, true);
            return;
        }
        let tree = Tree::scan(std::slice::from_ref(&path));
        self.workspace.show(tree, "Folder as Workspace");
        self.say(&format!("workspace: {}", path.display()), false);
    }

    /// Opens the row the tree's cursor is standing on.
    ///
    /// A folder opens or closes where it stands and the tree stays up, because
    /// the reader is still walking it. A file is opened and the dock goes: on a
    /// terminal the tree holds the arrows while it is up, so arriving somewhere
    /// has to hand them back. The window keeps its panel because a click on the
    /// text is a way to hand them back that a terminal does not have.
    fn choose_workspace_row(&mut self, index: usize) {
        let Some(path) = self.workspace.path_at(index) else {
            return;
        };
        if self.workspace.is_dir_at(index) {
            self.workspace.tree_mut().toggle(&path);
            return;
        }
        self.workspace.close();
        self.open_path(&path.display().to_string());
    }

    /// Shows what the document in front declares, or puts the list away.
    ///
    /// The window's command, and the window's rule for a document the scan finds
    /// nothing in: no symbols means no list. The shell says so in the bar,
    /// because a terminal has no column to be visibly absent.
    ///
    /// Opening it puts the results list away, and a search does the same to it
    /// in turn: the two share the rows over the status bar, and a frame that
    /// showed both would leave the document a few rows of its own.
    fn toggle_function_list(&mut self) {
        if self.symbols.is_open() {
            self.symbols.close();
            return;
        }
        let symbols = scan(&self.editor().doc.text());
        self.symbols.set_symbols(symbols);
        if !self.symbols.is_open() {
            self.say("no symbols in this document", true);
            return;
        }
        self.results.close();
    }

    /// Puts the caret on the line a symbol is declared on, and closes the list.
    ///
    /// The line's first column, which is where the window puts it when a row of
    /// its own column is clicked. The bar says which row of how many, the way it
    /// does for a hit: a list that vanishes on a keypress leaves nothing to say
    /// where the reader landed.
    fn goto_symbol(&mut self, index: usize) {
        let Some((line, name)) = self.symbols.symbol(index).map(|symbol| {
            (symbol.line, symbol.name.clone())
        }) else {
            return;
        };
        let position = self.symbols.position();
        let count = self.symbols.symbols().len();
        self.editor().goto_position(line, 0);
        self.symbols.close();
        let message = format!("{name}:{} ({position}/{count})", line + 1);
        self.say(&message, false);
    }

    /// Writes the document to `typed` and adopts that path.
    ///
    /// The language follows the extension, as it does when a file is opened: a
    /// buffer saved as `x.rs` should colour itself as Rust from the next line on,
    /// not keep the lexer of the name it used to have.
    fn save_as(&mut self, typed: &str) {
        if typed.is_empty() {
            self.say("save as: type a path", true);
            return;
        }
        let path = absolute(Path::new(typed));
        match self.editor().doc.save_as(&path) {
            Ok(()) => {
                self.tab_mut().highlighter = Highlighter::new(lexer_for(self.editor().doc.path()));
                self.refresh_all();
                self.record_recent(&path);
                self.say(&format!("saved {}", path.display()), false);
            }
            Err(error) => self.say(&format!("save failed: {error}"), true),
        }
    }

    /// Puts `doc` in front, dropping everything that described the one before it.
    ///
    /// Scroll, the remembered row and the quit warning all belong to the document
    /// that was open, so a new one starts at its first line with nothing armed.
    fn load(&mut self, doc: Document) {
        self.tabs[self.active] = Tab::new(doc);
        self.quit_armed = false;
    }

    /// Quits, or warns once that there is unsaved work.
    fn ask_to_quit(&mut self) {
        if self.editor().doc.is_dirty() && !self.quit_armed {
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
        if edit(self.editor()) {
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

    /// Jumps to the next or the previous bookmark, or says there is none.
    ///
    /// The window's words for the same refusal, on the bar rather than on
    /// stderr: a terminal has a line for what it could not do, and a key that
    /// does nothing at all is a key a reader presses twice.
    fn step_bookmark(&mut self, forward: bool) {
        if !self.tab_mut().editor.goto_bookmark(forward) {
            self.say("no bookmarks", true);
        }
    }

    fn say(&mut self, text: &str, alert: bool) {
        self.message = Some(Message {
            text: text.to_string(),
            alert,
        });
    }

    /// Re-lexes from `line` down, which is all an edit can have changed.
    fn refresh_from(&mut self, line: usize) {
        // Through one binding: the highlighter writes spans into the document it
        // reads, so the two halves of a tab have to be borrowed together.
        let tab = &mut self.tabs[self.active];
        tab.highlighter.highlight_from(&mut tab.editor.doc, line);
    }

    /// Fills the completion list for `mode`, and keeps it closed when there is
    /// nothing to offer.
    ///
    /// Every mode but path completion copies the document, which a large file
    /// cannot afford per keystroke, so the chord says so rather than doing
    /// nothing. The words, the cap and the rows themselves come from
    /// `nn-shared`, which is where the window's list comes from too; the anchor
    /// is the frame's business, and it is worked out where the frame is drawn.
    fn open_completion(&mut self, mode: CompletionMode) {
        // Every mode but path completion copies the document, which a large file
        // cannot afford per keystroke; a directory listing is not the document.
        if mode != CompletionMode::Path && self.editor().doc.is_large() {
            self.complete.close();
            self.say("Completion is off for large files", true);
            return;
        }
        let (text, caret) = {
            let editor = &self.tabs[self.active].editor;
            (editor.doc.text(), editor.caret())
        };
        let items = match mode {
            CompletionMode::Word => word_items(&text, caret, MAX_CANDIDATES),
            CompletionMode::Document => {
                // Every open document, large ones skipped and the pool capped:
                // document completion is a convenience, and copying a hundred
                // megabytes of text per keystroke would make it a liability.
                const MAX_POOL_BYTES: usize = 4 * 1024 * 1024;
                let mut texts: Vec<String> = Vec::new();
                let mut total = 0usize;
                for tab in &self.tabs {
                    if tab.editor.doc.is_large() {
                        continue;
                    }
                    let text = tab.editor.doc.text();
                    total += text.len();
                    texts.push(text);
                    if total >= MAX_POOL_BYTES {
                        break;
                    }
                }
                let pool: Vec<&str> = texts.iter().map(String::as_str).collect();
                pooled_items(&text, caret, &pool, MAX_CANDIDATES)
            }
            CompletionMode::Function => function_items(&text, caret, MAX_CANDIDATES),
            CompletionMode::Path => {
                let base = self
                    .tab()
                    .editor
                    .doc
                    .path()
                    .and_then(|path| path.parent())
                    .map(std::path::Path::to_path_buf);
                path_items(&text, caret, base.as_deref(), MAX_CANDIDATES)
            }
        };
        if items.is_empty() {
            self.complete.close();
            return;
        }
        self.complete.show(mode, items);
    }

    /// Replaces the word before the caret with the row under the cursor.
    ///
    /// The insertion is not the label: a function row shows its signature but
    /// inserts `name()`, and the caret then steps back between the parentheses.
    /// The prefix is replaced through the editor, so undo, the highlight and the
    /// caret all see one edit.
    fn accept_completion(&mut self) {
        let Some(item) = self.complete.selected().cloned() else {
            self.complete.close();
            return;
        };
        let (text, caret) = {
            let editor = &self.tabs[self.active].editor;
            (editor.doc.text(), editor.caret())
        };
        // A path fragment can hold dots and dashes and a slash, so it is read by
        // the path rule; the three word modes extend the word before the caret.
        let range = match self.complete.mode() {
            CompletionMode::Path => path_prefix(&text, caret),
            _ => prefix_range(&text, caret),
        };
        let Some(range) = range else {
            self.complete.close();
            return;
        };
        let doc = &self.tabs[self.active].editor.doc;
        let start = doc.byte_to_char(range.start);
        let end = doc.byte_to_char(range.end);
        let line = self.tabs[self.active].editor.caret_line();
        self.complete.close();
        let editor = self.editor();
        editor.select_range(start..end);
        editor.insert(&item.insert);
        for _ in 0..item.back {
            editor.move_caret(Motion::CharLeft, false);
        }
        self.refresh_from(line);
    }

    /// Re-lexes everything: undo and redo can have changed any line.
    fn refresh_all(&mut self) {
        if self.editor().doc.is_large() {
            let line = self.editor().caret_line();
            self.refresh_from(line);
        } else {
            let tab = &mut self.tabs[self.active];
            tab.highlighter.highlight_all(&mut tab.editor.doc);
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
            || self.editor().selections.len() > 1
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
        let line = self.tab().editor.caret_line();
        let column = self.tab().editor.caret_column();
        let display = self.display_column(line, column);
        let row_col = *self.tab_mut().desired_row.get_or_insert(display % cols);
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
        let position = self.editor().doc.line_to_char(at.0) + self.char_at_display(at.0, target);
        if extend {
            self.editor().selections.primary_mut().extend_to(position);
        } else {
            self.editor().selections.set_caret(position);
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
                (line + 1 < self.tab().editor.doc.len_lines()).then_some((line + 1, 0))
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
        for (index, ch) in self.tab().editor.doc.line_chars(line).enumerate() {
            if ch == '\n' || ch == '\r' || display >= target {
                return index;
            }
            display = advance(display, ch, self.settings.tab_width);
        }
        self.line_length(line)
    }

    /// How many columns the line numbers take, with their trailing space.
    fn gutter_width(&self) -> usize {
        let digits = self.tab().editor.doc.len_lines().to_string().len();
        digits + 1
    }

    /// The display column of character `column` of `line`, with tabs expanded.
    fn display_column(&self, line: usize, column: usize) -> usize {
        let mut display = 0;
        for (index, ch) in self.tab().editor.doc.line_chars(line).enumerate() {
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
        // The dock is counted first, because the gutter is counted after it: a
        // line number sits beside the text, and the tree sits beside the line
        // number. A third of the screen, and never more than the window's own
        // column aims to be -- a tree is read for the names on it, and a
        // terminal that gave it half its width would have given the document
        // nothing.
        let dock = if self.workspace.is_open() {
            (width / 3).min(28)
        } else {
            0
        };
        let gutter = (dock + self.gutter_width()).min(width);
        // The strip is drawn only when there is more than one document to name,
        // which is the window's rule, and only on a screen tall enough to spare
        // the row: two documents and two rows is a shell with nowhere to type.
        let strip = self.tabs.len() > 1 && height >= 3;
        // The bottom of the screen, counted upwards: the results list sits on the
        // status row and the prompt or the find bar sits above it. A list is a
        // convenience over the document, so it never takes more than half the
        // rows the text could have had.
        //
        // Which of the two is up decides where the block starts, and they are not
        // the same question: the bar row is the prompt's or the find bar's, and a
        // list on its own must not claim it -- the find bar would be drawn over
        // the list's header.
        let find_rows = if self.prompt.is_open() {
            1
        } else {
            self.find.rows()
        };
        let room = height.saturating_sub(find_rows + 1) / 2;
        let results_rows = if self.symbols.is_open() {
            self.symbols.screen_rows(room)
        } else {
            self.results.screen_rows(room)
        };
        let block_top = height.saturating_sub(1 + find_rows + results_rows);
        let layout = Layout {
            top: usize::from(strip),
            gutter,
            text_cols: width - gutter,
            dock,
            list: (results_rows > 0).then(|| (block_top + find_rows, results_rows)),
            bar_row: (find_rows > 0).then_some(block_top),
            status_row: height - 1,
        };
        let text_rows = layout.text_rows();
        let status_row = layout.status_row;

        // The core moves by a screenful on Page Up and Page Down, and it takes
        // "a screenful" from the viewport. In cells, a line is one unit high.
        let viewport = &mut self.editor().viewport;
        viewport.height = text_rows as f32;
        viewport.line_height = 1.0;
        viewport.char_width = 1.0;
        viewport.width = layout.text_cols as f32;

        let caret_line = self.editor().caret_line();
        let caret_col = self.editor().caret_column();
        let caret_display = self.display_column(caret_line, caret_col);

        let wrap = self.settings.wrap && layout.text_cols > 0;
        if !wrap {
            self.tab_mut().scroll_seg = 0;
        }
        if wrap {
            // Wrapping means the caret is somewhere on a line that may take
            // several rows, so the question is not which column is on screen but
            // which rows the last frame had.
            self.tab_mut().scroll_col = 0;
            self.reveal_wrapped(caret_line, caret_display, layout);
        } else {
            // Keep the caret on the screen, scrolling the least that will do it.
            if caret_line < self.tab_mut().scroll_line {
                self.tab_mut().scroll_line = caret_line;
            }
            if text_rows > 0 && caret_line >= self.tab_mut().scroll_line + text_rows {
                self.tab_mut().scroll_line = caret_line + 1 - text_rows;
            }
            if caret_display < self.tab_mut().scroll_col {
                self.tab_mut().scroll_col = caret_display;
            }
            if layout.text_cols > 0 && caret_display >= self.tab_mut().scroll_col + layout.text_cols {
                self.tab_mut().scroll_col = caret_display + 1 - layout.text_cols;
            }
            self.tab_mut().visible_until = self.tab_mut().scroll_line + text_rows;
        }

        self.view_cols = layout.text_cols;
        self.view_rows = text_rows;
        let caret = self.editor().caret();
        let selection = self.editor().selection_start()..self.editor().selection_end();

        for row in layout.top..layout.bottom() {
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
        let mut row = layout.top;
        let mut line = self.tab_mut().scroll_line;
        let mut skip = if wrap { self.tab_mut().scroll_seg } else { 0 };
        // Where the caret ended up on the screen, which is where the completion
        // list hangs from: only the line the caret is on can answer that, and
        // only once it has been drawn.
        let mut caret_cell = None;
        while row < layout.bottom() && line < self.editor().doc.len_lines() {
            if skip == 0 {
                // A line wears its number on its first row, and a continuation
                // row has none: the strip reads as one line rather than as
                // several with the same number.
                self.draw_gutter(grid, row, line, layout.dock, layout.gutter, caret_line);
            }
            let drawn = self.draw_line(grid, row, line, layout, caret, &selection, skip, wrap);
            if drawn.caret.is_some() {
                caret_cell = drawn.caret;
            }
            row += drawn.rows;
            line += 1;
            skip = 0;
        }
        self.tab_mut().visible_until = line;
        if strip {
            self.draw_strip(grid, 0, width);
        }
        if layout.dock > 0 {
            let rows = layout.bottom().saturating_sub(layout.top);
            self.workspace.draw(grid, layout.top, rows, layout.dock);
        }
        if let Some((top, rows)) = layout.list {
            if self.symbols.is_open() {
                self.symbols.draw(grid, top, rows, width);
            } else {
                self.results.draw(grid, top, rows, width);
            }
        }
        if let Some(row) = layout.bar_row {
            if self.prompt.is_open() {
                self.prompt.draw(grid, row, width);
            } else {
                self.find.draw(grid, row, width);
            }
        }
        self.draw_status(grid, status_row, width);
        // The completion list over the text: it is the answer to a chord the
        // reader pressed and it has to be readable, but it is a box beside the
        // caret rather than a mode, so a dropdown or the preferences dialog
        // still covers it.
        if let Some(cell) = caret_cell {
            self.hint.draw(grid, cell);
            self.complete.draw(grid, cell);
        }
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

    /// The strip of open documents, along the top row.
    ///
    /// The window's tab strip, in the one row a terminal gives it: the name of
    /// each document, the one in front lit by the same tone an open menu's title
    /// gets, a `*` after the name of one with unsaved changes, and the rest of the
    /// row painted the strip's own colour so it reads as a bar rather than as a
    /// line of text.
    ///
    /// A strip longer than the screen drops the names on the left until what is
    /// left fits, because the one thing a strip must never do is hide the
    /// document in front. The row is cut rather than wrapped for the same reason:
    /// a second row of names would be a second line of the document's.
    fn draw_strip(&self, grid: &mut Grid, row: usize, width: usize) {
        for column in 0..width {
            grid.put(row, column, ' ', Tone::Menu);
        }
        let names: Vec<String> = self
            .tabs
            .iter()
            .map(|tab| {
                let mark = if tab.editor.doc.is_dirty() { "*" } else { "" };
                format!(" {}{mark} ", strip_name(&tab.editor.doc))
            })
            .collect();
        // The name in front, then as many of its neighbours as the row holds --
        // to the right first, then to the left with what is left. That order is
        // the whole rule: the document in front is the one name that is always
        // drawn, and it is drawn whole unless it is wider than the screen.
        let active = self.active.min(names.len() - 1);
        let mut used = names[active].chars().count();
        let mut end = active + 1;
        while end < names.len() && used + names[end].chars().count() <= width {
            used += names[end].chars().count();
            end += 1;
        }
        let mut start = active;
        while start > 0 && used + names[start - 1].chars().count() <= width {
            start -= 1;
            used += names[start].chars().count();
        }
        let mut column = 0usize;
        for (index, name) in names.iter().enumerate().take(end).skip(start) {
            if column >= width {
                break;
            }
            let tone = if index == self.active {
                Tone::MenuHover
            } else {
                Tone::Menu
            };
            for ch in name.chars() {
                if column >= width {
                    break;
                }
                grid.put(row, column, ch, tone);
                column += 1;
            }
        }
    }

    /// Puts the line number, right-aligned, at the left of `row`.
    fn draw_gutter(
        &self,
        grid: &mut Grid,
        row: usize,
        line: usize,
        left: usize,
        gutter: usize,
        caret: usize,
    ) {
        if gutter <= left + 1 {
            return;
        }
        let tone = if line == caret {
            Tone::GutterActive
        } else {
            Tone::Gutter
        };
        let number = format!("{:>width$}", line + 1, width = gutter - left - 1);
        grid.text(row, left, &number, tone);
        // The one cell between the number and the text is the bookmark margin,
        // as it is in the window: a dot on the lines that carry one, and the
        // gutter's own blank everywhere else, so the numbers never shift.
        if self.tab().editor.doc.is_bookmarked(line) {
            grid.put(row, gutter - 1, '\u{2022}', Tone::Bookmark);
        } else {
            grid.put(row, gutter - 1, ' ', tone);
        }
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
        if caret_line < self.tab_mut().scroll_line {
            self.tab_mut().scroll_line = caret_line;
            self.tab_mut().scroll_seg = 0;
        }
        if self.tab_mut().visible_until <= self.tab_mut().scroll_line {
            // Nothing is known about what fits -- a first frame, or a window that
            // has just changed size -- so the caret's line is given the benefit.
            self.tab_mut().visible_until = self.tab_mut().scroll_line + rows;
        }
        if caret_line >= self.tab_mut().visible_until {
            self.tab_mut().scroll_line = if rows > 0 && caret_line < self.tab_mut().visible_until + rows {
                self.tab_mut().visible_until
            } else {
                (caret_line + 1).saturating_sub(rows)
            };
            self.tab_mut().scroll_seg = 0;
        }
        if caret_line == self.tab_mut().scroll_line && rows > 0 {
            if segment < self.tab_mut().scroll_seg {
                self.tab_mut().scroll_seg = segment;
            }
            if segment >= self.tab_mut().scroll_seg + rows {
                self.tab_mut().scroll_seg = segment + 1 - rows;
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
            if at_row >= layout.bottom() {
                return CellAt::Past;
            }
            CellAt::Put(at_row, layout.gutter + display % layout.text_cols)
        } else {
            if display < self.tab().scroll_col {
                return CellAt::Hidden;
            }
            let column = layout.gutter + display - self.tab().scroll_col;
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
    ) -> Drawn {
        let line_start = self.tab().editor.doc.line_to_char(line);
        let mut display = 0usize;
        let mut used = 1usize;
        let mut caret_cell = None;
        for (index, ch) in self.tab().editor.doc.line_chars(line).enumerate() {
            if ch == '\n' || ch == '\r' {
                break;
            }
            let width = ch.width().unwrap_or(1);
            let at = display;
            display = advance(display, ch, self.settings.tab_width);
            let char_index = line_start + index;
            let here = char_index == caret;
            let tone = if here {
                Tone::Caret
            } else if selection.contains(&char_index) {
                Tone::Selection
            } else {
                palette::tone(self.tab().editor.doc.style_at(char_index))
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
                        if here {
                            caret_cell = Some((at_row, column));
                        }
                    }
                    CellAt::Past => {
                        // Nothing after this fits. With wrapping on that is the
                        // bottom of the screen, and every row left belongs to
                        // this line; with it off it is the right-hand edge, and
                        // the line is as tall as it ever was.
                        return Drawn {
                            rows: if wrap {
                                layout.bottom().saturating_sub(row).max(1)
                            } else {
                                used
                            },
                            caret: caret_cell,
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
                    caret_cell = Some((at_row, column));
                }
                CellAt::Hidden => {}
                CellAt::Past => {
                    return Drawn {
                        rows: if wrap {
                            layout.bottom().saturating_sub(row).max(1)
                        } else {
                            used
                        },
                        caret: caret_cell,
                    };
                }
            }
        }
        Drawn {
            rows: used,
            caret: caret_cell,
        }
    }

    /// How many characters `line` holds, without its terminator.
    fn line_length(&self, line: usize) -> usize {
        self.tab().editor.doc.line_len(line)
    }

    /// The bar at the bottom: where the file is, and where the caret is in it.
    fn draw_status(&self, grid: &mut Grid, row: usize, width: usize) {
        for column in 0..width {
            grid.put(row, column, ' ', Tone::StatusBar);
        }
        let mut left = self.tabs[self.active]
            .editor
            .doc
            .path()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "[No Name]".to_string());
        if self.tab().editor.doc.is_dirty() {
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
        let line = self.tab().editor.caret_line() + 1;
        let column = self.tab().editor.caret_column() + 1;
        // The recording indicator, in the window's words and in front of the
        // labels the window puts after it: a recording is the one state the bar
        // has to announce, because nothing else on the screen shows it.
        let rec = if self.recorder.is_recording() {
            format!("REC {}  ", self.recorder.len())
        } else {
            String::new()
        };
        let right = format!(
            "{rec}Ln {}, Col {}  {}  {} ",
            line,
            column,
            self.tab().editor.doc.encoding().name(),
            self.tab().editor.doc.eol().name()
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

/// What the strip calls a document: its file name, or `[No Name]` for one that
/// has never been written.
///
/// The window's strip says `new` for such a document, which is the core's own
/// word for it; the terminal's bar has said `[No Name]` since its first frame, and
/// one screen calling one document two things is worse than either word. For a
/// document that has a file the two agree, because `display_name` is where the
/// window got the name as well.
fn strip_name(doc: &Document) -> String {
    if doc.path().is_none() {
        return "[No Name]".to_string();
    }
    doc.display_name()
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

    /// The caret, as a line and a column.
    fn caret(app: &App) -> (usize, usize) {
        let editor = &app.tabs[app.active].editor;
        (editor.caret_line(), editor.caret_column())
    }

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
        assert_eq!(app.tab().editor.doc.text(), "hello");
        let rows = drawn(&mut app, 60, 4);
        assert!(rows[0].contains("hello"), "{:?}", rows);
        // The line number is there, in the gutter.
        assert!(rows[0].starts_with("1 "), "{:?}", rows);
        assert!(rows[0].trim_end().ends_with("hello"), "{:?}", rows);
    }

    #[test]
    fn ctrl_space_lists_the_words_of_the_document_and_enter_takes_one() {
        // A new line under a document that already says `alpha`, which is the
        // case completion exists for.
        let mut app = app("alpha beta\n\n");
        app.tabs[0].editor.goto_position(1, 0);
        for ch in "alph".chars() {
            app.handle(Key::Char(ch));
        }
        app.handle(Key::Ctrl(' '));
        assert!(app.complete.is_open(), "the chord opened the list");
        assert_eq!(
            app.complete.selected().map(|item| item.label.as_str()),
            Some("alpha")
        );
        // The box is drawn over the text, under the caret's row and column.
        let rows = drawn(&mut app, 30, 10);
        assert!(rows[2].contains('┌'), "{rows:?}");
        assert!(rows[2].contains('┐'), "{rows:?}");
        assert!(rows[3].contains("alpha"), "{rows:?}");
        app.handle(Key::Enter);
        assert!(!app.complete.is_open());
        assert_eq!(app.tab().editor.doc.text(), "alpha beta\nalpha\n");
        assert_eq!(caret(&app), (1, 5));
    }

    #[test]
    fn a_letter_narrows_the_open_list_and_a_space_ends_it() {
        let mut app = app("alpha alpine beta\n\n");
        app.tabs[0].editor.goto_position(1, 0);
        for ch in "al".chars() {
            app.handle(Key::Char(ch));
        }
        app.handle(Key::Ctrl(' '));
        let names: Vec<&str> = app
            .complete
            .items()
            .iter()
            .map(|item| item.label.as_str())
            .collect();
        assert_eq!(names, ["alpha", "alpine"]);
        // Typing `p` keeps both: they start with `alp`.
        app.handle(Key::Char('p'));
        assert!(app.complete.is_open());
        assert_eq!(app.complete.len(), 2);
        // A space is not a word character: the list goes and the space is typed.
        app.handle(Key::Char(' '));
        assert!(!app.complete.is_open());
        assert_eq!(app.tab().editor.doc.text(), "alpha alpine beta\nalp \n");
    }

    #[test]
    fn escape_puts_the_list_away_and_leaves_the_word_alone() {
        // A prefix that matches nothing opens no box and says nothing.
        let mut blank = app("alpha\n");
        blank.handle(Key::Ctrl(' '));
        assert!(!blank.complete.is_open());
        let mut app = app("alpha\n\n");
        app.tabs[0].editor.goto_position(1, 0);
        for ch in "al".chars() {
            app.handle(Key::Char(ch));
        }
        app.handle(Key::Ctrl(' '));
        assert!(app.complete.is_open());
        app.handle(Key::Escape);
        assert!(!app.complete.is_open());
        assert_eq!(app.tab().editor.doc.text(), "alpha\nal\n");
        assert_eq!(caret(&app), (1, 2));
    }

    #[test]
    fn typing_an_open_parenthesis_shows_the_signature_of_the_call() {
        let mut app = app("fn save(path: &Path) {}\n");
        app.tabs[0].editor.goto_position(1, 0);
        for ch in "save(".chars() {
            app.handle(Key::Char(ch));
        }
        assert!(app.hint.is_open(), "the parenthesis asked for the signature");
        assert_eq!(app.hint.text(), "save(path: &Path)");
        // And it is on the screen, under the caret's row.
        let rows = drawn(&mut app, 40, 8);
        assert!(
            rows.iter().any(|row| row.contains("save(path: &Path)")),
            "{rows:?}"
        );
        // The caret is in the first argument, which is the lit one.
        assert_eq!(app.hint.active(), Some(5..16));
        // Typing past the closing parenthesis leaves the call, so the hint goes.
        app.handle(Key::Char(')'));
        assert!(!app.hint.is_open());
    }

    #[test]
    fn the_menu_rows_step_the_lit_argument_without_moving_the_caret() {
        let mut app = app("fn save(a: u8, b: u8) {}\n");
        app.tabs[0].editor.goto_position(1, 0);
        for ch in "save(".chars() {
            app.handle(Key::Char(ch));
        }
        assert!(app.hint.is_open());
        let before = caret(&app);
        app.run(Action::ParamHintNext);
        assert_eq!(caret(&app), before, "the highlight walks, the caret does not");
        assert_eq!(app.hint.active(), Some(12..17));
        // And back again, wrapping as it does in the window.
        app.run(Action::ParamHintPrev);
        assert_eq!(app.hint.active(), Some(5..10));
        // The hint row is a toggle: it puts the hint away and brings it back.
        app.run(Action::ParamHint);
        assert!(!app.hint.is_open());
        app.run(Action::ParamHint);
        assert!(app.hint.is_open());
    }

    #[test]
    fn path_completion_lists_the_documents_own_directory() {
        // A document in a scratch directory with a file beside it: a relative
        // fragment is measured from the document, not from where the shell was
        // started, which is what makes the row usable.
        let dir = temp("complete-path");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("main.rs"), "x\n").expect("write");
        let path = dir.join("notes.txt");
        std::fs::write(&path, "open ./ma\n").expect("write");
        let mut app = App::open(Some(&path)).expect("open");
        app.handle(Key::LineEnd);
        app.run(Action::CompletePath);
        assert!(app.complete.is_open(), "a directory row was offered");
        assert_eq!(
            app.complete.selected().map(|item| item.label.as_str()),
            Some("main.rs")
        );
        app.handle(Key::Enter);
        assert!(!app.complete.is_open());
        assert_eq!(app.tab().editor.doc.text(), "open ./main.rs\n");
        // A caret with nothing before it on the line offers nothing, and says
        // nothing: an empty list is a closed list.
        let mut plain = App::open(Some(&path)).expect("open");
        plain.tabs[0].editor.goto_position(0, 4);
        plain.run(Action::CompletePath);
        assert!(!plain.complete.is_open());
        let _ = std::fs::remove_dir_all(&dir);
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
        app.editor().goto_position(0, 14);
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
        app.editor().goto_position(0, line.chars().count());
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
        assert_eq!(app.tab().editor.caret_column(), 10);
        app.handle(Key::Down);
        assert_eq!(app.tab().editor.caret_column(), 20);
        // The third row is five characters long, and the caret arrives at the
        // start of it: a row move keeps the column inside the row, which is zero.
        app.handle(Key::Down);
        assert_eq!(app.tab().editor.caret_column(), 20);
        // Which is also the last row of the document: one more Down stays put.
        app.handle(Key::Down);
        assert_eq!(app.tab().editor.caret_column(), 20);
        // And Up walks back, arriving on the same two columns.
        app.handle(Key::Up);
        assert_eq!(app.tab().editor.caret_column(), 10);
        app.handle(Key::Up);
        assert_eq!(app.tab().editor.caret_column(), 0);
        app.handle(Key::Up);
        assert_eq!(app.tab().editor.caret_column(), 0);
    }

    #[test]
    fn the_column_inside_the_row_survives_a_short_line() {
        let text = "0123456789ABCDEFGHIJKLMNO\nab\n0123456789ABCDEFGHIJKLMNO\n";
        let mut app = app(text);
        app.settings_mut().wrap = true;
        let mut grid = Grid::new(12, 4);
        app.draw(&mut grid);
        app.editor().goto_position(0, 3);
        app.draw(&mut grid);
        // Down the rows of the same line: three more cells along, ten in.
        app.handle(Key::Down);
        assert_eq!(caret(&app), (0, 13));
        app.handle(Key::Down);
        assert_eq!(caret(&app), (0, 23));
        // Down onto `ab`: the remembered column is three, the line has two.
        app.handle(Key::Down);
        assert_eq!(caret(&app), (1, 2));
        // And the memory is not the clamp: the next line gets its own row read
        // from the column that was asked for, not from the two cells it gave.
        app.handle(Key::Down);
        assert_eq!(caret(&app), (2, 3));
    }

    #[test]
    fn up_from_the_first_row_arrives_on_the_last_row_of_the_line_above() {
        let mut app = app("0123456789ABCDEF\nabcdefghij\n");
        app.settings_mut().wrap = true;
        let mut grid = Grid::new(12, 4);
        app.draw(&mut grid);
        app.editor().goto_position(1, 2);
        app.draw(&mut grid);
        // The line above takes two rows, so Up lands on its second row, two cells
        // in -- not at the end of its first.
        app.handle(Key::Up);
        assert_eq!(caret(&app), (0, 12));
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
        assert_eq!(caret(&app).0, 2, "a page of four rows is two lines");
        app.handle(Key::PageDown);
        assert_eq!(app.tab().editor.caret_line(), 4);
        app.handle(Key::PageUp);
        assert_eq!(app.tab().editor.caret_line(), 2);
    }

    #[test]
    fn with_wrapping_off_a_vertical_move_is_still_a_line() {
        let mut app = app("0123456789ABCDEFGHIJ\nab\n0123456789ABCDEFGHIJ\n");
        let mut grid = Grid::new(12, 4);
        app.draw(&mut grid);
        app.editor().goto_position(0, 3);
        app.draw(&mut grid);
        // The second line holds two characters, the third none of the first
        // line's tail: the core's motion clamps to each line and remembers the
        // column, and nothing here interfered with it.
        app.handle(Key::Down);
        assert_eq!(caret(&app), (1, 2));
        app.handle(Key::Down);
        assert_eq!(caret(&app), (2, 3));
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
        assert_eq!(app.tab().editor.caret_line(), 1);
        // And the same walk back up returns it to where it started, which is what
        // makes the two motions inverses of one another.
        for _ in 0..12 {
            app.handle(Key::Up);
            app.draw(&mut grid);
            assert!(caret_cell(&grid).is_some(), "the caret left the screen");
        }
        assert_eq!(app.tab().editor.caret(), 0);
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
        assert_eq!(app.tab().editor.doc.text(), "    let x = 1;\n    a\n");
    }

    #[test]
    fn a_chord_runs_the_command_the_settings_bind_it_to() {
        // Ctrl+D is `duplicate_line` in the default keymap and Ctrl+L is
        // `delete_line`; both act on the caret's line when nothing is selected.
        let mut app = app("one\ntwo\n");
        app.handle(Key::Ctrl('d'));
        assert_eq!(app.tab().editor.doc.text(), "one\none\ntwo\n");
        app.editor().goto_line(1);
        app.handle(Key::Ctrl('l'));
        assert_eq!(app.tab().editor.doc.text(), "one\ntwo\n");
    }

    #[test]
    fn a_chord_the_settings_do_not_bind_does_nothing() {
        let mut app = app("one\n");
        app.settings_mut().keymap.unbind(Action::DuplicateLine);
        app.apply_settings();
        app.handle(Key::Ctrl('d'));
        let text = app.tab().editor.doc.text();
        assert_eq!(text, "one\n", "an unbound chord edited the file");
    }

    #[test]
    fn the_text_commands_run_over_the_selected_lines() {
        let mut app = app("b\na\n");
        // A caret alone means "the caret's line", which is what the window does
        // too: these commands are about a selection, and the shell does not
        // invent one for them.
        app.run(Action::SelectAll);
        app.run(Action::UpperCase);
        assert_eq!(app.tab().editor.doc.text(), "B\nA\n");
        // An edit replaces what it was given, so the selection is gone after it:
        // the next command needs one of its own.
        app.run(Action::SelectAll);
        app.run(Action::SortAsc);
        assert_eq!(app.tab().editor.doc.text(), "A\nB\n");
        app.run(Action::SelectAll);
        // The engine indents with a tab, not with `tab_width` spaces: the
        // column a tab stands for is a display question, and the file keeps the
        // character that is one keystroke away from the one it replaces.
        app.run(Action::Indent);
        assert_eq!(app.tab().editor.doc.text(), "\tA\n\tB\n");
        app.run(Action::SelectAll);
        app.run(Action::Unindent);
        assert_eq!(app.tab().editor.doc.text(), "A\nB\n");
    }

    #[test]
    fn the_line_commands_move_and_delete() {
        let mut app = app("one\ntwo\n");
        app.run(Action::MoveLineDown);
        assert_eq!(app.tab().editor.doc.text(), "two\none\n");
        app.editor().goto_line(1);
        app.run(Action::MoveLineUp);
        assert_eq!(app.tab().editor.doc.text(), "one\ntwo\n");
        // The caret is left at the end of the block the move touched, which is
        // the empty line after the last newline; a delete needs it somewhere
        // with a line under it.
        app.editor().goto_line(0);
        app.run(Action::DeleteLine);
        assert_eq!(app.tab().editor.doc.text(), "two\n");
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
        assert_eq!(app.tab().editor.doc.text(), "one two one\n");
        // Wide enough for the pattern to be read beside the nine marks.
        let rows = drawn(&mut app, 48, 5);
        // The bar is the row above the status bar, and the text lost the row.
        assert!(rows[3].starts_with("Find: one"), "{:?}", rows);
        assert!(rows[3].ends_with("0/2 "), "{:?}", rows);
        assert!(rows[4].contains("Ln 1, Col 1"), "{:?}", rows);
        app.handle(Key::Enter);
        let rows = drawn(&mut app, 48, 5);
        assert!(rows[3].ends_with("1/2 "), "{:?}", rows);
        // Escape closes and leaves the match selected, and the text gets its row
        // back: the row the bar was on is a line of the document again.
        app.handle(Key::Escape);
        let rows = drawn(&mut app, 48, 5);
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
        assert_eq!(app.tab().editor.selection_start(), 0);
        app.handle(Key::Function(3));
        assert_eq!(app.tab().editor.selection_start(), 8);
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
        // Tab ten times: the seven marks, then the marks tab's two chips, then
        // the replacement field.
        for _ in 0..10 {
            app.handle(Key::Tab);
        }
        app.handle(Key::Char('1'));
        app.handle(Key::Enter);
        assert_eq!(app.tab().editor.doc.text().as_str(), "1 two one\n");
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
    fn the_mark_chip_bookmarks_the_line_a_match_lands_on() {
        let mut app = app("one\ntwo\n");
        app.run(Action::Find);
        for ch in "one".chars() {
            app.handle(Key::Char(ch));
        }
        // Tab stands on the marks; the eighth is `B`, the bookmark chip that is
        // the window's Mark tab read in a terminal.
        for _ in 0..8 {
            app.handle(Key::Tab);
        }
        app.handle(Key::Char(' '));
        // Back to the field, where Enter walks to the match.
        app.handle(Key::Tab);
        app.handle(Key::Tab);
        app.handle(Key::Enter);
        assert!(app.tab().editor.doc.is_bookmarked(0));
        // The dot is the one the margin already draws for a bookmark, so the
        // two ways of marking a line look alike.
        let rows = drawn(&mut app, 48, 6);
        assert!(rows[0].contains('\u{2022}'), "{:?}", rows);
    }

    #[test]
    fn a_bookmark_puts_a_dot_in_the_margin_and_f2_walks_to_it() {
        let mut app = app("one\ntwo\nthree\n");
        // F2 with nothing marked says so, rather than doing nothing in silence.
        app.handle(Key::Function(2));
        assert_eq!(app.message(), Some("no bookmarks"));
        // Ctrl+F2 marks the caret's line, and the margin says which one.
        app.handle(Key::CtrlFunction(2));
        assert!(app.tab().editor.doc.is_bookmarked(0));
        let rows = drawn(&mut app, 30, 8);
        assert!(rows[0].contains('\u{2022}'), "{:?}", rows);
        assert!(!rows[1].contains('\u{2022}'), "{:?}", rows);
        // Another line, another mark, and F2 comes back round to the first.
        app.handle(Key::Down);
        app.handle(Key::Down);
        app.handle(Key::CtrlFunction(2));
        assert!(app.tab().editor.doc.is_bookmarked(2));
        app.handle(Key::Function(2));
        assert_eq!(app.tab().editor.caret_line(), 0, "F2 wrapped past the end");
        // Shift+F2 walks the other way, and Clear All empties the margin.
        app.handle(Key::ShiftFunction(2));
        assert_eq!(app.tab().editor.caret_line(), 2);
        app.run(Action::ClearBookmarks);
        assert_eq!(app.tab().editor.doc.bookmark_count(), 0);
        let rows = drawn(&mut app, 30, 8);
        assert!(rows.iter().all(|row| !row.contains('\u{2022}')), "{:?}", rows);
    }

    #[test]
    fn a_mark_that_cannot_light_says_why_on_the_shells_own_line() {
        let mut app = app("one two one\n");
        app.run(Action::Find);
        for ch in "one".chars() {
            app.handle(Key::Char(ch));
        }
        // Seven tabs from the field: case, word, the three readings, wrap, and
        // the selection mark, which has nothing to stand on.
        for _ in 0..7 {
            app.handle(Key::Tab);
        }
        app.handle(Key::Char(' '));
        assert_eq!(app.message(), Some("select the text to search in"));
        // The bar kept the whole document: the count is still both matches.
        let rows = drawn(&mut app, 40, 6);
        assert!(rows[4].contains("0/2"), "{:?}", rows);
    }

    #[test]
    fn walking_past_the_last_match_wraps_in_the_shell_until_the_mark_is_pressed() {
        let mut app = app("one two one\n");
        app.run(Action::Find);
        for ch in "one".chars() {
            app.handle(Key::Char(ch));
        }
        // Down three times wraps round to the first match, which is what a
        // search does by default.
        for _ in 0..3 {
            app.handle(Key::Down);
        }
        assert_eq!(app.tab().editor.selection_start(), 0);
        // Wrap off: the sixth tab is that mark, and pressing it stops the wrap.
        for _ in 0..6 {
            app.handle(Key::Tab);
        }
        app.handle(Key::Char(' '));
        for _ in 0..3 {
            app.handle(Key::Down);
        }
        assert_eq!(
            app.tab().editor.selection_start(),
            8,
            "the search wrapped with the mark off"
        );
        let rows = drawn(&mut app, 40, 6);
        assert!(rows[4].contains("2/2"), "{:?}", rows);
    }

    #[test]
    fn a_regex_typed_in_the_bar_is_the_pattern_the_document_is_walked_by() {
        let mut app = app("a.c abc\n");
        app.run(Action::Find);
        for ch in "a.c".chars() {
            app.handle(Key::Char(ch));
        }
        // Five tabs from the field: case, word, and the three ways to read the
        // pattern. The last one is `.*`, and Space presses it.
        for _ in 0..5 {
            app.handle(Key::Tab);
        }
        app.handle(Key::Char(' '));
        // The bar takes the row above the status line: four rows of document,
        // then the pattern, then the bar's own line.
        let rows = drawn(&mut app, 40, 6);
        assert!(rows[4].contains("0/2"), "{:?}", rows);
        // The arrows walk the matches from any focus, and the second one is the
        // `abc` that only a regex reads as a match.
        app.handle(Key::Down);
        assert_eq!(app.tab().editor.selection_start(), 0);
        let rows = drawn(&mut app, 40, 6);
        assert!(rows[4].contains("1/2"), "{:?}", rows);
        app.handle(Key::Down);
        assert_eq!(app.tab().editor.selection_start(), 4);
        let _ = rows;
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
        assert_eq!(app.tab().editor.selection_start(), 201);
        app.handle(Key::Down);
        assert_eq!(app.tab().editor.selection_start(), 409);
    }

    /// A folder of two files that share the word `needle`, for the searches.
    ///
    /// Named per test rather than once: the tests of one binary are threads of
    /// one process, so a shared folder would be removed by whichever test
    /// finished first while the others were still searching it.
    fn search_folder(name: &str) -> std::path::PathBuf {
        let dir = temp(&format!("search-{name}"));
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("one.txt"), "nothing\nneedle here\n").expect("write");
        std::fs::write(dir.join("two.txt"), "needle again\n").expect("write");
        dir
    }

    #[test]
    fn find_in_files_lists_what_it_found_and_says_where_it_looked() {
        let dir = search_folder("find_in_files_lists_what_it_found_and_says_where_it_looked");
        let mut app = App::open(Some(&dir.join("one.txt"))).expect("open");
        prompt(&mut app, Action::FindInFiles, "needle");
        // The folder holding the document is the one searched, which is what the
        // bar says and what makes the panel's short rows readable.
        let message = app.message().unwrap_or_default().to_string();
        assert!(message.starts_with("searched "), "{message}");
        assert!(message.ends_with(": 2 hits"), "{message}");
        assert!(app.results.is_open());
        let rows = drawn(&mut app, 60, 12);
        let header = rows
            .iter()
            .position(|row| row.contains("2 results in 2 files"))
            .unwrap_or_else(|| panic!("no header: {rows:?}"));
        // Header first, then one row per hit, and the row names the file and
        // counts the line from one.
        assert!(rows[header + 1].contains("one.txt:2: needle here"), "{rows:?}");
        assert!(rows[header + 2].contains("two.txt:1: needle again"), "{rows:?}");
        // The document keeps the rows above the list.
        assert!(rows[0].contains("nothing"), "{rows:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_suffix_scopes_the_search_to_another_folder() {
        let dir = search_folder("the_suffix_scopes_the_search_to_another_folder");
        // A document outside the folder, and the pattern saying where to look:
        // the prompt's whole syntax, and the window's.
        let other = temp("search-elsewhere.txt");
        std::fs::write(&other, "no needle here\n").expect("write");
        let mut app = App::open(Some(&other)).expect("open");
        let typed = format!("needle in {}", dir.display());
        prompt(&mut app, Action::FindInFiles, &typed);
        let message = app.message().unwrap_or_default().to_string();
        assert!(message.contains(&dir.display().to_string()), "{message}");
        assert!(message.ends_with(": 2 hits"), "{message}");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&other);
    }

    #[test]
    fn a_hit_is_opened_where_the_match_is_and_puts_the_list_away() {
        let dir = search_folder("a_hit_is_opened_where_the_match_is_and_puts_the_list_away");
        let mut app = App::open(Some(&dir.join("one.txt"))).expect("open");
        prompt(&mut app, Action::FindInFiles, "needle");
        // The second row is the match in the other file: Enter opens it in a tab
        // of its own -- not a second one for a file already open -- and puts the
        // caret on the match.
        app.handle(Key::Down);
        app.handle(Key::Enter);
        assert!(!app.results.is_open(), "the list stayed on screen");
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.tab().editor.doc.path(), Some(dir.join("two.txt").as_path()));
        assert_eq!(app.tab().editor.caret_line(), 0);
        assert_eq!(app.tab().editor.caret_column(), 0);
        let message = app.message().unwrap_or_default().to_string();
        assert_eq!(message, "two.txt:1 (2/2)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn next_and_previous_search_result_walk_the_hits_and_wrap() {
        let dir = search_folder("next_and_previous_search_result_walk_the_hits_and_wrap");
        let mut app = App::open(Some(&dir.join("one.txt"))).expect("open");
        // Nothing to step through before a search, and the bar says so.
        app.run(Action::NextSearchResult);
        assert_eq!(app.message(), Some("no search results yet"));
        prompt(&mut app, Action::FindInFiles, "needle");
        app.handle(Key::Escape);
        assert!(!app.results.is_open());
        // The first step opens the first hit, the window's rule; the list stays
        // down while stepping, because reading the matches is what this is for.
        app.run(Action::NextSearchResult);
        assert_eq!(app.message(), Some("one.txt:2 (1/2)"));
        assert_eq!(app.tab().editor.caret_line(), 1);
        app.run(Action::NextSearchResult);
        assert_eq!(app.message(), Some("two.txt:1 (2/2)"));
        // Wrapping at both ends, and Previous from the second hit is the first.
        app.run(Action::NextSearchResult);
        assert_eq!(app.message(), Some("one.txt:2 (1/2)"));
        app.run(Action::PrevSearchResult);
        assert_eq!(app.message(), Some("two.txt:1 (2/2)"));
        assert!(!app.results.is_open(), "stepping opened the list");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_results_list_comes_back_from_the_menu_with_the_same_search() {
        let dir = search_folder("the_results_list_comes_back_from_the_menu_with_the_same_search");
        let mut app = App::open(Some(&dir.join("one.txt"))).expect("open");
        app.run(Action::SearchResultsWindow);
        assert_eq!(app.message(), Some("no search results yet"));
        prompt(&mut app, Action::FindInFiles, "needle");
        app.handle(Key::Escape);
        app.run(Action::SearchResultsWindow);
        assert_eq!(app.message(), Some("2 results in the list"));
        assert!(app.results.is_open());
        let rows = drawn(&mut app, 60, 12);
        assert!(rows.iter().any(|row| row.contains("2 results in 2 files")), "{rows:?}");
        // Escape closes it again, which is the only way out of a list that is not
        // modal: the arrows move its cursor while it is up.
        app.handle(Key::Escape);
        assert!(!app.results.is_open());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_pattern_is_refused_on_the_bar() {
        let mut app = app("x\n");
        // Whitespace is trimmed before the search, so a field of spaces is an
        // empty pattern: walked, it would match every line of every file.
        prompt(&mut app, Action::FindInFiles, "   ");
        assert_eq!(app.message(), Some("find in files: type a pattern"));
        assert!(!app.results.is_open());
    }

    #[test]
    fn a_command_the_shell_has_not_got_says_so() {
        let mut app = app("x\n");
        // The searches and the function list have arrived; the document map
        // has not.
        app.run(Action::DocumentMap);
        let message = app.message().unwrap_or_default();
        assert!(message.contains("not in the terminal yet"), "{message}");
    }

    /// A document with four declarations, one of them nested.
    fn declarations() -> App {
        app(concat!(
            "fn main() {\n",
            "    let x = 1;\n",
            "}\n",
            "fn helper() {\n",
            "}\n",
            "impl Thing {\n",
            "    fn method(&self) {\n",
            "    }\n",
            "}\n",
        ))
    }

    #[test]
    fn the_function_list_lists_what_the_document_declares() {
        let mut app = declarations();
        app.run(Action::ToggleFunctionList);
        assert!(app.symbols.is_open());
        let rows = drawn(&mut app, 60, 16);
        let header = rows
            .iter()
            .position(|row| row.trim() == "4 symbols")
            .unwrap_or_else(|| panic!("no header: {rows:?}"));
        // One row per declaration, in the order they are declared, and the
        // nested one indented: the window's rows and the window's indent,
        // because both read them from `nn-shared`.
        assert_eq!(rows[header + 1].trim_end(), " main");
        assert_eq!(rows[header + 2].trim_end(), " helper");
        assert_eq!(rows[header + 3].trim_end(), " Thing");
        assert_eq!(rows[header + 4].trim_end(), "   method");
        // The document keeps the rows above the list.
        assert!(rows[0].contains("fn main"), "{rows:?}");
    }

    #[test]
    fn a_row_of_the_function_list_puts_the_caret_on_the_declaration() {
        let mut app = declarations();
        app.run(Action::ToggleFunctionList);
        app.handle(Key::Down);
        app.handle(Key::Down);
        app.handle(Key::Enter);
        // The third row is the `impl`, which is declared on the sixth line.
        assert!(!app.symbols.is_open(), "the list stayed on screen");
        assert_eq!(app.tab().editor.caret_line(), 5);
        assert_eq!(app.tab().editor.caret_column(), 0);
        assert_eq!(app.message(), Some("Thing:6 (3/4)"));
    }

    #[test]
    fn a_document_with_nothing_to_list_says_so_and_opens_nothing() {
        let mut app = app("just text\n");
        app.run(Action::ToggleFunctionList);
        assert!(!app.symbols.is_open());
        assert_eq!(app.message(), Some("no symbols in this document"));
        // The command on an open list puts it away, as the window's toggle does.
        let mut app = declarations();
        app.run(Action::ToggleFunctionList);
        app.run(Action::ToggleFunctionList);
        assert!(!app.symbols.is_open());
    }

    /// Where `needle` starts in `row`, counted in characters rather than bytes.
    ///
    /// A row is a string of cells, and the border glyph and the ellipsis are
    /// three bytes each: a byte offset would name a column no reader has.
    fn column_of(row: &str, needle: &str) -> Option<usize> {
        let bytes = row.find(needle)?;
        Some(row.get(..bytes)?.chars().count())
    }

    #[test]
    fn a_recording_replays_the_edits_it_caught() {
        let mut app = app("x\n");
        app.run(Action::ToggleRecording);
        app.handle(Key::Char('a'));
        app.handle(Key::Char('b'));
        app.run(Action::ToggleRecording);
        assert_eq!(app.message(), Some("macro recorded, 2 steps"));
        assert_eq!(app.tab().editor.doc.text(), "abx\n");
        // The caret is where the typing left it, so the replay lands there: the
        // steps are editor calls, not keys.
        app.run(Action::PlayMacro);
        assert_eq!(app.message(), Some("macro played, 2 steps"));
        assert_eq!(app.tab().editor.doc.text(), "ababx\n");
    }

    #[test]
    fn a_recording_catches_what_enter_and_the_arrows_did() {
        let mut app = app("ab\n");
        app.run(Action::ToggleRecording);
        app.handle(Key::Enter);
        app.handle(Key::Right);
        app.handle(Key::Backspace);
        app.run(Action::ToggleRecording);
        // A newline with the auto-indent it carried, a motion and a delete.
        assert_eq!(app.recorder.len(), 3);
        assert_eq!(
            app.recorder.steps(),
            &[
                Recorded::Insert("\n".to_string()),
                Recorded::Move(Motion::CharRight, false),
                Recorded::Backspace,
            ]
        );
    }

    #[test]
    fn the_bar_says_rec_and_counts_the_steps_it_caught() {
        let mut app = app("x\n");
        app.run(Action::ToggleRecording);
        app.handle(Key::Char('a'));
        let rows = drawn(&mut app, 80, 4);
        assert!(rows[3].contains("REC 1"), "{:?}", rows[3]);
        // Stopped, the count goes and the bar says what was caught.
        app.run(Action::StopRecording);
        let rows = drawn(&mut app, 80, 4);
        assert!(!rows[3].contains("REC"), "{:?}", rows[3]);
        assert!(rows[3].contains("macro recorded, 1 steps"), "{:?}", rows[3]);
    }

    #[test]
    fn a_macro_nobody_recorded_says_so() {
        let mut app = app("x\n");
        app.run(Action::PlayMacro);
        assert_eq!(app.message(), Some("no macro recorded"));
        // Stopping a recording that is not running is a refusal too.
        app.run(Action::StopRecording);
        assert_eq!(app.message(), Some("not recording"));
        assert!(app.recorder.steps().is_empty());
    }

    #[test]
    fn a_macro_can_be_played_several_times_from_the_prompt() {
        let mut app = app("\n");
        app.run(Action::ToggleRecording);
        app.handle(Key::Char('z'));
        app.run(Action::StopRecording);
        prompt(&mut app, Action::RunMacroMultiple, "3");
        assert_eq!(app.message(), Some("macro played 3 time(s)"));
        assert_eq!(app.tab().editor.doc.text(), "zzzz\n");
        // A count that is not a number is refused in the bar rather than played.
        prompt(&mut app, Action::RunMacroMultiple, "many");
        assert_eq!(app.message(), Some("play macro times: type a number"));
    }

    /// A folder holding two files, for the workspace tests.
    fn workspace_folder(name: &str) -> PathBuf {
        let dir = temp(&format!("workspace-{name}"));
        std::fs::create_dir_all(dir.join("sub")).expect("mkdir");
        std::fs::write(dir.join("one.txt"), "the document\n").expect("write");
        std::fs::write(dir.join("two.txt"), "the other one\n").expect("write");
        std::fs::write(dir.join("sub/deep.txt"), "deeper\n").expect("write");
        dir
    }

    /// Presses `Down` until the tree's cursor is on the row named `name`.
    fn walk_to(app: &mut App, name: &str) {
        let count = app.workspace.row_count();
        for index in 0..count {
            let here = app
                .workspace
                .path_at(index)
                .and_then(|path| path.file_name().map(|n| n.to_string_lossy().into_owned()));
            if here.as_deref() == Some(name) {
                for _ in 0..index {
                    app.handle(Key::Down);
                }
                return;
            }
        }
        panic!("no row called {name} in {} rows", count);
    }

    #[test]
    fn the_workspace_dock_lists_the_folder_beside_the_text() {
        let dir = workspace_folder("dock");
        let mut app = App::open(Some(&dir.join("one.txt"))).expect("open");
        app.run(Action::FolderAsWorkspace);
        assert!(app.workspace.is_open());
        let rows = drawn(&mut app, 60, 12);
        // The dock takes a third of the width, its header says what the panel
        // is, and its last column is the border rather than a row of the tree.
        let dock = 60 / 3;
        // The title is cut to the column the dock got, as the window's is cut
        // to its own column: on a third of a terminal there is no room for the
        // count and the name both.
        assert!(rows[0].contains("Folder as"), "{rows:?}");
        assert!(rows[0].chars().nth(dock - 1) == Some('│'), "{rows:?}");
        assert!(rows.iter().any(|row| row.contains("two.txt")), "{rows:?}");
        // The document starts after the dock and its line number, and it is the
        // document the whole time: the dock takes columns, not the screen.
        let text = rows
            .iter()
            .position(|row| row.contains("the document"))
            .unwrap_or_else(|| panic!("no text: {rows:?}"));
        assert_eq!(column_of(&rows[text], "the document"), Some(dock + 2), "{rows:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_row_opens_where_it_stands_and_a_file_row_opens_a_tab() {
        let dir = workspace_folder("rows");
        let mut app = App::open(Some(&dir.join("one.txt"))).expect("open");
        app.run(Action::FolderAsWorkspace);
        // A folder nobody has opened yet, one level in: `Enter` opens it, and
        // the dock stays up because the reader is still walking it.
        walk_to(&mut app, "sub");
        let before = app.workspace.row_count();
        app.handle(Key::Enter);
        assert!(app.workspace.is_open(), "the dock went away");
        assert!(app.workspace.row_count() > before, "the folder stayed shut");
        // A file is opened in a tab of its own, and the dock goes: on a terminal
        // the tree holds the arrows while it is up.
        walk_to(&mut app, "two.txt");
        app.handle(Key::Enter);
        assert!(!app.workspace.is_open(), "the dock stayed up");
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.tab().editor.doc.path(), Some(dir.join("two.txt").as_path()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_letter_narrows_the_dock_and_never_reaches_the_document() {
        let dir = workspace_folder("filter");
        let mut app = App::open(Some(&dir.join("one.txt"))).expect("open");
        app.run(Action::FolderAsWorkspace);
        let all = app.workspace.row_count();
        let text = app.tab().editor.doc.text();
        // `de` is in `deep.txt` and nowhere in the folder's own name, so the
        // tree narrows to that file and the path down to it.
        app.handle(Key::Char('d'));
        app.handle(Key::Char('e'));
        assert_eq!(app.tab().editor.doc.text(), text, "the letter was typed");
        assert_eq!(app.workspace.row_count(), 3, "the rows did not narrow");
        let rows = drawn(&mut app, 60, 12);
        assert!(rows[1].contains("Filter: de_"), "{rows:?}");
        // One Escape is the filter's, the next is the dock's.
        app.handle(Key::Escape);
        assert!(app.workspace.is_open(), "the dock went with the filter");
        assert_eq!(app.workspace.row_count(), all);
        app.handle(Key::Escape);
        assert!(!app.workspace.is_open(), "the dock stayed up");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_can_be_chosen_from_the_prompt() {
        let dir = workspace_folder("prompt");
        let mut app = app("just text\n");
        prompt(&mut app, Action::OpenFolderAsWorkspace, &dir.display().to_string());
        assert!(app.workspace.is_open());
        assert_eq!(
            app.message(),
            Some(&*format!("workspace: {}", dir.display()))
        );
        // A path that is not a folder is refused rather than shown as an empty
        // tree, which is what the window says.
        let file = dir.join("one.txt");
        prompt(&mut app, Action::OpenFolderAsWorkspace, &file.display().to_string());
        let message = app.message().unwrap_or_default().to_string();
        assert!(message.ends_with("is not a folder"), "{message}");
        // And the command that shows the document's folder closes it again.
        app.run(Action::FolderAsWorkspace);
        assert!(!app.workspace.is_open(), "the dock stayed up");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_two_lists_share_the_rows_above_the_status_bar() {
        let mut app = declarations();
        app.results.set_hits(vec![nn_core::Hit {
            path: PathBuf::from("/tmp/notes.txt"),
            line: 0,
            column: 0,
            text: "a hit".to_string(),
            tab: None,
        }]);
        app.run(Action::ToggleFunctionList);
        assert!(app.symbols.is_open());
        assert!(!app.results.is_open(), "both lists claimed the rows");
        let rows = drawn(&mut app, 60, 16);
        assert!(rows.iter().any(|row| row.trim() == "4 symbols"), "{rows:?}");
        assert!(!rows.iter().any(|row| row.contains("1 result")), "{rows:?}");
    }

    #[test]
    fn unsaved_changes_take_two_presses_of_ctrl_q() {
        let mut app = app("text");
        app.handle(Key::Char('!'));
        assert!(app.tab().editor.doc.is_dirty());
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
        assert!(app.tab().editor.doc.is_dirty());
        app.handle(Key::Ctrl('s'));
        let dirty = app.tab().editor.doc.is_dirty();
        assert!(!dirty, "{}", app.message().unwrap_or(""));
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "one!\n");
        let _ = std::fs::remove_file(&path);
    }

    /// A path in the temporary directory, named after this test binary.
    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("npp-tui-{}-{name}", std::process::id()))
    }

    /// Opens the prompt `action` asks for and types `text` into it.
    fn prompt(app: &mut App, action: Action, text: &str) {
        app.run(action);
        for ch in text.chars() {
            app.handle(Key::Char(ch));
        }
        app.handle(Key::Enter);
    }

    #[test]
    fn ctrl_o_opens_the_file_that_was_typed() {
        let path = temp("open.txt");
        std::fs::write(&path, "one\ntwo\n").expect("write");
        let mut app = app("first\n");
        app.handle(Key::Ctrl('o'));
        // The prompt says what it wants, in the window's words.
        let rows = drawn(&mut app, 40, 6);
        assert!(rows[4].starts_with("Open:"), "{:?}", rows);
        for _ in 0..5 {
            app.handle(Key::Backspace);
        }
        for ch in path.display().to_string().chars() {
            app.handle(Key::Char(ch));
        }
        app.handle(Key::Enter);
        assert_eq!(app.tab().editor.doc.path(), Some(path.as_path()));
        assert_eq!(app.tab().editor.doc.text().as_str(), "one\ntwo\n");
        assert_eq!(app.tab().editor.caret(), 0, "the caret did not go to the top");
        assert!(app.message().unwrap_or_default().starts_with("opened"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_name_that_is_not_there_opens_a_blank_page_for_it() {
        let path = temp("not-yet.txt");
        let _ = std::fs::remove_file(&path);
        let app = App::open(Some(&path)).expect("a missing name is not an error");
        assert_eq!(app.tab().editor.doc.path(), Some(path.as_path()));
        assert!(!app.tab().editor.doc.is_dirty(), "the blank page started dirty");
        assert_eq!(app.tab().editor.doc.text().as_str(), "");
    }

    #[test]
    fn save_as_writes_the_file_and_adopts_the_name() {
        let path = temp("save-as.txt");
        let _ = std::fs::remove_file(&path);
        let mut app = app("one\n");
        prompt(&mut app, Action::SaveAs, &path.display().to_string());
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "one\n");
        assert_eq!(app.tab().editor.doc.path(), Some(path.as_path()));
        assert!(!app.tab().editor.doc.is_dirty());
        assert!(app.message().unwrap_or_default().starts_with("saved"));
        // Once the message has been cleared by the next keystroke, the bar names
        // the file the buffer now holds.
        app.handle(Key::Down);
        let rows = drawn(&mut app, 60, 4);
        let name = path.file_name().expect("a file name").to_string_lossy();
        assert!(rows[3].contains(&*name), "{:?}", rows);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn saving_under_a_new_name_changes_the_language_with_it() {
        let path = temp("language.json");
        let _ = std::fs::remove_file(&path);
        let mut app = app("fn main() {}\n");
        // The caret stands on the `f`, and the caret's tone is drawn over the
        // text's, so it is moved out of the way first.
        app.handle(Key::LineEnd);
        let mut grid = Grid::new(40, 3);
        app.draw(&mut grid);
        // The document has no name, so it is drawn as code, and `fn` is a keyword.
        assert_eq!(grid.get(0, 2).tone, Tone::Keyword);
        prompt(&mut app, Action::SaveAs, &path.display().to_string());
        let mut grid = Grid::new(40, 3);
        app.draw(&mut grid);
        assert_ne!(
            grid.get(0, 2).tone,
            Tone::Keyword,
            "the buffer kept the lexer of the name it used to have"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn new_gives_a_blank_page_in_a_tab_of_its_own() {
        let path = temp("new.txt");
        std::fs::write(&path, "one\n").expect("write");
        let mut app = App::open(Some(&path)).expect("open");
        app.run(Action::New);
        assert_eq!(app.tab().editor.doc.path(), None);
        assert_eq!(app.tab().editor.doc.text().as_str(), "");
        // The file that was open is still open, with its own text: that is what
        // the window's `New` does and what this now does too.
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.tabs[0].editor.doc.text().as_str(), "one\n");
        // The strip names both documents and the text has moved down a row to
        // make room for it; the bar keeps naming the one in front.
        let rows = drawn(&mut app, 40, 4);
        assert!(rows[0].contains("new.txt"), "{:?}", rows);
        assert!(rows[0].contains("[No Name]"), "{:?}", rows);
        assert!(rows[1].starts_with('1'), "{:?}", rows);
        assert!(rows[3].trim_start().starts_with("[No Name]"), "{:?}", rows);
        // A second `New` is a third tab, and the first two keep what they had --
        // including the character typed into the second one.
        app.handle(Key::Char('x'));
        app.run(Action::New);
        assert_eq!(app.tabs.len(), 3);
        assert_eq!(app.tabs[1].editor.doc.text().as_str(), "x");
        assert!(app.tabs[1].editor.doc.is_dirty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_second_document_does_not_cost_the_first_its_changes() {
        let first = temp("tab-first.txt");
        let second = temp("tab-second.txt");
        std::fs::write(&first, "one\n").expect("write");
        std::fs::write(&second, "two\n").expect("write");
        let mut app = App::open(Some(&first)).expect("open");
        app.handle(Key::Char('X'));
        // Opening while the buffer is dirty used to be refused; a tab is the
        // answer that loses nothing, and it is the window's.
        prompt(&mut app, Action::Open, &second.display().to_string());
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.tab().editor.doc.text().as_str(), "two\n");
        assert_eq!(app.tabs[0].editor.doc.text().as_str(), "Xone\n");
        assert!(app.tabs[0].editor.doc.is_dirty());
        // `Ctrl+PageUp` goes back, and the edit and the caret are both there.
        app.handle(Key::CtrlPageUp);
        assert_eq!(app.tab().editor.doc.text().as_str(), "Xone\n");
        assert_eq!(caret(&app), (0, 1));
        let rows = drawn(&mut app, 100, 4);
        assert!(rows[0].contains("tab-first.txt"), "{:?}", rows);
        assert!(rows[0].contains("tab-second.txt"), "{:?}", rows);
        assert!(rows[1].starts_with("1 Xone"), "{:?}", rows);
        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);
    }

    #[test]
    fn the_strip_names_the_documents_and_lights_the_one_in_front() {
        let first = temp("strip-first.txt");
        let second = temp("strip-second.txt");
        std::fs::write(&first, "one\n").expect("write");
        std::fs::write(&second, "two\n").expect("write");
        let mut app = App::open(Some(&first)).expect("open");
        // One document is no strip at all, so its first line is on row nought.
        let rows = drawn(&mut app, 40, 4);
        assert!(rows[0].starts_with("1 one"), "{:?}", rows);
        prompt(&mut app, Action::Open, &second.display().to_string());
        // Wide enough for both names: the two temporary files carry the process
        // id in their names, and the strip drops what does not fit.
        let rows = drawn(&mut app, 80, 4);
        assert!(rows[0].contains("strip-first.txt"), "{:?}", rows);
        assert!(rows[0].contains("strip-second.txt"), "{:?}", rows);
        // The one in front wears the tone an open menu's title wears, and the
        // other one the strip's own; both are the window's menu colours.
        let mut grid = Grid::new(80, 4);
        app.draw(&mut grid);
        let strip: String = (0..80).map(|column| grid.get(0, column).ch).collect();
        let tone_of = |needle: &str| {
            let at = strip.find(needle).expect("on the strip");
            grid.get(0, at).tone
        };
        assert_eq!(tone_of("strip-first.txt"), Tone::Menu);
        assert_eq!(tone_of("strip-second.txt"), Tone::MenuHover);
        // Unsaved changes put a `*` after the name, which is the mark the bar
        // uses as well.
        app.handle(Key::Char('!'));
        let rows = drawn(&mut app, 80, 4);
        assert!(rows[0].contains("strip-second.txt*"), "{:?}", rows);
        // A strip wider than the screen keeps the document in front whole and
        // drops the name to its left, because the one being typed into is the one
        // that must not be hidden. Forty columns hold the second name and not the
        // two of them.
        let mut narrow = Grid::new(40, 4);
        app.draw(&mut narrow);
        let row: String = (0..40).map(|column| narrow.get(0, column).ch).collect();
        assert!(row.contains("strip-second.txt*"), "{row:?}");
        assert!(!row.contains("first"), "{row:?}");
        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);
    }

    #[test]
    fn a_wrapped_line_folds_below_the_strip_and_not_into_the_bar() {
        let mut app = app("short\n");
        app.settings_mut().wrap = true;
        app.apply_settings();
        app.run(Action::New);
        // One long line in the second document: sixty-one cells over a text area
        // thirty-eight wide, so two rows of it.
        app.handle(Key::Char('x'));
        app.tabs[app.active].editor.insert(&"0123456789".repeat(6));
        let mut grid = Grid::new(40, 4);
        app.draw(&mut grid);
        let row = |r: usize| -> String { (0..40).map(|c| grid.get(r, c).ch).collect() };
        assert!(row(0).contains("[No Name]"), "the strip is there: {:?}", row(0));
        // The line's number is on its first row and its continuation has none, so
        // the second row of the line starts in the text rather than the gutter.
        assert!(row(1).starts_with("1 "), "{:?}", row(1));
        assert!(row(2).starts_with("  "), "{:?}", row(2));
        // The caret is on the second row of the line, one row below the strip, and
        // the bar keeps the last row whatever the text does.
        let caret = caret_cell(&grid).expect("the caret is on screen");
        assert_eq!(caret.0, 2, "the caret follows the folded row");
        assert!(row(3).contains("Ln 1"), "the bar: {:?}", row(3));
    }

    #[test]
    fn the_page_chords_walk_the_documents_and_wrap() {
        let mut app = app("one\ntwo\n");
        // One document is nowhere to go, and a chord that looks broken is worse
        // than a sentence.
        app.handle(Key::CtrlPageDown);
        assert_eq!(app.tabs.len(), 1);
        let message = app.message().unwrap_or_default().to_string();
        assert!(message.contains("only one document"), "{message}");
        app.run(Action::New);
        app.handle(Key::CtrlPageDown);
        assert_eq!(app.active, 0, "two tabs wrap from the second to the first");
        app.handle(Key::CtrlPageDown);
        assert_eq!(app.active, 1);
        app.handle(Key::CtrlPageUp);
        assert_eq!(app.active, 0);
        // The chords are the window's, so the arrow-and-page pair is also on the
        // View menu, and the menu answers them through the same command.
        app.run(Action::PrevTab);
        assert_eq!(app.active, 1);
    }

    #[test]
    fn a_file_that_was_opened_goes_on_the_recent_list_and_the_file_menu() {
        let path = temp("recent-a.txt");
        std::fs::write(&path, "one\n").expect("write");
        let mut app = app("first\n");
        // Nothing has been opened yet, so the File menu is the short one.
        app.handle(Key::Function(9));
        app.handle(Key::Down);
        let rows = drawn(&mut app, 60, 30);
        assert!(!rows.iter().any(|row| row.contains("recent-a.txt")), "{rows:?}");
        app.handle(Key::Escape);
        prompt(&mut app, Action::Open, &path.display().to_string());
        assert_eq!(app.session.recent.first(), Some(&path));
        // The copy read at startup is untouched by an open: what gets written is
        // `session_now`, which is the tabs this shell has at exit.
        assert!(app.session.tabs.is_empty());
        // And the bar's File menu now carries the row, above `Reload from Disk`.
        app.handle(Key::Function(9));
        app.handle(Key::Down);
        let rows = drawn(&mut app, 60, 30);
        let at = rows
            .iter()
            .position(|row| row.contains("recent-a.txt"))
            .unwrap_or_else(|| panic!("no recent row: {rows:?}"));
        let reload = rows
            .iter()
            .position(|row| row.contains("Reload from Disk"))
            .expect("the menu is drawn");
        assert!(at < reload, "a recent row comes before Reload: {rows:?}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_recent_rows_open_their_file_and_clearing_empties_the_list() {
        let first = temp("recent-b.txt");
        let second = temp("recent-c.txt");
        std::fs::write(&first, "one\n").expect("write");
        std::fs::write(&second, "two\n").expect("write");
        let mut app = app("first\n");
        prompt(&mut app, Action::Open, &first.display().to_string());
        prompt(&mut app, Action::Open, &second.display().to_string());
        // Most recent first, which is the order the rows are in.
        assert_eq!(app.session.recent.first(), Some(&second));
        assert_eq!(app.session.recent.get(1), Some(&first));
        // The row opens the file and comes to front, rather than opening it twice.
        let tabs = app.tabs.len();
        app.run(Action::OpenRecent(1));
        assert_eq!(app.tabs.len(), tabs);
        assert_eq!(app.tab().editor.doc.path(), Some(first.as_path()));
        app.run(Action::ClearRecent);
        assert!(app.session.recent.is_empty());
        assert_eq!(app.message(), Some("the recent list is empty"));
        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);
    }

    #[test]
    fn the_tabs_a_session_remembers_come_back_in_order_with_their_carets() {
        let first = temp("restore-a.txt");
        let second = temp("restore-b.txt");
        std::fs::write(&first, "one\ntwo\nthree\n").expect("write");
        std::fs::write(&second, "four\n").expect("write");
        let mut restored = app("");
        restored.session = Session {
            recent: Vec::new(),
            tabs: vec![
                SessionTab {
                    path: first.clone(),
                    line: 2,
                    column: 1,
                    pinned: true,
                    colour: TabColour::Yellow,
                },
                // A file that is gone is skipped, and the tabs after it still
                // open: a stale entry is a convenience lost, not an error.
                SessionTab {
                    path: temp("restore-gone.txt"),
                    line: 0,
                    column: 0,
                    pinned: false,
                    colour: TabColour::None,
                },
                SessionTab {
                    path: second.clone(),
                    line: 0,
                    column: 0,
                    pinned: false,
                    colour: TabColour::None,
                },
            ],
            active: 2,
        };
        restored.restore_tabs();
        assert_eq!(restored.tabs.len(), 2, "the missing file is skipped");
        assert_eq!(restored.tabs[0].editor.doc.path(), Some(first.as_path()));
        assert_eq!(restored.tabs[1].editor.doc.path(), Some(second.as_path()));
        // The caret is where the session left it: the third line, second
        // character.
        assert_eq!(restored.tabs[0].editor.caret_line(), 2);
        assert_eq!(restored.tabs[0].editor.caret_column(), 1);
        // The index counts the tabs that survived, not the ones that did not.
        assert_eq!(restored.active, 1);
        assert_eq!(restored.tab().editor.doc.path(), Some(second.as_path()));
        // A session that restores nothing leaves the blank page in place.
        let mut empty = app("");
        empty.session = Session {
            tabs: vec![SessionTab {
                path: temp("restore-gone-too.txt"),
                line: 0,
                column: 0,
                pinned: false,
                colour: TabColour::None,
            }],
            active: 0,
            ..Session::default()
        };
        empty.restore_tabs();
        assert_eq!(empty.tabs.len(), 1);
        assert_eq!(empty.tab().editor.doc.path(), None);
        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);
    }

    #[test]
    fn what_the_shell_writes_is_the_tabs_it_has_where_they_stand() {
        let first = temp("now-a.txt");
        let second = temp("now-b.txt");
        std::fs::write(&first, "one\n").expect("write");
        std::fs::write(&second, "two\n").expect("write");
        let mut app = App::open(Some(&first)).expect("open");
        prompt(&mut app, Action::Open, &second.display().to_string());
        app.tabs[1].editor.goto_position(0, 2);
        // A buffer with no file has nothing to reopen, so it is not written --
        // and the tab that was in front before it keeps the front.
        app.run(Action::New);
        let session = app.session_now();
        assert_eq!(session.tabs.len(), 2, "the unnamed buffer is skipped");
        assert_eq!(session.tabs[0].path, first);
        assert_eq!(session.tabs[1].path, second);
        assert_eq!(session.tabs[1].column, 2);
        assert_eq!(session.active, 1);
        // A pin and a colour the window set survive the trip through a shell
        // that cannot set either: they are read back out of the copy it loaded.
        app.session.tabs = vec![SessionTab {
            path: first.clone(),
            line: 0,
            column: 0,
            pinned: true,
            colour: TabColour::Purple,
        }];
        let session = app.session_now();
        assert!(session.tabs[0].pinned);
        assert_eq!(session.tabs[0].colour, TabColour::Purple);
        assert!(!session.tabs[1].pinned);
        assert_eq!(session.tabs[1].colour, TabColour::None);
        // The recent list is the copy read at startup, kept whole.
        assert_eq!(session.recent, app.session.recent);
        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);
    }

    #[test]
    fn opening_a_file_that_is_already_open_switches_to_it() {
        let first = temp("again-first.txt");
        let second = temp("again-second.txt");
        std::fs::write(&first, "one\n").expect("write");
        std::fs::write(&second, "two\n").expect("write");
        let mut app = App::open(Some(&first)).expect("open");
        prompt(&mut app, Action::Open, &second.display().to_string());
        assert_eq!(app.tabs.len(), 2);
        // The same file again: no third tab, and the one that has it comes
        // forward rather than a second buffer writing over the first.
        prompt(&mut app, Action::Open, &first.display().to_string());
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active, 0);
        let message = app.message().unwrap_or_default().to_string();
        assert!(message.starts_with("already open:"), "{message}");
        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);
    }

    #[test]
    fn closing_a_document_refuses_unsaved_work_and_keeps_the_rest() {
        let first = temp("close-first.txt");
        let second = temp("close-second.txt");
        std::fs::write(&first, "one\n").expect("write");
        std::fs::write(&second, "two\n").expect("write");
        let mut app = App::open(Some(&first)).expect("open");
        prompt(&mut app, Action::Open, &second.display().to_string());
        // A dirty buffer is refused: there is no save prompt behind it.
        app.handle(Key::Char('!'));
        app.run(Action::CloseTab);
        assert_eq!(app.tabs.len(), 2);
        assert!(app.message().unwrap_or_default().contains("unsaved"));
        // Saved, it goes, and the neighbour takes its place.
        app.run(Action::Save);
        app.run(Action::CloseTab);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.tab().editor.doc.text().as_str(), "one\n");
        let message = app.message().unwrap_or_default().to_string();
        assert!(message.starts_with("closed "), "{message}");
        // The last one leaves a blank page rather than an empty strip.
        app.run(Action::CloseTab);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.tab().editor.doc.path(), None);
        assert_eq!(app.tab().editor.doc.text().as_str(), "");
        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);
    }

    #[test]
    fn close_all_stops_on_unsaved_work_and_collapses_otherwise() {
        let written = temp("close-all.txt");
        std::fs::write(&written, "one\n").expect("write");
        let mut app = app("plain\n");
        prompt(&mut app, Action::Open, &written.display().to_string());
        app.handle(Key::Char('x'));
        app.run(Action::CloseAll);
        assert_eq!(app.tabs.len(), 2, "one dirty buffer stops the whole command");
        let message = app.message().unwrap_or_default().to_string();
        assert!(message.contains("unsaved changes in 1 document"), "{message}");
        // Saved, the command goes ahead and leaves one blank page behind: the
        // window keeps a tab open for the same reason, an editor with no
        // document has nowhere to type.
        app.run(Action::Save);
        app.run(Action::CloseAll);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.tab().editor.doc.path(), None);
        let message = app.message().unwrap_or_default().to_string();
        assert!(message.contains("closed 2 documents"), "{message}");
        let _ = std::fs::remove_file(&written);
    }

    #[test]
    fn reload_reads_the_file_again_and_says_what_it_dropped() {
        // A buffer with no file has nothing to read again.
        let mut unnamed = app("text\n");
        unnamed.run(Action::Reload);
        let message = unnamed.message().unwrap_or_default().to_string();
        assert!(message.contains("nothing to reload"), "{message}");
        let path = temp("reload.txt");
        std::fs::write(&path, "one\n").expect("write");
        let mut app = App::open(Some(&path)).expect("open");
        std::fs::write(&path, "two\nthree\n").expect("rewrite");
        app.run(Action::Reload);
        assert_eq!(app.tab().editor.doc.text().as_str(), "two\nthree\n");
        assert!(!app.tab().editor.doc.is_dirty());
        assert!(app.message().unwrap_or_default().starts_with("reloaded"));
        // What was typed is dropped, with the bar saying so.
        app.handle(Key::Char('!'));
        app.run(Action::Reload);
        assert_eq!(app.tab().editor.doc.text().as_str(), "two\nthree\n");
        let message = app.message().unwrap_or_default().to_string();
        assert!(message.contains("were dropped"), "{message}");
        let _ = std::fs::remove_file(&path);
        // A file deleted since it was opened is an error, not a blank page.
        app.run(Action::Reload);
        let message = app.message().unwrap_or_default().to_string();
        assert!(message.contains("cannot reload"), "{message}");
        assert_eq!(app.tab().editor.doc.text().as_str(), "two\nthree\n");
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

        // The other shape the shell takes: eight rows of search results over the
        // document. The list repaints its own rows and nothing else, so the cost
        // of putting it up is a band of the screen rather than a screen.
        app.results.set_hits(
            (0..8)
                .map(|line| nn_core::Hit {
                    path: PathBuf::from("/tmp/notes.txt"),
                    line,
                    column: 0,
                    text: format!("the line the search found, number {line}"),
                    tab: None,
                })
                .collect(),
        );
        let start = std::time::Instant::now();
        app.draw(frame.grid());
        let list = frame.render().len();
        let list_micros = start.elapsed().as_nanos() as f64 / 1000.0;
        println!("opening the results list {list} bytes in {list_micros:.1} us");
        assert!(list < paint, "the list cost {list} bytes of a {paint}-byte screen");

        // And the function list's shape: one row per declaration, over a
        // document that declares five hundred of them. The scan runs once, on
        // the command -- no keystroke pays for it -- and the frame that puts the
        // rows up repaints the band they cover.
        let declarations: String = (0..500).map(|n| format!("fn value_{n}() {{}}\n")).collect();
        let mut listing = App::with_document(Document::from_text(&declarations));
        *listing.settings_mut() = Settings::default();
        listing.apply_settings();
        let mut listed = Frame::new(120, 40);
        listing.draw(listed.grid());
        let before = listed.render().len();
        let start = std::time::Instant::now();
        listing.run(Action::ToggleFunctionList);
        assert!(listing.symbols.is_open(), "the scan found nothing to list");
        listing.draw(listed.grid());
        let rows = listed.render().len();
        let rows_micros = start.elapsed().as_nanos() as f64 / 1000.0;
        println!("opening the function list {rows} bytes in {rows_micros:.1} us");
        assert!(rows < before, "the list cost {rows} bytes of a {before}-byte screen");

        // The third shape: a completion box beside the caret. Same discipline --
        // the box repaints the rows inside its own frame, so a chord that offers
        // ten candidates costs a band of the screen and not a screen.
        let words: String = (0..200).map(|n| format!("value_{n} ")).collect();
        let mut typing = App::with_document(Document::from_text(&words));
        *typing.settings_mut() = Settings::default();
        typing.apply_settings();
        typing.tabs[0].editor.goto_position(0, 6);
        typing.draw(frame.grid());
        frame.render();
        typing.handle(Key::Ctrl(' '));
        assert!(typing.complete.is_open(), "the box is what is being measured");
        let start = std::time::Instant::now();
        typing.draw(frame.grid());
        let box_bytes = frame.render().len();
        let box_micros = start.elapsed().as_nanos() as f64 / 1000.0;
        println!("opening the completion list {box_bytes} bytes in {box_micros:.1} us");
        assert!(
            box_bytes < paint,
            "the box cost {box_bytes} bytes of a {paint}-byte screen"
        );

        // And the parameter hint, which is three rows: one of signature between
        // two of border. Same band, same rule.
        typing.complete.close();
        typing.draw(frame.grid());
        frame.render();
        typing.hint.show(nn_shared::hint::Hint {
            signature: "save(path: &Path, force: bool)".to_string(),
            active: Some(5..16),
        });
        let start = std::time::Instant::now();
        typing.draw(frame.grid());
        let hint_bytes = frame.render().len();
        let hint_micros = start.elapsed().as_nanos() as f64 / 1000.0;
        println!("opening the parameter hint {hint_bytes} bytes in {hint_micros:.1} us");
        assert!(
            hint_bytes < paint,
            "the hint cost {hint_bytes} bytes of a {paint}-byte screen"
        );

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
