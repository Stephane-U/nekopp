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

//! What survives a restart: the recent file list and the open tabs.
//!
//! Notepad++ reopens the files you had open and remembers where you were in
//! them, and that is most of what makes an editor feel like a place you return
//! to rather than a program you launch. This module owns the file format and
//! nothing else — the application decides when to read and write it.
//!
//! The format is line-oriented and deliberately dumb: a keyword, a space, and
//! the rest of the line. Numbers come before paths so that a path containing
//! spaces needs no quoting.

use std::path::{Path, PathBuf};

use crate::tabcolour::TabColour;

/// How many recent files to remember.
pub const MAX_RECENT: usize = 10;

/// One tab to reopen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTab {
    /// The file to open.
    pub path: PathBuf,
    /// The caret's line, zero-based.
    pub line: usize,
    /// The caret's column, in characters.
    pub column: usize,
    /// Whether the tab was pinned.
    pub pinned: bool,
    /// The colour the tab was painted with.
    pub colour: TabColour,
}

/// What survives a restart.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Session {
    /// Most recent first.
    pub recent: Vec<PathBuf>,
    /// Tabs to reopen, in strip order.
    pub tabs: Vec<SessionTab>,
    /// Index of the tab that was active.
    pub active: usize,
}

impl Session {
    /// The file the session is stored in.
    ///
    /// Each platform has its own idea of where configuration belongs, and using
    /// the wrong one means the file is invisible to the tools that back it up.
    pub fn path() -> Option<PathBuf> {
        nn_core::paths::config_file("session.txt")
    }

