//! The XDG desktop portal's file chooser.
//!
//! # Why a portal and not our own dialog
//!
//! We already have a file browser (`browser.rs`), and it is what Notepad++ does:
//! its own dialog, drawn by the application. On Linux that is the wrong answer
//! for two reasons.
//!
//! The first is that a user's file chooser is where their bookmarks, their
//! recent files, their network shares and their mounted phones live. A dialog
//! drawn by us can only show what we know how to enumerate, which is the
//! filesystem.
//!
//! The second is sandboxing. Under Flatpak or snap the application cannot read
//! an arbitrary path at all; the only way to open a file is to ask the portal,
//! which asks the user, which grants a document-portal handle. An editor that
//! only knows how to open paths is an editor that cannot open anything in a
//! sandbox.
//!
//! So the portal is tried first and the built-in browser is the fallback. The
//! fallback matters: the portal is a separate service, and on a machine without
//! one -- a bare X session, a container, a CI box -- asking for it would mean no
//! file dialog at all.
//!
//! # The handshake
//!
//! `FileChooser.OpenFile` does not return a path. It returns an object path for
//! a *request*, and the answer arrives later as a `Response` signal on that
//! path. The path is derived from the caller's unique bus name and a token the
//! caller chooses, which is what makes it possible to subscribe before making
//! the call -- and subscribing after would race the signal.
//!
//! The call blocks until the user answers, so it runs on its own thread and the
//! result comes back through the event loop.

use std::collections::HashMap;
use std::path::PathBuf;

use futures_lite::StreamExt;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

/// The portal service.
pub const PORTAL: &str = "org.freedesktop.portal.Desktop";
/// The portal's object path.
pub const PATH: &str = "/org/freedesktop/portal/desktop";
/// The file chooser interface.
pub const FILE_CHOOSER: &str = "org.freedesktop.portal.FileChooser";
/// The interface a request's answer arrives on.
pub const REQUEST: &str = "org.freedesktop.portal.Request";
/// The settings interface, which is how a desktop says what it looks like.
pub const SETTINGS: &str = "org.freedesktop.portal.Settings";
/// The namespace the colour scheme lives in.
pub const APPEARANCE: &str = "org.freedesktop.appearance";
/// The key the colour scheme lives under.
pub const COLOR_SCHEME: &str = "color-scheme";

/// How long to wait for an answer before giving up.
///
/// Five minutes is longer than anyone takes to pick a file and short enough that
/// a portal that died mid-request does not leave a thread parked forever. The
/// timeout is reported as a cancellation, because from the user's side that is
/// what it is.
pub const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Runs `answer` and gives up after `timeout`.
///
/// `None` means the timer won. This is a separate function for one reason: the
/// race is the part that can be wrong, and it is the part that has nothing to do
/// with D-Bus, so it can be tested without a portal.
///
/// `async_io::block_on` rather than `futures_lite::block_on` because only the
/// former drives the reactor that `Timer` registers with; the latter would park
/// the thread and the timer would never fire.
fn with_timeout<T>(
    answer: impl std::future::Future<Output = T>,
    timeout: std::time::Duration,
) -> Option<T> {
    async_io::block_on(futures_lite::future::or(
        async { Some(answer.await) },
        async {
            async_io::Timer::after(timeout).await;
            None
        },
    ))
}

/// Which dialog to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Pick existing files.
    Open,
    /// Pick a path to write.
    Save,
}

impl Kind {
    /// The `FileChooser` method this kind calls.
    fn method(self) -> &'static str {
        match self {
            Kind::Open => "OpenFile",
            Kind::Save => "SaveFile",
        }
    }
}

/// A file-type filter, as the dialog shows it.
///
/// The portal wants glob patterns, not extensions, because a filter can be
/// `*.tar.gz`. The caller supplies the patterns it means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    /// The label in the dialog's filter dropdown.
    pub name: String,
    /// The glob patterns, for example `*.txt`.
    pub patterns: Vec<String>,
}

/// What came back from the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The user picked these paths.
    Chosen(Vec<PathBuf>),
    /// The user closed the dialog, or it timed out.
    Cancelled,
    /// The portal refused, or the bus is not there.
    Failed(String),
}

