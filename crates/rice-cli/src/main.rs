use std::fs;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

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
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Command::Run { path } => {
            let bytes = fs::read(&path)?;
            let module = ricevm_loader::load(&bytes)?;
            tracing::info!(name = %module.name, "Module loaded");
            ricevm_execute::execute(&module)?;
        }
    }

    Ok(())
}
