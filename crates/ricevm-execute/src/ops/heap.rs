use ricevm_core::ExecError;

use crate::heap::HeapData;
use crate::vm::VmState;

/// new src, dst:allocate a record of the type given by src (type index)
pub(crate) fn op_new(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let type_idx = vm.src_word()? as usize;
    let size = vm
        .current_type_size(type_idx)
        .ok_or_else(|| ExecError::Other(format!("invalid type index: {type_idx}")))?;
    let id = vm
        .heap
        .alloc(type_idx as u32, HeapData::Record(vec![0; size]));
    vm.move_ptr_to_dst(id)
}

/// newz src, dst:same as new but data is guaranteed zero-initialized (which it already is)
pub(crate) fn op_newz(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    op_new(vm)
}

/// newa src, mid, dst:allocate an array of length src, element type mid
pub(crate) fn op_newa(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let length = vm.src_word()? as usize;
    let elem_type_idx = vm.mid_word()? as usize;
    let elem_size = vm
        .current_type_size(elem_type_idx)
        .ok_or_else(|| ExecError::Other(format!("invalid element type index: {elem_type_idx}")))?;
    let data = vec![0u8; length * elem_size];
    let id = vm.heap.alloc(
        elem_type_idx as u32,
        HeapData::Array {
            elem_type: elem_type_idx as u32,
            elem_size,
            data,
            length,
        },
    );
    vm.move_ptr_to_dst(id)
}

/// newaz:same as newa (zero-initialized)
pub(crate) fn op_newaz(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    op_newa(vm)
}

/// mnewz src, mid, dst: allocate and zero a record (same as newz for us)
pub(crate) fn op_mnewz(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    op_new(vm)
}

fn alloc_channel(vm: &mut VmState<'_>, elem_size: usize) -> Result<(), ExecError> {
    let id = vm.heap.alloc(
        0,
        HeapData::Channel {
            elem_size,
            pending: None,
        },
    );
    vm.move_ptr_to_dst(id)
}

pub(crate) fn op_newcb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    alloc_channel(vm, 1)
}
pub(crate) fn op_newcw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    alloc_channel(vm, 4)
}
pub(crate) fn op_newcf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    alloc_channel(vm, 8)
}
pub(crate) fn op_newcp(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    alloc_channel(vm, 4)
}
pub(crate) fn op_newcm(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let elem_size = vm.current_type_size(vm.src_word()? as usize).unwrap_or(4);
    alloc_channel(vm, elem_size)
}
pub(crate) fn op_newcmp(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let elem_size = vm.current_type_size(vm.src_word()? as usize).unwrap_or(4);
    alloc_channel(vm, elem_size)
}
pub(crate) fn op_newcl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    alloc_channel(vm, 8)
}

#[cfg(test)]
mod tests {
    use ricevm_core::{
        Header, Instruction, MiddleOperand, Module, Opcode, Operand, PointerMap, RuntimeFlags,
        TypeDescriptor, XMAGIC,
    };

