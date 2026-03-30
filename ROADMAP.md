## Project Roadmap

This document outlines the features implemented in RiceVM and the future goals for the project.

> [!IMPORTANT]
> This roadmap is a work in progress and is subject to change.

### Module Loading

- [x] `.dis` binary format parsing (header, code, type descriptors, data, and module name)
- [x] Operand address decoding (immediate, indirect, and double-indirect modes)
- [x] Type descriptor parsing with pointer map reconstruction
- [x] Link and import section loading
- [x] File-based module loading (load `.dis` files from disk at runtime)
- [x] Built-in module registration and dispatch
- [x] Signature-based function mapping for both built-in and loaded modules
- [x] Multi-module execution with MP swap (persistent state across calls)
- [x] Current module tracking for correct import table and type resolution
- [x] Inferno path stripping (`/dis/lib/` prefix) for module resolution
- [x] Complete module signature and runtime flag validation

### Instruction Set

- [x] Arithmetic: word, byte, big, float variants with wrapping semantics
- [x] Comparison and branching: `if src OP mid, goto dst` (correct operand order)
- [x] Load, store, and move: `movw`, `movb`, `movf`, `movl`, `movp` (with ref counting), `movm`, `movmp`
- [x] Type conversions: all `cvt*` variants including string/real/big/word
- [x] Bitwise and shift: word, byte, and big variants including logical shift right
- [x] Control flow: `call`, `ret`, `frame`, `jmp`, `exit`, `goto`, `casew`, `casec`, `casel`, `raise`
- [x] String operations: `lenc`, `indc`, `insc`, `addc`, `slicec`, `cvtca`, `cvtac`
- [x] List operations: `cons*`, `head*`, and `tail` for all element types
- [x] `lenl` list-length semantics
- [x] Array operations: `indx`, `indw`, `indf`, `indb`, `indl` (array indexing via heap refs), `newa`, `slicea`
- [x] Module operations: `load`, `mcall`, `mframe` with signature-based dispatch
- [x] Memory allocation: `new`, `newz`, `newa`, `newaz`, `mnewz`, and channel allocation
- [x] Fixed-point arithmetic: `mulx`, `divx`, `cvtxx` variants
- [x] Exponentiation: `expw`, `expl`, `expf`
- [x] Concurrency: `spawn` and `mspawn` (cooperative inline), `send`/`recv` (single-slot buffered), and `alt`/`nbalt` (simplified table scan)

### Type System

- [x] Dis primitive types (byte, word, big, and real)
- [x] Pointer type with heap object tracking and reference counting
- [x] String type with full Unicode support and copy-on-write
- [x] Array type with bounds checking and heap array references
- [x] List type (singly linked with typed head values)
- [x] Channel type (allocation and simplified send and receive)
- [x] ADT (abstract data type) support
- [x] Tuple and reference types

### Memory Management

- [x] Stack frame allocation and deallocation with two-phase push
- [x] Heap allocation for dynamic types (records, arrays, strings, lists, channels, and module refs)
- [x] Reference counting for deterministic destruction
- [x] Mark-and-sweep garbage collector (scans frames, MP, and all loaded module MPs)
- [x] Optional toggle to disable mark-and-sweep collection (`--no-gc` flag)
- [x] Bounds-safe memory access (out-of-bounds reads return 0; writes are no-ops)

### Scheduler

- [x] Cooperative thread scheduler infrastructure (round-robin with quanta)
- [x] Thread spawn (cooperative: inline execution until return)
- [x] Channel storage with single-slot buffered payloads
- [x] `alt` and `nbalt` simplified selection over single-slot channel buffers
- [x] Full preemptive thread scheduling with OS thread pool
- [x] Blocking channel synchronization with thread wake/sleep

### Built-in Modules

