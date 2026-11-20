//! The LaunchBox Games Database (gamesdb.launchbox-app.com): every game of
//! every platform LaunchBox knows, published as one archive
//! (`Metadata.zip`, about 105 MB, refreshed daily) with no API and no
//! account. Its pictures are served by file name from
//! `images.launchbox-app.com`.
//!
//! Nothing is fetched until the user downloads the database from the
//! source's row in Settings ([`LaunchBox::download`]); then it is refreshed
//! at most weekly by the idle maintenance, or by hand. The archive becomes
//! a local index ([`index`], built by [`build`] from the XML read as a
//! stream) that a lookup queries in well under a millisecond.
//!
//! A lookup of an emulated game stays on its platform: the dump's file
//! name in LaunchBox's own file table, then that name cleaned the way dump
//! names are written ([`crate::rom_names`]) against the names and
//! alternate names, then the game's title; an exact match of one game is
//! [`MatchedBy::ExactTitle`], anything looser [`MatchedBy::FuzzyTitle`]. A
//! PC game is looked up on LaunchBox's PC platforms, by its Steam id first.
//! Only a search by hand looks at every platform.
//!
//! The database gives no picture sizes, and box art is tall on most
//! platforms but square for CD cases and landscape for Nintendo's cartridge
//! boxes (and changes with the region). A picture's size is read from its
//! first bytes when its shape matters ([`LaunchBox::picture_size`]): to put
//! the box art in the cover family of its shape ([`map::record`]), and for
//! the pictures the Covers dialog offers ([`map::choices`]).

mod build;
mod fetch;
pub mod index;
pub mod map;
mod parse;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::Result;
use heronite_data::paths::DataPaths;
use parking_lot::Mutex;

pub use build::{build, BadArchive, Built};
use fetch::{drop_archive, fetch_archive, now_secs, Archive, Files, Stored};
use index::{Index, LbGame, NameHit};

use crate::http::Http;
use crate::picture_size::fetch_size;
use crate::platforms::{self, Database, LAUNCHBOX_PC};
use crate::rom_names::{match_key, shorter_keys, strip_extension};
use crate::sources::download::{is_cancelled, DownloadProgress, DownloadState};
use crate::sources::SourceGame;
use crate::types::{Lookup, MatchedBy};
use crate::util::score_title;

pub const SOURCE: &str = "launchbox";
/// The archive under [`Endpoints::launchbox`](crate::http::Endpoints).
pub const ARCHIVE: &str = "Metadata.zip";
/// Games a search by hand shows at most.
const SEARCH_RESULTS: usize = 20;
/// Names of each kind (exact, start, inside) a search ranks at most.
const SEARCH_SCAN: usize = 300;
/// The shortest key a loose match starts from: shorter ones begin too many names.
const MIN_LOOSE_KEY: usize = 5;
/// Picture sizes remembered at most; past it the memory starts over.
const SIZES_CAP: usize = 20_000;

/// The index in use, opened once and again when its file changes.
struct Opened {
    stamp: (Option<SystemTime>, u64),
    index: Index,
}

/// The source's state: the index it has open, and the picture sizes read.
#[derive(Default)]
pub struct LaunchBox {
    open: Mutex<Option<Opened>>,
    /// By file name (a file name is one picture for good); `None`: not a
    /// picture whose size can be read.
    sizes: Mutex<HashMap<String, Option<(u32, u32)>>>,
}

impl LaunchBox {
    pub fn new() -> Self {
        Self::default()
    }

    fn files(paths: &DataPaths) -> Files {
        Files::new(paths.source_cache_dir(SOURCE))
    }

    /// Where the index is (whether it exists or not).
    pub fn index_path(paths: &DataPaths) -> PathBuf {
        Self::files(paths).index()
    }

    /// Whether the database is downloaded, when, and how big its index is.
    /// An index that no longer opens (damaged, or of another schema after
    /// an update) is not ready: the source is not asked, and the idle
    /// maintenance downloads the database again (at once, or after the
    /// back-off of the refreshes that failed).
    pub fn state(&self, paths: &DataPaths) -> DownloadState {
        let mut state = Self::files(paths).download_state();
        if state.ready && !matches!(self.with_index(paths, |_| Ok(())), Some(Ok(()))) {
            state.ready = false;
            state.retry_at = Some(state.retry_at.unwrap_or(SystemTime::UNIX_EPOCH));
        }
        state
    }

