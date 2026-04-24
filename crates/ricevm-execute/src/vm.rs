//! VM execution state and main loop.

use ricevm_core::{Big, Byte, ExecError, Instruction, Module, Pc, Real, Word};

use crate::address::{self, AddrTarget};
use crate::builtin::ModuleRegistry;
use crate::data;
use crate::frame::FrameStack;
use crate::heap::{self, Heap, HeapId};
use crate::memory;
use crate::ops;
use crate::sys;

/// A Dis module loaded from a .dis file at runtime.
pub(crate) struct LoadedModule {
    pub module: ricevm_core::Module,
    pub mp: Vec<u8>,
}

pub(crate) struct VmState<'m> {
    pub module: &'m Module,
    pub mp: Vec<u8>,
    pub frames: FrameStack,
    pub heap: Heap,
    pub modules: ModuleRegistry,
    pub loaded_modules: Vec<LoadedModule>,
    pub files: crate::filetab::FileTable,
    pub pc: usize,
    pub next_pc: usize,
    pub halted: bool,
    pub trace: bool,
    pub gc_enabled: bool,
    pub(crate) gc_counter: usize,
    /// Index of the currently executing loaded module (None = main module).
    pub(crate) current_loaded_module: Option<usize>,
    /// Inferno root directory for path resolution (empty = use host paths directly).
    pub(crate) root_path: String,

    // Resolved operand targets for the current instruction.
    pub src: AddrTarget,
    pub mid: AddrTarget,
    pub dst: AddrTarget,

    // Scratch storage for immediate values.
    pub imm_src: Word,
    pub imm_mid: Word,
    pub imm_dst: Word,

    /// Per-thread error string (set by werrstr, read by %r format specifier).
    pub(crate) last_error: String,

    /// Stack of caller MP buffers for cross-module MP address resolution.
    /// When a loaded module executes, the caller's MP is pushed here so that
    /// virtual addresses targeting the caller's module can be resolved.
    /// Each entry is (module_virt_idx, mp_data).
    pub(crate) caller_mp_stack: Vec<(usize, Vec<u8>)>,

    // Heap array reference table for indx results.
    pub heap_refs: Vec<(heap::HeapId, usize)>,

    /// Set by recv/alt when channel has no data; the run loop suspends the thread.
    pub(crate) blocked_channel: Option<heap::HeapId>,

    /// Queue of suspended threads for cooperative scheduling.
    pub(crate) thread_queue: std::collections::VecDeque<SuspendedThread>,
}

/// Per-thread state saved when a thread is suspended.
#[derive(Debug)]
pub(crate) struct SuspendedThread {
    pub frames: FrameStack,
    pub mp: Vec<u8>,
    pub pc: usize,
    pub heap_refs: Vec<(heap::HeapId, usize)>,
    pub last_error: String,
    pub current_loaded_module: Option<usize>,
    pub caller_mp_stack: Vec<(usize, Vec<u8>)>,
    /// None = ready to run, Some(chan_id) = blocked waiting for data on channel.
    /// Some(0) = blocked on alt (any channel send unblocks).
    pub blocked_on: Option<heap::HeapId>,
}

// $Keyring/$Crypt digest handlers using real MD4, MD5, SHA1, SHA224, SHA256,
// SHA384, and SHA512

/// DigestState stored as a heap record. The first 4 bytes contain a "kind" tag
/// (0=md5, 1=sha1, 2=sha224, 3=sha256, 4=sha384, 5=sha512, 6=md4), followed by
/// the accumulated input bytes.
const DIGEST_MD5: i32 = 0;
const DIGEST_SHA1: i32 = 1;
const DIGEST_SHA224: i32 = 2;
const DIGEST_SHA256: i32 = 3;
const DIGEST_SHA384: i32 = 4;
const DIGEST_SHA512: i32 = 5;
const DIGEST_MD4: i32 = 6;

fn keyring_md4(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    keyring_digest(vm, DIGEST_MD4)
}

fn keyring_md5(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    keyring_digest(vm, DIGEST_MD5)
}

fn keyring_sha1(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    keyring_digest(vm, DIGEST_SHA1)
}

fn keyring_sha224(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    keyring_digest(vm, DIGEST_SHA224)
}

fn keyring_sha256(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    keyring_digest(vm, DIGEST_SHA256)
}

fn keyring_sha384(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    keyring_digest(vm, DIGEST_SHA384)
}

fn keyring_sha512(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    keyring_digest(vm, DIGEST_SHA512)
}

