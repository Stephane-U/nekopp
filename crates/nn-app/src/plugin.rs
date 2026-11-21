//! Loading WebAssembly plugins and running their commands.
//!
//! The interpreter lives in `nn-core::wasm`; this module is the bridge between
//! it and the editor. Two things happen here. First, a plugin is given a
//! [`Bridge`] -- a scratch copy of the document, the caret, and a log -- rather
//! than the document itself, so a command that traps halfway through leaves the
//! editor's text untouched. Second, the bridge is read back afterwards and the
//! change applied in one step, which is what makes a plugin's edit a single
//! undo entry instead of a hundred.
//!
//! Plugins are discovered in `plugins/` under the configuration directory, one
//! `.wasm` file each. A file that fails to load is recorded rather than
//! skipped silently: a plugin that does not appear in the menu is a bug report
//! waiting to happen, and the log says which file and why.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use nn_core::wasm::abi;
use nn_core::wasm::{Error, Host, Plugin, Value};

/// The scratch state a plugin is given for one command.
///
/// The text is a copy. A plugin that runs off the end of it, or traps, or
/// loops until the fuel runs out, has changed nothing the editor can see --
/// the copy is dropped and the document is left as it was.
#[derive(Debug, Default, Clone)]
pub struct Bridge {
    pub text: String,
    pub caret: usize,
    pub read_only: bool,
    /// Lines the plugin asked to be logged, in order.
    pub log: Vec<String>,
    /// Whether the plugin replaced or inserted text.
    pub changed: bool,
    /// Whether the plugin moved the caret.
    pub caret_moved: bool,
}

impl Bridge {
    /// A bridge over `text` with the caret at `caret`.
    pub fn new(text: &str, caret: usize, read_only: bool) -> Bridge {
        Bridge {
            text: text.to_owned(),
            caret: caret.min(text.len()),
            read_only,
            ..Bridge::default()
        }
    }
}

/// The host functions, implemented over a [`Bridge`].
///
/// The bridge is shared rather than owned because the plugin owns its host: the
/// instance is built with a boxed `dyn Host`, so the only way for the caller to
/// see what the plugin did is to hold the other end of a shared cell.
pub struct DocumentHost {
    bridge: Rc<RefCell<Bridge>>,
}

impl DocumentHost {
    pub fn new(bridge: Rc<RefCell<Bridge>>) -> DocumentHost {
        DocumentHost { bridge }
    }
}

/// Reads `len` bytes at `ptr`, or reports that they are not there.
fn slice(memory: &[u8], ptr: u32, len: u32) -> Result<&[u8], Error> {
    let start = ptr as usize;
    let end = start.checked_add(len as usize).ok_or(Error::OutOfBounds)?;
    memory.get(start..end).ok_or(Error::OutOfBounds)
}

/// The largest offset a plugin may name. A `usize` on a 32-bit build would
/// wrap; clamping here keeps the arithmetic honest on both.
fn offset(value: i32) -> usize {
    value.max(0) as usize
}

impl Host for DocumentHost {
    fn call(
        &mut self,
        name: &str,
        args: &[Value],
        memory: &mut [u8],
    ) -> Result<Option<Value>, Error> {
        let mut bridge = self.bridge.borrow_mut();
        let arg = |n: usize| -> Result<i32, Error> {
            args.get(n)
                .copied()
                .and_then(Value::as_i32)
                .ok_or_else(|| Error::Host(format!("`{name}` is missing argument {n}")))
        };
        match name {
            abi::LOG => {
                let bytes = slice(memory, arg(0)? as u32, arg(1)? as u32)?;
                let line = String::from_utf8_lossy(bytes).into_owned();
                bridge.log.push(line);
                Ok(None)
            }
            abi::TEXT_LEN => Ok(Some(Value::I32(bridge.text.len() as i32))),
            abi::TEXT_READ => {
                let from = offset(arg(0)?);
                let ptr = arg(1)? as u32;
                let len = arg(2)? as u32;
                let available = bridge.text.len().saturating_sub(from);
                let n = available.min(len as usize);
                // The document is UTF-8 and the plugin asked for a byte range,
                // so a range that starts mid-character is widened back to the
                // character boundary rather than handing over a broken
                // sequence.
                let start = floor_boundary(&bridge.text, from);
                let bytes = bridge.text.as_bytes();
                let copied = abi::write_bytes(memory, ptr, &bytes[start..start + (from - start) + n]);
                Ok(Some(Value::I32(copied as i32)))
            }
            abi::TEXT_REPLACE => {
                if bridge.read_only {
                    return Ok(Some(Value::I32(1)));
                }
                let bytes = slice(memory, arg(0)? as u32, arg(1)? as u32)?;
                match std::str::from_utf8(bytes) {
                    Ok(text) => {
                        bridge.text = text.to_owned();
                        bridge.changed = true;
                        Ok(Some(Value::I32(0)))
                    }
                    Err(_) => Ok(Some(Value::I32(2))),
                }
            }
            abi::TEXT_INSERT => {
                if bridge.read_only {
                    return Ok(Some(Value::I32(1)));
                }
                let at = floor_boundary(&bridge.text, offset(arg(0)?));
                let bytes = slice(memory, arg(1)? as u32, arg(2)? as u32)?;
                match std::str::from_utf8(bytes) {
                    Ok(text) => {
                        bridge.text.insert_str(at, text);
                        bridge.changed = true;
                        Ok(Some(Value::I32(0)))
                    }
                    Err(_) => Ok(Some(Value::I32(2))),
                }
            }
            abi::CARET => Ok(Some(Value::I32(bridge.caret as i32))),
            abi::SET_CARET => {
                bridge.caret = floor_boundary(&bridge.text, offset(arg(0)?));
                bridge.caret_moved = true;
                Ok(None)
            }
            other => Err(Error::Host(format!("unknown host function `{other}`"))),
        }
    }
}

