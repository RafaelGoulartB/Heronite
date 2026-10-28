//! Folders in the games folders that look like the install of a game that is
//! gone: what an uninstall left behind (saves, configs, a half-removed
//! install), or a game removed from the Collection. "Nothing points to it" is
//! not enough, since a games folder may be any folder the user chose
//! (`~/Documents`, `/mnt/data`): a folder is offered only with evidence it is
//! a game install, see [`left_behind`].
//!
//! A folder is kept out when anything known lives in it: an installed game's
//! files, a prefix, a ROM folder, an emulator, a download in the queue, a
//! Steam library, a game of Lutris, Bottles, Faugus or a store client (or its
//! prefix), any path the settings name, or when [`protected_folder`] refuses
//! it (home, the XDG folders, this app's folders, a mount point...). Folders
//! changed in the last hours are left out too (an install may be writing to
//! them).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use heronite_data::install_size::SizeCache;
use heronite_data::sideload_uninstall::{guess_game_folder, protected_folder};
use heronite_data::{game_config, json_store, CollectionGame, Library};
use serde_json::Value;

use crate::{games, Ctx, Group, Item, Kind, State};

/// A folder changed more recently than this is not offered.
const RECENT: Duration = Duration::from_secs(2 * 3600);

/// A folder where games are installed.
struct Root {
    path: PathBuf,
    /// Made for games by this app or Heroic (`~/Games/Heronite`,
    /// `~/Games/Heroic`): any program file in a folder there is a game. Any
    /// other root may be a folder of the user's own.
    dedicated: bool,
}

/// The library as the Collection could show it, hidden games included: a
/// hidden game is installed all the same: it keeps its folder, prefix and
/// build. `None` when it cannot be read.
pub(crate) fn full_library(ctx: &Ctx) -> Option<Library> {
    Library::load_with(&ctx.data_paths, true).map_err(|e| log::warn!("storage: loading the library: {e:#}")).ok()
}

/// `lib` is the library with the hidden games (see [`full_library`]).
pub(crate) fn items(ctx: &Ctx, lib: &Library, listed: &[Item], walked: &SizeCache) -> Vec<Item> {
    let home = ctx.home_str();
    let global = game_config::load_global(&ctx.data_paths, &home);
    let heronite = PathBuf::from(game_config::default_install_dir(&home));
    let heroic = PathBuf::from(format!("{home}/Games/Heroic"));
    let mut roots: Vec<Root> = [
        global.get("defaultInstallPath").and_then(Value::as_str).map(|s| game_config::expand_tilde(s, &home)),
        Some(heronite.to_string_lossy().into_owned()),
        Some(heroic.to_string_lossy().into_owned()),
    ]
    .into_iter()
    .flatten()
    .map(|s| PathBuf::from(s.trim_end_matches('/')))
    .filter(|p| p.is_absolute() && p.is_dir())
    .map(|path| Root { dedicated: path == heronite || path == heroic, path })
    .collect();
    roots.sort_by(|a, b| a.path.cmp(&b.path));
    roots.dedup_by(|a, b| a.path == b.path);

    let installed_paths: Vec<PathBuf> = lib.games.iter().filter(|g| g.installed).flat_map(|g| game_paths(g)).collect();
    let mut known = installed_paths.clone();
    known.extend(listed.iter().flat_map(|i| i.paths.iter().cloned().chain([i.folder.clone()])));
    known.push(ctx.data_paths.root.clone());
    // What Lutris, Bottles, Faugus and the store clients keep is theirs, installed there or not.
    let managed = heronite_data::launchers::managed_paths(&ctx.data_paths, &home);
    known.extend(managed.iter().map(|m| m.path.clone()));
    known.extend(json_paths(&Value::Object(global)));
    for file in [ctx.data_paths.rom_scanners(), ctx.data_paths.emulators(), ctx.data_paths.download_manager()] {
        if let Ok(v) = json_store::read_value(&file) {
            known.extend(json_paths(&v));
        }
    }
    let own = [ctx.data_paths.root.clone(), ctx.app.config.clone(), ctx.app.cache.clone()];
    let former = former_folders(lib);

    let mut out = Vec::new();
    for root in roots {
        // A games folder that is itself (inside) a game.
        if installed_paths.iter().any(|p| root.path.starts_with(p)) {
            continue;
        }
        for dir in candidates(&root.path) {
            if holds_user_folder(&dir) || protected_folder(&dir, &managed, &known, &installed_paths, &own, &ctx.home).is_some() {
                continue;
            }
            let Some(title) = left_behind(&dir, root.dedicated, &former) else { continue };
            let title = title.unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            let size = walked.get(&dir);
            let mut item = Item::new(Group::Leftovers, Kind::Leftover, State::Unused, title, dir);
            item.size = size;
            item.needs_walk = size.is_none();
            out.push(item);
        }
    }
    out
}

