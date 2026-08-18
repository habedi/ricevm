//! Behavioral codegen tests: compile a Limbo snippet and *run* the resulting
//! Dis module on the RiceVM.
//!
//! The program under test reports its result by raising an exception whose
//! message encodes the computed values. An unhandled `raise` surfaces as
//! `ExecError::ThreadFault("unhandled exception: <msg>")`, which gives an
//! in-process observation channel without having to capture the guest's
//! stdout. Anything that silently miscompiles to 0 (or loops forever) is
//! therefore directly visible as a wrong — or missing — message.

use std::sync::mpsc;
use std::time::Duration;

/// Compile and run `src`, returning the message of the exception the program
/// raised. Runs on a worker thread with a wall-clock budget so that a
/// miscompiled loop (the classic symptom of a dropped `break`) fails the test
/// instead of hanging the suite forever.
fn run_src(src: &str) -> Result<String, String> {
    let owned = src.to_string();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let module = ricevm_limbo::compile(&owned, "test.b")?;
            match ricevm_execute::execute(&module) {
                Ok(()) => Err("program exited without raising a result".to_string()),
                Err(e) => Ok(e.to_string()),
            }
        })();
        let _ = tx.send(result);
    });
    match rx.recv_timeout(Duration::from_secs(20)) {
        Ok(r) => r,
        Err(_) => Err("timed out (probable infinite loop in generated code)".to_string()),
    }
}

/// Wrap a statement list in a minimal module and run it.
fn run_body(body: &str) -> Result<String, String> {
    run_src(&format!(
        "implement T;\ninit(nil: ref Draw->Context, nil: list of string)\n{{\n{body}\n}}\n"
    ))
}

/// Assert that the program raises exactly `expected`.
#[track_caller]
fn assert_raises(result: Result<String, String>, expected: &str) {
    let msg = result.unwrap_or_else(|e| panic!("program failed: {e}"));
    assert_eq!(
        msg,
        format!("thread exited with error: unhandled exception: {expected}"),
        "guest program produced the wrong result"
    );
}

// ── Finding 1: break / continue ─────────────────────────────────

/// `for(;;) { ...; break; }` must terminate. Before the fix `Stmt::Break` fell
/// into gen_stmt's `_ => Ok(())` catch-all and emitted nothing at all, turning
/// this into a genuine infinite loop.
#[test]
fn break_exits_infinite_for_loop() {
    let out = run_body(
        r#"
    i := 0;
    n := 0;
    for(;;) {
        i++;
        if(i >= 3)
            break;
        n++;
    }
    raise "i=" + string i + " n=" + string n;
"#,
    );
    assert_raises(out, "i=3 n=2");
}

/// `while(1) { if(done) break; }` must terminate.
#[test]
fn break_exits_while_loop() {
    let out = run_body(
        r#"
    i := 0;
    while(1) {
        i++;
        if(i == 4)
            break;
    }
    raise "i=" + string i;
"#,
    );
    assert_raises(out, "i=4");
}

/// `continue` in a `for` must jump to the post-statement, not skip it (which
/// would loop forever) and not fall through to the rest of the body.
#[test]
fn continue_in_for_runs_post_statement() {
    let out = run_body(
        r#"
    sum := 0;
    for(i := 0; i < 5; i++) {
        if(i == 2)
            continue;
        sum += i;
    }
    raise "sum=" + string sum;
"#,
    );
    assert_raises(out, "sum=8");
}

/// `continue` in a `while` re-evaluates the condition.
#[test]
fn continue_in_while_reevaluates_condition() {
    let out = run_body(
        r#"
    i := 0;
    sum := 0;
    while(i < 5) {
        i++;
        if(i == 3)
            continue;
        sum += i;
    }
    raise "sum=" + string sum;
"#,
    );
    assert_raises(out, "sum=12");
}

/// `break`/`continue` inside a `do ... while` loop.
#[test]
fn break_and_continue_in_do_while() {
    let out = run_body(
        r#"
    i := 0;
    sum := 0;
    do {
        i++;
        if(i == 2)
            continue;
        if(i == 5)
            break;
        sum += i;
    } while(i < 100);
    raise "i=" + string i + " sum=" + string sum;
"#,
    );
    assert_raises(out, "i=5 sum=8");
}

/// A labelled `break` must leave the labelled loop, not just the innermost one.
#[test]
fn labelled_break_exits_outer_loop() {
    let out = run_body(
        r#"
    n := 0;
    outer:
    for(i := 0; i < 3; i++) {
        for(j := 0; j < 3; j++) {
            if(j == 1)
                break outer;
            n++;
        }
    }
    raise "n=" + string n;
"#,
    );
    assert_raises(out, "n=1");
}

/// A labelled `continue` must continue the labelled loop.
#[test]
fn labelled_continue_targets_outer_loop() {
    let out = run_body(
        r#"
    n := 0;
    outer:
    for(i := 0; i < 3; i++) {
        for(j := 0; j < 3; j++) {
            if(j == 1)
                continue outer;
            n++;
        }
    }
    raise "n=" + string n;
"#,
    );
    assert_raises(out, "n=3");
}

/// `break` inside a `case` arm leaves the case statement (and *not* the
/// enclosing loop).
#[test]
fn break_inside_case_leaves_the_case() {
    let out = run_body(
        r#"
    n := 0;
    for(i := 0; i < 3; i++) {
        case i {
        1 =>
            n += 100;
            break;
            n += 1000;
        * =>
            n++;
        }
        n += 10;
    }
    raise "n=" + string n;
"#,
    );
    assert_raises(out, "n=132");
}

// ── Finding 2: module-level constants and variables ─────────────

/// `MAX: con 100;` used from a function body must load 100, not 0.
#[test]
fn module_constant_is_folded_at_use_site() {
    let out = run_src(
        r#"implement T;
MAX: con 100;
GREETING: con "hi";
init(nil: ref Draw->Context, nil: list of string)
{
    x := MAX;
    raise GREETING + " x=" + string x;
}
"#,
    );
    assert_raises(out, "hi x=100");
}

/// The `iota` idiom numbers the names of a `con` declaration from zero, and
/// restarts at the next declaration.
#[test]
fn iota_constants_are_numbered() {
    let out = run_src(
        r#"implement T;
Ared, Agreen, Ablue: con iota;
Bit0, Bit1, Bit2: con 1 << iota;
init(nil: ref Draw->Context, nil: list of string)
{
    raise "r=" + string Ared + " g=" + string Agreen + " b=" + string Ablue
        + " bits=" + string Bit0 + string Bit1 + string Bit2;
}
"#,
    );
    assert_raises(out, "r=0 g=1 b=2 bits=124");
}

/// Constants declared in the implemented module's own interface block are in
/// scope in the implementation.
#[test]
fn constant_from_module_block_is_in_scope() {
    let out = run_src(
        r#"implement T;
T: module {
    LIMIT: con 7;
    init: fn(nil: ref Draw->Context, args: list of string);
};
init(nil: ref Draw->Context, nil: list of string)
{
    raise "limit=" + string LIMIT;
}
"#,
    );
    assert_raises(out, "limit=7");
}

/// A constant expression built from other constants folds correctly.
#[test]
fn constant_expressions_fold() {
    let out = run_src(
        r#"implement T;
BASE: con 10;
DOUBLE: con BASE * 2;
FLAG: con 1 << 4;
init(nil: ref Draw->Context, nil: list of string)
{
    raise "d=" + string DOUBLE + " f=" + string FLAG;
}
"#,
    );
    assert_raises(out, "d=20 f=16");
}

/// A module-level variable assigned in one function must be visible in
/// another: it needs real MP-resident storage, not a per-function frame slot.
#[test]
fn module_variable_is_shared_between_functions() {
    let out = run_src(
        r#"implement T;
counter: int;
bump()
{
    counter = counter + 5;
}
init(nil: ref Draw->Context, nil: list of string)
{
    counter = 1;
    bump();
    bump();
    raise "counter=" + string counter;
}
"#,
    );
    assert_raises(out, "counter=11");
}

/// Module-level variables support the same read/modify forms as locals.
#[test]
fn module_variable_supports_compound_assign_and_incdec() {
    let out = run_src(
        r#"implement T;
total: int;
name: string;
init(nil: ref Draw->Context, nil: list of string)
{
    total = 1;
    total += 4;
    total++;
    name = "n";
    name += "x";
    raise name + "=" + string total;
}
"#,
    );
    assert_raises(out, "nx=6");
}

