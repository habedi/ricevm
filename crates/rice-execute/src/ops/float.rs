use ricevm_core::ExecError;

use crate::vm::VmState;

pub(crate) fn op_addf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_real()?;
    let m = vm.mid_real()?;
    vm.set_dst_real(s + m)
}

pub(crate) fn op_subf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_real()?;
    let m = vm.mid_real()?;
    vm.set_dst_real(s - m)
}

pub(crate) fn op_mulf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_real()?;
    let m = vm.mid_real()?;
    vm.set_dst_real(s * m)
}

pub(crate) fn op_divf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_real()?;
    let m = vm.mid_real()?;
    if m == 0.0 {
        return Err(ExecError::ThreadFault("float division by zero".to_string()));
    }
    vm.set_dst_real(s / m)
}

pub(crate) fn op_negf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_real()?;
    vm.set_dst_real(-s)
}
