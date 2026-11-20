//! A list of extensions (`index.json`): a small JSON document naming
//! extensions and where their two files are, with the SHA-256 of each, so
//! an entry is installed only when the files downloaded are the ones the
//! list names. The repository's `extensions/metadata/index.json` is the app's own
//! list (made by `scripts/extensions-index.py`); a user may add others.
//!
//! A list says what kind of extensions it holds in `"type"`. Only
//! `"metadata"` exists; a list without the key is read as one, so a
//! hand-written list still works, and a list of another type is refused.
//!
//! Read strictly: an unknown `index` version, an address that is not
//! `https`, an id that is not valid or is a built-in source's, a file over
//! its size, or an entry repeated refuses the whole list. Unknown keys are
//! left out, so a later minor addition still reads. The format is
//! documented in `docs/EXTENSIONS.md`.

use heronite_data::settings::ALL_FIELDS;
use heronite_metadata::sources::{is_reserved_id, AppliesTo, Asked, GameKind};
use heronite_metadata::SourceKind;
use serde_json::{Map, Value};

use crate::manifest::{applies_of, check_text, valid_id, API_VERSION};
use crate::net::HostRule;
use crate::package::{MANIFEST, SCRIPT};

/// The `"type"` of a list of metadata extensions: the only one there is,
/// and the one a list without a `"type"` is read as.
pub const LIST_TYPE: &str = "metadata";
/// The version of the list format this build reads.
pub const INDEX_VERSION: u32 = 1;
/// The largest list read.
pub const MAX_INDEX_BYTES: u64 = 1024 * 1024;
/// The most entries a list may have.
pub const MAX_ENTRIES: usize = 500;
/// The largest `source.toml` and `source.js` a list may name (the
/// package's own limits).
pub const MAX_MANIFEST: u64 = 64 * 1024;
pub const MAX_SCRIPT: u64 = 512 * 1024;

/// Why a list cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexError {
    /// Not JSON, or not an object.
    NotJson(String),
    /// Written for a later version of the format than this build reads.
    NewerFormat(u64),
    /// A key is missing or wrong; the text says where.
    Invalid(String),
    /// A list of another type of extensions than metadata (its `"type"`).
    WrongType(String),
}

impl std::fmt::Display for IndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotJson(m) => write!(f, "not a list of extensions: {m}"),
            Self::NewerFormat(n) => write!(f, "a list of format {n}; this build reads format {INDEX_VERSION}"),
            Self::Invalid(m) => write!(f, "{m}"),
            Self::WrongType(t) => write!(f, "a list of {t:?} extensions; this build reads lists of {LIST_TYPE:?}"),
        }
    }
}

impl std::error::Error for IndexError {}

/// One of an entry's two files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexFile {
    /// An `https` address.
    pub url: String,
    /// Lowercase hex.
    pub sha256: String,
    /// Bytes.
    pub size: u64,
}

/// A setting the extension asks for, as the list names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexSetting {
    pub label: String,
    pub required: bool,
}

/// One extension of a list: what its manifest says, copied so the gallery
/// shows it before anything is downloaded, and its two files.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexEntry {
    pub id: String,
    pub name: String,
    pub version: semver::Version,
    /// The contract version it is written for; one above
    /// [`API_VERSION`] is listed but cannot be installed.
    pub api: u32,
    pub author: String,
    pub kind: SourceKind,
    pub description: String,
    /// The fields it fills (`ALL_FIELDS` ids; others are left out).
    pub fields: Vec<String>,
    pub applies_to: AppliesTo,
    pub asked: Asked,
    pub game_kind: Option<GameKind>,
    /// As a manifest writes them (`*.example.org`, `http://old.example.org`).
    pub hosts: Vec<String>,
    /// Further hosts its pictures come from.
    pub images: Vec<String>,
    pub adult: bool,
    pub homepage: Option<String>,
    pub settings: Vec<IndexSetting>,
    pub manifest: IndexFile,
    pub script: IndexFile,
}

impl IndexEntry {
    /// Whether this build can install it.
    pub fn supported(&self) -> bool {
        self.api <= API_VERSION
    }
}

/// A list, read and checked.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Index {
    /// What the list calls itself, when it says.
    pub name: Option<String>,
    pub entries: Vec<IndexEntry>,
}

