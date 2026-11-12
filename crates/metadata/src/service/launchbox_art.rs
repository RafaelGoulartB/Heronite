//! LaunchBox's pictures of one game for the Covers dialog: every region's
//! box art (those of the slot's shape first) and its background, beside
//! what SteamGridDB and the web found. The game's real boxes, which a fetch
//! may not have picked for every shape.

use anyhow::Result;
use heronite_data::source_prefs::is_on;

use super::MetadataService;
use crate::launchbox::{self, index::LbGame, map as lb_map};
use crate::record;
use crate::rom_names::match_key;
use crate::sources::{box_regions, SourceContext};
use crate::store::get_game_metadata;
use crate::types::{ArtImages, ArtKind, MetadataApplyArgs, RegionImage};

/// Pictures listed at most; each costs a small request for its size.
const MAX_PICTURES: usize = 24;
/// Sizes read at once.
const AT_ONCE: usize = 6;

impl MetadataService {
    /// The Covers dialog's search for `game`: [`Self::search_art_images`] of
    /// `query` and, apart, LaunchBox's pictures of the game for `typed`
    /// (the words in the search field: [`Self::launchbox_pictures`]). A
    /// failing image search only leaves its part empty when LaunchBox has
    /// pictures.
    pub fn search_game_art_images(&self, game: &MetadataApplyArgs, typed: &str, query: &str, kind: ArtKind) -> Result<ArtImages> {
        let (launchbox, rest) = std::thread::scope(|s| {
            let launchbox = s.spawn(|| self.launchbox_pictures(game, typed, kind));
            let rest = self.search_art_images(query, kind, false);
            (launchbox.join().unwrap_or_default(), rest)
        });
        let mut images = match rest {
            Ok(images) => images,
            Err(e) if launchbox.is_empty() => return Err(e),
            Err(_) => ArtImages::default(),
        };
        images.launchbox = launchbox;
        Ok(images)
    }

    /// LaunchBox's pictures of `game` for a picture of `kind`
    /// ([`lb_map::choices`]), with their sizes. Only while LaunchBox is on,
    /// its database is downloaded and it is asked for this game. The game is
    /// the one the record matched on LaunchBox, else the one a lookup finds;
    /// with other words than the title in `typed`, the first game of that
    /// name (on the game's platform when one is there).
    pub fn launchbox_pictures(&self, game: &MetadataApplyArgs, typed: &str, kind: ArtKind) -> Vec<RegionImage> {
        let sources = self.sources();
        let settings = self.metadata_settings();
        let Some(d) = sources.get(launchbox::SOURCE) else { return Vec::new() };
        if !is_on(&settings, &d.id) || !self.launchbox.state(&self.paths).ready {
            return Vec::new();
        }
        let current = get_game_metadata(&self.paths, &game.runner, &game.app_name).ok().flatten();
        let view = self.source_game(&sources, game, current.as_ref());
        if !sources.applies(&self.paths, &settings, d, &view) {
            return Vec::new();
        }
        let typed = typed.trim();
        let found: Option<LbGame> = if !typed.is_empty() && match_key(typed) != match_key(&game.title) {
            self.launchbox.search_for(&self.paths, &view, typed)
        } else {
            current
                .as_ref()
                .and_then(|md| record::source_match(md, launchbox::SOURCE))
                .and_then(|m| self.launchbox.by_id(&self.paths, &m.id).into_option())
                .or_else(|| self.launchbox.lookup(&self.paths, &view).into_option().map(|(lb, _)| lb))
        };
        let Some(lb) = found else { return Vec::new() };
        let ctx = SourceContext::new(&self.paths, &self.http);
        let regions = box_regions(d, &ctx);
        let base = &self.http.endpoints.launchbox_images;
        let files: Vec<String> = lb_map::pictures_for(&lb, kind).into_iter().take(MAX_PICTURES).map(|i| i.file.clone()).collect();
        // Only the pictures listed (and sized): another kind's stay.
        let mut lb = lb;
        let listed = lb_map::pictures_for(&lb, kind).into_iter().map(|i| i.file.clone()).collect::<Vec<_>>();
        lb.images.retain(|i| files.contains(&i.file) || !listed.contains(&i.file));
        let mut sizes: Vec<(String, Option<(u32, u32)>)> = Vec::new();
        for batch in files.chunks(AT_ONCE) {
            std::thread::scope(|s| {
                let reads: Vec<_> = batch.iter().map(|f| (f, s.spawn(|| self.launchbox.picture_size(&self.http, base, f)))).collect();
                sizes.extend(reads.into_iter().map(|(f, r)| (f.clone(), r.join().ok().flatten())));
            });
        }
        let size_of = |img: &launchbox::index::LbImage| sizes.iter().find(|(f, _)| *f == img.file).and_then(|(_, s)| *s);
        lb_map::choices(&lb, kind, &regions, base, size_of)
    }
}
