use ricevm_core::ExecError;

use crate::heap;
use crate::vm::VmState;

/// movp src, dst:move pointer with reference counting
pub(crate) fn op_movp(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let new_id = vm.src_ptr()?;
    vm.move_ptr_to_dst(new_id)
}

/// lea src, dst:load effective address: stores the address from src into dst as a pointer
/// In our model, lea is used to get a "pointer" to a frame/mp location.
/// lea src, dst: store the absolute address of src into dst.
/// In the C++ VM this stores a raw pointer. In our model, we store
/// the absolute byte offset into the frame stack or MP.
pub(crate) fn op_lea(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let addr = match vm.src {
        crate::address::AddrTarget::Frame(off) => off as i32,
        crate::address::AddrTarget::Mp(off) => {
            let module_idx = if let Some(idx) = vm.current_loaded_module {
                idx + 1
            } else {
                0
            };
            (crate::address::MP_BASE + module_idx * crate::address::MP_STRIDE + off) as i32
        }
        crate::address::AddrTarget::ModuleMp { module_idx, offset } => {
            (crate::address::MP_BASE + module_idx * crate::address::MP_STRIDE + offset) as i32
        }
        crate::address::AddrTarget::HeapArray { id, offset } => {
            // Store a heap ref so downstream double-indirect addressing
            // can resolve back to this heap array element.
            let ref_idx = vm.heap_refs.len();
            vm.heap_refs.push((id, offset));
            crate::address::HEAP_REF_FLAG | (ref_idx as i32)
        }
        crate::address::AddrTarget::Immediate => vm.imm_src,
        crate::address::AddrTarget::None => 0,
    };
    vm.set_dst_word(addr)
}

/// indx src, mid, dst:array index: mid = &src[dst]
/// src = array pointer, dst = index, mid = result (address of element)
///
/// Stores a heap array reference in the frame slot pointed to by mid.
/// The reference is encoded as a flagged index into VmState.heap_refs.
/// Subsequent double-indirect addressing through this slot will resolve
/// to the actual array element in heap memory.
pub(crate) fn op_indx(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let arr_id = vm.src_ptr()?;
    let raw_index = vm.dst_word()?;
    if raw_index < 0 {
        return Err(ExecError::ThreadFault(format!(
            "array index negative: {raw_index}"
        )));
    }
    let index = raw_index as usize;

    let obj = match vm.heap.get(arr_id) {
        Some(o) => o,
        None => return vm.raise_exception("nil dereference"),
    };

    match &obj.data {
        crate::heap::HeapData::Array {
            elem_size, length, ..
        } => {
            if index >= *length {
                return Err(ExecError::ThreadFault(format!(
                    "array index out of bounds: {index} >= {length}"
                )));
            }
            let byte_offset = index * elem_size;
            let ref_idx = vm.heap_refs.len();
            vm.heap_refs.push((arr_id, byte_offset));
            let encoded = crate::address::HEAP_REF_FLAG | (ref_idx as i32);
            vm.write_word_at(vm.mid, encoded)?;
            Ok(())
        }
        crate::heap::HeapData::ArraySlice {
            parent_id,
            byte_start,
            elem_size,
            length,
            ..
        } => {
            if index >= *length {
                return Err(ExecError::ThreadFault(format!(
                    "array index out of bounds: {index} >= {length}"
                )));
            }
            let byte_offset = byte_start + index * elem_size;
            let ref_idx = vm.heap_refs.len();
            // Reference the parent array directly.
            vm.heap_refs.push((*parent_id, byte_offset));
            let encoded = crate::address::HEAP_REF_FLAG | (ref_idx as i32);
            vm.write_word_at(vm.mid, encoded)?;
            Ok(())
        }
        _ => Err(ExecError::ThreadFault("indx on non-array".to_string())),
    }
}

/// indw/indf/indb/indl: same as indx (array index by element type).
/// src = array pointer, dst = index, mid = result (address of element).
pub(crate) fn op_indw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    op_indx(vm)
}

pub(crate) fn op_indf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    op_indx(vm)
}

pub(crate) fn op_indb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    op_indx(vm)
}

pub(crate) fn op_indl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    op_indx(vm)
}

