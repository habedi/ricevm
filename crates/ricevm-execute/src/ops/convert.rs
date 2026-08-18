use ricevm_core::ExecError;

use crate::vm::VmState;

/// Emulate C's strtol/strtoll: skip leading whitespace, parse optional sign,
/// then decimal digits. Stops at the first non-digit. Returns 0 for empty/invalid.
fn parse_strtol(s: &str) -> i64 {
    let s = s.trim_start();
    if s.is_empty() {
        return 0;
    }
    let (neg, s) = if let Some(rest) = s.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = s.strip_prefix('+') {
        (false, rest)
    } else {
        (false, s)
    };
    let mut val: i64 = 0;
    let mut overflowed = false;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            break;
        }
        if overflowed {
            continue;
        }
        // C's strtol/strtoll clamp to LONG_MIN/LONG_MAX; they do not wrap.
        match val
            .checked_mul(10)
            .and_then(|v| v.checked_add((b - b'0') as i64))
        {
            Some(v) => val = v,
            None => overflowed = true,
        }
    }
    if overflowed {
        return if neg { i64::MIN } else { i64::MAX };
    }
    if neg { -val } else { val }
}

/// Emulate C's strtod: skip leading whitespace, then convert the longest
/// prefix that forms a number. Characters after that prefix are ignored, and a
/// string with no number in front converts to zero.
///
/// The reference reaches straight for `strtod` (`cvtcf`, libinterp/string.c), so
/// "3.5abc" is 3.5 there. Rust's own `str::parse` rejects the whole string
/// instead, which would turn every such value into zero.
fn parse_strtod(s: &str) -> f64 {
    let s = s.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let bytes = s.as_bytes();
    let mut end = 0usize;
    if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
        end += 1;
    }
    // Infinity and NaN are spelled out rather than written in digits, and
    // strtod accepts either spelling in any case.
    let rest = s[end..].to_ascii_lowercase();
    for name in ["infinity", "inf", "nan"] {
        if rest.starts_with(name) {
            let stop = end + name.len();
            return s[..stop].parse::<f64>().unwrap_or(0.0);
        }
    }
    let mut digits = 0usize;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
        digits += 1;
    }
    if end < bytes.len() && bytes[end] == b'.' {
        end += 1;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return 0.0;
    }
    // An exponent counts only when at least one digit follows it, so the 'e' of
    // "1e" stays outside the number.
    if end < bytes.len() && (bytes[end] == b'e' || bytes[end] == b'E') {
        let mut after = end + 1;
        if after < bytes.len() && (bytes[after] == b'+' || bytes[after] == b'-') {
            after += 1;
        }
        let exp_start = after;
        while after < bytes.len() && bytes[after].is_ascii_digit() {
            after += 1;
        }
        if after > exp_start {
            end = after;
        }
    }
    s[..end].parse::<f64>().unwrap_or(0.0)
}

/// Emulate C's %g format for f64.
/// %g uses the shorter of %e and %f, with 6 significant digits,
/// and strips trailing zeros.
fn format_g(val: f64) -> String {
    if val.is_nan() {
        return "NaN".to_string();
    }
    if val.is_infinite() {
        return if val > 0.0 {
            "+Inf".to_string()
        } else {
            "-Inf".to_string()
        };
    }
    // C picks between %e and %f from the exponent the value has *after* it is
    // rounded to six significant digits (C99 7.19.6.1), so the rounding has to
    // come first. Reading the exponent off a seven-digit conversion prints
    // 999999.9 as 1000000, where C prints 1e+06.
    let scientific = format!("{:.5e}", val);
    let exp = match scientific.find('e') {
        Some(pos) => scientific[pos + 1..].parse::<i32>().unwrap_or(0),
        None => 0,
    };
    if (-4..6).contains(&exp) {
        // Use fixed notation with enough precision
        let precision = (5 - exp).clamp(0, 20) as usize;
        let mut result = format!("{:.*}", precision, val);
        // Strip trailing zeros after decimal point
        if result.contains('.') {
            result = result.trim_end_matches('0').to_string();
            result = result.trim_end_matches('.').to_string();
        }
        result
    } else {
        // Use scientific notation
        let mut mantissa = scientific;
        // Normalize the exponent format to match C's %g
        if let Some(pos) = mantissa.find('e') {
            let (m, e) = mantissa.split_at(pos);
            let mut m = m.to_string();
            // Strip trailing zeros from mantissa
            if m.contains('.') {
                m = m.trim_end_matches('0').to_string();
                m = m.trim_end_matches('.').to_string();
            }
            // Format exponent like C: e+XX or e-XX with minimal digits
            let exp_val: i32 = e[1..].parse().unwrap_or(0);
            mantissa = if exp_val >= 0 {
                format!("{}e+{:02}", m, exp_val)
            } else {
                format!("{}e-{:02}", m, -exp_val)
            };
        }
        mantissa
    }
}

pub(crate) fn op_cvtbw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_byte()? as i32;
    vm.set_dst_word(val)
}

