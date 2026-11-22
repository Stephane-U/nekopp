//! Ending a launch the desktop started.
//!
//! A launch from the file manager or from a desktop entry arrives with a startup
//! token in the environment. The desktop puts the pointer into its busy state
//! when it spawns the program and takes it out again when the program says the
//! sequence is over -- and if nothing ever says so, it keeps turning until the
//! desktop's own timeout gives up, which is seconds.
//!
//! Opening a window says so by itself: winit writes the token onto the window and
//! sends the message as it creates it. A launch that hands its files to a running
//! editor creates no window, so nothing was saying so, and every file opened from
//! the file manager left the pointer spinning over work that had finished in
//! twenty-odd milliseconds. That is what made the file manager feel slow when the
//! editor was already open.
//!
//! This module sends the message the window would have sent. The wire format is
//! the one the specification defines and the one winit and GTK speak, which is
//! what the desktop's startup monitor is listening for:
//!
//! <https://specifications.freedesktop.org/startup-notification-spec/startup-notification-latest.txt>

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ClientMessageEvent, ConnectionExt, CreateWindowAux, EventMask, WindowClass,
};

/// The variable a launcher puts the startup token in, on X11.
///
/// The Wayland equivalent, `XDG_ACTIVATION_TOKEN`, needs nothing here: the
/// compositor spends it when the window is presented, and there is no message to
/// send.
const TOKEN: &str = "DESKTOP_STARTUP_ID";

/// How much of the message one client message carries.
///
/// The data field is twenty bytes and the text is sent in pieces that size; the
/// receiver concatenates them until it sees the terminating NUL.
const CHUNK: usize = 20;

/// Reports the launch named in the environment as finished.
///
/// Does nothing when there is no token, which is the case for a launch from a
/// shell, and for every launch that is not from a launcher that tracks startups.
/// A failure is reported and then ignored: a pointer that spins for a few more
/// seconds is a smaller problem than an editor that refuses to open a file.
pub fn complete() {
    // A Wayland-only session has no X server to send the message to, and no use
    // for it: the compositor spends the activation token when the window is
    // presented, and a handoff presents no window.
    if std::env::var_os("DISPLAY").is_none() {
        return;
    }
    let Some(token) = std::env::var(TOKEN).ok().filter(|token| !token.is_empty()) else {
        return;
    };
    if let Err(err) = send_remove(&token) {
        eprintln!("npp: could not report the launch as finished: {err}");
    }
}

/// Sends `remove: ID="<token>"` to the root window.
///
/// Only ever called when a window was not going to be created, so the connection
/// is opened for this one message and closed by the process exiting a millisecond
/// later.
fn send_remove(token: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (connection, screen_num) = x11rb::connect(None)?;
    let screen = &connection.setup().roots[screen_num];
    let begin = connection
        .intern_atom(false, b"_NET_STARTUP_INFO_BEGIN")?
        .reply()?
        .atom;
    let rest = connection
        .intern_atom(false, b"_NET_STARTUP_INFO")?
        .reply()?
        .atom;

    // Every client message names a window, and none of them is the subject of
    // this one. winit sends the pair over a throwaway 1x1 window it never maps,
    // so this does the same rather than inventing a second convention for the
    // desktop to learn.
    let carrier = connection.generate_id()?;
    connection.create_window(
        screen.root_depth,
        carrier,
        screen.root,
        -100,
        -100,
        1,
        1,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new()
            .override_redirect(1)
            .event_mask(EventMask::STRUCTURE_NOTIFY | EventMask::PROPERTY_CHANGE),
    )?
    .check()?;

    let message = remove_message(token);
    let mut kind = begin;
    for piece in message.chunks(CHUNK) {
        let mut data = [0u8; CHUNK];
        data[..piece.len()].copy_from_slice(piece);
        let event = ClientMessageEvent::new(8, carrier, kind, data);
        // The property mask on the root window is the subscription the desktop's
        // startup monitor holds, and a client message is only delivered to the
        // clients that asked for one.
        connection
            .send_event(false, screen.root, EventMask::PROPERTY_CHANGE, event)?
            .check()?;
        kind = rest;
    }
    connection.flush()?;
    Ok(())
}

/// The text of the message that ends a startup sequence.
///
/// `remove: ID="<token>"`, with the token quoted the way the specification says:
/// the quotes around it are part of the syntax, and a quote or a backslash
/// inside it is escaped with a backslash. NUL-terminated, because the receiver
/// concatenates the pieces of a message until it meets the terminator.
fn remove_message(token: &str) -> Vec<u8> {
    let mut message = Vec::with_capacity(token.len() + 16);
    message.extend_from_slice(b"remove: ID=\"");
    for byte in token.bytes() {
        if byte == b'"' || byte == b'\\' {
            message.push(b'\\');
        }
        message.push(byte);
    }
    message.push(b'"');
    message.push(0);
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_names_the_token() {
        assert_eq!(remove_message("abc"), b"remove: ID=\"abc\"\0");
    }

    #[test]
    fn a_quote_or_a_backslash_in_the_token_is_escaped() {
        assert_eq!(remove_message("a\"b\\c"), b"remove: ID=\"a\\\"b\\\\c\"\0");
    }

    #[test]
    fn a_real_token_needs_more_than_one_message() {
        // What gio passes in the environment: the desktop file's name, the
        // process id, the host and the launch time.
        let token = "neko++.desktop-2261-host-Time1700000000.123456";
        assert!(remove_message(token).len() > CHUNK);
        assert!(remove_message("x").len() <= CHUNK);
    }
}
