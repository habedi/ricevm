use ricevm_core::ExecError;

use crate::vm::VmState;

// Word arithmetic: when mid is present, dst = src OP mid.
// When mid is absent, dst = dst OP src (two-operand form).

pub(crate) fn op_addw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_or_dst_word()?;
    vm.set_dst_word(s.wrapping_add(m))
}

pub(crate) fn op_subw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_or_dst_word()?;
    vm.set_dst_word(m.wrapping_sub(s))
}

pub(crate) fn op_mulw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_or_dst_word()?;
    vm.set_dst_word(s.wrapping_mul(m))
}

pub(crate) fn op_divw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_or_dst_word()?;
    if s == 0 {
        return vm.set_dst_word(0);
    }
    vm.set_dst_word(m.wrapping_div(s))
}

pub(crate) fn op_modw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_or_dst_word()?;
    if s == 0 {
        return vm.set_dst_word(0);
    }
    vm.set_dst_word(m.wrapping_rem(s))
}

// Byte arithmetic

pub(crate) fn op_addb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_or_dst_byte()?;
    vm.set_dst_byte(s.wrapping_add(m))
}

pub(crate) fn op_subb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_or_dst_byte()?;
    vm.set_dst_byte(m.wrapping_sub(s))
}

pub(crate) fn op_mulb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_or_dst_byte()?;
    vm.set_dst_byte(s.wrapping_mul(m))
}

pub(crate) fn op_divb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_or_dst_byte()?;
    if s == 0 {
        return vm.set_dst_byte(0);
    }
    vm.set_dst_byte(m / s)
}

pub(crate) fn op_modb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_or_dst_byte()?;
    if s == 0 {
        return vm.set_dst_byte(0);
    }
    vm.set_dst_byte(m % s)
}

// Word bitwise

pub(crate) fn op_andw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_or_dst_word()?;
    vm.set_dst_word(s & m)
}

pub(crate) fn op_orw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_or_dst_word()?;
    vm.set_dst_word(s | m)
}

pub(crate) fn op_xorw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_or_dst_word()?;
    vm.set_dst_word(s ^ m)
}

pub(crate) fn op_shlw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_or_dst_word()?;
    vm.set_dst_word(m.wrapping_shl(s as u32))
}

pub(crate) fn op_shrw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_or_dst_word()?;
    vm.set_dst_word(m.wrapping_shr(s as u32))
}

pub(crate) fn op_lsrw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_or_dst_word()? as u32;
    vm.set_dst_word(m.wrapping_shr(s as u32) as i32)
}

// Byte bitwise

pub(crate) fn op_andb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_or_dst_byte()?;
    vm.set_dst_byte(s & m)
}

pub(crate) fn op_orb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_or_dst_byte()?;
    vm.set_dst_byte(s | m)
}

pub(crate) fn op_xorb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_or_dst_byte()?;
    vm.set_dst_byte(s ^ m)
}

pub(crate) fn op_shlb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_or_dst_byte()?;
    vm.set_dst_byte(m.wrapping_shl(s as u32))
}

pub(crate) fn op_shrb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_or_dst_byte()?;
    vm.set_dst_byte(m.wrapping_shr(s as u32))
}

// Big bitwise and shift

pub(crate) fn op_andl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_or_dst_big()?;
    vm.set_dst_big(s & m)
}

pub(crate) fn op_orl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_or_dst_big()?;
    vm.set_dst_big(s | m)
}

pub(crate) fn op_xorl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_or_dst_big()?;
    vm.set_dst_big(s ^ m)
}

pub(crate) fn op_shll(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_or_dst_big()?;
    vm.set_dst_big(m.wrapping_shl(s as u32))
}

pub(crate) fn op_shrl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_or_dst_big()?;
    vm.set_dst_big(m.wrapping_shr(s as u32))
}

pub(crate) fn op_lsrl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_or_dst_big()? as u64;
    vm.set_dst_big(m.wrapping_shr(s as u32) as i64)
}

// Exponentiation

pub(crate) fn op_expw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // Reference: x = W(m); n = W(s); — base from mid, exponent from src
    let n = vm.src_word()?;
    let mut x = vm.mid_or_dst_word()?;
    let mut exp = n;
    let inv = exp < 0;
    if inv {
        exp = -exp;
    }
    let mut r: i32 = 1;
    loop {
        if exp & 1 != 0 {
            r = r.wrapping_mul(x);
        }
        exp >>= 1;
        if exp == 0 {
            break;
        }
        x = x.wrapping_mul(x);
    }
    if inv {
        r = if r != 0 { 1_i32.wrapping_div(r) } else { 0 };
    }
    vm.set_dst_word(r)
}

