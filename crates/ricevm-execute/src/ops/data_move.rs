use ricevm_core::ExecError;

use crate::address::AddrTarget;
use crate::heap::{self, HeapData};
use crate::vm::VmState;

pub(crate) fn op_movw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    vm.set_dst_word(vm.src_word()?)
}

pub(crate) fn op_movb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_byte()?;
    vm.set_dst_byte(val)
}

pub(crate) fn op_movf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_real()?;
    vm.set_dst_real(val)
}

pub(crate) fn op_movl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    vm.set_dst_big(vm.src_big()?)
}

/// movm src, mid, dst:copy a block of `mid` bytes from src to dst
pub(crate) fn op_movm(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let size = vm.mid_word()? as usize;
    if size == 0 {
        return Ok(());
    }

    let src_bytes = read_block(vm, vm.src, size);
    write_block(vm, vm.dst, &src_bytes);
    Ok(())
}

/// movmp src, mid, dst:copy a typed record block.
/// Unlike movm where mid is a byte count, mid is a type descriptor index.
/// The actual size to copy comes from types[mid].size.
pub(crate) fn op_movmp(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let type_idx = vm.mid_word()? as usize;
    let types = if let Some(lm_idx) = vm.current_loaded_module {
        vm.loaded_modules.get(lm_idx).map(|lm| &lm.module.types)
    } else {
        Some(&vm.module.types)
    };
    let (size, ptr_map) = if let Some(td) = types.and_then(|t| t.get(type_idx)) {
        (td.size as usize, td.pointer_map.bytes.clone())
    } else {
        (type_idx, Vec::new()) // fallback to raw value if type not found
    };
    if size == 0 {
        return Ok(());
    }

    // incmem: increment ref counts for pointers in the source (before copy)
    let src_bytes = read_block(vm, vm.src, size);
    for (byte_idx, &map_byte) in ptr_map.iter().enumerate() {
        for bit in 0..8u32 {
            if map_byte & (1 << bit) != 0 {
                let ptr_offset = (byte_idx * 8 + bit as usize) * 4;
                if ptr_offset + 4 <= src_bytes.len() {
                    let ptr_val = crate::memory::read_word(&src_bytes, ptr_offset) as u32;
                    if ptr_val != crate::heap::NIL
                        && ptr_val >= crate::heap::HEAP_ID_BASE
                        && vm.heap.contains(ptr_val)
                    {
                        vm.heap.inc_ref(ptr_val);
                    }
                }
            }
        }
    }

    // freeptrs: decrement ref counts for pointers in the destination (about to be overwritten)
    let dst_bytes = read_block(vm, vm.dst, size);
    for (byte_idx, &map_byte) in ptr_map.iter().enumerate() {
        for bit in 0..8u32 {
            if map_byte & (1 << bit) != 0 {
                let ptr_offset = (byte_idx * 8 + bit as usize) * 4;
                if ptr_offset + 4 <= dst_bytes.len() {
                    let ptr_val = crate::memory::read_word(&dst_bytes, ptr_offset) as u32;
                    if ptr_val != crate::heap::NIL
                        && ptr_val >= crate::heap::HEAP_ID_BASE
                        && vm.heap.contains(ptr_val)
                    {
                        vm.heap.dec_ref(ptr_val);
                    }
                }
            }
        }
    }

    // Copy the bytes
    write_block(vm, vm.dst, &src_bytes);
    Ok(())
}

/// movpc src, dst:move program counter (word) to dst
pub(crate) fn op_movpc(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_word()?;
    vm.set_dst_word(val)
}

/// tcmp src, dst:type compare for two pointers.
pub(crate) fn op_tcmp(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let src_id = vm.src_ptr()?;
    let dst_id = vm.dst_ptr()?;

    if dst_id == heap::NIL {
        return Err(ExecError::ThreadFault("typecheck".to_string()));
    }

    let dst_obj = vm
        .heap
        .get(dst_id)
        .ok_or_else(|| ExecError::ThreadFault("typecheck".to_string()))?;

    if src_id == heap::NIL {
        return Ok(());
    }

    let src_obj = vm
        .heap
        .get(src_id)
        .ok_or_else(|| ExecError::ThreadFault("typecheck".to_string()))?;

    if src_obj.type_id == dst_obj.type_id {
        Ok(())
    } else {
        Err(ExecError::ThreadFault("typecheck".to_string()))
    }
}

