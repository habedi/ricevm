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

// Frame layout for Sys built-in function calls:
//   Offset 0..4:   return value (word or pointer)
//   Offset 4..16:  additional return values or padding
//   Offset 16..20: return address pointer (written by Lea instruction)
//   Offset 20..32: reserved/padding
//   Offset 32+:    arguments
const ARG_START: usize = 32;

fn read_ptr(data: &[u8], offset: usize) -> HeapId {
    memory::read_word(data, offset) as HeapId
}

fn write_ptr(data: &mut [u8], offset: usize, id: HeapId) {
    memory::write_word(data, offset, id as i32);
}

/// Read the return-value pointer stored at frame_base+16 and write a word
/// through it at the given byte offset within the caller's return area.
/// This is required for tuple returns: the callee must write each tuple
/// field directly into the caller's frame via the ret pointer.
fn write_ret_word(vm: &mut VmState<'_>, frame_base: usize, field_offset: usize, val: i32) {
    let ret_ptr = memory::read_word(&vm.frames.data, frame_base + 16);
    if ret_ptr != 0 {
        let target = crate::address::decode_virtual_addr(ret_ptr, 0);
        match target {
            crate::address::AddrTarget::Frame(off) => {
                memory::write_word(&mut vm.frames.data, off + field_offset, val);
            }
            crate::address::AddrTarget::Mp(off) => {
                if off + field_offset + 4 <= vm.mp.len() {
                    memory::write_word(&mut vm.mp, off + field_offset, val);
                }
            }
            _ => {}
        }
    }
    // Also write at frame_base for compatibility with mcall single-word copy
    if field_offset == 0 {
        memory::write_word(&mut vm.frames.data, frame_base, val);
    }
}

