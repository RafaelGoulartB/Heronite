//! The launch hook: puts a game's mods in place when it starts from Heronite
//! and takes them away when the session ends (`heronite_launch::GameMods`).
//! It runs on the launcher's session thread: no GPUI in here.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use heronite_data::DataPaths;
use heronite_i18n::tr;
use heronite_launch::{AppliedMods, GameMods};
use heronite_mods::apply::{self, ApplyRequest};
use heronite_mods::games;
use heronite_mods::{deploy, install, store, ModsRoot, Progress};

use super::facts;

/// What the launcher asks for the mods of a game it starts.
pub struct ModsLaunch {
    paths: DataPaths,
}

impl ModsLaunch {
    pub fn new(paths: DataPaths) -> Self {
        Self { paths }
    }
}

impl GameMods for ModsLaunch {
    fn apply(&self, app_name: &str, runner: &str, status: &mut dyn FnMut(&str)) -> Result<Option<AppliedMods>, String> {
        apply_to(&self.paths, &format!("{runner}_{app_name}"), status)
    }
}

fn apply_to(paths: &DataPaths, game_key: &str, status: &mut dyn FnMut(&str)) -> Result<Option<AppliedMods>, String> {
    let root = ModsRoot::default_for(paths);
    let dirs = root.game(game_key);
    // Most games have no mods: a file check, nothing else.
    if !dirs.list_file().is_file() || !store::load(&dirs).applies() {
        return Ok(None);
    }
    status(tr!("Applying mods…"));
    let game = facts::find_game(paths, game_key)?;
    let found = facts::resolve(paths, &game);
    let game_dir = found.dir.ok_or_else(|| {
        tr!("The folder of {game} was not found on this computer, so its mods cannot be applied", game = game.title.clone())
    })?;
    let tools_bin = paths.root.join("tools/bin");
    let request = ApplyRequest {
        root: &root,
        tools_bin: &tools_bin,
        game_key,
        game_dir: &game_dir,
        prefix: found.prefix.as_deref().filter(|_| !found.native),
        known: games::find(game.steam_app_id.as_deref(), &game.title),
    };
    // Progress only comes while fuse-overlayfs downloads (first use).
    let mut downloading = false;
    let mut progress = |_: Progress| {
        if !std::mem::replace(&mut downloading, true) {
            status(tr!("Downloading fuse-overlayfs…"));
        }
    };
    let applied = apply::apply(&request, &mut progress, &AtomicBool::new(false))?;
    Ok(applied.map(|applied| AppliedMods { env: applied.env.clone(), summary: applied.summary.clone(), guard: Box::new(applied) }))
}

/// At start, off the UI thread: sweeps what an install that never finished
/// left in each game's folder and, with `unmount_stale`, unmounts the
/// overlays a crash left (those of the games in `running` stay). Leave
/// `unmount_stale` off when a game is about to start: its own mount is
/// being made.
pub fn startup(paths: &DataPaths, running: &[String], unmount_stale: bool) {
    let root = ModsRoot::default_for(paths);
    for entry in std::fs::read_dir(root.path()).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_dir() && entry.path() != root.cache_dir() {
            install::sweep(&root.game(&name));
        }
    }
    if unmount_stale {
        deploy::cleanup_stale(&root, &folders_of(paths, running));
    }
}

/// The folders of the games that are running (their mounts are in use).
fn folders_of(paths: &DataPaths, running: &[String]) -> Vec<PathBuf> {
    if running.is_empty() {
        return Vec::new();
    }
    let Ok(library) = heronite_data::Library::load_with(paths, true) else { return Vec::new() };
    library
        .games
        .iter()
        .filter(|g| running.contains(&g.app_name))
        .filter_map(|g| crate::services::game_folders::game_folders(paths, g).into_iter().next().map(|(_, dir)| dir))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_mods::{ModEntry, ModList};

    fn paths(t: &tempfile::TempDir) -> DataPaths {
        DataPaths { root: t.path().to_path_buf() }
    }

    #[test]
    fn a_game_without_mods_is_left_alone() {
        let t = tempfile::tempdir().unwrap();
        let mut said = Vec::new();
        let r = apply_to(&paths(&t), "legendary_x", &mut |s| said.push(s.to_string()));
        assert!(matches!(r, Ok(None)));
        assert!(said.is_empty());
    }

    #[test]
    fn mods_that_are_off_or_not_played_with_are_not_applied() {
        let t = tempfile::tempdir().unwrap();
        let p = paths(&t);
        let root = ModsRoot::default_for(&p);
        let mut list = ModList::new("legendary_x", "X");
        list.mods.push(ModEntry { id: "a".into(), enabled: false, ..Default::default() });
        store::save(&root.game("legendary_x"), &list).unwrap();
        assert!(matches!(apply_to(&p, "legendary_x", &mut |_| {}), Ok(None)));
        list.mods[0].enabled = true;
        list.active = false;
        store::save(&root.game("legendary_x"), &list).unwrap();
        assert!(matches!(apply_to(&p, "legendary_x", &mut |_| {}), Ok(None)));
    }

    #[test]
    fn mods_that_apply_to_an_unknown_game_stop_the_launch() {
        let t = tempfile::tempdir().unwrap();
        let p = paths(&t);
        let root = ModsRoot::default_for(&p);
        let mut list = ModList::new("legendary_x", "X");
        list.mods.push(ModEntry { id: "a".into(), enabled: true, ..Default::default() });
        store::save(&root.game("legendary_x"), &list).unwrap();
        let mut said = Vec::new();
        let r = apply_to(&p, "legendary_x", &mut |s| said.push(s.to_string()));
        assert!(r.is_err());
        assert_eq!(said, vec!["Applying mods…".to_string()]);
    }

    #[test]
    fn startup_sweeps_every_game_folder() {
        let t = tempfile::tempdir().unwrap();
        let p = paths(&t);
        let root = ModsRoot::default_for(&p);
        let dirs = root.game("g");
        std::fs::create_dir_all(dirs.staging().join("junk")).unwrap();
        std::fs::create_dir_all(dirs.mods_dir().join("half.partial")).unwrap();
        std::fs::create_dir_all(root.cache_dir()).unwrap();
        startup(&p, &[], false);
        assert!(!dirs.staging().exists());
        assert!(!dirs.mods_dir().join("half.partial").exists());
        assert!(root.cache_dir().exists());
    }
}
