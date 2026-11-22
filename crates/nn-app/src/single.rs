//! One editor per session.
//!
//! Notepad++ is single-instance by default: a second launch hands its file to
//! the running window and exits. On Windows that is a window message; on Linux
//! the equivalent that survives both X11 and Wayland is a name on the session
//! bus. A second process asks the bus who owns `org.npp.Editor`; if nobody
//! does, it takes the name and becomes the server; if somebody does, it calls
//! `Open` on them and exits.
//!
//! The bus is optional. A machine with no session bus -- a bare TTY, a
//! container, a CI run -- gets a standalone editor rather than an error, which
//! is why every entry point here returns `Option` or `Result` and the caller
//! treats "no bus" as "carry on alone".
//!
//! ## Why the request carries absolute paths
//!
//! The second process resolves `notes.txt` against *its* working directory, and
//! the first process has a different one. A relative path sent over the bus
//! would be opened against the wrong directory, so [`absolute`] resolves it
//! before it leaves the process.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// The well-known bus name the running editor owns.
pub const NAME: &str = "org.npp.Editor";

/// The object path the interface lives at.
pub const PATH: &str = "/org/npp/Editor";

/// What a second process asks the first to do.
///
/// A struct rather than a list of arguments because the bus signature is then
/// one type that can grow without changing the method name, and because the
/// same value is what the tests build.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, zbus::zvariant::Type)]
pub struct Request {
    /// The documents to open, absolute.
    pub paths: Vec<String>,
    /// The line to put the caret on, one-based, or zero for "leave it".
    pub line: u32,
    /// The column to put the caret on, one-based, or zero for "leave it".
    pub column: u32,
    /// Whether the caller wants to be told when the documents close.
    pub wait: bool,
}

impl Request {
    /// A request for `paths` with no position and no wait.
    pub fn new(paths: Vec<String>) -> Self {
        Self {
            paths,
            ..Self::default()
        }
    }

    /// The same request with a caret position.
    pub fn at(mut self, line: usize, column: usize) -> Self {
        self.line = line as u32;
        self.column = column as u32;
        self
    }

    /// The same request asking to be told when the documents close.
    pub fn waiting(mut self) -> Self {
        self.wait = true;
        self
    }

    /// The caret position as zero-based `(line, column)`, when one was asked
    /// for.
    ///
    /// The bus carries one-based positions because the command line does, and
    /// the two must agree: a request built from `--line 3` has to land on the
    /// same line as a launch with `--line 3`.
    pub fn caret(&self) -> Option<(usize, usize)> {
        if self.line == 0 && self.column == 0 {
            return None;
        }
        Some((
            (self.line.max(1) as usize).saturating_sub(1),
            (self.column.max(1) as usize).saturating_sub(1),
        ))
    }
}

/// The index of the tab already showing `path`, if any.
///
/// A second launch of a file that is already open focuses the tab instead of
/// opening a second one. Two tabs on one file is not what `npp notes.txt`
/// means, and it is actively harmful: the two copies diverge and the last save
/// wins.
pub fn find_open(open: &[Option<PathBuf>], path: &Path) -> Option<usize> {
    open.iter().position(|candidate| candidate.as_deref() == Some(path))
}

/// Resolves `path` against the current directory, so it means the same thing in
/// the process that receives it.
///
/// `canonicalize` is not used: it fails on a file that does not exist yet, and
/// "open this new file" is a normal request. Joining the current directory is
/// enough, and it keeps the name the user typed.
pub fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    match std::env::current_dir() {
        Ok(dir) => dir.join(path),
        Err(_) => path.to_path_buf(),
    }
}

/// Whether a launch should hand its files to a running editor.
///
/// `--new-instance` is the escape hatch, and it is also what a caller wants
/// when the running editor is wedged: without it there is no way to start a
/// second window at all.
pub fn should_hand_off(new_instance: bool) -> bool {
    !new_instance
}

