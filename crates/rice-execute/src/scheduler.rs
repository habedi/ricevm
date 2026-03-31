#![allow(dead_code)]
//! Thread scheduler for the Dis VM.
//!
//! Supports both cooperative (single-threaded) and preemptive (multi-threaded) modes.
//! The cooperative scheduler runs threads in round-robin order with time quanta.
//! The preemptive scheduler uses OS threads from a pool with shared state.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

use ricevm_core::{ExecError, Instruction, Module};

use crate::address::{self, AddrTarget};
use crate::builtin::ModuleRegistry;
use crate::filetab::FileTable;
use crate::frame::FrameStack;
use crate::heap::{Heap, HeapId};
use crate::ops;
use crate::vm::LoadedModule;

/// Default number of instructions per quanta before switching threads.
const DEFAULT_QUANTA: usize = 2048;

/// State of a single VM thread.
pub(crate) struct VmThread {
    pub frames: FrameStack,
    pub mp: Vec<u8>,
    pub pc: usize,
    pub next_pc: usize,
    pub halted: bool,
    pub src: AddrTarget,
    pub mid: AddrTarget,
    pub dst: AddrTarget,
    pub imm_src: i32,
    pub imm_mid: i32,
    pub imm_dst: i32,

    pub heap_refs: Vec<(HeapId, usize)>,
    pub last_error: String,
    pub id: u32,
    pub state: ThreadState,
}

/// Thread scheduling state.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum ThreadState {
    Ready,
    Running,
    BlockedSend(HeapId),
    BlockedRecv(HeapId),
    Exited,
}

/// Shared VM state protected by a mutex (for preemptive scheduling).
pub(crate) struct SharedState<'m> {
    pub module: &'m Module,
    pub heap: Heap,
    pub modules: ModuleRegistry,
    pub loaded_modules: Vec<LoadedModule>,
    pub files: FileTable,
    pub gc_enabled: bool,
    pub gc_counter: usize,
    pub trace: bool,
}

/// The cooperative scheduler manages multiple threads sharing a common heap and module table.
pub(crate) struct Scheduler<'m> {
    pub module: &'m Module,
    pub heap: Heap,
    pub modules: ModuleRegistry,
    pub loaded_modules: Vec<LoadedModule>,
    pub threads: VecDeque<VmThread>,
    pub trace: bool,
    next_thread_id: u32,
}

impl<'m> Scheduler<'m> {
    pub fn new(module: &'m Module, heap: Heap, modules: ModuleRegistry) -> Self {
        Self {
            module,
            heap,
            modules,
            loaded_modules: Vec::new(),
            threads: VecDeque::new(),
            trace: std::env::var("RICEVM_TRACE").is_ok(),
            next_thread_id: 1,
        }
    }

    /// Add the initial thread.
    pub fn add_thread(&mut self, thread: VmThread) {
        self.threads.push_back(thread);
    }

    /// Spawn a new thread starting at the given PC with the given frame stack.
    pub fn spawn_thread(&mut self, frames: FrameStack, mp: Vec<u8>, pc: usize) -> u32 {
        let id = self.next_thread_id;
        self.next_thread_id += 1;
        self.threads.push_back(VmThread {
            frames,
            mp,
            pc,
            next_pc: 0,
            halted: false,
            src: AddrTarget::None,
            mid: AddrTarget::None,
            dst: AddrTarget::None,
            imm_src: 0,
            imm_mid: 0,
            imm_dst: 0,

            heap_refs: Vec::new(),
            last_error: String::new(),
            id,
            state: ThreadState::Ready,
        });
        id
    }

    /// Run all threads until all have halted or an error occurs.
    pub fn run(&mut self) -> Result<(), ExecError> {
        while !self.threads.is_empty() {
            // Remove halted threads
            self.threads.retain(|t| !t.halted);
            if self.threads.is_empty() {
                break;
            }

            // Run the front thread for one quanta
            let quanta = DEFAULT_QUANTA;
            self.run_thread_quanta(quanta)?;

            // Rotate: move front thread to back (round-robin)
            if self.threads.len() > 1
                && let Some(front) = self.threads.pop_front()
                && !front.halted
            {
                self.threads.push_back(front);
            }
        }
        Ok(())
    }

