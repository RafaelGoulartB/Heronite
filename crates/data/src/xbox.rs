//! The games of the Microsoft Store and PC Game Pass, which the Xbox app
//! installs as Windows packages (`heronite_xbox`). Windows only.
//!
//! They come into the Collection as sideload entries the system starts by
//! their application's id ([`LAUNCH_KIND`]; `launcherArgs` holds the AUMID,
//! `<family name>!<app id>`), whose `source` is [`SOURCE`]. Their folders
//! belong to the Xbox app: uninstalling, the Storage page and the trash never
//! delete them ([`crate::launchers::managed_paths`]).

use std::collections::BTreeSet;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::json_store;
use crate::models::{Extra, LocalGameMeta};
use crate::paths::DataPaths;

/// `launchKind` of the games Windows starts as packages.
pub const LAUNCH_KIND: &str = "xbox";

/// `source` of their Collection entries and the start of their app names
/// (`xbox_microsoft_624f8b84b80_8wekyb3d8bbwe`).
pub const SOURCE: &str = "xbox";

/// In a game's Collection metadata (`local_library/library.json`): its id on
/// Xbox network as its files write it (`79FC11E1`), when they name it.
pub const TITLE_ID_KEY: &str = "xboxTitleId";

/// In a game's Collection metadata: its product id in the Store
/// (`9NKX70BBCDRN`), when its files or the Xbox account name it.
pub const STORE_ID_KEY: &str = "xboxStoreId";

/// In a game's Collection metadata: the minutes Xbox network counted for it
/// (every device, Heronite's sessions included).
pub const MINUTES_KEY: &str = "xboxMinutes";

/// In a game's Collection metadata: when Xbox network last saw it played
/// (RFC 3339).
pub const LAST_PLAYED_KEY: &str = "xboxLastPlayed";

/// In a game's Collection metadata: `{"unlocked", "total", "gamerscore",
/// "totalGamerscore"}` of the Xbox account.
pub const ACHIEVEMENTS_KEY: &str = "xboxAchievements";

/// In a game's sideload entry: the package full name its `install_size` was
/// measured for (an update changes it, and the size is measured again).
pub const PACKAGE_KEY: &str = "heroniteXboxPackage";

/// The brand (not translated): the "Library" of a game's details.
pub const NAME: &str = "Xbox";

/// The minutes and the last play the Xbox account counted for a game
/// (its metadata, [`MINUTES_KEY`] / [`LAST_PLAYED_KEY`]); 0 and `None`
/// for games of other sources.
pub fn played(meta: Option<&LocalGameMeta>) -> (u64, Option<String>) {
    let Some(meta) = meta.filter(|m| m.source == SOURCE) else { return (0, None) };
    let minutes = meta.extra.get(MINUTES_KEY).and_then(|v| v.as_u64().or_else(|| v.as_f64().filter(|m| *m > 0.0).map(|m| m as u64)));
    let last = meta.extra.get(LAST_PLAYED_KEY).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_string);
    (minutes.unwrap_or(0), last)
}

/// The later of two play dates (RFC 3339, or the app's own
/// `2026-11-07T12:00:00.000Z`); one that does not read as a date loses.
pub fn later(ours: Option<String>, theirs: Option<String>) -> Option<String> {
    let parse = |s: &str| chrono::DateTime::parse_from_rfc3339(s).ok();
    match (ours, theirs) {
        (Some(a), Some(b)) => match (parse(&a), parse(&b)) {
            (_, None) => Some(a),
            (None, Some(_)) => Some(b),
            (Some(x), Some(y)) if y > x => Some(b),
            _ => Some(a),
        },
        (a, b) => a.or(b),
    }
}

/// When the games were last brought in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastSync {
    /// RFC 3339.
    #[serde(default)]
    pub at: String,
    /// Games installed then.
    #[serde(default)]
    pub games: usize,
}

/// `local_library/xbox.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XboxPrefs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<LastSync>,
    /// Games the user took out of the Collection (their app names): the
    /// sync leaves them out until they are installed again.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub removed: BTreeSet<String>,
    /// Games the user took out that the Xbox account would bring back as
    /// not installed: kept out of that until they are installed again.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub removed_titles: BTreeSet<String>,
    /// When the Xbox account was last read (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_synced: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl XboxPrefs {
    /// A missing or unreadable file: nothing synced yet.
    pub fn load(paths: &DataPaths) -> Self {
        json_store::read(&paths.xbox()).unwrap_or_default()
    }

    pub fn save(&self, paths: &DataPaths) -> Result<()> {
        json_store::write(&paths.xbox(), self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_later_play_and_the_xbox_minutes() {
        let a = Some("2026-01-05T10:00:00.000Z".to_string());
        let b = Some("2026-03-01T08:30:00Z".to_string());
        assert_eq!(later(a.clone(), b.clone()), b);
        assert_eq!(later(b.clone(), a.clone()), b);
        assert_eq!(later(a.clone(), None), a);
        assert_eq!(later(None, b.clone()), b);
        assert_eq!(later(a.clone(), Some("soon".into())), a);
        assert_eq!(later(Some("".into()).filter(|s: &String| !s.is_empty()), b.clone()), b);

        let mut meta = LocalGameMeta { source: SOURCE.into(), ..Default::default() };
        assert_eq!(played(Some(&meta)), (0, None));
        meta.extra.insert(MINUTES_KEY.into(), serde_json::json!(125));
        meta.extra.insert(LAST_PLAYED_KEY.into(), serde_json::json!("2026-03-01T08:30:00Z"));
        assert_eq!(played(Some(&meta)), (125, b.clone()));
        meta.source = "gog".into();
        assert_eq!(played(Some(&meta)), (0, None), "only the Xbox app's games");
    }

    #[test]
    fn prefs_round_trip_and_keep_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        assert_eq!(XboxPrefs::load(&paths), XboxPrefs::default());
        std::fs::create_dir_all(paths.xbox().parent().unwrap()).unwrap();
        std::fs::write(paths.xbox(), r#"{"last": {"at": "2026-11-03T10:00:00Z", "games": 2}, "removed": ["xbox_a"], "future": 1}"#)
            .unwrap();
        let prefs = XboxPrefs::load(&paths);
        assert_eq!(prefs.last.as_ref().map(|l| l.games), Some(2));
        assert!(prefs.removed.contains("xbox_a"));
        prefs.save(&paths).unwrap();
        let text = std::fs::read_to_string(paths.xbox()).unwrap();
        assert!(text.contains("\"future\": 1"), "{text}");
    }
}
