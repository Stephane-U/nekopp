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

//! Neko++ — a GPU-rendered Notepad++ workalike.
//!
//! The program is `npp` and the product is Neko++. The crate is `nn-app`: `nn`
//! names the workspace and `npp` names the program, so the two cannot drift
//! apart by accident.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use nn_app::{
    AboutBox, Action, Browser, BrowserKind, Catalog, ChangeHistory, ChangeRow, CharPanel, Chord, Chrome,
    CompletionMode,
    CompletionPopup, ContextMenu,
    DocumentPeeker, EditorView, Entry, FileDialog,
    FindBar, FindDialog, FolderPanel, FunctionList, Gpu, Import,
    Item, KeyName, Keymap, MatchMode, MenuBar, Mode, Palette, PaneRef, ParamHint, PluginList,
    PluginMenu, PluginRow, Plugins, Playground, Preferences,
    PrintPreview,
    PromptBar,
    PromptKind,
    Renderer,
    ResultsPanel, Session,
    SessionTab, Settings, Shortcuts, Snapshot, SnapshotEntry, Split, StatusBar, StyleConfigurator,
    TabBar, TabColour, TabInfo, TabLayout, Theme, ThemeChoice, Toolbar, Tooltip, WindowSort,
};
use nn_app::context;
use nn_app::plugin::Context;
use nn_app::renderer::UiMetrics;
use nn_app::layout::{self, Rect};
use nn_app::nppimport;
use nn_app::paths::{complete_path, resolve_dir, split_fragment};
use nn_core::backup::BackupMode;
use nn_core::fold::BraceFolder;
use nn_core::run::{RunCommand, RunContext};
use nn_core::{
    arg_range, call_at, collect_words, collect_words_many, file_stamp, lines, matching_words,
    now_secs, params_of, path_prefix, prefix_range, scan_functions, scan_symbols, split_query,
    CLikeLexer, ConfigLexer, DiskChange, Document, Editor, Encoding, EolMode, FindInFiles,
    Highlighter, Lexer, Macro, Motion, PlainLexer, Projects, Step, Tree, Typed, UdlSet, MAX_CANDIDATES,
};
use winit::platform::startup_notify::{EventLoopExtStartupNotify, WindowAttributesExtStartupNotify};
use winit::platform::x11::WindowAttributesExtX11;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    keyboard::{Key, ModifiersState, NamedKey},
    window::{CursorIcon, Fullscreen, Window, WindowId, WindowLevel},
};

/// How far the pointer has to travel before a press on a tab becomes a drag.
///
/// The platform's own double-click slop, for the same reason: a click that is
/// meant to select a tab must not move it, and a hand resting on a button moves
/// a pixel or two while it decides.
const TAB_DRAG_SLOP: f64 = 6.0;

/// How long two presses count as one click, and how far apart they may land.
///
/// Half a second and four pixels, which is what every desktop uses for a double
/// click. A press that misses either bound starts a new click, so a slow or a
/// wandering double click selects nothing rather than the wrong word; both
/// bounds are measured from the press before it, not from the first of a run, so
/// a triple click has to be three presses in a row that all pass.
const CLICK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);
const CLICK_SLOP: f64 = 4.0;

/// How often the open files are checked for changes made by another program.
///
/// One second is the interval every editor that polls uses, and it is well under
/// the time it takes a human to notice a stale buffer.
const DISK_POLL: std::time::Duration = std::time::Duration::from_millis(1000);

/// How long a key that arrives right after the window regained the focus is
/// treated as part of the window switch rather than as typing.
///
/// A window switcher hands back the very key that made the switch. `Alt+Tab` is
/// a grab the window manager owns, so the client never sees the `Tab` -- until
/// the switch commits and the grab is released, and then a `Tab` press that was
/// already in the queue is delivered to the window that just got the focus. It
/// physically predates the switch, so it belongs to the switch, and the editor
/// that takes it for typing indents over the selection the user just copied.
///
/// The measured gap is two milliseconds -- the same instant, from the loop's
/// point of view -- while the first key of someone who switched windows on
/// purpose comes hundreds of milliseconds later, after their hand has moved. So
/// a small number is enough, and a big one would eat real typing.
const LEAKED_KEY_GRACE: std::time::Duration = std::time::Duration::from_millis(60);

/// How long to wait before asking a window that reported itself occluded for a
/// frame again.
///
/// Long enough that the window is back in front and the compositor has finished
/// the switch, short enough that nobody sees the old frame first.
const OCCLUDED_RETRY: std::time::Duration = std::time::Duration::from_millis(120);

/// How many of those frames are asked for before giving up.
///
/// A window that really is hidden would otherwise ask eight times a second for
/// an image it cannot have, and a window that really is in front needs one.
const REPAINT_TRIES: u8 = 5;

/// The seed the playground's first room is built from.
///
/// Fixed, so the first press of the shortcut always opens the same room on the
/// same seed -- a mode that cannot be reproduced cannot be checked. Every later
/// visit mixes in the visit count and the clock.
const PLAYGROUND_SEED: u64 = 0x5EED_1CA7_2D0B_90F1;

/// How many clipboard entries the history keeps.
///
/// Ten is Notepad++'s own default and it is the right order of magnitude: enough
/// to recover from a copy you did not mean to make, few enough that the popup is
/// a glance rather than a list to read.
const CLIPBOARD_HISTORY: usize = 10;

/// The most times "Run a Macro Multiple Times" will play a recording.
///
/// A cap rather than a warning: a macro that types a character played a million
/// times would freeze the window with no way to stop it, and a number that large
/// is a typo rather than an intention.
const MAX_MACRO_REPEAT: usize = 1000;

/// Where verbose backups are written.
///
/// Inside the configuration directory rather than beside the file, which is the
/// whole point of the verbose mode: it works on read-only media and it does not
/// put anything into a source tree.
fn backup_dir() -> PathBuf {
    nn_app::paths::config_file("backup").unwrap_or_default()
}

/// The file-type filters the system dialog offers.
///
/// Two, because a longer list is a longer list to read and the portal's own
/// search covers the rest. `*` first so the dialog opens showing everything: a
/// filter that hides the file the user came for is worse than no filter.
fn portal_filters() -> Vec<nn_app::portal::Filter> {
    use nn_app::portal::Filter;
    vec![
        Filter {
            name: "All files".to_string(),
            patterns: vec!["*".to_string()],
        },
        Filter {
            name: "Text files".to_string(),
            patterns: ["*.txt", "*.log", "*.md", "*.ini", "*.cfg", "*.conf"]
                .iter()
                .map(|pattern| pattern.to_string())
                .collect(),
        },
    ]
}

/// Copies `path` aside before it is overwritten, according to `settings`.
///
/// Failures are reported and then ignored. A backup that cannot be written is
/// not a reason to refuse the save the user asked for: the alternative is an
/// editor that will not save because a directory is full, which loses more than
/// it protects.
fn backup_before_write(path: &Path, settings: &Settings) {
    if settings.backup_mode == BackupMode::None {
        return;
    }
    let dir = backup_dir();
    let stamp = file_stamp(now_secs());
    match nn_core::backup_before_save(settings.backup_mode, path, &dir, &stamp) {
        Ok(Some(_)) => {
            if settings.backup_mode == BackupMode::Verbose {
                // Pruned here rather than on a timer: the only moment the
                // directory grows is a save, so the only moment it needs
                // trimming is the same one.
                let _ = nn_core::prune_backups(&dir, settings.backup_keep);
            }
        }
        Ok(None) => {}
        Err(err) => eprintln!("npp: backup of {} failed: {err}", path.display()),
    }
}

/// Writes `tab`'s document, keeping a copy of what it replaces.
///
/// Every save goes through here so that no path can forget the backup. A save
/// that skipped it would be the one that mattered.
fn write_tab(tab: &mut Tab, settings: &Settings) -> std::io::Result<()> {
    let Some(path) = tab.editor.doc.path().map(Path::to_path_buf) else {
        return Ok(());
    };
    backup_before_write(&path, settings);
    tab.editor.doc.save()
}

/// One open document and the state that belongs to it.
/// Reads the user-defined languages from beside the settings.
///
/// A missing file is not an error: most installations have none, and the
/// Language menu then shows only its two built-in rows.
/// Scans the plugins directory, without running anything it finds.
///
/// A missing directory is the normal case, not an error: most installations
/// have no plugins and an editor that said so on every start would be noise.
///
/// Nothing is switched on here. Installation is a file in this folder; running
/// is a name in the settings file, and that list is applied by `load_settings`
/// -- which happens later, because the settings are read after the window
/// exists. A scan that loaded everything would make the manager's switches
/// meaningless on the next start.
fn load_plugins() -> Plugins {
    match nn_app::paths::config_file("plugins") {
        Some(dir) => Plugins::discover(&dir),
        None => Plugins::default(),
    }
}

fn load_langs() -> UdlSet {
    let Some(path) = nn_app::paths::config_file("userDefineLang.xml") else {
        return UdlSet::new();
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => UdlSet::parse(&text),
        Err(_) => UdlSet::new(),
    }
}

/// Writes the user-defined languages beside the settings.
///
/// The file is the format Notepad++ reads, so a language imported from a
/// Notepad++ installation can be edited here and taken back.
fn save_langs(langs: &UdlSet) {
    let Some(path) = nn_app::paths::config_file("userDefineLang.xml") else {
        return;
    };
    nn_app::paths::write(&path, &langs.render());
}

/// Which lexer highlights a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Language {
    /// No highlighting at all.
    Plain,
    /// The built-in C-like lexer, which is what a document gets by default.
    BuiltIn,
    /// Built-in language `n` of `LANGS`, described by its table.
    Spec(usize),
    /// User-defined language `n`.
    Udl(usize),
}

/// The lexer for a language choice.
///
/// An index past the end of the set falls back to the built-in lexer rather
/// than panicking: a language can disappear when a configuration is re-imported,
/// and a tab that pointed at it should lose its colours rather than take the
/// window down.
fn lexer_for(langs: &UdlSet, language: Language) -> Box<dyn Lexer> {
    match language {
        Language::Plain => Box::new(PlainLexer),
        Language::BuiltIn => Box::new(CLikeLexer),
        Language::Spec(index) => match nn_core::LANGS.get(index) {
            Some(spec) => Box::new(ConfigLexer::new(spec)),
            None => Box::new(CLikeLexer),
        },
        Language::Udl(index) => match langs.get(index) {
            Some(udl) => Box::new(udl.lexer()),
            None => Box::new(CLikeLexer),
        },
    }
}

struct Tab {
    editor: Editor,
    highlighter: Highlighter,
    folder: BraceFolder,
    /// The language highlighting this document.
    ///
    /// The choice is per tab rather than per extension so that a file with the
    /// wrong extension can still be highlighted correctly, which is the whole
    /// point of a Language menu.
    language: Language,
    /// Whether the tab is pinned.
    ///
    /// A pinned tab is moved to the front of the strip and refuses to be closed
    /// by anything but an explicit Unpin. It is a promise about the tab, not a
    /// decoration: the point is that a bulk close cannot take it.
    pinned: bool,
    /// The colour the user painted the tab with.
    colour: TabColour,
    /// Whether the file is reloaded whenever it changes on disk.
    ///
    /// Notepad++'s "Monitoring (tail -f)". Per tab rather than global because it
    /// is a statement about one file: a log you are watching, not a policy for
    /// every document you have open.
    monitoring: bool,
    /// The document group this tab belongs to.
    ///
    /// One tab is one group. A clone shares its source's group, and the group is
    /// what makes the two tabs one document: the sync copies text from whichever
    /// member changed to the others, before each frame.
    group: u64,
    /// The document revision this tab was last seen at.
    ///
    /// Compared with `doc.revision()` at the top of a frame: a difference means
    /// this tab is the one that changed, and its group has to be brought up to
    /// date. A stamp rather than a diff, because the sync does not care what
    /// changed, only that something did.
    mirror_rev: u64,
}

/// The next document group the editor hands out.
///
/// Process-wide rather than a field of `State` because `Tab::new` is what opens
/// a document, and it is called from paths that do not have the window — a
/// session restore, a test. A counter that starts at one means zero is never a
/// real group, which is what lets `0` stay a harmless default.
static NEXT_GROUP: AtomicU64 = AtomicU64::new(1);

/// A group id no tab has.
fn fresh_group() -> u64 {
    NEXT_GROUP.fetch_add(1, Ordering::Relaxed)
}

impl Tab {
    /// Opens `doc` and highlights it, unless it is too big to be worth it.
    ///
    /// A large file opens unhighlighted and unfolded. Both passes are O(file) and
    /// together they cost ten seconds on a 100 MB file, which is the difference
    /// between opening and appearing to hang.
    fn new(doc: Document, langs: &UdlSet) -> Self {
        let mut editor = Editor::new(doc);
        // A user-defined language wins over a built-in one: it is the more
        // specific claim, and a user who wrote one for `.log` meant it.
        let language = match editor.doc.path().and_then(|path| langs.by_extension(path)) {
            Some(index) => Language::Udl(index),
            None => match editor
                .doc
                .path()
                .and_then(|path| path.extension())
                .and_then(|ext| ext.to_str())
                .and_then(nn_core::lang_for_extension)
            {
                Some(index) => Language::Spec(index),
                None => Language::BuiltIn,
            },
        };
        let mut highlighter = Highlighter::new(lexer_for(langs, language));
        let folder = BraceFolder;
        if !editor.doc.is_large() {
            highlighter.highlight_all(&mut editor.doc);
            folder.apply(&mut editor.doc);
        }
        let mirror_rev = editor.doc.revision();
        Self {
            editor,
            highlighter,
            folder,
            language,
            pinned: false,
            colour: TabColour::None,
            monitoring: false,
            group: fresh_group(),
            mirror_rev,
        }
    }

    /// Switches the language and re-highlights from the top.
    ///
    /// A language change invalidates every line's state, so there is no cheaper
    /// path than a full pass. A large file is left alone for the same reason it
    /// is left alone on open.
    fn set_language(&mut self, langs: &UdlSet, language: Language) {
        if self.language == language {
            return;
        }
        self.language = language;
        self.highlighter.set_lexer(lexer_for(langs, language));
        if !self.editor.doc.is_large() {
            self.highlighter.highlight_all(&mut self.editor.doc);
        }
    }

    /// The title shown in the tab strip.
    ///
    /// Two markers, because they mean different things and a user has to be able
    /// to tell them apart at a glance: `*` is unsaved work, `!` is a file that
    /// changed underneath it.
    fn title(&self) -> String {
        let mut marker = String::new();
        if self.editor.doc.is_dirty() {
            marker.push_str(" *");
        }
        if self.editor.doc.has_disk_conflict() {
            marker.push_str(" !");
        }
        format!("{}{}", self.editor.doc.display_name(), marker)
    }

    /// The name of this tab's language, for the API.
    ///
    /// The user-defined set is passed in because a tab stores the index of its
    /// language rather than the language: the definitions can be reloaded while
    /// the tab is open, and the name has to be looked up in the current ones.
    fn language_name(&self, langs: &UdlSet) -> String {
        match self.language {
            Language::Plain => "Plain Text".to_string(),
            Language::BuiltIn => "C-like".to_string(),
            Language::Spec(index) => nn_core::LANGS
                .get(index)
                .map(|spec| spec.name.to_string())
                .unwrap_or_else(|| "C-like".to_string()),
            Language::Udl(index) => match langs.get(index) {
                Some(udl) => udl.name.clone(),
                None => "User Defined".to_string(),
            },
        }
    }

    /// The title as the tab strip shows it.
    ///
    /// The dirty marker is left off: the strip draws a pencil for that, and a
    /// `*` beside a pencil says the same thing twice. The conflict marker stays,
    /// because nothing else on the strip says it.
    fn strip_title(&self) -> String {
        let mut title = self.editor.doc.display_name();
        if self.editor.doc.has_disk_conflict() {
            title.push_str(" !");
        }
        title
    }
}

/// A fold command, from a key binding or the fold margin.
#[derive(Debug, Clone, Copy)]
enum FoldCommand {
    Toggle,
    All,
    None,
}

/// Which of the multi-select commands a menu row stands for.
///
/// The four share everything but the editor call, so they share one helper and
/// differ by this. Passing the command rather than four near-identical arms keeps
/// the status message in one place.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MultiCommand {
    /// Put a caret on every occurrence.
    All,
    /// Add the next occurrence.
    Next,
    /// Drop this occurrence and move on.
    Skip,
    /// Drop the last caret added.
    Undo,
}

/// Which bookmarked-line operation a Search menu row stands for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BookmarkCommand {
    Cut,
    Copy,
    Paste,
    Remove,
    RemoveUnmarked,
    Inverse,
}

/// The next match of `needle` at or after `from`, or before it going backwards.
///
/// Wraps at the ends, which is what every editor's Find Next does: a search that
/// stopped at the last match would make the user scroll back to the top by hand.
fn next_match(
    editor: &Editor,
    needle: &str,
    from: usize,
    forward: bool,
) -> Option<std::ops::Range<usize>> {
    let all = editor.find_all(needle, 100_000);
    if all.is_empty() {
        return None;
    }
    if forward {
        all.iter()
            .find(|range| range.start >= from)
            .or_else(|| all.first())
            .cloned()
    } else {
        all.iter()
            .rev()
            .find(|range| range.end <= from)
            .or_else(|| all.last())
            .cloned()
    }
}

/// How wide a read-only command reaches.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ReadScope {
    /// The document in front.
    Current,
    /// Every open document.
    All,
}

/// "1 hit" or "3 hits", for a search results header.
fn hits_noun(count: usize) -> String {
    if count == 1 {
        "1 hit".to_string()
    } else {
        format!("{count} hits")
    }
}

/// "1 file" or "3 files", for a search results header.
fn files_noun(count: usize) -> String {
    if count == 1 {
        "1 file".to_string()
    } else {
        format!("{count} files")
    }
}

/// Percent-encodes `text` for use as a query string value.
///
/// Only the characters that would otherwise change the meaning of the URL are
/// escaped: a space becomes `%20` rather than `+`, because `+` is only a space in
/// a form body and a search engine reading the path would take it literally.
fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The chord a key event stands for, if the keymap could name it.
///
/// Shift is dropped for punctuation by `Chord::from_char`: `Ctrl++` arrives as
/// `Ctrl+Shift++` on any layout where `+` needs shift, and a binding that only
/// fired on some layouts would be a trap.
fn chord_of(
    event: &winit::event::KeyEvent,
    ctrl: bool,
    shift: bool,
    alt: bool,
) -> Option<Chord> {
    let key = match &event.logical_key {
        Key::Character(c) => return Some(Chord::from_char(c.chars().next()?, ctrl, shift, alt)),
        Key::Named(named) => match named {
            NamedKey::Space => KeyName::Space,
            NamedKey::Tab => KeyName::Tab,
            NamedKey::Enter => KeyName::Enter,
            NamedKey::Escape => KeyName::Escape,
            NamedKey::Backspace => KeyName::Backspace,
            NamedKey::Delete => KeyName::Delete,
            NamedKey::ArrowUp => KeyName::Up,
            NamedKey::ArrowDown => KeyName::Down,
            NamedKey::ArrowLeft => KeyName::Left,
            NamedKey::ArrowRight => KeyName::Right,
            NamedKey::Home => KeyName::Home,
            NamedKey::End => KeyName::End,
            NamedKey::PageUp => KeyName::PageUp,
            NamedKey::PageDown => KeyName::PageDown,
            NamedKey::F1 => KeyName::Function(1),
            NamedKey::F2 => KeyName::Function(2),
            NamedKey::F3 => KeyName::Function(3),
            NamedKey::F4 => KeyName::Function(4),
            NamedKey::F5 => KeyName::Function(5),
            NamedKey::F6 => KeyName::Function(6),
            NamedKey::F7 => KeyName::Function(7),
            NamedKey::F8 => KeyName::Function(8),
            NamedKey::F9 => KeyName::Function(9),
            NamedKey::F10 => KeyName::Function(10),
            NamedKey::F11 => KeyName::Function(11),
            NamedKey::F12 => KeyName::Function(12),
            NamedKey::F13 => KeyName::Function(13),
            NamedKey::F14 => KeyName::Function(14),
            NamedKey::F15 => KeyName::Function(15),
            NamedKey::F16 => KeyName::Function(16),
            NamedKey::F17 => KeyName::Function(17),
            NamedKey::F18 => KeyName::Function(18),
            NamedKey::F19 => KeyName::Function(19),
            NamedKey::F20 => KeyName::Function(20),
            NamedKey::F21 => KeyName::Function(21),
            NamedKey::F22 => KeyName::Function(22),
            NamedKey::F23 => KeyName::Function(23),
            NamedKey::F24 => KeyName::Function(24),
            _ => return None,
        },
        _ => return None,
    };
    Some(Chord {
        ctrl,
        shift,
        alt,
        key,
    })
}

/// One editor pane: its tabs, which one is showing, and how it draws.
///
/// A single view is a `State` with `other: None`. A split keeps the focused pane
/// in the `State`'s own fields and the unfocused one here, which is what lets the
/// two hundred call sites that say `self.tabs` and `self.view` keep working
/// without knowing that splits exist. Focusing the other pane swaps the two.
struct Pane {
    tabs: Vec<Tab>,
    active: usize,
    view: EditorView,
    tab_bar: TabBar,
}

impl Pane {
    /// Rebuilds the tab strip titles from the open documents.
    fn sync_tabs(&mut self) {
        let tabs: Vec<TabInfo> = self
            .tabs
            .iter()
            .map(|tab| TabInfo {
                title: tab.strip_title(),
                pinned: tab.pinned,
                colour: tab.colour,
                modified: tab.editor.doc.is_dirty(),
            })
            .collect();
        self.tab_bar.set_tabs(tabs, self.active);
    }
}

/// The two buttons a tab carries at its trailing edge.
///
/// Only the active tab draws them, so neither needs to name a tab: the strip
/// answers "the cross" or "the pin", and the document they act on is the one
/// already in front.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabButton {
    Close,
    Pin,
}

/// What the second and third presses of a click take.
///
/// A click on text has three granularities: one press takes a character, two a
/// unit, three a line. The unit is kept because the drag that usually follows a
/// double click pulls the selection out by the same amount -- dragging from a
/// word takes whole words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClickUnit {
    Word,
    Line,
}

/// Everything that lives for as long as the window does.
struct State {
    window: Arc<Window>,
    gpu: Gpu,
    tabs: Vec<Tab>,
    active: usize,
    view: EditorView,
    find: FindBar,
    /// The floating presentation of the same search.
    ///
    /// It draws the bar's state when the "Floating Search" preference is on, so
    /// the two are never out of step: one search, two ways to show it.
    find_dialog: FindDialog,
    tab_bar: TabBar,
    menu: MenuBar,
    toolbar: Toolbar,
    prompt: PromptBar,
    status: StatusBar,
    results: ResultsPanel,
    history: ChangeHistory,
    charset: CharPanel,
    functions: FunctionList,
    /// The workspace panel: folder-as-workspace or one of the three projects.
    folder: FolderPanel,
    /// The three project slots and which one the panel is showing.
    projects: Projects,
    /// Which panel the workspace column is showing.
    ///
    /// `None` is the folder-as-workspace tree, `Some(n)` is project `n`. One
    /// column, several sources: two docked panels would halve the width the
    /// text gets for no gain, since a user reads one tree at a time.
    workspace: Option<usize>,
    /// The user-defined languages this installation has.
    ///
    /// Loaded once at startup from `userDefineLang.xml` beside the settings,
    /// and replaced wholesale when a Notepad++ configuration is imported.
    langs: UdlSet,
    /// The WebAssembly plugins this installation has.
    ///
    /// Loaded once at startup from `plugins/` beside the settings. A plugin
    /// that fails to load is kept in the list's failures rather than dropped,
    /// so the log can say which file and why.
    plugins: Plugins,
    /// The accessibility adapter.
    ///
    /// Held for the life of the window: dropping it withdraws the tree from
    /// AT-SPI, which a screen reader reports as the application closing.
    a11y: accesskit_winit::Adapter,
    /// The signature of the tree last sent.
    ///
    /// `None` means "send the next one whatever it says", which is what the
    /// initial-tree request and a reactivation both need.
    a11y_signature: Option<u64>,
    /// Where a file dialog's answer is delivered from its thread.
    proxy: EventLoopProxy<UserEvent>,
    /// True while a portal dialog is open.
    ///
    /// The portal call blocks on its own thread, so without this a second Open
    /// would start a second dialog and the two answers would arrive in an order
    /// nothing tracks.
    portal_pending: bool,
    /// The bus name this process owns, when it is the single instance.
    ///
    /// `None` when another editor already had the name, when `--new-instance`
    /// was given, or when there is no session bus. Every use is guarded, so a
    /// standalone editor simply never signals anything.
    server: Option<nn_app::single::Server>,
    complete: CompletionPopup,
    /// Which completion mode the popup is serving, so a keystroke knows what to
    /// refilter against.
    completion_mode: CompletionMode,
    /// The signature of the call the caret is inside, when there is one.
    param_hint: ParamHint,
    /// The label that appears when the pointer rests on a toolbar button.
    ///
    /// It owns its own dwell timer, so the application only has to tell it what
    /// is under the pointer and when to wake up.
    tooltip: Tooltip,
    context: ContextMenu,
    /// The order the text area's popup menu shows, when a `contextMenu.xml`
    /// says so. `None` keeps the built-in order.
    context_layout: Option<Vec<String>>,
    peeker: DocumentPeeker,
    about: AboutBox,
    /// Whether the pointer is on the About box's link.
    ///
    /// Tracked so the cursor is only changed when it crosses the link's edge:
    /// `set_cursor` is a round trip to the windowing system, and a pointer
    /// moving across a dialog would otherwise make one per motion event.
    link_hover: bool,
    prefs: Preferences,
    /// The plugin manager, closed until something opens it.
    plugin_list: PluginList,
    print: PrintPreview,
    styles: StyleConfigurator,
    shortcuts: Shortcuts,
    browser: Browser,
    /// Where the caret and the scroll were when incremental search opened.
    ///
    /// Escape puts them back, which is the whole point of the mode: a search you
    /// abandoned should leave the document where it found it.
    incremental_anchor: Option<(usize, usize, f32)>,
    /// The user's preferences, as last applied.
    settings: Settings,
    /// What the desktop said about its colour scheme, when it said anything.
    ///
    /// `None` means no portal, no session bus, or a desktop with no preference.
    /// Kept here rather than read on demand because the answer arrives on a
    /// thread, and the theme has to be applied from the event loop.
    system_light: Option<bool>,
    renderer: Renderer,
    /// The room behind `Ctrl+Alt+K`, and the idle clock that opens it.
    playground: Playground,
    /// How many times the mode has been opened, which seeds its rooms.
    ///
    /// A counter rather than the clock alone: two visits in the same second
    /// would otherwise be the same room, and pressing the shortcut twice is
    /// exactly how a user finds that out.
    playground_visits: u64,
    modifiers: ModifiersState,
    dragging: bool,
    /// How many presses the press in progress is: one, two for a word, three
    /// for a line.
    click_count: u8,
    /// Where and when the last press landed, so the next one can tell whether
    /// it is a repeat of it or a new click.
    last_press: Option<(f64, f64, std::time::Instant)>,
    /// The unit a double or triple click selected, held for the drag that may
    /// follow it.
    unit_drag: Option<(ClickUnit, (usize, usize))>,
    /// True while the left button is held down on the minimap column.
    minimap_drag: bool,
    /// When the loop owes the window a frame it could not draw earlier.
    ///
    /// A surface that reports itself occluded hands back no image, and on a
    /// compositing desktop the report outlives the reason for it: the window is
    /// back in front, the focus is back, and the surface still says hidden until
    /// something makes it look again. Retrying at once would spin the loop while
    /// the window really is hidden, and not retrying at all leaves the last
    /// frame on screen until an unrelated event arrives -- which on an idle
    /// window is never. A deadline is the middle: one frame, shortly after.
    repaint_at: Option<std::time::Instant>,
    /// How many of those frames in a row have gone by without one reaching the
    /// screen, so a window that stays hidden stops asking.
    repaint_tries: u8,
    /// When the window last regained the focus, so the key that made the switch
    /// can be told from the first key typed after it.
    focus_at: Option<std::time::Instant>,
    /// Whether the window is borderless full screen.
    full_screen: bool,
    /// Whether the window is pinned above every other one.
    ///
    /// Kept here as well as in the window because `set_window_level` has no
    /// getter: without a copy the row could only ever turn the pin on.
    always_on_top: bool,
    /// Last known cursor position in physical pixels.
    cursor: (f64, f64),
    /// Opened lazily: on a headless or clipboard-less system this stays `None`
    /// and the editor simply has no clipboard rather than failing to start.
    clipboard: Option<arboard::Clipboard>,
    /// The recent file list and the tabs to restore on the next launch.
    session: Session,
    /// Where a column block started, while one is being dragged out.
    block_anchor: Option<(usize, usize)>,
    /// The caret position "Begin/End Select" remembers.
    ///
    /// Notepad++'s version is a two-press command: the first press drops a marker,
    /// the second selects from it to wherever the caret has got to. Keeping the
    /// marker here rather than in the editor means an ordinary click or arrow key
    /// can clear it, which is what makes the mode feel like a mode.
    select_anchor: Option<usize>,
    /// Whether a plain drag selects a rectangle.
    ///
    /// Column mode is a sticky toggle in Notepad++ (Alt+C), not a modifier you
    /// hold, so it has to outlive the drag it was turned on for.
    column_mode: bool,
    /// Which hit in the results panel "Next Search Result" last went to.
    ///
    /// `None` before the first step, so the first press starts at the top rather
    /// than at whatever index zero happens to mean.
    result_cursor: Option<usize>,
    /// The digest a "Generate from files..." prompt is waiting to compute.
    ///
    /// The prompt answers with a path, and the path alone does not say which of
    /// the four digests to use, so the choice is parked here while the field is
    /// open.
    pending_hash: Option<nn_core::hash::Hash>,
    /// The last few things put on the clipboard, newest first.
    ///
    /// A ring rather than a full history: the point is to recover the thing you
    /// copied two copies ago, and a list long enough to need scrolling would be a
    /// worse way to find it than the file it came from.
    clipboard_history: Vec<String>,
    /// The recorded keystrokes, if any.
    recorder: Macro,
    /// The `(tab, revision)` the function list was last scanned at, so a frame
    /// that only moved the caret does not rescan the file. The tab is part of
    /// the key because two documents can sit at the same revision.
    symbols_key: (usize, u64),
    /// The unfocused pane, when the window is split.
    other: Option<Pane>,
    /// True when the focused pane is the second one on screen.
    ///
    /// `self` always names the focused pane, because everything that edits a
    /// document talks about `self.tabs` and `self.view`. Focus still has to be
    /// able to move without moving the documents with it — clicking the right
    /// view in Notepad++ leaves both files where they are — so the geometry
    /// remembers which rectangle the focused pane occupies, and this is that
    /// memory. [`State::pane_rects`] and the renderer both read it, which is what
    /// keeps a click and the pixel under it in agreement.
    focused_second: bool,
    /// How the window is split.
    split: Split,
    /// Where the divider sits, as a fraction of the pane area.
    ratio: f32,
    /// True while the left button is held down on the divider.
    divider_drag: bool,
    /// The tab a left press landed on, and the point it landed at.
    ///
    /// A press becomes a drag only once the pointer has moved
    /// [`TAB_DRAG_SLOP`] pixels, so the click that selects a tab cannot move it
    /// into the other view by mistake.
    tab_press: Option<(usize, f64, f64)>,
    /// The tab being dragged into the other view, if any.
    tab_drag: Option<usize>,
    /// The `(tab, revision, word)` the smart highlight was last built for.
    ///
    /// Building it copies the whole document to search it, so a caret move
    /// inside the same word — which is most arrow-key navigation — must not
    /// pay for it.
    smart_key: (usize, u64, String),
    /// Which window furniture is drawn.
    ///
    /// Distraction-free mode is a policy about the same window rather than a
    /// second window, so it is one value the renderer reads rather than a flag
    /// consulted at each of a dozen call sites.
    chrome: Chrome,
    /// How big the window furniture is drawn.
    ///
    /// The menu bar, the toolbar, the tab strip, the status bar and the panels
    /// are built once with these metrics, and rebuilding them is the only way a
    /// label changes size: a widget shapes its text when it is made. Which is why
    /// the density is read here, once, rather than asked for at each draw.
    ui: UiMetrics,
    /// When the unsaved buffers were last written to the snapshot directory.
    ///
    /// A timestamp rather than a deadline: the interval can change under it when
    /// the user edits the preference, and comparing an elapsed time against the
    /// current interval needs no bookkeeping when that happens.
    last_snapshot: std::time::Instant,
}

impl State {
    /// Rebuilds the other pane's tab strip, if there is one.
    fn sync_other_tabs(&mut self) {
        if let Some(other) = &mut self.other {
            other.sync_tabs();
        }
    }

    /// Whether a pane holds nothing but a pristine untitled buffer.
    ///
    /// A split has to start with a document in each pane, because every
    /// invariant in this file assumes `tabs` is non-empty. That placeholder is
    /// dropped the moment a real tab arrives, so a split does not leave an empty
    /// tab behind for the user to close.
    fn is_placeholder(pane: &Pane) -> bool {
        pane.tabs.len() == 1
            && pane.tabs[0].editor.doc.path().is_none()
            && !pane.tabs[0].editor.doc.is_dirty()
            && pane.tabs[0].editor.doc.len_chars() == 0
    }

    /// Applies `settings` to one view.
    ///
    /// A free function rather than a method because it has to run on either
    /// pane: the focused one lives in `self.view`, the other in `self.other`.
    fn apply_view_settings(
        view: &mut EditorView,
        font_system: &mut cosmic_text::FontSystem,
        settings: &Settings,
        system_light: Option<bool>,
        scale: f32,
    ) {
        view.set_theme(nn_app::styles::effective_theme(
            settings.theme.theme(system_light),
            &settings.styles,
        ));
        // The text follows the display as well as the furniture: a 4K screen that
        // doubled the menu bar and left the document at fifteen pixels would be
        // the same complaint in a different place. The preference stays what it
        // always was, a size in pixels at scale 1, so the number a user typed is
        // the number the dialog still shows.
        view.set_font_size(font_system, (settings.font_size * scale).round());
        view.set_tab_width(settings.tab_width);
        view.set_wrap(settings.wrap);
        view.minimap_mut().set_enabled(settings.minimap);
    }

    /// Builds a pane whose view matches the focused one's settings.
    ///
    /// The two panes are two views on the same preferences: a split that showed
    /// the light theme on one side and the dark one on the other would be a bug,
    /// not a feature.
    fn new_pane(&mut self) -> Pane {
        let settings = self.settings.clone();
        let theme = self.view.theme();
        let mut view = EditorView::new(self.renderer.font_system(), theme);
        Self::apply_view_settings(
            &mut view,
            self.renderer.font_system(),
            &settings,
            self.system_light,
            self.ui.scale(),
        );
        let mut tab_bar = TabBar::new(
            self.ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
        );
        tab_bar.set_layout(settings.tab_layout);
        Pane {
            tabs: vec![Tab::new(Document::new(), &self.langs)],
            active: 0,
            view,
            tab_bar,
        }
    }

    /// Creates the second pane if there is not one already.
    fn ensure_split(&mut self) {
        if self.other.is_some() {
            return;
        }
        let pane = self.new_pane();
        self.other = Some(pane);
        if self.split == Split::None {
            self.split = Split::Vertical;
        }
        self.resize_view();
    }

    /// Splits the window in `split`, or flips an existing split to it.
    fn split_window(&mut self, split: Split) {
        self.ensure_split();
        self.split = split;
        self.resize_view();
        self.window.request_redraw();
    }

    /// Turns the split a quarter turn, keeping the divider where it is.
    ///
    /// The ratio is a fraction of the window, so it survives the turn unchanged:
    /// a divider two thirds of the way across becomes one two thirds of the way
    /// down, which is where a user who dragged it there is looking for it.
    fn rotate_split(&mut self) {
        if !self.split.is_split() || self.other.is_none() {
            return;
        }
        self.split = self.split.flipped();
        self.resize_view();
        self.window.request_redraw();
    }

    /// Swaps the focused pane with the other one.
    ///
    /// The four fields are swapped rather than the two panes being addressed
    /// through an index, because the rest of this file talks about `self.tabs`
    /// and `self.view` and has no idea that a second pane exists. Flipping
    /// [`State::focused_second`] with them is what makes the swap a move of the
    /// focus rather than a move of the documents: the two panes change sides in
    /// the fields and the geometry changes sides with them, so what the user sees
    /// is the same two files in the same two places with the other one active.
    fn swap_panes(&mut self) {
        let Some(mut other) = self.other.take() else {
            return;
        };
        std::mem::swap(&mut self.tabs, &mut other.tabs);
        std::mem::swap(&mut self.active, &mut other.active);
        std::mem::swap(&mut self.view, &mut other.view);
        std::mem::swap(&mut self.tab_bar, &mut other.tab_bar);
        self.other = Some(other);
        self.focused_second = !self.focused_second;
        // Each view is now drawing the document the other one drew, so every
        // mark they hold points at the wrong text. The focused view is given
        // its own back by the `after_tab_change` the callers run.
        self.forget_marks();
    }

    /// Drops the marks that name positions in a document, in both views.
    ///
    /// The search matches, the current match, the bracket pair and the smart
    /// occurrences are byte ranges into the text a view was drawing, so they
    /// cannot survive a view being pointed at another document. The cache key
    /// is cleared with them: it is what makes `refresh_smart` cheap, and a key
    /// that outlived its ranges would skip the recompute that puts the focused
    /// view's own highlight back.
    fn forget_marks(&mut self) {
        self.view.forget_marks();
        if let Some(other) = &mut self.other {
            other.view.forget_marks();
        }
        self.smart_key = (usize::MAX, u64::MAX, String::new());
    }

    /// Makes the other pane the focused one.
    fn focus_other(&mut self) {
        if self.other.is_none() {
            return;
        }
        self.swap_panes();
        self.after_tab_change();
    }

    /// Moves the active tab into the other pane, creating the split if needed.
    fn move_to_other(&mut self) {
        let active = self.active;
        self.move_tab_to_other(active);
    }

    /// Moves tab `index` of the focused pane into the other one.
    ///
    /// `index` is an index into `self.tabs`, so a drag that started on a tab
    /// which has since been closed moves whatever is now at that position; the
    /// caller passes the index it recorded on the press, and a stale one is
    /// clamped rather than panicking.
    fn move_tab_to_other(&mut self, index: usize) {
        self.ensure_split();
        if index >= self.tabs.len() {
            return;
        }
        let Some(other) = &mut self.other else {
            return;
        };
        if Self::is_placeholder(other) {
            other.tabs.clear();
        }
        let tab = self.tabs.remove(index);
        other.tabs.push(tab);
        other.active = other.tabs.len() - 1;
        // A view that gives away its last document goes away itself, the way it
        // does when that document is closed: Notepad++ returns to a single
        // window instead of leaving an empty pane behind, and dragging the only
        // tab out of a view is the way to get rid of that view.
        let emptied = self.tabs.is_empty();
        if !emptied && self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
        // The focus follows the document, which is what Notepad++ does: the
        // point of moving a tab is to go on working on it over there.
        self.swap_panes();
        if emptied {
            // The pane that received the tab is the focused one now, and it is
            // the only one left: the emptied pane has nothing to draw.
            self.other = None;
            self.split = Split::None;
            self.focused_second = false;
        }
        self.after_tab_change();
        if emptied {
            // The surviving view now owns the whole window, so it has to be
            // widened from the rectangle it had on one side of the divider.
            self.resize_view();
        }
        self.sync_other_tabs();
        self.window.request_redraw();
    }

    /// Slides the dragged tab to whichever slot the pointer is over.
    ///
    /// Reordering is live rather than decided on the drop: the strip is the only
    /// feedback the gesture has, and a tab that jumps into place on release has
    /// to be aimed at rather than pushed. Moving the tab under the pointer leaves
    /// the pointer over that same tab afterwards, so the row cannot flicker
    /// between two positions -- which is why the plain hit test is enough here
    /// and no half-way threshold is needed.
    fn drag_tab_to(&mut self, x: f32, y: f32) {
        let Some(from) = self.tab_drag else {
            return;
        };
        if from >= self.tabs.len() {
            return;
        }
        let target = self
            .tab_at(x as f64, y as f64)
            .or_else(|| self.past_strip_end(x, y));
        let Some(to) = target else {
            return;
        };
        if to == from || to >= self.tabs.len() {
            return;
        }
        nn_app::tabbar::move_tab(&mut self.tabs, from, to);
        // The document did not change, only where its tab sits, so the index
        // follows the tab and nothing is reloaded.
        self.active = to;
        self.tab_drag = Some(to);
        self.tab_press = Some((to, x as f64, y as f64));
        self.sync_tabs();
        self.sync_other_tabs();
        self.window.request_redraw();
    }

    /// The last tab, for a pointer dragged past the end of the row.
    ///
    /// [`Self::tab_at`] answers nothing over the empty space beyond the last tab,
    /// so without this a tab dragged to the end would stop one place short. The
    /// pane's own rectangle is checked first: a drag heading for the other view
    /// must not slide along this one on the way.
    fn past_strip_end(&self, x: f32, y: f32) -> Option<usize> {
        if self.tab_bar.layout().is_vertical() {
            return None;
        }
        let (first, _) = self.pane_rects();
        if !first.contains(x, y) || y >= self.strip_band().1 {
            return None;
        }
        self.tab_bar.reorder_end(x, y, self.ui.line_height)
    }

    /// Finishes a tab drag.
    ///
    /// A tab released over the other view moves there. Released inside its own
    /// strip it stays where the drag left it, which is Notepad++'s behaviour: the
    /// reorder is the gesture, and the drop only has to decide between the strip
    /// and the pane beside it.
    fn drop_tab(&mut self) {
        let dragged = self.tab_drag.take();
        self.tab_press = None;
        let Some(index) = dragged else {
            return;
        };
        let Some(second) = self.pane_rects().1 else {
            return;
        };
        if !second.contains(self.cursor.0 as f32, self.cursor.1 as f32) {
            return;
        }
        self.move_tab_to_other(index);
    }

    /// Puts the active document in the other pane as a clone.
    ///
    /// Notepad++ puts one buffer in both views: type in either and the other
    /// follows. This editor has one `Document` per tab, so the clone is a second
    /// document that joins the source's *group*, and the group is what keeps the
    /// text equal — see [`State::sync_clones`], which runs before every frame.
    ///
    /// The clone starts at the source's caret rather than at the top, because
    /// the reason to clone is to watch two places in one file at once: a second
    /// view parked at line 1 would have to be scrolled before it was useful.
    fn clone_to_other(&mut self) {
        self.ensure_split();
        let doc = self.tab().editor.doc.clone();
        let caret = self.tab().editor.caret();
        let group = self.tab().group;
        let Some(other) = &mut self.other else {
            return;
        };
        if Self::is_placeholder(other) {
            other.tabs.clear();
        }
        let mut tab = Tab::new(doc, &self.langs);
        tab.group = group;
        tab.editor.set_caret(caret);
        other.tabs.push(tab);
        other.active = other.tabs.len() - 1;
        self.sync_other_tabs();
        self.window.request_redraw();
    }

    /// Clears the unsaved marker on every tab of `group`.
    ///
    /// Called after a save: the twins hold the text that was just written, so
    /// none of them differs from the file any more.
    fn clear_group_dirty(&mut self, group: u64) {
        for index in 0..self.tabs.len() {
            if self.tabs[index].group == group {
                self.tabs[index].editor.doc.set_dirty(false);
            }
        }
        if let Some(other) = &mut self.other {
            for index in 0..other.tabs.len() {
                if other.tabs[index].group == group {
                    other.tabs[index].editor.doc.set_dirty(false);
                }
            }
        }
    }

    /// The tab at `(second, index)`, in whichever pane that is.
    ///
    /// Two panes, addressed by a flag rather than by an enum because the caller
    /// has already decided: the scans that use this walk both panes in one pass.
    fn pane_tab(&mut self, second: bool, index: usize) -> &mut Tab {
        if second {
            &mut self.other.as_mut().expect("the second pane").tabs[index]
        } else {
            &mut self.tabs[index]
        }
    }

    /// Brings the other clones of a changed document up to date.
    ///
    /// The one place that makes a clone a clone. Each tab carries the revision
    /// of its document as of the last frame; a tab whose revision moved has
    /// changed, and every other tab in its group is given its text, its dirty
    /// marker and a fresh highlight before the frame that would show the
    /// difference is drawn.
    ///
    /// Three things are deliberately *not* shared, and each is a visible
    /// difference from Notepad++ worth knowing:
    ///
    /// - the caret: each view keeps its own, clamped to the new text, which is
    ///   what lets one view watch the top of a file and the other the bottom;
    /// - the undo history: the target's is dropped, because its ranges describe
    ///   text that has just been replaced and replaying one would cut at an
    ///   offset that may be past the end. Undo keeps working in the view the
    ///   user last typed in, which is the view they are looking at;
    /// - the scroll position: the target keeps its own, so cloning does not drag
    ///   the other view along.
    ///
    /// The cost is a copy of the text per edit, paid only while a group has more
    /// than one member.
    fn sync_clones(&mut self) {
        // (second pane?, index, group, revision, stamp), read once so the
        // decision below needs no borrow of a tab.
        let mut members: Vec<(bool, usize, u64, u64, u64)> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                (
                    false,
                    index,
                    tab.group,
                    tab.editor.doc.revision(),
                    tab.mirror_rev,
                )
            })
            .collect();
        if let Some(other) = &self.other {
            members.extend(other.tabs.iter().enumerate().map(|(index, tab)| {
                (
                    true,
                    index,
                    tab.group,
                    tab.editor.doc.revision(),
                    tab.mirror_rev,
                )
            }));
        }
        let mut groups: Vec<u64> = Vec::new();
        for (_, _, group, _, _) in &members {
            let shared = members.iter().filter(|member| member.2 == *group).count() > 1;
            if shared && !groups.contains(group) {
                groups.push(*group);
            }
        }
        for group in groups {
            let in_group: Vec<(bool, usize, u64, u64, u64)> = members
                .iter()
                .filter(|member| member.2 == group)
                .copied()
                .collect();
            // At most one member moves between two frames: a keystroke edits one
            // document. If two ever did, the first would win, and the loser's
            // text would be overwritten — which is the same answer as editing a
            // shared buffer from two windows at once.
            let Some(source) = in_group.iter().find(|member| member.3 != member.4) else {
                continue;
            };
            let (source_second, source_index) = (source.0, source.1);
            // Cloned, not borrowed: the loop below needs the other pane
            // mutably, and a `Rope` clone shares its chunks. The clone carries
            // the twin's style bytes as well as its text, so a keystroke in one
            // view costs the other a memcpy rather than a re-lex of the file.
            let source_doc = self
                .pane_tab(source_second, source_index)
                .editor
                .doc
                .clone();
            for (second, index, _, _, _) in &in_group {
                if *second == source_second && *index == source_index {
                    continue;
                }
                let target = self.pane_tab(*second, *index);
                target.editor.adopt(&source_doc);
                // Fold levels came across too, so the folder's marks are
                // already right; only the scroll needs bringing back in line.
                target.editor.ensure_caret_visible();
            }
            // Every member is stamped, not just the targets: the source's own
            // stamp has to move too, or it would look changed again next frame.
            for (second, index, _, _, _) in &in_group {
                let tab = self.pane_tab(*second, *index);
                tab.mirror_rev = tab.editor.doc.revision();
            }
        }
    }

    /// Merges the other pane's tabs back into this one and drops the split.
    fn unsplit(&mut self) {
        let Some(mut other) = self.other.take() else {
            self.split = Split::None;
            return;
        };
        if Self::is_placeholder(&other) {
            other.tabs.clear();
        }
        self.tabs.append(&mut other.tabs);
        self.split = Split::None;
        self.after_tab_change();
        self.resize_view();
    }

    /// The height of the menu bar as the renderer will lay it out this frame.
    ///
    /// Every one of these accessors exists because the chrome policy can hide a
    /// bar, and a hit test that used the bar's nominal height while the renderer
    /// used zero would put the click in the wrong place. Reading the policy in
    /// one place and the geometry in another is how that happens, so both are
    /// read here.
    fn menu_height(&self) -> f32 {
        let line_height = self.ui.line_height;
        if self.chrome.shows_menu(self.cursor.1 as f32, line_height) {
            self.menu.height(line_height)
        } else {
            0.0
        }
    }

    /// The height of the toolbar as the renderer will lay it out this frame.
    fn toolbar_height(&self) -> f32 {
        if self.chrome.shows_toolbar() {
            self.toolbar.height(self.ui.line_height)
        } else {
            0.0
        }
    }

    /// The height of the tab strip as the renderer will lay it out this frame.
    fn tab_height(&self) -> f32 {
        if self.chrome.shows_tabs() {
            self.tab_bar.height(self.ui.line_height)
        } else {
            0.0
        }
    }

    /// The height of the status bar as the renderer will lay it out this frame.
    fn status_height(&self) -> f32 {
        if self.chrome.shows_status() {
            self.status.height(self.ui.line_height)
        } else {
            0.0
        }
    }

    /// The height of the results panel as the renderer will lay it out.
    fn results_height(&self) -> f32 {
        if self.chrome.shows_results(self.results.is_open()) {
            self.results.height(self.ui.line_height)
        } else {
            0.0
        }
    }

    /// The height of the change-history panel as the renderer will lay it out.
    fn history_height(&self) -> f32 {
        if self.chrome.shows_history(self.history.is_open()) {
            self.history.height(self.ui.line_height)
        } else {
            0.0
        }
    }

    /// The width of the function list as the renderer will lay it out.
    fn functions_width(&self) -> f32 {
        if self.chrome.shows_functions() {
            self.functions.width(self.view.char_width())
        } else {
            0.0
        }
    }

    /// The blank margin on each side of the text column, in pixels.
    fn column_inset(&self) -> f32 {
        self.chrome
            .inset(self.gpu.config.width as f32, self.view.char_width())
    }

    /// The pane area as `(left, top, width, height)`.
    ///
    /// The same arithmetic the renderer does, so a click lands in the pane the
    /// user sees under the cursor. Both call `layout::panes`, which is the only
    /// place the split geometry is written down.
    fn pane_area(&self) -> (f32, f32, f32, f32) {
        let width = self.gpu.config.width as f32;
        let height = self.gpu.config.height as f32;
        let top = self.menu_height() + self.toolbar_height();
        let bottom = self.status_height()
            + self.history_height()
            + self.results_height()
            + self.find.height(width, self.ui.line_height, self.view.padding());
        let inset = self.column_inset();
        (inset, top, width - 2.0 * inset, (height - top - bottom).max(1.0))
    }

    /// The rectangle each pane occupies, in window pixels.
    fn pane_rects(&self) -> (Rect, Option<Rect>) {
        let (left, top, width, height) = self.pane_area();
        let (first, second) = layout::panes(self.split, left, top, width, height, self.ratio);
        // The vertical tab strip lives inside the pane it belongs to, so the
        // text starts to the right of it. Inset here rather than in the renderer
        // alone, because this is what decides which pane a click landed in: a
        // click in the column would otherwise be treated as a click on the text
        // underneath it.
        let column = self.tab_column();
        let first = layout::inset_left(first, column);
        let second = second.map(|rect| layout::inset_left(rect, column));
        layout::by_focus(first, second, self.focused_second)
    }

    /// The width of the vertical tab column, or zero when the strip is a row.
    fn tab_column(&self) -> f32 {
        if self.chrome.shows_tabs() {
            self.tab_bar.width()
        } else {
            0.0
        }
    }

    /// The band the focused pane's tab strip occupies, as `(top, bottom)`.
    ///
    /// The strip is drawn along the top edge of its own pane, so the two strips
    /// only share a height while the panes are side by side. Reading the band
    /// from the window's top instead is what made every click on the lower strip
    /// of a top-and-bottom split fall on nothing.
    fn strip_band(&self) -> (f32, f32) {
        let top = self.pane_rects().0.y;
        (top, top + self.tab_height())
    }

    /// The tab under the pointer, in whichever arrangement is in use.
    ///
    /// The horizontal strip is a band below the toolbar; the vertical column is
    /// the left edge of each pane. Both are derived from the same geometry the
    /// renderer draws with, so a click cannot land on a tab that is not there.
    fn tab_at(&self, x: f64, y: f64) -> Option<usize> {
        let line_height = self.ui.line_height;
        let (x, y) = (x as f32, y as f32);
        if self.tab_bar.layout().is_vertical() {
            let column = self.tab_column();
            if column <= 0.0 {
                return None;
            }
            let (first, second) = self.pane_rects();
            for rect in [Some(first), second].into_iter().flatten() {
                if x >= rect.x - column && x < rect.x && y >= rect.y && y < rect.y + rect.height {
                    return self.tab_bar.tab_at(x, y, line_height);
                }
            }
            return None;
        }
        let (top, bottom) = self.strip_band();
        if y < top || y >= bottom {
            return None;
        }
        self.tab_bar.tab_at(x, y, line_height)
    }

    /// Which scroll arrow of the tab strip the pointer is on, if either.
    ///
    /// `-1` is the left arrow, `1` the right. The column layout never scrolls --
    /// a column is as tall as its tabs, and vertical space is what a split gives
    /// up last -- so only the horizontal strip answers here.
    fn tab_arrow_at(&self, x: f64, y: f64) -> Option<i32> {
        if self.tab_bar.layout().is_vertical() {
            return None;
        }
        let (top, bottom) = self.strip_band();
        let (x, y) = (x as f32, y as f32);
        if y < top || y >= bottom {
            return None;
        }
        self.tab_bar.arrow_at(x, y, self.ui.line_height)
    }

    /// The close or pin button under the pointer, if any.
    ///
    /// Both buttons belong to the active tab, so neither carries an index: the
    /// strip only draws them on the tab you are already looking at, which is
    /// Notepad++'s default. The band guards mirror [`Self::tab_at`] so a click
    /// on the toolbar above cannot be read as a click on a tab's cross.
    fn tab_button_at(&self, x: f64, y: f64) -> Option<TabButton> {
        let line_height = self.ui.line_height;
        let (x, y) = (x as f32, y as f32);
        if self.tab_bar.layout().is_vertical() {
            let column = self.tab_column();
            if column <= 0.0 {
                return None;
            }
            let (first, second) = self.pane_rects();
            for rect in [Some(first), second].into_iter().flatten() {
                if x >= rect.x - column && x < rect.x && y >= rect.y && y < rect.y + rect.height {
                    return self.tab_button_in(x, y, line_height);
                }
            }
            return None;
        }
        let (top, bottom) = self.strip_band();
        if y < top || y >= bottom {
            return None;
        }
        self.tab_button_in(x, y, line_height)
    }

    /// Which of the two buttons the point is on, if either.
    fn tab_button_in(&self, x: f32, y: f32, line_height: f32) -> Option<TabButton> {
        if self.tab_bar.close_at(x, y, line_height).is_some() {
            return Some(TabButton::Close);
        }
        if self.tab_bar.pin_at(x, y, line_height).is_some() {
            return Some(TabButton::Pin);
        }
        None
    }

    /// The bar between the panes, if the window is split.
    fn divider_rect(&self) -> Option<Rect> {
        let (left, top, width, height) = self.pane_area();
        layout::divider(self.split, left, top, width, height, self.ratio)
    }

    /// Moves the divider to follow the pointer.
    fn drag_divider(&mut self, x: f64, y: f64) {
        let (left, top, width, height) = self.pane_area();
        self.ratio = layout::ratio_at(self.split, x as f32, y as f32, left, top, width, height);
        self.window.request_redraw();
    }

    fn new(
        window: Arc<Window>,
        gpu: Gpu,
        a11y: accesskit_winit::Adapter,
        proxy: EventLoopProxy<UserEvent>,
        path: Option<PathBuf>,
        fonts: Option<std::thread::JoinHandle<LoadedFonts>>,
    ) -> Self {
        let size = window.inner_size();
        // Read before anything is built, because every widget below shapes its
        // labels at construction. A display that reports a density of two -- a 4K
        // panel with the desktop's scaling on, which is what "everything is tiny"
        // looks like from inside the program -- gets furniture twice the size.
        let ui = UiMetrics::at(window.scale_factor() as f32);

        // The scan was started beside the graphics device; joining it here is
        // normally free, and a thread that panicked falls back to doing the work
        // on this one rather than taking the window down with it.
        let (mut font_system, char_width) = fonts
            .and_then(|thread| thread.join().ok())
            .unwrap_or_else(load_fonts);
        // The playground's glyph atlas is rasterised here, before the font
        // system is handed to the renderer. It costs one shaping pass over the
        // printable ASCII range, and doing it now is what keeps the mode from
        // stalling the window the first time the shortcut is pressed.
        let playground = Playground::new(
            &gpu.device,
            &gpu.queue,
            gpu.format(),
            &mut font_system,
            PLAYGROUND_SEED,
            std::time::Instant::now(),
        );
        let mut renderer =
            Renderer::with_font_system(&gpu.device, &gpu.queue, gpu.format(), font_system);
        let mut view = EditorView::with_char_width(Theme::DARK, char_width);

        let mut session = Session::load();
        let langs = load_langs();
        let plugins = load_plugins();
        let mut tabs: Vec<Tab> = Vec::new();
        let mut active = 0;
        match &path {
            Some(p) => {
                // The first file gets the window, so it is opened here rather
                // than through `open_path`. A name that is not there yet is a
                // blank page of that name, and it is not a recent file until
                // the first save puts it on disk: `npp notes.txt` in a fresh
                // directory is the usual way to start writing something new.
                let existed = p.exists();
                let doc = Document::open(p).unwrap_or_else(|err| {
                    eprintln!("npp: cannot open {}: {err}", p.display());
                    Document::new()
                });
                if existed {
                    session.push_recent(p);
                }
                tabs.push(Tab::new(doc, &langs));
            }
            None => {
                // Restoring is best-effort: a file that has been moved or
                // deleted since the last run is skipped, not reported.
                for saved in &session.tabs {
                    if let Ok(doc) = Document::from_file(&saved.path) {
                        let mut tab = Tab::new(doc, &langs);
                        tab.editor.goto_position(saved.line, saved.column);
                        tab.pinned = saved.pinned;
                        tab.colour = saved.colour;
                        tabs.push(tab);
                    }
                }
                // Pinned tabs go to the front, which is where the strip keeps
                // them. The session records the order they were in, and a file
                // that was pinned last run must not come back in the middle.
                tabs.sort_by_key(|tab| !tab.pinned);
                active = session.active.min(tabs.len().saturating_sub(1));
                if tabs.is_empty() {
                    tabs.push(Tab::new(Document::new(), &langs));
                }
            }
        }
        let find = FindBar::new(
            ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
        );
        let find_dialog = FindDialog::new(
            ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
        );
        let tab_bar = TabBar::new(
            ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
        );
        let menu = MenuBar::new(
            ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
            Keymap::default(),
        );
        let toolbar = Toolbar::new();
        let prompt = PromptBar::new(
            ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
        );
        let status = StatusBar::new(
            ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
        );
        let results = ResultsPanel::new(
            ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
        );
        let history = ChangeHistory::new(
            ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
        );
        let charset = CharPanel::new(
            ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
        );
        let functions = FunctionList::new(
            ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
        );
        let folder = FolderPanel::new(
            ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
        );
        let complete = CompletionPopup::new(
            ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
        );
        let param_hint = ParamHint::new(
            ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
        );
        let tooltip = Tooltip::new(
            ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
        );
        let peeker = DocumentPeeker::new(
            ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
        );
        let about = AboutBox::new(
            ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
        );
        let context = ContextMenu::new(
            ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
        );
        let prefs = Preferences::new(
            ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
        );
        let plugin_list = PluginList::new(
            ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
        );
        let print = PrintPreview::new(
            ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
        );
        let styles = StyleConfigurator::new(
            ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
        );
        let shortcuts = Shortcuts::new(
            ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
        );
        let browser = Browser::new(
            ui.metrics(),
            cosmic_text::Family::Monospace,
            view.char_width(),
        );
        view.resize(
            &mut tabs[active].editor,
            size.width as f32,
            size.height as f32,
        );
        renderer.resize(&gpu.queue, gpu.config.width, gpu.config.height);

        println!("npp: {}", gpu.describe());
        // Said out loud because a software rasterizer is the explanation for
        // every performance complaint that follows, and the adapter name alone
        // ("llvmpipe") does not mean anything to most people.
        if gpu.is_software() {
            println!("npp: software rendering, expect a slower frame rate");
        }
        println!("npp: present modes {:?}", gpu.present_modes());
        if let Some(monitor) = window.current_monitor() {
            if let Some(mhz) = monitor.refresh_rate_millihertz() {
                println!("npp: display refresh {} Hz", mhz as f32 / 1000.0);
            }
        }

        let mut state = Self {
            window,
            gpu,
            tabs,
            active,
            view,
            find,
            find_dialog,
            tab_bar,
            menu,
            toolbar,
            prompt,
            status,
            results,
            history,
            charset,
            functions,
            folder,
            projects: Projects::new(),
            workspace: None,
            langs,
            plugins,
            a11y,
            a11y_signature: None,
            proxy,
            portal_pending: false,
            server: None,
            complete,
            completion_mode: CompletionMode::Word,
            param_hint,
            tooltip,
            context,
            context_layout: None,
            peeker,
            about,
            link_hover: false,
            prefs,
            plugin_list,
            print,
            styles,
            shortcuts,
            browser,
            incremental_anchor: None,
            settings: Settings::default(),
            system_light: None,
            renderer,
            playground,
            playground_visits: 0,
            modifiers: ModifiersState::empty(),
            dragging: false,
            minimap_drag: false,
            repaint_at: None,
            repaint_tries: 0,
            focus_at: None,
            full_screen: false,
            always_on_top: false,
            cursor: (0.0, 0.0),
            click_count: 0,
            last_press: None,
            unit_drag: None,
            clipboard: arboard::Clipboard::new().ok(),
            session,
            block_anchor: None,
            select_anchor: None,
            column_mode: false,
            result_cursor: None,
            pending_hash: None,
            clipboard_history: Vec::new(),
            symbols_key: (usize::MAX, u64::MAX),
            smart_key: (usize::MAX, u64::MAX, String::new()),
            recorder: Macro::new(),
            other: None,
            focused_second: false,
            split: Split::None,
            ratio: 0.5,
            divider_drag: false,
            tab_press: None,
            tab_drag: None,
            chrome: Chrome::new(Mode::Normal),
            ui,
            last_snapshot: std::time::Instant::now(),
        };
        // Read before the settings are applied, so the first frame is already
        // the right palette rather than the dark one for a frame or two.
        state.system_light =
            nn_app::portal::color_scheme().and_then(|scheme| scheme.is_light());
        state.sync_recent();
        state.sync_runs();
        state.update_title();
        state.load_settings();
        state.load_projects();
        state.load_find_history();
        state.sync_language_menu();
        state.sync_plugin_menu();
        state.sync_palette();
        // After the session, so a buffer that is both in the session and in the
        // snapshot is replaced rather than opened twice.
        state.restore_snapshot(path.is_some());
        state.watch_scheme();
        state
    }

    /// Starts the thread that follows the desktop's colour scheme.
    ///
    /// One thread for the life of the process. The subscription is a match rule
    /// on the bus rather than a call on the portal's name, so it survives the
    /// portal restarting; the process exiting is what ends it. A failure is
    /// reported once and not retried -- a session with no bus is not going to
    /// grow one.
    fn watch_scheme(&mut self) {
        let proxy = self.proxy.clone();
        let spawned = std::thread::Builder::new()
            .name("npp-scheme".to_string())
            .spawn(move || {
                let result = nn_app::portal::watch_color_scheme(move |scheme| {
                    let _ = proxy.send_event(UserEvent::Scheme(scheme));
                });
                if let Err(err) = result {
                    eprintln!("npp: colour scheme: {err}");
                }
            });
        if let Err(err) = spawned {
            eprintln!("npp: colour scheme: {err}");
        }
    }

    /// Follows a change in the desktop's colour scheme.
    ///
    /// Only when the user asked to follow it: an explicit Light or Dark is a
    /// decision, and a desktop that changes its mind at sunset does not get to
    /// overrule it. The palette is rebuilt rather than switched, because the
    /// Style Configurator's overrides sit on top of whichever base is in force.
    fn scheme_changed(&mut self, scheme: nn_app::portal::Scheme) {
        let light = scheme.is_light();
        if light == self.system_light {
            return;
        }
        self.system_light = light;
        if self.settings.theme != ThemeChoice::System {
            return;
        }
        let theme = nn_app::styles::effective_theme(
            self.settings.theme.theme(self.system_light),
            &self.settings.styles,
        );
        self.apply_theme(theme);
        // A `stylers.xml` theme wins over the built-in palette, so it is put
        // back on top of the one just chosen.
        self.apply_saved_theme();
    }

    /// Applies the settings on disk, without writing them back.
    ///
    /// Startup is not a change: a first run should not create a settings file
    /// just because the defaults were applied.
    fn load_settings(&mut self) {
        let settings = Settings::load();
        self.prefs.open(&settings);
        self.apply_settings_quiet();
        self.prefs.close();
        // The tab layout is restored here rather than in `apply_settings_quiet`:
        // the preferences dialog has no tab-strip control, so a dialog round trip
        // must not disturb the arrangement the user made with the View menu.
        //
        // The split is deliberately not among them. A split is an arrangement of
        // documents, and the session records tabs, not panes: restoring one from
        // the settings would open the window with half of it empty on every
        // launch, which is a worse surprise than starting unsplit.
        self.set_tab_layout(settings.tab_layout);
        // Plugins switch on last, because the list of what to run is in the
        // file that was just applied. Nothing has run before this point, which
        // is what "disabled until enabled" means at startup.
        self.plugins.enable_saved(&self.settings.enabled_plugins);
        self.sync_plugin_menu();
        self.resize_view();
    }

    /// The page setup the settings describe.
    ///
    /// The paper key is looked up rather than trusted: a settings file edited by
    /// hand can name a size this build does not have, and falling back to A4 is
    /// better than refusing to print.
    fn print_setup(&self) -> nn_core::print::PageSetup {
        nn_core::print::PageSetup {
            paper: nn_core::print::Paper::from_key(&self.settings.print_paper)
                .unwrap_or(nn_core::print::Paper::A4),
            landscape: self.settings.print_landscape,
            line_numbers: self.settings.print_line_numbers,
            header: self.settings.print_header.clone(),
            footer: self.settings.print_footer.clone(),
            ..nn_core::print::PageSetup::default()
        }
    }

    /// The width of one character at the print font size, in points.
    ///
    /// The view measures its own cell at the editor's font size, so the ratio of
    /// the two sizes converts it. Without this the preview and the PDF would
    /// disagree about how many characters fit on a line.
    fn print_char_width(&self, setup: &nn_core::print::PageSetup) -> f32 {
        let editor_size = self.settings.font_size.max(1.0);
        self.view.char_width() * setup.font_size / editor_size
    }

    /// Opens the print preview on the active document.
    fn open_print_preview(&mut self) {
        let setup = self.print_setup();
        let char_width = self.print_char_width(&setup);
        let tab = &self.tabs[self.active];
        let name = tab.title();
        let path = tab.editor.doc.path().map(|path| path.to_path_buf());
        let text = tab.editor.doc.text();
        self.print
            .open(&text, &name, path.as_deref(), setup, char_width);
        self.window.request_redraw();
    }

    /// Sends the active document straight to the printer.
    ///
    /// No preview: this is the command for a document you have already printed
    /// once and know the shape of. The result goes to the status bar, because a
    /// print job that silently failed is worse than one that says why.
    fn print_now(&mut self) {
        let setup = self.print_setup();
        let char_width = self.print_char_width(&setup);
        let tab = &self.tabs[self.active];
        let name = tab.title();
        let text = tab.editor.doc.text();
        let fields = nn_core::print::Fields {
            file_name: name.clone(),
            path: tab
                .editor
                .doc
                .path()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
            ..nn_core::print::Fields::now()
        };
        let layout = nn_core::print::layout(&text, &setup, char_width, &fields);
        let pdf = nn_core::pdf::render(&layout, &setup, char_width);
        let message = match nn_app::print::spool(&pdf, &name) {
            Ok(text) => text,
            Err(error) => format!("Print: {error}"),
        };
        self.status.set_message(Some(message));
        self.window.request_redraw();
    }

    /// Records a change made in the preview, so it survives the next launch.
    fn save_print_setup(&mut self) {
        let setup = self.print.setup().clone();
        self.settings.print_paper = setup.paper.key.to_string();
        self.settings.print_landscape = setup.landscape;
        self.settings.print_line_numbers = setup.line_numbers;
        self.settings.save();
    }

    /// Handles a key while the print preview is open.
    ///
    /// The preview is modal: every key is either a command it understands or is
    /// swallowed, so a keystroke cannot reach the document behind the sheet.
    fn handle_print_key(&mut self, event: &winit::event::KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::ArrowRight) | Key::Named(NamedKey::PageDown) => {
                self.print.next_page()
            }
            Key::Named(NamedKey::ArrowLeft) | Key::Named(NamedKey::PageUp) => {
                self.print.previous_page()
            }
            Key::Named(NamedKey::Escape) => self.print.close(),
            Key::Named(NamedKey::Enter) => self.print_from_preview(),
            Key::Character(text) => match text.as_str() {
                "p" | "P" => {
                    self.print.cycle_paper();
                    self.save_print_setup();
                }
                "o" | "O" => {
                    self.print.toggle_orientation();
                    self.save_print_setup();
                }
                "l" | "L" => {
                    self.print.toggle_line_numbers();
                    self.save_print_setup();
                }
                _ => {}
            },
            _ => {}
        }
        self.window.request_redraw();
    }

    /// Prints what the preview is showing.
    fn print_from_preview(&mut self) {
        let pdf = self.print.pdf();
        let name = self.print.name().to_string();
        let message = match nn_app::print::spool(&pdf, &name) {
            Ok(text) => text,
            Err(error) => format!("Print: {error}"),
        };
        self.print.close();
        self.status.set_message(Some(message));
        self.window.request_redraw();
    }

    /// Handles a click while the print preview is open.
    fn handle_print_click(&mut self, x: f64, y: f64) {
        match self.print.control_at(x as f32, y as f32) {
            Some(nn_app::PrintControl::Previous) => self.print.previous_page(),
            Some(nn_app::PrintControl::Next) => self.print.next_page(),
            Some(nn_app::PrintControl::Paper) => {
                self.print.cycle_paper();
                self.save_print_setup();
            }
            Some(nn_app::PrintControl::Orientation) => {
                self.print.toggle_orientation();
                self.save_print_setup();
            }
            Some(nn_app::PrintControl::LineNumbers) => {
                self.print.toggle_line_numbers();
                self.save_print_setup();
            }
            Some(nn_app::PrintControl::Print) => self.print_from_preview(),
            Some(nn_app::PrintControl::Close) => self.print.close(),
            None => {}
        }
        self.window.request_redraw();
    }

    /// Loads the localization the settings name, or English when they name none.
    ///
    /// A file that will not parse leaves the menu in English and says so in the
    /// status bar. Refusing to start over a broken translation would cost the
    /// user their editor for a cosmetic fault.
    fn apply_language(&mut self) {
        let dir = nn_app::paths::config_file("localization");
        self.prefs.set_languages(&Catalog::languages(dir.as_deref()));
        let catalog = match self.settings.language.as_deref() {
            Some(file) => match Catalog::load(dir.as_deref(), file) {
                Ok(catalog) => Some(catalog),
                Err(error) => {
                    self.status
                        .set_message(Some(format!("Language: {error}")));
                    None
                }
            },
            None => None,
        };
        self.menu.set_catalog(catalog.clone());
        self.plugin_list.set_catalog(catalog.clone());
        self.prefs.set_catalog(catalog.clone());
        self.status.set_catalog(catalog.clone());
        self.prompt.set_catalog(catalog.clone());
        self.find.set_catalog(catalog.clone());
        self.find_dialog.set_catalog(catalog.clone());
        self.about.set_catalog(catalog.clone());
        self.toolbar.set_catalog(catalog);
    }

    /// Applies the dialog's settings and redraws, without writing the file.
    fn apply_settings_quiet(&mut self) {
        let settings = self.prefs.settings().clone();
        let system_light = self.system_light;
        let scale = ui_scale(&settings, self.window.scale_factor());
        Self::apply_view_settings(
            &mut self.view,
            self.renderer.font_system(),
            &settings,
            system_light,
            scale,
        );
        if let Some(other) = &mut self.other {
            Self::apply_view_settings(
                &mut other.view,
                self.renderer.font_system(),
                &settings,
                system_light,
                scale,
            );
        }
        self.set_ui_scale(scale);
        self.toolbar.set_enabled(settings.toolbar);
        self.set_function_list(settings.function_list);
        // The search keeps its pattern and options: the preference moves the
        // presentation, not the search.
        self.find.set_floating(settings.find_floating);
        self.menu.set_keymap(settings.keymap.clone());
        // Turning autosave off has to take the snapshot with it. Leaving it
        // behind would restore buffers on the next launch that the user had
        // already decided not to keep.
        if settings.autosave_seconds == 0 && self.settings.autosave_seconds != 0 {
            Snapshot::clear();
        }
        self.settings = settings;
        self.sync_runs();
        self.apply_language();
        self.apply_saved_theme();
        self.reload_context_layout();
        self.resize_view();
    }

    /// Puts the theme named in the settings into the views.
    ///
    /// Read from the file every time rather than cached: the file is the user's,
    /// and an edit to it should take effect on the next start without a second
    /// import. A path that no longer resolves leaves the built-in theme alone,
    /// which is the only sane fallback -- there is nothing to report to a user
    /// who deleted the file on purpose.
    fn apply_saved_theme(&mut self) {
        let Some(path) = self.settings.theme_path.clone() else {
            return;
        };
        let Some(theme) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| nn_app::theme::from_stylers(&text))
        else {
            return;
        };
        self.view.set_theme(theme);
        if let Some(pane) = &mut self.other {
            pane.view.set_theme(theme);
        }
        self.sync_palette();
    }

    /// Whether any open buffer has a file to watch.
    fn watches_disk(&self) -> bool {
        let focused = self.tabs.iter().any(|tab| tab.editor.doc.path().is_some());
        let other = self
            .other
            .as_ref()
            .is_some_and(|pane| pane.tabs.iter().any(|tab| tab.editor.doc.path().is_some()));
        focused || other
    }

    /// Reloads any clean buffer whose file changed on disk, and flags any dirty
    /// one.
    ///
    /// Polling rather than inotify: a watch per file means a descriptor per file,
    /// a thread or a poll loop to service them, and a dependency. One `stat` per
    /// open file per second costs microseconds and cannot fail in a way that
    /// loses data.
    ///
    /// A dirty buffer is never reloaded. The user's unsaved work wins until they
    /// say otherwise, so the file is flagged and the decision is theirs — which
    /// is the whole difference between noticing a change and losing an edit.
    fn poll_disk(&mut self) {
        let mut reloaded = 0;
        let mut conflicts = 0;
        for tab in self.all_tabs() {
            match tab.editor.doc.disk_change() {
                DiskChange::None => continue,
                DiskChange::Conflict => {
                    // A monitored tab is a file being watched, not edited: the
                    // whole point of tail -f is that the newest bytes win, so the
                    // conflict is resolved in the file's favour rather than the
                    // buffer's. Every other tab keeps the user's unsaved work.
                    if !tab.monitoring {
                        tab.editor.doc.note_disk_conflict();
                        conflicts += 1;
                        continue;
                    }
                }
                DiskChange::Reload => {}
            }
            if tab.editor.doc.reload().is_ok() {
                // The text is new, so the styling and the folds are stale. Both
                // are recomputed from the top: a reload is not an edit and there
                // is no earlier line to resume from.
                tab.highlighter.highlight_all(&mut tab.editor.doc);
                tab.folder.apply(&mut tab.editor.doc);
                // The caret may now be past the end of a shorter file.
                let caret = tab.editor.caret();
                tab.editor.set_caret(caret);
                println!(
                    "npp: reloaded {} (changed on disk)",
                    tab.editor.doc.display_name()
                );
                reloaded += 1;
            }
        }
        if reloaded > 0 {
            self.status
                .set_message(Some(format!("Reloaded {reloaded} file(s) changed on disk")));
        }
        if conflicts > 0 {
            self.status.set_message(Some(format!(
                "{conflicts} file(s) changed on disk — save or reload"
            )));
        }
        if reloaded > 0 || conflicts > 0 {
            self.update_title();
            self.window.request_redraw();
        }
    }

    /// Whether any open buffer has unsaved changes.
    fn any_dirty(&self) -> bool {
        let focused = self.tabs.iter().any(|tab| tab.editor.doc.is_dirty());
        let other = self
            .other
            .as_ref()
            .is_some_and(|pane| pane.tabs.iter().any(|tab| tab.editor.doc.is_dirty()));
        focused || other
    }

    /// How long until the next periodic snapshot, or `None` when none is wanted.
    fn autosave_in(&self) -> Option<std::time::Duration> {
        nn_app::autosave_wait(
            self.settings.autosave_seconds,
            self.any_dirty(),
            self.last_snapshot.elapsed(),
        )
    }

    /// How long the event loop may sleep before something needs doing.
    ///
    /// The two timers are independent and both are optional, so the loop sleeps
    /// until whichever comes first. Returning `None` is what keeps an idle
    /// window at zero CPU: with nothing to watch and nothing unsaved there is no
    /// reason to wake at all.
    fn next_wake(&self) -> Option<std::time::Duration> {
        // A room being drawn is a frame timer, and there is nothing else to
        // wait for: the window is the room.
        let now = std::time::Instant::now();
        if let Some(frame) = self.playground.deadline(now) {
            return Some(frame);
        }
        let disk = self.watches_disk().then_some(DISK_POLL);
        // A tooltip waiting out its half second is a third timer. Without it the
        // label would appear on the next unrelated event, which on an idle
        // window is never -- the loop would sleep through the deadline.
        let tip = self.tooltip.deadline(now);
        // A frame owed to the window is the fourth, and the one that matters
        // after an `Alt+Tab`: asleep, the loop would never come back to it.
        let repaint = self.repaint_at.map(|at| at.saturating_duration_since(now));
        // The idle timeout is the fifth: a screensaver that the loop sleeps
        // through is a screensaver that never starts.
        nn_app::soonest(
            nn_app::soonest(disk, self.autosave_in()),
            nn_app::soonest(repaint, nn_app::soonest(tip, self.screensaver_in(now))),
        )
    }

    /// Asks for a frame `after` from now, replacing any earlier promise.
    fn repaint_in(&mut self, after: std::time::Duration) {
        if self.repaint_tries >= REPAINT_TRIES {
            return;
        }
        let at = std::time::Instant::now() + after;
        self.repaint_at = Some(match self.repaint_at {
            Some(when) => when.min(at),
            None => at,
        });
    }

    /// Requests the frame once its deadline has passed.
    fn repaint_due(&mut self) {
        if self
            .repaint_at
            .is_some_and(|at| at <= std::time::Instant::now())
        {
            self.repaint_at = None;
            self.repaint_tries = self.repaint_tries.saturating_add(1);
            self.window.request_redraw();
        }
    }

    /// Whether the focus came back a moment ago, close enough that a key
    /// arriving now is the one that took it away.
    fn switched_in(&self) -> bool {
        self.focus_at
            .is_some_and(|at| at.elapsed() < LEAKED_KEY_GRACE)
    }

    /// Writes every modified buffer into the snapshot directory.
    ///
    /// Only the modified ones: a clean buffer is already on disk, and copying it
    /// would double the size of the snapshot for nothing.
    ///
    /// The text is written before the manifest, so a crash between the two
    /// leaves a manifest that names a file which is not there. That is the safe
    /// order — the restore skips an entry it cannot read, and a manifest written
    /// first would name files that were never written at all.
    fn snapshot_now(&mut self) {
        self.last_snapshot = std::time::Instant::now();
        let mut entries = Vec::new();
        for tab in self.all_tabs() {
            if !tab.editor.doc.is_dirty() {
                continue;
            }
            let index = entries.len();
            if Snapshot::write_buffer(index, &tab.editor.doc.text()).is_err() {
                continue;
            }
            let caret = tab.editor.caret();
            let line = tab.editor.doc.char_to_line(caret);
            let column = caret - tab.editor.doc.line_to_char(line);
            entries.push(SnapshotEntry {
                path: tab.editor.doc.path().map(Path::to_path_buf),
                line,
                column,
                file: Snapshot::file_name(index),
            });
        }
        if entries.is_empty() {
            // Nothing unsaved, so any snapshot on disk is stale. Clearing it
            // here is what stops a clean exit from restoring buffers the user
            // already saved.
            Snapshot::clear();
            return;
        }
        Snapshot { entries }.save();
    }

    /// Reopens the buffers a previous run left unsaved.
    ///
    /// Called once at startup, after the session has been restored. A buffer
    /// whose file is already open is replaced in place rather than opened twice,
    /// because the session and the snapshot describe the same tab and two copies
    /// of one file is how an edit gets lost.
    ///
    /// A buffer whose file has since been deleted comes back untitled. The path
    /// is lost, which is a nuisance; the text is not, which is the promise.
    fn restore_snapshot(&mut self, named: bool) {
        let snapshot = Snapshot::load();
        if snapshot.is_empty() {
            return;
        }
        // A fresh start opens one empty buffer. It is not something the user
        // asked for, so it goes once there is something real to show instead.
        let placeholder = self.tabs.len() == 1
            && self.tabs[0].editor.doc.path().is_none()
            && !self.tabs[0].editor.doc.is_dirty()
            && self.tabs[0].editor.doc.len_chars() == 0;
        let mut restored = 0;
        for entry in &snapshot.entries {
            let Ok(text) = Snapshot::read_buffer(&entry.file) else {
                continue;
            };
            let existing = entry.path.as_ref().and_then(|path| {
                self.tabs
                    .iter()
                    .position(|tab| tab.editor.doc.path() == Some(path.as_path()))
            });
            if let Some(index) = existing {
                // Already open from the session: replace its text rather than
                // opening a second copy of the same file.
                let tab = &mut self.tabs[index];
                tab.editor.doc.set_text(&text);
                // The snapshot holds buffers that were dirty when it was taken,
                // but the file may have been saved since -- by this program in
                // another window, or by anything else. A buffer whose text
                // already matches its file comes back clean, or the marker
                // would be permanent and the snapshot would never empty.
                tab.editor.doc.mark_clean_if_unchanged();
                tab.highlighter.highlight_all(&mut tab.editor.doc);
                tab.folder.apply(&mut tab.editor.doc);
                tab.editor.goto_position(entry.line, entry.column);
                restored += 1;
                continue;
            }
            // A file that has since been deleted comes back untitled. The path
            // is lost, which is a nuisance; the text is not, which is the point.
            let mut doc = entry
                .path
                .as_ref()
                .and_then(|path| Document::from_file(path).ok())
                .unwrap_or_else(Document::new);
            doc.set_text(&text);
            doc.mark_clean_if_unchanged();
            let mut tab = Tab::new(doc, &self.langs);
            tab.editor.goto_position(entry.line, entry.column);
            self.tabs.push(tab);
            restored += 1;
        }
        // Read once. Leaving it would restore the same buffers again on the next
        // launch, on top of whatever the user has done since.
        Snapshot::clear();
        if restored == 0 {
            return;
        }
        if placeholder {
            self.tabs.remove(0);
        }
        // A file named on the command line is the one the user asked to see, so
        // the recovered buffers queue behind it instead of taking the front. The
        // rest of the command line is applied after this, and activates what it
        // opens, so only the first file would have been displaced.
        if !named {
            self.active = self.tabs.len() - 1;
        }
        self.after_tab_change();
        self.status.set_message(Some(format!(
            "Restored {restored} unsaved buffer(s) from the last session"
        )));
        println!("npp: restored {restored} unsaved buffer(s)");
    }

    /// The active tab.
    fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    /// The active tab, mutably.
    fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    /// Every open tab, in both panes.
    ///
    /// The focused pane's tabs first, then the other's. Commands that act on
    /// "all documents" — save all, close all, the disk watch, the session — have
    /// to see both, or a split would quietly hide half the open files.
    fn all_tabs(&mut self) -> impl Iterator<Item = &mut Tab> {
        let State { tabs, other, .. } = self;
        let second = other.as_mut().map(|pane| pane.tabs.iter_mut());
        tabs.iter_mut().chain(second.into_iter().flatten())
    }

    /// Rebuilds the tab strip titles from the open documents.
    fn sync_tabs(&mut self) {
        let tabs: Vec<TabInfo> = self
            .tabs
            .iter()
            .map(|tab| TabInfo {
                title: tab.strip_title(),
                pinned: tab.pinned,
                colour: tab.colour,
                modified: tab.editor.doc.is_dirty(),
            })
            .collect();
        self.tab_bar.set_tabs(tabs, self.active);
    }

    /// Pushes the recent file list into the File menu.
    /// Points the Run menu at the saved commands.
    fn sync_runs(&mut self) {
        let labels: Vec<String> = self
            .settings
            .runs
            .commands()
            .iter()
            .map(|command| command.label())
            .collect();
        self.menu.set_runs(&labels);
    }

    fn sync_recent(&mut self) {
        let names: Vec<String> = self
            .session
            .recent
            .iter()
            .map(|path| match path.file_name() {
                Some(name) => name.to_string_lossy().into_owned(),
                None => path.display().to_string(),
            })
            .collect();
        self.menu.set_recent(&names);
    }

    /// Opens `path` in a new tab and records it as recent.
    /// Opens what the command line asked for, after the first document is up.
    ///
    /// The first file is already open -- `State::new` took it, because the
    /// window has to exist before there is anywhere to put a document. This
    /// adds the rest, reads standard input when `-` was given, and moves the
    /// caret when `--line`/`--column` asked for it.
    fn apply_args(&mut self, args: &nn_app::cli::Args) {
        for path in args.files.iter().skip(1) {
            self.open_path(path);
        }
        if args.stdin {
            let mut text = String::new();
            if std::io::Read::read_to_string(&mut std::io::stdin(), &mut text).is_ok() {
                self.tabs.push(Tab::new(Document::from_text(&text), &self.langs));
                self.active = self.tabs.len() - 1;
                self.after_tab_change();
            }
        }
        if let Some((line, column)) = args.caret() {
            self.tab_mut().editor.goto_position(line, column);
        }
        self.window.request_redraw();
    }

    /// Opens the documents a second process asked for.
    ///
    /// The "already open" rule lives in `open_path`, so a request and a launch
    /// cannot disagree about it.
    fn open_request(&mut self, request: &nn_app::single::Request) {
        for path in &request.paths {
            self.open_path(std::path::Path::new(path));
        }
        if let Some((line, column)) = request.caret() {
            self.tab_mut().editor.goto_position(line, column);
        }
        self.window.request_redraw();
    }

    /// Tells a waiting `--wait` caller that `path` has closed.
    fn signal_closed(&self, path: &std::path::Path) {
        let Some(server) = &self.server else {
            return;
        };
        server.closed(&path.to_string_lossy());
    }

    /// The index of the tab in `tabs` that holds `path`, if one does.
    ///
    /// The comparison is on the stored path rather than on a canonical form,
    /// because a file that has been deleted still has to match the tab that is
    /// holding its text.
    fn tab_holding(tabs: &[Tab], path: &std::path::Path) -> Option<usize> {
        let open: Vec<Option<PathBuf>> = tabs
            .iter()
            .map(|tab| tab.editor.doc.path().map(|p| p.to_path_buf()))
            .collect();
        nn_app::single::find_open(&open, path)
    }

    /// Opens `path` in a new tab, or brings the tab that already holds it to the
    /// front.
    ///
    /// The second half is not a convenience. Two tabs on one file diverge the
    /// moment either is edited and the last save wins, so every way into the
    /// editor -- the command line, File > Open, a drop, a search hit, a request
    /// from a second process -- has to agree that a file is open at most once.
    /// The startup order is what made this worth writing down: the snapshot is
    /// restored before the command line is applied, so `npp notes.txt` on a
    /// file with unsaved work used to come up with the restored text in one tab
    /// and the disk text in the other.
    ///
    /// The other pane is searched too, and focused when it is the one holding
    /// the file, because a split is a view of the same set of documents.
    fn open_path(&mut self, path: &std::path::Path) {
        // Every way in -- the command line, a drop, a dialog answer, a request
        // from a second process -- has to agree on what "this file" means, and
        // the only form that still means the same file after a restart is the
        // absolute one. `npp test/01-object.json` used to leave a tab, a session
        // line and a recent entry all holding `test/01-object.json`, which named
        // a file only until the next launch.
        let path = &nn_app::paths::absolute(path);
        if let Some(index) = Self::tab_holding(&self.tabs, path) {
            self.active = index;
            self.after_tab_change();
            return;
        }
        if let Some(index) = self
            .other
            .as_ref()
            .and_then(|pane| Self::tab_holding(&pane.tabs, path))
        {
            self.focus_other();
            self.active = index;
            self.after_tab_change();
            return;
        }
        // A name that is not there opens a blank page that will create it on
        // the first save; anything else that cannot be read is still an error.
        let existed = path.exists();
        match Document::open(path) {
            Ok(doc) => {
                // A buffer for a file that has never been written is not a
                // recent file yet: the list is a history of what was opened,
                // and the first save is what earns the entry.
                if existed {
                    self.session.push_recent(path);
                    self.sync_recent();
                }
                self.tabs.push(Tab::new(doc, &self.langs));
                self.active = self.tabs.len() - 1;
                self.after_tab_change();
            }
            // A recent entry can outlive the file it names, and a click that
            // does nothing looks like a broken menu rather than a missing file.
            Err(err) => {
                eprintln!("npp: cannot open {}: {err}", path.display());
                self.status
                    .set_message(Some(format!("Cannot open {}: {err}", path.display())));
            }
        }
    }

    /// Opens every file a dialog answered with.
    ///
    /// One place rather than a loop at each call site, because "open several"
    /// has to mean the same thing whether the answer came from the system
    /// dialog or from the built-in browser. `open_path` still decides what
    /// "already open" means, and the last file opened is the one left in front,
    /// which is what a user who picked a list expects to be looking at.
    fn open_files(&mut self, paths: &[std::path::PathBuf]) {
        for path in paths {
            self.open_path(path);
        }
    }

    /// Writes the open tabs and their caret positions to the session file.
    ///
    /// Untitled tabs have no path to restore, so they are dropped; the active
    /// index is remapped to the nearest surviving tab.
    fn save_session(&mut self) {
        let session = self.session_now();
        session.save();
        self.session = session;
    }

    /// The session as it stands: every tab with a path, and the active index.
    ///
    /// Split out of `save_session` so that "save the session" and "save a session
    /// file" cannot disagree about what is open.
    fn session_now(&self) -> Session {
        let mut session = self.session.clone();
        session.tabs.clear();
        let mut active = 0;
        let mut kept = 0;
        // The focused pane's tabs come first, so the active index still points at
        // the document the user was looking at when the session is restored.
        for (index, tab) in self.tabs.iter().enumerate() {
            let Some(path) = tab.editor.doc.path() else {
                continue;
            };
            if index <= self.active {
                active = kept;
            }
            kept += 1;
            session.tabs.push(SessionTab {
                path: path.to_path_buf(),
                line: tab.editor.caret_line(),
                column: tab.editor.caret_column(),
                pinned: tab.pinned,
                colour: tab.colour,
            });
        }
        if let Some(other) = &self.other {
            for tab in &other.tabs {
                let Some(path) = tab.editor.doc.path() else {
                    continue;
                };
                session.tabs.push(SessionTab {
                    path: path.to_path_buf(),
                    line: tab.editor.caret_line(),
                    column: tab.editor.caret_column(),
                    pinned: tab.pinned,
                    colour: tab.colour,
                });
            }
        }
        session.active = active;
        session
    }

    /// Opens an empty document in a new tab.
    fn new_tab(&mut self) {
        self.tabs.push(Tab::new(Document::new(), &self.langs));
        self.active = self.tabs.len() - 1;
        self.after_tab_change();
    }

    /// Pins or unpins the active tab.
    ///
    /// Pinning moves the tab to the front of the strip, after any tabs already
    /// pinned. The move is what makes the pin useful: a pinned tab that stayed
    /// where it was would still be findable, but the strip would not show the
    /// user's own ordering of what matters.
    ///
    /// The tab is moved in `self.tabs` rather than sorted at draw time, so the
    /// strip index and the tab index stay the same number. A strip that
    /// reordered itself would need a mapping from every click back to a tab, and
    /// a mapping that is wrong once is a click that closes the wrong document.
    fn pin_tab(&mut self) {
        let pinned = !self.tabs[self.active].pinned;
        self.tabs[self.active].pinned = pinned;
        if pinned {
            let tab = self.tabs.remove(self.active);
            // After the last pinned tab, so pinning twice keeps the order the
            // user pinned them in.
            let at = self.tabs.iter().take_while(|tab| tab.pinned).count();
            self.tabs.insert(at, tab);
            self.active = at;
        }
        self.after_tab_change();
        let state = if pinned { "Pinned" } else { "Unpinned" };
        self.status
            .set_message(Some(format!("{state} {}", self.tabs[self.active].title())));
    }

    /// Steps the active tab through the colours, wrapping back to none.
    fn cycle_tab_colour(&mut self) {
        let colour = self.tabs[self.active].colour.next();
        self.tabs[self.active].colour = colour;
        self.after_tab_change();
        self.status
            .set_message(Some(format!("Tab colour: {}", colour.label())));
    }

    /// Closes the active tab, keeping at least one open.
    fn close_tab(&mut self) {
        if self.tabs[self.active].pinned {
            self.status
                .set_message(Some("This tab is pinned — unpin it to close it".into()));
            return;
        }
        // Before the tab goes: a `--wait` caller is blocked on this path, and
        // the path is only readable while the tab still exists.
        if let Some(path) = self.tabs[self.active].editor.doc.path() {
            let path = path.to_path_buf();
            self.signal_closed(&path);
        }
        // Notepad++ closes a view with its last document: closing the only tab
        // of one side collapses the split and lets the other side fill the
        // window, rather than leaving an empty pane with an empty tab strip.
        if self.other.is_some() && self.tabs.len() == 1 {
            // The pane that survives is the other one, so its fields move into
            // the focused slot before the split is dropped.
            if let Some(mut kept) = self.other.take() {
                std::mem::swap(&mut self.tabs, &mut kept.tabs);
                std::mem::swap(&mut self.active, &mut kept.active);
                std::mem::swap(&mut self.view, &mut kept.view);
                std::mem::swap(&mut self.tab_bar, &mut kept.tab_bar);
            }
            self.split = Split::None;
            self.focused_second = false;
            self.after_tab_change();
            self.resize_view();
            self.window.request_redraw();
            return;
        }
        if self.tabs.len() == 1 {
            self.tabs[0] = Tab::new(Document::new(), &self.langs);
            self.active = 0;
        } else {
            self.tabs.remove(self.active);
            if self.active >= self.tabs.len() {
                self.active = self.tabs.len() - 1;
            }
        }
        self.after_tab_change();
    }

    /// Saves every tab that has a path, and reports how many were written.
    ///
    /// Untitled tabs are skipped rather than prompted for: a save-all that stops
    /// to ask for a filename is not a save-all.
    fn save_all(&mut self) {
        let mut saved = 0;
        let mut failed = 0;
        let settings = self.settings.clone();
        // A group is only clean once its file is written, so the groups are
        // collected from the tabs that saved rather than from every tab on
        // screen: a failed write has to keep its pencil.
        let mut written: Vec<u64> = Vec::new();
        for tab in self.all_tabs() {
            if tab.editor.doc.path().is_none() {
                continue;
            }
            match write_tab(tab, &settings) {
                Ok(()) => {
                    saved += 1;
                    written.push(tab.group);
                }
                Err(_) => failed += 1,
            }
        }
        for group in written {
            self.clear_group_dirty(group);
        }
        let message = if failed > 0 {
            format!("Saved {saved} file(s), {failed} failed")
        } else {
            format!("Saved {saved} file(s)")
        };
        self.status.set_message(Some(message));
        self.update_title();
    }

    /// Closes every tab for which `keep` returns false.
    ///
    /// Refuses when any tab it would close has unsaved changes, the same way
    /// Close All does and for the same reason: there is no save prompt, so a
    /// bulk close that went ahead would silently drop edits. The message names
    /// the count rather than the files, because the tab strip already shows
    /// which ones are dirty.
    ///
    /// Only the focused pane is touched. A bulk close that reached across a
    /// split would be closing documents the user is not looking at.
    fn close_tabs_where(&mut self, keep: impl Fn(usize) -> bool) {
        // A pinned tab is never doomed, whatever the predicate says. That is the
        // whole promise of pinning: a bulk close cannot take it.
        let doomed: Vec<usize> = (0..self.tabs.len())
            .filter(|&i| !keep(i) && !self.tabs[i].pinned)
            .collect();
        if doomed.is_empty() {
            return;
        }
        let dirty = doomed
            .iter()
            .filter(|&&i| self.tabs[i].editor.doc.is_dirty())
            .count();
        if dirty > 0 {
            self.status.set_message(Some(format!(
                "{dirty} unsaved tab(s) — save or close them first"
            )));
            return;
        }
        // Back to front, so the indices of the tabs still to remove stay valid.
        for &index in doomed.iter().rev() {
            if let Some(path) = self.tabs[index].editor.doc.path() {
                let path = path.to_path_buf();
                self.signal_closed(&path);
            }
            self.tabs.remove(index);
        }
        // The active tab survived, so it only moved down by the number of tabs
        // removed before it.
        self.active -= doomed.iter().filter(|&&i| i < self.active).count();
        self.after_tab_change();
    }

    /// Closes every tab, refusing while any of them has unsaved changes.
    ///
    /// There is no save prompt in this editor, so closing a dirty buffer would
    /// silently lose it. Refusing and saying so is the only safe answer.
    fn close_all(&mut self) {
        let dirty = self.all_tabs().filter(|tab| tab.editor.doc.is_dirty()).count();
        if dirty > 0 {
            self.status.set_message(Some(format!(
                "{dirty} unsaved tab(s) — save or close them first"
            )));
            return;
        }
        // Closing everything collapses the split: an empty pane has no tab strip
        // and no document, and every invariant here assumes `tabs` is non-empty.
        self.other = None;
        self.split = Split::None;
        self.focused_second = false;
        // Pinned tabs survive, which is what pinning is for. They are already at
        // the front of the strip, so keeping the prefix keeps their order.
        let kept = self.tabs.iter().take_while(|tab| tab.pinned).count();
        for tab in &self.tabs[kept..] {
            if let Some(path) = tab.editor.doc.path() {
                let path = path.to_path_buf();
                self.signal_closed(&path);
            }
        }
        self.tabs.truncate(kept);
        if self.tabs.is_empty() {
            self.tabs.push(Tab::new(Document::new(), &self.langs));
        }
        self.active = 0;
        self.after_tab_change();
        self.resize_view();
    }

    /// Hands the active tab's file to the desktop's default application.
    ///
    /// Distinct from `open_containing_folder`: this opens the file, not the
    /// folder, which is what a user checking a rendered HTML page wants.
    fn open_in_default_viewer(&mut self) {
        let Some(path) = self.tab().editor.doc.path().map(|p| p.to_path_buf()) else {
            self.status.set_message(Some("This tab has no file".into()));
            return;
        };
        match nn_app::desktop::open_path(&path) {
            Ok(()) => self
                .status
                .set_message(Some(format!("Opened {}", path.display()))),
            Err(err) => self
                .status
                .set_message(Some(format!("Cannot open: {err}"))),
        }
    }

    /// Opens a terminal in the active tab's directory.
    ///
    /// `shell` names the program to run inside it; `None` means the login shell.
    /// The two menu rows that reach here are Notepad++'s `cmd` and `PowerShell`,
    /// which on this platform are the same terminal with a different program in
    /// it. A missing program is reported rather than silently ignored, because a
    /// terminal that opens and immediately closes looks like a crash.
    fn open_terminal_here(&mut self, shell: Option<&str>) {
        let Some(dir) = self
            .tab()
            .editor
            .doc
            .path()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
        else {
            self.status.set_message(Some("This tab has no folder".into()));
            return;
        };
        match nn_app::desktop::open_terminal(&dir, shell) {
            Ok(()) => self.status.set_message(Some(format!(
                "Opened {} in {}",
                nn_app::desktop::shell_name(shell),
                dir.display()
            ))),
            Err(err) => self
                .status
                .set_message(Some(format!("Cannot open a terminal: {err}"))),
        }
    }

    /// Writes a copy of the active document without moving the tab.
    ///
    /// The tab keeps its own path and its dirty flag: a copy is a copy, and a
    /// user who asked for one did not ask to start editing the new file.
    fn save_copy_as(&mut self, path: &std::path::Path) {
        let text = self.tab().editor.doc.text();
        match std::fs::write(path, text.as_bytes()) {
            Ok(()) => self
                .status
                .set_message(Some(format!("Copied to {}", path.display()))),
            Err(err) => self
                .status
                .set_message(Some(format!("Cannot write {}: {err}", path.display()))),
        }
    }

    /// Renames the active tab's file on disk and follows it.
    ///
    /// A bare name is taken as a sibling of the current file, which is what a
    /// user typing `notes.txt` into a rename box means. A path with a separator
    /// is taken as given, so the file can also be moved.
    fn rename_to(&mut self, path: &std::path::Path) {
        let Some(old) = self.tab().editor.doc.path().map(|p| p.to_path_buf()) else {
            self.status.set_message(Some("This tab has no file".into()));
            return;
        };
        let target = if path.components().count() == 1 {
            old.with_file_name(path)
        } else {
            path.to_path_buf()
        };
        if target == old {
            return;
        }
        if target.exists() {
            self.status
                .set_message(Some(format!("{} already exists", target.display())));
            return;
        }
        match std::fs::rename(&old, &target) {
            Ok(()) => {
                self.tabs[self.active].editor.doc.set_path(&target);
                self.session.push_recent(&target);
                self.sync_recent();
                self.update_title();
                self.status
                    .set_message(Some(format!("Renamed to {}", target.display())));
            }
            Err(err) => self
                .status
                .set_message(Some(format!("Cannot rename: {err}"))),
        }
    }

    /// Moves the active tab's file to the desktop trash and closes the tab.
    ///
    /// The tab is closed rather than left pointing at a file that is no longer
    /// there: a tab whose path does not exist would fail on the next save, and
    /// the user has just said they do not want the file.
    fn move_to_trash(&mut self) {
        let Some(path) = self.tab().editor.doc.path().map(|p| p.to_path_buf()) else {
            self.status.set_message(Some("This tab has no file".into()));
            return;
        };
        if self.tab().editor.doc.is_dirty() {
            self.status
                .set_message(Some("Save or revert the file before deleting it".into()));
            return;
        }
        match nn_app::desktop::trash(&path) {
            Ok(where_to) => {
                self.signal_closed(&path);
                self.close_tab();
                self.status.set_message(Some(format!(
                    "Moved {} to the trash",
                    where_to.display()
                )));
            }
            Err(err) => self
                .status
                .set_message(Some(format!("Cannot move to the trash: {err}"))),
        }
    }

    /// Reads a session file and opens the tabs it names.
    fn load_session(&mut self, path: &std::path::Path) {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) => {
                self.status
                    .set_message(Some(format!("Cannot read {}: {err}", path.display())));
                return;
            }
        };
        let session = nn_app::Session::parse(&text);
        let before = self.tabs.len();
        for tab in &session.tabs {
            self.open_path(&tab.path);
        }
        let opened = self.tabs.len().saturating_sub(before);
        self.status.set_message(Some(format!(
            "Loaded {opened} of {} tab(s) from {}",
            session.tabs.len(),
            path.display()
        )));
    }

    /// Writes the open tabs to a session file the user names.
    ///
    /// Distinct from `save_session`, which writes the session the editor restores
    /// on the next start. This one is a snapshot the user keeps, so it is written
    /// where they said and the live session is left alone.
    fn write_session_file(&mut self, path: &std::path::Path) {
        let session = self.session_now();
        match std::fs::write(path, session.render()) {
            Ok(()) => self.status.set_message(Some(format!(
                "Saved {} tab(s) to {}",
                session.tabs.len(),
                path.display()
            ))),
            Err(err) => self
                .status
                .set_message(Some(format!("Cannot write {}: {err}", path.display()))),
        }
    }

    /// Adopts a directory as the workspace root and shows the panel.
    fn adopt_workspace(&mut self, path: &std::path::Path) {
        if !path.is_dir() {
            self.status
                .set_message(Some(format!("{} is not a folder", path.display())));
            return;
        }
        let tree = Tree::scan(&[path.to_path_buf()]);
        self.folder.show(tree, "Folder as Workspace");
        // The panel is now showing a plain folder, not a project, so the project
        // it was showing before must not come back on the next refresh.
        self.workspace = None;
        self.resize_view();
        self.status
            .set_message(Some(format!("Workspace: {}", path.display())));
    }

    /// Re-reads the active tab from disk, discarding unsaved changes.
    fn reload(&mut self) {
        let Some(path) = self.tab().editor.doc.path().map(|p| p.to_path_buf()) else {
            self.status
                .set_message(Some("Nothing to reload: this tab has no file".into()));
            return;
        };
        match Document::from_file(&path) {
            Ok(mut doc) => {
                doc.set_read_only(self.tab().editor.doc.is_read_only());
                self.tabs[self.active] = Tab::new(doc, &self.langs);
                self.after_tab_change();
                self.status
                    .set_message(Some(format!("Reloaded {}", path.display())));
            }
            Err(err) => self
                .status
                .set_message(Some(format!("Cannot reload: {err}"))),
        }
    }

    /// Copies the active tab's path to the clipboard.
    fn copy_path(&mut self) {
        let Some(path) = self.tab().editor.doc.path().map(|p| p.to_path_buf()) else {
            self.status.set_message(Some("This tab has no file".into()));
            return;
        };
        self.copy_to_clipboard(path.display().to_string());
        self.status
            .set_message(Some(format!("Copied {}", path.display())));
    }

    /// Hands the active tab's directory to the desktop's file manager.
    fn open_containing_folder(&mut self) {
        let Some(dir) = self
            .tab()
            .editor
            .doc
            .path()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
        else {
            self.status.set_message(Some("This tab has no folder".into()));
            return;
        };
        match std::process::Command::new("xdg-open").arg(&dir).spawn() {
            Ok(_) => self
                .status
                .set_message(Some(format!("Opened {}", dir.display()))),
            Err(err) => self
                .status
                .set_message(Some(format!("Cannot open folder: {err}"))),
        }
    }

    /// Changes the line-ending mode the next save will write.
    fn set_eol(&mut self, eol: EolMode) {
        self.tab_mut().editor.doc.set_eol(eol);
        self.update_title();
    }

    /// Reinterprets the active document's bytes as `encoding`.
    ///
    /// The file is not touched: the same bytes are read again under a different
    /// assumption, which is the only way to recover a file whose encoding was
    /// guessed wrong. A buffer with unsaved edits is not re-read — the edits
    /// would be lost — so the encoding is only recorded for the next save and
    /// the status bar says so rather than pretending the text changed.
    fn reinterpret(&mut self, encoding: Encoding) {
        let message = {
            let tab = &mut self.tabs[self.active];
            if tab.editor.doc.path().is_some() && !tab.editor.doc.is_dirty() {
                match tab.editor.doc.reload_as(encoding) {
                    Ok(()) => {
                        tab.highlighter.highlight_all(&mut tab.editor.doc);
                        tab.folder.apply(&mut tab.editor.doc);
                        format!("Reinterpreted as {}", encoding.name())
                    }
                    Err(error) => format!("Could not re-read the file: {error}"),
                }
            } else {
                tab.editor.doc.set_encoding(encoding);
                format!("Encoding set to {} — save to write it", encoding.name())
            }
        };
        self.status.set_message(Some(message));
        self.update_title();
        self.window.request_redraw();
    }

    /// Re-encodes the active document's text as `encoding`.
    ///
    /// The text is unchanged and the buffer becomes dirty, so the next save
    /// writes the same characters as different bytes. That is the difference
    /// between this and `reinterpret`, and it is the difference Notepad++ draws
    /// between the two halves of its Encoding menu.
    fn convert_encoding(&mut self, encoding: Encoding) {
        self.tabs[self.active].editor.doc.set_encoding(encoding);
        self.status.set_message(Some(format!(
            "Converted to {} — save to write it",
            encoding.name()
        )));
        self.update_title();
        self.window.request_redraw();
    }

    /// Opens or closes the document change history panel.
    ///
    /// The rows are rebuilt from the undo stack every time the panel is opened
    /// and after every edit while it is open, so the panel can never show a
    /// history that disagrees with what Ctrl+Z would do.
    fn toggle_change_history(&mut self) {
        if self.history.is_open() {
            self.history.close();
        } else {
            // The rows are built here rather than through `refresh_history`,
            // because that one refuses to run while the panel is closed and
            // `set_rows` is what opens it. Routing the open through it left the
            // panel closable but never openable: the shortcut did nothing.
            self.history.set_rows(self.change_rows());
        }
        self.window.request_redraw();
    }

    /// The active document's undo stack, as rows for the change-history panel.
    fn change_rows(&self) -> Vec<ChangeRow> {
        self.tab()
            .editor
            .history()
            .iter()
            .enumerate()
            .map(|(index, step)| ChangeRow {
                keep: index + 1,
                line: step.line,
                at: step.at,
                what: step.describe(),
            })
            .collect()
    }

    /// Rebuilds the change-history rows from the active document's undo stack.
    fn refresh_history(&mut self) {
        if !self.history.is_open() {
            return;
        }
        let rows = self.change_rows();
        self.history.set_rows(rows);
    }

    /// Opens or closes the floating character grid.
    ///
    /// The panel is a palette rather than a modal: it does not take the keyboard
    /// away from the document, so a character can be picked and then typed
    /// around without closing it. That is the whole reason it floats.
    fn toggle_char_panel(&mut self) {
        self.charset.toggle();
        self.window.request_redraw();
    }

    /// Inserts the character under the pointer, if there is one.
    ///
    /// The panel stays open, because picking one character from a grid is rarely
    /// the whole errand: the box-drawing set is used a row at a time.
    fn insert_hovered_char(&mut self) {
        let Some(c) = self.charset.hovered_char() else {
            return;
        };
        let mut buf = [0u8; 4];
        let text = c.encode_utf8(&mut buf).to_string();
        self.edit(Step::Insert(text.clone()), |editor| editor.insert(&text));
        self.status
            .set_message(Some(format!("Inserted U+{:04X}", c as u32)));
        self.window.request_redraw();
    }

    /// Handles a key while the character grid is open.
    ///
    /// Only the keys the grid itself uses are taken. Everything else falls
    /// through to the document, which is what makes the panel a palette: you can
    /// keep typing with it up.
    fn handle_charset_key(&mut self, event: &winit::event::KeyEvent) -> bool {
        match &event.logical_key {
            Key::Named(NamedKey::ArrowLeft) => self.charset.move_hover(-1, 0),
            Key::Named(NamedKey::ArrowRight) => self.charset.move_hover(1, 0),
            Key::Named(NamedKey::ArrowUp) => self.charset.move_hover(0, -1),
            Key::Named(NamedKey::ArrowDown) => self.charset.move_hover(0, 1),
            Key::Named(NamedKey::Enter) => {
                self.insert_hovered_char();
                return true;
            }
            Key::Named(NamedKey::Escape) => {
                self.charset.close();
                self.window.request_redraw();
                return true;
            }
            _ => return false,
        }
        self.window.request_redraw();
        true
    }

    /// Reverts the active document to the state after `keep` undo steps.
    fn revert_to_step(&mut self, keep: usize) {
        let reverted = self.tab_mut().editor.revert_to(keep);
        if reverted == 0 {
            return;
        }
        self.status
            .set_message(Some(format!("Reverted {reverted} change(s)")));
        self.after_edit(|_| {});
        self.refresh_history();
    }

    /// Enters or leaves distraction-free mode.
    ///
    /// The mode is a view of the same window, so nothing is saved, nothing is
    /// closed, and the caret does not move. The only state that changes is the
    /// chrome policy, which the renderer reads on the next frame.
    /// Steps the tab strip to its next arrangement.
    ///
    /// A cycle rather than three menu entries with a tick beside the current one:
    /// the menu has no check marks yet, and a cycle is one command to bind and one
    /// line to persist. The status bar names the arrangement that was picked, so
    /// the cycle is not silent.
    fn cycle_tab_layout(&mut self) {
        let next = self.tab_bar.layout().next();
        self.set_tab_layout(next);
        self.status
            .set_message(Some(format!("Tab bar: {}", next.label())));
    }

    /// Applies a tab strip arrangement to both panes and remembers it.
    ///
    /// Silent, and deliberately so: this is also the path that restores the
    /// arrangement at startup and the one the preferences dialog takes, and a
    /// status message about the tab bar is not news in either case. Announcing it
    /// here put "Tab bar: Single Row" in the status bar of every fresh window.
    /// `cycle_tab_layout` is the caller that says something, because that is the
    /// one the user just asked for.
    fn set_tab_layout(&mut self, layout: TabLayout) {
        self.tab_bar.set_layout(layout);
        if let Some(other) = &mut self.other {
            other.tab_bar.set_layout(layout);
        }
        self.settings.tab_layout = layout;
        self.window.request_redraw();
    }

    /// Opens or closes the playground.
    fn toggle_playground(&mut self) {
        if self.playground.is_active() {
            self.playground.leave();
        } else if self.settings.playground {
            self.open_playground();
        }
        self.window.request_redraw();
    }

    /// Opens the room, on a seed nothing else has used.
    fn open_playground(&mut self) {
        self.playground_visits = self.playground_visits.wrapping_add(1);
        let clock = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_secs())
            .unwrap_or(0);
        let seed = PLAYGROUND_SEED
            ^ clock.rotate_left(17)
            ^ self
                .playground_visits
                .wrapping_mul(0x2545_F491_4F6C_DD1D);
        self.playground
            .enter(seed, std::time::Instant::now());
        self.window.request_redraw();
    }

    /// Escape leaves; every other key only says that someone is still here.
    fn handle_playground_key(&mut self, event: &winit::event::KeyEvent) {
        let now = std::time::Instant::now();
        self.playground.activity(now);
        let escape = matches!(
            event.logical_key,
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        );
        if escape {
            self.playground.leave();
            self.window.request_redraw();
        }
    }

    /// Runs one frame of the room if one is due, and opens the room when the
    /// window has been idle long enough.
    ///
    /// Called from `about_to_wait`, which is the one place that runs between
    /// batches of events and before the loop sleeps.
    fn tick_playground(&mut self) {
        let now = std::time::Instant::now();
        if self.playground.is_active() {
            if self.playground.tick(now) {
                self.window.request_redraw();
            }
            return;
        }
        if self.screensaver_in(now) == Some(std::time::Duration::ZERO) {
            self.open_playground();
        }
    }

    /// How long until the idle window is long enough to open the playground.
    ///
    /// `None` when the mode is switched off, when the timeout is zero, or while
    /// the room is already open.
    fn screensaver_in(&self, now: std::time::Instant) -> Option<std::time::Duration> {
        if !self.settings.playground
            || self.settings.playground_after == 0
            || self.playground.is_active()
        {
            return None;
        }
        let after = std::time::Duration::from_secs(self.settings.playground_after as u64 * 60);
        Some(after.saturating_sub(self.playground.idle(now)))
    }

    fn toggle_distraction_free(&mut self) {
        self.chrome.toggle();
        self.status.set_message(Some(
            if self.chrome.is_distraction_free() {
                "Distraction Free Mode: move the pointer to the top edge for the menu"
            } else {
                "Normal mode"
            }
            .to_string(),
        ));
        self.window.request_redraw();
    }

    /// Flips the window between windowed and borderless full screen.
    ///
    /// Borderless rather than exclusive: exclusive full screen changes the
    /// display mode, which on a multi-monitor desktop blanks the other screens
    /// and takes a moment to come back. An editor has no use for that.
    fn toggle_full_screen(&mut self) {
        self.full_screen = !self.full_screen;
        let mode = self.full_screen.then_some(Fullscreen::Borderless(None));
        self.window.set_fullscreen(mode);
    }

    /// Flips the active tab's edit guard.
    fn toggle_read_only(&mut self) {
        let doc = &mut self.tabs[self.active].editor.doc;
        let next = !doc.is_read_only();
        doc.set_read_only(next);
        self.status.set_message(Some(
            if next { "Read only" } else { "Editable" }.to_string(),
        ));
    }

    /// Highlights every occurrence of the selection, or of the word under the
    /// caret when nothing is selected.
    fn mark_all(&mut self) {
        let editor = &self.tabs[self.active].editor;
        let needle = if editor.selection_len() > 0 {
            editor.selected_text()
        } else {
            editor
                .highlight_word()
                .map(|range| editor.doc.slice(range))
                .unwrap_or_default()
        };
        if needle.is_empty() {
            self.status.set_message(Some("Nothing to mark".into()));
            return;
        }
        let ranges = self.tabs[self.active].editor.find_all(&needle, 20_000);
        let count = ranges.len();
        self.view.set_matches(ranges, None);
        self.status
            .set_message(Some(format!("Marked {count} occurrence(s)")));
    }

    /// Puts the caret on the bracket that balances the one next to it.
    fn goto_brace(&mut self) {
        let editor = &mut self.tabs[self.active].editor;
        let Some((open, close)) = editor.matching_brace() else {
            return;
        };
        let target = if editor.caret() == open { close } else { open };
        editor.set_caret(target);
        editor.ensure_caret_visible();
        self.refresh_brace();
        self.window.request_redraw();
    }

    /// Inserts `text` at the caret's column on every line the selection covers.
    fn column_insert(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let editor = &self.tabs[self.active].editor;
        let column = editor.caret_column();
        let (first, last) = editor.selected_lines();
        let mut edits = Vec::with_capacity(last - first + 1);
        for line in first..=last {
            let start = editor.doc.line_to_char(line);
            let at = start + column.min(editor.doc.line_len(line));
            edits.push((at..at, text.to_string()));
        }
        self.after_edit(|ed| {
            ed.replace_ranges(&edits);
        });
    }

    /// Switches to tab `index`.
    fn switch_tab(&mut self, index: usize) {
        if index < self.tabs.len() && index != self.active {
            self.active = index;
            self.after_tab_change();
        }
    }

    /// Moves to the next or previous tab, wrapping.
    fn cycle_tab(&mut self, forward: bool) {
        let n = self.tabs.len();
        if n <= 1 {
            return;
        }
        let next = if forward {
            (self.active + 1) % n
        } else {
            (self.active + n - 1) % n
        };
        self.switch_tab(next);
    }

    /// Refreshes everything that depends on which tab is active.
    /// The window as the accessibility tree sees it.
    ///
    /// Everything is copied out of the widgets here rather than borrowed,
    /// because the tree is built while the adapter holds a mutable borrow of
    /// itself and the widgets are borrowed immutably: one snapshot struct is
    /// what lets both happen at once.
    fn a11y_snapshot(&self) -> nn_app::a11y::Snapshot {
        let editor = &self.tabs[self.active].editor;
        nn_app::a11y::Snapshot {
            title: self.window_title(),
            menus: self.menu.rows(),
            tabs: self
                .tabs
                .iter()
                .enumerate()
                .map(|(index, tab)| nn_app::a11y::TabRow {
                    name: tab.editor.doc.display_name(),
                    selected: index == self.active,
                })
                .collect(),
            // Only what the tree can carry: the whole document here would be a
            // copy of the file on every accessibility update, and `a11y` cuts it
            // to this size a moment later anyway.
            truncated: editor.doc.len_chars() > nn_app::a11y::MAX_TEXT,
            text: editor.doc.text_upto(nn_app::a11y::MAX_TEXT),
            caret_line: editor.caret_line(),
            status: self.status.text(),
            playground: self.playground.is_active().then(|| {
                "Playground. Press Escape to return to the document.".to_string()
            }),
        }
    }

    /// A value that changes whenever the tree would.
    ///
    /// The tree is rebuilt from the whole document, so it must not be rebuilt
    /// every frame: at 120 Hz that is 120 copies of the file per second for a
    /// screen reader that is not even running. The signature is the cheap
    /// question -- has anything the tree shows changed? -- and the document's
    /// own revision counter answers most of it without touching the text.
    fn a11y_signature(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.active.hash(&mut hasher);
        self.tabs.len().hash(&mut hasher);
        let editor = &self.tabs[self.active].editor;
        editor.doc.id().hash(&mut hasher);
        editor.doc.revision().hash(&mut hasher);
        editor.caret_line().hash(&mut hasher);
        self.status.text().hash(&mut hasher);
        self.playground.is_active().hash(&mut hasher);
        // The menu is hashed by its labels rather than by a counter: a language
        // change and a new recent file both alter the tree without touching the
        // document.
        for menu in self.menu.rows() {
            menu.title.hash(&mut hasher);
            for item in &menu.items {
                item.label.hash(&mut hasher);
                item.shortcut.hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    /// Sends the tree if anything it shows has changed.
    fn sync_a11y(&mut self) {
        let signature = self.a11y_signature();
        if self.a11y_signature == Some(signature) {
            return;
        }
        let snapshot = self.a11y_snapshot();
        let mut sent = false;
        self.a11y.update_if_active(|| {
            sent = true;
            nn_app::a11y::tree(&snapshot)
        });
        // Only remember the signature when the update actually went out. An
        // inactive adapter drops it, and recording it anyway would leave the
        // tree stale for the whole time the screen reader was off.
        if sent {
            self.a11y_signature = Some(signature);
        }
    }

    /// Runs a command that came from the accessibility tree.
    fn run_a11y_command(&mut self, command: nn_app::a11y::Command) {
        use nn_app::a11y::Command;
        match command {
            Command::OpenMenu(index) => self.menu.open_menu(index),
            Command::RunItem(menu, item) => {
                // Looked up by position rather than carried in the command: the
                // menu can have been rebuilt between the tree being sent and
                // the click arriving, and a stale action would run the wrong
                // command.
                let action = self
                    .menu
                    .rows()
                    .get(menu)
                    .and_then(|row| row.items.get(item))
                    .map(|row| row.action);
                if let Some(action) = action {
                    self.menu.close();
                    self.run(action);
                }
            }
            Command::SelectTab(index) => {
                if index < self.tabs.len() {
                    self.active = index;
                    self.after_tab_change();
                }
            }
            Command::FocusText => self.window.focus_window(),
        }
        self.window.request_redraw();
    }

    fn after_tab_change(&mut self) {
        self.update_title();
        self.sync_palette();
        // A closed bar with retained highlights is still tracking the text, so
        // the test is `is_active` rather than `open`.
        if self.find.is_active() {
            self.refresh_find();
        }
        self.refresh_brace();
        self.refresh_smart();
        self.refresh_symbols();
        self.window.request_redraw();
    }

    /// Reflects the document's name and dirty state in the window title.
    /// The window title, as the window shows it.
    ///
    /// Built here rather than read back from the window: winit's `title()`
    /// returns the attributes the window was created with, not the last string
    /// handed to `set_title`, so on X11 it answers with the empty string. One
    /// function means the title bar and the accessibility tree cannot disagree.
    ///
    /// The suffix is the product name, not the binary name. A title bar is read
    /// by a person looking at a list of windows, and `Neko++` is what the
    /// launcher, the desktop entry and the About row all say; `npp` is what a
    /// shell types, and a shell is not looking at the title bar.
    fn window_title(&self) -> String {
        let doc = &self.tab().editor.doc;
        let marker = if doc.is_dirty() { " *" } else { "" };
        format!("{}{} — Neko++", doc.display_name(), marker)
    }

    fn update_title(&mut self) {
        let title = self.window_title();
        self.window.set_title(&title);
        self.sync_tabs();
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.gpu.resize(width, height);
        self.resize_view();
        self.renderer.resize(&self.gpu.queue, width, height);
    }

    fn render(&mut self) {
        self.sync_clones();
        self.sync_scroll();
        let frame = match self.gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            // The surface outlived its configuration -- a resize the driver saw
            // before we did, or a move to another output. Configuring is the
            // whole fix, but the frame is gone, so the redraw has to be asked
            // for again. Without that request the window keeps whatever was last
            // presented until some unrelated event happens to arrive, which is
            // indistinguishable from the window freezing.
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.gpu
                    .surface
                    .configure(&self.gpu.device, &self.gpu.config);
                self.window.request_redraw();
                return;
            }
            // Lost is not the same as outdated: the surface itself is gone, so
            // it has to be recreated before it can be configured. `recover` does
            // both and reports whether it worked.
            wgpu::CurrentSurfaceTexture::Lost => {
                if self.gpu.recover() {
                    self.window.request_redraw();
                } else {
                    self.status
                        .set_message(Some("The window surface was lost".into()));
                }
                return;
            }
            // A timeout means the driver had no image ready. Skipping the frame
            // is correct; skipping it forever is not, so ask again.
            wgpu::CurrentSurfaceTexture::Timeout => {
                self.window.request_redraw();
                return;
            }
            // Occluded must not retry at once: the window is not visible, so a
            // redraw would spin the loop for nothing. Retrying later is not the
            // same thing, and the comment the other arms carry applies here too
            // -- skipping a frame forever is not an option. winit's own redraw
            // on the way back is a promise the surface does not have to keep: on
            // a compositing desktop the answer lags the window, so the frame
            // asked for on the way in can come back occluded as well, and then
            // nothing asks again. The window keeps the frame it had, which is
            // the text without the selection that was on it. One frame shortly
            // after breaks that, and a few at most when the window really is
            // hidden.
            wgpu::CurrentSurfaceTexture::Occluded => {
                self.repaint_in(OCCLUDED_RETRY);
                return;
            }
            // A validation error inside the acquire. Retrying would repeat it,
            // so it is reported once and the frame is dropped.
            wgpu::CurrentSurfaceTexture::Validation => {
                eprintln!("npp: surface acquire failed validation");
                return;
            }
        };
        let target = frame.texture.create_view(&Default::default());
        let active = self.active;
        let width = self.gpu.config.width as f32;
        let height = self.gpu.config.height as f32;
        let split = self.split;
        let ratio = self.ratio;

        // One `PaneRef` per pane, built from disjoint field borrows so the
        // renderer can hold both at once. The focused pane is the `State`'s own
        // fields; the other is behind `Option`.
        let mut panes = vec![PaneRef {
            editor: &mut self.tabs[active].editor,
            view: &mut self.view,
            tabs: &mut self.tab_bar,
        }];
        if let Some(other) = &mut self.other {
            let other_active = other.active;
            panes.push(PaneRef {
                editor: &mut other.tabs[other_active].editor,
                view: &mut other.view,
                tabs: &mut other.tab_bar,
            });
        }

        self.renderer.render(
            &self.gpu.device,
            &self.gpu.queue,
            &target,
            &mut panes,
            split,
            ratio,
            self.focused_second,
            &mut self.find,
            &mut self.find_dialog,
            &mut self.menu,
            &mut self.toolbar,
            &mut self.prompt,
            &mut self.status,
            &mut self.results,
            &mut self.history,
            &mut self.charset,
            &mut self.functions,
            &mut self.folder,
            &mut self.complete,
            &mut self.param_hint,
            &mut self.tooltip,
            &mut self.context,
            &mut self.peeker,
            &mut self.about,
            &mut self.prefs,
            &mut self.plugin_list,
            &mut self.print,
            &mut self.styles,
            &mut self.shortcuts,
            &mut self.browser,
            &mut self.playground,
            &self.chrome,
            self.ui.line_height,
            self.cursor.1 as f32,
            width,
            height,
        );
        self.gpu.queue.present(frame);
        // A frame reached the screen, so the window is no longer behind on
        // anything a retry was going to fix.
        self.repaint_at = None;
        self.repaint_tries = 0;
    }

    /// Runs `f`, records it in the macro, then redraws.
    ///
    /// Every keystroke that changes the document goes through here rather than
    /// `after_edit` directly, which is what makes the recorder complete without
    /// a second dispatch table to keep in sync.
    fn edit(&mut self, step: Step, f: impl FnOnce(&mut Editor)) {
        self.recorder.record(step);
        if self.recorder.is_recording() {
            self.status.set_recording(Some(self.recorder.len()));
        }
        self.after_edit(f);
    }

    /// Runs `f`, then redraws and refreshes the title.
    fn after_edit(&mut self, f: impl FnOnce(&mut Editor)) {
        let tab = &mut self.tabs[self.active];
        let before = tab.editor.doc.char_to_line(tab.editor.caret());
        f(&mut tab.editor);
        // An edit can only change styling from the first line it touched
        // downwards, so re-lex from there and let the highlighter stop as soon
        // as the lexer state converges.
        let after = tab.editor.doc.char_to_line(tab.editor.caret());
        let from = before.min(after);
        if !tab.editor.doc.is_large() {
            tab.highlighter.highlight_from(&mut tab.editor.doc, from);
            // Folding reads the styles the highlighter just wrote, so it runs
            // second.
            tab.folder.apply_from(&mut tab.editor.doc, from);
        }
        tab.editor.ensure_caret_visible();
        if self.find.open {
            self.refresh_find();
        }
        self.refresh_brace();
        self.refresh_smart();
        self.refresh_symbols();
        self.refresh_history();
        // An open hint follows the caret; a closed one stays closed, so the
        // hint never appears on its own while you are typing prose.
        if self.param_hint.is_open() {
            self.follow_param_hint();
        }
        self.update_title();
        self.window.request_redraw();
    }

    /// Adds one caret above or below the current stack.
    ///
    /// Routed through `after_edit` rather than `edit`: a caret is not a document
    /// change, so there is nothing for the macro recorder to replay, but the
    /// view still has to scroll the new caret into sight and the brace and word
    /// highlights still follow the primary caret.
    fn grow_caret(&mut self, above: bool) {
        let added = if above {
            self.tab_mut().editor.add_caret_above()
        } else {
            self.tab_mut().editor.add_caret_below()
        };
        if added {
            self.after_edit(|_| {});
        }
    }

    /// Recomputes the bracket highlight from the caret.
    ///
    /// Called after anything that can move the caret. The scan is bounded, so
    /// this is cheap enough to run on every keystroke rather than only when the
    /// character under the caret is a bracket.
    fn refresh_brace(&mut self) {
        let brace = self.tab().editor.matching_brace();
        self.view.set_brace(brace);
    }

    /// Recomputes the smart highlight from the caret.
    ///
    /// Notepad++ lights up every other occurrence of the word you are standing
    /// on. Bounded at 2000 ranges: a one-letter word in a large file would
    /// otherwise queue a rectangle per character, and past a few hundred the
    /// highlight stops being information.
    fn refresh_smart(&mut self) {
        // Every occurrence of the word means a scan of the whole document, which
        // on a large file is the one thing that would make typing stutter.
        if self.tab().editor.doc.is_large() {
            return;
        }
        let word = self.tab().editor.highlight_word();
        let text = word
            .as_ref()
            .map(|range| self.tab().editor.doc.slice(range.clone()))
            .unwrap_or_default();
        let key = (self.active, self.tab().editor.doc.revision(), text.clone());
        if key == self.smart_key {
            return;
        }
        self.smart_key = key;
        let ranges = if text.chars().count() < 2 {
            Vec::new()
        } else {
            self.tab().editor.occurrences(&text, 2000)
        };
        self.view.set_smart(ranges);
    }

    /// Rescans the active document for the function list.
    ///
    /// Keyed on the document revision, so a frame that only moved the caret or
    /// scrolled costs nothing. The scan is one pass over the text, but on a
    /// 20k-line file that is still work worth skipping.
    fn refresh_symbols(&mut self) {
        if !self.functions.is_open() || self.tab().editor.doc.is_large() {
            return;
        }
        let key = (self.active, self.tab().editor.doc.revision());
        if key == self.symbols_key {
            return;
        }
        self.symbols_key = key;
        let symbols = scan_symbols(&self.tab().editor.doc.text());
        self.functions.set_symbols(symbols);
    }

    /// Opens the word-completion popup for the word before the caret.
    fn open_completion(&mut self) {
        self.open_completion_in(CompletionMode::Word);
    }

    /// Opens the popup in `mode`.
    ///
    /// The index is rebuilt on every request rather than cached: a completion is
    /// a keystroke, not a frame, and a cache would have to be invalidated on
    /// every edit.
    fn open_completion_in(&mut self, mode: CompletionMode) {
        // Every mode but path completion copies the document, which a large file
        // cannot afford per keystroke.
        if mode != CompletionMode::Path && self.tab().editor.doc.is_large() {
            self.status
                .set_message(Some("Completion is off for large files".into()));
            return;
        }
        self.completion_mode = mode;
        if !self.fill_completion(mode) {
            self.complete.close();
        }
        self.window.request_redraw();
    }

    /// Fills the popup for `mode` and reports whether it found anything.
    ///
    /// One function for all four modes because the differences are only in what
    /// the candidate list is built from; the anchor, the cap and the popup are
    /// the same. That is also what lets `refilter_completion` be a one-liner.
    fn fill_completion(&mut self, mode: CompletionMode) -> bool {
        let text = self.tab().editor.doc.text();
        let caret = self.tab().editor.caret();
        let items = match mode {
            CompletionMode::Word => {
                let Some(range) = prefix_range(&text, caret) else {
                    return false;
                };
                let prefix = &text[range];
                let words = collect_words(&text, 20_000);
                matching_words(prefix, &words, MAX_CANDIDATES)
                    .into_iter()
                    .map(Item::word)
                    .collect()
            }
            CompletionMode::Document => {
                let Some(range) = prefix_range(&text, caret) else {
                    return false;
                };
                let prefix = &text[range];
                let words = self.pooled_words();
                matching_words(prefix, &words, MAX_CANDIDATES)
                    .into_iter()
                    .map(Item::word)
                    .collect()
            }
            CompletionMode::Function => {
                let Some(range) = prefix_range(&text, caret) else {
                    return false;
                };
                let prefix = text[range].to_lowercase();
                let mut seen: Vec<String> = Vec::new();
                let mut items: Vec<Item> = Vec::new();
                for function in scan_functions(&text) {
                    if function.name.to_lowercase() == prefix
                        || !function.name.to_lowercase().starts_with(&prefix)
                        || seen.contains(&function.name)
                    {
                        continue;
                    }
                    seen.push(function.name.clone());
                    items.push(Item::function(&function.name, &function.params));
                    if items.len() >= MAX_CANDIDATES {
                        break;
                    }
                }
                items
            }
            CompletionMode::Path => {
                let Some(range) = path_prefix(&text, caret) else {
                    return false;
                };
                let fragment = text[range].to_string();
                let (dir, stem) = split_fragment(&fragment);
                let base = self
                    .tab()
                    .editor
                    .doc
                    .path()
                    .and_then(|path| path.parent())
                    .map(std::path::Path::to_path_buf);
                let resolved = resolve_dir(base.as_deref(), dir);
                complete_path(&resolved, stem, MAX_CANDIDATES)
                    .into_iter()
                    .map(|name| Item {
                        label: name.clone(),
                        insert: format!("{dir}{name}"),
                        back: 0,
                    })
                    .collect()
            }
        };
        if items.is_empty() {
            return false;
        }
        let (x, y) = self.view.caret_point(&mut self.tabs[self.active].editor);
        let line_height = self.view.line_height();
        self.complete.show(items, (x, y + line_height));
        true
    }

    /// The words of every open document, for document completion.
    ///
    /// Large documents are skipped and the pool stops growing at a few megabytes:
    /// document completion is a convenience, and copying a hundred megabytes of
    /// text on every keystroke would make it a liability.
    fn pooled_words(&self) -> Vec<String> {
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
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        collect_words_many(&refs, 20_000)
    }

    /// Re-filters the open popup after a character was typed.
    fn refilter_completion(&mut self) {
        if !self.complete.is_open() {
            return;
        }
        let mode = self.completion_mode;
        if !self.fill_completion(mode) {
            self.complete.close();
        }
    }

    /// Replaces the word before the caret with the highlighted candidate.
    ///
    /// The insertion is not the label: a function row shows its signature but
    /// inserts `name()`, and the caret then steps back between the parentheses so
    /// the first argument can be typed straight away.
    fn accept_completion(&mut self) {
        let Some(item) = self.complete.selected().cloned() else {
            self.complete.close();
            return;
        };
        let text = self.tab().editor.doc.text();
        let caret = self.tab().editor.caret();
        let range = match self.completion_mode {
            CompletionMode::Path => path_prefix(&text, caret),
            _ => prefix_range(&text, caret),
        };
        let Some(range) = range else {
            self.complete.close();
            return;
        };
        let start = self.tab().editor.doc.byte_to_char(range.start);
        let end = self.tab().editor.doc.byte_to_char(range.end);
        self.complete.close();
        // Recorded as separate steps so a replayed macro leaves the caret where
        // accepting left it, inside the parentheses.
        self.recorder.record(Step::Insert(item.insert.clone()));
        for _ in 0..item.back {
            self.recorder.record(Step::Move(Motion::CharLeft, false));
        }
        if self.recorder.is_recording() {
            self.status.set_recording(Some(self.recorder.len()));
        }
        self.after_edit(|ed| {
            ed.select_range(start..end);
            ed.insert(&item.insert);
            for _ in 0..item.back {
                ed.move_caret(Motion::CharLeft, false);
            }
        });
    }

    /// Shows the signature of the call the caret is inside, or hides the hint.
    ///
    /// The hint is not shown on every keystroke: it appears when `(` is typed
    /// after a name the document declares, and then follows the caret until the
    /// caret leaves the call. A hint that popped up while you were typing prose
    /// would be noise, and Notepad++'s own "on input" setting exists because of
    /// exactly that.
    fn follow_param_hint(&mut self) {
        if self.tab().editor.doc.is_large() {
            self.param_hint.close();
            return;
        }
        let text = self.tab().editor.doc.text();
        let caret = self.tab().editor.caret();
        let Some(call) = call_at(&text, caret) else {
            self.param_hint.close();
            return;
        };
        let Some(params) = params_of(&text, &call.name) else {
            self.param_hint.close();
            return;
        };
        let signature = format!("{}({params})", call.name);
        // The active argument is highlighted inside the signature, so its range
        // shifts by the name and the opening parenthesis.
        let offset = call.name.len() + 1;
        let active = arg_range(&params, call.arg).map(|r| offset + r.start..offset + r.end);
        let (x, y) = self.view.caret_point(&mut self.tabs[self.active].editor);
        let line_height = self.view.line_height();
        self.param_hint.show(signature, active, (x, y + line_height));
    }

    /// Shows or hides the function list and reflows the text column.
    /// Opens the preferences dialog on the settings in force.
    fn open_prefs(&mut self) {
        self.prefs.open(&self.settings);
        self.window.request_redraw();
    }

    /// Applies the dialog's settings, writes them, and redraws.
    ///
    /// Called after every change rather than on close, so the editor updates
    /// live: a font size you cannot see until you dismiss the dialog is a font
    /// size you have to guess at.
    fn apply_settings(&mut self) {
        self.apply_settings_quiet();
        self.settings.save();
        self.window.request_redraw();
    }

    /// Draws the text and the furniture for a display of density `display`.
    ///
    /// One call rather than two, because a window that moved to another screen
    /// with the text at the old size and the menu bar at the new one is worse
    /// than either answer on its own.
    fn rescale(&mut self, display: f64) {
        let scale = ui_scale(&self.settings, display);
        let settings = self.settings.clone();
        let system_light = self.system_light;
        Self::apply_view_settings(
            &mut self.view,
            self.renderer.font_system(),
            &settings,
            system_light,
            scale,
        );
        if let Some(other) = &mut self.other {
            Self::apply_view_settings(
                &mut other.view,
                self.renderer.font_system(),
                &settings,
                system_light,
                scale,
            );
        }
        self.set_ui_scale(scale);
    }

    /// Draws the window furniture at `scale` from now on.
    ///
    /// Every widget shapes its labels when it is made and keeps them, so a new
    /// density is a reshape of each of them rather than a number the renderer
    /// reads at paint time. The toolbar is the exception and needs no telling: it
    /// takes the row height as a parameter and holds no text of its own, and its
    /// icons are rasterised afresh whenever the size they are drawn at changes.
    ///
    /// The document text is not here: it follows the display through
    /// `apply_view_settings`, which the caller has already run by this point.
    fn set_ui_scale(&mut self, scale: f32) {
        let ui = UiMetrics::at(scale);
        if ui == self.ui {
            return;
        }
        self.ui = ui;
        let metrics = ui.metrics();
        self.menu.set_metrics(metrics);
        self.prompt.set_metrics(metrics);
        self.status.set_metrics(metrics);
        self.results.set_metrics(metrics);
        self.history.set_metrics(metrics);
        self.charset.set_metrics(metrics);
        self.functions.set_metrics(metrics);
        self.folder.set_metrics(metrics);
        self.complete.set_metrics(metrics);
        self.param_hint.set_metrics(metrics);
        self.tooltip.set_metrics(metrics);
        self.context.set_metrics(metrics);
        self.peeker.set_metrics(metrics);
        self.about.set_metrics(metrics);
        self.prefs.set_metrics(metrics);
        self.plugin_list.set_metrics(metrics);
        self.print.set_metrics(metrics);
        self.styles.set_metrics(metrics);
        self.shortcuts.set_metrics(metrics);
        self.browser.set_metrics(metrics);
        self.find.set_metrics(metrics);
        self.find_dialog.set_metrics(metrics);
        self.tab_bar.set_metrics(metrics);
        if let Some(other) = &mut self.other {
            other.tab_bar.set_metrics(metrics);
        }
        self.resize_view();
        self.window.request_redraw();
    }

    /// Shows or hides the function list, scanning it when it opens.
    fn set_function_list(&mut self, open: bool) {
        if open == self.functions.is_open() {
            return;
        }
        if open {
            let symbols = scan_symbols(&self.tab().editor.doc.text());
            self.functions.set_symbols(symbols);
            self.symbols_key = (self.active, self.tab().editor.doc.revision());
        } else {
            self.functions.close();
        }
    }

    /// Re-lays out the view after something changed its geometry.
    fn resize_view(&mut self) {
        let (first, second) = self.pane_rects();
        self.view
            .resize(&mut self.tabs[self.active].editor, first.width, first.height);
        if let (Some(other), Some(rect)) = (&mut self.other, second) {
            let active = other.active;
            other
                .view
                .resize(&mut other.tabs[active].editor, rect.width, rect.height);
        }
    }

    /// Handles a key while the preferences dialog is open.
    ///
    /// Returns whether the key was consumed. Every key is: a modal that let
    /// typing through to the document would be a modal you cannot trust.
    fn handle_prefs_key(&mut self, event: &winit::event::KeyEvent) -> bool {
        // An open list takes the keys first, including Escape, so closing it
        // does not also close the dialog it lives in.
        if self.prefs.picker_open() {
            let changed = match &event.logical_key {
                Key::Named(NamedKey::ArrowDown) => self.prefs.picker_move(1),
                Key::Named(NamedKey::ArrowUp) => self.prefs.picker_move(-1),
                // A list taller than the window is still a list you have to be
                // able to reach the end of, so the paging keys move the
                // highlight rather than the view: the highlight is the view.
                Key::Named(NamedKey::PageDown) => self.prefs.picker_page(1),
                Key::Named(NamedKey::PageUp) => self.prefs.picker_page(-1),
                Key::Named(NamedKey::Home) => self.prefs.picker_home(),
                Key::Named(NamedKey::End) => self.prefs.picker_end(),
                // The highlight is already on screen, so Enter does not need to
                // apply it: it is the key that writes it down. The file is worth
                // rewriting even when the entry is the one the list opened on,
                // because the value in force is only ever as new as the last
                // write.
                Key::Named(NamedKey::Enter) | Key::Named(NamedKey::ArrowRight) => {
                    self.prefs.picker_choose();
                    self.apply_settings();
                    false
                }
                // Escape is the undo of a list: the value the list opened on
                // goes back, and the file, untouched by a preview, already
                // holds it.
                Key::Named(NamedKey::Escape) | Key::Named(NamedKey::ArrowLeft) => {
                    self.prefs.close_picker()
                }
                _ => false,
            };
            if changed {
                self.apply_settings_quiet();
            }
            self.window.request_redraw();
            return true;
        }
        match &event.logical_key {
            Key::Named(NamedKey::ArrowDown) => self.prefs.move_selection(1),
            Key::Named(NamedKey::ArrowUp) => self.prefs.move_selection(-1),
            Key::Named(NamedKey::ArrowLeft) => {
                if self.prefs.adjust(-1) {
                    self.apply_settings();
                }
            }
            Key::Named(NamedKey::ArrowRight) => {
                if self.prefs.adjust(1) {
                    self.apply_settings();
                }
            }
            Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Space) => {
                if self.prefs.selected().is_action() {
                    self.shortcuts.open(&self.settings.keymap);
                } else if !self.prefs.open_picker() && self.prefs.adjust(1) {
                    self.apply_settings();
                }
            }
            Key::Named(NamedKey::Escape) => self.prefs.close(),
            _ => return true,
        }
        self.window.request_redraw();
        true
    }

    /// Opens the Style Configurator on the theme in force.
    ///
    /// Seeded from the effective theme rather than the base one, so the dialog
    /// shows the colours the editor is actually drawing with, overrides and all.
    fn open_styles(&mut self) {
        self.styles.open(self.view.theme());
        self.window.request_redraw();
    }

    /// Handles a key while the Style Configurator is open.
    ///
    /// Modal, like the preferences dialog: every key is consumed, because a
    /// dialog that let typing through would be editing a document the user
    /// cannot see.
    fn handle_styles_key(&mut self, event: &winit::event::KeyEvent) -> bool {
        match &event.logical_key {
            Key::Named(NamedKey::ArrowDown) => self.styles.move_selection(1),
            Key::Named(NamedKey::ArrowUp) => self.styles.move_selection(-1),
            Key::Named(NamedKey::ArrowLeft) => {
                if self.styles.adjust(-1) {
                    self.commit_style();
                }
            }
            Key::Named(NamedKey::ArrowRight) => {
                if self.styles.adjust(1) {
                    self.commit_style();
                }
            }
            Key::Named(NamedKey::Escape) => self.styles.close(),
            _ => return true,
        }
        self.window.request_redraw();
        true
    }

    /// Writes the highlighted slot's colour into the settings and applies it.
    ///
    /// Only the slot that changed is recorded, so the file stays a list of
    /// deliberate choices rather than a dump of the whole theme.
    fn commit_style(&mut self) {
        let slot = self.styles.selected();
        let color = slot.get(&self.styles.theme());
        let mut settings = self.settings.clone();
        nn_app::styles::record(&mut settings.styles, slot, color);
        self.prefs.set_settings(settings);
        self.apply_settings();
    }

    /// Handles a key while the shortcut editor is open.
    ///
    /// While capturing, every chord is taken — including the ones the dialog
    /// would otherwise use for navigation. That is the point: `Ctrl+N` has to
    /// be bindable from inside a modal that would normally swallow it.
    fn handle_shortcuts_key(
        &mut self,
        event: &winit::event::KeyEvent,
        ctrl: bool,
        shift: bool,
        alt: bool,
    ) -> bool {
        if self.shortcuts.is_capturing() {
            if matches!(event.logical_key, Key::Named(NamedKey::Escape)) {
                self.shortcuts.cancel_capture();
            } else if let Some(chord) = chord_of(event, ctrl, shift, alt) {
                if self.shortcuts.capture(chord) {
                    self.prefs.set_keymap(self.shortcuts.keymap().clone());
                    self.apply_settings();
                }
            }
            self.window.request_redraw();
            return true;
        }

        match &event.logical_key {
            Key::Named(NamedKey::ArrowDown) => self.shortcuts.move_selection(1),
            Key::Named(NamedKey::ArrowUp) => self.shortcuts.move_selection(-1),
            Key::Named(NamedKey::Enter) | Key::Named(NamedKey::ArrowRight) => {
                self.shortcuts.begin_capture()
            }
            Key::Named(NamedKey::Delete) | Key::Named(NamedKey::Backspace) => {
                if self.shortcuts.unbind_selected() {
                    self.prefs.set_keymap(self.shortcuts.keymap().clone());
                    self.apply_settings();
                }
            }
            Key::Named(NamedKey::Escape) => self.shortcuts.close(),
            _ => return true,
        }
        self.window.request_redraw();
        true
    }

    fn toggle_function_list(&mut self) {
        if self.functions.is_open() {
            self.functions.close();
        } else {
            let symbols = scan_symbols(&self.tab().editor.doc.text());
            self.functions.set_symbols(symbols);
            self.symbols_key = (self.active, self.tab().editor.doc.revision());
        }
        let width = self.gpu.config.width as f32;
        let height = self.gpu.config.height as f32;
        self.view
            .resize(&mut self.tabs[self.active].editor, width, height);
        self.window.request_redraw();
    }

    /// Puts `text` on the system clipboard and remembers it.
    ///
    /// The history is fed here rather than at each copy site, so every path that
    /// reaches the clipboard — the menu, the context menu, a plugin — is
    /// recoverable. A repeat of the newest entry is dropped, because copying the
    /// same thing twice should not push the thing before it out of reach.
    fn copy_to_clipboard(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        if self.clipboard_history.first() != Some(&text) {
            self.clipboard_history.insert(0, text.clone());
            self.clipboard_history.truncate(CLIPBOARD_HISTORY);
        }
        if let Some(clipboard) = &mut self.clipboard {
            if let Err(err) = clipboard.set_text(text) {
                eprintln!("npp: clipboard write failed: {err}");
            }
        }
    }

    /// The text a copy should take: the selection, or the caret's line.
    fn copy_source(&self) -> String {
        let editor = &self.tab().editor;
        let selected = editor.selected_text();
        if !selected.is_empty() {
            return selected;
        }
        editor.line_with_break(editor.caret_line())
    }

    /// Recomputes matches and hands them to the view for highlighting.
    ///
    /// With "in selection" on, the matches are confined to what is selected
    /// right now. The selection is read here rather than stored in the bar
    /// because it changes as the user drags, and a range remembered from the
    /// click that turned the setting on would search a document that no longer
    /// matches the screen.
    fn refresh_find(&mut self) {
        let active = self.active;
        let within = if self.find.in_selection {
            let editor = &self.tabs[active].editor;
            let start = editor.selection_start();
            let end = editor.selection_end();
            let (start, end) = (start.min(end), start.max(end));
            if end > start {
                Some(start..end)
            } else {
                None
            }
        } else {
            None
        };
        self.find
            .refresh_within(&self.tabs[active].editor.doc, within);
        let matches = self.find.matches.clone();
        let current = self.find.current_match();
        self.view.set_matches(matches, current);
    }

    /// Opens the find bar in incremental mode.
    ///
    /// The caret and the scroll are recorded first, because Escape has to put
    /// them back: a search you abandoned should leave the document where it
    /// found it, not where the last match happened to be.
    fn open_incremental_search(&mut self) {
        let editor = &self.tab().editor;
        self.incremental_anchor = Some((
            editor.caret(),
            editor.viewport.first_line,
            editor.viewport.top_offset,
        ));
        self.find.open_incremental();
        self.status
            .set_message(Some("Incremental search".to_string()));
        self.refresh_find();
        self.window.request_redraw();
    }

    /// Closes the find bar, honouring the volatile-find setting.
    ///
    /// In incremental mode the caret and the scroll go back to where they were
    /// when the bar opened. In find mode they stay where the search left them,
    /// because there the user chose each match with Enter and closing the bar is
    /// not a cancellation.
    fn close_find(&mut self) {
        let anchor = self.incremental_anchor.take();
        self.find_dialog.end_drag();
        self.find.close(self.settings.volatile_find);
        self.save_find_history();
        if let Some((caret, first_line, top_offset)) = anchor {
            let editor = &mut self.tab_mut().editor;
            editor.set_caret(caret);
            editor.viewport.first_line = first_line;
            editor.viewport.top_offset = top_offset;
        }
        if self.settings.volatile_find {
            self.view.set_matches(Vec::new(), None);
        } else {
            let matches = self.find.matches.clone();
            let current = self.find.current_match();
            self.view.set_matches(matches, current);
        }
        self.results.close();
        self.window.request_redraw();
    }

    /// Selects the first match at or after the incremental anchor.
    ///
    /// The anchor is where the caret was when the bar opened, not where the last
    /// match was. Typing "f", "fo", "foo" has to refine the same search rather
    /// than walk forward through the document, which is what would happen if
    /// each keystroke searched from the previous match.
    fn follow_incremental(&mut self) {
        if !self.find.is_incremental() {
            return;
        }
        let Some((anchor, _, _)) = self.incremental_anchor else {
            return;
        };
        if let Some(range) = self.find.next_from(anchor) {
            self.goto_match(range);
        }
        self.refresh_find();
    }

    /// Selects a match and scrolls it into view.
    fn goto_match(&mut self, range: std::ops::Range<usize>) {
        self.tab_mut().editor.select_range(range);
        self.tab_mut().editor.ensure_caret_visible();
        self.window.request_redraw();
    }

    /// Moves to the next or previous match.
    fn step_match(&mut self, backwards: bool) {
        let caret = self.tab().editor.caret();
        let hit = if backwards {
            self.find.prev_from(caret)
        } else {
            self.find.next_from(caret)
        };
        if let Some(range) = hit {
            self.goto_match(range);
        }
        self.refresh_find();
    }

    /// Replaces the current match and advances.
    fn replace_current(&mut self) {
        let Some(range) = self.find.current_match() else {
            return;
        };
        let Some(search) = self.find.search() else {
            return;
        };
        let replacement =
            search.expand(&self.tab().editor.doc, range.clone(), self.find.replacement.text());
        self.tab_mut().editor.select_range(range);
        self.after_edit(|ed| ed.insert(&replacement));
        self.refresh_find();
        self.step_match(false);
    }

    /// Replaces every match as one undo group.
    fn replace_all(&mut self) {
        let Some(search) = self.find.search() else {
            return;
        };
        let matches = search.find_all(&self.tab().editor.doc);
        if matches.is_empty() {
            return;
        }
        let template = self.find.replacement.text().to_string();
        let edits: Vec<_> = matches
            .into_iter()
            .map(|m| {
                let text = search.expand(&self.tab().editor.doc, m.clone(), &template);
                (m, text)
            })
            .collect();
        self.after_edit(|ed| ed.replace_ranges(&edits));
        self.refresh_find();
    }

    /// Handles a key while the find bar is open. Returns true if consumed.
    fn handle_find_key(
        &mut self,
        event: &winit::event::KeyEvent,
        ctrl: bool,
        shift: bool,
        alt: bool,
    ) -> bool {
        let redraw = |state: &Self| state.window.request_redraw();
        // An open history list is a mode of its own: Escape closes it rather
        // than the bar, Enter takes the entry the arrows are on, and the arrows
        // walk the list instead of the caret.
        if let Some(field) = self.find.history_open() {
            match &event.logical_key {
                Key::Named(NamedKey::Escape) => {
                    self.find.close_history();
                    redraw(self);
                    return true;
                }
                Key::Named(NamedKey::Enter) => {
                    if let Some(pick) = self.find.history_pick() {
                        self.find.pick_history(field, pick);
                        self.refresh_find();
                    } else {
                        self.find.close_history();
                    }
                    redraw(self);
                    return true;
                }
                Key::Named(NamedKey::ArrowDown) => {
                    self.find.move_history(1);
                    redraw(self);
                    return true;
                }
                Key::Named(NamedKey::ArrowUp) => {
                    self.find.move_history(-1);
                    redraw(self);
                    return true;
                }
                _ => {}
            }
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.close_find();
                redraw(self);
                true
            }
            // Enter acts on what the bar is doing, the way Notepad++ does: in
            // the replacement field it replaces the match under the caret, in
            // the pattern field it walks on. Shift walks the other way, which
            // used to be the only direction there was.
            Key::Named(NamedKey::Enter) => {
                if ctrl {
                    self.replace_all();
                } else if self.find.focus == nn_app::findbar::Field::Replacement {
                    self.replace_current();
                } else if shift {
                    self.step_match(!self.find.up);
                } else {
                    self.step_match(self.find.up);
                }
                redraw(self);
                true
            }
            Key::Named(NamedKey::Tab) => {
                self.find.toggle_focus();
                redraw(self);
                true
            }
            Key::Named(NamedKey::Backspace) => {
                self.find.backspace();
                self.refresh_find();
                redraw(self);
                true
            }
            Key::Named(NamedKey::Delete) => {
                self.find.delete();
                self.refresh_find();
                redraw(self);
                true
            }
            Key::Named(NamedKey::ArrowLeft) => {
                self.find.left();
                redraw(self);
                true
            }
            Key::Named(NamedKey::ArrowRight) => {
                self.find.right();
                redraw(self);
                true
            }
            Key::Named(NamedKey::Home) => {
                self.find.home();
                redraw(self);
                true
            }
            Key::Named(NamedKey::End) => {
                self.find.end();
                redraw(self);
                true
            }
            // Notepad++'s combo boxes drop their history with Alt+Down, and so
            // does this one: the fields have no room for a second meaning for
            // the plain arrows.
            Key::Named(NamedKey::ArrowDown | NamedKey::ArrowUp) if alt => {
                self.find.toggle_history(self.find.focus);
                redraw(self);
                true
            }
            // Replace this one and move on, rather than all of them. Only the
            // three chords below are taken: every other control key falls
            // through to the keymap, so `Ctrl+F` still reopens the bar it is
            // already in.
            Key::Character(c) if ctrl && c.to_lowercase() == "r" => {
                self.replace_current();
                redraw(self);
                true
            }
            // Select the field, so the pattern can be typed over instead of
            // deleted character by character. Left to the keymap, this chord
            // selected the document behind the bar.
            Key::Character(c) if ctrl && c.to_lowercase() == "a" => {
                self.find.select_all();
                redraw(self);
                true
            }
            // A pattern is often copied from somewhere else -- a file, a
            // terminal, another editor -- so the paste belongs in the field
            // with the caret rather than in the text behind the bar. The key
            // is consumed even when the clipboard is empty or holds nothing
            // but white space, which is what stops that paste from landing in
            // the document.
            Key::Character(c) if ctrl && c.to_lowercase() == "v" => {
                let pasted = self
                    .clipboard
                    .as_mut()
                    .and_then(|c| c.get_text().ok())
                    .unwrap_or_default();
                if self.find.insert_text(&pasted) {
                    self.refresh_find();
                    self.follow_incremental();
                }
                redraw(self);
                true
            }
            _ => {
                // `Ctrl+H` and friends arrive with the letter as their text, so
                // a handler that trusted `event.text` here typed an `h` into
                // the pattern instead of opening the replace row. Anything
                // carrying a command modifier is the keymap's to answer.
                if !nn_app::findbar::types_text(ctrl, alt) {
                    return false;
                }
                if let Some(text) = &event.text {
                    let printable: String = text.chars().filter(|c| !c.is_control()).collect();
                    if printable.is_empty() {
                        return false;
                    }
                    for c in printable.chars() {
                        self.find.insert_char(c);
                    }
                    self.refresh_find();
                    self.follow_incremental();
                    redraw(self);
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Routes a keystroke to the file browser.
    fn handle_browser_key(&mut self, event: &winit::event::KeyEvent, ctrl: bool) {
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.browser.close(),
            Key::Named(NamedKey::Enter) => {
                let paths = self.browser.enter();
                if !paths.is_empty() {
                    self.browser.close();
                    self.open_files(&paths);
                }
            }
            Key::Named(NamedKey::ArrowDown) => self.browser.move_selection(1),
            Key::Named(NamedKey::ArrowUp) => self.browser.move_selection(-1),
            Key::Named(NamedKey::PageDown) => {
                let rows = self.browser.visible_rows() as isize;
                self.browser.move_selection(rows);
            }
            Key::Named(NamedKey::PageUp) => {
                let rows = self.browser.visible_rows() as isize;
                self.browser.move_selection(-rows);
            }
            // Backspace edits the filter, and goes up a directory once the
            // filter is empty — the idiom every file manager uses.
            Key::Named(NamedKey::Backspace) => {
                if self.browser.filter().is_empty() {
                    self.browser.up();
                } else {
                    self.browser.backspace();
                }
            }
            Key::Named(NamedKey::ArrowLeft) => self.browser.left(),
            Key::Named(NamedKey::ArrowRight) => self.browser.right(),
            Key::Named(NamedKey::Home) => self.browser.home(),
            Key::Named(NamedKey::End) => self.browser.end(),
            Key::Character(c) if ctrl => {
                // One lowercased copy for both keys, so the whole arm costs one
                // allocation whatever was pressed.
                let key = c.to_lowercase();
                if key == "a" {
                    // The keyboard's way into a multiple selection, and the only
                    // one that reaches the rows below the fold.
                    self.browser.mark_all();
                } else if key == "v" {
                    let pasted = self
                        .clipboard
                        .as_mut()
                        .and_then(|c| c.get_text().ok())
                        .unwrap_or_default();
                    for ch in pasted.chars().filter(|c| !c.is_control()) {
                        self.browser.insert_char(ch);
                    }
                }
            }
            _ => {
                if let Some(text) = &event.text {
                    for ch in text.chars().filter(|c| !c.is_control()) {
                        self.browser.insert_char(ch);
                    }
                }
            }
        }
        self.window.request_redraw();
    }

    /// Routes a keystroke to the path prompt.
    fn handle_prompt_key(&mut self, event: &winit::event::KeyEvent, ctrl: bool) {
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.prompt.close(),
            Key::Named(NamedKey::Enter) => self.submit_prompt(),
            Key::Named(NamedKey::Backspace) => self.prompt.backspace(),
            Key::Named(NamedKey::Delete) => self.prompt.delete(),
            Key::Named(NamedKey::ArrowLeft) => self.prompt.left(),
            Key::Named(NamedKey::ArrowRight) => self.prompt.right(),
            Key::Named(NamedKey::Home) => self.prompt.home(),
            Key::Named(NamedKey::End) => self.prompt.end(),
            Key::Character(c) if ctrl => {
                if c.to_lowercase() == "v" {
                    let pasted = self
                        .clipboard
                        .as_mut()
                        .and_then(|c| c.get_text().ok())
                        .unwrap_or_default();
                    for ch in pasted.chars().filter(|c| !c.is_control()) {
                        self.prompt.insert_char(ch);
                    }
                }
            }
            _ => {
                if let Some(text) = &event.text {
                    for ch in text.chars().filter(|c| !c.is_control()) {
                        self.prompt.insert_char(ch);
                    }
                }
            }
        }
        self.window.request_redraw();
    }

    /// Handles a key while the workspace filter has the keyboard.
    ///
    /// Returns whether the key was consumed. Escape clears the filter first and
    /// gives up the keyboard second, so one key press never does both: a user
    /// who typed a filter and wants it gone should not also lose the box.
    fn handle_folder_key(&mut self, event: &winit::event::KeyEvent) -> bool {
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                if self.folder.filter().is_empty() {
                    self.folder.blur_filter();
                } else {
                    self.folder.clear_filter();
                }
            }
            Key::Named(NamedKey::Enter) => self.folder.blur_filter(),
            Key::Named(NamedKey::Backspace) => self.folder.backspace_filter(),
            Key::Named(NamedKey::ArrowDown) => self.folder.move_selection(1),
            Key::Named(NamedKey::ArrowUp) => self.folder.move_selection(-1),
            Key::Character(_) | Key::Named(NamedKey::Space) => {
                let Some(text) = &event.text else {
                    return false;
                };
                let printable: String = text.chars().filter(|c| !c.is_control()).collect();
                if printable.is_empty() {
                    return false;
                }
                for c in printable.chars() {
                    self.folder.type_filter(c);
                }
            }
            _ => return false,
        }
        self.window.request_redraw();
        true
    }

    /// Drives the open menu from the keyboard.
    ///
    /// Returns true when the key belonged to the menu. A key the menu has no use
    /// for closes it and returns false, so typing with a menu down starts
    /// editing rather than being swallowed -- which is what Notepad++ does.
    ///
    /// Left and Right do two jobs each, because a menu bar has two levels of
    /// horizontal movement: inside a submenu they walk out and in, and at the
    /// top level they walk along the titles.
    fn handle_menu_key(&mut self, event: &winit::event::KeyEvent) -> bool {
        match &event.logical_key {
            Key::Named(NamedKey::ArrowDown) => self.menu.step(1),
            Key::Named(NamedKey::ArrowUp) => self.menu.step(-1),
            Key::Named(NamedKey::ArrowRight) => {
                if !self.menu.open_submenu() {
                    self.menu.cycle_menu(1);
                }
            }
            Key::Named(NamedKey::ArrowLeft) => {
                if !self.menu.back() {
                    self.menu.cycle_menu(-1);
                }
            }
            Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Space) => {
                if let Some(action) = self.menu.activate() {
                    self.run(action);
                }
            }
            Key::Named(NamedKey::Escape) => {
                if !self.menu.back() {
                    self.menu.close();
                }
            }
            _ => {
                self.menu.close();
                return false;
            }
        }
        self.window.request_redraw();
        true
    }

    fn handle_key(&mut self, event: &winit::event::KeyEvent) {
        // A key that arrives in the same instant as the focus is the key that
        // made the switch, handed back by the window manager once its grab is
        // gone. Taking it for typing edits the document with a keystroke the
        // user aimed at the window manager -- a `Tab` indents over the selection
        // they copied, which reads as their text disappearing.
        if self.switched_in() {
            return;
        }
        // The playground takes the keyboard before anything else. It is a whole
        // window rather than a layer over one, so a keystroke aimed at the room
        // must not reach the document behind it -- and Escape, which is the only
        // key that does anything, has to be caught here rather than by whatever
        // modal happens to be open underneath.
        if self.playground.is_active() {
            self.handle_playground_key(event);
            return;
        }
        // A message from the last command has been read by now.
        self.status.set_message(None);
        let ctrl = self.modifiers.control_key();
        let shift = self.modifiers.shift_key();
        let alt = self.modifiers.alt_key();

        // A context menu is modal to the keyboard: any key closes it and does
        // nothing else. That is what every desktop does, and it means Escape
        // needs no special case -- it is just the key people reach for.
        if self.context.is_open() {
            self.context.close();
            self.window.request_redraw();
            return;
        }

        // The print preview covers the window, so it takes the keyboard before
        // anything else: a keystroke must not reach the document behind the
        // sheet.
        if self.print.is_open() {
            self.handle_print_key(event);
            return;
        }

        if self.peeker.is_open() {
            self.handle_peeker_key(event);
            return;
        }

        // The About box has one button and no state, so every key dismisses it.
        // Escape and Enter are what people press; swallowing the rest is what
        // stops a keystroke aimed at the dialog from editing the document
        // behind it.
        if self.about.is_open() {
            self.close_about();
            self.window.request_redraw();
            return;
        }

        if self.browser.is_open() {
            self.handle_browser_key(event, ctrl);
            return;
        }

        if self.shortcuts.is_open() {
            self.handle_shortcuts_key(event, ctrl, shift, alt);
            return;
        }

        if self.plugin_list.is_open() {
            self.handle_plugin_list_key(event);
            return;
        }

        if self.prefs.is_open() {
            self.handle_prefs_key(event);
            return;
        }

        if self.styles.is_open() {
            self.handle_styles_key(event);
            return;
        }

        if self.prompt.is_open() {
            self.handle_prompt_key(event, ctrl);
            return;
        }

        if self.complete.is_open() && self.handle_completion_key(event, ctrl) {
            return;
        }

        if self.find.open && self.handle_find_key(event, ctrl, shift, alt) {
            return;
        }

        // The workspace filter takes the keyboard while it has focus, so a
        // letter typed into it filters the tree instead of reaching the text.
        if self.folder.is_filter_focused() && self.handle_folder_key(event) {
            return;
        }

        // The character grid is a palette, so it is asked after every modal and
        // before the document: it takes the arrows and Enter while it is up, and
        // lets everything else through to the text.
        if self.charset.is_open() && self.handle_charset_key(event) {
            return;
        }

        // Alt and a letter opens the menu that starts with it, the way the
        // underlined letters in a menu bar work. This comes before the menu's
        // own key handling so that pressing the same chord twice cycles between
        // two menus that share a letter instead of closing and reopening one.
        // AltGr arrives as Ctrl plus Alt, so the ctrl guard keeps it out.
        if alt && !ctrl && self.menu_height() > 0.0 {
            if let Key::Character(text) = &event.logical_key {
                if let Some(letter) = text.chars().next() {
                    if self.menu.open_by_letter(letter) {
                        self.window.request_redraw();
                        return;
                    }
                }
            }
        }

        // An open menu owns the arrows, Enter and Escape. It is asked before the
        // keymap so that a chord bound to a command cannot fire while a menu is
        // down over the text.
        if self.menu.is_open() && self.handle_menu_key(event) {
            return;
        }

        // Every command the menu can invoke is looked up here rather than
        // matched arm by arm, so a rebinding in the settings file reaches the
        // keyboard and the menu at once.
        if let Some(chord) = chord_of(event, ctrl, shift, alt) {
            if let Some(action) = self.settings.keymap.lookup(&chord) {
                self.run(action);
                return;
            }
        }

        let motion = match &event.logical_key {
            Key::Named(NamedKey::ArrowLeft) if ctrl => Some(Motion::WordLeft),
            Key::Named(NamedKey::ArrowRight) if ctrl => Some(Motion::WordRight),
            Key::Named(NamedKey::ArrowLeft) => Some(Motion::CharLeft),
            Key::Named(NamedKey::ArrowRight) => Some(Motion::CharRight),
            Key::Named(NamedKey::ArrowUp) => Some(Motion::LineUp),
            Key::Named(NamedKey::ArrowDown) => Some(Motion::LineDown),
            // Ctrl+Home and Ctrl+End reach the ends of the document, the way
            // Notepad++ does. `Motion::DocStart` and `DocEnd` existed in the
            // core and were exercised by its tests, but nothing on the keyboard
            // produced them, so the two chords did nothing.
            Key::Named(NamedKey::Home) if ctrl => Some(Motion::DocStart),
            Key::Named(NamedKey::End) if ctrl => Some(Motion::DocEnd),
            Key::Named(NamedKey::Home) => Some(Motion::LineStart),
            Key::Named(NamedKey::End) => Some(Motion::LineEnd),
            Key::Named(NamedKey::PageUp) => Some(Motion::PageUp),
            Key::Named(NamedKey::PageDown) => Some(Motion::PageDown),
            _ => None,
        };

        if let Some(motion) = motion {
            if (alt && shift) || (self.column_mode && shift) {
                self.extend_block(motion);
            } else {
                self.block_anchor = None;
                // Moving the caret is the "move" half of Begin/End Select, so the
                // marker has to survive it; only a fresh click clears it.
                self.edit(Step::Move(motion, shift), |ed| ed.move_caret(motion, shift));
            }
            return;
        }

        match &event.logical_key {
            // Escape drops the extra carets before it does anything else, which
            // is the way out of a caret stack that went wrong. With one caret
            // there is nothing to drop and the key falls through to nothing.
            Key::Named(NamedKey::Escape) => {
                // The hint is the most recent thing on screen, so it is the
                // first thing Escape takes away.
                if self.param_hint.is_open() {
                    self.param_hint.close();
                    self.window.request_redraw();
                } else if self.tab_mut().editor.clear_extra_carets() {
                    self.refresh_brace();
                    self.window.request_redraw();
                }
            }
            Key::Named(NamedKey::Insert) => {
                let on = self.tab_mut().editor.toggle_overwrite();
                self.status.set_message(Some(
                    if on { "Overwrite" } else { "Insert" }.to_string(),
                ));
            }
            Key::Named(NamedKey::Backspace) => self.edit(Step::Backspace, Editor::backspace),
            Key::Named(NamedKey::Delete) => self.edit(Step::Delete, Editor::delete_forward),
            Key::Named(NamedKey::Enter) => {
                let indent = self.tab().editor.auto_indent();
                let text = format!("\n{indent}");
                self.edit(Step::Insert(text.clone()), |ed| ed.insert(&text));
            }
            Key::Named(NamedKey::Tab) => {
                // Notepad++ indents a multi-line selection and inserts a tab
                // otherwise; Shift+Tab always unindents.
                let (first, last) = self.tab().editor.selected_lines();
                if shift || last > first {
                    let width = self.view.tab_width() as usize;
                    if shift {
                        self.line_op(|text| lines::unindent(text, width));
                    } else {
                        self.line_op(lines::indent);
                    }
                } else {
                    self.edit(Step::Insert("    ".into()), |ed| ed.insert("    "))
                }
            }
            // A control chord that got this far is not text. AltGr arrives as
            // Ctrl+Alt with real text, so only Ctrl alone is swallowed.
            //
            // The guard is on the wildcard rather than on `Key::Character`
            // because a Ctrl chord does not always arrive as a character. On a
            // layout where the digits sit on the shifted level, `Ctrl+Shift+0`
            // reaches winit as a control character, and winit reports the ones
            // that collide with a named key as that key -- `Ctrl+Shift+0` came
            // through as a space and typed one into the document. Every Ctrl
            // chord that means something has already been handled above or
            // found in the keymap, so anything left is noise.
            _ if ctrl && !alt => {}
            _ => {
                if let Some(text) = &event.text {
                    let printable: String = text.chars().filter(|c| !c.is_control()).collect();
                    if !printable.is_empty() {
                        self.type_text(&printable);
                    }
                }
            }
        }
    }

    /// Inserts typed text, closing brackets and quotes as it goes.
    ///
    /// Only a single character is considered for auto-close. A paste or an IME
    /// commit arrives as one `text` event holding many characters, and closing a
    /// bracket in the middle of it would be wrong.
    fn type_text(&mut self, text: &str) {
        let was_completing = self.complete.is_open();
        let mut chars = text.chars();
        let single = match (chars.next(), chars.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        };
        let Some(c) = single else {
            self.edit(Step::Insert(text.to_string()), |ed| ed.insert(text));
            return;
        };
        match self.tab().editor.typed(c) {
            Typed::Plain => {
                if self.tab().editor.overwrite() {
                    self.edit(Step::Overwrite(c), |ed| ed.overwrite_char(c));
                } else {
                    self.edit(Step::Insert(text.to_string()), |ed| ed.insert(text));
                }
            }
            Typed::Skip => {
                // The closer is already there: step over it instead of doubling
                // it. Recorded as a move so a replayed macro steps over too.
                self.edit(Step::Move(Motion::CharRight, false), |ed| {
                    ed.move_caret(Motion::CharRight, false)
                });
            }
            Typed::Pair(close) => {
                let pair = format!("{c}{close}");
                // Recorded as two steps so a replayed macro leaves the caret
                // between the pair, exactly where typing left it.
                self.recorder.record(Step::Insert(pair.clone()));
                self.recorder.record(Step::Move(Motion::CharLeft, false));
                if self.recorder.is_recording() {
                    self.status.set_recording(Some(self.recorder.len()));
                }
                self.after_edit(|ed| {
                    ed.insert(&pair);
                    ed.move_caret(Motion::CharLeft, false);
                });
            }
        }
        if was_completing && self.completion_mode.refilters_on(c) {
            self.refilter_completion();
        }
        // Typing the opening parenthesis of a call is the moment the signature
        // becomes useful, and it is the only moment the hint appears unasked.
        if c == '(' {
            self.follow_param_hint();
        }
    }

    /// Handles a key while the completion popup is open.
    ///
    /// Returns true when the key was consumed. Anything that is not navigation,
    /// acceptance, dismissal, or a word character closes the popup and falls
    /// through to the editor — so typing `(` after a half-typed word does what
    /// you meant rather than being swallowed.
    fn handle_completion_key(&mut self, event: &winit::event::KeyEvent, ctrl: bool) -> bool {
        match &event.logical_key {
            Key::Named(NamedKey::ArrowDown) => {
                self.complete.move_selection(1);
                self.window.request_redraw();
                true
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.complete.move_selection(-1);
                self.window.request_redraw();
                true
            }
            Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Tab) => {
                self.accept_completion();
                true
            }
            Key::Named(NamedKey::Escape) => {
                self.complete.close();
                self.window.request_redraw();
                true
            }
            Key::Named(NamedKey::Space) if ctrl => true,
            _ => {
                let word_char = event
                    .text
                    .as_deref()
                    .and_then(|t| t.chars().next())
                    .is_some_and(|c| self.completion_mode.refilters_on(c));
                if word_char {
                    // Leave the popup open: `type_text` sees it was open, inserts
                    // the character, and refilters against the longer prefix.
                    // Closing here would make the popup vanish on the first
                    // keystroke, which is the one thing it must survive.
                    false
                } else {
                    self.complete.close();
                    self.window.request_redraw();
                    false
                }
            }
        }
    }

    fn handle_wheel(&mut self, delta: MouseScrollDelta) {
        let dy = match delta {
            MouseScrollDelta::LineDelta(_, y) => -y * self.view.line_height() * 3.0,
            MouseScrollDelta::PixelDelta(pos) => -pos.y as f32,
        };
        // An open preferences list takes the wheel, the way a dropdown does: the
        // list is the thing under the pointer, and scrolling the document
        // behind it would move text the panel is covering.
        if self.prefs.picker_open() && dy != 0.0 {
            let rows = (-dy / self.view.line_height()).round() as isize;
            // The wheel moves the highlight, and the highlight is a preview, so
            // it changes the theme under the pointer. A wheel that only scrolled
            // would be a wheel the list does not answer to.
            if rows != 0 && self.prefs.picker_move(rows) {
                self.apply_settings_quiet();
            }
            self.window.request_redraw();
            return;
        }
        // Ctrl + wheel zooms, the way every other editor does it. The wheel event
        // carries no modifiers, so read the last reported state.
        if self.modifiers.control_key() && dy != 0.0 {
            self.zoom(if dy < 0.0 { 1 } else { -1 });
            self.window.request_redraw();
            return;
        }
        // The workspace panel is a column of its own, so the wheel over it
        // scrolls the tree and nothing else.
        if self.folder.is_open()
            && (self.cursor.0 as f32) < self.folder.width(self.view.char_width())
        {
            let rows = (-dy / self.view.line_height()).round() as isize;
            self.folder.scroll_by(rows);
            self.window.request_redraw();
            return;
        }
        if self.functions.is_open() && (self.cursor.0 as f32) >= self.functions_left() {
            let rows = (-dy / self.view.line_height()).round() as isize;
            self.functions.scroll_by(rows);
            self.window.request_redraw();
            return;
        }
        if self.history.is_open() && (self.cursor.1 as f32) >= self.history_top() {
            let rows = (-dy / self.view.line_height()).round() as isize;
            self.history.scroll_by(rows);
            self.window.request_redraw();
            return;
        }
        if self.results.is_open() && (self.cursor.1 as f32) >= self.results_top() {
            let rows = (-dy / self.view.line_height()).round() as isize;
            self.results.scroll_by(rows);
            self.window.request_redraw();
            return;
        }
        // The wheel scrolls the pane under the pointer rather than the focused
        // one: having to click a pane before scrolling it would be a step too
        // many, and Notepad++ scrolls without moving the focus.
        if self.other.is_some() {
            let (_, second) = self.pane_rects();
            let at = (self.cursor.0 as f32, self.cursor.1 as f32);
            if second.is_some_and(|rect| rect.contains(at.0, at.1)) {
                if let Some(other) = &mut self.other {
                    let active = other.active;
                    other.tabs[active].editor.scroll_pixels(dy);
                }
                self.window.request_redraw();
                return;
            }
        }
        self.tab_mut().editor.scroll_pixels(dy);
        self.window.request_redraw();
    }

    /// Routes a left-button press to the tab strip or the text area.
    fn click(&mut self, x: f64, y: f64) {
        let line_height = self.ui.line_height;
        let menu_height = self.menu_height();
        // Only the minimap arm below turns this back on.
        self.minimap_drag = false;
        // A press is not always a first press. Counting here, before any widget
        // gets its turn, keeps the count independent of what was hit: the second
        // press on the text has to know it is the second even if the first one
        // closed a menu.
        let now = std::time::Instant::now();
        let repeat = matches!(
            self.last_press,
            Some((px, py, at))
                if now.duration_since(at) <= CLICK_INTERVAL
                    && (x - px).abs() <= CLICK_SLOP
                    && (y - py).abs() <= CLICK_SLOP
        );
        self.click_count = if repeat { (self.click_count + 1).min(3) } else { 1 };
        self.last_press = Some((x, y, now));
        self.unit_drag = None;

        // The character grid is drawn above everything else in the overlay pass,
        // so it gets first refusal. A click on a cell inserts and leaves the
        // panel up; a click anywhere else closes it, which is the only way out
        // that does not need the keyboard.
        if self.charset.is_open() {
            match self.charset.cell_at(x as f32, y as f32) {
                Some(byte) => {
                    self.charset.set_hover(Some(byte));
                    self.insert_hovered_char();
                }
                None => {
                    self.charset.close();
                    self.window.request_redraw();
                }
            }
            self.dragging = false;
            return;
        }

        // The About box is a dialog with one button and one link. A click on the
        // link opens the address in the browser and leaves the box up, the way a
        // dialog's link does everywhere else: the browser comes to the front, and
        // the box is still there when the user comes back to it. Any other click
        // dismisses -- on the button because that is what the button says, and
        // anywhere else because that is what people try when they cannot find
        // the button.
        if self.about.is_open() {
            if self.about.link_hit(x as f32, y as f32, line_height) {
                self.open_page(nn_app::about::link_url());
            } else {
                self.close_about();
            }
            self.dragging = false;
            self.window.request_redraw();
            return;
        }

        // The peeker is a chooser, so a click on a row commits it and a click
        // anywhere else cancels. It is tested before the context menu because
        // the two can never be open at once, and before everything else because
        // it covers the window.
        if self.peeker.is_open() {
            if let Some(index) = self.peeker.item_at(x as f32, y as f32, line_height) {
                self.peeker.set_selected(index);
                self.commit_peeker();
            } else {
                self.peeker.close();
                self.window.request_redraw();
            }
            self.dragging = false;
            return;
        }

        // An open context menu is drawn over everything, so it gets first
        // refusal: a click on a row runs it, and a click anywhere else only
        // closes it.
        if self.context.is_open() {
            let action = self.context.action_at(x as f32, y as f32, line_height);
            self.context.close();
            if let Some(action) = action {
                self.run(action);
            }
            self.dragging = false;
            self.window.request_redraw();
            return;
        }

        // The print preview covers the window, so it takes the mouse first.
        if self.print.is_open() {
            self.handle_print_click(x, y);
            self.dragging = false;
            return;
        }

        // The file browser is modal and drawn in the overlay pass, so it gets
        // first refusal over everything.
        if self.browser.is_open() {
            // Ctrl and Shift are how a list names more than one row, so they are
            // read here rather than in the browser: the modifiers are the
            // window's state, and the widget is only told what happened.
            let ctrl = self.modifiers.control_key();
            let shift = self.modifiers.shift_key();
            if let Some(row) = self.browser.row_at(x as f32, y as f32) {
                if shift {
                    self.browser.mark_range(row, ctrl);
                } else if ctrl {
                    self.browser.toggle_mark(row);
                } else if row == self.browser.selected_index() {
                    let paths = self.browser.enter();
                    if !paths.is_empty() {
                        self.browser.close();
                        self.open_files(&paths);
                    }
                } else {
                    self.browser.set_selected(row);
                }
            }
            self.dragging = false;
            self.window.request_redraw();
            return;
        }

        // The shortcut editor sits on top of the preferences dialog, so it gets
        // first refusal over everything.
        if self.shortcuts.is_open() {
            if let Some(row) = self.shortcuts.row_at(x as f32, y as f32) {
                if row == self.shortcuts.selected_index() && !self.shortcuts.is_capturing() {
                    self.shortcuts.begin_capture();
                } else {
                    self.shortcuts.set_selected(row);
                }
            }
            self.dragging = false;
            self.window.request_redraw();
            return;
        }

        // The plugin manager is modal too, and older than the pointer: a click
        // on a row flips it, a click anywhere else closes the list. Closing is
        // safer than ignoring, because a modal with no exit but a key is a trap
        // for anyone who reached for the mouse.
        if self.plugin_list.is_open() {
            let (px, py) = (x as f32, y as f32);
            match self.plugin_list.row_at(px, py) {
                Some(index) => {
                    self.plugin_list.set_selected(index);
                    if let Some((index, enabled)) = self.plugin_list.toggle() {
                        self.set_plugin_enabled(index, enabled);
                    }
                }
                None => {
                    self.plugin_list.close();
                    self.window.request_redraw();
                }
            }
            self.dragging = false;
            return;
        }

        // The preferences dialog is modal and drawn in the overlay pass, so it
        // gets first refusal over everything.
        if self.prefs.is_open() {
            let (px, py) = (x as f32, y as f32);
            // An open list covers part of the dialog, so it is offered the click
            // before the rows are; a click outside it closes it, the way a menu
            // closes on a click elsewhere.
            if self.prefs.picker_open() {
                if let Some(index) = self.prefs.picker_at(px, py) {
                    // A click settles a list the way Enter does, so it writes
                    // the file: the preview is already on screen.
                    if self.prefs.picker_select(index) {
                        self.prefs.picker_choose();
                        self.apply_settings();
                    }
                } else if self.prefs.close_picker() {
                    // Clicking away from a list is the mouse's Escape, so it
                    // puts back the value the list opened on.
                    self.apply_settings_quiet();
                }
                self.dragging = false;
                self.window.request_redraw();
                return;
            }
            if let Some(row) = self.prefs.row_at(px, py) {
                self.prefs.set_selected(row);
                if self.prefs.selected().is_action() {
                    self.shortcuts.open(&self.settings.keymap);
                } else if !self.prefs.open_picker() && self.prefs.adjust(1) {
                    self.apply_settings();
                }
            }
            self.dragging = false;
            self.window.request_redraw();
            return;
        }

        if self.styles.is_open() {
            if let Some(row) = self.styles.row_at(x as f32, y as f32) {
                self.styles.set_selected(row);
            }
            self.dragging = false;
            self.window.request_redraw();
            return;
        }

        // The menu gets first refusal: an open dropdown covers the editor.
        if self.menu.is_open() || (y as f32) < menu_height {
            let width = self.gpu.config.width as f32;
            if let Some(action) = self.menu.click(x as f32, y as f32, line_height, width) {
                self.run(action);
            }
            self.dragging = false;
            self.window.request_redraw();
            return;
        }

        // The workspace panel is docked to the left edge, outside every pane,
        // so a click in its column belongs to it and never reaches the text.
        if self.folder.is_open()
            && (x as f32) < self.folder.width(self.view.char_width())
        {
            let top = menu_height + self.toolbar_height();
            if self.folder.filter_at(y as f32, top, line_height) {
                self.folder.focus_filter();
            } else if let Some(row) = self.folder.row_at(y as f32, top, line_height) {
                self.folder.select(row);
                if let Some(path) = self.folder.path_at(row) {
                    if self.folder.is_dir_at(row) {
                        self.folder.tree_mut().toggle(&path);
                    } else {
                        self.open_path(&path);
                    }
                }
            } else {
                self.folder.blur_filter();
            }
            self.dragging = false;
            self.window.request_redraw();
            return;
        }

        // The completion popup is drawn in the overlay pass, above the editor,
        // so it gets the next refusal after the menu.
        if self.complete.is_open() {
            if let Some(index) = self.complete.item_at(x as f32, y as f32, line_height) {
                self.complete.set_selected(index);
                self.accept_completion();
                self.dragging = false;
                self.window.request_redraw();
                return;
            }
        }

        // The floating Search window, when that is the presentation chosen: a
        // press on it belongs to it, and a press anywhere else falls through to
        // the document, because a search is not modal -- Find Next is only
        // useful if you can still edit the text behind the dialog.
        if self.find.open && self.find.floating() {
            if let Some(hit) =
                self.find_dialog
                    .hit_at(&self.find, x as f32, y as f32, line_height)
            {
                match hit {
                    nn_app::finddialog::DialogHit::Find(inner) => self.apply_find_hit(Some(inner)),
                    nn_app::finddialog::DialogHit::Tab(tab) => self.show_find_tab(tab),
                    nn_app::finddialog::DialogHit::Title => {
                        self.find_dialog.begin_drag(x as f32, y as f32);
                    }
                    nn_app::finddialog::DialogHit::Transparency(fraction) => {
                        self.find_dialog
                            .set_transparency(fraction * nn_app::finddialog::MAX_TRANSPARENCY);
                    }
                }
                self.dragging = false;
                self.window.request_redraw();
                return;
            }
        }

        if let Some(action) = self.toolbar.click(
            x as f32,
            y as f32,
            menu_height,
            self.gpu.config.width as f32,
            line_height,
        ) {
            self.run(action);
            self.dragging = false;
            self.window.request_redraw();
            return;
        }

        // The find bar is docked above the status bar, so a press in its band
        // belongs to it: a chip flips a setting, a button runs its command, and
        // a press in either field puts the caret there. Nothing used to look at
        // the bar at all, so the settings were drawn but unreachable.
        if self.find.open {
            let top = self.find_bar_top();
            let hit = self.find.hit_at(
                x as f32,
                y as f32,
                self.gpu.config.width as f32,
                top,
                line_height,
                self.view.padding(),
            );
            // The open history list hangs over the document above the bar, so a
            // press in its band belongs to it even though it is outside the
            // bar's own rectangle.
            let in_list = matches!(hit, Some(nn_app::findbar::Hit::HistoryEntry(_)));
            // The band ends at the bar's own bottom: everything below it belongs
            // to the results panel, which sits between the bar and the status
            // bar, and a press there must open its hit instead of doing nothing.
            let bottom =
                top + self
                    .find
                    .height(self.gpu.config.width as f32, line_height, self.view.padding());
            let inside = (y as f32) >= top && (y as f32) < bottom;
            if inside || in_list {
                self.apply_find_hit(hit);
                self.dragging = false;
                self.window.request_redraw();
                return;
            }
            if self.find.history_open().is_some() {
                // A press anywhere else is a decision to leave the list alone.
                self.find.close_history();
            }
        }

        // The divider is grabbed before the panes are: a drag that started on
        // the bar has to keep moving it even when the pointer crosses into a
        // pane, and a click on the bar must not also move the caret.
        if let Some(divider) = self.divider_rect() {
            if divider.contains(x as f32, y as f32) {
                self.divider_drag = true;
                self.dragging = false;
                return;
            }
        }

        // A click anywhere in the unfocused pane focuses it, so the rest of this
        // function can go on assuming that `self.tabs` and `self.view` are the
        // pane under the pointer.
        if self.other.is_some() {
            let (_, second) = self.pane_rects();
            if second.is_some_and(|rect| rect.contains(x as f32, y as f32)) {
                self.focus_other();
            }
        }

        // The arrows live in the strip's trailing edge, over no tab at all, so
        // they come before everything else in the strip.
        if let Some(direction) = self.tab_arrow_at(x, y) {
            if self.tab_bar.scroll_step(direction) {
                self.window.request_redraw();
            }
            self.dragging = false;
            return;
        }

        // The cross and the pin sit inside the active tab, so they are tested
        // before the tab itself: a click on the cross must close the document,
        // not merely reselect it.
        match self.tab_button_at(x, y) {
            Some(TabButton::Close) => {
                self.close_tab();
                self.dragging = false;
                self.window.request_redraw();
                return;
            }
            Some(TabButton::Pin) => {
                self.pin_tab();
                self.dragging = false;
                self.window.request_redraw();
                return;
            }
            None => {}
        }

        // A click on a tab switches to it and stops there: falling through would
        // also move the caret in the document the click just left.
        if let Some(index) = self.tab_at(x, y) {
            // Remembered in case this click turns into a drag: a tab released
            // over the other view is moved there, the way Notepad++ does it.
            self.tab_press = Some((index, x, y));
            self.switch_tab(index);
            self.dragging = false;
            return;
        }
        // The band below the toolbar belongs to the strip even when the click
        // missed a tab, so it must not reach the text behind it.
        if (y as f32) < self.strip_band().1 {
            self.dragging = false;
            return;
        }
        if self.history.is_open() {
            let top = self.history_top();
            if (y as f32) >= top {
                if let Some(index) = self.history.row_at(y as f32, top, line_height) {
                    let keep = self.history.rows()[index].keep;
                    self.revert_to_step(keep);
                }
                self.dragging = false;
                return;
            }
        }
        if self.results.is_open() {
            let top = self.results_top();
            if (y as f32) >= top {
                if let Some(index) = self.results.hit_at(y as f32, top, line_height) {
                    self.open_hit(index);
                }
                self.dragging = false;
                return;
            }
        }
        if self.functions.is_open() && x >= self.functions_left() as f64 {
            let top = self.view.content_top();
            if let Some(index) =
                self.functions
                    .symbol_at(y as f32, top, line_height)
            {
                self.goto_symbol(index);
            }
            self.dragging = false;
            return;
        }
        // The width, not `is_enabled`: distraction-free mode suppresses the
        // minimap without forgetting the preference, and a suppressed column
        // must not swallow clicks in the margin beside the text.
        if self.view.minimap().width() > 0.0 && x >= self.view.text_right() as f64 {
            self.minimap_drag = true;
            self.minimap_scroll_to(y);
            self.dragging = false;
            return;
        }
        if x >= self.view.bookmark_left() as f64 && x < self.view.fold_left() as f64 {
            let line = self.view.line_at(&mut self.tabs[self.active].editor, y as f32);
            self.tabs[self.active].editor.doc.toggle_bookmark(line);
            self.window.request_redraw();
            return;
        }
        if x >= self.view.fold_left() as f64 && x < self.view.text_left() as f64 {
            let line = self.view.line_at(&mut self.tabs[self.active].editor, y as f32);
            self.tabs[self.active].editor.doc.toggle_fold(line);
            self.window.request_redraw();
            return;
        }
        // The pane's chrome comes off before the editor sees the point: a window
        // y counts the menu bar, the toolbar and the tab strip as if they were
        // text, and the caret would land that far down the document.
        let (text_x, text_y) = self.view.text_point(x as f32, y as f32);
        let shift = self.modifiers.shift_key();
        let alt = self.modifiers.alt_key();
        // The second press takes the unit under the pointer and the third takes
        // the whole line, which is what a double and a triple click mean
        // everywhere else. Both return here: the caret the selection left at the
        // end of the unit is already where the next keystroke goes, and running
        // the single-click path after it would collapse what was just selected.
        if self.click_count >= 2 {
            let unit = if self.click_count >= 3 {
                ClickUnit::Line
            } else {
                ClickUnit::Word
            };
            let pos = self.tab_mut().editor.char_at_point(text_x, text_y);
            let range = match unit {
                ClickUnit::Word => self.tab().editor.word_range_at(pos),
                ClickUnit::Line => Some(self.tab().editor.line_range_at(pos)),
            };
            if let Some(range) = range {
                let (start, end) = (range.start, range.end);
                self.tab_mut().editor.select_range(start..end);
                // Held for the drag that usually follows: a drag out of a word
                // takes whole words, and out of a line whole lines.
                self.unit_drag = Some((unit, (start, end)));
            }
            self.block_anchor = None;
            self.select_anchor = None;
            self.refresh_brace();
            self.window.request_redraw();
            return;
        }
        // Ctrl+click drops another caret instead of moving the one there is.
        // Notepad++ reaches multi-caret this way and so does everything else,
        // and it is the only way to place carets on lines that are not
        // adjacent.
        if self.modifiers.control_key() {
            let pos = self.tab_mut().editor.char_at_point(text_x, text_y);
            self.tab_mut().editor.add_caret_at(pos);
            self.block_anchor = None;
            self.refresh_brace();
            self.window.request_redraw();
            return;
        }
        self.tab_mut()
            .editor
            .set_caret_from_point(text_x, text_y, shift);
        // Alt+drag is the one-off rectangle; column mode makes every drag one.
        // Both end up in the same anchor, so the drag code needs no second path.
        self.block_anchor = if alt || self.column_mode {
            Some(self.tab().editor.caret_point())
        } else {
            None
        };
        // A click is a new intent, so a half-finished Begin/End Select is dropped
        // rather than silently completed from wherever the click landed.
        self.select_anchor = None;
        self.refresh_brace();
        self.window.request_redraw();
    }

    /// The commands offered by a right-click in the text area.
    fn editor_menu_entries(&self) -> Vec<Entry> {
        let editor = &self.tab().editor;
        let entries = context::editor_entries(
            editor.can_undo(),
            editor.can_redo(),
            !editor.selections.primary().is_empty(),
            editor.doc.len_chars() == 0,
        );
        match &self.context_layout {
            Some(layout) => context::apply_layout(entries, layout),
            None => entries,
        }
    }

    /// The commands offered by a right-click on a tab.
    fn tab_menu_entries(&self) -> Vec<Entry> {
        let tab = self.tab();
        context::tab_entries(
            self.tabs.len(),
            self.active,
            tab.editor.doc.is_read_only(),
            tab.pinned,
            tab.colour,
        )
    }

    /// Opens the document peeker on the current tab.
    ///
    /// Refuses with one document: a list of one is not a choice, and the box
    /// would cover the text for nothing.
    fn open_peeker(&mut self) {
        if self.tabs.len() < 2 {
            return;
        }
        // A pinned tab is marked here as well as in the strip: the chooser is
        // where a user looks for a document they cannot see, and a pin they
        // cannot see is a pin they will try to close.
        let titles: Vec<String> = self
            .tabs
            .iter()
            .map(|tab| {
                if tab.pinned {
                    format!("* {}", tab.title())
                } else {
                    tab.title()
                }
            })
            .collect();
        self.peeker.show(titles, self.active);
        self.window.request_redraw();
    }

    /// Switches to the document the peeker has highlighted.
    fn commit_peeker(&mut self) {
        if let Some(index) = self.peeker.selected() {
            self.peeker.close();
            self.switch_tab(index);
        }
        self.window.request_redraw();
    }

    /// Handles a key while the document peeker is open.
    ///
    /// `Ctrl+Tab` moves down as well as opening, so holding it walks the list the
    /// way Notepad++ does. Enter commits and anything else cancels: the peeker is
    /// modal to the keyboard, and a chooser that let a keystroke through to the
    /// document behind it would be a trap.
    fn handle_peeker_key(&mut self, event: &winit::event::KeyEvent) {
        let ctrl = self.modifiers.control_key();
        match &event.logical_key {
            Key::Named(NamedKey::ArrowDown) => self.peeker.move_selection(1),
            Key::Named(NamedKey::ArrowUp) => self.peeker.move_selection(-1),
            Key::Named(NamedKey::Tab) if ctrl => self.peeker.move_selection(1),
            Key::Named(NamedKey::Enter) => {
                self.commit_peeker();
                return;
            }
            _ => self.peeker.close(),
        }
        self.window.request_redraw();
    }

    /// Opens the context menu for whatever is under the pointer.
    ///
    /// A right-click while a menu is already open closes it and opens nothing,
    /// which is what every desktop does and gives the user a way out that does
    /// not need the Escape key.
    fn right_click(&mut self, x: f64, y: f64) {
        if self.context.is_open() {
            self.context.close();
            self.window.request_redraw();
            return;
        }
        // A modal owns the window while it is up, so there is nothing to offer
        // a right-click behind it.
        if self.modal_open() {
            return;
        }
        // The bar between two views carries the one command that is about the
        // layout itself, which no other menu can offer: Notepad++ puts the
        // rotation on the splitter's own right-click, and so does this.
        if let Some(divider) = self.divider_rect() {
            if divider.contains(x as f32, y as f32) {
                let entries = context::divider_entries(self.split);
                if !entries.is_empty() {
                    self.context.show(x as f32, y as f32, entries);
                    self.window.request_redraw();
                }
                return;
            }
        }
        // The tab strip is the other part of the chrome with a menu. The pane
        // under the pointer is focused first, exactly as a left click does, so
        // that the strip whose band is checked is the strip the pointer is on.
        if self.other.is_some() {
            let (_, second) = self.pane_rects();
            if second.is_some_and(|rect| rect.contains(x as f32, y as f32)) {
                self.focus_other();
            }
        }
        if let Some(index) = self.tab_at(x, y) {
            // The menu acts on the tab it was opened on, so that tab is made
            // active first. Notepad++ leaves the active tab alone and acts on the
            // one under the pointer; switching is the simpler rule, and the only
            // visible difference is which tab is highlighted.
            self.switch_tab(index);
            let entries = self.tab_menu_entries();
            self.context.show(x as f32, y as f32, entries);
            self.window.request_redraw();
            return;
        }
        if (y as f32) < self.strip_band().1 {
            return;
        }
        // Only the text area has an editing menu. The gutter, the minimap and
        // the panels keep their left-click behaviour and offer nothing here.
        if (x as f32) < self.view.text_left() || (x as f32) >= self.view.text_right() {
            return;
        }
        if self.results.is_open() && (y as f32) >= self.results_top() {
            return;
        }
        if self.functions.is_open() && x >= self.functions_left() as f64 {
            return;
        }
        let entries = self.editor_menu_entries();
        self.context.show(x as f32, y as f32, entries);
        self.window.request_redraw();
    }

    /// Answers one call from the API (`npp --call`, or the MCP server behind it).
    ///
    /// The call has been parsed already: what is left is the part that needs the
    /// window -- which tab a path names, what the caret is in it, running a
    /// command the menu could have run. Every failure is an answer rather than a
    /// panic, because the caller is a script that cannot see the screen.
    fn call(&mut self, call: nn_app::api::Call) -> serde_json::Value {
        use nn_app::api;
        match call {
            api::Call::Help => serde_json::json!({"ok": true, "help": api::help()}),
            api::Call::Status => api::status(
                env!("CARGO_PKG_VERSION"),
                self.tabs.len(),
                self.active,
            ),
            api::Call::DocumentList => {
                let documents: Vec<api::DocumentInfo> = self
                    .tabs
                    .iter()
                    .enumerate()
                    .map(|(index, tab)| api::DocumentInfo {
                        path: tab
                            .editor
                            .doc
                            .path()
                            .map(|path| path.to_string_lossy().to_string()),
                        modified: tab.editor.doc.is_dirty(),
                        language: tab.language_name(&self.langs),
                        active: index == self.active,
                    })
                    .collect();
                api::documents(&documents)
            }
            api::Call::DocumentGet { which, range } => {
                let Some(index) = self.api_tab(&which) else {
                    return not_open(&which);
                };
                let tab = &self.tabs[index];
                let text = tab.editor.doc.text();
                let chars = tab.editor.doc.len_chars();
                let selection = selection_of(&tab.editor);
                let text = match range {
                    Some((start, end)) => {
                        if start >= chars {
                            return api::Error {
                                code: api::BAD_REQUEST,
                                message: format!(
                                    "`range` starts at {start} and the document is {chars} characters"
                                ),
                            }
                            .value();
                        }
                        text.chars()
                            .skip(start)
                            .take(end.min(chars) - start)
                            .collect()
                    }
                    None => text,
                };
                api::document(&api::DocumentText {
                    path: tab
                        .editor
                        .doc
                        .path()
                        .map(|path| path.to_string_lossy().to_string()),
                    text,
                    caret: tab.editor.caret(),
                    selection,
                    eol: tab.editor.doc.eol().name(),
                    modified: tab.editor.doc.is_dirty(),
                    chars,
                })
            }
            api::Call::DocumentSet {
                which,
                text,
                range,
            } => {
                let Some(index) = self.api_tab(&which) else {
                    return not_open(&which);
                };
                self.active = index;
                match range {
                    None => self.after_edit(move |ed| ed.doc.set_text(&text)),
                    Some((start, end)) => {
                        let whole = self.tab().editor.doc.text();
                        let chars = whole.chars().count();
                        if start >= chars || end > chars {
                            return api::Error {
                                code: api::BAD_REQUEST,
                                message: format!(
                                    "`range` {start}..{end} is outside a document of {chars} characters"
                                ),
                            }
                            .value();
                        }
                        let head: String = whole.chars().take(start).collect();
                        let tail: String = whole.chars().skip(end).collect();
                        let replaced = format!("{head}{text}{tail}");
                        self.after_edit(move |ed| ed.doc.set_text(&replaced));
                    }
                }
                api::done(&[
                    ("path", self.api_path(index)),
                    ("chars", serde_json::json!(self.tabs[index].editor.doc.len_chars())),
                ])
            }
            api::Call::DocumentSave { which } => {
                let Some(index) = self.api_tab(&which) else {
                    return not_open(&which);
                };
                if self.tabs[index].editor.doc.path().is_none() {
                    return api::Error {
                        code: api::FAILED,
                        message: "that document has never been saved, so it has no file to write"
                            .to_string(),
                    }
                    .value();
                }
                self.active = index;
                self.save_active();
                api::done(&[("path", self.api_path(index))])
            }
            api::Call::Command {
                name,
                text,
                range,
                whole,
            } => {
                let Some(action) = Action::from_name(&name) else {
                    return api::Error {
                        code: api::UNKNOWN_METHOD,
                        message: format!(
                            "no command called {name:?}; the MCP tool `list_commands` names the ones that work on text"
                        ),
                    }
                    .value();
                };
                // A command works on the selection, so picking the block first is
                // all it takes to run one over a whole document.
                if whole {
                    self.after_edit(|ed| ed.select_all());
                } else if let Some((start, end)) = range {
                    let chars = self.tab().editor.doc.len_chars();
                    if start > end || end > chars {
                        return api::Error {
                            code: api::BAD_REQUEST,
                            message: format!(
                                "`range` {start}..{end} is outside a document of {chars} characters"
                            ),
                        }
                        .value();
                    }
                    self.after_edit(|ed| ed.select_range(start..end));
                }
                self.run(action);
                let mut fields = vec![("command", serde_json::json!(name))];
                if text {
                    fields.push((
                        "text",
                        serde_json::json!(self.tab().editor.doc.text()),
                    ));
                }
                api::done(&fields)
            }
        }
    }

    /// The tab a call names, or `None` when no tab has that path open.
    ///
    /// The comparison is on the resolved path, so a call that says `notes.txt`
    /// from another working directory still finds the tab that has
    /// `/home/stef/notes.txt` open.
    fn api_tab(&self, which: &nn_app::api::Which) -> Option<usize> {
        match which {
            nn_app::api::Which::Active => Some(self.active),
            nn_app::api::Which::Path(path) => {
                let wanted = nn_app::single::absolute(std::path::Path::new(path));
                self.tabs.iter().position(|tab| {
                    tab.editor
                        .doc
                        .path()
                        .map(|open| nn_app::single::absolute(open) == wanted)
                        .unwrap_or(false)
                })
            }
        }
    }

    /// A tab's path as JSON, `null` for a document that has never been saved.
    fn api_path(&self, index: usize) -> serde_json::Value {
        match self.tabs[index].editor.doc.path() {
            Some(path) => serde_json::json!(path.to_string_lossy()),
            None => serde_json::Value::Null,
        }
    }

    /// Runs a menu command.
    fn run(&mut self, action: Action) {
        match action {
            // The line commands that are a function of text are run through
            // `nn_core::ops`, which is the same table an MCP client reaches: a
            // command cannot mean one thing to the menu and another to the API
            // if there is one place that turns its name into a call.
            //
            // The list is spelled out rather than reached with a guard because a
            // guard does not count towards exhaustiveness: with these rows named
            // here, adding a command to the enum and forgetting to route it is a
            // compile error rather than a menu row that does nothing.
            Action::SortAsc
            | Action::SortDesc
            | Action::SortNumeric
            | Action::SortLines(_)
            | Action::DedupeConsecutive
            | Action::DedupeAll
            | Action::ReverseLineOrder
            | Action::RandomizeLineOrder
            | Action::JoinLines
            | Action::RemoveEmpty
            | Action::RemoveEmptyWithBlank
            | Action::TrimTrailing
            | Action::TrimLeading
            | Action::TrimBoth
            | Action::TrimAll
            | Action::EolToSpace
            | Action::UpperCase
            | Action::LowerCase
            | Action::TitleCase
            | Action::ProperCaseForce
            | Action::ProperCaseBlend
            | Action::SentenceCaseForce
            | Action::SentenceCaseBlend
            | Action::InvertCase
            | Action::RandomCase
            | Action::ToggleComment
            | Action::SingleLineComment
            | Action::SingleLineUncomment
            | Action::BlockComment
            | Action::BlockUncomment
            | Action::TabsToSpaces
            | Action::SpacesToTabs
            | Action::SpacesToTabsAll
            | Action::SpacesToTabsLeading
            | Action::Indent
            | Action::Unindent => self.run_text_command(action),
            // A submenu row is a heading, not a command. It reaches here only if
            // something dispatched it by mistake, and doing nothing is the right
            // answer to that.
            Action::Submenu => {}
            Action::New => self.new_tab(),
            Action::Open => self.choose_file(nn_app::portal::Kind::Open),
            Action::Save => self.save_active(),
            Action::SaveAs => self.choose_file(nn_app::portal::Kind::Save),
            Action::CloseTab => self.close_tab(),
            Action::Exit => {
                self.save_session();
                std::process::exit(0);
            }
            Action::OpenRecent(index) => {
                if let Some(path) = self.session.recent.get(index).cloned() {
                    self.open_path(&path);
                }
            }
            Action::ClearRecent => {
                self.session.recent.clear();
                self.sync_recent();
            }
            Action::ToggleRecording => self.toggle_recording(),
            Action::PlayMacro => self.play_macro(),
            Action::FindInFiles => {
                self.prompt.open(PromptKind::FindInFiles, "");
                self.window.request_redraw();
            }
            Action::FindInProjects => {
                self.prompt.open(PromptKind::FindInProjects, "");
                self.window.request_redraw();
            }
            Action::SplitLines => {
                // The delimiter is the question this row asks, so it cannot be
                // routed to the table until the user has answered: the prompt's
                // answer handler calls the same `split_lines` the API does.
                self.prompt.open(PromptKind::SplitLines, ",");
                self.window.request_redraw();
            }
            Action::FolderAsWorkspace => self.toggle_folder_workspace(),
            Action::ProjectPanel(index) => self.toggle_project_panel(index),
            Action::AddProjectRoot => self.add_project_root(),
            Action::RefreshFolder => self.refresh_workspace(),
            Action::DuplicateLine => self.after_edit(|ed| {
                ed.duplicate_lines();
            }),
            Action::MoveLineUp => self.after_edit(|ed| {
                ed.move_lines(true);
            }),
            Action::MoveLineDown => self.after_edit(|ed| {
                ed.move_lines(false);
            }),
            Action::NextTab => self.cycle_tab(true),
            Action::PrevTab => self.cycle_tab(false),
            Action::DocumentPeeker => self.open_peeker(),
            Action::Undo => self.after_edit(|ed| {
                ed.undo();
            }),
            Action::Redo => self.after_edit(|ed| {
                ed.redo();
            }),
            Action::Cut => {
                let text = self.copy_source();
                self.copy_to_clipboard(text);
                self.after_edit(|ed| {
                    ed.delete_all_selections();
                });
            }
            Action::Copy => {
                let text = self.copy_source();
                self.copy_to_clipboard(text);
            }
            Action::Paste => {
                let pasted = self
                    .clipboard
                    .as_mut()
                    .and_then(|c| c.get_text().ok())
                    .unwrap_or_default();
                if !pasted.is_empty() {
                    self.after_edit(|ed| ed.insert(&pasted));
                }
            }
            Action::Delete => self.edit(Step::DeleteSelection, |ed| {
                ed.delete_selection();
            }),
            Action::SelectAll => self.after_edit(|ed| ed.select_all()),
            Action::Find => {
                self.incremental_anchor = None;
                self.find.open_find();
                self.find_dialog.set_tab(nn_app::finddialog::Tab::Find);
                self.refresh_find();
            }
            Action::Replace => {
                self.incremental_anchor = None;
                self.find.open_replace();
                self.find_dialog.set_tab(nn_app::finddialog::Tab::Replace);
                self.refresh_find();
            }
            Action::IncrementalSearch => self.open_incremental_search(),
            Action::FindNext => self.step_match(false),
            Action::FindPrev => self.step_match(true),
            Action::FoldToggle => self.fold(FoldCommand::Toggle),
            Action::FoldAll => self.fold(FoldCommand::All),
            Action::UnfoldAll => self.fold(FoldCommand::None),
            Action::ZoomIn => self.zoom(1),
            Action::ZoomOut => self.zoom(-1),
            Action::ZoomReset => self.zoom(0),
            Action::ToggleMinimap => self.toggle_minimap(),
            Action::ToggleToolbar => {
                let mut settings = self.settings.clone();
                settings.toolbar = !settings.toolbar;
                self.prefs.set_settings(settings);
                self.apply_settings();
            }
            Action::ToggleFunctionList => self.toggle_function_list(),
            Action::CompleteWord => self.open_completion(),
            Action::CompleteFunction => self.open_completion_in(CompletionMode::Function),
            Action::CompleteDocument => self.open_completion_in(CompletionMode::Document),
            Action::CompletePath => self.open_completion_in(CompletionMode::Path),
            Action::ParamHint => {
                if self.param_hint.is_open() {
                    self.param_hint.close();
                } else {
                    self.follow_param_hint();
                }
                self.window.request_redraw();
            }
            Action::Run => self.open_run_prompt(),
            Action::RunSaved(index) => {
                if let Some(command) = self.settings.runs.get(index).cloned() {
                    self.run_saved(&command);
                }
            }
            Action::Preferences => self.open_prefs(),
            Action::StyleConfigurator => self.open_styles(),
            Action::ToggleWrap => self.toggle_wrap(),
            Action::ToggleTheme => self.toggle_theme(),
            Action::Theme(index) => self.choose_theme(ThemeChoice::ALL[index]),
            Action::GotoLine => self.goto_line_prompt(),
            Action::ToggleBookmark => self.toggle_bookmark(),
            Action::NextBookmark => self.step_bookmark(true),
            Action::PrevBookmark => self.step_bookmark(false),
            Action::ClearBookmarks => {
                self.tab_mut().editor.doc.clear_bookmarks();
            }
            Action::ImportNotepad => self.import_prompt(),
            Action::Language(index) => self.set_language(Language::Udl(index)),
            Action::LanguagePlain => self.set_language(Language::Plain),
            Action::LanguageBuiltIn(index) => self.set_language(Language::Spec(index)),
            // The placeholder row in an empty Plugins menu.
            Action::NoOp => {}
            Action::ShortcutMapper => {
                self.shortcuts.open(&self.settings.keymap);
                self.window.request_redraw();
            }
            Action::ImportPlugin => {
                self.prompt.open(PromptKind::ImportPlugin, "");
                self.window.request_redraw();
            }
            Action::ImportTheme => {
                self.prompt.open(PromptKind::ImportTheme, "");
                self.window.request_redraw();
            }
            Action::EditContextMenu => self.edit_context_menu(),
            Action::DefineLanguage => self.define_language(),
            Action::OpenUdlFolder => self.open_udl_folder(),
            Action::UdlCollection => {
                nn_app::desktop::open_url("https://github.com/notepad-plus-plus/userDefinedLanguages")
                    .ok();
            }
            Action::ExportShortcuts => self.export_prompt(),
            Action::SaveAll => self.save_all(),
            Action::Print => self.open_print_preview(),
            Action::PrintNow => self.print_now(),
            Action::CloseAll => self.close_all(),
            Action::PinTab => self.pin_tab(),
            Action::TabColour => self.cycle_tab_colour(),
            Action::CloseOthers => {
                let active = self.active;
                self.close_tabs_where(|index| index == active);
            }
            Action::CloseToLeft => {
                let active = self.active;
                self.close_tabs_where(|index| index >= active);
            }
            Action::CloseToRight => {
                let active = self.active;
                self.close_tabs_where(|index| index <= active);
            }
            Action::Reload => self.reload(),
            Action::CopyPath => self.copy_path(),
            Action::OpenContainingFolder => self.open_containing_folder(),
            Action::OpenInDefaultViewer => self.open_in_default_viewer(),
            Action::OpenFolderAsWorkspace => {
                self.prompt.open(PromptKind::OpenFolderAsWorkspace, "");
                self.window.request_redraw();
            }
            Action::SaveCopyAs => {
                let initial = self
                    .tab()
                    .editor
                    .doc
                    .path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                self.prompt.open(PromptKind::SaveCopyAs, &initial);
                self.window.request_redraw();
            }
            Action::Rename => {
                let initial = self
                    .tab()
                    .editor
                    .doc
                    .path()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.prompt.open(PromptKind::Rename, &initial);
                self.window.request_redraw();
            }
            // The predicate is read off a snapshot rather than off `self`, because
            // `close_tabs_where` needs `self` mutably while it asks.
            Action::CloseAllButPinned => {
                let keep: Vec<bool> = self.tabs.iter().map(|tab| tab.pinned).collect();
                self.close_tabs_where(|index| keep[index]);
            }
            Action::CloseAllUnchanged => {
                let keep: Vec<bool> = self
                    .tabs
                    .iter()
                    .map(|tab| tab.editor.doc.is_dirty())
                    .collect();
                self.close_tabs_where(|index| keep[index]);
            }
            Action::MoveToTrash => self.move_to_trash(),
            Action::LoadSession => {
                let initial = nn_app::paths::config_file("session.txt")
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                self.prompt.open(PromptKind::LoadSession, &initial);
                self.window.request_redraw();
            }
            Action::SaveSession => {
                let initial = nn_app::paths::config_file("session.txt")
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                self.prompt.open(PromptKind::SaveSession, &initial);
                self.window.request_redraw();
            }
            Action::OpenContainingFolderCmd => self.open_terminal_here(None),
            Action::OpenContainingFolderPowerShell => self.open_terminal_here(Some("pwsh")),
            Action::DeleteLine => self.after_edit(|ed| {
                ed.delete_lines();
            }),
            Action::SelectLine => self.after_edit(|ed| ed.select_line()),
            Action::InsertDateTime => {
                let stamp = nn_core::local_stamp(nn_core::now_secs());
                self.after_edit(|ed| ed.insert(&stamp));
            }
            Action::ColumnEditor => {
                self.prompt.open(PromptKind::ColumnText, "");
                self.window.request_redraw();
            }
            Action::CharPanel => self.toggle_char_panel(),
            Action::AddCaretAbove => self.grow_caret(true),
            Action::AddCaretBelow => self.grow_caret(false),
            Action::EolWindows => self.set_eol(EolMode::CrLf),
            Action::EolUnix => self.set_eol(EolMode::Lf),
            Action::EolMac => self.set_eol(EolMode::Cr),
            Action::EncodingSet(encoding) => self.reinterpret(encoding),
            Action::EncodingConvert(encoding) => self.convert_encoding(encoding),
            Action::FullScreen => self.toggle_full_screen(),
            Action::DistractionFree => self.toggle_distraction_free(),
            Action::Playground => self.toggle_playground(),
            Action::ChangeHistory => self.toggle_change_history(),
            Action::TabLayout => self.cycle_tab_layout(),
            Action::BeginEndSelect => self.begin_end_select(),
            Action::BeginEndSelectColumn => self.begin_end_select_column(),
            Action::InsertDateTimeShort => self.insert_datetime("%H:%M:%S"),
            Action::InsertDateTimeLong => self.insert_datetime("%H:%M:%S  %Y-%m-%d"),
            Action::InsertDateTimeCustom => {
                let pattern = self.settings.datetime_format.clone();
                self.prompt.open(PromptKind::DateTimeFormat, &pattern);
            }
            Action::CopyFilename => self.copy_path_part(false),
            Action::CopyDirPath => self.copy_path_part(true),
            Action::CopyAllFilenames => self.copy_all_paths(false),
            Action::CopyAllFilePaths => self.copy_all_paths(true),
            Action::InsertBlankAbove => self.insert_blank_line(true),
            Action::InsertBlankBelow => self.insert_blank_line(false),
            Action::ParamHintPrev => self.param_hint.move_selection(-1),
            Action::ParamHintNext => self.param_hint.move_selection(1),
            Action::OpenSelectedFile => self.open_selected(false),
            Action::OpenSelectedFolder => self.open_selected(true),
            Action::RedactSelection => self.redact_selection(),
            Action::SearchOnInternet => self.search_on_internet(),
            Action::ChangeSearchEngine => {
                let engine = self.settings.search_engine.clone();
                self.prompt.open(PromptKind::SearchEngine, &engine);
            }
            Action::MultiSelectAll(mode) => self.multi_select(mode, MultiCommand::All),
            Action::MultiSelectNext(mode) => self.multi_select(mode, MultiCommand::Next),
            Action::MultiSelectSkip => self.multi_select(
                MatchMode {
                    match_case: true,
                    whole_word: false,
                },
                MultiCommand::Skip,
            ),
            Action::MultiSelectUndo => self.multi_select(
                MatchMode {
                    match_case: true,
                    whole_word: false,
                },
                MultiCommand::Undo,
            ),
            Action::ColumnMode => {
                self.column_mode = !self.column_mode;
                self.status.set_message(Some(
                    if self.column_mode {
                        "Column mode on"
                    } else {
                        "Column mode off"
                    }
                    .to_string(),
                ));
            }
            Action::ClipboardHistory => self.show_clipboard_history(),
            Action::PasteClipboard(index) => {
                if let Some(text) = self.clipboard_history.get(index).cloned() {
                    self.after_edit(|ed| ed.insert(&text));
                }
            }
            Action::ReadOnlyCurrent => self.set_read_only(ReadScope::Current, true),
            Action::ReadOnlyAll => self.set_read_only(ReadScope::All, true),
            Action::ClearReadOnlyAll => self.set_read_only(ReadScope::All, false),
            Action::SelectAndFindNext => self.select_and_find(true),
            Action::SelectAndFindPrev => self.select_and_find(false),
            Action::VolatileFindNext => self.volatile_find(true),
            Action::VolatileFindPrev => self.volatile_find(false),
            Action::SearchResultsWindow => self.search_results_window(),
            Action::NextSearchResult => self.step_result(true),
            Action::PrevSearchResult => self.step_result(false),
            Action::SelectBetweenBraces => self.select_between_braces(),
            Action::Mark => self.mark_dialog(),
            Action::ChangeNext => self.step_change(true),
            Action::ChangePrev => self.step_change(false),
            Action::ClearChangeHistory => {
                self.tab_mut().editor.clear_history();
                self.refresh_history();
                self.status
                    .set_message(Some("Change history cleared".into()));
            }
            Action::StyleAll(style) => self.style_all(style),
            Action::StyleOne(style) => self.style_one(style),
            Action::ClearStyle(style) => self.clear_style(Some(style)),
            Action::ClearAllStyles => self.clear_style(None),
            Action::JumpUp(style) => self.jump_mark(style, false),
            Action::JumpDown(style) => self.jump_mark(style, true),
            Action::CopyStyled(style) => self.copy_styled(style),
            Action::CutBookmarkedLines => self.bookmark_lines(BookmarkCommand::Cut),
            Action::CopyBookmarkedLines => self.bookmark_lines(BookmarkCommand::Copy),
            Action::PasteToBookmarkedLines => self.bookmark_lines(BookmarkCommand::Paste),
            Action::RemoveBookmarkedLines => self.bookmark_lines(BookmarkCommand::Remove),
            Action::RemoveNonBookmarkedLines => {
                self.bookmark_lines(BookmarkCommand::RemoveUnmarked)
            }
            Action::InverseBookmarks => self.bookmark_lines(BookmarkCommand::Inverse),
            Action::FindCharInRange => {
                self.prompt.open(PromptKind::FindChar, "");
            }
            Action::HashGenerate(hash) => self.hash_document(hash),
            Action::HashToClipboard(hash) => self.hash_selection(hash),
            Action::HashFromFile(hash) => {
                self.pending_hash = Some(hash);
                self.prompt.open(PromptKind::HashFile, "");
            }
            Action::OpenPluginsFolder => self.open_plugins_folder(),
            Action::ManagePlugins => self.open_plugin_manager(),
            Action::ReloadPlugins => self.reload_plugins(),
            Action::Plugin(plugin, command) => self.run_plugin(plugin, command),
            Action::ValidateShortcuts => {
                self.prompt.open(PromptKind::ValidateShortcuts, "");
            }
            Action::StopRecording => self.stop_recording(),
            Action::SaveMacro => self.save_macro(),
            Action::RunMacroMultiple => {
                self.prompt.open(PromptKind::MacroRepeat, "2");
            }
            Action::WindowsList => self.windows_list(),
            Action::FocusTab(index) => {
                if index < self.tabs.len() {
                    self.active = index;
                    self.sync_tabs();
                    self.refresh_brace();
                }
            }
            Action::RecentWindow => self.cycle_tab(false),
            Action::SortWindows(sort) => self.sort_windows(sort),
            Action::CommandLineArgs => self.command_line_args(),
            Action::DebugInfo => self.debug_info(),
            Action::HomePage => self.open_page(nn_app::links::HOME),
            Action::ProjectPage => self.open_page(nn_app::links::ISSUES),
            Action::OnlineManual => self.open_page(nn_app::links::MANUAL),
            Action::Forum => self.open_page(nn_app::links::community()),
            Action::License => self.open_page(nn_app::links::LICENCE),
            Action::ReadOnly => self.toggle_read_only(),
            Action::ShowWhitespace => {
                let on = !self.view.show_whitespace();
                self.view.set_show_whitespace(on);
            }
            Action::ShowEol => {
                let on = !self.view.show_eol();
                self.view.set_show_eol(on);
            }
            Action::ShowIndentGuides => {
                let on = !self.view.show_indent_guides();
                self.view.set_show_indent_guides(on);
            }
            Action::MarkAll => self.mark_all(),
            Action::GotoBrace => self.goto_brace(),
            Action::SplitVertical => self.split_window(Split::Vertical),
            Action::SplitHorizontal => self.split_window(Split::Horizontal),
            Action::RotateSplit => self.rotate_split(),
            Action::Unsplit => self.unsplit(),
            Action::MoveToOtherView => self.move_to_other(),
            Action::CloneToOtherView => self.clone_to_other(),
            Action::AlwaysOnTop => self.toggle_always_on_top(),
            Action::PostIt => self.post_it(),
            Action::ViewInBrowser(kind) => self.view_in_browser(kind),
            Action::ShowNonPrinting => {
                let on = !self.view.show_non_printing();
                self.view.set_show_non_printing(on);
            }
            Action::ShowControlChars => {
                let on = !self.view.show_control_chars();
                self.view.set_show_control_chars(on);
            }
            Action::ShowAllChars => self.show_all_chars(),
            Action::ShowWrapSymbol => {
                let on = !self.view.show_wrap_symbol();
                self.view.set_show_wrap_symbol(on);
            }
            Action::ZoomSync => {
                self.settings.zoom_sync = !self.settings.zoom_sync;
                self.settings.save();
                self.status.set_message(Some(
                    if self.settings.zoom_sync {
                        "Zoom: synchronized across views"
                    } else {
                        "Zoom: this view only"
                    }
                    .to_string(),
                ));
            }
            Action::MoveToNewInstance => self.new_instance(true),
            Action::OpenInNewInstance => self.new_instance(false),
            Action::GotoTab(index) => self.switch_tab(index),
            Action::FirstTab => self.switch_tab(0),
            Action::LastTab => self.switch_tab(self.tabs.len().saturating_sub(1)),
            Action::MoveTabStart => self.move_tab_to(0),
            Action::MoveTabEnd => self.move_tab_to(self.tabs.len().saturating_sub(1)),
            Action::MoveTabForward => self.move_tab_step(true),
            Action::MoveTabBackward => self.move_tab_step(false),
            Action::TabColourSet(index) => self.set_tab_colour(index),
            Action::FocusOtherView => self.focus_other(),
            Action::HideLines => self.hide_lines(),
            Action::FoldCurrentLevel => self.fold_current_level(true),
            Action::UnfoldCurrentLevel => self.fold_current_level(false),
            Action::FoldLevel(level) => self.fold_to_level(level as u16, true),
            Action::UnfoldLevel(level) => self.fold_to_level(level as u16, false),
            Action::Summary => self.summary(),
            Action::DocumentMap => self.toggle_minimap(),
            Action::DocumentList => self.windows_list(),
            Action::SyncScrollV => {
                self.settings.sync_scroll_v = !self.settings.sync_scroll_v;
                self.settings.save();
                self.status.set_message(Some(
                    if self.settings.sync_scroll_v {
                        "Vertical scrolling: synchronized"
                    } else {
                        "Vertical scrolling: independent"
                    }
                    .to_string(),
                ));
            }
            Action::SyncScrollH => {
                self.settings.sync_scroll_h = !self.settings.sync_scroll_h;
                self.settings.save();
                self.status.set_message(Some(
                    if self.settings.sync_scroll_h {
                        "Horizontal scrolling: synchronized"
                    } else {
                        "Horizontal scrolling: independent"
                    }
                    .to_string(),
                ));
            }
            Action::Monitoring => self.toggle_monitoring(),
            Action::About => self.about.open(),
        }
        self.window.request_redraw();
    }

    /// Switches to `choice` and records it.
    ///
    /// The Settings menu is the discoverable half of the theme choice: the
    /// Preferences row opens a list of fifty palettes, which is where you go to
    /// browse them, while this is where you name one you already know.
    fn choose_theme(&mut self, choice: ThemeChoice) {
        if self.settings.theme == choice {
            return;
        }
        let mut settings = self.settings.clone();
        settings.theme = choice;
        self.prefs.set_settings(settings);
        self.apply_settings();
        self.status
            .set_message(Some(format!("Theme: {}", choice.label())));
    }

    /// Flips between the dark and the light palette.
    ///
    /// The theme lives in the view and every other widget is handed it at draw
    /// time, so one assignment repaints the whole window.
    fn toggle_theme(&mut self) {
        // By brightness rather than by identity, so the toggle still does
        // something sensible when the current theme was imported from a file
        // and is neither of the two built-in ones.
        let next = if self.view.theme().background.luminance() < 128 {
            Theme::LIGHT
        } else {
            Theme::DARK
        };
        self.view.set_theme(next);
        self.window.request_redraw();
    }

    /// Applies a line transform to the selection, if it changes anything.
    ///
    /// These are commands rather than keystrokes, so they are not recorded into
    /// a macro: replaying a sort would sort an already-sorted block.
    /// Extends the selection to the next or previous match of what is selected.
    ///
    /// Notepad++'s "Select and Find Next": the first press selects the word under
    /// the caret, and each press after that grows the selection to the following
    /// occurrence. The selection is what the search uses, so the user never has to
    /// type the needle into the find bar.
    fn select_and_find(&mut self, forward: bool) {
        let editor = &self.tabs[self.active].editor;
        let needle = if editor.selection_len() > 0 {
            editor.selected_text()
        } else {
            editor
                .highlight_word()
                .map(|range| editor.doc.slice(range))
                .unwrap_or_default()
        };
        if needle.is_empty() {
            self.status
                .set_message(Some("Select something to search for".into()));
            return;
        }
        let from = if forward {
            self.tab().editor.selection_end()
        } else {
            self.tab().editor.selection_start()
        };
        let found = next_match(&self.tab().editor, &needle, from, forward);
        match found {
            Some(range) => {
                // The selection grows rather than moves: the anchor stays where it
                // was, so the user ends up with everything between the first hit
                // and this one.
                let anchor = self.tab().editor.selection_start();
                let end = range.end.max(anchor);
                self.after_edit(|ed| ed.select_range(anchor..end));
            }
            None => self
                .status
                .set_message(Some(format!("No further match for {needle:?}"))),
        }
    }

    /// Steps through matches without opening or changing the find bar.
    ///
    /// "Volatile" in Notepad++ means the search does not persist: the highlight
    /// is dropped as soon as the caret moves. Here the matches are recomputed from
    /// the selection each time, which is the same promise with less state.
    fn volatile_find(&mut self, forward: bool) {
        let editor = &self.tabs[self.active].editor;
        let needle = if editor.selection_len() > 0 {
            editor.selected_text()
        } else {
            editor
                .highlight_word()
                .map(|range| editor.doc.slice(range))
                .unwrap_or_default()
        };
        if needle.is_empty() {
            self.status
                .set_message(Some("Select something to search for".into()));
            return;
        }
        let from = if forward {
            self.tab().editor.selection_end()
        } else {
            self.tab().editor.selection_start()
        };
        let found = next_match(&self.tab().editor, &needle, from, forward);
        match found {
            Some(range) => {
                self.after_edit(|ed| ed.select_range(range));
                self.view.set_matches(Vec::new(), None);
            }
            None => self
                .status
                .set_message(Some(format!("No further match for {needle:?}"))),
        }
    }

    /// Brings the results panel to the front, opening it if it is closed.
    fn search_results_window(&mut self) {
        if !self.results.is_open() {
            self.status
                .set_message(Some("No search results yet".into()));
            return;
        }
        self.status.set_message(Some(format!(
            "{} result(s) in the panel",
            self.results.hits().len()
        )));
        self.window.request_redraw();
    }

    /// Steps through the hits in the results panel.
    fn step_result(&mut self, forward: bool) {
        let hits = self.results.hits().to_vec();
        if hits.is_empty() {
            self.status
                .set_message(Some("No search results yet".into()));
            return;
        }
        let current = self.result_cursor.unwrap_or(if forward {
            usize::MAX
        } else {
            0
        });
        let next = if forward {
            if current == usize::MAX {
                0
            } else {
                (current + 1) % hits.len()
            }
        } else if current == 0 {
            hits.len() - 1
        } else {
            current - 1
        };
        self.result_cursor = Some(next);
        let hit = &hits[next];
        let path = hit.path.clone();
        let line = hit.line;
        self.open_path(&path);
        self.tab_mut().editor.goto_position(line, 0);
        self.tab_mut().editor.ensure_caret_visible();
        self.status
            .set_message(Some(format!("Result {} of {}", next + 1, hits.len())));
    }

    /// Selects everything between the brace next to the caret and its partner.
    fn select_between_braces(&mut self) {
        let editor = &self.tabs[self.active].editor;
        let Some((open, close)) = editor.matching_brace() else {
            self.status
                .set_message(Some("No matching brace here".into()));
            return;
        };
        // Inside the braces, not including them: the command is about the text
        // between, and a selection that carried the delimiters would have to be
        // trimmed by hand before it could be used.
        let range = (open + 1).min(close)..close;
        self.after_edit(|ed| ed.select_range(range));
    }

    /// Marks every occurrence of the selection in the first style.
    ///
    /// Notepad++'s Mark dialog takes a regular expression and a style. This build
    /// has no regex engine, so the dialog is the selection itself and the style is
    /// the first one; the five-style submenus are how a user picks a different
    /// colour.
    fn mark_dialog(&mut self) {
        let editor = &self.tabs[self.active].editor;
        let needle = if editor.selection_len() > 0 {
            editor.selected_text()
        } else {
            editor
                .highlight_word()
                .map(|range| editor.doc.slice(range))
                .unwrap_or_default()
        };
        if needle.is_empty() {
            self.status
                .set_message(Some("Select what to mark first".into()));
            return;
        }
        let count = self.tab_mut().editor.style_all_occurrences(0, true, true);
        self.status
            .set_message(Some(format!("Marked {count} occurrence(s) in style 1")));
    }

    /// Paints every occurrence of the current token in `style`.
    fn style_all(&mut self, style: usize) {
        let count = self.tab_mut().editor.style_all_occurrences(style, true, true);
        self.status.set_message(Some(format!(
            "{} mark(s) in style {}",
            count,
            style + 1
        )));
    }

    /// Paints just the occurrence the caret is on.
    fn style_one(&mut self, style: usize) {
        let painted = self.tab_mut().editor.style_one_token(style);
        self.status.set_message(Some(if painted {
            format!("Marked in style {}", style + 1)
        } else {
            "Nothing under the caret to mark".to_string()
        }));
    }

    /// Drops the marks in `style`, or in every style.
    fn clear_style(&mut self, style: Option<usize>) {
        match style {
            Some(style) => self.tab_mut().editor.clear_style(style),
            None => self.tab_mut().editor.clear_all_styles(),
        }
        self.window.request_redraw();
    }

    /// Jumps to the next or previous mark.
    ///
    /// `None` means "whichever style has a mark in that direction", which is what
    /// Notepad++'s "Find Mark Style" does: it does not care which colour the mark
    /// is, only that there is one.
    fn jump_mark(&mut self, style: Option<usize>, forward: bool) {
        let jumped = match style {
            Some(style) => self.tab_mut().editor.jump_mark(style, forward),
            None => {
                // Try each style in turn, nearest first, so a document with marks
                // in two colours still walks them in document order.
                let mut best: Option<usize> = None;
                for candidate in 0..nn_core::editor::MARK_STYLES {
                    if self.tab().editor.marks(candidate).is_empty() {
                        continue;
                    }
                    let caret = self.tab().editor.caret();
                    let marks = self.tab().editor.marks(candidate);
                    let hit = if forward {
                        marks.iter().any(|m| m.start > caret)
                    } else {
                        marks.iter().any(|m| m.end < caret)
                    };
                    if hit {
                        best = Some(candidate);
                        break;
                    }
                }
                let style = best.unwrap_or_else(|| {
                    (0..nn_core::editor::MARK_STYLES)
                        .find(|&s| !self.tab().editor.marks(s).is_empty())
                        .unwrap_or(0)
                });
                self.tab_mut().editor.jump_mark(style, forward)
            }
        };
        if !jumped {
            self.status.set_message(Some("No marks to jump to".into()));
        }
    }

    /// Copies the marked text, one mark per line.
    fn copy_styled(&mut self, style: Option<usize>) {
        let text = match style {
            Some(style) => self.tab().editor.styled_text(style),
            None => {
                let mut parts: Vec<String> = Vec::new();
                for style in 0..nn_core::editor::MARK_STYLES {
                    let text = self.tab().editor.styled_text(style);
                    if !text.is_empty() {
                        parts.push(text);
                    }
                }
                parts.join("\n")
            }
        };
        if text.is_empty() {
            self.status.set_message(Some("No marks to copy".into()));
            return;
        }
        let lines = text.lines().count();
        self.copy_to_clipboard(text);
        self.status
            .set_message(Some(format!("Copied {lines} marked line(s)")));
    }

    /// Runs one of the bookmarked-line operations.
    fn bookmark_lines(&mut self, command: BookmarkCommand) {
        let editor = &self.tabs[self.active].editor;
        let doc = &editor.doc;
        let count = doc.len_lines();
        let marked: Vec<usize> = (0..count).filter(|&line| doc.is_bookmarked(line)).collect();
        if marked.is_empty() && command != BookmarkCommand::Inverse {
            self.status
                .set_message(Some("No bookmarked lines".into()));
            return;
        }
        match command {
            BookmarkCommand::Cut | BookmarkCommand::Copy => {
                let text: Vec<String> = marked.iter().map(|&line| doc.line(line)).collect();
                let text = text.join("\n");
                if command == BookmarkCommand::Cut {
                    self.copy_to_clipboard(text);
                    self.remove_lines(&marked);
                } else {
                    self.copy_to_clipboard(text);
                }
            }
            BookmarkCommand::Paste => {
                let pasted = self
                    .clipboard
                    .as_mut()
                    .and_then(|c| c.get_text().ok())
                    .unwrap_or_default();
                if pasted.is_empty() {
                    return;
                }
                let lines: Vec<&str> = pasted.lines().collect();
                // One pasted line per bookmarked line, in order. A paste with more
                // lines than there are bookmarks stops at the last bookmark rather
                // than inventing lines to put the rest on.
                let edits: Vec<(std::ops::Range<usize>, String)> = marked
                    .iter()
                    .zip(lines.iter())
                    .map(|(&line, text)| {
                        let start = doc.line_to_char(line);
                        (start..start + doc.line_len(line), (*text).to_string())
                    })
                    .collect();
                self.after_edit(|ed| ed.replace_ranges(&edits));
            }
            BookmarkCommand::Remove => self.remove_lines(&marked),
            BookmarkCommand::RemoveUnmarked => {
                let keep: Vec<usize> = (0..count).filter(|&line| doc.is_bookmarked(line)).collect();
                self.keep_lines(&keep);
            }
            BookmarkCommand::Inverse => {
                for line in 0..count {
                    self.tabs[self.active].editor.doc.toggle_bookmark(line);
                }
            }
        }
        self.window.request_redraw();
    }

    /// Steps through the change history, reverting to the row reached.
    ///
    /// The rows are the undo stack, so "go to the next change" is "undo one more
    /// step" and "go to the previous change" is "redo one". Walking the panel
    /// rather than the stack keeps the two in step: the row the user is looking at
    /// is the state they get.
    fn step_change(&mut self, forward: bool) {
        let moved = if forward {
            self.tab_mut().editor.undo()
        } else {
            self.tab_mut().editor.redo()
        };
        if !moved {
            self.status.set_message(Some(
                if forward {
                    "Already at the oldest change"
                } else {
                    "Already at the newest change"
                }
                .to_string(),
            ));
            return;
        }
        let tab = &mut self.tabs[self.active];
        let line = tab.editor.caret_line();
        tab.highlighter.highlight_from(&mut tab.editor.doc, line);
        tab.folder.apply_from(&mut tab.editor.doc, line);
        tab.editor.ensure_caret_visible();
        self.refresh_history();
        self.update_title();
        self.window.request_redraw();
    }

    /// Deletes the given lines, bottom up so the offsets stay valid.
    fn remove_lines(&mut self, lines: &[usize]) {
        let doc = &self.tabs[self.active].editor.doc;
        let mut edits: Vec<(std::ops::Range<usize>, String)> = lines
            .iter()
            .rev()
            .map(|&line| {
                // The whole line including its break, so removing it does not
                // leave a blank line behind.
                let range = doc.line_range(line);
                let end = if range.end < doc.len_chars() {
                    range.end + 1
                } else {
                    range.end
                };
                range.start..end
            })
            .map(|range| (range, String::new()))
            .collect();
        edits.sort_by_key(|(range, _)| range.start);
        self.after_edit(|ed| ed.replace_ranges(&edits));
    }

    /// Keeps only the given lines, dropping every other one.
    fn keep_lines(&mut self, keep: &[usize]) {
        let doc = &self.tabs[self.active].editor.doc;
        let count = doc.len_lines();
        let drop: Vec<usize> = (0..count).filter(|line| !keep.contains(line)).collect();
        self.remove_lines(&drop);
    }

    /// Finds the next character at or after a code point.
    fn find_char_in_range(&mut self, code: u32) {
        let Some(target) = char::from_u32(code) else {
            self.status
                .set_message(Some(format!("{code} is not a character")));
            return;
        };
        let editor = &self.tabs[self.active].editor;
        let text = editor.doc.text();
        let from = editor.selection_end();
        let found = text
            .chars()
            .enumerate()
            .skip(from)
            .find(|(_, c)| *c == target)
            .map(|(index, _)| index);
        match found {
            Some(index) => {
                self.after_edit(|ed| ed.select_range(index..index + 1));
                self.status
                    .set_message(Some(format!("Found {target:?} at {index}")));
            }
            None => self
                .status
                .set_message(Some(format!("No {target:?} after the caret"))),
        }
    }

    /// Hashes the active document and puts the digest on the clipboard.
    ///
    /// The clipboard rather than a dialog: a digest is only ever wanted pasted
    /// somewhere else, so a modal box would be one more click between the user and
    /// the thing they asked for. The status bar carries the confirmation, and the
    /// digest is short enough to read there.
    fn hash_document(&mut self, hash: nn_core::hash::Hash) {
        let text = self.tab().editor.doc.text();
        let digest = hash.digest(text.as_bytes());
        self.copy_to_clipboard(digest.clone());
        self.status
            .set_message(Some(format!("{}: {digest}", hash.name())));
    }

    /// Hashes the selection, or the whole document when nothing is selected.
    fn hash_selection(&mut self, hash: nn_core::hash::Hash) {
        let editor = &self.tabs[self.active].editor;
        let text = if editor.selection_len() > 0 {
            editor.selected_text()
        } else {
            editor.doc.text()
        };
        let digest = hash.digest(text.as_bytes());
        self.copy_to_clipboard(digest.clone());
        self.status
            .set_message(Some(format!("{}: {digest}", hash.name())));
    }

    /// Hashes a file on disk, read in one go.
    ///
    /// A digest is over the bytes, so the file is read as bytes rather than as
    /// text: a line-ending conversion on the way in would change the answer.
    fn hash_file(&mut self, hash: nn_core::hash::Hash, path: &Path) {
        match std::fs::read(path) {
            Ok(bytes) => {
                let digest = hash.digest(&bytes);
                self.copy_to_clipboard(digest.clone());
                self.status.set_message(Some(format!(
                    "{} of {}: {digest}",
                    hash.name(),
                    path.display()
                )));
            }
            Err(err) => self
                .status
                .set_message(Some(format!("Could not read {}: {err}", path.display()))),
        }
    }

    /// Opens the directory plugins are loaded from.
    fn open_plugins_folder(&mut self) {
        let Some(dir) = nn_app::paths::config_file("plugins") else {
            self.status
                .set_message(Some("No configuration directory".into()));
            return;
        };
        // The directory may not exist yet on a fresh install, and a file manager
        // pointed at a missing path opens nothing at all.
        if let Err(err) = std::fs::create_dir_all(&dir) {
            self.status
                .set_message(Some(format!("Could not create {}: {err}", dir.display())));
            return;
        }
        if let Err(err) = nn_app::desktop::open_path(&dir) {
            self.status
                .set_message(Some(format!("Could not open a file manager: {err}")));
        }
    }

    /// Reads the plugins directory again, without a restart.
    ///
    /// The directory is the list of what is installed, so this is also the way
    /// to pick up a `.wasm` that was copied in by hand. What it does *not* do is
    /// switch anything new on: the plugins that were running stay running and a
    /// file that has just arrived arrives switched off, which is the rule a
    /// first start follows too.
    fn reload_plugins(&mut self) {
        let running = self.plugins.names();
        self.plugins = load_plugins();
        let started = self.plugins.enable_saved(&running);
        self.remember_plugins();
        self.sync_plugin_menu();
        if self.plugin_list.is_open() {
            self.plugin_list.set_rows(self.plugin_rows());
        }
        let failures = self.plugins.failures().len();
        let available = self.plugins.available();
        self.status.set_message(Some(if failures == 0 {
            format!("{started} of {available} plugin(s) enabled")
        } else {
            format!("{started} of {available} plugin(s) enabled, {failures} failed")
        }));
        self.window.request_redraw();
    }

    /// Opens the plugin manager on what is installed right now.
    fn open_plugin_manager(&mut self) {
        self.plugin_list.open(self.plugin_rows());
        self.window.request_redraw();
    }

    /// The manager's rows, one per installed plugin, in folder order.
    fn plugin_rows(&self) -> Vec<PluginRow> {
        self.plugins
            .inventory()
            .into_iter()
            .map(|entry| PluginRow {
                name: entry.name,
                file: entry.file,
                enabled: entry.enabled,
            })
            .collect()
    }

    /// Switches one plugin on or off and writes the decision down.
    ///
    /// The settings are rewritten from the list of what is actually running,
    /// not edited in place: a plugin whose file was deleted while the editor
    /// was open leaves a name in the file that matches nothing, and this is the
    /// one moment where the truth is in memory.
    fn set_plugin_enabled(&mut self, index: usize, enabled: bool) {
        if enabled {
            match self.plugins.enable(index) {
                Ok(name) => self.status.set_message(Some(format!("Loaded {name}"))),
                Err(error) => self
                    .status
                    .set_message(Some(format!("Cannot load the plugin: {error}"))),
            }
        } else if let Some(name) = self.plugins.disable(index) {
            self.status.set_message(Some(format!("Unloaded {name}")));
        }
        self.remember_plugins();
        self.sync_plugin_menu();
        self.plugin_list.set_rows(self.plugin_rows());
        self.window.request_redraw();
    }

    /// Runs one command of one plugin over the active document.
    ///
    /// The crossing is where the units change: the editor counts characters and
    /// a plugin counts bytes, so the text, the caret and the selection are
    /// converted here, once per command, rather than in every plugin that opens
    /// a document.
    ///
    /// A command that writes comes back as a whole document, and the editor
    /// replaces the buffer with it, which lands in the undo stack as one edit. A
    /// command that fails leaves the document as it was and says why in the
    /// status bar: a plugin is other people's code, and the answer to its
    /// trouble is not a half-applied edit.
    fn run_plugin(&mut self, plugin: usize, command: usize) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let text = tab.editor.doc.text();
        let context = Context {
            text: &text,
            caret: tab.editor.doc.char_to_byte(tab.editor.caret()),
            selection: (
                tab.editor.doc.char_to_byte(tab.editor.selection_start()),
                tab.editor.doc.char_to_byte(tab.editor.selection_end()),
            ),
            read_only: tab.editor.doc.is_read_only(),
        };
        let outcome = match self.plugins.run(plugin, command, &context) {
            Ok(outcome) => outcome,
            Err(error) => {
                self.status
                    .set_message(Some(format!("Plugin failed: {error}")));
                self.window.request_redraw();
                return;
            }
        };
        for line in &outcome.log {
            println!("[plugin] {line}");
        }
        if let Some(new) = outcome.text {
            if new != text {
                self.after_edit(|ed| {
                    let len = ed.doc.len_chars();
                    ed.replace_ranges(&[(0..len, new.clone())]);
                });
            }
        }
        // A selection *is* a caret with an anchor, so a command that sets one
        // has said where the user now is and a separate caret would contradict
        // it. `set_caret` alone still works for a command that only moves.
        if let Some((start, end)) = outcome.selection {
            let (start, end) = {
                let doc = &self.tab().editor.doc;
                (doc.byte_to_char(start), doc.byte_to_char(end))
            };
            self.tab_mut().editor.select_range(start..end);
            self.tab_mut().editor.ensure_caret_visible();
        } else if let Some(caret) = outcome.caret {
            let caret = self.tab().editor.doc.byte_to_char(caret);
            self.tab_mut().editor.set_caret(caret);
            self.tab_mut().editor.ensure_caret_visible();
        }
        self.window.request_redraw();
    }

    /// Writes the names of the running plugins into the settings and to disk.
    fn remember_plugins(&mut self) {
        self.settings.enabled_plugins = self.plugins.names();
        self.settings.save();
    }

    /// Handles a key while the plugin manager is open.
    ///
    /// Every key is consumed: a modal that let typing through to the document
    /// would be a modal you cannot trust.
    fn handle_plugin_list_key(&mut self, event: &winit::event::KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::ArrowDown) => self.plugin_list.move_selection(1),
            Key::Named(NamedKey::ArrowUp) => self.plugin_list.move_selection(-1),
            Key::Named(NamedKey::PageDown) => self.plugin_list.page(1),
            Key::Named(NamedKey::PageUp) => self.plugin_list.page(-1),
            Key::Named(NamedKey::Home) => self.plugin_list.home(),
            Key::Named(NamedKey::End) => self.plugin_list.end(),
            Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Space) => {
                if let Some((index, enabled)) = self.plugin_list.toggle() {
                    self.set_plugin_enabled(index, enabled);
                }
            }
            Key::Named(NamedKey::Escape) => self.plugin_list.close(),
            _ => {}
        }
        self.window.request_redraw();
    }

    /// Stops the recorder, if it is running.
    fn stop_recording(&mut self) {
        if !self.recorder.is_recording() {
            self.status
                .set_message(Some("Not recording".into()));
            return;
        }
        self.recorder.stop();
        self.status.set_recording(None);
        self.status.set_message(Some(format!(
            "Macro recorded, {} steps",
            self.recorder.len()
        )));
    }

    /// Saves the recorded macro as a Run menu entry.
    ///
    /// Notepad++ writes a macro file per macro; this build keeps macros in the
    /// settings beside the Run commands, because a macro here *is* a recorded
    /// keystroke list and the Run list is already the place a saved sequence
    /// lives. The label is the step count, which is the only thing about a
    /// recording that can be named without asking the user to invent a name.
    fn save_macro(&mut self) {
        if self.recorder.is_empty() {
            self.status
                .set_message(Some("Nothing recorded to save".into()));
            return;
        }
        let label = format!("Macro ({} steps)", self.recorder.len());
        self.settings.runs.add(RunCommand::new(label));
        self.settings.save();
        self.sync_runs();
        self.status
            .set_message(Some("Macro saved to the Run menu".into()));
    }

    /// Plays the recorded macro a number of times.
    fn run_macro_multiple(&mut self, times: usize) {
        if self.recorder.is_empty() {
            self.status
                .set_message(Some("Nothing recorded to play".into()));
            return;
        }
        let times = times.clamp(1, MAX_MACRO_REPEAT);
        for _ in 0..times {
            self.play_macro();
        }
        self.status
            .set_message(Some(format!("Macro played {times} time(s)")));
    }

    /// Shows the open documents as a popup, newest tab last.
    fn windows_list(&mut self) {
        let entries: Vec<Entry> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let mut label = tab.title();
                if index == self.active {
                    label.push_str("  (active)");
                }
                Entry::labelled(label, Action::FocusTab(index), true)
            })
            .collect();
        let (x, y) = self.view.caret_point(&mut self.tabs[self.active].editor);
        self.context.show(x, y, entries);
    }

    /// Reorders the tab strip by one of the Window menu's ten keys.
    ///
    /// The active document is tracked by identity rather than by index, because
    /// sorting moves it: keeping the index would silently switch the user to a
    /// different file.
    fn sort_windows(&mut self, sort: WindowSort) {
        let active = self.active;
        let mut order: Vec<usize> = (0..self.tabs.len()).collect();
        let key = |index: usize| -> String {
            let tab = &self.tabs[index];
            let path = tab.editor.doc.path();
            match sort {
                WindowSort::NameAsc | WindowSort::NameDesc => tab.title(),
                WindowSort::PathAsc | WindowSort::PathDesc => path
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| tab.title()),
                WindowSort::TypeAsc | WindowSort::TypeDesc => path
                    .and_then(|p| p.extension())
                    .map(|e| e.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                WindowSort::SizeAsc | WindowSort::SizeDesc => String::new(),
                WindowSort::ModifiedAsc | WindowSort::ModifiedDesc => String::new(),
            }
        };
        match sort {
            WindowSort::SizeAsc | WindowSort::SizeDesc => order.sort_by_key(|&index| {
                let len = self.tabs[index].editor.doc.len_chars();
                if sort == WindowSort::SizeAsc {
                    len
                } else {
                    usize::MAX - len
                }
            }),
            WindowSort::ModifiedAsc | WindowSort::ModifiedDesc => {
                order.sort_by_key(|&index| {
                    let stamp = self.tabs[index]
                        .editor
                        .doc
                        .path()
                        .and_then(|p| std::fs::metadata(p).ok())
                        .and_then(|m| m.modified().ok());
                    // `SystemTime` is not `Ord` because it can be before the
                    // epoch; the seconds count is, and a file's mtime is never
                    // near enough to the epoch for the sign to matter.
                    let secs = stamp
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    if sort == WindowSort::ModifiedAsc {
                        secs
                    } else {
                        u64::MAX - secs
                    }
                });
            }
            _ => {
                let descending = matches!(
                    sort,
                    WindowSort::NameDesc
                        | WindowSort::PathDesc
                        | WindowSort::TypeDesc
                );
                order.sort_by_cached_key(|&index| {
                    let text = key(index);
                    if descending {
                        // Reversing the key rather than the vector keeps the sort
                        // stable, so two documents with the same name stay in the
                        // order they were opened.
                        text.chars()
                            .map(|c| char::MAX as u32 - c as u32)
                            .collect::<Vec<u32>>()
                    } else {
                        text.chars().map(|c| c as u32).collect::<Vec<u32>>()
                    }
                });
            }
        }
        // Taken out and put back rather than cloned: a `Tab` owns an editor, a
        // highlighter and a fold tree, and copying all three to reorder a strip
        // would be a lot of work for a list of indices.
        let mut slots: Vec<Option<Tab>> = self.tabs.drain(..).map(Some).collect();
        self.tabs = order
            .iter()
            .filter_map(|&index| slots[index].take())
            .collect();
        self.active = order.iter().position(|&index| index == active).unwrap_or(0);
        self.sync_tabs();
        self.window.request_redraw();
    }

    /// Checks a `shortcuts.xml` and reports what it holds.
    ///
    /// The same reader the import uses, so a file that validates here is a file
    /// the import will accept. Reporting the skipped count is the point: a
    /// shortcut naming a command this build does not have is the one thing that
    /// silently disappears on import.
    fn validate_shortcuts(&mut self, path: &Path) {
        if !path.is_file() {
            self.status
                .set_message(Some(format!("No such file: {}", path.display())));
            return;
        }
        let import = nppimport::Import::read(path);
        let message = if import.keymap.is_empty() && import.skipped == 0 {
            format!("{}: no shortcuts found", path.display())
        } else {
            format!(
                "{}: {} shortcut(s), {} not recognised",
                path.display(),
                import.keymap.len(),
                import.skipped
            )
        };
        self.status.set_message(Some(message));
    }

    /// Takes the colours from a Notepad++ `stylers.xml`.
    ///
    /// The path is remembered rather than the colours, so editing the file and
    /// restarting picks the edit up. A file that cannot be read, or that has no
    /// `GlobalStyles` element, leaves the current theme alone and says so: a
    /// half-parsed theme would be worse than none, because the user would have
    /// no way to tell which colours came from where.
    fn import_theme(&mut self, path: &Path) {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) => {
                self.status
                    .set_message(Some(format!("Cannot read {}: {err}", path.display())));
                return;
            }
        };
        let Some(theme) = nn_app::theme::from_stylers(&text) else {
            self.status.set_message(Some(format!(
                "{}: no GlobalStyles element, so no colours to take",
                path.display()
            )));
            return;
        };
        self.settings.theme_path = Some(path.display().to_string());
        self.settings.save();
        self.apply_theme(theme);
        self.status
            .set_message(Some(format!("Theme imported from {}", path.display())));
    }

    /// Copies a plugin file into the plugins directory and loads it.
    ///
    /// A copy rather than a reference: the plugins directory is the one place
    /// the editor looks, and a plugin that lived somewhere else would vanish the
    /// next time the user tidied their downloads.
    fn import_plugin(&mut self, path: &Path) {
        if !path.is_file() {
            self.status
                .set_message(Some(format!("No such file: {}", path.display())));
            return;
        }
        let Some(dir) = nn_app::paths::config_file("plugins") else {
            return;
        };
        if let Err(err) = std::fs::create_dir_all(&dir) {
            self.status
                .set_message(Some(format!("Cannot create {}: {err}", dir.display())));
            return;
        }
        let Some(name) = path.file_name() else {
            return;
        };
        let target = dir.join(name);
        if let Err(err) = std::fs::copy(path, &target) {
            self.status
                .set_message(Some(format!("Cannot copy to {}: {err}", target.display())));
            return;
        }
        self.plugins = load_plugins();
        self.sync_plugin_menu();
        let commands: usize = self
            .plugins
            .inventory()
            .iter()
            .map(|entry| entry.commands.len())
            .sum();
        self.status.set_message(Some(format!(
            "Imported {}: {} command(s)",
            name.to_string_lossy(),
            commands
        )));
    }

    /// Puts a theme into every view and re-syncs the palettes.
    ///
    /// Both panes, because a theme is a property of the window rather than of a
    /// document: leaving the right pane on the old colours would look like a bug
    /// in the split rather than a half-applied theme.
    fn apply_theme(&mut self, theme: nn_app::theme::Theme) {
        self.view.set_theme(theme);
        if let Some(pane) = &mut self.other {
            pane.view.set_theme(theme);
        }
        self.sync_palette();
        self.window.request_redraw();
    }

    /// Opens a URL in the user's browser, reporting a failure in the status bar.
    fn open_page(&mut self, url: &str) {
        if let Err(err) = nn_app::desktop::open_url(url) {
            self.status
                .set_message(Some(format!("Could not open a browser: {err}")));
        }
    }

    /// Prints the command line the process was started with.
    fn command_line_args(&mut self) {
        let args: Vec<String> = std::env::args().collect();
        println!("npp: {} argument(s)", args.len());
        for (index, arg) in args.iter().enumerate() {
            println!("  [{index}] {arg}");
        }
        self.status
            .set_message(Some(format!("{} command line argument(s)", args.len())));
    }

    /// Prints the build and environment details a bug report needs.
    fn debug_info(&mut self) {
        println!("Neko++ debug info");
        println!("  version: {}", env!("CARGO_PKG_VERSION"));
        println!("  target: {}", std::env::consts::ARCH);
        println!("  os: {}", std::env::consts::OS);
        println!("  config: {:?}", nn_app::paths::config_dir());
        println!("  documents: {}", self.tabs.len());
        println!("  plugins: {}", self.plugins.len());
        println!("  languages: {}", self.langs.len());
        println!(
            "  theme: {} ({})",
            self.settings.theme.key(),
            if self.settings.theme.is_light(self.system_light) {
                "light"
            } else {
                "dark"
            }
        );
        self.status.set_message(Some("Debug info written to the log".into()));
    }

    /// "Begin/End Select": the first press drops a marker, the second selects.
    ///
    /// The marker is a caret position rather than a selection, so the user can
    /// move around freely between the two presses and still get the range they
    /// meant. A third press starts over, which is how the command stays usable
    /// without a separate "cancel".
    fn begin_end_select(&mut self) {
        let caret = self.tab().editor.caret();
        match self.select_anchor.take() {
            None => {
                self.select_anchor = Some(caret);
                self.status
                    .set_message(Some("Select from here: move the caret, then run it again".into()));
            }
            Some(anchor) => {
                let range = anchor.min(caret)..anchor.max(caret);
                self.after_edit(|ed| ed.select_range(range));
            }
        }
    }

    /// "Begin/End Select in Column Mode": the same, but as a rectangle.
    ///
    /// A rectangle needs a line and a column at each end, so the marker is a
    /// `(line, column)` pair rather than a character offset. The column is the
    /// caret's column on its line, which is what the user sees.
    fn begin_end_select_column(&mut self) {
        let editor = &self.tabs[self.active].editor;
        let line = editor.caret_line();
        let column = editor.caret() - editor.doc.line_to_char(line);
        match self.block_anchor.take() {
            None => {
                self.block_anchor = Some((line, column));
                self.status
                    .set_message(Some("Column select from here: move, then run it again".into()));
            }
            Some((from_line, from_col)) => {
                self.block_anchor = Some((from_line, from_col));
                self.status.set_message(Some(format!(
                    "Column block {}:{} to {}:{}",
                    from_line + 1,
                    from_col + 1,
                    line + 1,
                    column + 1
                )));
            }
        }
    }

    /// Inserts a date and time stamp at the caret.
    ///
    /// The two fixed shapes are Notepad++'s own: the short one is the clock, the
    /// long one puts the date after it. The customized one asks for a pattern and
    /// remembers it, because a user who wants a shape wants it every time.
    fn insert_datetime(&mut self, pattern: &str) {
        let stamp = nn_core::datetime::format_local(nn_core::now_secs(), pattern);
        self.after_edit(|ed| ed.insert(&stamp));
    }

    /// Copies the active file's name, or its directory, to the clipboard.
    fn copy_path_part(&mut self, whole_path: bool) {
        let Some(path) = self.tab().editor.doc.path().map(Path::to_path_buf) else {
            self.status
                .set_message(Some("This document has no file yet".into()));
            return;
        };
        let text = if whole_path {
            path.display().to_string()
        } else {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string())
        };
        self.copy_to_clipboard(text);
    }

    /// Copies every open document's name, or full path, one per line.
    ///
    /// Unsaved buffers are skipped rather than shown as "new 1": the command is
    /// about files, and a name that is not a file would be a lie in a list the
    /// user is about to paste somewhere.
    fn copy_all_paths(&mut self, whole_path: bool) {
        let mut lines: Vec<String> = Vec::new();
        for tab in &self.tabs {
            let Some(path) = tab.editor.doc.path() else {
                continue;
            };
            lines.push(if whole_path {
                path.display().to_string()
            } else {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string())
            });
        }
        if lines.is_empty() {
            self.status.set_message(Some("No saved files are open".into()));
            return;
        }
        let count = lines.len();
        self.copy_to_clipboard(lines.join("\n"));
        self.status
            .set_message(Some(format!("Copied {count} file name(s)")));
    }

    /// Inserts an empty line above or below the caret's line.
    fn insert_blank_line(&mut self, above: bool) {
        let editor = &self.tabs[self.active].editor;
        let line = editor.caret_line();
        let at = if above {
            editor.doc.line_to_char(line)
        } else {
            editor.doc.line_to_char(line) + editor.doc.line_len(line)
        };
        self.after_edit(|ed| {
            ed.select_range(at..at);
            ed.insert("\n");
        });
    }

    /// Opens the selection as a path: a file in a tab, a directory as workspace.
    fn open_selected(&mut self, folder: bool) {
        let text = self.tab().editor.selected_text();
        let text = text.trim();
        if text.is_empty() {
            self.status
                .set_message(Some("Select a path first".into()));
            return;
        }
        let path = PathBuf::from(text);
        if folder {
            if path.is_dir() {
                self.adopt_workspace(&path);
            } else {
                self.status
                    .set_message(Some(format!("Not a folder: {}", path.display())));
            }
            return;
        }
        if path.is_file() {
            self.open_path(&path);
        } else {
            self.status
                .set_message(Some(format!("Not a file: {}", path.display())));
        }
    }

    /// Replaces every selected character with a full block.
    ///
    /// The replacement is one block per character rather than one per selection,
    /// so the redacted text keeps its length and anything that lines up with it
    /// still does. That is the point of redacting in place instead of deleting.
    fn redact_selection(&mut self) {
        let editor = &self.tabs[self.active].editor;
        if editor.selection_len() == 0 {
            self.status
                .set_message(Some("Select what to redact first".into()));
            return;
        }
        let ranges: Vec<std::ops::Range<usize>> = editor
            .selections
            .iter()
            .map(|sel| {
                let (from, to) = (sel.anchor.min(sel.caret), sel.anchor.max(sel.caret));
                from..to
            })
            .filter(|range| !range.is_empty())
            .collect();
        let edits: Vec<(std::ops::Range<usize>, String)> = ranges
            .into_iter()
            .map(|range| {
                let blocks = "\u{2588}".repeat(range.end - range.start);
                (range, blocks)
            })
            .collect();
        self.after_edit(|ed| ed.replace_ranges(&edits));
    }

    /// Opens the selection, or the word under the caret, in a web search.
    fn search_on_internet(&mut self) {
        let editor = &self.tabs[self.active].editor;
        let needle = if editor.selection_len() > 0 {
            editor.selected_text()
        } else {
            editor
                .highlight_word()
                .map(|range| editor.doc.slice(range))
                .unwrap_or_default()
        };
        let needle = needle.trim();
        if needle.is_empty() {
            self.status
                .set_message(Some("Select a word to search for".into()));
            return;
        }
        let url = self
            .settings
            .search_engine
            .replace("$(CURRENT_WORD)", &percent_encode(needle));
        if let Err(err) = nn_app::desktop::open_url(&url) {
            self.status
                .set_message(Some(format!("Could not open a browser: {err}")));
        }
    }

    /// Runs one of the multi-select commands and reports what happened.
    fn multi_select(&mut self, mode: MatchMode, command: MultiCommand) {
        let (match_case, whole_word) = (mode.match_case, mode.whole_word);
        let mut message = String::new();
        self.after_edit(|ed| match command {
            MultiCommand::All => {
                let count = ed.select_all_occurrences(match_case, whole_word);
                message = format!("{count} occurrence(s) selected");
            }
            MultiCommand::Next => {
                message = if ed.select_next_occurrence(match_case, whole_word) {
                    "Added the next occurrence".to_string()
                } else {
                    "No further occurrence".to_string()
                };
            }
            MultiCommand::Skip => {
                message = if ed.skip_occurrence(match_case, whole_word) {
                    "Skipped to the next occurrence".to_string()
                } else {
                    "No further occurrence".to_string()
                };
            }
            MultiCommand::Undo => {
                message = if ed.undo_multi_select() {
                    "Dropped the last caret".to_string()
                } else {
                    "Only one caret left".to_string()
                };
            }
        });
        self.status.set_message(Some(message));
    }

    /// Shows the clipboard history as a popup at the caret.
    ///
    /// A popup rather than a dialog: the entries are short, there are few of them,
    /// and picking one should not take the focus away from the text.
    fn show_clipboard_history(&mut self) {
        if self.clipboard_history.is_empty() {
            self.status
                .set_message(Some("Nothing has been copied yet".into()));
            return;
        }
        let entries: Vec<nn_app::context::Entry> = self
            .clipboard_history
            .iter()
            .enumerate()
            .map(|(index, text)| {
                // One line per row: a multi-line copy would otherwise push the
                // popup off the screen and hide the entries below it.
                let label = text.lines().next().unwrap_or("").trim();
                let label = if label.chars().count() > 60 {
                    let cut: String = label.chars().take(59).collect();
                    format!("{cut}\u{2026}")
                } else {
                    label.to_string()
                };
                nn_app::context::Entry::labelled(
                    label,
                    Action::PasteClipboard(index),
                    true,
                )
            })
            .collect();
        let (x, y) = self.view.caret_point(&mut self.tabs[self.active].editor);
        self.context.show(x, y, entries);
    }

    /// Sets or clears the read-only flag on one document or on all of them.
    fn set_read_only(&mut self, scope: ReadScope, read_only: bool) {
        match scope {
            ReadScope::Current => {
                self.tabs[self.active].editor.doc.set_read_only(read_only);
            }
            ReadScope::All => {
                for tab in &mut self.tabs {
                    tab.editor.doc.set_read_only(read_only);
                }
            }
        }
        self.status.set_message(Some(
            match (scope, read_only) {
                (ReadScope::Current, true) => "This document is read only",
                (ReadScope::Current, false) => "This document is editable",
                (ReadScope::All, true) => "All documents are read only",
                (ReadScope::All, false) => "All documents are editable",
            }
            .to_string(),
        ));
    }

    fn line_op(&mut self, f: impl Fn(&str) -> String) {
        self.after_edit(|ed| {
            ed.transform_lines(f);
        });
    }

    /// Runs `action` if it is one of the text commands `nn_core::ops` owns.
    ///
    /// The options come from the editor, not the command: the tab width is the
    /// configured one, the sort's direction and key are the ones the submenu row
    /// carries, and a shuffling command gets a fresh seed so two presses of the
    /// same key are two different shuffles.
    fn run_text_command(&mut self, action: Action) {
        // The arm that routes here and `Action::TEXT_COMMANDS` are two lists of
        // the same commands, and `menu`'s tests hold that one to the core's
        // table. A row routed here that the list does not name is a row that
        // would be missing from the API.
        debug_assert!(
            action.is_text_command(),
            "{} is routed to the core table but is not a TEXT_COMMAND",
            action.name()
        );
        let name = action.name();
        let options = nn_core::ops::Options {
            width: self.settings.tab_width as usize,
            seed: nn_core::now_secs() as u64,
            sort: match action {
                Action::SortLines(sort) => sort.options(),
                _ => Default::default(),
            },
            ..Default::default()
        };
        self.line_op(move |text| {
            // `knows` just said the name is in the table, so only a table that
            // contradicted itself could fail here; giving the text back unchanged
            // is a better answer than a panic in a text editor.
            nn_core::ops::run(name, text, &options).unwrap_or_else(|_| text.to_string())
        });
    }

    /// The x of the function list's left edge.
    fn functions_left(&self) -> f32 {
        self.gpu.config.width as f32 - self.functions_width()
    }

    /// Puts the caret on the line a function-list row points at.
    fn goto_symbol(&mut self, index: usize) {
        let Some(symbol) = self.functions.symbols().get(index) else {
            return;
        };
        let line = symbol.line;
        self.tab_mut().editor.goto_position(line, 0);
        self.tab_mut().editor.ensure_caret_visible();
        self.refresh_brace();
        self.window.request_redraw();
    }

    /// The y of the results panel's top edge.
    fn results_top(&self) -> f32 {
        let height = self.gpu.config.height as f32;
        height - self.status_height() - self.history_height() - self.results_height()
    }

    /// The y of the change-history panel's top edge.
    fn history_top(&self) -> f32 {
        let height = self.gpu.config.height as f32;
        height - self.status_height() - self.history_height()
    }

    /// The y of the find bar's top edge.
    ///
    /// The same arithmetic the renderer uses, so a press lands on the chip the
    /// pointer is over. The bar is zero tall when it is closed, which puts this
    /// on the status bar's edge and lets the check above skip the whole thing.
    fn find_bar_top(&self) -> f32 {
        let height = self.gpu.config.height as f32;
        height
            - self.status_height()
            - self.history_height()
            - self.results_height()
            - self.find.height(
                self.gpu.config.width as f32,
                self.ui.line_height,
                self.view.padding(),
            )
    }

    /// Runs whatever a press on the find bar landed on.
    /// Shows one tab of the floating Search window.
    ///
    /// The tabs are not three searches: Replace is the Find tab plus a
    /// replacement row, and Mark is Find with the marking options. So switching
    /// only says whether the replacement row is part of the sheet, and moves
    /// the caret to the field the new tab is about -- the one you opened it to
    /// type in.
    fn show_find_tab(&mut self, tab: nn_app::finddialog::Tab) {
        use nn_app::findbar::Field;
        self.find_dialog.set_tab(tab);
        self.find.replace = tab.replaces();
        self.find
            .focus_field(if tab.replaces() {
                Field::Replacement
            } else {
                Field::Query
            });
        self.window.request_redraw();
    }

    fn apply_find_hit(&mut self, hit: Option<nn_app::findbar::Hit>) {
        use nn_app::findbar::{Button, Hit};
        match hit {
            Some(Hit::Chip(chip)) => {
                self.find.activate(chip);
                self.refresh_find();
            }
            Some(Hit::Button(Button::FindNext)) => {
                self.find.remember();
                self.step_match(self.find.up);
            }
            Some(Hit::Button(Button::Count)) => self.count_matches(),
            Some(Hit::Button(Button::FindAll)) => self.find_all_in_document(),
            Some(Hit::Button(Button::FindAllAll)) => self.find_all_in_opened_documents(),
            Some(Hit::Button(Button::Replace)) => self.replace_current(),
            Some(Hit::Button(Button::ReplaceAll)) => self.replace_all(),
            Some(Hit::Button(Button::ReplaceAllAll)) => self.replace_all_in_opened_documents(),
            Some(Hit::Button(Button::MarkAll)) => self.mark_all_matches(),
            Some(Hit::Button(Button::ClearMarks)) => self.clear_marks(),
            Some(Hit::Button(Button::Close)) => self.close_find(),
            Some(Hit::Swap(chip)) => {
                self.find.swap(chip);
                self.refresh_find();
            }
            Some(Hit::Field(field)) => self.find.focus_field(field),
            Some(Hit::HistoryToggle(field)) => self.find.toggle_history(field),
            Some(Hit::HistoryEntry(index)) => {
                if let Some(field) = self.find.history_open() {
                    self.find.pick_history(field, index);
                    self.refresh_find();
                }
            }
            None => {}
        }
        self.window.request_redraw();
    }

    /// Reports how many matches the pattern has, without moving the caret.
    fn count_matches(&mut self) {
        self.refresh_find();
        let count = self.find.matches.len();
        self.status.set_message(Some(match count {
            0 => "No match".to_string(),
            1 => "1 match".to_string(),
            n => format!("{n} matches"),
        }));
    }

    /// One results row per match `search` finds in tab `index`.
    ///
    /// The tab number travels with the row, so the panel can jump to a buffer
    /// that has never been saved, which has no path to open.
    fn hits_in_tab(&self, search: &nn_core::Search, index: usize) -> Vec<nn_core::Hit> {
        let doc = &self.tabs[index].editor.doc;
        let path = doc
            .path()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| std::path::PathBuf::from(format!("new {}", index + 1)));
        search
            .find_all(doc)
            .into_iter()
            .map(|range| {
                let line = doc.char_to_line(range.start);
                let column = range.start - doc.line_to_char(line);
                nn_core::Hit {
                    path: path.clone(),
                    line,
                    column,
                    text: doc.line(line).trim_end().to_string(),
                    tab: Some(index),
                }
            })
            .collect()
    }

    /// Lists every match in the active document, Notepad++'s "Find All in
    /// Current Document".
    fn find_all_in_document(&mut self) {
        self.refresh_find();
        let pattern = self.find.query.text().to_string();
        if pattern.is_empty() {
            return;
        }
        self.find.remember();
        let hits = match self.find.search() {
            Some(search) => self.hits_in_tab(search, self.active),
            None => return,
        };
        let files = if hits.is_empty() { 0 } else { 1 };
        let header = format!(
            "Search \"{pattern}\" ({}, {})",
            hits_noun(hits.len()),
            files_noun(files)
        );
        self.results.set_hits_named(hits, header);
    }

    /// Lists every match in every open document, Notepad++'s "Find All in All
    /// Opened Documents".
    fn find_all_in_opened_documents(&mut self) {
        self.refresh_find();
        let pattern = self.find.query.text().to_string();
        if pattern.is_empty() {
            return;
        }
        self.find.remember();
        let mut hits = Vec::new();
        let mut files = 0usize;
        if let Some(search) = self.find.search() {
            for index in 0..self.tabs.len() {
                let found = self.hits_in_tab(search, index);
                if !found.is_empty() {
                    files += 1;
                }
                hits.extend(found);
            }
        }
        let searched = self.tabs.len();
        let header = format!(
            "Search \"{pattern}\" ({}, {files} of {})",
            hits_noun(hits.len()),
            files_noun(searched)
        );
        self.results.set_hits_named(hits, header);
    }

    /// Replaces every match in every open document.
    ///
    /// Each document gets its own undo group, the way a Replace All on that
    /// document would, and the highlighting is refreshed from the top because
    /// the first replacement can shift every line after it.
    fn replace_all_in_opened_documents(&mut self) {
        let template = self.find.replacement.text().to_string();
        let mut replaced = 0usize;
        let mut files = 0usize;
        if let Some(search) = self.find.search() {
            for tab in &mut self.tabs {
                let matches = search.find_all(&tab.editor.doc);
                if matches.is_empty() {
                    continue;
                }
                let edits: Vec<_> = matches
                    .into_iter()
                    .map(|m| {
                        let text = search.expand(&tab.editor.doc, m.clone(), &template);
                        (m, text)
                    })
                    .collect();
                replaced += edits.len();
                files += 1;
                tab.editor.replace_ranges(&edits);
                if !tab.editor.doc.is_large() {
                    tab.highlighter.highlight_from(&mut tab.editor.doc, 0);
                    tab.folder.apply_from(&mut tab.editor.doc, 0);
                }
            }
        }
        if replaced == 0 {
            self.status.set_message(Some("No match".to_string()));
            return;
        }
        self.refresh_find();
        self.refresh_brace();
        self.refresh_smart();
        self.refresh_symbols();
        self.refresh_history();
        self.update_title();
        self.status.set_message(Some(format!(
            "Replaced {replaced} occurrence(s) in {files} file(s)"
        )));
    }

    /// Marks every match of the bar's pattern, Notepad++'s "Mark All".
    ///
    /// The marks are the bar's own matches rather than the word under the caret,
    /// so the Mark tab's pattern box and its options are the ones that decide
    /// what a match is.
    fn mark_all_matches(&mut self) {
        self.refresh_find();
        if self.find.query.text().is_empty() {
            self.status.set_message(Some("Nothing to mark".to_string()));
            return;
        }
        self.find.remember();
        let ranges = self.find.matches.clone();
        let purge = self.find.purge_marks;
        let bookmark_lines = self.find.bookmark_lines;
        if purge {
            // "Purge for each search" drops the marks the previous searches
            // left, whichever style they were made in. Clearing only the bar's
            // own slot would make the option a no-op, because marking replaces
            // that slot anyway.
            self.tab_mut().editor.clear_all_styles();
        }
        let count = self.tab_mut().editor.set_marks(0, ranges.clone());
        if bookmark_lines {
            let doc = &mut self.tab_mut().editor.doc;
            for range in &ranges {
                let line = doc.char_to_line(range.start);
                if !doc.is_bookmarked(line) {
                    doc.toggle_bookmark(line);
                }
            }
        }
        self.status
            .set_message(Some(format!("Marked {count} occurrence(s)")));
    }

    /// Drops the marks a mark left, and the bookmarks with them when the bar is
    /// set to bookmark its lines.
    fn clear_marks(&mut self) {
        self.tab_mut().editor.clear_style(0);
        if self.find.bookmark_lines {
            self.tab_mut().editor.doc.clear_bookmarks();
        }
        self.status.set_message(Some("Marks cleared".to_string()));
    }

    /// The directory a find-in-files search runs over by default.
    ///
    /// The folder holding the active file, because "find in files" means "find
    /// in the files around this one". An unsaved buffer falls back to the
    /// process's working directory.
    fn default_search_dir(&self) -> PathBuf {
        self.tab()
            .editor
            .doc
            .path()
            .and_then(|path| path.parent())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }

    /// Runs find-in-files and shows the results.
    fn find_in_files(&mut self, typed: &str) {
        let (pattern, directory) = split_query(typed, self.default_search_dir());
        if pattern.is_empty() {
            return;
        }
        match FindInFiles::new(&pattern, &directory).run() {
            Ok(hits) => {
                println!(
                    "npp: {} hits for {:?} in {}",
                    hits.len(),
                    pattern,
                    directory.display()
                );
                self.results.set_hits(hits);
            }
            Err(err) => eprintln!("npp: bad pattern: {err}"),
        }
        self.window.request_redraw();
    }

    /// Opens the file a result points at and puts the caret on the line.
    fn open_hit(&mut self, index: usize) {
        let Some(hit) = self.results.hits().get(index).cloned() else {
            return;
        };
        match hit.tab {
            // A hit from "Find All in Current Document" came out of a buffer,
            // which may have no path at all: it names its tab instead.
            Some(tab) if tab < self.tabs.len() => self.switch_tab(tab),
            _ => self.open_path(&hit.path),
        }
        self.tab_mut().editor.goto_position(hit.line, hit.column);
        self.tab_mut().editor.ensure_caret_visible();
        self.window.request_redraw();
    }

    /// The directory the workspace panels start from.
    ///
    /// The active document's directory, because that is the tree the user is
    /// already thinking in. A document that has never been saved has no
    /// directory, so the process's working directory stands in -- which for a
    /// GUI launched from a desktop file is the home directory, a reasonable
    /// place to start browsing.
    fn workspace_root(&self) -> PathBuf {
        self.tab()
            .editor
            .doc
            .path()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("/"))
    }

    /// Shows or hides the folder-as-workspace panel.
    fn toggle_folder_workspace(&mut self) {
        if self.folder.is_open() && self.workspace.is_none() {
            self.folder.close();
        } else {
            let root = self.workspace_root();
            let tree = Tree::scan(&[root]);
            self.folder.show(tree, "Folder as Workspace");
            self.workspace = None;
        }
        self.resize_view();
        self.window.request_redraw();
    }

    /// Opens project panel `index`, creating the slot if it is empty.
    ///
    /// A project that has never been used gets the active document's directory
    /// as its first root. An empty panel would be a dead end the user has to
    /// escape from with a menu item they have not found yet; a panel showing
    /// where they already are is a starting point.
    fn open_project_panel(&mut self, index: usize) {
        while self.projects.projects.len() <= index {
            let next = self.projects.projects.len() + 1;
            if self.projects.add(format!("Project {next}")).is_none() {
                break;
            }
        }
        let root = self.workspace_root();
        let mut created = false;
        if let Some(project) = self.projects.projects.get_mut(index) {
            if project.is_empty() {
                project.add_root(root);
                created = true;
            }
        }
        if created {
            self.save_projects();
        }
        self.projects.set_active(index);
        let roots = self.projects.active_roots().to_vec();
        let name = self
            .projects
            .active_project()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| format!("Project {}", index + 1));
        self.folder.show(Tree::scan(&roots), name);
        self.workspace = Some(index);
        self.resize_view();
        self.window.request_redraw();
    }

    /// Shows project panel `index`, or hides it when it is already showing.
    fn toggle_project_panel(&mut self, index: usize) {
        if self.folder.is_open() && self.workspace == Some(index) {
            self.folder.close();
            self.workspace = None;
            self.resize_view();
            self.window.request_redraw();
            return;
        }
        self.open_project_panel(index);
    }

    /// Re-reads the filesystem behind the workspace panel.
    fn refresh_workspace(&mut self) {
        if !self.folder.is_open() {
            return;
        }
        self.folder.tree_mut().rescan();
        self.window.request_redraw();
    }

    /// Adds the active document's directory to the active project.
    fn add_project_root(&mut self) {
        let root = self.workspace_root();
        if self.projects.projects.is_empty() {
            self.projects.add("Project 1");
        }
        let index = self.projects.active();
        let added = match self.projects.projects.get_mut(index) {
            Some(project) => project.add_root(root),
            None => false,
        };
        if !added {
            self.status
                .set_message(Some("Already in the project".to_string()));
            self.window.request_redraw();
            return;
        }
        self.save_projects();
        self.open_project_panel(index);
    }

    /// Searches every root of every project.
    fn find_in_projects(&mut self, pattern: &str) {
        let roots = self.projects.all_roots();
        if roots.is_empty() {
            self.status
                .set_message(Some("No project folders to search".to_string()));
            self.window.request_redraw();
            return;
        }
        let search = FindInFiles::new(pattern, &roots[0]);
        match search.run_roots(&roots) {
            Ok(hits) => {
                println!(
                    "npp: {} hits for {:?} in {} project folders",
                    hits.len(),
                    pattern,
                    roots.len()
                );
                self.results.set_hits(hits);
            }
            Err(err) => eprintln!("npp: bad pattern: {err}"),
        }
        self.window.request_redraw();
    }

    /// Reads the project file, if there is one.
    fn load_projects(&mut self) {
        let Some(path) = nn_app::paths::config_file("projects.conf") else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        self.projects = Projects::parse(&text);
    }

    /// Reads the patterns and replacements used before.
    ///
    /// Kept in its own file rather than in the session: the session is about
    /// which files are open, and a search history that vanished because the
    /// files did would be a history of the wrong thing.
    fn load_find_history(&mut self) {
        let Some(path) = nn_app::paths::config_file("find_history.txt") else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        let mut patterns = Vec::new();
        let mut replacements = Vec::new();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("find ") {
                patterns.push(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("replace ") {
                replacements.push(rest.to_string());
            }
        }
        self.find.set_history(patterns, replacements);
    }

    /// Writes the search history, so the next launch remembers it.
    ///
    /// Both fields are remembered first: the bar only files its text away when
    /// a search runs or the bar closes, and quitting with a half-typed pattern
    /// is neither.
    fn save_find_history(&mut self) {
        self.find.remember();
        let Some(path) = nn_app::paths::config_file("find_history.txt") else {
            return;
        };
        let mut text = String::new();
        for pattern in &self.find.history {
            text.push_str("find ");
            text.push_str(pattern);
            text.push('\n');
        }
        for replacement in &self.find.replacement_history {
            text.push_str("replace ");
            text.push_str(replacement);
            text.push('\n');
        }
        nn_app::paths::write(&path, &text);
    }

    /// Writes the project file.
    ///
    /// Written on every change rather than on exit: a project list is a
    /// deliberate arrangement, and losing it because the process was killed
    /// would be a surprise.
    fn save_projects(&mut self) {
        let Some(path) = nn_app::paths::config_file("projects.conf") else {
            return;
        };
        nn_app::paths::write(&path, &self.projects.render());
    }

    /// Switches the active tab's language and re-highlights it.
    fn set_language(&mut self, language: Language) {
        // Cloned because `tab_mut` borrows the state the set lives in. A
        // language set is a few hundred words and this runs on a menu click.
        let langs = self.langs.clone();
        self.tab_mut().set_language(&langs, language);
        self.sync_palette();
        self.window.request_redraw();
    }

    /// Points each view's palette at the language of the document it shows.
    ///
    /// The theme colours the chrome and the nine built-in styles; a
    /// user-defined language brings colours for the keyword and delimiter
    /// slots, and those are per document. Two panes can show two languages, so
    /// this walks both rather than assuming the focused one speaks for the
    /// window.
    fn sync_palette(&mut self) {
        let theme = self.view.theme();
        let focused = match self.tab().language {
            Language::Udl(index) => match self.langs.get(index) {
                Some(udl) => Palette::with_udl(theme, udl),
                None => Palette::new(theme),
            },
            Language::Plain | Language::BuiltIn | Language::Spec(_) => Palette::new(theme),
        };
        self.view.set_palette(focused);

        let langs = &self.langs;
        if let Some(pane) = &mut self.other {
            let language = pane
                .tabs
                .get(pane.active)
                .map(|tab| tab.language)
                .unwrap_or(Language::BuiltIn);
            let palette = match language {
                Language::Udl(index) => match langs.get(index) {
                    Some(udl) => Palette::with_udl(theme, udl),
                    None => Palette::new(theme),
                },
                Language::Plain | Language::BuiltIn | Language::Spec(_) => Palette::new(theme),
            };
            pane.view.set_palette(palette);
        }
    }

    /// Opens the popup context menu's layout file in a tab.
    ///
    /// The file is written from the menu that is showing when it does not exist,
    /// so the first edit starts from what the user already sees. It is re-read
    /// when the tab is saved, which is what makes the edit take effect without a
    /// restart.
    fn edit_context_menu(&mut self) {
        let Some(path) = nn_app::paths::config_file("contextMenu.xml") else {
            return;
        };
        if !path.exists() {
            let entries = self.editor_menu_entries();
            nn_app::paths::write(&path, &context::render_layout(&entries));
        }
        self.open_path(&path);
    }

    /// Re-reads the context menu layout after its file is saved.
    ///
    /// Called from the save path rather than from a watcher: the file is only
    /// ever written by this editor, and a save is the moment it changes.
    fn reload_context_layout(&mut self) {
        let Some(path) = nn_app::paths::config_file("contextMenu.xml") else {
            return;
        };
        self.context_layout = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| context::parse_layout(&text));
    }

    /// Opens the user-defined language file in a tab.
    ///
    /// Notepad++ shows a dialog with a form per language. This shows the file
    /// the dialog would write, which is the same data in the format Notepad++
    /// itself reads: a language defined here can be taken to a Notepad++
    /// installation and one defined there can be brought back. A form would be
    /// a second way to say the same thing, and the one that cannot express
    /// everything the file can.
    ///
    /// The file is created empty-but-valid when it does not exist, so the first
    /// use of the command does not fail on a fresh installation.
    fn define_language(&mut self) {
        let Some(path) = nn_app::paths::config_file("userDefineLang.xml") else {
            return;
        };
        if !path.exists() {
            nn_app::paths::write(&path, &UdlSet::new().render());
        }
        self.open_path(&path);
    }

    /// Opens the directory the user-defined languages live in.
    ///
    /// The file manager rather than a built-in browser: the point of the command
    /// is to drop a file in, and the file manager is where a user does that.
    fn open_udl_folder(&mut self) {
        let Some(dir) = nn_app::paths::config_dir() else {
            return;
        };
        if let Err(err) = nn_app::desktop::open_path(&dir) {
            eprintln!("npp: cannot open {}: {err}", dir.display());
        }
    }

    /// Rebuilds the Language menu from the current set.
    fn sync_language_menu(&mut self) {
        let names: Vec<String> = self.langs.names().iter().map(|n| n.to_string()).collect();
        self.menu.set_languages(&names);
    }

    /// Rewrites the Plugins menu from the plugins the editor has now.
    ///
    /// The inventory crosses in its own order, because a submenu's position in
    /// that list is the index `run_plugin` is handed back. Everything in it is
    /// read-only data -- names, commands, a flag -- so the menu never holds a
    /// module, and a plugin that traps while its own submenu is open cannot
    /// leave a row pointing at something that is not there.
    fn sync_plugin_menu(&mut self) {
        let rows: Vec<PluginMenu> = self
            .plugins
            .inventory()
            .into_iter()
            .map(|plugin| PluginMenu {
                name: plugin.name,
                enabled: plugin.enabled,
                commands: plugin.commands,
            })
            .collect();
        self.menu.set_plugins(&rows);
    }

    /// Opens the go-to-line prompt.
    fn goto_line_prompt(&mut self) {
        self.prompt.open(PromptKind::GotoLine, "");
        self.window.request_redraw();
    }

    /// Starts or stops recording a macro.
    fn toggle_recording(&mut self) {
        if self.recorder.is_recording() {
            self.recorder.stop();
            self.status.set_recording(None);
            println!("npp: macro recorded, {} steps", self.recorder.len());
        } else {
            self.recorder.start();
            self.status.set_recording(Some(0));
        }
        self.window.request_redraw();
    }

    /// Replays the recorded macro against the active tab.
    fn play_macro(&mut self) {
        if self.recorder.is_empty() {
            eprintln!("npp: no macro recorded");
            return;
        }
        let steps = self.recorder.len();
        let tab = &mut self.tabs[self.active];
        let before = tab.editor.caret_line();
        self.recorder.play(&mut tab.editor);
        tab.highlighter.highlight_from(&mut tab.editor.doc, before);
        tab.folder.apply_from(&mut tab.editor.doc, before);
        tab.editor.ensure_caret_visible();
        self.update_title();
        self.window.request_redraw();
        println!("npp: macro played, {steps} steps");
    }

    /// Grows a column block by one motion step.
    ///
    /// The anchor is captured on the first step and kept until something else
    /// moves the caret, so the block follows the caret rather than the mouse.
    fn extend_block(&mut self, motion: Motion) {
        let anchor = match self.block_anchor {
            Some(anchor) => anchor,
            None => {
                let anchor = self.tab().editor.caret_point();
                self.block_anchor = Some(anchor);
                anchor
            }
        };
        self.after_edit(|ed| ed.move_caret(motion, false));
        let caret = self.tab().editor.caret_point();
        self.tab_mut().editor.select_block(anchor, caret);
        self.window.request_redraw();
    }

    /// Adds or removes a bookmark on the caret's line.
    fn toggle_bookmark(&mut self) {
        let tab = &mut self.tabs[self.active];
        let line = tab.editor.caret_line();
        tab.editor.doc.toggle_bookmark(line);
        self.window.request_redraw();
    }

    /// Jumps to the next or previous bookmark.
    fn step_bookmark(&mut self, forward: bool) {
        if !self.tab_mut().editor.goto_bookmark(forward) {
            eprintln!("npp: no bookmarks");
        }
        self.window.request_redraw();
    }

    /// Shows or hides the minimap and reflows the text column around it.
    fn toggle_minimap(&mut self) {
        self.view.minimap_mut().toggle();
        let width = self.gpu.config.width as f32;
        let height = self.gpu.config.height as f32;
        self.view
            .resize(&mut self.tabs[self.active].editor, width, height);
        self.window.request_redraw();
    }

    /// Turns word wrap on or off and reflows the text column.
    fn toggle_wrap(&mut self) {
        self.view.set_wrap(!self.view.wrap());
        let width = self.gpu.config.width as f32;
        let height = self.gpu.config.height as f32;
        self.view
            .resize(&mut self.tabs[self.active].editor, width, height);
        self.tabs[self.active].editor.ensure_caret_visible();
        self.window.request_redraw();
    }

    /// Asks the user for a file, by whichever dialog this installation uses.
    ///
    /// Both dialogs take several files: the open dialog names a list, and every
    /// path in it becomes a tab. Only Save As is single, because one buffer is
    /// one file.
    ///
    /// The portal is tried first. It is the only dialog that can reach the
    /// user's bookmarks and network shares, and under Flatpak or snap it is the
    /// only one that can reach any file at all -- the application has no
    /// permission to read a path it was not handed.
    ///
    /// The availability check runs here rather than on the dialog's thread
    /// because its answer decides whether a dialog appears at all: a fallback
    /// that arrives a frame later would look like a stutter. It is one round
    /// trip on a local socket, the same order of cost as the directory read the
    /// built-in browser does.
    fn choose_file(&mut self, kind: nn_app::portal::Kind) {
        use nn_app::portal;
        if self.settings.file_dialog == FileDialog::Builtin || !portal::available() {
            self.builtin_chooser(kind);
            return;
        }
        if self.portal_pending {
            return;
        }
        self.portal_pending = true;
        let proxy = self.proxy.clone();
        let parent = portal::parent_id(&self.window);
        let title = match kind {
            portal::Kind::Open => "Open",
            portal::Kind::Save => "Save As",
        };
        let filters = portal_filters();
        // Only a save dialog has a name to suggest, and only a document that
        // already has a path has one worth suggesting.
        let current = match kind {
            portal::Kind::Open => None,
            portal::Kind::Save => Some(
                self.tabs[self.active]
                    .editor
                    .doc
                    .path()
                    .and_then(|path| path.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "untitled.txt".to_string()),
            ),
        };
        std::thread::spawn(move || {
            let outcome = portal::choose(kind, title, &filters, current.as_deref(), &parent);
            // A send failure means the window is gone, and there is nothing left
            // to tell.
            let _ = proxy.send_event(UserEvent::Portal { kind, outcome });
        });
        self.window.request_redraw();
    }

    /// The dialog this application draws itself.
    ///
    /// Two shapes, because the two commands have always had two: Open browses,
    /// Save As types a path. Keeping them apart is what makes the fallback a
    /// drop-in -- neither command changes behaviour when the portal is missing.
    fn builtin_chooser(&mut self, kind: nn_app::portal::Kind) {
        match kind {
            nn_app::portal::Kind::Open => self.open_browser(),
            nn_app::portal::Kind::Save => self.save_as_prompt(),
        }
    }

    /// Applies a finished file dialog.
    fn portal_finished(
        &mut self,
        kind: nn_app::portal::Kind,
        outcome: nn_app::portal::Outcome,
    ) {
        use nn_app::portal::Outcome;
        self.portal_pending = false;
        match outcome {
            Outcome::Chosen(paths) => match kind {
                nn_app::portal::Kind::Open => self.open_files(&paths),
                // A save dialog returns one path. More than one would mean the
                // portal misunderstood the request, and writing the same buffer
                // to several files is not something to guess at.
                nn_app::portal::Kind::Save => {
                    if let Some(path) = paths.first() {
                        self.save_as_path(path);
                    }
                }
            },
            // A cancellation is the user's answer, not a failure, and says
            // nothing that needs reporting.
            Outcome::Cancelled => {}
            Outcome::Failed(err) => {
                // The portal was there a moment ago and is not now, or refused.
                // Falling back means the command still works; the message says
                // why the dialog looks different.
                self.status
                    .set_message(Some(format!("System dialog unavailable ({err})")));
                self.builtin_chooser(kind);
            }
        }
        self.window.request_redraw();
    }

    /// Opens the file browser on the active document's directory.
    ///
    /// Starting where the current file lives is the guess that is right most
    /// often; with no file open it falls back to wherever the browser was last.
    fn open_browser(&mut self) {
        let start = self.tabs[self.active]
            .editor
            .doc
            .path()
            .and_then(|path| path.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| self.browser.dir().to_path_buf());
        self.browser.open(&start);
        self.window.request_redraw();
    }

    /// Opens the path prompt for writing the active tab.
    fn save_as_prompt(&mut self) {
        let current = self.tabs[self.active]
            .editor
            .doc
            .path()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        self.prompt.open(PromptKind::SaveAs, &current);
        self.window.request_redraw();
    }

    /// Writes the active buffer to `path` and adopts it as the document's own.
    ///
    /// A Save As over an existing file replaces it, so it backs up the file it
    /// is about to replace rather than the buffer's old path, which is not the
    /// one being overwritten.
    fn save_as_path(&mut self, path: &std::path::Path) {
        backup_before_write(path, &self.settings);
        let doc = &mut self.tabs[self.active].editor.doc;
        match doc.save_as(path) {
            Ok(()) => {
                self.session.push_recent(path);
                self.sync_recent();
            }
            Err(err) => eprintln!("npp: cannot save {}: {err}", path.display()),
        }
        self.update_title();
    }

    /// Opens the prompt for where to write a `shortcuts.xml`.
    fn export_prompt(&mut self) {
        let default = nn_app::paths::config_file("shortcuts.xml")
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        self.prompt.open(PromptKind::ExportShortcuts, &default);
        self.window.request_redraw();
    }

    /// Writes the keymap as a `shortcuts.xml` Notepad++ can read.
    ///
    /// The export is partial for the same reason the import is: Notepad++ has
    /// no number for the commands this editor added, so those bindings cannot
    /// be named in its file. The summary says how many were left out rather
    /// than writing a plausible number that would come back as something else.
    fn export_shortcuts(&mut self, path: &str) {
        if path.is_empty() {
            return;
        }
        let keymap = self.settings.keymap.clone();
        match nppimport::write_shortcuts(&keymap, Path::new(path)) {
            Ok(export) => self.status.set_message(Some(export.summary())),
            Err(err) => self
                .status
                .set_message(Some(format!("Cannot write {path}: {err}"))),
        }
        self.window.request_redraw();
    }

    /// Opens the prompt for a Notepad++ configuration directory.
    fn import_prompt(&mut self) {
        self.prompt
            .open(PromptKind::ImportNotepad, &nppimport::default_dir());
        self.window.request_redraw();
    }

    /// Reads a Notepad++ configuration and applies what this editor can honour.
    ///
    /// The import is partial by design: Notepad++ has commands and settings
    /// with no counterpart here, and the summary says how many were left out
    /// rather than pretending the copy was exact.
    fn import_notepad(&mut self, path: &str) {
        if path.is_empty() {
            return;
        }
        let import = Import::read(Path::new(path));
        let mut settings = self.settings.clone();
        if import.apply(&mut settings) > 0 {
            self.prefs.set_settings(settings);
            self.apply_settings();
        }
        if !import.langs.is_empty() {
            // Merged rather than replaced: a user who has defined a language
            // here and imports a Notepad++ directory that does not have it
            // should keep it. A language of the same name is replaced, which is
            // what makes re-importing an edited language update it.
            for udl in import.langs.langs() {
                self.langs.add(udl.clone());
            }
            save_langs(&self.langs);
            self.sync_language_menu();
            // A tab already showing a language that was just redefined has to
            // re-lex, or it keeps the old colours until it is reopened.
            let langs = self.langs.clone();
            for tab in self.all_tabs() {
                if let Language::Udl(index) = tab.language {
                    tab.set_language(&langs, Language::BuiltIn);
                    tab.set_language(&langs, Language::Udl(index));
                }
            }
            self.sync_palette();
        }
        self.status.set_message(Some(import.summary()));
        self.window.request_redraw();
    }

    /// Runs the prompt's command with the path it holds.
    fn submit_prompt(&mut self) {
        let kind = self.prompt.kind();
        let typed = self.prompt.text().trim().to_string();
        self.prompt.close();
        if kind == PromptKind::GotoLine {
            if let Ok(line) = typed.parse::<usize>() {
                self.tab_mut().editor.goto_line(line.saturating_sub(1));
            }
            self.window.request_redraw();
            return;
        }
        if kind == PromptKind::SplitLines {
            self.line_op(|text| lines::split(text, &typed));
            return;
        }
        if kind == PromptKind::FindInFiles {
            self.find_in_files(&typed);
            return;
        }
        if kind == PromptKind::FindInProjects {
            self.find_in_projects(&typed);
            return;
        }
        if kind == PromptKind::ImportNotepad {
            self.import_notepad(&typed);
            return;
        }
        if kind == PromptKind::ExportShortcuts {
            self.export_shortcuts(&typed);
            return;
        }
        if kind == PromptKind::ColumnText {
            self.column_insert(&typed);
            return;
        }
        if kind == PromptKind::Run {
            self.run_command(&typed);
            return;
        }
        // The two text prompts answer with a pattern rather than a path, so they
        // are handled before the path is built.
        if kind == PromptKind::DateTimeFormat {
            if !typed.is_empty() {
                self.settings.datetime_format = typed.clone();
                self.settings.save();
                self.insert_datetime(&typed);
            }
            return;
        }
        if kind == PromptKind::SearchEngine {
            if !typed.is_empty() {
                self.settings.search_engine = typed.clone();
                self.settings.save();
            }
            return;
        }
        if kind == PromptKind::HashFile {
            if let Some(hash) = self.pending_hash.take() {
                if !typed.is_empty() {
                    self.hash_file(hash, Path::new(&typed));
                }
            }
            return;
        }
        if kind == PromptKind::MacroRepeat {
            if let Ok(times) = typed.trim().parse::<usize>() {
                self.run_macro_multiple(times);
            }
            return;
        }
        if kind == PromptKind::FindChar {
            // Decimal or `0x` hex, because a code point is written both ways and
            // refusing one of them would be a puzzle rather than a rule.
            let text = typed.trim();
            let code = if let Some(hex) = text.strip_prefix("0x").or(text.strip_prefix("0X")) {
                u32::from_str_radix(hex, 16).ok()
            } else {
                text.parse::<u32>().ok()
            };
            match code {
                Some(code) => self.find_char_in_range(code),
                None => self
                    .status
                    .set_message(Some(format!("{text:?} is not a character code"))),
            }
            return;
        }
        if kind == PromptKind::ValidateShortcuts {
            if !typed.is_empty() {
                self.validate_shortcuts(Path::new(&typed));
            }
            return;
        }
        let path = PathBuf::from(&typed);
        if path.as_os_str().is_empty() {
            return;
        }
        match kind {
            PromptKind::Open => self.open_path(&path),
            PromptKind::SaveAs => self.save_as_path(&path),
            PromptKind::SaveCopyAs => self.save_copy_as(&path),
            PromptKind::Rename => self.rename_to(&path),
            PromptKind::LoadSession => self.load_session(&path),
            PromptKind::SaveSession => self.write_session_file(&path),
            PromptKind::OpenFolderAsWorkspace => self.adopt_workspace(&path),
            PromptKind::ImportTheme => self.import_theme(&path),
            PromptKind::ImportPlugin => self.import_plugin(&path),
            PromptKind::GotoLine
            | PromptKind::SplitLines
            | PromptKind::FindInFiles
            | PromptKind::FindInProjects
            | PromptKind::ImportNotepad
            | PromptKind::ExportShortcuts
            | PromptKind::ColumnText
            | PromptKind::DateTimeFormat
            | PromptKind::SearchEngine
            | PromptKind::HashFile
            | PromptKind::ValidateShortcuts
            | PromptKind::MacroRepeat
            | PromptKind::FindChar
            | PromptKind::Run => {}
        }
        self.window.request_redraw();
    }

    /// Writes the active document, if it has a path.
    ///
    /// A document with no path is left alone rather than prompting: this is
    /// called from the Run command, and a modal Save As in the middle of running
    /// a command is a surprise, not a service.
    fn save_active(&mut self) {
        if self.tab().editor.doc.path().is_none() {
            return;
        }
        let settings = self.settings.clone();
        match write_tab(self.tab_mut(), &settings) {
            Ok(()) => {
                if let Some(path) = self.tab().editor.doc.path().map(|p| p.to_path_buf()) {
                    self.session.push_recent(&path);
                    self.sync_recent();
                }
                // The clone is holding the text that was just written, so it is
                // no longer unsaved. A save that cleared one pencil and left the
                // other would say the file differs from itself.
                let group = self.tab().group;
                self.clear_group_dirty(group);
            }
            Err(err) => eprintln!("npp: save failed: {err}"),
        }
        // The context menu's layout is one of the files this editor opens for
        // editing, so a save is the moment it may have changed.
        self.reload_context_layout();
        self.update_title();
    }

    /// Opens the Run prompt, pre-filled with the last command used.
    ///
    /// Pre-filled rather than empty: a command line is usually re-run with one
    /// argument changed, and retyping the whole thing to change one word is the
    /// kind of friction that stops people using the feature.
    fn open_run_prompt(&mut self) {
        let initial = self
            .settings
            .runs
            .commands()
            .last()
            .map(|command| command.command.clone())
            .unwrap_or_default();
        self.prompt.open(PromptKind::Run, &initial);
        self.window.request_redraw();
    }

    /// The facts a command line can refer to, read from the active tab.
    fn run_context(&self) -> RunContext {
        let editor = &self.tabs[self.active].editor;
        let doc = &editor.doc;
        let caret = editor.caret();
        let line = doc.char_to_line(caret);
        let column = caret - doc.line_to_char(line);
        let word = doc.word_at(caret).unwrap_or_default();
        let selection = editor.selected_text();
        let directory = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_default();
        RunContext {
            path: doc.path().map(Path::to_path_buf),
            line: line + 1,
            column: column + 1,
            word,
            selection,
            directory,
        }
    }

    /// Runs a command typed into the prompt, and remembers it.
    fn run_command(&mut self, command: &str) {
        if command.trim().is_empty() {
            return;
        }
        let mut settings = self.settings.clone();
        settings.runs.add(RunCommand::new(command));
        self.prefs.set_settings(settings);
        self.apply_settings();
        self.run_saved(&RunCommand::new(command));
    }

    /// Runs a saved command, saving the document first when it asks to.
    fn run_saved(&mut self, command: &RunCommand) {
        if command.save_first {
            self.save_active();
        }
        let expanded = nn_core::run::expand(&command.command, &self.run_context());
        if let Some(missing) = nn_core::run::missing_program(&expanded) {
            self.status
                .set_message(Some(format!("Run: {missing} does not exist")));
            self.window.request_redraw();
            return;
        }
        // Run in the file's directory, so a relative path in the command means
        // what the user sees in the tab rather than where the editor was started.
        let directory = self.tabs[self.active]
            .editor
            .doc
            .path()
            .and_then(|path| path.parent())
            .map(Path::to_path_buf);
        match nn_core::run::spawn(&expanded, directory.as_deref()) {
            Ok(_) => self.status.set_message(Some(format!("Run: {expanded}"))),
            Err(err) => self
                .status
                .set_message(Some(format!("Run failed: {err}"))),
        }
        self.window.request_redraw();
    }

    /// Steps the font size by `steps` notches, or resets it when `steps` is 0.
    fn zoom(&mut self, steps: i32) {
        let font_system = self.renderer.font_system();
        self.view.zoom(font_system, steps);
        // "Synchronize Across Views" is about the two panes showing the same
        // thing at the same size, so the other one takes the same step rather
        // than being set to the same absolute size: the two may have started
        // apart, and a step keeps whatever relationship they had.
        if self.settings.zoom_sync {
            if let Some(other) = &mut self.other {
                other.view.zoom(font_system, steps);
            }
        }
        self.resize_view();
    }

    /// Copies the focused pane's scroll to the other one, when the user asked
    /// for the two to move together.
    ///
    /// Done once per frame rather than at each of the two dozen places that
    /// scroll: the panes are drawn from the state this writes, so a frame is the
    /// natural granularity, and a scroll that happened for a reason this file has
    /// not thought of yet is still followed.
    fn sync_scroll(&mut self) {
        if !(self.settings.sync_scroll_v || self.settings.sync_scroll_h) {
            return;
        }
        let Some(other) = &mut self.other else {
            return;
        };
        let source = &self.tabs[self.active].editor.viewport;
        let target = &mut other.tabs[other.active].editor.viewport;
        if self.settings.sync_scroll_v {
            target.first_line = source.first_line;
            target.top_offset = source.top_offset;
        }
        if self.settings.sync_scroll_h {
            target.scroll_x = source.scroll_x;
        }
    }

    /// Applies a fold command to the active tab.
    fn fold(&mut self, command: FoldCommand) {
        let tab = &mut self.tabs[self.active];
        let caret = tab.editor.caret();
        let doc = &mut tab.editor.doc;
        match command {
            FoldCommand::Toggle => {
                let line = doc.char_to_line(caret);
                doc.toggle_fold(line);
            }
            FoldCommand::All => doc.fold_all(),
            FoldCommand::None => doc.unfold_all(),
        }
        tab.editor.ensure_caret_visible();
        self.window.request_redraw();
    }

    /// Pins the window above every other one, or lets it fall back.
    fn toggle_always_on_top(&mut self) {
        self.always_on_top = !self.always_on_top;
        self.window.set_window_level(if self.always_on_top {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        });
        self.status.set_message(Some(
            if self.always_on_top {
                "Always on top"
            } else {
                "Normal stacking"
            }
            .to_string(),
        ));
    }

    /// Notepad++'s Post-It: a window that stays on top and shows nothing but the
    /// text.
    ///
    /// The two halves are the whole point of the mode — a note you keep beside
    /// your work has to stay visible and has to be small — so the row sets both
    /// rather than being a third independent toggle. Pressing it again undoes
    /// both, which is what makes it a mode rather than two switches.
    fn post_it(&mut self) {
        let on = !(self.always_on_top && self.chrome.is_distraction_free());
        self.always_on_top = on;
        self.window.set_window_level(if on {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        });
        self.chrome
            .set_mode(if on { Mode::DistractionFree } else { Mode::Normal });
        self.status.set_message(Some(
            if on {
                "Post-It: always on top, no chrome"
            } else {
                "Normal mode"
            }
            .to_string(),
        ));
        self.window.request_redraw();
    }

    /// Opens the active document in a browser.
    ///
    /// The four rows differ only in which program they try first. A document
    /// that has never been saved has no URL to hand over, so the row says so
    /// rather than opening an empty page.
    fn view_in_browser(&mut self, kind: BrowserKind) {
        let Some(path) = self.tabs[self.active]
            .editor
            .doc
            .path()
            .map(Path::to_path_buf)
        else {
            self.status
                .set_message(Some("Save the file before viewing it in a browser".into()));
            return;
        };
        let url = nn_app::desktop::file_url(&path);
        match nn_app::desktop::open_url_with(kind.command(), &url) {
            Ok(()) => self
                .status
                .set_message(Some(format!("{}: {url}", kind.label()))),
            Err(error) => self
                .status
                .set_message(Some(format!("Could not open a browser: {error}"))),
        }
    }

    /// Turns on every mark the Show Symbol submenu has.
    ///
    /// "Show All Characters" is a command rather than a fifth independent flag:
    /// Notepad++ treats it as the union of the four above it, and a fifth flag
    /// would let the menu reach a state where "all" is on and one of the others
    /// is off.
    fn show_all_chars(&mut self) {
        let on = !(self.view.show_whitespace()
            && self.view.show_eol()
            && self.view.show_indent_guides()
            && self.view.show_non_printing()
            && self.view.show_control_chars());
        self.view.set_show_whitespace(on);
        self.view.set_show_eol(on);
        self.view.set_show_indent_guides(on);
        self.view.set_show_non_printing(on);
        self.view.set_show_control_chars(on);
        self.status.set_message(Some(
            if on {
                "Showing all characters"
            } else {
                "Hiding the character marks"
            }
            .to_string(),
        ));
    }

    /// Hands the active document to a second process.
    ///
    /// `move_doc` closes the tab here once the other process has it, which is
    /// the difference between "Move to New Instance" and "Open in New Instance".
    /// The tab is only closed when the spawn succeeded: a failed launch that
    /// also closed the document would lose it.
    fn new_instance(&mut self, move_doc: bool) {
        let Ok(exe) = std::env::current_exe() else {
            self.status
                .set_message(Some("Could not find this program's own path".into()));
            return;
        };
        let mut command = std::process::Command::new(exe);
        command.arg("--new-instance");
        let path = self.tabs[self.active]
            .editor
            .doc
            .path()
            .map(Path::to_path_buf);
        if let Some(path) = &path {
            command.arg(path);
        }
        match command.spawn() {
            Ok(_) => {
                self.status
                    .set_message(Some("Started a second instance".into()));
                if move_doc && path.is_some() {
                    self.close_tab();
                }
            }
            Err(error) => self
                .status
                .set_message(Some(format!("Could not start a second instance: {error}"))),
        }
    }

    /// Moves the active tab to `index`, keeping it active.
    fn move_tab_to(&mut self, index: usize) {
        let index = index.min(self.tabs.len().saturating_sub(1));
        if index == self.active {
            return;
        }
        let tab = self.tabs.remove(self.active);
        self.tabs.insert(index, tab);
        self.active = index;
        self.after_tab_change();
    }

    /// Swaps the active tab with its neighbour.
    fn move_tab_step(&mut self, forward: bool) {
        let target = if forward {
            self.active + 1
        } else {
            self.active.wrapping_sub(1)
        };
        if target < self.tabs.len() {
            self.move_tab_to(target);
        }
    }

    /// Paints the active tab, or clears its paint.
    fn set_tab_colour(&mut self, index: Option<usize>) {
        // `TabColour::ALL` starts with `None`, so the five paints are the five
        // entries after it. Indexing rather than a match keeps the menu order and
        // the palette order in one place.
        let colour = match index {
            Some(index) => TabColour::ALL[(index + 1).min(TabColour::ALL.len() - 1)],
            None => TabColour::None,
        };
        self.tabs[self.active].colour = colour;
        self.after_tab_change();
        self.status
            .set_message(Some(format!("Tab colour: {}", colour.label())));
    }

    /// Folds the selected lines away.
    ///
    /// Notepad++ hides exactly the lines the selection covers. This build's fold
    /// model is block-based — a fold runs from a header to the end of its block —
    /// so the closest reversible equivalent is to collapse every foldable block
    /// the selection touches. Nothing is deleted, and Unfold All brings it back.
    fn hide_lines(&mut self) {
        let tab = &mut self.tabs[self.active];
        let (first, last) = tab.editor.selected_lines();
        let mut folded = 0usize;
        for line in first..=last {
            if tab.editor.doc.is_foldable(line) && !tab.editor.doc.is_folded(line) {
                tab.editor.doc.fold(line);
                folded += 1;
            }
        }
        tab.editor.ensure_caret_visible();
        self.status.set_message(Some(if folded == 0 {
            "Nothing on these lines can be hidden".to_string()
        } else {
            format!("Hid {folded} block(s)")
        }));
        self.window.request_redraw();
    }

    /// Folds or unfolds every block at the caret line's own depth.
    ///
    /// Notepad++'s "Fold Current Level" is about the level the caret sits at, not
    /// about the caret's own block: it is how you collapse every sibling at once
    /// without counting levels from the top.
    fn fold_current_level(&mut self, fold: bool) {
        let tab = &mut self.tabs[self.active];
        let line = tab.editor.doc.char_to_line(tab.editor.caret());
        let depth = tab.editor.doc.fold_depth(line);
        let count = tab.editor.doc.len_lines();
        let mut changed = 0usize;
        for probe in 0..count {
            if tab.editor.doc.fold_depth(probe) != depth {
                continue;
            }
            if fold {
                if tab.editor.doc.is_foldable(probe) && !tab.editor.doc.is_folded(probe) {
                    tab.editor.doc.fold(probe);
                    changed += 1;
                }
            } else if tab.editor.doc.is_folded(probe) {
                tab.editor.doc.unfold(probe);
                changed += 1;
            }
        }
        tab.editor.ensure_caret_visible();
        self.status.set_message(Some(format!(
            "{} {changed} block(s) at level {depth}",
            if fold { "Folded" } else { "Unfolded" }
        )));
        self.window.request_redraw();
    }

    /// Folds or unfolds to a depth.
    fn fold_to_level(&mut self, level: u16, fold: bool) {
        let tab = &mut self.tabs[self.active];
        if fold {
            tab.editor.doc.fold_to_level(level);
        } else {
            tab.editor.doc.unfold_to_level(level);
        }
        tab.editor.ensure_caret_visible();
        self.status.set_message(Some(format!(
            "{} to level {level}",
            if fold { "Folded" } else { "Unfolded" }
        )));
        self.window.request_redraw();
    }

    /// Reports the document's counts.
    ///
    /// Notepad++ puts this in a dialog. A dialog for four numbers is a window to
    /// open and close, so the counts go to the status bar and the log instead —
    /// the same information, one press, no dismissal.
    fn summary(&mut self) {
        let editor = &self.tabs[self.active].editor;
        let doc = &editor.doc;
        let chars = doc.len_chars();
        let lines = doc.len_lines();
        let words = doc
            .slice(0..chars)
            .split_whitespace()
            .count();
        let selected = editor.selection_len();
        let text = if selected > 0 {
            let selected_words = editor.selected_text().split_whitespace().count();
            format!(
                "{lines} lines, {words} words, {chars} characters; selection: {selected} characters, {selected_words} words"
            )
        } else {
            format!("{lines} lines, {words} words, {chars} characters")
        };
        println!("{}: {text}", self.tabs[self.active].title());
        self.status.set_message(Some(text));
    }

    /// Turns tail -f on or off for the active tab.
    ///
    /// The reload itself is the existing disk-change path; the flag only decides
    /// whether a change is applied without asking. That is the whole difference
    /// between a file you are editing and a file you are watching.
    fn toggle_monitoring(&mut self) {
        let tab = &mut self.tabs[self.active];
        tab.monitoring = !tab.monitoring;
        let on = tab.monitoring;
        let title = tab.title();
        self.status.set_message(Some(if on {
            format!("Monitoring {title} — changes are reloaded as they happen")
        } else {
            format!("Stopped monitoring {title}")
        }));
    }

    /// Scrolls the active editor so the minimap row under `y` sits at the top.
    /// `row_at` clamps, so dragging past either end parks the view at that end.
    fn minimap_scroll_to(&mut self, y: f64) {
        let total = self.tabs[self.active].editor.doc.visible_line_count();
        let (top, height) = self.view.minimap_rect(total);
        let row = self.view.minimap().row_at(y as f32, top, height, total);
        let editor = &mut self.tabs[self.active].editor;
        editor.viewport.first_line = row;
        editor.viewport.top_offset = 0.0;
        self.window.request_redraw();
    }

    /// Whether a dialog owns the window.
    ///
    /// A modal covers the toolbar, so nothing behind it can be hovered or
    /// right-clicked. The list is spelled out rather than derived from a flag
    /// because each dialog is opened and closed on its own terms, and a single
    /// "modal" boolean would be one more thing to keep in step with them.
    fn modal_open(&self) -> bool {
        self.browser.is_open()
            || self.about.is_open()
            || self.prefs.is_open()
            || self.print.is_open()
            || self.styles.is_open()
            || self.shortcuts.is_open()
            || self.charset.is_open()
    }

    /// Whether a toolbar label may be shown at all.
    ///
    /// A dropdown covers the toolbar, and a dialog covers the window, so with
    /// either of them up a label would be naming a button the pointer cannot be
    /// touching: the menu row under the pointer is what a click would hit.
    fn tooltips_allowed(&self) -> bool {
        !self.modal_open() && !self.menu.is_open()
    }

    /// Tells the tooltip what the pointer is over.
    ///
    /// The toolbar is the only control with a tooltip. It is asked through
    /// `tip_at` rather than `hover_at` because the two answer different
    /// questions: `hover_at` moves the highlight and is only called once the
    /// pointer is known to be on the bar, while the tooltip has to be told about
    /// every position so that leaving the bar clears the label.
    fn rest_tooltip(&mut self, x: f32, y: f32) {
        let target = if self.toolbar_height() > 0.0 && self.tooltips_allowed() {
            self.toolbar.tip_at(
                x,
                y,
                self.menu_height(),
                self.gpu.config.width as f32,
                self.ui.line_height,
            )
        } else {
            None
        };
        let target = target
            .as_ref()
            .map(|(label, anchor)| (label.as_str(), *anchor));
        if self.tooltip.rest(target, std::time::Instant::now()) {
            self.window.request_redraw();
        }
    }

    /// Hides the tooltip, if one is showing.
    ///
    /// Called when the pointer leaves the window, when a button is pressed and
    /// when a key is typed: in all three cases the user has moved on, and a
    /// label that outlives the gesture is a label in the way. The pending label
    /// is dropped as well as the shown one, so a click does not bring the
    /// tooltip straight back -- it waits for the pointer to move again.
    fn hide_tooltip(&mut self) {
        let was_open = self.tooltip.is_open();
        self.tooltip.close();
        if was_open {
            self.window.request_redraw();
        }
    }

    /// Closes the About box, putting the pointer back if it was on the link.
    ///
    /// The cursor is restored here rather than left to the next motion event,
    /// because a box dismissed with Escape while the pointer sits on the link
    /// would otherwise leave a hand cursor over a text editor.
    fn close_about(&mut self) {
        self.about.close();
        if self.link_hover {
            self.link_hover = false;
            self.window.set_cursor(CursorIcon::Default);
        }
    }

    fn handle_cursor(&mut self, x: f64, y: f64) {
        // The About box's link is the only thing in the window that changes the
        // pointer, and it is asked before the early returns below so that a
        // pointer leaving the link while a drag is in progress still puts the
        // arrow back. Everything else in the window is text, where the default
        // arrow is what the platform gives a window that never asks.
        let over_link = self.about.is_open() && self.about.link_hit(x as f32, y as f32, self.ui.line_height);
        if over_link != self.link_hover {
            self.link_hover = over_link;
            self.window.set_cursor(if over_link {
                CursorIcon::Pointer
            } else {
                CursorIcon::Default
            });
        }

        // Before anything else, because the toolbar's hit test is a pure
        // function of the pointer position. Asking it here means the dwell timer
        // keeps running on the paths that return early -- a drag in progress, an
        // open completion popup -- and a pointer that leaves the bar clears the
        // label without every one of those paths having to remember to.
        self.rest_tooltip(x as f32, y as f32);

        // A press on the floating Search window's title bar drags the window.
        if self.find_dialog.dragging() {
            self.find_dialog.drag_to(x as f32, y as f32);
            self.window.request_redraw();
            return;
        }

        // A press that landed on a tab is followed here rather than by the
        // caret: dragging a tab must not drag out a selection underneath it.
        if let Some((index, press_x, press_y)) = self.tab_press {
            let moved = (x - press_x).abs() + (y - press_y).abs();
            if self.tab_drag.is_none() && moved > TAB_DRAG_SLOP {
                self.tab_drag = Some(index);
                self.window.request_redraw();
            }
            if self.tab_drag.is_some() {
                self.drag_tab_to(x as f32, y as f32);
            }
            return;
        }

        // The character grid is on top of everything, so it is asked first. The
        // hover is only repainted when it moves to a different cell, because a
        // pointer crossing a cell boundary is the only thing that changes the
        // readout.
        if self.charset.is_open() {
            let hovered = self.charset.cell_at(x as f32, y as f32);
            if hovered != self.charset.hover() {
                self.charset.set_hover(hovered);
                self.window.request_redraw();
            }
            return;
        }

        // Dragging the divider resizes the panes and nothing else.
        if self.divider_drag {
            self.drag_divider(x, y);
            return;
        }
        // Dragging on the minimap scrubs the view, like Notepad++.
        if self.minimap_drag {
            self.minimap_scroll_to(y);
            return;
        }
        if self.complete.is_open() {
            self.complete
                .hover(x as f32, y as f32, self.ui.line_height);
        }
        if self.context.is_open() {
            self.context
                .hover(x as f32, y as f32, self.ui.line_height);
            self.window.request_redraw();
        }
        if self.peeker.is_open() {
            self.peeker
                .hover(x as f32, y as f32, self.ui.line_height);
            self.window.request_redraw();
        }
        // A hidden bar cannot be hovered. Without these guards the menu bar's
        // hit test would still claim the top line of text in distraction-free
        // mode, and the toolbar's would claim the line below it.
        if self.menu_height() > 0.0
            && self.menu.hover_at(
                x as f32,
                y as f32,
                self.ui.line_height,
                self.gpu.config.width as f32,
            )
        {
            self.window.request_redraw();
            return;
        }
        let line_height = self.ui.line_height;
        if self.toolbar_height() > 0.0
            && self.toolbar.hover_at(
                x as f32,
                y as f32,
                self.menu_height(),
                self.gpu.config.width as f32,
                line_height,
            )
        {
            self.window.request_redraw();
            return;
        }
        // The strip highlights the tab under the pointer and, on the active tab,
        // whichever of its two buttons the pointer is over. Leaving the strip
        // has to clear the highlight, or the last tab stays lit.
        if self.tab_height() > 0.0 || self.tab_column() > 0.0 {
            if self.tab_bar.hover_at(x as f32, y as f32, line_height) {
                self.window.request_redraw();
                return;
            }
            self.tab_bar.clear_hover();
        }
        if !self.dragging {
            return;
        }
        // A drag is a selection, not a run of clicks. Every motion extends from
        // the anchor the press left behind, so the text between the press and
        // the pointer stays selected; passing the Shift key here instead
        // collapsed the selection on every motion event, which is why dragging
        // never selected anything.
        //
        // Same conversion as the press, so the selection follows the pointer
        // rather than trailing it by the height of the bar.
        let (text_x, text_y) = self.view.text_point(x as f32, y as f32);
        // A drag that began as a double or triple click pulls the selection out
        // by the unit it picked. The anchor keeps the end it started from, so
        // dragging back over it shrinks the same selection rather than starting
        // a second one.
        if let Some((unit, (anchor_start, anchor_end))) = self.unit_drag {
            let pos = self.tab_mut().editor.char_at_point(text_x, text_y);
            let under = match unit {
                ClickUnit::Word => self.tab().editor.word_range_at(pos),
                ClickUnit::Line => Some(self.tab().editor.line_range_at(pos)),
            };
            if let Some(range) = under {
                let (start, end) = if range.start < anchor_start {
                    (range.start, anchor_end)
                } else {
                    (anchor_start, range.end.max(anchor_end))
                };
                self.tab_mut().editor.select_range(start..end);
            }
            self.window.request_redraw();
            return;
        }
        self.tab_mut()
            .editor
            .set_caret_from_point(text_x, text_y, true);
        if let Some(anchor) = self.block_anchor {
            let caret = self.tab().editor.caret_point();
            self.tab_mut().editor.select_block(anchor, caret);
        }
        self.window.request_redraw();
    }
}

/// What the event loop carries besides window events.
///
/// Three sources: accessibility, the thread a portal file dialog blocks on, and
/// the thread that follows the desktop's colour scheme. The single-instance bus
/// uses its own thread and a shared queue, which predates this and does not need
/// a wake-up.
enum UserEvent {
    A11y(accesskit_winit::Event),
    /// The desktop changed its colour scheme.
    Scheme(nn_app::portal::Scheme),
    /// A file dialog finished.
    ///
    /// The kind travels with the outcome because the answer alone does not say
    /// what to do with it: a chosen path is opened for `Open` and written to for
    /// `Save`.
    Portal {
        kind: nn_app::portal::Kind,
        outcome: nn_app::portal::Outcome,
    },
    /// One REST request, waiting for the window to answer it.
    ///
    /// The listener runs on its own thread and the editor state lives on this
    /// one, so the request travels in and the answer travels back: the sender is
    /// the connection's end of that conversation, and it outlives the frame
    /// because the HTTP thread is still waiting on it.
    Rest {
        call: nn_app::api::Call,
        answer: std::sync::mpsc::Sender<serde_json::Value>,
    },
}

impl From<accesskit_winit::Event> for UserEvent {
    fn from(event: accesskit_winit::Event) -> Self {
        UserEvent::A11y(event)
    }
}

/// The winit application.
struct App {
    state: Option<State>,
    args: nn_app::cli::Args,
    /// The installed fonts, being scanned and measured on another thread.
    ///
    /// Started once the process has decided it is going to open a window, and
    /// joined when the renderer is built: the scan and the first shaping are
    /// pure CPU and cost about as much as opening the graphics device, so the
    /// two waits run together rather than one after the other. Measuring the
    /// column width there is what keeps the first shaping off the critical
    /// path.
    fonts: Option<std::thread::JoinHandle<LoadedFonts>>,
    /// Where accessibility events are delivered.
    ///
    /// Held here rather than in `State` because the adapter has to exist before
    /// the window is first shown, which is before `State` is built.
    proxy: EventLoopProxy<UserEvent>,
    /// The bus name this process owns, held here until the window exists.
    ///
    /// `resumed` is where the window is created, and the server has to be
    /// acquired before the event loop starts -- otherwise a second launch in
    /// the gap between `main` and the first frame would find no owner and start
    /// a second editor.
    server: Option<nn_app::single::Server>,
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Neko++")
            .with_inner_size(winit::dpi::LogicalSize::new(1100.0, 750.0))
            // The same kitten the desktop entry and the packages carry, so the
            // taskbar and the launcher agree on what this program is.
            .with_window_icon(nn_app::window_icon())
            // The window class is the desktop's handle on this program: the
            // taskbar icon, the desktop entry's `StartupWMClass` and the window
            // manager's rules all match on it. Left to the toolkit it is the
            // name the binary was invoked by, so a launcher symlink called `nn`
            // would arrive as a second, icon-less application. Pinned here so
            // every name the binary answers to is one window class.
            .with_name("npp", "npp")
            // The accessibility adapter refuses to attach to a window that is
            // already visible, so the window is created hidden and shown once
            // the adapter exists. The gap is one call wide and nothing is drawn
            // in it.
            .with_visible(false);
        // A launcher that opens a file -- a right click in the file manager, a
        // desktop entry, `xdg-open` -- hands the process a startup token in the
        // environment. Attaching it is what tells the window manager the launch
        // was the user's doing: without it the new window is not given the
        // focus until it is clicked, and the launcher keeps its spinner running
        // until the startup notification times out. Both of those are the
        // "slow and no cursor" a right click used to feel like.
        let token = event_loop.read_token_from_env();
        // Unset before anything here can spawn a child: an inherited token makes
        // the child ask for the focus its parent already took.
        winit::platform::startup_notify::reset_activation_token_env();
        let attrs = match token {
            Some(token) => attrs.with_activation_token(token),
            None => attrs,
        };
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        // The GPU is opened here rather than inside `State::new` so a machine
        // with no usable adapter gets a message and an exit code instead of a
        // panic from the middle of a constructor.
        let gpu = match Gpu::new(window.clone(), self.args.software) {
            Ok(gpu) => gpu,
            Err(err) => {
                eprintln!("npp: {err}");
                std::process::exit(1);
            }
        };
        let a11y = accesskit_winit::Adapter::with_event_loop_proxy(
            event_loop,
            &window,
            self.proxy.clone(),
        );
        window.set_visible(true);
        let first = self.args.files.first().cloned();
        let fonts = self.fonts.take();
        let mut state = State::new(window, gpu, a11y, self.proxy.clone(), first, fonts);
        state.server = self.server.take();
        state.apply_args(&self.args);
        state.window.request_redraw();
        self.state = Some(state);
    }

    /// Handles an event from another thread.
    ///
    /// Accessibility is the only source. The three cases are the whole protocol:
    /// a screen reader attaching, a screen reader asking for something to be
    /// done, and a screen reader detaching.
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        let event = match event {
            UserEvent::Portal { kind, outcome } => {
                state.portal_finished(kind, outcome);
                return;
            }
            UserEvent::Scheme(scheme) => {
                state.scheme_changed(scheme);
                return;
            }
            UserEvent::Rest { call, answer } => {
                // The same `call` the bus and MCP reach, answered on the thread
                // that owns the documents, and a repaint because a `PUT` may
                // have replaced the text the frame is holding.
                let value = state.call(call);
                state.window.request_redraw();
                let _ = answer.send(value);
                return;
            }
            UserEvent::A11y(event) => event,
        };
        match event.window_event {
            accesskit_winit::WindowEvent::InitialTreeRequested => {
                // The adapter has no tree yet, so the signature check would
                // suppress the very update that is being asked for.
                state.a11y_signature = None;
                state.sync_a11y();
            }
            accesskit_winit::WindowEvent::ActionRequested(request) => {
                let snapshot = state.a11y_snapshot();
                if let Some(command) = nn_app::a11y::command(request.target_node, &snapshot) {
                    state.run_a11y_command(command);
                }
            }
            accesskit_winit::WindowEvent::AccessibilityDeactivated => {
                // The next activation asks for a fresh tree, and the signature
                // from before the screen reader went away says nothing about
                // whether the document has changed since.
                state.a11y_signature = None;
            }
        }
    }

    /// Wakes for whichever timer is due, and sleeps otherwise.
    ///
    /// Two things want a timer: the open files, which are checked once a second,
    /// and the unsaved buffers, which are snapshotted every few seconds. The loop
    /// sleeps until the earlier of the two.
    ///
    /// `ControlFlow::Wait` when neither is wanted: an untitled clean buffer has
    /// nothing on disk to watch and nothing to save, and an idle window should
    /// stay asleep rather than wake to do nothing.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        // The tree is pushed here rather than from the render path: a screen
        // reader reads when the window is idle, and `about_to_wait` is the one
        // place that runs after every batch of events and before the loop
        // sleeps.
        state.sync_a11y();
        // A second launch queues its request on the bus; the queue is drained
        // here rather than from a thread because opening a document touches the
        // tab strip, the language list and the window title.
        // The server is moved out for the length of the drain because answering a
        // call needs `&mut state` and `server` is a field of it: taking it out is
        // cheaper than splitting the borrow into four helpers, and it goes back
        // before the frame ends.
        if let Some(server) = state.server.take() {
            for pending in server.take_calls() {
                let answer = answer_to(state, &pending.json);
                server.answer(pending.id, answer);
            }
            for request in server.take() {
                state.open_request(&request);
            }
            state.server = Some(server);
        }
        state.poll_disk();
        // A menu opened from the keyboard can leave a label standing over the
        // bar its dropdown now covers; the pointer has not moved, so nothing else
        // would clear it.
        if !state.tooltips_allowed() {
            state.hide_tooltip();
        }
        // The tooltip's delay is served here rather than from a thread: the
        // event loop has just woken for it, and opening a label is a repaint.
        if state.tooltip.tick(std::time::Instant::now()) {
            state.window.request_redraw();
        }
        if state.autosave_in() == Some(std::time::Duration::ZERO) {
            state.snapshot_now();
        }
        // The room's own frame timer, and the idle one that opens it.
        state.tick_playground();
        state.repaint_due();
        match state.next_wake() {
            Some(wait) => event_loop.set_control_flow(ControlFlow::WaitUntil(
                std::time::Instant::now() + wait,
            )),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else {
            return;
        };

        // The adapter sees every event before the editor does: it tracks focus
        // and size itself, and a screen reader that is told about a resize late
        // reads the old geometry.
        {
            let State { a11y, window, .. } = state;
            a11y.process_event(window, &event);
        }

        match event {
            WindowEvent::CloseRequested => {
                // Before the session, so a close with unsaved work leaves a
                // snapshot behind. Closing is not a decision to throw the work
                // away, and the next launch offers it back.
                state.snapshot_now();
                state.save_session();
                state.save_find_history();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                state.resize(size.width, size.height);
                state.window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                // The window has moved to a display of another density, or the
                // desktop's scaling changed under it. The factor comes from the
                // event rather than from the window, because the event is what
                // says it changed; a settings file that names a scale keeps
                // naming it, because that is what insisting means.
                state.rescale(scale_factor);
            }
            // The window is drawn on demand, so nothing is presented between
            // events: a window that comes back to the front, or regains the
            // focus, is showing whatever frame the compositor still holds, and
            // after a driver round-trip that can be an older one -- text
            // without the selection that was on it, or no text at all. One
            // frame on the way in is the fix and costs nothing when the focus
            // did not move.
            // A window on its way to the back does not get the button release
            // that ends a drag: the compositor takes the focus and the release
            // goes with it, or the pointer leaves for the other window first. A
            // drag that outlives its button then acts on the next motion, and
            // after an `Alt+Tab` the next motion is the first thing the pointer
            // does coming back -- a text drag collapses the selection to the
            // pointer, which reads as the selection disappearing, and a tab drag
            // moves a tab nobody is holding. Ending them here keeps the editor's
            // selection, and a click starts a new drag.
            WindowEvent::Focused(false) => {
                state.focus_at = None;
                state.dragging = false;
                state.minimap_drag = false;
                state.unit_drag = None;
                state.divider_drag = false;
                state.tab_press = None;
                state.tab_drag = None;
                state.find_dialog.end_drag();
            }
            WindowEvent::Focused(true) => {
                // Only a focus gain arms the leaked-key guard. An occlusion
                // report says nothing about the keyboard, and a window that is
                // uncovered while someone is typing must not lose a keystroke.
                state.focus_at = Some(std::time::Instant::now());
                state.window.request_redraw();
            }
            WindowEvent::Occluded(false) => {
                state.window.request_redraw();
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                state.modifiers = modifiers.state();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    state.playground.activity(std::time::Instant::now());
                    state.hide_tooltip();
                    state.handle_key(&event);
                }
            }
            WindowEvent::MouseInput {
                state: pressed,
                button,
                ..
            } => {
                if pressed == ElementState::Pressed {
                    state.hide_tooltip();
                }
                if state.playground.is_active() {
                    // Nothing in the room is clickable: a press is a sign of
                    // life and nothing else, and a right click must not open the
                    // editor's context menu over a kitten.
                    state.playground.activity(std::time::Instant::now());
                } else if button == MouseButton::Right {
                    if pressed == ElementState::Pressed {
                        state.right_click(state.cursor.0, state.cursor.1);
                    }
                } else if button == MouseButton::Left {
                    state.dragging = pressed == ElementState::Pressed;
                    if state.dragging {
                        state.click(state.cursor.0, state.cursor.1);
                    } else {
                        state.minimap_drag = false;
                        state.unit_drag = None;
                        state.find_dialog.end_drag();
                        state.drop_tab();
                        state.divider_drag = false;
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let was = state.cursor;
                state.cursor = (position.x, position.y);
                if state.playground.is_active() {
                    // The room takes the pointer in normalised device
                    // coordinates, because that is what a ray through the
                    // camera is made of; turning pixels into that is the window's
                    // job, and this is the window.
                    let width = state.gpu.config.width.max(1) as f64;
                    let height = state.gpu.config.height.max(1) as f64;
                    let ndc = [
                        (position.x / width * 2.0 - 1.0) as f32,
                        (1.0 - position.y / height * 2.0) as f32,
                    ];
                    state
                        .playground
                        .pointer(Some(ndc), std::time::Instant::now());
                    return;
                }
                state.playground.activity(std::time::Instant::now());
                // In distraction-free mode the menu bar appears when the pointer
                // reaches the top edge, which changes the layout. Crossing that
                // boundary has to repaint even though nothing else about the
                // frame changed, so the crossing is detected here rather than
                // left to whichever widget happens to notice.
                if state.chrome.is_distraction_free() {
                    let line_height = state.ui.line_height;
                    let before = state.chrome.shows_menu(was.1 as f32, line_height);
                    let after = state.chrome.shows_menu(position.y as f32, line_height);
                    if before != after {
                        state.window.request_redraw();
                    }
                }
                state.handle_cursor(position.x, position.y);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                state.hide_tooltip();
                if state.playground.is_active() {
                    state.playground.activity(std::time::Instant::now());
                } else {
                    state.handle_wheel(delta);
                }
            }
            // The pointer leaving the window is the one case the hit test cannot
            // see: no further move arrives, so the label would stay up over a
            // window the user is no longer pointing at. The link's cursor has
            // the same problem, and the same fix.
            WindowEvent::CursorLeft { .. } => {
                state
                    .playground
                    .pointer(None, std::time::Instant::now());
                state.hide_tooltip();
                if state.link_hover {
                    state.link_hover = false;
                    state.window.set_cursor(CursorIcon::Default);
                }
            }
            // A file dragged onto the window opens in a new tab, which is what
            // every editor does and what the plan's "drag-drop" line means.
            WindowEvent::DroppedFile(path) => {
                state.open_path(&path);
                state.window.request_redraw();
            }
            WindowEvent::RedrawRequested => state.render(),
            _ => {}
        }
    }
}

/// Appends one diagnostic line when `NPP_TRACE` names a file.
///
/// Temporary: it exists to see what the editor thinks while a window is in the
/// background, a state no test can reach.
fn main() {
    // Before anything reads a settings file, and before `--help` and `--version`
    // return, so a user who only ever runs `npp --version` still gets their
    // configuration moved. It is a directory rename, so it costs one syscall on
    // every run after the first.
    nn_app::paths::migrate();
    let args = match nn_app::cli::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(err) => {
            eprintln!("npp: {err}");
            eprint!("{}", nn_app::cli::usage());
            std::process::exit(2);
        }
    };
    if args.help {
        print!("{}", nn_app::cli::usage());
        return;
    }
    if args.version {
        println!("Neko++ {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    // A call is not a launch. It needs the window that is already running, it
    // prints the answer, and it exits: no GPU, no fonts, no event loop, because
    // the process that carries the question is not the one that answers it.
    if let Some(json) = args.call.as_deref() {
        std::process::exit(call_and_print(json));
    }
    // Hand the files to a running editor and exit. The waiter is registered
    // before the request is sent, so a document that closes immediately cannot
    // slip past a `--wait` caller.
    if nn_app::single::should_hand_off(args.new_instance) {
        if let Some(client) = nn_app::single::Client::connect() {
            if client.is_running() {
                let mut waiter = if args.wait { client.waiter().ok() } else { None };
                let request = request_for(&args);
                if client.send(&request).is_ok() {
                    // The launch is over as far as this process is concerned, and
                    // no window is going to be created to say so. Until it is
                    // said the desktop keeps its spinner turning over work that
                    // is already done, which is what makes the file manager feel
                    // slow. See `nn_app::startup`.
                    nn_app::startup::complete();
                    if let Some(waiter) = waiter.as_mut() {
                        for path in &request.paths {
                            let _ = waiter.wait_for(path);
                        }
                    }
                    return;
                }
                // The name had an owner but the call failed, which means the
                // owner is not us -- an older build, or a process that died
                // between the two calls. Falling through to a standalone
                // window is better than refusing to start.
                eprintln!("npp: could not hand off to the running editor");
            }
        }
    }
    let server = nn_app::single::Server::acquire();
    let fonts = std::thread::Builder::new()
        .name("npp-fonts".to_string())
        .spawn(load_fonts)
        .ok();
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .expect("create event loop");
    let proxy = event_loop.create_proxy();
    // Wait, not Poll: we redraw when something changes, and vsync paces us.
    event_loop.set_control_flow(ControlFlow::Wait);
    // The REST listener, when this launch asked for one. It is started here
    // rather than from `State::new` because it has to outlive the window: a
    // request that arrives while the frame is being drawn waits in the queue
    // instead of being refused.
    let settings = nn_app::settings::Settings::load();
    // The API's configuration file, when there is one: what a harness or a user
    // writes once instead of wrapping the command line. A file that exists and
    // does not load stops the launch rather than starting something other than
    // what it says -- a typo in `token` must not become a listener without one.
    let api_file = match nn_app::api_config::load() {
        Ok(loaded) => loaded,
        Err(error) => {
            eprintln!("npp: {error}");
            std::process::exit(1);
        }
    };
    if let Some(loaded) = &api_file {
        if let Some(warning) = &loaded.warning {
            eprintln!("npp: {warning}");
        }
        if let Some(ignored) = &loaded.ignored {
            eprintln!(
                "npp: {} is there and ignored; {} is the file in use",
                nn_app::api_config::describe(ignored),
                nn_app::api_config::describe(&loaded.path)
            );
        }
    }
    let api = nn_app::api_config::resolve(
        api_file.as_ref().map(|loaded| &loaded.config),
        args.rest,
        args.rest_port,
        &settings,
        std::env::var("NPP_REST_TOKEN").ok(),
    );
    if api.enabled {
        let port = api.port;
        let dispatch_proxy = proxy.clone();
        let dispatch = move |call| {
            let (sender, answer) = std::sync::mpsc::channel();
            dispatch_proxy
                .send_event(UserEvent::Rest { call, answer: sender })
                .ok()?;
            answer.recv_timeout(nn_app::rest::ANSWER_TIMEOUT).ok()
        };
        let from = api_file
            .as_ref()
            .map(|loaded| format!(", {}", nn_app::api_config::describe(&loaded.path)))
            .unwrap_or_default();
        match nn_app::rest::serve(port, api.policy.clone(), dispatch) {
            Ok(address) => {
                eprintln!("npp: REST on http://{address}/ (loopback only{from})")
            }
            Err(error) if args.rest => {
                // Asked for on the command line, so a silent fallback would be
                // the editor pretending to have done something it did not.
                eprintln!("npp: cannot serve REST on 127.0.0.1:{port}: {error}");
                std::process::exit(1);
            }
            Err(error) => eprintln!("npp: REST not started: {error}"),
        }
    }
    let mut app = App {
        state: None,
        args,
        fonts,
        proxy,
        server,
    };
    event_loop.run_app(&mut app).expect("run event loop");
}

/// The font system, and the width the editor will draw its columns at.
///
/// The two are produced together because the width is the price of the font
/// system's first shaping, and the thread that scans the fonts is otherwise
/// idle for as long as the graphics device takes to open.
type LoadedFonts = (cosmic_text::FontSystem, f32);

/// The density the window furniture is drawn at.
///
/// The settings file can insist, and does when it holds a number: some desktops
/// report a scale of one on a panel where fifteen-pixel labels are the complaint.
/// Zero, the default, means ask the display, which is the only answer that keeps
/// following the window as it moves between screens.
fn ui_scale(settings: &Settings, display: f64) -> f32 {
    if settings.ui_scale > 0.0 {
        settings.ui_scale
    } else {
        display as f32
    }
}

/// Scans the fonts and measures the editor font.
fn load_fonts() -> LoadedFonts {
    let mut font_system = nn_app::fonts::font_system();
    let width = nn_app::view::measure_editor_char_width(&mut font_system);
    (font_system, width)
}

/// Parses and answers one API call.
///
/// A request that is not JSON, or is JSON of the wrong shape, is answered with an
/// error object rather than no answer at all: a caller polls for an answer and
/// would otherwise wait out its whole timeout to be told nothing.
fn answer_to(state: &mut State, json: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(request) => match nn_app::api::parse(&request) {
            Ok(call) => state.call(call).to_string(),
            Err(error) => error.value().to_string(),
        },
        Err(error) => nn_app::api::Error {
            code: nn_app::api::BAD_REQUEST,
            message: format!("a call must be JSON: {error}"),
        }
        .value()
        .to_string(),
    }
}

/// The answer to a call that names a document no tab has open.
fn not_open(which: &nn_app::api::Which) -> serde_json::Value {
    let message = match which {
        nn_app::api::Which::Active => "there is no active document".to_string(),
        nn_app::api::Which::Path(path) => {
            format!("no tab has {path} open; `document.list` shows what is open")
        }
    };
    nn_app::api::Error {
        code: nn_app::api::UNKNOWN_DOCUMENT,
        message,
    }
    .value()
}

/// The editor's selection as a pair of character offsets, if it has one.
fn selection_of(editor: &Editor) -> Option<(usize, usize)> {
    if editor.selection_len() == 0 {
        None
    } else {
        Some((editor.selection_start(), editor.selection_end()))
    }
}

/// Sends one API call to the running editor and prints the answer.
///
/// Returns the exit status: 0 when the window answered, 1 when it did not, and 2
/// when the question itself was malformed. A refused call is a failure of the
/// script that asked, and a script that ignores an exit status deserves what it
/// gets.
fn call_and_print(json: &str) -> i32 {
    // Parsed here before it travels: a misspelling that reached the window would
    // come back as an error object a second later, and the caller would have to
    // read it to find out that nothing had happened at all.
    let request: serde_json::Value = match serde_json::from_str(json) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("npp: --call is not JSON: {error}");
            return 2;
        }
    };
    if let Err(error) = nn_app::api::parse(&request) {
        println!("{}", error.value());
        eprintln!("npp: {}", error.message);
        return 2;
    }
    let Some(client) = nn_app::single::Client::connect() else {
        return no_instance();
    };
    if !client.is_running() {
        return no_instance();
    }
    match client.call(json) {
        Ok(answer) => {
            println!("{answer}");
            0
        }
        Err(error) => {
            println!(
                "{}",
                nn_app::api::Error {
                    code: nn_app::api::FAILED,
                    message: error.to_string(),
                }
                .value()
            );
            eprintln!("npp: {error}");
            1
        }
    }
}

/// What a call prints when no editor is running.
///
/// The code is the MCP server's to reuse: `no_instance` is the one answer a
/// caller can act on, because the fix is to start the editor rather than to
/// correct the request.
fn no_instance() -> i32 {
    println!(
        "{}",
        nn_app::api::Error {
            code: "no_instance",
            message: "no Neko++ window is running to answer; start `npp` first".to_string(),
        }
        .value()
    );
    eprintln!("npp: no Neko++ window is running to answer");
    1
}

/// The bus request a launch turns into.
///
/// Paths are made absolute here, in the process that still knows its own
/// working directory. Standard input has no path to send, so a `-` launch is
/// never handed off -- it reads the pipe itself.
fn request_for(args: &nn_app::cli::Args) -> nn_app::single::Request {
    let paths = args
        .files
        .iter()
        .map(|path| nn_app::single::absolute(path).to_string_lossy().to_string())
        .collect();
    let mut request = nn_app::single::Request::new(paths);
    if let Some(line) = args.line {
        request.line = line as u32;
    }
    if let Some(column) = args.column {
        request.column = column as u32;
    }
    if args.wait {
        request.wait = true;
    }
    request
}
