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
use crate::models::{Extra, LocalGameMeta, LocalMetaFile};
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

/// In a game's Collection metadata: `{"minutes", "xboxMinutes"}`, the play
/// time the user set by hand and the minutes Xbox network had counted then
/// ([`note_playtime_edit`]), so an edit below Xbox's count holds.
pub const PLAYTIME_EDIT_KEY: &str = "xboxPlaytimeEdit";

/// In a game's sideload entry: the package full name its `install_size` was
/// measured for (an update changes it, and the size is measured again).
pub const PACKAGE_KEY: &str = "heroniteXboxPackage";

/// The brand (not translated): the "Library" of a game's details.
pub const NAME: &str = "Xbox";

/// The minutes and the last play the Xbox account counted for a game
/// (its metadata, [`MINUTES_KEY`] / [`LAST_PLAYED_KEY`]); 0 and `None`
/// for games of other sources.
///
/// After the user set the play time by hand ([`PLAYTIME_EDIT_KEY`]), Xbox's
/// count is what they set plus what Xbox counted since.
pub fn played(meta: Option<&LocalGameMeta>) -> (u64, Option<String>) {
    let Some(meta) = meta.filter(|m| m.source == SOURCE) else { return (0, None) };
    let mut minutes = meta.extra.get(MINUTES_KEY).and_then(whole_minutes).unwrap_or(0);
    if let Some(edit) = meta.extra.get(PLAYTIME_EDIT_KEY) {
        if let (Some(set), Some(then)) = (edit.get("minutes").and_then(whole_minutes), edit.get("xboxMinutes").and_then(whole_minutes)) {
            minutes = set + minutes.saturating_sub(then);
        }
    }
    let last = meta.extra.get(LAST_PLAYED_KEY).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_string);
    (minutes, last)
}

fn whole_minutes(v: &serde_json::Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_f64().filter(|m| *m > 0.0).map(|m| m as u64))
}

/// The user set an Xbox game's play time to `minutes` by hand: what Xbox
/// counted until now no longer raises it, only what it counts from now on
/// ([`PLAYTIME_EDIT_KEY`]). Nothing for other games, or before the Xbox
/// account counted anything for it.
pub fn note_playtime_edit(paths: &DataPaths, app_name: &str, minutes: u64) -> Result<()> {
    note_playtime_edits(paths, [(app_name, minutes)])
}

/// [`note_playtime_edit`] for several games, in one write.
pub fn note_playtime_edits<'a>(paths: &DataPaths, edits: impl IntoIterator<Item = (&'a str, u64)>) -> Result<()> {
    let edits: Vec<(&str, u64)> = edits.into_iter().collect();
    if edits.is_empty() {
        return Ok(());
    }
    json_store::update_if_changed::<LocalMetaFile, _>(&paths.local_meta(), |file| {
        for (app_name, minutes) in edits {
            let Some(meta) = file.games.get_mut(app_name).filter(|m| m.source == SOURCE) else { continue };
            match meta.extra.get(MINUTES_KEY).and_then(whole_minutes) {
                Some(xbox) => meta.extra.insert(PLAYTIME_EDIT_KEY.into(), serde_json::json!({"minutes": minutes, "xboxMinutes": xbox})),
                None => meta.extra.remove(PLAYTIME_EDIT_KEY),
            };
        }
        Ok(())
    })
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
    fn a_play_time_set_by_hand_holds_against_the_xbox_count() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let mut file = LocalMetaFile::default();
        let mut meta = LocalGameMeta { app_name: "xbox_a".into(), source: SOURCE.into(), ..Default::default() };
        meta.extra.insert(MINUTES_KEY.into(), serde_json::json!(754));
        file.games.insert("xbox_a".into(), meta);
        file.games.insert("gog_b".into(), LocalGameMeta { app_name: "gog_b".into(), source: "gog".into(), ..Default::default() });
        json_store::write(&paths.local_meta(), &file).unwrap();
        let read = |app: &str| json_store::read::<LocalMetaFile>(&paths.local_meta()).unwrap().games.get(app).cloned();

        // Lowered by hand below what Xbox counted: the edit holds…
        crate::playtime::set_playtime_minutes(&paths, "xbox_a", 100.0).unwrap();
        assert_eq!(played(read("xbox_a").as_ref()).0, 100);
        // …and what Xbox counts from now on adds to it.
        json_store::update_if_changed::<LocalMetaFile, _>(&paths.local_meta(), |f| {
            f.games.get_mut("xbox_a").unwrap().extra.insert(MINUTES_KEY.into(), serde_json::json!(800));
            Ok(())
        })
        .unwrap();
        assert_eq!(played(read("xbox_a").as_ref()).0, 146);

        // Other games' metadata is left alone.
        crate::playtime::set_playtime_minutes(&paths, "gog_b", 5.0).unwrap();
        assert!(read("gog_b").unwrap().extra.is_empty());
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
