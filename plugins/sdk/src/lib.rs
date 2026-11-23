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

//! The SDK for Neko++ WebAssembly plugins.
//!
//! A plugin is a `.wasm` module that exports four functions the editor calls:
//! its name, how many commands it has, the name of each, and the entry point
//! that runs one. This crate writes those four for you -- [`plugin!`] generates
//! them -- and gives the body of a command a typed way to reach the document.
//!
//! ```
//! use nn_plugin::host;
//!
//! fn shout() {
//!     let text = host::text();
//!     host::replace(&text.to_uppercase());
//! }
//!
//! nn_plugin::plugin! {
//!     name: "Shout",
//!     commands: [
//!         ("Uppercase the document", shout),
//!     ],
//! }
//! ```
//!
//! # Offsets are bytes
//!
//! Every offset, length and selection position here is a byte offset into the
//! document's UTF-8, because that is the unit a plugin can measure for itself.
//! A command that walks characters has to map them; [`host::text`] and
//! [`host::line`] hand back `String`s, so the usual `char_indices` is enough.
//!
//! # What a plugin cannot do
//!
//! Read a file, write a file, open a window, reach the network, or keep state
//! between commands. The host functions are the whole surface, and the module
//! is interpreted with bounded memory and a step budget: a command that loops
//! forever is stopped, and one that traps leaves the document exactly as it
//! was, because it works on a copy that is only written back on success.
//!
//! # Building
//!
//! ```sh
//! cargo build --release --target wasm32-unknown-unknown
//! ```
//!
//! The `.wasm` in `target/wasm32-unknown-unknown/release/` is the plugin. Copy
//! it into the editor's `plugins` directory -- `Plugins > Open Plugins Folder`
//! -- switch it on in `Plugins > Manage Plugins...`, and `npp-mcp` can run each
//! of its commands over a document.

#![deny(missing_docs)]

/// The host functions, typed.
///
/// The wrappers here are thin on purpose: a plugin that needs to think about
/// refusals asks [`replace_range`] whether it worked, and one that does not can
/// call [`replace`] and move on.
pub mod host {
    use std::ops::Range;

    /// The raw imports. Every one is `i32`-to-`i32`, which is what the host
    /// side of the ABI is written against.
    #[cfg(target_arch = "wasm32")]
    mod raw {
        #[link(wasm_import_module = "npp")]
        extern "C" {
            pub fn log(ptr: i32, len: i32);
            pub fn text_len() -> i32;
            pub fn text_read(offset: i32, ptr: i32, len: i32) -> i32;
            pub fn text_replace(ptr: i32, len: i32) -> i32;
            pub fn text_insert(pos: i32, ptr: i32, len: i32) -> i32;
            pub fn text_replace_range(pos: i32, len: i32, ptr: i32, len2: i32) -> i32;
            pub fn caret() -> i32;
            pub fn set_caret(pos: i32);
            pub fn line_count() -> i32;
            pub fn line_start(line: i32) -> i32;
            pub fn line_len(line: i32) -> i32;
            pub fn line_of_pos(pos: i32) -> i32;
            pub fn sel_start() -> i32;
            pub fn sel_end() -> i32;
            pub fn set_sel(start: i32, end: i32);
        }
    }

    /// The same functions on the host, so that a plugin crate builds and its
    /// tests run on a developer's machine.
    ///
    /// None of them can do anything: there is no editor. Panicking rather than
    /// answering zero means a test that strays into the host finds out, instead
    /// of asserting against an empty document forever.
    #[cfg(not(target_arch = "wasm32"))]
    mod raw {
        macro_rules! unavailable {
            ($($name:ident($($arg:ident: $ty:ty),*) $(-> $ret:ty)?;)*) => {
                $(
                    pub unsafe fn $name($(_: $ty),*) $(-> $ret)? {
                        panic!(concat!(
                            "`", stringify!($name),
                            "` needs the editor; only a plugin running inside Neko++ can call it"
                        ));
                    }
                )*
            };
        }
        unavailable! {
            log(ptr: i32, len: i32);
            text_len() -> i32;
            text_read(offset: i32, ptr: i32, len: i32) -> i32;
            text_replace(ptr: i32, len: i32) -> i32;
            text_insert(pos: i32, ptr: i32, len: i32) -> i32;
            text_replace_range(pos: i32, len: i32, ptr: i32, len2: i32) -> i32;
            caret() -> i32;
            set_caret(pos: i32);
            line_count() -> i32;
            line_start(line: i32) -> i32;
            line_len(line: i32) -> i32;
            line_of_pos(pos: i32) -> i32;
            sel_start() -> i32;
            sel_end() -> i32;
            set_sel(start: i32, end: i32);
        }
    }

