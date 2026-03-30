//! Built-in Sys module implementation.
//!
//! Functions are registered in alphabetical order to match the C++ Sysmodtab.
//! The import table in compiled .dis modules references functions by index
//! into this alphabetically-sorted table.

use std::net::{TcpListener, TcpStream};
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
            bf("announce", 0x0b7c4ac0, 40, sys_announce),
            bf("aprint", 0x77442d46, 64, sys_aprint),
            bf("bind", 0x66326d91, 48, sys_bind),
            bf("byte2char", 0x3d6094f9, 40, sys_byte2char),
            bf("char2byte", 0x2ba5ab41, 48, sys_char2byte),
            bf("chdir", 0xc6935858, 40, sys_chdir),
            bf("create", 0x54db77d9, 48, sys_create),
            bf("dial", 0x29e90174, 40, sys_dial),
            bf("dirread", 0x72210d71, 40, sys_dirread),
            bf("dup", 0x6584767b, 40, sys_dup),
            bf("export", 0x6fc6dc03, 48, sys_stub),
            bf("fauth", 0x20ccc34b, 40, sys_fauth),
            bf("fd2path", 0x749c6042, 40, sys_fd2path),
            bf("fildes", 0x1478f993, 40, sys_fildes),
            bf("file2chan", 0x9f34d686, 40, sys_file2chan),
            bf("fprint", 0xf46486c8, 64, sys_fprint),
            bf("fstat", 0xda4499c2, 40, sys_fstat),
            bf("fversion", 0xfe9c0a06, 48, sys_fversion),
            bf("fwstat", 0x50a6c7e0, 104, sys_stub),
            bf("iounit", 0x5583b730, 40, sys_iounit),
            bf("listen", 0xb97416e0, 48, sys_listen),
            bf("millisec", 0x616977e8, 32, sys_millisec),
            bf("mount", 0x74c17b3a, 56, sys_stub), // unsupported on host OS
            bf("open", 0x8f477f99, 40, sys_open),
            bf("pctl", 0x05df27fb, 40, sys_pctl),
            bf("pipe", 0x1f2c52ea, 40, sys_pipe),
            bf("pread", 0x09d8aac6, 56, sys_pread),
            bf("print", 0xac849033, 64, sys_print),
            bf("pwrite", 0x09d8aac6, 56, sys_pwrite),
            bf("read", 0x7cfef557, 48, sys_read),
            bf("readn", 0x7cfef557, 48, sys_read),
            bf("remove", 0xc6935858, 40, sys_remove),
            bf("seek", 0xaeccaddb, 56, sys_seek),
            bf("sleep", 0xe67bf126, 40, sys_sleep),
            bf("sprint", 0x4c0624b6, 64, sys_sprint),
            bf("stat", 0x319328dd, 40, sys_stat),
            bf("stream", 0xb9e8f9ea, 48, sys_stub),
            bf("tokenize", 0x57338f20, 40, sys_tokenize),
            bf("unmount", 0x21e337e3, 40, sys_stub), // unsupported on host OS
            bf("utfbytes", 0x01d4a1f4, 40, sys_utfbytes),
            bf("werrstr", 0xc6935858, 40, sys_werrstr),
            bf("write", 0x7cfef557, 48, sys_write),
            bf("wstat", 0x56b02096, 104, sys_stub), // requires OS-specific permissions API
        ],
    }
}

