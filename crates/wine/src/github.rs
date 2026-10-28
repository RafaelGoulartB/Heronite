//! HTTP towards GitHub: the releases API (with its rate-limit headers), the
//! public release feed and plain downloads (checksums).
//!
//! Anonymous API calls are limited to 60 an hour per IP address, shared with
//! every other program on the network; `checks` decides when to spend one.
//! The feed is a page of the website, not part of the API.

use std::time::Duration;

use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};

use crate::repos::Repository;

fn agent() -> &'static ureq::Agent {
    use std::sync::OnceLock;
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(15)))
            .max_redirects(8)
            .user_agent("heronite-rs")
            .build()
            .into()
    })
}

/// What GitHub said about the API budget of this address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub remaining: u32,
    /// When the budget is refilled (Unix seconds).
    pub reset_at: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ApiError {
    /// No call may be made before `until` (Unix seconds).
    RateLimited {
        until: u64,
    },
    Other(String),
}

/// One API call: the body (or why there is none) and the budget it reported.
pub struct ApiReply {
    pub result: std::result::Result<String, ApiError>,
    pub budget: Option<Budget>,
}

struct Reply {
    status: u16,
    header: Box<dyn Fn(&str) -> Option<String>>,
    body: Result<String>,
}

fn get(url: &str, accept: &str, max: u64) -> Result<Reply> {
    let mut resp = agent()
        .get(url)
        .header("Accept", accept)
        .config()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .build()
        .call()
        .map_err(|e| anyhow!("{e}"))?;
    let headers = resp.headers().clone();
    let header = Box::new(move |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(|v| v.trim().to_owned()));
    let status = resp.status().as_u16();
    let body = resp.body_mut().with_config().limit(max).read_to_string().map_err(|e| anyhow!("{e}"));
    Ok(Reply { status, header, body })
}

/// Reads the rate-limit headers of an API reply. `now`: Unix seconds.
fn api_outcome(status: u16, header: &dyn Fn(&str) -> Option<String>, body: Result<String>, now: u64) -> ApiReply {
    let num = |name: &str| header(name).and_then(|v| v.parse::<u64>().ok());
    let budget = match (num("x-ratelimit-remaining"), num("x-ratelimit-reset")) {
        (Some(remaining), Some(reset_at)) => Some(Budget { remaining: remaining.min(u32::MAX as u64) as u32, reset_at }),
        _ => None,
    };
    let result = match status {
        200..=299 => body.map_err(|e| ApiError::Other(e.to_string())),
        403 | 429 => {
            // The primary limit says when it resets; the secondary one sends
            // `retry-after`. A 403 with budget left is a plain refusal.
            let until = match (num("retry-after"), budget) {
                (Some(secs), _) => Some(now + secs),
                (None, Some(b)) if b.remaining == 0 => Some(b.reset_at),
                (None, None) if status == 429 => Some(now + 60),
                _ => None,
            };
            match until {
                Some(until) => Err(ApiError::RateLimited { until: until.max(now + 1) }),
                None => Err(ApiError::Other(format!("HTTP {status}"))),
            }
        }
        _ => Err(ApiError::Other(format!("HTTP {status}"))),
    };
    ApiReply { result, budget }
}

/// The newest `count` releases of `repo` (one API call).
pub fn api_releases(repo: Repository, count: usize, now: u64) -> ApiReply {
    let url = format!("{}?per_page={count}", repo.api_url());
    match get(&url, "application/vnd.github+json", 4 * 1024 * 1024) {
        Ok(r) => api_outcome(r.status, &*r.header, r.body, now),
        Err(e) => ApiReply { result: Err(ApiError::Other(e.to_string())), budget: None },
    }
}

/// The release feed of `repo`.
pub fn feed(repo: Repository) -> Result<String> {
    let r = get(&repo.feed_url(), "application/atom+xml", 2 * 1024 * 1024)?;
    match r.status {
        200..=299 => r.body,
        s => Err(anyhow!("HTTP {s}")),
    }
}

