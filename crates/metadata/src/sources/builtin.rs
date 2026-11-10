//! The built-in sources: their descriptors, in the order the merge lists
//! candidates, and thin adapters over the provider modules.

use std::sync::{Arc, LazyLock};

use heronite_data::models::CollectionGameMetadata;
use heronite_data::settings::METADATA_FIELDS;
use heronite_i18n::n_;
use parking_lot::RwLock;

use super::descriptor::extra_fields::{LINUX_RATING, STEAM_TAGS, TIME_TO_BEAT};
use super::{AppliesTo, Entry, MetadataSource, Needs, SourceContext, SourceDescriptor, SourceGame, SourceKind, SourceRegistry};
use crate::igdb::{Igdb, IgdbCredentials};
use crate::lutris::fetch_lutris_metadata;
use crate::providers::{fetch_playnite_metadata, metadata_from_store_game, PlayniteSource, StoreLibraries};
use crate::steam::{self, SteamState};
use crate::types::{Lookup, MatchedBy, MetadataCandidate};

const IGDB: &str = "igdb";
const STEAM: &str = "steam";
const STORE: &str = "store";
const LUTRIS: &str = "lutris";
const PLAYNITE: &str = "playnite";

/// The service's state the adapters share with its other entry points
/// (IGDB token and queue, Steam caches, parsed store libraries, Playnite).
pub(crate) struct Shared {
    pub igdb: Arc<Igdb>,
    pub steam: Arc<SteamState>,
    pub stores: Arc<StoreLibraries>,
    pub playnite: Arc<RwLock<Option<Arc<dyn PlayniteSource>>>>,
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn source(id: &str, name: &str, kind: SourceKind, provides: &[&str]) -> SourceDescriptor {
    SourceDescriptor {
        id: id.into(),
        name: name.into(),
        kind,
        provides: strings(provides),
        applies_to: AppliesTo::All,
        finds_by: Vec::new(),
        needs: Needs::Nothing,
        network: true,
        hidden: false,
    }
}

static BUILTIN: LazyLock<Vec<SourceDescriptor>> = LazyLock::new(|| {
    use MatchedBy::*;
    use SourceKind::*;
    vec![
        SourceDescriptor {
            finds_by: vec![SourceId, SteamId, ExactTitle, FuzzyTitle],
            needs: Needs::ApiKey,
            ..source(IGDB, "IGDB", Details, &METADATA_FIELDS)
        },
        SourceDescriptor {
            applies_to: AppliesTo::HasSteamId,
            finds_by: vec![SteamId],
            ..source(
                STEAM,
                "Steam",
                Details,
                &[
                    "description",
                    "releaseDate",
                    "developers",
                    "publishers",
                    "genres",
                    "features",
                    "criticScore",
                    "ageRatings",
                    "cover",
                    "hero",
                ],
            )
        },
        SourceDescriptor {
            finds_by: vec![Game],
            network: false,
            ..source(STORE, n_!("Store"), Details, &["description", "releaseDate", "developers", "genres", "cover", "hero"])
        },
        SourceDescriptor {
            finds_by: vec![SourceId, ExactTitle, FuzzyTitle],
            ..source(LUTRIS, "Lutris", Details, &["description", "releaseDate", "genres", "platforms", "cover", "hero"])
        },
        // Kept in saved priorities (Heronite's data) but not offered: nothing
        // feeds this source here yet (no Playnite dump is set).
        SourceDescriptor {
            finds_by: vec![SourceId],
            network: false,
            hidden: true,
            ..source(
                PLAYNITE,
                "Playnite",
                Details,
                &[
                    "description",
                    "releaseDate",
                    "developers",
                    "publishers",
                    "genres",
                    "themes",
                    "platforms",
                    "series",
                    "features",
                    "cover",
                    "hero",
                ],
            )
        },
        SourceDescriptor {
            finds_by: vec![SteamId, FuzzyTitle],
            needs: Needs::ApiKey,
            ..source("steamgriddb", "SteamGridDB", Pictures, &["cover", "hero"])
        },
        SourceDescriptor { finds_by: vec![FuzzyTitle], ..source("webimages", n_!("From the web"), Pictures, &["cover", "hero"]) },
        SourceDescriptor {
            finds_by: vec![SourceId, ExactTitle, FuzzyTitle],
            ..source("howlongtobeat", "HowLongToBeat", Extras, &[TIME_TO_BEAT])
        },
        SourceDescriptor {
            applies_to: AppliesTo::HasSteamId,
            finds_by: vec![SteamId],
            ..source("steamtags", n_!("Steam tags"), Extras, &[STEAM_TAGS])
        },
        SourceDescriptor { finds_by: vec![SteamId, ExactTitle], ..source("protondb", "ProtonDB", Extras, &[LINUX_RATING]) },
    ]
});

/// Every built-in source, in registry order.
pub fn builtin_descriptors() -> &'static [SourceDescriptor] {
    &BUILTIN
}

/// The built-in sources of `kind` offered as a choice (not hidden), in order.
pub fn offered(kind: SourceKind) -> impl Iterator<Item = &'static SourceDescriptor> {
    BUILTIN.iter().filter(move |d| d.kind == kind && !d.hidden)
}

