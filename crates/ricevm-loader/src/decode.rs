use ricevm_core::{
    AddressMode, DataItem, ExceptionCase, ExportEntry, Handler, Header, ImportEntry, ImportModule,
    Instruction, LoadError, MiddleMode, MiddleOperand, Module, Opcode, Operand, PointerMap,
    RuntimeFlags, SMAGIC, TypeDescriptor, XMAGIC,
};

use crate::reader::Reader;

pub(crate) fn parse_header(r: &mut Reader<'_>) -> Result<Header, LoadError> {
    let magic = r.read_operand("header")?;
    let signature = if magic == SMAGIC {
        let sig_len = r.read_operand("header")? as usize;
        r.read_bytes(sig_len, "header")?.to_vec()
    } else if magic == XMAGIC {
        Vec::new()
    } else {
        return Err(LoadError::InvalidMagic(magic));
    };

    let runtime_flags = RuntimeFlags(r.read_operand("header")? as u32);
    // Accept modules with the deprecated import flag; they use an older
    // format but can still be executed.

    let stack_extent = r.read_operand("header")?;
    let code_size = r.read_operand("header")?;
    let data_size = r.read_operand("header")?;
    let type_size = r.read_operand("header")?;
    let export_size = r.read_operand("header")?;
    let entry_pc = r.read_operand("header")?;
    let entry_type = r.read_operand("header")?;

    Ok(Header {
        magic,
        signature,
        runtime_flags,
        stack_extent,
        code_size,
        data_size,
        type_size,
        export_size,
        entry_pc,
        entry_type,
    })
}

fn parse_src_dst_operand(r: &mut Reader<'_>, mode_bits: u8) -> Result<Operand, LoadError> {
    let mode = match mode_bits {
        0 => AddressMode::OffsetIndirectMp,
        1 => AddressMode::OffsetIndirectFp,
        2 => AddressMode::Immediate,
        3 => return Ok(Operand::UNUSED),
        4 => AddressMode::OffsetDoubleIndirectMp,
        5 => AddressMode::OffsetDoubleIndirectFp,
        _ => return Err(LoadError::InvalidAddressMode(mode_bits)),
    };

    let register1 = r.read_operand("code")?;
    let register2 = if mode_bits == 4 || mode_bits == 5 {
        r.read_operand("code")?
    } else {
        0
    };

    Ok(Operand {
        mode,
        register1,
        register2,
    })
}

pub(crate) fn parse_code(r: &mut Reader<'_>, count: i32) -> Result<Vec<Instruction>, LoadError> {
    let mut instructions = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let op_byte = r.read_byte("code")?;
        let opcode = Opcode::try_from(op_byte).map_err(LoadError::InvalidOpcode)?;
        let addr_code = r.read_byte("code")?;

        let mid_bits = (addr_code >> 6) & 0x03;
        let src_bits = (addr_code >> 3) & 0x07;
        let dst_bits = addr_code & 0x07;

        let middle = if mid_bits == 0 {
            MiddleOperand::UNUSED
        } else {
            let mode = match mid_bits {
                1 => MiddleMode::SmallImmediate,
                2 => MiddleMode::SmallOffsetFp,
                3 => MiddleMode::SmallOffsetMp,
                _ => unreachable!(),
            };
            let register1 = r.read_operand("code")?;
            MiddleOperand { mode, register1 }
        };

        let source = parse_src_dst_operand(r, src_bits)?;
        let destination = parse_src_dst_operand(r, dst_bits)?;

        instructions.push(Instruction {
            opcode,
            source,
            middle,
            destination,
        });
    }

    Ok(instructions)
}

pub(crate) fn parse_types(
    r: &mut Reader<'_>,
    count: i32,
) -> Result<Vec<TypeDescriptor>, LoadError> {
    let mut descriptors = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let id = r.read_operand("type")? as u32;
        let size = r.read_operand("type")?;
        let map_in_bytes = r.read_operand("type")? as usize;
        let bytes = if map_in_bytes > 0 {
            r.read_bytes(map_in_bytes, "type")?.to_vec()
        } else {
            Vec::new()
        };

        let pointer_map = PointerMap { bytes };
        let pointer_count = pointer_map.count_pointers();
        descriptors.push(TypeDescriptor {
            id,
            size,
            pointer_map,
            pointer_count,
        });
    }

    Ok(descriptors)
}

fn read_real(r: &mut Reader<'_>) -> Result<f64, LoadError> {
    let hi = r.read_word_be("data")? as u32;
    let lo = r.read_word_be("data")? as u32;
    let bits = ((hi as u64) << 32) | (lo as u64);
    Ok(f64::from_bits(bits))
}

