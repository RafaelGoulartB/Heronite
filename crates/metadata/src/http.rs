//! Blocking HTTP used by every provider (`ureq` 3 + rustls). Callers run on
//! worker threads; nothing here spawns or needs an async runtime.
//!
//! All base URLs live in [`Endpoints`] so tests can point them at a local mock
//! server.

use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::de::DeserializeOwned;

pub const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/123.0.0.0 Safari/537.36";
pub const FIREFOX_UA: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0";
/// The app by name and version, for APIs that ask clients to say who they
/// are (every request of an extension carries it), as the GOG and Sunshine
/// clients do. Nothing about the user.
pub const APP_UA: &str = concat!("Heronite/", env!("CARGO_PKG_VERSION"));

/// Base URLs of every remote service.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub twitch_token: String,
    pub igdb: String,
    pub steam_store: String,
    /// The Steam Web API (keyless store services: tags).
    pub steam_api: String,
    pub steam_community: String,
    pub lutris: String,
    /// The LaunchBox Games Database (its archive, `/Metadata.zip`).
    pub launchbox: String,
    /// Where LaunchBox's pictures are, by file name.
    pub launchbox_images: String,
    /// libretro-thumbnails (`/<system>/Named_Boxarts/<game>.png`).
    pub libretro_thumbnails: String,
    pub steamgriddb: String,
    pub bottles_steamgrid: String,
    pub hltb: String,
    pub protondb: String,
    pub duckduckgo: String,
    pub bing: String,
    pub google: String,
    /// Replaces the five Steam hero CDN URLs when set (tests).
    pub steam_hero_urls: Option<Vec<String>>,
    /// Origins served elsewhere (tests): `("https://api.example.org",
    /// "http://127.0.0.1:4000/example")`. Requests to an address under an
    /// origin go to the replacement; [`Http::route`] applies them. Used for
    /// the addresses extensions build themselves.
    pub origins: Vec<(String, String)>,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            twitch_token: "https://id.twitch.tv/oauth2/token".into(),
            igdb: "https://api.igdb.com/v4".into(),
            steam_store: "https://store.steampowered.com".into(),
            steam_api: "https://api.steampowered.com".into(),
            steam_community: "https://steamcommunity.com".into(),
            lutris: "https://lutris.net".into(),
            launchbox: "https://gamesdb.launchbox-app.com".into(),
            launchbox_images: "https://images.launchbox-app.com".into(),
            libretro_thumbnails: "https://thumbnails.libretro.com".into(),
            steamgriddb: "https://www.steamgriddb.com/api/v2".into(),
            bottles_steamgrid: "https://steamgrid.usebottles.com".into(),
            hltb: "https://howlongtobeat.com".into(),
            protondb: "https://www.protondb.com".into(),
            duckduckgo: "https://duckduckgo.com".into(),
            bing: "https://www.bing.com".into(),
            google: "https://www.google.com".into(),
            steam_hero_urls: None,
            origins: Vec::new(),
        }
    }
}

/// A request that never got an answer (no network, DNS, timeout, dropped
/// connection), as opposed to a server that replied with an error status.
#[derive(Debug)]
pub struct Unreachable(String);

impl std::fmt::Display for Unreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for Unreachable {}

/// A body over the request's [`Request::max_bytes`]. Counts as
/// [`Unreachable`] for [`is_unreachable`], as it always did; the extension
/// runtime tells it apart.
#[derive(Debug)]
pub struct BodyTooLarge(pub u64);

impl std::fmt::Display for BodyTooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the answer is larger than {} bytes", self.0)
    }
}

impl std::error::Error for BodyTooLarge {}

/// Whether `err` came from a request that got no answer at all.
pub fn is_unreachable(err: &anyhow::Error) -> bool {
    err.downcast_ref::<Unreachable>().is_some() || err.downcast_ref::<BodyTooLarge>().is_some()
}

/// A finished response (status errors are not turned into `Err`).
#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub content_length: Option<u64>,
    /// `Retry-After` in seconds, when the server sent one.
    pub retry_after: Option<u64>,
    /// Every header, names lowercase, in the order received.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn json<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_slice(&self.body).map_err(|e| anyhow!("invalid JSON (status {}): {e}", self.status))
    }

    /// Error carrying the status and the start of the body, like an axios error.
    pub fn error(&self) -> anyhow::Error {
        let body = self.text();
        let body: String = body.chars().take(300).collect();
        anyhow!("HTTP {}: {}", self.status, body)
    }
}

#[derive(Clone)]
pub struct Http {
    agent: ureq::Agent,
    pub endpoints: Endpoints,
}

/// One request. Build with [`Http::get`] / [`Http::post`].
pub struct Request<'a> {
    http: &'a Http,
    post: bool,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
    timeout: Duration,
    max_bytes: u64,
    redirects: bool,
}

