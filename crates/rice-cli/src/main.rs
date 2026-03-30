use std::fs;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use ricevm_core::{AddressMode, MiddleMode, Module};

#[derive(Parser)]
#[command(name = "ricevm", version, about = "Dis virtual machine")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Execute a .dis module file
    Run {
        /// Path to the .dis module file
        path: PathBuf,
        /// Module search paths (repeatable)
        #[arg(long = "probe", short = 'p')]
        probe_paths: Vec<PathBuf>,
        /// Enable instruction tracing
        #[arg(long)]
        trace: bool,
        /// Disable mark-and-sweep garbage collection
        #[arg(long = "no-gc")]
        no_gc: bool,
        /// Thread pool size for the scheduler
        #[arg(long, default_value = "1")]
        threads: usize,
    },
    /// Disassemble a .dis module file
    Dis {
        /// Path to the .dis module file
        path: PathBuf,
    },
    /// Debug a .dis module file interactively
    Debug {
        /// Path to the .dis module file
        path: PathBuf,
        /// Module search paths (repeatable)
        #[arg(long = "probe", short = 'p')]
        probe_paths: Vec<PathBuf>,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Command::Run {
            path,
            probe_paths,
            trace,
            no_gc,
            threads,
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
                if !probe_paths.is_empty() {
                    let paths: Vec<String> = probe_paths
                        .iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect();
                    std::env::set_var("RICEVM_PROBE", paths.join(":"));
                }
            }
            let bytes = fs::read(&path)?;
            let module = ricevm_loader::load(&bytes)?;
            tracing::info!(name = %module.name, "Module loaded");
            ricevm_execute::execute(&module)?;
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
            let bytes = fs::read(&path)?;
            let module = ricevm_loader::load(&bytes)?;
            tracing::info!(name = %module.name, "Module loaded for debugging");
            ricevm_execute::debug(&module)?;
        }
        Command::Dis { path } => {
            let bytes = fs::read(&path)?;
            let module = ricevm_loader::load(&bytes)?;
            disassemble(&module);
        }
    }

    Ok(())
}

fn disassemble(module: &Module) {
    println!("Module: {}", module.name);
    println!(
        "  magic={:#x} flags={:#x} stack_extent={} entry_pc={} entry_type={}",
        module.header.magic,
        module.header.runtime_flags.0,
        module.header.stack_extent,
        module.header.entry_pc,
        module.header.entry_type
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
        println!("Types:");
        for td in &module.types {
            println!(
                "  [{}] size={} ptrmap={} bytes",
                td.id,
                td.size,
                td.pointer_map.bytes.len()
            );
        }
        println!();
    }

    // Code
    println!("Code:");
    for (i, inst) in module.code.iter().enumerate() {
        let mut line = format!("  {:4}: {:?}", i, inst.opcode);
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
        println!("Exports:");
        for e in &module.exports {
            println!(
                "  {} pc={} frame_type={} sig={:#x}",
                e.name, e.pc, e.frame_type, e.signature
            );
        }
        println!();
    }

    // Imports
    for (i, imp) in module.imports.iter().enumerate() {
        println!("Import module [{}]:", i);
        for f in &imp.functions {
            println!("  {} sig={:#x}", f.name, f.signature);
        }
        println!();
    }

    // Handlers
    if !module.handlers.is_empty() {
        println!("Handlers:");
        for h in &module.handlers {
            println!(
                "  pc=[{}..{}) exc_offset={} type={:?}",
                h.begin_pc, h.end_pc, h.exception_offset, h.type_descriptor
            );
            for c in &h.cases {
                match &c.name {
                    Some(name) => println!("    \"{}\" -> pc={}", name, c.pc),
                    None => println!("    * -> pc={}", c.pc),
                }
            }
        }
        println!();
    }

    // Data
    if !module.data.is_empty() {
        println!("Data:");
        for item in &module.data {
            match item {
                ricevm_core::DataItem::Bytes { offset, values } => {
                    println!("  @{offset}: bytes[{}]", values.len());
                }
                ricevm_core::DataItem::Words { offset, values } => {
                    println!("  @{offset}: words{values:?}");
                }
                ricevm_core::DataItem::Bigs { offset, values } => {
                    println!("  @{offset}: bigs{values:?}");
                }
                ricevm_core::DataItem::Reals { offset, values } => {
                    println!("  @{offset}: reals{values:?}");
                }
                ricevm_core::DataItem::String { offset, value } => {
                    println!("  @{offset}: \"{value}\"");
                }
                ricevm_core::DataItem::Array {
                    offset,
                    element_type,
                    length,
                } => {
                    println!("  @{offset}: array[{length}] of type {element_type}");
                }
                ricevm_core::DataItem::SetArray { offset, index } => {
                    println!("  @{offset}: set_array[{index}]");
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
