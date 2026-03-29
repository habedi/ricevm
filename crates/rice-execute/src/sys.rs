//! Built-in Sys module implementation.
//!
//! Functions are registered in alphabetical order to match the C++ Sysmodtab.
//! The import table in compiled .dis modules references functions by index
//! into this alphabetically-sorted table.

use std::time::{SystemTime, UNIX_EPOCH};

use ricevm_core::ExecError;

use crate::builtin::{BuiltinFunc, BuiltinModule};
use crate::heap::{self, HeapData, HeapId};
use crate::memory;
use crate::vm::VmState;

// Common frame layout offsets for Sys functions.
// Most Sys functions have:
//   Offset 0..4:   return value (word or pointer)
//   Offset 4..16:  temp registers (3 words)
//   Offset 16+:    arguments

/// Format a printf-style string with arguments from the frame.
fn format_string(
    vm: &VmState<'_>,
    frame_base: usize,
    fmt_offset: usize,
    args_offset: usize,
) -> String {
    let fmt_id = memory::read_word(&vm.frames.data, frame_base + fmt_offset) as HeapId;
    let fmt_str = match vm.heap.get_string(fmt_id) {
        Some(s) => s.to_string(),
        None => return String::new(),
    };

    let mut output = String::new();
    let mut arg_offset = frame_base + args_offset;
    let mut chars = fmt_str.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '%' {
            output.push(ch);
            continue;
        }
        match chars.next() {
            Some('%') => output.push('%'),
            Some('s') => {
                let str_id = memory::read_word(&vm.frames.data, arg_offset) as HeapId;
                if let Some(s) = vm.heap.get_string(str_id) {
                    output.push_str(s);
                } else {
                    output.push_str("<nil>");
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
            Some('X') => {
                let val = memory::read_word(&vm.frames.data, arg_offset);
                output.push_str(&format!("{val:X}"));
                arg_offset += 4;
            }
            Some('o') => {
                let val = memory::read_word(&vm.frames.data, arg_offset);
                output.push_str(&format!("{val:o}"));
                arg_offset += 4;
            }
            Some('r') => {
                output.push_str("(no error)");
            }
            Some(other) => {
                output.push('%');
                output.push(other);
            }
            None => output.push('%'),
        }
    }
    output
}

/// Create the $Sys built-in module.
///
/// Functions MUST be in alphabetical order to match the C++ Sysmodtab indices.
pub(crate) fn create_sys_module() -> BuiltinModule {
    BuiltinModule {
        name: "$Sys",
        funcs: vec![
            bf("announce", 40, sys_stub),       // 0
            bf("aprint", 64, sys_aprint),       // 1
            bf("bind", 48, sys_stub),           // 2
            bf("byte2char", 40, sys_byte2char), // 3
            bf("char2byte", 48, sys_char2byte), // 4
            bf("chdir", 40, sys_stub),          // 5
            bf("create", 48, sys_create),       // 6
            bf("dial", 40, sys_stub),           // 7
            bf("dirread", 40, sys_stub),        // 8
            bf("dup", 40, sys_dup),             // 9
            bf("export", 48, sys_stub),         // 10
            bf("fauth", 40, sys_stub),          // 11
            bf("fd2path", 40, sys_fd2path),     // 12
            bf("fildes", 40, sys_fildes),       // 13
            bf("file2chan", 40, sys_stub),      // 14
            bf("fprint", 64, sys_fprint),       // 15
            bf("fstat", 40, sys_stub),          // 16
            bf("fversion", 48, sys_stub),       // 17
            bf("fwstat", 104, sys_stub),        // 18
            bf("iounit", 40, sys_stub),         // 19
            bf("listen", 48, sys_stub),         // 20
            bf("millisec", 32, sys_millisec),   // 21
            bf("mount", 56, sys_stub),          // 22
            bf("open", 40, sys_open),           // 23
            bf("pctl", 40, sys_pctl),           // 24
            bf("pipe", 40, sys_stub),           // 25
            bf("pread", 56, sys_stub),          // 26
            bf("print", 64, sys_print),         // 27
            bf("pwrite", 56, sys_stub),         // 28
            bf("read", 48, sys_read),           // 29
            bf("readn", 48, sys_read),          // 30
            bf("remove", 40, sys_stub),         // 31
            bf("seek", 56, sys_stub),           // 32
            bf("sleep", 40, sys_sleep),         // 33
            bf("sprint", 64, sys_sprint),       // 34
            bf("stat", 40, sys_stub),           // 35
            bf("stream", 48, sys_stub),         // 36
            bf("tokenize", 40, sys_tokenize),   // 37
            bf("unmount", 40, sys_stub),        // 38
            bf("utfbytes", 40, sys_utfbytes),   // 39
            bf("werrstr", 40, sys_stub),        // 40
            bf("write", 48, sys_write),         // 41
            bf("wstat", 104, sys_stub),         // 42
        ],
    }
}

fn bf(
    name: &'static str,
    frame_size: usize,
    handler: fn(&mut VmState<'_>) -> Result<(), ExecError>,
) -> BuiltinFunc {
    BuiltinFunc {
        name,
        frame_size,
        handler,
    }
}

// --- Stub for unimplemented functions ---

fn sys_stub(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    // Set return value to -1 (error)
    memory::write_word(&mut vm.frames.data, frame_base, -1);
    Ok(())
}

// --- print / fprint / sprint ---

fn sys_print(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let output = format_string(vm, frame_base, 16, 20);
    let len = output.len() as i32;
    print!("{output}");
    memory::write_word(&mut vm.frames.data, frame_base, len);
    Ok(())
}

fn sys_fprint(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    // arg at offset 16 = fd pointer, offset 20 = format string, offset 24 = args
    let fd_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let fd_num = if fd_id == heap::NIL as HeapId {
        1 // default to stdout
    } else {
        // Read fd number from the FD adt (first field is the int fd)
        match vm.heap.get(fd_id) {
            Some(obj) => match &obj.data {
                HeapData::Record(data) => {
                    if data.len() >= 4 {
                        memory::read_word(data, 0)
                    } else {
                        1
                    }
                }
                _ => 1,
            },
            None => 1,
        }
    };
    let output = format_string(vm, frame_base, 20, 24);
    let len = output.len() as i32;
    match fd_num {
        2 => {
            eprint!("{output}");
        }
        _ => {
            print!("{output}");
        }
    }
    memory::write_word(&mut vm.frames.data, frame_base, len);
    Ok(())
}

fn sys_sprint(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let output = format_string(vm, frame_base, 16, 20);
    let str_id = vm.heap.alloc(0, HeapData::Str(output));
    // Return string pointer at frame offset 0
    memory::write_word(&mut vm.frames.data, frame_base, str_id as i32);
    Ok(())
}

fn sys_aprint(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let output = format_string(vm, frame_base, 16, 20);
    let bytes = output.into_bytes();
    let length = bytes.len();
    let arr_id = vm.heap.alloc(
        0,
        HeapData::Array {
            elem_type: 0,
            elem_size: 1,
            data: bytes,
            length,
        },
    );
    memory::write_word(&mut vm.frames.data, frame_base, arr_id as i32);
    Ok(())
}

// --- File I/O ---

fn sys_open(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let path_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let mode = memory::read_word(&vm.frames.data, frame_base + 20);

    let path = vm.heap.get_string(path_id).unwrap_or("").to_string();

    let file = match mode & 0x3 {
        0 => std::fs::File::open(&path),                          // OREAD
        1 => std::fs::OpenOptions::new().write(true).open(&path), // OWRITE
        _ => std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path), // ORDWR
    };

    match file {
        Ok(f) => {
            use std::os::unix::io::IntoRawFd;
            let raw_fd = f.into_raw_fd();
            // Allocate an FD adt on the heap (single word: the fd number)
            let mut fd_data = vec![0u8; 4];
            memory::write_word(&mut fd_data, 0, raw_fd);
            let fd_id = vm.heap.alloc(0, HeapData::Record(fd_data));
            memory::write_word(&mut vm.frames.data, frame_base, fd_id as i32);
        }
        Err(_) => {
            memory::write_word(&mut vm.frames.data, frame_base, 0); // nil
        }
    }
    Ok(())
}

fn sys_create(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let path_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let _mode = memory::read_word(&vm.frames.data, frame_base + 20);
    let _perm = memory::read_word(&vm.frames.data, frame_base + 24);

    let path = vm.heap.get_string(path_id).unwrap_or("").to_string();

    match std::fs::File::create(&path) {
        Ok(f) => {
            use std::os::unix::io::IntoRawFd;
            let raw_fd = f.into_raw_fd();
            let mut fd_data = vec![0u8; 4];
            memory::write_word(&mut fd_data, 0, raw_fd);
            let fd_id = vm.heap.alloc(0, HeapData::Record(fd_data));
            memory::write_word(&mut vm.frames.data, frame_base, fd_id as i32);
        }
        Err(_) => {
            memory::write_word(&mut vm.frames.data, frame_base, 0);
        }
    }
    Ok(())
}

fn sys_read(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let fd_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let buf_id = memory::read_word(&vm.frames.data, frame_base + 20) as HeapId;
    let count = memory::read_word(&vm.frames.data, frame_base + 24) as usize;

    let fd_num = get_fd_num(vm, fd_id);

    // Read into a temporary buffer
    let mut tmp = vec![0u8; count];
    let n = unsafe { libc::read(fd_num, tmp.as_mut_ptr() as *mut libc::c_void, count) };

    if n > 0
        && let Some(obj) = vm.heap.get_mut(buf_id)
        && let HeapData::Array { data, .. } = &mut obj.data
    {
        let copy_len = (n as usize).min(data.len());
        data[..copy_len].copy_from_slice(&tmp[..copy_len]);
    }

    memory::write_word(&mut vm.frames.data, frame_base, n as i32);
    Ok(())
}

fn sys_write(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let fd_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let buf_id = memory::read_word(&vm.frames.data, frame_base + 20) as HeapId;
    let count = memory::read_word(&vm.frames.data, frame_base + 24) as usize;

    let fd_num = get_fd_num(vm, fd_id);

    let bytes = match vm.heap.get(buf_id) {
        Some(obj) => match &obj.data {
            HeapData::Array { data, .. } => &data[..count.min(data.len())],
            _ => &[],
        },
        None => &[],
    };

    let n = unsafe { libc::write(fd_num, bytes.as_ptr() as *const libc::c_void, bytes.len()) };

    memory::write_word(&mut vm.frames.data, frame_base, n as i32);
    Ok(())
}

fn sys_fildes(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let fd_num = memory::read_word(&vm.frames.data, frame_base + 16);
    let mut fd_data = vec![0u8; 4];
    memory::write_word(&mut fd_data, 0, fd_num);
    let fd_id = vm.heap.alloc(0, HeapData::Record(fd_data));
    memory::write_word(&mut vm.frames.data, frame_base, fd_id as i32);
    Ok(())
}

fn sys_fd2path(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let fd_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let fd_num = get_fd_num(vm, fd_id);

    let path = format!("/proc/self/fd/{fd_num}");
    let result = std::fs::read_link(&path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    let str_id = vm.heap.alloc(0, HeapData::Str(result));
    memory::write_word(&mut vm.frames.data, frame_base, str_id as i32);
    Ok(())
}

fn sys_dup(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let old_fd = memory::read_word(&vm.frames.data, frame_base + 16);
    let new_fd = memory::read_word(&vm.frames.data, frame_base + 20);
    let result = unsafe {
        if new_fd < 0 {
            libc::dup(old_fd)
        } else {
            libc::dup2(old_fd, new_fd)
        }
    };
    memory::write_word(&mut vm.frames.data, frame_base, result);
    Ok(())
}

// --- Utility functions ---

fn sys_millisec(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i32)
        .unwrap_or(0);
    memory::write_word(&mut vm.frames.data, frame_base, ms);
    Ok(())
}

fn sys_sleep(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let period_ms = memory::read_word(&vm.frames.data, frame_base + 16);
    if period_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(period_ms as u64));
    }
    memory::write_word(&mut vm.frames.data, frame_base, 0);
    Ok(())
}

