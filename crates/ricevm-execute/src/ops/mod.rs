pub(crate) mod arith;
pub(crate) mod big;
pub(crate) mod compare;
pub(crate) mod concurrency;
pub(crate) mod control;
pub(crate) mod convert;
pub(crate) mod data_move;
pub(crate) mod fixedpoint;
pub(crate) mod float;
pub(crate) mod heap;
pub(crate) mod list;
pub(crate) mod pointer;
pub(crate) mod string;

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
        Opcode::Load => control::op_load(vm),
        Opcode::Mframe => control::op_mframe(vm),
        Opcode::Mcall => control::op_mcall(vm),

        // Data movement
        Opcode::Movw => data_move::op_movw(vm),
        Opcode::Movb => data_move::op_movb(vm),
        Opcode::Movf => data_move::op_movf(vm),
        Opcode::Movl => data_move::op_movl(vm),
        Opcode::Movp => pointer::op_movp(vm),
        Opcode::Movm => data_move::op_movm(vm),
        Opcode::Movmp => data_move::op_movmp(vm),
        Opcode::Movpc => data_move::op_movpc(vm),

        // Heap allocation
        Opcode::New => heap::op_new(vm),
        Opcode::Newz => heap::op_newz(vm),
        Opcode::Newa => heap::op_newa(vm),
        Opcode::Newaz => heap::op_newaz(vm),
        Opcode::Mnewz => heap::op_mnewz(vm),

        // Channel allocation
        Opcode::Newcb => heap::op_newcb(vm),
        Opcode::Newcw => heap::op_newcw(vm),
        Opcode::Newcf => heap::op_newcf(vm),
        Opcode::Newcp => heap::op_newcp(vm),
        Opcode::Newcm => heap::op_newcm(vm),
        Opcode::Newcmp => heap::op_newcmp(vm),
        Opcode::Newcl => heap::op_newcl(vm),

        // Pointer operations
        Opcode::Lea => pointer::op_lea(vm),
        Opcode::Indx => pointer::op_indx(vm),
        Opcode::Indw => pointer::op_indw(vm),
        Opcode::Indf => pointer::op_indf(vm),
        Opcode::Indb => pointer::op_indb(vm),
        Opcode::Indl => pointer::op_indl(vm),
        Opcode::Lena => pointer::op_lena(vm),

        // List operations
        Opcode::Consb => list::op_consb(vm),
        Opcode::Consw => list::op_consw(vm),
        Opcode::Consp => list::op_consp(vm),
        Opcode::Consf => list::op_consf(vm),
        Opcode::Consm => list::op_consm(vm),
        Opcode::Consmp => list::op_consmp(vm),
        Opcode::Consl => list::op_consl(vm),
        Opcode::Headb => list::op_headb(vm),
        Opcode::Headw => list::op_headw(vm),
        Opcode::Headp => list::op_headp(vm),
        Opcode::Headf => list::op_headf(vm),
        Opcode::Headm => list::op_headm(vm),
        Opcode::Headmp => list::op_headmp(vm),
        Opcode::Headl => list::op_headl(vm),
        Opcode::Tail => list::op_tail(vm),

        // String operations
        Opcode::Lenc => string::op_lenc(vm),
        Opcode::Indc => string::op_indc(vm),
        Opcode::Insc => string::op_insc(vm),
        Opcode::Addc => string::op_addc(vm),
        Opcode::Slicec => string::op_slicec(vm),
        Opcode::Cvtca => string::op_cvtca(vm),
        Opcode::Cvtac => string::op_cvtac(vm),
        Opcode::Lenl => string::op_lenl(vm),

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

        // Byte bitwise
        Opcode::Andb => arith::op_andb(vm),
        Opcode::Orb => arith::op_orb(vm),
        Opcode::Xorb => arith::op_xorb(vm),
        Opcode::Shlb => arith::op_shlb(vm),
        Opcode::Shrb => arith::op_shrb(vm),

        // Big bitwise and shift
        Opcode::Andl => arith::op_andl(vm),
        Opcode::Orl => arith::op_orl(vm),
        Opcode::Xorl => arith::op_xorl(vm),
        Opcode::Shll => arith::op_shll(vm),
        Opcode::Shrl => arith::op_shrl(vm),
        Opcode::Lsrl => arith::op_lsrl(vm),

        // Exponentiation
        Opcode::Expw => arith::op_expw(vm),
        Opcode::Expl => arith::op_expl(vm),
        Opcode::Expf => arith::op_expf(vm),

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

        // String comparisons
        Opcode::Beqc => compare::op_beqc(vm),
        Opcode::Bnec => compare::op_bnec(vm),
        Opcode::Bltc => compare::op_bltc(vm),
        Opcode::Blec => compare::op_blec(vm),
        Opcode::Bgtc => compare::op_bgtc(vm),
        Opcode::Bgec => compare::op_bgec(vm),

        // Type conversions
        Opcode::Cvtbw => convert::op_cvtbw(vm),
        Opcode::Cvtwb => convert::op_cvtwb(vm),
        Opcode::Cvtfw => convert::op_cvtfw(vm),
        Opcode::Cvtwf => convert::op_cvtwf(vm),
        Opcode::Cvtwl => convert::op_cvtwl(vm),
        Opcode::Cvtlw => convert::op_cvtlw(vm),
        Opcode::Cvtlf => convert::op_cvtlf(vm),
        Opcode::Cvtfl => convert::op_cvtfl(vm),
        Opcode::Cvtwc => convert::op_cvtwc(vm),
        Opcode::Cvtcw => convert::op_cvtcw(vm),
        Opcode::Cvtfc => convert::op_cvtfc(vm),
        Opcode::Cvtcf => convert::op_cvtcf(vm),
        Opcode::Cvtlc => convert::op_cvtlc(vm),
        Opcode::Cvtcl => convert::op_cvtcl(vm),
        Opcode::Cvtws => convert::op_cvtws(vm),
        Opcode::Cvtsw => convert::op_cvtsw(vm),

        // Array slice
        Opcode::Slicea => pointer::op_slicea(vm),
        Opcode::Slicela => pointer::op_slicela(vm),

        // Additional conversions
        Opcode::Cvtrf => convert::op_cvtrf(vm),
        Opcode::Cvtfr => convert::op_cvtfr(vm),

        // Control flow (additional)
        Opcode::Goto => control::op_goto(vm),
        Opcode::Casew => control::op_casew(vm),
        Opcode::Casec => control::op_casec(vm),
        Opcode::Casel => control::op_casel(vm),
        Opcode::Raise => control::op_raise(vm),
        Opcode::Runt => control::op_runt(vm),
        Opcode::Eclr => control::op_eclr(vm),
        Opcode::Brkpt => control::op_brkpt(vm),

        // Fixed-point arithmetic
        Opcode::Mulx => fixedpoint::op_mulx(vm),
        Opcode::Mulx0 => fixedpoint::op_mulx0(vm),
        Opcode::Mulx1 => fixedpoint::op_mulx1(vm),
        Opcode::Divx => fixedpoint::op_divx(vm),
        Opcode::Divx0 => fixedpoint::op_divx0(vm),
        Opcode::Divx1 => fixedpoint::op_divx1(vm),
        Opcode::Cvtxx => fixedpoint::op_cvtxx(vm),
        Opcode::Cvtxx0 => fixedpoint::op_cvtxx0(vm),
        Opcode::Cvtxx1 => fixedpoint::op_cvtxx1(vm),
        Opcode::Cvtfx => fixedpoint::op_cvtfx(vm),
        Opcode::Cvtxf => fixedpoint::op_cvtxf(vm),

        // Concurrency
        Opcode::Spawn => concurrency::op_spawn(vm),
        Opcode::Mspawn => concurrency::op_mspawn(vm),
        Opcode::Send => concurrency::op_send(vm),
        Opcode::Recv => concurrency::op_recv(vm),
        Opcode::Alt => concurrency::op_alt(vm),
        Opcode::Nbalt => concurrency::op_nbalt(vm),

        // Misc
        Opcode::Tcmp => data_move::op_tcmp(vm),
        Opcode::Self_ => data_move::op_self_(vm),
    }
}
