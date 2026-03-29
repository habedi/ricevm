//! Rice VM Execution Engine
//!
//! Takes a loaded [`Module`] and runs it from its entry point.

mod address;
mod data;
mod frame;
mod memory;
mod ops;
mod vm;

use ricevm_core::{ExecError, Module};

/// Execute a loaded Dis module from its entry point.
///
/// Returns `Ok(())` on clean exit, or an [`ExecError`] describing the failure.
pub fn execute(module: &Module) -> Result<(), ExecError> {
    let entry_pc = module.header.entry_pc;
    if entry_pc < 0 || entry_pc as usize >= module.code.len() {
        return Err(ExecError::InvalidPc(entry_pc));
    }
    tracing::info!(
        name = %module.name,
        entry_pc = entry_pc,
        instructions = module.code.len(),
        "Executing module"
    );
    let mut state = vm::VmState::new(module)?;
    state.run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ricevm_core::*;

    fn make_header(code_size: i32, type_size: i32, entry_type: i32) -> Header {
        Header {
            magic: XMAGIC,
            signature: vec![],
            runtime_flags: RuntimeFlags(0),
            stack_extent: 0,
            code_size,
            data_size: 0,
            type_size,
            export_size: 0,
            entry_pc: 0,
            entry_type,
        }
    }

    fn make_inst(opcode: Opcode, src: Operand, mid: MiddleOperand, dst: Operand) -> Instruction {
        Instruction {
            opcode,
            source: src,
            middle: mid,
            destination: dst,
        }
    }

    fn fp(offset: i32) -> Operand {
        Operand {
            mode: AddressMode::OffsetIndirectFp,
            register1: offset,
            register2: 0,
        }
    }

    fn imm(val: i32) -> Operand {
        Operand {
            mode: AddressMode::Immediate,
            register1: val,
            register2: 0,
        }
    }

    fn mid_imm(val: i32) -> MiddleOperand {
        MiddleOperand {
            mode: MiddleMode::SmallImmediate,
            register1: val,
        }
    }

    fn mid_fp(offset: i32) -> MiddleOperand {
        MiddleOperand {
            mode: MiddleMode::SmallOffsetFp,
            register1: offset,
        }
    }

    fn type_desc(size: i32) -> TypeDescriptor {
        TypeDescriptor {
            id: 0,
            size,
            pointer_map: PointerMap { bytes: vec![] },
        }
    }

    #[test]
    fn execute_exit_only() {
        let module = Module {
            header: make_header(1, 1, 0),
            code: vec![make_inst(
                Opcode::Exit,
                Operand::UNUSED,
                MiddleOperand::UNUSED,
                Operand::UNUSED,
            )],
            types: vec![type_desc(32)],
            data: vec![],
            name: "test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        };
        assert!(execute(&module).is_ok());
    }

    #[test]
    fn execute_movw_and_addw() {
        // movw $10, 0(fp)    -- fp[0] = 10
        // movw $20, 4(fp)    -- fp[4] = 20
        // addw 0(fp), 4(fp), 8(fp)  -- fp[8] = fp[0] + fp[4] = 30
        // exit
        let module = Module {
            header: make_header(4, 1, 0),
            code: vec![
                make_inst(Opcode::Movw, imm(10), MiddleOperand::UNUSED, fp(0)),
                make_inst(Opcode::Movw, imm(20), MiddleOperand::UNUSED, fp(4)),
                make_inst(Opcode::Addw, fp(0), mid_fp(4), fp(8)),
                make_inst(Opcode::Exit, Operand::UNUSED, MiddleOperand::UNUSED, Operand::UNUSED),
            ],
            types: vec![type_desc(32)],
            data: vec![],
            name: "test_add".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        };
        assert!(execute(&module).is_ok());
    }

    #[test]
    fn execute_branch_skip() {
        // movw $5, 0(fp)
        // movw $10, 4(fp)
        // beqw 0(fp), 3, 4(fp)  -- if fp[0] == fp[4], jump to 3 (skip exit)
        // exit                    -- should reach here since 5 != 10
        // movw $99, 0(fp)        -- should NOT reach here
        let module = Module {
            header: make_header(5, 1, 0),
            code: vec![
                make_inst(Opcode::Movw, imm(5), MiddleOperand::UNUSED, fp(0)),
                make_inst(Opcode::Movw, imm(10), MiddleOperand::UNUSED, fp(4)),
                make_inst(Opcode::Beqw, fp(0), mid_imm(4), fp(4)),
                make_inst(Opcode::Exit, Operand::UNUSED, MiddleOperand::UNUSED, Operand::UNUSED),
                make_inst(Opcode::Movw, imm(99), MiddleOperand::UNUSED, fp(0)),
            ],
            types: vec![type_desc(32)],
            data: vec![],
            name: "test_branch".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        };
        assert!(execute(&module).is_ok());
    }

    #[test]
    fn execute_division_by_zero() {
        // movw $0, 4(fp)    -- fp[4] = 0 (divisor)
        // divw $10, 4(fp), 0(fp)  -- should fail: 10 / 0
        let module = Module {
            header: make_header(2, 1, 0),
            code: vec![
                make_inst(Opcode::Movw, imm(0), MiddleOperand::UNUSED, fp(4)),
                make_inst(Opcode::Divw, imm(10), mid_fp(4), fp(0)),
            ],
            types: vec![type_desc(32)],
            data: vec![],
            name: "test_divzero".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        };
        let result = execute(&module);
        assert!(matches!(result, Err(ExecError::ThreadFault(_))));
    }

    #[test]
    fn invalid_entry_pc() {
        let mut module = Module {
            header: make_header(1, 1, 0),
            code: vec![make_inst(
                Opcode::Exit,
                Operand::UNUSED,
                MiddleOperand::UNUSED,
                Operand::UNUSED,
            )],
            types: vec![type_desc(32)],
            data: vec![],
            name: "test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        };
        module.header.entry_pc = 999;
        let result = execute(&module);
        assert!(matches!(result, Err(ExecError::InvalidPc(999))));
    }

    #[test]
    fn execute_jmp() {
        // jmp 2       -- jump to instruction 2 (skip the unreachable)
        // movw $99, 0(fp)  -- unreachable
        // exit
        let module = Module {
            header: make_header(3, 1, 0),
            code: vec![
                make_inst(Opcode::Jmp, Operand::UNUSED, MiddleOperand::UNUSED, imm(2)),
                make_inst(Opcode::Movw, imm(99), MiddleOperand::UNUSED, fp(0)),
                make_inst(Opcode::Exit, Operand::UNUSED, MiddleOperand::UNUSED, Operand::UNUSED),
            ],
            types: vec![type_desc(32)],
            data: vec![],
            name: "test_jmp".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        };
        assert!(execute(&module).is_ok());
    }

    #[test]
    fn execute_ret_from_entry() {
        // ret  -- return from entry function (sentinel pc = -1 → halt)
        let module = Module {
            header: make_header(1, 1, 0),
            code: vec![make_inst(
                Opcode::Ret,
                Operand::UNUSED,
                MiddleOperand::UNUSED,
                Operand::UNUSED,
            )],
            types: vec![type_desc(32)],
            data: vec![],
            name: "test_ret".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        };
        assert!(execute(&module).is_ok());
    }

    #[test]
    fn execute_type_conversion() {
        // movw $42, 0(fp)
        // cvtwf 0(fp), 8(fp)  -- fp[8] = 42.0 as f64
        // exit
        let module = Module {
            header: make_header(3, 1, 0),
            code: vec![
                make_inst(Opcode::Movw, imm(42), MiddleOperand::UNUSED, fp(0)),
                make_inst(Opcode::Cvtwf, fp(0), MiddleOperand::UNUSED, fp(8)),
                make_inst(Opcode::Exit, Operand::UNUSED, MiddleOperand::UNUSED, Operand::UNUSED),
            ],
            types: vec![type_desc(32)],
            data: vec![],
            name: "test_cvt".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        };
        assert!(execute(&module).is_ok());
    }
}
