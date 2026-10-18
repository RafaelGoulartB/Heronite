//! The thin HTTP layer for GOG's web APIs. Callers build URLs and read JSON in
//! pure functions of their own, so only these few calls touch the network.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};

/// A finished request; non-2xx statuses are results, not errors.
#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub etag: Option<String>,
    pub body: String,
}

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(15))).max_redirects(8).user_agent("Heronite").build().into()
    })
}

fn send(url: &str, headers: &[(&str, &str)]) -> Result<ureq::http::Response<ureq::Body>> {
    let mut req = agent().get(url);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    req.config()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(40)))
        .build()
        .call()
        .with_context(|| format!("GET {url}"))
}

fn etag_of(resp: &ureq::http::Response<ureq::Body>) -> Option<String> {
    resp.headers().get("etag").and_then(|v| v.to_str().ok()).map(str::to_string)
}

/// GET with the body read as text.
pub fn get(url: &str, headers: &[(&str, &str)]) -> Result<Response> {
    let mut resp = send(url, headers)?;
    let status = resp.status().as_u16();
    let etag = etag_of(&resp);
    let body = if status == 304 { String::new() } else { resp.body_mut().with_config().limit(64 * 1024 * 1024).read_to_string()? };
    Ok(Response { status, etag, body })
}

/// GET that only looks at the status and `ETag`: the body (a whole build
/// manifest) is never downloaded.
pub fn head_like(url: &str, headers: &[(&str, &str)]) -> Result<(u16, Option<String>)> {
    let resp = send(url, headers)?;
    Ok((resp.status().as_u16(), etag_of(&resp)))
}

/// `Authorization` header value.
pub fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

/// Bearer GET expecting a JSON body on success.
pub fn get_json(url: &str, token: &str) -> Result<serde_json::Value> {
    let auth = bearer(token);
    let resp = get(url, &[("Authorization", &auth)])?;
    if resp.status == 401 {
        // The token gogdl handed out is refused: the account must sign in again.
        return Err(anyhow::anyhow!(heronite_epic::session::SessionExpired).context(format!("{url}: HTTP 401")));
    }
    if resp.status / 100 != 2 {
        anyhow::bail!("{url}: HTTP {}", resp.status);
    }
    serde_json::from_str(&resp.body).with_context(|| format!("invalid JSON from {url}"))
}

/// Percent-encodes a query value.
pub fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `f` over `items` on up to `workers` threads, results in item order.
pub fn parallel_map<T: Sync, R: Send>(items: &[T], workers: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<R>>> = Mutex::new(items.iter().map(|_| None).collect());
    std::thread::scope(|scope| {
        for _ in 0..workers.clamp(1, items.len().max(1)) {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some(item) = items.get(i) else { break };
                let r = f(item);
                results.lock().unwrap_or_else(|e| e.into_inner())[i] = Some(r);
            });
        }
    });
    results.into_inner().unwrap_or_else(|e| e.into_inner()).into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(percent_encode("abc-1_2.3~"), "abc-1_2.3~");
        assert_eq!(percent_encode("a b/c=d+"), "a%20b%2Fc%3Dd%2B");
    }

    #[test]
    fn parallel_map_keeps_order() {
        let items: Vec<u32> = (0..50).collect();
        let out = parallel_map(&items, 8, |n| n * 2);
        assert_eq!(out, items.iter().map(|n| n * 2).collect::<Vec<_>>());
        assert!(parallel_map(&Vec::<u32>::new(), 4, |n| *n).is_empty());
    }
}
