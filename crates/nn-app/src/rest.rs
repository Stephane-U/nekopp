//! The REST face of the call API.
//!
//! `--call` speaks one JSON object in and one out over the session bus, and MCP
//! speaks JSON-RPC over a pipe. Neither is convenient from a language whose HTTP
//! client is already on the shelf or from a tool that only knows how to fetch a
//! URL, so this module adds the third spelling: the same call table, addressed by
//! URL, over HTTP on the loopback interface.
//!
//! One table, three transports. A route does not reimplement anything: it builds
//! the JSON request a `--call` user would have typed and hands it to
//! [`api::parse`], so a rule -- what a `range` may name, whether `whole_document`
//! and `range` can travel together, what an empty `path` means -- is enforced in
//! the one place that already has tests for it. A route table that validated its
//! own arguments would be a second answer to the same question, and the day the
//! two disagreed the REST face would be the one nobody had tested.
//!
//! What HTTP adds is the shape of the address: a document is a thing at
//! `/document`, read with GET and written with PUT, which is what a REST client
//! expects. What it does not add is a second error format: a refusal is
//! `{"error": {"code": ..., "message": ...}}`, exactly what `--call` answers, with
//! the code in the body and the matching number in the status line.
//!
//! Loopback only. The listener binds `127.0.0.1` and has no way to bind anything
//! else: this API can replace the text in a window and save it over a file, and a
//! desktop editor is not something to expose on a network without being asked.
//! A request that carries an `Origin` header is refused, which is the whole of
//! the cross-site story -- no shell tool sends one, every browser does -- and
//! `NPP_REST_TOKEN` adds a bearer check for the case where other users share the
//! machine.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::api::{self, Call};

/// The port the listener takes when no other was asked for.
pub const DEFAULT_PORT: u16 = 8085;
/// The largest header block, in bytes.
///
/// A request line, four or five headers and their names: 8 KiB is a hundred times
/// what any client sends, and a cap is what stops a peer from making the editor
/// allocate without limit before it has decided the request is valid.
pub const MAX_HEAD: usize = 8 * 1024;
/// The largest body, in bytes.
///
/// A `document.set` carries the whole new text of a document, so the cap is the
/// size of a document rather than the size of a request: 32 MiB is past the point
/// where `--call` on a command line is usable anyway.
pub const MAX_BODY: usize = 32 * 1024 * 1024;
/// How many requests may be in flight at once.
///
/// Bounded because each one holds a thread and, for its whole life, a slot in the
/// window's event loop: a client that opens a hundred connections should be told
/// to wait, not given a hundred threads that all time out.
pub const MAX_CONNECTIONS: usize = 8;
/// How long a request waits for the window to answer it.
///
/// The same 5 s `--call` gives the bus: both are waiting on the same event loop,
/// and a window that cannot answer one in 5 s will not answer the other either.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the listener waits for the rest of a request that has stalled.
const READ_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the listener waits to write a response.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// A refusal, with the status line it belongs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The HTTP status.
    pub status: u16,
    /// The code the body carries, from the same vocabulary as `--call`.
    pub code: &'static str,
    /// What a person is told.
    pub message: String,
    /// The `Allow` header, on a refusal that is about the method.
    pub allow: Option<&'static str>,
}

impl Failure {
    /// A request that was understood and is malformed.
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: 400,
            code: api::BAD_REQUEST,
            message: message.into(),
            allow: None,
        }
    }

    /// A path this build does not serve.
    pub fn not_found(path: &str) -> Self {
        Self {
            status: 404,
            code: api::UNKNOWN_METHOD,
            message: format!("no route for {path}; GET / lists them"),
            allow: None,
        }
    }

    /// A path that exists, reached with the wrong verb.
    pub fn method_not_allowed(method: &str, allow: &'static str) -> Self {
        Self {
            status: 405,
            code: api::BAD_REQUEST,
            message: format!("{method} is not how that route is reached; use {allow}"),
            allow: Some(allow),
        }
    }

    /// A request from a browser, which the listener does not serve.
    pub fn browser() -> Self {
        Self {
            status: 403,
            code: "browser",
            message: "this API refuses requests that carry an Origin header: it is \
                      loopback-only, and a page should not be able to drive an editor"
                .to_string(),
            allow: None,
        }
    }

    /// A request without the token the listener was started with.
    pub fn unauthorized() -> Self {
        Self {
            status: 401,
            code: "unauthorized",
            message: "NPP_REST_TOKEN is set, so this listener needs \
                      `Authorization: Bearer <token>`"
                .to_string(),
            allow: None,
        }
    }

    /// A body past [`MAX_BODY`].
    pub fn too_large() -> Self {
        Self {
            status: 413,
            code: api::BAD_REQUEST,
            message: format!("the body is larger than {MAX_BODY} bytes"),
            allow: None,
        }
    }

    /// A request that no window answered.
    pub fn unavailable() -> Self {
        Self {
            status: 503,
            code: api::FAILED,
            message: "the editor did not answer in time".to_string(),
            allow: None,
        }
    }

    /// A write, refused because the listener was started read-only.
    pub fn read_only() -> Self {
        Self {
            status: 403,
            code: "read_only",
            message: "this listener is read-only: its configuration file says \
                      `read_only: true`, so document.set, document.save and command.run \
                      are refused"
                .to_string(),
            allow: None,
        }
    }

    /// The body this refusal is: the same `error` object `--call` answers with.
    pub fn value(&self) -> Value {
        api::Error {
            code: self.code,
            message: self.message.clone(),
        }
        .value()
    }
}

