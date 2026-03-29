use ricevm_core::ExecError;

use crate::vm::VmState;

pub(crate) fn op_cvtbw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_byte()? as i32;
    vm.set_dst_word(val)
}

pub(crate) fn op_cvtwb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_word()? as u8;
    vm.set_dst_byte(val)
}

pub(crate) fn op_cvtfw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_real()? as i32;
    vm.set_dst_word(val)
}

pub(crate) fn op_cvtwf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_word()? as f64;
    vm.set_dst_real(val)
}

pub(crate) fn op_cvtwl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_word()? as i64;
    vm.set_dst_big(val)
}

pub(crate) fn op_cvtlw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_big()? as i32;
    vm.set_dst_word(val)
}

pub(crate) fn op_cvtlf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_big()? as f64;
    vm.set_dst_real(val)
}

pub(crate) fn op_cvtfl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_real()? as i64;
    vm.set_dst_big(val)
}
