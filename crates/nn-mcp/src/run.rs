//! The editor's text commands, run without a window.
//!
//! `nn_core::ops` is the table: a name, the options it reads, and the function
//! that implements it. This module is the JSON in front of it -- the arguments a
//! client sends turned into [`nn_core::ops::Options`], and the answer turned back
//! into text -- so that "sort my file" is a request a harness can make with no
//! editor running, no window, no display and no D-Bus.
//!
//! ## Where the text comes from
//!
//! Either `text`, which is the document the caller is holding, or `path`, which
//! is a file on this machine. Exactly one: a request with both has two answers
//! and a request with neither has nothing to work on. `path` with `write` puts
//! the result back where it came from, which is the one operation here that
//! changes anything on disk.

use std::fs;
use std::path::PathBuf;

use nn_core::ops::{self, Options};
use nn_core::lines::{SortKey, SortOptions, SortOrder};
use serde_json::Value;

/// What a command produced.
#[derive(Debug)]
pub struct Outcome {
    /// The text after the command ran.
    pub text: String,
    /// The file the result was written to, when the caller asked for that.
    pub wrote: Option<PathBuf>,
}

/// The command names, for a schema or a listing.
pub fn commands() -> &'static [ops::Command] {
    ops::commands()
}

/// The spellings `sort_key` accepts, with the comparison each one means.
const SORT_KEYS: &[(&str, SortKey)] = &[
    ("text", SortKey::Text),
    ("numeric", SortKey::Numeric),
    ("text_ignore_case", SortKey::TextIgnoreCase),
    ("locale", SortKey::Locale),
    ("integer", SortKey::Integer),
    ("decimal_comma", SortKey::DecimalComma),
    ("decimal_dot", SortKey::DecimalDot),
    ("length", SortKey::Length),
];

/// The widest tab stop a caller may ask for.
///
/// A tab width is a column count, and a hundred-column tab stop is a caller's
/// mistake rather than a layout: the cap keeps the conversion from allocating a
/// line of spaces per tab.
const MAX_WIDTH: usize = 64;

/// Runs `command` over the text or file the arguments name.
pub fn run(command: &str, arguments: &Value) -> Result<Outcome, String> {
    if !ops::knows(command) {
        return Err(format!(
            "no command called {command:?}; `list_commands` has the ones that work on text"
        ));
    }
    let options = options(command, arguments)?;
    let (text, path) = source(arguments)?;
    let out = ops::run(command, &text, &options).map_err(|error| error.to_string())?;
    let wrote = match (path, write(arguments)?) {
        (Some(path), true) => {
            fs::write(&path, out.as_bytes())
                .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
            Some(path)
        }
        _ => None,
    };
    Ok(Outcome { text: out, wrote })
}

/// The options the arguments carry, or the defaults for the ones they do not.
///
/// An option a command does not read is ignored rather than refused: a client
/// that sends the same option block to every command is doing the obvious thing,
/// and `list_commands` says which ones each command uses.
fn options(command: &str, arguments: &Value) -> Result<Options, String> {
    let mut options = Options::default();
    if let Some(name) = string(arguments, "sort_key")? {
        let wanted = name.trim().to_ascii_lowercase().replace([' ', '-'], "_");
        let (_, key) = SORT_KEYS
            .iter()
            .find(|(spelling, _)| *spelling == wanted)
            .ok_or_else(|| {
                let known: Vec<&str> = SORT_KEYS.iter().map(|(name, _)| *name).collect();
                format!(
                    "`sort_key` must be one of {} -- not {name:?}",
                    known.join(", ")
                )
            })?;
        options.sort = SortOptions {
            key: *key,
            ..options.sort
        };
    }
    if let Some(name) = string(arguments, "sort_order")? {
        options.sort = SortOptions {
            order: match name.trim().to_ascii_lowercase().as_str() {
                "ascending" | "asc" => SortOrder::Ascending,
                "descending" | "desc" => SortOrder::Descending,
                other => {
                    return Err(format!(
                        "`sort_order` must be ascending or descending -- not {other:?}"
                    ))
                }
            },
            ..options.sort
        };
    }
    if let Some(value) = boolean(arguments, "case_sensitive")? {
        options.sort = SortOptions {
            case_sensitive: value,
            ..options.sort
        };
    }
    if let Some(value) = number(arguments, "width")? {
        if value == 0 {
            return Err("`width` must be 1 or more".to_string());
        }
        options.width = (value as usize).min(MAX_WIDTH);
    }
    if let Some(value) = string(arguments, "delimiter")? {
        options.delimiter = value;
    }
    if let Some(value) = string(arguments, "prefix")? {
        options.prefix = value;
    }
    if let Some(value) = string(arguments, "block_open")? {
        options.block_open = value;
    }
    if let Some(value) = string(arguments, "block_close")? {
        options.block_close = value;
    }
    if let Some(value) = number(arguments, "seed")? {
        options.seed = value;
    }
    // A splitting command with an empty delimiter has no lines to split on and
    // would return the text unchanged; saying so is more useful than a silent
    // no-op.
    if command == "split_lines" && options.delimiter.is_empty() {
        return Err("`split_lines` needs a `delimiter` to split on".to_string());
    }
    Ok(options)
}

