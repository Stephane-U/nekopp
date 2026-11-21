//! The Model Context Protocol surface: dispatch, tools, resources.
//!
//! The transport is newline-delimited JSON-RPC 2.0 over stdio, which is what
//! MCP calls a stdio server: one JSON object per line in, one JSON object per
//! line out, and nothing else on stdout. Every diagnostic goes to stderr,
//! because a stray `println!` on stdout is not a warning to a client, it is a
//! parse error that breaks the session.
//!
//! The protocol is implemented by hand rather than through an SDK. The methods
//! this server needs are eight, the SDK would be a dependency in a workspace
//! that builds offline, and the parts that are easy to get wrong -- notifications
//! must not be answered, errors must be JSON-RPC error objects, a tool failure is
//! a successful call with `isError` -- are exactly the parts that are written and
//! tested here.

use std::io::{BufRead, Write};

use serde_json::{json, Map, Value};

use crate::commands::Inventory;
use crate::doc::Document;
use crate::search;

/// The MCP revision this server speaks.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// The name a client shows for this server.
pub const SERVER_NAME: &str = "npp-mcp";

/// The server's version, taken from the crate so it cannot drift.
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// JSON-RPC: the line was not JSON.
const PARSE_ERROR: i64 = -32700;
/// JSON-RPC: the line was JSON, but not a request.
const INVALID_REQUEST: i64 = -32600;
/// JSON-RPC: no such method.
const METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC: the method exists, the arguments do not fit it.
const INVALID_PARAMS: i64 = -32602;
/// MCP: the URI names no resource this server publishes.
const RESOURCE_NOT_FOUND: i64 = -32002;

/// The URI of the index resource.
pub const INDEX_URI: &str = "nekopp://features";

/// One MCP server over one parsed reference.
pub struct Server {
    doc: Document,
    inventory: Inventory,
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

impl Server {
    /// Build a server over the embedded reference.
    pub fn new() -> Self {
        let doc = Document::embedded();
        let inventory = Inventory::from_document(&doc);
        Self { doc, inventory }
    }

    /// The reference, for a caller that wants to inspect what is served.
    pub fn document(&self) -> &Document {
        &self.doc
    }

    /// The command inventory, for the same reason.
    pub fn inventory(&self) -> &Inventory {
        &self.inventory
    }

