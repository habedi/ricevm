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

/// Read the `size` bytes a record copy is about to overwrite, so the pointers
/// among them can be released.
///
/// Only the locations `op_headm` writes are read back. Releasing the pointers at
/// a location the copy never reaches would free an object that is still in use,
/// so anything else answers `None` and the counting is skipped altogether. A
/// location that is out of range reads as zeroes, which name no object.
fn read_dst_block(vm: &VmState<'_>, size: usize) -> Option<Vec<u8>> {
    let block = |buf: &[u8], off: usize| -> Vec<u8> {
        match off.checked_add(size) {
            Some(end) if end <= buf.len() => buf[off..end].to_vec(),
            _ => vec![0u8; size],
        }
    };
    match vm.dst {
        crate::address::AddrTarget::Frame(off) => Some(block(&vm.frames.data, off)),
        crate::address::AddrTarget::Mp(off) => Some(block(&vm.mp, off)),
        crate::address::AddrTarget::ModuleMp { module_idx, offset } => {
            Some(match vm.module_mp(module_idx) {
                Some(mp) => block(mp, offset),
                None => vec![0u8; size],
            })
        }
        _ => None,
    }
}

/// headmp: extract the head of a list of records that hold pointers.
///
/// The copy is the same one `headm` makes, but the reference does it through
/// `movmp` (`OP(headmp) { l = L(s); R.s = l->data; movmp(); }`,
/// libinterp/xec.c), which counts what it moves: `incmem` takes a reference for
/// every pointer the record holds, and `freeptrs` releases every pointer the
/// copy overwrites. Without that the destination holds pointers nobody counted,
/// and whichever operation releases them first frees an object still in use.
/// Its middle operand is a *type* index, as `movmp` and `consmp` take.
pub(crate) fn op_headmp(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let type_idx = vm.mid_word()? as usize;
    // An unknown type has no map and no size; fall back to the operand as a byte
    // count, which carries no pointers to count either.
    let size = vm.current_type_size(type_idx).unwrap_or(type_idx);
    if let Some(ptr_map) = vm.trace_map_for_type(type_idx)
        && let Some(overwritten) = read_dst_block(vm, size)
    {
        let list_id = vm.src_ptr()?;
        let head = head_read(vm, list_id)?.to_vec();
        // Only the words the copy really moves change hands. A head shorter than
        // the type says, and the empty head of a nil list, leave the fields past
        // its end alone, so their pointers are neither counted nor released.
        let moved: Vec<usize> = ptr_map
            .pointer_offsets(size)
            .filter(|offset| offset + 4 <= head.len())
            .collect();
        // incmem: the copy is a new reference to every pointer the record holds.
        for &offset in &moved {
            let ptr_val = memory::read_word(&head, offset) as u32;
            if ptr_val >= heap::HEAP_ID_BASE && vm.heap.contains(ptr_val) {
                vm.heap.inc_ref(ptr_val);
            }
        }
        // freeptrs: release what the copy overwrites, after the increments above,
        // so copying a record onto itself cannot drop a field's last reference
        // in the middle of the instruction.
        for &offset in &moved {
            let ptr_val = memory::read_word(&overwritten, offset) as u32;
            if ptr_val >= heap::HEAP_ID_BASE && vm.heap.contains(ptr_val) {
                vm.heap.dec_ref(ptr_val);
            }
        }
    }
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

    // --- cons and head, one element width at a time, through the dispatch table ---
    //
    // Each case builds a one-element list with the cons opcode for that width
    // and reads it back with the matching head opcode, both through
    // `ops::dispatch`. Routing through dispatch means an arm wired to the
    // handler for a different width fails here rather than in a real program.
    // The element sizes follow the reference: `IBY2WD` for a word, `IBY2LG` for
    // a big, `sizeof(REAL)` for a real, and a pointer-sized cell for `consp`
    // (libinterp/xec.c).

    /// Frame offset of the list pointer, relative to the frame's data area.
    const LIST_SLOT: usize = 32;
    /// Frame offset of the value a head opcode writes, relative to the data area.
    const DEST_SLOT: usize = 40;

    fn list_inst(opcode: Opcode) -> Instruction {
        Instruction {
            opcode,
            source: Operand::UNUSED,
            middle: MiddleOperand::UNUSED,
            destination: Operand::UNUSED,
        }
    }

    fn dispatch_op(vm: &mut VmState<'_>, opcode: Opcode) -> Result<(), ExecError> {
        crate::ops::dispatch(vm, &list_inst(opcode))
    }

    #[test]
    fn consb_and_headb_carry_a_byte() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, fp + LIST_SLOT, heap::NIL as i32);

        vm.src = AddrTarget::Immediate;
        vm.imm_src = 0xA5;
        vm.dst = AddrTarget::Frame(fp + LIST_SLOT);
        dispatch_op(&mut vm, Opcode::Consb).expect("consb should succeed");

        vm.src = AddrTarget::Frame(fp + LIST_SLOT);
        vm.dst = AddrTarget::Frame(fp + DEST_SLOT);
        dispatch_op(&mut vm, Opcode::Headb).expect("headb should succeed");
        assert_eq!(vm.frames.data[fp + DEST_SLOT], 0xA5);
    }

    #[test]
    fn consw_and_headw_carry_a_signed_word() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, fp + LIST_SLOT, heap::NIL as i32);

        vm.src = AddrTarget::Immediate;
        vm.imm_src = -70_000;
        vm.dst = AddrTarget::Frame(fp + LIST_SLOT);
        dispatch_op(&mut vm, Opcode::Consw).expect("consw should succeed");

        vm.src = AddrTarget::Frame(fp + LIST_SLOT);
        vm.dst = AddrTarget::Frame(fp + DEST_SLOT);
        dispatch_op(&mut vm, Opcode::Headw).expect("headw should succeed");
        assert_eq!(memory::read_word(&vm.frames.data, fp + DEST_SLOT), -70_000);
    }

    #[test]
    fn consf_and_headf_carry_a_real() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, fp, -2.5);
        memory::write_word(&mut vm.frames.data, fp + LIST_SLOT, heap::NIL as i32);

        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + LIST_SLOT);
        dispatch_op(&mut vm, Opcode::Consf).expect("consf should succeed");

        let list_id = memory::read_word(&vm.frames.data, fp + LIST_SLOT) as HeapId;
        match &vm.heap.get(list_id).expect("list should exist").data {
            HeapData::List { head, .. } => assert_eq!(head.len(), 8, "a real cell is eight bytes"),
            _ => panic!("expected List"),
        }

        vm.src = AddrTarget::Frame(fp + LIST_SLOT);
        vm.dst = AddrTarget::Frame(fp + DEST_SLOT);
        dispatch_op(&mut vm, Opcode::Headf).expect("headf should succeed");
        assert_eq!(memory::read_real(&vm.frames.data, fp + DEST_SLOT), -2.5);
    }

    #[test]
    fn consl_and_headl_carry_all_sixty_four_bits() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        // A value with bits above the low word, so a 32-bit copy would lose it.
        let value = (1_i64 << 40) + 7;
        memory::write_big(&mut vm.frames.data, fp, value);
        memory::write_word(&mut vm.frames.data, fp + LIST_SLOT, heap::NIL as i32);

        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + LIST_SLOT);
        dispatch_op(&mut vm, Opcode::Consl).expect("consl should succeed");

        vm.src = AddrTarget::Frame(fp + LIST_SLOT);
        vm.dst = AddrTarget::Frame(fp + DEST_SLOT);
        dispatch_op(&mut vm, Opcode::Headl).expect("headl should succeed");
        assert_eq!(memory::read_big(&vm.frames.data, fp + DEST_SLOT), value);
    }

    #[test]
    fn consp_and_headp_carry_a_heap_id() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        let str_id = vm.heap.alloc(0, HeapData::Str("payload".to_string()));
        memory::write_word(&mut vm.frames.data, fp, str_id as i32);
        memory::write_word(&mut vm.frames.data, fp + LIST_SLOT, heap::NIL as i32);
        memory::write_word(&mut vm.frames.data, fp + DEST_SLOT, heap::NIL as i32);

        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + LIST_SLOT);
        dispatch_op(&mut vm, Opcode::Consp).expect("consp should succeed");

        vm.src = AddrTarget::Frame(fp + LIST_SLOT);
        vm.dst = AddrTarget::Frame(fp + DEST_SLOT);
        dispatch_op(&mut vm, Opcode::Headp).expect("headp should succeed");
        assert_eq!(
            memory::read_word(&vm.frames.data, fp + DEST_SLOT) as HeapId,
            str_id
        );
    }

    #[test]
    fn consm_and_headm_carry_a_whole_record() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, fp, 0x1111_2222);
        memory::write_word(&mut vm.frames.data, fp + 4, 0x3333_4444);
        memory::write_word(&mut vm.frames.data, fp + LIST_SLOT, heap::NIL as i32);

        vm.src = AddrTarget::Frame(fp);
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 8; // Byte count, which is what consm takes.
        vm.dst = AddrTarget::Frame(fp + LIST_SLOT);
        dispatch_op(&mut vm, Opcode::Consm).expect("consm should succeed");

        vm.src = AddrTarget::Frame(fp + LIST_SLOT);
        vm.dst = AddrTarget::Frame(fp + DEST_SLOT);
        dispatch_op(&mut vm, Opcode::Headm).expect("headm should succeed");
        assert_eq!(
            memory::read_word(&vm.frames.data, fp + DEST_SLOT),
            0x1111_2222
        );
        assert_eq!(
            memory::read_word(&vm.frames.data, fp + DEST_SLOT + 4),
            0x3333_4444
        );
    }

    /// A record with a pointer in it: `consmp` and `headmp` name a type index,
    /// not a byte count, and the type descriptor's pointer map says which words
    /// hold pointers.
    #[test]
    fn consmp_and_headmp_carry_a_record_that_holds_a_pointer() {
        let mut module = test_module();
        module.types.push(TypeDescriptor {
            id: 1,
            size: 8,
            pointer_map: PointerMap { bytes: vec![0x40] },
            pointer_count: 1,
        });
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        let str_id = vm.heap.alloc(0, HeapData::Str("field".to_string()));
        memory::write_word(&mut vm.frames.data, fp, 99);
        memory::write_word(&mut vm.frames.data, fp + 4, str_id as i32);
        memory::write_word(&mut vm.frames.data, fp + LIST_SLOT, heap::NIL as i32);

        vm.src = AddrTarget::Frame(fp);
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 1; // Type index.
        vm.dst = AddrTarget::Frame(fp + LIST_SLOT);
        dispatch_op(&mut vm, Opcode::Consmp).expect("consmp should succeed");

        vm.src = AddrTarget::Frame(fp + LIST_SLOT);
        vm.dst = AddrTarget::Frame(fp + DEST_SLOT);
        dispatch_op(&mut vm, Opcode::Headmp).expect("headmp should succeed");
        assert_eq!(memory::read_word(&vm.frames.data, fp + DEST_SLOT), 99);
        assert_eq!(
            memory::read_word(&vm.frames.data, fp + DEST_SLOT + 4) as HeapId,
            str_id
        );
    }

    /// `tail` reached through the dispatch table, walking a two-element list to
    /// its end. The reference takes the address of the node's tail field and
    /// runs `movp` (libinterp/xec.c), so the destination ends up naming the
    /// next node and then nil.
    #[test]
    fn tail_walks_a_list_to_nil() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, fp + LIST_SLOT, heap::NIL as i32);

        vm.src = AddrTarget::Immediate;
        vm.imm_src = 1;
        vm.dst = AddrTarget::Frame(fp + LIST_SLOT);
        dispatch_op(&mut vm, Opcode::Consw).expect("consw should succeed");
        vm.imm_src = 2;
        dispatch_op(&mut vm, Opcode::Consw).expect("consw should succeed");

        // The destination has to start as nil: the tail handler releases
        // whatever the slot named before.
        memory::write_word(&mut vm.frames.data, fp + DEST_SLOT, heap::NIL as i32);
        vm.src = AddrTarget::Frame(fp + LIST_SLOT);
        vm.dst = AddrTarget::Frame(fp + DEST_SLOT);
        dispatch_op(&mut vm, Opcode::Tail).expect("tail should succeed");
        let rest = memory::read_word(&vm.frames.data, fp + DEST_SLOT) as HeapId;
        assert_ne!(rest, heap::NIL, "a two-element list has a tail");

        vm.src = AddrTarget::Frame(fp + DEST_SLOT);
        vm.dst = AddrTarget::Frame(fp + DEST_SLOT + 4);
        memory::write_word(&mut vm.frames.data, fp + DEST_SLOT + 4, heap::NIL as i32);
        dispatch_op(&mut vm, Opcode::Tail).expect("tail should succeed");
        assert_eq!(
            memory::read_word(&vm.frames.data, fp + DEST_SLOT + 4) as HeapId,
            heap::NIL,
            "the tail of the last node is nil"
        );
    }

    // --- nil operands ---

    /// Every head opcode has to survive a nil list. The reference faults on it;
    /// this VM answers with a zero value instead, and that has to stay
    /// deliberate rather than turn into a panic.
    #[test]
    fn head_of_nil_yields_a_zero_value_for_every_width() {
        let module = test_module();
        for opcode in [
            Opcode::Headb,
            Opcode::Headw,
            Opcode::Headf,
            Opcode::Headl,
            Opcode::Headp,
            Opcode::Headm,
            Opcode::Headmp,
        ] {
            let mut vm = VmState::new(&module).expect("vm init");
            let fp = vm.frames.current_data_offset();
            memory::write_big(&mut vm.frames.data, fp + DEST_SLOT, -1);
            vm.src = AddrTarget::Immediate;
            vm.imm_src = heap::NIL as i32;
            vm.mid = AddrTarget::Immediate;
            vm.imm_mid = 8;
            vm.dst = AddrTarget::Frame(fp + DEST_SLOT);
            dispatch_op(&mut vm, opcode).unwrap_or_else(|e| panic!("{opcode:?} of nil: {e}"));
            match opcode {
                // A record copy of nothing leaves the destination alone, which
                // is what the reference `memmove` of a zero-length head does.
                Opcode::Headm | Opcode::Headmp => {
                    assert_eq!(memory::read_big(&vm.frames.data, fp + DEST_SLOT), -1);
                }
                Opcode::Headf => {
                    assert_eq!(memory::read_real(&vm.frames.data, fp + DEST_SLOT), 0.0);
                }
                Opcode::Headl => {
                    assert_eq!(memory::read_big(&vm.frames.data, fp + DEST_SLOT), 0);
                }
                Opcode::Headb => assert_eq!(vm.frames.data[fp + DEST_SLOT], 0),
                _ => assert_eq!(memory::read_word(&vm.frames.data, fp + DEST_SLOT), 0),
            }
        }
    }

    #[test]
    fn cons_onto_nil_leaves_the_tail_nil_for_every_width() {
        let module = test_module();
        for (opcode, size) in [
            (Opcode::Consb, 1usize),
            (Opcode::Consw, 4),
            (Opcode::Consf, 8),
            (Opcode::Consl, 8),
            (Opcode::Consp, 4),
        ] {
            let mut vm = VmState::new(&module).expect("vm init");
            let fp = vm.frames.current_data_offset();
            memory::write_word(&mut vm.frames.data, fp + LIST_SLOT, heap::NIL as i32);
            vm.src = AddrTarget::Frame(fp);
            vm.dst = AddrTarget::Frame(fp + LIST_SLOT);
            dispatch_op(&mut vm, opcode).unwrap_or_else(|e| panic!("{opcode:?}: {e}"));
            let list_id = memory::read_word(&vm.frames.data, fp + LIST_SLOT) as HeapId;
            assert_ne!(list_id, heap::NIL, "{opcode:?} must allocate a node");
            match &vm.heap.get(list_id).expect("node should exist").data {
                HeapData::List { head, tail } => {
                    assert_eq!(head.len(), size, "{opcode:?} cell size");
                    assert_eq!(*tail, heap::NIL, "{opcode:?} tail");
                }
                _ => panic!("{opcode:?} did not allocate a list node"),
            }
        }
    }

    // --- reference counting ---

    /// The reference's `cons` does not raise the tail's count: it moves the
    /// reference the destination slot held into the new node's tail field
    /// (`l->tail = lv; *lp = l;` with no `destroy`, libinterp/xec.c). The count
    /// therefore has to come out unchanged, and the tail has to stay alive.
    #[test]
    fn consw_moves_the_destination_reference_into_the_new_node() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        let old_head = vm.heap.alloc(
            0,
            HeapData::List {
                head: vec![1, 0, 0, 0],
                tail: heap::NIL,
            },
        );
        let before = vm.heap.get(old_head).expect("just allocated").ref_count;
        memory::write_word(&mut vm.frames.data, fp + LIST_SLOT, old_head as i32);

        vm.src = AddrTarget::Immediate;
        vm.imm_src = 2;
        vm.dst = AddrTarget::Frame(fp + LIST_SLOT);
        op_consw(&mut vm).expect("consw should succeed");

        let new_head = memory::read_word(&vm.frames.data, fp + LIST_SLOT) as HeapId;
        assert_ne!(new_head, old_head);
        let after = vm
            .heap
            .get(old_head)
            .expect("the tail must still be live")
            .ref_count;
        assert_eq!(
            after, before,
            "the destination's reference moves into the node's tail field"
        );
        match &vm.heap.get(new_head).expect("node should exist").data {
            HeapData::List { tail, .. } => assert_eq!(*tail, old_head),
            _ => panic!("expected List"),
        }
    }

    /// `consp` also counts the pointer it stores as the head: the reference
    /// does `h->ref++` on the source pointer before copying it into the cell
    /// (libinterp/xec.c).
    #[test]
    fn consp_takes_a_reference_for_its_head_pointer() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        let held = vm.heap.alloc(0, HeapData::Str("head object".to_string()));
        let tail = vm.heap.alloc(
            0,
            HeapData::List {
                head: vec![0, 0, 0, 0],
                tail: heap::NIL,
            },
        );
        let head_before = vm.heap.get(held).expect("just allocated").ref_count;
        let tail_before = vm.heap.get(tail).expect("just allocated").ref_count;

        memory::write_word(&mut vm.frames.data, fp + LIST_SLOT, tail as i32);
        vm.src = AddrTarget::Immediate;
        vm.imm_src = held as i32;
        vm.dst = AddrTarget::Frame(fp + LIST_SLOT);
        op_consp(&mut vm).expect("consp should succeed");

        assert_eq!(
            vm.heap.get(held).expect("head must stay live").ref_count,
            head_before + 1,
            "the cell holds a new reference to the head pointer"
        );
        assert_eq!(
            vm.heap.get(tail).expect("tail must stay live").ref_count,
            tail_before,
            "the tail reference is moved, not copied"
        );
    }

    /// `headp` and `tail` both finish with `movp` in the reference, which counts
    /// the value it stores and releases the one it overwrites.
    #[test]
    fn headp_and_tail_count_the_pointer_they_store() {
        let module = test_module();
        for opcode in [Opcode::Headp, Opcode::Tail] {
            let mut vm = VmState::new(&module).expect("vm init");
            let fp = vm.frames.current_data_offset();

            let carried = vm.heap.alloc(0, HeapData::Str("carried".to_string()));
            let mut head_bytes = vec![0u8; 4];
            memory::write_word(&mut head_bytes, 0, carried as i32);
            let tail_node = vm.heap.alloc(
                0,
                HeapData::List {
                    head: vec![0, 0, 0, 0],
                    tail: heap::NIL,
                },
            );
            let node = vm.heap.alloc(
                0,
                HeapData::List {
                    head: head_bytes,
                    tail: tail_node,
                },
            );
            let displaced = vm.heap.alloc(0, HeapData::Str("displaced".to_string()));
            let expected = if opcode == Opcode::Headp {
                carried
            } else {
                tail_node
            };
            let before = vm.heap.get(expected).expect("just allocated").ref_count;
            let displaced_before = vm.heap.get(displaced).expect("just allocated").ref_count;

            memory::write_word(&mut vm.frames.data, fp + DEST_SLOT, displaced as i32);
            vm.src = AddrTarget::Immediate;
            vm.imm_src = node as i32;
            vm.dst = AddrTarget::Frame(fp + DEST_SLOT);
            dispatch_op(&mut vm, opcode).unwrap_or_else(|e| panic!("{opcode:?}: {e}"));

            assert_eq!(
                memory::read_word(&vm.frames.data, fp + DEST_SLOT) as HeapId,
                expected
            );
            assert_eq!(
                vm.heap.get(expected).expect("must stay live").ref_count,
                before + 1,
                "{opcode:?} takes a reference for the destination"
            );
            let displaced_after = vm.heap.get(displaced).map(|o| o.ref_count).unwrap_or(0);
            assert!(
                displaced_after < displaced_before,
                "{opcode:?} must release the pointer it overwrote"
            );
        }
    }

    /// The reference finishes `headmp` with `movmp()` (libinterp/xec.c), and
    /// `movmp` counts what it copies: `incmem` takes a reference for every
    /// pointer the record holds, and `freeptrs` releases every pointer the copy
    /// overwrites. Moving the bytes alone would leave the destination naming
    /// objects nobody counted, and the first release of that field would free an
    /// object that is still in use.
    #[test]
    fn headmp_counts_the_pointers_it_copies() {
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
        let fp = vm.frames.current_data_offset();

        let carried = vm.heap.alloc(0, HeapData::Str("carried".to_string()));
        let displaced = vm.heap.alloc(0, HeapData::Str("displaced".to_string()));
        let carried_before = vm.heap.get(carried).expect("just allocated").ref_count;
        let displaced_before = vm.heap.get(displaced).expect("just allocated").ref_count;

        let mut head = vec![0u8; 8];
        memory::write_word(&mut head, 0, 7);
        memory::write_word(&mut head, 4, carried as i32);
        let node = vm.heap.alloc(
            0,
            HeapData::List {
                head,
                tail: heap::NIL,
            },
        );

        // The destination already holds a record of the same type, and its
        // pointer field names another object.
        memory::write_word(&mut vm.frames.data, fp + DEST_SLOT, 0);
        memory::write_word(&mut vm.frames.data, fp + DEST_SLOT + 4, displaced as i32);

        vm.src = AddrTarget::Immediate;
        vm.imm_src = node as i32;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 1; // Type index, not a byte count.
        vm.dst = AddrTarget::Frame(fp + DEST_SLOT);
        dispatch_op(&mut vm, Opcode::Headmp).expect("headmp should succeed");

        assert_eq!(
            memory::read_word(&vm.frames.data, fp + DEST_SLOT + 4) as HeapId,
            carried,
            "the record's pointer field has to arrive in the destination"
        );
        assert_eq!(
            vm.heap.get(carried).expect("must stay live").ref_count,
            carried_before + 1,
            "the copy in the destination is a new reference"
        );
        let displaced_after = vm.heap.get(displaced).map(|o| o.ref_count).unwrap_or(0);
        assert_eq!(
            displaced_after,
            displaced_before - 1,
            "the pointer the copy overwrote has to be released"
        );
    }

    /// The pointers a record copy overwrites have to be released wherever the
    /// copy lands, module data included, and a destination that does not fit
    /// leaves everything alone.
    #[test]
    fn headmp_releases_overwritten_pointers_in_module_data() {
        let mut module = test_module();
        module.types.push(TypeDescriptor {
            id: 1,
            size: 8,
            pointer_map: PointerMap { bytes: vec![0x40] },
            pointer_count: 1,
        });

        // Byte offset of the destination record in module data, for the plain Mp
        // target and for the same module named by index.
        for dst in [
            AddrTarget::Mp(8),
            AddrTarget::ModuleMp {
                module_idx: 0,
                offset: 8,
            },
        ] {
            let mut vm = VmState::new(&module).expect("vm init");
            vm.mp = vec![0u8; 32];
            let carried = vm.heap.alloc(0, HeapData::Str("carried".to_string()));
            let displaced = vm.heap.alloc(0, HeapData::Str("displaced".to_string()));
            memory::write_word(&mut vm.mp, 12, displaced as i32);

            let mut head = vec![0u8; 8];
            memory::write_word(&mut head, 4, carried as i32);
            let node = vm.heap.alloc(
                0,
                HeapData::List {
                    head,
                    tail: heap::NIL,
                },
            );

            vm.src = AddrTarget::Immediate;
            vm.imm_src = node as i32;
            vm.mid = AddrTarget::Immediate;
            vm.imm_mid = 1;
            vm.dst = dst;
            dispatch_op(&mut vm, Opcode::Headmp).expect("headmp should succeed");

            assert_eq!(
                memory::read_word(&vm.mp, 12) as HeapId,
                carried,
                "{dst:?}: the copy has to reach module data"
            );
            assert!(
                !vm.heap.contains(displaced),
                "{dst:?}: the overwritten pointer has to be released"
            );
        }

        // A destination the record does not fit in: nothing is copied, so
        // nothing may be released either.
        let mut vm = VmState::new(&module).expect("vm init");
        vm.mp = vec![0u8; 32];
        let held = vm.heap.alloc(0, HeapData::Str("still in use".to_string()));
        memory::write_word(&mut vm.mp, 28, held as i32);
        let node = vm.heap.alloc(
            0,
            HeapData::List {
                head: vec![0u8; 8],
                tail: heap::NIL,
            },
        );
        vm.src = AddrTarget::Immediate;
        vm.imm_src = node as i32;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 1;
        vm.dst = AddrTarget::Mp(28);
        dispatch_op(&mut vm, Opcode::Headmp).expect("headmp should succeed");
        assert_eq!(memory::read_word(&vm.mp, 28) as HeapId, held);
        assert!(
            vm.heap.contains(held),
            "a copy that does not fit releases nothing"
        );
    }

    /// A nil list has no head to copy, so `headmp` must leave the destination
    /// exactly as it found it. Releasing the pointer that is still sitting there
    /// would free an object the frame is about to use again.
    #[test]
    fn headmp_of_nil_releases_nothing() {
        let mut module = test_module();
        module.types.push(TypeDescriptor {
            id: 1,
            size: 8,
            pointer_map: PointerMap { bytes: vec![0x40] },
            pointer_count: 1,
        });
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        let held = vm.heap.alloc(0, HeapData::Str("still in use".to_string()));
        let before = vm.heap.get(held).expect("just allocated").ref_count;
        memory::write_word(&mut vm.frames.data, fp + DEST_SLOT + 4, held as i32);

        vm.src = AddrTarget::Immediate;
        vm.imm_src = heap::NIL as i32;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 1;
        vm.dst = AddrTarget::Frame(fp + DEST_SLOT);
        dispatch_op(&mut vm, Opcode::Headmp).expect("headmp of nil should succeed");

        assert_eq!(
            memory::read_word(&vm.frames.data, fp + DEST_SLOT + 4) as HeapId,
            held,
            "the destination keeps the pointer it had"
        );
        assert_eq!(
            vm.heap.get(held).expect("must stay live").ref_count,
            before,
            "a copy that never happened releases nothing"
        );
    }

    // --- faults ---

    /// A non-nil id with nothing behind it is a dangling reference. Saying "nil
    /// dereference" would send the reader looking at the program instead of at
    /// the heap, so the message names the id.
    #[test]
    fn head_of_a_released_list_faults() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        let gone = vm.heap.alloc(
            0,
            HeapData::List {
                head: vec![1, 0, 0, 0],
                tail: heap::NIL,
            },
        );
        vm.heap.dec_ref(gone);
        assert!(!vm.heap.contains(gone), "the node must really be gone");

        vm.src = AddrTarget::Immediate;
        vm.imm_src = gone as i32;
        vm.dst = AddrTarget::Frame(fp + DEST_SLOT);
        let err = op_headw(&mut vm).expect_err("head of a released list must fault");
        assert!(
            err.to_string().contains("released list"),
            "expected a released-list fault, got: {err}"
        );
    }

    #[test]
    fn head_of_something_that_is_not_a_list_faults() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        let text = vm.heap.alloc(0, HeapData::Str("not a list".to_string()));
        vm.src = AddrTarget::Immediate;
        vm.imm_src = text as i32;
        vm.dst = AddrTarget::Frame(fp + DEST_SLOT);
        let err = op_headw(&mut vm).expect_err("head of a string must fault");
        assert!(
            err.to_string().contains("head on non-list"),
            "expected a non-list fault, got: {err}"
        );
    }

    #[test]
    fn tail_of_something_that_is_not_a_list_faults() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        let text = vm.heap.alloc(0, HeapData::Str("not a list".to_string()));
        memory::write_word(&mut vm.frames.data, fp + DEST_SLOT, heap::NIL as i32);
        vm.src = AddrTarget::Immediate;
        vm.imm_src = text as i32;
        vm.dst = AddrTarget::Frame(fp + DEST_SLOT);
        let err = op_tail(&mut vm).expect_err("tail of a string must fault");
        assert!(
            err.to_string().contains("tail on non-list"),
            "expected a non-list fault, got: {err}"
        );
    }

    #[test]
    fn tail_of_a_released_list_faults() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        let gone = vm.heap.alloc(
            0,
            HeapData::List {
                head: vec![1, 0, 0, 0],
                tail: heap::NIL,
            },
        );
        vm.heap.dec_ref(gone);
        memory::write_word(&mut vm.frames.data, fp + DEST_SLOT, heap::NIL as i32);
        vm.src = AddrTarget::Immediate;
        vm.imm_src = gone as i32;
        vm.dst = AddrTarget::Frame(fp + DEST_SLOT);
        let err = op_tail(&mut vm).expect_err("tail of a released list must fault");
        assert!(
            err.to_string().contains("nil list dereference"),
            "expected a dereference fault, got: {err}"
        );
    }

    // --- where the cons head comes from ---

    #[test]
    fn cons_reads_its_head_from_module_data() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        vm.mp = vec![0u8; 32];
        memory::write_word(&mut vm.mp, 8, 0x0BAD_F00D_u32 as i32);

        memory::write_word(&mut vm.frames.data, fp + LIST_SLOT, heap::NIL as i32);
        vm.src = AddrTarget::Mp(8);
        vm.dst = AddrTarget::Frame(fp + LIST_SLOT);
        op_consw(&mut vm).expect("consw from module data should succeed");

        let list_id = memory::read_word(&vm.frames.data, fp + LIST_SLOT) as HeapId;
        match &vm.heap.get(list_id).expect("node should exist").data {
            HeapData::List { head, .. } => {
                assert_eq!(memory::read_word(head, 0), 0x0BAD_F00D_u32 as i32);
            }
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn cons_out_of_range_module_data_source_is_rejected() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        vm.mp = vec![0u8; 8];

        memory::write_word(&mut vm.frames.data, fp + LIST_SLOT, heap::NIL as i32);
        vm.src = AddrTarget::Mp(6); // Only two bytes left.
        vm.dst = AddrTarget::Frame(fp + LIST_SLOT);
        let err = op_consw(&mut vm).expect_err("an out-of-range source must be rejected");
        assert!(
            err.to_string().contains("out of bounds"),
            "expected out of bounds, got: {err}"
        );
    }

    #[test]
    fn cons_from_an_unused_operand_stores_zeroes() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, fp + LIST_SLOT, heap::NIL as i32);
        vm.src = AddrTarget::None;
        vm.dst = AddrTarget::Frame(fp + LIST_SLOT);
        op_consw(&mut vm).expect("consw should succeed");

        let list_id = memory::read_word(&vm.frames.data, fp + LIST_SLOT) as HeapId;
        match &vm.heap.get(list_id).expect("node should exist").data {
            HeapData::List { head, .. } => assert_eq!(head, &vec![0u8; 4]),
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn cons_reads_its_head_from_a_heap_array() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        let mut data = vec![0u8; 16];
        memory::write_word(&mut data, 4, 0x5A5A);
        let array = vm.heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 4,
                data,
                length: 4,
            },
        );

        memory::write_word(&mut vm.frames.data, fp + LIST_SLOT, heap::NIL as i32);
        vm.src = AddrTarget::HeapArray {
            id: array,
            offset: 4,
        };
        vm.dst = AddrTarget::Frame(fp + LIST_SLOT);
        op_consw(&mut vm).expect("consw from an array element should succeed");

        let list_id = memory::read_word(&vm.frames.data, fp + LIST_SLOT) as HeapId;
        match &vm.heap.get(list_id).expect("node should exist").data {
            HeapData::List { head, .. } => assert_eq!(memory::read_word(head, 0), 0x5A5A),
            _ => panic!("expected List"),
        }
    }

    /// An immediate narrower than a word keeps only its low byte, which is what
    /// a byte cell holds.
    #[test]
    fn consb_from_an_immediate_keeps_the_low_byte() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, fp + LIST_SLOT, heap::NIL as i32);
        vm.src = AddrTarget::Immediate;
        vm.imm_src = 0x1234_5678;
        vm.dst = AddrTarget::Frame(fp + LIST_SLOT);
        op_consb(&mut vm).expect("consb should succeed");

        let list_id = memory::read_word(&vm.frames.data, fp + LIST_SLOT) as HeapId;
        match &vm.heap.get(list_id).expect("node should exist").data {
            HeapData::List { head, .. } => assert_eq!(head, &vec![0x78]),
            _ => panic!("expected List"),
        }
    }

    /// `consmp` takes a type index. An index no type descriptor covers has no
    /// size to look up, so it falls back to treating the operand as a byte
    /// count, exactly as the untyped `consm` does.
    #[test]
    fn consmp_with_an_unknown_type_falls_back_to_a_byte_count() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, fp, 0x2222_1111);
        memory::write_word(&mut vm.frames.data, fp + LIST_SLOT, heap::NIL as i32);

        vm.src = AddrTarget::Frame(fp);
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 4; // No type 4 exists, so this is a byte count.
        vm.dst = AddrTarget::Frame(fp + LIST_SLOT);
        op_consmp(&mut vm).expect("consmp should succeed");

        let list_id = memory::read_word(&vm.frames.data, fp + LIST_SLOT) as HeapId;
        match &vm.heap.get(list_id).expect("node should exist").data {
            HeapData::List { head, .. } => {
                assert_eq!(head.len(), 4);
                assert_eq!(memory::read_word(head, 0), 0x2222_1111);
            }
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn consmp_oversized_block_is_rejected() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, fp + LIST_SLOT, heap::NIL as i32);
        vm.src = AddrTarget::Frame(fp);
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = (MAX_CONS_BYTES + 1) as i32;
        vm.dst = AddrTarget::Frame(fp + LIST_SLOT);
        let err = op_consmp(&mut vm).expect_err("an absurd block size must be rejected");
        assert!(
            err.to_string().contains("too large"),
            "expected a size fault, got: {err}"
        );
    }

    /// A zero-byte element is legal: the reference conses `W(m)` bytes and makes
    /// no exception for zero.
    #[test]
    fn consm_of_zero_bytes_makes_an_empty_cell() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, fp + LIST_SLOT, heap::NIL as i32);
        vm.src = AddrTarget::Frame(fp);
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 0;
        vm.dst = AddrTarget::Frame(fp + LIST_SLOT);
        op_consm(&mut vm).expect("consm of zero bytes should succeed");

        let list_id = memory::read_word(&vm.frames.data, fp + LIST_SLOT) as HeapId;
        match &vm.heap.get(list_id).expect("node should exist").data {
            HeapData::List { head, tail } => {
                assert!(head.is_empty());
                assert_eq!(*tail, heap::NIL);
            }
            _ => panic!("expected List"),
        }
    }

    /// A cons source can name a module's data area by module index, which is how
    /// an operand resolved from a cross-module virtual address arrives. Index 0
    /// is the module that is running, and an index no module holds reads as
    /// zeroes rather than faulting.
    #[test]
    fn cons_reads_its_head_from_a_module_data_area_by_index() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        vm.mp = vec![0u8; 32];
        memory::write_word(&mut vm.mp, 12, 0x7788_99AA);

        memory::write_word(&mut vm.frames.data, fp + LIST_SLOT, heap::NIL as i32);
        vm.src = AddrTarget::ModuleMp {
            module_idx: 0,
            offset: 12,
        };
        vm.dst = AddrTarget::Frame(fp + LIST_SLOT);
        op_consw(&mut vm).expect("consw from module data should succeed");
        let list_id = memory::read_word(&vm.frames.data, fp + LIST_SLOT) as HeapId;
        match &vm.heap.get(list_id).expect("node should exist").data {
            HeapData::List { head, .. } => assert_eq!(memory::read_word(head, 0), 0x7788_99AA),
            _ => panic!("expected List"),
        }

        memory::write_word(&mut vm.frames.data, fp + LIST_SLOT + 4, heap::NIL as i32);
        vm.src = AddrTarget::ModuleMp {
            module_idx: 9,
            offset: 0,
        };
        vm.dst = AddrTarget::Frame(fp + LIST_SLOT + 4);
        op_consw(&mut vm).expect("consw from a module that is not loaded should succeed");
        let zeroed = memory::read_word(&vm.frames.data, fp + LIST_SLOT + 4) as HeapId;
        match &vm.heap.get(zeroed).expect("node should exist").data {
            HeapData::List { head, .. } => assert_eq!(head, &vec![0u8; 4]),
            _ => panic!("expected List"),
        }
    }

    /// A cons has to write the new node somewhere. An immediate destination is
    /// not a place, so the write is refused rather than silently dropped.
    #[test]
    fn cons_to_an_immediate_destination_is_rejected() {
        let module = test_module();
        for opcode in [Opcode::Consw, Opcode::Consp] {
            let mut vm = VmState::new(&module).expect("vm init");
            let fp = vm.frames.current_data_offset();
            vm.src = AddrTarget::Frame(fp);
            vm.dst = AddrTarget::Immediate;
            vm.imm_dst = heap::NIL as i32;
            let err = dispatch_op(&mut vm, opcode)
                .expect_err("an immediate destination must be rejected");
            assert!(
                err.to_string().contains("immediate"),
                "{opcode:?}: expected an immediate-destination fault, got: {err}"
            );
        }
    }

    /// `headm` copies into a module data area named by index, the same way a
    /// cons reads from one.
    #[test]
    fn headm_writes_into_a_module_data_area_by_index() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        vm.mp = vec![0u8; 32];
        let node = vm.heap.alloc(
            0,
            HeapData::List {
                head: vec![1, 2, 3, 4],
                tail: heap::NIL,
            },
        );
        vm.src = AddrTarget::Immediate;
        vm.imm_src = node as i32;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 4;
        vm.dst = AddrTarget::ModuleMp {
            module_idx: 0,
            offset: 8,
        };
        op_headm(&mut vm).expect("headm should succeed");
        assert_eq!(&vm.mp[8..12], &[1, 2, 3, 4]);

        // A module index nothing holds leaves the copy undone rather than
        // faulting, which matches how the read side treats it.
        vm.dst = AddrTarget::ModuleMp {
            module_idx: 9,
            offset: 0,
        };
        op_headm(&mut vm).expect("headm into an absent module should succeed");
        assert_eq!(&vm.mp[0..4], &[0, 0, 0, 0]);
    }

    /// `headm` copies into module data as readily as into a frame.
    #[test]
    fn headm_writes_into_module_data() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        vm.mp = vec![0u8; 32];
        let node = vm.heap.alloc(
            0,
            HeapData::List {
                head: vec![9, 8, 7, 6],
                tail: heap::NIL,
            },
        );
        vm.src = AddrTarget::Immediate;
        vm.imm_src = node as i32;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 4;
        vm.dst = AddrTarget::Mp(4);
        op_headm(&mut vm).expect("headm should succeed");
        assert_eq!(&vm.mp[4..8], &[9, 8, 7, 6]);
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
