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
    /// Headers beyond the content type and length.
    pub headers: Vec<(&'static str, String)>,
}

impl Resp {
    fn new(status: u16, content_type: &'static str, body: Vec<u8>) -> Self {
        Self { status, content_type, body, headers: Vec::new() }
    }
    pub fn json(v: impl serde::Serialize) -> Self {
        Self::new(200, "application/json", serde_json::to_vec(&v).unwrap())
    }
    pub fn raw_json(s: String) -> Self {
        Self::new(200, "application/json", s.into_bytes())
    }
    pub fn html(s: &str) -> Self {
        Self::new(200, "text/html", s.as_bytes().to_vec())
    }
    pub fn image(bytes: Vec<u8>) -> Self {
        Self::new(200, "image/png", bytes)
    }
    pub fn status(code: u16) -> Self {
        Self::new(code, "text/plain", b"nope".to_vec())
    }
    pub fn with_header(mut self, name: &'static str, value: &str) -> Self {
        self.headers.push((name, value.to_owned()));
        self
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
            steam_api: format!("{b}/steamapi"),
            steam_community: format!("{b}/community"),
            lutris: format!("{b}/lutris"),
            launchbox: format!("{b}/launchbox"),
            launchbox_images: format!("{b}/lbimages"),
            libretro_thumbnails: format!("{b}/libretro"),
            steamgriddb: format!("{b}/sgdb/api/v2"),
            bottles_steamgrid: format!("{b}/bottles"),
            hltb: format!("{b}/hltb"),
            protondb: format!("{b}/protondb"),
            duckduckgo: format!("{b}/ddg"),
            bing: format!("{b}/bing"),
            google: format!("{b}/google"),
            steam_hero_urls: Some(vec![format!("{b}/cdn/missing_hero.jpg"), format!("{b}/cdn/hero.jpg")]),
            origins: Vec::new(),
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
    let extra: String = resp.headers.iter().map(|(k, v)| format!("{k}: {v}\r\n")).collect();
    let head = format!(
        "HTTP/1.1 {} X\r\nContent-Type: {}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
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

/// The first bytes of a PNG of `w` x `h` (signature and `IHDR`: all a
/// source reads to learn a picture's shape), padded to `len` bytes.
pub fn png_of(w: u32, h: u32, len: usize) -> Vec<u8> {
    let mut v = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
    v.extend(w.to_be_bytes());
    v.extend(h.to_be_bytes());
    v.extend([8, 6, 0, 0, 0]);
    v.resize(len.max(v.len()), 7);
    v
}

pub fn png() -> Vec<u8> {
    let mut v = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    v.extend(std::iter::repeat_n(7u8, 9000));
    v
}

/// A details source asked as a fallback for visual novels, declared the
/// way an extension declares one, that knows no game: added to a service
/// (`MetadataService::add_source`) where a test needs a fallback source.
/// It counts the lookups and searches it gets.
pub struct Fallback {
    descriptor: heronite_metadata::SourceDescriptor,
    pub lookups: std::sync::atomic::AtomicUsize,
    pub searches: std::sync::atomic::AtomicUsize,
}

pub const FALLBACK: &str = "novels";

pub fn fallback() -> Arc<Fallback> {
    let descriptor = serde_json::from_value(serde_json::json!({
        "id": FALLBACK, "name": "Novels", "kind": "details", "network": true, "searchable": true,
        "provides": ["description", "releaseDate", "developers", "genres", "themes", "criticScore", "cover", "hero"],
        "asked": "asFallback", "gameKind": {"name": "Visual novels", "signals": ["Visual Novel"]},
        "extension": {"version": "1.0.0", "author": "tests"}
    }))
    .expect("a test descriptor");
    Arc::new(Fallback { descriptor, lookups: Default::default(), searches: Default::default() })
}

impl heronite_metadata::MetadataSource for Fallback {
    fn descriptor(&self) -> &heronite_metadata::SourceDescriptor {
        &self.descriptor
    }

    fn lookup(
        &self,
        _: &heronite_metadata::SourceContext<'_>,
        _: &heronite_metadata::SourceGame,
    ) -> heronite_metadata::Lookup<heronite_metadata::MetadataCandidate> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        heronite_metadata::Lookup::NotFound
    }

    fn search(&self, _: &heronite_metadata::SourceContext<'_>, _: &str) -> heronite_metadata::Lookup<Vec<heronite_metadata::SourceHit>> {
        self.searches.fetch_add(1, Ordering::SeqCst);
        heronite_metadata::Lookup::Found(Vec::new())
    }
}