    /// Handle one line of stdin: a request, a notification, or noise.
    ///
    /// Returns the response to write, or `None` for a notification and for a
    /// blank line. A notification is never answered, not even an unknown one:
    /// the client is not listening for an answer, and a server that sends one
    /// anyway has broken the framing for every later message.
    pub fn handle_line(&self, line: &str) -> Option<Value> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(request) => self.handle(&request),
            Err(error) => Some(error_response(
                Value::Null,
                PARSE_ERROR,
                &format!("invalid JSON: {error}"),
            )),
        }
    }

    /// Handle one parsed message.
    pub fn handle(&self, request: &Value) -> Option<Value> {
        let Some(object) = request.as_object() else {
            return Some(error_response(
                Value::Null,
                INVALID_REQUEST,
                "a request must be a JSON object",
            ));
        };

        let id = object.get("id").cloned();
        let method = match object.get("method") {
            Some(Value::String(method)) => method.clone(),
            _ => {
                return Some(error_response(
                    id.unwrap_or(Value::Null),
                    INVALID_REQUEST,
                    "a request needs a string `method`",
                ))
            }
        };

        if object.get("jsonrpc") != Some(&Value::String("2.0".to_string())) {
            return Some(error_response(
                id.unwrap_or(Value::Null),
                INVALID_REQUEST,
                "`jsonrpc` must be \"2.0\"",
            ));
        }

        // Absent id is a notification; a null id is a request whose id happens
        // to be null, and it gets an answer.
        let id = id?;

        let params = object.get("params").cloned().unwrap_or(Value::Null);
        Some(match self.dispatch(&method, &params) {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err((code, message)) => error_response(id, code, &message),
        })
    }

    /// Run the stdio loop until the input ends.
    ///
    /// The loop is in the library so that a test can drive a whole session
    /// through it; `main` is then three lines and has nothing to get wrong.
    pub fn run<R: BufRead, W: Write>(&self, mut input: R, mut output: W) -> std::io::Result<()> {
        let mut line = Vec::new();
        loop {
            line.clear();
            if input.read_until(b'\n', &mut line)? == 0 {
                return Ok(());
            }
            // Lossy rather than an error: a client that sends a broken byte
            // gets a parse error it can see, instead of a session that ends.
            let text = String::from_utf8_lossy(&line);
            if let Some(response) = self.handle_line(&text) {
                writeln!(output, "{response}")?;
                output.flush()?;
            }
        }
    }

    /// The one `match` that is the protocol.
    fn dispatch(&self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => self.initialize(params),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools() })),
            "tools/call" => self.call_tool(params),
            "resources/list" => Ok(json!({ "resources": self.resources() })),
            "resources/read" => self.read_resource(params),
            "prompts/list" => Ok(json!({ "prompts": [] })),
            other => Err((
                METHOD_NOT_FOUND,
                format!("method not found: {other}"),
            )),
        }
    }

    fn initialize(&self, params: &Value) -> Result<Value, (i64, String)> {
        // Nothing in the client's params is used, but params that are not an
        // object are a client bug worth reporting rather than ignoring.
        if object_params(params).is_err() {
            return Err((INVALID_PARAMS, "params must be an object".to_string()));
        }
        Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {"tools": {}, "resources": {}},
            "serverInfo": {"name": SERVER_NAME, "version": SERVER_VERSION},
            "instructions": concat!(
                "The Neko++ feature reference. `search_features` finds sections, ",
                "`get_section` returns one as markdown, `list_sections` is the table ",
                "of contents, `list_shortcuts` is the whole default keymap, and ",
                "`get_command` answers about a single command. Every section is also ",
                "a resource: nekopp://features/<slug>, indexed by nekopp://features."
            ),
        }))
    }

    fn call_tool(&self, params: &Value) -> Result<Value, (i64, String)> {
        let Some(params) = object_params(params)? else {
            return Err((INVALID_PARAMS, "tools/call needs `name`".to_string()));
        };
        let name = match params.get("name") {
            Some(Value::String(name)) => name.clone(),
            _ => return Err((INVALID_PARAMS, "tools/call needs a string `name`".to_string())),
        };
        let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
        match name.as_str() {
            "search_features" => self.search_features(&arguments),
            "get_section" => self.get_section(&arguments),
            "list_sections" => Ok(tool_text(self.table_of_contents())),
            "list_shortcuts" => Ok(tool_text(self.shortcut_table())),
            "get_command" => self.get_command(&arguments),
            other => Err((INVALID_PARAMS, format!("unknown tool: {other}"))),
        }
    }

    fn search_features(&self, arguments: &Value) -> Result<Value, (i64, String)> {
        let query = required_string(arguments, "query")?;
        let limit = optional_limit(arguments, "limit", 10, 50)?;
        let hits = search::search(&self.doc, &query, limit);
        if hits.is_empty() {
            return Ok(tool_text(format!(
                "No section mentions \"{query}\". `list_sections` shows every heading."
            )));
        }

        let mut text = format!(
            "{} section{} match \"{}\":\n",
            hits.len(),
            if hits.len() == 1 { "" } else { "s" },
            query
        );
        for hit in &hits {
            let section = &self.doc.sections[hit.index];
            text.push_str(&format!(
                "\n{} {} -- {}\n  {}\n",
                "#".repeat(section.level as usize),
                section.heading,
                section_uri(&section.slug),
                hit.snippet
            ));
        }
        Ok(tool_text(text))
    }

    fn get_section(&self, arguments: &Value) -> Result<Value, (i64, String)> {
        let key = required_string(arguments, "section")?;
        match self.doc.section(&key) {
            Some(section) => Ok(tool_text(format!(
                "{}\nResource: {}\n",
                section.markdown(),
                section_uri(&section.slug)
            ))),
            None => {
                let suggestions = search::search(&self.doc, &key, 3);
                let mut text = format!("No section is called \"{key}\".\n");
                if suggestions.is_empty() {
                    text.push_str("`list_sections` shows every heading.\n");
                } else {
                    text.push_str("Closest sections:\n");
                    for hit in &suggestions {
                        let section = &self.doc.sections[hit.index];
                        text.push_str(&format!(
                            "- {} ({})\n",
                            section.heading,
                            section_uri(&section.slug)
                        ));
                    }
                }
                Ok(tool_error(text))
            }
        }
    }

    fn get_command(&self, arguments: &Value) -> Result<Value, (i64, String)> {
        let name = required_string(arguments, "name")?;
        match self.inventory.lookup(&name) {
            Some(command) => {
                let mut text = format!("{}\n", command.label);
                text.push_str(&format!(
                    "shortcut: {}\n",
                    command.shortcut.as_deref().unwrap_or("none bound by default")
                ));
                text.push_str(&format!(
                    "action:   {}\n",
                    command.action.as_deref().unwrap_or("none -- the row is not in the keymap table")
                ));
                text.push_str(&format!(
                    "menu:     {}\n",
                    command.path.as_deref().unwrap_or("no menu row")
                ));
                if let Some(section) = self.command_section(command.path.as_deref()) {
                    text.push_str(&format!("resource: {}\n", section_uri(&section.slug)));
                }
                Ok(tool_text(text))
            }
            None => Ok(tool_error(format!(
                "No command is called \"{name}\". `list_shortcuts` has the bound ones."
            ))),
        }
    }

    /// The section that documents a command, from the menu it lives in.
    fn command_section(&self, path: Option<&str>) -> Option<&crate::doc::Section> {
        let menu = path?.split(" > ").next()?;
        self.doc.section(menu)
    }

    /// Every section as a table of contents, chapters first.
    pub fn table_of_contents(&self) -> String {
        let chapters = self.doc.sections.iter().filter(|s| s.level == 2).count();
        let subsections = self.doc.sections.len() - chapters;
        let mut text = format!(
            "{} -- {chapters} chapters, {subsections} subsections\n",
            self.doc.title
        );
        for section in &self.doc.sections {
            let indent = if section.level > 2 { "  " } else { "" };
            text.push_str(&format!(
                "{indent}{} {} -- {}\n",
                "#".repeat(section.level as usize),
                section.heading,
                section_uri(&section.slug)
            ));
        }
        text
    }

    /// The whole default keymap as a markdown table.
    pub fn shortcut_table(&self) -> String {
        let mut text = format!(
            "{} default key binding{}:\n\n| Shortcut | Command | Action |\n|---|---|---|\n",
            self.inventory.shortcuts().len(),
            if self.inventory.shortcuts().len() == 1 {
                ""
            } else {
                "s"
            }
        );
        for shortcut in self.inventory.shortcuts() {
            text.push_str(&format!(
                "| `{}` | {} | `{}` |\n",
                shortcut.keys, shortcut.command, shortcut.action
            ));
        }
        text
    }

    /// The index plus one resource per section.
    fn resources(&self) -> Vec<Value> {
        let mut resources = vec![json!({
            "uri": INDEX_URI,
            "name": self.doc.title,
            "description": "Every section of the Neko++ feature reference, with its URI.",
            "mimeType": "text/markdown",
        })];
        for section in &self.doc.sections {
            resources.push(json!({
                "uri": section_uri(&section.slug),
                "name": section.heading,
                "description": section.summary(),
                "mimeType": "text/markdown",
            }));
        }
        resources
    }

    fn read_resource(&self, params: &Value) -> Result<Value, (i64, String)> {
        let Some(params) = object_params(params)? else {
            return Err((INVALID_PARAMS, "resources/read needs `uri`".to_string()));
        };
        let uri = match params.get("uri") {
            Some(Value::String(uri)) => uri.clone(),
            _ => return Err((INVALID_PARAMS, "resources/read needs a string `uri`".to_string())),
        };

        let text = if uri == INDEX_URI {
            self.table_of_contents()
        } else if let Some(slug) = uri.strip_prefix("nekopp://features/") {
            match self.doc.sections.iter().find(|section| section.slug == slug) {
                Some(section) => section.markdown(),
                None => {
                    return Err((
                        RESOURCE_NOT_FOUND,
                        format!("resource not found: {uri}"),
                    ))
                }
            }
        } else {
            return Err((RESOURCE_NOT_FOUND, format!("resource not found: {uri}")));
        };

        Ok(json!({
            "contents": [{
                "uri": uri,
                "mimeType": "text/markdown",
                "text": text,
            }],
        }))
    }
}