pub(crate) fn op_cvtwb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let val = vm.src_word()? as u8;
    vm.set_dst_byte(val)
}

pub(crate) fn op_cvtfw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // Reference: f = F(s); W(d) = f < 0 ? f - .5 : f + .5;
    // Rounds to nearest integer, not truncates.
    let f = vm.src_real()?;
    let val = if f < 0.0 { f - 0.5 } else { f + 0.5 };
    vm.set_dst_word(val as i32)
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
    // Reference: f = F(s); V(d) = f < 0 ? f - .5 : f + .5;
    // Rounds to nearest integer, not truncates.
    let f = vm.src_real()?;
    let val = if f < 0.0 { f - 0.5 } else { f + 0.5 };
    vm.set_dst_big(val as i64)
}

pub(crate) fn op_cvtwc(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // word to string: format integer as decimal string (reference: sprint("%d", W(s)))
    let val = vm.src_word()?;
    let s = format!("{}", val);
    let id = vm.heap.alloc(0, crate::heap::HeapData::Str(s));
    vm.move_ptr_to_dst(id)
}

pub(crate) fn op_cvtcw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // string to word: parse string as base-10 integer (reference: strtol(s, nil, 10))
    let str_id = vm.src_ptr()?;
    let val = match vm.heap.get_string(str_id) {
        // strtol on a 32-bit long saturates at LONG_MIN/LONG_MAX.
        Some(s) => parse_strtol(s).clamp(i32::MIN as i64, i32::MAX as i64) as i32,
        None => 0,
    };
    vm.set_dst_word(val)
}

pub(crate) fn op_cvtfc(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // float to string (reference: sprint("%g", F(s)))
    let val = vm.src_real()?;
    let s = format_g(val);
    let id = vm.heap.alloc(0, crate::heap::HeapData::Str(s));
    vm.move_ptr_to_dst(id)
}

pub(crate) fn op_cvtcf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // string to float (reference: strtod(s, nil))
    let str_id = vm.src_ptr()?;
    let val = match vm.heap.get_string(str_id) {
        Some(s) => parse_strtod(s),
        None => 0.0,
    };
    vm.set_dst_real(val)
}

pub(crate) fn op_cvtlc(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // big to string
    let val = vm.src_big()?;
    let s = val.to_string();
    let id = vm.heap.alloc(0, crate::heap::HeapData::Str(s));
    vm.move_ptr_to_dst(id)
}

pub(crate) fn op_cvtcl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // string to big (reference: strtoll(s, nil, 10))
    let str_id = vm.src_ptr()?;
    let val = match vm.heap.get_string(str_id) {
        Some(s) => parse_strtol(s),
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
    // Reference: F(d) = SR(s);
    // cvtrf = convert SREAL(src) to REAL(dst).
    // SREAL is 32-bit IEEE754 float stored in a word slot.
    let bits = vm.src_word()? as u32;
    let f32_val = f32::from_bits(bits);
    vm.set_dst_real(f32_val as f64)
}

pub(crate) fn op_cvtfr(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    // Reference: SR(d) = F(s);
    // cvtfr = convert REAL(src) to SREAL(dst).
    // Reads f64, converts to f32, stores the f32 bits in a word slot.
    let val = vm.src_real()?;
    let f32_val = val as f32;
    vm.set_dst_word(f32::to_bits(f32_val) as i32)
}

#[cfg(test)]
mod tests {
    use ricevm_core::{
        Header, Instruction, MiddleOperand, Module, Opcode, Operand, PointerMap, RuntimeFlags,
        TypeDescriptor, XMAGIC,
    };

