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
}

/// Wrapper for std::fs::File (supports all operations).
struct RegularFile(std::fs::File);

impl FileOps for RegularFile {
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
/// Buffered stdin that pre-reads in a background thread so reads don't block the VM.
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

impl FileOps for StdinFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Try to read from buffer; if empty and not EOF, wait briefly
        for _ in 0..50 {
            if let Ok(mut b) = self.buffer.lock() {
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
        // Timeout: treat as EOF (the reader thread may not have started yet)
        Ok(0)
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
struct AudioCtlFile(Arc<Mutex<AudioState>>);

impl FileOps for AudioCtlFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let status = state.status();
        let bytes = status.as_bytes();
        let n = buf.len().min(bytes.len());
        buf[..n].copy_from_slice(&bytes[..n]);
        Ok(n)
    }
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let cmd = String::from_utf8_lossy(buf);
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.configure(&cmd);
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

/// Shared buffer for in-memory pipes.
type PipeBuffer = Arc<Mutex<VecDeque<u8>>>;

/// Read end of an in-memory pipe.
struct PipeReader(PipeBuffer);

impl FileOps for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut queue = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let n = buf.len().min(queue.len());
        for b in buf.iter_mut().take(n) {
            *b = queue.pop_front().unwrap_or(0);
        }
        Ok(n)
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
struct PipeWriter(PipeBuffer);

impl FileOps for PipeWriter {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cannot read from write end of pipe",
        ))
    }
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut queue = self.0.lock().unwrap_or_else(|e| e.into_inner());
        queue.extend(buf);
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
    pub fn resolve_path(&self, path: &str) -> String {
        if !self.root.is_empty() && path.starts_with('/') {
            format!("{}{path}", self.root)
        } else {
            path.to_string()
        }
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
        let file = match mode & 0x3 {
            0 => std::fs::File::open(&resolved)?, // OREAD
            1 => std::fs::OpenOptions::new().write(true).open(&resolved)?, // OWRITE
            _ => std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&resolved)?, // ORDWR
        };
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

    /// Create a file and return its fd number.
    pub fn create(&mut self, path: &str) -> io::Result<i32> {
        let resolved = self.resolve_path(path);
        let file = std::fs::File::create(&resolved)?;
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
        let buf = Arc::new(Mutex::new(VecDeque::new()));
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

    /// Duplicate an fd (returns the new fd number).
    pub fn dup(&mut self, _old_fd: i32, _new_fd: i32) -> i32 {
        // Simplified: we can't truly duplicate Rust file handles without platform-specific code.
        // Return the old fd as-is (aliasing). This is imperfect but portable.
        _old_fd
    }

    /// Create a "fildes" entry for a given fd number (just validates it exists).
    pub fn fildes(&self, fd: i32) -> bool {
        (0..=2).contains(&fd) || self.files.contains_key(&fd)
    }

    /// Close an fd.
    #[allow(dead_code)]
    pub fn close(&mut self, fd: i32) {
        if fd > 2 {
            self.files.remove(&fd);
        }
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
                inner: Box::new(AudioCtlFile(Arc::clone(&self.audio))),
                path: Some(path.to_string()),
            },
        );
        Ok(fd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
