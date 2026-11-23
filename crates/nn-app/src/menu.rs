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

//! The menu bar.
//!
//! Notepad++ is a menu-driven editor, so the menu is not decoration: it is the
//! primary way to reach a command. This module owns the bar, the dropdowns, and
//! the hit testing; it does not own the commands themselves. A click resolves to
//! an `Action` and the application decides what that means.
//!
//! Everything is laid out from the monospace advance width, so a dropdown is a
//! rectangle whose width is the longest label plus its shortcut. That keeps the
//! layout arithmetic exact and avoids measuring text twice.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics};
use glyphon::{TextArea, TextBounds};

use nn_core::{CodePage, Encoding};

use crate::i18n::Catalog;
use crate::keymap::Keymap;
use crate::quad::QuadRenderer;
use crate::settings::ThemeChoice;
use crate::text::shaping_for;
use crate::theme::Theme;

/// One of the fourteen sort orders the Line Operations submenu offers.
///
/// An enum rather than a `SortOptions` value because the menu offers exactly
/// these fourteen and nothing else: a user cannot ask for "integers, descending,
/// case-insensitively", so there is no reason for the type to allow it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    LexAsc,
    LexAscIgnoreCase,
    LocaleAsc,
    IntegerAsc,
    DecimalCommaAsc,
    DecimalDotAsc,
    LengthAsc,
    LexDesc,
    LexDescIgnoreCase,
    LocaleDesc,
    IntegerDesc,
    DecimalCommaDesc,
    DecimalDotDesc,
    LengthDesc,
}

impl Sort {
    /// The `SortOptions` this row means.
    pub fn options(self) -> nn_core::lines::SortOptions {
        use nn_core::lines::{SortKey, SortOrder, SortOptions};
        let (order, key) = match self {
            Sort::LexAsc => (SortOrder::Ascending, SortKey::Text),
            Sort::LexAscIgnoreCase => (SortOrder::Ascending, SortKey::TextIgnoreCase),
            Sort::LocaleAsc => (SortOrder::Ascending, SortKey::Locale),
            Sort::IntegerAsc => (SortOrder::Ascending, SortKey::Integer),
            Sort::DecimalCommaAsc => (SortOrder::Ascending, SortKey::DecimalComma),
            Sort::DecimalDotAsc => (SortOrder::Ascending, SortKey::DecimalDot),
            Sort::LengthAsc => (SortOrder::Ascending, SortKey::Length),
            Sort::LexDesc => (SortOrder::Descending, SortKey::Text),
            Sort::LexDescIgnoreCase => (SortOrder::Descending, SortKey::TextIgnoreCase),
            Sort::LocaleDesc => (SortOrder::Descending, SortKey::Locale),
            Sort::IntegerDesc => (SortOrder::Descending, SortKey::Integer),
            Sort::DecimalCommaDesc => (SortOrder::Descending, SortKey::DecimalComma),
            Sort::DecimalDotDesc => (SortOrder::Descending, SortKey::DecimalDot),
            Sort::LengthDesc => (SortOrder::Descending, SortKey::Length),
        };
        SortOptions {
            order,
            key,
            case_sensitive: true,
        }
    }
}

/// The four matching rules the multi-select rows offer.
///
/// A struct rather than four variants because the four rows are the four
/// combinations of two independent flags, and naming the flags is what lets the
/// editor take them without the menu having to enumerate them again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchMode {
    /// Whether `Foo` and `foo` are different.
    pub match_case: bool,
    /// Whether a match has to be a whole word.
    pub whole_word: bool,
}

/// How the Window menu's "Sort By" rows order the tab strip.
///
/// The ten rows are Notepad++'s own, and they sort the *tabs*, not the text: the
/// Window menu is about which document is where, so its sort is a view of the
/// open set rather than an edit of any one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowSort {
    NameAsc,
    NameDesc,
    PathAsc,
    PathDesc,
    TypeAsc,
    TypeDesc,
    SizeAsc,
    SizeDesc,
    ModifiedAsc,
    ModifiedDesc,
}

impl WindowSort {
    /// The label the Window menu shows.
    pub fn label(self) -> &'static str {
        match self {
            WindowSort::NameAsc => "Name A to Z",
            WindowSort::NameDesc => "Name Z to A",
            WindowSort::PathAsc => "Path A to Z",
            WindowSort::PathDesc => "Path Z to A",
            WindowSort::TypeAsc => "Type A to Z",
            WindowSort::TypeDesc => "Type Z to A",
            WindowSort::SizeAsc => "Content Length Ascending",
            WindowSort::SizeDesc => "Content Length Descending",
            WindowSort::ModifiedAsc => "Modified Time Ascending",
            WindowSort::ModifiedDesc => "Modified Time Descending",
        }
    }

    /// Every row, in menu order.
    pub fn all() -> [WindowSort; 10] {
        [
            WindowSort::NameAsc,
            WindowSort::NameDesc,
            WindowSort::PathAsc,
            WindowSort::PathDesc,
            WindowSort::TypeAsc,
            WindowSort::TypeDesc,
            WindowSort::SizeAsc,
            WindowSort::SizeDesc,
            WindowSort::ModifiedAsc,
            WindowSort::ModifiedDesc,
        ]
    }
}

/// Which browser "View Current File in" names.
///
/// The four rows are Notepad++'s, and on Linux none of them is a promise the
/// program can keep: there is no registry to ask which browser is installed. All
/// four open the default browser, and the labels are kept because a user looking
/// for the row should find it where they expect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserKind {
    Firefox,
    Chrome,
    Edge,
    Ie,
}

impl BrowserKind {
    /// The label the View menu shows.
    pub fn label(self) -> &'static str {
        match self {
            BrowserKind::Firefox => "Firefox",
            BrowserKind::Chrome => "Chrome",
            BrowserKind::Edge => "Edge",
            BrowserKind::Ie => "IE",
        }
    }

    /// The program this row tries to launch.
    ///
    /// `IE` has no Linux binary, so it names the one program that is always
    /// there: the desktop's own handler. The row keeps Notepad++'s label because
    /// the menu is meant to be recognisable, and the fallback in
    /// `desktop::open_url_with` means a missing browser is not a dead row.
    pub fn command(self) -> &'static str {
        match self {
            BrowserKind::Firefox => "firefox",
            BrowserKind::Chrome => "google-chrome",
            BrowserKind::Edge => "microsoft-edge",
            BrowserKind::Ie => "xdg-open",
        }
    }

    /// Every row, in menu order.
    pub fn all() -> [BrowserKind; 4] {
        [
            BrowserKind::Firefox,
            BrowserKind::Chrome,
            BrowserKind::Edge,
            BrowserKind::Ie,
        ]
    }
}

/// A command the menu can invoke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    New,
    Open,
    Save,
    SaveAs,
    CloseTab,
    Exit,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Delete,
    SelectAll,
    Find,
    Replace,
    FindNext,
    FindPrev,
    FindInFiles,
    /// Search as you type, following the first match.
    IncrementalSearch,
    FoldToggle,
    FoldAll,
    UnfoldAll,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ToggleMinimap,
    ToggleToolbar,
    ToggleFunctionList,
    /// Show or hide the folder-as-workspace panel.
    FolderAsWorkspace,
    /// Show project panel `n`, zero-based.
    ProjectPanel(usize),
    /// Highlight the document with user-defined language `n`.
    Language(usize),
    /// Highlight the document with no lexer at all.
    LanguagePlain,
    /// Highlight the document with built-in language `n` of `LANGS`.
    ///
    /// An index rather than a name so the variant stays `Copy` and so the
    /// dispatcher does not have to search a table of ninety-three strings on
    /// every click.
    LanguageBuiltIn(usize),
    /// A row that exists so a dropdown is not empty, and does nothing when
    /// chosen: "No Plugins Installed", "(none installed)".
    NoOp,
    /// Open the shortcut mapper.
    ShortcutMapper,
    /// Copy a plugin file into the plugins directory.
    ImportPlugin,
    /// Take the colours from a `stylers.xml`.
    ImportTheme,
    /// Open the popup context menu's layout file for editing.
    EditContextMenu,
    /// Open the user-defined language file for editing.
    DefineLanguage,
    /// Open the directory the user-defined languages live in.
    OpenUdlFolder,
    /// Open the page Notepad++ points at for ready-made languages.
    UdlCollection,
    /// Add the active document's directory to the active project.
    AddProjectRoot,
    /// Rescan the workspace panel.
    RefreshFolder,
    /// Search every root of every project.
    FindInProjects,
    CompleteWord,
    /// Function completion: declarations from the active document.
    CompleteFunction,
    /// Document completion: words from every open document.
    CompleteDocument,
    /// Path completion: filesystem entries.
    CompletePath,
    /// The signature of the call the caret is inside.
    ParamHint,
    Preferences,
    StyleConfigurator,
    ToggleWrap,
    ToggleTheme,
    /// Switch to the `n`th entry of [`ThemeChoice::ALL`].
    ///
    /// One action per theme rather than a "next theme" command: fifty entries
    /// is a list to choose from, not a knob to turn, and the menu can
    /// only show what each row does if each row is its own action.
    Theme(usize),
    GotoLine,
    ToggleBookmark,
    NextBookmark,
    PrevBookmark,
    ClearBookmarks,
    /// Open the `n`th entry of the recent file list.
    OpenRecent(usize),
    /// Run a command line typed into the prompt.
    Run,
    /// Run the `n`th saved command.
    RunSaved(usize),
    ClearRecent,
    ToggleRecording,
    PlayMacro,
    SortAsc,
    SortDesc,
    SortNumeric,
    DedupeConsecutive,
    DedupeAll,
    TrimTrailing,
    TrimLeading,
    RemoveEmpty,
    JoinLines,
    SplitLines,
    UpperCase,
    LowerCase,
    TitleCase,
    ToggleComment,
    TabsToSpaces,
    SpacesToTabs,
    DuplicateLine,
    MoveLineUp,
    MoveLineDown,
    NextTab,
    PrevTab,
    DocumentPeeker,
    ImportNotepad,
    ExportShortcuts,
    SaveAll,
    /// Show the print preview.
    Print,
    /// Send the document straight to the printer, with no preview.
    PrintNow,
    CloseAll,
    PinTab,
    TabColour,
    /// Cycle the tab strip between single-row, multi-line and vertical.
    TabLayout,
    CloseOthers,
    CloseToLeft,
    CloseToRight,
    Reload,
    CopyPath,
    OpenContainingFolder,
    DeleteLine,
    SelectLine,
    Indent,
    Unindent,
    InsertDateTime,
    ColumnEditor,
    /// Open the floating character grid.
    CharPanel,
    AddCaretAbove,
    AddCaretBelow,
    EolWindows,
    EolUnix,
    EolMac,
    /// Reinterpret the file's bytes as `Encoding`.
    EncodingSet(Encoding),
    /// Re-encode the text as `Encoding`, so the next save writes new bytes.
    EncodingConvert(Encoding),
    FullScreen,
    DistractionFree,
    /// Hide the window behind a room with a kitten in it.
    Playground,
    ChangeHistory,
    ReadOnly,
    ShowWhitespace,
    ShowEol,
    ShowIndentGuides,
    MarkAll,
    GotoBrace,
    SplitVertical,
    SplitHorizontal,
    Unsplit,
    /// Turn the split a quarter turn: side-by-side becomes top-and-bottom.
    RotateSplit,
    MoveToOtherView,
    CloneToOtherView,
    About,
    /// Open the active tab's file with whatever the desktop has registered.
    OpenInDefaultViewer,
    /// Adopt a directory as the workspace root.
    OpenFolderAsWorkspace,
    /// Write a copy of the active tab somewhere, leaving the tab where it is.
    SaveCopyAs,
    /// Rename the active tab's file on disk.
    Rename,
    /// Close every tab that is not pinned.
    CloseAllButPinned,
    /// Close every tab with no unsaved changes.
    CloseAllUnchanged,
    /// Move the active tab's file to the desktop trash.
    MoveToTrash,
    /// Read a session file.
    LoadSession,
    /// Write a session file.
    SaveSession,
    /// Open a terminal in the active tab's directory.
    OpenContainingFolderCmd,
    /// Open a shell in the active tab's directory.
    OpenContainingFolderPowerShell,
    /// Start or end a stream selection.
    BeginEndSelect,
    /// Start or end a rectangular selection.
    BeginEndSelectColumn,
    /// Insert the date and time in one of three shapes.
    InsertDateTimeShort,
    InsertDateTimeLong,
    InsertDateTimeCustom,
    /// Copy the active file's name, or its directory, to the clipboard.
    CopyFilename,
    CopyDirPath,
    /// Copy every open file's name, or its path, one per line.
    CopyAllFilenames,
    CopyAllFilePaths,
    /// The six case conversions beyond upper, lower and title.
    ProperCaseForce,
    ProperCaseBlend,
    SentenceCaseForce,
    SentenceCaseBlend,
    InvertCase,
    RandomCase,
    /// Drop lines that are empty or contain only blanks.
    RemoveEmptyWithBlank,
    /// Insert a blank line above or below the caret's line.
    InsertBlankAbove,
    InsertBlankBelow,
    /// Reverse or shuffle the order of the lines.
    ReverseLineOrder,
    RandomizeLineOrder,
    /// Sort the lines one of the fourteen ways.
    SortLines(Sort),
    /// Add or remove a comment marker.
    SingleLineComment,
    SingleLineUncomment,
    BlockComment,
    BlockUncomment,
    /// Step the parameter hint to the previous or next overload.
    ParamHintPrev,
    ParamHintNext,
    /// Strip whitespace from both ends, or turn line ends into spaces.
    TrimBoth,
    EolToSpace,
    TrimAll,
    /// Convert spaces to tabs everywhere, or only in the indent.
    SpacesToTabsAll,
    SpacesToTabsLeading,
    /// Open the file or folder named by the selection.
    OpenSelectedFile,
    OpenSelectedFolder,
    /// Replace the selection with blocks.
    RedactSelection,
    /// Search the web for the selection.
    SearchOnInternet,
    ChangeSearchEngine,
    /// Put a caret on every occurrence of the current word or selection.
    MultiSelectAll(MatchMode),
    /// Add a caret on the next occurrence.
    MultiSelectNext(MatchMode),
    /// Drop the most recently added caret.
    MultiSelectUndo,
    /// Skip the next occurrence and select the one after it.
    MultiSelectSkip,
    /// Toggle rectangular selection for the mouse.
    ColumnMode,
    /// Show or hide the clipboard history panel.
    ClipboardHistory,
    /// Paste the `index`th entry of the clipboard history.
    ///
    /// The history is a popup of rows rather than a dialog, so each row needs an
    /// action that names which entry it stands for.
    PasteClipboard(usize),
    /// Keep the window above every other one.
    AlwaysOnTop,
    /// Frameless, always on top, no chrome.
    PostIt,
    /// Open the saved document in a browser.
    ViewInBrowser(BrowserKind),
    /// Draw the characters that have no glyph of their own.
    ShowNonPrinting,
    ShowControlChars,
    ShowAllChars,
    /// Draw a mark where a long line wraps.
    ShowWrapSymbol,
    /// Apply a zoom change to both panes.
    ZoomSync,
    /// Hand the document to a second process.
    MoveToNewInstance,
    OpenInNewInstance,
    /// Bring the nth tab to the front.
    GotoTab(usize),
    FirstTab,
    LastTab,
    /// Move the active tab to either end of the strip.
    MoveTabStart,
    MoveTabEnd,
    MoveTabForward,
    MoveTabBackward,
    /// Colour the active tab, or clear its colour.
    TabColourSet(Option<usize>),
    /// Put the keyboard in the other pane.
    FocusOtherView,
    /// Fold the selected lines away.
    HideLines,
    /// Fold or unfold at the caret's own depth.
    FoldCurrentLevel,
    UnfoldCurrentLevel,
    /// Fold or unfold to a depth.
    FoldLevel(usize),
    UnfoldLevel(usize),
    /// Report the document's counts.
    Summary,
    /// Toggle the minimap column.
    DocumentMap,
    /// List the open documents.
    DocumentList,
    /// Keep the two panes' scrolls together.
    SyncScrollV,
    SyncScrollH,
    /// Reload the file whenever it changes on disk.
    Monitoring,
    /// Extend the selection to the next match and go there.
    SelectAndFindNext,
    SelectAndFindPrev,
    /// Step through matches without disturbing the find bar's own state.
    VolatileFindNext,
    VolatileFindPrev,
    /// Bring the results panel to the front.
    SearchResultsWindow,
    /// Step through the hits in the results panel.
    NextSearchResult,
    PrevSearchResult,
    /// Select everything between the brace next to the caret and its partner.
    SelectBetweenBraces,
    /// Open the Mark dialog.
    Mark,
    /// Step through the change history.
    ChangeNext,
    ChangePrev,
    ClearChangeHistory,
    /// Paint every occurrence of the current token in style `n`.
    StyleAll(usize),
    /// Paint just this occurrence in style `n`.
    StyleOne(usize),
    /// Drop the marks in style `n`.
    ClearStyle(usize),
    ClearAllStyles,
    /// Jump to the next or previous mark; `None` is "whichever style has one".
    JumpUp(Option<usize>),
    JumpDown(Option<usize>),
    /// Copy the marked text; `None` is every style.
    CopyStyled(Option<usize>),
    /// Bookmarked-line operations.
    CutBookmarkedLines,
    CopyBookmarkedLines,
    PasteToBookmarkedLines,
    RemoveBookmarkedLines,
    RemoveNonBookmarkedLines,
    InverseBookmarks,
    /// Find a character by code point.
    FindCharInRange,
    /// Hash the active document with one of the four digests.
    HashGenerate(nn_core::hash::Hash),
    /// Hash a file chosen from disk.
    HashFromFile(nn_core::hash::Hash),
    /// Hash the selection and put the digest on the clipboard.
    HashToClipboard(nn_core::hash::Hash),
    /// Open the plugin manager: every installed plugin, with a switch.
    ManagePlugins,
    /// Open the plugins directory in the file manager.
    OpenPluginsFolder,
    /// Re-read the plugins directory, picking up files added since startup.
    ReloadPlugins,
    /// Run one command of one running plugin: its index in the inventory, then
    /// the index of the command in the list the plugin gave.
    ///
    /// Two indices rather than two names because the rows are built from the
    /// inventory and dropped when it changes: a row and the pair it carries are
    /// never older than each other. Nothing saves one of these, so an index
    /// cannot be replayed against a folder that has moved on -- the row is not
    /// bindable, for the same reason a language row is not.
    Plugin(usize, usize),
    /// Check the keymap against a `shortcuts.xml`.
    ValidateShortcuts,
    /// Stop the macro recorder and keep what it has.
    StopRecording,
    /// Save the recorded macro into the Run menu's list.
    SaveMacro,
    /// Play the recorded macro a number of times.
    RunMacroMultiple,
    /// Show the list of open documents.
    WindowsList,
    /// Bring a tab to the front by index.
    FocusTab(usize),
    /// Bring the most recently used other tab to the front.
    RecentWindow,
    /// Reorder the tab strip.
    SortWindows(WindowSort),
    /// Show the command line the process was started with.
    CommandLineArgs,
    /// Open one of the project's web pages.
    HomePage,
    ProjectPage,
    OnlineManual,
    Forum,
    /// Open the terms the source is under.
    License,
    /// Print the build and environment details.
    DebugInfo,
    /// Make the active document read-only, or every document.
    ReadOnlyCurrent,
    ReadOnlyAll,
    ClearReadOnlyAll,
    /// A row that opens a submenu rather than running anything.
    ///
    /// It exists so a submenu row is still an `Item` with an `Action` like every
    /// other row: the alternative was an `Option<Action>` threaded through the
    /// public `RowItem`, and every caller would then have to handle a `None` that
    /// only ever means "this row is a heading". It is not bindable and never
    /// reaches the dispatcher.
    Submenu,
}