pub(super) fn registry(shared: Shared) -> SourceRegistry {
    let entries = BUILTIN
        .iter()
        .map(|d| match d.id.as_str() {
            IGDB => Entry::Lookup(Box::new(IgdbSource { d, igdb: shared.igdb.clone() })),
            STEAM => Entry::Lookup(Box::new(SteamSource { d, state: shared.steam.clone() })),
            STORE => Entry::Lookup(Box::new(StoreSource { d, stores: shared.stores.clone() })),
            LUTRIS => Entry::Lookup(Box::new(LutrisSource { d })),
            PLAYNITE => Entry::Lookup(Box::new(PlayniteDump { d, source: shared.playnite.clone() })),
            _ => Entry::DescriptorOnly(d.clone()),
        })
        .collect();
    SourceRegistry { entries }
}

/// Local sources have nothing to be unreachable about.
fn local(found: Option<CollectionGameMetadata>) -> Lookup<CollectionGameMetadata> {
    found.map_or(Lookup::NotFound, Lookup::Found)
}

fn candidate(d: &SourceDescriptor, matched_by: MatchedBy) -> impl FnOnce(CollectionGameMetadata) -> MetadataCandidate + '_ {
    move |metadata| MetadataCandidate::new(&d.id, metadata, Some(matched_by))
}

struct IgdbSource {
    d: &'static SourceDescriptor,
    igdb: Arc<Igdb>,
}

impl MetadataSource for IgdbSource {
    fn descriptor(&self) -> &SourceDescriptor {
        self.d
    }

    fn lookup(&self, ctx: &SourceContext<'_>, game: &SourceGame) -> Lookup<MetadataCandidate> {
        let m = ctx.settings();
        let creds = IgdbCredentials::new(&m.igdb_client_id, &m.igdb_client_secret);
        let steam_id = game.steam_app_id.as_deref();
        let found = self.igdb.fetch(ctx.http, &creds, &game.app_name, &game.runner, &game.title, steam_id, game.igdb_id);
        // `fetch` tries the IGDB id, then the Steam id, then the title: the
        // first that gives this game is how it was found.
        let pinned = game.igdb_id.filter(|i| *i != 0);
        found.map(|md| {
            let matched_by = if pinned.is_some() && md.igdb_id.map(|n| n.get() as i64) == pinned {
                MatchedBy::SourceId
            } else if steam_id.is_some() && md.steam_app_id.as_deref() == steam_id {
                MatchedBy::SteamId
            } else {
                MatchedBy::by_title(&game.title, md.title.as_deref())
            };
            candidate(self.d, matched_by)(md)
        })
    }
}

struct SteamSource {
    d: &'static SourceDescriptor,
    state: Arc<SteamState>,
}

impl MetadataSource for SteamSource {
    fn descriptor(&self) -> &SourceDescriptor {
        self.d
    }

    fn lookup(&self, ctx: &SourceContext<'_>, game: &SourceGame) -> Lookup<MetadataCandidate> {
        steam::fetch_steam_metadata(
            ctx.paths,
            ctx.http,
            &self.state,
            &game.app_name,
            &game.runner,
            game.steam_app_id.as_deref(),
            game.language.as_deref(),
        )
        .map(candidate(self.d, MatchedBy::SteamId))
    }
}

struct StoreSource {
    d: &'static SourceDescriptor,
    stores: Arc<StoreLibraries>,
}

impl MetadataSource for StoreSource {
    fn descriptor(&self) -> &SourceDescriptor {
        self.d
    }

    fn lookup(&self, ctx: &SourceContext<'_>, game: &SourceGame) -> Lookup<MetadataCandidate> {
        let (app, runner) = (game.app_name.as_str(), game.runner.as_str());
        local(self.stores.find(ctx.paths, runner, app).map(|g| metadata_from_store_game(app, runner, &g)))
            .map(candidate(self.d, MatchedBy::Game))
    }
}

struct LutrisSource {
    d: &'static SourceDescriptor,
}

impl MetadataSource for LutrisSource {
    fn descriptor(&self) -> &SourceDescriptor {
        self.d
    }

    fn lookup(&self, ctx: &SourceContext<'_>, game: &SourceGame) -> Lookup<MetadataCandidate> {
        let slug = game.lutris_slug.as_deref().filter(|s| !s.is_empty());
        fetch_lutris_metadata(ctx.http, &game.app_name, &game.runner, &game.title, slug).map(|md| {
            let matched_by = if slug.is_some() { MatchedBy::SourceId } else { MatchedBy::by_title(&game.title, md.title.as_deref()) };
            candidate(self.d, matched_by)(md)
        })
    }
}

struct PlayniteDump {
    d: &'static SourceDescriptor,
    source: Arc<RwLock<Option<Arc<dyn PlayniteSource>>>>,
}

impl MetadataSource for PlayniteDump {
    fn descriptor(&self) -> &SourceDescriptor {
        self.d
    }

    fn lookup(&self, ctx: &SourceContext<'_>, game: &SourceGame) -> Lookup<MetadataCandidate> {
        let src = self.source.read().clone();
        local(fetch_playnite_metadata(ctx.paths, src.as_deref(), &game.app_name, &game.runner)).map(candidate(self.d, MatchedBy::SourceId))
    }
}