fn sys_pctl(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    // Stub: return 0 (success)
    memory::write_word(&mut vm.frames.data, frame_base, 0);
    Ok(())
}

fn sys_tokenize(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let str_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let delim_id = memory::read_word(&vm.frames.data, frame_base + 20) as HeapId;

    let s = vm.heap.get_string(str_id).unwrap_or("").to_string();
    let delim = vm.heap.get_string(delim_id).unwrap_or(" \t\n").to_string();

    // Split by any delimiter character
    let tokens: Vec<&str> = s
        .split(|c: char| delim.contains(c))
        .filter(|t| !t.is_empty())
        .collect();

    let count = tokens.len() as i32;

    // Build a list of strings (in reverse order, then cons builds forward)
    let mut list_id = heap::NIL;
    for token in tokens.iter().rev() {
        let tok_id = vm.heap.alloc(0, HeapData::Str(token.to_string()));
        let mut head = vec![0u8; 4];
        memory::write_word(&mut head, 0, tok_id as i32);
        if list_id != heap::NIL {
            vm.heap.inc_ref(list_id);
        }
        list_id = vm.heap.alloc(
            0,
            HeapData::List {
                head,
                tail: list_id,
            },
        );
    }

    // Return (count, list) — count at offset 0, list at offset 4
    memory::write_word(&mut vm.frames.data, frame_base, count);
    memory::write_word(&mut vm.frames.data, frame_base + 4, list_id as i32);
    Ok(())
}

