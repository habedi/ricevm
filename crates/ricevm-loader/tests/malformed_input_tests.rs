//! Malformed-input tests for the loader.
//!
//! Every case here feeds `ricevm_loader::load` a byte sequence that a hostile
//! or corrupt `.dis` file could contain. The loader must answer with a
//! `LoadError`; it must never panic, never index out of bounds, and never
//! reserve memory proportional to an attacker-chosen count instead of the
//! bytes actually present in the file.

use ricevm_core::{Opcode, SMAGIC, XMAGIC};

const HAS_HANDLER: u32 = 1 << 5;
const HAS_IMPORT: u32 = 1 << 6;

/// Encode an i32 as a Dis variable-length operand.
fn encode_operand(value: i32) -> Vec<u8> {
    if (0..=63).contains(&value) {
        vec![value as u8]
    } else if (-64..=-1).contains(&value) {
        vec![0x40 | ((value & 0x3F) as u8)]
    } else {
        let mut buf = [0u8; 4];
        buf[0] = 0xC0 | (((value >> 24) as u8) & 0x3F);
        buf[1] = (value >> 16) as u8;
        buf[2] = (value >> 8) as u8;
        buf[3] = value as u8;
        buf.to_vec()
    }
}

/// Header fields, so each test can perturb exactly one of them.
struct HeaderSpec {
    flags: u32,
    stack_extent: i32,
    code_size: i32,
    data_size: i32,
    type_size: i32,
    export_size: i32,
    entry_pc: i32,
    entry_type: i32,
}

impl Default for HeaderSpec {
    fn default() -> Self {
        Self {
            flags: 0,
            stack_extent: 0,
            code_size: 1,
            data_size: 0,
            type_size: 0,
            export_size: 0,
            entry_pc: 0,
            entry_type: 0,
        }
    }
}

impl HeaderSpec {
    fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(encode_operand(XMAGIC));
        out.extend(encode_operand(self.flags as i32));
        out.extend(encode_operand(self.stack_extent));
        out.extend(encode_operand(self.code_size));
        out.extend(encode_operand(self.data_size));
        out.extend(encode_operand(self.type_size));
        out.extend(encode_operand(self.export_size));
        out.extend(encode_operand(self.entry_pc));
        out.extend(encode_operand(self.entry_type));
        out
    }
}

/// One `exit` instruction with both operands unused.
fn exit_instruction() -> Vec<u8> {
    vec![Opcode::Exit as u8, 0x1B]
}

/// A module that must load cleanly, so the builders above stay honest.
fn valid_module() -> Vec<u8> {
    let mut bytes = HeaderSpec::default().bytes();
    bytes.extend(exit_instruction());
    bytes.push(0x00); // data terminator
    bytes.extend(b"m\0");
    bytes
}

/// A module with a handler section: `code_size` instructions, `type_size`
/// trivial type descriptors, and one handler with a single named case.
fn module_with_handler(
    code_size: i32,
    type_size: i32,
    type_desc_number: i32,
    case_pc: i32,
    wildcard_pc: i32,
) -> Vec<u8> {
    let spec = HeaderSpec {
        flags: HAS_HANDLER,
        code_size,
        type_size,
        ..Default::default()
    };
    let mut bytes = spec.bytes();
    for _ in 0..code_size {
        bytes.extend(exit_instruction());
    }
    for _ in 0..type_size {
        bytes.extend(encode_operand(0)); // id
        bytes.extend(encode_operand(0)); // size
        bytes.extend(encode_operand(0)); // map_in_bytes
    }
    bytes.push(0x00); // data terminator
    bytes.extend(b"m\0");
    // handler section
    bytes.extend(encode_operand(1)); // handler count
    bytes.extend(encode_operand(0)); // exception_offset
    bytes.extend(encode_operand(0)); // begin_pc
    bytes.extend(encode_operand(code_size)); // end_pc
    bytes.extend(encode_operand(type_desc_number));
    bytes.extend(encode_operand((1 << 16) | 1)); // one named case
    bytes.extend(b"oops\0");
    bytes.extend(encode_operand(case_pc));
    bytes.extend(encode_operand(wildcard_pc));
    bytes.push(0x00); // trailing null
    bytes
}

