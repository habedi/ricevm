//! Built-in Sys module implementation.
//!
//! Provides `print` and stubs for other Sys functions.

use ricevm_core::ExecError;

use crate::builtin::{BuiltinFunc, BuiltinModule};
use crate::heap;
use crate::memory;
use crate::vm::VmState;

/// Frame layout for Sys->print:
/// Offset 0: return value pointer (word)
/// Offset 4..16: temp registers
/// Offset 16: format string pointer (word = HeapId)
/// Offset 20: start of packed varargs
const PRINT_FMT_OFFSET: usize = 16;
const PRINT_ARGS_OFFSET: usize = 20;

/// Create the $Sys built-in module.
pub(crate) fn create_sys_module() -> BuiltinModule {
    BuiltinModule {
        name: "$Sys",
        funcs: vec![BuiltinFunc {
            name: "print",
            frame_size: 64,
            handler: sys_print,
        }],
    }
}

/// Sys->print implementation.
///
/// Reads the format string from the frame and handles basic format specifiers:
/// `%s` (string), `%d` (int), `%g`/`%f`/`%e` (float), `%c` (char), `%%` (literal %).
fn sys_print(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let fmt_id = memory::read_word(&vm.frames.data, frame_base + PRINT_FMT_OFFSET) as heap::HeapId;

    let fmt_str = match vm.heap.get_string(fmt_id) {
        Some(s) => s.to_string(),
        None => return Ok(()), // nil string → print nothing
    };

    let mut output = String::new();
    let mut arg_offset = frame_base + PRINT_ARGS_OFFSET;
    let mut chars = fmt_str.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '%' {
            output.push(ch);
            continue;
        }
        match chars.next() {
            Some('%') => output.push('%'),
            Some('s') => {
                let str_id = memory::read_word(&vm.frames.data, arg_offset) as heap::HeapId;
                if let Some(s) = vm.heap.get_string(str_id) {
                    output.push_str(s);
                }
                arg_offset += 4;
            }
            Some('d') => {
                let val = memory::read_word(&vm.frames.data, arg_offset);
                output.push_str(&val.to_string());
                arg_offset += 4;
            }
            Some('g' | 'f' | 'e') => {
                let val = memory::read_real(&vm.frames.data, arg_offset);
                output.push_str(&val.to_string());
                arg_offset += 8;
            }
            Some('c') => {
                let val = memory::read_word(&vm.frames.data, arg_offset) as u32;
                if let Some(c) = char::from_u32(val) {
                    output.push(c);
                }
                arg_offset += 4;
            }
            Some('x') => {
                let val = memory::read_word(&vm.frames.data, arg_offset);
                output.push_str(&format!("{val:x}"));
                arg_offset += 4;
            }
            Some(other) => {
                output.push('%');
                output.push(other);
            }
            None => output.push('%'),
        }
    }

    print!("{output}");
    Ok(())
}