fn read_big(r: &mut Reader<'_>) -> Result<i64, LoadError> {
    let hi = r.read_word_be("data")? as u32;
    let lo = r.read_word_be("data")? as u32;
    let bits = ((hi as u64) << 32) | (lo as u64);
    Ok(bits as i64)
}

pub(crate) fn parse_data(r: &mut Reader<'_>) -> Result<Vec<DataItem>, LoadError> {
    let mut items = Vec::new();

    loop {
        let code = r.read_byte("data")?;
        if code == 0 {
            break;
        }

        let item_type = (code >> 4) & 0x0F;
        let mut count = (code & 0x0F) as i32;
        if count == 0 {
            count = r.read_operand("data")?;
        }

        let offset = r.read_operand("data")?;

        let item = match item_type {
            1 => {
                // value_bit8
                let values = r.read_bytes(count as usize, "data")?.to_vec();
                DataItem::Bytes { offset, values }
            }
            2 => {
                // value_bit32
                let mut values = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    values.push(r.read_word_be("data")?);
                }
                DataItem::Words { offset, values }
            }
            3 => {
                // utf_string
                let bytes = r.read_bytes(count as usize, "data")?;
                let value = String::from_utf8_lossy(bytes).into_owned();
                DataItem::String { offset, value }
            }
            4 => {
                // value_real64
                let mut values = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    values.push(read_real(r)?);
                }
                DataItem::Reals { offset, values }
            }
            5 => {
                // array
                let element_type = r.read_word_be("data")?;
                let length = r.read_word_be("data")?;
                DataItem::Array {
                    offset,
                    element_type,
                    length,
                }
            }
            6 => {
                // set_array
                let index = r.read_word_be("data")?;
                DataItem::SetArray { offset, index }
            }
            7 => {
                // restore_load_address
                DataItem::RestoreBase
            }
            8 => {
                // value_bit64
                let mut values = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    values.push(read_big(r)?);
                }
                DataItem::Bigs { offset, values }
            }
            _ => return Err(LoadError::InvalidDataType(item_type)),
        };

        items.push(item);
    }

    Ok(items)
}

pub(crate) fn parse_name(r: &mut Reader<'_>) -> Result<String, LoadError> {
    r.read_cstring("module name")
}

pub(crate) fn parse_exports(r: &mut Reader<'_>, count: i32) -> Result<Vec<ExportEntry>, LoadError> {
    let mut entries = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let pc = r.read_operand("export")?;
        let frame_type = r.read_operand("export")?;
        let signature = r.read_word_be("export")?;
        let name = r.read_cstring("export")?;

        entries.push(ExportEntry {
            pc,
            frame_type,
            signature,
            name,
        });
    }

    Ok(entries)
}

pub(crate) fn parse_imports(r: &mut Reader<'_>) -> Result<Vec<ImportModule>, LoadError> {
    let module_count = r.read_operand("import")?;
    let mut modules = Vec::with_capacity(module_count as usize);

    for _ in 0..module_count {
        let func_count = r.read_operand("import")?;
        let mut functions = Vec::with_capacity(func_count as usize);

        for _ in 0..func_count {
            let signature = r.read_word_be("import")?;
            let name = r.read_cstring("import")?;
            functions.push(ImportEntry { signature, name });
        }

        modules.push(ImportModule { functions });
    }

    // Trailing null byte
    let end = r.read_byte("import")?;
    if end != 0 {
        return Err(LoadError::Other(
            "expected null byte at end of import section".to_string(),
        ));
    }

    Ok(modules)
}

pub(crate) fn parse_handlers(r: &mut Reader<'_>) -> Result<Vec<Handler>, LoadError> {
    let handler_count = r.read_operand("handler")?;
    let mut handlers = Vec::with_capacity(handler_count as usize);

    for _ in 0..handler_count {
        let exception_offset = r.read_operand("handler")?;
        let begin_pc = r.read_operand("handler")?;
        let end_pc = r.read_operand("handler")?;
        let type_desc_number = r.read_operand("handler")?;
        let type_descriptor = if type_desc_number == -1 {
            None
        } else {
            Some(type_desc_number as u32)
        };

        let packed_cases = r.read_operand("handler")?;
        let _exception_type_count = packed_cases >> 16;
        let total_count = packed_cases & 0xFFFF;

        let mut cases = Vec::with_capacity((total_count + 1) as usize);

        for _ in 0..total_count {
            let name_str = r.read_cstring("handler")?;
            let name = if name_str.is_empty() {
                None
            } else {
                Some(name_str)
            };
            let pc = r.read_operand("handler")?;
            cases.push(ExceptionCase { name, pc });
        }

        // Wildcard exception (always present)
        let wildcard_pc = r.read_operand("handler")?;
        cases.push(ExceptionCase {
            name: None,
            pc: wildcard_pc,
        });

        handlers.push(Handler {
            exception_offset,
            begin_pc,
            end_pc,
            type_descriptor,
            cases,
        });
    }

    // Trailing null byte
    let end = r.read_byte("handler")?;
    if end != 0 {
        return Err(LoadError::Other(
            "expected null byte at end of handler section".to_string(),
        ));
    }

    Ok(handlers)
}

