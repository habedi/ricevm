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

    // Get the import table index to build function mapping
    let import_idx = vm.mid_word()? as usize;

    // 1. Try built-in module first
    if let Some(module_id) = vm.modules.find_builtin(&path) {
        // Build function mapping from caller's import table signatures
        // to builtin function indices
        let func_map = if import_idx < vm.module.imports.len() {
            vm.module.imports[import_idx]
                .functions
                .iter()
                .map(|imp| {
                    let sig = imp.signature as u32;
                    vm.modules
                        .get_module(module_id)
                        .and_then(|m| m.funcs.iter().position(|f| f.sig == sig))
                })
                .collect()
        } else {
            Vec::new()
        };
        let ref_id = vm.heap.alloc(0, HeapData::ModuleRef { module_id, func_map });
        return vm.move_ptr_to_dst(ref_id);
    }

    // 2. Try loading from filesystem
    let mut candidates = vec![
        path.clone(),
        format!("{path}.dis"),
    ];
    // Add probe paths from RICEVM_PROBE env var
    if let Ok(probe) = std::env::var("RICEVM_PROBE") {
        for dir in probe.split(':') {
            if !dir.is_empty() {
                candidates.push(format!("{dir}/{path}"));
                candidates.push(format!("{dir}/{path}.dis"));
            }
        }
    }
    candidates.push(format!("./{path}.dis"));

    for candidate in &candidates {
        if let Ok(bytes) = std::fs::read(candidate)
            && let Ok(module) = ricevm_loader::load(&bytes)
        {
            tracing::info!(name = %module.name, path = %candidate, "Loaded module from file");
            let mp =
                crate::data::init_mp(module.header.data_size as usize, &module.data, &mut vm.heap);
            let module_idx = vm.loaded_modules.len();
            vm.loaded_modules
                .push(crate::vm::LoadedModule { module, mp });
            let ref_id = vm.heap.alloc(0, HeapData::LoadedModule { module_idx });
            return vm.move_ptr_to_dst(ref_id);
        }
    }

    // Module not found: set dst to nil
    vm.move_ptr_to_dst(heap::NIL)
}

/// Resolved module reference: either a built-in or a loaded .dis module.
enum ModuleKind {
    Builtin {
        module_id: u32,
        func_map: Vec<Option<usize>>,
    },
    Loaded {
        module_idx: usize,
    },
}

fn resolve_module_ref(vm: &VmState<'_>, heap_id: heap::HeapId) -> Result<ModuleKind, ExecError> {
    match vm.heap.get(heap_id) {
        Some(obj) => match &obj.data {
            HeapData::ModuleRef {
                module_id,
                func_map,
            } => Ok(ModuleKind::Builtin {
                module_id: *module_id,
                func_map: func_map.clone(),
            }),
            HeapData::LoadedModule { module_idx } => Ok(ModuleKind::Loaded {
                module_idx: *module_idx,
            }),
            _ => Err(ExecError::ThreadFault("not a module ref".to_string())),
        },
        None => Err(ExecError::ThreadFault("nil module".to_string())),
    }
}

/// mframe src, mid, dst — create frame for module call
/// src = module ref pointer, mid = function index, dst = frame pointer (output)
pub(crate) fn op_mframe(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let mod_ref_id = vm.src_ptr()?;
    let func_idx = vm.mid_word()? as u32;

    let frame_size = match resolve_module_ref(vm, mod_ref_id)? {
        ModuleKind::Builtin {
            module_id,
            func_map,
        } => {
            // Map import index to builtin index via func_map
            let builtin_idx = func_map
                .get(func_idx as usize)
                .copied()
                .flatten()
                .unwrap_or(func_idx as usize);
            vm.modules
                .get_func(module_id, builtin_idx as u32)
                .map(|f| f.frame_size)
                .ok_or_else(|| {
                    ExecError::Other(format!(
                        "builtin function not found: module={module_id}, func={func_idx} (mapped to {builtin_idx})"
                    ))
                })?
        }
        ModuleKind::Loaded { module_idx } => {
            // For loaded modules, get frame size from the module's export + type section
            let loaded = &vm.loaded_modules[module_idx];
            if (func_idx as usize) < loaded.module.exports.len() {
                let frame_type = loaded.module.exports[func_idx as usize].frame_type as usize;
                if frame_type < loaded.module.types.len() {
                    loaded.module.types[frame_type].size as usize
                } else {
                    64 // default
                }
            } else {
                64 // default
            }
        }
    };

    let pending_data_offset = vm.frames.alloc_pending(frame_size)?;
    vm.set_dst_word(pending_data_offset as i32)
}

