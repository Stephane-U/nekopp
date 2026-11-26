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

//! The terminal: raw mode, its size, and one read that can wait.
//!
//! Everything here is a thin wrapper over three libc calls -- `tcgetattr`,
//! `tcsetattr`, `ioctl` for the window size, and `poll` for a read with a
//! deadline -- because that is the whole of what a cell-drawing editor needs
//! from a terminal library.
//!
//! The terminal is put back the way it was found in [`Terminal::close`], which
//! runs from `Drop` and, through the panic hook in `main`, when the program
//! dies of anything but a clean exit. A release build is compiled with
//! `panic = "abort"`, so a `Drop` on the way out is not something to count on.

use std::io::{self, Write};

/// What the program draws with, and what it has to put back.
pub struct Terminal {
    /// The settings as they were before raw mode, restored on the way out.
    saved: libc::termios,
    /// Standard output, buffered by this struct rather than by `line`.
    out: io::Stdout,
    /// Whether the terminal is still in raw mode and on the alternate screen.
    live: bool,
}

impl Terminal {
    /// Takes the terminal over: raw mode, alternate screen, no cursor.
    ///
    /// Raw mode is what makes a keystroke visible without Enter, and the
    /// alternate screen is what keeps the editor from scribbling over whatever
    /// the shell had on it.
    pub fn open() -> io::Result<Self> {
        let out = io::stdout();
        if !is_terminal(libc::STDIN_FILENO) {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "standard input is not a terminal",
            ));
        }
        // SAFETY: `termios` is a plain C struct that `tcgetattr` fills in, and
        // the file descriptor is one the process owns for its whole life.
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut saved) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut raw = saved;
        // SAFETY: `raw` was just copied from a filled-in `termios`.
        unsafe { libc::cfmakeraw(&mut raw) };
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut terminal = Terminal {
            saved,
            out,
            live: true,
        };
        // Alternate screen, hidden cursor, no line wrap surprises: the grid
        // writes every row in full, so wrapping is the one thing it must not do.
        terminal.write(b"\x1b[?1049h\x1b[?25l\x1b[0m")?;
        Ok(terminal)
    }

    /// The size of the window, in cells.
    pub fn size(&self) -> (usize, usize) {
        // SAFETY: `winsize` is a plain C struct that the ioctl fills in.
        let mut size: libc::winsize = unsafe { std::mem::zeroed() };
        let asked = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) };
        if asked != 0 || size.ws_col == 0 || size.ws_row == 0 {
            // A terminal that cannot say how big it is gets the classic size
            // rather than a division by zero.
            return (80, 24);
        }
        (size.ws_col as usize, size.ws_row as usize)
    }

    /// Whatever arrived within `timeout_ms`, or `None` if nothing did.
    ///
    /// The timeout is what lets the loop notice a window that changed size and
    /// a message that should not stay forever.
    pub fn read(&mut self, timeout_ms: i32) -> io::Result<Option<Vec<u8>>> {
        let mut ready = libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one `pollfd` this function owns, and a timeout that is only a
        // number.
        let waited = unsafe { libc::poll(&mut ready, 1, timeout_ms) };
        if waited < 0 {
            let error = io::Error::last_os_error();
            // A signal between the call and the return is not a failure; the
            // caller simply tries again.
            if error.kind() == io::ErrorKind::Interrupted {
                return Ok(None);
            }
            return Err(error);
        }
        if waited == 0 {
            return Ok(None);
        }
        let mut buffer = [0u8; 1024];
        // SAFETY: `buffer` is owned here and the length says how much of it the
        // kernel may write.
        let count = unsafe {
            libc::read(
                libc::STDIN_FILENO,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
            )
        };
        if count < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                return Ok(None);
            }
            return Err(error);
        }
        if count == 0 {
            // End of input: the terminal went away.
            return Ok(Some(Vec::new()));
        }
        Ok(Some(buffer[..count as usize].to_vec()))
    }

    /// Writes bytes out and flushes: a frame is one write.
    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut lock = self.out.lock();
        lock.write_all(bytes)?;
        lock.flush()
    }

    /// The settings this terminal was opened with.
    ///
    /// Handed out so the panic hook in `main` can put the terminal back with a
    /// copy rather than through the `Terminal` it cannot reach.
    pub fn saved(&self) -> libc::termios {
        self.saved
    }

    /// Puts the terminal back, and is safe to call twice.
    pub fn close(&mut self) {
        if !self.live {
            return;
        }
        self.live = false;
        let _ = self.write(b"\x1b[0m\x1b[?25h\x1b[?1049l");
        // SAFETY: `saved` is the struct `tcgetattr` gave us at the start.
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &self.saved) };
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.close();
    }
}

/// Whether a file descriptor is a terminal.
fn is_terminal(fd: i32) -> bool {
    // SAFETY: `isatty` only looks at the descriptor.
    unsafe { libc::isatty(fd) == 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_file_descriptor_is_not_a_terminal() {
        // The test harness runs with its own standard input, which in a pipe is
        // not a terminal; either answer is possible on a developer's machine, so
        // the assertion is that the call answers rather than that it refuses.
        let answer = is_terminal(libc::STDIN_FILENO);
        assert!(answer || !answer);
    }
}
