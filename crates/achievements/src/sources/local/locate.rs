//! Where a game keeps its achievement files: the emulator's folder next to
//! the game (its `steam_settings`), the game's Steam app id, and the unlock
//! state in the emulator's saves folder (the user's, or the game's own).
//! Only files are looked at; nothing is written.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

use crate::target::LocalTarget;

/// The folders the emulators name their saves folder (the fork's, then the
/// original's).
const SAVES_NAMES: [&str; 2] = ["GSE Saves", "Goldberg SteamEmu Saves"];
/// The emulator's library, next to which `steam_settings` sits.
const LIBRARIES: [&str; 3] = ["steam_api64.dll", "steam_api.dll", "libsteam_api.so"];
/// A walk of the game's folder stops this deep, or after this many entries.
const WALK_DEPTH: usize = 4;
const WALK_ENTRIES: usize = 4000;

/// The places of this PC, apart from the game's folders.
#[derive(Debug, Clone, Default)]
pub struct Places {
    /// Folders that hold the emulators' saves folders: `%APPDATA%` on
    /// Windows, `~/.local/share` on Linux, and each user's
    /// `AppData/Roaming` of the game's Wine prefix.
    pub roots: Vec<PathBuf>,
    /// `SteamAppId` / `SteamGameId` of the game's launch settings.
    pub env_app_id: Option<String>,
}

/// Where the app id came from, for the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppIdFrom {
    Pin,
    File,
    LaunchSettings,
    Collection,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Located {
    pub app_id: String,
    pub from: AppIdFrom,
    /// The emulator's `steam_settings` folder.
    pub settings: Option<PathBuf>,
    /// The unlock state (the newest of the places it can be).
    pub state: Option<PathBuf>,
}

impl Located {
    /// The emulator's list of the game's achievements.
    pub fn schema_file(&self) -> Option<PathBuf> {
        self.settings.as_ref().map(|s| s.join("achievements.json")).filter(|f| f.is_file())
    }
}

/// A folder holding the emulator: `steam_settings` or its library in it.
fn is_emulator_dir(dir: &Path) -> bool {
    dir.join("steam_settings").is_dir() || LIBRARIES.iter().any(|l| dir.join(l).is_file())
}

/// The emulator's folder under `folder`: the folder itself, else the first
/// found in a short walk (no symlinks followed).
fn emulator_dir(folder: &Path) -> Option<PathBuf> {
    if is_emulator_dir(folder) {
        return Some(folder.to_path_buf());
    }
    let mut queue = VecDeque::from([(folder.to_path_buf(), 0usize)]);
    let mut seen = 0;
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            seen += 1;
            if seen > WALK_ENTRIES {
                return None;
            }
            let Ok(kind) = e.file_type() else { continue };
            if !kind.is_dir() || kind.is_symlink() {
                continue;
            }
            let path = e.path();
            if is_emulator_dir(&path) {
                return Some(path);
            }
            if depth + 1 < WALK_DEPTH {
                queue.push_back((path, depth + 1));
            }
        }
    }
    None
}

/// The digits of the first line of an app id file.
fn read_app_id(file: &Path) -> Option<String> {
    let text = fs::read_to_string(file).ok()?;
    let id = text.lines().next()?.trim();
    (!id.is_empty() && id.len() <= 10 && id.bytes().all(|b| b.is_ascii_digit())).then(|| id.to_string())
}

/// A value of `configs.user.ini`'s `[user::saves]`.
fn saves_setting(settings: &Path, key: &str) -> Option<String> {
    let text = fs::read_to_string(settings.join("configs.user.ini")).ok()?;
    let mut in_saves = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_saves = line.eq_ignore_ascii_case("[user::saves]");
            continue;
        }
        if !in_saves || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == key {
                return Some(v.trim().to_string()).filter(|v| !v.is_empty());
            }
        }
    }
    None
}

