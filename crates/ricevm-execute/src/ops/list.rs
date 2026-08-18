use ricevm_core::ExecError;

use crate::heap::{self, HeapData, HeapId};
use crate::memory;
use crate::vm::VmState;

/// Upper bound on a single list element, used to reject absurd `consm` sizes
/// before anything is allocated. Dis list elements are single Limbo values or
/// records; a megabyte is already far past anything a compiler emits.
const MAX_CONS_BYTES: usize = 1 << 20;

// --- cons operations: push a value onto the front of a list ---
// consX src, dst: dst = src :: dst
// src = value to prepend, dst = existing list pointer (modified in place)

/// Read the `size` bytes of the cons head from the source location.
///
/// Every block read is bounds-checked: the size can come straight from an
/// untrusted `consm` middle word, and the source offset from any operand.
fn read_cons_head(vm: &VmState<'_>, size: usize) -> Result<Vec<u8>, ExecError> {
    let out_of_bounds = || {
        ExecError::ThreadFault(format!(
            "cons source out of bounds: {size} bytes at {:?}",
            vm.src
        ))
    };
    let block = |buf: &[u8], off: usize| -> Result<Vec<u8>, ExecError> {
        let end = off.checked_add(size).ok_or_else(out_of_bounds)?;
        if end > buf.len() {
            return Err(out_of_bounds());
        }
        Ok(buf[off..end].to_vec())
    };

    match vm.src {
        crate::address::AddrTarget::Frame(off) => block(&vm.frames.data, off),
        crate::address::AddrTarget::Mp(off) => block(&vm.mp, off),
        crate::address::AddrTarget::ModuleMp { module_idx, offset } => {
            match vm.module_mp(module_idx) {
                Some(mp) => block(mp, offset),
                None => Ok(vec![0u8; size]),
            }
        }
        crate::address::AddrTarget::Immediate => {
            // For immediate, store the word value
            let mut head = vec![0u8; size];
            let val = vm.imm_src;
            if size >= 4 {
                memory::write_word(&mut head, 0, val);
            } else if size >= 1 {
                head[0] = val as u8;
            }
            Ok(head)
        }
        crate::address::AddrTarget::None => Ok(vec![0u8; size]),
        crate::address::AddrTarget::HeapArray { id, offset } => {
            let mut head = vec![0u8; size];
            if let Some(bytes) = vm.heap_slice(id, offset, size) {
                let n = size.min(bytes.len());
                head[..n].copy_from_slice(&bytes[..n]);
            }
            Ok(head)
        }
    }
}

fn cons_bytes(vm: &mut VmState<'_>, size: usize) -> Result<(), ExecError> {
    let tail_id = vm.dst_ptr()?;
    let head = read_cons_head(vm, size)?;

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
    // The block size is untrusted: a negative word would become a ~1.8e19 byte
    // allocation, and even a large positive one is never a real record.
    let size = vm.mid_word()?;
    if size < 0 {
        return Err(ExecError::ThreadFault(format!(
            "consm: negative block size: {size}"
        )));
    }
    let size = size as usize;
    if size > MAX_CONS_BYTES {
        return Err(ExecError::ThreadFault(format!(
            "consm: block size too large: {size}"
        )));
    }
    cons_bytes(vm, size)
}

/// consmp: cons a record that contains pointers.
///
/// Unlike `consm`, whose middle operand is a plain byte count, this one names a
/// *type* -- and the copy it makes is a new reference to every pointer the
/// record holds, so each has to be counted before the bytes are moved
/// (`incmem(R.s, t)` in the reference, libinterp/xec.c). Skipping that leaves
/// the list holding pointers nobody counted, and whichever operation releases
/// them first frees an object that is still in use.
pub(crate) fn op_consmp(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let type_idx = vm.mid_word()? as usize;
    // An unknown type has no map and no size; fall back to treating the operand
    // as a byte count, exactly as the untyped `movm`/`consm` pair does.
    let size = vm.current_type_size(type_idx).unwrap_or(type_idx);
    if size > MAX_CONS_BYTES {
        return Err(ExecError::ThreadFault(format!(
            "consmp: block size too large: {size}"
        )));
    }
    if let Some(ptr_map) = vm.trace_map_for_type(type_idx) {
        let head = read_cons_head(vm, size)?;
        for offset in ptr_map.pointer_offsets(size) {
            let ptr_val = memory::read_word(&head, offset) as u32;
            if ptr_val >= heap::HEAP_ID_BASE && vm.heap.contains(ptr_val) {
                vm.heap.inc_ref(ptr_val);
            }
        }
    }
    cons_bytes(vm, size)
}

