//! Extended fixed-point arithmetic opcodes.
//!
//! These operate on Word (i32) values using Big (i64) intermediate precision,
//! with power-of-2 scaling factors stored in fixed-point registers (frame slots).

use ricevm_core::{ExecError, Word};

use crate::memory;
use crate::vm::VmState;

/// Read the fixed-point register 1 value (stored at a known frame offset).
/// In the C++ VM, this is at `fixed_point_register_1_offset()`.
/// We use a fixed offset in the frame: typically 16 bytes into the frame header area.
fn read_fpr1(vm: &VmState<'_>) -> Word {
    let base = vm.frames.current_data_offset();
    // fpr1 is at a fixed offset in the frame. Using offset that mirrors C++ layout.
    if base + 8 <= vm.frames.data.len() {
        memory::read_word(&vm.frames.data, base)
    } else {
        0
    }
}

fn read_fpr2(vm: &VmState<'_>) -> Word {
    let base = vm.frames.current_data_offset();
    if base + 12 <= vm.frames.data.len() {
        memory::read_word(&vm.frames.data, base + 4)
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

/// mulx src, mid, dst — fixed-point multiply: dst = (src * mid) scaled by fpr2
pub(crate) fn op_mulx(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let y = vm.src_word()? as i64;
    let x = vm.mid_word()? as i64;
    let scale = read_fpr2(vm);
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
    let scale = read_fpr2(vm);
    let residual = read_fpr1(vm) as i64;
    let mut z = apply_scale(x.wrapping_mul(y), scale);
    if residual != 0 {
        z /= residual;
    }
    vm.set_dst_word(z as Word)
}

/// mulx1 — not implemented in reference, stub
pub(crate) fn op_mulx1(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let _ = vm.src_word()?;
    vm.set_dst_word(0)
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
    let scale = read_fpr2(vm);
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
    let residual = read_fpr1(vm) as i64;
    let scale = read_fpr2(vm);
    let tmp = if residual != 0 {
        x.wrapping_mul(residual)
    } else {
        x
    };
    let scaled = apply_scale(tmp, scale);
    vm.set_dst_word((scaled / y) as Word)
}

/// divx1 — not implemented in reference, stub
pub(crate) fn op_divx1(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let y = vm.src_word()?;
    if y == 0 {
        return Err(ExecError::ThreadFault(
            "fixed-point division by zero".to_string(),
        ));
    }
    vm.set_dst_word(0)
}

/// cvtxx src, dst — fixed-point scaling: dst = src scaled by fpr2
pub(crate) fn op_cvtxx(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let x = vm.src_word()? as i64;
    let scale = read_fpr2(vm);
    vm.set_dst_word(apply_scale(x, scale) as Word)
}

/// cvtxx0 src, dst — fixed-point scaling with residual
pub(crate) fn op_cvtxx0(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let x = vm.src_word()? as i64;
    if x == 0 {
        return vm.set_dst_word(0);
    }
    let residual = read_fpr1(vm) as i64;
    let scale = read_fpr2(vm);
    let scaled = apply_scale(x, scale);
    let z = if residual != 0 {
        scaled / residual
    } else {
        scaled
    };
    vm.set_dst_word(z as Word)
}

/// cvtxx1 — not implemented in reference, stub
pub(crate) fn op_cvtxx1(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let x = vm.src_word()?;
    vm.set_dst_word(x)
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