impl<'a> Request<'a> {
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub fn query(mut self, params: &[(&str, &str)]) -> Self {
        if let Ok(mut u) = url::Url::parse(&self.url) {
            u.query_pairs_mut().extend_pairs(params.iter().copied());
            self.url = u.into();
        }
        self
    }

    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn timeout_secs(mut self, secs: u64) -> Self {
        self.timeout = Duration::from_secs(secs);
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn max_bytes(mut self, max: u64) -> Self {
        self.max_bytes = max;
        self
    }

    /// Returns a redirect as it is (status 3xx, `location` header) instead
    /// of following it.
    pub fn no_redirects(mut self) -> Self {
        self.redirects = false;
        self
    }

    pub fn send(self) -> Result<Response> {
        let max = self.max_bytes;
        self.send_reading(|url, body| {
            body.with_config().limit(max).read_to_vec().map_err(|e| match e {
                ureq::Error::BodyExceedsLimit(max) => anyhow::Error::new(BodyTooLarge(max)),
                e => anyhow::Error::new(Unreachable(format!("{url}: {e}"))),
            })
        })
    }

    /// [`send`](Self::send) that reads only the start of the body: up to
    /// `max` bytes, and no further once `enough` says the bytes read are
    /// (checked after each chunk). For a server that ignores `Range` (the
    /// rest of the answer is dropped with the connection). Never fails for
    /// a long body.
    pub fn send_head(self, max: usize, enough: impl Fn(&[u8]) -> bool) -> Result<Response> {
        self.send_reading(|url, body| {
            let mut reader = body.as_reader();
            let mut out: Vec<u8> = Vec::new();
            let mut chunk = [0u8; 8 * 1024];
            while out.len() < max && !enough(&out) {
                let want = chunk.len().min(max - out.len());
                match reader.read(&mut chunk[..want]) {
                    Ok(0) => break,
                    Ok(n) => out.extend_from_slice(&chunk[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(anyhow::Error::new(Unreachable(format!("{url}: {e}")))),
                }
            }
            Ok(out)
        })
    }

    /// Sends the request and reads its body with `read` (given the address,
    /// for its errors).
    fn send_reading(self, read: impl FnOnce(&str, &mut ureq::Body) -> Result<Vec<u8>>) -> Result<Response> {
        let agent = &self.http.agent;
        let redirects = if self.redirects { MAX_REDIRECTS } else { 0 };
        let mut resp = if self.post {
            let mut req = agent.post(&self.url).config().timeout_global(Some(self.timeout)).max_redirects(redirects).build();
            for (k, v) in &self.headers {
                req = req.header(k, v);
            }
            match &self.body {
                Some(b) => req.send(b.as_str()),
                None => req.send_empty(),
            }
        } else {
            let mut req = agent.get(&self.url).config().timeout_global(Some(self.timeout)).max_redirects(redirects).build();
            for (k, v) in &self.headers {
                req = req.header(k, v);
            }
            req.call()
        }
        .map_err(|e| match heronite_images::public::not_public(&e) {
            Some(n) => anyhow::Error::new(n),
            None => anyhow::Error::new(Unreachable(format!("{}: {e}", self.url))),
        })?;
        let status = resp.status().as_u16();
        let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
        let content_type = header("content-type").unwrap_or_default();
        let content_length = header("content-length").and_then(|v| v.parse().ok());
        let retry_after = header("retry-after").and_then(|v| v.trim().parse().ok());
        let headers =
            resp.headers().iter().filter_map(|(k, v)| Some((k.as_str().to_ascii_lowercase(), v.to_str().ok()?.to_owned()))).collect();
        let body = read(&self.url, resp.body_mut())?;
        Ok(Response { status, content_type, content_length, retry_after, headers, body })
    }

    /// `send` that fails on a non-2xx status.
    pub fn send_ok(self) -> Result<Response> {
        let r = self.send()?;
        if r.ok() {
            Ok(r)
        } else {
            Err(r.error())
        }
    }
}

/// How a download into a file ended ([`Http::download_to`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetched {
    /// The file holds the whole body.
    Complete { etag: Option<String>, last_modified: Option<String> },
    /// The server says the copy named in `If-None-Match` is current.
    NotModified,
    /// `go` asked to stop; the file keeps what came.
    Stopped,
}

/// The longest a download into a file may take.
const DOWNLOAD_LIMIT: Duration = Duration::from_secs(3 * 3600);

impl Http {
    /// GETs `url` into the file at `path`, calling `go` after each chunk with
    /// the bytes in the file and the full size when known; `go` returning
    /// `false` stops it. With `resume` and a file already there, asks only
    /// for the rest (`Range`, guarded by `If-Range: etag` so a changed file
    /// comes whole); a server that sends it whole starts the file over.
    /// `not_modified_if`: an ETag whose copy is current (`If-None-Match`).
    /// `started` gets the body's ETag before the first byte is written.
    /// A request without answer is [`Unreachable`]; the file keeps what came.
    pub fn download_to(
        &self,
        url: &str,
        path: &Path,
        resume: Option<&str>,
        not_modified_if: Option<&str>,
        started: &mut dyn FnMut(Option<&str>),
        go: &mut dyn FnMut(u64, Option<u64>) -> bool,
    ) -> Result<Fetched> {
        let have = resume.and(std::fs::metadata(path).ok()).map_or(0, |m| m.len());
        let mut req = self.agent.get(url).config().timeout_global(Some(DOWNLOAD_LIMIT)).build().header("User-Agent", APP_UA);
        if have > 0 {
            req = req.header("Range", format!("bytes={have}-"));
            if let Some(etag) = resume.filter(|e| !e.is_empty()) {
                req = req.header("If-Range", etag);
            }
        }
        if let Some(etag) = not_modified_if.filter(|e| !e.is_empty()) {
            req = req.header("If-None-Match", etag);
        }
        let unreachable = |e: &dyn std::fmt::Display| anyhow::Error::new(Unreachable(format!("{url}: {e}")));
        let mut resp = req.call().map_err(|e| unreachable(&e))?;
        let status = resp.status().as_u16();
        let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
        let (etag, last_modified) = (header("etag"), header("last-modified"));
        let length: Option<u64> = header("content-length").and_then(|v| v.parse().ok());
        let (append, total) = match status {
            304 => return Ok(Fetched::NotModified),
            // Asked from past the end: the file is whole already.
            416 if have > 0 => return Ok(Fetched::Complete { etag: resume.map(str::to_owned), last_modified }),
            206 if have > 0 => (true, length.map(|l| l + have)),
            200..=299 => (false, length),
            _ => anyhow::bail!("HTTP {status} for {url}"),
        };
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(path)
            .with_context(|| format!("cannot write {}", path.display()))?;
        // A part resumed keeps the ETag it was asked with (`If-Range`).
        started(if append { resume } else { etag.as_deref() });
        let mut written = if append { have } else { 0 };
        let mut reader = resp.body_mut().as_reader();
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            if !go(written, total) {
                file.flush()?;
                return Ok(Fetched::Stopped);
            }
            let n = reader.read(&mut buf).map_err(|e| unreachable(&e))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).with_context(|| format!("cannot write {}", path.display()))?;
            written += n as u64;
        }
        file.sync_all()?;
        if total.is_some_and(|t| t != written) {
            return Err(unreachable(&format!("the download ended at {written} of {} bytes", total.unwrap_or(0))));
        }
        go(written, total);
        Ok(Fetched::Complete { etag, last_modified })
    }
}

