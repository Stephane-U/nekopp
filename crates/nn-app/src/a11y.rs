//! The accessibility tree.
//!
//! A screen reader on Linux talks AT-SPI over D-Bus. `accesskit_unix` speaks
//! that protocol; this module's job is to say *what* is on screen, and
//! `accesskit_winit`'s job is to deliver it.
//!
//! # Why the tree is built from a snapshot
//!
//! The obvious implementation walks the live widgets and emits nodes as it
//! goes. It cannot be tested: every widget needs a font system, a theme and a
//! window, and the tree is the one thing a screen reader user cannot check by
//! looking. So the widgets are flattened into a [`Snapshot`] -- plain strings
//! and indices -- and [`tree`] is a pure function of that. The whole shape of
//! the tree, the node ids and the action mapping are then testable without a
//! display.
//!
//! # What is exposed
//!
//! Four things, because they are the four a screen reader user needs to use an
//! editor:
//!
//! - the window title, which names the document;
//! - the menu bar, with every row and its shortcut, each runnable;
//! - the tab strip, with the active tab marked;
//! - the text, as a multiline text input with the caret's line selected;
//! - the status bar, which carries the line and column.
//!
//! # The caret is line-granular, and that is the protocol's fault
//!
//! AccessKit expresses a text position as `(text run, character index)`, and the
//! character index selects an entry in `character_lengths`, which is one entry
//! per *line*. There is no way to say "line 4, column 12" in the tree. So the
//! selection marks the caret's line, and the column is carried by the status
//! bar's label, which is where a screen reader reads it from anyway.
//!
//! # Large documents
//!
//! The text is sent over D-Bus on every update, so a 100 MB file would be a
//! 100 MB message. [`MAX_TEXT`] caps what is exposed; past it the value is cut
//! and a line saying so is appended, so a screen reader reports a truncated
//! document rather than a wrong one.

use accesskit::{
    Action, Node, NodeId, Role, TextDirection, TextPosition, TextSelection, TreeInfo, TreeUpdate,
};

use crate::menu::Row;

/// The root of the tree.
pub const ROOT: NodeId = NodeId(0);
/// The menu bar.
pub const MENU_BAR: NodeId = NodeId(1);
/// The tab strip.
pub const TAB_LIST: NodeId = NodeId(2);
/// The editor's text input.
pub const TEXT: NodeId = NodeId(3);
/// The single text run inside [`TEXT`].
pub const TEXT_RUN: NodeId = NodeId(4);
/// The status bar.
pub const STATUS: NodeId = NodeId(5);
/// The playground's one line, when the room is on screen.
pub const PLAYGROUND: NodeId = NodeId(6);

/// The first top-level menu's id; menu `i` is `MENU_BASE + i`.
pub const MENU_BASE: u64 = 10;
/// The first dropdown row's id; menu `i` row `j` is `ITEM_BASE + i * 100 + j`.
///
/// A hundred rows per menu is far more than any menu has, and the stride keeps
/// the id a function of the position rather than of a counter that would have to
/// be kept in step with the menu.
pub const ITEM_BASE: u64 = 1_000;
/// The first tab's id; tab `i` is `TAB_BASE + i`.
pub const TAB_BASE: u64 = 100_000;

/// How much of the document is exposed, in characters.
///
/// 64 KiB is about a thousand lines of prose: enough that a screen reader can
/// read a whole source file, small enough that the D-Bus message stays under a
/// millisecond on a local socket.
pub const MAX_TEXT: usize = 64 * 1024;

/// One tab, as the tree sees it.
pub struct TabRow {
    /// The tab's title.
    pub name: String,
    /// Whether this is the tab the editor is showing.
    pub selected: bool,
}

/// Everything the tree needs, flattened out of the widgets.
pub struct Snapshot {
    /// The window title, which is the document name and the application name.
    pub title: String,
    /// The menu bar's rows.
    pub menus: Vec<Row>,
    /// The open documents.
    pub tabs: Vec<TabRow>,
    /// The active document's text.
    pub text: String,
    /// The caret's line, zero-based.
    pub caret_line: usize,
    /// The status bar's text.
    pub status: String,
    /// The playground's description, when the room is on screen.
    ///
    /// The mode hides the document, so a screen reader has to be told that the
    /// window is no longer an editor and how to get it back. There is one line to
    /// read and one key to press, which is why this is a string rather than a
    /// tree of its own.
    pub playground: Option<String>,
}

