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

pub(crate) fn op_cvtwc(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // word to string: create a 1-character string from a rune value
    let rune = vm.src_word()? as u32;
    let ch = char::from_u32(rune).unwrap_or('\u{FFFD}');
    let s = ch.to_string();
    let id = vm
        .heap
        .alloc(0, crate::heap::HeapData::Str(s));
    vm.move_ptr_to_dst(id)
}

pub(crate) fn op_cvtcw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // string to word: get the first character as a rune value
    let str_id = vm.src_ptr()?;
    let val = match vm.heap.get_string(str_id) {
        Some(s) => s.chars().next().map(|c| c as i32).unwrap_or(0),
        None => 0,
    };
    vm.set_dst_word(val)
}

pub(crate) fn op_cvtfc(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // float to string
    let val = vm.src_real()?;
    let s = val.to_string();
    let id = vm
        .heap
        .alloc(0, crate::heap::HeapData::Str(s));
    vm.move_ptr_to_dst(id)
}

pub(crate) fn op_cvtcf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // string to float
    let str_id = vm.src_ptr()?;
    let val = match vm.heap.get_string(str_id) {
        Some(s) => s.parse::<f64>().unwrap_or(0.0),
        None => 0.0,
    };
    vm.set_dst_real(val)
}

pub(crate) fn op_cvtlc(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // big to string
    let val = vm.src_big()?;
    let s = val.to_string();
    let id = vm
        .heap
        .alloc(0, crate::heap::HeapData::Str(s));
    vm.move_ptr_to_dst(id)
}

pub(crate) fn op_cvtcl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // string to big
    let str_id = vm.src_ptr()?;
    let val = match vm.heap.get_string(str_id) {
        Some(s) => s.parse::<i64>().unwrap_or(0),
        None => 0,
    };
    vm.set_dst_big(val)
}

pub(crate) fn op_cvtws(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // word to short (truncate to 16-bit, sign-extend back)
    let val = vm.src_word()? as i16 as i32;
    vm.set_dst_word(val)
}

pub(crate) fn op_cvtsw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // short to word (sign-extend)
    let val = vm.src_word()? as i16 as i32;
    vm.set_dst_word(val)
}

pub(crate) fn op_cvtrf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // fixed-point real to float (stub: treat as identity since we don't have fixed-point)
    let val = vm.src_real()?;
    vm.set_dst_real(val)
}

pub(crate) fn op_cvtfr(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // float to fixed-point real (stub: treat as identity)
    let val = vm.src_real()?;
    vm.set_dst_real(val)
}
