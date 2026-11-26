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

//! Every command the menus offer.
//!
//! An `Action` is what a menu row, a key binding or an automation call resolves
//! to before anything happens. It is a value rather than a function pointer so
//! that the same command can be named in a settings file (`save_as`), listed in
//! the shortcut editor (`Save As`), bound to a chord, offered by a menu and
//! called over the API -- and so that the dispatch that finally runs it is a
//! single `match` the compiler checks for exhaustiveness.
//!
//! Payloads are engine types where the command needs one ([`Encoding`],
//! [`nn_core::hash::Hash`]) and plain numbers where the command is an index
//! into a list the front-end builds (a recent file, a plugin's command). None
//! of them names a window or a terminal, which is what lets both front-ends
//! share the vocabulary and keeps this file free of a drawing dependency.

use nn_core::{CodePage, Encoding};

/// The encoding a saved shortcut's key names.
///
/// The forty-one code pages are matched here rather than as ninety-two literal
/// arms in `from_name`, because the key is the code page's own key and a second
/// copy of the list would be a second thing to keep in step.
pub fn encoding_from_key(key: &str) -> Option<Encoding> {
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
                // A theme is deliberately not read here. `theme_<key>` names an
                // entry of the settings' own list, and the list is a hundred
                // and fifty lines of palette names that no command needs to
                // know; the front-end that owns the setting resolves that
                // prefix first and falls back to this function. The vocabulary
                // still carries the action -- `Action::Theme(n)` -- because a
                // menu row and a chord have to be able to mean it.
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

#[cfg(test)]
mod tests {
    use super::*;
    use nn_core::CodePage;

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
}
