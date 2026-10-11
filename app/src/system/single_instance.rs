//! One running instance per data folder (`app.requestSingleInstanceLock()` in
//! Heroic): a unix socket in `$XDG_RUNTIME_DIR`, named after the data folder
//! so an instance on a fixture folder never talks to the user's.
//!
//! A second process connects, sends one line of JSON ([`Message`]) and exits
//! when the first answers `ok` (only sent once the request was really handed
//! to the running instance). If nobody listens (no socket, or a stale file
//! from a crash) the process becomes the instance and binds the socket. If
//! somebody listens but does not answer in time, or answers an error, it is
//! [`Forward::Unresponsive`]: the caller must NOT start a second instance.
//! At quit the first instance calls [`close_active`] before anything slow, so
//! a late second process finds no socket instead of a dying listener.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use super::cli::LaunchRequest;

/// What a second invocation asks the running instance to do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Message {
    pub launch: Option<LaunchRequest>,
    /// `--no-gui` on the second invocation.
    pub no_gui: bool,
}

impl Message {
    pub fn to_line(&self) -> String {
        let launch = self.launch.as_ref().map(|r| {
            json!({
                "runner": r.runner,
                "appName": r.app_name,
                "args": r.args,
                "altExe": r.alt_exe,
                "guiFalse": r.gui_false,
            })
        });
        json!({ "v": 1, "launch": launch, "noGui": self.no_gui }).to_string()
    }

    pub fn from_line(line: &str) -> Option<Message> {
        let v: Value = serde_json::from_str(line).ok()?;
        let launch = match v.get("launch") {
            Some(Value::Object(o)) => Some(LaunchRequest {
                runner: o.get("runner").and_then(Value::as_str).map(str::to_string),
                app_name: o.get("appName").and_then(Value::as_str)?.to_string(),
                args: o
                    .get("args")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                    .unwrap_or_default(),
                alt_exe: o.get("altExe").and_then(Value::as_str).map(str::to_string),
                gui_false: o.get("guiFalse").and_then(Value::as_bool).unwrap_or(false),
            }),
            _ => None,
        };
        Some(Message { launch, no_gui: v.get("noGui").and_then(Value::as_bool).unwrap_or(false) })
    }
}

/// FNV-1a 64 of the data folder path: short and stable across runs.
fn digest(root: &Path) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in root.to_string_lossy().as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Socket path for a data folder. `runtime_dir` is `$XDG_RUNTIME_DIR`.
pub fn socket_path_in(runtime_dir: &Path, data_root: &Path) -> PathBuf {
    runtime_dir.join(format!("heronite-rs-{}.sock", digest(data_root)))
}

/// Socket path for the current user and data folder.
pub fn socket_path(data_root: &Path) -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        // SAFETY: getuid has no preconditions.
        .unwrap_or_else(|| std::env::temp_dir().join(format!("heronite-rs-{}", unsafe { libc::getuid() })));
    let _ = std::fs::create_dir_all(&dir);
    socket_path_in(&dir, data_root)
}

/// Result of trying to reach a running instance.
#[derive(Debug, PartialEq, Eq)]
pub enum Forward {
    /// The running instance took the message.
    Delivered,
    /// Nobody is listening (a stale socket file was removed).
    NoInstance,
    /// Somebody is listening but did not take the message: no answer in time,
    /// the connection broke, or it answered an error. The reason is for the
    /// user.
    Unresponsive(String),
}

/// How long a second invocation waits for the running instance.
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    pub write: Duration,
    pub answer: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self { write: Duration::from_secs(3), answer: Duration::from_secs(5) }
    }
}

/// Sends `msg` to the instance listening on `socket`.
pub fn forward(socket: &Path, msg: &Message) -> std::io::Result<Forward> {
    forward_with(socket, msg, Timeouts::default())
}

pub fn forward_with(socket: &Path, msg: &Message, timeouts: Timeouts) -> std::io::Result<Forward> {
    let mut stream = match UnixStream::connect(socket) {
        Ok(s) => s,
        Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound) => return Ok(Forward::NoInstance),
        Err(e) if matches!(e.kind(), std::io::ErrorKind::ConnectionRefused) => {
            // Stale file of a crashed instance.
            let _ = std::fs::remove_file(socket);
            return Ok(Forward::NoInstance);
        }
        Err(e) => return Err(e),
    };
    // From here on somebody is there: failures are `Unresponsive`, never a
    // reason to start a second instance.
    let unresponsive = |what: &str, e: std::io::Error| {
        let reason = match e.kind() {
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => format!("{what}: no answer in time"),
            _ => format!("{what}: {e}"),
        };
        Ok(Forward::Unresponsive(reason))
    };
    if let Err(e) = stream.set_write_timeout(Some(timeouts.write)).and_then(|_| stream.set_read_timeout(Some(timeouts.answer))) {
        return unresponsive("configuring the connection", e);
    }
    if let Err(e) = stream.write_all(msg.to_line().as_bytes()).and_then(|_| stream.write_all(b"\n")) {
        return unresponsive("sending the request", e);
    }
    let mut ack = String::new();
    match BufReader::new(&stream).read_line(&mut ack) {
        Err(e) => unresponsive("waiting for the answer", e),
        Ok(0) => Ok(Forward::Unresponsive("the running instance closed the connection without answering".into())),
        Ok(_) if ack.trim() == "ok" => Ok(Forward::Delivered),
        Ok(_) => Ok(Forward::Unresponsive(format!("unexpected answer {:?}", ack.trim()))),
    }
}

