//! The lists of extensions the gallery shows ("Get more sources"): the
//! app's own list ([`HERONITE_LIST`]) and the ones the user added
//! (`settings.metadata.extensionLists`). Each is fetched with the service's
//! `Http` (so tests point it at a fixture server), checked strictly
//! (`index.rs`) and its last good copy kept in
//! `store_cache/extensions/metadata/` with the time it was read, so the
//! gallery opens offline with that copy.
//!
//! A list may also be a file on this computer ([`ListLocation::File`]: a
//! debug build's own list is the repository's
//! `extensions/metadata/index.json`, and `HERONITE_EXTENSIONS_INDEX` may
//! name one). It is read as strictly, without the network and without a kept
//! copy, and its entries' files come from the folders beside it, never from
//! their `url`s.
//!
//! Installing an entry downloads its two files (or reads them beside a
//! list on this computer), checks both SHA-256 and sizes against the list,
//! then goes through the same `inspect` → `install` path as a file: [`prepare`] gives the [`PackageInfo`] the user confirms, refusing a
//! manifest that is not what the entry says (another id, version or kind,
//! more fields, a host the entry does not list, adult while the entry is
//! not); [`install_prepared`] installs it and records the list it came
//! from. Nothing updates by itself: [`updates`] only says which installed
//! extensions their list offers in a newer version.
//!
//! Everything is blocking (network, files): call it from a worker thread.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use heronite_data::settings;
use heronite_metadata::http::{BodyTooLarge, Http, APP_UA};
use heronite_metadata::MetadataService;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::index::{self, https_url, Index, IndexEntry, IndexError, IndexFile, MAX_INDEX_BYTES};
use crate::manifest::valid_id;
use crate::package::{MANIFEST, SCRIPT};
use crate::service::{self, Approval, ExtensionsExt, InstallOutcome, InstalledExtension, PackageInfo};
use crate::ExtensionError;

/// The app's own list: `extensions/metadata/index.json` on the repository's
/// main branch (`scripts/extensions-index.py` writes it).
pub const HERONITE_LIST: &str = "https://raw.githubusercontent.com/RafaelGoulartB/Heronite/main/extensions/metadata/index.json";

/// The longest a list or one of an entry's files may take to come.
const TIMEOUT: Duration = Duration::from_secs(20);

/// Why a list could not be read now, added or removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListError {
    /// The address is not `https` (or has a user and password).
    NotHttps,
    /// No answer (offline, DNS, timeout).
    Unreachable(String),
    /// The server answered with this status (404: nothing there).
    Status(u16),
    /// Over [`MAX_INDEX_BYTES`].
    TooLarge,
    /// Not a list this build reads, or a wrong one.
    Invalid(IndexError),
    /// The list is in use already.
    AlreadyAdded,
    /// Saving the settings failed.
    Io(String),
    /// A list on this computer cannot be opened or read; the text says
    /// which file and why.
    NoFile(String),
}

impl std::fmt::Display for ListError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotHttps => write!(f, "not an https address"),
            Self::Unreachable(m) => write!(f, "no answer: {m}"),
            Self::Status(s) => write!(f, "HTTP {s}"),
            Self::TooLarge => write!(f, "larger than {MAX_INDEX_BYTES} bytes"),
            Self::Invalid(e) => write!(f, "{e}"),
            Self::AlreadyAdded => write!(f, "already in use"),
            Self::Io(m) => write!(f, "{m}"),
            Self::NoFile(m) => write!(f, "cannot read {m}"),
        }
    }
}

impl std::error::Error for ListError {}

/// One list in use, as last read.
#[derive(Debug, Clone, PartialEq)]
pub struct ListCopy {
    /// Its address, or its path for a list on this computer
    /// ([`ListLocation::key`]).
    pub url: String,
    /// The app's own list (it cannot be removed).
    pub builtin: bool,
    /// The list, read now or the copy kept from before; `None` when there
    /// is none (or the one kept no longer reads).
    pub index: Option<Index>,
    /// When `index` came from the network.
    pub read_at: Option<SystemTime>,
    /// Why it could not be read now: `index`, if any, is the copy kept.
    pub error: Option<ListError>,
}

