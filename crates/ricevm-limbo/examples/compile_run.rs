/// Compile a Limbo program with control flow and run it via RiceVM.
fn main() {
    let src = r#"implement Test;
include "sys.m";
    sys: Sys;
include "draw.m";
Test: module { init: fn(nil: ref Draw->Context, nil: list of string); };
init(nil: ref Draw->Context, nil: list of string)
{
    sys = load Sys Sys->PATH;
    x := 42;
    if (x > 10)
        sys->print("big\n");
    else
        sys->print("small\n");
    i := 0;
    while (i < 3) {
        sys->print("loop\n");
        i++;
    }
    sys->print("done\n");
}
"#;
    let bytes = ricevm_limbo::compile_to_bytes(src, "test.b")
        .unwrap_or_else(|e| panic!("compile failed: {e}"));
    std::fs::write("test_compiled.dis", &bytes).unwrap_or_else(|e| panic!("write failed: {e}"));
    println!("Wrote {} bytes to test_compiled.dis", bytes.len());
}