/// Constant module-level initialisers are laid down in the data section.
#[test]
fn module_variable_constant_initialiser_is_preloaded() {
    let out = run_src(
        r#"implement T;
greeting := "hi";
count := 7;
init(nil: ref Draw->Context, nil: list of string)
{
    raise greeting + "=" + string count;
}
"#,
    );
    assert_raises(out, "hi=7");
}

/// A module-level initialiser that is not a compile-time constant runs at the
/// top of the entry function, before any other code in the module.
#[test]
fn module_variable_computed_initialiser_runs_before_init_body() {
    let out = run_src(
        r#"implement T;
SIZE: con 4;
buf := array[SIZE] of int;
init(nil: ref Draw->Context, nil: list of string)
{
    buf[0] = 5;
    raise "len=" + string len buf + " v=" + string buf[0];
}
"#,
    );
    assert_raises(out, "len=4 v=5");
}

// ── Finding 3: `x++` / `x--` in value context ───────────────────

/// `y := x++` must yield the *old* value of x and still increment x.
#[test]
fn post_increment_in_value_context() {
    let out = run_body(
        r#"
    x := 5;
    y := x++;
    z := x--;
    raise "x=" + string x + " y=" + string y + " z=" + string z;
"#,
    );
    assert_raises(out, "x=5 y=5 z=6");
}

/// `a[i++]` must index with the old i and still advance i.
#[test]
fn post_increment_inside_index_expression() {
    let out = run_body(
        r#"
    a := array[3] of int;
    a[0] = 10;
    a[1] = 20;
    a[2] = 30;
    i := 0;
    v := a[i++];
    w := a[i++];
    raise "v=" + string v + " w=" + string w + " i=" + string i;
"#,
    );
    assert_raises(out, "v=10 w=20 i=2");
}

/// Post-increment of a module-level variable in value context.
#[test]
fn post_increment_of_module_variable() {
    let out = run_src(
        r#"implement T;
seq: int;
init(nil: ref Draw->Context, nil: list of string)
{
    seq = 3;
    a := seq++;
    raise "a=" + string a + " seq=" + string seq;
}
"#,
    );
    assert_raises(out, "a=3 seq=4");
}

/// `++` on an array element and on an ADT field updates storage in place.
#[test]
fn increment_of_array_element_and_adt_field() {
    let out = run_src(
        r#"implement T;
T: module {
    init: fn(nil: ref Draw->Context, args: list of string);
    P: adt { n: int; };
};
init(nil: ref Draw->Context, nil: list of string)
{
    a := array[2] of int;
    a[0] = 5;
    a[0]++;
    old := a[0]++;
    p := ref P(1);
    p.n++;
    raise "a=" + string a[0] + " old=" + string old + " n=" + string p.n;
}
"#,
    );
    assert_raises(out, "a=7 old=6 n=2");
}

/// `++` on a string character reads, bumps and writes the character back.
#[test]
fn increment_of_string_character() {
    let out = run_body(
        r#"
    s := "abc";
    s[0]++;
    raise "s=" + s;
"#,
    );
    assert_raises(out, "s=bbc");
}

/// A constant too wide for the 30-bit Dis immediate encoding must travel
/// through the data section instead of being truncated or rejected.
#[test]
fn constant_wider_than_the_immediate_encoding() {
    let out = run_src(
        r#"implement T;
LIMIT: con 16r7fffffff;
init(nil: ref Draw->Context, nil: list of string)
{
    raise "v=" + string LIMIT;
}
"#,
    );
    assert_raises(out, "v=2147483647");
}

// ── Finding 4: `case` range patterns ────────────────────────────

/// A value below the range must fall through to the next arm. Before the fix
/// the "skip if val < lo" branch was never patched and jumped to PC 0,
/// re-entering the function from its entry point.
#[test]
fn case_range_below_low_bound_falls_through() {
    let out = run_body(
        r#"
    x := 0;
    r := 9;
    case x {
    1 to 10 =>
        r = 1;
    * =>
        r = 2;
    }
    raise "r=" + string r;
"#,
    );
    assert_raises(out, "r=2");
}

/// A value inside the range still selects the arm, and one above it does not.
#[test]
fn case_range_matches_inside_and_not_above() {
    let out = run_body(
        r#"
    lo := 9;
    hi := 9;
    case 5 {
    1 to 10 =>
        lo = 1;
    * =>
        lo = 2;
    }
    case 11 {
    1 to 10 =>
        hi = 1;
    * =>
        hi = 2;
    }
    raise "lo=" + string lo + " hi=" + string hi;
"#,
    );
    assert_raises(out, "lo=1 hi=2");
}

// ── Finding 5: slot sizing for `:=` / `=` in expression position ─

/// `(r := 3.25)` inside an expression must allocate an 8-byte slot for r.
/// With the hardcoded 4-byte slot the real value spilled into the following
/// temp and the comparison read garbage.
#[test]
fn decl_assign_in_expression_sizes_real_slot() {
    let out = run_body(
        r#"
    n := 0;
    if((r := 3.25) > 0.0)
        n = 1;
    raise "n=" + string n + " r=" + string int (r * 4.0);
"#,
    );
    assert_raises(out, "n=1 r=13");
}

/// The `=`-to-a-fresh-name fallback must size the new slot by the value's
/// kind too, or the 8-byte store clobbers the next variable's slot.
#[test]
fn assign_fallback_sizes_real_slot() {
    let out = run_body(
        r#"
    x = 2.5;
    y = 7;
    raise "x=" + string int (x * 2.0) + " y=" + string y;
"#,
    );
    assert_raises(out, "x=5 y=7");
}

// ── Cross-cutting: the frame must still be big enough ───────────

/// A function with many locals still gets a frame that fits them, and calls
/// between differently-sized frames keep working (guards the per-function
/// frame-size reset that goes with the entry_type fix).
#[test]
fn functions_with_different_frame_sizes_interoperate() {
    let out = run_src(
        r#"implement T;
small(a: int): int
{
    return a + 1;
}
big_frame(a: int): int
{
    b := a * 2;
    c := b * 2;
    d := c * 2;
    e := d + b;
    f := e + c;
    g := f + d;
    h := g + e;
    return h;
}
init(nil: ref Draw->Context, nil: list of string)
{
    x := small(1);
    y := big_frame(1);
    raise "x=" + string x + " y=" + string y;
}
"#,
    );
    assert_raises(out, "x=2 y=32");
}

// ── import declarations ─────────────────────────────────────────

/// `NAME: import modvar;` brings a module constant into unqualified scope.
/// Before the fix codegen had no `Decl::Import` arm at all, so the name never
/// entered scope and every use site failed with "undefined identifier".
#[test]
fn imported_constant_folds_to_its_value() {
    let out = run_src(
        r#"implement T;
M: module {
    UTFmax: con 4;
};
m: M;
UTFmax: import m;
init(nil: ref Draw->Context, nil: list of string)
{
    n := 3;
    raise "v=" + string (n * UTFmax + 1);
}
"#,
    );
    assert_raises(out, "v=13");
}

/// The operand of `import` may also be the module *type* name rather than a
/// module variable — `Next, Down, Skip, Quit: import Fs;` is the idiom used
/// throughout appl/alphabet.
#[test]
fn imported_constant_from_module_type_name() {
    let out = run_src(
        r#"implement T;
Fs: module {
    Next, Down, Skip, Quit: con iota;
};
Next, Down, Skip, Quit: import Fs;
init(nil: ref Draw->Context, nil: list of string)
{
    raise "v=" + string Next + string Down + string Skip + string Quit;
}
"#,
    );
    assert_raises(out, "v=0123");
}

/// An imported constant must be usable inside another constant expression,
/// exactly like a locally declared one.
#[test]
fn imported_constant_folds_inside_a_constant_expression() {
    let out = run_src(
        r#"implement T;
M: module {
    Udphdrlen: con 12;
};
m: M;
Udphdrlen: import m;
Udphdrsize: con Udphdrlen + 8;
init(nil: ref Draw->Context, nil: list of string)
{
    raise "v=" + string Udphdrsize;
}
"#,
    );
    assert_raises(out, "v=20");
}