impl Action {
    /// The name this command is written as in the settings file.
    ///
    /// Stable: a user's `key` lines have to keep meaning the same thing across
    /// versions, so these are not derived from the enum's `Debug` output.
    pub fn name(self) -> &'static str {
        match self {
            Action::New => "new",
            Action::Open => "open",
            Action::Save => "save",
            Action::SaveAs => "save_as",
            Action::CloseTab => "close_tab",
            Action::Exit => "exit",
            Action::Undo => "undo",
            Action::Redo => "redo",
            Action::Cut => "cut",
            Action::Copy => "copy",
            Action::Paste => "paste",
            Action::Delete => "delete",
            Action::SelectAll => "select_all",
            Action::Find => "find",
            Action::Replace => "replace",
            Action::FindNext => "find_next",
            Action::FindPrev => "find_prev",
            Action::FindInFiles => "find_in_files",
            Action::IncrementalSearch => "incremental_search",
            Action::FoldToggle => "fold_toggle",
            Action::FoldAll => "fold_all",
            Action::UnfoldAll => "unfold_all",
            Action::ZoomIn => "zoom_in",
            Action::ZoomOut => "zoom_out",
            Action::ZoomReset => "zoom_reset",
            Action::ToggleMinimap => "toggle_minimap",
            Action::ToggleToolbar => "toggle_toolbar",
            Action::ToggleFunctionList => "toggle_function_list",
            Action::FolderAsWorkspace => "folder_as_workspace",
            Action::ProjectPanel(_) => "project_panel",
            Action::Language(_) => "language",
            Action::LanguagePlain => "language_plain",
            Action::LanguageBuiltIn(_) => "language_built_in",
            Action::NoOp => "no_op",
            Action::ShortcutMapper => "shortcut_mapper",
            Action::ImportPlugin => "import_plugin",
            Action::ImportTheme => "import_theme",
            Action::EditContextMenu => "edit_context_menu",
            Action::DefineLanguage => "define_language",
            Action::OpenUdlFolder => "open_udl_folder",
            Action::UdlCollection => "udl_collection",
            Action::AddProjectRoot => "add_project_root",
            Action::RefreshFolder => "refresh_folder",
            Action::FindInProjects => "find_in_projects",
            Action::CompleteWord => "complete_word",
            Action::CompleteFunction => "complete_function",
            Action::CompleteDocument => "complete_document",
            Action::CompletePath => "complete_path",
            Action::ParamHint => "param_hint",
            Action::Preferences => "preferences",
            Action::StyleConfigurator => "style_configurator",
            Action::ToggleWrap => "toggle_wrap",
            Action::ToggleTheme => "toggle_theme",
            Action::Theme(_) => "theme",
            Action::GotoLine => "goto_line",
            Action::ToggleBookmark => "toggle_bookmark",
            Action::NextBookmark => "next_bookmark",
            Action::PrevBookmark => "prev_bookmark",
            Action::ClearBookmarks => "clear_bookmarks",
            Action::OpenRecent(_) => "open_recent",
            Action::Run => "run",
            Action::RunSaved(_) => "run_saved",
            Action::ClearRecent => "clear_recent",
            Action::ToggleRecording => "toggle_recording",
            Action::PlayMacro => "play_macro",
            Action::SortAsc => "sort_asc",
            Action::SortDesc => "sort_desc",
            Action::SortNumeric => "sort_numeric",
            Action::DedupeConsecutive => "dedupe_consecutive",
            Action::DedupeAll => "dedupe_all",
            Action::TrimTrailing => "trim_trailing",
            Action::TrimLeading => "trim_leading",
            Action::RemoveEmpty => "remove_empty",
            Action::JoinLines => "join_lines",
            Action::SplitLines => "split_lines",
            Action::UpperCase => "upper_case",
            Action::LowerCase => "lower_case",
            Action::TitleCase => "title_case",
            Action::ToggleComment => "toggle_comment",
            Action::TabsToSpaces => "tabs_to_spaces",
            Action::SpacesToTabs => "spaces_to_tabs",
            Action::DuplicateLine => "duplicate_line",
            Action::MoveLineUp => "move_line_up",
            Action::MoveLineDown => "move_line_down",
            Action::NextTab => "next_tab",
            Action::PrevTab => "prev_tab",
            Action::DocumentPeeker => "document_peeker",
            Action::ImportNotepad => "import_notepad",
            Action::ExportShortcuts => "export_shortcuts",
            Action::SaveAll => "save_all",
            Action::Print => "print",
            Action::PrintNow => "print_now",
            Action::CloseAll => "close_all",
            Action::PinTab => "pin_tab",
            Action::TabColour => "tab_colour",
            Action::TabLayout => "tab_layout",
            Action::CloseOthers => "close_others",
            Action::CloseToLeft => "close_to_left",
            Action::CloseToRight => "close_to_right",
            Action::Reload => "reload",
            Action::CopyPath => "copy_path",
            Action::OpenContainingFolder => "open_containing_folder",
            Action::DeleteLine => "delete_line",
            Action::SelectLine => "select_line",
            Action::Indent => "indent",
            Action::Unindent => "unindent",
            Action::InsertDateTime => "insert_datetime",
            Action::ColumnEditor => "column_editor",
            Action::CharPanel => "char_panel",
            Action::AddCaretAbove => "add_caret_above",
            Action::AddCaretBelow => "add_caret_below",
            Action::EolWindows => "eol_windows",
            Action::EolUnix => "eol_unix",
            Action::EolMac => "eol_mac",
            Action::EncodingSet(_) => "encoding_set",
            Action::EncodingConvert(_) => "encoding_convert",
            Action::FullScreen => "full_screen",
            Action::DistractionFree => "distraction_free",
            Action::Playground => "playground",
            Action::ChangeHistory => "change_history",
            Action::ReadOnly => "read_only",
            Action::ShowWhitespace => "show_whitespace",
            Action::ShowEol => "show_eol",
            Action::ShowIndentGuides => "show_indent_guides",
            Action::MarkAll => "mark_all",
            Action::GotoBrace => "goto_brace",
            Action::SplitVertical => "split_vertical",
            Action::SplitHorizontal => "split_horizontal",
            Action::Unsplit => "unsplit",
            Action::RotateSplit => "rotate_split",
            Action::MoveToOtherView => "move_to_other_view",
            Action::CloneToOtherView => "clone_to_other_view",
            Action::About => "about",
            Action::OpenInDefaultViewer => "open_in_default_viewer",
            Action::OpenFolderAsWorkspace => "open_folder_as_workspace",
            Action::SaveCopyAs => "save_copy_as",
            Action::Rename => "rename",
            Action::CloseAllButPinned => "close_all_but_pinned",
            Action::CloseAllUnchanged => "close_all_unchanged",
            Action::MoveToTrash => "move_to_trash",
            Action::LoadSession => "load_session",
            Action::SaveSession => "save_session",
            Action::OpenContainingFolderCmd => "open_containing_folder_cmd",
            Action::OpenContainingFolderPowerShell => "open_containing_folder_powershell",
            Action::BeginEndSelect => "begin_end_select",
            Action::BeginEndSelectColumn => "begin_end_select_column",
            Action::InsertDateTimeShort => "insert_datetime_short",
            Action::InsertDateTimeLong => "insert_datetime_long",
            Action::InsertDateTimeCustom => "insert_datetime_custom",
            Action::CopyFilename => "copy_filename",
            Action::CopyDirPath => "copy_dir_path",
            Action::CopyAllFilenames => "copy_all_filenames",
            Action::CopyAllFilePaths => "copy_all_file_paths",
            Action::ProperCaseForce => "proper_case",
            Action::ProperCaseBlend => "proper_case_blend",
            Action::SentenceCaseForce => "sentence_case",
            Action::SentenceCaseBlend => "sentence_case_blend",
            Action::InvertCase => "invert_case",
            Action::RandomCase => "random_case",
            Action::RemoveEmptyWithBlank => "remove_empty_with_blank",
            Action::InsertBlankAbove => "insert_blank_above",
            Action::InsertBlankBelow => "insert_blank_below",
            Action::ReverseLineOrder => "reverse_line_order",
            Action::RandomizeLineOrder => "randomize_line_order",
            Action::SortLines(_) => "sort_lines",
            Action::SingleLineComment => "single_line_comment",
            Action::SingleLineUncomment => "single_line_uncomment",
            Action::BlockComment => "block_comment",
            Action::BlockUncomment => "block_uncomment",
            Action::ParamHintPrev => "param_hint_prev",
            Action::ParamHintNext => "param_hint_next",
            Action::TrimBoth => "trim_both",
            Action::EolToSpace => "eol_to_space",
            Action::TrimAll => "trim_all",
            Action::SpacesToTabsAll => "spaces_to_tabs_all",
            Action::SpacesToTabsLeading => "spaces_to_tabs_leading",
            Action::OpenSelectedFile => "open_selected_file",
            Action::OpenSelectedFolder => "open_selected_folder",
            Action::RedactSelection => "redact_selection",
            Action::SearchOnInternet => "search_on_internet",
            Action::ChangeSearchEngine => "change_search_engine",
            Action::MultiSelectAll(_) => "multi_select_all",
            Action::MultiSelectNext(_) => "multi_select_next",
            Action::MultiSelectUndo => "multi_select_undo",
            Action::MultiSelectSkip => "multi_select_skip",
            Action::ColumnMode => "column_mode",
            Action::ClipboardHistory => "clipboard_history",
            Action::PasteClipboard(_) => "paste_clipboard",
            Action::ReadOnlyCurrent => "read_only_current",
            Action::ReadOnlyAll => "read_only_all",
            Action::ClearReadOnlyAll => "clear_read_only_all",
            // The digest is part of the name so a binding can name which one it
            // wants; the three commands differ only in where the bytes come from.
            Action::HashGenerate(hash) => match hash {
                nn_core::hash::Hash::Md5 => "hash_md5",
                nn_core::hash::Hash::Sha1 => "hash_sha1",
                nn_core::hash::Hash::Sha256 => "hash_sha256",
                nn_core::hash::Hash::Sha512 => "hash_sha512",
            },
            Action::HashFromFile(hash) => match hash {
                nn_core::hash::Hash::Md5 => "hash_file_md5",
                nn_core::hash::Hash::Sha1 => "hash_file_sha1",
                nn_core::hash::Hash::Sha256 => "hash_file_sha256",
                nn_core::hash::Hash::Sha512 => "hash_file_sha512",
            },
            Action::HashToClipboard(hash) => match hash {
                nn_core::hash::Hash::Md5 => "hash_clipboard_md5",
                nn_core::hash::Hash::Sha1 => "hash_clipboard_sha1",
                nn_core::hash::Hash::Sha256 => "hash_clipboard_sha256",
                nn_core::hash::Hash::Sha512 => "hash_clipboard_sha512",
            },
            Action::AlwaysOnTop => "always_on_top",
            Action::PostIt => "post_it",
            Action::ViewInBrowser(_) => "view_in_browser",
            Action::ShowNonPrinting => "show_non_printing",
            Action::ShowControlChars => "show_control_chars",
            Action::ShowAllChars => "show_all_chars",
            Action::ShowWrapSymbol => "show_wrap_symbol",
            Action::ZoomSync => "zoom_sync",
            Action::MoveToNewInstance => "move_to_new_instance",
            Action::OpenInNewInstance => "open_in_new_instance",
            Action::GotoTab(_) => "goto_tab",
            Action::FirstTab => "first_tab",
            Action::LastTab => "last_tab",
            Action::MoveTabStart => "move_tab_start",
            Action::MoveTabEnd => "move_tab_end",
            Action::MoveTabForward => "move_tab_forward",
            Action::MoveTabBackward => "move_tab_backward",
            Action::TabColourSet(_) => "tab_colour_set",
            Action::FocusOtherView => "focus_other_view",
            Action::HideLines => "hide_lines",
            Action::FoldCurrentLevel => "fold_current_level",
            Action::UnfoldCurrentLevel => "unfold_current_level",
            Action::FoldLevel(_) => "fold_level",
            Action::UnfoldLevel(_) => "unfold_level",
            Action::Summary => "summary",
            Action::DocumentMap => "document_map",
            Action::DocumentList => "document_list",
            Action::SyncScrollV => "sync_scroll_v",
            Action::SyncScrollH => "sync_scroll_h",
            Action::Monitoring => "monitoring",
            Action::SelectAndFindNext => "select_and_find_next",
            Action::SelectAndFindPrev => "select_and_find_prev",
            Action::VolatileFindNext => "volatile_find_next",
            Action::VolatileFindPrev => "volatile_find_prev",
            Action::SearchResultsWindow => "search_results_window",
            Action::NextSearchResult => "next_search_result",
            Action::PrevSearchResult => "prev_search_result",
            Action::SelectBetweenBraces => "select_between_braces",
            Action::Mark => "mark",
            Action::ChangeNext => "change_next",
            Action::ChangePrev => "change_prev",
            Action::ClearChangeHistory => "clear_change_history",
            Action::StyleAll(style) => match style {
                0 => "style_all_1",
                1 => "style_all_2",
                2 => "style_all_3",
                3 => "style_all_4",
                _ => "style_all_5",
            },
            Action::StyleOne(style) => match style {
                0 => "style_one_1",
                1 => "style_one_2",
                2 => "style_one_3",
                3 => "style_one_4",
                _ => "style_one_5",
            },
            Action::ClearStyle(style) => match style {
                0 => "clear_style_1",
                1 => "clear_style_2",
                2 => "clear_style_3",
                3 => "clear_style_4",
                _ => "clear_style_5",
            },
            Action::ClearAllStyles => "clear_all_styles",
            Action::JumpUp(style) => match style {
                Some(0) => "jump_up_1",
                Some(1) => "jump_up_2",
                Some(2) => "jump_up_3",
                Some(3) => "jump_up_4",
                Some(_) => "jump_up_5",
                None => "jump_up_mark",
            },
            Action::JumpDown(style) => match style {
                Some(0) => "jump_down_1",
                Some(1) => "jump_down_2",
                Some(2) => "jump_down_3",
                Some(3) => "jump_down_4",
                Some(_) => "jump_down_5",
                None => "jump_down_mark",
            },
            Action::CopyStyled(style) => match style {
                Some(0) => "copy_style_1",
                Some(1) => "copy_style_2",
                Some(2) => "copy_style_3",
                Some(3) => "copy_style_4",
                Some(_) => "copy_style_5",
                None => "copy_all_styles",
            },
            Action::CutBookmarkedLines => "cut_bookmarked_lines",
            Action::CopyBookmarkedLines => "copy_bookmarked_lines",
            Action::PasteToBookmarkedLines => "paste_bookmarked_lines",
            Action::RemoveBookmarkedLines => "remove_bookmarked_lines",
            Action::RemoveNonBookmarkedLines => "remove_non_bookmarked_lines",
            Action::InverseBookmarks => "inverse_bookmarks",
            Action::FindCharInRange => "find_char_in_range",
            Action::ManagePlugins => "manage_plugins",
            Action::OpenPluginsFolder => "open_plugins_folder",
            Action::ReloadPlugins => "reload_plugins",
            Action::Plugin(_, _) => "plugin",
            Action::ValidateShortcuts => "validate_shortcuts",
            Action::StopRecording => "stop_recording",
            Action::SaveMacro => "save_macro",
            Action::RunMacroMultiple => "run_macro_multiple",
            Action::WindowsList => "windows_list",
            Action::FocusTab(_) => "focus_tab",
            Action::RecentWindow => "recent_window",
            Action::SortWindows(_) => "sort_windows",
            Action::CommandLineArgs => "command_line_args",
            Action::HomePage => "home_page",
            Action::ProjectPage => "project_page",
            Action::OnlineManual => "online_manual",
            Action::Forum => "forum",
            Action::License => "license",
            Action::DebugInfo => "debug_info",
            Action::Submenu => "submenu",
        }
    }

    /// Reads a name written by [`Action::name`].
    ///
    /// `OpenRecent` is not here: it carries an index, and a key that opened
    /// "the third recent file" would mean something different every day.
    pub fn from_name(name: &str) -> Option<Action> {
        let action = match name {
            "new" => Action::New,
            "open" => Action::Open,
            "save" => Action::Save,
            "save_as" => Action::SaveAs,
            "close_tab" => Action::CloseTab,
            "exit" => Action::Exit,
            "undo" => Action::Undo,
            "redo" => Action::Redo,
            "cut" => Action::Cut,
            "copy" => Action::Copy,
            "paste" => Action::Paste,
            "delete" => Action::Delete,
            "select_all" => Action::SelectAll,
            "find" => Action::Find,
            "replace" => Action::Replace,
            "find_next" => Action::FindNext,
            "find_prev" => Action::FindPrev,
            "find_in_files" => Action::FindInFiles,
            "incremental_search" => Action::IncrementalSearch,
            "fold_toggle" => Action::FoldToggle,
            "fold_all" => Action::FoldAll,
            "unfold_all" => Action::UnfoldAll,
            "zoom_in" => Action::ZoomIn,
            "zoom_out" => Action::ZoomOut,
            "zoom_reset" => Action::ZoomReset,
            "toggle_minimap" => Action::ToggleMinimap,
            "toggle_toolbar" => Action::ToggleToolbar,
            "toggle_function_list" => Action::ToggleFunctionList,
            "folder_as_workspace" => Action::FolderAsWorkspace,
            "add_project_root" => Action::AddProjectRoot,
            "refresh_folder" => Action::RefreshFolder,
            "find_in_projects" => Action::FindInProjects,
            "complete_word" => Action::CompleteWord,
            "complete_function" => Action::CompleteFunction,
            "complete_document" => Action::CompleteDocument,
            "complete_path" => Action::CompletePath,
            "param_hint" => Action::ParamHint,
            "preferences" => Action::Preferences,
            "style_configurator" => Action::StyleConfigurator,
            "toggle_wrap" => Action::ToggleWrap,
            "toggle_theme" => Action::ToggleTheme,
            "goto_line" => Action::GotoLine,
            "toggle_bookmark" => Action::ToggleBookmark,
            "next_bookmark" => Action::NextBookmark,
            "prev_bookmark" => Action::PrevBookmark,
            "clear_bookmarks" => Action::ClearBookmarks,
            "clear_recent" => Action::ClearRecent,
            "run" => Action::Run,
            "toggle_recording" => Action::ToggleRecording,
            "play_macro" => Action::PlayMacro,
            "sort_asc" => Action::SortAsc,
            "sort_desc" => Action::SortDesc,
            "sort_numeric" => Action::SortNumeric,
            "dedupe_consecutive" => Action::DedupeConsecutive,
            "dedupe_all" => Action::DedupeAll,
            "trim_trailing" => Action::TrimTrailing,
            "trim_leading" => Action::TrimLeading,
            "remove_empty" => Action::RemoveEmpty,
            "join_lines" => Action::JoinLines,
            "split_lines" => Action::SplitLines,
            "upper_case" => Action::UpperCase,
            "lower_case" => Action::LowerCase,
            "title_case" => Action::TitleCase,
            "toggle_comment" => Action::ToggleComment,
            "tabs_to_spaces" => Action::TabsToSpaces,
            "spaces_to_tabs" => Action::SpacesToTabs,
            "duplicate_line" => Action::DuplicateLine,
            "move_line_up" => Action::MoveLineUp,
            "move_line_down" => Action::MoveLineDown,
            "next_tab" => Action::NextTab,
            "prev_tab" => Action::PrevTab,
            "document_peeker" => Action::DocumentPeeker,
            "import_notepad" => Action::ImportNotepad,
            "export_shortcuts" => Action::ExportShortcuts,
            "save_all" => Action::SaveAll,
            "print" => Action::Print,
            "print_now" => Action::PrintNow,
            "close_all" => Action::CloseAll,
            "pin_tab" => Action::PinTab,
            "tab_colour" => Action::TabColour,
            "tab_layout" => Action::TabLayout,
            "close_others" => Action::CloseOthers,
            "close_to_left" => Action::CloseToLeft,
            "close_to_right" => Action::CloseToRight,
            "reload" => Action::Reload,
            "copy_path" => Action::CopyPath,
            "open_containing_folder" => Action::OpenContainingFolder,
            "delete_line" => Action::DeleteLine,
            "select_line" => Action::SelectLine,
            "indent" => Action::Indent,
            "unindent" => Action::Unindent,
            "insert_datetime" => Action::InsertDateTime,
            "column_editor" => Action::ColumnEditor,
            "char_panel" => Action::CharPanel,
            "add_caret_above" => Action::AddCaretAbove,
            "add_caret_below" => Action::AddCaretBelow,
            "eol_windows" => Action::EolWindows,
            "eol_unix" => Action::EolUnix,
            "eol_mac" => Action::EolMac,
            "encoding_set_utf8" => Action::EncodingSet(Encoding::Utf8),
            "encoding_set_utf8bom" => Action::EncodingSet(Encoding::Utf8Bom),
            "encoding_set_utf16le" => Action::EncodingSet(Encoding::Utf16Le),
            "encoding_set_utf16be" => Action::EncodingSet(Encoding::Utf16Be),
            "encoding_set_ansi" => Action::EncodingSet(Encoding::CodePage(CodePage::Windows1252)),
            "encoding_convert_utf8" => Action::EncodingConvert(Encoding::Utf8),
            "encoding_convert_utf8bom" => Action::EncodingConvert(Encoding::Utf8Bom),
            "encoding_convert_utf16le" => Action::EncodingConvert(Encoding::Utf16Le),
            "encoding_convert_utf16be" => Action::EncodingConvert(Encoding::Utf16Be),
            "encoding_convert_ansi" => {
                Action::EncodingConvert(Encoding::CodePage(CodePage::Windows1252))
            }
            "full_screen" => Action::FullScreen,
            "distraction_free" => Action::DistractionFree,
            "playground" => Action::Playground,
            "change_history" => Action::ChangeHistory,
            "read_only" => Action::ReadOnly,
            "show_whitespace" => Action::ShowWhitespace,
            "show_eol" => Action::ShowEol,
            "show_indent_guides" => Action::ShowIndentGuides,
            "mark_all" => Action::MarkAll,
            "goto_brace" => Action::GotoBrace,
            "split_vertical" => Action::SplitVertical,
            "split_horizontal" => Action::SplitHorizontal,
            "unsplit" => Action::Unsplit,
            "rotate_split" => Action::RotateSplit,
            "move_to_other_view" => Action::MoveToOtherView,
            "clone_to_other_view" => Action::CloneToOtherView,
            "about" => Action::About,
            "open_in_default_viewer" => Action::OpenInDefaultViewer,
            "open_folder_as_workspace" => Action::OpenFolderAsWorkspace,
            "save_copy_as" => Action::SaveCopyAs,
            "rename" => Action::Rename,
            "close_all_but_pinned" => Action::CloseAllButPinned,
            "close_all_unchanged" => Action::CloseAllUnchanged,
            "move_to_trash" => Action::MoveToTrash,
            "load_session" => Action::LoadSession,
            "save_session" => Action::SaveSession,
            "open_containing_folder_cmd" => Action::OpenContainingFolderCmd,
            "open_containing_folder_powershell" => Action::OpenContainingFolderPowerShell,
            "begin_end_select" => Action::BeginEndSelect,
            "begin_end_select_column" => Action::BeginEndSelectColumn,
            "insert_datetime_short" => Action::InsertDateTimeShort,
            "insert_datetime_long" => Action::InsertDateTimeLong,
            "insert_datetime_custom" => Action::InsertDateTimeCustom,
            "copy_filename" => Action::CopyFilename,
            "copy_dir_path" => Action::CopyDirPath,
            "copy_all_filenames" => Action::CopyAllFilenames,
            "copy_all_file_paths" => Action::CopyAllFilePaths,
            "proper_case" => Action::ProperCaseForce,
            "proper_case_blend" => Action::ProperCaseBlend,
            "sentence_case" => Action::SentenceCaseForce,
            "sentence_case_blend" => Action::SentenceCaseBlend,
            "invert_case" => Action::InvertCase,
            "random_case" => Action::RandomCase,
            "remove_empty_with_blank" => Action::RemoveEmptyWithBlank,
            "insert_blank_above" => Action::InsertBlankAbove,
            "insert_blank_below" => Action::InsertBlankBelow,
            "reverse_line_order" => Action::ReverseLineOrder,
            "randomize_line_order" => Action::RandomizeLineOrder,
            "single_line_comment" => Action::SingleLineComment,
            "single_line_uncomment" => Action::SingleLineUncomment,
            "block_comment" => Action::BlockComment,
            "block_uncomment" => Action::BlockUncomment,
            "param_hint_prev" => Action::ParamHintPrev,
            "param_hint_next" => Action::ParamHintNext,
            "trim_both" => Action::TrimBoth,
            "eol_to_space" => Action::EolToSpace,
            "trim_all" => Action::TrimAll,
            "spaces_to_tabs_all" => Action::SpacesToTabsAll,
            "spaces_to_tabs_leading" => Action::SpacesToTabsLeading,
            "open_selected_file" => Action::OpenSelectedFile,
            "open_selected_folder" => Action::OpenSelectedFolder,
            "redact_selection" => Action::RedactSelection,
            "search_on_internet" => Action::SearchOnInternet,
            "change_search_engine" => Action::ChangeSearchEngine,
            "multi_select_undo" => Action::MultiSelectUndo,
            "multi_select_skip" => Action::MultiSelectSkip,
            "column_mode" => Action::ColumnMode,
            "clipboard_history" => Action::ClipboardHistory,
            // The index is not part of the name: a binding pastes the newest
            // entry, which is what a keyboard user wants from a shortcut.
            "paste_clipboard" => Action::PasteClipboard(0),
            "read_only_current" => Action::ReadOnlyCurrent,
            "read_only_all" => Action::ReadOnlyAll,
            "clear_read_only_all" => Action::ClearReadOnlyAll,
            "hash_md5" => Action::HashGenerate(nn_core::hash::Hash::Md5),
            "hash_sha1" => Action::HashGenerate(nn_core::hash::Hash::Sha1),
            "hash_sha256" => Action::HashGenerate(nn_core::hash::Hash::Sha256),
            "hash_sha512" => Action::HashGenerate(nn_core::hash::Hash::Sha512),
            "hash_file_md5" => Action::HashFromFile(nn_core::hash::Hash::Md5),
            "hash_file_sha1" => Action::HashFromFile(nn_core::hash::Hash::Sha1),
            "hash_file_sha256" => Action::HashFromFile(nn_core::hash::Hash::Sha256),
            "hash_file_sha512" => Action::HashFromFile(nn_core::hash::Hash::Sha512),
            "hash_clipboard_md5" => Action::HashToClipboard(nn_core::hash::Hash::Md5),
            "hash_clipboard_sha1" => Action::HashToClipboard(nn_core::hash::Hash::Sha1),
            "hash_clipboard_sha256" => Action::HashToClipboard(nn_core::hash::Hash::Sha256),
            "hash_clipboard_sha512" => Action::HashToClipboard(nn_core::hash::Hash::Sha512),
            "always_on_top" => Action::AlwaysOnTop,
            "post_it" => Action::PostIt,
            "show_non_printing" => Action::ShowNonPrinting,
            "show_control_chars" => Action::ShowControlChars,
            "show_all_chars" => Action::ShowAllChars,
            "show_wrap_symbol" => Action::ShowWrapSymbol,
            "zoom_sync" => Action::ZoomSync,
            "move_to_new_instance" => Action::MoveToNewInstance,
            "open_in_new_instance" => Action::OpenInNewInstance,
            "first_tab" => Action::FirstTab,
            "last_tab" => Action::LastTab,
            "move_tab_start" => Action::MoveTabStart,
            "move_tab_end" => Action::MoveTabEnd,
            "move_tab_forward" => Action::MoveTabForward,
            "move_tab_backward" => Action::MoveTabBackward,
            "focus_other_view" => Action::FocusOtherView,
            "hide_lines" => Action::HideLines,
            "fold_current_level" => Action::FoldCurrentLevel,
            "unfold_current_level" => Action::UnfoldCurrentLevel,
            "summary" => Action::Summary,
            "document_map" => Action::DocumentMap,
            "document_list" => Action::DocumentList,
            "sync_scroll_v" => Action::SyncScrollV,
            "sync_scroll_h" => Action::SyncScrollH,
            "monitoring" => Action::Monitoring,
            "goto_tab_1" => Action::GotoTab(0),
            "goto_tab_2" => Action::GotoTab(1),
            "goto_tab_3" => Action::GotoTab(2),
            "goto_tab_4" => Action::GotoTab(3),
            "goto_tab_5" => Action::GotoTab(4),
            "goto_tab_6" => Action::GotoTab(5),
            "goto_tab_7" => Action::GotoTab(6),
            "goto_tab_8" => Action::GotoTab(7),
            "goto_tab_9" => Action::GotoTab(8),
            "fold_level_1" => Action::FoldLevel(1),
            "fold_level_2" => Action::FoldLevel(2),
            "fold_level_3" => Action::FoldLevel(3),
            "fold_level_4" => Action::FoldLevel(4),
            "fold_level_5" => Action::FoldLevel(5),
            "fold_level_6" => Action::FoldLevel(6),
            "fold_level_7" => Action::FoldLevel(7),
            "fold_level_8" => Action::FoldLevel(8),
            "unfold_level_1" => Action::UnfoldLevel(1),
            "unfold_level_2" => Action::UnfoldLevel(2),
            "unfold_level_3" => Action::UnfoldLevel(3),
            "unfold_level_4" => Action::UnfoldLevel(4),
            "unfold_level_5" => Action::UnfoldLevel(5),
            "unfold_level_6" => Action::UnfoldLevel(6),
            "unfold_level_7" => Action::UnfoldLevel(7),
            "unfold_level_8" => Action::UnfoldLevel(8),
            "tab_colour_1" => Action::TabColourSet(Some(0)),
            "tab_colour_2" => Action::TabColourSet(Some(1)),
            "tab_colour_3" => Action::TabColourSet(Some(2)),
            "tab_colour_4" => Action::TabColourSet(Some(3)),
            "tab_colour_5" => Action::TabColourSet(Some(4)),
            "tab_colour_none" => Action::TabColourSet(None),
            "view_in_Firefox" => Action::ViewInBrowser(BrowserKind::Firefox),
            "view_in_Chrome" => Action::ViewInBrowser(BrowserKind::Chrome),
            "view_in_Edge" => Action::ViewInBrowser(BrowserKind::Edge),
            "view_in_Ie" => Action::ViewInBrowser(BrowserKind::Ie),
            "select_and_find_next" => Action::SelectAndFindNext,
            "select_and_find_prev" => Action::SelectAndFindPrev,
            "volatile_find_next" => Action::VolatileFindNext,
            "volatile_find_prev" => Action::VolatileFindPrev,
            "search_results_window" => Action::SearchResultsWindow,
            "next_search_result" => Action::NextSearchResult,
            "prev_search_result" => Action::PrevSearchResult,
            "select_between_braces" => Action::SelectBetweenBraces,
            "mark" => Action::Mark,
            "change_next" => Action::ChangeNext,
            "change_prev" => Action::ChangePrev,
            "clear_change_history" => Action::ClearChangeHistory,
            "style_all_1" => Action::StyleAll(0),
            "style_all_2" => Action::StyleAll(1),
            "style_all_3" => Action::StyleAll(2),
            "style_all_4" => Action::StyleAll(3),
            "style_all_5" => Action::StyleAll(4),
            "style_one_1" => Action::StyleOne(0),
            "style_one_2" => Action::StyleOne(1),
            "style_one_3" => Action::StyleOne(2),
            "style_one_4" => Action::StyleOne(3),
            "style_one_5" => Action::StyleOne(4),
            "clear_style_1" => Action::ClearStyle(0),
            "clear_style_2" => Action::ClearStyle(1),
            "clear_style_3" => Action::ClearStyle(2),
            "clear_style_4" => Action::ClearStyle(3),
            "clear_style_5" => Action::ClearStyle(4),
            "clear_all_styles" => Action::ClearAllStyles,
            "jump_up_1" => Action::JumpUp(Some(0)),
            "jump_up_2" => Action::JumpUp(Some(1)),
            "jump_up_3" => Action::JumpUp(Some(2)),
            "jump_up_4" => Action::JumpUp(Some(3)),
            "jump_up_5" => Action::JumpUp(Some(4)),
            "jump_up_mark" => Action::JumpUp(None),
            "jump_down_1" => Action::JumpDown(Some(0)),
            "jump_down_2" => Action::JumpDown(Some(1)),
            "jump_down_3" => Action::JumpDown(Some(2)),
            "jump_down_4" => Action::JumpDown(Some(3)),
            "jump_down_5" => Action::JumpDown(Some(4)),
            "jump_down_mark" => Action::JumpDown(None),
            "copy_style_1" => Action::CopyStyled(Some(0)),
            "copy_style_2" => Action::CopyStyled(Some(1)),
            "copy_style_3" => Action::CopyStyled(Some(2)),
            "copy_style_4" => Action::CopyStyled(Some(3)),
            "copy_style_5" => Action::CopyStyled(Some(4)),
            "copy_all_styles" => Action::CopyStyled(None),
            "cut_bookmarked_lines" => Action::CutBookmarkedLines,
            "copy_bookmarked_lines" => Action::CopyBookmarkedLines,
            "paste_bookmarked_lines" => Action::PasteToBookmarkedLines,
            "remove_bookmarked_lines" => Action::RemoveBookmarkedLines,
            "remove_non_bookmarked_lines" => Action::RemoveNonBookmarkedLines,
            "inverse_bookmarks" => Action::InverseBookmarks,
            "find_char_in_range" => Action::FindCharInRange,
            "manage_plugins" => Action::ManagePlugins,
            "open_plugins_folder" => Action::OpenPluginsFolder,
            "reload_plugins" => Action::ReloadPlugins,
            "validate_shortcuts" => Action::ValidateShortcuts,
            "stop_recording" => Action::StopRecording,
            "save_macro" => Action::SaveMacro,
            "run_macro_multiple" => Action::RunMacroMultiple,
            "windows_list" => Action::WindowsList,
            "recent_window" => Action::RecentWindow,
            "command_line_args" => Action::CommandLineArgs,
            "home_page" => Action::HomePage,
            "project_page" => Action::ProjectPage,
            "online_manual" => Action::OnlineManual,
            "forum" => Action::Forum,
            "license" => Action::License,
            "debug_info" => Action::DebugInfo,
            other => {
                if let Some(key) = other.strip_prefix("encoding_set_") {
                    return encoding_from_key(key).map(Action::EncodingSet);
                }
                if let Some(key) = other.strip_prefix("encoding_convert_") {
                    return encoding_from_key(key).map(Action::EncodingConvert);
                }
                if let Some(key) = other.strip_prefix("language_built_in_") {
                    return nn_core::lang_by_key(key).map(Action::LanguageBuiltIn);
                }
                if let Some(key) = other.strip_prefix("theme_") {
                    return ThemeChoice::ALL
                        .iter()
                        .position(|choice| choice.key() == key)
                        .map(Action::Theme);
                }
                return None;
            }
        };
        Some(action)
    }

    /// The label the shortcut editor draws for this command.
    ///
    /// Derived from [`Action::name`] rather than written out again: a second
    /// table would be one more thing to keep in step, and the names are already
    /// the words a user would look for.
    pub fn label(self) -> String {
        let mut out = String::new();
        for (index, word) in self.name().split('_').enumerate() {
            if index > 0 {
                out.push(' ');
            }
            let mut chars = word.chars();
            if let Some(first) = chars.next() {
                out.push(first.to_ascii_uppercase());
                out.extend(chars);
            }
        }
        out
    }

    /// The commands whose whole effect is a function of the text they are
    /// given, and which are therefore run by `nn_core::ops` rather than by the
    /// dispatcher here.
    ///
    /// The list is what keeps the two crates honest about each other: the test
    /// below requires it to be exactly the names in `ops::COMMANDS`, so a
    /// command added to the table but forgotten here would be unreachable from
    /// the menu, and one added here but not to the table would not compile
    /// without a matching arm.
    ///
    /// `DuplicateLine`, `MoveLineUp` and `DeleteLine` are deliberately absent
    /// even though a block of text could be passed to them: they act on the line
    /// the caret is on, and a caller with no caret is asking a different
    /// question.
    pub const TEXT_COMMANDS: &'static [Action] = &[
        Action::SortAsc,
        Action::SortDesc,
        Action::SortNumeric,
        Action::DedupeConsecutive,
        Action::DedupeAll,
        Action::ReverseLineOrder,
        Action::RandomizeLineOrder,
        Action::JoinLines,
        Action::RemoveEmpty,
        Action::RemoveEmptyWithBlank,
        Action::TrimTrailing,
        Action::TrimLeading,
        Action::TrimBoth,
        Action::TrimAll,
        Action::EolToSpace,
        Action::UpperCase,
        Action::LowerCase,
        Action::TitleCase,
        Action::ProperCaseForce,
        Action::ProperCaseBlend,
        Action::SentenceCaseForce,
        Action::SentenceCaseBlend,
        Action::InvertCase,
        Action::RandomCase,
        Action::ToggleComment,
        Action::SingleLineComment,
        Action::SingleLineUncomment,
        Action::BlockComment,
        Action::BlockUncomment,
        Action::TabsToSpaces,
        Action::SpacesToTabs,
        Action::SpacesToTabsAll,
        Action::SpacesToTabsLeading,
        Action::Indent,
        Action::Unindent,
        Action::SortLines(Sort::LexAsc),
    ];

    /// Whether this command is run by the core's text-command table.
    pub fn is_text_command(self) -> bool {
        // `SortLines` carries a direction and a key, so it never compares equal
        // to the default it is listed with: the name is the question, and every
        // `SortLines` is one command as far as the table is concerned.
        if matches!(self, Action::SortLines(_)) {
            return true;
        }
        Self::TEXT_COMMANDS.contains(&self)
    }

    /// Every command the shortcut editor offers, in menu order.
    ///
    /// `OpenRecent` is absent: it carries an index, and a key that opened "the
    /// third recent file" would mean something different every day.
    pub const BINDABLE: &'static [Action] = &[
        Action::New,
        Action::Open,
        Action::Save,
        Action::SaveAs,
        Action::CloseTab,
        Action::Exit,
        Action::Undo,
        Action::Redo,
        Action::Cut,
        Action::Copy,
        Action::Paste,
        Action::Delete,
        Action::SelectAll,
        Action::Find,
        Action::Replace,
        Action::FindNext,
        Action::FindPrev,
        Action::FindInFiles,
        Action::IncrementalSearch,
        Action::FoldToggle,
        Action::FoldAll,
        Action::UnfoldAll,
        Action::ZoomIn,
        Action::ZoomOut,
        Action::ZoomReset,
        Action::ToggleMinimap,
        Action::ToggleToolbar,
        Action::ToggleFunctionList,
        Action::FolderAsWorkspace,
        Action::AddProjectRoot,
        Action::RefreshFolder,
        Action::FindInProjects,
        Action::CompleteWord,
        Action::CompleteFunction,
        Action::CompleteDocument,
        Action::CompletePath,
        Action::ParamHint,
        Action::Preferences,
        Action::StyleConfigurator,
        Action::ToggleWrap,
        Action::ToggleTheme,
        Action::GotoLine,
        Action::ToggleBookmark,
        Action::NextBookmark,
        Action::PrevBookmark,
        Action::ClearBookmarks,
        Action::ClearRecent,
        Action::Run,
        Action::ToggleRecording,
        Action::PlayMacro,
        Action::SortAsc,
        Action::SortDesc,
        Action::SortNumeric,
        Action::DedupeConsecutive,
        Action::DedupeAll,
        Action::TrimTrailing,
        Action::TrimLeading,
        Action::RemoveEmpty,
        Action::JoinLines,
        Action::SplitLines,
        Action::UpperCase,
        Action::LowerCase,
        Action::TitleCase,
        Action::ToggleComment,
        Action::TabsToSpaces,
        Action::SpacesToTabs,
        Action::DuplicateLine,
        Action::MoveLineUp,
        Action::MoveLineDown,
        Action::NextTab,
        Action::PrevTab,
        Action::DocumentPeeker,
        Action::ImportNotepad,
        Action::ExportShortcuts,
        Action::SaveAll,
        Action::Print,
        Action::PrintNow,
        Action::CloseAll,
        Action::PinTab,
        Action::TabColour,
        Action::TabLayout,
        Action::CloseOthers,
        Action::CloseToLeft,
        Action::CloseToRight,
        Action::Reload,
        Action::CopyPath,
        Action::OpenContainingFolder,
        Action::DeleteLine,
        Action::SelectLine,
        Action::Indent,
        Action::Unindent,
        Action::InsertDateTime,
        Action::ColumnEditor,
        Action::CharPanel,
        Action::AddCaretAbove,
        Action::AddCaretBelow,
        Action::EolWindows,
        Action::EolUnix,
        Action::EolMac,
        Action::FullScreen,
        Action::DistractionFree,
        Action::Playground,
        Action::ChangeHistory,
        Action::ReadOnly,
        Action::ShowWhitespace,
        Action::ShowEol,
        Action::ShowIndentGuides,
        Action::MarkAll,
        Action::GotoBrace,
        Action::SplitVertical,
        Action::SplitHorizontal,
        Action::Unsplit,
        Action::RotateSplit,
        Action::MoveToOtherView,
        Action::CloneToOtherView,
        Action::ManagePlugins,
        Action::ReloadPlugins,
        Action::About,
    ];
}

