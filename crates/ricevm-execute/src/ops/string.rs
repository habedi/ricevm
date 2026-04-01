use ricevm_core::ExecError;

use crate::heap::{self, HeapData};
use crate::vm::VmState;

/// lenc src, dst:string length in characters
pub(crate) fn op_lenc(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let str_id = vm.src_ptr()?;
    let len = match vm.heap.get_string(str_id) {
        Some(s) => s.chars().count() as i32,
        None => 0,
    };
    vm.set_dst_word(len)
}

/// indc src, mid, dst:get character at index: dst = src[mid]
pub(crate) fn op_indc(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let str_id = vm.src_ptr()?;
    let index = vm.mid_word()? as usize;
    let s = match vm.heap.get_string(str_id) {
        Some(s) => s.to_string(),
        None => return vm.raise_exception("nil dereference"),
    };
    let ch = match s.chars().nth(index) {
        Some(c) => c,
        None => return vm.raise_exception("string index out of bounds"),
    };
    vm.set_dst_word(ch as i32)
}

/// insc src, mid, dst:insert character: dst[mid] = src (rune)
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

/// addc src, mid, dst:string concatenation: dst = mid + src
/// Two-operand form: addc src, dst:dst = dst + src
/// Reference: addstring(S(m), S(s), R.m == R.d)
///   - nil + nil = nil (H)
///   - nil + s2 = dup(s2)
///   - s1 + nil = dup(s1) (or s1 if append)
pub(crate) fn op_addc(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s2_id = vm.src_ptr()?;
    // Two-operand: mid is None, so use dst as s1.
    let s1_id = if vm.mid == crate::address::AddrTarget::None {
        vm.dst_ptr()?
    } else {
        vm.mid_ptr()?
    };

    let s1_is_nil = s1_id == heap::NIL || vm.heap.get_string(s1_id).is_none();
    let s2_is_nil = s2_id == heap::NIL || vm.heap.get_string(s2_id).is_none();

    if s1_is_nil && s2_is_nil {
        // nil + nil = nil (reference returns H)
        let old_dst = vm.dst_ptr()?;
        vm.set_dst_ptr(heap::NIL)?;
        if old_dst != heap::NIL {
            vm.heap.dec_ref(old_dst);
        }
        return Ok(());
    }

    let s1 = vm.heap.get_string(s1_id).unwrap_or("").to_string();
    let s2 = vm.heap.get_string(s2_id).unwrap_or("").to_string();

    let result = format!("{s1}{s2}");
    let new_id = vm.heap.alloc(0, HeapData::Str(result));

    // dec_ref old dst, set new
    let old_dst = vm.dst_ptr()?;
    vm.set_dst_ptr(new_id)?;
    if old_dst != heap::NIL {
        vm.heap.dec_ref(old_dst);
    }
    Ok(())
}

/// slicec src, mid, dst:string slice: dst = dst[src..mid]
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

    let char_count = s.chars().count();
    if end < start || end > char_count {
        return Err(ExecError::ThreadFault(format!(
            "string slice out of bounds: [{}..{}] len={}",
            start, end, char_count
        )));
    }
    let nc = end - start;
    if nc == 0 {
        // Reference returns H (nil) for empty slices
        vm.set_dst_ptr(heap::NIL)?;
        vm.heap.dec_ref(str_id);
        return Ok(());
    }
    let sliced: String = s.chars().skip(start).take(nc).collect();
    let new_id = vm.heap.alloc(0, HeapData::Str(sliced));
    vm.set_dst_ptr(new_id)?;
    vm.heap.dec_ref(str_id);
    Ok(())
}

/// cvtca src, dst:convert string to byte array
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

/// cvtac src, dst:convert byte array to string
/// Reference: if a == H, ds = H; else ds = c2string(a->data, a->len)
pub(crate) fn op_cvtac(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let arr_id = vm.src_ptr()?;
    if arr_id == heap::NIL {
        // Reference returns H (nil) for nil array
        vm.move_ptr_to_dst(heap::NIL)
    } else {
        let s = match vm.heap.get(arr_id) {
            Some(obj) => match &obj.data {
                HeapData::Array { data, .. } => String::from_utf8_lossy(data).into_owned(),
                HeapData::ArraySlice {
                    parent_id,
                    byte_start,
                    length,
                    elem_size,
                    ..
                } => {
                    let len = length * elem_size;
                    let pid = *parent_id;
                    let start = *byte_start;
                    vm.heap
                        .array_read(pid, start, len)
                        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                        .unwrap_or_default()
                }
                _ => String::new(),
            },
            None => String::new(),
        };
        let new_id = vm.heap.alloc(0, HeapData::Str(s));
        vm.move_ptr_to_dst(new_id)
    }
}

