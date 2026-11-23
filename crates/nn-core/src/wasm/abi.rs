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

//! The calling convention between the editor and a plugin.
//!
//! A plugin is a module that exports four functions and imports a handful. The
//! exports are how the editor asks what the plugin is called and what it can
//! do; the imports are the only way the plugin can reach the document. Nothing
//! else crosses the boundary, which is what makes the sandbox a sandbox: a
//! plugin cannot open a file, spawn a process, or read the clipboard unless a
//! host function here says it can.
//!
//! # Exports
//!
//! | export | signature | meaning |
//! |---|---|---|
//! | `npp_name` | `() -> i32` | pointer to a NUL-terminated UTF-8 name |
//! | `npp_command_count` | `() -> i32` | how many commands the plugin offers |
//! | `npp_command_name` | `(i32) -> i32` | pointer to command `i`'s name |
//! | `npp_run` | `(i32) -> ()` | run command `i` |
//!
//! Strings are NUL-terminated rather than length-prefixed because the plugin
//! author writes them in their own language's static data, where a C string is
//! the thing that already exists. A length prefix would mean every plugin
//! carries a small constructor to build one.
//!
//! # Imports
//!
//! All from the module `npp`. Every one takes and returns `i32`; a pointer is
//! an offset into the plugin's own memory, and a length is a byte count.
//!
//! | import | signature | meaning |
//! |---|---|---|
//! | `log` | `(ptr, len)` | append a line to the plugin log |
//! | `text_len` | `() -> i32` | the document's length in bytes |
//! | `text_read` | `(offset, ptr, len) -> i32` | copy document bytes into plugin memory |
//! | `text_replace` | `(ptr, len) -> i32` | replace the whole document |
//! | `text_insert` | `(pos, ptr, len) -> i32` | insert at a byte offset |
//! | `text_replace_range` | `(pos, len, ptr, len) -> i32` | replace `len` bytes at `pos` |
//! | `caret` | `() -> i32` | the caret's byte offset |
//! | `set_caret` | `(pos)` | move the caret |
//! | `line_count` | `() -> i32` | how many lines the document has |
//! | `line_start` | `(line) -> i32` | the byte offset of line `line`'s first byte |
//! | `line_len` | `(line) -> i32` | line `line`'s length in bytes, without its terminator |
//! | `line_of_pos` | `(pos) -> i32` | which line byte `pos` is on |
//! | `sel_start` | `() -> i32` | the selection's lower byte offset |
//! | `sel_end` | `() -> i32` | the selection's upper byte offset |
//! | `set_sel` | `(start, end)` | select from `start` to `end` |
//!
//! `text_read` returns how many bytes it copied, which is fewer than asked for
//! at the end of the document. `text_replace`, `text_insert` and
//! `text_replace_range` return zero on success and non-zero on refusal -- a
//! read-only document, or an offset past the end -- so a plugin can tell the
//! difference between "done" and "the editor said no" without a second call.
//!
//! # Everything is a byte offset
//!
//! Lengths and offsets are bytes, not characters, because that is what a plugin
//! written in C, Rust or Zig can measure for the UTF-8 it is handed. The editor
//! works in characters internally and converts at the boundary, which is the
//! one place that knowledge belongs: a plugin that mixed the two would corrupt
//! a document on its first non-ASCII line, and every port would have to get the
//! same conversion right.
//!
//! The line functions are the exception in the way they read: a *line index*
//! counts lines, as the name says. `line_start`, `line_len` and `line_of_pos`
//! are offered because a line-oriented command -- align this column, find the
//! shortest line -- should not have to copy the whole document to find out
//! where a line begins, and the intraline offsets those functions return are
//! bytes again.

use super::{Error, Host, Instance, Module, Value};

/// The import module every host function lives in.
pub const HOST_MODULE: &str = "npp";

pub const LOG: &str = "log";
pub const TEXT_LEN: &str = "text_len";
pub const TEXT_READ: &str = "text_read";
pub const TEXT_REPLACE: &str = "text_replace";
pub const TEXT_INSERT: &str = "text_insert";
pub const TEXT_REPLACE_RANGE: &str = "text_replace_range";
pub const CARET: &str = "caret";
pub const SET_CARET: &str = "set_caret";
pub const LINE_COUNT: &str = "line_count";
pub const LINE_START: &str = "line_start";
pub const LINE_LEN: &str = "line_len";
pub const LINE_OF_POS: &str = "line_of_pos";
pub const SEL_START: &str = "sel_start";
pub const SEL_END: &str = "sel_end";
pub const SET_SEL: &str = "set_sel";