/// The evidence that `dir` is a game install left behind, or `None` when
/// there is none. `Some(title)` carries the title of the uninstalled game
/// that was in it, when one was.
///
/// Evidence is either of:
/// - an uninstalled game of the library was installed in it ([`former_folders`]);
/// - it holds what a store writes into the folder of a game: `.egstore` (Epic),
///   `goggame-*.info` or `gameinfo` with `start.sh` (GOG), `fuel.json`
///   (Amazon), and in a dedicated root, also a Windows program (`*.exe`) in it
///   or in a folder right below.
///
/// A Wine prefix never counts as a game.
fn left_behind(dir: &Path, dedicated: bool, former: &HashMap<PathBuf, String>) -> Option<Option<String>> {
    if let Some(title) = former.get(dir) {
        return Some(Some(title.clone()));
    }
    let prefix = dir.join("drive_c").exists() || dir.join("system.reg").is_file();
    (!prefix && (store_marker(dir) || (dedicated && has_program(dir)))).then_some(None)
}

/// What the stores write at the top of a game's folder.
fn store_marker(dir: &Path) -> bool {
    let gog_info = fs::read_dir(dir).is_ok_and(|rd| {
        rd.flatten().any(|e| {
            let name = e.file_name().to_string_lossy().to_lowercase();
            name.starts_with("goggame-") && name.ends_with(".info") && e.file_type().is_ok_and(|t| t.is_file())
        })
    });
    dir.join(".egstore").is_dir()
        || dir.join("fuel.json").is_file()
        || gog_info
        || (dir.join("gameinfo").is_file() && dir.join("start.sh").is_file())
}

/// A `.exe` in `dir` or in a folder right below it.
fn has_program(dir: &Path) -> bool {
    let is_exe = |p: &Path| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"));
    let Ok(rd) = fs::read_dir(dir) else { return false };
    rd.flatten().any(|e| {
        let (path, kind) = (e.path(), e.file_type());
        match kind {
            Ok(t) if t.is_file() => is_exe(&path),
            // Not followed: a link leads out of the folder.
            Ok(t) if t.is_dir() => {
                fs::read_dir(&path).is_ok_and(|rd| rd.flatten().any(|e| e.file_type().is_ok_and(|t| t.is_file()) && is_exe(&e.path())))
            }
            _ => false,
        }
    })
}

/// The folders the uninstalled games of the library were installed in,
/// with their titles: the install path they kept, or the folder of their
/// executable. Emulated games are left out: their paths are the ROMs'.
fn former_folders(lib: &Library) -> HashMap<PathBuf, String> {
    let mut out = HashMap::new();
    for g in lib.games.iter().filter(|g| !g.installed && g.launch_kind.as_deref() != Some("emulator")) {
        let exe = g.game.install.executable.as_deref().map(str::trim).filter(|e| Path::new(e).is_absolute());
        let exe_dir = exe.filter(|e| !e.to_ascii_lowercase().ends_with(".appimage")).and_then(|e| guess_game_folder(Path::new(e)));
        let paths = [g.details.install_path.as_deref(), g.game.install.install_path.as_deref()]
            .into_iter()
            .flatten()
            .map(|p| PathBuf::from(p.trim()))
            .filter(|p| p.is_absolute())
            .chain(exe_dir);
        for p in paths {
            out.entry(p).or_insert_with(|| g.title.clone());
        }
    }
    out
}

