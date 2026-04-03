//! Built-in Math module implementation.
//!
//! Functions are registered in alphabetical order matching the C++ Mathmodtab.
//! Most are unary or binary real→real operations backed by Rust's f64 methods.

// Bessel function approximations use numerical coefficients that happen to be
// close to standard constants but are not meant to be replaced.
#![allow(clippy::approx_constant)]

use ricevm_core::ExecError;

use crate::builtin::{BuiltinFunc, BuiltinModule};
use crate::memory;
use crate::vm::VmState;

// Frame layout for Math built-in function calls:
//   Offset 0..8:   return value (real)
//   Offset 8..16:  additional return values or padding
//   Offset 16..20: return address pointer (written by Lea)
//   Offset 20..32: reserved/padding
//   Offset 32+:    arguments
//
// Unary: arg at offset 32 (real, 8 bytes). Return at offset 0.
// Binary: arg1 at offset 32, arg2 at offset 40. Return at offset 0.

const RET_OFF: usize = 0;
const ARG1_OFF: usize = 32;
const ARG2_OFF: usize = 40;

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
            mf("dot", 40, math_dot),
            mf("erf", 40, math_erf),
            mf("erfc", 40, math_erfc),
            mf("exp", 40, math_exp),
            mf("expm1", 40, math_expm1),
            mf("export_int", 40, math_export_int),
            mf("export_real", 40, math_export_real),
            mf("export_real32", 40, math_export_real32),
            mf("fabs", 40, math_fabs),
            mf("fdim", 48, math_fdim),
            mf("finite", 40, math_finite),
            mf("floor", 40, math_floor),
            mf("fmax", 48, math_fmax),
            mf("fmin", 48, math_fmin),
            mf("fmod", 48, math_fmod),
            mf("gemm", 96, math_gemm),
            mf("getFPcontrol", 32, math_get_fp_control),
            mf("getFPstatus", 32, math_get_fp_status),
            mf("hypot", 48, math_hypot),
            mf("iamax", 40, math_iamax),
            mf("ilogb", 40, math_ilogb),
            mf("import_int", 40, math_import_int),
            mf("import_real", 40, math_import_real),
            mf("import_real32", 40, math_import_real32),
            mf("isnan", 40, math_isnan),
            mf("j0", 40, math_j0),
            mf("j1", 40, math_j1),
            mf("jn", 48, math_jn),
            mf("lgamma", 40, math_lgamma),
            mf("log", 40, math_log),
            mf("log10", 40, math_log10),
            mf("log1p", 40, math_log1p),
            mf("modf", 40, math_modf),
            mf("nextafter", 48, math_nextafter),
            mf("norm1", 40, math_norm1),
            mf("norm2", 40, math_norm2),
            mf("pow", 48, math_pow),
            mf("pow10", 40, math_pow10),
            mf("realbits32", 40, math_realbits32),
            mf("realbits64", 40, math_realbits64),
            mf("remainder", 48, math_remainder),
            mf("rint", 40, math_rint),
            mf("scalbn", 48, math_scalbn),
            mf("sin", 40, math_sin),
            mf("sinh", 40, math_sinh),
            mf("sort", 40, math_sort),
            mf("sqrt", 40, math_sqrt),
            mf("tan", 40, math_tan),
            mf("tanh", 40, math_tanh),
            mf("y0", 40, math_y0),
            mf("y1", 40, math_y1),
            mf("yn", 48, math_yn),
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

/// Write a real (8-byte) return value to both frame offset 0 and the return
/// pointer at offset 16. The 4-byte mcall return copy cannot handle 8-byte values.
fn write_real_return(vm: &mut VmState<'_>, base: usize, val: f64) {
    memory::write_real(&mut vm.frames.data, base + RET_OFF, val);
    let ret_ptr = memory::read_word(&vm.frames.data, base + 16);
    if ret_ptr != 0 {
        let target = crate::address::decode_virtual_addr(ret_ptr, 0);
        let mut buf = [0u8; 8];
        memory::write_real(&mut buf, 0, val);
        match target {
            crate::address::AddrTarget::Frame(off) if off + 8 <= vm.frames.data.len() => {
                vm.frames.data[off..off + 8].copy_from_slice(&buf);
            }
            crate::address::AddrTarget::ModuleMp { module_idx, offset } => {
                if let Some(mp) = vm
                    .module_mp_mut(module_idx)
                    .filter(|mp| offset + 8 <= mp.len())
                {
                    mp[offset..offset + 8].copy_from_slice(&buf);
                }
            }
            _ => {}
        }
    }
}

/// Write a big (8-byte) return value to both frame offset 0 and the return pointer.
fn write_big_return(vm: &mut VmState<'_>, base: usize, val: i64) {
    memory::write_big(&mut vm.frames.data, base, val);
    let ret_ptr = memory::read_word(&vm.frames.data, base + 16);
    if ret_ptr != 0 {
        let target = crate::address::decode_virtual_addr(ret_ptr, 0);
        let mut buf = [0u8; 8];
        memory::write_big(&mut buf, 0, val);
        match target {
            crate::address::AddrTarget::Frame(off) if off + 8 <= vm.frames.data.len() => {
                vm.frames.data[off..off + 8].copy_from_slice(&buf);
            }
            crate::address::AddrTarget::ModuleMp { module_idx, offset } => {
                if let Some(mp) = vm
                    .module_mp_mut(module_idx)
                    .filter(|mp| offset + 8 <= mp.len())
                {
                    mp[offset..offset + 8].copy_from_slice(&buf);
                }
            }
            _ => {}
        }
    }
}

/// Return the FP control word. Since we don't have hardware FP control
/// registers, return 0 which means all exceptions masked, round-to-nearest.
fn math_get_fp_control(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    memory::write_word(&mut vm.frames.data, base, 0);
    Ok(())
}

