//! The built-in sources: their descriptors, in the order the merge lists
//! candidates, and thin adapters over the provider modules.

use std::sync::{Arc, LazyLock};

use heronite_data::models::CollectionGameMetadata;
use heronite_data::settings::METADATA_FIELDS;
use heronite_data::source_prefs::ids::{HOW_LONG_TO_BEAT, PROTONDB, STEAMGRIDDB, STEAM_TAGS, WEB_IMAGES};
use heronite_i18n::n_;
use parking_lot::RwLock;

use super::descriptor::extra_fields::{LINUX_RATING, STEAM_TAGS as STEAM_TAGS_FIELD, TIME_TO_BEAT};
use super::options::{OptionChoice, OptionKind, OptionValue, SourceOption};
use super::{AppliesTo, Entry, MetadataSource, Needs, SourceContext, SourceDescriptor, SourceGame, SourceKind, SourceRegistry, SourceTest};
use crate::covers::search_steam;
use crate::igdb::{Igdb, IgdbCredentials};
use crate::lutris::{fetch_lutris_metadata, search_lutris};
use crate::providers::{fetch_playnite_metadata, metadata_from_store_game, PlayniteSource, StoreLibraries};
use crate::steam::{self, SteamState};
use crate::types::{Lookup, MatchedBy, MetadataCandidate, SourceHit};

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
        manual_only: false,
        searchable: false,
        only_on: Vec::new(),
        options: Vec::new(),
    }
}

fn option(key: &str, label: &str, kind: OptionKind) -> SourceOption {
    SourceOption { key: key.into(), label: label.into(), kind, choices: Vec::new(), default: None, help: None, link: None, required: false }
}

/// A key the source cannot be asked without.
fn key(key: &str, label: &str, kind: OptionKind, help: Option<&str>, link: Option<&str>) -> SourceOption {
    SourceOption { help: help.map(Into::into), link: link.map(Into::into), required: true, ..option(key, label, kind) }
}

static BUILTIN: LazyLock<Vec<SourceDescriptor>> = LazyLock::new(|| {
    use MatchedBy::*;
    use SourceKind::*;
    vec![
        SourceDescriptor {
            finds_by: vec![SourceId, SteamId, ExactTitle, FuzzyTitle],
            needs: Needs::ApiKey,
            searchable: true,
            options: vec![
                key(
                    "clientId",
                    n_!("Client ID"),
                    OptionKind::Text,
                    Some(n_!("Create a Twitch app to get IGDB keys: dev.twitch.tv/console")),
                    Some("https://dev.twitch.tv/console"),
                ),
                key("clientSecret", n_!("Client secret"), OptionKind::Secret, None, None),
            ],
            ..source(IGDB, "IGDB", Details, &METADATA_FIELDS)
        },
        SourceDescriptor {
            applies_to: AppliesTo::HasSteamId,
            finds_by: vec![SteamId],
            searchable: true,
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
            searchable: true,
            ..source(LUTRIS, "Lutris", Details, &["description", "releaseDate", "genres", "platforms", "cover", "hero"])
        },
        // Kept in saved priorities (Heronite's data) but not offered: nothing
        // feeds this source here yet (no Playnite dump is set).
        SourceDescriptor {
            finds_by: vec![SourceId],
            network: false,
            hidden: true,
            only_on: strings(&["windows"]),
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
            options: vec![key(
                "apiKey",
                n_!("SteamGridDB API key"),
                OptionKind::Secret,
                Some(n_!("Optional. Get a key at steamgriddb.com/profile/preferences/api")),
                Some("https://www.steamgriddb.com/profile/preferences/api"),
            )],
            ..source(STEAMGRIDDB, "SteamGridDB", Pictures, &["cover", "hero"])
        },
        SourceDescriptor {
            finds_by: vec![FuzzyTitle],
            manual_only: true,
            ..source(WEB_IMAGES, n_!("From the web"), Pictures, &["cover", "hero"])
        },
        SourceDescriptor {
            finds_by: vec![SourceId, ExactTitle, FuzzyTitle],
            ..source(HOW_LONG_TO_BEAT, "HowLongToBeat", Extras, &[TIME_TO_BEAT])
        },
        SourceDescriptor {
            applies_to: AppliesTo::HasSteamId,
            finds_by: vec![SteamId],
            options: vec![SourceOption {
                choices: [5, 10, 15, 20].iter().map(|n| OptionChoice { value: n.to_string(), label: n.to_string() }).collect(),
                default: Some(OptionValue::Text(heronite_data::steam_tags::DEFAULT_LIMIT.to_string())),
                ..option("limit", n_!("Steam tags per game"), OptionKind::Select)
            }],
            ..source(STEAM_TAGS, n_!("Steam tags"), Extras, &[STEAM_TAGS_FIELD])
        },
        SourceDescriptor { finds_by: vec![SteamId, ExactTitle], ..source(PROTONDB, "ProtonDB", Extras, &[LINUX_RATING]) },
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
        let creds = igdb_credentials(ctx);
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
            igdb_candidate(self.d, matched_by, md)
        })
    }

    fn lookup_id(&self, ctx: &SourceContext<'_>, game: &SourceGame, id: &str) -> Lookup<MetadataCandidate> {
        let Ok(id) = id.trim().parse::<i64>() else { return Lookup::NotFound };
        self.igdb
            .fetch_id(ctx.http, &igdb_credentials(ctx), &game.app_name, &game.runner, id)
            .map(|md| igdb_candidate(self.d, MatchedBy::SourceId, md))
    }

    fn search(&self, ctx: &SourceContext<'_>, query: &str) -> Lookup<Vec<SourceHit>> {
        self.igdb.search_hits(ctx.http, &igdb_credentials(ctx), query)
    }

    fn has_test(&self) -> bool {
        true
    }

    /// Asks Twitch for a token and IGDB for one game with the saved keys.
    fn test(&self, ctx: &SourceContext<'_>) -> Option<SourceTest> {
        Some(self.igdb.test_credentials(ctx.http, &igdb_credentials(ctx)))
    }
}