/// mcall src, mid, dst — call function in loaded module
/// src = frame pointer, mid = function index, dst = module ref pointer
pub(crate) fn op_mcall(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_data_offset = vm.src_word()? as usize;
    let func_idx = vm.mid_word()? as u32;
    let mod_ref_id = vm.dst_ptr()?;

    let kind = resolve_module_ref(vm, mod_ref_id)?;

    // Activate the pending frame
    vm.frames
        .activate_pending(frame_data_offset, vm.next_pc as i32)?;

    match kind {
        ModuleKind::Builtin {
            module_id,
            func_map,
        } => {
            // Map import index to builtin index via func_map
            let builtin_idx = func_map
                .get(func_idx as usize)
                .copied()
                .flatten()
                .unwrap_or(func_idx as usize);
            let handler = vm
                .modules
                .get_func(module_id, builtin_idx as u32)
                .map(|f| f.handler)
                .ok_or_else(|| {
                    ExecError::Other(format!(
                        "builtin function not found: module={module_id}, func={func_idx} (mapped to {builtin_idx})"
                    ))
                })?;
            handler(vm)?;
            // Auto-return from built-in call
            let prev_pc = vm.frames.pop()?;
            if prev_pc >= 0 {
                vm.next_pc = prev_pc as usize;
            }
        }
        ModuleKind::Loaded { module_idx } => {
            // For loaded Limbo modules: we would need to switch the execution
            // context to the loaded module's code. This requires saving the
            // current module/mp/pc and running the loaded module's code.
            // For now, this is a simplified implementation that doesn't support
            // re-entrant execution across modules.
            let entry_pc = {
                let loaded = &vm.loaded_modules[module_idx];
                if (func_idx as usize) < loaded.module.exports.len() {
                    loaded.module.exports[func_idx as usize].pc as usize
                } else {
                    return Err(ExecError::Other(format!(
                        "export function {func_idx} not found in loaded module"
                    )));
                }
            };

            // Save current execution context
            let saved_pc = vm.pc;
            let saved_next_pc = vm.next_pc;
            let saved_mp = std::mem::take(&mut vm.mp);

            // Switch to loaded module's context
            // Safety: we need to borrow the loaded module's code section
            // This is a temporary pointer swap — the module lives in loaded_modules
            let loaded_code_len = vm.loaded_modules[module_idx].module.code.len();
            vm.mp = vm.loaded_modules[module_idx].mp.clone();
            vm.pc = entry_pc;
            vm.halted = false;

            // Execute the loaded module's code until it returns
            while !vm.halted && vm.pc < loaded_code_len {
                let inst = vm.loaded_modules[module_idx].module.code[vm.pc].clone();
                if vm.trace {
                    vm.trace_instruction(&inst);
                }
                vm.resolve_operands(&inst)?;
                vm.next_pc = vm.pc + 1;
                crate::ops::dispatch(vm, &inst)?;
                vm.pc = vm.next_pc;

                // Check if we've returned (frame popped back to caller)
                // The saved_pc in the frame header will be the return address
            }

            // Restore context
            vm.mp = saved_mp;
            vm.pc = saved_pc;
            vm.next_pc = saved_next_pc;
            vm.halted = false;
        }
    }

    Ok(())
}

/// goto src, dst — computed goto: dst holds a pc table pointer, src is the index.
/// In Dis, `goto` jumps to the PC stored in a case table.
/// src = pointer to word array of PCs, dst = index.
/// goto src, dst — computed goto: read index from src, jump to table[index].
/// dst points to a flat array of word-sized PCs in frame or MP memory.
pub(crate) fn op_goto(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let index = vm.src_word()? as usize;
    // Read the target PC from the table at dst + index * 4
    let target = match vm.dst {
        crate::address::AddrTarget::Frame(off) => {
            crate::memory::read_word(&vm.frames.data, off + index * 4)
        }
        crate::address::AddrTarget::Mp(off) => {
            crate::memory::read_word(&vm.mp, off + index * 4)
        }
        _ => vm.dst_word()?,
    };
    vm.next_pc = target as usize;
    Ok(())
}

