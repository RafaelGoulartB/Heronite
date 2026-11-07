//! Which sources a game of the Collection can have achievements on, and
//! what each source needs to find it.

use std::path::PathBuf;

use heronite_data::CollectionGame;

use crate::model::{LocalPin, Source};
use crate::prefs::Prefs;

/// One source to read for a game.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceTarget {
    Steam {
        app_id: String,
    },
    Gog {
        product_id: String,
    },
    /// Epic games by legendary's app name (the sandbox comes from its metadata).
    Epic {
        app_name: String,
    },
    Retro(RetroTarget),
    Local(LocalTarget),
}

/// Where a game that keeps its achievements in its own files may have them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LocalTarget {
    /// For its launch settings (the Wine prefix, `SteamAppId`).
    pub app_name: String,
    /// The executable's folder, then the install folder (absolute, no repeats).
    pub folders: Vec<PathBuf>,
    /// The Wine prefix of a game imported from Lutris, Bottles or Faugus.
    pub launcher_prefix: Option<PathBuf>,
    /// The Steam app id the Collection knows: a hint, used only when the
    /// game's files confirm it.
    pub steam_app_id: Option<String>,
    /// The choice made by hand (filled in from the copy at refresh).
    pub pin: Option<LocalPin>,
}

/// What identifies a ROM on RetroAchievements.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RetroTarget {
    /// The ROM files, the one picked to play first. Only absolute paths.
    pub roms: Vec<PathBuf>,
    /// The Collection's platform id (`sony_playstation2`), when it is a real one.
    pub platform_id: Option<String>,
    /// Platform names of the metadata (`PlayStation 2`).
    pub platforms: Vec<String>,
    pub title: String,
    /// The game picked by hand for a ROM no set matches.
    pub pinned_game_id: Option<u32>,
}

impl SourceTarget {
    pub fn source(&self) -> Source {
        match self {
            SourceTarget::Steam { .. } => Source::Steam,
            SourceTarget::Gog { .. } => Source::Gog,
            SourceTarget::Epic { .. } => Source::Epic,
            SourceTarget::Retro(_) => Source::RetroAchievements,
            SourceTarget::Local(_) => Source::Local,
        }
    }
}

/// The sources of a game, the one it plays from first. A game of another
/// store that is also owned on Steam gets Steam after its own store.
pub fn targets_of(game: &CollectionGame) -> Vec<SourceTarget> {
    let meta = game.meta.as_deref();
    let source = meta.map(|m| m.source.as_str()).unwrap_or("");
    let mut out = Vec::new();
    match game.runner.as_str() {
        "legendary" => out.push(SourceTarget::Epic { app_name: game.app_name.clone() }),
        "gog" => out.push(SourceTarget::Gog { product_id: game.app_name.clone() }),
        _ => {}
    }
    if source == "steam" {
        if let Some(id) = game.steam_app_id.clone().filter(|id| !id.is_empty()) {
            out.push(SourceTarget::Steam { app_id: id });
        }
    }
    if source == "emulator" || game.launch_kind.as_deref() == Some("emulator") {
        let roms: Vec<PathBuf> = meta
            .and_then(|m| m.selected_rom_path.as_deref())
            .into_iter()
            .chain(meta.and_then(|m| m.roms.as_ref()).into_iter().flatten().map(|r| r.path.as_str()))
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .fold(Vec::new(), |mut v, p| {
                if !v.contains(&p) {
                    v.push(p);
                }
                v
            });
        if !roms.is_empty() {
            let platform_id = meta.and_then(|m| m.platform_id.clone()).filter(|p| !p.is_empty() && p != "other");
            out.push(SourceTarget::Retro(RetroTarget {
                roms,
                platform_id,
                platforms: game.details.platforms.clone(),
                title: game.title.clone(),
                pinned_game_id: None,
            }));
        }
    }
    if let Some(link) = &game.steam_link {
        if !out.iter().any(|t| matches!(t, SourceTarget::Steam { .. })) {
            out.push(SourceTarget::Steam { app_id: link.app_id.clone() });
        }
    }
    out
}

/// [`targets_of`] with the options: game files (off by default) add the
/// [`LocalTarget`], after the game's own store and before its Steam copy.
pub fn targets_with(game: &CollectionGame, prefs: &Prefs) -> Vec<SourceTarget> {
    let mut out = targets_of(game);
    if prefs.game_files {
        if let Some(local) = local_target(game) {
            let at = out.iter().position(|t| matches!(t, SourceTarget::Steam { .. })).unwrap_or(out.len());
            out.insert(at, SourceTarget::Local(local));
        }
    }
    out
}

/// A game that may keep its achievements in its own files: started from its
/// executable, not through the Steam client nor as an emulator's ROM.
fn plays_its_own_files(source: &str, launch_kind: &str, via_steam: bool) -> bool {
    !(via_steam || matches!(source, "steam" | "emulator") || matches!(launch_kind, "steam-uri" | "emulator"))
}

/// The game-files target of a game started from its own executable (not
/// through the Steam client, not an emulator's ROM), when it has a folder.
fn local_target(game: &CollectionGame) -> Option<LocalTarget> {
    let meta = game.meta.as_deref();
    let source = meta.map(|m| m.source.as_str()).unwrap_or("");
    if !plays_its_own_files(source, game.launch_kind.as_deref().unwrap_or(""), game.via_steam) {
        return None;
    }
    let exe = [
        meta.and_then(|m| m.remapped_executable.as_deref()),
        meta.and_then(|m| m.windows_executable.as_deref()),
        game.game.install.executable.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(PathBuf::from)
    .find(|p| p.is_absolute())
    .and_then(|p| p.parent().map(PathBuf::from));
    let installs = [meta.and_then(|m| m.windows_install_directory.as_deref()), game.game.install.install_path.as_deref()]
        .into_iter()
        .flatten()
        .map(PathBuf::from);
    let mut folders: Vec<PathBuf> = Vec::new();
    for f in exe.into_iter().chain(installs) {
        if f.is_absolute() && !folders.contains(&f) {
            folders.push(f);
        }
    }
    if folders.is_empty() {
        return None;
    }
    let launcher_prefix = meta
        .and_then(|m| m.extra.get(heronite_data::launchers::PREFIX_KEY))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let steam_app_id = game.steam_app_id.clone().or_else(|| game.steam_link.as_ref().map(|l| l.app_id.clone())).filter(|s| !s.is_empty());
    Some(LocalTarget { app_name: game.app_name.clone(), folders, launcher_prefix, steam_app_id, pin: None })
}

/// The order the sets of a game are shown in: the order of its targets.
pub fn order_of(targets: &[SourceTarget]) -> Vec<Source> {
    targets.iter().map(SourceTarget::source).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_games_started_from_their_own_files_have_game_files() {
        assert!(plays_its_own_files("manual", "executable", false));
        assert!(plays_its_own_files("", "", false));
        assert!(plays_its_own_files("gog", "executable", false));
        assert!(!plays_its_own_files("steam", "executable", false), "a Steam game's achievements are Steam's");
        assert!(!plays_its_own_files("other", "steam-uri", false), "started through the Steam client");
        assert!(!plays_its_own_files("emulator", "emulator", false));
        assert!(!plays_its_own_files("gog", "executable", true), "plays through its Steam copy");
    }
}
