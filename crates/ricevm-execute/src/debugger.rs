//! Interactive debugger for the Dis VM.
//!
//! Provides breakpoints, single-stepping, stack inspection, and backtrace.
//!
//! The command loop is split so that every decision can be tested without a
//! terminal. `parse_command` turns a line of text into a `Command`, the state
//! transitions below turn a `Command` into a change in debugger state plus a
//! list of `InfoValue` results, and `run_interactive` is the thin shell that
//! reads lines from stdin and prints what the transitions produced.

use std::collections::HashSet;
use std::io::{self, BufRead, Write};

use colored::Colorize;
use ricevm_core::{ExecError, Module, Pc, Word};

use crate::memory;
use crate::ops;
use crate::vm::VmState;

/// Number of instructions the `list` command shows before the current pc.
const LIST_BEFORE: usize = 5;

/// Number of instructions the `list` command shows from the current pc onward.
const LIST_AFTER: usize = 10;

/// Number of frames a backtrace walks before it gives up.
const MAX_BACKTRACE_DEPTH: usize = 100;

/// Number of stepped instructions between garbage collections.
const GC_INTERVAL: usize = 10_000;

/// A command the debugger understands, as parsed from one line of input.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Step,
    Continue,
    Break(usize),
    Delete(usize),
    ListBreakpoints,
    /// Print state. An absent target means print the current instruction.
    Print {
        target: Option<String>,
        arg: Option<String>,
    },
    Info(InfoTarget),
    List,
    Backtrace,
    Quit,
    Help,
}

/// A target of the `info` command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InfoTarget {
    Regs,
    Breakpoints,
    Frame,
    Heap,
    Mp,
}

/// The reason a line of input produced no command.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ParseError {
    /// The line held no words at all.
    Empty,
    /// The first word is not a command the debugger knows.
    UnknownCommand(String),
    /// A required argument was missing. The payload is the usage text to show.
    MissingArgument(&'static str),
    /// A pc argument was not a number the debugger can use.
    InvalidPc(String),
    /// The `info` target is not one the debugger knows.
    UnknownInfoTarget(String),
}

/// Whether the command loop keeps reading input after a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Continue,
    Quit,
}

/// The reason `run_to_breakpoint` gave execution back to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopReason {
    /// Execution reached a breakpoint at this pc.
    Breakpoint(usize),
    /// The program halted before reaching a breakpoint.
    Halted,
}

/// One line of a backtrace above frame #0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BacktraceEntry {
    /// A caller frame that saved this pc.
    Caller(Pc),
    /// The entry frame, which has no caller.
    Entry,
    /// The walk gave up because the frame chain was too long.
    Truncated,
}

/// A single value the `print` and `info` commands report about VM state.
#[derive(Debug, Clone, PartialEq, Eq)]
enum InfoValue {
    Pc(usize),
    Fp(usize),
    FrameStackBytes(usize),
    CurrentFrameBase(usize),
    HeapObjects(usize),
    MpBytes(usize),
    Instruction(String),
    Word { offset: usize, value: Word },
    OffsetOutOfBounds,
    Breakpoint(usize),
    NoBreakpoints,
    Usage(&'static str),
    UnknownTarget(String),
}

/// Parse one line of debugger input into a command.
///
/// The grammar lives here rather than in the command loop so that it can be
/// tested without stdin.
fn parse_command(line: &str) -> Result<Command, ParseError> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    let (name, args) = parts.split_first().ok_or(ParseError::Empty)?;
    match *name {
        "step" | "s" | "n" => Ok(Command::Step),
        "continue" | "c" | "r" => Ok(Command::Continue),
        "break" | "b" => Ok(Command::Break(parse_pc(args.first(), "break <pc>")?)),
        "delete" | "d" => Ok(Command::Delete(parse_pc(args.first(), "delete <pc>")?)),
        "breakpoints" | "bl" => Ok(Command::ListBreakpoints),
        "print" | "p" => Ok(Command::Print {
            target: args.first().map(|word| (*word).to_string()),
            arg: args.get(1).map(|word| (*word).to_string()),
        }),
        "info" | "i" => {
            let what = args
                .first()
                .ok_or(ParseError::MissingArgument("info regs|break|frame|heap|mp"))?;
            parse_info_target(what)
        }
        "list" | "l" => Ok(Command::List),
        "backtrace" | "bt" | "where" => Ok(Command::Backtrace),
        "quit" | "q" | "exit" => Ok(Command::Quit),
        "help" | "h" | "?" => Ok(Command::Help),
        other => Err(ParseError::UnknownCommand(other.to_string())),
    }
}

/// Parse the target word of the `info` command.
fn parse_info_target(what: &str) -> Result<Command, ParseError> {
    match what {
        "regs" | "registers" => Ok(Command::Info(InfoTarget::Regs)),
        "break" | "breakpoints" => Ok(Command::Info(InfoTarget::Breakpoints)),
        "frame" => Ok(Command::Info(InfoTarget::Frame)),
        "heap" => Ok(Command::Info(InfoTarget::Heap)),
        "mp" => Ok(Command::Info(InfoTarget::Mp)),
        other => Err(ParseError::UnknownInfoTarget(other.to_string())),
    }
}

/// Parse the pc argument that `break` and `delete` both take.
fn parse_pc(arg: Option<&&str>, usage: &'static str) -> Result<usize, ParseError> {
    let text = arg.ok_or(ParseError::MissingArgument(usage))?;
    text.parse::<usize>()
        .map_err(|_| ParseError::InvalidPc((*text).to_string()))
}

/// The message shown when a line of input does not parse.
///
/// An empty line has no message because it repeats the last command instead.
fn parse_error_message(err: &ParseError) -> Option<String> {
    match err {
        ParseError::Empty => None,
        ParseError::UnknownCommand(name) => Some(format!(
            "Unknown command: '{name}'. Type {} for commands.",
            "help".bold()
        )),
        ParseError::MissingArgument(usage) => Some(format!("Usage: {usage}")),
        ParseError::InvalidPc(text) => Some(format!("Invalid PC: {text}")),
        ParseError::UnknownInfoTarget(what) => Some(format!(
            "Unknown info target: {what}. Try: regs, break, frame, heap, mp"
        )),
    }
}