/// The listening side. Dropping it removes the socket file (unless
/// [`close_active`] already did).
pub struct Server {
    path: PathBuf,
    closed: Arc<AtomicBool>,
}

/// The socket this process listens on, for [`close_active`].
static ACTIVE: Mutex<Option<(PathBuf, Arc<AtomicBool>)>> = Mutex::new(None);

/// Stops accepting requests and removes the socket file now (the quit has
/// started: a second invocation must find no socket, not a dying listener).
/// Safe to call without a server and more than once.
pub fn close_active() {
    let active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some((path, closed)) = active {
        close_socket(&path, &closed);
    }
}

fn close_socket(path: &Path, closed: &AtomicBool) {
    if !closed.swap(true, Ordering::SeqCst) {
        let _ = std::fs::remove_file(path);
    }
}

impl Server {
    /// Same as [`close_active`] for this server.
    pub fn close(&self) {
        close_socket(&self.path, &self.closed);
    }

    /// Binds `socket` and calls `on_message` (from a worker thread) for every
    /// message received; it returns whether the message was delivered, which
    /// decides the answer (`ok` or `err`). Fails with `AddrInUse` when
    /// another live instance owns the socket.
    pub fn bind(socket: &Path, on_message: impl Fn(Message) -> bool + Send + 'static) -> std::io::Result<Server> {
        let listener = match UnixListener::bind(socket) {
            Ok(l) => l,
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                // Alive? Then it is not ours to take.
                match UnixStream::connect(socket) {
                    Ok(_) => return Err(e),
                    Err(_) => {
                        std::fs::remove_file(socket)?;
                        UnixListener::bind(socket)?
                    }
                }
            }
            Err(e) => return Err(e),
        };
        let _ = std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600));
        let closed = Arc::new(AtomicBool::new(false));
        let flag = closed.clone();
        std::thread::Builder::new().name("single-instance".into()).spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut line = String::new();
                if BufReader::new(&stream).read_line(&mut line).is_err() {
                    continue;
                }
                let answer = if flag.load(Ordering::SeqCst) {
                    "closing\n"
                } else {
                    match Message::from_line(line.trim()) {
                        Some(msg) => {
                            if on_message(msg) {
                                "ok\n"
                            } else {
                                "err\n"
                            }
                        }
                        None => "bad\n",
                    }
                };
                let mut stream = stream;
                let _ = stream.write_all(answer.as_bytes());
            }
        })?;
        *ACTIVE.lock().unwrap_or_else(|e| e.into_inner()) = Some((socket.to_path_buf(), closed.clone()));
        Ok(Server { path: socket.to_path_buf(), closed })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // After `close_active` the path may belong to a newer instance.
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn msg() -> Message {
        Message {
            launch: Some(LaunchRequest {
                runner: Some("legendary".into()),
                app_name: "Quail".into(),
                args: vec!["--x".into(), "a b".into()],
                alt_exe: Some("/g/x.exe".into()),
                gui_false: true,
            }),
            no_gui: true,
        }
    }

    #[test]
    fn message_round_trips_through_a_line() {
        let m = msg();
        assert_eq!(Message::from_line(&m.to_line()), Some(m));
        let plain = Message::default();
        assert_eq!(Message::from_line(&plain.to_line()), Some(plain));
        assert_eq!(Message::from_line("garbage"), None);
        assert!(!msg().to_line().contains('\n'));
    }

    #[test]
    fn socket_name_depends_on_the_data_folder() {
        let rt = Path::new("/run/user/1000");
        let a = socket_path_in(rt, Path::new("/home/u/.local/share/heronite-rs"));
        let b = socket_path_in(rt, Path::new("/tmp/heronite-fixture-system"));
        assert_ne!(a, b);
        assert_eq!(a, socket_path_in(rt, Path::new("/home/u/.local/share/heronite-rs")));
        assert!(a.to_string_lossy().starts_with("/run/user/1000/heronite-rs-"));
    }

    #[test]
    fn forward_without_an_instance() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("s.sock");
        assert_eq!(forward(&sock, &msg()).unwrap(), Forward::NoInstance);
    }

    #[test]
    fn server_receives_forwarded_messages_and_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("s.sock");
        let (tx, rx) = mpsc::channel();
        let server = Server::bind(&sock, move |m| tx.send(m).is_ok()).unwrap();
        // A second server on the same socket is refused while the first lives.
        let err = Server::bind(&sock, |_| true).err().expect("in use");
        assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);

        assert_eq!(forward(&sock, &msg()).unwrap(), Forward::Delivered);
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), msg());
        assert_eq!(forward(&sock, &Message::default()).unwrap(), Forward::Delivered);
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), Message::default());

        drop(server);
        assert!(!sock.exists());
    }

    #[test]
    fn stale_socket_file_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("s.sock");
        // A socket file nobody listens on (a crashed instance).
        drop(UnixListener::bind(&sock).unwrap());
        assert!(sock.exists());
        assert_eq!(forward(&sock, &msg()).unwrap(), Forward::NoInstance);
        assert!(!sock.exists(), "stale file removed");

        drop(UnixListener::bind(&sock).unwrap());
        let (tx, rx) = mpsc::channel();
        let _server = Server::bind(&sock, move |m| tx.send(m).is_ok()).unwrap();
        assert_eq!(forward(&sock, &msg()).unwrap(), Forward::Delivered);
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), msg());
    }

    fn quick() -> Timeouts {
        Timeouts { write: Duration::from_millis(500), answer: Duration::from_millis(300) }
    }

    #[test]
    fn a_listener_that_never_answers_is_unresponsive_not_absent() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("s.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let t = std::time::Instant::now();
        let got = forward_with(&sock, &msg(), quick()).unwrap();
        assert!(matches!(&got, Forward::Unresponsive(r) if r.contains("no answer in time")), "{got:?}");
        assert!(t.elapsed() < Duration::from_secs(3));
        // The socket is the live instance's: it must not be removed.
        assert!(sock.exists());
        drop(listener);
    }

    #[test]
    fn a_listener_that_hangs_up_or_answers_an_error_is_unresponsive() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("s.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        // Closes the connection without a word.
        let hangup = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            drop(stream);
            listener
        });
        let got = forward_with(&sock, &msg(), quick()).unwrap();
        assert!(matches!(got, Forward::Unresponsive(_)), "{got:?}");
        let listener = hangup.join().unwrap();

        // Answers something that is not `ok`.
        let angry = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            stream.write_all(b"nope\n").unwrap();
        });
        let got = forward_with(&sock, &msg(), quick()).unwrap();
        assert!(matches!(&got, Forward::Unresponsive(r) if r.contains("nope")), "{got:?}");
        angry.join().unwrap();
    }

    #[test]
    fn ok_is_only_answered_when_the_message_was_delivered() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("s.sock");
        // The receiving side is gone (the app is shutting down).
        let _server = Server::bind(&sock, |_| false).unwrap();
        let got = forward_with(&sock, &msg(), quick()).unwrap();
        assert!(matches!(&got, Forward::Unresponsive(r) if r.contains("err")), "{got:?}");
    }

    #[test]
    fn a_closed_server_has_no_socket_and_never_removes_a_newer_one() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("s.sock");
        let (tx, rx) = mpsc::channel();
        let server = Server::bind(&sock, move |m| tx.send(m).is_ok()).unwrap();
        server.close();
        assert!(!sock.exists());
        assert_eq!(forward_with(&sock, &msg(), quick()).unwrap(), Forward::NoInstance);
        assert!(rx.try_recv().is_err());
        // A newer instance took the path: dropping the old server leaves it alone.
        let newer = UnixListener::bind(&sock).unwrap();
        drop(server);
        assert!(sock.exists());
        drop(newer);
    }

    #[test]
    fn a_request_that_races_the_close_is_told_so() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("s.sock");
        let (tx, rx) = mpsc::channel();
        let server = Server::bind(&sock, move |m| tx.send(m).is_ok()).unwrap();
        // Connected before the close, handled after it.
        let mut stream = UnixStream::connect(&sock).unwrap();
        server.close();
        stream.write_all(msg().to_line().as_bytes()).unwrap();
        stream.write_all(b"\n").unwrap();
        let mut ack = String::new();
        BufReader::new(&stream).read_line(&mut ack).unwrap();
        assert_eq!(ack.trim(), "closing");
        assert!(rx.try_recv().is_err(), "nothing was delivered");
    }
}