    use super::*;
    use crate::address::AddrTarget;
    use crate::heap;
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
                type_size: 2,
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
            types: vec![
                TypeDescriptor {
                    id: 0,
                    size: 64,
                    pointer_map: PointerMap { bytes: vec![] },
                    pointer_count: 0,
                },
                TypeDescriptor {
                    id: 1,
                    size: 16,
                    pointer_map: PointerMap { bytes: vec![] },
                    pointer_count: 0,
                },
            ],
            data: vec![],
            name: "heap_test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        }
    }

    #[test]
    fn op_new_allocates_record() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let fp = vm.frames.current_data_offset();

        // src = type index 1 (size 16), dst = frame slot for the pointer
        vm.src = AddrTarget::Immediate;
        vm.imm_src = 1; // type index 1
        vm.dst = AddrTarget::Frame(fp);
        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);

        op_new(&mut vm).expect("op_new should succeed");

        let ptr = memory::read_word(&vm.frames.data, fp) as u32;
        assert_ne!(ptr, heap::NIL, "should allocate a non-nil heap object");
        assert!(
            ptr >= heap::HEAP_ID_BASE,
            "allocated id should be a valid heap id"
        );

        // Verify the allocated record has the right size
        let obj = vm.heap.get(ptr).expect("heap object should exist");
        match &obj.data {
            HeapData::Record(data) => {
                assert_eq!(data.len(), 16, "record should be 16 bytes (type 1 size)");
                assert!(
                    data.iter().all(|&b| b == 0),
                    "record data should be zero-initialized"
                );
            }
            other => panic!("expected Record, got {:?}", std::mem::discriminant(other)),
        }
    }

    #[test]
    fn op_newz_allocates_zeroed_record() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let fp = vm.frames.current_data_offset();

        vm.src = AddrTarget::Immediate;
        vm.imm_src = 0; // type index 0 (size 64)
        vm.dst = AddrTarget::Frame(fp);
        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);

        op_newz(&mut vm).expect("op_newz should succeed");

        let ptr = memory::read_word(&vm.frames.data, fp) as u32;
        assert_ne!(ptr, heap::NIL);

        let obj = vm.heap.get(ptr).expect("heap object should exist");
        match &obj.data {
            HeapData::Record(data) => {
                assert_eq!(data.len(), 64, "record should be 64 bytes (type 0 size)");
                assert!(
                    data.iter().all(|&b| b == 0),
                    "newz data should be zero-initialized"
                );
            }
            other => panic!("expected Record, got {:?}", std::mem::discriminant(other)),
        }
    }

    #[test]
    fn op_newa_allocates_array() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let fp = vm.frames.current_data_offset();

        // src = length 10, mid = element type index 1 (size 16)
        vm.src = AddrTarget::Immediate;
        vm.imm_src = 10; // length
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 1; // element type index
        vm.dst = AddrTarget::Frame(fp);
        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);

        op_newa(&mut vm).expect("op_newa should succeed");

        let ptr = memory::read_word(&vm.frames.data, fp) as u32;
        assert_ne!(ptr, heap::NIL);

        let obj = vm.heap.get(ptr).expect("heap object should exist");
        match &obj.data {
            HeapData::Array {
                elem_type,
                elem_size,
                data,
                length,
            } => {
                assert_eq!(*length, 10, "array length should be 10");
                assert_eq!(*elem_type, 1, "element type should be 1");
                assert_eq!(*elem_size, 16, "element size should be 16");
                assert_eq!(data.len(), 10 * 16, "data should be length * elem_size");
                assert!(
                    data.iter().all(|&b| b == 0),
                    "array data should be zero-initialized"
                );
            }
            other => panic!("expected Array, got {:?}", std::mem::discriminant(other)),
        }
    }

    #[test]
    fn op_newaz_allocates_zeroed_array() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let fp = vm.frames.current_data_offset();

        vm.src = AddrTarget::Immediate;
        vm.imm_src = 5;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 0; // element type 0, size 64
        vm.dst = AddrTarget::Frame(fp);
        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);

        op_newaz(&mut vm).expect("op_newaz should succeed");

        let ptr = memory::read_word(&vm.frames.data, fp) as u32;
        assert_ne!(ptr, heap::NIL);

        let obj = vm.heap.get(ptr).expect("heap object should exist");
        match &obj.data {
            HeapData::Array { data, length, .. } => {
                assert_eq!(*length, 5);
                assert!(
                    data.iter().all(|&b| b == 0),
                    "newaz data should be zero-initialized"
                );
            }
            other => panic!("expected Array, got {:?}", std::mem::discriminant(other)),
        }
    }

    #[test]
    fn op_new_invalid_type_returns_error() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let fp = vm.frames.current_data_offset();

        vm.src = AddrTarget::Immediate;
        vm.imm_src = 999; // invalid type index
        vm.dst = AddrTarget::Frame(fp);
        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);

        let err = op_new(&mut vm).expect_err("op_new with invalid type should fail");
        assert!(
            err.to_string().contains("invalid type index"),
            "error should mention invalid type index, got: {err}"
        );
    }

    #[test]
    fn op_newcb_allocates_byte_channel() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let fp = vm.frames.current_data_offset();

        vm.dst = AddrTarget::Frame(fp);
        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);

        op_newcb(&mut vm).expect("op_newcb should succeed");

        let ptr = memory::read_word(&vm.frames.data, fp) as u32;
        assert_ne!(ptr, heap::NIL);

        let obj = vm.heap.get(ptr).expect("heap object should exist");
        match &obj.data {
            HeapData::Channel { elem_size, pending } => {
                assert_eq!(*elem_size, 1, "byte channel elem_size should be 1");
                assert!(pending.is_none(), "new channel should have no pending data");
            }
            other => panic!("expected Channel, got {:?}", std::mem::discriminant(other)),
        }
    }

    #[test]
    fn op_newcw_allocates_word_channel() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let fp = vm.frames.current_data_offset();

        vm.dst = AddrTarget::Frame(fp);
        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);

        op_newcw(&mut vm).expect("op_newcw should succeed");

        let ptr = memory::read_word(&vm.frames.data, fp) as u32;
        let obj = vm.heap.get(ptr).expect("heap object should exist");
        match &obj.data {
            HeapData::Channel { elem_size, .. } => {
                assert_eq!(*elem_size, 4, "word channel elem_size should be 4");
            }
            other => panic!("expected Channel, got {:?}", std::mem::discriminant(other)),
        }
    }

    #[test]
    fn op_newcf_allocates_real_channel() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let fp = vm.frames.current_data_offset();

        vm.dst = AddrTarget::Frame(fp);
        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);

        op_newcf(&mut vm).expect("op_newcf should succeed");

        let ptr = memory::read_word(&vm.frames.data, fp) as u32;
        let obj = vm.heap.get(ptr).expect("heap object should exist");
        match &obj.data {
            HeapData::Channel { elem_size, .. } => {
                assert_eq!(*elem_size, 8, "real channel elem_size should be 8");
            }
            other => panic!("expected Channel, got {:?}", std::mem::discriminant(other)),
        }
    }
}