/// md4/md5/sha1/sha224/sha256/sha384/sha512(
///     data: array of byte,
///     n: int,
///     digest: array of byte,
///     state: ref DigestState,
/// ): ref DigestState
///
/// The DigestState is a heap record storing: [kind(4 bytes), accumulated_data...]
/// When digest is non-nil, we finalize the hash and write the result.
fn keyring_digest(vm: &mut VmState<'_>, kind: i32) -> Result<(), ExecError> {
    use crate::heap::{HeapData, HeapId};

    let frame_base = vm.frames.current_data_offset();
    let data_id = memory::read_word(&vm.frames.data, frame_base + 32) as HeapId;
    let n = memory::read_word(&vm.frames.data, frame_base + 36) as usize;
    let digest_id = memory::read_word(&vm.frames.data, frame_base + 40) as HeapId;
    let state_id = memory::read_word(&vm.frames.data, frame_base + 44) as HeapId;

    // Read input data
    let input = if data_id != heap::NIL && n > 0 {
        vm.heap.array_read(data_id, 0, n).unwrap_or_default()
    } else {
        Vec::new()
    };

    // Get or create accumulated data from state
    let mut accumulated = if state_id != heap::NIL {
        if let Some(obj) = vm.heap.get(state_id) {
            if let HeapData::Record(data) = &obj.data {
                // Skip first 4 bytes (kind tag), rest is accumulated data
                if data.len() > 4 {
                    data[4..].to_vec()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    // Append new input
    accumulated.extend_from_slice(&input);

    // If digest array is provided, finalize and write the hash
    if digest_id != heap::NIL {
        let digest_bytes: Vec<u8> = match kind {
            DIGEST_MD4 => {
                use md4::Digest;
                let mut hasher = md4::Md4::new();
                hasher.update(&accumulated);
                hasher.finalize().to_vec()
            }
            DIGEST_MD5 => {
                use md5::Digest;
                let mut hasher = md5::Md5::new();
                hasher.update(&accumulated);
                hasher.finalize().to_vec()
            }
            DIGEST_SHA1 => {
                use sha1::Digest;
                let mut hasher = sha1::Sha1::new();
                hasher.update(&accumulated);
                hasher.finalize().to_vec()
            }
            DIGEST_SHA224 => {
                use sha2::Digest;
                let mut hasher = sha2::Sha224::new();
                hasher.update(&accumulated);
                hasher.finalize().to_vec()
            }
            DIGEST_SHA256 => {
                use sha2::Digest;
                let mut hasher = sha2::Sha256::new();
                hasher.update(&accumulated);
                hasher.finalize().to_vec()
            }
            DIGEST_SHA384 => {
                use sha2::Digest;
                let mut hasher = sha2::Sha384::new();
                hasher.update(&accumulated);
                hasher.finalize().to_vec()
            }
            DIGEST_SHA512 => {
                use sha2::Digest;
                let mut hasher = sha2::Sha512::new();
                hasher.update(&accumulated);
                hasher.finalize().to_vec()
            }
            _ => Vec::new(),
        };
        vm.heap.array_write(digest_id, 0, &digest_bytes);
    }

    // Return updated DigestState: [kind(4), accumulated_data...]
    let mut state_data = vec![0u8; 4 + accumulated.len()];
    memory::write_word(&mut state_data, 0, kind);
    state_data[4..].copy_from_slice(&accumulated);
    let new_state_id = vm.heap.alloc(0, HeapData::Record(state_data));
    memory::write_word(&mut vm.frames.data, frame_base, new_state_id as i32);
    Ok(())
}

fn keyring_return_nil(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    memory::write_word(&mut vm.frames.data, frame_base, 0);
    Ok(())
}

fn keyring_return_zero(vm: &mut VmState<'_>) -> Result<(), ExecError> {
    let frame_base = vm.frames.current_data_offset();
    memory::write_word(&mut vm.frames.data, frame_base, 0);
    Ok(())
}

impl<'m> VmState<'m> {
    pub fn new(module: &'m Module) -> Result<Self, ExecError> {
        Self::with_args(module, Vec::new())
    }

    pub fn with_args(module: &'m Module, args: Vec<String>) -> Result<Self, ExecError> {
        let mut heap = Heap::new();
        let mp = data::init_mp_with_types(
            module.header.data_size as usize,
            &module.data,
            &mut heap,
            &module.types,
        );
        let mut frames = FrameStack::new();

        // Push the initial entry frame.
        let entry_type_idx = module.header.entry_type;
        let frame_size = if entry_type_idx >= 0 && (entry_type_idx as usize) < module.types.len() {
            module.types[entry_type_idx as usize].size as usize
        } else {
            // No type descriptor: use a default frame size.
            64
        };
        frames.push_entry(frame_size, -1); // -1 sentinel for "no caller"

        // Set up entry frame args for the Limbo init() convention:
        // fp[32] = ref Draw->Context
        // fp[36] = list of string (program name)
        if frame_size >= 40 {
            // Create Draw->Context record:
            //   offset 0: ref Display (pointer)
            //   offset 4: ref Screen (pointer)
            //   offset 8: wm channel (pointer)
            let mut ctx_data = vec![0u8; 16];
            // Create a Display record (simplified)
            let display_id = heap.alloc(0, heap::HeapData::Record(vec![0u8; 32]));
            memory::write_word(&mut ctx_data, 0, display_id as i32);
            // Create a Screen record
            let screen_id = heap.alloc(0, heap::HeapData::Record(vec![0u8; 16]));
            memory::write_word(&mut ctx_data, 4, screen_id as i32);
            // Leave wm channel (offset 8) as nil so standalone mode is used
            // by wmlib->connect(). Non-nil wm triggers /chan/wmctl lookup which
            // requires a running Inferno window manager.
            let ctx_id = heap.alloc(0, heap::HeapData::Record(ctx_data));

            // Build args list: [module_name, arg1, arg2, ...] as a linked list.
            // Construct in reverse so the list is in forward order.
            let mut all_args = vec![module.name.clone()];
            all_args.extend(args.iter().cloned());

            let mut args_list = heap::NIL;
            for arg in all_args.iter().rev() {
                let str_id = heap.alloc(0, heap::HeapData::Str(arg.clone()));
                let mut head_buf = vec![0u8; 4];
                memory::write_word(&mut head_buf, 0, str_id as i32);
                if args_list != heap::NIL {
                    heap.inc_ref(args_list);
                }
                args_list = heap.alloc(
                    0,
                    heap::HeapData::List {
                        head: head_buf,
                        tail: args_list,
                    },
                );
            }

            let fp_base = frames.current_data_offset();
            // fp[32] = Draw->Context
            if fp_base + 36 <= frames.data.len() {
                memory::write_word(&mut frames.data, fp_base + 32, ctx_id as i32);
            }
            // fp[36] = args list
            if fp_base + 40 <= frames.data.len() {
                memory::write_word(&mut frames.data, fp_base + 36, args_list as i32);
            }
        }

        let mut modules = ModuleRegistry::new();
        modules.register(sys::create_sys_module());
        modules.register(crate::math::create_math_module());
        modules.register(crate::draw::create_draw_module());
        modules.register(crate::tk::create_tk_module());
        modules.register(crate::builtin::BuiltinModule {
            name: "$Crypt",
            funcs: vec![
                crate::builtin::BuiltinFunc {
                    name: "md5",
                    sig: 0x07656377,
                    frame_size: 56,
                    handler: keyring_md5,
                },
                crate::builtin::BuiltinFunc {
                    name: "sha1",
                    sig: 0x07656377,
                    frame_size: 56,
                    handler: keyring_sha1,
                },
                crate::builtin::BuiltinFunc {
                    name: "sha224",
                    sig: 0x07656377,
                    frame_size: 56,
                    handler: keyring_sha224,
                },
                crate::builtin::BuiltinFunc {
                    name: "sha256",
                    sig: 0x07656377,
                    frame_size: 56,
                    handler: keyring_sha256,
                },
                crate::builtin::BuiltinFunc {
                    name: "sha384",
                    sig: 0x07656377,
                    frame_size: 56,
                    handler: keyring_sha384,
                },
                crate::builtin::BuiltinFunc {
                    name: "sha512",
                    sig: 0x07656377,
                    frame_size: 56,
                    handler: keyring_sha512,
                },
                crate::builtin::BuiltinFunc {
                    name: "md4",
                    sig: 0x07656377,
                    frame_size: 56,
                    handler: keyring_md4,
                },
            ],
        });
        modules.register(crate::builtin::BuiltinModule {
            name: "$Keyring",
            funcs: vec![
                crate::builtin::BuiltinFunc {
                    name: "md5",
                    sig: 0x07656377,
                    frame_size: 56,
                    handler: keyring_md5,
                },
                crate::builtin::BuiltinFunc {
                    name: "sha1",
                    sig: 0x07656377,
                    frame_size: 56,
                    handler: keyring_sha1,
                },
                crate::builtin::BuiltinFunc {
                    name: "sha224",
                    sig: 0x07656377,
                    frame_size: 56,
                    handler: keyring_sha224,
                },
                crate::builtin::BuiltinFunc {
                    name: "sha256",
                    sig: 0x07656377,
                    frame_size: 56,
                    handler: keyring_sha256,
                },
                crate::builtin::BuiltinFunc {
                    name: "sha384",
                    sig: 0x07656377,
                    frame_size: 56,
                    handler: keyring_sha384,
                },
                crate::builtin::BuiltinFunc {
                    name: "sha512",
                    sig: 0x07656377,
                    frame_size: 56,
                    handler: keyring_sha512,
                },
                crate::builtin::BuiltinFunc {
                    name: "md4",
                    sig: 0x07656377,
                    frame_size: 56,
                    handler: keyring_md4,
                },
                crate::builtin::BuiltinFunc {
                    name: "readauthinfo",
                    sig: 0xb2c82015,
                    frame_size: 40,
                    handler: keyring_return_nil,
                },
                crate::builtin::BuiltinFunc {
                    name: "writeauthinfo",
                    sig: 0, // accept any
                    frame_size: 40,
                    handler: keyring_return_zero,
                },
                crate::builtin::BuiltinFunc {
                    name: "getstring",
                    sig: 0,
                    frame_size: 40,
                    handler: keyring_return_nil,
                },
                crate::builtin::BuiltinFunc {
                    name: "putstring",
                    sig: 0,
                    frame_size: 40,
                    handler: keyring_return_zero,
                },
                crate::builtin::BuiltinFunc {
                    name: "getbytearray",
                    sig: 0,
                    frame_size: 40,
                    handler: keyring_return_nil,
                },
                crate::builtin::BuiltinFunc {
                    name: "putbytearray",
                    sig: 0,
                    frame_size: 40,
                    handler: keyring_return_zero,
                },
                crate::builtin::BuiltinFunc {
                    name: "auth",
                    sig: 0,
                    frame_size: 48,
                    handler: keyring_return_nil,
                },
            ],
        });

        let trace = std::env::var("RICEVM_TRACE").is_ok();
        let gc_enabled = std::env::var("RICEVM_NO_GC").is_err();
        let root_path = std::env::var("RICEVM_ROOT").unwrap_or_default();

        Ok(Self {
            module,
            mp,
            frames,
            heap,
            modules,
            loaded_modules: Vec::new(),
            files: crate::filetab::FileTable::with_root(root_path.clone()),
            pc: module.header.entry_pc as usize,
            next_pc: 0,
            halted: false,
            trace,
            gc_enabled,
            gc_counter: 0,
            current_loaded_module: None,
            root_path,
            last_error: String::new(),
            src: AddrTarget::None,
            mid: AddrTarget::None,
            dst: AddrTarget::None,
            imm_src: 0,
            imm_mid: 0,
            imm_dst: 0,
            caller_mp_stack: Vec::new(),
            blocked_channel: None,
            thread_queue: std::collections::VecDeque::new(),
            heap_refs: Vec::new(),
        })
    }

    /// Get the type descriptor size for the currently executing module.
    pub(crate) fn current_type_size(&self, type_idx: usize) -> Option<usize> {
        if let Some(lm_idx) = self.current_loaded_module {
            self.loaded_modules
                .get(lm_idx)
                .and_then(|lm| lm.module.types.get(type_idx))
                .map(|td| td.size as usize)
        } else {
            self.module.types.get(type_idx).map(|td| td.size as usize)
        }
    }

    /// Check if a type (by index) contains any pointer fields.
    pub fn run(&mut self) -> Result<(), ExecError> {
        // Library modules with entry_pc = -1 have no init function.
        if self.module.header.entry_pc < 0 {
            return Ok(());
        }

        const GC_INTERVAL: usize = 10_000;
        const THREAD_QUANTUM: usize = 2048;
        let mut quantum_counter = 0usize;

        loop {
            if self.halted {
                // Current thread halted: check for other threads
                if self.thread_queue.is_empty() {
                    return Ok(());
                }
                self.resume_next_thread();
                continue;
            }

            let code_len = if let Some(lm_idx) = self.current_loaded_module {
                self.loaded_modules[lm_idx].module.code.len()
            } else {
                self.module.code.len()
            };
            if self.pc >= code_len {
                if self.current_loaded_module.is_some() {
                    // Loaded module finished; shouldn't happen normally
                    self.halted = true;
                    continue;
                }
                return Err(ExecError::InvalidPc(self.pc as Pc));
            }

            let inst = if let Some(lm_idx) = self.current_loaded_module {
                self.loaded_modules[lm_idx].module.code[self.pc].clone()
            } else {
                self.module.code[self.pc].clone()
            };
            if self.trace {
                self.trace_instruction(&inst);
            }
            self.resolve_operands(&inst)?;
            self.next_pc = self.pc + 1;
            match ops::dispatch(self, &inst) {
                Ok(()) => {}
                Err(ExecError::ThreadFault(ref msg))
                    if msg.contains("nil")
                        || msg.contains("out of bounds")
                        || msg.contains("not a module") =>
                {
                    // Try to find an exception handler for nil dereferences
                    self.raise_exception(msg)?;
                }
                Err(e) => return Err(e),
            }

            // Check if the instruction blocked on a channel (recv/alt with no data)
            if let Some(chan_id) = self.blocked_channel.take() {
                // Don't advance PC; will re-execute the recv/alt when unblocked
                if self.thread_queue.is_empty() {
                    // No other threads; can't block, just continue (return zeros)
                    self.pc = self.next_pc;
                } else {
                    self.suspend_as_blocked(chan_id);
                    self.resume_next_ready_thread();
                }
                continue;
            }

            self.pc = self.next_pc;

            // Thread quantum: switch if other threads are waiting
            quantum_counter += 1;
            if quantum_counter >= THREAD_QUANTUM && !self.thread_queue.is_empty() {
                quantum_counter = 0;
                self.suspend_and_rotate();
            }

            // Periodic GC
            if self.gc_enabled {
                self.gc_counter += 1;
                if self.gc_counter >= GC_INTERVAL {
                    self.gc_counter = 0;
                    crate::gc::collect(
                        &mut self.heap,
                        &self.frames,
                        &self.mp,
                        &self.loaded_modules,
                        &self.thread_queue,
                        &self.caller_mp_stack,
                    );
                }
            }
        }
    }

    /// Suspend the current thread and move to the next one in the queue.
    fn suspend_and_rotate(&mut self) {
        let suspended = SuspendedThread {
            frames: std::mem::replace(&mut self.frames, FrameStack::new()),
            mp: std::mem::take(&mut self.mp),
            pc: self.pc,
            heap_refs: std::mem::take(&mut self.heap_refs),
            last_error: std::mem::take(&mut self.last_error),
            current_loaded_module: self.current_loaded_module.take(),
            caller_mp_stack: std::mem::take(&mut self.caller_mp_stack),
            blocked_on: None,
        };
        self.thread_queue.push_back(suspended);
        self.resume_next_ready_thread();
    }

    /// Suspend the current thread as blocked on a channel.
    fn suspend_as_blocked(&mut self, chan_id: heap::HeapId) {
        let suspended = SuspendedThread {
            frames: std::mem::replace(&mut self.frames, FrameStack::new()),
            mp: std::mem::take(&mut self.mp),
            pc: self.pc, // DON'T advance; will re-execute recv/alt when unblocked
            heap_refs: std::mem::take(&mut self.heap_refs),
            last_error: std::mem::take(&mut self.last_error),
            current_loaded_module: self.current_loaded_module.take(),
            caller_mp_stack: std::mem::take(&mut self.caller_mp_stack),
            blocked_on: Some(chan_id),
        };
        self.thread_queue.push_back(suspended);
    }

    /// Unblock threads waiting on a specific channel (called after send).
    /// Raise a VM exception. Searches the current module's handler table
    /// for a matching handler at the current PC. If found, jumps to the handler.
    /// If not found, returns a ThreadFault error.
    pub(crate) fn raise_exception(&mut self, msg: &str) -> Result<(), ExecError> {
        let current_pc = self.pc as i32;
        let str_id = self.heap.alloc(0, heap::HeapData::Str(msg.to_string()));

        // Search current module's handler table
        let handlers = if let Some(lm_idx) = self.current_loaded_module {
            self.loaded_modules
                .get(lm_idx)
                .map(|lm| &lm.module.handlers)
        } else {
            Some(&self.module.handlers)
        };

        if let Some(handlers) = handlers {
            for handler in handlers {
                if current_pc < handler.begin_pc || current_pc >= handler.end_pc {
                    continue;
                }
                for case in &handler.cases {
                    let matches = match &case.name {
                        Some(name) => msg.starts_with(name.as_str()),
                        None => true, // wildcard
                    };
                    if matches {
                        self.next_pc = case.pc as usize;
                        let frame_base = self.frames.current_data_offset();
                        let off = frame_base + handler.exception_offset as usize;
                        if off + 4 <= self.frames.data.len() {
                            crate::memory::write_word(&mut self.frames.data, off, str_id as i32);
                        }
                        return Ok(());
                    }
                }
            }
        }

        Err(ExecError::ThreadFault(format!(
            "unhandled exception: {msg}"
        )))
    }

    pub(crate) fn unblock_channel(&mut self, chan_id: heap::HeapId) {
        for thread in self.thread_queue.iter_mut() {
            if let Some(blocked_id) = thread.blocked_on {
                // Unblock if waiting on this channel OR waiting on any channel (alt with id=0)
                if blocked_id == chan_id || blocked_id == 0 {
                    thread.blocked_on = None;
                }
            }
        }
    }

    /// Resume the next READY (non-blocked) thread from the queue.
    fn resume_next_ready_thread(&mut self) {
        let len = self.thread_queue.len();
        for _ in 0..len {
            if let Some(thread) = self.thread_queue.pop_front() {
                if thread.blocked_on.is_none() {
                    // Ready: resume it
                    self.load_thread(thread);
                    return;
                }
                // Still blocked; put back
                self.thread_queue.push_back(thread);
            }
        }
        // No ready threads: deadlock or all blocked
        self.halted = true;
    }

    /// Resume the next thread from the queue (any state).
    fn resume_next_thread(&mut self) {
        if let Some(thread) = self.thread_queue.pop_front() {
            self.load_thread(thread);
        }
    }

    fn load_thread(&mut self, thread: SuspendedThread) {
        self.frames = thread.frames;
        self.mp = thread.mp;
        self.pc = thread.pc;
        self.heap_refs = thread.heap_refs;
        self.last_error = thread.last_error;
        self.current_loaded_module = thread.current_loaded_module;
        self.caller_mp_stack = thread.caller_mp_stack;
        self.halted = false;
    }

    pub(crate) fn trace_instruction(&self, inst: &Instruction) {
        use ricevm_core::{AddressMode, MiddleMode};
        let mut parts = Vec::new();
        parts.push(format!("{:4}: {:?}", self.pc, inst.opcode));

        if inst.source.mode != AddressMode::None {
            parts.push(format_operand("src", &inst.source));
        }
        if inst.middle.mode != MiddleMode::None {
            parts.push(format_mid("mid", &inst.middle));
        }
        if inst.destination.mode != AddressMode::None {
            parts.push(format_operand("dst", &inst.destination));
        }
        eprintln!("{}", parts.join(" "));
    }

    pub(crate) fn resolve_operands(&mut self, inst: &Instruction) -> Result<(), ExecError> {
        let fp_base = self.frames.current_data_offset();

        self.imm_src = inst.source.register1;
        self.src = address::resolve_operand_with_heap(
            &inst.source,
            fp_base,
            &self.frames.data,
            &self.mp,
            &self.heap_refs,
            Some(&self.heap),
        )?;

        self.imm_mid = inst.middle.register1;
        self.mid = address::resolve_middle(&inst.middle, fp_base)?;

        self.imm_dst = inst.destination.register1;
        self.dst = address::resolve_operand_with_heap(
            &inst.destination,
            fp_base,
            &self.frames.data,
            &self.mp,
            &self.heap_refs,
            Some(&self.heap),
        )?;

        Ok(())
    }

    // --- Value read helpers ---

    /// Get a read-only copy of bytes from a heap object at the given offset.
    pub(crate) fn heap_slice(
        &self,
        id: heap::HeapId,
        offset: usize,
        len: usize,
    ) -> Option<Vec<u8>> {
        let obj = self.heap.get(id)?;
        match &obj.data {
            heap::HeapData::Array { data, .. }
            | heap::HeapData::Record(data)
            | heap::HeapData::Adt { data, .. } => {
                if offset + len <= data.len() {
                    Some(data[offset..offset + len].to_vec())
                } else {
                    Some(vec![0u8; len])
                }
            }
            heap::HeapData::ArraySlice {
                parent_id,
                byte_start,
                ..
            } => self.heap_slice(*parent_id, byte_start + offset, len),
            _ => Some(vec![0u8; len]),
        }
    }

    /// Write bytes to a heap array element. Resolves ArraySlice to parent.
    pub(crate) fn heap_write(&mut self, id: heap::HeapId, offset: usize, bytes: &[u8]) {
        // Check for ArraySlice first and redirect to parent.
        if let Some(obj) = self.heap.get(id)
            && let heap::HeapData::ArraySlice {
                parent_id,
                byte_start,
                ..
            } = &obj.data
        {
            let pid = *parent_id;
            let bs = *byte_start;
            self.heap_write(pid, bs + offset, bytes);
            return;
        }
        if let Some(obj) = self.heap.get_mut(id) {
            match &mut obj.data {
                heap::HeapData::Array { data, .. }
                | heap::HeapData::Record(data)
                | heap::HeapData::Adt { data, .. }
                    if offset + bytes.len() <= data.len() =>
                {
                    data[offset..offset + bytes.len()].copy_from_slice(bytes);
                }
                _ => {}
            }
        }
    }

    /// Return the virtual module index for the currently executing module.
    /// Module 0 = main, module N+1 = loaded_modules[N].
    pub(crate) fn current_module_virt_idx(&self) -> usize {
        match self.current_loaded_module {
            None => 0,
            Some(idx) => idx + 1,
        }
    }

    /// Get a reference to a module's MP by virtual index.
    /// If the requested module is the currently executing one, returns self.mp.
    /// Also checks the caller_mp_stack for MPs that have been swapped out
    /// during cross-module calls.
    pub(crate) fn module_mp(&self, module_idx: usize) -> Option<&Vec<u8>> {
        if module_idx == self.current_module_virt_idx() {
            return Some(&self.mp);
        }
        // Check the caller_mp_stack (for parent modules whose MP was swapped out)
        for (virt_idx, mp) in self.caller_mp_stack.iter().rev() {
            if *virt_idx == module_idx {
                return Some(mp);
            }
        }
        // Check loaded_modules (for non-active loaded modules)
        if module_idx == 0 {
            // Main module not in stack and not current -- not accessible
            None
        } else {
            self.loaded_modules.get(module_idx - 1).map(|lm| &lm.mp)
        }
    }

    /// Get a mutable reference to a module's MP by virtual index.
    pub(crate) fn module_mp_mut(&mut self, module_idx: usize) -> Option<&mut Vec<u8>> {
        if module_idx == self.current_module_virt_idx() {
            return Some(&mut self.mp);
        }
        for (virt_idx, mp) in self.caller_mp_stack.iter_mut().rev() {
            if *virt_idx == module_idx {
                return Some(mp);
            }
        }
        if module_idx == 0 {
            None
        } else {
            self.loaded_modules
                .get_mut(module_idx - 1)
                .map(|lm| &mut lm.mp)
        }
    }

    pub(crate) fn read_word_at(&self, target: AddrTarget, imm: Word) -> Result<Word, ExecError> {
        match target {
            AddrTarget::Frame(off) => Ok(memory::read_word(&self.frames.data, off)),
            AddrTarget::Mp(off) => {
                if off + 4 <= self.mp.len() {
                    Ok(memory::read_word(&self.mp, off))
                } else {
                    Ok(0)
                }
            }
            AddrTarget::ModuleMp { module_idx, offset } => {
                let mp = match self.module_mp(module_idx) {
                    Some(mp) => mp,
                    None => return Ok(0),
                };
                if offset + 4 <= mp.len() {
                    Ok(memory::read_word(mp, offset))
                } else {
                    Ok(0)
                }
            }
            AddrTarget::Immediate => Ok(imm),
            AddrTarget::None => Ok(0),
            AddrTarget::HeapArray { id, offset } => Ok(self
                .heap_slice(id, offset, 4)
                .map(|b| memory::read_word(&b, 0))
                .unwrap_or(0)),
        }
    }

    pub(crate) fn write_word_at(&mut self, target: AddrTarget, val: Word) -> Result<(), ExecError> {
        match target {
            AddrTarget::Frame(off) => {
                memory::write_word(&mut self.frames.data, off, val);
                Ok(())
            }
            AddrTarget::Mp(off) => {
                if off + 4 <= self.mp.len() {
                    memory::write_word(&mut self.mp, off, val);
                }
                Ok(())
            }
            AddrTarget::ModuleMp { module_idx, offset } => {
                if let Some(mp) = self.module_mp_mut(module_idx)
                    && offset + 4 <= mp.len()
                {
                    memory::write_word(mp, offset, val);
                }
                Ok(())
            }
            AddrTarget::Immediate => Err(ExecError::Other("cannot write to immediate".to_string())),
            AddrTarget::None => Ok(()),
            AddrTarget::HeapArray { id, offset } => {
                let mut buf = [0u8; 4];
                memory::write_word(&mut buf, 0, val);
                self.heap_write(id, offset, &buf);
                Ok(())
            }
        }
    }

    pub(crate) fn read_big_at(&self, target: AddrTarget, imm: Word) -> Result<Big, ExecError> {
        match target {
            AddrTarget::Frame(off) => Ok(memory::read_big(&self.frames.data, off)),
            AddrTarget::Mp(off) => Ok(memory::read_big(&self.mp, off)),
            AddrTarget::ModuleMp { module_idx, offset } => {
                let mp = match self.module_mp(module_idx) {
                    Some(mp) => mp,
                    None => return Ok(0),
                };
                if offset + 8 <= mp.len() {
                    Ok(memory::read_big(mp, offset))
                } else {
                    Ok(0)
                }
            }
            AddrTarget::Immediate => Ok(imm as Big),
            AddrTarget::None => Ok(0),
            AddrTarget::HeapArray { id, offset } => Ok(self
                .heap_slice(id, offset, 8)
                .map(|b| memory::read_big(&b, 0))
                .unwrap_or(0)),
        }
    }

    pub(crate) fn write_big_at(&mut self, target: AddrTarget, val: Big) -> Result<(), ExecError> {
        match target {
            AddrTarget::Frame(off) => {
                memory::write_big(&mut self.frames.data, off, val);
                Ok(())
            }
            AddrTarget::Mp(off) => {
                memory::write_big(&mut self.mp, off, val);
                Ok(())
            }
            AddrTarget::ModuleMp { module_idx, offset } => {
                if let Some(mp) = self.module_mp_mut(module_idx)
                    && offset + 8 <= mp.len()
                {
                    memory::write_big(mp, offset, val);
                }
                Ok(())
            }
            AddrTarget::Immediate => Err(ExecError::Other("cannot write to immediate".to_string())),
            AddrTarget::None => Ok(()),
            AddrTarget::HeapArray { id, offset } => {
                let mut buf = [0u8; 8];
                memory::write_big(&mut buf, 0, val);
                self.heap_write(id, offset, &buf);
                Ok(())
            }
        }
    }

    pub(crate) fn read_real_at(&self, target: AddrTarget, _imm: Word) -> Result<Real, ExecError> {
        match target {
            AddrTarget::Frame(off) => Ok(memory::read_real(&self.frames.data, off)),
            AddrTarget::Mp(off) => Ok(memory::read_real(&self.mp, off)),
            AddrTarget::ModuleMp { module_idx, offset } => {
                let mp = match self.module_mp(module_idx) {
                    Some(mp) => mp,
                    None => return Ok(0.0),
                };
                if offset + 8 <= mp.len() {
                    Ok(memory::read_real(mp, offset))
                } else {
                    Ok(0.0)
                }
            }
            AddrTarget::Immediate => Ok(0.0),
            AddrTarget::None => Ok(0.0),
            AddrTarget::HeapArray { id, offset } => Ok(self
                .heap_slice(id, offset, 8)
                .map(|b| memory::read_real(&b, 0))
                .unwrap_or(0.0)),
        }
    }

    pub(crate) fn write_real_at(&mut self, target: AddrTarget, val: Real) -> Result<(), ExecError> {
        match target {
            AddrTarget::Frame(off) => {
                memory::write_real(&mut self.frames.data, off, val);
                Ok(())
            }
            AddrTarget::Mp(off) => {
                memory::write_real(&mut self.mp, off, val);
                Ok(())
            }
            AddrTarget::ModuleMp { module_idx, offset } => {
                if let Some(mp) = self.module_mp_mut(module_idx)
                    && offset + 8 <= mp.len()
                {
                    memory::write_real(mp, offset, val);
                }
                Ok(())
            }
            AddrTarget::Immediate => Err(ExecError::Other("cannot write to immediate".to_string())),
            AddrTarget::None => Ok(()),
            AddrTarget::HeapArray { id, offset } => {
                let mut buf = [0u8; 8];
                memory::write_real(&mut buf, 0, val);
                self.heap_write(id, offset, &buf);
                Ok(())
            }
        }
    }

    pub(crate) fn read_byte_at(&self, target: AddrTarget, imm: Word) -> Result<Byte, ExecError> {
        match target {
            AddrTarget::Frame(off) => Ok(memory::read_byte(&self.frames.data, off)),
            AddrTarget::Mp(off) => Ok(memory::read_byte(&self.mp, off)),
            AddrTarget::ModuleMp { module_idx, offset } => {
                let mp = match self.module_mp(module_idx) {
                    Some(mp) => mp,
                    None => return Ok(0),
                };
                if offset < mp.len() {
                    Ok(memory::read_byte(mp, offset))
                } else {
                    Ok(0)
                }
            }
            AddrTarget::Immediate => Ok(imm as Byte),
            AddrTarget::None => Ok(0),
            AddrTarget::HeapArray { id, offset } => {
                Ok(self.heap_slice(id, offset, 1).map(|b| b[0]).unwrap_or(0))
            }
        }
    }

    pub(crate) fn write_byte_at(&mut self, target: AddrTarget, val: Byte) -> Result<(), ExecError> {
        match target {
            AddrTarget::Frame(off) => {
                memory::write_byte(&mut self.frames.data, off, val);
                Ok(())
            }
            AddrTarget::Mp(off) => {
                memory::write_byte(&mut self.mp, off, val);
                Ok(())
            }
            AddrTarget::ModuleMp { module_idx, offset } => {
                if let Some(mp) = self.module_mp_mut(module_idx)
                    && offset < mp.len()
                {
                    memory::write_byte(mp, offset, val);
                }
                Ok(())
            }
            AddrTarget::Immediate => Err(ExecError::Other("cannot write to immediate".to_string())),
            AddrTarget::None => Ok(()),
            AddrTarget::HeapArray { id, offset } => {
                self.heap_write(id, offset, &[val]);
                Ok(())
            }
        }
    }

    // --- Convenience accessors for src/mid/dst ---

    pub(crate) fn src_word(&self) -> Result<Word, ExecError> {
        self.read_word_at(self.src, self.imm_src)
    }
    pub(crate) fn mid_word(&self) -> Result<Word, ExecError> {
        self.read_word_at(self.mid, self.imm_mid)
    }
    pub(crate) fn dst_word(&self) -> Result<Word, ExecError> {
        self.read_word_at(self.dst, self.imm_dst)
    }
    pub(crate) fn set_dst_word(&mut self, val: Word) -> Result<(), ExecError> {
        self.write_word_at(self.dst, val)
    }

    pub(crate) fn src_big(&self) -> Result<Big, ExecError> {
        self.read_big_at(self.src, self.imm_src)
    }
    pub(crate) fn mid_big(&self) -> Result<Big, ExecError> {
        self.read_big_at(self.mid, self.imm_mid)
    }
    pub(crate) fn set_dst_big(&mut self, val: Big) -> Result<(), ExecError> {
        self.write_big_at(self.dst, val)
    }

    pub(crate) fn src_real(&self) -> Result<Real, ExecError> {
        self.read_real_at(self.src, self.imm_src)
    }
    pub(crate) fn mid_real(&self) -> Result<Real, ExecError> {
        self.read_real_at(self.mid, self.imm_mid)
    }
    #[allow(dead_code)]
    pub(crate) fn dst_real(&self) -> Result<Real, ExecError> {
        self.read_real_at(self.dst, self.imm_dst)
    }
    pub(crate) fn set_dst_real(&mut self, val: Real) -> Result<(), ExecError> {
        self.write_real_at(self.dst, val)
    }

    pub(crate) fn src_byte(&self) -> Result<Byte, ExecError> {
        self.read_byte_at(self.src, self.imm_src)
    }
    pub(crate) fn mid_byte(&self) -> Result<Byte, ExecError> {
        self.read_byte_at(self.mid, self.imm_mid)
    }
    pub(crate) fn set_dst_byte(&mut self, val: Byte) -> Result<(), ExecError> {
        self.write_byte_at(self.dst, val)
    }

    // --- Two-operand arithmetic helpers ---
    // In Dis, `op src, dst` (no mid) means `dst = dst OP src`.
    // When mid is present, `op src, mid, dst` means `dst = src OP mid`.
    // These helpers return the effective second operand for arithmetic.

    pub(crate) fn mid_or_dst_word(&self) -> Result<Word, ExecError> {
        if self.mid == AddrTarget::None {
            self.dst_word()
        } else {
            self.mid_word()
        }
    }

    pub(crate) fn mid_or_dst_byte(&self) -> Result<Byte, ExecError> {
        if self.mid == AddrTarget::None {
            self.read_byte_at(self.dst, self.imm_dst)
        } else {
            self.mid_byte()
        }
    }

    pub(crate) fn mid_or_dst_big(&self) -> Result<Big, ExecError> {
        if self.mid == AddrTarget::None {
            self.read_big_at(self.dst, self.imm_dst)
        } else {
            self.mid_big()
        }
    }

    pub(crate) fn mid_or_dst_real(&self) -> Result<Real, ExecError> {
        if self.mid == AddrTarget::None {
            self.read_real_at(self.dst, self.imm_dst)
        } else {
            self.mid_real()
        }
    }

    // --- Pointer (HeapId) accessors ---

    pub(crate) fn src_ptr(&self) -> Result<HeapId, ExecError> {
        Ok(self.read_word_at(self.src, self.imm_src)? as HeapId)
    }
    pub(crate) fn mid_ptr(&self) -> Result<HeapId, ExecError> {
        Ok(self.read_word_at(self.mid, self.imm_mid)? as HeapId)
    }
    pub(crate) fn dst_ptr(&self) -> Result<HeapId, ExecError> {
        Ok(self.read_word_at(self.dst, self.imm_dst)? as HeapId)
    }
    pub(crate) fn set_dst_ptr(&mut self, id: HeapId) -> Result<(), ExecError> {
        self.write_word_at(self.dst, id as Word)
    }

    /// Move a pointer from one location to another with reference counting.
    /// Increments the new value's ref count and decrements the old value's ref count.
    pub(crate) fn move_ptr_to_dst(&mut self, new_id: HeapId) -> Result<(), ExecError> {
        let old_id = self.dst_ptr()?;
        if new_id != heap::NIL {
            self.heap.inc_ref(new_id);
        }
        self.set_dst_ptr(new_id)?;
        if old_id != heap::NIL {
            self.heap.dec_ref(old_id);
        }
        Ok(())
    }

    /// Read a pointer (HeapId) from an arbitrary absolute offset in the frame stack.
    #[allow(dead_code)]
    pub(crate) fn read_ptr_stack(&self, abs_offset: usize) -> HeapId {
        memory::read_word(&self.frames.data, abs_offset) as HeapId
    }

    /// Write a pointer (HeapId) to an arbitrary absolute offset in the frame stack.
    #[allow(dead_code)]
    pub(crate) fn write_ptr_stack(&mut self, abs_offset: usize, id: HeapId) {
        memory::write_word(&mut self.frames.data, abs_offset, id as Word);
    }
}

fn format_operand(label: &str, op: &ricevm_core::Operand) -> String {
    use ricevm_core::AddressMode;
    match op.mode {
        AddressMode::OffsetIndirectFp => format!("{label}={}(fp)", op.register1),
        AddressMode::OffsetIndirectMp => format!("{label}={}(mp)", op.register1),
        AddressMode::Immediate => format!("{label}=${}", op.register1),
        AddressMode::None => String::new(),
        AddressMode::OffsetDoubleIndirectFp => {
            format!("{label}={}({}(fp))", op.register2, op.register1)
        }
        AddressMode::OffsetDoubleIndirectMp => {
            format!("{label}={}({}(mp))", op.register2, op.register1)
        }
        AddressMode::Reserved1 | AddressMode::Reserved2 => format!("{label}=reserved"),
    }
}

fn format_mid(label: &str, op: &ricevm_core::MiddleOperand) -> String {
    use ricevm_core::MiddleMode;
    match op.mode {
        MiddleMode::None => String::new(),
        MiddleMode::SmallImmediate => format!("{label}=${}", op.register1),
        MiddleMode::SmallOffsetFp => format!("{label}={}(fp)", op.register1),
        MiddleMode::SmallOffsetMp => format!("{label}={}(mp)", op.register1),
    }
}

#[cfg(test)]
mod tests {
    use ricevm_core::{
        Header, Instruction, MiddleOperand, Module, Opcode, Operand, PointerMap, RuntimeFlags,
        TypeDescriptor, XMAGIC,
    };

    use super::*;
    use crate::heap::HeapData;

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
            name: "vm_test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        }
    }

    fn alloc_byte_array(vm: &mut VmState<'_>, data: &[u8]) -> heap::HeapId {
        vm.heap.alloc(
            0,
            HeapData::Array {
                elem_type: 0,
                elem_size: 1,
                data: data.to_vec(),
                length: data.len(),
            },
        )
    }

    #[test]
    fn vmstate_new_creates_valid_initial_state() {
        let module = test_module();
        let vm = VmState::new(&module).expect("vm should initialize");
        assert!(!vm.halted, "should not start halted");
        assert_eq!(vm.pc, 0, "pc should start at entry_pc (0)");
        assert!(
            vm.thread_queue.is_empty(),
            "thread queue should start empty"
        );
        assert!(vm.loaded_modules.is_empty(), "no loaded modules initially");
        assert!(vm.blocked_channel.is_none(), "should not start blocked");
    }

    #[test]
    fn read_write_word_at_frame() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let fp = vm.frames.current_data_offset();

        vm.write_word_at(AddrTarget::Frame(fp + 4), 12345)
            .expect("write to frame");
        let val = vm
            .read_word_at(AddrTarget::Frame(fp + 4), 0)
            .expect("read from frame");
        assert_eq!(val, 12345);
    }

    #[test]
    fn read_write_word_at_mp() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        // Ensure mp is large enough
        if vm.mp.len() < 8 {
            vm.mp.resize(8, 0);
        }

        vm.write_word_at(AddrTarget::Mp(0), 42)
            .expect("write to mp");
        let val = vm.read_word_at(AddrTarget::Mp(0), 0).expect("read from mp");
        assert_eq!(val, 42);
    }

    #[test]
    fn read_word_at_immediate() {
        let module = test_module();
        let vm = VmState::new(&module).expect("vm init");
        let val = vm
            .read_word_at(AddrTarget::Immediate, 999)
            .expect("read immediate");
        assert_eq!(val, 999);
    }

    #[test]
    fn write_word_at_immediate_errors() {
        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm init");
        let result = vm.write_word_at(AddrTarget::Immediate, 42);
        assert!(result.is_err(), "writing to immediate should fail");
    }

    #[test]
    fn read_word_at_none_returns_zero() {
        let module = test_module();
        let vm = VmState::new(&module).expect("vm init");
        let val = vm.read_word_at(AddrTarget::None, 0).expect("read none");
        assert_eq!(val, 0);
    }

    #[test]
    fn raise_exception_finds_matching_handler() {
        use ricevm_core::{ExceptionCase, Handler};

        let module = Module {
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
            name: "exc_test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![Handler {
                exception_offset: 0,
                begin_pc: 0,
                end_pc: 10,
                type_descriptor: None,
                cases: vec![ExceptionCase {
                    name: Some("file".to_string()),
                    pc: 42,
                }],
            }],
        };

        let mut vm = VmState::new(&module).expect("vm init");
        vm.pc = 0;

        let result = vm.raise_exception("file not found");
        assert!(result.is_ok(), "matching handler should be found");
        assert_eq!(vm.next_pc, 42, "should jump to handler pc");
    }

    #[test]
    fn raise_exception_wildcard_handler() {
        use ricevm_core::{ExceptionCase, Handler};

        let module = Module {
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
            name: "exc_wild_test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![Handler {
                exception_offset: 0,
                begin_pc: 0,
                end_pc: 10,
                type_descriptor: None,
                cases: vec![ExceptionCase {
                    name: None, // wildcard
                    pc: 99,
                }],
            }],
        };

        let mut vm = VmState::new(&module).expect("vm init");
        vm.pc = 5;

        let result = vm.raise_exception("anything");
        assert!(result.is_ok(), "wildcard should match");
        assert_eq!(vm.next_pc, 99);
    }

    #[test]
    fn raise_exception_no_handler_returns_error() {
        let module = test_module(); // no handlers
        let mut vm = VmState::new(&module).expect("vm init");
        vm.pc = 0;

        let result = vm.raise_exception("some error");
        assert!(result.is_err(), "unhandled exception should return error");
    }

    #[test]
    fn raise_exception_out_of_range_not_matched() {
        use ricevm_core::{ExceptionCase, Handler};

        let module = Module {
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
            name: "exc_range_test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![Handler {
                exception_offset: 0,
                begin_pc: 5,
                end_pc: 10,
                type_descriptor: None,
                cases: vec![ExceptionCase { name: None, pc: 50 }],
            }],
        };

        let mut vm = VmState::new(&module).expect("vm init");
        vm.pc = 0; // outside [5, 10)

        let result = vm.raise_exception("error");
        assert!(
            result.is_err(),
            "handler with non-matching PC range should not match"
        );
    }

    #[test]
    fn keyring_sha224_writes_real_digest() {
        use sha2::Digest;

        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let frame_base = vm.frames.current_data_offset();
        let data_id = alloc_byte_array(&mut vm, b"abc");
        let digest_id = alloc_byte_array(&mut vm, &[0; 28]);

        memory::write_word(&mut vm.frames.data, frame_base + 32, data_id as i32);
        memory::write_word(&mut vm.frames.data, frame_base + 36, 3);
        memory::write_word(&mut vm.frames.data, frame_base + 40, digest_id as i32);
        memory::write_word(&mut vm.frames.data, frame_base + 44, heap::NIL as i32);

        keyring_sha224(&mut vm).expect("sha224 handler should succeed");

        let expected = sha2::Sha224::digest(b"abc").to_vec();
        let actual = vm
            .heap
            .array_read(digest_id, 0, expected.len())
            .expect("digest array should exist");
        assert_eq!(actual, expected);

        let state_id = memory::read_word(&vm.frames.data, frame_base) as heap::HeapId;
        let state = vm.heap.get(state_id).expect("state record should exist");
        let HeapData::Record(state_bytes) = &state.data else {
            panic!("digest state should be stored as a record");
        };
        assert_eq!(memory::read_word(state_bytes, 0), DIGEST_SHA224);
        assert_eq!(&state_bytes[4..], b"abc");
    }

    #[test]
    fn keyring_md4_writes_real_digest() {
        use md4::Digest;

        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let frame_base = vm.frames.current_data_offset();
        let data_id = alloc_byte_array(&mut vm, b"abc");
        let digest_id = alloc_byte_array(&mut vm, &[0; 16]);

        memory::write_word(&mut vm.frames.data, frame_base + 32, data_id as i32);
        memory::write_word(&mut vm.frames.data, frame_base + 36, 3);
        memory::write_word(&mut vm.frames.data, frame_base + 40, digest_id as i32);
        memory::write_word(&mut vm.frames.data, frame_base + 44, heap::NIL as i32);

        keyring_md4(&mut vm).expect("md4 handler should succeed");

        let expected = md4::Md4::digest(b"abc").to_vec();
        let actual = vm
            .heap
            .array_read(digest_id, 0, expected.len())
            .expect("digest array should exist");
        assert_eq!(actual, expected);

        let state_id = memory::read_word(&vm.frames.data, frame_base) as heap::HeapId;
        let state = vm.heap.get(state_id).expect("state record should exist");
        let HeapData::Record(state_bytes) = &state.data else {
            panic!("digest state should be stored as a record");
        };
        assert_eq!(memory::read_word(state_bytes, 0), DIGEST_MD4);
        assert_eq!(&state_bytes[4..], b"abc");
    }

    #[test]
    fn keyring_sha256_supports_incremental_state() {
        use sha2::Digest;

        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let frame_base = vm.frames.current_data_offset();
        let first_chunk_id = alloc_byte_array(&mut vm, b"abc");

        memory::write_word(&mut vm.frames.data, frame_base + 32, first_chunk_id as i32);
        memory::write_word(&mut vm.frames.data, frame_base + 36, 3);
        memory::write_word(&mut vm.frames.data, frame_base + 40, heap::NIL as i32);
        memory::write_word(&mut vm.frames.data, frame_base + 44, heap::NIL as i32);

        keyring_sha256(&mut vm).expect("first sha256 call should succeed");
        let state_id = memory::read_word(&vm.frames.data, frame_base) as heap::HeapId;

        let second_chunk_id = alloc_byte_array(&mut vm, b"def");
        let digest_id = alloc_byte_array(&mut vm, &[0; 32]);
        memory::write_word(&mut vm.frames.data, frame_base + 32, second_chunk_id as i32);
        memory::write_word(&mut vm.frames.data, frame_base + 36, 3);
        memory::write_word(&mut vm.frames.data, frame_base + 40, digest_id as i32);
        memory::write_word(&mut vm.frames.data, frame_base + 44, state_id as i32);

        keyring_sha256(&mut vm).expect("second sha256 call should succeed");

        let expected = sha2::Sha256::digest(b"abcdef").to_vec();
        let actual = vm
            .heap
            .array_read(digest_id, 0, expected.len())
            .expect("digest array should exist");
        assert_eq!(actual, expected);

        let final_state_id = memory::read_word(&vm.frames.data, frame_base) as heap::HeapId;
        let state = vm
            .heap
            .get(final_state_id)
            .expect("final state record should exist");
        let HeapData::Record(state_bytes) = &state.data else {
            panic!("digest state should be stored as a record");
        };
        assert_eq!(memory::read_word(state_bytes, 0), DIGEST_SHA256);
        assert_eq!(&state_bytes[4..], b"abcdef");
    }

    #[test]
    fn keyring_module_registers_real_sha512_handler() {
        use sha2::Digest;

        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let frame_base = vm.frames.current_data_offset();
        let data_id = alloc_byte_array(&mut vm, b"abc");
        let digest_id = alloc_byte_array(&mut vm, &[0; 64]);

        memory::write_word(&mut vm.frames.data, frame_base + 32, data_id as i32);
        memory::write_word(&mut vm.frames.data, frame_base + 36, 3);
        memory::write_word(&mut vm.frames.data, frame_base + 40, digest_id as i32);
        memory::write_word(&mut vm.frames.data, frame_base + 44, heap::NIL as i32);

        let module_id = vm
            .modules
            .find_builtin("$Keyring")
            .expect("$Keyring should be registered");
        let keyring = vm
            .modules
            .get_module(module_id)
            .expect("$Keyring module should exist");
        assert_eq!(
            keyring
                .funcs
                .iter()
                .map(|func| func.name)
                .collect::<Vec<_>>(),
            vec![
                "md5",
                "sha1",
                "sha224",
                "sha256",
                "sha384",
                "sha512",
                "md4",
                "readauthinfo",
                "writeauthinfo",
                "getstring",
                "putstring",
                "getbytearray",
                "putbytearray",
                "auth",
            ]
        );
        let sha512 = keyring
            .funcs
            .iter()
            .find(|func| func.name == "sha512")
            .expect("$Keyring->sha512 should be registered");

        (sha512.handler)(&mut vm).expect("$Keyring->sha512 should succeed");

        let expected = sha2::Sha512::digest(b"abc").to_vec();
        let actual = vm
            .heap
            .array_read(digest_id, 0, expected.len())
            .expect("digest array should exist");
        assert_eq!(actual, expected);
    }

    #[test]
    fn crypt_module_registers_real_digest_handlers() {
        use sha2::Digest;

        let module = test_module();
        let mut vm = VmState::new(&module).expect("vm should initialize");
        let frame_base = vm.frames.current_data_offset();
        let data_id = alloc_byte_array(&mut vm, b"abc");
        let digest_id = alloc_byte_array(&mut vm, &[0; 64]);

        memory::write_word(&mut vm.frames.data, frame_base + 32, data_id as i32);
        memory::write_word(&mut vm.frames.data, frame_base + 36, 3);
        memory::write_word(&mut vm.frames.data, frame_base + 40, digest_id as i32);
        memory::write_word(&mut vm.frames.data, frame_base + 44, heap::NIL as i32);

        let module_id = vm
            .modules
            .find_builtin("$Crypt")
            .expect("$Crypt should be registered");
        let crypt = vm
            .modules
            .get_module(module_id)
            .expect("$Crypt module should exist");
        assert_eq!(
            crypt.funcs.iter().map(|func| func.name).collect::<Vec<_>>(),
            vec!["md5", "sha1", "sha224", "sha256", "sha384", "sha512", "md4"]
        );
        let sha512 = crypt
            .funcs
            .iter()
            .find(|func| func.name == "sha512")
            .expect("$Crypt->sha512 should be registered");

        (sha512.handler)(&mut vm).expect("$Crypt->sha512 should succeed");

        let expected = sha2::Sha512::digest(b"abc").to_vec();
        let actual = vm
            .heap
            .array_read(digest_id, 0, expected.len())
            .expect("digest array should exist");
        assert_eq!(actual, expected);
    }
}