/// `cache/<hash>.json`: the last good copy of a list.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cached {
    url: String,
    /// Seconds since 1970 (UTC).
    read_at: u64,
    /// The list as it came, checked again when read.
    body: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// Where a list is read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListLocation {
    /// An address (read only when `https`).
    Address(String),
    /// An `index.json` on this computer. Its entries' files are the
    /// folders beside it (`<id>/source.toml`, `<id>/source.js`).
    File(PathBuf),
}

impl ListLocation {
    /// A list as it is written (the settings, `install.json`'s `list`,
    /// `HERONITE_EXTENSIONS_INDEX`): a `file://` address or a path is a
    /// file, anything else with `://` an address. No file is touched.
    pub fn of(text: &str) -> Self {
        let text = text.trim();
        if text.starts_with("file://") {
            if let Some(path) = url::Url::parse(text).ok().and_then(|u| u.to_file_path().ok()) {
                return Self::File(path);
            }
        }
        if text.contains("://") {
            Self::Address(text.to_owned())
        } else {
            Self::File(PathBuf::from(text))
        }
    }

    /// The text the list is known by: [`ListCopy::url`], and the `list`
    /// recorded for what is installed from it.
    pub fn key(&self) -> String {
        match self {
            Self::Address(url) => url.clone(),
            Self::File(path) => path.display().to_string(),
        }
    }
}

/// Whether the list known by `url` is a file on this computer.
pub fn is_local(url: &str) -> bool {
    matches!(ListLocation::of(url), ListLocation::File(_))
}

/// The host of `url`, for "From example.org".
pub fn host_of(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_else(|| url.to_owned())
}

/// Reads the lists named in the settings, the app's own first. `builtin`
/// is the app's list: [`HERONITE_LIST`], or what a debug build or an
/// environment switch reads instead (an address, or a path,
/// [`ListLocation::key`]).
#[derive(Debug, Clone)]
pub struct Lists {
    pub builtin: String,
}

impl Default for Lists {
    fn default() -> Self {
        Self::new(HERONITE_LIST)
    }
}

impl Lists {
    pub fn new(builtin: impl Into<String>) -> Self {
        Self { builtin: builtin.into() }
    }

    /// Whether `url` is the app's own list (also its usual address when an
    /// environment switch replaced it).
    pub fn is_builtin(&self, url: &str) -> bool {
        url == self.builtin || url == HERONITE_LIST
    }

    /// The lists in use, the app's own first: `(address, builtin)`.
    pub fn in_use(&self, service: &MetadataService) -> Vec<(String, bool)> {
        let mut out = vec![(self.builtin.clone(), true)];
        for url in service.metadata_settings().extension_lists {
            if !out.iter().any(|(u, _)| *u == url) {
                out.push((url, false));
            }
        }
        out
    }

    fn cache_file(service: &MetadataService, url: &str) -> PathBuf {
        service.paths.extension_lists_cache_dir().join(format!("{}.json", &sha256(url.as_bytes())[..16]))
    }

    fn read_cache(service: &MetadataService, url: &str, builtin: bool) -> ListCopy {
        let mut copy = ListCopy { url: url.to_owned(), builtin, index: None, read_at: None, error: None };
        let Ok(text) = std::fs::read_to_string(Self::cache_file(service, url)) else { return copy };
        let Ok(cached) = serde_json::from_str::<Cached>(&text) else { return copy };
        if cached.url != url {
            return copy;
        }
        match index::parse(cached.body.as_bytes()) {
            Ok(i) => {
                copy.index = Some(i);
                copy.read_at = Some(UNIX_EPOCH + Duration::from_secs(cached.read_at));
            }
            Err(e) => copy.error = Some(ListError::Invalid(e)),
        }
        copy
    }

    /// The lists as last read, without the network (a list on this
    /// computer is read now).
    pub fn cached(&self, service: &MetadataService) -> Vec<ListCopy> {
        self.in_use(service)
            .into_iter()
            .map(|(url, builtin)| match ListLocation::of(&url) {
                ListLocation::File(path) => local_copy(&url, builtin, &path),
                ListLocation::Address(_) => Self::read_cache(service, &url, builtin),
            })
            .collect()
    }

