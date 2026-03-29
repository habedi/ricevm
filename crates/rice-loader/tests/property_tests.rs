//! Property-based tests for the RiceVM loader.

#[test]
fn opcode_roundtrip() {
    for byte in 0x00..=0xAF_u8 {
        let op = ricevm_core::Opcode::try_from(byte).unwrap();
        assert_eq!(op as u8, byte);
    }
}

#[test]
#[ignore = "requires proptest dependency"]
fn arbitrary_bytes_never_panic_loader() {
    // Feed random byte slices to the loader and verify
    // it never panics, only returns Ok or Err.
    // TODO: use proptest to generate arbitrary Vec<u8>
    todo!()
}

#[test]
#[ignore = "requires real .dis files"]
fn valid_module_roundtrip_header_fields() {
    // Parse a valid .dis file, then verify that all header
    // fields match expected values from the binary.
    todo!()
}
