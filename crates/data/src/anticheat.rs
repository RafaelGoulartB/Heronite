//! Anti-cheat compatibility of a game on Linux, from Heroic's copy of
//! areweanticheatyet.com (`<heroic root>/areweanticheatyet.json`).

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

use crate::json_store;
use crate::paths::DataPaths;

/// Status as written in the file (`Supported`, `Running`, `Planned`,
/// `Broken`, `Denied`) and the anti-cheat systems the game uses.
#[derive(Debug, Clone, PartialEq)]
pub struct AntiCheatInfo {
    pub status: String,
    pub anticheats: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Entry {
    name: String,
    status: String,
    anticheats: Vec<String>,
    /// Values are strings for Steam but objects for Epic.
    #[serde(rename = "storeIds")]
    store_ids: HashMap<String, Value>,
}

impl Entry {
    fn steam_id(&self) -> Option<&str> {
        self.store_ids.get("steam").and_then(Value::as_str)
    }

    fn info(&self) -> AntiCheatInfo {
        AntiCheatInfo { status: self.status.clone(), anticheats: self.anticheats.clone() }
    }
}

/// Lowercase alphanumerics only, so "ELDEN RING" equals "Elden Ring".
fn normalize(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Finds the game by Steam app id first, then by normalized title. A missing
/// or unreadable file gives `None`.
pub fn lookup(paths: &DataPaths, steam_app_id: Option<&str>, title: &str) -> Option<AntiCheatInfo> {
    let entries: Vec<Entry> = json_store::read(&paths.anticheat()).ok()?;
    let steam_app_id = steam_app_id.map(str::trim).filter(|id| !id.is_empty());
    if let Some(id) = steam_app_id {
        if let Some(entry) = entries.iter().find(|e| e.steam_id() == Some(id)) {
            return Some(entry.info());
        }
    }
    let wanted = normalize(title);
    if wanted.is_empty() {
        return None;
    }
    entries.iter().find(|e| normalize(&e.name) == wanted).map(Entry::info)
}

/// Short label for badges: "Easy Anti-Cheat" is "EAC"; others are unchanged.
pub fn short_name(anticheat: &str) -> String {
    match anticheat {
        "Easy Anti-Cheat" => "EAC".to_owned(),
        other => other.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"[
        {"name":"Elden Ring","status":"Supported","anticheats":["Easy Anti-Cheat"],"storeIds":{"steam":"1245620"},"extra":1},
        {"name":"Fortnite","status":"Denied","anticheats":["Easy Anti-Cheat","BattlEye"],"storeIds":{"epic":{"namespace":"fn","slug":"fortnite"}}},
        {"name":"Mystery"}
    ]"#;

    fn paths_with(content: Option<&str>) -> (tempfile::TempDir, DataPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        if let Some(c) = content {
            std::fs::write(paths.anticheat(), c).unwrap();
        }
        (dir, paths)
    }

    #[test]
    fn matches_by_steam_id_then_by_name() {
        let (_dir, paths) = paths_with(Some(FILE));
        let info = lookup(&paths, Some("1245620"), "whatever").unwrap();
        assert_eq!(info.status, "Supported");
        assert_eq!(info.anticheats, ["Easy Anti-Cheat"]);
        let info = lookup(&paths, None, "FORTNITE").unwrap();
        assert_eq!(info.status, "Denied");
        assert_eq!(info.anticheats.len(), 2);
        assert_eq!(lookup(&paths, Some("1"), "elden-ring").unwrap().status, "Supported");
    }

    #[test]
    fn steam_id_wins_over_title() {
        let (_dir, paths) = paths_with(Some(FILE));
        assert_eq!(lookup(&paths, Some("1245620"), "Fortnite").unwrap().status, "Supported");
    }

    #[test]
    fn lenient_parsing_and_no_match() {
        let (_dir, paths) = paths_with(Some(FILE));
        let info = lookup(&paths, None, "Mystery").unwrap();
        assert_eq!((info.status.as_str(), info.anticheats.len()), ("", 0));
        assert!(lookup(&paths, Some("999"), "Nope").is_none());
        assert!(lookup(&paths, None, "").is_none());
    }

    #[test]
    fn missing_or_broken_file_is_none() {
        let (_dir, paths) = paths_with(None);
        assert!(lookup(&paths, None, "Elden Ring").is_none());
        let (_dir, paths) = paths_with(Some("not json"));
        assert!(lookup(&paths, None, "Elden Ring").is_none());
    }

    #[test]
    fn short_names() {
        assert_eq!(short_name("Easy Anti-Cheat"), "EAC");
        assert_eq!(short_name("BattlEye"), "BattlEye");
        assert_eq!(short_name("Vanguard"), "Vanguard");
    }
}
