fn main() {
    let src = r#"implement Test;
include "sys.m";
    sys: Sys;
include "draw.m";
Test: module { init: fn(nil: ref Draw->Context, nil: list of string); };

sender(c: chan of int)
{
    c <-= 42;
}

init(nil: ref Draw->Context, nil: list of string)
{
    sys = load Sys Sys->PATH;

    c := chan of int;
    spawn sender(c);
    v := <-c;
    sys->print("received: %d\n", v);
}
"#;
    let bytes = ricevm_limbo::compile_to_bytes(src, "test.b").unwrap_or_else(|e| panic!("{e}"));
    std::fs::write("test_chan.dis", &bytes).unwrap_or_else(|e| panic!("{e}"));
    println!("Wrote {} bytes", bytes.len());
}
