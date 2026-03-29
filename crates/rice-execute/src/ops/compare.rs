use ricevm_core::ExecError;

use crate::vm::VmState;

// Word comparisons: if src OP dst, branch to mid

pub(crate) fn op_beqw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let d = vm.dst_word()?;
    if s == d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}

pub(crate) fn op_bnew(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let d = vm.dst_word()?;
    if s != d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}

pub(crate) fn op_bltw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let d = vm.dst_word()?;
    if s < d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}

pub(crate) fn op_blew(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let d = vm.dst_word()?;
    if s <= d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}

pub(crate) fn op_bgtw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let d = vm.dst_word()?;
    if s > d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}

pub(crate) fn op_bgew(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let d = vm.dst_word()?;
    if s >= d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}

// Float comparisons

pub(crate) fn op_beqf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_real()?;
    let d = vm.dst_real()?;
    if s == d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}

pub(crate) fn op_bnef(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_real()?;
    let d = vm.dst_real()?;
    if s != d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}

pub(crate) fn op_bltf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_real()?;
    let d = vm.dst_real()?;
    if s < d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}

pub(crate) fn op_blef(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_real()?;
    let d = vm.dst_real()?;
    if s <= d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}

pub(crate) fn op_bgtf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_real()?;
    let d = vm.dst_real()?;
    if s > d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}

pub(crate) fn op_bgef(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_real()?;
    let d = vm.dst_real()?;
    if s >= d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}

// Big comparisons

pub(crate) fn op_beql(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let d = vm.read_big_at(vm.dst, vm.imm_dst)?;
    if s == d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}

pub(crate) fn op_bnel(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let d = vm.read_big_at(vm.dst, vm.imm_dst)?;
    if s != d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}

pub(crate) fn op_bltl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let d = vm.read_big_at(vm.dst, vm.imm_dst)?;
    if s < d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}

pub(crate) fn op_blel(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let d = vm.read_big_at(vm.dst, vm.imm_dst)?;
    if s <= d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}

pub(crate) fn op_bgtl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let d = vm.read_big_at(vm.dst, vm.imm_dst)?;
    if s > d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}

pub(crate) fn op_bgel(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let d = vm.read_big_at(vm.dst, vm.imm_dst)?;
    if s >= d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}

// Byte comparisons

pub(crate) fn op_beqb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let d = vm.read_byte_at(vm.dst, vm.imm_dst)?;
    if s == d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}

pub(crate) fn op_bneb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let d = vm.read_byte_at(vm.dst, vm.imm_dst)?;
    if s != d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}

pub(crate) fn op_bltb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let d = vm.read_byte_at(vm.dst, vm.imm_dst)?;
    if s < d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}

pub(crate) fn op_bleb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let d = vm.read_byte_at(vm.dst, vm.imm_dst)?;
    if s <= d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}

pub(crate) fn op_bgtb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let d = vm.read_byte_at(vm.dst, vm.imm_dst)?;
    if s > d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}

pub(crate) fn op_bgeb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let d = vm.read_byte_at(vm.dst, vm.imm_dst)?;
    if s >= d { vm.next_pc = vm.mid_word()? as usize; }
    Ok(())
}
