use ricevm_core::ExecError;

use crate::heap;
use crate::vm::VmState;

/// movp src, dst — move pointer with reference counting
pub(crate) fn op_movp(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let new_id = vm.src_ptr()?;
    vm.move_ptr_to_dst(new_id)
}

/// lea src, dst — load effective address: stores the address from src into dst as a pointer
/// In our model, lea is used to get a "pointer" to a frame/mp location.
/// We store the resolved address as a word value (not a heap pointer).
pub(crate) fn op_lea(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // lea stores the raw address value. In the C++ VM this is a raw pointer.
    // In our model, src is already resolved to an AddrTarget. We store the
    // address value so double-indirect addressing can use it later.
    let addr = vm.imm_src; // For immediate, this is the value; for FP/MP, the register1 offset
    vm.set_dst_word(addr)
}

/// indx src, mid, dst — array index: mid = &src[dst]
/// src = array pointer, dst = index, mid = result (address of element)
///
/// Stores a heap array reference in the frame slot pointed to by mid.
/// The reference is encoded as a flagged index into VmState.heap_refs.
/// Subsequent double-indirect addressing through this slot will resolve
/// to the actual array element in heap memory.
pub(crate) fn op_indx(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let arr_id = vm.src_ptr()?;
    let index = vm.dst_word()? as usize;

    let obj = vm
        .heap
        .get(arr_id)
        .ok_or_else(|| ExecError::ThreadFault("nil array dereference".to_string()))?;

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
            // Store (arr_id, byte_offset) in the heap_refs table
            let ref_idx = vm.heap_refs.len();
            vm.heap_refs.push((arr_id, byte_offset));
            // Write the encoded reference to the mid slot
            let encoded = crate::address::HEAP_REF_FLAG | (ref_idx as i32);
            vm.write_word_at(vm.mid, encoded)?;
            Ok(())
        }
        _ => Err(ExecError::ThreadFault("indx on non-array".to_string())),
    }
}

/// indw src, dst — load word from pointer: dst = *src (word)
pub(crate) fn op_indw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_word()?;
    vm.set_dst_word(val)
}

/// indf src, dst — load real from pointer: dst = *src (real)
pub(crate) fn op_indf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_real()?;
    vm.set_dst_real(val)
}

/// indb src, dst — load byte from pointer: dst = *src (byte)
pub(crate) fn op_indb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_byte()?;
    vm.set_dst_byte(val)
}

/// indl src, dst — load big from pointer: dst = *src (big)
pub(crate) fn op_indl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_big()?;
    vm.set_dst_big(val)
}

/// lena src, dst — array length
pub(crate) fn op_lena(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let arr_id = vm.src_ptr()?;
    let len = if arr_id == heap::NIL {
        0
    } else {
        match vm.heap.get(arr_id) {
            Some(obj) => match &obj.data {
                crate::heap::HeapData::Array { length, .. } => *length as i32,
                _ => 0,
            },
            None => 0,
        }
    };
    vm.set_dst_word(len)
}

/// slicea src, mid, dst — slice an array: dst = dst[src..mid]
pub(crate) fn op_slicea(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let start = vm.src_word()? as usize;
    let end = vm.mid_word()? as usize;
    let arr_id = vm.dst_ptr()?;

    if arr_id == heap::NIL {
        if start == 0 && end == 0 {
            return Ok(());
        }
        return Err(ExecError::ThreadFault("slice of nil array".to_string()));
    }

    let obj = vm
        .heap
        .get(arr_id)
        .ok_or_else(|| ExecError::ThreadFault("slicea: invalid array".to_string()))?;

    match &obj.data {
        heap::HeapData::Array {
            elem_type,
            elem_size,
            data,
            length,
        } => {
            if end > *length || start > end {
                return Err(ExecError::ThreadFault(format!(
                    "array slice out of bounds: [{start}..{end}] for length {length}"
                )));
            }
            let new_len = end - start;
            let byte_start = start * elem_size;
            let byte_end = end * elem_size;
            let new_data = data[byte_start..byte_end].to_vec();
            let et = *elem_type;
            let es = *elem_size;

            let new_id = vm.heap.alloc(
                et,
                heap::HeapData::Array {
                    elem_type: et,
                    elem_size: es,
                    data: new_data,
                    length: new_len,
                },
            );
            vm.set_dst_ptr(new_id)?;
            vm.heap.dec_ref(arr_id);
            Ok(())
        }
        _ => Err(ExecError::ThreadFault("slicea on non-array".to_string())),
    }
}

/// slicela — slice with list assignment (same as slicea for now)
pub(crate) fn op_slicela(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    op_slicea(vm)
}
