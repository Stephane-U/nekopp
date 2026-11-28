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

//! `npp-tui`: Neko++ in a terminal.
//!
//! The editor -- the document, the caret, the undo history, the lexers -- is
//! `nn-core`, which knows nothing about a window or a terminal. This binary adds
//! what a terminal needs and nothing else: raw mode, a grid of cells, a
//! difference between the last frame and this one, and a loop that reads a key
//! at a time.
//!
//! The loop draws first and reads second, so a keystroke is on the screen before
//! the next one is waited for, and the read has a deadline so a window that has
//! been resized is noticed while the editor is idle rather than at the next key.

mod app;
mod complete;
mod find;
mod keys;
mod menus;
mod palette;
mod playground;
mod plugins;
mod paramhint;
mod prefs;
mod prompt;
mod results;
mod screen;
mod split;
mod styles;
mod symbols;
mod term;
mod workspace;

use std::path::PathBuf;
use std::sync::Mutex;

use app::App;
use screen::Frame;
use term::Terminal;

/// The terminal settings to put back if the program dies of a panic.
///
/// A release build is compiled with `panic = "abort"`, so a `Drop` never runs on
/// the way out of a crash; without this the shell would come back with no echo
/// and no line ending.
static SAVED: Mutex<Option<libc::termios>> = Mutex::new(None);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print!("{}", help());
        return;
    }
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("npp-tui {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let path: Option<PathBuf> = args.iter().find(|arg| !arg.starts_with('-')).map(PathBuf::from);

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_now();
        default_hook(info);
    }));

    match run(path) {
        Ok(()) => {}
        Err(error) => {
            restore_now();
            eprintln!("npp-tui: {error}");
            std::process::exit(1);
        }
    }
}

/// Opens the terminal, edits until `Ctrl+Q`, and puts the terminal back.
fn run(path: Option<PathBuf>) -> std::io::Result<()> {
    let mut app = App::open(path.as_deref())?;
    let mut terminal = Terminal::open()?;
    remember(&terminal);
    let (width, height) = terminal.size();
    let mut frame = Frame::new(width, height);
    loop {
        // The playground is a clock as well as a screen, so it is stepped once per
        // frame; the editor is not, and one `Instant` per iteration is the whole
        // cost of asking.
        app.advance();
        frame.set_palette(app.palette());
        app.draw(frame.grid());
        terminal.write(frame.render())?;
        match terminal.read(app.tick_ms())? {
            None => {}
            Some(bytes) if bytes.is_empty() => break,
            Some(bytes) => {
                let mut at = 0;
                while at < bytes.len() {
                    match keys::parse(&bytes[at..]) {
                        Some((key, used)) => {
                            at += used;
                            app.handle(key);
                        }
                        // A sequence this shell does not know: skip its first
                        // byte and try again, so one unknown key cannot wedge
                        // the loop.
                        None => at += 1,
                    }
                }
            }
        }
        if app.take_quit() {
            break;
        }
        let size = terminal.size();
        if size != (frame.width(), frame.height()) {
            frame.resize(size.0, size.1);
        }
    }
    terminal.close();
    // The recent files this session touched and the tabs it has open, written
    // once, on the way out, as the window writes them: a shell that is killed
    // rather than quit loses the same thing the window would.
    app.save_session();
    Ok(())
}

/// Keeps a copy of the terminal settings where the panic hook can reach it.
fn remember(terminal: &Terminal) {
    if let Ok(mut saved) = SAVED.lock() {
        *saved = Some(terminal.saved());
    }
}

/// Puts the terminal back without needing the `Terminal` that owns it.
fn restore_now() {
    let Ok(saved) = SAVED.lock() else {
        return;
    };
    let Some(saved) = *saved else {
        return;
    };
    let escapes = b"\x1b[0m\x1b[?25h\x1b[?1049l";
    // SAFETY: the descriptor is the process's own standard output and the bytes
    // are a fixed escape sequence.
    unsafe {
        libc::write(libc::STDOUT_FILENO, escapes.as_ptr().cast(), escapes.len());
    }
    // SAFETY: `saved` came from `tcgetattr` on standard input at startup.
    unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &saved) };
}

/// What the program takes, said in the terminal it is run from.
fn help() -> String {
    format!(
        "npp-tui {} -- Neko++ in a terminal\n\
         \n\
         Usage: npp-tui [FILE]\n\
         \n\
         Keys:\n\
         \x20 F9       the menus, and every command they offer\n\
         \x20 Ctrl+F   the find bar; Enter walks the matches\n\
         \x20 Ctrl+Q   quit; twice when there are unsaved changes\n\
         \x20 Arrows, Home, End, Page Up, Page Down\n\
         \x20 Shift with an arrow extends the selection\n\
         \x20 Ctrl with Left or Right moves by the word\n\
         \n\
         The control chords are the ones in the settings file, which is the\n\
         windowed editor's list: Ctrl+S saves, Ctrl+Z undoes, Ctrl+D duplicates\n\
         the line, Ctrl+L deletes it, Ctrl+U lowercases the selection. A chord\n\
         bound to a command this shell has not got yet says so on the bar.\n",
        env!("CARGO_PKG_VERSION")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_help_names_the_program_and_its_keys() {
        let text = help();
        assert!(text.contains("npp-tui"), "{text}");
        assert!(text.contains("Ctrl+S"), "{text}");
        assert!(text.contains("Ctrl+Q"), "{text}");
    }
}
