//! Portable file descriptor table.
//!
//! Maps integer fd numbers to Rust I/O objects, replacing raw Unix fd operations.
//! Pre-populated with stdin(0), stdout(1), stderr(2).

use std::any::Any;
use std::collections::HashMap;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};

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
struct StdinFile;

impl FileOps for StdinFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        io::stdin().read(buf)
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

/// Portable file descriptor table.
pub(crate) struct FileTable {
    files: HashMap<i32, FileEntry>,
    next_fd: i32,
    /// Inferno root directory for resolving absolute guest paths.
    /// When non-empty, guest paths starting with `/` are resolved as
    /// `{root}/{guest_path}`. Relative paths are used as-is.
    root: String,
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
        };
        ft.files.insert(
            0,
            FileEntry {
                inner: Box::new(StdinFile),
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
    pub fn open(&mut self, path: &str, mode: i32) -> io::Result<i32> {
        let resolved = self.resolve_path(path);
        let file = match mode & 0x3 {
            0 => std::fs::File::open(&resolved)?,                          // OREAD
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
}
