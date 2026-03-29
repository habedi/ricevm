//! Rice VM Execution Engine
//!
//! Takes a loaded [`Module`] and runs it from its entry point.

mod address;
mod builtin;
mod data;
mod frame;
mod heap;
mod memory;
mod ops;
mod sys;
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
                make_inst(
                    Opcode::Exit,
                    Operand::UNUSED,
                    MiddleOperand::UNUSED,
                    Operand::UNUSED,
                ),
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
                make_inst(
                    Opcode::Exit,
                    Operand::UNUSED,
                    MiddleOperand::UNUSED,
                    Operand::UNUSED,
                ),
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
                make_inst(
                    Opcode::Exit,
                    Operand::UNUSED,
                    MiddleOperand::UNUSED,
                    Operand::UNUSED,
                ),
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
                make_inst(
                    Opcode::Exit,
                    Operand::UNUSED,
                    MiddleOperand::UNUSED,
                    Operand::UNUSED,
                ),
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

    fn mp(offset: i32) -> Operand {
        Operand {
            mode: AddressMode::OffsetIndirectMp,
            register1: offset,
            register2: 0,
        }
    }

    fn mid_none() -> MiddleOperand {
        MiddleOperand::UNUSED
    }

    fn none() -> Operand {
        Operand::UNUSED
    }

    fn exit_inst() -> Instruction {
        make_inst(Opcode::Exit, none(), mid_none(), none())
    }

    #[test]
    fn execute_string_length() {
        // MP has a string "hello" at offset 0 (as HeapId)
        // lenc 0(mp), 0(fp)  -- fp[0] = len("hello") = 5
        // exit
        let module = Module {
            header: Header {
                data_size: 4,
                ..make_header(2, 1, 0)
            },
            code: vec![
                make_inst(Opcode::Lenc, mp(0), mid_none(), fp(0)),
                exit_inst(),
            ],
            types: vec![type_desc(32)],
            data: vec![DataItem::String {
                offset: 0,
                value: "hello".to_string(),
            }],
            name: "test_lenc".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        };
        assert!(execute(&module).is_ok());
    }

    #[test]
    fn execute_string_concat() {
        // MP[0] = "hello " (HeapId), MP[4] = "world" (HeapId)
        // addc 4(mp), 0(mp), 0(fp)  -- fp[0] = "hello " + "world" = "hello world"
        // lenc 0(fp), 4(fp)          -- fp[4] = len("hello world") = 11
        // exit
        let module = Module {
            header: Header {
                data_size: 8,
                ..make_header(3, 1, 0)
            },
            code: vec![
                // addc: dst = mid + src → fp[0] = mp[0] + mp[4]
                make_inst(Opcode::Addc, mp(4), MiddleOperand {
                    mode: MiddleMode::SmallOffsetMp,
                    register1: 0,
                }, fp(0)),
                make_inst(Opcode::Lenc, fp(0), mid_none(), fp(4)),
                exit_inst(),
            ],
            types: vec![type_desc(32)],
            data: vec![
                DataItem::String {
                    offset: 0,
                    value: "hello ".to_string(),
                },
                DataItem::String {
                    offset: 4,
                    value: "world".to_string(),
                },
            ],
            name: "test_addc".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        };
        assert!(execute(&module).is_ok());
    }

    #[test]
    fn execute_heap_alloc_and_movp() {
        // new $0, 0(fp)    -- allocate record of type 0, store ptr at fp[0]
        // movp 0(fp), 4(fp) -- copy ptr to fp[4] (ref count should be 2)
        // exit
        let module = Module {
            header: make_header(3, 1, 0),
            code: vec![
                make_inst(Opcode::New, imm(0), mid_none(), fp(0)),
                make_inst(Opcode::Movp, fp(0), mid_none(), fp(4)),
                exit_inst(),
            ],
            types: vec![type_desc(32)],
            data: vec![],
            name: "test_new".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        };
        assert!(execute(&module).is_ok());
    }

    #[test]
    fn execute_load_sys_module() {
        // MP[0] = "$Sys" string
        // load 0(mp), $0, 0(fp)  -- load $Sys module, store ref at fp[0]
        // exit
        let module = Module {
            header: Header {
                data_size: 4,
                ..make_header(2, 1, 0)
            },
            code: vec![
                make_inst(Opcode::Load, mp(0), mid_imm(0), fp(0)),
                exit_inst(),
            ],
            types: vec![type_desc(32)],
            data: vec![DataItem::String {
                offset: 0,
                value: "$Sys".to_string(),
            }],
            name: "test_load".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        };
        assert!(execute(&module).is_ok());
    }

    #[test]
    fn execute_sys_print() {
        // MP[0] = "$Sys" path, MP[4] = "hello!\n" format string
        //
        // Instruction sequence:
        // 0: load 0(mp), $0, 0(fp)      -- fp[0] = $Sys module ref
        // 1: mframe 0(fp), $0, 4(fp)    -- fp[4] = pending frame for print
        // 2: movp 4(mp), 16(fp[4])      -- write format string ptr to pending frame
        //    (this uses double-indirect: fp[4] holds the pending frame offset,
        //     we need to write at offset 16 within it)
        //    For simplicity, we use a different approach:
        //    Store the format string HeapId using movw from mp to the pending frame
        //
        // Actually, writing to a pending frame requires double-indirect addressing:
        //   dst mode = OffsetDoubleIndirectFp, register1 = 4 (where frame ptr lives),
        //   register2 = 16 (offset within the frame for the format string)
        //
        // 3: mcall 4(fp), $0, 0(fp)     -- call print, auto-return
        // 4: exit

        let double_indirect_fp = |base_offset: i32, field_offset: i32| -> Operand {
            Operand {
                mode: AddressMode::OffsetDoubleIndirectFp,
                register1: base_offset,
                register2: field_offset,
            }
        };

        let module = Module {
            header: Header {
                data_size: 8,
                ..make_header(5, 2, 0)
            },
            code: vec![
                // 0: load $Sys
                make_inst(Opcode::Load, mp(0), mid_imm(0), fp(0)),
                // 1: mframe for print (func index 0)
                make_inst(Opcode::Mframe, fp(0), mid_imm(0), fp(4)),
                // 2: copy format string pointer into pending frame at offset 16
                // src = mp(4) which holds the HeapId of "hello!\n"
                // dst = double_indirect_fp(4, 16) = *(fp[4] + 16)
                make_inst(Opcode::Movw, mp(4), mid_none(), double_indirect_fp(4, 16)),
                // 3: mcall print
                make_inst(Opcode::Mcall, fp(4), mid_imm(0), fp(0)),
                // 4: exit
                exit_inst(),
            ],
            types: vec![type_desc(64), type_desc(64)],
            data: vec![
                DataItem::String {
                    offset: 0,
                    value: "$Sys".to_string(),
                },
                DataItem::String {
                    offset: 4,
                    value: "hello!\n".to_string(),
                },
            ],
            name: "test_print".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        };
        // This should print "hello!\n" to stdout and exit cleanly
        assert!(execute(&module).is_ok());
    }

    #[test]
    fn execute_load_nonexistent_module() {
        // load a module that doesn't exist → should set dst to nil (0)
        let module = Module {
            header: Header {
                data_size: 4,
                ..make_header(2, 1, 0)
            },
            code: vec![
                make_inst(Opcode::Load, mp(0), mid_imm(0), fp(0)),
                exit_inst(),
            ],
            types: vec![type_desc(32)],
            data: vec![DataItem::String {
                offset: 0,
                value: "$NonExistent".to_string(),
            }],
            name: "test_load_fail".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        };
        assert!(execute(&module).is_ok());
    }

    #[test]
    fn execute_list_consw_headw_tail() {
        // Build list [10, 20] and extract elements:
        // movw $0, 0(fp)          -- fp[0] = nil (empty list)
        // consw $20, 0(fp)        -- fp[0] = 20 :: nil
        // consw $10, 0(fp)        -- fp[0] = 10 :: 20 :: nil
        // headw 0(fp), 4(fp)      -- fp[4] = hd(fp[0]) = 10
        // tail 0(fp), 8(fp)       -- fp[8] = tl(fp[0]) = 20 :: nil
        // headw 8(fp), 12(fp)     -- fp[12] = hd(fp[8]) = 20
        // exit
        let module = Module {
            header: make_header(7, 1, 0),
            code: vec![
                make_inst(Opcode::Movw, imm(0), mid_none(), fp(0)),
                make_inst(Opcode::Consw, imm(20), mid_none(), fp(0)),
                make_inst(Opcode::Consw, imm(10), mid_none(), fp(0)),
                make_inst(Opcode::Headw, fp(0), mid_none(), fp(4)),
                make_inst(Opcode::Tail, fp(0), mid_none(), fp(8)),
                make_inst(Opcode::Headw, fp(8), mid_none(), fp(12)),
                exit_inst(),
            ],
            types: vec![type_desc(32)],
            data: vec![],
            name: "test_list".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        };
        assert!(execute(&module).is_ok());
    }

    #[test]
    fn execute_movm_block_copy() {
        // movw $42, 0(fp)         -- fp[0] = 42
        // movw $99, 4(fp)         -- fp[4] = 99
        // movm 0(fp), $8, 16(fp)  -- copy 8 bytes from fp[0..8] to fp[16..24]
        // exit
        let module = Module {
            header: make_header(4, 1, 0),
            code: vec![
                make_inst(Opcode::Movw, imm(42), mid_none(), fp(0)),
                make_inst(Opcode::Movw, imm(99), mid_none(), fp(4)),
                make_inst(Opcode::Movm, fp(0), mid_imm(8), fp(16)),
                exit_inst(),
            ],
            types: vec![type_desc(32)],
            data: vec![],
            name: "test_movm".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        };
        assert!(execute(&module).is_ok());
    }
}