// --- head operations: extract the head value from a list ---
// headX src, dst: dst = hd(src)

fn head_read<'a>(vm: &'a VmState<'_>, list_id: HeapId) -> Result<&'a [u8], ExecError> {
    if list_id == heap::NIL {
        // Head of nil: return empty slice (graceful)
        return Ok(&[]);
    }
    // A non-nil id with nothing behind it is a dangling reference, not a nil
    // one -- the list was released while this slot still named it. Say so, and
    // name the id: calling it a nil dereference sends the reader looking at the
    // program's own logic instead of at the heap.
    let obj = vm
        .heap
        .get(list_id)
        .ok_or_else(|| ExecError::ThreadFault(format!("head of a released list (id {list_id})")))?;
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

    /// `consmp` conses a record that contains pointers, so it has to take a
    /// reference for each one -- the reference calls `incmem(R.s, t)` before
    /// copying (libinterp/xec.c). Without it the list holds pointers nobody
    /// counted, and the first operation to release them (a `movmp` writing over
    /// that field, say) frees an object that is still in use. Its middle
    /// operand is a *type index*, not the byte count `consm` takes.
    #[test]
    fn consmp_takes_a_reference_for_every_pointer_it_copies() {
        let mut module = test_module();
        // Type 1: an 8-byte record whose second word is a pointer. Pointer maps
        // are most-significant-bit first, so word 1 is 0x40.
        module.types.push(TypeDescriptor {
            id: 1,
            size: 8,
            pointer_map: PointerMap { bytes: vec![0x40] },
            pointer_count: 1,
        });
        let mut vm = VmState::new(&module).expect("vm init");

        let held = vm
            .heap
            .alloc(0, HeapData::Str("still referenced".to_string()));
        let before = vm.heap.get(held).expect("just allocated").ref_count;

        // A record at fp+0: a plain word, then the pointer.
        let fp = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, fp, 1234);
        memory::write_word(&mut vm.frames.data, fp + 4, held as i32);

        vm.src = AddrTarget::Frame(fp);
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 1; // type index, not a byte count
        vm.dst = AddrTarget::Frame(fp + 16);
        op_consmp(&mut vm).expect("consmp should succeed");

        let after = vm.heap.get(held).expect("must still be live").ref_count;
        assert_eq!(
            after,
            before + 1,
            "the copy in the list node is a new reference"
        );
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
    fn consw_out_of_range_frame_source_is_rejected() {
        // The source read must be bounds-checked like every other block read.
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);
        vm.src = AddrTarget::Frame(vm.frames.data.len() - 2); // only 2 bytes left
        vm.dst = AddrTarget::Frame(fp);

        let err = op_consw(&mut vm).expect_err("out-of-range cons source must be rejected");
        assert!(
            err.to_string().contains("out of bounds"),
            "expected out of bounds, got: {err}"
        );
    }

    #[test]
    fn consm_negative_size_is_rejected() {
        // A negative mid word must not become a ~1.8e19 byte allocation.
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);
        vm.src = AddrTarget::Frame(fp);
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = -1;
        vm.dst = AddrTarget::Frame(fp);

        let err = op_consm(&mut vm).expect_err("negative cons size must be rejected");
        assert!(
            err.to_string().contains("negative"),
            "expected negative size error, got: {err}"
        );
    }

    #[test]
    fn consm_oversized_size_is_rejected() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);
        vm.src = AddrTarget::Frame(fp);
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = i32::MAX;
        vm.dst = AddrTarget::Frame(fp);

        let err = op_consm(&mut vm).expect_err("oversized cons size must be rejected");
        assert!(
            err.to_string().contains("too large"),
            "expected size error, got: {err}"
        );
    }

    #[test]
    fn consm_copies_block_from_frame() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        memory::write_word(&mut vm.frames.data, fp, 7);
        memory::write_word(&mut vm.frames.data, fp + 4, 8);
        memory::write_word(&mut vm.frames.data, fp + 8, heap::NIL as i32);
        vm.src = AddrTarget::Frame(fp);
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 8;
        vm.dst = AddrTarget::Frame(fp + 8);

        op_consm(&mut vm).expect("consm should succeed");

        let list_id = memory::read_word(&vm.frames.data, fp + 8) as HeapId;
        let obj = vm.heap.get(list_id).expect("list should exist");
        match &obj.data {
            HeapData::List { head, .. } => {
                assert_eq!(head.len(), 8);
                assert_eq!(memory::read_word(head, 0), 7);
                assert_eq!(memory::read_word(head, 4), 8);
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