/// Return the FP status word. Since we don't have hardware FP status
/// registers, return 0 which means no exceptions raised.
fn math_get_fp_status(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    memory::write_word(&mut vm.frames.data, base, 0);
    Ok(())
}

fn math_acos(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::acos)
}
fn math_acosh(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::acosh)
}
fn math_asin(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::asin)
}
fn math_asinh(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::asinh)
}
fn math_atan(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::atan)
}
fn math_atanh(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::atanh)
}
fn math_cbrt(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::cbrt)
}
fn math_ceil(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::ceil)
}
fn math_cos(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::cos)
}
fn math_cosh(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::cosh)
}
fn math_exp(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::exp)
}
fn math_fabs(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::abs)
}
fn math_floor(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::floor)
}
fn math_log(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::ln)
}
fn math_log10(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::log10)
}
fn math_log1p(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::ln_1p)
}
fn math_rint(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::round)
}
fn math_sin(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::sin)
}
fn math_sinh(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::sinh)
}
fn math_sqrt(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::sqrt)
}
fn math_tan(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::tan)
}
fn math_tanh(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::tanh)
}

fn unary_real(vm: &mut VmState<'_>, f: fn(f64) -> f64) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    write_real_return(vm, base, f(x));
    Ok(())
}

// Binary real functions

fn math_atan2(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let y = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let x = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    write_real_return(vm, base, y.atan2(x));
    Ok(())
}

fn math_copysign(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let s = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    write_real_return(vm, base, x.copysign(s));
    Ok(())
}

fn math_fmax(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let y = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    write_real_return(vm, base, x.max(y));
    Ok(())
}

fn math_fmin(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let y = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    write_real_return(vm, base, x.min(y));
    Ok(())
}

fn math_fmod(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let y = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    write_real_return(vm, base, x % y);
    Ok(())
}

fn math_hypot(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let y = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    write_real_return(vm, base, x.hypot(y));
    Ok(())
}

fn math_pow(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let y = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    write_real_return(vm, base, x.powf(y));
    Ok(())
}

fn math_remainder(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let y = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    write_real_return(vm, base, x % y);
    Ok(())
}

fn math_scalbn(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let n = memory::read_word(&vm.frames.data, base + ARG2_OFF);
    write_real_return(vm, base, x * (2.0_f64).powi(n));
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
    let result = if x == 0.0 {
        i32::MIN
    } else {
        x.abs().log2().floor() as i32
    };
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
    write_real_return(vm, base, 10.0_f64.powi(p));
    Ok(())
}

// Bit conversion functions

fn math_bits32real(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let bits = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let val = f32::from_bits(bits) as f64;
    write_real_return(vm, base, val);
    Ok(())
}

fn math_bits64real(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let bits = memory::read_big(&vm.frames.data, base + ARG1_OFF) as u64;
    let val = f64::from_bits(bits);
    write_real_return(vm, base, val);
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
    write_big_return(vm, base, x.to_bits() as i64);
    Ok(())
}

fn math_expm1(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, f64::exp_m1)
}

fn math_fdim(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let y = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    memory::write_real(
        &mut vm.frames.data,
        base + RET_OFF,
        if x > y { x - y } else { 0.0 },
    );
    Ok(())
}

// --- Newly implemented functions ---

/// Error function using Abramowitz & Stegun approximation.
fn erf_approx(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.47047 * x);
    let poly = t * (0.3480242 + t * (-0.0958798 + t * 0.7478556));
    sign * (1.0 - poly * (-x * x).exp())
}

fn math_erf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, erf_approx)
}

fn math_erfc(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, |x| 1.0 - erf_approx(x))
}

/// Bessel J0 approximation.
fn bessel_j0(x: f64) -> f64 {
    let ax = x.abs();
    if ax < 8.0 {
        let y = x * x;
        let n = 57568490574.0
            + y * (-13362590354.0
                + y * (651619640.7 + y * (-11214424.18 + y * (77392.33017 + y * (-184.9052456)))));
        let d = 57568490411.0
            + y * (1029532985.0
                + y * (9494680.718 + y * (59272.64853 + y * (267.8532712 + y * 1.0))));
        n / d
    } else {
        let z = 8.0 / ax;
        let y = z * z;
        let xx = ax - 0.785398164;
        let p = 1.0
            + y * (-0.1098628627e-2
                + y * (0.2734510407e-4 + y * (-0.2073370639e-5 + y * 0.2093887211e-6)));
        let q = -0.1562499995e-1
            + y * (0.1430488765e-3
                + y * (-0.6911147651e-5 + y * (0.7621095161e-6 - y * 0.934935152e-7)));
        (0.636619772 / ax).sqrt() * (p * xx.cos() - z * q * xx.sin())
    }
}

fn math_j0(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, bessel_j0)
}

/// Bessel J1 approximation.
fn bessel_j1(x: f64) -> f64 {
    let ax = x.abs();
    if ax < 8.0 {
        let y = x * x;
        let n = x
            * (72362614232.0
                + y * (-7895059235.0
                    + y * (242396853.1
                        + y * (-2972611.439 + y * (15704.48260 + y * (-30.16036606))))));
        let d = 144725228442.0
            + y * (2300535178.0
                + y * (18583304.74 + y * (99447.43394 + y * (376.9991397 + y * 1.0))));
        n / d
    } else {
        let z = 8.0 / ax;
        let y = z * z;
        let xx = ax - 2.356194491;
        let p = 1.0
            + y * (0.183105e-2
                + y * (-0.3516396496e-4 + y * (0.2457520174e-5 - y * 0.240337019e-6)));
        let q = 0.04687499995
            + y * (-0.2002690873e-3
                + y * (0.8449199096e-5 + y * (-0.88228987e-6 + y * 0.105787412e-6)));
        let ans = (0.636619772 / ax).sqrt() * (p * xx.cos() - z * q * xx.sin());
        if x < 0.0 { -ans } else { ans }
    }
}