/// Importing a name the module does not declare is a hard error that names
/// both the module and the member — never a silent zero.
#[test]
fn importing_a_name_the_module_lacks_is_an_error() {
    let err = run_src(
        r#"implement T;
M: module {
    Real: con 1;
};
m: M;
Bogus: import m;
init(nil: ref Draw->Context, nil: list of string)
{
    raise "v=" + string Bogus;
}
"#,
    )
    .unwrap_err();
    assert!(
        err.contains("Bogus") && err.contains("M"),
        "error should name the missing member and its module, got: {err}"
    );
}

/// A function-body `import` binds names too; the parser used to drop it on the
/// floor with the comment "import handled as side effect", and there was no
/// such side effect.
#[test]
fn import_in_statement_position_binds_names() {
    let out = run_src(
        r#"implement T;
M: module {
    Bufsize: con 7;
};
m: M;
init(nil: ref Draw->Context, nil: list of string)
{
    Bufsize: import m;
    raise "v=" + string (Bufsize * 3);
}
"#,
    );
    assert_raises(out, "v=21");
}

/// An imported function must reach the module it came from. `sprint` imported
/// from `sys` has to make the same cross-module call `sys->sprint(...)` makes,
/// so the $Sys builtin actually formats the string.
#[test]
fn imported_sys_function_runs_as_a_cross_module_call() {
    let out = run_src(
        r#"implement T;
include "sys.m";
Sys: module {
    PATH: con "$Sys";
    sprint: fn(s: string): string;
};
sys: Sys;
sprint: import sys;
init(nil: ref Draw->Context, nil: list of string)
{
    sys = load Sys Sys->PATH;
    raise sprint("v=%d", 40 + 2);
}
"#,
    );
    assert_raises(out, "v=42");
}

// ── module-level initialisers in a module with no `init` ────────

/// A module-level array initialiser is a *constant* in Limbo: the reference
/// compiler accepts it (`initable`, nodes.c:168-186), writes it into the `.dis`
/// data section (`disvar`/`disdatum`, dis.c:138) and the loader materialises it
/// when it builds the module's MP — no code runs. So it needs no `init`
/// function, and reading it back must give the written elements.
#[test]
fn module_level_array_literal_is_materialised_from_the_data_section() {
    let out = run_src(
        r#"implement T;
tab := array[] of {11, 22, 33};
init(nil: ref Draw->Context, nil: list of string)
{
    raise "v=" + string tab[0] + "," + string tab[2] + " n=" + string len tab;
}
"#,
    );
    assert_raises(out, "v=11,33 n=3");
}

/// A sized allocation with no elements is initable too, and its elements read
/// back as zero.
#[test]
fn module_level_sized_array_needs_no_init_function() {
    let out = run_src(
        r#"implement T;
Cache: module {
    lookup: fn(): int;
};
tab := array[4] of int;
lookup(): int
{
    return len tab + tab[3];
}
init(nil: ref Draw->Context, nil: list of string)
{
    raise "v=" + string lookup();
}
"#,
    );
    assert_raises(out, "v=4");
}

/// An array of strings lands in the data section as well.
#[test]
fn module_level_string_array_is_materialised_from_the_data_section() {
    let out = run_src(
        r#"implement T;
names := array[] of {"concat", "join"};
init(nil: ref Draw->Context, nil: list of string)
{
    raise "v=" + names[1] + names[0];
}
"#,
    );
    assert_raises(out, "v=joinconcat");
}

/// The module in the corpus that motivated this has *no* `init` at all, which
/// used to be rejected outright. It must now compile.
#[test]
fn module_without_init_accepts_a_constant_array_initialiser() {
    let module = ricevm_limbo::compile(
        r#"implement GenCP;
GenCP: module {
    cstab: array of int;
};
cstab := array[] of {16r00, 16r01, 16r02};
"#,
        "test.b",
    )
    .expect("a constant array initialiser needs no init function");
    assert!(
        module
            .data
            .iter()
            .any(|d| matches!(d, ricevm_core::DataItem::Array { length: 3, .. })),
        "the initialiser must be emitted into the data section"
    );
}

/// A genuinely non-constant initialiser in a module with no `init` stays a
/// hard error — the reference rejects it too ("x's initializer, f(), is not a
/// constant expression", nodes.c:196).
#[test]
fn module_without_init_still_rejects_a_computed_initialiser() {
    let err = ricevm_limbo::compile(
        r#"implement T;
T: module {
    f: fn(): int;
};
x := f();
f(): int
{
    return 1;
}
"#,
        "test.b",
    )
    .unwrap_err();
    assert!(err.contains("init"), "unexpected error: {err}");
}

/// `array[n] of {..}` has length `n`, not the number of elements written. The
/// parser used to throw the declared size away.
#[test]
fn sized_array_literal_keeps_its_declared_length() {
    let out = run_src(
        r#"implement T;
u := array[6] of {byte 3, byte 4};
init(nil: ref Draw->Context, nil: list of string)
{
    raise "n=" + string len u + " u1=" + string int u[1] + " u5=" + string int u[5];
}
"#,
    );
    assert_raises(out, "n=6 u1=4 u5=0");
}

/// Array-literal elements may name the index they initialise, including a
/// `* => v` default for everything else. Dropping the selectors packed the
/// values in declaration order — silently the wrong table.
#[test]
fn indexed_array_literal_places_elements_at_their_index() {
    let out = run_src(
        r#"implement T;
Naughty: con 9;
t := array[8] of {'a' - 'a' + 1 => byte 1, 3 => byte 2, * => byte Naughty};
init(nil: ref Draw->Context, nil: list of string)
{
    raise "n=" + string len t + " " + string int t[0] + string int t[1]
        + string int t[2] + string int t[3];
}
"#,
    );
    assert_raises(out, "n=8 9192");
}

/// `lo to hi => v` fills the whole range.
#[test]
fn ranged_array_literal_fills_the_range() {
    let out = run_src(
        r#"implement T;
t := array[5] of {1 to 3 => 7};
init(nil: ref Draw->Context, nil: list of string)
{
    raise "v=" + string t[0] + string t[1] + string t[3] + string t[4];
}
"#,
    );
    assert_raises(out, "v=0770");
}

// ── qualified constants in constant expressions ─────────────────

/// `Udphdrsize: con IP->Udphdrlen + 8;` — a `Mod->NAME` reference has to fold
/// inside another constant's expression, not just at ordinary use sites.
#[test]
fn qualified_constant_folds_inside_a_constant_expression() {
    let out = run_src(
        r#"implement T;
IP: module {
    Udphdrlen: con 12;
};
Udphdrsize: con IP->Udphdrlen + 8;
init(nil: ref Draw->Context, nil: list of string)
{
    raise "v=" + string Udphdrsize;
}
"#,
    );
    assert_raises(out, "v=20");
}

/// `len` of a constant string is itself a constant, so it may appear in a
/// `con` declaration (appl/cmd/auth/aescbc.b does exactly this).
#[test]
fn len_of_a_constant_string_folds() {
    let out = run_src(
        r#"implement T;
Checkpat: con "AESCBC";
Checklen: con len Checkpat;
init(nil: ref Draw->Context, nil: list of string)
{
    raise "v=" + string Checklen;
}
"#,
    );
    assert_raises(out, "v=6");
}

/// An imported constant whose value the compiler cannot represent (an
/// ADT- or tuple-valued `con`) must say so at the use site. It must not be
/// reported as a missing member — the module does declare it — and above all
/// it must not quietly become zero.
#[test]
fn imported_unrepresentable_constant_is_reported_at_its_use_site() {
    let err = ricevm_limbo::compile(
        r#"implement T;
Fs: module {
    Nilentry: con (nil, nil, 0);
};
fs: Fs;
Nilentry: import fs;
init(nil: ref Draw->Context, nil: list of string)
{
    x := Nilentry;
}
"#,
        "test.b",
    )
    .unwrap_err();
    assert!(
        err.contains("Nilentry") && !err.contains("not a member"),
        "unexpected error: {err}"
    );
}

// ── cross-module calls through any module handle ────────────────

/// A call through a module handle that is not spelled `sys` must actually
/// reach the module. Before the fix only the literal name `sys` was routed to
/// the cross-module call path; every other handle fell through to `Movw $0`,
/// so this program silently saw `0` instead of the module's answer.
#[test]
fn call_through_a_non_sys_module_handle_reaches_the_module() {
    let out = run_src(
        r#"implement T;
Sys: module {
    PATH: con "$Sys";
    print: fn(s: string, *): int;
};
s: Sys;
init(nil: ref Draw->Context, nil: list of string)
{
    s = load Sys Sys->PATH;
    n := s->print("hi\n");
    raise "n=" + string n;
}
"#,
    );
    assert_raises(out, "n=3");
}

