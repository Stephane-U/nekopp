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
//!
//! Loading is not a startup step. [`Plugins::unload`] takes a plugin out of the
//! menu and drops its module, and [`Plugins::load`] reads it back; the directory
//! is the list of what is installed, and the editor's idea of what is *running*
//! is a second, narrower list that a user can edit without restarting.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use nn_core::wasm::abi;
use nn_core::wasm::{Error, Host, Plugin, Value};

/// What a plugin is given to work on: the document, and where the user is in it.
///
/// Every offset here is a byte offset, because that is the unit the host
/// functions speak. The app holds character offsets and converts, which is why
/// this type exists rather than a handful of arguments: the conversion has to
/// happen in one place, or the first non-ASCII document teaches it to everyone
/// separately.
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    pub text: &'a str,
    /// The caret's byte offset.
    pub caret: usize,
    /// The primary selection's byte offsets, lower first. Equal for a caret with
    /// nothing selected.
    pub selection: (usize, usize),
    pub read_only: bool,
}

/// The scratch state a plugin is given for one command.
///
/// The text is a copy. A plugin that runs off the end of it, or traps, or
/// loops until the fuel runs out, has changed nothing the editor can see --
/// the copy is dropped and the document is left as it was.
#[derive(Debug, Default, Clone)]
pub struct Bridge {
    pub text: String,
    pub caret: usize,
    pub selection: (usize, usize),
    pub read_only: bool,
    /// Lines the plugin asked to be logged, in order.
    pub log: Vec<String>,
    /// Whether the plugin replaced or inserted text.
    pub changed: bool,
    /// Whether the plugin moved the caret.
    pub caret_moved: bool,
    /// Whether the plugin moved the selection.
    pub selection_moved: bool,
}

