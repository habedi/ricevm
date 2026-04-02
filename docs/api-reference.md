# API Reference

## Built-in Modules

### `$Sys`

43 functions for system I/O, process control, and string formatting.

| Function                                                    | Description                                 |
|-------------------------------------------------------------|---------------------------------------------|
| `print(fmt: string, ...)`                                   | Print formatted string to stdout            |
| `fprint(fd: ref FD, fmt: string, ...)`                      | Print formatted string to a file descriptor |
| `sprint(fmt: string, ...): string`                          | Format a string and return it               |
| `open(path: string, mode: int): ref FD`                     | Open a file                                 |
| `create(path: string, mode: int, perm: int): ref FD`        | Create a file                               |
| `read(fd: ref FD, buf: array of byte, n: int): int`         | Read from a file descriptor                 |
| `write(fd: ref FD, buf: array of byte, n: int): int`        | Write to a file descriptor                  |
| `seek(fd: ref FD, off: big, whence: int): big`              | Seek in a file                              |
| `fildes(fd: int): ref FD`                                   | Convert integer to file descriptor          |
| `tokenize(s: string, delim: string): (int, list of string)` | Split a string by delimiters                |
| `millisec(): int`                                           | Current time in milliseconds                |
| `sleep(ms: int)`                                            | Sleep for milliseconds                      |
| `byte2char(buf: array of byte, n: int): (int, int, int)`    | Decode a UTF-8 character                    |
| `dial(addr: string, local: string): (int, Sys->Connection)` | Connect to a network address                |

### `$Math`

66 functions for mathematical operations, including trigonometry, linear algebra, and bit conversions.

### `$Draw`

62 functions for graphics via SDL2 (optional `gui` feature). Supports display allocation, image
drawing, line and ellipse rendering, font metrics, and event handling.

### `$Tk`

10 functions for the Tk widget toolkit. Supports toplevel windows, widget creation, command
dispatch, named channels, and mouse event handling.

### `$Keyring`

11 functions for cryptographic operations.

| Function                                                                                            | Description                     |
|-----------------------------------------------------------------------------------------------------|---------------------------------|
| `md5(data: array of byte, n: int, digest: array of byte, state: ref DigestState): ref DigestState`  | MD5 hash (real implementation)  |
| `sha1(data: array of byte, n: int, digest: array of byte, state: ref DigestState): ref DigestState` | SHA1 hash (real implementation) |
| `readauthinfo(path: string): ref Authinfo`                                                          | Read authentication info (stub) |

### `$Crypt`

Stub module with `md5` function for compiler signature computation.

## Limbo Compiler API

The `ricevm-limbo` crate exposes a public Rust API:

```rust
// Compile source to a Module
let module = ricevm_limbo::compile(src, "hello.b")?;

// Compile source to .dis binary bytes
let bytes = ricevm_limbo::compile_to_bytes(src, "hello.b")?;

// Compile with include paths
let opts = ricevm_limbo::CompileOptions {
    include_paths: vec!["external/inferno-os/module".to_string()],
};
let module = ricevm_limbo::compile_with_options(src, "hello.b", &opts)?;
```

## Virtual Device Files

RiceVM emulates these Inferno device files on the host OS:

| Path             | Description                                 |
|------------------|---------------------------------------------|
| `/dev/cons`      | Console (stdin for read, stdout for write)  |
| `/dev/null`      | Discard writes, EOF on read                 |
| `/dev/random`    | Pseudo-random bytes                         |
| `/dev/time`      | Nanoseconds since epoch                     |
| `/dev/user`      | Current user name                           |
| `/dev/sysctl`    | System version string ("RiceVM")            |
| `/dev/sysname`   | System name ("ricevm")                      |
| `/dev/drivers`   | Available device driver list                |
| `/dev/audio`     | PCM audio output (optional `audio` feature) |
| `/dev/audioctl`  | Audio configuration                         |
| `/prog/N/status` | Process status                              |
| `/prog/N/wait`   | Process wait (returns EOF)                  |
| `/env/*`         | Environment variables                       |