    /// Reads one list from the network and keeps it; when that fails, the
    /// copy kept from before with why. A list on this computer is read
    /// from its file, and no copy of it is kept.
    pub fn read(&self, service: &MetadataService, url: &str) -> ListCopy {
        let builtin = self.is_builtin(url);
        if let ListLocation::File(path) = ListLocation::of(url) {
            return local_copy(url, builtin, &path);
        }
        match fetch_list(&service.http, url) {
            Ok((body, index)) => {
                let now = SystemTime::now();
                let secs = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
                let cached = Cached { url: url.to_owned(), read_at: secs, body };
                let file = Self::cache_file(service, url);
                let written =
                    std::fs::create_dir_all(file.parent().unwrap_or(Path::new("."))).map_err(anyhow::Error::from).and_then(|_| {
                        heronite_data::temps::write_atomic(&file, serde_json::to_string(&cached)?.as_bytes(), None).map_err(Into::into)
                    });
                if let Err(e) = written {
                    log::warn!("Could not keep the list of extensions {url}: {e:#}");
                }
                ListCopy { url: url.to_owned(), builtin, index: Some(index), read_at: Some(now), error: None }
            }
            Err(e) => {
                log::info!("The list of extensions {url} could not be read: {}", crate::manifest::one_line(&e.to_string()));
                let mut copy = Self::read_cache(service, url, builtin);
                copy.error = Some(e);
                copy
            }
        }
    }

    /// Reads every list in use from the network (see [`Self::read`]).
    pub fn refresh(&self, service: &MetadataService) -> Vec<ListCopy> {
        self.in_use(service).into_iter().map(|(url, _)| self.read(service, &url)).collect()
    }

    /// Adds the list at `url` (after the others); it is read by the next
    /// [`Self::refresh`]. Returns the address as saved.
    pub fn add(&self, service: &MetadataService, url: &str) -> Result<String, ListError> {
        let url = url.trim().to_owned();
        if !https_url(&url) {
            return Err(ListError::NotHttps);
        }
        if self.in_use(service).iter().any(|(u, _)| *u == url) || self.is_builtin(&url) {
            return Err(ListError::AlreadyAdded);
        }
        let saved = url.clone();
        settings::update_metadata(&service.paths, move |m| m.extension_lists.push(saved)).map_err(|e| ListError::Io(format!("{e:#}")))?;
        Ok(url)
    }

    /// Stops reading the list at `url` and forgets its copy. The
    /// extensions installed from it stay ([`Origin::GoneList`]). The app's
    /// own list cannot be removed.
    pub fn remove(&self, service: &MetadataService, url: &str) -> Result<(), ListError> {
        if self.is_builtin(url) {
            return Ok(());
        }
        settings::update_metadata(&service.paths, |m| m.extension_lists.retain(|u| u != url))
            .map_err(|e| ListError::Io(format!("{e:#}")))?;
        let _ = std::fs::remove_file(Self::cache_file(service, url));
        Ok(())
    }

    /// Where an installed extension came from.
    pub fn origin(&self, service: &MetadataService, ext: &InstalledExtension) -> Origin {
        let urls = self.in_use(service);
        origin_among(self, &urls, ext.list.as_deref())
    }
}

/// A list on this computer, read now. No copy is kept: the file is always
/// at hand, and a copy would only hide that it changed or went.
fn local_copy(url: &str, builtin: bool, path: &Path) -> ListCopy {
    match read_list_file(path) {
        Ok((_, index)) => ListCopy { url: url.to_owned(), builtin, index: Some(index), read_at: Some(SystemTime::now()), error: None },
        Err(e) => {
            log::info!("The list of extensions {url} could not be read: {}", crate::manifest::one_line(&e.to_string()));
            ListCopy { url: url.to_owned(), builtin, index: None, read_at: None, error: Some(e) }
        }
    }
}

