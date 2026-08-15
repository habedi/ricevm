//! Portable file descriptor table.
//!
//! Maps integer fd numbers to Rust I/O objects, replacing raw Unix fd operations.
//! Pre-populated with stdin(0), stdout(1), stderr(2).

use crate::audio::AudioState;
use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

/// Inferno open mode bit requesting truncation of an existing file.
const OTRUNC: i32 = 16;

/// A file table entry that supports read, write, and seek.
pub(crate) struct FileEntry {
    inner: Box<dyn FileOps>,
    pub path: Option<String>,
}

/// Trait combining Read + Write + Seek, with optional support for each.
#[allow(dead_code)]
pub(crate) trait FileOps: Send {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    fn write(&mut self, buf: &[u8]) -> io::Result<usize>;
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64>;
    fn flush(&mut self) -> io::Result<()>;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    /// Duplicate this handle so two fds refer to the same open file.
    /// Returns None for handles that cannot be duplicated.
    fn try_clone(&self) -> Option<Box<dyn FileOps>> {
        None
    }
}

/// Wrapper for std::fs::File (supports all operations).
struct RegularFile(std::fs::File);

impl FileOps for RegularFile {
    fn try_clone(&self) -> Option<Box<dyn FileOps>> {
        let file = self.0.try_clone().ok()?;
        Some(Box::new(RegularFile(file)))
    }
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        Read::read(&mut self.0, buf)
    }
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Write::write(&mut self.0, buf)
    }
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        Seek::seek(&mut self.0, pos)
    }
    fn flush(&mut self) -> io::Result<()> {
        Write::flush(&mut self.0)
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Wrapper for stdout.
struct StdoutFile;

impl FileOps for StdoutFile {
    fn try_clone(&self) -> Option<Box<dyn FileOps>> {
        Some(Box::new(StdoutFile))
    }
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot read stdout",
        ))
    }
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        io::stdout().write(buf)
    }
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot seek stdout",
        ))
    }
    fn flush(&mut self) -> io::Result<()> {
        io::stdout().flush()
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Wrapper for stderr.
struct StderrFile;

impl FileOps for StderrFile {
    fn try_clone(&self) -> Option<Box<dyn FileOps>> {
        Some(Box::new(StderrFile))
    }
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot read stderr",
        ))
    }
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        io::stderr().write(buf)
    }
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot seek stderr",
        ))
    }
    fn flush(&mut self) -> io::Result<()> {
        io::stderr().flush()
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Wrapper for stdin.
/// Buffered stdin that pre-reads in a background thread, so a read only waits
/// when no input has arrived yet.
struct StdinFile {
    buffer: Arc<Mutex<StdinBuffer>>,
}

struct StdinBuffer {
    data: std::collections::VecDeque<u8>,
    eof: bool,
}

impl StdinFile {
    fn new() -> Self {
        let buffer = Arc::new(Mutex::new(StdinBuffer {
            data: std::collections::VecDeque::new(),
            eof: false,
        }));
        let buf_clone = Arc::clone(&buffer);
        std::thread::spawn(move || {
            let stdin = io::stdin();
            let mut tmp = [0u8; 4096];
            loop {
                match stdin.lock().read(&mut tmp) {
                    Ok(0) => {
                        if let Ok(mut b) = buf_clone.lock() {
                            b.eof = true;
                        }
                        break;
                    }
                    Ok(n) => {
                        if let Ok(mut b) = buf_clone.lock() {
                            b.data.extend(&tmp[..n]);
                        }
                    }
                    Err(_) => {
                        if let Ok(mut b) = buf_clone.lock() {
                            b.eof = true;
                        }
                        break;
                    }
                }
            }
        });
        Self { buffer }
    }
}

/// Take buffered stdin data, waiting for the reader thread to supply it.
/// Returns 0 only at real end of input, so a user typing slowly is never
/// mistaken for EOF.
fn read_stdin_buffer(buffer: &Arc<Mutex<StdinBuffer>>, buf: &mut [u8]) -> io::Result<usize> {
    if buf.is_empty() {
        return Ok(0);
    }
    loop {
        if let Ok(mut b) = buffer.lock() {
            if !b.data.is_empty() {
                let n = buf.len().min(b.data.len());
                for (i, byte) in b.data.drain(..n).enumerate() {
                    buf[i] = byte;
                }
                return Ok(n);
            }
            if b.eof {
                return Ok(0);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

impl FileOps for StdinFile {
    fn try_clone(&self) -> Option<Box<dyn FileOps>> {
        Some(Box::new(StdinFile {
            buffer: Arc::clone(&self.buffer),
        }))
    }
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        read_stdin_buffer(&self.buffer, buf)
    }
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot write stdin",
        ))
    }
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot seek stdin",
        ))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Pseudo-random byte generator for /dev/random.
