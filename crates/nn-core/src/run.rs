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

//! The Run command.
//!
//! Notepad++'s Run dialog takes a command line, substitutes a handful of
//! `$(NAME)` macros for facts about the current document, and hands the result to
//! the shell. This is the same idea with the same macro names, because a user
//! switching over should be able to paste their existing command and have it work.
//!
//! The expansion is a pure function of the template and a context, which is the
//! whole reason it lives in this crate: a command line that silently expands to
//! the wrong path is a command that runs on the wrong file, and that is worth a
//! test rather than a click.

use std::path::{Path, PathBuf};

/// The facts a command line can refer to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunContext {
    /// The active document's path, when it has one.
    pub path: Option<PathBuf>,
    /// The caret's line, one-based.
    pub line: usize,
    /// The caret's column, one-based.
    pub column: usize,
    /// The word under the caret.
    pub word: String,
    /// The selected text, empty when there is no selection.
    pub selection: String,
    /// The directory the editor's own binary lives in.
    pub directory: PathBuf,
}

impl RunContext {
    /// The full path of the current file, or the empty string.
    fn full_path(&self) -> String {
        self.path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default()
    }

    /// The directory holding the current file.
    fn current_directory(&self) -> String {
        self.path
            .as_ref()
            .and_then(|path| path.parent())
            .map(|path| path.display().to_string())
            .unwrap_or_default()
    }

    /// The file name with its extension.
    fn file_name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// The file name without its extension.
    fn name_part(&self) -> String {
        self.path
            .as_ref()
            .and_then(|path| path.file_stem())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// The extension, without the dot.
    fn ext_part(&self) -> String {
        self.path
            .as_ref()
            .and_then(|path| path.extension())
            .map(|ext| ext.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// Every macro this understands, in the order the help text lists them.
pub const MACROS: [&str; 10] = [
    "FULL_CURRENT_PATH",
    "CURRENT_DIRECTORY",
    "FILE_NAME",
    "NAME_PART",
    "EXT_PART",
    "CURRENT_WORD",
    "CURRENT_LINE",
    "CURRENT_COLUMN",
    "SELECTION",
    "NPP_DIRECTORY",
];

/// Substitutes every `$(NAME)` macro in `template`.
///
/// An unknown macro is left exactly as written. That is deliberate: a command
/// line is something a user typed, and turning `$(CURRNET_LINE)` into an empty
/// string would run the command against nothing and report success. Leaving it
/// visible means the shell complains about it, which is a better failure.
///
/// `$(SYS.NAME)` reads an environment variable, matching Notepad++.
pub fn expand(template: &str, context: &RunContext) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("$(") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find(')') else {
            // An unclosed `$(` is literal text, not a macro.
            out.push_str(&rest[start..]);
            return out;
        };
        let name = &after[..end];
        match value(name, context) {
            Some(value) => out.push_str(&value),
            None => {
                out.push_str("$(");
                out.push_str(name);
                out.push(')');
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// The replacement for one macro name, or `None` when it is not one.
fn value(name: &str, context: &RunContext) -> Option<String> {
    let value = match name {
        "FULL_CURRENT_PATH" => context.full_path(),
        "CURRENT_DIRECTORY" => context.current_directory(),
        "FILE_NAME" => context.file_name(),
        "NAME_PART" => context.name_part(),
        "EXT_PART" => context.ext_part(),
        "CURRENT_WORD" => context.word.clone(),
        "CURRENT_LINE" => context.line.to_string(),
        "CURRENT_COLUMN" => context.column.to_string(),
        "SELECTION" => context.selection.clone(),
        "NPP_DIRECTORY" => context.directory.display().to_string(),
        _ => {
            let variable = name.strip_prefix("SYS.")?;
            std::env::var(variable).ok()?
        }
    };
    Some(value)
}

/// One saved command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCommand {
    /// The command line, with macros unexpanded.
    pub command: String,
    /// Whether to write the document before running.
    ///
    /// On by default: a command that compiles the current file is useless if the
    /// file on disk is the version from before the last edit.
    pub save_first: bool,
}

impl RunCommand {
    /// Builds a command that saves first.
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            save_first: true,
        }
    }

    /// The label shown in the Run menu.
    ///
    /// The command itself, truncated: a saved command is recognised by what it
    /// does, and a name would be a second thing to invent and keep in sync.
    pub fn label(&self) -> String {
        const LIMIT: usize = 48;
        let mut label: String = self.command.chars().take(LIMIT).collect();
        if self.command.chars().count() > LIMIT {
            label.push_str("...");
        }
        label
    }
}

/// The saved commands, in menu order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunList {
    commands: Vec<RunCommand>,
}

impl RunList {
    /// The saved commands.
    pub fn commands(&self) -> &[RunCommand] {
        &self.commands
    }

    /// Whether the list is empty.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Adds a command, replacing an identical one so the menu cannot grow
    /// duplicates of the same line.
    pub fn add(&mut self, command: RunCommand) {
        if command.command.trim().is_empty() {
            return;
        }
        self.commands.retain(|c| c.command != command.command);
        self.commands.push(command);
    }

    /// Removes the command at `index`.
    pub fn remove(&mut self, index: usize) {
        if index < self.commands.len() {
            self.commands.remove(index);
        }
    }

    /// The command at `index`.
    pub fn get(&self, index: usize) -> Option<&RunCommand> {
        self.commands.get(index)
    }

    /// Reads the `run` lines of a settings file.
    ///
    /// The format is `run <command>`, with a leading `!` on the command meaning
    /// "do not save first". A command containing spaces needs no quoting because
    /// the rest of the line is the command.
    pub fn parse_line(&mut self, value: &str) {
        let value = value.trim();
        if value.is_empty() {
            return;
        }
        let (save_first, command) = match value.strip_prefix('!') {
            Some(rest) => (false, rest.trim()),
            None => (true, value),
        };
        self.add(RunCommand {
            command: command.to_string(),
            save_first,
        });
    }

    /// Renders the `run` lines for a settings file.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for command in &self.commands {
            out.push_str("run ");
            if !command.save_first {
                out.push('!');
            }
            out.push_str(&command.command);
            out.push('\n');
        }
        out
    }
}

