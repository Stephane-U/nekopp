//! The command inventory: what the keyboard does and where the rows live.
//!
//! Two lists in the reference have to agree, and this module is what makes them
//! agree: the table under "Keyboard Shortcuts" (chord, command, action) and the
//! menu tree under "The Menus" (the literal rows a user sees). Joining them gives
//! one answer to "what is this command, where is it, and what key runs it", which
//! is the question `list_shortcuts` and `get_command` exist for.
//!
//! Both are parsed from the markdown rather than kept as a second list in Rust:
//! a command table maintained twice is a command table that is wrong once.

use crate::doc::Document;

/// One row of the default keymap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    /// The chord as the reference spells it, e.g. `Ctrl+Shift+Q`.
    pub keys: String,
    /// The menu label the chord runs, e.g. `Exit`.
    pub command: String,
    /// The action name the keymap file stores, e.g. `exit`.
    pub action: String,
}

/// One literal row of the menu tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuRow {
    /// Menu, submenus, and the row itself, outermost first.
    pub path: Vec<String>,
    /// The row's own label, without its shortcut.
    pub label: String,
    /// The chord shown in the menu's shortcut column, if the row has one.
    pub shortcut: Option<String>,
}

impl MenuRow {
    /// The row as `File > Close Multiple Documents > Close All Unchanged`.
    pub fn path_text(&self) -> String {
        self.path.join(" > ")
    }
}

/// A command, as both lists together describe it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// The menu label, or the command name from the keymap table.
    pub label: String,
    /// The default chord, if any.
    pub shortcut: Option<String>,
    /// The action name in the keymap, if the command is bound.
    pub action: Option<String>,
    /// Where the row sits in the menu tree, if it has a row.
    pub path: Option<String>,
}

/// Everything the reference says about commands.
#[derive(Debug, Clone)]
pub struct Inventory {
    shortcuts: Vec<Shortcut>,
    rows: Vec<MenuRow>,
    commands: Vec<Command>,
}

impl Inventory {
    /// Read the keymap table and the menu tree out of the reference.
    pub fn from_document(doc: &Document) -> Self {
        let shortcuts = parse_shortcut_table(doc);
        let rows = parse_menu_tree(doc);
        let commands = join(&shortcuts, &rows);
        Self {
            shortcuts,
            rows,
            commands,
        }
    }

    /// The default keymap, in menu order.
    pub fn shortcuts(&self) -> &[Shortcut] {
        &self.shortcuts
    }

    /// Every menu row, in menu order.
    pub fn rows(&self) -> &[MenuRow] {
        &self.rows
    }

    /// Every command, bound and unbound.
    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    /// Find one command by label, action name, or enough of its label.
    ///
    /// The three spellings are all in front of an agent at different times: the
    /// menu shows "Print Now", the keymap file stores `print_now`, and a user
    /// asks about "Zoom In" without the "(Ctrl+Mouse Wheel Up)" the reference
    /// appends to the row that has two ways to be reached.
    pub fn lookup(&self, name: &str) -> Option<&Command> {
        let wanted = normalise(name);
        if wanted.is_empty() {
            return None;
        }
        let matches = |command: &Command| {
            normalise(&command.label) == wanted
                || command.action.as_deref() == Some(wanted.as_str())
                || normalise(&without_parenthetical(&command.label)) == wanted
        };
        self.commands
            .iter()
            .find(|command| matches(command))
            .or_else(|| {
                self.commands
                    .iter()
                    .find(|command| normalise(&command.label).contains(&wanted))
            })
    }
}

/// Read the `| Shortcut | Command | Action name |` table.
fn parse_shortcut_table(doc: &Document) -> Vec<Shortcut> {
    let mut shortcuts = Vec::new();
    for section in &doc.sections {
        let mut lines = section.body.lines();
        let mut header = false;
        for line in lines.by_ref() {
            let cells = table_cells(line);
            if cells.len() == 3 && cells[0] == "Shortcut" && cells[1] == "Command" {
                header = true;
                break;
            }
        }
        if !header {
            continue;
        }
        for line in lines {
            let cells = table_cells(line);
            if cells.len() != 3 {
                break;
            }
            // The rule under the header is drawn with dashes, not a binding.
            if cells
                .iter()
                .all(|cell| cell.chars().all(|ch| ch == '-' || ch == ':'))
            {
                continue;
            }
            shortcuts.push(Shortcut {
                keys: cells[0].to_string(),
                command: cells[1].to_string(),
                action: cells[2].to_string(),
            });
        }
        break;
    }
    shortcuts
}

/// Read every bullet under every menu subsection of "The Menus".
fn parse_menu_tree(doc: &Document) -> Vec<MenuRow> {
    let mut rows = Vec::new();
    for menu in doc.children("the-menus") {
        rows.extend(menu_rows(&menu.heading, &menu.body));
    }
    rows
}

