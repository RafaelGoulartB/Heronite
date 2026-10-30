//! Downloads legendary into `<data>/tools/bin/legendary` when no legendary is
//! available, so a new setup can log in to Epic without installing anything
//! (Heroic ships the same binary inside its package; see `RELEASE_TAGS` in
//! its `meta/downloadHelperBinaries.ts`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

use heronite_data::DataPaths;

use crate::legendary::find_legendary;
use crate::net::{Cancel, Net, CANCELLED};
use crate::plan::PlanContext;
use crate::probe::Probe;

/// The release Heroic pins.
pub const VERSION: &str = "0.21.1";
const REPO: &str = "legendary-gl/legendary";

/// Release asset for this CPU, `None` where legendary has no Linux build.
pub fn asset_name() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("legendary_linux_x64"),
        "aarch64" => Some("legendary_linux_arm64"),
        _ => None,
    }
}

pub fn download_url(asset: &str) -> String {
    format!("https://github.com/{REPO}/releases/download/{VERSION}/{asset}")
}

/// One download at a time: the startup check and a login can both ask.
static BUSY: Mutex<()> = Mutex::new(());

/// The legendary this app runs: an existing one (settings, data folder,
/// `$PATH`), else a fresh download into the data folder. Blocking.
pub fn ensure(paths: &DataPaths, probe: &dyn Probe, net: &dyn Net, cancel: Cancel) -> Result<PathBuf, String> {
    let _guard = BUSY.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(found) = find_legendary(&PlanContext::load(paths), probe) {
        return Ok(PathBuf::from(found));
    }
    let asset = asset_name().ok_or_else(|| format!("legendary has no build for {}", std::env::consts::ARCH))?;
    if !net.is_online() {
        return Err("legendary is not installed and there is no internet connection".into());
    }
    let target = paths.legendary_bin();
    install_binary("legendary", &download_url(asset), &target, net, cancel)?;
    log::info!("legendary {VERSION} downloaded to {}", target.display());
    Ok(target)
}

/// Downloads `name` next to `target`, checks that it runs (`--version`), then
/// renames it into place, so an interrupted or broken download never becomes
/// the binary.
pub(crate) fn install_binary(name: &str, url: &str, target: &Path, net: &dyn Net, cancel: Cancel) -> Result<(), String> {
    let dir = target.parent().ok_or_else(|| format!("{name} has no folder"))?;
    fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let part = target.with_extension("part");
    let _ = fs::remove_file(&part);
    let fail = |msg: String| {
        let _ = fs::remove_file(&part);
        msg
    };
    net.download(url, &part, cancel).map_err(|e| fail(if e == CANCELLED { e } else { format!("downloading {name} failed: {e}") }))?;
    heronite_platform::fs::make_executable(&part).map_err(|e| fail(format!("making {name} executable: {e}")))?;
    if !runs_version(&part) {
        return Err(fail(format!("the downloaded {name} does not run")));
    }
    fs::rename(&part, target).map_err(|e| fail(format!("installing {name}: {e}")))
}

/// Whether `bin --version` exits well. A file just written can be "busy"
/// (`ETXTBSY`) for a moment when another thread forks while it was open for
/// writing (the child holds the descriptor until it execs): try again then.
fn runs_version(bin: &Path) -> bool {
    const ETXTBSY: i32 = 26;
    for _ in 0..20 {
        match Command::new(bin).arg("--version").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status() {
            Ok(s) => return s.success(),
            Err(e) if e.raw_os_error() == Some(ETXTBSY) => std::thread::sleep(std::time::Duration::from_millis(25)),
            Err(_) => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::FakeNet;
    use crate::probe::FakeProbe;

    fn fake_bin(ok: bool) -> Vec<u8> {
        format!("#!/bin/sh\nexit {}\n", if ok { 0 } else { 1 }).into_bytes()
    }

    #[test]
    fn downloads_into_the_data_folder_when_none_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heronite"));
        let url = download_url(asset_name().unwrap());
        let net = FakeNet::online().with_file(&url, fake_bin(true));
        let got = ensure(&paths, &FakeProbe::default(), &net, &|| false).unwrap();
        assert_eq!(got, paths.legendary_bin());
        assert!(paths.legendary_bin().exists());
        assert!(!paths.legendary_bin().with_extension("part").exists());

        // Found now: no second download.
        let probe = FakeProbe::default().with_path(paths.legendary_bin().to_str().unwrap());
        ensure(&paths, &probe, &net, &|| false).unwrap();
        assert_eq!(net.requests().len(), 1);
    }

    #[test]
    fn an_existing_legendary_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heronite"));
        let probe = FakeProbe::default().with_bin("legendary", "/usr/bin/legendary");
        let net = FakeNet::online();
        assert_eq!(ensure(&paths, &probe, &net, &|| false).unwrap(), PathBuf::from("/usr/bin/legendary"));
        assert!(net.requests().is_empty());
    }

    #[test]
    fn a_binary_that_does_not_run_is_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heronite"));
        let url = download_url(asset_name().unwrap());
        let net = FakeNet::online().with_file(&url, fake_bin(false));
        let err = ensure(&paths, &FakeProbe::default(), &net, &|| false).unwrap_err();
        assert!(err.contains("does not run"), "{err}");
        assert!(!paths.legendary_bin().exists());
        assert!(!paths.legendary_bin().with_extension("part").exists());
    }

    #[test]
    fn offline_is_an_error_without_a_download() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heronite"));
        let net = FakeNet::offline();
        assert!(ensure(&paths, &FakeProbe::default(), &net, &|| false).unwrap_err().contains("no internet"));
        assert!(net.requests().is_empty());
    }
}
