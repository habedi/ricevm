//! Symbol table for type checking and code generation.
//!
//! Tracks types, constants, variables, and functions from both the
//! current module and included .m files.

use std::collections::HashMap;

/// A resolved type in the Limbo type system.
#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedType {
    Int,
    Byte,
    Big,
    Real,
    String,
    Array(Box<ResolvedType>),
    List(Box<ResolvedType>),
    Chan(Box<ResolvedType>),
    Ref(Box<ResolvedType>),
    Tuple(Vec<ResolvedType>),
    Adt(String),
    Module(String),
    Fn(FnType),
    Nil,
    Unknown,
}

impl ResolvedType {
    /// Is this type a pointer in the Dis VM?
    pub fn is_ptr(&self) -> bool {
        !matches!(
            self,
            ResolvedType::Int
                | ResolvedType::Byte
                | ResolvedType::Big
                | ResolvedType::Real
                | ResolvedType::Unknown
        )
    }

    /// Is this an array type?
    pub fn is_array(&self) -> bool {
        matches!(self, ResolvedType::Array(_))
    }

    /// Size in bytes in a Dis frame.
    pub fn frame_size(&self) -> i32 {
        match self {
            ResolvedType::Big | ResolvedType::Real => 8,
            _ => 4,
        }
    }
}

/// Function type: parameter types and return type.
#[derive(Clone, Debug, PartialEq)]
pub struct FnType {
    pub params: Vec<(String, ResolvedType)>,
    pub ret: Option<Box<ResolvedType>>,
}

/// A constant value.
#[derive(Clone, Debug)]
pub enum ConstValue {
    Int(i64),
    Real(f64),
    String(String),
}

/// Symbol table entry.
#[derive(Clone, Debug)]
pub enum Symbol {
    Var {
        ty: ResolvedType,
    },
    Const {
        ty: ResolvedType,
        value: ConstValue,
    },
    Func {
        ty: FnType,
    },
    Type {
        resolved: ResolvedType,
    },
    Module {
        name: String,
        members: HashMap<String, Symbol>,
    },
}

/// Module-level symbol table.
#[derive(Clone, Debug, Default)]
pub struct SymbolTable {
    /// Symbols in scope: name -> Symbol
    pub symbols: HashMap<String, Symbol>,
    /// Module declarations from included .m files
    pub modules: HashMap<String, HashMap<String, Symbol>>,
    /// Include search paths
    pub include_paths: Vec<String>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_include_path(&mut self, path: &str) {
        self.include_paths.push(path.to_string());
    }

    /// Look up a symbol by name.
    pub fn lookup(&self, name: &str) -> Option<&Symbol> {
        self.symbols.get(name)
    }

    /// Look up a qualified symbol: Module->member or Module.member.
    pub fn lookup_qualified(&self, module: &str, member: &str) -> Option<&Symbol> {
        self.modules.get(module).and_then(|m| m.get(member))
    }

    /// Define a new symbol.
    pub fn define(&mut self, name: &str, sym: Symbol) {
        self.symbols.insert(name.to_string(), sym);
    }

    /// Register a module's members.
    pub fn register_module(&mut self, name: &str, members: HashMap<String, Symbol>) {
        self.modules.insert(name.to_string(), members);
    }

    /// Resolve a type from a module: e.g., "Draw->Context".
    pub fn resolve_module_type(&self, module: &str, type_name: &str) -> ResolvedType {
        if let Some(members) = self.modules.get(module)
            && let Some(Symbol::Type { resolved }) = members.get(type_name)
        {
            return resolved.clone();
        }
        ResolvedType::Adt(format!("{module}.{type_name}"))
    }
}
