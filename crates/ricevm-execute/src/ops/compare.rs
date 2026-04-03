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
        crate::memory::write_real(&mut vm.frames.data, fp, 3.14);
        crate::memory::write_real(&mut vm.frames.data, fp + 8, 3.14);
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
}