    /// Reads the session, or an empty one when there is none.
    pub fn load() -> Session {
        Self::path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    /// Writes the session.
    ///
    /// Failures are ignored on purpose: a read-only home directory should not
    /// stop the editor from closing.
    pub fn save(&self) {
        let Some(path) = Self::path() else {
            return;
        };
        nn_core::paths::write(&path, &self.render());
    }

    /// Records `path` as the most recent file, moving it to the front.
    ///
    /// The path is made absolute first. A recent file is read back at the next
    /// start, long after the directory it was typed in is gone, so an entry
    /// stored as `test/01-object.json` would be asking for that path relative to
    /// whatever directory the editor was launched in -- and would fail to open
    /// for a reason the user could not see. The same rule makes the duplicate
    /// check work: one file reached two ways is one entry.
    pub fn push_recent(&mut self, path: &Path) {
        let path = nn_core::paths::absolute(path);
        self.recent.retain(|p| p != &path);
        self.recent.insert(0, path);
        self.recent.truncate(MAX_RECENT);
    }

    /// Parses the on-disk format.
    ///
    /// Unknown keywords and malformed lines are skipped rather than rejected:
    /// a session file written by a newer version should still open the tabs it
    /// can understand.
    pub fn parse(text: &str) -> Session {
        let mut session = Session::default();
        for line in text.lines() {
            let line = line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((keyword, rest)) = line.split_once(' ') else {
                continue;
            };
            match keyword {
                "recent" => session.recent.push(PathBuf::from(rest)),
                // `tab` is the three-field form written before tabs could be
                // pinned or painted. It is still read so an existing session
                // survives the upgrade; `tab2` carries the extra two fields.
                "tab" => {
                    let mut parts = rest.splitn(3, ' ');
                    let (Some(line), Some(column), Some(path)) =
                        (parts.next(), parts.next(), parts.next())
                    else {
                        continue;
                    };
                    let (Ok(line), Ok(column)) = (line.parse(), column.parse()) else {
                        continue;
                    };
                    session.tabs.push(SessionTab {
                        path: PathBuf::from(path),
                        line,
                        column,
                        pinned: false,
                        colour: TabColour::None,
                    });
                }
                "tab2" => {
                    let mut parts = rest.splitn(5, ' ');
                    let (Some(line), Some(column), Some(pinned), Some(colour), Some(path)) = (
                        parts.next(),
                        parts.next(),
                        parts.next(),
                        parts.next(),
                        parts.next(),
                    ) else {
                        continue;
                    };
                    let (Ok(line), Ok(column)) = (line.parse(), column.parse()) else {
                        continue;
                    };
                    session.tabs.push(SessionTab {
                        path: PathBuf::from(path),
                        line,
                        column,
                        pinned: pinned == "1",
                        colour: TabColour::from_key(colour).unwrap_or(TabColour::None),
                    });
                }
                "active" => {
                    if let Ok(index) = rest.parse() {
                        session.active = index;
                    }
                }
                _ => {}
            }
        }
        session.recent.truncate(MAX_RECENT);
        if session.active >= session.tabs.len() {
            session.active = session.tabs.len().saturating_sub(1);
        }
        session
    }

    /// Renders the on-disk format.
    pub fn render(&self) -> String {
        let mut out = String::from("# Neko++ session\n");
        for path in &self.recent {
            out.push_str("recent ");
            out.push_str(&path.to_string_lossy());
            out.push('\n');
        }
        for tab in &self.tabs {
            out.push_str(&format!(
                "tab2 {} {} {} {} {}\n",
                tab.line,
                tab.column,
                u8::from(tab.pinned),
                tab.colour.key(),
                tab.path.to_string_lossy()
            ));
        }
        out.push_str(&format!("active {}\n", self.active));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_session_round_trips() {
        let session = Session::default();
        assert_eq!(Session::parse(&session.render()), session);
    }

    #[test]
    fn tabs_and_recent_files_round_trip() {
        let session = Session {
            recent: vec![PathBuf::from("/tmp/a.rs"), PathBuf::from("/tmp/b.rs")],
            tabs: vec![
                SessionTab {
                    path: PathBuf::from("/tmp/a.rs"),
                    line: 12,
                    column: 4,
                    pinned: true,
                    colour: TabColour::Yellow,
                },
                SessionTab {
                    path: PathBuf::from("/tmp/b.rs"),
                    line: 0,
                    column: 0,
                    pinned: false,
                    colour: TabColour::None,
                },
            ],
            active: 1,
        };
        assert_eq!(Session::parse(&session.render()), session);
    }

    #[test]
    fn a_pinned_and_painted_tab_round_trips() {
        let session = Session {
            recent: Vec::new(),
            tabs: vec![SessionTab {
                path: PathBuf::from("/tmp/a.rs"),
                line: 1,
                column: 2,
                pinned: true,
                colour: TabColour::Purple,
            }],
            active: 0,
        };
        let text = session.render();
        assert!(text.contains("\ntab2 1 2 1 purple /tmp/a.rs\n"), "{text}");
        assert_eq!(Session::parse(&text), session);
    }

    #[test]
    fn the_old_three_field_tab_line_still_loads() {
        // A session written before tabs could be pinned or painted.
        let session = Session::parse("tab 5 3 /tmp/old.rs\nactive 0\n");
        assert_eq!(session.tabs.len(), 1);
        assert_eq!(session.tabs[0].path, PathBuf::from("/tmp/old.rs"));
        assert_eq!(session.tabs[0].line, 5);
        assert!(!session.tabs[0].pinned);
        assert_eq!(session.tabs[0].colour, TabColour::None);
    }

    #[test]
    fn an_unknown_tab_colour_falls_back_to_none() {
        let session = Session::parse("tab2 0 0 0 chartreuse /tmp/a.rs\n");
        assert_eq!(session.tabs[0].colour, TabColour::None);
        // The path still survives, so the tab is not lost over a colour.
        assert_eq!(session.tabs[0].path, PathBuf::from("/tmp/a.rs"));
    }

    #[test]
    fn a_truncated_tab2_line_is_skipped() {
        assert!(Session::parse("tab2 0 0 1\n").tabs.is_empty());
    }

    #[test]
    fn a_path_with_spaces_survives() {
        let session = Session {
            recent: Vec::new(),
            tabs: vec![SessionTab {
                path: PathBuf::from("/tmp/my notes.txt"),
                line: 3,
                column: 7,
                pinned: false,
                colour: TabColour::None,
            }],
            active: 0,
        };
        let parsed = Session::parse(&session.render());
        assert_eq!(parsed.tabs[0].path, PathBuf::from("/tmp/my notes.txt"));
        assert_eq!(parsed.tabs[0].line, 3);
    }

    #[test]
    fn malformed_lines_are_skipped() {
        let text = "tab not-a-number 0 /tmp/a\ntab 1 2\nnonsense\nrecent /tmp/ok\n";
        let session = Session::parse(text);
        assert!(session.tabs.is_empty());
        assert_eq!(session.recent, vec![PathBuf::from("/tmp/ok")]);
    }

    #[test]
    fn a_recent_file_is_stored_as_an_absolute_path() {
        // The list outlives the directory it was filled from: the session is
        // written now and read at the next start, from whichever directory the
        // editor is launched in.
        let mut session = Session::default();
        session.push_recent(Path::new("test/01-object.json"));
        assert!(session.recent[0].is_absolute());
        assert!(session.recent[0].ends_with("test/01-object.json"));
    }

    #[test]
    fn one_file_reached_two_ways_is_one_entry() {
        let mut session = Session::default();
        session.push_recent(Path::new("test/01-object.json"));
        let stored = session.recent[0].clone();
        assert!(stored.is_absolute());
        session.push_recent(&stored);
        assert_eq!(session.recent.len(), 1);
        session.push_recent(Path::new("test/01-object.json"));
        assert_eq!(session.recent.len(), 1);
    }

    #[test]
    fn pushing_a_recent_file_moves_it_to_the_front() {
        let mut session = Session::default();
        session.push_recent(Path::new("/tmp/a"));
        session.push_recent(Path::new("/tmp/b"));
        session.push_recent(Path::new("/tmp/a"));
        assert_eq!(
            session.recent,
            vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")]
        );
    }

    #[test]
    fn the_recent_list_is_capped() {
        let mut session = Session::default();
        for i in 0..MAX_RECENT + 5 {
            session.push_recent(Path::new(&format!("/tmp/{i}")));
        }
        assert_eq!(session.recent.len(), MAX_RECENT);
        assert_eq!(session.recent[0], PathBuf::from("/tmp/14"));
    }

    #[test]
    fn an_out_of_range_active_index_is_clamped() {
        let text = "tab 0 0 /tmp/a\nactive 9\n";
        assert_eq!(Session::parse(text).active, 0);
    }

    #[test]
    fn saving_and_loading_round_trips_through_the_filesystem() {
        // The only test that touches the environment, so it takes the lock and
        // puts the variable back before it returns.
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let dir = std::env::temp_dir().join(format!("npp-session-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let previous = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", &dir);

        let session = Session {
            recent: vec![PathBuf::from("/tmp/a.rs")],
            tabs: vec![SessionTab {
                path: PathBuf::from("/tmp/a.rs"),
                line: 7,
                column: 2,
                pinned: true,
                colour: TabColour::Blue,
            }],
            active: 0,
        };
        session.save();
        assert_eq!(Session::load(), session);

        match previous {
            Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