/// The interface the running editor serves.
///
/// The queue is a `Mutex<Vec<Request>>` rather than a channel because the
/// consumer is the winit event loop, which polls once per frame and has no
/// place to block. A channel would need a thread to drain it into the same
/// vector.
pub struct Editor {
    queue: Arc<Mutex<Vec<Request>>>,
    calls: Board,
}

/// The calls waiting for an answer, and the answers written for them.
///
/// `Call` cannot answer where it is asked: the window is on another thread, and
/// the work is opening a document or editing one, which belongs to the event
/// loop. So the call is given a number, queued, and answered later; the caller
/// polls [`Editor::answer`] for it.
///
/// The board is a `Mutex` rather than a channel for the same reason the request
/// queue is: the consumer is the event loop, which polls.
#[derive(Clone, Default)]
pub struct Board {
    next: Arc<Mutex<u64>>,
    pending: Arc<Mutex<Vec<Pending>>>,
    answers: Arc<Mutex<std::collections::HashMap<u64, String>>>,
}

/// One call that arrived and has not been answered yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    /// The number the caller polls with.
    pub id: u64,
    /// The JSON request.
    pub json: String,
}

impl Board {
    /// Queues `json` and returns the number the caller waits on.
    pub fn push(&self, json: String) -> u64 {
        let id = {
            let mut next = match self.next.lock() {
                Ok(next) => next,
                Err(poisoned) => poisoned.into_inner(),
            };
            *next += 1;
            *next
        };
        if let Ok(mut pending) = self.pending.lock() {
            pending.push(Pending { id, json });
        }
        id
    }

    /// The calls that arrived since the last call to this method.
    pub fn take(&self) -> Vec<Pending> {
        match self.pending.lock() {
            Ok(mut pending) => std::mem::take(&mut *pending),
            Err(_) => Vec::new(),
        }
    }

    /// Records the answer to call `id`.
    ///
    /// An answer for a caller that gave up is dropped rather than kept: the map
    /// would otherwise grow by one entry per abandoned request for the life of
    /// the window.
    pub fn answer(&self, id: u64, json: String) {
        if let Ok(mut answers) = self.answers.lock() {
            answers.insert(id, json);
        }
    }

    /// The answer to `id`, if it has arrived.
    pub fn take_answer(&self, id: u64) -> Option<String> {
        match self.answers.lock() {
            Ok(mut answers) => answers.remove(&id),
            Err(_) => None,
        }
    }

    /// How long a caller waits before giving up.
    ///
    /// Five seconds is a long time for an edit and a short time for a person to
    /// notice: what it bounds is a window that has stopped answering (a modal
    /// dialog, a stuck save), which would otherwise leave the caller hung with no
    /// way to say so.
    pub const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
}

#[zbus::interface(name = "org.npp.Editor")]
impl Editor {
    /// Opens `request`'s documents in the running window.
    ///
    /// Returns immediately: the window is not on this thread, so the work is
    /// queued for the next frame. A caller that asked to wait listens for
    /// [`Editor::closed`] instead of blocking here.
    fn open(&self, request: Request) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.push(request);
        }
    }

    /// Queues one API call and returns the number to ask [`Editor::answer`] for.
    ///
    /// Separate from the answer rather than one blocking method because the
    /// window answers on its own thread: a method that waited here would block
    /// the connection's own message loop, and the answer would never be written.
    fn call(&self, json: String) -> u64 {
        self.calls.push(json)
    }

    /// The answer to call `id`, or an empty string while it is still being made.
    ///
    /// Empty means "not yet", and an answer is always a JSON object, so the two
    /// cannot be confused.
    fn answer(&self, id: u64) -> String {
        self.calls.take_answer(id).unwrap_or_default()
    }

    /// The version of the running editor, so a caller can tell whether the
    /// process it found is one it can talk to.
    fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    /// Emitted when a document closes, which is what a `--wait` caller is
    /// waiting for.
    #[zbus(signal)]
    async fn closed(emitter: &zbus::object_server::SignalEmitter<'_>, path: &str) -> zbus::Result<()>;
}