/// The unqualified spelling of the same call — `import` routes it through the
/// very same path, so a handle that is not `sys` must work there too.
#[test]
fn imported_call_through_a_non_sys_handle_reaches_the_module() {
    let out = run_src(
        r#"implement T;
Sys: module {
    PATH: con "$Sys";
    print: fn(s: string, *): int;
};
s: Sys;
print: import s;
init(nil: ref Draw->Context, nil: list of string)
{
    s = load Sys Sys->PATH;
    n := print("hi\n");
    raise "n=" + string n;
}
"#,
    );
    assert_raises(out, "n=3");
}

/// A call through a name that is not a module variable must fail loudly. The
/// old code emitted nothing at all for it, so the program ran on with a zero
/// where the module's answer belonged.
#[test]
fn call_through_an_unknown_module_handle_is_an_error() {
    let err = ricevm_limbo::compile(
        r#"implement T;
init(nil: ref Draw->Context, nil: list of string)
{
    bufio->open("x", 0);
}
"#,
        "test.b",
    )
    .unwrap_err();
    assert!(
        err.contains("bufio"),
        "the error must name the unresolved handle: {err}"
    );
}

/// Calling a function through an interface *name* has no module reference to
/// call through; the reference compiler rejects it (typecheck.c:1459).
#[test]
fn call_through_an_interface_name_is_an_error() {
    let err = ricevm_limbo::compile(
        r#"implement T;
Bufio: module {
    open: fn(name: string, mode: int): int;
};
init(nil: ref Draw->Context, nil: list of string)
{
    Bufio->open("x", 0);
}
"#,
        "test.b",
    )
    .unwrap_err();
    assert!(err.contains("module interface"), "unexpected error: {err}");
}

/// `Mod->NAME` for a name the interface does not declare used to become
/// `Movw $0`. A wrong constant is worse than no program.
#[test]
fn unknown_qualified_name_is_an_error_not_a_zero() {
    let err = ricevm_limbo::compile(
        r#"implement T;
Fs: module {
    Real: con 7;
};
init(nil: ref Draw->Context, nil: list of string)
{
    x := Fs->Imaginary;
    raise "x=" + string x;
}
"#,
        "test.b",
    )
    .unwrap_err();
    assert!(
        err.contains("Imaginary"),
        "the error must name the missing member: {err}"
    );
}

/// `Mod->PATH` must be the module's declared `PATH`, not a `$Mod` guess. A
/// `load` of the wrong path silently yields nil, and every call through the
/// handle then faults far from the cause.
#[test]
fn qualified_path_uses_the_declared_constant() {
    let module = ricevm_limbo::compile(
        r#"implement T;
Bufio: module {
    PATH: con "/dis/lib/bufio.dis";
};
b: Bufio;
init(nil: ref Draw->Context, nil: list of string)
{
    b = load Bufio Bufio->PATH;
}
"#,
        "test.b",
    )
    .expect("should compile");
    let strings: Vec<&str> = module
        .data
        .iter()
        .filter_map(|d| match d {
            ricevm_core::DataItem::String { value, .. } => Some(value.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        strings.contains(&"/dis/lib/bufio.dis"),
        "the declared PATH must reach the data section, got {strings:?}"
    );
}

// ── `implement X` brings X's own interface into scope ────────────

/// Compile `src` with `interfaces` written into a temporary include
/// directory, then run it. `implement X; include "x.m";` is the shape every
/// real Limbo program has, so testing that scope rule needs a real `.m`.
fn run_with_includes(src: &str, interfaces: &[(&str, &str)]) -> Result<String, String> {
    let dir = std::env::temp_dir().join(format!(
        "ricevm-limbo-inc-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (name, body) in interfaces {
        std::fs::write(dir.join(name), body).map_err(|e| e.to_string())?;
    }
    let opts = ricevm_limbo::CompileOptions {
        include_paths: vec![dir.to_string_lossy().into_owned()],
    };
    let module = ricevm_limbo::compile_with_options(src, "test.b", &opts)?;
    let _ = std::fs::remove_dir_all(&dir);
    match ricevm_execute::execute(&module) {
        Ok(()) => Err("program exited without raising a result".to_string()),
        Err(e) => Ok(e.to_string()),
    }
}

/// A constant declared in the interface this file implements is in scope
/// unqualified. Before the fix it was reported as an undefined identifier,
/// which is why `STATFIXLEN` and friends failed across the corpus.
#[test]
fn implement_brings_its_own_interface_constants_into_scope() {
    let out = run_with_includes(
        r#"implement Styx;
include "styx.m";
init(nil: ref Draw->Context, nil: list of string)
{
    raise "v=" + string STATFIXLEN;
}
"#,
        &[(
            "styx.m",
            r#"Styx: module {
    STATFIXLEN: con 49;
    init: fn(ctxt: ref Draw->Context, argv: list of string);
};
"#,
        )],
    );
    assert_raises(out, "v=49");
}

/// An ADT declared in the implemented interface is in scope unqualified, and
/// its *layout* comes across too — a record built from a guessed layout reads
/// its own fields back wrong.
#[test]
fn implement_brings_its_own_interface_adts_into_scope() {
    let out = run_with_includes(
        r#"implement Styxservers;
include "styxservers.m";
init(nil: ref Draw->Context, nil: list of string)
{
    x := ref Xfid(7, 9);
    raise "v=" + string (x.fid + x.mode);
}
"#,
        &[(
            "styxservers.m",
            r#"Styxservers: module {
    Xfid: adt {
        fid: int;
        mode: int;
    };
    init: fn(ctxt: ref Draw->Context, argv: list of string);
};
"#,
        )],
    );
    assert_raises(out, "v=16");
}

/// An ADT reached through another module's interface must use that module's
/// real field layout, not a positional guess.
#[test]
fn qualified_interface_adt_has_a_real_layout() {
    let out = run_with_includes(
        r#"implement T;
include "bufio.m";
init(nil: ref Draw->Context, nil: list of string)
{
    b := ref Bufio->Iobuf(3, 5, 11);
    raise "v=" + string (b.fid + b.size + b.mode);
}
"#,
        &[(
            "bufio.m",
            r#"Bufio: module {
    Iobuf: adt {
        fid: int;
        size: int;
        mode: int;
    };
};
"#,
        )],
    );
    assert_raises(out, "v=19");
}

/// Calling a function that is not declared anywhere used to emit nothing at
/// all, so the program ran on as if the call had happened.
#[test]
fn calling_an_undefined_function_is_an_error() {
    let err = ricevm_limbo::compile(
        r#"implement T;
init(nil: ref Draw->Context, nil: list of string)
{
    nosuchfunction(1, 2);
}
"#,
        "test.b",
    )
    .unwrap_err();
    assert!(err.contains("nosuchfunction"), "unexpected error: {err}");
}

// ── type descriptors ────────────────────────────────────────────

/// Read back the type descriptor an instruction refers to.
fn descriptor_of(module: &ricevm_core::Module, idx: i32) -> &ricevm_core::TypeDescriptor {
    module
        .types
        .get(idx as usize)
        .unwrap_or_else(|| panic!("type index {idx} is outside the descriptor table"))
}

/// `new` used to hardcode type index 1 — the 48-byte sys call frame — so any
/// ADT bigger than 48 bytes was silently truncated, and the collector traced
/// every record at the frame's pointer offsets instead of the record's own.
#[test]
fn record_allocation_uses_the_adts_own_descriptor() {
    let module = ricevm_limbo::compile(
        r#"implement T;
Big: adt {
    a: string;
    b: int;
    c: string;
    d: int;
    e: int;
    f: int;
    g: int;
    h: int;
    i: int;
    j: int;
    k: int;
    l: int;
    m: int;
    n: int;
};
init(nil: ref Draw->Context, nil: list of string)
{
    x := ref Big;
}
"#,
        "test.b",
    )
    .expect("should compile");
    let new = module
        .code
        .iter()
        .find(|i| i.opcode == ricevm_core::Opcode::New)
        .expect("a `ref Adt` must allocate a record");
    let td = descriptor_of(&module, new.source.register1);
    // 14 four-byte fields.
    assert_eq!(td.size, 56, "descriptor must be the ADT's own size");
    // Pointers at byte offsets 0 and 8 -> words 0 and 2 -> MSB-first bits
    // 0x80 and 0x20 in the first map byte.
    assert_eq!(
        td.pointer_map.bytes,
        vec![0xA0, 0x00],
        "pointer map must mark the string fields, MSB first"
    );
    assert_eq!(td.pointer_count, 2);
}

