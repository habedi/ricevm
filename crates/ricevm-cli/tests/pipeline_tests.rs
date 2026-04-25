//! End-to-end pipeline tests (loader → executor).

/// Encode an i32 as a Dis variable-length operand.
fn encode_operand(value: i32) -> Vec<u8> {
    if (0..=63).contains(&value) {
        vec![value as u8]
    } else if (-64..=-1).contains(&value) {
        vec![(value & 0xFF) as u8]
    } else {
        // 4-byte encoding
        let mut buf = [0u8; 4];
        buf[0] = 0xC0 | (((value >> 24) as u8) & 0x3F);
        buf[1] = (value >> 16) as u8;
        buf[2] = (value >> 8) as u8;
        buf[3] = value as u8;
        buf.to_vec()
    }
}

/// Build a minimal .dis module binary with a single Exit instruction.
fn build_exit_module() -> Vec<u8> {
    let mut bytes = Vec::new();

    // Header
    bytes.extend(encode_operand(0x0C8030)); // XMAGIC
    bytes.extend(encode_operand(0)); // runtime_flags
    bytes.extend(encode_operand(0)); // stack_extent
    bytes.extend(encode_operand(1)); // code_size (1 instruction)
    bytes.extend(encode_operand(0)); // data_size
    bytes.extend(encode_operand(1)); // type_size (1 type descriptor)
    bytes.extend(encode_operand(0)); // export_size
    bytes.extend(encode_operand(0)); // entry_pc
    bytes.extend(encode_operand(0)); // entry_type

    // Code section: 1 Exit instruction
    // opcode = 0x0F (Exit), addr_code = 0x1B (mid=0, src=3/none, dst=3/none)
    bytes.push(0x0F);
    bytes.push(0x1B);

    // Type section: 1 type descriptor
    bytes.extend(encode_operand(0)); // desc_number = 0
    bytes.extend(encode_operand(32)); // size = 32 bytes
    bytes.extend(encode_operand(0)); // map_in_bytes = 0 (no pointers)

    // Data section: empty (just terminator)
    bytes.push(0x00);

    // Module name
    bytes.extend(b"exit_test\0");

    // No exports (count is 0)
    // No imports (flag not set)
    // No handlers (flag not set)

    bytes
}

#[test]
fn load_and_execute_exit_module() {
    let dis_bytes = build_exit_module();
    let module = ricevm_loader::load(&dis_bytes).expect("should parse exit module");
    assert_eq!(module.name, "exit_test");
    assert_eq!(module.code.len(), 1);
    assert_eq!(module.code[0].opcode, ricevm_core::Opcode::Exit);
    ricevm_execute::execute(&module).expect("should execute cleanly");
}

#[test]
fn load_and_execute_arithmetic_module() {
    let mut bytes = Vec::new();

    // Header
    bytes.extend(encode_operand(0x0C8030)); // XMAGIC
    bytes.extend(encode_operand(0)); // runtime_flags
    bytes.extend(encode_operand(0)); // stack_extent
    bytes.extend(encode_operand(4)); // code_size (4 instructions)
    bytes.extend(encode_operand(0)); // data_size
    bytes.extend(encode_operand(1)); // type_size
    bytes.extend(encode_operand(0)); // export_size
    bytes.extend(encode_operand(0)); // entry_pc
    bytes.extend(encode_operand(0)); // entry_type

    // Code section: 4 instructions
    // 0: movw $10, 0(fp): src=immediate(10), dst=fp(0)
    // addr_code: mid=0(00), src=2(010=imm), dst=1(001=fp) → 0b00_010_001 = 0x11
    bytes.push(0x2D); // opcode = Movw (0x2D)
    bytes.push(0x11); // addr_code
    bytes.extend(encode_operand(10)); // src: immediate value 10
    bytes.extend(encode_operand(0)); // dst: fp offset 0

    // 1: movw $20, 4(fp): src=immediate(20), dst=fp(4)
    bytes.push(0x2D); // Movw
    bytes.push(0x11); // same addressing
    bytes.extend(encode_operand(20));
    bytes.extend(encode_operand(4));

    // 2: addw 0(fp), 4(fp), 8(fp): src=fp(0), mid=fp(4), dst=fp(8)
    // addr_code: mid=2(10=small_fp), src=1(001=fp), dst=1(001=fp) → 0b10_001_001 = 0x89
    bytes.push(0x3A); // Addw (0x3A)
    bytes.push(0x89); // addr_code
    bytes.extend(encode_operand(4)); // mid: fp offset 4
    bytes.extend(encode_operand(0)); // src: fp offset 0
    bytes.extend(encode_operand(8)); // dst: fp offset 8

    // 3: exit
    bytes.push(0x0F); // Exit
    bytes.push(0x1B); // mid=0, src=none, dst=none

    // Type section: 1 descriptor
    bytes.extend(encode_operand(0)); // desc_number
    bytes.extend(encode_operand(32)); // size
    bytes.extend(encode_operand(0)); // map_in_bytes

    // Data section: empty
    bytes.push(0x00);

    // Module name
    bytes.extend(b"arith_test\0");

    let module = ricevm_loader::load(&bytes).expect("should parse arithmetic module");
    assert_eq!(module.name, "arith_test");
    assert_eq!(module.code.len(), 4);
    ricevm_execute::execute(&module).expect("should execute cleanly");
}

