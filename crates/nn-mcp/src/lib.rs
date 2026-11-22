//! The Neko++ feature reference, and its plugins, served over the Model Context
//! Protocol.
//!
//! Neko++ is documented once, in `docs/nekopp-features.md`, and that file is both
//! the source of the PDF and the corpus this server answers from. Embedding it
//! with `include_str!` is what makes the two agree: a documentation server that
//! reads a file from a checkout is a server that answers with whatever version
//! happens to be on the machine.
//!
//! The crate is in four parts plus the protocol:
//!
//! - [`doc`] cuts the markdown into sections and gives each one a stable slug.
//! - [`commands`] joins the keymap table with the menu tree.
//! - [`search`] ranks sections against a query.
//! - [`plugins`] finds the editor's WebAssembly plugins and runs one of their
//!   commands over a string.
//! - [`server`] speaks JSON-RPC 2.0 over stdin and stdout.
//!
//! Nothing here depends on `nn-app` or on a window: the server is a separate
//! binary that can be started on a machine that has never run the editor. What
//! it shares with the editor is `nn-core`, because the plugin tools run the
//! commands the editor runs and a second implementation of the ABI would be a
//! second thing for a plugin to be wrong about.

pub mod commands;
pub mod doc;
pub mod plugins;
pub mod search;
pub mod server;

pub use commands::{Command, Inventory, MenuRow, Shortcut};
pub use plugins::Installed;
pub use doc::{Document, Section, FEATURES_MARKDOWN};
pub use server::{section_uri, Server, INDEX_URI, PROTOCOL_VERSION, SERVER_NAME, SERVER_VERSION};
