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

RiceVM is an implementation of the [Dis virtual machine](https://en.wikipedia.org/wiki/Limbo_(programming_language)#Virtual_machine) in Rust.
The Dis virtual machine is a register machine that executes bytecode compiled from
the [Limbo programming language](https://en.wikipedia.org/wiki/Limbo_(programming_language)).

### Features

- Supports all 176 Dis VM opcodes
- Supports running `.dis` files, inclduing the Limbo compiler support from Inferno OS
- Includes built-in modules like `$Sys`, `$Math`, `$Draw`, and `$Tk` with SDL2 backend
- Includes a built-in debuagger and a disassembler for `.dis` files
- Cross-platform; supports running `.dis` files on Windows, Linux, and macOS

See [ROADMAP.md](ROADMAP.md) for the list of implemented and planned features.

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

# Compile the Limbo program to Dis bytecode (`hello.dis`) using the Inferno Limbo compiler
cargo run -p ricevm-cli -- run external/inferno-os/dis/limbo.dis \
    --probe external/inferno-os/dis --probe external/inferno-os/dis/lib \
    -- -I external/inferno-os/module hello.b

# Run the Dis bytecode program (`hello.dis`)
cargo run -p ricevm-cli -- run hello.dis --probe external/inferno-os/dis

# Run the `echo` program in Inferno OS with instruction tracing enabled (good for debugging)
RICEVM_TRACE=1 cargo run -p ricevm-cli -- run external/inferno-os/dis/echo.dis \
    --probe external/inferno-os/dis -- hi
```

---

### Architecture

RiceVM is consists of the following main components:

| Crate                                   | Purpose                                                                                      |
|-----------------------------------------|----------------------------------------------------------------------------------------------|
| [ricevm-core](crates/ricevm-core)       | Core shared types, including `Module`, `Opcode`, `Instruction`, `TypeDescriptor`, and errors |
| [ricevm-loader](crates/ricevm-loader)   | `.dis` binary format parser                                                                  |
| [ricevm-execute](crates/ricevm-execute) | Bytecode loader, runner, and runtime                                                         |
| [ricevm-cli](crates/ricevm-cli)         | CLI frontend to run `.dis` files                                                             |

---

### Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for details on how to make a contribution.

### License

This project is licensed under either of these:

* MIT License ([LICENSE-MIT](LICENSE-MIT))
* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

### Acknowledgements

* The logo is from [SVG Repo](https://www.svgrepo.com/svg/293420/hexagon) with some modifications.
