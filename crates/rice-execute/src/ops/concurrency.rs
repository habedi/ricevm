//! Concurrency opcodes.
//!
//! `spawn` executes the spawned function inline (cooperative, not preemptive).
//! Channels are modeled as single-slot buffers stored in heap objects.
//! `alt` and `nbalt` scan a flat table of channel operations.

use ricevm_core::ExecError;

use super::control::{ModuleKind, resolve_module_ref};
use crate::address::AddrTarget;
use crate::heap::{self, HeapData, HeapId};
use crate::memory;
use crate::vm::VmState;

#[derive(Clone, Copy)]
enum TableBase {
    Frame,
    Mp,
}

#[derive(Clone, Copy)]
struct AltEntry {
    channel_id: HeapId,
    is_send: bool,
    data_offset: usize,
}

enum AltOutcome {
    Selected(usize),
    NoneReady,
}

/// spawn src, dst:create a new thread in the current module.
/// src = frame pointer (pre-allocated via `frame`), dst = target PC.
///
/// Cooperative implementation: activates the pending frame and runs the
/// spawned function inline until it returns, then continues the caller.
pub(crate) fn op_spawn(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_ptr = vm.src_word()? as usize;
    let target_pc = vm.dst_word()? as usize;

    // The pending frame was allocated by a previous 'frame' instruction.
    // Activate it so we can extract a properly initialized frame.
    vm.frames.activate_pending(frame_ptr, -1)?; // -1 = thread entry sentinel

    // Extract the child's frame from the top of the parent's stack.
    let child_base = vm.frames.current_base;
    let child_data = vm.frames.data[child_base..].to_vec();
    vm.frames.data.truncate(child_base);

    // Restore parent's frame state (pop the child frame we just activated).
    // The activate_pending updated current_base/current_size; revert them.
    // We read prev_base from the child frame header we just extracted.
    let prev_base = crate::memory::read_word(&child_data, 4) as usize;
    if prev_base < vm.frames.data.len() {
        vm.frames.current_base = prev_base;
        vm.frames.current_size = vm.frames.data.len() - prev_base;
    } else {
        vm.frames.current_base = 0;
        vm.frames.current_size = vm.frames.data.len();
    }

    // Create child's frame stack with just the extracted frame.
    let mut child_frames = crate::frame::FrameStack::new();
    child_frames.data = child_data;
    child_frames.current_base = 0;
    child_frames.current_size = child_frames.data.len();

    // Create suspended thread for the child.
    let child = crate::vm::SuspendedThread {
        frames: child_frames,
        mp: vm.mp.clone(),
        pc: target_pc,
        heap_refs: Vec::new(),
        last_error: String::new(),
        current_loaded_module: vm.current_loaded_module,
        caller_mp_stack: vm.caller_mp_stack.clone(),
        blocked_on: None,
    };
    vm.thread_queue.push_back(child);

    // Parent continues at the next instruction.
    Ok(())
}

