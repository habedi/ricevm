//! Built-in Math module implementation.
//!
//! Functions are registered in alphabetical order matching the C++ Mathmodtab.
//! Most are unary or binary real→real operations backed by Rust's f64 methods.

use ricevm_core::ExecError;

use crate::builtin::{BuiltinFunc, BuiltinModule};
use crate::memory;
use crate::vm::VmState;

// Frame layout for most Math functions:
//   Offset 0..8:  return value (real)
//   Offset 8..16: temp registers
//   Offset 16+:   arguments
//
// Unary: arg at offset 16 (real, 8 bytes). Return at offset 0.
// Binary: arg1 at offset 16, arg2 at offset 24. Return at offset 0.

const RET_OFF: usize = 0;
const ARG1_OFF: usize = 16;
const ARG2_OFF: usize = 24;

pub(crate) fn create_math_module() -> BuiltinModule {
    BuiltinModule {
        name: "$Math",
        funcs: vec![
            mf("acos", 40, math_acos),
            mf("acosh", 40, math_acosh),
            mf("asin", 40, math_asin),
            mf("asinh", 40, math_asinh),
            mf("atan", 40, math_atan),
            mf("atan2", 48, math_atan2),
            mf("atanh", 40, math_atanh),
            mf("bits32real", 40, math_bits32real),
            mf("bits64real", 40, math_bits64real),
            mf("cbrt", 40, math_cbrt),
            mf("ceil", 40, math_ceil),
            mf("copysign", 48, math_copysign),
            mf("cos", 40, math_cos),
            mf("cosh", 40, math_cosh),
            mf("dot", 40, math_stub),
            mf("erf", 40, math_stub),
            mf("erfc", 40, math_stub),
            mf("exp", 40, math_exp),
            mf("expm1", 40, math_expm1),
            mf("export_int", 40, math_stub),
            mf("export_real", 40, math_stub),
            mf("export_real32", 40, math_stub),
            mf("fabs", 40, math_fabs),
            mf("fdim", 48, math_fdim),
            mf("finite", 40, math_finite),
            mf("floor", 40, math_floor),
            mf("fmax", 48, math_fmax),
            mf("fmin", 48, math_fmin),
            mf("fmod", 48, math_fmod),
            mf("gemm", 96, math_stub),
            mf("getFPcontrol", 32, math_stub_int),
            mf("getFPstatus", 32, math_stub_int),
            mf("hypot", 48, math_hypot),
            mf("iamax", 40, math_stub_int),
            mf("ilogb", 40, math_ilogb),
            mf("import_int", 40, math_stub),
            mf("import_real", 40, math_stub),
            mf("import_real32", 40, math_stub),
            mf("isnan", 40, math_isnan),
            mf("j0", 40, math_stub),
            mf("j1", 40, math_stub),
            mf("jn", 48, math_stub),
            mf("lgamma", 40, math_stub),
            mf("log", 40, math_log),
            mf("log10", 40, math_log10),
            mf("log1p", 40, math_log1p),
            mf("modf", 40, math_stub),
            mf("nextafter", 48, math_stub),
            mf("norm1", 40, math_stub),
            mf("norm2", 40, math_stub),
            mf("pow", 48, math_pow),
            mf("pow10", 40, math_pow10),
            mf("realbits32", 40, math_realbits32),
            mf("realbits64", 40, math_realbits64),
            mf("remainder", 48, math_remainder),
            mf("rint", 40, math_rint),
            mf("scalbn", 48, math_scalbn),
            mf("sin", 40, math_sin),
            mf("sinh", 40, math_sinh),
            mf("sort", 40, math_stub),
            mf("sqrt", 40, math_sqrt),
            mf("tan", 40, math_tan),
            mf("tanh", 40, math_tanh),
            mf("y0", 40, math_stub),
            mf("y1", 40, math_stub),
            mf("yn", 48, math_stub),
        ],
    }
}