/// The URI of one section.
pub fn section_uri(slug: &str) -> String {
    format!("{INDEX_URI}/{slug}")
}

/// A successful tool call: one text block and no error flag.
fn tool_text(text: String) -> Value {
    json!({
        "content": [{"type": "text", "text": text}],
        "isError": false,
    })
}

/// A failed tool call.
///
/// The protocol's own rule: the call succeeded, the tool did not, and the model
/// gets to read why. Only a request the server cannot understand is an error
/// object.
fn tool_error(text: String) -> Value {
    json!({
        "content": [{"type": "text", "text": text}],
        "isError": true,
    })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// `params` as an object, absent or `null` counting as empty.
fn object_params(params: &Value) -> Result<Option<&Map<String, Value>>, (i64, String)> {
    match params {
        Value::Null => Ok(None),
        Value::Object(map) => Ok(Some(map)),
        _ => Err((INVALID_PARAMS, "params must be an object".to_string())),
    }
}

fn required_string(arguments: &Value, name: &str) -> Result<String, (i64, String)> {
    match arguments.get(name) {
        Some(Value::String(text)) if !text.trim().is_empty() => Ok(text.clone()),
        Some(Value::String(_)) => Err((INVALID_PARAMS, format!("`{name}` must not be empty"))),
        Some(_) => Err((INVALID_PARAMS, format!("`{name}` must be a string"))),
        None => Err((
            INVALID_PARAMS,
            format!("missing required argument `{name}`"),
        )),
    }
}

fn optional_limit(arguments: &Value, name: &str, default: usize, max: usize) -> Result<usize, (i64, String)> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Number(number)) => match number.as_u64() {
            Some(value) if value >= 1 => Ok((value as usize).min(max)),
            _ => Err((INVALID_PARAMS, format!("`{name}` must be 1 or more"))),
        },
        Some(_) => Err((INVALID_PARAMS, format!("`{name}` must be a number"))),
    }
}

