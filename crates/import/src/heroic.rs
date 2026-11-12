//! Heroic Games Launcher keeps its files where this app keeps its own, in
//! the same formats: the games added by hand there
//! (`sideload_apps/library.json`) are copied and start here, and its records
//! of the Epic, GOG and Amazon games it installed are added to this app's,
//! so those games show as installed (in the same folders) once the store is
//! logged in here. Each of those games also gets its game settings
//! (`GamesConfig/<app>.json`: its Wine prefix and Wine above all) while it
//! has none here, so it starts in the prefix it had in Heroic. Nothing in
//! Heroic's folder is written.
//!
//! Nothing comes in on its own: only once the user turns Heroic's import on
//! in Settings → Other launchers (never from the Collection's welcome or
//! the first-start dialog), then kept in sync while it is on.

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
        let key = self.path_key();
        let folder_exists = |e: &Value| e.get(key).and_then(Value::as_str).is_some_and(|p| !p.is_empty() && Path::new(p).is_dir());
        self.records(file).into_iter().filter(|(_, e)| folder_exists(e)).collect()
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

/// Heroic's folders on this system, with whether each is the Flatpak's: on
/// Linux the native one and the Flatpak's, on Windows `%APPDATA%\heroic`.
fn candidates(home: &Path) -> Vec<(PathBuf, bool)> {
    if cfg!(windows) {
        vec![(roaming(home).join("heroic"), false)]
    } else {
        vec![
            (crate::xdg(home, "XDG_CONFIG_HOME", ".config").join("heroic"), false),
            (home.join(".var/app").join(FLATPAK_ID).join("config/heroic"), true),
        ]
    }
}

/// Windows' roaming folder of `home`: `%APPDATA%` when `home` is this
/// user's own, else `home\AppData\Roaming`.
fn roaming(home: &Path) -> PathBuf {
    let own = std::env::var_os("USERPROFILE").is_some_and(|h| Path::new(&h) == home);
    std::env::var_os("APPDATA").filter(|_| own).map(PathBuf::from).unwrap_or_else(|| home.join("AppData").join("Roaming"))
}

/// Heroic's folder, unless it is this app's own. On Linux: the native one,
/// else the Flatpak's; with both around, the one with more games (a native
/// folder left behind by a switch to the Flatpak has none).
pub fn detect(home: &Path, data: &DataPaths) -> Result<Option<Found>> {
    let candidates = candidates(home);
    let same = |a: &Path, b: &Path| std::fs::canonicalize(a).ok().is_some_and(|a| std::fs::canonicalize(b).is_ok_and(|b| a == b));
    let mut found = candidates
        .into_iter()
        .filter(|(r, _)| (r.join("config.json").is_file() || r.join("store").is_dir()) && !same(r, &data.root))
        .map(|(root, flatpak)| (read_library(root), flatpak));
    let Some(mut best) = found.next() else {
        return Ok(None);
    };
    for other in found {
        if other.0.count() > best.0.count() {
            best = other;
        }
    }
    let (heroic, flatpak) = best;
    Ok(Some(Found {
        launcher: Launcher::Heroic,
        location: heroic.root.clone(),
        flatpak,
        games: Vec::new(),
        heroic: Some(heroic),
        in_use: false,
    }))
}

/// The games added by hand and the install records in Heroic's `root`.
fn read_library(root: PathBuf) -> Library {
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
    Library { root, sideload, installs }
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
        report.added += new.len();
    }
    for installs in &lib.installs {
        report.added += merge_installs(paths, installs)?;
    }
    let settings = copy_settings(paths, lib, removed)?;
    if settings > 0 {
        log::info!("Brought the game settings (Wine prefix) of {settings} game(s) from Heroic");
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

/// Copies Heroic's `GamesConfig/<app>.json` of each of its games that has
/// no game settings here: the games added by hand that came in, and the
/// store games whose install record here is Heroic's (same folder; one
/// installed again here keeps its own settings). Settings the game already
/// has here (picked by the user, or written by an install here) stay. How
/// many were copied.
fn copy_settings(paths: &DataPaths, lib: &Library, removed: &BTreeSet<String>) -> Result<usize> {
    let mut apps: Vec<&str> = lib.sideload.iter().filter_map(|g| g.get("app_name").and_then(Value::as_str)).collect();
    for installs in &lib.installs {
        let key = installs.kind.path_key();
        let here = installs.kind.records(&installs.kind.target(paths));
        let folder = |e: &Value| e.get(key).and_then(Value::as_str).map(str::to_owned);
        apps.extend(
            installs
                .entries
                .iter()
                .filter(|(id, theirs)| here.iter().any(|(i, mine)| i == id && folder(mine).is_some() && folder(mine) == folder(theirs)))
                .map(|(id, _)| id.as_str()),
        );
    }
    let mut copied = 0;
    for app in apps.into_iter().filter(|a| !removed.contains(*a)) {
        let (from, to) = (lib.root.join("GamesConfig").join(format!("{app}.json")), paths.game_config(app));
        if !from.is_file() || to.exists() {
            continue;
        }
        std::fs::create_dir_all(paths.games_config_dir())?;
        // `create_new`: settings this app writes meanwhile are never replaced.
        let mut out = match std::fs::OpenOptions::new().write(true).create_new(true).open(&to) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        };
        if let Err(e) = std::io::copy(&mut std::fs::File::open(&from)?, &mut out) {
            drop(out);
            let _ = std::fs::remove_file(&to);
            return Err(e.into());
        }
        copied += 1;
    }
    Ok(copied)
}