/// Every host function, in the order the documentation lists them. The app's
/// host implementation matches on these, and a test asserts the list and the
/// implementation agree.
///
/// Additions are made for a plugin that exists, not in anticipation of one: the
/// list is a promise the host has to keep, and a function nothing calls is a
/// function whose behaviour will be discovered by the first plugin that tries
/// it. The line and selection calls are here because the three ports that came
/// with this version needed them, and for no other reason.
pub const HOST_FUNCTIONS: &[&str] = &[
    LOG,
    TEXT_LEN,
    TEXT_READ,
    TEXT_REPLACE,
    TEXT_INSERT,
    TEXT_REPLACE_RANGE,
    CARET,
    SET_CARET,
    LINE_COUNT,
    LINE_START,
    LINE_LEN,
    LINE_OF_POS,
    SEL_START,
    SEL_END,
    SET_SEL,
];

pub const EXPORT_NAME: &str = "npp_name";
pub const EXPORT_COMMAND_COUNT: &str = "npp_command_count";
pub const EXPORT_COMMAND_NAME: &str = "npp_command_name";
pub const EXPORT_RUN: &str = "npp_run";

/// The longest string a plugin may hand back. A name is a label in a menu; a
/// plugin that wants to return a kilobyte of text has misunderstood the
/// interface, and reading until a NUL that never comes would hang the editor.
pub const MAX_STRING: usize = 256;

/// Reads a NUL-terminated UTF-8 string out of a plugin's memory.
///
/// Returns `None` for a pointer outside the memory, a string with no NUL
/// within [`MAX_STRING`], or bytes that are not UTF-8. All three are the same
/// answer to the caller -- there is no name here -- and distinguishing them
/// would only produce error messages about a plugin that is already broken.
pub fn read_string(memory: &[u8], ptr: u32) -> Option<String> {
    let start = ptr as usize;
    let rest = memory.get(start..)?;
    let end = rest.iter().take(MAX_STRING).position(|&b| b == 0)?;
    std::str::from_utf8(&rest[..end]).ok().map(str::to_owned)
}

/// Copies `bytes` into a plugin's memory at `ptr`, returning how many fitted.
///
/// Used by the host side of `text_read`: the plugin says where it wants the
/// bytes and how many it can take, and gets back the count it actually got.
pub fn write_bytes(memory: &mut [u8], ptr: u32, bytes: &[u8]) -> usize {
    let start = ptr as usize;
    let Some(slot) = memory.get_mut(start..) else {
        return 0;
    };
    let n = slot.len().min(bytes.len());
    slot[..n].copy_from_slice(&bytes[..n]);
    n
}

/// A loaded plugin: an instance plus the name and commands read out of it.
pub struct Plugin {
    instance: Instance,
    name: String,
    commands: Vec<String>,
}

impl Plugin {
    /// Loads a plugin from its bytes.
    ///
    /// `fallback` names the plugin when it does not export `npp_name` -- the
    /// file's stem, in practice. A plugin with no name is still usable; a
    /// plugin with no `npp_run` is not, and that is the one thing this
    /// refuses.
    pub fn load(bytes: &[u8], fallback: &str, host: Box<dyn Host>) -> Result<Plugin, Error> {
        let module = Module::parse(bytes)?;
        if module.export_func(EXPORT_RUN).is_none() {
            let offered = module.func_exports();
            return Err(Error::Host(format!(
                "the plugin does not export `{EXPORT_RUN}`; it offers {}",
                if offered.is_empty() {
                    "nothing".to_string()
                } else {
                    offered.join(", ")
                }
            )));
        }
        let mut instance = Instance::new(module, host)?;
        let name = instance
            .call_export(EXPORT_NAME, &[])
            .ok()
            .and_then(|values| values.first().copied())
            .and_then(Value::as_u32)
            .and_then(|ptr| read_string(instance.memory(), ptr))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| fallback.to_owned());
        let count = instance
            .call_export(EXPORT_COMMAND_COUNT, &[])
            .ok()
            .and_then(|values| values.first().copied())
            .and_then(Value::as_i32)
            .unwrap_or(0)
            .clamp(0, 64);
        let mut commands = Vec::new();
        for index in 0..count {
            let name = instance
                .call_export(EXPORT_COMMAND_NAME, &[Value::I32(index)])
                .ok()
                .and_then(|values| values.first().copied())
                .and_then(Value::as_u32)
                .and_then(|ptr| read_string(instance.memory(), ptr));
            if let Some(name) = name {
                commands.push(name);
            }
        }
        Ok(Plugin { instance, name, commands })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn commands(&self) -> &[String] {
        &self.commands
    }

    /// Runs command `index`. The index is checked against the list read at
    /// load, so a stale menu entry cannot ask for a command that is not there.
    pub fn run(&mut self, index: usize) -> Result<(), Error> {
        if index >= self.commands.len() {
            return Err(Error::Host(format!(
                "`{}` has no command {index}",
                self.name
            )));
        }
        self.instance.call_export(EXPORT_RUN, &[Value::I32(index as i32)])?;
        Ok(())
    }

