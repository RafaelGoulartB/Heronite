//! The HTTP client of every Epic request: the Epic Games Launcher's user
//! agent, timeouts, and retries of reads that failed on the way (network,
//! HTTP 5xx). Blocking.

use std::time::Duration;

use anyhow::{anyhow, Result};
use serde_json::Value;

/// User agent of the Epic Games Launcher that legendary sends.
pub const USER_AGENT: &str = "UELauncher/11.0.1-14907503+++Portal+Release-Live Windows/10.0.19041.1.256.64bit";
/// User agent of the launcher's store pages (GraphQL).
pub const STORE_USER_AGENT: &str = "EpicGamesLauncher/14.0.8-22004686+++Portal+Release-Live";

/// The largest body read: a big game's manifest is tens of MiB.
const MAX_BODY: u64 = 512 * 1024 * 1024;
const ATTEMPTS: u32 = 3;

/// A finished request.
#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Response {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn json(&self) -> Result<Value> {
        if self.body.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&self.body).map_err(|e| anyhow!("invalid JSON (HTTP {}): {e}", self.status))
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// An answer of an Epic service that refused the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub status: u16,
    /// Epic's `errorCode`, such as `errors.com.epicgames.account.invalid_account_credentials`.
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.code.is_empty(), self.message.is_empty()) {
            (false, false) => write!(f, "{} ({}, HTTP {})", self.message, self.code, self.status),
            (false, true) => write!(f, "{} (HTTP {})", self.code, self.status),
            _ => write!(f, "HTTP {}", self.status),
        }
    }
}

impl std::error::Error for ApiError {}

impl ApiError {
    /// From a refused response; Epic's JSON errors carry `errorCode` and
    /// `errorMessage`.
    pub fn from_response(resp: &Response) -> Self {
        let doc = resp.json().unwrap_or(Value::Null);
        let text = |k: &str| doc.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        ApiError { status: resp.status, code: text("errorCode"), message: text("errorMessage") }
    }
}

/// The body of a request.
#[derive(Debug, Clone, Copy)]
pub enum Body<'a> {
    None,
    Form(&'a [(&'a str, &'a str)]),
    Json(&'a Value),
    Bytes(&'a [u8]),
}

/// What to authenticate a request with.
#[derive(Debug, Clone, Copy)]
pub enum Auth<'a> {
    None,
    Bearer(&'a str),
    Basic(&'a str, &'a str),
}

/// One request.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub auth: Auth<'a>,
    pub query: &'a [(&'a str, &'a str)],
    pub body: Body<'a>,
    pub user_agent: &'a str,
}

impl<'a> Request<'a> {
    pub fn get(url: &'a str) -> Self {
        Request { method: "GET", url, auth: Auth::None, query: &[], body: Body::None, user_agent: USER_AGENT }
    }

    pub fn post(url: &'a str, body: Body<'a>) -> Self {
        Request { method: "POST", body, ..Self::get(url) }
    }

    pub fn auth(self, auth: Auth<'a>) -> Self {
        Request { auth, ..self }
    }

    pub fn bearer(self, token: &'a str) -> Self {
        self.auth(Auth::Bearer(token))
    }

    pub fn query(self, query: &'a [(&'a str, &'a str)]) -> Self {
        Request { query, ..self }
    }
}

/// The client: one connection pool for every request.
#[derive(Clone)]
pub struct Http {
    agent: ureq::Agent,
}

impl Default for Http {
    fn default() -> Self {
        Self::new(Duration::from_secs(30))
    }
}

impl Http {
    /// A client whose requests give up after `timeout`.
    pub fn new(timeout: Duration) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .max_idle_connections_per_host(16)
            .build()
            .into();
        Self { agent }
    }

    /// Sends `req`. A GET is tried again (three times in all) after a network
    /// error or an HTTP 5xx; the last answer is returned whatever its status.
    pub fn send(&self, req: &Request) -> Result<Response> {
        let retries = if req.method == "GET" { ATTEMPTS } else { 1 };
        let mut last_err = None;
        for attempt in 1..=retries {
            match self.send_once(req) {
                Ok(resp) if resp.status >= 500 && attempt < retries => {
                    log::debug!("{} {}: HTTP {}, trying again", req.method, host(req.url), resp.status);
                }
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    log::debug!("{} {}: {e}", req.method, host(req.url));
                    last_err = Some(e);
                }
            }
            if attempt < retries {
                std::thread::sleep(Duration::from_millis(400 * attempt as u64));
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("{}: no answer", host(req.url))))
    }

    fn send_once(&self, req: &Request) -> Result<Response> {
        let fail = |e: ureq::Error| anyhow!("{}: {e}", host(req.url));
        let auth_header = match req.auth {
            Auth::None => None,
            Auth::Bearer(t) => Some(format!("bearer {t}")),
            Auth::Basic(user, pass) => {
                use base64::Engine;
                Some(format!("basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"))))
            }
        };
        let mut resp = match req.method {
            "GET" | "DELETE" => {
                let mut b = if req.method == "GET" { self.agent.get(req.url) } else { self.agent.delete(req.url) };
                b = b.header("User-Agent", req.user_agent).query_pairs(req.query.iter().copied());
                if let Some(a) = &auth_header {
                    b = b.header("Authorization", a);
                }
                b.call().map_err(fail)?
            }
            method => {
                let mut b = match method {
                    "PUT" => self.agent.put(req.url),
                    _ => self.agent.post(req.url),
                };
                b = b.header("User-Agent", req.user_agent).query_pairs(req.query.iter().copied());
                if let Some(a) = &auth_header {
                    b = b.header("Authorization", a);
                }
                match req.body {
                    Body::None => b.send_empty(),
                    Body::Form(fields) => b.send_form(fields.iter().copied()),
                    Body::Json(v) => b.content_type("application/json").send(serde_json::to_vec(v)?.as_slice()),
                    Body::Bytes(bytes) => b.send(bytes),
                }
                .map_err(fail)?
            }
        };
        let status = resp.status().as_u16();
        let body = resp.body_mut().with_config().limit(MAX_BODY).read_to_vec().map_err(fail)?;
        Ok(Response { status, body })
    }

    /// `GET` that must succeed; the JSON answer.
    pub fn get_json(&self, req: &Request) -> Result<Value> {
        let resp = self.send(req)?;
        if !resp.ok() {
            return Err(ApiError::from_response(&resp).into());
        }
        resp.json()
    }

    /// A request that must succeed; the JSON answer.
    pub fn json(&self, req: &Request) -> Result<Value> {
        self.get_json(req)
    }

    /// A download that must succeed; the bytes.
    pub fn bytes(&self, url: &str) -> Result<Vec<u8>> {
        let resp = self.send(&Request::get(url))?;
        if !resp.ok() {
            return Err(anyhow!("{}: HTTP {}", host(url), resp.status));
        }
        Ok(resp.body)
    }
}

