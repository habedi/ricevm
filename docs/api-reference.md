# API Reference

## Architecture

| Crate            | Purpose                     |
|------------------|-----------------------------|
| `ricevm-core`    | Shared types and utilities  |
| `ricevm-loader`  | `.dis` binary format parser |
| `ricevm-execute` | Dis runtime                 |
| `ricevm-limbo`   | Limbo compiler              |
| `ricevm-cli`     | CLI frontend                |

## Built-in Modules

### `$Sys`

This module includes functions for system I/O, process control, and string formatting.

| Category                                | Functions                                                                                                                     | Description                                                                            |
|-----------------------------------------|-------------------------------------------------------------------------------------------------------------------------------|----------------------------------------------------------------------------------------|
| Formatting and errors                   | `aprint`, `fprint`, `print`, `sprint`, `werrstr`                                                                              | Formatted output helpers plus per-thread error-string support.                         |
| File and stream I/O                     | `create`, `dup`, `fd2path`, `fildes`, `iounit`, `open`, `pipe`, `pread`, `pwrite`, `read`, `readn`, `seek`, `stream`, `write` | File-descriptor creation, conversion, random access, pipes, and streaming operations.  |
| File metadata and filesystem changes    | `chdir`, `dirread`, `fstat`, `fwstat`, `remove`, `stat`, `wstat`                                                              | Directory traversal, stat/wstat operations, and filesystem mutation helpers.           |
| Networking                              | `announce`, `dial`, `listen`                                                                                                  | Host-network connection setup and listener support.                                    |
| Namespace and host-dependent operations | `bind`, `export`, `fauth`, `file2chan`, `mount`, `unmount`                                                                    | Plan 9 namespace and authentication entry points. Some remain host-limited or stubbed. |
| Text and Unicode helpers                | `byte2char`, `char2byte`, `tokenize`, `utfbytes`                                                                              | UTF decoding, UTF encoding, tokenization, and byte-length helpers.                     |
| Time and process control                | `fversion`, `millisec`, `pctl`, `sleep`                                                                                       | Version negotiation, wall-clock time, process control, and sleeping.                   |

### `$Math`

This module includes functions for mathematical operations, including trigonometry, linear algebra, and bit conversions.

| Category                         | Functions                                                                                                                                          | Description                                                                              |
|----------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------|
| Trigonometric and hyperbolic     | `acos`, `acosh`, `asin`, `asinh`, `atan`, `atan2`, `atanh`, `cos`, `cosh`, `sin`, `sinh`, `tan`, `tanh`, `hypot`                                   | Angle, inverse-angle, and hyperbolic operations on real values.                          |
| Exponential and logarithmic      | `exp`, `expm1`, `ilogb`, `lgamma`, `log`, `log10`, `log1p`, `pow`, `pow10`, `scalbn`                                                               | Exponentials, logarithms, powers, scaling by powers of two, and gamma/log-gamma helpers. |
| Rounding and remainders          | `cbrt`, `ceil`, `fabs`, `fdim`, `floor`, `fmax`, `fmin`, `fmod`, `modf`, `nextafter`, `remainder`, `rint`, `sqrt`                                  | Magnitude, rounding, remainder, root, and neighboring floating-point operations.         |
| Bit conversion and serialization | `bits32real`, `bits64real`, `export_int`, `export_real`, `export_real32`, `import_int`, `import_real`, `import_real32`, `realbits32`, `realbits64` | Convert between integer bit patterns, serialized byte formats, and real values.          |
| Classification and status        | `finite`, `getFPcontrol`, `getFPstatus`, `isnan`                                                                                                   | Floating-point classification plus emulated FP control and status accessors.             |
| Bessel family                    | `j0`, `j1`, `jn`, `y0`, `y1`, `yn`                                                                                                                 | Bessel functions of the first and second kinds.                                          |
| Error functions                  | `erf`, `erfc`                                                                                                                                      | Gaussian error function helpers.                                                         |
| Vector and matrix helpers        | `dot`, `gemm`, `iamax`, `norm1`, `norm2`, `sort`                                                                                                   | Small linear-algebra and numeric-array operations.                                       |
| Sign handling                    | `copysign`                                                                                                                                         | Combine the magnitude of one real with the sign bit of another.                          |

