//! Interactive debugger for the Dis VM.
//!
//! Provides breakpoints, single-stepping, stack inspection, and backtrace.

use std::collections::HashSet;
use std::io::{self, BufRead, Write};

use colored::Colorize;
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
        println!("{}", "RiceVM debugger. Type 'help' for commands.".bold());
        self.print_current_instruction();

        let stdin = io::stdin();
        let mut last_cmd = String::new();

        loop {
            print!("{} ", "(ricevm)".bold());
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
                "step" | "s" | "n" => {
                    if self.vm.halted {
                        println!("Program has exited.");
                        continue;
                    }
                    self.step()?;
                    self.print_current_instruction();
                }
                "continue" | "c" | "r" => {
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
                            println!(
                                "{} set at pc={}",
                                "Breakpoint".red().bold(),
                                format!("{pc}").yellow()
                            );
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
                                println!(
                                    "{} at pc={} removed.",
                                    "Breakpoint".red().bold(),
                                    format!("{pc}").yellow()
                                );
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
                            println!("  {} pc={}", "*".red(), format!("{bp}").yellow());
                        }
                    }
                }
                "print" | "p" => {
                    if let Some(what) = parts.get(1) {
                        self.print_info(what, parts.get(2).copied());
                    } else {
                        // Default: show current instruction with full detail
                        self.print_current_instruction();
                    }
                }
                "info" | "i" => {
                    // GDB-style info command
                    if let Some(what) = parts.get(1) {
                        match *what {
                            "regs" | "registers" => {
                                self.print_info("pc", None);
                                self.print_info("fp", None);
                                self.print_info("stack", None);
                            }
                            "break" | "breakpoints" => {
                                if self.breakpoints.is_empty() {
                                    println!("No breakpoints set.");
                                } else {
                                    let mut bps: Vec<_> =
                                        self.breakpoints.iter().copied().collect();
                                    bps.sort();
                                    for bp in bps {
                                        println!("  {} pc={}", "*".red(), format!("{bp}").yellow());
                                    }
                                }
                            }
                            "frame" => self.print_info("stack", None),
                            "heap" => self.print_info("heap", None),
                            "mp" => {
                                println!(
                                    "  mp: {} bytes",
                                    format!("{}", self.vm.mp.len()).yellow()
                                );
                            }
                            _ => println!(
                                "Unknown info target: {what}. Try: regs, break, frame, heap, mp"
                            ),
                        }
                    } else {
                        println!("Usage: info regs|break|frame|heap|mp");
                    }
                }
                "list" | "l" => {
                    self.list_instructions();
                }
                "backtrace" | "bt" | "where" => {
                    self.print_backtrace();
                }
                "quit" | "q" | "exit" => {
                    println!("Exiting debugger.");
                    return Ok(());
                }
                "help" | "h" | "?" => {
                    Self::print_help();
                }
                _ => {
                    println!(
                        "Unknown command: '{}'. Type {} for commands.",
                        parts[0],
                        "help".bold()
                    );
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
                println!(
                    "Hit {} at pc={}",
                    "breakpoint".red().bold(),
                    format!("{}", self.vm.pc).yellow()
                );
                return Ok(());
            }
        }
    }

    fn print_current_instruction(&self) {
        if self.vm.halted {
            println!("  {}", "(halted)".dimmed());
            return;
        }
        if self.vm.pc < self.module.code.len() {
            let inst = &self.module.code[self.vm.pc];
            let operands = self.format_instruction_operands(inst);
            if operands.is_empty() {
                println!(
                    "  {}: {}",
                    format!("{:4}", self.vm.pc).yellow(),
                    format!("{:?}", inst.opcode).cyan().bold()
                );
            } else {
                println!(
                    "  {}: {} {}",
                    format!("{:4}", self.vm.pc).yellow(),
                    format!("{:?}", inst.opcode).cyan().bold(),
                    operands
                );
            }
        }
    }

    fn list_instructions(&self) {
        let start = self.vm.pc.saturating_sub(5);
        let end = (self.vm.pc + 10).min(self.module.code.len());
        for i in start..end {
            let bp = if self.breakpoints.contains(&i) {
                format!("{}", "*".red())
            } else {
                " ".to_string()
            };
            let marker = if i == self.vm.pc { ">" } else { " " };
            let addr = format!("{:4}", i).yellow();
            let opcode = format!("{:?}", self.module.code[i].opcode);
            if i == self.vm.pc {
                println!("{bp}{marker} {addr}: {}", opcode.cyan().bold());
            } else {
                println!("{bp}{marker} {addr}: {opcode}");
            }
        }
    }

    fn print_info(&self, what: &str, arg: Option<&str>) {
        match what {
            "pc" => println!("  pc = {}", format!("{}", self.vm.pc).yellow()),
            "fp" => println!(
                "  fp = {}",
                format!("{}", self.vm.frames.current_data_offset()).yellow()
            ),
            "stack" => {
                println!(
                    "  frame stack: {} bytes",
                    format!("{}", self.vm.frames.data.len()).yellow()
                );
                println!(
                    "  current frame base: {}",
                    format!("{}", self.vm.frames.current_data_offset()).yellow()
                );
            }
            "heap" => {
                println!(
                    "  heap objects: {}",
                    format!("{}", self.vm.heap.len()).yellow()
                );
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
                            println!(
                                "  fp+{off} = {} ({})",
                                format!("{val}").yellow(),
                                format!("0x{val:08x}").yellow()
                            );
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
        println!("  #0 pc={} (current)", format!("{}", self.vm.pc).yellow());
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
            println!("  #{depth} pc={}", format!("{prev_pc}").yellow());
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

    fn format_instruction_operands(&self, inst: &ricevm_core::Instruction) -> String {
        use ricevm_core::{AddressMode, MiddleMode};
        let mut parts = Vec::new();
        if inst.source.mode != AddressMode::None {
            parts.push(format!("src={}", Self::format_operand(&inst.source)));
        }
        if inst.middle.mode != MiddleMode::None {
            parts.push(format!("mid={}", Self::format_mid(&inst.middle)));
        }
        if inst.destination.mode != AddressMode::None {
            parts.push(format!("dst={}", Self::format_operand(&inst.destination)));
        }
        parts.join(" ")
    }

    fn format_operand(op: &ricevm_core::Operand) -> String {
        use ricevm_core::AddressMode;
        match op.mode {
            AddressMode::OffsetIndirectFp => format!("{}(fp)", op.register1),
            AddressMode::OffsetIndirectMp => format!("{}(mp)", op.register1),
            AddressMode::OffsetDoubleIndirectFp => {
                format!("{}({}(fp))", op.register2, op.register1)
            }
            AddressMode::OffsetDoubleIndirectMp => {
                format!("{}({}(mp))", op.register2, op.register1)
            }
            AddressMode::Immediate => format!("${}", op.register1 as i16),
            _ => String::new(),
        }
    }

    fn format_mid(op: &ricevm_core::MiddleOperand) -> String {
        use ricevm_core::MiddleMode;
        match op.mode {
            MiddleMode::None => String::new(),
            MiddleMode::SmallImmediate => format!("${}", op.register1 as i16),
            MiddleMode::SmallOffsetFp => format!("{}(fp)", op.register1),
            MiddleMode::SmallOffsetMp => format!("{}(mp)", op.register1),
        }
    }

    fn print_help() {
        println!("{}", "Commands:".bold());
        let cmds = [
            ("step (s, n)", "Execute one instruction"),
            ("continue (c, r)", "Run until breakpoint or exit"),
            ("break (b) <pc>", "Set breakpoint at PC"),
            ("delete (d) <pc>", "Remove breakpoint"),
            ("breakpoints (bl)", "List breakpoints"),
            (
                "print (p) [what]",
                "Print state: pc, fp, stack, heap, inst, word <offset>",
            ),
            ("info (i) <what>", "Show info: regs, break, frame, heap, mp"),
            ("list (l)", "Show instructions around current PC"),
            ("backtrace (bt)", "Show call stack"),
            ("quit (q)", "Exit debugger"),
            ("help (h, ?)", "Show this help"),
        ];
        for (cmd, desc) in cmds {
            println!("  {:<22} {}", cmd.bold(), desc.dimmed());
        }
        println!();
        println!("  {}", "Press Enter to repeat the last command.".dimmed());
    }
}
