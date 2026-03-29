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

- `crates/rice-core/src/lib.rs`: Core library with shared types, error definitions, and initialization logic.
- `crates/rice-loader/src/lib.rs`: Binary format parser for `.dis` module files (header, code, type descriptors, data, and imports).
- `crates/rice-execute/src/lib.rs`: Execution engine for Dis bytecode (instruction dispatch, stack frames, and thread management).
- `crates/rice-cli/src/main.rs`: CLI entry point using `clap`. Orchestrates the loader and executor.
- `tests/integration_tests.rs`: Integration tests (currently empty).
- `tests/property_tests.rs`: Property-based tests (currently empty).
- `Makefile`: GNU Make wrapper around `cargo` commands (`make test`, `make build`, `make lint`, etc.).
- `Cargo.toml`: Workspace root defining all four crate members and shared dependencies.
- `rust-toolchain.toml`: Pinned Rust toolchain (1.92.0) with `rustfmt`, `clippy`, and `rust-analyzer`.
- `tmp/disvm/`: Reference C++ Dis VM implementation used for cross-checking behavior.
- `tmp/awesome-inferno/`: Curated list of Inferno OS, Limbo, and Dis resources.

## Architecture

### Crate Dependency Graph

```
rice-cli
├── rice-core
├── rice-loader → rice-core
└── rice-execute → rice-core
```

`rice-cli` depends on all three library crates. `rice-loader` and `rice-execute` each depend on `rice-core` but not on each other.

### Crate Responsibilities

- **rice-core**: Types and definitions shared across crates (opcodes, type descriptors, module structures, error types). No VM runtime logic.
- **rice-loader**: Reads `.dis` binary files and produces in-memory module representations defined in `rice-core`. No execution logic.
- **rice-execute**: Takes loaded modules and runs them (instruction dispatch, memory management, scheduling). Depends on types from `rice-core`.
- **rice-cli**: Argument parsing, tracing setup, and orchestration. Thin glue between the loader and executor.

### Workspace Dependencies

- `anyhow` for application-level error handling (CLI and top-level operations).
- `thiserror` for typed library errors in core, loader, and execute crates.
- `tracing` and `tracing-subscriber` for structured logging.
- `clap` for CLI argument parsing.

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

Clippy is configured to deny `warnings`, `clippy::unwrap_used`, and `clippy::expect_used`. Use `anyhow` or `thiserror` for error handling instead of
`unwrap()`/`expect()`.

## First Contribution Flow

1. Read the relevant crate's `src/lib.rs` (or `src/main.rs` for `rice-cli`).
2. Implement the smallest possible change.
3. Add unit tests in the same file, or integration tests in `tests/integration_tests.rs` if behavior crosses crate boundaries.
4. Run `make lint && make test`.

## Testing Expectations

- Unit tests live in each crate's source files using `#[cfg(test)]` modules.
- Integration tests live in `tests/integration_tests.rs` for end-to-end validation with `.dis` module files.
- Property-based tests live in `tests/property_tests.rs` for fuzzing parsers and verifying invariants.
- No public API change is complete without a corresponding test.

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
