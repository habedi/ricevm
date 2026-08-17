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
use crate::vm::{LoadedModule, deadlock_fault};

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
    /// Index of the loaded module this thread executes (None = main module).
    pub current_loaded_module: Option<usize>,
    /// Caller MP buffers for cross-module address resolution.
    pub caller_mp_stack: Vec<(usize, Vec<u8>)>,
    pub id: u32,
    pub state: ThreadState,
    /// Preemptive-pool bookkeeping: the instruction count the VM had retired
    /// when this thread last parked on a channel. Retrying is pointless until
    /// some other thread retires an instruction. Unused in cooperative mode.
    blocked_at: u64,
}

impl VmThread {
    /// Adopt a thread that a running thread spawned.
    ///
    /// Both schedulers build children here so neither can forget part of the
    /// context `spawn`/`mspawn` handed over. Dropping `current_loaded_module`
    /// makes a child spawned inside a loaded module run the MAIN module's code
    /// at the LOADED module's entry pc, against the wrong MP.
    fn from_suspended(id: u32, child: crate::vm::SuspendedThread) -> Self {
        Self {
            frames: child.frames,
            mp: child.mp,
            pc: child.pc,
            next_pc: 0,
            halted: false,
            src: AddrTarget::None,
            mid: AddrTarget::None,
            dst: AddrTarget::None,
            imm_src: 0,
            imm_mid: 0,
            imm_dst: 0,

            heap_refs: child.heap_refs,
            last_error: child.last_error,
            current_loaded_module: child.current_loaded_module,
            caller_mp_stack: child.caller_mp_stack,
            id,
            state: match child.blocked_on {
                Some(chan_id) => ThreadState::Blocked(chan_id),
                None => ThreadState::Ready,
            },
            blocked_at: 0,
        }
    }

    /// A fresh thread running `pc` in the main module.
    fn new(id: u32, frames: FrameStack, mp: Vec<u8>, pc: usize) -> Self {
        Self::from_suspended(
            id,
            crate::vm::SuspendedThread {
                frames,
                mp,
                pc,
                pid: id as i32,
                heap_refs: Vec::new(),
                last_error: String::new(),
                current_loaded_module: None,
                caller_mp_stack: Vec::new(),
                blocked_on: None,
            },
        )
    }
}