### `$Draw`

This module includes functions for graphics via SDL2 (optional `gui` feature).
This module is to support display allocation, image drawing, line and ellipse rendering, font metrics, and event handling.

| Area                          | Functions                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | Description                                                                                           |
|-------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------|
| Display                       | `Display.allocate`, `Display.cmap2rgb`, `Display.cmap2rgba`, `Display.color`, `Display.colormix`, `Display.getwindow`, `Display.namedimage`, `Display.newimage`, `Display.open`, `Display.publicscreen`, `Display.readimage`, `Display.rgb`, `Display.rgb2cmap`, `Display.startrefresh`, `Display.writeimage`                                                                                                                                                                                                                                                                                                                                                                                            | Display creation, color conversion, window binding, image allocation, and display I/O helpers.        |
| Font                          | `Font.bbox`, `Font.build`, `Font.open`, `Font.width`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | Font loading, metrics, and bounding-box helpers.                                                      |
| Image drawing                 | `Image.arc`, `Image.arcop`, `Image.arrow`, `Image.bezier`, `Image.bezierop`, `Image.bezspline`, `Image.bezsplineop`, `Image.border`, `Image.bottom`, `Image.draw`, `Image.drawop`, `Image.ellipse`, `Image.ellipseop`, `Image.fillarc`, `Image.fillarcop`, `Image.fillbezier`, `Image.fillbezierop`, `Image.fillbezspline`, `Image.fillbezsplineop`, `Image.fillellipse`, `Image.fillellipseop`, `Image.fillpoly`, `Image.fillpolyop`, `Image.flush`, `Image.gendraw`, `Image.gendrawop`, `Image.line`, `Image.lineop`, `Image.name`, `Image.origin`, `Image.poly`, `Image.polyop`, `Image.readpixels`, `Image.text`, `Image.textbg`, `Image.textbgop`, `Image.textop`, `Image.top`, `Image.writepixels` | Primitive drawing, compositing, text rendering, flush/present, pixel transfer, and z-order helpers.   |
| Screen                        | `Screen.allocate`, `Screen.bottom`, `Screen.newwindow`, `Screen.top`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | Screen allocation, window creation, and stacking-order helpers.                                       |
| Current implementation status | Implemented handlers include `Display.allocate`, `Display.color`, `Display.getwindow`, `Display.newimage`, `Font.open`, `Font.width`, `Image.border`, `Image.draw`, `Image.drawop`, `Image.ellipse`, `Image.ellipseop`, `Image.fillellipse`, `Image.fillellipseop`, `Image.flush`, `Image.line`, `Image.lineop`, `Image.text`, `Image.textop`, `Screen.allocate`, and `Screen.newwindow`. Remaining entries currently return stub values.                                                                                                                                                                                                                                                                | SDL2-backed coverage exists for the common drawing path, but advanced Draw APIs are still incomplete. |

### `$Tk`

This module includes functions for the Tk widget toolkit. Supports toplevel windows, widget creation, command dispatch, named channels, and mouse event handling.

