//! A WebAssembly host for editor plugins.
//!
//! Notepad++ plugins are DLLs loaded into the process. That is a Windows
//! answer to a Windows problem: it needs a compiler per platform, it gives the
//! plugin the run of the address space, and a bad plugin takes the editor down
//! with it. This module takes the other road. A plugin is a `.wasm` file, it
//! runs in an interpreter with a bounded stack, bounded memory, and a step
//! budget, and the only way it can touch the editor is through the handful of
//! host functions in [`abi`].
//!
//! The interpreter is written here rather than pulled from a crate because the
//! build is offline and no WebAssembly runtime is vendored. That turned out to
//! be a reasonable trade: the subset a text-editing plugin needs is small, and
//! owning it means the sandbox limits are ours to set rather than a feature
//! flag on somebody else's engine.
//!
//! # What is supported
//!
//! The MVP instruction set minus the parts that need a table: `block`, `loop`,
//! `if`/`else`, `br`, `br_if`, `br_table`, `return`, `call`, `drop`, `select`,
//! locals, one memory, all the integer and floating-point arithmetic and
//! comparison operators, and the conversions between them. `call_indirect`,
//! globals, tables, and the reference types are rejected at load with
//! [`Error::Unsupported`] naming what was found.
//!
//! # What is not
//!
//! There is no validator. Values carry their type at run time and an
//! instruction that finds the wrong one traps, so a malformed module fails on
//! the instruction that is wrong rather than at load. There is no JIT: a
//! command that walks a large document is interpreted, which is slow enough to
//! notice and fast enough for the commands plugins actually implement.
//!
//! # Shape
//!
//! [`Module::parse`] reads the bytes. [`Instance`] gives the module a memory
//! and a [`Host`] to call out to. [`abi::Plugin`] wraps an instance in the
//! calling convention this editor defines -- a name, a list of commands, and a
//! `run` entry point -- so the app never touches the raw module.

pub mod abi;
pub mod binary;
pub mod exec;

use std::fmt;

pub use abi::Plugin;
pub use binary::Reader;
pub use exec::{Host, Instance, NullHost};

/// A WebAssembly value. The four numeric types, and nothing else: the
/// reference types are part of the proposal this interpreter does not
/// implement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    /// A function reference: an index into the function index space, or
    /// nothing. Only `call_indirect` and the element segments use it.
    FuncRef(Option<u32>),
}

impl Value {
    pub fn ty(self) -> ValType {
        match self {
            Value::I32(_) => ValType::I32,
            Value::I64(_) => ValType::I64,
            Value::F32(_) => ValType::F32,
            Value::F64(_) => ValType::F64,
            Value::FuncRef(_) => ValType::FuncRef,
        }
    }

    /// The value as an `i32`, or `None` if it is not one. Used by the host
    /// functions, which all take and return `i32`.
    pub fn as_i32(self) -> Option<i32> {
        match self {
            Value::I32(v) => Some(v),
            _ => None,
        }
    }

    /// The value as a `u32` bit pattern, which is how an `i32` is used when it
    /// is really an address or a length.
    pub fn as_u32(self) -> Option<u32> {
        self.as_i32().map(|v| v as u32)
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::I32(v) => write!(f, "{v}"),
            Value::I64(v) => write!(f, "{v}"),
            Value::F32(v) => write!(f, "{v}"),
            Value::F64(v) => write!(f, "{v}"),
            Value::FuncRef(Some(index)) => write!(f, "func {index}"),
            Value::FuncRef(None) => f.write_str("null"),
        }
    }
}

/// A value type, as a function signature writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValType {
    I32,
    I64,
    F32,
    F64,
    /// The only reference type the interpreter carries, because the only one
    /// the format needs for an indirect call.
    FuncRef,
}

impl fmt::Display for ValType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ValType::I32 => "i32",
            ValType::I64 => "i64",
            ValType::F32 => "f32",
            ValType::F64 => "f64",
            ValType::FuncRef => "funcref",
        })
    }
}

/// A function signature.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FuncType {
    pub params: Vec<ValType>,
    pub results: Vec<ValType>,
}

/// A memory's declared size, in 64 KiB pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Memory {
    pub min: u32,
    pub max: Option<u32>,
}

/// A table: an array of function references, indexed by `call_indirect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Table {
    pub min: u32,
    pub max: Option<u32>,
}

/// A global variable and how it starts.
#[derive(Debug, Clone, PartialEq)]
pub struct Global {
    pub ty: ValType,
    pub mutable: bool,
    pub init: Init,
}

