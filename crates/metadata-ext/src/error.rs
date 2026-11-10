//! Why a package cannot be inspected, installed or loaded. The interface
//! words each case; the text is technical detail for the author.

/// A problem with an extension's package or files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionError {
    /// Neither a folder nor a zip file, or it cannot be read.
    NotAPackage(String),
    /// A file, or the whole package, is over its size limit.
    TooLarge(String),
    /// A file a package may not have (another name, a folder, a link).
    UnexpectedFile(String),
    /// A path that leaves the package (`..`, an absolute path).
    UnsafePath(String),
    /// `source.toml` or `source.js` is missing.
    MissingFile(String),
    /// `source.toml` is wrong; the text says where.
    Manifest(String),
    /// It is written for a newer contract than this build speaks.
    NewerApi(u32),
    /// Its id is a built-in source's.
    IdTaken(String),
    /// `source.js` does not compile, load or export `find` and `fetch`.
    Script(String),
    /// No extension with this id is installed.
    NotInstalled(String),
    /// Writing to the data folder failed.
    Io(String),
    /// A file of a list's entry could not be downloaded (no answer, or an
    /// error status); the text says which and why.
    Download(String),
    /// A downloaded file is not the one its list names: its SHA-256
    /// differs. The text names the file.
    Checksum(String),
    /// The package is not what its list's entry says (another id, version
    /// or kind, a host the entry does not list).
    NotAsListed(String),
    /// The package found when installing is not the one the installer
    /// showed (its SHA-256 differs): the file changed in between.
    Changed,
}

impl std::fmt::Display for ExtensionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAPackage(m) => write!(f, "not an extension package: {m}"),
            Self::TooLarge(m) => write!(f, "too large: {m}"),
            Self::UnexpectedFile(m) => write!(f, "unexpected file: {m}"),
            Self::UnsafePath(m) => write!(f, "unsafe path: {m}"),
            Self::MissingFile(m) => write!(f, "missing file: {m}"),
            Self::Manifest(m) => write!(f, "source.toml: {m}"),
            Self::NewerApi(n) => {
                write!(f, "made for a newer Heronite (contract version {n}; this build supports {})", crate::manifest::API_VERSION)
            }
            Self::IdTaken(id) => write!(f, "the id \"{id}\" belongs to a built-in source"),
            Self::Script(m) => write!(f, "source.js: {m}"),
            Self::NotInstalled(id) => write!(f, "no extension \"{id}\" is installed"),
            Self::Io(m) => write!(f, "could not write the extension: {m}"),
            Self::Download(m) => write!(f, "could not download: {m}"),
            Self::Checksum(m) => write!(f, "{m} is not the file the list names (SHA-256 differs)"),
            Self::NotAsListed(m) => write!(f, "not what the list says: {m}"),
            Self::Changed => write!(f, "the package changed after it was shown (SHA-256 differs)"),
        }
    }
}

impl std::error::Error for ExtensionError {}
