//! Reading the WebAssembly binary format into a [`Module`].
//!
//! This is a reader, not a validator. It checks the things that make the bytes
//! unreadable -- a bad magic number, a truncated section, a length that runs
//! past the end -- and leaves the things that make a module *wrong* to the
//! interpreter, which is typed at run time. The split is deliberate: a full
//! validator is a second implementation of the type system, and the only
//! modules this editor loads are ones the user chose to install. A module that
//! lies about its types traps on the first bad instruction instead of being
//! rejected up front, which is a worse error message but the same outcome.
//!
//! What is read: types, imports, functions, memory, exports, start, code, and
//! data. What is skipped: tables, globals, elements, and custom sections. A
//! module that needs a table -- `call_indirect`, which is what a C++ compiler
//! emits for virtual calls -- is rejected at load with
//! [`Error::Unsupported`] rather than trapping halfway through a command.

use super::{Data, Element, Error, Export, ExportKind, Func, FuncType, Global, Import, ImportKind,
            Init, Memory, Module, Table, ValType, Value};

/// A cursor over a byte slice that knows how to read the format's primitives.
///
/// Every read is bounds checked and every failure is an [`Error`], so a
/// truncated file produces a message rather than a panic. The cursor is not
/// `Clone` on purpose: a section reader that could be rewound would make it
/// easy to read the same bytes twice by accident.
pub struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    /// How many bytes are left. Used by section readers to assert they
    /// consumed exactly the declared length.
    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    pub fn at_end(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    pub fn u8(&mut self) -> Result<u8, Error> {
        let byte = *self.bytes.get(self.pos).ok_or(Error::Truncated)?;
        self.pos += 1;
        Ok(byte)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.pos.checked_add(n).ok_or(Error::Truncated)?;
        let slice = self.bytes.get(self.pos..end).ok_or(Error::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    /// An unsigned LEB128 integer, at most five bytes.
    ///
    /// The fifth byte may only carry four significant bits; anything else is a
    /// value that does not fit in a `u32` and is rejected rather than silently
    /// truncated. Silently truncating a length is how a reader ends up
    /// allocating the wrong amount and reading the wrong bytes.
    pub fn u32(&mut self) -> Result<u32, Error> {
        let mut result: u32 = 0;
        let mut shift = 0;
        loop {
            let byte = self.u8()?;
            if shift == 28 && byte & 0xF0 != 0 {
                return Err(Error::BadLeb);
            }
            result |= u32::from(byte & 0x7F) << shift;
            if byte & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
            if shift > 28 {
                return Err(Error::BadLeb);
            }
        }
    }

    /// A signed LEB128 integer, at most five bytes.
    pub fn i32(&mut self) -> Result<i32, Error> {
        let value = self.sleb(35)?;
        if value < i128::from(i32::MIN) || value > i128::from(i32::MAX) {
            return Err(Error::BadLeb);
        }
        Ok(value as i32)
    }

    /// A signed LEB128 integer, at most ten bytes.
    pub fn i64(&mut self) -> Result<i64, Error> {
        let value = self.sleb(70)?;
        if value < i128::from(i64::MIN) || value > i128::from(i64::MAX) {
            return Err(Error::BadLeb);
        }
        Ok(value as i64)
    }

    /// The shared signed-LEB reader. `limit` is the number of bits the
    /// encoding may occupy, which is what distinguishes the 32-bit and 64-bit
    /// forms; the accumulator is `i128` so neither can overflow it.
    fn sleb(&mut self, limit: u32) -> Result<i128, Error> {
        let mut result: i128 = 0;
        let mut shift = 0;
        loop {
            let byte = self.u8()?;
            result |= i128::from(byte & 0x7F) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                if shift < 128 && byte & 0x40 != 0 {
                    result |= -1i128 << shift;
                }
                return Ok(result);
            }
            if shift >= limit {
                return Err(Error::BadLeb);
            }
        }
    }

    pub fn f32(&mut self) -> Result<f32, Error> {
        let bytes = self.take(4)?;
        Ok(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub fn f64(&mut self) -> Result<f64, Error> {
        let bytes = self.take(8)?;
        Ok(f64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    /// A length-prefixed UTF-8 string, which the format calls a `name`.
    pub fn name(&mut self) -> Result<String, Error> {
        let len = self.u32()? as usize;
        let bytes = self.take(len)?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| Error::BadUtf8)
    }

    /// A sub-reader over the next `len` bytes, with the parent advanced past
    /// them. Section bodies are read through this so a section that declares
    /// more than it contains fails at the boundary rather than at the end of
    /// the file.
    fn section(&mut self, len: usize) -> Result<Reader<'a>, Error> {
        let bytes = self.take(len)?;
        Ok(Reader::new(bytes))
    }

    fn val_type(&mut self) -> Result<ValType, Error> {
        match self.u8()? {
            0x7F => Ok(ValType::I32),
            0x7E => Ok(ValType::I64),
            0x7D => Ok(ValType::F32),
            0x7C => Ok(ValType::F64),
            other => Err(Error::BadType(other)),
        }
    }

    /// A table's element type and size. Only `funcref` is accepted: an
    /// `externref` table has no use in an editor plugin and no representation
    /// in this interpreter.
    fn table_type(&mut self) -> Result<Table, Error> {
        match self.u8()? {
            0x70 => {}
            _ => return Err(Error::Unsupported("a table of something other than funcref")),
        }
        let (min, max) = self.limits()?;
        Ok(Table { min, max })
    }

    /// A global's initialiser: a constant, or another global's value.
    fn init(&mut self) -> Result<Init, Error> {
        let init = match self.u8()? {
            0x41 => Init::Value(Value::I32(self.i32()?)),
            0x42 => Init::Value(Value::I64(self.i64()?)),
            0x43 => Init::Value(Value::F32(self.f32()?)),
            0x44 => Init::Value(Value::F64(self.f64()?)),
            0x23 => Init::Global(self.u32()?),
            0xD0 => {
                self.u8()?;
                Init::Value(Value::FuncRef(None))
            }
            0xD2 => Init::Value(Value::FuncRef(Some(self.u32()?))),
            _ => return Err(Error::BadModule("a global initialiser is not a constant")),
        };
        if self.u8()? != 0x0B {
            return Err(Error::BadModule("a global initialiser is not a constant"));
        }
        Ok(init)
    }

    /// One entry of an expression-form element segment: `ref.func N end` or
    /// `ref.null funcref end`.
    fn element_expr(&mut self) -> Result<Option<u32>, Error> {
        let value = match self.u8()? {
            0xD2 => Some(self.u32()?),
            0xD0 => {
                self.u8()?;
                None
            }
            _ => return Err(Error::BadModule("an element entry is not a reference")),
        };
        if self.u8()? != 0x0B {
            return Err(Error::BadModule("an element entry is not a reference"));
        }
        Ok(value)
    }

    fn limits(&mut self) -> Result<(u32, Option<u32>), Error> {
        match self.u8()? {
            0x00 => Ok((self.u32()?, None)),
            0x01 => Ok((self.u32()?, Some(self.u32()?))),
            other => Err(Error::BadModule(match other {
                0x02 | 0x03 => "shared memory is not supported",
                _ => "unknown limits flag",
            })),
        }
    }
}

impl Module {
    /// Reads a module. The only entry point: there is no streaming form,
    /// because a plugin is a file the user installed and it is already in
    /// memory by the time this is called.
    pub fn parse(bytes: &[u8]) -> Result<Module, Error> {
        let mut reader = Reader::new(bytes);
        let magic = reader.take(4)?;
        if magic != b"\0asm" {
            return Err(Error::BadMagic);
        }
        let version = reader.take(4)?;
        if version != [1, 0, 0, 0] {
            return Err(Error::BadVersion);
        }

        let mut module = Module::default();
        let mut bodies: Vec<(Vec<ValType>, Vec<u8>)> = Vec::new();
        let mut declared: Vec<u32> = Vec::new();

        while !reader.at_end() {
            let id = reader.u8()?;
            let len = reader.u32()? as usize;
            let mut section = reader.section(len)?;
            match id {
                0 => {
                    // Custom sections carry names and debug info. Nothing here
                    // reads them, but they must be skipped rather than
                    // rejected: every Rust-compiled module has several.
                }
                1 => {
                    let count = section.u32()?;
                    for _ in 0..count {
                        if section.u8()? != 0x60 {
                            return Err(Error::BadModule("type is not a function type"));
                        }
                        let params = read_types(&mut section)?;
                        let results = read_types(&mut section)?;
                        module.types.push(FuncType { params, results });
                    }
                }
                2 => {
                    let count = section.u32()?;
                    for _ in 0..count {
                        let name = section.name()?;
                        let field = section.name()?;
                        let kind = match section.u8()? {
                            0x00 => ImportKind::Func(section.u32()?),
                            0x01 => ImportKind::Table(section.table_type()?),
                            0x02 => {
                                let (min, max) = section.limits()?;
                                ImportKind::Memory(Memory { min, max })
                            }
                            0x03 => return Err(Error::Unsupported("a global import")),
                            _ => return Err(Error::BadModule("unknown import kind")),
                        };
                        module.imports.push(Import { module: name, name: field, kind });
                    }
                }
                3 => {
                    let count = section.u32()?;
                    for _ in 0..count {
                        declared.push(section.u32()?);
                    }
                }
                4 => {
                    let count = section.u32()?;
                    for _ in 0..count {
                        let table = section.table_type()?;
                        if module.table.is_some() {
                            return Err(Error::Unsupported("more than one table"));
                        }
                        module.table = Some(table);
                    }
                }
                5 => {
                    let count = section.u32()?;
                    for _ in 0..count {
                        let (min, max) = section.limits()?;
                        if module.memory.is_some() {
                            return Err(Error::Unsupported("more than one memory"));
                        }
                        module.memory = Some(Memory { min, max });
                    }
                }
                6 => {
                    let count = section.u32()?;
                    for _ in 0..count {
                        let ty = section.val_type()?;
                        let mutable = match section.u8()? {
                            0x00 => false,
                            0x01 => true,
                            _ => return Err(Error::BadModule("unknown global mutability")),
                        };
                        let init = section.init()?;
                        module.globals.push(Global { ty, mutable, init });
                    }
                }
                7 => {
                    let count = section.u32()?;
                    for _ in 0..count {
                        let name = section.name()?;
                        let kind = match section.u8()? {
                            0x00 => ExportKind::Func(section.u32()?),
                            0x02 => ExportKind::Memory(section.u32()?),
                            0x01 => ExportKind::Table(section.u32()?),
                            0x03 => return Err(Error::Unsupported("a global export")),
                            _ => return Err(Error::BadModule("unknown export kind")),
                        };
                        module.exports.push(Export { name, kind });
                    }
                }
                8 => module.start = Some(section.u32()?),
                9 => {
                    let count = section.u32()?;
                    for _ in 0..count {
                        let flag = section.u32()?;
                        // The eight forms differ in whether the segment is
                        // active, which table it targets, and whether the
                        // entries are indices or expressions. Only the active
                        // forms put anything in the table, so the passive and
                        // declarative ones are read and dropped.
                        let (active, table_index) = match flag {
                            0 | 4 => (true, 0),
                            2 | 6 => (true, section.u32()?),
                            1 | 3 | 5 | 7 => (false, 0),
                            _ => return Err(Error::BadModule("unknown element segment")),
                        };
                        if (1..=3).contains(&flag) {
                            // A passive or declarative segment still carries an
                            // element kind byte.
                            section.u8()?;
                        }
                        let offset = if active { read_offset(&mut section)? } else { 0 };
                        if flag == 2 || flag == 6 {
                            section.u8()?;
                        }
                        let n = section.u32()?;
                        let mut funcs = Vec::new();
                        for _ in 0..n {
                            if flag >= 4 {
                                funcs.push(section.element_expr()?);
                            } else {
                                funcs.push(Some(section.u32()?));
                            }
                        }
                        if active && table_index == 0 {
                            module.elements.push(Element { offset, funcs });
                        }
                    }
                }
                10 => {
                    let count = section.u32()?;
                    for _ in 0..count {
                        let size = section.u32()? as usize;
                        let mut body = section.section(size)?;
                        let groups = body.u32()?;
                        let mut locals = Vec::new();
                        for _ in 0..groups {
                            let n = body.u32()?;
                            let ty = body.val_type()?;
                            // A group is a run-length encoding of repeated
                            // locals. Expanding it here means the interpreter
                            // never has to think about groups.
                            if n > 50_000 {
                                return Err(Error::BadModule("too many locals"));
                            }
                            for _ in 0..n {
                                locals.push(ty);
                            }
                        }
                        let code = body.take(body.remaining())?.to_vec();
                        bodies.push((locals, code));
                    }
                }
                11 => {
                    let count = section.u32()?;
                    for _ in 0..count {
                        let flag = section.u32()?;
                        match flag {
                            0 => {
                                let offset = read_offset(&mut section)?;
                                let len = section.u32()? as usize;
                                let bytes = section.take(len)?.to_vec();
                                module.data.push(Data { offset, bytes });
                            }
                            1 => {
                                // Passive: only reachable through
                                // `memory.init`, which is not implemented.
                                let len = section.u32()? as usize;
                                section.take(len)?;
                            }
                            2 => {
                                section.u32()?;
                                let offset = read_offset(&mut section)?;
                                let len = section.u32()? as usize;
                                let bytes = section.take(len)?.to_vec();
                                module.data.push(Data { offset, bytes });
                            }
                            _ => return Err(Error::BadModule("unknown data segment")),
                        }
                    }
                }
                12 => {
                    // The data count section exists for `memory.init`; the
                    // number itself is not needed.
                    section.u32()?;
                }
                _ => return Err(Error::BadModule("unknown section id")),
            }
        }

        if declared.len() != bodies.len() {
            return Err(Error::BadModule(
                "the function and code sections disagree on how many functions there are",
            ));
        }
        for (type_index, (locals, code)) in declared.into_iter().zip(bodies) {
            if type_index as usize >= module.types.len() {
                return Err(Error::BadModule("a function names a type that does not exist"));
            }
            let jumps = super::exec::jumps(&code)?;
            module.funcs.push(Func { type_index, locals, code, jumps });
        }
        Ok(module)
    }
}

fn read_types(reader: &mut Reader<'_>) -> Result<Vec<ValType>, Error> {
    let count = reader.u32()?;
    let mut types = Vec::new();
    for _ in 0..count {
        types.push(reader.val_type()?);
    }
    Ok(types)
}

/// Reads an `i32.const N end` initialiser expression.
///
/// Only that one form is accepted. A data offset computed at run time would
/// need the interpreter to run before the memory exists, which is a
/// bootstrapping problem no plugin needs to have.
fn read_offset(reader: &mut Reader<'_>) -> Result<u32, Error> {
    if reader.u8()? != 0x41 {
        return Err(Error::BadModule("a data offset is not a constant"));
    }
    let value = reader.i32()?;
    if reader.u8()? != 0x0B {
        return Err(Error::BadModule("a data offset is not a constant"));
    }
    if value < 0 {
        return Err(Error::BadModule("a data offset is negative"));
    }
    Ok(value as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal module: no types, no functions, nothing. Every other test
    /// builds on this by appending sections.
    fn empty() -> Vec<u8> {
        let mut bytes = b"\0asm".to_vec();
        bytes.extend_from_slice(&[1, 0, 0, 0]);
        bytes
    }

    fn section(id: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![id];
        out.push(body.len() as u8);
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn an_empty_module_parses() {
        let module = Module::parse(&empty()).unwrap();
        assert!(module.types.is_empty());
        assert!(module.funcs.is_empty());
        assert!(module.memory.is_none());
    }

    #[test]
    fn a_bad_magic_number_is_rejected() {
        let mut bytes = empty();
        bytes[0] = b'x';
        assert_eq!(Module::parse(&bytes), Err(Error::BadMagic));
    }

    #[test]
    fn a_future_version_is_rejected() {
        let mut bytes = empty();
        bytes[4] = 2;
        assert_eq!(Module::parse(&bytes), Err(Error::BadVersion));
    }

    #[test]
    fn a_truncated_section_is_rejected() {
        let mut bytes = empty();
        bytes.extend_from_slice(&[1, 4, 1, 0x60, 0, 0]);
        // The section claims four bytes but only three follow.
        bytes.pop();
        assert_eq!(Module::parse(&bytes), Err(Error::Truncated));
    }

    #[test]
    fn a_function_type_is_read() {
        let mut bytes = empty();
        bytes.extend_from_slice(&section(1, &[1, 0x60, 2, 0x7F, 0x7F, 1, 0x7F]));
        let module = Module::parse(&bytes).unwrap();
        assert_eq!(module.types.len(), 1);
        assert_eq!(module.types[0].params, vec![ValType::I32, ValType::I32]);
        assert_eq!(module.types[0].results, vec![ValType::I32]);
    }

    #[test]
    fn an_unknown_value_type_is_rejected() {
        let mut bytes = empty();
        bytes.extend_from_slice(&section(1, &[1, 0x60, 1, 0x7B, 0]));
        assert_eq!(Module::parse(&bytes), Err(Error::BadType(0x7B)));
    }

    #[test]
    fn a_memory_is_read_with_its_maximum() {
        let mut bytes = empty();
        bytes.extend_from_slice(&section(5, &[1, 1, 2, 8]));
        let module = Module::parse(&bytes).unwrap();
        assert_eq!(module.memory, Some(Memory { min: 2, max: Some(8) }));
    }

    #[test]
    fn a_shared_memory_is_rejected() {
        let mut bytes = empty();
        bytes.extend_from_slice(&section(5, &[1, 3, 1, 1]));
        assert!(matches!(Module::parse(&bytes), Err(Error::BadModule(_))));
    }

    #[test]
    fn an_export_is_read() {
        let mut bytes = empty();
        bytes.extend_from_slice(&section(1, &[1, 0x60, 0, 0]));
        bytes.extend_from_slice(&section(3, &[1, 0]));
        bytes.extend_from_slice(&section(7, &[1, 3, b'r', b'u', b'n', 0, 0]));
        bytes.extend_from_slice(&section(10, &[1, 2, 0, 0x0B]));
        let module = Module::parse(&bytes).unwrap();
        assert_eq!(module.exports, vec![Export {
            name: "run".into(),
            kind: ExportKind::Func(0),
        }]);
    }

    #[test]
    fn a_table_section_is_read() {
        let mut bytes = empty();
        bytes.extend_from_slice(&section(4, &[1, 0x70, 0, 1]));
        let module = Module::parse(&bytes).unwrap();
        assert_eq!(module.table, Some(Table { min: 1, max: None }));
    }

    #[test]
    fn a_table_of_something_other_than_funcref_is_rejected() {
        let mut bytes = empty();
        bytes.extend_from_slice(&section(4, &[1, 0x6F, 0, 1]));
        assert!(matches!(Module::parse(&bytes), Err(Error::Unsupported(_))));
    }

    #[test]
    fn a_global_is_read_with_its_initialiser() {
        let mut bytes = empty();
        // One mutable i32 global starting at 1024.
        bytes.extend_from_slice(&section(6, &[1, 0x7F, 1, 0x41, 0x80, 0x08, 0x0B]));
        let module = Module::parse(&bytes).unwrap();
        assert_eq!(module.globals.len(), 1);
        assert_eq!(module.globals[0].ty, ValType::I32);
        assert!(module.globals[0].mutable);
        assert_eq!(module.globals[0].init, Init::Value(Value::I32(1024)));
    }

    #[test]
    fn a_global_initialised_from_another_global_is_read() {
        let mut bytes = empty();
        bytes.extend_from_slice(&section(6, &[1, 0x7F, 0, 0x23, 0, 0x0B]));
        let module = Module::parse(&bytes).unwrap();
        assert_eq!(module.globals[0].init, Init::Global(0));
    }

    #[test]
    fn an_element_segment_is_read() {
        let mut bytes = empty();
        // Active, table 0, offset 0, two function indices.
        bytes.extend_from_slice(&section(9, &[1, 0, 0x41, 0, 0x0B, 2, 0, 1]));
        let module = Module::parse(&bytes).unwrap();
        assert_eq!(module.elements, vec![Element {
            offset: 0,
            funcs: vec![Some(0), Some(1)],
        }]);
    }

    #[test]
    fn an_expression_element_segment_is_read() {
        let mut bytes = empty();
        // Flag 4: active, offset 0, entries are `ref.func N end`.
        bytes.extend_from_slice(&section(9, &[1, 4, 0x41, 0, 0x0B, 1, 0xD2, 3, 0x0B]));
        let module = Module::parse(&bytes).unwrap();
        assert_eq!(module.elements[0].funcs, vec![Some(3)]);
    }

    #[test]
    fn a_function_without_a_body_is_rejected() {
        let mut bytes = empty();
        bytes.extend_from_slice(&section(1, &[1, 0x60, 0, 0]));
        bytes.extend_from_slice(&section(3, &[1, 0]));
        assert!(matches!(Module::parse(&bytes), Err(Error::BadModule(_))));
    }

    #[test]
    fn a_function_naming_a_missing_type_is_rejected() {
        let mut bytes = empty();
        bytes.extend_from_slice(&section(3, &[1, 0]));
        bytes.extend_from_slice(&section(10, &[1, 2, 0, 0x0B]));
        assert!(matches!(Module::parse(&bytes), Err(Error::BadModule(_))));
    }

    #[test]
    fn locals_are_expanded_from_their_groups() {
        let mut bytes = empty();
        bytes.extend_from_slice(&section(1, &[1, 0x60, 0, 0]));
        bytes.extend_from_slice(&section(3, &[1, 0]));
        // Two groups: three i32s, then one i64.
        bytes.extend_from_slice(&section(10, &[1, 6, 2, 3, 0x7F, 1, 0x7E, 0x0B]));
        let module = Module::parse(&bytes).unwrap();
        assert_eq!(module.funcs[0].locals, vec![
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I64,
        ]);
    }

    #[test]
    fn a_data_segment_is_read_at_its_offset() {
        let mut bytes = empty();
        bytes.extend_from_slice(&section(5, &[1, 0, 1]));
        bytes.extend_from_slice(&section(11, &[1, 0, 0x41, 4, 0x0B, 2, b'h', b'i']));
        let module = Module::parse(&bytes).unwrap();
        assert_eq!(module.data, vec![Data { offset: 4, bytes: b"hi".to_vec() }]);
    }

    #[test]
    fn a_data_offset_that_is_not_a_constant_is_rejected() {
        let mut bytes = empty();
        bytes.extend_from_slice(&section(5, &[1, 0, 1]));
        bytes.extend_from_slice(&section(11, &[1, 0, 0x20, 0, 0x0B, 0]));
        assert!(matches!(Module::parse(&bytes), Err(Error::BadModule(_))));
    }

    #[test]
    fn a_custom_section_is_skipped() {
        let mut bytes = empty();
        bytes.extend_from_slice(&section(0, &[4, b'n', b'a', b'm', b'e', 1, 2, 3]));
        bytes.extend_from_slice(&section(1, &[1, 0x60, 0, 0]));
        let module = Module::parse(&bytes).unwrap();
        assert_eq!(module.types.len(), 1);
    }

    #[test]
    fn an_overlong_leb_is_rejected() {
        let mut reader = Reader::new(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01]);
        assert_eq!(reader.u32(), Err(Error::BadLeb));
    }

    #[test]
    fn a_negative_i32_round_trips() {
        // -1 encodes as a single 0x7F byte.
        let mut reader = Reader::new(&[0x7F]);
        assert_eq!(reader.i32(), Ok(-1));
    }

    #[test]
    fn a_large_i64_round_trips() {
        let mut reader = Reader::new(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x7F]);
        assert_eq!(reader.i64(), Ok(i64::MIN));
    }

    #[test]
    fn a_name_that_is_not_utf8_is_rejected() {
        let mut reader = Reader::new(&[2, 0xFF, 0xFE]);
        assert_eq!(reader.name(), Err(Error::BadUtf8));
    }
}
