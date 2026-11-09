//! `store/download-manager.json` in Heroic's `DMQueueElement` format: the
//! persisted queue and the download history.

use anyhow::Result;
use heronite_data::json_store;
use heronite_data::DataPaths;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// `InstallParams` (+ `InstallArgs`). Unknown keys survive a round trip.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallParams {
    #[serde(default)]
    pub app_name: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub install_dlcs: Vec<String>,
    #[serde(default)]
    pub sdl_list: Vec<String>,
    #[serde(default)]
    pub install_language: String,
    #[serde(default)]
    pub runner: String,
    #[serde(default)]
    pub platform_to_install: String,
    /// The game's `GameInfo` at the time it was queued.
    #[serde(default)]
    pub game_info: Value,
    /// Download size as text (`25.87 GiB`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    /// "Force install": the tool skips its own free space check (only
    /// legendary has one that can be skipped). Not in Heroic's format.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ignore_free_space: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl InstallParams {
    /// Title from the stored `GameInfo` (falls back to the app name).
    pub fn title(&self) -> String {
        self.game_info.get("title").and_then(Value::as_str).filter(|t| !t.is_empty()).unwrap_or(&self.app_name).to_string()
    }

    /// Cover shown in the lists: the tall art (`art_square`), else the wide one.
    pub fn cover(&self) -> Option<String> {
        self.first_art(&["art_square", "art_cover"])
    }

    /// Wide art (`art_cover`, else `art_background`), behind the running download.
    pub fn art(&self) -> Option<String> {
        self.first_art(&["art_cover", "art_background"])
    }

    fn first_art(&self, keys: &[&str]) -> Option<String> {
        keys.iter()
            .filter_map(|k| self.game_info.get(*k).and_then(Value::as_str))
            .find(|s| !s.is_empty() && *s != "fallback")
            .map(str::to_string)
    }

    pub fn folder_name(&self) -> Option<String> {
        self.game_info.get("folder_name").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
    }

    /// The stored `GameInfo` is a DLC: it installs into its base game's folder.
    pub fn is_dlc(&self) -> bool {
        self.game_info.pointer("/install/is_dlc").and_then(Value::as_bool).unwrap_or(false)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ElementKind {
    Install,
    Update,
}

/// `DMStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DmStatus {
    Done,
    Error,
    Abort,
    Paused,
}

