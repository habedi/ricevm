//! Include file processing.
//!
//! Reads `.m` module interface files, parses them, and populates
//! the symbol table with type and function declarations.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ast::*;
use crate::codegen::{ConstVal, IotaCounter, fold_const_binary};
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::symtab::*;

/// Process all include directives in a source file.
pub fn process_includes(file: &SourceFile, symtab: &mut SymbolTable) {
    for inc in &file.includes {
        process_include(&inc.path, symtab);
    }

    // Also process top-level module declarations from the source file itself
    for decl in &file.decls {
        if let Decl::Module(md) = decl {
            let members = extract_module_members(&md.members);
            symtab.register_module(&md.name, members.clone());
            symtab.define(
                &md.name,
                Symbol::Module {
                    name: md.name.clone(),
                    members,
                },
            );
        }
        if let Decl::Var(v) = decl {
            for name in &v.names {
                let ty = resolve_ast_type(v.ty.as_ref(), symtab);
                symtab.define(name, Symbol::Var { ty });
            }
        }
        // `Decl::Import` is deliberately *not* handled here. Resolving it needs
        // the module-variable-to-module-type mapping and has to report an
        // unknown member as an error, so codegen owns it (`collect_imports`).
    }
}

fn process_include(path: &str, symtab: &mut SymbolTable) {
    let file_path = find_include_file(path, &symtab.include_paths);
    let Some(file_path) = file_path else { return };

    let src = match std::fs::read_to_string(&file_path) {
        Ok(s) => s,
        Err(_) => return,
    };

    let tokens = match Lexer::new(&src, path).tokenize() {
        Ok(t) => t,
        Err(_) => return,
    };

    let parsed = match Parser::new(tokens, path).parse_file() {
        Ok(f) => f,
        Err(_) => return,
    };

    // Extract module declarations from the .m file
    for decl in &parsed.decls {
        match decl {
            Decl::Module(md) => {
                let members = extract_module_members(&md.members);
                symtab.register_module(&md.name, members.clone());
                symtab.define(
                    &md.name,
                    Symbol::Module {
                        name: md.name.clone(),
                        members,
                    },
                );
            }
            Decl::Var(v) => {
                for name in &v.names {
                    let ty = resolve_ast_type(v.ty.as_ref(), symtab);
                    symtab.define(name, Symbol::Var { ty });
                }
            }
            Decl::Const(c) => {
                if let Some(value) = eval_const_expr(&c.value, 0, &HashMap::new()) {
                    symtab.define(
                        &c.name,
                        Symbol::Const {
                            ty: ResolvedType::Int,
                            value,
                        },
                    );
                }
            }
            _ => {}
        }
    }
}

fn find_include_file(path: &str, search_paths: &[String]) -> Option<PathBuf> {
    // Try relative to search paths
    for dir in search_paths {
        let full = Path::new(dir).join(path);
        if full.exists() {
            return Some(full);
        }
    }
    // Try relative to current directory
    let p = Path::new(path);
    if p.exists() {
        return Some(p.to_path_buf());
    }
    None
}

fn extract_module_members(members: &[ModuleMember]) -> HashMap<String, Symbol> {
    let mut map = HashMap::new();
    // `iota` counts up within one `con` declaration and restarts at the next,
    // so `Next, Down, Skip, Quit: con iota;` yields 0,1,2,3 — not four zeroes.
    // Earlier constants of the same interface are visible to later ones.
    let mut iota = IotaCounter::default();
    let mut prior: HashMap<String, ConstValue> = HashMap::new();
    for member in members {
        match member {
            ModuleMember::Const(c) => {
                // A constant this evaluator cannot fold is left out of the
                // interface entirely: a use site then reports the name as
                // undefined instead of silently reading zero.
                let Some(value) = eval_const_expr(&c.value, iota.next(c.span), &prior) else {
                    continue;
                };
                prior.insert(c.name.clone(), value.clone());
                map.insert(
                    c.name.clone(),
                    Symbol::Const {
                        ty: match &value {
                            ConstValue::Real(_) => ResolvedType::Real,
                            ConstValue::String(_) => ResolvedType::String,
                            ConstValue::Int(_) => ResolvedType::Int,
                        },
                        value,
                    },
                );
            }
            ModuleMember::Func(sig) => {
                let params: Vec<(String, ResolvedType)> = sig
                    .params
                    .iter()
                    .flat_map(|p| p.names.iter().map(|n| (n.clone(), resolve_param_type(p))))
                    .collect();
                let ret = sig
                    .ret
                    .as_ref()
                    .map(|t| Box::new(resolve_ast_type_simple(t)));
                map.insert(
                    sig.name.clone(),
                    Symbol::Func {
                        ty: FnType { params, ret },
                    },
                );
            }
            ModuleMember::Var(v) => {
                let ty =
                    resolve_ast_type_simple(v.ty.as_ref().unwrap_or(&Type::Basic(BasicType::Int)));
                for name in &v.names {
                    map.insert(name.clone(), Symbol::Var { ty: ty.clone() });
                }
            }
            ModuleMember::TypeAlias(ta) => {
                let resolved = resolve_ast_type_simple(&ta.ty);
                map.insert(ta.name.clone(), Symbol::Type { resolved });
            }
            ModuleMember::Adt(adt) => {
                map.insert(
                    adt.name.clone(),
                    Symbol::Type {
                        resolved: ResolvedType::Adt(adt.name.clone()),
                    },
                );
            }
            ModuleMember::Exception(exc) => {
                map.insert(
                    exc.name.clone(),
                    Symbol::Const {
                        ty: ResolvedType::String,
                        value: ConstValue::String(exc.name.clone()),
                    },
                );
            }
        }
    }
    map
}