/// Whether a portal is running on the session bus.
///
/// A name lookup rather than a call: it is one round trip and it answers the
/// only question that matters before showing a dialog.
pub fn available() -> bool {
    let Ok(connection) = zbus::blocking::Connection::session() else {
        return false;
    };
    connection
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "NameHasOwner",
            &(PORTAL,),
        )
        .ok()
        .and_then(|reply| reply.body().deserialize::<bool>().ok())
        .unwrap_or(false)
}

/// The desktop's colour-scheme preference.
///
/// The portal defines three values, and the third one matters: "no preference"
/// is not "light". A desktop that has not been told what to do should leave the
/// editor on the palette it has always used rather than guess.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    /// The desktop has no opinion.
    NoPreference,
    /// The desktop prefers a dark interface.
    Dark,
    /// The desktop prefers a light interface.
    Light,
}

impl Scheme {
    /// What the portal's `u32` means.
    ///
    /// Anything the spec does not define is treated as "no preference" rather
    /// than as an error: a future value is a preference this build does not
    /// know how to honour, and guessing at it would be worse than leaving the
    /// editor as it is.
    pub fn from_value(value: u32) -> Scheme {
        match value {
            1 => Scheme::Dark,
            2 => Scheme::Light,
            _ => Scheme::NoPreference,
        }
    }

    /// Whether this preference means the light palette, or `None` when the
    /// desktop has no opinion.
    pub fn is_light(self) -> Option<bool> {
        match self {
            Scheme::Light => Some(true),
            Scheme::Dark => Some(false),
            Scheme::NoPreference => None,
        }
    }
}

/// How long to wait for the settings portal before starting without it.
///
/// The call is one round trip to a service that is already running, which is
/// under two milliseconds on a healthy session. The bound exists for the other
/// case: a portal that is wedged must not hold the window closed, because a
/// wrong palette is a much smaller problem than an editor that does not start.
pub const SCHEME_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(200);

/// The desktop's colour-scheme preference, or `None` when there is no portal to
/// ask.
///
/// Blocking, and bounded: the call runs on its own thread and the caller waits
/// at most [`SCHEME_TIMEOUT`]. A thread rather than a timeout on the proxy
/// because zbus's blocking proxy has no per-call deadline, and a wedged portal
/// is exactly the case this has to survive.
pub fn color_scheme() -> Option<Scheme> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(read_color_scheme());
    });
    receiver.recv_timeout(SCHEME_TIMEOUT).ok().flatten()
}

/// One `Read` call against the settings portal.
fn read_color_scheme() -> Option<Scheme> {
    let connection = zbus::blocking::Connection::session().ok()?;
    let proxy = zbus::blocking::Proxy::new(&connection, PORTAL, PATH, SETTINGS).ok()?;
    // `Read` answers with a variant, and `OwnedValue` keeps the wrapper: its
    // signature is `v`, so deserializing a `v` into it gives a value that is
    // still inside a variant. `downcast_ref` is what steps through that
    // wrapper; a plain `u32::try_from` fails with `IncorrectType` on a value
    // that looks exactly right in a debug print.
    let value: OwnedValue = proxy.call("Read", &(APPEARANCE, COLOR_SCHEME)).ok()?;
    let value = value.downcast_ref::<u32>().ok()?;
    Some(Scheme::from_value(value))
}

/// Calls `on_change` for every colour-scheme change the desktop announces.
///
/// Blocks until the bus goes away, which is the only thing that ends it: the
/// caller runs it on a thread and lets the process exit stop it. The match rule
/// filters on the namespace and the key as well as the interface, so a desktop
/// changing its font size does not wake this thread.
///
/// A signal that cannot be read is skipped rather than reported. The alternative
/// is a thread that dies on the first malformed message and leaves the editor
/// following a preference that changed ten minutes ago.
pub fn watch_color_scheme(mut on_change: impl FnMut(Scheme)) -> Result<(), String> {
    let connection = zbus::blocking::Connection::session()
        .map_err(|err| format!("no session bus: {err}"))?;
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(SETTINGS)
        .and_then(|builder| builder.member("SettingChanged"))
        .and_then(|builder| builder.arg(0, APPEARANCE))
        .and_then(|builder| builder.arg(1, COLOR_SCHEME))
        .map_err(|err| format!("bad match rule: {err}"))?
        .build();
    let signals = zbus::blocking::MessageIterator::for_match_rule(rule, &connection, Some(4))
        .map_err(|err| format!("cannot watch the settings: {err}"))?;
    for message in signals {
        let Ok(message) = message else {
            continue;
        };
        let body = message.body();
        let Ok((_namespace, _key, value)) = body.deserialize::<(String, String, OwnedValue)>()
        else {
            continue;
        };
        // The third argument is a variant, so it is unwrapped the same way the
        // `Read` answer is.
        let Ok(value) = value.downcast_ref::<u32>() else {
            continue;
        };
        on_change(Scheme::from_value(value));
    }
    Ok(())
}

