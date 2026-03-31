//! Rice VM Execution Engine
//!
//! Takes a loaded [`Module`] and runs it from its entry point.

mod address;
mod builtin;
mod channel;
mod data;
mod debugger;
#[allow(unused_imports, unused_variables, dead_code, clippy::collapsible_if)]
mod draw;
mod filetab;
mod frame;
mod gc;
mod heap;
mod math;
mod memory;
mod ops;
mod scheduler;
mod sys;
#[allow(unused_imports, unused_variables, dead_code)]
mod tk;
mod vm;

use ricevm_core::{ExecError, Module};

/// Execute a loaded Dis module from its entry point.
///
/// Returns `Ok(())` on clean exit, or an [`ExecError`] describing the failure.
pub fn execute(module: &Module) -> Result<(), ExecError> {
    execute_with_args(module, Vec::new())
}

/// Execute a loaded Dis module with guest program arguments.
///
/// `args` are passed to the guest program's `init()` as the argv list.
/// The module name is automatically prepended as argv[0].
pub fn execute_with_args(module: &Module, args: Vec<String>) -> Result<(), ExecError> {
    let entry_pc = module.header.entry_pc;
    if entry_pc < 0 || entry_pc as usize >= module.code.len() {
        return Err(ExecError::InvalidPc(entry_pc));
    }
    tracing::info!(
        name = %module.name,
        entry_pc = entry_pc,
        instructions = module.code.len(),
        args = ?args,
        "Executing module"
    );
    let mut state = vm::VmState::with_args(module, args)?;
    state.run()
}