fn resolve_param_type(param: &Param) -> ResolvedType {
    resolve_ast_type_simple(&param.ty)
}

/// Resolve an AST type to a ResolvedType (simple version, no symtab lookup).
fn resolve_ast_type_simple(ty: &Type) -> ResolvedType {
    match ty {
        Type::Basic(BasicType::Int) => ResolvedType::Int,
        Type::Basic(BasicType::Byte) => ResolvedType::Byte,
        Type::Basic(BasicType::Big) => ResolvedType::Big,
        Type::Basic(BasicType::Real) => ResolvedType::Real,
        Type::Basic(BasicType::String) => ResolvedType::String,
        Type::Array(elem) => ResolvedType::Array(Box::new(resolve_ast_type_simple(elem))),
        Type::List(elem) => ResolvedType::List(Box::new(resolve_ast_type_simple(elem))),
        Type::Chan(elem) => ResolvedType::Chan(Box::new(resolve_ast_type_simple(elem))),
        Type::Ref(inner) => ResolvedType::Ref(Box::new(resolve_ast_type_simple(inner))),
        Type::Tuple(types) => {
            ResolvedType::Tuple(types.iter().map(resolve_ast_type_simple).collect())
        }
        Type::Named(qn) => {
            if let Some(qualifier) = &qn.qualifier {
                ResolvedType::Adt(format!("{qualifier}.{}", qn.name))
            } else {
                ResolvedType::Adt(qn.name.clone())
            }
        }
        Type::Func(sig) => {
            let params: Vec<(String, ResolvedType)> = sig
                .params
                .iter()
                .flat_map(|p| p.names.iter().map(|n| (n.clone(), resolve_param_type(p))))
                .collect();
            let ret = sig
                .ret
                .as_ref()
                .map(|t| Box::new(resolve_ast_type_simple(t)));
            ResolvedType::Fn(FnType { params, ret })
        }
        Type::Module(_) => ResolvedType::Module("module".to_string()),
        _ => ResolvedType::Unknown,
    }
}

/// Resolve an AST type with symtab lookup.
fn resolve_ast_type(ty: Option<&Type>, symtab: &SymbolTable) -> ResolvedType {
    match ty {
        Some(t) => {
            // Try to resolve named types via symtab
            if let Type::Named(qn) = t
                && qn.qualifier.is_none()
            {
                if let Some(Symbol::Type { resolved }) = symtab.lookup(&qn.name) {
                    return resolved.clone();
                }
                if let Some(Symbol::Module { .. }) = symtab.lookup(&qn.name) {
                    return ResolvedType::Module(qn.name.clone());
                }
            }
            resolve_ast_type_simple(t)
        }
        None => ResolvedType::Unknown,
    }
}

/// Evaluate a module-interface constant expression.
///
/// `iota` supplies the value of the `iota` keyword for the declaration being
/// folded and `prior` holds the constants already declared in the same
/// interface, which a later one may refer to. Returns `None` when the
/// expression is not a compile-time constant — the caller then leaves the name
/// out of the interface rather than inventing a value for it.
fn eval_const_expr(
    expr: &Expr,
    iota: i64,
    prior: &HashMap<String, ConstValue>,
) -> Option<ConstValue> {
    let folded = fold(expr, iota, prior)?;
    Some(ConstValue::from(&folded))
}

