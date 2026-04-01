# AGENTS.md

This file provides guidance to coding agents collaborating on this repository.

## Mission

RiceVM is a re-implementation of the Dis virtual machine in Rust.
The Dis VM is a register machine that executes bytecode compiled from the Limbo programming language.
Priorities, in order:

1. Correct implementation of the Dis VM specification.
2. Clean, idiomatic Rust with safe abstractions over VM internals.
3. Clear separation of concerns across workspace crates.
4. Maintainable and well-tested code.

## Core Rules

- Use English for code, comments, docs, and tests.
- Keep `unsafe` usage minimal and well-documented; prefer safe Rust wherever possible.
- Never use `.unwrap()` or `.expect()` in non-test code (enforced by `make lint`). Production code should never panic.
- Prefer small, focused changes over large refactoring.
- Add comments only when they clarify non-obvious behavior.
- Do not add features, error handling, or abstractions beyond what is needed for the current task.
- Add tests for every bug fix and new feature to prevent regression.

## Writing Style

- Use Oxford commas in inline lists: "a, b, and c" not "a, b, c".
- Do not use em dashes. Restructure the sentence, or use a colon or semicolon instead.
- Avoid colorful adjectives and adverbs. Write "TCP proxy" not "lightweight TCP proxy", "scoring components" not "transparent scoring components".
- Use noun phrases for checklist items, not imperative verbs. Write "redundant index detection" not "detect redundant indexes".
- Headings in Markdown files must be in the title case: "Build from Source" not "Build from source". Minor words (a, an, the, and, but, or, for, in,
  on, at, to, by, of) stay lowercase unless they are the first word.

## Repository Layout

- `crates/ricevm-core/`: Shared types (Module, Opcode, Instruction, TypeDescriptor, and errors). No runtime logic.
- `crates/ricevm-loader/`: Binary format parser for `.dis` module files. One public function: `load(&[u8]) -> Result<Module, LoadError>`.
- `crates/ricevm-execute/`: Execution engine with 176 opcode handlers, heap, GC, built-in modules ($Sys, $Math, $Draw, $Tk, $Keyring, $Crypt, and audio),
  virtual device files, and file-based module loading.
- `crates/ricevm-cli/`: CLI with `run` and `dis` subcommands.
- `external/inferno-os/`: Git submodule of the Inferno OS repository (866 pre-compiled `.dis` files, Limbo source, and reference VM source in
  `libinterp/xec.c` for correctness validation).
- `Makefile`: GNU Make wrapper around `cargo` commands (`make test`, `make build`, `make lint`, etc.).
- `rust-toolchain.toml`: Pinned Rust toolchain (1.92.0) with `rustfmt`, `clippy`, and `rust-analyzer`.

## Architecture

### Crate Dependency Graph

```
ricevm-cli
├── ricevm-core
├── ricevm-loader → ricevm-core
└── ricevm-execute → ricevm-core and ricevm-loader
```

`ricevm-execute` depends on `ricevm-loader` for runtime module loading (the `load` opcode reads `.dis` files from disk).

### Key Internal Modules in `ricevm-execute`

