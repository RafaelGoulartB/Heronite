//! Cloud saves (`LegendaryGame.syncSaves` and `getDefaultLegendarySavePath`),
//! natively: [`heronite_epic_api::saves`] syncs them the way `legendary
//! sync-saves` did, with the save folder resolved on Windows or in the
//! game's Wine prefix.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use heronite_data::json_store;
use heronite_data::DataPaths;
use heronite_epic_api::saves::{self as api, Place, SaveState, SyncOutcome};
use heronite_i18n::tr;
use heronite_launch::host::{run_mode, Os, RunMode};
use heronite_launch::settings::load_game_settings;
use heronite_launch::{PlanContext, SystemProbe};
use serde_json::Value;

use crate::client::Epic;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncMode {
    /// Upload or download, whichever side is newer.
    Both,
    ForceDownload,
    ForceUpload,
}

impl SyncMode {
    fn api(self) -> api::SyncMode {
        match self {
            SyncMode::Both => api::SyncMode::Both,
            SyncMode::ForceDownload => api::SyncMode::ForceDownload,
            SyncMode::ForceUpload => api::SyncMode::ForceUpload,
        }
    }
}

/// The mode of one of Heroic's manual-sync flags: `--skip-upload`
/// (download), `--skip-download` (upload), `--force-download`,
/// `--force-upload`, or none.
pub fn mode_of_flag(flag: Option<&str>) -> api::SyncMode {
    match flag.map(str::trim) {
        Some("--skip-upload") => api::SyncMode::DownloadOnly,
        Some("--skip-download") => api::SyncMode::UploadOnly,
        Some("--force-download") => api::SyncMode::ForceDownload,
        Some("--force-upload") => api::SyncMode::ForceUpload,
        _ => api::SyncMode::Both,
    }
}

/// Syncs `app_name`'s saves in `saves_path`; returns what was done, for
/// the user. Blocking, network: call it off the UI thread.
pub fn sync(data_paths: &DataPaths, app_name: &str, saves_path: &str, mode: SyncMode) -> Result<String> {
    sync_mode(data_paths, &Epic::new(data_paths), app_name, saves_path, mode.api())
}

/// Like [`sync`] with any of Heroic's manual-sync flags (see [`mode_of_flag`]).
pub fn sync_flag(data_paths: &DataPaths, app_name: &str, saves_path: &str, flag: Option<&str>) -> Result<String> {
    sync_mode(data_paths, &Epic::new(data_paths), app_name, saves_path, mode_of_flag(flag))
}

pub(crate) fn sync_mode(data_paths: &DataPaths, epic: &Epic, app_name: &str, saves_path: &str, mode: api::SyncMode) -> Result<String> {
    if saves_path.trim().is_empty() {
        bail!(tr!("No path provided."));
    }
    let session = epic.session()?;
    let place_dir = save_place(data_paths, app_name);
    let place = place_of(place_dir.as_deref());
    let (outcome, path) = epic.client.sync_saves(&epic.config_dir, &session, app_name, Some(saves_path), mode, place).map_err(|e| {
        log::error!("Failed to sync saves for {app_name}: {e:#}");
        e
    })?;
    log::info!("epic: cloud saves of {app_name} in {}: {outcome:?}", path.display());
    Ok(describe(&outcome))
}

/// What a sync did, for the user.
fn describe(outcome: &SyncOutcome) -> String {
    match outcome {
        SyncOutcome::Uploaded => tr!("The saves were uploaded to the cloud").into(),
        SyncOutcome::Downloaded => tr!("The saves were downloaded from the cloud").into(),
        SyncOutcome::UpToDate => tr!("Cloud saves are up to date").into(),
        SyncOutcome::NoSave => tr!("There are no saves to sync yet").into(),
        SyncOutcome::Skipped(SaveState::LocalNewer) => tr!("The saves on this PC are newer: nothing was downloaded").into(),
        SyncOutcome::Skipped(_) => tr!("The saves in the cloud are newer: nothing was uploaded").into(),
        SyncOutcome::Conflict => tr!("The saves changed both on this PC and in the cloud: upload or download them by hand").into(),
    }
}

/// Where the game's Windows folders are: this PC on Windows (and a Mac
/// build's), else its Wine prefix (the folder with `drive_c`, inside `pfx`
/// for a Proton prefix). `None` on Windows.
fn save_place(data_paths: &DataPaths, app_name: &str) -> Option<PathBuf> {
    if run_mode(Os::Windows) == RunMode::Native {
        return None;
    }
    let ctx = PlanContext::load(data_paths);
    let settings = load_game_settings(data_paths, app_name, &ctx.global, &SystemProbe);
    let prefix = PathBuf::from(&settings.wine_prefix);
    if prefix.as_os_str().is_empty() {
        return Some(prefix);
    }
    Some(if prefix.join("pfx").join("user.reg").exists() { prefix.join("pfx") } else { prefix })
}

fn place_of(prefix: Option<&Path>) -> Place<'_> {
    match prefix {
        None => Place::Windows,
        Some(p) => Place::Prefix(p),
    }
}

