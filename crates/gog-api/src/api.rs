//! The HTTP side: one [`Client`] per sign-in, retrying what the network
//! loses, and the GOG addresses the other modules read. Everything that
//! interprets an answer lives with its model, so only this file touches the
//! network.

use std::io::Read;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;

use crate::auth::SignInExpired;

pub const CONTENT_SYSTEM: &str = "https://content-system.gog.com";
pub const CDN: &str = "https://gog-cdn-fastly.gog.com";
pub const EMBED: &str = "https://embed.gog.com";
pub const AUTH: &str = "https://auth.gog.com";
pub const API: &str = "https://api.gog.com";
pub const CLOUD_STORAGE: &str = "https://cloudstorage.gog.com";

/// Tries of a request that failed on the way (not a refusal by GOG).
const ATTEMPTS: u32 = 4;
/// Biggest body read into memory: a v1 build of a large game lists every
/// file in one JSON document.
const BODY_LIMIT: u64 = 1024 * 1024 * 1024;
/// The User-Agent GOG Galaxy's own service sends; cloud storage refuses
/// others.
pub const GALAXY_AGENT: &str = "GOGGalaxyCommunicationService/2.0.13.27 (Windows_32bit) dont_sync_marker/true installation_source/gog";

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(15)))
            .max_redirects(8)
            .user_agent("Heronite")
            .http_status_as_error(false)
            .build()
            .into()
    })
}

/// A finished request; any status is a result, not an error.
#[derive(Debug, Clone, Default)]
pub struct Response {
    pub status: u16,
    pub etag: Option<String>,
    /// Headers of interest, lower-case names.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// The body as JSON.
    pub fn json(&self) -> Result<Value> {
        serde_json::from_slice(&self.body).context("GOG answered with invalid JSON")
    }
}

/// What GOG's servers are asked with: the account's token (if any) and the
/// User-Agent.
#[derive(Debug, Clone, Default)]
pub struct Client {
    token: Option<String>,
    user_agent: Option<String>,
}

/// One request of [`Client::request`].
enum Method<'a> {
    Get,
    Put(&'a [u8]),
    Delete,
    Post,
}

impl Client {
    /// Requests without a sign-in (public meta and chunks).
    pub fn anonymous() -> Self {
        Self::default()
    }

    /// Requests signed in with `access_token`.
    pub fn with_token(access_token: &str) -> Self {
        Self { token: Some(access_token.to_string()), user_agent: None }
    }

    /// Sends `agent` as the User-Agent.
    pub fn user_agent(mut self, agent: &str) -> Self {
        self.user_agent = Some(agent.to_string());
        self
    }