    fn run_thread_quanta(&mut self, quanta: usize) -> Result<(), ExecError> {
        for _ in 0..quanta {
            // Check if front thread exists and is still running
            let (pc, code_len) = match self.threads.front() {
                Some(t) if !t.halted => (t.pc, self.module.code.len()),
                _ => break,
            };
            if pc >= code_len {
                break;
            }

            let inst = self.module.code[pc].clone();
            if self.trace {
                trace_inst(pc, &inst);
            }

            // Resolve operands on the front thread
            {
                let Some(thread) = self.threads.front_mut() else {
                    return Err(ExecError::Other(
                        "scheduler thread queue unexpectedly empty".to_string(),
                    ));
                };
                let fp_base = thread.frames.current_data_offset();
                thread.imm_src = inst.source.register1;
                thread.src = address::resolve_operand(
                    &inst.source,
                    fp_base,
                    &thread.frames.data,
                    &thread.mp,
                    &thread.heap_refs,
                )?;
                thread.imm_mid = inst.middle.register1;
                thread.mid = address::resolve_middle(&inst.middle, fp_base)?;
                thread.imm_dst = inst.destination.register1;
                thread.dst = address::resolve_operand(
                    &inst.destination,
                    fp_base,
                    &thread.frames.data,
                    &thread.mp,
                    &thread.heap_refs,
                )?;
                thread.next_pc = thread.pc + 1;
            }

            // Dispatch (borrows self mutably)
            dispatch_for_thread(self, &inst)?;

            // Update PC
            if let Some(thread) = self.threads.front_mut() {
                thread.pc = thread.next_pc;
            }
        }
        Ok(())
    }
}

/// Preemptive scheduler using OS threads.
pub(crate) struct PreemptiveScheduler<'m> {
    shared: Arc<Mutex<SharedState<'m>>>,
    threads: Arc<Mutex<VecDeque<VmThread>>>,
    condvar: Arc<Condvar>,
    next_thread_id: u32,
    pool_size: usize,
}

impl<'m> PreemptiveScheduler<'m> {
    pub fn new(
        module: &'m Module,
        heap: Heap,
        modules: ModuleRegistry,
        files: FileTable,
        pool_size: usize,
    ) -> Self {
        let shared = SharedState {
            module,
            heap,
            modules,
            loaded_modules: Vec::new(),
            files,
            gc_enabled: std::env::var("RICEVM_NO_GC").is_err(),
            gc_counter: 0,
            trace: std::env::var("RICEVM_TRACE").is_ok(),
        };
        Self {
            shared: Arc::new(Mutex::new(shared)),
            threads: Arc::new(Mutex::new(VecDeque::new())),
            condvar: Arc::new(Condvar::new()),
            next_thread_id: 1,
            pool_size,
        }
    }

    pub fn add_thread(&mut self, thread: VmThread) {
        self.threads.lock().unwrap().push_back(thread);
    }

    pub fn spawn_thread(&mut self, frames: FrameStack, mp: Vec<u8>, pc: usize) -> u32 {
        let id = self.next_thread_id;
        self.next_thread_id += 1;
        self.threads.lock().unwrap().push_back(VmThread {
            frames,
            mp,
            pc,
            next_pc: 0,
            halted: false,
            src: AddrTarget::None,
            mid: AddrTarget::None,
            dst: AddrTarget::None,
            imm_src: 0,
            imm_mid: 0,
            imm_dst: 0,

            heap_refs: Vec::new(),
            last_error: String::new(),
            id,
            state: ThreadState::Ready,
        });
        self.condvar.notify_one();
        id
    }

    /// Run all threads using a thread pool until all have exited.
    pub fn run(&self) -> Result<(), ExecError> {
        // Use scoped threads so we can borrow self (which contains 'm lifetime)
        std::thread::scope(|scope| {
            let workers: Vec<_> = (0..self.pool_size)
                .map(|_| {
                    let shared = Arc::clone(&self.shared);
                    let threads = Arc::clone(&self.threads);
                    let condvar = Arc::clone(&self.condvar);
                    scope.spawn(move || worker_loop(shared, threads, condvar))
                })
                .collect();

            // Wait for all workers to finish
            let mut result = Ok(());
            for w in workers {
                if let Err(e) = w
                    .join()
                    .unwrap_or(Err(ExecError::Other("worker thread panicked".to_string())))
                {
                    result = Err(e);
                }
            }
            result
        })
    }
}

