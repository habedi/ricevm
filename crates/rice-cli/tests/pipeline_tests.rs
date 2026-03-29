//! End-to-end pipeline tests (loader → executor).

/// Encode an i32 as a Dis variable-length operand.
fn encode_operand(value: i32) -> Vec<u8> {
    if (0..=63).contains(&value) {
        vec![value as u8]
    } else if (-64..=-1).contains(&value) {
        vec![(value & 0xFF) as u8]
    } else {
        // 4-byte encoding
        let mut buf = [0u8; 4];
        buf[0] = 0xC0 | (((value >> 24) as u8) & 0x3F);
        buf[1] = (value >> 16) as u8;
        buf[2] = (value >> 8) as u8;
        buf[3] = value as u8;
        buf.to_vec()
    }
}

/// Build a minimal .dis module binary with a single Exit instruction.
fn build_exit_module() -> Vec<u8> {
    let mut bytes = Vec::new();

    // Header
    bytes.extend(encode_operand(0x0C8030)); // XMAGIC
    bytes.extend(encode_operand(0)); // runtime_flags
    bytes.extend(encode_operand(0)); // stack_extent
    bytes.extend(encode_operand(1)); // code_size (1 instruction)
    bytes.extend(encode_operand(0)); // data_size
    bytes.extend(encode_operand(1)); // type_size (1 type descriptor)
    bytes.extend(encode_operand(0)); // export_size
    bytes.extend(encode_operand(0)); // entry_pc
    bytes.extend(encode_operand(0)); // entry_type

    // Code section: 1 Exit instruction
    // opcode = 0x0F (Exit), addr_code = 0x1B (mid=0, src=3/none, dst=3/none)
    bytes.push(0x0F);
    bytes.push(0x1B);

    // Type section: 1 type descriptor
    bytes.extend(encode_operand(0)); // desc_number = 0
    bytes.extend(encode_operand(32)); // size = 32 bytes
    bytes.extend(encode_operand(0)); // map_in_bytes = 0 (no pointers)

    // Data section: empty (just terminator)
    bytes.push(0x00);

    // Module name
    bytes.extend(b"exit_test\0");

    // No exports (count is 0)
    // No imports (flag not set)
    // No handlers (flag not set)

    bytes
}

#[test]
fn load_and_execute_exit_module() {
    let dis_bytes = build_exit_module();
    let module = ricevm_loader::load(&dis_bytes).expect("should parse exit module");
    assert_eq!(module.name, "exit_test");
    assert_eq!(module.code.len(), 1);
    assert_eq!(module.code[0].opcode, ricevm_core::Opcode::Exit);
    ricevm_execute::execute(&module).expect("should execute cleanly");
}

#[test]
fn load_and_execute_arithmetic_module() {
    let mut bytes = Vec::new();

    // Header
    bytes.extend(encode_operand(0x0C8030)); // XMAGIC
    bytes.extend(encode_operand(0)); // runtime_flags
    bytes.extend(encode_operand(0)); // stack_extent
    bytes.extend(encode_operand(4)); // code_size (4 instructions)
    bytes.extend(encode_operand(0)); // data_size
    bytes.extend(encode_operand(1)); // type_size
    bytes.extend(encode_operand(0)); // export_size
    bytes.extend(encode_operand(0)); // entry_pc
    bytes.extend(encode_operand(0)); // entry_type

    // Code section: 4 instructions
    // 0: movw $10, 0(fp) — src=immediate(10), dst=fp(0)
    // addr_code: mid=0(00), src=2(010=imm), dst=1(001=fp) → 0b00_010_001 = 0x11
    bytes.push(0x2D); // opcode = Movw (0x2D)
    bytes.push(0x11); // addr_code
    bytes.extend(encode_operand(10)); // src: immediate value 10
    bytes.extend(encode_operand(0)); // dst: fp offset 0

    // 1: movw $20, 4(fp) — src=immediate(20), dst=fp(4)
    bytes.push(0x2D); // Movw
    bytes.push(0x11); // same addressing
    bytes.extend(encode_operand(20));
    bytes.extend(encode_operand(4));

    // 2: addw 0(fp), 4(fp), 8(fp) — src=fp(0), mid=fp(4), dst=fp(8)
    // addr_code: mid=2(10=small_fp), src=1(001=fp), dst=1(001=fp) → 0b10_001_001 = 0x89
    bytes.push(0x3A); // Addw (0x3A)
    bytes.push(0x89); // addr_code
    bytes.extend(encode_operand(4)); // mid: fp offset 4
    bytes.extend(encode_operand(0)); // src: fp offset 0
    bytes.extend(encode_operand(8)); // dst: fp offset 8

    // 3: exit
    bytes.push(0x0F); // Exit
    bytes.push(0x1B); // mid=0, src=none, dst=none

    // Type section: 1 descriptor
    bytes.extend(encode_operand(0)); // desc_number
    bytes.extend(encode_operand(32)); // size
    bytes.extend(encode_operand(0)); // map_in_bytes

    // Data section: empty
    bytes.push(0x00);

    // Module name
    bytes.extend(b"arith_test\0");

    let module = ricevm_loader::load(&bytes).expect("should parse arithmetic module");
    assert_eq!(module.name, "arith_test");
    assert_eq!(module.code.len(), 4);
    ricevm_execute::execute(&module).expect("should execute cleanly");
}

#[test]
#[ignore = "requires real Limbo-compiled .dis file"]
fn load_and_execute_hello_world() {
    todo!()
}
