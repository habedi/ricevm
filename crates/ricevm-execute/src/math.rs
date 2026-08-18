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
    // rint follows the current rounding mode, i.e. round-half-to-even.
    unary_real(vm, f64::round_ties_even)
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

/// IEEE-754 remainder: `x - n*y` where `n` is `x/y` rounded half-to-even.
/// This is not `fmod` (which truncates the quotient).
fn ieee_remainder(x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() || y == 0.0 || x.is_infinite() {
        return f64::NAN;
    }
    if y.is_infinite() {
        return x;
    }
    let ay = y.abs();
    // fmod gives x - trunc(x/y)*y, so |r| < |y| with the sign of x.
    let mut r = x % ay;
    let half = ay * 0.5;
    if r.abs() > half {
        r -= ay.copysign(r);
    } else if r.abs() == half {
        // Exact tie: keep the remainder that leaves an even quotient.
        let q = (x - r) / ay;
        if q % 2.0 != 0.0 {
            r -= ay.copysign(r);
        }
    }
    if r == 0.0 { 0.0_f64.copysign(x) } else { r }
}

fn math_remainder(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let y = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    write_real_return(vm, base, ieee_remainder(x, y));
    Ok(())
}

/// 2^1023, the largest power of two that is still a normal f64.
const TWO_POW_1023: f64 = f64::from_bits(0x7fe0_0000_0000_0000);
/// 2^-969 (= 2^-1022 * 2^53): scaling down by this keeps the final
/// multiply out of the subnormal range, avoiding double rounding.
const TWO_POW_M969: f64 = f64::from_bits(0x0360_0000_0000_0000);

/// `x * 2^n`, scaled in steps so the power of two never overflows or
/// underflows independently of the product (as `x * 2f64.powi(n)` does).
fn scalbn(x: f64, n: i32) -> f64 {
    let mut y = x;
    let mut n = n;
    if n > 1023 {
        y *= TWO_POW_1023;
        n -= 1023;
        if n > 1023 {
            y *= TWO_POW_1023;
            n -= 1023;
            if n > 1023 {
                n = 1023;
            }
        }
    } else if n < -1022 {
        y *= TWO_POW_M969;
        n += 1022 - 53;
        if n < -1022 {
            y *= TWO_POW_M969;
            n += 1022 - 53;
            if n < -1022 {
                n = -1022;
            }
        }
    }
    y * f64::from_bits(((0x3ff + n) as u64) << 52)
}

fn math_scalbn(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let n = memory::read_word(&vm.frames.data, base + ARG2_OFF);
    write_real_return(vm, base, scalbn(x, n));
    Ok(())
}

// Functions returning int

fn math_finite(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    memory::write_word(&mut vm.frames.data, base, if x.is_finite() { 1 } else { 0 });
    Ok(())
}

/// The binary exponent of x, that is floor(log2(|x|)).
///
/// The exponent field gives the answer directly. Taking the floor of a computed
/// log2 does not: for the largest double below a power of two the logarithm
/// rounds up to the exponent itself, which is one too many.
/// `external/inferno-os/libmath/fdlibm/s_ilogb.c` fixes the special cases as
/// ilogb(0) = 0x80000001 and ilogb(inf) = ilogb(NaN) = 0x7fffffff.
fn ilogb(x: f64) -> i32 {
    if x == 0.0 {
        return 0x8000_0001_u32 as i32;
    }
    if !x.is_finite() {
        return i32::MAX;
    }
    let bits = x.abs().to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32;
    if exp == 0 {
        // A subnormal is mantissa * 2^-1074, so the exponent follows from the
        // position of its highest set bit.
        let mantissa = bits & 0x000f_ffff_ffff_ffff;
        -1074 + 63 - mantissa.leading_zeros() as i32
    } else {
        exp - 1023
    }
}

fn math_ilogb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    memory::write_word(&mut vm.frames.data, base, ilogb(x));
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
    // `ipow10` in `external/inferno-os/libmath/pow10.c` is pow(10., n), which is
    // correctly rounded. Repeated squaring through `powi` is off by one unit in
    // the last place for most exponents.
    write_real_return(vm, base, 10.0_f64.powf(p as f64));
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
    // The result has to travel through the return pointer like every other
    // real result; writing frame offset 0 alone loses it at the call site.
    write_real_return(vm, base, if x > y { x - y } else { 0.0 });
    Ok(())
}

// --- Newly implemented functions ---

