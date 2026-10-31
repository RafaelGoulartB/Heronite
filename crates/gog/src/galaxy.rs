//! Games GOG Galaxy installed on this PC (Windows), shown as installed
//! without downloading them again.
//!
//! Galaxy writes a key per installed game and DLC under
//! `HKLM\SOFTWARE\WOW6432Node\GOG.com\Games\<id>` (`path`, `gameName`,
//! `buildId`, `ver`, `langCode` / `language`, and `dependsOn` for a DLC).
//! [`import`] reads them (only reads: nothing of Galaxy's is ever written,
//! moved or deleted) and records the games of the account's library in
//! `gog_store/installed.json`, marked [`installed::GALAXY_KEY`]: the app
//! then starts them from their folder like its own installs, and leaves
//! updates, repairs, moves and uninstalls to Galaxy. A game Galaxy no
//! longer lists, or whose folder is gone, is taken off the list again. A game
//! installed by Heronite stays as it is.
//!
//! It runs after every library refresh (so after logging in) and from the
//! GOG page's button.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use heronite_data::DataPaths;
use serde_json::json;

use crate::disk::dir_size;
use crate::installed::{self, Installed, GALAXY_KEY};
use crate::{lang, library};

/// Galaxy's key of installed games (the 32-bit view: Galaxy is a 32-bit program).
pub const GAMES_KEY: &str = r"SOFTWARE\WOW6432Node\GOG.com\Games";

/// The values of one game's key that matter here.
const VALUES: [&str; 7] = ["path", "gameName", "buildId", "ver", "langCode", "language", "dependsOn"];

/// One key below [`GAMES_KEY`]: its name (the game's id) and its text values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub values: BTreeMap<String, String>,
}

impl Entry {
    fn get(&self, name: &str) -> &str {
        self.values.get(name).map(|v| v.trim()).unwrap_or("")
    }
}

/// The keys below `key` of the registry (Windows; nothing elsewhere).
#[cfg(windows)]
pub fn read_entries(hive: heronite_platform::registry::Hive, key: &str) -> Vec<Entry> {
    use heronite_platform::registry;
    registry::subkeys(hive, key)
        .into_iter()
        .map(|id| {
            let sub = format!(r"{key}\{id}");
            let values = VALUES.iter().filter_map(|v| Some((v.to_string(), registry::string(hive, &sub, v)?))).collect();
            Entry { id, values }
        })
        .collect()
}