impl Index {
    pub fn get(&self, id: &str) -> Option<&IndexEntry> {
        self.entries.iter().find(|e| e.id == id)
    }
}

fn invalid(msg: impl Into<String>) -> IndexError {
    IndexError::Invalid(msg.into())
}

/// Whether `url` is an `https` address with a host and no user or password.
pub fn https_url(url: &str) -> bool {
    match url::Url::parse(url) {
        Ok(u) => u.scheme() == "https" && u.host_str().is_some_and(|h| !h.is_empty()) && u.username().is_empty() && u.password().is_none(),
        Err(_) => false,
    }
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Reads the keys of one entry, naming the entry in every error.
struct Entry<'a> {
    map: &'a Map<String, Value>,
    at: String,
}

impl Entry<'_> {
    fn err(&self, msg: impl std::fmt::Display) -> IndexError {
        invalid(format!("{}: {msg}", self.at))
    }

    fn text(&self, key: &str, max: usize) -> Result<String, IndexError> {
        match self.map.get(key) {
            Some(Value::String(s)) => {
                let s = s.trim().to_owned();
                check_text(key, &s, max).map_err(|e| self.err(e))?;
                Ok(s)
            }
            Some(_) => Err(self.err(format!("{key} must be text"))),
            None => Err(self.err(format!("{key} is missing"))),
        }
    }

    fn strings(&self, key: &str) -> Result<Vec<String>, IndexError> {
        match self.map.get(key) {
            None => Ok(Vec::new()),
            Some(Value::Array(items)) => items
                .iter()
                .map(|v| v.as_str().map(|s| s.trim().to_owned()).ok_or_else(|| self.err(format!("{key} must be a list of text"))))
                .collect(),
            Some(_) => Err(self.err(format!("{key} must be a list of text"))),
        }
    }

    fn hosts(&self, key: &str) -> Result<Vec<String>, IndexError> {
        let list = self.strings(key)?;
        if list.len() > 16 {
            return Err(self.err(format!("{key} lists more than 16 hosts")));
        }
        list.iter().map(|h| HostRule::parse(h).map(|r| r.text()).map_err(|e| self.err(format!("{key}: {e}")))).collect()
    }

    fn file(&self, name: &str, max: u64) -> Result<IndexFile, IndexError> {
        let files = self.map.get("files").and_then(Value::as_object).ok_or_else(|| self.err("files is missing"))?;
        let file = files.get(name).and_then(Value::as_object).ok_or_else(|| self.err(format!("files.{name} is missing")))?;
        let url = file.get("url").and_then(Value::as_str).unwrap_or_default().trim().to_owned();
        if !https_url(&url) {
            return Err(self.err(format!("files.{name}.url \"{url}\" is not an https address")));
        }
        let sha256 = file.get("sha256").and_then(Value::as_str).unwrap_or_default().trim().to_ascii_lowercase();
        if !is_sha256(&sha256) {
            return Err(self.err(format!("files.{name}.sha256 is not a SHA-256 in hex")));
        }
        let size = file.get("size").and_then(Value::as_u64).ok_or_else(|| self.err(format!("files.{name}.size is missing")))?;
        if size == 0 || size > max {
            return Err(self.err(format!("files.{name} is {size} bytes; it may be 1 to {max}")));
        }
        Ok(IndexFile { url, sha256, size })
    }
}

