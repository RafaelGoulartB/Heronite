//! A launch plan is built before the mods are mounted, and it refuses an
//! executable that is not on disk. A script extender's loader is only in the
//! mod's folder until the overlay covers the game's, so the plan of a game
//! with mods is built with a probe that also looks in the enabled mods.

use std::path::{Path, PathBuf};

use heronite_data::{CollectionGame, DataPaths};
use heronite_launch::probe::{Probe, SystemProbe};
use heronite_mods::fomod::resolve;
use heronite_mods::{store, ModsRoot};

use super::facts;

/// [`SystemProbe`] that also sees the files the enabled mods put in the
/// game's folder.
pub struct ModsProbe {
    game_dir: Option<PathBuf>,
    /// The enabled mods' folders, lowest priority first.
    layers: Vec<PathBuf>,
}

impl ModsProbe {
    /// Reads the game's list; without mods that apply it adds nothing.
    pub fn for_game(paths: &DataPaths, game: &CollectionGame) -> Self {
        let dirs = ModsRoot::default_for(paths).game(&game.key);
        if !dirs.list_file().is_file() {
            return Self { game_dir: None, layers: Vec::new() };
        }
        let list = store::load(&dirs);
        if !list.applies() {
            return Self { game_dir: None, layers: Vec::new() };
        }
        let layers = list.enabled().map(|m| dirs.mod_dir(&m.id)).collect();
        Self { game_dir: facts::resolve(paths, game).dir, layers }
    }

    /// Whether an enabled mod brings `path`, a file in the game's folder.
    fn mods_bring(&self, path: &str) -> bool {
        let Some(game_dir) = &self.game_dir else { return false };
        let Ok(rel) = Path::new(path).strip_prefix(game_dir) else { return false };
        let rel = rel.to_string_lossy();
        !rel.is_empty() && self.layers.iter().any(|layer| resolve(layer, &rel).is_some_and(|p| p.is_file()))
    }
}

impl Probe for ModsProbe {
    fn exists(&self, path: &str) -> bool {
        SystemProbe.exists(path) || self.mods_bring(path)
    }

    fn read_head(&self, path: &str, n: usize) -> Option<Vec<u8>> {
        SystemProbe.read_head(path, n)
    }

    fn which(&self, name: &str) -> Option<String> {
        SystemProbe.which(name)
    }

    fn env_var(&self, key: &str) -> Option<String> {
        SystemProbe.env_var(key)
    }

    fn home(&self) -> String {
        SystemProbe.home()
    }

    fn is_executable(&self, path: &str) -> bool {
        SystemProbe.is_executable(path) || self.mods_bring(path)
    }

    fn list_dir(&self, path: &str) -> Vec<String> {
        SystemProbe.list_dir(path)
    }

    fn gamescope_is_old(&self, bin: &str) -> bool {
        SystemProbe.gamescope_is_old(bin)
    }

    fn bwrap(&self) -> Option<heronite_launch::sandbox::Bwrap> {
        SystemProbe.bwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_only_a_mod_has_exists_in_the_games_folder() {
        let t = tempfile::tempdir().unwrap();
        let game = t.path().join("game");
        let layer = t.path().join("mod");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::create_dir_all(&layer).unwrap();
        std::fs::write(game.join("Game.exe"), b"x").unwrap();
        std::fs::write(layer.join("Skse64_Loader.exe"), b"x").unwrap();
        let probe = ModsProbe { game_dir: Some(game.clone()), layers: vec![layer] };
        let at = |name: &str| game.join(name).to_string_lossy().into_owned();
        assert!(probe.exists(&at("Game.exe")));
        assert!(probe.exists(&at("skse64_loader.exe")), "whatever the case");
        assert!(!probe.exists(&at("missing.exe")));
        assert!(!probe.exists(&t.path().join("elsewhere/skse64_loader.exe").to_string_lossy()));
        assert!(!probe.mods_bring(&game.to_string_lossy()), "the folder itself is not a mod's file");
        let none = ModsProbe { game_dir: None, layers: Vec::new() };
        assert!(!none.exists(&at("skse64_loader.exe")));
    }
}
