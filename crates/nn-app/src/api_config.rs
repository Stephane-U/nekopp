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

//! The API's configuration file.
//!
//! The REST listener is off unless something asks for it, and the two things that
//! can ask are a switch and a file: `--rest=8085` for one launch, and
//! `api.yaml` or `api.json` in the configuration directory for every launch. The
//! file exists because a listener has more to say than a switch can carry -- a
//! token, whether the API may write -- and because a harness that starts the
//! editor wants to describe what it needs without wrapping the command line.
//!
//! YAML and JSON are read through the same door. JSON is parsed properly; YAML is
//! read as the subset a configuration file actually uses: `key: value` pairs, one
//! level of `section:` nesting, `#` comments, quoted or bare values. Lists and
//! deeper nesting are refused with a message rather than half-read, because a
//! configuration that silently means something other than what it says is worse
//! than one that does not load.
//!
//! Both syntaxes end up as one flat map of dotted keys -- `rest.port` -- so
//! everything after the reader is written once and tested once.
//!
//! Only `127.0.0.1` is served and the file cannot change that. An API that can
//! replace the text in a window and save it over a file should not be one edit
//! away from being reachable from the network.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nn_core::paths;

use crate::settings::Settings;

/// What a configuration file said, with nothing defaulted.
///
/// Every field is an `Option` because "the file did not mention it" and "the file
/// said false" are different answers to the question `resolve` is asked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// Whether to serve the API at all.
    pub enabled: Option<bool>,
    /// The port to serve it on.
    pub port: Option<u16>,
    /// The bearer token a request has to carry.
    pub token: Option<String>,
    /// Whether the API may write: `false` refuses `document.set`, `document.save`
    /// and `command.run`.
    pub read_only: Option<bool>,
}

/// A configuration file that was found and read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    /// What it said.
    pub config: Config,
    /// The file it was.
    pub path: PathBuf,
    /// The other file name that was there and was not read.
    pub ignored: Option<PathBuf>,
    /// Something the user should know about the file, such as a token that any
    /// user on the machine can read.
    pub warning: Option<String>,
}

/// What the listener will actually do, once the file, the switch and the settings
/// have had their say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// Whether to serve.
    pub enabled: bool,
    /// The port.
    pub port: u16,
    /// Handed to the listener unchanged.
    pub policy: crate::rest::Policy,
}

/// Why a configuration file did not load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// The file, so the message says which one.
    pub path: PathBuf,
    /// What is wrong with it.
    pub message: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.message)
    }
}

/// Which syntax a file is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// `key: value`, comments, one level of nesting.
    Yaml,
    /// Real JSON, nesting and all.
    Json,
}

/// The file names the reader looks for, in the order it looks.
pub const NAMES: [(&str, Format); 2] = [("api.yaml", Format::Yaml), ("api.json", Format::Json)];

/// Reads the configuration file, if there is one.
///
/// YAML wins when both are present: it is the name the documentation uses, and
/// the ignored one is reported so the choice is never a mystery.
pub fn load() -> Result<Option<Loaded>, Error> {
    let mut found: Option<(PathBuf, Format)> = None;
    let mut ignored: Option<PathBuf> = None;
    for (name, format) in NAMES {
        let Some(path) = paths::config_file(name) else {
            continue;
        };
        if !path.is_file() {
            continue;
        }
        match &found {
            Some(_) => ignored = Some(path),
            None => found = Some((path, format)),
        }
    }
    let Some((path, format)) = found else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(&path).map_err(|error| Error {
        path: path.clone(),
        message: error.to_string(),
    })?;
    let config = parse(&text, format).map_err(|message| Error {
        path: path.clone(),
        message,
    })?;
    // A token in a file the rest of the machine can read is a token that has
    // leaked. The listener still starts -- refusing would trade one problem for
    // another -- but the line is printed where the user will see it.
    let warning = match (&config.token, mode_of(&path)) {
        (Some(_), Some(mode)) if mode & 0o077 != 0 => Some(format!(
            "{} holds a token and other users can read it; `chmod 600 {}`",
            path.display(),
            path.display()
        )),
        _ => None,
    };
    Ok(Some(Loaded {
        config,
        path,
        ignored,
        warning,
    }))
}

/// The permission bits of a file, where the platform has them.
#[cfg(unix)]
fn mode_of(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions().mode() & 0o777)
}

