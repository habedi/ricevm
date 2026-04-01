use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use clap::{Parser, Subcommand};
use colored::Colorize;
use ricevm_core::{AddressMode, MiddleMode, Module};

#[derive(Parser)]
#[command(
    name = "ricevm",
    version,
    about = "RiceVM: A Dis virtual machine implementation in Rust"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Execute a compiled .dis module file
    Run {
        /// Path to the .dis module file
        path: PathBuf,
        /// Add a directory to the module search path (can be repeated)
        #[arg(long = "probe", short = 'p')]
        probe_paths: Vec<PathBuf>,
        /// Map Inferno root paths to a host directory
        #[arg(long)]
        root: Option<PathBuf>,
        /// Print each instruction as it executes
        #[arg(long)]
        trace: bool,
        /// Disable the mark-and-sweep garbage collector
        #[arg(long = "no-gc")]
        no_gc: bool,
        /// Thread pool size for the scheduler
        #[arg(long, default_value = "1")]
        threads: usize,
        /// Arguments passed to the guest program
        #[arg(last = true)]
        guest_args: Vec<String>,
    },
    /// Disassemble a .dis module into human-readable output
    Dis {
        /// Path to the .dis module file
        path: PathBuf,
    },
    /// Debug a .dis module file interactively
    Debug {
        /// Path to the .dis module file
        path: PathBuf,
        /// Add a directory to the module search path (can be repeated)
        #[arg(long = "probe", short = 'p')]
        probe_paths: Vec<PathBuf>,
    },
}

fn main() {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    let code = run(cli);
    std::process::exit(code);
}

fn run(cli: Cli) -> i32 {
    match cli.command {
        Command::Run {
            path,
            probe_paths,
            root,
            trace,
            no_gc,
            threads,
            guest_args,
        } => {
            // SAFETY: set_var is unsafe in edition 2024 due to potential data races,
            // but we're single-threaded at this point before spawning any threads.
            unsafe {
                if trace {
                    std::env::set_var("RICEVM_TRACE", "1");
                }
                if no_gc {
                    std::env::set_var("RICEVM_NO_GC", "1");
                }
                if threads > 1 {
                    std::env::set_var("RICEVM_THREADS", threads.to_string());
                }
                if let Some(ref root_dir) = root {
                    std::env::set_var("RICEVM_ROOT", root_dir.to_string_lossy().as_ref());
                }
                if !probe_paths.is_empty() {
                    let paths: Vec<String> = probe_paths
                        .iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect();
                    std::env::set_var("RICEVM_PROBE", paths.join(":"));
                }
            }
            let bytes = match read_file(&path) {
                Ok(b) => b,
                Err(code) => return code,
            };
            let module = match load_module(&bytes) {
                Ok(m) => m,
                Err(code) => return code,
            };
            eprintln!("{} Module loaded: {}", "✓".green(), module.name);
            let start = Instant::now();
            match ricevm_execute::execute_with_args(&module, guest_args) {
                Ok(()) => {
                    let elapsed = start.elapsed();
                    eprintln!(
                        "{} {} in {:.2}s",
                        "✓".green(),
                        module.name,
                        elapsed.as_secs_f64()
                    );
                    0
                }
                Err(e) => {
                    eprintln!("{}: {}", "error".red().bold(), e);
                    1
                }
            }
        }
        Command::Debug { path, probe_paths } => {
            unsafe {
                if !probe_paths.is_empty() {
                    let paths: Vec<String> = probe_paths
                        .iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect();
                    std::env::set_var("RICEVM_PROBE", paths.join(":"));
                }
            }
            let bytes = match read_file(&path) {
                Ok(b) => b,
                Err(code) => return code,
            };
            let module = match load_module(&bytes) {
                Ok(m) => m,
                Err(code) => return code,
            };
            eprintln!(
                "{} Module loaded for debugging: {}",
                "✓".green(),
                module.name
            );
            match ricevm_execute::debug(&module) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("{}: {}", "error".red().bold(), e);
                    1
                }
            }
        }
        Command::Dis { path } => {
            let bytes = match read_file(&path) {
                Ok(b) => b,
                Err(code) => return code,
            };
            let module = match load_module(&bytes) {
                Ok(m) => m,
                Err(code) => return code,
            };
            disassemble(&module);
            0
        }
    }
}

fn read_file(path: &PathBuf) -> Result<Vec<u8>, i32> {
    match fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                eprintln!(
                    "{}: file not found: {}",
                    "error".red().bold(),
                    path.display()
                );
                Err(3)
            } else {
                eprintln!(
                    "{}: cannot read {}: {}",
                    "error".red().bold(),
                    path.display(),
                    e
                );
                Err(1)
            }
        }
    }
}

fn load_module(bytes: &[u8]) -> Result<Module, i32> {
    match ricevm_loader::load(bytes) {
        Ok(m) => Ok(m),
        Err(e) => {
            eprintln!("{}: failed to load module: {}", "error".red().bold(), e);
            Err(2)
        }
    }
}