struct RandomFile;

impl FileOps for RandomFile {
    fn try_clone(&self) -> Option<Box<dyn FileOps>> {
        Some(Box::new(RandomFile))
    }
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Simple xorshift-based PRNG seeded from system time
        let mut seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        for b in buf.iter_mut() {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            *b = seed as u8;
        }
        Ok(buf.len())
    }
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Ok(_buf.len())
    }
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        Ok(0)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// In-memory read-only file for virtual device files.
struct MemoryFile(std::io::Cursor<Vec<u8>>);

impl FileOps for MemoryFile {
    fn try_clone(&self) -> Option<Box<dyn FileOps>> {
        Some(Box::new(MemoryFile(self.0.clone())))
    }
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        Read::read(&mut self.0, buf)
    }
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Ok(_buf.len()) // silently accept writes
    }
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.0.seek(pos)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Wrapper for a connected TCP stream.
pub(crate) struct TcpStreamFile(pub TcpStream);

impl FileOps for TcpStreamFile {
    fn try_clone(&self) -> Option<Box<dyn FileOps>> {
        let stream = self.0.try_clone().ok()?;
        Some(Box::new(TcpStreamFile(stream)))
    }
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        Read::read(&mut self.0, buf)
    }
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Write::write(&mut self.0, buf)
    }
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot seek TCP stream",
        ))
    }
    fn flush(&mut self) -> io::Result<()> {
        Write::flush(&mut self.0)
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Wrapper for a TCP listener (used for announce/listen).
pub(crate) struct TcpListenerFile(pub TcpListener);

impl FileOps for TcpListenerFile {
    fn try_clone(&self) -> Option<Box<dyn FileOps>> {
        let listener = self.0.try_clone().ok()?;
        Some(Box::new(TcpListenerFile(listener)))
    }
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot read listener",
        ))
    }
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot write listener",
        ))
    }
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot seek listener",
        ))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Write-only handle for /dev/audio. Delegates to shared AudioState.
struct AudioFile(Arc<Mutex<AudioState>>);

impl FileOps for AudioFile {
    fn try_clone(&self) -> Option<Box<dyn FileOps>> {
        Some(Box::new(AudioFile(Arc::clone(&self.0))))
    }
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot read /dev/audio",
        ))
    }
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        Ok(state.write(buf))
    }
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot seek /dev/audio",
        ))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Control handle for /dev/audioctl. Writes configure the audio device,
/// reads return the current configuration.
struct AudioCtlFile {
    state: Arc<Mutex<AudioState>>,
    /// Read position in the status string, so a read-until-EOF loop terminates.
    pos: usize,
}

impl FileOps for AudioCtlFile {
    fn try_clone(&self) -> Option<Box<dyn FileOps>> {
        Some(Box::new(AudioCtlFile {
            state: Arc::clone(&self.state),
            pos: self.pos,
        }))
    }
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let status = state.status();
        let bytes = status.as_bytes();
        if self.pos >= bytes.len() {
            return Ok(0);
        }
        let n = buf.len().min(bytes.len() - self.pos);
        buf[..n].copy_from_slice(&bytes[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let cmd = String::from_utf8_lossy(buf);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.configure(&cmd);
        // The status changed, so let it be read again from the start.
        self.pos = 0;
        Ok(buf.len())
    }
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot seek /dev/audioctl",
        ))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Shared state of an in-memory pipe: queued bytes and the number of open write ends.
struct PipeState {
    data: VecDeque<u8>,
    writers: usize,
}

/// Shared buffer for in-memory pipes.
type PipeBuffer = Arc<Mutex<PipeState>>;

/// Longest a pipe read waits for data before reporting `WouldBlock`.
/// The cooperative scheduler runs every VM thread on one OS thread, so a pipe
/// read can never block forever: the writer may be a thread that cannot run
/// until this call returns.
const PIPE_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);

/// Read end of an in-memory pipe.
struct PipeReader(PipeBuffer);

impl FileOps for PipeReader {
    fn try_clone(&self) -> Option<Box<dyn FileOps>> {
        Some(Box::new(PipeReader(Arc::clone(&self.0))))
    }
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let deadline = std::time::Instant::now() + PIPE_READ_TIMEOUT;
        loop {
            {
                let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
                if !state.data.is_empty() {
                    let n = buf.len().min(state.data.len());
                    for (i, b) in state.data.drain(..n).enumerate() {
                        buf[i] = b;
                    }
                    return Ok(n);
                }
                // EOF only once every write end is closed.
                if state.writers == 0 {
                    return Ok(0);
                }
            }
            if std::time::Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "pipe has no data yet",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot write to read end of pipe",
        ))
    }
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot seek pipe",
        ))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Write end of an in-memory pipe.
