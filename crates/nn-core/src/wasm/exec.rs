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

//! The interpreter: one function body at a time, over a flat value stack.
//!
//! The design is the obvious one for a tree-walking WebAssembly engine. The
//! body is a byte slice; `pc` walks it; a control stack records where each open
//! `block`, `loop`, and `if` ends so a branch can jump without rescanning. The
//! jump table is built once at load ([`jumps`]) rather than on entry to each
//! block, because a `loop` that rescans its body on every iteration turns an
//! O(n) program into an O(n^2) one.
//!
//! Values carry their type. There is no separate validation pass, so an
//! instruction that finds the wrong type on the stack reports a malformed
//! module at the moment it runs. That is a worse diagnostic than a validator
//! would give and the same refusal, and it keeps the engine to one
//! implementation of the type rules instead of two.
//!
//! Three limits keep a plugin from taking the editor with it: a call depth, a
//! value stack size, and a step budget. The budget is the important one -- a
//! plugin that loops forever is stopped after [`MAX_FUEL`] instructions and
//! reported as such, rather than freezing the window.

use super::{Error, Func, Module, ValType, Value};

/// One 64 KiB WebAssembly page.
pub const PAGE: usize = 65_536;

/// The most memory a plugin may have, in pages. Sixteen mebibytes is far more
/// than a text transformation needs and small enough that a runaway
/// `memory.grow` cannot exhaust the machine.
pub const MAX_PAGES: u32 = 256;

/// How deep calls may nest. A recursive plugin is a bug; this turns it into an
/// error message instead of a stack overflow.
pub const MAX_DEPTH: u32 = 128;

/// How many values may be on the stack at once.
pub const MAX_STACK: usize = 1 << 16;

/// How many entries a table may have. A table holds function references, so a
/// few thousand is already far more than a plugin has functions.
pub const MAX_TABLE: u32 = 4096;

/// How many instructions one command may execute. At roughly a hundred million
/// simple instructions a second in an interpreter, this is about half a second
/// of work -- long enough for a plugin to walk a large document, short enough
/// that a hang is not mistaken for a freeze.
pub const MAX_FUEL: u64 = 50_000_000;

/// Where a block's `else` and `end` are, so a branch can find them in constant
/// time. `usize::MAX` means "there is none".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Jump {
    pub end: usize,
    pub else_at: usize,
}

impl Jump {
    pub const NONE: Jump = Jump { end: usize::MAX, else_at: usize::MAX };
}

/// The result type of a block, which in the MVP is either nothing or one value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockType {
    Empty,
    Value(ValType),
}

impl BlockType {
    fn arity(self) -> usize {
        match self {
            BlockType::Empty => 0,
            BlockType::Value(_) => 1,
        }
    }
}

/// A decoded instruction. Arithmetic and comparison share [`Op::Numeric`],
/// which carries the raw opcode: there are a hundred and fifty of them and
/// giving each its own variant would triple the size of the enum for no gain,
/// since they all take their operands from the stack and push one result.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Unreachable,
    Nop,
    Block(BlockType),
    Loop(BlockType),
    If(BlockType),
    Else,
    End,
    Br(u32),
    BrIf(u32),
    /// `at` is the byte offset of the target list, which is walked on demand
    /// rather than decoded into a `Vec` on every execution.
    BrTable { at: usize, count: u32, default: u32 },
    Return,
    Call(u32),
    Drop,
    Select,
    LocalGet(u32),
    LocalSet(u32),
    LocalTee(u32),
    GlobalGet(u32),
    GlobalSet(u32),
    CallIndirect { type_index: u32, table: u32 },
    MemoryCopy,
    MemoryFill,
    RefNull,
    RefIsNull,
    RefFunc(u32),
    /// One of the eight saturating conversions, which live behind the `0xFC`
    /// prefix and are numbered 0 to 7.
    Saturating(u32),
    Load { op: u8, offset: u32 },
    Store { op: u8, offset: u32 },
    MemorySize,
    MemoryGrow,
    Const(Value),
    Numeric(u8),
}

/// Decodes one instruction, advancing `pc` past its immediates.
///
/// The alignment immediate of a load or store is read and discarded. It is a
/// hint to a compiler that the access is aligned; the interpreter bounds checks
/// every access anyway, so honouring it would change nothing.
pub fn step(code: &[u8], pc: &mut usize) -> Result<Op, Error> {
    let start = *pc;
    let opcode = *code.get(start).ok_or(Error::Truncated)?;
    let mut reader = super::Reader::new(code.get(start + 1..).ok_or(Error::Truncated)?);
    let total = reader.remaining();
    let op = match opcode {
        0x00 => Op::Unreachable,
        0x01 => Op::Nop,
        0x02 => Op::Block(block_type(&mut reader)?),
        0x03 => Op::Loop(block_type(&mut reader)?),
        0x04 => Op::If(block_type(&mut reader)?),
        0x05 => Op::Else,
        0x0B => Op::End,
        0x0C => Op::Br(reader.u32()?),
        0x0D => Op::BrIf(reader.u32()?),
        0x0E => {
            let count = reader.u32()?;
            let at = start + 1 + (total - reader.remaining());
            // The targets are walked on demand by `br_target`, but they still
            // have to be stepped over here so `pc` lands on the next
            // instruction.
            for _ in 0..count {
                reader.u32()?;
            }
            let default = reader.u32()?;
            Op::BrTable { at, count, default }
        }
        0x0F => Op::Return,
        0x10 => Op::Call(reader.u32()?),
        0x11 => {
            let type_index = reader.u32()?;
            let table = reader.u32()?;
            Op::CallIndirect { type_index, table }
        }
        0x1A => Op::Drop,
        0x1B => Op::Select,
        0x1C => {
            // `select` with an explicit result type. The type is redundant for
            // this interpreter, which checks at run time, so it is read and
            // dropped.
            let count = reader.u32()?;
            for _ in 0..count {
                reader.u8()?;
            }
            Op::Select
        }
        0x20 => Op::LocalGet(reader.u32()?),
        0x21 => Op::LocalSet(reader.u32()?),
        0x22 => Op::LocalTee(reader.u32()?),
        0x23 => Op::GlobalGet(reader.u32()?),
        0x24 => Op::GlobalSet(reader.u32()?),
        0x28..=0x35 => {
            let _align = reader.u32()?;
            Op::Load { op: opcode, offset: reader.u32()? }
        }
        0x36..=0x3E => {
            let _align = reader.u32()?;
            Op::Store { op: opcode, offset: reader.u32()? }
        }
        0x3F => {
            reader.u32()?;
            Op::MemorySize
        }
        0x40 => {
            reader.u32()?;
            Op::MemoryGrow
        }
        0x41 => Op::Const(Value::I32(reader.i32()?)),
        0x42 => Op::Const(Value::I64(reader.i64()?)),
        0x43 => Op::Const(Value::F32(reader.f32()?)),
        0x44 => Op::Const(Value::F64(reader.f64()?)),
        0x45..=0xC4 => Op::Numeric(opcode),
        0xD0 => {
            reader.u8()?;
            Op::RefNull
        }
        0xD1 => Op::RefIsNull,
        0xD2 => Op::RefFunc(reader.u32()?),
        0xFC => {
            let sub = reader.u32()?;
            match sub {
                0..=7 => Op::Saturating(sub),
                10 => {
                    reader.u32()?;
                    reader.u32()?;
                    Op::MemoryCopy
                }
                11 => {
                    reader.u32()?;
                    Op::MemoryFill
                }
                8 => return Err(Error::Unsupported("memory.init")),
                9 => return Err(Error::Unsupported("data.drop")),
                12 => return Err(Error::Unsupported("table.init")),
                13 => return Err(Error::Unsupported("elem.drop")),
                14 => return Err(Error::Unsupported("table.copy")),
                15 => return Err(Error::Unsupported("table.grow")),
                16 => return Err(Error::Unsupported("table.size")),
                17 => return Err(Error::Unsupported("table.fill")),
                _ => return Err(Error::Unsupported("an unknown 0xFC instruction")),
            }
        }
        other => return Err(Error::UnsupportedOpcode(other)),
    };
    *pc = start + 1 + (total - reader.remaining());
    Ok(op)
}

fn block_type(reader: &mut super::Reader<'_>) -> Result<BlockType, Error> {
    // The three forms are unambiguous: a type index is a non-negative signed
    // LEB, and every special form is a negative one.
    match reader.u8()? {
        0x40 => Ok(BlockType::Empty),
        0x7F => Ok(BlockType::Value(ValType::I32)),
        0x7E => Ok(BlockType::Value(ValType::I64)),
        0x7D => Ok(BlockType::Value(ValType::F32)),
        0x7C => Ok(BlockType::Value(ValType::F64)),
        _ => Err(Error::Unsupported("a block with a type index (multi-value)")),
    }
}

/// Builds the jump table for one function body.
///
/// Every byte offset gets an entry, most of them [`Jump::NONE`]. A `Vec` the
/// length of the body is a few hundred bytes for a typical function and turns
/// "where does this block end" from a scan into an index.
pub fn jumps(code: &[u8]) -> Result<Vec<Jump>, Error> {
    let mut table = vec![Jump::NONE; code.len()];
    let mut open: Vec<usize> = Vec::new();
    let mut pc = 0;
    while pc < code.len() {
        let at = pc;
        let op = step(code, &mut pc)?;
        match op {
            Op::Block(_) | Op::Loop(_) | Op::If(_) => open.push(at),
            Op::Else => {
                let top = *open.last().ok_or(Error::BadModule("an else outside a block"))?;
                table[top].else_at = at;
            }
            Op::End => {
                if let Some(top) = open.pop() {
                    table[top].end = pc;
                }
            }
            _ => {}
        }
    }
    if !open.is_empty() {
        return Err(Error::BadModule("a block is never closed"));
    }
    Ok(table)
}