    /// How many bytes the document has.
    pub fn text_len() -> usize {
        unsafe { raw::text_len() }.max(0) as usize
    }

    /// The whole document.
    ///
    /// One call, one copy. A command that only needs a line should ask for a
    /// line: on a large file the difference is a megabyte of plugin memory.
    pub fn text() -> String {
        let len = text_len();
        let mut buffer = vec![0u8; len];
        let copied = unsafe { raw::text_read(0, buffer.as_mut_ptr() as i32, len as i32) };
        buffer.truncate(copied.max(0) as usize);
        String::from_utf8_lossy(&buffer).into_owned()
    }

    /// `len` bytes of the document starting at `offset`.
    pub fn text_in(offset: usize, len: usize) -> String {
        let mut buffer = vec![0u8; len];
        let copied =
            unsafe { raw::text_read(offset as i32, buffer.as_mut_ptr() as i32, len as i32) };
        buffer.truncate(copied.max(0) as usize);
        String::from_utf8_lossy(&buffer).into_owned()
    }

    /// The text between two byte offsets.
    pub fn range(range: Range<usize>) -> String {
        let start = range.start.min(range.end);
        text_in(start, range.end - start)
    }

    /// Replaces the whole document. False if it is read-only.
    pub fn replace(text: &str) -> bool {
        let code = unsafe { raw::text_replace(text.as_ptr() as i32, text.len() as i32) };
        code == 0
    }

    /// Inserts `text` at byte offset `pos`. False if the document is read-only.
    pub fn insert(pos: usize, text: &str) -> bool {
        let code = unsafe { raw::text_insert(pos as i32, text.as_ptr() as i32, text.len() as i32) };
        code == 0
    }

    /// Replaces `remove` bytes at `pos` with `text`.
    ///
    /// The one editing primitive here: a delete is a replacement with no text,
    /// an insert is a replacement of nothing, and a rename is both at once. Both
    /// ends of `remove` are widened to character boundaries by the host, so a
    /// range that names half a character does not corrupt the document.
    pub fn replace_range(pos: usize, remove: usize, text: &str) -> bool {
        let code = unsafe {
            raw::text_replace_range(
                pos as i32,
                remove as i32,
                text.as_ptr() as i32,
                text.len() as i32,
            )
        };
        code == 0
    }

    /// The caret's byte offset.
    pub fn caret() -> usize {
        unsafe { raw::caret() }.max(0) as usize
    }

    /// Moves the caret.
    pub fn set_caret(pos: usize) {
        unsafe { raw::set_caret(pos as i32) }
    }

    /// How many lines the document has.
    pub fn line_count() -> usize {
        unsafe { raw::line_count() }.max(0) as usize
    }

    /// The byte offset of line `line`'s first byte.
    pub fn line_start(line: usize) -> usize {
        unsafe { raw::line_start(line as i32) }.max(0) as usize
    }

    /// Line `line`'s length in bytes, without its terminator.
    pub fn line_len(line: usize) -> usize {
        unsafe { raw::line_len(line as i32) }.max(0) as usize
    }

    /// Line `line`, without its terminator.
    pub fn line(line: usize) -> String {
        text_in(line_start(line), line_len(line))
    }

    /// Which line a byte offset falls on.
    pub fn line_of_pos(pos: usize) -> usize {
        unsafe { raw::line_of_pos(pos as i32) }.max(0) as usize
    }

    /// Every line, without terminators.
    pub fn lines() -> Vec<String> {
        (0..line_count()).map(line).collect()
    }

