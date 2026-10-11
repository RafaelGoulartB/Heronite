//! Cloud saves (`LegendaryGame.syncSaves` and `getDefaultLegendarySavePath`).

use anyhow::{bail, Result};
use heronite_data::json_store;
use heronite_data::DataPaths;
use heronite_launch::env::setup_wine_env_vars;
use heronite_launch::settings::load_game_settings;
use heronite_launch::{PlanContext, SystemProbe};
use serde_json::Value;

use crate::cmd::{Cancel, Legendary};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncMode {
    /// Upload and download as needed (`legendary sync-saves`).
    Both,
    ForceDownload,
    ForceUpload,
}

impl SyncMode {
    fn flag(self) -> Option<&'static str> {
        match self {
            SyncMode::Both => None,
            SyncMode::ForceDownload => Some("--force-download"),
            SyncMode::ForceUpload => Some("--force-upload"),
        }
    }
}

/// `sync-saves` arguments (Heroic's order: the mode flag, the path, `-y`).
pub fn sync_args(app_name: &str, saves_path: &str, flag: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = vec!["sync-saves".into(), app_name.into()];
    if let Some(f) = flag.filter(|f| !f.is_empty()) {
        args.push(f.into());
    }
    args.extend(["--save-path".into(), saves_path.into(), "-y".into()]);
    args
}

/// Runs `legendary sync-saves` for `app_name` with `saves_path`; returns
/// legendary's output. Blocking: call off the UI thread.
pub fn sync(data_paths: &DataPaths, app_name: &str, saves_path: &str, mode: SyncMode) -> Result<String> {
    sync_flag(data_paths, app_name, saves_path, mode.flag())
}

/// Like [`sync`] with any of Heroic's manual-sync flags: `--skip-upload`
/// (download), `--skip-download` (upload), `--force-download`,
/// `--force-upload`, or none.
pub fn sync_flag(data_paths: &DataPaths, app_name: &str, saves_path: &str, flag: Option<&str>) -> Result<String> {
    if saves_path.trim().is_empty() {
        bail!("No path provided.");
    }
    let legendary = Legendary::locate(data_paths)?;
    sync_with(&legendary, app_name, saves_path, flag)
}

pub(crate) fn sync_with(legendary: &Legendary, app_name: &str, saves_path: &str, flag: Option<&str>) -> Result<String> {
    let args = sync_args(app_name, saves_path, flag);
    let mut output = String::new();
    let out = legendary.run_streaming(&args, &[], &Cancel::new(), |line| {
        output.push_str(line);
        output.push('\n');
    })?;
    if !out.success() {
        log::error!("Failed to sync saves for {app_name}: {}", out.error_lines());
    }
    Ok(output)
}