- [x] `$Sys` module (43 functions, 27 real implementations, 16 stubs)
    - I/O: `print`, `fprint`, `sprint`, `aprint`, `open`, `create`, `read`, `write`, `seek`, `fildes`, `fd2path`, `dup`, `pipe`
    - Utilities: `millisec`, `sleep`, `pctl`, `tokenize`, `byte2char`, `char2byte`, `utfbytes`, `chdir`, `remove`, `iounit`
    - All functions have correct signature hashes from the C++ Sysmodtab
- [x] `$Math` module (66 functions, 42+ real implementations)
    - Trig, log, exp, pow, sqrt, floor, ceil, hypot, bit conversions, and more
- [x] `$Draw` module (62 functions registered, SDL2 backend behind `gui` feature, many operations still stubbed)
    - `Display.allocate`: opens an SDL2 window and creates proper Display/Image/Screen ADT records
    - `Image.draw`, `Image.line`, `Image.ellipse`, `Image.flush`: basic SDL2 rendering
    - `Display.getwindow`, `Screen.allocate`, `Screen.newwindow`: proper ADT record creation
    - `Font.open`, `Font.width`: default font metrics
- [x] `$Tk` module (10 functions registered, several signatures still placeholders)
    - `toplevel`: creates Toplevel ADT with display, wreq channel, image, and screen rect
    - `cmd`: processes Tk command strings (widget creation logging, SDL2 update)
    - `namechan`, `pointer`, `keyboard`, `quote`, `color`
- [x] Exception handler table lookup for `raise` opcode
- [x] Signature-based function dispatch where built-in signatures are available, with positional fallback for placeholder signatures
- [x] `$Sys` module: `fstat`, `stat`, `bind`, `mount`, `dial`, `announce`, `listen`, `dirread`, and more
- [x] `$Draw` module: Tk widget rendering via SDL2

### GUI Support

- [x] SDL2 backend behind optional `gui` feature flag
- [x] `Display.allocate` creates SDL2 window with proper Display/Image/Screen ADTs
- [x] Proper Inferno ADT record layouts for Display, Image, Screen, Font, and Toplevel
- [x] Event loop integration via `Image.flush` (polls SDL events, handles window close)
- [x] Manual milestone: `wm/about.dis` loads, initializes, and enters the event loop without crashing
- [x] Tk widget rendering (label, button, frame, pack, and canvas)
- [x] Font rendering (monospace bitmap fallback; SDL2_ttf planned)
- [x] Mouse and keyboard event delivery to Tk

### Portability

- [x] Fully portable (no `libc` dependency; all I/O via `std::fs` and `std::io`)
- [x] Compiles on Linux, macOS, and Windows
- [x] SDL2 GUI is optional (`--features gui`)

### CLI

- [x] Argument parsing via `clap`
- [x] Tracing and logging setup via `tracing-subscriber`
- [x] `run` subcommand to execute a `.dis` module file
- [x] `dis` subcommand for human-readable disassembly
- [x] `--probe` flag to add module probing paths
- [x] `--trace` flag for instruction-level debugging
- [x] `--no-gc` flag to disable mark-and-sweep garbage collection
- [x] `--threads` flag to configure scheduler thread pool size
- [x] Debugger integration (breakpoints, single-stepping, and stack inspection)

### Development and Testing

- [x] Cargo workspace with modular crate structure
- [x] CI pipeline with automated tests
- [x] Dual license (MIT and Apache 2.0)
- [x] Unit tests for instruction decoding and execution
- [x] End-to-end pipeline tests with hand-crafted `.dis` binaries
- [x] Integration tests with real Inferno OS `.dis` modules (`echo.dis` and `cat.dis`)
- [x] Property-based tests for binary format parsing (`property_tests.rs` still contains ignored placeholders)
- [x] Fuzz testing setup for the module loader (`cargo-fuzz` with `libfuzzer`)
- [x] 866 pre-compiled `.dis` files available via `external/inferno-os` submodule
- [x] C++ DisVM reference compiled and available in `tmp/disvm/build/`

### Documentation

- [x] Quickstart guide (in README)
- [x] Mapping from Dis VM specification to RiceVM internals
- [ ] Examples with precompiled Limbo programs
