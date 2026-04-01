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
pub mod lexer;
pub mod parser;
pub mod token;
