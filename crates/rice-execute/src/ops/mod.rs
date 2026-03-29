pub(crate) mod arith;
pub(crate) mod big;
pub(crate) mod compare;
pub(crate) mod control;
pub(crate) mod convert;
pub(crate) mod data_move;
pub(crate) mod float;

use ricevm_core::{ExecError, Instruction, Opcode};

use crate::vm::VmState;

pub(crate) fn dispatch(vm: &mut VmState<'_>, inst: &Instruction) -> Result<(), ExecError> {
    match inst.opcode {
        // Control flow
        Opcode::Nop => Ok(()),
        Opcode::Exit => control::op_exit(vm),
        Opcode::Jmp => control::op_jmp(vm),
        Opcode::Frame => control::op_frame(vm),
        Opcode::Call => control::op_call(vm),
        Opcode::Ret => control::op_ret(vm),

        // Data movement
        Opcode::Movw => data_move::op_movw(vm),
        Opcode::Movb => data_move::op_movb(vm),
        Opcode::Movf => data_move::op_movf(vm),
        Opcode::Movl => data_move::op_movl(vm),

        // Word arithmetic
        Opcode::Addw => arith::op_addw(vm),
        Opcode::Subw => arith::op_subw(vm),
        Opcode::Mulw => arith::op_mulw(vm),
        Opcode::Divw => arith::op_divw(vm),
        Opcode::Modw => arith::op_modw(vm),

        // Byte arithmetic
        Opcode::Addb => arith::op_addb(vm),
        Opcode::Subb => arith::op_subb(vm),
        Opcode::Mulb => arith::op_mulb(vm),
        Opcode::Divb => arith::op_divb(vm),
        Opcode::Modb => arith::op_modb(vm),

        // Word bitwise
        Opcode::Andw => arith::op_andw(vm),
        Opcode::Orw => arith::op_orw(vm),
        Opcode::Xorw => arith::op_xorw(vm),
        Opcode::Shlw => arith::op_shlw(vm),
        Opcode::Shrw => arith::op_shrw(vm),
        Opcode::Lsrw => arith::op_lsrw(vm),

        // Float arithmetic
        Opcode::Addf => float::op_addf(vm),
        Opcode::Subf => float::op_subf(vm),
        Opcode::Mulf => float::op_mulf(vm),
        Opcode::Divf => float::op_divf(vm),
        Opcode::Negf => float::op_negf(vm),

        // Big arithmetic
        Opcode::Addl => big::op_addl(vm),
        Opcode::Subl => big::op_subl(vm),
        Opcode::Mull => big::op_mull(vm),
        Opcode::Divl => big::op_divl(vm),
        Opcode::Modl => big::op_modl(vm),

        // Word comparisons
        Opcode::Beqw => compare::op_beqw(vm),
        Opcode::Bnew => compare::op_bnew(vm),
        Opcode::Bltw => compare::op_bltw(vm),
        Opcode::Blew => compare::op_blew(vm),
        Opcode::Bgtw => compare::op_bgtw(vm),
        Opcode::Bgew => compare::op_bgew(vm),

        // Float comparisons
        Opcode::Beqf => compare::op_beqf(vm),
        Opcode::Bnef => compare::op_bnef(vm),
        Opcode::Bltf => compare::op_bltf(vm),
        Opcode::Blef => compare::op_blef(vm),
        Opcode::Bgtf => compare::op_bgtf(vm),
        Opcode::Bgef => compare::op_bgef(vm),

        // Big comparisons
        Opcode::Beql => compare::op_beql(vm),
        Opcode::Bnel => compare::op_bnel(vm),
        Opcode::Bltl => compare::op_bltl(vm),
        Opcode::Blel => compare::op_blel(vm),
        Opcode::Bgtl => compare::op_bgtl(vm),
        Opcode::Bgel => compare::op_bgel(vm),

        // Byte comparisons
        Opcode::Beqb => compare::op_beqb(vm),
        Opcode::Bneb => compare::op_bneb(vm),
        Opcode::Bltb => compare::op_bltb(vm),
        Opcode::Bleb => compare::op_bleb(vm),
        Opcode::Bgtb => compare::op_bgtb(vm),
        Opcode::Bgeb => compare::op_bgeb(vm),

        // Type conversions
        Opcode::Cvtbw => convert::op_cvtbw(vm),
        Opcode::Cvtwb => convert::op_cvtwb(vm),
        Opcode::Cvtfw => convert::op_cvtfw(vm),
        Opcode::Cvtwf => convert::op_cvtwf(vm),
        Opcode::Cvtwl => convert::op_cvtwl(vm),
        Opcode::Cvtlw => convert::op_cvtlw(vm),
        Opcode::Cvtlf => convert::op_cvtlf(vm),
        Opcode::Cvtfl => convert::op_cvtfl(vm),

        _ => Err(ExecError::Other(format!(
            "unimplemented opcode: {:?}",
            inst.opcode
        ))),
    }
}