/// Parse one menu section's nested bullet list into rows with full paths.
fn menu_rows(menu: &str, body: &str) -> Vec<MenuRow> {
    let mut rows = Vec::new();
    // (indent level, label) of the items above the current line, outermost
    // first; it is a stack because the markdown nests by indentation and a row
    // belongs to the last item shallower than itself.
    let mut open: Vec<(usize, String)> = Vec::new();
    for line in body.lines() {
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim_start();
        let Some(text) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) else {
            continue;
        };
        let (label, shortcut) = split_shortcut(text);
        let level = indent / 2;
        while open.last().is_some_and(|(above, _)| *above >= level) {
            open.pop();
        }
        let mut path = vec![menu.to_string()];
        path.extend(open.iter().map(|(_, label)| label.clone()));
        path.push(label.clone());
        open.push((level, label.clone()));
        rows.push(MenuRow {
            path,
            label,
            shortcut,
        });
    }
    rows
}

/// Split a menu row into its label and the chord in its trailing parentheses.
fn split_shortcut(text: &str) -> (String, Option<String>) {
    let text = text.trim();
    if let Some(start) = text.strip_suffix("`)").and_then(|rest| rest.rfind(" (`")) {
        let keys = text[start + 3..text.len() - 2].trim();
        if !keys.is_empty() {
            return (text[..start].trim().to_string(), Some(keys.to_string()));
        }
    }
    (text.to_string(), None)
}

/// One command per row, with the keymap's action joined on where there is one.
fn join(shortcuts: &[Shortcut], rows: &[MenuRow]) -> Vec<Command> {
    let mut commands = Vec::new();
    let mut claimed = vec![false; rows.len()];

    for shortcut in shortcuts {
        let row = rows.iter().position(|row| {
            normalise(&row.label) == normalise(&shortcut.command)
                || normalise(&without_parenthetical(&row.label)) == normalise(&shortcut.command)
        });
        if let Some(index) = row {
            claimed[index] = true;
        }
        commands.push(Command {
            label: shortcut.command.clone(),
            shortcut: Some(shortcut.keys.clone()),
            action: Some(shortcut.action.clone()),
            path: row.map(|index| rows[index].path_text()),
        });
    }

    // Rows with no default chord are still commands: "Close All Unchanged" can
    // be bound from the Shortcut Mapper even though nothing is bound to it yet.
    for (index, row) in rows.iter().enumerate() {
        if claimed[index] {
            continue;
        }
        commands.push(Command {
            label: row.label.clone(),
            shortcut: row.shortcut.clone(),
            action: None,
            path: Some(row.path_text()),
        });
    }

    commands
}

/// A `| a | b | c |` row as cells, with the emphasis marks taken off.
fn table_cells(line: &str) -> Vec<&str> {
    let line = line.trim();
    if !line.starts_with('|') {
        return Vec::new();
    }
    let line = line.strip_suffix('|').unwrap_or(line);
    line.trim_start_matches('|')
        .split('|')
        .map(|cell| cell.trim().trim_matches('`'))
        .collect()
}

