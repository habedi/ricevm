//! Limbo programming language compiler for the Dis virtual machine.
//!
//! Compiles Limbo source code (`.b` files) to Dis bytecode (`.dis` files).
//!
//! # Architecture
//!
//! The compiler pipeline has four stages:
//!
//! 1. **Lexer** (`lexer.rs`): source text to tokens
//! 2. **Parser** (`parser.rs`): tokens to AST
//! 3. **Type checker** (`typecheck.rs`): AST validation and type inference
//! 4. **Code generator** (`codegen.rs`): AST to Dis bytecode

pub mod ast;
pub mod codegen;
pub mod includes;
pub mod lexer;
pub mod parser;
pub mod symtab;
pub mod token;
pub mod writer;

use lexer::Lexer;
use parser::Parser;

/// Compilation options.
pub struct CompileOptions {
    /// Search paths for .m include files.
    pub include_paths: Vec<String>,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            include_paths: vec![".".to_string()],
        }
    }
}

/// Compile Limbo source code to a Dis Module.
pub fn compile(src: &str, filename: &str) -> Result<ricevm_core::Module, String> {
    compile_with_options(src, filename, &CompileOptions::default())
}

/// Compile Limbo source code to a Dis Module with options.
pub fn compile_with_options(
    src: &str,
    filename: &str,
    opts: &CompileOptions,
) -> Result<ricevm_core::Module, String> {
    let tokens = Lexer::new(src, filename)
        .tokenize()
        .map_err(|e| format!("{e}"))?;
    let ast = Parser::new(tokens, filename)
        .parse_file()
        .map_err(|e| format!("{e}"))?;

    // Build symbol table from include files
    let mut symtab = symtab::SymbolTable::new();
    for path in &opts.include_paths {
        symtab.add_include_path(path);
    }
    includes::process_includes(&ast, &mut symtab);

    codegen::CodeGen::new().compile(&ast)
}

/// Compile Limbo source code to Dis binary format.
pub fn compile_to_bytes(src: &str, filename: &str) -> Result<Vec<u8>, String> {
    let module = compile(src, filename)?;
    Ok(writer::write_dis(&module))
}

/// Compile Limbo source code to Dis binary format with options.
pub fn compile_to_bytes_with_options(
    src: &str,
    filename: &str,
    opts: &CompileOptions,
) -> Result<Vec<u8>, String> {
    let module = compile_with_options(src, filename, opts)?;
    Ok(writer::write_dis(&module))
}
