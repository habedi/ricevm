use ricevm_core::ExecError;

use crate::heap::{self, HeapData};
use crate::vm::VmState;

/// lenc src, dst — string length in characters
pub(crate) fn op_lenc(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let str_id = vm.src_ptr()?;
    let len = match vm.heap.get_string(str_id) {
        Some(s) => s.chars().count() as i32,
        None => 0,
    };
    vm.set_dst_word(len)
}

/// indc src, mid, dst — get character at index: dst = src[mid]
pub(crate) fn op_indc(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let str_id = vm.src_ptr()?;
    let index = vm.mid_word()? as usize;
    let s = vm
        .heap
        .get_string(str_id)
        .ok_or_else(|| ExecError::ThreadFault("nil string dereference".to_string()))?;
    let ch = s
        .chars()
        .nth(index)
        .ok_or_else(|| ExecError::ThreadFault(format!("string index out of bounds: {index}")))?;
    vm.set_dst_word(ch as i32)
}

/// insc src, mid, dst — insert character: dst[mid] = src (rune)
pub(crate) fn op_insc(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let rune_val = vm.src_word()? as u32;
    let index = vm.mid_word()? as usize;
    let str_id = vm.dst_ptr()?;

    let ch = char::from_u32(rune_val).unwrap_or('\u{FFFD}');

    if str_id == heap::NIL {
        // Create a new string
        let mut s = String::new();
        while s.chars().count() < index {
            s.push('\0');
        }
        if index == s.chars().count() {
            s.push(ch);
        }
        let new_id = vm.heap.alloc(0, HeapData::Str(s));
        vm.set_dst_ptr(new_id)?;
    } else {
        // Copy-on-write
        let (new_id, s) = vm
            .heap
            .cow_string(str_id)
            .ok_or_else(|| ExecError::ThreadFault("insc on non-string".to_string()))?;
        // Extend if needed
        while s.chars().count() <= index {
            s.push('\0');
        }
        // Replace character at index
        let byte_start = s
            .char_indices()
            .nth(index)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        let byte_end = s
            .char_indices()
            .nth(index + 1)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        s.replace_range(byte_start..byte_end, &ch.to_string());

        if new_id != str_id {
            vm.set_dst_ptr(new_id)?;
        }
    }
    Ok(())
}

/// addc src, mid, dst — string concatenation: dst = mid + src
/// Two-operand form: addc src, dst — dst = dst + src
pub(crate) fn op_addc(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s2_id = vm.src_ptr()?;
    // Two-operand: mid is None, so use dst as s1.
    let s1_id = if vm.mid == crate::address::AddrTarget::None {
        vm.dst_ptr()?
    } else {
        vm.mid_ptr()?
    };

    let s1 = vm.heap.get_string(s1_id).unwrap_or("").to_string();
    let s2 = vm.heap.get_string(s2_id).unwrap_or("").to_string();

    let result = format!("{s1}{s2}");
    let new_id = vm.heap.alloc(0, HeapData::Str(result));

    // dec_ref old dst, set new
    let old_dst = vm.dst_ptr()?;
    vm.set_dst_ptr(new_id)?;
    vm.heap.dec_ref(old_dst);
    Ok(())
}

/// slicec src, mid, dst — string slice: dst = dst[src..mid]
pub(crate) fn op_slicec(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let start = vm.src_word()? as usize;
    let end = vm.mid_word()? as usize;
    let str_id = vm.dst_ptr()?;

    if str_id == heap::NIL {
        if start == 0 && end == 0 {
            return Ok(()); // nil slice of nil is nil
        }
        return Err(ExecError::ThreadFault("slice of nil string".to_string()));
    }

    let s = vm
        .heap
        .get_string(str_id)
        .ok_or_else(|| ExecError::ThreadFault("slicec on non-string".to_string()))?
        .to_string();

    let sliced: String = s.chars().skip(start).take(end - start).collect();
    let new_id = vm.heap.alloc(0, HeapData::Str(sliced));
    vm.set_dst_ptr(new_id)?;
    vm.heap.dec_ref(str_id);
    Ok(())
}

/// cvtca src, dst — convert string to byte array
pub(crate) fn op_cvtca(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let str_id = vm.src_ptr()?;
    let bytes = match vm.heap.get_string(str_id) {
        Some(s) => s.as_bytes().to_vec(),
        None => Vec::new(),
    };
    let length = bytes.len();
    let id = vm.heap.alloc(
        0,
        HeapData::Array {
            elem_type: 0,
            elem_size: 1,
            data: bytes,
            length,
        },
    );
    vm.move_ptr_to_dst(id)
}

/// cvtac src, dst — convert byte array to string
pub(crate) fn op_cvtac(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let arr_id = vm.src_ptr()?;
    let s = if arr_id == heap::NIL {
        String::new()
    } else {
        match vm.heap.get(arr_id) {
            Some(obj) => match &obj.data {
                HeapData::Array { data, .. } => String::from_utf8_lossy(data).into_owned(),
                _ => String::new(),
            },
            None => String::new(),
        }
    };
    let new_id = vm.heap.alloc(0, HeapData::Str(s));
    vm.move_ptr_to_dst(new_id)
}

/// lenl src, dst — list length
pub(crate) fn op_lenl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let mut list_id = vm.src_ptr()?;
    let mut len = 0;

    while list_id != heap::NIL {
        let obj = vm
            .heap
            .get(list_id)
            .ok_or_else(|| ExecError::ThreadFault("nil list dereference (lenl)".to_string()))?;
        list_id = match &obj.data {
            HeapData::List { tail, .. } => *tail,
            _ => return Err(ExecError::ThreadFault("lenl on non-list".to_string())),
        };
        len += 1;
    }

    vm.set_dst_word(len)
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
                size: 32,
                pointer_map: PointerMap { bytes: vec![] },
                pointer_count: 0,
            }],
            data: vec![],
            name: "lenl_test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        }
    }

    #[test]
    fn lenl_counts_list_nodes() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");

        let tail = vm.heap.alloc(
            0,
            HeapData::List {
                head: vec![20, 0, 0, 0],
                tail: heap::NIL,
            },
        );
        let head = vm.heap.alloc(
            0,
            HeapData::List {
                head: vec![10, 0, 0, 0],
                tail,
            },
        );

        let fp_base = vm.frames.current_data_offset();
        vm.src = AddrTarget::Immediate;
        vm.imm_src = head as i32;
        vm.dst = AddrTarget::Frame(fp_base);

        op_lenl(&mut vm).expect("lenl should succeed");

        assert_eq!(memory::read_word(&vm.frames.data, fp_base), 2);
    }

    #[test]
    fn lenl_rejects_non_list_values() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let string_id = vm.heap.alloc(0, HeapData::Str("not a list".to_string()));

        vm.src = AddrTarget::Immediate;
        vm.imm_src = string_id as i32;

        let err = op_lenl(&mut vm).expect_err("lenl should reject non-list values");
        assert!(err.to_string().contains("lenl on non-list"));
    }
}