/// The permission bits of a file, where the platform has them.
#[cfg(not(unix))]
fn mode_of(_path: &Path) -> Option<u32> {
    None
}

/// Parses one configuration document.
///
/// Split out of [`load`] so that the reader, the flattening and the value checks
/// are reachable from a test without a file on disk.
pub fn parse(text: &str, format: Format) -> Result<Config, String> {
    let map = match format {
        Format::Yaml => yaml(text)?,
        Format::Json => json(text)?,
    };
    config_of(&map)
}

/// The precedence, in one place.
///
/// The switch beats the file and the file beats `settings.txt`, because that is
/// the order of decreasing specificity: a word typed for this launch, a file
/// written for this machine's automation, a preference remembered from the
/// dialog. The environment variable beats the file for the token, because a
/// secret belongs in the environment rather than on disk wherever that is
/// possible.
pub fn resolve(
    config: Option<&Config>,
    cli_rest: bool,
    cli_port: Option<u16>,
    settings: &Settings,
    env_token: Option<String>,
) -> Resolved {
    let config = config.cloned().unwrap_or_default();
    Resolved {
        enabled: cli_rest || config.enabled.unwrap_or(settings.rest_enabled),
        port: cli_port
            .or(config.port)
            .unwrap_or(settings.rest_port),
        policy: crate::rest::Policy {
            token: env_token
                .filter(|token| !token.is_empty())
                .or(config.token),
            read_only: config.read_only.unwrap_or(false),
        },
    }
}

/// Reads the flat map out of a JSON document.
fn json(text: &str) -> Result<BTreeMap<String, String>, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("not JSON: {error}"))?;
    let mut out = BTreeMap::new();
    flatten(&value, "", &mut out)?;
    Ok(out)
}

/// Turns a JSON value into dotted keys, refusing lists.
fn flatten(
    value: &serde_json::Value,
    prefix: &str,
    out: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let name = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(child, &name, out)?;
            }
            Ok(())
        }
        // A null is "not mentioned": JSON has no comment, and a null is how a
        // file says a key is deliberately absent without deleting the line.
        Value::Null => Ok(()),
        Value::String(text) => {
            out.insert(prefix.to_string(), text.clone());
            Ok(())
        }
        Value::Bool(value) => {
            out.insert(prefix.to_string(), value.to_string());
            Ok(())
        }
        Value::Number(value) => {
            out.insert(prefix.to_string(), value.to_string());
            Ok(())
        }
        Value::Array(_) => Err(format!(
            "{prefix} is a list, and this file takes values: the API's settings are \
             port, token, enabled and read_only, one each"
        )),
    }
}

/// Reads the flat map out of a YAML document, as far as the subset goes.
fn yaml(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    let mut section: Option<(String, usize)> = None;
    for (index, raw) in text.lines().enumerate() {
        let number = index + 1;
        let line = strip_comment(raw);
        let line = line.trim_end();
        if line.trim().is_empty() || line.trim() == "---" || line.trim() == "..." {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if line[..indent].contains('\t') {
            return Err(format!(
                "line {number}: indented with a tab; this reader counts spaces"
            ));
        }
        let body = &line[indent..];
        if body.starts_with('-') {
            return Err(format!(
                "line {number}: a list is not something the API's settings need; \
                 use one `key: value` line per setting"
            ));
        }
        let Some((key, value)) = body.split_once(':') else {
            return Err(format!(
                "line {number}: {body:?} is not a setting; a setting is `key: value`"
            ));
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(format!("line {number}: a setting needs a name before the colon"));
        }
        let value = value.trim();
        // A line indented past the open section belongs to it. A line at its
        // indentation or above it closes the section and speaks for itself, which
        // is what makes a reader that allows one level still read
        // `rest:` ... `port:` without a rule about ordering.
        let inside = matches!(&section, Some((_, depth)) if indent > *depth);
        if inside {
            if value.is_empty() {
                return Err(format!(
                    "line {number}: `{key}:` opens a second level; the API's settings are \
                     port, token, enabled and read_only, and one section holds them"
                ));
            }
            let name = section
                .as_ref()
                .map(|(name, _)| name.clone())
                .unwrap_or_default();
            out.insert(format!("{name}.{key}"), scalar(value).to_string());
            continue;
        }
        section = None;
        if value.is_empty() {
            section = Some((key.to_string(), indent));
        } else {
            out.insert(key.to_string(), scalar(value).to_string());
        }
    }
    Ok(out)
}

/// Drops a `#` comment, leaving `#` inside quotes alone.
fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    for (index, character) in line.char_indices() {
        match quote {
            Some(open) if character == open => quote = None,
            Some(_) => {}
            None if character == '"' || character == '\'' => quote = Some(character),
            None if character == '#' => return &line[..index],
            None => {}
        }
    }
    line
}

