## RiceVM

<div align="center">
  <picture>
    <img alt="Project Logo" src="logo.svg" height="25%" width="25%">
  </picture>
</div>
<br>

[![Tests](https://img.shields.io/github/actions/workflow/status/habedi/ricevm/tests.yml?label=tests&style=flat&labelColor=282c34&color=4caf50&logo=github)](https://github.com/habedi/ricevm/actions/workflows/tests.yml)
[![Code Coverage](https://img.shields.io/codecov/c/github/habedi/ricevm?style=flat&labelColor=282c34&color=ffca28&logo=codecov)](https://codecov.io/gh/habedi/ricevm)
[![Docs](https://img.shields.io/badge/docs-latest-007ec6?style=flat&labelColor=282c34&logo=readthedocs)](docs)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-007ec6?style=flat&labelColor=282c34&logo=open-source-initiative)](https://github.com/habedi/ricevm)
[![Release](https://img.shields.io/github/release/habedi/ricevm.svg?label=release&style=flat&labelColor=282c34&logo=github)](https://github.com/habedi/ricevm/releases/latest)

RiceVM is an implementation of the [Dis virtual machine](https://www.inferno-os.org/inferno/papers/dis.pdf) in Rust.
The Dis virtual machine is a register machine that executes bytecode compiled from
the [Limbo programming language](https://inferno-os.org/inferno/papers/limbo.html).

### Features

- All 176 Dis VM opcodes implemented and audited against the reference C implementation
- Built-in Limbo compiler: compile `.b` source files to `.dis` bytecode without external tools
- 546/844 (65%) pre-compiled Inferno programs pass; 159/159 Limbo source files parse
- Built-in modules: `$Sys`, `$Math`, `$Draw` (SDL2), `$Tk` (widget toolkit), `$Keyring` (MD5, SHA1), and `$Crypt`
- Cooperative threading with channels, spawn, and non-blocking stdin
- Mark-and-sweep garbage collector with reference counting
- GUI support via SDL2 (optional `gui` feature) with embedded bitmap font rendering
- Audio support via cpal (optional `audio` feature)
- Interactive debugger with breakpoints, single-stepping, and stack inspection
- Disassembler for `.dis` module files
- Cross-platform: runs on Linux, macOS, and Windows

See [ROADMAP.md](ROADMAP.md) for the full list of implemented and planned features.

> [!IMPORTANT]
> RiceVM is still in early development, so bugs and breaking changes are expected.
> Please use the [issues page](https://github.com/habedi/ricevm/issues) to report bugs or request features.

---

### Quickstart

```bash
# Clone the repository
git clone --recursive --depth=1 https://github.com/habedi/ricevm.git
cd ricevm

# Build RiceVM from source
cargo build --release

# Run a pre-compiled Inferno program
cargo run -p ricevm-cli -- run external/inferno-os/dis/echo.dis \
    --probe external/inferno-os/dis -- hello world

# Disassemble a .dis module
cargo run -p ricevm-cli -- dis external/inferno-os/dis/echo.dis

# Create a Hello world program in Limbo language (`hello.b`)
cat > hello.b << 'EOF'
implement Hello;

include "sys.m";
include "draw.m";

Hello: module {
    init: fn(ctxt: ref Draw->Context, argv: list of string);
};

init(ctxt: ref Draw->Context, argv: list of string) {
    sys := load Sys Sys->PATH;
    sys->print("hello world\n");
}
EOF

# Compile the Limbo program using the built-in compiler
cargo run -p ricevm-cli -- compile hello.b

# Run the compiled program
cargo run -p ricevm-cli -- run hello.dis --probe external/inferno-os/dis

# Or compile using the reference Inferno Limbo compiler (runs on RiceVM itself)
cargo run -p ricevm-cli -- run external/inferno-os/dis/limbo.dis \
    --probe external/inferno-os/dis --probe external/inferno-os/dis/lib \
    -- -I external/inferno-os/module hello.b

# Run the compiled program
cargo run -p ricevm-cli -- run hello.dis --probe external/inferno-os/dis

# Run with instruction tracing (useful for debugging)
cargo run -p ricevm-cli -- run external/inferno-os/dis/echo.dis \
    --probe external/inferno-os/dis --trace -- hello world

# Run the About Inferno dialog with GUI (requires SDL2)
cargo run -p ricevm-cli --release --features gui -- run external/inferno-os/dis/wm/about.dis \
    --probe external/inferno-os/dis \
    --probe external/inferno-os/dis/lib \
    --root external/inferno-os
```

---

### Architecture

RiceVM consists of the following crates:

| Crate                                   | Purpose                                                                                      |
|-----------------------------------------|----------------------------------------------------------------------------------------------|
| [ricevm-core](crates/ricevm-core)       | Core shared types, including `Module`, `Opcode`, `Instruction`, `TypeDescriptor`, and errors |
| [ricevm-loader](crates/ricevm-loader)   | `.dis` binary format parser                                                                  |
| [ricevm-execute](crates/ricevm-execute) | Execution engine: 176 opcodes, heap, GC, built-in modules, and threading                    |
| [ricevm-limbo](crates/ricevm-limbo)     | Built-in Limbo compiler: lexer, parser, code generator, and `.dis` binary writer             |
| [ricevm-cli](crates/ricevm-cli)         | CLI with `run`, `compile`, `dis`, and `debug` subcommands                                    |

---

### Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for details on how to make a contribution.

### License

This project is licensed under either of these:

* MIT License ([LICENSE-MIT](LICENSE-MIT))
* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

### Acknowledgements

* The logo is from [SVG Repo](https://www.svgrepo.com/svg/293420/hexagon) with some modifications.
* This project uses various things from the [Inferno OS](https://github.com/inferno-os/inferno-os) project including the Limbo compiler and Dis
  virtual machine implementation as reference for testing and verifying implementation correctness.
