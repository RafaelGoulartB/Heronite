//! "Remove Steam games" (Stores → Steam): takes the Steam entries Heronite
//! added out of the Collection, in one pass over each file (a library of
//! thousands of games would take minutes one game at a time), and forgets the
//! cached library. Nothing is uninstalled and Steam's own files are left
//! alone.

use std::collections::HashSet;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::*;
use heronite_data::library::metadata_key;
use heronite_data::models::{ArtFile, LocalMetaFile, MetadataFile, SessionsFile};
use heronite_data::steam_owned;
use heronite_data::{json_store, DataPaths};
use heronite_shortcuts::menu::Roots;
use serde_json::Value;

use super::{save_wanted, SteamLibrary, SteamLibraryEvent};

/// What a removal did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Removal {
    /// Games taken out of the Collection.
    pub removed: usize,
    /// Games left in, because they were running.
    pub running: usize,
}

/// The app names of the Steam entries Heronite added. Blocking.
fn added(paths: &DataPaths) -> Result<Vec<String>, String> {
    let metas: LocalMetaFile = json_store::read(&paths.local_meta()).map_err(|e| format!("{e:#}"))?;
    Ok(steam_owned::added_entries(&metas))
}

/// Removes the desktop and menu shortcuts that start one of the `gone` games,
/// found by what they run (`--launch sideload <app name>`), not by their
/// title: another game with the same title (a GOG copy of the same game)
/// keeps its own. Windows shortcuts (`.lnk`) cannot be read that way: they
/// stay.
fn remove_shortcuts(roots: &Roots, gone: &HashSet<String>) {
    if cfg!(windows) {
        return;
    }
    for dir in roots.desktop.iter().chain([&roots.applications]) {
        let Ok(files) = fs::read_dir(dir) else { continue };
        for file in files.flatten().map(|f| f.path()).filter(|p| p.extension().is_some_and(|e| e == "desktop")) {
            let Ok(text) = fs::read_to_string(&file) else { continue };
            let starts_one = text
                .lines()
                .filter_map(|l| l.strip_prefix("Exec="))
                .any(|exec| exec.rsplit_once(" --launch sideload ").is_some_and(|(_, app)| gone.contains(app.trim())));
            if starts_one {
                match fs::remove_file(&file) {
                    Ok(()) => log::info!("shortcut removed: {}", file.display()),
                    Err(e) => log::warn!("Could not remove shortcut {}: {e}", file.display()),
                }
            }
        }
    }
}

/// The `appName` entries of Heroic's `games.recent|favourites|hidden` lists.
fn drop_from_lists(paths: &DataPaths, gone: &HashSet<String>) -> anyhow::Result<()> {
    json_store::update_value(&paths.store_config(), |doc| {
        for list in ["recent", "favourites", "hidden"] {
            if let Some(items) = doc.get_mut("games").and_then(|g| g.get_mut(list)).and_then(Value::as_array_mut) {
                items.retain(|v| v.get("appName").and_then(Value::as_str).is_none_or(|a| !gone.contains(a)));
            }
        }
        Ok(())
    })
}

