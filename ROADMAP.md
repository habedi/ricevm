## Project Roadmap

This document outlines the features implemented in RiceVM and the future goals for the project.

> [!IMPORTANT]
> This roadmap is a work in progress and is subject to change.

### Module Loading

- [x] `.dis` binary format parsing (header, code, type descriptors, data, and module name)
- [x] Operand address decoding (immediate, indirect, and double-indirect modes)
- [x] Type descriptor parsing with pointer map reconstruction
- [x] Link and import section loading (including deprecated import flag acceptance)
- [x] File-based module loading (load `.dis` files from disk at runtime)
- [x] Built-in module registration and dispatch
- [x] Name-based function mapping for built-in modules (with signature fallback)
- [x] Name-based function mapping for loaded modules (with signature fallback)
- [x] Multi-module execution with MP swap (persistent state across calls)
- [x] Cross-module MP addressing via `ModuleMp` virtual address ranges
- [x] Current module tracking for correct import table and type resolution
- [x] Inferno path stripping (`/dis/lib/` prefix) for module resolution
- [x] Library module support (`entry_pc = -1` returns success immediately)
- [x] Type-aware array element sizes in data initialization
- [x] Nested array initialization via `SetArray` with active buffer context

### Instruction Set

- [x] Arithmetic: word, byte, big, float variants with wrapping semantics
- [x] Comparison and branching: `if src OP mid, goto dst` (correct operand order)
- [x] Load, store, and move: `movw`, `movb`, `movf`, `movl`, `movp` (with ref counting), `movm`, and `movmp`
- [x] `movmp` type descriptor lookup (mid is a type index, not a byte count)
- [x] Type conversions: all `cvt*` variants including string/real/big/word
- [x] `cvtfw`/`cvtfl` with correct rounding (±0.5, not truncation)
- [x] `cvtrf`/`cvtfr` as f32↔f64 conversion (SREAL = IEEE 754 float)
- [x] `cvtwc`/`cvtcw` as decimal string formatting/parsing (this matches C `strtol`)
- [x] `cvtfc` using `%g` format for real-to-string conversion
- [x] Bitwise and shift: word, byte, and big variants including logical shift right
- [x] Control flow: `call`, `ret`, `frame`, `jmp`, `exit`, `goto`, `casew`, `casec`, `casel`, and `raise`
- [x] Binary search dispatch for `casew`, `casec`, and `casel` (matching reference Dis VM)
- [x] `casel` with correct 24-byte entry layout (LONG alignment padding)
- [x] String operations: `lenc`, `indc`, `insc`, `addc` (two-operand fix), `slicec` (with bounds checks), `cvtca`, and `cvtac`
- [x] List operations: `cons*`, `head*`, and `tail` for all element types
- [x] `lenl` list-length semantics
- [x] Array operations: `indx`, `indw`, `indf`, `indb`, `indl` (array indexing via heap refs), `newa`, `slicea`, and `slicela`
- [x] `slicea` creates shared-storage `ArraySlice` views (not copies)
- [x] `slicela` array append with pointer ref counting (distinct from `slicea`)
- [x] Module operations: `load`, `mcall`, and `mframe` with name-based dispatch
- [x] Builtin return value copy in `mcall` (4 bytes via return pointer at offset 16)
- [x] Tuple-returning functions write all fields through ret pointer directly
- [x] `Lea` handles Frame, MP (`ModuleMp` virtual ranges), and HeapArray (`HEAP_REF_FLAG`) sources
- [x] Memory allocation: `new`, `newz`, `newa`, `newaz`, `mnewz`, and channel allocation
- [x] Exponentiation: `expw`, `expl`, and `expf` with correct operand order and integer exponents
- [x] Concurrency: `spawn` (creates cooperative thread), `mspawn`, `send`/`recv` (with blocking), and `alt`/`nbalt`

### Type System

