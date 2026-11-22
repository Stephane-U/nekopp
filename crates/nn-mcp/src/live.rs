//! One command, sent to the editor that is already open.
//!
//! [`crate::run`] is the headless path: text in, text out, no window anywhere.
//! This is the other one. A call is handed to the running editor through
//! `npp --call '<json>'`, which is the editor's own API client: it finds the
//! window over D-Bus, asks it to do the work, and prints the answer as JSON.
//!
//! The indirection is deliberate. The protocol between the two is the editor's
//! API, not a second one invented here: `live_call` sends whatever a client
//! sends, and a client that has `npp` in a terminal has exactly the same
//! answers. It also keeps this crate free of D-Bus, which is a dependency the
//! documentation tools would otherwise carry for one tool.

use std::path::{Path, PathBuf};
use std::process::Command;

/// What can go wrong before the editor answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No `npp` to run: nothing on the path, nothing next to this binary.
    NoEditor(String),
    /// `npp` ran and refused, with its own explanation on stderr.
    Refused(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NoEditor(tried) => write!(
                formatter,
                "No Neko++ binary to talk to. Set NPP_BIN to the editor's path, \
                 put `npp` on PATH, or send the call to `run_command` instead, \
                 which needs no window. Tried: {tried}"
            ),
            Error::Refused(message) => write!(formatter, "{message}"),
        }
    }
}

/// Sends one call to the running editor and returns the JSON it printed.
///
/// The call is the editor's own API request: `{"method": "app.status"}`, or
/// `{"method": "command.run", "command": "sort_asc", "whole_document": true}`.
pub fn call(json: &str) -> Result<String, Error> {
    let binary = binary()?;
    run(&binary, json)
}

/// The editor's location, in the order a person would look for it.
///
/// `NPP_BIN` first, because a checkout or a wrapper is the case this cannot
/// guess; then next to this binary, which is what an installed `npp-mcp` beside
/// an installed `npp` looks like; then the PATH.
pub fn binary() -> Result<PathBuf, Error> {
    if let Some(named) = std::env::var_os("NPP_BIN") {
        let path = PathBuf::from(named);
        if path.is_file() {
            return Ok(path);
        }
    }

    if let Some(own) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        for name in ["npp", "nn"] {
            let candidate = own.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }

    let path = std::env::var_os("PATH").unwrap_or_default();
    for directory in std::env::split_paths(&path) {
        for name in ["npp", "nn"] {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }

    Err(Error::NoEditor(
        "NPP_BIN, a sibling of this binary, and every directory on PATH".to_string(),
    ))
}

/// Runs `<binary> --call <json>` and returns its stdout.
fn run(binary: &Path, json: &str) -> Result<String, Error> {
    let output = Command::new(binary)
        .arg("--call")
        .arg(json)
        .output()
        .map_err(|error| Error::Refused(format!("could not run {}: {error}", binary.display())))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(Error::Refused(if stderr.is_empty() {
        if stdout.is_empty() {
            format!("{} printed nothing and failed", binary.display())
        } else {
            stdout
        }
    } else {
        stderr
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in for the editor: prints the arguments it was given, or fails
    /// with the message it was told to fail with.
    fn fake(body: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "nn-mcp-live-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("npp");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn a_call_is_passed_through_word_for_word() {
        let path = fake(r#"printf '%s' "$2""#);
        let json = r#"{"method":"command.run","command":"sort_asc"}"#;
        assert_eq!(run(&path, json), Ok(json.to_string()));
    }

    #[test]
    fn the_call_arrives_after_the_flag() {
        let path = fake(r#"printf '%s' "$1""#);
        assert_eq!(run(&path, "{}"), Ok("--call".to_string()));
    }

    #[test]
    fn a_refusal_is_the_message_the_editor_printed() {
        let path = fake(r#"echo 'npp: no window is running' >&2; exit 1"#);
        assert_eq!(
            run(&path, "{}"),
            Err(Error::Refused("npp: no window is running".to_string()))
        );
    }

    #[test]
    fn a_refusal_with_nothing_to_say_still_says_something() {
        let path = fake("exit 1");
        let message = run(&path, "{}").unwrap_err().to_string();
        assert!(message.contains("printed nothing and failed"), "{message}");
    }

    #[test]
    fn a_binary_that_cannot_be_run_is_named() {
        let path = Path::new("/nonexistent/npp");
        let message = run(path, "{}").unwrap_err().to_string();
        assert!(message.contains("/nonexistent/npp"), "{message}");
    }

    #[test]
    fn no_binary_anywhere_names_the_places_it_looked() {
        let message = Error::NoEditor("everywhere".to_string()).to_string();
        assert!(message.contains("NPP_BIN"), "{message}");
        assert!(message.contains("everywhere"), "{message}");
    }
}