/// A value with its quotes taken off, if it had a matching pair.
fn scalar(value: &str) -> &str {
    for quote in ['"', '\''] {
        if value.len() >= 2 && value.starts_with(quote) && value.ends_with(quote) {
            return &value[1..value.len() - 1];
        }
    }
    value
}

/// The known keys, checked, into a [`Config`].
///
/// Both spellings are accepted: `rest.port` inside a `rest:` section, and `port`
/// at the top of a file that is nothing but the API's settings. Unknown keys are
/// ignored the way `settings.txt` ignores them, so a file written for a later
/// version still loads here.
fn config_of(map: &BTreeMap<String, String>) -> Result<Config, String> {
    let mut config = Config::default();
    for (key, value) in map {
        match key.as_str() {
            "rest.enabled" | "enabled" => config.enabled = Some(boolean(key, value)?),
            "rest.read_only" | "read_only" => config.read_only = Some(boolean(key, value)?),
            "rest.port" | "port" => config.port = Some(port_of(key, value)?),
            "rest.token" | "token" => {
                config.token = Some(value.clone()).filter(|token| !token.is_empty())
            }
            _ => {}
        }
    }
    Ok(config)
}

/// A value that says yes or no.
fn boolean(key: &str, value: &str) -> Result<bool, String> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "on" | "yes" | "1" => Ok(true),
        "false" | "off" | "no" | "0" => Ok(false),
        _ => Err(format!("{key} takes true or false, not {value:?}")),
    }
}

/// A port, refusing the two ranges a client cannot reach.
fn port_of(key: &str, value: &str) -> Result<u16, String> {
    match value.parse::<u32>() {
        Ok(port) if (1..=65535).contains(&port) => Ok(port as u16),
        _ => Err(format!(
            "{key} takes a port between 1 and 65535, not {value:?}"
        )),
    }
}