- [x] Dis primitive types (byte, word, big, and real)
- [x] Pointer type with heap object tracking and reference counting
- [x] String type with full Unicode support and copy-on-write
- [x] Array type with bounds checking and heap array references
- [x] `ArraySlice` type for shared-storage array views (Bufio buffer semantics); fully supported in `cvtac`, `slicela`, `pread`, `pwrite`, and channel
  operations
- [x] List type (singly linked with typed head values)
- [x] Channel type (allocation and simplified send and receive)
- [x] ADT (abstract data type) support
- [x] Tuple and reference types

### Memory Management

- [x] Stack frame allocation and deallocation with two-phase push
- [x] Heap allocation for dynamic types (records, arrays, strings, lists, channels, and module refs)
- [x] Reference counting for deterministic destruction (module refs protected from premature freeing)
- [ ] `op_ret` frees frame pointers via type descriptor pointer maps (matching reference `freeptrs`) -- disabled: the frame header's type index is
  unreliable for frames built by builtin `mcall`/`mframe`, so releasing by it would dec_ref values that are not pointers. The collector reclaims the
  frame's objects instead (ops/control.rs)
- [x] Mark-and-sweep garbage collector (scans frames, MP, caller MP stacks, all loaded module MPs, and all suspended threads)
- [x] Optional toggle to disable mark-and-sweep collection (`--no-gc` flag; debug only, since reference counting alone leaks -- see Known Limitations)
- [x] Bounds-safe memory access (out-of-bounds reads return 0; writes are no-ops)

### Scheduler

- [x] Cooperative thread scheduler with round-robin quantum rotation (2048 instructions)
- [x] `spawn` creates cooperative threads (not inline execution)
- [x] Channel blocking: `recv` on empty channel suspends thread; `send` on full channel suspends thread; both directions unblock on state change
- [x] Thread queue with ready/blocked state tracking
- [x] Deadlock detection (all threads blocked → halt)
- [x] Non-blocking stdin via background read thread (prevents `sys->read` from freezing all VM threads)
- [ ] Full preemptive thread scheduling with OS thread pool (infrastructure exists but not connected)

### Built-in Modules

- [x] `$Sys` module (43 functions, 38+ real implementations, ~5 stubs)
    - I/O: `print`, `fprint`, `sprint`, `aprint`, `open`, `create`, `read`, `write`, `seek`, `fildes`, `fd2path`, `dup`, `pipe`, and `stream`
    - Utilities: `millisec`, `sleep`, `pctl`, `tokenize`, `byte2char`, `char2byte`, `utfbytes`, `chdir`, `remove`, and `iounit`
    - File info: `fstat`, `stat`, `fwstat`, `wstat`, and `dirread`
    - Network: `dial`, `announce`, and `listen`
    - Error strings: `werrstr` with `%r` format specifier support
    - `seek` with correct big-value alignment and direct return pointer write
    - `pread`/`pwrite` with correct big-value alignment
    - Tuple-returning functions (tokenize, byte2char, stat, fstat, dirread, dial, announce, listen, and fversion) write all fields through ret pointer
    - All functions have correct signature hashes from the C++ Sysmodtab
- [x] `$Math` module (66 functions, 50+ real implementations)
    - Trig, log, exp, pow, sqrt, floor, ceil, hypot, bit conversions, and more
    - Linear algebra: `dot`, `norm1`, `norm2`, `iamax`, and `gemm`
    - `import_real`/`export_real` with correct byte-order conversion
    - `getFPcontrol` and `getFPstatus` with dedicated implementations
    - Frame layout with `ARG1_OFF=32`, `ARG2_OFF=40`
- [x] `$Draw` module (62 functions registered, SDL2 backend behind `gui` feature, and many operations still stubbed)
    - `Display.allocate`: opens an SDL2 window and creates proper Display/Image/Screen ADT records
    - `Image.draw`, `Image.line`, `Image.ellipse`, and `Image.flush`: basic SDL2 rendering
    - `Display.getwindow`, `Screen.allocate`, and `Screen.newwindow`: proper ADT record creation
    - `Font.open` and `Font.width`: default font metrics