    fn request(&self, method: &Method, url: &str, headers: &[(&str, &str)]) -> Result<Response> {
        let mut last = None;
        for attempt in 0..ATTEMPTS {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(500 * u64::from(attempt * attempt)));
            }
            match self.once(method, url, headers) {
                // A busy server answers again; anything else stands.
                Ok(resp) if matches!(resp.status, 429 | 500 | 502 | 503 | 504) && attempt + 1 < ATTEMPTS => {
                    log::debug!("{}: HTTP {}, trying again", redact(url), resp.status);
                    last = Some(anyhow!("HTTP {}", resp.status));
                }
                Ok(resp) => return Ok(resp),
                Err(err) => {
                    log::debug!("{}: {err:#}", redact(url));
                    last = Some(err);
                }
            }
        }
        Err(last.unwrap_or_else(|| anyhow!("no answer")).context(format!("GET {}", redact(url))))
    }

    fn once(&self, method: &Method, url: &str, headers: &[(&str, &str)]) -> Result<Response> {
        let auth = self.token.as_ref().map(|t| format!("Bearer {t}"));
        let mut extra: Vec<(&str, &str)> = headers.to_vec();
        if let Some(a) = auth.as_deref() {
            if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("authorization")) {
                extra.push(("Authorization", a));
            }
        }
        if let Some(ua) = self.user_agent.as_deref() {
            extra.push(("User-Agent", ua));
        }
        let timeout = Some(Duration::from_secs(120));
        let result = match method {
            Method::Get => {
                let mut req = agent().get(url);
                for (k, v) in &extra {
                    req = req.header(*k, *v);
                }
                req.config().timeout_global(timeout).build().call()
            }
            Method::Delete => {
                let mut req = agent().delete(url);
                for (k, v) in &extra {
                    req = req.header(*k, *v);
                }
                req.config().timeout_global(timeout).build().call()
            }
            Method::Put(body) => {
                let mut req = agent().put(url);
                for (k, v) in &extra {
                    req = req.header(*k, *v);
                }
                req.config().timeout_global(timeout).build().send(*body)
            }
            Method::Post => {
                let mut req = agent().post(url);
                for (k, v) in &extra {
                    req = req.header(*k, *v);
                }
                req.config().timeout_global(timeout).build().send_empty()
            }
        };
        let mut resp = result?;
        let status = resp.status().as_u16();
        let headers: Vec<(String, String)> =
            resp.headers().iter().filter_map(|(k, v)| Some((k.as_str().to_ascii_lowercase(), v.to_str().ok()?.to_string()))).collect();
        let etag = headers.iter().find(|(k, _)| k == "etag").map(|(_, v)| v.clone());
        let body = resp.body_mut().with_config().limit(BODY_LIMIT).read_to_vec()?;
        Ok(Response { status, etag, headers, body })
    }

    /// GET; every status is returned.
    pub fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<Response> {
        self.request(&Method::Get, url, headers)
    }

    /// PUT of `body`; every status is returned.
    pub fn put(&self, url: &str, body: &[u8], headers: &[(&str, &str)]) -> Result<Response> {
        self.request(&Method::Put(body), url, headers)
    }

    /// DELETE; every status is returned.
    pub fn delete(&self, url: &str) -> Result<Response> {
        self.request(&Method::Delete, url, &[])
    }

    /// POST without a body; every status is returned.
    pub fn post(&self, url: &str) -> Result<Response> {
        self.request(&Method::Post, url, &[])
    }

    /// GET of a JSON document. A 401 is a [`SignInExpired`].
    pub fn get_json(&self, url: &str) -> Result<Value> {
        let resp = self.get(url, &[("Accept", "application/json")])?;
        check(&resp, url)?;
        resp.json().with_context(|| redact(url))
    }

    /// GET of a document the content system sends zlib-compressed (meta,
    /// depot manifests), or as plain JSON for a few old ones. Returns it and
    /// its `ETag`.
    pub fn get_zlib_json(&self, url: &str) -> Result<(Value, Option<String>)> {
        let resp = self.get(url, &[])?;
        check(&resp, url)?;
        Ok((decode_zlib_json(&resp.body).with_context(|| redact(url))?, resp.etag))
    }

    /// `size` bytes of `url` from `offset` (an HTTP range).
    pub fn get_range(&self, url: &str, offset: u64, size: u64) -> Result<Vec<u8>> {
        if size == 0 {
            return Ok(Vec::new());
        }
        let range = range_header(offset, size);
        let resp = self.get(url, &[("Range", &range)])?;
        check(&resp, url)?;
        if resp.status != 206 && resp.body.len() as u64 != size {
            bail!("{}: the server ignored the byte range", redact(url));
        }
        if resp.body.len() as u64 != size {
            bail!("{}: got {} bytes of {size}", redact(url), resp.body.len());
        }
        Ok(resp.body)
    }

    /// The total size of `url` from a one-byte range (`Content-Range`).
    pub fn content_length(&self, url: &str) -> Result<u64> {
        let resp = self.get(url, &[("Range", "bytes=0-0")])?;
        check(&resp, url)?;
        resp.header("content-range")
            .and_then(|r| r.rsplit('/').next())
            .and_then(|n| n.trim().parse().ok())
            .ok_or_else(|| anyhow!("{}: no size in the answer", redact(url)))
    }

    /// Where `url` leads after its redirects, and the size of what is there:
    /// a download link sends to a signed CDN address, asked directly from
    /// then on.
    pub fn resolve(&self, url: &str) -> Result<(String, u64)> {
        use ureq::ResponseExt;
        let mut last = None;
        for attempt in 0..ATTEMPTS {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(500 * u64::from(attempt * attempt)));
            }
            let mut req = agent().get(url).header("Range", "bytes=0-0");
            if let Some(t) = self.token.as_deref() {
                req = req.header("Authorization", format!("Bearer {t}"));
            }
            match req.config().timeout_global(Some(Duration::from_secs(60))).build().call() {
                Ok(mut resp) => {
                    let status = resp.status().as_u16();
                    let size = resp
                        .headers()
                        .get("content-range")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|r| r.rsplit('/').next()?.trim().parse().ok());
                    let final_url = resp.get_uri().to_string();
                    let _ = resp.body_mut().read_to_vec();
                    match (status, size) {
                        (206, Some(size)) => return Ok((final_url, size)),
                        (401, _) => return Err(anyhow!(SignInExpired).context(format!("{}: HTTP 401", redact(url)))),
                        _ => last = Some(anyhow!("{}: HTTP {status}", redact(url))),
                    }
                }
                Err(err) => last = Some(anyhow!(err)),
            }
        }
        Err(last.unwrap_or_else(|| anyhow!("no answer")))
    }

    /// A streaming GET of `size` bytes from `offset`, for a body too big to
    /// hold (one file of a Linux installer). The reader yields the raw bytes.
    pub fn open_range(&self, url: &str, offset: u64, size: u64) -> Result<Box<dyn Read + Send>> {
        let range = range_header(offset, size);
        let mut req = agent().get(url).header("Range", &range);
        if let Some(ua) = self.user_agent.as_deref() {
            req = req.header("User-Agent", ua);
        }
        let resp = req.config().timeout_global(None).build().call().with_context(|| format!("GET {}", redact(url)))?;
        let status = resp.status().as_u16();
        if status != 206 {
            bail!("{}: HTTP {status}", redact(url));
        }
        Ok(Box::new(resp.into_body().into_reader().take(size)))
    }
}

