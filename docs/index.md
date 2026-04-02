# RiceVM

<p align="center">
  <img src="assets/logo.svg" alt="Project Logo" width="250" />
</p>

---

RiceVM is a cross-platform [Dis virtual machine](https://www.inferno-os.org/inferno/papers/dis.pdf)
and [Limbo](https://inferno-os.org/inferno/papers/limbo.html) compiler implemented in Rust.

## Features

- Supports all 176 Dis VM opcodes
- Provides a fully functional Dis runtime (with GC, concurrency, etc.)
- Includes a Limbo compiler, `.dis` file disassembler, and debugger
- Includes most of the built-in modules from Dis virtual machine, including `$Sys`, `$Math`, `$Crypt`, etc.
- Supports for GUI applications and audio playback
- Is fully cross-platform (runs on Windows, Linux, and macOS)

## Architecture

| Crate            | Purpose                     |
|------------------|-----------------------------|
| `ricevm-core`    | Shared types and utilities  |
| `ricevm-loader`  | `.dis` binary format parser |
| `ricevm-execute` | Dis runtime                 |
| `ricevm-limbo`   | Limbo compiler              |
| `ricevm-cli`     | CLI frontend                |

## Documentation

- [Getting Started](getting-started.md)
- [Examples](examples.md)
- [API Reference](api-reference.md)
- [Limitations](limitations.md)