/// Every place the unlock state of `app_id` can be, best first.
fn state_files(target: &LocalTarget, places: &Places, emulator: Option<&Path>, app_id: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(folder) = target.pin.as_ref().and_then(|p| p.folder.as_ref()) {
        out.push(folder.join(app_id).join("achievements.json"));
        out.push(folder.join("achievements.json"));
    }
    let settings = emulator.map(|e| e.join("steam_settings"));
    if let (Some(emulator), Some(local)) = (emulator, settings.as_deref().and_then(|s| saves_setting(s, "local_save_path"))) {
        let base = if Path::new(&local).is_absolute() { PathBuf::from(&local) } else { emulator.join(&local) };
        out.push(base.join(app_id).join("achievements.json"));
        out.push(base.join("achievements.json"));
    }
    let custom = settings.as_deref().and_then(|s| saves_setting(s, "saves_folder_name"));
    let names: Vec<&str> = custom.as_deref().into_iter().chain(SAVES_NAMES).collect();
    for root in &places.roots {
        for name in &names {
            out.push(root.join(name).join(app_id).join("achievements.json"));
        }
    }
    out
}

fn newest(files: &[PathBuf]) -> Option<PathBuf> {
    files.iter().filter_map(|f| Some((fs::metadata(f).ok()?.modified().ok()?, f))).max_by_key(|(t, _)| *t).map(|(_, f)| f.clone())
}

/// The game's emulator, app id and unlock state; `None` without an app id.
pub fn locate(target: &LocalTarget, places: &Places) -> Option<Located> {
    let emulator = target.folders.iter().find_map(|f| emulator_dir(f));
    let settings = emulator.as_ref().map(|e| e.join("steam_settings")).filter(|s| s.is_dir());
    let pinned = target.pin.as_ref().and_then(|p| p.app_id.clone()).map(|id| (id, AppIdFrom::Pin));
    let from_file = || {
        let files = settings
            .iter()
            .map(|s| s.join("steam_appid.txt"))
            .chain(emulator.iter().map(|e| e.join("steam_appid.txt")))
            .chain(target.folders.iter().map(|f| f.join("steam_appid.txt")));
        files.into_iter().find_map(|f| read_app_id(&f)).map(|id| (id, AppIdFrom::File))
    };
    let (app_id, from) =
        pinned.or_else(from_file).or_else(|| places.env_app_id.clone().map(|id| (id, AppIdFrom::LaunchSettings))).or_else(|| {
            // The Collection's id only when the game's files confirm it.
            let id = target.steam_app_id.clone()?;
            let schema = settings.as_ref().is_some_and(|s| s.join("achievements.json").is_file());
            let saves = state_files(target, places, emulator.as_deref(), &id).iter().any(|f| f.is_file());
            (schema || saves).then_some((id, AppIdFrom::Collection))
        })?;
    let candidates = state_files(target, places, emulator.as_deref(), &app_id);
    let found: Vec<PathBuf> = candidates.into_iter().filter(|f| f.is_file()).collect();
    if found.len() > 1 {
        log::debug!("game files of {}: {} state files, the newest is read", target.app_name, found.len());
    }
    Some(Located { app_id, from, settings, state: newest(&found) })
}

/// The users' `AppData/Roaming` folders of a Wine prefix.
pub fn prefix_roots(prefix: &Path) -> Vec<PathBuf> {
    let users = heronite_data::clients::drive_c(prefix).join("users");
    let Ok(entries) = fs::read_dir(&users) else { return Vec::new() };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| !e.file_name().eq_ignore_ascii_case("Public"))
        .map(|e| e.path().join("AppData").join("Roaming"))
        .filter(|p| p.is_dir())
        .collect();
    out.sort();
    out
}

