//! The JSON API a running window answers.
//!
//! One request in, one answer out, and the same answers whether the request came
//! from `npp --call` on a terminal, from a script, or from the MCP server. The
//! request is parsed here rather than in `main.rs` because the parsing is where
//! the decisions are -- which document a call means, which ranges are allowed,
//! what a misspelling is told -- and a decision that lives in `main.rs` is a
//! decision no test can reach.
//!
//! ## Shape
//!
//! A request is an object with a `method`:
//!
//! ```json
//! {"method": "document.get", "path": "/tmp/notes.txt"}
//! {"method": "document.set", "active": true, "text": "hello\n"}
//! {"method": "command.run", "command": "sort_asc"}
//! ```
//!
//! An answer is either `{"ok": true, ...}` or
//! `{"error": {"code": "...", "message": "..."}}`. The codes are stable and few:
//! a caller branches on the code and shows the message to a person.

use std::fmt;

use serde_json::{json, Value};

/// Which document a call is about.
///
/// `Active` is the default because it is what a person means by "the document":
/// the one they are looking at. A path picks another tab out, and a path that no
/// tab has open is an error rather than an implicit open, because a caller that
/// meant to open one should say so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Which {
    /// The tab with the caret in it.
    Active,
    /// The tab holding this path, as it was opened.
    Path(String),
}

/// A call the window knows how to answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    /// What the window has open, and which version it is.
    Status,
    /// Every open document, in tab order.
    DocumentList,
    /// One document's text, and where the caret is in it.
    DocumentGet {
        which: Which,
        range: Option<(usize, usize)>,
    },
    /// Replaces the whole document, or `range` of it.
    DocumentSet {
        which: Which,
        text: String,
        range: Option<(usize, usize)>,
    },
    /// Writes the document to its file.
    DocumentSave { which: Which },
    /// Runs a menu command by its action name.
    Command {
        name: String,
        /// Whether to answer with the document's text afterwards.
        text: bool,
        /// A block of the document to run it over, as character offsets.
        range: Option<(usize, usize)>,
        /// Whether to run it over the whole document instead of the caret's line.
        whole: bool,
    },
    /// The methods, for a caller that has forgotten them.
    Help,
}

/// Why a request was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// The stable code a caller branches on.
    pub code: &'static str,
    /// What a person is told.
    pub message: String,
}

impl Error {
    /// The answer this error is.
    pub fn value(&self) -> Value {
        json!({"error": {"code": self.code, "message": self.message}})
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// The code for a request that is not shaped like a call.
pub const BAD_REQUEST: &str = "bad_request";
/// The code for a method this build does not have.
pub const UNKNOWN_METHOD: &str = "unknown_method";
/// The code for a document that is not open.
pub const UNKNOWN_DOCUMENT: &str = "unknown_document";
/// The code for a call that was understood and then failed.
pub const FAILED: &str = "failed";

fn bad(message: impl Into<String>) -> Error {
    Error {
        code: BAD_REQUEST,
        message: message.into(),
    }
}

/// Parses one request.
pub fn parse(request: &Value) -> Result<Call, Error> {
    let Value::Object(map) = request else {
        return Err(bad("a call is a JSON object with a `method`"));
    };
    let Some(Value::String(method)) = map.get("method") else {
        return Err(bad("a call needs a string `method`, e.g. \"document.get\""));
    };
    // Dots and underlines are the two spellings a caller has seen: the CLI's
    // help writes dots, and a shell script that remembered the MCP tool names
    // writes underlines.
    let method = method.trim().to_ascii_lowercase().replace('_', ".");
    match method.as_str() {
        "status" | "app.status" => Ok(Call::Status),
        "help" | "app.help" => Ok(Call::Help),
        "document.list" => Ok(Call::DocumentList),
        "document.get" => Ok(Call::DocumentGet {
            which: which(request)?,
            range: range(request)?,
        }),
        "document.save" => Ok(Call::DocumentSave {
            which: which(request)?,
        }),
        "document.set" => {
            let text = match map.get("text") {
                Some(Value::String(text)) => text.clone(),
                Some(_) => return Err(bad("`text` must be a string")),
                None => return Err(bad("`document.set` needs a string `text`")),
            };
            Ok(Call::DocumentSet {
                which: which(request)?,
                text,
                range: range(request)?,
            })
        }
        "command.run" => {
            let name = match map.get("command") {
                Some(Value::String(name)) if !name.trim().is_empty() => name.clone(),
                Some(_) => return Err(bad("`command` must be a string")),
                None => return Err(bad("`command.run` needs a `command` name")),
            };
            let text = match map.get("return_text") {
                None | Some(Value::Null) => false,
                Some(Value::Bool(value)) => *value,
                Some(_) => return Err(bad("`return_text` must be true or false")),
            };
            let whole = match map.get("whole_document") {
                None | Some(Value::Null) => false,
                Some(Value::Bool(value)) => *value,
                Some(_) => return Err(bad("`whole_document` must be true or false")),
            };
            let range = range(request)?;
            if whole && range.is_some() {
                return Err(bad(
                    "send `whole_document` or `range`, not both: they choose the same block",
                ));
            }
            Ok(Call::Command {
                name,
                text,
                range,
                whole,
            })
        }
        other => Err(Error {
            code: UNKNOWN_METHOD,
            message: format!("no method called {other:?}; `help` lists them"),
        }),
    }
}

/// What `which` each call means: a path, or the active tab.
fn which(request: &Value) -> Result<Which, Error> {
    match (request.get("path"), request.get("active")) {
        (Some(Value::String(path)), _) if !path.trim().is_empty() => {
            Ok(Which::Path(path.clone()))
        }
        (Some(Value::String(_)), _) => Err(bad("`path` must not be empty")),
        (Some(_), _) => Err(bad("`path` must be a string")),
        (None, Some(Value::Bool(true))) | (None, None) => Ok(Which::Active),
        (None, Some(Value::Bool(false))) => Err(bad(
            "`active: false` names no document: give a `path` instead",
        )),
        (None, Some(_)) => Err(bad("`active` must be true or false")),
    }
}

/// A `[start, end]` pair of character offsets, in either order.
fn range(request: &Value) -> Result<Option<(usize, usize)>, Error> {
    match request.get("range") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(values)) if values.len() == 2 => {
            let mut ends = [0usize; 2];
            for (index, value) in values.iter().enumerate() {
                let Some(number) = value.as_u64() else {
                    return Err(bad("`range` needs two character offsets"));
                };
                ends[index] = number as usize;
            }
            // A selection dragged backwards is still a selection.
            let (start, end) = (ends[0].min(ends[1]), ends[0].max(ends[1]));
            if start == end {
                return Err(bad("`range` is empty: leave it out to mean the whole document"));
            }
            Ok(Some((start, end)))
        }
        Some(_) => Err(bad("`range` must be [start, end]")),
    }
}