fn math_j1(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, bessel_j1)
}

fn math_jn(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let n = memory::read_word(&vm.frames.data, base + ARG1_OFF);
    let x = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    let result = match n {
        0 => bessel_j0(x),
        1 => bessel_j1(x),
        _ => {
            // Forward recurrence
            let mut jnm1 = bessel_j0(x);
            let mut jn = bessel_j1(x);
            for k in 1..n.unsigned_abs() {
                let jnp1 = (2.0 * k as f64 / x) * jn - jnm1;
                jnm1 = jn;
                jn = jnp1;
            }
            if n < 0 && n % 2 != 0 { -jn } else { jn }
        }
    };
    write_real_return(vm, base, result);
    Ok(())
}

/// Bessel Y0 approximation.
fn bessel_y0(x: f64) -> f64 {
    if x < 8.0 {
        let y = x * x;
        let n = -2957821389.0
            + y * (7062834065.0
                + y * (-512359803.6 + y * (10879881.29 + y * (-86327.92757 + y * 228.4622733))));
        let d = 40076544269.0
            + y * (745249964.8
                + y * (7189466.438 + y * (47447.26470 + y * (226.1030244 + y * 1.0))));
        (n / d) + 0.636619772 * bessel_j0(x) * x.ln()
    } else {
        let z = 8.0 / x;
        let y = z * z;
        let xx = x - 0.785398164;
        let p = 1.0
            + y * (-0.1098628627e-2
                + y * (0.2734510407e-4 + y * (-0.2073370639e-5 + y * 0.2093887211e-6)));
        let q = -0.1562499995e-1
            + y * (0.1430488765e-3
                + y * (-0.6911147651e-5 + y * (0.7621095161e-6 - y * 0.934935152e-7)));
        (0.636619772 / x).sqrt() * (p * xx.sin() + z * q * xx.cos())
    }
}

fn math_y0(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, bessel_y0)
}

/// Bessel Y1 approximation.
fn bessel_y1(x: f64) -> f64 {
    if x < 8.0 {
        let y = x * x;
        let n = x
            * (-4900604943000.0
                + y * (1275274390000.0
                    + y * (-51534866838.0
                        + y * (622785432.7 + y * (-3130827.838 + y * (7.374510624e3))))));
        let d = 24995805700000.0
            + y * (424441966400.0
                + y * (3733650367.0
                    + y * (22459040.02 + y * (103680.2068 + y * (365.9584658 + y * 1.0)))));
        (n / d) + 0.636619772 * (bessel_j1(x) * x.ln() - 1.0 / x)
    } else {
        let z = 8.0 / x;
        let y = z * z;
        let xx = x - 2.356194491;
        let p = 1.0
            + y * (0.183105e-2
                + y * (-0.3516396496e-4 + y * (0.2457520174e-5 - y * 0.240337019e-6)));
        let q = 0.04687499995
            + y * (-0.2002690873e-3
                + y * (0.8449199096e-5 + y * (-0.88228987e-6 + y * 0.105787412e-6)));
        (0.636619772 / x).sqrt() * (p * xx.sin() + z * q * xx.cos())
    }
}

fn math_y1(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, bessel_y1)
}

fn math_yn(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let n = memory::read_word(&vm.frames.data, base + ARG1_OFF);
    let x = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    let result = match n {
        0 => bessel_y0(x),
        1 => bessel_y1(x),
        _ => {
            let mut ynm1 = bessel_y0(x);
            let mut yn = bessel_y1(x);
            for k in 1..n.unsigned_abs() {
                let ynp1 = (2.0 * k as f64 / x) * yn - ynm1;
                ynm1 = yn;
                yn = ynp1;
            }
            yn
        }
    };
    write_real_return(vm, base, result);
    Ok(())
}

/// Log-gamma using Lanczos approximation.
fn lgamma_approx(x: f64) -> f64 {
    let g = 7.0;
    let c = [
        0.999_999_999_999_809_9,
        676.5203681218851,
        -1259.1392167224028,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507343278686905,
        -0.13857109526572012,
        9.984_369_578_019_572e-6,
        1.5056327351493116e-7,
    ];
    if x < 0.5 {
        let pi = std::f64::consts::PI;
        (pi / (pi * x).sin()).ln() - lgamma_approx(1.0 - x)
    } else {
        let x = x - 1.0;
        let mut sum = c[0];
        for (i, &ci) in c.iter().enumerate().skip(1) {
            sum += ci / (x + i as f64);
        }
        let t = x + g + 0.5;
        0.5 * (2.0 * std::f64::consts::PI).ln() + (t.ln()) * (x + 0.5) - t + sum.ln()
    }
}

fn math_lgamma(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    unary_real(vm, lgamma_approx)
}

fn math_modf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let int_part = x.trunc();
    let frac_part = x.fract();
    write_real_return(vm, base, int_part);
    memory::write_real(&mut vm.frames.data, base + RET_OFF + 8, frac_part);
    Ok(())
}

fn math_nextafter(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let y = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    let result = if x == y {
        y
    } else if x.is_nan() || y.is_nan() {
        f64::NAN
    } else {
        let bits = x.to_bits();
        let next = if (y > x) == (x >= 0.0) {
            bits + 1
        } else {
            bits - 1
        };
        f64::from_bits(next)
    };
    write_real_return(vm, base, result);
    Ok(())
}

fn read_real_array(vm: &VmState<'_>, arr_id: u32, n: usize) -> Vec<f64> {
    let data = vm.heap.array_read(arr_id, 0, n * 8).unwrap_or_default();
    (0..n.min(data.len() / 8))
        .map(|i| memory::read_real(&data, i * 8))
        .collect()
}