/// The object path a request's answer will arrive on.
///
/// The portal derives it from the caller's unique name with the leading colon
/// dropped and every dot replaced by an underscore, plus the token the caller
/// chose. Getting this wrong means subscribing to a path nothing will ever be
/// sent to, and the call then hangs until the timeout -- which is why it is a
/// separate function with its own test.
pub fn handle_path(sender: &str, token: &str) -> String {
    let sender = sender.trim_start_matches(':').replace('.', "_");
    format!("/org/freedesktop/portal/desktop/request/{sender}/{token}")
}

/// A `file://` URI as a path.
///
/// The portal answers in URIs, not paths, because a document-portal handle is
/// not a path at all. Percent escapes have to be undone: a file called
/// `my notes.txt` arrives as `my%20notes.txt`, and a path with a `%` in it
/// arrives as `%25`.
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // Everything up to the first slash is the authority. `file:///tmp/a` has an
    // empty one and an absolute path; `file://host/a` names a file on another
    // machine, and opening it as a local path would silently open the wrong file.
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, ""),
    };
    // `localhost` is the one authority that does mean this machine.
    if !authority.is_empty() && authority != "localhost" {
        return None;
    }
    if path.is_empty() {
        return None;
    }
    Some(PathBuf::from(percent_decode(path)))
}

/// Undoes `%XX` escapes.
///
/// Hand-written rather than pulled from a crate: it is fifteen lines, it is the
/// only place we need it, and a dependency for it would be larger than the code.
/// A malformed escape is left as it is rather than dropped, because a path that
/// keeps its literal `%` is more likely to be the file the user meant than one
/// with the character silently removed.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
            if let Some(byte) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    // The escapes are UTF-8 sequences, so the result is valid UTF-8 whenever the
    // input was; a lossy conversion is the safe answer for the case where it is
    // not.
    String::from_utf8_lossy(&out).into_owned()
}

/// The `filters` option, in the shape the portal expects.
///
/// `a(sa(us))`: a list of `(name, [(0, pattern), ...])`. The `0` is the filter
/// type, and 0 means "glob pattern" -- the only type the file chooser defines.
pub fn filters_value(filters: &[Filter]) -> Vec<(String, Vec<(u32, String)>)> {
    filters
        .iter()
        .map(|filter| {
            (
                filter.name.clone(),
                filter
                    .patterns
                    .iter()
                    .map(|pattern| (0u32, pattern.clone()))
                    .collect(),
            )
        })
        .collect()
}

/// What a `Response` code means.
///
/// The spec defines three. 0 is "carried out", 1 is "the user cancelled", and 2
/// is "the interaction ended some other way" -- the dialog was closed, or the
/// window manager took it away. The last two are the same thing from the user's
/// side, and treating 2 as a failure would answer a closed dialog with a second
/// dialog, which is exactly what pressing Escape must not do.
fn response_outcome(response: u32) -> Result<(), Outcome> {
    match response {
        0 => Ok(()),
        1 | 2 => Err(Outcome::Cancelled),
        other => Err(Outcome::Failed(format!("the portal answered {other}"))),
    }
}