/// One open document, as `document.list` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentInfo {
    /// The file, or `None` for a buffer that has never been saved.
    pub path: Option<String>,
    /// Whether it has unsaved changes.
    pub modified: bool,
    /// The language highlighting it.
    pub language: String,
    /// Whether it is the tab with the caret in it.
    pub active: bool,
}

/// One document's text and caret, as `document.get` returns them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentText {
    /// The file, or `None` for an untitled buffer.
    pub path: Option<String>,
    /// The text, whole or the requested range of it.
    pub text: String,
    /// The caret, as a character offset into the whole document.
    pub caret: usize,
    /// The selection, as character offsets, if there is one.
    pub selection: Option<(usize, usize)>,
    /// `LF`, `CRLF` or `CR`.
    pub eol: &'static str,
    /// Whether it has unsaved changes.
    pub modified: bool,
    /// How many characters the document has, so a caller reading a range knows
    /// where the end is.
    pub chars: usize,
}

/// The answer `app.status` gives.
pub fn status(version: &str, documents: usize, active: usize) -> Value {
    json!({
        "ok": true,
        "version": version,
        "documents": documents,
        "active": active,
    })
}

/// The answer `document.list` gives.
pub fn documents(documents: &[DocumentInfo]) -> Value {
    let list: Vec<Value> = documents
        .iter()
        .map(|document| {
            json!({
                "path": document.path,
                "modified": document.modified,
                "language": document.language,
                "active": document.active,
            })
        })
        .collect();
    json!({"ok": true, "count": list.len(), "documents": list})
}

/// The answer `document.get` gives.
pub fn document(document: &DocumentText) -> Value {
    let (start, end) = match document.selection {
        Some((start, end)) if start != end => (json!(start), json!(end)),
        _ => (Value::Null, Value::Null),
    };
    json!({
        "ok": true,
        "path": document.path,
        "text": document.text,
        "chars": document.chars,
        "caret": document.caret,
        "selection_start": start,
        "selection_end": end,
        "eol": document.eol,
        "modified": document.modified,
    })
}

/// The answer `document.set`, `document.save` and `command.run` give: `ok`, and
/// whatever the particular call has to report.
pub fn done(fields: &[(&str, Value)]) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("ok".to_string(), Value::Bool(true));
    for (key, value) in fields {
        map.insert((*key).to_string(), value.clone());
    }
    Value::Object(map)
}

