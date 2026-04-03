<div align="center">
  <picture>
    <img alt="Project Logo" src="docs/assets/logo.svg" height="25%" width="25%">
  </picture>
<br>

<h2>RiceVM</h2>

[![Tests](https://img.shields.io/github/actions/workflow/status/habedi/ricevm/tests.yml?label=tests&style=flat&labelColor=282c34&logo=github)](https://github.com/habedi/ricevm/actions/workflows/tests.yml)
[![Code Coverage](https://img.shields.io/codecov/c/github/habedi/ricevm?label=coverage&style=flat&labelColor=282c34&logo=codecov)](https://codecov.io/gh/habedi/ricevm)
[![Docs](https://img.shields.io/badge/docs-read-007ec6?label=docs&style=flat&labelColor=282c34&logo=readthedocs)](https://habedi.github.io/ricevm/)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-007ec6?style=flat&labelColor=282c34&logo=open-source-initiative)](https://github.com/habedi/ricevm/)
[![Release](https://img.shields.io/github/release/habedi/ricevm.svg?label=release&style=flat&labelColor=282c34&logo=github)](https://github.com/habedi/ricevm/releases/latest)

A Dis virtual machine and Limbo compiler implemented in Rust

</div>

---

RiceVM is a cross-platform [Dis virtual machine](https://www.inferno-os.org/inferno/papers/dis.pdf) and
[Limbo](https://inferno-os.org/inferno/papers/limbo.html) compiler implemented in Rust.

### Features

- Supports all 176 Dis VM opcodes
- Provides a fully functional Dis runtime (with GC, concurrency, etc.)
- Includes a Limbo compiler, `.dis` file disassembler, and debugger
- Includes most of the built-in modules from Dis virtual machine, including `$Sys`, `$Math`, `$Crypt`, etc.
- Supports for GUI applications and audio playback
- Is fully cross-platform (runs on Windows, Linux, and macOS)

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

# Disassemble a `.dis` file
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

# Or compile using the original Inferno Limbo compiler (runs on RiceVM itself)
cargo run -p ricevm-cli -- run external/inferno-os/dis/limbo.dis \
    --probe external/inferno-os/dis --probe external/inferno-os/dis/lib \
    -- -I external/inferno-os/module hello.b

# Run the compiled program
cargo run -p ricevm-cli -- run hello.dis --probe external/inferno-os/dis

# Run with instruction tracing (this is useful for debugging)
cargo run -p ricevm-cli -- run external/inferno-os/dis/echo.dis \
    --probe external/inferno-os/dis --trace -- hello world

# Run the About Inferno dialog with GUI (this needs SDL2)
cargo run -p ricevm-cli --release --features gui -- run external/inferno-os/dis/wm/about.dis \
    --probe external/inferno-os/dis --probe external/inferno-os/dis/lib \
    --root external/inferno-os
```

#### Simple Demo

[![asciicast](https://asciinema.org/a/BM2tUfsqKEwqPsxD.svg)](https://asciinema.org/a/BM2tUfsqKEwqPsxD)

#### Pre-built Binaries

The latest release of RiceVM can be downloaded from [here](https://github.com/habedi/ricevm/releases/latest) insead of building it from source.

---

### Documentation

Check the [RiceVM documentation](https://habedi.github.io/ricevm/) for usage examples and API reference for the CLI and built-in Limbo modules, etc.

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
