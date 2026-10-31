//! Downloads legendary into `<data>/tools/bin/legendary` (`legendary.exe` on
//! Windows) when no legendary is available, so a new setup can log in to Epic
//! without installing anything (Heroic ships the same binary inside its
//! package; see [`crate::store_tools`]).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use heronite_data::DataPaths;

use crate::net::{Cancel, Net, CANCELLED};
use crate::probe::Probe;
use crate::store_tools::LEGENDARY;

/// The release Heroic pins.
pub const VERSION: &str = LEGENDARY.version();

/// Release asset for this system and CPU, `None` where legendary has no build.
pub fn asset_name() -> Option<&'static str> {
    LEGENDARY.asset_name()
}

pub fn download_url(asset: &str) -> String {
    LEGENDARY.download_url(asset)
}

/// The legendary this app runs: an existing one (settings, data folder,
/// `PATH`), else a fresh download into the data folder. Blocking.
pub fn ensure(paths: &DataPaths, probe: &dyn Probe, net: &dyn Net, cancel: Cancel) -> Result<PathBuf, String> {
    LEGENDARY.ensure(paths, probe, net, cancel)
}

/// Where a download waits until it proved it runs: `<name>.part` next to
/// `target` (`<name>.part.exe` on Windows, which only starts `.exe` files
/// as programs).
pub(crate) fn part_file(target: &Path) -> PathBuf {
    target.with_extension(format!("part{}", std::env::consts::EXE_SUFFIX))
}

/// Downloads `name` next to `target`, checks that it runs (`--version`), then
/// renames it into place, so an interrupted or broken download never becomes
/// the binary.
pub(crate) fn install_binary(name: &str, url: &str, target: &Path, net: &dyn Net, cancel: Cancel) -> Result<(), String> {
    let dir = target.parent().ok_or_else(|| format!("{name} has no folder"))?;
    fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let part = part_file(target);
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
    heronite_platform::fs::rename_replacing(&part, target).map_err(|e| fail(format!("installing {name}: {e}")))
}

/// Whether `bin --version` exits well. It runs as a helper: no console
/// window on Windows, and a file just written that is still "busy"
/// (`ETXTBSY`) is tried again (`heronite_data::proc::spawn`).
fn runs_version(bin: &Path) -> bool {
    let mut cmd = Command::new(bin);
    cmd.arg("--version");
    // The first run of a Windows build unpacks itself: give it time.
    let run = heronite_data::proc::Run::new().timeout(Duration::from_secs(120)).grace(Duration::from_secs(2));
    run.run(&mut cmd).is_ok_and(|out| out.success())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::FakeNet;
    use crate::probe::FakeProbe;

    use crate::store_tools::tests::fake_bin;

    #[test]
    fn downloads_into_the_data_folder_when_none_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heronite"));
        let url = download_url(asset_name().unwrap());
        let net = FakeNet::online().with_file(&url, fake_bin(true));
        let got = ensure(&paths, &FakeProbe::default(), &net, &|| false).unwrap();
        assert_eq!(got, paths.legendary_bin());
        assert!(paths.legendary_bin().exists());
        assert!(!part_file(&paths.legendary_bin()).exists());

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
        assert!(!part_file(&paths.legendary_bin()).exists());
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