/// What the listener is allowed to do, from `api.yaml`.
///
/// The token and read-only-ness travel together because they are the two things a
/// configuration file decides about a request, and because passing them as two
/// arguments invites a call site that gets the order wrong and silently disables
/// the check.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policy {
    /// The bearer token a request must carry, if the file set one.
    pub token: Option<String>,
    /// Whether the API may write.
    pub read_only: bool,
}

/// One route, once it has been resolved.
#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    /// `GET /`: what this listener serves, so a client can discover it.
    Index,
    /// A call from the table `--call`, MCP and the menu share.
    Call(Call),
}

/// The head of one request, as far as reading it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    /// The method, as sent.
    pub method: String,
    /// The request target: path and query, still percent-encoded.
    pub target: String,
    /// How many bytes of body follow, from `Content-Length`.
    pub content_length: usize,
    /// Whether the request carries an `Origin` header.
    pub origin: bool,
    /// The `Authorization` header, if there was one.
    pub authorization: Option<String>,
    /// The `Content-Type`, if there was one.
    pub content_type: Option<String>,
}

/// Where the header block ends, if the buffer holds all of it.
///
/// The delimiter is returned as its index past the blank line, so a reader can
/// hand the body on without counting anything again.
pub fn head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|at| at + 4)
}

/// Parses the header block of one request.
///
/// `bytes` is the block, delimiter included. Anything the listener does not act
/// on is ignored rather than refused: HTTP has a hundred headers, and a client is
/// free to send `User-Agent`, `Accept` and `Expect`. What is read is what changes
/// the answer.
pub fn parse_head(bytes: &[u8]) -> Result<Head, Failure> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| Failure::bad_request("the request head is not UTF-8"))?;
    let mut lines = text.split("\r\n");
    let line = lines
        .next()
        .ok_or_else(|| Failure::bad_request("the request line is missing"))?;
    let mut parts = line.split(' ');
    let method = parts
        .next()
        .filter(|part| !part.is_empty())
        .ok_or_else(|| Failure::bad_request("the request line has no method"))?;
    let target = parts
        .next()
        .filter(|part| part.starts_with('/'))
        .ok_or_else(|| {
            Failure::bad_request(
                "the request target must be a path, e.g. `GET /status`: a proxy-style \
                 absolute URL is not served",
            )
        })?;
    let version = parts
        .next()
        .ok_or_else(|| Failure::bad_request("the request line has no HTTP version"))?;
    if !version.starts_with("HTTP/1.") {
        return Err(Failure::bad_request(format!(
            "{version} is not served: this listener speaks HTTP/1.1"
        )));
    }
    let mut head = Head {
        method: method.to_string(),
        target: target.to_string(),
        content_length: 0,
        origin: false,
        authorization: None,
        content_type: None,
    };
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(Failure::bad_request(format!(
                "{line:?} is not a header: a header is `name: value`"
            )));
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => {
                let length = value.parse::<usize>().map_err(|_| {
                    Failure::bad_request(format!("Content-Length {value:?} is not a number"))
                })?;
                if length > MAX_BODY {
                    return Err(Failure::too_large());
                }
                head.content_length = length;
            }
            "origin" => head.origin = true,
            "authorization" => head.authorization = Some(value.to_string()),
            "content-type" => head.content_type = Some(value.to_string()),
            _ => {}
        }
    }
    Ok(head)
}

