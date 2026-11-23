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

//! The plugins installed on this machine, for the tools that run one.
//!
//! The server reads the same directory the editor reads -- `plugins` under the
//! configuration directory -- and honours the same `plugin <name>` lines in
//! `settings.txt`, so a plugin switched off in the manager is switched off
//! here. The protocol must not be a way around a switch the user has already
//! set, and a plugin that is on in the editor is on in both places.
//!
//! Nothing is loaded at startup. A plugin that is installed but switched off is
//! a row in `list_plugins` and nothing more, which is the state the editor
//! leaves it in and the state it stays in here. The folder is read again when a
//! call needs something it does not have, because a server outlives the editor
//! that was running when it started: a plugin switched on five minutes ago
//! should be usable by the next call rather than by the next process.
//!
//! What runs is the same code the editor runs -- `nn_core::plugin` -- over the
//! same bytes. A command that minifies a document in the editor minifies a
//! string here, and a plugin that traps leaves the string it was given
//! untouched, because the bridge it works on is a copy.

use std::path::{Path, PathBuf};

use nn_core::paths;
use nn_core::plugin::{self, Context, Inventory, Outcome, Plugins};

/// The plugins this installation has, and the ones switched on.
pub struct Installed {
    /// The folder they were read from.
    ///
    /// `None` when the configuration directory cannot be found, which is not an
    /// error: a server on a machine that has never run the editor answers about
    /// the reference and says it has no plugins.
    dir: Option<PathBuf>,
    /// The names `settings.txt` switches on, kept so the folder can be read
    /// again without going back to the file.
    enabled: Vec<String>,
    plugins: Plugins,
}

impl Default for Installed {
    fn default() -> Self {
        Self::empty()
    }
}

impl Installed {
    /// No folder and no plugins: what the library hands a caller that does not
    /// ask for any.
    pub fn empty() -> Installed {
        Installed {
            dir: None,
            enabled: Vec::new(),
            plugins: Plugins::default(),
        }
    }

