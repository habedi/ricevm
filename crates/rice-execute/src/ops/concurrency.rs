//! Concurrency opcodes (stubs).
//!
//! These opcodes require a thread scheduler and channel system,
//! which are not yet implemented. They return meaningful errors
//! so that programs hitting them fail gracefully.

use ricevm_core::ExecError;

use crate::vm::VmState;

pub(crate) fn op_spawn(_vm: &mut VmState<'_>) -> Result<(), ExecError> {
    Err(ExecError::Other(
        "spawn: threading not yet implemented".to_string(),
    ))
}

pub(crate) fn op_mspawn(_vm: &mut VmState<'_>) -> Result<(), ExecError> {
    Err(ExecError::Other(
        "mspawn: threading not yet implemented".to_string(),
    ))
}

pub(crate) fn op_send(_vm: &mut VmState<'_>) -> Result<(), ExecError> {
    Err(ExecError::Other(
        "send: channels not yet implemented".to_string(),
    ))
}

pub(crate) fn op_recv(_vm: &mut VmState<'_>) -> Result<(), ExecError> {
    Err(ExecError::Other(
        "recv: channels not yet implemented".to_string(),
    ))
}

pub(crate) fn op_alt(_vm: &mut VmState<'_>) -> Result<(), ExecError> {
    Err(ExecError::Other(
        "alt: channels not yet implemented".to_string(),
    ))
}

pub(crate) fn op_nbalt(_vm: &mut VmState<'_>) -> Result<(), ExecError> {
    Err(ExecError::Other(
        "nbalt: channels not yet implemented".to_string(),
    ))
}