/// [`Lists::origin`] knowing the lists in use. Only [`HERONITE_LIST`] is
/// the Heronite list: the app's own list set to another address
/// (`HERONITE_EXTENSIONS_INDEX`) is a list like one the user added.
pub fn origin_among(lists: &Lists, in_use: &[(String, bool)], list: Option<&str>) -> Origin {
    match list {
        None => Origin::File,
        // Before the app's own: a debug build's own list is a file, and
        // what came from it is not from the Heronite list.
        Some(u) if is_local(u) => Origin::Local(u.to_owned()),
        Some(HERONITE_LIST) => Origin::Heronite,
        Some(u) if u == lists.builtin || in_use.iter().any(|(x, _)| x == u) => Origin::List(u.to_owned()),
        Some(u) => Origin::GoneList(u.to_owned()),
    }
}

/// Where an installed extension came from, which decides how the app
/// words what was checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// A file or a folder the user chose: nobody checked it.
    File,
    /// The app's own list: its files are the ones that list names.
    Heronite,
    /// A list the user added (its address).
    List(String),
    /// A list the user removed since.
    GoneList(String),
    /// A list on this computer (its path): a debug build's copy of the
    /// repository's, or one `HERONITE_EXTENSIONS_INDEX` names.
    Local(String),
}

/// GETs `url` with the app's user agent, at most `max` bytes, without
/// following redirects (a list could move to plain `http` otherwise).
/// `public`: only to a public address ([`crate::net::public_client`]), for
/// an entry's files, whose addresses whoever keeps the list chose (a test
/// serves its origins itself).
fn get(http: &Http, url: &str, max: u64, public: bool) -> Result<Vec<u8>, ListError> {
    let (to, routed) = http.route(url);
    let client = if public && !routed { crate::net::public_client() } else { http };
    let resp = client.get(to).header("User-Agent", APP_UA).max_bytes(max).timeout(TIMEOUT).no_redirects().send().map_err(|e| {
        if e.downcast_ref::<BodyTooLarge>().is_some() {
            ListError::TooLarge
        } else {
            ListError::Unreachable(format!("{e:#}"))
        }
    })?;
    if !resp.ok() {
        return Err(ListError::Status(resp.status));
    }
    Ok(resp.body)
}

/// Downloads and checks the list at `url`: its text and what it says.
pub fn fetch_list(http: &Http, url: &str) -> Result<(String, Index), ListError> {
    if !https_url(url) {
        return Err(ListError::NotHttps);
    }
    checked(get(http, url, MAX_INDEX_BYTES, false)?)
}

/// Reads and checks the list at `path` on this computer, as strictly as
/// one downloaded: its text and what it says.
pub fn read_list_file(path: &Path) -> Result<(String, Index), ListError> {
    let no_file = |e: std::io::Error| ListError::NoFile(format!("{}: {e}", path.display()));
    let file = std::fs::File::open(path).map_err(no_file)?;
    let mut body = Vec::new();
    file.take(MAX_INDEX_BYTES + 1).read_to_end(&mut body).map_err(no_file)?;
    if body.len() as u64 > MAX_INDEX_BYTES {
        return Err(ListError::TooLarge);
    }
    checked(body)
}

fn checked(body: Vec<u8>) -> Result<(String, Index), ListError> {
    let index = index::parse(&body).map_err(ListError::Invalid)?;
    let text = String::from_utf8(body).map_err(|_| ListError::Invalid(IndexError::NotJson("not UTF-8".into())))?;
    Ok((text, index))
}

/// An installed extension that its list offers in a newer version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Available {
    pub id: String,
    pub installed: String,
    pub offered: semver::Version,
    /// The list that offers it (the one it was installed from).
    pub list: String,
}