    /// The plugins of the user running this server.
    pub fn load() -> Installed {
        let Some(dir) = paths::config_file("plugins") else {
            return Installed::empty();
        };
        let enabled: Vec<String> = paths::config_file("settings.txt")
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| {
                text.lines()
                    .filter_map(plugin::enabled_line)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        Installed::load_in(&dir, &enabled)
    }

    /// The plugins in `dir`, with the ones named in `enabled` switched on.
    ///
    /// Names rather than indices, because that is what the settings file holds
    /// and what survives a plugin being added to the folder.
    pub fn load_in(dir: &Path, enabled: &[String]) -> Installed {
        let mut plugins = Plugins::discover(dir);
        plugins.enable_saved(enabled);
        Installed {
            dir: Some(dir.to_path_buf()),
            enabled: enabled.to_vec(),
            plugins,
        }
    }

    /// Reads the folder again, keeping the same set switched on.
    ///
    /// Cheap enough to do on a tool call and not on a frame: a scan
    /// instantiates each module once to read its name and its commands, then
    /// drops it. Nothing else in this server is on a frame budget.
    pub fn reload(&mut self) {
        if let Some(dir) = self.dir.clone() {
            *self = Installed::load_in(&dir, &self.enabled);
        }
    }

    /// The folder the plugins came from, if there is one.
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// How many plugins are installed, running or not.
    pub fn installed(&self) -> usize {
        self.plugins.available()
    }

    /// How many are switched on.
    pub fn enabled(&self) -> usize {
        self.plugins.len()
    }

    /// One row per installed plugin, in folder order.
    pub fn rows(&self) -> Vec<Inventory> {
        self.plugins.inventory()
    }

    /// The files that could not be read, with the reason.
    pub fn failures(&self) -> Vec<String> {
        self.plugins
            .failures()
            .iter()
            .map(|(path, error)| {
                format!(
                    "`{}`: {error}",
                    path.file_name().and_then(|name| name.to_str()).unwrap_or("?")
                )
            })
            .collect()
    }

    /// Runs one command of one plugin over `text`.
    ///
    /// Offsets are bytes, as everywhere on the plugin side of the boundary. A
    /// selection with its ends the wrong way round is put in order first: the
    /// ABI reads them as a range, and a range that starts after it ends is not
    /// what any caller meant.
    pub fn run(
        &mut self,
        name: &str,
        command: &str,
        text: &str,
        caret: usize,
        selection: (usize, usize),
    ) -> Result<Outcome, String> {
        let (plugin, command) = match self.locate(name, command) {
            Ok(pair) => pair,
            Err(error) => {
                // The folder is the user's, and it changes under a server that
                // has been running since before the plugin was switched on:
                // read it once more before refusing.
                self.reload();
                match self.locate(name, command) {
                    Ok(pair) => pair,
                    Err(_) => return Err(error),
                }
            }
        };
        let context = Context {
            text,
            caret,
            selection: (selection.0.min(selection.1), selection.0.max(selection.1)),
            read_only: false,
        };
        self.plugins
            .run(plugin, command, &context)
            .map_err(|error| error.to_string())
    }

    /// Where a plugin and one of its commands sit in the folder.
    ///
    /// Names are matched without case, because a model that has been told the
    /// plugin is called `JSON` will sometimes send `json`, and refusing that
    /// teaches it nothing. The command may also carry the `Plugin - ` prefix the
    /// editor's menu used to label its rows with: a caller that copies one of
    /// those labels should not be told the command does not exist.
    fn locate(&self, name: &str, command: &str) -> Result<(usize, usize), String> {
        let rows = self.rows();
        let Some(index) = rows
            .iter()
            .position(|row| row.name.eq_ignore_ascii_case(name.trim()))
        else {
            let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
            return Err(format!(
                "no plugin is called \"{name}\"; installed: {}",
                names.join(", ")
            ));
        };
        let row = &rows[index];
        if !row.enabled {
            return Err(format!(
                "plugin \"{}\" is switched off; switch it on in Plugins > Manage Plugins...",
                row.name
            ));
        }
        let command = match command.split_once(" - ") {
            Some((prefix, rest)) if prefix.eq_ignore_ascii_case(&row.name) => rest,
            _ => command,
        };
        let Some(at) = row
            .commands
            .iter()
            .position(|candidate| candidate.eq_ignore_ascii_case(command.trim()))
        else {
            return Err(format!(
                "plugin \"{}\" has no command called \"{command}\"; it has: {}",
                row.name,
                row.commands.join(", ")
            ));
        };
        Ok((index, at))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A folder holding one copy of the fixture plugin, named `npp_test.wasm`.
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("npp-mcp-plugins-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("npp_test.wasm"), plugin::fixture::module()).unwrap();
        dir
    }

    #[test]
    fn a_directory_of_plugins_is_listed_with_its_commands() {
        let dir = folder("row");
        let installed = Installed::load_in(&dir, &[]);
        assert_eq!(installed.installed(), 1);
        assert_eq!(installed.enabled(), 0);
        let rows = installed.rows();
        assert_eq!(rows[0].name, "Test");
        assert_eq!(rows[0].file, "npp_test.wasm");
        assert!(!rows[0].enabled);
        assert_eq!(rows[0].commands, vec!["Uppercase", "Log"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_directory_has_no_plugins_and_no_rows() {
        let installed = Installed::load_in(Path::new("/nonexistent/plugins"), &[]);
        assert_eq!(installed.installed(), 0);
        assert!(installed.rows().is_empty());
        assert!(installed.failures().is_empty());
    }

    #[test]
    fn a_name_in_the_settings_switches_a_plugin_on() {
        let dir = folder("enable");
        let installed = Installed::load_in(&dir, &["Test".to_string()]);
        assert_eq!(installed.enabled(), 1);
        assert!(installed.rows()[0].enabled);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_command_runs_over_the_text_it_is_given() {
        let dir = folder("run");
        let mut installed = Installed::load_in(&dir, &["Test".to_string()]);
        let outcome = installed.run("Test", "Uppercase", "original", 0, (0, 0)).unwrap();
        assert_eq!(outcome.text.as_deref(), Some("REPLACED"));
        // The command that only logs leaves the text alone and says so.
        let outcome = installed.run("Test", "Log", "original", 0, (0, 0)).unwrap();
        assert_eq!(outcome.text, None);
        assert_eq!(outcome.log, vec!["REPLACED"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_is_matched_without_case_and_with_the_menu_prefix() {
        let dir = folder("naming");
        let mut installed = Installed::load_in(&dir, &["Test".to_string()]);
        for (plugin, command) in [
            ("Test", "Uppercase"),
            ("test", "uppercase"),
            ("TEST", "Test - Uppercase"),
        ] {
            let outcome = installed.run(plugin, command, "original", 0, (0, 0)).unwrap();
            assert_eq!(outcome.text.as_deref(), Some("REPLACED"), "{plugin} {command}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_switched_off_plugin_refuses_to_run_and_says_where_the_switch_is() {
        let dir = folder("off");
        let mut installed = Installed::load_in(&dir, &[]);
        let error = installed.run("Test", "Uppercase", "original", 0, (0, 0)).unwrap_err();
        assert!(error.contains("switched off"), "{error}");
        assert!(error.contains("Manage Plugins"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_plugin_is_named_and_the_installed_ones_are_listed() {
        let dir = folder("unknown");
        let mut installed = Installed::load_in(&dir, &["Test".to_string()]);
        let error = installed.run("Missing", "Uppercase", "", 0, (0, 0)).unwrap_err();
        assert!(error.contains("no plugin is called \"Missing\""), "{error}");
        assert!(error.contains("installed: Test"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_command_lists_the_ones_the_plugin_has() {
        let dir = folder("no-command");
        let mut installed = Installed::load_in(&dir, &["Test".to_string()]);
        let error = installed.run("Test", "Downcase", "", 0, (0, 0)).unwrap_err();
        assert!(error.contains("has no command called \"Downcase\""), "{error}");
        assert!(error.contains("Uppercase, Log"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_plugin_switched_on_after_the_server_started_is_usable() {
        let dir = folder("late");
        let mut installed = Installed::load_in(&dir, &[]);
        assert!(installed.run("Test", "Uppercase", "x", 0, (0, 0)).is_err());
        // The same folder, now named in the settings the reload reads from.
        installed.enabled = vec!["Test".to_string()];
        let outcome = installed.run("Test", "Uppercase", "original", 0, (0, 0)).unwrap();
        assert_eq!(outcome.text.as_deref(), Some("REPLACED"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_selection_reaches_the_plugin_lower_end_first() {
        let dir = folder("selection");
        let mut installed = Installed::load_in(&dir, &["Test".to_string()]);
        // The fixture ignores the selection, so what is asserted is that a
        // backwards range is accepted rather than refused: the bridge snaps
        // both ends to characters and the command still runs.
        let outcome = installed.run("Test", "Log", "abcdef", 3, (5, 1)).unwrap();
        assert_eq!(outcome.log, vec!["REPLACED"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_host_has_no_directory_and_no_rows() {
        let mut installed = Installed::empty();
        assert!(installed.dir().is_none());
        assert_eq!(installed.installed(), 0);
        assert!(installed.run("Test", "Uppercase", "x", 0, (0, 0)).is_err());
    }
}
