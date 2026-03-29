use ricevm_core::ExecError;

use crate::vm::VmState;

// Word arithmetic: dst = src OP mid

pub(crate) fn op_addw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_word()?;
    vm.set_dst_word(s.wrapping_add(m))
}

pub(crate) fn op_subw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_word()?;
    vm.set_dst_word(s.wrapping_sub(m))
}

pub(crate) fn op_mulw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_word()?;
    vm.set_dst_word(s.wrapping_mul(m))
}

pub(crate) fn op_divw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_word()?;
    if m == 0 {
        return Err(ExecError::ThreadFault("division by zero".to_string()));
    }
    vm.set_dst_word(s.wrapping_div(m))
}

pub(crate) fn op_modw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_word()?;
    if m == 0 {
        return Err(ExecError::ThreadFault("modulo by zero".to_string()));
    }
    vm.set_dst_word(s.wrapping_rem(m))
}

// Byte arithmetic: dst = src OP mid

pub(crate) fn op_addb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_byte()?;
    vm.set_dst_byte(s.wrapping_add(m))
}

pub(crate) fn op_subb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_byte()?;
    vm.set_dst_byte(s.wrapping_sub(m))
}

pub(crate) fn op_mulb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_byte()?;
    vm.set_dst_byte(s.wrapping_mul(m))
}

pub(crate) fn op_divb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_byte()?;
    if m == 0 {
        return Err(ExecError::ThreadFault("division by zero".to_string()));
    }
    vm.set_dst_byte(s / m)
}

pub(crate) fn op_modb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_byte()?;
    if m == 0 {
        return Err(ExecError::ThreadFault("modulo by zero".to_string()));
    }
    vm.set_dst_byte(s % m)
}

// Word bitwise

pub(crate) fn op_andw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_word()?;
    vm.set_dst_word(s & m)
}

pub(crate) fn op_orw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_word()?;
    vm.set_dst_word(s | m)
}

pub(crate) fn op_xorw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_word()?;
    vm.set_dst_word(s ^ m)
}

pub(crate) fn op_shlw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_word()?;
    vm.set_dst_word(s.wrapping_shl(m as u32))
}

pub(crate) fn op_shrw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_word()?;
    vm.set_dst_word(s.wrapping_shr(m as u32))
}

pub(crate) fn op_lsrw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()? as u32;
    let m = vm.mid_word()?;
    vm.set_dst_word(s.wrapping_shr(m as u32) as i32)
}

// Byte bitwise

pub(crate) fn op_andb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_byte()?;
    vm.set_dst_byte(s & m)
}

pub(crate) fn op_orb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_byte()?;
    vm.set_dst_byte(s | m)
}

pub(crate) fn op_xorb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_byte()?;
    vm.set_dst_byte(s ^ m)
}

pub(crate) fn op_shlb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_byte()?;
    vm.set_dst_byte(s.wrapping_shl(m as u32))
}

pub(crate) fn op_shrb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_byte()?;
    vm.set_dst_byte(s.wrapping_shr(m as u32))
}

// Big bitwise and shift

pub(crate) fn op_andl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_big()?;
    vm.set_dst_big(s & m)
}

pub(crate) fn op_orl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_big()?;
    vm.set_dst_big(s | m)
}

pub(crate) fn op_xorl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_big()?;
    vm.set_dst_big(s ^ m)
}

pub(crate) fn op_shll(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_word()?;
    vm.set_dst_big(s.wrapping_shl(m as u32))
}

pub(crate) fn op_shrl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_word()?;
    vm.set_dst_big(s.wrapping_shr(m as u32))
}

pub(crate) fn op_lsrl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()? as u64;
    let m = vm.mid_word()?;
    vm.set_dst_big(s.wrapping_shr(m as u32) as i64)
}

// Exponentiation

pub(crate) fn op_expw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.src_word()?;
    let exp = vm.mid_word()?;
    if exp < 0 {
        vm.set_dst_word(0)
    } else {
        vm.set_dst_word(base.wrapping_pow(exp as u32))
    }
}

pub(crate) fn op_expl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.src_big()?;
    let exp = vm.mid_word()?;
    if exp < 0 {
        vm.set_dst_big(0)
    } else {
        vm.set_dst_big(base.wrapping_pow(exp as u32))
    }
}

pub(crate) fn op_expf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.src_real()?;
    let exp = vm.mid_real()?;
    vm.set_dst_real(base.powf(exp))
}