fn math_dot(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let n = memory::read_word(&vm.frames.data, base + ARG1_OFF) as usize;
    let x_id = memory::read_word(&vm.frames.data, base + ARG1_OFF + 4) as u32;
    let y_id = memory::read_word(&vm.frames.data, base + ARG1_OFF + 8) as u32;
    let x = read_real_array(vm, x_id, n);
    let y = read_real_array(vm, y_id, n);
    let result: f64 = x.iter().zip(y.iter()).map(|(a, b)| a * b).sum();
    write_real_return(vm, base, result);
    Ok(())
}

fn math_norm1(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x_id = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let n = memory::read_word(&vm.frames.data, base + ARG1_OFF + 4) as usize;
    let x = read_real_array(vm, x_id, n);
    let result: f64 = x.iter().map(|v| v.abs()).sum();
    write_real_return(vm, base, result);
    Ok(())
}

fn math_norm2(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x_id = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let n = memory::read_word(&vm.frames.data, base + ARG1_OFF + 4) as usize;
    let x = read_real_array(vm, x_id, n);
    let result: f64 = x.iter().map(|v| v * v).sum::<f64>().sqrt();
    write_real_return(vm, base, result);
    Ok(())
}

fn math_sort(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let arr_id = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let count = memory::read_word(&vm.frames.data, base + ARG1_OFF + 4) as usize;
    if let Some(obj) = vm.heap.get_mut(arr_id)
        && let crate::heap::HeapData::Array {
            data, elem_size, ..
        } = &mut obj.data
        && *elem_size == 8
    {
        let n = count.min(data.len() / 8);
        let mut vals: Vec<f64> = (0..n).map(|i| memory::read_real(data, i * 8)).collect();
        vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        for (i, &v) in vals.iter().enumerate() {
            memory::write_real(data, i * 8, v);
        }
    }
    Ok(())
}

fn math_iamax(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x_id = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let n = memory::read_word(&vm.frames.data, base + ARG1_OFF + 4) as usize;
    let x = read_real_array(vm, x_id, n);
    let result = x
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| {
            a.abs()
                .partial_cmp(&b.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i as i32)
        .unwrap_or(0);
    memory::write_word(&mut vm.frames.data, base, result);
    Ok(())
}

/// General matrix multiply: C = alpha * op(A) * op(B) + beta * C
///
/// Frame layout (96 bytes total):
///   32: transa (int)   36: transb (int)
///   40: m (int)        44: n (int)        48: k (int)
///   52: pad            56: alpha (real)
///   64: a (ptr)        68: lda (int)
///   72: b (ptr)        76: ldb (int)
///   80: beta (real)
///   88: c (ptr)        92: ldc (int)
fn math_gemm(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let transa = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u8 as char;
    let transb = memory::read_word(&vm.frames.data, base + ARG1_OFF + 4) as u8 as char;
    let m = memory::read_word(&vm.frames.data, base + ARG1_OFF + 8) as usize;
    let n = memory::read_word(&vm.frames.data, base + ARG1_OFF + 12) as usize;
    let k = memory::read_word(&vm.frames.data, base + ARG1_OFF + 16) as usize;
    // alpha at offset 56 (8-byte aligned after 5 ints + 4 bytes padding)
    let alpha = memory::read_real(&vm.frames.data, base + 56);
    let a_id = memory::read_word(&vm.frames.data, base + 64) as u32;
    let lda = memory::read_word(&vm.frames.data, base + 68) as usize;
    let b_id = memory::read_word(&vm.frames.data, base + 72) as u32;
    let ldb = memory::read_word(&vm.frames.data, base + 76) as usize;
    let beta = memory::read_real(&vm.frames.data, base + 80);
    let c_id = memory::read_word(&vm.frames.data, base + 88) as u32;
    let ldc = memory::read_word(&vm.frames.data, base + 92) as usize;

    let nota = transa == 'N';
    let notb = transb == 'N';

    if m == 0 || n == 0 || ((alpha == 0.0 || k == 0) && beta == 1.0) {
        return Ok(());
    }

    // Helper: read a real from a heap array at the given element index
    let read_arr = |vm: &VmState<'_>, id: u32, idx: usize| -> f64 {
        if let Some(bytes) = vm.heap.array_read(id, idx * 8, 8) {
            memory::read_real(&bytes, 0)
        } else {
            0.0
        }
    };

    // Read C into a working buffer
    let c_size = ldc * (n.max(1) - 1) + m;
    let mut c_buf: Vec<f64> = (0..c_size).map(|i| read_arr(vm, c_id, i)).collect();

    if alpha == 0.0 {
        for j in 0..n {
            let jc = j * ldc;
            for i in 0..m {
                if beta == 0.0 {
                    c_buf[i + jc] = 0.0;
                } else {
                    c_buf[i + jc] *= beta;
                }
            }
        }
    } else if a_id == 0 {
        // A is nil: C := alpha*op(B) + beta*C
        for j in 0..n {
            let jc = j * ldc;
            for i in 0..m {
                let b_val = if notb {
                    read_arr(vm, b_id, i + j * ldb)
                } else {
                    read_arr(vm, b_id, j + i * ldb)
                };
                c_buf[i + jc] = alpha * b_val + beta * c_buf[i + jc];
            }
        }
    } else if notb {
        if nota {
            // C := alpha*A*B + beta*C
            for j in 0..n {
                let jc = j * ldc;
                for i in 0..m {
                    if beta == 0.0 {
                        c_buf[i + jc] = 0.0;
                    } else if beta != 1.0 {
                        c_buf[i + jc] *= beta;
                    }
                }
                for l in 0..k {
                    let b_val = read_arr(vm, b_id, l + j * ldb);
                    if b_val != 0.0 {
                        let temp = alpha * b_val;
                        for i in 0..m {
                            let a_val = read_arr(vm, a_id, i + l * lda);
                            c_buf[i + jc] += temp * a_val;
                        }
                    }
                }
            }
        } else {
            // C := alpha*A'*B + beta*C
            for j in 0..n {
                let jc = j * ldc;
                for i in 0..m {
                    let mut temp = 0.0;
                    for l in 0..k {
                        temp += read_arr(vm, a_id, l + i * lda) * read_arr(vm, b_id, l + j * ldb);
                    }
                    if beta == 0.0 {
                        c_buf[i + jc] = alpha * temp;
                    } else {
                        c_buf[i + jc] = alpha * temp + beta * c_buf[i + jc];
                    }
                }
            }
        }
    } else if nota {
        // C := alpha*A*B' + beta*C
        for j in 0..n {
            let jc = j * ldc;
            for i in 0..m {
                if beta == 0.0 {
                    c_buf[i + jc] = 0.0;
                } else if beta != 1.0 {
                    c_buf[i + jc] *= beta;
                }
            }
            for l in 0..k {
                let b_val = read_arr(vm, b_id, j + l * ldb);
                if b_val != 0.0 {
                    let temp = alpha * b_val;
                    for i in 0..m {
                        let a_val = read_arr(vm, a_id, i + l * lda);
                        c_buf[i + jc] += temp * a_val;
                    }
                }
            }
        }
    } else {
        // C := alpha*A'*B' + beta*C
        for j in 0..n {
            let jc = j * ldc;
            for i in 0..m {
                let mut temp = 0.0;
                for l in 0..k {
                    temp += read_arr(vm, a_id, l + i * lda) * read_arr(vm, b_id, j + l * ldb);
                }
                if beta == 0.0 {
                    c_buf[i + jc] = alpha * temp;
                } else {
                    c_buf[i + jc] = alpha * temp + beta * c_buf[i + jc];
                }
            }
        }
    }

    // Write C buffer back to heap
    for (i, &val) in c_buf.iter().enumerate() {
        let mut buf = [0u8; 8];
        memory::write_real(&mut buf, 0, val);
        vm.heap.array_write(c_id, i * 8, &buf);
    }

    Ok(())
}