/// Lowercase with single spaces, for comparing labels with what a client typed.
fn normalise(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// A label without the trailing "(...)" the reference appends to some rows.
fn without_parenthetical(label: &str) -> String {
    let label = label.trim();
    if let Some(open) = label.rfind(" (") {
        if label.ends_with(')') {
            return label[..open].trim().to_string();
        }
    }
    label.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inventory() -> Inventory {
        Inventory::from_document(&Document::embedded())
    }

    #[test]
    fn the_default_keymap_has_seventy_three_chords() {
        let inventory = inventory();
        assert_eq!(inventory.shortcuts().len(), 73);
        let mut seen = std::collections::HashSet::new();
        for shortcut in inventory.shortcuts() {
            assert!(seen.insert(shortcut.keys.clone()), "{}", shortcut.keys);
            assert!(!shortcut.command.is_empty());
            assert!(shortcut.action.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'));
        }
    }

    #[test]
    fn a_chord_keeps_the_command_and_action_it_was_written_with() {
        let inventory = inventory();
        let exit = inventory
            .shortcuts()
            .iter()
            .find(|shortcut| shortcut.keys == "Ctrl+Shift+Q")
            .expect("Exit is bound");
        assert_eq!(exit.command, "Exit");
        assert_eq!(exit.action, "exit");

        let comment = inventory
            .shortcuts()
            .iter()
            .find(|shortcut| shortcut.action == "toggle_comment")
            .unwrap();
        assert_eq!(comment.keys, "Ctrl+Q");
        assert_eq!(comment.command, "Toggle Single Line Comment");
    }

    #[test]
    fn menu_rows_nest_by_indentation() {
        let inventory = inventory();
        let row = inventory
            .rows()
            .iter()
            .find(|row| row.label == "PowerShell")
            .expect("the PowerShell row");
        assert_eq!(
            row.path,
            vec!["File", "Open Containing Folder", "PowerShell"]
        );
        assert_eq!(row.path_text(), "File > Open Containing Folder > PowerShell");
        assert!(row.shortcut.is_none());
    }

    #[test]
    fn a_menu_row_keeps_its_chord_out_of_its_label() {
        let inventory = inventory();
        let print = inventory
            .rows()
            .iter()
            .find(|row| row.label == "Print Now")
            .expect("Print Now");
        assert_eq!(print.shortcut.as_deref(), Some("Ctrl+Alt+Shift+P"));
        assert_eq!(print.path_text(), "File > Print Now");

        // The one row that names a second way to reach it keeps that in the
        // label, because it is part of what the menu says.
        let zoom = inventory
            .rows()
            .iter()
            .find(|row| row.path.last().unwrap() == "Zoom In (Ctrl+Mouse Wheel Up)")
            .expect("Zoom In");
        assert_eq!(zoom.shortcut.as_deref(), Some("Ctrl++"));
    }

    #[test]
    fn every_menu_has_rows_and_the_tree_is_thirteen_menus_wide() {
        let inventory = inventory();
        let menus: std::collections::BTreeSet<_> =
            inventory.rows().iter().map(|row| row.path[0].clone()).collect();
        assert_eq!(menus.len(), 13);
        assert!(menus.contains("File"));
        assert!(menus.contains("Help"));
        // Nested rows exist, otherwise the stack in `menu_rows` is untested.
        assert!(inventory
            .rows()
            .iter()
            .any(|row| row.path.len() >= 3));
    }

    #[test]
    fn a_command_is_found_by_label_action_or_shortened_label() {
        let inventory = inventory();
        let by_label = inventory.lookup("Print Now").expect("Print Now");
        assert_eq!(by_label.action.as_deref(), Some("print_now"));
        assert_eq!(by_label.shortcut.as_deref(), Some("Ctrl+Alt+Shift+P"));
        assert_eq!(by_label.path.as_deref(), Some("File > Print Now"));

        assert_eq!(inventory.lookup("print_now"), Some(by_label));
        assert_eq!(inventory.lookup("  print   NOW "), Some(by_label));

        let zoom = inventory.lookup("Zoom In").expect("Zoom In");
        assert_eq!(zoom.shortcut.as_deref(), Some("Ctrl++"));
    }

    #[test]
    fn an_unbound_row_is_still_a_command() {
        let inventory = inventory();
        let command = inventory
            .lookup("Close All Unchanged")
            .expect("Close All Unchanged");
        assert!(command.shortcut.is_none());
        assert!(command.action.is_none());
        assert_eq!(
            command.path.as_deref(),
            Some("File > Close Multiple Documents > Close All Unchanged")
        );
    }

    #[test]
    fn an_unknown_command_is_none() {
        let inventory = inventory();
        assert!(inventory.lookup("Format Everything").is_none());
        assert!(inventory.lookup("").is_none());
    }

    #[test]
    fn every_bound_chord_has_a_command_in_the_inventory() {
        // The two lists are joined at parse time; a chord that lost its command
        // would make `get_command` disagree with `list_shortcuts`.
        let inventory = inventory();
        for shortcut in inventory.shortcuts() {
            assert!(
                inventory.lookup(&shortcut.command).is_some(),
                "{}",
                shortcut.command
            );
        }
    }

    #[test]
    fn a_shortcut_line_without_a_chord_is_just_a_label() {
        assert_eq!(
            split_shortcut("One row per plugin command, labelled `Plugin - Command`").0,
            "One row per plugin command, labelled `Plugin - Command`"
        );
        assert_eq!(
            split_shortcut("New (`Ctrl+N`)"),
            ("New".to_string(), Some("Ctrl+N".to_string()))
        );
        assert_eq!(
            split_shortcut("Zoom In (Ctrl+Mouse Wheel Up) (`Ctrl++`)"),
            (
                "Zoom In (Ctrl+Mouse Wheel Up)".to_string(),
                Some("Ctrl++".to_string())
            )
        );
    }

    #[test]
    fn a_table_row_is_split_on_pipes_and_unquoted() {
        assert_eq!(
            table_cells("| `Ctrl++` | Zoom In | `zoom_in` |"),
            vec!["Ctrl++", "Zoom In", "zoom_in"]
        );
        assert!(table_cells("- not a table").is_empty());
        assert!(table_cells("|---|---|---|").len() == 3);
    }
}
