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
        crate::address::AddrTarget::ModuleMp { module_idx, offset } => {
            if let Some(mp) = vm.module_mp(module_idx)
                && offset + size <= mp.len()
            {
                head.copy_from_slice(&mp[offset..offset + size]);
            }
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
        crate::address::AddrTarget::HeapArray { id, offset } => {
            if let Some(bytes) = vm.heap_slice(id, offset, size) {
                head[..size].copy_from_slice(&bytes[..size]);
            }
        }
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
    if list_id == heap::NIL {
        // Head of nil: return empty slice (graceful)
        return Ok(&[]);
    }
    let obj = vm
        .heap
        .get(list_id)
        .ok_or_else(|| ExecError::ThreadFault("nil list dereference (head)".to_string()))?;
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
        crate::address::AddrTarget::ModuleMp { module_idx, offset } => {
            if let Some(mp) = vm.module_mp_mut(module_idx) {
                let end = offset + head.len();
                if end <= mp.len() {
                    mp[offset..end].copy_from_slice(&head);
                }
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
    if list_id == heap::NIL {
        // Tail of nil is nil (graceful handling)
        return vm.move_ptr_to_dst(heap::NIL);
    }
    let obj = vm
        .heap
        .get(list_id)
        .ok_or_else(|| ExecError::ThreadFault("nil list dereference (tail)".to_string()))?;
    let tail_id = match &obj.data {
        HeapData::List { tail, .. } => *tail,
        _ => return Err(ExecError::ThreadFault("tail on non-list".to_string())),
    };
    vm.move_ptr_to_dst(tail_id)
}

#[cfg(test)]
mod tests {
    use ricevm_core::{
        Header, Instruction, MiddleOperand, Module, Opcode, Operand, PointerMap, RuntimeFlags,
        TypeDescriptor, XMAGIC,
    };

    use super::*;
    use crate::address::AddrTarget;

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
            name: "list_test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        }
    }

    #[test]
    fn consw_creates_single_element_list() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        // dst starts as NIL (empty list)
        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);
        vm.src = AddrTarget::Immediate;
        vm.imm_src = 42;
        vm.dst = AddrTarget::Frame(fp);

        op_consw(&mut vm).expect("consw should succeed");

        let list_id = memory::read_word(&vm.frames.data, fp) as HeapId;
        assert_ne!(list_id, heap::NIL);

        let obj = vm.heap.get(list_id).expect("list should exist");
        match &obj.data {
            HeapData::List { head, tail } => {
                assert_eq!(memory::read_word(head, 0), 42);
                assert_eq!(*tail, heap::NIL);
            }
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn consw_prepends_to_existing_list() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        // Build list [10] first
        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);
        vm.src = AddrTarget::Immediate;
        vm.imm_src = 10;
        vm.dst = AddrTarget::Frame(fp);
        op_consw(&mut vm).expect("consw should succeed");

        // Prepend 20 -> [20, 10]
        vm.src = AddrTarget::Immediate;
        vm.imm_src = 20;
        vm.dst = AddrTarget::Frame(fp);
        op_consw(&mut vm).expect("consw should succeed");

        let list_id = memory::read_word(&vm.frames.data, fp) as HeapId;
        let obj = vm.heap.get(list_id).expect("list should exist");
        match &obj.data {
            HeapData::List { head, tail } => {
                assert_eq!(memory::read_word(head, 0), 20);
                assert_ne!(*tail, heap::NIL);
            }
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn consp_creates_list_of_pointers() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        let str_id = vm.heap.alloc(0, HeapData::Str("hello".to_string()));

        // dst = NIL (empty list)
        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);
        vm.src = AddrTarget::Immediate;
        vm.imm_src = str_id as i32;
        vm.dst = AddrTarget::Frame(fp);

        op_consp(&mut vm).expect("consp should succeed");

        let list_id = memory::read_word(&vm.frames.data, fp) as HeapId;
        assert_ne!(list_id, heap::NIL);

        let obj = vm.heap.get(list_id).expect("list should exist");
        match &obj.data {
            HeapData::List { head, tail } => {
                assert_eq!(memory::read_word(head, 0) as HeapId, str_id);
                assert_eq!(*tail, heap::NIL);
            }
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn headw_extracts_head_value() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        let list_id = vm.heap.alloc(
            0,
            HeapData::List {
                head: vec![99, 0, 0, 0],
                tail: heap::NIL,
            },
        );

        vm.src = AddrTarget::Immediate;
        vm.imm_src = list_id as i32;
        vm.dst = AddrTarget::Frame(fp);

        op_headw(&mut vm).expect("headw should succeed");
        assert_eq!(memory::read_word(&vm.frames.data, fp), 99);
    }

    #[test]
    fn headp_extracts_pointer_head() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        let str_id = vm.heap.alloc(0, HeapData::Str("world".to_string()));
        let mut head_bytes = vec![0u8; 4];
        memory::write_word(&mut head_bytes, 0, str_id as i32);

        let list_id = vm.heap.alloc(
            0,
            HeapData::List {
                head: head_bytes,
                tail: heap::NIL,
            },
        );

        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);
        vm.src = AddrTarget::Immediate;
        vm.imm_src = list_id as i32;
        vm.dst = AddrTarget::Frame(fp);

        op_headp(&mut vm).expect("headp should succeed");
        assert_eq!(memory::read_word(&vm.frames.data, fp) as HeapId, str_id);
    }

    #[test]
    fn headw_of_nil_returns_zero() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        vm.src = AddrTarget::Immediate;
        vm.imm_src = heap::NIL as i32;
        vm.dst = AddrTarget::Frame(fp);

        op_headw(&mut vm).expect("headw of nil should succeed");
        assert_eq!(memory::read_word(&vm.frames.data, fp), 0);
    }

    #[test]
    fn headb_extracts_byte_head() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        let list_id = vm.heap.alloc(
            0,
            HeapData::List {
                head: vec![0xAB],
                tail: heap::NIL,
            },
        );

        vm.src = AddrTarget::Immediate;
        vm.imm_src = list_id as i32;
        vm.dst = AddrTarget::Frame(fp);

        op_headb(&mut vm).expect("headb should succeed");
        assert_eq!(vm.frames.data[fp], 0xAB);
    }

    #[test]
    fn tail_advances_to_next_node() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        let second = vm.heap.alloc(
            0,
            HeapData::List {
                head: vec![2, 0, 0, 0],
                tail: heap::NIL,
            },
        );
        let first = vm.heap.alloc(
            0,
            HeapData::List {
                head: vec![1, 0, 0, 0],
                tail: second,
            },
        );

        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);
        vm.src = AddrTarget::Immediate;
        vm.imm_src = first as i32;
        vm.dst = AddrTarget::Frame(fp);

        op_tail(&mut vm).expect("tail should succeed");
        let result = memory::read_word(&vm.frames.data, fp) as HeapId;
        assert_eq!(result, second);
    }

    #[test]
    fn tail_of_nil_returns_nil() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);
        vm.src = AddrTarget::Immediate;
        vm.imm_src = heap::NIL as i32;
        vm.dst = AddrTarget::Frame(fp);

        op_tail(&mut vm).expect("tail of nil should succeed");
        assert_eq!(memory::read_word(&vm.frames.data, fp) as HeapId, heap::NIL);
    }

    #[test]
    fn tail_of_single_element_returns_nil() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        let single = vm.heap.alloc(
            0,
            HeapData::List {
                head: vec![1, 0, 0, 0],
                tail: heap::NIL,
            },
        );

        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);
        vm.src = AddrTarget::Immediate;
        vm.imm_src = single as i32;
        vm.dst = AddrTarget::Frame(fp);

        op_tail(&mut vm).expect("tail should succeed");
        assert_eq!(memory::read_word(&vm.frames.data, fp) as HeapId, heap::NIL);
    }

    #[test]
    fn consb_creates_byte_list() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);
        vm.src = AddrTarget::Immediate;
        vm.imm_src = 0x42;
        vm.dst = AddrTarget::Frame(fp);

        op_consb(&mut vm).expect("consb should succeed");

        let list_id = memory::read_word(&vm.frames.data, fp) as HeapId;
        let obj = vm.heap.get(list_id).expect("list should exist");
        match &obj.data {
            HeapData::List { head, tail } => {
                assert_eq!(head.len(), 1);
                assert_eq!(head[0], 0x42);
                assert_eq!(*tail, heap::NIL);
            }
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn roundtrip_consw_headw_tail() {
        // Build [30, 20, 10], then extract head+tail to verify structure
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        // Start with NIL
        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);

        // cons 10 -> [10]
        vm.src = AddrTarget::Immediate;
        vm.imm_src = 10;
        vm.dst = AddrTarget::Frame(fp);
        op_consw(&mut vm).expect("consw 10");

        // cons 20 -> [20, 10]
        vm.imm_src = 20;
        op_consw(&mut vm).expect("consw 20");

        // cons 30 -> [30, 20, 10]
        vm.imm_src = 30;
        op_consw(&mut vm).expect("consw 30");

        // headw -> 30
        let list_id = memory::read_word(&vm.frames.data, fp) as HeapId;
        let fp2 = fp + 4;
        vm.src = AddrTarget::Immediate;
        vm.imm_src = list_id as i32;
        vm.dst = AddrTarget::Frame(fp2);
        op_headw(&mut vm).expect("headw");
        assert_eq!(memory::read_word(&vm.frames.data, fp2), 30);

        // tail -> rest = [20, 10]
        let fp3 = fp + 8;
        memory::write_word(&mut vm.frames.data, fp3, heap::NIL as i32);
        vm.src = AddrTarget::Immediate;
        vm.imm_src = list_id as i32;
        vm.dst = AddrTarget::Frame(fp3);
        op_tail(&mut vm).expect("tail");
        let rest_id = memory::read_word(&vm.frames.data, fp3) as HeapId;
        assert_ne!(rest_id, heap::NIL);

        // headw of rest -> 20
        vm.src = AddrTarget::Immediate;
        vm.imm_src = rest_id as i32;
        vm.dst = AddrTarget::Frame(fp2);
        op_headw(&mut vm).expect("headw of rest");
        assert_eq!(memory::read_word(&vm.frames.data, fp2), 20);
    }
}
