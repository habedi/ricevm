# Current Limitations

RiceVM currently supports a large subset of the Dis VM and Limbo language, but there are some limitations to be aware of.
These include design choices made for simplicity and performance, as well as features that are not implementable on the host OS or are still
incomplete.

## VM Limitations

### Design Choices

- Cooperative threading: the run loop rotates threads by quantum (2048 instructions). A preemptive scheduler with an
  OS thread pool exists in `scheduler.rs` and keeps its state in `Arc<Mutex<SharedState>>`, but nothing instantiates
  it, so the live loop is `VmState::run`. Connecting it needs the same treatment for the rest of `VmState`.
- Non-blocking stdin: stdin reads use a background thread to avoid freezing all VM threads. A read that has no data
  yet reports that it would block, and the run loop then hands the OS thread to another VM thread and retries. Only
  when no other thread can run does the read wait on the host.

### Not Implementable on the Host OS

- `$Sys` functions that need Plan 9 namespace semantics: `bind`, `mount`, `unmount`, `export`,
  `fauth`, and `file2chan` have no host OS equivalent.
- 104 of the pre-compiled Inferno programs fault and 8 time out, mostly where the program needs something the host
  cannot provide: a display, a network service, or Plan 9 namespace semantics. The faults gather under `wm/` (33) and
  `charon/` (9), with the rest spread across `acme/`, `svc/httpd/`, `collab/servers/`, and `spree/clients/`. A further
  194 programs exit through `fail:...`, most of them printing a usage message because they were run without
  arguments, so those count as working.

### Incomplete Modules

- `$Draw` has 35+ stub functions. Basic rendering (rectangles, lines, text, and images) works via
  SDL2, but many advanced drawing operations are not implemented.
- `$Keyring` provides real MD4, MD5, SHA1, SHA224, SHA256, SHA384, and SHA512 digests, but IPint (big integer), TLS, and authentication
  functions are stubs.
- `$IPints` is not implemented at all, so a program that needs big-integer arithmetic, such as `randpass`, cannot run.

## Compiler Limitations

The built-in Limbo compiler (`ricevm-limbo`) handles a large subset of the language but has gaps:

- No type checker: type inference is used during code generation, but there is no validation pass that reports type errors before execution. Programs
  that the reference compiler would reject as type-incorrect (for example, mixed-width arithmetic without an explicit cast) compile silently and can
  produce results.
- Array-of-channels `alt` guards: an `alt` arm can receive from a single channel, but not from an array of channels.
  The alt table the VM reads has no way to express that form.
- No exception handler block codegen: `raise` works, but `{ ... } exception { ... }` blocks do not generate handler table entries.
- ADT function member calls through a receiver the compiler cannot type, such as one reached through `hd list`, are a
  compile error. This is the largest remaining gap.
- Cyclic ADT references: standard ADTs with int, byte, big, real, string, list, ref, and array fields work end-to-end (including correct
  field offsets, kind-matched moves, and nested access), and so do tagged ADTs with `pick`. Cyclic ADT references are not yet supported.
- Field-access heuristics for unknown ADTs: when the compiler cannot resolve a value's ADT (for example, fields read off an opaque module return),
  field offsets and types fall back to a name-based heuristic with `Movw`. Resolving these fully requires the type checker.
- No import signature hashes: all import signatures are 0; the VM uses name-based function matching.

## Compatibility

At the moment, 669 of the 781 runnable pre-compiled Inferno `.dis` programs (86%) run without a VM fault. Of the 866
files in the submodule, 85 are library modules with no init function to execute. Of the remainder, 475 run to
completion and 194 exit through `fail:...`, which is how a Limbo program reports a usage message or a missing service.
The 104 faults and 8 timeouts are concentrated in the subsystems that need a display or a network service, `wm/` and
`charon/` most of all.

This counts programs that start and do not fault, which is a floor rather than a guarantee. Note that a program can run
to completion and still print the wrong thing.

The built-in compiler parses 159/159 (100%) of Inferno `cmd/` source files and compiles 98 of them, or 356 of the 945
`.b` files under `appl/` when measured with `-I external/inferno-os/module`. An earlier figure of 155/159 counted
programs that compiled only because unresolved names lowered to a zero and unsupported statements emitted nothing;
those are diagnostics now, so the count is lower and means more. See the roadmap for the breakdown of what remains.
