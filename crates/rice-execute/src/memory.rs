//! Typed read/write helpers for flat byte buffers.
//!
//! All operations use native-endian byte order.
//! Out-of-bounds reads return 0; out-of-bounds writes are no-ops.

use ricevm_core::{Big, Byte, Real, Word};

pub(crate) fn read_word(buf: &[u8], offset: usize) -> Word {
    if offset + 4 > buf.len() {
        return 0;
    }
    let bytes: [u8; 4] = buf[offset..offset + 4].try_into().unwrap_or([0; 4]);
    Word::from_ne_bytes(bytes)
}

pub(crate) fn write_word(buf: &mut [u8], offset: usize, val: Word) {
    if offset + 4 > buf.len() {
        return;
    }
    buf[offset..offset + 4].copy_from_slice(&val.to_ne_bytes());
}

pub(crate) fn read_big(buf: &[u8], offset: usize) -> Big {
    if offset + 8 > buf.len() {
        return 0;
    }
    let bytes: [u8; 8] = buf[offset..offset + 8].try_into().unwrap_or([0; 8]);
    Big::from_ne_bytes(bytes)
}

pub(crate) fn write_big(buf: &mut [u8], offset: usize, val: Big) {
    if offset + 8 > buf.len() {
        return;
    }
    buf[offset..offset + 8].copy_from_slice(&val.to_ne_bytes());
}

pub(crate) fn read_real(buf: &[u8], offset: usize) -> Real {
    if offset + 8 > buf.len() {
        return 0.0;
    }
    let bytes: [u8; 8] = buf[offset..offset + 8].try_into().unwrap_or([0; 8]);
    Real::from_ne_bytes(bytes)
}

pub(crate) fn write_real(buf: &mut [u8], offset: usize, val: Real) {
    if offset + 8 > buf.len() {
        return;
    }
    buf[offset..offset + 8].copy_from_slice(&val.to_ne_bytes());
}

pub(crate) fn read_byte(buf: &[u8], offset: usize) -> Byte {
    if offset >= buf.len() {
        return 0;
    }
    buf[offset]
}

pub(crate) fn write_byte(buf: &mut [u8], offset: usize, val: Byte) {
    if offset >= buf.len() {
        return;
    }
    buf[offset] = val;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_roundtrip() {
        let mut buf = [0u8; 8];
        write_word(&mut buf, 0, 42);
        assert_eq!(read_word(&buf, 0), 42);
        write_word(&mut buf, 4, -1);
        assert_eq!(read_word(&buf, 4), -1);
    }

    #[test]
    fn big_roundtrip() {
        let mut buf = [0u8; 8];
        write_big(&mut buf, 0, 0x0102030405060708);
        assert_eq!(read_big(&buf, 0), 0x0102030405060708);
    }

    #[test]
    fn real_roundtrip() {
        let mut buf = [0u8; 8];
        write_real(&mut buf, 0, 3.14);
        assert!((read_real(&buf, 0) - 3.14).abs() < f64::EPSILON);
    }

    #[test]
    fn byte_roundtrip() {
        let mut buf = [0u8; 4];
        write_byte(&mut buf, 2, 0xAB);
        assert_eq!(read_byte(&buf, 2), 0xAB);
    }

    #[test]
    fn out_of_bounds_returns_zero() {
        let buf = [0u8; 2];
        assert_eq!(read_word(&buf, 0), 0);
        assert_eq!(read_big(&buf, 0), 0);
        assert_eq!(read_byte(&buf, 5), 0);
    }
}