/// What activating a node does.
///
/// A value rather than a closure because the caller has to run it against the
/// application state, which the tree builder does not have. It is also what
/// makes the mapping testable: the test asserts the command, not the effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Open the dropdown at this index.
    OpenMenu(usize),
    /// Run the row at `(menu, item)`.
    RunItem(usize, usize),
    /// Switch to the tab at this index.
    SelectTab(usize),
    /// Put the caret in the text.
    FocusText,
}

/// The id of the menu at `index`.
pub fn menu_id(index: usize) -> NodeId {
    NodeId(MENU_BASE + index as u64)
}

/// The id of row `item` of the menu at `index`.
pub fn item_id(index: usize, item: usize) -> NodeId {
    NodeId(ITEM_BASE + index as u64 * 100 + item as u64)
}

/// The id of the tab at `index`.
pub fn tab_id(index: usize) -> NodeId {
    NodeId(TAB_BASE + index as u64)
}

/// The lines of `text`, each with its terminator, as `character_lengths` wants
/// them.
///
/// A trailing newline does not start a new line: `"a\n"` is one line, not two,
/// because the caret after it is at the end of line one rather than at the start
/// of an empty line two. That is the same rule the editor's own line count uses,
/// and the two disagreeing is how a screen reader ends up one line off.
fn line_lengths(text: &str) -> Vec<u8> {
    let mut lengths: Vec<u8> = Vec::new();
    let mut current: usize = 0;
    for ch in text.chars() {
        current += 1;
        if ch == '\n' {
            // A line longer than 255 characters cannot be expressed: the field
            // is a `u8`. Saturating keeps the count of lines right, which is
            // what the caret index depends on; the length is only used for
            // word wrapping by the consumer.
            lengths.push(current.min(255) as u8);
            current = 0;
        }
    }
    if current > 0 {
        lengths.push(current.min(255) as u8);
    }
    lengths
}

/// The text as it is exposed, cut at [`MAX_TEXT`] with a note when it was.
fn exposed_text(text: &str) -> String {
    if text.len() <= MAX_TEXT {
        return text.to_string();
    }
    // Cut on a character boundary, not a byte one: slicing a `str` mid-character
    // panics, and a document with an accent near the limit would take the
    // accessibility tree down with it.
    let mut cut = MAX_TEXT;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut out = text[..cut].to_string();
    out.push_str("\n[document truncated for the screen reader]\n");
    out
}