/// A global's initial value: a constant, or another global's value.
#[derive(Debug, Clone, PartialEq)]
pub enum Init {
    Value(Value),
    Global(u32),
}

/// One element segment: function indices written into the table at `offset`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub offset: u32,
    pub funcs: Vec<Option<u32>>,
}

/// What an import brings in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportKind {
    Func(u32),
    Table(Table),
    Memory(Memory),
}

/// One entry of the import section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    pub module: String,
    pub name: String,
    pub kind: ImportKind,
}

/// What an export offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportKind {
    Func(u32),
    Table(u32),
    Memory(u32),
}

/// One entry of the export section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Export {
    pub name: String,
    pub kind: ExportKind,
}

/// A data segment: bytes copied into memory at instantiation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Data {
    pub offset: u32,
    pub bytes: Vec<u8>,
}

/// A defined function: its signature, its locals, its body, and the jump table
/// that lets a branch find its target without rescanning the body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Func {
    pub type_index: u32,
    pub locals: Vec<ValType>,
    pub code: Vec<u8>,
    pub jumps: Vec<exec::Jump>,
}

/// A parsed module. Everything the interpreter needs and nothing it does not:
/// names, custom sections, and debug info are dropped at parse time.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Module {
    pub types: Vec<FuncType>,
    pub imports: Vec<Import>,
    pub funcs: Vec<Func>,
    pub exports: Vec<Export>,
    pub memory: Option<Memory>,
    pub table: Option<Table>,
    pub globals: Vec<Global>,
    pub elements: Vec<Element>,
    pub data: Vec<Data>,
    pub start: Option<u32>,
}

impl Module {
    /// The type indices of the imported functions, in index order. They come
    /// before the defined ones in the function index space, which is why this
    /// is a separate list rather than a field on [`Func`].
    pub fn imported_funcs(&self) -> Vec<u32> {
        self.imports
            .iter()
            .filter_map(|import| match import.kind {
                ImportKind::Func(index) => Some(index),
                ImportKind::Table(_) | ImportKind::Memory(_) => None,
            })
            .collect()
    }

    /// The signature of the function at `index` in the function index space,
    /// imported functions first.
    pub fn func_type(&self, index: u32) -> Option<&FuncType> {
        let imported = self.imported_funcs();
        let type_index = if (index as usize) < imported.len() {
            imported[index as usize]
        } else {
            self.funcs.get(index as usize - imported.len())?.type_index
        };
        self.types.get(type_index as usize)
    }

    /// The import that supplies the function at `index`, if it is imported.
    pub fn imported_func(&self, index: u32) -> Option<&Import> {
        let mut seen = 0;
        for import in &self.imports {
            if matches!(import.kind, ImportKind::Func(_)) {
                if seen == index {
                    return Some(import);
                }
                seen += 1;
            }
        }
        None
    }

    /// The function index an export names, if it names one.
    pub fn export_func(&self, name: &str) -> Option<u32> {
        self.exports.iter().find_map(|export| match export.kind {
            ExportKind::Func(index) if export.name == name => Some(index),
            _ => None,
        })
    }

    /// Every function export, in declaration order. The plugin loader uses it
    /// to report what a module offers when it is missing an entry point.
    pub fn func_exports(&self) -> Vec<&str> {
        self.exports
            .iter()
            .filter(|export| matches!(export.kind, ExportKind::Func(_)))
            .map(|export| export.name.as_str())
            .collect()
    }
}