/// lena src, dst:array length
pub(crate) fn op_lena(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let arr_id = vm.src_ptr()?;
    let len = if arr_id == heap::NIL {
        0
    } else {
        match vm.heap.get(arr_id) {
            Some(obj) => match &obj.data {
                crate::heap::HeapData::Array { length, .. }
                | crate::heap::HeapData::ArraySlice { length, .. } => *length as i32,
                _ => 0,
            },
            None => 0,
        }
    };
    vm.set_dst_word(len)
}

/// slicea src, mid, dst:slice an array: dst = dst[src..mid]
pub(crate) fn op_slicea(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let raw_start = vm.src_word()?;
    let raw_end = vm.mid_word()?;
    let start = raw_start.max(0) as usize;
    let end = raw_end.max(0) as usize;
    let arr_id = vm.dst_ptr()?;

    if arr_id == heap::NIL {
        if start == 0 && end == 0 {
            return Ok(());
        }
        return Err(ExecError::ThreadFault("slice of nil array".to_string()));
    }

    // Resolve to the root array (unwrap nested slices).
    let (root_id, base_byte_start, elem_type, elem_size, root_length) = {
        let obj = vm
            .heap
            .get(arr_id)
            .ok_or_else(|| ExecError::ThreadFault("slicea: invalid array".to_string()))?;
        match &obj.data {
            heap::HeapData::Array {
                elem_type,
                elem_size,
                length,
                ..
            } => (arr_id, 0, *elem_type, *elem_size, *length),
            heap::HeapData::ArraySlice {
                parent_id,
                byte_start: bs,
                elem_type,
                elem_size,
                length,
            } => (*parent_id, *bs, *elem_type, *elem_size, *length),
            _ => return Err(ExecError::ThreadFault("slicea on non-array".to_string())),
        }
    };

    let source_length = if root_id == arr_id {
        root_length
    } else {
        // For slices, the relevant length is the slice length
        let Some(obj) = vm.heap.get(arr_id) else {
            return Err(ExecError::Other(format!("invalid heap id {arr_id}")));
        };
        match &obj.data {
            heap::HeapData::ArraySlice { length, .. } => *length,
            _ => root_length,
        }
    };

    if end > source_length || start > end {
        return Err(ExecError::ThreadFault(format!(
            "array slice out of bounds: [{start}..{end}] for length {source_length}"
        )));
    }

    let new_len = end - start;
    let byte_start = base_byte_start + start * elem_size;

    let new_id = vm.heap.alloc(
        elem_type,
        heap::HeapData::ArraySlice {
            parent_id: root_id,
            byte_start,
            elem_type,
            elem_size,
            length: new_len,
        },
    );
    vm.heap.inc_ref(root_id);
    vm.set_dst_ptr(new_id)?;
    vm.heap.dec_ref(arr_id);
    Ok(())
}