/// What a plugin can call out to.
///
/// One method, dispatched by name, rather than a trait method per host
/// function. The set of host functions is defined by [`super::abi`] and grows
/// with the editor; a trait that grew with it would need every implementor
/// updated for every addition, and the only implementor that matters is the
/// app's document bridge.
///
/// The host is handed the plugin's linear memory so it can read and write the
/// strings the plugin passes by pointer. It is a plain slice: the interpreter
/// bounds checks every access it makes itself, and the host is expected to do
/// the same.
pub trait Host {
    fn call(
        &mut self,
        name: &str,
        args: &[Value],
        memory: &mut [u8],
    ) -> Result<Option<Value>, Error>;
}

/// A host that records what it was asked and answers nothing. Used by the
/// tests, and by the loader when it only wants to read a plugin's name.
#[derive(Debug, Default)]
pub struct NullHost {
    pub calls: Vec<(String, Vec<Value>)>,
}

impl Host for NullHost {
    fn call(
        &mut self,
        name: &str,
        args: &[Value],
        _memory: &mut [u8],
    ) -> Result<Option<Value>, Error> {
        self.calls.push((name.to_owned(), args.to_vec()));
        Ok(None)
    }
}

/// The mutable state a module runs against: its memory, its globals, and its
/// table.
///
/// Bundled rather than kept as three fields of [`Instance`] so the interpreter
/// can be handed one `&mut` instead of three, which is what keeps the recursive
/// call path from turning into a dozen parameters.
pub struct Store {
    pub memory: Vec<u8>,
    pub globals: Vec<Value>,
    pub table: Vec<Option<u32>>,
}

/// A module with a store and a host: something that can be called.
pub struct Instance {
    module: Module,
    store: Store,
    host: Box<dyn Host>,
    depth: u32,
    stack: Vec<Value>,
    fuel: u64,
}

impl Instance {
    /// Instantiates a module: allocates its memory, copies its data segments
    /// in, and runs its start function if it has one.
    pub fn new(module: Module, host: Box<dyn Host>) -> Result<Instance, Error> {
        let pages = module.memory.map(|m| m.min).unwrap_or(0);
        if pages > MAX_PAGES {
            return Err(Error::Host(format!(
                "the plugin asks for {pages} pages of memory, more than the {MAX_PAGES} allowed"
            )));
        }
        let mut memory = vec![0u8; pages as usize * PAGE];
        for segment in &module.data {
            let start = segment.offset as usize;
            let end = start.checked_add(segment.bytes.len()).ok_or(Error::OutOfBounds)?;
            let slot = memory.get_mut(start..end).ok_or(Error::OutOfBounds)?;
            slot.copy_from_slice(&segment.bytes);
        }
        // Globals are evaluated in order, so one may be initialised from an
        // earlier one -- which is how a compiler emits a constant it computed
        // at build time.
        let mut globals: Vec<Value> = Vec::new();
        for global in &module.globals {
            let value = match &global.init {
                super::Init::Value(value) => *value,
                super::Init::Global(index) => *globals
                    .get(*index as usize)
                    .ok_or(Error::BadModule("a global initialised from a missing global"))?,
            };
            globals.push(value);
        }
        let table_pages = module.table.map(|table| table.min).unwrap_or(0);
        if table_pages > MAX_TABLE {
            return Err(Error::Host(format!(
                "the plugin asks for a table of {table_pages} entries, more than the {MAX_TABLE} allowed"
            )));
        }
        let mut table = vec![None; table_pages as usize];
        for element in &module.elements {
            let start = element.offset as usize;
            let end = start.checked_add(element.funcs.len()).ok_or(Error::OutOfBounds)?;
            let slot = table.get_mut(start..end).ok_or(Error::OutOfBounds)?;
            slot.copy_from_slice(&element.funcs);
        }
        let mut instance = Instance {
            module,
            store: Store { memory, globals, table },
            host,
            depth: 0,
            stack: Vec::new(),
            fuel: MAX_FUEL,
        };
        if let Some(start) = instance.module.start {
            instance.call(start, &[])?;
        }
        Ok(instance)
    }

    pub fn module(&self) -> &Module {
        &self.module
    }

    pub fn memory(&self) -> &[u8] {
        &self.store.memory
    }

    pub fn memory_mut(&mut self) -> &mut [u8] {
        &mut self.store.memory
    }

    pub fn globals(&self) -> &[Value] {
        &self.store.globals
    }

    pub fn table(&self) -> &[Option<u32>] {
        &self.store.table
    }

    pub fn host_mut(&mut self) -> &mut dyn Host {
        &mut *self.host
    }

    /// Calls an exported function by name. The fuel budget is reset here, so
    /// each command a plugin runs gets its own allowance rather than sharing
    /// one with every command before it.
    pub fn call_export(&mut self, name: &str, args: &[Value]) -> Result<Vec<Value>, Error> {
        let index = self
            .module
            .export_func(name)
            .ok_or_else(|| Error::NoSuchExport(name.to_owned()))?;
        self.fuel = MAX_FUEL;
        self.call(index, args)
    }

    /// Calls a function by index in the function index space.
    pub fn call(&mut self, index: u32, args: &[Value]) -> Result<Vec<Value>, Error> {
        let Self { module, store, host, depth, stack, fuel } = self;
        exec(module, store, &mut **host, depth, stack, fuel, index, args)
    }
}

#[allow(clippy::too_many_arguments)]
fn exec(
    module: &Module,
    store: &mut Store,
    host: &mut dyn Host,
    depth: &mut u32,
    stack: &mut Vec<Value>,
    fuel: &mut u64,
    index: u32,
    args: &[Value],
) -> Result<Vec<Value>, Error> {
    let imported = module.imported_funcs();
    if (index as usize) < imported.len() {
        let import = module.imported_func(index).ok_or(Error::NoSuchFunction(index))?;
        let name = import.name.clone();
        return Ok(host.call(&name, args, &mut store.memory[..])?.into_iter().collect());
    }
    let func = module
        .funcs
        .get(index as usize - imported.len())
        .ok_or(Error::NoSuchFunction(index))?;
    let ty = module
        .types
        .get(func.type_index as usize)
        .ok_or(Error::NoSuchFunction(index))?;
    if args.len() != ty.params.len() {
        return Err(Error::Host(format!(
            "a function takes {} arguments but {} were given",
            ty.params.len(),
            args.len()
        )));
    }
    if *depth >= MAX_DEPTH {
        return Err(Error::StackExhausted);
    }
    *depth += 1;
    let base = stack.len();
    stack.extend_from_slice(args);
    for local in &func.locals {
        stack.push(zero(*local));
    }
    let result = run_body(module, store, host, depth, stack, fuel, func, base);
    *depth -= 1;
    stack.truncate(base);
    result
}