/// The host of a URL, for messages (paths carry ids and query strings carry
/// signatures, neither belongs in a log).
pub fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['/', '?']).next().unwrap_or(rest)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn hosts_for_messages() {
        assert_eq!(host("https://a.example.com/x/y?sig=1"), "a.example.com");
        assert_eq!(host("a.example.com"), "a.example.com");
    }

    #[test]
    fn api_errors_read_epics_fields() {
        let resp = Response {
            status: 400,
            body: br#"{"errorCode":"errors.com.epicgames.account.auth_token.invalid_refresh_token","errorMessage":"Sorry"}"#.to_vec(),
        };
        let e = ApiError::from_response(&resp);
        assert_eq!(e.code, "errors.com.epicgames.account.auth_token.invalid_refresh_token");
        assert_eq!(e.to_string(), "Sorry (errors.com.epicgames.account.auth_token.invalid_refresh_token, HTTP 400)");
        assert_eq!(ApiError::from_response(&Response { status: 502, body: vec![] }).to_string(), "HTTP 502");
    }

    /// A local server answering each connection with the next canned reply.
    pub(crate) fn serve(replies: Vec<(u16, &'static str)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for (status, body) in replies {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                let mut len = 0usize;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                let mut body_in = vec![0; len];
                std::io::Read::read_exact(&mut reader, &mut body_in).unwrap();
                seen.push(format!("{} {}", first.trim(), String::from_utf8_lossy(&body_in)));
                let mut s = stream;
                write!(s, "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            seen
        });
        (url, handle)
    }

    #[test]
    fn reads_are_retried_after_server_errors() {
        let (url, server) = serve(vec![(503, ""), (200, r#"{"a":1}"#)]);
        let http = Http::new(Duration::from_secs(5));
        let doc = http.get_json(&Request::get(&format!("{url}/x")).query(&[("label", "Live")])).unwrap();
        assert_eq!(doc["a"], 1);
        let seen = server.join().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen[1].starts_with("GET /x?label=Live"), "{seen:?}");
    }

    #[test]
    fn posts_are_sent_once_with_their_form() {
        let (url, server) = serve(vec![(400, r#"{"errorCode":"bad"}"#)]);
        let http = Http::new(Duration::from_secs(5));
        let err = http.json(&Request::post(&format!("{url}/t"), Body::Form(&[("grant_type", "refresh_token")]))).unwrap_err();
        assert_eq!(err.downcast_ref::<ApiError>().unwrap().code, "bad");
        assert_eq!(server.join().unwrap(), ["POST /t HTTP/1.1 grant_type=refresh_token"]);
    }
}
