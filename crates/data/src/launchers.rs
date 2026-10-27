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
//! The same goes for the prefix and the games of a store client
//! ([`crate::clients`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::clients::{self, Client};
use crate::json_store;
use crate::models::{Extra, LocalGameMeta, LocalMetaFile};
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

/// A folder or Wine prefix that another launcher or a store client keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Managed {
    pub path: PathBuf,
    /// The brand that keeps it ([`Launcher::name`], [`Client::name`]).
    pub owner: &'static str,
    /// The Collection entry's app name (a `sideload` game); empty for a
    /// client's Wine prefix, which belongs to no single game.
    pub app_name: String,
    /// What is kept there: the game's title, or the client's name for its
    /// prefix.
    pub title: String,
}

/// The folders and Wine prefixes that are not this app's to delete: those
/// of the games Lutris, Bottles and Faugus start (the Collection's entries of
/// [`LAUNCH_KIND`]), and each store client's Wine prefix and the folders of
/// its games ([`clients::LAUNCH_KIND`]), installed or not. Unreadable files
/// give none. `home` expands the `~` of a client's prefix setting.
pub fn managed_paths(paths: &DataPaths, home: &str) -> Vec<Managed> {
    let metas: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap_or_default();
    // App name -> (who keeps it, the entry).
    let theirs: BTreeMap<&str, (&'static str, &LocalGameMeta)> = metas
        .games
        .values()
        .filter_map(|m| {
            let owner = match m.launch_kind.as_str() {
                LAUNCH_KIND => Launcher::from_source(&m.source)?.name(),
                clients::LAUNCH_KIND => Client::from_id(&m.source)?.name(),
                _ => return None,
            };
            Some((m.app_name.as_str(), (owner, m)))
        })
        .collect();
    let mut out = Vec::new();
    let mut keep = |path: &str, owner: &'static str, app_name: &str, title: &str| {
        let path = PathBuf::from(path.trim().trim_end_matches('/'));
        if path.is_absolute() && path.components().count() > 1 {
            out.push(Managed { path, owner, app_name: app_name.to_string(), title: title.to_string() });
        }
    };
    // A client's prefix comes first: a path overlapping it is matched to it
    // before the games inside it.
    for client in Client::ALL {
        if let Some(prefix) = clients::prefix_of(paths, client, home) {
            keep(&prefix.to_string_lossy(), client.name(), "", client.name());
        }
    }
    if theirs.is_empty() {
        return out;
    }
    let library = json_store::read_value(&paths.sideload_library()).unwrap_or(Value::Null);
    let folders: BTreeMap<&str, &str> = library
        .get("games")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|g| Some((g.get("app_name")?.as_str()?, g.get("install")?.get("install_path")?.as_str()?)))
        .collect();
    for (app, (owner, meta)) in theirs {
        let prefix = meta.extra.get(PREFIX_KEY).and_then(Value::as_str);
        for path in [folders.get(app).copied(), prefix].into_iter().flatten() {
            keep(path, owner, app, &meta.title);
        }
    }
    out
}

/// Whether `path` (`real`: its canonical form) is, is inside or holds `m`.
fn touches(m: &Managed, path: &Path, real: Option<&Path>) -> bool {
    let overlap = |a: &Path, b: &Path| a.starts_with(b) || b.starts_with(a);
    overlap(path, &m.path) || real.is_some_and(|r| m.path.canonicalize().is_ok_and(|mp| overlap(r, &mp)))
}

/// The first of the [`managed_paths`] that `path` is, is inside or holds.
pub fn managed_by<'a>(managed: &'a [Managed], path: &Path) -> Option<&'a Managed> {
    let real = path.canonicalize().ok();
    managed.iter().find(|m| touches(m, path, real.as_deref()))
}