#[track_caller]
fn assert_rejected(bytes: &[u8], what: &str) {
    match ricevm_loader::load(bytes) {
        Err(_) => {}
        Ok(m) => panic!(
            "{what}: loader accepted malformed input (module {:?})",
            m.name
        ),
    }
}

#[test]
fn builders_produce_a_loadable_module() {
    ricevm_loader::load(&valid_module()).expect("baseline module must load");
    ricevm_loader::load(&module_with_handler(2, 1, -1, 1, 0))
        .expect("baseline handler module must load");
}

// --- Header signature length (findings 1 and 2) ---

#[test]
fn negative_signature_length_is_rejected() {
    // SMAGIC then a signature length of -1: cast to usize this is
    // usize::MAX, which overflows the reader's bounds check.
    let mut bytes = encode_operand(SMAGIC);
    bytes.extend(encode_operand(-1));
    bytes.extend(b"whatever");
    assert_rejected(&bytes, "negative signature length");
}

#[test]
fn huge_signature_length_is_rejected() {
    let mut bytes = encode_operand(SMAGIC);
    bytes.extend(encode_operand(0x1FFF_FFFF));
    bytes.extend(b"whatever");
    assert_rejected(&bytes, "huge signature length");
}

// --- Section counts taken from the header (finding 3) ---

#[test]
fn negative_code_size_is_rejected() {
    let bytes = HeaderSpec {
        code_size: -1,
        ..Default::default()
    }
    .bytes();
    assert_rejected(&bytes, "negative code size");
}

#[test]
fn huge_code_size_is_rejected() {
    let mut bytes = HeaderSpec {
        code_size: 0x0FFF_FFFF,
        ..Default::default()
    }
    .bytes();
    bytes.extend(exit_instruction());
    assert_rejected(&bytes, "huge code size");
}

#[test]
fn negative_type_size_is_rejected() {
    let mut bytes = HeaderSpec {
        type_size: -1,
        ..Default::default()
    }
    .bytes();
    bytes.extend(exit_instruction());
    assert_rejected(&bytes, "negative type size");
}

#[test]
fn huge_type_size_is_rejected() {
    let mut bytes = HeaderSpec {
        type_size: 0x0FFF_FFFF,
        ..Default::default()
    }
    .bytes();
    bytes.extend(exit_instruction());
    assert_rejected(&bytes, "huge type size");
}

#[test]
fn negative_export_size_is_rejected() {
    let mut bytes = HeaderSpec {
        export_size: -1,
        ..Default::default()
    }
    .bytes();
    bytes.extend(exit_instruction());
    bytes.push(0x00); // data terminator
    bytes.extend(b"m\0");
    assert_rejected(&bytes, "negative export size");
}

#[test]
fn negative_import_module_count_is_rejected() {
    let spec = HeaderSpec {
        flags: HAS_IMPORT,
        ..Default::default()
    };
    let mut bytes = spec.bytes();
    bytes.extend(exit_instruction());
    bytes.push(0x00);
    bytes.extend(b"m\0");
    bytes.extend(encode_operand(-1)); // import module count
    assert_rejected(&bytes, "negative import module count");
}

#[test]
fn negative_import_function_count_is_rejected() {
    let spec = HeaderSpec {
        flags: HAS_IMPORT,
        ..Default::default()
    };
    let mut bytes = spec.bytes();
    bytes.extend(exit_instruction());
    bytes.push(0x00);
    bytes.extend(b"m\0");
    bytes.extend(encode_operand(1)); // one import module
    bytes.extend(encode_operand(-1)); // negative function count
    assert_rejected(&bytes, "negative import function count");
}

#[test]
fn negative_handler_count_is_rejected() {
    let spec = HeaderSpec {
        flags: HAS_HANDLER,
        ..Default::default()
    };
    let mut bytes = spec.bytes();
    bytes.extend(exit_instruction());
    bytes.push(0x00);
    bytes.extend(b"m\0");
    bytes.extend(encode_operand(-1)); // handler count
    assert_rejected(&bytes, "negative handler count");
}

// --- Type descriptor pointer map length (finding 4) ---

