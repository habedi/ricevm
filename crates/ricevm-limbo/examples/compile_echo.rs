/// Compile a simplified echo program and run it.
fn main() {
    let src = r#"implement Echo;
include "sys.m";
    sys: Sys;
include "draw.m";
Echo: module { init: fn(nil: ref Draw->Context, args: list of string); };
init(nil: ref Draw->Context, args: list of string)
{
    sys = load Sys Sys->PATH;
    if(args != nil)
        args = tl args;
    s := "";
    while(args != nil) {
        if(s != "")
            s = s + " ";
        s = s + hd args;
        args = tl args;
    }
    s = s + "\n";
    a := array of byte s;
    sys->write(sys->fildes(1), a, len a);
}
"#;
    let bytes = ricevm_limbo::compile_to_bytes(src, "echo.b")
        .unwrap_or_else(|e| panic!("compile failed: {e}"));
    std::fs::write("echo_compiled.dis", &bytes).unwrap_or_else(|e| panic!("write failed: {e}"));
    println!("Wrote {} bytes", bytes.len());
}
