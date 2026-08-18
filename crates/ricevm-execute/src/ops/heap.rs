use ricevm_core::ExecError;

use crate::heap::HeapData;
use crate::vm::VmState;

/// Dis exception for an array allocation with a negative length
/// (Inferno: `acheck` -> `error(exNegsize)`).
const NEGATIVE_ARRAY_SIZE: &str = "negative array size";

/// Dis exception for an allocation the heap cannot serve
/// (Inferno: `acheck` -> `error(exHeap)`).
const OUT_OF_MEMORY: &str = "out of memory: heap";

/// Largest array a single allocation may produce, shared by `newa` here and
/// by the module data section in `data.rs` so the run-time and load-time
/// limits cannot drift apart.
///
/// This is a per-request cap, not a memory budget: it does not promise the
/// host can satisfy a request of this size, and a program making many
/// requests can still exhaust memory. What it does guarantee is that a single
/// hostile length from untrusted bytecode — `i32::MAX` elements of a large
/// type — is turned into a Dis `exHeap` exception the program can handle,
/// rather than an allocation the process has no chance of serving. 64 MiB is
/// far above any array a real Limbo program builds in one go.
pub(crate) const MAX_ARRAY_BYTES: usize = 64 * 1024 * 1024;

/// new src, dst:allocate a record of the type given by src (type index)
pub(crate) fn op_new(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let type_idx = vm.src_word()? as usize;
    let size = vm
        .current_type_size(type_idx)
        .ok_or_else(|| ExecError::Other(format!("invalid type index: {type_idx}")))?;
    // Resolve the record's pointer map now: after this instruction nothing can
    // tell which module's type index `type_idx` was.
    let trace = vm.trace_map_for_type(type_idx);
    let id = vm
        .heap
        .alloc_typed(type_idx as u32, HeapData::Record(vec![0; size]), trace);
    vm.move_ptr_to_dst(id)
}

/// newz src, dst:same as new but data is guaranteed zero-initialized (which it already is)
pub(crate) fn op_newz(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    op_new(vm)
}