/// A record's fields must survive a round trip through the heap. With the
/// 48-byte descriptor, field 13 of this ADT lived past the end of the object.
#[test]
fn record_larger_than_the_old_fixed_size_keeps_its_fields() {
    let out = run_src(
        r#"implement T;
Big: adt {
    a, b, c, d, e, f, g, h, i, j, k, l, m, n: int;
};
init(nil: ref Draw->Context, nil: list of string)
{
    x := ref Big;
    x.a = 1;
    x.n = 42;
    raise "v=" + string x.n + "," + string x.a;
}
"#,
    );
    assert_raises(out, "v=42,1");
}

/// `newa` used type descriptor 0 — 16-byte elements with a pointer at offset
/// 0 — for every runtime array. A `byte` array was four times too big and the
/// collector read a "pointer" out of every fourth element.
#[test]
fn array_allocation_uses_an_element_sized_descriptor() {
    let module = ricevm_limbo::compile(
        r#"implement T;
init(nil: ref Draw->Context, nil: list of string)
{
    b := array[4] of byte;
    w := array[4] of int;
    s := array[4] of string;
}
"#,
        "test.b",
    )
    .expect("should compile");
    let newas: Vec<_> = module
        .code
        .iter()
        .filter(|i| i.opcode == ricevm_core::Opcode::Newa)
        .collect();
    assert_eq!(newas.len(), 3, "one Newa per array");
    let sizes: Vec<i32> = newas
        .iter()
        .map(|i| descriptor_of(&module, i.middle.register1).size)
        .collect();
    assert_eq!(sizes, vec![1, 4, 4], "byte/int/string element widths");
    let byte_td = descriptor_of(&module, newas[0].middle.register1);
    assert_eq!(byte_td.pointer_count, 0, "bytes are not pointers");
    let str_td = descriptor_of(&module, newas[2].middle.register1);
    assert_eq!(str_td.pointer_count, 1, "string elements are pointers");
    assert_eq!(str_td.pointer_map.bytes, vec![0x80]);
}

/// A byte array must hold `len` bytes, not `len` 16-byte slots, and index by
/// one byte per element.
#[test]
fn runtime_byte_array_indexes_by_one_byte() {
    let out = run_src(
        r#"implement T;
init(nil: ref Draw->Context, nil: list of string)
{
    b := array[4] of byte;
    b[0] = byte 7;
    b[3] = byte 9;
    raise "v=" + string int b[0] + "," + string int b[3] + ",n=" + string len b;
}
"#,
    );
    assert_raises(out, "v=7,9,n=4");
}

/// Pointer-map bits are most-significant-bit first, as `types.c` in the
/// reference compiler writes them: word `n` of the record is bit
/// `1 << (7 - n % 8)` of byte `n / 8`. A pointer past the eighth word is what
/// tells the two bit orders apart.
#[test]
fn pointer_map_bits_are_most_significant_bit_first() {
    let module = ricevm_limbo::compile(
        r#"implement T;
Wide: adt {
    a, b, c, d, e, f, g, h, i: int;
    p: string;
};
init(nil: ref Draw->Context, nil: list of string)
{
    x := ref Wide;
}
"#,
        "test.b",
    )
    .expect("should compile");
    let new = module
        .code
        .iter()
        .find(|i| i.opcode == ricevm_core::Opcode::New)
        .expect("a `ref Adt` must allocate a record");
    let td = descriptor_of(&module, new.source.register1);
    assert_eq!(td.size, 40);
    // The pointer is word 9: byte 1, bit 1 << (7 - 1) = 0x40.
    assert_eq!(
        td.pointer_map.bytes,
        vec![0x00, 0x40],
        "LSB-first would have produced 0x02 in byte 1"
    );
    assert_eq!(td.pointer_count, 1);
}

/// A `big` field is 8-byte aligned, so it occupies two map words and pushes
/// the fields after it along. The descriptor has to agree with the offsets
/// the field writes use, or the collector reads pointers out of halves of a
/// 64-bit integer.
#[test]
fn descriptor_agrees_with_eight_byte_field_alignment() {
    let module = ricevm_limbo::compile(
        r#"implement T;
Mixed: adt {
    n: int;
    v: big;
    s: string;
};
init(nil: ref Draw->Context, nil: list of string)
{
    x := ref Mixed;
}
"#,
        "test.b",
    )
    .expect("should compile");
    let new = module
        .code
        .iter()
        .find(|i| i.opcode == ricevm_core::Opcode::New)
        .expect("a `ref Adt` must allocate a record");
    let td = descriptor_of(&module, new.source.register1);
    // n at 0, v at 8 (aligned), s at 16.
    assert_eq!(td.size, 20);
    assert_eq!(td.pointer_map.bytes, vec![0x08], "the string is word 4");
    assert_eq!(td.pointer_count, 1);
}

// ── ADT function members ────────────────────────────────────────

/// `p.sum(5)` on an ADT this module declares is a call to `Point.sum` with
/// `p` supplied as the `self` parameter. It used to compile to nothing at
/// all in statement position, and to a zero in value position.
#[test]
fn adt_function_member_is_called_with_self() {
    let out = run_src(
        r#"implement T;
Point: adt {
    x, y: int;
    sum: fn(p: self ref Point, k: int): int;
};
Point.sum(p: self ref Point, k: int): int
{
    return p.x + p.y + k;
}
init(nil: ref Draw->Context, nil: list of string)
{
    p := ref Point(3, 4);
    raise "v=" + string p.sum(5);
}
"#,
    );
    assert_raises(out, "v=12");
}

/// A function member declared without `self` is called through the ADT name.
#[test]
fn adt_function_member_without_self_is_called_through_the_type() {
    let out = run_src(
        r#"implement T;
Point: adt {
    x, y: int;
    make: fn(k: int): int;
};
Point.make(k: int): int
{
    return k * 3;
}
init(nil: ref Draw->Context, nil: list of string)
{
    raise "v=" + string Point.make(7);
}
"#,
    );
    assert_raises(out, "v=21");
}

/// An ADT that belongs to another module is implemented by that module, so
/// its function members are reached by a cross-module call. The .dis export
/// for one is named `Adt.method`, which is what the import entry has to say.
#[test]
fn foreign_adt_function_member_is_a_cross_module_call() {
    let module = ricevm_limbo::compile(
        r#"implement T;
Bufio: module {
    PATH: con "/dis/lib/bufio.dis";
    Iobuf: adt {
        fd: int;
        gets: fn(b: self ref Iobuf, sep: int): string;
    };
    open: fn(name: string, mode: int): ref Iobuf;
};
bufio: Bufio;
init(nil: ref Draw->Context, nil: list of string)
{
    bufio = load Bufio Bufio->PATH;
    b := bufio->open("x", 0);
    s := b.gets('\n');
}
"#,
        "test.b",
    )
    .expect("should compile");
    let names: Vec<&str> = module
        .imports
        .iter()
        .flat_map(|m| m.functions.iter().map(|f| f.name.as_str()))
        .collect();
    assert!(
        names.contains(&"Iobuf.gets"),
        "the import entry must name the ADT method as the .dis export does, got {names:?}"
    );
    assert!(
        module
            .code
            .iter()
            .filter(|i| i.opcode == ricevm_core::Opcode::Mcall)
            .count()
            >= 2,
        "both `bufio->open` and `b.gets` must be cross-module calls"
    );
}

/// `b: self ref Iobuf` declares one parameter of type `ref Iobuf`, not a
/// double reference. Getting that wrong loses the ADT the method belongs to.
#[test]
fn self_parameter_keeps_its_declared_type() {
    let out = run_src(
        r#"implement T;
Point: adt {
    x: int;
    get: fn(p: self ref Point): int;
};
Point.get(p: self ref Point): int
{
    return p.x;
}
init(nil: ref Draw->Context, nil: list of string)
{
    p := ref Point(11);
    raise "v=" + string p.get();
}
"#,
    );
    assert_raises(out, "v=11");
}

// ── array literals outside a declaration ────────────────────────

