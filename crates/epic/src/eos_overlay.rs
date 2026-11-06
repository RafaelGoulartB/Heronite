//! The EOS Overlay (port of `legendary/eos_overlay/eos_overlay.ts`),
//! natively ([`heronite_epic_api::eos`]). The overlay is installed into
//! `<data>/tools/eos_overlay`; on Linux it is enabled per Wine prefix, on
//! Windows for the user. Its record is legendary's `overlay_install.json`,
//! its latest version `overlay_version.json`, in legendary's config folder.

use std::path::{Path, PathBuf};

use heronite_data::game_config;
use heronite_data::DataPaths;
use heronite_depot as depot;
use heronite_epic_api::eos;
use heronite_i18n::tr;
use serde_json::Value;

use crate::client::Epic;
use crate::cmd::Cancel;
use crate::ops::{home, DownloadSettings, OpResult};

/// `eosOverlayAppName`: the id Heroic gives the overlay in its status maps.
pub const APP_NAME: &str = eos::APP_NAME;

/// Where the overlay is installed by default (`toolsPath/eos_overlay`).
pub fn default_install_path(data_paths: &DataPaths) -> PathBuf {
    data_paths.tools_dir().join("eos_overlay")
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub installed: bool,
    pub version: Option<String>,
    pub install_path: Option<String>,
}

/// `getStatus`.
pub fn status(data_paths: &DataPaths) -> Status {
    let Some(ig) = eos::installed(&data_paths.legendary_config_dir()) else { return Status::default() };
    if Path::new(&ig.install_path) != default_install_path(data_paths) {
        log::warn!("EOS Overlay is not installed in default location, permission issues might arise");
    }
    let some = |s: String| Some(s).filter(|s| !s.is_empty());
    Status { installed: true, version: some(ig.version), install_path: some(ig.install_path) }
}

/// `updateInfo`: refreshes the latest version (`overlay_version.json`) when
/// the overlay is installed. Blocking, network.
pub fn update_info(data_paths: &DataPaths, epic: &Epic) {
    if !status(data_paths).installed {
        return;
    }
    if let Err(err) = eos::latest_version(&epic.client, &epic.config_dir) {
        log::warn!("Updating EOS Overlay information: {err:#}");
    }
}

/// `getLatestVersion`: the latest build version (asked of Epic at most once
/// a day). Empty when unknown or when the overlay is not installed.
pub fn latest_version(data_paths: &DataPaths, epic: &Epic) -> String {
    if !status(data_paths).installed {
        return String::new();
    }
    eos::latest_version(&epic.client, &epic.config_dir).unwrap_or_else(|err| {
        log::error!("EOS Overlay information: {err:#}");
        String::new()
    })
}

/// `install`: installs (or updates) the overlay, reporting the percent.
/// Blocking, network.
pub fn install(data_paths: &DataPaths, epic: &Epic, cancel: &std::sync::Arc<Cancel>, on_percent: &dyn Fn(f32)) -> OpResult {
    if cancel.is_requested() {
        return OpResult::cancelled();
    }
    let path = default_install_path(data_paths);
    let prepared = match eos::prepare_install(&epic.client, &epic.config_dir, &path) {
        Ok(p) => p,
        Err(err) => return OpResult::err(tr!("Could not install the EOS Overlay: {error}", error = format!("{err:#}"))),
    };
    let settings = DownloadSettings::load(data_paths);
    let outcome = crate::download::install_overlay(epic, &prepared, &settings, cancel, &mut |p| on_percent(p.percent() as f32));
    match outcome {
        Err(_) | Ok(depot::Outcome::Cancelled) if cancel.is_requested() => OpResult::cancelled(),
        Ok(depot::Outcome::Cancelled) => OpResult::cancelled(),
        Err(err) => OpResult::err(tr!("Could not install the EOS Overlay: {error}", error = format!("{err:#}"))),
        Ok(depot::Outcome::Done) => match eos::finish_install(&epic.config_dir, &prepared.record) {
            Ok(()) => OpResult::ok(tr!("EOS Overlay installed")),
            Err(err) => OpResult::err(format!("{err:#}")),
        },
    }
}

/// `remove` (the caller asks the user first).
pub fn remove(epic: &Epic) -> OpResult {
    match eos::remove(&epic.config_dir) {
        Ok(()) => OpResult::ok(tr!("EOS Overlay uninstalled")),
        Err(err) => OpResult::err(format!("{err:#}")),
    }
}

/// `enable` for a valid prefix folder (see [`prefix_folder`]); `None`: for
/// the user on Windows.
pub fn enable(epic: &Epic, prefix: Option<&str>) -> OpResult {
    let Some(ig) = eos::installed(&epic.config_dir) else { return OpResult::err(tr!("The EOS Overlay is not installed")) };
    let overlay = PathBuf::from(&ig.install_path);
    let done = match prefix {
        Some(p) => eos::enable_in_prefix(Path::new(p), &overlay),
        None => windows_registry::set(&eos::windows_values(&overlay)),
    };
    match done {
        Ok(()) => OpResult::ok(tr!("EOS Overlay enabled")),
        Err(err) => OpResult::err(format!("{err:#}")),
    }
}

