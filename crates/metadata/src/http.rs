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

/// `url` for messages and logs: scheme, host, port and path only. The query
/// (API keys, client secrets), fragment and userinfo never reach error text.
pub fn redact_url(url: &str) -> String {
    if let Ok(u) = url::Url::parse(url) {
        if let Some(host) = u.host_str() {
            let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
            return format!("{}://{host}{port}{}", u.scheme(), u.path());
        }
    }
    // Not parseable: still cut the query, the fragment and any `user:pass@`.
    let bare = url.split(['?', '#']).next().unwrap_or("");
    match bare.split_once("://") {
        Some((scheme, rest)) => {
            let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
            format!("{scheme}://{}{path}", authority.rsplit('@').next().unwrap_or(""))
        }
        None => bare.to_owned(),
    }
}

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

    /// `error` prefixed with the URL, minus anything secret: the error text
    /// may repeat the URL or its query too.
    fn describe(&self, error: &ureq::Error) -> String {
        let shown = redact_url(&self.url);
        let mut text = error.to_string().replace(&self.url, &shown);
        if let Some(query) = url::Url::parse(&self.url).ok().and_then(|u| u.query().map(str::to_owned)).filter(|q| !q.is_empty()) {
            text = text.replace(&query, "***");
        }
        format!("{shown}: {text}")
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
        .map_err(|e| anyhow::Error::new(Unreachable(self.describe(&e))))?;
        let status = resp.status().as_u16();
        let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
        let content_type = header("content-type").unwrap_or_default();
        let content_length = header("content-length").and_then(|v| v.parse().ok());
        let body = resp.body_mut().with_config().limit(self.max_bytes).read_to_vec().map_err(|e| match e {
            ureq::Error::BodyExceedsLimit(max) => anyhow!("{}: response larger than {max} bytes", redact_url(&self.url)),
            e => anyhow::Error::new(Unreachable(self.describe(&e))),
        })?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_url_keeps_only_scheme_host_port_and_path() {
        assert_eq!(redact_url("https://id.twitch.tv/oauth2/token?client_id=ID&client_secret=SECRET"), "https://id.twitch.tv/oauth2/token");
        assert_eq!(redact_url("http://user:pw@127.0.0.1:8080/a/b?k=v#frag"), "http://127.0.0.1:8080/a/b");
        assert_eq!(redact_url("not a url?secret=1"), "not a url");
        assert_eq!(redact_url("weird://u:p@host/x?s=1"), "weird://host/x");
    }

    #[test]
    fn an_unreachable_host_never_shows_the_query() {
        let http = Http::default();
        let err = http
            .post("http://127.0.0.1:1/oauth2/token")
            .query(&[("client_id", "the-id"), ("client_secret", "hunter2-secret")])
            .timeout_secs(2)
            .send()
            .unwrap_err();
        assert!(is_unreachable(&err));
        let text = format!("{err:#}");
        assert!(text.contains("http://127.0.0.1:1/oauth2/token"), "{text}");
        assert!(!text.contains("hunter2-secret") && !text.contains("the-id"), "{text}");
    }

    #[test]
    fn a_body_over_the_limit_is_not_unreachable() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf);
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5000\r\nConnection: close\r\n\r\n");
                let _ = s.write_all(&[b'x'; 5000]);
            }
        });
        let err = Http::default().get(format!("http://127.0.0.1:{port}/big?token=sekret")).max_bytes(100).send().unwrap_err();
        assert!(!is_unreachable(&err), "{err:#}");
        let text = err.to_string();
        assert!(text.contains("larger than 100 bytes") && !text.contains("sekret"), "{text}");
    }
}
