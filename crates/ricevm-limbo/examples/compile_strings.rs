/// Compile a Limbo program with string concatenation, list ops, and sys->print with args.
fn main() {
    let src = r#"implement Test;
include "sys.m";
    sys: Sys;
include "draw.m";
Test: module { init: fn(nil: ref Draw->Context, args: list of string); };
init(nil: ref Draw->Context, args: list of string)
{
    sys = load Sys Sys->PATH;
    s := "hello";
    s = s + " world";
    sys->print("%s\n", s);
    if(args != nil)
        args = tl args;
    while(args != nil) {
        sys->print("%s\n", hd args);
        args = tl args;
    }
}
"#;
    let bytes = ricevm_limbo::compile_to_bytes(src, "test.b")
        .unwrap_or_else(|e| panic!("compile failed: {e}"));
    std::fs::write("test_strings.dis", &bytes).unwrap_or_else(|e| panic!("write failed: {e}"));
    println!("Wrote {} bytes", bytes.len());
}