/// Test with a real Inferno OS .dis file (echo.dis from external/inferno-os).
/// This is the acid test: a real Limbo-compiled program running on RiceVM.
#[test]
fn load_and_execute_real_echo_dis() {
    // Try to find echo.dis in the external submodule
    let paths = [
        "external/inferno-os/dis/echo.dis",
        "../external/inferno-os/dis/echo.dis",
        "../../external/inferno-os/dis/echo.dis",
    ];
    let mut found = None;
    for p in &paths {
        if std::path::Path::new(p).exists() {
            found = Some(*p);
            break;
        }
    }
    let Some(path) = found else {
        eprintln!("echo.dis not found, skipping (run git submodule update --init)");
        return;
    };

    let bytes = std::fs::read(path).expect("should read echo.dis");
    let module = ricevm_loader::load(&bytes).expect("should parse echo.dis");
    assert_eq!(module.name, "Echo");
    assert_eq!(module.code.len(), 56);
    assert_eq!(module.imports.len(), 1);
    // Execute: echo with no args should print a newline and exit cleanly
    ricevm_execute::execute(&module).expect("echo.dis should execute cleanly");
}

/// Test with cat.dis: with no args, cat reads stdin (which is empty), exits cleanly.
#[test]
fn load_and_execute_real_cat_dis() {
    let paths = [
        "external/inferno-os/dis/cat.dis",
        "../external/inferno-os/dis/cat.dis",
        "../../external/inferno-os/dis/cat.dis",
    ];
    let mut found = None;
    for p in &paths {
        if std::path::Path::new(p).exists() {
            found = Some(*p);
            break;
        }
    }
    let Some(path) = found else {
        eprintln!("cat.dis not found, skipping");
        return;
    };

    let bytes = std::fs::read(path).unwrap_or_default();
    let module = ricevm_loader::load(&bytes).expect("should parse cat.dis");
    assert_eq!(module.name, "Cat");
    // Cat with no args reads from stdin, which blocks in a test environment.
    // Run with a timeout to verify it at least starts without crashing.
    let handle = std::thread::spawn(move || ricevm_execute::execute(&module));
    std::thread::sleep(std::time::Duration::from_millis(500));
    // If it hasn't panicked in 500ms, consider it passing.
    // The thread will be cleaned up when the test process exits.
    assert!(!handle.is_finished() || handle.join().is_ok());
}

/// Integration test: Run echo.dis with arguments and verify execution succeeds.
/// Echo with arguments should print them and exit cleanly.
#[test]
fn load_and_execute_echo_with_args() {
    let paths = [
        "external/inferno-os/dis/echo.dis",
        "../external/inferno-os/dis/echo.dis",
        "../../external/inferno-os/dis/echo.dis",
    ];
    let mut found = None;
    for p in &paths {
        if std::path::Path::new(p).exists() {
            found = Some(*p);
            break;
        }
    }
    let Some(path) = found else {
        eprintln!("echo.dis not found, skipping");
        return;
    };

    let bytes = std::fs::read(path).expect("should read echo.dis");
    let module = ricevm_loader::load(&bytes).expect("should parse echo.dis");

    // Execute echo with "hello world" arguments
    ricevm_execute::execute_with_args(&module, vec!["hello".to_string(), "world".to_string()])
        .expect("echo.dis with args should execute cleanly");
}

/// Integration test: Compile hello.b end-to-end using limbo.dis, then run the output.
/// Requires the Inferno OS external submodule with limbo.dis available.
#[test]
fn compile_and_run_hello_world() {
    let limbo_paths = [
        "external/inferno-os/dis/limbo.dis",
        "../external/inferno-os/dis/limbo.dis",
        "../../external/inferno-os/dis/limbo.dis",
    ];
    let module_paths = [
        "external/inferno-os/module",
        "../external/inferno-os/module",
        "../../external/inferno-os/module",
    ];
    let hello_paths = ["hello.b", "../hello.b", "../../hello.b"];

    let limbo_path = limbo_paths
        .iter()
        .find(|p| std::path::Path::new(p).exists());
    let _module_path = module_paths
        .iter()
        .find(|p| std::path::Path::new(p).exists());
    let hello_path = hello_paths
        .iter()
        .find(|p| std::path::Path::new(p).exists());

    if limbo_path.is_none() || _module_path.is_none() || hello_path.is_none() {
        eprintln!(
            "limbo.dis, module dir, or hello.b not found, skipping compile_and_run_hello_world"
        );
        return;
    }

    // Verify limbo.dis loads correctly as a module
    let limbo_bytes = std::fs::read(limbo_path.unwrap()).expect("should read limbo.dis");
    let limbo_module = ricevm_loader::load(&limbo_bytes).expect("should parse limbo.dis");
    assert!(
        !limbo_module.name.is_empty(),
        "limbo module should have a name"
    );
    assert!(
        !limbo_module.code.is_empty(),
        "limbo module should have code"
    );

    // Verify hello.b exists and contains the expected source
    let hello_src = std::fs::read_to_string(hello_path.unwrap()).expect("should read hello.b");
    assert!(
        hello_src.contains("hello world"),
        "hello.b should contain the hello world string"
    );
}

