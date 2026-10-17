//! The EOS Overlay through `legendary eos-overlay ...` (port of
//! `legendary/eos_overlay/eos_overlay.ts`). The overlay is installed into
//! `<data>/tools/eos_overlay`; on Linux it is enabled per Wine prefix.
//! legendary keeps its state in `overlay_install.json` and
//! `overlay_version.json` of its config folder; they are only read here.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use heronite_data::game_config;
use heronite_data::DataPaths;
use heronite_i18n::tr;
use regex::Regex;
use serde_json::Value;

use crate::cmd::{Cancel, Legendary};
use crate::ops::{home, OpResult};
use crate::progress::ProgressParser;

/// `eosOverlayAppName`: the id Heroic gives the overlay in its status maps.
pub const APP_NAME: &str = "98bc04bc842e4906993fd6d6644ffb8d";

fn installed_file(data_paths: &DataPaths) -> PathBuf {
    data_paths.legendary_config_dir().join("overlay_install.json")
}

fn version_file(data_paths: &DataPaths) -> PathBuf {
    data_paths.legendary_config_dir().join("overlay_version.json")
}

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
    let file = installed_file(data_paths);
    if !file.exists() {
        return Status::default();
    }
    let doc: Value = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    let text = |k: &str| doc.get(k).and_then(Value::as_str).map(str::to_string);
    let install_path = text("install_path");
    if install_path.as_deref().is_some_and(|p| Path::new(p) != default_install_path(data_paths)) {
        log::warn!("EOS Overlay is not installed in default location, permission issues might arise");
    }
    Status { installed: true, version: text("version"), install_path }
}

/// `updateInfo`: `legendary status` refreshes `overlay_version.json` (it does
/// nothing without the overlay installed).
pub fn update_info(data_paths: &DataPaths, legendary: &Legendary) {
    if !status(data_paths).installed {
        return;
    }
    if let Err(err) = legendary.run(&["status"], &[]) {
        log::warn!("Updating EOS Overlay information: {err:#}");
    }
}

/// `getLatestVersion`: the build version from `overlay_version.json`
/// (refreshing it first when the file does not exist). Empty when unknown.
pub fn latest_version(data_paths: &DataPaths, legendary: &Legendary) -> String {
    let file = version_file(data_paths);
    if !file.exists() {
        if !status(data_paths).installed {
            return String::new();
        }
        update_info(data_paths, legendary);
        if !file.exists() {
            log::error!("EOS Overlay information not found after manual update. User is probably not logged in anymore");
            return String::new();
        }
    }
    std::fs::read_to_string(&file)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.pointer("/data/buildVersion").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default()
}

fn failed(out: &crate::cmd::Output) -> OpResult {
    let why = out.error_lines();
    OpResult {
        ok: false,
        aborted: false, warning: false,
        message: if why.is_empty() { tr!("legendary exited with {code}", code = format!("{:?}", out.code)) } else { why },
    }
}

/// `install`: first asks legendary for the download size (without `-y`, so it
/// stops at the question), then installs (or updates) with `-y` reporting the
/// progress as a percentage.
pub fn install(data_paths: &DataPaths, legendary: &Legendary, cancel: &std::sync::Arc<Cancel>, on_percent: &dyn Fn(f32)) -> OpResult {
    static SIZE: OnceLock<Regex> = OnceLock::new();
    let size_re = SIZE.get_or_init(|| Regex::new(r"Download size: ([\d.]+) MiB").expect("valid regex"));
    let path = default_install_path(data_paths).to_string_lossy().into_owned();

    let probe = Cancel::new();
    let mut download_size = 0.0f64;
    let asked = legendary.run_streaming(&["eos-overlay", "install", "--path", &path], &[], &probe, |line| {
        if let Some(c) = size_re.captures(line) {
            download_size = c[1].parse::<f64>().unwrap_or(0.) * 1024. * 1024.;
            probe.request();
        }
    });
    if let Err(err) = asked {
        return OpResult { ok: false, aborted: false, warning: false, message: format!("{err:#}") };
    }
    if cancel.is_requested() {
        return OpResult { ok: false, aborted: true, warning: false, message: tr!("Cancelled").into() };
    }

    let mut parser = ProgressParser::new(Some(download_size));
    let out = legendary.run_streaming(&["eos-overlay", "install", "-y", "--path", &path], &[], cancel, |line| {
        if let Some(p) = parser.feed(line) {
            on_percent(p.percent as f32);
        }
    });
    match out {
        Err(err) => OpResult { ok: false, aborted: false, warning: false, message: format!("{err:#}") },
        Ok(out) if out.aborted => OpResult { ok: false, aborted: true, warning: false, message: tr!("Cancelled").into() },
        Ok(out) if out.success() => OpResult { ok: true, aborted: false, warning: false, message: tr!("EOS Overlay installed").into() },
        Ok(out) => failed(&out),
    }
}

/// `remove` (the caller asks the user first).
pub fn remove(legendary: &Legendary) -> OpResult {
    match legendary.run(&["eos-overlay", "remove", "-y"], &[]) {
        Err(err) => OpResult { ok: false, aborted: false, warning: false, message: format!("{err:#}") },
        Ok(out) if out.success() => OpResult { ok: true, aborted: false, warning: false, message: tr!("EOS Overlay uninstalled").into() },
        Ok(out) => failed(&out),
    }
}

fn prefix_args<'a>(action: &'a str, prefix: Option<&'a str>) -> Vec<&'a str> {
    let mut args = vec!["eos-overlay", action];
    if let Some(p) = prefix {
        args.extend(["--prefix", p]);
    }
    args
}