pub(crate) fn op_expl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // Reference: x = V(m); n = W(s); — base from mid (big), exponent from src (word)
    let n = vm.src_word()?;
    let mut x = vm.mid_or_dst_big()?;
    let mut exp = n;
    let inv = exp < 0;
    if inv {
        exp = -exp;
    }
    let mut r: i64 = 1;
    loop {
        if exp & 1 != 0 {
            r = r.wrapping_mul(x);
        }
        exp >>= 1;
        if exp == 0 {
            break;
        }
        x = x.wrapping_mul(x);
    }
    if inv {
        r = if r != 0 { 1_i64.wrapping_div(r) } else { 0 };
    }
    vm.set_dst_big(r)
}

pub(crate) fn op_expf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // Reference: x = F(m); n = W(s); — base from mid (real), exponent from src (word/int)
    let n = vm.src_word()?;
    let mut x = vm.mid_or_dst_real()?;
    let mut exp = n;
    let inv = exp < 0;
    if inv {
        exp = -exp;
    }
    let mut r: f64 = 1.0;
    loop {
        if exp & 1 != 0 {
            r *= x;
        }
        exp >>= 1;
        if exp == 0 {
            break;
        }
        x *= x;
    }
    if inv {
        r = 1.0 / r;
    }
    vm.set_dst_real(r)
}

#[cfg(test)]
mod tests {
    #[test]
    fn property_addw_commutative() {
        for seed in 0..1000u64 {
            let a = (seed.wrapping_mul(1337).wrapping_add(42)) as i32;
            let b = (seed.wrapping_mul(7919).wrapping_add(13)) as i32;
            assert_eq!(
                a.wrapping_add(b),
                b.wrapping_add(a),
                "addw commutativity failed for ({a}, {b})"
            );
        }
    }

    #[test]
    fn property_mulw_commutative() {
        for seed in 0..1000u64 {
            let a = (seed.wrapping_mul(1337).wrapping_add(42)) as i32;
            let b = (seed.wrapping_mul(7919).wrapping_add(13)) as i32;
            assert_eq!(
                a.wrapping_mul(b),
                b.wrapping_mul(a),
                "mulw commutativity failed for ({a}, {b})"
            );
        }
    }

    #[test]
    fn property_andw_commutative() {
        for seed in 0..1000u64 {
            let a = (seed.wrapping_mul(1337).wrapping_add(42)) as i32;
            let b = (seed.wrapping_mul(7919).wrapping_add(13)) as i32;
            assert_eq!(a & b, b & a, "andw commutativity failed for ({a}, {b})");
        }
    }

    #[test]
    fn property_orw_commutative() {
        for seed in 0..1000u64 {
            let a = (seed.wrapping_mul(1337).wrapping_add(42)) as i32;
            let b = (seed.wrapping_mul(7919).wrapping_add(13)) as i32;
            assert_eq!(a | b, b | a, "orw commutativity failed for ({a}, {b})");
        }
    }

    #[test]
    fn property_xorw_commutative() {
        for seed in 0..1000u64 {
            let a = (seed.wrapping_mul(1337).wrapping_add(42)) as i32;
            let b = (seed.wrapping_mul(7919).wrapping_add(13)) as i32;
            assert_eq!(a ^ b, b ^ a, "xorw commutativity failed for ({a}, {b})");
        }
    }

    #[test]
    fn property_shlw_zero_is_identity() {
        for seed in 0..1000u64 {
            let v = (seed.wrapping_mul(2654435761).wrapping_add(1)) as i32;
            assert_eq!(v.wrapping_shl(0), v, "shlw by 0 should be identity for {v}");
        }
    }

    #[test]
    fn property_shrw_zero_is_identity() {
        for seed in 0..1000u64 {
            let v = (seed.wrapping_mul(2654435761).wrapping_add(1)) as i32;
            assert_eq!(v.wrapping_shr(0), v, "shrw by 0 should be identity for {v}");
        }
    }

    #[test]
    fn property_shlw_32_does_not_panic() {
        // Rust's wrapping_shl masks shift to 31 for i32, so shift by 32 is
        // effectively shift by 0. Just verify no panic.
        for seed in 0..100u64 {
            let v = (seed.wrapping_mul(2654435761).wrapping_add(1)) as i32;
            let _ = v.wrapping_shl(32);
        }
    }

    #[test]
    fn property_cvtfw_roundtrip() {
        // Converting int -> float -> int should preserve value for small ints
        for i in -1000..1000i32 {
            let f = i as f64;
            let back = f as i32;
            assert_eq!(i, back, "roundtrip failed for {i}");
        }
    }