/// Pick the command text to run for a line of input.
///
/// An empty line repeats the last command, so the last non-empty line is
/// remembered in `last`.
fn command_text(line: &str, last: &mut String) -> String {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        last.clone()
    } else {
        *last = trimmed.to_string();
        trimmed.to_string()
    }
}

/// Set a breakpoint. Returns false when one was already set at that pc.
fn set_breakpoint(breakpoints: &mut HashSet<usize>, pc: usize) -> bool {
    breakpoints.insert(pc)
}

/// Clear a breakpoint. Returns false when there was no breakpoint at that pc.
fn clear_breakpoint(breakpoints: &mut HashSet<usize>, pc: usize) -> bool {
    breakpoints.remove(&pc)
}

/// Whether execution should stop at this pc.
fn should_stop(breakpoints: &HashSet<usize>, pc: usize) -> bool {
    breakpoints.contains(&pc)
}

/// The breakpoints in ascending pc order, which is the order they are listed in.
fn sorted_breakpoints(breakpoints: &HashSet<usize>) -> Vec<usize> {
    let mut sorted: Vec<usize> = breakpoints.iter().copied().collect();
    sorted.sort_unstable();
    sorted
}

/// The values the breakpoint list reports.
fn breakpoint_values(breakpoints: &HashSet<usize>) -> Vec<InfoValue> {
    let sorted = sorted_breakpoints(breakpoints);
    if sorted.is_empty() {
        vec![InfoValue::NoBreakpoints]
    } else {
        sorted.into_iter().map(InfoValue::Breakpoint).collect()
    }
}

/// The half-open instruction range that `list` shows around a pc.
fn list_window(pc: usize, code_len: usize) -> (usize, usize) {
    let start = pc.saturating_sub(LIST_BEFORE);
    let end = pc.saturating_add(LIST_AFTER).min(code_len);
    (start, end)
}

/// Walk the saved frame headers from `current_base` down towards the entry frame.
///
/// Each header holds the caller's pc followed by the caller's frame base, so the
/// walk follows `prev_base` down the stack. Both `current_base` and the stored
/// `prev_base` are frame bases, not data area offsets.
fn walk_backtrace(data: &[u8], current_base: usize) -> Vec<BacktraceEntry> {
    let mut entries = Vec::new();
    let mut base = current_base;
    loop {
        if base.saturating_add(8) > data.len() {
            break;
        }
        let prev_pc: Pc = memory::read_word(data, base);
        let prev_base = memory::read_word(data, base + 4) as usize;
        if prev_pc < 0 {
            entries.push(BacktraceEntry::Entry);
            break;
        }
        entries.push(BacktraceEntry::Caller(prev_pc));
        // The entry frame sits at base 0, and every caller sits below its
        // callee, so anything else is a corrupt chain and ends the walk.
        if prev_base == 0 || prev_base >= base {
            break;
        }
        base = prev_base;
        if entries.len() >= MAX_BACKTRACE_DEPTH {
            entries.push(BacktraceEntry::Truncated);
            break;
        }
    }
    entries
}

/// Render the operands of an instruction the way the debugger shows them.
fn format_instruction_operands(inst: &ricevm_core::Instruction) -> String {
    use ricevm_core::{AddressMode, MiddleMode};
    let mut parts = Vec::new();
    if inst.source.mode != AddressMode::None {
        parts.push(format!("src={}", format_operand(&inst.source)));
    }
    if inst.middle.mode != MiddleMode::None {
        parts.push(format!("mid={}", format_mid(&inst.middle)));
    }
    if inst.destination.mode != AddressMode::None {
        parts.push(format!("dst={}", format_operand(&inst.destination)));
    }
    parts.join(" ")
}

/// Render a source or destination operand in Dis assembler notation.
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

/// Render a middle operand in Dis assembler notation.
fn format_mid(op: &ricevm_core::MiddleOperand) -> String {
    use ricevm_core::MiddleMode;
    match op.mode {
        MiddleMode::None => String::new(),
        MiddleMode::SmallImmediate => format!("${}", op.register1 as i16),
        MiddleMode::SmallOffsetFp => format!("{}(fp)", op.register1),
        MiddleMode::SmallOffsetMp => format!("{}(mp)", op.register1),
    }
}

/// Print each reported value in the form the user sees.
fn print_values(values: &[InfoValue]) {
    for value in values {
        print_info_value(value);
    }
}

