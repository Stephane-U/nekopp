//! `npp-mcp`: the Neko++ feature reference as a Model Context Protocol server.
//!
//! Started with no arguments. It reads JSON-RPC 2.0 from stdin, one object per
//! line, and writes one object per line to stdout; everything else -- the
//! startup line, an I/O failure -- goes to stderr, because stdout is the
//! protocol. A client starts it as `npp-mcp` and talks to it over a pipe, so
//! there is nothing to configure and no argument to get wrong.

use std::io;

use nn_mcp::{Server, SERVER_NAME, SERVER_VERSION};

fn main() {
    let server = Server::new();
    let sections = server.document().sections.len();
    let shortcuts = server.inventory().shortcuts().len();
    eprintln!("{SERVER_NAME} {SERVER_VERSION}: {sections} sections, {shortcuts} shortcuts, one JSON object per line on stdio");

    let stdin = io::stdin();
    let stdout = io::stdout();
    if let Err(error) = server.run(stdin.lock(), stdout.lock()) {
        eprintln!("{SERVER_NAME}: {error}");
        std::process::exit(1);
    }
}