fn zero(ty: ValType) -> Value {
    match ty {
        ValType::I32 => Value::I32(0),
        ValType::I64 => Value::I64(0),
        ValType::F32 => Value::F32(0.0),
        ValType::F64 => Value::F64(0.0),
        ValType::FuncRef => Value::FuncRef(None),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Block,
    Loop,
    If,
}

#[derive(Debug, Clone, Copy)]
struct Label {
    kind: Kind,
    start: usize,
    end: usize,
    arity: usize,
    height: usize,
}

#[allow(clippy::too_many_arguments)]
fn run_body(
    module: &Module,
    store: &mut Store,
    host: &mut dyn Host,
    depth: &mut u32,
    stack: &mut Vec<Value>,
    fuel: &mut u64,
    func: &Func,
    base: usize,
) -> Result<Vec<Value>, Error> {
    let results = module
        .types
        .get(func.type_index as usize)
        .map(|ty| ty.results.len())
        .unwrap_or(0);
    let mut labels: Vec<Label> = Vec::new();
    let mut pc = 0usize;
    loop {
        if *fuel == 0 {
            return Err(Error::OutOfFuel);
        }
        *fuel -= 1;
        let at = pc;
        let op = step(&func.code, &mut pc)?;
        match op {
            Op::Unreachable => return Err(Error::Unreachable),
            Op::Nop => {}
            Op::Block(bt) => {
                let jump = func.jumps.get(at).copied().unwrap_or(Jump::NONE);
                labels.push(Label {
                    kind: Kind::Block,
                    start: pc,
                    end: jump.end,
                    arity: bt.arity(),
                    height: stack.len(),
                });
            }
            Op::Loop(_) => {
                let jump = func.jumps.get(at).copied().unwrap_or(Jump::NONE);
                // A loop's branch target takes the loop's parameters, which in
                // the MVP are none, so its arity is zero regardless of the
                // block type's result.
                labels.push(Label {
                    kind: Kind::Loop,
                    start: pc,
                    end: jump.end,
                    arity: 0,
                    height: stack.len(),
                });
            }
            Op::If(bt) => {
                let cond = pop_i32(stack)?;
                let jump = func.jumps.get(at).copied().unwrap_or(Jump::NONE);
                if cond != 0 {
                    labels.push(Label {
                        kind: Kind::If,
                        start: pc,
                        end: jump.end,
                        arity: bt.arity(),
                        height: stack.len(),
                    });
                } else if jump.else_at != usize::MAX {
                    labels.push(Label {
                        kind: Kind::If,
                        start: pc,
                        end: jump.end,
                        arity: bt.arity(),
                        height: stack.len(),
                    });
                    pc = jump.else_at + 1;
                } else {
                    pc = jump.end;
                }
            }
            Op::Else => {
                // Reached by falling out of the true branch: skip the false
                // branch entirely.
                let label = labels.pop().ok_or(Error::BadModule("an else outside a block"))?;
                pc = label.end;
            }
            Op::End => {
                if labels.pop().is_none() {
                    return take_results(stack, results);
                }
            }
            Op::Br(n) => {
                let target = labels
                    .len()
                    .checked_sub(1 + n as usize)
                    .ok_or(Error::BadModule("a branch out of range"))?;
                let label = labels[target];
                if label.kind == Kind::Loop {
                    stack.truncate(label.height);
                    labels.truncate(target + 1);
                    pc = label.start;
                } else {
                    let values = take_results(stack, label.arity)?;
                    stack.truncate(label.height);
                    stack.extend(values);
                    labels.truncate(target);
                    pc = label.end;
                }
            }
            Op::BrIf(n) => {
                if pop_i32(stack)? != 0 {
                    let target = labels
                        .len()
                        .checked_sub(1 + n as usize)
                        .ok_or(Error::BadModule("a branch out of range"))?;
                    let label = labels[target];
                    if label.kind == Kind::Loop {
                        stack.truncate(label.height);
                        labels.truncate(target + 1);
                        pc = label.start;
                    } else {
                        let values = take_results(stack, label.arity)?;
                        stack.truncate(label.height);
                        stack.extend(values);
                        labels.truncate(target);
                        pc = label.end;
                    }
                }
            }
            Op::BrTable { at, count, default } => {
                let index = pop_i32(stack)? as u32;
                let n = if index < count {
                    br_target(&func.code, at, index)?
                } else {
                    default
                };
                let target = labels
                    .len()
                    .checked_sub(1 + n as usize)
                    .ok_or(Error::BadModule("a branch out of range"))?;
                let label = labels[target];
                if label.kind == Kind::Loop {
                    stack.truncate(label.height);
                    labels.truncate(target + 1);
                    pc = label.start;
                } else {
                    let values = take_results(stack, label.arity)?;
                    stack.truncate(label.height);
                    stack.extend(values);
                    labels.truncate(target);
                    pc = label.end;
                }
            }
            Op::Return => return take_results(stack, results),
            Op::Call(n) => {
                let ty = module.func_type(n).ok_or(Error::NoSuchFunction(n))?;
                let nargs = ty.params.len();
                let split = stack.len().checked_sub(nargs).ok_or(Error::StackExhausted)?;
                let args = stack.split_off(split);
                let values = exec(module, store, host, depth, stack, fuel, n, &args)?;
                stack.extend(values);
            }
            Op::Drop => {
                stack.pop().ok_or(Error::StackExhausted)?;
            }
            Op::Select => {
                let cond = pop_i32(stack)?;
                let b = stack.pop().ok_or(Error::StackExhausted)?;
                let a = stack.pop().ok_or(Error::StackExhausted)?;
                stack.push(if cond != 0 { a } else { b });
            }
            Op::LocalGet(i) => {
                let value = *stack
                    .get(base + i as usize)
                    .ok_or(Error::BadModule("a local that does not exist"))?;
                stack.push(value);
            }
            Op::LocalSet(i) => {
                let value = stack.pop().ok_or(Error::StackExhausted)?;
                let slot = stack
                    .get_mut(base + i as usize)
                    .ok_or(Error::BadModule("a local that does not exist"))?;
                *slot = value;
            }
            Op::LocalTee(i) => {
                let value = *stack.last().ok_or(Error::StackExhausted)?;
                let slot = stack
                    .get_mut(base + i as usize)
                    .ok_or(Error::BadModule("a local that does not exist"))?;
                *slot = value;
            }
            Op::Load { op, offset } => {
                let address = pop_i32(stack)? as u32 as usize;
                let at = address.checked_add(offset as usize).ok_or(Error::OutOfBounds)?;
                stack.push(load(op, &store.memory[..], at)?);
            }
            Op::Store { op, offset } => {
                let value = stack.pop().ok_or(Error::StackExhausted)?;
                let address = pop_i32(stack)? as u32 as usize;
                let at = address.checked_add(offset as usize).ok_or(Error::OutOfBounds)?;
                write(op, &mut store.memory[..], at, value)?;
            }
            Op::MemorySize => stack.push(Value::I32((store.memory.len() / PAGE) as i32)),
            Op::MemoryGrow => {
                let delta = pop_i32(stack)?;
                let pages = (store.memory.len() / PAGE) as u32;
                let wanted = pages as i64 + i64::from(delta);
                if delta < 0 || wanted > i64::from(MAX_PAGES) {
                    stack.push(Value::I32(-1));
                } else {
                    store.memory.resize(wanted as usize * PAGE, 0);
                    stack.push(Value::I32(pages as i32));
                }
            }
            Op::GlobalGet(index) => {
                let value = *store
                    .globals
                    .get(index as usize)
                    .ok_or(Error::BadModule("a global that does not exist"))?;
                stack.push(value);
            }
            Op::GlobalSet(index) => {
                let value = stack.pop().ok_or(Error::StackExhausted)?;
                let slot = store
                    .globals
                    .get_mut(index as usize)
                    .ok_or(Error::BadModule("a global that does not exist"))?;
                *slot = value;
            }
            Op::CallIndirect { type_index, table } => {
                if table != 0 {
                    return Err(Error::Unsupported("a call through a table other than the first"));
                }
                let slot = pop_i32(stack)? as u32 as usize;
                let target = store
                    .table
                    .get(slot)
                    .copied()
                    .flatten()
                    .ok_or(Error::BadModule("an indirect call to an empty table slot"))?;
                let want = module
                    .types
                    .get(type_index as usize)
                    .ok_or(Error::BadModule("an indirect call naming a missing type"))?;
                let got = module.func_type(target).ok_or(Error::NoSuchFunction(target))?;
                if want.params != got.params || want.results != got.results {
                    return Err(Error::BadModule(
                        "an indirect call through the wrong signature",
                    ));
                }
                let nargs = got.params.len();
                let split = stack.len().checked_sub(nargs).ok_or(Error::StackExhausted)?;
                let args = stack.split_off(split);
                let values = exec(module, store, host, depth, stack, fuel, target, &args)?;
                stack.extend(values);
            }
            Op::MemoryCopy => {
                let n = pop_i32(stack)? as u32 as usize;
                let from = pop_i32(stack)? as u32 as usize;
                let to = pop_i32(stack)? as u32 as usize;
                let from_end = from.checked_add(n).ok_or(Error::OutOfBounds)?;
                let to_end = to.checked_add(n).ok_or(Error::OutOfBounds)?;
                if from_end > store.memory.len() || to_end > store.memory.len() {
                    return Err(Error::OutOfBounds);
                }
                store.memory.copy_within(from..from_end, to);
            }
            Op::MemoryFill => {
                let n = pop_i32(stack)? as u32 as usize;
                let byte = pop_i32(stack)? as u8;
                let at = pop_i32(stack)? as u32 as usize;
                let end = at.checked_add(n).ok_or(Error::OutOfBounds)?;
                let slot = store.memory.get_mut(at..end).ok_or(Error::OutOfBounds)?;
                slot.fill(byte);
            }
            Op::RefNull => stack.push(Value::FuncRef(None)),
            Op::RefIsNull => {
                let value = stack.pop().ok_or(Error::StackExhausted)?;
                stack.push(Value::I32(i32::from(matches!(value, Value::FuncRef(None)))));
            }
            Op::RefFunc(index) => stack.push(Value::FuncRef(Some(index))),
            Op::Saturating(sub) => saturating(sub, stack)?,
            Op::Const(value) => stack.push(value),
            Op::Numeric(op) => numeric(op, stack)?,
        }
        if stack.len() > MAX_STACK {
            return Err(Error::StackExhausted);
        }
    }
}

/// Reads the `index`th target of a `br_table`, walking the LEB128 list from
/// its start. Walking rather than decoding into a `Vec` keeps the instruction
/// allocation-free; a `br_table` in a hot loop is rare enough that the walk is
/// not worth optimising away.
fn br_target(code: &[u8], at: usize, index: u32) -> Result<u32, Error> {
    let mut reader = super::Reader::new(code.get(at..).ok_or(Error::Truncated)?);
    for _ in 0..index {
        reader.u32()?;
    }
    reader.u32()
}

fn take_results(stack: &mut Vec<Value>, arity: usize) -> Result<Vec<Value>, Error> {
    let at = stack.len().checked_sub(arity).ok_or(Error::StackExhausted)?;
    Ok(stack.split_off(at))
}

fn pop(stack: &mut Vec<Value>) -> Result<Value, Error> {
    stack.pop().ok_or(Error::StackExhausted)
}

fn pop_i32(stack: &mut Vec<Value>) -> Result<i32, Error> {
    pop(stack)?.as_i32().ok_or(Error::BadModule("expected an i32 on the stack"))
}

fn pop_i64(stack: &mut Vec<Value>) -> Result<i64, Error> {
    match pop(stack)? {
        Value::I64(v) => Ok(v),
        _ => Err(Error::BadModule("expected an i64 on the stack")),
    }
}

fn pop_f32(stack: &mut Vec<Value>) -> Result<f32, Error> {
    match pop(stack)? {
        Value::F32(v) => Ok(v),
        _ => Err(Error::BadModule("expected an f32 on the stack")),
    }
}

fn pop_f64(stack: &mut Vec<Value>) -> Result<f64, Error> {
    match pop(stack)? {
        Value::F64(v) => Ok(v),
        _ => Err(Error::BadModule("expected an f64 on the stack")),
    }
}

fn load(op: u8, memory: &[u8], at: usize) -> Result<Value, Error> {
    let read = |n: usize| -> Result<&[u8], Error> {
        let end = at.checked_add(n).ok_or(Error::OutOfBounds)?;
        memory.get(at..end).ok_or(Error::OutOfBounds)
    };
    Ok(match op {
        0x28 => Value::I32(i32::from_le_bytes(read(4)?.try_into().unwrap())),
        0x29 => Value::I64(i64::from_le_bytes(read(8)?.try_into().unwrap())),
        0x2A => Value::F32(f32::from_le_bytes(read(4)?.try_into().unwrap())),
        0x2B => Value::F64(f64::from_le_bytes(read(8)?.try_into().unwrap())),
        0x2C => Value::I32(i32::from(read(1)?[0] as i8)),
        0x2D => Value::I32(i32::from(read(1)?[0])),
        0x2E => Value::I32(i32::from(i16::from_le_bytes(read(2)?.try_into().unwrap()))),
        0x2F => Value::I32(i32::from(u16::from_le_bytes(read(2)?.try_into().unwrap()))),
        0x30 => Value::I64(i64::from(read(1)?[0] as i8)),
        0x31 => Value::I64(i64::from(read(1)?[0])),
        0x32 => Value::I64(i64::from(i16::from_le_bytes(read(2)?.try_into().unwrap()))),
        0x33 => Value::I64(i64::from(u16::from_le_bytes(read(2)?.try_into().unwrap()))),
        0x34 => Value::I64(i64::from(i32::from_le_bytes(read(4)?.try_into().unwrap()))),
        0x35 => Value::I64(i64::from(u32::from_le_bytes(read(4)?.try_into().unwrap()))),
        _ => return Err(Error::UnsupportedOpcode(op)),
    })
}

fn write(op: u8, memory: &mut [u8], at: usize, value: Value) -> Result<(), Error> {
    let mut write = |n: usize, bytes: &[u8]| -> Result<(), Error> {
        let end = at.checked_add(n).ok_or(Error::OutOfBounds)?;
        let slot = memory.get_mut(at..end).ok_or(Error::OutOfBounds)?;
        slot.copy_from_slice(bytes);
        Ok(())
    };
    match (op, value) {
        (0x36, Value::I32(v)) => write(4, &v.to_le_bytes()),
        (0x37, Value::I64(v)) => write(8, &v.to_le_bytes()),
        (0x38, Value::F32(v)) => write(4, &v.to_le_bytes()),
        (0x39, Value::F64(v)) => write(8, &v.to_le_bytes()),
        (0x3A, Value::I32(v)) => write(1, &[v as u8]),
        (0x3B, Value::I32(v)) => write(2, &(v as u16).to_le_bytes()),
        (0x3C, Value::I64(v)) => write(1, &[v as u8]),
        (0x3D, Value::I64(v)) => write(2, &(v as u16).to_le_bytes()),
        (0x3E, Value::I64(v)) => write(4, &(v as u32).to_le_bytes()),
        _ => Err(Error::BadModule("a store whose value does not match its width")),
    }
}

/// WebAssembly's `min` and `max` propagate NaN, which Rust's do not: Rust
/// returns the non-NaN operand. Signed zero is the other difference -- wasm
/// says `min(+0, -0)` is `-0` -- so both are handled explicitly.
fn fmin32(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        return f32::NAN;
    }
    if a == b {
        return if a.is_sign_negative() { a } else { b };
    }
    if a < b { a } else { b }
}