#[test]
fn negative_type_map_length_is_rejected() {
    let mut bytes = HeaderSpec {
        type_size: 1,
        ..Default::default()
    }
    .bytes();
    bytes.extend(exit_instruction());
    bytes.extend(encode_operand(0)); // id
    bytes.extend(encode_operand(0)); // size
    bytes.extend(encode_operand(-1)); // map_in_bytes
    assert_rejected(&bytes, "negative type map length");
}

// --- Data item counts and offsets (findings 5 and 8) ---

/// Build a module whose data section holds one item with an extended
/// (out-of-line) count.
fn module_with_data_item(item_code: u8, count: i32, offset: i32, payload: &[u8]) -> Vec<u8> {
    let mut bytes = HeaderSpec::default().bytes();
    bytes.extend(exit_instruction());
    bytes.push(item_code);
    if item_code & 0x0F == 0 {
        bytes.extend(encode_operand(count));
    }
    bytes.extend(encode_operand(offset));
    bytes.extend(payload);
    bytes.push(0x00); // data terminator
    bytes.extend(b"m\0");
    bytes
}

#[test]
fn negative_data_byte_count_is_rejected() {
    // item type 1 (bytes), extended count = -1
    let bytes = module_with_data_item(0x10, -1, 0, &[]);
    assert_rejected(&bytes, "negative byte-data count");
}

#[test]
fn negative_data_word_count_is_rejected() {
    // item type 2 (words), extended count = -1
    let bytes = module_with_data_item(0x20, -1, 0, &[]);
    assert_rejected(&bytes, "negative word-data count");
}

#[test]
fn negative_data_real_count_is_rejected() {
    let bytes = module_with_data_item(0x40, -1, 0, &[]);
    assert_rejected(&bytes, "negative real-data count");
}

#[test]
fn negative_data_big_count_is_rejected() {
    let bytes = module_with_data_item(0x80, -1, 0, &[]);
    assert_rejected(&bytes, "negative big-data count");
}

#[test]
fn huge_data_word_count_is_rejected() {
    let bytes = module_with_data_item(0x20, 0x1FFF_FFFF, 0, &[]);
    assert_rejected(&bytes, "huge word-data count");
}

#[test]
fn huge_data_big_count_is_rejected() {
    let bytes = module_with_data_item(0x80, 0x1FFF_FFFF, 0, &[]);
    assert_rejected(&bytes, "huge big-data count");
}

#[test]
fn negative_data_offset_is_rejected() {
    // item type 1 (bytes), inline count = 1, offset = -1
    let bytes = module_with_data_item(0x11, 1, -1, &[0xAA]);
    assert_rejected(&bytes, "negative data offset");
}

// --- Handler validation (findings 6 and 7) ---

#[test]
fn handler_type_descriptor_out_of_range_is_rejected() {
    // one type descriptor in the table, handler references index 5
    let bytes = module_with_handler(2, 1, 5, 1, 0);
    assert_rejected(&bytes, "handler type descriptor out of range");
}

#[test]
fn handler_type_descriptor_negative_is_rejected() {
    // -1 means "no type"; -2 is nonsense and would become a huge u32.
    let bytes = module_with_handler(2, 1, -2, 1, 0);
    assert_rejected(&bytes, "negative handler type descriptor");
}

#[test]
fn handler_case_pc_out_of_range_is_rejected() {
    let bytes = module_with_handler(2, 1, -1, 99, 0);
    assert_rejected(&bytes, "handler case pc past end of code");
}

#[test]
fn handler_wildcard_pc_out_of_range_is_rejected() {
    let bytes = module_with_handler(2, 1, -1, 1, 99);
    assert_rejected(&bytes, "handler wildcard pc past end of code");
}

#[test]
fn handler_named_case_pc_minus_one_is_rejected() {
    // Only the trailing wildcard case may carry the -1 sentinel.
    let bytes = module_with_handler(2, 1, -1, -1, 0);
    assert_rejected(&bytes, "named handler case with pc -1");
}

#[test]
fn handler_wildcard_pc_minus_one_is_accepted() {
    // -1 on the wildcard case means "this handler has no catch-all";
    // Inferno's loader emits it, so it must keep loading.
    let bytes = module_with_handler(2, 1, -1, 1, -1);
    ricevm_loader::load(&bytes).expect("wildcard pc -1 is a legitimate sentinel");
}
