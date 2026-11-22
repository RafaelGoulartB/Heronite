//! The connection between instances. Unix: a unix socket. Windows: Rust has
//! no unix sockets there, so the instance listens on a loopback port and
//! writes the port and a random key into the file at the socket's path (in
//! the user's own temp folder); a connection starts with the key, so other
//! users of the computer cannot send requests. On Windows the instance also
//! keeps that file open without write-sharing, so a second invocation can
//! tell a live owner from a stale file whose port another process has since
//! taken (after a crash or forced restart).

#[cfg(unix)]
pub(super) use std::os::unix::net::UnixStream as Stream;

/// Keeps the socket file claimed while the listener lives. Unix sockets are
/// bound in the kernel by the listener itself, so there is nothing extra to
/// hold or release there.
#[cfg(unix)]
pub(super) struct Lock;

#[cfg(unix)]
impl Lock {
    pub(super) fn release(&self) {}
}

#[cfg(unix)]
pub(super) fn lock(_listener: &std::os::unix::net::UnixListener) -> Lock {
    Lock
}

/// Removes a stale socket file during takeover. `NotFound` is ignored in case
/// another starting process already removed it.
#[cfg(unix)]
pub(super) fn remove_stale(path: &std::path::Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Listens at `path`; `AddrInUse` when something is there already (a live
/// instance or a stale file: the caller tells them apart with [`connect`]).
#[cfg(unix)]
pub(super) fn bind(path: &std::path::Path) -> std::io::Result<std::os::unix::net::UnixListener> {
    std::os::unix::net::UnixListener::bind(path)
}

/// Connects to the instance at `path`: `NotFound` when there is no file,
/// `ConnectionRefused` when nobody listens behind it.
#[cfg(unix)]
pub(super) fn connect(path: &std::path::Path) -> std::io::Result<Stream> {
    Stream::connect(path)
}

#[cfg(windows)]
pub(super) use windows::{bind, connect, lock, remove_stale, Lock};

#[cfg(windows)]
mod windows {
    use std::collections::hash_map::RandomState;
    use std::fs::{File, OpenOptions};
    use std::hash::{BuildHasher, Hasher};
    use std::io::{self, ErrorKind, Read, Write};
    use std::net::{Ipv4Addr, SocketAddr, TcpListener};
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    pub(in super::super) use std::net::TcpStream as Stream;

    /// Win32 `FILE_SHARE_READ`: other processes may read the port and key,
    /// while opening the file for writing fails with `ERROR_SHARING_VIOLATION`
    /// as long as the owning instance is alive (the kernel closes the handle
    /// on crash or forced restart).
    const FILE_SHARE_READ: u32 = 1;
    /// Win32 `ERROR_SHARING_VIOLATION`.
    const ERROR_SHARING_VIOLATION: i32 = 32;

    /// Handle to the open socket file held by [`Listener`]. Released right
    /// before removing the file on shutdown, so Windows deletes it immediately
    /// instead of leaving it delete-pending while the accept thread runs.
    #[derive(Clone)]
    pub(in super::super) struct Lock(Arc<Mutex<Option<File>>>);

    impl Lock {
        pub(in super::super) fn release(&self) {
            self.0.lock().unwrap_or_else(|e| e.into_inner()).take();
        }
    }

    pub(in super::super) fn lock(listener: &Listener) -> Lock {
        listener.lock.clone()
    }

    pub(in super::super) struct Listener {
        inner: TcpListener,
        key: String,
        lock: Lock,
    }

    impl Listener {
        /// The next connection that brought the key; others are dropped.
        pub(in super::super) fn accept(&self) -> io::Result<(Stream, SocketAddr)> {
            loop {
                let (stream, addr) = self.inner.accept()?;
                if self.knows_key(&stream) {
                    return Ok((stream, addr));
                }
            }
        }

        pub(in super::super) fn incoming(&self) -> impl Iterator<Item = io::Result<Stream>> + '_ {
            std::iter::from_fn(|| Some(self.accept().map(|(stream, _)| stream)))
        }

        /// The first line of the connection is the key. Read a byte at a time:
        /// what follows is the request, which the caller reads.
        fn knows_key(&self, mut stream: &Stream) -> bool {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut line = Vec::new();
            let mut byte = [0u8];
            while line.len() <= self.key.len() {
                match stream.read(&mut byte) {
                    Ok(1) if byte[0] == b'\n' => return line == self.key.as_bytes(),
                    Ok(1) => line.push(byte[0]),
                    _ => return false,
                }
            }
            false
        }
    }

    fn new_key() -> String {
        // Each `RandomState` is seeded from the system's random source.
        let half = || {
            let mut h = RandomState::new().build_hasher();
            h.write_u32(std::process::id());
            h.finish()
        };
        format!("{:016x}{:016x}", half(), half())
    }

    fn parse_endpoint(text: &str) -> Option<(u16, String)> {
        let line = text.split_once('\n')?.0;
        let (port, key) = line.trim().split_once(' ')?;
        let port: u16 = port.parse().ok()?;
        (!key.is_empty()).then(|| (port, key.to_string()))
    }

    /// Removes a stale socket file during takeover. `NotFound` means another
    /// process already removed it; `ERROR_SHARING_VIOLATION` means another
    /// live instance won the race and bound `path` (`AddrInUse`).
    pub(in super::super) fn remove_stale(path: &Path) -> io::Result<()> {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(e) if e.raw_os_error() == Some(ERROR_SHARING_VIOLATION) => Err(io::Error::new(ErrorKind::AddrInUse, e)),
            other => other,
        }
    }

    /// Probes whether a live owner currently holds `path` open without
    /// write-sharing: `Ok(true)` on `ERROR_SHARING_VIOLATION`, `Ok(false)`
    /// when unlocked (stale), and `Err` for `NotFound` or other open errors.
    fn is_held(path: &Path) -> io::Result<bool> {
        match OpenOptions::new().write(true).open(path) {
            Ok(_) => Ok(false),
            Err(e) if e.raw_os_error() == Some(ERROR_SHARING_VIOLATION) => Ok(true),
            Err(e) => Err(e),
        }
    }

    /// Reads `<port> <key>\n` from a live owner's socket file. If the owner
    /// just created the file and has not finished writing the line yet, wait
    /// briefly for the trailing newline to appear.
    fn read_endpoint(path: &Path) -> io::Result<(u16, String)> {
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            let text = std::fs::read_to_string(path)?;
            if text.contains('\n') {
                return parse_endpoint(&text).ok_or_else(|| io::Error::from(ErrorKind::InvalidData));
            }
            if Instant::now() >= deadline {
                return Err(io::Error::from(ErrorKind::TimedOut));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    pub(in super::super) fn bind(path: &Path) -> io::Result<Listener> {
        let inner = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = inner.local_addr()?.port();
        let key = new_key();
        let mut file =
            OpenOptions::new().write(true).create_new(true).share_mode(FILE_SHARE_READ).open(path).map_err(|e| match e.kind() {
                ErrorKind::AlreadyExists => io::Error::new(ErrorKind::AddrInUse, e),
                _ => e,
            })?;
        let line = format!("{port} {key}\n");
        if let Err(e) = file.write_all(line.as_bytes()).and_then(|_| file.flush()) {
            drop(file);
            let _ = std::fs::remove_file(path);
            return Err(e);
        }
        Ok(Listener { inner, key, lock: Lock(Arc::new(Mutex::new(Some(file)))) })
    }

    pub(in super::super) fn connect(path: &Path) -> io::Result<Stream> {
        let refused = || io::Error::from(ErrorKind::ConnectionRefused);
        // Nobody holds the file open without write-sharing: the instance that
        // created it is gone (crash or forced restart), even if another
        // process has since taken its ephemeral TCP port. (A transient
        // antivirus lock without write-sharing falls through once to the
        // normal connect path below.)
        if !is_held(path)? {
            return Err(refused());
        }
        let timeout_or_stale = |e: io::Error| match is_held(path) {
            Ok(false) => refused(),
            Ok(true) => e,
            Err(probe_err) => probe_err,
        };
        let (port, key) = read_endpoint(path).map_err(|e| if e.kind() == ErrorKind::TimedOut { timeout_or_stale(e) } else { e })?;
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let mut stream = Stream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| {
            if e.kind() == ErrorKind::TimedOut {
                timeout_or_stale(e)
            } else {
                e
            }
        })?;
        stream.write_all(format!("{key}\n").as_bytes())?;
        Ok(stream)
    }
}
