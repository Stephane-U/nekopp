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

//! The menus, as a tree of rows rather than as pixels.
//!
//! A `Menu` is a titled list; an `Item` is one row: a label, the chord drawn
//! beside it, and the [`Action`] it runs -- or, when it has children, the
//! submenu it opens. Nothing here knows how wide a character is, where the panel
//! lands on screen, or what colour a disabled row is. The windowed front-end
//! shapes these labels and draws the boxes; the terminal front-end writes them
//! into cells. Both read the same tree, which is what makes "any feature added
//! to the windowed version is converted to the terminal version" a property of
//! the code rather than a promise.
//!
//! The builders below are the tree itself: one function per dropdown, each
//! returning its rows in the order a person reads them, with separators where
//! Notepad++ has them. Three of them need something from the outside world --
//! the recent file list, the saved run commands, the languages the user defined
//! -- and take it as an argument rather than reaching for it. The Plugins menu is
//! the exception that stays behind in the front-end: its rows are composed from a
//! catalogue of translations, which the crate that owns translations owns.
//!
//! A row's label is stored twice, in English and as displayed, because a language
//! change relabels a menu in place and because "did this list change?" is a
//! question about the English text: a rebuilt recent-file list has to be compared
//! with the same words it was built from, not with whatever the current language
//! made of them.

use nn_core::{CodePage, Encoding};

use crate::action::{Action, BrowserKind, MatchMode, Sort, WindowSort};
use crate::theme::ThemeChoice;

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
pub struct Item {
    pub label: String,
    /// The untranslated label.
    ///
    /// Kept beside the displayed one so a language change relabels the row
    /// without rebuilding the menu, and so the "did the list change?" checks in
    /// `set_recent` and friends compare like with like whatever the language.
    pub english: String,
    pub shortcut: String,
    pub action: Action,
    /// Draws a separator line above this row instead of a label.
    pub separator: bool,
    /// Whether the label is a word of its own that no catalog translates.
    ///
    /// A plugin's name and the names of its commands are the plugin's text. A
    /// catalog that happens to hold the same string -- "Cancel", say -- would
    /// otherwise rewrite a row into a command the plugin does not have.
    pub literal: bool,
    /// The rows this row opens, empty for a command.
    ///
    /// A submenu is an `Item` with children rather than a second type, because
    /// every row is laid out, hit-tested, translated and drawn the same way. The
    /// only difference is what a click does, and that is one branch.
    pub children: Vec<Item>,
}

