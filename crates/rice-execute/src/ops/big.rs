use ricevm_core::ExecError;

use crate::vm::VmState;

pub(crate) fn op_addl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_big()?;
    vm.set_dst_big(s.wrapping_add(m))
}

pub(crate) fn op_subl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_big()?;
    vm.set_dst_big(s.wrapping_sub(m))
}

pub(crate) fn op_mull(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_big()?;
    vm.set_dst_big(s.wrapping_mul(m))
}

pub(crate) fn op_divl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_big()?;
    if m == 0 {
        return vm.set_dst_big(0);
    }
    vm.set_dst_big(s.wrapping_div(m))
}

pub(crate) fn op_modl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_big()?;
    if m == 0 {
        return vm.set_dst_big(0);
    }
    vm.set_dst_big(s.wrapping_rem(m))
}
