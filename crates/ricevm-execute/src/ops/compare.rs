use ricevm_core::ExecError;

use crate::heap;
use crate::vm::VmState;

// Dis VM branch semantics: if src OP mid, goto dst
// (src and mid are compared; dst is the branch target PC)

// Word comparisons

pub(crate) fn op_beqw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_word()?;
    if s == m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_bnew(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_word()?;
    if s != m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_bltw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_word()?;
    if s < m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_blew(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_word()?;
    if s <= m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_bgtw(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_word()?;
    if s > m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_bgew(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_word()?;
    let m = vm.mid_word()?;
    if s >= m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

// Float comparisons

pub(crate) fn op_beqf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_real()?;
    let m = vm.mid_real()?;
    if s == m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_bnef(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_real()?;
    let m = vm.mid_real()?;
    if s != m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_bltf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_real()?;
    let m = vm.mid_real()?;
    if s < m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_blef(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_real()?;
    let m = vm.mid_real()?;
    if s <= m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_bgtf(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_real()?;
    let m = vm.mid_real()?;
    if s > m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_bgef(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_real()?;
    let m = vm.mid_real()?;
    if s >= m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

// Big comparisons

pub(crate) fn op_beql(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_big()?;
    if s == m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_bnel(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_big()?;
    if s != m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_bltl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_big()?;
    if s < m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_blel(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_big()?;
    if s <= m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_bgtl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_big()?;
    if s > m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_bgel(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_big()?;
    let m = vm.mid_big()?;
    if s >= m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

// Byte comparisons

pub(crate) fn op_beqb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_byte()?;
    if s == m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_bneb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_byte()?;
    if s != m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_bltb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_byte()?;
    if s < m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_bleb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_byte()?;
    if s <= m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_bgtb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_byte()?;
    if s > m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_bgeb(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let s = vm.src_byte()?;
    let m = vm.mid_byte()?;
    if s >= m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

// String comparisons

fn get_str_pair_src_mid<'a>(vm: &'a VmState<'_>) -> Result<(&'a str, &'a str), ExecError> {
    let s_id = vm.read_word_at(vm.src, vm.imm_src)? as heap::HeapId;
    let m_id = vm.read_word_at(vm.mid, vm.imm_mid)? as heap::HeapId;
    let s = vm.heap.get_string(s_id).unwrap_or("");
    let m = vm.heap.get_string(m_id).unwrap_or("");
    Ok((s, m))
}

pub(crate) fn op_beqc(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let (s, m) = get_str_pair_src_mid(vm)?;
    if s == m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_bnec(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let (s, m) = get_str_pair_src_mid(vm)?;
    if s != m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_bltc(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let (s, m) = get_str_pair_src_mid(vm)?;
    if s < m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_blec(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let (s, m) = get_str_pair_src_mid(vm)?;
    if s <= m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_bgtc(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let (s, m) = get_str_pair_src_mid(vm)?;
    if s > m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

pub(crate) fn op_bgec(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let (s, m) = get_str_pair_src_mid(vm)?;
    if s >= m {
        vm.next_pc = vm.dst_word()? as usize;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use ricevm_core::{
        Header, Instruction, MiddleOperand, Module, Opcode, Operand, PointerMap, RuntimeFlags,
        TypeDescriptor, XMAGIC,
    };

    use super::*;
    use crate::address::AddrTarget;

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
            name: "compare_test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        }
    }

    const BRANCH_TARGET: i32 = 42;

    /// Helper: set up vm for a word comparison with immediates.
    fn setup_word_cmp(vm: &mut VmState<'_>, src_val: i32, mid_val: i32) {
        vm.src = AddrTarget::Immediate;
        vm.imm_src = src_val;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = mid_val;
        vm.dst = AddrTarget::Immediate;
        vm.imm_dst = BRANCH_TARGET;
    }

    // --- beqw ---

    #[test]
    fn beqw_branches_when_equal() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let orig_pc = vm.next_pc;
        setup_word_cmp(&mut vm, 5, 5);
        op_beqw(&mut vm).expect("beqw should succeed");
        assert_eq!(vm.next_pc, BRANCH_TARGET as usize);
        assert_ne!(vm.next_pc, orig_pc);
    }

    #[test]
    fn beqw_does_not_branch_when_not_equal() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let orig_pc = vm.next_pc;
        setup_word_cmp(&mut vm, 5, 10);
        op_beqw(&mut vm).expect("beqw should succeed");
        assert_eq!(vm.next_pc, orig_pc);
    }

    #[test]
    fn beqw_zero_equals_zero() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        setup_word_cmp(&mut vm, 0, 0);
        op_beqw(&mut vm).expect("beqw should succeed");
        assert_eq!(vm.next_pc, BRANCH_TARGET as usize);
    }

    #[test]
    fn beqw_negative_one() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        setup_word_cmp(&mut vm, -1, -1);
        op_beqw(&mut vm).expect("beqw should succeed");
        assert_eq!(vm.next_pc, BRANCH_TARGET as usize);
    }

    // --- bnew ---

    #[test]
    fn bnew_branches_when_not_equal() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        setup_word_cmp(&mut vm, 1, 2);
        op_bnew(&mut vm).expect("bnew should succeed");
        assert_eq!(vm.next_pc, BRANCH_TARGET as usize);
    }

    #[test]
    fn bnew_does_not_branch_when_equal() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let orig_pc = vm.next_pc;
        setup_word_cmp(&mut vm, 7, 7);
        op_bnew(&mut vm).expect("bnew should succeed");
        assert_eq!(vm.next_pc, orig_pc);
    }

    // --- bltw ---

    #[test]
    fn bltw_branches_when_less() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        setup_word_cmp(&mut vm, 3, 10);
        op_bltw(&mut vm).expect("bltw should succeed");
        assert_eq!(vm.next_pc, BRANCH_TARGET as usize);
    }

    #[test]
    fn bltw_does_not_branch_when_equal() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let orig_pc = vm.next_pc;
        setup_word_cmp(&mut vm, 5, 5);
        op_bltw(&mut vm).expect("bltw should succeed");
        assert_eq!(vm.next_pc, orig_pc);
    }

    #[test]
    fn bltw_does_not_branch_when_greater() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let orig_pc = vm.next_pc;
        setup_word_cmp(&mut vm, 10, 3);
        op_bltw(&mut vm).expect("bltw should succeed");
        assert_eq!(vm.next_pc, orig_pc);
    }

    #[test]
    fn bltw_negative_less_than_zero() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        setup_word_cmp(&mut vm, -1, 0);
        op_bltw(&mut vm).expect("bltw should succeed");
        assert_eq!(vm.next_pc, BRANCH_TARGET as usize);
    }

    #[test]
    fn bltw_min_less_than_max() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        setup_word_cmp(&mut vm, i32::MIN, i32::MAX);
        op_bltw(&mut vm).expect("bltw should succeed");
        assert_eq!(vm.next_pc, BRANCH_TARGET as usize);
    }

    // --- bgtw ---

    #[test]
    fn bgtw_branches_when_greater() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        setup_word_cmp(&mut vm, 10, 3);
        op_bgtw(&mut vm).expect("bgtw should succeed");
        assert_eq!(vm.next_pc, BRANCH_TARGET as usize);
    }

    #[test]
    fn bgtw_does_not_branch_when_less() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let orig_pc = vm.next_pc;
        setup_word_cmp(&mut vm, 3, 10);
        op_bgtw(&mut vm).expect("bgtw should succeed");
        assert_eq!(vm.next_pc, orig_pc);
    }

    #[test]
    fn bgtw_max_greater_than_min() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        setup_word_cmp(&mut vm, i32::MAX, i32::MIN);
        op_bgtw(&mut vm).expect("bgtw should succeed");
        assert_eq!(vm.next_pc, BRANCH_TARGET as usize);
    }

    // --- blew ---

    #[test]
    fn blew_branches_when_less() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        setup_word_cmp(&mut vm, 3, 10);
        op_blew(&mut vm).expect("blew should succeed");
        assert_eq!(vm.next_pc, BRANCH_TARGET as usize);
    }

    #[test]
    fn blew_branches_when_equal() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        setup_word_cmp(&mut vm, 5, 5);
        op_blew(&mut vm).expect("blew should succeed");
        assert_eq!(vm.next_pc, BRANCH_TARGET as usize);
    }

    #[test]
    fn blew_does_not_branch_when_greater() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let orig_pc = vm.next_pc;
        setup_word_cmp(&mut vm, 10, 3);
        op_blew(&mut vm).expect("blew should succeed");
        assert_eq!(vm.next_pc, orig_pc);
    }

    // --- bgew ---

    #[test]
    fn bgew_branches_when_greater() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        setup_word_cmp(&mut vm, 10, 3);
        op_bgew(&mut vm).expect("bgew should succeed");
        assert_eq!(vm.next_pc, BRANCH_TARGET as usize);
    }

    #[test]
    fn bgew_branches_when_equal() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        setup_word_cmp(&mut vm, 5, 5);
        op_bgew(&mut vm).expect("bgew should succeed");
        assert_eq!(vm.next_pc, BRANCH_TARGET as usize);
    }

    #[test]
    fn bgew_does_not_branch_when_less() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let orig_pc = vm.next_pc;
        setup_word_cmp(&mut vm, 3, 10);
        op_bgew(&mut vm).expect("bgew should succeed");
        assert_eq!(vm.next_pc, orig_pc);
    }

    // --- byte comparisons ---

    #[test]
    fn beqb_branches_when_equal() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        vm.src = AddrTarget::Immediate;
        vm.imm_src = 0xFF;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 0xFF;
        vm.dst = AddrTarget::Immediate;
        vm.imm_dst = BRANCH_TARGET;
        op_beqb(&mut vm).expect("beqb should succeed");
        assert_eq!(vm.next_pc, BRANCH_TARGET as usize);
    }

    #[test]
    fn bltb_branches_when_less() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        vm.src = AddrTarget::Immediate;
        vm.imm_src = 10;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 200;
        vm.dst = AddrTarget::Immediate;
        vm.imm_dst = BRANCH_TARGET;
        op_bltb(&mut vm).expect("bltb should succeed");
        assert_eq!(vm.next_pc, BRANCH_TARGET as usize);
    }

    // --- string comparisons ---

    #[test]
    fn beqc_branches_for_equal_strings() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let s1 = vm.heap.alloc(0, heap::HeapData::Str("hello".to_string()));
        let s2 = vm.heap.alloc(0, heap::HeapData::Str("hello".to_string()));
        vm.src = AddrTarget::Immediate;
        vm.imm_src = s1 as i32;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = s2 as i32;
        vm.dst = AddrTarget::Immediate;
        vm.imm_dst = BRANCH_TARGET;
        op_beqc(&mut vm).expect("beqc should succeed");
        assert_eq!(vm.next_pc, BRANCH_TARGET as usize);
    }

    #[test]
    fn bnec_branches_for_different_strings() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let s1 = vm.heap.alloc(0, heap::HeapData::Str("abc".to_string()));
        let s2 = vm.heap.alloc(0, heap::HeapData::Str("def".to_string()));
        vm.src = AddrTarget::Immediate;
        vm.imm_src = s1 as i32;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = s2 as i32;
        vm.dst = AddrTarget::Immediate;
        vm.imm_dst = BRANCH_TARGET;
        op_bnec(&mut vm).expect("bnec should succeed");
        assert_eq!(vm.next_pc, BRANCH_TARGET as usize);
    }

    #[test]
    fn bltc_branches_for_lexicographic_order() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let s1 = vm.heap.alloc(0, heap::HeapData::Str("abc".to_string()));
        let s2 = vm.heap.alloc(0, heap::HeapData::Str("xyz".to_string()));
        vm.src = AddrTarget::Immediate;
        vm.imm_src = s1 as i32;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = s2 as i32;
        vm.dst = AddrTarget::Immediate;
        vm.imm_dst = BRANCH_TARGET;
        op_bltc(&mut vm).expect("bltc should succeed");
        assert_eq!(vm.next_pc, BRANCH_TARGET as usize);
    }

    #[test]
    fn bgec_does_not_branch_when_less() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let orig_pc = vm.next_pc;
        let s1 = vm.heap.alloc(0, heap::HeapData::Str("abc".to_string()));
        let s2 = vm.heap.alloc(0, heap::HeapData::Str("xyz".to_string()));
        vm.src = AddrTarget::Immediate;
        vm.imm_src = s1 as i32;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = s2 as i32;
        vm.dst = AddrTarget::Immediate;
        vm.imm_dst = BRANCH_TARGET;
        op_bgec(&mut vm).expect("bgec should succeed");
        assert_eq!(vm.next_pc, orig_pc);
    }

    // --- float comparisons (use frame for real values) ---

    #[test]
    fn beqf_branches_when_equal() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        crate::memory::write_real(&mut vm.frames.data, fp, 3.75);
        crate::memory::write_real(&mut vm.frames.data, fp + 8, 3.75);
        vm.src = AddrTarget::Frame(fp);
        vm.mid = AddrTarget::Frame(fp + 8);
        vm.dst = AddrTarget::Immediate;
        vm.imm_dst = BRANCH_TARGET;
        op_beqf(&mut vm).expect("beqf should succeed");
        assert_eq!(vm.next_pc, BRANCH_TARGET as usize);
    }

    #[test]
    fn bltf_branches_when_less() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        crate::memory::write_real(&mut vm.frames.data, fp, 1.0);
        crate::memory::write_real(&mut vm.frames.data, fp + 8, 2.0);
        vm.src = AddrTarget::Frame(fp);
        vm.mid = AddrTarget::Frame(fp + 8);
        vm.dst = AddrTarget::Immediate;
        vm.imm_dst = BRANCH_TARGET;
        op_bltf(&mut vm).expect("bltf should succeed");
        assert_eq!(vm.next_pc, BRANCH_TARGET as usize);
    }

    #[test]
    fn bnef_branches_when_not_equal() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();
        crate::memory::write_real(&mut vm.frames.data, fp, 1.0);
        crate::memory::write_real(&mut vm.frames.data, fp + 8, 2.0);
        vm.src = AddrTarget::Frame(fp);
        vm.mid = AddrTarget::Frame(fp + 8);
        vm.dst = AddrTarget::Immediate;
        vm.imm_dst = BRANCH_TARGET;
        op_bnef(&mut vm).expect("bnef should succeed");
        assert_eq!(vm.next_pc, BRANCH_TARGET as usize);
    }

    // --- truth tables driven through the dispatch table ---
    //
    // Every branch goes through `ops::dispatch` here rather than calling the
    // handler directly, so an arm wired to the wrong handler fails as loudly as
    // a wrong comparison. The reference is `libinterp/xec.c`, where each branch
    // reads `if(W(s) OP W(m)) JMP(d)`: the compared values are the source and
    // the middle operand, and the destination is the target pc.

    /// The pc a branch falls through to when it is not taken. The run loop sets
    /// `next_pc` to `pc + 1` before dispatch, and a branch that does nothing has
    /// to leave that value alone.
    const FALL_THROUGH: usize = 7;

    /// Whether each comparison branches when the source is less than, equal to,
    /// or greater than the middle operand. The rows are ordered eq, ne, lt, le,
    /// gt, and ge, matching every `BRANCH_FAMILY` array below. Each row is a
    /// distinct pattern, so no two comparisons can be confused for each other.
    const TAKEN: [[bool; 3]; 6] = [
        [false, true, false], // eq
        [true, false, true],  // ne
        [true, false, false], // lt
        [true, true, false],  // le
        [false, false, true], // gt
        [false, true, true],  // ge
    ];

    const CASE_NAMES: [&str; 3] = ["src < mid", "src == mid", "src > mid"];

    fn branch_inst(opcode: Opcode) -> Instruction {
        Instruction {
            opcode,
            source: Operand::UNUSED,
            middle: MiddleOperand::UNUSED,
            destination: Operand::UNUSED,
        }
    }

    /// Dispatch one branch and report the pc it leaves behind.
    fn run_branch(vm: &mut VmState<'_>, opcode: Opcode) -> usize {
        vm.dst = AddrTarget::Immediate;
        vm.imm_dst = BRANCH_TARGET;
        vm.next_pc = FALL_THROUGH;
        crate::ops::dispatch(vm, &branch_inst(opcode)).expect("a branch must not fault");
        vm.next_pc
    }

    /// Check one comparison family against `TAKEN`. `store` writes the operand
    /// pair for the given case index into the machine.
    fn check_family(
        family: [Opcode; 6],
        width: &str,
        mut store: impl FnMut(&mut VmState<'_>, usize),
    ) {
        let module = test_module();
        for (row, opcode) in family.iter().enumerate() {
            for (case, expected) in TAKEN[row].iter().enumerate() {
                let mut vm = VmState::new(&module).expect("vm init");
                store(&mut vm, case);
                let landed = run_branch(&mut vm, *opcode);
                let want = if *expected {
                    BRANCH_TARGET as usize
                } else {
                    FALL_THROUGH
                };
                assert_eq!(
                    landed,
                    want,
                    "{width} {opcode:?} with {}: expected pc {want}, landed on {landed}",
                    CASE_NAMES[case]
                );
            }
        }
    }

    const WORD_FAMILY: [Opcode; 6] = [
        Opcode::Beqw,
        Opcode::Bnew,
        Opcode::Bltw,
        Opcode::Blew,
        Opcode::Bgtw,
        Opcode::Bgew,
    ];
    const BYTE_FAMILY: [Opcode; 6] = [
        Opcode::Beqb,
        Opcode::Bneb,
        Opcode::Bltb,
        Opcode::Bleb,
        Opcode::Bgtb,
        Opcode::Bgeb,
    ];
    const BIG_FAMILY: [Opcode; 6] = [
        Opcode::Beql,
        Opcode::Bnel,
        Opcode::Bltl,
        Opcode::Blel,
        Opcode::Bgtl,
        Opcode::Bgel,
    ];
    const REAL_FAMILY: [Opcode; 6] = [
        Opcode::Beqf,
        Opcode::Bnef,
        Opcode::Bltf,
        Opcode::Blef,
        Opcode::Bgtf,
        Opcode::Bgef,
    ];
    const STRING_FAMILY: [Opcode; 6] = [
        Opcode::Beqc,
        Opcode::Bnec,
        Opcode::Bltc,
        Opcode::Blec,
        Opcode::Bgtc,
        Opcode::Bgec,
    ];

    /// Operand pairs that differ by one in each direction, plus an equal pair.
    /// One is the smallest difference there is, so a comparison that reads its
    /// operands the wrong way round cannot hide behind a wide margin.
    const WORD_CASES: [(i32, i32); 3] = [(4, 5), (5, 5), (5, 4)];

    #[test]
    fn word_branches_follow_the_comparison_they_name() {
        check_family(WORD_FAMILY, "word", |vm, case| {
            let (s, m) = WORD_CASES[case];
            vm.src = AddrTarget::Immediate;
            vm.imm_src = s;
            vm.mid = AddrTarget::Immediate;
            vm.imm_mid = m;
        });
    }

    #[test]
    fn byte_branches_follow_the_comparison_they_name() {
        check_family(BYTE_FAMILY, "byte", |vm, case| {
            let (s, m) = WORD_CASES[case];
            vm.src = AddrTarget::Immediate;
            vm.imm_src = s;
            vm.mid = AddrTarget::Immediate;
            vm.imm_mid = m;
        });
    }

    #[test]
    fn big_branches_follow_the_comparison_they_name() {
        // The operands go in frame slots because a big read of an immediate
        // sign-extends a word, which cannot reach the 64-bit range.
        check_family(BIG_FAMILY, "big", |vm, case| {
            let fp = vm.frames.current_data_offset();
            let (s, m) = WORD_CASES[case];
            crate::memory::write_big(&mut vm.frames.data, fp, s as i64);
            crate::memory::write_big(&mut vm.frames.data, fp + 8, m as i64);
            vm.src = AddrTarget::Frame(fp);
            vm.mid = AddrTarget::Frame(fp + 8);
        });
    }

    #[test]
    fn real_branches_follow_the_comparison_they_name() {
        // A real read of an immediate is always 0.0, so both operands live in
        // the frame.
        check_family(REAL_FAMILY, "real", |vm, case| {
            let fp = vm.frames.current_data_offset();
            let (s, m) = WORD_CASES[case];
            crate::memory::write_real(&mut vm.frames.data, fp, s as f64);
            crate::memory::write_real(&mut vm.frames.data, fp + 8, m as f64);
            vm.src = AddrTarget::Frame(fp);
            vm.mid = AddrTarget::Frame(fp + 8);
        });
    }

    #[test]
    fn string_branches_follow_the_comparison_they_name() {
        // "abc" and "abd" differ in their last character only, and the equal
        // case uses two separate objects so identity cannot stand in for
        // content. The reference compares code point by code point and then by
        // length (`stringcmp`, libinterp/string.c).
        check_family(STRING_FAMILY, "string", |vm, case| {
            let pairs = [("abc", "abd"), ("abc", "abc"), ("abd", "abc")];
            let (s, m) = pairs[case];
            let s_id = vm.heap.alloc(0, heap::HeapData::Str(s.to_string()));
            let m_id = vm.heap.alloc(0, heap::HeapData::Str(m.to_string()));
            vm.src = AddrTarget::Immediate;
            vm.imm_src = s_id as i32;
            vm.mid = AddrTarget::Immediate;
            vm.imm_mid = m_id as i32;
        });
    }

    /// Word comparison is signed. Comparing the two extremes as unsigned would
    /// reverse both answers, which is why they are checked in both directions.
    #[test]
    fn word_branches_compare_as_signed() {
        let module = test_module();
        for (s, m, taken) in [
            (i32::MIN, i32::MAX, true),
            (i32::MAX, i32::MIN, false),
            (-1, 0, true),
            (0, -1, false),
        ] {
            let mut vm = VmState::new(&module).expect("vm init");
            vm.src = AddrTarget::Immediate;
            vm.imm_src = s;
            vm.mid = AddrTarget::Immediate;
            vm.imm_mid = m;
            let landed = run_branch(&mut vm, Opcode::Bltw);
            let want = if taken {
                BRANCH_TARGET as usize
            } else {
                FALL_THROUGH
            };
            assert_eq!(landed, want, "bltw {s} < {m}");
        }
    }

    /// Byte comparison is unsigned: `B(s)` is a `BYTE` in the reference, so 255
    /// is above 0, not below it the way a sign-extended byte would be.
    #[test]
    fn byte_branches_compare_as_unsigned() {
        let module = test_module();
        for (opcode, s, m, taken) in [
            (Opcode::Bgtb, 0xFF, 0x00, true),
            (Opcode::Bltb, 0xFF, 0x00, false),
            (Opcode::Bltb, 0x00, 0xFF, true),
            (Opcode::Beqb, 0xFF, 0xFF, true),
        ] {
            let mut vm = VmState::new(&module).expect("vm init");
            vm.src = AddrTarget::Immediate;
            vm.imm_src = s;
            vm.mid = AddrTarget::Immediate;
            vm.imm_mid = m;
            let landed = run_branch(&mut vm, opcode);
            let want = if taken {
                BRANCH_TARGET as usize
            } else {
                FALL_THROUGH
            };
            assert_eq!(landed, want, "{opcode:?} {s:#x} against {m:#x}");
        }
    }

    /// A big comparison has to read all 64 bits. Values that agree in their low
    /// word would compare equal if only a word were read.
    #[test]
    fn big_branches_compare_all_sixty_four_bits() {
        let module = test_module();
        let cases = [
            (Opcode::Bgtl, 1_i64 << 32, 0_i64, true),
            (Opcode::Beql, 1_i64 << 32, 0_i64, false),
            (Opcode::Bltl, i64::MIN, i64::MAX, true),
            (Opcode::Bgtl, i64::MAX, i64::MIN, true),
            (Opcode::Beql, i64::MIN, i64::MIN, true),
        ];
        for (opcode, s, m, taken) in cases {
            let mut vm = VmState::new(&module).expect("vm init");
            let fp = vm.frames.current_data_offset();
            crate::memory::write_big(&mut vm.frames.data, fp, s);
            crate::memory::write_big(&mut vm.frames.data, fp + 8, m);
            vm.src = AddrTarget::Frame(fp);
            vm.mid = AddrTarget::Frame(fp + 8);
            let landed = run_branch(&mut vm, opcode);
            let want = if taken {
                BRANCH_TARGET as usize
            } else {
                FALL_THROUGH
            };
            assert_eq!(landed, want, "{opcode:?} {s} against {m}");
        }
    }

    /// NaN is unordered, so in C every comparison against it is false except
    /// `!=`, which is true. The reference branches are plain C comparisons
    /// (`OP(beqf) { if(F(s) == F(m)) JMP(d); }`, libinterp/xec.c), so the same
    /// has to hold here.
    #[test]
    fn real_branches_treat_nan_as_unordered() {
        let module = test_module();
        for (opcode, taken) in [
            (Opcode::Beqf, false),
            (Opcode::Bnef, true),
            (Opcode::Bltf, false),
            (Opcode::Blef, false),
            (Opcode::Bgtf, false),
            (Opcode::Bgef, false),
        ] {
            let mut vm = VmState::new(&module).expect("vm init");
            let fp = vm.frames.current_data_offset();
            crate::memory::write_real(&mut vm.frames.data, fp, f64::NAN);
            crate::memory::write_real(&mut vm.frames.data, fp + 8, 1.0);
            vm.src = AddrTarget::Frame(fp);
            vm.mid = AddrTarget::Frame(fp + 8);
            let landed = run_branch(&mut vm, opcode);
            let want = if taken {
                BRANCH_TARGET as usize
            } else {
                FALL_THROUGH
            };
            assert_eq!(landed, want, "{opcode:?} against NaN");
        }
    }

    /// Negative zero equals positive zero in IEEE 754, and the smallest
    /// representable difference is still a difference.
    #[test]
    fn real_branches_handle_zero_and_tiny_differences() {
        let module = test_module();
        let cases = [
            (Opcode::Beqf, -0.0_f64, 0.0_f64, true),
            (Opcode::Bltf, -0.0_f64, 0.0_f64, false),
            (Opcode::Bltf, 1.0_f64, 1.0 + f64::EPSILON, true),
            (Opcode::Beqf, 1.0_f64, 1.0 + f64::EPSILON, false),
            (Opcode::Bgtf, f64::INFINITY, f64::MAX, true),
            (Opcode::Bltf, f64::NEG_INFINITY, f64::MIN, true),
        ];
        for (opcode, s, m, taken) in cases {
            let mut vm = VmState::new(&module).expect("vm init");
            let fp = vm.frames.current_data_offset();
            crate::memory::write_real(&mut vm.frames.data, fp, s);
            crate::memory::write_real(&mut vm.frames.data, fp + 8, m);
            vm.src = AddrTarget::Frame(fp);
            vm.mid = AddrTarget::Frame(fp + 8);
            let landed = run_branch(&mut vm, opcode);
            let want = if taken {
                BRANCH_TARGET as usize
            } else {
                FALL_THROUGH
            };
            assert_eq!(landed, want, "{opcode:?} {s} against {m}");
        }
    }

    /// `stringcmp` substitutes the empty string for a nil operand, so a nil
    /// string equals "" and sorts below any non-empty string
    /// (libinterp/string.c).
    #[test]
    fn string_branches_treat_nil_as_the_empty_string() {
        let module = test_module();
        let cases = [
            (Opcode::Beqc, None, Some(""), true),
            (Opcode::Beqc, None, None, true),
            (Opcode::Bltc, None, Some("a"), true),
            (Opcode::Bgtc, Some("a"), None, true),
            (Opcode::Bnec, None, Some("a"), true),
        ];
        for (opcode, s, m, taken) in cases {
            let mut vm = VmState::new(&module).expect("vm init");
            let mut id_of = |vm: &mut VmState<'_>, text: Option<&str>| match text {
                None => heap::NIL,
                Some(t) => vm.heap.alloc(0, heap::HeapData::Str(t.to_string())),
            };
            let s_id = id_of(&mut vm, s);
            let m_id = id_of(&mut vm, m);
            vm.src = AddrTarget::Immediate;
            vm.imm_src = s_id as i32;
            vm.mid = AddrTarget::Immediate;
            vm.imm_mid = m_id as i32;
            let landed = run_branch(&mut vm, opcode);
            let want = if taken {
                BRANCH_TARGET as usize
            } else {
                FALL_THROUGH
            };
            assert_eq!(landed, want, "{opcode:?} {s:?} against {m:?}");
        }
    }

    /// A prefix sorts below the longer string it starts, which is the `n1 - n2`
    /// tail of `stringcmp` (libinterp/string.c), and comparison runs over code
    /// points rather than bytes. UTF-8 keeps byte order and code point order
    /// the same, so 'a' stays below 'ä' either way.
    #[test]
    fn string_branches_order_by_code_point_then_length() {
        let module = test_module();
        let cases = [
            (Opcode::Bltc, "abc", "abcd", true),
            (Opcode::Bgtc, "abcd", "abc", true),
            (Opcode::Bltc, "", "a", true),
            (Opcode::Bltc, "a", "\u{00E4}", true),
            (Opcode::Bgtc, "\u{00E4}", "a", true),
            (Opcode::Bltc, "\u{00E4}", "\u{4E00}", true),
        ];
        for (opcode, s, m, taken) in cases {
            let mut vm = VmState::new(&module).expect("vm init");
            let s_id = vm.heap.alloc(0, heap::HeapData::Str(s.to_string()));
            let m_id = vm.heap.alloc(0, heap::HeapData::Str(m.to_string()));
            vm.src = AddrTarget::Immediate;
            vm.imm_src = s_id as i32;
            vm.mid = AddrTarget::Immediate;
            vm.imm_mid = m_id as i32;
            let landed = run_branch(&mut vm, opcode);
            let want = if taken {
                BRANCH_TARGET as usize
            } else {
                FALL_THROUGH
            };
            assert_eq!(landed, want, "{opcode:?} {s:?} against {m:?}");
        }
    }

    /// Pointer equality has no opcode of its own: the compiler emits the word
    /// branches over the pointer slots, so a heap id compares as a word and nil
    /// is zero.
    #[test]
    fn pointer_slots_compare_through_the_word_branches() {
        // Which object each slot names. Two distinct objects hold identical
        // text, which shows that the comparison is by heap id and not by value.
        #[derive(Copy, Clone, Debug)]
        enum Slot {
            First,
            Second,
            Nil,
        }
        let module = test_module();
        let cases = [
            (Opcode::Beqw, Slot::First, Slot::First, true),
            (Opcode::Beqw, Slot::First, Slot::Second, false),
            (Opcode::Bnew, Slot::First, Slot::Second, true),
            (Opcode::Beqw, Slot::Nil, Slot::Nil, true),
            (Opcode::Bnew, Slot::First, Slot::Nil, true),
            (Opcode::Beqw, Slot::First, Slot::Nil, false),
        ];
        for (opcode, src_slot, mid_slot, taken) in cases {
            let mut vm = VmState::new(&module).expect("vm init");
            let fp = vm.frames.current_data_offset();
            let first = vm.heap.alloc(0, heap::HeapData::Str("object".to_string()));
            let second = vm.heap.alloc(0, heap::HeapData::Str("object".to_string()));
            let id_of = |slot: Slot| match slot {
                Slot::First => first,
                Slot::Second => second,
                Slot::Nil => heap::NIL,
            };
            let src_id = id_of(src_slot);
            let mid_id = id_of(mid_slot);
            crate::memory::write_word(&mut vm.frames.data, fp, src_id as i32);
            crate::memory::write_word(&mut vm.frames.data, fp + 4, mid_id as i32);
            vm.src = AddrTarget::Frame(fp);
            vm.mid = AddrTarget::Frame(fp + 4);
            let landed = run_branch(&mut vm, opcode);
            let want = if taken {
                BRANCH_TARGET as usize
            } else {
                FALL_THROUGH
            };
            assert_eq!(
                landed, want,
                "{opcode:?} {src_slot:?} against {mid_slot:?} (ids {src_id} and {mid_id})"
            );
        }
    }

    /// A branch that is not taken must leave the pc exactly where the run loop
    /// put it, and a branch that is taken must land on the destination operand
    /// rather than on the middle one. Reading the middle operand as the target
    /// is the mistake this rules out: it would send the thread to pc 5 below.
    #[test]
    fn a_taken_branch_lands_on_the_destination_operand() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        vm.src = AddrTarget::Immediate;
        vm.imm_src = 5;
        vm.mid = AddrTarget::Immediate;
        vm.imm_mid = 5;
        vm.dst = AddrTarget::Immediate;
        vm.imm_dst = BRANCH_TARGET;
        vm.next_pc = FALL_THROUGH;
        crate::ops::dispatch(&mut vm, &branch_inst(Opcode::Beqw)).expect("beqw should succeed");
        assert_eq!(vm.next_pc, BRANCH_TARGET as usize);
        assert_ne!(vm.next_pc, vm.imm_mid as usize);
    }
}
