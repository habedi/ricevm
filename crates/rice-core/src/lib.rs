//! Core types and definitions for Rice VM.
//!
//! This crate defines the shared data structures used across `ricevm-loader`
//! and `ricevm-execute`. It contains no runtime logic.

pub mod error;
pub mod instruction;
pub mod module;
pub mod opcode;
pub mod types;

pub use error::{ExecError, LoadError};
pub use instruction::{AddressMode, Instruction, MiddleMode, MiddleOperand, Operand};
pub use module::{
    DataItem, ExceptionCase, ExportEntry, Handler, Header, ImportEntry, ImportModule, Module,
    RuntimeFlags, SMAGIC, XMAGIC,
};
pub use opcode::Opcode;
pub use types::{Big, Byte, Pc, PointerMap, Real, TypeDescriptor, Word};
