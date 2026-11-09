//! The requests to Steam's Web API, behind [`Http`] so the tests answer them
//! from a list ([`Fake`]) and nothing touches the network then.

use std::time::Duration;

use anyhow::{anyhow, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: &'static str,
    pub url: String,
    /// Form fields of a POST.
    pub form: Vec<(&'static str, String)>,
}

impl Request {
    pub fn get(url: String) -> Self {
        Self { method: "GET", url, form: Vec::new() }
    }

    pub fn post(url: String, form: Vec<(&'static str, String)>) -> Self {
        Self { method: "POST", url, form }
    }
}

/// An answer, whatever its status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    /// Steam's `x-eresult` header (`1`: OK); the authentication calls
    /// answer 200 with an empty body and say what went wrong only here.
    pub eresult: Option<i32>,
    pub body: String,
}

pub trait Http: Send + Sync {
    fn send(&self, req: Request) -> Result<Response>;
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
    fn send(&self, req: Request) -> Result<Response> {
        // Tokens travel in the query and the form: errors name the host only.
        let host = req.url.split("://").nth(1).and_then(|r| r.split(['/', '?']).next()).unwrap_or_default().to_string();
        let fail = |e: ureq::Error| anyhow!("{host}: {e}");
        let mut resp = match req.method {
            "POST" => self.agent.post(&req.url).send_form(req.form.iter().map(|(k, v)| (*k, v.as_str()))).map_err(fail)?,
            _ => self.agent.get(&req.url).call().map_err(fail)?,
        };
        let eresult = resp.headers().get("x-eresult").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok());
        let status = resp.status().as_u16();
        let body = resp.body_mut().read_to_string().map_err(fail)?;
        Ok(Response { status, eresult, body })
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
    /// `(status, eresult, body)` per request.
    pub fn new(answers: &[(u16, i32, &str)]) -> Self {
        let fake = Self::default();
        fake.answers.lock().unwrap().extend(answers.iter().map(|(status, eresult, body)| Response {
            status: *status,
            eresult: Some(*eresult),
            body: body.to_string(),
        }));
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
