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
- Prefer small, focused changes over large refactoring.
- Add comments only when they clarify non-obvious behavior.
- Do not add features, error handling, or abstractions beyond what is needed for the current task.

## Writing Style

- Use Oxford commas in inline lists: "a, b, and c" not "a, b, c".
- Do not use em dashes. Restructure the sentence, or use a colon or semicolon instead.
- Avoid colorful adjectives and adverbs. Write "TCP proxy" not "lightweight TCP proxy", "scoring components" not "transparent scoring components".
- Use noun phrases for checklist items, not imperative verbs. Write "redundant index detection" not "detect redundant indexes".
- Headings in Markdown files must be in the title case: "Build from Source" not "Build from source". Minor words (a, an, the, and, but, or, for, in,
  on, at, to, by, of) stay lowercase unless they are the first word.

## Repository Layout

- `crates/rice-core/`: Shared types (Module, Opcode, Instruction, TypeDescriptor, errors). No runtime logic.
- `crates/rice-loader/`: Binary format parser for `.dis` module files. One public function: `load(&[u8]) -> Result<Module, LoadError>`.
- `crates/rice-execute/`: Execution engine with 176 opcode handlers, heap, GC, built-in modules ($Sys, $Math, $Draw, $Tk, $Crypt), and file-based
  module loading.
- `crates/rice-cli/`: CLI with `run` and `dis` subcommands.
- `external/inferno-os/`: Git submodule of the Inferno OS repository (866 pre-compiled `.dis` files and Limbo source for testing).
- `Makefile`: GNU Make wrapper around `cargo` commands (`make test`, `make build`, `make lint`, etc.).
- `rust-toolchain.toml`: Pinned Rust toolchain (1.92.0) with `rustfmt`, `clippy`, and `rust-analyzer`.

## Architecture

### Crate Dependency Graph

```
ricevm-cli
├── ricevm-core
├── ricevm-loader → ricevm-core
└── ricevm-execute → ricevm-core, ricevm-loader
```

`ricevm-execute` depends on `ricevm-loader` for runtime module loading (the `load` opcode reads `.dis` files from disk).

### Key Internal Modules in `ricevm-execute`

| Module         | Purpose                                                                            |
|----------------|------------------------------------------------------------------------------------|
| `vm.rs`        | `VmState` struct, execution loop, operand read/write helpers, parent MP stack      |
| `frame.rs`     | `FrameStack` with two-phase push (`alloc_pending` and `activate_pending`)          |
| `heap.rs`      | `Heap` with reference counting, copy-on-write strings, `ArraySlice` shared views   |
| `gc.rs`        | Mark-and-sweep garbage collector (scans frames, MP, and loaded module MPs)         |
| `address.rs`   | Operand resolution: `Operand` to `AddrTarget` (frame, MP, immediate, heap array)   |
| `memory.rs`    | Typed read/write on byte buffers with bounds checking                              |
| `data.rs`      | Module data (MP) initialization from `DataItem` entries with type-aware elem sizes |
| `filetab.rs`   | Portable file descriptor table with in-memory pipe support                         |
| `ops/`         | 176 instruction handlers organized by category                                     |
| `sys.rs`       | Built-in `$Sys` module (43 functions with signature hashes)                        |
| `math.rs`      | Built-in `$Math` module (66 functions)                                             |
| `draw.rs`      | Built-in `$Draw` module (SDL2 backend, optional `gui` feature)                     |
| `tk.rs`        | Built-in `$Tk` module (widget toolkit stubs)                                       |
| `builtin.rs`   | `ModuleRegistry` for built-in module registration with name and signature lookup   |
| `scheduler.rs` | Cooperative thread scheduler (infrastructure)                                      |
| `channel.rs`   | Channel data structure for inter-thread communication                              |

### Key Design Decisions

- Package names use hyphens (`ricevm-core`); Rust identifiers use underscores (`ricevm_core`).
- The heap uses `HashMap<u32, HeapObject>` with monotonic IDs starting at `HEAP_ID_BASE` (0x0100_0000);
  pointers stored as `Word` (i32) in frames.
- Array element references use a `heap_refs` table with `HEAP_REF_FLAG` sentinel, resolved during double-indirect addressing.
- `ArraySlice` heap type provides shared-storage views into parent arrays (required for Bufio buffer semantics).
- `MP_REF_FLAG` (0x4000_0000) tags MP addresses stored by `Lea` so double-indirect resolution returns `AddrTarget::Mp`.
- `parent_mps` stack in `VmState` enables cross-module MP reads/writes when loaded modules access their caller's MP.
- Branch instructions: `if src OP mid, goto dst` (not `if src OP dst, goto mid`).
- Case instructions (`casew`, `casec`, `casel`) use binary search matching the reference Dis VM implementation.
- Multi-module execution: loaded modules' MPs are swapped (not cloned) to persist state across calls.
- Built-in function dispatch prefers name matching over signature hash matching (avoids collisions like read/write).
- `movmp` looks up the type descriptor to determine copy size (mid is a type index, not a byte count).
- Data initialization (`data.rs`) uses type descriptors for array element sizes and writes to the active buffer context
  (parent array or MP) for correct nested array initialization.
- Builtin return value copy in `mcall` transfers 4 bytes to the caller's return pointer; big-returning functions
  (like `seek`) write their 8-byte result directly through the return pointer.
- Per-thread error string (`last_error`) implements the `werrstr`/`%r` mechanism.
- Portable I/O via `FileTable` (no `libc` dependency) with in-memory pipe support.
- SDL2 for GUI is behind an optional `gui` feature flag.

## Required Validation

Run `make test` for any change. Key targets:

| Target   | Command         | What It Runs                                      |
|----------|-----------------|---------------------------------------------------|
| Format   | `make format`   | `cargo fmt`                                       |
| Lint     | `make lint`     | `cargo clippy` with `-D warnings` and strict deny |
| Test     | `make test`     | All workspace tests with `--nocapture`            |
| Build    | `make build`    | Release build                                     |
| Coverage | `make coverage` | `cargo tarpaulin` with XML and HTML output        |
| Audit    | `make audit`    | `cargo audit` on dependencies                     |

## Testing Expectations

- Unit tests live in each crate's source files using `#[cfg(test)]` modules.
- Integration tests for the loader live in `crates/rice-loader/tests/`.
- Pipeline tests (loader to executor) live in `crates/rice-cli/tests/`, including tests with real Inferno `.dis` files.
- Fuzz testing for the loader is set up in `crates/rice-loader/fuzz/`.
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
