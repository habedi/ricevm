#!/usr/bin/env bash
#
# Test the built-in Limbo compiler (ricevm-limbo) against the reference
# compiler (limbo.dis running on RiceVM).
#
# Phase 1: Compile hand-crafted test programs with both compilers, run both,
#          compare outputs (correctness validation).
# Phase 2: Compile real Inferno programs from external/inferno-os/appl/cmd/
#          with both compilers and verify the built-in compiler succeeds
#          whenever the reference compiler does.
#
# Exit 0 if all Phase 1 tests pass, 1 otherwise.
# Phase 2 failures are reported but do not cause a non-zero exit.

set -euo pipefail

RICEVM="./target/release/ricevm-cli"
LIMBO_DIS="external/inferno-os/dis/limbo.dis"
PROBE="--probe external/inferno-os/dis --probe external/inferno-os/dis/lib"
INCLUDE="-I external/inferno-os/module"
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

pass=0
fail=0
skip=0
total=0

green=$'\033[32m'
red=$'\033[31m'
yellow=$'\033[33m'
cyan=$'\033[36m'
reset=$'\033[0m'

# ── Phase 1: Hand-crafted correctness tests ─────────────────────

# Generate test source files
cat > "$TMPDIR/hello.b" << 'LIMBO'
implement Hello;
include "sys.m"; sys: Sys;
include "draw.m";
Hello: module { init: fn(nil: ref Draw->Context, nil: list of string); };
init(nil: ref Draw->Context, nil: list of string)
{
    sys = load Sys Sys->PATH;
    sys->print("hello, world\n");
}
LIMBO

cat > "$TMPDIR/args.b" << 'LIMBO'
implement Args;
include "sys.m"; sys: Sys;
include "draw.m";
Args: module { init: fn(nil: ref Draw->Context, args: list of string); };
init(nil: ref Draw->Context, args: list of string)
{
    sys = load Sys Sys->PATH;
    if(args != nil) args = tl args;
    while(args != nil) { sys->print("%s\n", hd args); args = tl args; }
}
LIMBO

cat > "$TMPDIR/arith.b" << 'LIMBO'
implement Arith;
include "sys.m"; sys: Sys;
include "draw.m";
Arith: module { init: fn(nil: ref Draw->Context, nil: list of string); };
init(nil: ref Draw->Context, nil: list of string)
{
    sys = load Sys Sys->PATH;
    sys->print("%d\n", 6 * 7);
}
LIMBO

cat > "$TMPDIR/string_ops.b" << 'LIMBO'
implement StringOps;
include "sys.m"; sys: Sys;
include "draw.m";
StringOps: module { init: fn(nil: ref Draw->Context, nil: list of string); };
init(nil: ref Draw->Context, nil: list of string)
{
    sys = load Sys Sys->PATH;
    sys->print("%s\n", "HELLO" + " " + "WORLD");
}
LIMBO

cat > "$TMPDIR/control.b" << 'LIMBO'
implement Control;
include "sys.m"; sys: Sys;
include "draw.m";
Control: module { init: fn(nil: ref Draw->Context, nil: list of string); };
init(nil: ref Draw->Context, nil: list of string)
{
    sys = load Sys Sys->PATH;
    x := 5;
    if(x > 3) sys->print("big\n"); else sys->print("small\n");
    for(i := 0; i < 3; i++) sys->print("i=%d\n", i);
    sys->print("done\n");
}
LIMBO

cat > "$TMPDIR/list_ops.b" << 'LIMBO'
implement ListOps;
include "sys.m"; sys: Sys;
include "draw.m";
ListOps: module { init: fn(nil: ref Draw->Context, nil: list of string); };
init(nil: ref Draw->Context, nil: list of string)
{
    sys = load Sys Sys->PATH;
    l := "c" :: "b" :: "a" :: nil;
    n := 0;
    while(l != nil) { n++; l = tl l; }
    sys->print("%d items\n", n);
}
LIMBO