| Module         | Purpose                                                                                |
|----------------|----------------------------------------------------------------------------------------|
| `vm.rs`        | `VmState` struct, execution loop with cooperative threading, and thread suspend/resume |
| `frame.rs`     | `FrameStack` with two-phase push (`alloc_pending` and `activate_pending`)              |
| `heap.rs`      | `Heap` with reference counting, copy-on-write strings, and `ArraySlice` shared views   |
| `gc.rs`        | Mark-and-sweep garbage collector (scans frames, MP, and loaded module MPs)             |
| `address.rs`   | Operand resolution with `ModuleMp` virtual ranges and `decode_virtual_addr`            |
| `memory.rs`    | Typed read/write on byte buffers with bounds checking                                  |
| `data.rs`      | Module data (MP) initialization with type-aware elem sizes and nested arrays           |
| `filetab.rs`   | Portable file descriptor table with in-memory pipe, virtual device files, and non-blocking stdin |
| `ops/`         | 176 instruction handlers organized by category                                         |
| `sys.rs`       | Built-in `$Sys` module (43 functions with tuple return support, `%b`, `%u`, and `%.*`) |
| `math.rs`      | Built-in `$Math` module (66 functions including linear algebra)                        |
| `draw.rs`      | Built-in `$Draw` module (SDL2 backend, optional `gui` feature)                         |
| `tk.rs`        | Built-in `$Tk` module (widget toolkit with embedded bitmap font and SDL2 rendering)    |
| `audio.rs`     | `/dev/audio` and `/dev/audioctl` support (cpal backend, optional `audio` feature)      |
| `builtin.rs`   | `ModuleRegistry` for built-in module registration with name and signature lookup       |
| `scheduler.rs` | Preemptive thread scheduler infrastructure (not yet connected to main loop)            |
| `channel.rs`   | Channel data structure for inter-thread communication                                  |

### Key Design Decisions

- Package names use hyphens (`ricevm-core`); Rust identifiers use underscores (`ricevm_core`).
- The heap uses `HashMap<u32, HeapObject>` with monotonic IDs starting at `HEAP_ID_BASE` (0x0100_0000);
  pointers stored as `Word` (i32) in frames.
- Array element references use a `heap_refs` table with `HEAP_REF_FLAG` sentinel, resolved during double-indirect addressing.
- `ArraySlice` heap type provides shared-storage views into parent arrays (required for Bufio buffer semantics);
  all operations (`cvtac`, `slicela`, `pread`, `pwrite`, channel send/recv) resolve slices to their parent.
- `slicela` adjusts reference counts for pointer-containing elements (e.g., `array of string`) to prevent
  premature freeing during operations like mergesort.
- Unified virtual address space: frame addresses are low, each module's MP has a unique range starting at
  `MP_BASE` (0x0080_0000) with `MP_STRIDE` (0x0010_0000) between modules, heap IDs above `HEAP_ID_BASE`.
  The `decode_virtual_addr()` function decodes addresses back to `AddrTarget`.
- `caller_mp_stack` in `VmState` tracks caller module MPs during loaded module execution.
- Branch instructions: `if src OP mid, goto dst` (not `if src OP dst, goto mid`).
- Case instructions (`casew`, `casec`, and `casel`) use binary search matching the reference Dis VM (`xec.c`).
- `casel` entries are 24 bytes (not 20) due to LONG alignment padding.
- Multi-module execution: loaded modules' MPs are swapped (not cloned) to persist state across calls.
- Built-in function dispatch prefers name matching over signature hash matching (avoids collisions like read/write).
- `movmp` looks up the type descriptor to determine copy size (mid is a type index, not a byte count).
- Data initialization (`data.rs`) uses type descriptors for array element sizes and writes to the active buffer context
  (parent array or MP) for correct nested array initialization.
- Builtin return value copy in `mcall` transfers 4 bytes to the caller's return pointer; big-returning functions
  (like `seek`) and tuple-returning functions write their results directly through the return pointer.
- Type conversions match the reference: `cvtfw`/`cvtfl` round (±0.5), `cvtrf`/`cvtfr` convert f32↔f64,
  `cvtwc`/`cvtcw` format/parse decimal strings, `cvtfc` uses `%g` format.
- `expw`/`expl`/`expf` use repeated-squaring with base from mid and integer exponent from src.
- Cooperative threading: `spawn` creates threads in a queue, the run loop rotates every 2048 instructions,
  `recv` on empty channels blocks the thread, `send` on full channels blocks the thread,
  both directions unblock on state change. Cloned MP adjusts heap ref counts.
- Alt table format: `{nsend, nrecv}` header followed by 8-byte `{channel_ptr, data_ptr}` entries.
- Per-thread error string (`last_error`) implements the `werrstr`/`%r` mechanism.
- Portable I/O via `FileTable` (no `libc` dependency) with in-memory pipe support and non-blocking stdin
  (background reader thread prevents `sys->read` from freezing all VM threads).