/// One plugin, as the Plugins menu shows it.
///
/// The caller builds this from the inventory, in inventory order, so a plugin's
/// position in the slice is the index [`Action::Plugin`] carries and the index
/// the plugin host runs a command by. Nothing here decides whether a plugin is
/// running: `enabled` is the answer, and a plugin that is off is a row in the
/// manager rather than a row in the dropdown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PluginMenu {
    /// The name the plugin gives itself, in its own words.
    pub name: String,
    /// Whether it is running.
    pub enabled: bool,
    /// Its commands, in the order it lists them, in its own words.
    pub commands: Vec<String>,
}

/// One row of a dropdown.
///
/// Labels are owned strings because the File menu carries the recent file list,
/// which changes while the program runs. The shortcut column is not stored here
/// at construction: it is filled from the keymap, so the menu cannot advertise a
/// chord the editor does not honour.
struct Item {
    label: String,
    /// The untranslated label.
    ///
    /// Kept beside the displayed one so a language change relabels the row
    /// without rebuilding the menu, and so the "did the list change?" checks in
    /// `set_recent` and friends compare like with like whatever the language.
    english: String,
    shortcut: String,
    action: Action,
    /// Draws a separator line above this row instead of a label.
    separator: bool,
    /// Whether the label is a word of its own that no catalog translates.
    ///
    /// A plugin's name and the names of its commands are the plugin's text. A
    /// catalog that happens to hold the same string -- "Cancel", say -- would
    /// otherwise rewrite a row into a command the plugin does not have.
    literal: bool,
    /// The rows this row opens, empty for a command.
    ///
    /// A submenu is an `Item` with children rather than a second type, because
    /// every row is laid out, hit-tested, translated and drawn the same way. The
    /// only difference is what a click does, and that is one branch.
    children: Vec<Item>,
}

impl Item {
    fn new(label: impl Into<String>, action: Action) -> Self {
        let english = label.into();
        Self {
            label: english.clone(),
            english,
            shortcut: String::new(),
            action,
            separator: false,
            literal: false,
            children: Vec::new(),
        }
    }

    fn separator() -> Self {
        Self {
            label: String::new(),
            english: String::new(),
            shortcut: String::new(),
            action: Action::About,
            separator: true,
            literal: false,
            children: Vec::new(),
        }
    }

    /// A row whose label was built translated rather than left for `relabel`.
    ///
    /// The Plugins menu is built from the catalog and the folder rather than
    /// relabelled in place, because its rows arrive and leave with the disk. The
    /// English text is stored beside the label anyway: `same_rows` compares it,
    /// and a rebuild that changed nothing is skipped.
    fn composed(english: String, label: String, action: Action) -> Self {
        Self {
            label,
            english,
            shortcut: String::new(),
            action,
            separator: false,
            literal: false,
            children: Vec::new(),
        }
    }

    /// A row that opens `children` instead of running a command.
    fn submenu(label: impl Into<String>, children: Vec<Item>) -> Self {
        let english = label.into();
        Self {
            label: english.clone(),
            english,
            shortcut: String::new(),
            action: Action::Submenu,
            separator: false,
            literal: false,
            children,
        }
    }

    /// A row whose label is the plugin's own text, in every language.
    ///
    /// The English text is the label: there is no other spelling of it to keep,
    /// and comparing the two is how `same_rows` notices that a rebuilt plugin
    /// renamed a command.
    fn literal(label: impl Into<String>, action: Action) -> Self {
        let label = label.into();
        Self {
            label: label.clone(),
            english: label,
            shortcut: String::new(),
            action,
            separator: false,
            literal: true,
            children: Vec::new(),
        }
    }

    /// A submenu whose title is the plugin's own text.
    fn literal_submenu(label: impl Into<String>, children: Vec<Item>) -> Self {
        let label = label.into();
        Self {
            label: label.clone(),
            english: label,
            shortcut: String::new(),
            action: Action::Submenu,
            separator: false,
            literal: true,
            children,
        }
    }

    /// Whether this row opens a submenu.
    fn is_submenu(&self) -> bool {
        !self.children.is_empty()
    }
}

/// Where one dropdown panel sits on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Panel {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
}

impl Panel {
    /// Whether a point is inside the panel.
    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.left && x < self.left + self.width && y >= self.top && y < self.top + self.height
    }
}

/// One dropdown row, shaped.
///
/// Mirrors `Item` so a submenu's labels are shaped once, at `prepare`, rather
/// than every frame. The tree is rebuilt whenever the labels change, which is
/// what `shaped` already gates.
struct Shaped {
    label: Buffer,
    shortcut: Buffer,
    children: Vec<Shaped>,
}

/// One top-level menu.
struct Menu {
    title: String,
    /// The untranslated title, for the same reason `Item::english` exists.
    english: &'static str,
    items: Vec<Item>,
}

/// One top-level menu, as something outside the menu needs to see it.
pub struct Row {
    /// The title shown in the bar, already translated.
    pub title: String,
    /// The dropdown's rows, in order.
    pub items: Vec<RowItem>,
}

/// One dropdown row, as something outside the menu needs to see it.
pub struct RowItem {
    /// The label shown in the row, already translated.
    pub label: String,
    /// The chord shown in the shortcut column, empty when there is none.
    pub shortcut: String,
    /// What the row runs.
    pub action: Action,
    /// Whether the row is a separator rather than a command.
    pub separator: bool,
    /// The rows this row opens, empty for a command.
    ///
    /// Exposed so the accessibility tree can describe a submenu as a submenu
    /// rather than as a command that does nothing.
    pub children: Vec<RowItem>,
}

/// The horizontal bar and its dropdowns.
pub struct MenuBar {
    menus: Vec<Menu>,
    /// Index of the open menu, if any.
    open: Option<usize>,
    /// Index of the highlighted row in the open menu.
    hover: Option<usize>,
    /// The chords shown in the shortcut column.
    keymap: Keymap,
    metrics: Metrics,
    family: Family<'static>,
    char_width: f32,
    /// Shaped top-level titles, one per menu.
    titles: Vec<Buffer>,
    /// How wide each title is, in pixels.
    ///
    /// Measured from the shaped buffer, not counted in characters: a title
    /// translated into a script whose glyphs are twice a cell wide -- Japanese,
    /// Korean, Chinese -- would otherwise be placed on top of the menu after it.
    title_widths: Vec<f32>,
    /// Shaped dropdown rows, mirroring the item tree.
    shaped_rows: Vec<Vec<Shaped>>,
    /// The arrow drawn on a row that opens a submenu.
    arrow: Buffer,
    /// The chain of open submenus, outermost first.
    ///
    /// Each entry is a row index in the level above it: `path[0]` is a row of the
    /// top dropdown, `path[1]` a row of that row's submenu, and so on. Empty means
    /// only the top dropdown is showing. A `Vec` rather than a fixed depth because
    /// the depth is whatever the menu happens to be, and a menu tree is data.
    path: Vec<usize>,
    /// The active localization, if one was loaded.
    catalog: Option<Catalog>,
    /// The plugins the Plugins menu was last built from.
    ///
    /// Kept rather than counted, because a language change rebuilds the menu and
    /// can only do that from what it was built from. The slice is in inventory
    /// order: its positions are the plugin indices the rows carry.
    plugins: Vec<PluginMenu>,
    shaped: bool,
}

/// Padding either side of a top-level title.
const TITLE_PADDING: f32 = 10.0;
/// Padding either side of a dropdown row.
const ROW_PADDING: f32 = 12.0;
/// Extra height of a dropdown row over the line height.
const ROW_EXTRA: f32 = 4.0;
/// Gap between a label and its shortcut column.
const COLUMN_GAP: f32 = 24.0;
/// Width reserved at the right of a row for the submenu arrow.
const ARROW_COLUMN: f32 = 14.0;
/// How far a submenu overlaps the panel it came from.
const PANEL_OVERLAP: f32 = 4.0;
/// The glyph on a row that opens a submenu.
///
/// U+25B8, which Noto Sans Mono has. A `>` would have been safe in any font and
/// would have looked like a typo.
const SUBMENU_ARROW: &str = "\u{25b8}";

/// The index of the Run menu in the bar.
///
/// Fixed, because the bar's order is fixed.
const RUN_MENU: usize = 9;

/// The index of the Language menu in the bar.
const LANGUAGE_MENU: usize = 5;

/// The index of the Plugins menu in the bar.
///
/// The bar is Notepad++'s own thirteen, in its order: File, Edit, Search, View,
/// Encoding, Language, Settings, Tools, Macro, Run, Plugins, Window, Help. The
/// indices below are the ones that have to be reached by position, so they are
/// named rather than written as literals at each use.
const PLUGIN_MENU: usize = 10;

