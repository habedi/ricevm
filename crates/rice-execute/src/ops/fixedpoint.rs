//! Extended fixed-point arithmetic opcodes.
//!
//! These follow Inferno's `xec.c` fixed-point helpers.
//! The compiler stores `STemp` and `DTemp` in reserved frame slots.

use ricevm_core::{ExecError, Word};

use crate::memory;
use crate::vm::VmState;

const STEMP_OFFSET: usize = 4;
const DTEMP_OFFSET: usize = 12;

fn read_stmp(vm: &VmState<'_>) -> Word {
    let base = vm.frames.current_data_offset();
    if base + STEMP_OFFSET + 4 <= vm.frames.data.len() {
        memory::read_word(&vm.frames.data, base + STEMP_OFFSET)
    } else {
        0
    }
}

fn read_dtmp(vm: &VmState<'_>) -> Word {
    let base = vm.frames.current_data_offset();
    if base + DTEMP_OFFSET + 4 <= vm.frames.data.len() {
        memory::read_word(&vm.frames.data, base + DTEMP_OFFSET)
    } else {
        0
    }
}

fn apply_scale(val: i64, scale: i32) -> i64 {
    if scale >= 0 {
        val.wrapping_shl(scale as u32)
    } else {
        val.wrapping_shr((-scale) as u32)
    }
}

fn rounding_mask(scale: i32) -> i64 {
    if scale >= 0 || (-scale as u32) >= 63 {
        0
    } else {
        (1_i64 << (-scale as u32)) - 1
    }
}

/// mulx src, mid, dst — fixed-point multiply: dst = (src * mid) scaled by fpr2
pub(crate) fn op_mulx(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let y = vm.src_word()? as i64;
    let x = vm.mid_word()? as i64;
    let scale = read_dtmp(vm);
    let z = apply_scale(x.wrapping_mul(y), scale);
    vm.set_dst_word(z as Word)
}

/// mulx0 src, mid, dst — fixed-point multiply with residual: dst = ((src * mid) scaled) / fpr1
pub(crate) fn op_mulx0(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let y = vm.src_word()? as i64;
    let x = vm.mid_word()? as i64;
    if x == 0 || y == 0 {
        return vm.set_dst_word(0);
    }
    let scale = read_dtmp(vm);
    let residual = read_stmp(vm) as i64;
    let mut z = apply_scale(x.wrapping_mul(y), scale);
    if residual != 0 {
        z /= residual;
    }
    vm.set_dst_word(z as Word)
}

/// mulx1 — fixed-point multiply with rounding flags encoded in DTemp.
pub(crate) fn op_mulx1(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let y = vm.src_word()? as i64;
    let x = vm.mid_word()? as i64;
    let p = read_dtmp(vm);
    let a = read_stmp(vm) as i64;

    if x == 0 || y == 0 {
        return vm.set_dst_word(0);
    }

    let vnz = (p & 2) != 0;
    let wnz = (p & 1) != 0;
    let scale = p >> 2;

    let mut v = 0_i64;
    if vnz {
        v = a - 1;
        if (x >= 0 && y < 0) || (x < 0 && y >= 0) {
            v = -v;
        }
    }

    let mut w = 0_i64;
    if wnz
        && ((!vnz && ((x > 0 && y < 0) || (x < 0 && y > 0)))
            || (vnz && ((x > 0 && y > 0) || (x < 0 && y < 0))))
    {
        w = rounding_mask(scale);
    }

    let mut r = x.wrapping_mul(y).wrapping_add(w);
    r = apply_scale(r, scale);
    r = r.wrapping_add(v);
    if a != 0 {
        r /= a;
    }
    vm.set_dst_word(r as Word)
}

/// divx src, mid, dst — fixed-point divide: dst = (mid scaled by fpr2) / src
pub(crate) fn op_divx(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let y = vm.src_word()? as i64;
    if y == 0 {
        return Err(ExecError::ThreadFault(
            "fixed-point division by zero".to_string(),
        ));
    }
    let x = vm.mid_word()? as i64;
    let scale = read_dtmp(vm);
    let scaled_x = apply_scale(x, scale);
    vm.set_dst_word((scaled_x / y) as Word)
}

/// divx0 src, mid, dst — fixed-point divide with residual
pub(crate) fn op_divx0(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let y = vm.src_word()? as i64;
    if y == 0 {
        return Err(ExecError::ThreadFault(
            "fixed-point division by zero".to_string(),
        ));
    }
    let x = vm.mid_word()? as i64;
    if x == 0 {
        return vm.set_dst_word(0);
    }
    let residual = read_stmp(vm) as i64;
    let scale = read_dtmp(vm);
    let tmp = if residual != 0 {
        x.wrapping_mul(residual)
    } else {
        x
    };
    let scaled = apply_scale(tmp, scale);
    vm.set_dst_word((scaled / y) as Word)
}

/// divx1 — fixed-point divide with rounding flags encoded in DTemp.
pub(crate) fn op_divx1(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let y = vm.src_word()? as i64;
    if y == 0 {
        return Err(ExecError::ThreadFault(
            "fixed-point division by zero".to_string(),
        ));
    }
    let x = vm.mid_word()? as i64;
    let p = read_dtmp(vm);
    let b = read_stmp(vm) as i64;

    if x == 0 {
        return vm.set_dst_word(0);
    }

    let vnz = (p & 2) != 0;
    let wnz = (p & 1) != 0;
    let scale = p >> 2;

    let mut v = 0_i64;
    if vnz {
        v = 1;
        if (x >= 0 && y < 0) || (x < 0 && y >= 0) {
            v = -v;
        }
    }

    let mut w = 0_i64;
    if wnz && x <= 0 {
        w = rounding_mask(scale);
    }

    let mut s = b.wrapping_mul(x).wrapping_add(w);
    s = apply_scale(s, scale);
    s /= y;
    vm.set_dst_word((s + v) as Word)
}