/// Resolves a request to what it asks for.
///
/// The route table is the whole of the REST vocabulary, and each row names the
/// call it stands for. A method that a path does not take is a 405 with the one it
/// does; a path that no row names is a 404.
pub fn route(head: &Head, body: &[u8]) -> Result<Route, Failure> {
    let (path, query) = match head.target.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (head.target.as_str(), None),
    };
    let params = params(query)?;
    // Every row builds the object `--call` would have been given, and the parser
    // that `--call` uses decides whether it is a call. Doing it this way is what
    // keeps the two faces from growing a difference: there is one validator.
    match (head.method.as_str(), path) {
        ("GET", "/") => Ok(Route::Index),
        ("GET", "/help") => parse(json!({ "method": "app.help" })),
        ("GET", "/status") => parse(json!({ "method": "app.status" })),
        ("GET", "/documents") => parse(json!({ "method": "document.list" })),
        ("GET", "/document") => {
            let mut request = json!({ "method": "document.get" });
            with_params(&mut request, &params, &["path"])?;
            if let Some(range) = param(&params, "range") {
                request["range"] = json!(range_of(range)?);
            }
            parse(request)
        }
        ("PUT", "/document") => {
            let mut request = json!({ "method": "document.set" });
            with_params(&mut request, &params, &["path"])?;
            let json_body = head
                .content_type
                .as_deref()
                .is_some_and(|kind| kind.to_ascii_lowercase().starts_with("application/json"));
            if json_body {
                let value: Value = serde_json::from_slice(body).map_err(|error| {
                    Failure::bad_request(format!("the body is not JSON: {error}"))
                })?;
                let Some(object) = value.as_object() else {
                    return Err(Failure::bad_request(
                        "the body must be an object with a `text` string",
                    ));
                };
                for key in ["text", "range"] {
                    if let Some(field) = object.get(key) {
                        request[key] = field.clone();
                    }
                }
            } else {
                let text = std::str::from_utf8(body).map_err(|_| {
                    Failure::bad_request(
                        "the body is not UTF-8: send JSON, or a body in the document's own \
                         encoding is not supported over this face",
                    )
                })?;
                request["text"] = json!(text);
            }
            if let Some(range) = param(&params, "range") {
                request["range"] = json!(range_of(range)?);
            }
            parse(request)
        }
        ("POST", "/document/save") => {
            let mut request = json!({ "method": "document.save" });
            with_params(&mut request, &params, &["path"])?;
            parse(request)
        }
        ("POST", path) if path.starts_with("/command/") => {
            let name = decode(&path["/command/".len()..], false)?;
            if name.is_empty() {
                return Err(Failure::bad_request(
                    "the path names no command: `POST /command/sort_asc`",
                ));
            }
            let mut request = json!({ "method": "command.run", "command": name });
            with_params(&mut request, &params, &["path"])?;
            for (key, field) in [
                ("return_text", "return_text"),
                ("whole_document", "whole_document"),
            ] {
                if let Some(value) = param(&params, key) {
                    request[field] = json!(flag(key, value)?);
                }
            }
            if let Some(range) = param(&params, "range") {
                request["range"] = json!(range_of(range)?);
            }
            parse(request)
        }
        (method, path) => match allowed(path) {
            Some(allow) => Err(Failure::method_not_allowed(method, allow)),
            None => Err(Failure::not_found(path)),
        },
    }
}

/// The methods a path takes, for a refusal that has to say which.
fn allowed(path: &str) -> Option<&'static str> {
    if path.starts_with("/command/") {
        return Some("POST");
    }
    match path {
        "/" | "/help" | "/status" | "/documents" => Some("GET"),
        "/document" => Some("GET, PUT"),
        "/document/save" => Some("POST"),
        _ => None,
    }
}

/// What `GET /` answers: the routes, and the methods behind them.
///
/// Built from the same table `route` matches on, written out by hand because the
/// table is `match` arms rather than data. A client that can read this does not
/// need to be told anything about the editor to use it.
pub fn index(version: &str) -> Value {
    json!({
        "ok": true,
        "version": version,
        "api": "REST over the same call table as `npp --call`",
        "routes": [
            {"method": "GET", "path": "/", "answers": "this list"},
            {"method": "GET", "path": "/help", "call": "app.help"},
            {"method": "GET", "path": "/status", "call": "app.status"},
            {"method": "GET", "path": "/documents", "call": "document.list"},
            {"method": "GET", "path": "/document", "call": "document.get",
             "query": ["path", "range"]},
            {"method": "PUT", "path": "/document", "call": "document.set",
             "query": ["path", "range"], "body": "the new text, or JSON {\"text\", \"range\"}"},
            {"method": "POST", "path": "/document/save", "call": "document.save",
             "query": ["path"]},
            {"method": "POST", "path": "/command/<name>", "call": "command.run",
             "query": ["return_text", "whole_document", "range"]},
        ],
        "range": "`range=start-end`, half-open character offsets, as `--call` takes them",
        "auth": "loopback only; `Origin` is refused; NPP_REST_TOKEN adds a bearer check",
    })
}

