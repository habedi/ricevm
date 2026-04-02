# RiceVM

<p align="center">
  <img src="https://raw.githubusercontent.com/habedi/ricevm/main/logo.svg" alt="Project Logo" width="200" />
</p>

---

RiceVM is a [Dis virtual machine](https://www.inferno-os.org/inferno/papers/dis.pdf) and [Limbo](https://inferno-os.org/inferno/papers/limbo.html)
compiler implemented in Rust.

## Features

- Supports all 176 Dis VM opcodes and a fully functional Dis runtime
- Includes a Limbo compiler, `.dis` file disassembler, and debugger
- Includes built-in modules from Dis virtual machine, including `$Sys`, `$Math`, `$Crypt`, etc.
- Supports for GUI applications and audeo playback
- Fully cross-platform (runs on Windows, Linux, and macOS)


## RiceVM Architecture

| Crate            | Purpose                         |
|------------------|---------------------------------|
| `ricevm-core`    | Core shared types and utilities |
| `ricevm-loader`  | `.dis` binary format parser     |
| `ricevm-execute` | Dis runtime                     |
| `ricevm-limbo`   | Limbo compiler                  |
| `ricevm-cli`     | CLI frontend                    |


## Documentation

- [Getting Started](getting-started.md)
- [Examples](examples.md)
- [API Reference](api-reference.md)
- [Limitations](limitations.md)