// Byte-order conversion functions

fn math_export_int(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let val = memory::read_word(&vm.frames.data, base + ARG1_OFF);
    let buf_id = memory::read_word(&vm.frames.data, base + ARG1_OFF + 4) as u32;
    if let Some(obj) = vm.heap.get_mut(buf_id)
        && let crate::heap::HeapData::Array { data, .. } = &mut obj.data
        && data.len() >= 4
    {
        let bytes = val.to_be_bytes();
        data[..4].copy_from_slice(&bytes);
    }
    Ok(())
}

fn math_export_real(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    // Signature: export_real(buf: array of byte, x: real)
    // The compiler passes x via a 1-element float array at +36.
    let buf_id = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let val_arr_id = memory::read_word(&vm.frames.data, base + ARG1_OFF + 4) as u32;
    // Read the real from the float array
    let val = if let Some(data) = vm.heap.array_read(val_arr_id, 0, 8) {
        memory::read_real(&data, 0)
    } else {
        // Fallback: try reading as a raw real at +40 (older calling convention)
        memory::read_real(&vm.frames.data, base + ARG2_OFF)
    };
    let bytes = val.to_be_bytes();
    vm.heap.array_write(buf_id, 0, &bytes);
    Ok(())
}

fn math_export_real32(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    // Signature: export_real32(buf: array of byte, val: real)
    let buf_id = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let val = memory::read_real(&vm.frames.data, base + ARG2_OFF) as f32;
    if let Some(obj) = vm.heap.get_mut(buf_id)
        && let crate::heap::HeapData::Array { data, .. } = &mut obj.data
        && data.len() >= 4
    {
        let bytes = val.to_be_bytes();
        data[..4].copy_from_slice(&bytes);
    }
    Ok(())
}

fn math_import_int(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let buf_id = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let val = if let Some(obj) = vm.heap.get(buf_id)
        && let crate::heap::HeapData::Array { data, .. } = &obj.data
        && data.len() >= 4
    {
        i32::from_be_bytes([data[0], data[1], data[2], data[3]])
    } else {
        0
    };
    memory::write_word(&mut vm.frames.data, base, val);
    Ok(())
}

fn math_import_real(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    // Frame layout: buf at offset 32, ret destination array at offset 36
    let buf_id = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let ret_ref = memory::read_word(&vm.frames.data, base + ARG1_OFF + 4) as u32;
    let val = if let Some(obj) = vm.heap.get(buf_id)
        && let crate::heap::HeapData::Array { data, .. } = &obj.data
        && data.len() >= 8
    {
        f64::from_be_bytes([
            data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
        ])
    } else {
        0.0
    };
    // Write result to the caller's storage via ret pointer (heap array ref)
    if let Some(obj) = vm.heap.get_mut(ret_ref)
        && let crate::heap::HeapData::Array { data, .. } = &mut obj.data
        && data.len() >= 8
    {
        memory::write_real(data, 0, val);
    }
    // Also write at frame offset 0 for the standard return mechanism
    write_real_return(vm, base, val);
    Ok(())
}

fn math_import_real32(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let buf_id = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let val = if let Some(obj) = vm.heap.get(buf_id)
        && let crate::heap::HeapData::Array { data, .. } = &obj.data
        && data.len() >= 4
    {
        f32::from_be_bytes([data[0], data[1], data[2], data[3]]) as f64
    } else {
        0.0
    };
    write_real_return(vm, base, val);
    Ok(())
}

#[cfg(test)]
mod tests {
    use ricevm_core::{
        Header, Instruction, MiddleOperand, Module, Opcode, Operand, PointerMap, RuntimeFlags,
        TypeDescriptor, XMAGIC,
    };

