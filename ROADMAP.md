## Project Roadmap

This document outlines the features implemented in RiceVM and the future goals for the project.

> [!IMPORTANT]
> This roadmap is a work in progress and is subject to change.

### Module Loading

- [x] `.dis` binary format parsing (header, code, type descriptors, data, and module name)
- [x] Operand address decoding (immediate, indirect, and double-indirect modes)
- [x] Type descriptor parsing with pointer map reconstruction
- [x] Link and import section loading
- [ ] Module signature and runtime flag validation
- [ ] Module resolver with configurable probing paths
- [ ] Built-in module registration and dispatch

### Instruction Set

- [ ] Arithmetic operations (`addi`, `subi`, `muli`, `divi`, `modi`, and float/big variants)
- [ ] Comparison and branching (`beqi`, `bnei`, `blti`, `bgti`, `jmp`, `casew`)
- [ ] Load and store operations (`movb`, `movw`, `movf`, `movp`, `movm`)
- [ ] String operations (`insc`, `headc`, `lenc`, `addsc`, `slicec`, etc.)
- [ ] Array and list operations (`newa`, `lena`, `headb`, `headw`, `headp`, `cons`, `slice`, etc.)
- [ ] Channel operations (`send`, `recv`, `alt`, `nbalt`, `newc`)
- [ ] Control flow (`call`, `ret`, `frame`, `spawn`, `mspawn`, `mcall`, `mframe`, `exit`)
- [ ] Memory allocation (`new`, `newz`, `newa`, `newcb`, `newcw`, `newcf`, `newcp`, `newcm`, `newcmp`)
- [ ] Exception handling (`raisex`, `rescued`, `casel`, `caseh`)

### Type System

- [ ] Dis primitive types (byte, word, big, real, pointer)
- [ ] String type with full Unicode support
- [ ] Array type with bounds checking
- [ ] List type (singly linked)
- [ ] Channel type with synchronous send and receive
- [ ] ADT (abstract data type) support
- [ ] Tuple and reference types
- [ ] Type-safe pointer representation

### Memory Management

- [ ] Stack frame allocation and deallocation
- [ ] Heap allocation for dynamic types (arrays, lists, strings, channels, and ADTs)
- [ ] Reference counting for deterministic destruction
- [ ] Mark-and-sweep garbage collector for cyclic reference detection
- [ ] Optional toggle to disable mark-and-sweep collection

### Scheduler

- [ ] Cooperative thread scheduling with configurable quanta
- [ ] Thread spawn and exit
- [ ] Channel-based inter-thread communication and synchronization
- [ ] `alt` statement support for multiplexed channel operations
- [ ] Configurable system thread pool (1 to N OS threads)

### Built-in Modules

- [ ] `Sys` module (partial): `print`, `fprint`, `open`, `read`, `write`, `seek`, `filstat`, `fd2path`, etc.
- [ ] `Math` module (partial): basic floating point operations
- [ ] `$Sys` and `$Math` module type descriptors and entry points
- [ ] Extension mechanism for registering custom built-in modules

### CLI

- [x] Argument parsing via `clap`
- [x] Tracing and logging setup via `tracing-subscriber`
- [x] `run` subcommand to execute a `.dis` module file
- [ ] `--dis-gc` flag to enable or disable mark-and-sweep garbage collection
- [ ] `--threads` flag to configure scheduler thread pool size
- [ ] `--probe` flag to add module probing paths
- [ ] Debugger integration (breakpoints, single-stepping, and stack inspection)

### Development and Testing

- [x] Cargo workspace with modular crate structure (`rice-core`, `rice-loader`, `rice-execute`, `rice-cli`)
- [x] CI pipeline with automated tests
- [x] Dual license (MIT and Apache 2.0)
- [x] Unit tests for instruction decoding and execution
- [ ] Integration tests with precompiled `.dis` modules
- [x] Property-based tests for binary format parsing
- [ ] Fuzz testing for the module loader
- [ ] Benchmarks against the reference C++ DisVM implementation

### Documentation

- [ ] Quickstart guide
- [ ] Architecture overview (crate responsibilities and data flow)
- [ ] Supported Dis opcodes and built-in module coverage matrix
- [ ] Mapping from Dis VM specification to RiceVM internals
- [ ] Examples with precompiled Limbo programs
