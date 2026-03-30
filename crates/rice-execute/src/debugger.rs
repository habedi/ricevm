//! Interactive debugger for the Dis VM.
//!
//! Provides breakpoints, single-stepping, stack inspection, and backtrace.

use std::collections::HashSet;
use std::io::{self, BufRead, Write};

use ricevm_core::{ExecError, Module, Pc};

use crate::memory;
use crate::ops;
use crate::vm::VmState;

/// Interactive debugger wrapping a VmState.
pub(crate) struct Debugger<'m> {
    vm: VmState<'m>,
    breakpoints: HashSet<usize>,
    module: &'m Module,
}

impl<'m> Debugger<'m> {
    pub fn new(module: &'m Module) -> Result<Self, ExecError> {
        let vm = VmState::new(module)?;
        Ok(Self {
            vm,
            breakpoints: HashSet::new(),
            module,
        })
    }

    /// Run the interactive debugger loop.
    pub fn run_interactive(&mut self) -> Result<(), ExecError> {
        println!("RiceVM debugger. Type 'help' for commands.");
        self.print_current_instruction();

        let stdin = io::stdin();
        let mut last_cmd = String::new();

        loop {
            print!("(ricevm) ");
            io::stdout().flush().unwrap_or(());

            let mut line = String::new();
            if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            let line = line.trim().to_string();
            let cmd = if line.is_empty() {
                last_cmd.clone()
            } else {
                last_cmd = line.clone();
                line
            };

            let parts: Vec<&str> = cmd.split_whitespace().collect();
            if parts.is_empty() {
                continue;
            }

            match parts[0] {
                "step" | "s" => {
                    if self.vm.halted {
                        println!("Program has exited.");
                        continue;
                    }
                    self.step()?;
                    self.print_current_instruction();
                }
                "continue" | "c" => {
                    self.run_to_breakpoint()?;
                    if self.vm.halted {
                        println!("Program exited.");
                    } else {
                        self.print_current_instruction();
                    }
                }
                "break" | "b" => {
                    if let Some(pc_str) = parts.get(1) {
                        if let Ok(pc) = pc_str.parse::<usize>() {
                            self.breakpoints.insert(pc);
                            println!("Breakpoint set at pc={pc}");
                        } else {
                            println!("Invalid PC: {pc_str}");
                        }
                    } else {
                        println!("Usage: break <pc>");
                    }
                }
                "delete" | "d" => {
                    if let Some(pc_str) = parts.get(1) {
                        if let Ok(pc) = pc_str.parse::<usize>() {
                            if self.breakpoints.remove(&pc) {
                                println!("Breakpoint at pc={pc} removed.");
                            } else {
                                println!("No breakpoint at pc={pc}.");
                            }
                        }
                    } else {
                        println!("Usage: delete <pc>");
                    }
                }
                "breakpoints" | "bl" => {
                    if self.breakpoints.is_empty() {
                        println!("No breakpoints set.");
                    } else {
                        let mut bps: Vec<_> = self.breakpoints.iter().copied().collect();
                        bps.sort();
                        for bp in bps {
                            println!("  pc={bp}");
                        }
                    }
                }
                "print" | "p" => {
                    if let Some(what) = parts.get(1) {
                        self.print_info(what, parts.get(2).copied());
                    } else {
                        println!("Usage: print pc|fp|stack|heap|inst|word <offset>");
                    }
                }
                "list" | "l" => {
                    self.list_instructions();
                }
                "backtrace" | "bt" => {
                    self.print_backtrace();
                }
                "quit" | "q" => {
                    println!("Exiting debugger.");
                    return Ok(());
                }
                "help" | "h" => {
                    Self::print_help();
                }
                _ => {
                    println!("Unknown command: '{}'. Type 'help' for commands.", parts[0]);
                }
            }
        }
        Ok(())
    }

    fn step(&mut self) -> Result<(), ExecError> {
        if self.vm.pc >= self.module.code.len() {
            self.vm.halted = true;
            return Ok(());
        }
        let inst = self.module.code[self.vm.pc].clone();
        self.vm.resolve_operands(&inst)?;
        self.vm.next_pc = self.vm.pc + 1;
        ops::dispatch(&mut self.vm, &inst)?;
        self.vm.pc = self.vm.next_pc;

        if self.vm.gc_enabled {
            self.vm.gc_counter += 1;
            if self.vm.gc_counter >= 10_000 {
                self.vm.gc_counter = 0;
                crate::gc::collect(
                    &mut self.vm.heap,
                    &self.vm.frames,
                    &self.vm.mp,
                    &self.vm.loaded_modules,
                );
            }
        }
        Ok(())
    }

