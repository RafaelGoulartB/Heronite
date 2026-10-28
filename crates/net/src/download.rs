use std::fs::{self, File};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, RecvTimeoutError, SyncSender};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256, Sha512};

use crate::error::{io_error, redact_url, scrub, Error};

/// How often the caller wakes up to look at the cancel check and the clock.
const TICK: Duration = Duration::from_millis(100);
/// Bytes of the body kept in `Outcome::head` for magic-number checks.
const HEAD_LEN: usize = 512;
const CHUNK: usize = 64 * 1024;
/// Chunks in flight between the reader thread and the caller.
const QUEUE: usize = 4;
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hash {
    Sha256,
    Sha512,
}

/// What a finished download looks like, handed to `Options::validate` before
/// the file gets its final name.
#[derive(Debug)]
pub struct Outcome {
    pub bytes: u64,
    /// The first bytes of the body (up to 512).
    pub head: Vec<u8>,
    /// Lowercase hex of the body, when `Options::hash` asked for one.
    pub hash: Option<String>,
}

pub struct Options<'a> {
    pub user_agent: &'a str,
    /// Extra request headers.
    pub headers: &'a [(&'a str, &'a str)],
    /// DNS lookup and TCP/TLS connect, each.
    pub connect_timeout: Duration,
    /// Sending the request and waiting for the response headers, each.
    pub header_timeout: Duration,
    /// No bytes for this long fails the download.
    pub stall_timeout: Duration,
    /// Upper bound for the whole body. It only matters once a read is stuck
    /// and abandoned: that reader thread cannot be woken, it ends when this
    /// elapses, so keep it far above any real download.
    pub max_duration: Duration,
    pub max_redirects: u32,
    /// Exact size of the body; any other length is `Invalid`.
    pub expected_size: Option<u64>,
    /// Size the caller believes in when the server sends no `Content-Length`:
    /// the progress total, and a shorter body is `Invalid`.
    pub size_hint: Option<u64>,
    /// More bytes than this is `Invalid` (checked while streaming).
    pub max_bytes: Option<u64>,
    pub hash: Option<Hash>,
    /// Lowercase hex SHA-256 the body must have (turns `hash` on).
    pub expected_sha256: Option<&'a str>,
    /// Runs once the whole body arrived and passed the checks above, before
    /// the rename. `Err` becomes `Error::Invalid`.
    pub validate: Option<&'a dyn Fn(&Outcome) -> Result<(), String>>,
    /// Gets every chunk as it is written (a streaming signature check).
    pub on_data: Option<&'a mut dyn FnMut(&[u8])>,
    /// `(done, total)` about every 100 ms, once at the start and once at the end.
    pub on_progress: Option<&'a mut dyn FnMut(u64, Option<u64>)>,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Self {
            user_agent: "heronite-rs",
            headers: &[],
            connect_timeout: Duration::from_secs(15),
            header_timeout: Duration::from_secs(30),
            stall_timeout: Duration::from_secs(30),
            max_duration: Duration::from_secs(12 * 3600),
            max_redirects: 8,
            expected_size: None,
            size_hint: None,
            max_bytes: None,
            hash: None,
            expected_sha256: None,
            validate: None,
            on_data: None,
            on_progress: None,
        }
    }
}

/// What the reader thread tells the caller.
enum Msg {
    Start { total: Option<u64> },
    Data(Vec<u8>),
    Done,
    Failed(Error),
}

/// Removes the partial file unless the download finished.
struct Part {
    path: PathBuf,
    keep: bool,
}

impl Drop for Part {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_file(&self.path);
        }
    }
}

enum Hasher {
    Sha256(Sha256),
    Sha512(Sha512),
}