/// `disable` for a valid prefix folder; `None`: for the user on Windows.
pub fn disable(epic: &Epic, prefix: Option<&str>) -> OpResult {
    let done = match prefix {
        Some(p) => eos::disable_in_prefix(Path::new(p)),
        None => {
            let overlay = eos::installed(&epic.config_dir).map(|g| PathBuf::from(g.install_path)).unwrap_or_default();
            windows_registry::delete(&eos::windows_values(&overlay))
        }
    };
    match done {
        Ok(()) => OpResult::ok(tr!("EOS Overlay disabled")),
        Err(err) => OpResult::err(format!("{err:#}")),
    }
}

/// `isEnabled`: the prefix's (or, `None`, the user's) registry points at
/// an overlay install.
pub fn is_enabled(_epic: &Epic, prefix: Option<&str>) -> bool {
    match prefix {
        Some(p) => eos::enabled_in_prefix(Path::new(p)),
        None => windows_registry::overlay_path().is_some_and(|p| eos::is_overlay_install(&p)),
    }
}

/// The overlay's values in the Windows registry of the user, through
/// `reg.exe` (as legendary writes them). Elsewhere there is no such
/// registry: setting fails, reading finds nothing.
mod windows_registry {
    use std::path::PathBuf;
    use std::process::Command;

    use anyhow::{bail, Result};
    use heronite_epic_api::eos::{RegValue, WINDOWS_KEY};

    fn reg(args: &[&str]) -> Result<std::process::Output> {
        if !cfg!(windows) {
            bail!("the Windows registry is only on Windows");
        }
        let mut cmd = Command::new("reg");
        cmd.args(args);
        heronite_platform::process::new_helper_group(&mut cmd);
        Ok(cmd.output()?)
    }

    pub fn set(values: &[RegValue]) -> Result<()> {
        for v in values {
            let out = match v {
                RegValue::String { key, name, value } => {
                    reg(&["add", &format!(r"HKCU\{key}"), "/v", name, "/t", "REG_SZ", "/d", value, "/f"])?
                }
                RegValue::Dword { key, name, value } => {
                    reg(&["add", &format!(r"HKCU\{key}"), "/v", name, "/t", "REG_DWORD", "/d", &value.to_string(), "/f"])?
                }
            };
            if !out.status.success() {
                bail!("reg add: {}", String::from_utf8_lossy(&out.stderr).trim());
            }
        }
        Ok(())
    }

    pub fn delete(values: &[RegValue]) -> Result<()> {
        for v in values {
            let (RegValue::String { key, name, .. } | RegValue::Dword { key, name, .. }) = v;
            // A value that is not there is fine.
            reg(&["delete", &format!(r"HKCU\{key}"), "/v", name, "/f"])?;
        }
        Ok(())
    }

    /// `OverlayPath` of the user, when set.
    pub fn overlay_path() -> Option<PathBuf> {
        let out = reg(&["query", &format!(r"HKCU\{WINDOWS_KEY}"), "/v", "OverlayPath"]).ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text.lines().find(|l| l.trim_start().starts_with("OverlayPath"))?;
        line.split_once("REG_SZ").map(|(_, v)| PathBuf::from(v.trim())).filter(|p| !p.as_os_str().is_empty())
    }
}

/// What the Wine prefix of a game gives the overlay commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrefixFolder {
    /// Not Linux (the overlay is global there).
    NotApplicable,
    /// A prefix is configured but it is not a Wine prefix (no `user.reg`).
    Invalid,
    Valid(String),
}

