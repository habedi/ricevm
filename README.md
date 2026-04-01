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

- **All Dis VM opcodes**: Arithmetic, branching, control flow, string, list, pointer, heap allocation,
  type conversions, fixed-point math, and module operations
- **Limbo compiler support**: Runs the Inferno `limbo.dis` compiler to compile `.b` source files to `.dis` bytecode,
  then executes the output
- **Built-in modules**: `$Sys` (I/O, formatting, networking), `$Math` (trig, linear algebra),
  `$Draw` (SDL2 rendering), `$Tk` (widget toolkit), and `$Crypt` (MD5)
- **Built-in disassembler**: `ricevm dis` prints human-readable module contents
- **Instruction tracing**: Set `RICEVM_TRACE=1` for step-by-step execution output

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

# Build
cargo build --release

# Run a pre-compiled .dis module
cargo run -p ricevm-cli -- run program.dis --probe external/inferno-os/dis

# Disassemble a .dis module
cargo run -p ricevm-cli -- dis program.dis

# Compile a Limbo source file and run the output
cargo run -p ricevm-cli -- run external/inferno-os/dis/limbo.dis \
    --probe external/inferno-os/dis --probe external/inferno-os/dis/lib \
    -- -I external/inferno-os/module hello.b
cargo run -p ricevm-cli -- run hello.dis --probe external/inferno-os/dis

# Run with instruction tracing (for debugging)
RICEVM_TRACE=1 cargo run -p ricevm-cli -- run program.dis
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
