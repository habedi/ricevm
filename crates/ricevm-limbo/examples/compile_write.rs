fn main() {
    let src = r#"implement Test;
include "sys.m";
    sys: Sys;
include "draw.m";
Test: module { init: fn(nil: ref Draw->Context, nil: list of string); };
init(nil: ref Draw->Context, nil: list of string)
{
    sys = load Sys Sys->PATH;
    a := array of byte "hello\n";
    sys->write(sys->fildes(1), a, len a);
}
"#;
    let bytes = ricevm_limbo::compile_to_bytes(src, "test.b").unwrap_or_else(|e| panic!("{e}"));
    std::fs::write("test_write.dis", &bytes).unwrap_or_else(|e| panic!("{e}"));
    println!("Wrote {} bytes", bytes.len());
}