fn igdb_credentials(ctx: &SourceContext<'_>) -> IgdbCredentials {
    let m = ctx.settings();
    IgdbCredentials::new(&m.igdb_client_id, &m.igdb_client_secret)
}

fn igdb_candidate(d: &SourceDescriptor, matched_by: MatchedBy, md: CollectionGameMetadata) -> MetadataCandidate {
    let id = md.igdb_id.map(|n| (n.get() as i64).to_string());
    candidate(d, matched_by)(md).with_id(id)
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
        self.app(ctx, game, game.steam_app_id.as_deref(), MatchedBy::SteamId)
    }

    fn lookup_id(&self, ctx: &SourceContext<'_>, game: &SourceGame, id: &str) -> Lookup<MetadataCandidate> {
        self.app(ctx, game, Some(id.trim()), MatchedBy::SourceId)
    }

    fn search(&self, ctx: &SourceContext<'_>, query: &str) -> Lookup<Vec<SourceHit>> {
        search_steam(ctx.http, query)
    }
}

impl SteamSource {
    fn app(&self, ctx: &SourceContext<'_>, game: &SourceGame, id: Option<&str>, matched_by: MatchedBy) -> Lookup<MetadataCandidate> {
        steam::fetch_steam_metadata(ctx.paths, ctx.http, &self.state, &game.app_name, &game.runner, id, game.language.as_deref()).map(
            |md| {
                let id = md.steam_app_id.clone();
                candidate(self.d, matched_by)(md).with_id(id)
            },
        )
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
            self.candidate(matched_by, md)
        })
    }

    fn lookup_id(&self, ctx: &SourceContext<'_>, game: &SourceGame, id: &str) -> Lookup<MetadataCandidate> {
        let slug = id.trim();
        if slug.is_empty() {
            return Lookup::NotFound;
        }
        fetch_lutris_metadata(ctx.http, &game.app_name, &game.runner, &game.title, Some(slug))
            .map(|md| self.candidate(MatchedBy::SourceId, md))
    }

    fn search(&self, ctx: &SourceContext<'_>, query: &str) -> Lookup<Vec<SourceHit>> {
        search_lutris(ctx.http, query)
    }
}

impl LutrisSource {
    fn candidate(&self, matched_by: MatchedBy, md: CollectionGameMetadata) -> MetadataCandidate {
        let id = md.lutris_slug.clone();
        candidate(self.d, matched_by)(md).with_id(id)
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