/// Whether a call changes something.
///
/// The three that do are the three a read-only listener refuses: everything else
/// reads a document, a status or the help text. Kept as a function beside the
/// check so the list is stated once and a new mutating method cannot be added
/// without this going red.
pub fn mutating(call: &Call) -> bool {
    matches!(
        call,
        Call::DocumentSet { .. } | Call::DocumentSave { .. } | Call::Command { .. }
    )
}

/// The status a call's answer belongs on.
pub fn status_of(answer: &Value) -> u16 {
    match answer
        .pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "" => 200,
        api::BAD_REQUEST => 400,
        api::UNKNOWN_DOCUMENT => 404,
        api::UNKNOWN_METHOD => 404,
        _ => 500,
    }
}

/// One response, ready to write.
pub fn response(status: u16, allow: Option<&str>, body: &Value) -> Vec<u8> {
    let body = body.to_string();
    let mut head = format!(
        "HTTP/1.1 {status} {}\r\n\
         content-type: application/json; charset=utf-8\r\n\
         content-length: {}\r\n\
         cache-control: no-store\r\n\
         connection: close\r\n",
        reason(status),
        body.len()
    );
    if let Some(allow) = allow {
        head.push_str(&format!("allow: {allow}\r\n"));
    }
    head.push_str("\r\n");
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(body.as_bytes());
    bytes
}

/// The standard phrase for a status this listener can send.
fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

/// Serves the REST face on the loopback interface.
///
/// `dispatch` is what turns a call into an answer, and it is the caller's because
/// the editor state lives on the event loop: this module knows HTTP and the route
/// table, and knows nothing about a window. `None` means the window did not answer
/// in time, which is a 503.
///
/// The listener returns immediately: the thread it starts owns the socket for the
/// life of the process. Port 0 means "any port", and the address that was actually
/// bound is the answer, which is what a test uses and what the caller prints.
pub fn serve<F>(port: u16, policy: Policy, dispatch: F) -> io::Result<SocketAddr>
where
    F: Fn(Call) -> Option<Value> + Send + Sync + 'static,
{
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))?;
    let address = listener.local_addr()?;
    let dispatch = Arc::new(dispatch);
    let policy = Arc::new(policy);
    let live = Arc::new(AtomicUsize::new(0));
    std::thread::Builder::new()
        .name("npp-rest".to_string())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                if live.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                    // The refusal is written rather than the connection dropped:
                    // a client told "busy" can wait and retry, one whose socket
                    // closes for no reason cannot.
                    let mut stream = stream;
                    let _ = stream.write_all(&response(503, None, &Failure::unavailable().value()));
                    continue;
                }
                live.fetch_add(1, Ordering::Relaxed);
                let dispatch = Arc::clone(&dispatch);
                let policy = Arc::clone(&policy);
                let counter = Arc::clone(&live);
                let spawned = std::thread::Builder::new()
                    .name("npp-rest-request".to_string())
                    .spawn(move || {
                        let _ = handle(stream, &policy, dispatch.as_ref());
                        counter.fetch_sub(1, Ordering::Relaxed);
                    });
                if spawned.is_err() {
                    live.fetch_sub(1, Ordering::Relaxed);
                }
            }
        })?;
    Ok(address)
}