fn fmax32(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        return f32::NAN;
    }
    if a == b {
        return if a.is_sign_negative() { b } else { a };
    }
    if a > b { a } else { b }
}

fn fmin64(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    if a == b {
        return if a.is_sign_negative() { a } else { b };
    }
    if a < b { a } else { b }
}

fn fmax64(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    if a == b {
        return if a.is_sign_negative() { b } else { a };
    }
    if a > b { a } else { b }
}

/// The arithmetic and comparison instructions, all of which take their
/// operands from the stack and push one result.
fn numeric(op: u8, stack: &mut Vec<Value>) -> Result<(), Error> {
    macro_rules! cmp_i32 {
        ($f:expr) => {{
            let b = pop_i32(stack)?;
            let a = pop_i32(stack)?;
            stack.push(Value::I32(i32::from($f(a, b))));
        }};
    }
    macro_rules! cmp_i64 {
        ($f:expr) => {{
            let b = pop_i64(stack)?;
            let a = pop_i64(stack)?;
            stack.push(Value::I32(i32::from($f(a, b))));
        }};
    }
    macro_rules! bin_i32 {
        ($f:expr) => {{
            let b = pop_i32(stack)?;
            let a = pop_i32(stack)?;
            stack.push(Value::I32($f(a, b)));
        }};
    }
    macro_rules! bin_i64 {
        ($f:expr) => {{
            let b = pop_i64(stack)?;
            let a = pop_i64(stack)?;
            stack.push(Value::I64($f(a, b)));
        }};
    }
    macro_rules! cmp_f32 {
        ($f:expr) => {{
            let b = pop_f32(stack)?;
            let a = pop_f32(stack)?;
            stack.push(Value::I32(i32::from($f(a, b))));
        }};
    }
    macro_rules! cmp_f64 {
        ($f:expr) => {{
            let b = pop_f64(stack)?;
            let a = pop_f64(stack)?;
            stack.push(Value::I32(i32::from($f(a, b))));
        }};
    }
    macro_rules! bin_f32 {
        ($f:expr) => {{
            let b = pop_f32(stack)?;
            let a = pop_f32(stack)?;
            stack.push(Value::F32($f(a, b)));
        }};
    }
    macro_rules! bin_f64 {
        ($f:expr) => {{
            let b = pop_f64(stack)?;
            let a = pop_f64(stack)?;
            stack.push(Value::F64($f(a, b)));
        }};
    }
    macro_rules! un_f32 {
        ($f:expr) => {{
            let a = pop_f32(stack)?;
            stack.push(Value::F32($f(a)));
        }};
    }
    macro_rules! un_f64 {
        ($f:expr) => {{
            let a = pop_f64(stack)?;
            stack.push(Value::F64($f(a)));
        }};
    }

    match op {
        0x45 => {
            let a = pop_i32(stack)?;
            stack.push(Value::I32(i32::from(a == 0)));
        }
        0x46 => cmp_i32!(|a, b| a == b),
        0x47 => cmp_i32!(|a, b| a != b),
        0x48 => cmp_i32!(|a, b| a < b),
        0x49 => cmp_i32!(|a: i32, b: i32| (a as u32) < (b as u32)),
        0x4A => cmp_i32!(|a, b| a > b),
        0x4B => cmp_i32!(|a: i32, b: i32| (a as u32) > (b as u32)),
        0x4C => cmp_i32!(|a, b| a <= b),
        0x4D => cmp_i32!(|a: i32, b: i32| (a as u32) <= (b as u32)),
        0x4E => cmp_i32!(|a, b| a >= b),
        0x4F => cmp_i32!(|a: i32, b: i32| (a as u32) >= (b as u32)),
        0x50 => {
            let a = pop_i64(stack)?;
            stack.push(Value::I32(i32::from(a == 0)));
        }
        0x51 => cmp_i64!(|a, b| a == b),
        0x52 => cmp_i64!(|a, b| a != b),
        0x53 => cmp_i64!(|a, b| a < b),
        0x54 => cmp_i64!(|a: i64, b: i64| (a as u64) < (b as u64)),
        0x55 => cmp_i64!(|a, b| a > b),
        0x56 => cmp_i64!(|a: i64, b: i64| (a as u64) > (b as u64)),
        0x57 => cmp_i64!(|a, b| a <= b),
        0x58 => cmp_i64!(|a: i64, b: i64| (a as u64) <= (b as u64)),
        0x59 => cmp_i64!(|a, b| a >= b),
        0x5A => cmp_i64!(|a: i64, b: i64| (a as u64) >= (b as u64)),
        0x5B => cmp_f32!(|a: f32, b: f32| a == b),
        0x5C => cmp_f32!(|a: f32, b: f32| a != b),
        0x5D => cmp_f32!(|a: f32, b: f32| a < b),
        0x5E => cmp_f32!(|a: f32, b: f32| a > b),
        0x5F => cmp_f32!(|a: f32, b: f32| a <= b),
        0x60 => cmp_f32!(|a: f32, b: f32| a >= b),
        0x61 => cmp_f64!(|a: f64, b: f64| a == b),
        0x62 => cmp_f64!(|a: f64, b: f64| a != b),
        0x63 => cmp_f64!(|a: f64, b: f64| a < b),
        0x64 => cmp_f64!(|a: f64, b: f64| a > b),
        0x65 => cmp_f64!(|a: f64, b: f64| a <= b),
        0x66 => cmp_f64!(|a: f64, b: f64| a >= b),
        0x67 => {
            let a = pop_i32(stack)?;
            stack.push(Value::I32(a.leading_zeros() as i32));
        }
        0x68 => {
            let a = pop_i32(stack)?;
            stack.push(Value::I32(a.trailing_zeros() as i32));
        }
        0x69 => {
            let a = pop_i32(stack)?;
            stack.push(Value::I32(a.count_ones() as i32));
        }
        0x6A => bin_i32!(|a: i32, b: i32| a.wrapping_add(b)),
        0x6B => bin_i32!(|a: i32, b: i32| a.wrapping_sub(b)),
        0x6C => bin_i32!(|a: i32, b: i32| a.wrapping_mul(b)),
        0x6D => {
            let b = pop_i32(stack)?;
            let a = pop_i32(stack)?;
            if b == 0 {
                return Err(Error::DivideByZero);
            }
            if a == i32::MIN && b == -1 {
                return Err(Error::IntegerOverflow);
            }
            stack.push(Value::I32(a / b));
        }
        0x6E => {
            let b = pop_i32(stack)?;
            let a = pop_i32(stack)?;
            if b == 0 {
                return Err(Error::DivideByZero);
            }
            stack.push(Value::I32(((a as u32) / (b as u32)) as i32));
        }
        0x6F => {
            let b = pop_i32(stack)?;
            let a = pop_i32(stack)?;
            if b == 0 {
                return Err(Error::DivideByZero);
            }
            if a == i32::MIN && b == -1 {
                return Err(Error::IntegerOverflow);
            }
            stack.push(Value::I32(a % b));
        }
        0x70 => {
            let b = pop_i32(stack)?;
            let a = pop_i32(stack)?;
            if b == 0 {
                return Err(Error::DivideByZero);
            }
            stack.push(Value::I32(((a as u32) % (b as u32)) as i32));
        }
        0x71 => bin_i32!(|a: i32, b: i32| a & b),
        0x72 => bin_i32!(|a: i32, b: i32| a | b),
        0x73 => bin_i32!(|a: i32, b: i32| a ^ b),
        0x74 => bin_i32!(|a: i32, b: i32| a.wrapping_shl(b as u32)),
        0x75 => bin_i32!(|a: i32, b: i32| a.wrapping_shr(b as u32)),
        0x76 => bin_i32!(|a: i32, b: i32| ((a as u32).wrapping_shr(b as u32)) as i32),
        0x77 => bin_i32!(|a: i32, b: i32| a.rotate_left(b as u32 % 32)),
        0x78 => bin_i32!(|a: i32, b: i32| a.rotate_right(b as u32 % 32)),
        0x79 => {
            let a = pop_i64(stack)?;
            stack.push(Value::I32(a.leading_zeros() as i32));
        }
        0x7A => {
            let a = pop_i64(stack)?;
            stack.push(Value::I32(a.trailing_zeros() as i32));
        }
        0x7B => {
            let a = pop_i64(stack)?;
            stack.push(Value::I32(a.count_ones() as i32));
        }
        0x7C => bin_i64!(|a: i64, b: i64| a.wrapping_add(b)),
        0x7D => bin_i64!(|a: i64, b: i64| a.wrapping_sub(b)),
        0x7E => bin_i64!(|a: i64, b: i64| a.wrapping_mul(b)),
        0x7F => {
            let b = pop_i64(stack)?;
            let a = pop_i64(stack)?;
            if b == 0 {
                return Err(Error::DivideByZero);
            }
            if a == i64::MIN && b == -1 {
                return Err(Error::IntegerOverflow);
            }
            stack.push(Value::I64(a / b));
        }
        0x80 => {
            let b = pop_i64(stack)?;
            let a = pop_i64(stack)?;
            if b == 0 {
                return Err(Error::DivideByZero);
            }
            stack.push(Value::I64(((a as u64) / (b as u64)) as i64));
        }
        0x81 => {
            let b = pop_i64(stack)?;
            let a = pop_i64(stack)?;
            if b == 0 {
                return Err(Error::DivideByZero);
            }
            if a == i64::MIN && b == -1 {
                return Err(Error::IntegerOverflow);
            }
            stack.push(Value::I64(a % b));
        }
        0x82 => {
            let b = pop_i64(stack)?;
            let a = pop_i64(stack)?;
            if b == 0 {
                return Err(Error::DivideByZero);
            }
            stack.push(Value::I64(((a as u64) % (b as u64)) as i64));
        }
        0x83 => bin_i64!(|a: i64, b: i64| a & b),
        0x84 => bin_i64!(|a: i64, b: i64| a | b),
        0x85 => bin_i64!(|a: i64, b: i64| a ^ b),
        0x86 => bin_i64!(|a: i64, b: i64| a.wrapping_shl(b as u32)),
        0x87 => bin_i64!(|a: i64, b: i64| a.wrapping_shr(b as u32)),
        0x88 => bin_i64!(|a: i64, b: i64| ((a as u64).wrapping_shr(b as u32)) as i64),
        0x89 => bin_i64!(|a: i64, b: i64| a.rotate_left(b as u32 % 64)),
        0x8A => bin_i64!(|a: i64, b: i64| a.rotate_right(b as u32 % 64)),
        0x8B => un_f32!(f32::abs),
        0x8C => un_f32!(|a: f32| -a),
        0x8D => un_f32!(f32::ceil),
        0x8E => un_f32!(f32::floor),
        0x8F => un_f32!(f32::trunc),
        0x90 => un_f32!(|a: f32| a.round_ties_even()),
        0x91 => un_f32!(f32::sqrt),
        0x92 => bin_f32!(|a: f32, b: f32| a + b),
        0x93 => bin_f32!(|a: f32, b: f32| a - b),
        0x94 => bin_f32!(|a: f32, b: f32| a * b),
        0x95 => bin_f32!(|a: f32, b: f32| a / b),
        0x96 => bin_f32!(fmin32),
        0x97 => bin_f32!(fmax32),
        0x98 => bin_f32!(|a: f32, b: f32| a.copysign(b)),
        0x99 => un_f64!(f64::abs),
        0x9A => un_f64!(|a: f64| -a),
        0x9B => un_f64!(f64::ceil),
        0x9C => un_f64!(f64::floor),
        0x9D => un_f64!(f64::trunc),
        0x9E => un_f64!(|a: f64| a.round_ties_even()),
        0x9F => un_f64!(f64::sqrt),
        0xA0 => bin_f64!(|a: f64, b: f64| a + b),
        0xA1 => bin_f64!(|a: f64, b: f64| a - b),
        0xA2 => bin_f64!(|a: f64, b: f64| a * b),
        0xA3 => bin_f64!(|a: f64, b: f64| a / b),
        0xA4 => bin_f64!(fmin64),
        0xA5 => bin_f64!(fmax64),
        0xA6 => bin_f64!(|a: f64, b: f64| a.copysign(b)),
        0xA7 => {
            let a = pop_i64(stack)?;
            stack.push(Value::I32(a as i32));
        }
        0xA8 => {
            let a = pop_f32(stack)?;
            stack.push(Value::I32(trunc_i32(f64::from(a))?));
        }
        0xA9 => {
            let a = pop_f32(stack)?;
            stack.push(Value::I32(trunc_u32(f64::from(a))? as i32));
        }
        0xAA => {
            let a = pop_f64(stack)?;
            stack.push(Value::I32(trunc_i32(a)?));
        }
        0xAB => {
            let a = pop_f64(stack)?;
            stack.push(Value::I32(trunc_u32(a)? as i32));
        }
        0xAC => {
            let a = pop_i32(stack)?;
            stack.push(Value::I64(i64::from(a)));
        }
        0xAD => {
            let a = pop_i32(stack)?;
            stack.push(Value::I64(i64::from(a as u32)));
        }
        0xAE => {
            let a = pop_f32(stack)?;
            stack.push(Value::I64(trunc_i64(f64::from(a))?));
        }
        0xAF => {
            let a = pop_f32(stack)?;
            stack.push(Value::I64(trunc_u64(f64::from(a))? as i64));
        }
        0xB0 => {
            let a = pop_f64(stack)?;
            stack.push(Value::I64(trunc_i64(a)?));
        }
        0xB1 => {
            let a = pop_f64(stack)?;
            stack.push(Value::I64(trunc_u64(a)? as i64));
        }
        0xB2 => {
            let a = pop_i32(stack)?;
            stack.push(Value::F32(a as f32));
        }
        0xB3 => {
            let a = pop_i32(stack)?;
            stack.push(Value::F32(a as u32 as f32));
        }
        0xB4 => {
            let a = pop_i64(stack)?;
            stack.push(Value::F32(a as f32));
        }
        0xB5 => {
            let a = pop_i64(stack)?;
            stack.push(Value::F32(a as u64 as f32));
        }
        0xB6 => {
            let a = pop_f64(stack)?;
            stack.push(Value::F32(a as f32));
        }
        0xB7 => {
            let a = pop_i32(stack)?;
            stack.push(Value::F64(f64::from(a)));
        }
        0xB8 => {
            let a = pop_i32(stack)?;
            stack.push(Value::F64(f64::from(a as u32)));
        }
        0xB9 => {
            let a = pop_i64(stack)?;
            stack.push(Value::F64(a as f64));
        }
        0xBA => {
            let a = pop_i64(stack)?;
            stack.push(Value::F64(a as u64 as f64));
        }
        0xBB => {
            let a = pop_f32(stack)?;
            stack.push(Value::F64(f64::from(a)));
        }
        0xBC => {
            let a = pop_f32(stack)?;
            stack.push(Value::I32(a.to_bits() as i32));
        }
        0xBD => {
            let a = pop_f64(stack)?;
            stack.push(Value::I64(a.to_bits() as i64));
        }
        0xBE => {
            let a = pop_i32(stack)?;
            stack.push(Value::F32(f32::from_bits(a as u32)));
        }
        0xBF => {
            let a = pop_i64(stack)?;
            stack.push(Value::F64(f64::from_bits(a as u64)));
        }
        0xC0 => {
            let a = pop_i32(stack)?;
            stack.push(Value::I32(i32::from(a as i8)));
        }
        0xC1 => {
            let a = pop_i32(stack)?;
            stack.push(Value::I32(i32::from(a as i16)));
        }
        0xC2 => {
            let a = pop_i64(stack)?;
            stack.push(Value::I64(i64::from(a as i8)));
        }
        0xC3 => {
            let a = pop_i64(stack)?;
            stack.push(Value::I64(i64::from(a as i16)));
        }
        0xC4 => {
            let a = pop_i64(stack)?;
            stack.push(Value::I64(i64::from(a as i32)));
        }
        other => return Err(Error::UnsupportedOpcode(other)),
    }
    Ok(())
}