/// The encoding a saved shortcut's key names.
///
/// The forty-one code pages are matched here rather than as ninety-two literal
/// arms in `from_name`, because the key is the code page's own key and a second
/// copy of the list would be a second thing to keep in step.
fn encoding_from_key(key: &str) -> Option<Encoding> {
    let unicode = match key {
        "utf8" => Some(Encoding::Utf8),
        "utf8bom" => Some(Encoding::Utf8Bom),
        "utf16le" => Some(Encoding::Utf16Le),
        "utf16be" => Some(Encoding::Utf16Be),
        // "ANSI" is Windows-1252 on every Windows Notepad++ runs on, and the
        // label is kept because the menu is meant to be recognisable.
        "ansi" => Some(Encoding::CodePage(CodePage::Windows1252)),
        _ => None,
    };
    if unicode.is_some() {
        return unicode;
    }
    CodePage::ALL
        .into_iter()
        .find(|page| page.key() == key)
        .map(Encoding::CodePage)
}

/// The Character sets submenu: Notepad++'s scripts, each with the sets that
/// belong to it.
///
/// Grouped by script rather than listed flat because forty-one rows in one
/// column is a list you read rather than scan, and the script is how a user
/// knows which one they want. Chinese, Japanese and Korean are absent: their
/// sets are multi-byte, and `codepage` explains why those are not here.
const CHARACTER_SETS: [(&str, &[CodePage]); 13] = [
    (
        "Arabic",
        &[CodePage::Iso8859_6, CodePage::Oem720, CodePage::Windows1256],
    ),
    (
        "Baltic",
        &[
            CodePage::Iso8859_4,
            CodePage::Iso8859_13,
            CodePage::Oem775,
            CodePage::Windows1257,
        ],
    ),
    ("Celtic", &[CodePage::Iso8859_14]),
    (
        "Cyrillic",
        &[
            CodePage::Iso8859_5,
            CodePage::Koi8R,
            CodePage::Koi8U,
            CodePage::MacCyrillic,
            CodePage::Oem855,
            CodePage::Oem866,
            CodePage::Windows1251,
        ],
    ),
    (
        "Central European",
        &[CodePage::Oem852, CodePage::Windows1250],
    ),
    ("Eastern European", &[CodePage::Iso8859_2]),
    (
        "Greek",
        &[
            CodePage::Iso8859_7,
            CodePage::Oem737,
            CodePage::Oem869,
            CodePage::Windows1253,
        ],
    ),
    (
        "Hebrew",
        &[
            CodePage::Iso8859_8,
            CodePage::Oem862,
            CodePage::Windows1255,
        ],
    ),
    ("North European", &[CodePage::Oem861, CodePage::Oem865]),
    ("Thai", &[CodePage::Tis620]),
    (
        "Turkish",
        &[
            CodePage::Iso8859_3,
            CodePage::Iso8859_9,
            CodePage::Oem857,
            CodePage::Windows1254,
        ],
    ),
    (
        "Western European",
        &[
            CodePage::Iso8859_1,
            CodePage::Iso8859_15,
            CodePage::Oem850,
            CodePage::Oem858,
            CodePage::Oem860,
            CodePage::Oem863,
            CodePage::Oem437,
            CodePage::Windows1252,
        ],
    ),
    ("Vietnamese", &[CodePage::Windows1258]),
];

/// The rows of the Encoding menu.
///
/// Two halves with different meanings. The top rows reinterpret the bytes
/// already on disk -- the file is unchanged and the same bytes are read again
/// under a different assumption, which is the only way to recover a file whose
/// encoding was guessed wrong. The "Convert to" rows re-encode the text in
/// memory, so the next save writes different bytes.
fn encoding_items() -> Vec<Item> {
    let mut items = vec![
        Item::new(
            "ANSI",
            Action::EncodingSet(Encoding::CodePage(CodePage::Windows1252)),
        ),
        Item::new("UTF-8", Action::EncodingSet(Encoding::Utf8)),
        Item::new("UTF-8-BOM", Action::EncodingSet(Encoding::Utf8Bom)),
        Item::new("UTF-16 BE BOM", Action::EncodingSet(Encoding::Utf16Be)),
        Item::new("UTF-16 LE BOM", Action::EncodingSet(Encoding::Utf16Le)),
        Item::submenu(
            "Character sets",
            CHARACTER_SETS
                .iter()
                .map(|(script, pages)| {
                    Item::submenu(
                        *script,
                        pages
                            .iter()
                            .map(|page| {
                                Item::new(
                                    page.name(),
                                    Action::EncodingSet(Encoding::CodePage(*page)),
                                )
                            })
                            .collect(),
                    )
                })
                .collect(),
        ),
        Item::separator(),
    ];
    for (label, encoding) in [
        ("Convert to ANSI", Encoding::CodePage(CodePage::Windows1252)),
        ("Convert to UTF-8", Encoding::Utf8),
        ("Convert to UTF-8-BOM", Encoding::Utf8Bom),
        ("Convert to UTF-16 BE BOM", Encoding::Utf16Be),
        ("Convert to UTF-16 LE BOM", Encoding::Utf16Le),
    ] {
        items.push(Item::new(label, Action::EncodingConvert(encoding)));
    }
    items
}

/// The rows of the View menu.
///
/// The longest menu in the bar, and the one with the most submenus: five of them
/// before the first flat row after "Hide Lines". The order is Notepad++'s.
///
/// Two rows are absent on purpose. "Text Direction RTL" and "Text Direction LTR"
/// would set a flag the renderer cannot honour: lines are shaped by cosmic-text
/// and the caret's x is `column * char_width`, so a right-to-left run would put
/// the caret on the wrong side of the text. A row that appears to work and does
/// not is worse than a row that is not there.
fn view_items() -> Vec<Item> {
    let tabs = ["1st Tab", "2nd Tab", "3rd Tab", "4th Tab", "5th Tab", "6th Tab", "7th Tab",
        "8th Tab", "9th Tab"];
    let mut tab_rows: Vec<Item> = tabs
        .iter()
        .enumerate()
        .map(|(index, label)| Item::new(*label, Action::GotoTab(index)))
        .collect();
    tab_rows.push(Item::separator());
    tab_rows.extend([
        Item::new("First Tab", Action::FirstTab),
        Item::new("Last Tab", Action::LastTab),
        Item::new("Next Tab", Action::NextTab),
        Item::new("Previous Tab", Action::PrevTab),
        Item::separator(),
        Item::new("Move to Start", Action::MoveTabStart),
        Item::new("Move to End", Action::MoveTabEnd),
        Item::new("Move Tab Forward", Action::MoveTabForward),
        Item::new("Move Tab Backward", Action::MoveTabBackward),
        Item::separator(),
    ]);
    for index in 0..5 {
        tab_rows.push(Item::new(
            format!("Apply Color {}", index + 1),
            Action::TabColourSet(Some(index)),
        ));
    }
    tab_rows.push(Item::new("Remove Color", Action::TabColourSet(None)));

    let levels = |make: &dyn Fn(usize) -> Action| -> Vec<Item> {
        (1..=8)
            .map(|level| Item::new(level.to_string(), make(level)))
            .collect()
    };

    vec![
        Item::new("Always on Top", Action::AlwaysOnTop),
        Item::new("Toggle Full Screen Mode", Action::FullScreen),
        Item::new("Post-It", Action::PostIt),
        Item::new("Distraction Free Mode", Action::DistractionFree),
        Item::new("Playground", Action::Playground),
        Item::separator(),
        Item::submenu(
            "View Current File in",
            BrowserKind::all()
                .into_iter()
                .map(|kind| Item::new(kind.label(), Action::ViewInBrowser(kind)))
                .collect(),
        ),
        Item::separator(),
        Item::submenu(
            "Show Symbol",
            vec![
                Item::new("Show Space and Tab", Action::ShowWhitespace),
                Item::new("Show End of Line", Action::ShowEol),
                Item::new("Show Non-Printing Characters", Action::ShowNonPrinting),
                Item::new(
                    "Show Control Characters & Unicode EOL",
                    Action::ShowControlChars,
                ),
                Item::new("Show All Characters", Action::ShowAllChars),
                Item::separator(),
                Item::new("Show Indent Guide", Action::ShowIndentGuides),
                Item::new("Show Wrap Symbol", Action::ShowWrapSymbol),
            ],
        ),
        Item::submenu(
            "Zoom",
            vec![
                Item::new("Zoom In (Ctrl+Mouse Wheel Up)", Action::ZoomIn),
                Item::new("Zoom Out (Ctrl+Mouse Wheel Down)", Action::ZoomOut),
                Item::new("Restore Default Zoom", Action::ZoomReset),
                Item::separator(),
                Item::new("Synchronize Across Views", Action::ZoomSync),
            ],
        ),
        Item::submenu(
            "Move/Clone Current Document",
            vec![
                Item::new("Move to Other View", Action::MoveToOtherView),
                Item::new("Clone to Other View", Action::CloneToOtherView),
                Item::new("Move to New Instance", Action::MoveToNewInstance),
                Item::new("Open in New Instance", Action::OpenInNewInstance),
            ],
        ),
        Item::submenu("Tab", tab_rows),
        Item::new("Word wrap", Action::ToggleWrap),
        Item::new("Focus on Another View", Action::FocusOtherView),
        Item::new("Hide Lines", Action::HideLines),
        Item::separator(),
        Item::new("Fold All", Action::FoldAll),
        Item::new("Unfold All", Action::UnfoldAll),
        Item::new("Fold Current Level", Action::FoldCurrentLevel),
        Item::new("Unfold Current Level", Action::UnfoldCurrentLevel),
        Item::submenu("Fold Level", levels(&|level| Action::FoldLevel(level))),
        Item::submenu("Unfold Level", levels(&|level| Action::UnfoldLevel(level))),
        Item::separator(),
        Item::new("Summary...", Action::Summary),
        Item::separator(),
        Item::submenu(
            "Project Panels",
            (0..3)
                .map(|index| {
                    Item::new(
                        format!("Project Panel {}", index + 1),
                        Action::ProjectPanel(index),
                    )
                })
                .collect(),
        ),
        Item::new("Folder as Workspace", Action::FolderAsWorkspace),
        Item::new("Document Map", Action::DocumentMap),
        Item::new("Document List", Action::DocumentList),
        Item::new("Function List", Action::ToggleFunctionList),
        Item::separator(),
        Item::new("Synchronize Vertical Scrolling", Action::SyncScrollV),
        Item::new("Synchronize Horizontal Scrolling", Action::SyncScrollH),
        Item::separator(),
        Item::new("Monitoring (tail -f)", Action::Monitoring),
    ]
}

/// The rows of the Search menu.
///
/// The largest menu after Edit, and the one with the most submenus: the five mark
/// styles alone account for four of them. The order is Notepad++'s, so a user who
/// knows that "Jump Up" is below "Clear Style" does not have to look.
fn search_items() -> Vec<Item> {
    let style_rows = |make: &dyn Fn(usize) -> Action, labels: [&str; 5]| -> Vec<Item> {
        labels
            .iter()
            .enumerate()
            .map(|(index, label)| Item::new(*label, make(index)))
            .collect()
    };
    let ordinals = ["1st Style", "2nd Style", "3rd Style", "4th Style", "5th Style"];
    let mut jump_up = style_rows(&|n| Action::JumpUp(Some(n)), ordinals);
    jump_up.push(Item::new("Find Mark Style", Action::JumpUp(None)));
    let mut jump_down = style_rows(&|n| Action::JumpDown(Some(n)), ordinals);
    jump_down.push(Item::new("Find Mark Style", Action::JumpDown(None)));
    let mut copy_styled = style_rows(&|n| Action::CopyStyled(Some(n)), ordinals);
    copy_styled.push(Item::new("All Styles", Action::CopyStyled(None)));
    copy_styled.push(Item::new("Find Mark Style", Action::CopyStyled(None)));

    vec![
        Item::new("Find...", Action::Find),
        Item::new("Find in Files...", Action::FindInFiles),
        Item::new("Find Next", Action::FindNext),
        Item::new("Find Previous", Action::FindPrev),
        Item::new("Select and Find Next", Action::SelectAndFindNext),
        Item::new("Select and Find Previous", Action::SelectAndFindPrev),
        Item::new("Find (Volatile) Next", Action::VolatileFindNext),
        Item::new("Find (Volatile) Previous", Action::VolatileFindPrev),
        Item::new("Replace...", Action::Replace),
        Item::new("Incremental Search", Action::IncrementalSearch),
        Item::new("Search Results Window", Action::SearchResultsWindow),
        Item::new("Next Search Result", Action::NextSearchResult),
        Item::new("Previous Search Result", Action::PrevSearchResult),
        Item::new("Go to...", Action::GotoLine),
        Item::new("Go to Matching Brace", Action::GotoBrace),
        Item::new(
            "Select All In-between {} [] or ()",
            Action::SelectBetweenBraces,
        ),
        Item::new("Mark...", Action::Mark),
        Item::separator(),
        Item::submenu(
            "Change History",
            vec![
                Item::new("Go to Next Change", Action::ChangeNext),
                Item::new("Go to Previous Change", Action::ChangePrev),
                Item::new("Clear Change History", Action::ClearChangeHistory),
            ],
        ),
        Item::separator(),
        Item::submenu(
            "Style All Occurrences of Token",
            style_rows(&|n| Action::StyleAll(n), ordinals),
        ),
        Item::submenu(
            "Style One Token",
            style_rows(&|n| Action::StyleOne(n), ordinals),
        ),
        Item::submenu(
            "Clear Style",
            {
                let mut rows = style_rows(&|n| Action::ClearStyle(n), [
                    "Clear 1st Style",
                    "Clear 2nd Style",
                    "Clear 3rd Style",
                    "Clear 4th Style",
                    "Clear 5th Style",
                ]);
                rows.push(Item::new("Clear all Styles", Action::ClearAllStyles));
                rows
            },
        ),
        Item::submenu("Jump Up", jump_up),
        Item::submenu("Jump Down", jump_down),
        Item::submenu("Copy Styled Text", copy_styled),
        Item::separator(),
        Item::submenu(
            "Bookmark",
            vec![
                Item::new("Toggle Bookmark", Action::ToggleBookmark),
                Item::new("Next Bookmark", Action::NextBookmark),
                Item::new("Previous Bookmark", Action::PrevBookmark),
                Item::new("Clear All Bookmarks", Action::ClearBookmarks),
                Item::new("Cut Bookmarked Lines", Action::CutBookmarkedLines),
                Item::new("Copy Bookmarked Lines", Action::CopyBookmarkedLines),
                Item::new(
                    "Paste to (Replace) Bookmarked Lines",
                    Action::PasteToBookmarkedLines,
                ),
                Item::new("Remove Bookmarked Lines", Action::RemoveBookmarkedLines),
                Item::new(
                    "Remove Non-Bookmarked Lines",
                    Action::RemoveNonBookmarkedLines,
                ),
                Item::new("Inverse Bookmarks", Action::InverseBookmarks),
            ],
        ),
        Item::separator(),
        Item::new("Find characters in range...", Action::FindCharInRange),
    ]
}

/// The rows of the Tools menu.
///
/// Four digests, each with the same three sources. Notepad++ nests them one level
/// deep and so does this: twelve flat rows would bury the fact that the digest is
/// the choice and the source is the detail.
fn tools_items() -> Vec<Item> {
    use nn_core::hash::Hash;
    let mut items = Vec::new();
    for hash in [Hash::Md5, Hash::Sha1, Hash::Sha256, Hash::Sha512] {
        items.push(Item::submenu(
            hash.name(),
            vec![
                Item::new("Generate...", Action::HashGenerate(hash)),
                Item::new("Generate from files...", Action::HashFromFile(hash)),
                Item::new(
                    "Generate from selection into clipboard",
                    Action::HashToClipboard(hash),
                ),
            ],
        ));
    }
    items
}

/// The rows of the Macro menu.
fn macro_items() -> Vec<Item> {
    vec![
        Item::new("Start Recording", Action::ToggleRecording),
        Item::new("Stop Recording", Action::StopRecording),
        Item::new("Playback", Action::PlayMacro),
        Item::new("Save Current Recorded Macro...", Action::SaveMacro),
        Item::new("Run a Macro Multiple Times...", Action::RunMacroMultiple),
    ]
}

/// The rows of the Window menu.
///
/// The tab list itself is not here: Notepad++ puts it in a dialog behind
/// "Windows...", and a dropdown that grew with every open file would be a worse
/// way to find one than the tab strip already on screen.
fn window_items() -> Vec<Item> {
    let sort = WindowSort::all()
        .into_iter()
        .map(|sort| Item::new(sort.label(), Action::SortWindows(sort)))
        .collect();
    vec![
        Item::submenu("Sort By", sort),
        Item::new("Windows...", Action::WindowsList),
        Item::separator(),
        Item::new("Recent Window", Action::RecentWindow),
    ]
}

/// The rows of the Help menu.
///
/// The updater rows are absent: this build has no updater, and a row that opened
/// a browser at a download page would be a promise the program cannot keep.
///
/// The four link rows name what they open. Until this release they were labelled
/// `Neko++ ...` and went to Notepad++'s pages, which was the closest thing to
/// documentation this program had; now that the project has pages of its own,
/// the label and the destination agree. The addresses are in [`crate::links`],
/// and the rows are still `Action::HomePage` and friends, so a keymap or a
/// settings file written before the move still names them.
fn help_items() -> Vec<Item> {
    vec![
        Item::new("Command Line Arguments...", Action::CommandLineArgs),
        Item::separator(),
        Item::new("Neko++ Home", Action::HomePage),
        Item::new("Neko++ Issues", Action::ProjectPage),
        Item::new("Neko++ Online User Manual", Action::OnlineManual),
        Item::new("Neko++ Community (LinkedIn)", Action::Forum),
        Item::separator(),
        Item::new("Neko++ License (GPLv3)", Action::License),
        Item::separator(),
        Item::new("Debug Info...", Action::DebugInfo),
        Item::new("About Neko++", Action::About),
    ]
}

/// The rows of the Run menu, including the saved commands.
///
/// A saved command is labelled with the command itself, truncated. A name would
/// be a second thing to invent, and the command is what the user recognises.
fn run_items(saved: &[String]) -> Vec<Item> {
    let mut items = vec![Item::new("Run...", Action::Run)];
    if !saved.is_empty() {
        items.push(Item::separator());
        for (index, label) in saved.iter().enumerate() {
            items.push(Item::new(label.clone(), Action::RunSaved(index)));
        }
    }
    items.push(Item::separator());
    items.push(Item::new(
        "Validate shortcuts.xml",
        Action::ValidateShortcuts,
    ));
    items
}

/// The rows of the Plugins menu.
///
/// Three rows that never change: the manager, the folder, and the reload. Below
/// them, one submenu per *running* plugin, named after the plugin and holding
/// its commands -- the arrangement Notepad++ uses, where "JSON" opens onto
/// "Pretty Print", "Minify" and "Validate".
///
/// A plugin that is switched off gets no submenu, not a greyed one: the switch
/// is what makes its commands exist, and the manager one row up is where it is
/// flipped. That also keeps the dropdown still while the pointer is over it --
/// its length follows the switches, and the switches are never touched from
/// inside the dropdown.
///
/// Nothing below the management rows is translatable. A plugin's name and its
/// command names are the plugin's own words; the rows are literal, so no
/// catalog can rename a command into one the plugin does not have.
///
/// With nothing installed the menu still says so, because an empty dropdown
/// reads as a bug rather than as an empty list. A folder that holds plugins,
/// switched on or not, gets no note: the manager is one row up.
fn plugin_items(plugins: &[PluginMenu], catalog: Option<&Catalog>) -> Vec<Item> {
    let verb = |english: &'static str| match catalog {
        Some(catalog) => catalog.tr(english).to_string(),
        None => english.to_string(),
    };
    // Every row here is built translated, prose and all, because `relabel_items`
    // rebuilds each label from its English text. Building them is cheap, so
    // nothing in this menu relies on being relabelled afterwards.
    let row = |english: &'static str, action: Action| {
        Item::composed(english.to_string(), verb(english), action)
    };
    // The folder row is always there, even with nothing installed: it is how a
    // user gets from "no plugins" to having one. Reload sits beside it so a
    // plugin dropped into the folder by hand does not need a restart.
    let mut items = vec![
        row("Manage Plugins...", Action::ManagePlugins),
        row("Open Plugins Folder...", Action::OpenPluginsFolder),
        row("Reload Plugins", Action::ReloadPlugins),
    ];
    if plugins.is_empty() {
        items.push(Item::separator());
        items.push(row("No Plugins Installed", Action::NoOp));
        return items;
    }
    // The index of a plugin in this list is the index the inventory gave it,
    // because `set_plugins` is handed the inventory in its own order. Nothing
    // else in this module needs to know that; the rows carry the pair.
    let commands = |index: usize, plugin: &PluginMenu| -> Vec<Item> {
        plugin
            .commands
            .iter()
            .enumerate()
            .map(|(command, label)| Item::literal(label.clone(), Action::Plugin(index, command)))
            .collect()
    };
    let running: Vec<Item> = plugins
        .iter()
        .enumerate()
        .filter(|(_, plugin)| plugin.enabled && !plugin.commands.is_empty())
        .map(|(index, plugin)| {
            Item::literal_submenu(plugin.name.clone(), commands(index, plugin))
        })
        .collect();
    if !running.is_empty() {
        items.push(Item::separator());
        items.extend(running);
    }
    items
}