/// Galaxy's installed games on this PC; empty without Galaxy (or off Windows).
pub fn galaxy_entries() -> Vec<Entry> {
    #[cfg(windows)]
    {
        read_entries(heronite_platform::registry::Hive::LocalMachine, GAMES_KEY)
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Whether GOG Galaxy has installed games on this PC (no files read).
pub fn has_games() -> bool {
    #[cfg(windows)]
    {
        !heronite_platform::registry::subkeys(heronite_platform::registry::Hive::LocalMachine, GAMES_KEY).is_empty()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// What [`import`] changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Games newly shown as installed.
    pub added: Vec<String>,
    /// Games Galaxy no longer has, taken off the list.
    pub removed: Vec<String>,
    /// Galaxy games already listed (their record refreshed).
    pub kept: usize,
}

/// Records GOG Galaxy's games of the library as installed (see the module
/// docs). No network; reads the registry and walks new games' folders for
/// their size.
pub fn import(paths: &DataPaths) -> Result<Report> {
    import_from(paths, &galaxy_entries())
}

/// The language code of an entry: `langCode`, else the English name in
/// `language`, else `en-US`.
fn language(entry: &Entry) -> String {
    if !entry.get("langCode").is_empty() {
        return entry.get("langCode").to_string();
    }
    lang::code_of(entry.get("language")).unwrap_or("en-US").to_string()
}

/// [`import`] with the entries given (tests use fixtures).
pub fn import_from(paths: &DataPaths, entries: &[Entry]) -> Result<Report> {
    let mut report = Report::default();
    let current = installed::load(paths);
    let usable = |e: &&Entry| {
        let path = e.get("path");
        !e.id.is_empty() && !path.is_empty() && Path::new(path).join(format!("goggame-{}.info", e.id)).is_file()
    };
    let games: Vec<&Entry> = entries.iter().filter(usable).filter(|e| e.get("dependsOn").is_empty()).collect();

    for game in &games {
        let old = current.iter().find(|i| i.app_name == game.id);
        // Heronite's own install, or a game of another account.
        if old.is_some_and(|o| !o.from_galaxy()) || !library::owns(paths, &game.id) {
            continue;
        }
        let path = game.get("path").trim_end_matches(['\\', '/']).to_string();
        let mut entry = Installed::new(&game.id);
        entry.platform = "windows".into();
        entry.install_size = match old.filter(|o| o.install_path == path && !o.install_size.is_empty()) {
            Some(o) => o.install_size.clone(),
            None => heronite_epic::util::file_size(dir_size(Path::new(&path)) as f64),
        };
        entry.install_path = path;
        entry.version = game.get("ver").to_string();
        entry.build_id = game.get("buildId").to_string();
        entry.language = language(game);
        entry.installed_dlcs =
            entries.iter().filter(usable).filter(|d| d.get("dependsOn") == game.id && d.id != game.id).map(|d| d.id.clone()).collect();
        entry.extra.insert(GALAXY_KEY.into(), json!(true));
        if old == Some(&entry) {
            report.kept += 1;
            continue;
        }
        installed::upsert(paths, &entry)?;
        library::sync_install_state(paths, &game.id, Path::new(&entry.install_path).file_name().and_then(|n| n.to_str()));
        // Galaxy ran the game's installers when it installed it.
        if let Err(e) = heronite_launch::native_setup::record(paths, "gog", &game.id, &entry.install_path) {
            log::warn!("recording the setup of GOG game {}: {e:#}", game.id);
        }
        match old {
            Some(_) => report.kept += 1,
            None => report.added.push(game.id.clone()),
        }
    }

    for gone in current.iter().filter(|i| i.from_galaxy() && !games.iter().any(|g| g.id == i.app_name)) {
        installed::remove(paths, &gone.app_name)?;
        crate::updates::forget(paths, &gone.app_name);
        heronite_launch::native_setup::forget(paths, "gog", &gone.app_name);
        library::sync_install_state(paths, &gone.app_name, None);
        report.removed.push(gone.app_name.clone());
    }
    if !report.added.is_empty() || !report.removed.is_empty() {
        log::info!("GOG Galaxy: {} games added, {} taken off the list", report.added.len(), report.removed.len());
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::testing::setup;
    use heronite_epic::cache_store::CacheStore;

    fn entry(id: &str, values: &[(&str, &str)]) -> Entry {
        Entry { id: id.into(), values: values.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    /// A Galaxy folder with its info file.
    fn folder(dir: &Path, name: &str, id: &str) -> String {
        let game = dir.join("GOG Galaxy").join("Games").join(name);
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join(format!("goggame-{id}.info")), "{}").unwrap();
        std::fs::write(game.join("game.exe"), [0u8; 2048]).unwrap();
        game.display().to_string()
    }

    fn library(paths: &DataPaths, ids: &[&str]) {
        let games: Vec<_> = ids.iter().map(|id| json!({"app_name": id, "title": id, "is_installed": false, "install": {}})).collect();
        CacheStore::new(paths, "gog_library", None).set("games", json!(games)).unwrap();
    }

    #[test]
    fn galaxy_games_of_the_library_show_as_installed() {
        let (dir, paths) = setup();
        library(&paths, &["1", "2", "3"]);
        let one = folder(dir.path(), "One", "1");
        let dlc_dir = one.clone();
        std::fs::write(Path::new(&dlc_dir).join("goggame-11.info"), "{}").unwrap();
        let two = folder(dir.path(), "Two", "2");
        let foreign = folder(dir.path(), "Foreign", "9");
        let entries = [
            entry("1", &[("path", &format!("{one}\\")), ("gameName", "One"), ("buildId", "555"), ("ver", "1.2"), ("langCode", "de-DE")]),
            entry("11", &[("path", &one), ("dependsOn", "1")]),
            entry("2", &[("path", &two), ("language", "French")]),
            // Not in the library, its folder gone, no path at all.
            entry("9", &[("path", &foreign)]),
            entry("3", &[("path", &dir.path().join("gone").display().to_string())]),
            entry("4", &[]),
        ];
        let report = import_from(&paths, &entries).unwrap();
        assert_eq!(report.added, ["1", "2"]);

        let got = installed::get(&paths, "1").unwrap();
        assert!(got.from_galaxy());
        assert_eq!(got.install_path, one);
        assert_eq!((got.platform.as_str(), got.version.as_str(), got.build_id.as_str()), ("windows", "1.2", "555"));
        assert_eq!((got.language.as_str(), got.installed_dlcs.clone()), ("de-DE", vec!["11".to_string()]));
        assert_eq!(got.install_size, "2 KiB");
        assert_eq!(installed::get(&paths, "2").unwrap().language, "fr-FR");
        // Galaxy already ran their installers.
        assert!(heronite_launch::native_setup::is_recorded(&paths, "gog", "1", &one));
        assert!(installed::get(&paths, "9").is_none() && installed::get(&paths, "3").is_none());
        let game = library::game_info(&paths, "1").unwrap();
        assert_eq!((game["is_installed"].as_bool(), game["folder_name"].as_str()), (Some(true), Some("One")));

        // Again: nothing new. Galaxy uninstalled Two: it goes off the list, its folder stays.
        let report = import_from(&paths, &entries).unwrap();
        assert_eq!((report.added.len(), report.removed.len(), report.kept), (0, 0, 2));
        let report = import_from(&paths, &entries[..2]).unwrap();
        assert_eq!(report.removed, ["2"]);
        assert!(installed::get(&paths, "2").is_none());
        assert!(Path::new(&two).join("game.exe").exists());
        assert_eq!(library::game_info(&paths, "2").unwrap()["is_installed"], false);
    }

    #[test]
    fn heronites_own_installs_are_left_alone() {
        let (dir, paths) = setup();
        library(&paths, &["1"]);
        let mut own = Installed::new("1");
        own.install_path = dir.path().join("mine").display().to_string();
        installed::upsert(&paths, &own).unwrap();
        let galaxy = folder(dir.path(), "One", "1");
        let report = import_from(&paths, &[entry("1", &[("path", &galaxy)])]).unwrap();
        assert!(report.added.is_empty() && report.removed.is_empty());
        let kept = installed::get(&paths, "1").unwrap();
        assert!(!kept.from_galaxy());
        assert_eq!(kept.install_path, own.install_path);
        // And nothing is removed when Galaxy has none.
        import_from(&paths, &[]).unwrap();
        assert!(installed::get(&paths, "1").is_some());
    }

    #[test]
    fn galaxys_files_are_never_moved_updated_repaired_or_deleted() {
        let (dir, paths) = setup();
        library(&paths, &["1"]);
        let one = folder(dir.path(), "One", "1");
        import_from(&paths, &[entry("1", &[("path", &one)])]).unwrap();
        let cancel = heronite_data::proc::Cancel::new();
        let err = crate::ops::move_install(&paths, "1", &dir.path().join("elsewhere").display().to_string(), &cancel, &mut |_| {});
        assert!(err.unwrap_err().to_string().contains("GOG Galaxy"));
        let g = crate::cmd::Gogdl::with_bin("gogdl", &paths);
        assert!(crate::ops::repair(&paths, &g, "1", &cancel, &mut |_| {}).unwrap_err().to_string().contains("GOG Galaxy"));
        let note = crate::ops::uninstall(&paths, "1").unwrap().expect("a note");
        assert!(note.contains("GOG Galaxy"), "{note}");
        assert!(Path::new(&one).join("game.exe").exists());
        assert!(installed::get(&paths, "1").is_none());
    }

    /// A test key under `HKCU\Software\Heronite\Tests`, never Galaxy's own.
    #[cfg(windows)]
    #[test]
    fn the_registry_layout_galaxy_writes_is_read() {
        use heronite_platform::registry::{self, Hive};
        let key = format!(r"Software\Heronite\Tests\GogGalaxy-{}", std::process::id());
        let game = format!(r"{key}\1207658924");
        registry::set_string(Hive::CurrentUser, &game, "path", r"C:\GOG Games\Witcher").unwrap();
        registry::set_string(Hive::CurrentUser, &game, "gameName", "The Witcher").unwrap();
        registry::set_string(Hive::CurrentUser, &game, "langCode", "en-US").unwrap();
        registry::set_string(Hive::CurrentUser, &format!(r"{key}\1207658925"), "dependsOn", "1207658924").unwrap();
        let entries = read_entries(Hive::CurrentUser, &key);
        delete_tree(&key);
        assert_eq!(entries.len(), 2, "{entries:?}");
        let witcher = entries.iter().find(|e| e.id == "1207658924").unwrap();
        assert_eq!(witcher.get("path"), r"C:\GOG Games\Witcher");
        assert_eq!((witcher.get("gameName"), witcher.get("langCode"), witcher.get("buildId")), ("The Witcher", "en-US", ""));
        assert_eq!(entries.iter().find(|e| e.id == "1207658925").unwrap().get("dependsOn"), "1207658924");
        assert!(read_entries(Hive::CurrentUser, &key).is_empty());
    }

    #[cfg(windows)]
    fn delete_tree(key: &str) {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = std::ffi::OsStr::new(key).encode_wide().chain([0]).collect();
        // SAFETY: `wide` is NUL-terminated; the key is the test's own.
        let r = unsafe {
            windows_sys::Win32::System::Registry::RegDeleteTreeW(windows_sys::Win32::System::Registry::HKEY_CURRENT_USER, wide.as_ptr())
        };
        assert_eq!(r, 0, "deleting {key}");
    }
}