/// A path as the one line at startup spells it.
pub fn describe(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Config {
        parse(text, Format::Yaml).unwrap()
    }

    #[test]
    fn a_flat_file_is_the_whole_configuration() {
        let config = parsed(
            "enabled: true\nport: 9000\ntoken: s3cret\nread_only: yes\n",
        );
        assert_eq!(
            config,
            Config {
                enabled: Some(true),
                port: Some(9000),
                token: Some("s3cret".to_string()),
                read_only: Some(true),
            }
        );
    }

    #[test]
    fn one_section_holds_the_same_keys() {
        let config = parsed(
            "# the API\nrest:\n  enabled: true\n  port: 8085\n  token: \"a#b\"\n",
        );
        assert_eq!(config.enabled, Some(true));
        assert_eq!(config.port, Some(8085));
        // The `#` is inside quotes, so it is part of the token, not a comment.
        assert_eq!(config.token.as_deref(), Some("a#b"));
    }

    #[test]
    fn a_comment_beside_a_value_is_dropped() {
        let config = parsed("port: 9000   # the port\n");
        assert_eq!(config.port, Some(9000));
    }

    #[test]
    fn an_empty_token_is_no_token() {
        assert_eq!(parsed("token:\n").token, None);
        assert_eq!(parsed("token: \"\"\n").token, None);
    }

    #[test]
    fn a_setting_the_file_does_not_mention_stays_unset() {
        let config = parsed("port: 9000\n");
        assert_eq!(config.enabled, None);
        assert_eq!(config.read_only, None);
        assert_eq!(config.token, None);
    }

    #[test]
    fn a_second_level_of_nesting_is_refused() {
        let error = parse("rest:\n  a:\n    b: 1\n", Format::Yaml).unwrap_err();
        assert!(error.contains("line 2"), "{error}");
    }

    #[test]
    fn a_list_is_refused_with_a_reason() {
        let error = parse("ports:\n  - 8085\n", Format::Yaml).unwrap_err();
        assert!(error.contains("list"), "{error}");
        // Under a section, the second level is what is wrong with it, and that is
        // the message that tells the user what to do about the file.
        let error = parse("rest:\n  ports:\n    - 8085\n", Format::Yaml).unwrap_err();
        assert!(error.contains("second level"), "{error}");
    }

    #[test]
    fn a_tab_is_not_an_indent() {
        let error = parse("rest:\n\tenabled: true\n", Format::Yaml).unwrap_err();
        assert!(error.contains("tab"), "{error}");
    }

    #[test]
    fn a_line_without_a_colon_is_refused() {
        let error = parse("enabled true\n", Format::Yaml).unwrap_err();
        assert!(error.contains("key: value"), "{error}");
    }

    #[test]
    fn a_port_that_is_not_a_port_is_refused() {
        for text in ["port: 0\n", "port: 70000\n", "port: http\n"] {
            let error = parse(text, Format::Yaml).unwrap_err();
            assert!(error.contains("port between 1 and 65535"), "{error}");
        }
    }

    #[test]
    fn a_flag_that_is_not_a_flag_is_refused() {
        let error = parse("enabled: maybe\n", Format::Yaml).unwrap_err();
        assert!(error.contains("true or false"), "{error}");
    }

    #[test]
    fn a_key_the_reader_does_not_know_is_ignored() {
        let config = parsed("port: 9000\ncolour: blue\nfuture_thing: 3\n");
        assert_eq!(config.port, Some(9000));
    }

    #[test]
    fn the_same_settings_load_from_json() {
        let config = parse(
            r#"{"rest": {"enabled": true, "port": 9000, "token": "s3cret"}}"#,
            Format::Json,
        )
        .unwrap();
        assert_eq!(
            config,
            Config {
                enabled: Some(true),
                port: Some(9000),
                token: Some("s3cret".to_string()),
                read_only: None,
            }
        );
    }

    #[test]
    fn a_json_null_is_not_a_setting() {
        let config = parse(r#"{"port": 9000, "token": null}"#, Format::Json).unwrap();
        assert_eq!(config.token, None);
        assert_eq!(config.port, Some(9000));
    }

    #[test]
    fn a_json_list_is_refused() {
        let error = parse(r#"{"ports": [8085]}"#, Format::Json).unwrap_err();
        assert!(error.contains("list"), "{error}");
    }

    #[test]
    fn a_json_number_travels_as_its_digits() {
        let config = parse(r#"{"port": 9000}"#, Format::Json).unwrap();
        assert_eq!(config.port, Some(9000));
        let error = parse(r#"{"port": "nine thousand"}"#, Format::Json).unwrap_err();
        assert!(error.contains("port between"), "{error}");
    }

    #[test]
    fn a_file_that_is_not_json_says_so() {
        let error = parse("port: 9000\n", Format::Json).unwrap_err();
        assert!(error.starts_with("not JSON:"), "{error}");
    }

    #[test]
    fn the_switch_beats_the_file_and_the_file_beats_the_settings() {
        let settings = Settings {
            rest_enabled: true,
            rest_port: 7000,
            ..Settings::default()
        };
        let file = Config {
            enabled: Some(false),
            port: Some(9000),
            ..Config::default()
        };
        // The switch is a word typed for this launch: it wins over both.
        let resolved = resolve(Some(&file), true, Some(9100), &settings, None);
        assert!(resolved.enabled);
        assert_eq!(resolved.port, 9100);
        // The file wins over the preference.
        let resolved = resolve(Some(&file), false, None, &settings, None);
        assert!(!resolved.enabled);
        assert_eq!(resolved.port, 9000);
        // With neither, the preference stands.
        let resolved = resolve(None, false, None, &settings, None);
        assert!(resolved.enabled);
        assert_eq!(resolved.port, 7000);
    }

    #[test]
    fn the_environment_holds_the_token_when_it_can() {
        let file = Config {
            token: Some("from-file".to_string()),
            ..Config::default()
        };
        let resolved = resolve(Some(&file), true, None, &Settings::default(), Some("from-env".into()));
        assert_eq!(resolved.policy.token.as_deref(), Some("from-env"));
        let resolved = resolve(Some(&file), true, None, &Settings::default(), None);
        assert_eq!(resolved.policy.token.as_deref(), Some("from-file"));
    }

    #[test]
    fn read_only_is_the_files_alone_and_off_by_default() {
        let resolved = resolve(None, true, None, &Settings::default(), None);
        assert!(!resolved.policy.read_only);
        let file = Config {
            read_only: Some(true),
            ..Config::default()
        };
        let resolved = resolve(Some(&file), true, None, &Settings::default(), None);
        assert!(resolved.policy.read_only);
    }
}