/// cvtxx src, dst — fixed-point scaling: dst = src scaled by the middle operand.
pub(crate) fn op_cvtxx(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let x = vm.src_word()? as i64;
    let scale = vm.mid_word()?;
    vm.set_dst_word(apply_scale(x, scale) as Word)
}

/// cvtxx0 src, dst — fixed-point scaling with residual from STemp.
pub(crate) fn op_cvtxx0(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let x = vm.src_word()? as i64;
    if x == 0 {
        return vm.set_dst_word(0);
    }
    let residual = read_stmp(vm) as i64;
    let scale = vm.mid_word()?;
    let scaled = apply_scale(x, scale);
    let z = if residual != 0 {
        scaled / residual
    } else {
        scaled
    };
    vm.set_dst_word(z as Word)
}

/// cvtxx1 — fixed-point scaling with rounding flags in the middle operand.
pub(crate) fn op_cvtxx1(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let x = vm.src_word()? as i64;
    let p = vm.mid_word()?;
    let a = read_stmp(vm) as i64;

    if x == 0 {
        return vm.set_dst_word(0);
    }

    let vnz = (p & 2) != 0;
    let wnz = (p & 1) != 0;
    let scale = p >> 2;

    let mut v = 0_i64;
    if vnz {
        v = a - 1;
        if x < 0 {
            v = -v;
        }
    }

    let mut w = 0_i64;
    if wnz && ((!vnz && x < 0) || (vnz && x > 0)) {
        w = rounding_mask(scale);
    }

    let mut r = x.wrapping_add(w);
    r = apply_scale(r, scale);
    r = r.wrapping_add(v);
    if a != 0 {
        r /= a;
    }
    vm.set_dst_word(r as Word)
}

/// cvtfx src, mid, dst — float to fixed-point: dst = round(src * mid)
pub(crate) fn op_cvtfx(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let f = vm.src_real()?;
    let scale = vm.mid_real()?;
    let result = f * scale;
    let rounded = if result < 0.0 {
        (result - 0.5) as Word
    } else {
        (result + 0.5) as Word
    };
    vm.set_dst_word(rounded)
}

/// cvtxf src, mid, dst — fixed-point to float: dst = src * mid
pub(crate) fn op_cvtxf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let x = vm.src_word()? as f64;
    let scale = vm.mid_real()?;
    vm.set_dst_real(x * scale)
}

#[cfg(test)]
mod tests {
    use ricevm_core::{
        Header, Instruction, MiddleOperand, Module, Opcode, Operand, PointerMap, RuntimeFlags,
        TypeDescriptor, XMAGIC,
    };

    use crate::address::AddrTarget;

    use super::*;

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
            name: "fixedpoint_test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        }
    }

    fn write_stmp(vm: &mut VmState<'_>, value: i32) {
        let base = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, base + STEMP_OFFSET, value);
    }

    fn write_dtmp(vm: &mut VmState<'_>, value: i32) {
        let base = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, base + DTEMP_OFFSET, value);
    }

    #[test]
    fn mulx_uses_dtemp_scale() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        write_dtmp(&mut vm, -1);
        vm.src = AddrTarget::Immediate;
        vm.imm_src = 3;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 4;
        vm.dst = AddrTarget::Frame(vm.frames.current_data_offset());

        op_mulx(&mut vm).expect("mulx should succeed");

        assert_eq!(
            memory::read_word(&vm.frames.data, vm.frames.current_data_offset()),
            6
        );
    }

    #[test]
    fn cvtxx_uses_middle_operand_scale() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        write_dtmp(&mut vm, 99);
        vm.src = AddrTarget::Immediate;
        vm.imm_src = 5;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 1;
        vm.dst = AddrTarget::Frame(vm.frames.current_data_offset());

        op_cvtxx(&mut vm).expect("cvtxx should succeed");

        assert_eq!(
            memory::read_word(&vm.frames.data, vm.frames.current_data_offset()),
            10
        );
    }

    #[test]
    fn mulx1_matches_reference_rounding() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        write_stmp(&mut vm, 3);
        write_dtmp(&mut vm, -1);
        vm.src = AddrTarget::Immediate;
        vm.imm_src = -5;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 7;
        vm.dst = AddrTarget::Frame(vm.frames.current_data_offset());

        op_mulx1(&mut vm).expect("mulx1 should succeed");

        assert_eq!(
            memory::read_word(&vm.frames.data, vm.frames.current_data_offset()),
            -6
        );
    }

    #[test]
    fn divx1_matches_reference_rounding() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        write_stmp(&mut vm, 10);
        write_dtmp(&mut vm, -1);
        vm.src = AddrTarget::Immediate;
        vm.imm_src = 3;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = -2;
        vm.dst = AddrTarget::Frame(vm.frames.current_data_offset());

        op_divx1(&mut vm).expect("divx1 should succeed");

        assert_eq!(
            memory::read_word(&vm.frames.data, vm.frames.current_data_offset()),
            -4
        );
    }

    #[test]
    fn cvtxx1_matches_reference_rounding() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        write_stmp(&mut vm, 3);
        vm.src = AddrTarget::Immediate;
        vm.imm_src = -5;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = -1;
        vm.dst = AddrTarget::Frame(vm.frames.current_data_offset());

        op_cvtxx1(&mut vm).expect("cvtxx1 should succeed");

        assert_eq!(
            memory::read_word(&vm.frames.data, vm.frames.current_data_offset()),
            -1
        );
    }
}