/// The tool list.
///
/// The schemas are hand-written JSON Schema: the MCP client uses them to tell a
/// model what it may send, so they describe the arguments and nothing else.
fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "search_features",
            "description": "Search the Neko++ feature reference. Returns the matching sections with their headings, resource URIs, and a line of context, best match first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Words to look for, e.g. \"column editing\" or \"Ctrl+Shift+Q\"."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 50,
                        "description": "How many sections to return. Defaults to 10."
                    }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "get_section",
            "description": "Return one section of the Neko++ feature reference as markdown, by heading (\"Find and Replace\") or slug (\"find-and-replace\").",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "section": {
                        "type": "string",
                        "description": "A section heading or slug."
                    }
                },
                "required": ["section"]
            }
        }),
        json!({
            "name": "list_sections",
            "description": "List every section of the Neko++ feature reference, chapter by chapter, with the resource URI of each.",
            "inputSchema": {"type": "object", "properties": {}}
        }),
        json!({
            "name": "list_shortcuts",
            "description": "List every default keyboard shortcut in Neko++, with the command it runs and the action name the keymap stores.",
            "inputSchema": {"type": "object", "properties": {}}
        }),
        json!({
            "name": "get_command",
            "description": "Look up one command by menu label (\"Print Now\") or action name (\"print_now\"): its default shortcut, its action, and the menu path it lives at.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "A menu label or an action name."
                    }
                },
                "required": ["name"]
            }
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> Server {
        Server::new()
    }

    fn call(request: Value) -> Value {
        server()
            .handle(&request)
            .expect("a request with an id is answered")
    }

    fn result_of(response: &Value) -> &Value {
        response
            .get("result")
            .unwrap_or_else(|| panic!("expected a result, got {response}"))
    }

    fn text_of(response: &Value) -> &str {
        result_of(response)["content"][0]["text"]
            .as_str()
            .expect("a text block")
    }

    #[test]
    fn initialize_names_the_protocol_and_the_server() {
        let response = call(json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": PROTOCOL_VERSION, "clientInfo": {"name": "test"}}
        }));
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 1);
        let result = result_of(&response);
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["serverInfo"]["name"], "npp-mcp");
        assert_eq!(result["serverInfo"]["version"], SERVER_VERSION);
        assert_eq!(result["capabilities"]["tools"], json!({}));
        assert_eq!(result["capabilities"]["resources"], json!({}));
        assert!(result["instructions"].as_str().unwrap().contains("search_features"));
    }

    #[test]
    fn a_notification_is_never_answered() {
        let server = server();
        for method in ["notifications/initialized", "notifications/cancelled", "no/such"] {
            assert!(server
                .handle(&json!({"jsonrpc": "2.0", "method": method, "params": {}}))
                .is_none());
        }
        // A request that only looks like a notification still gets an answer
        // when it carries an id, even a null one.
        let answered = server.handle(&json!({"jsonrpc": "2.0", "id": null, "method": "ping"}));
        assert!(answered.is_some());
    }

    #[test]
    fn an_unknown_method_is_a_method_not_found_error() {
        let response = call(json!({"jsonrpc": "2.0", "id": 7, "method": "tools/explode"}));
        assert_eq!(response["id"], 7);
        assert!(response.get("result").is_none());
        assert_eq!(response["error"]["code"], -32601);
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("tools/explode"));
    }

    #[test]
    fn a_broken_line_is_a_parse_error_and_a_broken_request_is_not() {
        let server = server();
        let response = server.handle_line("{not json").expect("an answer");
        assert_eq!(response["error"]["code"], -32700);
        assert_eq!(response["id"], Value::Null);

        let response = server.handle_line("[1, 2, 3]").expect("an answer");
        assert_eq!(response["error"]["code"], -32600);

        let response = server
            .handle_line(r#"{"id": 3, "method": "ping"}"#)
            .expect("an answer");
        assert_eq!(response["error"]["code"], -32600);

        assert!(server.handle_line("   ").is_none());
    }

    #[test]
    fn ping_answers_with_an_empty_result() {
        let response = call(json!({"jsonrpc": "2.0", "id": 2, "method": "ping"}));
        assert_eq!(response["result"], json!({}));
    }

    #[test]
    fn tools_list_describes_every_tool_with_a_schema() {
        let response = call(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}));
        let tools = result_of(&response)["tools"].as_array().unwrap();
        let names: Vec<_> = tools.iter().map(|tool| tool["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            vec![
                "search_features",
                "get_section",
                "list_sections",
                "list_shortcuts",
                "get_command"
            ]
        );
        for tool in tools {
            assert!(!tool["description"].as_str().unwrap().is_empty());
            assert_eq!(tool["inputSchema"]["type"], "object");
        }
        assert_eq!(tools[0]["inputSchema"]["required"], json!(["query"]));
        assert_eq!(tools[2]["inputSchema"]["properties"], json!({}));
    }

    #[test]
    fn a_search_returns_content_blocks_with_headings_and_uris() {
        let response = call(json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": {"name": "search_features", "arguments": {"query": "bookmark", "limit": 3}}
        }));
        assert_eq!(result_of(&response)["isError"], false);
        assert_eq!(result_of(&response)["content"][0]["type"], "text");
        let text = text_of(&response);
        assert!(text.contains("## Bookmarks -- nekopp://features/bookmarks"), "{text}");
        assert!(text.contains("sections match \"bookmark\""), "{text}");
        assert_eq!(text.lines().filter(|line| line.starts_with('#')).count(), 3);
    }

    #[test]
    fn a_search_that_matches_nothing_says_so_without_an_error() {
        let response = call(json!({
            "jsonrpc": "2.0", "id": 5, "method": "tools/call",
            "params": {"name": "search_features", "arguments": {"query": "zzzz"}}
        }));
        assert_eq!(result_of(&response)["isError"], false);
        assert!(text_of(&response).contains("No section mentions \"zzzz\""));
    }

    #[test]
    fn get_section_returns_the_markdown_and_the_uri() {
        let response = call(json!({
            "jsonrpc": "2.0", "id": 6, "method": "tools/call",
            "params": {"name": "get_section", "arguments": {"section": "Encodings and Line Endings"}}
        }));
        let text = text_of(&response);
        assert!(text.starts_with("## Encodings and Line Endings\n\n"));
        assert!(text.contains("Resource: nekopp://features/encodings-and-line-endings"));
        // The same section by slug is the same answer.
        let by_slug = call(json!({
            "jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": {"name": "get_section", "arguments": {"section": "encodings-and-line-endings"}}
        }));
        assert_eq!(text_of(&by_slug), text);
    }

    #[test]
    fn a_missing_section_is_a_tool_error_with_suggestions() {
        let response = call(json!({
            "jsonrpc": "2.0", "id": 8, "method": "tools/call",
            "params": {"name": "get_section", "arguments": {"section": "foldin"}}
        }));
        assert_eq!(result_of(&response)["isError"], true);
        let text = text_of(&response);
        assert!(text.contains("No section is called \"foldin\""), "{text}");
        assert!(text.contains("Folding"), "{text}");
    }

    #[test]
    fn list_sections_is_the_table_of_contents() {
        let response = call(json!({
            "jsonrpc": "2.0", "id": 9, "method": "tools/call",
            "params": {"name": "list_sections", "arguments": {}}
        }));
        let text = text_of(&response);
        assert!(text.starts_with("Neko++ Feature Reference -- 27 chapters, 75 subsections"));
        assert!(text.contains("## The Menus -- nekopp://features/the-menus"));
        assert!(text.contains("  ### File -- nekopp://features/file"));
        assert_eq!(text.lines().count(), 103);
    }

    #[test]
    fn list_shortcuts_is_the_whole_keymap() {
        let response = call(json!({
            "jsonrpc": "2.0", "id": 10, "method": "tools/call",
            "params": {"name": "list_shortcuts", "arguments": {}}
        }));
        let text = text_of(&response);
        assert!(text.starts_with("73 default key bindings:"));
        assert!(text.contains("| `Ctrl+Shift+Q` | Exit | `exit` |"));
        assert_eq!(text.lines().filter(|line| line.starts_with("| `")).count(), 73);
    }

    #[test]
    fn get_command_answers_by_label_and_by_action() {
        let by_label = call(json!({
            "jsonrpc": "2.0", "id": 11, "method": "tools/call",
            "params": {"name": "get_command", "arguments": {"name": "Print Now"}}
        }));
        let text = text_of(&by_label);
        assert!(text.starts_with("Print Now\n"));
        assert!(text.contains("shortcut: Ctrl+Alt+Shift+P"));
        assert!(text.contains("action:   print_now"));
        assert!(text.contains("menu:     File > Print Now"));
        assert!(text.contains("resource: nekopp://features/file"));

        let by_action = call(json!({
            "jsonrpc": "2.0", "id": 12, "method": "tools/call",
            "params": {"name": "get_command", "arguments": {"name": "print_now"}}
        }));
        assert_eq!(text_of(&by_action), text);
    }

    #[test]
    fn a_missing_command_is_a_tool_error() {
        let response = call(json!({
            "jsonrpc": "2.0", "id": 13, "method": "tools/call",
            "params": {"name": "get_command", "arguments": {"name": "Format Everything"}}
        }));
        assert_eq!(result_of(&response)["isError"], true);
        assert!(text_of(&response).contains("No command is called"));
    }

    #[test]
    fn a_bad_tool_call_is_an_error_object() {
        let server = server();
        for params in [
            json!({}),
            json!({"name": "search_features"}),
            json!({"name": "search_features", "arguments": {"query": 42}}),
            json!({"name": "search_features", "arguments": {"query": "find", "limit": 0}}),
            json!({"name": "search_features", "arguments": {"query": "find", "limit": "many"}}),
            json!({"name": "get_section", "arguments": {"section": ""}}),
            json!({"name": "not_a_tool", "arguments": {}}),
        ] {
            let response = server
                .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params}))
                .expect("an answer");
            assert_eq!(response["error"]["code"], -32602, "{params}");
        }

        // params that are not an object at all
        let response = server
            .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": "nope"}))
            .unwrap();
        assert_eq!(response["error"]["code"], -32602);
    }

    #[test]
    fn the_tool_limit_is_honoured_and_bounded() {
        let response = call(json!({
            "jsonrpc": "2.0", "id": 14, "method": "tools/call",
            "params": {"name": "search_features", "arguments": {"query": "line", "limit": 2}}
        }));
        assert_eq!(
            text_of(&response)
                .lines()
                .filter(|line| line.starts_with('#'))
                .count(),
            2
        );
    }

    #[test]
    fn resources_list_the_index_and_every_section() {
        let response = call(json!({"jsonrpc": "2.0", "id": 15, "method": "resources/list"}));
        let resources = result_of(&response)["resources"].as_array().unwrap();
        assert_eq!(resources.len(), 103);
        assert_eq!(resources[0]["uri"], INDEX_URI);
        assert_eq!(resources[0]["mimeType"], "text/markdown");
        assert!(resources
            .iter()
            .any(|resource| resource["uri"] == "nekopp://features/find-and-replace"));
        for resource in resources {
            assert!(!resource["name"].as_str().unwrap().is_empty());
            assert!(!resource["description"].as_str().unwrap().is_empty());
        }
    }

    #[test]
    fn a_resource_read_returns_markdown_for_that_uri() {
        let response = call(json!({
            "jsonrpc": "2.0", "id": 16, "method": "resources/read",
            "params": {"uri": "nekopp://features/folding"}
        }));
        let contents = result_of(&response)["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["uri"], "nekopp://features/folding");
        assert_eq!(contents[0]["mimeType"], "text/markdown");
        assert!(contents[0]["text"].as_str().unwrap().starts_with("## Folding\n\n"));
    }

    #[test]
    fn the_index_resource_lists_every_section() {
        let response = call(json!({
            "jsonrpc": "2.0", "id": 17, "method": "resources/read",
            "params": {"uri": INDEX_URI}
        }));
        let text = result_of(&response)["contents"][0]["text"].as_str().unwrap();
        let server = server();
        for section in &server.document().sections {
            assert!(text.contains(&section_uri(&section.slug)), "{}", section.slug);
        }
    }

    #[test]
    fn an_unknown_resource_is_a_resource_not_found_error() {
        let server = server();
        for uri in [
            "nekopp://features/no-such-section",
            "nekopp://features/",
            "nekopp://elsewhere/features",
            "file:///etc/passwd",
        ] {
            let response = server
                .handle(&json!({
                    "jsonrpc": "2.0", "id": 18, "method": "resources/read",
                    "params": {"uri": uri}
                }))
                .expect("an answer");
            assert_eq!(response["error"]["code"], -32002, "{uri}");
        }

        let response = server
            .handle(&json!({"jsonrpc": "2.0", "id": 19, "method": "resources/read", "params": {}}))
            .unwrap();
        assert_eq!(response["error"]["code"], -32602);
    }

    #[test]
    fn prompts_list_is_empty_rather_than_absent() {
        let response = call(json!({"jsonrpc": "2.0", "id": 20, "method": "prompts/list"}));
        assert_eq!(result_of(&response), &json!({"prompts": []}));
    }

    #[test]
    fn the_stdio_loop_answers_one_object_per_line_and_nothing_for_notifications() {
        let script = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n",
            "\n",
            "not json\n",
        );
        let mut out = Vec::new();
        server().run(script.as_bytes(), &mut out).expect("no io error");
        let lines: Vec<&str> = std::str::from_utf8(&out).unwrap().trim_end().split('\n').collect();
        assert_eq!(lines.len(), 3);
        let first: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["id"], 1);
        let second: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["result"], json!({}));
        let third: Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(third["error"]["code"], -32700);
    }
}