/// Starts `command` through the shell, in `directory` when one is given.
///
/// The shell does the splitting, because a command line is a shell command:
/// quoting, pipes, and redirection are the user's to use, and reimplementing
/// them would be a worse shell.
///
/// The child is not waited for. A command that compiles a project takes minutes,
/// and an editor that freezes until it finishes is an editor that looks hung.
/// Its output goes to the terminal the editor was started from, which is where a
/// user who typed a command expects to find it.
pub fn spawn(command: &str, directory: Option<&Path>) -> std::io::Result<std::process::Child> {
    let mut process = std::process::Command::new("sh");
    process.arg("-c").arg(command);
    if let Some(directory) = directory {
        process.current_dir(directory);
    }
    process.spawn()
}

/// Splits a command line into the program and its arguments.
///
/// Not used to run anything — the shell does that — but the Run dialog shows the
/// program it is about to start, and a user who has mistyped the path should see
/// it before the shell does.
pub fn program_of(command: &str) -> Option<&str> {
    command.split_whitespace().next()
}

/// Whether `command` names a file that exists, when it names a path at all.
///
/// A bare name like `gcc` is looked up by the shell and is not this function's
/// business; a path with a separator in it is, and a missing one is worth saying
/// so before the shell prints its own version of the same thing.
pub fn missing_program(command: &str) -> Option<String> {
    let program = program_of(command)?;
    if !program.contains('/') {
        return None;
    }
    let path = Path::new(program);
    (!path.exists()).then(|| program.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> RunContext {
        RunContext {
            path: Some(PathBuf::from("/home/user/src/main.rs")),
            line: 42,
            column: 7,
            word: "expand".to_string(),
            selection: "let x = 1;".to_string(),
            directory: PathBuf::from("/opt/npp"),
        }
    }

    #[test]
    fn every_macro_expands() {
        let text = expand(
            "$(FULL_CURRENT_PATH)|$(CURRENT_DIRECTORY)|$(FILE_NAME)|$(NAME_PART)|$(EXT_PART)|\
             $(CURRENT_WORD)|$(CURRENT_LINE)|$(CURRENT_COLUMN)|$(SELECTION)|$(NPP_DIRECTORY)",
            &context(),
        );
        assert_eq!(
            text,
            "/home/user/src/main.rs|/home/user/src|main.rs|main|rs|expand|42|7|let x = 1;|/opt/npp"
        );
    }

    #[test]
    fn a_template_without_macros_is_unchanged() {
        assert_eq!(expand("cargo build", &context()), "cargo build");
    }

    #[test]
    fn an_unknown_macro_is_left_alone() {
        assert_eq!(
            expand("echo $(CURRNET_LINE)", &context()),
            "echo $(CURRNET_LINE)"
        );
    }

    #[test]
    fn an_unclosed_macro_is_literal_text() {
        assert_eq!(expand("echo $(FILE_NAME", &context()), "echo $(FILE_NAME");
    }

    #[test]
    fn a_document_without_a_path_expands_to_empty() {
        let context = RunContext::default();
        assert_eq!(expand("$(FULL_CURRENT_PATH)", &context), "");
        assert_eq!(expand("$(FILE_NAME)", &context), "");
        assert_eq!(expand("$(EXT_PART)", &context), "");
    }

    #[test]
    fn a_file_without_an_extension_has_no_ext_part() {
        let context = RunContext {
            path: Some(PathBuf::from("/tmp/Makefile")),
            ..RunContext::default()
        };
        assert_eq!(expand("$(NAME_PART)", &context), "Makefile");
        assert_eq!(expand("$(EXT_PART)", &context), "");
    }

    #[test]
    fn a_dotted_name_splits_at_the_last_dot() {
        let context = RunContext {
            path: Some(PathBuf::from("/tmp/archive.tar.gz")),
            ..RunContext::default()
        };
        assert_eq!(expand("$(NAME_PART)", &context), "archive.tar");
        assert_eq!(expand("$(EXT_PART)", &context), "gz");
    }

    #[test]
    fn adjacent_macros_both_expand() {
        assert_eq!(expand("$(NAME_PART).$(EXT_PART)", &context()), "main.rs");
    }

    #[test]
    fn a_macro_at_the_very_end_expands() {
        assert_eq!(expand("echo $(CURRENT_LINE)", &context()), "echo 42");
    }

    #[test]
    fn an_environment_variable_is_read() {
        // `PATH` is set in every environment this runs in.
        let text = expand("$(SYS.PATH)", &RunContext::default());
        assert!(!text.is_empty());
        assert!(!text.contains("$("));
    }

    #[test]
    fn an_unset_environment_variable_is_left_alone() {
        let text = expand("$(SYS.NPP_RS_SURELY_NOT_SET)", &RunContext::default());
        assert_eq!(text, "$(SYS.NPP_RS_SURELY_NOT_SET)");
    }

    #[test]
    fn the_macro_list_matches_what_expands() {
        let context = context();
        for name in MACROS {
            let text = expand(&format!("$({name})"), &context);
            assert!(!text.contains("$("), "{name} did not expand");
        }
    }

    #[test]
    fn a_saved_command_saves_first_by_default() {
        assert!(RunCommand::new("cargo build").save_first);
    }

    #[test]
    fn a_long_command_is_truncated_in_the_menu() {
        let command = RunCommand::new("x".repeat(80));
        assert_eq!(command.label().chars().count(), 51);
        assert!(command.label().ends_with("..."));
    }

    #[test]
    fn a_short_command_is_its_own_label() {
        assert_eq!(RunCommand::new("make").label(), "make");
    }

    #[test]
    fn adding_the_same_command_twice_keeps_one() {
        let mut list = RunList::default();
        list.add(RunCommand::new("make"));
        list.add(RunCommand::new("make"));
        assert_eq!(list.commands().len(), 1);
    }

    #[test]
    fn an_empty_command_is_not_added() {
        let mut list = RunList::default();
        list.add(RunCommand::new("   "));
        assert!(list.is_empty());
    }

    #[test]
    fn removing_takes_the_right_one() {
        let mut list = RunList::default();
        list.add(RunCommand::new("one"));
        list.add(RunCommand::new("two"));
        list.remove(0);
        assert_eq!(list.commands()[0].command, "two");
    }

    #[test]
    fn removing_past_the_end_does_nothing() {
        let mut list = RunList::default();
        list.add(RunCommand::new("one"));
        list.remove(9);
        assert_eq!(list.commands().len(), 1);
    }

    #[test]
    fn the_run_lines_round_trip() {
        let mut list = RunList::default();
        list.add(RunCommand::new("cargo build --release"));
        list.add(RunCommand {
            command: "make -j8".to_string(),
            save_first: false,
        });
        let text = list.render();
        assert_eq!(text, "run cargo build --release\nrun !make -j8\n");

        let mut back = RunList::default();
        for line in text.lines() {
            back.parse_line(line.strip_prefix("run ").unwrap());
        }
        assert_eq!(back, list);
    }

    #[test]
    fn a_command_with_spaces_keeps_them() {
        let mut list = RunList::default();
        list.parse_line("gcc -o $(NAME_PART) $(FULL_CURRENT_PATH)");
        assert_eq!(
            list.commands()[0].command,
            "gcc -o $(NAME_PART) $(FULL_CURRENT_PATH)"
        );
    }

    #[test]
    fn a_command_runs_and_writes_where_it_was_told() {
        let dir = std::env::temp_dir().join("npp-run-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut child = spawn("printf hello > proof.txt", Some(&dir)).unwrap();
        child.wait().unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("proof.txt")).unwrap(),
            "hello"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failing_command_still_starts() {
        // The shell starts and reports the failure; `spawn` itself succeeds,
        // which is why the status bar says "Run:" rather than "Run failed".
        let mut child = spawn("exit 3", None).unwrap();
        assert_eq!(child.wait().unwrap().code(), Some(3));
    }

    #[test]
    fn the_program_is_the_first_word() {
        assert_eq!(program_of("cargo build --release"), Some("cargo"));
        assert_eq!(program_of("   "), None);
    }

    #[test]
    fn a_missing_absolute_program_is_reported() {
        assert_eq!(
            missing_program("/nonexistent/tool --flag"),
            Some("/nonexistent/tool".to_string())
        );
    }

    #[test]
    fn a_bare_name_is_left_to_the_shell() {
        assert_eq!(missing_program("cargo build"), None);
    }

    #[test]
    fn an_existing_program_is_not_reported() {
        assert_eq!(missing_program("/bin/sh -c true"), None);
    }
}
