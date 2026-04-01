fn main() {
    let src = r#"implement Test;
include "sys.m";
    sys: Sys;
include "draw.m";
Test: module { init: fn(nil: ref Draw->Context, args: list of string); };
init(nil: ref Draw->Context, args: list of string)
{
    sys = load Sys Sys->PATH;

    # Test case statement
    x := 2;
    case x {
    1 =>
        sys->print("one\n");
    2 =>
        sys->print("two\n");
    * =>
        sys->print("other\n");
    }

    # Test string indexing
    s := "hello";
    sys->print("char: %d\n", s[0]);

    # Test do-while
    i := 3;
    do {
        sys->print("do: %d\n", i);
        i--;
    } while(i > 0);

    sys->print("done\n");
}
"#;
    let bytes = ricevm_limbo::compile_to_bytes(src, "test.b").unwrap_or_else(|e| panic!("{e}"));
    std::fs::write("test_features.dis", &bytes).unwrap_or_else(|e| panic!("{e}"));
    println!("Wrote {} bytes", bytes.len());
}