/// The method list `help` returns, as prose.
///
/// Prose rather than JSON: this is what a person reads after typing
/// `npp --call '{"method":"help"}'` into a shell they have half forgotten.
pub fn help() -> String {
    [
        "npp --call '<json>' asks the running window to do something and prints its answer.",
        "",
        "  {\"method\": \"help\"}                                   this text",
        "  {\"method\": \"app.status\"}                             version and how many documents are open",
        "  {\"method\": \"document.list\"}                          every open document, in tab order",
        "  {\"method\": \"document.get\"}                           the active document's text and caret",
        "  {\"method\": \"document.get\", \"path\": \"/tmp/a.txt\"}    one named document",
        "  {\"method\": \"document.get\", \"range\": [0, 80]}         just the first 80 characters",
        "  {\"method\": \"document.set\", \"text\": \"hello\\n\"}       replace the active document",
        "  {\"method\": \"document.set\", \"text\": \"X\", \"range\": [2, 5]}   replace a span of it",
        "  {\"method\": \"document.save\"}                           write the active document to its file",
        "  {\"method\": \"command.run\", \"command\": \"sort_asc\"}     run any command, as the menu would",
        "  {\"method\": \"command.run\", \"command\": \"upper_case\", \"return_text\": true}",
        "  {\"method\": \"command.run\", \"command\": \"sort_asc\", \"whole_document\": true}",
        "",
        "A command runs the way the menu does: over the selection, or over the line",
        "the caret is on. `whole_document` or a `range` says which block instead.",
        "",
        "A command runs through the same table the menu and the MCP server reach, so",
        "`list_commands` over MCP and `command.run` here agree about what exists.",
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_call_needs_nothing_but_its_method() {
        assert_eq!(parse(&json!({"method": "app.status"})), Ok(Call::Status));
        assert_eq!(parse(&json!({"method": "status"})), Ok(Call::Status));
    }

    #[test]
    fn the_method_is_read_the_way_a_person_would_write_it() {
        assert_eq!(parse(&json!({"method": " Status "})), Ok(Call::Status));
        assert_eq!(
            parse(&json!({"method": "document_list"})),
            Ok(Call::DocumentList)
        );
        assert_eq!(
            parse(&json!({"method": "Document.Get"})),
            Ok(Call::DocumentGet {
                which: Which::Active,
                range: None
            })
        );
    }

    #[test]
    fn a_document_is_named_by_path_or_left_active() {
        assert_eq!(
            parse(&json!({"method": "document.get", "path": "/tmp/a.txt"})),
            Ok(Call::DocumentGet {
                which: Which::Path("/tmp/a.txt".to_string()),
                range: None
            })
        );
        assert_eq!(
            parse(&json!({"method": "document.get", "active": true})),
            Ok(Call::DocumentGet {
                which: Which::Active,
                range: None
            })
        );
        // `active: false` names nothing, and guessing would edit the wrong file.
        assert!(parse(&json!({"method": "document.get", "active": false}))
            .unwrap_err()
            .message
            .contains("names no document"));
    }

    #[test]
    fn a_range_is_two_offsets_in_either_order() {
        assert_eq!(
            parse(&json!({"method": "document.get", "range": [8, 2]})),
            Ok(Call::DocumentGet {
                which: Which::Active,
                range: Some((2, 8))
            })
        );
        assert!(parse(&json!({"method": "document.get", "range": [3, 3]}))
            .unwrap_err()
            .message
            .contains("empty"));
        assert!(parse(&json!({"method": "document.get", "range": [1]})).is_err());
        assert!(parse(&json!({"method": "document.get", "range": "0-4"})).is_err());
    }

    #[test]
    fn setting_a_document_needs_text_and_takes_a_range() {
        assert_eq!(
            parse(&json!({"method": "document.set", "text": "hello\n"})),
            Ok(Call::DocumentSet {
                which: Which::Active,
                text: "hello\n".to_string(),
                range: None
            })
        );
        assert_eq!(
            parse(&json!({"method": "document.set", "text": "X", "range": [2, 5]})),
            Ok(Call::DocumentSet {
                which: Which::Active,
                text: "X".to_string(),
                range: Some((2, 5))
            })
        );
        assert!(parse(&json!({"method": "document.set"}))
            .unwrap_err()
            .message
            .contains("needs a string `text`"));
        // An empty replacement is a deletion, so it is allowed; a missing one is
        // not the same thing.
        assert!(parse(&json!({"method": "document.set", "text": ""})).is_ok());
    }

    #[test]
    fn a_command_needs_a_name_and_an_optional_return() {
        assert_eq!(
            parse(&json!({"method": "command.run", "command": "sort_asc"})),
            Ok(Call::Command {
                name: "sort_asc".to_string(),
                text: false,
                range: None,
                whole: false
            })
        );
        assert_eq!(
            parse(&json!({"method": "command.run", "command": "sort_asc", "return_text": true})),
            Ok(Call::Command {
                name: "sort_asc".to_string(),
                text: true,
                range: None,
                whole: false
            })
        );
        assert!(parse(&json!({"method": "command.run"}))
            .unwrap_err()
            .message
            .contains("needs a `command`"));
        assert!(parse(&json!({"method": "command.run", "command": " "})).is_err());
    }

    #[test]
    fn a_command_can_be_given_the_block_it_runs_over() {
        assert_eq!(
            parse(&json!({"method": "command.run", "command": "sort_asc", "whole_document": true})),
            Ok(Call::Command {
                name: "sort_asc".to_string(),
                text: false,
                range: None,
                whole: true
            })
        );
        assert_eq!(
            parse(&json!({"method": "command.run", "command": "upper_case", "range": [2, 6]})),
            Ok(Call::Command {
                name: "upper_case".to_string(),
                text: false,
                range: Some((2, 6)),
                whole: false
            })
        );
        // Both would be a guess about which one wins.
        assert!(parse(&json!(
            {"method": "command.run", "command": "sort_asc", "whole_document": true, "range": [0, 4]}
        ))
        .unwrap_err()
        .message
        .contains("not both"));
    }

    #[test]
    fn an_unknown_method_is_told_about_help() {
        let error = parse(&json!({"method": "document.delete"})).unwrap_err();
        assert_eq!(error.code, UNKNOWN_METHOD);
        assert!(error.message.contains("help"), "{}", error.message);
    }

    #[test]
    fn a_request_that_is_not_a_call_says_so() {
        assert_eq!(parse(&json!([1, 2, 3])).unwrap_err().code, BAD_REQUEST);
        assert_eq!(parse(&json!("sort_asc")).unwrap_err().code, BAD_REQUEST);
        let wording = parse(&json!({})).unwrap_err().message;
        assert!(wording.contains("`method`"), "{wording}");
    }

    #[test]
    fn the_help_text_names_every_method_that_exists() {
        let help = help();
        for method in [
            "app.status",
            "document.list",
            "document.get",
            "document.set",
            "document.save",
            "command.run",
        ] {
            assert!(help.contains(method), "{method} is missing from the help");
            // And the method really is one: the help cannot document a lie.
            let request = json!({"method": method, "text": "x", "command": "sort_asc"});
            assert!(parse(&request).is_ok(), "{method} does not parse");
        }
    }

    #[test]
    fn a_document_list_carries_the_four_facts_a_caller_asks_for() {
        let value = documents(&[
            DocumentInfo {
                path: Some("/tmp/a.txt".to_string()),
                modified: true,
                language: "Rust".to_string(),
                active: true,
            },
            DocumentInfo {
                path: None,
                modified: false,
                language: "Plain Text".to_string(),
                active: false,
            },
        ]);
        assert_eq!(value["ok"], true);
        assert_eq!(value["count"], 2);
        assert_eq!(value["documents"][0]["path"], "/tmp/a.txt");
        assert_eq!(value["documents"][0]["modified"], true);
        assert_eq!(value["documents"][0]["active"], true);
        assert_eq!(value["documents"][1]["path"], Value::Null);
    }

    #[test]
    fn a_document_answer_reports_an_absent_selection_as_null() {
        let value = document(&DocumentText {
            path: Some("/tmp/a.txt".to_string()),
            text: "abc".to_string(),
            caret: 1,
            selection: None,
            eol: "LF",
            modified: false,
            chars: 3,
        });
        assert_eq!(value["caret"], 1);
        assert_eq!(value["chars"], 3);
        assert_eq!(value["selection_start"], Value::Null);
        let value = document(&DocumentText {
            path: None,
            text: "abc".to_string(),
            caret: 3,
            selection: Some((1, 3)),
            eol: "LF",
            modified: false,
            chars: 3,
        });
        assert_eq!(value["selection_start"], 1);
        assert_eq!(value["selection_end"], 3);
    }

    #[test]
    fn an_error_answer_carries_the_code_and_the_message() {
        let value = Error {
            code: UNKNOWN_DOCUMENT,
            message: "no tab has /tmp/zzz open".to_string(),
        }
        .value();
        assert_eq!(value["error"]["code"], "unknown_document");
        assert_eq!(value["error"]["message"], "no tab has /tmp/zzz open");
        assert_eq!(value["ok"], Value::Null);
    }
}