/// self dst:store the current module pointer into dst
pub(crate) fn op_self_(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let module_ref = if let Some(module_idx) = vm.current_loaded_module {
        vm.heap.alloc(
            0,
            HeapData::LoadedModule {
                module_idx,
                func_map: Vec::new(),
            },
        )
    } else {
        vm.heap.alloc(
            0,
            HeapData::MainModule {
                func_map: Vec::new(),
            },
        )
    };
    vm.move_ptr_to_dst(module_ref)
}

fn read_block(vm: &VmState<'_>, target: AddrTarget, size: usize) -> Vec<u8> {
    let mut buf = vec![0u8; size];
    match target {
        AddrTarget::Frame(off) => {
            if off < vm.frames.data.len() {
                let copy_len = size.min(vm.frames.data.len() - off);
                buf[..copy_len].copy_from_slice(&vm.frames.data[off..off + copy_len]);
            }
        }
        AddrTarget::Mp(off) => {
            if off + size <= vm.mp.len() {
                buf.copy_from_slice(&vm.mp[off..off + size]);
            }
        }
        AddrTarget::ModuleMp { module_idx, offset } => {
            let mp = match vm.module_mp(module_idx) {
                Some(mp) => mp,
                None => return buf,
            };
            if offset + size <= mp.len() {
                buf.copy_from_slice(&mp[offset..offset + size]);
            }
        }
        AddrTarget::HeapArray { id, offset } => {
            if let Some(bytes) = vm.heap_slice(id, offset, size) {
                buf.copy_from_slice(&bytes);
            }
        }
        AddrTarget::Immediate | AddrTarget::None => {}
    }
    buf
}

fn write_block(vm: &mut VmState<'_>, target: AddrTarget, data: &[u8]) {
    match target {
        AddrTarget::Frame(off) => {
            if off < vm.frames.data.len() {
                let copy_len = data.len().min(vm.frames.data.len() - off);
                vm.frames.data[off..off + copy_len].copy_from_slice(&data[..copy_len]);
            }
        }
        AddrTarget::Mp(off) => {
            if off + data.len() <= vm.mp.len() {
                vm.mp[off..off + data.len()].copy_from_slice(data);
            }
        }
        AddrTarget::ModuleMp { module_idx, offset } => {
            if let Some(mp) = vm.module_mp_mut(module_idx)
                && offset + data.len() <= mp.len()
            {
                mp[offset..offset + data.len()].copy_from_slice(data);
            }
        }
        AddrTarget::HeapArray { id, offset } => {
            vm.heap_write(id, offset, data);
        }
        AddrTarget::Immediate | AddrTarget::None => {}
    }
}

#[cfg(test)]
mod tests {
    use ricevm_core::{
        Header, Instruction, MiddleOperand, Module, Opcode, Operand, PointerMap, RuntimeFlags,
        TypeDescriptor, XMAGIC,
    };