/// `getWinePrefixFolder`: `pfx` inside a Proton prefix, checked for `user.reg`.
pub fn prefix_folder(data_paths: &DataPaths, app_name: &str) -> PrefixFolder {
    if !cfg!(target_os = "linux") || app_name.is_empty() {
        return PrefixFolder::NotApplicable;
    }
    let settings = game_config::load_game(data_paths, app_name, &home());
    let prefix = settings.get("winePrefix").and_then(Value::as_str).unwrap_or_default();
    let proton = settings.get("wineVersion").and_then(|w| w.get("type")).and_then(Value::as_str) == Some("proton");
    let folder = if proton { Path::new(prefix).join("pfx") } else { PathBuf::from(prefix) };
    if !prefix.is_empty() && folder.join("user.reg").exists() {
        PrefixFolder::Valid(folder.to_string_lossy().into_owned())
    } else {
        PrefixFolder::Invalid
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::testing::{serve_overlay, Server};

    fn fixture() -> (tempfile::TempDir, DataPaths) {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("h"));
        std::fs::create_dir_all(data_paths.legendary_config_dir()).unwrap();
        (dir, data_paths)
    }

    #[test]
    fn status_reads_the_install_file() {
        let (_d, data_paths) = fixture();
        assert_eq!(status(&data_paths), Status::default());
        std::fs::write(
            data_paths.legendary_config_dir().join("overlay_install.json"),
            r#"{"version": "1.1.9", "install_path": "/x/eos_overlay", "managed": true}"#,
        )
        .unwrap();
        let s = status(&data_paths);
        assert!(s.installed);
        assert_eq!(s.version.as_deref(), Some("1.1.9"));
        assert_eq!(s.install_path.as_deref(), Some("/x/eos_overlay"));
    }

    #[test]
    fn the_overlay_installs_switches_in_a_prefix_and_goes() {
        let (d, data_paths) = fixture();
        let server = Server::start();
        serve_overlay(&server, "1.2.0");
        let epic = server.epic(&data_paths.legendary_config_dir());
        // Not installed: no version to ask for.
        assert_eq!(latest_version(&data_paths, &epic), "");
        let seen = std::sync::Mutex::new(Vec::new());
        let r = install(&data_paths, &epic, &Cancel::new(), &|p| seen.lock().unwrap().push(p));
        assert!(r.ok, "{}", r.message);
        assert_eq!(seen.lock().unwrap().last(), Some(&100.0f32));
        let s = status(&data_paths);
        assert_eq!(s.version.as_deref(), Some("1.2.0"));
        assert!(eos::is_overlay_install(&default_install_path(&data_paths)));
        assert_eq!(latest_version(&data_paths, &epic), "1.2.0");

        let prefix = d.path().join("pfx");
        std::fs::create_dir_all(&prefix).unwrap();
        std::fs::write(prefix.join("user.reg"), "WINE REGISTRY Version 2\n;; All keys relative to \\User\\S-1-5-21-0-0-0-1000\n\n")
            .unwrap();
        let p = prefix.to_str();
        assert!(!is_enabled(&epic, p));
        assert!(enable(&epic, p).ok);
        assert!(is_enabled(&epic, p));
        assert!(disable(&epic, p).ok);
        assert!(!is_enabled(&epic, p));

        assert!(remove(&epic).ok);
        assert!(!status(&data_paths).installed && !default_install_path(&data_paths).exists());
    }

    #[test]
    fn a_cancelled_install_says_so() {
        let (_d, data_paths) = fixture();
        let server = Server::start();
        let cancel: std::sync::Arc<Cancel> = Cancel::new();
        cancel.request();
        assert!(install(&data_paths, &server.epic(&data_paths.legendary_config_dir()), &cancel, &|_| {}).aborted);
        // Epic does not answer: an error to read.
        let r = install(&data_paths, &server.epic(&data_paths.legendary_config_dir()), &Cancel::new(), &|_| {});
        assert!(!r.ok && !r.aborted && r.message.contains("EOS Overlay"), "{}", r.message);
    }

    #[test]
    fn the_overlay_has_no_prefix_outside_linux() {
        let (_d, data_paths) = fixture();
        if !cfg!(target_os = "linux") {
            assert_eq!(prefix_folder(&data_paths, "g"), PrefixFolder::NotApplicable);
        }
    }

    #[test]
    fn prefix_folder_follows_proton_and_needs_a_real_prefix() {
        if !cfg!(target_os = "linux") {
            return;
        }
        let (d, data_paths) = fixture();
        let write = |settings: serde_json::Value| {
            std::fs::create_dir_all(data_paths.games_config_dir()).unwrap();
            std::fs::write(data_paths.game_config("g"), serde_json::json!({ "g": settings }).to_string()).unwrap();
        };
        let prefix = d.path().join("pfx");
        write(serde_json::json!({"winePrefix": prefix.to_str().unwrap(), "wineVersion": {"type": "wine"}}));
        assert_eq!(prefix_folder(&data_paths, "g"), PrefixFolder::Invalid);
        std::fs::create_dir_all(&prefix).unwrap();
        std::fs::write(prefix.join("user.reg"), "").unwrap();
        assert_eq!(prefix_folder(&data_paths, "g"), PrefixFolder::Valid(prefix.to_string_lossy().into_owned()));
        write(serde_json::json!({"winePrefix": prefix.to_str().unwrap(), "wineVersion": {"type": "proton"}}));
        assert_eq!(prefix_folder(&data_paths, "g"), PrefixFolder::Invalid);
        std::fs::create_dir_all(prefix.join("pfx")).unwrap();
        std::fs::write(prefix.join("pfx/user.reg"), "").unwrap();
        assert_eq!(prefix_folder(&data_paths, "g"), PrefixFolder::Valid(prefix.join("pfx").to_string_lossy().into_owned()));
        assert_eq!(prefix_folder(&data_paths, ""), PrefixFolder::NotApplicable);
    }
}