/// Thread scheduling state.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum ThreadState {
    Ready,
    Running,
    /// Waiting for a channel operation on the given channel to become
    /// possible. `NIL` marks a thread blocked in `alt`, which any channel
    /// operation wakes.
    Blocked(HeapId),
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
    /// Source of thread ids, shared so worker-spawned threads get unique ones.
    pub next_thread_id: u32,
    /// Instructions retired by all threads. Bumped under this lock, so it
    /// orders "the channel may have changed since I parked" for the pool.
    pub progress: u64,
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
        let id = self.next_id();
        self.threads.push_back(VmThread::new(id, frames, mp, pc));
        id
    }

    /// Queue a thread the running thread spawned, keeping its full context.
    fn adopt_child(&mut self, child: crate::vm::SuspendedThread) -> u32 {
        let id = self.next_id();
        self.threads.push_back(VmThread::from_suspended(id, child));
        id
    }

    fn next_id(&mut self) -> u32 {
        let id = self.next_thread_id;
        self.next_thread_id += 1;
        id
    }

    /// The code of the module a thread executes.
    fn code_of<'a>(&'a self, thread: &VmThread) -> &'a [Instruction] {
        match thread.current_loaded_module {
            Some(idx) => match self.loaded_modules.get(idx) {
                Some(lm) => &lm.module.code,
                None => &self.module.code,
            },
            None => &self.module.code,
        }
    }

    /// Run all threads until all have halted or an error occurs.
    pub fn run(&mut self) -> Result<(), ExecError> {
        while !self.threads.is_empty() {
            // Remove halted threads
            self.threads.retain(|t| !t.halted);
            if self.threads.is_empty() {
                break;
            }

            // Bring a runnable thread to the front. Only another thread's
            // channel operation can wake a blocked one, so when every thread
            // is blocked the program cannot make progress.
            if !self.rotate_to_ready() {
                return Err(deadlock_fault());
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

    /// Rotate the queue so that a runnable thread is at the front.
    /// Returns false when every thread is blocked on a channel.
    fn rotate_to_ready(&mut self) -> bool {
        match self
            .threads
            .iter()
            .position(|t| !matches!(t.state, ThreadState::Blocked(_)))
        {
            Some(idx) => {
                self.threads.rotate_left(idx);
                true
            }
            None => false,
        }
    }

    fn run_thread_quanta(&mut self, quanta: usize) -> Result<(), ExecError> {
        for _ in 0..quanta {
            // Check if front thread exists and is still running. A thread that
            // entered a loaded module executes that module's code, not the
            // main module's.
            let (pc, inst) = match self.threads.front() {
                Some(t) if !t.halted => (t.pc, self.code_of(t).get(t.pc).cloned()),
                _ => break,
            };
            let Some(inst) = inst else {
                // Running off the end of the code ends the thread. Without
                // halting it here `run()` would retain it, this loop would
                // break immediately, and the outer loop would spin forever.
                if let Some(thread) = self.threads.front_mut() {
                    thread.halted = true;
                }
                break;
            };
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

            // Update PC. A thread that blocked on a channel keeps its PC so it
            // re-executes the operation once woken, and yields the quanta.
            match self.threads.front_mut() {
                Some(thread) if matches!(thread.state, ThreadState::Blocked(_)) => break,
                Some(thread) => thread.pc = thread.next_pc,
                None => break,
            }
        }
        Ok(())
    }
}

/// The preemptive pool's run queue, plus the bookkeeping needed to tell a real
/// deadlock ("every thread is blocked and no worker is running, so nothing can
/// ever complete the channel operation they wait for") apart from a temporary
/// wait ("a worker is still running and may unblock one").
struct ThreadPool {
    queue: VecDeque<VmThread>,
    /// Threads handed to a worker and not yet returned.
    running: usize,
    /// Highest instruction count any worker has observed.
    progress: u64,
    /// Set once a worker reported a deadlock, so the others stop waiting.
    shutdown: bool,
}

impl ThreadPool {
    fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            running: 0,
            progress: 0,
            shutdown: false,
        }
    }

    /// Whether a worker should pick this thread up.
    ///
    /// A parked thread becomes runnable again as soon as the VM has retired an
    /// instruction since it parked: whatever it waits for may have happened, and
    /// re-executing the operation simply parks it again when it has not. The
    /// comparison is against the instruction count at park time rather than a
    /// wake signal, because a signal sent while the thread is still in a
    /// worker's hands would be lost -- and losing it reports a deadlock for a
    /// program that only needed to retry a send.
    fn is_runnable(&self, thread: &VmThread) -> bool {
        match thread.state {
            ThreadState::Ready => true,
            ThreadState::Blocked(_) => thread.blocked_at < self.progress,
            ThreadState::Running | ThreadState::Exited => false,
        }
    }
}

/// Preemptive scheduler using OS threads.
pub(crate) struct PreemptiveScheduler<'m> {
    shared: Arc<Mutex<SharedState<'m>>>,
    threads: Arc<Mutex<ThreadPool>>,
    condvar: Arc<Condvar>,
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
            next_thread_id: 1,
            progress: 0,
        };
        Self {
            shared: Arc::new(Mutex::new(shared)),
            threads: Arc::new(Mutex::new(ThreadPool::new())),
            condvar: Arc::new(Condvar::new()),
            pool_size,
        }
    }

    pub fn add_thread(&mut self, thread: VmThread) {
        self.threads
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .queue
            .push_back(thread);
    }

    pub fn spawn_thread(&mut self, frames: FrameStack, mp: Vec<u8>, pc: usize) -> u32 {
        // Take the id from the shared state so ids stay unique across the
        // threads workers spawn while running.
        let id = {
            let mut shared = self.shared.lock().unwrap_or_else(|e| e.into_inner());
            let id = shared.next_thread_id;
            shared.next_thread_id += 1;
            id
        };
        self.threads
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .queue
            .push_back(VmThread::new(id, frames, mp, pc));
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
    threads: Arc<Mutex<ThreadPool>>,
    condvar: Arc<Condvar>,
) -> Result<(), ExecError> {
    loop {
        // Pop a ready thread
        let mut thread = {
            let mut pool = threads.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if pool.shutdown {
                    return Ok(());
                }
                // Remove halted threads
                pool.queue.retain(|t| t.state != ThreadState::Exited);
                // Find a thread to run: ready, or parked with something having
                // happened since it parked.
                if let Some(idx) = pool.queue.iter().position(|t| pool.is_runnable(t)) {
                    let Some(mut t) = pool.queue.remove(idx) else {
                        return Ok(());
                    };
                    t.state = ThreadState::Running;
                    pool.running += 1;
                    break t;
                }
                if pool.running == 0 {
                    // No threads left at all? Exit.
                    if pool.queue.is_empty() {
                        return Ok(());
                    }
                    // Only parked threads are left, none of them has seen any
                    // progress since parking, and no worker is running: nothing
                    // will ever complete the operation they wait for. Stop the
                    // other workers before reporting it.
                    pool.shutdown = true;
                    drop(pool);
                    condvar.notify_all();
                    return Err(deadlock_fault());
                }
                // Something is still running and may unblock a thread or spawn
                // one; wait to be signalled.
                pool = condvar.wait(pool).unwrap_or_else(|e| e.into_inner());
            }
        };

        // Execute the thread for one quanta. Read the instruction count under
        // the same lock: nothing else can run while it is held, so this is the
        // count the thread saw if it parked.
        let mut spawned = Vec::new();
        let (result, epoch) = {
            let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
            let result =
                run_thread_quanta_shared(&mut state, &mut thread, DEFAULT_QUANTA, &mut spawned);
            (result, state.progress)
        };

        // Return thread to queue, along with anything it spawned
        {
            let mut pool = threads.lock().unwrap_or_else(|e| e.into_inner());
            pool.running -= 1;
            pool.progress = pool.progress.max(epoch);
            if thread.halted {
                thread.state = ThreadState::Exited;
            } else if thread.state == ThreadState::Running {
                thread.state = ThreadState::Ready;
            } else if matches!(thread.state, ThreadState::Blocked(_)) {
                thread.blocked_at = epoch;
            }
            pool.queue.push_back(thread);
            pool.queue.extend(spawned);
        }
        condvar.notify_all();

        result?;
    }
}