/// Reads one request, answers it, and closes.
fn handle<F>(mut stream: TcpStream, policy: &Policy, dispatch: &F) -> io::Result<()>
where
    F: Fn(Call) -> Option<Value>,
{
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    let (head_bytes, mut body) = match read_head(&mut stream) {
        Ok(pair) => pair,
        Err(failure) => {
            stream.write_all(&response(failure.status, failure.allow, &failure.value()))?;
            return Ok(());
        }
    };
    let reply = match parse_head(&head_bytes).and_then(|head| {
        if head.origin {
            return Err(Failure::browser());
        }
        if let Some(expected) = policy.token.as_deref() {
            let offered = head
                .authorization
                .as_deref()
                .and_then(|value| value.strip_prefix("Bearer "))
                .map(str::trim);
            if offered != Some(expected) {
                return Err(Failure::unauthorized());
            }
        }
        if head.content_length > MAX_BODY {
            return Err(Failure::too_large());
        }
        while body.len() < head.content_length {
            let mut chunk = vec![0u8; head.content_length - body.len()];
            match stream.read(&mut chunk) {
                Ok(0) => {
                    return Err(Failure::bad_request(
                        "the body ended before Content-Length said it would",
                    ))
                }
                Ok(read) => body.extend_from_slice(&chunk[..read]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(Failure::bad_request(error.to_string())),
            }
        }
        body.truncate(head.content_length);
        route(&head, &body)
    }) {
        Ok(route) => match route {
            Route::Index => response(200, None, &index(env!("CARGO_PKG_VERSION"))),
            // The read-only check sits after routing rather than inside it: the
            // route table says what a request means, this says whether the file
            // allows it, and a 403 here is the answer to the second question.
            Route::Call(call) if policy.read_only && mutating(&call) => {
                response(403, None, &Failure::read_only().value())
            }
            Route::Call(call) => match dispatch(call) {
                Some(answer) => {
                    let status = status_of(&answer);
                    response(status, None, &answer)
                }
                None => response(503, None, &Failure::unavailable().value()),
            },
        },
        Err(failure) => response(failure.status, failure.allow, &failure.value()),
    };
    stream.write_all(&reply)?;
    Ok(())
}

/// Reads up to the end of the header block, and whatever body arrived with it.
fn read_head(stream: &mut TcpStream) -> Result<(Vec<u8>, Vec<u8>), Failure> {
    let mut buffer = Vec::with_capacity(1024);
    loop {
        if let Some(end) = head_end(&buffer) {
            let body = buffer.split_off(end);
            return Ok((buffer, body));
        }
        if buffer.len() > MAX_HEAD {
            return Err(Failure::bad_request(format!(
                "the header block is larger than {MAX_HEAD} bytes"
            )));
        }
        let mut chunk = [0u8; 1024];
        match stream.read(&mut chunk) {
            Ok(0) => {
                return Err(Failure::bad_request(
                    "the connection closed before the header block ended",
                ))
            }
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(Failure::bad_request(error.to_string())),
        }
    }
}

/// Hands a request object to the parser `--call` uses.
///
/// The answer is a [`Failure`] rather than an `api::Error` so that a route cannot
/// accidentally answer with a status of its own: 400 for anything a caller got
/// wrong, because that is what a malformed request is.
fn parse(request: Value) -> Result<Route, Failure> {
    api::parse(&request)
        .map(Route::Call)
        .map_err(|error| Failure {
            status: 400,
            code: error.code,
            message: error.message,
            allow: None,
        })
}

/// The query parameters of a target, decoded.
fn params(query: Option<&str>) -> Result<Vec<(String, String)>, Failure> {
    let Some(query) = query else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (name, value) = match pair.split_once('=') {
            Some((name, value)) => (name, value),
            None => (pair, ""),
        };
        out.push((decode(name, true)?, decode(value, true)?));
    }
    Ok(out)
}

/// The value of a query parameter, if it was given.
fn param<'a>(params: &'a [(String, String)], name: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// Copies the parameters a route reads into its request object.
///
/// `allowed` is spelled out rather than taken from the values so that a
/// misspelled parameter is not silently dropped: `?pth=/tmp/a.txt` would otherwise
/// act on the active tab, which is the one document the caller did not mean.
fn with_params(
    request: &mut Value,
    params: &[(String, String)],
    allowed: &[&str],
) -> Result<(), Failure> {
    for (name, value) in params {
        if name == "range" || !allowed.contains(&name.as_str()) {
            continue;
        }
        request[name.as_str()] = json!(value);
    }
    Ok(())
}

/// A `start-end` pair of character offsets.
fn range_of(value: &str) -> Result<(usize, usize), Failure> {
    let (start, end) = value.split_once('-').ok_or_else(|| {
        Failure::bad_request(format!(
            "`range` is `start-end`, not {value:?}: `?range=12-20`"
        ))
    })?;
    let start = start
        .parse::<usize>()
        .map_err(|_| Failure::bad_request(format!("{start:?} is not a character offset")))?;
    let end = end
        .parse::<usize>()
        .map_err(|_| Failure::bad_request(format!("{end:?} is not a character offset")))?;
    Ok((start, end))
}

/// A query flag: `1`, `true`, `yes`, `on` and their off positions.
fn flag(name: &str, value: &str) -> Result<bool, Failure> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        other => Err(Failure::bad_request(format!(
            "`{name}` is a flag, not {other:?}: use 1 or 0"
        ))),
    }
}

