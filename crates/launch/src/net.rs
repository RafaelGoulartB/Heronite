//! The little network and archive access the pre-launch preparation needs
//! (tool releases, runtimes), behind a trait so tests never touch the
//! network.

use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use heronite_i18n::tr;
use serde_json::Value;

/// Cancellation check polled while downloading or extracting.
pub type Cancel<'a> = &'a dyn Fn() -> bool;

pub trait Net: Send + Sync {
    /// Heroic's `isOnline()`.
    fn is_online(&self) -> bool;
    /// GET `url` and parse the body as JSON.
    fn get_json(&self, url: &str) -> Result<Value, String>;
    /// Streams `url` into `dest` (replaced only once the whole body arrived).
    /// `Err("cancelled")` when `cancel` fires; a connection that stalls fails.
    fn download(&self, url: &str, dest: &Path, cancel: Cancel) -> Result<(), String>;
}

pub const CANCELLED: &str = "cancelled";

/// Real network through `ureq`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemNet;

fn agent() -> &'static ureq::Agent {
    use std::sync::OnceLock;
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(15)))
            .max_redirects(8)
            .user_agent("heronite-rs")
            .build()
            .into()
    })
}

fn http_error(e: ureq::Error) -> String {
    match e {
        ureq::Error::StatusCode(403) | ureq::Error::StatusCode(429) => tr!("rate limit reached, try again later").to_string(),
        ureq::Error::StatusCode(c) => format!("HTTP {c}"),
        e => e.to_string(),
    }
}

/// Hosts probed by the connectivity check (IP literals: no DNS to hang on).
const PROBES: [&str; 3] = ["1.1.1.1:443", "9.9.9.9:443", "8.8.8.8:443"];

impl Net for SystemNet {
    fn is_online(&self) -> bool {
        static LAST: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
        if let Some((at, online)) = *LAST.lock().unwrap() {
            if at.elapsed() < Duration::from_secs(20) {
                return online;
            }
        }
        let online = PROBES.iter().any(|addr| {
            addr.parse::<SocketAddr>().ok().map(|a| TcpStream::connect_timeout(&a, Duration::from_secs(2)).is_ok()).unwrap_or(false)
        });
        *LAST.lock().unwrap() = Some((Instant::now(), online));
        online
    }

    fn get_json(&self, url: &str) -> Result<Value, String> {
        let mut resp = agent()
            .get(url)
            .header("Accept", "application/vnd.github+json, application/json, */*")
            .config()
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .call()
            .map_err(http_error)?;
        let text = resp.body_mut().with_config().limit(32 * 1024 * 1024).read_to_string().map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| format!("invalid JSON from {url}: {e}"))
    }

    fn download(&self, url: &str, dest: &Path, cancel: Cancel) -> Result<(), String> {
        heronite_net::download(url, dest, heronite_net::Options::default(), cancel).map(|_| ()).map_err(|e| match e {
            heronite_net::Error::Cancelled => CANCELLED.to_string(),
            e => e.to_string(),
        })
    }
}

/// Extracts `archive` (any format the system `tar` knows) into `dest`,
/// dropping `strip` leading path components. Killed when `cancel` fires.
pub fn extract_tar(archive: &Path, dest: &Path, strip: u32, cancel: Cancel) -> Result<(), String> {
    use heronite_data::proc::Run;
    use std::process::Command;
    fs::create_dir_all(dest).map_err(|e| format!("creating {}: {e}", dest.display()))?;
    let mut cmd = Command::new("tar");
    cmd.arg("-xf").arg(archive).arg("-C").arg(dest).arg(format!("--strip-components={strip}"));
    // A guarded helper: told to stop if the app dies, whole group killed on cancel.
    let out = Run::new().poll(cancel).grace(Duration::from_secs(3)).run(&mut cmd).map_err(|e| format!("could not run tar: {e}"))?;
    if out.aborted {
        Err(CANCELLED.into())
    } else if out.success() {
        Ok(())
    } else {
        Err(format!("tar failed: {}", out.stderr_text().trim()))
    }
}

/// Test double: serves canned JSON and files, records what was asked.
#[derive(Default)]
pub struct FakeNet {
    pub online: bool,
    state: Mutex<FakeNetState>,
}

#[derive(Default)]
struct FakeNetState {
    json: Vec<(String, Value)>,
    files: Vec<(String, Vec<u8>)>,
    requests: Vec<String>,
}

impl FakeNet {
    pub fn online() -> Self {
        Self { online: true, ..Default::default() }
    }
    pub fn offline() -> Self {
        Self::default()
    }
    pub fn with_json(self, url: &str, v: Value) -> Self {
        self.state.lock().unwrap().json.push((url.into(), v));
        self
    }
    pub fn with_file(self, url: &str, bytes: Vec<u8>) -> Self {
        self.state.lock().unwrap().files.push((url.into(), bytes));
        self
    }
    /// URLs requested so far, in order.
    pub fn requests(&self) -> Vec<String> {
        self.state.lock().unwrap().requests.clone()
    }
}

impl Net for FakeNet {
    fn is_online(&self) -> bool {
        self.online
    }
    fn get_json(&self, url: &str) -> Result<Value, String> {
        let mut s = self.state.lock().unwrap();
        s.requests.push(url.to_string());
        s.json.iter().find(|(u, _)| u == url).map(|(_, v)| v.clone()).ok_or_else(|| format!("HTTP 404 for {url}"))
    }
    fn download(&self, url: &str, dest: &Path, cancel: Cancel) -> Result<(), String> {
        let bytes = {
            let mut s = self.state.lock().unwrap();
            s.requests.push(url.to_string());
            s.files.iter().find(|(u, _)| u == url).map(|(_, b)| b.clone())
        };
        if cancel() {
            return Err(CANCELLED.into());
        }
        let bytes = bytes.ok_or_else(|| format!("HTTP 404 for {url}"))?;
        if let Some(dir) = dest.parent() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        fs::write(dest, bytes).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    /// One-shot HTTP server on a random port answering every request with `body`.
    fn serve(body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let body = body.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    loop {
                        let mut h = String::new();
                        if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                            break;
                        }
                    }
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    let _ = stream.write_all(&body);
                });
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn downloads_a_file_and_parses_json() {
        let base = serve(br#"{"a": [1, 2]}"#.to_vec());
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("sub/file.bin");
        SystemNet.download(&format!("{base}/x"), &dest, &|| false).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), br#"{"a": [1, 2]}"#);
        let v = SystemNet.get_json(&format!("{base}/x")).unwrap();
        assert_eq!(v["a"][1], 2);
    }

    #[test]
    fn a_download_can_be_cancelled() {
        let base = serve(vec![0u8; 1024]);
        let dir = tempfile::tempdir().unwrap();
        let err = SystemNet.download(&format!("{base}/x"), &dir.path().join("f"), &|| true).unwrap_err();
        assert_eq!(err, CANCELLED);
    }

    #[test]
    fn extract_strips_leading_components() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src/top/x64");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.dll"), "a").unwrap();
        let archive = dir.path().join("t.tar.gz");
        let ok = std::process::Command::new("tar")
            .args(["-czf"])
            .arg(&archive)
            .arg("-C")
            .arg(dir.path().join("src"))
            .arg("top")
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let out = dir.path().join("out");
        extract_tar(&archive, &out, 1, &|| false).unwrap();
        assert!(out.join("x64/a.dll").exists());
        assert!(extract_tar(&archive, &out, 1, &|| true).is_err());
        assert!(extract_tar(&dir.path().join("missing.tar"), &out, 1, &|| false).is_err());
    }
}