    /// The instance, for the host to inspect after a command has run.
    pub fn instance(&self) -> &Instance {
        &self.instance
    }

    pub fn instance_mut(&mut self) -> &mut Instance {
        &mut self.instance
    }

    /// One line for the plugin log: what it is called and what it offers.
    pub fn describe(&self) -> String {
        match self.commands.len() {
            0 => format!("{} (no commands)", self.name),
            1 => format!("{} (1 command: {})", self.name, self.commands[0]),
            n => format!("{} ({n} commands: {})", self.name, self.commands.join(", ")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wasm::NullHost;

    fn leb(mut value: u32) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (value & 0x7F) as u8;
            value >>= 7;
            if value == 0 {
                out.push(byte);
                return out;
            }
            out.push(byte | 0x80);
        }
    }

    fn sleb(mut value: i32) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (value & 0x7F) as u8;
            value >>= 7;
            let done = (value == 0 && byte & 0x40 == 0) || (value == -1 && byte & 0x40 != 0);
            if done {
                out.push(byte);
                return out;
            }
            out.push(byte | 0x80);
        }
    }

    fn section(id: u8, body: Vec<u8>) -> Vec<u8> {
        let mut out = vec![id];
        out.extend(leb(body.len() as u32));
        out.extend(body);
        out
    }

    fn vec_of(items: &[Vec<u8>]) -> Vec<u8> {
        let mut out = leb(items.len() as u32);
        for item in items {
            out.extend_from_slice(item);
        }
        out
    }

    /// Builds a plugin module exporting the four entry points, with `name` and
    /// `commands` in its data segment. `with_run` is false for the test that
    /// checks a plugin without an entry point is refused.
    fn build(name: &str, commands: &[&str], with_run: bool) -> Vec<u8> {
        let mut data = Vec::new();
        let mut name_at = 0u32;
        let mut command_at = Vec::new();
        for (i, text) in std::iter::once(name).chain(commands.iter().copied()).enumerate() {
            let at = data.len() as u32;
            data.extend_from_slice(text.as_bytes());
            data.push(0);
            if i == 0 {
                name_at = at;
            } else {
                command_at.push(at);
            }
        }

        // type 0: () -> i32, type 1: (i32) -> i32, type 2: (i32) -> ()
        let types = vec_of(&[
            vec![0x60, 0, 1, 0x7F],
            vec![0x60, 1, 0x7F, 1, 0x7F],
            vec![0x60, 1, 0x7F, 0],
        ]);

        let mut name_body = vec![0x41];
        name_body.extend(sleb(name_at as i32));
        name_body.push(0x0B);

        let mut count_body = vec![0x41];
        count_body.extend(sleb(commands.len() as i32));
        count_body.push(0x0B);

        // A chain of `if (result i32)` comparing the argument to each index.
        fn chain(at: &[u32], depth: usize) -> Vec<u8> {
            if depth >= at.len() {
                return vec![0x41, 0];
            }
            let mut body = vec![0x20, 0, 0x41];
            body.extend(sleb(depth as i32));
            body.push(0x46); // i32.eq
            body.extend_from_slice(&[0x04, 0x7F]); // if (result i32)
            body.push(0x41);
            body.extend(sleb(at[depth] as i32));
            body.push(0x05); // else
            body.extend(chain(at, depth + 1));
            body.push(0x0B); // end
            body
        }
        let mut command_body = chain(&command_at, 0);
        command_body.push(0x0B);

        let mut funcs = vec![
            (0u32, name_body),
            (0, count_body),
            (1, command_body),
        ];
        if with_run {
            funcs.push((2, vec![0x0B]));
        }
        let function_section = vec_of(&funcs.iter().map(|(t, _)| leb(*t)).collect::<Vec<_>>());
        let code_section = vec_of(
            &funcs
                .iter()
                .map(|(_, body)| {
                    let mut locals = vec![0u8]; // no local groups
                    locals.extend_from_slice(body);
                    let mut out = leb(locals.len() as u32);
                    out.extend(locals);
                    out
                })
                .collect::<Vec<_>>(),
        );

        let mut exports = vec![
            (EXPORT_NAME, 0u32),
            (EXPORT_COMMAND_COUNT, 1),
            (EXPORT_COMMAND_NAME, 2),
        ];
        if with_run {
            exports.push((EXPORT_RUN, 3));
        }
        let export_section = vec_of(
            &exports
                .iter()
                .map(|(name, index)| {
                    let mut out = leb(name.len() as u32);
                    out.extend_from_slice(name.as_bytes());
                    out.push(0x00);
                    out.extend(leb(*index));
                    out
                })
                .collect::<Vec<_>>(),
        );

        let mut data_body = leb(1);
        data_body.extend_from_slice(&[0x00, 0x41, 0x00, 0x0B]);
        data_body.extend(leb(data.len() as u32));
        data_body.extend_from_slice(&data);

        let mut bytes = b"\0asm".to_vec();
        bytes.extend_from_slice(&[1, 0, 0, 0]);
        bytes.extend(section(1, types));
        bytes.extend(section(3, function_section));
        bytes.extend(section(5, vec_of(&[vec![0x00, 1]])));
        bytes.extend(section(7, export_section));
        bytes.extend(section(10, code_section));
        bytes.extend(section(11, data_body));
        bytes
    }

    #[test]
    fn a_plugin_reports_its_name_and_commands() {
        let bytes = build("Logfile Tools", &["Sort by time", "Strip blank lines"], true);
        let plugin = Plugin::load(&bytes, "fallback", Box::new(NullHost::default())).unwrap();
        assert_eq!(plugin.name(), "Logfile Tools");
        assert_eq!(plugin.commands(), ["Sort by time", "Strip blank lines"]);
    }

    #[test]
    fn a_plugin_without_a_name_uses_the_fallback() {
        let bytes = build("", &["Only"], true);
        let plugin = Plugin::load(&bytes, "logtool", Box::new(NullHost::default())).unwrap();
        assert_eq!(plugin.name(), "logtool");
    }

    #[test]
    fn a_plugin_without_an_entry_point_is_refused() {
        let bytes = build("Broken", &["Nothing"], false);
        match Plugin::load(&bytes, "broken", Box::new(NullHost::default())) {
            Err(Error::Host(message)) => assert!(message.contains(EXPORT_RUN)),
            Ok(plugin) => panic!("expected a refusal, got {}", plugin.describe()),
            Err(other) => panic!("expected a refusal, got {other}"),
        }
    }

    #[test]
    fn a_plugin_with_no_commands_loads() {
        let bytes = build("Quiet", &[], true);
        let plugin = Plugin::load(&bytes, "quiet", Box::new(NullHost::default())).unwrap();
        assert!(plugin.commands().is_empty());
        assert_eq!(plugin.describe(), "Quiet (no commands)");
    }

    #[test]
    fn running_a_command_that_does_not_exist_is_refused() {
        let bytes = build("Small", &["One"], true);
        let mut plugin = Plugin::load(&bytes, "small", Box::new(NullHost::default())).unwrap();
        assert!(plugin.run(0).is_ok());
        assert!(matches!(plugin.run(1), Err(Error::Host(_))));
    }

    #[test]
    fn describe_counts_the_commands() {
        let one = build("One", &["A"], true);
        let plugin = Plugin::load(&one, "one", Box::new(NullHost::default())).unwrap();
        assert_eq!(plugin.describe(), "One (1 command: A)");
        let two = build("Two", &["A", "B"], true);
        let plugin = Plugin::load(&two, "two", Box::new(NullHost::default())).unwrap();
        assert_eq!(plugin.describe(), "Two (2 commands: A, B)");
    }

    #[test]
    fn a_string_is_read_to_its_nul() {
        let memory = b"hello\0world\0";
        assert_eq!(read_string(memory, 0).as_deref(), Some("hello"));
        assert_eq!(read_string(memory, 6).as_deref(), Some("world"));
    }

    #[test]
    fn a_string_without_a_nul_is_refused() {
        let memory = vec![b'a'; MAX_STRING + 10];
        assert_eq!(read_string(&memory, 0), None);
    }

    #[test]
    fn a_string_outside_memory_is_refused() {
        assert_eq!(read_string(b"hi\0", 99), None);
    }

    #[test]
    fn a_string_that_is_not_utf8_is_refused() {
        assert_eq!(read_string(&[0xFF, 0xFE, 0], 0), None);
    }

    #[test]
    fn writing_bytes_stops_at_the_end_of_memory() {
        let mut memory = vec![0u8; 4];
        assert_eq!(write_bytes(&mut memory, 2, b"abcd"), 2);
        assert_eq!(memory, vec![0, 0, b'a', b'b']);
        assert_eq!(write_bytes(&mut memory, 9, b"x"), 0);
    }

    #[test]
    fn the_host_function_list_is_the_documented_one() {
        assert_eq!(HOST_FUNCTIONS.len(), 15);
        assert!(HOST_FUNCTIONS.contains(&LOG));
        assert!(HOST_FUNCTIONS.contains(&SET_CARET));
        assert!(HOST_FUNCTIONS.contains(&TEXT_REPLACE_RANGE));
        assert!(HOST_FUNCTIONS.contains(&SET_SEL));
        assert!(HOST_FUNCTIONS.contains(&LINE_OF_POS));
    }
}
