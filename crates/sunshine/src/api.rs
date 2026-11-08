//! Sunshine's web API (`src/confighttp.cpp` of Sunshine):
//!
//! * `GET /api/apps`: the whole `apps.json`, `{"env": {..}, "apps": [..]}`.
//! * `POST /api/apps` with an application plus `"index"`: `-1` appends it,
//!   another index replaces that application. Sunshine then sorts the list
//!   by name, so indexes move after every save.
//! * `DELETE /api/apps/<index>`: removes one; the others keep their order.
//!
//! Every request carries basic auth. A 401 is a wrong user name or password;
//! a redirect (to `/welcome`) means Sunshine has no account yet; a 403 means
//! the address does not accept this computer (`origin_web_ui_allowed`).
//! POST and DELETE check a CSRF token only when the request has an `Origin`
//! or `Referer` header, which this client never sends.
//!
//! The certificate is Sunshine's own self-signed one. On this computer it is
//! not checked; on another computer it is trusted on first use and refused
//! when it changes ([`crate::pin`]).

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use serde_json::{json, Value};
use ureq::unversioned::resolver::DefaultResolver;
use ureq::unversioned::transport::{Connector, TcpConnector};

use crate::pin::{self, PinnedConnector, Seen};

/// What went wrong talking to Sunshine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The address is not an `http(s)://host[:port]` address.
    BadAddress,
    /// Nothing answers there (refused, unknown host, timed out). The text is
    /// for the log.
    Unreachable(String),
    /// Wrong user name or password.
    Unauthorized,
    /// Sunshine runs but has no web UI account yet.
    NotSetUp,
    /// Sunshine refuses this computer (its "origin" setting).
    Forbidden,
    /// The certificate is not the one kept on first use (another computer at
    /// the address, a reinstalled Sunshine, or someone in between). `seen` is
    /// the new fingerprint, for the user to accept.
    CertificateChanged { seen: String },
    /// Anything else Sunshine (or the connection) answered.
    Api(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::BadAddress => write!(f, "not a Sunshine address"),
            Error::Unreachable(e) => write!(f, "Sunshine does not answer: {e}"),
            Error::Unauthorized => write!(f, "Sunshine refused the user name or password"),
            Error::NotSetUp => write!(f, "Sunshine has no web UI account yet"),
            Error::Forbidden => write!(f, "Sunshine does not accept requests from this computer"),
            Error::CertificateChanged { seen } => write!(f, "Sunshine's certificate changed (now {seen})"),
            Error::Api(e) => write!(f, "Sunshine: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// The default address of Sunshine's web UI.
pub const DEFAULT_ADDRESS: &str = "https://localhost:47990";

/// `localhost:47990` → `https://localhost:47990`, trailing slashes off.
/// `None` when it cannot be an address.
pub fn normalize_address(address: &str) -> Option<String> {
    let a = address.trim();
    let (scheme, rest) = a.split_once("://").unwrap_or(("https", a));
    let rest = rest.trim_end_matches('/');
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let host = rest.split('/').next().unwrap_or("");
    if host.is_empty() || host.contains(char::is_whitespace) || rest.contains('?') || rest.contains('#') {
        return None;
    }
    Some(format!("{scheme}://{rest}"))
}

/// How the server's certificate is trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trust {
    /// Not checked: plain HTTP, or Sunshine on this computer.
    Any,
    /// Pinned. `Some` is the fingerprint kept from the first use; `None` is
    /// a first use, which takes the certificate it sees.
    Pin(Option<String>),
}

impl Trust {
    /// What a (normalized) address gets: nothing checked over plain HTTP or
    /// on this computer, else the pin (`pin` empty: a first use).
    pub fn for_address(address: &str, pin: &str) -> Trust {
        let https = address.trim_start().get(..8).is_some_and(|s| s.eq_ignore_ascii_case("https://"));
        if !https || pin::is_loopback(pin::host_of(address)) {
            return Trust::Any;
        }
        let pin = pin.trim();
        Trust::Pin((!pin.is_empty()).then(|| pin.to_string()))
    }
}

/// A signed-in connection to one Sunshine.
pub struct Client {
    base: String,
    auth: String,
    agent: ureq::Agent,
    /// What the pinned handshakes saw; `None` when certificates are not
    /// checked.
    seen: Option<Arc<Mutex<Seen>>>,
}

impl Client {
    /// `pin`: the certificate fingerprint kept from the first use (empty for
    /// none). It only applies to an HTTPS address on another computer.
    pub fn new(address: &str, username: &str, password: &str, pin: &str) -> Result<Self, Error> {
        let base = normalize_address(address).ok_or(Error::BadAddress)?;
        let trust = Trust::for_address(&base, pin);
        Self::with_trust(&base, username, password, trust)
    }

    /// [`Client::new`] with the trust given (tests pin a local server).
    pub fn with_trust(address: &str, username: &str, password: &str, trust: Trust) -> Result<Self, Error> {
        let base = normalize_address(address).ok_or(Error::BadAddress)?;
        let auth = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}")));
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent("Heronite");
        let (agent, seen) = match trust {
            Trust::Any => {
                let tls = ureq::tls::TlsConfig::builder().disable_verification(true).build();
                (config.tls_config(tls).build().into(), None)
            }
            Trust::Pin(pin) => {
                let seen = Arc::new(Mutex::new(Seen::default()));
                let connector = TcpConnector::default().chain(PinnedConnector::new(pin, seen.clone()));
                (ureq::Agent::with_parts(config.build(), connector, DefaultResolver::default()), Some(seen))
            }
        };
        Ok(Self { base, auth, agent, seen })
    }

    /// The address requests go to (normalized).
    pub fn address(&self) -> &str {
        &self.base
    }

    /// The fingerprint of the certificate the last handshake saw; `None`
    /// when certificates are not checked for this address (or nothing
    /// answered). After a first use, this is the one to keep.
    pub fn certificate(&self) -> Option<String> {
        self.seen.as_ref().and_then(|s| s.lock().ok()?.fingerprint.clone())
    }

    /// The applications, in Sunshine's order (the order indexes refer to).
    pub fn apps(&self) -> Result<Vec<Value>, Error> {
        let url = format!("{}/api/apps", self.base);
        let resp = self.agent.get(&url).header("Authorization", &self.auth).call();
        let doc = answer(resp.map_err(|e| self.failed(e))?)?;
        match doc.get("apps") {
            Some(Value::Array(apps)) => Ok(apps.clone()),
            None | Some(Value::Null) => Ok(Vec::new()),
            Some(_) => Err(Error::Api("the application list is not a list".into())),
        }
    }

    /// Saves `app` at `index` (`None`: as a new application).
    pub fn save(&self, app: &Value, index: Option<usize>) -> Result<(), Error> {
        let mut body = app.clone();
        let Some(obj) = body.as_object_mut() else { return Err(Error::Api("an application must be an object".into())) };
        obj.insert("index".into(), index.map_or(json!(-1), |i| json!(i)));
        let url = format!("{}/api/apps", self.base);
        let resp =
            self.agent.post(&url).header("Authorization", &self.auth).header("Content-Type", "application/json").send(body.to_string());
        answer(resp.map_err(|e| self.failed(e))?).map(drop)
    }

    /// Deletes the application at `index`.
    pub fn delete(&self, index: usize) -> Result<(), Error> {
        let url = format!("{}/api/apps/{index}", self.base);
        let resp = self.agent.delete(&url).header("Authorization", &self.auth).call();
        answer(resp.map_err(|e| self.failed(e))?).map(drop)
    }

    /// A failed request as an error, telling a refused certificate apart.
    fn failed(&self, e: ureq::Error) -> Error {
        if let Some(seen) = self.seen.as_ref().and_then(|s| s.lock().ok()) {
            if seen.changed {
                return Error::CertificateChanged { seen: seen.fingerprint.clone().unwrap_or_default() };
            }
        }
        transport(e)
    }
}