cat > "$TMPDIR/while_loop.b" << 'LIMBO'
implement WhileLoop;
include "sys.m"; sys: Sys;
include "draw.m";
WhileLoop: module { init: fn(nil: ref Draw->Context, nil: list of string); };
init(nil: ref Draw->Context, nil: list of string)
{
    sys = load Sys Sys->PATH;
    i := 0; while(i < 3) i++;
    sys->print("loop: %d\n", i);
}
LIMBO

cat > "$TMPDIR/echo.b" << 'LIMBO'
implement Echo;
include "sys.m"; sys: Sys;
include "draw.m";
Echo: module { init: fn(nil: ref Draw->Context, args: list of string); };
init(nil: ref Draw->Context, args: list of string)
{
    sys = load Sys Sys->PATH;
    if(args != nil) args = tl args;
    s := "";
    while(args != nil) {
        if(s != "") s = s + " ";
        s = s + hd args;
        args = tl args;
    }
    s = s + "\n";
    a := array of byte s;
    sys->write(sys->fildes(1), a, len a);
}
LIMBO

cat > "$TMPDIR/case_stmt.b" << 'LIMBO'
implement CaseStmt;
include "sys.m"; sys: Sys;
include "draw.m";
CaseStmt: module { init: fn(nil: ref Draw->Context, nil: list of string); };
init(nil: ref Draw->Context, nil: list of string)
{
    sys = load Sys Sys->PATH;
    x := 2;
    case x { 1 => sys->print("one\n"); 2 => sys->print("two\n"); * => sys->print("other\n"); }
}
LIMBO

cat > "$TMPDIR/local_func.b" << 'LIMBO'
implement LocalFunc;
include "sys.m"; sys: Sys;
include "draw.m";
LocalFunc: module { init: fn(nil: ref Draw->Context, nil: list of string); };
double(x: int): int { return x * 2; }
init(nil: ref Draw->Context, nil: list of string)
{
    sys = load Sys Sys->PATH;
    sys->print("%d\n", double(21));
}
LIMBO

cat > "$TMPDIR/chan_test.b" << 'LIMBO'
implement ChanTest;
include "sys.m"; sys: Sys;
include "draw.m";
ChanTest: module { init: fn(nil: ref Draw->Context, nil: list of string); };
sender(c: chan of int) { c <-= 99; }
init(nil: ref Draw->Context, nil: list of string)
{
    sys = load Sys Sys->PATH;
    c := chan of int;
    spawn sender(c);
    sys->print("%d\n", <-c);
}
LIMBO

# Test entries: "name|guest_args|expected_substring"
declare -a TESTS=(
    "hello|--|hello, world"
    "args|-- alpha beta|alpha"
    "arith|--|42"
    "string_ops|--|HELLO WORLD"
    "control|--|done"
    "list_ops|--|3 items"
    "while_loop|--|loop: 3"
    "echo|-- hello world|hello world"
    "case_stmt|--|two"
    "local_func|--|42"
    "chan_test|--|99"
)

echo "=== Phase 1: Correctness Tests (built-in vs reference compiler) ==="
echo ""