/// All of them: a client's prefix holds many games.
pub fn managed_in<'a>(managed: &'a [Managed], path: &Path) -> Vec<&'a Managed> {
    let real = path.canonicalize().ok();
    managed.iter().filter(|m| touches(m, path, real.as_deref())).collect()
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

    fn write_collection(paths: &DataPaths, library: Value, meta: Value) {
        json_store::write(&paths.sideload_library(), &library).unwrap();
        json_store::write(&paths.local_meta(), &meta).unwrap();
    }

    #[test]
    fn the_launchers_games_are_kept_by_the_launcher() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        write_collection(
            &paths,
            serde_json::json!({"games": [{"app_name": "lutris_7", "install": {"install_path": "/home/u/Games/heroes/"}}]}),
            serde_json::json!({"games": {
                "lutris_7": {"appName": "lutris_7", "runner": "sideload", "source": "lutris", "launchKind": "launcher", "title": "Heroes III",
                             "heroniteLauncherPrefix": "/home/u/Prefixes/heroes"},
                "by_hand": {"appName": "by_hand", "runner": "sideload", "source": "lutris", "launchKind": "executable", "title": "By hand"}}}),
        );
        let managed = managed_paths(&paths, "/home/u");
        let kept: Vec<_> = managed.iter().map(|m| (m.path.to_str().unwrap(), m.owner, m.app_name.as_str(), m.title.as_str())).collect();
        assert_eq!(
            kept,
            [("/home/u/Games/heroes", "Lutris", "lutris_7", "Heroes III"), ("/home/u/Prefixes/heroes", "Lutris", "lutris_7", "Heroes III")]
        );
        assert_eq!(managed_by(&managed, Path::new("/home/u/Games/heroes/drive_c")).map(|m| m.owner), Some("Lutris"));
        assert_eq!(managed_by(&managed, Path::new("/home/u/Games/other")), None);
    }

    /// A client's prefix is kept without any game, its games' folders too.
    #[test]
    fn a_clients_prefix_and_games_are_kept_by_the_client() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        assert!(managed_paths(&paths, "/home/u").is_empty());

        let setting = |client: Client, prefix: &str| {
            crate::game_config::set_game_setting(&paths, client.config_app(), "/home/u", "winePrefix", prefix.into()).unwrap();
        };
        setting(Client::BattleNet, "~/Prefixes/Battle.net");
        setting(Client::Ea, "");
        assert_eq!(
            managed_paths(&paths, "/home/u"),
            [Managed {
                path: "/home/u/Prefixes/Battle.net".into(),
                owner: "Battle.net",
                app_name: String::new(),
                title: "Battle.net".into()
            }]
        );

        write_collection(
            &paths,
            serde_json::json!({"games": [
                {"app_name": "battlenet_wow", "install": {"install_path": "/home/u/Prefixes/Battle.net/drive_c/WoW"}},
                {"app_name": "ubisoft_635", "install": {"install_path": "/games/ubi/Anno"}},
                {"app_name": "lutris_7", "install": {"install_path": "/home/u/Games/heroes"}}]}),
            serde_json::json!({"games": {
                "battlenet_wow": {"appName": "battlenet_wow", "runner": "sideload", "source": "battlenet", "launchKind": "client", "title": "World of Warcraft"},
                "ubisoft_635": {"appName": "ubisoft_635", "runner": "sideload", "source": "ubisoft", "launchKind": "client", "title": "Anno 1800"},
                "lutris_7": {"appName": "lutris_7", "runner": "sideload", "source": "lutris", "launchKind": "launcher", "title": "Heroes III"},
                "odd": {"appName": "odd", "runner": "sideload", "source": "lutris", "launchKind": "client", "title": "Not a client's"}}}),
        );
        let managed = managed_paths(&paths, "/home/u");
        let kept: Vec<_> = managed.iter().map(|m| (m.path.to_str().unwrap(), m.owner, m.title.as_str())).collect();
        assert_eq!(
            kept,
            [
                ("/home/u/Prefixes/Battle.net", "Battle.net", "Battle.net"),
                ("/home/u/Prefixes/Battle.net/drive_c/WoW", "Battle.net", "World of Warcraft"),
                ("/home/u/Games/heroes", "Lutris", "Heroes III"),
                ("/games/ubi/Anno", "Ubisoft Connect", "Anno 1800"),
            ]
        );
        // The prefix is what a path in it is matched to, not the game inside.
        let wow = Path::new("/home/u/Prefixes/Battle.net/drive_c/WoW/Data");
        assert_eq!(managed_by(&managed, wow).map(|m| m.app_name.as_str()), Some(""));
        assert_eq!(managed_in(&managed, wow).len(), 2);
        assert_eq!(managed_in(&managed, Path::new("/home/u/Prefixes")).len(), 2, "a folder holding them");
        assert_eq!(managed_by(&managed, Path::new("/games/ubi")).map(|m| m.owner), Some("Ubisoft Connect"));
        assert_eq!(managed_by(&managed, Path::new("/games/other")), None);
    }
}