impl Item {
    pub fn new(label: impl Into<String>, action: Action) -> Self {
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

    pub fn separator() -> Self {
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
    pub fn composed(english: String, label: String, action: Action) -> Self {
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
    pub fn submenu(label: impl Into<String>, children: Vec<Item>) -> Self {
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
    pub fn literal(label: impl Into<String>, action: Action) -> Self {
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
    pub fn literal_submenu(label: impl Into<String>, children: Vec<Item>) -> Self {
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
    pub fn is_submenu(&self) -> bool {
        !self.children.is_empty()
    }
}

/// One top-level menu.
pub struct Menu {
    pub title: String,
    /// The untranslated title, for the same reason `Item::english` exists.
    pub english: &'static str,
    pub items: Vec<Item>,
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

/// Where the menus that are filled from outside sit in the bar.
///
/// The index matters to both front-ends: a rebuild of the recent-file list, the
/// run list, the language list or the plugin list replaces one menu's rows, and
/// finding it by title would break the moment a catalogue translated the title.
/// The action a settings file names, theme rows included.
///
/// [`Action::from_name`] answers what a *command* is called; a theme is a
/// setting, and `theme_<key>` is resolved against [`ThemeChoice::ALL`] here
/// rather than in the shared vocabulary, because the list of fifty themes is
/// palette names that no command has a use for. The order the two are tried in
/// cannot be observed: no name starts with both `theme_` and another prefix.
pub fn action_from_name(name: &str) -> Option<Action> {
    if let Some(key) = name.strip_prefix("theme_") {
        return ThemeChoice::ALL
            .iter()
            .position(|choice| choice.key() == key)
            .map(Action::Theme);
    }
    Action::from_name(name)
}

pub const LANGUAGE_MENU: usize = 5;
pub const RUN_MENU: usize = 9;
pub const PLUGIN_MENU: usize = 10;

/// What the builders cannot answer for themselves.
///
/// Four menus hold rows that come from the disk or from the settings -- the files
/// that were open lately, the commands a user saved, the languages they defined,
/// and the themes the settings offer -- and each builder takes its list rather
/// than reaching for it. This is the bundle a caller fills in, so that adding a
/// fifth such menu is a field here and a compile error at every call site instead
/// of a fifth argument threaded through by hand.
#[derive(Debug, Clone, Copy, Default)]
pub struct Contents<'a> {
    /// The recently opened files, most recent first.
    pub recent: &'a [String],
    /// The saved run commands, in the order the user saved them.
    pub runs: &'a [String],
    /// The user-defined languages, in the order the folder listing gave them.
    pub languages: &'a [String],
    /// The themes, in the order the preference row steps through them.
    pub themes: &'a [&'a str],
}

/// The whole menu bar, in bar order.
///
/// The Plugins menu is the one that stays empty: its rows are built by the
/// front-end that owns the plugin host, because they are composed from a
/// catalogue of translations and from names the plugins give themselves. An empty
/// item list is a menu with nothing in it rather than a row saying so, and both
/// front-ends fill it from the same `plugin` rows when they have them.
pub fn menus(contents: Contents<'_>) -> Vec<Menu> {
    vec![
        Menu {
            title: "File".to_string(),
            english: "File",
            items: file_items(contents.recent),
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
            items: language_items(contents.languages),
        },
        Menu {
            title: "Settings".to_string(),
            english: "Settings",
            items: settings_items(contents.themes),
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
            items: run_items(contents.runs),
        },
        Menu {
            title: "Plugins".to_string(),
            english: "Plugins",
            items: Vec::new(),
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
    ]
}

/// The Character sets submenu: Notepad++'s scripts, each with the sets that
/// belong to it.
///
/// Grouped by script rather than listed flat because forty-one rows in one
/// column is a list you read rather than scan, and the script is how a user
/// knows which one they want. Chinese, Japanese and Korean are absent: their
/// sets are multi-byte, and `codepage` explains why those are not here.
///
/// Public because it is the answer to "which sets does this menu offer", and a
/// caller reading the menu rather than drawing it -- the accessibility tree, or a
/// test counting rows -- should be able to ask the same question of the table
/// that fills it instead of counting by eye.
pub const CHARACTER_SETS: [(&str, &[CodePage]); 13] = [
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
pub fn encoding_items() -> Vec<Item> {
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
pub fn view_items() -> Vec<Item> {
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
pub fn search_items() -> Vec<Item> {
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
pub fn tools_items() -> Vec<Item> {
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
pub fn macro_items() -> Vec<Item> {
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
pub fn window_items() -> Vec<Item> {
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
pub fn help_items() -> Vec<Item> {
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
pub fn run_items(saved: &[String]) -> Vec<Item> {
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
pub fn edit_items() -> Vec<Item> {
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
pub fn line_operation_items() -> Vec<Item> {
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
pub fn multi_select_items(make: fn(MatchMode) -> Action) -> Vec<Item> {
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

pub fn file_items(recent: &[String]) -> Vec<Item> {
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
///
/// `themes` is the list of themes in the order the preference row steps through
/// them, which is the front-end's to know: the fifty names, their order and their
/// labels belong to the settings, and a menu builder that assumed them would be
/// the second copy of that list. `Action::Theme(n)` carries the position in this
/// slice, so the two agree by construction.
pub fn settings_items(themes: &[&str]) -> Vec<Item> {
    vec![
        Item::new("Preferences...", Action::Preferences),
        Item::new("Style Configurator...", Action::StyleConfigurator),
        Item::submenu(
            "Theme",
            themes
                .iter()
                .enumerate()
                .map(|(index, label)| Item::new(*label, Action::Theme(index)))
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
pub fn language_items(names: &[String]) -> Vec<Item> {
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
pub fn same_rows(a: &[Item], b: &[Item]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            a.english == b.english
                && a.action == b.action
                && a.separator == b.separator
                && same_rows(&a.children, &b.children)
        })
}

/// The public view of `items`, and of everything under them.
pub fn row_items(items: &[Item]) -> Vec<RowItem> {
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
