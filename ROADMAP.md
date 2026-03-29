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
- [x] Built-in module registration and dispatch

### Instruction Set

- [x] Arithmetic operations (`addw`, `subw`, `mulw`, `divw`, `modw`, and float/big/byte variants)
- [x] Comparison and branching (`beqw`, `bnew`, `bltw`, `bgtw`, `jmp`, and float/big/byte variants)
- [x] Load and store operations (`movb`, `movw`, `movf`, `movl`)
- [x] Type conversions (`cvtbw`, `cvtwb`, `cvtfw`, `cvtwf`, `cvtwl`, `cvtlw`, `cvtlf`, `cvtfl`)
- [x] Bitwise operations (`andw`, `orw`, `xorw`, `shlw`, `shrw`, `lsrw`)
- [x] Control flow (`call`, `ret`, `frame`, `jmp`, `exit`, `nop`)
- [x] String operations (`lenc`, `indc`, `insc`, `addc`, `slicec`, `cvtca`, `cvtac`)
- [x] Module operations (`load`, `mcall`, `mframe`)
- [x] Memory allocation (`new`, `newz`, `newa`, `newaz`)
- [x] Pointer operations (`movp`, `lea`, `indx`, `indw`, `indf`, `indb`, `indl`, `lena`)
- [x] String comparisons (`beqc`, `bnec`, `bltc`, `blec`, `bgtc`, `bgec`)
- [x] List operations (`consb`, `consw`, `consp`, `consf`, `consl`, `consm`, `consmp`, `headb`, `headw`, `headp`, `headf`, `headl`, `headm`, `headmp`, `tail`)
- [x] Memory block operations (`movm`, `movmp`, `movpc`)
- [x] Channel allocation stubs (`newcb`, `newcw`, `newcf`, `newcp`, `newcm`, `newcmp`, `newcl`)
- [x] Additional conversions (`cvtwc`, `cvtcw`, `cvtfc`, `cvtcf`, `cvtlc`, `cvtcl`, `cvtws`, `cvtsw`)
- [x] Additional bitwise (`andb`, `orb`, `xorb`, `shlb`, `shrb`, `andl`, `orl`, `xorl`, `shll`, `shrl`, `lsrl`)
- [x] Exponentiation (`expw`, `expl`, `expf`)
- [x] Misc (`tcmp`, `self_`, `mnewz`, `lenl`)
- [ ] Array slice operations (`slicea`, `slicela`)
- [ ] Channel operations (`send`, `recv`, `alt`, `nbalt`)
- [ ] Thread operations (`spawn`, `mspawn`)
- [ ] Exception handling (`raise`, `rescued`, `casel`, `casew`, `casec`)
- [ ] Remaining opcodes (`goto`, `runt`, `casew`, `casec`, `casel`, `eclr`, `cvtrf`, `cvtfr`, `cvtxx`, `mulx`, `divx`, `brkpt`)

### Type System

- [x] Dis primitive types (byte, word, big, real)
- [x] Pointer type with heap object tracking
- [x] String type with full Unicode support
- [x] Array type with bounds checking
- [x] List type (singly linked)
- [ ] Channel type with synchronous send and receive
- [ ] ADT (abstract data type) support
- [ ] Tuple and reference types
- [ ] Type-safe pointer representation

### Memory Management

- [x] Stack frame allocation and deallocation
- [x] Heap allocation for dynamic types (arrays, strings)
- [x] Reference counting for deterministic destruction
- [ ] Mark-and-sweep garbage collector for cyclic reference detection
- [ ] Optional toggle to disable mark-and-sweep collection

### Scheduler

- [ ] Cooperative thread scheduling with configurable quanta
- [ ] Thread spawn and exit
- [ ] Channel-based inter-thread communication and synchronization
- [ ] `alt` statement support for multiplexed channel operations
- [ ] Configurable system thread pool (1 to N OS threads)

### Built-in Modules

- [x] `Sys` module (partial): `print` with format string support
- [ ] `Sys` module: `fprint`, `open`, `read`, `write`, `seek`, `filstat`, `fd2path`, etc.
- [ ] `Math` module (partial): basic floating point operations
- [x] `$Sys` module type descriptors and entry points
- [x] Extension mechanism for registering custom built-in modules

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
- [x] End-to-end pipeline tests (loader → executor) with hand-crafted `.dis` binaries
- [ ] Integration tests with Limbo-compiled `.dis` modules
- [x] Property-based tests for binary format parsing
- [ ] Fuzz testing for the module loader
- [ ] Benchmarks against the reference C++ DisVM implementation

### Documentation

- [ ] Quickstart guide
- [ ] Architecture overview (crate responsibilities and data flow)
- [ ] Supported Dis opcodes and built-in module coverage matrix
- [ ] Mapping from Dis VM specification to RiceVM internals
- [ ] Examples with precompiled Limbo programs
