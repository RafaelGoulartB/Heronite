//! Blocking HTTP used by every provider (`ureq` 3 + rustls). Callers run on
//! worker threads; nothing here spawns or needs an async runtime.
//!
//! All base URLs live in [`Endpoints`] so tests can point them at a local mock
//! server.

use std::time::Duration;

use anyhow::{anyhow, Result};
use serde::de::DeserializeOwned;

pub const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/123.0.0.0 Safari/537.36";
pub const FIREFOX_UA: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0";

/// Base URLs of every remote service.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub twitch_token: String,
    pub igdb: String,
    pub steam_store: String,
    pub steam_community: String,
    pub lutris: String,
    pub steamgriddb: String,
    pub bottles_steamgrid: String,
    pub hltb: String,
    pub protondb: String,
    pub duckduckgo: String,
    pub bing: String,
    pub google: String,
    /// Replaces the five Steam hero CDN URLs when set (tests).
    pub steam_hero_urls: Option<Vec<String>>,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            twitch_token: "https://id.twitch.tv/oauth2/token".into(),
            igdb: "https://api.igdb.com/v4".into(),
            steam_store: "https://store.steampowered.com".into(),
            steam_community: "https://steamcommunity.com".into(),
            lutris: "https://lutris.net".into(),
            steamgriddb: "https://www.steamgriddb.com/api/v2".into(),
            bottles_steamgrid: "https://steamgrid.usebottles.com".into(),
            hltb: "https://howlongtobeat.com".into(),
            protondb: "https://www.protondb.com".into(),
            duckduckgo: "https://duckduckgo.com".into(),
            bing: "https://www.bing.com".into(),
            google: "https://www.google.com".into(),
            steam_hero_urls: None,
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

/// Whether `err` came from a request that got no answer at all.
pub fn is_unreachable(err: &anyhow::Error) -> bool {
    err.downcast_ref::<Unreachable>().is_some()
}

/// A finished response (status errors are not turned into `Err`).
#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub content_length: Option<u64>,
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

    pub fn max_bytes(mut self, max: u64) -> Self {
        self.max_bytes = max;
        self
    }

    pub fn send(self) -> Result<Response> {
        let agent = &self.http.agent;
        let mut resp = if self.post {
            let mut req = agent.post(&self.url).config().timeout_global(Some(self.timeout)).build();
            for (k, v) in &self.headers {
                req = req.header(k, v);
            }
            match &self.body {
                Some(b) => req.send(b.as_str()),
                None => req.send_empty(),
            }
        } else {
            let mut req = agent.get(&self.url).config().timeout_global(Some(self.timeout)).build();
            for (k, v) in &self.headers {
                req = req.header(k, v);
            }
            req.call()
        }
        .map_err(|e| anyhow::Error::new(Unreachable(format!("{}: {e}", self.url))))?;
        let status = resp.status().as_u16();
        let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
        let content_type = header("content-type").unwrap_or_default();
        let content_length = header("content-length").and_then(|v| v.parse().ok());
        let body = resp
            .body_mut()
            .with_config()
            .limit(self.max_bytes)
            .read_to_vec()
            .map_err(|e| anyhow::Error::new(Unreachable(format!("{}: {e}", self.url))))?;
        Ok(Response { status, content_type, content_length, body })
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

impl Http {
    pub fn new(endpoints: Endpoints) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(8)
            .timeout_connect(Some(Duration::from_secs(10)))
            .user_agent(BROWSER_UA)
            .build()
            .into();
        Self { agent, endpoints }
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
        }
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
