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

//! `npp-mcp`: the Neko++ feature reference as a Model Context Protocol server.
//!
//! Started with no arguments. It reads JSON-RPC 2.0 from stdin, one object per
//! line, and writes one object per line to stdout; everything else -- the
//! startup line, an I/O failure -- goes to stderr, because stdout is the
//! protocol. A client starts it as `npp-mcp` and talks to it over a pipe, so
//! there is nothing to configure and no argument to get wrong.

use std::io;

use nn_mcp::{Installed, Server, SERVER_NAME, SERVER_VERSION};

fn main() {
    // Read once, here, so the library has no opinion about the filesystem and
    // a test can hand the server a folder of its own.
    let plugins = Installed::load();
    let (installed, enabled) = (plugins.installed(), plugins.enabled());
    let server = Server::with_plugins(plugins);
    let sections = server.document().sections.len();
    let shortcuts = server.inventory().shortcuts().len();
    eprintln!("{SERVER_NAME} {SERVER_VERSION}: {sections} sections, {shortcuts} shortcuts, {enabled} of {installed} plugins enabled, one JSON object per line on stdio");
    // The licence's notice for a program that prints no banner of its own: a
    // JSON-RPC server on a pipe has nowhere to put a copyright except stderr,
    // which is exactly where the line above goes.
    eprintln!(
        "Copyright (C) 2026 Stéphane Urdy. GNU General Public License, version 3: \
         there is ABSOLUTELY NO WARRANTY. The terms are in LICENSE."
    );

    let stdin = io::stdin();
    let stdout = io::stdout();
    if let Err(error) = server.run(stdin.lock(), stdout.lock()) {
        eprintln!("{SERVER_NAME}: {error}");
        std::process::exit(1);
    }
}