/// Format a printf-style string with arguments from the frame.
fn format_string(
    vm: &VmState<'_>,
    frame_base: usize,
    fmt_offset: usize,
    args_offset: usize,
) -> String {
    let fmt_id = read_ptr(&vm.frames.data, frame_base + fmt_offset);
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

        // Parse optional flags: '-', '0', '+', ' '
        let mut left_align = false;
        let mut zero_pad = false;
        let mut plus_sign = false;
        let mut space_sign = false;
        while let Some(&fc) = chars.peek() {
            match fc {
                '-' => {
                    left_align = true;
                    chars.next();
                }
                '0' => {
                    zero_pad = true;
                    chars.next();
                }
                '+' => {
                    plus_sign = true;
                    chars.next();
                }
                ' ' => {
                    space_sign = true;
                    chars.next();
                }
                _ => break,
            }
        }

        // Parse optional width
        let mut width: Option<usize> = None;
        while let Some(&wc) = chars.peek() {
            if wc.is_ascii_digit() {
                chars.next();
                let w = width.unwrap_or(0);
                width = Some(w * 10 + (wc as usize - '0' as usize));
            } else {
                break;
            }
        }

        // Parse optional precision: '.' followed by digits or '*' (from argument)
        let mut precision: Option<usize> = None;
        if chars.peek() == Some(&'.') {
            chars.next();
            if chars.peek() == Some(&'*') {
                chars.next();
                // Precision comes from the next argument (word)
                let p = memory::read_word(&vm.frames.data, arg_offset) as usize;
                arg_offset += 4;
                precision = Some(p);
            } else {
                let mut p: usize = 0;
                while let Some(&pc) = chars.peek() {
                    if pc.is_ascii_digit() {
                        chars.next();
                        p = p * 10 + (pc as usize - '0' as usize);
                    } else {
                        break;
                    }
                }
                precision = Some(p);
            }
        }

        // Helper: apply width/alignment to an already-formatted string
        let apply_width = |s: String| -> String {
            let w = width.unwrap_or(0);
            if w == 0 || s.len() >= w {
                return s;
            }
            if left_align {
                format!("{:<width$}", s, width = w)
            } else if zero_pad {
                // For zero-padding, handle negative numbers specially
                if let Some(digits) = s.strip_prefix('-') {
                    format!("-{:0>width$}", digits, width = w - 1)
                } else {
                    format!("{:0>width$}", s, width = w)
                }
            } else {
                format!("{:>width$}", s, width = w)
            }
        };

        match chars.next() {
            Some('%') => output.push('%'),
            Some('s') => {
                let str_id = read_ptr(&vm.frames.data, arg_offset);
                let s = if let Some(s) = vm.heap.get_string(str_id) {
                    s.to_string()
                } else {
                    "<nil>".to_string()
                };
                let s = if let Some(prec) = precision {
                    if prec < s.len() {
                        s[..prec].to_string()
                    } else {
                        s
                    }
                } else {
                    s
                };
                output.push_str(&apply_width(s));
                arg_offset += 4;
            }
            Some('d') => {
                let val = memory::read_word(&vm.frames.data, arg_offset);
                let s = if let Some(prec) = precision {
                    // Precision for integers means minimum digits
                    let abs_s = format!("{}", val.unsigned_abs());
                    let padded = format!("{:0>width$}", abs_s, width = prec);
                    if val < 0 {
                        format!("-{padded}")
                    } else {
                        padded
                    }
                } else {
                    val.to_string()
                };
                let s = if plus_sign && !s.starts_with('-') {
                    format!("+{s}")
                } else if space_sign && !s.starts_with('-') {
                    format!(" {s}")
                } else {
                    s
                };
                output.push_str(&apply_width(s));
                arg_offset += 4;
            }
            Some(fc @ ('g' | 'f' | 'e')) => {
                let val = memory::read_real(&vm.frames.data, arg_offset);
                let s = match (fc, precision) {
                    ('f', Some(p)) => format!("{val:.prec$}", prec = p),
                    ('f', None) => format!("{val:.6}"),
                    ('e', Some(p)) => format!("{val:.prec$e}", prec = p),
                    ('e', None) => format!("{val:e}"),
                    (_, Some(p)) => format!("{val:.prec$}", prec = p),
                    _ => val.to_string(),
                };
                let s = if plus_sign && !s.starts_with('-') {
                    format!("+{s}")
                } else if space_sign && !s.starts_with('-') {
                    format!(" {s}")
                } else {
                    s
                };
                output.push_str(&apply_width(s));
                arg_offset += 8;
            }
            Some('c') => {
                let val = memory::read_word(&vm.frames.data, arg_offset) as u32;
                if let Some(c) = char::from_u32(val) {
                    let s = c.to_string();
                    output.push_str(&apply_width(s));
                }
                arg_offset += 4;
            }
            Some('x') => {
                let val = memory::read_word(&vm.frames.data, arg_offset);
                let s = if let Some(prec) = precision {
                    format!("{:0>width$x}", val, width = prec)
                } else {
                    format!("{val:x}")
                };
                output.push_str(&apply_width(s));
                arg_offset += 4;
            }
            Some('X') => {
                let val = memory::read_word(&vm.frames.data, arg_offset);
                let s = if let Some(prec) = precision {
                    format!("{:0>width$X}", val, width = prec)
                } else {
                    format!("{val:X}")
                };
                output.push_str(&apply_width(s));
                arg_offset += 4;
            }
            Some('o') => {
                let val = memory::read_word(&vm.frames.data, arg_offset);
                let s = if let Some(prec) = precision {
                    format!("{:0>width$o}", val, width = prec)
                } else {
                    format!("{val:o}")
                };
                output.push_str(&apply_width(s));
                arg_offset += 4;
            }
            Some('r') => {
                if vm.last_error.is_empty() {
                    output.push_str("(no error)");
                } else {
                    output.push_str(&vm.last_error);
                }
            }
            // 'b' prefix: big (64-bit) integer format modifier
            // %bd = big decimal, %bx = big hex, %bo = big octal
            // %bud = big unsigned decimal, %bux = big unsigned hex
            Some('b') => {
                let unsigned = chars.next_if_eq(&'u').is_some();
                match chars.next() {
                    Some('d') => {
                        let val = memory::read_big(&vm.frames.data, arg_offset);
                        let s = if unsigned {
                            (val as u64).to_string()
                        } else {
                            val.to_string()
                        };
                        output.push_str(&apply_width(s));
                    }
                    Some('x') => {
                        let val = memory::read_big(&vm.frames.data, arg_offset) as u64;
                        let s = if let Some(prec) = precision {
                            format!("{:0>width$x}", val, width = prec)
                        } else {
                            format!("{val:x}")
                        };
                        output.push_str(&apply_width(s));
                    }
                    Some('X') => {
                        let val = memory::read_big(&vm.frames.data, arg_offset) as u64;
                        let s = if let Some(prec) = precision {
                            format!("{:0>width$X}", val, width = prec)
                        } else {
                            format!("{val:X}")
                        };
                        output.push_str(&apply_width(s));
                    }
                    Some('o') => {
                        let val = memory::read_big(&vm.frames.data, arg_offset) as u64;
                        let s = format!("{val:o}");
                        output.push_str(&apply_width(s));
                    }
                    _ => {
                        output.push_str("<bad %b>");
                    }
                }
                arg_offset += 8; // big values are 8 bytes
            }
            // 'u' prefix for unsigned word (rarely used without 'b')
            Some('u') => {
                match chars.next() {
                    Some('d') => {
                        let val = memory::read_word(&vm.frames.data, arg_offset) as u32;
                        output.push_str(&apply_width(val.to_string()));
                    }
                    Some('x') => {
                        let val = memory::read_word(&vm.frames.data, arg_offset) as u32;
                        let s = if let Some(prec) = precision {
                            format!("{:0>width$x}", val, width = prec)
                        } else {
                            format!("{val:x}")
                        };
                        output.push_str(&apply_width(s));
                    }
                    _ => {
                        output.push_str("<bad %u>");
                    }
                }
                arg_offset += 4;
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
            bf("fwstat", 0x50a6c7e0, 104, sys_fwstat),
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
            bf("stream", 0xb9e8f9ea, 48, sys_stream),
            bf("tokenize", 0x57338f20, 40, sys_tokenize),
            bf("unmount", 0x21e337e3, 40, sys_stub), // unsupported on host OS
            bf("utfbytes", 0x01d4a1f4, 40, sys_utfbytes),
            bf("werrstr", 0xc6935858, 40, sys_werrstr),
            bf("write", 0x7cfef557, 48, sys_write),
            bf("wstat", 0x56b02096, 104, sys_wstat),
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
    let output = format_string(vm, frame_base, ARG_START, ARG_START + 4);
    let len = output.len() as i32;
    print!("{output}");
    memory::write_word(&mut vm.frames.data, frame_base, len);
    Ok(())
}

fn sys_fprint(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let fd_id = read_ptr(&vm.frames.data, frame_base + ARG_START);
    let fd_num = get_fd_num(vm, fd_id);
    let fd_num = if fd_num < 0 { 1 } else { fd_num }; // default to stdout
    let output = format_string(vm, frame_base, ARG_START + 4, ARG_START + 8);
    let len = output.len() as i32;
    let _ = vm.files.write(fd_num, output.as_bytes());
    memory::write_word(&mut vm.frames.data, frame_base, len);
    Ok(())
}

fn sys_sprint(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let output = format_string(vm, frame_base, ARG_START, ARG_START + 4);
    let str_id = vm.heap.alloc(0, HeapData::Str(output));
    // Return string pointer at frame offset 0
    write_ptr(&mut vm.frames.data, frame_base, str_id);
    Ok(())
}

fn sys_aprint(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let output = format_string(vm, frame_base, ARG_START, ARG_START + 4);
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
    write_ptr(&mut vm.frames.data, frame_base, arr_id);
    Ok(())
}

// --- File I/O (portable, no libc) ---

fn sys_open(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let path_id = read_ptr(&vm.frames.data, frame_base + ARG_START);
    let mode = memory::read_word(&vm.frames.data, frame_base + ARG_START + 4);

    let path = vm.heap.get_string(path_id).unwrap_or("").to_string();
    match vm.files.open(&path, mode) {
        Ok(fd) => {
            let mut fd_data = vec![0u8; 4];
            memory::write_word(&mut fd_data, 0, fd);
            let fd_id = vm.heap.alloc(0, HeapData::Record(fd_data));
            write_ptr(&mut vm.frames.data, frame_base, fd_id);
        }
        Err(e) => {
            vm.last_error = format!("{e}");
            memory::write_word(&mut vm.frames.data, frame_base, 0);
        }
    }
    Ok(())
}

fn sys_create(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let path_id = read_ptr(&vm.frames.data, frame_base + ARG_START);

    let path = vm.heap.get_string(path_id).unwrap_or("").to_string();
    match vm.files.create(&path) {
        Ok(fd) => {
            let mut fd_data = vec![0u8; 4];
            memory::write_word(&mut fd_data, 0, fd);
            let fd_id = vm.heap.alloc(0, HeapData::Record(fd_data));
            write_ptr(&mut vm.frames.data, frame_base, fd_id);
        }
        Err(e) => {
            vm.last_error = format!("{e}");
            memory::write_word(&mut vm.frames.data, frame_base, 0);
        }
    }
    Ok(())
}

fn sys_read(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let fd_id = read_ptr(&vm.frames.data, frame_base + ARG_START);
    let buf_id = read_ptr(&vm.frames.data, frame_base + ARG_START + 4);
    let count = memory::read_word(&vm.frames.data, frame_base + ARG_START + 8) as usize;

    let fd_num = get_fd_num(vm, fd_id);
    let mut tmp = vec![0u8; count];
    let n = match vm.files.read(fd_num, &mut tmp) {
        Ok(n) => n as i32,
        Err(e) => {
            vm.last_error = format!("{e}");
            -1
        }
    };

    if n > 0 {
        vm.heap.array_write(buf_id, 0, &tmp[..(n as usize)]);
    }

    memory::write_word(&mut vm.frames.data, frame_base, n);
    Ok(())
}

fn sys_write(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let fd_id = read_ptr(&vm.frames.data, frame_base + ARG_START);
    let buf_id = read_ptr(&vm.frames.data, frame_base + ARG_START + 4);
    let count = memory::read_word(&vm.frames.data, frame_base + ARG_START + 8) as usize;

    let fd_num = get_fd_num(vm, fd_id);
    let bytes: Vec<u8> = vm.heap.array_read(buf_id, 0, count).unwrap_or_default();

    let n = match vm.files.write(fd_num, &bytes) {
        Ok(n) => n as i32,
        Err(e) => {
            vm.last_error = format!("{e}");
            0
        }
    };
    memory::write_word(&mut vm.frames.data, frame_base, n);
    Ok(())
}

fn sys_pread(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let fd_id = read_ptr(&vm.frames.data, frame_base + ARG_START);
    let buf_id = read_ptr(&vm.frames.data, frame_base + ARG_START + 4);
    let count = memory::read_word(&vm.frames.data, frame_base + ARG_START + 8) as usize;
    // big (8 bytes) is aligned to 8-byte boundary: fd(4)+buf(4)+n(4)=offset 44,
    // aligned up to 48 = ARG_START + 16
    let offset = memory::read_big(&vm.frames.data, frame_base + ARG_START + 16);
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

        if n > 0 {
            vm.heap.array_write(buf_id, 0, &tmp[..(n as usize)]);
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
    let fd_id = read_ptr(&vm.frames.data, frame_base + ARG_START);
    let buf_id = read_ptr(&vm.frames.data, frame_base + ARG_START + 4);
    let count = memory::read_word(&vm.frames.data, frame_base + ARG_START + 8) as usize;
    // big (8 bytes) is aligned to 8-byte boundary: fd(4)+buf(4)+n(4)=offset 44,
    // aligned up to 48 = ARG_START + 16
    let offset = memory::read_big(&vm.frames.data, frame_base + ARG_START + 16);
    let fd_num = get_fd_num(vm, fd_id);

    let original_pos = match vm.files.seek(fd_num, 0, 1) {
        Ok(pos) => pos as i64,
        Err(_) => {
            memory::write_word(&mut vm.frames.data, frame_base, -1);
            return Ok(());
        }
    };

    let bytes: Vec<u8> = vm.heap.array_read(buf_id, 0, count).unwrap_or_default();

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
    let fd_num = memory::read_word(&vm.frames.data, frame_base + ARG_START);
    if vm.files.fildes(fd_num) {
        let mut fd_data = vec![0u8; 4];
        memory::write_word(&mut fd_data, 0, fd_num);
        let fd_id = vm.heap.alloc(0, HeapData::Record(fd_data));
        write_ptr(&mut vm.frames.data, frame_base, fd_id);
    } else {
        memory::write_word(&mut vm.frames.data, frame_base, 0);
    }
    Ok(())
}

fn sys_fd2path(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let fd_id = read_ptr(&vm.frames.data, frame_base + ARG_START);
    let fd_num = get_fd_num(vm, fd_id);
    // Use the file table's stored path when available, fall back to placeholder
    let result = match vm.files.get_path(fd_num) {
        Some(p) if !p.is_empty() => p.to_string(),
        _ => format!("/fd/{fd_num}"),
    };
    let str_id = vm.heap.alloc(0, HeapData::Str(result));
    write_ptr(&mut vm.frames.data, frame_base, str_id);
    Ok(())
}

fn sys_dup(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let old_fd = memory::read_word(&vm.frames.data, frame_base + ARG_START);
    let new_fd = memory::read_word(&vm.frames.data, frame_base + ARG_START + 4);
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
    let period_ms = memory::read_word(&vm.frames.data, frame_base + ARG_START);
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
    let str_id = read_ptr(&vm.frames.data, frame_base + ARG_START);
    let delim_id = read_ptr(&vm.frames.data, frame_base + ARG_START + 4);

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

    // Return (count, list):write through ret pointer into caller's frame
    write_ret_word(vm, frame_base, 0, count);
    write_ret_word(vm, frame_base, 4, list_id as i32);
    Ok(())
}

fn sys_byte2char(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let buf_id = read_ptr(&vm.frames.data, frame_base + ARG_START);
    let n = memory::read_word(&vm.frames.data, frame_base + ARG_START + 4) as usize;

    // byte2char(buf, n): convert UTF-8 bytes starting at buf[n] to a Unicode codepoint.
    // Determine the available bytes from offset n, then read only what exists.
    let buf_len = vm.heap.array_byte_len(buf_id).unwrap_or(0);
    let avail = buf_len.saturating_sub(n);
    let read_len = avail.min(6);
    let (char_val, bytes_consumed, status) = if read_len == 0 {
        (0, 0, -1)
    } else if let Some(data) = vm.heap.array_read(buf_id, n, read_len) {
        // Determine expected UTF-8 sequence length from first byte
        let first = data[0];
        let seq_len = if first < 0x80 {
            1
        } else if first < 0xE0 {
            2
        } else if first < 0xF0 {
            3
        } else {
            4
        };
        if seq_len <= data.len() {
            match std::str::from_utf8(&data[..seq_len]) {
                Ok(s) => match s.chars().next() {
                    Some(ch) => (ch as i32, ch.len_utf8() as i32, 0),
                    None => (0, 0, -1),
                },
                Err(_) => (0xFFFD, 1, 0),
            }
        } else {
            (0xFFFD, 1, 0)
        }
    } else {
        (0, 0, -1)
    };

    // Return (char, bytes_consumed, status):write through ret pointer
    write_ret_word(vm, frame_base, 0, char_val);
    write_ret_word(vm, frame_base, 4, bytes_consumed);
    write_ret_word(vm, frame_base, 8, status);
    Ok(())
}

fn sys_char2byte(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let ch_val = memory::read_word(&vm.frames.data, frame_base + ARG_START) as u32;
    let buf_id = read_ptr(&vm.frames.data, frame_base + ARG_START + 4);
    let n = memory::read_word(&vm.frames.data, frame_base + ARG_START + 8) as usize;

    let ch = char::from_u32(ch_val).unwrap_or('\u{FFFD}');
    let mut utf8_buf = [0u8; 4];
    let encoded = ch.encode_utf8(&mut utf8_buf);
    let bytes_written = encoded.len();

    vm.heap.array_write(buf_id, n, &utf8_buf[..bytes_written]);

    memory::write_word(&mut vm.frames.data, frame_base, bytes_written as i32);
    Ok(())
}

fn sys_utfbytes(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let buf_id = read_ptr(&vm.frames.data, frame_base + ARG_START);
    let n = memory::read_word(&vm.frames.data, frame_base + ARG_START + 4) as usize;

    let result = if let Some(data) = vm.heap.array_read(buf_id, 0, n) {
        // Find the last valid UTF-8 boundary
        match std::str::from_utf8(&data) {
            Ok(s) => s.len() as i32,
            Err(e) => e.valid_up_to() as i32,
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
    let path_id = read_ptr(&vm.frames.data, frame_base + ARG_START);
    let raw_path = vm.heap.get_string(path_id).unwrap_or("").to_string();
    // Map Inferno /usr/<name> to the host home directory
    let path = if let Some(user) = raw_path.strip_prefix("/usr/") {
        std::env::var("HOME").unwrap_or_else(|_| format!("/home/{user}"))
    } else {
        vm.files.resolve_path(&raw_path)
    };
    let result = match std::env::set_current_dir(&path) {
        Ok(()) => 0,
        Err(e) => {
            vm.last_error = format!("{e}");
            -1
        }
    };
    memory::write_word(&mut vm.frames.data, frame_base, result);
    Ok(())
}

fn sys_remove(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let path_id = read_ptr(&vm.frames.data, frame_base + ARG_START);
    let path = vm.heap.get_string(path_id).unwrap_or("").to_string();
    let result = match std::fs::remove_file(&path) {
        Ok(()) => 0,
        Err(e) => {
            vm.last_error = format!("{e}");
            -1
        }
    };
    memory::write_word(&mut vm.frames.data, frame_base, result);
    Ok(())
}

fn sys_seek(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let fd_id = read_ptr(&vm.frames.data, frame_base + ARG_START);
    // Big values are aligned to 8 bytes: fd at +32 (4 bytes), padding +36,
    // offset at +40 (8 bytes), whence at +48 (4 bytes)
    let offset = memory::read_big(&vm.frames.data, frame_base + ARG_START + 8);
    let whence = memory::read_word(&vm.frames.data, frame_base + ARG_START + 16);
    let fd_num = get_fd_num(vm, fd_id);

    // Non-seekable standard streams: return 0 position without actually seeking.
    // Bufio calls seek on stdin to get the current position, and an error would
    // make it think the file is invalid.
    if (0..=2).contains(&fd_num) {
        memory::write_big(&mut vm.frames.data, frame_base, 0i64);
        let ret_ptr = memory::read_word(&vm.frames.data, frame_base + 16);
        if ret_ptr > 0 && (ret_ptr as usize) + 8 <= vm.frames.data.len() {
            memory::write_big(&mut vm.frames.data, ret_ptr as usize, 0i64);
        }
        return Ok(());
    }

    let result = match vm.files.seek(fd_num, offset, whence) {
        Ok(p) => p as i64,
        Err(e) => {
            vm.last_error = format!("{e}");
            -1
        }
    };
    // Write result at frame offset 0 (standard) AND through the return
    // pointer at offset 16. seek returns a big (8 bytes), and the 4-byte
    // mcall return copy can't handle that, so write directly.
    memory::write_big(&mut vm.frames.data, frame_base, result);
    let ret_ptr = memory::read_word(&vm.frames.data, frame_base + 16);
    if ret_ptr > 0 && (ret_ptr as usize) + 8 <= vm.frames.data.len() {
        memory::write_big(&mut vm.frames.data, ret_ptr as usize, result);
    }
    Ok(())
}

fn sys_pipe(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let fds_arr_id = read_ptr(&vm.frames.data, frame_base + ARG_START);

    // Create an in-memory pipe (read_fd, write_fd).
    let (read_fd, write_fd) = vm.files.pipe();

    // Allocate FD records on the heap (4-byte Record containing fd number).
    let mut fd0_data = vec![0u8; 4];
    memory::write_word(&mut fd0_data, 0, read_fd);
    let fd0_id = vm.heap.alloc(0, HeapData::Record(fd0_data));

    let mut fd1_data = vec![0u8; 4];
    memory::write_word(&mut fd1_data, 0, write_fd);
    let fd1_id = vm.heap.alloc(0, HeapData::Record(fd1_data));

    // Write the FD record pointers into the array at indices 0 and 1.
    // Each element is a 4-byte pointer (HeapId).
    let mut ptr_bytes = [0u8; 4];
    memory::write_word(&mut ptr_bytes, 0, fd0_id as i32);
    vm.heap.array_write(fds_arr_id, 0, &ptr_bytes);

    memory::write_word(&mut ptr_bytes, 0, fd1_id as i32);
    vm.heap.array_write(fds_arr_id, 4, &ptr_bytes);

    // Return 0 (success).
    memory::write_word(&mut vm.frames.data, frame_base, 0);
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
    let str_id = read_ptr(&vm.frames.data, frame_base + ARG_START);
    let msg = vm.heap.get_string(str_id).unwrap_or("").to_string();
    vm.last_error = msg;
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
    let fd_id = read_ptr(&vm.frames.data, frame_base + ARG_START);
    let fd_num = get_fd_num(vm, fd_id);
    let path = vm.files.get_path(fd_num).unwrap_or("").to_string();

    match std::fs::metadata(&path) {
        Ok(meta) => {
            let name = std::path::Path::new(&path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            let dir_id = build_dir_record(vm, &meta, name);
            // Return (0, Dir):write through ret pointer into caller's frame
            write_ret_word(vm, frame_base, 0, 0);
            write_ret_word(vm, frame_base, 4, dir_id as i32);
        }
        Err(e) => {
            vm.last_error = format!("{e}");
            write_ret_word(vm, frame_base, 0, -1);
        }
    }
    Ok(())
}

fn sys_stat(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let path_id = read_ptr(&vm.frames.data, frame_base + ARG_START);
    let path = vm.heap.get_string(path_id).unwrap_or("").to_string();

    match std::fs::metadata(&path) {
        Ok(meta) => {
            let name = std::path::Path::new(&path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            let dir_id = build_dir_record(vm, &meta, name);
            // Return (0, Dir):write through ret pointer into caller's frame
            write_ret_word(vm, frame_base, 0, 0);
            write_ret_word(vm, frame_base, 4, dir_id as i32);
        }
        Err(e) => {
            vm.last_error = format!("{e}");
            write_ret_word(vm, frame_base, 0, -1);
        }
    }
    Ok(())
}

fn sys_dirread(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let fd_id = read_ptr(&vm.frames.data, frame_base + ARG_START);
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
            // Return (count, array of Dir):write through ret pointer
            write_ret_word(vm, frame_base, 0, count as i32);
            write_ret_word(vm, frame_base, 4, arr_id as i32);
        }
        Err(e) => {
            vm.last_error = format!("{e}");
            write_ret_word(vm, frame_base, 0, -1);
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
    // fversion returns (int, string):write through ret pointer
    write_ret_word(vm, frame_base, 0, -1);
    write_ret_word(vm, frame_base, 4, 0); // nil string
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
    let addr_id = read_ptr(&vm.frames.data, frame_base + ARG_START);
    let addr = vm.heap.get_string(addr_id).unwrap_or("").to_string();

    match parse_dial_addr(&addr) {
        Some((host, port)) => match TcpStream::connect(format!("{host}:{port}")) {
            Ok(stream) => {
                let peer = stream
                    .peer_addr()
                    .map(|a| a.to_string())
                    .unwrap_or_default();
                let fd = vm.files.insert_tcp_stream(stream, Some(peer.clone()));
                let conn = build_connection(vm, fd, fd, &peer);
                // Return (0, Connection):write through ret pointer
                write_ret_word(vm, frame_base, 0, 0);
                write_ret_word(vm, frame_base, 4, conn as i32);
            }
            Err(e) => {
                vm.last_error = format!("{e}");
                write_ret_word(vm, frame_base, 0, -1);
                write_ret_word(vm, frame_base, 4, 0);
            }
        },
        None => {
            vm.last_error = "invalid dial address".to_string();
            write_ret_word(vm, frame_base, 0, -1);
            write_ret_word(vm, frame_base, 4, 0);
        }
    }
    Ok(())
}

fn sys_announce(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let addr_id = read_ptr(&vm.frames.data, frame_base + ARG_START);
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
                // Return (0, Connection):write through ret pointer
                write_ret_word(vm, frame_base, 0, 0);
                write_ret_word(vm, frame_base, 4, conn as i32);
            }
            Err(e) => {
                vm.last_error = format!("{e}");
                write_ret_word(vm, frame_base, 0, -1);
                write_ret_word(vm, frame_base, 4, 0);
            }
        },
        None => {
            vm.last_error = "invalid announce address".to_string();
            write_ret_word(vm, frame_base, 0, -1);
            write_ret_word(vm, frame_base, 4, 0);
        }
    }
    Ok(())
}

fn sys_listen(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let conn_id = read_ptr(&vm.frames.data, frame_base + ARG_START);
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
        write_ret_word(vm, frame_base, 0, -1);
        write_ret_word(vm, frame_base, 4, 0);
        return Ok(());
    }

    match vm.files.accept_on(cfd_num) {
        Ok((stream_fd, addr)) => {
            let conn = build_connection(vm, stream_fd, cfd_num, &addr);
            // Return (0, Connection):write through ret pointer
            write_ret_word(vm, frame_base, 0, 0);
            write_ret_word(vm, frame_base, 4, conn as i32);
        }
        Err(e) => {
            vm.last_error = format!("{e}");
            write_ret_word(vm, frame_base, 0, -1);
            write_ret_word(vm, frame_base, 4, 0);
        }
    }
    Ok(())
}

/// Set file metadata (permissions) for a named file.
/// wstat(s: string, d: Dir): int
/// Frame: ARG_START+0 = path string ptr, ARG_START+4 = Dir record ptr
fn sys_wstat(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let path_id = read_ptr(&vm.frames.data, frame_base + ARG_START);
    let dir_id = read_ptr(&vm.frames.data, frame_base + ARG_START + 4);
    let path = vm.heap.get_string(path_id).unwrap_or("").to_string();

    let result = apply_dir_permissions(vm, &path, dir_id);
    memory::write_word(&mut vm.frames.data, frame_base, result);
    Ok(())
}

/// Set file metadata (permissions) for an open fd.
/// fwstat(fd: ref FD, d: Dir): int
/// Frame: ARG_START+0 = fd record ptr, ARG_START+4 = Dir record ptr
fn sys_fwstat(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let fd_id = read_ptr(&vm.frames.data, frame_base + ARG_START);
    let dir_id = read_ptr(&vm.frames.data, frame_base + ARG_START + 4);
    let fd_num = get_fd_num(vm, fd_id);
    let path = vm.files.get_path(fd_num).unwrap_or("").to_string();

    let result = if path.is_empty() {
        vm.last_error = "unknown file".to_string();
        -1
    } else {
        apply_dir_permissions(vm, &path, dir_id)
    };
    memory::write_word(&mut vm.frames.data, frame_base, result);
    Ok(())
}

/// Read the mode field from a Dir record (offset 32) and apply it as file permissions.
fn apply_dir_permissions(vm: &mut VmState<'_>, path: &str, dir_id: HeapId) -> i32 {
    let mode = if let Some(obj) = vm.heap.get(dir_id) {
        if let HeapData::Record(data) = &obj.data {
            if data.len() >= 36 {
                // mode is at offset 32 in the Dir record (see build_dir_record)
                memory::read_word(data, 32)
            } else {
                return -1;
            }
        } else {
            return -1;
        }
    } else {
        return -1;
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(mode as u32);
        match std::fs::set_permissions(path, perms) {
            Ok(()) => 0,
            Err(e) => {
                vm.last_error = format!("{e}");
                -1
            }
        }
    }
    #[cfg(not(unix))]
    {
        // On non-Unix, we can only set readonly status
        let readonly = (mode & 0o222) == 0;
        let mut perms = match std::fs::metadata(path) {
            Ok(m) => m.permissions(),
            Err(e) => {
                vm.last_error = format!("{e}");
                return -1;
            }
        };
        perms.set_readonly(readonly);
        match std::fs::set_permissions(path, perms) {
            Ok(()) => 0,
            Err(e) => {
                vm.last_error = format!("{e}");
                -1
            }
        }
    }
}

/// Continuously read from src and write to dst until EOF.
/// stream(src, dst: ref FD, bufsiz: int): int
fn sys_stream(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    let src_fd_id = read_ptr(&vm.frames.data, frame_base + ARG_START);
    let dst_fd_id = read_ptr(&vm.frames.data, frame_base + ARG_START + 4);
    let bufsiz = memory::read_word(&vm.frames.data, frame_base + ARG_START + 8) as usize;

    let src_fd = get_fd_num(vm, src_fd_id);
    let dst_fd = get_fd_num(vm, dst_fd_id);
    let bufsiz = if bufsiz == 0 { 8192 } else { bufsiz };

    let mut total: i64 = 0;
    let mut buf = vec![0u8; bufsiz];
    loop {
        let n = match vm.files.read(src_fd, &mut buf) {
            Ok(0) => break, // EOF
            Ok(n) => n,
            Err(e) => {
                vm.last_error = format!("{e}");
                memory::write_word(&mut vm.frames.data, frame_base, -1);
                return Ok(());
            }
        };
        match vm.files.write(dst_fd, &buf[..n]) {
            Ok(w) => total += w as i64,
            Err(e) => {
                vm.last_error = format!("{e}");
                memory::write_word(&mut vm.frames.data, frame_base, -1);
                return Ok(());
            }
        }
    }
    memory::write_word(&mut vm.frames.data, frame_base, total as i32);
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
        write_ptr(&mut vm.frames.data, frame_base + ARG_START, fd_id);
        write_ptr(&mut vm.frames.data, frame_base + ARG_START + 4, buf_id);
        memory::write_word(&mut vm.frames.data, frame_base + ARG_START + 8, 3);
        // big offset is aligned to 8 bytes: ARG_START+12 = 44, rounds up to 48 = ARG_START+16
        memory::write_big(&mut vm.frames.data, frame_base + ARG_START + 16, 2);
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
        write_ptr(&mut vm.frames.data, frame_base + ARG_START, fd_id);
        write_ptr(&mut vm.frames.data, frame_base + ARG_START + 4, buf_id);
        memory::write_word(&mut vm.frames.data, frame_base + ARG_START + 8, 3);
        // big offset is aligned to 8 bytes: ARG_START+12 = 44, rounds up to 48 = ARG_START+16
        memory::write_big(&mut vm.frames.data, frame_base + ARG_START + 16, 1);
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

    /// Helper: set up a VmState with a format string and arguments, then call format_string.
    fn run_format(fmt: &str, setup_args: impl FnOnce(&mut VmState<'_>, usize)) -> String {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let frame_base = vm.frames.current_data_offset();

        // Write format string to heap and store pointer at ARG_START
        let fmt_id = vm.heap.alloc(0, HeapData::Str(fmt.to_string()));
        write_ptr(&mut vm.frames.data, frame_base + ARG_START, fmt_id);

        // Let the caller set up arguments starting at ARG_START + 4
        setup_args(&mut vm, frame_base + ARG_START + 4);

        format_string(&vm, frame_base, ARG_START, ARG_START + 4)
    }

    #[test]
    fn format_string_width_d() {
        let result = run_format("%7d", |vm, off| {
            memory::write_word(&mut vm.frames.data, off, 42);
        });
        assert_eq!(result, "     42");
    }

    #[test]
    fn format_string_precision_f() {
        let result = run_format("%.2f", |vm, off| {
            memory::write_real(&mut vm.frames.data, off, 3.14159);
        });
        assert_eq!(result, "3.14");
    }

    #[test]
    fn format_string_left_align_s() {
        let result = run_format("%-10s", |vm, off| {
            let s_id = vm.heap.alloc(0, HeapData::Str("hi".to_string()));
            write_ptr(&mut vm.frames.data, off, s_id);
        });
        assert_eq!(result, "hi        ");
    }

    #[test]
    fn format_string_zero_pad_x() {
        let result = run_format("%07x", |vm, off| {
            memory::write_word(&mut vm.frames.data, off, 0x1a2b);
        });
        assert_eq!(result, "0001a2b");
    }

    #[test]
    fn byte2char_decodes_ascii() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let buf_id = vm.heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 1,
                data: vec![0x41, 0x42, 0x43], // "ABC"
                length: 3,
            },
        );
        let frame_base = vm.frames.current_data_offset();
        // Use a frame scratch area as return pointer target
        let ret_off = frame_base + 48;
        memory::write_word(&mut vm.frames.data, frame_base + 16, ret_off as i32);
        write_ptr(&mut vm.frames.data, frame_base + ARG_START, buf_id);
        memory::write_word(&mut vm.frames.data, frame_base + ARG_START + 4, 0);

        sys_byte2char(&mut vm).expect("byte2char should succeed");

        let char_val = memory::read_word(&vm.frames.data, ret_off);
        let bytes_consumed = memory::read_word(&vm.frames.data, ret_off + 4);
        let status = memory::read_word(&vm.frames.data, ret_off + 8);
        assert_eq!(char_val, 0x41, "should decode 'A'");
        assert_eq!(bytes_consumed, 1, "ASCII is 1 byte");
        assert_eq!(status, 0, "status should be success");
    }

    #[test]
    fn byte2char_decodes_multibyte_utf8() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        // α = U+03B1, UTF-8: 0xCE 0xB1
        let buf_id = vm.heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 1,
                data: vec![0xCE, 0xB1, 0x00],
                length: 3,
            },
        );
        let frame_base = vm.frames.current_data_offset();
        let ret_off = frame_base + 48;
        memory::write_word(&mut vm.frames.data, frame_base + 16, ret_off as i32);
        write_ptr(&mut vm.frames.data, frame_base + ARG_START, buf_id);
        memory::write_word(&mut vm.frames.data, frame_base + ARG_START + 4, 0);

        sys_byte2char(&mut vm).expect("byte2char should succeed");

        let char_val = memory::read_word(&vm.frames.data, ret_off);
        let bytes_consumed = memory::read_word(&vm.frames.data, ret_off + 4);
        let status = memory::read_word(&vm.frames.data, ret_off + 8);
        assert_eq!(char_val, 945, "should decode α (U+03B1)");
        assert_eq!(bytes_consumed, 2, "α is 2 UTF-8 bytes");
        assert_eq!(status, 0, "status should be success");
    }

    #[test]
    fn byte2char_reads_from_offset_n() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        // Put ASCII 'X' at offset 0, then α (0xCE, 0xB1) at offset 1
        let buf_id = vm.heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 1,
                data: vec![0x58, 0xCE, 0xB1, 0x00],
                length: 4,
            },
        );
        let frame_base = vm.frames.current_data_offset();
        let ret_off = frame_base + 48;
        memory::write_word(&mut vm.frames.data, frame_base + 16, ret_off as i32);
        write_ptr(&mut vm.frames.data, frame_base + ARG_START, buf_id);
        memory::write_word(&mut vm.frames.data, frame_base + ARG_START + 4, 1); // n=1

        sys_byte2char(&mut vm).expect("byte2char should succeed");

        let char_val = memory::read_word(&vm.frames.data, ret_off);
        let bytes_consumed = memory::read_word(&vm.frames.data, ret_off + 4);
        assert_eq!(char_val, 945, "should decode α at offset 1");
        assert_eq!(bytes_consumed, 2, "α is 2 UTF-8 bytes");
    }
}