- [x] `$Tk` module (10 functions with full command dispatch)
    - `toplevel`: creates Toplevel ADT with display, wreq channel, image, and screen rect
    - `cmd`: full command dispatch (widget creation, configure, cget, winfo, bind, send, pack -in/-side, compound commands)
    - `namechan`: registers named channels for Tk event delivery
    - `pointer`, `keyboard`, `quote`, `color`, `rect`, `getimage`, and `putimage`
- [x] `$Keyring` module (14 functions: `md4`, `md5`, `sha1`, `sha224`, `sha256`, `sha384`, and `sha512` with real digests; `readauthinfo`,
  `writeauthinfo`, `getstring`, `putstring`, `getbytearray`, `putbytearray`, and `auth` stubs)
- [x] `$Crypt` module (`md4`, `md5`, `sha1`, `sha224`, `sha256`, `sha384`, and `sha512` digests)
- [x] Exception handler table lookup for `raise` opcode and nil dereference faults
- [x] Name-based function dispatch with signature-hash fallback for built-in and loaded modules

### Limbo Compiler Support

- [x] Limbo compiler (`limbo.dis`) runs end-to-end on RiceVM
- [x] Compiles `.b` source files to `.dis` bytecode
- [x] Include file processing (`include "sys.m"` and `include "draw.m"`)
- [x] Compiled output verified: `echo`, `cat`, `basename`, `date`, `mkdir`, `rm`, `sleep`, `tee`, `wc`, `du`, `strings`, `sort`, `grep`, `uniq`,
  `tail`, `tr`, `string`, `calc`, `mash`, `sh`, `math`, `cmp`, `freq`, `tcs`, and 20 more
- [x] Programs with float literals compile correctly
- [x] Compiled programs execute correctly on RiceVM (echo, cat, basename, rm, mkdir, tr, sleep, date, and wc verified)
- [x] `sprint`/`fprint` format specifiers: `%d`, `%s`, `%f`, `%g`, `%x`, `%o`, `%c`, `%r`, `%b`, `%u`, `%.*`, width, precision, and flags
- [x] Stdin piping for Bufio-based programs (grep, sort, uniq, wc via Bufio->gets)

### GUI Support

- [x] SDL2 backend behind optional `gui` feature flag
- [x] `Display.allocate` creates SDL2 window with proper Display/Image/Screen ADTs
- [x] Proper Inferno ADT record layouts for Display, Image, Screen, Font, and Toplevel
- [x] Event loop integration via `Image.flush` (polls SDL events, handles window close)
- [x] `wm/about.dis` loads all modules (tkclient, wmlib, titlebar), processes Tk commands, and enters the event loop
- [x] Tk widget rendering (label, button, frame, pack, canvas, configure, cget, and winfo)
- [x] Tk command dispatch with widget subcommands, bind, send, and pack -in/-side
- [x] Named channel support via `Tk->namechan` with send-to-channel from Tk commands
- [x] Hex color parsing (`#RRGGBB`) for `-bg` and `-fg` widget options
- [x] Embedded 8x13 bitmap font for all printable ASCII (no external font library needed)
- [x] Mouse click dispatch to Tk button widgets (sends command to named channels)
- [x] Mouse and keyboard event delivery to Tk
- [x] Virtual device files: `/dev/sysctl`, `/dev/sysname`, `/dev/user`, `/dev/time`, `/dev/cons`, `/dev/null`, `/dev/random`, `/dev/drivers`,
  `/prog/N/status`, `/prog/N/wait`, `/prog/N/ns`, `/prog/N/ctl`, and `/env/*`

### Audio Support

- [x] `/dev/audio` and `/dev/audioctl` support behind optional `audio` feature flag (cpal backend)

### Portability

- [x] Fully portable (no `libc` dependency; all I/O via `std::fs` and `std::io`)
- [x] Compiles on Linux, macOS, and Windows
- [x] SDL2 GUI is optional (`--features gui`)