/// Parse a complete `.dis` module from a reader.
pub(crate) fn parse_module(r: &mut Reader<'_>) -> Result<Module, LoadError> {
    let header = parse_header(r)?;
    let code = parse_code(r, header.code_size)?;
    let types = parse_types(r, header.type_size)?;
    let data = parse_data(r)?;
    let name = parse_name(r)?;
    let exports = parse_exports(r, header.export_size)?;

    let imports = if header.runtime_flags.contains(RuntimeFlags::HAS_IMPORT) {
        parse_imports(r)?
    } else {
        Vec::new()
    };

    let handlers = if header.runtime_flags.contains(RuntimeFlags::HAS_HANDLER) {
        parse_handlers(r)?
    } else {
        Vec::new()
    };

    let module = Module {
        header,
        code,
        types,
        data,
        name,
        exports,
        imports,
        handlers,
    };

    validate_module(&module)?;
    Ok(module)
}

/// Post-parse validation of a loaded module.
fn validate_module(module: &Module) -> Result<(), LoadError> {
    let code_len = module.code.len() as i32;
    let type_len = module.types.len() as i32;

    // Entry PC must be within code bounds.
    // entry_pc == -1 is valid for library modules with no entry point.
    if module.header.entry_pc != -1
        && (module.header.entry_pc < 0 || module.header.entry_pc >= code_len)
    {
        return Err(LoadError::ValidationError(format!(
            "entry_pc {} is out of bounds (code size {})",
            module.header.entry_pc, code_len
        )));
    }

    // Entry type must reference a valid type descriptor (or -1 for none).
    // When type_len is 0, entry_type 0 is allowed (uses default frame size).
    if module.header.entry_type != -1
        && type_len > 0
        && (module.header.entry_type < 0 || module.header.entry_type >= type_len)
    {
        return Err(LoadError::ValidationError(format!(
            "entry_type {} is out of bounds (type count {})",
            module.header.entry_type, type_len
        )));
    }

    // MUST_COMPILE and DONT_COMPILE must not both be set.
    if module
        .header
        .runtime_flags
        .contains(RuntimeFlags::MUST_COMPILE)
        && module
            .header
            .runtime_flags
            .contains(RuntimeFlags::DONT_COMPILE)
    {
        return Err(LoadError::ValidationError(
            "MUST_COMPILE and DONT_COMPILE flags are both set".to_string(),
        ));
    }

    // Validate exports: each PC must be within code bounds.
    // Special exports like ".mp" (module data pointer) have pc == -1, which is valid.
    for (i, exp) in module.exports.iter().enumerate() {
        if exp.pc >= 0 && exp.pc >= code_len {
            return Err(LoadError::ValidationError(format!(
                "export[{i}] '{}' has pc {} out of bounds (code size {code_len})",
                exp.name, exp.pc
            )));
        }
        if exp.frame_type >= 0 && type_len > 0 && exp.frame_type >= type_len {
            return Err(LoadError::ValidationError(format!(
                "export[{i}] '{}' has frame_type {} out of bounds (type count {type_len})",
                exp.name, exp.frame_type
            )));
        }
    }

    // Validate handler ranges.
    for (i, h) in module.handlers.iter().enumerate() {
        if h.begin_pc < 0 || h.begin_pc >= code_len {
            return Err(LoadError::ValidationError(format!(
                "handler[{i}] begin_pc {} out of bounds",
                h.begin_pc
            )));
        }
        if h.end_pc < 0 || h.end_pc > code_len {
            return Err(LoadError::ValidationError(format!(
                "handler[{i}] end_pc {} out of bounds",
                h.end_pc
            )));
        }
        if h.begin_pc >= h.end_pc {
            return Err(LoadError::ValidationError(format!(
                "handler[{i}] begin_pc {} >= end_pc {}",
                h.begin_pc, h.end_pc
            )));
        }
    }

    // Note: HAS_IMPORT flag may be set even when module_count is 0 in the import
    // section (the section still exists with a zero count followed by a null byte).
    // This is valid and not an error.

    // Log signature info for signed modules (verification not implemented).
    if module.header.magic == SMAGIC && !module.header.signature.is_empty() {
        tracing::debug!(
            sig_len = module.header.signature.len(),
            "signed module loaded (signature verification not implemented)"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: encode an i32 as a Dis operand (variable-length).
    fn encode_operand(value: i32) -> Vec<u8> {
        if (0..=63).contains(&value) {
            vec![value as u8]
        } else if (-64..=-1).contains(&value) {
            // 1-byte signed: top 2 bits = 01, low 6 bits carry the sign-extended value.
            vec![0x40 | ((value & 0x3F) as u8)]
        } else {
            // Use 4-byte encoding for simplicity
            let mut buf = [0u8; 4];
            buf[0] = 0xC0 | (((value >> 24) as u8) & 0x3F);
            buf[1] = (value >> 16) as u8;
            buf[2] = (value >> 8) as u8;
            buf[3] = value as u8;
            buf.to_vec()
        }
    }

    /// Build a minimal valid module byte sequence.
    fn build_minimal_module() -> Vec<u8> {
        let mut bytes = Vec::new();

        // Header
        bytes.extend(encode_operand(XMAGIC)); // magic
        bytes.extend(encode_operand(0)); // runtime_flags
        bytes.extend(encode_operand(0)); // stack_extent
        bytes.extend(encode_operand(1)); // code_size (1 instruction)
        bytes.extend(encode_operand(0)); // data_size
        bytes.extend(encode_operand(0)); // type_size
        bytes.extend(encode_operand(0)); // export_size
        bytes.extend(encode_operand(0)); // entry_pc
        bytes.extend(encode_operand(0)); // entry_type

        // Code section: 1 Exit instruction with all-none addressing
        bytes.push(Opcode::Exit as u8); // opcode
        bytes.push(0x1B); // addr_code: mid=0, src=3(none), dst=3(none)

        // Data section: empty (just terminator)
        bytes.push(0x00);

        // Module name
        bytes.extend(b"test\0");

        // No exports (count is 0 from header)
        // No imports (flag not set)
        // No handlers (flag not set)

        bytes
    }

    #[test]
    fn parse_minimal_module() {
        let bytes = build_minimal_module();
        let mut r = Reader::new(&bytes);
        let module = parse_module(&mut r).unwrap();

        assert_eq!(module.header.magic, XMAGIC);
        assert_eq!(module.header.code_size, 1);
        assert_eq!(module.code.len(), 1);
        assert_eq!(module.code[0].opcode, Opcode::Exit);
        assert_eq!(module.name, "test");
        assert!(module.types.is_empty());
        assert!(module.exports.is_empty());
        assert!(module.imports.is_empty());
        assert!(module.handlers.is_empty());
    }

    #[test]
    fn parse_header_invalid_magic() {
        let bytes = encode_operand(0xDEAD);
        let mut r = Reader::new(&bytes);
        let result = parse_header(&mut r);
        assert!(matches!(result, Err(LoadError::InvalidMagic(0xDEAD))));
    }

    #[test]
    fn parse_header_truncated() {
        // Just the magic, then EOF
        let bytes = encode_operand(XMAGIC);
        let mut r = Reader::new(&bytes);
        let result = parse_header(&mut r);
        assert!(matches!(
            result,
            Err(LoadError::UnexpectedEof { section: "header" })
        ));
    }

    #[test]
    fn parse_one_instruction_with_operands() {
        let mut bytes = Vec::new();
        // addw src=fp(4), dst=fp(8) (no middle)
        bytes.push(Opcode::Addw as u8);
        // addr_code: mid=0(00), src=1(001=fp), dst=1(001=fp) -> 0b00_001_001 = 0x09
        bytes.push(0x09);
        bytes.extend(encode_operand(4)); // src register1
        bytes.extend(encode_operand(8)); // dst register1

        let mut r = Reader::new(&bytes);
        let code = parse_code(&mut r, 1).unwrap();

        assert_eq!(code.len(), 1);
        assert_eq!(code[0].opcode, Opcode::Addw);
        assert_eq!(code[0].source.mode, AddressMode::OffsetIndirectFp);
        assert_eq!(code[0].source.register1, 4);
        assert_eq!(code[0].destination.mode, AddressMode::OffsetIndirectFp);
        assert_eq!(code[0].destination.register1, 8);
        assert_eq!(code[0].middle.mode, MiddleMode::None);
    }

    #[test]
    fn parse_type_descriptor() {
        let mut bytes = Vec::new();
        bytes.extend(encode_operand(0)); // desc_number
        bytes.extend(encode_operand(16)); // size
        bytes.extend(encode_operand(2)); // map_in_bytes
        bytes.extend(&[0x03, 0x00]); // pointer map

        let mut r = Reader::new(&bytes);
        let types = parse_types(&mut r, 1).unwrap();

        assert_eq!(types.len(), 1);
        assert_eq!(types[0].id, 0);
        assert_eq!(types[0].size, 16);
        assert_eq!(types[0].pointer_map.bytes, vec![0x03, 0x00]);
    }

    #[test]
    fn parse_data_words() {
        let mut bytes = Vec::new();
        // datum code: type=2 (words), count=1
        bytes.push(0x21);
        // offset = 0
        bytes.extend(encode_operand(0));
        // one word, big-endian: 42
        bytes.extend(&[0x00, 0x00, 0x00, 0x2A]);
        // terminator
        bytes.push(0x00);

        let mut r = Reader::new(&bytes);
        let data = parse_data(&mut r).unwrap();

        assert_eq!(data.len(), 1);
        match &data[0] {
            DataItem::Words { offset, values } => {
                assert_eq!(*offset, 0);
                assert_eq!(values, &[42]);
            }
            _ => panic!("expected DataItem::Words"),
        }
    }

    #[test]
    fn parse_data_string() {
        let mut bytes = Vec::new();
        // datum code: type=3 (string), count=5
        bytes.push(0x35);
        // offset = 8
        bytes.extend(encode_operand(8));
        // 5 bytes of string data
        bytes.extend(b"hello");
        // terminator
        bytes.push(0x00);

        let mut r = Reader::new(&bytes);
        let data = parse_data(&mut r).unwrap();

        assert_eq!(data.len(), 1);
        match &data[0] {
            DataItem::String { offset, value } => {
                assert_eq!(*offset, 8);
                assert_eq!(value, "hello");
            }
            _ => panic!("expected DataItem::String"),
        }
    }

    #[test]
    fn parse_export_entry() {
        let mut bytes = Vec::new();
        bytes.extend(encode_operand(0)); // pc
        bytes.extend(encode_operand(1)); // frame_type
        bytes.extend(&[0x12, 0x34, 0x56, 0x78]); // signature (BE)
        bytes.extend(b"init\0"); // name

        let mut r = Reader::new(&bytes);
        let exports = parse_exports(&mut r, 1).unwrap();

        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].pc, 0);
        assert_eq!(exports[0].frame_type, 1);
        assert_eq!(exports[0].signature, 0x12345678);
        assert_eq!(exports[0].name, "init");
    }

    /// Regression: `encode_operand` must produce bytes that round-trip through
    /// `Reader::read_operand` for every representable value, including the
    /// 1-byte signed range (-64..=-1). An earlier version of this helper
    /// masked the value with `0xFF`, which placed `11` in the top two bits
    /// and misled the reader into parsing a 4-byte operand, corrupting every
    /// test that passed a negative value through the helper.
    #[test]
    fn encode_operand_roundtrips_for_all_ranges() {
        // The helper uses 1-byte encodings for small values and 4-byte
        // encoding otherwise. The 4-byte Dis operand sign-extends from bit 5
        // of the first byte, so the representable range is roughly ±2^29.
        // Stay inside that range so the roundtrip assertion is well-defined.
        let cases = [
            0,
            1,
            63, // 1-byte positive
            -1,
            -2,
            -32,
            -64, // 1-byte signed (the regression range)
            64,
            100,
            8191,
            -65,
            -8192, // 4-byte encoding
            (1 << 29) - 1,
            -(1 << 29), // edges of the 4-byte sign-extension window
        ];
        for value in cases {
            let bytes = encode_operand(value);
            let mut r = Reader::new(&bytes);
            let decoded = r
                .read_operand("roundtrip")
                .unwrap_or_else(|e| panic!("read_operand failed for value {value}: {e:?}"));
            assert_eq!(
                decoded, value,
                "encode_operand({value}) -> {bytes:?} decoded as {decoded}"
            );
            assert!(
                r.read_byte("after").is_err(),
                "encode_operand({value}) produced extra bytes: {bytes:?}"
            );
        }
    }

    /// Double-indirect addressing modes (src/dst codes 4 and 5) must read two
    /// operand registers, not one. A regression that reads only one would
    /// desynchronize every subsequent instruction.
    #[test]
    fn parse_code_double_indirect_reads_two_registers() {
        let mut bytes = Vec::new();
        // addr_code: mid=0, src=5 (double-indirect fp), dst=4 (double-indirect mp)
        //   0b00_101_100 = 0x2C
        bytes.push(Opcode::Addw as u8);
        bytes.push(0x2C);
        bytes.extend(encode_operand(12)); // src register1
        bytes.extend(encode_operand(4)); // src register2
        bytes.extend(encode_operand(7)); // dst register1
        bytes.extend(encode_operand(8)); // dst register2

        let mut r = Reader::new(&bytes);
        let code = parse_code(&mut r, 1).unwrap();

        assert_eq!(code[0].source.mode, AddressMode::OffsetDoubleIndirectFp);
        assert_eq!(code[0].source.register1, 12);
        assert_eq!(code[0].source.register2, 4);
        assert_eq!(
            code[0].destination.mode,
            AddressMode::OffsetDoubleIndirectMp
        );
        assert_eq!(code[0].destination.register1, 7);
        assert_eq!(code[0].destination.register2, 8);
        // No bytes should remain unread.
        assert!(
            r.read_byte("after").is_err(),
            "parser left bytes behind -> misaligned reads"
        );
    }

    /// Non-double-indirect modes must leave register2 at its default (0).
    #[test]
    fn parse_code_single_indirect_leaves_register2_zero() {
        let mut bytes = Vec::new();
        // mid=0, src=2 (immediate), dst=0 (indirect mp)
        //   0b00_010_000 = 0x10
        bytes.push(Opcode::Movw as u8);
        bytes.push(0x10);
        bytes.extend(encode_operand(5)); // src immediate value
        bytes.extend(encode_operand(16)); // dst mp offset

        let mut r = Reader::new(&bytes);
        let code = parse_code(&mut r, 1).unwrap();

        assert_eq!(code[0].source.mode, AddressMode::Immediate);
        assert_eq!(code[0].source.register1, 5);
        assert_eq!(code[0].source.register2, 0);
        assert_eq!(code[0].destination.mode, AddressMode::OffsetIndirectMp);
        assert_eq!(code[0].destination.register1, 16);
        assert_eq!(code[0].destination.register2, 0);
        assert!(r.read_byte("after").is_err());
    }

    /// All three middle-operand modes must parse a single register value.
    #[test]
    fn parse_code_middle_operand_modes() {
        let triples = [
            (1u8, MiddleMode::SmallImmediate, 42),
            (2u8, MiddleMode::SmallOffsetFp, 12),
            (3u8, MiddleMode::SmallOffsetMp, 20),
        ];
        for (mid_bits, expected_mode, mid_val) in triples {
            let mut bytes = Vec::new();
            // src=3 (unused), dst=3 (unused) -> 0b??_011_011 with ?? being mid
            let addr_code = (mid_bits << 6) | 0x1B;
            bytes.push(Opcode::Addw as u8);
            bytes.push(addr_code);
            bytes.extend(encode_operand(mid_val));

            let mut r = Reader::new(&bytes);
            let code = parse_code(&mut r, 1).unwrap();

            assert_eq!(code[0].middle.mode, expected_mode);
            assert_eq!(code[0].middle.register1, mid_val);
            assert!(
                r.read_byte("after").is_err(),
                "unread bytes for mid mode {mid_bits}"
            );
        }
    }

    /// Address mode bits 6 and 7 are reserved: the decoder must reject them.
    #[test]
    fn parse_code_rejects_invalid_address_mode() {
        for invalid in [6u8, 7u8] {
            let mut bytes = Vec::new();
            // src = invalid, dst = 3 (unused)
            let addr_code = (invalid << 3) | 0x03;
            bytes.push(Opcode::Exit as u8);
            bytes.push(addr_code);

            let mut r = Reader::new(&bytes);
            let result = parse_code(&mut r, 1);
            assert!(
                matches!(result, Err(LoadError::InvalidAddressMode(m)) if m == invalid),
                "address mode {invalid} should be rejected, got {result:?}"
            );
        }
    }

    #[test]
    fn parse_code_rejects_invalid_opcode() {
        let bytes = vec![0xFFu8, 0x1B];
        let mut r = Reader::new(&bytes);
        let result = parse_code(&mut r, 1);
        assert!(
            matches!(result, Err(LoadError::InvalidOpcode(0xFF))),
            "0xFF should be rejected as an unknown opcode, got {result:?}"
        );
    }

    /// value_bit64 with the high bit of the upper word set must decode as a
    /// negative i64, not a positive value from sign-ignored bit munging.
    #[test]
    fn parse_data_bigs_preserves_negative_sign() {
        let mut bytes = Vec::new();
        // datum code: type=8 (bigs), count=2
        bytes.push(0x82);
        // offset = 8
        bytes.extend(encode_operand(8));
        // first: -1 (0xFFFFFFFF_FFFFFFFF) big-endian
        bytes.extend(&[0xFF; 8]);
        // second: i64::MIN
        bytes.extend(&[0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        // terminator
        bytes.push(0x00);

        let mut r = Reader::new(&bytes);
        let data = parse_data(&mut r).unwrap();

        match &data[0] {
            DataItem::Bigs { offset, values } => {
                assert_eq!(*offset, 8);
                assert_eq!(values, &[-1i64, i64::MIN]);
            }
            other => panic!("expected Bigs, got {other:?}"),
        }
    }

    /// The data section must chain multiple items (bytes, array, set_array,
    /// restore_base) terminated by a zero byte.
    #[test]
    fn parse_data_chains_items_and_terminates() {
        let mut bytes = Vec::new();
        // array: type=5 count=0 -> inline extended count
        bytes.push(0x50);
        bytes.extend(encode_operand(1)); // count (extended)
        bytes.extend(encode_operand(0)); // offset
        bytes.extend(&[0x00, 0x00, 0x00, 0x02]); // element_type
        bytes.extend(&[0x00, 0x00, 0x00, 0x04]); // length
        // set_array: type=6 count=1
        bytes.push(0x61);
        bytes.extend(encode_operand(0)); // offset
        bytes.extend(&[0x00, 0x00, 0x00, 0x00]); // index
        // bytes item: type=1 count=3
        bytes.push(0x13);
        bytes.extend(encode_operand(0)); // offset
        bytes.extend(&[0xAA, 0xBB, 0xCC]);
        // restore_load_address: type=7 count=1 (embed count to avoid extended read)
        bytes.push(0x71);
        bytes.extend(encode_operand(0)); // offset (unused but still consumed)
        // terminator
        bytes.push(0x00);

        let mut r = Reader::new(&bytes);
        let items = parse_data(&mut r).unwrap();

        assert_eq!(items.len(), 4);
        matches!(items[0], DataItem::Array { .. });
        matches!(items[1], DataItem::SetArray { .. });
        if let DataItem::Bytes { values, .. } = &items[2] {
            assert_eq!(values, &vec![0xAAu8, 0xBB, 0xCC]);
        } else {
            panic!("expected Bytes");
        }
        matches!(items[3], DataItem::RestoreBase);
    }

    /// Unknown data item type 9 must raise InvalidDataType, not be silently
    /// skipped. Accidental acceptance would corrupt the MP.
    #[test]
    fn parse_data_rejects_unknown_item_type() {
        let mut bytes = Vec::new();
        bytes.push(0x91); // type=9, count=1
        bytes.extend(encode_operand(0));
        let mut r = Reader::new(&bytes);
        let result = parse_data(&mut r);
        assert!(
            matches!(result, Err(LoadError::InvalidDataType(9))),
            "data type 9 should be rejected, got {result:?}"
        );
    }

    /// Handler section encodes named cases followed by a wildcard pc. The
    /// decoder appends a wildcard `ExceptionCase` with `name == None` for it.
    #[test]
    fn parse_handlers_named_cases_plus_wildcard() {
        let mut bytes = Vec::new();
        bytes.extend(encode_operand(1)); // handler count
        bytes.extend(encode_operand(16)); // exception_offset
        bytes.extend(encode_operand(0)); // begin_pc
        bytes.extend(encode_operand(10)); // end_pc
        bytes.extend(encode_operand(-1)); // type_desc_number (-> None)
        // packed_cases: exception_type_count (upper 16) = 1, total_count (lower 16) = 2
        bytes.extend(encode_operand((1 << 16) | 2));
        bytes.extend(b"oops\0");
        bytes.extend(encode_operand(5)); // pc for "oops"
        bytes.extend(b"bad\0");
        bytes.extend(encode_operand(6)); // pc for "bad"
        bytes.extend(encode_operand(7)); // wildcard pc
        bytes.push(0x00); // trailing null

        let mut r = Reader::new(&bytes);
        let handlers = parse_handlers(&mut r).unwrap();

        assert_eq!(handlers.len(), 1);
        let h = &handlers[0];
        assert_eq!(h.exception_offset, 16);
        assert_eq!(h.begin_pc, 0);
        assert_eq!(h.end_pc, 10);
        assert_eq!(h.type_descriptor, None);
        // Two named cases plus an appended wildcard.
        assert_eq!(h.cases.len(), 3);
        assert_eq!(h.cases[0].name.as_deref(), Some("oops"));
        assert_eq!(h.cases[0].pc, 5);
        assert_eq!(h.cases[1].name.as_deref(), Some("bad"));
        assert_eq!(h.cases[1].pc, 6);
        assert!(h.cases[2].name.is_none(), "last case must be the wildcard");
        assert_eq!(h.cases[2].pc, 7);
    }

    /// Handler section without a trailing null byte must error, not succeed.
    #[test]
    fn parse_handlers_missing_trailing_null_errors() {
        let mut bytes = Vec::new();
        bytes.extend(encode_operand(0)); // 0 handlers
        bytes.push(0x01); // wrong terminator

        let mut r = Reader::new(&bytes);
        let result = parse_handlers(&mut r);
        assert!(matches!(result, Err(LoadError::Other(_))));
    }

    /// Import section decodes multiple modules with multiple functions and
    /// requires a trailing null byte.
    #[test]
    fn parse_imports_multiple_modules_with_trailing_null() {
        let mut bytes = Vec::new();
        bytes.extend(encode_operand(2)); // 2 modules
        // module 0: 1 function
        bytes.extend(encode_operand(1));
        bytes.extend(&[0xAA, 0xBB, 0xCC, 0xDD]); // signature
        bytes.extend(b"foo\0");
        // module 1: 2 functions
        bytes.extend(encode_operand(2));
        bytes.extend(&[0x11, 0x22, 0x33, 0x44]);
        bytes.extend(b"bar\0");
        bytes.extend(&[0x55, 0x66, 0x77, 0x88]);
        bytes.extend(b"baz\0");
        bytes.push(0x00); // trailing null

        let mut r = Reader::new(&bytes);
        let imports = parse_imports(&mut r).unwrap();

        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].functions.len(), 1);
        assert_eq!(imports[0].functions[0].name, "foo");
        assert_eq!(imports[0].functions[0].signature, 0xAABB_CCDDu32 as i32);
        assert_eq!(imports[1].functions.len(), 2);
        assert_eq!(imports[1].functions[0].name, "bar");
        assert_eq!(imports[1].functions[1].name, "baz");
    }

    /// Build a module buffer with custom header fields and empty sections,
    /// so validation tests can target specific header invariants.
    fn build_module_for_validation(
        code_size: i32,
        type_size: i32,
        entry_pc: i32,
        entry_type: i32,
        runtime_flags: u32,
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(encode_operand(XMAGIC));
        bytes.extend(encode_operand(runtime_flags as i32));
        bytes.extend(encode_operand(0)); // stack_extent
        bytes.extend(encode_operand(code_size));
        bytes.extend(encode_operand(0)); // data_size
        bytes.extend(encode_operand(type_size));
        bytes.extend(encode_operand(0)); // export_size
        bytes.extend(encode_operand(entry_pc));
        bytes.extend(encode_operand(entry_type));
        for _ in 0..code_size {
            bytes.push(Opcode::Exit as u8);
            bytes.push(0x1B);
        }
        for _ in 0..type_size {
            bytes.extend(encode_operand(0)); // id
            bytes.extend(encode_operand(0)); // size
            bytes.extend(encode_operand(0)); // map_in_bytes
        }
        bytes.push(0x00); // data terminator
        bytes.extend(b"m\0"); // name
        bytes
    }

    /// Entry PC outside the code range must fail validation. entry_pc == -1 is
    /// the only valid out-of-range value (library modules).
    #[test]
    fn validate_rejects_entry_pc_beyond_code() {
        let bytes = build_module_for_validation(2, 1, 5, 0, 0);
        let mut r = Reader::new(&bytes);
        let result = parse_module(&mut r);
        match result {
            Err(LoadError::ValidationError(msg)) => {
                assert!(
                    msg.contains("entry_pc"),
                    "error should mention entry_pc, got: {msg}"
                );
            }
            other => panic!("expected ValidationError, got {other:?}"),
        }
    }

    /// entry_pc == -1 is valid (library module with no entry function).
    #[test]
    fn validate_accepts_entry_pc_minus_one_for_library_modules() {
        let bytes = build_module_for_validation(1, 1, -1, 0, 0);
        let mut r = Reader::new(&bytes);
        parse_module(&mut r).expect("entry_pc == -1 must be accepted");
    }

    /// Setting both MUST_COMPILE and DONT_COMPILE is contradictory.
    #[test]
    fn validate_rejects_conflicting_compile_flags() {
        let flags = RuntimeFlags::MUST_COMPILE.0 | RuntimeFlags::DONT_COMPILE.0;
        let bytes = build_module_for_validation(1, 1, 0, 0, flags);
        let mut r = Reader::new(&bytes);
        let result = parse_module(&mut r);
        match result {
            Err(LoadError::ValidationError(msg)) => {
                assert!(
                    msg.contains("MUST_COMPILE") && msg.contains("DONT_COMPILE"),
                    "error should mention both flags, got: {msg}"
                );
            }
            other => panic!("expected ValidationError, got {other:?}"),
        }
    }
}