const MAX_REDIRECTS: u32 = 8;

fn config() -> ureq::config::ConfigBuilder<ureq::typestate::AgentScope> {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(MAX_REDIRECTS)
        .timeout_connect(Some(Duration::from_secs(10)))
        .user_agent(BROWSER_UA)
}

impl Http {
    pub fn new(endpoints: Endpoints) -> Self {
        Self { agent: config().build().into(), endpoints }
    }

    /// The same client, connecting to public addresses only and never
    /// through a proxy (`heronite_images::public`): for what an extension
    /// asks. A name that leads elsewhere fails with
    /// [`heronite_images::public::NotPublic`].
    pub fn public_only(&self) -> Self {
        Self { agent: heronite_images::public::agent(config()), endpoints: self.endpoints.clone() }
    }

    pub fn get(&self, url: impl Into<String>) -> Request<'_> {
        Request {
            http: self,
            post: false,
            url: url.into(),
            headers: Vec::new(),
            body: None,
            timeout: Duration::from_secs(20),
            max_bytes: 16 * 1024 * 1024,
            redirects: true,
        }
    }

    /// `url` with [`Endpoints::origins`] applied: the address it is really
    /// sent to (itself, outside tests), and whether an origin replaced it.
    pub fn route(&self, url: &str) -> (String, bool) {
        for (origin, to) in &self.endpoints.origins {
            if let Some(rest) = url.strip_prefix(origin.as_str()) {
                if rest.is_empty() || rest.starts_with(['/', '?', '#']) {
                    return (format!("{to}{rest}"), true);
                }
            }
        }
        (url.to_owned(), false)
    }

    pub fn post(&self, url: impl Into<String>) -> Request<'_> {
        Request { post: true, ..self.get(url) }
    }
}

impl Default for Http {
    fn default() -> Self {
        Self::new(Endpoints::default())
    }
}