/// slicela src, mid, dst:array append/extend.
/// Copies elements from src into dst starting at index mid.
/// dst = dst[0..mid] ++ src[0..len(src)]
/// The dst array must be large enough to hold mid + len(src) elements.
pub(crate) fn op_slicela(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let src_id = vm.src_ptr()?;
    let insert_pos = vm.mid_word()?.max(0) as usize;
    let dst_id = vm.dst_ptr()?;

    // If src is nil, nothing to copy.
    if src_id == heap::NIL {
        return Ok(());
    }

    // Read src array data (resolving ArraySlice to parent).
    let (src_data, src_len, elem_size, _elem_type) = {
        let obj = vm
            .heap
            .get(src_id)
            .ok_or_else(|| ExecError::ThreadFault("slicela: invalid src array".to_string()))?;
        match &obj.data {
            heap::HeapData::Array {
                data,
                length,
                elem_size,
                elem_type,
            } => (data.clone(), *length, *elem_size, *elem_type),
            heap::HeapData::ArraySlice {
                parent_id,
                byte_start,
                elem_size,
                length,
                elem_type,
            } => {
                let pid = *parent_id;
                let bs = *byte_start;
                let es = *elem_size;
                let len = *length;
                let et = *elem_type;
                let byte_len = len * es;
                let data = vm.heap.array_read(pid, bs, byte_len).unwrap_or_default();
                (data, len, es, et)
            }
            _ => {
                return Err(ExecError::ThreadFault(
                    "slicela: src not an array".to_string(),
                ));
            }
        }
    };

    // Resolve dst: if it's an ArraySlice, redirect write to the parent array.
    let (real_dst_id, dst_byte_offset) = {
        if let Some(obj) = vm.heap.get(dst_id) {
            match &obj.data {
                heap::HeapData::ArraySlice {
                    parent_id,
                    byte_start,
                    ..
                } => (*parent_id, *byte_start),
                _ => (dst_id, 0),
            }
        } else {
            (dst_id, 0)
        }
    };

    // For pointer-sized elements, collect old dst values before overwrite
    // so we can adjust reference counts (inc new, dec old).
    let mut old_ptrs = Vec::new();
    let mut new_ptrs = Vec::new();
    if elem_size == 4 {
        let copy_start = dst_byte_offset + insert_pos * elem_size;
        if let Some(obj) = vm.heap.get(real_dst_id)
            && let heap::HeapData::Array { data, .. } = &obj.data
        {
            for i in 0..src_len {
                let off = copy_start + i * 4;
                if off + 4 <= data.len() {
                    let id = crate::memory::read_word(data, off) as u32;
                    if id != heap::NIL && vm.heap.contains(id) {
                        old_ptrs.push(id);
                    }
                }
            }
        }
        for i in 0..src_len {
            let off = i * 4;
            if off + 4 <= src_data.len() {
                let id = crate::memory::read_word(&src_data, off) as u32;
                if id != heap::NIL && vm.heap.contains(id) {
                    new_ptrs.push(id);
                }
            }
        }
    }

    // Write into dst array (now always the root Array).
    if let Some(obj) = vm.heap.get_mut(real_dst_id)
        && let heap::HeapData::Array { data, length, .. } = &mut obj.data
    {
        let copy_start = dst_byte_offset + insert_pos * elem_size;
        let copy_bytes = src_len * elem_size;
        let needed = copy_start + copy_bytes;
        if needed > data.len() {
            data.resize(needed, 0);
        }
        data[copy_start..copy_start + copy_bytes]
            .copy_from_slice(&src_data[..copy_bytes.min(src_data.len())]);
        // Only update root length when writing directly to root (not through a slice).
        if dst_byte_offset == 0 {
            *length = (insert_pos + src_len).max(*length);
        }
    }

    // Adjust ref counts: inc new pointers, dec old ones.
    for id in &new_ptrs {
        vm.heap.inc_ref(*id);
    }
    for id in &old_ptrs {
        vm.heap.dec_ref(*id);
    }

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
    use crate::heap::HeapData;
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
                size: 64,
                pointer_map: PointerMap { bytes: vec![] },
                pointer_count: 0,
            }],
            data: vec![],
            name: "pointer_test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        }
    }

    #[test]
    fn slicea_shares_storage_with_parent() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        // Create a parent array of 5 i32 elements (elem_size=4).
        let mut data = vec![0u8; 20];
        for i in 0..5i32 {
            memory::write_word(&mut data, i as usize * 4, (i + 1) * 10);
        }
        let arr_id = vm.heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 4,
                data,
                length: 5,
            },
        );

        // slicea(start=1, end=4, arr) -> slice of elements [1..4)
        vm.src = AddrTarget::Immediate;
        vm.imm_src = 1; // start
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 4; // end
        // Store arr_id in a frame slot for dst
        memory::write_word(&mut vm.frames.data, fp, arr_id as i32);
        vm.dst = AddrTarget::Frame(fp);

        op_slicea(&mut vm).expect("slicea should succeed");

        // dst now holds the slice id
        let slice_id = memory::read_word(&vm.frames.data, fp) as u32;
        assert_ne!(slice_id, arr_id);

        // Write through the slice (at offset 0 in the slice = offset 1 in parent)
        vm.heap.array_write(slice_id, 0, &99i32.to_le_bytes());

        // Read from the parent at element index 1 -- should see the change
        let parent_bytes = vm.heap.array_read(arr_id, 4, 4).unwrap();
        let val = i32::from_le_bytes(parent_bytes.try_into().unwrap());
        assert_eq!(val, 99);
    }

    #[test]
    fn slicela_appends_arrays() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        // Source array: [100, 200]
        let mut src_data = vec![0u8; 8];
        memory::write_word(&mut src_data, 0, 100);
        memory::write_word(&mut src_data, 4, 200);
        let src_id = vm.heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 4,
                data: src_data,
                length: 2,
            },
        );

        // Destination array: [10, 20, 0, 0] (length 4, but only 2 meaningful)
        let mut dst_data = vec![0u8; 16];
        memory::write_word(&mut dst_data, 0, 10);
        memory::write_word(&mut dst_data, 4, 20);
        let dst_id = vm.heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 4,
                data: dst_data,
                length: 4,
            },
        );

        // slicela(src, insert_pos=2, dst) -> dst[0..2] ++ src[0..2]
        memory::write_word(&mut vm.frames.data, fp, src_id as i32);
        vm.src = AddrTarget::Frame(fp);
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 2; // insert at index 2
        memory::write_word(&mut vm.frames.data, fp + 4, dst_id as i32);
        vm.dst = AddrTarget::Frame(fp + 4);

        op_slicela(&mut vm).expect("slicela should succeed");

        // Verify dst array now has [10, 20, 100, 200]
        let obj = vm.heap.get(dst_id).unwrap();
        match &obj.data {
            HeapData::Array { data, length, .. } => {
                assert_eq!(*length, 4);
                assert_eq!(memory::read_word(data, 0), 10);
                assert_eq!(memory::read_word(data, 4), 20);
                assert_eq!(memory::read_word(data, 8), 100);
                assert_eq!(memory::read_word(data, 12), 200);
            }
            _ => panic!("expected Array"),
        }
    }

    #[test]
    fn indx_on_array_slice() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        // Parent array: [10, 20, 30, 40, 50]
        let mut data = vec![0u8; 20];
        for i in 0..5i32 {
            memory::write_word(&mut data, i as usize * 4, (i + 1) * 10);
        }
        let parent_id = vm.heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 4,
                data,
                length: 5,
            },
        );

        // Create a slice [1..4) => elements 20, 30, 40
        let slice_id = vm.heap.alloc(
            0,
            HeapData::ArraySlice {
                parent_id,
                byte_start: 4, // starts at element 1
                elem_type: 0,
                elem_size: 4,
                length: 3,
            },
        );

        // indx(slice_id, index=1, mid_slot) -> should point to element 30 (parent offset 8)
        memory::write_word(&mut vm.frames.data, fp, slice_id as i32);
        vm.src = AddrTarget::Frame(fp);
        memory::write_word(&mut vm.frames.data, fp + 8, 1); // index = 1
        vm.dst = AddrTarget::Frame(fp + 8);
        vm.mid = AddrTarget::Frame(fp + 4); // result goes here

        op_indx(&mut vm).expect("indx should succeed");

        // The mid slot should have a heap ref that points to (parent_id, byte_offset=8)
        let encoded = memory::read_word(&vm.frames.data, fp + 4);
        assert!(encoded & crate::address::HEAP_REF_FLAG != 0);
        let ref_idx = (encoded & !crate::address::HEAP_REF_FLAG) as usize;
        let (ref_id, ref_offset) = vm.heap_refs[ref_idx];
        assert_eq!(ref_id, parent_id);
        assert_eq!(ref_offset, 8); // byte_start(4) + index(1) * elem_size(4) = 8
    }

    #[test]
    fn lena_on_array_slice_returns_slice_length() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        // Parent array with 10 elements
        let parent_id = vm.heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 4,
                data: vec![0u8; 40],
                length: 10,
            },
        );

        // Slice with length 3
        let slice_id = vm.heap.alloc(
            0,
            HeapData::ArraySlice {
                parent_id,
                byte_start: 8,
                elem_type: 0,
                elem_size: 4,
                length: 3,
            },
        );

        memory::write_word(&mut vm.frames.data, fp, slice_id as i32);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 4);

        op_lena(&mut vm).expect("lena should succeed");

        let len = memory::read_word(&vm.frames.data, fp + 4);
        assert_eq!(len, 3); // slice length, not parent length (10)
    }

    /// Regression: slicea must produce a view that shares storage with the parent.
    /// Writes through the slice must be visible from the parent array.
    #[test]
    fn slicea_shared_storage_write_through() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        // Parent array: [10, 20, 30, 40, 50]
        let mut data = vec![0u8; 20];
        for i in 0..5i32 {
            memory::write_word(&mut data, i as usize * 4, (i + 1) * 10);
        }
        let arr_id = vm.heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 4,
                data,
                length: 5,
            },
        );

        // Slice [1..4) => elements 20, 30, 40
        vm.src = AddrTarget::Immediate;
        vm.imm_src = 1;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 4;
        memory::write_word(&mut vm.frames.data, fp, arr_id as i32);
        vm.dst = AddrTarget::Frame(fp);

        op_slicea(&mut vm).expect("slicea should succeed");
        let slice_id = memory::read_word(&vm.frames.data, fp) as u32;

        // Write 999 to slice[0] (= parent[1])
        vm.heap.array_write(slice_id, 0, &999i32.to_le_bytes());

        // Read parent[1] and verify the write is visible
        let parent_bytes = vm.heap.array_read(arr_id, 4, 4).unwrap();
        let val = i32::from_le_bytes(parent_bytes.try_into().unwrap());
        assert_eq!(val, 999, "write through slice should be visible in parent");

        // Read parent[0] to verify we did not corrupt adjacent data
        let parent_bytes0 = vm.heap.array_read(arr_id, 0, 4).unwrap();
        let val0 = i32::from_le_bytes(parent_bytes0.try_into().unwrap());
        assert_eq!(val0, 10, "adjacent parent element should be untouched");
    }

    /// Property: slicea preserves the expected length.
    #[test]
    fn property_slicea_preserves_length() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        let arr_id = vm.heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 4,
                data: vec![0u8; 40],
                length: 10,
            },
        );

        // Test various slice ranges
        let ranges: &[(i32, i32, usize)] = &[
            (0, 10, 10),
            (0, 5, 5),
            (3, 7, 4),
            (0, 0, 0),
            (5, 5, 0),
            (9, 10, 1),
        ];

        for &(start, end, expected_len) in ranges {
            vm.src = AddrTarget::Immediate;
            vm.imm_src = start;
            vm.mid = AddrTarget::Immediate;
            vm.imm_mid = end;
            memory::write_word(&mut vm.frames.data, fp, arr_id as i32);
            // Inc ref so the slice can decrement it
            vm.heap.inc_ref(arr_id);
            vm.dst = AddrTarget::Frame(fp);

            op_slicea(&mut vm).expect("slicea should succeed");

            let slice_id = memory::read_word(&vm.frames.data, fp) as u32;
            if expected_len == 0 && slice_id == arr_id {
                // Empty slices may still point to parent
                continue;
            }
            let obj = vm.heap.get(slice_id).unwrap();
            match &obj.data {
                HeapData::ArraySlice { length, .. } => {
                    assert_eq!(
                        *length, expected_len,
                        "slice [{start}..{end}) should have length {expected_len}"
                    );
                }
                _ => panic!("expected ArraySlice for range [{start}..{end})"),
            }
        }
    }

    /// Property: indx rejects out-of-bounds indices.
    #[test]
    fn property_indx_rejects_out_of_bounds() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        let arr_id = vm.heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 4,
                data: vec![0u8; 20],
                length: 5,
            },
        );

        // Index 5 should fail (length is 5, valid indices are 0..4)
        memory::write_word(&mut vm.frames.data, fp, arr_id as i32);
        vm.src = AddrTarget::Frame(fp);
        memory::write_word(&mut vm.frames.data, fp + 8, 5);
        vm.dst = AddrTarget::Frame(fp + 8);
        vm.mid = AddrTarget::Frame(fp + 4);

        let err = op_indx(&mut vm).expect_err("index 5 on array of length 5 should fail");
        assert!(
            err.to_string().contains("out of bounds"),
            "error should mention out of bounds: {err}"
        );

        // Negative index should also fail
        memory::write_word(&mut vm.frames.data, fp, arr_id as i32);
        vm.src = AddrTarget::Frame(fp);
        memory::write_word(&mut vm.frames.data, fp + 8, -1);
        vm.dst = AddrTarget::Frame(fp + 8);
        vm.mid = AddrTarget::Frame(fp + 4);

        let err = op_indx(&mut vm).expect_err("negative index should fail");
        assert!(
            err.to_string().contains("negative"),
            "error should mention negative: {err}"
        );
    }
}
