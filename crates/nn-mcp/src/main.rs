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

    let stdin = io::stdin();
    let stdout = io::stdout();
    if let Err(error) = server.run(stdin.lock(), stdout.lock()) {
        eprintln!("{SERVER_NAME}: {error}");
        std::process::exit(1);
    }
}
