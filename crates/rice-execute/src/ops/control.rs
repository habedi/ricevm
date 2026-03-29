use ricevm_core::ExecError;

use crate::heap::{self, HeapData};
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

/// load src, mid, dst — load a module by name
/// src = string pointer (module path), mid = import table index, dst = result module ref
pub(crate) fn op_load(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let path_id = vm.src_ptr()?;
    let path = vm.heap.get_string(path_id).unwrap_or("").to_string();

    // Look up built-in module
    let module_id = match vm.modules.find_builtin(&path) {
        Some(id) => id,
        None => {
            // Module not found: set dst to nil (like the C++ impl)
            vm.move_ptr_to_dst(heap::NIL)?;
            return Ok(());
        }
    };

    // Allocate a ModuleRef on the heap
    let ref_id = vm.heap.alloc(0, HeapData::ModuleRef { module_id });
    vm.move_ptr_to_dst(ref_id)
}

/// mframe src, mid, dst — create frame for module call
/// src = module ref pointer, mid = function index, dst = frame pointer (output)
pub(crate) fn op_mframe(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let mod_ref_id = vm.src_ptr()?;
    let func_idx = vm.mid_word()? as u32;

    let module_id = match vm.heap.get(mod_ref_id) {
        Some(obj) => match &obj.data {
            HeapData::ModuleRef { module_id } => *module_id,
            _ => {
                return Err(ExecError::ThreadFault(
                    "mframe: not a module ref".to_string(),
                ));
            }
        },
        None => return Err(ExecError::ThreadFault("mframe: nil module".to_string())),
    };

    let frame_size = vm
        .modules
        .get_func(module_id, func_idx)
        .map(|f| f.frame_size)
        .ok_or_else(|| {
            ExecError::Other(format!(
                "builtin function not found: module={module_id}, func={func_idx}"
            ))
        })?;

    let pending_data_offset = vm.frames.alloc_pending(frame_size)?;
    vm.set_dst_word(pending_data_offset as i32)
}

/// mcall src, mid, dst — call function in loaded module
/// src = frame pointer, mid = function index, dst = module ref pointer
pub(crate) fn op_mcall(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_data_offset = vm.src_word()? as usize;
    let func_idx = vm.mid_word()? as u32;
    let mod_ref_id = vm.dst_ptr()?;

    let module_id = match vm.heap.get(mod_ref_id) {
        Some(obj) => match &obj.data {
            HeapData::ModuleRef { module_id } => *module_id,
            _ => {
                return Err(ExecError::ThreadFault(
                    "mcall: not a module ref".to_string(),
                ));
            }
        },
        None => return Err(ExecError::ThreadFault("mcall: nil module".to_string())),
    };

    // Activate the pending frame
    vm.frames
        .activate_pending(frame_data_offset, vm.next_pc as i32)?;

    // Get and call the built-in handler
    let handler = vm
        .modules
        .get_func(module_id, func_idx)
        .map(|f| f.handler)
        .ok_or_else(|| {
            ExecError::Other(format!(
                "builtin function not found: module={module_id}, func={func_idx}"
            ))
        })?;

    handler(vm)?;

    // Auto-return from built-in call
    let prev_pc = vm.frames.pop()?;
    if prev_pc >= 0 {
        vm.next_pc = prev_pc as usize;
    }
    Ok(())
}

/// goto src, dst — computed goto: dst holds a pc table pointer, src is the index.
/// In Dis, `goto` jumps to the PC stored in a case table.
/// src = pointer to word array of PCs, dst = index.
pub(crate) fn op_goto(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // The goto instruction uses src as a pointer to an array of PCs
    // and dst as an index. We read the target PC from src[dst].
    let _src = vm.src_word()?;
    let target = vm.dst_word()?;
    vm.next_pc = target as usize;
    Ok(())
}

/// casew src, mid, dst — word case dispatch.
/// src = value to match, dst = pointer to case table.
/// Case table format: N pairs of (value, pc), then a default pc.
/// For simplicity, we just read the default target from dst.
pub(crate) fn op_casew(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // In the full VM, casew reads a case table from memory.
    // For now, treat as a computed jump to the value at dst.
    let target = vm.dst_word()?;
    vm.next_pc = target as usize;
    Ok(())
}

/// casec — string case dispatch (same stub as casew)
pub(crate) fn op_casec(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let target = vm.dst_word()?;
    vm.next_pc = target as usize;
    Ok(())
}

/// casel — big case dispatch (same stub as casew)
pub(crate) fn op_casel(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let target = vm.dst_word()?;
    vm.next_pc = target as usize;
    Ok(())
}

/// raise src — raise an exception.
/// Searches the handler table for a matching handler at the current PC.
/// If found, jumps to the handler. If not, returns a ThreadFault.
pub(crate) fn op_raise(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let str_id = vm.src_ptr()?;
    let msg = vm
        .heap
        .get_string(str_id)
        .unwrap_or("unknown exception")
        .to_string();

    let current_pc = vm.pc as i32;

    // Search the handler table for a matching handler
    for handler in &vm.module.handlers {
        if current_pc < handler.begin_pc || current_pc >= handler.end_pc {
            continue;
        }
        // Found a handler covering this PC. Search cases.
        for case in &handler.cases {
            match &case.name {
                Some(name) if *name == msg => {
                    vm.next_pc = case.pc as usize;
                    let frame_base = vm.frames.current_data_offset();
                    let off = frame_base + handler.exception_offset as usize;
                    if off + 4 <= vm.frames.data.len() {
                        crate::memory::write_word(&mut vm.frames.data, off, str_id as i32);
                    }
                    return Ok(());
                }
                None => {
                    // Wildcard handler
                    vm.next_pc = case.pc as usize;
                    let frame_base = vm.frames.current_data_offset();
                    let off = frame_base + handler.exception_offset as usize;
                    if off + 4 <= vm.frames.data.len() {
                        crate::memory::write_word(&mut vm.frames.data, off, str_id as i32);
                    }
                    return Ok(());
                }
                _ => continue,
            }
        }
    }

    // No handler found
    Err(ExecError::ThreadFault(format!("unhandled exception: {msg}")))
}

/// runt src — runtime check (module type validation). Stub: no-op.
pub(crate) fn op_runt(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let _ = vm.src_word()?;
    Ok(())
}

/// eclr — clear exception state. Stub: no-op.
pub(crate) fn op_eclr(_vm: &mut VmState<'_>) -> Result<(), ExecError> {
    Ok(())
}

/// brkpt — breakpoint. For now, just halt.
pub(crate) fn op_brkpt(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    vm.halted = true;
    Ok(())
}