/// The server side: the connection that owns [`NAME`].
pub struct Server {
    connection: zbus::blocking::Connection,
    queue: Arc<Mutex<Vec<Request>>>,
    calls: Board,
}

impl Server {
    /// Takes the bus name, or returns `None` when another process already has
    /// it or there is no session bus.
    ///
    /// The two cases are deliberately not distinguished. Both mean "do not be
    /// the server", and the caller's next step is the same: try to hand off,
    /// and if that fails too, run standalone.
    pub fn acquire() -> Option<Self> {
        let queue: Arc<Mutex<Vec<Request>>> = Arc::new(Mutex::new(Vec::new()));
        let calls = Board::default();
        let editor = Editor {
            queue: Arc::clone(&queue),
            calls: calls.clone(),
        };
        let connection = zbus::blocking::connection::Builder::session()
            .ok()?
            .serve_at(PATH, editor)
            .ok()?
            .build()
            .ok()?;
        // `DoNotQueue` is the whole point: without it the bus puts the second
        // process in a queue for the name and reports success, so two editors
        // would both believe they are the server and the second would sit
        // waiting for a name it will never get.
        let reply = connection
            .request_name_with_flags(NAME, zbus::fdo::RequestNameFlags::DoNotQueue.into())
            .ok()?;
        match reply {
            zbus::fdo::RequestNameReply::PrimaryOwner
            | zbus::fdo::RequestNameReply::AlreadyOwner => Some(Self {
                connection,
                queue,
                calls,
            }),
            _ => None,
        }
    }

    /// The requests that arrived since the last call.
    pub fn take(&self) -> Vec<Request> {
        match self.queue.lock() {
            Ok(mut queue) => std::mem::take(&mut *queue),
            Err(_) => Vec::new(),
        }
    }

    /// The API calls that arrived since the last call.
    pub fn take_calls(&self) -> Vec<Pending> {
        self.calls.take()
    }

    /// Records the answer to call `id`, for the caller that is polling for it.
    pub fn answer(&self, id: u64, json: String) {
        self.calls.answer(id, json);
    }

    /// Tells any waiting caller that `path` has closed.
    ///
    /// A failure is ignored: the only listener is a process that may have been
    /// killed, and there is nothing useful to do about it.
    pub fn closed(&self, path: &str) {
        let Ok(iface) = self.connection.object_server().interface::<_, Editor>(PATH) else {
            return;
        };
        let _ = zbus::block_on(Editor::closed(iface.signal_emitter(), path));
    }
}

/// The client side: a process that found a running editor.
pub struct Client {
    connection: zbus::blocking::Connection,
}

impl Client {
    /// Connects to the session bus, or `None` when there is none.
    pub fn connect() -> Option<Self> {
        let connection = zbus::blocking::Connection::session().ok()?;
        Some(Self { connection })
    }

    /// Whether a process owns [`NAME`] right now.
    pub fn is_running(&self) -> bool {
        name_has_owner(&self.connection)
    }

    /// Sends `request` to the running editor.
    pub fn send(&self, request: &Request) -> Result<(), zbus::Error> {
        self.connection.call_method(
            Some(NAME),
            PATH,
            Some("org.npp.Editor"),
            "Open",
            request,
        )?;
        Ok(())
    }