/// Run a loaded Dis module under the interactive debugger.
pub fn debug(module: &Module) -> Result<(), ExecError> {
    let entry_pc = module.header.entry_pc;
    if entry_pc < 0 || entry_pc as usize >= module.code.len() {
        return Err(ExecError::InvalidPc(entry_pc));
    }
    let mut dbg = debugger::Debugger::new(module)?;
    dbg.run_interactive()
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
            pointer_count: 0,
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
        // Division by zero now writes 0 to dst instead of faulting.
        // The module has no Exit instruction so it falls off the end with InvalidPc.
        assert!(matches!(result, Err(ExecError::InvalidPc(_))));
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
                make_inst(
                    Opcode::Addc,
                    mp(4),
                    MiddleOperand {
                        mode: MiddleMode::SmallOffsetMp,
                        register1: 0,
                    },
                    fp(0),
                ),
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
    fn execute_load_self_and_mcall_main_module() {
        let module = Module {
            header: Header {
                data_size: 8,
                ..make_header(6, 2, 0)
            },
            code: vec![
                make_inst(Opcode::Load, mp(0), mid_imm(0), fp(0)),
                make_inst(Opcode::Mframe, fp(0), mid_imm(0), fp(4)),
                make_inst(Opcode::Mcall, fp(4), mid_imm(0), fp(0)),
                exit_inst(),
                make_inst(Opcode::Movw, imm(42), mid_none(), mp(4)),
                make_inst(Opcode::Ret, none(), mid_none(), none()),
            ],
            types: vec![type_desc(64), type_desc(32)],
            data: vec![DataItem::String {
                offset: 0,
                value: "$self".to_string(),
            }],
            name: "test_load_self".to_string(),
            exports: vec![ExportEntry {
                pc: 4,
                frame_type: 1,
                signature: 0x1234,
                name: "set_value".to_string(),
            }],
            imports: vec![],
            handlers: vec![],
        };

        let mut state = vm::VmState::new(&module).expect("vm should initialize");
        state.run().expect("self-load mcall should execute cleanly");
        assert_eq!(crate::memory::read_word(&state.mp, 4), 42);
    }

    #[test]
    fn execute_load_self_and_mspawn_main_module() {
        let module = Module {
            header: Header {
                data_size: 8,
                ..make_header(6, 2, 0)
            },
            code: vec![
                make_inst(Opcode::Load, mp(0), mid_imm(0), fp(0)),
                make_inst(Opcode::Mframe, fp(0), mid_imm(0), fp(4)),
                make_inst(Opcode::Mspawn, fp(4), mid_imm(0), fp(0)),
                exit_inst(),
                make_inst(Opcode::Movw, imm(77), mid_none(), mp(4)),
                make_inst(Opcode::Ret, none(), mid_none(), none()),
            ],
            types: vec![type_desc(64), type_desc(32)],
            data: vec![DataItem::String {
                offset: 0,
                value: "$self".to_string(),
            }],
            name: "test_mspawn_self".to_string(),
            exports: vec![ExportEntry {
                pc: 4,
                frame_type: 1,
                signature: 0x4321,
                name: "set_value".to_string(),
            }],
            imports: vec![],
            handlers: vec![],
        };

        let mut state = vm::VmState::new(&module).expect("vm should initialize");
        state
            .run()
            .expect("self-load mspawn should execute cleanly");
        assert_eq!(crate::memory::read_word(&state.mp, 4), 77);
    }

    #[test]
    fn direct_mspawn_executes_loaded_module_inline() {
        let main_module = Module {
            header: make_header(1, 1, 0),
            code: vec![exit_inst()],
            types: vec![type_desc(32)],
            data: vec![],
            name: "test_mspawn_main".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        };
        let loaded_module = Module {
            header: Header {
                data_size: 4,
                ..make_header(2, 2, 0)
            },
            code: vec![
                make_inst(Opcode::Movw, imm(99), mid_none(), mp(0)),
                make_inst(Opcode::Ret, none(), mid_none(), none()),
            ],
            types: vec![type_desc(64), type_desc(32)],
            data: vec![],
            name: "loaded_for_mspawn".to_string(),
            exports: vec![ExportEntry {
                pc: 0,
                frame_type: 1,
                signature: 0x9999,
                name: "set_loaded".to_string(),
            }],
            imports: vec![],
            handlers: vec![],
        };

        let mut state = vm::VmState::new(&main_module).expect("vm should initialize");
        state.loaded_modules.push(vm::LoadedModule {
            module: loaded_module,
            mp: vec![0; 4],
        });
        let mod_ref_id = state.heap.alloc(
            0,
            crate::heap::HeapData::LoadedModule {
                module_idx: 0,
                func_map: Vec::new(),
            },
        );
        let frame_ptr = state.frames.alloc_pending(32).expect("frame alloc");

        state.src = crate::address::AddrTarget::Immediate;
        state.imm_src = frame_ptr as i32;
        state.mid = crate::address::AddrTarget::Immediate;
        state.imm_mid = 0;
        state.dst = crate::address::AddrTarget::Immediate;
        state.imm_dst = mod_ref_id as i32;

        crate::ops::concurrency::op_mspawn(&mut state).expect("mspawn should succeed");

        assert_eq!(crate::memory::read_word(&state.loaded_modules[0].mp, 0), 99);
        assert!(state.current_loaded_module.is_none());
    }

    #[test]
    fn execute_self_opcode_returns_main_module_ref() {
        let module = Module {
            header: make_header(2, 1, 0),
            code: vec![
                make_inst(Opcode::Self_, none(), mid_none(), fp(0)),
                exit_inst(),
            ],
            types: vec![type_desc(32)],
            data: vec![],
            name: "test_self_opcode".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        };

        let mut state = vm::VmState::new(&module).expect("vm should initialize");
        state.run().expect("self opcode should execute cleanly");

        let ref_id =
            crate::memory::read_word(&state.frames.data, state.frames.current_data_offset()) as u32;
        match &state
            .heap
            .get(ref_id)
            .expect("module ref should exist")
            .data
        {
            crate::heap::HeapData::MainModule { func_map } => assert!(func_map.is_empty()),
            other => panic!("expected main module ref, got {other:?}"),
        }
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

    #[test]
    fn execute_indx_array_write_read() {
        // Create an array of 3 words, write 42 to index 1, read it back.
        //
        // 0: newa $3, $0, 0(fp)           -- fp[0] = new word[3] (type 0, 4 bytes each)
        // 1: indx 0(fp), 4(fp), $1        -- fp[4] = &array[1] (heap ref)
        // 2: movw $42, 0(4(fp))           -- *fp[4] = 42 (double indirect write to array)
        // 3: indx 0(fp), 8(fp), $1        -- fp[8] = &array[1] again
        // 4: movw 0(8(fp)), 12(fp)        -- fp[12] = *fp[8] = 42 (double indirect read)
        // 5: exit
        let double_fp = |base: i32, off: i32| -> Operand {
            Operand {
                mode: AddressMode::OffsetDoubleIndirectFp,
                register1: base,
                register2: off,
            }
        };

        let module = Module {
            header: make_header(6, 2, 0),
            code: vec![
                // newa: src=$3 (length), mid=$1 (elem type = type 1 = 4 bytes), dst=0(fp)
                make_inst(Opcode::Newa, imm(3), mid_imm(1), fp(0)),
                // indx: src=0(fp) (array), mid=4(fp) (result), dst=$1 (index)
                make_inst(Opcode::Indx, fp(0), mid_fp(4), imm(1)),
                // movw $42 to 0(4(fp)) — double indirect through heap ref
                make_inst(Opcode::Movw, imm(42), mid_none(), double_fp(4, 0)),
                // indx again to read
                make_inst(Opcode::Indx, fp(0), mid_fp(8), imm(1)),
                // read from 0(8(fp)) into 12(fp)
                make_inst(Opcode::Movw, double_fp(8, 0), mid_none(), fp(12)),
                exit_inst(),
            ],
            types: vec![type_desc(64), type_desc(4)], // type 0 = frame (64B), type 1 = word element (4B)
            data: vec![],
            name: "test_indx".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        };
        assert!(execute(&module).is_ok());
    }

    #[test]
    fn execute_casew_dispatch() {
        // Case table in MP: match value 2
        //   count=2, [1, 2, pc=5], [3, 4, pc=6], default=7
        // Table layout (words at MP offsets):
        //   MP[0]=2 (count), MP[4]=1, MP[8]=2, MP[12]=5 (first entry: [1,2)->pc 5)
        //   MP[16]=3, MP[20]=4, MP[24]=6 (second: [3,4)->pc 6)
        //   MP[28]=7 (default)
        //
        // Code:
        // 0: movw $3, 0(fp)     -- value = 3
        // 1: casew 0(fp), 0(mp) -- match 3 in table -> [3,4) -> pc 6
        // ... (pcs 2-5 are fillers)
        // 6: movw $99, 4(fp)    -- hit: marker
        // 7: exit

        let module = Module {
            header: Header {
                data_size: 32,
                ..make_header(8, 1, 0)
            },
            code: vec![
                // 0
                make_inst(Opcode::Movw, imm(3), mid_none(), fp(0)),
                // 1: casew src=0(fp) value, dst=0(mp) table
                make_inst(Opcode::Casew, fp(0), mid_none(), mp(0)),
                // 2-5: fillers (should not be reached)
                exit_inst(), // 2
                exit_inst(), // 3
                exit_inst(), // 4
                exit_inst(), // 5
                // 6: target for value 3
                make_inst(Opcode::Movw, imm(99), mid_none(), fp(4)),
                // 7: exit (default and also end)
                exit_inst(),
            ],
            types: vec![type_desc(32)],
            data: vec![DataItem::Words {
                offset: 0,
                values: vec![
                    2, // count
                    1, 2, 5, // entry 0: [1,2) -> pc 5
                    3, 4, 6, // entry 1: [3,4) -> pc 6
                    7, // default pc
                ],
            }],
            name: "test_casew".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        };
        assert!(execute(&module).is_ok());
    }

    #[test]
    fn execute_casew_default() {
        // Value 99 doesn't match any case, should jump to default (pc 3)
        let module = Module {
            header: Header {
                data_size: 20,
                ..make_header(4, 1, 0)
            },
            code: vec![
                // 0: value = 99
                make_inst(Opcode::Movw, imm(99), mid_none(), fp(0)),
                // 1: casew — should go to default (pc 3)
                make_inst(Opcode::Casew, fp(0), mid_none(), mp(0)),
                // 2: should not be reached
                make_inst(Opcode::Movw, imm(1), mid_none(), fp(4)),
                // 3: exit (default target)
                exit_inst(),
            ],
            types: vec![type_desc(32)],
            data: vec![DataItem::Words {
                offset: 0,
                values: vec![
                    1, // count = 1 entry
                    0, 1, 2, // [0,1) -> pc 2
                    3, // default -> pc 3
                ],
            }],
            name: "test_casew_default".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        };
        assert!(execute(&module).is_ok());
    }

    #[test]
    fn execute_comprehensive_string_array_list_print() {
        // A complex module that exercises multiple subsystems together:
        //
        // MP layout:
        //   0: "$Sys" string HeapId
        //   4: "count: %d\n" format string HeapId
        //
        // Program:
        //   0: load 0(mp), $0, 0(fp)      -- fp[0] = load $Sys
        //   1: newa $5, $1, 4(fp)          -- fp[4] = new int[5] (type 1 = 4 bytes)
        //   2: movw $0, 8(fp)              -- fp[8] = 0 (loop counter i)
        //   3: movw $5, 12(fp)             -- fp[12] = 5 (limit)
        //   --- loop: store i*10 into array[i] ---
        //   4: mulw 8(fp), $10, 16(fp)     -- fp[16] = i * 10
        //   5: indx 4(fp), 20(fp), 8(fp)   -- fp[20] = &array[i]
        //   6: movw 16(fp), 0(20(fp))      -- array[i] = i * 10
        //   7: addw 8(fp), $1, 8(fp)       -- i++
        //   8: bltw 8(fp), $4, 12(fp)      -- if i < 5, goto 4
        //   --- print array[3] (should be 30) ---
        //   9: indx 4(fp), 20(fp), $3      -- fp[20] = &array[3]
        //  10: movw 0(20(fp)), 24(fp)      -- fp[24] = array[3] = 30
        //  11: mframe 0(fp), $0, 28(fp)    -- fp[28] = pending frame for print
        //  12: movw 4(mp), 16(28(fp))      -- pending[16] = format string
        //  13: movw 24(fp), 20(28(fp))     -- pending[20] = value (30)
        //  14: mcall 28(fp), $0, 0(fp)     -- call $Sys.print("count: %d\n", 30)
        //  15: exit

        let double_fp = |base: i32, off: i32| -> Operand {
            Operand {
                mode: AddressMode::OffsetDoubleIndirectFp,
                register1: base,
                register2: off,
            }
        };

        let module = Module {
            header: Header {
                data_size: 8,
                ..make_header(16, 2, 0)
            },
            code: vec![
                // 0: load $Sys
                make_inst(Opcode::Load, mp(0), mid_imm(0), fp(0)),
                // 1: newa $5, $1 (elem type), fp[4]
                make_inst(Opcode::Newa, imm(5), mid_imm(1), fp(4)),
                // 2: i = 0
                make_inst(Opcode::Movw, imm(0), mid_none(), fp(8)),
                // 3: limit = 5
                make_inst(Opcode::Movw, imm(5), mid_none(), fp(12)),
                // 4: fp[16] = i * 10
                make_inst(Opcode::Mulw, fp(8), mid_imm(10), fp(16)),
                // 5: fp[20] = &array[i]
                make_inst(Opcode::Indx, fp(4), mid_fp(20), fp(8)),
                // 6: array[i] = fp[16]
                make_inst(Opcode::Movw, fp(16), mid_none(), double_fp(20, 0)),
                // 7: i++
                make_inst(Opcode::Addw, fp(8), mid_imm(1), fp(8)),
                // 8: if i < 5, goto 4
                make_inst(Opcode::Bltw, fp(8), mid_imm(4), fp(12)),
                // 9: fp[20] = &array[3]
                make_inst(Opcode::Indx, fp(4), mid_fp(20), imm(3)),
                // 10: fp[24] = array[3]
                make_inst(Opcode::Movw, double_fp(20, 0), mid_none(), fp(24)),
                // 11: mframe for print
                make_inst(Opcode::Mframe, fp(0), mid_imm(27), fp(28)),
                // 12: write format string to pending frame offset 16
                make_inst(Opcode::Movw, mp(4), mid_none(), double_fp(28, 16)),
                // 13: write value to pending frame offset 20
                make_inst(Opcode::Movw, fp(24), mid_none(), double_fp(28, 20)),
                // 14: mcall print (func index 27 = "print" in alphabetical Sys table)
                make_inst(Opcode::Mcall, fp(28), mid_imm(27), fp(0)),
                // 15: exit
                exit_inst(),
            ],
            types: vec![type_desc(64), type_desc(4)],
            data: vec![
                DataItem::String {
                    offset: 0,
                    value: "$Sys".to_string(),
                },
                DataItem::String {
                    offset: 4,
                    value: "count: %d\n".to_string(),
                },
            ],
            name: "comprehensive_test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        };
        // Should print "count: 30\n"
        assert!(execute(&module).is_ok());
    }
}