impl Hasher {
    fn new(kind: Hash) -> Self {
        match kind {
            Hash::Sha256 => Hasher::Sha256(Sha256::new()),
            Hash::Sha512 => Hasher::Sha512(Sha512::new()),
        }
    }
    fn update(&mut self, data: &[u8]) {
        match self {
            Hasher::Sha256(h) => h.update(data),
            Hasher::Sha512(h) => h.update(data),
        }
    }
    fn hex(self) -> String {
        let bytes = match self {
            Hasher::Sha256(h) => h.finalize().to_vec(),
            Hasher::Sha512(h) => h.finalize().to_vec(),
        };
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Streams `url` into `dest` through `<dest>.part`, renamed onto `dest` when
/// the whole body arrived and passed every check. Blocking. On any error,
/// cancel included, the partial file is removed and `dest` is left as it was.
///
/// Stall and cancel: ureq has only deadline timeouts (none restarts on a
/// read), so the request and the reads run on a helper thread and this thread
/// waits on a channel with a short tick. A cancel or a stall returns at once;
/// the helper may stay blocked in `read()` until the server answers or
/// `Options::max_duration` ends the body, then it exits by itself.
pub fn download(url: &str, dest: &Path, mut opts: Options, cancel: &dyn Fn() -> bool) -> Result<Outcome, Error> {
    if cancel() {
        return Err(Error::Cancelled);
    }
    if let Some(dir) = dest.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir).map_err(|e| io_error(format!("creating {}", dir.display()), e))?;
    }
    let mut part = Part { path: part_path(dest), keep: false };

    let (tx, rx) = sync_channel::<Msg>(QUEUE);
    let request = Request::of(url, &opts);
    std::thread::Builder::new()
        .name("heronite-net".into())
        .spawn(move || request.run(tx))
        .map_err(|e| io_error("starting the download", e))?;

    let hash = opts.hash.or(opts.expected_sha256.map(|_| Hash::Sha256));
    let mut hasher = hash.map(Hasher::new);
    let mut file: Option<File> = None;
    let (mut got, mut total) = (0u64, opts.size_hint);
    let mut head = Vec::new();
    let mut last_data = Instant::now();
    let mut last_report = Instant::now();
    let mut started = false;

    loop {
        if cancel() {
            return Err(Error::Cancelled);
        }
        let msg = match rx.recv_timeout(TICK) {
            Ok(m) => m,
            Err(RecvTimeoutError::Timeout) => {
                if started && last_data.elapsed() >= opts.stall_timeout {
                    return Err(Error::Stalled(opts.stall_timeout));
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(Error::Io { context: "downloading".into(), source: ErrorKind::BrokenPipe.into() });
            }
        };
        last_data = Instant::now();
        match msg {
            Msg::Start { total: header_total } => {
                started = true;
                total = header_total.or(total);
                if let (Some(max), Some(t)) = (opts.max_bytes, header_total) {
                    if t > max {
                        return Err(Error::Invalid(format!("the file is larger than the {max} bytes allowed")));
                    }
                }
                file = Some(File::create(&part.path).map_err(|e| io_error(format!("creating {}", part.path.display()), e))?);
                if let Some(f) = opts.on_progress.as_mut() {
                    f(0, total);
                }
            }
            Msg::Data(chunk) => {
                got += chunk.len() as u64;
                if opts.max_bytes.is_some_and(|max| got > max) {
                    return Err(Error::Invalid(format!("the file is larger than the {} bytes allowed", opts.max_bytes.unwrap_or(0))));
                }
                if let Some(f) = file.as_mut() {
                    f.write_all(&chunk).map_err(|e| io_error(format!("writing {}", part.path.display()), e))?;
                }
                if head.len() < HEAD_LEN {
                    head.extend_from_slice(&chunk[..chunk.len().min(HEAD_LEN - head.len())]);
                }
                if let Some(h) = hasher.as_mut() {
                    h.update(&chunk);
                }
                if let Some(f) = opts.on_data.as_mut() {
                    f(&chunk);
                }
                if last_report.elapsed() >= PROGRESS_EVERY {
                    last_report = Instant::now();
                    if let Some(f) = opts.on_progress.as_mut() {
                        f(got, total);
                    }
                }
            }
            Msg::Failed(e) => return Err(e),
            Msg::Done => break,
        }
    }

    if let Some(mut f) = file {
        f.flush().map_err(|e| io_error(format!("writing {}", part.path.display()), e))?;
    }
    if let Some(t) = total.filter(|t| got < *t) {
        return Err(Error::Invalid(format!("connection closed after {got} of {t} bytes")));
    }
    if let Some(n) = opts.expected_size.filter(|n| got != *n) {
        return Err(Error::Invalid(format!("the file has {got} bytes, expected {n}")));
    }
    let outcome = Outcome { bytes: got, head, hash: hasher.map(Hasher::hex) };
    if let Some(want) = opts.expected_sha256 {
        if outcome.hash.as_deref() != Some(&want.to_ascii_lowercase()) {
            return Err(Error::Invalid("the checksum of the file does not match".into()));
        }
    }
    if let Some(validate) = opts.validate {
        validate(&outcome).map_err(Error::Invalid)?;
    }
    if let Some(f) = opts.on_progress.as_mut() {
        f(got, total.or(Some(got)));
    }
    fs::rename(&part.path, dest).map_err(|e| io_error(format!("renaming {}", part.path.display()), e))?;
    part.keep = true;
    Ok(outcome)
}

fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".part");
    dest.with_file_name(name)
}