/// Dropping the last writer makes reads on the other end report EOF.
struct PipeWriter(PipeBuffer);

impl Drop for PipeWriter {
    fn drop(&mut self) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.writers = state.writers.saturating_sub(1);
    }
}

impl FileOps for PipeWriter {
    fn try_clone(&self) -> Option<Box<dyn FileOps>> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.writers += 1;
        drop(state);
        Some(Box::new(PipeWriter(Arc::clone(&self.0))))
    }
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot read from write end of pipe",
        ))
    }
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.data.extend(buf);
        Ok(buf.len())
    }
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot seek pipe",
        ))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Portable file descriptor table.
pub(crate) struct FileTable {
    files: HashMap<i32, FileEntry>,
    next_fd: i32,
    /// Inferno root directory for resolving absolute guest paths.
    /// When non-empty, guest paths starting with `/` are resolved as
    /// `{root}/{guest_path}`. Relative paths are used as-is.
    root: String,
    /// Shared audio state for /dev/audio and /dev/audioctl.
    audio: Arc<Mutex<AudioState>>,
    /// (fd, heap id of the owning FD record) for descriptors opened by the guest.
    owners: Vec<(i32, u32)>,
    /// Number of directory entries already returned per fd, for `dirread`.
    dir_offsets: HashMap<i32, usize>,
}

impl FileTable {
    pub fn new() -> Self {
        Self::with_root(String::new())
    }

    pub fn with_root(root: String) -> Self {
        let mut ft = Self {
            files: HashMap::new(),
            next_fd: 3,
            root,
            audio: Arc::new(Mutex::new(AudioState::new())),
            owners: Vec::new(),
            dir_offsets: HashMap::new(),
        };
        ft.files.insert(
            0,
            FileEntry {
                inner: Box::new(StdinFile::new()),
                path: Some("/dev/stdin".to_string()),
            },
        );
        ft.files.insert(
            1,
            FileEntry {
                inner: Box::new(StdoutFile),
                path: Some("/dev/stdout".to_string()),
            },
        );
        ft.files.insert(
            2,
            FileEntry {
                inner: Box::new(StderrFile),
                path: Some("/dev/stderr".to_string()),
            },
        );
        ft
    }

