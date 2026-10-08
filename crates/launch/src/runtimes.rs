//! Runtimes Heroic downloads from Lutris into `<heroic>/tools/runtimes`: umu,
//! the EAC and the BattlEye runtimes (port of `backend/wine/runtimes`).
//! Each lives in `runtimes/<name>/` with a `current_version` file holding the
//! `created_at` of the release that was installed.

use std::fs;
use std::path::PathBuf;

use heronite_data::HeroicPaths;
use serde_json::Value;

use crate::net::{extract_tar, Cancel, Net, CANCELLED};
use crate::tools::write_atomic;

pub const RUNTIMES_URL: &str = "https://lutris.net/api/runtimes";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runtime {
    Umu,
    Eac,
    BattlEye,
}

impl Runtime {
    /// Folder name, also the name in Lutris' list.
    pub fn name(self) -> &'static str {
        match self {
            Runtime::Umu => "umu",
            Runtime::Eac => "eac_runtime",
            Runtime::BattlEye => "battleye_runtime",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Runtime::Umu => "umu",
            Runtime::Eac => "Easy Anti-Cheat runtime",
            Runtime::BattlEye => "BattlEye runtime",
        }
    }
}

pub fn runtimes_dir(paths: &HeroicPaths) -> PathBuf {
    paths.tools_dir().join("runtimes")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    url: String,
    created_at: String,
}

fn find_entry(list: &Value, runtime: Runtime) -> Option<Entry> {
    list.as_array()?.iter().find_map(|e| {
        (e.get("name")?.as_str()? == runtime.name()).then(|| Entry {
            url: e.get("url").and_then(Value::as_str).unwrap_or_default().to_string(),
            created_at: e.get("created_at").and_then(Value::as_str).unwrap_or_default().to_string(),
        })
    })
}

/// `isInstalled`: the folder exists and `current_version` matches Lutris'
/// newest release. When the list cannot be fetched (offline) an existing
/// folder counts as installed, so a missing connection never forces a
/// download loop.
pub fn is_installed(paths: &HeroicPaths, net: &dyn Net, runtime: Runtime) -> bool {
    let dir = runtimes_dir(paths).join(runtime.name());
    if !dir.is_dir() {
        return false;
    }
    let Ok(list) = net.get_json(RUNTIMES_URL) else {
        return true;
    };
    let Some(entry) = find_entry(&list, runtime) else {
        return true;
    };
    fs::read_to_string(dir.join("current_version")).map(|v| v == entry.created_at).unwrap_or(false)
}

/// Whether the runtime has to be downloaded now.
pub fn needs_download(paths: &HeroicPaths, net: &dyn Net, runtime: Runtime) -> bool {
    net.is_online() && !is_installed(paths, net, runtime)
}

/// `download`: fetches Lutris' archive and replaces `runtimes/<name>`. The old
/// folder stays until the new one is fully extracted.
pub fn download(paths: &HeroicPaths, net: &dyn Net, runtime: Runtime, cancel: Cancel) -> Result<(), String> {
    let list = net.get_json(RUNTIMES_URL).map_err(|e| format!("could not fetch the runtime list: {e}"))?;
    let entry = find_entry(&list, runtime)
        .filter(|e| !e.url.is_empty())
        .ok_or_else(|| format!("{} was not found in the runtime list", runtime.label()))?;
    let base = runtimes_dir(paths);
    fs::create_dir_all(&base).map_err(|e| format!("creating {}: {e}", base.display()))?;
    let file = entry.url.rsplit('/').next().unwrap_or("runtime.tar.xz");
    if file.is_empty() || file.contains("..") {
        return Err(format!("unusable runtime url {}", entry.url));
    }
    let archive = base.join(format!("{file}.part"));
    let _ = fs::remove_file(&archive);
    if let Err(e) = net.download(&entry.url, &archive, cancel) {
        let _ = fs::remove_file(&archive);
        return Err(if e == CANCELLED { e } else { format!("downloading {} failed: {e}", runtime.label()) });
    }
    let tmp = base.join(format!(".{}.extracting", runtime.name()));
    let _ = fs::remove_dir_all(&tmp);
    let extracted = extract_tar(&archive, &tmp, 1, cancel);
    let _ = fs::remove_file(&archive);
    if let Err(e) = extracted {
        let _ = fs::remove_dir_all(&tmp);
        return Err(if e == CANCELLED { e } else { format!("unpacking {} failed: {e}", runtime.label()) });
    }
    if let Err(e) = write_atomic(&tmp.join("current_version"), &entry.created_at) {
        let _ = fs::remove_dir_all(&tmp);
        return Err(e);
    }
    let dest = base.join(runtime.name());
    let _ = fs::remove_dir_all(&dest);
    fs::rename(&tmp, &dest).map_err(|e| {
        let _ = fs::remove_dir_all(&tmp);
        format!("could not move {} into place: {e}", runtime.label())
    })
}