/// An error for a refused request; 401 is a [`SignInExpired`].
pub fn check(resp: &Response, url: &str) -> Result<()> {
    if resp.status == 401 {
        return Err(anyhow!(SignInExpired).context(format!("{}: HTTP 401", redact(url))));
    }
    if !resp.ok() {
        bail!("{}: HTTP {}", redact(url), resp.status);
    }
    Ok(())
}

/// A zlib document, or plain JSON when it is not compressed.
pub fn decode_zlib_json(body: &[u8]) -> Result<Value> {
    let mut text = Vec::new();
    match flate2::read::ZlibDecoder::new(body).read_to_end(&mut text) {
        Ok(_) => serde_json::from_slice(&text).context("invalid JSON in a compressed document"),
        Err(_) => serde_json::from_slice(body).context("invalid JSON"),
    }
}

/// `Range` header value for `size` bytes from `offset`.
pub fn range_header(offset: u64, size: u64) -> String {
    format!("bytes={offset}-{}", offset + size.max(1) - 1)
}

/// `url` without its query, for logs (tokens and signatures live there).
pub fn redact(url: &str) -> String {
    match url.split_once('?') {
        Some((base, _)) => format!("{base}?…"),
        None => url.to_string(),
    }
}

/// Percent-encodes `text` for a URL path or query (`/` kept when `path`).
pub fn encode(text: &str, path: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') || (path && b == b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The `/xx/yy/` folders the content system files a hash under
/// (`abcdef…` → `ab/cd/abcdef…`); a path that has them already is kept.
pub fn galaxy_path(hash: &str) -> String {
    if hash.contains('/') || hash.len() < 4 {
        return hash.to_string();
    }
    format!("{}/{}/{hash}", &hash[0..2], &hash[2..4])
}

/// `/products/<id>/os/<os>/builds` (`generation` 1 or 2).
pub fn builds_url(game_id: &str, os: &str, generation: u8) -> String {
    format!("{CONTENT_SYSTEM}/products/{game_id}/os/{os}/builds?generation={generation}")
}

/// A depot manifest of a v2 build.
pub fn depot_url(manifest: &str) -> String {
    format!("{CDN}/content-system/v2/meta/{}", galaxy_path(manifest))
}

/// A depot manifest of a redistributable.
pub fn dependency_depot_url(manifest: &str) -> String {
    format!("{CDN}/content-system/v2/dependencies/meta/{}", galaxy_path(manifest))
}

/// The repository of redistributables (generation 2).
pub const DEPENDENCIES_URL: &str = "https://content-system.gog.com/dependencies/repository?generation=2";

/// The product document (`expand` e.g. `downloads,expanded_dlcs`).
pub fn product_url(game_id: &str, expand: &[&str]) -> String {
    if expand.is_empty() {
        format!("{API}/products/{game_id}")
    } else {
        format!("{API}/products/{game_id}?expand={}", expand.join(","))
    }
}

/// The ids the account owns (`{"owned": [..]}`).
pub const OWNED_URL: &str = "https://embed.gog.com/user/data/games";

impl Client {
    /// Ids of every product the account owns, as text.
    pub fn owned(&self) -> Result<Vec<String>> {
        let doc = self.get_json(OWNED_URL)?;
        Ok(doc.get("owned").and_then(Value::as_array).map(|l| l.iter().filter_map(id_text).collect()).unwrap_or_default())
    }

    /// The repository of redistributables: its build (`build_id`,
    /// `repository_manifest`) and the repository itself, with `build_id`
    /// added the way gogdl keeps it.
    pub fn dependencies_repository(&self) -> Result<Value> {
        let build = self.get_json(DEPENDENCIES_URL)?;
        let url = build.get("repository_manifest").and_then(Value::as_str).ok_or_else(|| anyhow!("no redist repository"))?;
        let (mut repo, _) = self.get_zlib_json(url)?;
        if let Some(obj) = repo.as_object_mut() {
            obj.insert("build_id".into(), build.get("build_id").cloned().unwrap_or(Value::Null));
        }
        Ok(repo)
    }
}

/// An id GOG sends as a number or as text.
pub fn id_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn hashes_are_filed_in_two_folders() {
        assert_eq!(galaxy_path("abcdef0123"), "ab/cd/abcdef0123");
        assert_eq!(galaxy_path("ab/cd/abcdef"), "ab/cd/abcdef");
        assert_eq!(depot_url("0011aa"), format!("{CDN}/content-system/v2/meta/00/11/0011aa"));
    }

    #[test]
    fn documents_may_be_compressed_or_not() {
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(br#"{"a": 1}"#).unwrap();
        assert_eq!(decode_zlib_json(&z.finish().unwrap()).unwrap()["a"], 1);
        assert_eq!(decode_zlib_json(br#"{"b": 2}"#).unwrap()["b"], 2);
        assert!(decode_zlib_json(b"nope").is_err());
    }

    #[test]
    fn ranges_urls_and_logs() {
        assert_eq!(range_header(10, 5), "bytes=10-14");
        assert_eq!(redact("https://x/y?token=secret"), "https://x/y?…");
        assert_eq!(encode("a b/c", true), "a%20b/c");
        assert_eq!(encode("a b/c", false), "a%20b%2Fc");
        assert_eq!(builds_url("1", "windows", 2), "https://content-system.gog.com/products/1/os/windows/builds?generation=2");
        assert_eq!(product_url("1", &["downloads", "expanded_dlcs"]), "https://api.gog.com/products/1?expand=downloads,expanded_dlcs");
        assert_eq!(id_text(&serde_json::json!(12)), Some("12".into()));
        assert_eq!(id_text(&serde_json::json!("")), None);
    }

    #[test]
    fn refusals_are_errors_and_401_is_an_expired_sign_in() {
        let r = |status| Response { status, ..Default::default() };
        assert!(check(&r(200), "u").is_ok());
        assert!(check(&r(404), "u").is_err());
        assert!(check(&r(401), "u").unwrap_err().downcast_ref::<SignInExpired>().is_some());
    }
}