/// Everything that can go wrong reading or running a module.
///
/// One enum for both, because the caller does the same thing with either: it
/// shows the message and refuses to run the plugin. Splitting load errors from
/// traps would only make the app match twice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The file does not start with `\0asm`.
    BadMagic,
    /// The file is a WebAssembly version this reader does not know.
    BadVersion,
    /// A read ran past the end of the bytes it was given.
    Truncated,
    /// A LEB128 integer was longer than its type allows.
    BadLeb,
    /// A name was not UTF-8.
    BadUtf8,
    /// A value type byte that is not one of the four.
    BadType(u8),
    /// The bytes are well formed but describe something impossible.
    BadModule(&'static str),
    /// The module uses a feature this interpreter does not implement.
    Unsupported(&'static str),
    /// `unreachable` was executed.
    Unreachable,
    /// A load or store was outside the memory.
    OutOfBounds,
    /// An integer division or remainder by zero.
    DivideByZero,
    /// `i32.div_s` of `i32::MIN` by `-1`, which has no representable result.
    IntegerOverflow,
    /// The call stack or the value stack hit its limit.
    StackExhausted,
    /// The step budget ran out. A plugin that loops forever is stopped here
    /// rather than freezing the editor.
    OutOfFuel,
    /// A call to a function index that does not exist.
    NoSuchFunction(u32),
    /// An export the host asked for is not there.
    NoSuchExport(String),
    /// A host function returned an error, or was called with the wrong
    /// arguments.
    Host(String),
    /// An opcode the interpreter does not implement.
    UnsupportedOpcode(u8),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::BadMagic => f.write_str("not a WebAssembly module"),
            Error::BadVersion => f.write_str("unsupported WebAssembly version"),
            Error::Truncated => f.write_str("the module ends in the middle of something"),
            Error::BadLeb => f.write_str("a length or constant is malformed"),
            Error::BadUtf8 => f.write_str("a name is not valid UTF-8"),
            Error::BadType(byte) => write!(f, "unknown value type 0x{byte:02X}"),
            Error::BadModule(what) => write!(f, "malformed module: {what}"),
            Error::Unsupported(what) => write!(f, "unsupported: {what}"),
            Error::Unreachable => f.write_str("the plugin reached an unreachable instruction"),
            Error::OutOfBounds => f.write_str("the plugin read or wrote outside its memory"),
            Error::DivideByZero => f.write_str("the plugin divided by zero"),
            Error::IntegerOverflow => f.write_str("the plugin overflowed an integer division"),
            Error::StackExhausted => f.write_str("the plugin recursed too deeply"),
            Error::OutOfFuel => f.write_str("the plugin ran for too long and was stopped"),
            Error::NoSuchFunction(index) => write!(f, "no function at index {index}"),
            Error::NoSuchExport(name) => write!(f, "the plugin does not export `{name}`"),
            Error::Host(message) => write!(f, "{message}"),
            Error::UnsupportedOpcode(op) => write!(f, "unsupported opcode 0x{op:02X}"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    fn module() -> Module {
        Module {
            types: vec![FuncType { params: vec![ValType::I32], results: vec![] }],
            imports: vec![Import {
                module: "npp".into(),
                name: "log".into(),
                kind: ImportKind::Func(0),
            }],
            funcs: vec![Func {
                type_index: 0,
                locals: vec![],
                code: vec![0x0B],
                jumps: vec![],
            }],
            exports: vec![Export { name: "run".into(), kind: ExportKind::Func(1) }],
            memory: None,
            table: None,
            globals: vec![],
            elements: vec![],
            data: vec![],
            start: None,
        }
    }

    #[test]
    fn an_imported_function_comes_first_in_the_index_space() {
        let module = module();
        assert_eq!(module.imported_funcs(), vec![0]);
        assert_eq!(module.func_type(0), Some(&module.types[0]));
        assert_eq!(module.func_type(1), Some(&module.types[0]));
        assert_eq!(module.func_type(2), None);
    }

    #[test]
    fn an_imported_function_can_be_named() {
        let module = module();
        assert_eq!(module.imported_func(0).map(|i| i.name.as_str()), Some("log"));
        assert_eq!(module.imported_func(1), None);
    }

    #[test]
    fn an_export_is_found_by_name() {
        let module = module();
        assert_eq!(module.export_func("run"), Some(1));
        assert_eq!(module.export_func("stop"), None);
        assert_eq!(module.func_exports(), vec!["run"]);
    }

    #[test]
    fn a_value_reports_its_type() {
        assert_eq!(Value::I32(0).ty(), ValType::I32);
        assert_eq!(Value::F64(0.0).ty(), ValType::F64);
        assert_eq!(Value::I32(-1).as_u32(), Some(u32::MAX));
        assert_eq!(Value::I64(0).as_i32(), None);
    }

    #[test]
    fn every_error_has_a_message() {
        let errors = [
            Error::BadMagic,
            Error::BadVersion,
            Error::Truncated,
            Error::BadLeb,
            Error::BadUtf8,
            Error::BadType(0x7B),
            Error::BadModule("x"),
            Error::Unsupported("y"),
            Error::Unreachable,
            Error::OutOfBounds,
            Error::DivideByZero,
            Error::IntegerOverflow,
            Error::StackExhausted,
            Error::OutOfFuel,
            Error::NoSuchFunction(3),
            Error::NoSuchExport("run".into()),
            Error::Host("z".into()),
            Error::UnsupportedOpcode(0xFC),
        ];
        for error in errors {
            assert!(!error.to_string().is_empty(), "{error:?} has no message");
        }
    }
}
