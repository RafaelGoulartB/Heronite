use std::fmt;

/// Cloneable error so one failed load can be fanned out to several waiters.
#[derive(Debug, Clone)]
pub enum ImageError {
    /// The candidate chain was empty.
    NoCandidates,
    /// Filesystem error (message only, so the error stays `Clone`).
    Io(String),
    /// Download failed without an HTTP answer (network, timeout, TLS, ...).
    Http(String),
    /// The server answered with an error status (code, url).
    HttpStatus(u16, String),
    /// Bytes are not a supported raster image (e.g. an HTML 404 page).
    NotAnImage(String),
    /// Decoder rejected the data.
    Decode(String),
    /// Every candidate in the chain failed: one message per candidate, and
    /// whether at least one of them may work if tried again later.
    AllFailed { errors: Vec<String>, transient: bool },
    /// Worker panicked while loading.
    Panic(String),
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCandidates => write!(f, "no image candidates"),
            Self::Io(m) => write!(f, "io: {m}"),
            Self::Http(m) => write!(f, "http: {m}"),
            Self::HttpStatus(code, url) => write!(f, "http: {url}: status {code}"),
            Self::NotAnImage(m) => write!(f, "not an image: {m}"),
            Self::Decode(m) => write!(f, "decode: {m}"),
            Self::AllFailed { errors, .. } => write!(f, "all candidates failed: {}", errors.join("; ")),
            Self::Panic(m) => write!(f, "panic: {m}"),
        }
    }
}

impl std::error::Error for ImageError {}

impl ImageError {
    /// True when trying again later may succeed (offline, timeout, rate
    /// limit, server error, I/O); false when the source itself is bad (404,
    /// not an image, undecodable) and a retry would fail the same way.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Io(_) | Self::Http(_) | Self::Panic(_) => true,
            Self::HttpStatus(code, _) => !status_is_definitive(*code),
            Self::AllFailed { transient, .. } => *transient,
            Self::NoCandidates | Self::NotAnImage(_) | Self::Decode(_) => false,
        }
    }
}

/// True for an HTTP status that says this URL has no usable image, so it is
/// safe to remember the failure: the client errors, except the ones that
/// only mean "not now": 408 (request timeout), 425 (too early) and 429 (rate
/// limited). 403 stays definitive: image CDNs (S3, CloudFront) answer 403 for
/// a missing object. 5xx and everything else is transient.
pub(crate) fn status_is_definitive(code: u16) -> bool {
    (400..500).contains(&code) && !matches!(code, 408 | 425 | 429)
}

impl From<std::io::Error> for ImageError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, ImageError>;