fn fold(expr: &Expr, iota: i64, prior: &HashMap<String, ConstValue>) -> Option<ConstVal> {
    match expr {
        Expr::IntLit(v, _) => Some(ConstVal::Int(*v)),
        Expr::CharLit(v, _) => Some(ConstVal::Int(*v as i64)),
        Expr::RealLit(v, _) => Some(ConstVal::Real(*v)),
        Expr::StringLit(s, _) => Some(ConstVal::Str(s.clone())),
        Expr::Ident(name, _) if name == "iota" => Some(ConstVal::Int(iota)),
        Expr::Ident(name, _) => prior.get(name).map(ConstVal::from),
        Expr::Unary(op, inner, _) => match (op, fold(inner, iota, prior)?) {
            (UnaryOp::Neg, ConstVal::Int(v)) => Some(ConstVal::Int(-v)),
            (UnaryOp::Neg, ConstVal::Real(v)) => Some(ConstVal::Real(-v)),
            (UnaryOp::BitNot, ConstVal::Int(v)) => Some(ConstVal::Int(!v)),
            (UnaryOp::Not, ConstVal::Int(v)) => Some(ConstVal::Int((v == 0) as i64)),
            _ => None,
        },
        Expr::Binary(lhs, op, rhs, _) => {
            let l = fold(lhs, iota, prior)?;
            let r = fold(rhs, iota, prior)?;
            fold_const_binary(&l, *op, &r).ok()
        }
        Expr::Cast(ty, inner, _) => match (ty.as_ref(), fold(inner, iota, prior)?) {
            (Type::Basic(BasicType::Real), ConstVal::Int(v)) => Some(ConstVal::Real(v as f64)),
            (Type::Basic(BasicType::Int | BasicType::Big | BasicType::Byte), ConstVal::Real(v)) => {
                Some(ConstVal::Int(v as i64))
            }
            (Type::Basic(BasicType::String), ConstVal::Int(v)) => Some(ConstVal::Str(v.to_string())),
            (_, v) => Some(v),
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::Span;

    #[test]
    fn eval_const_int() {
        let expr = Expr::IntLit(42, Span::default());
        assert!(matches!(
            eval_const_expr(&expr, 0, &HashMap::new()),
            Some(ConstValue::Int(42))
        ));
    }

    #[test]
    fn eval_const_add() {
        let expr = Expr::Binary(
            Box::new(Expr::IntLit(10, Span::default())),
            BinOp::Add,
            Box::new(Expr::IntLit(32, Span::default())),
            Span::default(),
        );
        assert!(matches!(
            eval_const_expr(&expr, 0, &HashMap::new()),
            Some(ConstValue::Int(42))
        ));
    }

    #[test]
    fn eval_const_shift() {
        let expr = Expr::Binary(
            Box::new(Expr::IntLit(1, Span::default())),
            BinOp::Lshift,
            Box::new(Expr::IntLit(8, Span::default())),
            Span::default(),
        );
        assert!(matches!(
            eval_const_expr(&expr, 0, &HashMap::new()),
            Some(ConstValue::Int(256))
        ));
    }

    #[test]
    fn eval_const_neg() {
        let expr = Expr::Unary(
            UnaryOp::Neg,
            Box::new(Expr::IntLit(7, Span::default())),
            Span::default(),
        );
        assert!(matches!(
            eval_const_expr(&expr, 0, &HashMap::new()),
            Some(ConstValue::Int(-7))
        ));
    }

    #[test]
    fn eval_const_string() {
        let expr = Expr::StringLit("hello".to_string(), Span::default());
        assert!(
            matches!(eval_const_expr(&expr, 0, &HashMap::new()), Some(ConstValue::String(s)) if s == "hello")
        );
    }

    #[test]
    fn process_includes_extracts_module_decl() {
        let src = r#"implement Test;
Test: module {
    PATH: con "$Test";
    init: fn(nil: ref Draw->Context, nil: list of string);
};
"#;
        let tokens = Lexer::new(src, "<test>")
            .tokenize()
            .ok()
            .unwrap_or_default();
        let file = crate::parser::Parser::new(tokens, "<test>")
            .parse_file()
            .ok()
            .unwrap_or_else(|| SourceFile {
                implement: vec![],
                includes: vec![],
                decls: vec![],
            });
        let mut symtab = SymbolTable::new();
        process_includes(&file, &mut symtab);
        assert!(symtab.lookup("Test").is_some());
    }
}