/// Whether `dir` holds an XDG user folder (`Documents`, `Downloads`...): a
/// whole drive of the user's files, wherever it is mounted.
fn holds_user_folder(dir: &Path) -> bool {
    [
        dirs::document_dir(),
        dirs::download_dir(),
        dirs::desktop_dir(),
        dirs::picture_dir(),
        dirs::video_dir(),
        dirs::audio_dir(),
        dirs::public_dir(),
        dirs::template_dir(),
        dirs::data_dir(),
        dirs::config_dir(),
        dirs::cache_dir(),
        dirs::state_dir(),
    ]
    .into_iter()
    .flatten()
    .any(|d| d.starts_with(dir))
}

/// Every path a game's entry names: install folder, executable, ROMs.
fn game_paths(g: &CollectionGame) -> Vec<PathBuf> {
    let meta = g.meta.as_deref();
    [g.details.install_path.as_deref(), g.game.install.install_path.as_deref(), g.game.install.executable.as_deref()]
        .into_iter()
        .flatten()
        .chain(meta.and_then(|m| m.selected_rom_path.as_deref()))
        .chain(meta.and_then(|m| m.roms.as_ref()).into_iter().flatten().map(|r| r.path.as_str()))
        .map(|p| PathBuf::from(p.trim()))
        .filter(|p| p.is_absolute())
        .chain(games::install_dir(g))
        .collect()
}

/// The visible, non-empty folders of `root` not changed for a while.
fn candidates(root: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(root) else { return Vec::new() };
    let now = SystemTime::now();
    let mut out: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        // Not followed: a link to a folder elsewhere is not this folder's.
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .filter(|p| fs::read_dir(p).is_ok_and(|mut d| d.next().is_some()))
        .filter(|p| {
            let changed = fs::metadata(p).and_then(|m| m.modified()).unwrap_or(now);
            now.duration_since(changed).unwrap_or_default() >= RECENT
        })
        .collect();
    out.sort();
    out
}

