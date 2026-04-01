# RiceVM

**A Dis virtual machine and Limbo compiler in Rust.**

---

RiceVM is a re-implementation of the [Dis virtual machine](https://www.inferno-os.org/inferno/papers/dis.pdf) in Rust.
The Dis VM is a register machine that executes bytecode compiled from the
[Limbo programming language](https://inferno-os.org/inferno/papers/limbo.html),
originally designed for the [Inferno operating system](https://en.wikipedia.org/wiki/Inferno_(operating_system)).

## Highlights

- **All 176 Dis VM opcodes** implemented and audited against the reference C implementation
- **Built-in Limbo compiler** that compiles `.b` source files to `.dis` bytecode without external tools
- **546/844 pre-compiled Inferno programs pass** (65%); 159/159 Limbo source files parse (100%)
- **Built-in modules**: `$Sys`, `$Math`, `$Draw` (SDL2), `$Tk`, `$Keyring` (MD5, SHA1), and `$Crypt`
- **Cooperative threading** with channels, spawn, and non-blocking stdin
- **Mark-and-sweep garbage collector** with reference counting
- **GUI support** via SDL2 with embedded bitmap font rendering (optional `gui` feature)
- **Audio support** via cpal (optional `audio` feature)
- **Interactive debugger** with breakpoints, single-stepping, and stack inspection
- **Cross-platform**: Linux, macOS, and Windows

## Quick Example

```bash
# Write a Limbo program
cat > hello.b << 'EOF'
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
EOF

# Compile and run
ricevm-cli compile hello.b
ricevm-cli run hello.dis --probe external/inferno-os/dis
```

## Project Structure

| Crate | Purpose |
|-------|---------|
| `ricevm-core` | Shared types: `Module`, `Opcode`, `Instruction`, `TypeDescriptor`, and errors |
| `ricevm-loader` | `.dis` binary format parser |
| `ricevm-execute` | Execution engine: 176 opcodes, heap, GC, built-in modules, and threading |
| `ricevm-limbo` | Built-in Limbo compiler: lexer, parser, code generator, and `.dis` writer |
| `ricevm-cli` | CLI with `run`, `compile`, `dis`, and `debug` subcommands |
