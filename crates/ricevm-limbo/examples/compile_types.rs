fn main() {
    let src = r#"implement Test;
include "sys.m";
    sys: Sys;
include "draw.m";
Test: module { init: fn(nil: ref Draw->Context, nil: list of string); };
init(nil: ref Draw->Context, nil: list of string)
{
    sys = load Sys Sys->PATH;

    # Real literals
    x := 3.14;
    sys->print("real: %g\n", x);

    # Type conversions
    n := 65;
    s := string n;
    sys->print("string 65 = %s\n", s);

    # Array of byte conversion
    a := array of byte "test";
    sys->print("array len = %d\n", len a);

    sys->print("done\n");
}
"#;
    let bytes = ricevm_limbo::compile_to_bytes(src, "test.b").unwrap_or_else(|e| panic!("{e}"));
    std::fs::write("test_types.dis", &bytes).unwrap_or_else(|e| panic!("{e}"));
    println!("Wrote {} bytes", bytes.len());
}
