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
    #[allow(dead_code)]
    pub fn get_module(&self, id: u32) -> Option<&BuiltinModule> {
        self.builtins.get(id as usize)
    }

    /// Get a function from a built-in module.
    pub fn get_func(&self, module_id: u32, func_idx: u32) -> Option<&BuiltinFunc> {
        self.builtins
            .get(module_id as usize)?
            .funcs
            .get(func_idx as usize)
    }
}
