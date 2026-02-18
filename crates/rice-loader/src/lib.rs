//! Dis VM Binary Loader

/// Load a program from a byte slice
pub fn load(data: &[u8]) -> anyhow::Result<()> {
    tracing::info!("Loading program of size: {} bytes", data.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_loads() {
        let data = [0u8; 4];
        assert!(load(&data).is_ok());
    }
}
