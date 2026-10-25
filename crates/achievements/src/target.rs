//! Which sources a game of the Collection can have achievements on, and
//! what each source needs to find it.

use std::path::PathBuf;

use heronite_data::CollectionGame;

use crate::model::Source;

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

/// The order the sets of a game are shown in: the order of its targets.
pub fn order_of(targets: &[SourceTarget]) -> Vec<Source> {
    targets.iter().map(SourceTarget::source).collect()
}