impl Bridge {
    /// A bridge over the document in `context`.
    pub fn new(context: &Context) -> Bridge {
        let caret = floor_boundary(context.text, context.caret);
        let (start, end) = context.selection;
        Bridge {
            text: context.text.to_owned(),
            caret,
            selection: (
                floor_boundary(context.text, start),
                floor_boundary(context.text, end),
            ),
            read_only: context.read_only,
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
            abi::TEXT_REPLACE_RANGE => {
                if bridge.read_only {
                    return Ok(Some(Value::I32(1)));
                }
                let from = offset(arg(0)?);
                let remove = offset(arg(1)?);
                let bytes = slice(memory, arg(2)? as u32, arg(3)? as u32)?;
                let Ok(text) = std::str::from_utf8(bytes) else {
                    return Ok(Some(Value::I32(2)));
                };
                // Both ends are snapped to character boundaries and clamped to
                // the document, so a range that names the middle of a character
                // widens to cover it rather than panicking on `String::replace
                // _range`.
                let start = floor_boundary(&bridge.text, from);
                let mut end = floor_boundary(&bridge.text, from.saturating_add(remove));
                end = end.max(start);
                bridge.text.replace_range(start..end, text);
                bridge.changed = true;
                Ok(Some(Value::I32(0)))
            }
            abi::LINE_COUNT => Ok(Some(Value::I32(line_count(&bridge.text) as i32))),
            abi::LINE_START => Ok(Some(Value::I32(
                line_start(&bridge.text, offset(arg(0)?)) as i32,
            ))),
            abi::LINE_LEN => Ok(Some(Value::I32(
                line_len(&bridge.text, offset(arg(0)?)) as i32,
            ))),
            abi::LINE_OF_POS => Ok(Some(Value::I32(
                line_of_pos(&bridge.text, offset(arg(0)?)) as i32,
            ))),
            abi::SEL_START => Ok(Some(Value::I32(bridge.selection.0 as i32))),
            abi::SEL_END => Ok(Some(Value::I32(bridge.selection.1 as i32))),
            abi::SET_SEL => {
                let start = floor_boundary(&bridge.text, offset(arg(0)?));
                let end = floor_boundary(&bridge.text, offset(arg(1)?));
                bridge.selection = (start.min(end), start.max(end));
                bridge.selection_moved = true;
                Ok(None)
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

/// How many lines `text` has.
///
/// The count a reader would make: the number of terminators plus one, so that a
/// document ending in a newline has a final empty line and an empty document has
/// one line rather than none. This is the editor's own answer -- `len_lines` on
/// the rope -- which is why it is computed here rather than left to the plugin
/// to derive from offsets.
fn line_count(text: &str) -> usize {
    text.bytes().filter(|&b| b == b'\n').count() + 1
}

/// The byte offset of the first byte of line `line`, clamped to the last line.
fn line_start(text: &str, line: usize) -> usize {
    let mut start = 0;
    for _ in 0..line {
        match text[start..].find('\n') {
            Some(at) => start += at + 1,
            None => return text.len(),
        }
    }
    start
}

/// The byte length of line `line`, without its terminator.
///
/// Both endings are stripped: a document read from disk may still carry
/// `\r\n`, and a plugin asking for a line's length wants the text, not the
/// line discipline of the file it came from.
fn line_len(text: &str, line: usize) -> usize {
    let start = line_start(text, line);
    let rest = &text[start..];
    let end = rest.find('\n').unwrap_or(rest.len());
    rest[..end].trim_end_matches('\r').len()
}

/// Which line byte offset `pos` falls on.
fn line_of_pos(text: &str, pos: usize) -> usize {
    let pos = pos.min(text.len());
    text.as_bytes()[..pos].iter().filter(|&&b| b == b'\n').count()
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
///
/// Offsets are bytes, as everywhere on the plugin side of the boundary; the
/// caller converts back. A command that sets both a selection and a caret has
/// said two things that cannot both be true -- a selection's end *is* the caret
/// -- so the selection wins and the caret is ignored.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outcome {
    /// The new document text, if the plugin changed it.
    pub text: Option<String>,
    /// The new caret byte offset, if the plugin moved it.
    pub caret: Option<usize>,
    /// The new selection, if the plugin changed it.
    pub selection: Option<(usize, usize)>,
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

/// A plugin the user took out of the menu.
///
/// What is kept is the name and the file, and nothing else: the module, its
/// memory and its host bridge are dropped by [`Plugins::unload`], which is what
/// makes the word mean something. Reading the file back is the whole cost of
/// loading it again, and it has the useful side effect that a plugin rebuilt
/// since it was unloaded comes back as the new build.
struct Unloaded {
    name: String,
    path: PathBuf,
}

/// Every plugin that loaded, every one the user unloaded, and every file that
/// did not load at all.
#[derive(Default)]
pub struct Plugins {
    loaded: Vec<Loaded>,
    unloaded: Vec<Unloaded>,
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

    /// The name of every loaded plugin, in menu order.
    ///
    /// The names rather than the plugins: the menu needs to write "Unload
    /// Half Width" and has no business holding a module to do it.
    pub fn names(&self) -> Vec<String> {
        self.loaded
            .iter()
            .map(|loaded| loaded.plugin.name().to_owned())
            .collect()
    }

    /// The name of every plugin the user unloaded, in the order they went.
    pub fn unloaded(&self) -> Vec<String> {
        self.unloaded.iter().map(|entry| entry.name.clone()).collect()
    }

    /// Takes a plugin out of the menu and drops its module.
    ///
    /// Returns the name that went, or `None` for an index that is not there --
    /// which a menu row can still be holding if a plugin was unloaded while it
    /// was open. The module is dropped rather than parked, so an unloaded
    /// plugin holds no memory and cannot be run by accident; [`load`] puts it
    /// back from its file.
    ///
    /// [`load`]: Self::load
    pub fn unload(&mut self, index: usize) -> Option<String> {
        if index >= self.loaded.len() {
            return None;
        }
        let gone = self.loaded.remove(index);
        let name = gone.plugin.name().to_owned();
        self.unloaded.push(Unloaded {
            name: name.clone(),
            path: gone.path,
        });
        Some(name)
    }

    /// Reads an unloaded plugin back in, at the end of the menu order.
    ///
    /// A failure leaves it in the unloaded list, so the row that asked for it
    /// is still there to try again after the file is fixed.
    pub fn load(&mut self, index: usize) -> Result<String, String> {
        let Some(path) = self.unloaded.get(index).map(|entry| entry.path.clone()) else {
            return Err(format!("no unloaded plugin {index}"));
        };
        let fallback = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("plugin")
            .to_owned();
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                let message = error.to_string();
                self.failures.push((path, message.clone()));
                return Err(message);
            }
        };
        let bridge = Rc::new(RefCell::new(Bridge::default()));
        let host = DocumentHost::new(Rc::clone(&bridge));
        match Plugin::load(&bytes, &fallback, Box::new(host)) {
            Ok(plugin) => {
                let name = plugin.name().to_owned();
                self.loaded.push(Loaded { plugin, bridge, path });
                self.unloaded.remove(index);
                Ok(name)
            }
            Err(error) => {
                let message = error.to_string();
                self.failures.push((path, message.clone()));
                Err(message)
            }
        }
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
        context: &Context,
    ) -> Result<Outcome, Error> {
        let loaded = self
            .loaded
            .get_mut(plugin)
            .ok_or_else(|| Error::Host(format!("no plugin {plugin}")))?;
        *loaded.bridge.borrow_mut() = Bridge::new(context);
        loaded.plugin.run(command)?;
        let bridge = loaded.bridge.borrow();
        Ok(Outcome {
            text: bridge.changed.then(|| bridge.text.clone()),
            caret: bridge.caret_moved.then_some(bridge.caret),
            selection: bridge.selection_moved.then_some(bridge.selection),
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
            unloaded: Vec::new(),
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
        let outcome = plugins.run(0, 0, &doc("original", 3, (3, 3))).unwrap();
        assert_eq!(outcome.text.as_deref(), Some("REPLACED"));
        assert!(outcome.log.is_empty());
    }

    #[test]
    fn a_command_that_only_logs_leaves_the_text_alone() {
        let mut plugins = plugins();
        let outcome = plugins.run(0, 1, &doc("original", 3, (3, 3))).unwrap();
        assert_eq!(outcome.text, None);
        assert_eq!(outcome.log, vec!["REPLACED"]);
    }

    #[test]
    fn a_read_only_document_refuses_the_change() {
        let mut plugins = plugins();
        let outcome = plugins.run(0, 0, &read_only("original")).unwrap();
        assert_eq!(outcome.text, None);
    }

    #[test]
    fn the_bridge_is_reset_between_commands() {
        let mut plugins = plugins();
        plugins.run(0, 0, &doc("first", 0, (0, 0))).unwrap();
        let outcome = plugins.run(0, 1, &doc("second", 0, (0, 0))).unwrap();
        assert_eq!(outcome.text, None);
        assert_eq!(outcome.log, vec!["REPLACED"]);
    }

    #[test]
    fn a_missing_plugin_is_reported() {
        let mut plugins = plugins();
        assert!(matches!(
            plugins.run(9, 0, &doc("", 0, (0, 0))),
            Err(Error::Host(_))
        ));
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
    fn unloading_a_plugin_takes_it_out_of_the_menu() {
        let mut plugins = plugins();
        assert_eq!(plugins.names(), vec!["Test".to_string()]);
        assert_eq!(plugins.unload(0).as_deref(), Some("Test"));
        assert!(plugins.is_empty());
        assert!(plugins.commands().is_empty());
        assert_eq!(plugins.unloaded(), vec!["Test".to_string()]);
        // The row that asked to unload it is a click behind the menu now, so an
        // index that is no longer there has to be refused rather than panic.
        assert_eq!(plugins.unload(0), None);
    }

    #[test]
    fn a_plugin_loads_back_from_its_file() {
        let dir = std::env::temp_dir().join("npp-plugin-test-unload");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.wasm");
        std::fs::write(&path, build()).unwrap();
        let mut plugins = Plugins::load_dir(&dir);
        assert_eq!(plugins.len(), 1);

        assert_eq!(plugins.unload(0).as_deref(), Some("Test"));
        assert!(plugins.is_empty());
        assert_eq!(plugins.load(0), Ok("Test".to_string()));
        assert_eq!(plugins.len(), 1);
        assert!(plugins.unloaded().is_empty());
        assert_eq!(plugins.labels(), vec!["Test - Uppercase", "Test - Log"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_plugin_whose_file_is_gone_stays_unloaded() {
        let dir = std::env::temp_dir().join("npp-plugin-test-gone");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.wasm");
        std::fs::write(&path, build()).unwrap();
        let mut plugins = Plugins::load_dir(&dir);
        assert_eq!(plugins.unload(0).as_deref(), Some("Test"));
        std::fs::remove_file(&path).unwrap();

        assert!(plugins.load(0).is_err());
        // Still offered, because the file may be put back.
        assert_eq!(plugins.unloaded(), vec!["Test".to_string()]);
        assert_eq!(plugins.failures().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loading_a_plugin_that_was_never_unloaded_is_refused() {
        let mut plugins = plugins();
        assert!(plugins.load(0).is_err());
    }

    /// A context over `text`, with the caret and selection in bytes.
    fn doc(text: &str, caret: usize, selection: (usize, usize)) -> Context<'_> {
        Context {
            text,
            caret,
            selection,
            read_only: false,
        }
    }

    fn read_only(text: &str) -> Context<'_> {
        Context {
            text,
            caret: 0,
            selection: (0, 0),
            read_only: true,
        }
    }

    /// A host over the document in `context`, and the bridge behind it.
    fn host(context: &Context) -> (DocumentHost, Rc<RefCell<Bridge>>) {
        let cell = Rc::new(RefCell::new(Bridge::new(context)));
        (DocumentHost::new(Rc::clone(&cell)), cell)
    }

    /// Calls one host function with integer arguments and an empty memory.
    fn call(host: &mut DocumentHost, name: &str, args: &[i32]) -> i32 {
        call_with(host, name, args, "")
    }

    /// Calls one host function, with `payload` written at the start of memory.
    fn call_with(host: &mut DocumentHost, name: &str, args: &[i32], payload: &str) -> i32 {
        let args: Vec<Value> = args.iter().copied().map(Value::I32).collect();
        let mut memory = vec![0u8; payload.len().max(64)];
        memory[..payload.len()].copy_from_slice(payload.as_bytes());
        match host.call(name, &args, &mut memory) {
            Ok(value) => value.and_then(|v| v.as_i32()).unwrap_or(-7),
            Err(error) => panic!("{name}: {error}"),
        }
    }

    #[test]
    fn a_caret_move_is_reported() {
        let context = doc("hello", 1, (1, 1));
        let (mut host, bridge) = host(&context);
        call(&mut host, abi::SET_CARET, &[4]);
        let bridge = bridge.borrow();
        assert_eq!(bridge.caret, 4);
        assert!(bridge.caret_moved);
        assert!(!bridge.changed);
    }

    #[test]
    fn a_selection_can_be_set_and_read_back() {
        let context = doc("hello world", 0, (0, 0));
        let (mut host, bridge) = host(&context);
        // The arguments arrive the other way round, as a drag backwards would.
        call(&mut host, abi::SET_SEL, &[11, 6]);
        assert_eq!(bridge.borrow().selection, (6, 11));
        assert!(bridge.borrow().selection_moved);
        assert_eq!(call(&mut host, abi::SEL_START, &[]), 6);
        assert_eq!(call(&mut host, abi::SEL_END, &[]), 11);
    }

    #[test]
    fn a_selection_never_starts_inside_a_character() {
        let text = "e\u{301}x";
        let context = doc(text, 0, (0, 0));
        let (mut host, bridge) = host(&context);
        call(&mut host, abi::SET_SEL, &[2, 3]);
        // Offset 2 is the second byte of the accent, so the start falls back to
        // 1 rather than splitting the character.
        assert_eq!(bridge.borrow().selection, (1, 3));
    }

    #[test]
    fn a_range_is_replaced_and_the_rest_is_kept() {
        let context = doc("hello world", 0, (0, 0));
        let (mut host, bridge) = host(&context);
        let code = call_with(&mut host, abi::TEXT_REPLACE_RANGE, &[6, 5, 0, 5], "there");
        assert_eq!(code, 0);
        assert_eq!(bridge.borrow().text, "hello there");
    }

    #[test]
    fn a_range_may_be_empty_or_past_the_end() {
        let context = doc("hello", 0, (0, 0));
        let (mut host, bridge) = host(&context);
        // An empty range is an insert at that offset.
        call_with(&mut host, abi::TEXT_REPLACE_RANGE, &[5, 0, 0, 1], "!");
        assert_eq!(bridge.borrow().text, "hello!");
        // A range running past the end stops at the end instead of panicking.
        call_with(&mut host, abi::TEXT_REPLACE_RANGE, &[0, 99, 0, 1], "H");
        assert_eq!(bridge.borrow().text, "H");
    }

    #[test]
    fn a_read_only_document_refuses_a_range_replacement() {
        let context = read_only("hello");
        let (mut host, bridge) = host(&context);
        let code = call_with(&mut host, abi::TEXT_REPLACE_RANGE, &[0, 5, 0, 1], "x");
        assert_eq!(code, 1);
        assert_eq!(bridge.borrow().text, "hello");
        assert!(!bridge.borrow().changed);
    }

    #[test]
    fn line_offsets_count_bytes_not_characters() {
        // Two lines: "é" is two bytes, and the second line ends in CRLF.
        let text = "\u{e9}\nbc\r\nz";
        let context = doc(text, 0, (0, 0));
        let (mut host, _bridge) = host(&context);
        assert_eq!(call(&mut host, abi::LINE_COUNT, &[]), 3);
        assert_eq!(call(&mut host, abi::LINE_START, &[0]), 0);
        assert_eq!(call(&mut host, abi::LINE_START, &[1]), 3);
        assert_eq!(call(&mut host, abi::LINE_START, &[2]), 7);
        assert_eq!(call(&mut host, abi::LINE_LEN, &[0]), 2);
        // The carriage return is not part of the line.
        assert_eq!(call(&mut host, abi::LINE_LEN, &[1]), 2);
        assert_eq!(call(&mut host, abi::LINE_LEN, &[2]), 1);
        assert_eq!(call(&mut host, abi::LINE_OF_POS, &[0]), 0);
        assert_eq!(call(&mut host, abi::LINE_OF_POS, &[3]), 1);
        assert_eq!(call(&mut host, abi::LINE_OF_POS, &[7]), 2);
    }

    #[test]
    fn an_empty_document_has_one_line_and_no_offsets() {
        let context = doc("", 0, (0, 0));
        let (mut host, _bridge) = host(&context);
        assert_eq!(call(&mut host, abi::LINE_COUNT, &[]), 1);
        assert_eq!(call(&mut host, abi::LINE_START, &[0]), 0);
        assert_eq!(call(&mut host, abi::LINE_LEN, &[0]), 0);
        assert_eq!(call(&mut host, abi::LINE_OF_POS, &[0]), 0);
    }

    #[test]
    fn a_line_past_the_last_is_the_end_of_the_document() {
        let context = doc("a\nb", 0, (0, 0));
        let (mut host, _bridge) = host(&context);
        // Past the last line there is no line to answer with, so the offsets
        // pin to the end rather than wrapping back to the start.
        assert_eq!(call(&mut host, abi::LINE_START, &[9]), 3);
        assert_eq!(call(&mut host, abi::LINE_LEN, &[9]), 0);
    }

    #[test]
    fn a_line_ending_in_a_newline_has_a_final_empty_line() {
        let context = doc("a\nb\n", 0, (0, 0));
        let (mut host, _bridge) = host(&context);
        assert_eq!(call(&mut host, abi::LINE_COUNT, &[]), 3);
        assert_eq!(call(&mut host, abi::LINE_START, &[2]), 4);
        assert_eq!(call(&mut host, abi::LINE_LEN, &[2]), 0);
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