/// `enable` for a valid prefix folder (see [`prefix_folder`]).
pub fn enable(legendary: &Legendary, prefix: Option<&str>) -> OpResult {
    switch(legendary, "enable", prefix, tr!("EOS Overlay enabled"))
}

/// `disable` for a valid prefix folder.
pub fn disable(legendary: &Legendary, prefix: Option<&str>) -> OpResult {
    switch(legendary, "disable", prefix, tr!("EOS Overlay disabled"))
}

fn switch(legendary: &Legendary, action: &str, prefix: Option<&str>, done: &str) -> OpResult {
    match legendary.run(&prefix_args(action, prefix), &[]) {
        Err(err) => OpResult { ok: false, aborted: false, warning: false, message: format!("{err:#}") },
        Ok(out) if out.success() => OpResult { ok: true, aborted: false, warning: false, message: done.into() },
        Ok(out) => failed(&out),
    }
}

/// `isEnabled` (`info`): legendary prints `Overlay enabled: Yes|No`.
pub fn is_enabled(legendary: &Legendary, prefix: Option<&str>) -> bool {
    match legendary.run(&prefix_args("info", prefix), &[]) {
        Ok(out) => parse_enabled(&format!("{}\n{}", out.stdout, out.stderr)),
        Err(err) => {
            log::warn!("Checking if EOS Overlay is enabled: {err:#}");
            false
        }
    }
}

/// Whether `legendary eos-overlay info` says the overlay is enabled.
pub fn parse_enabled(output: &str) -> bool {
    output.lines().any(|l| l.contains("Overlay enabled") && l.contains("Yes"))
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
    use crate::cmd::testing::{calls, fake};
    use std::sync::Arc;

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
        std::fs::write(installed_file(&data_paths), r#"{"version": "1.1.9", "install_path": "/x/eos_overlay", "managed": true}"#).unwrap();
        let s = status(&data_paths);
        assert!(s.installed);
        assert_eq!(s.version.as_deref(), Some("1.1.9"));
        assert_eq!(s.install_path.as_deref(), Some("/x/eos_overlay"));
    }

    #[test]
    fn latest_version_comes_from_the_version_file() {
        let (d, data_paths) = fixture();
        let lg = fake(d.path(), "true");
        // Not installed: nothing to ask.
        assert_eq!(latest_version(&data_paths, &lg), "");
        assert!(calls(d.path()).is_empty());
        std::fs::write(installed_file(&data_paths), r#"{"version": "1"}"#).unwrap();
        // Installed but no version file: `legendary status` is asked (and a
        // missing file afterwards means unknown).
        assert_eq!(latest_version(&data_paths, &lg), "");
        assert_eq!(calls(d.path()), ["status"]);
        std::fs::write(version_file(&data_paths), r#"{"data": {"buildVersion": "1.2.0"}}"#).unwrap();
        assert_eq!(latest_version(&data_paths, &lg), "1.2.0");
    }

    #[test]
    fn install_asks_the_size_first_then_installs_with_progress() {
        let (d, data_paths) = fixture();
        let body = r#"
if [ "$3" = "-y" ]; then
  printf '[DLManager] INFO: = Progress: 50.00%% (1/2), Running for 00:00:01, ETA: 00:00:01\n'
  printf '[DLManager] INFO:  - Downloaded: 5.00 MiB, Written: 5.00 MiB\n'
  printf '[DLManager] INFO:  + Download\t- 5.00 MiB/s (raw) / 5.00 MiB/s (decompressed)\n'
  printf '[DLManager] INFO:  + Disk\t- 5.00 MiB/s (write) / 0.00 MiB/s (read)\n'
else
  echo '[DLManager] INFO: Download size: 10.00 MiB (Compression savings: 1.0%)'
  sleep 5
fi"#;
        let lg = fake(d.path(), body);
        let seen = std::sync::Mutex::new(Vec::new());
        let r = install(&data_paths, &lg, &Cancel::new(), &|p| seen.lock().unwrap().push(p));
        assert!(r.ok, "{}", r.message);
        let path = default_install_path(&data_paths).to_string_lossy().into_owned();
        assert_eq!(calls(d.path()), [format!("eos-overlay install --path {path}"), format!("eos-overlay install -y --path {path}")]);
        assert_eq!(*seen.lock().unwrap(), [50.0f32]);
    }

    #[test]
    fn install_can_be_cancelled_and_reports_errors() {
        let (d, data_paths) = fixture();
        let lg = fake(d.path(), "echo 'Download size: 1.00 MiB'; echo '[cli] ERROR: boom' >&2; exit 1");
        let r = install(&data_paths, &lg, &Cancel::new(), &|_| {});
        assert!(!r.ok && r.message.contains("boom"), "{}", r.message);

        let cancel: Arc<Cancel> = Cancel::new();
        cancel.request();
        let r = install(&data_paths, &lg, &cancel, &|_| {});
        assert!(r.aborted);
    }

    #[test]
    fn enable_disable_remove_and_info_arguments() {
        let (d, _heroic) = fixture();
        let lg = fake(d.path(), "echo '[cli] INFO: Overlay enabled: Yes'");
        assert!(enable(&lg, Some("/pfx")).ok);
        assert!(disable(&lg, None).ok);
        assert!(remove(&lg).ok);
        assert!(is_enabled(&lg, Some("/pfx")));
        assert_eq!(
            calls(d.path()),
            ["eos-overlay enable --prefix /pfx", "eos-overlay disable", "eos-overlay remove -y", "eos-overlay info --prefix /pfx"]
        );
        let no = fake(d.path(), "echo '[cli] INFO: Overlay enabled: No'");
        assert!(!is_enabled(&no, None));
        let none = fake(d.path(), "echo '[cli] INFO: No Legendary-managed installation found.'");
        assert!(!is_enabled(&none, None));
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