/// Decodes percent escapes, and `+` in a query.
///
/// `plus` is false for a path segment: `+` is a plus sign in a path and a space in
/// a query string, and a command named `a+b` is a command named `a+b`.
fn decode(text: &str, plus: bool) -> Result<String, Failure> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let hex = bytes.get(index + 1..index + 3).ok_or_else(|| {
                    Failure::bad_request(format!("{text:?} ends in the middle of a percent escape"))
                })?;
                let hex = std::str::from_utf8(hex)
                    .ok()
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                    .ok_or_else(|| {
                        Failure::bad_request(format!(
                            "{text:?} has a percent escape that is not hex"
                        ))
                    })?;
                out.push(hex);
                index += 3;
            }
            b'+' if plus => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out)
        .map_err(|_| Failure::bad_request(format!("{text:?} decodes to bytes that are not UTF-8")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request head, built the way a client would send it.
    fn head(method: &str, target: &str) -> Head {
        parse_head(format!("{method} {target} HTTP/1.1\r\n\r\n").as_bytes()).unwrap()
    }

    /// The call a target resolves to, with an empty body.
    fn call_of(method: &str, target: &str) -> Result<Call, Failure> {
        match route(&head(method, target), b"")? {
            Route::Call(call) => Ok(call),
            Route::Index => Err(Failure::bad_request("that route is the index")),
        }
    }

    #[test]
    fn a_head_is_a_method_a_target_and_its_headers() {
        let parsed = parse_head(
            b"PUT /document?path=%2Ftmp%2Fa.txt HTTP/1.1\r\n\
              Content-Length: 5\r\n\
              Content-Type: text/plain\r\n\
              Origin: http://example.com\r\n\
              Authorization: Bearer s3cret\r\n\r\n",
        )
        .unwrap();
        assert_eq!(parsed.method, "PUT");
        assert_eq!(parsed.target, "/document?path=%2Ftmp%2Fa.txt");
        assert_eq!(parsed.content_length, 5);
        assert!(parsed.origin);
        assert_eq!(parsed.authorization.as_deref(), Some("Bearer s3cret"));
        assert_eq!(parsed.content_type.as_deref(), Some("text/plain"));
    }

    #[test]
    fn a_header_block_that_has_not_arrived_yet_is_not_an_error() {
        assert_eq!(head_end(b"GET / HTTP/1.1\r\nHost: x\r\n"), None);
        assert_eq!(head_end(b"GET / HTTP/1.1\r\n\r\nBODY"), Some(18));
    }

    #[test]
    fn a_status_line_that_is_not_a_path_is_refused() {
        // The absolute form is what a proxy sends; nothing here is a proxy.
        let failure = parse_head(b"GET http://example.com/ HTTP/1.1\r\n\r\n").unwrap_err();
        assert_eq!(failure.status, 400);
    }

    #[test]
    fn a_body_larger_than_the_cap_is_refused_while_reading_the_head() {
        let text = format!(
            "PUT /document HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY + 1
        );
        assert_eq!(parse_head(text.as_bytes()).unwrap_err().status, 413);
    }

    #[test]
    fn every_route_is_the_call_it_names() {
        assert_eq!(call_of("GET", "/status").unwrap(), Call::Status);
        assert_eq!(call_of("GET", "/help").unwrap(), Call::Help);
        assert_eq!(call_of("GET", "/documents").unwrap(), Call::DocumentList);
        assert_eq!(
            call_of("POST", "/document/save").unwrap(),
            Call::DocumentSave {
                which: api::Which::Active
            }
        );
        assert!(matches!(
            call_of("GET", "/document").unwrap(),
            Call::DocumentGet { .. }
        ));
    }

    #[test]
    fn the_index_is_a_route_of_its_own() {
        assert_eq!(route(&head("GET", "/"), b"").unwrap(), Route::Index);
        assert!(index("0.3.0")["routes"].as_array().unwrap().len() >= 7);
    }

    #[test]
    fn a_path_parameter_names_the_document() {
        let call = call_of("GET", "/document?path=%2Ftmp%2Fa.txt").unwrap();
        assert_eq!(
            call,
            Call::DocumentGet {
                which: api::Which::Path("/tmp/a.txt".to_string()),
                range: None,
            }
        );
    }

    #[test]
    fn a_range_is_two_offsets_in_either_order() {
        let call = call_of("GET", "/document?range=20-12").unwrap();
        assert_eq!(
            call,
            Call::DocumentGet {
                which: api::Which::Active,
                range: Some((12, 20)),
            }
        );
    }

    #[test]
    fn a_range_that_is_not_two_numbers_is_refused() {
        for target in ["/document?range=12", "/document?range=a-b", "/document?range=1-"] {
            let failure = call_of("GET", target).unwrap_err();
            assert_eq!(failure.status, 400, "{target}");
        }
    }

    #[test]
    fn a_body_of_text_is_the_new_document() {
        match route(&head("PUT", "/document"), b"hello\n").unwrap() {
            Route::Call(Call::DocumentSet { text, range, which }) => {
                assert_eq!(text, "hello\n");
                assert_eq!(range, None);
                assert_eq!(which, api::Which::Active);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_json_body_carries_text_and_range() {
        let mut head = head("PUT", "/document?path=%2Ftmp%2Fa.txt");
        head.content_type = Some("application/json".to_string());
        match route(&head, br#"{"text": "abc", "range": [1, 3]}"#).unwrap() {
            Route::Call(Call::DocumentSet { text, range, which }) => {
                assert_eq!(text, "abc");
                assert_eq!(range, Some((1, 3)));
                assert_eq!(which, api::Which::Path("/tmp/a.txt".to_string()));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_body_that_is_not_utf8_is_refused_with_a_reason() {
        let failure = route(&head("PUT", "/document"), &[0xff, 0xfe]).unwrap_err();
        assert_eq!(failure.status, 400);
        assert!(failure.message.contains("UTF-8"));
    }

    #[test]
    fn a_command_is_named_in_the_path_and_flag_its_options() {
        match call_of("POST", "/command/sort_asc?whole_document=1&return_text=1").unwrap() {
            Call::Command {
                name,
                text,
                whole,
                range,
            } => {
                assert_eq!(name, "sort_asc");
                assert!(text);
                assert!(whole);
                assert_eq!(range, None);
            }
            other => panic!("{other:?}"),
        }
        // A flag that is not a flag is a caller mistake, not a false.
        assert_eq!(
            call_of("POST", "/command/sort_asc?whole_document=maybe")
                .unwrap_err()
                .status,
            400
        );
    }

    #[test]
    fn whole_document_and_range_together_are_refused_by_the_shared_parser() {
        let failure = call_of(
            "POST",
            "/command/sort_asc?whole_document=1&range=1-2",
        )
        .unwrap_err();
        assert_eq!(failure.status, 400);
        assert!(failure.message.contains("not both"), "{}", failure.message);
    }

    #[test]
    fn the_wrong_method_is_a_405_that_says_which() {
        let failure = route(&head("PUT", "/documents"), b"").unwrap_err();
        assert_eq!(failure.status, 405);
        assert_eq!(failure.allow, Some("GET"));

        let failure = route(&head("GET", "/document/save"), b"").unwrap_err();
        assert_eq!(failure.status, 405);
        assert_eq!(failure.allow, Some("POST"));
    }

    #[test]
    fn a_path_that_is_not_a_route_is_a_404() {
        let failure = route(&head("GET", "/nope"), b"").unwrap_err();
        assert_eq!(failure.status, 404);
        assert_eq!(failure.code, api::UNKNOWN_METHOD);
    }

    #[test]
    fn a_command_path_with_no_name_is_refused() {
        assert_eq!(call_of("POST", "/command/").unwrap_err().status, 400);
    }

    #[test]
    fn a_percent_escape_that_is_not_hex_is_refused() {
        assert_eq!(call_of("GET", "/document?path=%zz").unwrap_err().status, 400);
        assert_eq!(call_of("GET", "/document?path=%2").unwrap_err().status, 400);
    }

    #[test]
    fn a_plus_is_a_space_in_a_query_and_a_plus_in_a_path() {
        assert_eq!(decode("a+b", true).unwrap(), "a b");
        assert_eq!(decode("a+b", false).unwrap(), "a+b");
        assert_eq!(decode("%C3%A9", false).unwrap(), "é");
    }

    #[test]
    fn an_answer_becomes_the_status_it_earned() {
        let error = |code| api::Error {
            code,
            message: "x".into(),
        }
        .value();
        assert_eq!(status_of(&json!({"ok": true})), 200);
        assert_eq!(status_of(&error(api::BAD_REQUEST)), 400);
        assert_eq!(status_of(&error(api::UNKNOWN_DOCUMENT)), 404);
        assert_eq!(status_of(&error(api::FAILED)), 500);
    }

    #[test]
    fn a_response_declares_the_length_it_sends() {
        let bytes = response(200, None, &json!({"ok": true}));
        let text = String::from_utf8(bytes.clone()).unwrap();
        let (head, body) = text.split_once("\r\n\r\n").unwrap();
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(head.contains(&format!("content-length: {}", body.len())));
        assert_eq!(body, r#"{"ok":true}"#);
        assert_eq!(text.len(), bytes.len());
    }

    #[test]
    fn a_405_response_carries_its_allow_header() {
        let bytes = response(405, Some("GET, PUT"), &json!({"ok": false}));
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("allow: GET, PUT\r\n"), "{text}");
    }

    /// A dispatch that answers every call the same way, for the socket tests.
    fn canned(_call: Call) -> Option<Value> {
        Some(json!({"ok": true, "version": "0.3.0"}))
    }

    /// Sends one raw request to a listener and reads its answer.
    fn ask(address: SocketAddr, request: &str) -> String {
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        answer
    }

    #[test]
    fn a_request_is_answered_over_a_socket() {
        let address = serve(0, Policy::default(), canned).unwrap();
        let answer = ask(address, "GET /status HTTP/1.1\r\nHost: localhost\r\n\r\n");
        assert!(answer.starts_with("HTTP/1.1 200 OK\r\n"), "{answer}");
        assert!(answer.ends_with(r#"{"ok":true,"version":"0.3.0"}"#), "{answer}");
    }

    #[test]
    fn the_index_is_served_without_a_window() {
        let address = serve(0, Policy::default(), canned).unwrap();
        let answer = ask(address, "GET / HTTP/1.1\r\n\r\n");
        assert!(answer.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(answer.contains("/document/save"), "{answer}");
    }

    #[test]
    fn a_body_split_over_two_writes_is_read_whole() {
        let address = serve(0, Policy::default(), canned).unwrap();
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .write_all(b"PUT /document HTTP/1.1\r\nContent-Length: 5\r\n\r\nhe")
            .unwrap();
        stream.write_all(b"llo").unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        assert!(answer.starts_with("HTTP/1.1 200 OK\r\n"), "{answer}");
    }

    #[test]
    fn a_browser_origin_is_refused() {
        let address = serve(0, Policy::default(), canned).unwrap();
        let answer = ask(
            address,
            "GET /status HTTP/1.1\r\nOrigin: http://evil.example\r\n\r\n",
        );
        assert!(answer.starts_with("HTTP/1.1 403 Forbidden\r\n"), "{answer}");
        assert!(answer.contains("\"code\":\"browser\""), "{answer}");
    }

    #[test]
    fn a_token_is_required_when_one_was_given() {
        let address = serve(0, Policy { token: Some("s3cret".to_string()), ..Policy::default() }, canned).unwrap();
        let refused = ask(address, "GET /status HTTP/1.1\r\n\r\n");
        assert!(refused.starts_with("HTTP/1.1 401 Unauthorized\r\n"), "{refused}");
        let allowed = ask(
            address,
            "GET /status HTTP/1.1\r\nAuthorization: Bearer s3cret\r\n\r\n",
        );
        assert!(allowed.starts_with("HTTP/1.1 200 OK\r\n"), "{allowed}");
    }

    #[test]
    fn a_window_that_does_not_answer_is_a_503() {
        let address = serve(0, Policy::default(), |_call| None).unwrap();
        let answer = ask(address, "GET /status HTTP/1.1\r\n\r\n");
        assert!(answer.starts_with("HTTP/1.1 503 Service Unavailable\r\n"), "{answer}");
    }

    #[test]
    fn a_read_only_listener_refuses_a_write_and_answers_a_read() {
        let policy = Policy {
            read_only: true,
            ..Policy::default()
        };
        let address = serve(0, policy, canned).unwrap();
        let refused = ask(address, "PUT /document HTTP/1.1\r\nContent-Length: 2\r\n\r\nhi");
        assert!(refused.starts_with("HTTP/1.1 403 Forbidden\r\n"), "{refused}");
        assert!(refused.contains("\"code\":\"read_only\""), "{refused}");
        let allowed = ask(address, "GET /documents HTTP/1.1\r\n\r\n");
        assert!(allowed.starts_with("HTTP/1.1 200 OK\r\n"), "{allowed}");
    }

    #[test]
    fn the_three_writing_calls_are_the_ones_read_only_names() {
        assert!(mutating(&Call::DocumentSet {
            which: api::Which::Active,
            text: "x".into(),
            range: None,
        }));
        assert!(mutating(&Call::DocumentSave {
            which: api::Which::Active
        }));
        assert!(mutating(&Call::Command {
            name: "sort_asc".into(),
            text: false,
            range: None,
            whole: true,
        }));
        for call in [
            Call::Status,
            Call::Help,
            Call::DocumentList,
            Call::DocumentGet {
                which: api::Which::Active,
                range: None,
            },
        ] {
            assert!(!mutating(&call), "{call:?}");
        }
    }

    #[test]
    fn the_wrong_method_over_a_socket_is_a_405_with_an_allow_header() {
        let address = serve(0, Policy::default(), canned).unwrap();
        let answer = ask(address, "DELETE /document HTTP/1.1\r\n\r\n");
        assert!(answer.starts_with("HTTP/1.1 405 Method Not Allowed\r\n"), "{answer}");
        assert!(answer.contains("allow: GET, PUT\r\n"), "{answer}");
    }
}
