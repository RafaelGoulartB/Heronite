//! Blocking HTTP for the mod sites: one shared agent, JSON helpers, and
//! errors worded for the UI.

use std::fmt;
use std::sync::OnceLock;
use std::time::Duration;

use heronite_i18n::tr;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Time a JSON call may take, redirects included. Downloads have no such
/// limit: a mod can be gigabytes.
const JSON_TIMEOUT: Duration = Duration::from_secs(40);
/// Largest JSON answer read (a Thunderstore listing page is well below).
const JSON_LIMIT: u64 = 32 * 1024 * 1024;

pub fn agent() -> ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT
        .get_or_init(|| {
            ureq::Agent::config_builder()
                .timeout_connect(Some(Duration::from_secs(15)))
                .timeout_recv_response(Some(Duration::from_secs(30)))
                .max_redirects(10)
                .user_agent(concat!("Heronite/", env!("CARGO_PKG_VERSION")))
                .build()
                .into()
        })
        .clone()
}

/// A failed request. `Display` is the sentence the UI shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpError {
    status: Option<u16>,
    message: String,
}

impl HttpError {
    /// The HTTP status, `None` when the request never got an answer.
    pub fn status(&self) -> Option<u16> {
        self.status
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// Replaces the sentence, keeping the status (a caller that knows why a
    /// status happened says it better).
    pub fn with_message(mut self, message: String) -> Self {
        self.message = message;
        self
    }

    /// The error for an answer with `status` >= 400.
    pub fn from_status(url: &str, status: u16) -> Self {
        let host = host_of(url);
        let nexus = is_nexus_host(&host);
        let message = match status {
            401 | 403 if nexus => tr!("Nexus Mods refused the API key. Paste it again in Settings → Mods.").to_string(),
            429 if nexus => tr!("Nexus Mods asks to wait: the hourly limit of requests was reached").to_string(),
            401 | 403 => tr!("{host} refused the request (HTTP {status})", host, status),
            404 => tr!("Not found on {host}", host),
            429 => tr!("{host} asks to wait: too many requests", host),
            _ => tr!("{host} answered with HTTP {status}", host, status),
        };
        Self { status: Some(status), message }
    }

    /// The error for a request that got no answer (offline, DNS, timeout).
    pub fn network(url: &str, error: impl fmt::Display) -> Self {
        Self { status: None, message: tr!("Could not reach {host}: {error}", host = host_of(url), error) }
    }

    fn unexpected(url: &str, error: impl fmt::Display) -> Self {
        Self { status: None, message: tr!("{host} sent an answer Heronite does not understand: {error}", host = host_of(url), error) }
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HttpError {}

impl From<HttpError> for String {
    fn from(e: HttpError) -> String {
        e.message
    }
}

/// An answer already checked for success, with its headers.
pub struct Reply<T> {
    pub value: T,
    headers: ureq::http::HeaderMap,
}

impl<T> Reply<T> {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

/// The host of `url` (`www.nexusmods.com`), for messages.
pub fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    authority.rsplit('@').next().unwrap_or("").to_string()
}

fn is_nexus_host(host: &str) -> bool {
    host.split(':').next().is_some_and(|h| h == "nexusmods.com" || h.ends_with(".nexusmods.com"))
}

/// Opens `url` with a streaming body and the status checked; for downloads.
pub fn get_stream(url: &str, headers: &[(&str, &str)]) -> Result<ureq::http::Response<ureq::Body>, HttpError> {
    let mut req = agent().get(url);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let resp = req.config().http_status_as_error(false).build().call().map_err(|e| HttpError::network(url, e))?;
    check(url, resp)
}

fn check(url: &str, resp: ureq::http::Response<ureq::Body>) -> Result<ureq::http::Response<ureq::Body>, HttpError> {
    let status = resp.status().as_u16();
    if status >= 400 {
        return Err(HttpError::from_status(url, status));
    }
    Ok(resp)
}

fn read_json<T: DeserializeOwned>(url: &str, resp: ureq::http::Response<ureq::Body>) -> Result<Reply<T>, HttpError> {
    let mut resp = check(url, resp)?;
    let headers = resp.headers().clone();
    let body = resp.body_mut().with_config().limit(JSON_LIMIT).read_to_string().map_err(|e| HttpError::network(url, e))?;
    let value = serde_json::from_str(&body).map_err(|e| HttpError::unexpected(url, e))?;
    Ok(Reply { value, headers })
}

/// GET `url` and parse the JSON answer.
pub fn get_json<T: DeserializeOwned>(url: &str, headers: &[(&str, &str)]) -> Result<T, HttpError> {
    get_json_reply(url, headers).map(|r| r.value)
}

/// [`get_json`] keeping the answer's headers (rate limits).
pub fn get_json_reply<T: DeserializeOwned>(url: &str, headers: &[(&str, &str)]) -> Result<Reply<T>, HttpError> {
    let mut req = agent().get(url);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    req = req.header("Accept", "application/json");
    let resp = req
        .config()
        .http_status_as_error(false)
        .timeout_global(Some(JSON_TIMEOUT))
        .build()
        .call()
        .map_err(|e| HttpError::network(url, e))?;
    read_json(url, resp)
}

/// POST `body` as JSON to `url` and parse the JSON answer.
pub fn post_json<B: Serialize, T: DeserializeOwned>(url: &str, headers: &[(&str, &str)], body: &B) -> Result<T, HttpError> {
    let text = serde_json::to_string(body).map_err(|e| HttpError::unexpected(url, e))?;
    let mut req = agent().post(url).header("Content-Type", "application/json").header("Accept", "application/json");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let resp = req
        .config()
        .http_status_as_error(false)
        .timeout_global(Some(JSON_TIMEOUT))
        .build()
        .send(text.as_str())
        .map_err(|e| HttpError::network(url, e))?;
    read_json(url, resp).map(|r| r.value)
}

/// Deserializes `null` as the type's default (the APIs send `null` for
/// fields that are empty).
pub(crate) fn null_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// `s` as one URL path segment or query value: nothing but `A-Za-z0-9-_.`
/// stays raw.
pub(crate) fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

/// Unix seconds now.
pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Now as RFC 3339, for `checked_at` / `validated_at` fields.
pub(crate) fn now_iso() -> String {
    heronite_data::steam_owned::unix_to_iso(now_secs() as i64).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert_eq!(host_of("https://api.nexusmods.com/v1/x?a=b"), "api.nexusmods.com");
        assert_eq!(host_of("http://user:pw@127.0.0.1:8080/x"), "127.0.0.1:8080");
        assert_eq!(host_of("thunderstore.io"), "thunderstore.io");
        assert!(is_nexus_host("api.nexusmods.com"));
        assert!(is_nexus_host("nexusmods.com"));
        assert!(!is_nexus_host("evilnexusmods.com"));
    }

    #[test]
    fn components_cannot_escape() {
        assert_eq!(encode("skyrimspecialedition"), "skyrimspecialedition");
        assert_eq!(encode("a/../b?c d"), "a%2F..%2Fb%3Fc%20d");
    }

    #[test]
    fn statuses_keep_their_code_and_site_wording() {
        let e = HttpError::from_status("https://api.nexusmods.com/v1/users/validate.json", 401);
        assert_eq!(e.status(), Some(401));
        assert!(e.message().contains("API key"));
        assert!(HttpError::from_status("https://api.nexusmods.com/x", 429).message().contains("hourly"));
        let e = HttpError::from_status("https://thunderstore.io/api/x", 404);
        assert_eq!((e.status(), e.message()), (Some(404), "Not found on thunderstore.io"));
        assert!(HttpError::from_status("https://thunderstore.io/api/x", 429).message().contains("too many requests"));
        assert!(HttpError::from_status("https://thunderstore.io/api/x", 502).message().contains("HTTP 502"));
        assert_eq!(HttpError::network("https://a.b/c", "dns").status(), None);
    }
}