/// `DMQueueElement`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DmElement {
    pub params: InstallParams,
    #[serde(rename = "type")]
    pub kind: ElementKind,
    #[serde(rename = "addToQueueTime", default)]
    pub add_to_queue_time: u64,
    #[serde(rename = "endTime", default)]
    pub end_time: u64,
    #[serde(rename = "startTime", default)]
    pub start_time: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<DmStatus>,
    /// Why it failed (`status: error`), for the history. Not in Heroic's format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl DmElement {
    pub fn new(kind: ElementKind, params: InstallParams) -> Self {
        Self { params, kind, add_to_queue_time: now_ms(), end_time: 0, start_time: 0, status: None, error: None, extra: Map::new() }
    }

    /// The folder "Stop, delete files" may delete: `(base path, folder)` of a
    /// game's own install. Never for an update (the folder is the installed
    /// game) or a DLC (it shares its base game's folder), nor for a path that
    /// is not absolute (`thirdParty` installs).
    pub fn partial_folder(&self) -> Option<(String, String)> {
        let absolute = std::path::Path::new(self.params.path.trim_matches('\'')).is_absolute();
        if self.kind != ElementKind::Install || self.params.is_dlc() || !absolute {
            return None;
        }
        self.params.folder_name().map(|f| (self.params.path.clone(), f))
    }

    pub fn same_game(&self, app_name: &str, runner: &str) -> bool {
        self.params.app_name == app_name && (runner.is_empty() || self.params.runner == runner)
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// The whole file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DmFile {
    #[serde(default)]
    pub finished: Vec<DmElement>,
    #[serde(default)]
    pub queue: Vec<DmElement>,
    /// The user paused the queue: it stays paused after a restart.
    #[serde(default, rename = "userPaused", skip_serializing_if = "std::ops::Not::not")]
    pub user_paused: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

pub fn path(data_paths: &DataPaths) -> std::path::PathBuf {
    data_paths.download_manager()
}

pub fn load(data_paths: &DataPaths) -> DmFile {
    match json_store::read::<DmFile>(&path(data_paths)) {
        Ok(f) => f,
        Err(err) => {
            log::warn!("reading download manager: {err:#}");
            DmFile::default()
        }
    }
}

/// Replaces the queue, the history and the pause flag, keeping every other key of the file.
pub fn save(data_paths: &DataPaths, queue: &[DmElement], finished: &[DmElement], user_paused: bool) -> Result<()> {
    json_store::update::<DmFile, _>(&path(data_paths), |f| {
        f.queue = queue.to_vec();
        f.finished = finished.to_vec();
        f.user_paused = user_paused;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Shape of an entry Heronite wrote.
    fn heronite_entry() -> Value {
        json!({
            "params": {
                "appName": "Calluna", "path": "/data/Games/Heroic/Control", "installDlcs": [], "sdlList": [],
                "installLanguage": "", "runner": "legendary", "platformToInstall": "Windows",
                "gameInfo": {"app_name": "Calluna", "title": "Control", "art_cover": "https://x/c.jpg", "folder_name": "Control"},
                "size": "25.87 GiB", "somethingNew": 1
            },
            "type": "install", "addToQueueTime": 1790051478808u64, "endTime": 1790052058257u64,
            "startTime": 1790051478816u64, "status": "done"
        })
    }

    #[test]
    fn covers_prefer_the_tall_art_and_art_the_wide_one() {
        let params = |info: Value| InstallParams { game_info: info, ..Default::default() };
        let p = params(json!({"art_square": "tall", "art_cover": "wide", "art_background": "bg"}));
        assert_eq!(p.cover().as_deref(), Some("tall"));
        assert_eq!(p.art().as_deref(), Some("wide"));
        let p = params(json!({"art_square": "fallback", "art_cover": "", "art_background": "bg"}));
        assert_eq!(p.cover(), None);
        assert_eq!(p.art().as_deref(), Some("bg"));
    }

    #[test]
    fn heronite_entries_round_trip_unchanged() {
        let e: DmElement = serde_json::from_value(heronite_entry()).unwrap();
        assert_eq!(e.kind, ElementKind::Install);
        assert_eq!(e.status, Some(DmStatus::Done));
        assert_eq!(e.params.title(), "Control");
        assert_eq!(e.params.cover().as_deref(), Some("https://x/c.jpg"));
        assert_eq!(e.params.art().as_deref(), Some("https://x/c.jpg"));
        assert_eq!(e.params.folder_name().as_deref(), Some("Control"));
        let back = serde_json::to_value(&e).unwrap();
        assert_eq!(back, heronite_entry());
    }

    #[test]
    fn file_keeps_unknown_keys_and_uses_tabs() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(dir.path().join("store")).unwrap();
        std::fs::write(path(&data_paths), json!({"finished": [heronite_entry()], "queue": [], "future": {"a": 1}}).to_string()).unwrap();
        let f = load(&data_paths);
        assert_eq!(f.finished.len(), 1);
        let mut queued = f.finished[0].clone();
        queued.status = None;
        queued.params.app_name = "Other".into();
        save(&data_paths, &[queued], &f.finished, false).unwrap();
        let raw = std::fs::read_to_string(path(&data_paths)).unwrap();
        assert!(raw.starts_with("{\n\t\"finished\": ["), "{raw}");
        let doc: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(doc["future"]["a"], 1);
        assert_eq!(doc["queue"][0]["params"]["appName"], "Other");
        assert!(doc["queue"][0].get("status").is_none());
    }

    #[test]
    fn the_pause_flag_round_trips_and_is_absent_when_not_paused() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(dir.path().join("store")).unwrap();
        // Files written before the flag existed load as not paused.
        std::fs::write(path(&data_paths), json!({"finished": [], "queue": [], "future": 1}).to_string()).unwrap();
        assert!(!load(&data_paths).user_paused);
        save(&data_paths, &[], &[], true).unwrap();
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(path(&data_paths)).unwrap()).unwrap();
        assert_eq!(doc["userPaused"], true);
        assert_eq!(doc["future"], 1, "the flag is not mistaken for an unknown key");
        assert!(load(&data_paths).user_paused);
        save(&data_paths, &[], &[], false).unwrap();
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(path(&data_paths)).unwrap()).unwrap();
        assert!(doc.get("userPaused").is_none());
    }

    #[test]
    fn missing_or_odd_files_are_empty() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path());
        assert!(load(&data_paths).queue.is_empty());
        std::fs::create_dir_all(dir.path().join("store")).unwrap();
        std::fs::write(path(&data_paths), "{}").unwrap();
        assert!(load(&data_paths).finished.is_empty());
    }
}