    /// Runs `f` on the index; `None` when there is none (not downloaded).
    fn with_index<T>(&self, paths: &DataPaths, f: impl FnOnce(&Index) -> Result<T>) -> Option<Result<T>> {
        let path = Self::files(paths).index();
        let meta = std::fs::metadata(&path).ok()?;
        let stamp = (meta.modified().ok(), meta.len());
        let mut open = self.open.lock();
        if open.as_ref().is_none_or(|o| o.stamp != stamp) {
            *open = None;
            match Index::open(&path) {
                Ok(index) => *open = Some(Opened { stamp, index }),
                Err(e) => return Some(Err(e)),
            }
        }
        open.as_ref().map(|o| f(&o.index))
    }

    /// The game of the database `game` is, and how it was found.
    /// [`Lookup::Skipped`] without a database; [`Lookup::NotFound`] for an
    /// emulated game of a platform LaunchBox does not have.
    pub fn lookup(&self, paths: &DataPaths, game: &SourceGame) -> Lookup<(LbGame, MatchedBy)> {
        let found = self.with_index(paths, |index| {
            let Some((id, by)) = find(index, game)? else { return Ok(None) };
            Ok(index.game(id)?.map(|g| (g, by)))
        });
        answer(found, &game.title)
    }

    /// Its game `id` (a DatabaseID).
    pub fn by_id(&self, paths: &DataPaths, id: &str) -> Lookup<LbGame> {
        let Ok(id) = id.trim().parse::<i64>() else { return Lookup::NotFound };
        answer(self.with_index(paths, |index| index.game(id)), &id.to_string())
    }

    /// Games of any platform whose name is like `query`, best first: those
    /// named so, then those whose name starts so, then those whose name holds
    /// it (each by how close the name is, then by votes). Digits are a name
    /// too, and the game with that DatabaseID comes right after those named
    /// so ("1942").
    pub fn search(&self, paths: &DataPaths, query: &str) -> Lookup<Vec<LbGame>> {
        let query = query.trim();
        if query.is_empty() {
            return Lookup::Found(Vec::new());
        }
        let found = self.with_index(paths, |index| {
            let mut ranked: Vec<(u8, i64)> = Vec::new();
            let mut add = |tier: u8, ids: Vec<i64>| {
                for id in ids {
                    if !ranked.iter().any(|(_, g)| *g == id) {
                        ranked.push((tier, id));
                    }
                }
            };
            let key = match_key(query);
            add(0, index.named(&key, SEARCH_SCAN)?);
            if let Some(id) = query.parse::<i64>().ok().filter(|_| query.bytes().all(|b| b.is_ascii_digit())) {
                add(1, index.game(id)?.map(|g| g.id).into_iter().collect());
            }
            add(2, index.starting(&key, SEARCH_SCAN)?);
            add(3, index.containing(&key, SEARCH_SCAN)?);
            let mut games: Vec<(u8, LbGame)> =
                ranked.into_iter().filter_map(|(tier, id)| Some((tier, index.game(id).ok().flatten()?))).collect();
            games.sort_by_key(|(tier, g)| (*tier, std::cmp::Reverse(score_title(query, &g.name)), std::cmp::Reverse(g.detail.votes), g.id));
            Ok(games.into_iter().take(SEARCH_RESULTS).map(|(_, g)| g).collect())
        });
        match found {
            None => Lookup::Skipped,
            Some(Ok(games)) => Lookup::Found(games),
            Some(Err(e)) => {
                log::warn!("LaunchBox search of {query} failed: {e:#}");
                Lookup::NotFound
            }
        }
    }

    /// The game a search by hand for `query` finds first ([`Self::search`]),
    /// on the platform of `game` when one of its games is there.
    pub fn search_for(&self, paths: &DataPaths, game: &SourceGame, query: &str) -> Option<LbGame> {
        let games = self.search(paths, query).into_option()?;
        let own = platform_names(game);
        let first = games.iter().position(|g| own.contains(&g.platform.as_str())).unwrap_or(0);
        games.into_iter().nth(first)
    }

