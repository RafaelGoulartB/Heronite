//! The requests this crate sends, behind [`Http`] so the tests answer them
//! from fixtures ([`Fake`]) and nothing here touches the network then.

use std::time::Duration;

use anyhow::{anyhow, Result};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    None,
    Form(Vec<(&'static str, String)>),
    Json(Value),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Body,
}

impl Request {
    pub fn get(url: &str, headers: Vec<(&'static str, String)>) -> Self {
        Self { method: "GET", url: url.into(), headers, body: Body::None }
    }

    pub fn post_json(url: &str, headers: Vec<(&'static str, String)>, body: Value) -> Self {
        Self { method: "POST", url: url.into(), headers, body: Body::Json(body) }
    }

    pub fn post_form(url: &str, form: Vec<(&'static str, String)>) -> Self {
        Self { method: "POST", url: url.into(), headers: Vec::new(), body: Body::Form(form) }
    }
}

/// Sends a request; the status and the body's text come back whatever the
/// status (the caller decides what an error status means).
pub trait Http: Send + Sync {
    fn send(&self, req: Request) -> Result<(u16, String)>;
}

/// The network, through ureq.
pub struct Net {
    agent: ureq::Agent,
}

impl Default for Net {
    fn default() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .user_agent(concat!("Heronite/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self { agent }
    }
}

impl Http for Net {
    fn send(&self, req: Request) -> Result<(u16, String)> {
        let host = url::Url::parse(&req.url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
        // Tokens travel in headers and bodies only; errors name the host.
        let fail = |e: ureq::Error| anyhow!("{host}: {e}");
        let mut resp = match (&req.body, req.method) {
            (Body::None, _) => {
                let mut r = self.agent.get(&req.url);
                for (k, v) in &req.headers {
                    r = r.header(*k, v);
                }
                r.call().map_err(fail)?
            }
            (Body::Json(v), _) => {
                let mut r = self.agent.post(&req.url).header("Content-Type", "application/json");
                for (k, h) in &req.headers {
                    r = r.header(*k, h);
                }
                r.send(v.to_string()).map_err(fail)?
            }
            (Body::Form(form), _) => {
                let mut r = self.agent.post(&req.url);
                for (k, h) in &req.headers {
                    r = r.header(*k, h);
                }
                r.send_form(form.iter().map(|(k, v)| (*k, v.as_str()))).map_err(fail)?
            }
        };
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().map_err(fail)?;
        Ok((status, text))
    }
}

/// Answers from a list, in order, and keeps the requests (tests).
#[cfg(test)]
#[derive(Default)]
pub struct Fake {
    pub answers: std::sync::Mutex<std::collections::VecDeque<(u16, String)>>,
    pub sent: std::sync::Mutex<Vec<Request>>,
}

#[cfg(test)]
impl Fake {
    pub fn new(answers: &[(u16, &str)]) -> Self {
        let fake = Self::default();
        fake.answers.lock().unwrap().extend(answers.iter().map(|(s, b)| (*s, b.to_string())));
        fake
    }

    pub fn sent(&self) -> Vec<Request> {
        self.sent.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl Http for Fake {
    fn send(&self, req: Request) -> Result<(u16, String)> {
        self.sent.lock().unwrap().push(req);
        self.answers.lock().unwrap().pop_front().ok_or_else(|| anyhow!("no answer left"))
    }
}