/// Where umu's script lives once downloaded.
pub fn umu_script(paths: &HeroicPaths) -> PathBuf {
    runtimes_dir(paths).join("umu/umu_run.py")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::FakeNet;
    use crate::tools::testing::tarball;
    use serde_json::json;

    fn list(created: &str) -> Value {
        json!([
            {"name": "other", "url": "http://x/o.tar.xz", "created_at": "x"},
            {"name": "eac_runtime", "url": "http://x/eac.tar.xz", "created_at": created},
            {"name": "umu", "url": "http://x/umu.tar.xz", "created_at": created}
        ])
    }

    #[test]
    fn downloads_and_marks_the_version() {
        let dir = tempfile::tempdir().unwrap();
        let paths = HeroicPaths::at(dir.path().join("h"));
        let tar = tarball(dir.path(), Some("eac"), &[("v2/lib64/x.so", "so")]);
        let net = FakeNet::online().with_json(RUNTIMES_URL, list("2024-01-01T00:00:00Z")).with_file("http://x/eac.tar.xz", tar);
        assert!(!is_installed(&paths, &net, Runtime::Eac));
        assert!(needs_download(&paths, &net, Runtime::Eac));
        download(&paths, &net, Runtime::Eac, &|| false).unwrap();
        let rt = runtimes_dir(&paths).join("eac_runtime");
        assert!(rt.join("v2/lib64/x.so").exists());
        assert_eq!(fs::read_to_string(rt.join("current_version")).unwrap(), "2024-01-01T00:00:00Z");
        assert!(is_installed(&paths, &net, Runtime::Eac));
        assert!(!needs_download(&paths, &net, Runtime::Eac));
        let left: Vec<_> = fs::read_dir(runtimes_dir(&paths)).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left.len(), 1, "{left:?}");
    }

    #[test]
    fn a_newer_release_is_detected_but_offline_never_forces_a_download() {
        let dir = tempfile::tempdir().unwrap();
        let paths = HeroicPaths::at(dir.path().join("h"));
        let rt = runtimes_dir(&paths).join("umu");
        fs::create_dir_all(&rt).unwrap();
        fs::write(rt.join("current_version"), "old").unwrap();
        let newer = FakeNet::online().with_json(RUNTIMES_URL, list("new"));
        assert!(!is_installed(&paths, &newer, Runtime::Umu));
        // The list cannot be fetched: an existing folder is good enough.
        assert!(is_installed(&paths, &FakeNet::offline(), Runtime::Umu));
        assert!(!needs_download(&paths, &FakeNet::offline(), Runtime::Umu));
    }

    #[test]
    fn unknown_runtimes_and_failures_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let paths = HeroicPaths::at(dir.path().join("h"));
        let net = FakeNet::online().with_json(RUNTIMES_URL, list("t"));
        assert!(download(&paths, &net, Runtime::BattlEye, &|| false).unwrap_err().contains("BattlEye"));
        // Listed, but the archive is not served.
        assert!(download(&paths, &net, Runtime::Umu, &|| false).unwrap_err().contains("failed"));
        assert!(!runtimes_dir(&paths).join("umu").exists());
    }
}
