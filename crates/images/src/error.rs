use std::fmt;

/// Cloneable error so one failed load can be fanned out to several waiters.
#[derive(Debug, Clone)]
pub enum ImageError {
    /// The candidate chain was empty.
    NoCandidates,
    /// Filesystem error (message only, so the error stays `Clone`).
    Io(String),
    /// Download failed (network, timeout, HTTP status).
    Http(String),
    /// Bytes are not a supported raster image (e.g. an HTML 404 page).
    NotAnImage(String),
    /// Decoder rejected the data.
    Decode(String),
    /// Every candidate in the chain failed; one message per candidate.
    AllFailed(Vec<String>),
    /// Worker panicked while loading.
    Panic(String),
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCandidates => write!(f, "no image candidates"),
            Self::Io(m) => write!(f, "io: {m}"),
            Self::Http(m) => write!(f, "http: {m}"),
            Self::NotAnImage(m) => write!(f, "not an image: {m}"),
            Self::Decode(m) => write!(f, "decode: {m}"),
            Self::AllFailed(v) => write!(f, "all candidates failed: {}", v.join("; ")),
            Self::Panic(m) => write!(f, "panic: {m}"),
        }
    }
}

impl std::error::Error for ImageError {}

impl From<std::io::Error> for ImageError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, ImageError>;
