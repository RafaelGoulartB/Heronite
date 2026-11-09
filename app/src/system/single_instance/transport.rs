//! The connection between instances. Unix: a unix socket. Windows: Rust has
//! no unix sockets there, so the instance listens on a loopback port and
//! writes the port and a random key into the file at the socket's path (in
//! the user's own temp folder); a connection starts with the key, so other
//! users of the computer cannot send requests.

#[cfg(unix)]
pub(super) use std::os::unix::net::UnixStream as Stream;

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
pub(super) use windows::{bind, connect};

#[cfg(windows)]
mod windows {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::io::{self, ErrorKind, Read, Write};
    use std::net::{Ipv4Addr, SocketAddr, TcpListener};
    use std::path::Path;
    use std::time::Duration;

    pub(in super::super) use std::net::TcpStream as Stream;

    pub(in super::super) struct Listener {
        inner: TcpListener,
        key: String,
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

    pub(in super::super) fn bind(path: &Path) -> io::Result<Listener> {
        let inner = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let key = new_key();
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|e| match e.kind() {
            ErrorKind::AlreadyExists => io::Error::new(ErrorKind::AddrInUse, e),
            _ => e,
        })?;
        writeln!(file, "{} {key}", inner.local_addr()?.port())?;
        Ok(Listener { inner, key })
    }

    pub(in super::super) fn connect(path: &Path) -> io::Result<Stream> {
        let text = std::fs::read_to_string(path)?;
        let refused = || io::Error::from(ErrorKind::ConnectionRefused);
        let (port, key) = text.trim().split_once(' ').ok_or_else(refused)?;
        let port: u16 = port.parse().map_err(|_| refused())?;
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let mut stream = Stream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| match e.kind() {
            ErrorKind::TimedOut => refused(),
            _ => e,
        })?;
        stream.write_all(format!("{key}\n").as_bytes())?;
        Ok(stream)
    }
}
