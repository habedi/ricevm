//! Operand address resolution.
//!
//! Resolves instruction operands into `AddrTarget` values that identify
//! which memory buffer and byte offset a value lives at.

use ricevm_core::{AddressMode, ExecError, MiddleMode, MiddleOperand, Operand};

use crate::memory;

/// Resolved location of an operand value.
#[derive(Debug, Clone, Copy)]
pub(crate) enum AddrTarget {
    /// Absolute byte offset into the frame stack data buffer.
    Frame(usize),
    /// Byte offset into the MP (module data) buffer.
    Mp(usize),
    /// An immediate value stored in a scratch slot on VmState.
    Immediate,
    /// No operand (unused slot).
    None,
}

/// Resolve a source or destination operand.
///
/// `fp_base` is the absolute byte offset of the current frame's data area in the stack.
/// `stack_data` is needed for double-indirect modes to dereference the first indirection.
pub(crate) fn resolve_operand(
    op: &Operand,
    fp_base: usize,
    stack_data: &[u8],
    mp_data: &[u8],
) -> Result<AddrTarget, ExecError> {
    match op.mode {
        AddressMode::OffsetIndirectFp => {
            Ok(AddrTarget::Frame(fp_base + op.register1 as usize))
        }
        AddressMode::OffsetIndirectMp => {
            Ok(AddrTarget::Mp(op.register1 as usize))
        }
        AddressMode::Immediate => {
            Ok(AddrTarget::Immediate)
        }
        AddressMode::None => {
            Ok(AddrTarget::None)
        }
        AddressMode::OffsetDoubleIndirectFp => {
            let base_addr = fp_base + op.register1 as usize;
            let indirect = memory::read_word(stack_data, base_addr) as usize;
            Ok(AddrTarget::Frame(indirect + op.register2 as usize))
        }
        AddressMode::OffsetDoubleIndirectMp => {
            let indirect = memory::read_word(mp_data, op.register1 as usize) as usize;
            Ok(AddrTarget::Mp(indirect + op.register2 as usize))
        }
        AddressMode::Reserved1 | AddressMode::Reserved2 => {
            Err(ExecError::Other("reserved address mode".to_string()))
        }
    }
}

/// Resolve a middle operand.
///
/// `fp_base` is the absolute byte offset of the current frame's data area.
pub(crate) fn resolve_middle(op: &MiddleOperand, fp_base: usize) -> Result<AddrTarget, ExecError> {
    match op.mode {
        MiddleMode::None => Ok(AddrTarget::None),
        MiddleMode::SmallImmediate => Ok(AddrTarget::Immediate),
        MiddleMode::SmallOffsetFp => {
            Ok(AddrTarget::Frame(fp_base + op.register1 as usize))
        }
        MiddleMode::SmallOffsetMp => {
            Ok(AddrTarget::Mp(op.register1 as usize))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_fp_indirect() {
        let op = Operand {
            mode: AddressMode::OffsetIndirectFp,
            register1: 8,
            register2: 0,
        };
        let target = resolve_operand(&op, 16, &[], &[]).unwrap();
        assert!(matches!(target, AddrTarget::Frame(24)));
    }

    #[test]
    fn resolve_mp_indirect() {
        let op = Operand {
            mode: AddressMode::OffsetIndirectMp,
            register1: 4,
            register2: 0,
        };
        let target = resolve_operand(&op, 0, &[], &[]).unwrap();
        assert!(matches!(target, AddrTarget::Mp(4)));
    }

    #[test]
    fn resolve_immediate() {
        let op = Operand {
            mode: AddressMode::Immediate,
            register1: 42,
            register2: 0,
        };
        let target = resolve_operand(&op, 0, &[], &[]).unwrap();
        assert!(matches!(target, AddrTarget::Immediate));
    }

    #[test]
    fn resolve_none() {
        let target = resolve_operand(&Operand::UNUSED, 0, &[], &[]).unwrap();
        assert!(matches!(target, AddrTarget::None));
    }

    #[test]
    fn resolve_middle_small_fp() {
        let op = MiddleOperand {
            mode: MiddleMode::SmallOffsetFp,
            register1: 4,
        };
        let target = resolve_middle(&op, 16).unwrap();
        assert!(matches!(target, AddrTarget::Frame(20)));
    }
}