/// newa src, mid, dst:allocate an array of length src, element type mid
pub(crate) fn op_newa(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let raw_length = vm.src_word()?;
    // Reference: acheck() raises exNegsize for a negative length and exHeap
    // when length * element size overflows the allocation.
    if raw_length < 0 {
        return vm.raise_exception(NEGATIVE_ARRAY_SIZE);
    }
    let length = raw_length as usize;
    let elem_type_idx = vm.mid_word()? as usize;
    let elem_size = vm
        .current_type_size(elem_type_idx)
        .ok_or_else(|| ExecError::Other(format!("invalid element type index: {elem_type_idx}")))?;
    let byte_len = match length.checked_mul(elem_size) {
        Some(n) if n <= MAX_ARRAY_BYTES => n,
        _ => return vm.raise_exception(OUT_OF_MEMORY),
    };
    let data = vec![0u8; byte_len];
    // The element type's map, repeated once per element by `TraceMap`.
    let trace = vm.trace_map_for_type(elem_type_idx);
    let id = vm.heap.alloc_typed(
        elem_type_idx as u32,
        HeapData::Array {
            elem_type: elem_type_idx as u32,
            elem_size,
            data,
            length,
        },
        trace,
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
                type_size: 3,
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
                // Two words, of which only the second is a pointer. The map is
                // most-significant-bit first, as the .dis format writes it.
                TypeDescriptor {
                    id: 2,
                    size: 8,
                    pointer_map: PointerMap { bytes: vec![0x40] },
                    pointer_count: 1,
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
    fn op_newa_negative_length_is_rejected() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let fp = vm.frames.current_data_offset();

        vm.src = AddrTarget::Immediate;
        vm.imm_src = -1; // negative length from untrusted bytecode
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 1; // element type index 1 (size 16)
        vm.dst = AddrTarget::Frame(fp);
        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);

        let err = op_newa(&mut vm).expect_err("negative array size must be rejected");
        assert!(
            err.to_string().contains("negative array size"),
            "expected negative array size, got: {err}"
        );
    }

    #[test]
    fn op_newaz_negative_length_is_rejected() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let fp = vm.frames.current_data_offset();

        vm.src = AddrTarget::Immediate;
        vm.imm_src = i32::MIN;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 0;
        vm.dst = AddrTarget::Frame(fp);
        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);

        let err = op_newaz(&mut vm).expect_err("negative array size must be rejected");
        assert!(
            err.to_string().contains("negative array size"),
            "expected negative array size, got: {err}"
        );
    }

    #[test]
    fn op_newa_oversized_length_is_rejected() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let fp = vm.frames.current_data_offset();

        vm.src = AddrTarget::Immediate;
        vm.imm_src = i32::MAX; // * 16 bytes/elem is far beyond any real heap
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 1;
        vm.dst = AddrTarget::Frame(fp);
        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);

        let err = op_newa(&mut vm).expect_err("oversized array must be rejected");
        assert!(
            err.to_string().contains("out of memory"),
            "expected out of memory, got: {err}"
        );
    }

    #[test]
    fn array_cap_is_the_agreed_limit() {
        // One limit, one definition. Module data (`data.rs`) allocates
        // arrays against this same constant, so neither path can hand out
        // an array the other would refuse.
        assert_eq!(MAX_ARRAY_BYTES, 64 * 1024 * 1024);
    }

    #[test]
    fn op_newa_respects_the_same_cap_as_the_data_section() {
        // Just over the cap: small enough that the old 2 GiB limit let it
        // through, so `newa` used to hand out an array 32x larger than the
        // identical request made through the module data section.
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let fp = vm.frames.current_data_offset();

        let over_cap: i32 = 64 * 1024 * 1024 / 16 + 1;
        vm.src = AddrTarget::Immediate;
        vm.imm_src = over_cap;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 1; // element type index 1 (size 16)
        vm.dst = AddrTarget::Frame(fp);
        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);

        let err = op_newa(&mut vm).expect_err("an array over the cap must be rejected");
        assert!(
            err.to_string().contains("out of memory"),
            "expected out of memory, got: {err}"
        );
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

    /// Allocate through `op_new` and hand back the object's trace map offsets.
    fn new_of_type(vm: &mut VmState<'_>, type_idx: i32) -> (u32, Option<Vec<usize>>) {
        let fp = vm.frames.current_data_offset();
        vm.src = AddrTarget::Immediate;
        vm.imm_src = type_idx;
        vm.dst = AddrTarget::Frame(fp);
        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);
        op_new(vm).expect("op_new should succeed");
        let id = memory::read_word(&vm.frames.data, fp) as u32;
        let obj = vm.heap.get(id).expect("heap object should exist");
        let size = match &obj.data {
            HeapData::Record(data) => data.len(),
            other => panic!("expected Record, got {:?}", std::mem::discriminant(other)),
        };
        let offsets = obj
            .trace
            .as_ref()
            .map(|map| map.pointer_offsets(size).collect());
        (id, offsets)
    }

    /// The type index alone cannot be resolved later -- index 2 means something
    /// different in every module -- so the descriptor is resolved here, where
    /// the module is known, and the map travels with the object.
    #[test]
    fn op_new_takes_the_pointer_map_from_the_type_descriptor() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");

        let (_, offsets) = new_of_type(&mut vm, 2);

        assert_eq!(
            offsets,
            Some(vec![4]),
            "type 2's second word is its only pointer"
        );
    }

    #[test]
    fn op_new_records_an_empty_map_for_a_pointerless_type() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");

        let (_, offsets) = new_of_type(&mut vm, 1);

        assert_eq!(
            offsets,
            Some(Vec::new()),
            "a type with no pointers is traced precisely as holding none, \
             which is what stops its buffer retaining objects by coincidence"
        );
    }

    #[test]
    fn op_new_shares_one_map_between_objects_of_the_same_type() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");

        let (first, _) = new_of_type(&mut vm, 2);
        let (second, _) = new_of_type(&mut vm, 2);

        let first_map = vm.heap.get(first).unwrap().trace.clone().unwrap();
        let second_map = vm.heap.get(second).unwrap().trace.clone().unwrap();
        assert!(
            std::sync::Arc::ptr_eq(&first_map, &second_map),
            "one map per type, not a copy per object"
        );
    }

    #[test]
    fn op_newa_takes_the_pointer_map_from_the_element_type() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let fp = vm.frames.current_data_offset();

        vm.src = AddrTarget::Immediate;
        vm.imm_src = 3; // three elements
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 2; // of type 2: two words, the second a pointer
        vm.dst = AddrTarget::Frame(fp);
        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);

        op_newa(&mut vm).expect("op_newa should succeed");

        let id = memory::read_word(&vm.frames.data, fp) as u32;
        let obj = vm.heap.get(id).expect("heap object should exist");
        let map = obj.trace.as_ref().expect("array carries its element map");
        assert_eq!(
            map.pointer_offsets(24).collect::<Vec<_>>(),
            vec![4, 12, 20],
            "the element map repeats once per element"
        );
    }

    /// The false retention this precision is for: bytes read into a buffer can
    /// spell out a live id, and used to keep that object alive indefinitely.
    #[test]
    fn a_byte_array_does_not_retain_an_object_by_coincidence() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let fp = vm.frames.current_data_offset();

        let victim = vm.heap.alloc(0, HeapData::Str("unreferenced".to_string()));

        // `array[16] of byte`, whose element type has no pointers.
        vm.src = AddrTarget::Immediate;
        vm.imm_src = 16;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 1; // pointerless element type
        vm.dst = AddrTarget::Frame(fp);
        memory::write_word(&mut vm.frames.data, fp, heap::NIL as i32);
        op_newa(&mut vm).expect("op_newa should succeed");
        let buf = memory::read_word(&vm.frames.data, fp) as u32;

        // What `sys->read` would have left in it.
        let mut payload = vec![0u8; 8];
        memory::write_word(&mut payload, 0, victim as i32);
        vm.heap.array_write(buf, 0, &payload);

        vm.collect_garbage();

        assert!(vm.heap.contains(buf), "the array itself is rooted");
        assert!(
            !vm.heap.contains(victim),
            "data bytes must not keep an object alive"
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
