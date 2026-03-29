//! Rice VM Binary Loader
//!
//! Parses `.dis` binary module files into the [`Module`] representation
//! defined in `ricevm-core`.

use ricevm_core::{LoadError, Module};

/// Parse a Dis module from its binary representation.
///
/// The input is the complete contents of a `.dis` file.
/// Returns a fully decoded [`Module`] on success.
pub fn load(data: &[u8]) -> Result<Module, LoadError> {
    if data.is_empty() {
        return Err(LoadError::UnexpectedEof { section: "header" });
    }
    tracing::info!("Loading program of size: {} bytes", data.len());
    // TODO: implement binary format parsing
    Err(LoadError::Other("loader not yet implemented".to_string()))
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
    fn stub_returns_not_implemented() {
        let data = [0u8; 4];
        let result = load(&data);
        assert!(result.is_err());
    }
}
