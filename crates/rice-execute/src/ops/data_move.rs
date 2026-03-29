use ricevm_core::ExecError;

use crate::address::AddrTarget;
use crate::vm::VmState;

pub(crate) fn op_movw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_word()?;
    vm.set_dst_word(val)
}

pub(crate) fn op_movb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_byte()?;
    vm.set_dst_byte(val)
}

pub(crate) fn op_movf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_real()?;
    vm.set_dst_real(val)
}

pub(crate) fn op_movl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_big()?;
    vm.set_dst_big(val)
}

/// movm src, mid, dst — copy a block of `mid` bytes from src to dst
pub(crate) fn op_movm(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let size = vm.mid_word()? as usize;
    if size == 0 {
        return Ok(());
    }

    // Read bytes from source
    let src_bytes = match vm.src {
        AddrTarget::Frame(off) => vm.frames.data[off..off + size].to_vec(),
        AddrTarget::Mp(off) => vm.mp[off..off + size].to_vec(),
        _ => vec![0u8; size],
    };

    // Write bytes to destination
    match vm.dst {
        AddrTarget::Frame(off) => {
            vm.frames.data[off..off + size].copy_from_slice(&src_bytes);
        }
        AddrTarget::Mp(off) => {
            vm.mp[off..off + size].copy_from_slice(&src_bytes);
        }
        _ => {}
    }
    Ok(())
}

/// movmp src, mid, dst — copy a block with pointer tracking (same as movm for now)
pub(crate) fn op_movmp(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    op_movm(vm)
}

/// movpc src, dst — move program counter (word) to dst
pub(crate) fn op_movpc(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_word()?;
    vm.set_dst_word(val)
}

/// tcmp src, dst — type compare (stub: always returns 0 = equal)
pub(crate) fn op_tcmp(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // In the full VM, this compares type descriptors of two allocations.
    // For now, stub it to set dst to 0 (types match).
    let _src = vm.src_ptr()?;
    vm.set_dst_word(0)
}

/// self dst — store the current module pointer into dst
pub(crate) fn op_self_(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // In a full VM, this stores the current module reference.
    // For now, store 0 (nil) since we don't have multi-module support yet.
    vm.set_dst_word(0)
}
