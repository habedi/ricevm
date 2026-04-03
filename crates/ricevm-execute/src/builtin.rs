//! Built-in module registry.
//!
//! Built-in modules (like `$Sys`) are registered with native Rust handlers
//! that execute directly instead of interpreting bytecode.

use ricevm_core::ExecError;

use crate::vm::VmState;

/// A native function handler for a built-in module function.
pub(crate) type BuiltinFn = fn(&mut VmState<'_>) -> Result<(), ExecError>;

/// A single function in a built-in module.
#[allow(dead_code)]
pub(crate) struct BuiltinFunc {
    pub name: &'static str,
    pub sig: u32,
    pub frame_size: usize,
    pub handler: BuiltinFn,
}

/// A built-in module with its exported functions.
pub(crate) struct BuiltinModule {
    pub name: &'static str,
    pub funcs: Vec<BuiltinFunc>,
}

/// Registry of built-in modules.
pub(crate) struct ModuleRegistry {
    builtins: Vec<BuiltinModule>,
}

impl ModuleRegistry {
    pub fn new() -> Self {
        Self {
            builtins: Vec::new(),
        }
    }

    /// Register a built-in module.
    pub fn register(&mut self, module: BuiltinModule) {
        self.builtins.push(module);
    }

    /// Find a built-in module by path. Returns the module index.
    pub fn find_builtin(&self, path: &str) -> Option<u32> {
        self.builtins
            .iter()
            .position(|m| m.name == path)
            .map(|i| i as u32)
    }

    /// Get a built-in module by index.
    pub fn get_module(&self, id: u32) -> Option<&BuiltinModule> {
        self.builtins.get(id as usize)
    }

    /// Get a function from a built-in module by position index.
    pub fn get_func(&self, module_id: u32, func_idx: u32) -> Option<&BuiltinFunc> {
        self.builtins
            .get(module_id as usize)?
            .funcs
            .get(func_idx as usize)
    }

    /// Get a function from a built-in module by matching a signature hash.
    #[allow(dead_code)]
    pub fn get_func_by_sig(&self, module_id: u32, sig: u32) -> Option<&BuiltinFunc> {
        self.builtins
            .get(module_id as usize)?
            .funcs
            .iter()
            .find(|f| f.sig == sig)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_handler(_: &mut VmState<'_>) -> Result<(), ExecError> {
        Ok(())
    }

    #[test]
    fn register_and_find_module() {
        let mut reg = ModuleRegistry::new();
        reg.register(BuiltinModule {
            name: "$Test",
            funcs: vec![BuiltinFunc {
                name: "hello",
                sig: 0x1234,
                frame_size: 32,
                handler: dummy_handler,
            }],
        });
        assert_eq!(reg.find_builtin("$Test"), Some(0));
        assert_eq!(reg.find_builtin("$Missing"), None);
    }

    #[test]
    fn get_func_by_index() {
        let mut reg = ModuleRegistry::new();
        reg.register(BuiltinModule {
            name: "$Sys",
            funcs: vec![
                BuiltinFunc {
                    name: "print",
                    sig: 0xAA,
                    frame_size: 40,
                    handler: dummy_handler,
                },
                BuiltinFunc {
                    name: "write",
                    sig: 0xBB,
                    frame_size: 48,
                    handler: dummy_handler,
                },
            ],
        });
        let f = reg.get_func(0, 1);
        assert!(f.is_some());
        assert_eq!(f.map(|f| f.name), Some("write"));
        assert!(reg.get_func(0, 5).is_none());
        assert!(reg.get_func(9, 0).is_none());
    }

    #[test]
    fn get_func_by_signature() {
        let mut reg = ModuleRegistry::new();
        reg.register(BuiltinModule {
            name: "$Sys",
            funcs: vec![
                BuiltinFunc {
                    name: "print",
                    sig: 0xAA,
                    frame_size: 40,
                    handler: dummy_handler,
                },
                BuiltinFunc {
                    name: "write",
                    sig: 0xBB,
                    frame_size: 48,
                    handler: dummy_handler,
                },
            ],
        });
        let f = reg.get_func_by_sig(0, 0xBB);
        assert_eq!(f.map(|f| f.name), Some("write"));
        assert!(reg.get_func_by_sig(0, 0xFF).is_none());
    }

    #[test]
    fn multiple_modules() {
        let mut reg = ModuleRegistry::new();
        reg.register(BuiltinModule {
            name: "$Sys",
            funcs: vec![],
        });
        reg.register(BuiltinModule {
            name: "$Math",
            funcs: vec![],
        });
        reg.register(BuiltinModule {
            name: "$Draw",
            funcs: vec![],
        });
        assert_eq!(reg.find_builtin("$Sys"), Some(0));
        assert_eq!(reg.find_builtin("$Math"), Some(1));
        assert_eq!(reg.find_builtin("$Draw"), Some(2));
    }
}
