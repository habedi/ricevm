/// Compile a hello world program and write it as .dis.
fn main() {
    let src = r#"implement Hello;
include "sys.m";
    sys: Sys;
include "draw.m";
Hello: module { init: fn(nil: ref Draw->Context, nil: list of string); };
init(nil: ref Draw->Context, nil: list of string)
{
    sys = load Sys Sys->PATH;
    sys->print("hello from ricevm-limbo!\n");
}
"#;
    let bytes = ricevm_limbo::compile_to_bytes(src, "hello.b")
        .unwrap_or_else(|e| panic!("compile failed: {e}"));
    std::fs::write("hello_compiled.dis", &bytes).unwrap_or_else(|e| panic!("write failed: {e}"));
    println!("Wrote {} bytes to hello_compiled.dis", bytes.len());
}
