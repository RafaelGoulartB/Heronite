//! libretro-thumbnails, the box art RetroArch shows for its playlists:
//! `https://thumbnails.libretro.com/<System>/Named_Boxarts/<Game>.png`, one
//! PNG (512 px wide at most) per game, named after the game as No-Intro
//! and Redump name its dump ([`names`]). No API, no key, no account, and
//! no search: a picture exists or the server answers 404.
//!
//! A lookup of an emulated game asks for its dump's file name on its
//! platform's system folder ([`crate::platforms::Database::Libretro`]),
//! then for shorter and guessed names ([`names::attempts`]); each try is one
//! small request, and a game costs [`MAX_REQUESTS`] at most. The request
//! is a `Range` GET of a PNG's first 32 bytes: the server answers 404 for a
//! missing file and the header of the PNG, which says its size, for an
//! existing one. The size decides the cover family ([`family_of`]): box art
//! is tall for most systems but square for CD games and landscape for
//! Nintendo's cartridge boxes, so no platform can say it in advance.
//!
//! Only the dump's own name is a sure match; any other name is the same
//! title, and a fuzzy one.

pub mod names;
#[cfg(test)]
mod tests;

use heronite_data::cover_family::CoverFamily;
use heronite_i18n::Lang;

use crate::http::{Http, APP_UA};
use crate::platforms::{self, Database, Platform};
use crate::sources::SourceGame;
use crate::types::{Lookup, MatchedBy};
use crate::util::encode_uri_component;

pub const SOURCE: &str = "libretro";
/// Requests one game costs at most, whatever the platform has: its own
/// name on each system of the platform, then the shorter and guessed ones.
pub const MAX_REQUESTS: usize = 8;

/// The system a file extension says, for the platforms with several
/// folders (a `.gbc` game is on the Game Boy Color's, a `.gb` one on the
/// Game Boy's). Each extension is one the platform lists
/// ([`platforms::by_extension`]) and each system one of its libretro
/// column. A disc image or an archive says nothing: every system is asked.
const BY_EXTENSION: &[(&str, &str)] = &[
    ("nes", "Nintendo - Nintendo Entertainment System"),
    ("unf", "Nintendo - Nintendo Entertainment System"),
    ("unif", "Nintendo - Nintendo Entertainment System"),
    ("fds", "Nintendo - Family Computer Disk System"),
    ("sfc", "Nintendo - Super Nintendo Entertainment System"),
    ("smc", "Nintendo - Super Nintendo Entertainment System"),
    ("swc", "Nintendo - Super Nintendo Entertainment System"),
    ("fig", "Nintendo - Super Nintendo Entertainment System"),
    ("bs", "Nintendo - Satellaview"),
    ("gb", "Nintendo - Game Boy"),
    ("sgb", "Nintendo - Game Boy"),
    ("gbc", "Nintendo - Game Boy Color"),
    ("n64", "Nintendo - Nintendo 64"),
    ("v64", "Nintendo - Nintendo 64"),
    ("z64", "Nintendo - Nintendo 64"),
    ("nds", "Nintendo - Nintendo DS"),
    ("dsi", "Nintendo - Nintendo DSi"),
    ("ngp", "SNK - Neo Geo Pocket"),
    ("ngc", "SNK - Neo Geo Pocket Color"),
    ("ngpc", "SNK - Neo Geo Pocket Color"),
    ("ws", "Bandai - WonderSwan"),
    ("wsc", "Bandai - WonderSwan Color"),
    ("mx1", "Microsoft - MSX"),
    ("mx2", "Microsoft - MSX2"),
    ("pce", "NEC - PC Engine - TurboGrafx 16"),
    ("sgx", "NEC - PC Engine SuperGrafx"),
    ("adf", "Commodore - Amiga"),
    ("adz", "Commodore - Amiga"),
    ("hdf", "Commodore - Amiga"),
];

/// Width / height below which box art is a tall cover and up to which it
/// is a square one; above [`WIDE_FROM`] it is wide. The middles (in the
/// ratio) of 2:3..5:7 and 1:1, and of 1:1 and 16:9.
const SQUARE_FROM: f32 = 0.85;
const WIDE_FROM: f32 = 1.33;

/// The cover family a picture of `width` x `height` belongs to.
pub fn family_of(width: u32, height: u32) -> CoverFamily {
    let ratio = width as f32 / height.max(1) as f32;
    if ratio < SQUARE_FROM {
        CoverFamily::Tall
    } else if ratio < WIDE_FROM {
        CoverFamily::Square
    } else {
        CoverFamily::Wide
    }
}