/// casew src, dst — word case dispatch.
/// src = value to match, dst = pointer to case table in frame/MP.
///
/// Case table format (words):
///   [0]     = N (number of entries)
///   [1..3N] = N triples of (lo, hi, target_pc) — matches if lo <= value < hi
///   [3N+1]  = default_pc
pub(crate) fn op_casew(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let value = vm.src_word()?;

    // Read the case table from the dst location
    let table_base = vm.dst;
    let count = vm.read_word_at(table_base, vm.imm_dst)?;

    // Helper to read the i-th word from the table
    let read_table = |vm: &VmState<'_>, idx: usize| -> Result<i32, ExecError> {
        match table_base {
            crate::address::AddrTarget::Frame(off) => {
                Ok(crate::memory::read_word(&vm.frames.data, off + idx * 4))
            }
            crate::address::AddrTarget::Mp(off) => {
                Ok(crate::memory::read_word(&vm.mp, off + idx * 4))
            }
            _ => Ok(0),
        }
    };

    // Default PC is after all entries
    let default_pc = read_table(vm, 1 + count as usize * 3)?;
    let mut target_pc = default_pc;

    // Search entries
    for i in 0..count as usize {
        let base = 1 + i * 3;
        let lo = read_table(vm, base)?;
        let hi = read_table(vm, base + 1)?;
        let pc = read_table(vm, base + 2)?;
        if lo <= value && value < hi {
            target_pc = pc;
            break;
        }
    }

    vm.next_pc = target_pc as usize;
    Ok(())
}

/// casec src, dst — string case dispatch.
///
/// Same table format as casew, but lo/hi are string pointer HeapIds.
/// Matches if value == lo, or (value > lo && value == hi).
pub(crate) fn op_casec(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let value_id = vm.src_ptr()?;
    let value_str = vm.heap.get_string(value_id).unwrap_or("").to_string();

    let table_base = vm.dst;
    let count = vm.read_word_at(table_base, vm.imm_dst)?;

    let read_table = |vm: &VmState<'_>, idx: usize| -> Result<i32, ExecError> {
        match table_base {
            crate::address::AddrTarget::Frame(off) => {
                Ok(crate::memory::read_word(&vm.frames.data, off + idx * 4))
            }
            crate::address::AddrTarget::Mp(off) => {
                Ok(crate::memory::read_word(&vm.mp, off + idx * 4))
            }
            _ => Ok(0),
        }
    };

    let default_pc = read_table(vm, 1 + count as usize * 3)?;
    let mut target_pc = default_pc;

    for i in 0..count as usize {
        let base = 1 + i * 3;
        let lo_id = read_table(vm, base)? as heap::HeapId;
        let hi_id = read_table(vm, base + 1)? as heap::HeapId;
        let pc = read_table(vm, base + 2)?;

        let lo_str = vm.heap.get_string(lo_id).unwrap_or("");
        let cmp = value_str.as_str().cmp(lo_str);

        if cmp == std::cmp::Ordering::Equal {
            target_pc = pc;
            break;
        }
        if cmp == std::cmp::Ordering::Greater {
            let hi_str = vm.heap.get_string(hi_id).unwrap_or("");
            if !hi_str.is_empty() && value_str.as_str() == hi_str {
                target_pc = pc;
                break;
            }
        }
    }

    vm.next_pc = target_pc as usize;
    Ok(())
}

/// casel src, dst — big case dispatch.
/// Same table format as casew but values are big (i64, 8 bytes each).
/// Table entries: count(word), then N triples of (lo_big, hi_big, pc_word).
pub(crate) fn op_casel(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let value = vm.src_big()?;

    let table_base = vm.dst;
    let count = vm.read_word_at(table_base, vm.imm_dst)?;

    let read_word = |vm: &VmState<'_>, byte_off: usize| -> Result<i32, ExecError> {
        match table_base {
            crate::address::AddrTarget::Frame(off) => {
                Ok(crate::memory::read_word(&vm.frames.data, off + byte_off))
            }
            crate::address::AddrTarget::Mp(off) => {
                Ok(crate::memory::read_word(&vm.mp, off + byte_off))
            }
            _ => Ok(0),
        }
    };

    let read_big = |vm: &VmState<'_>, byte_off: usize| -> Result<i64, ExecError> {
        match table_base {
            crate::address::AddrTarget::Frame(off) => {
                Ok(crate::memory::read_big(&vm.frames.data, off + byte_off))
            }
            crate::address::AddrTarget::Mp(off) => {
                Ok(crate::memory::read_big(&vm.mp, off + byte_off))
            }
            _ => Ok(0),
        }
    };

    // Layout: count(4 bytes), then N * (lo_big(8) + hi_big(8) + pc(4)) = 20 bytes per entry, then default_pc(4)
    let entry_size = 20; // 8 + 8 + 4
    let default_off = 4 + count as usize * entry_size;
    let default_pc = read_word(vm, default_off)?;
    let mut target_pc = default_pc;

    for i in 0..count as usize {
        let base = 4 + i * entry_size;
        let lo = read_big(vm, base)?;
        let hi = read_big(vm, base + 8)?;
        let pc = read_word(vm, base + 16)?;
        if lo <= value && value < hi {
            target_pc = pc;
            break;
        }
    }

    vm.next_pc = target_pc as usize;
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
    Err(ExecError::ThreadFault(format!(
        "unhandled exception: {msg}"
    )))
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