/// The text to work on, and the file it came from if it came from one.
fn source(arguments: &Value) -> Result<(String, Option<PathBuf>), String> {
    let text = string(arguments, "text")?;
    let path = string(arguments, "path")?.map(PathBuf::from);
    match (text, path) {
        (Some(text), None) => Ok((text, None)),
        (None, Some(path)) => {
            let bytes = fs::read(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            let text = String::from_utf8(bytes).map_err(|_| {
                format!(
                    "{} is not UTF-8; `run_command` works on UTF-8 text, and the window can \
                     convert other encodings",
                    path.display()
                )
            })?;
            Ok((text, Some(path)))
        }
        (Some(_), Some(_)) => {
            Err("send either `text` or `path`, not both: the request has two answers".to_string())
        }
        (None, None) => Err("send `text` or `path`: there is nothing to work on".to_string()),
    }
}

fn write(arguments: &Value) -> Result<bool, String> {
    Ok(boolean(arguments, "write")?.unwrap_or(false))
}

fn string(arguments: &Value, name: &str) -> Result<Option<String>, String> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(format!("`{name}` must be a string")),
    }
}

fn number(arguments: &Value, name: &str) -> Result<Option<u64>, String> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("`{name}` must be 0 or more")),
        Some(_) => Err(format!("`{name}` must be a number")),
    }
}

fn boolean(arguments: &Value, name: &str) -> Result<Option<bool>, String> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(format!("`{name}` must be true or false")),
    }
}

