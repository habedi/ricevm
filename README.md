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
The Dis virtual machine is a register machine that can execute programs written in
the [Limbo programming language](https://en.wikipedia.org/wiki/Limbo_(programming_language)).

### Features

- **Full instruction set**: All 176 Dis VM opcodes implemented (arithmetic, branching, control flow, string, list,
  pointer, heap allocation, type conversions, fixed-point math, and module operations)
- **Binary loader**: Parses `.dis` module files (header, code, type descriptors, data, exports, imports, and handlers)
- **Heap with reference counting**: Typed records, strings, arrays, lists, and module references with automatic
  memory management
- **Built-in Sys module**: `print`/`fprint`/`sprint` with printf-style formatting, file I/O (`open`, `read`, `write`,
  `create`), `tokenize`, `millisec`, `sleep`, `byte2char`, `utfbytes`, and more
- **Exception handling**: Handler table lookup with named and wildcard exception matching
- **Disassembler**: `ricevm dis` prints human-readable module contents
- **Instruction tracing**: Set `RICEVM_TRACE=1` for step-by-step execution output

---

### Quickstart

```bash
# Build
cargo build --release

# Run a .dis module
cargo run -p ricevm-cli -- run program.dis

# Disassemble a .dis module
cargo run -p ricevm-cli -- dis program.dis

# Run with instruction tracing
RICEVM_TRACE=1 cargo run -p ricevm-cli -- run program.dis
```

---

### Architecture

RiceVM is organized as a Cargo workspace with four crates:

| Crate | Purpose |
|---|---|
| `ricevm-core` | Shared types: `Module`, `Opcode`, `Instruction`, `TypeDescriptor`, error types |
| `ricevm-loader` | `.dis` binary format parser: `load(&[u8]) -> Result<Module, LoadError>` |
| `ricevm-execute` | Execution engine: `execute(&Module) -> Result<(), ExecError>` |
| `ricevm-cli` | CLI with `run` and `dis` subcommands |

Data flows through the `Module` struct defined in `ricevm-core`. The loader produces it from bytes;
the executor consumes it. Neither depends on the other.

---

### Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for details on how to make a contribution.

### License

This project is licensed under either of these:

* MIT License ([LICENSE-MIT](LICENSE-MIT))
* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

### Acknowledgements

* The logo is from [SVG Repo](https://www.svgrepo.com/svg/293420/hexagon) with some modifications.
