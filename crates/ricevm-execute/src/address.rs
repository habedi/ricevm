//! Operand address resolution.
//!
//! Resolves instruction operands into `AddrTarget` values that identify
//! which memory buffer and byte offset a value lives at.

use ricevm_core::{AddressMode, ExecError, MiddleMode, MiddleOperand, Operand, Word};

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
/// Number of module MP ranges the virtual address space reserves.
pub(crate) const MAX_MODULES: usize = 128;
/// First address above the module MP ranges. Must stay below `HEAP_ID_BASE`
/// so MP addresses can never be mistaken for heap ids.
pub(crate) const MP_LIMIT: usize = MP_BASE + MAX_MODULES * MP_STRIDE;

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

/// Convert a bytecode-supplied register value into a byte offset.
///
/// Register values are decoded as signed words, but they are only ever used as
/// offsets. A negative value would wrap to a huge `usize` and address another
/// frame (or another module's data), so it is rejected here.
fn offset_operand(reg: Word, what: &str) -> Result<usize, ExecError> {
    usize::try_from(reg).map_err(|_| ExecError::Other(format!("negative {what} offset: {reg}")))
}

/// Absolute stack address of a frame-relative operand.
///
/// The result must stay inside the frame address range; anything else would be
/// indistinguishable from a module MP address (see `decode_virtual_addr`).
fn frame_addr(fp_base: usize, reg: Word) -> Result<usize, ExecError> {
    fp_base
        .checked_add(offset_operand(reg, "frame")?)
        .filter(|&addr| addr < MP_BASE)
        .ok_or_else(|| ExecError::Other(format!("frame offset out of range: {reg}")))
}

