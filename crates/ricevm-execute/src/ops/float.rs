use ricevm_core::ExecError;

use crate::vm::VmState;

pub(crate) fn op_addf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_real()?;
    let m = vm.mid_or_dst_real()?;
    vm.set_dst_real(s + m)
}

pub(crate) fn op_subf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_real()?;
    let m = vm.mid_or_dst_real()?;
    vm.set_dst_real(m - s)
}

pub(crate) fn op_mulf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_real()?;
    let m = vm.mid_or_dst_real()?;
    vm.set_dst_real(s * m)
}

pub(crate) fn op_divf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_real()?;
    let m = vm.mid_or_dst_real()?;
    if s == 0.0 {
        return vm.set_dst_real(f64::INFINITY.copysign(m));
    }
    vm.set_dst_real(m / s)
}

pub(crate) fn op_negf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_real()?;
    vm.set_dst_real(-s)
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
                size: 64,
                pointer_map: PointerMap { bytes: vec![] },
                pointer_count: 0,
            }],
            data: vec![],
            name: "float_test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        }
    }

    /// Set up src and mid as frame-based reals, dst as a separate frame slot.
    /// Layout: fp+0 = src (8 bytes), fp+8 = mid (8 bytes), fp+16 = dst (8 bytes)
    fn setup_real_binop(vm: &mut VmState<'_>, src: f64, mid: f64) -> usize {
        let fp = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, fp, src);
        memory::write_real(&mut vm.frames.data, fp + 8, mid);
        vm.src = AddrTarget::Frame(fp);
        vm.mid = AddrTarget::Frame(fp + 8);
        vm.dst = AddrTarget::Frame(fp + 16);
        fp
    }

    fn read_dst_real(vm: &VmState<'_>, fp: usize) -> f64 {
        memory::read_real(&vm.frames.data, fp + 16)
    }

    #[test]
    fn addf_adds_two_reals() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = setup_real_binop(&mut vm, 1.5, 2.5);
        op_addf(&mut vm).expect("addf should succeed");
        assert!((read_dst_real(&vm, fp) - 4.0).abs() < f64::EPSILON);
    }

    #[test]
    fn subf_subtracts_src_from_mid() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = setup_real_binop(&mut vm, 1.5, 4.0);
        op_subf(&mut vm).expect("subf should succeed");
        assert!((read_dst_real(&vm, fp) - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn mulf_multiplies_two_reals() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = setup_real_binop(&mut vm, 3.0, 4.0);
        op_mulf(&mut vm).expect("mulf should succeed");
        assert!((read_dst_real(&vm, fp) - 12.0).abs() < f64::EPSILON);
    }

    #[test]
    fn divf_divides_mid_by_src() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = setup_real_binop(&mut vm, 2.0, 7.0);
        op_divf(&mut vm).expect("divf should succeed");
        assert!((read_dst_real(&vm, fp) - 3.5).abs() < f64::EPSILON);
    }

    #[test]
    fn divf_by_zero_returns_infinity() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = setup_real_binop(&mut vm, 0.0, 5.0);
        op_divf(&mut vm).expect("divf should succeed");
        assert!(read_dst_real(&vm, fp).is_infinite());
        assert!(read_dst_real(&vm, fp).is_sign_positive());
    }

    #[test]
    fn divf_negative_by_zero_returns_neg_infinity() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = setup_real_binop(&mut vm, 0.0, -5.0);
        op_divf(&mut vm).expect("divf should succeed");
        assert!(read_dst_real(&vm, fp).is_infinite());
        assert!(read_dst_real(&vm, fp).is_sign_negative());
    }

    #[test]
    fn negf_negates_positive() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, fp, 3.14);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 8);
        op_negf(&mut vm).expect("negf should succeed");
        assert!((memory::read_real(&vm.frames.data, fp + 8) + 3.14).abs() < f64::EPSILON);
    }

    #[test]
    fn negf_negates_negative() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, fp, -2.0);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 8);
        op_negf(&mut vm).expect("negf should succeed");
        assert!((memory::read_real(&vm.frames.data, fp + 8) - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn addf_two_operand_form() {
        // When mid = None, dst = dst + src
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, fp, 1.5); // src
        memory::write_real(&mut vm.frames.data, fp + 8, 3.5); // dst (also mid)
        vm.src = AddrTarget::Frame(fp);
        vm.mid = AddrTarget::None;
        vm.dst = AddrTarget::Frame(fp + 8);
        op_addf(&mut vm).expect("addf two-operand should succeed");
        assert!((memory::read_real(&vm.frames.data, fp + 8) - 5.0).abs() < f64::EPSILON);
    }
}