/// The options dictionary for a call.
fn options(
    token: &str,
    kind: Kind,
    filters: &[Filter],
    current_name: Option<&str>,
) -> HashMap<String, Value<'static>> {
    let mut options: HashMap<String, Value<'static>> = HashMap::new();
    options.insert("handle_token".to_string(), Value::from(token.to_string()));
    // Modal to the parent window, so the dialog cannot be lost behind the
    // editor. The portal ignores this when it has no parent to attach to.
    options.insert("modal".to_string(), Value::from(true));
    if !filters.is_empty() {
        options.insert(
            "filters".to_string(),
            Value::from(filters_value(filters)),
        );
    }
    // An open dialog may name more than one file, which is what Notepad++'s
    // Open has always allowed. The answer is already a list of URIs, so this
    // flag is the whole of the request; without it every portal returns exactly
    // one, however many the user picked.
    if kind == Kind::Open {
        options.insert("multiple".to_string(), Value::from(true));
    }
    if kind == Kind::Save {
        if let Some(name) = current_name {
            options.insert("current_name".to_string(), Value::from(name.to_string()));
        }
    }
    options
}

/// Shows the dialog and waits for the answer.
///
/// Blocking: the caller runs it on a thread. `parent` is the window identifier
/// the portal wants (`x11:<hex>` or `wayland:<handle>`), or the empty string for
/// a dialog with no parent.
pub fn choose(
    kind: Kind,
    title: &str,
    filters: &[Filter],
    current_name: Option<&str>,
    parent: &str,
) -> Outcome {
    let connection = match zbus::blocking::Connection::session() {
        Ok(connection) => connection,
        Err(err) => return Outcome::Failed(format!("no session bus: {err}")),
    };
    let Some(sender) = connection.unique_name().map(|name| name.to_string()) else {
        return Outcome::Failed("the bus gave us no name".to_string());
    };
    // The token has to be unique per request and a valid object-path element, so
    // it is letters and digits only.
    let token = format!("npp{}", std::process::id());
    let path = handle_path(&sender, &token);

    // Subscribed before the call: the portal can answer before `OpenFile`
    // returns, and a subscription made afterwards would miss it.
    let rule = match zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(REQUEST)
        .and_then(|builder| builder.member("Response"))
        .and_then(|builder| builder.path(path.as_str()))
    {
        Ok(builder) => builder.build(),
        Err(err) => return Outcome::Failed(format!("bad match rule: {err}")),
    };
    let answers = match zbus::blocking::MessageIterator::for_match_rule(rule, &connection, Some(4)) {
        Ok(answers) => answers,
        Err(err) => return Outcome::Failed(format!("cannot watch for the answer: {err}")),
    };
    // The blocking iterator's `next` waits forever. Taking the stream out of it
    // is what makes the wait boundable: a portal that never answers would
    // otherwise park this thread with the "a dialog is open" flag still set, and
    // Open and Save As would be dead for the rest of the session.
    let mut answers = answers.into_inner();

    let proxy = match zbus::blocking::Proxy::new(&connection, PORTAL, PATH, FILE_CHOOSER) {
        Ok(proxy) => proxy,
        Err(err) => return Outcome::Failed(format!("no file chooser: {err}")),
    };
    let options = options(&token, kind, filters, current_name);
    let call: Result<OwnedObjectPath, _> =
        proxy.call(kind.method(), &(parent, title, options));
    if let Err(err) = call {
        return Outcome::Failed(format!("{}: {err}", kind.method()));
    }

    // The answer, or nothing if the portal took too long. A dialog that never
    // came back is, from the user's side, a dialog they closed.
    let Some(answer) = with_timeout(answers.next(), TIMEOUT) else {
        return Outcome::Cancelled;
    };
    // The stream ending is a different thing: the connection went away.
    let Some(answer) = answer else {
        return Outcome::Failed("the portal closed the request".to_string());
    };
    let message = match answer {
        Ok(message) => message,
        Err(err) => return Outcome::Failed(format!("the answer was lost: {err}")),
    };
    let body = message.body();
    let Ok((response, results)) = body.deserialize::<(u32, HashMap<String, OwnedValue>)>() else {
        return Outcome::Failed("the answer was not a response".to_string());
    };
    if let Err(outcome) = response_outcome(response) {
        return outcome;
    }
    let Some(uris) = results.get("uris") else {
        return Outcome::Failed("the answer had no uris".to_string());
    };
    let Ok(uris) = <Vec<String>>::try_from(uris.clone()) else {
        return Outcome::Failed("the uris were not strings".to_string());
    };
    let paths: Vec<PathBuf> = uris.iter().filter_map(|uri| uri_to_path(uri)).collect();
    if paths.is_empty() {
        Outcome::Cancelled
    } else {
        Outcome::Chosen(paths)
    }
}