/// Builds the tree.
///
/// Every node is emitted on every call rather than diffed against the previous
/// tree. AccessKit accepts a full tree as an update, and a diff would have to be
/// correct about which nodes were removed -- a bug there leaves a screen reader
/// reading a document that is no longer open, which is worse than sending a few
/// hundred nodes.
pub fn tree(snapshot: &Snapshot) -> TreeUpdate {
    let text = exposed_text(&snapshot.text);
    let lengths = line_lengths(&text);
    let caret = snapshot.caret_line.min(lengths.len().saturating_sub(1));

    let mut nodes: Vec<(NodeId, Node)> = Vec::new();

    // The root. Its children are the bars and the text, in the order they are
    // drawn, so a screen reader walking the tree walks the window top to bottom.
    let mut root = Node::new(Role::Window);
    root.set_label(snapshot.title.clone());
    let mut root_children = vec![MENU_BAR, TAB_LIST, TEXT];
    if !snapshot.status.is_empty() {
        root_children.push(STATUS);
    }
    if snapshot.playground.is_some() {
        root_children.push(PLAYGROUND);
    }
    root.set_children(root_children);
    nodes.push((ROOT, root));

    // The playground, when it is what the window is showing: one label, read
    // after the bars it replaced, saying what took the editor's place and what
    // gets it back.
    if let Some(description) = &snapshot.playground {
        let mut node = Node::new(Role::Label);
        node.set_label(description.clone());
        nodes.push((PLAYGROUND, node));
    }

    // The menu bar. Each top-level title is a `Menu` whose children are its
    // rows; a separator is skipped because it is not a thing to read out.
    let mut bar = Node::new(Role::MenuBar);
    bar.set_children((0..snapshot.menus.len()).map(menu_id).collect::<Vec<_>>());
    nodes.push((MENU_BAR, bar));
    for (index, menu) in snapshot.menus.iter().enumerate() {
        let mut node = Node::new(Role::Menu);
        node.set_label(menu.title.clone());
        let rows: Vec<NodeId> = menu
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| !item.separator)
            .map(|(item, _)| item_id(index, item))
            .collect();
        node.set_children(rows);
        // A menu title is clickable in the window, so it has to be clickable
        // here: without the action the node is a label, and a screen reader
        // user can read the menu but never open it.
        node.add_action(Action::Click);
        nodes.push((menu_id(index), node));
        for (item, row) in menu.items.iter().enumerate() {
            if row.separator {
                continue;
            }
            let mut node = Node::new(Role::MenuItem);
            node.set_label(row.label.clone());
            if !row.shortcut.is_empty() {
                node.set_keyboard_shortcut(row.shortcut.clone());
            }
            node.add_action(Action::Click);
            nodes.push((item_id(index, item), node));
        }
    }

    // The tab strip. A single tab is still a list, because the role is what
    // tells a screen reader that the window has documents rather than one.
    let mut list = Node::new(Role::TabList);
    list.set_children((0..snapshot.tabs.len()).map(tab_id).collect::<Vec<_>>());
    nodes.push((TAB_LIST, list));
    for (index, tab) in snapshot.tabs.iter().enumerate() {
        let mut node = Node::new(Role::Tab);
        node.set_label(tab.name.clone());
        if tab.selected {
            node.set_selected(true);
        }
        node.add_action(Action::Click);
        nodes.push((tab_id(index), node));
    }

    // The text. The input carries the selection and the run carries the value,
    // which is the shape AccessKit's own tests use: a position names a run, and
    // a run is where the characters live.
    let mut input = Node::new(Role::MultilineTextInput);
    input.set_label(snapshot.title.clone());
    input.set_children(vec![TEXT_RUN]);
    input.set_text_selection(TextSelection {
        anchor: TextPosition {
            node: TEXT_RUN,
            character_index: caret,
        },
        focus: TextPosition {
            node: TEXT_RUN,
            character_index: caret,
        },
    });
    input.add_action(Action::Focus);
    nodes.push((TEXT, input));

    let mut run = Node::new(Role::TextRun);
    run.set_value(text);
    run.set_character_lengths(lengths);
    run.set_text_direction(TextDirection::LeftToRight);
    nodes.push((TEXT_RUN, run));

    if !snapshot.status.is_empty() {
        let mut status = Node::new(Role::Status);
        status.set_label(snapshot.status.clone());
        nodes.push((STATUS, status));
    }

    TreeUpdate {
        nodes,
        tree: Some(TreeInfo::new(ROOT)),
        tree_id: accesskit::TreeId::ROOT,
        focus: TEXT,
    }
}