/// Run one quanta of `thread`. Returns the number of instructions it retired,
/// which the worker uses to decide whether blocked threads are worth waking.
fn run_thread_quanta_shared(
    state: &mut SharedState<'_>,
    thread: &mut VmThread,
    quanta: usize,
    spawned: &mut Vec<VmThread>,
) -> Result<usize, ExecError> {
    let mut executed = 0usize;
    for _ in 0..quanta {
        if thread.halted {
            break;
        }
        // A thread that entered a loaded module executes that module's code.
        let inst = match thread.current_loaded_module {
            Some(idx) => state
                .loaded_modules
                .get(idx)
                .and_then(|lm| lm.module.code.get(thread.pc))
                .cloned(),
            None => state.module.code.get(thread.pc).cloned(),
        };
        let Some(inst) = inst else {
            thread.halted = true;
            break;
        };
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
            current_loaded_module: thread.current_loaded_module,
            root_path: String::new(),
            src: thread.src,
            mid: thread.mid,
            dst: thread.dst,
            imm_src: thread.imm_src,
            imm_mid: thread.imm_mid,
            imm_dst: thread.imm_dst,
            last_error: std::mem::take(&mut thread.last_error),
            caller_mp_stack: std::mem::take(&mut thread.caller_mp_stack),
            blocked_channel: None,
            unwind_floor: 0,
            thread_queue: std::collections::VecDeque::new(),
            current_pid: 1,
            next_pid: 2,
            wait_records: std::collections::VecDeque::new(),
            heap_refs: std::mem::take(&mut thread.heap_refs),
        };

        let result = ops::dispatch(&mut vm, &inst);

        // Threads created by this instruction live in the temporary VmState's
        // queue; hand them back instead of dropping them with it.
        let children = std::mem::take(&mut vm.thread_queue);
        let blocked_channel = vm.blocked_channel.take();

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
        thread.current_loaded_module = vm.current_loaded_module;
        thread.caller_mp_stack = vm.caller_mp_stack;
        state.heap = vm.heap;
        state.modules = vm.modules;
        state.loaded_modules = vm.loaded_modules;
        state.files = vm.files;
        state.gc_counter = vm.gc_counter;

        for child in children {
            let id = state.next_thread_id;
            state.next_thread_id += 1;
            spawned.push(VmThread::from_suspended(id, child));
        }

        result?;

        if let Some(chan_id) = blocked_channel {
            // A recv with no data, or a send into a full single-slot channel
            // (ordinary back-pressure), cannot complete yet. Park the thread
            // without advancing its pc so it re-executes the operation, and
            // let the worker pick up another thread; the worker wakes it once
            // any thread makes progress. Advancing instead would leave a
            // receive destination unwritten, and failing would abort every
            // program that uses a channel for flow control.
            thread.state = ThreadState::Blocked(chan_id);
            return Ok(executed);
        }

        thread.pc = thread.next_pc;
        executed += 1;
        state.progress += 1;
    }
    Ok(executed)
}