/// Everything the reader thread needs, owned.
struct Request {
    url: String,
    user_agent: String,
    headers: Vec<(String, String)>,
    connect_timeout: Duration,
    header_timeout: Duration,
    stall_timeout: Duration,
    max_duration: Duration,
    max_redirects: u32,
}

impl Request {
    fn of(url: &str, o: &Options) -> Self {
        Self {
            url: url.to_string(),
            user_agent: o.user_agent.to_string(),
            headers: o.headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            connect_timeout: o.connect_timeout,
            header_timeout: o.header_timeout,
            stall_timeout: o.stall_timeout,
            max_duration: o.max_duration,
            max_redirects: o.max_redirects,
        }
    }

    fn run(self, tx: SyncSender<Msg>) {
        if let Err(e) = self.stream(&tx) {
            let _ = tx.send(Msg::Failed(e));
        }
    }

    /// Sends are best-effort: a closed channel means the caller gave up.
    fn stream(&self, tx: &SyncSender<Msg>) -> Result<(), Error> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_resolve(Some(self.connect_timeout))
            .timeout_connect(Some(self.connect_timeout))
            .timeout_send_request(Some(self.header_timeout))
            .timeout_recv_response(Some(self.header_timeout))
            .timeout_recv_body(Some(self.max_duration))
            .max_redirects(self.max_redirects)
            .user_agent(&self.user_agent)
            .build()
            .into();
        let mut req = agent.get(&self.url);
        for (k, v) in &self.headers {
            req = req.header(k, v);
        }
        let mut resp = req.call().map_err(|e| self.request_error(e))?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(status_error(status, resp.headers()));
        }
        let total =
            resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok()).filter(|n| *n > 0);
        if tx.send(Msg::Start { total }).is_err() {
            return Ok(());
        }
        let mut body = resp.body_mut().with_config().limit(u64::MAX).reader();
        loop {
            let mut buf = vec![0u8; CHUNK];
            let n = body.read(&mut buf).map_err(|e| self.body_error(e))?;
            if n == 0 {
                let _ = tx.send(Msg::Done);
                return Ok(());
            }
            buf.truncate(n);
            if tx.send(Msg::Data(buf)).is_err() {
                return Ok(());
            }
        }
    }

    fn request_error(&self, e: ureq::Error) -> Error {
        use ureq::{Error as E, Timeout};
        let url = redact_url(&self.url);
        let unreachable = |e: &E| Error::Unreachable { url: url.clone(), detail: scrub(&e.to_string()) };
        match e {
            E::Timeout(Timeout::RecvResponse | Timeout::RecvBody) => Error::Stalled(self.header_timeout),
            E::Timeout(_) | E::HostNotFound | E::ConnectionFailed | E::Io(_) | E::Tls(_) => unreachable(&e),
            e => Error::Invalid(scrub(&e.to_string())),
        }
    }

    fn body_error(&self, e: std::io::Error) -> Error {
        match ureq::Error::from(e) {
            ureq::Error::Timeout(_) => Error::Stalled(self.stall_timeout),
            ureq::Error::Io(e) => io_error("reading the response", e),
            e => Error::Invalid(scrub(&e.to_string())),
        }
    }
}

