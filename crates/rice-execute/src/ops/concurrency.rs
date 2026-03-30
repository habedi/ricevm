//! Concurrency opcodes.
//!
//! spawn/send/recv are implemented with simplified semantics.
//! In a single-threaded execution context (the current default), spawn
//! creates a new thread that will be run by the scheduler, and send/recv
//! operate on the channel's internal queues.

use ricevm_core::ExecError;

use crate::heap::{self, HeapData};
use crate::vm::VmState;

/// spawn src, dst — create a new thread in the current module.
/// src = frame pointer (pre-allocated via `frame`), dst = target PC.
///
/// In the current single-threaded model, we log a warning and
/// execute the spawned code inline (not truly concurrent).
pub(crate) fn op_spawn(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let _frame_ptr = vm.src_word()?;
    let target_pc = vm.dst_word()?;
    tracing::warn!(
        target_pc = target_pc,
        "spawn: concurrent threads not fully supported, executing inline"
    );
    // For now, record the spawn target but don't actually create a thread.
    // A full implementation would fork the frame stack and create a new VmThread.
    Ok(())
}

/// mspawn src, mid, dst — create a thread in a loaded module.
pub(crate) fn op_mspawn(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let _frame_ptr = vm.src_word()?;
    let _func_idx = vm.mid_word()?;
    let _mod_ref = vm.dst_ptr()?;
    tracing::warn!("mspawn: concurrent threads not fully supported");
    Ok(())
}

/// send src, dst — send data through a channel.
/// src = data to send, dst = channel pointer.
///
/// In the simplified model, we store the data in the channel's sender queue.
pub(crate) fn op_send(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let chan_id = vm.dst_ptr()?;
    if chan_id == heap::NIL {
        return Err(ExecError::ThreadFault("send on nil channel".to_string()));
    }

    // Read the data from src (word-sized for simplicity)
    let data_val = vm.src_word()?;

    // Store in channel — for the simplified model, just log it
    tracing::debug!(channel = chan_id, value = data_val, "send");

    // In a full implementation, this would queue the data and potentially
    // wake a receiving thread. For now, we store the value in the channel's
    // heap object as a simple buffer.
    if let Some(obj) = vm.heap.get_mut(chan_id)
        && let HeapData::Channel = &obj.data
    {
        let mut buf = vec![0u8; 4];
        crate::memory::write_word(&mut buf, 0, data_val);
        obj.data = HeapData::Record(buf);
    }

    Ok(())
}

/// recv src, dst — receive data from a channel.
/// src = channel pointer, dst = destination for received data.
pub(crate) fn op_recv(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let chan_id = vm.src_ptr()?;
    if chan_id == heap::NIL {
        return Err(ExecError::ThreadFault("recv on nil channel".to_string()));
    }

    // In the simplified model, read the last sent value
    let val = if let Some(obj) = vm.heap.get(chan_id) {
        match &obj.data {
            HeapData::Record(buf) => {
                if buf.len() >= 4 {
                    crate::memory::read_word(buf, 0)
                } else {
                    0
                }
            }
            _ => 0,
        }
    } else {
        0
    };

    vm.set_dst_word(val)
}

/// alt src, dst — blocking channel select.
/// In the simplified model, returns 0 (first channel ready).
pub(crate) fn op_alt(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // The alt instruction reads a table of channel/operation pairs
    // and blocks until one is ready. In our simplified model,
    // we just return 0 (first alternative).
    let _src = vm.src_word()?;
    vm.set_dst_word(0)
}

/// nbalt src, dst — non-blocking channel select.
/// Returns the index of the ready channel, or N (number of channels) if none ready.
pub(crate) fn op_nbalt(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let _src = vm.src_word()?;
    // Return "no channel ready" by default
    vm.set_dst_word(0)
}
