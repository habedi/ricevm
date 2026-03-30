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
            bf("announce", 0x0b7c4ac0, 40, sys_stub),
            bf("aprint",   0x77442d46, 64, sys_aprint),
            bf("bind",     0x66326d91, 48, sys_stub),
            bf("byte2char",0x3d6094f9, 40, sys_byte2char),
            bf("char2byte",0x2ba5ab41, 48, sys_char2byte),
            bf("chdir",    0xc6935858, 40, sys_chdir),
            bf("create",   0x54db77d9, 48, sys_create),
            bf("dial",     0x29e90174, 40, sys_stub),
            bf("dirread",  0x72210d71, 40, sys_stub),
            bf("dup",      0x6584767b, 40, sys_dup),
            bf("export",   0x6fc6dc03, 48, sys_stub),
            bf("fauth",    0x20ccc34b, 40, sys_stub),
            bf("fd2path",  0x749c6042, 40, sys_fd2path),
            bf("fildes",   0x1478f993, 40, sys_fildes),
            bf("file2chan", 0x9f34d686, 40, sys_stub),
            bf("fprint",   0xf46486c8, 64, sys_fprint),
            bf("fstat",    0xda4499c2, 40, sys_stub),
            bf("fversion", 0xfe9c0a06, 48, sys_stub),
            bf("fwstat",   0x50a6c7e0, 104, sys_stub),
            bf("iounit",   0x5583b730, 40, sys_iounit),
            bf("listen",   0xb97416e0, 48, sys_stub),
            bf("millisec", 0x616977e8, 32, sys_millisec),
            bf("mount",    0x74c17b3a, 56, sys_stub),
            bf("open",     0x8f477f99, 40, sys_open),
            bf("pctl",     0x05df27fb, 40, sys_pctl),
            bf("pipe",     0x1f2c52ea, 40, sys_pipe),
            bf("pread",    0x09d8aac6, 56, sys_stub),
            bf("print",    0xac849033, 64, sys_print),
            bf("pwrite",   0x09d8aac6, 56, sys_stub),
            bf("read",     0x7cfef557, 48, sys_read),
            bf("readn",    0x7cfef557, 48, sys_read),
            bf("remove",   0xc6935858, 40, sys_remove),
            bf("seek",     0xaeccaddb, 56, sys_seek),
            bf("sleep",    0xe67bf126, 40, sys_sleep),
            bf("sprint",   0x4c0624b6, 64, sys_sprint),
            bf("stat",     0x319328dd, 40, sys_stub),
            bf("stream",   0xb9e8f9ea, 48, sys_stub),
            bf("tokenize", 0x57338f20, 40, sys_tokenize),
            bf("unmount",  0x21e337e3, 40, sys_stub),
            bf("utfbytes", 0x01d4a1f4, 40, sys_utfbytes),
            bf("werrstr",  0xc6935858, 40, sys_werrstr),
            bf("write",    0x7cfef557, 48, sys_write),
            bf("wstat",    0x56b02096, 104, sys_stub),
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

    let seek_whence = match whence {
        0 => libc::SEEK_SET,
        1 => libc::SEEK_CUR,
        2 => libc::SEEK_END,
        _ => libc::SEEK_SET,
    };
    let result = unsafe { libc::lseek(fd_num, offset, seek_whence) };
    memory::write_big(&mut vm.frames.data, frame_base, result);
    Ok(())
}

fn sys_pipe(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let mut fds = [0i32; 2];
    let result = unsafe { libc::pipe(fds.as_mut_ptr()) };

    if result == 0 {
        let arr_id = memory::read_word(&vm.frames.data, frame_base + 16) as HeapId;
        // Write FD objects into the array
        if let Some(obj) = vm.heap.get_mut(arr_id)
            && let HeapData::Array { data, .. } = &mut obj.data
            && data.len() >= 8
        {
            memory::write_word(data, 0, fds[0]);
            memory::write_word(data, 4, fds[1]);
        }
    }
    memory::write_word(&mut vm.frames.data, frame_base, result);
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
