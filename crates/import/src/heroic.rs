//! Heroic Games Launcher keeps its files where this app keeps its own, in
//! the same formats: the games added by hand there
//! (`sideload_apps/library.json`) are copied with their game settings
//! (`GamesConfig/<app>.json`) and start here, and its records of the Epic,
//! GOG and Amazon games it installed are added to this app's, so those
//! games show as installed (in the same folders) once the store is logged
//! in here. Nothing in Heroic's folder is written.
//!
//! A first start with Heroic around copies all of it already
//! (`heronite_data::migrate`); this catches up with what was added since.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use heronite_data::launchers::Launcher;
use heronite_data::models::{LocalGameMeta, LocalMetaFile, TimestampFile};
use heronite_data::{json_store, DataPaths};
use serde_json::{json, Value};

use crate::{Found, Report};

const FLATPAK_ID: &str = "com.heroicgameslauncher.hgl";

/// What Heroic has that this app can take.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Library {
    /// Heroic's config folder.
    pub root: PathBuf,
    /// Its games added by hand (`sideload_apps/library.json` entries).
    pub sideload: Vec<Value>,
    /// Installed store games, by store: `(file in Heroic's folder, file here)`.
    pub installs: Vec<Installs>,
}

/// One store's install records.
#[derive(Debug, Clone, PartialEq)]
pub struct Installs {
    pub kind: Store,
    /// Entries whose folder exists.
    pub entries: Vec<(String, Value)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Store {
    /// legendary's `installed.json`: an object by app name.
    Epic,
    /// `gog_store/installed.json`: `{"installed": [ {appName, ...} ]}`.
    Gog,
    /// nile's `installed.json`: `[ {id, path, ...} ]`.
    Amazon,
}

impl Store {
    fn source(self, heroic: &Path) -> PathBuf {
        match self {
            Store::Epic => heroic.join("legendaryConfig/legendary/installed.json"),
            Store::Gog => heroic.join("gog_store/installed.json"),
            Store::Amazon => heroic.join("nile_config/nile/installed.json"),
        }
    }

    fn target(self, paths: &DataPaths) -> PathBuf {
        match self {
            Store::Epic => paths.legendary_config_dir().join("installed.json"),
            Store::Gog => paths.gog_installed(),
            Store::Amazon => paths.nile_config_dir().join("nile/installed.json"),
        }
    }