/// Print one reported value in the form the user sees.
fn print_info_value(value: &InfoValue) {
    match value {
        InfoValue::Pc(pc) => println!("  pc = {}", format!("{pc}").yellow()),
        InfoValue::Fp(fp) => println!("  fp = {}", format!("{fp}").yellow()),
        InfoValue::FrameStackBytes(bytes) => {
            println!("  frame stack: {} bytes", format!("{bytes}").yellow());
        }
        InfoValue::CurrentFrameBase(base) => {
            println!("  current frame base: {}", format!("{base}").yellow());
        }
        InfoValue::HeapObjects(count) => {
            println!("  heap objects: {}", format!("{count}").yellow());
        }
        InfoValue::MpBytes(bytes) => println!("  mp: {} bytes", format!("{bytes}").yellow()),
        InfoValue::Instruction(text) => println!("  {text}"),
        InfoValue::Word { offset, value } => println!(
            "  fp+{offset} = {} ({})",
            format!("{value}").yellow(),
            format!("0x{value:08x}").yellow()
        ),
        InfoValue::OffsetOutOfBounds => println!("  offset out of bounds"),
        InfoValue::Breakpoint(pc) => println!("  {} pc={}", "*".red(), format!("{pc}").yellow()),
        InfoValue::NoBreakpoints => println!("No breakpoints set."),
        InfoValue::Usage(usage) => println!("Usage: {usage}"),
        InfoValue::UnknownTarget(what) => println!("Unknown print target: {what}"),
    }
}

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
    ///
    /// This is the only part of the debugger that touches stdin and stdout. It
    /// reads a line, hands it to `parse_command`, and hands the command to
    /// `execute_command`.
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
            let cmd = command_text(&line, &mut last_cmd);

            match parse_command(&cmd) {
                Ok(command) => {
                    if self.execute_command(command)? == Flow::Quit {
                        return Ok(());
                    }
                }
                Err(err) => {
                    if let Some(message) = parse_error_message(&err) {
                        println!("{message}");
                    }
                }
            }
        }
        Ok(())
    }

    /// Apply one command to the debugger state and print what it produced.
    fn execute_command(&mut self, command: Command) -> Result<Flow, ExecError> {
        match command {
            Command::Step => {
                if self.vm.halted {
                    println!("Program has exited.");
                } else {
                    self.step()?;
                    self.print_current_instruction();
                }
            }
            Command::Continue => {
                if let StopReason::Breakpoint(pc) = self.run_to_breakpoint()? {
                    println!(
                        "Hit {} at pc={}",
                        "breakpoint".red().bold(),
                        format!("{pc}").yellow()
                    );
                }
                if self.vm.halted {
                    println!("Program exited.");
                } else {
                    self.print_current_instruction();
                }
            }
            Command::Break(pc) => {
                set_breakpoint(&mut self.breakpoints, pc);
                println!(
                    "{} set at pc={}",
                    "Breakpoint".red().bold(),
                    format!("{pc}").yellow()
                );
            }
            Command::Delete(pc) => {
                if clear_breakpoint(&mut self.breakpoints, pc) {
                    println!(
                        "{} at pc={} removed.",
                        "Breakpoint".red().bold(),
                        format!("{pc}").yellow()
                    );
                } else {
                    println!("No breakpoint at pc={pc}.");
                }
            }
            Command::ListBreakpoints => print_values(&breakpoint_values(&self.breakpoints)),
            Command::Print { target, arg } => match target {
                Some(what) => print_values(&self.print_values_for(&what, arg.as_deref())),
                // Without a target, `print` shows the current instruction.
                None => self.print_current_instruction(),
            },
            Command::Info(target) => print_values(&self.info_values(target)),
            Command::List => self.list_instructions(),
            Command::Backtrace => self.print_backtrace(),
            Command::Quit => {
                println!("Exiting debugger.");
                return Ok(Flow::Quit);
            }
            Command::Help => Self::print_help(),
        }
        Ok(Flow::Continue)
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
            if self.vm.gc_counter >= GC_INTERVAL {
                self.vm.gc_counter = 0;
                crate::gc::collect(
                    &mut self.vm.heap,
                    &self.vm.frames,
                    &self.vm.mp,
                    &self.vm.loaded_modules,
                    &self.vm.thread_queue,
                    &self.vm.caller_mp_stack,
                    &self.vm.heap_refs,
                );
            }
        }
        Ok(())
    }

    /// Step until a breakpoint is reached or the program halts.
    fn run_to_breakpoint(&mut self) -> Result<StopReason, ExecError> {
        loop {
            if self.vm.halted {
                return Ok(StopReason::Halted);
            }
            self.step()?;
            if should_stop(&self.breakpoints, self.vm.pc) {
                return Ok(StopReason::Breakpoint(self.vm.pc));
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
            let operands = format_instruction_operands(inst);
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
        let (start, end) = list_window(self.vm.pc, self.module.code.len());
        for i in start..end {
            let bp = if should_stop(&self.breakpoints, i) {
                format!("{}", "*".red())
            } else {
                " ".to_string()
            };
            let marker = if i == self.vm.pc { ">" } else { " " };
            let addr = format!("{i:4}").yellow();
            let opcode = format!("{:?}", self.module.code[i].opcode);
            if i == self.vm.pc {
                println!("{bp}{marker} {addr}: {}", opcode.cyan().bold());
            } else {
                println!("{bp}{marker} {addr}: {opcode}");
            }
        }
    }

    /// The values that `print <what>` reports, in the order they are shown.
    fn print_values_for(&self, what: &str, arg: Option<&str>) -> Vec<InfoValue> {
        match what {
            "pc" => vec![InfoValue::Pc(self.vm.pc)],
            "fp" => vec![InfoValue::Fp(self.vm.frames.current_data_offset())],
            "stack" => vec![
                InfoValue::FrameStackBytes(self.vm.frames.data.len()),
                InfoValue::CurrentFrameBase(self.vm.frames.current_data_offset()),
            ],
            "heap" => vec![InfoValue::HeapObjects(self.vm.heap.len())],
            "inst" => {
                if self.vm.pc < self.module.code.len() {
                    vec![InfoValue::Instruction(format!(
                        "{:?}",
                        self.module.code[self.vm.pc]
                    ))]
                } else {
                    Vec::new()
                }
            }
            "word" => self.word_values(arg),
            other => vec![InfoValue::UnknownTarget(other.to_string())],
        }
    }

    /// The value that `print word <offset>` reports for a frame offset.
    fn word_values(&self, arg: Option<&str>) -> Vec<InfoValue> {
        let Some(off_str) = arg else {
            return vec![InfoValue::Usage("print word <offset>")];
        };
        let Ok(offset) = off_str.parse::<usize>() else {
            return Vec::new();
        };
        // The offset is typed by the user, so both sums need checking before the
        // result is used as an address.
        let fits = self
            .vm
            .frames
            .current_data_offset()
            .checked_add(offset)
            .filter(|abs| {
                abs.checked_add(4)
                    .is_some_and(|end| end <= self.vm.frames.data.len())
            });
        match fits {
            Some(abs) => vec![InfoValue::Word {
                offset,
                value: memory::read_word(&self.vm.frames.data, abs),
            }],
            None => vec![InfoValue::OffsetOutOfBounds],
        }
    }

    /// The values that `info <target>` reports, in the order they are shown.
    fn info_values(&self, target: InfoTarget) -> Vec<InfoValue> {
        match target {
            InfoTarget::Regs => {
                let mut values = self.print_values_for("pc", None);
                values.extend(self.print_values_for("fp", None));
                values.extend(self.print_values_for("stack", None));
                values
            }
            InfoTarget::Frame => self.print_values_for("stack", None),
            InfoTarget::Heap => self.print_values_for("heap", None),
            InfoTarget::Mp => vec![InfoValue::MpBytes(self.vm.mp.len())],
            InfoTarget::Breakpoints => breakpoint_values(&self.breakpoints),
        }
    }

    fn print_backtrace(&self) {
        println!("  #0 pc={} (current)", format!("{}", self.vm.pc).yellow());
        let entries = walk_backtrace(&self.vm.frames.data, self.vm.frames.current_base);
        for (index, entry) in entries.iter().enumerate() {
            let depth = index + 1;
            match entry {
                BacktraceEntry::Caller(pc) => {
                    println!("  #{depth} pc={}", format!("{pc}").yellow())
                }
                BacktraceEntry::Entry => println!("  #{depth} pc=<entry> (bottom of stack)"),
                BacktraceEntry::Truncated => println!("  ... (truncated)"),
            }
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

#[cfg(test)]
mod tests {
    use ricevm_core::{
        AddressMode, Header, Instruction, MiddleMode, MiddleOperand, Opcode, Operand, PointerMap,
        RuntimeFlags, TypeDescriptor, XMAGIC,
    };

    use super::*;
    use crate::frame::FRAME_HEADER_SIZE;
    use crate::heap::HeapData;

    /// An instruction that does nothing and falls through to the next pc.
    fn nop() -> Instruction {
        Instruction {
            opcode: Opcode::Nop,
            source: Operand::UNUSED,
            middle: MiddleOperand::UNUSED,
            destination: Operand::UNUSED,
        }
    }

    /// An instruction that halts the program.
    fn exit() -> Instruction {
        Instruction {
            opcode: Opcode::Exit,
            ..nop()
        }
    }

    fn test_module(code: Vec<Instruction>) -> Module {
        let code_size = code.len() as i32;
        Module {
            header: Header {
                magic: XMAGIC,
                signature: vec![],
                runtime_flags: RuntimeFlags(0),
                stack_extent: 0,
                code_size,
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
            name: "debugger_test".to_string(),
            exports: vec![],
            imports: vec![],
            handlers: vec![],
        }
    }

    /// A module of `count` no-ops followed by an exit, so pc `count` is the exit.
    fn nop_module(count: usize) -> Module {
        let mut code: Vec<Instruction> = (0..count).map(|_| nop()).collect();
        code.push(exit());
        test_module(code)
    }

    fn print_cmd(target: Option<&str>, arg: Option<&str>) -> Command {
        Command::Print {
            target: target.map(str::to_string),
            arg: arg.map(str::to_string),
        }
    }

    // --- Command parsing ---

    #[test]
    fn parse_step_and_its_abbreviations() {
        for line in ["step", "s", "n"] {
            assert_eq!(parse_command(line), Ok(Command::Step), "line: {line}");
        }
    }

    #[test]
    fn parse_continue_and_its_abbreviations() {
        for line in ["continue", "c", "r"] {
            assert_eq!(parse_command(line), Ok(Command::Continue), "line: {line}");
        }
    }

    #[test]
    fn parse_break_with_a_pc() {
        assert_eq!(parse_command("break 7"), Ok(Command::Break(7)));
        assert_eq!(parse_command("b 0"), Ok(Command::Break(0)));
    }

    #[test]
    fn parse_delete_with_a_pc() {
        assert_eq!(parse_command("delete 3"), Ok(Command::Delete(3)));
        assert_eq!(parse_command("d 12"), Ok(Command::Delete(12)));
    }

    #[test]
    fn parse_breakpoint_list_and_its_abbreviation() {
        for line in ["breakpoints", "bl"] {
            assert_eq!(
                parse_command(line),
                Ok(Command::ListBreakpoints),
                "line: {line}"
            );
        }
    }

    #[test]
    fn parse_print_with_and_without_arguments() {
        assert_eq!(parse_command("print"), Ok(print_cmd(None, None)));
        assert_eq!(parse_command("p"), Ok(print_cmd(None, None)));
        assert_eq!(parse_command("p pc"), Ok(print_cmd(Some("pc"), None)));
        assert_eq!(
            parse_command("print word 12"),
            Ok(print_cmd(Some("word"), Some("12")))
        );
    }

    #[test]
    fn parse_every_info_target() {
        let cases = [
            ("info regs", InfoTarget::Regs),
            ("info registers", InfoTarget::Regs),
            ("i break", InfoTarget::Breakpoints),
            ("info breakpoints", InfoTarget::Breakpoints),
            ("info frame", InfoTarget::Frame),
            ("info heap", InfoTarget::Heap),
            ("info mp", InfoTarget::Mp),
        ];
        for (line, target) in cases {
            assert_eq!(
                parse_command(line),
                Ok(Command::Info(target)),
                "line: {line}"
            );
        }
    }

    #[test]
    fn parse_remaining_commands_and_their_abbreviations() {
        let cases = [
            ("list", Command::List),
            ("l", Command::List),
            ("backtrace", Command::Backtrace),
            ("bt", Command::Backtrace),
            ("where", Command::Backtrace),
            ("quit", Command::Quit),
            ("q", Command::Quit),
            ("exit", Command::Quit),
            ("help", Command::Help),
            ("h", Command::Help),
            ("?", Command::Help),
        ];
        for (line, expected) in cases {
            assert_eq!(parse_command(line), Ok(expected), "line: {line}");
        }
    }

    #[test]
    fn parse_ignores_surrounding_and_repeated_whitespace() {
        assert_eq!(parse_command("   break    12   "), Ok(Command::Break(12)));
        assert_eq!(parse_command("\tstep\n"), Ok(Command::Step));
    }

    #[test]
    fn parse_break_ignores_words_after_the_pc() {
        assert_eq!(
            parse_command("break 5 and then some"),
            Ok(Command::Break(5))
        );
    }

    #[test]
    fn parse_rejects_an_empty_line() {
        assert_eq!(parse_command(""), Err(ParseError::Empty));
        assert_eq!(parse_command("   \t "), Err(ParseError::Empty));
    }

    #[test]
    fn parse_rejects_an_unknown_command() {
        assert_eq!(
            parse_command("frobnicate 3"),
            Err(ParseError::UnknownCommand("frobnicate".to_string()))
        );
    }

    #[test]
    fn parse_is_case_sensitive() {
        assert_eq!(
            parse_command("STEP"),
            Err(ParseError::UnknownCommand("STEP".to_string()))
        );
    }

    #[test]
    fn parse_reports_a_missing_pc_argument() {
        assert_eq!(
            parse_command("break"),
            Err(ParseError::MissingArgument("break <pc>"))
        );
        assert_eq!(
            parse_command("delete"),
            Err(ParseError::MissingArgument("delete <pc>"))
        );
    }

    #[test]
    fn parse_reports_a_missing_info_target() {
        assert_eq!(
            parse_command("info"),
            Err(ParseError::MissingArgument("info regs|break|frame|heap|mp"))
        );
    }

    #[test]
    fn parse_reports_an_unknown_info_target() {
        assert_eq!(
            parse_command("info stack"),
            Err(ParseError::UnknownInfoTarget("stack".to_string()))
        );
    }

    #[test]
    fn parse_rejects_a_pc_that_is_not_a_number() {
        assert_eq!(
            parse_command("break abc"),
            Err(ParseError::InvalidPc("abc".to_string()))
        );
        assert_eq!(
            parse_command("break 1.5"),
            Err(ParseError::InvalidPc("1.5".to_string()))
        );
        assert_eq!(
            parse_command("delete xyz"),
            Err(ParseError::InvalidPc("xyz".to_string()))
        );
    }

    #[test]
    fn parse_rejects_a_negative_pc() {
        assert_eq!(
            parse_command("break -1"),
            Err(ParseError::InvalidPc("-1".to_string()))
        );
    }

    #[test]
    fn parse_rejects_a_pc_too_large_for_an_address() {
        // A pc that does not fit in a usize must be reported, not wrapped.
        let huge = "99999999999999999999999999999999";
        assert_eq!(
            parse_command(&format!("break {huge}")),
            Err(ParseError::InvalidPc(huge.to_string()))
        );
    }

    #[test]
    fn parse_error_messages_name_the_problem() {
        assert_eq!(parse_error_message(&ParseError::Empty), None);
        assert_eq!(
            parse_error_message(&ParseError::MissingArgument("break <pc>")),
            Some("Usage: break <pc>".to_string())
        );
        assert_eq!(
            parse_error_message(&ParseError::InvalidPc("abc".to_string())),
            Some("Invalid PC: abc".to_string())
        );
        assert_eq!(
            parse_error_message(&ParseError::UnknownInfoTarget("stack".to_string())),
            Some("Unknown info target: stack. Try: regs, break, frame, heap, mp".to_string())
        );
        let unknown = parse_error_message(&ParseError::UnknownCommand("zz".to_string()))
            .expect("unknown commands report a message");
        assert!(unknown.contains("Unknown command: 'zz'"), "{unknown}");
    }

    // --- Repeating the last command ---

    #[test]
    fn an_empty_line_repeats_the_last_command() {
        let mut last = String::new();
        assert_eq!(command_text("step", &mut last), "step");
        assert_eq!(command_text("", &mut last), "step");
        assert_eq!(command_text("   \n", &mut last), "step");
    }

    #[test]
    fn a_new_line_replaces_the_remembered_command() {
        let mut last = String::new();
        command_text("step", &mut last);
        assert_eq!(command_text(" break 4 ", &mut last), "break 4");
        assert_eq!(last, "break 4");
        assert_eq!(command_text("", &mut last), "break 4");
    }

    #[test]
    fn an_empty_line_with_no_history_yields_no_command() {
        let mut last = String::new();
        let text = command_text("", &mut last);
        assert_eq!(parse_command(&text), Err(ParseError::Empty));
    }

    // --- Breakpoint state transitions ---

    #[test]
    fn setting_a_breakpoint_twice_reports_the_duplicate() {
        let mut breakpoints = HashSet::new();
        assert!(set_breakpoint(&mut breakpoints, 4), "first set is new");
        assert!(
            !set_breakpoint(&mut breakpoints, 4),
            "second set is a duplicate"
        );
        assert_eq!(sorted_breakpoints(&breakpoints), vec![4]);
    }

    #[test]
    fn clearing_a_breakpoint_reports_whether_one_was_there() {
        let mut breakpoints = HashSet::new();
        set_breakpoint(&mut breakpoints, 4);
        assert!(clear_breakpoint(&mut breakpoints, 4), "4 was set");
        assert!(!clear_breakpoint(&mut breakpoints, 4), "4 is already gone");
        assert!(!clear_breakpoint(&mut breakpoints, 9), "9 was never set");
        assert!(breakpoints.is_empty());
    }

    #[test]
    fn execution_stops_only_at_a_breakpoint_pc() {
        let mut breakpoints = HashSet::new();
        set_breakpoint(&mut breakpoints, 3);
        assert!(
            !should_stop(&breakpoints, 2),
            "pc 2 is before the breakpoint"
        );
        assert!(should_stop(&breakpoints, 3), "pc 3 is the breakpoint");
        assert!(
            !should_stop(&breakpoints, 4),
            "pc 4 is after the breakpoint"
        );
    }

    #[test]
    fn a_cleared_breakpoint_no_longer_stops_execution() {
        let mut breakpoints = HashSet::new();
        set_breakpoint(&mut breakpoints, 3);
        clear_breakpoint(&mut breakpoints, 3);
        assert!(!should_stop(&breakpoints, 3));
    }

    #[test]
    fn breakpoints_are_listed_in_ascending_pc_order() {
        let mut breakpoints = HashSet::new();
        for pc in [9, 2, 7, 0] {
            set_breakpoint(&mut breakpoints, pc);
        }
        assert_eq!(sorted_breakpoints(&breakpoints), vec![0, 2, 7, 9]);
        assert_eq!(
            breakpoint_values(&breakpoints),
            vec![
                InfoValue::Breakpoint(0),
                InfoValue::Breakpoint(2),
                InfoValue::Breakpoint(7),
                InfoValue::Breakpoint(9),
            ]
        );
    }

    #[test]
    fn an_empty_breakpoint_list_says_so() {
        assert_eq!(
            breakpoint_values(&HashSet::new()),
            vec![InfoValue::NoBreakpoints]
        );
    }

    // --- Stepping ---

    #[test]
    fn step_advances_the_pc_by_exactly_one_instruction() {
        let module = nop_module(3);
        let mut dbg = Debugger::new(&module).expect("debugger should start");
        assert_eq!(dbg.vm.pc, 0, "execution starts at entry_pc");

        dbg.step().expect("step over a no-op");
        assert_eq!(dbg.vm.pc, 1);
        dbg.step().expect("step over a no-op");
        assert_eq!(dbg.vm.pc, 2);
        assert!(!dbg.vm.halted, "no-ops do not halt the program");
    }

    #[test]
    fn step_over_the_exit_instruction_halts() {
        let module = nop_module(1);
        let mut dbg = Debugger::new(&module).expect("debugger should start");
        dbg.step().expect("step over the no-op");
        assert_eq!(dbg.vm.pc, 1, "pc 1 holds the exit");
        assert!(!dbg.vm.halted);

        dbg.step().expect("step over the exit");
        assert!(dbg.vm.halted, "the exit instruction halts the program");
    }

    #[test]
    fn step_past_the_end_of_the_code_halts_without_executing() {
        let module = nop_module(1);
        let mut dbg = Debugger::new(&module).expect("debugger should start");
        dbg.vm.pc = module.code.len();
        dbg.step().expect("step past the end");
        assert!(dbg.vm.halted);
        assert_eq!(
            dbg.vm.pc,
            module.code.len(),
            "pc must not move past the end"
        );
    }

    #[test]
    fn stepping_a_halted_program_leaves_the_state_alone() {
        let module = nop_module(0);
        let mut dbg = Debugger::new(&module).expect("debugger should start");
        dbg.step().expect("step over the exit");
        assert!(dbg.vm.halted);
        let pc_at_halt = dbg.vm.pc;

        let flow = dbg
            .execute_command(Command::Step)
            .expect("step on a halted program");
        assert_eq!(flow, Flow::Continue);
        assert_eq!(dbg.vm.pc, pc_at_halt, "pc must not move after the exit");
    }

    #[test]
    fn stepping_collects_garbage_once_per_interval() {
        let module = nop_module(4);
        let mut dbg = Debugger::new(&module).expect("debugger should start");
        dbg.vm.gc_enabled = true;
        dbg.vm.gc_counter = 0;

        dbg.step().expect("step");
        assert_eq!(
            dbg.vm.gc_counter, 1,
            "each step counts towards the next collection"
        );

        dbg.vm.gc_counter = GC_INTERVAL - 1;
        dbg.step().expect("step over the interval");
        assert_eq!(
            dbg.vm.gc_counter, 0,
            "reaching the interval collects and resets the counter"
        );
        assert_eq!(dbg.vm.pc, 2, "collecting does not disturb the pc");
    }

    #[test]
    fn stepping_does_not_count_when_the_collector_is_off() {
        let module = nop_module(4);
        let mut dbg = Debugger::new(&module).expect("debugger should start");
        dbg.vm.gc_enabled = false;
        dbg.vm.gc_counter = 0;

        dbg.step().expect("step");
        assert_eq!(dbg.vm.gc_counter, 0);
        assert_eq!(dbg.vm.pc, 1, "the step still runs");
    }

    // --- Continuing to a breakpoint ---

    #[test]
    fn continue_stops_at_the_breakpoint_and_nowhere_else() {
        let module = nop_module(6);
        let mut dbg = Debugger::new(&module).expect("debugger should start");
        set_breakpoint(&mut dbg.breakpoints, 3);

        let reason = dbg.run_to_breakpoint().expect("run to the breakpoint");
        assert_eq!(reason, StopReason::Breakpoint(3));
        assert_eq!(dbg.vm.pc, 3, "execution stops at the breakpoint pc");
        assert!(!dbg.vm.halted, "the program has not run to the exit yet");
    }

    #[test]
    fn continue_without_a_breakpoint_runs_to_the_exit() {
        let module = nop_module(4);
        let mut dbg = Debugger::new(&module).expect("debugger should start");

        let reason = dbg.run_to_breakpoint().expect("run to the exit");
        assert_eq!(reason, StopReason::Halted);
        assert!(dbg.vm.halted);
    }

    #[test]
    fn continue_stops_at_each_breakpoint_in_turn() {
        let module = nop_module(6);
        let mut dbg = Debugger::new(&module).expect("debugger should start");
        set_breakpoint(&mut dbg.breakpoints, 2);
        set_breakpoint(&mut dbg.breakpoints, 5);

        assert_eq!(
            dbg.run_to_breakpoint().expect("first run"),
            StopReason::Breakpoint(2)
        );
        assert_eq!(dbg.vm.pc, 2);
        assert_eq!(
            dbg.run_to_breakpoint().expect("second run"),
            StopReason::Breakpoint(5)
        );
        assert_eq!(dbg.vm.pc, 5);
        assert_eq!(
            dbg.run_to_breakpoint().expect("third run"),
            StopReason::Halted
        );
    }

    #[test]
    fn a_breakpoint_at_the_current_pc_does_not_stop_execution_immediately() {
        let module = nop_module(4);
        let mut dbg = Debugger::new(&module).expect("debugger should start");
        set_breakpoint(&mut dbg.breakpoints, 0);
        assert_eq!(dbg.vm.pc, 0);

        // Otherwise the user could never leave a breakpoint on the current line.
        let reason = dbg.run_to_breakpoint().expect("run past pc 0");
        assert_eq!(reason, StopReason::Halted);
        assert!(dbg.vm.halted);
    }

    #[test]
    fn a_deleted_breakpoint_no_longer_halts_a_continue() {
        let module = nop_module(4);
        let mut dbg = Debugger::new(&module).expect("debugger should start");
        set_breakpoint(&mut dbg.breakpoints, 2);
        clear_breakpoint(&mut dbg.breakpoints, 2);

        assert_eq!(
            dbg.run_to_breakpoint().expect("run to the exit"),
            StopReason::Halted
        );
    }

    #[test]
    fn break_then_continue_through_the_command_path() {
        let module = nop_module(6);
        let mut dbg = Debugger::new(&module).expect("debugger should start");

        dbg.execute_command(parse_command("break 4").expect("break parses"))
            .expect("set a breakpoint");
        assert!(should_stop(&dbg.breakpoints, 4));

        dbg.execute_command(parse_command("continue").expect("continue parses"))
            .expect("run to the breakpoint");
        assert_eq!(dbg.vm.pc, 4, "the command path stops at the breakpoint too");

        dbg.execute_command(parse_command("delete 4").expect("delete parses"))
            .expect("clear the breakpoint");
        assert!(dbg.breakpoints.is_empty());

        dbg.execute_command(parse_command("c").expect("c parses"))
            .expect("run to the exit");
        assert!(dbg.vm.halted);
    }

    #[test]
    fn quit_ends_the_command_loop_and_other_commands_do_not() {
        let module = nop_module(2);
        let mut dbg = Debugger::new(&module).expect("debugger should start");

        assert_eq!(
            dbg.execute_command(Command::Quit).expect("quit"),
            Flow::Quit
        );
        for command in [
            Command::Help,
            Command::List,
            Command::Backtrace,
            Command::ListBreakpoints,
            Command::Info(InfoTarget::Regs),
            print_cmd(None, None),
        ] {
            assert_eq!(
                dbg.execute_command(command.clone())
                    .expect("command should run"),
                Flow::Continue,
                "command: {command:?}"
            );
        }
    }

    // --- Reported state ---

    #[test]
    fn info_regs_reports_the_live_pc_frame_and_stack() {
        let module = nop_module(4);
        let mut dbg = Debugger::new(&module).expect("debugger should start");
        dbg.step().expect("step");
        dbg.step().expect("step");

        let expected_fp = FRAME_HEADER_SIZE;
        let expected_bytes = dbg.vm.frames.data.len();
        assert_eq!(
            dbg.info_values(InfoTarget::Regs),
            vec![
                InfoValue::Pc(2),
                InfoValue::Fp(expected_fp),
                InfoValue::FrameStackBytes(expected_bytes),
                InfoValue::CurrentFrameBase(expected_fp),
            ]
        );
        assert_eq!(
            expected_bytes,
            FRAME_HEADER_SIZE + 64,
            "the entry frame is the header plus the 64 byte entry type"
        );
    }

    #[test]
    fn info_frame_reports_the_frame_stack_size() {
        let module = nop_module(1);
        let dbg = Debugger::new(&module).expect("debugger should start");
        assert_eq!(
            dbg.info_values(InfoTarget::Frame),
            vec![
                InfoValue::FrameStackBytes(dbg.vm.frames.data.len()),
                InfoValue::CurrentFrameBase(dbg.vm.frames.current_data_offset()),
            ]
        );
    }

    #[test]
    fn info_heap_follows_the_number_of_heap_objects() {
        let module = nop_module(1);
        let mut dbg = Debugger::new(&module).expect("debugger should start");
        let before = dbg.vm.heap.len();
        assert_eq!(
            dbg.info_values(InfoTarget::Heap),
            vec![InfoValue::HeapObjects(before)]
        );

        dbg.vm.heap.alloc(0, HeapData::Record(vec![0u8; 8]));
        assert_eq!(
            dbg.info_values(InfoTarget::Heap),
            vec![InfoValue::HeapObjects(before + 1)],
            "a new allocation must show up in the reported count"
        );
    }

    #[test]
    fn info_mp_reports_the_module_data_size() {
        let module = nop_module(1);
        let mut dbg = Debugger::new(&module).expect("debugger should start");
        assert_eq!(
            dbg.info_values(InfoTarget::Mp),
            vec![InfoValue::MpBytes(0)],
            "the test module has no module data"
        );

        dbg.vm.mp.resize(32, 0);
        assert_eq!(
            dbg.info_values(InfoTarget::Mp),
            vec![InfoValue::MpBytes(32)]
        );
    }

    #[test]
    fn info_break_reports_the_breakpoints_that_are_set() {
        let module = nop_module(4);
        let mut dbg = Debugger::new(&module).expect("debugger should start");
        assert_eq!(
            dbg.info_values(InfoTarget::Breakpoints),
            vec![InfoValue::NoBreakpoints]
        );

        set_breakpoint(&mut dbg.breakpoints, 3);
        set_breakpoint(&mut dbg.breakpoints, 1);
        assert_eq!(
            dbg.info_values(InfoTarget::Breakpoints),
            vec![InfoValue::Breakpoint(1), InfoValue::Breakpoint(3)]
        );
    }

    #[test]
    fn print_pc_and_fp_report_the_current_position() {
        let module = nop_module(3);
        let mut dbg = Debugger::new(&module).expect("debugger should start");
        dbg.step().expect("step");
        assert_eq!(dbg.print_values_for("pc", None), vec![InfoValue::Pc(1)]);
        assert_eq!(
            dbg.print_values_for("fp", None),
            vec![InfoValue::Fp(FRAME_HEADER_SIZE)]
        );
    }

    #[test]
    fn print_word_reports_the_value_stored_in_the_frame() {
        let module = nop_module(1);
        let mut dbg = Debugger::new(&module).expect("debugger should start");
        let fp = dbg.vm.frames.current_data_offset();
        memory::write_word(&mut dbg.vm.frames.data, fp + 8, 0x1234_5678);

        assert_eq!(
            dbg.print_values_for("word", Some("8")),
            vec![InfoValue::Word {
                offset: 8,
                value: 0x1234_5678
            }]
        );
    }

    #[test]
    fn print_word_rejects_an_offset_outside_the_frame_stack() {
        let module = nop_module(1);
        let dbg = Debugger::new(&module).expect("debugger should start");
        assert_eq!(
            dbg.print_values_for("word", Some("100000")),
            vec![InfoValue::OffsetOutOfBounds]
        );
    }

    #[test]
    fn print_word_rejects_an_offset_that_would_overflow_an_address() {
        let module = nop_module(1);
        let dbg = Debugger::new(&module).expect("debugger should start");
        // The offset is typed by the user, so adding it to the frame pointer must
        // not overflow.
        assert_eq!(
            dbg.print_values_for("word", Some(&usize::MAX.to_string())),
            vec![InfoValue::OffsetOutOfBounds]
        );
    }

    #[test]
    fn print_word_without_an_offset_shows_its_usage() {
        let module = nop_module(1);
        let dbg = Debugger::new(&module).expect("debugger should start");
        assert_eq!(
            dbg.print_values_for("word", None),
            vec![InfoValue::Usage("print word <offset>")]
        );
    }

    #[test]
    fn print_rejects_an_unknown_target() {
        let module = nop_module(1);
        let dbg = Debugger::new(&module).expect("debugger should start");
        assert_eq!(
            dbg.print_values_for("registers", None),
            vec![InfoValue::UnknownTarget("registers".to_string())]
        );
    }

    #[test]
    fn print_inst_reports_the_instruction_at_the_current_pc() {
        let module = test_module(vec![nop(), exit()]);
        let mut dbg = Debugger::new(&module).expect("debugger should start");
        let values = dbg.print_values_for("inst", None);
        match values.as_slice() {
            [InfoValue::Instruction(text)] => {
                assert!(text.contains("Nop"), "pc 0 holds the no-op: {text}");
            }
            other => panic!("expected one instruction, got {other:?}"),
        }

        dbg.step().expect("step");
        let values = dbg.print_values_for("inst", None);
        match values.as_slice() {
            [InfoValue::Instruction(text)] => {
                assert!(text.contains("Exit"), "pc 1 holds the exit: {text}");
            }
            other => panic!("expected one instruction, got {other:?}"),
        }
    }

    #[test]
    fn print_inst_past_the_end_of_the_code_reports_nothing() {
        let module = nop_module(1);
        let mut dbg = Debugger::new(&module).expect("debugger should start");
        dbg.vm.pc = module.code.len();
        assert!(dbg.print_values_for("inst", None).is_empty());
    }

    // --- The list window ---

    #[test]
    fn the_list_window_starts_five_instructions_before_the_pc() {
        assert_eq!(list_window(20, 100), (15, 30));
    }

    #[test]
    fn the_list_window_clamps_to_the_ends_of_the_code() {
        assert_eq!(list_window(0, 100), (0, 10), "no lines before the first pc");
        assert_eq!(list_window(2, 100), (0, 12));
        assert_eq!(
            list_window(98, 100),
            (93, 100),
            "clamped to the code length"
        );
        assert_eq!(list_window(0, 0), (0, 0), "an empty module lists nothing");
    }

    // --- Backtrace ---

    /// Push a frame whose header records `saved_pc` as the caller's pc.
    fn push_frame(dbg: &mut Debugger<'_>, saved_pc: Pc) {
        let pending = dbg
            .vm
            .frames
            .alloc_pending(16)
            .expect("frame should be allocated");
        dbg.vm
            .frames
            .activate_pending(pending, saved_pc)
            .expect("frame should be activated");
    }

    #[test]
    fn a_backtrace_of_the_entry_frame_reaches_the_bottom_of_the_stack() {
        let module = nop_module(1);
        let dbg = Debugger::new(&module).expect("debugger should start");
        assert_eq!(
            walk_backtrace(&dbg.vm.frames.data, dbg.vm.frames.current_base),
            vec![BacktraceEntry::Entry]
        );
    }

    #[test]
    fn a_backtrace_reports_the_saved_pc_of_every_caller() {
        let module = nop_module(9);
        let mut dbg = Debugger::new(&module).expect("debugger should start");
        // The entry frame calls at pc 3, and that frame calls at pc 7.
        push_frame(&mut dbg, 3);
        push_frame(&mut dbg, 7);

        assert_eq!(
            walk_backtrace(&dbg.vm.frames.data, dbg.vm.frames.current_base),
            vec![BacktraceEntry::Caller(7), BacktraceEntry::Caller(3)],
            "the walk must follow saved frame bases, not data area offsets"
        );
    }

    #[test]
    fn a_backtrace_stops_when_the_frame_chain_leaves_the_stack() {
        assert!(
            walk_backtrace(&[0u8; 4], 0).is_empty(),
            "a header that does not fit in the buffer ends the walk"
        );
        assert!(
            walk_backtrace(&[0u8; 64], 60).is_empty(),
            "a base near the top of the buffer ends the walk"
        );
    }

    #[test]
    fn a_backtrace_stops_when_a_saved_base_does_not_move_down_the_stack() {
        // A frame whose caller sits at or above it means the chain is corrupt.
        let mut data = vec![0u8; 64];
        memory::write_word(&mut data, 32, 5);
        memory::write_word(&mut data, 36, 32);
        assert_eq!(
            walk_backtrace(&data, 32),
            vec![BacktraceEntry::Caller(5)],
            "the walk reports the frame and then stops"
        );
    }

    #[test]
    fn a_backtrace_truncates_a_frame_chain_that_is_too_long() {
        // Build one header every 8 bytes, each pointing at the one below it.
        let frames = MAX_BACKTRACE_DEPTH + 5;
        let mut data = vec![0u8; (frames + 1) * 8];
        for index in 1..=frames {
            let base = index * 8;
            memory::write_word(&mut data, base, index as Word);
            memory::write_word(&mut data, base + 4, (base - 8) as Word);
        }
        let entries = walk_backtrace(&data, frames * 8);
        assert_eq!(entries.len(), MAX_BACKTRACE_DEPTH + 1);
        assert_eq!(entries.last(), Some(&BacktraceEntry::Truncated));
        assert_eq!(entries[0], BacktraceEntry::Caller(frames as Pc));
    }

    // --- Operand formatting ---

    #[test]
    fn operands_are_formatted_in_assembler_notation() {
        let cases = [
            (AddressMode::OffsetIndirectFp, 8, 0, "8(fp)"),
            (AddressMode::OffsetIndirectMp, 4, 0, "4(mp)"),
            (AddressMode::OffsetDoubleIndirectFp, 8, 12, "12(8(fp))"),
            (AddressMode::OffsetDoubleIndirectMp, 8, 12, "12(8(mp))"),
            (AddressMode::Immediate, 42, 0, "$42"),
            (AddressMode::None, 0, 0, ""),
        ];
        for (mode, register1, register2, expected) in cases {
            let op = Operand {
                mode,
                register1,
                register2,
            };
            assert_eq!(format_operand(&op), expected, "mode: {mode:?}");
        }
    }

    #[test]
    fn middle_operands_are_formatted_in_assembler_notation() {
        let cases = [
            (MiddleMode::None, 0, ""),
            (MiddleMode::SmallImmediate, 7, "$7"),
            (MiddleMode::SmallOffsetFp, 20, "20(fp)"),
            (MiddleMode::SmallOffsetMp, 24, "24(mp)"),
        ];
        for (mode, register1, expected) in cases {
            let op = MiddleOperand { mode, register1 };
            assert_eq!(format_mid(&op), expected, "mode: {mode:?}");
        }
    }

    #[test]
    fn an_instruction_with_no_operands_formats_as_an_empty_string() {
        assert_eq!(format_instruction_operands(&nop()), "");
    }

    #[test]
    fn an_instruction_formats_its_source_middle_and_destination() {
        let inst = Instruction {
            opcode: Opcode::Addw,
            source: Operand {
                mode: AddressMode::OffsetIndirectFp,
                register1: 8,
                register2: 0,
            },
            middle: MiddleOperand {
                mode: MiddleMode::SmallOffsetFp,
                register1: 12,
            },
            destination: Operand {
                mode: AddressMode::Immediate,
                register1: 3,
                register2: 0,
            },
        };
        assert_eq!(
            format_instruction_operands(&inst),
            "src=8(fp) mid=12(fp) dst=$3"
        );
    }
}