    /// The primary selection as byte offsets, lower first. Empty when nothing
    /// is selected.
    pub fn selection() -> Range<usize> {
        let start = unsafe { raw::sel_start() }.max(0) as usize;
        let end = unsafe { raw::sel_end() }.max(0) as usize;
        start.min(end)..start.max(end)
    }

    /// Selects from `start` to `end`. Either order is accepted.
    pub fn select(start: usize, end: usize) {
        unsafe { raw::set_sel(start as i32, end as i32) }
    }

    /// Appends a line to the plugin log.
    ///
    /// The editor writes it to its standard error as `[plugin] ...`, which is
    /// where a command that has something to say and no menu to say it in puts
    /// it. There is no return value because there is no failure: a line too long
    /// for the log is the host's problem, not the plugin's.
    pub fn log(message: &str) {
        unsafe { raw::log(message.as_ptr() as i32, message.len() as i32) }
    }
}

/// The machinery behind [`plugin!`], not meant to be called directly.
pub mod export {
    use std::cell::UnsafeCell;

    /// Where a name is copied so the host can read it.
    ///
    /// The exports answer with a pointer to a NUL-terminated string, and a Rust
    /// `&str` has no NUL at the end. Copying into one shared buffer is the whole
    /// trick: the host reads the pointer before the next call, and a plugin has
    /// one name and a handful of commands, so 128 bytes is not a ration.
    struct Buffer(UnsafeCell<[u8; 128]>);

    // Safe because a plugin is single-threaded: the host calls into it, and a
    // wasm module has no threads unless it asks for them.
    unsafe impl Sync for Buffer {}

    static BUFFER: Buffer = Buffer(UnsafeCell::new([0; 128]));

    /// Copies `text` into the shared buffer, NUL-terminated, and answers its
    /// address.
    fn c_string(text: &str) -> i32 {
        // Truncated at a character boundary: half a character in a menu label
        // is a replacement glyph, and the host would refuse the bytes anyway.
        let mut end = text.len().min(127);
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        let bytes = &text.as_bytes()[..end];
        unsafe {
            let buffer = &mut *BUFFER.0.get();
            buffer[..bytes.len()].copy_from_slice(bytes);
            buffer[bytes.len()] = 0;
            buffer.as_ptr() as i32
        }
    }

    /// The address of `name` as a C string.
    pub fn name(name: &str) -> i32 {
        c_string(name)
    }

    /// The address of command `index`'s label, or an empty string past the end.
    pub fn command(labels: &[&str], index: i32) -> i32 {
        match labels.get(index.max(0) as usize) {
            Some(label) => c_string(label),
            None => c_string(""),
        }
    }
}

/// Declares a plugin: its name and its commands.
///
/// ```
/// use nn_plugin::host;
///
/// fn trim() {
///     let lines: Vec<String> = host::lines().iter().map(|l| l.trim_end().to_string()).collect();
///     host::replace(&lines.join("\n"));
/// }
///
/// nn_plugin::plugin! {
///     name: "Whitespace",
///     commands: [
///         ("Trim trailing spaces", trim),
///     ],
/// }
/// ```
///
/// This generates the four functions the editor looks for. Nothing else in the
/// crate has to be `extern "C"`; a command is an ordinary Rust function taking
/// nothing and returning nothing, and what it does to the document it does
/// through [`host`].
#[macro_export]
macro_rules! plugin {
    (
        name: $name:expr,
        commands: [
            $( ($label:expr, $run:path) ),* $(,)?
        ] $(,)?
    ) => {
        /// The commands this plugin offers, as the macro was given them.
        #[doc(hidden)]
        pub const COMMAND_LABELS: &[&str] = &[$($label),*];

        #[no_mangle]
        pub extern "C" fn npp_name() -> i32 {
            $crate::export::name($name)
        }

        #[no_mangle]
        pub extern "C" fn npp_command_count() -> i32 {
            COMMAND_LABELS.len() as i32
        }

        #[no_mangle]
        pub extern "C" fn npp_command_name(index: i32) -> i32 {
            $crate::export::command(COMMAND_LABELS, index)
        }

        #[no_mangle]
        pub extern "C" fn npp_run(index: i32) {
            let runners: &[fn()] = &[$($run),*];
            if let Some(run) = runners.get(index.max(0) as usize) {
                run();
            }
        }
    };
}