/// What identifies the state of a release feed: the id and last update of
/// every entry. Changes when a release (or a tag) is published, edited or
/// deleted. `None` when `xml` is not a feed.
pub fn feed_fingerprint(xml: &str) -> Option<String> {
    if !xml.contains("<feed") {
        return None;
    }
    let mut hasher = Sha256::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<entry>") {
        let end = rest[start..].find("</entry>").map(|e| start + e).unwrap_or(rest.len());
        let entry = &rest[start..end];
        for tag in ["id", "updated"] {
            hasher.update(element(entry, tag).unwrap_or_default().as_bytes());
            hasher.update(b"\n");
        }
        rest = &rest[end..];
    }
    Some(hasher.finalize().iter().take(12).map(|b| format!("{b:02x}")).collect())
}

/// Text of the first `<tag>...</tag>` in `xml`.
fn element<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&format!("</{tag}>"))? + start;
    Some(xml[start..end].trim())
}

/// A small text file (checksums), following redirects.
pub(crate) fn http_get_text(url: &str, max: u64) -> Result<String> {
    let r = get(url, "text/plain, */*", max)?;
    match r.status {
        200..=299 => r.body,
        s => Err(anyhow!("HTTP {s}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(list: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| list.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string())
    }

    #[test]
    fn budget_is_read_from_every_reply() {
        let ok = api_outcome(200, &headers(&[("x-ratelimit-remaining", "41"), ("x-ratelimit-reset", "1700")]), Ok("[]".into()), 1000);
        assert_eq!(ok.result, Ok("[]".into()));
        assert_eq!(ok.budget, Some(Budget { remaining: 41, reset_at: 1700 }));
        assert_eq!(api_outcome(200, &headers(&[]), Ok("[]".into()), 1000).budget, None);
    }

    #[test]
    fn spent_budget_and_secondary_limits_say_when_to_retry() {
        let spent = api_outcome(403, &headers(&[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "1900")]), Ok("{}".into()), 1000);
        assert_eq!(spent.result, Err(ApiError::RateLimited { until: 1900 }));
        let secondary = api_outcome(403, &headers(&[("retry-after", "120"), ("x-ratelimit-remaining", "30")]), Ok("{}".into()), 1000);
        assert_eq!(secondary.result, Err(ApiError::RateLimited { until: 1120 }));
        assert_eq!(api_outcome(429, &headers(&[]), Ok(String::new()), 1000).result, Err(ApiError::RateLimited { until: 1060 }));
        // Forbidden for another reason: not a limit.
        let refused = api_outcome(403, &headers(&[("x-ratelimit-remaining", "30"), ("x-ratelimit-reset", "1900")]), Ok("{}".into()), 1000);
        assert_eq!(refused.result, Err(ApiError::Other("HTTP 403".into())));
        assert_eq!(api_outcome(500, &headers(&[]), Ok(String::new()), 1000).result, Err(ApiError::Other("HTTP 500".into())));
    }

    const FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <id>tag:github.com,2008:https://github.com/o/r/releases</id>
  <updated>2026-09-29T21:34:27Z</updated>
  <entry>
    <id>tag:github.com,2008:Repository/1/v2</id>
    <updated>2026-09-30T13:14:54Z</updated>
    <title>v2</title>
  </entry>
  <entry>
    <id>tag:github.com,2008:Repository/1/v1</id>
    <updated>2026-09-01T10:00:00Z</updated>
  </entry>
</feed>"#;

    #[test]
    fn feed_fingerprint_follows_its_entries() {
        let base = feed_fingerprint(FEED).unwrap();
        assert_eq!(feed_fingerprint(FEED).unwrap(), base);
        // The feed's own date changes on its own: ignored.
        assert_eq!(feed_fingerprint(&FEED.replace("2026-09-29T21:34:27Z", "2026-10-01T00:00:00Z")).unwrap(), base);
        assert_ne!(feed_fingerprint(&FEED.replace("2026-09-30T13:14:54Z", "2026-10-02T00:00:00Z")).unwrap(), base);
        assert_ne!(feed_fingerprint(&FEED.replace("Repository/1/v2", "Repository/1/v3")).unwrap(), base);
        assert!(feed_fingerprint("<html>rate limited</html>").is_none());
        assert!(feed_fingerprint(r#"<feed xmlns="x"></feed>"#).is_some(), "a source without releases");
    }
}