    /// `(id, entry)` of each record whose install folder exists.
    fn read(self, file: &Path) -> Vec<(String, Value)> {
        let Ok(doc) = json_store::read_value(file) else { return Vec::new() };
        let folder_exists =
            |e: &Value, key: &str| e.get(key).and_then(Value::as_str).is_some_and(|p| !p.is_empty() && Path::new(p).is_dir());
        let list: Vec<(String, Value)> = match self {
            Store::Epic => doc.as_object().map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default(),
            Store::Gog => list_by(doc.get("installed"), "appName"),
            Store::Amazon => list_by(Some(&doc), "id"),
        };
        let key = if self == Store::Amazon { "path" } else { "install_path" };
        list.into_iter().filter(|(_, e)| folder_exists(e, key)).collect()
    }
}

fn list_by(list: Option<&Value>, id: &str) -> Vec<(String, Value)> {
    list.and_then(Value::as_array)
        .map(|l| l.iter().filter_map(|e| Some((e.get(id)?.as_str()?.to_string(), e.clone()))).collect())
        .unwrap_or_default()
}

impl Library {
    pub fn count(&self) -> usize {
        self.sideload.len() + self.installs.iter().map(|i| i.entries.len()).sum::<usize>()
    }
}

/// Heroic's folder (native, else Flatpak), unless it is this app's own.
pub fn detect(home: &Path, data: &DataPaths) -> Result<Option<Found>> {
    let candidates = [
        (crate::xdg(home, "XDG_CONFIG_HOME", ".config").join("heroic"), false),
        (home.join(".var/app").join(FLATPAK_ID).join("config/heroic"), true),
    ];
    let same = |a: &Path, b: &Path| std::fs::canonicalize(a).ok().is_some_and(|a| std::fs::canonicalize(b).is_ok_and(|b| a == b));
    let Some((root, flatpak)) =
        candidates.into_iter().find(|(r, _)| (r.join("config.json").is_file() || r.join("store").is_dir()) && !same(r, &data.root))
    else {
        return Ok(None);
    };
    let sideload = json_store::read_value(&root.join("sideload_apps/library.json"))
        .ok()
        .and_then(|d| d.get("games").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(|g| g.get("app_name").and_then(Value::as_str).is_some_and(|a| !a.is_empty()))
        .collect();
    let installs = [Store::Epic, Store::Gog, Store::Amazon]
        .into_iter()
        .map(|kind| Installs { kind, entries: kind.read(&kind.source(&root)) })
        .filter(|i| !i.entries.is_empty())
        .collect();
    let heroic = Library { root: root.clone(), sideload, installs };
    Ok(Some(Found { launcher: Launcher::Heroic, location: root, flatpak, games: Vec::new(), heroic: Some(heroic) }))
}

/// Copies what this app does not have yet; play time of the copied games
/// moves up to Heroic's.
pub fn sync(paths: &DataPaths, lib: &Library, removed: &BTreeSet<String>) -> Result<Report> {
    let mut report = Report::default();
    let here: HashSet<String> = json_store::read_value(&paths.sideload_library())
        .ok()
        .and_then(|d| d.get("games").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|g| g.get("app_name").and_then(Value::as_str).map(str::to_owned))
        .collect();
    let new: Vec<&Value> = lib
        .sideload
        .iter()
        .filter(|g| g.get("app_name").and_then(Value::as_str).is_some_and(|a| !here.contains(a) && !removed.contains(a)))
        .collect();
    if !new.is_empty() {
        let played: TimestampFile = json_store::read(&lib.root.join("store/timestamp.json")).unwrap_or_default();
        let statuses = heronite_data::status::load_statuses(paths).unwrap_or_else(|_| heronite_data::status::default_statuses());
        json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |file| {
            for g in &new {
                let app = g["app_name"].as_str().unwrap_or_default().to_string();
                let minutes = played.get(&app).map(|t| t.total_played.get()).unwrap_or(0.0);
                let executable = g.pointer("/install/executable").and_then(Value::as_str).filter(|e| !e.is_empty());
                file.games.entry(app.clone()).or_insert_with(|| LocalGameMeta {
                    app_name: app.clone(),
                    runner: "sideload".into(),
                    playnite_id: format!("heroic_{app}"),
                    source: Launcher::Heroic.id().into(),
                    launch_kind: if executable.is_some() { "executable" } else { "unavailable" }.into(),
                    title: g.get("title").and_then(Value::as_str).unwrap_or(&app).to_string(),
                    remapped_executable: executable.map(str::to_owned),
                    completion_status_id: Some(heronite_data::status::inferred_status_id(minutes, &statuses)),
                    ..Default::default()
                });
            }
            Ok(())
        })?;
        json_store::update_value(&paths.sideload_library(), |doc| {
            if !doc.is_object() {
                *doc = json!({});
            }
            let games = doc.as_object_mut().expect("object").entry("games").or_insert_with(|| json!([]));
            if !games.is_array() {
                *games = json!([]);
            }
            games.as_array_mut().expect("array").extend(new.iter().map(|g| (*g).clone()));
            Ok(())
        })?;
        for g in &new {
            let app = g["app_name"].as_str().unwrap_or_default();
            let (from, to) = (lib.root.join("GamesConfig").join(format!("{app}.json")), paths.game_config(app));
            if from.is_file() && !to.exists() {
                std::fs::create_dir_all(paths.games_config_dir())?;
                std::fs::copy(&from, &to)?;
            }
        }
        report.added += new.len();
    }
    for installs in &lib.installs {
        report.added += merge_installs(paths, installs)?;
    }
    let played: TimestampFile = json_store::read(&lib.root.join("store/timestamp.json")).unwrap_or_default();
    let entries: Vec<_> = lib
        .sideload
        .iter()
        .filter_map(|g| g.get("app_name").and_then(Value::as_str))
        .filter_map(|app| {
            let t = played.get(app)?;
            Some((app.to_string(), t.total_played.get().max(0.0).round() as u64, t.last_played.clone().filter(|l| !l.is_empty())))
        })
        .collect();
    let new: HashSet<&str> = new.iter().filter_map(|g| g.get("app_name").and_then(Value::as_str)).collect();
    report.updated += crate::collection::raise_playtime(paths, &entries)?.iter().filter(|app| !new.contains(app.as_str())).count();
    if report.added > 0 {
        log::info!("Brought {} game(s) and install record(s) from Heroic", report.added);
    }
    Ok(report)
}

/// Adds the records this app does not have. How many were added.
fn merge_installs(paths: &DataPaths, installs: &Installs) -> Result<usize> {
    let target = installs.kind.target(paths);
    let have: HashSet<String> = installs.kind.read_ids(&target);
    let missing: Vec<&(String, Value)> = installs.entries.iter().filter(|(id, _)| !have.contains(id)).collect();
    if missing.is_empty() {
        return Ok(0);
    }
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    json_store::update_value(&target, |doc| {
        match installs.kind {
            Store::Epic => {
                if !doc.is_object() {
                    *doc = json!({});
                }
                let map = doc.as_object_mut().expect("object");
                for (id, e) in &missing {
                    map.insert(id.clone(), e.clone());
                }
            }
            Store::Gog => {
                if !doc.is_object() {
                    *doc = json!({});
                }
                let list = doc.as_object_mut().expect("object").entry("installed").or_insert_with(|| json!([]));
                if !list.is_array() {
                    *list = json!([]);
                }
                list.as_array_mut().expect("array").extend(missing.iter().map(|(_, e)| e.clone()));
            }
            Store::Amazon => {
                if !doc.is_array() {
                    *doc = json!([]);
                }
                doc.as_array_mut().expect("array").extend(missing.iter().map(|(_, e)| e.clone()));
            }
        }
        Ok(())
    })?;
    Ok(missing.len())
}

impl Store {
    /// The ids this app's file has (folders gone or not).
    fn read_ids(self, file: &Path) -> HashSet<String> {
        let Ok(doc) = json_store::read_value(file) else { return HashSet::new() };
        match self {
            Store::Epic => doc.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default(),
            Store::Gog => list_by(doc.get("installed"), "appName").into_iter().map(|(id, _)| id).collect(),
            Store::Amazon => list_by(Some(&doc), "id").into_iter().map(|(id, _)| id).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(p: &Path, s: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    }

    // Linux launchers' data and paths; imports are off on Windows (`Feature::LauncherImports`).
    #[cfg(unix)]
    #[test]
    fn copies_new_hand_added_games_and_install_records_once() {
        let h = tempfile::tempdir().unwrap();
        let home = h.path().join("home");
        let heroic = home.join(".config/heroic");
        let games = h.path().join("games");
        std::fs::create_dir_all(games.join("Control")).unwrap();
        std::fs::create_dir_all(games.join("Witcher")).unwrap();
        w(&heroic.join("config.json"), "{}");
        w(
            &heroic.join("sideload_apps/library.json"),
            r#"{"games":[{"runner":"sideload","app_name":"abc","title":"Doom","is_installed":true,"install":{"executable":"/g/doom.exe","platform":"Windows"}}]}"#,
        );
        w(&heroic.join("GamesConfig/abc.json"), r#"{"abc":{"wineVersion":{"name":"GE"}}}"#);
        w(
            &heroic.join("store/timestamp.json"),
            r#"{"abc":{"firstPlayed":"2025-01-01T00:00:00.000Z","lastPlayed":"2025-02-01T00:00:00.000Z","totalPlayed":42}}"#,
        );
        let control = games.join("Control").to_string_lossy().into_owned();
        w(
            &heroic.join("legendaryConfig/legendary/installed.json"),
            &format!(r#"{{"ctl":{{"app_name":"ctl","install_path":"{control}"}},"gone":{{"app_name":"gone","install_path":"/nowhere"}}}}"#),
        );
        let witcher = games.join("Witcher").to_string_lossy().into_owned();
        w(&heroic.join("gog_store/installed.json"), &format!(r#"{{"installed":[{{"appName":"w3","install_path":"{witcher}"}}]}}"#));

        let paths = DataPaths::at(h.path().join("data"));
        let found = detect(&home, &paths).unwrap().unwrap();
        assert_eq!(found.count(), 3);
        let lib = found.heroic.as_ref().unwrap();
        assert_eq!(sync(&paths, lib, &BTreeSet::new()).unwrap(), Report { added: 3, updated: 0 });

        let collection = heronite_data::Library::load(&paths).unwrap();
        let doom = collection.get("sideload_abc").unwrap();
        assert_eq!(doom.source_label, "Heroic");
        assert_eq!(doom.playtime_minutes, 42);
        assert!(paths.game_config("abc").is_file());
        let epic = json_store::read_value(&paths.legendary_config_dir().join("installed.json")).unwrap();
        assert!(epic.get("ctl").is_some() && epic.get("gone").is_none());
        let gog = json_store::read_value(&paths.gog_installed()).unwrap();
        assert_eq!(gog["installed"][0]["appName"], "w3");

        // Again: nothing new.
        assert_eq!(sync(&paths, lib, &BTreeSet::new()).unwrap(), Report::default());
    }

    #[test]
    fn this_apps_own_folder_is_not_heroic() {
        let h = tempfile::tempdir().unwrap();
        let heroic = h.path().join(".config/heroic");
        w(&heroic.join("config.json"), "{}");
        assert!(detect(h.path(), &DataPaths::at(&heroic)).unwrap().is_none());
    }
}