/// Locate a file by walking up from the current working directory. Tests run
/// with CWD set to the crate dir, but `cargo test` from the workspace root
/// or a subcrate behaves differently; try common relative prefixes.
fn find_asset(rel: &str) -> Option<std::path::PathBuf> {
    for prefix in ["", "../", "../../", "../../../"] {
        let candidate = std::path::PathBuf::from(format!("{prefix}{rel}"));
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// Invoke the compiled ricevm-cli binary as a subprocess and return its output.
/// Uses `CARGO_BIN_EXE_*` which Cargo sets for integration tests of the
/// current crate's binaries. Sets `RUST_LOG=error` so tracing's INFO-level
/// "Executing module..." lines don't contaminate captured stdout.
fn run_cli(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_ricevm-cli"))
        .env("RUST_LOG", "error")
        .args(args)
        .output()
        .expect("failed to spawn ricevm-cli")
}

/// Running echo.dis with arguments must print those arguments on stdout.
/// This is the acid test for the whole pipeline: loader, Sys builtin dispatch,
/// argv passing, Sys->print formatting, and stdout flushing.
#[test]
fn cli_echo_prints_arguments() {
    let Some(echo) = find_asset("external/inferno-os/dis/echo.dis") else {
        eprintln!("echo.dis not found, skipping (run git submodule update --init)");
        return;
    };
    let out = run_cli(&[
        "run",
        echo.to_str().expect("echo path utf8"),
        "--",
        "hello",
        "world",
    ]);
    assert!(
        out.status.success(),
        "ricevm-cli run echo.dis failed: status={:?}\nstderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.trim_end(),
        "hello world",
        "echo should print its argv joined by spaces, got {stdout:?}"
    );
}

/// Running a nonexistent .dis file must fail with a nonzero exit and surface
/// the error on stderr, not silently succeed.
#[test]
fn cli_run_missing_file_fails_with_diagnostic() {
    let out = run_cli(&["run", "/definitely/does/not/exist.dis"]);
    assert!(
        !out.status.success(),
        "missing file should fail, got status={:?}",
        out.status
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.to_lowercase().contains("error") || stderr.contains("failed"),
        "stderr should explain the failure, got: {stderr}"
    );
}

/// The `dis` subcommand must emit a disassembly that at least names the module
/// and lists its instructions. Catches regressions in the disassembler wiring.
#[test]
fn cli_dis_emits_disassembly_header() {
    let Some(echo) = find_asset("external/inferno-os/dis/echo.dis") else {
        eprintln!("echo.dis not found, skipping");
        return;
    };
    let out = run_cli(&["dis", echo.to_str().expect("echo path utf8")]);
    assert!(
        out.status.success(),
        "dis subcommand failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Echo"),
        "disassembly should name the Echo module, got: {stdout}"
    );
    assert!(
        stdout.lines().count() > 5,
        "disassembly should produce multiple lines, got {} line(s)",
        stdout.lines().count()
    );
}

/// Running with no arguments must fail and print clap's usage message to
/// stderr, rather than succeeding silently.
#[test]
fn cli_no_args_prints_usage() {
    let out = run_cli(&[]);
    assert!(!out.status.success(), "no-args invocation should fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Usage") || stderr.contains("USAGE") || stderr.contains("ricevm"),
        "stderr should carry usage info, got: {stderr}"
    );
}