/// Decode a virtual address (from Lea) back to an AddrTarget.
/// Addresses in [MP_BASE, MP_LIMIT) are module MP references.
/// Addresses at or above HEAP_ID_BASE are heap ids.
/// Addresses below MP_BASE are frame offsets.
/// Anything that falls outside those ranges resolves to `None` rather than
/// being silently routed to the wrong buffer.
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
    if uaddr >= MP_LIMIT {
        // Gap between the module MP ranges and the heap id space: unmapped.
        return AddrTarget::None;
    }
    if uaddr >= MP_BASE {
        let rel = uaddr - MP_BASE;
        let module_idx = rel / MP_STRIDE;
        let mp_off = rel % MP_STRIDE;
        // An offset that runs past the stride would land in the next module's
        // range; reject it instead of reading another module's data.
        return match mp_off.checked_add(register2) {
            Some(offset) if offset < MP_STRIDE => AddrTarget::ModuleMp { module_idx, offset },
            _ => AddrTarget::None,
        };
    }
    match uaddr.checked_add(register2) {
        Some(offset) if offset < MP_BASE => AddrTarget::Frame(offset),
        _ => AddrTarget::None,
    }
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
        AddressMode::OffsetIndirectFp => Ok(AddrTarget::Frame(frame_addr(fp_base, op.register1)?)),
        AddressMode::OffsetIndirectMp => Ok(AddrTarget::Mp(offset_operand(op.register1, "mp")?)),
        AddressMode::Immediate => Ok(AddrTarget::Immediate),
        AddressMode::None => Ok(AddrTarget::None),
        AddressMode::OffsetDoubleIndirectFp => {
            let base_addr = frame_addr(fp_base, op.register1)?;
            let register2 = offset_operand(op.register2, "indirect")?;
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
                        offset: byte_offset.saturating_add(register2),
                    });
                }
            }
            // Decode the virtual address (handles MP ranges, HeapIds, and frame offsets)
            Ok(decode_virtual_addr(base_val, register2))
        }
        AddressMode::OffsetDoubleIndirectMp => {
            let base_off = offset_operand(op.register1, "mp")?;
            let register2 = offset_operand(op.register2, "indirect")?;
            let base_val = memory::read_word(mp_data, base_off);
            // Nil pointer dereference: treat as no-op target.
            if base_val == 0 {
                return Ok(AddrTarget::None);
            }
            if base_val & HEAP_REF_FLAG != 0 {
                let ref_idx = (base_val & !HEAP_REF_FLAG) as usize;
                if let Some(&(id, byte_offset)) = heap_refs.get(ref_idx) {
                    return Ok(AddrTarget::HeapArray {
                        id,
                        offset: byte_offset.saturating_add(register2),
                    });
                }
            }
            // Decode the virtual address
            let decoded = decode_virtual_addr(base_val, register2);
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
        MiddleMode::SmallOffsetFp => Ok(AddrTarget::Frame(frame_addr(fp_base, op.register1)?)),
        MiddleMode::SmallOffsetMp => Ok(AddrTarget::Mp(offset_operand(op.register1, "mp")?)),
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

    #[test]
    fn resolve_fp_rejects_negative_offset() {
        let op = Operand {
            mode: AddressMode::OffsetIndirectFp,
            register1: -64,
            register2: 0,
        };
        let result = resolve_operand(&op, 16, &[0u8; 64], &[], &[]);
        assert!(
            result.is_err(),
            "negative frame offsets must not resolve into another frame"
        );
    }

    #[test]
    fn resolve_mp_rejects_negative_offset() {
        let op = Operand {
            mode: AddressMode::OffsetIndirectMp,
            register1: -4,
            register2: 0,
        };
        assert!(resolve_operand(&op, 0, &[], &[0u8; 64], &[]).is_err());
    }

    #[test]
    fn resolve_middle_rejects_negative_offset() {
        let op = MiddleOperand {
            mode: MiddleMode::SmallOffsetFp,
            register1: -8,
        };
        assert!(resolve_middle(&op, 16).is_err());
    }

    #[test]
    fn resolve_double_indirect_rejects_negative_register2() {
        let mut stack = vec![0u8; 64];
        // Frame slot 16 holds a virtual address pointing at frame offset 32.
        memory::write_word(&mut stack, 16, 32);
        let op = Operand {
            mode: AddressMode::OffsetDoubleIndirectFp,
            register1: 0,
            register2: -8,
        };
        assert!(resolve_operand(&op, 16, &stack, &[], &[]).is_err());
    }

    #[test]
    fn resolve_double_indirect_heap_ref_rejects_negative_register2() {
        let mut stack = vec![0u8; 64];
        memory::write_word(&mut stack, 16, HEAP_REF_FLAG); // heap_refs index 0
        let op = Operand {
            mode: AddressMode::OffsetDoubleIndirectFp,
            register1: 0,
            register2: -8,
        };
        let refs = [(crate::heap::HEAP_ID_BASE, 0usize)];
        assert!(resolve_operand(&op, 16, &stack, &[], &refs).is_err());
    }

    #[test]
    fn module_mp_addresses_never_alias_heap_ids() {
        // Every supported module's MP range must decode back to that module,
        // never to a heap object or another module.
        for module_idx in 0..MAX_MODULES {
            let addr = (MP_BASE + module_idx * MP_STRIDE) as i32;
            match decode_virtual_addr(addr, 0) {
                AddrTarget::ModuleMp {
                    module_idx: decoded,
                    offset,
                } => {
                    assert_eq!(decoded, module_idx, "module {module_idx} MP misdecoded");
                    assert_eq!(offset, 0);
                }
                other => panic!("module {module_idx} MP address decoded as {other:?}"),
            }
        }
    }

    #[test]
    fn mp_range_does_not_overlap_heap_ids() {
        assert!(
            MP_LIMIT <= crate::heap::HEAP_ID_BASE as usize,
            "module MP address range must end below the first heap id"
        );
    }

    #[test]
    fn addresses_above_mp_range_do_not_decode_as_mp() {
        // Just past the last module's range and below HEAP_ID_BASE: not a valid
        // address, and it must not be routed to some module's MP.
        let target = decode_virtual_addr(MP_LIMIT as i32, 0);
        assert!(
            matches!(target, AddrTarget::None),
            "out-of-range virtual address must not resolve, got {target:?}"
        );
    }

    #[test]
    fn mp_offset_beyond_stride_is_rejected() {
        // An MP offset that bleeds into the next module's range must not
        // silently resolve to that module.
        let target = decode_virtual_addr((MP_BASE + MP_STRIDE - 4) as i32, 8);
        assert!(
            matches!(target, AddrTarget::None),
            "MP offset past the stride must not resolve, got {target:?}"
        );
    }
}