    /// Resolve a guest path to a host path.
    /// If a root is set and the path starts with `/`, prepend the root.
    /// Otherwise, use the path as-is.
    ///
    /// Absolute guest paths are normalized first, so `..` components can never
    /// climb above the root and reach the rest of the host filesystem.
    pub fn resolve_path(&self, path: &str) -> String {
        if self.root.is_empty() || !path.starts_with('/') {
            return path.to_string();
        }
        let mut parts: Vec<&str> = Vec::new();
        for part in path.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                other => parts.push(other),
            }
        }
        format!("{}/{}", self.root.trim_end_matches('/'), parts.join("/"))
    }

    /// Open a file and return its fd number.
    ///
    /// Intercepts `/dev/audio` and `/dev/audioctl` to provide virtual
    /// audio device files. All other paths are opened on the host filesystem.
    pub fn open(&mut self, path: &str, mode: i32) -> io::Result<i32> {
        // Virtual device files.
        if path == "/dev/audio" {
            return self.open_audio_file(path);
        }
        if path == "/dev/audioctl" {
            return self.open_audioctl_file(path);
        }
        if path == "/dev/sysctl" {
            return self.open_virtual_file(path, b"RiceVM");
        }
        if path == "/dev/sysname" {
            return self.open_virtual_file(path, b"ricevm");
        }
        // /dev/user: current user name (used by many Inferno programs)
        if path == "/dev/user" {
            let user = std::env::var("USER").unwrap_or_else(|_| "inferno".to_string());
            return self.open_virtual_file(path, user.as_bytes());
        }
        // /dev/time: nanoseconds since epoch (used by lockfs, profiling tools)
        if path == "/dev/time" {
            let ns = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let time_str = format!("{ns}");
            return self.open_virtual_file(path, time_str.as_bytes());
        }
        // /dev/cons: console (alias for stdin/stdout depending on mode)
        if path == "/dev/cons" {
            return if mode & 0x3 == 0 {
                // OREAD: return stdin
                Ok(0)
            } else {
                // OWRITE/ORDWR: return stdout
                Ok(1)
            };
        }
        // /dev/null: discard writes, EOF on read
        if path == "/dev/null" {
            return self.open_virtual_file(path, b"");
        }
        // /dev/random: pseudo-random bytes
        if path == "/dev/random" || path == "/dev/urandom" {
            return self.open_random_file(path);
        }
        // /dev/drivers: list of available device drivers (stub)
        if path == "/dev/drivers" {
            return self.open_virtual_file(path, b"#c cons\n#d ssl\n#e env\n#I ip\n#p prog\n");
        }
        // /prog/N/status: process status (stub with running state)
        if path.starts_with("/prog/") && path.ends_with("/status") {
            let status = format!(
                "{:28} {:8} {:12} {:8}\n",
                "ricevm", "running", "release", "0:00"
            );
            return self.open_virtual_file(path, status.as_bytes());
        }
        // /prog/N/wait: process wait file (returns EOF immediately)
        if path.starts_with("/prog/") && path.ends_with("/wait") {
            return self.open_virtual_file(path, b"");
        }
        // /prog/N/ns: namespace listing (stub)
        if path.starts_with("/prog/") && path.ends_with("/ns") {
            return self.open_virtual_file(path, b"");
        }
        // /prog/N/ctl: process control (accepts writes, returns empty on read)
        if path.starts_with("/prog/") && path.ends_with("/ctl") {
            return self.open_virtual_file(path, b"");
        }
        // /env/: environment variables
        if let Some(var_name) = path.strip_prefix("/env/") {
            let val = std::env::var(var_name).unwrap_or_default();
            return self.open_virtual_file(path, val.as_bytes());
        }

        let resolved = self.resolve_path(path);
        let mut opts = std::fs::OpenOptions::new();
        match mode & 0x3 {
            0 => opts.read(true),             // OREAD
            1 => opts.write(true),            // OWRITE
            _ => opts.read(true).write(true), // ORDWR
        };
        if mode & OTRUNC != 0 {
            // Truncation needs write access even for an OREAD open.
            opts.write(true).truncate(true);
        }
        let file = opts.open(&resolved)?;
        let fd = self.next_fd;
        self.next_fd += 1;
        self.files.insert(
            fd,
            FileEntry {
                inner: Box::new(RegularFile(file)),
                path: Some(resolved),
            },
        );
        Ok(fd)
    }

    /// Create a file with the given open mode and permission bits,
    /// and return its fd number.
    pub fn create(&mut self, path: &str, mode: i32, perm: i32) -> io::Result<i32> {
        let resolved = self.resolve_path(path);
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).truncate(true).write(true);
        if mode & 0x3 != 1 {
            // OREAD and ORDWR both leave the new file readable.
            opts.read(true);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let bits = perm as u32 & 0o7777;
            opts.mode(if bits == 0 { 0o666 } else { bits });
        }
        #[cfg(not(unix))]
        let _ = perm;
        let file = opts.open(&resolved)?;
        let fd = self.next_fd;
        self.next_fd += 1;
        self.files.insert(
            fd,
            FileEntry {
                inner: Box::new(RegularFile(file)),
                path: Some(resolved),
            },
        );
        Ok(fd)
    }

    /// Create an in-memory pipe. Returns (read_fd, write_fd).
    pub fn pipe(&mut self) -> (i32, i32) {
        let buf = Arc::new(Mutex::new(PipeState {
            data: VecDeque::new(),
            writers: 1,
        }));
        let read_fd = self.next_fd;
        self.next_fd += 1;
        let write_fd = self.next_fd;
        self.next_fd += 1;
        self.files.insert(
            read_fd,
            FileEntry {
                inner: Box::new(PipeReader(buf.clone())),
                path: None,
            },
        );
        self.files.insert(
            write_fd,
            FileEntry {
                inner: Box::new(PipeWriter(buf)),
                path: None,
            },
        );
        (read_fd, write_fd)
    }

    /// Get an fd entry for reading.
    pub fn read(&mut self, fd: i32, buf: &mut [u8]) -> io::Result<usize> {
        let entry = self
            .files
            .get_mut(&fd)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "bad fd"))?;
        entry.inner.read(buf)
    }

    /// Write to an fd.
    pub fn write(&mut self, fd: i32, buf: &[u8]) -> io::Result<usize> {
        let entry = self
            .files
            .get_mut(&fd)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "bad fd"))?;
        entry.inner.write(buf)
    }

    /// Seek on an fd.
    pub fn seek(&mut self, fd: i32, offset: i64, whence: i32) -> io::Result<u64> {
        let entry = self
            .files
            .get_mut(&fd)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "bad fd"))?;
        let pos = match whence {
            0 => SeekFrom::Start(offset as u64),
            1 => SeekFrom::Current(offset),
            2 => SeekFrom::End(offset),
            _ => SeekFrom::Start(offset as u64),
        };
        entry.inner.seek(pos)
    }

    /// Duplicate an fd onto `new_fd`, closing whatever it held.
    /// A negative `new_fd` allocates a fresh descriptor.
    /// Returns the new fd number, or -1 if the handle cannot be duplicated.
    pub fn dup(&mut self, old_fd: i32, new_fd: i32) -> i32 {
        let Some(entry) = self.files.get(&old_fd) else {
            return -1;
        };
        let Some(inner) = entry.inner.try_clone() else {
            return -1;
        };
        let path = entry.path.clone();

        let target = if new_fd < 0 {
            let fd = self.next_fd;
            self.next_fd += 1;
            fd
        } else {
            self.close(new_fd);
            if new_fd >= self.next_fd {
                self.next_fd = new_fd + 1;
            }
            new_fd
        };
        self.files.insert(target, FileEntry { inner, path });
        target
    }

    /// Create a "fildes" entry for a given fd number (just validates it exists).
    pub fn fildes(&self, fd: i32) -> bool {
        (0..=2).contains(&fd) || self.files.contains_key(&fd)
    }

    /// Close an fd.
    pub fn close(&mut self, fd: i32) {
        if fd > 2 {
            self.files.remove(&fd);
        }
        self.owners.retain(|&(owned, _)| owned != fd);
        self.dir_offsets.remove(&fd);
    }

    /// Record the heap id of the FD record that owns `fd`.
    ///
    /// Inferno closes the underlying descriptor when the `ref FD` value is
    /// reclaimed; tracking the owner lets the caller do the same.
    pub fn set_owner(&mut self, fd: i32, owner: u32) {
        self.owners.retain(|&(owned, _)| owned != fd);
        self.owners.push((fd, owner));
    }

    /// All (fd, owner heap id) pairs currently tracked.
    pub fn owned_fds(&self) -> Vec<(i32, u32)> {
        self.owners.clone()
    }

    /// Number of directory entries already returned for `fd`.
    pub fn dir_offset(&self, fd: i32) -> usize {
        self.dir_offsets.get(&fd).copied().unwrap_or(0)
    }

    /// Record how many directory entries have been returned for `fd`.
    pub fn set_dir_offset(&mut self, fd: i32, offset: usize) {
        self.dir_offsets.insert(fd, offset);
    }

    /// Get the path associated with an fd.
    pub fn get_path(&self, fd: i32) -> Option<&str> {
        self.files.get(&fd)?.path.as_deref()
    }

    /// Insert a TCP stream and return its fd.
    pub fn insert_tcp_stream(&mut self, stream: TcpStream, addr: Option<String>) -> i32 {
        let fd = self.next_fd;
        self.next_fd += 1;
        self.files.insert(
            fd,
            FileEntry {
                inner: Box::new(TcpStreamFile(stream)),
                path: addr,
            },
        );
        fd
    }

    /// Insert a TCP listener and return its fd.
    pub fn insert_tcp_listener(&mut self, listener: TcpListener, addr: Option<String>) -> i32 {
        let fd = self.next_fd;
        self.next_fd += 1;
        self.files.insert(
            fd,
            FileEntry {
                inner: Box::new(TcpListenerFile(listener)),
                path: addr,
            },
        );
        fd
    }

    /// Accept a connection on a listener fd. Returns (new stream fd, peer address).
    pub fn accept_on(&mut self, listener_fd: i32) -> io::Result<(i32, String)> {
        // We need to take the entry out temporarily to call accept, then put it back.
        let mut entry = self
            .files
            .remove(&listener_fd)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "bad fd"))?;

        let result = match entry.inner.as_any_mut().downcast_mut::<TcpListenerFile>() {
            Some(listener_file) => {
                let (stream, addr) = listener_file.0.accept()?;
                let addr_str = addr.to_string();
                let stream_fd = self.insert_tcp_stream(stream, Some(addr_str.clone()));
                Ok((stream_fd, addr_str))
            }
            None => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "fd is not a listener",
            )),
        };

        self.files.insert(listener_fd, entry);
        result
    }

    /// Open a virtual file backed by in-memory data.
    fn open_virtual_file(&mut self, path: &str, data: &[u8]) -> io::Result<i32> {
        let fd = self.next_fd;
        self.next_fd += 1;
        self.files.insert(
            fd,
            FileEntry {
                inner: Box::new(MemoryFile(std::io::Cursor::new(data.to_vec()))),
                path: Some(path.to_string()),
            },
        );
        Ok(fd)
    }

    fn open_random_file(&mut self, path: &str) -> io::Result<i32> {
        let fd = self.next_fd;
        self.next_fd += 1;
        self.files.insert(
            fd,
            FileEntry {
                inner: Box::new(RandomFile),
                path: Some(path.to_string()),
            },
        );
        Ok(fd)
    }

    /// Open a virtual /dev/audio file descriptor.
    fn open_audio_file(&mut self, path: &str) -> io::Result<i32> {
        let fd = self.next_fd;
        self.next_fd += 1;
        self.files.insert(
            fd,
            FileEntry {
                inner: Box::new(AudioFile(Arc::clone(&self.audio))),
                path: Some(path.to_string()),
            },
        );
        Ok(fd)
    }

    /// Open a virtual /dev/audioctl file descriptor.
    fn open_audioctl_file(&mut self, path: &str) -> io::Result<i32> {
        let fd = self.next_fd;
        self.next_fd += 1;
        self.files.insert(
            fd,
            FileEntry {
                inner: Box::new(AudioCtlFile {
                    state: Arc::clone(&self.audio),
                    pos: 0,
                }),
                path: Some(path.to_string()),
            },
        );
        Ok(fd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file_path(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("ricevm_ft_{name}_{nanos}.tmp"))
    }

    #[test]
    fn pipe_read_write() {
        let mut ft = FileTable::new();
        let (read_fd, write_fd) = ft.pipe();

        let data = b"hello pipe";
        let written = ft.write(write_fd, data).expect("pipe write should succeed");
        assert_eq!(written, data.len());

        let mut buf = vec![0u8; 32];
        let n = ft
            .read(read_fd, &mut buf)
            .expect("pipe read should succeed");
        assert_eq!(n, data.len());
        assert_eq!(&buf[..n], data);
    }

    #[test]
    fn new_creates_stdin_stdout_stderr() {
        let ft = FileTable::new();
        assert!(ft.fildes(0), "stdin should exist");
        assert!(ft.fildes(1), "stdout should exist");
        assert!(ft.fildes(2), "stderr should exist");
        assert!(!ft.fildes(99), "fd 99 should not exist");
    }

    #[test]
    fn open_dev_null_reads_eof() {
        let mut ft = FileTable::new();
        let fd = ft
            .open("/dev/null", 0)
            .expect("open /dev/null should succeed");
        assert!(fd >= 3, "virtual file fd should be >= 3");
        let mut buf = [0u8; 16];
        let n = ft.read(fd, &mut buf).expect("read should succeed");
        assert_eq!(n, 0, "/dev/null should read as empty (EOF)");
    }

    #[test]
    fn open_dev_sysctl_returns_ricevm() {
        let mut ft = FileTable::new();
        let fd = ft.open("/dev/sysctl", 0).expect("open /dev/sysctl");
        let mut buf = [0u8; 64];
        let n = ft.read(fd, &mut buf).expect("read should succeed");
        assert_eq!(&buf[..n], b"RiceVM");
    }

    #[test]
    fn open_dev_sysname_returns_ricevm() {
        let mut ft = FileTable::new();
        let fd = ft.open("/dev/sysname", 0).expect("open /dev/sysname");
        let mut buf = [0u8; 64];
        let n = ft.read(fd, &mut buf).expect("read should succeed");
        assert_eq!(&buf[..n], b"ricevm");
    }

    #[test]
    fn open_dev_user_returns_nonempty() {
        let mut ft = FileTable::new();
        let fd = ft.open("/dev/user", 0).expect("open /dev/user");
        let mut buf = [0u8; 256];
        let n = ft.read(fd, &mut buf).expect("read should succeed");
        assert!(n > 0, "/dev/user should return a non-empty username");
    }

    #[test]
    fn open_dev_time_returns_digits() {
        let mut ft = FileTable::new();
        let fd = ft.open("/dev/time", 0).expect("open /dev/time");
        let mut buf = [0u8; 64];
        let n = ft.read(fd, &mut buf).expect("read should succeed");
        let time_str = std::str::from_utf8(&buf[..n]).expect("should be utf8");
        assert!(
            time_str.chars().all(|c| c.is_ascii_digit()),
            "time should be all digits, got: {time_str}"
        );
        assert!(n > 10, "nanosecond timestamp should be long");
    }

    #[test]
    fn resolve_path_no_root() {
        let ft = FileTable::new();
        assert_eq!(ft.resolve_path("/foo/bar"), "/foo/bar");
        assert_eq!(ft.resolve_path("relative"), "relative");
    }

    #[test]
    fn resolve_path_with_root() {
        let ft = FileTable::with_root("/host/inferno".to_string());
        assert_eq!(ft.resolve_path("/foo/bar"), "/host/inferno/foo/bar");
        assert_eq!(
            ft.resolve_path("relative"),
            "relative",
            "relative paths should not be prefixed"
        );
    }

    #[test]
    fn regular_file_read_write_seek() {
        let path = std::env::temp_dir().join(format!(
            "ricevm_ft_test_{}.tmp",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"abcdef").expect("write temp file");

        let mut ft = FileTable::new();
        let fd = ft
            .open(path.to_str().unwrap(), 2) // ORDWR
            .expect("open should succeed");

        // Read first 3 bytes
        let mut buf = [0u8; 3];
        let n = ft.read(fd, &mut buf).expect("read");
        assert_eq!(n, 3);
        assert_eq!(&buf, b"abc");

        // Seek back to start
        let pos = ft.seek(fd, 0, 0).expect("seek start");
        assert_eq!(pos, 0);

        // Read again
        let n = ft.read(fd, &mut buf).expect("read after seek");
        assert_eq!(n, 3);
        assert_eq!(&buf, b"abc");

        // Seek to offset 2 from start
        ft.seek(fd, 2, 0).expect("seek to 2");
        let n = ft.read(fd, &mut buf).expect("read from offset 2");
        assert_eq!(n, 3);
        assert_eq!(&buf, b"cde");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn close_removes_fd() {
        let mut ft = FileTable::new();
        let fd = ft.open("/dev/null", 0).expect("open /dev/null");
        assert!(ft.fildes(fd));
        ft.close(fd);
        assert!(!ft.fildes(fd), "fd should be gone after close");
    }

    #[test]
    fn close_does_not_remove_stdio() {
        let mut ft = FileTable::new();
        ft.close(0);
        ft.close(1);
        ft.close(2);
        assert!(ft.fildes(0), "stdin should not be closeable");
        assert!(ft.fildes(1), "stdout should not be closeable");
        assert!(ft.fildes(2), "stderr should not be closeable");
    }

    #[test]
    fn get_path_for_stdio() {
        let ft = FileTable::new();
        assert_eq!(ft.get_path(0), Some("/dev/stdin"));
        assert_eq!(ft.get_path(1), Some("/dev/stdout"));
        assert_eq!(ft.get_path(2), Some("/dev/stderr"));
        assert_eq!(ft.get_path(99), None);
    }

    #[test]
    fn open_dev_cons_read_returns_stdin() {
        let mut ft = FileTable::new();
        let fd = ft.open("/dev/cons", 0).expect("open /dev/cons OREAD");
        assert_eq!(fd, 0, "/dev/cons OREAD should alias stdin");
    }

    #[test]
    fn open_dev_cons_write_returns_stdout() {
        let mut ft = FileTable::new();
        let fd = ft.open("/dev/cons", 1).expect("open /dev/cons OWRITE");
        assert_eq!(fd, 1, "/dev/cons OWRITE should alias stdout");
    }

    #[test]
    fn create_file_and_write() {
        let path = temp_file_path("create_test");

        let mut ft = FileTable::new();
        let fd = ft.create(path.to_str().unwrap(), 1, 0o666).expect("create");
        let n = ft.write(fd, b"hello").expect("write");
        assert_eq!(n, 5);
        ft.close(fd);

        let contents = std::fs::read(&path).expect("read back");
        assert_eq!(&contents, b"hello");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn resolve_path_cannot_escape_root() {
        let ft = FileTable::with_root("/host/inferno".to_string());
        assert_eq!(
            ft.resolve_path("/../../etc/passwd"),
            "/host/inferno/etc/passwd",
            "guest paths must stay inside the root"
        );
        assert_eq!(ft.resolve_path("/a/../b"), "/host/inferno/b");
        assert_eq!(ft.resolve_path("/./a//b"), "/host/inferno/a/b");
    }

    #[test]
    fn dup_binds_new_fd() {
        let mut ft = FileTable::new();
        let fd = ft.open("/dev/sysctl", 0).expect("open /dev/sysctl");
        assert_eq!(ft.dup(fd, 7), 7, "dup should return the requested fd");
        assert!(ft.fildes(7), "the new fd should be bound");

        let mut buf = [0u8; 64];
        let n = ft.read(7, &mut buf).expect("read from dup'd fd");
        assert_eq!(&buf[..n], b"RiceVM");
    }

    #[test]
    fn dup_with_negative_new_fd_allocates_one() {
        let mut ft = FileTable::new();
        let fd = ft.open("/dev/sysctl", 0).expect("open /dev/sysctl");
        let new_fd = ft.dup(fd, -1);
        assert!(new_fd >= 3, "dup(-1) should allocate a fresh fd");
        assert_ne!(new_fd, fd);
        assert!(ft.fildes(new_fd));
    }

    #[test]
    fn dup_bad_old_fd_returns_error() {
        let mut ft = FileTable::new();
        assert_eq!(ft.dup(999, 7), -1, "dup of an unknown fd should fail");
        assert!(!ft.fildes(7));
    }

    #[test]
    fn create_honours_mode_and_perm() {
        let path = temp_file_path("create_mode");

        let mut ft = FileTable::new();
        let fd = ft
            .create(path.to_str().expect("temp path should be utf-8"), 2, 0o600)
            .expect("create ORDWR");
        ft.write(fd, b"hello").expect("write");
        ft.seek(fd, 0, 0).expect("seek to start");
        let mut buf = [0u8; 8];
        let n = ft
            .read(fd, &mut buf)
            .expect("an ORDWR fd should be readable");
        assert_eq!(&buf[..n], b"hello");
        ft.close(fd);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path)
                .expect("stat created file")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "perm argument should be applied");
        }

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn open_with_otrunc_truncates() {
        let path = temp_file_path("otrunc");
        std::fs::write(&path, b"abcdefgh").expect("write temp file");

        let mut ft = FileTable::new();
        let fd = ft
            .open(path.to_str().expect("temp path should be utf-8"), 1 | 16)
            .expect("open OWRITE|OTRUNC");
        ft.write(fd, b"xy").expect("write");
        ft.close(fd);

        assert_eq!(
            std::fs::read(&path).expect("read back"),
            b"xy",
            "OTRUNC should discard the old contents"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn owned_fds_track_and_close() {
        let mut ft = FileTable::new();
        let fd = ft.open("/dev/sysctl", 0).expect("open /dev/sysctl");
        ft.set_owner(fd, 42);
        assert_eq!(ft.owned_fds(), vec![(fd, 42)]);
        ft.close(fd);
        assert!(!ft.fildes(fd), "close should release the fd");
        assert!(
            ft.owned_fds().is_empty(),
            "close should drop the owner entry"
        );
    }

    #[test]
    fn stdin_read_waits_for_late_data() {
        let buffer = Arc::new(Mutex::new(StdinBuffer {
            data: VecDeque::new(),
            eof: false,
        }));
        let writer = Arc::clone(&buffer);
        let handle = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            writer
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .data
                .extend(b"hi");
        });

        let mut buf = [0u8; 8];
        let n = read_stdin_buffer(&buffer, &mut buf).expect("stdin read");
        handle.join().expect("writer thread");
        assert_eq!(&buf[..n], b"hi", "slow input must not look like EOF");
    }

    #[test]
    fn stdin_read_returns_eof_at_end_of_input() {
        let buffer = Arc::new(Mutex::new(StdinBuffer {
            data: VecDeque::new(),
            eof: true,
        }));
        let mut buf = [0u8; 8];
        let n = read_stdin_buffer(&buffer, &mut buf).expect("stdin read");
        assert_eq!(n, 0, "EOF is reported only once the input really ended");
    }

    #[test]
    fn pipe_read_reports_eof_only_after_writer_closes() {
        let mut ft = FileTable::new();
        let (read_fd, write_fd) = ft.pipe();
        ft.write(write_fd, b"ab").expect("pipe write");
        ft.close(write_fd);

        let mut buf = [0u8; 8];
        assert_eq!(ft.read(read_fd, &mut buf).expect("pipe read"), 2);
        assert_eq!(
            ft.read(read_fd, &mut buf).expect("pipe read at EOF"),
            0,
            "EOF once the writer is closed"
        );
    }

    #[test]
    fn pipe_read_does_not_report_eof_while_writer_open() {
        let mut ft = FileTable::new();
        let (read_fd, _write_fd) = ft.pipe();
        let mut buf = [0u8; 8];
        let err = ft
            .read(read_fd, &mut buf)
            .expect_err("an empty pipe with an open writer must not report EOF");
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
    }

    #[test]
    fn audioctl_read_reaches_eof() {
        let mut ft = FileTable::new();
        let fd = ft.open("/dev/audioctl", 0).expect("open /dev/audioctl");
        let mut buf = [0u8; 8];
        let mut all = Vec::new();
        loop {
            let n = ft.read(fd, &mut buf).expect("audioctl read");
            if n == 0 {
                break;
            }
            all.extend_from_slice(&buf[..n]);
            assert!(
                all.len() <= 256,
                "a read-until-EOF loop should terminate, got {} bytes",
                all.len()
            );
        }
        assert!(String::from_utf8_lossy(&all).contains("rate 44100"));
    }

    #[test]
    fn read_bad_fd_returns_error() {
        let mut ft = FileTable::new();
        let result = ft.read(999, &mut [0u8; 4]);
        assert!(result.is_err(), "reading from bad fd should error");
    }

    #[test]
    fn write_bad_fd_returns_error() {
        let mut ft = FileTable::new();
        let result = ft.write(999, b"data");
        assert!(result.is_err(), "writing to bad fd should error");
    }
}
