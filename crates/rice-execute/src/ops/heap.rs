use ricevm_core::ExecError;

use crate::heap::HeapData;
use crate::vm::VmState;

/// new src, dst:allocate a record of the type given by src (type index)
pub(crate) fn op_new(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let type_idx = vm.src_word()? as usize;
    let size = vm
        .current_type_size(type_idx)
        .ok_or_else(|| ExecError::Other(format!("invalid type index: {type_idx}")))?;
    let id = vm
        .heap
        .alloc(type_idx as u32, HeapData::Record(vec![0; size]));
    vm.move_ptr_to_dst(id)
}

/// newz src, dst:same as new but data is guaranteed zero-initialized (which it already is)
pub(crate) fn op_newz(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    op_new(vm)
}

/// newa src, mid, dst:allocate an array of length src, element type mid
pub(crate) fn op_newa(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let length = vm.src_word()? as usize;
    let elem_type_idx = vm.mid_word()? as usize;
    let elem_size = vm
        .current_type_size(elem_type_idx)
        .ok_or_else(|| ExecError::Other(format!("invalid element type index: {elem_type_idx}")))?;
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

/// newaz:same as newa (zero-initialized)
pub(crate) fn op_newaz(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    op_newa(vm)
}

/// mnewz src, mid, dst: allocate and zero a record (same as newz for us)
pub(crate) fn op_mnewz(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    op_new(vm)
}

fn alloc_channel(vm: &mut VmState<'_>, elem_size: usize) -> Result<(), ExecError> {
    let id = vm.heap.alloc(
        0,
        HeapData::Channel {
            elem_size,
            pending: None,
        },
    );
    vm.move_ptr_to_dst(id)
}

pub(crate) fn op_newcb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    alloc_channel(vm, 1)
}
pub(crate) fn op_newcw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    alloc_channel(vm, 4)
}
pub(crate) fn op_newcf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    alloc_channel(vm, 8)
}
pub(crate) fn op_newcp(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    alloc_channel(vm, 4)
}
pub(crate) fn op_newcm(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let elem_size = vm.current_type_size(vm.src_word()? as usize).unwrap_or(4);
    alloc_channel(vm, elem_size)
}
pub(crate) fn op_newcmp(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let elem_size = vm.current_type_size(vm.src_word()? as usize).unwrap_or(4);
    alloc_channel(vm, elem_size)
}
pub(crate) fn op_newcl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    alloc_channel(vm, 8)
}