### CLI

- [x] Argument parsing via `clap`
- [x] Tracing and logging setup via `tracing-subscriber`
- [x] `run` subcommand to execute a `.dis` module file
- [x] `compile` subcommand to compile Limbo `.b` source to `.dis` bytecode
- [x] `dis` subcommand for human-readable disassembly
- [x] `--probe` flag to add module probing paths
- [x] `--root` flag for Inferno root path mapping
- [x] `--trace` flag for instruction-level debugging
- [x] `--no-gc` flag to disable mark-and-sweep garbage collection
- [x] `--threads` flag to configure scheduler thread pool size
- [x] `-- arg1 arg2` guest argument passing
- [x] Colored output, elapsed time reporting, and exit codes
- [x] Debugger integration (breakpoints, single-stepping, stack inspection, colored output, and `info` command)

### Compatibility

- [x] 546 of 844 pre-compiled Inferno `.dis` programs pass (65%); ~83% effective pass rate excluding programs that need arguments or are library
  modules
- [x] 58 timeouts (programs waiting for interactive input; expected with no stdin)
- [x] Systematic audit against reference xec.c implementation
- [ ] Target: 600+ programs passing (70%+); remaining failures are mostly environment-dependent (Plan 9 namespaces, crypto, device files)

### Development and Testing

- [x] Cargo workspace with modular crate structure
- [x] CI pipeline with automated tests
- [x] Dual license (MIT and Apache 2.0)
- [x] 200+ tests total:
    - Unit tests for instruction decoding and execution
    - Property-based tests for arithmetic (commutativity, associativity, and identity)
    - Property-based tests for string operations (slicec bounds, addc associativity)
    - Regression tests for all major bug fixes (movmp, casew, slicea, channel blocking, and spawn)
    - Integration tests with real Inferno OS `.dis` modules (echo, cat, and multi-module loading)
    - Limbo compiler end-to-end test
- [x] End-to-end pipeline tests with hand-crafted `.dis` binaries
- [x] Fuzz testing setup for the module loader (`cargo-fuzz` with `libfuzzer`)
- [x] 800+ pre-compiled `.dis` files available via `external/inferno-os` submodule
- [x] `make lint` passes (clippy with `-D warnings -D clippy::unwrap_used -D clippy::expect_used`)
- [x] `make test` passes (233 tests, 0 failures)

### Built-in Limbo Compiler (`ricevm-limbo` crate)

- [x] Lexer: all 48 Limbo keywords, operators, string/char/number/real literals with trailing and leading dots (~680 lines)
- [x] Parser: recursive descent with Pratt expression parsing; 159/159 (100%) Inferno cmd/ programs parse (~2200 lines)
- [x] Polymorphic type parameters (`Type[T]`, `func[T]`, `adt[T]`), `raises` clauses, varargs `fn(args, *)`
- [x] Dereference operator (`*expr`), `Type.SubType` qualified types, exception handler blocks
- [x] Prefix-operator binding power above every infix bp so `-a - b`, `big lv % big rv`, and similar parse with Limbo's intended precedence
- [x] AST: complete type definitions for all Limbo language constructs (395 lines)
- [x] Code generator: if/else, while, for, do-while, case, all arithmetic/comparison/logic operators (~1300 lines)
- [x] Three-operand emission for non-commutative arithmetic (Subw, Divw, Modw, Shlw, Shrw, Expw, and big/real variants), preserving Dis `dst = mid OP
  src` semantics
- [x] Numeric kind tracking (`Word`, `Big`, `Real`) across allocation, expression evaluation, and slot sizing, so big and real values keep all 64
  bits through arithmetic, comparisons, casts, and moves
- [x] Mixed-kind promotion via `gen_expr_to_kind`: a narrower operand (for example, an int literal in `big_var + 1`) is widened with `Cvtwl`,
  `Cvtwf`, or `Cvtlf` before the wide arithmetic opcode
