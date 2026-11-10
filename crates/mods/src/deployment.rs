//! What the app asks about a game's applied mods, whatever the system: on
//! Linux they are a `fuse-overlayfs` mount ([`crate::deploy`]), on Windows
//! files put into the game folder ([`crate::journal_deploy`]).

use std::path::{Path, PathBuf};

use crate::paths::ModsRoot;

/// Whether the game in `dir` runs with mods of ours applied (or a crash left
/// them for the next start): its mods and overwrite folder are not to be
/// changed meanwhile.
pub fn in_use(root: &ModsRoot, dir: &Path) -> bool {
    #[cfg(not(windows))]
    {
        crate::deploy::is_ours_mounted(root, dir)
    }
    #[cfg(windows)]
    {
        windows::journal_for(root, dir).is_some()
    }
}

/// Takes away the mods of ours applied to `dir`, if any (a game picked up
/// again after the app restarted has ended). Whether the folder is free of
/// them afterwards.
pub fn release(root: &ModsRoot, dir: &Path) -> bool {
    #[cfg(not(windows))]
    {
        crate::deploy::unmount_ours(root, dir)
    }
    #[cfg(windows)]
    {
        match windows::journal_for(root, dir) {
            Some(dirs) => crate::journal_deploy::restore(&dirs).map_err(|e| log::warn!("mods: {e}")).is_ok(),
            None => true,
        }
    }
}

/// At start: takes away the mods a crash or a kill left applied, except in
/// the folders in `keep` (games that are running now).
pub fn cleanup_stale(root: &ModsRoot, keep: &[PathBuf]) {
    #[cfg(not(windows))]
    crate::deploy::cleanup_stale(root, keep);
    #[cfg(windows)]
    windows::cleanup_stale(root, keep);
}

/// The Windows half; built everywhere so its tests run on Linux too.
#[cfg_attr(not(windows), allow(dead_code))]
mod windows {
    use std::path::{Path, PathBuf};

    use crate::journal_deploy::{deployed_game_dir, restore};
    use crate::paths::{GameDirs, ModsRoot};

    fn same(a: &Path, b: &Path) -> bool {
        let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        a == b || canon(a) == canon(b)
    }

    /// Every game of the root with a deployment still in its folder.
    fn deployed(root: &ModsRoot) -> Vec<(GameDirs, PathBuf)> {
        std::fs::read_dir(root.path())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| {
                let dirs = root.game(&e.file_name().to_string_lossy());
                let game = deployed_game_dir(&dirs)?;
                Some((dirs, game))
            })
            .collect()
    }

    pub(super) fn journal_for(root: &ModsRoot, dir: &Path) -> Option<GameDirs> {
        deployed(root).into_iter().find(|(_, game)| same(game, dir)).map(|(dirs, _)| dirs)
    }

    pub(super) fn cleanup_stale(root: &ModsRoot, keep: &[PathBuf]) {
        for (dirs, game) in deployed(root) {
            if keep.iter().any(|k| same(k, &game)) {
                continue;
            }
            if let Err(e) = restore(&dirs) {
                log::warn!("mods: the mods a crash left in {} could not be taken out: {e}", game.display());
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::journal_deploy::{deploy, is_deployed};
        use std::fs;

        fn put(path: PathBuf, text: &str) {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }

        #[test]
        fn a_crash_is_cleaned_at_start_except_for_running_games() {
            let t = tempfile::tempdir().unwrap();
            let root = ModsRoot::at(t.path().join("mods"));
            let mut games = Vec::new();
            for key in ["one", "two"] {
                let game = t.path().join(key);
                put(game.join("a.txt"), "game");
                let dirs = root.game(key);
                put(dirs.mod_dir("m").join("a.txt"), "mod");
                std::mem::forget(deploy(&dirs, &game, &[dirs.mod_dir("m")]).unwrap());
                games.push((dirs, game));
            }
            assert!(journal_for(&root, &games[0].1).is_some());
            assert!(journal_for(&root, &t.path().join("elsewhere")).is_none());
            cleanup_stale(&root, std::slice::from_ref(&games[1].1));
            assert!(!is_deployed(&games[0].0));
            assert_eq!(fs::read_to_string(games[0].1.join("a.txt")).unwrap(), "game");
            assert!(is_deployed(&games[1].0), "a running game keeps its mods");
            assert_eq!(fs::read_to_string(games[1].1.join("a.txt")).unwrap(), "mod");
            assert!(restore(&games[1].0).is_ok());
        }
    }
}