/// The rows of the File menu, including the recent file list.
///
/// Recent entries are labelled with the file name alone: a dropdown is a fixed
/// width and a full path would either be truncated or make the menu enormous.
/// The rows of the Edit menu.
///
/// Notepad++'s Edit menu is the largest in the bar and almost all of it is
/// submenus, because the flat list would be sixty rows long. The order and the
/// grouping are copied exactly: a user who knows where "Convert Case to" lives
/// should not have to learn a second arrangement.
fn edit_items() -> Vec<Item> {
    vec![
        Item::new("Undo", Action::Undo),
        Item::new("Redo", Action::Redo),
        Item::separator(),
        Item::new("Cut", Action::Cut),
        Item::new("Copy", Action::Copy),
        Item::new("Paste", Action::Paste),
        Item::new("Delete", Action::Delete),
        Item::new("Select All", Action::SelectAll),
        Item::new("Begin/End Select", Action::BeginEndSelect),
        Item::new(
            "Begin/End Select in Column Mode",
            Action::BeginEndSelectColumn,
        ),
        Item::separator(),
        Item::submenu(
            "Insert",
            vec![
                Item::new("Date Time (short)", Action::InsertDateTimeShort),
                Item::new("Date Time (long)", Action::InsertDateTimeLong),
                Item::new("Date Time (customized)", Action::InsertDateTimeCustom),
            ],
        ),
        Item::submenu(
            "Copy to Clipboard",
            vec![
                Item::new("Copy Current Full File path", Action::CopyPath),
                Item::new("Copy Current Filename", Action::CopyFilename),
                Item::new("Copy Current Dir. Path", Action::CopyDirPath),
                Item::separator(),
                Item::new("Copy All Filenames", Action::CopyAllFilenames),
                Item::new("Copy All File Paths", Action::CopyAllFilePaths),
            ],
        ),
        Item::submenu(
            "Indent",
            vec![
                Item::new("Increase Line Indent", Action::Indent),
                Item::new("Decrease Line Indent", Action::Unindent),
            ],
        ),
        Item::submenu(
            "Convert Case to",
            vec![
                Item::new("UPPERCASE", Action::UpperCase),
                Item::new("lowercase", Action::LowerCase),
                Item::new("Proper Case", Action::ProperCaseForce),
                Item::new("Proper Case (blend)", Action::ProperCaseBlend),
                Item::new("Sentence case", Action::SentenceCaseForce),
                Item::new("Sentence case (blend)", Action::SentenceCaseBlend),
                Item::new("iNVERT cASE", Action::InvertCase),
                Item::new("ranDOm CasE", Action::RandomCase),
            ],
        ),
        Item::submenu("Line Operations", line_operation_items()),
        Item::submenu(
            "Comment/Uncomment",
            vec![
                Item::new("Toggle Single Line Comment", Action::ToggleComment),
                Item::new("Single Line Comment", Action::SingleLineComment),
                Item::new("Single Line Uncomment", Action::SingleLineUncomment),
                Item::new("Block Comment", Action::BlockComment),
                Item::new("Block Uncomment", Action::BlockUncomment),
            ],
        ),
        Item::submenu(
            "Auto-Completion",
            vec![
                Item::new("Function Completion", Action::CompleteFunction),
                Item::new("Word Completion", Action::CompleteWord),
                Item::new("Function Parameters Hint", Action::ParamHint),
                Item::new(
                    "Function Parameters Previous Hint",
                    Action::ParamHintPrev,
                ),
                Item::new("Function Parameters Next Hint", Action::ParamHintNext),
                Item::new("Path Completion", Action::CompletePath),
            ],
        ),
        Item::submenu(
            "EOL Conversion",
            vec![
                Item::new("Windows (CR LF)", Action::EolWindows),
                Item::new("Unix (LF)", Action::EolUnix),
                Item::new("Macintosh (CR)", Action::EolMac),
            ],
        ),
        Item::submenu(
            "Blank Operations",
            vec![
                Item::new("Trim Trailing Space", Action::TrimTrailing),
                Item::new("Trim Leading Space", Action::TrimLeading),
                Item::new("Trim Leading and Trailing Space", Action::TrimBoth),
                Item::new("EOL to Space", Action::EolToSpace),
                Item::new("Trim both and EOL to Space", Action::TrimAll),
                Item::separator(),
                Item::new("TAB to Space", Action::TabsToSpaces),
                Item::new("Space to TAB (All)", Action::SpacesToTabsAll),
                Item::new("Space to TAB (Leading)", Action::SpacesToTabsLeading),
            ],
        ),
        Item::submenu(
            "On Selection",
            vec![
                Item::new("Open File", Action::OpenSelectedFile),
                Item::new(
                    "Open Containing Folder in Explorer",
                    Action::OpenSelectedFolder,
                ),
                Item::separator(),
                Item::new("Redact Selection", Action::RedactSelection),
                Item::separator(),
                Item::new("Search on Internet", Action::SearchOnInternet),
                Item::new("Change Search Engine...", Action::ChangeSearchEngine),
            ],
        ),
        Item::separator(),
        Item::submenu(
            "Multi-select All",
            multi_select_items(Action::MultiSelectAll),
        ),
        Item::submenu(
            "Multi-select Next",
            multi_select_items(Action::MultiSelectNext),
        ),
        Item::new(
            "Undo the Latest Added Multi-Select",
            Action::MultiSelectUndo,
        ),
        Item::new(
            "Skip Current & Go to Next Multi-select",
            Action::MultiSelectSkip,
        ),
        Item::separator(),
        Item::new("Column Mode...", Action::ColumnMode),
        Item::new("Column Editor...", Action::ColumnEditor),
        Item::new("Character Panel", Action::CharPanel),
        Item::new("Clipboard History", Action::ClipboardHistory),
        Item::separator(),
        Item::submenu(
            "Read-Only in Neko++",
            vec![
                Item::new("Read-Only on Current Document", Action::ReadOnlyCurrent),
                Item::new("Read-Only for All Documents", Action::ReadOnlyAll),
                Item::new(
                    "Clear Read-Only for All Documents",
                    Action::ClearReadOnlyAll,
                ),
            ],
        ),
    ]
}

/// The rows of the Line Operations submenu.
///
/// The fourteen sorts are three groups of seven separated by rules, which is how
/// Notepad++ lays them out: the ascending orders, then the descending ones, with
/// the three non-sort rows above them.
fn line_operation_items() -> Vec<Item> {
    let mut items = vec![
        Item::new("Duplicate Current Line", Action::DuplicateLine),
        Item::new("Remove Duplicate Lines", Action::DedupeAll),
        Item::new(
            "Remove Consecutive Duplicate Lines",
            Action::DedupeConsecutive,
        ),
        Item::new("Split Lines", Action::SplitLines),
        Item::new("Join Lines", Action::JoinLines),
        Item::new("Move Up Current Line", Action::MoveLineUp),
        Item::new("Move Down Current Line", Action::MoveLineDown),
        Item::new("Remove Empty Lines", Action::RemoveEmpty),
        Item::new(
            "Remove Empty Lines (Containing Blank characters)",
            Action::RemoveEmptyWithBlank,
        ),
        Item::new("Insert Blank Line Above Current", Action::InsertBlankAbove),
        Item::new("Insert Blank Line Below Current", Action::InsertBlankBelow),
        Item::new("Reverse Line Order", Action::ReverseLineOrder),
        Item::new("Randomize Line Order", Action::RandomizeLineOrder),
        Item::separator(),
    ];
    let ascending = [
        ("Sort Lines Lexicographically Ascending", Sort::LexAsc),
        (
            "Sort Lines Lex. Ascending Ignoring Case",
            Sort::LexAscIgnoreCase,
        ),
        ("Sort Lines In Locale Order Ascending", Sort::LocaleAsc),
        ("Sort Lines As Integers Ascending", Sort::IntegerAsc),
        (
            "Sort Lines As Decimals (Comma) Ascending",
            Sort::DecimalCommaAsc,
        ),
        (
            "Sort Lines As Decimals (Dot) Ascending",
            Sort::DecimalDotAsc,
        ),
        ("Sort Lines By Length Ascending", Sort::LengthAsc),
    ];
    let descending = [
        ("Sort Lines Lexicographically Descending", Sort::LexDesc),
        (
            "Sort Lines Lex. Descending Ignoring Case",
            Sort::LexDescIgnoreCase,
        ),
        ("Sort Lines In Locale Order Descending", Sort::LocaleDesc),
        ("Sort Lines As Integers Descending", Sort::IntegerDesc),
        (
            "Sort Lines As Decimals (Comma) Descending",
            Sort::DecimalCommaDesc,
        ),
        (
            "Sort Lines As Decimals (Dot) Descending",
            Sort::DecimalDotDesc,
        ),
        ("Sort Lines By Length Descending", Sort::LengthDesc),
    ];
    for (label, sort) in ascending {
        items.push(Item::new(label, Action::SortLines(sort)));
    }
    items.push(Item::separator());
    for (label, sort) in descending {
        items.push(Item::new(label, Action::SortLines(sort)));
    }
    items
}

/// The four matching rules, as rows of a multi-select submenu.
///
/// `make` is the action constructor, so the two submenus share one list and
/// cannot drift apart.
fn multi_select_items(make: fn(MatchMode) -> Action) -> Vec<Item> {
    let modes = [
        (
            "Ignore Case & Whole Word",
            MatchMode {
                match_case: false,
                whole_word: true,
            },
        ),
        (
            "Match Case Only",
            MatchMode {
                match_case: true,
                whole_word: false,
            },
        ),
        (
            "Match Whole Word Only",
            MatchMode {
                match_case: false,
                whole_word: false,
            },
        ),
        (
            "Match Case & Whole Word",
            MatchMode {
                match_case: true,
                whole_word: true,
            },
        ),
    ];
    modes
        .into_iter()
        .map(|(label, mode)| Item::new(label, make(mode)))
        .collect()
}

fn file_items(recent: &[String]) -> Vec<Item> {
    let mut items = vec![
        Item::new("New", Action::New),
        Item::new("Open...", Action::Open),
    ];
    if !recent.is_empty() {
        items.push(Item::separator());
        for (index, name) in recent.iter().enumerate() {
            items.push(Item::new(name.clone(), Action::OpenRecent(index)));
        }
        items.push(Item::new("Clear Recent", Action::ClearRecent));
    }
    items.push(Item::separator());
    // Notepad++ puts the three ways of reaching a file's folder in a submenu,
    // because on Windows they are three different programs. They are three
    // different programs here too, so the submenu is not decoration.
    items.push(Item::submenu(
        "Open Containing Folder",
        vec![
            Item::new("Explorer", Action::OpenContainingFolder),
            Item::new("cmd", Action::OpenContainingFolderCmd),
            Item::new("PowerShell", Action::OpenContainingFolderPowerShell),
            Item::separator(),
            Item::new("Folder as Workspace", Action::FolderAsWorkspace),
        ],
    ));
    items.push(Item::new("Open in Default Viewer", Action::OpenInDefaultViewer));
    items.push(Item::new(
        "Open Folder as Workspace...",
        Action::OpenFolderAsWorkspace,
    ));
    items.push(Item::new("Reload from Disk", Action::Reload));
    items.push(Item::new("Save", Action::Save));
    items.push(Item::new("Save As...", Action::SaveAs));
    items.push(Item::new("Save a Copy As...", Action::SaveCopyAs));
    items.push(Item::new("Save All", Action::SaveAll));
    items.push(Item::new("Rename...", Action::Rename));
    items.push(Item::new("Close", Action::CloseTab));
    items.push(Item::new("Close All", Action::CloseAll));
    items.push(Item::submenu(
        "Close Multiple Documents",
        vec![
            Item::new("Close All but Active Document", Action::CloseOthers),
            Item::new("Close All but Pinned Documents", Action::CloseAllButPinned),
            Item::new("Close All to the Left", Action::CloseToLeft),
            Item::new("Close All to the Right", Action::CloseToRight),
            Item::new("Close All Unchanged", Action::CloseAllUnchanged),
        ],
    ));
    items.push(Item::new("Move to Recycle Bin", Action::MoveToTrash));
    items.push(Item::separator());
    items.push(Item::new("Load Session...", Action::LoadSession));
    items.push(Item::new("Save Session...", Action::SaveSession));
    items.push(Item::separator());
    items.push(Item::new("Print...", Action::Print));
    items.push(Item::new("Print Now", Action::PrintNow));
    items.push(Item::separator());
    items.push(Item::new("Exit", Action::Exit));
    items
}

/// The Settings menu's rows.
///
/// Notepad++'s shape: three dialogs, a rule, the two imports, a rule, and the
/// context menu editor.
///
/// One deviation, and it is deliberate. Notepad++'s Shortcut Mapper dialog has
/// Import and Export buttons along its bottom edge; this editor has no such
/// dialog, so the two commands that would be those buttons are rows under
/// `Shortcut Mapper` instead. They are the same two operations in the same
/// place, one level down, and the alternative was dropping them.
fn settings_items() -> Vec<Item> {
    vec![
        Item::new("Preferences...", Action::Preferences),
        Item::new("Style Configurator...", Action::StyleConfigurator),
        Item::submenu(
            "Theme",
            ThemeChoice::ALL
                .iter()
                .enumerate()
                .map(|(index, choice)| Item::new(choice.label(), Action::Theme(index)))
                .collect(),
        ),
        Item::submenu(
            "Shortcut Mapper",
            vec![
                Item::new("Open Shortcut Mapper...", Action::ShortcutMapper),
                Item::separator(),
                Item::new("Import Notepad++ Config...", Action::ImportNotepad),
                Item::new("Export Shortcuts...", Action::ExportShortcuts),
            ],
        ),
        Item::separator(),
        Item::submenu(
            "Import",
            vec![
                Item::new("Import plugin(s)...", Action::ImportPlugin),
                Item::new("Import style theme(s)...", Action::ImportTheme),
            ],
        ),
        Item::separator(),
        Item::new("Edit Popup ContextMenu", Action::EditContextMenu),
    ]
}

/// The rows Notepad++ keeps outside the letter groups.
///
/// `KIXtart` sits between the `J` and `L` groups, and `XML` and `YAML` follow
/// the `V` group. They are not filed under their own letters because Notepad++
/// added them after the grouped block was written and never moved them in.
const UNGROUPED: [&str; 3] = ["KIXtart", "XML", "YAML"];

/// The Language menu's rows.
///
/// Notepad++ does not list ninety-odd languages in one column: it groups them
/// under `A`, `B`, `C` and so on, which turns a menu taller than the screen into
/// a dozen short ones. The groups are built from the table rather than written
/// out, so adding a language to `LANGS` adds it to the right group.
///
/// The user-defined languages go under `User-Defined` for the same reason, and
/// the three commands that manage them go under `User Defined Language`.
fn language_items(names: &[String]) -> Vec<Item> {
    let mut items = vec![
        Item::new("None (Normal Text)", Action::LanguagePlain),
        Item::separator(),
    ];

    // One submenu per first letter, in the order the table lists them. The
    // table is grouped already, so this walks it once and starts a new submenu
    // whenever the letter changes.
    let mut groups: Vec<(char, Vec<Item>)> = Vec::new();
    for (index, spec) in nn_core::LANGS.iter().enumerate() {
        if UNGROUPED.contains(&spec.name) {
            continue;
        }
        let letter = spec.name.chars().next().unwrap_or('?').to_ascii_uppercase();
        let row = Item::new(spec.name, Action::LanguageBuiltIn(index));
        match groups.last_mut() {
            Some((open, rows)) if *open == letter => rows.push(row),
            _ => groups.push((letter, vec![row])),
        }
    }
    for (letter, rows) in groups {
        items.push(Item::submenu(letter.to_string(), rows));
    }

    // The ungrouped rows go where Notepad++ puts them: `KIXtart` after `J`,
    // `XML` and `YAML` after `V`. Inserting by letter keeps that true without
    // hard-coding positions that would drift if a group were ever added.
    for name in UNGROUPED {
        let Some(index) = nn_core::lang_by_name(name) else {
            continue;
        };
        let letter = name.chars().next().unwrap_or('?').to_ascii_uppercase();
        let at = items
            .iter()
            .position(|item| {
                item.children
                    .first()
                    .and_then(|row| row.label.chars().next())
                    .is_some_and(|first| first > letter)
            })
            .unwrap_or(items.len());
        items.insert(at, Item::new(name, Action::LanguageBuiltIn(index)));
    }

    items.push(Item::separator());
    items.push(Item::submenu(
        "User Defined Language",
        vec![
            Item::new("Define your language...", Action::DefineLanguage),
            Item::new("Open User Defined Language folder...", Action::OpenUdlFolder),
            Item::new(
                "Notepad++ User Defined Languages Collection",
                Action::UdlCollection,
            ),
        ],
    ));
    // The installed languages are a submenu of their own, and the row exists
    // even when there are none so the menu does not change shape when the first
    // one is installed.
    let mut udl: Vec<Item> = names
        .iter()
        .enumerate()
        .map(|(index, name)| Item::new(name.clone(), Action::Language(index)))
        .collect();
    if udl.is_empty() {
        udl.push(Item::new("(none installed)", Action::NoOp));
    }
    items.push(Item::submenu("User-Defined", udl));
    items
}

/// Whether two row lists say the same thing, at every depth.
///
/// The "did this change?" checks used to compare the top-level labels only,
/// which was enough while every row was a command. Now that the Language menu
/// keeps its user-defined languages in a submenu, a new language changes no
/// top-level label at all, and a shallow comparison would leave the menu
/// showing the old list.
fn same_rows(a: &[Item], b: &[Item]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            a.english == b.english
                && a.action == b.action
                && a.separator == b.separator
                && same_rows(&a.children, &b.children)
        })
}

/// The public view of `items`, and of everything under them.
fn row_items(items: &[Item]) -> Vec<RowItem> {
    items
        .iter()
        .map(|item| RowItem {
            label: item.label.clone(),
            shortcut: item.shortcut.clone(),
            action: item.action,
            separator: item.separator,
            children: row_items(&item.children),
        })
        .collect()
}

/// Rewrites every row's shortcut column from the keymap, at every depth.
///
/// A submenu row has no chord of its own: the arrow is drawn where the shortcut
/// column would be, and a chord beside it would read as the chord for opening the
/// submenu.
fn set_shortcuts(keymap: &Keymap, items: &mut [Item]) {
    for item in items {
        if item.separator || item.is_submenu() {
            item.shortcut = String::new();
        } else {
            item.shortcut = keymap.display(item.action);
        }
        set_shortcuts(keymap, &mut item.children);
    }
}

/// Shapes every row of `items`, and of every submenu under them.
/// The widest line of a shaped buffer, in pixels.
///
/// The menu is laid out in pixels rather than in character cells because a
/// translated label is not necessarily as wide as it is long: a Japanese title
/// of four characters is eight cells wide, and the cell count would put the next
/// one on top of it.
fn widest_line(buffer: &Buffer) -> f32 {
    buffer.layout_runs().map(|run| run.line_w).fold(0.0, f32::max)
}

fn shape_items(bar: &MenuBar, font_system: &mut FontSystem, items: &[Item]) -> Vec<Shaped> {
    items
        .iter()
        .map(|item| Shaped {
            label: bar.shape(font_system, &item.label),
            shortcut: bar.shape(font_system, &item.shortcut),
            children: shape_items(bar, font_system, &item.children),
        })
        .collect()
}

/// Rewrites every title and row from the catalog.
///
/// A free function rather than a method so the catalog and the menus can be
/// borrowed at once: `self.catalog.as_ref()` and `&mut self.menus` are disjoint
/// fields, but a `&mut self` method could not see both.
fn relabel(catalog: Option<&Catalog>, menus: &mut [Menu]) {
    for menu in menus {
        menu.title = match catalog {
            Some(catalog) => catalog.tr(menu.english).to_string(),
            None => menu.english.to_string(),
        };
        relabel_items(catalog, &mut menu.items);
    }
}

/// Relabels `items` and everything under them.
fn relabel_items(catalog: Option<&Catalog>, items: &mut [Item]) {
    for item in items {
        if item.separator || item.literal {
            continue;
        }
        item.label = match catalog {
            Some(catalog) => catalog.tr(&item.english).to_string(),
            None => item.english.clone(),
        };
        relabel_items(catalog, &mut item.children);
    }
}

impl MenuBar {
    /// Builds the standard menu set.
    pub fn new(
        metrics: Metrics,
        family: Family<'static>,
        char_width: f32,
        keymap: Keymap,
    ) -> Self {
        let menus = vec![
            Menu {
                title: "File".to_string(),
                english: "File",
                items: file_items(&[]),
            },
            Menu {
                title: "Edit".to_string(),
                english: "Edit",
                items: edit_items(),
            },
            Menu {
                title: "Search".to_string(),
                english: "Search",
                items: search_items(),
            },
            Menu {
                title: "View".to_string(),
                english: "View",
                items: view_items(),
            },
            Menu {
                title: "Encoding".to_string(),
                english: "Encoding",
                items: encoding_items(),
            },
            Menu {
                title: "Language".to_string(),
                english: "Language",
                items: language_items(&[]),
            },
            Menu {
                title: "Settings".to_string(),
                english: "Settings",
                items: settings_items(),
            },
            Menu {
                title: "Tools".to_string(),
                english: "Tools",
                items: tools_items(),
            },
            Menu {
                title: "Macro".to_string(),
                english: "Macro",
                items: macro_items(),
            },
            Menu {
                title: "Run".to_string(),
                english: "Run",
                items: run_items(&[]),
            },
            Menu {
                title: "Plugins".to_string(),
                english: "Plugins",
                items: plugin_items(&[], None),
            },
            Menu {
                title: "Window".to_string(),
                english: "Window",
                items: window_items(),
            },
            Menu {
                title: "Help".to_string(),
                english: "Help",
                items: help_items(),
            },
        ];

        // Guessed from the character grid until `prepare` measures the shaped
        // titles, so the bar has a layout before the first frame.
        let title_widths: Vec<f32> = menus
            .iter()
            .map(|menu| menu.title.chars().count() as f32 * char_width)
            .collect();
        let mut bar = Self {
            menus,
            open: None,
            hover: None,
            keymap,
            metrics,
            family,
            char_width,
            titles: Vec::new(),
            title_widths,
            shaped_rows: Vec::new(),
            arrow: Buffer::new_empty(metrics),
            path: Vec::new(),
            catalog: None,
            plugins: Vec::new(),
            shaped: false,
        };
        bar.refresh_shortcuts();
        bar
    }

    /// Points the shortcut column at a new keymap.
    pub fn set_keymap(&mut self, keymap: Keymap) {
        if self.keymap == keymap {
            return;
        }
        self.keymap = keymap;
        self.refresh_shortcuts();
    }

    /// Swaps in a localization, or clears it with `None`.
    ///
    /// The whole bar is relabelled from the stored English text, so switching
    /// languages twice does not translate a translation.
    pub fn set_catalog(&mut self, catalog: Option<Catalog>) {
        self.catalog = catalog;
        relabel(self.catalog.as_ref(), &mut self.menus);
        // The plugin rows are built from the catalog and the folder rather than
        // relabelled above, so they are built again here: a row that appears
        // only when the folder is empty has to arrive in the new language too,
        // and a plugin's own words have to keep the spelling the plugin chose.
        self.menus[PLUGIN_MENU].items = plugin_items(&self.plugins, self.catalog.as_ref());
        self.refresh_shortcuts();
        self.close();
    }

    /// The name of the active localization, if any.
    pub fn catalog_name(&self) -> Option<&str> {
        self.catalog.as_ref().map(Catalog::name)
    }

