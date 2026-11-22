//! Loading WebAssembly plugins and running their commands.
//!
//! The interpreter lives in [`crate::wasm`]; this module is the bridge between
//! it and a document. Two things happen here. First, a plugin is given a
//! [`Bridge`] -- a scratch copy of the document, the caret, and a log -- rather
//! than the document itself, so a command that traps halfway through leaves the
//! text untouched. Second, the bridge is read back afterwards and the change
//! applied in one step, which is what makes a plugin's edit a single undo entry
//! instead of a hundred.
//!
//! Plugins are discovered in `plugins/` under the configuration directory, one
//! `.wasm` file each. A file that fails to load is recorded rather than skipped
//! silently: a plugin whose commands never run is a bug report waiting to
//! happen, and the log says which file and why.
//!
//! Loading is not a startup step. [`Plugins::enable`] and [`Plugins::disable`]
//! switch one plugin on or off, and the directory is the list of what is
//! installed while the editor's idea of what is *running* is a second, narrower
//! list that a user can edit without restarting.
//!
//! This module is in `nn-core` rather than beside the window because there is
//! no window in it: `npp` runs a command against the document being edited and
//! `npp-mcp` runs the same command against text a client handed it. One
//! implementation behind both is what stops a plugin from behaving one way in
//! the editor and another over the protocol.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::wasm::abi;
use crate::wasm::{Error, Host, Plugin, Value};

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

/// A running plugin: the module, and the cell its host writes into.
struct Loaded {
    plugin: Plugin,
    bridge: Rc<RefCell<Bridge>>,
}

/// One plugin found in the folder, and whether it is running.
///
/// A file in the directory is *installed*; the module is *running* only when the
/// user enabled it, which is why the name has to be remembered separately from
/// the module. Reading the name is the one thing that costs an instantiation at
/// scan time, and the module is dropped immediately afterwards: an installation
/// of twenty plugins holds twenty names and nothing else until one is enabled.
struct Entry {
    /// What the module calls itself, read once when the folder was scanned.
    name: String,
    path: PathBuf,
    /// The commands the module offered at scan time, so something useful can be
    /// said about a plugin that is switched off.
    commands: Vec<String>,
    /// The module and its bridge, when the plugin is enabled.
    loaded: Option<Loaded>,
}

/// One row of the manager: what the file is called and whether it is running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inventory {
    /// The name the module gives itself.
    pub name: String,
    /// The file it came from, without its directory.
    pub file: String,
    /// Whether the module is instantiated and its commands can be run.
    pub enabled: bool,
    /// The commands it offers, by the name the module gives each one.
    ///
    /// Read from the running module when there is one and from the scan when
    /// there is not, so a switched-off plugin still says what it would add.
    pub commands: Vec<String>,
}

/// Every plugin in the folder, the ones that are running, and every file that
/// could not be read as a module at all.
///
/// The folder is the list of what is installed and this struct is the list of
/// what is running: a plugin that is installed but switched off keeps its name,
/// its path and its command names, and nothing of the module. That is the state
/// the user asked for when they switched it off, and it is also why a rebuilt
/// file comes back as the new build when it is switched on again.
#[derive(Default)]
pub struct Plugins {
    entries: Vec<Entry>,
    failures: Vec<(PathBuf, String)>,
}

