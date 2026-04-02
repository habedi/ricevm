# Examples

These examples assume RiceVM is built and the Inferno OS submodule is checked out.

## Hello World

```limbo
implement Hello;

include "sys.m";
include "draw.m";

Hello: module {
    init: fn(nil: ref Draw->Context, nil: list of string);
};

init(nil: ref Draw->Context, nil: list of string) {
    sys := load Sys Sys->PATH;
    sys->print("hello, world\n");
}
```

```bash
ricevm-cli compile hello.b
ricevm-cli run hello.dis --probe external/inferno-os/dis
```

## Echo with Arguments

```limbo
implement Echo;

include "sys.m";
include "draw.m";

Echo: module {
    init: fn(nil: ref Draw->Context, args: list of string);
};

init(nil: ref Draw->Context, args: list of string) {
    sys := load Sys Sys->PATH;
    if (args != nil)
        args = tl args;
    s := "";
    while (args != nil) {
        if (s != "")
            s = s + " ";
        s = s + hd args;
        args = tl args;
    }
    s = s + "\n";
    a := array of byte s;
    sys->write(sys->fildes(1), a, len a);
}
```

```bash
ricevm-cli compile echo.b
ricevm-cli run echo.dis --probe external/inferno-os/dis -- hello world
# Output: hello world
```

## Channels and Concurrency

```limbo
implement ChanDemo;

include "sys.m";
include "draw.m";

ChanDemo: module {
    init: fn(nil: ref Draw->Context, nil: list of string);
};

sender(c: chan of int) {
    c <-= 42;
}

init(nil: ref Draw->Context, nil: list of string) {
    sys := load Sys Sys->PATH;
    c := chan of int;
    spawn sender(c);
    sys->print("received: %d\n", <-c);
}
```

```bash
ricevm-cli compile chan_demo.b
ricevm-cli run chan_demo.dis --probe external/inferno-os/dis
# Output: received: 42
```

## Running Pre-compiled Inferno Programs

RiceVM includes 866 pre-compiled `.dis` files from the Inferno OS distribution:

```bash
# List files
ricevm-cli run external/inferno-os/dis/ls.dis --probe external/inferno-os/dis

# Word count
echo "hello world" | ricevm-cli run external/inferno-os/dis/wc.dis \
    --probe external/inferno-os/dis --probe external/inferno-os/dis/lib

# Sort
printf "cherry\napple\nbanana\n" | ricevm-cli run external/inferno-os/dis/sort.dis \
    --probe external/inferno-os/dis --probe external/inferno-os/dis/lib

# Grep
echo -e "hello\nworld\nhello again" | ricevm-cli run external/inferno-os/dis/grep.dis \
    --probe external/inferno-os/dis --probe external/inferno-os/dis/lib -- hello

# MD5 checksum
echo -n "test" | ricevm-cli run external/inferno-os/dis/md5sum.dis \
    --probe external/inferno-os/dis --probe external/inferno-os/dis/lib
```

## GUI Programs

With the `gui` feature enabled and SDL2 installed:

```bash
# Run the About Inferno dialog
cargo run -p ricevm-cli --release --features gui -- \
    run external/inferno-os/dis/wm/about.dis \
    --probe external/inferno-os/dis \
    --probe external/inferno-os/dis/lib \
    --root external/inferno-os
```

## Using the Limbo Compiler from Inferno

RiceVM can run the original Limbo compiler (compiled to Dis bytecode) on itself:

```bash
# Compile a source file using the Inferno Limbo compiler running on RiceVM
ricevm-cli run external/inferno-os/dis/limbo.dis \
    --probe external/inferno-os/dis --probe external/inferno-os/dis/lib \
    -- -I external/inferno-os/module hello.b

# Run the result
ricevm-cli run hello.dis --probe external/inferno-os/dis
```