fn bf(
    name: &'static str,
    sig: u32,
    frame_size: usize,
    handler: fn(&mut VmState<'_>) -> Result<(), ExecError>,
) -> BuiltinFunc {
    BuiltinFunc {
        name,
        sig,
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
    let fd_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let fd_num = get_fd_num(vm, fd_id);
    let fd_num = if fd_num < 0 { 1 } else { fd_num }; // default to stdout
    let output = format_string(vm, frame_base, 20, 24);
    let len = output.len() as i32;
    let _ = vm.files.write(fd_num, output.as_bytes());
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

// --- File I/O (portable, no libc) ---

fn sys_open(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let path_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let mode = memory::read_word(&vm.frames.data, frame_base + 20);

    let path = vm.heap.get_string(path_id).unwrap_or("").to_string();

    match vm.files.open(&path, mode) {
        Ok(fd) => {
            let mut fd_data = vec![0u8; 4];
            memory::write_word(&mut fd_data, 0, fd);
            let fd_id = vm.heap.alloc(0, HeapData::Record(fd_data));
            memory::write_word(&mut vm.frames.data, frame_base, fd_id as i32);
        }
        Err(_) => {
            memory::write_word(&mut vm.frames.data, frame_base, 0);
        }
    }
    Ok(())
}

fn sys_create(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let path_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;

    let path = vm.heap.get_string(path_id).unwrap_or("").to_string();

    match vm.files.create(&path) {
        Ok(fd) => {
            let mut fd_data = vec![0u8; 4];
            memory::write_word(&mut fd_data, 0, fd);
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
    let mut tmp = vec![0u8; count];
    let n = vm.files.read(fd_num, &mut tmp).unwrap_or(-1_i32 as usize) as i32;

    if n > 0
        && let Some(obj) = vm.heap.get_mut(buf_id)
        && let HeapData::Array { data, .. } = &mut obj.data
    {
        let copy_len = (n as usize).min(data.len());
        data[..copy_len].copy_from_slice(&tmp[..copy_len]);
    }

    memory::write_word(&mut vm.frames.data, frame_base, n);
    Ok(())
}

fn sys_write(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let fd_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let buf_id = memory::read_word(&vm.frames.data, frame_base + 20) as HeapId;
    let count = memory::read_word(&vm.frames.data, frame_base + 24) as usize;

    let fd_num = get_fd_num(vm, fd_id);
    let bytes: Vec<u8> = match vm.heap.get(buf_id) {
        Some(obj) => match &obj.data {
            HeapData::Array { data, .. } => data[..count.min(data.len())].to_vec(),
            _ => Vec::new(),
        },
        None => Vec::new(),
    };

    let n = vm.files.write(fd_num, &bytes).unwrap_or(0) as i32;
    memory::write_word(&mut vm.frames.data, frame_base, n);
    Ok(())
}

fn sys_pread(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let fd_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let buf_id = memory::read_word(&vm.frames.data, frame_base + 20) as HeapId;
    let count = memory::read_word(&vm.frames.data, frame_base + 24) as usize;
    let offset = memory::read_big(&vm.frames.data, frame_base + 28);
    let fd_num = get_fd_num(vm, fd_id);

    let original_pos = match vm.files.seek(fd_num, 0, 1) {
        Ok(pos) => pos as i64,
        Err(_) => {
            memory::write_word(&mut vm.frames.data, frame_base, -1);
            return Ok(());
        }
    };

    let result = if vm.files.seek(fd_num, offset, 0).is_ok() {
        let mut tmp = vec![0u8; count];
        let n = vm.files.read(fd_num, &mut tmp).unwrap_or(-1_i32 as usize) as i32;

        if n > 0
            && let Some(obj) = vm.heap.get_mut(buf_id)
            && let HeapData::Array { data, .. } = &mut obj.data
        {
            let copy_len = (n as usize).min(data.len());
            data[..copy_len].copy_from_slice(&tmp[..copy_len]);
        }
        n
    } else {
        -1
    };

    let _ = vm.files.seek(fd_num, original_pos, 0);
    memory::write_word(&mut vm.frames.data, frame_base, result);
    Ok(())
}

fn sys_pwrite(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let fd_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let buf_id = memory::read_word(&vm.frames.data, frame_base + 20) as HeapId;
    let count = memory::read_word(&vm.frames.data, frame_base + 24) as usize;
    let offset = memory::read_big(&vm.frames.data, frame_base + 28);
    let fd_num = get_fd_num(vm, fd_id);

    let original_pos = match vm.files.seek(fd_num, 0, 1) {
        Ok(pos) => pos as i64,
        Err(_) => {
            memory::write_word(&mut vm.frames.data, frame_base, -1);
            return Ok(());
        }
    };

    let bytes: Vec<u8> = match vm.heap.get(buf_id) {
        Some(obj) => match &obj.data {
            HeapData::Array { data, .. } => data[..count.min(data.len())].to_vec(),
            _ => Vec::new(),
        },
        None => Vec::new(),
    };

    let result = if vm.files.seek(fd_num, offset, 0).is_ok() {
        vm.files.write(fd_num, &bytes).unwrap_or(0) as i32
    } else {
        -1
    };

    let _ = vm.files.seek(fd_num, original_pos, 0);
    memory::write_word(&mut vm.frames.data, frame_base, result);
    Ok(())
}

fn sys_fildes(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let fd_num = memory::read_word(&vm.frames.data, frame_base + 16);
    if vm.files.fildes(fd_num) {
        let mut fd_data = vec![0u8; 4];
        memory::write_word(&mut fd_data, 0, fd_num);
        let fd_id = vm.heap.alloc(0, HeapData::Record(fd_data));
        memory::write_word(&mut vm.frames.data, frame_base, fd_id as i32);
    } else {
        memory::write_word(&mut vm.frames.data, frame_base, 0);
    }
    Ok(())
}

fn sys_fd2path(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let fd_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let fd_num = get_fd_num(vm, fd_id);
    // Portable: return a placeholder since we can't resolve fd→path without OS-specific APIs
    let result = format!("/fd/{fd_num}");
    let str_id = vm.heap.alloc(0, HeapData::Str(result));
    memory::write_word(&mut vm.frames.data, frame_base, str_id as i32);
    Ok(())
}

fn sys_dup(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let old_fd = memory::read_word(&vm.frames.data, frame_base + 16);
    let new_fd = memory::read_word(&vm.frames.data, frame_base + 20);
    let result = vm.files.dup(old_fd, new_fd);
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

fn sys_chdir(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let path_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let path = vm.heap.get_string(path_id).unwrap_or("").to_string();
    let result = if std::env::set_current_dir(&path).is_ok() {
        0
    } else {
        -1
    };
    memory::write_word(&mut vm.frames.data, frame_base, result);
    Ok(())
}

fn sys_remove(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let path_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let path = vm.heap.get_string(path_id).unwrap_or("").to_string();
    let result = if std::fs::remove_file(&path).is_ok() {
        0
    } else {
        -1
    };
    memory::write_word(&mut vm.frames.data, frame_base, result);
    Ok(())
}

fn sys_seek(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let fd_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let offset = memory::read_big(&vm.frames.data, frame_base + 20);
    let whence = memory::read_word(&vm.frames.data, frame_base + 28);
    let fd_num = get_fd_num(vm, fd_id);

    let result = vm
        .files
        .seek(fd_num, offset, whence)
        .map(|p| p as i64)
        .unwrap_or(-1);
    memory::write_big(&mut vm.frames.data, frame_base, result);
    Ok(())
}

fn sys_pipe(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    // Portable pipe is not available in std without platform-specific code.
    // Return -1 (error) as a stub.
    memory::write_word(&mut vm.frames.data, frame_base, -1);
    Ok(())
}

fn sys_iounit(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    // Return a reasonable default IO unit size
    memory::write_word(&mut vm.frames.data, frame_base, 8192);
    Ok(())
}

fn sys_werrstr(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    // Stub: always returns 0 (no error string to set)
    memory::write_word(&mut vm.frames.data, frame_base, 0);
    Ok(())
}

// --- File status ---

/// Build a 60-byte Dir record from std::fs::Metadata.
fn build_dir_record(vm: &mut VmState<'_>, meta: &std::fs::Metadata, name: &str) -> HeapId {
    let mut data = vec![0u8; 60];
    // name string
    let name_id = vm.heap.alloc(0, HeapData::Str(name.to_string()));
    memory::write_word(&mut data, 0, name_id as i32);
    // uid, gid, muid as empty strings
    let empty_id = vm.heap.alloc(0, HeapData::Str(String::new()));
    memory::write_word(&mut data, 4, empty_id as i32);
    vm.heap.inc_ref(empty_id);
    memory::write_word(&mut data, 8, empty_id as i32);
    vm.heap.inc_ref(empty_id);
    memory::write_word(&mut data, 12, empty_id as i32);
    // qid.path (8 bytes big)
    memory::write_big(&mut data, 16, 0);
    // qid.vers, qid.qtype
    memory::write_word(&mut data, 24, 0);
    let qtype = if meta.is_dir() { 0x80 } else { 0 };
    memory::write_word(&mut data, 28, qtype);
    // mode
    #[cfg(unix)]
    let mode_val = {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() as i32
    };
    #[cfg(not(unix))]
    let mode_val = if meta.is_dir() {
        0o755i32 | (0x80000000u32 as i32)
    } else if meta.permissions().readonly() {
        0o444i32
    } else {
        0o644i32
    };
    memory::write_word(&mut data, 32, mode_val);
    // atime, mtime
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i32)
        .unwrap_or(0);
    memory::write_word(&mut data, 36, mtime);
    memory::write_word(&mut data, 40, mtime);
    // length (8 bytes big)
    memory::write_big(&mut data, 44, meta.len() as i64);
    // dtype, dev
    memory::write_word(&mut data, 52, 0);
    memory::write_word(&mut data, 56, 0);
    vm.heap.alloc(0, HeapData::Record(data))
}

fn sys_fstat(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let fd_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let fd_num = get_fd_num(vm, fd_id);
    let path = vm.files.get_path(fd_num).unwrap_or("").to_string();

    match std::fs::metadata(&path) {
        Ok(meta) => {
            let name = std::path::Path::new(&path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            let dir_id = build_dir_record(vm, &meta, name);
            // Return (0, Dir) at frame+0 and frame+4
            memory::write_word(&mut vm.frames.data, frame_base, 0);
            memory::write_word(&mut vm.frames.data, frame_base + 4, dir_id as i32);
        }
        Err(_) => {
            memory::write_word(&mut vm.frames.data, frame_base, -1);
        }
    }
    Ok(())
}

fn sys_stat(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let path_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let path = vm.heap.get_string(path_id).unwrap_or("").to_string();

    match std::fs::metadata(&path) {
        Ok(meta) => {
            let name = std::path::Path::new(&path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            let dir_id = build_dir_record(vm, &meta, name);
            memory::write_word(&mut vm.frames.data, frame_base, 0);
            memory::write_word(&mut vm.frames.data, frame_base + 4, dir_id as i32);
        }
        Err(_) => {
            memory::write_word(&mut vm.frames.data, frame_base, -1);
        }
    }
    Ok(())
}

fn sys_dirread(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let fd_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let fd_num = get_fd_num(vm, fd_id);
    let path = vm.files.get_path(fd_num).unwrap_or("").to_string();

    match std::fs::read_dir(&path) {
        Ok(entries) => {
            let mut dir_ids: Vec<HeapId> = Vec::new();
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let dir_id = build_dir_record(vm, &meta, &name);
                    dir_ids.push(dir_id);
                }
            }
            let count = dir_ids.len();
            // Build array of Dir pointers
            let mut arr_data = vec![0u8; count * 4];
            for (i, &id) in dir_ids.iter().enumerate() {
                memory::write_word(&mut arr_data, i * 4, id as i32);
            }
            let arr_id = vm.heap.alloc(
                0,
                HeapData::Array {
                    elem_type: 0,
                    elem_size: 4,
                    data: arr_data,
                    length: count,
                },
            );
            memory::write_word(&mut vm.frames.data, frame_base, count as i32);
            memory::write_word(&mut vm.frames.data, frame_base + 4, arr_id as i32);
        }
        Err(_) => {
            memory::write_word(&mut vm.frames.data, frame_base, -1);
        }
    }
    Ok(())
}

// --- Namespace operations ---

fn sys_bind(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    // bind is a Plan 9 namespace operation; return 0 (success) as no-op on host.
    memory::write_word(&mut vm.frames.data, frame_base, 0);
    Ok(())
}

fn sys_fauth(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    // Return nil (not supported)
    memory::write_word(&mut vm.frames.data, frame_base, 0);
    Ok(())
}

fn sys_fversion(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    memory::write_word(&mut vm.frames.data, frame_base, -1);
    Ok(())
}

fn sys_file2chan(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    // Return nil
    memory::write_word(&mut vm.frames.data, frame_base, 0);
    Ok(())
}

// --- Networking ---

/// Parse an Inferno dial address "net!host!port" into (host, port).
fn parse_dial_addr(addr: &str) -> Option<(String, u16)> {
    let parts: Vec<&str> = addr.split('!').collect();
    match parts.len() {
        3 => {
            let host = parts[1].to_string();
            let port = parts[2].parse::<u16>().ok()?;
            Some((host, port))
        }
        2 => {
            let host = parts[0].to_string();
            let port = parts[1].parse::<u16>().ok()?;
            Some((host, port))
        }
        _ => None,
    }
}

/// Build a Connection record: { dfd: ref FD, cfd: ref FD, dir: string }
fn build_connection(vm: &mut VmState<'_>, dfd: i32, cfd: i32, dir: &str) -> HeapId {
    let mut conn_data = vec![0u8; 12];
    // dfd record
    let mut dfd_data = vec![0u8; 4];
    memory::write_word(&mut dfd_data, 0, dfd);
    let dfd_id = vm.heap.alloc(0, HeapData::Record(dfd_data));
    memory::write_word(&mut conn_data, 0, dfd_id as i32);
    // cfd record
    let mut cfd_data = vec![0u8; 4];
    memory::write_word(&mut cfd_data, 0, cfd);
    let cfd_id = vm.heap.alloc(0, HeapData::Record(cfd_data));
    memory::write_word(&mut conn_data, 4, cfd_id as i32);
    // dir string
    let dir_id = vm.heap.alloc(0, HeapData::Str(dir.to_string()));
    memory::write_word(&mut conn_data, 8, dir_id as i32);
    vm.heap.alloc(0, HeapData::Record(conn_data))
}

fn sys_dial(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let addr_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let addr = vm.heap.get_string(addr_id).unwrap_or("").to_string();

    match parse_dial_addr(&addr) {
        Some((host, port)) => match TcpStream::connect(format!("{host}:{port}")) {
            Ok(stream) => {
                let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
                let fd = vm.files.insert_tcp_stream(stream, Some(peer.clone()));
                let conn = build_connection(vm, fd, fd, &peer);
                memory::write_word(&mut vm.frames.data, frame_base, 0);
                memory::write_word(&mut vm.frames.data, frame_base + 4, conn as i32);
            }
            Err(_) => {
                memory::write_word(&mut vm.frames.data, frame_base, -1);
                memory::write_word(&mut vm.frames.data, frame_base + 4, 0);
            }
        },
        None => {
            memory::write_word(&mut vm.frames.data, frame_base, -1);
            memory::write_word(&mut vm.frames.data, frame_base + 4, 0);
        }
    }
    Ok(())
}

fn sys_announce(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let addr_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    let addr = vm.heap.get_string(addr_id).unwrap_or("").to_string();

    match parse_dial_addr(&addr) {
        Some((_host, port)) => match TcpListener::bind(format!("0.0.0.0:{port}")) {
            Ok(listener) => {
                let local = listener
                    .local_addr()
                    .map(|a| a.to_string())
                    .unwrap_or_default();
                let lfd = vm.files.insert_tcp_listener(listener, Some(local.clone()));
                let conn = build_connection(vm, -1, lfd, &local);
                memory::write_word(&mut vm.frames.data, frame_base, 0);
                memory::write_word(&mut vm.frames.data, frame_base + 4, conn as i32);
            }
            Err(_) => {
                memory::write_word(&mut vm.frames.data, frame_base, -1);
                memory::write_word(&mut vm.frames.data, frame_base + 4, 0);
            }
        },
        None => {
            memory::write_word(&mut vm.frames.data, frame_base, -1);
            memory::write_word(&mut vm.frames.data, frame_base + 4, 0);
        }
    }
    Ok(())
}

fn sys_listen(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let conn_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
    // Read cfd from Connection record (offset 4)
    let cfd_num = if let Some(obj) = vm.heap.get(conn_id) {
        if let HeapData::Record(data) = &obj.data {
            let cfd_id = memory::read_word(data, 4) as HeapId;
            get_fd_num(vm, cfd_id)
        } else {
            -1
        }
    } else {
        -1
    };

    if cfd_num < 0 {
        memory::write_word(&mut vm.frames.data, frame_base, -1);
        memory::write_word(&mut vm.frames.data, frame_base + 4, 0);
        return Ok(());
    }

    match vm.files.accept_on(cfd_num) {
        Ok((stream_fd, addr)) => {
            let conn = build_connection(vm, stream_fd, cfd_num, &addr);
            memory::write_word(&mut vm.frames.data, frame_base, 0);
            memory::write_word(&mut vm.frames.data, frame_base + 4, conn as i32);
        }
        Err(_) => {
            memory::write_word(&mut vm.frames.data, frame_base, -1);
            memory::write_word(&mut vm.frames.data, frame_base + 4, 0);
        }
    }
    Ok(())
}

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

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use ricevm_core::{
        Header, Instruction, MiddleOperand, Module, Opcode, Operand, PointerMap, RuntimeFlags,
        TypeDescriptor, XMAGIC,
    };

    use super::*;

    fn test_module() -> Module {
        Module {
            header: Header {
                magic: XMAGIC,
                signature: vec![],
                runtime_flags: RuntimeFlags(0),
                stack_extent: 0,
                code_size: 1,
                data_size: 0,
                type_size: 1,
                export_size: 0,
                entry_pc: 0,
                entry_type: 0,
            },
            code: vec![Instruction {
                opcode: Opcode::Exit,
                source: Operand::UNUSED,
                middle: MiddleOperand::UNUSED,
                destination: Operand::UNUSED,
            }],
            types: vec![TypeDescriptor {
                id: 0,
                size: 64,
                pointer_map: PointerMap { bytes: vec![] },
                pointer_count: 0,
            }],
            data: vec![],
            name: "sys_test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        }
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("ricevm_{name}_{nanos}.tmp"))
    }

    fn alloc_fd_record(vm: &mut VmState<'_>, fd: i32) -> HeapId {
        let mut fd_data = vec![0u8; 4];
        memory::write_word(&mut fd_data, 0, fd);
        vm.heap.alloc(0, HeapData::Record(fd_data))
    }

    #[test]
    fn pread_reads_from_offset_without_changing_current_position() {
        let path = temp_path("pread");
        std::fs::write(&path, b"abcdef").expect("temp file write should succeed");

        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let fd = vm
            .files
            .open(path.to_str().expect("temp path should be utf-8"), 2)
            .expect("open should succeed");
        let fd_id = alloc_fd_record(&mut vm, fd);
        let buf_id = vm.heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 1,
                data: vec![0; 3],
                length: 3,
            },
        );
        let frame_base = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, frame_base + 16, fd_id as i32);
        memory::write_word(&mut vm.frames.data, frame_base + 20, buf_id as i32);
        memory::write_word(&mut vm.frames.data, frame_base + 24, 3);
        memory::write_big(&mut vm.frames.data, frame_base + 28, 2);
        vm.files.seek(fd, 5, 0).expect("seek should succeed");

        sys_pread(&mut vm).expect("pread should succeed");

        assert_eq!(memory::read_word(&vm.frames.data, frame_base), 3);
        let pos = vm.files.seek(fd, 0, 1).expect("seek should succeed");
        assert_eq!(pos, 5);
        match &vm.heap.get(buf_id).expect("buffer should exist").data {
            HeapData::Array { data, .. } => assert_eq!(data, b"cde"),
            other => panic!("expected array buffer, got {other:?}"),
        }

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn pwrite_writes_at_offset_without_changing_current_position() {
        let path = temp_path("pwrite");
        std::fs::write(&path, b"abcdef").expect("temp file write should succeed");

        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let fd = vm
            .files
            .open(path.to_str().expect("temp path should be utf-8"), 2)
            .expect("open should succeed");
        let fd_id = alloc_fd_record(&mut vm, fd);
        let buf_id = vm.heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 1,
                data: b"XYZ".to_vec(),
                length: 3,
            },
        );
        let frame_base = vm.frames.current_data_offset();
        memory::write_word(&mut vm.frames.data, frame_base + 16, fd_id as i32);
        memory::write_word(&mut vm.frames.data, frame_base + 20, buf_id as i32);
        memory::write_word(&mut vm.frames.data, frame_base + 24, 3);
        memory::write_big(&mut vm.frames.data, frame_base + 28, 1);
        vm.files.seek(fd, 4, 0).expect("seek should succeed");

        sys_pwrite(&mut vm).expect("pwrite should succeed");

        assert_eq!(memory::read_word(&vm.frames.data, frame_base), 3);
        let pos = vm.files.seek(fd, 0, 1).expect("seek should succeed");
        assert_eq!(pos, 4);
        assert_eq!(
            std::fs::read(&path).expect("temp file read should succeed"),
            b"aXYZef"
        );

        let _ = std::fs::remove_file(path);
    }
}
