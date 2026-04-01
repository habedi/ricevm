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
    let frame_size = vm
        .current_type_size(type_idx)
        .ok_or_else(|| ExecError::Other(format!("invalid type index: {type_idx}")))?;
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

    // Resolve the caller's import table for this import index.
    // Use the currently executing module's import table.
    let imports = if let Some(lm_idx) = vm.current_loaded_module {
        vm.loaded_modules
            .get(lm_idx)
            .and_then(|lm| lm.module.imports.get(import_idx))
            .cloned()
    } else if import_idx < vm.module.imports.len() {
        Some(vm.module.imports[import_idx].clone())
    } else {
        None
    };

    // 1. Special case: "$self" returns a reference to the current module.
    if path == "$self" {
        let func_map = if let Some(imp_mod) = imports.as_ref() {
            let exports = if let Some(module_idx) = vm.current_loaded_module {
                &vm.loaded_modules[module_idx].module.exports
            } else {
                &vm.module.exports
            };
            imp_mod
                .functions
                .iter()
                .map(|imp| {
                    let sig = imp.signature as u32;
                    exports.iter().position(|e| e.signature as u32 == sig)
                })
                .collect()
        } else {
            Vec::new()
        };

        let ref_id = if let Some(module_idx) = vm.current_loaded_module {
            vm.heap.alloc(
                0,
                HeapData::LoadedModule {
                    module_idx,
                    func_map,
                },
            )
        } else {
            vm.heap.alloc(0, HeapData::MainModule { func_map })
        };
        return vm.move_ptr_to_dst(ref_id);
    }

    // 2. Try built-in module first
    if let Some(module_id) = vm.modules.find_builtin(&path) {
        tracing::trace!(
            path = path,
            import_idx = import_idx,
            current_loaded = ?vm.current_loaded_module,
            has_imports = imports.is_some(),
            "load: building func_map"
        );
        let func_map = if let Some(imp_mod) = imports.as_ref() {
            imp_mod
                .functions
                .iter()
                .map(|imp| {
                    let sig = imp.signature as u32;
                    let name = &imp.name;
                    vm.modules.get_module(module_id).and_then(|m| {
                        // Prefer name match (avoids collisions like read/write
                        // sharing the same signature hash).
                        m.funcs
                            .iter()
                            .position(|f| f.name == name)
                            .or_else(|| m.funcs.iter().position(|f| f.sig == sig))
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        let ref_id = vm.heap.alloc(
            0,
            HeapData::ModuleRef {
                module_id,
                func_map,
            },
        );
        return vm.move_ptr_to_dst(ref_id);
    }

    // 3. Try loading from filesystem
    let mut candidates = vec![path.clone(), format!("{path}.dis")];

    // If a root path is configured, resolve absolute Inferno paths through it.
    let root = &vm.root_path;
    if !root.is_empty() && path.starts_with('/') {
        candidates.insert(0, format!("{root}{path}"));
        candidates.insert(1, format!("{root}{path}.dis"));
    }

    // Strip common Inferno prefixes for relative resolution
    let stripped_paths: Vec<String> = ["/dis/lib/", "/dis/", "/"]
        .iter()
        .filter_map(|prefix| path.strip_prefix(prefix).map(|s| s.to_string()))
        .collect();

    // Add probe paths from RICEVM_PROBE env var
    if let Ok(probe) = std::env::var("RICEVM_PROBE") {
        for dir in probe.split(':') {
            if !dir.is_empty() {
                candidates.push(format!("{dir}/{path}"));
                candidates.push(format!("{dir}/{path}.dis"));
                // Also try stripped paths
                for sp in &stripped_paths {
                    candidates.push(format!("{dir}/{sp}"));
                    candidates.push(format!("{dir}/{sp}.dis"));
                }
            }
        }
    }
    // Also try root + stripped paths
    if !root.is_empty() {
        for sp in &stripped_paths {
            candidates.push(format!("{root}/dis/{sp}"));
            candidates.push(format!("{root}/dis/{sp}.dis"));
            candidates.push(format!("{root}/dis/lib/{sp}"));
            candidates.push(format!("{root}/dis/lib/{sp}.dis"));
        }
    }
    candidates.push(format!("./{path}.dis"));
    for sp in &stripped_paths {
        candidates.push(sp.clone());
        candidates.push(format!("{sp}.dis"));
    }

    for candidate in &candidates {
        if let Ok(bytes) = std::fs::read(candidate)
            && let Ok(module) = ricevm_loader::load(&bytes)
        {
            tracing::trace!(name = %module.name, path = %candidate, "Loaded module from file");
            let mp = crate::data::init_mp_with_types(
                module.header.data_size as usize,
                &module.data,
                &mut vm.heap,
                &module.types,
            );

            // Build func_map: match caller's import functions against the
            // loaded module's exports. Prefer name matching to avoid
            // collisions (e.g. splitl/splitr sharing the same signature).
            let func_map = if let Some(imp_mod) = imports.as_ref() {
                imp_mod
                    .functions
                    .iter()
                    .map(|imp| {
                        let sig = imp.signature as u32;
                        let name = &imp.name;
                        module
                            .exports
                            .iter()
                            .position(|e| e.name == *name)
                            .or_else(|| {
                                module.exports.iter().position(|e| e.signature as u32 == sig)
                            })
                    })
                    .collect()
            } else {
                Vec::new()
            };

            let module_idx = vm.loaded_modules.len();
            vm.loaded_modules
                .push(crate::vm::LoadedModule { module, mp });
            let ref_id = vm.heap.alloc(
                0,
                HeapData::LoadedModule {
                    module_idx,
                    func_map,
                },
            );
            return vm.move_ptr_to_dst(ref_id);
        }
    }

    // Module not found: set dst to nil
    vm.move_ptr_to_dst(heap::NIL)
}

/// Resolved module reference: either a built-in or a loaded .dis module.
pub(crate) enum ModuleKind {
    Builtin {
        module_id: u32,
        func_map: Vec<Option<usize>>,
    },
    Main {
        func_map: Vec<Option<usize>>,
    },
    Loaded {
        module_idx: usize,
        func_map: Vec<Option<usize>>,
    },
}

pub(crate) fn resolve_module_ref(
    vm: &VmState<'_>,
    heap_id: heap::HeapId,
) -> Result<ModuleKind, ExecError> {
    match vm.heap.get(heap_id) {
        Some(obj) => match &obj.data {
            HeapData::ModuleRef {
                module_id,
                func_map,
            } => Ok(ModuleKind::Builtin {
                module_id: *module_id,
                func_map: func_map.clone(),
            }),
            HeapData::MainModule { func_map } => Ok(ModuleKind::Main {
                func_map: func_map.clone(),
            }),
            HeapData::LoadedModule {
                module_idx,
                func_map,
            } => Ok(ModuleKind::Loaded {
                module_idx: *module_idx,
                func_map: func_map.clone(),
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
        ModuleKind::Main { func_map } => {
            let export_idx = func_map
                .get(func_idx as usize)
                .copied()
                .flatten()
                .unwrap_or(func_idx as usize);
            if export_idx < vm.module.exports.len() {
                let frame_type = vm.module.exports[export_idx].frame_type as usize;
                if frame_type < vm.module.types.len() {
                    vm.module.types[frame_type].size as usize
                } else {
                    64
                }
            } else {
                64
            }
        }
        ModuleKind::Loaded {
            module_idx,
            func_map,
        } => {
            // Map caller's import index to loaded module's export index
            let export_idx = func_map
                .get(func_idx as usize)
                .copied()
                .flatten()
                .unwrap_or(func_idx as usize);
            let loaded = &vm.loaded_modules[module_idx];
            if export_idx < loaded.module.exports.len() {
                let frame_type = loaded.module.exports[export_idx].frame_type as usize;
                if frame_type < loaded.module.types.len() {
                    loaded.module.types[frame_type].size as usize
                } else {
                    64
                }
            } else {
                64
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
            // Copy return value from callee frame to caller via return pointer.
            let data_off = vm.frames.current_data_offset();
            let ret_ptr = crate::memory::read_word(&vm.frames.data, data_off + 16);
            if ret_ptr != 0 {
                let target = crate::address::decode_virtual_addr(ret_ptr, 0);
                let ret_val = crate::memory::read_word(&vm.frames.data, data_off);
                vm.write_word_at(target, ret_val)?;
            }
            // Auto-return from built-in call
            let prev_pc = vm.frames.pop()?;
            if prev_pc >= 0 {
                vm.next_pc = prev_pc as usize;
            }
        }
        ModuleKind::Main { func_map } => {
            if vm.current_loaded_module.is_some() {
                return Err(ExecError::Other(
                    "calling main-module refs from loaded modules is unsupported".to_string(),
                ));
            }

            let export_idx = func_map
                .get(func_idx as usize)
                .copied()
                .flatten()
                .unwrap_or(func_idx as usize);
            let entry_pc = if export_idx < vm.module.exports.len() {
                vm.module.exports[export_idx].pc as usize
            } else {
                return Err(ExecError::Other(format!(
                    "export function {func_idx} (mapped to {export_idx}) not found in main module"
                )));
            };

            let saved_pc = vm.pc;
            let saved_next_pc = vm.next_pc;

            vm.pc = entry_pc;
            vm.halted = false;
            let mcall_frame_base = vm.frames.current_data_offset();

            while !vm.halted && vm.pc < vm.module.code.len() {
                let inst = vm.module.code[vm.pc].clone();
                if vm.trace {
                    vm.trace_instruction(&inst);
                }
                vm.resolve_operands(&inst)?;
                vm.next_pc = vm.pc + 1;
                crate::ops::dispatch(vm, &inst)?;
                vm.pc = vm.next_pc;

                if vm.frames.current_data_offset() < mcall_frame_base {
                    break;
                }
            }

            vm.pc = saved_pc;
            vm.next_pc = saved_next_pc;
            vm.halted = false;
        }
        ModuleKind::Loaded {
            module_idx,
            func_map,
        } => {
            // Map caller's import index to loaded module's export index
            let export_idx = func_map
                .get(func_idx as usize)
                .copied()
                .flatten()
                .unwrap_or(func_idx as usize);
            let entry_pc = {
                let loaded = &vm.loaded_modules[module_idx];
                if export_idx < loaded.module.exports.len() {
                    loaded.module.exports[export_idx].pc as usize
                } else {
                    return Err(ExecError::Other(format!(
                        "export function {func_idx} (mapped to {export_idx}) not found in loaded module"
                    )));
                }
            };

            // Save current execution context
            let saved_pc = vm.pc;
            let saved_next_pc = vm.next_pc;
            let saved_loaded_module = vm.current_loaded_module;

            // Swap MP with the loaded module's persistent MP.
            // This ensures module refs stored during execution persist
            // in the loaded module's MP for subsequent calls.
            let caller_virt_idx = vm.current_module_virt_idx();
            let loaded_mp = std::mem::take(&mut vm.loaded_modules[module_idx].mp);
            let parent_mp = std::mem::replace(&mut vm.mp, loaded_mp);
            // Push the caller's MP onto the stack so cross-module virtual
            // addresses can resolve to it during execution.
            vm.caller_mp_stack.push((caller_virt_idx, parent_mp));

            let loaded_code_len = vm.loaded_modules[module_idx].module.code.len();
            vm.current_loaded_module = Some(module_idx);
            vm.pc = entry_pc;
            vm.halted = false;

            // Track the frame stack state before entering the loaded module.
            // The mcall frame was already activated above. Record the current
            // frame base so we can detect when Ret pops past it.
            let mcall_frame_base = vm.frames.current_data_offset();

            // Execute the loaded module's code
            while !vm.halted && vm.pc < loaded_code_len {
                let inst = vm.loaded_modules[module_idx].module.code[vm.pc].clone();
                if vm.trace {
                    vm.trace_instruction(&inst);
                }
                vm.resolve_operands(&inst)?;
                vm.next_pc = vm.pc + 1;
                crate::ops::dispatch(vm, &inst)?;
                vm.pc = vm.next_pc;

                // If Ret popped our mcall frame (current frame's data area
                // is now below where we started), the function returned.
                if vm.frames.current_data_offset() < mcall_frame_base {
                    break;
                }
            }

            // Write back the loaded module's MP (preserving any changes),
            // then restore the parent's MP from the stack.
            let (_, parent_mp) = vm.caller_mp_stack.pop().unwrap_or_default();
            vm.loaded_modules[module_idx].mp = std::mem::replace(&mut vm.mp, parent_mp);
            vm.pc = saved_pc;
            vm.next_pc = saved_next_pc;
            vm.current_loaded_module = saved_loaded_module;
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
        crate::address::AddrTarget::Mp(off) => crate::memory::read_word(&vm.mp, off + index * 4),
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

    // Binary search matching the reference Dis VM implementation.
    // Table layout: [count, (lo, hi, pc)..., default_pc]
    // Match condition: lo <= value < hi
    let n = count as usize;
    let default_pc = read_table(vm, 1 + n * 3)?;
    let mut target_pc = default_pc;

    // t points to index 1 (first entry after count)
    let mut t_off = 1usize;
    let mut remaining = n;
    while remaining > 0 {
        let n2 = remaining >> 1;
        let l_off = t_off + n2 * 3;
        let lo = read_table(vm, l_off)?;
        let hi = read_table(vm, l_off + 1)?;
        if value < lo {
            remaining = n2;
        } else if value >= hi {
            t_off = l_off + 3;
            remaining -= n2 + 1;
        } else {
            target_pc = read_table(vm, l_off + 2)?;
            break;
        }
    }

    vm.next_pc = target_pc as usize;
    Ok(())
}

/// casec src, dst — string case dispatch.
///
/// Same table format as casew, but lo/hi are string pointer HeapIds.
/// Binary search with string comparison matching the reference Dis VM.
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

    let n = count as usize;
    let default_pc = read_table(vm, 1 + n * 3)?;
    let mut target_pc = default_pc;

    // Binary search matching reference Dis VM casec implementation.
    let mut t_off = 1usize;
    let mut remaining = n;
    while remaining > 0 {
        let n2 = remaining >> 1;
        let l_off = t_off + n2 * 3;
        let lo_id = read_table(vm, l_off)? as heap::HeapId;
        let lo_str = vm.heap.get_string(lo_id).unwrap_or("");
        let cmp = value_str.as_str().cmp(lo_str);

        if cmp == std::cmp::Ordering::Equal {
            target_pc = read_table(vm, l_off + 2)?;
            break;
        }
        if cmp == std::cmp::Ordering::Less {
            remaining = n2;
            continue;
        }
        // value > lo: check hi
        let hi_id = read_table(vm, l_off + 1)? as heap::HeapId;
        if hi_id == heap::NIL
            || value_str.as_str().cmp(vm.heap.get_string(hi_id).unwrap_or(""))
                == std::cmp::Ordering::Greater
        {
            t_off = l_off + 3;
            remaining -= n2 + 1;
            continue;
        }
        // lo < value <= hi: match
        target_pc = read_table(vm, l_off + 2)?;
        break;
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

    // Reference layout: count at word[0], then padding word[1], then entries
    // starting at byte offset 8.  Each entry is 6 words (24 bytes):
    //   lo_big(8) + hi_big(8) + pc(4) + pad(4)
    // Default pc is at t[n*6] = byte offset 8 + n*24.
    let entry_size = 24; // 6 words
    let n = count as usize;
    let t_start = 8usize; // entries begin at byte 8
    let default_off = t_start + n * entry_size;
    let default_pc = read_word(vm, default_off)?;
    let mut target_pc = default_pc;

    let mut t_off = t_start;
    let mut remaining = n;
    while remaining > 0 {
        let n2 = remaining >> 1;
        let l_off = t_off + n2 * entry_size;
        let lo = read_big(vm, l_off)?;
        let hi = read_big(vm, l_off + 8)?;
        if value < lo {
            remaining = n2;
        } else if value >= hi {
            t_off = l_off + entry_size;
            remaining -= n2 + 1;
        } else {
            target_pc = read_word(vm, l_off + 16)?;
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

#[cfg(test)]
mod tests {
    use ricevm_core::{
        Header, Instruction, MiddleOperand, Module, Opcode, Operand, PointerMap, RuntimeFlags,
        TypeDescriptor, XMAGIC,
    };

    use super::*;
    use crate::address::AddrTarget;
    use crate::memory;

    fn test_module() -> Module {
        Module {
            header: Header {
                magic: XMAGIC,
                signature: vec![],
                runtime_flags: RuntimeFlags(0),
                stack_extent: 0,
                code_size: 1,
                data_size: 0,
                type_size: 1,
                export_size: 0,
                entry_pc: 0,
                entry_type: 0,
            },
            code: vec![Instruction {
                opcode: Opcode::Exit,
                source: Operand::UNUSED,
                middle: MiddleOperand::UNUSED,
                destination: Operand::UNUSED,
            }],
            types: vec![TypeDescriptor {
                id: 0,
                size: 128,
                pointer_map: PointerMap { bytes: vec![] },
                pointer_count: 0,
            }],
            data: vec![],
            name: "control_test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        }
    }

    /// Regression: casew must use binary search and find values at exact
    /// boundaries of ranges. The original linear search missed boundary values.
    #[test]
    fn casew_binary_search_finds_boundary_values() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        // Build a case table with 3 entries:
        //   [0, 10)  -> pc 100
        //   [10, 20) -> pc 200
        //   [20, 30) -> pc 300
        //   default  -> pc 999
        //
        // Table layout: [count, lo0, hi0, pc0, lo1, hi1, pc1, lo2, hi2, pc2, default]
        let table_off = fp + 4;
        memory::write_word(&mut vm.frames.data, table_off, 3);      // count
        memory::write_word(&mut vm.frames.data, table_off + 4, 0);  // lo0
        memory::write_word(&mut vm.frames.data, table_off + 8, 10); // hi0
        memory::write_word(&mut vm.frames.data, table_off + 12, 100); // pc0
        memory::write_word(&mut vm.frames.data, table_off + 16, 10); // lo1
        memory::write_word(&mut vm.frames.data, table_off + 20, 20); // hi1
        memory::write_word(&mut vm.frames.data, table_off + 24, 200); // pc1
        memory::write_word(&mut vm.frames.data, table_off + 28, 20); // lo2
        memory::write_word(&mut vm.frames.data, table_off + 32, 30); // hi2
        memory::write_word(&mut vm.frames.data, table_off + 36, 300); // pc2
        memory::write_word(&mut vm.frames.data, table_off + 40, 999); // default

        // Test exact boundary: value 0 (lower bound of first range)
        vm.src = AddrTarget::Immediate;
        vm.imm_src = 0;
        vm.dst = AddrTarget::Frame(table_off);
        vm.imm_dst = 0;
        vm.next_pc = 0;
        op_casew(&mut vm).expect("casew should succeed");
        assert_eq!(vm.next_pc, 100, "value 0 should match [0, 10) -> pc 100");

        // Test exact boundary: value 10 (lower bound of second range)
        vm.imm_src = 10;
        vm.next_pc = 0;
        op_casew(&mut vm).expect("casew should succeed");
        assert_eq!(vm.next_pc, 200, "value 10 should match [10, 20) -> pc 200");

        // Test exact boundary: value 20 (lower bound of third range)
        vm.imm_src = 20;
        vm.next_pc = 0;
        op_casew(&mut vm).expect("casew should succeed");
        assert_eq!(vm.next_pc, 300, "value 20 should match [20, 30) -> pc 300");

        // Test value just before boundary: value 9 (last in first range)
        vm.imm_src = 9;
        vm.next_pc = 0;
        op_casew(&mut vm).expect("casew should succeed");
        assert_eq!(vm.next_pc, 100, "value 9 should match [0, 10) -> pc 100");

        // Test value just before boundary: value 19 (last in second range)
        vm.imm_src = 19;
        vm.next_pc = 0;
        op_casew(&mut vm).expect("casew should succeed");
        assert_eq!(vm.next_pc, 200, "value 19 should match [10, 20) -> pc 200");

        // Test value just before boundary: value 29 (last in third range)
        vm.imm_src = 29;
        vm.next_pc = 0;
        op_casew(&mut vm).expect("casew should succeed");
        assert_eq!(vm.next_pc, 300, "value 29 should match [20, 30) -> pc 300");

        // Test default: value 30 (outside all ranges)
        vm.imm_src = 30;
        vm.next_pc = 0;
        op_casew(&mut vm).expect("casew should succeed");
        assert_eq!(vm.next_pc, 999, "value 30 should go to default pc 999");

        // Test default: value -1 (below all ranges)
        vm.imm_src = -1;
        vm.next_pc = 0;
        op_casew(&mut vm).expect("casew should succeed");
        assert_eq!(vm.next_pc, 999, "value -1 should go to default pc 999");
    }

    /// Regression: casew binary search with a single entry should still match.
    #[test]
    fn casew_single_entry() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        let table_off = fp + 4;
        memory::write_word(&mut vm.frames.data, table_off, 1);      // count
        memory::write_word(&mut vm.frames.data, table_off + 4, 5);  // lo
        memory::write_word(&mut vm.frames.data, table_off + 8, 15); // hi
        memory::write_word(&mut vm.frames.data, table_off + 12, 42); // pc
        memory::write_word(&mut vm.frames.data, table_off + 16, 99); // default

        // Value in range
        vm.src = AddrTarget::Immediate;
        vm.imm_src = 7;
        vm.dst = AddrTarget::Frame(table_off);
        vm.imm_dst = 0;
        vm.next_pc = 0;
        op_casew(&mut vm).expect("casew should succeed");
        assert_eq!(vm.next_pc, 42, "value 7 should match [5, 15) -> pc 42");

        // Value out of range
        vm.imm_src = 15;
        vm.next_pc = 0;
        op_casew(&mut vm).expect("casew should succeed");
        assert_eq!(vm.next_pc, 99, "value 15 should go to default");
    }
}