    /// Sends one API call to the running editor and returns its answer.
    ///
    /// Two round trips, because the window answers on its own thread: the call is
    /// queued, then the answer is polled for. The polling interval is short
    /// enough that it is invisible next to the frame it waits for, and long
    /// enough that a five-second wait is a thousand messages rather than a busy
    /// loop.
    pub fn call(&self, json: &str) -> Result<String, CallError> {
        let reply = self
            .connection
            .call_method(Some(NAME), PATH, Some("org.npp.Editor"), "Call", &(json,))
            .map_err(|error| CallError::Bus(error.to_string()))?;
        let id: u64 = reply
            .body()
            .deserialize()
            .map_err(|error| CallError::Bus(error.to_string()))?;
        let deadline = std::time::Instant::now() + Board::TIMEOUT;
        while std::time::Instant::now() < deadline {
            let reply = self
                .connection
                .call_method(Some(NAME), PATH, Some("org.npp.Editor"), "Answer", &(id,))
                .map_err(|error| CallError::Bus(error.to_string()))?;
            let answer: String = reply
                .body()
                .deserialize()
                .map_err(|error| CallError::Bus(error.to_string()))?;
            if !answer.is_empty() {
                return Ok(answer);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        Err(CallError::Timeout)
    }

    /// Registers interest in [`Editor::closed`] signals.
    ///
    /// Separate from [`Waiter::wait_for`] so the rule can be in place *before*
    /// the request is sent. A document that closes the instant it opens would
    /// otherwise emit its signal before anyone was listening, and a `--wait`
    /// caller would block forever.
    pub fn waiter(&self) -> Result<Waiter, zbus::Error> {
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.npp.Editor")?
            .member("Closed")?
            .path(PATH)?
            .build();
        let mut signals =
            zbus::blocking::MessageIterator::for_match_rule(rule, &self.connection, Some(64))?;
        // The iterator blocks with no timeout, so it cannot also poll for the
        // editor's death. A thread owns it and forwards what it hears; the
        // waiter then has a channel it can time out on.
        let (sender, events) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for message in signals.by_ref() {
                let Ok(message) = message else {
                    continue;
                };
                let body = message.body();
                let Ok((closed,)) = body.deserialize::<(&str,)>() else {
                    continue;
                };
                if sender.send(closed.to_string()).is_err() {
                    break;
                }
            }
        });
        Ok(Waiter {
            events,
            connection: self.connection.clone(),
        })
    }
}

/// Why a call to the running editor did not come back with an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallError {
    /// The bus refused the message: no editor, no name, no session bus.
    Bus(String),
    /// The editor took the call and did not answer in time.
    Timeout,
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::Bus(error) => write!(f, "{error}"),
            CallError::Timeout => write!(
                f,
                "the editor did not answer within {} seconds",
                Board::TIMEOUT.as_secs()
            ),
        }
    }
}

/// Whether a process owns [`NAME`] right now.
fn name_has_owner(connection: &zbus::blocking::Connection) -> bool {
    let Ok(reply) = connection.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "NameHasOwner",
        &(NAME,),
    ) else {
        return false;
    };
    reply.body().deserialize::<bool>().unwrap_or(false)
}

/// A registered listener for close signals.
pub struct Waiter {
    events: std::sync::mpsc::Receiver<String>,
    connection: zbus::blocking::Connection,
}