fn worker_loop(
    shared: Arc<Mutex<SharedState<'_>>>,
    threads: Arc<Mutex<VecDeque<VmThread>>>,
    condvar: Arc<Condvar>,
) -> Result<(), ExecError> {
    loop {
        // Pop a ready thread
        let mut thread = {
            let mut queue = threads.lock().unwrap();
            loop {
                // Remove halted threads
                queue.retain(|t| t.state != ThreadState::Exited);
                // Find a ready thread
                if let Some(idx) = queue.iter().position(|t| t.state == ThreadState::Ready) {
                    let mut t = queue.remove(idx).unwrap();
                    t.state = ThreadState::Running;
                    break t;
                }
                // No threads left at all? Exit.
                if queue.is_empty() {
                    return Ok(());
                }
                // All threads are blocked; wait for a signal.
                queue = condvar.wait(queue).unwrap();
            }
        };

        // Execute the thread for one quanta
        let result = {
            let mut state = shared.lock().unwrap();
            run_thread_quanta_shared(&mut state, &mut thread, DEFAULT_QUANTA)
        };

        // Return thread to queue
        {
            let mut queue = threads.lock().unwrap();
            if thread.halted {
                thread.state = ThreadState::Exited;
            } else if thread.state == ThreadState::Running {
                thread.state = ThreadState::Ready;
            }
            queue.push_back(thread);
        }
        condvar.notify_all();

        result?;
    }
}

fn run_thread_quanta_shared(
    state: &mut SharedState<'_>,
    thread: &mut VmThread,
    quanta: usize,
) -> Result<(), ExecError> {
    for _ in 0..quanta {
        if thread.halted || thread.pc >= state.module.code.len() {
            thread.halted = true;
            break;
        }

        let inst = state.module.code[thread.pc].clone();
        if state.trace {
            trace_inst(thread.pc, &inst);
        }

        // Resolve operands
        let fp_base = thread.frames.current_data_offset();
        thread.imm_src = inst.source.register1;
        thread.src = address::resolve_operand(
            &inst.source,
            fp_base,
            &thread.frames.data,
            &thread.mp,
            &thread.heap_refs,
        )?;
        thread.imm_mid = inst.middle.register1;
        thread.mid = address::resolve_middle(&inst.middle, fp_base)?;
        thread.imm_dst = inst.destination.register1;
        thread.dst = address::resolve_operand(
            &inst.destination,
            fp_base,
            &thread.frames.data,
            &thread.mp,
            &thread.heap_refs,
        )?;
        thread.next_pc = thread.pc + 1;

        // Build temp VmState and dispatch
        let mut vm = crate::vm::VmState {
            module: state.module,
            mp: std::mem::take(&mut thread.mp),
            frames: std::mem::replace(&mut thread.frames, FrameStack::new()),
            heap: std::mem::replace(&mut state.heap, Heap::new()),
            modules: std::mem::replace(&mut state.modules, ModuleRegistry::new()),
            loaded_modules: std::mem::take(&mut state.loaded_modules),
            files: std::mem::replace(&mut state.files, FileTable::new()),
            pc: thread.pc,
            next_pc: thread.next_pc,
            halted: thread.halted,
            trace: state.trace,
            gc_enabled: state.gc_enabled,
            gc_counter: state.gc_counter,
            current_loaded_module: None,
            root_path: String::new(),
            src: thread.src,
            mid: thread.mid,
            dst: thread.dst,
            imm_src: thread.imm_src,
            imm_mid: thread.imm_mid,
            imm_dst: thread.imm_dst,
            last_error: std::mem::take(&mut thread.last_error),
            caller_mp_stack: Vec::new(),
            heap_refs: std::mem::take(&mut thread.heap_refs),
        };

        let result = ops::dispatch(&mut vm, &inst);

        // Move state back
        thread.mp = vm.mp;
        thread.frames = vm.frames;
        thread.pc = vm.pc;
        thread.next_pc = vm.next_pc;
        thread.halted = vm.halted;
        thread.src = vm.src;
        thread.mid = vm.mid;
        thread.dst = vm.dst;
        thread.imm_src = vm.imm_src;
        thread.imm_mid = vm.imm_mid;
        thread.imm_dst = vm.imm_dst;
        thread.heap_refs = vm.heap_refs;
        thread.last_error = vm.last_error;
        state.heap = vm.heap;
        state.modules = vm.modules;
        state.loaded_modules = vm.loaded_modules;
        state.files = vm.files;
        state.gc_counter = vm.gc_counter;

        result?;

        thread.pc = thread.next_pc;
    }
    Ok(())
}