/// The largest character boundary at or below `at`.
fn floor_boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// What a command did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outcome {
    /// The new document text, if the plugin changed it.
    pub text: Option<String>,
    /// The new caret offset, if the plugin moved it.
    pub caret: Option<usize>,
    /// Whatever the plugin logged.
    pub log: Vec<String>,
}

/// One row of the Plugins menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub plugin: usize,
    pub index: usize,
    pub label: String,
}

struct Loaded {
    plugin: Plugin,
    bridge: Rc<RefCell<Bridge>>,
    path: PathBuf,
}

/// Every plugin that loaded, and every file that did not.
#[derive(Default)]
pub struct Plugins {
    loaded: Vec<Loaded>,
    failures: Vec<(PathBuf, String)>,
}

impl Plugins {
    /// Loads every `.wasm` file in `dir`, in file-name order.
    ///
    /// A missing directory is not an error: most installations have no
    /// plugins, and an editor that complained about that on every start would
    /// be wrong.
    pub fn load_dir(dir: &Path) -> Plugins {
        let mut plugins = Plugins::default();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return plugins;
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("wasm"))
            })
            .collect();
        paths.sort();
        for path in paths {
            let stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("plugin")
                .to_owned();
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let bridge = Rc::new(RefCell::new(Bridge::default()));
                    let host = DocumentHost::new(Rc::clone(&bridge));
                    match Plugin::load(&bytes, &stem, Box::new(host)) {
                        Ok(plugin) => plugins.loaded.push(Loaded { plugin, bridge, path }),
                        Err(error) => plugins.failures.push((path, error.to_string())),
                    }
                }
                Err(error) => plugins.failures.push((path, error.to_string())),
            }
        }
        plugins
    }

    pub fn is_empty(&self) -> bool {
        self.loaded.is_empty()
    }

    pub fn len(&self) -> usize {
        self.loaded.len()
    }

    /// The files that did not load, with the reason.
    pub fn failures(&self) -> &[(PathBuf, String)] {
        &self.failures
    }

    /// Every command of every plugin, in load order.
    pub fn commands(&self) -> Vec<Command> {
        let mut out = Vec::new();
        for (plugin, loaded) in self.loaded.iter().enumerate() {
            for (index, name) in loaded.plugin.commands().iter().enumerate() {
                out.push(Command {
                    plugin,
                    index,
                    label: format!("{} - {name}", loaded.plugin.name()),
                });
            }
        }
        out
    }

    /// The menu labels, which is what the menu bar wants.
    pub fn labels(&self) -> Vec<String> {
        self.commands().into_iter().map(|command| command.label).collect()
    }

    /// One line per plugin, for the log.
    pub fn describe(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .loaded
            .iter()
            .map(|loaded| {
                format!(
                    "{}: {}",
                    loaded.path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                    loaded.plugin.describe()
                )
            })
            .collect();
        for (path, error) in &self.failures {
            out.push(format!(
                "{}: {error}",
                path.file_name().and_then(|n| n.to_str()).unwrap_or("?")
            ));
        }
        out
    }

    /// Runs one command against a copy of `text`.
    ///
    /// The bridge is reset before the call and read back after it, so a plugin
    /// never sees state left by the previous command and the caller never sees
    /// a half-applied edit.
    pub fn run(
        &mut self,
        plugin: usize,
        command: usize,
        text: &str,
        caret: usize,
        read_only: bool,
    ) -> Result<Outcome, Error> {
        let loaded = self
            .loaded
            .get_mut(plugin)
            .ok_or_else(|| Error::Host(format!("no plugin {plugin}")))?;
        *loaded.bridge.borrow_mut() = Bridge::new(text, caret, read_only);
        loaded.plugin.run(command)?;
        let bridge = loaded.bridge.borrow();
        Ok(Outcome {
            text: bridge.changed.then(|| bridge.text.clone()),
            caret: bridge.caret_moved.then_some(bridge.caret),
            log: bridge.log.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn const_i32(value: i32) -> Vec<u8> {
        let mut out = vec![0x41];
        out.extend(sleb(value));
        out
    }

    /// A plugin with two commands. Command 0 replaces the document with
    /// `REPLACED`; command 1 logs `REPLACED` and leaves the text alone.
    ///
    /// The data segment is laid out as `Test\0Uppercase\0Log\0REPLACED\0`, so
    /// the name is at 0, the first command name at 5, the second at 15, and the
    /// replacement text at 19.
    fn build() -> Vec<u8> {
        let mut data = Vec::new();
        for text in ["Test", "Uppercase", "Log", "REPLACED"] {
            data.extend_from_slice(text.as_bytes());
            data.push(0);
        }

        // 0: () -> i32, 1: (i32) -> i32, 2: (i32, i32) -> i32,
        // 3: (i32, i32) -> (), 4: (i32) -> ()
        let types = vec_of(&[
            vec![0x60, 0, 1, 0x7F],
            vec![0x60, 1, 0x7F, 1, 0x7F],
            vec![0x60, 2, 0x7F, 0x7F, 1, 0x7F],
            vec![0x60, 2, 0x7F, 0x7F, 0],
            vec![0x60, 1, 0x7F, 0],
        ]);

        // Imports: text_replace (type 2) then log (type 3).
        let mut imports = leb(2);
        for (name, ty) in [("text_replace", 2u32), ("log", 3)] {
            imports.extend(leb(abi::HOST_MODULE.len() as u32));
            imports.extend_from_slice(abi::HOST_MODULE.as_bytes());
            imports.extend(leb(name.len() as u32));
            imports.extend_from_slice(name.as_bytes());
            imports.push(0x00);
            imports.extend(leb(ty));
        }

        let mut name_body = const_i32(0);
        name_body.push(0x0B);

        let mut count_body = const_i32(2);
        count_body.push(0x0B);

        // local.get 0, i32.eqz, if (result i32) 5 else 15 end
        let mut command_body = vec![0x20, 0, 0x45, 0x04, 0x7F];
        command_body.extend(const_i32(5));
        command_body.push(0x05);
        command_body.extend(const_i32(15));
        command_body.push(0x0B);
        command_body.push(0x0B);

        // local.get 0, i32.eqz, if
        //   i32.const 19, i32.const 8, call 0, drop
        // else
        //   i32.const 19, i32.const 8, call 1
        // end
        let mut run_body = vec![0x20, 0, 0x45, 0x04, 0x40];
        run_body.extend(const_i32(19));
        run_body.extend(const_i32(8));
        run_body.extend_from_slice(&[0x10, 0, 0x1A]);
        run_body.push(0x05);
        run_body.extend(const_i32(19));
        run_body.extend(const_i32(8));
        run_body.extend_from_slice(&[0x10, 1]);
        run_body.push(0x0B);
        run_body.push(0x0B);

        let funcs = [(0u32, name_body), (0, count_body), (1, command_body), (4, run_body)];
        let function_section = vec_of(&funcs.iter().map(|(t, _)| leb(*t)).collect::<Vec<_>>());
        let code_section = vec_of(
            &funcs
                .iter()
                .map(|(_, body)| {
                    let mut locals = vec![0u8];
                    locals.extend_from_slice(body);
                    let mut out = leb(locals.len() as u32);
                    out.extend(locals);
                    out
                })
                .collect::<Vec<_>>(),
        );

        let exports = [
            (abi::EXPORT_NAME, 2u32),
            (abi::EXPORT_COMMAND_COUNT, 3),
            (abi::EXPORT_COMMAND_NAME, 4),
            (abi::EXPORT_RUN, 5),
        ];
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
        bytes.extend(section(2, imports));
        bytes.extend(section(3, function_section));
        bytes.extend(section(5, vec_of(&[vec![0x00, 1]])));
        bytes.extend(section(7, export_section));
        bytes.extend(section(10, code_section));
        bytes.extend(section(11, data_body));
        bytes
    }

    fn plugins() -> Plugins {
        let bridge = Rc::new(RefCell::new(Bridge::default()));
        let host = DocumentHost::new(Rc::clone(&bridge));
        let plugin = Plugin::load(&build(), "test", Box::new(host)).unwrap();
        Plugins {
            loaded: vec![Loaded {
                plugin,
                bridge,
                path: PathBuf::from("test.wasm"),
            }],
            failures: Vec::new(),
        }
    }

    #[test]
    fn a_plugin_lists_its_commands() {
        let plugins = plugins();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins.labels(), vec!["Test - Uppercase", "Test - Log"]);
    }

    #[test]
    fn a_command_that_replaces_the_text_reports_it() {
        let mut plugins = plugins();
        let outcome = plugins.run(0, 0, "original", 3, false).unwrap();
        assert_eq!(outcome.text.as_deref(), Some("REPLACED"));
        assert!(outcome.log.is_empty());
    }

    #[test]
    fn a_command_that_only_logs_leaves_the_text_alone() {
        let mut plugins = plugins();
        let outcome = plugins.run(0, 1, "original", 3, false).unwrap();
        assert_eq!(outcome.text, None);
        assert_eq!(outcome.log, vec!["REPLACED"]);
    }

    #[test]
    fn a_read_only_document_refuses_the_change() {
        let mut plugins = plugins();
        let outcome = plugins.run(0, 0, "original", 0, true).unwrap();
        assert_eq!(outcome.text, None);
    }

    #[test]
    fn the_bridge_is_reset_between_commands() {
        let mut plugins = plugins();
        plugins.run(0, 0, "first", 0, false).unwrap();
        let outcome = plugins.run(0, 1, "second", 0, false).unwrap();
        assert_eq!(outcome.text, None);
        assert_eq!(outcome.log, vec!["REPLACED"]);
    }

    #[test]
    fn a_missing_plugin_is_reported() {
        let mut plugins = plugins();
        assert!(matches!(plugins.run(9, 0, "", 0, false), Err(Error::Host(_))));
    }

    #[test]
    fn a_missing_directory_loads_nothing() {
        let plugins = Plugins::load_dir(Path::new("/nonexistent/plugins"));
        assert!(plugins.is_empty());
        assert!(plugins.failures().is_empty());
    }

    #[test]
    fn a_broken_file_is_recorded_rather_than_skipped() {
        let dir = std::env::temp_dir().join("npp-plugin-test-broken");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("broken.wasm"), b"not a module").unwrap();
        let plugins = Plugins::load_dir(&dir);
        assert!(plugins.is_empty());
        assert_eq!(plugins.failures().len(), 1);
        assert!(plugins.failures()[0].1.contains("WebAssembly"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_directory_of_plugins_loads_in_name_order() {
        let dir = std::env::temp_dir().join("npp-plugin-test-order");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.wasm"), build()).unwrap();
        std::fs::write(dir.join("a.wasm"), build()).unwrap();
        std::fs::write(dir.join("notes.txt"), b"ignored").unwrap();
        let plugins = Plugins::load_dir(&dir);
        assert_eq!(plugins.len(), 2);
        assert_eq!(plugins.failures().len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_caret_move_is_reported() {
        let mut bridge = Bridge::new("hello", 0, false);
        bridge.caret = 4;
        bridge.caret_moved = true;
        assert_eq!(bridge.caret, 4);
    }

    #[test]
    fn a_boundary_is_floored_to_a_character() {
        // "e" with an acute is two bytes, so offset 2 is inside it.
        let text = "e\u{301}x";
        assert_eq!(floor_boundary(text, 1), 1);
        assert_eq!(floor_boundary(text, 2), 1);
        assert_eq!(floor_boundary(text, 3), 3);
        assert_eq!(floor_boundary(text, 99), 4);
    }

    #[test]
    fn a_negative_offset_is_clamped_to_zero() {
        assert_eq!(offset(-5), 0);
        assert_eq!(offset(7), 7);
    }

    #[test]
    fn reading_past_the_end_of_memory_is_refused() {
        assert_eq!(slice(&[0u8; 4], 2, 4), Err(Error::OutOfBounds));
        assert_eq!(slice(&[0u8; 4], 0, 4), Ok(&[0u8; 4][..]));
    }
}