/// What activating `id` does, if anything.
///
/// A node with no command is one that is only there to be read -- the status
/// bar, the text run, the root.
pub fn command(id: NodeId, snapshot: &Snapshot) -> Option<Command> {
    if id == TEXT {
        return Some(Command::FocusText);
    }
    if id.0 >= TAB_BASE {
        let index = (id.0 - TAB_BASE) as usize;
        return (index < snapshot.tabs.len()).then_some(Command::SelectTab(index));
    }
    if id.0 >= ITEM_BASE {
        let offset = (id.0 - ITEM_BASE) as usize;
        let (menu, item) = (offset / 100, offset % 100);
        let row = snapshot.menus.get(menu)?.items.get(item)?;
        // A separator has no action of its own; the menu stores `About` in the
        // field because it has to store something, and running it would be a
        // surprise.
        if row.separator {
            return None;
        }
        return Some(Command::RunItem(menu, item));
    }
    if id.0 >= MENU_BASE {
        let index = (id.0 - MENU_BASE) as usize;
        return (index < snapshot.menus.len()).then_some(Command::OpenMenu(index));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    // `Action` is both AccessKit's and the menu's; the menu one is aliased so
    // the two cannot be confused in an assertion.
    use crate::menu::{Action as MenuAction, RowItem};

    fn row(label: &str, action: MenuAction) -> RowItem {
        RowItem {
            label: label.to_string(),
            shortcut: String::new(),
            action,
            separator: false,
            children: Vec::new(),
        }
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            playground: None,
            title: "notes.txt - Neko++".to_string(),
            menus: vec![
                Row {
                    title: "File".to_string(),
                    items: vec![
                        row("New", MenuAction::New),
                        RowItem {
                            label: String::new(),
                            shortcut: String::new(),
                            action: MenuAction::About,
                            separator: true,
                            children: Vec::new(),
                        },
                        row("Save", MenuAction::Save),
                    ],
                },
                Row {
                    title: "Edit".to_string(),
                    items: vec![row("Undo", MenuAction::Undo)],
                },
            ],
            tabs: vec![
                TabRow {
                    name: "notes.txt".to_string(),
                    selected: true,
                },
                TabRow {
                    name: "other.txt".to_string(),
                    selected: false,
                },
            ],
            text: "alpha\nbeta\ngamma\n".to_string(),
            caret_line: 1,
            status: "Ln: 2, Col: 1".to_string(),
        }
    }

    fn node(update: &TreeUpdate, id: NodeId) -> &Node {
        &update
            .nodes
            .iter()
            .find(|(node_id, _)| *node_id == id)
            .unwrap_or_else(|| panic!("no node {id:?}"))
            .1
    }

    #[test]
    fn the_root_is_a_window_named_after_the_document() {
        let update = tree(&snapshot());
        let root = node(&update, ROOT);
        assert_eq!(root.role(), Role::Window);
        assert_eq!(root.label(), Some("notes.txt - Neko++"));
        assert_eq!(update.tree.as_ref().unwrap().root, ROOT);
    }

    #[test]
    fn the_root_lists_the_bars_and_the_text_in_drawing_order() {
        let update = tree(&snapshot());
        assert_eq!(
            node(&update, ROOT).children(),
            &[MENU_BAR, TAB_LIST, TEXT, STATUS]
        );
    }

    #[test]
    fn the_status_bar_is_left_out_when_it_is_empty() {
        let mut snap = snapshot();
        snap.status = String::new();
        let update = tree(&snap);
        assert_eq!(node(&update, ROOT).children(), &[MENU_BAR, TAB_LIST, TEXT]);
        assert!(!update.nodes.iter().any(|(id, _)| *id == STATUS));
    }

    #[test]
    fn every_menu_is_a_node_with_its_rows_as_children() {
        let update = tree(&snapshot());
        let bar = node(&update, MENU_BAR);
        assert_eq!(bar.role(), Role::MenuBar);
        assert_eq!(bar.children(), &[menu_id(0), menu_id(1)]);
        assert_eq!(node(&update, menu_id(0)).label(), Some("File"));
        assert_eq!(node(&update, menu_id(1)).label(), Some("Edit"));
    }

    #[test]
    fn a_separator_is_not_a_row() {
        // The File menu has three entries and one of them is a separator, so the
        // menu has two children and the separator's id is absent.
        let update = tree(&snapshot());
        assert_eq!(
            node(&update, menu_id(0)).children(),
            &[item_id(0, 0), item_id(0, 2)]
        );
        assert!(!update.nodes.iter().any(|(id, _)| *id == item_id(0, 1)));
    }

    #[test]
    fn a_menu_title_is_clickable_so_it_can_be_opened() {
        let update = tree(&snapshot());
        assert!(node(&update, menu_id(0)).supports_action(Action::Click));
    }

    #[test]
    fn a_row_carries_its_label_and_is_clickable() {
        let update = tree(&snapshot());
        let save = node(&update, item_id(0, 2));
        assert_eq!(save.role(), Role::MenuItem);
        assert_eq!(save.label(), Some("Save"));
        assert!(save.supports_action(Action::Click));
    }

    #[test]
    fn a_row_with_a_chord_advertises_it() {
        let mut snap = snapshot();
        snap.menus[0].items[2].shortcut = "Ctrl+S".to_string();
        let update = tree(&snap);
        assert_eq!(
            node(&update, item_id(0, 2)).keyboard_shortcut(),
            Some("Ctrl+S")
        );
    }

    #[test]
    fn a_row_without_a_chord_does_not_advertise_one() {
        let update = tree(&snapshot());
        assert_eq!(node(&update, item_id(0, 2)).keyboard_shortcut(), None);
    }

    #[test]
    fn the_tab_strip_marks_the_active_tab() {
        let update = tree(&snapshot());
        let list = node(&update, TAB_LIST);
        assert_eq!(list.role(), Role::TabList);
        assert_eq!(list.children(), &[tab_id(0), tab_id(1)]);
        assert_eq!(node(&update, tab_id(0)).is_selected(), Some(true));
        assert_eq!(node(&update, tab_id(1)).is_selected(), None);
    }

    #[test]
    fn the_text_is_a_multiline_input_with_one_run() {
        let update = tree(&snapshot());
        let input = node(&update, TEXT);
        assert_eq!(input.role(), Role::MultilineTextInput);
        assert_eq!(input.children(), &[TEXT_RUN]);
        assert_eq!(node(&update, TEXT_RUN).role(), Role::TextRun);
        assert_eq!(node(&update, TEXT_RUN).value(), Some("alpha\nbeta\ngamma\n"));
    }

    #[test]
    fn the_run_counts_one_entry_per_line() {
        // Three lines, and the trailing newline does not start a fourth.
        let update = tree(&snapshot());
        assert_eq!(node(&update, TEXT_RUN).character_lengths(), &[6, 5, 6]);
    }

    #[test]
    fn a_document_without_a_trailing_newline_still_counts_its_last_line() {
        let mut snap = snapshot();
        snap.text = "alpha\nbeta".to_string();
        let update = tree(&snap);
        assert_eq!(node(&update, TEXT_RUN).character_lengths(), &[6, 4]);
    }

    #[test]
    fn an_empty_document_has_no_lines() {
        let mut snap = snapshot();
        snap.text = String::new();
        let update = tree(&snap);
        assert_eq!(node(&update, TEXT_RUN).character_lengths(), &[] as &[u8]);
        // The caret still has to point somewhere, and index 0 of an empty slice
        // is the only valid answer.
        let selection = node(&update, TEXT).text_selection().unwrap();
        assert_eq!(selection.focus.character_index, 0);
    }

    #[test]
    fn the_caret_marks_its_line() {
        let update = tree(&snapshot());
        let selection = node(&update, TEXT).text_selection().unwrap();
        assert_eq!(selection.focus.node, TEXT_RUN);
        assert_eq!(selection.focus.character_index, 1);
        // A caret is a degenerate selection, not a missing one.
        assert_eq!(selection.anchor, selection.focus);
    }

    #[test]
    fn a_caret_past_the_last_line_is_clamped() {
        // The document can shrink between the snapshot and the tree, and an
        // index past the end is rejected by the consumer rather than clamped.
        let mut snap = snapshot();
        snap.caret_line = 99;
        let update = tree(&snap);
        let selection = node(&update, TEXT).text_selection().unwrap();
        assert_eq!(selection.focus.character_index, 2);
    }

    #[test]
    fn a_long_document_is_cut_and_says_so() {
        let mut snap = snapshot();
        snap.text = "x".repeat(MAX_TEXT + 100);
        let update = tree(&snap);
        let value = node(&update, TEXT_RUN).value().unwrap();
        assert!(value.len() < MAX_TEXT + 200, "{}", value.len());
        assert!(value.ends_with("[document truncated for the screen reader]\n"));
    }

    #[test]
    fn a_cut_lands_on_a_character_boundary() {
        // A two-byte character straddling the limit must not be sliced in half:
        // `&text[..MAX_TEXT]` would panic.
        let mut snap = snapshot();
        let mut text = "x".repeat(MAX_TEXT - 1);
        text.push('e');
        text.push('\u{00e9}');
        snap.text = text;
        let update = tree(&snap);
        let value = node(&update, TEXT_RUN).value().unwrap();
        assert!(value.contains("truncated"), "{}", value.len());
    }

    #[test]
    fn a_short_document_is_not_cut() {
        let update = tree(&snapshot());
        let value = node(&update, TEXT_RUN).value().unwrap();
        assert!(!value.contains("truncated"));
    }

    #[test]
    fn the_text_is_focused_so_a_reader_starts_in_the_document() {
        let update = tree(&snapshot());
        assert_eq!(update.focus, TEXT);
    }

    #[test]
    fn clicking_a_row_runs_it() {
        let snap = snapshot();
        assert_eq!(
            command(item_id(0, 2), &snap),
            Some(Command::RunItem(0, 2))
        );
        assert_eq!(
            command(item_id(1, 0), &snap),
            Some(Command::RunItem(1, 0))
        );
    }

    #[test]
    fn clicking_a_separator_does_nothing() {
        // The separator stores `About` because the field has to hold something.
        // Running it would open the About box from a line.
        assert_eq!(command(item_id(0, 1), &snapshot()), None);
    }

    #[test]
    fn clicking_a_menu_title_opens_it() {
        assert_eq!(command(menu_id(1), &snapshot()), Some(Command::OpenMenu(1)));
    }

    #[test]
    fn clicking_a_tab_selects_it() {
        assert_eq!(command(tab_id(1), &snapshot()), Some(Command::SelectTab(1)));
    }

    #[test]
    fn clicking_the_text_focuses_it() {
        assert_eq!(command(TEXT, &snapshot()), Some(Command::FocusText));
    }

    #[test]
    fn the_read_only_nodes_have_no_command() {
        let snap = snapshot();
        for id in [ROOT, MENU_BAR, TAB_LIST, TEXT_RUN, STATUS] {
            assert_eq!(command(id, &snap), None, "{id:?}");
        }
    }

    #[test]
    fn an_id_past_the_end_has_no_command() {
        // A stale id from a tree the user has since changed must not index out
        // of bounds; the menu can shrink between the update and the click.
        let snap = snapshot();
        assert_eq!(command(menu_id(9), &snap), None);
        assert_eq!(command(item_id(9, 0), &snap), None);
        assert_eq!(command(item_id(0, 9), &snap), None);
        assert_eq!(command(tab_id(9), &snap), None);
    }

    #[test]
    fn the_ids_of_different_things_do_not_collide() {
        // The three bases are far apart on purpose; a collision would make a
        // click on a tab run a menu row.
        let ids = [
            ROOT,
            MENU_BAR,
            TAB_LIST,
            TEXT,
            TEXT_RUN,
            STATUS,
            menu_id(0),
            menu_id(99),
            item_id(0, 0),
            item_id(99, 99),
            tab_id(0),
            tab_id(999),
        ];
        let mut seen = std::collections::HashSet::new();
        for id in ids {
            assert!(seen.insert(id.0), "{id:?} collides");
        }
    }

    #[test]
    fn every_node_in_the_tree_is_reachable_from_the_root() {
        // AccessKit rejects an update containing a node that is neither the root
        // nor a child of another node, so an orphan is a dropped update.
        let update = tree(&snapshot());
        let mut reachable = std::collections::HashSet::new();
        let mut stack = vec![ROOT];
        while let Some(id) = stack.pop() {
            if !reachable.insert(id) {
                continue;
            }
            if let Some((_, node)) = update.nodes.iter().find(|(node_id, _)| *node_id == id) {
                stack.extend(node.children().iter().copied());
            }
        }
        for (id, _) in &update.nodes {
            assert!(reachable.contains(id), "{id:?} is orphaned");
        }
    }
}