/// The window identifier the portal wants for `window`.
///
/// X11 wants `x11:<window id in hex>`; Wayland wants `wayland:<handle>`. An
/// empty string is always legal and means "no parent", which is what we fall
/// back to when the handle is not one of those two.
pub fn parent_id(window: &winit::window::Window) -> String {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = window.window_handle() else {
        return String::new();
    };
    match handle.as_raw() {
        RawWindowHandle::Xlib(xlib) => format!("x11:{:x}", xlib.window),
        RawWindowHandle::Xcb(xcb) => format!("x11:{:x}", xcb.window.get()),
        RawWindowHandle::Wayland(wayland) => format!("wayland:{:?}", wayland.surface),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_scheme_values_map_to_the_three_answers() {
        assert_eq!(Scheme::from_value(0), Scheme::NoPreference);
        assert_eq!(Scheme::from_value(1), Scheme::Dark);
        assert_eq!(Scheme::from_value(2), Scheme::Light);
    }

    #[test]
    fn a_scheme_value_from_the_future_is_no_preference() {
        // The spec defines 0, 1 and 2. A newer desktop sending 3 is asking for
        // something this build does not know how to honour, and guessing would
        // be worse than leaving the editor alone.
        assert_eq!(Scheme::from_value(3), Scheme::NoPreference);
        assert_eq!(Scheme::from_value(u32::MAX), Scheme::NoPreference);
    }

    #[test]
    fn only_the_two_real_preferences_name_a_palette() {
        assert_eq!(Scheme::Dark.is_light(), Some(false));
        assert_eq!(Scheme::Light.is_light(), Some(true));
        assert_eq!(Scheme::NoPreference.is_light(), None);
    }

    #[test]
    fn the_handle_path_drops_the_colon_and_flattens_the_dots() {
        assert_eq!(
            handle_path(":1.42", "npp7"),
            "/org/freedesktop/portal/desktop/request/1_42/npp7"
        );
    }

    #[test]
    fn a_sender_without_a_colon_is_left_alone() {
        assert_eq!(
            handle_path("1.42", "t"),
            "/org/freedesktop/portal/desktop/request/1_42/t"
        );
    }

    #[test]
    fn a_plain_file_uri_becomes_a_path() {
        assert_eq!(
            uri_to_path("file:///tmp/a.txt"),
            Some(PathBuf::from("/tmp/a.txt"))
        );
    }

    #[test]
    fn a_percent_escape_is_undone() {
        assert_eq!(
            uri_to_path("file:///tmp/my%20notes.txt"),
            Some(PathBuf::from("/tmp/my notes.txt"))
        );
    }

    #[test]
    fn a_literal_percent_survives_a_round_trip() {
        assert_eq!(
            uri_to_path("file:///tmp/100%25.txt"),
            Some(PathBuf::from("/tmp/100%.txt"))
        );
    }

    #[test]
    fn a_malformed_escape_is_kept_rather_than_dropped() {
        // `%zz` is not an escape. Dropping it would turn a file called `%zz` into
        // one called `zz`, which is a different file.
        assert_eq!(
            uri_to_path("file:///tmp/%zz.txt"),
            Some(PathBuf::from("/tmp/%zz.txt"))
        );
        assert_eq!(
            uri_to_path("file:///tmp/trail%"),
            Some(PathBuf::from("/tmp/trail%"))
        );
    }

    #[test]
    fn a_utf8_escape_becomes_the_character() {
        // `é` is C3 A9.
        assert_eq!(
            uri_to_path("file:///tmp/caf%C3%A9.txt"),
            Some(PathBuf::from("/tmp/café.txt"))
        );
    }

    #[test]
    fn a_remote_host_is_not_a_local_path() {
        // `file://host/path` names a file on another machine; opening it as a
        // local path would silently open the wrong file.
        assert_eq!(uri_to_path("file://host/tmp/a.txt"), None);
    }

    #[test]
    fn a_non_file_uri_is_rejected() {
        assert_eq!(uri_to_path("http://example.com/a.txt"), None);
        assert_eq!(uri_to_path("/tmp/a.txt"), None);
    }

    #[test]
    fn the_root_is_a_path() {
        assert_eq!(uri_to_path("file:///"), Some(PathBuf::from("/")));
    }

    #[test]
    fn filters_become_glob_patterns() {
        let filters = vec![Filter {
            name: "Text".to_string(),
            patterns: vec!["*.txt".to_string(), "*.md".to_string()],
        }];
        assert_eq!(
            filters_value(&filters),
            vec![(
                "Text".to_string(),
                vec![(0, "*.txt".to_string()), (0, "*.md".to_string())]
            )]
        );
    }

    #[test]
    fn no_filters_is_an_empty_list() {
        assert!(filters_value(&[]).is_empty());
    }

    #[test]
    fn the_options_always_carry_a_token_and_are_modal() {
        let options = options("tok", Kind::Open, &[], None);
        assert!(options.contains_key("handle_token"));
        assert!(options.contains_key("modal"));
        // An open dialog has nothing to suggest, and a `current_name` on an open
        // dialog is ignored by every portal implementation.
        assert!(!options.contains_key("current_name"));
    }

    #[test]
    fn a_save_dialog_carries_the_suggested_name() {
        let options = options("tok", Kind::Save, &[], Some("untitled.txt"));
        assert!(options.contains_key("current_name"));
    }

    #[test]
    fn filters_are_only_sent_when_there_are_some() {
        assert!(!options("tok", Kind::Open, &[], None).contains_key("filters"));
        let filters = vec![Filter {
            name: "Text".to_string(),
            patterns: vec!["*.txt".to_string()],
        }];
        assert!(options("tok", Kind::Open, &filters, None).contains_key("filters"));
    }

    #[test]
    fn an_open_dialog_asks_for_more_than_one_file() {
        // Without this flag a portal answers with exactly one URI, however many
        // files the user picked, and every way into the editor that goes through
        // the system dialog opens one document at a time.
        let options = options("tok", Kind::Open, &[], None);
        assert_eq!(options.get("multiple"), Some(&Value::from(true)));
    }

    #[test]
    fn a_save_dialog_asks_for_one_file() {
        // The save answer is one buffer written to one path, so asking for a
        // list would be asking a question the caller would have to refuse.
        assert!(!options("tok", Kind::Save, &[], Some("notes.txt")).contains_key("multiple"));
    }

    #[test]
    fn a_zero_response_is_success() {
        assert_eq!(response_outcome(0), Ok(()));
    }

    #[test]
    fn both_cancellation_codes_are_cancellations() {
        // 1 is "the user cancelled" and 2 is "the interaction ended some other
        // way". Escape in the GTK dialog sends 2, so treating it as a failure
        // would pop a second dialog at the user.
        assert_eq!(response_outcome(1), Err(Outcome::Cancelled));
        assert_eq!(response_outcome(2), Err(Outcome::Cancelled));
    }

    #[test]
    fn an_unknown_response_is_a_failure() {
        assert!(matches!(response_outcome(7), Err(Outcome::Failed(_))));
    }

    #[test]
    fn the_two_kinds_call_different_methods() {
        assert_eq!(Kind::Open.method(), "OpenFile");
        assert_eq!(Kind::Save.method(), "SaveFile");
    }

    #[test]
    fn an_answer_that_arrives_is_returned() {
        assert_eq!(
            with_timeout(async { 7 }, std::time::Duration::from_secs(30)),
            Some(7)
        );
    }

    #[test]
    fn an_answer_that_never_arrives_gives_up() {
        // The point of the whole exercise: a portal that stops answering must
        // not park the caller. `pending` is a future that is never ready, which
        // is exactly what a wedged `MessageStream` looks like from here.
        let started = std::time::Instant::now();
        let answer = with_timeout(
            std::future::pending::<u8>(),
            std::time::Duration::from_millis(20),
        );
        assert_eq!(answer, None);
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
    }

    #[test]
    fn the_timeout_is_long_enough_to_pick_a_file() {
        // A guard against someone "fixing" a slow test by shortening this to
        // something a human would hit.
        assert!(TIMEOUT >= std::time::Duration::from_secs(60));
    }
}