for entry in "${TESTS[@]}"; do
    IFS='|' read -r name guest_args expected <<< "$entry"
    total=$((total + 1))
    src="$TMPDIR/${name}.b"

    # Compile with reference
    ref_dis="$TMPDIR/${name}_ref.dis"
    ref_out="$TMPDIR/${name}_ref.out"
    if timeout 10 "$RICEVM" run "$LIMBO_DIS" $PROBE -- $INCLUDE "$src" </dev/null >/dev/null 2>&1; then
        # Find the produced .dis (module name may differ from filename)
        for candidate in "${name}.dis" $(head -1 "$src" | sed 's/implement //;s/;//' | while read m; do echo "${m}.dis"; echo "$(echo $m | tr '[:upper:]' '[:lower:]').dis"; done); do
            if [ -f "$candidate" ]; then mv "$candidate" "$ref_dis" 2>/dev/null; break; fi
        done
    fi

    if [ ! -f "$ref_dis" ]; then
        echo "${yellow}SKIP${reset} $name (reference compiler failed)"
        skip=$((skip + 1))
        continue
    fi

    # Compile with built-in
    builtin_dis="$TMPDIR/${name}_builtin.dis"
    builtin_out="$TMPDIR/${name}_builtin.out"
    if ! timeout 5 "$RICEVM" compile "$src" -o "$builtin_dis" >/dev/null 2>&1; then
        echo "${red}FAIL${reset} $name (built-in compiler failed)"
        fail=$((fail + 1))
        continue
    fi

    # Run both and capture stdout only
    timeout 5 "$RICEVM" run "$ref_dis" $PROBE $guest_args </dev/null 2>/dev/null | grep -v "^✓\|INFO\|Module loaded" >"$ref_out" || true
    timeout 5 "$RICEVM" run "$builtin_dis" $PROBE $guest_args </dev/null 2>/dev/null | grep -v "^✓\|INFO\|Module loaded" >"$builtin_out" || true

    # Compare
    if diff -q "$ref_out" "$builtin_out" >/dev/null 2>&1; then
        if grep -q "$expected" "$builtin_out" 2>/dev/null; then
            echo "${green}PASS${reset} $name"
            pass=$((pass + 1))
        else
            echo "${red}FAIL${reset} $name (missing expected: '$expected')"
            echo "  ref:     $(head -1 "$ref_out")"
            echo "  built-in: $(head -1 "$builtin_out")"
            fail=$((fail + 1))
        fi
    else
        echo "${red}FAIL${reset} $name (output mismatch)"
        echo "  ref:     $(head -1 "$ref_out")"
        echo "  built-in: $(head -1 "$builtin_out")"
        fail=$((fail + 1))
    fi
done

echo ""
echo "Phase 1: $pass passed, $fail failed, $skip skipped (of $total)"

# ── Phase 2: Inferno program compilation coverage ──────────────

echo ""
echo "=== Phase 2: Inferno Program Compilation Coverage ==="
echo ""

inferno_total=0
inferno_both=0
inferno_ref_only=0
inferno_builtin_only=0

for f in external/inferno-os/appl/cmd/*.b; do
    inferno_total=$((inferno_total + 1))
    name=$(basename "$f" .b)
    builtin_ok=false
    ref_ok=false

    if timeout 3 "$RICEVM" compile "$f" -o "$TMPDIR/${name}_inf_bi.dis" >/dev/null 2>&1; then
        builtin_ok=true
    fi
    if timeout 10 "$RICEVM" run "$LIMBO_DIS" $PROBE -- $INCLUDE "$f" </dev/null >/dev/null 2>&1; then
        ref_ok=true
        # Clean up ref output
        for candidate in "${name}.dis" $(head -1 "$f" 2>/dev/null | sed 's/implement //;s/;//' | while read m; do echo "${m}.dis"; echo "$(echo $m | tr '[:upper:]' '[:lower:]').dis"; done); do
            rm -f "$candidate" 2>/dev/null
        done
    fi

    if $builtin_ok && $ref_ok; then
        inferno_both=$((inferno_both + 1))
    elif $ref_ok; then
        inferno_ref_only=$((inferno_ref_only + 1))
    elif $builtin_ok; then
        inferno_builtin_only=$((inferno_builtin_only + 1))
    fi
done

echo "Inferno cmd/ programs:  $inferno_total total"
echo "  Both compile:         ${cyan}$inferno_both${reset}"
echo "  Reference only:       $inferno_ref_only"
echo "  Built-in only:        $inferno_builtin_only"
echo "  Neither:              $((inferno_total - inferno_both - inferno_ref_only - inferno_builtin_only))"
echo ""

# Phase 1 determines exit code
if [ "$fail" -gt 0 ]; then
    exit 1
fi
exit 0
