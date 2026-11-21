//! The requests the EA account sends, behind [`Http`] so the tests answer
//! them from fixtures ([`Fake`]) and nothing here touches the network then.

use std::time::Duration;

use anyhow::{anyhow, Result};
use serde_json::Value;

#[derive(Clone, PartialEq)]
pub struct Request {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub json: Option<Value>,
}

impl Request {
    pub fn get(url: &str, headers: Vec<(&'static str, String)>) -> Self {
        Self { method: "GET", url: url.into(), headers, json: None }
    }

    pub fn post_json(url: &str, headers: Vec<(&'static str, String)>, body: Value) -> Self {
        Self { method: "POST", url: url.into(), headers, json: Some(body) }
    }
}

/// What came back, whatever the status (the caller decides what it means).
#[derive(Clone, Default, PartialEq)]
pub struct Response {
    pub status: u16,
    pub body: String,
    /// Each `Set-Cookie` line.
    pub set_cookies: Vec<String>,
    pub location: Option<String>,
}

// Neither prints a header's value, a cookie or the body: they hold the
// session and its tokens.
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let headers: Vec<&str> = self.headers.iter().map(|(k, _)| *k).collect();
        f.debug_struct("Request").field("method", &self.method).field("url", &self.url).field("headers", &headers).finish()
    }
}

impl std::fmt::Debug for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Response")
            .field("status", &self.status)
            .field("body_len", &self.body.len())
            .field("set_cookies", &self.set_cookies.len())
            .field("location", &self.location.as_deref().map(|l| l.split(['?', '#']).next().unwrap_or("")))
            .finish()
    }
}

pub trait Http: Send + Sync {
    fn send(&self, req: Request) -> Result<Response>;
}

/// The network, through ureq. Redirects are not followed here:
/// [`super::api::token`] follows the ones that stay on `accounts.ea.com`,
/// with the cookies each one sets.
pub struct Net {
    agent: ureq::Agent,
}

impl Default for Net {
    fn default() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .max_redirects(0)
            .user_agent(concat!("Heronite/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self { agent }
    }
}

impl Http for Net {
    fn send(&self, req: Request) -> Result<Response> {
        let host = url::Url::parse(&req.url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
        // Cookies and tokens travel in headers only; errors name the host.
        let fail = |e: ureq::Error| anyhow!("{host}: {e}");
        let mut resp = match &req.json {
            None => {
                let mut r = self.agent.get(&req.url);
                for (k, v) in &req.headers {
                    r = r.header(*k, v);
                }
                r.call().map_err(fail)?
            }
            Some(body) => {
                let mut r = self.agent.post(&req.url).header("Content-Type", "application/json");
                for (k, v) in &req.headers {
                    r = r.header(*k, v);
                }
                r.send(body.to_string()).map_err(fail)?
            }
        };
        let headers = resp.headers();
        let set_cookies = headers.get_all("set-cookie").iter().filter_map(|v| v.to_str().ok()).map(str::to_string).collect();
        let location = headers.get("location").and_then(|v| v.to_str().ok()).map(str::to_string);
        let status = resp.status().as_u16();
        let body = resp.body_mut().read_to_string().map_err(fail)?;
        Ok(Response { status, body, set_cookies, location })
    }
}

/// Answers from a list, in order, and keeps the requests (tests).
#[cfg(test)]
#[derive(Default)]
pub struct Fake {
    pub answers: std::sync::Mutex<std::collections::VecDeque<Response>>,
    pub sent: std::sync::Mutex<Vec<Request>>,
}

#[cfg(test)]
impl Fake {
    pub fn new(answers: Vec<Response>) -> Self {
        let fake = Self::default();
        fake.answers.lock().unwrap().extend(answers);
        fake
    }

    pub fn sent(&self) -> Vec<Request> {
        self.sent.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl Http for Fake {
    fn send(&self, req: Request) -> Result<Response> {
        self.sent.lock().unwrap().push(req);
        self.answers.lock().unwrap().pop_front().ok_or_else(|| anyhow!("no answer left"))
    }
}

/// A plain answer (tests).
#[cfg(test)]
pub fn answer(status: u16, body: &str) -> Response {
    Response { status, body: body.into(), ..Default::default() }
}