fn disassemble(module: &Module) {
    println!("{} {}", "Module:".bold(), module.name);
    println!(
        "  magic={} flags={} stack_extent={} entry_pc={} entry_type={}",
        format!("{:#x}", module.header.magic).yellow(),
        format!("{:#x}", module.header.runtime_flags.0).yellow(),
        format!("{}", module.header.stack_extent).yellow(),
        format!("{}", module.header.entry_pc).yellow(),
        format!("{}", module.header.entry_type).yellow()
    );
    println!(
        "  code={} types={} data={} exports={} imports={} handlers={}",
        module.code.len(),
        module.types.len(),
        module.data.len(),
        module.exports.len(),
        module.imports.len(),
        module.handlers.len()
    );
    println!();

    // Type descriptors
    if !module.types.is_empty() {
        println!("{}", "Types:".bold());
        for td in &module.types {
            println!(
                "  [{}] size={} ptrmap={} bytes",
                format!("{}", td.id).yellow(),
                td.size,
                td.pointer_map.bytes.len()
            );
        }
        println!();
    }

    // Code
    println!("{}", "Code:".bold());
    for (i, inst) in module.code.iter().enumerate() {
        let addr = format!("  {:4}:", i).yellow();
        let opcode = format!("{:?}", inst.opcode).cyan().bold();
        let mut line = format!("{addr} {opcode}");
        if inst.source.mode != AddressMode::None {
            line.push_str(&format!(" {}", fmt_op(&inst.source)));
        }
        if inst.middle.mode != MiddleMode::None {
            line.push_str(&format!(", {}", fmt_mid(&inst.middle)));
        }
        if inst.destination.mode != AddressMode::None {
            line.push_str(&format!(", {}", fmt_op(&inst.destination)));
        }
        println!("{line}");
    }
    println!();

    // Exports
    if !module.exports.is_empty() {
        println!("{}", "Exports:".bold());
        for e in &module.exports {
            println!(
                "  {} pc={} frame_type={} sig={}",
                e.name,
                format!("{}", e.pc).yellow(),
                e.frame_type,
                format!("{:#x}", e.signature).yellow()
            );
        }
        println!();
    }

    // Imports
    for (i, imp) in module.imports.iter().enumerate() {
        println!("{}", format!("Import module [{i}]:").bold());
        for f in &imp.functions {
            println!(
                "  {} sig={}",
                f.name,
                format!("{:#x}", f.signature).yellow()
            );
        }
        println!();
    }

    // Handlers
    if !module.handlers.is_empty() {
        println!("{}", "Handlers:".bold());
        for h in &module.handlers {
            println!(
                "  pc=[{}..{}) exc_offset={} type={:?}",
                format!("{}", h.begin_pc).yellow(),
                format!("{}", h.end_pc).yellow(),
                h.exception_offset,
                h.type_descriptor
            );
            for c in &h.cases {
                match &c.name {
                    Some(name) => println!(
                        "    {} -> pc={}",
                        format!("\"{name}\"").green(),
                        format!("{}", c.pc).yellow()
                    ),
                    None => println!("    * -> pc={}", format!("{}", c.pc).yellow()),
                }
            }
        }
        println!();
    }

    // Data
    if !module.data.is_empty() {
        println!("{}", "Data:".bold());
        for item in &module.data {
            match item {
                ricevm_core::DataItem::Bytes { offset, values } => {
                    println!(
                        "  {}: bytes[{}]",
                        format!("@{offset}").yellow(),
                        values.len()
                    );
                }
                ricevm_core::DataItem::Words { offset, values } => {
                    println!("  {}: words{values:?}", format!("@{offset}").yellow());
                }
                ricevm_core::DataItem::Bigs { offset, values } => {
                    println!("  {}: bigs{values:?}", format!("@{offset}").yellow());
                }
                ricevm_core::DataItem::Reals { offset, values } => {
                    println!("  {}: reals{values:?}", format!("@{offset}").yellow());
                }
                ricevm_core::DataItem::String { offset, value } => {
                    println!(
                        "  {}: {}",
                        format!("@{offset}").yellow(),
                        format!("\"{value}\"").green()
                    );
                }
                ricevm_core::DataItem::Array {
                    offset,
                    element_type,
                    length,
                } => {
                    println!(
                        "  {}: array[{length}] of type {element_type}",
                        format!("@{offset}").yellow()
                    );
                }
                ricevm_core::DataItem::SetArray { offset, index } => {
                    println!("  {}: set_array[{index}]", format!("@{offset}").yellow());
                }
                ricevm_core::DataItem::RestoreBase => {
                    println!("  restore_base");
                }
            }
        }
    }
}

fn fmt_op(op: &ricevm_core::Operand) -> String {
    match op.mode {
        AddressMode::OffsetIndirectFp => format!("{}(fp)", op.register1),
        AddressMode::OffsetIndirectMp => format!("{}(mp)", op.register1),
        AddressMode::Immediate => format!("${}", op.register1),
        AddressMode::None => "-".to_string(),
        AddressMode::OffsetDoubleIndirectFp => {
            format!("{}({}(fp))", op.register2, op.register1)
        }
        AddressMode::OffsetDoubleIndirectMp => {
            format!("{}({}(mp))", op.register2, op.register1)
        }
        AddressMode::Reserved1 | AddressMode::Reserved2 => "?".to_string(),
    }
}

fn fmt_mid(op: &ricevm_core::MiddleOperand) -> String {
    match op.mode {
        MiddleMode::None => "-".to_string(),
        MiddleMode::SmallImmediate => format!("${}", op.register1),
        MiddleMode::SmallOffsetFp => format!("{}(fp)", op.register1),
        MiddleMode::SmallOffsetMp => format!("{}(mp)", op.register1),
    }
}
