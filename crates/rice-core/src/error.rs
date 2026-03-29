use thiserror::Error;

/// Errors that occur while loading a `.dis` module.
#[derive(Debug, Error)]
pub enum LoadError {
    #[error("invalid magic number: {0:#x}")]
    InvalidMagic(i32),

    #[error("unexpected end of input while reading {section}")]
    UnexpectedEof { section: &'static str },

    #[error("invalid opcode byte: {0:#x}")]
    InvalidOpcode(u8),

    #[error("obsolete module format (deprecated import flag)")]
    ObsoleteModule,

    #[error("{0}")]
    Other(String),
}

/// Errors that occur during execution of a Dis module.
#[derive(Debug, Error)]
pub enum ExecError {
    #[error("no entry point in module")]
    NoEntryPoint,

    #[error("invalid program counter: {0}")]
    InvalidPc(i32),

    #[error("thread exited with error: {0}")]
    ThreadFault(String),

    #[error("{0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_error_display() {
        let err = LoadError::InvalidMagic(0xDEAD);
        assert!(err.to_string().contains("0xdead"));
    }

    #[test]
    fn exec_error_display() {
        let err = ExecError::NoEntryPoint;
        assert!(err.to_string().contains("no entry point"));
    }
}