- [x] String operations: concatenation (Addc with chaining), comparison (Beqc/Bnec), indexing (Indc), assignment (Insc), length (Lenc)
- [x] List operations: hd (Headp), tl (Tail), cons (Consp), nil comparison, list literals
- [x] Array operations: creation (Newa), byte conversion (Cvtca), length (Lena), slicing (Slicea), slice assignment (Slicela), kind-aware element
  read and write via Indw/Indl/Indf/Indb plus Movw/Movl/Movf/Movb through a heap-ref slot, and `arr[i] op= val` compound assignment
- [x] Recursive nested array typing: `array of array of T` peels one `Type::Array` layer per `Index`, so the inner indexing picks the correct
  element-width opcode pair
- [x] Channel operations: kind-aware allocation via Newcw/Newcb/Newcl/Newcf/Newcp, plus Send and Recv sized by the channel's element width
- [x] Thread creation: spawn with correct per-function frame type descriptors and kind-aware argument packing
- [x] Forward-referenced local calls and spawns: a pre-pass populates `func_table` for every declared function, and a fixup pass patches
  Call/Spawn destination operands after every body has been emitted
- [x] Multiple functions per module with local calls (Frame + Call) and return values written directly through the return pointer (the caller's
  destination slot is installed as the return target via Lea, eliminating an intermediate copy and supporting tuple-shaped returns wider than 8
  bytes)
- [x] Tuple unpack: `(a, b, ...) := func()` allocates per-field locals at the function's actual return-tuple offsets and uses kind-matched Mov per
  field
- [x] Module loading: $Sys and generic modules via Load instruction
- [x] Sys vararg call packing: cumulative argument offsets sized by each argument's `NumKind`, so `sys->print("%bd", x)` and similar correctly read
  8 bytes for big and real values
- [x] ADT layout-driven field access: a pre-pass over `Decl::Adt` and `ModuleMember::Adt` records each ADT's field offsets and types with proper
  alignment (4 for word, byte, and pointer fields; 8 for big and real fields). `Dot` reads, `Dot` lvalue writes, and `ref Adt(...)` initialization
  use the layout for offsets and pick `Movw`, `Movl`, `Movf`, or `Movp` per field
- [x] Type conversions: int, big, real, string, array casts (Cvtwl, Cvtwf, Cvtwc, Cvtcw, Cvtac, Cvtca, Cvtlw, Cvtfw, Cvtlf, Cvtfl)
- [x] Real and big literal support (Movf, Movl from 8-byte-aligned data section)
- [x] Include file processing: reads .m module interface files, extracts types, constants, and function signatures
- [x] Symbol table with type resolution and constant evaluation
- [x] .dis binary writer: complete format with operand encoding, handler tables, and null terminators
- [x] CLI integration: `ricevm-cli compile source.b [-o output.dis] [-I include_path]`
- [x] `make test-limbo`: 11 correctness tests (built-in vs reference compiler output comparison)
- [x] `import` declarations: `a, b: import m;` binds module members unqualified, for both a module variable and a module interface name. Constants
  fold (including in constant expressions), functions lower to the same cross-module call as the qualified spelling
