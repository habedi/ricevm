//! Integration tests: parse real Inferno Limbo source files.
//!
//! These tests require the `external/inferno-os` git submodule to be checked out.
//! They skip gracefully if the source files are not available.

use ricevm_limbo::lexer::Lexer;
use ricevm_limbo::parser::Parser;

fn parse_file(path: &str) -> Result<ricevm_limbo::ast::SourceFile, String> {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(std::path::Path::new("."));
    let full_path = workspace_root.join(path);
    let src = std::fs::read_to_string(&full_path).map_err(|e| format!("{e}"))?;
    let tokens = Lexer::new(&src, path)
        .tokenize()
        .map_err(|e| format!("{e}"))?;
    Parser::new(tokens, path)
        .parse_file()
        .map_err(|e| format!("{e}"))
}

/// Try to parse a file; skip the test if the submodule is not available.
macro_rules! parse_or_skip {
    ($path:expr) => {
        match parse_file($path) {
            Ok(f) => f,
            Err(e) if e.contains("No such file") => {
                eprintln!("SKIP: {} (submodule not checked out)", $path);
                return;
            }
            Err(e) => panic!("{} should parse: {e}", $path),
        }
    };
}

#[test]
fn parse_echo() {
    let file = parse_or_skip!("external/inferno-os/appl/cmd/echo.b");
    assert_eq!(file.implement, vec!["Echo"]);
    assert!(file.decls.len() >= 3);
}

#[test]
fn parse_cat() {
    let file = parse_or_skip!("external/inferno-os/appl/cmd/cat.b");
    assert_eq!(file.implement, vec!["Cat"]);
}

#[test]
fn parse_date() {
    let file = parse_or_skip!("external/inferno-os/appl/cmd/date.b");
    assert_eq!(file.implement, vec!["Date"]);
}

#[test]
fn parse_mkdir() {
    let file = parse_or_skip!("external/inferno-os/appl/cmd/mkdir.b");
    assert_eq!(file.implement, vec!["Mkdir"]);
}

#[test]
fn parse_rm() {
    let file = parse_or_skip!("external/inferno-os/appl/cmd/rm.b");
    assert_eq!(file.implement, vec!["Rm"]);
}

#[test]
fn parse_sleep() {
    let file = parse_or_skip!("external/inferno-os/appl/cmd/sleep.b");
    assert_eq!(file.implement, vec!["Sleep"]);
}

#[test]
fn parse_basename() {
    let file = parse_or_skip!("external/inferno-os/appl/cmd/basename.b");
    assert_eq!(file.implement, vec!["Basename"]);
}

#[test]
fn parse_wc() {
    let file = parse_or_skip!("external/inferno-os/appl/cmd/wc.b");
    assert_eq!(file.implement, vec!["Wc"]);
}

#[test]
fn parse_tee() {
    let file = parse_or_skip!("external/inferno-os/appl/cmd/tee.b");
    assert_eq!(file.implement, vec!["Tee"]);
}

#[test]
fn parse_tail() {
    let file = parse_or_skip!("external/inferno-os/appl/cmd/tail.b");
    assert_eq!(file.implement, vec!["Tail"]);
}

#[test]
fn compile_hello_world() {
    let src = r#"implement Hello;

include "sys.m";
    sys: Sys;

include "draw.m";

Hello: module
{
    init: fn(nil: ref Draw->Context, nil: list of string);
};

init(nil: ref Draw->Context, nil: list of string)
{
    sys = load Sys Sys->PATH;
    sys->print("hello, world\n");
}
"#;
    let module = ricevm_limbo::compile(src, "hello.b").expect("compile should succeed");
    assert_eq!(module.name, "Hello");
    assert!(!module.code.is_empty(), "should have code");
    assert!(!module.exports.is_empty(), "should have init export");
    assert!(!module.imports.is_empty(), "should have $Sys import");
    assert_eq!(module.exports[0].name, "init");

    let bytes = ricevm_limbo::writer::write_dis(&module);
    assert!(bytes.len() > 20, "binary should be non-trivial");
}

#[test]
fn roundtrip_hello_compiled() {
    let src = r#"implement Hello;
include "sys.m";
    sys: Sys;
include "draw.m";
Hello: module { init: fn(nil: ref Draw->Context, nil: list of string); };
init(nil: ref Draw->Context, nil: list of string)
{
    sys = load Sys Sys->PATH;
    sys->print("hello!\n");
}
"#;
    let bytes = ricevm_limbo::compile_to_bytes(src, "hello.b").expect("compile");
    let loaded = ricevm_loader::load(&bytes);
    match &loaded {
        Ok(m) => {
            assert_eq!(m.name, "Hello");
            assert!(!m.code.is_empty(), "should have code");
        }
        Err(e) => {
            panic!("loader failed: {e}");
        }
    }
}
