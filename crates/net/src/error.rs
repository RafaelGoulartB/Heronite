use std::fmt;
use std::io;
use std::time::Duration;

use heronite_i18n::tr;

#[derive(Debug)]
pub enum Error {
    /// The caller's cancel check fired.
    Cancelled,
    /// No bytes arrived for this long.
    Stalled(Duration),
    /// The server could not be reached (DNS, connect, TLS, a dropped request).
    Unreachable {
        url: String,
        detail: String,
    },
    /// HTTP 429, or 403 with a rate-limit header. `retry_after` is the
    /// server's `Retry-After` in seconds, when it sent one.
    RateLimited {
        retry_after: Option<Duration>,
    },
    Http(u16),
    /// Reading the body or writing the file failed.
    Io {
        context: String,
        source: io::Error,
    },
    /// The connection closed after `got` of the `total` bytes the server
    /// announced.
    Truncated {
        got: u64,
        total: u64,
    },
    /// The body arrived but is not what was expected: validation, size or
    /// checksum mismatch, a bad request.
    Invalid(String),
}

/// Texts for the interface (`tr!`): callers put them in their own messages.
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Cancelled => f.write_str(tr!("cancelled")),
            Error::Stalled(d) => f.write_str(&tr!("the download stalled: no data for {secs} s", secs = d.as_secs().max(1))),
            Error::Unreachable { url, detail } => f.write_str(&tr!("could not reach {url}: {detail}", url, detail)),
            Error::RateLimited { retry_after: Some(d) } => {
                f.write_str(&tr!("rate limit reached, try again in {secs} s", secs = d.as_secs()))
            }
            Error::RateLimited { retry_after: None } => f.write_str(tr!("rate limit reached, try again later")),
            Error::Http(code) => write!(f, "HTTP {code}"),
            Error::Io { context, source } => write!(f, "{context}: {source}"),
            Error::Truncated { got, total } => f.write_str(&tr!("connection closed after {got} of {total} bytes", got, total)),
            Error::Invalid(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// `url` as scheme, host, port and path only: the userinfo (`user:pass@`),
/// query and fragment can carry secrets (credentials, tokens, signatures) and
/// never reach an error message or a log.
pub fn redact_url(url: &str) -> String {
    let end = url.find(['?', '#']).unwrap_or(url.len());
    let url = &url[..end];
    let Some(scheme_end) = url.find("://").map(|i| i + 3) else { return url.to_string() };
    let authority_end = url[scheme_end..].find('/').map_or(url.len(), |i| scheme_end + i);
    match url[scheme_end..authority_end].rfind('@') {
        Some(at) => format!("{}{}", &url[..scheme_end], &url[scheme_end + at + 1..]),
        None => url.to_string(),
    }
}

/// Redacts every URL inside free text (ureq quotes the address in some errors).
pub(crate) fn scrub(text: &str) -> String {
    text.split_inclusive(char::is_whitespace)
        .map(
            |word| {
                if word.contains("://") {
                    redact_url(word.trim_end()).to_string() + &word[word.trim_end().len()..]
                } else {
                    word.to_string()
                }
            },
        )
        .collect()
}

/// `context` is already in the interface language (`tr!`).
pub(crate) fn io_error(context: impl Into<String>, source: io::Error) -> Error {
    Error::Io { context: context.into(), source }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_url_keeps_scheme_host_and_path_only() {
        assert_eq!(redact_url("https://user:pass@host/p?q=1"), "https://host/p");
        assert_eq!(redact_url("https://token@host:8443/a/b#frag"), "https://host:8443/a/b");
        assert_eq!(redact_url("http://host/p"), "http://host/p");
        assert_eq!(redact_url("https://host"), "https://host");
        assert_eq!(redact_url("https://us@er:pa@ss@host/p"), "https://host/p");
        assert_eq!(redact_url("https://host/a@b"), "https://host/a@b");
        assert_eq!(redact_url("not a url"), "not a url");
    }

    #[test]
    fn scrub_strips_userinfo_inside_text() {
        assert_eq!(scrub("failed https://u:p@host/x?t=1 again"), "failed https://host/x again");
    }
}
