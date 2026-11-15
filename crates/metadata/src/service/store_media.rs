//! The Store media of a game: from the first source, in the order of the
//! "Store media" field, that has any.

use std::path::Path;

use heronite_data::settings::STORE_MEDIA;

use super::MetadataService;
use crate::store_media::{kept_or_fetched, shots, steam_for, MediaGame, StoreMedia};
use crate::types::Lookup;

impl MetadataService {
    /// The trailers and screenshots of `game` from the first source of the
    /// "Store media" field's order (the user's, else Steam, IGDB, the store,
    /// LaunchBox, libretro) that is on, is not switched off for the field,
    /// and has anything the section shows: only one source is ever shown.
    /// Steam's app id comes with its media. Each other source answers for the
    /// game its record matched ([`shots`]) and has screenshots only. The
    /// lists of the sources asked over the network are kept under `dir`.
    /// Blocking.
    pub fn store_media(&self, dir: &Path, game: &MediaGame) -> Lookup<(Option<String>, StoreMedia)> {
        let m = self.metadata_settings();
        let o = &game.others;
        let mut unreachable = false;
        for id in self.sources().field_order(&m, STORE_MEDIA) {
            let found = match id.as_str() {
                "steam" => match &game.steam {
                    Some(target) => steam_for(&self.http, &self.paths, dir, target).map(|(id, m)| (Some(id), m)),
                    None => continue,
                },
                _ if !game.screenshots => continue,
                "igdb" => match o.igdb_id {
                    Some(i) => kept_or_fetched(dir, "igdb", &i.to_string(), || shots::igdb(&self.igdb, &self.http, &self.credentials(), i))
                        .map(|m| (None, m)),
                    None => continue,
                },
                "store" => match &o.gog_id {
                    Some(g) => kept_or_fetched(dir, "gog", g, || shots::gog(&self.http, g)).map(|m| (None, m)),
                    None if !o.store_shots.is_empty() => shots::amazon(&o.store_shots).map(|m| (None, m)),
                    None => continue,
                },
                crate::launchbox::SOURCE => match o.launchbox_id {
                    Some(i) => shots::launchbox(&self.launchbox, &self.paths, &self.http, i).map(|m| (None, m)),
                    None => continue,
                },
                crate::libretro::SOURCE => match &o.libretro_boxart {
                    Some(u) => kept_or_fetched(dir, "libretro", &url_key(u), || shots::libretro(&self.http, u)).map(|m| (None, m)),
                    None => continue,
                },
                _ => continue,
            };
            match found {
                Lookup::Found((id, m)) if game.shows(&m) => return Lookup::Found((id, m)),
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