    /// Downloads the archive (only when it changed since the index in use
    /// was built), builds a new index and swaps it in. The old index stays
    /// until then: a failed or cancelled download changes nothing, but a
    /// failure is recorded, and the idle maintenance waits before it tries
    /// again ([`fetch::Stored::retry_at`]). Blocking.
    pub fn download(&self, paths: &DataPaths, http: &Http, progress: &DownloadProgress) -> Result<()> {
        let files = Self::files(paths);
        let result = self.refresh(&files, http, progress);
        if let Err(e) = &result {
            if !is_cancelled(e) {
                files.note_failure();
            }
        }
        result
    }

    fn refresh(&self, files: &Files, http: &Http, progress: &DownloadProgress) -> Result<()> {
        // An index that no longer opens is built again, whatever the archive's ETag says.
        let current = files.stored().filter(|_| Index::open(&files.index()).is_ok());
        let url = format!("{}/{ARCHIVE}", http.endpoints.launchbox.trim_end_matches('/'));
        let (etag, last_modified) = match fetch_archive(files, http, &url, current.as_ref().and_then(|s| s.etag.as_deref()), progress)? {
            Archive::Unchanged => {
                let stored = Stored { refreshed_at: now_secs(), failures: 0, failed_at: None, ..current.unwrap_or_default() };
                return files.store(&stored);
            }
            Archive::Whole { etag, last_modified } => (etag, last_modified),
        };
        let part = files.index_part();
        let built = build(&files.archive_part(), &part, progress).and_then(|built| {
            // Only an index that opens and has games replaces the one in use.
            let games = Index::open(&part)?.game_count()?;
            anyhow::ensure!(games > 0, "the new LaunchBox index has no games");
            Ok(built)
        });
        let built = match built {
            Ok(built) => built,
            Err(e) => {
                let _ = std::fs::remove_file(&part);
                // An archive that does not read is downloaded again; one
                // whose index could not be written (a full disk) or that
                // was cancelled is kept for the next try.
                if e.is::<BadArchive>() {
                    drop_archive(files);
                }
                return Err(e);
            }
        };
        {
            let mut open = self.open.lock();
            *open = None;
            // Windows refuses for a moment while an antivirus scans the
            // new ~100 MB file: tried again there.
            heronite_platform::fs::rename_replacing(&part, &files.index())?;
        }
        files.store(&Stored { refreshed_at: now_secs(), etag, last_modified, games: built.games, ..Stored::default() })?;
        drop_archive(files);
        log::info!(
            "LaunchBox Games Database: {} games, {} names, {} files, {} pictures, {} records skipped",
            built.games,
            built.names,
            built.files,
            built.images,
            built.skipped
        );
        Ok(())
    }

    /// The size of the picture `file` (under `images_base`), read from its
    /// first bytes ([`fetch_size`]: the server ignores `Range`, the rest of
    /// the answer is dropped) and remembered. `None` when it cannot be
    /// read: no answer (asked again next time), or not a picture.
    pub fn picture_size(&self, http: &Http, images_base: &str, file: &str) -> Option<(u32, u32)> {
        if let Some(known) = self.sizes.lock().get(file) {
            return *known;
        }
        let size = match fetch_size(http, &map::image_url(images_base, file)) {
            Ok(size) => size,
            Err(e) => {
                log::info!("LaunchBox picture {file} not read: {e:#}");
                return None;
            }
        };
        let mut sizes = self.sizes.lock();
        if sizes.len() >= SIZES_CAP {
            sizes.clear();
        }
        sizes.insert(file.to_owned(), size);
        size
    }

    /// Removes the index and whatever a download left: the source needs a
    /// download again.
    pub fn remove(&self, paths: &DataPaths) -> Result<()> {
        let mut open = self.open.lock();
        *open = None;
        Self::files(paths).remove_all()
    }
}