fn entry(value: &Value, i: usize) -> Result<IndexEntry, IndexError> {
    let map = value.as_object().ok_or_else(|| invalid(format!("entry {} is not an object", i + 1)))?;
    let id = map.get("id").and_then(Value::as_str).unwrap_or_default().trim().to_owned();
    let e = Entry { map, at: if id.is_empty() { format!("entry {}", i + 1) } else { id.clone() } };
    if !valid_id(&id) {
        return Err(e.err("the id must be 2 to 40 lowercase letters, digits and single dashes"));
    }
    if is_reserved_id(&id) {
        return Err(e.err("the id is a built-in source's"));
    }
    let version_text = e.text("version", 64)?;
    let version = semver::Version::parse(&version_text).map_err(|err| e.err(format!("version \"{version_text}\": {err}")))?;
    let api =
        map.get("api").and_then(Value::as_u64).filter(|n| *n >= 1).ok_or_else(|| e.err("api must be a contract version (1 or more)"))?;
    let kind = match map.get("kind").and_then(Value::as_str) {
        Some("details") => SourceKind::Details,
        Some("pictures") => SourceKind::Pictures,
        Some("extras") => SourceKind::Extras,
        _ => return Err(e.err("kind must be details, pictures or extras")),
    };
    let fields: Vec<String> = e.strings("fields")?.into_iter().filter(|f| ALL_FIELDS.contains(&f.as_str())).collect();
    if fields.is_empty() {
        return Err(e.err("fields names no field this app knows"));
    }
    let homepage = match map.get("homepage") {
        None | Some(Value::Null) => None,
        Some(Value::String(h)) if https_url(h.trim()) => Some(h.trim().to_owned()),
        Some(_) => return Err(e.err("homepage is not an https address")),
    };
    let applies = match map.get("applies") {
        None | Some(Value::Null) => None,
        Some(v @ Value::Object(_)) => match toml::Value::try_from(v) {
            Ok(toml::Value::Table(t)) => Some(t),
            _ => return Err(e.err("applies cannot be read")),
        },
        Some(_) => return Err(e.err("applies must be an object")),
    };
    let (applies_to, asked, game_kind) = applies_of(applies.as_ref(), &mut Vec::new()).map_err(|err| e.err(err))?;
    let settings = match map.get("settings") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) if items.len() <= 16 => items
            .iter()
            .map(|s| {
                let label = s.get("label").and_then(Value::as_str).map(str::trim).filter(|l| !l.is_empty() && l.chars().count() <= 60);
                let label = label.ok_or_else(|| e.err("a setting has no label (60 characters at most)"))?;
                Ok(IndexSetting { label: label.to_owned(), required: s.get("required").and_then(Value::as_bool).unwrap_or(false) })
            })
            .collect::<Result<_, IndexError>>()?,
        Some(_) => return Err(e.err("settings must be a list of 16 at most")),
    };
    Ok(IndexEntry {
        name: e.text("name", 40)?,
        version,
        api: u32::try_from(api).unwrap_or(u32::MAX),
        author: e.text("author", 80)?,
        kind,
        description: e.text("description", 300)?,
        fields,
        applies_to,
        asked,
        game_kind,
        hosts: e.hosts("hosts")?,
        images: e.hosts("images")?,
        adult: map.get("adult").and_then(Value::as_bool).unwrap_or(false),
        homepage,
        settings,
        manifest: e.file(MANIFEST, MAX_MANIFEST)?,
        script: e.file(SCRIPT, MAX_SCRIPT)?,
        id,
    })
}

/// Reads and checks a list.
pub fn parse(bytes: &[u8]) -> Result<Index, IndexError> {
    if bytes.len() as u64 > MAX_INDEX_BYTES {
        return Err(invalid(format!("the list is larger than {MAX_INDEX_BYTES} bytes")));
    }
    let doc: Value = serde_json::from_slice(bytes).map_err(|e| IndexError::NotJson(e.to_string()))?;
    let top = doc.as_object().ok_or_else(|| IndexError::NotJson("not an object".into()))?;
    match top.get("index").and_then(Value::as_u64) {
        Some(n) if n == u64::from(INDEX_VERSION) => {}
        Some(n) if n > u64::from(INDEX_VERSION) => return Err(IndexError::NewerFormat(n)),
        _ => return Err(IndexError::NotJson(format!("\"index\" must be {INDEX_VERSION}"))),
    }
    match top.get("type") {
        None => {}
        Some(Value::String(t)) if t == LIST_TYPE => {}
        Some(Value::String(t)) => return Err(IndexError::WrongType(t.clone())),
        Some(other) => return Err(IndexError::WrongType(other.to_string())),
    }
    let name = top.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).map(|n| n.chars().take(60).collect());
    let items = top.get("extensions").and_then(Value::as_array).ok_or_else(|| invalid("\"extensions\" is missing"))?;
    if items.len() > MAX_ENTRIES {
        return Err(invalid(format!("more than {MAX_ENTRIES} extensions")));
    }
    let mut entries: Vec<IndexEntry> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let e = entry(item, i)?;
        if entries.iter().any(|x| x.id == e.id) {
            return Err(invalid(format!("{}: listed twice", e.id)));
        }
        entries.push(e);
    }
    Ok(Index { name, entries })
}

#[cfg(test)]
#[path = "index_tests.rs"]
mod tests;