/// mspawn src, mid, dst:create a thread in a loaded module.
pub(crate) fn op_mspawn(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_ptr = vm.src_word()? as usize;
    let func_idx = vm.mid_word()? as u32;
    let mod_ref_id = vm.dst_ptr()?;
    let kind = resolve_module_ref(vm, mod_ref_id)?;

    vm.frames.activate_pending(frame_ptr, vm.next_pc as i32)?;

    let saved_pc = vm.pc;
    let saved_next_pc = vm.next_pc;
    let spawn_frame_base = vm.frames.current_data_offset();

    match kind {
        ModuleKind::Builtin {
            module_id,
            func_map,
        } => {
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
            let prev_pc = vm.frames.pop()?;
            if prev_pc >= 0 {
                vm.next_pc = prev_pc as usize;
            }
        }
        ModuleKind::Main { func_map } => {
            if vm.current_loaded_module.is_some() {
                return Err(ExecError::Other(
                    "spawning main-module refs from loaded modules is unsupported".to_string(),
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

            vm.pc = entry_pc;
            vm.halted = false;

            while !vm.halted && vm.pc < vm.module.code.len() {
                let inst = vm.module.code[vm.pc].clone();
                if vm.trace {
                    vm.trace_instruction(&inst);
                }
                vm.resolve_operands(&inst)?;
                vm.next_pc = vm.pc + 1;
                crate::ops::dispatch(vm, &inst)?;
                vm.pc = vm.next_pc;

                if vm.frames.current_data_offset() < spawn_frame_base {
                    break;
                }
            }

            vm.halted = false;
        }
        ModuleKind::Loaded {
            module_idx,
            func_map,
        } => {
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

            let saved_loaded_module = vm.current_loaded_module;
            let caller_virt_idx = vm.current_module_virt_idx();
            let loaded_mp = std::mem::take(&mut vm.loaded_modules[module_idx].mp);
            let parent_mp = std::mem::replace(&mut vm.mp, loaded_mp);
            vm.caller_mp_stack.push((caller_virt_idx, parent_mp));

            let loaded_code_len = vm.loaded_modules[module_idx].module.code.len();
            vm.current_loaded_module = Some(module_idx);
            vm.pc = entry_pc;
            vm.halted = false;

            while !vm.halted && vm.pc < loaded_code_len {
                let inst = vm.loaded_modules[module_idx].module.code[vm.pc].clone();
                if vm.trace {
                    vm.trace_instruction(&inst);
                }
                vm.resolve_operands(&inst)?;
                vm.next_pc = vm.pc + 1;
                crate::ops::dispatch(vm, &inst)?;
                vm.pc = vm.next_pc;

                if vm.frames.current_data_offset() < spawn_frame_base {
                    break;
                }
            }

            let (_, parent_mp) = vm.caller_mp_stack.pop().unwrap_or_default();
            vm.loaded_modules[module_idx].mp = std::mem::replace(&mut vm.mp, parent_mp);
            vm.current_loaded_module = saved_loaded_module;
            vm.halted = false;
        }
    }

    vm.pc = saved_pc;
    vm.next_pc = saved_next_pc;
    Ok(())
}

fn channel_ref(vm: &VmState<'_>, chan_id: HeapId) -> Result<(usize, Option<Vec<u8>>), ExecError> {
    if chan_id == heap::NIL {
        return Err(ExecError::ThreadFault("nil channel".to_string()));
    }

    let obj = vm
        .heap
        .get(chan_id)
        .ok_or_else(|| ExecError::ThreadFault("dangling channel".to_string()))?;
    match &obj.data {
        HeapData::Channel { elem_size, pending } => Ok((*elem_size, pending.clone())),
        _ => Err(ExecError::ThreadFault(
            "operation on non-channel".to_string(),
        )),
    }
}

fn with_channel_mut<R>(
    vm: &mut VmState<'_>,
    chan_id: HeapId,
    f: impl FnOnce(usize, &mut Option<Vec<u8>>) -> R,
) -> Result<R, ExecError> {
    if chan_id == heap::NIL {
        return Err(ExecError::ThreadFault("nil channel".to_string()));
    }

    let obj = vm
        .heap
        .get_mut(chan_id)
        .ok_or_else(|| ExecError::ThreadFault("dangling channel".to_string()))?;
    match &mut obj.data {
        HeapData::Channel { elem_size, pending } => Ok(f(*elem_size, pending)),
        _ => Err(ExecError::ThreadFault(
            "operation on non-channel".to_string(),
        )),
    }
}

fn read_addr_bytes(
    vm: &VmState<'_>,
    target: AddrTarget,
    imm: i32,
    size: usize,
) -> Result<Vec<u8>, ExecError> {
    let mut buf = vec![0u8; size];
    match target {
        AddrTarget::Frame(off) => {
            if off < vm.frames.data.len() {
                let copy_len = size.min(vm.frames.data.len() - off);
                buf[..copy_len].copy_from_slice(&vm.frames.data[off..off + copy_len]);
            }
        }
        AddrTarget::Mp(off) => {
            if off < vm.mp.len() {
                let copy_len = size.min(vm.mp.len() - off);
                buf[..copy_len].copy_from_slice(&vm.mp[off..off + copy_len]);
            }
        }
        AddrTarget::ModuleMp { module_idx, offset } => {
            if let Some(mp) = vm.module_mp(module_idx)
                && offset < mp.len()
            {
                let copy_len = size.min(mp.len() - offset);
                buf[..copy_len].copy_from_slice(&mp[offset..offset + copy_len]);
            }
        }
        AddrTarget::Immediate => match size {
            1 => buf[0] = imm as u8,
            8 => buf.copy_from_slice(&(imm as i64).to_ne_bytes()),
            _ => {
                let word = imm.to_ne_bytes();
                let copy_len = size.min(word.len());
                buf[..copy_len].copy_from_slice(&word[..copy_len]);
            }
        },
        AddrTarget::None => {}
        AddrTarget::HeapArray { id, offset } => {
            if let Some(bytes) = vm.heap_slice(id, offset, size) {
                buf.copy_from_slice(&bytes);
            }
        }
    }
    Ok(buf)
}

fn write_addr_bytes(
    vm: &mut VmState<'_>,
    target: AddrTarget,
    data: &[u8],
) -> Result<(), ExecError> {
    match target {
        AddrTarget::Frame(off) => {
            if off < vm.frames.data.len() {
                let copy_len = data.len().min(vm.frames.data.len() - off);
                vm.frames.data[off..off + copy_len].copy_from_slice(&data[..copy_len]);
            }
            Ok(())
        }
        AddrTarget::Mp(off) => {
            if off < vm.mp.len() {
                let copy_len = data.len().min(vm.mp.len() - off);
                vm.mp[off..off + copy_len].copy_from_slice(&data[..copy_len]);
            }
            Ok(())
        }
        AddrTarget::ModuleMp { module_idx, offset } => {
            if let Some(mp) = vm.module_mp_mut(module_idx)
                && offset < mp.len()
            {
                let copy_len = data.len().min(mp.len() - offset);
                mp[offset..offset + copy_len].copy_from_slice(&data[..copy_len]);
            }
            Ok(())
        }
        AddrTarget::Immediate => Err(ExecError::Other("cannot write to immediate".to_string())),
        AddrTarget::None => Ok(()),
        AddrTarget::HeapArray { id, offset } => {
            if let Some(obj) = vm.heap.get_mut(id) {
                match &mut obj.data {
                    HeapData::Array { data: buf, .. } | HeapData::Record(buf) => {
                        if offset < buf.len() {
                            let copy_len = data.len().min(buf.len() - offset);
                            buf[offset..offset + copy_len].copy_from_slice(&data[..copy_len]);
                        }
                    }
                    _ => {}
                }
            }
            Ok(())
        }
    }
}

fn read_table_word(vm: &VmState<'_>, base: TableBase, offset: usize) -> i32 {
    match base {
        TableBase::Frame => memory::read_word(&vm.frames.data, offset),
        TableBase::Mp => memory::read_word(&vm.mp, offset),
    }
}

fn read_table_bytes(vm: &VmState<'_>, base: TableBase, offset: usize, size: usize) -> Vec<u8> {
    let mut buf = vec![0u8; size];
    match base {
        TableBase::Frame => {
            if offset < vm.frames.data.len() {
                let copy_len = size.min(vm.frames.data.len() - offset);
                buf[..copy_len].copy_from_slice(&vm.frames.data[offset..offset + copy_len]);
            }
        }
        TableBase::Mp => {
            if offset < vm.mp.len() {
                let copy_len = size.min(vm.mp.len() - offset);
                buf[..copy_len].copy_from_slice(&vm.mp[offset..offset + copy_len]);
            }
        }
    }
    buf
}

fn write_table_bytes(vm: &mut VmState<'_>, base: TableBase, offset: usize, data: &[u8]) {
    match base {
        TableBase::Frame => {
            if offset < vm.frames.data.len() {
                let copy_len = data.len().min(vm.frames.data.len() - offset);
                vm.frames.data[offset..offset + copy_len].copy_from_slice(&data[..copy_len]);
            }
        }
        TableBase::Mp => {
            if offset < vm.mp.len() {
                let copy_len = data.len().min(vm.mp.len() - offset);
                vm.mp[offset..offset + copy_len].copy_from_slice(&data[..copy_len]);
            }
        }
    }
}

fn parse_alt_table(
    vm: &VmState<'_>,
) -> Result<(TableBase, usize, usize, Vec<AltEntry>), ExecError> {
    let (base, table_offset) = match vm.src {
        AddrTarget::Frame(off) => (TableBase::Frame, off),
        AddrTarget::Mp(off) => (TableBase::Mp, off),
        _ => {
            return Err(ExecError::Other(
                "alt table must live in frame or module memory".to_string(),
            ));
        }
    };

    // Reference layout: { nsend (word), nrecv (word), entries[] }
    // Each entry is { channel_ptr (word), data_ptr (word) } = 8 bytes
    // First nsend entries are send, next nrecv are recv.
    let nsend = read_table_word(vm, base, table_offset).max(0) as usize;
    let nrecv = read_table_word(vm, base, table_offset + 4).max(0) as usize;
    let count = nsend + nrecv;
    let mut entries = Vec::with_capacity(count);
    for idx in 0..count {
        let base_off = table_offset + 8 + idx * 8;
        let channel_id = read_table_word(vm, base, base_off) as HeapId;
        let data_offset = read_table_word(vm, base, base_off + 4).max(0) as usize;
        let is_send = idx < nsend;
        entries.push(AltEntry {
            channel_id,
            is_send,
            data_offset,
        });
    }

    Ok((base, nsend, nrecv, entries))
}

fn execute_alt(vm: &mut VmState<'_>, select_first_if_none: bool) -> Result<AltOutcome, ExecError> {
    let (base, nsend, nrecv, entries) = parse_alt_table(vm)?;
    let count = nsend + nrecv;

    // Collect ready indices first, then pick one (reference picks randomly,
    // we pick the first ready one for determinism in our cooperative model).
    for (idx, entry) in entries.iter().copied().enumerate() {
        if entry.channel_id == heap::NIL {
            continue; // skip nil channels (reference skips them in altrdy)
        }
        let (elem_size, pending) = channel_ref(vm, entry.channel_id)?;
        let ready = if entry.is_send {
            pending.is_none()
        } else {
            pending.is_some()
        };
        if !ready {
            continue;
        }

        if entry.is_send {
            let data = read_table_bytes(vm, base, entry.data_offset, elem_size);
            with_channel_mut(vm, entry.channel_id, |_, pending| {
                *pending = Some(data);
            })?;
        } else {
            let data = with_channel_mut(vm, entry.channel_id, |_, pending| {
                pending.take().unwrap_or_else(|| vec![0u8; elem_size])
            })?;
            write_table_bytes(vm, base, entry.data_offset, &data);
        }

        return Ok(AltOutcome::Selected(idx));
    }

    if select_first_if_none && count > 0 {
        Ok(AltOutcome::Selected(0))
    } else {
        Ok(AltOutcome::NoneReady)
    }
}

/// send src, dst:send data through a channel.
/// src = data to send, dst = channel pointer.
pub(crate) fn op_send(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let chan_id = vm.dst_ptr()?;
    let (elem_size, pending) = channel_ref(vm, chan_id)?;
    if pending.is_some() {
        // Channel already has data:overwrite (simplified; full impl would block sender)
    }

    let src_data = read_addr_bytes(vm, vm.src, vm.imm_src, elem_size)?;
    with_channel_mut(vm, chan_id, |_, pending| {
        *pending = Some(src_data);
    })?;

    // Unblock any threads waiting to recv on this channel
    vm.unblock_channel(chan_id);
    Ok(())
}

/// recv src, dst:receive data from a channel.
/// src = channel pointer, dst = destination for received data.
pub(crate) fn op_recv(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let chan_id = vm.src_ptr()?;
    let (_elem_size, pending) = channel_ref(vm, chan_id)?;

    if let Some(data) = pending {
        // Channel has data:consume it
        with_channel_mut(vm, chan_id, |_, pending| {
            *pending = None;
        })?;
        write_addr_bytes(vm, vm.dst, &data)
    } else {
        // Channel empty:signal the run loop to block this thread
        vm.blocked_channel = Some(chan_id);
        Ok(())
    }
}

/// alt src, dst:simplified blocking channel select.
/// The table layout is:
///   [0] = entry count
///   [1..] = triples of (channel pointer, send flag, data offset)
///
/// Send entries are ready when the single-slot channel buffer is empty.
/// Receive entries are ready when the channel has a pending payload.
/// If none are ready, this simplified implementation returns index 0.
pub(crate) fn op_alt(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    match execute_alt(vm, true)? {
        AltOutcome::Selected(idx) => vm.set_dst_word(idx as i32),
        AltOutcome::NoneReady => vm.set_dst_word(0),
    }
}

/// nbalt src, dst:simplified non-blocking channel select.
/// Returns the chosen index, or `nsend + nrecv` when no entries are ready.
pub(crate) fn op_nbalt(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let count = match parse_alt_table(vm) {
        Ok((_, nsend, nrecv, _)) => nsend + nrecv,
        Err(err) => return Err(err),
    };

    match execute_alt(vm, false)? {
        AltOutcome::Selected(idx) => vm.set_dst_word(idx as i32),
        AltOutcome::NoneReady => vm.set_dst_word(count as i32),
    }
}

#[cfg(test)]
mod tests {
    use ricevm_core::{
        Header, Instruction, MiddleOperand, Module, Opcode, Operand, PointerMap, RuntimeFlags,
        TypeDescriptor, XMAGIC,
    };

    use super::*;

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
                size: 64,
                pointer_map: PointerMap { bytes: vec![] },
                pointer_count: 0,
            }],
            data: vec![],
            name: "concurrency_test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        }
    }

    #[test]
    fn send_recv_roundtrip_word_channel() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let channel_id = vm.heap.alloc(
            0,
            HeapData::Channel {
                elem_size: 4,
                pending: None,
            },
        );
        let fp_base = vm.frames.current_data_offset();

        vm.src = AddrTarget::Immediate;
        vm.imm_src = 42;
        vm.dst = AddrTarget::Immediate;
        vm.imm_dst = channel_id as i32;
        op_send(&mut vm).expect("send should succeed");

        vm.src = AddrTarget::Immediate;
        vm.imm_src = channel_id as i32;
        vm.dst = AddrTarget::Frame(fp_base);
        op_recv(&mut vm).expect("recv should succeed");

        assert_eq!(memory::read_word(&vm.frames.data, fp_base), 42);
        match vm.heap.get(channel_id).expect("channel should exist").data {
            HeapData::Channel { ref pending, .. } => assert!(pending.is_none()),
            _ => panic!("expected channel after recv"),
        }
    }

    #[test]
    fn nbalt_selects_ready_receive_entry() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let chan_a = vm.heap.alloc(
            0,
            HeapData::Channel {
                elem_size: 4,
                pending: None,
            },
        );
        let chan_b = vm.heap.alloc(
            0,
            HeapData::Channel {
                elem_size: 4,
                pending: Some(77_i32.to_ne_bytes().to_vec()),
            },
        );
        let fp_base = vm.frames.current_data_offset();
        let table_off = fp_base + 8;

        // Reference format: { nsend, nrecv, entries[8 bytes each] }
        // Two recv entries, zero send entries
        memory::write_word(&mut vm.frames.data, table_off, 0); // nsend = 0
        memory::write_word(&mut vm.frames.data, table_off + 4, 2); // nrecv = 2
        // Entry 0 (recv): channel=chan_a, data_ptr
        memory::write_word(&mut vm.frames.data, table_off + 8, chan_a as i32);
        memory::write_word(&mut vm.frames.data, table_off + 12, (fp_base + 40) as i32);
        // Entry 1 (recv): channel=chan_b, data_ptr
        memory::write_word(&mut vm.frames.data, table_off + 16, chan_b as i32);
        memory::write_word(&mut vm.frames.data, table_off + 20, (fp_base + 44) as i32);

        vm.src = AddrTarget::Frame(table_off);
        vm.dst = AddrTarget::Frame(fp_base);
        op_nbalt(&mut vm).expect("nbalt should succeed");

        assert_eq!(memory::read_word(&vm.frames.data, fp_base), 1);
        assert_eq!(memory::read_word(&vm.frames.data, fp_base + 44), 77);
    }

    #[test]
    fn alt_performs_ready_send_entry() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let chan = vm.heap.alloc(
            0,
            HeapData::Channel {
                elem_size: 4,
                pending: None,
            },
        );
        let fp_base = vm.frames.current_data_offset();
        let table_off = fp_base + 8;

        // Reference format: { nsend=1, nrecv=0, entries[8 bytes each] }
        memory::write_word(&mut vm.frames.data, fp_base + 32, 99);
        memory::write_word(&mut vm.frames.data, table_off, 1); // nsend = 1
        memory::write_word(&mut vm.frames.data, table_off + 4, 0); // nrecv = 0
        // Entry 0 (send): channel=chan, data_ptr
        memory::write_word(&mut vm.frames.data, table_off + 8, chan as i32);
        memory::write_word(&mut vm.frames.data, table_off + 12, (fp_base + 32) as i32);

        vm.src = AddrTarget::Frame(table_off);
        vm.dst = AddrTarget::Frame(fp_base);
        op_alt(&mut vm).expect("alt should succeed");

        assert_eq!(memory::read_word(&vm.frames.data, fp_base), 0);
        match vm.heap.get(chan).expect("channel should exist").data {
            HeapData::Channel {
                ref pending,
                elem_size,
            } => {
                assert_eq!(elem_size, 4);
                assert_eq!(
                    pending.as_ref().expect("send should fill channel"),
                    &99_i32.to_ne_bytes().to_vec()
                );
            }
            _ => panic!("expected channel after alt send"),
        }
    }

    /// Regression: recv on an empty channel must set blocked_channel to signal
    /// the run loop to suspend the thread (not spin or panic).
    #[test]
    fn channel_recv_blocks_when_empty() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp_base = vm.frames.current_data_offset();

        // Create an empty channel
        let chan_id = vm.heap.alloc(
            0,
            HeapData::Channel {
                elem_size: 4,
                pending: None,
            },
        );

        // Attempt to recv from the empty channel
        vm.src = AddrTarget::Immediate;
        vm.imm_src = chan_id as i32;
        vm.dst = AddrTarget::Frame(fp_base);

        assert!(vm.blocked_channel.is_none(), "should start unblocked");
        op_recv(&mut vm).expect("recv on empty channel should not error");
        assert_eq!(
            vm.blocked_channel,
            Some(chan_id),
            "recv on empty channel should set blocked_channel"
        );
    }

    /// Regression: recv after send should consume the pending data and not block.
    #[test]
    fn channel_recv_after_send_does_not_block() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp_base = vm.frames.current_data_offset();

        let chan_id = vm.heap.alloc(
            0,
            HeapData::Channel {
                elem_size: 4,
                pending: Some(42_i32.to_ne_bytes().to_vec()),
            },
        );

        vm.src = AddrTarget::Immediate;
        vm.imm_src = chan_id as i32;
        vm.dst = AddrTarget::Frame(fp_base);

        op_recv(&mut vm).expect("recv should succeed");
        assert!(
            vm.blocked_channel.is_none(),
            "recv on full channel should not block"
        );
        assert_eq!(
            memory::read_word(&vm.frames.data, fp_base),
            42,
            "recv should deliver the sent value"
        );
    }

    /// Regression: spawn must add a new thread to thread_queue without
    /// corrupting the parent's state.
    #[test]
    fn spawn_creates_separate_thread() {
        // We need a module with at least 2 instructions:
        // PC 0: frame 0, dst -> call target
        // PC 1: exit
        let module = Module {
            header: Header {
                magic: XMAGIC,
                signature: vec![],
                runtime_flags: RuntimeFlags(0),
                stack_extent: 0,
                code_size: 2,
                data_size: 0,
                type_size: 1,
                export_size: 0,
                entry_pc: 0,
                entry_type: 0,
            },
            code: vec![
                Instruction {
                    opcode: Opcode::Exit,
                    source: Operand::UNUSED,
                    middle: MiddleOperand::UNUSED,
                    destination: Operand::UNUSED,
                },
                Instruction {
                    opcode: Opcode::Exit,
                    source: Operand::UNUSED,
                    middle: MiddleOperand::UNUSED,
                    destination: Operand::UNUSED,
                },
            ],
            types: vec![TypeDescriptor {
                id: 0,
                size: 64,
                pointer_map: PointerMap { bytes: vec![] },
                pointer_count: 0,
            }],
            data: vec![],
            name: "spawn_test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        };

        let mut vm = VmState::new(&module).expect("vm init");

        // Allocate a pending frame (simulating the 'frame' instruction)
        let pending_offset = vm.frames.alloc_pending(64).expect("alloc_pending");

        assert!(
            vm.thread_queue.is_empty(),
            "thread queue should start empty"
        );

        // Spawn: src = pending frame, dst = target PC 1
        vm.src = AddrTarget::Immediate;
        vm.imm_src = pending_offset as i32;
        vm.dst = AddrTarget::Immediate;
        vm.imm_dst = 1; // target PC

        op_spawn(&mut vm).expect("spawn should succeed");

        assert_eq!(
            vm.thread_queue.len(),
            1,
            "spawn should add exactly one thread"
        );
        let child = &vm.thread_queue[0];
        assert_eq!(child.pc, 1, "child thread should start at target PC");
    }
}