fn installed_save_path(data_paths: &DataPaths, app_name: &str) -> Option<String> {
    let doc = json_store::read_value(&data_paths.legendary_config_dir().join("installed.json")).ok()?;
    doc.get(app_name)?.get("save_path")?.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

/// Default save location of the game, in its Wine prefix on Linux (Heroic's
/// "Cloud saves" tab auto-detection), if any; it is stored for the game.
///
/// A path already stored for the game is returned as it is (Heroic discards
/// it by editing `installed.json`, which this app never touches).
pub fn detect_path(data_paths: &DataPaths, app_name: &str) -> Result<Option<String>> {
    if let Some(path) = installed_save_path(data_paths, app_name) {
        return Ok(Some(path));
    }
    detect_path_with(data_paths, &Epic::new(data_paths), app_name)
}

pub(crate) fn detect_path_with(data_paths: &DataPaths, epic: &Epic, app_name: &str) -> Result<Option<String>> {
    let place_dir = save_place(data_paths, app_name);
    if let Some(prefix) = &place_dir {
        if prefix.as_os_str().is_empty() || !prefix.join("user.reg").exists() {
            bail!(tr!("The Wine prefix of this game does not exist yet; launch the game once or set the save folder by hand"));
        }
    }
    let account = epic
        .session()
        .map(|s| s.account_id().to_string())
        .or_else(|_| heronite_epic_api::Session::load(&epic.config_dir).ok().flatten().map(|s| s.account_id().to_string()).ok_or(()));
    let account = account.unwrap_or_default();
    let path = match api::default_save_path(&epic.config_dir, app_name, &account, place_of(place_dir.as_deref())) {
        Ok(p) => p,
        Err(err) => {
            log::warn!("Unable to compute the default save path for {app_name}: {err:#}");
            return Ok(None);
        }
    };
    if api::has_unresolved(&path) {
        log::warn!("The save path of {app_name} could not be resolved: {}", path.display());
        return Ok(None);
    }
    let text = path.to_string_lossy().into_owned();
    if let Some(mut game) = heronite_epic_api::installed::get(&epic.config_dir, app_name) {
        game.save_path = Some(text.clone());
        heronite_epic_api::installed::set(&epic.config_dir, &game)?;
    }
    Ok(Some(text))
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
    use crate::download::testing::{sign_in_and_own, Server};
    use serde_json::json;

    #[test]
    fn heroics_flags_pick_the_mode() {
        assert_eq!(mode_of_flag(Some("--skip-upload")), api::SyncMode::DownloadOnly);
        assert_eq!(mode_of_flag(Some("--skip-download")), api::SyncMode::UploadOnly);
        assert_eq!(mode_of_flag(Some("--force-upload")), api::SyncMode::ForceUpload);
        assert_eq!(mode_of_flag(Some("--force-download")), api::SyncMode::ForceDownload);
        assert_eq!(mode_of_flag(None), api::SyncMode::Both);
        assert_eq!(SyncMode::ForceDownload.api(), api::SyncMode::ForceDownload);
    }

    #[test]
    fn a_sync_needs_a_path() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path());
        assert!(sync(&data_paths, "A", "  ", SyncMode::Both).is_err());
    }

    #[test]
    fn stored_save_paths_are_returned_as_they_are() {
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

    #[cfg(unix)]
    #[test]
    fn the_default_folder_is_found_in_the_games_prefix_and_stored() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("h"));
        let cfg = data_paths.legendary_config_dir();
        sign_in_and_own(&cfg, "A", "Game");
        // The game's folder template.
        let meta = cfg.join("metadata/A.json");
        let mut doc: Value = serde_json::from_str(&std::fs::read_to_string(&meta).unwrap()).unwrap();
        doc["metadata"]["customAttributes"]["CloudSaveFolder"] = json!({"type": "STRING", "value": "{AppData}/Game/Saves"});
        std::fs::write(&meta, doc.to_string()).unwrap();
        let game =
            heronite_epic_api::installed::InstalledGame { app_name: "A".into(), install_path: "/games/A".into(), ..Default::default() };
        heronite_epic_api::installed::set(&cfg, &game).unwrap();
        let prefix = dir.path().join("pfx");
        std::fs::create_dir_all(prefix.join("drive_c/users/steamuser/AppData/Local")).unwrap();
        std::fs::write(
            prefix.join("user.reg"),
            r#"WINE REGISTRY Version 2
;; All keys relative to \\User\\S-1-5-21-0-0-0-1000

[Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Shell Folders] 1700000000
"AppData"="C:\\users\\steamuser\\AppData\\Roaming"
"Local AppData"="C:\\users\\steamuser\\AppData\\Local"
"Personal"="C:\\users\\steamuser\\Documents"
"#,
        )
        .unwrap();
        heronite_data::game_config::set_game_setting(&data_paths, "A", "/home/x", "winePrefix", json!(prefix.to_str().unwrap())).unwrap();
        let server = Server::start();
        let found = detect_path_with(&data_paths, &server.epic(&cfg), "A").unwrap().expect("a folder");
        assert!(found.starts_with(prefix.join("drive_c/users").to_str().unwrap()), "{found}");
        assert!(found.ends_with("Game/Saves"), "{found}");
        assert_eq!(installed_save_path(&data_paths, "A").as_deref(), Some(found.as_str()));
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_prefix_is_explained() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("h"));
        heronite_data::game_config::set_game_setting(&data_paths, "A", "/home/x", "winePrefix", json!("/nonexistent/pfx")).unwrap();
        let server = Server::start();
        let err = detect_path_with(&data_paths, &server.epic(&data_paths.legendary_config_dir()), "A").unwrap_err().to_string();
        assert!(err.contains("prefix"), "{err}");
    }
}
