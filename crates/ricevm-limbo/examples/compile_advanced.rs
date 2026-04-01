fn main() {
    let src = r#"implement Test;
include "sys.m";
    sys: Sys;
include "draw.m";
Test: module { init: fn(nil: ref Draw->Context, nil: list of string); };

double(x: int): int
{
    return x * 2;
}

greet(name: string)
{
    sys->print("Hello, %s!\n", name);
}

init(nil: ref Draw->Context, nil: list of string)
{
    sys = load Sys Sys->PATH;

    # Test local function calls
    n := double(21);
    sys->print("double(21) = %d\n", n);

    greet("World");
    greet("Limbo");

    sys->print("done\n");
}
"#;
    let bytes = ricevm_limbo::compile_to_bytes(src, "test.b").unwrap_or_else(|e| panic!("{e}"));
    std::fs::write("test_advanced.dis", &bytes).unwrap_or_else(|e| panic!("{e}"));
    println!("Wrote {} bytes", bytes.len());
}