/// The saturating conversions: NaN becomes zero and out-of-range values clamp,
/// which is the whole difference between these and the trapping ones.
fn saturating(sub: u32, stack: &mut Vec<Value>) -> Result<(), Error> {
    match sub {
        0 => {
            let a = pop_f32(stack)?;
            stack.push(Value::I32(sat_i32(f64::from(a))));
        }
        1 => {
            let a = pop_f32(stack)?;
            stack.push(Value::I32(sat_u32(f64::from(a)) as i32));
        }
        2 => {
            let a = pop_f64(stack)?;
            stack.push(Value::I32(sat_i32(a)));
        }
        3 => {
            let a = pop_f64(stack)?;
            stack.push(Value::I32(sat_u32(a) as i32));
        }
        4 => {
            let a = pop_f32(stack)?;
            stack.push(Value::I64(sat_i64(f64::from(a))));
        }
        5 => {
            let a = pop_f32(stack)?;
            stack.push(Value::I64(sat_u64(f64::from(a)) as i64));
        }
        6 => {
            let a = pop_f64(stack)?;
            stack.push(Value::I64(sat_i64(a)));
        }
        7 => {
            let a = pop_f64(stack)?;
            stack.push(Value::I64(sat_u64(a) as i64));
        }
        other => return Err(Error::UnsupportedOpcode(other as u8)),
    }
    Ok(())
}

