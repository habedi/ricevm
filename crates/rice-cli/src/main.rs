use clap::Parser;
use dis_core;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Optional name to operate on
    name: Option<String>,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    tracing::info!("Starting Dis VM CLI");
    dis_core::init();

    if let Some(name) = cli.name {
        println!("Hello, {}!", name);
        // Simulate loading/execution flow
        let dummy_program = [0u8; 4];
        dis_loader::load(&dummy_program)?;
        dis_execute::execute()?;
    } else {
        println!("Hello, Dis VM!");
    }

    Ok(())
}