    /// Every fixed English label the bar draws, at every depth.
    ///
    /// The stored English text rather than what is on screen, so a test can ask
    /// what a catalog has to cover without first applying one. Labels that come
    /// from outside -- plugin names, user-defined languages, recent files -- are
    /// in the list too when the bar was built with them, which is why the
    /// completeness test builds a bar with none of them.
    pub fn english_labels(&self) -> Vec<&str> {
        fn walk<'a>(items: &'a [Item], out: &mut Vec<&'a str>) {
            for item in items {
                if !item.separator && !item.literal {
                    out.push(&item.english);
                }
                walk(&item.children, out);
            }
        }
        let mut out: Vec<&str> = self.menus.iter().map(|menu| menu.english).collect();
        for menu in &self.menus {
            walk(&menu.items, &mut out);
        }
        out
    }

    /// Rewrites every row's shortcut from the keymap.
    ///
    /// Called whenever the rows or the keymap change, so the two cannot drift.
    fn refresh_shortcuts(&mut self) {
        let keymap = &self.keymap;
        for menu in &mut self.menus {
            set_shortcuts(keymap, &mut menu.items);
        }
        self.shaped = false;
    }

    /// Replaces the recent file list shown in the File menu.
    ///
    /// The labels are reshaped on the next frame rather than here, so the caller
    /// does not need the font system.
    pub fn set_recent(&mut self, names: &[String]) {
        let items = file_items(names);
        if self.menus[0].items.len() == items.len()
            && self.menus[0]
                .items
                .iter()
                .zip(&items)
                .all(|(a, b)| a.english == b.english)
        {
            return;
        }
        self.menus[0].items = items;
        relabel(self.catalog.as_ref(), &mut self.menus);
        self.refresh_shortcuts();
        self.close();
    }

    /// Replaces the saved commands shown in the Run menu.
    pub fn set_runs(&mut self, labels: &[String]) {
        let items = run_items(labels);
        if self.menus[RUN_MENU].items.len() == items.len()
            && self.menus[RUN_MENU]
                .items
                .iter()
                .zip(&items)
                .all(|(a, b)| a.english == b.english)
        {
            return;
        }
        self.menus[RUN_MENU].items = items;
        relabel(self.catalog.as_ref(), &mut self.menus);
        self.refresh_shortcuts();
        self.close();
    }

    /// Replaces the user-defined languages shown in the Language menu.
    pub fn set_languages(&mut self, names: &[String]) {
        let items = language_items(names);
        if same_rows(&self.menus[LANGUAGE_MENU].items, &items) {
            return;
        }
        self.menus[LANGUAGE_MENU].items = items;
        relabel(self.catalog.as_ref(), &mut self.menus);
        self.refresh_shortcuts();
        self.close();
    }

    /// Rebuilds the Plugins menu from the plugins the editor has.
    ///
    /// The list is in inventory order and is kept even when the rows come out
    /// identical, because the next language change rebuilds them from it.
    ///
    /// Nothing is relabelled here. `plugin_items` builds every row already
    /// translated, and the rows of this menu are the only ones whose words come
    /// from a catalog and whose position comes from the disk.
    pub fn set_plugins(&mut self, plugins: &[PluginMenu]) {
        self.plugins = plugins.to_vec();
        let items = plugin_items(&self.plugins, self.catalog.as_ref());
        if same_rows(&self.menus[PLUGIN_MENU].items, &items) {
            return;
        }
        self.menus[PLUGIN_MENU].items = items;
        self.refresh_shortcuts();
        self.close();
    }

    /// The bar as a list of rows, for anything outside the menu that needs to
    /// see it.
    ///
    /// The accessibility tree is the caller. It needs the titles, the labels,
    /// the shortcuts and the action each row runs, and it must not reach into
    /// the private `Item` to get them -- a second reader of the same fields is
    /// how a menu and its accessibility tree end up disagreeing.
    pub fn rows(&self) -> Vec<Row> {
        self.menus
            .iter()
            .map(|menu| Row {
                title: menu.title.clone(),
                items: row_items(&menu.items),
            })
            .collect()
    }

    /// The height of the bar itself.
    pub fn height(&self, line_height: f32) -> f32 {
        line_height + 6.0
    }

    /// Whether a dropdown is open.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Closes any open dropdown, and every submenu under it.
    pub fn close(&mut self) {
        self.open = None;
        self.path.clear();
        self.hover = None;
    }

    /// Shapes every label once.
    pub fn prepare(&mut self, font_system: &mut FontSystem) {
        if self.shaped {
            return;
        }
        self.shaped = true;
        self.titles = self
            .menus
            .iter()
            .map(|m| self.shape(font_system, &m.title))
            .collect();
        self.title_widths = self.titles.iter().map(widest_line).collect();
        self.arrow = self.shape(font_system, SUBMENU_ARROW);
        self.shaped_rows = self
            .menus
            .iter()
            .map(|m| shape_items(self, font_system, &m.items))
            .collect();
    }

    fn shape(&self, font_system: &mut FontSystem, text: &str) -> Buffer {
        let mut buffer = Buffer::new_empty(self.metrics);
        buffer.set_size(None, None);
        buffer.set_text(
            text,
            &Attrs::new().family(self.family),
            shaping_for(text),
            None,
        );
        buffer.shape_until_scroll(font_system, false);
        buffer
    }

    /// The x pixel where menu `index` starts.
    fn title_left(&self, index: usize) -> f32 {
        let mut x = 0.0;
        for width in &self.title_widths[..index] {
            x += width + 2.0 * TITLE_PADDING;
        }
        x
    }

    /// The width of menu `index`'s title button.
    fn title_width(&self, index: usize) -> f32 {
        self.title_widths[index] + 2.0 * TITLE_PADDING
    }

    /// The x pixel where menu `index`'s dropdown starts.
    ///
    /// Normally the title's left edge, but pulled back when the dropdown would
    /// run off the right of the window. The last menus in the bar are the ones
    /// with the longest rows -- Help carries a full path -- so without the clamp
    /// the rightmost dropdown is cut off mid-label.
    fn dropdown_left(&self, index: usize, width: f32) -> f32 {
        let left = self.title_left(index);
        let room = width - self.dropdown_width(index);
        left.min(room.max(0.0))
    }

    /// The width of a panel holding `items`.
    ///
    /// The arrow column is only reserved when the panel actually has a row that
    /// opens a submenu, so a menu without one is exactly as wide as it was before
    /// submenus existed.
    fn panel_width(&self, items: &[Item], shaped: &[Shaped]) -> f32 {
        // Measured where the rows have been shaped, counted in cells before the
        // first prepare -- which is what a monospace font makes of them anyway.
        let (label, shortcut) = if shaped.len() == items.len() {
            (
                shaped.iter().map(|s| widest_line(&s.label)).fold(0.0, f32::max),
                shaped
                    .iter()
                    .map(|s| widest_line(&s.shortcut))
                    .fold(0.0, f32::max),
            )
        } else {
            let cell = |count: usize| count as f32 * self.char_width;
            (
                cell(items.iter().map(|i| i.label.chars().count()).max().unwrap_or(0)),
                cell(items
                    .iter()
                    .map(|i| i.shortcut.chars().count())
                    .max()
                    .unwrap_or(0)),
            )
        };
        let arrow = if items.iter().any(Item::is_submenu) {
            ARROW_COLUMN
        } else {
            0.0
        };
        label
            + if shortcut > 0.0 {
                COLUMN_GAP + shortcut
            } else {
                0.0
            }
            + arrow
            + 2.0 * ROW_PADDING
    }

    /// The width of menu `index`'s dropdown.
    fn dropdown_width(&self, index: usize) -> f32 {
        let shaped = self.shaped_rows.get(index).map_or(&[][..], Vec::as_slice);
        self.panel_width(&self.menus[index].items, shaped)
    }

    /// The height of one dropdown row.
    fn row_height(&self, line_height: f32) -> f32 {
        line_height + ROW_EXTRA
    }

    /// The total height of menu `index`'s dropdown.
    fn dropdown_height(&self, index: usize, line_height: f32) -> f32 {
        self.menus[index].items.len() as f32 * self.row_height(line_height) + 2.0
    }

    /// The rows of the submenu `item` opens.
    fn submenu_width(&self, item: &Item, shaped: &[Shaped]) -> f32 {
        self.panel_width(&item.children, shaped)
    }

    /// The height of the submenu `item` opens.
    fn submenu_height(&self, item: &Item, line_height: f32) -> f32 {
        item.children.len() as f32 * self.row_height(line_height) + 2.0
    }

    /// The rows shown at `level`, where level 0 is the top dropdown.
    fn level_items(&self, level: usize) -> &[Item] {
        let Some(index) = self.open else {
            return &[];
        };
        let mut items: &[Item] = &self.menus[index].items;
        for &row in self.path.iter().take(level) {
            match items.get(row) {
                Some(item) => items = &item.children,
                None => return &[],
            }
        }
        items
    }

    /// The shaped rows shown at `level`, mirroring [`Self::level_items`].
    fn level_shaped(&self, level: usize) -> &[Shaped] {
        let Some(index) = self.open else {
            return &[];
        };
        let mut rows: &[Shaped] = match self.shaped_rows.get(index) {
            Some(rows) => rows,
            None => return &[],
        };
        for &row in self.path.iter().take(level) {
            match rows.get(row) {
                Some(shaped) => rows = &shaped.children,
                None => return &[],
            }
        }
        rows
    }

    /// Where every visible panel sits, outermost first.
    ///
    /// One entry per open level: the top dropdown, then one per entry in `path`.
    /// Everything that draws or hit-tests a dropdown goes through this, so the
    /// background, the labels and the pointer cannot disagree about where a panel
    /// is -- which is the bug a second copy of the arithmetic always produces.
    fn panels(&self, width: f32, line_height: f32) -> Vec<Panel> {
        let mut out = Vec::with_capacity(1 + self.path.len());
        let Some(index) = self.open else {
            return out;
        };
        let row_height = self.row_height(line_height);
        out.push(Panel {
            left: self.dropdown_left(index, width),
            top: self.height(line_height) + 1.0,
            width: self.dropdown_width(index),
            height: self.dropdown_height(index, line_height),
        });
        for level in 0..self.path.len() {
            let parent = out[level];
            let row = self.path[level];
            let Some(item) = self.level_items(level).get(row) else {
                break;
            };
            let children = self
                .level_shaped(level)
                .get(row)
                .map_or(&[][..], |shaped| shaped.children.as_slice());
            let panel_width = self.submenu_width(item, children);
            let panel_height = self.submenu_height(item, line_height);
            // To the right of the parent row, overlapping its border so the two
            // panels read as one object rather than two.
            let mut left = parent.left + parent.width - PANEL_OVERLAP;
            if left + panel_width > width {
                // No room on the right: open to the left instead, which is what
                // every desktop menu does at the edge of the screen.
                left = (parent.left - panel_width + PANEL_OVERLAP).max(0.0);
            }
            out.push(Panel {
                left,
                top: parent.top + 1.0 + row as f32 * row_height - PANEL_OVERLAP,
                width: panel_width,
                height: panel_height,
            });
        }
        out
    }

    /// The menu whose title is under `x`, if any.
    fn title_at(&self, x: f32) -> Option<usize> {
        let mut left = 0.0;
        for index in 0..self.menus.len() {
            let width = self.title_width(index);
            if x >= left && x < left + width {
                return Some(index);
            }
            left += width;
        }
        None
    }

    /// The row of `panel` under `y`, if any.
    fn row_in(&self, level: usize, panel: &Panel, y: f32, line_height: f32) -> Option<usize> {
        let top = panel.top + 1.0;
        if y < top {
            return None;
        }
        let row = ((y - top) / self.row_height(line_height)).floor() as usize;
        if row < self.level_items(level).len() {
            Some(row)
        } else {
            None
        }
    }

    /// The deepest panel containing a point, with its level.
    ///
    /// Deepest first, because a submenu overlaps the panel it came from and the
    /// overlap belongs to the submenu.
    fn panel_at(&self, x: f32, y: f32, width: f32, line_height: f32) -> Option<(usize, Panel)> {
        self.panels(width, line_height)
            .iter()
            .enumerate()
            .rev()
            .find(|(_, panel)| panel.contains(x, y))
            .map(|(level, panel)| (level, *panel))
    }

    /// Handles a pointer move. Returns true when the bar consumed it.
    pub fn hover_at(&mut self, x: f32, y: f32, line_height: f32, width: f32) -> bool {
        if y < self.height(line_height) {
            // Hovering the bar while a menu is open switches to that menu, which
            // is how every desktop menu bar behaves.
            if self.open.is_some() {
                if let Some(index) = self.title_at(x) {
                    if self.open != Some(index) {
                        self.open = Some(index);
                        self.path.clear();
                        self.hover = None;
                    }
                }
            }
            return true;
        }
        if self.open.is_none() {
            return false;
        }
        let Some((level, panel)) = self.panel_at(x, y, width, line_height) else {
            // Outside every panel. The highlight is left alone rather than
            // cleared: the pointer crosses the gap between a row and its submenu
            // on the way there, and clearing would make the submenu flicker shut.
            return false;
        };
        let row = self.row_in(level, &panel, y, line_height);
        // Moving back up the tree closes the submenus below this level.
        self.path.truncate(level);
        self.hover = row;
        if let Some(row) = row {
            if self.level_items(level).get(row).is_some_and(Item::is_submenu) {
                self.path.push(row);
                self.hover = None;
            }
        }
        true
    }

    /// Handles a click. Returns the action to run, if the click hit an item.
    ///
    /// A click on a title opens or closes that menu; a click elsewhere closes it.
    /// Opens the dropdown at `index`, or closes it when it is already open.
    ///
    /// The mouse path goes through `click`; this is for keyboard access and for
    /// the offscreen bench, which has no pointer.
    pub fn open_menu(&mut self, index: usize) {
        if index >= self.menus.len() {
            return;
        }
        self.open = if self.open == Some(index) {
            None
        } else {
            Some(index)
        };
        self.path.clear();
        self.hover = None;
    }

    /// Opens the menu whose title starts with `letter`.
    ///
    /// Two menus can start with the same letter -- Edit and Encoding, Search and
    /// Settings -- so pressing the same chord again moves to the next one, which
    /// is how Windows and Notepad++ resolve the collision. A letter that names
    /// only one menu toggles it, so the chord that opened a menu closes it.
    ///
    /// Returns false when no menu starts with that letter, which is the caller's
    /// cue to let the key through to whatever else wanted it.
    pub fn open_by_letter(&mut self, letter: char) -> bool {
        let letter = letter.to_ascii_lowercase();
        let matches: Vec<usize> = self
            .menus
            .iter()
            .enumerate()
            .filter(|(_, menu)| {
                menu.title
                    .chars()
                    .next()
                    .is_some_and(|first| first.to_ascii_lowercase() == letter)
            })
            .map(|(index, _)| index)
            .collect();
        if matches.is_empty() {
            return false;
        }
        let next = match self
            .open
            .and_then(|open| matches.iter().position(|&index| index == open))
        {
            Some(_) if matches.len() == 1 => {
                self.close();
                return true;
            }
            Some(position) => matches[(position + 1) % matches.len()],
            None => matches[0],
        };
        self.open = Some(next);
        self.path.clear();
        self.hover = None;
        true
    }

    /// Moves to the next or previous menu title, keeping the dropdown open.
    ///
    /// This is what Left and Right do at the top level of a menu bar: the
    /// dropdown follows the title rather than closing.
    pub fn cycle_menu(&mut self, delta: isize) -> bool {
        let Some(open) = self.open else {
            return false;
        };
        if self.menus.is_empty() {
            return false;
        }
        let count = self.menus.len() as isize;
        self.open = Some((open as isize + delta).rem_euclid(count) as usize);
        self.path.clear();
        self.hover = None;
        true
    }

    pub fn click(&mut self, x: f32, y: f32, line_height: f32, width: f32) -> Option<Action> {
        if y < self.height(line_height) {
            match self.title_at(x) {
                Some(index) => {
                    self.open = if self.open == Some(index) {
                        None
                    } else {
                        Some(index)
                    };
                    self.path.clear();
                    self.hover = None;
                }
                None => self.close(),
            }
            return None;
        }

        let Some((level, panel)) = self.panel_at(x, y, width, line_height) else {
            self.close();
            return None;
        };
        let row = self.row_in(level, &panel, y, line_height)?;
        // Copied out before anything mutates: the row borrows the menu.
        let item = self.level_items(level).get(row)?;
        let (separator, submenu, action) = (item.separator, item.is_submenu(), item.action);
        if separator {
            return None;
        }
        if submenu {
            self.path.truncate(level);
            self.path.push(row);
            self.hover = None;
            return None;
        }
        self.close();
        Some(action)
    }

    /// Opens the submenu under the highlight.
    ///
    /// Returns false when the highlighted row is a command, which is the caller's
    /// cue to run it instead.
    /// Puts the highlight on `row` of the deepest open level.
    ///
    /// The pointer path goes through `hover_at`, which needs a geometry the
    /// caller may not have. This is the same thing by index, which is what a
    /// test or the bench wants when it is posing a menu rather than driving one.
    pub fn set_hover(&mut self, row: Option<usize>) {
        self.hover = row;
    }

    pub fn open_submenu(&mut self) -> bool {
        let level = self.path.len();
        let Some(row) = self.hover else {
            return false;
        };
        if !self
            .level_items(level)
            .get(row)
            .is_some_and(Item::is_submenu)
        {
            return false;
        }
        self.path.push(row);
        self.hover = None;
        true
    }

    /// Closes the deepest submenu.
    ///
    /// Returns false when none was open, which is the caller's cue to close the
    /// whole menu -- the same key does both, one level at a time.
    pub fn back(&mut self) -> bool {
        if self.path.pop().is_some() {
            self.hover = None;
            true
        } else {
            false
        }
    }

    /// Moves the highlight within the deepest panel, skipping separators.
    ///
    /// Wraps, because a menu is a ring: Down on the last row goes to the first,
    /// which is what every desktop menu does and what stops a user having to
    /// think about where the end is.
    pub fn step(&mut self, delta: isize) {
        let level = self.path.len();
        let count = self.level_items(level).len();
        if count == 0 {
            return;
        }
        let mut row = match self.hover {
            Some(row) => row as isize,
            None if delta > 0 => -1,
            None => count as isize,
        };
        for _ in 0..count {
            row += delta;
            if row < 0 {
                row = count as isize - 1;
            }
            if row >= count as isize {
                row = 0;
            }
            if !self.level_items(level)[row as usize].separator {
                self.hover = Some(row as usize);
                return;
            }
        }
    }

    /// Runs the highlighted row, or opens it when it is a submenu.
    pub fn activate(&mut self) -> Option<Action> {
        let level = self.path.len();
        let row = self.hover?;
        let (separator, submenu, action) = {
            let item = self.level_items(level).get(row)?;
            (item.separator, item.is_submenu(), item.action)
        };
        if separator {
            return None;
        }
        if submenu {
            self.path.push(row);
            self.hover = None;
            return None;
        }
        self.close();
        Some(action)
    }

    /// The level the highlight is on: 0 for the top dropdown.
    pub fn depth(&self) -> usize {
        self.path.len()
    }

    /// Queues the bar, the open dropdown, and their borders.
    pub fn push_quads(&self, quads: &mut QuadRenderer, width: f32, line_height: f32, theme: &Theme) {
        let bar = self.height(line_height);
        quads.push(
            0.0,
            0.0,
            width,
            bar,
            theme.menu_background.to_linear(),
        );
        quads.push(
            0.0,
            bar - 1.0,
            width,
            1.0,
            theme.menu_border.to_linear(),
        );

        if let Some(open) = self.open {
            quads.push(
                self.title_left(open),
                0.0,
                self.title_width(open),
                bar,
                theme.menu_hover.to_linear(),
            );
        }

    }

    /// The quads of the open dropdown, empty when nothing is open.
    ///
    /// These are queued in a separate batch drawn after the editor's glyphs,
    /// because a dropdown has to cover the text underneath it.
    pub fn push_dropdown_quads(
        &self,
        quads: &mut QuadRenderer,
        line_height: f32,
        theme: &Theme,
        width: f32,
    ) {
        let row_height = self.row_height(line_height);
        let border = theme.menu_border.to_linear();
        for (level, panel) in self.panels(width, line_height).iter().enumerate() {
            quads.push(
                panel.left,
                panel.top,
                panel.width,
                panel.height,
                theme.menu_background.to_linear(),
            );
            // A one pixel frame, drawn as four bars.
            quads.push(panel.left, panel.top, panel.width, 1.0, border);
            quads.push(
                panel.left,
                panel.top + panel.height - 1.0,
                panel.width,
                1.0,
                border,
            );
            quads.push(panel.left, panel.top, 1.0, panel.height, border);
            quads.push(
                panel.left + panel.width - 1.0,
                panel.top,
                1.0,
                panel.height,
                border,
            );

            for (row, item) in self.level_items(level).iter().enumerate() {
                let y = panel.top + 1.0 + row as f32 * row_height;
                if item.separator {
                    quads.push(
                        panel.left + 6.0,
                        y + row_height * 0.5,
                        panel.width - 12.0,
                        1.0,
                        border,
                    );
                    continue;
                }
                if self.is_highlighted(level, row) {
                    quads.push(
                        panel.left + 1.0,
                        y,
                        panel.width - 2.0,
                        row_height,
                        theme.menu_hover.to_linear(),
                    );
                }
            }
        }
    }

    /// Whether row `row` of `level` is drawn as highlighted.
    ///
    /// A row is lit when the pointer or the keyboard is on it, and also while its
    /// submenu is open: the parent stays lit so a user can see which branch of the
    /// tree they are in after the child panel has taken the highlight.
    fn is_highlighted(&self, level: usize, row: usize) -> bool {
        if level < self.path.len() {
            return self.path[level] == row;
        }
        self.hover == Some(row)
    }

    /// The text areas of the bar's own titles.
    pub fn text_areas<'a>(&'a self, width: f32, line_height: f32, theme: &Theme) -> Vec<TextArea<'a>> {
        let bar = self.height(line_height);
        let bounds = TextBounds {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: bar as i32,
        };
        let mut areas = Vec::with_capacity(self.menus.len());
        for (index, buffer) in self.titles.iter().enumerate() {
            areas.push(TextArea {
                buffer,
                left: self.title_left(index) + TITLE_PADDING,
                top: 3.0,
                scale: 1.0,
                bounds,
                default_color: theme.menu_text.to_cosmic(),
                custom_glyphs: &[],
            });
        }
        areas
    }

    /// The text areas of the open dropdown, empty when nothing is open.
    ///
    /// These are drawn in a second pass, after the editor's glyphs, because a
    /// dropdown has to cover the text underneath it.
    pub fn dropdown_areas<'a>(
        &'a self,
        line_height: f32,
        theme: &Theme,
        width: f32,
    ) -> Vec<TextArea<'a>> {
        let mut areas = Vec::with_capacity(32);
        let row_height = self.row_height(line_height);
        for (level, panel) in self.panels(width, line_height).iter().enumerate() {
            // The same clamp the background uses: a label drawn at the title's
            // left edge while the panel is pulled back would hang off the panel.
            let bounds = TextBounds {
                left: panel.left as i32,
                top: panel.top as i32,
                right: (panel.left + panel.width) as i32,
                bottom: (panel.top + panel.height) as i32,
            };
            let shaped = self.level_shaped(level);
            for (row, item) in self.level_items(level).iter().enumerate() {
                if item.separator {
                    continue;
                }
                let Some(shaped) = shaped.get(row) else {
                    continue;
                };
                let y = panel.top + 1.0 + row as f32 * row_height + ROW_EXTRA * 0.5;
                areas.push(TextArea {
                    buffer: &shaped.label,
                    left: panel.left + ROW_PADDING,
                    top: y,
                    scale: 1.0,
                    bounds,
                    default_color: theme.menu_text.to_cosmic(),
                    custom_glyphs: &[],
                });
                if !item.shortcut.is_empty() {
                    let shortcut_width = widest_line(&shaped.shortcut);
                    areas.push(TextArea {
                        buffer: &shaped.shortcut,
                        left: panel.left + panel.width - ROW_PADDING - shortcut_width,
                        top: y,
                        scale: 1.0,
                        bounds,
                        default_color: theme.menu_shortcut.to_cosmic(),
                        custom_glyphs: &[],
                    });
                }
                if item.is_submenu() {
                    areas.push(TextArea {
                        buffer: &self.arrow,
                        left: panel.left + panel.width - ROW_PADDING - ARROW_COLUMN + 4.0,
                        top: y,
                        scale: 1.0,
                        bounds,
                        default_color: theme.menu_shortcut.to_cosmic(),
                        custom_glyphs: &[],
                    });
                }
            }
        }

        areas
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar() -> MenuBar {
        MenuBar::new(
            Metrics::new(15.0, 20.0),
            Family::Monospace,
            8.0,
            Keymap::default(),
        )
    }

    /// Finds a row anywhere in a menu, submenus included.
    ///
    /// The Edit menu is almost entirely submenus now, so a test that looked only
    /// at the top level would be asserting about a menu that no longer exists.
    fn find(items: &[Item], action: Action) -> Option<&Item> {
        for item in items {
            if item.action == action {
                return Some(item);
            }
            if let Some(found) = find(&item.children, action) {
                return Some(found);
            }
        }
        None
    }

    /// Every row in a menu, submenus flattened in.
    fn flatten(items: &[Item]) -> Vec<&Item> {
        let mut out = Vec::new();
        for item in items {
            out.push(item);
            out.extend(flatten(&item.children));
        }
        out
    }

    /// A bar whose menus are given, so the submenu tests do not depend on what
    /// the real menus happen to contain this week.
    fn bar_with(menus: Vec<Menu>) -> MenuBar {
        let mut bar = bar();
        bar.menus = menus;
        bar.refresh_shortcuts();
        bar
    }

    /// One menu with a submenu in it, and a command after it. A second menu
    /// follows so the flip-at-the-edge test has a panel that does not start at
    /// x = 0, where there is nothing to flip towards.
    fn submenu_bar() -> MenuBar {
        bar_with(vec![
            Menu {
                title: "File".to_string(),
                english: "File",
                items: vec![
                    Item::new("New", Action::New),
                    Item::submenu(
                        "Open Containing Folder",
                        vec![
                            Item::new("Explorer", Action::OpenContainingFolder),
                            Item::separator(),
                            Item::new("cmd", Action::OpenContainingFolder),
                        ],
                    ),
                    Item::new("Save", Action::Save),
                ],
            },
            Menu {
                title: "Edit".to_string(),
                english: "Edit",
                items: vec![
                    Item::new("Undo", Action::Undo),
                    Item::submenu(
                        "Convert Case to",
                        vec![
                            Item::new("UPPERCASE", Action::UpperCase),
                            Item::new("lowercase", Action::LowerCase),
                        ],
                    ),
                ],
            },
        ])
    }

    /// The row of the open menu that the pointer would be over at `row`.
    fn row_y(bar: &MenuBar, row: usize, line_height: f32) -> f32 {
        let panel = bar.panels(900.0, line_height)[0];
        panel.top + 1.0 + row as f32 * bar.row_height(line_height) + 2.0
    }

    #[test]
    fn a_submenu_row_is_a_heading_not_a_command() {
        let bar = submenu_bar();
        let item = &bar.menus[0].items[1];
        assert!(item.is_submenu());
        assert_eq!(item.action, Action::Submenu);
        // No chord: the arrow is drawn where the shortcut column would be.
        assert!(item.shortcut.is_empty());
    }

    #[test]
    fn a_submenu_row_carries_its_children() {
        let bar = submenu_bar();
        let rows = bar.rows();
        assert_eq!(rows[0].items[1].children.len(), 3);
        assert_eq!(rows[0].items[1].children[0].label, "Explorer");
        assert!(rows[0].items[1].children[1].separator);
        assert!(rows[0].items[0].children.is_empty());
    }

    #[test]
    fn hovering_a_submenu_row_opens_it() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        let y = row_y(&bar, 1, 20.0);
        assert!(bar.hover_at(20.0, y, 20.0, 900.0));
        assert_eq!(bar.depth(), 1);
        assert_eq!(bar.panels(900.0, 20.0).len(), 2);
    }

    #[test]
    fn hovering_a_plain_row_does_not_open_anything() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        let y = row_y(&bar, 2, 20.0);
        assert!(bar.hover_at(20.0, y, 20.0, 900.0));
        assert_eq!(bar.depth(), 0);
    }

    #[test]
    fn moving_back_to_the_parent_row_closes_the_submenu() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        let submenu_y = row_y(&bar, 1, 20.0);
        bar.hover_at(20.0, submenu_y, 20.0, 900.0);
        assert_eq!(bar.depth(), 1);
        let plain_y = row_y(&bar, 2, 20.0);
        bar.hover_at(20.0, plain_y, 20.0, 900.0);
        assert_eq!(bar.depth(), 0);
    }

    #[test]
    fn clicking_a_submenu_row_opens_it_and_runs_nothing() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        let y = row_y(&bar, 1, 20.0);
        assert_eq!(bar.click(20.0, y, 20.0, 900.0), None);
        assert_eq!(bar.depth(), 1);
        assert!(bar.is_open());
    }

    #[test]
    fn clicking_a_row_inside_a_submenu_runs_it() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        let y = row_y(&bar, 1, 20.0);
        bar.click(20.0, y, 20.0, 900.0);
        let panel = bar.panels(900.0, 20.0)[1];
        let child_y = panel.top + 1.0 + 2.0 * bar.row_height(20.0) + 2.0;
        let x = panel.left + 10.0;
        assert_eq!(
            bar.click(x, child_y, 20.0, 900.0),
            Some(Action::OpenContainingFolder)
        );
        assert!(!bar.is_open());
    }

    #[test]
    fn a_submenu_opens_to_the_right_of_its_parent() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        let y = row_y(&bar, 1, 20.0);
        bar.hover_at(20.0, y, 20.0, 900.0);
        let panels = bar.panels(900.0, 20.0);
        assert!(panels[1].left > panels[0].left);
        // Overlapping the parent's right edge, so the two read as one object.
        assert!(panels[1].left < panels[0].left + panels[0].width);
    }

    #[test]
    fn a_submenu_with_no_room_on_the_right_opens_to_the_left() {
        let mut bar = submenu_bar();
        // The second menu, so its panel starts to the right of x = 0 and there
        // is somewhere for a flipped submenu to go.
        bar.open_menu(1);
        let y = row_y(&bar, 1, 20.0);
        let x = bar.panels(900.0, 20.0)[0].left + 10.0;
        bar.hover_at(x, y, 20.0, 900.0);
        let wide = bar.panels(900.0, 20.0);
        assert!(wide[0].left > 0.0);
        assert!(wide[1].left > wide[0].left);
        // A window just wide enough for the parent panel, so the submenu has
        // nowhere to go on the right but the parent still sits where it wants.
        let narrow = wide[0].left + wide[0].width + 10.0;
        let panels = bar.panels(narrow, 20.0);
        assert_eq!(panels[0].left, wide[0].left);
        assert!(panels[1].left < panels[0].left);
        assert!(panels[1].left >= 0.0);
    }

    #[test]
    fn back_closes_one_level_at_a_time() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        let y = row_y(&bar, 1, 20.0);
        bar.hover_at(20.0, y, 20.0, 900.0);
        assert_eq!(bar.depth(), 1);
        assert!(bar.back());
        assert_eq!(bar.depth(), 0);
        assert!(bar.is_open());
        assert!(!bar.back());
    }

    #[test]
    fn closing_the_menu_closes_every_submenu() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        let y = row_y(&bar, 1, 20.0);
        bar.hover_at(20.0, y, 20.0, 900.0);
        bar.close();
        assert_eq!(bar.depth(), 0);
        assert!(!bar.is_open());
    }

    #[test]
    fn stepping_skips_separators_and_wraps() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        bar.step(1);
        assert_eq!(bar.hover, Some(0));
        bar.step(1);
        assert_eq!(bar.hover, Some(1));
        bar.step(1);
        assert_eq!(bar.hover, Some(2));
        // Wraps to the top rather than stopping at the end.
        bar.step(1);
        assert_eq!(bar.hover, Some(0));
        bar.step(-1);
        assert_eq!(bar.hover, Some(2));
    }

    #[test]
    fn stepping_inside_a_submenu_skips_its_separator() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        bar.step(1);
        bar.step(1);
        assert!(bar.open_submenu());
        assert_eq!(bar.depth(), 1);
        bar.step(1);
        assert_eq!(bar.hover, Some(0));
        // Row 1 is a separator, so the next step lands on row 2.
        bar.step(1);
        assert_eq!(bar.hover, Some(2));
    }

    #[test]
    fn open_submenu_refuses_a_command() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        bar.step(1);
        assert_eq!(bar.hover, Some(0));
        assert!(!bar.open_submenu());
        assert_eq!(bar.depth(), 0);
    }

    #[test]
    fn activate_opens_a_submenu_rather_than_running_it() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        bar.step(1);
        bar.step(1);
        assert_eq!(bar.activate(), None);
        assert_eq!(bar.depth(), 1);
        assert!(bar.is_open());
    }

    #[test]
    fn activate_runs_a_command_and_closes_the_menu() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        bar.step(1);
        assert_eq!(bar.activate(), Some(Action::New));
        assert!(!bar.is_open());
    }

    #[test]
    fn a_panel_with_a_submenu_reserves_the_arrow_column() {
        let plain = bar_with(vec![Menu {
            title: "File".to_string(),
            english: "File",
            items: vec![Item::new("New", Action::New)],
        }]);
        let nested = submenu_bar();
        assert!(nested.dropdown_width(0) > plain.dropdown_width(0));
    }

    #[test]
    fn a_letter_opens_the_menu_that_starts_with_it() {
        let mut bar = submenu_bar();
        assert!(bar.open_by_letter('f'));
        assert_eq!(bar.open, Some(0));
        assert!(bar.open_by_letter('e'));
        assert_eq!(bar.open, Some(1));
    }

    #[test]
    fn a_letter_with_no_menu_is_refused() {
        let mut bar = submenu_bar();
        assert!(!bar.open_by_letter('z'));
        assert_eq!(bar.open, None);
    }

    #[test]
    fn a_shared_letter_cycles_through_the_menus_that_share_it() {
        let mut bar = bar_with(vec![
            Menu {
                title: "Edit".to_string(),
                english: "Edit",
                items: vec![Item::new("Undo", Action::Undo)],
            },
            Menu {
                title: "Encoding".to_string(),
                english: "Encoding",
                items: vec![Item::new("UTF-8", Action::EncodingSet(Encoding::Utf8))],
            },
        ]);
        assert!(bar.open_by_letter('e'));
        assert_eq!(bar.open, Some(0));
        // The same chord again moves to the other menu with that letter.
        assert!(bar.open_by_letter('e'));
        assert_eq!(bar.open, Some(1));
        assert!(bar.open_by_letter('e'));
        assert_eq!(bar.open, Some(0));
    }

    #[test]
    fn a_letter_that_names_one_menu_toggles_it() {
        let mut bar = submenu_bar();
        assert!(bar.open_by_letter('f'));
        assert!(bar.is_open());
        assert!(bar.open_by_letter('f'));
        assert!(!bar.is_open());
    }

    #[test]
    fn cycling_moves_along_the_bar_and_wraps() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        assert!(bar.cycle_menu(1));
        assert_eq!(bar.open, Some(1));
        assert!(bar.cycle_menu(1));
        assert_eq!(bar.open, Some(0));
        assert!(bar.cycle_menu(-1));
        assert_eq!(bar.open, Some(1));
    }

    #[test]
    fn cycling_with_nothing_open_does_nothing() {
        let mut bar = submenu_bar();
        assert!(!bar.cycle_menu(1));
        assert_eq!(bar.open, None);
    }

    #[test]
    fn cycling_closes_the_submenus_of_the_menu_it_leaves() {
        let mut bar = submenu_bar();
        bar.open_menu(0);
        let y = row_y(&bar, 1, 20.0);
        bar.hover_at(20.0, y, 20.0, 900.0);
        assert_eq!(bar.depth(), 1);
        bar.cycle_menu(1);
        assert_eq!(bar.depth(), 0);
        assert_eq!(bar.hover, None);
    }

    #[test]
    fn the_bar_is_taller_than_a_line() {
        assert_eq!(bar().height(20.0), 26.0);
    }

    #[test]
    fn titles_are_hit_tested_left_to_right() {
        let b = bar();
        assert_eq!(b.title_at(2.0), Some(0));
        let second = b.title_left(1);
        assert_eq!(b.title_at(second + 1.0), Some(1));
        assert_eq!(b.title_at(1e6), None);
    }

    #[test]
    fn clicking_a_title_opens_and_closes_it() {
        let mut b = bar();
        assert!(!b.is_open());
        b.click(2.0, 2.0, 20.0, 900.0);
        assert_eq!(b.open, Some(0));
        b.click(2.0, 2.0, 20.0, 900.0);
        assert!(!b.is_open());
    }

    #[test]
    fn clicking_another_title_switches_menus() {
        let mut b = bar();
        b.click(2.0, 2.0, 20.0, 900.0);
        let second = b.title_left(1) + 1.0;
        b.click(second, 2.0, 20.0, 900.0);
        assert_eq!(b.open, Some(1));
    }

    #[test]
    fn clicking_an_item_returns_its_action() {
        let mut b = bar();
        b.click(2.0, 2.0, 20.0, 900.0);
        // First row of File is New.
        let y = b.height(20.0) + 1.0 + 2.0;
        assert_eq!(b.click(20.0, y, 20.0, 900.0), Some(Action::New));
        assert!(!b.is_open());
    }

    #[test]
    fn clicking_a_separator_does_nothing() {
        let mut b = bar();
        b.click(2.0, 2.0, 20.0, 900.0);
        let row = b.row_height(20.0);
        let y = b.height(20.0) + 1.0 + 2.0 * row + 2.0;
        assert_eq!(b.click(20.0, y, 20.0, 900.0), None);
        assert!(b.is_open());
    }

    #[test]
    fn clicking_outside_closes_the_menu() {
        let mut b = bar();
        b.click(2.0, 2.0, 20.0, 900.0);
        b.click(900.0, 500.0, 20.0, 900.0);
        assert!(!b.is_open());
    }

    #[test]
    fn hovering_the_bar_switches_the_open_menu() {
        let mut b = bar();
        b.click(2.0, 2.0, 20.0, 900.0);
        let second = b.title_left(1) + 1.0;
        assert!(b.hover_at(second, 2.0, 20.0, 900.0));
        assert_eq!(b.open, Some(1));
    }

    #[test]
    fn hovering_a_row_highlights_it() {
        let mut b = bar();
        b.click(2.0, 2.0, 20.0, 900.0);
        let y = b.height(20.0) + 1.0 + 2.0;
        assert!(b.hover_at(20.0, y, 20.0, 900.0));
        assert_eq!(b.hover, Some(0));
    }

    #[test]
    fn a_dropdown_is_wide_enough_for_its_longest_row() {
        let b = bar();
        let width = b.dropdown_width(0);
        // "Save As..." plus "Ctrl+Shift+S" plus padding.
        assert!(width > 20.0 * 8.0);
    }

    #[test]
    fn a_dropdown_near_the_right_edge_is_pulled_back_inside_the_window() {
        let mut b = bar();
        b.prepare(&mut FontSystem::new());
        let last = b.menus.len() - 1;
        b.open_menu(last);
        let width = 400.0;
        let left = b.dropdown_left(last, width);
        assert!(left + b.dropdown_width(last) <= width + 0.01);
        // The text follows the panel: the first label starts inside it.
        let areas = b.dropdown_areas(20.0, &Theme::DARK, width);
        let first = areas.first().expect("a label");
        assert!(first.left >= left);
        assert!(first.left < left + b.dropdown_width(last));
    }

    #[test]
    fn a_dropdown_with_room_stays_under_its_title() {
        let mut b = bar();
        b.open_menu(0);
        assert_eq!(b.dropdown_left(0, 4000.0), b.title_left(0));
    }

    #[test]
    fn the_edit_menu_offers_word_completion() {
        let b = bar();
        let edit = &b.menus[1];
        assert_eq!(edit.title, "Edit");
        let item = find(&edit.items, Action::CompleteWord).expect("Edit menu has Word Completion");
        assert_eq!(item.label, "Word Completion");
        assert_eq!(item.shortcut, "Ctrl+Space");
    }

    #[test]
    fn the_shortcut_column_comes_from_the_keymap() {
        let mut b = bar();
        let mut keymap = Keymap::default();
        keymap.set(Action::Save, crate::keymap::Chord::parse("F5").unwrap());
        b.set_keymap(keymap);
        let save = b.menus[0]
            .items
            .iter()
            .find(|item| item.action == Action::Save)
            .expect("File menu has Save");
        assert_eq!(save.shortcut, "F5");
    }

    #[test]
    fn a_command_with_no_chord_shows_an_empty_column() {
        let b = bar();
        let sort = find(&b.menus[1].items, Action::SortLines(Sort::LexAsc))
            .expect("Edit > Line Operations has Sort Ascending");
        assert_eq!(sort.shortcut, "");
    }

    #[test]
    fn separators_never_carry_a_shortcut() {
        let b = bar();
        for menu in &b.menus {
            for item in flatten(&menu.items) {
                if item.separator {
                    assert_eq!(item.shortcut, "");
                }
            }
        }
    }

    #[test]
    fn every_advertised_shortcut_is_bound() {
        let b = bar();
        for menu in &b.menus {
            for item in flatten(&menu.items) {
                if item.separator || item.shortcut.is_empty() {
                    continue;
                }
                let chord = crate::keymap::Chord::parse(&item.shortcut)
                    .unwrap_or_else(|| panic!("{} has an unparsable chord", item.label));
                assert_eq!(
                    b.keymap.lookup(&chord),
                    Some(item.action),
                    "{} advertises {} but the keymap disagrees",
                    item.label,
                    item.shortcut
                );
            }
        }
    }

    #[test]
    fn the_encoding_menu_is_notepads_shape() {
        let items = encoding_items();
        let labels: Vec<&str> = items
            .iter()
            .filter(|item| !item.separator)
            .map(|item| item.label.as_str())
            .collect();
        assert_eq!(
            labels,
            [
                "ANSI",
                "UTF-8",
                "UTF-8-BOM",
                "UTF-16 BE BOM",
                "UTF-16 LE BOM",
                "Character sets",
                "Convert to ANSI",
                "Convert to UTF-8",
                "Convert to UTF-8-BOM",
                "Convert to UTF-16 BE BOM",
                "Convert to UTF-16 LE BOM",
            ]
        );
    }

    #[test]
    fn every_character_set_row_names_a_code_page() {
        let items = encoding_items();
        let sets = items
            .iter()
            .find(|item| item.label == "Character sets")
            .expect("the Character sets submenu");
        assert_eq!(sets.children.len(), CHARACTER_SETS.len());
        let mut seen = 0usize;
        for script in &sets.children {
            assert!(!script.children.is_empty(), "{} is empty", script.label);
            for row in &script.children {
                match row.action {
                    Action::EncodingSet(Encoding::CodePage(_)) => seen += 1,
                    other => panic!("{} is {other:?}", row.label),
                }
            }
        }
        // Every single-byte set is reachable from the menu, and none twice.
        assert_eq!(seen, CodePage::ALL.len());
    }

    #[test]
    fn an_encoding_key_reads_back_from_its_name() {
        for key in ["utf8", "utf8bom", "utf16le", "utf16be", "ansi"] {
            assert!(
                Action::from_name(&format!("encoding_set_{key}")).is_some(),
                "encoding_set_{key}"
            );
            assert!(
                Action::from_name(&format!("encoding_convert_{key}")).is_some(),
                "encoding_convert_{key}"
            );
        }
        for page in CodePage::ALL {
            let set = Action::from_name(&format!("encoding_set_{}", page.key()));
            assert_eq!(set, Some(Action::EncodingSet(Encoding::CodePage(page))));
            let convert = Action::from_name(&format!("encoding_convert_{}", page.key()));
            assert_eq!(
                convert,
                Some(Action::EncodingConvert(Encoding::CodePage(page)))
            );
        }
        assert_eq!(Action::from_name("encoding_set_nonsense"), None);
    }

    #[test]
    fn every_bindable_command_reads_back_from_its_name() {
        for action in Action::BINDABLE {
            assert_eq!(
                Action::from_name(action.name()),
                Some(*action),
                "{} does not read back",
                action.name()
            );
        }
    }

    #[test]
    fn the_text_commands_are_the_core_table() {
        // One list in the app, one in the core, and they have to be the same
        // list: this is the test that says so in both directions.
        // Two of the core's commands have no single menu row to route: `sort_lines`
        // is a submenu of fourteen rows that each carry a direction and a key,
        // and `split_lines` asks for its delimiter in a prompt before it runs.
        // Both are reachable from the API and neither can be reached by name from
        // the dispatcher, so the agreement is modulo these two.
        let api_only = ["sort_lines", "split_lines"];
        let ours: Vec<&str> = Action::TEXT_COMMANDS.iter().map(|one| one.name()).collect();
        for command in nn_core::ops::commands() {
            if api_only.contains(&command.name) {
                continue;
            }
            assert!(
                ours.contains(&command.name),
                "{} is in the core table but not in TEXT_COMMANDS",
                command.name
            );
        }
        for name in ours {
            assert!(
                nn_core::ops::knows(name),
                "{name} is a text command but the core has no such command"
            );
        }
        // And a text command is one the keymap can name, so it is bindable and
        // reachable from the menu rather than only from the API.
        for action in Action::TEXT_COMMANDS {
            assert!(
                Action::from_name(action.name()).is_some() || matches!(action, Action::SortLines(_)),
                "{} cannot be read back from its name",
                action.name()
            );
        }
    }

    #[test]
    fn the_bindable_list_has_no_duplicates() {
        let mut seen: Vec<&str> = Vec::new();
        for action in Action::BINDABLE {
            assert!(!seen.contains(&action.name()), "{} twice", action.name());
            seen.push(action.name());
        }
    }

    /// A catalog that renames a handful of rows, built the way a real
    /// localization file is: numeric ids joined against an English reference.
    fn catalog() -> Catalog {
        let english = "<NotepadPlus><Native-Langue name=\"English\"><Menu>\
            <Main><Item id=\"0\" name=\"File\"/><Item id=\"1\" name=\"Edit\"/>\
            <Item id=\"2\" name=\"New\"/></Main>\
            <SubEntries><Item id=\"0\" name=\"File\"/></SubEntries>\
            </Menu></Native-Langue></NotepadPlus>";
        let french = "<NotepadPlus><Native-Langue name=\"Francais\"><Menu>\
            <Main><Item id=\"0\" name=\"Fichier\"/><Item id=\"1\" name=\"Edition\"/>\
            <Item id=\"2\" name=\"Nouveau\"/></Main>\
            <SubEntries><Item id=\"0\" name=\"Fichier\"/></SubEntries>\
            </Menu></Native-Langue></NotepadPlus>";
        Catalog::parse(Some(english), french).expect("a catalog")
    }

    #[test]
    fn a_catalog_relabels_the_titles_and_the_rows() {
        let mut b = bar();
        b.set_catalog(Some(catalog()));
        assert_eq!(b.menus[0].title, "Fichier");
        assert_eq!(b.menus[0].items[0].label, "Nouveau");
        // A row the catalog does not mention keeps its English text.
        assert_eq!(b.menus[0].items[1].label, "Open...");
        assert_eq!(b.catalog_name(), Some("Francais"));
    }

    #[test]
    fn clearing_the_catalog_restores_every_english_label() {
        let mut b = bar();
        let before: Vec<String> = b.menus[0].items.iter().map(|i| i.label.clone()).collect();
        b.set_catalog(Some(catalog()));
        b.set_catalog(None);
        let after: Vec<String> = b.menus[0].items.iter().map(|i| i.label.clone()).collect();
        assert_eq!(before, after);
        assert_eq!(b.menus[0].title, "File");
        assert_eq!(b.catalog_name(), None);
    }

    #[test]
    fn switching_languages_twice_does_not_translate_a_translation() {
        // The second catalog is applied to the stored English text, not to the
        // French that is currently on screen.
        let mut b = bar();
        b.set_catalog(Some(catalog()));
        b.set_catalog(Some(catalog()));
        assert_eq!(b.menus[0].title, "Fichier");
        assert_eq!(b.menus[0].items[0].label, "Nouveau");
    }

    #[test]
    fn a_rebuilt_list_is_relabelled_too() {
        // `set_recent` rebuilds the File menu from English literals; the
        // catalog has to survive that.
        let mut b = bar();
        b.set_catalog(Some(catalog()));
        b.set_recent(&["notes.txt".to_string()]);
        assert_eq!(b.menus[0].title, "Fichier");
        assert_eq!(b.menus[0].items[0].label, "Nouveau");
        assert!(b.menus[0].items.iter().any(|i| i.label == "notes.txt"));
    }

    #[test]
    fn a_catalog_does_not_change_which_action_a_row_runs() {
        let mut b = bar();
        let before: Vec<Action> = b.menus[0].items.iter().map(|i| i.action).collect();
        b.set_catalog(Some(catalog()));
        let after: Vec<Action> = b.menus[0].items.iter().map(|i| i.action).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn the_bar_is_notepads_thirteen_in_notepads_order() {
        let b = bar();
        let titles: Vec<&str> = b.menus.iter().map(|menu| menu.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "File", "Edit", "Search", "View", "Encoding", "Language", "Settings", "Tools",
                "Macro", "Run", "Plugins", "Window", "Help",
            ]
        );
        // The three menus reached by index have to agree with that order.
        assert_eq!(titles[LANGUAGE_MENU], "Language");
        assert_eq!(titles[RUN_MENU], "Run");
        assert_eq!(titles[PLUGIN_MENU], "Plugins");
    }

    #[test]
    fn the_view_menu_is_notepads_shape() {
        let items = view_items();
        // Playground is ours: Notepad++ has no row there. Everything else keeps
        // Notepad++'s order, which is what the rest of this list is about.
        let labels: Vec<&str> = items
            .iter()
            .filter(|item| !item.separator)
            .map(|item| item.label.as_str())
            .filter(|label| *label != "Playground")
            .collect();
        assert_eq!(
            labels,
            [
                "Always on Top",
                "Toggle Full Screen Mode",
                "Post-It",
                "Distraction Free Mode",
                "View Current File in",
                "Show Symbol",
                "Zoom",
                "Move/Clone Current Document",
                "Tab",
                "Word wrap",
                "Focus on Another View",
                "Hide Lines",
                "Fold All",
                "Unfold All",
                "Fold Current Level",
                "Unfold Current Level",
                "Fold Level",
                "Unfold Level",
                "Summary...",
                "Project Panels",
                "Folder as Workspace",
                "Document Map",
                "Document List",
                "Function List",
                "Synchronize Vertical Scrolling",
                "Synchronize Horizontal Scrolling",
                "Monitoring (tail -f)",
            ]
        );
    }

    #[test]
    fn the_playground_row_sits_under_distraction_free_mode() {
        let items = view_items();
        let at = items
            .iter()
            .position(|item| item.action == Action::Playground)
            .expect("a Playground row");
        assert_eq!(items[at - 1].action, Action::DistractionFree);
    }

    #[test]
    fn the_view_menu_has_no_text_direction_rows() {
        // The renderer has no bidi layout, so the two rows Notepad++ has would be
        // switches nothing reads. Their absence is a decision, not an oversight.
        let items = view_items();
        assert!(find(&items, Action::AlwaysOnTop).is_some());
        let labels: Vec<&str> = flatten(&items).iter().map(|i| i.label.as_str()).collect();
        assert!(!labels.iter().any(|label| label.starts_with("Text Direction")));
    }

    #[test]
    fn the_tab_submenu_counts_to_nine_and_paints_five() {
        let items = view_items();
        let submenu = items
            .iter()
            .find(|item| item.label == "Tab")
            .expect("a Tab row");
        let rows = &submenu.children;
        let gotos: Vec<usize> = rows
            .iter()
            .filter_map(|row| match row.action {
                Action::GotoTab(index) => Some(index),
                _ => None,
            })
            .collect();
        assert_eq!(gotos, (0..9).collect::<Vec<usize>>());
        let paints = rows
            .iter()
            .filter(|row| matches!(row.action, Action::TabColourSet(_)))
            .count();
        assert_eq!(paints, 6);
    }

    #[test]
    fn the_fold_level_submenus_go_from_one_to_eight() {
        let items = view_items();
        for (label, make) in [
            ("Fold Level", Action::FoldLevel as fn(usize) -> Action),
            ("Unfold Level", Action::UnfoldLevel as fn(usize) -> Action),
        ] {
            let row = items
                .iter()
                .find(|item| item.label == label)
                .expect("the submenu");
            let rows = &row.children;
            let levels: Vec<usize> = rows
                .iter()
                .filter_map(|row| match row.action {
                    Action::FoldLevel(level) | Action::UnfoldLevel(level) => Some(level),
                    _ => None,
                })
                .collect();
            assert_eq!(levels, (1..=8).collect::<Vec<usize>>());
            assert_eq!(rows[0].action, make(1));
        }
    }

    #[test]
    fn every_browser_row_names_a_program() {
        for kind in BrowserKind::all() {
            assert!(!kind.command().is_empty());
            assert!(!kind.label().is_empty());
        }
    }

    #[test]
    fn the_settings_menu_matches_notepads_shape() {
        let b = bar();
        let items = &b.menus[6].items;
        let labels: Vec<&str> = items.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Preferences...",
                "Style Configurator...",
                "Theme",
                "Shortcut Mapper",
                "",
                "Import",
                "",
                "Edit Popup ContextMenu",
            ]
        );
        assert_eq!(items[0].action, Action::Preferences);
        assert_eq!(items[1].action, Action::StyleConfigurator);
        assert!(items[4].separator);
        assert!(items[6].separator);
        assert_eq!(items[7].action, Action::EditContextMenu);
    }

    #[test]
    fn every_theme_is_reachable_from_the_settings_menu() {
        // The Preferences row steps through the same list one arrow press at a
        // time; the submenu is what makes a theme findable by name.
        let b = bar();
        let items = &b.menus[6].items;
        let theme = items
            .iter()
            .find(|item| item.label == "Theme")
            .expect("the theme submenu");
        let actions: Vec<Action> = theme.children.iter().map(|row| row.action).collect();
        let expected: Vec<Action> = (0..ThemeChoice::ALL.len()).map(Action::Theme).collect();
        assert_eq!(actions, expected);
        assert_eq!(
            theme.children[0].label,
            ThemeChoice::ALL[0].label(),
            "the rows are labelled in the same order the row steps"
        );
        assert_eq!(theme.children.len(), ThemeChoice::ALL.len());
    }

    #[test]
    fn a_theme_row_round_trips_through_its_shortcut_key() {
        // The rows are addressable, so a shortcut can name one; that only works
        // if the key is unique per theme and parses back to the same index.
        for (index, choice) in ThemeChoice::ALL.iter().enumerate() {
            let key = format!("theme_{}", choice.key());
            assert_eq!(Action::from_name(&key), Some(Action::Theme(index)), "{key}");
        }
        assert_eq!(Action::from_name("theme_nonexistent"), None);
    }

    #[test]
    fn the_two_imports_are_under_the_import_submenu() {
        let b = bar();
        let items = &b.menus[6].items;
        let import = items.iter().find(|item| item.label == "Import").unwrap();
        let actions: Vec<Action> = import.children.iter().map(|row| row.action).collect();
        assert_eq!(actions, [Action::ImportPlugin, Action::ImportTheme]);
    }

    #[test]
    fn the_shortcut_mapper_keeps_the_import_and_export_reachable() {
        // Notepad++ puts these two on the mapper dialog's own buttons. There is
        // no such dialog here, so they are rows under the mapper instead of
        // being dropped.
        let b = bar();
        let items = &b.menus[6].items;
        let mapper = items.iter().find(|item| item.label == "Shortcut Mapper").unwrap();
        let actions: Vec<Action> = mapper
            .children
            .iter()
            .filter(|row| !row.separator)
            .map(|row| row.action)
            .collect();
        assert_eq!(
            actions,
            [Action::ShortcutMapper, Action::ImportNotepad, Action::ExportShortcuts]
        );
    }

    #[test]
    fn the_language_menu_starts_with_plain_text() {
        let b = bar();
        let items = &b.menus[LANGUAGE_MENU].items;
        assert_eq!(items[0].label, "None (Normal Text)");
        assert_eq!(items[0].action, Action::LanguagePlain);
        assert!(items[1].separator);
    }

    #[test]
    fn every_built_in_language_is_reachable_from_the_menu() {
        // The menu groups the table by first letter, so a language could be
        // dropped by a grouping bug without any single row looking wrong.
        let b = bar();
        let items = &b.menus[LANGUAGE_MENU].items;
        let mut seen = Vec::new();
        for item in items {
            // The three ungrouped rows are top-level; the rest are children.
            for row in std::iter::once(item).chain(&item.children) {
                if let Action::LanguageBuiltIn(index) = row.action {
                    seen.push(index);
                }
            }
        }
        seen.sort_unstable();
        let expected: Vec<usize> = (0..nn_core::LANGS.len()).collect();
        assert_eq!(seen, expected);
    }

    #[test]
    fn the_language_groups_are_lettered_and_in_order() {
        let b = bar();
        let items = &b.menus[LANGUAGE_MENU].items;
        let letters: Vec<&str> = items
            .iter()
            .filter(|item| !item.children.is_empty())
            .map(|item| item.label.as_str())
            .collect();
        // `User Defined Language` and `User-Defined` are submenus too, so the
        // letter groups are the ones that are a single character.
        let letters: Vec<&str> = letters
            .into_iter()
            .filter(|label| label.chars().count() == 1)
            .collect();
        assert_eq!(
            letters,
            [
                "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "R",
                "S", "T", "V", "W", "Z"
            ]
        );
    }

    #[test]
    fn the_three_ungrouped_languages_sit_where_notepad_puts_them() {
        let b = bar();
        let items = &b.menus[LANGUAGE_MENU].items;
        let labels: Vec<&str> = items.iter().map(|item| item.label.as_str()).collect();
        let at = |label: &str| labels.iter().position(|l| *l == label).unwrap();
        // `KIXtart` between the `J` and `L` groups, `XML` and `YAML` after `V`.
        assert!(at("J") < at("KIXtart") && at("KIXtart") < at("L"));
        assert!(at("V") < at("XML") && at("XML") < at("YAML"));
    }

    #[test]
    fn a_user_defined_language_becomes_a_row_under_its_own_submenu() {
        let mut b = bar();
        b.set_languages(&["Logfile".to_string(), "Ini".to_string()]);
        let items = &b.menus[LANGUAGE_MENU].items;
        let udl = items
            .iter()
            .find(|item| item.label == "User-Defined")
            .expect("the User-Defined submenu is always there");
        assert_eq!(udl.children.len(), 2);
        assert_eq!(udl.children[0].label, "Logfile");
        assert_eq!(udl.children[0].action, Action::Language(0));
        assert_eq!(udl.children[1].action, Action::Language(1));
    }

    #[test]
    fn an_empty_user_defined_submenu_says_so() {
        let b = bar();
        let items = &b.menus[LANGUAGE_MENU].items;
        let udl = items.iter().find(|item| item.label == "User-Defined").unwrap();
        assert_eq!(udl.children.len(), 1);
        assert_eq!(udl.children[0].action, Action::NoOp);
    }

    #[test]
    fn the_language_menu_ends_with_the_three_management_commands() {
        let b = bar();
        let items = &b.menus[LANGUAGE_MENU].items;
        let manage = items
            .iter()
            .find(|item| item.label == "User Defined Language")
            .expect("the management submenu is always there");
        let actions: Vec<Action> = manage.children.iter().map(|row| row.action).collect();
        assert_eq!(
            actions,
            [Action::DefineLanguage, Action::OpenUdlFolder, Action::UdlCollection]
        );
    }

    #[test]
    fn a_built_in_language_round_trips_through_its_key() {
        for (index, spec) in nn_core::LANGS.iter().enumerate() {
            let key = format!("language_built_in_{}", spec.key());
            assert_eq!(Action::from_name(&key), Some(Action::LanguageBuiltIn(index)), "{key}");
        }
        assert_eq!(Action::from_name("language_built_in_klingon"), None);
    }

    #[test]
    fn setting_the_same_languages_twice_keeps_the_menu_closed() {
        let mut b = bar();
        b.set_languages(&["Logfile".to_string()]);
        b.open_menu(LANGUAGE_MENU);
        assert!(b.is_open());
        b.set_languages(&["Logfile".to_string()]);
        assert!(b.is_open());
        b.set_languages(&["Other".to_string()]);
        assert!(!b.is_open());
    }

    #[test]
    fn the_plugins_menu_sits_between_run_and_window() {
        let b = bar();
        let titles: Vec<&str> = b.menus.iter().map(|menu| menu.title.as_str()).collect();
        assert_eq!(titles[PLUGIN_MENU], "Plugins");
        assert_eq!(titles[PLUGIN_MENU - 1], "Run");
        assert_eq!(titles[PLUGIN_MENU + 1], "Window");
        assert_eq!(titles[PLUGIN_MENU + 2], "Help");
    }

    #[test]
    fn an_empty_plugins_menu_says_so() {
        let b = bar();
        let items = &b.menus[PLUGIN_MENU].items;
        // The manager and folder rows are always first, so "no plugins" is
        // still reachable from a menu that has nothing in it.
        assert_eq!(items[0].label, "Manage Plugins...");
        assert_eq!(items[0].action, Action::ManagePlugins);
        assert_eq!(items[1].label, "Open Plugins Folder...");
        assert_eq!(items[1].action, Action::OpenPluginsFolder);
        assert_eq!(items[2].label, "Reload Plugins");
        assert_eq!(items[2].action, Action::ReloadPlugins);
        assert!(items[3].separator);
        assert_eq!(items[4].label, "No Plugins Installed");
        assert_eq!(items[4].action, Action::NoOp);
        assert_eq!(items.len(), 5);
    }

    /// A plugin list as `set_plugins` takes one, in inventory order.
    fn plugins(rows: &[(&str, bool, &[&str])]) -> Vec<PluginMenu> {
        rows.iter()
            .map(|(name, enabled, commands)| PluginMenu {
                name: (*name).to_string(),
                enabled: *enabled,
                commands: commands.iter().map(|c| (*c).to_string()).collect(),
            })
            .collect()
    }

    #[test]
    fn a_folder_with_plugins_in_it_gets_no_note() {
        let mut b = bar();
        b.set_plugins(&plugins(&[("JSON", false, &["Minify"])]));
        let items = &b.menus[PLUGIN_MENU].items;
        // The folder is not empty, so "No Plugins Installed" would be a lie, and
        // the manager above lists every plugin whether it runs or not. The one
        // plugin is switched off, so it gets no submenu either: three rows.
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].label, "Manage Plugins...");
        assert!(items.iter().all(|item| item.label != "No Plugins Installed"));
    }

    #[test]
    fn each_running_plugin_opens_onto_its_commands() {
        let mut b = bar();
        b.set_plugins(&plugins(&[
            ("JSON", true, &["Pretty Print", "Minify", "Validate"]),
            ("XML", false, &["Pretty Print", "Check Well-Formedness"]),
        ]));
        let items = &b.menus[PLUGIN_MENU].items;
        // The three management rows, a separator, then one submenu per running
        // plugin -- and none for the one that is switched off, whose commands
        // do not exist until it is.
        assert_eq!(items.len(), 5);
        assert!(items[3].separator);
        assert!(items[4].is_submenu());
        assert_eq!(items[4].label, "JSON");
        assert_eq!(items[4].action, Action::Submenu);
        let commands: Vec<(&str, Action)> = items[4]
            .children
            .iter()
            .map(|item| (item.label.as_str(), item.action))
            .collect();
        assert_eq!(
            commands,
            vec![
                ("Pretty Print", Action::Plugin(0, 0)),
                ("Minify", Action::Plugin(0, 1)),
                ("Validate", Action::Plugin(0, 2)),
            ]
        );
    }

    #[test]
    fn a_plugin_that_is_switched_off_gets_no_submenu() {
        let mut b = bar();
        b.set_plugins(&plugins(&[
            ("JSON", false, &["Minify"]),
            ("XML", true, &["Linearize"]),
        ]));
        let items = &b.menus[PLUGIN_MENU].items;
        // The submenu that is there belongs to the second plugin, and its rows
        // carry the index the inventory gave that plugin, not the row above it.
        assert_eq!(items.len(), 5);
        assert_eq!(items[4].label, "XML");
        assert_eq!(items[4].children[0].action, Action::Plugin(1, 0));
    }

    #[test]
    fn a_plugin_submenu_keeps_the_plugins_words_in_every_language() {
        // The trap this exists for: a command named after a menu word. "Undo"
        // is a catalog entry, and a translated plugin row would name a command
        // the plugin does not have.
        let italian = crate::i18n::Catalog::load(None, "italian.xml").expect("italian.xml");
        let mut b = bar();
        b.set_catalog(Some(italian));
        b.set_plugins(&plugins(&[("JSON", true, &["Undo", "Minify"])]));
        let items = &b.menus[PLUGIN_MENU].items;
        assert_eq!(items[0].label, "Gestisci i plugin...");
        assert_eq!(items[4].label, "JSON");
        assert_eq!(items[4].children[0].label, "Undo");
        // The same word in the Edit menu, which the catalog does own.
        assert_eq!(find(&b.menus[1].items, Action::Undo).unwrap().label, "Annulla");
        // A plugin's name and its commands are not phrases of the program's, so
        // they are not labels a catalog owes a translation for either. A name no
        // menu uses, because a plugin called "JSON" would find one in the
        // Language menu and prove nothing.
        b.set_plugins(&plugins(&[("Zebra", true, &["Zebra Command"])]));
        assert!(!b.english_labels().contains(&"Zebra"));
        assert!(!b.english_labels().contains(&"Zebra Command"));
        assert!(b.english_labels().contains(&"Manage Plugins..."));
    }

    #[test]
    fn the_italian_catalog_translates_the_manager_row() {
        // The same path the menu takes at startup: a real catalog, loaded from
        // the bundled copy. The manager row is the only plugin row left that
        // carries a translated verb, so a catalog that missed it would put an
        // English word inside an otherwise Italian dropdown.
        let italian = crate::i18n::Catalog::load(None, "italian.xml").expect("italian.xml");
        let mut b = bar();
        b.set_catalog(Some(italian));
        assert_eq!(b.menus[PLUGIN_MENU].items[0].label, "Gestisci i plugin...");
        // A language change rebuilds the rows from the list, so the note comes
        // back translated rather than from a stale English label.
        b.set_plugins(&[]);
        assert_eq!(
            b.menus[PLUGIN_MENU].items[4].label,
            "Nessun plugin installato"
        );
    }

    #[test]
    fn the_plugin_rows_that_take_no_index_are_bindable() {
        // Reloading takes no index, so it can be bound like anything else.
        assert!(Action::BINDABLE.contains(&Action::ReloadPlugins));
        assert_eq!(Action::from_name("reload_plugins"), Some(Action::ReloadPlugins));
        // Run-this-command-of-this-plugin takes two indices, and a saved chord
        // would point at whatever plugin happened to be there after an install.
        // It is a row in a submenu and nothing else.
        assert!(!Action::BINDABLE.contains(&Action::Plugin(0, 0)));
        assert_eq!(Action::from_name("plugin"), None);
        // Switching one plugin on or off is the manager's row: there is no id
        // for it, so no chord can name it.
        assert_eq!(Action::from_name("load_plugin"), None);
    }

    #[test]
    fn a_language_row_is_not_bindable() {
        // A chord that highlighted "the third language" would mean something
        // different after every import, the same reason OpenRecent is out.
        assert!(!Action::BINDABLE.contains(&Action::Language(0)));
        assert_eq!(Action::from_name("language"), None);
    }

    #[test]
    fn a_label_is_the_name_in_words() {
        assert_eq!(Action::Save.label(), "Save");
        assert_eq!(Action::SaveAs.label(), "Save As");
        assert_eq!(Action::ToggleFunctionList.label(), "Toggle Function List");
        assert_eq!(Action::MoveLineDown.label(), "Move Line Down");
    }

    /// Whether a label is the same word in every language.
    ///
    /// Palette names, programming language names, encoding names, submenu
    /// letters and digits are proper nouns: a French user's copy of "HTML" says
    /// "HTML". The case-conversion rows are samples of the effect rather than
    /// words. None of them has, or should have, an entry in a translation file.
    fn is_literal(label: &str) -> bool {
        if label.chars().count() == 1 {
            return true;
        }
        const SAMPLES: [&str; 5] = ["cmd", "lowercase", "UPPERCASE", "iNVERT cASE", "ranDOm CasE"];
        if SAMPLES.contains(&label) {
            return true;
        }
        if crate::themes::CATALOGUE.iter().any(|p| p.name == label) {
            return true;
        }
        nn_core::LANGS.iter().any(|lang| lang.name == label)
    }

    /// Every label every bundled language has to translate.
    ///
    /// The mix this exists to catch: a menu bar that says "Fichier" at the top
    /// and "Fold All" inside, because the translation file predates the row.
    /// Nothing in the bar may fall back to English except a proper noun.
    #[test]
    fn every_bundled_language_translates_every_menu_label() {
        let bar = bar();
        for (file, source) in crate::i18n::BUNDLED {
            let catalog = Catalog::parse(None, source).expect(file);
            let missing: Vec<&str> = bar
                .english_labels()
                .into_iter()
                .filter(|label| !is_literal(label))
                .filter(|label| !catalog.translates(label))
                .collect();
            assert!(missing.is_empty(), "{file} leaves {} in English: {missing:?}", missing.len());
        }
    }

    /// The same question from the other side: a label nobody wrote a row for.
    ///
    /// A menu row added without a translation would pass the test above only if
    /// the catalog happened to know the word, which it would not; this one names
    /// the row instead of the file.
    #[test]
    fn every_prose_label_in_the_bar_is_translated_by_every_language() {
        let bar = bar();
        let mut prose: Vec<&str> = bar
            .english_labels()
            .into_iter()
            .filter(|label| !is_literal(label))
            .collect();
        prose.sort_unstable();
        prose.dedup();
        assert!(prose.len() > 350, "{} prose labels", prose.len());
    }

    #[test]
    fn the_help_menu_opens_the_licence_between_two_rules() {
        // The terms are not a page about this project, so the row is a section
        // of its own: a rule above it and a rule below it, and the page rows
        // stay where they were.
        let bar = bar();
        let help = bar.menus.last().expect("no menus");
        let index = help
            .items
            .iter()
            .position(|item| item.action == Action::License)
            .expect("the Help menu has no licence row");
        assert_eq!(help.items[index].label, "Neko++ License (GPLv3)");
        assert!(help.items[index - 1].separator, "no rule above the licence");
        assert!(help.items[index + 1].separator, "no rule below the licence");
        // It is a link, not a command: nothing to bind, and no chord of its own.
        assert!(!Action::BINDABLE.contains(&Action::License));
        assert!(help.items[index].shortcut.is_empty());
    }
}
