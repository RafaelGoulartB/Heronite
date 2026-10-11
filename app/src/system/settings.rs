//! The global settings the desktop integration reads
//! (`config.json` -> `defaultSettings`).

use heronite_data::game_config::{load_global, Settings};
use heronite_data::DataPaths;
use serde_json::Value;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SysSettings {
    /// Closing the window keeps the app in the tray.
    pub exit_to_tray: bool,
    /// Start without a window (needs the tray).
    pub start_in_tray: bool,
    pub no_tray_icon: bool,
    /// Close the window when a game starts (needs the tray).
    pub minimize_on_launch: bool,
    pub dark_tray_icon: bool,
    pub hide_window_on_protocol_launch: bool,
    /// Entries of the tray's recent games (Heroic: `maxRecentGames || 5`).
    pub max_recent: usize,
}

impl SysSettings {
    pub fn from_map(m: &Settings) -> Self {
        let flag = |k: &str| m.get(k).and_then(Value::as_bool).unwrap_or(false);
        let max_recent = m.get("maxRecentGames").and_then(Value::as_f64).filter(|n| *n >= 1.).map(|n| n as usize).unwrap_or(5);
        Self {
            exit_to_tray: flag("exitToTray"),
            start_in_tray: flag("startInTray"),
            no_tray_icon: flag("noTrayIcon"),
            minimize_on_launch: flag("minimizeOnLaunch"),
            dark_tray_icon: flag("darkTrayIcon"),
            hide_window_on_protocol_launch: flag("hideWindowOnProtocolLaunch"),
            max_recent,
        }
    }

    pub fn load(data_paths: &DataPaths) -> Self {
        let home = dirs::home_dir().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default();
        Self::from_map(&load_global(data_paths, &home))
    }
}

/// `games.recent` of `store/config.json` (most recent first), limited to
/// `limit`: `(appName, title)`.
pub fn recent_games(data_paths: &DataPaths, limit: usize) -> Vec<(String, String)> {
    let doc = heronite_data::json_store::read_value(&data_paths.store_config()).unwrap_or(Value::Null);
    doc.get("games")
        .and_then(|g| g.get("recent"))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|e| {
                    let app = e.get("appName")?.as_str().filter(|s| !s.is_empty())?;
                    let title = e.get("title").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(app);
                    Some((app.to_string(), title.to_string()))
                })
                .take(limit)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_and_overrides() {
        let m = Settings::new();
        let s = SysSettings::from_map(&m);
        assert!(!s.exit_to_tray && !s.start_in_tray && !s.no_tray_icon);
        assert_eq!(s.max_recent, 5);
        let m: Settings = json!({"exitToTray": true, "startInTray": true, "darkTrayIcon": true, "maxRecentGames": 3, "hideWindowOnProtocolLaunch": true})
            .as_object()
            .unwrap()
            .clone();
        let s = SysSettings::from_map(&m);
        assert!(s.exit_to_tray && s.start_in_tray && s.dark_tray_icon && s.hide_window_on_protocol_launch);
        assert_eq!(s.max_recent, 3);
        // 0 means "use the default", like `maxRecentGames || 5`.
        let m: Settings = json!({"maxRecentGames": 0}).as_object().unwrap().clone();
        assert_eq!(SysSettings::from_map(&m).max_recent, 5);
    }

    #[test]
    fn recent_games_are_read_in_order_and_limited() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(dir.path().join("store")).unwrap();
        std::fs::write(
            data_paths.store_config(),
            r#"{"games":{"recent":[{"appName":"a","title":"A"},{"appName":"b"},{"title":"no app"},{"appName":"c","title":"C"}]}}"#,
        )
        .unwrap();
        assert_eq!(recent_games(&data_paths, 5), vec![("a".into(), "A".into()), ("b".into(), "b".into()), ("c".into(), "C".into())]);
        assert_eq!(recent_games(&data_paths, 1).len(), 1);
        assert!(recent_games(&DataPaths::at(dir.path().join("none")), 5).is_empty());
    }
}
