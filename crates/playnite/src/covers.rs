//! Cover lookup for freshly imported sideload games (`fetchCoversForGame` of
//! `covers.ts`). The network parts (SteamGridDB, Steam/Lutris searches, Bottles
//! grid) are not ported; plug them in through [`CoverFetcher`].

use heronite_data::models::{GameInfo, LocalGameMeta};
use heronite_launch::steam::steam_cdn_covers;

/// `Pick<GameInfo, 'art_cover' | 'art_square'>`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Covers {
    pub art_cover: Option<String>,
    pub art_square: Option<String>,
}

impl Covers {
    pub fn has_cover(&self) -> bool {
        self.art_cover.as_deref().is_some_and(|s| !s.is_empty())
            || self.art_square.as_deref().is_some_and(|s| !s.is_empty())
    }
}

pub trait CoverFetcher: Send + Sync {
    fn fetch(&self, meta: &LocalGameMeta, game: &GameInfo) -> Covers;
}

/// Offline default: Steam CDN URLs for games with a Steam app id (exactly what
/// `steamCovers` returns when no SteamGridDB key is set); other games keep
/// their current (empty) artwork.
#[derive(Debug, Default, Clone, Copy)]
pub struct SteamCdnCovers;

impl CoverFetcher for SteamCdnCovers {
    fn fetch(&self, meta: &LocalGameMeta, game: &GameInfo) -> Covers {
        if let Some(id) = meta.steam_app_id.as_deref().filter(|s| !s.is_empty()) {
            let (art_cover, art_square) = steam_cdn_covers(id);
            return Covers { art_cover: Some(art_cover), art_square: Some(art_square) };
        }
        Covers { art_cover: game.art_cover.clone(), art_square: game.art_square.clone() }
    }
}

/// Never fetches anything.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoCovers;

impl CoverFetcher for NoCovers {
    fn fetch(&self, _meta: &LocalGameMeta, game: &GameInfo) -> Covers {
        Covers { art_cover: game.art_cover.clone(), art_square: game.art_square.clone() }
    }
}