/// `a := array[] of {1, 2, 3}` inside a function used to compile to `Movw $0`
/// — the local silently became nil, and every element read faulted or
/// answered zero.
#[test]
fn array_literal_in_an_expression_builds_a_real_array() {
    let out = run_src(
        r#"implement T;
init(nil: ref Draw->Context, nil: list of string)
{
    a := array[] of {1, 2, 3};
    raise "v=" + string a[0] + string a[1] + string a[2] + ",n=" + string len a;
}
"#,
    );
    assert_raises(out, "v=123,n=3");
}

/// A declared length wins over the number of elements, and an element may
/// name the index it initialises.
#[test]
fn array_literal_honours_its_length_and_indices() {
    let out = run_src(
        r#"implement T;
init(nil: ref Draw->Context, nil: list of string)
{
    b := array[4] of {2 => 7, 0 => 5};
    raise "v=" + string b[0] + string b[1] + string b[2] + ",n=" + string len b;
}
"#,
    );
    assert_raises(out, "v=507,n=4");
}

/// String elements are heap pointers and must be stored with the
/// ref-counting move, not a raw word copy.
#[test]
fn array_literal_of_strings_holds_its_strings() {
    let out = run_src(
        r#"implement T;
init(nil: ref Draw->Context, nil: list of string)
{
    a := array[] of {"ab", "cd"};
    raise "v=" + a[0] + a[1];
}
"#,
    );
    assert_raises(out, "v=abcd");
}

/// `* => v` fills every slot the other elements do not name, including in a
/// literal built at run time.
#[test]
fn array_literal_wildcard_default_fills_the_rest() {
    let out = run_src(
        r#"implement T;
init(nil: ref Draw->Context, nil: list of string)
{
    n := 4;
    a := array[n] of {1 => 9, * => 2};
    raise "v=" + string a[0] + string a[1] + string a[2] + string a[3];
}
"#,
    );
    assert_raises(out, "v=2922");
}

// ── Tuple values ────────────────────────────────────────────────

/// The simplest tuple value: a literal destructured straight into locals.
#[test]
fn tuple_literal_binds_every_field() {
    let out = run_body(
        r#"
    (a, b) := (7, 9);
    raise "a=" + string a + " b=" + string b;
"#,
    );
    assert_raises(out, "a=7 b=9");
}

/// `string` must choose its conversion from the operand's width. Every operand
/// went through `Cvtwc`, which reads only the low four bytes, so a `big` came
/// back truncated and a `real` came back as 0.
#[test]
fn string_conversion_matches_the_operand_width() {
    let out = run_body(
        r#"
    b := 5000000000;
    r := 2.5;
    i := 42;
    raise "b=" + string b + " r=" + string r + " i=" + string i;
"#,
    );
    assert_raises(out, "b=5000000000 r=2.5 i=42");
}

/// A tuple held in a local, read back through `.t0`/`.t1`.
#[test]
fn tuple_local_is_read_through_numbered_fields() {
    let out = run_body(
        r#"
    t := (7, 9);
    raise "t=" + string t.t0 + "," + string t.t1;
"#,
    );
    assert_raises(out, "t=7,9");
}

/// A whole tuple copied from one local to another: the copy has to move
/// every field, not just the first word.
#[test]
fn tuple_local_copies_all_of_its_fields() {
    let out = run_body(
        r#"
    t := (7, 9);
    u := t;
    raise "u=" + string u.t0 + "," + string u.t1;
"#,
    );
    assert_raises(out, "u=7,9");
}

/// A `big` field is 8 bytes wide. Laid out as 4, the 8-byte store of `a`
/// runs over `b`, and `b`'s own store runs back over the top half of `a` —
/// so this catches the width even without a value that needs all 64 bits.
/// (`big` *literals* above 2^31 truncate for an unrelated reason; see the
/// note in the report.)
#[test]
fn tuple_with_a_big_field_keeps_its_full_width() {
    let out = run_body(
        r#"
    (a, b) := (big 5, 9);
    raise "a=" + string a + " b=" + string b;
"#,
    );
    assert_raises(out, "a=5 b=9");
}

/// A pointer field has to be moved as a pointer, and survive to be read.
#[test]
fn tuple_with_a_string_field_keeps_the_string() {
    let out = run_body(
        r#"
    (n, s) := (7, "hi");
    raise "n=" + string n + " s=" + s;
"#,
    );
    assert_raises(out, "n=7 s=hi");
}

/// `nil` names a field that is received but not bound; the fields after it
/// must still land in the right locals.
#[test]
fn tuple_decl_with_nil_field_still_binds_the_rest() {
    let out = run_body(
        r#"
    (nil, b) := (7, 9);
    raise "b=" + string b;
"#,
    );
    assert_raises(out, "b=9");
}

/// A tuple literal passed as an argument — `regex->executese(re, s, (0, len
/// s-1), 1, 1)` is the corpus shape.
#[test]
fn tuple_literal_passes_as_a_call_argument() {
    let out = run_src(
        r#"implement T;
init(nil: ref Draw->Context, nil: list of string)
{
	f(1, (7, 9), 2);
}
f(x: int, t: (int, int), y: int)
{
	raise "x=" + string x + " t=" + string t.t0 + "," + string t.t1 + " y=" + string y;
}
"#,
    );
    assert_raises(out, "x=1 t=7,9 y=2");
}

/// `for ((i, j) := (0, 5); ...)` — a tuple declaration in a `for` initialiser.
#[test]
fn tuple_declaration_works_in_a_for_initialiser() {
    let out = run_body(
        r#"
    s := 0;
    for ((i, j) := (0, 5); i < 3; i++)
        s += i + j;
    raise "s=" + string s;
"#,
    );
    assert_raises(out, "s=18");
}

// ── Tuple channels (element size) ───────────────────────────────

/// `chan of (int, int)` carries 8 bytes, not 4. With `Newcw`'s fixed
/// `elem_size` of 4 the second field arrived as zero — the channel compiled
/// and ran, and delivered a wrong value.
#[test]
fn tuple_channel_delivers_every_field() {
    let out = run_src(
        r#"implement T;
init(nil: ref Draw->Context, nil: list of string)
{
	c := chan of (int, int);
	spawn producer(c);
	(a, b) := <-c;
	raise "a=" + string a + " b=" + string b;
}
producer(c: chan of (int, int))
{
	c <-= (7, 9);
}
"#,
    );
    assert_raises(out, "a=7 b=9");
}

/// The same channel read into a tuple local rather than destructured.
#[test]
fn tuple_channel_receives_into_a_tuple_local() {
    let out = run_src(
        r#"implement T;
init(nil: ref Draw->Context, nil: list of string)
{
	c := chan of (int, int);
	spawn producer(c);
	t := <-c;
	raise "t=" + string t.t0 + "," + string t.t1;
}
producer(c: chan of (int, int))
{
	c <-= (7, 9);
}
"#,
    );
    assert_raises(out, "t=7,9");
}

/// A tuple channel whose fields are of different widths: the channel's
/// element size is the tuple's real size, including alignment padding.
#[test]
fn tuple_channel_carries_mixed_width_fields() {
    let out = run_src(
        r#"implement T;
init(nil: ref Draw->Context, nil: list of string)
{
	c := chan of (int, big, string);
	spawn producer(c);
	(a, b, s) := <-c;
	raise "a=" + string a + " b=" + string b + " s=" + s;
}
producer(c: chan of (int, big, string))
{
	c <-= (7, big 5, "hi");
}
"#,
    );
    assert_raises(out, "a=7 b=5 s=hi");
}

/// `((d, reply) := <-c).t0` — a tuple-valued declaration used as an
/// expression and then field-selected. The declaration still binds.
#[test]
fn tuple_declaration_is_usable_as_an_expression() {
    let out = run_src(
        r#"implement T;
init(nil: ref Draw->Context, nil: list of string)
{
	c := chan of (int, int);
	spawn producer(c);
	x := ((a, b) := <-c).t0;
	raise "x=" + string x + " a=" + string a + " b=" + string b;
}
producer(c: chan of (int, int))
{
	c <-= (7, 9);
}
"#,
    );
    assert_raises(out, "x=7 a=7 b=9");
}

/// Isolates the channel element-size bug from tuple *literals*: this program
/// compiled cleanly even before tuple values were supported, because the
/// message comes from a tuple-returning call — and it reported `b=0`. The
/// second word of every message was dropped on the floor by `Newcw`'s fixed
/// 4-byte element size.
#[test]
fn tuple_channel_does_not_truncate_a_call_result() {
    let out = run_src(
        r#"implement T;
init(nil: ref Draw->Context, nil: list of string)
{
	c := chan of (int, int);
	spawn producer(c);
	(a, b) := <-c;
	raise "a=" + string a + " b=" + string b;
}
mk(): (int, int)
{
	return (7, 9);
}
producer(c: chan of (int, int))
{
	c <-= mk();
}
"#,
    );
    assert_raises(out, "a=7 b=9");
}

