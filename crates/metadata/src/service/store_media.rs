//! The Store media's screenshots of a game Steam has nothing for.

use std::path::Path;

use heronite_data::source_prefs::is_on;

use super::MetadataService;
use crate::store_media::shots::{self, ShotsGame};
use crate::store_media::{kept_or_fetched, StoreMedia};
use crate::types::Lookup;

impl MetadataService {
    /// The screenshots of `game` from the first source that is on and has
    /// any, in the user's source order (Steam is not asked here: the caller
    /// asks it first). Each source answers for the game the record matched
    /// ([`shots`]); the lists of the sources asked over the network are kept
    /// under `dir`. Blocking.
    pub fn other_screenshots(&self, dir: &Path, game: &ShotsGame) -> Lookup<StoreMedia> {
        let m = self.metadata_settings();
        let mut unreachable = false;
        for id in self.sources().order(&m) {
            if !is_on(&m, &id) {
                continue;
            }
            let found = match id.as_str() {
                "igdb" => match game.igdb_id {
                    Some(i) => kept_or_fetched(dir, "igdb", &i.to_string(), || shots::igdb(&self.igdb, &self.http, &self.credentials(), i)),
                    None => continue,
                },
                "store" => match &game.gog_id {
                    Some(g) => kept_or_fetched(dir, "gog", g, || shots::gog(&self.http, g)),
                    None if !game.store_shots.is_empty() => shots::amazon(&game.store_shots),
                    None => continue,
                },
                crate::launchbox::SOURCE => match game.launchbox_id {
                    Some(i) => shots::launchbox(&self.launchbox, &self.paths, &self.http, i),
                    None => continue,
                },
                crate::libretro::SOURCE => match &game.libretro_boxart {
                    Some(u) => kept_or_fetched(dir, "libretro", &url_key(u), || shots::libretro(&self.http, u)),
                    None => continue,
                },
                _ => continue,
            };
            match found {
                Lookup::Found(m) if !m.screenshots.is_empty() => return Lookup::Found(m),
                Lookup::Unreachable => unreachable = true,
                _ => {}
            }
        }
        if unreachable {
            Lookup::Unreachable
        } else {
            Lookup::NotFound
        }
    }
}

/// A short name for the list of an address (FNV-1a, stable across builds).
fn url_key(url: &str) -> String {
    let hash = url.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3));
    format!("{hash:016x}")
}