/// Where this PC's emulators keep their saves folders: `%APPDATA%` on
/// Windows; `$XDG_DATA_HOME` or `~/.local/share` elsewhere (the host's,
/// from inside a Flatpak, whose own data folder is its sandbox's).
pub fn native_roots() -> Vec<PathBuf> {
    if cfg!(windows) {
        return std::env::var_os("APPDATA").map(PathBuf::from).into_iter().collect();
    }
    let home = heronite_platform::dirs::home();
    let flatpak = std::env::var_os("FLATPAK_ID").is_some();
    let xdg = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute() && !flatpak);
    xdg.or_else(|| home.map(|h| h.join(".local").join("share"))).into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::LocalPin;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn target(folders: Vec<PathBuf>) -> LocalTarget {
        LocalTarget { app_name: "game".into(), folders, ..Default::default() }
    }

    #[test]
    fn finds_the_emulator_its_app_id_and_the_state_in_the_saves_folder() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("Game");
        write(&game.join("bin/x64/steam_api64.dll"), "");
        write(&game.join("bin/x64/steam_settings/steam_appid.txt"), "480\n");
        let roaming = dir.path().join("Roaming");
        write(&roaming.join("GSE Saves/480/achievements.json"), "{}");
        let places = Places { roots: vec![roaming.clone()], env_app_id: None };
        let found = locate(&target(vec![game.clone()]), &places).unwrap();
        assert_eq!((found.app_id.as_str(), found.from), ("480", AppIdFrom::File));
        assert_eq!(found.settings, Some(game.join("bin/x64/steam_settings")));
        assert_eq!(found.state, Some(roaming.join("GSE Saves/480/achievements.json")));
    }

    #[test]
    fn a_portable_save_path_and_a_custom_folder_name_are_followed() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("Game");
        write(&game.join("steam_appid.txt"), "70");
        write(&game.join("steam_settings/configs.user.ini"), "[user::saves]\nlocal_save_path=./saves\nsaves_folder_name=My Saves\n");
        write(&game.join("saves/70/achievements.json"), "{}");
        let found = locate(&target(vec![game.clone()]), &Places::default()).unwrap();
        assert_eq!(found.state, Some(game.join("saves/70/achievements.json")));

        let roaming = dir.path().join("Roaming");
        fs::remove_file(game.join("saves/70/achievements.json")).unwrap();
        write(&roaming.join("My Saves/70/achievements.json"), "{}");
        let found = locate(&target(vec![game]), &Places { roots: vec![roaming.clone()], env_app_id: None }).unwrap();
        assert_eq!(found.state, Some(roaming.join("My Saves/70/achievements.json")));
    }

    #[test]
    fn the_collections_app_id_needs_the_files_to_confirm_it() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("Game");
        fs::create_dir_all(&game).unwrap();
        let mut t = target(vec![game]);
        t.steam_app_id = Some("220".into());
        let roaming = dir.path().join("Roaming");
        let places = Places { roots: vec![roaming.clone()], env_app_id: None };
        assert_eq!(locate(&t, &places), None, "nothing says the game uses an emulator");
        write(&roaming.join("Goldberg SteamEmu Saves/220/achievements.json"), "{}");
        let found = locate(&t, &places).unwrap();
        assert_eq!(found.from, AppIdFrom::Collection);
        assert!(found.state.is_some());
    }

    #[test]
    fn the_launch_settings_and_a_pin_win_in_their_order() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("Game");
        write(&game.join("steam_appid.txt"), "1");
        let mut t = target(vec![game]);
        let places = Places { roots: vec![], env_app_id: Some("2".into()) };
        assert_eq!(locate(&t, &places).unwrap().app_id, "1");
        t.pin = Some(LocalPin { app_id: Some("3".into()), folder: Some(dir.path().join("pinned")), off: false });
        write(&dir.path().join("pinned/achievements.json"), "{}");
        let found = locate(&t, &places).unwrap();
        assert_eq!((found.app_id.as_str(), found.from), ("3", AppIdFrom::Pin));
        assert_eq!(found.state, Some(dir.path().join("pinned/achievements.json")));
        assert_eq!(locate(&target(vec![dir.path().join("none")]), &places).unwrap().from, AppIdFrom::LaunchSettings);
    }

    #[test]
    fn a_wine_prefix_gives_each_users_roaming_folder() {
        let dir = tempfile::tempdir().unwrap();
        let users = dir.path().join("drive_c/users");
        fs::create_dir_all(users.join("steamuser/AppData/Roaming")).unwrap();
        fs::create_dir_all(users.join("Public/AppData/Roaming")).unwrap();
        assert_eq!(prefix_roots(dir.path()), vec![users.join("steamuser/AppData/Roaming")]);
    }

    #[test]
    fn the_walk_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("a/b/c/d/e/steam_api.dll"), "");
        assert_eq!(emulator_dir(dir.path()), None, "deeper than the walk goes");
        write(&dir.path().join("a/b/steam_api.dll"), "");
        assert_eq!(emulator_dir(dir.path()), Some(dir.path().join("a/b")));
    }
}
