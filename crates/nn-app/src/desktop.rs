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

//! Handing work to the desktop.
//!
//! Notepad++'s File menu is full of rows that do not edit text: open the
//! containing folder, open a shell there, move a file to the recycle bin, open a
//! URL. On Windows those are shell calls. Here they are `xdg-open`, a terminal
//! emulator, and the freedesktop trash specification.
//!
//! The split is deliberate: this module decides *what* to run and *where the
//! trash lives*, and returns a `Result` rather than printing. The caller owns the
//! status bar, so a failure reads the same as every other failure in the app.
//!
//! The trash is the only part with real logic, and it is written to the
//! specification rather than to a private directory: a file moved here shows up
//! in the desktop's own trash can and can be restored from it. A private
//! directory would have been easier and would have been a lie.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

/// Opens `path` with whatever the desktop has registered for it.
///
/// Used for both "open in default viewer" and "open containing folder": the
/// desktop already knows the difference between a file and a directory, so there
/// is no reason to duplicate that decision here.
pub fn open_path(path: &Path) -> io::Result<()> {
    spawn("xdg-open", &[path.as_os_str().to_os_string()])
}

/// Opens `url` in the default browser.
pub fn open_url(url: &str) -> io::Result<()> {
    spawn("xdg-open", &[OsString::from(url)])
}

/// Opens `url` with `program`, falling back to the desktop's default handler.
///
/// The named browser is tried first because that is what the menu row says it
/// will do. The fallback exists because a row that failed on a machine without
/// that particular browser would be a row that never worked, and the user's
/// default browser is a better answer than an error message.
pub fn open_url_with(program: &str, url: &str) -> io::Result<()> {
    match spawn(program, &[OsString::from(url)]) {
        Ok(()) => Ok(()),
        Err(_) => open_url(url),
    }
}

/// The `file://` URL for `path`.
///
/// Percent-encoded, because a path with a space in it is not a URL and a browser
/// handed one will either refuse it or truncate at the space. The separators and
/// the unreserved set are left alone: encoding `/` as `%2F` would produce a URL
/// that names one very long file name rather than a path.
pub fn file_url(path: &Path) -> String {
    let mut out = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Opens a terminal emulator whose working directory is `dir`.
///
/// `shell` names the program to run inside it. `None` means the user's login
/// shell, which is what a bare "open a terminal here" means.
///
/// The emulator is tried in order of preference and the first one that exists
/// wins. `$TERMINAL` comes first because a user who set it meant it.
pub fn open_terminal(dir: &Path, shell: Option<&str>) -> io::Result<()> {
    let candidates: Vec<(String, Vec<OsString>)> = terminal_commands(shell);
    let mut last = io::Error::new(io::ErrorKind::NotFound, "no terminal emulator found");
    for (program, args) in candidates {
        let mut command = std::process::Command::new(&program);
        command.args(&args).current_dir(dir);
        match command.spawn() {
            Ok(_) => return Ok(()),
            Err(err) => last = err,
        }
    }
    Err(last)
}

/// The terminal invocations to try, most preferred first.
///
/// Split out from [`open_terminal`] so the argument shapes can be tested without
/// a display: getting `--working-directory` wrong is the kind of mistake that
/// only shows up as a terminal opening in the wrong place.
fn terminal_commands(shell: Option<&str>) -> Vec<(String, Vec<OsString>)> {
    let mut out = Vec::new();
    let run: Vec<OsString> = match shell {
        Some(shell) => vec![OsString::from("-e"), OsString::from(shell)],
        None => Vec::new(),
    };
    if let Some(terminal) = std::env::var_os("TERMINAL") {
        let terminal = terminal.to_string_lossy().into_owned();
        let mut args = vec![OsString::from("--working-directory"), OsString::from(".")];
        args.extend(run.iter().cloned());
        out.push((terminal, args));
    }
    // `x-terminal-emulator` is the Debian alternative and resolves to whatever
    // the distribution considers the default, so it is the best generic guess.
    let mut generic = vec![OsString::from("--working-directory"), OsString::from(".")];
    generic.extend(run.iter().cloned());
    out.push(("x-terminal-emulator".to_string(), generic.clone()));
    out.push(("gnome-terminal".to_string(), generic.clone()));
    out.push(("konsole".to_string(), generic));
    // xterm has no working-directory switch; the caller's `current_dir` is all
    // it gets, and it honours that.
    out.push(("xterm".to_string(), run));
    out
}

/// The command that runs `shell` in a terminal, for the status bar to name.
pub fn shell_name(shell: Option<&str>) -> &str {
    shell.unwrap_or("the login shell")
}

/// Moves `path` to the freedesktop trash.
///
/// Returns where the file went, which is what the caller reports: a user who
/// moved the wrong file needs to know where to find it.
pub fn trash(path: &Path) -> Result<PathBuf, TrashError> {
    let root = trash_root().ok_or(TrashError::NoHome)?;
    let files = root.join("files");
    let info = root.join("info");
    std::fs::create_dir_all(&files).map_err(TrashError::Io)?;
    std::fs::create_dir_all(&info).map_err(TrashError::Io)?;

    let name = path
        .file_name()
        .ok_or_else(|| TrashError::Io(io::Error::new(io::ErrorKind::InvalidInput, "no file name")))?;
    let taken = |candidate: &str| files.join(candidate).exists() || info.join(format!("{candidate}.trashinfo")).exists();
    let target = unique_name(&name.to_string_lossy(), &taken);

    let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let entry = trash_info(&absolute, &now_stamp());
    std::fs::write(info.join(format!("{target}.trashinfo")), entry).map_err(TrashError::Io)?;
    let destination = files.join(&target);
    if let Err(err) = std::fs::rename(path, &destination) {
        // The rename failed, so the info file describes a file that is not
        // there. Leaving it would put a phantom entry in the desktop's trash.
        let _ = std::fs::remove_file(info.join(format!("{target}.trashinfo")));
        return Err(TrashError::Io(err));
    }
    Ok(destination)
}

/// Why a file could not be trashed.
#[derive(Debug)]
pub enum TrashError {
    /// There is no home directory, so there is nowhere to put the trash.
    NoHome,
    /// The filesystem refused.
    Io(io::Error),
}

impl std::fmt::Display for TrashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrashError::NoHome => write!(f, "no home directory, so no trash"),
            TrashError::Io(err) => write!(f, "{err}"),
        }
    }
}