/// The listing `list_commands` returns: the table, and the commands the window
/// has that this one does not.
pub fn listing() -> String {
    let commands = ops::commands();
    let mut text = format!(
        "{} commands work on text and need no window:\n\n",
        commands.len()
    );
    for command in commands {
        let options = if command.params.is_empty() {
            String::new()
        } else {
            let names: Vec<&str> = command.params.iter().map(|param| param.name()).collect();
            format!(" ({})", names.join(", "))
        };
        text.push_str(&format!("- {}{options}\n", command.name));
    }
    text.push_str(
        "\nSend one with `run_command`, over `text` or over a file's `path`. \
         Everything else the editor can do needs its window: `live_call` drives a \
         running one, and `get_command` says which commands are which.\n",
    );
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn arguments(value: Value) -> Value {
        value
    }

    #[test]
    fn a_command_runs_over_the_text_it_is_given() {
        let out = run("sort_asc", &arguments(json!({"text": "b\na\n"}))).unwrap();
        assert_eq!(out.text, "a\nb\n");
        assert!(out.wrote.is_none());
    }

    #[test]
    fn an_unknown_command_names_the_listing() {
        let error = run("sort_sideways", &arguments(json!({"text": "a\n"}))).unwrap_err();
        assert!(error.contains("list_commands"), "{error}");
        assert!(error.contains("sort_sideways"), "{error}");
    }

    #[test]
    fn both_sources_is_an_error_and_neither_is_an_error() {
        let both = run(
            "sort_asc",
            &arguments(json!({"text": "a\n", "path": "/tmp/x.txt"})),
        )
        .unwrap_err();
        assert!(both.contains("not both"), "{both}");
        let neither = run("sort_asc", &arguments(json!({}))).unwrap_err();
        assert!(neither.contains("nothing to work on"), "{neither}");
    }

    #[test]
    fn the_sort_options_are_read_in_the_spellings_a_client_sends() {
        let out = run(
            "sort_lines",
            &arguments(json!({
                "text": "10\n9\n",
                "sort_key": "Numeric",
                "sort_order": "desc",
            })),
        );
        // "Numeric" is the first number in the line, so 10 sorts after 9
        // ascending and before it descending.
        assert_eq!(out.unwrap().text, "10\n9\n");
        let out = run(
            "sort_lines",
            &arguments(json!({
                "text": "10\n9\n",
                "sort_key": "numeric",
                "sort_order": "ascending",
            })),
        );
        assert_eq!(out.unwrap().text, "9\n10\n");
    }

    #[test]
    fn a_misspelled_sort_key_says_what_the_keys_are() {
        let error = run(
            "sort_lines",
            &arguments(json!({"text": "a\n", "sort_key": "alphabetical"})),
        )
        .unwrap_err();
        assert!(error.contains("text, numeric"), "{error}");
        assert!(error.contains("alphabetical"), "{error}");
    }

    #[test]
    fn a_width_of_zero_is_refused_and_a_huge_one_is_capped() {
        let zero = run("tabs_to_spaces", &arguments(json!({"text": "\tx", "width": 0})));
        assert!(zero.unwrap_err().contains("1 or more"));
        let huge = run(
            "tabs_to_spaces",
            &arguments(json!({"text": "\tx", "width": 100000})),
        )
        .unwrap();
        assert_eq!(huge.text.chars().count(), MAX_WIDTH + 1);
    }

    #[test]
    fn the_seed_makes_a_shuffle_repeatable() {
        let text = "a\nb\nc\nd\ne\nf\n";
        let first = run(
            "randomize_line_order",
            &arguments(json!({"text": text, "seed": 42})),
        )
        .unwrap();
        let again = run(
            "randomize_line_order",
            &arguments(json!({"text": text, "seed": 42})),
        )
        .unwrap();
        assert_eq!(first.text, again.text);
    }

    #[test]
    fn splitting_without_a_delimiter_is_refused() {
        let error = run(
            "split_lines",
            &arguments(json!({"text": "a,b", "delimiter": ""})),
        )
        .unwrap_err();
        assert!(error.contains("needs a `delimiter`"), "{error}");
    }

    #[test]
    fn a_file_is_read_and_can_be_written_back() {
        let path = std::env::temp_dir().join(format!("nn-run-{}.txt", std::process::id()));
        fs::write(&path, "c\nb\na\n").unwrap();
        let read = run(
            "sort_asc",
            &arguments(json!({"path": path.display().to_string()})),
        )
        .unwrap();
        assert_eq!(read.text, "a\nb\nc\n");
        assert!(read.wrote.is_none());
        // Not written until asked.
        assert_eq!(fs::read_to_string(&path).unwrap(), "c\nb\na\n");

        let wrote = run(
            "sort_desc",
            &arguments(json!({"path": path.display().to_string(), "write": true})),
        )
        .unwrap();
        assert_eq!(wrote.wrote.as_deref(), Some(path.as_path()));
        assert_eq!(fs::read_to_string(&path).unwrap(), "c\nb\na\n");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_missing_file_says_so() {
        let error = run(
            "sort_asc",
            &arguments(json!({"path": "/tmp/nn-no-such-file-4f2a.txt"})),
        )
        .unwrap_err();
        assert!(error.contains("cannot read"), "{error}");
    }

    #[test]
    fn a_file_that_is_not_utf8_says_so() {
        let path = std::env::temp_dir().join(format!("nn-run-latin1-{}.txt", std::process::id()));
        fs::write(&path, [0xE9, 0x0A]).unwrap();
        let error = run(
            "sort_asc",
            &arguments(json!({"path": path.display().to_string()})),
        )
        .unwrap_err();
        assert!(error.contains("not UTF-8"), "{error}");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn an_option_of_the_wrong_type_is_refused() {
        let error = run(
            "sort_lines",
            &arguments(json!({"text": "a\n", "seed": "soon"})),
        )
        .unwrap_err();
        assert!(error.contains("`seed` must be a number"), "{error}");
    }

    #[test]
    fn every_command_is_runnable_from_json_with_no_options() {
        for command in ops::commands() {
            let out = run(command.name, &arguments(json!({"text": "b\na\n"})));
            assert!(out.is_ok(), "{}: {:?}", command.name, out.err());
        }
    }

    #[test]
    fn the_listing_names_every_command_and_its_options() {
        let text = listing();
        assert!(text.starts_with("37 commands"), "{text}");
        assert!(text.contains("- sort_asc\n"), "{text}");
        assert!(text.contains("- sort_lines (sort_key, sort_order, case_sensitive)"), "{text}");
        assert!(text.contains("live_call"), "{text}");
        for command in ops::commands() {
            assert!(text.contains(command.name), "{} is not listed", command.name);
        }
    }
}