fn status_error(status: u16, headers: &ureq::http::HeaderMap) -> Error {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let retry_after = header("retry-after").and_then(|v| v.trim().parse::<u64>().ok()).map(Duration::from_secs);
    let limit_hit = header("x-ratelimit-remaining").is_some_and(|v| v.trim() == "0");
    if status == 429 || (status == 403 && (limit_hit || retry_after.is_some())) {
        Error::RateLimited { retry_after }
    } else {
        Error::Http(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// Local server: `handle` runs per connection after the request headers were read.
    fn serve(handle: impl Fn(TcpStream) + Send + Sync + 'static) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = Arc::new(handle);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let handle = handle.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                            break;
                        }
                    }
                    handle(stream);
                });
            }
        });
        format!("http://{addr}/file.bin?token=secret")
    }

    fn ok(body: &'static [u8]) -> String {
        serve(move |mut s| {
            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = s.write_all(body);
        })
    }

    /// Headers and a few bytes, then silence for `secs`.
    fn stalling(secs: u64) -> String {
        serve(move |mut s| {
            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n");
            let _ = s.write_all(b"0123456789");
            let _ = s.flush();
            std::thread::sleep(Duration::from_secs(secs));
        })
    }

    fn quick_stall() -> Options<'static> {
        Options { stall_timeout: Duration::from_millis(600), ..Options::default() }
    }

    fn only_files(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn downloads_checks_and_renames() {
        let url = ok(b"hello world");
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("sub/out.bin");
        let mut seen = Vec::new();
        let mut progress = |done: u64, total: Option<u64>| seen.push((done, total));
        let opts = Options { hash: Some(Hash::Sha256), on_progress: Some(&mut progress), ..Options::default() };
        let out = download(&url, &dest, opts, &|| false).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"hello world");
        assert_eq!(out.bytes, 11);
        assert_eq!(out.head, b"hello world");
        assert_eq!(out.hash.as_deref(), Some("b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"));
        assert_eq!(seen.first(), Some(&(0, Some(11))));
        assert_eq!(seen.last(), Some(&(11, Some(11))));
        assert_eq!(only_files(&dir.path().join("sub")), ["out.bin"]);
    }

    #[test]
    fn a_stalled_body_fails_quickly_and_leaves_nothing() {
        let url = stalling(4);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        let t = Instant::now();
        let err = download(&url, &dest, quick_stall(), &|| false).unwrap_err();
        assert!(matches!(err, Error::Stalled(_)), "{err}");
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
        assert!(only_files(dir.path()).is_empty());
    }

    #[test]
    fn cancel_during_a_stall_returns_within_a_second() {
        let url = stalling(4);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            flag.store(true, Ordering::Relaxed);
        });
        let opts = Options { stall_timeout: Duration::from_secs(30), ..Options::default() };
        let t = Instant::now();
        let err = download(&url, &dest, opts, &|| cancel.load(Ordering::Relaxed)).unwrap_err();
        assert!(matches!(err, Error::Cancelled), "{err}");
        assert!(t.elapsed() < Duration::from_millis(1400), "{:?}", t.elapsed());
        assert!(only_files(dir.path()).is_empty());
    }

    #[test]
    fn cancel_while_waiting_for_the_headers_returns_within_a_second() {
        let url = serve(|_s| std::thread::sleep(Duration::from_secs(3)));
        let dir = tempfile::tempdir().unwrap();
        let start = Instant::now();
        let err = download(&url, &dir.path().join("f"), Options::default(), &|| start.elapsed() > Duration::from_millis(300)).unwrap_err();
        assert!(matches!(err, Error::Cancelled), "{err}");
        assert!(start.elapsed() < Duration::from_millis(1300), "{:?}", start.elapsed());
    }

    #[test]
    fn a_429_is_rate_limited_with_its_retry_after() {
        let url = serve(|mut s| {
            let _ = write!(s, "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 7\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        });
        let dir = tempfile::tempdir().unwrap();
        let err = download(&url, &dir.path().join("f"), Options::default(), &|| false).unwrap_err();
        assert!(matches!(err, Error::RateLimited { retry_after: Some(d) } if d == Duration::from_secs(7)), "{err}");
    }

    #[test]
    fn a_403_is_rate_limited_only_when_the_server_says_so() {
        let limited = serve(|mut s| {
            let _ = write!(s, "HTTP/1.1 403 Forbidden\r\nX-RateLimit-Remaining: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        });
        let plain = serve(|mut s| {
            let _ = write!(s, "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        });
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f");
        assert!(matches!(download(&limited, &dest, Options::default(), &|| false), Err(Error::RateLimited { retry_after: None })));
        assert!(matches!(download(&plain, &dest, Options::default(), &|| false), Err(Error::Http(403))));
    }

    #[test]
    fn a_cut_connection_leaves_no_file_and_keeps_the_old_one() {
        let url = serve(|mut s| {
            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n");
            let _ = s.write_all(b"only ten b");
        });
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        fs::write(&dest, "previous").unwrap();
        let err = download(&url, &dest, Options::default(), &|| false).unwrap_err();
        assert!(!matches!(err, Error::Cancelled), "{err}");
        assert_eq!(fs::read_to_string(&dest).unwrap(), "previous");
        assert_eq!(only_files(dir.path()), ["out.bin"]);
    }

    #[test]
    fn a_failed_validation_leaves_no_file() {
        let url = ok(b"<html>please sign in</html>");
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("setup.exe");
        let is_exe = |o: &Outcome| if o.head.starts_with(b"MZ") { Ok(()) } else { Err("not a Windows program".to_string()) };
        let err = download(&url, &dest, Options { validate: Some(&is_exe), ..Options::default() }, &|| false).unwrap_err();
        assert!(matches!(&err, Error::Invalid(m) if m == "not a Windows program"), "{err}");
        assert!(only_files(dir.path()).is_empty());
    }

    #[test]
    fn size_checksum_and_limit_mismatches_are_invalid() {
        let url = ok(b"hello world");
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f");
        let bad = |opts: Options| download(&url, &dest, opts, &|| false).unwrap_err();
        assert!(matches!(bad(Options { expected_size: Some(5), ..Options::default() }), Error::Invalid(_)));
        assert!(matches!(bad(Options { expected_sha256: Some("00"), ..Options::default() }), Error::Invalid(_)));
        assert!(matches!(bad(Options { max_bytes: Some(5), ..Options::default() }), Error::Invalid(_)));
        let good = "B94D27B9934D3E08A52E52D7DA7DABFAC484EFE37A5380EE9088F7ACE2EFCDE9";
        download(&url, &dest, Options { expected_sha256: Some(good), expected_size: Some(11), ..Options::default() }, &|| false).unwrap();
        assert!(dest.exists());
    }

    #[test]
    fn errors_never_show_the_query_string() {
        let dir = tempfile::tempdir().unwrap();
        // Nothing listens on the port the listener just released.
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let url = format!("http://127.0.0.1:{port}/file.bin?token=secret");
        let err = download(&url, &dir.path().join("f"), Options::default(), &|| false).unwrap_err();
        assert!(matches!(err, Error::Unreachable { .. }), "{err}");
        assert!(!err.to_string().contains("secret"), "{err}");
        assert!(err.to_string().contains("/file.bin"), "{err}");
        assert_eq!(redact_url("https://h.example/a/b?sig=1#frag"), "https://h.example/a/b");
        assert_eq!(scrub("GET https://h/x?k=v failed"), "GET https://h/x failed");
    }

    #[test]
    fn a_cancel_before_the_start_does_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let err = download("http://127.0.0.1:1/x", &dir.path().join("f"), Options::default(), &|| true).unwrap_err();
        assert!(matches!(err, Error::Cancelled));
        assert!(only_files(dir.path()).is_empty());
    }
}
