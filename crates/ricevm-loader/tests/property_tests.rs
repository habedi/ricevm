//! Loader robustness tests.

use std::panic::{self, AssertUnwindSafe};

use ricevm_core::XMAGIC;

fn encode_operand(value: i32) -> Vec<u8> {
    if (0..=63).contains(&value) {
        vec![value as u8]
    } else if (-64..=-1).contains(&value) {
        vec![(value & 0xFF) as u8]
    } else {
        let mut buf = [0u8; 4];
        buf[0] = 0xC0 | (((value >> 24) as u8) & 0x3F);
        buf[1] = (value >> 16) as u8;
        buf[2] = (value >> 8) as u8;
        buf[3] = value as u8;
        buf.to_vec()
    }
}

fn build_valid_module() -> Vec<u8> {
    let mut bytes = Vec::new();

    bytes.extend(encode_operand(XMAGIC));
    bytes.extend(encode_operand(0)); // runtime_flags
    bytes.extend(encode_operand(12)); // stack_extent
    bytes.extend(encode_operand(1)); // code_size
    bytes.extend(encode_operand(4)); // data_size
    bytes.extend(encode_operand(1)); // type_size
    bytes.extend(encode_operand(0)); // export_size
    bytes.extend(encode_operand(0)); // entry_pc
    bytes.extend(encode_operand(0)); // entry_type

    // exit, with no operands
    bytes.push(ricevm_core::Opcode::Exit as u8);
    bytes.push(0x1B);

    // one type descriptor, 32 bytes, no pointers
    bytes.extend(encode_operand(0));
    bytes.extend(encode_operand(32));
    bytes.extend(encode_operand(0));

    // data: one word at mp[0], then terminator
    bytes.push(0x21);
    bytes.extend(encode_operand(0));
    bytes.extend(&1234_i32.to_be_bytes());
    bytes.push(0x00);

    bytes.extend(b"header_roundtrip\0");
    bytes
}

fn visit_interesting_prefixes(
    alphabet: &[u8],
    buf: &mut Vec<u8>,
    remaining: usize,
    visit: &mut dyn FnMut(&[u8]),
) {
    visit(buf);
    if remaining == 0 {
        return;
    }

    for &byte in alphabet {
        buf.push(byte);
        visit_interesting_prefixes(alphabet, buf, remaining - 1, visit);
        buf.pop();
    }
}

#[test]
fn opcode_roundtrip() {
    for byte in 0x00..=0xAF_u8 {
        let op = ricevm_core::Opcode::try_from(byte).unwrap();
        assert_eq!(op as u8, byte);
    }
}

#[test]
fn arbitrary_bytes_never_panic_loader() {
    let alphabet = [0x00, 0x01, 0x3F, 0x40, 0x7F, 0x80, 0xBF, 0xC0, 0xFF];
    let mut buf = Vec::new();
    let mut cases = 0usize;

    visit_interesting_prefixes(&alphabet, &mut buf, 4, &mut |bytes| {
        let result = panic::catch_unwind(AssertUnwindSafe(|| ricevm_loader::load(bytes)));
        assert!(result.is_ok(), "loader panicked on bytes: {bytes:02X?}");
        cases += 1;
    });

    // Add a deterministic pseudo-random corpus to cover longer slices.
    let mut seed = 0x5EED_1234_u32;
    for len in [8usize, 16, 32, 64, 128] {
        for _ in 0..64 {
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                *byte = (seed >> 24) as u8;
            }
            let result = panic::catch_unwind(AssertUnwindSafe(|| ricevm_loader::load(&bytes)));
            assert!(result.is_ok(), "loader panicked on bytes: {bytes:02X?}");
            cases += 1;
        }
    }

    assert!(cases > 7_000, "expected a broad generated corpus");
}

#[test]
fn valid_module_roundtrip_header_fields() {
    let bytes = build_valid_module();
    let module = ricevm_loader::load(&bytes).expect("valid synthetic module should parse");

    assert_eq!(module.header.magic, XMAGIC);
    assert!(module.header.signature.is_empty());
    assert_eq!(module.header.runtime_flags.0, 0);
    assert_eq!(module.header.stack_extent, 12);
    assert_eq!(module.header.code_size, 1);
    assert_eq!(module.header.data_size, 4);
    assert_eq!(module.header.type_size, 1);
    assert_eq!(module.header.export_size, 0);
    assert_eq!(module.header.entry_pc, 0);
    assert_eq!(module.header.entry_type, 0);

    assert_eq!(module.name, "header_roundtrip");
    assert_eq!(module.code.len(), 1);
    assert_eq!(module.types.len(), 1);
    assert_eq!(module.data.len(), 1);
    match &module.data[0] {
        ricevm_core::DataItem::Words { offset, values } => {
            assert_eq!(*offset, 0);
            assert_eq!(values, &vec![1234]);
        }
        other => panic!("expected one word data item, got {other:?}"),
    }
}
