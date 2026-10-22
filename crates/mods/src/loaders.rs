//! Script extenders and mod loaders that start the game themselves (SKSE,
//! F4SE, ModEngine 2, SMAPI…). Installing one puts its executable next to the
//! game's, but the game still starts through its own, so nothing is loaded
//! until the game is started through the loader's.

use std::path::Path;

use crate::fomod::resolve;
use crate::install::exe_dirs;
use crate::model::ModList;
use crate::paths::GameDirs;

#[derive(Debug, PartialEq, Eq)]
pub struct Loader {
    pub name: &'static str,
    /// The executable that starts the game with the loader.
    pub file: &'static str,
    /// The executable Steam starts (the part of its command to replace in the
    /// launch options); `None` when there is no such one-line replacement.
    pub steam_replaces: Option<&'static str>,
    /// Needs arguments besides the executable.
    pub needs_arguments: bool,
}

pub const LOADERS: &[Loader] = &[
    Loader { name: "SKSE", file: "skse64_loader.exe", steam_replaces: Some("SkyrimSELauncher.exe"), needs_arguments: false },
    Loader { name: "SKSE", file: "skse_loader.exe", steam_replaces: Some("SkyrimLauncher.exe"), needs_arguments: false },
    Loader { name: "F4SE", file: "f4se_loader.exe", steam_replaces: Some("Fallout4Launcher.exe"), needs_arguments: false },
    Loader { name: "NVSE", file: "nvse_loader.exe", steam_replaces: Some("FalloutNVLauncher.exe"), needs_arguments: false },
    Loader { name: "SFSE", file: "sfse_loader.exe", steam_replaces: Some("StarfieldLauncher.exe"), needs_arguments: false },
    Loader { name: "OBSE", file: "obse_loader.exe", steam_replaces: Some("OblivionLauncher.exe"), needs_arguments: false },
    Loader { name: "ModEngine 2", file: "modengine2_launcher.exe", steam_replaces: None, needs_arguments: true },
    Loader { name: "SMAPI", file: "StardewModdingAPI.exe", steam_replaces: Some("Stardew Valley.exe"), needs_arguments: false },
];

/// A loader an enabled mod brings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub loader: &'static Loader,
    pub mod_id: String,
    /// Where it will be in the game's folder (`/`-separated, as spelled in
    /// the mod).
    pub rel: String,
}

/// The loaders of the enabled mods, in the game's folder or in a folder of it
/// that holds an executable (`exe_dirs` of `game_dir`). A loader in two mods
/// is listed once, for the last mod of the list (the one that wins the file).
pub fn find(dirs: &GameDirs, list: &ModList, game_dir: &Path) -> Vec<Found> {
    let mut places = vec![String::new()];
    places.extend(exe_dirs(game_dir).into_iter().filter(|d| !d.is_empty()));
    let mut out: Vec<Found> = Vec::new();
    for m in list.enabled() {
        let base = dirs.mod_dir(&m.id);
        for loader in LOADERS {
            for place in &places {
                let rel = if place.is_empty() { loader.file.to_string() } else { format!("{place}/{}", loader.file) };
                let Some(path) = resolve(&base, &rel).filter(|p| p.is_file()) else { continue };
                let spelled = path.strip_prefix(&base).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or(rel);
                out.retain(|f| f.rel.to_lowercase() != spelled.to_lowercase());
                out.push(Found { loader, mod_id: m.id.clone(), rel: spelled });
                break;
            }
        }
    }
    out
}

/// The value for the game's `targetExe` setting: Epic's legendary takes a
/// path inside the game's folder, the other stores an absolute one.
pub fn target_exe(game_dir: &Path, rel: &str, relative: bool) -> String {
    if relative {
        rel.to_string()
    } else {
        game_dir.join(rel).to_string_lossy().into_owned()
    }
}

