//! Dis VM Execution Engine

/// Execute a loaded program
pub fn execute() -> anyhow::Result<()> {
    tracing::info!("Executing program");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_executes() {
        assert!(execute().is_ok());
    }
}
