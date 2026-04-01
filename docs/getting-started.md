# Getting Started

## Build from Source

RiceVM requires Rust 1.92.0 or newer. The optional GUI feature requires SDL2.

```bash
# Clone the repository with the Inferno OS submodule
git clone --recursive --depth=1 https://github.com/habedi/ricevm.git
cd ricevm

# Build in release mode
cargo build --release

# Verify the build
cargo run -p ricevm-cli -- --version
```

### Optional Features

```bash
# Build with SDL2 GUI support (requires libsdl2-dev)
cargo build --release --features gui

# Build with audio support (requires cpal dependencies)
cargo build --release --features audio
```

## Running Programs

### Run a Pre-compiled Inferno Program

```bash
cargo run -p ricevm-cli -- run external/inferno-os/dis/echo.dis \
    --probe external/inferno-os/dis -- hello world
```

### Compile and Run a Limbo Program

Write a Limbo source file:

```limbo
implement Hello;

include "sys.m";
include "draw.m";

Hello: module {
    init: fn(ctxt: ref Draw->Context, argv: list of string);
};

init(ctxt: ref Draw->Context, argv: list of string) {
    sys := load Sys Sys->PATH;
    sys->print("hello, world\n");
}
```

Compile with the built-in compiler and run:

```bash
# Compile .b to .dis
cargo run -p ricevm-cli -- compile hello.b

# Run the compiled bytecode
cargo run -p ricevm-cli -- run hello.dis --probe external/inferno-os/dis
```

Or use the reference Inferno Limbo compiler (runs on RiceVM itself):

```bash
cargo run -p ricevm-cli -- run external/inferno-os/dis/limbo.dis \
    --probe external/inferno-os/dis --probe external/inferno-os/dis/lib \
    -- -I external/inferno-os/module hello.b
```

### Disassemble a Module

```bash
cargo run -p ricevm-cli -- dis external/inferno-os/dis/echo.dis
```

### Debug a Program

```bash
cargo run -p ricevm-cli -- debug external/inferno-os/dis/echo.dis \
    --probe external/inferno-os/dis
```

## CLI Reference

| Subcommand | Description |
|------------|-------------|
| `run` | Execute a `.dis` module file |
| `compile` | Compile a Limbo `.b` source to `.dis` bytecode |
| `dis` | Disassemble a `.dis` module into human-readable output |
| `debug` | Debug a `.dis` module interactively |

### Common Flags

| Flag | Description |
|------|-------------|
| `--probe PATH` | Add a directory to the module search path (repeatable) |
| `--root PATH` | Map Inferno root paths to a host directory |
| `--trace` | Print each instruction as it executes |
| `--no-gc` | Disable mark-and-sweep garbage collection |
| `-I PATH` | Include search path for `.m` files (compile subcommand) |
| `-o PATH` | Output file path (compile subcommand) |
