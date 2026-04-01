//! Operand address resolution.
//!
//! Resolves instruction operands into `AddrTarget` values that identify
//! which memory buffer and byte offset a value lives at.

use ricevm_core::{AddressMode, ExecError, MiddleMode, MiddleOperand, Operand};

use crate::memory;

use crate::heap::HeapId;

/// Sentinel bit indicating a heap array element reference (from `indx`).
pub(crate) const HEAP_REF_FLAG: i32 = i32::MIN; // 0x80000000

/// Base address for module MP virtual address ranges.
/// Each module's MP occupies a unique range: MP_BASE + module_index * MP_STRIDE.
/// Module index 0 = main module, 1+ = loaded modules (index + 1).
pub(crate) const MP_BASE: usize = 0x0080_0000; // 8MB
/// Stride between module MP address ranges (1MB per module).
pub(crate) const MP_STRIDE: usize = 0x0010_0000; // 1MB

use crate::heap::Heap;

/// Resolved location of an operand value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum AddrTarget {
    /// Absolute byte offset into the frame stack data buffer.
    Frame(usize),
    /// Byte offset into the current module's MP (module data) buffer.
    Mp(usize),
    /// Byte offset into a specific module's MP, identified by module index.
    /// module_idx 0 = main module, 1+ = loaded module (index + 1).
    ModuleMp { module_idx: usize, offset: usize },
    /// An immediate value stored in a scratch slot on VmState.
    Immediate,
    /// No operand (unused slot).
    None,
    /// A reference into a heap array object's data buffer.
    HeapArray { id: HeapId, offset: usize },
}

/// Decode a virtual address (from Lea) back to an AddrTarget.
/// Addresses in [MP_BASE, HEAP_ID_BASE) are module MP references.
/// Addresses below MP_BASE are frame offsets.
pub(crate) fn decode_virtual_addr(addr: i32, register2: usize) -> AddrTarget {
    if addr == 0 {
        return AddrTarget::None;
    }
    if addr & HEAP_REF_FLAG != 0 {
        // Heap array reference (from indx): can't decode without heap_refs
        // This case is handled separately in resolve_operand
        return AddrTarget::None;
    }
    let uaddr = addr as usize;
    if uaddr >= crate::heap::HEAP_ID_BASE as usize {
        return AddrTarget::HeapArray {
            id: uaddr as HeapId,
            offset: register2,
        };
    }
    if uaddr >= MP_BASE {
        let rel = uaddr - MP_BASE;
        let module_idx = rel / MP_STRIDE;
        let mp_off = rel % MP_STRIDE;
        return AddrTarget::ModuleMp {
            module_idx,
            offset: mp_off + register2,
        };
    }
    AddrTarget::Frame(uaddr + register2)
}

/// Resolve a source or destination operand.
///
/// `fp_base` is the absolute byte offset of the current frame's data area in the stack.
/// `stack_data` is needed for double-indirect modes to dereference the first indirection.
/// Resolve an operand without heap awareness (backwards compatibility).
pub(crate) fn resolve_operand(
    op: &Operand,
    fp_base: usize,
    stack_data: &[u8],
    mp_data: &[u8],
    heap_refs: &[(HeapId, usize)],
) -> Result<AddrTarget, ExecError> {
    resolve_operand_with_heap(op, fp_base, stack_data, mp_data, heap_refs, None)
}

/// Resolve an operand to a target address.
///
/// When `heap` is provided, double-indirect addressing checks if the intermediate
/// pointer is a valid HeapId and resolves to `HeapArray` if so.
pub(crate) fn resolve_operand_with_heap(
    op: &Operand,
    fp_base: usize,
    stack_data: &[u8],
    mp_data: &[u8],
    heap_refs: &[(HeapId, usize)],
    _heap: Option<&Heap>,
) -> Result<AddrTarget, ExecError> {
    match op.mode {
        AddressMode::OffsetIndirectFp => Ok(AddrTarget::Frame(fp_base + op.register1 as usize)),
        AddressMode::OffsetIndirectMp => Ok(AddrTarget::Mp(op.register1 as usize)),
        AddressMode::Immediate => Ok(AddrTarget::Immediate),
        AddressMode::None => Ok(AddrTarget::None),
        AddressMode::OffsetDoubleIndirectFp => {
            let base_addr = fp_base + op.register1 as usize;
            let base_val = memory::read_word(stack_data, base_addr);
            // Nil pointer dereference: treat as no-op target.
            if base_val == 0 {
                return Ok(AddrTarget::None);
            }
            // Check if the base value is a heap array reference (from indx)
            if base_val & HEAP_REF_FLAG != 0 {
                let ref_idx = (base_val & !HEAP_REF_FLAG) as usize;
                if let Some(&(id, byte_offset)) = heap_refs.get(ref_idx) {
                    return Ok(AddrTarget::HeapArray {
                        id,
                        offset: byte_offset + op.register2 as usize,
                    });
                }
            }
            // Decode the virtual address (handles MP ranges, HeapIds, and frame offsets)
            Ok(decode_virtual_addr(base_val, op.register2 as usize))
        }
        AddressMode::OffsetDoubleIndirectMp => {
            let base_val = memory::read_word(mp_data, op.register1 as usize);
            // Nil pointer dereference: treat as no-op target.
            if base_val == 0 {
                return Ok(AddrTarget::None);
            }
            if base_val & HEAP_REF_FLAG != 0 {
                let ref_idx = (base_val & !HEAP_REF_FLAG) as usize;
                if let Some(&(id, byte_offset)) = heap_refs.get(ref_idx) {
                    return Ok(AddrTarget::HeapArray {
                        id,
                        offset: byte_offset + op.register2 as usize,
                    });
                }
            }
            // Decode the virtual address
            let decoded = decode_virtual_addr(base_val, op.register2 as usize);
            // If it decoded to Frame, it was actually an MP-relative offset
            // (since the base came from MP, not frame)
            if let AddrTarget::Frame(off) = decoded {
                Ok(AddrTarget::Mp(off))
            } else {
                Ok(decoded)
            }
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
        MiddleMode::SmallOffsetFp => Ok(AddrTarget::Frame(fp_base + op.register1 as usize)),
        MiddleMode::SmallOffsetMp => Ok(AddrTarget::Mp(op.register1 as usize)),
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
        let target = resolve_operand(&op, 16, &[], &[], &[]).unwrap();
        assert!(matches!(target, AddrTarget::Frame(24)));
    }

    #[test]
    fn resolve_mp_indirect() {
        let op = Operand {
            mode: AddressMode::OffsetIndirectMp,
            register1: 4,
            register2: 0,
        };
        let target = resolve_operand(&op, 0, &[], &[], &[]).unwrap();
        assert!(matches!(target, AddrTarget::Mp(4)));
    }

    #[test]
    fn resolve_immediate() {
        let op = Operand {
            mode: AddressMode::Immediate,
            register1: 42,
            register2: 0,
        };
        let target = resolve_operand(&op, 0, &[], &[], &[]).unwrap();
        assert!(matches!(target, AddrTarget::Immediate));
    }

    #[test]
    fn resolve_none() {
        let target = resolve_operand(&Operand::UNUSED, 0, &[], &[], &[]).unwrap();
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