/// The installed extensions whose list (the one each was installed from)
/// has a higher version. A file's extension is never flagged.
pub fn updates(copies: &[ListCopy], installed: &[InstalledExtension]) -> Vec<Available> {
    installed
        .iter()
        .filter_map(|ext| {
            let list = ext.list.as_deref()?;
            let entry = copies.iter().find(|c| c.url == list)?.index.as_ref()?.get(&ext.id)?;
            let have = semver::Version::parse(&ext.version).ok()?;
            (entry.version > have && entry.supported()).then(|| Available {
                id: ext.id.clone(),
                installed: ext.version.clone(),
                offered: entry.version.clone(),
                list: list.to_owned(),
            })
        })
        .collect()
}

/// A list's entry downloaded and checked, waiting for the user to confirm
/// what [`Prepared::info`] shows. Its files are deleted when it is dropped.
#[derive(Debug)]
pub struct Prepared {
    pub info: PackageInfo,
    /// The list's address (or path, [`ListLocation::key`]).
    pub list: String,
    dir: Folder,
    /// The address (or path) of its manifest, recorded as where it came
    /// from.
    from: String,
}

/// A folder of downloaded files, deleted with it.
#[derive(Debug)]
struct Folder(PathBuf);

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn download(http: &Http, file: &IndexFile, name: &str) -> Result<Vec<u8>, ExtensionError> {
    // One byte over: a longer file is caught here rather than read whole.
    let bytes = match get(http, &file.url, file.size + 1, true) {
        Ok(b) => b,
        // Larger than the list says: not the file it names.
        Err(ListError::TooLarge) => return Err(ExtensionError::Checksum(name.to_owned())),
        Err(e) => return Err(ExtensionError::Download(format!("{name}: {e}"))),
    };
    as_listed(&bytes, file, name)?;
    Ok(bytes)
}

/// Whether `bytes` are the file the list names: its size and SHA-256.
fn as_listed(bytes: &[u8], file: &IndexFile, name: &str) -> Result<(), ExtensionError> {
    if bytes.len() as u64 != file.size || sha256(bytes) != file.sha256 {
        return Err(ExtensionError::Checksum(name.to_owned()));
    }
    Ok(())
}

/// `<the list's folder>/<id>`, where a list on this computer keeps an
/// entry's files. The id must be one plain name: the list's parser lets
/// only valid ids through, checked again here because a path is built of it.
fn entry_folder(index: &Path, id: &str) -> Result<PathBuf, ExtensionError> {
    let outside = || ExtensionError::UnsafePath(format!("the id \"{id}\" is not a folder beside the list"));
    let plain = valid_id(id) && matches!(Path::new(id).components().collect::<Vec<_>>()[..], [std::path::Component::Normal(_)]);
    if !plain {
        return Err(outside());
    }
    Ok(index.parent().ok_or_else(outside)?.join(id))
}

/// Reads `name` in `dir` for a list on this computer whose folder is
/// `root` (links resolved), and checks it as a download is checked.
/// Nothing outside `root` is read, a link that leaves it included, and
/// nothing but a plain file (a pipe would never end).
fn read_beside(root: &Path, dir: &Path, file: &IndexFile, name: &str) -> Result<Vec<u8>, ExtensionError> {
    let path = dir.join(name);
    let missing = |e: std::io::Error| {
        log::info!("{} could not be read: {e}", path.display());
        ExtensionError::MissingFile(name.to_owned())
    };
    let real = path.canonicalize().map_err(missing)?;
    if !real.starts_with(root) {
        return Err(ExtensionError::UnsafePath(format!("{} leaves the list's folder", path.display())));
    }
    if !real.metadata().map_err(missing)?.is_file() {
        return Err(ExtensionError::UnexpectedFile(path.display().to_string()));
    }
    let mut bytes = Vec::new();
    // One byte over: a longer file is caught without reading it whole.
    std::fs::File::open(&real).and_then(|f| f.take(file.size + 1).read_to_end(&mut bytes)).map_err(missing)?;
    as_listed(&bytes, file, name)?;
    Ok(bytes)
}