- [x] `implement X` brings X's own interface into unqualified scope, with real ADT field layouts
- [x] Cross-module calls through any module handle (previously only `sys->f()` worked; every other handle emitted nothing)
- [x] Per-type descriptors for `new`/`newa`, with MSB-first pointer maps, so the collector traces compiled programs at the right offsets
- [x] Tuple values: literals, `.tN` read and write, whole-tuple copies, tuple arguments, returns and messages, and `((a, b) := <-c).t0`
- [x] Tuple channels via `Newcm`/`Newcmp` with a real block descriptor
- [x] Module-level data initialisers in a module with no `init`, written to the data section as the reference does
- [x] `alt` statement codegen: send, receive and wildcard arms, `or`-joined guards sharing one body, tuple receives, and blocking via `Alt`/`Nbalt`
- [x] `pick` statement codegen: tagged-ADT layout (tag at 0, then common fields, variants overlaid), `ref Adt.Variant(...)` construction, and `tagof`
- [x] 356/945 programs under `external/inferno-os/appl` compile, measured with `-I external/inferno-os/module`
- [ ] Full type checker (validation, not just inference)
- [ ] ADT function member calls whose receiver the compiler cannot type -- e.g. through `hd list` (199 corpus files, the largest remaining gap)
- [ ] Exception handler block codegen
- [ ] Cyclic ADT references
- [ ] Array-of-channels `alt` guards (the VM's alt table cannot express them)

### Documentation

- [x] Quickstart guide (in README)
- [x] Mapping from Dis VM specification to RiceVM internals
- [ ] Examples with precompiled Limbo programs

### Known Limitations

#### Design Choices

- Cooperative threading with non-blocking stdin: the run loop rotates threads by quantum; stdin reads use a background thread to avoid blocking the
  VM; a preemptive scheduler with OS threads exists but is not connected because it would require `Arc<Mutex<>>` refactoring of VmState
- `op_ret` does not restore module context from the frame; the `mcall` wrapper handles module context restoration instead (correct behavior, different
  structure from reference)
- Heap tracing is precise where a type descriptor is known, conservative everywhere else, and releasing is still conservative throughout. `new`, `newa`
  and `newaz` resolve the allocating module's type descriptor while the module is still known and hang the resulting pointer map on the object
  (`heap::TraceMap`, interned per `(module, type)`), so mark-and-sweep traces exactly the words the module calls pointers: bytes read into an
  `array of byte`, or the halves of a `real`, no longer keep an object alive by coincidence. Objects allocated without a descriptor -- strings, list
  nodes, channels, module data arrays, the records the runtime builds for its own use -- keep the word-by-word scan, which retains anything that looks
  like a live id. `dec_ref` still cascades only through slots whose reference is provably taken on store (list tails, array-slice parents) and not
  through buffers, mapped or not: a pointer map states layout, not ownership, and the block-copy paths (`heap_write`, `array_write`, `cons_bytes`,
  `movm`, `headm`) move pointers in *and out* of buffers without counting anything, while `op_ret` never releases a frame's pointers. Cascading on the
  map alone would release references that were never acquired. So pointers owned by a buffer are still reclaimed by the collector rather than at drop,
  and with `--no-gc` they leak. Closing that gap means a counted write barrier on every block copy in both directions, plus a working `freeptrs` at
  `ret`.

#### Unimplementable on Host OS

- `$Sys` stubs that require Plan 9 namespace semantics: `bind`, `mount`, `unmount`, `export`, `fauth`, and `file2chan` (no host OS equivalent)
- ~240 pre-compiled programs fail: ~100 need command-line arguments (working correctly), ~50 need Plan 9 namespace/device features, ~30 need crypto
  modules beyond the current `$Keyring` stub, and ~60 have other environment dependencies

#### Measuring built-in compiler coverage

A compile-success count only means something if unresolved names are errors. Before the built-in compiler reported them, an unknown identifier
lowered to `Movw $0`, an `alt` statement emitted no code at all, and a call through any module handle other than `sys` emitted nothing -- so almost
every program "compiled" and a coverage number could not distinguish working output from silently wrong output. An earlier claim of 155/159 came
from that regime, as did a measured 856/945 over `external/inferno-os/appl`.

Those paths are now diagnostics, so the count reflects programs the compiler can actually lower: **356/945**. Measure with
`-I external/inferno-os/module`; without it no `.m` interface resolves and the number is meaningless. Remaining causes, largest first: 199 ADT
function member calls, 97 calls to undefined functions, 54 undefined identifiers, 35 interface members this compiler cannot yet represent, 40
missing interfaces, and a long tail. No input makes the compiler panic.

#### Incomplete Modules

- `$Draw` module has 35+ stub functions (only basic rendering works; full implementation requires extensive SDL2 work)