impl Store {
    /// The key of a record's install folder.
    fn path_key(self) -> &'static str {
        if self == Store::Amazon {
            "path"
        } else {
            "install_path"
        }
    }

    /// `(id, entry)` of each record in `file` (folders gone or not).
    fn records(self, file: &Path) -> Vec<(String, Value)> {
        let Ok(doc) = json_store::read_value(file) else { return Vec::new() };
        match self {
            Store::Epic => doc.as_object().map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default(),
            Store::Gog => list_by(doc.get("installed"), "appName"),
            Store::Amazon => list_by(Some(&doc), "id"),
        }
    }

    /// The ids this app's file has (folders gone or not).
    fn read_ids(self, file: &Path) -> HashSet<String> {
        self.records(file).into_iter().map(|(id, _)| id).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(p: &Path, s: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    }

    // Linux's folders and paths; Windows' are in the test below.
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

    /// The store games' Wine prefix comes with them: Heroic's game settings
    /// are copied for each of its installs that has none here, the ones an
    /// earlier import brought too; settings the game has here stay, and a
    /// game installed again here (another folder) keeps its own.
    #[cfg(unix)]
    #[test]
    fn store_games_get_their_heroic_settings_unless_they_have_some_here() {
        let h = tempfile::tempdir().unwrap();
        let home = h.path().join("home");
        let heroic = home.join(".config/heroic");
        let games = h.path().join("games");
        let at = |g: &str| {
            std::fs::create_dir_all(games.join(g)).unwrap();
            games.join(g).to_string_lossy().into_owned()
        };
        let (control, alan, witcher, fallout, hades) = (at("Control"), at("Alan"), at("Witcher"), at("Fallout"), at("Hades"));
        w(&heroic.join("config.json"), "{}");
        let config = |app: &str| json!({ app: {"winePrefix": format!("/pfx/{app}"), "wineVersion": {"name": "GE"}}, "version": "v0", "explicit": true });
        for app in ["ctl", "alan", "hades", "w3", "amzn1.fo3"] {
            w(&heroic.join("GamesConfig").join(format!("{app}.json")), &config(app).to_string());
        }
        let epic = json!({
            "ctl": {"app_name": "ctl", "install_path": control},
            "alan": {"app_name": "alan", "install_path": alan},
            "hades": {"app_name": "hades", "install_path": hades}});
        w(&heroic.join("legendaryConfig/legendary/installed.json"), &epic.to_string());
        w(&heroic.join("gog_store/installed.json"), &json!({"installed": [{"appName": "w3", "install_path": witcher}]}).to_string());
        w(&heroic.join("nile_config/nile/installed.json"), &json!([{"id": "amzn1.fo3", "path": fallout}]).to_string());

        let paths = DataPaths::at(h.path().join("data"));
        // Brought by an earlier import, without settings: "ctl". Installed
        // again here, in another folder: "hades". "alan" has settings here.
        let here = json!({
            "ctl": {"app_name": "ctl", "install_path": control},
            "hades": {"app_name": "hades", "install_path": "/elsewhere/Hades"}});
        w(&paths.legendary_config_dir().join("installed.json"), &here.to_string());
        let mine = json!({"alan": {"winePrefix": "/mine"}, "version": "v0", "explicit": true}).to_string();
        w(&paths.game_config("alan"), &mine);

        let lib = detect(&home, &paths).unwrap().unwrap().heroic.unwrap();
        sync(&paths, &lib, &BTreeSet::new()).unwrap();

        let prefix = |app: &str| {
            let doc = json_store::read_value(&paths.game_config(app)).ok()?;
            doc.get(app)?.get("winePrefix")?.as_str().map(str::to_owned)
        };
        assert_eq!(prefix("ctl").as_deref(), Some("/pfx/ctl"), "imported before, settings now");
        assert_eq!(prefix("w3").as_deref(), Some("/pfx/w3"));
        assert_eq!(prefix("amzn1.fo3").as_deref(), Some("/pfx/amzn1.fo3"));
        assert_eq!(std::fs::read_to_string(paths.game_config("alan")).unwrap(), mine, "settings here stay");
        assert!(!paths.game_config("hades").exists(), "installed here: not Heroic's prefix");
        let settings = heronite_data::game_config::load_game(&paths, "ctl", home.to_str().unwrap());
        assert_eq!(settings["winePrefix"], "/pfx/ctl");
        assert_eq!(settings["wineVersion"]["name"], "GE");

        // Again: what is here stays as it is.
        let before = std::fs::read_to_string(paths.game_config("ctl")).unwrap();
        assert_eq!(copy_settings(&paths, &lib, &BTreeSet::new()).unwrap(), 0);
        assert_eq!(std::fs::read_to_string(paths.game_config("ctl")).unwrap(), before);
    }

    /// The games added by hand get their settings once in the Collection,
    /// the ones an earlier import brought too, but not the ones the user
    /// took out (the only games `removed` holds for Heroic).
    #[cfg(unix)]
    #[test]
    fn hand_added_games_get_their_heroic_settings_unless_taken_out() {
        let h = tempfile::tempdir().unwrap();
        let home = h.path().join("home");
        let heroic = home.join(".config/heroic");
        w(&heroic.join("config.json"), "{}");
        let game = |app: &str| json!({"runner": "sideload", "app_name": app, "title": app, "install": {"executable": "/g/x.exe"}});
        w(&heroic.join("sideload_apps/library.json"), &json!({"games": [game("kept"), game("gone")]}).to_string());
        for app in ["kept", "gone"] {
            let config = json!({ app: {"winePrefix": format!("/pfx/{app}")}, "version": "v0" });
            w(&heroic.join("GamesConfig").join(format!("{app}.json")), &config.to_string());
        }
        let paths = DataPaths::at(h.path().join("data"));
        let lib = detect(&home, &paths).unwrap().unwrap().heroic.unwrap();
        let removed = BTreeSet::from(["gone".to_string()]);
        sync(&paths, &lib, &removed).unwrap();
        assert!(paths.game_config("kept").is_file());
        assert!(!paths.game_config("gone").exists());

        // Brought by an earlier import, which left its settings behind.
        std::fs::remove_file(paths.game_config("kept")).unwrap();
        sync(&paths, &lib, &removed).unwrap();
        assert!(paths.game_config("kept").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn the_flatpak_folder_wins_over_an_empty_native_one() {
        let h = tempfile::tempdir().unwrap();
        let home = h.path().join("home");
        let paths = DataPaths::at(h.path().join("data"));
        let native = home.join(".config/heroic");
        w(&native.join("config.json"), "{}");
        let flatpak = home.join(".var/app").join(FLATPAK_ID).join("config/heroic");
        w(&flatpak.join("config.json"), "{}");
        let game = h.path().join("games/Control");
        std::fs::create_dir_all(&game).unwrap();
        w(
            &flatpak.join("legendaryConfig/legendary/installed.json"),
            &format!(r#"{{"ctl":{{"app_name":"ctl","install_path":"{}"}}}}"#, game.display()),
        );
        let found = detect(&home, &paths).unwrap().unwrap();
        assert!(found.flatpak);
        assert_eq!(found.location, flatpak);
        assert_eq!(found.count(), 1);
        // Nothing in either: the native one.
        std::fs::remove_file(flatpak.join("legendaryConfig/legendary/installed.json")).unwrap();
        let found = detect(&home, &paths).unwrap().unwrap();
        assert!(!found.flatpak);
        assert_eq!(found.location, native);
    }

    #[test]
    fn this_apps_own_folder_is_not_heroic() {
        let h = tempfile::tempdir().unwrap();
        let heroic = candidates(h.path()).remove(0).0;
        w(&heroic.join("config.json"), "{}");
        assert!(detect(h.path(), &DataPaths::at(h.path().join("data"))).unwrap().is_some());
        assert!(detect(h.path(), &DataPaths::at(&heroic)).unwrap().is_none());
    }

    #[test]
    fn another_users_roaming_folder_is_under_their_home() {
        let home = Path::new("/elsewhere/u");
        assert_eq!(roaming(home), home.join("AppData").join("Roaming"));
    }

    /// Heroic for Windows: `%APPDATA%\heroic`, its records holding Windows
    /// paths (JSON doubles their backslashes).
    #[cfg(windows)]
    #[test]
    fn windows_heroic_brings_hand_added_games_and_epic_gog_amazon_installs() {
        let h = tempfile::tempdir().unwrap();
        let home = h.path().join("home");
        let heroic = home.join(r"AppData\Roaming\heroic");
        let games = h.path().join("Games");
        for g in ["Control", "Witcher 3", "Fallout 3"] {
            std::fs::create_dir_all(games.join(g)).unwrap();
        }
        let at = |g: &str| games.join(g).to_string_lossy().into_owned();
        w(&heroic.join("config.json"), "{}");
        let doom = json!({"runner": "sideload", "app_name": "doom1", "title": "Doom", "is_installed": true,
            "install": {"executable": r"C:\Games\Doom\doom.exe", "install_path": r"C:\Games\Doom", "platform": "Windows"}});
        w(&heroic.join(r"sideload_apps\library.json"), &json!({ "games": [doom] }).to_string());
        w(&heroic.join(r"GamesConfig\doom1.json"), &json!({"doom1": {"targetExe": r"C:\Games\Doom\doom.exe"}}).to_string());
        w(&heroic.join(r"store\timestamp.json"), r#"{"doom1":{"lastPlayed":"2026-01-01T00:00:00.000Z","totalPlayed":90}}"#);
        let epic = json!({
            "ctl": {"app_name": "ctl", "install_path": at("Control"), "platform": "Windows"},
            "gone": {"app_name": "gone", "install_path": r"Z:\Nowhere\Gone", "platform": "Windows"}});
        w(&heroic.join(r"legendaryConfig\legendary\installed.json"), &epic.to_string());
        let gog = json!({"installed": [{"appName": "w3", "install_path": at("Witcher 3"), "platform": "windows"}]});
        w(&heroic.join(r"gog_store\installed.json"), &gog.to_string());
        let amazon = json!([{"id": "amzn1.fo3", "path": at("Fallout 3"), "version": "1"}, {"id": "amzn1.gone", "path": r"Z:\Nowhere"}]);
        w(&heroic.join(r"nile_config\nile\installed.json"), &amazon.to_string());

        let paths = DataPaths::at(h.path().join(r"heronite\data"));
        let found = detect(&home, &paths).unwrap().unwrap();
        assert_eq!((found.location.as_path(), found.flatpak), (heroic.as_path(), false));
        assert_eq!(found.count(), 4, "the hand-added game and the three installs whose folders exist");
        let lib = found.heroic.as_ref().unwrap();
        assert_eq!(sync(&paths, lib, &BTreeSet::new()).unwrap(), Report { added: 4, updated: 0 });

        let collection = heronite_data::Library::load(&paths).unwrap();
        let game = collection.get("sideload_doom1").unwrap();
        assert_eq!(game.source_label, "Heroic");
        assert_eq!(game.playtime_minutes, 90);
        let meta: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap();
        assert_eq!(meta.games["doom1"].remapped_executable.as_deref(), Some(r"C:\Games\Doom\doom.exe"));
        assert!(paths.game_config("doom1").is_file());
        let epic = json_store::read_value(&paths.legendary_config_dir().join("installed.json")).unwrap();
        assert_eq!(epic["ctl"]["install_path"], at("Control"));
        assert!(epic.get("gone").is_none());
        assert_eq!(json_store::read_value(&paths.gog_installed()).unwrap()["installed"][0]["install_path"], at("Witcher 3"));
        let amazon = json_store::read_value(&paths.nile_config_dir().join(r"nile\installed.json")).unwrap();
        assert_eq!(amazon.as_array().map(Vec::len), Some(1));
        assert_eq!(amazon[0]["path"], at("Fallout 3"));

        assert_eq!(sync(&paths, lib, &BTreeSet::new()).unwrap(), Report::default(), "nothing new the second time");
    }
}