fn sat_i32(value: f64) -> i32 {
    if value.is_nan() {
        return 0;
    }
    let truncated = value.trunc();
    if truncated <= f64::from(i32::MIN) {
        i32::MIN
    } else if truncated >= f64::from(i32::MAX) {
        i32::MAX
    } else {
        truncated as i32
    }
}

fn sat_u32(value: f64) -> u32 {
    if value.is_nan() {
        return 0;
    }
    let truncated = value.trunc();
    if truncated <= 0.0 {
        0
    } else if truncated >= f64::from(u32::MAX) {
        u32::MAX
    } else {
        truncated as u32
    }
}

fn sat_i64(value: f64) -> i64 {
    if value.is_nan() {
        return 0;
    }
    let truncated = value.trunc();
    let bound = (1u64 << 63) as f64;
    if truncated <= -bound {
        i64::MIN
    } else if truncated >= bound {
        i64::MAX
    } else {
        truncated as i64
    }
}

fn sat_u64(value: f64) -> u64 {
    if value.is_nan() {
        return 0;
    }
    let truncated = value.trunc();
    let bound = (1u128 << 64) as f64;
    if truncated <= 0.0 {
        0
    } else if truncated >= bound {
        u64::MAX
    } else {
        truncated as u64
    }
}

/// `i32.trunc_f64_s` and friends trap on NaN and on values outside the target
/// range, which is why they are functions rather than `as` casts: a cast
/// saturates, and saturating is not what the format says.
fn trunc_i32(value: f64) -> Result<i32, Error> {
    if value.is_nan() {
        return Err(Error::BadModule("a conversion of NaN"));
    }
    let truncated = value.trunc();
    if truncated < f64::from(i32::MIN) || truncated > f64::from(i32::MAX) {
        return Err(Error::IntegerOverflow);
    }
    Ok(truncated as i32)
}

fn trunc_u32(value: f64) -> Result<u32, Error> {
    if value.is_nan() {
        return Err(Error::BadModule("a conversion of NaN"));
    }
    let truncated = value.trunc();
    if truncated < 0.0 || truncated > f64::from(u32::MAX) {
        return Err(Error::IntegerOverflow);
    }
    Ok(truncated as u32)
}

fn trunc_i64(value: f64) -> Result<i64, Error> {
    if value.is_nan() {
        return Err(Error::BadModule("a conversion of NaN"));
    }
    let truncated = value.trunc();
    // 2^63, written as a shift so the literal is exact rather than a decimal
    // that rounds to it.
    let bound = (1u64 << 63) as f64;
    if truncated < -bound || truncated >= bound {
        return Err(Error::IntegerOverflow);
    }
    Ok(truncated as i64)
}

