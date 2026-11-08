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
use crate::models::Extra;
use crate::paths::DataPaths;

/// `launchKind` of the games Windows starts as packages.
pub const LAUNCH_KIND: &str = "xbox";

/// `source` of their Collection entries and the start of their app names
/// (`xbox_microsoft_624f8b84b80_8wekyb3d8bbwe`).
pub const SOURCE: &str = "xbox";

/// In a game's Collection metadata (`local_library/library.json`): its id on
/// Xbox network as its files write it (`79FC11E1`), when they name it.
pub const TITLE_ID_KEY: &str = "xboxTitleId";

/// In a game's sideload entry: the package full name its `install_size` was
/// measured for (an update changes it, and the size is measured again).
pub const PACKAGE_KEY: &str = "heroniteXboxPackage";

/// The brand (not translated): the "Library" of a game's details.
pub const NAME: &str = "Xbox";

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
