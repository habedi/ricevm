use ricevm_core::ExecError;

use crate::vm::VmState;

pub(crate) fn op_exit(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    vm.halted = true;
    Ok(())
}

pub(crate) fn op_jmp(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    vm.next_pc = vm.dst_word()? as usize;
    Ok(())
}

pub(crate) fn op_frame(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let type_idx = vm.src_word()? as usize;
    let frame_size = if type_idx < vm.module.types.len() {
        vm.module.types[type_idx].size as usize
    } else {
        return Err(ExecError::Other(format!(
            "invalid type descriptor index: {type_idx}"
        )));
    };
    let pending_data_offset = vm.frames.alloc_pending(frame_size)?;
    vm.set_dst_word(pending_data_offset as i32)
}

pub(crate) fn op_call(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_data_offset = vm.src_word()? as usize;
    let target_pc = vm.dst_word()? as usize;
    vm.frames
        .activate_pending(frame_data_offset, vm.next_pc as i32)?;
    vm.next_pc = target_pc;
    Ok(())
}

pub(crate) fn op_ret(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let prev_pc = vm.frames.pop()?;
    if prev_pc < 0 {
        // Sentinel: returning from entry function.
        vm.halted = true;
    } else {
        vm.next_pc = prev_pc as usize;
    }
    Ok(())
}
