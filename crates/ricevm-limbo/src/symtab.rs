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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn define_and_lookup_variable() {
        let mut st = SymbolTable::new();
        st.define(
            "x",
            Symbol::Var {
                ty: ResolvedType::Int,
            },
        );
        let sym = st.lookup("x");
        assert!(sym.is_some());
        match sym.unwrap() {
            Symbol::Var { ty } => assert_eq!(*ty, ResolvedType::Int),
            _ => panic!("expected Var symbol"),
        }
    }

    #[test]
    fn define_and_lookup_const() {
        let mut st = SymbolTable::new();
        st.define(
            "N",
            Symbol::Const {
                ty: ResolvedType::Int,
                value: ConstValue::Int(42),
            },
        );
        let sym = st.lookup("N").unwrap();
        match sym {
            Symbol::Const { ty, value } => {
                assert_eq!(*ty, ResolvedType::Int);
                match value {
                    ConstValue::Int(v) => assert_eq!(*v, 42),
                    _ => panic!("expected Int const"),
                }
            }
            _ => panic!("expected Const symbol"),
        }
    }

    #[test]
    fn lookup_missing_returns_none() {
        let st = SymbolTable::new();
        assert!(st.lookup("nonexistent").is_none());
    }

    #[test]
    fn register_module_and_lookup_members() {
        let mut st = SymbolTable::new();
        let mut members = HashMap::new();
        members.insert(
            "print".to_string(),
            Symbol::Func {
                ty: FnType {
                    params: vec![("s".to_string(), ResolvedType::String)],
                    ret: Some(Box::new(ResolvedType::Int)),
                },
            },
        );
        members.insert(
            "FD".to_string(),
            Symbol::Type {
                resolved: ResolvedType::Adt("Sys.FD".to_string()),
            },
        );
        st.register_module("Sys", members);

        // Lookup qualified member
        let print_sym = st.lookup_qualified("Sys", "print");
        assert!(print_sym.is_some());
        match print_sym.unwrap() {
            Symbol::Func { ty } => {
                assert_eq!(ty.params.len(), 1);
                assert_eq!(ty.params[0].0, "s");
            }
            _ => panic!("expected Func symbol"),
        }

        // Lookup missing member
        assert!(st.lookup_qualified("Sys", "nonexistent").is_none());
        // Lookup missing module
        assert!(st.lookup_qualified("Draw", "Context").is_none());
    }

    #[test]
    fn resolve_module_type_found() {
        let mut st = SymbolTable::new();
        let mut members = HashMap::new();
        members.insert(
            "Context".to_string(),
            Symbol::Type {
                resolved: ResolvedType::Adt("Draw.Context".to_string()),
            },
        );
        st.register_module("Draw", members);

        let ty = st.resolve_module_type("Draw", "Context");
        assert_eq!(ty, ResolvedType::Adt("Draw.Context".to_string()));
    }

    #[test]
    fn resolve_module_type_not_found_returns_adt() {
        let st = SymbolTable::new();
        let ty = st.resolve_module_type("Draw", "Unknown");
        assert_eq!(ty, ResolvedType::Adt("Draw.Unknown".to_string()));
    }

    #[test]
    fn resolved_type_is_ptr() {
        assert!(!ResolvedType::Int.is_ptr());
        assert!(!ResolvedType::Byte.is_ptr());
        assert!(!ResolvedType::Big.is_ptr());
        assert!(!ResolvedType::Real.is_ptr());
        assert!(!ResolvedType::Unknown.is_ptr());
        assert!(ResolvedType::String.is_ptr());
        assert!(ResolvedType::Nil.is_ptr());
        assert!(ResolvedType::Array(Box::new(ResolvedType::Int)).is_ptr());
        assert!(ResolvedType::List(Box::new(ResolvedType::Int)).is_ptr());
        assert!(ResolvedType::Chan(Box::new(ResolvedType::Int)).is_ptr());
        assert!(ResolvedType::Ref(Box::new(ResolvedType::Int)).is_ptr());
        assert!(ResolvedType::Module("Sys".to_string()).is_ptr());
    }

    #[test]
    fn resolved_type_is_array() {
        assert!(ResolvedType::Array(Box::new(ResolvedType::Int)).is_array());
        assert!(!ResolvedType::Int.is_array());
        assert!(!ResolvedType::String.is_array());
    }

    #[test]
    fn resolved_type_frame_size() {
        assert_eq!(ResolvedType::Int.frame_size(), 4);
        assert_eq!(ResolvedType::Byte.frame_size(), 4);
        assert_eq!(ResolvedType::String.frame_size(), 4);
        assert_eq!(ResolvedType::Big.frame_size(), 8);
        assert_eq!(ResolvedType::Real.frame_size(), 8);
    }

    #[test]
    fn add_include_path() {
        let mut st = SymbolTable::new();
        st.add_include_path("/usr/lib/inferno");
        st.add_include_path("/opt/limbo");
        assert_eq!(st.include_paths.len(), 2);
        assert_eq!(st.include_paths[0], "/usr/lib/inferno");
        assert_eq!(st.include_paths[1], "/opt/limbo");
    }
}
