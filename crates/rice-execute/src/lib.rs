//! Rice VM Execution Engine
//!
//! Takes a loaded [`Module`] and runs it from its entry point.

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
    // TODO: implement instruction dispatch loop
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ricevm_core::{Header, Opcode, RuntimeFlags};

    fn minimal_module() -> Module {
        Module {
            header: Header {
                magic: ricevm_core::XMAGIC,
                signature: vec![],
                runtime_flags: RuntimeFlags(0),
                stack_extent: 0,
                code_size: 1,
                data_size: 0,
                type_size: 0,
                export_size: 0,
                entry_pc: 0,
                entry_type: 0,
            },
            code: vec![ricevm_core::Instruction {
                opcode: Opcode::Exit,
                source: ricevm_core::Operand::UNUSED,
                middle: ricevm_core::MiddleOperand::UNUSED,
                destination: ricevm_core::Operand::UNUSED,
            }],
            types: vec![],
            data: vec![],
            name: "test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        }
    }

    #[test]
    fn execute_minimal_module() {
        let module = minimal_module();
        assert!(execute(&module).is_ok());
    }

    #[test]
    fn invalid_entry_pc() {
        let mut module = minimal_module();
        module.header.entry_pc = 999;
        let result = execute(&module);
        assert!(matches!(result, Err(ExecError::InvalidPc(999))));
    }
}