    use super::*;
    use crate::ops::pointer::op_movp;

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
            name: "data_move_test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        }
    }

    #[test]
    fn tcmp_allows_nil_source() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let dst_id = vm.heap.alloc(7, HeapData::Record(vec![0; 8]));

        vm.src = AddrTarget::Immediate;
        vm.imm_src = heap::NIL as i32;
        vm.dst = AddrTarget::Immediate;
        vm.imm_dst = dst_id as i32;

        op_tcmp(&mut vm).expect("nil source should pass typecheck");
    }

    #[test]
    fn tcmp_accepts_matching_type_ids_without_overwriting_dst() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let src_id = vm.heap.alloc(7, HeapData::Record(vec![0; 8]));
        let dst_id = vm.heap.alloc(7, HeapData::Record(vec![0; 8]));
        let fp_base = vm.frames.current_data_offset();

        crate::memory::write_word(&mut vm.frames.data, fp_base, dst_id as i32);
        vm.src = AddrTarget::Immediate;
        vm.imm_src = src_id as i32;
        vm.dst = AddrTarget::Frame(fp_base);

        op_tcmp(&mut vm).expect("matching types should pass");

        assert_eq!(
            crate::memory::read_word(&vm.frames.data, fp_base),
            dst_id as i32
        );
    }

    #[test]
    fn tcmp_rejects_mismatched_type_ids() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let src_id = vm.heap.alloc(7, HeapData::Record(vec![0; 8]));
        let dst_id = vm.heap.alloc(8, HeapData::Record(vec![0; 8]));

        vm.src = AddrTarget::Immediate;
        vm.imm_src = src_id as i32;
        vm.dst = AddrTarget::Immediate;
        vm.imm_dst = dst_id as i32;

        let err = op_tcmp(&mut vm).expect_err("mismatched types should fail");
        assert!(matches!(err, ExecError::ThreadFault(msg) if msg == "typecheck"));
    }

    #[test]
    fn movm_zero_fills_out_of_bounds_source_bytes() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let fp_base = vm.frames.current_data_offset();

        vm.frames.data[fp_base + 60..fp_base + 64].copy_from_slice(&[1, 2, 3, 4]);
        vm.src = AddrTarget::Frame(fp_base + 60);
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 8;
        vm.dst = AddrTarget::Frame(fp_base + 32);

        op_movm(&mut vm).expect("movm should succeed");

        assert_eq!(
            &vm.frames.data[fp_base + 32..fp_base + 40],
            &[1, 2, 3, 4, 0, 0, 0, 0]
        );
    }

    #[test]
    fn movm_caps_out_of_bounds_destination_writes() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let fp_base = vm.frames.current_data_offset();

        vm.frames.data[fp_base..fp_base + 8].copy_from_slice(&[9, 8, 7, 6, 5, 4, 3, 2]);
        vm.src = AddrTarget::Frame(fp_base);
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 8;
        vm.dst = AddrTarget::Frame(fp_base + 62);

        op_movm(&mut vm).expect("movm should succeed");

        assert_eq!(&vm.frames.data[fp_base + 62..fp_base + 64], &[9, 8]);
    }

    #[test]
    fn movmp_uses_type_descriptor_size() {
        // Create a module with type index 1 having size=8
        let mut module = test_module();
        module.types.push(TypeDescriptor {
            id: 1,
            size: 8,
            pointer_map: PointerMap { bytes: vec![] },
            pointer_count: 0,
        });

        let mut vm = VmState::new(&module).expect("vm should initialize");
        let fp_base = vm.frames.current_data_offset();

        // Write 16 bytes of recognizable data at source
        vm.frames.data[fp_base..fp_base + 16]
            .copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);

        vm.src = AddrTarget::Frame(fp_base);
        // mid = type index 1, which has size=8
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 1;
        vm.dst = AddrTarget::Frame(fp_base + 32);

        op_movmp(&mut vm).expect("movmp should succeed");

        // Only 8 bytes should be copied (types[1].size=8), not 1 byte (raw index)
        assert_eq!(
            &vm.frames.data[fp_base + 32..fp_base + 40],
            &[1, 2, 3, 4, 5, 6, 7, 8]
        );
        // Bytes beyond the 8 copied should remain zero
        assert_eq!(
            &vm.frames.data[fp_base + 40..fp_base + 48],
            &[0, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    /// Regression: movmp with type_idx > type.size must not copy more bytes
    /// than the type descriptor specifies. The old code fell back to using the
    /// raw type_idx as byte count when the type was not found, which could
    /// overcopy and corrupt adjacent frame data.
    #[test]
    fn movmp_does_not_overcopy() {
        // Create a module with type 0 (size=64) and type 1 (size=8)
        let mut module = test_module();
        module.types.push(TypeDescriptor {
            id: 1,
            size: 8,
            pointer_map: PointerMap { bytes: vec![] },
            pointer_count: 0,
        });

        let mut vm = VmState::new(&module).expect("vm init");
        let fp_base = vm.frames.current_data_offset();

        // Source: 16 bytes of recognizable data
        vm.frames.data[fp_base..fp_base + 16].copy_from_slice(&[0xAA; 16]);

        // Place a sentinel pattern after the destination area
        let dst_off = fp_base + 32;
        vm.frames.data[dst_off + 8..dst_off + 16].copy_from_slice(&[0xBB; 8]);

        vm.src = AddrTarget::Frame(fp_base);
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 1; // type_idx=1, size=8
        vm.dst = AddrTarget::Frame(dst_off);

        op_movmp(&mut vm).expect("movmp should succeed");

        // Only 8 bytes should be copied
        assert_eq!(
            &vm.frames.data[dst_off..dst_off + 8],
            &[0xAA; 8],
            "movmp should copy exactly type.size bytes"
        );
        // The sentinel after should be untouched
        assert_eq!(
            &vm.frames.data[dst_off + 8..dst_off + 16],
            &[0xBB; 8],
            "movmp should not overcopy into adjacent data"
        );
    }

    #[test]
    fn movw_copies_word_between_frame_locations() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        crate::memory::write_word(&mut vm.frames.data, fp, 12345);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 4);

        op_movw(&mut vm).expect("movw should succeed");
        assert_eq!(crate::memory::read_word(&vm.frames.data, fp + 4), 12345);
    }

    #[test]
    fn movw_copies_negative_word() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        crate::memory::write_word(&mut vm.frames.data, fp, -42);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 4);

        op_movw(&mut vm).expect("movw should succeed");
        assert_eq!(crate::memory::read_word(&vm.frames.data, fp + 4), -42);
    }

    #[test]
    fn movw_immediate_to_frame() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        vm.src = AddrTarget::Immediate;
        vm.imm_src = 9999;
        vm.dst = AddrTarget::Frame(fp);

        op_movw(&mut vm).expect("movw from immediate should succeed");
        assert_eq!(crate::memory::read_word(&vm.frames.data, fp), 9999);
    }

    #[test]
    fn movb_copies_byte() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        vm.frames.data[fp] = 0xAB;
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 1);

        op_movb(&mut vm).expect("movb should succeed");
        assert_eq!(vm.frames.data[fp + 1], 0xAB);
    }

    #[test]
    fn movb_immediate_to_frame() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        vm.src = AddrTarget::Immediate;
        vm.imm_src = 0x42;
        vm.dst = AddrTarget::Frame(fp + 2);

        op_movb(&mut vm).expect("movb from immediate should succeed");
        assert_eq!(vm.frames.data[fp + 2], 0x42);
    }

    #[test]
    fn movf_copies_float() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        crate::memory::write_real(&mut vm.frames.data, fp, 3.14159);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 8);

        op_movf(&mut vm).expect("movf should succeed");
        let result = crate::memory::read_real(&vm.frames.data, fp + 8);
        assert!((result - 3.14159).abs() < f64::EPSILON);
    }

    #[test]
    fn movf_negative_float() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        crate::memory::write_real(&mut vm.frames.data, fp, -2.71828);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 8);

        op_movf(&mut vm).expect("movf should succeed");
        let result = crate::memory::read_real(&vm.frames.data, fp + 8);
        assert!((result - (-2.71828)).abs() < f64::EPSILON);
    }

    #[test]
    fn movl_copies_big_value() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        crate::memory::write_big(&mut vm.frames.data, fp, 0x0102030405060708i64);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 8);

        op_movl(&mut vm).expect("movl should succeed");
        assert_eq!(
            crate::memory::read_big(&vm.frames.data, fp + 8),
            0x0102030405060708i64
        );
    }

    #[test]
    fn movl_negative_big() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        crate::memory::write_big(&mut vm.frames.data, fp, -1i64);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 8);

        op_movl(&mut vm).expect("movl should succeed");
        assert_eq!(crate::memory::read_big(&vm.frames.data, fp + 8), -1i64);
    }

    #[test]
    fn movp_moves_pointer_with_refcounting() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        let id = vm.heap.alloc(0, HeapData::Record(vec![0; 8]));
        assert_eq!(vm.heap.get(id).unwrap().ref_count, 1);

        // Store the pointer in src
        crate::memory::write_word(&mut vm.frames.data, fp, id as i32);
        vm.src = AddrTarget::Frame(fp);
        // dst starts as NIL
        crate::memory::write_word(&mut vm.frames.data, fp + 4, heap::NIL as i32);
        vm.dst = AddrTarget::Frame(fp + 4);

        op_movp(&mut vm).expect("movp should succeed");

        // dst should now hold the pointer
        assert_eq!(crate::memory::read_word(&vm.frames.data, fp + 4), id as i32);
        // ref count should be incremented (src_ptr reads, move_ptr_to_dst increments)
        assert_eq!(vm.heap.get(id).unwrap().ref_count, 2);
    }

    #[test]
    fn movm_copies_block_of_bytes() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        vm.frames.data[fp..fp + 6].copy_from_slice(&[10, 20, 30, 40, 50, 60]);
        vm.src = AddrTarget::Frame(fp);
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 6;
        vm.dst = AddrTarget::Frame(fp + 16);

        op_movm(&mut vm).expect("movm should succeed");
        assert_eq!(&vm.frames.data[fp + 16..fp + 22], &[10, 20, 30, 40, 50, 60]);
    }

    #[test]
    fn movm_zero_size_is_noop() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        vm.frames.data[fp + 16] = 0xFF;
        vm.src = AddrTarget::Frame(fp);
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 0;
        vm.dst = AddrTarget::Frame(fp + 16);

        op_movm(&mut vm).expect("movm with size 0 should succeed");
        assert_eq!(vm.frames.data[fp + 16], 0xFF, "dst should be unchanged");
    }

    #[test]
    fn movpc_copies_program_counter_value() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        vm.src = AddrTarget::Immediate;
        vm.imm_src = 42;
        vm.dst = AddrTarget::Frame(fp);

        op_movpc(&mut vm).expect("movpc should succeed");
        assert_eq!(crate::memory::read_word(&vm.frames.data, fp), 42);
    }
}
