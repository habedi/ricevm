//! Integration tests: parse real Inferno Limbo source files.

use ricevm_limbo::lexer::Lexer;
use ricevm_limbo::parser::Parser;

fn parse_file(path: &str) -> Result<ricevm_limbo::ast::SourceFile, String> {
    // Resolve relative to workspace root
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

#[test]
fn parse_echo() {
    let file = parse_file("external/inferno-os/appl/cmd/echo.b")
        .expect("echo.b should parse");
    assert_eq!(file.implement, vec!["Echo"]);
    assert!(file.decls.len() >= 3); // sys var, Echo module, init func
}

#[test]
fn parse_cat() {
    let file = parse_file("external/inferno-os/appl/cmd/cat.b")
        .expect("cat.b should parse");
    assert_eq!(file.implement, vec!["Cat"]);
}

#[test]
fn parse_date() {
    let file = parse_file("external/inferno-os/appl/cmd/date.b")
        .expect("date.b should parse");
    assert_eq!(file.implement, vec!["Date"]);
}

#[test]
fn parse_mkdir() {
    let file = parse_file("external/inferno-os/appl/cmd/mkdir.b")
        .expect("mkdir.b should parse");
    assert_eq!(file.implement, vec!["Mkdir"]);
}

#[test]
fn parse_rm() {
    let file = parse_file("external/inferno-os/appl/cmd/rm.b")
        .expect("rm.b should parse");
    assert_eq!(file.implement, vec!["Rm"]);
}

#[test]
fn parse_sleep() {
    let file = parse_file("external/inferno-os/appl/cmd/sleep.b")
        .expect("sleep.b should parse");
    assert_eq!(file.implement, vec!["Sleep"]);
}

#[test]
fn parse_basename() {
    let file = parse_file("external/inferno-os/appl/cmd/basename.b")
        .expect("basename.b should parse");
    assert_eq!(file.implement, vec!["Basename"]);
}

#[test]
fn parse_wc() {
    let file = parse_file("external/inferno-os/appl/cmd/wc.b")
        .expect("wc.b should parse");
    assert_eq!(file.implement, vec!["Wc"]);
}

#[test]
fn parse_tee() {
    let file = parse_file("external/inferno-os/appl/cmd/tee.b")
        .expect("tee.b should parse");
    assert_eq!(file.implement, vec!["Tee"]);
}

#[test]
fn parse_tail() {
    let file = parse_file("external/inferno-os/appl/cmd/tail.b")
        .expect("tail.b should parse");
    assert_eq!(file.implement, vec!["Tail"]);
}