/// `(a, b) = (b, a)` assigns to existing variables. It used to fall through
/// to a branch that evaluated the right-hand side into a scratch temp and
/// threw it away, so the swap simply did not happen.
#[test]
fn tuple_assignment_to_existing_variables_takes_effect() {
    let out = run_body(
        r#"
    a := 1;
    b := 2;
    (a, b) = (b, a);
    raise "a=" + string a + " b=" + string b;
"#,
    );
    assert_raises(out, "a=2 b=1");
}

/// A tuple field that is itself a tuple is more than one word wide. Sized as
/// one, the inner tuple's later fields would run over the outer tuple's.
#[test]
fn nested_tuple_keeps_every_field() {
    let out = run_body(
        r#"
    (p, n) := ((7, 9), 3);
    raise "p=" + string p.t0 + "," + string p.t1 + " n=" + string n;
"#,
    );
    assert_raises(out, "p=7,9 n=3");
}

/// `(date, tm.mday) = datenum(date)` — a tuple assignment whose targets are
/// not plain variables. Every target has to be written, whatever kind of
/// lvalue it is.
#[test]
fn tuple_assignment_writes_through_complex_lvalues() {
    let out = run_src(
        r#"implement T;
T: module {
    P: adt { n: int; };
};
init(nil: ref Draw->Context, nil: list of string)
{
	p := ref P(0);
	a := array[2] of int;
	s := 0;
	(p.n, a[1], nil) = (7, 9, 5);
	raise "n=" + string p.n + " a1=" + string a[1] + " s=" + string s;
}
"#,
    );
    assert_raises(out, "n=7 a1=9 s=0");
}

// ── `alt` ───────────────────────────────────────────────────────
//
// An `alt` compiles to a table of `{channel, value address}` entries plus a
// `{nsend, nrecv}` header, and the `alt`/`nbalt` instruction answers with the
// index of the entry it selected. Every test below asserts the value the
// selected arm computed, because the failure mode being guarded against is a
// statement that emits no code at all: a program that merely compiles proves
// nothing about which arm ran.

/// No guard is ready, so the `*` arm runs. `nbalt` reports "nothing ready" as
/// the index `nsend + nrecv`, one past the last entry, so that is the index
/// the wildcard arm has to answer to.
#[test]
fn alt_wildcard_arm_runs_when_no_guard_is_ready() {
    let out = run_body(
        r#"
    c := chan of int;
    n := 0;
    alt {
    x := <-c =>
        n = x;
    * =>
        n = 5;
    }
    raise "n=" + string n;
"#,
    );
    assert_raises(out, "n=5");
}

/// A send guard on an empty single-slot channel is ready, so its arm runs and
/// the value reaches the channel. The value has to be evaluated into its slot
/// before the `alt` executes, because the instruction itself performs the
/// transfer out of that slot.
#[test]
fn alt_ready_send_arm_transfers_its_value() {
    let out = run_body(
        r#"
    c := chan of int;
    n := 0;
    alt {
    c <-= 7 =>
        n = 1;
    * =>
        n = 2;
    }
    v := <-c;
    raise "n=" + string n + " v=" + string v;
"#,
    );
    assert_raises(out, "n=1 v=7");
}

/// A receive guard on a full channel is ready, and the received value is
/// visible in the arm body under the name the guard declared.
#[test]
fn alt_ready_recv_arm_binds_the_received_value() {
    let out = run_body(
        r#"
    c := chan of int;
    c <-= 7;
    n := 0;
    alt {
    x := <-c =>
        n = x;
    * =>
        n = 2;
    }
    raise "n=" + string n;
"#,
    );
    assert_raises(out, "n=7");
}

/// An `alt` with no `*` arm blocks. The VM parks the thread and re-executes
/// the same instruction when it wakes, so the table (and the addresses in it)
/// must still be valid after the suspension.
#[test]
fn alt_blocks_until_a_spawned_producer_sends() {
    let out = run_src(
        r#"implement T;
init(nil: ref Draw->Context, nil: list of string)
{
	c := chan of int;
	spawn producer(c);
	alt {
	x := <-c =>
		raise "x=" + string x;
	}
}
producer(c: chan of int)
{
	c <-= 77;
}
"#,
    );
    assert_raises(out, "x=77");
}

/// The same blocking path with a pointer-valued element: the received string
/// is written into the arm's slot through the address in the table.
#[test]
fn alt_receives_a_string_message() {
    let out = run_src(
        r#"implement T;
init(nil: ref Draw->Context, nil: list of string)
{
	c := chan of string;
	spawn producer(c);
	alt {
	s := <-c =>
		raise "s=" + s;
	}
}
producer(c: chan of string)
{
	c <-= "hi";
}
"#,
    );
    assert_raises(out, "s=hi");
}

/// Two receive guards, and the producer feeds the second channel. The arm
/// that runs is the one whose guard was ready, which is what proves the
/// selected index is mapped back to the right arm.
#[test]
fn alt_runs_the_arm_whose_channel_became_ready() {
    let out = run_src(
        r#"implement T;
init(nil: ref Draw->Context, nil: list of string)
{
	c1 := chan of int;
	c2 := chan of int;
	spawn producer(c2);
	alt {
	x := <-c1 =>
		raise "first x=" + string x;
	y := <-c2 =>
		raise "second y=" + string y;
	}
}
producer(c: chan of int)
{
	c <-= 88;
}
"#,
    );
    assert_raises(out, "second y=88");
}

/// A mixed table: the send guard comes first in the source but its channel is
/// full, so the receive guard is the ready one. Send entries occupy indices
/// `0..nsend` and receive entries `nsend..nsend+nrecv`, so a receive numbered
/// from zero instead of from `nsend` would dispatch this to the wrong arm (or
/// to no arm at all).
#[test]
fn alt_mixed_table_indexes_receives_after_sends() {
    let out = run_body(
        r#"
    c1 := chan of int;
    c2 := chan of int;
    c1 <-= 1;
    c2 <-= 2;
    n := 0;
    alt {
    c1 <-= 9 =>
        n = 100;
    y := <-c2 =>
        n = y;
    }
    raise "n=" + string n;
"#,
    );
    assert_raises(out, "n=2");
}

/// An `alt` guard may assign to an existing variable rather than declare a
/// new one; the arm body then sees the received value in that variable.
#[test]
fn alt_recv_guard_assigns_to_an_existing_variable() {
    let out = run_body(
        r#"
    c := chan of int;
    c <-= 6;
    x := 0;
    alt {
    x = <-c =>
        x = x + 1;
    * =>
        x = 99;
    }
    raise "x=" + string x;
"#,
    );
    assert_raises(out, "x=7");
}

/// A guard with no destination receives and discards the value. The arm still
/// has to run.
#[test]
fn alt_bare_receive_guard_discards_the_value() {
    let out = run_body(
        r#"
    c := chan of int;
    c <-= 4;
    n := 0;
    alt {
    <-c =>
        n = 1;
    * =>
        n = 2;
    }
    raise "n=" + string n;
"#,
    );
    assert_raises(out, "n=1");
}

/// `or`-joined guards share one body. Each is a separate table entry, so an
/// implementation that kept only the first would listen on `c1` alone and take
/// the `*` arm here.
#[test]
fn alt_or_joined_guards_share_one_body() {
    let out = run_body(
        r#"
    c1 := chan of int;
    c2 := chan of int;
    c2 <-= 5;
    n := 0;
    alt {
    x := <-c1 or x = <-c2 =>
        n = x;
    * =>
        n = 99;
    }
    raise "n=" + string n;
"#,
    );
    assert_raises(out, "n=5");
}

/// A tuple-valued channel delivers its whole message into a tuple guard, and
/// every declared name binds.
#[test]
fn alt_tuple_receive_guard_binds_every_field() {
    let out = run_body(
        r#"
    c := chan of (int, int);
    c <-= (3, 4);
    alt {
    (a, b) := <-c =>
        raise "a=" + string a + " b=" + string b;
    * =>
        raise "none";
    }
"#,
    );
    assert_raises(out, "a=3 b=4");
}