impl Plugins {
    /// Reads every `.wasm` file in `dir`, in file-name order.
    ///
    /// Nothing is left running: this is a scan, and the caller decides which
    /// plugins to switch on from the inventory it gets back. A missing
    /// directory is not an error -- most installations have no plugins, and an
    /// editor that complained about that on every start would be wrong.
    pub fn discover(dir: &Path) -> Plugins {
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
            match probe(&path, &stem) {
                Ok((name, commands)) => plugins.entries.push(Entry {
                    name,
                    path,
                    commands,
                    loaded: None,
                }),
                Err(error) => plugins.failures.push((path, error)),
            }
        }
        plugins
    }

    /// Whether any plugin is installed, running or not.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many plugins are running.
    pub fn len(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.loaded.is_some())
            .count()
    }

    /// How many plugins are installed, running or not.
    pub fn available(&self) -> usize {
        self.entries.len()
    }

    /// The files that could not be read, with the reason.
    pub fn failures(&self) -> &[(PathBuf, String)] {
        &self.failures
    }

    /// One row per installed plugin, in folder order.
    ///
    /// The command names come from the running module when there is one, and
    /// from the scan otherwise, so a plugin rebuilt on disk and switched on
    /// again reports the commands it has now rather than the ones the old file
    /// had when the folder was read.
    pub fn inventory(&self) -> Vec<Inventory> {
        self.entries
            .iter()
            .map(|entry| Inventory {
                name: entry.name.clone(),
                file: entry
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("?")
                    .to_owned(),
                enabled: entry.loaded.is_some(),
                commands: match &entry.loaded {
                    Some(loaded) => loaded.plugin.commands().to_vec(),
                    None => entry.commands.clone(),
                },
            })
            .collect()
    }

    /// The name of every running plugin, in folder order.
    ///
    /// The names rather than the plugins: the settings file records them, and
    /// the manager's rows are names with a switch, so nothing outside this
    /// module needs to hold a module to ask which plugins are on.
    pub fn names(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|entry| entry.loaded.is_some())
            .map(|entry| entry.name.clone())
            .collect()
    }

    /// Whether a plugin with this name is installed.
    pub fn knows(&self, name: &str) -> bool {
        self.entries.iter().any(|entry| entry.name == name)
    }

    /// Switches on every installed plugin named in `names`.
    ///
    /// The names come from the settings file, so a renamed plugin or a deleted
    /// file leaves a name that matches nothing; that is ignored rather than
    /// treated as an error, and the next time the user toggles anything the
    /// list is written back from what is actually installed.
    ///
    /// Returns how many were switched on.
    pub fn enable_saved(&mut self, names: &[String]) -> usize {
        let mut started = 0;
        for index in 0..self.entries.len() {
            if !names.contains(&self.entries[index].name) {
                continue;
            }
            if self.enable(index).is_ok() {
                started += 1;
            }
        }
        started
    }

    /// Reads an installed plugin's file and runs it, at its place in the folder
    /// order.
    ///
    /// The file is read again rather than kept from the scan, because a plugin
    /// that was rebuilt while the editor was open should come back as the new
    /// build. A failure leaves the entry switched off, so the row that asked
    /// for it is still there to try again after the file is fixed.
    pub fn enable(&mut self, index: usize) -> Result<String, String> {
        let Some(entry) = self.entries.get_mut(index) else {
            return Err(format!("no plugin {index}"));
        };
        if entry.loaded.is_some() {
            return Ok(entry.name.clone());
        }
        let path = entry.path.clone();
        let name = entry.name.clone();
        let stem = path
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
        match Plugin::load(&bytes, &stem, Box::new(host)) {
            Ok(plugin) => {
                entry.loaded = Some(Loaded { plugin, bridge });
                Ok(name)
            }
            Err(error) => {
                let message = error.to_string();
                self.failures.push((path, message.clone()));
                Err(message)
            }
        }
    }

    /// Drops a plugin's module, keeping its place and its file.
    ///
    /// Returns the name that stopped, or `None` for an index that is not there
    /// -- which a click in the manager can still be holding if the file was
    /// deleted between the scan and the toggle.
    pub fn disable(&mut self, index: usize) -> Option<String> {
        let entry = self.entries.get_mut(index)?;
        entry.loaded.take()?;
        Some(entry.name.clone())
    }

    /// One line per running plugin and per file that did not load, for the log.
    pub fn describe(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .entries
            .iter()
            .filter_map(|entry| {
                let loaded = entry.loaded.as_ref()?;
                Some(format!(
                    "{}: {}",
                    entry.path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                    loaded.plugin.describe()
                ))
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
        let entry = self
            .entries
            .get_mut(plugin)
            .ok_or_else(|| Error::Host(format!("no plugin {plugin}")))?;
        let loaded = entry
            .loaded
            .as_mut()
            .ok_or_else(|| Error::Host(format!("plugin {plugin} is switched off")))?;
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

/// Reads a plugin's name and commands without keeping it running.
///
/// The name lives in the module's own data, so the only way to ask is to
/// instantiate it. The instance is dropped on the way out: installation is a
/// file, and this is a scan.
fn probe(path: &Path, stem: &str) -> Result<(String, Vec<String>), String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let bridge = Rc::new(RefCell::new(Bridge::default()));
    let host = DocumentHost::new(Rc::clone(&bridge));
    let plugin = Plugin::load(&bytes, stem, Box::new(host)).map_err(|error| error.to_string())?;
    Ok((plugin.name().to_owned(), plugin.commands().to_vec()))
}

/// The plugin a settings line switches on, if it is one.
///
/// The line is `plugin <name>`: the keyword, a space, and a name that may hold
/// spaces of its own. One line per plugin because a name is a phrase -- "Half
/// Width" -- and a list of them on one line would need a separator that no
/// plugin name is allowed to contain.
///
/// Both readers of the file go through here: the editor, which parses the rest
/// of its keywords in the same loop, and `npp-mcp`, which reads these lines and
/// nothing else. A rule written twice is a rule that disagrees with itself the
/// first time one side is corrected.
pub fn enabled_line(line: &str) -> Option<&str> {
    let (keyword, value) = line.trim().split_once(' ')?;
    if keyword != "plugin" {
        return None;
    }
    let name = value.trim();
    (!name.is_empty()).then_some(name)
}

/// A plugin to load, for the tests of anything that has to load one.
///
/// Two commands: `Uppercase` replaces the document with `REPLACED`, and `Log`
/// logs `REPLACED` and leaves the text alone.
///
/// Assembled here from the same constants the interpreter uses rather than kept
/// as a `.wasm` file in the tree, because a fixture nothing rebuilds is a
/// fixture that stops describing the ABI the first time the ABI moves. A
/// module rather than a `#[cfg(test)]` item because `nn-mcp` tests its plugin
/// tools against the same bytes: the editor and the server that runs its
/// plugins should not each keep their own idea of what a plugin is.
///
/// Behind the `fixture` feature, so neither binary ships it; `cfg(test)` turns
/// it on for this crate's own tests.
#[cfg(any(test, feature = "fixture"))]
pub mod fixture {
    use crate::wasm::abi;

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
pub fn module() -> Vec<u8> {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugins() -> Plugins {
        let bridge = Rc::new(RefCell::new(Bridge::default()));
        let host = DocumentHost::new(Rc::clone(&bridge));
        let plugin = Plugin::load(&fixture::module(), "test", Box::new(host)).unwrap();
        Plugins {
            entries: vec![Entry {
                name: "Test".to_string(),
                path: PathBuf::from("test.wasm"),
                commands: vec!["Uppercase".to_string(), "Log".to_string()],
                loaded: Some(Loaded { plugin, bridge }),
            }],
            failures: Vec::new(),
        }
    }

    #[test]
    fn a_plugin_lists_its_commands() {
        let plugins = plugins();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins.available(), 1);
        assert_eq!(
            plugins.inventory()[0].commands,
            vec!["Uppercase".to_string(), "Log".to_string()]
        );
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

    /// A fresh folder, so each test's plugins are its own.
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_missing_directory_loads_nothing() {
        let plugins = Plugins::discover(Path::new("/nonexistent/plugins"));
        assert!(plugins.is_empty());
        assert_eq!(plugins.available(), 0);
        assert!(plugins.failures().is_empty());
    }

    #[test]
    fn a_broken_file_is_recorded_rather_than_skipped() {
        let dir = folder("npp-plugin-test-broken");
        std::fs::write(dir.join("broken.wasm"), b"not a module").unwrap();
        let plugins = Plugins::discover(&dir);
        assert!(plugins.is_empty());
        assert_eq!(plugins.failures().len(), 1);
        assert!(plugins.failures()[0].1.contains("WebAssembly"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_directory_of_plugins_lists_in_name_order() {
        let dir = folder("npp-plugin-test-order");
        std::fs::write(dir.join("b.wasm"), fixture::module()).unwrap();
        std::fs::write(dir.join("a.wasm"), fixture::module()).unwrap();
        std::fs::write(dir.join("notes.txt"), b"ignored").unwrap();
        let plugins = Plugins::discover(&dir);
        assert_eq!(plugins.available(), 2);
        assert!(plugins.failures().is_empty());
        // The scan keeps the folder order and nothing of the modules.
        let files: Vec<String> = plugins
            .inventory()
            .into_iter()
            .map(|entry| entry.file)
            .collect();
        assert_eq!(files, vec!["a.wasm", "b.wasm"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discovery_leaves_every_plugin_switched_off() {
        let dir = folder("npp-plugin-test-off");
        std::fs::write(dir.join("one.wasm"), fixture::module()).unwrap();
        std::fs::write(dir.join("two.wasm"), fixture::module()).unwrap();
        let plugins = Plugins::discover(&dir);
        assert_eq!(plugins.available(), 2);
        // Installed, named, with its command names, and not running: the scan
        // reads the module only far enough to fill in the row.
        assert_eq!(plugins.len(), 0);
        assert!(plugins.names().is_empty());
        let inventory = plugins.inventory();
        assert!(inventory.iter().all(|entry| !entry.enabled));
        assert!(inventory
            .iter()
            .all(|entry| entry.commands == vec!["Uppercase", "Log"]));
        assert!(inventory.iter().all(|entry| entry.name == "Test"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn switching_a_plugin_on_makes_its_commands_runnable() {
        let dir = folder("npp-plugin-test-enable");
        std::fs::write(dir.join("a.wasm"), fixture::module()).unwrap();
        std::fs::write(dir.join("b.wasm"), fixture::module()).unwrap();
        let mut plugins = Plugins::discover(&dir);
        assert_eq!(plugins.enable(1).as_deref(), Ok("Test"));
        assert_eq!(plugins.len(), 1);
        // The plugin index is the folder index, so switching the first one on
        // does not shift the second one's commands.
        let inventory = plugins.inventory();
        assert!(inventory[1].enabled);
        assert!(!inventory[0].enabled);
        assert_eq!(inventory[1].commands, vec!["Uppercase", "Log"]);
        let outcome = plugins.run(1, 0, &doc("original", 0, (0, 0))).unwrap();
        assert_eq!(outcome.text.as_deref(), Some("REPLACED"));
        assert_eq!(plugins.enable(1).as_deref(), Ok("Test"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_command_of_a_switched_off_plugin_is_refused() {
        let dir = folder("npp-plugin-test-refused");
        std::fs::write(dir.join("a.wasm"), fixture::module()).unwrap();
        let mut plugins = Plugins::discover(&dir);
        assert!(matches!(
            plugins.run(0, 0, &doc("", 0, (0, 0))),
            Err(Error::Host(message)) if message.contains("switched off")
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disabling_a_plugin_takes_its_commands_away() {
        let mut plugins = plugins();
        assert_eq!(plugins.names(), vec!["Test".to_string()]);
        assert_eq!(plugins.disable(0).as_deref(), Some("Test"));
        // Installed but not running: the row stays, and the commands it would
        // add are still named, because the file they come from is still there.
        assert_eq!(plugins.available(), 1);
        assert_eq!(plugins.len(), 0);
        let inventory = plugins.inventory();
        assert!(!inventory[0].enabled);
        assert_eq!(inventory[0].commands, vec!["Uppercase", "Log"]);
        // The manager's cursor is a click behind the state, so an index that is
        // already off has to answer rather than panic.
        assert_eq!(plugins.disable(0), None);
        assert_eq!(plugins.disable(7), None);
    }

    #[test]
    fn a_plugin_switched_off_is_read_back_from_its_file() {
        let dir = folder("npp-plugin-test-reenable");
        let path = dir.join("test.wasm");
        std::fs::write(&path, fixture::module()).unwrap();
        let mut plugins = Plugins::discover(&dir);
        assert_eq!(plugins.enable(0).as_deref(), Ok("Test"));
        assert_eq!(plugins.len(), 1);

        assert_eq!(plugins.disable(0).as_deref(), Some("Test"));
        assert_eq!(plugins.len(), 0);
        assert_eq!(plugins.enable(0).as_deref(), Ok("Test"));
        assert_eq!(plugins.len(), 1);
        assert!(plugins.inventory()[0].enabled);
        assert_eq!(plugins.inventory()[0].commands, vec!["Uppercase", "Log"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_plugin_whose_file_is_gone_stays_switched_off() {
        let dir = folder("npp-plugin-test-gone");
        let path = dir.join("test.wasm");
        std::fs::write(&path, fixture::module()).unwrap();
        let mut plugins = Plugins::discover(&dir);
        assert_eq!(plugins.enable(0).as_deref(), Ok("Test"));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(plugins.disable(0).as_deref(), Some("Test"));

        // Still offered, because the file may be put back.
        assert!(plugins.enable(0).is_err());
        assert_eq!(plugins.len(), 0);
        assert!(!plugins.inventory()[0].enabled);
        assert_eq!(plugins.failures().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn switching_on_a_plugin_that_is_already_running_is_not_an_error() {
        let mut plugins = plugins();
        assert_eq!(plugins.enable(0).as_deref(), Ok("Test"));
        assert_eq!(plugins.len(), 1);
    }

    #[test]
    fn the_saved_names_switch_on_only_what_is_installed() {
        let dir = folder("npp-plugin-test-saved");
        std::fs::write(dir.join("a.wasm"), fixture::module()).unwrap();
        std::fs::write(dir.join("b.wasm"), fixture::module()).unwrap();
        let mut plugins = Plugins::discover(&dir);
        // A name that names nothing -- a deleted or renamed plugin -- is not an
        // error: the settings file is allowed to be out of date.
        let saved = vec!["Test".to_string(), "Gone".to_string()];
        assert_eq!(plugins.enable_saved(&saved), 2);
        assert_eq!(plugins.len(), 2);
        assert!(plugins.knows("Test"));
        assert!(!plugins.knows("Gone"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_inventory_names_the_file_behind_each_plugin() {
        let dir = folder("npp-plugin-test-inventory");
        std::fs::write(dir.join("npp_test.wasm"), fixture::module()).unwrap();
        let plugins = Plugins::discover(&dir);
        assert_eq!(
            plugins.inventory(),
            vec![Inventory {
                name: "Test".to_string(),
                file: "npp_test.wasm".to_string(),
                enabled: false,
                commands: vec!["Uppercase".to_string(), "Log".to_string()],
            }]
        );
        let _ = std::fs::remove_dir_all(&dir);
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
    fn a_settings_line_names_the_plugin_it_switches_on() {
        assert_eq!(enabled_line("plugin JSON"), Some("JSON"));
        // A name with spaces, and whitespace that came from a hand edit.
        assert_eq!(enabled_line("plugin  Half Width  "), Some("Half Width"));
        // A keyword without a value says nothing, and a different keyword is
        // not this one however it starts.
        assert_eq!(enabled_line("plugin"), None);
        assert_eq!(enabled_line("plugin   "), None);
        assert_eq!(enabled_line("plugins JSON"), None);
        assert_eq!(enabled_line("# plugin JSON"), None);
        assert_eq!(enabled_line("theme dark"), None);
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