/// A lookup's answer: no index is "not asked", a broken one "not found".
fn answer<T>(found: Option<Result<Option<T>>>, what: &str) -> Lookup<T> {
    match found {
        None => Lookup::Skipped,
        Some(Ok(Some(t))) => Lookup::Found(t),
        Some(Ok(None)) => Lookup::NotFound,
        Some(Err(e)) => {
            log::warn!("LaunchBox lookup of {what} failed: {e:#}");
            Lookup::NotFound
        }
    }
}

/// LaunchBox's platforms a game is looked up on: an emulated game's own
/// (none when its platform is unknown), a PC game's PC platforms.
fn platform_names(game: &SourceGame) -> Vec<&'static str> {
    if !game.emulated {
        return LAUNCHBOX_PC.to_vec();
    }
    game.platform.as_deref().and_then(platforms::by_id).map(|p| p.names_in(Database::LaunchBox).to_vec()).unwrap_or_default()
}

/// The game among `hits`: own names before alternate names, then the
/// platform first in `order`; and whether it is the only game of that kind
/// of name on all of `order`'s platforms (a game of the same name on the
/// platform's other LaunchBox platform, as the Famicom Disk System's is on
/// the NES list, makes it a guess).
fn choose(hits: &[NameHit], order: &[i64]) -> Option<(i64, bool)> {
    let rank = |h: &NameHit| (h.alt, order.iter().position(|p| *p == h.platform).unwrap_or(order.len()));
    let best = hits.iter().min_by_key(|h| (rank(h), h.game))?;
    let alone = !hits.iter().any(|h| h.game != best.game && h.alt == best.alt);
    Some((best.game, alone))
}

/// The game named exactly `key` on `platforms`: sure when it is the only one.
fn exact(index: &Index, platforms: &[i64], key: &str) -> Result<Option<(i64, MatchedBy)>> {
    let hits = index.by_key(platforms, key)?;
    Ok(choose(&hits, platforms).map(|(game, alone)| (game, if alone { MatchedBy::ExactTitle } else { MatchedBy::FuzzyTitle })))
}

/// How `game` is found in the index (see the module docs).
fn find(index: &Index, game: &SourceGame) -> Result<Option<(i64, MatchedBy)>> {
    let platforms = index.platform_ids(&platform_names(game))?;
    if platforms.is_empty() {
        return Ok(None);
    }
    if !game.emulated {
        if let Some(steam) = game.steam_app_id.as_deref().and_then(|s| s.trim().parse::<i64>().ok()) {
            if let [only] = index.by_steam_id(&platforms, steam)?.as_slice() {
                return Ok(Some((*only, MatchedBy::SteamId)));
            }
        }
    }
    let rom = game.rom_file.as_deref().filter(|_| game.emulated).filter(|f| !f.trim().is_empty());
    let mut names: Vec<&str> = rom.into_iter().collect();
    names.push(&game.title);
    if let Some(file) = rom {
        if let Some(key) = index.file_key(&platforms, strip_extension(file))? {
            if let Some(found) = exact(index, &platforms, &key)? {
                return Ok(Some(found));
            }
        }
    }
    for name in &names {
        if let Some(found) = exact(index, &platforms, &match_key(name))? {
            return Ok(Some(found));
        }
    }
    // Looser: a longer name of LaunchBox's starting with the whole name
    // (`Metal Gear Solid 2` → `… 2: Sons of Liberty`), else the name without
    // its last words (`Gran Turismo 4 Spec II` → `Gran Turismo 4`).
    for name in &names {
        let key = match_key(name);
        if key.chars().count() >= MIN_LOOSE_KEY {
            let hits = index.by_prefix(&platforms, &key)?;
            if let Some((game, _)) = choose(&hits, &platforms) {
                return Ok(Some((game, MatchedBy::FuzzyTitle)));
            }
        }
    }
    for name in &names {
        for key in shorter_keys(name).iter().filter(|k| k.chars().count() >= MIN_LOOSE_KEY) {
            if let Some((game, _)) = choose(&index.by_key(&platforms, key)?, &platforms) {
                return Ok(Some((game, MatchedBy::FuzzyTitle)));
            }
        }
    }
    Ok(None)
}
