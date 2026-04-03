use ricevm_core::ExecError;

use crate::vm::VmState;

pub(crate) fn op_addl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_or_dst_big()?;
    vm.set_dst_big(s.wrapping_add(m))
}

pub(crate) fn op_subl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_or_dst_big()?;
    vm.set_dst_big(m.wrapping_sub(s))
}

pub(crate) fn op_mull(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_or_dst_big()?;
    vm.set_dst_big(s.wrapping_mul(m))
}

pub(crate) fn op_divl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_or_dst_big()?;
    if s == 0 {
        return vm.set_dst_big(0);
    }
    vm.set_dst_big(m.wrapping_div(s))
}

pub(crate) fn op_modl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_or_dst_big()?;
    if s == 0 {
        return vm.set_dst_big(0);
    }
    vm.set_dst_big(m.wrapping_rem(s))
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
            name: "big_test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        }
    }

    /// Layout: fp+0 = src (8 bytes), fp+8 = mid (8 bytes), fp+16 = dst (8 bytes)
    fn setup_big_binop(vm: &mut VmState<'_>, src: i64, mid: i64) -> usize {
        let fp = vm.frames.current_data_offset();
        memory::write_big(&mut vm.frames.data, fp, src);
        memory::write_big(&mut vm.frames.data, fp + 8, mid);
        vm.src = AddrTarget::Frame(fp);
        vm.mid = AddrTarget::Frame(fp + 8);
        vm.dst = AddrTarget::Frame(fp + 16);
        fp
    }

    fn read_dst_big(vm: &VmState<'_>, fp: usize) -> i64 {
        memory::read_big(&vm.frames.data, fp + 16)
    }

    #[test]
    fn addl_adds_two_bigs() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = setup_big_binop(&mut vm, 100, 200);
        op_addl(&mut vm).expect("addl should succeed");
        assert_eq!(read_dst_big(&vm, fp), 300);
    }

    #[test]
    fn addl_wraps_on_overflow() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = setup_big_binop(&mut vm, 1, i64::MAX);
        op_addl(&mut vm).expect("addl should succeed");
        assert_eq!(read_dst_big(&vm, fp), i64::MIN);
    }

    #[test]
    fn subl_subtracts_src_from_mid() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = setup_big_binop(&mut vm, 30, 100);
        op_subl(&mut vm).expect("subl should succeed");
        assert_eq!(read_dst_big(&vm, fp), 70);
    }

    #[test]
    fn mull_multiplies_two_bigs() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = setup_big_binop(&mut vm, 7, 6);
        op_mull(&mut vm).expect("mull should succeed");
        assert_eq!(read_dst_big(&vm, fp), 42);
    }

    #[test]
    fn divl_divides_mid_by_src() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = setup_big_binop(&mut vm, 3, 15);
        op_divl(&mut vm).expect("divl should succeed");
        assert_eq!(read_dst_big(&vm, fp), 5);
    }

    #[test]
    fn divl_by_zero_returns_zero() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = setup_big_binop(&mut vm, 0, 42);
        op_divl(&mut vm).expect("divl by zero should succeed");
        assert_eq!(read_dst_big(&vm, fp), 0);
    }

    #[test]
    fn modl_computes_remainder() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = setup_big_binop(&mut vm, 3, 10);
        op_modl(&mut vm).expect("modl should succeed");
        assert_eq!(read_dst_big(&vm, fp), 1);
    }

    #[test]
    fn modl_by_zero_returns_zero() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = setup_big_binop(&mut vm, 0, 42);
        op_modl(&mut vm).expect("modl by zero should succeed");
        assert_eq!(read_dst_big(&vm, fp), 0);
    }

    #[test]
    fn addl_two_operand_form() {
        // When mid = None, dst = dst + src
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        memory::write_big(&mut vm.frames.data, fp, 10); // src
        memory::write_big(&mut vm.frames.data, fp + 8, 25); // dst
        vm.src = AddrTarget::Frame(fp);
        vm.mid = AddrTarget::None;
        vm.dst = AddrTarget::Frame(fp + 8);
        op_addl(&mut vm).expect("addl two-operand should succeed");
        assert_eq!(memory::read_big(&vm.frames.data, fp + 8), 35);
    }

    #[test]
    fn subl_negative_values() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = setup_big_binop(&mut vm, -5, -10);
        op_subl(&mut vm).expect("subl should succeed");
        // mid - src = -10 - (-5) = -5
        assert_eq!(read_dst_big(&vm, fp), -5);
    }
}