    use super::*;
    use crate::vm::VmState;

    /// Create a module whose entry frame is 64 bytes -- large enough for
    /// binary math functions (which read up to offset 40+8 = 48).
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
            name: "math_test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        }
    }

    /// Helper: set a unary real argument at ARG1_OFF and call the function,
    /// then read the real return value from RET_OFF.
    fn call_unary_real(f: fn(&mut VmState<'_>) -> Result<(), ExecError>, arg: f64) -> f64 {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, arg);
        f(&mut vm).expect("math function should succeed");
        memory::read_real(&vm.frames.data, base + RET_OFF)
    }

    /// Helper: set two real arguments and call a binary function.
    fn call_binary_real(
        f: fn(&mut VmState<'_>) -> Result<(), ExecError>,
        arg1: f64,
        arg2: f64,
    ) -> f64 {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, arg1);
        memory::write_real(&mut vm.frames.data, base + ARG2_OFF, arg2);
        f(&mut vm).expect("math function should succeed");
        memory::read_real(&vm.frames.data, base + RET_OFF)
    }

    // ---- Trigonometric functions ----

    #[test]
    fn sin_zero() {
        let r = call_unary_real(math_sin, 0.0);
        assert!((r - 0.0).abs() < 1e-15, "sin(0) = 0, got {r}");
    }

    #[test]
    fn sin_pi_half() {
        let r = call_unary_real(math_sin, std::f64::consts::FRAC_PI_2);
        assert!((r - 1.0).abs() < 1e-15, "sin(pi/2) = 1, got {r}");
    }

    #[test]
    fn cos_zero() {
        let r = call_unary_real(math_cos, 0.0);
        assert!((r - 1.0).abs() < 1e-15, "cos(0) = 1, got {r}");
    }

    #[test]
    fn cos_pi() {
        let r = call_unary_real(math_cos, std::f64::consts::PI);
        assert!((r - (-1.0)).abs() < 1e-15, "cos(pi) = -1, got {r}");
    }

    #[test]
    fn tan_zero() {
        let r = call_unary_real(math_tan, 0.0);
        assert!((r - 0.0).abs() < 1e-15, "tan(0) = 0, got {r}");
    }

    #[test]
    fn tan_pi_over_4() {
        let r = call_unary_real(math_tan, std::f64::consts::FRAC_PI_4);
        assert!((r - 1.0).abs() < 1e-12, "tan(pi/4) = 1, got {r}");
    }

    // ---- Inverse trigonometric ----

    #[test]
    fn asin_zero() {
        let r = call_unary_real(math_asin, 0.0);
        assert!((r - 0.0).abs() < 1e-15, "asin(0) = 0, got {r}");
    }

    #[test]
    fn acos_one() {
        let r = call_unary_real(math_acos, 1.0);
        assert!((r - 0.0).abs() < 1e-15, "acos(1) = 0, got {r}");
    }

    #[test]
    fn atan_zero() {
        let r = call_unary_real(math_atan, 0.0);
        assert!((r - 0.0).abs() < 1e-15, "atan(0) = 0, got {r}");
    }

    #[test]
    fn atan2_unit() {
        let r = call_binary_real(math_atan2, 1.0, 1.0);
        assert!(
            (r - std::f64::consts::FRAC_PI_4).abs() < 1e-15,
            "atan2(1,1) = pi/4, got {r}"
        );
    }

    // ---- sqrt, pow, log, exp ----

    #[test]
    fn sqrt_four() {
        let r = call_unary_real(math_sqrt, 4.0);
        assert!((r - 2.0).abs() < 1e-15, "sqrt(4) = 2, got {r}");
    }

    #[test]
    fn sqrt_zero() {
        let r = call_unary_real(math_sqrt, 0.0);
        assert!((r - 0.0).abs() < 1e-15, "sqrt(0) = 0, got {r}");
    }

    #[test]
    fn sqrt_one() {
        let r = call_unary_real(math_sqrt, 1.0);
        assert!((r - 1.0).abs() < 1e-15, "sqrt(1) = 1, got {r}");
    }

    #[test]
    fn pow_two_cubed() {
        let r = call_binary_real(math_pow, 2.0, 3.0);
        assert!((r - 8.0).abs() < 1e-12, "pow(2,3) = 8, got {r}");
    }

    #[test]
    fn pow_anything_zero() {
        let r = call_binary_real(math_pow, 42.0, 0.0);
        assert!((r - 1.0).abs() < 1e-15, "pow(42,0) = 1, got {r}");
    }

    #[test]
    fn log_one() {
        let r = call_unary_real(math_log, 1.0);
        assert!((r - 0.0).abs() < 1e-15, "ln(1) = 0, got {r}");
    }

    #[test]
    fn log_e() {
        let r = call_unary_real(math_log, std::f64::consts::E);
        assert!((r - 1.0).abs() < 1e-15, "ln(e) = 1, got {r}");
    }

    #[test]
    fn log10_hundred() {
        let r = call_unary_real(math_log10, 100.0);
        assert!((r - 2.0).abs() < 1e-12, "log10(100) = 2, got {r}");
    }

    #[test]
    fn exp_zero() {
        let r = call_unary_real(math_exp, 0.0);
        assert!((r - 1.0).abs() < 1e-15, "exp(0) = 1, got {r}");
    }

    #[test]
    fn exp_one() {
        let r = call_unary_real(math_exp, 1.0);
        assert!(
            (r - std::f64::consts::E).abs() < 1e-12,
            "exp(1) = e, got {r}"
        );
    }

    // ---- floor, ceil, rint ----

    #[test]
    fn floor_positive_fraction() {
        let r = call_unary_real(math_floor, 2.7);
        assert!((r - 2.0).abs() < 1e-15, "floor(2.7) = 2, got {r}");
    }

    #[test]
    fn floor_negative_fraction() {
        let r = call_unary_real(math_floor, -2.3);
        assert!((r - (-3.0)).abs() < 1e-15, "floor(-2.3) = -3, got {r}");
    }

    #[test]
    fn ceil_positive_fraction() {
        let r = call_unary_real(math_ceil, 2.3);
        assert!((r - 3.0).abs() < 1e-15, "ceil(2.3) = 3, got {r}");
    }

    #[test]
    fn ceil_negative_fraction() {
        let r = call_unary_real(math_ceil, -2.7);
        assert!((r - (-2.0)).abs() < 1e-15, "ceil(-2.7) = -2, got {r}");
    }

    #[test]
    fn ceil_integer() {
        let r = call_unary_real(math_ceil, 5.0);
        assert!((r - 5.0).abs() < 1e-15, "ceil(5.0) = 5, got {r}");
    }

    #[test]
    fn rint_rounds_half() {
        let r = call_unary_real(math_rint, 2.5);
        assert!((r - 3.0).abs() < 1e-15, "rint(2.5) = 3, got {r}");
    }

    // ---- fabs, cbrt ----

    #[test]
    fn fabs_negative() {
        let r = call_unary_real(math_fabs, -3.14);
        assert!((r - 3.14).abs() < 1e-15, "fabs(-3.14) = 3.14, got {r}");
    }

    #[test]
    fn cbrt_27() {
        let r = call_unary_real(math_cbrt, 27.0);
        assert!((r - 3.0).abs() < 1e-12, "cbrt(27) = 3, got {r}");
    }

    // ---- Hyperbolic ----

    #[test]
    fn sinh_zero() {
        let r = call_unary_real(math_sinh, 0.0);
        assert!((r - 0.0).abs() < 1e-15, "sinh(0) = 0, got {r}");
    }

    #[test]
    fn cosh_zero() {
        let r = call_unary_real(math_cosh, 0.0);
        assert!((r - 1.0).abs() < 1e-15, "cosh(0) = 1, got {r}");
    }

    #[test]
    fn tanh_zero() {
        let r = call_unary_real(math_tanh, 0.0);
        assert!((r - 0.0).abs() < 1e-15, "tanh(0) = 0, got {r}");
    }

    // ---- Bit conversion functions ----

    #[test]
    fn realbits64_roundtrip() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();

        let original: f64 = 3.14;
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, original);
        math_realbits64(&mut vm).expect("realbits64 should succeed");
        let bits = memory::read_big(&vm.frames.data, base + RET_OFF);

        // Now convert back using bits64real
        let mut vm2 = VmState::new(&module).expect("vm should initialize");
        let base2 = vm2.frames.current_data_offset();
        memory::write_big(&mut vm2.frames.data, base2 + ARG1_OFF, bits);
        math_bits64real(&mut vm2).expect("bits64real should succeed");
        let result = memory::read_real(&vm2.frames.data, base2 + RET_OFF);

        assert!(
            (result - original).abs() < 1e-15,
            "roundtrip: expected {original}, got {result}"
        );
    }

    #[test]
    fn realbits32_roundtrip() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();

        let original: f64 = 2.5; // exactly representable as f32
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, original);
        math_realbits32(&mut vm).expect("realbits32 should succeed");
        let bits = memory::read_word(&vm.frames.data, base + RET_OFF);

        // Now convert back using bits32real
        let mut vm2 = VmState::new(&module).expect("vm should initialize");
        let base2 = vm2.frames.current_data_offset();
        memory::write_word(&mut vm2.frames.data, base2 + ARG1_OFF, bits);
        math_bits32real(&mut vm2).expect("bits32real should succeed");
        let result = memory::read_real(&vm2.frames.data, base2 + RET_OFF);

        assert!(
            (result - original).abs() < 1e-15,
            "roundtrip: expected {original}, got {result}"
        );
    }

    #[test]
    fn bits64real_known_value() {
        // IEEE 754: 1.0 as f64 has bits 0x3FF0000000000000
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        memory::write_big(
            &mut vm.frames.data,
            base + ARG1_OFF,
            0x3FF0000000000000_u64 as i64,
        );
        math_bits64real(&mut vm).expect("bits64real should succeed");
        let result = memory::read_real(&vm.frames.data, base + RET_OFF);
        assert!(
            (result - 1.0).abs() < 1e-15,
            "bits64real(0x3FF0...) = 1.0, got {result}"
        );
    }

    // ---- Functions returning int ----

    #[test]
    fn isnan_detects_nan() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, f64::NAN);
        math_isnan(&mut vm).expect("isnan should succeed");
        let result = memory::read_word(&vm.frames.data, base + RET_OFF);
        assert_eq!(result, 1, "isnan(NaN) should be 1");
    }

    #[test]
    fn isnan_rejects_normal() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, 42.0);
        math_isnan(&mut vm).expect("isnan should succeed");
        let result = memory::read_word(&vm.frames.data, base + RET_OFF);
        assert_eq!(result, 0, "isnan(42.0) should be 0");
    }

    #[test]
    fn finite_normal() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, 1.0);
        math_finite(&mut vm).expect("finite should succeed");
        let result = memory::read_word(&vm.frames.data, base + RET_OFF);
        assert_eq!(result, 1, "finite(1.0) should be 1");
    }

    #[test]
    fn finite_infinity() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, f64::INFINITY);
        math_finite(&mut vm).expect("finite should succeed");
        let result = memory::read_word(&vm.frames.data, base + RET_OFF);
        assert_eq!(result, 0, "finite(inf) should be 0");
    }

    // ---- Binary real ops ----

    #[test]
    fn fmod_basic() {
        let r = call_binary_real(math_fmod, 7.0, 3.0);
        assert!((r - 1.0).abs() < 1e-12, "fmod(7,3) = 1, got {r}");
    }

    #[test]
    fn hypot_3_4() {
        let r = call_binary_real(math_hypot, 3.0, 4.0);
        assert!((r - 5.0).abs() < 1e-12, "hypot(3,4) = 5, got {r}");
    }

    #[test]
    fn fmax_picks_larger() {
        let r = call_binary_real(math_fmax, 2.0, 5.0);
        assert!((r - 5.0).abs() < 1e-15, "fmax(2,5) = 5, got {r}");
    }

    #[test]
    fn fmin_picks_smaller() {
        let r = call_binary_real(math_fmin, 2.0, 5.0);
        assert!((r - 2.0).abs() < 1e-15, "fmin(2,5) = 2, got {r}");
    }

    #[test]
    fn copysign_positive_to_negative() {
        let r = call_binary_real(math_copysign, 3.0, -1.0);
        assert!((r - (-3.0)).abs() < 1e-15, "copysign(3,-1) = -3, got {r}");
    }

    // ---- pow10, scalbn ----

    #[test]
    fn pow10_two() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF, 2);
        math_pow10(&mut vm).expect("pow10 should succeed");
        let result = memory::read_real(&vm.frames.data, base + RET_OFF);
        assert!(
            (result - 100.0).abs() < 1e-12,
            "pow10(2) = 100, got {result}"
        );
    }

    #[test]
    fn scalbn_basic() {
        // scalbn(1.5, 3) = 1.5 * 2^3 = 12.0
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, 1.5);
        memory::write_word(&mut vm.frames.data, base + ARG2_OFF, 3);
        math_scalbn(&mut vm).expect("scalbn should succeed");
        let result = memory::read_real(&vm.frames.data, base + RET_OFF);
        assert!(
            (result - 12.0).abs() < 1e-12,
            "scalbn(1.5,3) = 12.0, got {result}"
        );
    }

    // ---- modf ----

    #[test]
    fn modf_splits_correctly() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, 3.75);
        math_modf(&mut vm).expect("modf should succeed");
        let int_part = memory::read_real(&vm.frames.data, base + RET_OFF);
        let frac_part = memory::read_real(&vm.frames.data, base + RET_OFF + 8);
        assert!(
            (int_part - 3.0).abs() < 1e-15,
            "modf(3.75) int = 3, got {int_part}"
        );
        assert!(
            (frac_part - 0.75).abs() < 1e-15,
            "modf(3.75) frac = 0.75, got {frac_part}"
        );
    }

    // ---- ilogb ----

    #[test]
    fn ilogb_of_8() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, 8.0);
        math_ilogb(&mut vm).expect("ilogb should succeed");
        let result = memory::read_word(&vm.frames.data, base + RET_OFF);
        assert_eq!(result, 3, "ilogb(8) = 3, got {result}");
    }

    #[test]
    fn ilogb_of_zero() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, 0.0);
        math_ilogb(&mut vm).expect("ilogb should succeed");
        let result = memory::read_word(&vm.frames.data, base + RET_OFF);
        assert_eq!(result, i32::MIN, "ilogb(0) = i32::MIN, got {result}");
    }

    // ---- expm1, log1p ----

    #[test]
    fn expm1_zero() {
        let r = call_unary_real(math_expm1, 0.0);
        assert!((r - 0.0).abs() < 1e-15, "expm1(0) = 0, got {r}");
    }

    #[test]
    fn log1p_zero() {
        let r = call_unary_real(math_log1p, 0.0);
        assert!((r - 0.0).abs() < 1e-15, "log1p(0) = 0, got {r}");
    }

    // ---- fdim ----

    #[test]
    fn fdim_positive_diff() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, 5.0);
        memory::write_real(&mut vm.frames.data, base + ARG2_OFF, 3.0);
        math_fdim(&mut vm).expect("fdim should succeed");
        let result = memory::read_real(&vm.frames.data, base + RET_OFF);
        assert!((result - 2.0).abs() < 1e-15, "fdim(5,3) = 2, got {result}");
    }

    #[test]
    fn fdim_negative_diff_returns_zero() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, 3.0);
        memory::write_real(&mut vm.frames.data, base + ARG2_OFF, 5.0);
        math_fdim(&mut vm).expect("fdim should succeed");
        let result = memory::read_real(&vm.frames.data, base + RET_OFF);
        assert!((result - 0.0).abs() < 1e-15, "fdim(3,5) = 0, got {result}");
    }

    // ---- getFPcontrol / getFPstatus ----

    #[test]
    fn get_fp_control_returns_zero() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        math_get_fp_control(&mut vm).expect("getFPcontrol should succeed");
        let base = vm.frames.current_data_offset();
        let result = memory::read_word(&vm.frames.data, base + RET_OFF);
        assert_eq!(result, 0, "getFPcontrol should return 0");
    }

    #[test]
    fn get_fp_status_returns_zero() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        math_get_fp_status(&mut vm).expect("getFPstatus should succeed");
        let base = vm.frames.current_data_offset();
        let result = memory::read_word(&vm.frames.data, base + RET_OFF);
        assert_eq!(result, 0, "getFPstatus should return 0");
    }

    // ---- create_math_module ----

    #[test]
    fn create_math_module_has_expected_functions() {
        let m = create_math_module();
        assert_eq!(m.name, "$Math");
        let names: Vec<&str> = m.funcs.iter().map(|f| f.name).collect();
        assert!(names.contains(&"sin"), "should contain sin");
        assert!(names.contains(&"cos"), "should contain cos");
        assert!(names.contains(&"sqrt"), "should contain sqrt");
        assert!(names.contains(&"pow"), "should contain pow");
        assert!(names.contains(&"log"), "should contain log");
        assert!(names.contains(&"floor"), "should contain floor");
        assert!(names.contains(&"ceil"), "should contain ceil");
    }
}
