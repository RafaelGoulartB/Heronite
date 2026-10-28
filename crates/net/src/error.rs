use std::fmt;
use std::io;
use std::time::Duration;

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

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Cancelled => write!(f, "cancelled"),
            Error::Stalled(d) => write!(f, "the download stalled: no data for {} s", d.as_secs().max(1)),
            Error::Unreachable { url, detail } => write!(f, "could not reach {url}: {detail}"),
            Error::RateLimited { retry_after: Some(d) } => write!(f, "rate limit reached, try again in {} s", d.as_secs()),
            Error::RateLimited { retry_after: None } => write!(f, "rate limit reached, try again later"),
            Error::Http(code) => write!(f, "HTTP {code}"),
            Error::Io { context, source } => write!(f, "{context}: {source}"),
            Error::Truncated { got, total } => write!(f, "connection closed after {got} of {total} bytes"),
            Error::Invalid(m) => write!(f, "{m}"),
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

/// `url` as scheme, host and path only: the query and fragment can carry
/// secrets (tokens, signatures) and never reach an error message or a log.
pub fn redact_url(url: &str) -> String {
    let end = url.find(['?', '#']).unwrap_or(url.len());
    url[..end].to_string()
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

pub(crate) fn io_error(context: impl Into<String>, source: io::Error) -> Error {
    Error::Io { context: context.into(), source }
}