fn sys_byte2char(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let buf_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let n = memory::read_word(&vm.frames.data, frame_base + 20) as usize;

    let (char_val, bytes_consumed, status) = if let Some(obj) = vm.heap.get(buf_id) {
        match &obj.data {
            HeapData::Array { data, .. } => {
                let slice = &data[..n.min(data.len())];
                match std::str::from_utf8(slice) {
                    Ok(s) => {
                        if let Some(ch) = s.chars().next() {
                            (ch as i32, ch.len_utf8() as i32, 0)
                        } else {
                            (0, 0, -1)
                        }
                    }
                    Err(e) => {
                        let valid = e.valid_up_to();
                        if valid > 0 {
                            let s = std::str::from_utf8(&slice[..valid]).unwrap_or("");
                            if let Some(ch) = s.chars().next() {
                                (ch as i32, ch.len_utf8() as i32, 0)
                            } else {
                                (0xFFFD, 1, 0)
                            }
                        } else {
                            (0xFFFD, 1, 0) // replacement character
                        }
                    }
                }
            }
            _ => (0, 0, -1),
        }
    } else {
        (0, 0, -1)
    };

    // Return (char, bytes_consumed, status)
    memory::write_word(&mut vm.frames.data, frame_base, char_val);
    memory::write_word(&mut vm.frames.data, frame_base + 4, bytes_consumed);
    memory::write_word(&mut vm.frames.data, frame_base + 8, status);
    Ok(())
}

