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
//! Only a Sunshine on this computer is talked to ([`is_this_computer`]): the
//! entries hold this computer's paths (Heronite's executable, the covers),
//! which mean nothing to a Sunshine elsewhere. That is also why its
//! self-signed certificate is not verified: what answers on the loopback
//! address is already on this machine.

use std::fmt;
use std::time::Duration;

use base64::Engine;
use serde_json::{json, Value};

/// What went wrong talking to Sunshine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The address is not an `http(s)://host[:port]` address.
    BadAddress,
    /// The address is another computer's; only this computer's Sunshine
    /// can run the entries.
    NotThisComputer,
    /// Nothing answers there (refused, unknown host, timed out). The text is
    /// for the log.
    Unreachable(String),
    /// Wrong user name or password.
    Unauthorized,
    /// Sunshine runs but has no web UI account yet.
    NotSetUp,
    /// Sunshine refuses this computer (its "origin" setting).
    Forbidden,
    /// Anything else Sunshine (or the connection) answered.
    Api(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::BadAddress => write!(f, "not a Sunshine address"),
            Error::NotThisComputer => write!(f, "not an address on this computer"),
            Error::Unreachable(e) => write!(f, "Sunshine does not answer: {e}"),
            Error::Unauthorized => write!(f, "Sunshine refused the user name or password"),
            Error::NotSetUp => write!(f, "Sunshine has no web UI account yet"),
            Error::Forbidden => write!(f, "Sunshine does not accept requests from this computer"),
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

/// The host of `https://host:port/path` (IPv6 without its brackets).
fn host_of(address: &str) -> &str {
    let rest = address.split_once("://").map_or(address, |(_, r)| r);
    let authority = rest.split('/').next().unwrap_or("");
    let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    if let Some(v6) = authority.strip_prefix('[') {
        return v6.split(']').next().unwrap_or("");
    }
    authority.split(':').next().unwrap_or("")
}

/// Whether `address` is this computer: `localhost`, `127.x.x.x` or `::1`.
pub fn is_this_computer(address: &str) -> bool {
    let host = host_of(address);
    host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// A signed-in connection to one Sunshine.
pub struct Client {
    base: String,
    auth: String,
    agent: ureq::Agent,
}

impl Client {
    pub fn new(address: &str, username: &str, password: &str) -> Result<Self, Error> {
        let base = normalize_address(address).ok_or(Error::BadAddress)?;
        if !is_this_computer(&base) {
            return Err(Error::NotThisComputer);
        }
        let auth = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}")));
        let tls = ureq::tls::TlsConfig::builder().disable_verification(true).build();
        let agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent("Heronite")
            .build()
            .into();
        Ok(Self { base, auth, agent })
    }

    /// The address requests go to (normalized).
    pub fn address(&self) -> &str {
        &self.base
    }

    /// The applications, in Sunshine's order (the order indexes refer to).
    pub fn apps(&self) -> Result<Vec<Value>, Error> {
        let url = format!("{}/api/apps", self.base);
        let resp = self.agent.get(&url).header("Authorization", &self.auth).call();
        let doc = answer(resp)?;
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
        answer(resp).map(drop)
    }

    /// Deletes the application at `index`.
    pub fn delete(&self, index: usize) -> Result<(), Error> {
        let url = format!("{}/api/apps/{index}", self.base);
        let resp = self.agent.delete(&url).header("Authorization", &self.auth).call();
        answer(resp).map(drop)
    }
}

/// Turns a response into its JSON body, or the error it stands for.
fn answer(resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Value, Error> {
    let mut resp = resp.map_err(transport)?;
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
    use super::{is_this_computer, normalize_address, Client, Error};

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
    fn only_this_computer_is_accepted() {
        for a in ["https://localhost:47990", "https://LOCALHOST", "http://127.0.0.1:47990", "https://127.1.2.3", "https://[::1]:47990/x"] {
            assert!(is_this_computer(a), "{a}");
        }
        for a in [
            "https://pc.lan:47990",
            "https://192.168.1.20:47990",
            "https://[::2]:47990",
            "https://localhost.evil.com",
            "https://localhost@evil.com",
        ] {
            assert!(!is_this_computer(a), "{a}");
        }
        assert!(matches!(Client::new("https://192.168.1.20:47990", "u", "p"), Err(Error::NotThisComputer)));
        assert!(matches!(Client::new("ftp://localhost", "u", "p"), Err(Error::BadAddress)));
        assert!(Client::new("localhost:47990", "u", "p").is_ok());
    }
}