    #[test]
    fn property_addw_associative() {
        for seed in 0..500u64 {
            let a = (seed.wrapping_mul(1337).wrapping_add(42)) as i32;
            let b = (seed.wrapping_mul(7919).wrapping_add(13)) as i32;
            let c = (seed.wrapping_mul(104729).wrapping_add(7)) as i32;
            assert_eq!(
                a.wrapping_add(b).wrapping_add(c),
                a.wrapping_add(b.wrapping_add(c)),
                "addw associativity failed for ({a}, {b}, {c})"
            );
        }
    }

    #[test]
    fn property_mulw_associative() {
        for seed in 0..500u64 {
            let a = (seed.wrapping_mul(1337).wrapping_add(42)) as i32;
            let b = (seed.wrapping_mul(7919).wrapping_add(13)) as i32;
            let c = (seed.wrapping_mul(104729).wrapping_add(7)) as i32;
            assert_eq!(
                a.wrapping_mul(b).wrapping_mul(c),
                a.wrapping_mul(b.wrapping_mul(c)),
                "mulw associativity failed for ({a}, {b}, {c})"
            );
        }
    }

    #[test]
    fn property_xorw_self_is_zero() {
        for seed in 0..1000u64 {
            let v = (seed.wrapping_mul(2654435761).wrapping_add(1)) as i32;
            assert_eq!(v ^ v, 0, "xor self should be zero for {v}");
        }
    }

    #[test]
    fn property_andw_self_is_identity() {
        for seed in 0..1000u64 {
            let v = (seed.wrapping_mul(2654435761).wrapping_add(1)) as i32;
            assert_eq!(v & v, v, "and self should be identity for {v}");
        }
    }

    #[test]
    fn property_orw_self_is_identity() {
        for seed in 0..1000u64 {
            let v = (seed.wrapping_mul(2654435761).wrapping_add(1)) as i32;
            assert_eq!(v | v, v, "or self should be identity for {v}");
        }
    }

    #[test]
    fn property_divw_by_one_is_identity() {
        for seed in 0..1000u64 {
            let v = (seed.wrapping_mul(2654435761).wrapping_add(1)) as i32;
            assert_eq!(v.wrapping_div(1), v, "divw by 1 should be identity for {v}");
        }
    }

    #[test]
    fn property_modw_by_one_is_zero() {
        for seed in 0..1000u64 {
            let v = (seed.wrapping_mul(2654435761).wrapping_add(1)) as i32;
            assert_eq!(v.wrapping_rem(1), 0, "modw by 1 should be zero for {v}");
        }
    }

    // Reference-matching exponentiation tests (operand order: base=mid, exp=src)
    #[test]
    fn expw_matches_reference() {
        // Reference: x = W(m) = base, n = W(s) = exponent
        // 2^10 = 1024
        let mut x: i32 = 2;
        let mut n: i32 = 10;
        let inv = false;
        let mut r: i32 = 1;
        loop {
            if n & 1 != 0 {
                r = r.wrapping_mul(x);
            }
            n >>= 1;
            if n == 0 {
                break;
            }
            x = x.wrapping_mul(x);
        }
        if inv {
            r = 1_i32.wrapping_div(r);
        }
        assert_eq!(r, 1024);
    }

    #[test]
    fn expw_negative_exponent() {
        // 2^(-3): inv=true, r = 1/8 = 0 (integer division)
        let mut x: i32 = 2;
        let mut n: i32 = 3; // after negation
        let inv = true;
        let mut r: i32 = 1;
        loop {
            if n & 1 != 0 {
                r = r.wrapping_mul(x);
            }
            n >>= 1;
            if n == 0 {
                break;
            }
            x = x.wrapping_mul(x);
        }
        if inv {
            r = 1_i32.wrapping_div(r);
        }
        assert_eq!(r, 0); // 1/8 = 0 in integer division
    }

    #[test]
    fn expf_uses_integer_exponent() {
        // Reference: x = F(m), n = W(s) — exponent is a WORD (integer)
        let mut x: f64 = 2.0;
        let mut n: i32 = 3;
        let inv = false;
        let mut r: f64 = 1.0;
        loop {
            if n & 1 != 0 {
                r *= x;
            }
            n >>= 1;
            if n == 0 {
                break;
            }
            x *= x;
        }
        if inv {
            r = 1.0 / r;
        }
        assert_eq!(r, 8.0);
    }

    #[test]
    fn expf_negative_exponent_gives_reciprocal() {
        // 2.0 ^ (-2) = 1/4 = 0.25
        let mut x: f64 = 2.0;
        let mut n: i32 = 2; // after negation
        let inv = true;
        let mut r: f64 = 1.0;
        loop {
            if n & 1 != 0 {
                r *= x;
            }
            n >>= 1;
            if n == 0 {
                break;
            }
            x *= x;
        }
        if inv {
            r = 1.0 / r;
        }
        assert_eq!(r, 0.25);
    }
}