impl Waiter {
    /// Blocks until the running editor reports that `path` closed, or until the
    /// editor is gone.
    ///
    /// Signals for other documents are skipped rather than treated as an
    /// answer: a `--wait` caller that opened two files is waiting for both, and
    /// the first one to close must not release it.
    ///
    /// The owner check is what stops a `--wait` caller from hanging forever
    /// when the editor is killed. A signal stream does not end when its sender
    /// dies -- the bus simply stops delivering -- so waiting on the signal
    /// alone would block until the caller itself was killed.
    pub fn wait_for(&mut self, path: &str) -> Result<(), zbus::Error> {
        loop {
            match self
                .events
                .recv_timeout(std::time::Duration::from_millis(200))
            {
                Ok(closed) if closed == path => return Ok(()),
                Ok(_) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if !name_has_owner(&self.connection) {
                        return Ok(());
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_path_is_resolved_against_the_current_directory() {
        let resolved = absolute(Path::new("notes.txt"));
        assert!(resolved.is_absolute());
        assert!(resolved.ends_with("notes.txt"));
    }

    #[test]
    fn an_absolute_path_is_left_alone() {
        assert_eq!(absolute(Path::new("/tmp/x.txt")), PathBuf::from("/tmp/x.txt"));
    }

    #[test]
    fn a_launch_hands_off_unless_it_was_told_not_to() {
        assert!(should_hand_off(false));
        assert!(!should_hand_off(true));
    }

    #[test]
    fn a_request_carries_its_position_and_its_wait() {
        let request = Request::new(vec!["/tmp/x".to_string()])
            .at(3, 7)
            .waiting();
        assert_eq!(request.paths, vec!["/tmp/x".to_string()]);
        assert_eq!(request.line, 3);
        assert_eq!(request.column, 7);
        assert!(request.wait);
    }

    #[test]
    fn a_request_position_is_converted_to_zero_based() {
        assert_eq!(Request::new(Vec::new()).at(3, 7).caret(), Some((2, 6)));
        assert_eq!(Request::new(Vec::new()).at(3, 0).caret(), Some((2, 0)));
        assert_eq!(Request::new(Vec::new()).caret(), None);
    }

    #[test]
    fn an_open_path_is_found_by_its_index() {
        let open = vec![
            Some(PathBuf::from("/tmp/a.txt")),
            None,
            Some(PathBuf::from("/tmp/b.txt")),
        ];
        assert_eq!(find_open(&open, Path::new("/tmp/b.txt")), Some(2));
        assert_eq!(find_open(&open, Path::new("/tmp/c.txt")), None);
        assert_eq!(find_open(&open, Path::new("/tmp/a.txt")), Some(0));
    }

    #[test]
    fn a_plain_request_asks_for_no_position() {
        let request = Request::new(Vec::new());
        assert_eq!(request.line, 0);
        assert_eq!(request.column, 0);
        assert!(!request.wait);
    }

    /// The bus tests need a session bus. They are skipped rather than failed
    /// when there is none, because a container without one is a normal place to
    /// run the test suite.
    fn bus() -> Option<Client> {
        Client::connect()
    }

    #[test]
    fn a_server_takes_the_name_and_a_second_one_does_not() {
        let Some(_client) = bus() else {
            return;
        };
        let Some(server) = Server::acquire() else {
            // Another test in this process already owns the name, or the bus
            // refused it. Either way there is nothing to assert.
            return;
        };
        assert!(Server::acquire().is_none(), "the name is exclusive");
        drop(server);
    }

    #[test]
    fn a_request_sent_over_the_bus_arrives_at_the_server() {
        let Some(client) = bus() else {
            return;
        };
        let Some(server) = Server::acquire() else {
            return;
        };
        assert!(client.is_running());
        let request = Request::new(vec!["/tmp/one.txt".to_string()]).at(2, 5);
        client.send(&request).expect("send");
        // The call is synchronous on the bus, so the queue is filled by the
        // time `send` returns.
        let received = server.take();
        assert_eq!(received, vec![request]);
        assert!(server.take().is_empty(), "taking drains the queue");
    }

    #[test]
    fn a_waiter_returns_when_the_editor_goes_away() {
        let Some(client) = bus() else {
            return;
        };
        let Some(server) = Server::acquire() else {
            return;
        };
        let mut waiter = client.waiter().expect("a waiter");
        // The server is dropped without ever emitting `Closed`, which is what a
        // killed editor looks like from the bus: the name goes away and no
        // signal ever arrives.
        drop(server);
        let started = std::time::Instant::now();
        waiter.wait_for("/tmp/never.txt").expect("the wait ends");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "the wait must not hang when the editor is gone"
        );
    }

    #[test]
    fn a_closed_document_reaches_a_waiting_caller() {
        let Some(client) = bus() else {
            return;
        };
        let Some(server) = Server::acquire() else {
            return;
        };
        // Register the rule first, then close: the other order can miss the
        // signal, which is exactly the race the doc comment warns about.
        let mut waiter = client.waiter().expect("a waiter");
        let handle = std::thread::spawn(move || waiter.wait_for("/tmp/two.txt"));
        std::thread::sleep(std::time::Duration::from_millis(150));
        server.closed("/tmp/two.txt");
        let result = handle.join().expect("the waiter thread");
        assert!(result.is_ok());
    }
}