- Virtual device files: `/dev/sysctl`, `/dev/sysname`, `/dev/user`, `/dev/time`, `/dev/cons`, `/dev/null`,
  `/dev/random`, `/dev/drivers`, `/prog/N/{status,wait,ns,ctl}`, and `/env/*`.
- `$Keyring` module provides real MD5 and SHA1 digests via the `md-5` and `sha1` crates;
  auth functions (`readauthinfo`, `auth`, etc.) are stubs returning nil.
- Nil dereferences (channel, array, string, list) are caught in the run loop and dispatched to
  exception handler tables, matching the reference Dis VM behavior.
- Library modules with `entry_pc = -1` return success immediately (no init function).
- SDL2 for GUI is behind an optional `gui` feature flag.
- Audio support (`/dev/audio` and `/dev/audioctl`) is behind an optional `audio` feature flag (cpal backend).
- `export_real`/`export_real32` use correct argument order (address, then value).

### Reference Implementation

The original Dis VM source is in `external/inferno-os/libinterp/`:

| File       | Lines | What It Covers                                        |
|------------|-------|-------------------------------------------------------|
| `xec.c`    | 1698  | All 176 opcode handlers (the authoritative reference) |
| `alt.c`    | 294   | Channel alt/nbalt implementation                      |
| `string.c` | 616   | String operations                                     |
| `gc.c`     | 383   | Garbage collector                                     |
| `heap.c`   | 533   | Heap allocation and ref counting                      |
| `math.c`   | 955   | Math module                                           |

Always compare against these files when fixing instruction correctness issues.

## Required Validation

Run `make lint` and `make test` for any change. Key targets:

| Target   | Command         | What It Runs                                                                    |
|----------|-----------------|---------------------------------------------------------------------------------|
| Format   | `make format`   | `cargo fmt`                                                                     |
| Lint     | `make lint`     | `cargo clippy` with `-D warnings -D clippy::unwrap_used -D clippy::expect_used` |
| Test     | `make test`     | All workspace tests with `--nocapture`                                          |
| Build    | `make build`    | Release build                                                                   |
| Coverage | `make coverage` | `cargo tarpaulin` with XML and HTML output                                      |
| Audit    | `make audit`    | `cargo audit` on dependencies                                                   |

## Testing Expectations

- Unit tests live in each crate's source files using `#[cfg(test)]` modules.
- Integration tests for the loader live in `crates/ricevm-loader/tests/`.
- Pipeline tests (loader to executor) live in `crates/ricevm-cli/tests/`, including tests with real Inferno `.dis` files.
- Property-based tests cover arithmetic commutativity/associativity, string slice bounds, and conversion roundtrips.
- Regression tests exist for every major bug fix (movmp, casew, slicea, slicela ref counting, cvtac ArraySlice, byte2char, channel blocking, spawn, etc.).
- Fuzz testing for the loader is set up in `crates/ricevm-loader/fuzz/`.
- No public API change is complete without a corresponding test.
- The Limbo compiler (`external/inferno-os/dis/limbo.dis`) can compile and run programs as an end-to-end validation:
  ```
  ricevm-cli run external/inferno-os/dis/limbo.dis \
      --probe external/inferno-os/dis --probe external/inferno-os/dis/lib \
      -- -I external/inferno-os/module hello.b
  ricevm-cli run hello.dis --probe external/inferno-os/dis
  ```

## Commit and PR Hygiene

- Keep commits scoped to one logical change.
- PR descriptions should include:
    1. Behavioral change summary.
    2. Tests added or updated.
    3. `make lint && make test` passes (yes/no).

Suggested PR checklist:

- [ ] Unit tests added or updated for logic changes
- [ ] Integration test added for new user-facing behavior
- [ ] `make lint && make test` passes
- [ ] Docs or README updated (if API surface changed)
