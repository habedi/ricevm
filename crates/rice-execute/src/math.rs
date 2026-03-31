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
            mf("getFPcontrol", 32, math_stub_int),
            mf("getFPstatus", 32, math_stub_int),
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

fn math_stub_int(vm: &mut VmState<'_>) -> Result<(), ExecError> {
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
    memory::write_real(&mut vm.frames.data, base + RET_OFF, result);
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
    memory::write_real(&mut vm.frames.data, base + RET_OFF, result);
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
    memory::write_real(&mut vm.frames.data, base + RET_OFF, int_part);
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
    memory::write_real(&mut vm.frames.data, base + RET_OFF, result);
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
    memory::write_real(&mut vm.frames.data, base + RET_OFF, result);
    Ok(())
}

fn math_norm1(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x_id = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let n = memory::read_word(&vm.frames.data, base + ARG1_OFF + 4) as usize;
    let x = read_real_array(vm, x_id, n);
    let result: f64 = x.iter().map(|v| v.abs()).sum();
    memory::write_real(&mut vm.frames.data, base + RET_OFF, result);
    Ok(())
}

fn math_norm2(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let x_id = memory::read_word(&vm.frames.data, base + ARG1_OFF) as u32;
    let n = memory::read_word(&vm.frames.data, base + ARG1_OFF + 4) as usize;
    let x = read_real_array(vm, x_id, n);
    let result: f64 = x.iter().map(|v| v * v).sum::<f64>().sqrt();
    memory::write_real(&mut vm.frames.data, base + RET_OFF, result);
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
        .max_by(|(_, a), (_, b)| a.abs().partial_cmp(&b.abs()).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i as i32)
        .unwrap_or(0);
    memory::write_word(&mut vm.frames.data, base, result);
    Ok(())
}

fn math_gemm(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // General matrix multiply C = alpha*A*B + beta*C
    // Complex frame layout; stub that does nothing.
    let _ = vm;
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
    let val = memory::read_real(&vm.frames.data, base + ARG1_OFF);
    let buf_id = memory::read_word(&vm.frames.data, base + ARG2_OFF) as u32;
    if let Some(obj) = vm.heap.get_mut(buf_id)
        && let crate::heap::HeapData::Array { data, .. } = &mut obj.data
        && data.len() >= 8
    {
        let bytes = val.to_be_bytes();
        data[..8].copy_from_slice(&bytes);
    }
    Ok(())
}

fn math_export_real32(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let base = vm.frames.current_data_offset();
    let val = memory::read_real(&vm.frames.data, base + ARG1_OFF) as f32;
    let buf_id = memory::read_word(&vm.frames.data, base + ARG2_OFF) as u32; // arg1 is real (8 bytes), so arg2 at ARG2_OFF
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
    memory::write_real(&mut vm.frames.data, base + RET_OFF, val);
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
    memory::write_real(&mut vm.frames.data, base + RET_OFF, val);
    Ok(())
}