/// Absolute paths among the string values of `v`, at any depth.
fn json_paths(v: &Value) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![v];
    while let Some(v) = stack.pop() {
        match v {
            Value::String(s) if s.starts_with('/') => out.push(PathBuf::from(s.trim_end_matches('/'))),
            Value::Array(a) => stack.extend(a),
            Value::Object(o) => stack.extend(o.values()),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finds_paths_at_any_depth() {
        let v = json!({"a": "/games/x/", "b": [{"c": "/roms"}, "relative", 3], "d": "https://x"});
        let mut p = json_paths(&v);
        p.sort();
        assert_eq!(p, [PathBuf::from("/games/x"), PathBuf::from("/roms")]);
    }

    struct Env {
        _dir: tempfile::TempDir,
        ctx: Ctx,
    }

    fn env() -> Env {
        use heronite_data::{AppPaths, DataPaths};
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let ctx = Ctx {
            data_paths: DataPaths::at(dir.path().join("data")),
            app: AppPaths { config: dir.path().join("cfg"), cache: dir.path().join("cache") },
            home,
        };
        Env { _dir: dir, ctx }
    }

    impl Env {
        /// A folder with `files` (a name ending in `/` is a folder), old enough to be offered.
        fn folder(&self, path: &Path, files: &[&str]) -> PathBuf {
            fs::create_dir_all(path).unwrap();
            for f in files {
                match f.strip_suffix('/') {
                    Some(d) => fs::create_dir_all(path.join(d)).unwrap(),
                    None => {
                        fs::create_dir_all(path.join(f).parent().unwrap()).unwrap();
                        fs::write(path.join(f), "x").unwrap();
                    }
                }
            }
            let old = SystemTime::now() - Duration::from_secs(3 * 3600);
            fs::File::open(path).unwrap().set_modified(old).unwrap();
            path.to_path_buf()
        }

        fn offered(&self) -> Vec<PathBuf> {
            let lib = full_library(&self.ctx).expect("the library loads");
            items(&self.ctx, &lib, &[], &SizeCache::default()).into_iter().map(|i| i.folder).collect()
        }

        fn set_install_path(&self, path: &Path) {
            game_config::set_global_setting(&self.ctx.data_paths, &self.ctx.home_str(), "defaultInstallPath", json!(path)).unwrap();
        }

        fn sideload(&self, installed: bool, folder: &Path) {
            json_store::write(
                &self.ctx.data_paths.sideload_library(),
                &json!({"games": [{"runner": "sideload", "app_name": "g", "title": "Gone Game", "is_installed": installed,
                                   "install": {"install_path": folder}}]}),
            )
            .unwrap();
        }
    }

    /// A store client's prefix and games sit in the games folder like a
    /// leftover would, without any installed game of ours pointing there.
    #[test]
    fn what_a_store_client_keeps_is_not_a_leftover() {
        let e = env();
        let ctx = &e.ctx;
        let root = ctx.home.join("Games/Heroic");
        let battlenet = e.folder(&root.join("Battle.net"), &["drive_c/"]);
        let anno = e.folder(&root.join("Anno 1800"), &[".egstore/"]);
        let old = e.folder(&root.join("Old Game"), &[".egstore/"]);
        let paths = &ctx.data_paths;
        let client = heronite_data::clients::Client::BattleNet;
        game_config::set_game_setting(paths, client.config_app(), &ctx.home_str(), "winePrefix", json!(battlenet)).unwrap();
        json_store::write(
            &paths.sideload_library(),
            &json!({"games": [{"runner": "sideload", "app_name": "ubisoft_635", "title": "Anno 1800", "install": {"install_path": anno}}]}),
        )
        .unwrap();
        json_store::write(
            &paths.local_meta(),
            &json!({"games": {"ubisoft_635": {"appName": "ubisoft_635", "runner": "sideload", "source": "ubisoft", "launchKind": "client"}}}),
        )
        .unwrap();

        assert_eq!(e.offered(), [old]);
    }

    #[test]
    fn a_plain_folder_under_the_default_install_path_is_not_offered() {
        let e = env();
        let root = e.ctx.home.join("Games/Heronite");
        e.folder(&root.join("Photos 2019"), &["a.jpg", "trip/b.jpg"]);
        e.folder(&root.join("Notes"), &["todo.txt"]);
        // A prefix-looking folder is not a game either, even with programs in it.
        e.folder(&root.join("Old Prefix"), &["drive_c/windows/notepad.exe", "system.reg"]);
        assert!(e.offered().is_empty());

        // A folder the user set as the games folder is no better.
        let docs = e.ctx.home.join("Documents");
        e.folder(&docs.join("Taxes"), &["2023.pdf"]);
        e.folder(&docs.join("Hobby"), &["tool.exe"]);
        e.set_install_path(&docs);
        assert!(e.offered().is_empty());
    }

    #[test]
    fn a_folder_with_a_store_marker_is_offered() {
        let e = env();
        let root = e.ctx.home.join("Games/Heronite");
        let epic = e.folder(&root.join("Epic Game"), &[".egstore/", "Game.exe"]);
        let gog = e.folder(&root.join("GOG Game"), &["goggame-1207658930.info"]);
        let amazon = e.folder(&root.join("Amazon Game"), &["fuel.json"]);
        let gog_linux = e.folder(&root.join("GOG Linux"), &["gameinfo", "start.sh"]);
        e.folder(&root.join("Only gameinfo"), &["gameinfo"]);
        assert_eq!(e.offered(), [amazon, epic, gog, gog_linux]);

        // In a folder of the user's own, a store marker is still a store's install.
        let docs = e.ctx.home.join("Documents");
        let epic = e.folder(&docs.join("Epic Game"), &[".egstore/"]);
        e.folder(&docs.join("Hobby"), &["tool.exe"]);
        e.set_install_path(&docs);
        assert!(e.offered().contains(&epic));
        assert!(!e.offered().contains(&docs.join("Hobby")));
    }

    #[test]
    fn a_program_makes_a_folder_a_game_only_in_a_games_folder_of_the_app() {
        let e = env();
        let root = e.ctx.home.join("Games/Heroic");
        let top = e.folder(&root.join("Top"), &["game.EXE"]);
        let below = e.folder(&root.join("Below"), &["bin/game.exe"]);
        e.folder(&root.join("Deeper"), &["a/b/game.exe"]);
        assert_eq!(e.offered(), [below, top]);
    }

    #[test]
    fn the_folder_of_an_uninstalled_game_is_offered_with_its_title() {
        let e = env();
        let docs = e.ctx.home.join("Documents");
        let game = e.folder(&docs.join("Whatever"), &["readme.txt"]);
        e.folder(&docs.join("Taxes"), &["2023.pdf"]);
        e.set_install_path(&docs);
        e.sideload(false, &game);
        let lib = full_library(&e.ctx).expect("the library loads");
        let found = items(&e.ctx, &lib, &[], &SizeCache::default());
        assert_eq!(
            found.iter().map(|i| (i.folder.clone(), i.title.clone())).collect::<Vec<_>>(),
            [(game.clone(), "Gone Game".to_string())]
        );

        // Installed again: nothing is left behind.
        e.sideload(true, &game);
        assert!(e.offered().is_empty());
    }

    #[test]
    fn a_protected_folder_is_never_offered() {
        let e = env();
        let ctx = &e.ctx;
        // Every folder has a store marker: only the protection keeps it out.
        let marked = [".egstore/"];

        // The games folder is the home folder: what is in it is the user's own.
        let documents = e.folder(&ctx.home.join("Documents"), &marked);
        e.set_install_path(&ctx.home);
        assert!(e.offered().is_empty(), "{:?}", e.offered());

        // Next to this app's folders: they and the folder holding home are kept.
        let top = ctx.home.parent().unwrap();
        e.folder(&ctx.data_paths.root, &marked);
        e.folder(&ctx.home, &marked);
        e.set_install_path(top);
        assert!(e.offered().is_empty(), "{:?}", e.offered());

        // A ROM folder, and a folder holding an installed game.
        let root = ctx.home.join("Games/Heronite");
        let roms = e.folder(&root.join("Roms"), &marked);
        let nested = e.folder(&root.join("Collection"), &marked);
        let stale = e.folder(&root.join("Stale"), &marked);
        e.set_install_path(&root);
        json_store::write(&ctx.data_paths.rom_scanners(), &json!({"scanners": [{"path": roms}]})).unwrap();
        json_store::write(
            &ctx.data_paths.sideload_library(),
            &json!({"games": [{"runner": "sideload", "app_name": "n", "title": "N", "is_installed": true,
                               "install": {"install_path": nested.join("Game")}}]}),
        )
        .unwrap();
        assert_eq!(e.offered(), [stale]);
        assert!(documents.exists());
    }

    #[test]
    fn leftovers_are_not_ticked_when_the_page_opens() {
        use crate::Sweep;
        assert!(!Sweep::Leftovers.default_on());
        assert!(Sweep::Prefixes.default_on() && Sweep::Caches.default_on());
    }

    #[test]
    fn candidates_are_old_non_empty_folders() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir_all(r.join("Old Game/saves")).unwrap();
        fs::create_dir_all(r.join("Empty")).unwrap();
        fs::create_dir_all(r.join(".hidden/x")).unwrap();
        fs::write(r.join("file.txt"), "x").unwrap();
        // Everything was just made: nothing is old enough yet.
        assert!(candidates(r).is_empty());
        let old = SystemTime::now() - Duration::from_secs(3 * 3600);
        for d in ["Old Game", "Empty", ".hidden"] {
            fs::File::open(r.join(d)).unwrap().set_modified(old).unwrap();
        }
        assert_eq!(candidates(r), [r.join("Old Game")]);
    }
}
