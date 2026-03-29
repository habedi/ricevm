use ricevm_core::ExecError;

use crate::heap::{self, HeapData, HeapId};
use crate::memory;
use crate::vm::VmState;

// --- cons operations: push a value onto the front of a list ---
// consX src, dst: dst = src :: dst
// src = value to prepend, dst = existing list pointer (modified in place)

fn cons_bytes(vm: &mut VmState<'_>, size: usize) -> Result<(), ExecError> {
    let tail_id = vm.dst_ptr()?;
    // Read `size` bytes from the source location
    let mut head = vec![0u8; size];
    match vm.src {
        crate::address::AddrTarget::Frame(off) => {
            head.copy_from_slice(&vm.frames.data[off..off + size]);
        }
        crate::address::AddrTarget::Mp(off) => {
            head.copy_from_slice(&vm.mp[off..off + size]);
        }
        crate::address::AddrTarget::Immediate => {
            // For immediate, store the word value
            let val = vm.imm_src;
            if size >= 4 {
                memory::write_word(&mut head, 0, val);
            } else {
                head[0] = val as u8;
            }
        }
        crate::address::AddrTarget::None => {}
    }

    if tail_id != heap::NIL {
        vm.heap.inc_ref(tail_id);
    }
    let list_id = vm.heap.alloc(
        0,
        HeapData::List {
            head,
            tail: tail_id,
        },
    );
    // dec_ref old dst, set new
    let old_id = vm.dst_ptr()?;
    vm.set_dst_ptr(list_id)?;
    if old_id != heap::NIL {
        vm.heap.dec_ref(old_id);
    }
    Ok(())
}

pub(crate) fn op_consb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    cons_bytes(vm, 1)
}

pub(crate) fn op_consw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    cons_bytes(vm, 4)
}

pub(crate) fn op_consf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    cons_bytes(vm, 8)
}

pub(crate) fn op_consl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    cons_bytes(vm, 8)
}

/// consp: cons a pointer (HeapId). The head stores the HeapId as 4 bytes.
pub(crate) fn op_consp(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let ptr_id = vm.src_ptr()?;
    let tail_id = vm.dst_ptr()?;

    // Inc ref the pointer being stored as head
    if ptr_id != heap::NIL {
        vm.heap.inc_ref(ptr_id);
    }
    if tail_id != heap::NIL {
        vm.heap.inc_ref(tail_id);
    }

    let mut head = vec![0u8; 4];
    memory::write_word(&mut head, 0, ptr_id as i32);

    let list_id = vm.heap.alloc(
        0,
        HeapData::List {
            head,
            tail: tail_id,
        },
    );
    let old_id = vm.dst_ptr()?;
    vm.set_dst_ptr(list_id)?;
    if old_id != heap::NIL {
        vm.heap.dec_ref(old_id);
    }
    Ok(())
}

/// consm: cons a memory block (record). Size comes from mid operand.
pub(crate) fn op_consm(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let size = vm.mid_word()? as usize;
    cons_bytes(vm, size)
}

/// consmp: cons a memory block with pointers. Same as consm for now.
pub(crate) fn op_consmp(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    op_consm(vm)
}

// --- head operations: extract the head value from a list ---
// headX src, dst: dst = hd(src)

fn head_read<'a>(vm: &'a VmState<'_>, list_id: HeapId) -> Result<&'a [u8], ExecError> {
    let obj = vm.heap.get(list_id).ok_or_else(|| {
        ExecError::ThreadFault("nil list dereference (head)".to_string())
    })?;
    match &obj.data {
        HeapData::List { head, .. } => Ok(head.as_slice()),
        _ => Err(ExecError::ThreadFault("head on non-list".to_string())),
    }
}

pub(crate) fn op_headb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let list_id = vm.src_ptr()?;
    let head = head_read(vm, list_id)?;
    let val = if head.is_empty() { 0 } else { head[0] };
    vm.set_dst_byte(val)
}

pub(crate) fn op_headw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let list_id = vm.src_ptr()?;
    let head = head_read(vm, list_id)?;
    let val = if head.len() >= 4 {
        memory::read_word(head, 0)
    } else {
        0
    };
    vm.set_dst_word(val)
}

pub(crate) fn op_headf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let list_id = vm.src_ptr()?;
    let head = head_read(vm, list_id)?;
    let val = if head.len() >= 8 {
        memory::read_real(head, 0)
    } else {
        0.0
    };
    vm.set_dst_real(val)
}

pub(crate) fn op_headl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let list_id = vm.src_ptr()?;
    let head = head_read(vm, list_id)?;
    let val = if head.len() >= 8 {
        memory::read_big(head, 0)
    } else {
        0
    };
    vm.set_dst_big(val)
}

pub(crate) fn op_headp(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let list_id = vm.src_ptr()?;
    let head = head_read(vm, list_id)?;
    let ptr_id = if head.len() >= 4 {
        memory::read_word(head, 0) as HeapId
    } else {
        heap::NIL
    };
    vm.move_ptr_to_dst(ptr_id)
}

/// headm: extract record head. Copies bytes into dst.
pub(crate) fn op_headm(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let list_id = vm.src_ptr()?;
    let head = head_read(vm, list_id)?.to_vec();
    // Write head bytes to dst location
    match vm.dst {
        crate::address::AddrTarget::Frame(off) => {
            let end = off + head.len();
            if end <= vm.frames.data.len() {
                vm.frames.data[off..end].copy_from_slice(&head);
            }
        }
        crate::address::AddrTarget::Mp(off) => {
            let end = off + head.len();
            if end <= vm.mp.len() {
                vm.mp[off..end].copy_from_slice(&head);
            }
        }
        _ => {}
    }
    Ok(())
}

/// headmp: same as headm (with pointer tracking, skipped for now)
pub(crate) fn op_headmp(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    op_headm(vm)
}

/// tail src, dst: dst = tl(src)
pub(crate) fn op_tail(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let list_id = vm.src_ptr()?;
    let obj = vm.heap.get(list_id).ok_or_else(|| {
        ExecError::ThreadFault("nil list dereference (tail)".to_string())
    })?;
    let tail_id = match &obj.data {
        HeapData::List { tail, .. } => *tail,
        _ => return Err(ExecError::ThreadFault("tail on non-list".to_string())),
    };
    vm.move_ptr_to_dst(tail_id)
}