/// Takes the `gone` games (app names) out of the Collection: their shortcuts,
/// then their data, each file read and written once. How many left. Blocking.
fn remove_entries(paths: &DataPaths, roots: &Roots, gone: &HashSet<String>) -> anyhow::Result<usize> {
    if gone.is_empty() {
        return Ok(0);
    }
    remove_shortcuts(roots, gone);
    let keys: HashSet<String> = gone.iter().map(|app| metadata_key("sideload", app)).collect();
    let art_keys: HashSet<String> = gone.iter().map(|app| format!("sideload:{app}")).collect();

    json_store::update_value(&paths.sideload_library(), |doc| {
        if let Some(games) = doc.get_mut("games").and_then(Value::as_array_mut) {
            games.retain(|g| g.get("app_name").and_then(Value::as_str).is_none_or(|a| !gone.contains(a)));
        }
        Ok(())
    })?;
    json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |f| {
        f.games.retain(|app, _| !gone.contains(app));
        Ok(())
    })?;
    json_store::update::<SessionsFile, _>(&paths.sessions(), |f| {
        f.sessions.retain(|app, _| !gone.contains(app));
        Ok(())
    })?;
    json_store::update::<ArtFile, _>(&paths.art(), |f| {
        f.games.retain(|key, _| !art_keys.contains(key));
        Ok(())
    })?;
    json_store::update::<MetadataFile, _>(&paths.metadata(), |f| {
        f.games.retain(|key, _| !keys.contains(key));
        Ok(())
    })?;
    heronite_metadata::review::update(paths, |f| {
        f.games.retain(|key, _| !keys.contains(key));
        Ok(())
    })?;
    json_store::update_value(&paths.timestamps(), |doc| {
        if let Some(map) = doc.as_object_mut() {
            map.retain(|app, _| !gone.contains(app));
        }
        Ok(())
    })?;
    drop_from_lists(paths, gone)?;
    for app in gone {
        // The pictures the app saved for the game (its own art and the metadata's).
        let _ = fs::remove_dir_all(paths.art_dir().join(heronite_metadata::util::folder_name("sideload", app)));
        heronite_metadata::images::remove_metadata_art(paths, "sideload", app);
    }
    log::info!("Removed {} Steam game(s) from Collection", gone.len());
    Ok(gone.len())
}

/// The work of [`SteamLibrary::remove_games`] once nothing reads: saves what
/// the switch asked for, removes the entries but the `running` ones, and
/// forgets the cached library. Blocking.
fn run(paths: &DataPaths, roots: &Roots, wanted: &AtomicBool, all: Vec<String>, running: &HashSet<String>) -> Result<Removal, String> {
    let (kept, entries): (Vec<String>, Vec<String>) = all.into_iter().partition(|app| running.contains(app));
    let entries: HashSet<String> = entries.into_iter().collect();
    save_wanted(paths, wanted).map_err(|e| format!("{e:#}"))?;
    let removed = remove_entries(paths, roots, &entries).map_err(|e| format!("{e:#}"))?;
    steam_owned::forget(paths).map_err(|e| format!("{e:#}"))?;
    Ok(Removal { removed, running: kept.len() })
}

impl SteamLibrary {
    /// Takes the Steam games Heronite added out of the Collection and forgets
    /// the cached library, so no game joins a Steam copy any more. Nothing is
    /// uninstalled and Steam is not touched; games the user added or got from
    /// another store stay (one that had Steam's install as its twin loses that
    /// install), and so do games that are running. Turns reading off first, or
    /// the next read would bring the games back; a read that runs now ends
    /// first.
    pub fn remove_games(&mut self, cx: &mut Context<Self>) {
        if self.removing {
            return;
        }
        self.removing = true;
        self.removal = None;
        self.enabled = false;
        self.wanted.store(false, Ordering::SeqCst);
        if self.syncing {
            self.remove_after_read = true;
            cx.notify();
            return;
        }
        self.start_removal(cx);
        cx.notify();
    }