fn installed_save_path(data_paths: &DataPaths, app_name: &str) -> Option<String> {
    let doc = json_store::read_value(&data_paths.legendary_config_dir().join("installed.json")).ok()?;
    doc.get(app_name)?.get("save_path")?.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

/// Default save location legendary computes for the game in its Wine
/// prefix (Heroic's "Cloud saves" tab auto-detection), if any.
///
/// A path legendary already stored for the game is returned as it is (Heroic
/// discards it by editing `installed.json`, which this app never touches).
/// Otherwise `legendary sync-saves --skip-upload --skip-download
/// --accept-path` runs with the game's Wine environment, which makes
/// legendary resolve and store the path.
pub fn detect_path(data_paths: &DataPaths, app_name: &str) -> Result<Option<String>> {
    if let Some(path) = installed_save_path(data_paths, app_name) {
        return Ok(Some(path));
    }
    let legendary = Legendary::locate(data_paths)?;
    detect_path_with(data_paths, &legendary, app_name)
}

pub(crate) fn detect_path_with(data_paths: &DataPaths, legendary: &Legendary, app_name: &str) -> Result<Option<String>> {
    let ctx = PlanContext::load(data_paths);
    let probe = SystemProbe;
    let settings = load_game_settings(data_paths, app_name, &ctx.global, &probe);
    let prefix = settings.wine_prefix.clone();
    if !prefix.is_empty() && !std::path::Path::new(&prefix).join("user.reg").exists() && !std::path::Path::new(&prefix).join("pfx/user.reg").exists() {
        bail!("The Wine prefix of this game does not exist yet; launch the game once or set the save folder by hand");
    }
    let env: Vec<(String, String)> = setup_wine_env_vars(&settings, "0", &ctx.runtime_dir(), &probe).into_iter().collect();
    let out = legendary.run(&["sync-saves", app_name, "--skip-upload", "--skip-download", "--accept-path"], &env)?;
    if !out.success() {
        log::warn!("Unable to compute the default save path for {app_name}: {}", out.error_lines());
    }
    Ok(installed_save_path(data_paths, app_name))
}

/// Whether the game supports cloud saves (the library's `cloud_save_enabled`).
pub fn supported(data_paths: &DataPaths, app_name: &str) -> bool {
    let lib = json_store::read_value(&data_paths.legendary_library()).unwrap_or(Value::Null);
    lib.get("library")
        .and_then(Value::as_array)
        .and_then(|l| l.iter().find(|g| g.get("app_name").and_then(Value::as_str) == Some(app_name)))
        .and_then(|g| g.get("cloud_save_enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::testing::{calls, fake};
    use serde_json::json;

    #[test]
    fn sync_arguments() {
        assert_eq!(sync_args("A", "/s", None), ["sync-saves", "A", "--save-path", "/s", "-y"]);
        assert_eq!(
            sync_args("A", "/s", Some("--force-upload")),
            ["sync-saves", "A", "--force-upload", "--save-path", "/s", "-y"]
        );
        assert_eq!(SyncMode::ForceDownload.flag(), Some("--force-download"));
        assert_eq!(SyncMode::Both.flag(), None);
    }

    #[test]
    fn sync_returns_the_output_and_needs_a_path() {
        let dir = tempfile::tempdir().unwrap();
        let lg = fake(dir.path(), "echo 'Cloud saves are up to date'; echo '[cli] INFO: done' >&2");
        let out = sync_with(&lg, "A", "/saves", Some("--skip-upload")).unwrap();
        assert!(out.contains("up to date") && out.contains("done"), "{out}");
        assert_eq!(calls(dir.path()), ["sync-saves A --skip-upload --save-path /saves -y"]);
        let data_paths = DataPaths::at(dir.path());
        assert!(sync(&data_paths, "A", "  ", SyncMode::Both).is_err());
    }

    #[test]
    fn stored_save_paths_are_returned_without_running_legendary() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(data_paths.legendary_config_dir()).unwrap();
        std::fs::write(
            data_paths.legendary_config_dir().join("installed.json"),
            json!({"A": {"save_path": "/pfx/drive_c/saves"}, "B": {"save_path": null}}).to_string(),
        )
        .unwrap();
        assert_eq!(detect_path(&data_paths, "A").unwrap().as_deref(), Some("/pfx/drive_c/saves"));
    }

    #[test]
    fn detection_runs_legendary_in_the_games_prefix_and_reads_the_result() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("h"));
        std::fs::create_dir_all(data_paths.legendary_config_dir()).unwrap();
        let prefix = dir.path().join("pfx");
        std::fs::create_dir_all(&prefix).unwrap();
        std::fs::write(prefix.join("user.reg"), "").unwrap();
        heronite_data::game_config::set_game_setting(&data_paths, "A", "/home/x", "winePrefix", json!(prefix.to_str().unwrap())).unwrap();
        heronite_data::game_config::set_game_setting(
            &data_paths,
            "A",
            "/home/x",
            "wineVersion",
            json!({"bin": "/w/bin/wine", "name": "Wine", "type": "wine"}),
        )
        .unwrap();
        // The fake "computes" the path the way legendary stores it.
        let inst = data_paths.legendary_config_dir().join("installed.json");
        let body = format!("echo '{{\"A\": {{\"save_path\": \"/pfx/saves\"}}}}' > \"{}\"; echo WINEPREFIX=$WINEPREFIX >> \"{}\"", inst.display(), dir.path().join("calls.log").display());
        let lg = fake(dir.path(), &body);
        let found = detect_path_with(&data_paths, &lg, "A").unwrap();
        assert_eq!(found.as_deref(), Some("/pfx/saves"));
        let log = std::fs::read_to_string(dir.path().join("calls.log")).unwrap();
        assert!(log.contains("sync-saves A --skip-upload --skip-download --accept-path"), "{log}");
        assert!(log.contains(&format!("WINEPREFIX={}", prefix.display())), "{log}");
    }

    #[test]
    fn a_missing_prefix_is_explained() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("h"));
        heronite_data::game_config::set_game_setting(&data_paths, "A", "/home/x", "winePrefix", json!("/nonexistent/pfx")).unwrap();
        let lg = fake(dir.path(), "true");
        let err = detect_path_with(&data_paths, &lg, "A").unwrap_err().to_string();
        assert!(err.contains("prefix"), "{err}");
        assert!(calls(dir.path()).is_empty());
    }
}