fn sys_char2byte(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let ch_val = memory::read_word(&vm.frames.data, frame_base + 16) as u32;
    let buf_id = memory::read_word(&vm.frames.data, frame_base + 20) as HeapId;
    let n = memory::read_word(&vm.frames.data, frame_base + 24) as usize;

    let ch = char::from_u32(ch_val).unwrap_or('\u{FFFD}');
    let mut utf8_buf = [0u8; 4];
    let encoded = ch.encode_utf8(&mut utf8_buf);
    let bytes_written = encoded.len();

    if let Some(obj) = vm.heap.get_mut(buf_id)
        && let HeapData::Array { data, .. } = &mut obj.data
    {
        let copy_len = bytes_written.min(data.len() - n);
        data[n..n + copy_len].copy_from_slice(&utf8_buf[..copy_len]);
    }

    memory::write_word(&mut vm.frames.data, frame_base, bytes_written as i32);
    Ok(())
}

fn sys_utfbytes(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let buf_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let n = memory::read_word(&vm.frames.data, frame_base + 20) as usize;

    let result = if let Some(obj) = vm.heap.get(buf_id) {
        match &obj.data {
            HeapData::Array { data, .. } => {
                let slice = &data[..n.min(data.len())];
                // Find the last valid UTF-8 boundary
                match std::str::from_utf8(slice) {
                    Ok(s) => s.len() as i32,
                    Err(e) => e.valid_up_to() as i32,
                }
            }
            _ => 0,
        }
    } else {
        0
    };

    memory::write_word(&mut vm.frames.data, frame_base, result);
    Ok(())
}

// --- Helper ---

fn get_fd_num(vm: &VmState<'_>, fd_id: HeapId) -> i32 {
    if fd_id == heap::NIL as HeapId {
        return -1;
    }
    match vm.heap.get(fd_id) {
        Some(obj) => match &obj.data {
            HeapData::Record(data) => {
                if data.len() >= 4 {
                    memory::read_word(data, 0)
                } else {
                    -1
                }
            }
            _ => -1,
        },
        None => -1,
    }
}
