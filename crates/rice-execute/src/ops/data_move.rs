use ricevm_core::ExecError;

use crate::address::AddrTarget;
use crate::heap::{self, HeapData};
use crate::vm::VmState;

pub(crate) fn op_movw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_word()?;
    vm.set_dst_word(val)
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
    let val = vm.src_big()?;
    vm.set_dst_big(val)
}

/// movm src, mid, dst — copy a block of `mid` bytes from src to dst
pub(crate) fn op_movm(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let size = vm.mid_word()? as usize;
    if size == 0 {
        return Ok(());
    }

    let src_bytes = read_block(vm, vm.src, size);
    write_block(vm, vm.dst, &src_bytes);
    Ok(())
}

/// movmp src, mid, dst — copy a block with pointer tracking (same as movm for now)
pub(crate) fn op_movmp(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    op_movm(vm)
}

/// movpc src, dst — move program counter (word) to dst
pub(crate) fn op_movpc(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_word()?;
    vm.set_dst_word(val)
}

/// tcmp src, dst — type compare for two pointers.
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

/// self dst — store the current module pointer into dst
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
            if off < vm.mp.len() {
                let copy_len = size.min(vm.mp.len() - off);
                buf[..copy_len].copy_from_slice(&vm.mp[off..off + copy_len]);
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
            if off < vm.mp.len() {
                let copy_len = data.len().min(vm.mp.len() - off);
                vm.mp[off..off + copy_len].copy_from_slice(&data[..copy_len]);
            }
        }
        AddrTarget::HeapArray { id, offset } => {
            if let Some(obj) = vm.heap.get_mut(id) {
                match &mut obj.data {
                    HeapData::Array { data: buf, .. } | HeapData::Record(buf) => {
                        if offset < buf.len() {
                            let copy_len = data.len().min(buf.len() - offset);
                            buf[offset..offset + copy_len].copy_from_slice(&data[..copy_len]);
                        }
                    }
                    _ => {}
                }
            }
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
}