fn mf(
    name: &'static str,
    frame_size: usize,
    handler: fn(&mut VmState<'_>) -> Result<(), ExecError>,
) -> BuiltinFunc {
    BuiltinFunc {
        name,
        sig: 0, // Math functions use a simpler matching; sigs can be added later
        frame_size,
        handler,
    }
}

fn math_stub(_vm: &mut VmState<'_>) -> Result<(), ExecError> {
    Ok(())
}

fn math_stub_int(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    memory::write_word(&mut vm.frames.data, base, 0);
    Ok(())
}

fn math_acos(vm: &mut VmState<'_>) -> Result<(), ExecError> { unary_real(vm, f64::acos) }
fn math_acosh(vm: &mut VmState<'_>) -> Result<(), ExecError> { unary_real(vm, f64::acosh) }
fn math_asin(vm: &mut VmState<'_>) -> Result<(), ExecError> { unary_real(vm, f64::asin) }
fn math_asinh(vm: &mut VmState<'_>) -> Result<(), ExecError> { unary_real(vm, f64::asinh) }
fn math_atan(vm: &mut VmState<'_>) -> Result<(), ExecError> { unary_real(vm, f64::atan) }
fn math_atanh(vm: &mut VmState<'_>) -> Result<(), ExecError> { unary_real(vm, f64::atanh) }
fn math_cbrt(vm: &mut VmState<'_>) -> Result<(), ExecError> { unary_real(vm, f64::cbrt) }
fn math_ceil(vm: &mut VmState<'_>) -> Result<(), ExecError> { unary_real(vm, f64::ceil) }
fn math_cos(vm: &mut VmState<'_>) -> Result<(), ExecError> { unary_real(vm, f64::cos) }
fn math_cosh(vm: &mut VmState<'_>) -> Result<(), ExecError> { unary_real(vm, f64::cosh) }
fn math_exp(vm: &mut VmState<'_>) -> Result<(), ExecError> { unary_real(vm, f64::exp) }
fn math_fabs(vm: &mut VmState<'_>) -> Result<(), ExecError> { unary_real(vm, f64::abs) }
fn math_floor(vm: &mut VmState<'_>) -> Result<(), ExecError> { unary_real(vm, f64::floor) }
fn math_log(vm: &mut VmState<'_>) -> Result<(), ExecError> { unary_real(vm, f64::ln) }
fn math_log10(vm: &mut VmState<'_>) -> Result<(), ExecError> { unary_real(vm, f64::log10) }
fn math_log1p(vm: &mut VmState<'_>) -> Result<(), ExecError> { unary_real(vm, f64::ln_1p) }
fn math_rint(vm: &mut VmState<'_>) -> Result<(), ExecError> { unary_real(vm, f64::round) }
fn math_sin(vm: &mut VmState<'_>) -> Result<(), ExecError> { unary_real(vm, f64::sin) }
fn math_sinh(vm: &mut VmState<'_>) -> Result<(), ExecError> { unary_real(vm, f64::sinh) }
fn math_sqrt(vm: &mut VmState<'_>) -> Result<(), ExecError> { unary_real(vm, f64::sqrt) }
fn math_tan(vm: &mut VmState<'_>) -> Result<(), ExecError> { unary_real(vm, f64::tan) }
fn math_tanh(vm: &mut VmState<'_>) -> Result<(), ExecError> { unary_real(vm, f64::tanh) }

fn unary_real(vm: &mut VmState<'_>, f: fn(f64) -> f64) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    memory::write_real(&mut vm.frames.data, base + RET_OFF, f(x));
    Ok(())
}

// Binary real functions

fn math_atan2(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let y = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let x = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    memory::write_real(&mut vm.frames.data, base + RET_OFF, y.atan2(x));
    Ok(())
}

fn math_copysign(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let s = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    memory::write_real(&mut vm.frames.data, base + RET_OFF, x.copysign(s));
    Ok(())
}

fn math_fmax(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let y = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    memory::write_real(&mut vm.frames.data, base + RET_OFF, x.max(y));
    Ok(())
}

fn math_fmin(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let y = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    memory::write_real(&mut vm.frames.data, base + RET_OFF, x.min(y));
    Ok(())
}

fn math_fmod(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let y = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    memory::write_real(&mut vm.frames.data, base + RET_OFF, x % y);
    Ok(())
}

fn math_hypot(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let y = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    memory::write_real(&mut vm.frames.data, base + RET_OFF, x.hypot(y));
    Ok(())
}

fn math_pow(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let y = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    memory::write_real(&mut vm.frames.data, base + RET_OFF, x.powf(y));
    Ok(())
}

fn math_remainder(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let y = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    memory::write_real(&mut vm.frames.data, base + RET_OFF, x % y);
    Ok(())
}

fn math_scalbn(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let n = memory::read_word(&vm.frames.data, base + ARG2_OFF);
    memory::write_real(&mut vm.frames.data, base + RET_OFF, x * (2.0_f64).powi(n));
    Ok(())
}

// Functions returning int

fn math_finite(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    memory::write_word(&mut vm.frames.data, base, if x.is_finite() { 1 } else { 0 });
    Ok(())
}

fn math_ilogb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let result = if x == 0.0 { i32::MIN } else { x.abs().log2().floor() as i32 };
    memory::write_word(&mut vm.frames.data, base, result);
    Ok(())
}

fn math_isnan(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    memory::write_word(&mut vm.frames.data, base, if x.is_nan() { 1 } else { 0 });
    Ok(())
}

fn math_pow10(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let p = memory::read_word(&vm.frames.data, base + ARG1_OFF);
    memory::write_real(&mut vm.frames.data, base + RET_OFF, 10.0_f64.powi(p));
    Ok(())
}

// Bit conversion functions

fn math_bits32real(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let bits = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let val = f32::from_bits(bits) as f64;
    memory::write_real(&mut vm.frames.data, base + RET_OFF, val);
    Ok(())
}

fn math_bits64real(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let bits = memory::read_big(&vm.frames.data, base + ARG1_OFF) as u64;
    let val = f64::from_bits(bits);
    memory::write_real(&mut vm.frames.data, base + RET_OFF, val);
    Ok(())
}

fn math_realbits32(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let bits = (x as f32).to_bits() as i32;
    memory::write_word(&mut vm.frames.data, base, bits);
    Ok(())
}

fn math_realbits64(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let bits = x.to_bits() as i64;
    memory::write_big(&mut vm.frames.data, base, bits);
    Ok(())
}

fn math_expm1(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::exp_m1)
}

fn math_fdim(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let y = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    memory::write_real(&mut vm.frames.data, base + RET_OFF, if x > y { x - y } else { 0.0 });
    Ok(())
}
