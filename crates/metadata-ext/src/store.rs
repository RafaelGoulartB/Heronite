//! The installed extensions: `<data>/extensions/metadata/<id>/` holds the
//! package's files as they were and `install.json`, what the host records
//! (when, from where, the package's SHA-256, whether it was stopped after
//! failing). A folder is replaced whole: the new one is written beside it
//! and swapped in by renames, so a crash leaves the old or the new one.

use std::path::{Path, PathBuf};

use heronite_data::paths::DataPaths;
use heronite_data::temps::write_atomic;
// Windows refuses a rename for a moment while an antivirus or the indexer
// holds a file of the folder: tried again there.
use heronite_platform::fs::rename_replacing;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::manifest::valid_id;
use crate::package::{Package, RECORD};
use crate::ExtensionError;

/// `install.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InstallRecord {
    /// ISO 8601, UTC.
    pub installed_at: String,
    /// The file or folder it was installed from, as given.
    pub from: String,
    /// [`Package::sha256`].
    pub sha256: String,
    pub version: String,
    /// The last error, while it is stopped after failing too often.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failing: Option<String>,
    /// The address of the list it was installed from (`index.rs`); none
    /// for a file or a folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Who made an extension removed with its settings kept, and where it
/// connected: `<root>/.kept/<id>.json`. A later install of that id from
/// someone else, or to other hosts, is shown as such before those settings
/// reach it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Kept {
    pub author: String,
    /// `[network] hosts` and `images`, as the manifest wrote them.
    pub hosts: Vec<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

pub fn root(paths: &DataPaths) -> PathBuf {
    paths.metadata_extensions_dir()
}

fn kept_file(paths: &DataPaths, id: &str) -> Option<PathBuf> {
    valid_id(id).then(|| root(paths).join(".kept").join(format!("{id}.json")))
}

pub fn read_kept(paths: &DataPaths, id: &str) -> Option<Kept> {
    serde_json::from_str(&std::fs::read_to_string(kept_file(paths, id)?).ok()?).ok()
}

pub fn write_kept(paths: &DataPaths, id: &str, kept: &Kept) -> Result<(), ExtensionError> {
    let Some(file) = kept_file(paths, id) else { return Ok(()) };
    std::fs::create_dir_all(file.parent().unwrap_or(&root(paths))).map_err(io)?;
    let text = serde_json::to_string_pretty(kept).map_err(|e| ExtensionError::Io(e.to_string()))?;
    write_atomic(&file, text.as_bytes(), None).map_err(io)
}

pub fn delete_kept(paths: &DataPaths, id: &str) {
    if let Some(file) = kept_file(paths, id) {
        let _ = std::fs::remove_file(file);
    }
}

fn io(e: std::io::Error) -> ExtensionError {
    ExtensionError::Io(e.to_string())
}

/// Folders of installed extensions (named by a valid id), by name. Only
/// reads: what an interrupted install left is put back by [`recover`].
pub fn folders(paths: &DataPaths) -> Vec<(String, PathBuf)> {
    let root = root(paths);
    let Ok(entries) = std::fs::read_dir(&root) else { return Vec::new() };
    let mut out: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            valid_id(&name).then(|| (name, e.path()))
        })
        .collect();
    out.sort();
    out
}

pub fn read_record(dir: &Path) -> Option<InstallRecord> {
    let text = std::fs::read_to_string(dir.join(RECORD)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write_record(dir: &Path, record: &InstallRecord) -> Result<(), ExtensionError> {
    let text = serde_json::to_string_pretty(record).map_err(|e| ExtensionError::Io(e.to_string()))?;
    write_atomic(&dir.join(RECORD), text.as_bytes(), None).map_err(io)
}

/// Sets or clears the failure an installed extension was stopped with,
/// when the package installed there is still `sha256`: a lookup of a
/// version removed or replaced meanwhile leaves the new one's record alone.
pub fn set_failing(dir: &Path, sha256: &str, failing: Option<&str>) {
    let Some(mut record) = read_record(dir) else { return };
    if record.sha256 != sha256 || record.failing.as_deref() == failing {
        return;
    }
    record.failing = failing.map(str::to_owned);
    if let Err(e) = write_record(dir, &record) {
        log::warn!("Could not record the state of the extension in {}: {e}", dir.display());
    }
}

fn stamp() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{}-{nanos}", std::process::id())
}

/// Held while folders are written, swapped, deleted or put back, so
/// [`recover`] never takes an install of this process in progress for one
/// a crash interrupted.
static CHANGING: Mutex<()> = Mutex::new(());

/// Puts back what an interrupted install left: a replaced folder whose new
/// one never arrived, and half-written folders. Called when the installed
/// extensions are loaded at startup, not on every read.
pub fn recover(paths: &DataPaths) {
    let _changing = CHANGING.lock();
    let root = root(paths);
    let Ok(entries) = std::fs::read_dir(&root) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".tmp-") {
            let _ = std::fs::remove_dir_all(entry.path());
        } else if let Some(rest) = name.strip_prefix(".old-") {
            let id = rest.rsplitn(3, '-').nth(2).unwrap_or("");
            let target = root.join(id);
            if valid_id(id) && !target.exists() {
                let _ = rename_replacing(&entry.path(), &target);
            } else {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
}

/// Writes `package` as the installed extension of its id, replacing the
/// one installed. `list`: the list it comes from. Returns its folder.
pub fn write(paths: &DataPaths, package: &Package, from: &str, list: Option<&str>) -> Result<PathBuf, ExtensionError> {
    let _changing = CHANGING.lock();
    let id = &package.manifest.id;
    let root = root(paths);
    std::fs::create_dir_all(&root).map_err(io)?;
    let stamp = stamp();
    let tmp = root.join(format!(".tmp-{id}-{stamp}"));
    std::fs::create_dir(&tmp).map_err(io)?;
    let written = (|| {
        for (name, bytes) in package.files() {
            std::fs::write(tmp.join(&name), bytes).map_err(io)?;
        }
        let record = InstallRecord {
            installed_at: heronite_metadata::util::now_iso(),
            from: from.to_owned(),
            sha256: package.sha256.clone(),
            version: package.manifest.version.to_string(),
            failing: None,
            list: list.map(str::to_owned),
            extra: Default::default(),
        };
        write_record(&tmp, &record)
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    let target = root.join(id);
    if target.exists() {
        let old = root.join(format!(".old-{id}-{stamp}"));
        rename_replacing(&target, &old).map_err(io)?;
        if let Err(e) = rename_replacing(&tmp, &target) {
            let _ = rename_replacing(&old, &target);
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(io(e));
        }
        let _ = std::fs::remove_dir_all(&old);
    } else {
        rename_replacing(&tmp, &target).map_err(io)?;
    }
    Ok(target)
}

/// Deletes the installed extension `id`; `false` when there was none.
pub fn delete(paths: &DataPaths, id: &str) -> Result<bool, ExtensionError> {
    if !valid_id(id) {
        return Ok(false);
    }
    let _changing = CHANGING.lock();
    let dir = root(paths).join(id);
    if !dir.is_dir() {
        return Ok(false);
    }
    std::fs::remove_dir_all(&dir).map_err(io)?;
    Ok(true)
}
