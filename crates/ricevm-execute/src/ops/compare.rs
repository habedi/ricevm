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