/// Dispatch an instruction for the current front thread (cooperative mode).
fn dispatch_for_thread(sched: &mut Scheduler<'_>, inst: &Instruction) -> Result<(), ExecError> {
    let Some(thread) = sched.threads.front_mut() else {
        return Err(ExecError::Other(
            "scheduler thread queue unexpectedly empty".to_string(),
        ));
    };
    let mut vm = crate::vm::VmState {
        module: sched.module,
        mp: std::mem::take(&mut thread.mp),
        frames: std::mem::replace(&mut thread.frames, FrameStack::new()),
        heap: std::mem::replace(&mut sched.heap, Heap::new()),
        modules: std::mem::replace(&mut sched.modules, ModuleRegistry::new()),
        loaded_modules: std::mem::take(&mut sched.loaded_modules),
        files: crate::filetab::FileTable::new(),
        pc: thread.pc,
        next_pc: thread.next_pc,
        halted: thread.halted,
        trace: sched.trace,
        gc_enabled: false,
        gc_counter: 0,
        current_loaded_module: None,
        root_path: String::new(),
        src: thread.src,
        mid: thread.mid,
        dst: thread.dst,
        imm_src: thread.imm_src,
        imm_mid: thread.imm_mid,
        imm_dst: thread.imm_dst,
        last_error: std::mem::take(&mut thread.last_error),
        caller_mp_stack: Vec::new(),
        heap_refs: std::mem::take(&mut thread.heap_refs),
    };

    let result = ops::dispatch(&mut vm, inst);

    // Move state back
    let Some(thread) = sched.threads.front_mut() else {
        return Err(ExecError::Other(
            "scheduler thread queue unexpectedly empty".to_string(),
        ));
    };
    thread.mp = vm.mp;
    thread.frames = vm.frames;
    thread.pc = vm.pc;
    thread.next_pc = vm.next_pc;
    thread.halted = vm.halted;
    thread.src = vm.src;
    thread.mid = vm.mid;
    thread.dst = vm.dst;
    thread.imm_src = vm.imm_src;
    thread.imm_mid = vm.imm_mid;
    thread.imm_dst = vm.imm_dst;
    thread.heap_refs = vm.heap_refs;
    thread.last_error = vm.last_error;
    sched.heap = vm.heap;
    sched.modules = vm.modules;
    sched.loaded_modules = vm.loaded_modules;

    result
}

fn trace_inst(pc: usize, inst: &Instruction) {
    use ricevm_core::{AddressMode, MiddleMode};
    let mut parts = vec![format!("{pc:4}: {:?}", inst.opcode)];
    if inst.source.mode != AddressMode::None {
        parts.push(format!("src={}", fmt_op_short(&inst.source)));
    }
    if inst.middle.mode != MiddleMode::None {
        parts.push(format!("mid={}", fmt_mid_short(&inst.middle)));
    }
    if inst.destination.mode != AddressMode::None {
        parts.push(format!("dst={}", fmt_op_short(&inst.destination)));
    }
    eprintln!("{}", parts.join(" "));
}

fn fmt_op_short(op: &ricevm_core::Operand) -> String {
    use ricevm_core::AddressMode;
    match op.mode {
        AddressMode::OffsetIndirectFp => format!("{}(fp)", op.register1),
        AddressMode::OffsetIndirectMp => format!("{}(mp)", op.register1),
        AddressMode::Immediate => format!("${}", op.register1),
        _ => "?".to_string(),
    }
}

fn fmt_mid_short(op: &ricevm_core::MiddleOperand) -> String {
    use ricevm_core::MiddleMode;
    match op.mode {
        MiddleMode::SmallImmediate => format!("${}", op.register1),
        MiddleMode::SmallOffsetFp => format!("{}(fp)", op.register1),
        MiddleMode::SmallOffsetMp => format!("{}(mp)", op.register1),
        _ => "?".to_string(),
    }
}
