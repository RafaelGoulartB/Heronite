//! Games of the other Linux launchers (Heroic, Lutris, Faugus) imported into
//! the Collection (`heronite_import`), and which launchers the user turned
//! the import on for (`local_library/launcher-imports.json`).
//!
//! Lutris and Faugus keep starting their games: the entries hold the command
//! that asks them to ([`LAUNCH_KIND`]). Heroic's games added by hand are
//! copied as this app's own and start here.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::json_store;
use crate::models::Extra;
use crate::paths::DataPaths;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Launcher {
    Heroic,
    Lutris,
    Faugus,
}

impl Launcher {
    pub const ALL: [Launcher; 3] = [Launcher::Heroic, Launcher::Lutris, Launcher::Faugus];

    /// Also the `source` of the Collection entries it brought in.
    pub fn id(self) -> &'static str {
        match self {
            Launcher::Heroic => "heroic",
            Launcher::Lutris => "lutris",
            Launcher::Faugus => "faugus",
        }
    }

    /// The brand (not translated).
    pub fn name(self) -> &'static str {
        match self {
            Launcher::Heroic => "Heroic",
            Launcher::Lutris => "Lutris",
            Launcher::Faugus => "Faugus",
        }
    }

    pub fn from_source(source: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.id() == source)
    }
}

/// `launchKind` of the games another launcher starts (`launcherArgs` holds
/// its command line).
pub const LAUNCH_KIND: &str = "launcher";
/// `meta` key: the Wine prefix such a game runs in, to follow it when the
/// launcher, already open, takes the request and returns at once.
pub const PREFIX_KEY: &str = "heroniteLauncherPrefix";

/// When a launcher's games were last brought in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastImport {
    /// RFC 3339.
    #[serde(default)]
    pub at: String,
    /// Games the launcher had.
    #[serde(default)]
    pub games: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPrefs {
    /// Launchers whose games are imported (and kept in sync).
    #[serde(default)]
    pub enabled: BTreeSet<Launcher>,
    #[serde(default)]
    pub last: BTreeMap<Launcher, LastImport>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl ImportPrefs {
    /// A missing or unreadable file: nothing imported.
    pub fn load(paths: &DataPaths) -> Self {
        json_store::read(&paths.launcher_imports()).unwrap_or_default()
    }

    pub fn save(&self, paths: &DataPaths) -> Result<()> {
        json_store::write(&paths.launcher_imports(), self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_round_trip_and_keep_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        assert_eq!(ImportPrefs::load(&paths), ImportPrefs::default());
        std::fs::create_dir_all(paths.launcher_imports().parent().unwrap()).unwrap();
        std::fs::write(paths.launcher_imports(), r#"{"enabled":["lutris"],"future":1}"#).unwrap();
        let mut prefs = ImportPrefs::load(&paths);
        assert!(prefs.enabled.contains(&Launcher::Lutris));
        prefs.enabled.insert(Launcher::Faugus);
        prefs.last.insert(Launcher::Faugus, LastImport { at: "2026-10-19T10:00:00Z".into(), games: 3 });
        prefs.save(&paths).unwrap();
        let again = ImportPrefs::load(&paths);
        assert_eq!(again.enabled.iter().copied().collect::<Vec<_>>(), [Launcher::Lutris, Launcher::Faugus]);
        assert_eq!(again.last[&Launcher::Faugus].games, 3);
        assert_eq!(again.extra.get("future"), Some(&serde_json::json!(1)));
        assert_eq!(Launcher::from_source("faugus"), Some(Launcher::Faugus));
    }
}
