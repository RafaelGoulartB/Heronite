//! Games of the other Linux launchers (Heroic, Lutris, Bottles, Faugus) imported into
//! the Collection (`heronite_import`), and which launchers the user turned
//! the import on for (`local_library/launcher-imports.json`).
//!
//! Lutris, Bottles and Faugus keep starting their games: the entries hold the command
//! that asks them to ([`LAUNCH_KIND`]). Heroic's games added by hand are
//! copied as this app's own and start here.
//!
//! The folders and Wine prefixes of those games belong to their launcher
//! ([`managed_paths`]): uninstalling a game added here never deletes them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::json_store;
use crate::models::{Extra, LocalMetaFile};
use crate::paths::DataPaths;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Launcher {
    Heroic,
    Lutris,
    Bottles,
    Faugus,
}

impl Launcher {
    pub const ALL: [Launcher; 4] = [Launcher::Heroic, Launcher::Lutris, Launcher::Bottles, Launcher::Faugus];

    /// Also the `source` of the Collection entries it brought in.
    pub fn id(self) -> &'static str {
        match self {
            Launcher::Heroic => "heroic",
            Launcher::Lutris => "lutris",
            Launcher::Bottles => "bottles",
            Launcher::Faugus => "faugus",
        }
    }

    /// The brand (not translated).
    pub fn name(self) -> &'static str {
        match self {
            Launcher::Heroic => "Heroic",
            Launcher::Lutris => "Lutris",
            Launcher::Bottles => "Bottles",
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
    /// Pairs of a game added here and one of a launcher the user keeps as
    /// two games ([`kept_apart_key`]), never offered to merge again.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub kept_apart: BTreeSet<String>,
    /// Games of each launcher the user took out of the Collection (the app
    /// names the launcher's library has for them): its import leaves them
    /// out until it is turned off and on again.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub removed: BTreeMap<Launcher, BTreeSet<String>>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// How [`ImportPrefs::kept_apart`] names a pair: `<mine>|<theirs>` (app names).
pub fn kept_apart_key(mine: &str, theirs: &str) -> String {
    format!("{mine}|{theirs}")
}

/// A folder or Wine prefix of a game another launcher starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Managed {
    pub path: PathBuf,
    pub launcher: Launcher,
    /// The Collection entry's app name (a `sideload` game).
    pub app_name: String,
    pub title: String,
}

/// The folders and Wine prefixes of the games Lutris, Bottles and Faugus start (the
/// Collection's entries of [`LAUNCH_KIND`]), installed or not: they are that
/// launcher's to delete. Unreadable files give none.
pub fn managed_paths(paths: &DataPaths) -> Vec<Managed> {
    let metas: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap_or_default();
    let theirs: BTreeMap<&str, (Launcher, &str, Option<&str>)> = metas
        .games
        .values()
        .filter(|m| m.launch_kind == LAUNCH_KIND)
        .filter_map(|m| {
            let launcher = Launcher::from_source(&m.source)?;
            Some((m.app_name.as_str(), (launcher, m.title.as_str(), m.extra.get(PREFIX_KEY).and_then(Value::as_str))))
        })
        .collect();
    if theirs.is_empty() {
        return Vec::new();
    }
    let library = json_store::read_value(&paths.sideload_library()).unwrap_or(Value::Null);
    let folders: BTreeMap<&str, &str> = library
        .get("games")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|g| Some((g.get("app_name")?.as_str()?, g.get("install")?.get("install_path")?.as_str()?)))
        .collect();
    let mut out = Vec::new();
    for (app, (launcher, title, prefix)) in theirs {
        for path in [folders.get(app).copied(), prefix].into_iter().flatten().map(str::trim) {
            let path = PathBuf::from(path.trim_end_matches('/'));
            if path.is_absolute() && path.components().count() > 1 {
                out.push(Managed { path, launcher, app_name: app.to_string(), title: title.to_string() });
            }
        }
    }
    out
}

/// The game of another launcher whose folder or prefix `path` is, is inside
/// or holds.
pub fn managed_by<'a>(managed: &'a [Managed], path: &std::path::Path) -> Option<&'a Managed> {
    let overlap = |a: &std::path::Path, b: &std::path::Path| a.starts_with(b) || b.starts_with(a);
    let real = path.canonicalize().ok();
    managed
        .iter()
        .find(|m| overlap(path, &m.path) || real.as_deref().is_some_and(|r| m.path.canonicalize().is_ok_and(|mp| overlap(r, &mp))))
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