/// `break` in an arm leaves the `alt`, not the enclosing loop: the reference
/// compiler pushes a break scope for `alt` with no continue target
/// (`limbo/com.c:579-605`). The loop body after the `alt` therefore still
/// runs, three times over.
#[test]
fn break_inside_an_alt_arm_leaves_only_the_alt() {
    let out = run_body(
        r#"
    c := chan of int;
    n := 0;
    for(i := 0; i < 3; i++) {
        alt {
        x := <-c =>
            n = n + x;
        * =>
            n = n + 1;
            break;
        }
        n = n + 10;
    }
    raise "n=" + string n;
"#,
    );
    assert_raises(out, "n=33");
}

// ── `pick` and tagged ADTs ──────────────────────────────────────
//
// A tagged (`pick`) ADT is laid out with its tag in word 0, the ADT's own
// fields after it, and each variant's fields overlaid on one another after
// those. `tagof` reads word 0 and `pick` dispatches on it, so every test here
// asserts a value that a wrong tag, a wrong union base, or a constructor that
// emitted no code would get wrong.

/// The gate test for `tagof`: tags are dense from zero in declaration order,
/// so `Square` is 1. A `Circle`-only test would pass even against the old
/// lowering, which compiled every `tagof` to a literal zero.
#[test]
fn tagof_reports_the_variant_a_value_was_built_with() {
    let out = run_src(
        r#"implement T;
Shape: adt {
	pick {
	Circle =>
		r: int;
	Square =>
		s: int;
	}
};
init(nil: ref Draw->Context, nil: list of string)
{
	c := ref Shape.Circle(5);
	q := ref Shape.Square(3);
	ct := tagof c;
	qt := tagof q;
	raise "c=" + string ct + " q=" + string qt;
}
"#,
    );
    assert_raises(out, "c=0 q=1");
}

/// `tagof Shape.Square` names a variant rather than a value, and folds to that
/// variant's tag.
#[test]
fn tagof_a_variant_name_folds_to_its_tag() {
    let out = run_src(
        r#"implement T;
Shape: adt {
	pick {
	Circle =>
		r: int;
	Square =>
		s: int;
	}
};
init(nil: ref Draw->Context, nil: list of string)
{
	t := tagof Shape.Square;
	raise "t=" + string t;
}
"#,
    );
    assert_raises(out, "t=1");
}

/// `pick` runs the arm naming the variant the value was built with.
#[test]
fn pick_dispatches_on_the_tag() {
    let out = run_src(
        r#"implement T;
Shape: adt {
	pick {
	Circle =>
		r: int;
	Square =>
		s: int;
	}
};
init(nil: ref Draw->Context, nil: list of string)
{
	sh := ref Shape.Square(3);
	n := 0;
	pick x := sh {
	Circle =>
		n = 1;
	Square =>
		n = 2;
	}
	raise "n=" + string n;
}
"#,
    );
    assert_raises(out, "n=2");
}

/// Inside an arm, the bound name has the arm's variant type, so the variant's
/// own fields resolve. Both variants place their field at the same offset, so
/// a wrong union base reads the tag or runs off the record instead.
#[test]
fn pick_arm_reads_the_variant_fields() {
    let out = run_src(
        r#"implement T;
Shape: adt {
	pick {
	Circle =>
		r: int;
	Square =>
		s: int;
	}
};
init(nil: ref Draw->Context, nil: list of string)
{
	sh := ref Shape.Square(7);
	n := 0;
	pick x := sh {
	Circle =>
		n = x.r;
	Square =>
		n = x.s;
	}
	raise "n=" + string n;
}
"#,
    );
    assert_raises(out, "n=7");
}

/// The ADT's own fields sit between the tag and the variant fields, and are
/// readable in every arm. This is what fixes tag at 0, common at 4, and the
/// first variant field at 8.
#[test]
fn pick_arm_reads_the_common_fields_too() {
    let out = run_src(
        r#"implement T;
Shape: adt {
	name: string;
	pick {
	Circle =>
		r: int;
	Square =>
		s: int;
	}
};
init(nil: ref Draw->Context, nil: list of string)
{
	sh := ref Shape.Circle("circ", 5);
	pick x := sh {
	Circle =>
		raise "name=" + x.name + " r=" + string x.r;
	Square =>
		raise "square";
	}
}
"#,
    );
    assert_raises(out, "name=circ r=5");
}

/// A `*` arm answers for every tag no arm names.
#[test]
fn pick_wildcard_arm_catches_the_other_variants() {
    let out = run_src(
        r#"implement T;
Shape: adt {
	pick {
	Circle =>
		r: int;
	Square =>
		s: int;
	}
};
init(nil: ref Draw->Context, nil: list of string)
{
	sh := ref Shape.Square(3);
	n := 0;
	pick x := sh {
	Circle =>
		n = 1;
	* =>
		n = 9;
	}
	raise "n=" + string n;
}
"#,
    );
    assert_raises(out, "n=9");
}

/// `A or B` in a pick clause gives each name its own tag while they share one
/// field layout, and an arm may name several tags.
#[test]
fn pick_arm_may_name_several_tags() {
    let out = run_src(
        r#"implement T;
Shape: adt {
	pick {
	Circle =>
		r: int;
	Square or Rect =>
		w: int;
	}
};
init(nil: ref Draw->Context, nil: list of string)
{
	sh := ref Shape.Rect(4);
	t := tagof sh;
	n := 0;
	pick x := sh {
	Circle =>
		n = 1;
	Square or Rect =>
		n = x.w;
	}
	raise "t=" + string t + " n=" + string n;
}
"#,
    );
    assert_raises(out, "t=2 n=4");
}

/// A `pick` that names neither every variant nor `*` simply falls through when
/// nothing matches: the single-arm downcast idiom depends on the statement
/// after it still running.
#[test]
fn non_exhaustive_pick_falls_through_to_the_next_statement() {
    let out = run_src(
        r#"implement T;
Shape: adt {
	pick {
	Circle =>
		r: int;
	Square =>
		s: int;
	}
};
init(nil: ref Draw->Context, nil: list of string)
{
	sh := ref Shape.Square(3);
	n := 5;
	pick x := sh {
	Circle =>
		n = 1;
	}
	n = n + 1;
	raise "n=" + string n;
}
"#,
    );
    assert_raises(out, "n=6");
}

/// `break` in a `pick` arm leaves the `pick`, not the enclosing loop: the
/// reference compiler pushes a break scope for `pick` with no continue target
/// (`limbo/com.c:579-605`).
#[test]
fn break_inside_a_pick_arm_leaves_only_the_pick() {
    let out = run_src(
        r#"implement T;
Shape: adt {
	pick {
	Circle =>
		r: int;
	Square =>
		s: int;
	}
};
init(nil: ref Draw->Context, nil: list of string)
{
	sh := ref Shape.Circle(1);
	n := 0;
	for(i := 0; i < 3; i++) {
		pick x := sh {
		Circle =>
			n = n + 1;
			break;
		}
		n = n + 10;
	}
	raise "n=" + string n;
}
"#,
    );
    assert_raises(out, "n=33");
}

/// `pick m := <-c` picks over the message a channel just delivered, so the
/// tags have to be resolved from the channel's element type. Eight corpus
/// files open with exactly this line.
#[test]
fn pick_resolves_the_tags_of_a_received_message() {
    let out = run_src(
        r#"implement T;
Msg: adt {
	pick {
	Ping =>
		a: int;
	Pong =>
		b: int;
	}
};
init(nil: ref Draw->Context, nil: list of string)
{
	c := chan of ref Msg;
	c <-= ref Msg.Pong(7);
	n := 0;
	pick m := <-c {
	Ping =>
		n = m.a;
	Pong =>
		n = m.b;
	}
	raise "n=" + string n;
}
"#,
    );
    assert_raises(out, "n=7");
}

/// A channel that arrives through another variable keeps its element type.
/// `r := dummy; ... alt { (a, b) := <-r => }` is the shape six corpus files
/// use to switch a guard between two channels, and a channel whose element
/// type went missing moves four bytes per message instead of the message.
#[test]
fn a_channel_alias_keeps_its_element_type() {
    let out = run_body(
        r#"
    c := chan of (int, int);
    r := c;
    c <-= (3, 4);
    alt {
    (a, b) := <-r =>
        raise "a=" + string a + " b=" + string b;
    * =>
        raise "none";
    }
"#,
    );
    assert_raises(out, "a=3 b=4");
}