/// Error function using Abramowitz & Stegun approximation 7.1.25, whose
/// absolute error bound is 2.5e-5.
fn erf_approx(x: f64) -> f64 {
    // erf is odd, so erf(-0) is -0: the sign bit decides, not the comparison.
    let sign = if x.is_sign_negative() { -1.0 } else { 1.0 };
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
    // J0 of an infinity is zero, as `__ieee754_j0` in
    // `external/inferno-os/libmath/fdlibm/e_j0.c` returns one/(x*x) there.
    if ax.is_infinite() {
        return 0.0;
    }
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
    // J1 of an infinity is zero, like J0.
    if ax.is_infinite() {
        return 0.0;
    }
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

/// Bessel Jn for an order of 2 or more.
///
/// The recurrence J(k+1) = 2k/x*J(k) - J(k-1) is only stable upwards while the
/// order stays below x, which is the split `__ieee754_jn` makes in
/// `external/inferno-os/libmath/fdlibm/e_jn.c`. Above that it loses every
/// significant digit, so the recurrence runs downwards from an order well above
/// the wanted one and the result is normalised by the identity
/// J0(x) + 2*J2(x) + 2*J4(x) + ... = 1.
fn bessel_jn(order: u32, x: f64) -> f64 {
    /// Sets how far above the wanted order the downward recurrence starts.
    const ACC: f64 = 160.0;
    /// Bounds the downward recurrence before it can overflow.
    const BIG: f64 = 1e10;
    const SMALL: f64 = 1e-10;

    let ax = x.abs();
    // J(n, 0) is 0 for every order above 0, and J(n, inf) is 0.
    if ax == 0.0 || ax.is_infinite() {
        return 0.0;
    }
    // |J(n, x)| is at most (x/2)^n/n!, which underflows to zero well before an
    // order large enough to make either recurrence run for a long time.
    if order as f64 * (0.5 * ax).ln() - lgamma_approx(order as f64 + 1.0) < -745.0 {
        return 0.0;
    }
    let two_over_x = 2.0 / ax;
    let magnitude = if ax > order as f64 {
        let mut jkm1 = bessel_j0(ax);
        let mut jk = bessel_j1(ax);
        for k in 1..order {
            let jkp1 = k as f64 * two_over_x * jk - jkm1;
            jkm1 = jk;
            jk = jkp1;
        }
        jk
    } else {
        // The recurrence starts at an even order so that the normalising sum
        // picks up only the even terms.
        let start = 2 * ((order + (ACC * order as f64).sqrt() as u32) / 2);
        let mut wanted = 0.0;
        let mut sum = 0.0;
        let mut jkp1 = 0.0;
        let mut jk = 1.0;
        let mut in_sum = false;
        for k in (1..=start).rev() {
            let jkm1 = k as f64 * two_over_x * jk - jkp1;
            jkp1 = jk;
            jk = jkm1;
            if jk.abs() > BIG {
                jk *= SMALL;
                jkp1 *= SMALL;
                wanted *= SMALL;
                sum *= SMALL;
            }
            if in_sum {
                sum += jk;
            }
            in_sum = !in_sum;
            if k == order {
                wanted = jkp1;
            }
        }
        wanted / (2.0 * sum - jk)
    };
    // J(n, -x) = (-1)^n * J(n, x).
    if x < 0.0 && order % 2 == 1 {
        -magnitude
    } else {
        magnitude
    }
}

fn math_jn(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let n = memory::read_word(&vm.frames.data, base + ARG1_OFF);
    let x = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    let order = n.unsigned_abs();
    let magnitude = match order {
        0 => bessel_j0(x),
        1 => bessel_j1(x),
        _ => bessel_jn(order, x),
    };
    // J(-n, x) = (-1)^n * J(n, x).
    let result = if n < 0 && order % 2 == 1 {
        -magnitude
    } else {
        magnitude
    };
    write_real_return(vm, base, result);
    Ok(())
}

/// Bessel Y0 approximation.
///
/// The edge cases follow `__ieee754_y0` in
/// `external/inferno-os/libmath/fdlibm/e_j0.c`: Y0 is -inf at zero, NaN for a
/// negative argument, and zero at +inf.
fn bessel_y0(x: f64) -> f64 {
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if x < 0.0 {
        return f64::NAN;
    }
    if x.is_infinite() {
        return 0.0;
    }
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

/// Bessel Y1 for 0 < x < 8, from the ascending series of Abramowitz and Stegun
/// 9.1.11 with psi(k+1) = -gamma + H(k).
///
/// The series converges over the whole interval and its accuracy is limited by
/// J1 alone, which is about eight significant digits.
fn bessel_y1_series(x: f64) -> f64 {
    /// Euler's constant.
    const GAMMA: f64 = 0.577_215_664_901_532_9;
    /// Terms past this point are far below the rounding error of the sum.
    const TERMS: u32 = 64;

    let half = 0.5 * x;
    // `term` is (x/2)^(2k+1) / (k!*(k+1)!), `hk` is H(k), and `hk1` is H(k+1).
    let mut term = half;
    let mut hk = 0.0;
    let mut hk1 = 1.0;
    let mut sign = 1.0;
    let mut sum = 0.0;
    for k in 1..=TERMS {
        sum += sign * term * (2.0 * GAMMA - hk - hk1);
        sign = -sign;
        term *= half * half / (k as f64 * (k + 1) as f64);
        hk = hk1;
        hk1 += 1.0 / (k + 1) as f64;
    }
    let pi = std::f64::consts::PI;
    (2.0 / pi) * bessel_j1(x) * half.ln() - 2.0 / (pi * x) + sum / pi
}

/// Bessel Y1 approximation.
///
/// The edge cases follow `__ieee754_y1` in
/// `external/inferno-os/libmath/fdlibm/e_j1.c`: Y1 is -inf at zero, NaN for a
/// negative argument, and zero at +inf.
fn bessel_y1(x: f64) -> f64 {
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if x < 0.0 {
        return f64::NAN;
    }
    if x.is_infinite() {
        return 0.0;
    }
    if x < 8.0 {
        bessel_y1_series(x)
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

/// Bessel Yn.
///
/// `__ieee754_yn` in `external/inferno-os/libmath/fdlibm/e_jn.c` settles the
/// argument before it looks at the order, so Y is -inf at zero, NaN below zero,
/// and zero at +inf whatever the order is. Unlike J, the upward recurrence is
/// stable for Y, so it serves every order.
fn bessel_yn(n: i32, x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if x < 0.0 {
        return f64::NAN;
    }
    if x.is_infinite() {
        return 0.0;
    }
    let order = n.unsigned_abs();
    let magnitude = match order {
        0 => bessel_y0(x),
        1 => bessel_y1(x),
        _ => {
            let mut ykm1 = bessel_y0(x);
            let mut yk = bessel_y1(x);
            for k in 1..order {
                let ykp1 = (2.0 * k as f64 / x) * yk - ykm1;
                ykm1 = yk;
                yk = ykp1;
            }
            yk
        }
    };
    // Y(-n, x) = (-1)^n * Y(n, x).
    if n < 0 && order % 2 == 1 {
        -magnitude
    } else {
        magnitude
    }
}

fn math_yn(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let n = memory::read_word(&vm.frames.data, base + ARG1_OFF);
    let x = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    write_real_return(vm, base, bessel_yn(n, x));
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
    // Gamma has a pole at every non-positive integer, and lgamma of either
    // infinity is +inf.
    if (x <= 0.0 && x == x.trunc()) || x.is_infinite() {
        return f64::INFINITY;
    }
    if x < 0.5 {
        // lgamma is log|Gamma(x)|, which is finite for a negative non-integer,
        // so the reflection formula needs the magnitude of the sine. The sign
        // is what `__ieee754_lgamma_r` reports through its second result.
        let pi = std::f64::consts::PI;
        (pi / (pi * x).sin().abs()).ln() - lgamma_approx(1.0 - x)
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
    // C99 modf gives both parts the sign of x, and the fractional part of an
    // infinity is a zero rather than the NaN that x - x.trunc() produces.
    let frac_part = if x.is_infinite() {
        0.0_f64.copysign(x)
    } else {
        (x - int_part).copysign(x)
    };
    write_real_return(vm, base, int_part);
    memory::write_real(&mut vm.frames.data, base + RET_OFF + 8, frac_part);
    Ok(())
}

fn math_nextafter(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let y = memory::read_real(&vm.frames.data, base + ARG2_OFF);
    let result = if x.is_nan() || y.is_nan() {
        f64::NAN
    } else if x == y {
        y
    } else if x == 0.0 {
        // Both zeros compare equal to 0.0, so the sign of x tells us nothing:
        // step to the smallest subnormal in the direction of y.
        f64::from_bits(1).copysign(y)
    } else {
        // Away from zero the bit patterns are monotonic in magnitude, so a
        // step "towards y" is +1 when y and x point the same way from zero.
        let bits = x.to_bits();
        let next = if (y > x) == (x > 0.0) {
            bits + 1
        } else {
            bits - 1
        };
        f64::from_bits(next)
    };
    write_real_return(vm, base, result);
    Ok(())
}

/// Read every element of a heap array of reals.
///
/// The element count belongs to the array. The BLAS entry points in
/// `external/inferno-os/libinterp/math.c` all pass `f->x->len`, and the Limbo
/// declarations in `external/inferno-os/module/math.m` have no length argument
/// for the frame to carry.
fn read_real_array(vm: &VmState<'_>, arr_id: u32) -> Vec<f64> {
    let n = vm.heap.array_byte_len(arr_id).unwrap_or(0) / 8;
    let data = vm.heap.array_read(arr_id, 0, n * 8).unwrap_or_default();
    (0..n.min(data.len() / 8))
        .map(|i| memory::read_real(&data, i * 8))
        .collect()
}

/// Read every element of a heap array of ints.
fn read_int_array(vm: &VmState<'_>, arr_id: u32) -> Vec<i32> {
    let n = vm.heap.array_byte_len(arr_id).unwrap_or(0) / 4;
    let data = vm.heap.array_read(arr_id, 0, n * 4).unwrap_or_default();
    (0..n.min(data.len() / 4))
        .map(|i| memory::read_word(&data, i * 4))
        .collect()
}

fn math_dot(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x_id = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let y_id = memory::read_word(&vm.frames.data, base + ARG1_OFF + 4) as u32;
    let x = read_real_array(vm, x_id);
    let y = read_real_array(vm, y_id);
    // `Math_dot` raises an error for incompatible lengths rather than reading
    // past the end of the shorter vector.
    if x.len() != y.len() {
        return Err(ExecError::ThreadFault(format!(
            "dot: incompatible lengths ({} and {})",
            x.len(),
            y.len()
        )));
    }
    let result: f64 = x.iter().zip(y.iter()).map(|(a, b)| a * b).sum();
    write_real_return(vm, base, result);
    Ok(())
}

fn math_norm1(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x_id = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let x = read_real_array(vm, x_id);
    let result: f64 = x.iter().map(|v| v.abs()).sum();
    write_real_return(vm, base, result);
    Ok(())
}

fn math_norm2(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x_id = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let x = read_real_array(vm, x_id);
    let result: f64 = x.iter().map(|v| v * v).sum::<f64>().sqrt();
    write_real_return(vm, base, result);
    Ok(())
}

/// `sort(x, pi)` reorders the permutation pi so that x[pi[i]] <= x[pi[i+1]] and
/// leaves x untouched, as described in
/// `external/inferno-os/man/2/math-linalg`. Every entry of pi has to address an
/// element of x, which `Math_sort` checks before it sorts.
fn math_sort(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x_id = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let p_id = memory::read_word(&vm.frames.data, base + ARG1_OFF + 4) as u32;
    let x = read_real_array(vm, x_id);
    let mut p = read_int_array(vm, p_id);
    if let Some(&bad) = p.iter().find(|&&i| i < 0 || i as usize >= x.len()) {
        return Err(ExecError::ThreadFault(format!(
            "sort: permutation entry {bad} is outside 0..{}",
            x.len()
        )));
    }
    let key = |i: i32| x.get(i as usize).copied().unwrap_or(0.0);
    p.sort_by(|&a, &b| {
        key(a)
            .partial_cmp(&key(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut out = vec![0u8; p.len() * 4];
    for (i, &v) in p.iter().enumerate() {
        memory::write_word(&mut out, i * 4, v);
    }
    vm.heap.array_write(p_id, 0, &out);
    Ok(())
}

fn math_iamax(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x_id = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let x = read_real_array(vm, x_id);
    // `iamax` in `external/inferno-os/libmath/blas.c` replaces the running
    // maximum only on a strict increase, so the first index of the largest
    // magnitude wins and a NaN never displaces a real value.
    let mut result = 0usize;
    if let Some(&first) = x.first() {
        let mut largest = first.abs();
        for (i, &v) in x.iter().enumerate().skip(1) {
            if largest < v.abs() {
                result = i;
                largest = v.abs();
            }
        }
    }
    memory::write_word(&mut vm.frames.data, base, result as i32);
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
    let raw_m = memory::read_word(&vm.frames.data, base + ARG1_OFF + 8);
    let raw_n = memory::read_word(&vm.frames.data, base + ARG1_OFF + 12);
    let raw_k = memory::read_word(&vm.frames.data, base + ARG1_OFF + 16);
    // alpha at offset 56 (8-byte aligned after 5 ints + 4 bytes padding)
    let alpha = memory::read_real(&vm.frames.data, base + 56);
    let a_id = memory::read_word(&vm.frames.data, base + 64) as u32;
    let raw_lda = memory::read_word(&vm.frames.data, base + 68);
    let b_id = memory::read_word(&vm.frames.data, base + 72) as u32;
    let raw_ldb = memory::read_word(&vm.frames.data, base + 76);
    let beta = memory::read_real(&vm.frames.data, base + 80);
    let c_id = memory::read_word(&vm.frames.data, base + 88) as u32;
    let raw_ldc = memory::read_word(&vm.frames.data, base + 92);

    let nota = transa == 'N';
    let notb = transb == 'N';

    // `Math_gemm` accepts only 'N', 'T', and 'C'. Anything else is a caller
    // error rather than a silent transpose.
    if (!nota && transa != 'T' && transa != 'C') || (!notb && transb != 'T' && transb != 'C') {
        return Err(ExecError::ThreadFault(format!(
            "gemm: transpose flag must be N, T, or C (transa={transa}, transb={transb})"
        )));
    }

    // Every dimension is untrusted: negatives would become huge `usize`
    // values and drive both the buffer allocation and the loop bounds.
    if raw_m < 0 || raw_n < 0 || raw_k < 0 || raw_lda < 0 || raw_ldb < 0 || raw_ldc < 0 {
        return Err(ExecError::ThreadFault(format!(
            "gemm: negative dimension (m={raw_m}, n={raw_n}, k={raw_k}, \
             lda={raw_lda}, ldb={raw_ldb}, ldc={raw_ldc})"
        )));
    }
    let (m, n, k) = (raw_m as usize, raw_n as usize, raw_k as usize);
    let (lda, ldb, ldc) = (raw_lda as usize, raw_ldb as usize, raw_ldc as usize);

    if m == 0 || n == 0 || ((alpha == 0.0 || k == 0) && beta == 1.0) {
        return Ok(());
    }

    // BLAS requires the leading dimensions to span each matrix; checking the
    // spans against the real array lengths bounds every loop and the C buffer.
    let elems = |id: u32| vm.heap.array_byte_len(id).unwrap_or(0) / 8;
    let a_rows = if nota { m } else { k };
    let b_rows = if notb { k } else { n };
    let a_cols = if nota { k } else { m };
    let b_cols = if notb { n } else { k };
    let span = |ld: usize, rows: usize, cols: usize| {
        if ld < rows.max(1) {
            None
        } else {
            ld.checked_mul(cols.saturating_sub(1))
                .and_then(|v| v.checked_add(rows))
        }
    };
    let c_size = match span(ldc, m, n) {
        Some(size) if size <= elems(c_id) => size,
        _ => {
            return Err(ExecError::ThreadFault(format!(
                "gemm: C does not hold {m}x{n} with ldc={ldc}"
            )));
        }
    };
    // A may legitimately be nil (C := alpha*op(B) + beta*C); that path and the
    // alpha == 0 path only walk m and n, which the C check already bounds.
    if alpha != 0.0 && a_id != 0 {
        if span(lda, a_rows, a_cols).is_none_or(|size| size > elems(a_id)) {
            return Err(ExecError::ThreadFault(format!(
                "gemm: A does not hold {a_rows}x{a_cols} with lda={lda}"
            )));
        }
        if span(ldb, b_rows, b_cols).is_none_or(|size| size > elems(b_id)) {
            return Err(ExecError::ThreadFault(format!(
                "gemm: B does not hold {b_rows}x{b_cols} with ldb={ldb}"
            )));
        }
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

// Byte-order conversion functions.
//
// Every import and export function is declared in
// `external/inferno-os/module/math.m` as `fn(b: array of byte, x: array of T)`,
// so the byte buffer is the first argument and the value array is the second,
// and each element of x maps to its big-endian encoding in b
// (`external/inferno-os/man/2/math-export`). The reference raises an error when
// the two lengths disagree; converting as many elements as both arrays hold is
// enough to keep every access in bounds.

/// Number of elements of `elem_size` bytes a heap array holds.
fn array_elems(vm: &VmState<'_>, id: u32, elem_size: usize) -> usize {
    vm.heap.array_byte_len(id).unwrap_or(0) / elem_size
}

/// Number of elements an import or export call converts.
fn conversion_count(vm: &VmState<'_>, b_id: u32, x_id: u32, elem_size: usize) -> usize {
    array_elems(vm, x_id, elem_size).min(array_elems(vm, b_id, 1) / elem_size)
}

fn math_export_int(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let b_id = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let x_id = memory::read_word(&vm.frames.data, base + ARG1_OFF + 4) as u32;
    let n = conversion_count(vm, b_id, x_id, 4);
    let x = read_int_array(vm, x_id);
    let mut out = vec![0u8; n * 4];
    for (i, &v) in x.iter().take(n).enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    vm.heap.array_write(b_id, 0, &out);
    Ok(())
}

fn math_export_real(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let b_id = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let x_id = memory::read_word(&vm.frames.data, base + ARG1_OFF + 4) as u32;
    let n = conversion_count(vm, b_id, x_id, 8);
    if n == 0 && array_elems(vm, b_id, 1) >= 8 {
        // Fallback for an older calling convention that passed the value as a
        // raw real at +40 instead of a one-element array.
        let val = memory::read_real(&vm.frames.data, base + ARG2_OFF);
        vm.heap.array_write(b_id, 0, &val.to_be_bytes());
        return Ok(());
    }
    let x = read_real_array(vm, x_id);
    let mut out = vec![0u8; n * 8];
    for (i, &v) in x.iter().take(n).enumerate() {
        out[i * 8..i * 8 + 8].copy_from_slice(&v.to_be_bytes());
    }
    vm.heap.array_write(b_id, 0, &out);
    Ok(())
}

fn math_export_real32(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let b_id = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let x_id = memory::read_word(&vm.frames.data, base + ARG1_OFF + 4) as u32;
    // The values are doubles in Limbo and narrow to single precision here.
    let n = array_elems(vm, x_id, 8).min(array_elems(vm, b_id, 1) / 4);
    let x = read_real_array(vm, x_id);
    let mut out = vec![0u8; n * 4];
    for (i, &v) in x.iter().take(n).enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&(v as f32).to_be_bytes());
    }
    vm.heap.array_write(b_id, 0, &out);
    Ok(())
}

fn math_import_int(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let b_id = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let x_id = memory::read_word(&vm.frames.data, base + ARG1_OFF + 4) as u32;
    let n = conversion_count(vm, b_id, x_id, 4);
    let bytes = vm.heap.array_read(b_id, 0, n * 4).unwrap_or_default();
    let mut out = vec![0u8; n * 4];
    for i in 0..n {
        let val = i32::from_be_bytes([
            bytes[i * 4],
            bytes[i * 4 + 1],
            bytes[i * 4 + 2],
            bytes[i * 4 + 3],
        ]);
        memory::write_word(&mut out, i * 4, val);
    }
    vm.heap.array_write(x_id, 0, &out);
    // The first value also lands in the frame return slot, which is where the
    // generated code for a one-element call reads it.
    let first = if n > 0 { memory::read_word(&out, 0) } else { 0 };
    memory::write_word(&mut vm.frames.data, base, first);
    Ok(())
}

fn math_import_real(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let b_id = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let x_id = memory::read_word(&vm.frames.data, base + ARG1_OFF + 4) as u32;
    let n = conversion_count(vm, b_id, x_id, 8);
    let bytes = vm.heap.array_read(b_id, 0, n * 8).unwrap_or_default();
    let mut out = vec![0u8; n * 8];
    let mut first = 0.0;
    for i in 0..n {
        let mut be = [0u8; 8];
        be.copy_from_slice(&bytes[i * 8..i * 8 + 8]);
        let val = f64::from_be_bytes(be);
        if i == 0 {
            first = val;
        }
        memory::write_real(&mut out, i * 8, val);
    }
    vm.heap.array_write(x_id, 0, &out);
    write_real_return(vm, base, first);
    Ok(())
}

fn math_import_real32(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let b_id = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let x_id = memory::read_word(&vm.frames.data, base + ARG1_OFF + 4) as u32;
    // Each 4-byte value widens into an 8-byte element of x.
    let n = array_elems(vm, x_id, 8).min(array_elems(vm, b_id, 1) / 4);
    let bytes = vm.heap.array_read(b_id, 0, n * 4).unwrap_or_default();
    let mut out = vec![0u8; n * 8];
    let mut first = 0.0;
    for i in 0..n {
        let val = f32::from_be_bytes([
            bytes[i * 4],
            bytes[i * 4 + 1],
            bytes[i * 4 + 2],
            bytes[i * 4 + 3],
        ]) as f64;
        if i == 0 {
            first = val;
        }
        memory::write_real(&mut out, i * 8, val);
    }
    vm.heap.array_write(x_id, 0, &out);
    write_real_return(vm, base, first);
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

    /// Helper: set one real argument and read the int return value.
    fn call_unary_int(f: fn(&mut VmState<'_>) -> Result<(), ExecError>, arg: f64) -> i32 {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, arg);
        f(&mut vm).expect("math function should succeed");
        memory::read_word(&vm.frames.data, base + RET_OFF)
    }

    /// Helper: call a function whose first argument is an int and whose second
    /// argument is a real, as `jn` and `yn` are declared in `module/math.m`.
    fn call_int_real(f: fn(&mut VmState<'_>) -> Result<(), ExecError>, n: i32, x: f64) -> f64 {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF, n);
        memory::write_real(&mut vm.frames.data, base + ARG2_OFF, x);
        f(&mut vm).expect("math function should succeed");
        memory::read_real(&vm.frames.data, base + RET_OFF)
    }

    /// Assert that two reals agree to within a relative error of `tol`.
    fn assert_close(got: f64, want: f64, tol: f64, what: &str) {
        let err = if want == 0.0 {
            got.abs()
        } else {
            ((got - want) / want).abs()
        };
        assert!(
            err <= tol,
            "{what}: want {want:e}, got {got:e}, rel error {err:e}"
        );
    }

    // ---- Return value plumbing ----

    /// A real return value must reach both frame offset 0 and the caller's
    /// storage through the return pointer at offset 16, because the 4-byte
    /// mcall return copy cannot move an 8-byte value.
    #[test]
    fn write_real_return_follows_the_return_pointer() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        // Offsets 20..32 of the frame are reserved padding, so they are a safe
        // target for the returned value.
        let target = base + 24;
        memory::write_word(&mut vm.frames.data, base + 16, target as i32);
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, 9.0);
        math_sqrt(&mut vm).expect("sqrt should succeed");
        assert_eq!(memory::read_real(&vm.frames.data, base + RET_OFF), 3.0);
        assert_eq!(memory::read_real(&vm.frames.data, target), 3.0);
    }

    /// A big return value takes the same path as a real one.
    #[test]
    fn write_big_return_follows_the_return_pointer() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        let target = base + 24;
        memory::write_word(&mut vm.frames.data, base + 16, target as i32);
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, 1.0);
        math_realbits64(&mut vm).expect("realbits64 should succeed");
        let want = 0x3ff0_0000_0000_0000_u64 as i64;
        assert_eq!(memory::read_big(&vm.frames.data, base + RET_OFF), want);
        assert_eq!(memory::read_big(&vm.frames.data, target), want);

        // An unmapped pointer leaves the value at frame offset 0 alone.
        let mut vm = VmState::new(&module).expect("vm should initialize");
        memory::write_word(
            &mut vm.frames.data,
            base + 16,
            crate::address::MP_LIMIT as i32,
        );
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, 1.0);
        math_realbits64(&mut vm).expect("realbits64 should succeed");
        assert_eq!(memory::read_big(&vm.frames.data, base + RET_OFF), want);
    }

    /// A Limbo global lives in the module data area, so the return pointer can
    /// address a module MP rather than the frame.
    #[test]
    fn returns_reach_a_module_global() {
        let mut module = test_module();
        module.header.data_size = 64;
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        let target = crate::address::MP_BASE + 8;
        memory::write_word(&mut vm.frames.data, base + 16, target as i32);
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, 9.0);
        math_sqrt(&mut vm).expect("sqrt should succeed");
        assert_eq!(memory::read_real(&vm.mp, 8), 3.0);

        let mut vm = VmState::new(&module).expect("vm should initialize");
        memory::write_word(&mut vm.frames.data, base + 16, target as i32);
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, 1.0);
        math_realbits64(&mut vm).expect("realbits64 should succeed");
        assert_eq!(
            memory::read_big(&vm.mp, 8),
            0x3ff0_0000_0000_0000_u64 as i64
        );
    }

    /// A module MP offset with no room for the value is left alone.
    #[test]
    fn returns_skip_a_module_global_that_does_not_fit() {
        let mut module = test_module();
        module.header.data_size = 16;
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        let target = crate::address::MP_BASE + 12;
        memory::write_word(&mut vm.frames.data, base + 16, target as i32);
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, 9.0);
        math_sqrt(&mut vm).expect("sqrt should succeed");
        assert_eq!(memory::read_real(&vm.frames.data, base + RET_OFF), 3.0);
        assert_eq!(vm.mp, vec![0u8; 16], "the MP must not be touched");
    }

    /// An unmapped return pointer must be ignored rather than written through.
    #[test]
    fn write_real_return_ignores_an_unmapped_pointer() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        // The gap between the module MP ranges and the heap id space is
        // unmapped, so `decode_virtual_addr` reports no target.
        memory::write_word(
            &mut vm.frames.data,
            base + 16,
            crate::address::MP_LIMIT as i32,
        );
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, 9.0);
        math_sqrt(&mut vm).expect("sqrt should succeed");
        assert_eq!(memory::read_real(&vm.frames.data, base + RET_OFF), 3.0);
    }

    /// Every real-returning Math function must use the same return path, so
    /// fdim has to reach the caller through the return pointer too.
    #[test]
    fn fdim_follows_the_return_pointer() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        let target = base + 24;
        memory::write_word(&mut vm.frames.data, base + 16, target as i32);
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, 5.0);
        memory::write_real(&mut vm.frames.data, base + ARG2_OFF, 3.0);
        math_fdim(&mut vm).expect("fdim should succeed");
        assert_eq!(memory::read_real(&vm.frames.data, base + RET_OFF), 2.0);
        assert_eq!(memory::read_real(&vm.frames.data, target), 2.0);
    }

    /// fdim, fmax, and fmin are defined in `external/inferno-os/libmath/fdim.c`
    /// by a single comparison, so NaN arguments fall through to the else arm.
    #[test]
    fn fdim_edge_cases() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, f64::NAN);
        memory::write_real(&mut vm.frames.data, base + ARG2_OFF, 1.0);
        math_fdim(&mut vm).expect("fdim should succeed");
        assert_eq!(memory::read_real(&vm.frames.data, base + RET_OFF), 0.0);

        let mut vm = VmState::new(&module).expect("vm should initialize");
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, f64::INFINITY);
        memory::write_real(&mut vm.frames.data, base + ARG2_OFF, 1.0);
        math_fdim(&mut vm).expect("fdim should succeed");
        assert_eq!(
            memory::read_real(&vm.frames.data, base + RET_OFF),
            f64::INFINITY
        );
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
    fn rint_rounds_half_to_even() {
        // rint uses the current rounding mode: round-half-to-even, not
        // round-half-away-from-zero.
        let r = call_unary_real(math_rint, 2.5);
        assert!((r - 2.0).abs() < 1e-15, "rint(2.5) = 2, got {r}");
        let r = call_unary_real(math_rint, 3.5);
        assert!((r - 4.0).abs() < 1e-15, "rint(3.5) = 4, got {r}");
        let r = call_unary_real(math_rint, -2.5);
        assert!((r - (-2.0)).abs() < 1e-15, "rint(-2.5) = -2, got {r}");
        let r = call_unary_real(math_rint, 2.7);
        assert!((r - 3.0).abs() < 1e-15, "rint(2.7) = 3, got {r}");
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

    fn call_scalbn(x: f64, n: i32) -> f64 {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, x);
        memory::write_word(&mut vm.frames.data, base + ARG2_OFF, n);
        math_scalbn(&mut vm).expect("scalbn should succeed");
        memory::read_real(&vm.frames.data, base + RET_OFF)
    }

    #[test]
    fn scalbn_basic() {
        // scalbn(1.5, 3) = 1.5 * 2^3 = 12.0
        let result = call_scalbn(1.5, 3);
        assert!(
            (result - 12.0).abs() < 1e-12,
            "scalbn(1.5,3) = 12.0, got {result}"
        );
        assert_eq!(call_scalbn(1.5, 0), 1.5);
        assert_eq!(call_scalbn(1.5, -1), 0.75);
        assert_eq!(call_scalbn(0.0, 100), 0.0);
    }

    #[test]
    fn scalbn_scales_in_steps() {
        // The intermediate 2^n must not overflow/underflow independently of
        // the product: both of these results are finite and non-zero.
        let expected = 1e-300 * 2.0_f64.powi(1000) * 2.0_f64.powi(1000);
        let result = call_scalbn(1e-300, 2000);
        assert!(result.is_finite(), "scalbn(1e-300, 2000) should be finite");
        assert!(
            ((result - expected) / expected).abs() < 1e-12,
            "scalbn(1e-300,2000) = {expected}, got {result}"
        );

        let expected = 1e300 * 2.0_f64.powi(-1000) * 2.0_f64.powi(-1000);
        let result = call_scalbn(1e300, -2000);
        assert!(result > 0.0, "scalbn(1e300, -2000) should not underflow");
        assert!(
            ((result - expected) / expected).abs() < 1e-12,
            "scalbn(1e300,-2000) = {expected}, got {result}"
        );
    }

    #[test]
    fn scalbn_saturates_at_the_extremes() {
        assert!(call_scalbn(1.0, 100_000).is_infinite());
        assert_eq!(call_scalbn(1.0, -100_000), 0.0);
    }

    // ---- remainder ----

    #[test]
    fn remainder_is_ieee_not_fmod() {
        // remainder rounds the quotient half-to-even; fmod truncates it.
        let r = call_binary_real(math_remainder, 7.0, 2.0);
        assert!((r - (-1.0)).abs() < 1e-15, "remainder(7,2) = -1, got {r}");
        let r = call_binary_real(math_remainder, 5.0, 2.0);
        assert!((r - 1.0).abs() < 1e-15, "remainder(5,2) = 1, got {r}");
        let r = call_binary_real(math_remainder, -7.0, 2.0);
        assert!((r - 1.0).abs() < 1e-15, "remainder(-7,2) = 1, got {r}");
        let r = call_binary_real(math_remainder, 8.0, 3.0);
        assert!((r - (-1.0)).abs() < 1e-15, "remainder(8,3) = -1, got {r}");
        let r = call_binary_real(math_remainder, 7.0, 3.0);
        assert!((r - 1.0).abs() < 1e-15, "remainder(7,3) = 1, got {r}");
        assert!(
            call_binary_real(math_remainder, 1.0, 0.0).is_nan(),
            "remainder(x,0) is NaN"
        );
    }

    // ---- nextafter ----

    #[test]
    fn nextafter_around_zero() {
        let tiny = f64::from_bits(1); // 5e-324
        let r = call_binary_real(math_nextafter, 0.0, -1.0);
        assert_eq!(r, -tiny, "nextafter(+0,-1) = -5e-324, got {r}");
        let r = call_binary_real(math_nextafter, -0.0, 1.0);
        assert_eq!(r, tiny, "nextafter(-0,1) = 5e-324, got {r}");
        let r = call_binary_real(math_nextafter, 0.0, 1.0);
        assert_eq!(r, tiny, "nextafter(+0,1) = 5e-324, got {r}");
    }

    #[test]
    fn nextafter_normal_values() {
        let up = f64::from_bits(1.0_f64.to_bits() + 1);
        let down = f64::from_bits(1.0_f64.to_bits() - 1);
        assert_eq!(call_binary_real(math_nextafter, 1.0, 2.0), up);
        assert_eq!(call_binary_real(math_nextafter, 1.0, 0.0), down);
        let neg_up = f64::from_bits((-1.0_f64).to_bits() + 1);
        assert_eq!(call_binary_real(math_nextafter, -1.0, -2.0), neg_up);
        assert_eq!(call_binary_real(math_nextafter, 3.0, 3.0), 3.0);
        assert!(call_binary_real(math_nextafter, f64::NAN, 1.0).is_nan());
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
        // `external/inferno-os/libmath/fdlibm/s_ilogb.c` returns 0x80000001,
        // which is one above i32::MIN.
        let want = 0x8000_0001_u32 as i32;
        assert_eq!(result, want, "ilogb(0) = 0x80000001, got {result}");
    }

    /// ilogb returns the binary exponent of x, that is floor(log2(|x|)).
    /// `external/inferno-os/libmath/fdlibm/s_ilogb.c` documents the special
    /// cases: ilogb(0) is 0x80000001 and ilogb(inf) and ilogb(NaN) are
    /// 0x7fffffff.
    #[test]
    fn ilogb_reference_cases() {
        assert_eq!(call_unary_int(math_ilogb, 1.0), 0);
        assert_eq!(call_unary_int(math_ilogb, 0.5), -1);
        assert_eq!(call_unary_int(math_ilogb, -8.0), 3);
        assert_eq!(call_unary_int(math_ilogb, 1023.0), 9);
        assert_eq!(call_unary_int(math_ilogb, f64::MAX), 1023);
        // The exponent of the smallest normal is -1022, and subnormals run
        // down to 2^-1074.
        assert_eq!(call_unary_int(math_ilogb, f64::MIN_POSITIVE), -1022);
        assert_eq!(call_unary_int(math_ilogb, f64::from_bits(1)), -1074);
        assert_eq!(
            call_unary_int(math_ilogb, f64::from_bits(0x000f_ffff_ffff_ffff)),
            -1023
        );
        assert_eq!(call_unary_int(math_ilogb, 0.0), 0x8000_0001_u32 as i32);
        assert_eq!(call_unary_int(math_ilogb, -0.0), 0x8000_0001_u32 as i32);
        assert_eq!(call_unary_int(math_ilogb, f64::INFINITY), i32::MAX);
        assert_eq!(call_unary_int(math_ilogb, f64::NEG_INFINITY), i32::MAX);
        assert_eq!(call_unary_int(math_ilogb, f64::NAN), i32::MAX);
    }

    /// The largest double below a power of two still has the exponent of the
    /// power below it. A log2 based implementation rounds the logarithm up to
    /// the integer and reports one too many.
    #[test]
    fn ilogb_just_below_a_power_of_two() {
        for k in [4_i32, 32, 100, 512, 1023, -100, -1000] {
            let below = f64::from_bits(2.0_f64.powi(k).to_bits() - 1);
            assert_eq!(
                call_unary_int(math_ilogb, below),
                k - 1,
                "ilogb of the largest double below 2^{k}"
            );
        }
    }

    #[test]
    fn finite_and_isnan_edges() {
        assert_eq!(call_unary_int(math_finite, f64::NEG_INFINITY), 0);
        assert_eq!(call_unary_int(math_finite, f64::NAN), 0);
        assert_eq!(call_unary_int(math_finite, f64::from_bits(1)), 1);
        assert_eq!(call_unary_int(math_finite, -0.0), 1);
        assert_eq!(call_unary_int(math_isnan, f64::INFINITY), 0);
        assert_eq!(call_unary_int(math_isnan, -0.0), 0);
    }

    /// pow10 is `pow(10., n)` in `external/inferno-os/libmath/pow10.c`, which is
    /// correctly rounded. Repeated squaring is not: it is off by one unit in the
    /// last place for most exponents, including this one.
    #[test]
    fn pow10_is_correctly_rounded() {
        let call_pow10 = |p: i32| {
            let module = test_module();
            let mut vm = VmState::new(&module).expect("vm should initialize");
            let base = vm.frames.current_data_offset();
            memory::write_word(&mut vm.frames.data, base + ARG1_OFF, p);
            math_pow10(&mut vm).expect("pow10 should succeed");
            memory::read_real(&vm.frames.data, base + RET_OFF)
        };
        assert_eq!(call_pow10(0), 1.0);
        assert_eq!(call_pow10(1), 10.0);
        assert_eq!(call_pow10(-1), 0.1);
        assert_eq!(call_pow10(-23), 1e-23);
        assert_eq!(call_pow10(22), 1e22);
        assert_eq!(call_pow10(308), 1e308);
        assert_eq!(call_pow10(400), f64::INFINITY);
        assert_eq!(call_pow10(-400), 0.0);
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

    // ---- Inverse trigonometric and hyperbolic edges ----

    /// acos and asin are defined only on [-1, 1]; outside it C returns NaN.
    #[test]
    fn asin_and_acos_domain() {
        assert_eq!(call_unary_real(math_asin, 1.0), std::f64::consts::FRAC_PI_2);
        assert_eq!(
            call_unary_real(math_asin, -1.0),
            -std::f64::consts::FRAC_PI_2
        );
        assert!(call_unary_real(math_asin, 2.0).is_nan(), "asin(2) is NaN");
        assert!(call_unary_real(math_acos, -2.0).is_nan(), "acos(-2) is NaN");
        assert_eq!(call_unary_real(math_acos, 0.0), std::f64::consts::FRAC_PI_2);
        assert_eq!(call_unary_real(math_acos, -1.0), std::f64::consts::PI);
        // asin(-0) is -0, so the sign of zero has to survive the call.
        assert!(call_unary_real(math_asin, -0.0).is_sign_negative());
    }

    #[test]
    fn atan_infinities() {
        assert_eq!(
            call_unary_real(math_atan, f64::INFINITY),
            std::f64::consts::FRAC_PI_2
        );
        assert_eq!(
            call_unary_real(math_atan, f64::NEG_INFINITY),
            -std::f64::consts::FRAC_PI_2
        );
        assert!(call_unary_real(math_atan, -0.0).is_sign_negative());
    }

    /// atan2 takes y first and x second, matching `atan2(y, x)` in
    /// `external/inferno-os/module/math.m`.
    #[test]
    fn atan2_argument_order_and_quadrants() {
        // A swapped implementation would return 0 here instead of pi/2.
        assert_eq!(
            call_binary_real(math_atan2, 1.0, 0.0),
            std::f64::consts::FRAC_PI_2
        );
        assert_eq!(call_binary_real(math_atan2, 0.0, 1.0), 0.0);
        assert_eq!(
            call_binary_real(math_atan2, 0.0, -1.0),
            std::f64::consts::PI
        );
        assert_eq!(
            call_binary_real(math_atan2, -0.0, -1.0),
            -std::f64::consts::PI
        );
        assert_eq!(
            call_binary_real(math_atan2, f64::INFINITY, f64::INFINITY),
            std::f64::consts::FRAC_PI_4
        );
        assert!(call_binary_real(math_atan2, -0.0, 0.0).is_sign_negative());
    }

    #[test]
    fn acosh_asinh_atanh_values_and_domains() {
        assert_eq!(call_unary_real(math_acosh, 1.0), 0.0);
        assert_close(
            call_unary_real(math_acosh, 2.0),
            1.316_957_896_924_816_6,
            1e-15,
            "acosh(2)",
        );
        assert!(
            call_unary_real(math_acosh, 0.5).is_nan(),
            "acosh is undefined below 1"
        );
        assert_eq!(call_unary_real(math_acosh, f64::INFINITY), f64::INFINITY);

        assert_eq!(call_unary_real(math_asinh, 0.0), 0.0);
        assert_close(
            call_unary_real(math_asinh, 1.0),
            0.881_373_587_019_543,
            1e-15,
            "asinh(1)",
        );
        assert_eq!(
            call_unary_real(math_asinh, -1.0),
            -call_unary_real(math_asinh, 1.0)
        );

        assert_eq!(call_unary_real(math_atanh, 0.0), 0.0);
        assert_close(
            call_unary_real(math_atanh, 0.5),
            0.549_306_144_334_054_9,
            1e-15,
            "atanh(0.5)",
        );
        assert_eq!(call_unary_real(math_atanh, 1.0), f64::INFINITY);
        assert_eq!(call_unary_real(math_atanh, -1.0), f64::NEG_INFINITY);
        assert!(
            call_unary_real(math_atanh, 2.0).is_nan(),
            "atanh is undefined above 1"
        );
    }

    // ---- Rounding, absolute value, and roots at the edges ----

    #[test]
    fn sqrt_and_cbrt_edges() {
        assert!(call_unary_real(math_sqrt, -1.0).is_nan(), "sqrt(-1) is NaN");
        // IEEE 754 requires sqrt(-0) to be -0.
        let r = call_unary_real(math_sqrt, -0.0);
        assert!(r == 0.0 && r.is_sign_negative(), "sqrt(-0) = -0, got {r}");
        assert_eq!(call_unary_real(math_sqrt, f64::INFINITY), f64::INFINITY);
        assert_eq!(call_unary_real(math_cbrt, -27.0), -3.0);
        assert!(call_unary_real(math_cbrt, -0.0).is_sign_negative());
    }

    #[test]
    fn fabs_edges() {
        let r = call_unary_real(math_fabs, -0.0);
        assert!(r == 0.0 && r.is_sign_positive(), "fabs(-0) = +0, got {r}");
        assert_eq!(call_unary_real(math_fabs, f64::NEG_INFINITY), f64::INFINITY);
        assert!(call_unary_real(math_fabs, f64::NAN).is_nan());
    }

    #[test]
    fn floor_and_ceil_edges() {
        assert_eq!(call_unary_real(math_floor, -0.5), -1.0);
        assert!(call_unary_real(math_floor, -0.0).is_sign_negative());
        assert_eq!(call_unary_real(math_floor, f64::INFINITY), f64::INFINITY);
        // ceil(-0.5) is -0, not +0.
        let r = call_unary_real(math_ceil, -0.5);
        assert!(r == 0.0 && r.is_sign_negative(), "ceil(-0.5) = -0, got {r}");
        assert_eq!(
            call_unary_real(math_ceil, f64::NEG_INFINITY),
            f64::NEG_INFINITY
        );
    }

    #[test]
    fn rint_edges() {
        // Half-to-even applies at every tie, not only at 2.5.
        assert_eq!(call_unary_real(math_rint, 0.5), 0.0);
        assert_eq!(call_unary_real(math_rint, 1.5), 2.0);
        assert_eq!(call_unary_real(math_rint, 4.5), 4.0);
        assert_eq!(call_unary_real(math_rint, -1.5), -2.0);
        let r = call_unary_real(math_rint, -0.5);
        assert!(r == 0.0 && r.is_sign_negative(), "rint(-0.5) = -0, got {r}");
        assert!(call_unary_real(math_rint, -0.0).is_sign_negative());
        // Values too large to have a fractional part come back unchanged.
        assert_eq!(call_unary_real(math_rint, 1e300), 1e300);
        assert_eq!(call_unary_real(math_rint, f64::INFINITY), f64::INFINITY);
        assert!(call_unary_real(math_rint, f64::NAN).is_nan());
    }

    // ---- Logarithms and exponentials at the edges ----

    #[test]
    fn log_family_edges() {
        assert_eq!(call_unary_real(math_log, 0.0), f64::NEG_INFINITY);
        assert!(call_unary_real(math_log, -1.0).is_nan(), "ln(-1) is NaN");
        assert_eq!(call_unary_real(math_log, f64::INFINITY), f64::INFINITY);
        assert_eq!(call_unary_real(math_log10, 0.0), f64::NEG_INFINITY);
        assert_eq!(call_unary_real(math_log10, 1000.0), 3.0);
        assert!(call_unary_real(math_log10, -1.0).is_nan());
        assert_eq!(call_unary_real(math_log1p, -1.0), f64::NEG_INFINITY);
        assert!(call_unary_real(math_log1p, -2.0).is_nan());
        // log1p keeps full precision where ln(1+x) would lose all of it.
        assert_eq!(call_unary_real(math_log1p, 1e-16), 1e-16);
    }

    #[test]
    fn exp_family_edges() {
        assert_eq!(call_unary_real(math_exp, 1000.0), f64::INFINITY);
        assert_eq!(call_unary_real(math_exp, -1000.0), 0.0);
        assert_eq!(call_unary_real(math_exp, f64::NEG_INFINITY), 0.0);
        // expm1 keeps full precision where exp(x)-1 would lose all of it.
        assert_eq!(call_unary_real(math_expm1, 1e-16), 1e-16);
        assert_eq!(call_unary_real(math_expm1, f64::NEG_INFINITY), -1.0);
        assert_close(
            call_unary_real(math_expm1, 1.0),
            std::f64::consts::E - 1.0,
            1e-15,
            "expm1(1)",
        );
    }

    #[test]
    fn hyperbolic_values_and_edges() {
        assert_close(
            call_unary_real(math_sinh, 1.0),
            1.175_201_193_643_801_4,
            1e-15,
            "sinh(1)",
        );
        assert_close(
            call_unary_real(math_cosh, -1.0),
            1.543_080_634_815_243_7,
            1e-15,
            "cosh(1)",
        );
        assert!(call_unary_real(math_sinh, -0.0).is_sign_negative());
        assert_eq!(call_unary_real(math_sinh, 1000.0), f64::INFINITY);
        assert_eq!(call_unary_real(math_cosh, f64::NEG_INFINITY), f64::INFINITY);
        assert_eq!(call_unary_real(math_tanh, f64::INFINITY), 1.0);
        assert_eq!(call_unary_real(math_tanh, f64::NEG_INFINITY), -1.0);
    }

    // ---- Binary operations at the edges ----

    /// fmod keeps the sign of x and is exact, unlike remainder.
    #[test]
    fn fmod_edges() {
        assert_eq!(call_binary_real(math_fmod, -7.0, 3.0), -1.0);
        assert_eq!(call_binary_real(math_fmod, 7.0, -3.0), 1.0);
        assert!(call_binary_real(math_fmod, 5.0, 0.0).is_nan(), "fmod(x,0)");
        assert!(
            call_binary_real(math_fmod, f64::INFINITY, 2.0).is_nan(),
            "fmod(inf,y)"
        );
        assert_eq!(call_binary_real(math_fmod, 2.0, f64::INFINITY), 2.0);
        assert!(call_binary_real(math_fmod, -0.0, 1.0).is_sign_negative());
    }

    #[test]
    fn hypot_edges() {
        assert_eq!(call_binary_real(math_hypot, 0.0, 0.0), 0.0);
        assert_eq!(call_binary_real(math_hypot, -3.0, -4.0), 5.0);
        // The intermediate squares must not overflow.
        assert_close(
            call_binary_real(math_hypot, 1e300, 1e300),
            1.414_213_562_373_095_1e300,
            1e-15,
            "hypot(1e300,1e300)",
        );
        // C99 requires hypot(inf, NaN) to be inf.
        assert_eq!(
            call_binary_real(math_hypot, f64::INFINITY, f64::NAN),
            f64::INFINITY
        );
    }

    /// C99 fmax and fmin return the non-NaN operand. The naive comparison in
    /// `external/inferno-os/libmath/fdim.c` returns NaN when y is NaN, so this
    /// is a deliberate divergence from that reference in favour of the standard.
    #[test]
    fn fmax_and_fmin_with_nan_and_infinities() {
        assert_eq!(call_binary_real(math_fmax, f64::NAN, 1.0), 1.0);
        assert_eq!(call_binary_real(math_fmax, 1.0, f64::NAN), 1.0);
        assert_eq!(call_binary_real(math_fmin, f64::NAN, 1.0), 1.0);
        assert_eq!(call_binary_real(math_fmin, 1.0, f64::NAN), 1.0);
        assert!(call_binary_real(math_fmax, f64::NAN, f64::NAN).is_nan());
        assert_eq!(
            call_binary_real(math_fmax, f64::INFINITY, 1.0),
            f64::INFINITY
        );
        assert_eq!(
            call_binary_real(math_fmin, f64::NEG_INFINITY, 1.0),
            f64::NEG_INFINITY
        );
    }

    /// copysign takes the value first and the sign donor second, matching
    /// `copysign(x, s)` in `external/inferno-os/module/math.m`.
    #[test]
    fn copysign_argument_order_and_zeros() {
        // A swapped implementation would return -3 here instead of 1.
        assert_eq!(call_binary_real(math_copysign, -1.0, 3.0), 1.0);
        assert_eq!(call_binary_real(math_copysign, 3.0, -0.0), -3.0);
        assert_eq!(call_binary_real(math_copysign, -3.0, 0.0), 3.0);
        assert_eq!(
            call_binary_real(math_copysign, f64::INFINITY, -1.0),
            f64::NEG_INFINITY
        );
        assert!(call_binary_real(math_copysign, f64::NAN, -1.0).is_nan());
    }

    #[test]
    fn pow_special_cases() {
        // C99 pow: pow(x, 0) is 1 for every x, even NaN, and pow(1, y) is 1.
        assert_eq!(call_binary_real(math_pow, 0.0, 0.0), 1.0);
        assert_eq!(call_binary_real(math_pow, f64::NAN, 0.0), 1.0);
        assert_eq!(call_binary_real(math_pow, 1.0, f64::NAN), 1.0);
        assert_eq!(call_binary_real(math_pow, -1.0, f64::INFINITY), 1.0);
        assert_eq!(call_binary_real(math_pow, -2.0, 3.0), -8.0);
        assert_eq!(call_binary_real(math_pow, 2.0, -1.0), 0.5);
        // A negative base with a non-integer exponent has no real value.
        assert!(
            call_binary_real(math_pow, -8.0, 1.0 / 3.0).is_nan(),
            "pow(-8, 1/3) is NaN"
        );
        assert_eq!(call_binary_real(math_pow, 0.0, -1.0), f64::INFINITY);
    }

    /// The IEEE remainder is exact and its magnitude never exceeds half of |y|
    /// (see the description of remainder in `external/inferno-os/man/2/math-fp`).
    #[test]
    fn remainder_edges() {
        assert_eq!(call_binary_real(math_remainder, 0.5, 1.0), 0.5);
        assert_eq!(call_binary_real(math_remainder, 1.5, 1.0), -0.5);
        assert_eq!(call_binary_real(math_remainder, 2.5, 1.0), 0.5);
        assert_eq!(call_binary_real(math_remainder, 5.0, f64::INFINITY), 5.0);
        assert!(
            call_binary_real(math_remainder, f64::INFINITY, 2.0).is_nan(),
            "remainder(inf,y) is NaN"
        );
        assert!(call_binary_real(math_remainder, f64::NAN, 1.0).is_nan());
        assert!(call_binary_real(math_remainder, 1.0, f64::NAN).is_nan());
        // remainder(+-0, y) is +-0.
        let r = call_binary_real(math_remainder, -0.0, 1.0);
        assert!(r == 0.0 && r.is_sign_negative(), "remainder(-0,1) = -0");
        let r = call_binary_real(math_remainder, 4.0, 2.0);
        assert!(r == 0.0 && r.is_sign_positive(), "remainder(4,2) = +0");
        let r = call_binary_real(math_remainder, -4.0, 2.0);
        assert!(r == 0.0 && r.is_sign_negative(), "remainder(-4,2) = -0");
        // The sign of y is irrelevant.
        assert_eq!(
            call_binary_real(math_remainder, 7.0, -2.0),
            call_binary_real(math_remainder, 7.0, 2.0)
        );
    }

    #[test]
    fn scalbn_edges() {
        assert!(call_scalbn(-0.0, 5).is_sign_negative(), "scalbn(-0,n) = -0");
        assert!(call_scalbn(f64::NAN, 1).is_nan());
        assert_eq!(call_scalbn(f64::INFINITY, -5000), f64::INFINITY);
        assert_eq!(call_scalbn(f64::MAX, 1), f64::INFINITY);
        // The exponent argument may be any int, including the extremes.
        assert_eq!(call_scalbn(1.0, i32::MAX), f64::INFINITY);
        assert_eq!(call_scalbn(1.0, i32::MIN), 0.0);
        // 2^-1074 is the smallest subnormal, so this scaling is exact.
        assert_eq!(call_scalbn(1.0, -1074), f64::from_bits(1));
        assert_eq!(call_scalbn(f64::from_bits(1), 1074), 1.0);
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

    // ---- Bit conversions ----

    #[test]
    fn realbits_and_bits_known_patterns() {
        assert_eq!(
            call_unary_int(math_realbits32, -0.0),
            0x8000_0000_u32 as i32
        );
        assert_eq!(call_unary_int(math_realbits32, 1.0), 0x3f80_0000);
        // 0.1 is not representable in binary, so it rounds to the nearest f32.
        assert_eq!(call_unary_int(math_realbits32, 0.1), 0x3dcc_cccd);
        // A double too large for an f32 becomes an f32 infinity.
        assert_eq!(call_unary_int(math_realbits32, 1e300), 0x7f80_0000);

        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, -0.0);
        math_realbits64(&mut vm).expect("realbits64 should succeed");
        assert_eq!(
            memory::read_big(&vm.frames.data, base + RET_OFF),
            0x8000_0000_0000_0000_u64 as i64
        );
    }

    #[test]
    fn bits_to_real_known_patterns() {
        let call_bits32 = |bits: i32| {
            let module = test_module();
            let mut vm = VmState::new(&module).expect("vm should initialize");
            let base = vm.frames.current_data_offset();
            memory::write_word(&mut vm.frames.data, base + ARG1_OFF, bits);
            math_bits32real(&mut vm).expect("bits32real should succeed");
            memory::read_real(&vm.frames.data, base + RET_OFF)
        };
        assert_eq!(call_bits32(0x3f80_0000), 1.0);
        assert_eq!(call_bits32(0x7f80_0000), f64::INFINITY);
        assert_eq!(call_bits32(0xff80_0000_u32 as i32), f64::NEG_INFINITY);
        // The smallest f32 subnormal widens exactly to a double.
        assert_eq!(call_bits32(1), 1.401_298_464_324_817e-45);
        assert!(call_bits32(0x7fc0_0000).is_nan());

        let call_bits64 = |bits: i64| {
            let module = test_module();
            let mut vm = VmState::new(&module).expect("vm should initialize");
            let base = vm.frames.current_data_offset();
            memory::write_big(&mut vm.frames.data, base + ARG1_OFF, bits);
            math_bits64real(&mut vm).expect("bits64real should succeed");
            memory::read_real(&vm.frames.data, base + RET_OFF)
        };
        assert_eq!(
            call_bits64(0xfff0_0000_0000_0000_u64 as i64),
            f64::NEG_INFINITY
        );
        assert_eq!(call_bits64(1), f64::from_bits(1));
        assert!(call_bits64(0x8000_0000_0000_0000_u64 as i64).is_sign_negative());
    }

    // ---- erf and erfc ----

    /// `erf_approx` is formula 7.1.25 of Abramowitz and Stegun, whose absolute
    /// error bound is 2.5e-5. That bound is the tolerance every erf test uses.
    const ERF_TOL: f64 = 2.5e-5;

    #[test]
    fn erf_known_values() {
        let cases = [
            (0.5, 0.520_499_877_813_046_5),
            (1.0, 0.842_700_792_949_714_9),
            (2.0, 0.995_322_265_018_952_7),
            (3.0, 0.999_977_909_503_001_4),
        ];
        for (x, want) in cases {
            let got = call_unary_real(math_erf, x);
            assert!(
                (got - want).abs() <= ERF_TOL,
                "erf({x}): want {want}, got {got}"
            );
            // erf is odd.
            let got_neg = call_unary_real(math_erf, -x);
            assert_eq!(got_neg, -got, "erf(-x) = -erf(x)");
        }
    }

    #[test]
    fn erf_edges() {
        assert_eq!(call_unary_real(math_erf, 0.0), 0.0);
        // erf is odd, so erf(-0) is -0.
        let r = call_unary_real(math_erf, -0.0);
        assert!(r == 0.0 && r.is_sign_negative(), "erf(-0) = -0, got {r}");
        assert_eq!(call_unary_real(math_erf, f64::INFINITY), 1.0);
        assert_eq!(call_unary_real(math_erf, f64::NEG_INFINITY), -1.0);
        assert!(call_unary_real(math_erf, f64::NAN).is_nan());
    }

    /// The man page `external/inferno-os/man/2/math-elem` defines erfc(x) as
    /// 1-erf(x), so the two must agree exactly.
    #[test]
    fn erfc_is_one_minus_erf() {
        for x in [0.0, 0.5, 1.0, 2.0, -1.0] {
            let erf = call_unary_real(math_erf, x);
            let erfc = call_unary_real(math_erfc, x);
            assert_eq!(erfc, 1.0 - erf, "erfc({x}) = 1 - erf({x})");
        }
        assert_eq!(call_unary_real(math_erfc, 0.0), 1.0);
        assert_eq!(call_unary_real(math_erfc, f64::INFINITY), 0.0);
        assert_eq!(call_unary_real(math_erfc, f64::NEG_INFINITY), 2.0);
        assert!(call_unary_real(math_erfc, f64::NAN).is_nan());
    }

    // ---- modf ----

    /// Call modf and return its (integer, fractional) pair.
    fn call_modf(x: f64) -> (f64, f64) {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let base = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, base + ARG1_OFF, x);
        math_modf(&mut vm).expect("modf should succeed");
        (
            memory::read_real(&vm.frames.data, base + RET_OFF),
            memory::read_real(&vm.frames.data, base + RET_OFF + 8),
        )
    }

    /// C99 modf gives both parts the sign of x, and the fractional part of an
    /// infinity is a zero rather than a NaN.
    #[test]
    fn modf_signs_and_infinities() {
        assert_eq!(call_modf(-3.75), (-3.0, -0.75));
        assert_eq!(call_modf(0.5), (0.0, 0.5));
        let (i, f) = call_modf(-0.25);
        assert!(i == 0.0 && i.is_sign_negative(), "modf(-0.25) int = -0");
        assert_eq!(f, -0.25);
        let (i, f) = call_modf(f64::INFINITY);
        assert_eq!(i, f64::INFINITY);
        assert!(f == 0.0 && f.is_sign_positive(), "modf(inf) frac = +0");
        let (i, f) = call_modf(f64::NEG_INFINITY);
        assert_eq!(i, f64::NEG_INFINITY);
        assert!(f == 0.0 && f.is_sign_negative(), "modf(-inf) frac = -0");
        let (i, f) = call_modf(-0.0);
        assert!(i.is_sign_negative() && f.is_sign_negative(), "modf(-0)");
        let (i, f) = call_modf(f64::NAN);
        assert!(i.is_nan() && f.is_nan(), "modf(NaN) is NaN in both parts");
    }

    // ---- Bessel functions ----

    /// The rational approximations for j0, j1, and y0 come from Numerical
    /// Recipes and carry about eight significant digits.
    const BESSEL_TOL: f64 = 1e-7;

    #[test]
    fn j0_known_values() {
        // Both branches of the approximation are covered: |x| < 8 and |x| >= 8.
        let cases = [
            (1.0, 0.765_197_686_557_966_6),
            (2.0, 0.223_890_779_141_235_7),
            (5.0, -0.177_596_771_314_338_3),
            (8.0, 0.171_650_807_137_553_9),
            (10.0, -0.245_935_764_451_348_3),
            (20.0, 0.167_024_664_340_501_3),
        ];
        for (x, want) in cases {
            assert_close(call_unary_real(math_j0, x), want, BESSEL_TOL, "j0");
            // J0 is even.
            assert_eq!(
                call_unary_real(math_j0, -x),
                call_unary_real(math_j0, x),
                "j0(-x) = j0(x)"
            );
        }
        assert_close(call_unary_real(math_j0, 0.0), 1.0, 3e-9, "j0(0)");
    }

    #[test]
    fn j1_known_values() {
        let cases = [
            (1.0, 0.440_050_585_744_933_5),
            (2.0, 0.576_724_807_756_873_4),
            (5.0, -0.327_579_137_591_465_2),
            (8.0, 0.234_636_346_853_914_6),
            (10.0, 0.043_472_746_168_861_4),
        ];
        for (x, want) in cases {
            assert_close(call_unary_real(math_j1, x), want, BESSEL_TOL, "j1");
            // J1 is odd.
            assert_eq!(
                call_unary_real(math_j1, -x),
                -call_unary_real(math_j1, x),
                "j1(-x) = -j1(x)"
            );
        }
        assert_eq!(call_unary_real(math_j1, 0.0), 0.0);
    }

    #[test]
    fn y0_known_values() {
        let cases = [
            (1.0, 0.088_256_964_215_677_0),
            (2.0, 0.510_375_672_649_745_1),
            (5.0, -0.308_517_625_249_033_8),
            (8.0, 0.223_521_489_387_566_2),
            (10.0, 0.055_671_167_283_599_4),
        ];
        for (x, want) in cases {
            assert_close(call_unary_real(math_y0, x), want, BESSEL_TOL, "y0");
        }
    }

    #[test]
    fn y1_known_values() {
        let cases = [
            (0.5, -1.471_472_392_670_243),
            (1.0, -0.781_212_821_300_288_7),
            (2.0, -0.107_032_431_540_937_5),
            (4.0, 0.397_925_710_557_1),
            (5.0, 0.147_863_143_391_226_8),
            (6.0, -0.175_010_344_300_398_3),
            (7.0, -0.302_667_237_024_184_9),
            (8.0, -0.158_060_461_731_247_5),
            (10.0, 0.249_015_424_206_953_9),
        ];
        for (x, want) in cases {
            assert_close(call_unary_real(math_y1, x), want, 1e-7, "y1");
        }
    }

    /// `external/inferno-os/libmath/fdlibm/e_j0.c` and `e_j1.c` fix the edge
    /// cases: y0 and y1 are -inf at zero, NaN for a negative argument, and all
    /// four functions are zero at +inf.
    #[test]
    fn bessel_edge_cases() {
        assert_eq!(call_unary_real(math_y0, 0.0), f64::NEG_INFINITY);
        assert_eq!(call_unary_real(math_y1, 0.0), f64::NEG_INFINITY);
        assert!(call_unary_real(math_y0, -1.0).is_nan(), "y0(-1) is NaN");
        assert!(call_unary_real(math_y1, -1.0).is_nan(), "y1(-1) is NaN");
        assert_eq!(call_unary_real(math_j0, f64::INFINITY), 0.0);
        assert_eq!(call_unary_real(math_j1, f64::INFINITY), 0.0);
        assert_eq!(call_unary_real(math_j0, f64::NEG_INFINITY), 0.0);
        assert_eq!(call_unary_real(math_y0, f64::INFINITY), 0.0);
        assert_eq!(call_unary_real(math_y1, f64::INFINITY), 0.0);
        assert!(call_unary_real(math_j0, f64::NAN).is_nan());
        assert!(call_unary_real(math_j1, f64::NAN).is_nan());
        assert!(call_unary_real(math_y0, f64::NAN).is_nan());
        assert!(call_unary_real(math_y1, f64::NAN).is_nan());
    }

    #[test]
    fn jn_matches_j0_and_j1_for_low_orders() {
        for x in [0.5, 1.0, 5.0, 10.0] {
            assert_eq!(call_int_real(math_jn, 0, x), call_unary_real(math_j0, x));
            assert_eq!(call_int_real(math_jn, 1, x), call_unary_real(math_j1, x));
        }
    }

    /// The recurrence J(n+1) = 2n/x*J(n) - J(n-1) loses every significant digit
    /// when it runs upwards past n = x, so `__ieee754_jn` in
    /// `external/inferno-os/libmath/fdlibm/e_jn.c` only uses it while n <= x.
    #[test]
    fn jn_known_values() {
        let cases = [
            (2, 1.0, 0.114_903_484_931_900_48),
            (3, 1.0, 0.019_563_353_982_668_407),
            (10, 1.0, 2.630_615_123_687_453_2e-10),
            (2, 10.0, 0.254_630_313_685_120_6),
            (5, 10.0, -0.234_061_528_186_793_6),
            (20, 10.0, 1.151_336_924_781_34e-5),
            (7, 3.0, 0.002_547_294_451_816_536_5),
        ];
        for (n, x, want) in cases {
            assert_close(
                call_int_real(math_jn, n, x),
                want,
                1e-6,
                &format!("jn({n},{x})"),
            );
        }
    }

    /// J(-n, x) = (-1)^n * J(n, x) and J(n, -x) = (-1)^n * J(n, x).
    #[test]
    fn jn_symmetries_and_edges() {
        for x in [1.0, 6.0] {
            assert_close(
                call_int_real(math_jn, -2, x),
                call_int_real(math_jn, 2, x),
                1e-12,
                "jn(-2,x) = jn(2,x)",
            );
            assert_close(
                call_int_real(math_jn, -3, x),
                -call_int_real(math_jn, 3, x),
                1e-12,
                "jn(-3,x) = -jn(3,x)",
            );
            assert_close(
                call_int_real(math_jn, 3, -x),
                -call_int_real(math_jn, 3, x),
                1e-12,
                "jn(3,-x) = -jn(3,x)",
            );
            assert_close(
                call_int_real(math_jn, 2, -x),
                call_int_real(math_jn, 2, x),
                1e-12,
                "jn(2,-x) = jn(2,x)",
            );
        }
        // An order far above x underflows to zero instead of driving the
        // recurrence through millions of steps.
        assert_eq!(call_int_real(math_jn, 400, 1.0), 0.0);
        assert_eq!(call_int_real(math_jn, 1_000_000, 2.0), 0.0);
        // J(n, 0) is 0 for every n >= 1, and J(n, inf) is 0.
        assert_eq!(call_int_real(math_jn, 2, 0.0), 0.0);
        assert_eq!(call_int_real(math_jn, 5, 0.0), 0.0);
        assert_eq!(call_int_real(math_jn, 2, f64::INFINITY), 0.0);
        assert!(call_int_real(math_jn, 2, f64::NAN).is_nan());
    }

    #[test]
    fn yn_known_values() {
        assert_close(
            call_int_real(math_yn, 2, 1.0),
            -1.650_682_606_816_254_8,
            1e-6,
            "yn(2,1)",
        );
        assert_close(
            call_int_real(math_yn, 3, 1.0),
            -5.821_517_605_964_729,
            1e-6,
            "yn(3,1)",
        );
        assert_close(
            call_int_real(math_yn, 2, 10.0),
            -0.005_868_082_442_208_6,
            1e-5,
            "yn(2,10)",
        );
        for x in [1.0, 10.0] {
            assert_eq!(call_int_real(math_yn, 0, x), call_unary_real(math_y0, x));
            assert_eq!(call_int_real(math_yn, 1, x), call_unary_real(math_y1, x));
        }
    }

    /// `__ieee754_yn` in `external/inferno-os/libmath/fdlibm/e_jn.c` returns
    /// -inf at zero, NaN below zero, and zero at +inf, and it applies
    /// Y(-n, x) = (-1)^n * Y(n, x).
    #[test]
    fn yn_edges_and_negative_order() {
        assert_eq!(call_int_real(math_yn, 2, 0.0), f64::NEG_INFINITY);
        assert_eq!(call_int_real(math_yn, 0, 0.0), f64::NEG_INFINITY);
        assert!(call_int_real(math_yn, 2, -1.0).is_nan(), "yn(2,-1) is NaN");
        assert_eq!(call_int_real(math_yn, 2, f64::INFINITY), 0.0);
        assert!(call_int_real(math_yn, 2, f64::NAN).is_nan());
        assert_close(
            call_int_real(math_yn, -1, 1.0),
            -call_unary_real(math_y1, 1.0),
            1e-12,
            "yn(-1,1) = -y1(1)",
        );
        assert_close(
            call_int_real(math_yn, -2, 1.0),
            call_int_real(math_yn, 2, 1.0),
            1e-12,
            "yn(-2,1) = yn(2,1)",
        );
        assert_close(
            call_int_real(math_yn, -3, 1.0),
            -call_int_real(math_yn, 3, 1.0),
            1e-12,
            "yn(-3,1) = -yn(3,1)",
        );
    }

    // ---- lgamma ----

    #[test]
    fn lgamma_known_values() {
        let cases = [
            (1.0, 0.0),
            (2.0, 0.0),
            (0.5, 0.572_364_942_924_700_1),
            (0.1, 2.252_712_651_734_206),
            (5.0, 3.178_053_830_347_945_8),
            (100.0, 359.134_205_369_575_4),
        ];
        for (x, want) in cases {
            let got = call_unary_real(math_lgamma, x);
            assert!(
                (got - want).abs() <= 1e-12 * want.abs().max(1.0),
                "lgamma({x}): want {want}, got {got}"
            );
        }
    }

    /// lgamma is log|Gamma(x)|, which is finite for a negative non-integer.
    /// `__ieee754_lgamma_r` returns the log of the magnitude and reports the
    /// sign separately, so the reflection formula needs |sin(pi*x)|.
    #[test]
    fn lgamma_negative_arguments() {
        // Gamma(-0.5) = -2*sqrt(pi), so lgamma(-0.5) = ln(2*sqrt(pi)).
        let cases = [
            (-0.5, 1.265_512_123_484_645_4),
            (-1.5, 0.860_047_015_376_481),
            (-2.5, -0.056_243_716_497_674_05),
        ];
        for (x, want) in cases {
            let got = call_unary_real(math_lgamma, x);
            assert!(
                (got - want).abs() <= 1e-12 * want.abs().max(1.0),
                "lgamma({x}): want {want}, got {got}"
            );
        }
    }

    /// Gamma has a pole at every non-positive integer, so lgamma is +inf there,
    /// and lgamma of either infinity is +inf.
    #[test]
    fn lgamma_poles_and_infinities() {
        assert_eq!(call_unary_real(math_lgamma, 0.0), f64::INFINITY);
        assert_eq!(call_unary_real(math_lgamma, -0.0), f64::INFINITY);
        assert_eq!(call_unary_real(math_lgamma, -1.0), f64::INFINITY);
        assert_eq!(call_unary_real(math_lgamma, -2.0), f64::INFINITY);
        assert_eq!(call_unary_real(math_lgamma, -20.0), f64::INFINITY);
        assert_eq!(call_unary_real(math_lgamma, f64::INFINITY), f64::INFINITY);
        assert_eq!(
            call_unary_real(math_lgamma, f64::NEG_INFINITY),
            f64::INFINITY
        );
        assert!(call_unary_real(math_lgamma, f64::NAN).is_nan());
    }

    // ---- gemm ----

    /// A module whose entry frame is big enough for gemm's 96-byte layout.
    fn gemm_module() -> Module {
        let mut module = test_module();
        module.types[0].size = 128;
        module
    }

    fn alloc_real_array(vm: &mut VmState<'_>, vals: &[f64]) -> u32 {
        let mut data = vec![0u8; vals.len() * 8];
        for (i, &v) in vals.iter().enumerate() {
            memory::write_real(&mut data, i * 8, v);
        }
        vm.heap.alloc(
            0,
            crate::heap::HeapData::Array {
                elem_type: 0,
                elem_size: 8,
                data,
                length: vals.len(),
            },
        )
    }

    /// Fill the gemm frame. Dimensions are words so they can be made negative.
    #[allow(clippy::too_many_arguments)]
    fn setup_gemm(
        vm: &mut VmState<'_>,
        m: i32,
        n: i32,
        k: i32,
        a_id: u32,
        lda: i32,
        b_id: u32,
        ldb: i32,
        c_id: u32,
        ldc: i32,
    ) {
        let base = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF, b'N' as i32);
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF + 4, b'N' as i32);
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF + 8, m);
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF + 12, n);
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF + 16, k);
        memory::write_real(&mut vm.frames.data, base + 56, 1.0); // alpha
        memory::write_word(&mut vm.frames.data, base + 64, a_id as i32);
        memory::write_word(&mut vm.frames.data, base + 68, lda);
        memory::write_word(&mut vm.frames.data, base + 72, b_id as i32);
        memory::write_word(&mut vm.frames.data, base + 76, ldb);
        memory::write_real(&mut vm.frames.data, base + 80, 1.0); // beta
        memory::write_word(&mut vm.frames.data, base + 88, c_id as i32);
        memory::write_word(&mut vm.frames.data, base + 92, ldc);
    }

    #[test]
    fn gemm_multiplies_one_by_one() {
        let module = gemm_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let a = alloc_real_array(&mut vm, &[2.0]);
        let b = alloc_real_array(&mut vm, &[3.0]);
        let c = alloc_real_array(&mut vm, &[1.0]);
        setup_gemm(&mut vm, 1, 1, 1, a, 1, b, 1, c, 1);

        math_gemm(&mut vm).expect("gemm should succeed");

        let out = vm.heap.array_read(c, 0, 8).expect("c should be readable");
        assert_eq!(memory::read_real(&out, 0), 7.0, "1*2*3 + 1*1 = 7");
    }

    #[test]
    fn gemm_rejects_negative_dimensions() {
        let module = gemm_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let a = alloc_real_array(&mut vm, &[2.0]);
        let b = alloc_real_array(&mut vm, &[3.0]);
        let c = alloc_real_array(&mut vm, &[1.0]);
        setup_gemm(&mut vm, -1, 1, 1, a, 1, b, 1, c, 1);

        let err = math_gemm(&mut vm).expect_err("negative m must be rejected");
        assert!(
            err.to_string().contains("gemm"),
            "expected a gemm parameter error, got: {err}"
        );
    }

    #[test]
    fn gemm_rejects_dimensions_larger_than_the_arrays() {
        let module = gemm_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let a = alloc_real_array(&mut vm, &[2.0]);
        let b = alloc_real_array(&mut vm, &[3.0]);
        let c = alloc_real_array(&mut vm, &[1.0]);
        // ldc * (n-1) + m would need a 2-billion element C buffer.
        setup_gemm(&mut vm, 1, i32::MAX, 1, a, 1, b, 1, c, 1);

        let err = math_gemm(&mut vm).expect_err("oversized C must be rejected");
        assert!(
            err.to_string().contains("gemm"),
            "expected a gemm parameter error, got: {err}"
        );
    }

    /// Fill the gemm frame with every parameter under the caller's control.
    #[allow(clippy::too_many_arguments)]
    fn setup_gemm_full(
        vm: &mut VmState<'_>,
        transa: u8,
        transb: u8,
        m: i32,
        n: i32,
        k: i32,
        alpha: f64,
        a_id: u32,
        lda: i32,
        b_id: u32,
        ldb: i32,
        beta: f64,
        c_id: u32,
        ldc: i32,
    ) {
        let base = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF, transa as i32);
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF + 4, transb as i32);
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF + 8, m);
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF + 12, n);
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF + 16, k);
        memory::write_real(&mut vm.frames.data, base + 56, alpha);
        memory::write_word(&mut vm.frames.data, base + 64, a_id as i32);
        memory::write_word(&mut vm.frames.data, base + 68, lda);
        memory::write_word(&mut vm.frames.data, base + 72, b_id as i32);
        memory::write_word(&mut vm.frames.data, base + 76, ldb);
        memory::write_real(&mut vm.frames.data, base + 80, beta);
        memory::write_word(&mut vm.frames.data, base + 88, c_id as i32);
        memory::write_word(&mut vm.frames.data, base + 92, ldc);
    }

    /// Read `n` reals out of a heap array.
    fn heap_reals(vm: &VmState<'_>, id: u32, n: usize) -> Vec<f64> {
        let data = vm
            .heap
            .array_read(id, 0, n * 8)
            .expect("array should be readable");
        (0..n).map(|i| memory::read_real(&data, i * 8)).collect()
    }

    /// Run one 2x2 gemm and return C. Matrices are stored in column major
    /// order, as `external/inferno-os/man/2/math-linalg` requires.
    fn gemm_2x2(transa: u8, transb: u8, alpha: f64, beta: f64, c_init: &[f64]) -> Vec<f64> {
        let module = gemm_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let a = alloc_real_array(&mut vm, &[1.0, 3.0, 2.0, 4.0]);
        let b = alloc_real_array(&mut vm, &[5.0, 7.0, 6.0, 8.0]);
        let c = alloc_real_array(&mut vm, c_init);
        setup_gemm_full(
            &mut vm, transa, transb, 2, 2, 2, alpha, a, 2, b, 2, beta, c, 2,
        );
        math_gemm(&mut vm).expect("gemm should succeed");
        heap_reals(&vm, c, 4)
    }

    /// A = [[1,2],[3,4]] and B = [[5,6],[7,8]], so A*B = [[19,22],[43,50]].
    #[test]
    fn gemm_multiplies_two_by_two() {
        let zero = [0.0; 4];
        assert_eq!(
            gemm_2x2(b'N', b'N', 1.0, 0.0, &zero),
            vec![19.0, 43.0, 22.0, 50.0]
        );
        // A' * B = [[26,30],[38,44]].
        assert_eq!(
            gemm_2x2(b'T', b'N', 1.0, 0.0, &zero),
            vec![26.0, 38.0, 30.0, 44.0]
        );
        // A * B' = [[17,23],[39,53]].
        assert_eq!(
            gemm_2x2(b'N', b'T', 1.0, 0.0, &zero),
            vec![17.0, 39.0, 23.0, 53.0]
        );
        // A' * B' = [[23,31],[34,46]].
        assert_eq!(
            gemm_2x2(b'T', b'T', 1.0, 0.0, &zero),
            vec![23.0, 34.0, 31.0, 46.0]
        );
        // 'C' means the same as 'T' for a real matrix.
        assert_eq!(
            gemm_2x2(b'C', b'C', 1.0, 0.0, &zero),
            gemm_2x2(b'T', b'T', 1.0, 0.0, &zero)
        );
    }

    #[test]
    fn gemm_applies_alpha_and_beta() {
        let ones = [1.0; 4];
        // 2*A*B + 3*C with C all ones.
        assert_eq!(
            gemm_2x2(b'N', b'N', 2.0, 3.0, &ones),
            vec![41.0, 89.0, 47.0, 103.0]
        );
        // Both transposed paths honour beta as well.
        assert_eq!(
            gemm_2x2(b'T', b'N', 1.0, 2.0, &ones),
            vec![28.0, 40.0, 32.0, 46.0]
        );
        assert_eq!(
            gemm_2x2(b'N', b'T', 1.0, 2.0, &ones),
            vec![19.0, 41.0, 25.0, 55.0]
        );
        assert_eq!(
            gemm_2x2(b'T', b'T', 1.0, 2.0, &ones),
            vec![25.0, 36.0, 33.0, 48.0]
        );
        // alpha = 0 with beta = 0 clears C, and with beta = 2 it scales C.
        assert_eq!(gemm_2x2(b'N', b'N', 0.0, 0.0, &ones), vec![0.0; 4]);
        assert_eq!(gemm_2x2(b'N', b'N', 0.0, 2.0, &ones), vec![2.0; 4]);
        // alpha = 0 with beta = 1 leaves C alone.
        assert_eq!(gemm_2x2(b'N', b'N', 0.0, 1.0, &ones), vec![1.0; 4]);
    }

    /// A nil A computes C := alpha*op(B) + beta*C, which is how the BLAS-style
    /// entry point doubles as a matrix scale and add.
    #[test]
    fn gemm_with_nil_a_adds_scaled_b() {
        let module = gemm_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let b = alloc_real_array(&mut vm, &[1.0, 2.0, 3.0, 4.0]);
        let c = alloc_real_array(&mut vm, &[10.0, 10.0, 10.0, 10.0]);
        setup_gemm_full(&mut vm, b'N', b'N', 2, 2, 2, 2.0, 0, 2, b, 2, 1.0, c, 2);
        math_gemm(&mut vm).expect("gemm should succeed");
        assert_eq!(heap_reals(&vm, c, 4), vec![12.0, 14.0, 16.0, 18.0]);
    }

    /// A nil A with a transposed B walks B the other way round.
    #[test]
    fn gemm_with_nil_a_and_transposed_b() {
        let module = gemm_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let b = alloc_real_array(&mut vm, &[1.0, 2.0, 3.0, 4.0]);
        let c = alloc_real_array(&mut vm, &[0.0; 4]);
        setup_gemm_full(&mut vm, b'N', b'T', 2, 2, 2, 1.0, 0, 2, b, 2, 0.0, c, 2);
        math_gemm(&mut vm).expect("gemm should succeed");
        // B is [[1,3],[2,4]] in column major order, so B' is [[1,2],[3,4]].
        assert_eq!(heap_reals(&vm, c, 4), vec![1.0, 3.0, 2.0, 4.0]);
    }

    /// A nil B reads as zero rather than faulting when A is nil too.
    #[test]
    fn gemm_with_nil_a_and_nil_b_scales_c() {
        let module = gemm_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let c = alloc_real_array(&mut vm, &[5.0, 6.0]);
        setup_gemm_full(&mut vm, b'N', b'N', 2, 1, 1, 1.0, 0, 2, 0, 2, 2.0, c, 2);
        math_gemm(&mut vm).expect("gemm should succeed");
        assert_eq!(heap_reals(&vm, c, 2), vec![10.0, 12.0]);
    }

    /// With k = 0 the product is empty, so C := beta*C.
    #[test]
    fn gemm_with_zero_k_scales_c() {
        let module = gemm_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let a = alloc_real_array(&mut vm, &[1.0, 1.0]);
        let b = alloc_real_array(&mut vm, &[1.0, 1.0]);
        let c = alloc_real_array(&mut vm, &[3.0, 4.0]);
        setup_gemm_full(&mut vm, b'N', b'N', 2, 1, 0, 1.0, a, 2, b, 2, 2.0, c, 2);
        math_gemm(&mut vm).expect("gemm should succeed");
        assert_eq!(heap_reals(&vm, c, 2), vec![6.0, 8.0]);
    }

    /// An empty active area is not an error and must leave C untouched.
    #[test]
    fn gemm_with_zero_dimensions_is_a_no_op() {
        for (m, n) in [(0, 2), (2, 0)] {
            let module = gemm_module();
            let mut vm = VmState::new(&module).expect("vm should initialize");
            let c = alloc_real_array(&mut vm, &[7.0]);
            // The arrays are far too small for the declared dimensions, which
            // proves the early return happens before any bound is used.
            setup_gemm_full(&mut vm, b'N', b'N', m, n, 2, 1.0, 0, 99, 0, 99, 1.0, c, 99);
            math_gemm(&mut vm).expect("an empty gemm should succeed");
            assert_eq!(heap_reals(&vm, c, 1), vec![7.0]);
        }
    }

    #[test]
    fn gemm_rejects_bad_shapes() {
        /// One rejected gemm call: a description, the dimensions m, n, and k,
        /// the leading dimensions lda, ldb, and ldc, and the length of A.
        struct BadShape {
            what: &'static str,
            m: i32,
            n: i32,
            k: i32,
            lda: i32,
            ldb: i32,
            ldc: i32,
            a_len: usize,
        }
        let case = |what, m, n, k, lda, ldb, ldc, a_len| BadShape {
            what,
            m,
            n,
            k,
            lda,
            ldb,
            ldc,
            a_len,
        };
        let cases = [
            case("negative n", 1, -1, 1, 1, 1, 1, 1),
            case("negative k", 1, 1, -1, 1, 1, 1, 1),
            case("negative lda", 1, 1, 1, -1, 1, 1, 1),
            case("negative ldb", 1, 1, 1, 1, -1, 1, 1),
            case("negative ldc", 1, 1, 1, 1, 1, -1, 1),
            case("ldc below m", 2, 2, 2, 2, 2, 1, 4),
            case("A too short", 2, 2, 2, 2, 2, 2, 3),
            case("lda below rows", 2, 2, 2, 1, 2, 2, 4),
        ];
        for BadShape {
            what,
            m,
            n,
            k,
            lda,
            ldb,
            ldc,
            a_len,
        } in cases
        {
            let module = gemm_module();
            let mut vm = VmState::new(&module).expect("vm should initialize");
            let a = alloc_real_array(&mut vm, &vec![1.0; a_len]);
            let b = alloc_real_array(&mut vm, &[1.0, 1.0, 1.0, 1.0]);
            let c = alloc_real_array(&mut vm, &[1.0, 1.0, 1.0, 1.0]);
            setup_gemm_full(
                &mut vm, b'N', b'N', m, n, k, 1.0, a, lda, b, ldb, 1.0, c, ldc,
            );
            let err = match math_gemm(&mut vm) {
                Err(e) => e,
                Ok(()) => panic!("{what} must be rejected"),
            };
            assert!(err.to_string().contains("gemm"), "{what}: got {err}");
        }
    }

    /// A short B is rejected before any element of it is read.
    #[test]
    fn gemm_rejects_a_short_b() {
        let module = gemm_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let a = alloc_real_array(&mut vm, &[1.0, 1.0, 1.0, 1.0]);
        let b = alloc_real_array(&mut vm, &[1.0, 1.0, 1.0]);
        let c = alloc_real_array(&mut vm, &[1.0, 1.0, 1.0, 1.0]);
        setup_gemm_full(&mut vm, b'N', b'N', 2, 2, 2, 1.0, a, 2, b, 2, 1.0, c, 2);
        let err = math_gemm(&mut vm).expect_err("a short B must be rejected");
        assert!(err.to_string().contains("gemm"), "got: {err}");
    }

    /// `Math_gemm` in `external/inferno-os/libinterp/math.c` accepts only 'N',
    /// 'T', and 'C' for the transpose flags.
    #[test]
    fn gemm_rejects_an_unknown_transpose_flag() {
        let module = gemm_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let a = alloc_real_array(&mut vm, &[1.0]);
        let b = alloc_real_array(&mut vm, &[1.0]);
        let c = alloc_real_array(&mut vm, &[1.0]);
        setup_gemm_full(&mut vm, b'X', b'N', 1, 1, 1, 1.0, a, 1, b, 1, 1.0, c, 1);
        let err = math_gemm(&mut vm).expect_err("transa = 'X' must be rejected");
        assert!(err.to_string().contains("gemm"), "got: {err}");
    }

    // ---- dot, norm1, norm2, iamax, and sort ----

    /// Allocate an array of ints on the heap.
    fn alloc_int_array(vm: &mut VmState<'_>, vals: &[i32]) -> u32 {
        let mut data = vec![0u8; vals.len() * 4];
        for (i, &v) in vals.iter().enumerate() {
            memory::write_word(&mut data, i * 4, v);
        }
        vm.heap.alloc(
            0,
            crate::heap::HeapData::Array {
                elem_type: 0,
                elem_size: 4,
                data,
                length: vals.len(),
            },
        )
    }

    /// Allocate an array of bytes on the heap.
    fn alloc_byte_array(vm: &mut VmState<'_>, bytes: &[u8]) -> u32 {
        vm.heap.alloc(
            0,
            crate::heap::HeapData::Array {
                elem_type: 0,
                elem_size: 1,
                data: bytes.to_vec(),
                length: bytes.len(),
            },
        )
    }

    /// Read `n` ints out of a heap array.
    fn heap_ints(vm: &VmState<'_>, id: u32, n: usize) -> Vec<i32> {
        let data = vm
            .heap
            .array_read(id, 0, n * 4)
            .expect("array should be readable");
        (0..n).map(|i| memory::read_word(&data, i * 4)).collect()
    }

    /// Call a function whose only argument is one array of reals.
    fn call_with_one_array(
        f: fn(&mut VmState<'_>) -> Result<(), ExecError>,
        vals: &[f64],
    ) -> (f64, i32) {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let id = alloc_real_array(&mut vm, vals);
        let base = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF, id as i32);
        f(&mut vm).expect("the function should succeed");
        (
            memory::read_real(&vm.frames.data, base + RET_OFF),
            memory::read_word(&vm.frames.data, base + RET_OFF),
        )
    }

    /// dot(x, y) is sum(x[i]*y[i]) over the whole array, whose length comes
    /// from the array itself (`f->x->len` in `Math_dot`).
    #[test]
    fn dot_known_answer() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let x = alloc_real_array(&mut vm, &[1.0, 2.0, 3.0]);
        let y = alloc_real_array(&mut vm, &[4.0, 5.0, 6.0]);
        let base = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF, x as i32);
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF + 4, y as i32);
        math_dot(&mut vm).expect("dot should succeed");
        // 1*4 + 2*5 + 3*6 = 32.
        assert_eq!(memory::read_real(&vm.frames.data, base + RET_OFF), 32.0);
    }

    #[test]
    fn dot_of_empty_and_nil_arrays_is_zero() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let x = alloc_real_array(&mut vm, &[]);
        let y = alloc_real_array(&mut vm, &[]);
        let base = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF, x as i32);
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF + 4, y as i32);
        math_dot(&mut vm).expect("dot of empty arrays should succeed");
        assert_eq!(memory::read_real(&vm.frames.data, base + RET_OFF), 0.0);

        let mut vm = VmState::new(&module).expect("vm should initialize");
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF, 0);
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF + 4, 0);
        math_dot(&mut vm).expect("dot of nil arrays should succeed");
        assert_eq!(memory::read_real(&vm.frames.data, base + RET_OFF), 0.0);
    }

    /// `Math_dot` raises an error when the two arrays have different lengths.
    #[test]
    fn dot_rejects_mismatched_lengths() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let x = alloc_real_array(&mut vm, &[1.0, 2.0]);
        let y = alloc_real_array(&mut vm, &[1.0]);
        let base = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF, x as i32);
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF + 4, y as i32);
        let err = math_dot(&mut vm).expect_err("mismatched lengths must be rejected");
        assert!(err.to_string().contains("dot"), "got: {err}");
    }

    #[test]
    fn norm1_and_norm2_known_answers() {
        // norm1 is the sum of the magnitudes and norm2 is sqrt(dot(x,x)).
        assert_eq!(call_with_one_array(math_norm1, &[1.0, -2.0, 3.0]).0, 6.0);
        assert_eq!(call_with_one_array(math_norm2, &[3.0, 4.0]).0, 5.0);
        assert_eq!(call_with_one_array(math_norm1, &[-0.0]).0, 0.0);
        assert_eq!(call_with_one_array(math_norm1, &[]).0, 0.0);
        assert_eq!(call_with_one_array(math_norm2, &[]).0, 0.0);
        // norm2 of a single element is its magnitude.
        assert_eq!(call_with_one_array(math_norm2, &[-7.0]).0, 7.0);
    }

    /// iamax returns an index i such that |x[i]| is maximal. `iamax` in
    /// `external/inferno-os/libmath/blas.c` replaces the running maximum only
    /// on a strict increase, so the first such index wins and a NaN is skipped.
    #[test]
    fn iamax_returns_the_first_largest_magnitude() {
        assert_eq!(call_with_one_array(math_iamax, &[1.0, -3.0, 2.0]).1, 1);
        assert_eq!(call_with_one_array(math_iamax, &[1.0, -1.0]).1, 0);
        assert_eq!(call_with_one_array(math_iamax, &[5.0, f64::NAN, 1.0]).1, 0);
        assert_eq!(call_with_one_array(math_iamax, &[-9.0]).1, 0);
        assert_eq!(call_with_one_array(math_iamax, &[]).1, 0);
        assert_eq!(
            call_with_one_array(math_iamax, &[1.0, f64::INFINITY, 2.0]).1,
            1
        );
    }

    /// `sort(x, pi)` updates the permutation pi so that x[pi[i]] <= x[pi[i+1]]
    /// and leaves x unchanged (`external/inferno-os/man/2/math-linalg`).
    #[test]
    fn sort_permutes_the_index_array() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let x = alloc_real_array(&mut vm, &[3.0, 1.0, 2.0]);
        let p = alloc_int_array(&mut vm, &[0, 1, 2]);
        let base = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF, x as i32);
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF + 4, p as i32);
        math_sort(&mut vm).expect("sort should succeed");
        assert_eq!(heap_ints(&vm, p, 3), vec![1, 2, 0], "pi orders x");
        assert_eq!(
            heap_reals(&vm, x, 3),
            vec![3.0, 1.0, 2.0],
            "x must not move"
        );
    }

    /// A permutation may address a subset of x, in any starting order.
    #[test]
    fn sort_handles_a_partial_permutation() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let x = alloc_real_array(&mut vm, &[5.0, -1.0, 4.0, 0.0]);
        let p = alloc_int_array(&mut vm, &[2, 0, 1]);
        let base = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF, x as i32);
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF + 4, p as i32);
        math_sort(&mut vm).expect("sort should succeed");
        assert_eq!(heap_ints(&vm, p, 3), vec![1, 2, 0]);
    }

    /// `Math_sort` checks that every permutation entry is in [0, len(x)-1].
    #[test]
    fn sort_rejects_an_out_of_range_permutation() {
        for bad in [3, -1] {
            let module = test_module();
            let mut vm = VmState::new(&module).expect("vm should initialize");
            let x = alloc_real_array(&mut vm, &[3.0, 1.0, 2.0]);
            let p = alloc_int_array(&mut vm, &[0, bad, 2]);
            let base = vm.frames.current_data_offset();
            memory::write_word(&mut vm.frames.data, base + ARG1_OFF, x as i32);
            memory::write_word(&mut vm.frames.data, base + ARG1_OFF + 4, p as i32);
            let err = math_sort(&mut vm).expect_err("an invalid index must be rejected");
            assert!(err.to_string().contains("sort"), "got: {err}");
            assert_eq!(heap_ints(&vm, p, 3), vec![0, bad, 2], "pi must not move");
        }
    }

    /// A NaN in x has no order, so it must not upset the sort.
    #[test]
    fn sort_tolerates_a_nan() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let x = alloc_real_array(&mut vm, &[2.0, f64::NAN, 1.0]);
        let p = alloc_int_array(&mut vm, &[0, 1, 2]);
        let base = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF, x as i32);
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF + 4, p as i32);
        math_sort(&mut vm).expect("sort should succeed");
        let p = heap_ints(&vm, p, 3);
        assert_eq!(p.len(), 3);
        // The two ordered elements keep their relative order.
        let pos = |v: i32| p.iter().position(|&e| e == v).unwrap_or(9);
        assert!(pos(2) < pos(0), "x[2] = 1 sorts before x[0] = 2, got {p:?}");
    }

    #[test]
    fn sort_with_an_empty_permutation_is_a_no_op() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let x = alloc_real_array(&mut vm, &[3.0, 1.0]);
        let p = alloc_int_array(&mut vm, &[]);
        let base = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF, x as i32);
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF + 4, p as i32);
        math_sort(&mut vm).expect("sort should succeed");
        assert_eq!(heap_reals(&vm, x, 2), vec![3.0, 1.0]);
    }

    // ---- Byte order conversion ----

    /// Set up a call whose arguments are a byte array and a value array, the
    /// shape every import and export function has in `module/math.m`.
    fn call_with_two_arrays(
        f: fn(&mut VmState<'_>) -> Result<(), ExecError>,
        b_id: u32,
        x_id: u32,
        vm: &mut VmState<'_>,
    ) {
        let base = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF, b_id as i32);
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF + 4, x_id as i32);
        f(vm).expect("the conversion should succeed");
    }

    /// Values are big endian, most significant byte first
    /// (`external/inferno-os/man/2/math-export`).
    #[test]
    fn export_int_writes_big_endian_words() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let x = alloc_int_array(&mut vm, &[1, -1]);
        let b = alloc_byte_array(&mut vm, &[0xaa; 8]);
        call_with_two_arrays(math_export_int, b, x, &mut vm);
        let bytes = vm.heap.array_read(b, 0, 8).expect("b should be readable");
        assert_eq!(bytes, vec![0x00, 0x00, 0x00, 0x01, 0xff, 0xff, 0xff, 0xff]);
    }

    #[test]
    fn import_int_reads_big_endian_words() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let b = alloc_byte_array(&mut vm, &[0x00, 0x00, 0x00, 0x07, 0xff, 0xff, 0xff, 0xff]);
        let x = alloc_int_array(&mut vm, &[0, 0]);
        call_with_two_arrays(math_import_int, b, x, &mut vm);
        assert_eq!(heap_ints(&vm, x, 2), vec![7, -1]);
    }

    #[test]
    fn export_real_writes_ieee_doubles() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let x = alloc_real_array(&mut vm, &[1.0, -2.0]);
        let b = alloc_byte_array(&mut vm, &[0xaa; 16]);
        call_with_two_arrays(math_export_real, b, x, &mut vm);
        let bytes = vm.heap.array_read(b, 0, 16).expect("b should be readable");
        assert_eq!(&bytes[..8], &1.0_f64.to_be_bytes());
        assert_eq!(&bytes[8..], &(-2.0_f64).to_be_bytes());
    }

    #[test]
    fn import_real_reads_ieee_doubles() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1.5_f64.to_be_bytes());
        bytes.extend_from_slice(&(-0.25_f64).to_be_bytes());
        let b = alloc_byte_array(&mut vm, &bytes);
        let x = alloc_real_array(&mut vm, &[0.0, 0.0]);
        call_with_two_arrays(math_import_real, b, x, &mut vm);
        assert_eq!(heap_reals(&vm, x, 2), vec![1.5, -0.25]);
    }

    #[test]
    fn export_real32_narrows_to_single_precision() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let x = alloc_real_array(&mut vm, &[1.0, 0.1]);
        let b = alloc_byte_array(&mut vm, &[0xaa; 8]);
        call_with_two_arrays(math_export_real32, b, x, &mut vm);
        let bytes = vm.heap.array_read(b, 0, 8).expect("b should be readable");
        assert_eq!(&bytes[..4], &1.0_f32.to_be_bytes());
        assert_eq!(&bytes[4..], &0.1_f32.to_be_bytes());
    }

    #[test]
    fn import_real32_widens_to_double_precision() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1.5_f32.to_be_bytes());
        bytes.extend_from_slice(&(-0.25_f32).to_be_bytes());
        let b = alloc_byte_array(&mut vm, &bytes);
        let x = alloc_real_array(&mut vm, &[0.0, 0.0]);
        call_with_two_arrays(math_import_real32, b, x, &mut vm);
        assert_eq!(heap_reals(&vm, x, 2), vec![1.5, -0.25]);
    }

    /// Each import undoes the matching export.
    #[test]
    fn export_and_import_round_trip() {
        let module = test_module();
        let vals = [0.0, 1.0, -2.5, 1e100];
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let src = alloc_real_array(&mut vm, &vals);
        let b = alloc_byte_array(&mut vm, &[0; 32]);
        let dst = alloc_real_array(&mut vm, &[9.0; 4]);
        call_with_two_arrays(math_export_real, b, src, &mut vm);
        call_with_two_arrays(math_import_real, b, dst, &mut vm);
        assert_eq!(heap_reals(&vm, dst, 4), vals.to_vec());

        let mut vm = VmState::new(&module).expect("vm should initialize");
        let src = alloc_int_array(&mut vm, &[0, 1, -1, i32::MIN]);
        let b = alloc_byte_array(&mut vm, &[0; 16]);
        let dst = alloc_int_array(&mut vm, &[9; 4]);
        call_with_two_arrays(math_export_int, b, src, &mut vm);
        call_with_two_arrays(math_import_int, b, dst, &mut vm);
        assert_eq!(heap_ints(&vm, dst, 4), vec![0, 1, -1, i32::MIN]);
    }

    /// A buffer shorter than the value array converts as many elements as it
    /// holds instead of writing out of bounds.
    #[test]
    fn export_stops_at_the_end_of_the_buffer() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let x = alloc_int_array(&mut vm, &[1, 2, 3]);
        let b = alloc_byte_array(&mut vm, &[0; 4]);
        call_with_two_arrays(math_export_int, b, x, &mut vm);
        let bytes = vm.heap.array_read(b, 0, 4).expect("b should be readable");
        assert_eq!(bytes, vec![0x00, 0x00, 0x00, 0x01]);
    }

    /// Older generated code passed the single value of export_real as a raw
    /// real at +40 instead of a one-element array, and that path still works.
    #[test]
    fn export_real_falls_back_to_a_raw_argument() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let b = alloc_byte_array(&mut vm, &[0; 8]);
        let base = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF, b as i32);
        memory::write_word(&mut vm.frames.data, base + ARG1_OFF + 4, 0);
        memory::write_real(&mut vm.frames.data, base + ARG2_OFF, -2.5);
        math_export_real(&mut vm).expect("export_real should succeed");
        let bytes = vm.heap.array_read(b, 0, 8).expect("b should be readable");
        assert_eq!(bytes, (-2.5_f64).to_be_bytes().to_vec());
    }

    /// A nil array on either side leaves the other side untouched.
    #[test]
    fn conversion_with_nil_arrays_is_a_no_op() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let b = alloc_byte_array(&mut vm, &[7; 8]);
        call_with_two_arrays(math_export_int, b, 0, &mut vm);
        assert_eq!(
            vm.heap.array_read(b, 0, 8).expect("b should be readable"),
            vec![7; 8]
        );
        let x = alloc_int_array(&mut vm, &[5, 5]);
        call_with_two_arrays(math_import_int, 0, x, &mut vm);
        assert_eq!(heap_ints(&vm, x, 2), vec![5, 5]);
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

    /// The table is registered in alphabetical order, matching Mathmodtab.
    #[test]
    fn create_math_module_is_sorted_and_complete() {
        let m = create_math_module();
        assert_eq!(m.funcs.len(), 66, "the table has 66 entries");
        let names: Vec<&str> = m.funcs.iter().map(|f| f.name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "the table must be in alphabetical order");
        // Frame sizes follow the argument lists: 32 bytes of header plus the
        // arguments, rounded up for alignment.
        let frame_of = |name: &str| {
            m.funcs
                .iter()
                .find(|f| f.name == name)
                .map(|f| f.frame_size)
                .unwrap_or(0)
        };
        assert_eq!(frame_of("sin"), 40, "one real argument");
        assert_eq!(frame_of("pow"), 48, "two real arguments");
        assert_eq!(frame_of("getFPcontrol"), 32, "no arguments");
        assert_eq!(frame_of("gemm"), 96, "thirteen arguments");
    }
}