/// Dispatch an instruction for the current front thread (cooperative mode).
fn dispatch_for_thread(sched: &mut Scheduler<'_>, inst: &Instruction) -> Result<(), ExecError> {
    // Mirror the other threads' blocked state into the temporary VmState so a
    // channel operation performed by this instruction wakes them, exactly as
    // it does in the main run loop. Entry `i` mirrors `sched.threads[i + 1]`.
    let waiters: Vec<Option<HeapId>> = sched
        .threads
        .iter()
        .skip(1)
        .map(|t| match t.state {
            ThreadState::Blocked(chan_id) => Some(chan_id),
            _ => None,
        })
        .collect();
    let thread_queue: VecDeque<crate::vm::SuspendedThread> = waiters
        .iter()
        .map(|blocked_on| crate::vm::SuspendedThread {
            frames: FrameStack::new(),
            mp: Vec::new(),
            pc: 0,
            pid: 0,
            heap_refs: Vec::new(),
            last_error: String::new(),
            current_loaded_module: None,
            caller_mp_stack: Vec::new(),
            blocked_on: *blocked_on,
        })
        .collect();

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
        current_loaded_module: thread.current_loaded_module,
        root_path: String::new(),
        src: thread.src,
        mid: thread.mid,
        dst: thread.dst,
        imm_src: thread.imm_src,
        imm_mid: thread.imm_mid,
        imm_dst: thread.imm_dst,
        last_error: std::mem::take(&mut thread.last_error),
        caller_mp_stack: std::mem::take(&mut thread.caller_mp_stack),
        blocked_channel: None,
        unwind_floor: 0,
        thread_queue,
        current_pid: thread.id as i32,
        next_pid: 0,
        wait_records: std::collections::VecDeque::new(),
        heap_refs: std::mem::take(&mut thread.heap_refs),
    };

    let result = ops::dispatch(&mut vm, inst);

    // Entries beyond the mirrors were spawned by this instruction; the mirrors
    // record which blocked threads it woke.
    let mut mirrors = std::mem::take(&mut vm.thread_queue);
    let spawned = mirrors.split_off(waiters.len());
    let blocked_channel = vm.blocked_channel.take();

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
    thread.current_loaded_module = vm.current_loaded_module;
    thread.caller_mp_stack = vm.caller_mp_stack;
    if let Some(chan_id) = blocked_channel {
        // recv/alt found no data: the run loop keeps this thread's PC and
        // leaves it blocked until another thread makes the operation possible.
        thread.state = ThreadState::Blocked(chan_id);
    }
    sched.heap = vm.heap;
    sched.modules = vm.modules;
    sched.loaded_modules = vm.loaded_modules;

    // Wake the threads whose mirror was unblocked by this instruction.
    for (idx, mirror) in mirrors.iter().enumerate() {
        if mirror.blocked_on.is_none()
            && waiters[idx].is_some()
            && let Some(woken) = sched.threads.get_mut(idx + 1)
        {
            woken.state = ThreadState::Ready;
        }
    }

    // Keep the threads this instruction spawned; they used to be dropped with
    // the temporary VmState. `adopt_child` carries the whole context `spawn`
    // gave them (module, caller MPs, heap refs), not just frames/mp/pc.
    for child in spawned {
        sched.adopt_child(child);
    }

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

#[cfg(test)]
mod tests {
    use ricevm_core::{
        AddressMode, Header, Instruction, MiddleOperand, Module, Opcode, Operand, PointerMap,
        RuntimeFlags, TypeDescriptor, XMAGIC,
    };

    use super::*;
    use crate::heap::HeapData;
    use crate::memory;

    fn fp_operand(offset: i32) -> Operand {
        Operand {
            mode: AddressMode::OffsetIndirectFp,
            register1: offset,
            register2: 0,
        }
    }

    fn module_with_code(name: &str, code: Vec<Instruction>) -> Module {
        Module {
            header: Header {
                magic: XMAGIC,
                signature: vec![],
                runtime_flags: RuntimeFlags(0),
                stack_extent: 0,
                code_size: code.len() as i32,
                data_size: 0,
                type_size: 1,
                export_size: 0,
                entry_pc: 0,
                entry_type: 0,
            },
            code,
            types: vec![TypeDescriptor {
                id: 0,
                size: 64,
                pointer_map: PointerMap { bytes: vec![] },
                pointer_count: 0,
            }],
            data: vec![],
            name: name.to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        }
    }