/// The trash directory, per the freedesktop base directory specification.
fn trash_root() -> Option<PathBuf> {
    if let Some(data) = std::env::var_os("XDG_DATA_HOME") {
        let data = PathBuf::from(data);
        if !data.as_os_str().is_empty() {
            return Some(data.join("Trash"));
        }
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share/Trash"))
}

/// A name that is not already in the trash.
///
/// The specification says to append `.1`, `.2` and so on before the extension,
/// so `notes.txt` becomes `notes.1.txt`. Keeping the extension matters: the
/// desktop's trash can shows the name, and a file that lost its extension would
/// be unrecognisable there.
fn unique_name(name: &str, taken: &impl Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_string();
    }
    let (stem, extension) = match name.rfind('.') {
        Some(dot) if dot > 0 => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    };
    for n in 1..10_000 {
        let candidate = format!("{stem}.{n}{extension}");
        if !taken(&candidate) {
            return candidate;
        }
    }
    // Ten thousand files of the same name in the trash is not a real case; a
    // name that cannot collide is better than a loop that never ends.
    format!("{stem}.{}{extension}", now_stamp().replace(':', "-"))
}

/// The `.trashinfo` body for a file that was at `path` when it was deleted.
fn trash_info(path: &Path, deleted: &str) -> String {
    format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        path.display(),
        deleted
    )
}

/// The current local time as `YYYY-MM-DDThh:mm:ss`, which is what the trash
/// specification asks for.
///
/// Derived from the editor's own local-time stamp rather than recomputed, so the
/// trash and `Insert Date/Time` can never disagree about what time it is.
fn now_stamp() -> String {
    nn_core::local_stamp(nn_core::now_secs()).replace(' ', "T")
}

/// Runs `program` with `args`, detached.
fn spawn(program: &str, args: &[OsString]) -> io::Result<()> {
    std::process::Command::new(program)
        .args(args)
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_free_name_is_kept_as_it_is() {
        assert_eq!(unique_name("notes.txt", &|_| false), "notes.txt");
    }

    #[test]
    fn a_taken_name_gets_a_number_before_the_extension() {
        let taken = |name: &str| name == "notes.txt" || name == "notes.1.txt";
        assert_eq!(unique_name("notes.txt", &taken), "notes.2.txt");
    }

    #[test]
    fn a_name_without_an_extension_gets_the_number_at_the_end() {
        let taken = |name: &str| name == "Makefile";
        assert_eq!(unique_name("Makefile", &taken), "Makefile.1");
    }

    #[test]
    fn a_dot_file_keeps_its_leading_dot() {
        // `.bashrc` has no extension: the dot is part of the name, so the number
        // goes after it rather than turning the file into `.1bashrc`.
        let taken = |name: &str| name == ".bashrc";
        assert_eq!(unique_name(".bashrc", &taken), ".bashrc.1");
    }

    #[test]
    fn the_trash_info_names_the_original_path_and_the_date() {
        let entry = trash_info(Path::new("/home/u/notes.txt"), "2024-05-06T07:08:09");
        assert_eq!(
            entry,
            "[Trash Info]\nPath=/home/u/notes.txt\nDeletionDate=2024-05-06T07:08:09\n"
        );
    }

    #[test]
    fn the_stamp_has_the_shape_the_specification_asks_for() {
        let stamp = now_stamp();
        assert_eq!(stamp.len(), 19, "{stamp}");
        assert_eq!(&stamp[4..5], "-");
        assert_eq!(&stamp[7..8], "-");
        assert_eq!(&stamp[10..11], "T");
        assert_eq!(&stamp[13..14], ":");
        assert_eq!(&stamp[16..17], ":");
    }

    #[test]
    fn a_terminal_with_a_shell_runs_it() {
        let commands = terminal_commands(Some("pwsh"));
        let (_, args) = &commands[0];
        assert!(args.iter().any(|arg| arg == "-e"));
        assert!(args.iter().any(|arg| arg == "pwsh"));
    }

    #[test]
    fn a_terminal_without_a_shell_runs_the_login_shell() {
        let commands = terminal_commands(None);
        let (_, args) = &commands[0];
        assert!(!args.iter().any(|arg| arg == "-e"));
    }

    #[test]
    fn a_file_url_keeps_its_separators_and_encodes_the_rest() {
        let url = file_url(Path::new("/tmp/a b/c.txt"));
        assert_eq!(url, "file:///tmp/a%20b/c.txt");
    }

    #[test]
    fn a_file_url_leaves_an_ordinary_path_alone() {
        assert_eq!(file_url(Path::new("/tmp/a.txt")), "file:///tmp/a.txt");
    }

    #[test]
    fn every_terminal_candidate_is_tried() {
        let commands = terminal_commands(None);
        let names: Vec<&str> = commands.iter().map(|(name, _)| name.as_str()).collect();
        assert!(names.contains(&"x-terminal-emulator"));
        assert!(names.contains(&"xterm"));
    }
}
