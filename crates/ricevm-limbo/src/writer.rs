//! Dis module binary writer.
//!
//! Serializes a `ricevm_core::Module` to the `.dis` binary format.

use ricevm_core::{AddressMode, DataItem, MiddleMode, Module, XMAGIC};

/// Write a Module to the Dis binary format.
pub fn write_dis(module: &Module) -> Vec<u8> {
    let mut buf = Vec::new();

    write_op(&mut buf, XMAGIC);
    // For XMAGIC, no signature length field is written
    write_op(&mut buf, module.header.runtime_flags.0 as i32);
    write_op(&mut buf, module.header.stack_extent);
    write_op(&mut buf, module.header.code_size);
    write_op(&mut buf, module.header.data_size);
    write_op(&mut buf, module.header.type_size);
    write_op(&mut buf, module.header.export_size);
    write_op(&mut buf, module.header.entry_pc);
    write_op(&mut buf, module.header.entry_type);

    // Code section
    for inst in &module.code {
        buf.push(inst.opcode as u8);
        let mid_mode = encode_mid_mode(inst.middle.mode);
        let src_mode = encode_addr_mode(inst.source.mode);
        let dst_mode = encode_addr_mode(inst.destination.mode);
        buf.push((mid_mode << 6) | (src_mode << 3) | dst_mode);
        write_middle_operand(&mut buf, &inst.middle);
        write_operand(&mut buf, &inst.source);
        write_operand(&mut buf, &inst.destination);
    }

    // Type descriptors
    for td in &module.types {
        write_op(&mut buf, td.id as i32);
        write_op(&mut buf, td.size);
        let map_len = td.pointer_map.bytes.len() as i32;
        write_op(&mut buf, map_len);
        for &b in &td.pointer_map.bytes {
            buf.push(b);
        }
    }

    // Data section
    for item in &module.data {
        write_data_item(&mut buf, item);
    }
    buf.push(0x00); // data section terminator

    // Module name (null-terminated)
    write_cstring(&mut buf, &module.name);

    // Export section (count comes from header.export_size, not written here)
    for export in &module.exports {
        write_op(&mut buf, export.pc);
        write_op(&mut buf, export.frame_type);
        // Signature is 4-byte big-endian
        buf.extend_from_slice(&export.signature.to_be_bytes());
        write_cstring(&mut buf, &export.name);
    }

    // Import section
    write_op(&mut buf, module.imports.len() as i32);
    for import_mod in &module.imports {
        write_op(&mut buf, import_mod.functions.len() as i32);
        for func in &import_mod.functions {
            // Signature is 4-byte big-endian
            buf.extend_from_slice(&func.signature.to_be_bytes());
            write_cstring(&mut buf, &func.name);
        }
    }

    // Import section trailing null byte
    if !module.imports.is_empty() {
        buf.push(0x00);
    }

    // Handler section (only if HAS_HANDLER flag is set)
    if module
        .header
        .runtime_flags
        .contains(ricevm_core::RuntimeFlags::HAS_HANDLER)
    {
        write_op(&mut buf, module.handlers.len() as i32);
        for handler in &module.handlers {
            write_op(&mut buf, handler.exception_offset);
            write_op(&mut buf, handler.begin_pc);
            write_op(&mut buf, handler.end_pc);
            // Type descriptor (-1 for none)
            write_op(
                &mut buf,
                handler.type_descriptor.map(|t| t as i32).unwrap_or(-1),
            );
            // Named cases (exclude wildcard which is last)
            let named_cases: Vec<_> = handler.cases.iter().filter(|c| c.name.is_some()).collect();
            let wildcard = handler.cases.iter().find(|c| c.name.is_none());
            // packed_cases: (exception_type_count << 16) | total_named_count
            let packed = (named_cases.len() as i32) & 0xFFFF;
            write_op(&mut buf, packed);
            for case in &named_cases {
                if let Some(name) = &case.name {
                    write_cstring(&mut buf, name);
                }
                write_op(&mut buf, case.pc);
            }
            // Wildcard PC
            write_op(&mut buf, wildcard.map(|c| c.pc).unwrap_or(-1));
        }
        // Trailing null byte
        buf.push(0x00);
    }

    buf
}