    fn exit_instruction() -> Instruction {
        Instruction {
            opcode: Opcode::Exit,
            source: Operand::UNUSED,
            middle: MiddleOperand::UNUSED,
            destination: Operand::UNUSED,
        }
    }

    /// A module whose code receives from the channel in `fp[0]` into `fp[8]`.
    fn recv_module(name: &str) -> Module {
        module_with_code(
            name,
            vec![
                Instruction {
                    opcode: Opcode::Recv,
                    source: fp_operand(0),
                    middle: MiddleOperand::UNUSED,
                    destination: fp_operand(8),
                },
                exit_instruction(),
            ],
        )
    }

    fn entry_frames() -> FrameStack {
        let mut frames = FrameStack::new();
        frames.push_entry(64, -1);
        frames
    }

    fn scheduler_with_thread<'m>(module: &'m Module, pc: usize) -> Scheduler<'m> {
        let mut sched = Scheduler::new(module, Heap::new(), ModuleRegistry::new());
        sched.spawn_thread(entry_frames(), Vec::new(), pc);
        sched
    }

    /// Regression: a thread whose PC ran past the end of the code must be
    /// halted. Leaving it alive makes `run()` retain it, the quanta loop break
    /// immediately, and the outer loop spin forever.
    #[test]
    fn quanta_halts_a_thread_whose_pc_ran_off_the_end() {
        let module = module_with_code("sched_halt", vec![exit_instruction()]);
        let mut sched = scheduler_with_thread(&module, module.code.len());

        sched.run_thread_quanta(4).expect("quanta should not error");

        assert!(
            sched.threads.front().expect("thread").halted,
            "a thread whose pc ran off the end must be halted so run() can drop it"
        );
    }

    /// The symptom of the above: `run()` must terminate rather than spin.
    #[test]
    fn run_terminates_when_a_thread_runs_off_the_end_of_the_code() {
        let module = module_with_code("sched_run", vec![exit_instruction()]);
        let mut sched = scheduler_with_thread(&module, module.code.len());

        sched.run().expect("run should terminate");
        assert!(
            sched.threads.is_empty(),
            "the halted thread must be dropped"
        );
    }

    /// Regression: a thread spawned while dispatching must join the scheduler's
    /// queue. It used to be pushed onto a throwaway VmState and dropped.
    #[test]
    fn dispatch_keeps_threads_spawned_by_the_running_thread() {
        let module = module_with_code("sched_spawn", vec![exit_instruction(), exit_instruction()]);
        let mut sched = scheduler_with_thread(&module, 0);

        let pending = {
            let thread = sched.threads.front_mut().expect("thread");
            thread.frames.alloc_pending(64).expect("alloc_pending")
        };
        {
            let thread = sched.threads.front_mut().expect("thread");
            thread.src = AddrTarget::Immediate;
            thread.imm_src = pending as i32;
            thread.dst = AddrTarget::Immediate;
            thread.imm_dst = 1; // spawn target pc
            thread.next_pc = 1;
        }

        let spawn = Instruction {
            opcode: Opcode::Spawn,
            source: Operand::UNUSED,
            middle: MiddleOperand::UNUSED,
            destination: Operand::UNUSED,
        };
        dispatch_for_thread(&mut sched, &spawn).expect("spawn should dispatch");

        assert_eq!(
            sched.threads.len(),
            2,
            "the spawned child must join the scheduler's queue"
        );
        assert!(
            sched.threads.iter().any(|t| t.pc == 1),
            "the child must start at the spawn target pc"
        );
    }

    /// Regression: a thread spawned while a loaded module is current must keep
    /// running that module. The cooperative scheduler rebuilt children through
    /// `spawn_thread`, which knows only frames/mp/pc, so the child ran the MAIN
    /// module's code at the LOADED module's entry pc, against the wrong MP.
    #[test]
    fn dispatch_preserves_the_spawning_threads_module_context() {
        let module = module_with_code("sched_ctx", vec![exit_instruction(), exit_instruction()]);
        let loaded = module_with_code(
            "sched_ctx_lib",
            vec![exit_instruction(), exit_instruction()],
        );
        let mut sched = scheduler_with_thread(&module, 0);
        sched.loaded_modules.push(LoadedModule {
            module: loaded,
            mp: Vec::new(),
        });

        let caller_mp_stack = vec![(0usize, vec![1u8, 0, 0, 0])];
        let module_mp = vec![2u8, 0, 0, 0];
        let pending = {
            let thread = sched.threads.front_mut().expect("thread");
            thread.frames.alloc_pending(64).expect("alloc_pending")
        };
        {
            let thread = sched.threads.front_mut().expect("thread");
            thread.current_loaded_module = Some(0);
            thread.caller_mp_stack = caller_mp_stack.clone();
            thread.mp = module_mp.clone();
            thread.src = AddrTarget::Immediate;
            thread.imm_src = pending as i32;
            thread.dst = AddrTarget::Immediate;
            thread.imm_dst = 1; // spawn target pc
            thread.next_pc = 1;
        }

        let spawn = Instruction {
            opcode: Opcode::Spawn,
            source: Operand::UNUSED,
            middle: MiddleOperand::UNUSED,
            destination: Operand::UNUSED,
        };
        dispatch_for_thread(&mut sched, &spawn).expect("spawn should dispatch");

        let child = sched.threads.back().expect("the spawned child");
        assert_eq!(
            child.current_loaded_module,
            Some(0),
            "the child must keep executing the module its parent was in"
        );
        assert_eq!(
            child.caller_mp_stack, caller_mp_stack,
            "the child must keep the caller MP stack `spawn` gave it"
        );
        assert_eq!(
            child.mp, module_mp,
            "the child must run against the module MP its parent was using"
        );
        assert_eq!(
            sched.threads.front().expect("parent").current_loaded_module,
            Some(0),
            "dispatch must not lose the parent's module either"
        );
    }

    /// The preemptive worker must carry the same context across, so the two
    /// schedulers cannot disagree about what a spawned thread holds.
    #[test]
    fn shared_quanta_preserves_the_spawning_threads_module_context() {
        let mut frames = entry_frames();
        let pending = frames.alloc_pending(64).expect("alloc_pending");
        // The main module would halt at pc 0: only a thread that fetches from
        // its own loaded module reaches the spawn.
        let main = module_with_code("shared_ctx", vec![exit_instruction()]);
        let loaded = module_with_code(
            "shared_ctx_lib",
            vec![
                Instruction {
                    opcode: Opcode::Spawn,
                    source: imm_operand(pending as i32),
                    middle: MiddleOperand::UNUSED,
                    destination: imm_operand(1),
                },
                exit_instruction(),
            ],
        );
        let mut state = shared_state(&main);
        state.loaded_modules.push(LoadedModule {
            module: loaded,
            mp: Vec::new(),
        });

        let caller_mp_stack = vec![(0usize, vec![1u8, 0, 0, 0])];
        let mut thread = vm_thread(frames, 0);
        thread.current_loaded_module = Some(0);
        thread.caller_mp_stack = caller_mp_stack.clone();
        let mut spawned = Vec::new();

        run_thread_quanta_shared(&mut state, &mut thread, 1, &mut spawned)
            .expect("quanta should not error");

        assert_eq!(spawned.len(), 1, "the spawned child must be handed back");
        assert_eq!(
            spawned[0].current_loaded_module,
            Some(0),
            "the child must keep executing the module its parent was in"
        );
        assert_eq!(
            spawned[0].caller_mp_stack, caller_mp_stack,
            "the child must keep the caller MP stack `spawn` gave it"
        );
    }

    /// Regression: a receive from an empty channel must block the thread. The
    /// scheduler used to ignore `blocked_channel`, advancing the PC and leaving
    /// the receive destination holding stale bytes.
    #[test]
    fn recv_from_an_empty_channel_blocks_instead_of_advancing() {
        let module = recv_module("sched_block");
        let mut sched = scheduler_with_thread(&module, 0);
        let chan = sched.heap.alloc(
            0,
            HeapData::Channel {
                elem_size: 4,
                pending: None,
            },
        );
        {
            let thread = sched.threads.front_mut().expect("thread");
            let fp = thread.frames.current_data_offset();
            memory::write_word(&mut thread.frames.data, fp, chan as i32);
            memory::write_word(&mut thread.frames.data, fp + 8, 0x5eed); // stale value
        }

        sched.run_thread_quanta(4).expect("quanta should not error");

        let thread = sched.threads.front().expect("thread");
        assert_eq!(
            thread.state,
            ThreadState::Blocked(chan),
            "a receive with no data must block the thread"
        );
        assert_eq!(thread.pc, 0, "a blocked receive must re-execute when woken");
        let fp = thread.frames.current_data_offset();
        assert_eq!(
            memory::read_word(&thread.frames.data, fp + 8),
            0x5eed,
            "a blocked receive must not pretend to have delivered a value"
        );
    }

    /// With every thread blocked nothing can wake anything: report a deadlock
    /// instead of running a blocked thread over and over.
    #[test]
    fn run_faults_when_every_thread_is_blocked() {
        let module = recv_module("sched_deadlock");
        let mut sched = scheduler_with_thread(&module, 0);
        let chan = sched.heap.alloc(
            0,
            HeapData::Channel {
                elem_size: 4,
                pending: None,
            },
        );
        {
            let thread = sched.threads.front_mut().expect("thread");
            let fp = thread.frames.current_data_offset();
            memory::write_word(&mut thread.frames.data, fp, chan as i32);
        }

        match sched.run() {
            Err(ExecError::ThreadFault(msg)) => {
                assert!(msg.contains("deadlock"), "expected a deadlock fault: {msg}");
            }
            other => panic!("a blocked-only schedule must deadlock, got {other:?}"),
        }
    }

    fn imm_operand(value: i32) -> Operand {
        Operand {
            mode: AddressMode::Immediate,
            register1: value,
            register2: 0,
        }
    }

    fn shared_state(module: &Module) -> SharedState<'_> {
        SharedState {
            module,
            heap: Heap::new(),
            modules: ModuleRegistry::new(),
            loaded_modules: Vec::new(),
            files: FileTable::new(),
            gc_enabled: false,
            gc_counter: 0,
            trace: false,
            next_thread_id: 1,
            progress: 0,
        }
    }

    fn vm_thread(frames: FrameStack, pc: usize) -> VmThread {
        VmThread::new(1, frames, Vec::new(), pc)
    }

    /// Regression: the preemptive worker built a throwaway VmState per
    /// instruction, so a thread spawned by the running thread was dropped.
    #[test]
    fn shared_quanta_keeps_threads_spawned_by_the_running_thread() {
        let mut frames = entry_frames();
        let pending = frames.alloc_pending(64).expect("alloc_pending");
        let module = module_with_code(
            "shared_spawn",
            vec![
                Instruction {
                    opcode: Opcode::Spawn,
                    source: imm_operand(pending as i32),
                    middle: MiddleOperand::UNUSED,
                    destination: imm_operand(1),
                },
                exit_instruction(),
            ],
        );
        let mut state = shared_state(&module);
        let mut thread = vm_thread(frames, 0);
        let mut spawned = Vec::new();

        run_thread_quanta_shared(&mut state, &mut thread, 1, &mut spawned)
            .expect("quanta should not error");

        assert_eq!(spawned.len(), 1, "the spawned child must be handed back");
        assert_eq!(spawned[0].pc, 1, "the child starts at the spawn target pc");
    }

    /// Regression: `blocked_channel` was ignored, so a receive with no data
    /// advanced the PC and left the destination holding stale bytes; the fix
    /// for that then made it a fatal error, which kills any program using a
    /// channel for flow control (a full single-slot channel is back-pressure,
    /// not a fault). The thread must park and re-execute the operation.
    #[test]
    fn shared_quanta_parks_a_blocking_channel_operation() {
        let module = recv_module("shared_block");
        let mut state = shared_state(&module);
        let chan = state.heap.alloc(
            0,
            HeapData::Channel {
                elem_size: 4,
                pending: None,
            },
        );
        let mut thread = vm_thread(entry_frames(), 0);
        let fp = thread.frames.current_data_offset();
        memory::write_word(&mut thread.frames.data, fp, chan as i32);
        memory::write_word(&mut thread.frames.data, fp + 8, 0x5eed); // stale value
        let mut spawned = Vec::new();

        let executed = run_thread_quanta_shared(&mut state, &mut thread, 4, &mut spawned)
            .expect("a blocking channel operation must not kill the VM");

        assert_eq!(executed, 0, "the blocking instruction did not retire");
        assert_eq!(
            thread.state,
            ThreadState::Blocked(chan),
            "a receive with no data must park the thread"
        );
        assert_eq!(thread.pc, 0, "a parked receive must re-execute when woken");
        assert_eq!(
            memory::read_word(&thread.frames.data, fp + 8),
            0x5eed,
            "a blocked receive must not pretend to have delivered a value"
        );
    }

    /// A single-slot channel that already holds a payload makes `send` block.
    /// That is ordinary producer/consumer back-pressure: the pool must run the
    /// consumer and let the producer retry, not abort the program.
    #[test]
    fn preemptive_run_survives_channel_back_pressure() {
        const MESSAGES: usize = 8;
        // Producer: MESSAGES sends through a one-slot channel, then exit.
        // Consumer: MESSAGES receives, then exit. Every send but the first
        // meets back-pressure, and every receive but the first an empty
        // channel, so both threads park and retry repeatedly.
        let mut code: Vec<Instruction> = (0..MESSAGES)
            .map(|_| Instruction {
                opcode: Opcode::Send,
                source: fp_operand(4),
                middle: MiddleOperand::UNUSED,
                destination: fp_operand(0),
            })
            .collect();
        code.push(exit_instruction());
        let consumer_pc = code.len();
        code.extend((0..MESSAGES).map(|_| Instruction {
            opcode: Opcode::Recv,
            source: fp_operand(0),
            middle: MiddleOperand::UNUSED,
            destination: fp_operand(8),
        }));
        code.push(exit_instruction());
        let module = module_with_code("preempt_pipe", code);

        let mut heap = Heap::new();
        let chan = heap.alloc(
            0,
            HeapData::Channel {
                elem_size: 4,
                pending: None,
            },
        );
        let thread_with_channel = |pc: usize| {
            let mut thread = vm_thread(entry_frames(), pc);
            let fp = thread.frames.current_data_offset();
            memory::write_word(&mut thread.frames.data, fp, chan as i32);
            memory::write_word(&mut thread.frames.data, fp + 4, 0x1234);
            thread
        };

        let mut sched =
            PreemptiveScheduler::new(&module, heap, ModuleRegistry::new(), FileTable::new(), 4);
        sched.add_thread(thread_with_channel(0));
        sched.add_thread(thread_with_channel(consumer_pc));

        sched
            .run()
            .expect("back-pressure must not abort the program");

        let state = sched.shared.lock().unwrap_or_else(|e| e.into_inner());
        match state.heap.get(chan).map(|obj| &obj.data) {
            Some(HeapData::Channel { pending, .. }) => assert!(
                pending.is_none(),
                "both sends must have been consumed by the receiver"
            ),
            other => panic!("expected the channel to survive, got {other:?}"),
        }
    }

    /// Parking a blocked thread must not hang the pool: with nothing running
    /// and only blocked threads left, no channel operation can ever happen.
    #[test]
    fn preemptive_run_reports_a_deadlock_when_every_thread_is_blocked() {
        let module = recv_module("preempt_deadlock");
        let mut heap = Heap::new();
        let chan = heap.alloc(
            0,
            HeapData::Channel {
                elem_size: 4,
                pending: None,
            },
        );
        let mut thread = vm_thread(entry_frames(), 0);
        let fp = thread.frames.current_data_offset();
        memory::write_word(&mut thread.frames.data, fp, chan as i32);

        let mut sched =
            PreemptiveScheduler::new(&module, heap, ModuleRegistry::new(), FileTable::new(), 2);
        sched.add_thread(thread);

        match sched.run() {
            Err(ExecError::ThreadFault(msg)) => {
                assert!(msg.contains("deadlock"), "expected a deadlock fault: {msg}");
            }
            other => panic!("a blocked-only schedule must deadlock, got {other:?}"),
        }
    }

    /// A send that fills a channel must wake the threads blocked receiving on
    /// it, otherwise they stay blocked and the scheduler reports a deadlock.
    #[test]
    fn a_send_wakes_threads_blocked_on_that_channel() {
        let module = recv_module("sched_wake");
        let mut sched = scheduler_with_thread(&module, 0);
        let chan = sched.heap.alloc(
            0,
            HeapData::Channel {
                elem_size: 4,
                pending: None,
            },
        );

        // A second thread, blocked receiving on the channel.
        sched.spawn_thread(entry_frames(), Vec::new(), 0);
        if let Some(blocked) = sched.threads.back_mut() {
            blocked.state = ThreadState::Blocked(chan);
        }

        // The running thread sends on the channel.
        {
            let thread = sched.threads.front_mut().expect("thread");
            thread.src = AddrTarget::Immediate;
            thread.imm_src = 7;
            thread.dst = AddrTarget::Immediate;
            thread.imm_dst = chan as i32;
            thread.next_pc = 1;
        }
        let send = Instruction {
            opcode: Opcode::Send,
            source: Operand::UNUSED,
            middle: MiddleOperand::UNUSED,
            destination: Operand::UNUSED,
        };
        dispatch_for_thread(&mut sched, &send).expect("send should dispatch");

        assert_eq!(
            sched.threads.back().expect("blocked thread").state,
            ThreadState::Ready,
            "a send must wake the threads blocked on that channel"
        );
    }
}