    fn run_to_breakpoint(&mut self) -> Result<(), ExecError> {
        loop {
            if self.vm.halted {
                return Ok(());
            }
            self.step()?;
            if self.breakpoints.contains(&self.vm.pc) {
                println!("Hit breakpoint at pc={}", self.vm.pc);
                return Ok(());
            }
        }
    }

    fn print_current_instruction(&self) {
        if self.vm.halted {
            println!("  (halted)");
            return;
        }
        if self.vm.pc < self.module.code.len() {
            let inst = &self.module.code[self.vm.pc];
            println!("  {:4}: {:?}", self.vm.pc, inst.opcode);
        }
    }

    fn list_instructions(&self) {
        let start = self.vm.pc.saturating_sub(5);
        let end = (self.vm.pc + 10).min(self.module.code.len());
        for i in start..end {
            let marker = if i == self.vm.pc { ">" } else { " " };
            let bp = if self.breakpoints.contains(&i) {
                "*"
            } else {
                " "
            };
            println!("{bp}{marker} {:4}: {:?}", i, self.module.code[i].opcode);
        }
    }

    fn print_info(&self, what: &str, arg: Option<&str>) {
        match what {
            "pc" => println!("  pc = {}", self.vm.pc),
            "fp" => println!("  fp = {}", self.vm.frames.current_data_offset()),
            "stack" => {
                println!("  frame stack: {} bytes", self.vm.frames.data.len());
                println!("  current frame base: {}", self.vm.frames.current_data_offset());
            }
            "heap" => {
                println!("  heap objects: {}", self.vm.heap.len());
            }
            "inst" => {
                if self.vm.pc < self.module.code.len() {
                    println!("  {:?}", self.module.code[self.vm.pc]);
                }
            }
            "word" => {
                if let Some(off_str) = arg {
                    if let Ok(off) = off_str.parse::<usize>() {
                        let fp = self.vm.frames.current_data_offset();
                        let abs = fp + off;
                        if abs + 4 <= self.vm.frames.data.len() {
                            let val = memory::read_word(&self.vm.frames.data, abs);
                            println!("  fp+{off} = {val} (0x{val:08x})");
                        } else {
                            println!("  offset out of bounds");
                        }
                    }
                } else {
                    println!("Usage: print word <offset>");
                }
            }
            _ => println!("Unknown print target: {what}"),
        }
    }

    fn print_backtrace(&self) {
        println!("  #0 pc={} (current)", self.vm.pc);
        // Walk frame chain by reading prev_base and prev_pc from frame headers.
        let mut base = self.vm.frames.current_data_offset();
        let mut depth = 1;
        loop {
            if base < 16 {
                break;
            }
            let header_base = base - 16; // FRAME_HEADER_SIZE
            if header_base + 8 > self.vm.frames.data.len() {
                break;
            }
            let prev_pc = memory::read_word(&self.vm.frames.data, header_base) as Pc;
            let prev_base = memory::read_word(&self.vm.frames.data, header_base + 4) as usize;
            if prev_pc < 0 {
                println!("  #{depth} pc=<entry> (bottom of stack)");
                break;
            }
            println!("  #{depth} pc={prev_pc}");
            if prev_base == 0 || prev_base >= base {
                break;
            }
            base = prev_base;
            depth += 1;
            if depth > 100 {
                println!("  ... (truncated)");
                break;
            }
        }
    }

    fn print_help() {
        println!("Commands:");
        println!("  step (s)         - Execute one instruction");
        println!("  continue (c)     - Run until breakpoint or exit");
        println!("  break (b) <pc>   - Set breakpoint at PC");
        println!("  delete (d) <pc>  - Remove breakpoint");
        println!("  breakpoints (bl) - List breakpoints");
        println!("  print (p) <what> - Print state: pc, fp, stack, heap, inst, word <offset>");
        println!("  list (l)         - Show instructions around current PC");
        println!("  backtrace (bt)   - Show call stack");
        println!("  quit (q)         - Exit debugger");
        println!("  help (h)         - Show this help");
    }
}