    pub(super) fn start_removal(&mut self, cx: &mut Context<Self>) {
        let (paths, wanted) = (self.paths.clone(), self.wanted.clone());
        cx.spawn(async move |this, cx| {
            let listed = {
                let paths = paths.clone();
                cx.background_spawn(async move { added(&paths) }).await
            };
            let res = match listed {
                Err(e) => Err(e),
                Ok(entries) => {
                    let running: HashSet<String> = this
                        .update(cx, |_, cx| entries.iter().filter(|app| crate::services::session::is_running(cx, app)).cloned().collect())
                        .unwrap_or_default();
                    cx.background_spawn(async move { run(&paths, &Roots::detect(), &wanted, entries, &running) }).await
                }
            };
            this.update(cx, |this, cx| {
                this.removing = false;
                if let Err(e) = &res {
                    log::warn!("Removing the Steam games: {e}");
                }
                this.removal = Some(res);
                cx.emit(SteamLibraryEvent::Removed);
                this.load_summary(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicBool;

    use heronite_data::json_store;
    use heronite_data::steam_owned::SteamOwnedFile;
    use heronite_data::DataPaths;
    use heronite_shortcuts::menu::Roots;

    use super::{added, run, Removal};

    fn write(path: PathBuf, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn entry(app: &str, title: &str) -> String {
        format!(r#"{{"runner":"sideload","app_name":"{app}","title":"{title}","is_installed":true,"install":{{"executable":"x"}}}}"#)
    }

    fn meta(app: &str, steam: bool, auto: bool, id: &str) -> String {
        let kind = if steam { r#""launchKind":"steam-uri","# } else { "" };
        let auto = if auto { r#","heroniteSteamAuto":true"# } else { "" };
        format!(r#""{app}":{{"appName":"{app}","runner":"sideload",{kind}"steamAppId":"{id}"{auto}}}"#)
    }

    fn shortcut(dir: &Path, title: &str, app: &str) -> PathBuf {
        let file = dir.join(format!("{title}.desktop"));
        write(file.clone(), &format!("[Desktop Entry]\nName={title}\nExec=/bin/heronite-rs --launch sideload {app}\n"));
        file
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        paths: DataPaths,
        roots: Roots,
    }

    /// Two Steam entries the app added, a Steam game added by hand, a game
    /// from another store owned on Steam too, and a game with a GOG twin that
    /// shares a title with an entry.
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        let roots = Roots { desktop: Some(dir.path().join("Desktop")), applications: dir.path().join("menu") };
        write(
            paths.sideload_library(),
            &format!(
                r#"{{"games":[{},{},{},{},{}]}}"#,
                entry("steam_10", "Heronite Test One"),
                entry("steam_11", "Heronite Test Eleven"),
                entry("steam_400", "Heronite Test Two"),
                entry("mine", "Heronite Test Mine"),
                entry("ctl", "Heronite Test Control"),
            ),
        );
        write(
            paths.local_meta(),
            &format!(
                r#"{{"games":{{{},{},{},{},{}}}}}"#,
                meta("steam_10", true, true, "10"),
                meta("steam_11", true, true, "11"),
                // A Steam game the user added by hand.
                meta("steam_400", true, false, "400"),
                meta("mine", false, false, ""),
                // Owned on Steam too: Steam's copy is its twin.
                meta("ctl", false, false, "870780"),
            ),
        );
        write(
            paths.steam_owned(),
            r#"{"games":[{"appId":"10","name":"Heronite Test One"},{"appId":"870780","name":"Heronite Test Control","installed":true}],"updatedAt":"x"}"#,
        );
        write(paths.sessions(), r#"{"sessions":{"steam_10":[],"mine":[]}}"#);
        write(paths.timestamps(), r#"{"steam_10":{"totalPlayed":3},"mine":{"totalPlayed":1}}"#);
        write(
            paths.metadata(),
            r#"{"games":{"sideload:steam_10":{"appName":"steam_10","runner":"sideload"},"sideload:mine":{"appName":"mine","runner":"sideload"}}}"#,
        );
        write(
            paths.art(),
            r#"{"games":{"sideload:steam_10":{"coverUrl":"https://x/y.png"},"sideload:mine":{"coverUrl":"https://x/z.png"}}}"#,
        );
        write(
            paths.store_config(),
            r#"{"games":{"recent":[{"appName":"steam_10","title":"a"},{"appName":"mine","title":"b"}],"favourites":[{"appName":"steam_10","title":"a"}],"hidden":[]},"other":1}"#,
        );
        Fixture { _dir: dir, paths, roots }
    }

    #[test]
    fn removing_the_steam_games_keeps_the_others_and_forgets_the_library() {
        let f = fixture();
        let load = || heronite_data::Library::load_with(&f.paths, true).unwrap();
        let steam_link =
            |lib: &heronite_data::Library, app: &str| lib.games.iter().find(|g| g.app_name == app).map(|g| g.steam_link.is_some());
        assert_eq!(steam_link(&load(), "ctl"), Some(true));
        let wanted = AtomicBool::new(false);

        let all = added(&f.paths).unwrap();
        assert_eq!(all, ["steam_10", "steam_11"]);
        assert_eq!(run(&f.paths, &f.roots, &wanted, all, &HashSet::new()), Ok(Removal { removed: 2, running: 0 }));

        let lib = load();
        let names: Vec<&str> = lib.games.iter().map(|g| g.app_name.as_str()).collect();
        assert!(!names.contains(&"steam_10") && !names.contains(&"steam_11"), "{names:?}");
        for stays in ["steam_400", "mine", "ctl"] {
            assert!(names.contains(&stays), "{stays} stays: {names:?}");
        }
        // The game owned on Steam too stays, without Steam's copy.
        assert_eq!(steam_link(&lib, "ctl"), Some(false));
        assert!(!SteamOwnedFile::read(&f.paths).known());
        assert!(!heronite_data::settings::read_steam_library(&f.paths));
        other_files_lost_only_those_games(&f.paths);
    }

    fn other_files_lost_only_those_games(paths: &DataPaths) {
        let sessions = json_store::read_value(&paths.sessions()).unwrap();
        assert!(sessions["sessions"].get("steam_10").is_none());
        assert!(sessions["sessions"].get("mine").is_some());
        let stamps = json_store::read_value(&paths.timestamps()).unwrap();
        assert!(stamps.get("steam_10").is_none());
        assert!(stamps.get("mine").is_some());
        let md = json_store::read_value(&paths.metadata()).unwrap();
        assert!(md["games"].get("sideload:steam_10").is_none());
        assert!(md["games"].get("sideload:mine").is_some());
        let art = json_store::read_value(&paths.art()).unwrap();
        assert!(art["games"].get("sideload:steam_10").is_none());
        assert!(art["games"].get("sideload:mine").is_some());
        let cfg = json_store::read_value(&paths.store_config()).unwrap();
        assert_eq!(cfg["games"]["recent"].as_array().unwrap().len(), 1);
        assert!(cfg["games"]["favourites"].as_array().unwrap().is_empty());
        assert_eq!(cfg["other"], 1);
    }

    #[test]
    fn a_running_game_stays() {
        let f = fixture();
        let wanted = AtomicBool::new(false);
        let all = added(&f.paths).unwrap();
        let running = HashSet::from(["steam_11".to_string()]);
        assert_eq!(run(&f.paths, &f.roots, &wanted, all, &running), Ok(Removal { removed: 1, running: 1 }));
        // Found again by the next try.
        assert_eq!(added(&f.paths).unwrap(), ["steam_11"]);
    }

    #[test]
    fn only_the_shortcuts_that_start_the_game_go() {
        let f = fixture();
        let menu = &f.roots.applications;
        let desktop = f.roots.desktop.clone().unwrap();
        // The Steam entry's own shortcuts, and a GOG game of the same title
        // (the file is named after the title, so it replaced no other).
        let own = shortcut(menu, "Heronite Test One", "steam_10");
        let own_desktop = shortcut(&desktop, "Heronite Test One", "steam_10");
        let other = shortcut(menu, "Heronite Test One GOG", "gog_1234");
        let renamed = shortcut(menu, "A title the user changed", "steam_11");
        let wanted = AtomicBool::new(false);
        let all = added(&f.paths).unwrap();
        run(&f.paths, &f.roots, &wanted, all, &HashSet::new()).unwrap();
        assert!(!own.exists() && !own_desktop.exists());
        assert!(!renamed.exists(), "found by what it runs, not by its title");
        assert!(other.exists(), "a shortcut of another game stays");
    }
}