/// Whether the `targetExe` `current` names the loader at `rel`.
pub fn is_target(current: &str, game_dir: &Path, rel: &str) -> bool {
    let current = current.trim().replace('\\', "/").to_lowercase();
    let rel = rel.to_lowercase();
    let absolute = game_dir.join(&rel).to_string_lossy().to_lowercase();
    current == rel || current.trim_start_matches("./") == rel || current == absolute
}

/// The line to paste in a Steam game's launch options so Steam starts the
/// loader in place of the game's own launcher. `None` for a loader with no
/// such replacement.
pub fn steam_launch_line(loader: &Loader) -> Option<String> {
    let from = loader.steam_replaces?;
    Some(format!("bash -c 'exec \"${{@/{from}/{}}}\"' -- %command%", loader.file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModEntry;
    use crate::paths::ModsRoot;
    use std::fs;
    use std::path::PathBuf;

    fn put(path: PathBuf) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "x").unwrap();
    }

    fn entry(id: &str, enabled: bool) -> ModEntry {
        ModEntry { id: id.into(), enabled, ..Default::default() }
    }

    #[test]
    fn loaders_of_enabled_mods_are_found_next_to_the_games_executables() {
        let t = tempfile::tempdir().unwrap();
        let dirs = ModsRoot::at(t.path().join("mods")).game("g");
        let game = t.path().join("game");
        put(game.join("SkyrimSE.exe"));
        put(game.join("Game/eldenring.exe"));
        put(dirs.mod_dir("skse").join("Skse64_Loader.EXE"));
        put(dirs.mod_dir("skse").join("Data/other.txt"));
        put(dirs.mod_dir("me2").join("Game/modengine2_launcher.exe"));
        put(dirs.mod_dir("off").join("f4se_loader.exe"));
        put(dirs.mod_dir("plain").join("Data/x.esp"));
        let mut list = ModList::new("g", "G");
        list.mods = vec![entry("skse", true), entry("me2", true), entry("off", false), entry("plain", true)];
        let found = find(&dirs, &list, &game);
        let seen: Vec<_> = found.iter().map(|f| (f.loader.name, f.mod_id.as_str(), f.rel.as_str())).collect();
        assert_eq!(seen, [("SKSE", "skse", "Skse64_Loader.EXE"), ("ModEngine 2", "me2", "Game/modengine2_launcher.exe")]);
    }

    #[test]
    fn a_loader_two_mods_bring_is_listed_once_for_the_last() {
        let t = tempfile::tempdir().unwrap();
        let dirs = ModsRoot::at(t.path().join("mods")).game("g");
        let game = t.path().join("game");
        put(game.join("a.exe"));
        put(dirs.mod_dir("one").join("skse64_loader.exe"));
        put(dirs.mod_dir("two").join("skse64_loader.exe"));
        let mut list = ModList::new("g", "G");
        list.mods = vec![entry("one", true), entry("two", true)];
        let found = find(&dirs, &list, &game);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].mod_id, "two");
    }

    #[test]
    fn the_setting_and_the_steam_line() {
        let game = Path::new("/games/Skyrim");
        assert_eq!(target_exe(game, "skse64_loader.exe", true), "skse64_loader.exe");
        assert_eq!(target_exe(game, "skse64_loader.exe", false), "/games/Skyrim/skse64_loader.exe");
        assert!(is_target("skse64_loader.exe", game, "skse64_loader.exe"));
        assert!(is_target("/games/Skyrim/SKSE64_loader.exe", game, "skse64_loader.exe"));
        assert!(!is_target("", game, "skse64_loader.exe"));
        assert!(!is_target("/games/Skyrim/SkyrimSE.exe", game, "skse64_loader.exe"));
        assert_eq!(steam_launch_line(&LOADERS[0]).unwrap(), "bash -c 'exec \"${@/SkyrimSELauncher.exe/skse64_loader.exe}\"' -- %command%");
        assert_eq!(steam_launch_line(&LOADERS[6]), None);
    }
}
