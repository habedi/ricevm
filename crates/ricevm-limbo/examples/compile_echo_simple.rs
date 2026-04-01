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
    while(args != nil) {
        sys->print("%s\n", hd args);
        args = tl args;
    }
}
"#;
    let bytes = ricevm_limbo::compile_to_bytes(src, "echo.b").unwrap_or_else(|e| panic!("{e}"));
    std::fs::write("echo_simple.dis", &bytes).unwrap_or_else(|e| panic!("{e}"));
    println!("Wrote {} bytes", bytes.len());
}