/// The two files of `entry` of the list at `list`, checked, and where its
/// manifest came from.
fn entry_files(service: &MetadataService, list: &str, entry: &IndexEntry) -> Result<(Vec<u8>, Vec<u8>, String), ExtensionError> {
    match ListLocation::of(list) {
        ListLocation::Address(_) => Ok((
            download(&service.http, &entry.manifest, MANIFEST)?,
            download(&service.http, &entry.script, SCRIPT)?,
            entry.manifest.url.clone(),
        )),
        ListLocation::File(index) => {
            let index = std::path::absolute(&index).unwrap_or(index);
            let dir = entry_folder(&index, &entry.id)?;
            let root = index
                .parent()
                .and_then(|p| p.canonicalize().ok())
                .ok_or_else(|| ExtensionError::MissingFile(index.display().to_string()))?;
            let manifest = read_beside(&root, &dir, &entry.manifest, MANIFEST)?;
            let script = read_beside(&root, &dir, &entry.script, SCRIPT)?;
            Ok((manifest, script, dir.join(MANIFEST).display().to_string()))
        }
    }
}

/// Whether the package read from the files is what `entry` says.
fn check_as_listed(info: &PackageInfo, entry: &IndexEntry) -> Result<(), ExtensionError> {
    let wrong = |what: String| Err(ExtensionError::NotAsListed(what));
    if info.id != entry.id {
        return wrong(format!("its id is \"{}\", the list says \"{}\"", info.id, entry.id));
    }
    if info.version != entry.version.to_string() {
        return wrong(format!("its version is {}, the list says {}", info.version, entry.version));
    }
    if info.kind != entry.kind {
        return wrong("its kind is not the one the list says".into());
    }
    if info.adult && !entry.adult {
        return wrong("it is about adult games and the list does not say so".into());
    }
    if let Some(f) = info.provides.iter().find(|f| !entry.fields.contains(f)) {
        return wrong(format!("it fills {f}, which the list does not say"));
    }
    if let Some(h) = info.hosts.iter().find(|h| !entry.hosts.contains(h)) {
        return wrong(format!("it connects to {h}, which the list does not name"));
    }
    if let Some(h) = info.image_hosts.iter().find(|h| !entry.hosts.contains(h) && !entry.images.contains(h)) {
        return wrong(format!("its pictures come from {h}, which the list does not name"));
    }
    Ok(())
}

fn stamp() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    format!("{}-{nanos}", std::process::id())
}

/// Downloads `entry` of the list at `list` (or reads it beside a list on
/// this computer), checks both files against it and reads the package as a
/// file would be ([`ExtensionsExt::inspect_extension`]). The files checked
/// are copied aside, so what is installed is what was checked.
pub fn prepare(service: &MetadataService, list: &str, entry: &IndexEntry) -> Result<Prepared, ExtensionError> {
    if !entry.supported() {
        return Err(ExtensionError::NewerApi(entry.api));
    }
    let (manifest, script, from) = entry_files(service, list, entry)?;
    let dir = Folder(service.paths.extension_lists_cache_dir().join("downloads").join(format!("{}-{}", entry.id, stamp())));
    let io = |e: std::io::Error| ExtensionError::Io(e.to_string());
    std::fs::create_dir_all(&dir.0).map_err(io)?;
    std::fs::write(dir.0.join(MANIFEST), manifest).map_err(io)?;
    std::fs::write(dir.0.join(SCRIPT), script).map_err(io)?;
    let info = service.inspect_extension(&dir.0)?;
    check_as_listed(&info, entry)?;
    Ok(Prepared { info, list: list.to_owned(), dir, from })
}

/// Installs what [`prepare`] checked; the same or an older version than
/// the installed one needs `confirmed`, as for a file.
pub fn install_prepared(service: &MetadataService, prepared: &Prepared, confirmed: bool) -> Result<InstallOutcome, ExtensionError> {
    service::install(service, &prepared.dir.0, confirmed, &prepared.from, Some(&prepared.list), None)
}

/// Installs what [`prepare`] checked and the user approved: the same files
/// (by the SHA-256 shown), keeping or forgetting the saved settings as
/// chosen.
pub fn install_approved(service: &MetadataService, prepared: &Prepared, approval: &Approval) -> Result<InstallOutcome, ExtensionError> {
    service::install(service, &prepared.dir.0, true, &prepared.from, Some(&prepared.list), Some(approval))
}
