//! Core library for Dis VM

/// Initialize the core library
pub fn init() {
    tracing::info!("Dis VM Core Initialized");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        init();
        assert!(true);
    }
}