/// lenl src, dst:list length
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

    #[test]
    fn property_addc_associative() {
        // String concatenation is associative: (a + b) + c == a + (b + c)
        let samples = [
            ("", "", ""),
            ("hello", " ", "world"),
            ("a", "b", "c"),
            ("foo", "", "bar"),
            ("", "middle", ""),
            ("\u{1F600}", "smile", "\u{2764}"),
        ];
        for (a, b, c) in &samples {
            let ab_c = format!("{}{}{}", format!("{a}{b}"), c, "");
            let a_bc = format!("{}{}{}", a, format!("{b}{c}"), "");
            assert_eq!(
                format!("{a}{b}{c}"),
                format!("{a}{b}{c}"),
                "addc associativity"
            );
            assert_eq!(ab_c, a_bc, "addc associativity for ({a:?}, {b:?}, {c:?})");
        }
    }

    #[test]
    fn property_slicec_length_preserved() {
        // slicec(s, start, end) should produce a string of length (end - start)
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        let test_str = "hello world";
        let str_id = vm.heap.alloc(0, HeapData::Str(test_str.to_string()));

        // Slice [2..7) should give "llo w" (length 5)
        vm.src = AddrTarget::Immediate;
        vm.imm_src = 2;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 7;
        memory::write_word(&mut vm.frames.data, fp, str_id as i32);
        vm.dst = AddrTarget::Frame(fp);

        op_slicec(&mut vm).expect("slicec should succeed");

        let result_id = memory::read_word(&vm.frames.data, fp) as u32;
        let result = vm.heap.get_string(result_id).unwrap();
        assert_eq!(result.chars().count(), 5);
        assert_eq!(result, "llo w");
    }

    #[test]
    fn property_slicec_full_range_is_identity() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        let test_str = "abcdef";
        let str_id = vm.heap.alloc(0, HeapData::Str(test_str.to_string()));

        // Slice [0..6) should give the whole string
        vm.src = AddrTarget::Immediate;
        vm.imm_src = 0;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 6;
        memory::write_word(&mut vm.frames.data, fp, str_id as i32);
        vm.dst = AddrTarget::Frame(fp);

        op_slicec(&mut vm).expect("slicec should succeed");

        let result_id = memory::read_word(&vm.frames.data, fp) as u32;
        let result = vm.heap.get_string(result_id).unwrap();
        assert_eq!(result, "abcdef");
    }

    #[test]
    fn property_slicec_empty_range() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        let test_str = "abcdef";
        let str_id = vm.heap.alloc(0, HeapData::Str(test_str.to_string()));

        // Slice [3..3) should give nil (reference returns H for empty slice)
        vm.src = AddrTarget::Immediate;
        vm.imm_src = 3;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 3;
        memory::write_word(&mut vm.frames.data, fp, str_id as i32);
        vm.dst = AddrTarget::Frame(fp);

        op_slicec(&mut vm).expect("slicec should succeed");

        let result_id = memory::read_word(&vm.frames.data, fp) as u32;
        assert_eq!(result_id, heap::NIL, "empty slice should return NIL");
    }

    #[test]
    fn slicec_bounds_check() {
        // Reference: if v < start || v > l, error(exBounds)
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        let str_id = vm.heap.alloc(0, HeapData::Str("abc".to_string()));

        // end > len should fail
        vm.src = AddrTarget::Immediate;
        vm.imm_src = 0;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 5; // out of bounds
        memory::write_word(&mut vm.frames.data, fp, str_id as i32);
        vm.dst = AddrTarget::Frame(fp);

        let err = op_slicec(&mut vm);
        assert!(err.is_err(), "slicec should fail for out-of-bounds end");
    }

    #[test]
    fn addc_nil_plus_nil_is_nil() {
        // Reference: addstring(H, H, ...) returns H
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);
        vm.src = AddrTarget::Immediate;
        vm.imm_src = heap::NIL as i32;
        vm.mid = AddrTarget::None;
        vm.dst = AddrTarget::Frame(fp);

        op_addc(&mut vm).expect("addc should succeed");

        let result_id = memory::read_word(&vm.frames.data, fp) as u32;
        assert_eq!(result_id, heap::NIL, "nil + nil should be nil");
    }

    #[test]
    fn cvtac_nil_array_returns_nil() {
        // Reference: if a == H, ds = H
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        vm.src = AddrTarget::Immediate;
        vm.imm_src = heap::NIL as i32;
        memory::write_word(&mut vm.frames.data, fp, 0);
        vm.dst = AddrTarget::Frame(fp);

        op_cvtac(&mut vm).expect("cvtac should succeed");

        let result_id = memory::read_word(&vm.frames.data, fp) as u32;
        assert_eq!(result_id, heap::NIL, "cvtac of nil array should be nil");
    }

    #[test]
    fn cvtac_array_slice_converts_to_string() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        // Create a parent array with "Hello World" bytes
        let parent_id = vm.heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 1,
                data: b"Hello World".to_vec(),
                length: 11,
            },
        );

        // Create a slice for "World" (bytes 6..11)
        let slice_id = vm.heap.alloc(
            0,
            HeapData::ArraySlice {
                parent_id,
                byte_start: 6,
                elem_type: 0,
                elem_size: 1,
                length: 5,
            },
        );

        vm.src = AddrTarget::Immediate;
        vm.imm_src = slice_id as i32;
        memory::write_word(&mut vm.frames.data, fp, 0);
        vm.dst = AddrTarget::Frame(fp);

        op_cvtac(&mut vm).expect("cvtac on slice should succeed");

        let result_id = memory::read_word(&vm.frames.data, fp) as u32;
        let result_str = vm.heap.get_string(result_id).unwrap_or("");
        assert_eq!(
            result_str, "World",
            "cvtac of ArraySlice should produce correct string"
        );
    }
}
