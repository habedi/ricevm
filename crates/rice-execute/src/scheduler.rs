#![allow(dead_code)]
//! Cooperative thread scheduler for the Dis VM.
//!
//! Each thread has its own frame stack, PC, and MP. The scheduler runs threads
//! in round-robin order, giving each a time quanta of instructions before switching.

use std::collections::VecDeque;

use ricevm_core::{ExecError, Instruction, Module};

use crate::address::{self, AddrTarget};
use crate::builtin::ModuleRegistry;
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
    pub id: u32,
}

/// The scheduler manages multiple threads sharing a common heap and module table.
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
            id,
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
                let thread = self.threads.front_mut().unwrap();
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

/// Dispatch an instruction for the current front thread.
/// This bridges the Scheduler model to the existing VmState-based ops.
fn dispatch_for_thread(sched: &mut Scheduler<'_>, inst: &Instruction) -> Result<(), ExecError> {
    // Create a temporary VmState from the scheduler + current thread
    let thread = sched.threads.front_mut().unwrap();
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
        gc_enabled: false, // GC runs at scheduler level, not per-dispatch
        gc_counter: 0,
        src: thread.src,
        mid: thread.mid,
        dst: thread.dst,
        imm_src: thread.imm_src,
        imm_mid: thread.imm_mid,
        imm_dst: thread.imm_dst,
        heap_refs: std::mem::take(&mut thread.heap_refs),
    };

    let result = ops::dispatch(&mut vm, inst);

    // Move state back
    let thread = sched.threads.front_mut().unwrap();
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