/// Turns a response into its JSON body, or the error it stands for.
fn answer(mut resp: ureq::http::Response<ureq::Body>) -> Result<Value, Error> {
    let status = resp.status().as_u16();
    // Small answers only; the app list of a big library stays well under this.
    let body = resp.body_mut().with_config().limit(32 * 1024 * 1024).read_to_string().unwrap_or_default();
    match status {
        200..=299 => {
            let doc: Value = serde_json::from_str(&body).map_err(|e| Error::Api(format!("unreadable answer: {e}")))?;
            if doc.get("status").and_then(Value::as_bool) == Some(false) {
                return Err(Error::Api(message(&doc).unwrap_or_else(|| "refused".into())));
            }
            Ok(doc)
        }
        300..=399 => Err(Error::NotSetUp),
        401 => Err(Error::Unauthorized),
        403 => Err(Error::Forbidden),
        _ => {
            let detail = serde_json::from_str::<Value>(&body).ok().and_then(|d| message(&d)).unwrap_or_else(|| short(&body));
            Err(Error::Api(if detail.is_empty() { format!("HTTP {status}") } else { format!("HTTP {status}: {detail}") }))
        }
    }
}

fn message(doc: &Value) -> Option<String> {
    doc.get("error").and_then(Value::as_str).map(str::to_string)
}

fn short(body: &str) -> String {
    body.trim().chars().take(200).collect()
}

fn transport(e: ureq::Error) -> Error {
    match e {
        ureq::Error::Io(_) | ureq::Error::ConnectionFailed | ureq::Error::HostNotFound | ureq::Error::Timeout(_) => {
            Error::Unreachable(e.to_string())
        }
        other => Error::Api(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_address, Trust};

    #[test]
    fn addresses_get_a_scheme_and_lose_trailing_slashes() {
        assert_eq!(normalize_address("localhost:47990").as_deref(), Some("https://localhost:47990"));
        assert_eq!(normalize_address(" https://localhost:47990/ ").as_deref(), Some("https://localhost:47990"));
        assert_eq!(normalize_address("HTTP://127.0.0.1:47990").as_deref(), Some("http://127.0.0.1:47990"));
        assert_eq!(normalize_address("https://pc.lan:47990/sub").as_deref(), Some("https://pc.lan:47990/sub"));
        assert_eq!(normalize_address(""), None);
        assert_eq!(normalize_address("ftp://x"), None);
        assert_eq!(normalize_address("https://"), None);
        assert_eq!(normalize_address("https://a b"), None);
        assert_eq!(normalize_address("https://x/?q=1"), None);
    }

    #[test]
    fn certificates_are_pinned_only_for_https_on_another_computer() {
        assert_eq!(Trust::for_address("https://localhost:47990", "AA"), Trust::Any);
        assert_eq!(Trust::for_address("https://127.0.0.1:47990", ""), Trust::Any);
        assert_eq!(Trust::for_address("https://[::1]:47990", ""), Trust::Any);
        assert_eq!(Trust::for_address("http://192.168.1.20:47990", ""), Trust::Any);
        assert_eq!(Trust::for_address("https://192.168.1.20:47990", ""), Trust::Pin(None));
        assert_eq!(Trust::for_address("https://pc.lan:47990", " AA:BB "), Trust::Pin(Some("AA:BB".into())));
    }
}