    use crate::address::AddrTarget;
    use crate::memory;
    use crate::vm::VmState;

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
            name: "convert_test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        }
    }

    #[test]
    fn cvtfw_rounds_positive_halfway() {
        // 2.5 should round to 3 (not truncate to 2)
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, fp, 2.5);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 8);

        op_cvtfw(&mut vm).expect("cvtfw should succeed");

        assert_eq!(memory::read_word(&vm.frames.data, fp + 8), 3);
    }

    #[test]
    fn cvtfw_rounds_negative_halfway() {
        // -2.5 should round to -3 (not truncate to -2)
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, fp, -2.5);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 8);

        op_cvtfw(&mut vm).expect("cvtfw should succeed");

        assert_eq!(memory::read_word(&vm.frames.data, fp + 8), -3);
    }

    #[test]
    fn cvtfw_rounds_positive_below_half() {
        // 2.3 + 0.5 = 2.8 -> truncated to 2
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, fp, 2.3);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 8);

        op_cvtfw(&mut vm).expect("cvtfw should succeed");

        assert_eq!(memory::read_word(&vm.frames.data, fp + 8), 2);
    }

    #[test]
    fn cvtfl_rounds_positive_halfway() {
        // 2.5 should round to 3 (not truncate to 2)
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, fp, 2.5);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 8);

        op_cvtfl(&mut vm).expect("cvtfl should succeed");

        assert_eq!(memory::read_big(&vm.frames.data, fp + 8), 3);
    }

    #[test]
    fn cvtfl_rounds_negative_halfway() {
        // -2.5 should round to -3
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, fp, -2.5);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 8);

        op_cvtfl(&mut vm).expect("cvtfl should succeed");

        assert_eq!(memory::read_big(&vm.frames.data, fp + 8), -3);
    }

    #[test]
    fn cvtrf_reads_f32_bits_from_word() {
        // Store f32 3.5 as bits in a word slot, then convert to f64
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        let f32_bits = f32::to_bits(3.5_f32) as i32;
        memory::write_word(&mut vm.frames.data, fp, f32_bits);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 4);

        op_cvtrf(&mut vm).expect("cvtrf should succeed");

        let result = memory::read_real(&vm.frames.data, fp + 4);
        assert!((result - 3.5_f32 as f64).abs() < 1e-6);
    }

    #[test]
    fn cvtfr_stores_f32_bits_in_word() {
        // Convert f64 3.5 to f32, store bits as word
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, fp, 3.5);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 8);

        op_cvtfr(&mut vm).expect("cvtfr should succeed");

        let word = memory::read_word(&vm.frames.data, fp + 8);
        let f32_val = f32::from_bits(word as u32);
        assert!((f32_val - 3.5_f32).abs() < 1e-6);
    }

    #[test]
    fn cvtrf_cvtfr_roundtrip() {
        // f64 -> f32 -> f64 should preserve f32 precision
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        // First: cvtfr to convert f64 to f32 bits
        memory::write_real(&mut vm.frames.data, fp, 42.5);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 8);
        op_cvtfr(&mut vm).expect("cvtfr should succeed");

        // Then: cvtrf to convert f32 bits back to f64
        vm.src = AddrTarget::Frame(fp + 8);
        vm.dst = AddrTarget::Frame(fp + 12);
        op_cvtrf(&mut vm).expect("cvtrf should succeed");

        let result = memory::read_real(&vm.frames.data, fp + 12);
        assert_eq!(result, 42.5); // 42.5 is exactly representable in both f32 and f64
    }

    #[test]
    fn cvtwc_formats_integer_as_decimal_string() {
        // Reference: sprint("%d", W(s)) -- should format integer as decimal
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        memory::write_word(&mut vm.frames.data, fp, 42);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 4);
        op_cvtwc(&mut vm).expect("cvtwc should succeed");

        let str_id = memory::read_word(&vm.frames.data, fp + 4) as u32;
        let result = vm.heap.get_string(str_id).unwrap();
        assert_eq!(result, "42");
    }

    #[test]
    fn cvtwc_formats_negative_integer() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        memory::write_word(&mut vm.frames.data, fp, -123);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 4);
        op_cvtwc(&mut vm).expect("cvtwc should succeed");

        let str_id = memory::read_word(&vm.frames.data, fp + 4) as u32;
        let result = vm.heap.get_string(str_id).unwrap();
        assert_eq!(result, "-123");
    }

    #[test]
    fn cvtcw_parses_decimal_string() {
        // Reference: strtol(s, nil, 10)
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        let str_id = vm
            .heap
            .alloc(0, crate::heap::HeapData::Str("42".to_string()));
        vm.src = AddrTarget::Immediate;
        vm.imm_src = str_id as i32;
        vm.dst = AddrTarget::Frame(fp);
        op_cvtcw(&mut vm).expect("cvtcw should succeed");

        assert_eq!(memory::read_word(&vm.frames.data, fp), 42);
    }

    #[test]
    fn cvtcw_handles_leading_whitespace_and_sign() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        let str_id = vm
            .heap
            .alloc(0, crate::heap::HeapData::Str("  -99xyz".to_string()));
        vm.src = AddrTarget::Immediate;
        vm.imm_src = str_id as i32;
        vm.dst = AddrTarget::Frame(fp);
        op_cvtcw(&mut vm).expect("cvtcw should succeed");

        assert_eq!(memory::read_word(&vm.frames.data, fp), -99);
    }

    #[test]
    fn cvtcw_nil_returns_zero() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        vm.src = AddrTarget::Immediate;
        vm.imm_src = crate::heap::NIL as i32;
        vm.dst = AddrTarget::Frame(fp);
        op_cvtcw(&mut vm).expect("cvtcw should succeed");

        assert_eq!(memory::read_word(&vm.frames.data, fp), 0);
    }

    #[test]
    fn cvtfc_uses_g_format() {
        // Reference: sprint("%g", F(s))
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        memory::write_real(&mut vm.frames.data, fp, 3.75);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 8);
        op_cvtfc(&mut vm).expect("cvtfc should succeed");

        let str_id = memory::read_word(&vm.frames.data, fp + 8) as u32;
        let result = vm.heap.get_string(str_id).unwrap();
        assert_eq!(result, "3.75");
    }

    #[test]
    fn cvtfc_formats_zero() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        memory::write_real(&mut vm.frames.data, fp, 0.0);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 8);
        op_cvtfc(&mut vm).expect("cvtfc should succeed");

        let str_id = memory::read_word(&vm.frames.data, fp + 8) as u32;
        let result = vm.heap.get_string(str_id).unwrap();
        assert_eq!(result, "0");
    }

    #[test]
    fn cvtfc_large_number_uses_scientific() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        memory::write_real(&mut vm.frames.data, fp, 1e20);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 8);
        op_cvtfc(&mut vm).expect("cvtfc should succeed");

        let str_id = memory::read_word(&vm.frames.data, fp + 8) as u32;
        let result = vm.heap.get_string(str_id).unwrap();
        assert_eq!(result, "1e+20");
    }

    #[test]
    fn parse_strtol_basic() {
        assert_eq!(parse_strtol("123"), 123);
        assert_eq!(parse_strtol("-456"), -456);
        assert_eq!(parse_strtol("+789"), 789);
        assert_eq!(parse_strtol("  42"), 42);
        assert_eq!(parse_strtol("12abc"), 12);
        assert_eq!(parse_strtol(""), 0);
        assert_eq!(parse_strtol("abc"), 0);
    }

    #[test]
    fn parse_strtol_saturates_like_c() {
        // C's strtoll clamps to LLONG_MIN/LLONG_MAX instead of wrapping.
        assert_eq!(parse_strtol("99999999999999999999999"), i64::MAX);
        assert_eq!(parse_strtol("-99999999999999999999999"), i64::MIN);
        assert_eq!(parse_strtol("9223372036854775807"), i64::MAX);
        assert_eq!(parse_strtol("-9223372036854775808"), i64::MIN);
    }

    #[test]
    fn cvtcw_saturates_out_of_range_string() {
        // C's strtol on a 32-bit long clamps to LONG_MAX, it does not wrap.
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        let str_id = vm
            .heap
            .alloc(0, crate::heap::HeapData::Str("9999999999".to_string()));
        vm.src = AddrTarget::Immediate;
        vm.imm_src = str_id as i32;
        vm.dst = AddrTarget::Frame(fp);
        op_cvtcw(&mut vm).expect("cvtcw should succeed");

        assert_eq!(memory::read_word(&vm.frames.data, fp), i32::MAX);
    }

    #[test]
    fn cvtcw_saturates_negative_out_of_range_string() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        let str_id = vm
            .heap
            .alloc(0, crate::heap::HeapData::Str("-9999999999".to_string()));
        vm.src = AddrTarget::Immediate;
        vm.imm_src = str_id as i32;
        vm.dst = AddrTarget::Frame(fp);
        op_cvtcw(&mut vm).expect("cvtcw should succeed");

        assert_eq!(memory::read_word(&vm.frames.data, fp), i32::MIN);
    }

    #[test]
    fn cvtcl_saturates_out_of_range_string() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        let str_id = vm.heap.alloc(
            0,
            crate::heap::HeapData::Str("99999999999999999999".to_string()),
        );
        vm.src = AddrTarget::Immediate;
        vm.imm_src = str_id as i32;
        vm.dst = AddrTarget::Frame(fp);
        op_cvtcl(&mut vm).expect("cvtcl should succeed");

        assert_eq!(memory::read_big(&vm.frames.data, fp), i64::MAX);
    }

    /// `strtod` takes the longest numeric prefix, accepts a named infinity or a
    /// NaN in any case, and answers zero when there is no number to read.
    #[test]
    fn parse_strtod_matches_c() {
        assert_eq!(parse_strtod("3.5"), 3.5);
        assert_eq!(parse_strtod("  -0.25xyz"), -0.25);
        assert_eq!(parse_strtod("+2"), 2.0);
        assert_eq!(parse_strtod("1e3"), 1000.0);
        assert_eq!(parse_strtod("1e+3"), 1000.0);
        assert_eq!(parse_strtod("1e-3"), 0.001);
        // The exponent needs a digit, so the 'e' is not part of these numbers.
        assert_eq!(parse_strtod("1e"), 1.0);
        assert_eq!(parse_strtod("1e+"), 1.0);
        assert_eq!(parse_strtod("2.5e"), 2.5);
        assert_eq!(parse_strtod(""), 0.0);
        assert_eq!(parse_strtod("."), 0.0);
        assert_eq!(parse_strtod("-"), 0.0);
        assert_eq!(parse_strtod("abc"), 0.0);
        assert_eq!(parse_strtod("5."), 5.0);
        assert_eq!(parse_strtod(".5"), 0.5);
        assert!(parse_strtod("inf").is_infinite());
        assert!(parse_strtod("-INFINITY") < 0.0 && parse_strtod("-INFINITY").is_infinite());
        assert!(parse_strtod("NaN").is_nan());
        assert!(parse_strtod("nanabc").is_nan());
        // An overflowing exponent gives an infinity, as strtod's HUGE_VAL does.
        assert!(parse_strtod("1e999").is_infinite());
    }

    #[test]
    fn format_g_basic() {
        assert_eq!(format_g(0.0), "0");
        assert_eq!(format_g(1.0), "1");
        assert_eq!(format_g(3.75), "3.75");
        assert_eq!(format_g(1e20), "1e+20");
        assert_eq!(format_g(-1.5), "-1.5");
        assert_eq!(format_g(100.0), "100");
    }

    // --- the conversions, run through the dispatch table ---
    //
    // Every case below dispatches on the opcode instead of calling the handler,
    // so a table arm wired to the wrong conversion fails here. The expected
    // values come from `libinterp/xec.c` for the numeric conversions and
    // `libinterp/string.c` for the ones that build or read a string.

    fn cvt_inst(opcode: Opcode) -> Instruction {
        Instruction {
            opcode,
            source: Operand::UNUSED,
            middle: MiddleOperand::UNUSED,
            destination: Operand::UNUSED,
        }
    }

    fn dispatch_cvt(vm: &mut VmState<'_>, opcode: Opcode) {
        crate::ops::dispatch(vm, &cvt_inst(opcode))
            .unwrap_or_else(|e| panic!("{opcode:?} should succeed: {e}"));
    }

    /// Set up a conversion whose source is a frame slot and whose destination is
    /// the slot 16 bytes above it, leaving room for the widest value either side
    /// can hold.
    fn frame_pair(vm: &mut VmState<'_>) -> (usize, usize) {
        let fp = vm.frames.current_data_offset();
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 16);
        (fp, fp + 16)
    }

    /// `W(d) = B(s)`: a byte is unsigned, so 0xFF widens to 255 rather than to
    /// -1.
    #[test]
    fn cvtbw_widens_a_byte_as_unsigned() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let (src, dst) = frame_pair(&mut vm);
        vm.frames.data[src] = 0xFF;
        dispatch_cvt(&mut vm, Opcode::Cvtbw);
        assert_eq!(memory::read_word(&vm.frames.data, dst), 255);
    }

    /// `B(d) = W(s)`: the word narrows to its low byte.
    #[test]
    fn cvtwb_keeps_the_low_byte() {
        let module = test_module();
        for (word, byte) in [
            (0x1234_5678_i32, 0x78_u8),
            (-1, 0xFF),
            (256, 0),
            (255, 0xFF),
        ] {
            let mut vm = VmState::new(&module).expect("vm init");
            let (src, dst) = frame_pair(&mut vm);
            memory::write_word(&mut vm.frames.data, src, word);
            dispatch_cvt(&mut vm, Opcode::Cvtwb);
            assert_eq!(vm.frames.data[dst], byte, "cvtwb of {word}");
        }
    }

    #[test]
    fn cvtwf_converts_a_word_to_a_real() {
        let module = test_module();
        for word in [0_i32, -7, i32::MAX, i32::MIN] {
            let mut vm = VmState::new(&module).expect("vm init");
            let (src, dst) = frame_pair(&mut vm);
            memory::write_word(&mut vm.frames.data, src, word);
            dispatch_cvt(&mut vm, Opcode::Cvtwf);
            assert_eq!(
                memory::read_real(&vm.frames.data, dst),
                word as f64,
                "cvtwf of {word}"
            );
        }
    }

    /// `V(d) = W(s)`: the word widens with its sign.
    #[test]
    fn cvtwl_sign_extends_a_word() {
        let module = test_module();
        for word in [-5_i32, i32::MIN, i32::MAX, 0] {
            let mut vm = VmState::new(&module).expect("vm init");
            let (src, dst) = frame_pair(&mut vm);
            memory::write_word(&mut vm.frames.data, src, word);
            dispatch_cvt(&mut vm, Opcode::Cvtwl);
            assert_eq!(
                memory::read_big(&vm.frames.data, dst),
                word as i64,
                "cvtwl of {word}"
            );
        }
    }

    /// `W(d) = V(s)`: a C narrowing conversion, which keeps the low word and
    /// discards the rest. It does not saturate.
    #[test]
    fn cvtlw_keeps_the_low_word() {
        let module = test_module();
        for (big, word) in [
            (-5_i64, -5_i32),
            ((1_i64 << 32) + 5, 5),
            (i64::MIN, 0),
            (i64::MAX, -1),
        ] {
            let mut vm = VmState::new(&module).expect("vm init");
            let (src, dst) = frame_pair(&mut vm);
            memory::write_big(&mut vm.frames.data, src, big);
            dispatch_cvt(&mut vm, Opcode::Cvtlw);
            assert_eq!(
                memory::read_word(&vm.frames.data, dst),
                word,
                "cvtlw of {big}"
            );
        }
    }

    #[test]
    fn cvtlf_converts_a_big_to_a_real() {
        let module = test_module();
        for big in [0_i64, -5, 1_i64 << 40, i64::MAX, i64::MIN] {
            let mut vm = VmState::new(&module).expect("vm init");
            let (src, dst) = frame_pair(&mut vm);
            memory::write_big(&mut vm.frames.data, src, big);
            dispatch_cvt(&mut vm, Opcode::Cvtlf);
            assert_eq!(
                memory::read_real(&vm.frames.data, dst),
                big as f64,
                "cvtlf of {big}"
            );
        }
    }

    /// `W(d) = f < 0 ? f - .5 : f + .5` truncated, so the rounding is away from
    /// zero at a half and toward zero below it.
    #[test]
    fn cvtfw_rounds_away_from_zero_at_a_half() {
        let module = test_module();
        for (real, word) in [
            (0.4_f64, 0_i32),
            (-0.4, 0),
            (0.6, 1),
            (-0.6, -1),
            (1.5, 2),
            (-1.5, -2),
            (0.0, 0),
            (-0.0, 0),
        ] {
            let mut vm = VmState::new(&module).expect("vm init");
            let (src, dst) = frame_pair(&mut vm);
            memory::write_real(&mut vm.frames.data, src, real);
            dispatch_cvt(&mut vm, Opcode::Cvtfw);
            assert_eq!(
                memory::read_word(&vm.frames.data, dst),
                word,
                "cvtfw of {real}"
            );
        }
    }

    /// The reference leaves an out-of-range float-to-integer conversion
    /// undefined, since it is a plain C assignment. Rust's cast saturates, and
    /// that is the behavior this VM commits to: a program gets the nearest
    /// representable value rather than whatever the host happens to produce.
    #[test]
    fn cvtfw_saturates_outside_the_word_range() {
        let module = test_module();
        for (real, word) in [
            (1e30_f64, i32::MAX),
            (-1e30, i32::MIN),
            (f64::INFINITY, i32::MAX),
            (f64::NEG_INFINITY, i32::MIN),
            (f64::NAN, 0),
        ] {
            let mut vm = VmState::new(&module).expect("vm init");
            let (src, dst) = frame_pair(&mut vm);
            memory::write_real(&mut vm.frames.data, src, real);
            dispatch_cvt(&mut vm, Opcode::Cvtfw);
            assert_eq!(
                memory::read_word(&vm.frames.data, dst),
                word,
                "cvtfw of {real}"
            );
        }
    }

    #[test]
    fn cvtfl_saturates_outside_the_big_range() {
        let module = test_module();
        for (real, big) in [
            (1e30_f64, i64::MAX),
            (-1e30, i64::MIN),
            (f64::INFINITY, i64::MAX),
            (f64::NEG_INFINITY, i64::MIN),
            (f64::NAN, 0),
            (2.5, 3),
            (-2.5, -3),
        ] {
            let mut vm = VmState::new(&module).expect("vm init");
            let (src, dst) = frame_pair(&mut vm);
            memory::write_real(&mut vm.frames.data, src, real);
            dispatch_cvt(&mut vm, Opcode::Cvtfl);
            assert_eq!(
                memory::read_big(&vm.frames.data, dst),
                big,
                "cvtfl of {real}"
            );
        }
    }

    /// `SH(d) = W(s)` keeps the low 16 bits, and `W(d) = SH(s)` reads them back
    /// with their sign. Note that the reference stores a 16-bit value where this
    /// handler stores a whole word; the 16 bits themselves agree.
    #[test]
    fn cvtws_and_cvtsw_work_on_sixteen_bits() {
        let module = test_module();
        for (word, short) in [
            (0x0001_8000_i32, -32768_i32),
            (0x0000_FFFF, -1),
            (0x0000_7FFF, 32767),
            (5, 5),
            (-1, -1),
        ] {
            for opcode in [Opcode::Cvtws, Opcode::Cvtsw] {
                let mut vm = VmState::new(&module).expect("vm init");
                let (src, dst) = frame_pair(&mut vm);
                memory::write_word(&mut vm.frames.data, src, word);
                dispatch_cvt(&mut vm, opcode);
                assert_eq!(
                    memory::read_word(&vm.frames.data, dst),
                    short,
                    "{opcode:?} of {word:#x}"
                );
            }
        }
    }

    /// `sprint("%lld", V(s))`: a big prints in full, including the value with no
    /// positive counterpart.
    #[test]
    fn cvtlc_formats_a_big_in_decimal() {
        let module = test_module();
        for (big, text) in [
            (0_i64, "0"),
            (-5, "-5"),
            (1_i64 << 40, "1099511627776"),
            (i64::MAX, "9223372036854775807"),
            (i64::MIN, "-9223372036854775808"),
        ] {
            let mut vm = VmState::new(&module).expect("vm init");
            let (src, dst) = frame_pair(&mut vm);
            memory::write_big(&mut vm.frames.data, src, big);
            memory::write_word(&mut vm.frames.data, dst, crate::heap::NIL as i32);
            dispatch_cvt(&mut vm, Opcode::Cvtlc);
            let str_id = memory::read_word(&vm.frames.data, dst) as u32;
            assert_eq!(
                vm.heap.get_string(str_id).expect("a string"),
                text,
                "cvtlc of {big}"
            );
        }
    }

    /// `strtoll(s, nil, 10)`: leading whitespace is skipped, a sign is allowed,
    /// and conversion stops at the first character that is not a digit.
    #[test]
    fn cvtcl_parses_a_big_like_strtoll() {
        let module = test_module();
        for (text, big) in [
            ("42", 42_i64),
            ("  -42abc", -42),
            ("+7", 7),
            ("", 0),
            ("abc", 0),
            ("9223372036854775807", i64::MAX),
            ("1099511627776", 1_i64 << 40),
        ] {
            let mut vm = VmState::new(&module).expect("vm init");
            let (_, dst) = frame_pair(&mut vm);
            let str_id = vm
                .heap
                .alloc(0, crate::heap::HeapData::Str(text.to_string()));
            vm.src = AddrTarget::Immediate;
            vm.imm_src = str_id as i32;
            dispatch_cvt(&mut vm, Opcode::Cvtcl);
            assert_eq!(
                memory::read_big(&vm.frames.data, dst),
                big,
                "cvtcl of {text:?}"
            );
        }
    }

    #[test]
    fn cvtcl_of_nil_is_zero() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let (_, dst) = frame_pair(&mut vm);
        memory::write_big(&mut vm.frames.data, dst, -1);
        vm.src = AddrTarget::Immediate;
        vm.imm_src = crate::heap::NIL as i32;
        dispatch_cvt(&mut vm, Opcode::Cvtcl);
        assert_eq!(memory::read_big(&vm.frames.data, dst), 0);
    }

    /// `strtod(s, nil)` converts the longest prefix that forms a number, so
    /// trailing text is ignored rather than turning the whole conversion into
    /// zero (libinterp/string.c calls it from `cvtcf`).
    #[test]
    fn cvtcf_parses_a_real_like_strtod() {
        let module = test_module();
        for (text, real) in [
            ("3.5", 3.5_f64),
            ("-2.5e2", -250.0),
            ("  3.5", 3.5),
            ("3.5abc", 3.5),
            ("1.5 ", 1.5),
            ("12", 12.0),
            (".5", 0.5),
            ("-.5", -0.5),
            ("5.", 5.0),
            ("1e3", 1000.0),
            ("1e", 1.0),
            ("", 0.0),
            ("abc", 0.0),
            ("+2.25xyz", 2.25),
        ] {
            let mut vm = VmState::new(&module).expect("vm init");
            let (_, dst) = frame_pair(&mut vm);
            let str_id = vm
                .heap
                .alloc(0, crate::heap::HeapData::Str(text.to_string()));
            vm.src = AddrTarget::Immediate;
            vm.imm_src = str_id as i32;
            dispatch_cvt(&mut vm, Opcode::Cvtcf);
            assert_eq!(
                memory::read_real(&vm.frames.data, dst),
                real,
                "cvtcf of {text:?}"
            );
        }
    }

    #[test]
    fn cvtcf_of_nil_is_zero() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let (_, dst) = frame_pair(&mut vm);
        memory::write_real(&mut vm.frames.data, dst, -1.0);
        vm.src = AddrTarget::Immediate;
        vm.imm_src = crate::heap::NIL as i32;
        dispatch_cvt(&mut vm, Opcode::Cvtcf);
        assert_eq!(memory::read_real(&vm.frames.data, dst), 0.0);
    }

    /// C's `%g` uses six significant digits and picks scientific notation from
    /// the exponent the value has after that rounding, which is why 999999.9
    /// prints as 1e+06 and not as 1000000 (C99 7.19.6.1).
    #[test]
    fn format_g_matches_c() {
        let cases = [
            (0.0, "0"),
            (-0.0, "-0"),
            (1.0, "1"),
            (-1.5, "-1.5"),
            (2.5, "2.5"),
            (3.75, "3.75"),
            (100.0, "100"),
            (0.1, "0.1"),
            (2.0 / 3.0, "0.666667"),
            (999999.0, "999999"),
            (999999.4, "999999"),
            (999999.9, "1e+06"),
            (1000000.0, "1e+06"),
            (1234567.0, "1.23457e+06"),
            (123456.7, "123457"),
            (0.0001, "0.0001"),
            (9.999999e-5, "0.0001"),
            (9.99999e-5, "9.99999e-05"),
            (1e-5, "1e-05"),
            (0.000123456749, "0.000123457"),
            (1e20, "1e+20"),
            (1e300, "1e+300"),
            (-999999.9, "-1e+06"),
        ];
        for (val, text) in cases {
            assert_eq!(format_g(val), text, "%g of {val}");
        }
    }

    /// Inferno's own float formatting spells these three out as "NaN", "+Inf",
    /// and "-Inf" (lib9/fltfmt.c), so `%g` of them is not the C library's
    /// lowercase spelling.
    #[test]
    fn format_g_names_nan_and_infinity_the_way_inferno_does() {
        assert_eq!(format_g(f64::NAN), "NaN");
        assert_eq!(format_g(f64::INFINITY), "+Inf");
        assert_eq!(format_g(f64::NEG_INFINITY), "-Inf");
    }

    #[test]
    fn cvtfc_of_nan_and_infinity_uses_the_reference_spelling() {
        let module = test_module();
        for (real, text) in [
            (f64::NAN, "NaN"),
            (f64::INFINITY, "+Inf"),
            (f64::NEG_INFINITY, "-Inf"),
        ] {
            let mut vm = VmState::new(&module).expect("vm init");
            let (src, dst) = frame_pair(&mut vm);
            memory::write_real(&mut vm.frames.data, src, real);
            memory::write_word(&mut vm.frames.data, dst, crate::heap::NIL as i32);
            dispatch_cvt(&mut vm, Opcode::Cvtfc);
            let str_id = memory::read_word(&vm.frames.data, dst) as u32;
            assert_eq!(
                vm.heap.get_string(str_id).expect("a string"),
                text,
                "cvtfc of {real}"
            );
        }
    }

    #[test]
    fn cvtfc_formats_with_six_significant_digits() {
        let module = test_module();
        for (real, text) in [
            (2.0 / 3.0, "0.666667"),
            (1234567.0, "1.23457e+06"),
            (999999.9, "1e+06"),
            (-1.5, "-1.5"),
        ] {
            let mut vm = VmState::new(&module).expect("vm init");
            let (src, dst) = frame_pair(&mut vm);
            memory::write_real(&mut vm.frames.data, src, real);
            memory::write_word(&mut vm.frames.data, dst, crate::heap::NIL as i32);
            dispatch_cvt(&mut vm, Opcode::Cvtfc);
            let str_id = memory::read_word(&vm.frames.data, dst) as u32;
            assert_eq!(
                vm.heap.get_string(str_id).expect("a string"),
                text,
                "cvtfc of {real}"
            );
        }
    }

    #[test]
    fn cvtwc_formats_the_word_extremes() {
        let module = test_module();
        for (word, text) in [
            (0_i32, "0"),
            (i32::MAX, "2147483647"),
            (i32::MIN, "-2147483648"),
        ] {
            let mut vm = VmState::new(&module).expect("vm init");
            let (src, dst) = frame_pair(&mut vm);
            memory::write_word(&mut vm.frames.data, src, word);
            memory::write_word(&mut vm.frames.data, dst, crate::heap::NIL as i32);
            dispatch_cvt(&mut vm, Opcode::Cvtwc);
            let str_id = memory::read_word(&vm.frames.data, dst) as u32;
            assert_eq!(
                vm.heap.get_string(str_id).expect("a string"),
                text,
                "cvtwc of {word}"
            );
        }
    }

    /// A word that survives a trip through a string is the pair's real contract,
    /// including the value with no positive counterpart.
    #[test]
    fn cvtwc_and_cvtcw_round_trip_a_word() {
        let module = test_module();
        for word in [0_i32, 42, -42, i32::MAX, i32::MIN] {
            let mut vm = VmState::new(&module).expect("vm init");
            let fp = vm.frames.current_data_offset();
            memory::write_word(&mut vm.frames.data, fp, word);
            memory::write_word(&mut vm.frames.data, fp + 16, crate::heap::NIL as i32);
            vm.src = AddrTarget::Frame(fp);
            vm.dst = AddrTarget::Frame(fp + 16);
            dispatch_cvt(&mut vm, Opcode::Cvtwc);

            vm.src = AddrTarget::Frame(fp + 16);
            vm.dst = AddrTarget::Frame(fp + 24);
            dispatch_cvt(&mut vm, Opcode::Cvtcw);
            assert_eq!(
                memory::read_word(&vm.frames.data, fp + 24),
                word,
                "round trip of {word}"
            );
        }
    }

    /// `SR(d) = F(s)` keeps only what a 32-bit float can hold, so the trip back
    /// through `cvtrf` returns the rounded value rather than the original.
    #[test]
    fn cvtfr_rounds_to_f32_precision() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, fp, 0.1);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 16);
        dispatch_cvt(&mut vm, Opcode::Cvtfr);

        vm.src = AddrTarget::Frame(fp + 16);
        vm.dst = AddrTarget::Frame(fp + 24);
        dispatch_cvt(&mut vm, Opcode::Cvtrf);
        let back = memory::read_real(&vm.frames.data, fp + 24);
        assert_eq!(back, 0.1_f32 as f64);
        assert_ne!(
            back, 0.1,
            "a real does not survive a 32-bit float unchanged"
        );
    }

    /// A value past the 32-bit float range becomes an infinity, which is what a
    /// C conversion from double to float produces.
    #[test]
    fn cvtfr_overflows_to_infinity() {
        let module = test_module();
        for (real, expected) in [
            (1e300_f64, f64::INFINITY),
            (-1e300, f64::NEG_INFINITY),
            (1e-300, 0.0),
        ] {
            let mut vm = VmState::new(&module).expect("vm init");
            let fp = vm.frames.current_data_offset();
            memory::write_real(&mut vm.frames.data, fp, real);
            vm.src = AddrTarget::Frame(fp);
            vm.dst = AddrTarget::Frame(fp + 16);
            dispatch_cvt(&mut vm, Opcode::Cvtfr);

            vm.src = AddrTarget::Frame(fp + 16);
            vm.dst = AddrTarget::Frame(fp + 24);
            dispatch_cvt(&mut vm, Opcode::Cvtrf);
            assert_eq!(
                memory::read_real(&vm.frames.data, fp + 24),
                expected,
                "cvtfr of {real}"
            );
        }
    }
}