fn trunc_u64(value: f64) -> Result<u64, Error> {
    if value.is_nan() {
        return Err(Error::BadModule("a conversion of NaN"));
    }
    let truncated = value.trunc();
    let bound = (1u128 << 64) as f64;
    if truncated < 0.0 || truncated >= bound {
        return Err(Error::IntegerOverflow);
    }
    Ok(truncated as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wasm::{FuncType, Import, ImportKind};

    /// Builds a module with one function of the given signature and body, and
    /// exports it as `run`. Every test below is a body plus an assertion, which
    /// is the whole point of having a builder: the binary encoding is noise
    /// around the instruction under test.
    fn module(params: &[ValType], results: &[ValType], body: &[u8]) -> Module {
        let mut code = body.to_vec();
        code.push(0x0B);
        Module {
            types: vec![FuncType { params: params.to_vec(), results: results.to_vec() }],
            imports: vec![],
            funcs: vec![Func {
                type_index: 0,
                locals: vec![],
                jumps: jumps(&code).unwrap(),
                code,
            }],
            exports: vec![crate::wasm::Export {
                name: "run".into(),
                kind: crate::wasm::ExportKind::Func(0),
            }],
            memory: Some(crate::wasm::Memory { min: 1, max: None }),
            table: None,
            globals: vec![],
            elements: vec![],
            data: vec![],
            start: None,
        }
    }

    fn run(params: &[ValType], results: &[ValType], body: &[u8], args: &[Value]) -> Result<Vec<Value>, Error> {
        let mut instance = Instance::new(module(params, results, body), Box::new(NullHost::default()))?;
        instance.call_export("run", args)
    }

    fn i32_of(body: &[u8]) -> i32 {
        match run(&[], &[ValType::I32], body, &[]).unwrap().as_slice() {
            [Value::I32(v)] => *v,
            other => panic!("expected one i32, got {other:?}"),
        }
    }

    #[test]
    fn a_constant_comes_back_out() {
        // i32.const 42
        assert_eq!(i32_of(&[0x41, 42]), 42);
    }

    #[test]
    fn addition_works() {
        // i32.const 2, i32.const 3, i32.add
        assert_eq!(i32_of(&[0x41, 2, 0x41, 3, 0x6A]), 5);
    }

    #[test]
    fn a_negative_constant_is_signed() {
        // i32.const -1 encodes as 0x7F
        assert_eq!(i32_of(&[0x41, 0x7F]), -1);
    }

    #[test]
    fn a_local_round_trips() {
        // (local i32) local.get 0, local.set 1, local.get 1
        let mut code = vec![0x20, 0, 0x21, 1, 0x20, 1];
        code.push(0x0B);
        let mut module = module(&[ValType::I32], &[ValType::I32], &code);
        module.funcs[0].locals = vec![ValType::I32];
        module.funcs[0].jumps = jumps(&code).unwrap();
        let mut instance = Instance::new(module, Box::new(NullHost::default())).unwrap();
        assert_eq!(instance.call_export("run", &[Value::I32(7)]).unwrap(), vec![Value::I32(7)]);
    }

    #[test]
    fn an_if_takes_the_true_branch() {
        // if (result i32) i32.const 1 else i32.const 2 end
        let body = [0x41, 1, 0x04, 0x7F, 0x41, 1, 0x05, 0x41, 2, 0x0B];
        assert_eq!(i32_of(&body), 1);
    }

    #[test]
    fn an_if_takes_the_false_branch() {
        let body = [0x41, 0, 0x04, 0x7F, 0x41, 1, 0x05, 0x41, 2, 0x0B];
        assert_eq!(i32_of(&body), 2);
    }

    #[test]
    fn an_if_without_an_else_skips_to_the_end() {
        // if (empty) i32.const 1 drop end, then i32.const 9
        let body = [0x41, 0, 0x04, 0x40, 0x41, 1, 0x1A, 0x0B, 0x41, 9];
        assert_eq!(i32_of(&body), 9);
    }

    #[test]
    fn a_block_branches_to_its_end() {
        // block (result i32) i32.const 5 br 0 i32.const 6 end
        let body = [0x02, 0x7F, 0x41, 5, 0x0C, 0, 0x41, 6, 0x0B];
        assert_eq!(i32_of(&body), 5);
    }

    #[test]
    fn a_loop_branches_back_to_its_start() {
        // A countdown: local 0 starts at 3, decremented until zero.
        // block
        //   loop
        //     local.get 0
        //     i32.eqz
        //     br_if 1
        //     local.get 0
        //     i32.const 1
        //     i32.sub
        //     local.set 0
        //     br 0
        //   end
        // end
        // local.get 0
        let mut code = vec![
            0x02, 0x40, 0x03, 0x40, 0x20, 0, 0x45, 0x0D, 1, 0x20, 0, 0x41, 1, 0x6B, 0x21, 0, 0x0C,
            0, 0x0B, 0x0B, 0x20, 0,
        ];
        code.push(0x0B);
        let mut module = module(&[ValType::I32], &[ValType::I32], &code);
        module.funcs[0].locals = vec![];
        module.funcs[0].jumps = jumps(&code).unwrap();
        let mut instance = Instance::new(module, Box::new(NullHost::default())).unwrap();
        assert_eq!(instance.call_export("run", &[Value::I32(3)]).unwrap(), vec![Value::I32(0)]);
    }

    #[test]
    fn a_br_table_picks_the_indexed_target() {
        // block block block br_table 0 1 2 (index) end i32.const 1 return end
        // i32.const 2 return end i32.const 3
        let body = [
            0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x20, 0, 0x0E, 2, 0, 1, 2, 0x0B, 0x41, 1, 0x0F,
            0x0B, 0x41, 2, 0x0F, 0x0B, 0x41, 3,
        ];
        let mut code = body.to_vec();
        code.push(0x0B);
        let mut module = module(&[ValType::I32], &[ValType::I32], &code);
        module.funcs[0].jumps = jumps(&code).unwrap();
        let mut instance = Instance::new(module, Box::new(NullHost::default())).unwrap();
        assert_eq!(instance.call_export("run", &[Value::I32(0)]).unwrap(), vec![Value::I32(1)]);
        assert_eq!(instance.call_export("run", &[Value::I32(1)]).unwrap(), vec![Value::I32(2)]);
        assert_eq!(instance.call_export("run", &[Value::I32(9)]).unwrap(), vec![Value::I32(3)]);
    }

    #[test]
    fn a_call_passes_arguments_and_returns_a_result() {
        // Two functions: `add` (index 0) and `run` (index 1) which calls it.
        let add = vec![0x20, 0, 0x20, 1, 0x6A, 0x0B];
        let run = vec![0x41, 4, 0x41, 5, 0x10, 0, 0x0B];
        let module = Module {
            types: vec![
                FuncType { params: vec![ValType::I32, ValType::I32], results: vec![ValType::I32] },
                FuncType { params: vec![], results: vec![ValType::I32] },
            ],
            imports: vec![],
            funcs: vec![
                Func { type_index: 0, locals: vec![], jumps: jumps(&add).unwrap(), code: add },
                Func { type_index: 1, locals: vec![], jumps: jumps(&run).unwrap(), code: run },
            ],
            exports: vec![crate::wasm::Export {
                name: "run".into(),
                kind: crate::wasm::ExportKind::Func(1),
            }],
            memory: None,
            table: None,
            globals: vec![],
            elements: vec![],
            data: vec![],
            start: None,
        };
        let mut instance = Instance::new(module, Box::new(NullHost::default())).unwrap();
        assert_eq!(instance.call_export("run", &[]).unwrap(), vec![Value::I32(9)]);
    }

    /// A host that records into a handle the test keeps, so the calls can be
    /// inspected after the instance has taken ownership of it.
    type Calls = std::rc::Rc<std::cell::RefCell<Vec<(String, Vec<Value>)>>>;

    #[derive(Clone, Default)]
    struct Recorder(Calls);

    impl Host for Recorder {
        fn call(
            &mut self,
            name: &str,
            args: &[Value],
            _memory: &mut [u8],
        ) -> Result<Option<Value>, Error> {
            self.0.borrow_mut().push((name.to_owned(), args.to_vec()));
            Ok(None)
        }
    }

    #[test]
    fn a_host_function_is_called_by_name_with_its_arguments() {
        // i32.const 7, call 0
        let body = [0x41, 7, 0x10, 0, 0x0B];
        let module = Module {
            types: vec![
                FuncType { params: vec![ValType::I32], results: vec![] },
                FuncType { params: vec![], results: vec![] },
            ],
            imports: vec![Import {
                module: "npp".into(),
                name: "log".into(),
                kind: ImportKind::Func(0),
            }],
            funcs: vec![Func {
                type_index: 1,
                locals: vec![],
                jumps: jumps(&body).unwrap(),
                code: body.to_vec(),
            }],
            exports: vec![crate::wasm::Export {
                name: "run".into(),
                kind: crate::wasm::ExportKind::Func(1),
            }],
            memory: None,
            table: None,
            globals: vec![],
            elements: vec![],
            data: vec![],
            start: None,
        };
        let recorder = Recorder::default();
        let mut instance = Instance::new(module, Box::new(recorder.clone())).unwrap();
        instance.call_export("run", &[]).unwrap();
        assert_eq!(
            *recorder.0.borrow(),
            vec![("log".to_string(), vec![Value::I32(7)])]
        );
    }

    #[test]
    fn a_host_function_can_return_a_value() {
        struct Answer;
        impl Host for Answer {
            fn call(
                &mut self,
                _name: &str,
                _args: &[Value],
                _memory: &mut [u8],
            ) -> Result<Option<Value>, Error> {
                Ok(Some(Value::I32(11)))
            }
        }
        // call 0
        let body = [0x10, 0, 0x0B];
        let module = Module {
            types: vec![FuncType { params: vec![], results: vec![ValType::I32] }],
            imports: vec![Import {
                module: "npp".into(),
                name: "text_len".into(),
                kind: ImportKind::Func(0),
            }],
            funcs: vec![Func {
                type_index: 0,
                locals: vec![],
                jumps: jumps(&body).unwrap(),
                code: body.to_vec(),
            }],
            exports: vec![crate::wasm::Export {
                name: "run".into(),
                kind: crate::wasm::ExportKind::Func(1),
            }],
            memory: None,
            table: None,
            globals: vec![],
            elements: vec![],
            data: vec![],
            start: None,
        };
        let mut instance = Instance::new(module, Box::new(Answer)).unwrap();
        assert_eq!(instance.call_export("run", &[]).unwrap(), vec![Value::I32(11)]);
    }

    #[test]
    fn memory_loads_and_stores_round_trip() {
        // i32.const 0, i32.const 99, i32.store, i32.const 0, i32.load
        let body = [0x41, 0, 0x41, 0xE3, 0x00, 0x36, 2, 0, 0x41, 0, 0x28, 2, 0];
        assert_eq!(i32_of(&body), 99);
    }

    #[test]
    fn a_load_outside_memory_traps() {
        // i32.const 65535, i32.load
        let body = [0x41, 0xFF, 0xFF, 0x03, 0x28, 2, 0];
        assert_eq!(run(&[], &[ValType::I32], &body, &[]), Err(Error::OutOfBounds));
    }

    #[test]
    fn a_store_outside_memory_traps() {
        // i32.const 65534, i32.const 1, i32.store
        let body = [0x41, 0xFF, 0xFF, 0x03, 0x41, 1, 0x36, 2, 0];
        assert_eq!(run(&[], &[], &body, &[]), Err(Error::OutOfBounds));
    }

    #[test]
    fn memory_size_reports_the_pages() {
        // memory.size
        assert_eq!(i32_of(&[0x3F, 0]), 1);
    }

    #[test]
    fn memory_grow_returns_the_old_size() {
        // i32.const 1, memory.grow, drop, memory.size
        assert_eq!(i32_of(&[0x41, 1, 0x40, 0, 0x1A, 0x3F, 0]), 2);
    }

    #[test]
    fn memory_grow_past_the_limit_fails() {
        // i32.const 1000, memory.grow
        assert_eq!(i32_of(&[0x41, 0xE8, 0x07, 0x40, 0]), -1);
    }

    #[test]
    fn division_by_zero_traps() {
        let body = [0x41, 1, 0x41, 0, 0x6D];
        assert_eq!(run(&[], &[ValType::I32], &body, &[]), Err(Error::DivideByZero));
    }

    #[test]
    fn the_signed_division_overflow_traps() {
        // i32.const i32::MIN, i32.const -1, i32.div_s
        let body = [0x41, 0x80, 0x80, 0x80, 0x80, 0x78, 0x41, 0x7F, 0x6D];
        assert_eq!(run(&[], &[ValType::I32], &body, &[]), Err(Error::IntegerOverflow));
    }

    #[test]
    fn unreachable_traps() {
        assert_eq!(run(&[], &[], &[0x00], &[]), Err(Error::Unreachable));
    }

    #[test]
    fn a_shift_masks_its_count() {
        // i32.const 1, i32.const 33, i32.shl -- 33 & 31 is 1, so the result is 2
        assert_eq!(i32_of(&[0x41, 1, 0x41, 33, 0x74]), 2);
    }

    #[test]
    fn an_unsigned_comparison_differs_from_a_signed_one() {
        // -1 < 0 signed, but not unsigned
        assert_eq!(i32_of(&[0x41, 0x7F, 0x41, 0, 0x48]), 1);
        assert_eq!(i32_of(&[0x41, 0x7F, 0x41, 0, 0x49]), 0);
    }

    #[test]
    fn a_float_comparison_handles_nan() {
        // f32.const nan, f32.const nan, f32.eq -- NaN is never equal to itself
        let body = [0x43, 0, 0, 0xC0, 0x7F, 0x43, 0, 0, 0xC0, 0x7F, 0x5B];
        assert_eq!(i32_of(&body), 0);
    }

    #[test]
    fn a_float_min_propagates_nan() {
        let body = [0x43, 0, 0, 0xC0, 0x7F, 0x43, 0, 0, 0x80, 0x3F, 0x96];
        match run(&[], &[ValType::F32], &body, &[]).unwrap().as_slice() {
            [Value::F32(v)] => assert!(v.is_nan()),
            other => panic!("expected one f32, got {other:?}"),
        }
    }

    #[test]
    fn a_conversion_of_nan_traps() {
        // f32.const nan, i32.trunc_f32_s
        let body = [0x43, 0, 0, 0xC0, 0x7F, 0xA8];
        assert!(matches!(run(&[], &[ValType::I32], &body, &[]), Err(Error::BadModule(_))));
    }

    #[test]
    fn a_conversion_out_of_range_traps() {
        // f64.const 1e30, i32.trunc_f64_s
        let body = [0x44, 0, 0, 0, 0, 0, 0, 0x9C, 0x46, 0xAA];
        assert_eq!(run(&[], &[ValType::I32], &body, &[]), Err(Error::IntegerOverflow));
    }

    #[test]
    fn a_select_picks_by_condition() {
        // i32.const 1, i32.const 2, i32.const 1, select
        assert_eq!(i32_of(&[0x41, 1, 0x41, 2, 0x41, 1, 0x1B]), 1);
        assert_eq!(i32_of(&[0x41, 1, 0x41, 2, 0x41, 0, 0x1B]), 2);
    }

    #[test]
    fn a_missing_export_is_reported() {
        let mut instance =
            Instance::new(module(&[], &[], &[0x41, 1]), Box::new(NullHost::default())).unwrap();
        assert_eq!(
            instance.call_export("nope", &[]),
            Err(Error::NoSuchExport("nope".into()))
        );
    }

    #[test]
    fn a_call_to_a_missing_function_is_reported() {
        // call 9
        let body = [0x10, 9];
        assert_eq!(run(&[], &[], &body, &[]), Err(Error::NoSuchFunction(9)));
    }

    #[test]
    fn a_runaway_loop_runs_out_of_fuel() {
        // loop br 0 end
        let body = [0x03, 0x40, 0x0C, 0, 0x0B];
        let mut instance =
            Instance::new(module(&[], &[], &body), Box::new(NullHost::default())).unwrap();
        instance.fuel = 1000;
        assert_eq!(instance.call_export("run", &[]), Err(Error::OutOfFuel));
    }

    #[test]
    fn deep_recursion_is_stopped() {
        // A function that calls itself: call 0
        let body = [0x10, 0];
        let mut instance =
            Instance::new(module(&[], &[], &body), Box::new(NullHost::default())).unwrap();
        assert_eq!(instance.call_export("run", &[]), Err(Error::StackExhausted));
    }

    #[test]
    fn a_data_segment_is_copied_into_memory() {
        let mut module = module(&[], &[ValType::I32], &[0x41, 0, 0x28, 2, 0]);
        module.data = vec![crate::wasm::Data { offset: 0, bytes: vec![7, 0, 0, 0] }];
        let mut instance = Instance::new(module, Box::new(NullHost::default())).unwrap();
        assert_eq!(instance.call_export("run", &[]).unwrap(), vec![Value::I32(7)]);
    }

    #[test]
    fn a_data_segment_past_the_end_is_rejected() {
        let mut module = module(&[], &[], &[]);
        module.memory = Some(crate::wasm::Memory { min: 1, max: None });
        module.data = vec![crate::wasm::Data { offset: 65_535, bytes: vec![1, 2] }];
        assert_eq!(
            Instance::new(module, Box::new(NullHost::default())).err(),
            Some(Error::OutOfBounds)
        );
    }

    #[test]
    fn a_module_asking_for_too_much_memory_is_rejected() {
        let mut module = module(&[], &[], &[]);
        module.memory = Some(crate::wasm::Memory { min: MAX_PAGES + 1, max: None });
        assert!(matches!(
            Instance::new(module, Box::new(NullHost::default())),
            Err(Error::Host(_))
        ));
    }

    #[test]
    fn a_start_function_runs_at_instantiation() {
        let mut module = module(&[], &[], &[0x41, 1, 0x1A]);
        module.start = Some(0);
        assert!(Instance::new(module, Box::new(NullHost::default())).is_ok());
    }

    #[test]
    fn the_jump_table_finds_a_blocks_end() {
        // block end
        let table = jumps(&[0x02, 0x40, 0x0B]).unwrap();
        assert_eq!(table[0].end, 3);
        assert_eq!(table[0].else_at, usize::MAX);
    }

    #[test]
    fn the_jump_table_finds_an_else() {
        // if else end
        let table = jumps(&[0x04, 0x40, 0x05, 0x0B]).unwrap();
        assert_eq!(table[0].else_at, 2);
        assert_eq!(table[0].end, 4);
    }

    #[test]
    fn an_unclosed_block_is_rejected() {
        assert!(matches!(jumps(&[0x02, 0x40]), Err(Error::BadModule(_))));
    }

    #[test]
    fn an_else_outside_a_block_is_rejected() {
        assert!(matches!(jumps(&[0x05, 0x0B]), Err(Error::BadModule(_))));
    }

    #[test]
    fn an_unknown_opcode_is_rejected() {
        // 0x27 is not assigned to anything.
        assert_eq!(step(&[0x27], &mut 0), Err(Error::UnsupportedOpcode(0x27)));
    }

    #[test]
    fn an_unknown_prefixed_opcode_is_rejected() {
        // 0xFC 99 is not assigned to anything.
        assert!(matches!(step(&[0xFC, 99], &mut 0), Err(Error::Unsupported(_))));
    }

    #[test]
    fn call_indirect_is_decoded() {
        assert_eq!(
            step(&[0x11, 2, 0], &mut 0),
            Ok(Op::CallIndirect { type_index: 2, table: 0 })
        );
    }

    #[test]
    fn a_global_access_is_decoded() {
        assert_eq!(step(&[0x23, 1], &mut 0), Ok(Op::GlobalGet(1)));
        assert_eq!(step(&[0x24, 1], &mut 0), Ok(Op::GlobalSet(1)));
    }

    #[test]
    fn a_global_round_trips() {
        // global.get 0, i32.const 5, i32.add, global.set 0, global.get 0
        let body = [0x23, 0, 0x41, 5, 0x6A, 0x24, 0, 0x23, 0];
        let mut module = module(&[], &[ValType::I32], &body);
        module.globals = vec![crate::wasm::Global {
            ty: ValType::I32,
            mutable: true,
            init: crate::wasm::Init::Value(Value::I32(37)),
        }];
        let mut instance = Instance::new(module, Box::new(NullHost::default())).unwrap();
        assert_eq!(instance.call_export("run", &[]).unwrap(), vec![Value::I32(42)]);
        assert_eq!(instance.globals(), &[Value::I32(42)]);
    }

    #[test]
    fn an_indirect_call_reaches_the_table() {
        // Two functions: `double` (index 0) and `run` (index 1), which calls
        // through table slot 0.
        let double = vec![0x20, 0, 0x41, 2, 0x6C, 0x0B];
        let run = vec![0x41, 21, 0x41, 0, 0x11, 0, 0, 0x0B];
        let module = Module {
            types: vec![
                FuncType { params: vec![ValType::I32], results: vec![ValType::I32] },
                FuncType { params: vec![], results: vec![ValType::I32] },
            ],
            imports: vec![],
            funcs: vec![
                Func { type_index: 0, locals: vec![], jumps: jumps(&double).unwrap(), code: double },
                Func { type_index: 1, locals: vec![], jumps: jumps(&run).unwrap(), code: run },
            ],
            exports: vec![crate::wasm::Export {
                name: "run".into(),
                kind: crate::wasm::ExportKind::Func(1),
            }],
            memory: None,
            table: Some(crate::wasm::Table { min: 1, max: None }),
            globals: vec![],
            elements: vec![crate::wasm::Element { offset: 0, funcs: vec![Some(0)] }],
            data: vec![],
            start: None,
        };
        let mut instance = Instance::new(module, Box::new(NullHost::default())).unwrap();
        assert_eq!(instance.call_export("run", &[]).unwrap(), vec![Value::I32(42)]);
    }

    #[test]
    fn an_indirect_call_to_an_empty_slot_traps() {
        let run = vec![0x41, 0, 0x11, 0, 0, 0x0B];
        let module = Module {
            types: vec![FuncType { params: vec![], results: vec![] }],
            imports: vec![],
            funcs: vec![Func {
                type_index: 0,
                locals: vec![],
                jumps: jumps(&run).unwrap(),
                code: run,
            }],
            exports: vec![crate::wasm::Export {
                name: "run".into(),
                kind: crate::wasm::ExportKind::Func(0),
            }],
            memory: None,
            table: Some(crate::wasm::Table { min: 1, max: None }),
            globals: vec![],
            elements: vec![],
            data: vec![],
            start: None,
        };
        let mut instance = Instance::new(module, Box::new(NullHost::default())).unwrap();
        assert!(matches!(instance.call_export("run", &[]), Err(Error::BadModule(_))));
    }

    #[test]
    fn memory_copy_moves_bytes() {
        // Store 7 at 0, copy 4 bytes from 0 to 8, load from 8.
        let body = [
            0x41, 0, 0x41, 7, 0x36, 2, 0, 0x41, 8, 0x41, 0, 0x41, 4, 0xFC, 10, 0, 0, 0x41, 8,
            0x28, 2, 0,
        ];
        assert_eq!(i32_of(&body), 7);
    }

    #[test]
    fn memory_fill_writes_a_byte() {
        // Fill 4 bytes at 0 with 0x41, then load.
        let body = [0x41, 0, 0x41, 0xC1, 0x00, 0x41, 4, 0xFC, 11, 0, 0x41, 0, 0x28, 2, 0];
        assert_eq!(i32_of(&body), 0x41414141);
    }

    #[test]
    fn a_saturating_conversion_clamps_instead_of_trapping() {
        // f64.const 1e30, i32.trunc_sat_f64_s
        let body = [0x44, 0, 0, 0, 0, 0, 0, 0x9C, 0x46, 0xFC, 2];
        assert_eq!(i32_of(&body), i32::MAX);
    }

    #[test]
    fn a_saturating_conversion_of_nan_is_zero() {
        // f64.const nan, i32.trunc_sat_f64_s
        let body = [0x44, 0, 0, 0, 0, 0, 0, 0xF8, 0x7F, 0xFC, 2];
        assert_eq!(i32_of(&body), 0);
    }

    #[test]
    fn a_reference_can_be_null_checked() {
        // ref.null funcref, ref.is_null
        assert_eq!(i32_of(&[0xD0, 0x70, 0xD1]), 1);
        // ref.func 0, ref.is_null
        assert_eq!(i32_of(&[0xD2, 0, 0xD1]), 0);
    }
}
