# Current Limitations

RiceVM currently supports a large subset of the Dis VM and Limbo language, but there are some limitations to be aware of.
These include design choices made for simplicity and performance, as well as features that are not implementable on the host OS or are still incomplete.

## VM Limitations

### Design Choices

- **Cooperative threading**: currently, the run loop rotates threads by quantum (2048 instructions). A preemptive
  scheduler with OS threads exists as infrastructure but is not connected, because it would require
  `Arc<Mutex<>>` refactoring of `VmState`.
- **Non-blocking stdin**: stdin reads use a background thread to avoid freezing all VM threads. Reads
  return EOF after 250ms if no data is available.

### Not Implementable on the Host OS

- `$Sys` functions that need Plan 9 namespace semantics: `bind`, `mount`, `unmount`, `export`,
  `fauth`, and `file2chan` have no host OS equivalent.
- About 240 pre-compiled Inferno programs fail: ~100 need command-line arguments (the programs work
  correctly but exit with usage errors), ~50 need Plan 9 namespace or device features, ~30 need
  cryptographic modules beyond the current `$Keyring` stub, and ~60 have other environment dependencies.

### Incomplete Modules

- `$Draw` has 35+ stub functions. Basic rendering (rectangles, lines, text, and images) works via
  SDL2, but many advanced drawing operations are not implemented.
- `$Keyring` provides real MD5, SHA1, SHA224, and SHA256 digests, but IPint (big integer), TLS, and authentication
  functions are stubs.

## Compiler Limitations

The built-in Limbo compiler (`ricevm-limbo`) handles a large subset of the language but has gaps:

- **No type checker**: type inference is used during code generation, but there is no validation pass
  that reports type errors before execution.
- **No alt statement codegen**: the `alt` statement parses but does not generate the alt table format
  required by the VM.
- **No exception handler block codegen**: `raise` works, but `{ ... } exception { ... }` blocks do
  not generate handler table entries.
- **Simplified ADT support**: inline ADTs work for simple fields; complex nested ADTs, pick types, and
  cyclic types are not fully supported in code generation.
- **No import signature hashes**: all import signatures are 0; the VM uses name-based function matching.

## Compatibility

546 of 844 (65%) pre-compiled Inferno `.dis` programs pass. Excluding programs that need command-line
arguments or are library modules not meant to run standalone, the effective pass rate is about 83%.

The built-in compiler parses 159/159 (100%) of Inferno `cmd/` source files and compiles 155/159 (97%)
with both the built-in and reference compilers producing identical outputs.
