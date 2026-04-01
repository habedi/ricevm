use ricevm_core::LoadError;

/// A cursor over a byte slice for parsing `.dis` binary modules.
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Read a single byte, or return `UnexpectedEof` with the given section name.
    pub fn read_byte(&mut self, section: &'static str) -> Result<u8, LoadError> {
        if self.pos >= self.data.len() {
            return Err(LoadError::UnexpectedEof { section });
        }
        let b = self.data[self.pos];
        self.pos += 1;
        Ok(b)
    }

    /// Read `n` bytes as a borrowed slice (zero-copy).
    pub fn read_bytes(&mut self, n: usize, section: &'static str) -> Result<&'a [u8], LoadError> {
        if self.pos + n > self.data.len() {
            return Err(LoadError::UnexpectedEof { section });
        }
        let slice = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    /// Read a variable-length operand as defined in the Dis VM specification.
    ///
    /// Encoding based on the high 2 bits of the first byte:
    /// - `0x00`: 1-byte positive (0..63)
    /// - `0x40`: 1-byte negative (-64..-1)
    /// - `0x80`: 2-byte signed
    /// - `0xC0`: 4-byte signed
    pub fn read_operand(&mut self, section: &'static str) -> Result<i32, LoadError> {
        let b = self.read_byte(section)? as i32;
        match b & 0xC0 {
            0x00 => {
                // 1-byte positive: value is the byte itself (0..63)
                Ok(b)
            }
            0x40 => {
                // 1-byte negative: sign-extend from bit 7
                Ok(b | !0x7F)
            }
            0x80 => {
                // 2-byte: sign-extend from bit 5 of first byte, then shift and combine
                let b2 = self.read_byte(section)? as i32;
                let high = if b & 0x20 != 0 { b | !0x3F } else { b & 0x3F };
                Ok((high << 8) | b2)
            }
            0xC0 => {
                // 4-byte: sign-extend from bit 5 of first byte, then shift and combine
                let b2 = self.read_byte(section)? as i32;
                let b3 = self.read_byte(section)? as i32;
                let b4 = self.read_byte(section)? as i32;
                let high = if b & 0x20 != 0 { b | !0x3F } else { b & 0x3F };
                Ok((high << 24) | (b2 << 16) | (b3 << 8) | b4)
            }
            _ => unreachable!(),
        }
    }

    /// Read a fixed 4-byte big-endian word (used for signatures, array data, etc.).
    pub fn read_word_be(&mut self, section: &'static str) -> Result<i32, LoadError> {
        let bytes = self.read_bytes(4, section)?;
        Ok(((bytes[0] as i32) << 24)
            | ((bytes[1] as i32) << 16)
            | ((bytes[2] as i32) << 8)
            | (bytes[3] as i32))
    }

    /// Read a null-terminated UTF-8 string.
    pub fn read_cstring(&mut self, section: &'static str) -> Result<String, LoadError> {
        let start = self.pos;
        loop {
            if self.pos >= self.data.len() {
                return Err(LoadError::UnexpectedEof { section });
            }
            if self.data[self.pos] == 0 {
                let slice = &self.data[start..self.pos];
                self.pos += 1; // skip the null terminator
                return Ok(String::from_utf8_lossy(slice).into_owned());
            }
            self.pos += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Operand encoding tests ---

    #[test]
    fn operand_1byte_zero() {
        let mut r = Reader::new(&[0x00]);
        assert_eq!(r.read_operand("test").unwrap(), 0);
    }

    #[test]
    fn operand_1byte_max_positive() {
        let mut r = Reader::new(&[0x3F]);
        assert_eq!(r.read_operand("test").unwrap(), 63);
    }

    #[test]
    fn operand_1byte_neg_one() {
        // 0x7F: bits[7:6]=01, always sign-extend -> -1
        let mut r = Reader::new(&[0x7F]);
        assert_eq!(r.read_operand("test").unwrap(), -1);
    }

    #[test]
    fn operand_1byte_neg_64() {
        // 0x40: bits[7:6]=01, result = 0x40 | !0x7F = 0x40 | 0xFFFFFF80 = 0xFFFFFFC0 = -64
        let mut r = Reader::new(&[0x40]);
        assert_eq!(r.read_operand("test").unwrap(), -64);
    }

    #[test]
    fn operand_2byte_positive() {
        // 0x80, 0x80: bits[7:6]=10, bit5=0, high = 0x80 & 0x3F = 0x00, value = (0 << 8) | 0x80 = 128
        let mut r = Reader::new(&[0x80, 0x80]);
        assert_eq!(r.read_operand("test").unwrap(), 128);
    }

    #[test]
    fn operand_2byte_negative() {
        // 0xBF, 0xFF: bits[7:6]=10, bit5=1, high = 0xBF | !0x3F = 0xFFFFFFBF
        // value = (0xFFFFFFBF << 8) | 0xFF = 0xFFFFBFFF | 0xFF = 0xFFFFBFFF
        // Wait, let me recalculate:
        // high = 0xBF_i32 | !0x3F_i32 = 0xBF | 0xFFFFFFC0 = 0xFFFFFFBF (which is -65 as i32)
        // value = (-65 << 8) | 0xFF = -16640 | 255 = -16385
        // Hmm, that's not -1. Let me figure out what gives -1 for 2-byte.
        // For -1: we need (high << 8) | b2 = -1 = 0xFFFFFFFF
        // high << 8 = 0xFFFFFF00, so b2 must be 0xFF -> high = 0xFFFFFFFF >> 8... no.
        // Actually high << 8 needs to be 0xFFFFFF00, so high = -1 = 0xFFFFFFFF
        // high = byte | !0x3F = byte | 0xFFFFFFC0. For this to be 0xFFFFFFFF:
        // byte | 0xFFFFFFC0 = 0xFFFFFFFF -> byte must have bits 0-5 all set = 0xBF (10_111111)
        // So 0xBF, 0xFF -> -1
        let mut r = Reader::new(&[0xBF, 0xFF]);
        assert_eq!(r.read_operand("test").unwrap(), -1);
    }

    #[test]
    fn operand_4byte_xmagic() {
        // XMAGIC = 0x0C8030
        // For 4-byte: high = byte & 0x3F (since bit5=0 for 0x0C8030)
        // 0x0C8030 = (high << 24) | (b2 << 16) | (b3 << 8) | b4
        // high = 0x0C8030 >> 24 = 0x00, b2 = 0x0C, b3 = 0x80, b4 = 0x30
        // First byte = 0xC0 | high = 0xC0 | 0x00 = 0xC0
        let mut r = Reader::new(&[0xC0, 0x0C, 0x80, 0x30]);
        assert_eq!(r.read_operand("test").unwrap(), 0x0C8030);
    }

    #[test]
    fn operand_4byte_neg_one() {
        // -1 = 0xFFFFFFFF
        // high = byte | !0x3F, needs all bits set in result
        // byte = 0xFF (11_111111), high = 0xFF | 0xFFFFFFC0 = 0xFFFFFFFF
        // value = (0xFFFFFFFF << 24) | (0xFF << 16) | (0xFF << 8) | 0xFF = 0xFFFFFFFF = -1
        let mut r = Reader::new(&[0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(r.read_operand("test").unwrap(), -1);
    }

    #[test]
    fn operand_4byte_smagic() {
        // SMAGIC = 0x0E1722
        // high = 0x0E1722 >> 24 = 0x00, b2 = 0x0E, b3 = 0x17, b4 = 0x22
        // First byte = 0xC0 | 0x00 = 0xC0
        let mut r = Reader::new(&[0xC0, 0x0E, 0x17, 0x22]);
        assert_eq!(r.read_operand("test").unwrap(), 0x0E1722);
    }

    #[test]
    fn operand_eof() {
        let mut r = Reader::new(&[]);
        assert!(r.read_operand("test").is_err());
    }

    #[test]
    fn operand_2byte_truncated() {
        let mut r = Reader::new(&[0x80]); // needs 1 more byte
        assert!(r.read_operand("test").is_err());
    }

    #[test]
    fn operand_4byte_truncated() {
        let mut r = Reader::new(&[0xC0, 0x00]); // needs 2 more bytes
        assert!(r.read_operand("test").is_err());
    }

    // --- read_word_be tests ---

    #[test]
    fn word_be_positive() {
        let mut r = Reader::new(&[0x00, 0x00, 0x01, 0x00]);
        assert_eq!(r.read_word_be("test").unwrap(), 256);
    }

    #[test]
    fn word_be_negative() {
        let mut r = Reader::new(&[0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(r.read_word_be("test").unwrap(), -1);
    }

    // --- read_cstring tests ---

    #[test]
    fn cstring_basic() {
        let mut r = Reader::new(b"hello\0rest");
        assert_eq!(r.read_cstring("test").unwrap(), "hello");
        // cursor should be past the null
        assert_eq!(r.read_byte("test").unwrap(), b'r');
    }

    #[test]
    fn cstring_empty() {
        let mut r = Reader::new(b"\0");
        assert_eq!(r.read_cstring("test").unwrap(), "");
    }

    #[test]
    fn cstring_eof() {
        let mut r = Reader::new(b"no null");
        assert!(r.read_cstring("test").is_err());
    }
}
