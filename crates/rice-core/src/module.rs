use crate::instruction::Instruction;
use crate::types::{Pc, TypeDescriptor, Word};

/// Magic number for unsigned .dis modules.
pub const XMAGIC: Word = 0x0C8030;

/// Magic number for signed .dis modules.
pub const SMAGIC: Word = 0x0E1722;

/// Runtime flags from the module header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeFlags(pub u32);

impl RuntimeFlags {
    pub const MUST_COMPILE: Self = Self(1 << 0);
    pub const DONT_COMPILE: Self = Self(1 << 1);
    pub const SHARE_MODULE: Self = Self(1 << 2);
    pub const HAS_IMPORT_DEPRECATED: Self = Self(1 << 4);
    pub const HAS_HANDLER: Self = Self(1 << 5);
    pub const HAS_IMPORT: Self = Self(1 << 6);

    pub fn contains(self, flag: Self) -> bool {
        (self.0 & flag.0) != 0
    }
}

/// Module header as read from the .dis binary.
#[derive(Debug, Clone)]
pub struct Header {
    pub magic: Word,
    pub signature: Vec<u8>,
    pub runtime_flags: RuntimeFlags,
    pub stack_extent: Word,
    pub code_size: Word,
    pub data_size: Word,
    pub type_size: Word,
    pub export_size: Word,
    pub entry_pc: Pc,
    pub entry_type: Word,
}

/// An exported function entry.
#[derive(Debug, Clone)]
pub struct ExportEntry {
    pub pc: Pc,
    pub frame_type: Word,
    pub signature: Word,
    pub name: String,
}

/// An imported function entry.
#[derive(Debug, Clone)]
pub struct ImportEntry {
    pub signature: Word,
    pub name: String,
}

/// A group of imports from a single external module.
#[derive(Debug, Clone)]
pub struct ImportModule {
    pub functions: Vec<ImportEntry>,
}

/// A single exception case inside a handler.
#[derive(Debug, Clone)]
pub struct ExceptionCase {
    /// Exception name, or `None` for the wildcard case.
    pub name: Option<String>,
    pub pc: Pc,
}

/// An exception handler covering a range of program counters.
#[derive(Debug, Clone)]
pub struct Handler {
    pub exception_offset: Word,
    pub begin_pc: Pc,
    pub end_pc: Pc,
    /// Index into the module's type section, if present.
    pub type_descriptor: Option<u32>,
    pub cases: Vec<ExceptionCase>,
}

/// Initial data item for the module's data section.
///
/// The loader produces these from the binary format. The executor
/// interprets them to initialize the module pointer (MP) before execution.
#[derive(Debug, Clone)]
pub enum DataItem {
    Bytes { offset: Word, values: Vec<u8> },
    Words { offset: Word, values: Vec<Word> },
    Bigs { offset: Word, values: Vec<i64> },
    Reals { offset: Word, values: Vec<f64> },
    String { offset: Word, value: String },
    Array { offset: Word, element_type: Word, length: Word },
    SetArray { offset: Word, index: Word },
    RestoreBase,
}

/// A fully parsed Dis module.
///
/// This is the primary data structure that flows from the loader
/// to the executor. It contains all sections of a `.dis` file
/// in decoded form.
#[derive(Debug, Clone)]
pub struct Module {
    pub header: Header,
    pub code: Vec<Instruction>,
    pub types: Vec<TypeDescriptor>,
    pub data: Vec<DataItem>,
    pub name: String,
    pub exports: Vec<ExportEntry>,
    pub imports: Vec<ImportModule>,
    pub handlers: Vec<Handler>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_flags_contains() {
        let flags = RuntimeFlags(RuntimeFlags::HAS_HANDLER.0 | RuntimeFlags::HAS_IMPORT.0);
        assert!(flags.contains(RuntimeFlags::HAS_HANDLER));
        assert!(flags.contains(RuntimeFlags::HAS_IMPORT));
        assert!(!flags.contains(RuntimeFlags::MUST_COMPILE));
    }

    #[test]
    fn magic_constants() {
        assert_eq!(XMAGIC, 0x0C8030);
        assert_eq!(SMAGIC, 0x0E1722);
    }
}
