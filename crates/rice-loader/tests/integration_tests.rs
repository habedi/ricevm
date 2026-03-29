//! Integration tests for the RiceVM loader.

#[test]
fn load_invalid_magic_returns_error() {
    // 0xFF 0xFF 0xFF 0xFF decodes as operand -1, which is not XMAGIC or SMAGIC
    let data = [0xFF, 0xFF, 0xFF, 0xFF];
    let result = ricevm_loader::load(&data);
    assert!(result.is_err());
}

#[test]
fn load_truncated_module_returns_eof_error() {
    // Valid XMAGIC (0x0C8030) encoded as 4-byte operand, then nothing
    let data = [0xC0, 0x0C, 0x80, 0x30];
    let result = ricevm_loader::load(&data);
    assert!(result.is_err());
}

#[test]
fn load_empty_returns_error() {
    let result = ricevm_loader::load(&[]);
    assert!(result.is_err());
}
