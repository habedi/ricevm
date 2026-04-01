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
    for b in s.bytes() {
        if b.is_ascii_digit() {
            val = val.wrapping_mul(10).wrapping_add((b - b'0') as i64);
        } else {
            break;
        }
    }
    if neg { val.wrapping_neg() } else { val }
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
    // Use %g-like formatting: 6 significant digits, choose shorter form
    let s = format!("{:.6e}", val);
    // Parse the exponent to decide between %f and %e style
    let exp = if let Some(pos) = s.find('e') {
        s[pos + 1..].parse::<i32>().unwrap_or(0)
    } else {
        0
    };
    if (-4..6).contains(&exp) {
        // Use fixed notation with enough precision
        let precision = if exp >= 0 {
            (5 - exp).max(0) as usize
        } else {
            (5 + (-exp) as usize).min(20)
        };
        let mut result = format!("{:.*}", precision, val);
        // Strip trailing zeros after decimal point
        if result.contains('.') {
            result = result.trim_end_matches('0').to_string();
            result = result.trim_end_matches('.').to_string();
        }
        result
    } else {
        // Use scientific notation
        let mut mantissa = format!("{:.*e}", 5, val);
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
        Some(s) => parse_strtol(s) as i32,
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
    // string to float
    let str_id = vm.src_ptr()?;
    let val = match vm.heap.get_string(str_id) {
        Some(s) => s.parse::<f64>().unwrap_or(0.0),
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
        // Store f32 3.14 as bits in a word slot, then convert to f64
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        let f32_bits = f32::to_bits(3.14_f32) as i32;
        memory::write_word(&mut vm.frames.data, fp, f32_bits);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 4);

        op_cvtrf(&mut vm).expect("cvtrf should succeed");

        let result = memory::read_real(&vm.frames.data, fp + 4);
        assert!((result - 3.14_f32 as f64).abs() < 1e-6);
    }

    #[test]
    fn cvtfr_stores_f32_bits_in_word() {
        // Convert f64 3.14 to f32, store bits as word
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        memory::write_real(&mut vm.frames.data, fp, 3.14);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 8);

        op_cvtfr(&mut vm).expect("cvtfr should succeed");

        let word = memory::read_word(&vm.frames.data, fp + 8);
        let f32_val = f32::from_bits(word as u32);
        assert!((f32_val - 3.14_f32).abs() < 1e-6);
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

        memory::write_real(&mut vm.frames.data, fp, 3.14);
        vm.src = AddrTarget::Frame(fp);
        vm.dst = AddrTarget::Frame(fp + 8);
        op_cvtfc(&mut vm).expect("cvtfc should succeed");

        let str_id = memory::read_word(&vm.frames.data, fp + 8) as u32;
        let result = vm.heap.get_string(str_id).unwrap();
        assert_eq!(result, "3.14");
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
    fn format_g_basic() {
        assert_eq!(format_g(0.0), "0");
        assert_eq!(format_g(1.0), "1");
        assert_eq!(format_g(3.14), "3.14");
        assert_eq!(format_g(1e20), "1e+20");
        assert_eq!(format_g(-1.5), "-1.5");
        assert_eq!(format_g(100.0), "100");
    }
}
