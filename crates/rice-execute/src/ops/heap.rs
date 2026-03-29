use ricevm_core::ExecError;

use crate::heap::HeapData;
use crate::vm::VmState;

/// new src, dst — allocate a record of the type given by src (type index)
pub(crate) fn op_new(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let type_idx = vm.src_word()? as usize;
    let size = if type_idx < vm.module.types.len() {
        vm.module.types[type_idx].size as usize
    } else {
        return Err(ExecError::Other(format!("invalid type index: {type_idx}")));
    };
    let id = vm
        .heap
        .alloc(type_idx as u32, HeapData::Record(vec![0; size]));
    vm.move_ptr_to_dst(id)
}

/// newz src, dst — same as new but data is guaranteed zero-initialized (which it already is)
pub(crate) fn op_newz(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    op_new(vm)
}

/// newa src, mid, dst — allocate an array of length src, element type mid
pub(crate) fn op_newa(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let length = vm.src_word()? as usize;
    let elem_type_idx = vm.mid_word()? as usize;
    let elem_size = if elem_type_idx < vm.module.types.len() {
        vm.module.types[elem_type_idx].size as usize
    } else {
        return Err(ExecError::Other(format!(
            "invalid element type index: {elem_type_idx}"
        )));
    };
    let data = vec![0u8; length * elem_size];
    let id = vm.heap.alloc(
        elem_type_idx as u32,
        HeapData::Array {
            elem_type: elem_type_idx as u32,
            elem_size,
            data,
            length,
        },
    );
    vm.move_ptr_to_dst(id)
}

/// newaz — same as newa (zero-initialized)
pub(crate) fn op_newaz(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    op_newa(vm)
}

/// mnewz src, mid, dst — allocate and zero a record (same as newz for us)
pub(crate) fn op_mnewz(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    op_new(vm)
}

// Channel allocation stubs — allocate a Channel sentinel on the heap.
// Actual channel operations (send/recv) are not yet implemented.

fn alloc_channel(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let id = vm.heap.alloc(0, HeapData::Channel);
    vm.move_ptr_to_dst(id)
}

pub(crate) fn op_newcb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    alloc_channel(vm)
}
pub(crate) fn op_newcw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    alloc_channel(vm)
}
pub(crate) fn op_newcf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    alloc_channel(vm)
}
pub(crate) fn op_newcp(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    alloc_channel(vm)
}
pub(crate) fn op_newcm(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    alloc_channel(vm)
}
pub(crate) fn op_newcmp(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    alloc_channel(vm)
}
pub(crate) fn op_newcl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    alloc_channel(vm)
}
