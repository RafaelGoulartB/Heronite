//! A tiny HTTP server for tests: routes by closure, records requests.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use heronite_metadata::Endpoints;
use parking_lot::Mutex;

#[derive(Debug, Clone)]
pub struct Req {
    pub method: String,
    /// Path and query.
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Req {
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
    pub fn query(&self, key: &str) -> Option<String> {
        let q = self.target.split_once('?')?.1;
        url::form_urlencoded::parse(q.as_bytes()).find(|(k, _)| k == key).map(|(_, v)| v.into_owned())
    }
}

pub struct Resp {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn json(v: impl serde::Serialize) -> Self {
        Self { status: 200, content_type: "application/json", body: serde_json::to_vec(&v).unwrap() }
    }
    pub fn raw_json(s: String) -> Self {
        Self { status: 200, content_type: "application/json", body: s.into_bytes() }
    }
    pub fn html(s: &str) -> Self {
        Self { status: 200, content_type: "text/html", body: s.as_bytes().to_vec() }
    }
    pub fn image(bytes: Vec<u8>) -> Self {
        Self { status: 200, content_type: "image/png", body: bytes }
    }
    pub fn status(code: u16) -> Self {
        Self { status: code, content_type: "text/plain", body: b"nope".to_vec() }
    }
}

pub struct Server {
    pub base: String,
    pub log: Arc<Mutex<Vec<Req>>>,
    stop: Arc<AtomicBool>,
}

impl Server {
    pub fn start(handler: impl Fn(&Req) -> Resp + Send + Sync + 'static) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let handler = Arc::new(handler);
        let (log2, stop2) = (log.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop2.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (h, l) = (handler.clone(), log2.clone());
                        std::thread::spawn(move || {
                            let _ = serve(stream, &*h, &l);
                        });
                    }
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(2)),
                }
            }
        });
        Server { base: format!("http://127.0.0.1:{port}"), log, stop }
    }

    pub fn count(&self, pred: impl Fn(&Req) -> bool) -> usize {
        self.log.lock().iter().filter(|r| pred(r)).count()
    }

    /// Endpoints with every service under its own prefix of this server.
    pub fn endpoints(&self) -> Endpoints {
        let b = &self.base;
        Endpoints {
            twitch_token: format!("{b}/twitch/oauth2/token"),
            igdb: format!("{b}/igdb/v4"),
            steam_store: format!("{b}/steam"),
            steam_community: format!("{b}/community"),
            lutris: format!("{b}/lutris"),
            steamgriddb: format!("{b}/sgdb/api/v2"),
            bottles_steamgrid: format!("{b}/bottles"),
            hltb: format!("{b}/hltb"),
            protondb: format!("{b}/protondb"),
            duckduckgo: format!("{b}/ddg"),
            bing: format!("{b}/bing"),
            google: format!("{b}/google"),
            steam_hero_urls: Some(vec![format!("{b}/cdn/missing_hero.jpg"), format!("{b}/cdn/hero.jpg")]),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn serve(mut stream: TcpStream, handler: &dyn Fn(&Req) -> Resp, log: &Mutex<Vec<Req>>) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.lines();
    let mut first = lines.next().unwrap_or("").split_whitespace();
    let (method, target) = (first.next().unwrap_or("").to_owned(), first.next().unwrap_or("").to_owned());
    let headers: Vec<(String, String)> =
        lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))).collect();
    let len: usize = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
    while buf.len() < head_end + len {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let body = String::from_utf8_lossy(&buf[head_end..]).into_owned();
    let req = Req { method, target, headers, body };
    log.lock().push(req.clone());
    let resp = handler(&req);
    let head = format!(
        "HTTP/1.1 {} X\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        resp.status,
        resp.content_type,
        resp.body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(&resp.body)?;
    stream.flush()
}

pub fn fixture(name: &str) -> String {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).unwrap()
}

pub fn png() -> Vec<u8> {
    let mut v = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    v.extend(std::iter::repeat_n(7u8, 9000));
    v
}