| Function                                                          | Description                                                      |
|-------------------------------------------------------------------|------------------------------------------------------------------|
| `cmd(top: ref Toplevel, command: string): string`                 | Execute a Tk command string and return a result string.          |
| `color(name: string): int`                                        | Resolve a Tk color name or encoded color value.                  |
| `getimage(top: ref Toplevel): ref Draw->Image`                    | Return the image associated with a toplevel window.              |
| `keyboard(top: ref Toplevel, ctl: string): int`                   | Configure or query keyboard handling for a toplevel.             |
| `namechan(name: string, c: chan of string): int`                  | Register a named channel used by Tk callbacks and `send`.        |
| `pointer(top: ref Toplevel, ctl: string): int`                    | Configure or query pointer handling for a toplevel.              |
| `putimage(top: ref Toplevel, img: ref Draw->Image): int`          | Replace the image associated with a toplevel window.             |
| `quote(value: string): string`                                    | Quote a string for safe embedding in Tk command text.            |
| `rect(top: ref Toplevel): Draw->Rect`                             | Return the current rectangle for the toplevel area.              |
| `toplevel(display: ref Draw->Display, arg: string): ref Toplevel` | Create a top-level Tk window record and its backing image state. |

### `$Keyring`

This module includes functions for cryptographic operations.

| Function                                                                                              | Description                                   |
|-------------------------------------------------------------------------------------------------------|-----------------------------------------------|
| `md4(data: array of byte, n: int, digest: array of byte, state: ref DigestState): ref DigestState`    | MD4 hash (real implementation)                |
| `md5(data: array of byte, n: int, digest: array of byte, state: ref DigestState): ref DigestState`    | MD5 hash (real implementation)                |
| `sha1(data: array of byte, n: int, digest: array of byte, state: ref DigestState): ref DigestState`   | SHA1 hash (real implementation)               |
| `sha224(data: array of byte, n: int, digest: array of byte, state: ref DigestState): ref DigestState` | SHA224 hash (real implementation)             |
| `sha256(data: array of byte, n: int, digest: array of byte, state: ref DigestState): ref DigestState` | SHA256 hash (real implementation)             |
| `sha384(data: array of byte, n: int, digest: array of byte, state: ref DigestState): ref DigestState` | SHA384 hash (real implementation)             |
| `sha512(data: array of byte, n: int, digest: array of byte, state: ref DigestState): ref DigestState` | SHA512 hash (real implementation)             |
| `readauthinfo(path: string): ref Authinfo`                                                            | Read authentication info (stub)               |
| `writeauthinfo(path: string, info: ref Authinfo): int`                                                | Write authentication info (stub)              |
| `getstring(fd: ref Sys->FD): string`                                                                  | Read a string from a secure source (stub)     |
| `putstring(fd: ref Sys->FD, s: string): int`                                                          | Write a string to a secure sink (stub)        |
| `getbytearray(fd: ref Sys->FD): array of byte`                                                        | Read a byte array from a secure source (stub) |
| `putbytearray(fd: ref Sys->FD, data: array of byte): int`                                             | Write a byte array to a secure sink (stub)    |
| `auth(fd: ref Sys->FD, info: ref Authinfo): ref AuthResult`                                           | Perform authentication exchange (stub)        |

### `$Crypt`

This module includes digest functions for cryptographic hashing.

| Function                                                                                              | Description                                    |
|-------------------------------------------------------------------------------------------------------|------------------------------------------------|
| `md4(data: array of byte, n: int, digest: array of byte, state: ref DigestState): ref DigestState`    | MD4 digest with incremental-state support.     |
| `md5(data: array of byte, n: int, digest: array of byte, state: ref DigestState): ref DigestState`    | MD5 digest with incremental-state support.     |
| `sha1(data: array of byte, n: int, digest: array of byte, state: ref DigestState): ref DigestState`   | SHA-1 digest with incremental-state support.   |
| `sha224(data: array of byte, n: int, digest: array of byte, state: ref DigestState): ref DigestState` | SHA-224 digest with incremental-state support. |
| `sha256(data: array of byte, n: int, digest: array of byte, state: ref DigestState): ref DigestState` | SHA-256 digest with incremental-state support. |
| `sha384(data: array of byte, n: int, digest: array of byte, state: ref DigestState): ref DigestState` | SHA-384 digest with incremental-state support. |
| `sha512(data: array of byte, n: int, digest: array of byte, state: ref DigestState): ref DigestState` | SHA-512 digest with incremental-state support. |

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