fn write_op(buf: &mut Vec<u8>, val: i32) {
    // Operand encoding matches the Dis binary format:
    // 1-byte: 0x00-0x3F = 0..63, 0x40-0x7F = -64..-1
    // 2-byte: 0x80-0xBF prefix (values -8192..8191)
    // 4-byte: 0xC0-0xFF prefix (all other values)
    if (0..64).contains(&val) {
        buf.push(val as u8);
    } else if (-64..0).contains(&val) {
        buf.push(val as u8); // sign-extended 1-byte negative
    } else if (-8192..8192).contains(&val) {
        let v = val as u16;
        buf.push(((v >> 8) & 0x3F | 0x80) as u8);
        buf.push(v as u8);
    } else {
        let v = val as u32;
        buf.push(((v >> 24) & 0x3F | 0xC0) as u8);
        buf.push((v >> 16) as u8);
        buf.push((v >> 8) as u8);
        buf.push(v as u8);
    }
}

fn write_cstring(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(s.as_bytes());
    buf.push(0x00); // null terminator
}

fn encode_addr_mode(mode: AddressMode) -> u8 {
    match mode {
        AddressMode::OffsetIndirectMp => 0,
        AddressMode::OffsetIndirectFp => 1,
        AddressMode::Immediate => 2,
        AddressMode::None => 3,
        AddressMode::OffsetDoubleIndirectMp => 4,
        AddressMode::OffsetDoubleIndirectFp => 5,
        _ => 3, // None
    }
}

fn encode_mid_mode(mode: MiddleMode) -> u8 {
    match mode {
        MiddleMode::None => 0,
        MiddleMode::SmallImmediate => 1,
        MiddleMode::SmallOffsetFp => 2,
        MiddleMode::SmallOffsetMp => 3,
    }
}

fn write_middle_operand(buf: &mut Vec<u8>, mid: &ricevm_core::MiddleOperand) {
    match mid.mode {
        MiddleMode::None => {}
        _ => write_op(buf, mid.register1),
    }
}

fn write_operand(buf: &mut Vec<u8>, op: &ricevm_core::Operand) {
    match op.mode {
        AddressMode::None => {}
        AddressMode::OffsetIndirectFp | AddressMode::OffsetIndirectMp | AddressMode::Immediate => {
            write_op(buf, op.register1);
        }
        AddressMode::OffsetDoubleIndirectFp | AddressMode::OffsetDoubleIndirectMp => {
            write_op(buf, op.register1);
            write_op(buf, op.register2);
        }
        _ => {}
    }
}

fn write_data_item(buf: &mut Vec<u8>, item: &DataItem) {
    // Data item format: (type << 4) | count_low, then offset, then data.
    // If count > 15 or count == 0, count_low = 0 and a separate operand-encoded count follows.
    match item {
        DataItem::Bytes { offset, values } => {
            write_data_header(buf, 1, values.len() as i32, *offset);
            buf.extend_from_slice(values);
        }
        DataItem::Words { offset, values } => {
            write_data_header(buf, 2, values.len() as i32, *offset);
            for w in values {
                buf.extend_from_slice(&w.to_be_bytes());
            }
        }
        DataItem::String { offset, value } => {
            write_data_header(buf, 3, value.len() as i32, *offset);
            buf.extend_from_slice(value.as_bytes());
        }
        DataItem::Reals { offset, values } => {
            write_data_header(buf, 4, values.len() as i32, *offset);
            for v in values {
                buf.extend_from_slice(&v.to_be_bytes());
            }
        }
        DataItem::Array {
            offset,
            element_type,
            length,
        } => {
            // Array uses count=1 in header; the actual type and length follow
            buf.push((5 << 4) | 1);
            write_op(buf, *offset);
            buf.extend_from_slice(&element_type.to_be_bytes());
            buf.extend_from_slice(&length.to_be_bytes());
        }
        DataItem::SetArray { offset: _, index } => {
            buf.push((6 << 4) | 1);
            buf.extend_from_slice(&index.to_be_bytes());
        }
        DataItem::RestoreBase => {
            buf.push(7 << 4);
        }
        DataItem::Bigs { offset, values } => {
            write_data_header(buf, 8, values.len() as i32, *offset);
            for v in values {
                buf.extend_from_slice(&v.to_be_bytes());
            }
        }
    }
}

fn write_data_header(buf: &mut Vec<u8>, item_type: u8, count: i32, offset: i32) {
    if count > 0 && count <= 15 {
        buf.push((item_type << 4) | count as u8);
    } else {
        buf.push(item_type << 4);
        write_op(buf, count);
    }
    write_op(buf, offset);
}