/// The size a PNG's first bytes say (its signature, then the `IHDR`
/// chunk). `None`: not a PNG.
pub fn png_size(head: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if head.len() < 24 || head[..8] != SIGNATURE || &head[12..16] != b"IHDR" {
        return None;
    }
    let number = |at: usize| u32::from_be_bytes([head[at], head[at + 1], head[at + 2], head[at + 3]]);
    let (w, h) = (number(16), number(20));
    (w > 0 && h > 0).then_some((w, h))
}

/// The systems to look a game of `platform` up in: the one its ROM's
/// extension says, else all of the platform's.
pub fn systems_for(platform: &Platform, rom_file: Option<&str>) -> Vec<&'static str> {
    let systems = platform.names_in(Database::Libretro);
    let extension = rom_file.and_then(|f| f.rsplit_once('.')).map(|(_, e)| e.to_ascii_lowercase());
    let said = extension.and_then(|e| BY_EXTENSION.iter().find(|(x, _)| *x == e).map(|(_, system)| *system));
    match systems.iter().find(|s| Some(**s) == said) {
        Some(system) => vec![*system],
        None => systems.to_vec(),
    }
}

/// The picture a lookup found.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// The system folder and the file name (without `.png`) it was found as.
    pub system: &'static str,
    pub name: String,
    /// The address of the box art.
    pub url: String,
    pub family: CoverFamily,
    /// The game's own dump name: [`MatchedBy::ExactTitle`]; any other name
    /// of the title, [`MatchedBy::FuzzyTitle`].
    pub matched_by: MatchedBy,
    /// The title the file is named after (article first, no tags).
    pub title: String,
    /// Requests the lookup made.
    pub requests: usize,
}

enum Probe {
    Missing,
    /// The server could not answer (no network, 429, 5xx).
    Down,
    Hit(u32, u32),
}

fn picture_url(http: &Http, system: &str, name: &str) -> String {
    format!(
        "{}/{}/Named_Boxarts/{}.png",
        http.endpoints.libretro_thumbnails.trim_end_matches('/'),
        encode_uri_component(system),
        encode_uri_component(name)
    )
}

fn probe(http: &Http, url: &str) -> Probe {
    let sent = http.get(url).header("Range", "bytes=0-31").header("User-Agent", APP_UA).timeout_secs(15).max_bytes(4 * 1024 * 1024).send();
    match sent {
        Err(_) => Probe::Down,
        Ok(r) if r.ok() => png_size(&r.body).map_or(Probe::Missing, |(w, h)| Probe::Hit(w, h)),
        Ok(r) if r.status == 429 || r.status >= 500 => Probe::Down,
        Ok(_) => Probe::Missing,
    }
}

/// The box art of an emulated game, when it is of one of `wanted`'s
/// families. [`Lookup::Skipped`] for a game without a platform that has a
/// system folder; [`Lookup::NotFound`] when no name exists, the cap was
/// reached, or the picture found is of a shape nobody asked for.
pub fn find(http: &Http, game: &SourceGame, lang: Lang, wanted: &[CoverFamily]) -> Lookup<Found> {
    let Some(platform) = game.platform.as_deref().and_then(platforms::by_id) else { return Lookup::Skipped };
    let systems = systems_for(platform, game.rom_file.as_deref());
    if systems.is_empty() || wanted.is_empty() {
        return Lookup::Skipped;
    }
    let mut requests = 0;
    for (i, attempt) in names::attempts(game.rom_file.as_deref(), &game.title, names::regions_for(lang)).into_iter().enumerate() {
        // The dump's own name is asked on every system the ROM can be of (a
        // PC Engine CD game is on another folder than a cartridge); the
        // others, and a game without a ROM, on the first.
        let tried = if i == 0 && game.rom_file.is_some() { &systems[..] } else { &systems[..1] };
        for &system in tried {
            if requests == MAX_REQUESTS {
                return Lookup::NotFound;
            }
            requests += 1;
            let url = picture_url(http, system, &attempt.name);
            match probe(http, &url) {
                Probe::Missing => {}
                Probe::Down => return Lookup::Unreachable,
                Probe::Hit(w, h) => {
                    let family = family_of(w, h);
                    if !wanted.contains(&family) {
                        return Lookup::NotFound;
                    }
                    let matched_by = if attempt.exact { MatchedBy::ExactTitle } else { MatchedBy::FuzzyTitle };
                    return Lookup::Found(Found { system, name: attempt.name, url, family, matched_by, title: attempt.title, requests });
                }
            }
        }
    }
    Lookup::NotFound
}