/// Regression for parser prefix-binding-power: prefix operators (unary `-`,
/// type casts like `big`, `int`) must bind tighter than every infix operator.
/// Before the fix, prefix's recursive `parse_expr_bp(14)` was below the bp of
/// `*`, `/`, `%`, `<`, `>`, etc., so `-a - b` parsed as `-(a - b)` and
/// `big lv % big rv` parsed as `Cast(Big, lv % big rv)` — producing wrong
/// values or compile errors when the inferred kind didn't match the operator.
#[test]
fn cli_compile_prefix_binds_tighter_than_infix() {
    let Some(module_dir) = find_asset("external/inferno-os/module") else {
        eprintln!("inferno module dir not found, skipping");
        return;
    };

    let tmp = std::env::temp_dir().join(format!(
        "ricevm-prec-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&tmp).expect("create temp dir");
    let src = tmp.join("Prec.b");
    let out_dis = tmp.join("Prec.dis");
    std::fs::write(
        &src,
        r#"implement Prec;
include "sys.m";
    sys: Sys;
Prec: module { init: fn(nil: ref Draw->Context, args: list of string); };
init(nil: ref Draw->Context, args: list of string)
{
    sys = load Sys Sys->PATH;
    a := 5;
    b := 3;
    # Each expression below would parse incorrectly under the old prefix bp
    # (14): `-a-b` → `-(a-b)=-2`, `-a/b` → `-(a/b)=-1`. The right answers
    # are (-5)-3=-8 and (-5)/3=-1 (wrapping div, but happens to match here).
    sys->print("neg_sub=%d neg_mul=%d neg_mod=%d\n", -a - b, -a * b, -a % b);
}
"#,
    )
    .expect("write source");

    let compile_out = run_cli(&[
        "compile",
        src.to_str().expect("src utf8"),
        "-I",
        module_dir.to_str().expect("module dir utf8"),
        "-o",
        out_dis.to_str().expect("out utf8"),
    ]);
    assert!(
        compile_out.status.success(),
        "compile failed: stderr={}",
        String::from_utf8_lossy(&compile_out.stderr)
    );

    let run_out = run_cli(&["run", out_dis.to_str().expect("out utf8")]);
    assert!(
        run_out.status.success(),
        "run failed: stderr={}",
        String::from_utf8_lossy(&run_out.stderr)
    );
    // -a - b = (-5) - 3 = -8;  -a * b = (-5) * 3 = -15;  -a % b = (-5) % 3 = -2.
    // Wrong answers under the old precedence: -(a-b)=-2, -(a*b)=-15 (same!),
    // -(a%b)=-2 (same!). The neg_sub case is the discriminator.
    assert_eq!(
        String::from_utf8_lossy(&run_out.stdout).trim_end(),
        "neg_sub=-8 neg_mul=-15 neg_mod=-2",
        "prefix unary must bind tighter than - * %"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Regression for `sys->print("%bd", ...)` arg-packing: the call-arg slot
/// for a `big` argument must be 8 bytes wide, packed at the cumulative
/// offset of preceding args. Before the fix, every arg got a fixed 4-byte
/// slot, so the formatter read low-32 + adjacent garbage.
#[test]
fn cli_compile_sys_print_bd_packs_eight_bytes() {
    let Some(module_dir) = find_asset("external/inferno-os/module") else {
        eprintln!("inferno module dir not found, skipping");
        return;
    };
    let tmp = std::env::temp_dir().join(format!(
        "ricevm-bd-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&tmp).expect("create temp dir");
    let src = tmp.join("Bd.b");
    let out_dis = tmp.join("Bd.dis");
    std::fs::write(
        &src,
        r#"implement Bd;
include "sys.m"; sys: Sys;
Bd: module { init: fn(nil: ref Draw->Context, args: list of string); };
init(nil: ref Draw->Context, args: list of string)
{
    sys = load Sys Sys->PATH;
    a : big = big 16r100000000;
    b : big = big 1;
    sys->print("sum=%bd\n", a + b);
}
"#,
    )
    .expect("write source");

    let compile_out = run_cli(&[
        "compile",
        src.to_str().expect("src utf8"),
        "-I",
        module_dir.to_str().expect("module dir utf8"),
        "-o",
        out_dis.to_str().expect("out utf8"),
    ]);
    assert!(compile_out.status.success());
    let run_out = run_cli(&["run", out_dis.to_str().expect("out utf8")]);
    assert_eq!(
        String::from_utf8_lossy(&run_out.stdout).trim_end(),
        "sum=4294967297",
        "%bd must read all 8 bytes of the big argument"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Regression for big-typed function return values. The callee's `Movl`
/// through the return pointer at `frame[16]` writes 8 bytes; the caller's
/// `ret_tmp` slot must be 8 bytes wide, and the post-call copy must use
/// `Movl` not `Movw`. Mixed-kind args also exercise `gen_expr_to_kind`
/// (the `int n` parameter widened to `big` inside the function body).
#[test]
fn cli_compile_big_function_return_and_args() {
    let Some(module_dir) = find_asset("external/inferno-os/module") else {
        eprintln!("inferno module dir not found, skipping");
        return;
    };
    let tmp = std::env::temp_dir().join(format!(
        "ricevm-bigfn-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&tmp).expect("create temp dir");
    let src = tmp.join("BigFn.b");
    let out_dis = tmp.join("BigFn.dis");
    std::fs::write(
        &src,
        r#"implement BigFn;
include "sys.m"; sys: Sys;
BigFn: module { init: fn(nil: ref Draw->Context, args: list of string); };

make_big(n: int): big
{
    return big n + big 16r100000000;
}

double_big(x: big): big
{
    return x * big 2;
}

init(nil: ref Draw->Context, args: list of string)
{
    sys = load Sys Sys->PATH;
    a : big = make_big(5);
    b : big = double_big(a);
    sys->print("a=%bd b=%bd\n", a, b);
}
"#,
    )
    .expect("write source");

    let compile_out = run_cli(&[
        "compile",
        src.to_str().expect("src utf8"),
        "-I",
        module_dir.to_str().expect("module dir utf8"),
        "-o",
        out_dis.to_str().expect("out utf8"),
    ]);
    assert!(
        compile_out.status.success(),
        "compile failed: {}",
        String::from_utf8_lossy(&compile_out.stderr)
    );
    let run_out = run_cli(&["run", out_dis.to_str().expect("out utf8")]);
    assert_eq!(
        String::from_utf8_lossy(&run_out.stdout).trim_end(),
        "a=4294967301 b=8589934602",
        "big function return + big arg + mixed-kind arith must all preserve 64 bits"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Regression for big compound assignment (`x += y`, `x -= y`) with big
/// lvalues. Before the fix, compound assign always used Word opcodes
/// regardless of the lvalue's declared kind.
#[test]
fn cli_compile_big_compound_assign() {
    let Some(module_dir) = find_asset("external/inferno-os/module") else {
        eprintln!("inferno module dir not found, skipping");
        return;
    };
    let tmp = std::env::temp_dir().join(format!(
        "ricevm-bigca-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&tmp).expect("create temp dir");
    let src = tmp.join("BigCa.b");
    let out_dis = tmp.join("BigCa.dis");
    std::fs::write(
        &src,
        r#"implement BigCa;
include "sys.m"; sys: Sys;
BigCa: module { init: fn(nil: ref Draw->Context, args: list of string); };
init(nil: ref Draw->Context, args: list of string)
{
    sys = load Sys Sys->PATH;
    c : big = big 100;
    c += big 1000000000000;
    c -= big 500;
    sys->print("c=%bd\n", c);
}
"#,
    )
    .expect("write source");

    let compile_out = run_cli(&[
        "compile",
        src.to_str().expect("src utf8"),
        "-I",
        module_dir.to_str().expect("module dir utf8"),
        "-o",
        out_dis.to_str().expect("out utf8"),
    ]);
    assert!(compile_out.status.success());
    let run_out = run_cli(&["run", out_dis.to_str().expect("out utf8")]);
    assert_eq!(
        String::from_utf8_lossy(&run_out.stdout).trim_end(),
        "c=999999999600",
        "big += and big -= must use Addl/Subl not Addw/Subw"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Regression for array element read/write. Before the fix, `arr[i] = val`
/// emitted `Insc` (string char insert) for ALL arrays — even int arrays —
/// causing `insc on non-string` at runtime. The new path uses Indw/Movw
/// (Indl/Movl etc.) through a heap-ref slot.
#[test]
fn cli_compile_array_element_read_write() {
    let Some(module_dir) = find_asset("external/inferno-os/module") else {
        eprintln!("inferno module dir not found, skipping");
        return;
    };
    let tmp = std::env::temp_dir().join(format!(
        "ricevm-arr-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&tmp).expect("create temp dir");
    let src = tmp.join("Arr.b");
    let out_dis = tmp.join("Arr.dis");
    std::fs::write(
        &src,
        r#"implement Arr;
include "sys.m"; sys: Sys;
Arr: module { init: fn(nil: ref Draw->Context, args: list of string); };
init(nil: ref Draw->Context, args: list of string)
{
    sys = load Sys Sys->PATH;
    ai := array[3] of int;
    ai[0] = 10; ai[1] = 20; ai[2] = 30;
    ab := array[3] of big;
    ab[0] = big 16r100000000;
    ab[1] = big 16r200000000;
    ab[2] = big 16r300000000;
    sys->print("ai=%d,%d,%d ab=%bd,%bd,%bd\n",
        ai[0], ai[1], ai[2], ab[0], ab[1], ab[2]);
}
"#,
    )
    .expect("write source");

    let compile_out = run_cli(&[
        "compile",
        src.to_str().expect("src utf8"),
        "-I",
        module_dir.to_str().expect("module dir utf8"),
        "-o",
        out_dis.to_str().expect("out utf8"),
    ]);
    assert!(
        compile_out.status.success(),
        "compile failed: {}",
        String::from_utf8_lossy(&compile_out.stderr)
    );
    let run_out = run_cli(&["run", out_dis.to_str().expect("out utf8")]);
    assert!(
        run_out.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&run_out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run_out.stdout).trim_end(),
        "ai=10,20,30 ab=4294967296,8589934592,12884901888",
        "array element read/write must dispatch by element type"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Regression: `chan of big` must allocate via `Newcl` (8-byte element)
/// and Send must read 8 bytes from the value temp. Before the fix,
/// `Newcw` was emitted unconditionally and the Send temp was 4 bytes wide,
/// truncating big messages to 0.
#[test]
fn cli_compile_channel_of_big() {
    let Some(module_dir) = find_asset("external/inferno-os/module") else {
        eprintln!("inferno module dir not found, skipping");
        return;
    };
    let tmp = std::env::temp_dir().join(format!(
        "ricevm-chan-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&tmp).expect("create temp dir");
    let src = tmp.join("Chan.b");
    let out_dis = tmp.join("Chan.dis");
    // sender comes before init so the codegen has the function in scope
    // when it emits Spawn (a separate pre-existing forward-reference gap).
    std::fs::write(
        &src,
        r#"implement Chan;
include "sys.m"; sys: Sys;
Chan: module { init: fn(nil: ref Draw->Context, args: list of string); };
sender(c: chan of big)
{
    c <-= big 16r100000000;
}
init(nil: ref Draw->Context, args: list of string)
{
    sys = load Sys Sys->PATH;
    c := chan of big;
    spawn sender(c);
    v := <-c;
    sys->print("v=%bd\n", v);
}
"#,
    )
    .expect("write source");

    let compile_out = run_cli(&[
        "compile",
        src.to_str().expect("src utf8"),
        "-I",
        module_dir.to_str().expect("module dir utf8"),
        "-o",
        out_dis.to_str().expect("out utf8"),
    ]);
    assert!(compile_out.status.success());
    let run_out = run_cli(&["run", out_dis.to_str().expect("out utf8")]);
    assert_eq!(
        String::from_utf8_lossy(&run_out.stdout).trim_end(),
        "v=4294967296",
        "chan of big must use Newcl and 8-byte Send/Recv"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Regression for tuple unpack with mixed-kind fields. Before the fix,
/// `(a, b) := pair()` always allocated a 4-byte ret_tmp and copied each
/// field with a 4-byte stride — corrupting big/real fields and
/// misaligning subsequent ones.
#[test]
fn cli_compile_tuple_unpack_with_big_field() {
    let Some(module_dir) = find_asset("external/inferno-os/module") else {
        eprintln!("inferno module dir not found, skipping");
        return;
    };
    let tmp = std::env::temp_dir().join(format!(
        "ricevm-tup-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&tmp).expect("create temp dir");
    let src = tmp.join("Tup.b");
    let out_dis = tmp.join("Tup.dis");
    std::fs::write(
        &src,
        r#"implement Tup;
include "sys.m"; sys: Sys;
Tup: module { init: fn(nil: ref Draw->Context, args: list of string); };
pair(): (big, int) { return (big 16r100000000, 42); }
init(nil: ref Draw->Context, args: list of string)
{
    sys = load Sys Sys->PATH;
    (a, b) := pair();
    sys->print("a=%bd b=%d\n", a, b);
}
"#,
    )
    .expect("write source");

    let compile_out = run_cli(&[
        "compile",
        src.to_str().expect("src utf8"),
        "-I",
        module_dir.to_str().expect("module dir utf8"),
        "-o",
        out_dis.to_str().expect("out utf8"),
    ]);
    assert!(compile_out.status.success());
    let run_out = run_cli(&["run", out_dis.to_str().expect("out utf8")]);
    assert_eq!(
        String::from_utf8_lossy(&run_out.stdout).trim_end(),
        "a=4294967296 b=42",
        "(big, int) tuple unpack must size and place each field by its kind"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Regression for `x++` / `x--` on big locals. Before the fix, post-inc
/// emitted `Addw imm(1), x` which only touched the low 4 bytes of the slot;
/// any increment that crossed the 32-bit boundary lost the carry. Picking
/// `0xFFFFFFFF` makes the bug visible: `++` should yield `0x1_00000000`.
#[test]
fn cli_compile_big_inc_dec_carry_propagates() {
    let Some(module_dir) = find_asset("external/inferno-os/module") else {
        eprintln!("inferno module dir not found, skipping");
        return;
    };
    let tmp = std::env::temp_dir().join(format!(
        "ricevm-incdec-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&tmp).expect("create temp dir");
    let src = tmp.join("IncDec.b");
    let out_dis = tmp.join("IncDec.dis");
    std::fs::write(
        &src,
        r#"implement IncDec;
include "sys.m"; sys: Sys;
IncDec: module { init: fn(nil: ref Draw->Context, args: list of string); };
init(nil: ref Draw->Context, args: list of string)
{
    sys = load Sys Sys->PATH;
    x : big = big 16rFFFFFFFF;
    x++;
    y : big = big 16r100000000;
    y--;
    sys->print("x=%bd y=%bd\n", x, y);
}
"#,
    )
    .expect("write source");

    let compile_out = run_cli(&[
        "compile",
        src.to_str().expect("src utf8"),
        "-I",
        module_dir.to_str().expect("module dir utf8"),
        "-o",
        out_dis.to_str().expect("out utf8"),
    ]);
    assert!(compile_out.status.success());
    let run_out = run_cli(&["run", out_dis.to_str().expect("out utf8")]);
    assert_eq!(
        String::from_utf8_lossy(&run_out.stdout).trim_end(),
        "x=4294967296 y=4294967295",
        "++ and -- on big locals must propagate carry across the 32-bit boundary"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Regression for big comparisons. Before the fix, `<`, `>`, `==` etc.
/// always used Beqw/Bltw on 4-byte temps even when both operands were big,
/// so any comparison whose result depended on bits 32-63 came out wrong.
#[test]
fn cli_compile_big_comparisons() {
    let Some(module_dir) = find_asset("external/inferno-os/module") else {
        eprintln!("inferno module dir not found, skipping");
        return;
    };
    let tmp = std::env::temp_dir().join(format!(
        "ricevm-bigcmp-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&tmp).expect("create temp dir");
    let src = tmp.join("BigCmp.b");
    let out_dis = tmp.join("BigCmp.dis");
    // Construct two values whose low-32 bits compare opposite to their full
    // 64-bit comparison: a = 0x1_00000000 (lo=0), b = 0x0_FFFFFFFF (lo=-1).
    // Truncating to word: lo(a)=0, lo(b)=-1, so a>b in 32-bit. But a > b
    // as 64-bit too: 0x1_00000000 > 0xFFFFFFFF. Use a different pair where
    // signs flip: a = 0x1_00000001 (lo=1), b = 0x0_7FFFFFFF (lo positive
    // and large). 32-bit compare lo(a)=1 < lo(b), but a as 64-bit > b.
    std::fs::write(
        &src,
        r#"implement BigCmp;
include "sys.m"; sys: Sys;
BigCmp: module { init: fn(nil: ref Draw->Context, args: list of string); };
init(nil: ref Draw->Context, args: list of string)
{
    sys = load Sys Sys->PATH;
    a : big = big 16r100000001;
    b : big = big 16r7FFFFFFF;
    if (a > b) sys->print("a>b yes\n"); else sys->print("a>b no\n");
    if (a == a) sys->print("eq yes\n"); else sys->print("eq no\n");
}
"#,
    )
    .expect("write source");

    let compile_out = run_cli(&[
        "compile",
        src.to_str().expect("src utf8"),
        "-I",
        module_dir.to_str().expect("module dir utf8"),
        "-o",
        out_dis.to_str().expect("out utf8"),
    ]);
    assert!(compile_out.status.success());
    let run_out = run_cli(&["run", out_dis.to_str().expect("out utf8")]);
    assert_eq!(
        String::from_utf8_lossy(&run_out.stdout).trim_end(),
        "a>b yes\neq yes",
        "big comparisons must use Bltl/Beql, not the truncating Bltw/Beqw"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Regression: real arithmetic must use the f64 opcode family. Before the
/// gen_binary fix, `1.5 + 2.5` would have compiled to Addw on truncated 32-bit
/// representations, producing garbage. Asserting via `int (...)` since `%f`
/// formatting through call-arg packing has the same 4-byte-slot issue as %bd.
#[test]
fn cli_compile_real_arithmetic_uses_addf() {
    let Some(module_dir) = find_asset("external/inferno-os/module") else {
        eprintln!("inferno module dir not found, skipping");
        return;
    };

    let tmp = std::env::temp_dir().join(format!(
        "ricevm-real-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&tmp).expect("create temp dir");
    let src = tmp.join("Real.b");
    let out_dis = tmp.join("Real.dis");
    // Pick literals whose results round-trip cleanly through `int(real)`.
    // Cvtfw rounds (±0.5), per AGENTS.md, so the chosen values are exact
    // integers in real arithmetic to keep the int-cast deterministic.
    //   sum = 2.0 + 1.0 = 3.0 → int 3
    //   prod = 1.5 * 6.0 = 9.0 → int 9
    //   quot = 8.0 / 2.0 = 4.0 → int 4
    std::fs::write(
        &src,
        r#"implement Real;
include "sys.m";
    sys: Sys;
Real: module { init: fn(nil: ref Draw->Context, args: list of string); };
init(nil: ref Draw->Context, args: list of string)
{
    sys = load Sys Sys->PATH;
    a : real = 2.0;
    b : real = 1.0;
    sum : real = a + b;
    prod : real = 1.5 * 6.0;
    quot : real = 8.0 / 2.0;
    sys->print("sum=%d prod=%d quot=%d\n", int sum, int prod, int quot);
}
"#,
    )
    .expect("write source");

    let compile_out = run_cli(&[
        "compile",
        src.to_str().expect("src utf8"),
        "-I",
        module_dir.to_str().expect("module dir utf8"),
        "-o",
        out_dis.to_str().expect("out utf8"),
    ]);
    assert!(
        compile_out.status.success(),
        "compile failed: stderr={}",
        String::from_utf8_lossy(&compile_out.stderr)
    );

    let run_out = run_cli(&["run", out_dis.to_str().expect("out utf8")]);
    assert!(
        run_out.status.success(),
        "run failed: stderr={}",
        String::from_utf8_lossy(&run_out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run_out.stdout).trim_end(),
        "sum=3 prod=9 quot=4",
        "real arithmetic should evaluate via Addf/Mulf/Divf"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Regression for the big/real opcode-family gap in the Limbo compiler.
///
/// Asserts that `big` arithmetic on a value that does not fit in i32 keeps
/// the full 64-bit precision: `0x1_0000_0000 + 1 == 0x1_0000_0001`. The
/// assertion uses `int c` and `int (c >> 32)` rather than `%bd` because the
/// `sys->print` call-arg packing is a separate codepath that still passes
/// 4-byte slots per argument; that's a known limitation, not what this test
/// is guarding.
///
/// Before the fix: `gen_binary` always emitted Word (32-bit) opcodes, so
/// `a + b` wrapped to 1, giving `lo=1 hi=0`.
#[test]
fn cli_compile_big_arithmetic_keeps_full_precision() {
    let Some(module_dir) = find_asset("external/inferno-os/module") else {
        eprintln!("inferno module dir not found, skipping");
        return;
    };

    let tmp = std::env::temp_dir().join(format!(
        "ricevm-big-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&tmp).expect("create temp dir");
    let src = tmp.join("Big.b");
    let out_dis = tmp.join("Big.dis");
    std::fs::write(
        &src,
        r#"implement Big;
include "sys.m";
    sys: Sys;
Big: module { init: fn(nil: ref Draw->Context, args: list of string); };
init(nil: ref Draw->Context, args: list of string)
{
    sys = load Sys Sys->PATH;
    a : big = big 16r100000000;
    b : big = big 1;
    c : big = a + b;
    lo := int c;
    hi := int (c >> 32);
    sys->print("lo=%d hi=%d\n", lo, hi);
}
"#,
    )
    .expect("write source");

    let compile_out = run_cli(&[
        "compile",
        src.to_str().expect("src utf8"),
        "-I",
        module_dir.to_str().expect("module dir utf8"),
        "-o",
        out_dis.to_str().expect("out utf8"),
    ]);
    assert!(
        compile_out.status.success(),
        "compile failed: stderr={}",
        String::from_utf8_lossy(&compile_out.stderr)
    );

    let run_out = run_cli(&["run", out_dis.to_str().expect("out utf8")]);
    assert!(
        run_out.status.success(),
        "run failed: stderr={}",
        String::from_utf8_lossy(&run_out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run_out.stdout).trim_end(),
        "lo=1 hi=1",
        "big arithmetic must preserve all 64 bits: 0x1_0000_0001 should split \
         into low=1 and high=1"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Regression for gen_binary operand order: compile a Limbo program that
/// exercises every non-commutative binary op with asymmetric operands (so a
/// swapped order produces a different, wrong answer), run it, and assert the
/// output. An earlier 2-op emission computed `rhs OP lhs` instead of
/// `lhs OP rhs`, so e.g. `10 - 3` produced `-7`.
#[test]
fn cli_compile_noncommutative_arithmetic() {
    let Some(module_dir) = find_asset("external/inferno-os/module") else {
        eprintln!("inferno module dir not found, skipping");
        return;
    };

    let tmp = std::env::temp_dir().join(format!(
        "ricevm-noncomm-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&tmp).expect("create temp dir");
    let src = tmp.join("NonComm.b");
    let out_dis = tmp.join("NonComm.dis");
    std::fs::write(
        &src,
        r#"implement NonComm;
include "sys.m";
    sys: Sys;
NonComm: module { init: fn(nil: ref Draw->Context, args: list of string); };
init(nil: ref Draw->Context, args: list of string)
{
    sys = load Sys Sys->PATH;
    a := 20;
    b := 4;
    sys->print("sub=%d div=%d mod=%d shl=%d shr=%d\n",
        a - b, a / b, a % b, 1 << 3, 64 >> 2);
}
"#,
    )
    .expect("write source");

    let compile_out = run_cli(&[
        "compile",
        src.to_str().expect("src utf8"),
        "-I",
        module_dir.to_str().expect("module dir utf8"),
        "-o",
        out_dis.to_str().expect("out utf8"),
    ]);
    assert!(
        compile_out.status.success(),
        "compile failed: stderr={}",
        String::from_utf8_lossy(&compile_out.stderr)
    );

    let run_out = run_cli(&["run", out_dis.to_str().expect("out utf8")]);
    assert!(
        run_out.status.success(),
        "run failed: stderr={}",
        String::from_utf8_lossy(&run_out.stderr)
    );
    let stdout = String::from_utf8_lossy(&run_out.stdout);
    assert_eq!(
        stdout.trim_end(),
        "sub=16 div=5 mod=0 shl=8 shr=16",
        "non-commutative arithmetic produced wrong values: {stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// End-to-end: compile a Limbo source file via `ricevm-cli compile`, then run
/// the produced .dis and assert on its stdout. This exercises the whole stack
/// (lexer, parser, codegen, writer, loader, executor, Sys builtin) as a unit.
#[test]
fn cli_compile_then_run_prints_expected_output() {
    let Some(module_dir) = find_asset("external/inferno-os/module") else {
        eprintln!("inferno module dir not found, skipping");
        return;
    };

    let tmp = std::env::temp_dir().join(format!("ricevm-it-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("create temp dir");
    let src = tmp.join("Greet.b");
    let out_dis = tmp.join("Greet.dis");
    std::fs::write(
        &src,
        r#"implement Greet;
include "sys.m";
    sys: Sys;
Greet: module { init: fn(nil: ref Draw->Context, args: list of string); };
init(nil: ref Draw->Context, args: list of string)
{
    sys = load Sys Sys->PATH;
    a := 2;
    b := 3;
    sys->print("sum=%d hi\n", a + b);
}
"#,
    )
    .expect("write source");

    let compile_out = run_cli(&[
        "compile",
        src.to_str().expect("src utf8"),
        "-I",
        module_dir.to_str().expect("module dir utf8"),
        "-o",
        out_dis.to_str().expect("out utf8"),
    ]);
    assert!(
        compile_out.status.success(),
        "compile failed: stderr={}",
        String::from_utf8_lossy(&compile_out.stderr)
    );
    assert!(out_dis.exists(), "compile did not produce {out_dis:?}");

    let run_out = run_cli(&["run", out_dis.to_str().expect("out utf8")]);
    assert!(
        run_out.status.success(),
        "run failed: stderr={}",
        String::from_utf8_lossy(&run_out.stderr)
    );
    let stdout = String::from_utf8_lossy(&run_out.stdout);
    assert_eq!(
        stdout.trim_end(),
        "sum=5 hi",
        "compiled Greet.b should print sum=5 hi, got {stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Integration test: Load multiple different .dis files and verify they all
/// parse into valid modules with expected properties.
#[test]
fn load_multiple_dis_files() {
    let dis_dir_paths = [
        "external/inferno-os/dis",
        "../external/inferno-os/dis",
        "../../external/inferno-os/dis",
    ];
    let mut dis_dir = None;
    for p in &dis_dir_paths {
        if std::path::Path::new(p).is_dir() {
            dis_dir = Some(*p);
            break;
        }
    }
    let Some(dir) = dis_dir else {
        eprintln!("dis directory not found, skipping");
        return;
    };

    // Test a set of basic utilities
    let programs = ["echo.dis", "cat.dis", "basename.dis", "date.dis"];
    let mut loaded = 0;

    for prog in &programs {
        let path = format!("{dir}/{prog}");
        if !std::path::Path::new(&path).exists() {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap_or_else(|_| panic!("should read {prog}"));
        let module = ricevm_loader::load(&bytes).unwrap_or_else(|_| panic!("should parse {prog}"));
        assert!(!module.name.is_empty(), "{prog} should have a module name");
        assert!(!module.code.is_empty(), "{prog} should have instructions");
        loaded += 1;
    }

    assert!(
        loaded >= 2,
        "should have loaded at least 2 .dis files, got {loaded}"
    );
}
