//! Rice VM Binary Loader
//!
//! Parses `.dis` binary module files into the [`Module`] representation
//! defined in `ricevm-core`.

mod decode;
mod reader;

use reader::Reader;
use ricevm_core::{LoadError, Module};

/// Parse a Dis module from its binary representation.
///
/// The input is the complete contents of a `.dis` file.
/// Returns a fully decoded [`Module`] on success.
pub fn load(data: &[u8]) -> Result<Module, LoadError> {
    if data.is_empty() {
        return Err(LoadError::UnexpectedEof { section: "header" });
    }
    let mut r = Reader::new(data);
    decode::parse_module(&mut r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_returns_error() {
        let result = load(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn invalid_magic_returns_error() {
        // 0x01 is a single-byte operand (value=1), which is not a valid magic number
        let result = load(&[0x01]);
        assert!(matches!(result, Err(LoadError::InvalidMagic(1))));
    }
}
