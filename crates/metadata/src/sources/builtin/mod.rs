//! The built-in sources: their descriptors, in the order the merge lists
//! candidates, and thin adapters over the provider modules.

use std::sync::{Arc, LazyLock};

use heronite_data::models::CollectionGameMetadata;
use heronite_data::settings::METADATA_FIELDS;
use heronite_data::source_prefs::ids::{HOW_LONG_TO_BEAT, PROTONDB, STEAMGRIDDB, STEAM_TAGS, WEB_IMAGES};
use heronite_i18n::n_;
use parking_lot::RwLock;

use super::descriptor::extra_fields::{LINUX_RATING, STEAM_TAGS as STEAM_TAGS_FIELD, TIME_TO_BEAT};
use super::download::SourceDownload;
use super::options::{OptionChoice, OptionKind, OptionValue, SourceOption};
use super::{
    AlsoFor, AppliesTo, Asked, Entry, MetadataSource, Needs, SourceContext, SourceDescriptor, SourceGame, SourceKind, SourceRegistry,
    SourceTest,
};
use crate::covers::{find_steamgrid_game, search_steam, steamgrid_api_key, steamgrid_pictures};
use crate::igdb::{Igdb, IgdbCredentials};
use crate::launchbox::{self, map as lb_map, LaunchBox};
use crate::libretro;
use crate::lutris::{fetch_lutris_metadata, search_lutris};
use crate::providers::{fetch_playnite_metadata, metadata_from_store_game, PlayniteSource, StoreLibraries};
use crate::steam::{self, SteamState};
use crate::types::{ArtKind, Lookup, MatchedBy, MetadataCandidate, SourceHit};
use launchbox_source::{region_label, LaunchBoxSource};
use libretro_source::LibretroSource;

const IGDB: &str = "igdb";
const STEAM: &str = "steam";
const STORE: &str = "store";
const LUTRIS: &str = "lutris";
const PLAYNITE: &str = "playnite";
const LAUNCHBOX: &str = launchbox::SOURCE;
const LIBRETRO: &str = libretro::SOURCE;

mod launchbox_source;
mod libretro_source;
/// LaunchBox's options.
const BOX_REGION: &str = "boxRegion";
const PC_GAMES: &str = "pcGames";

/// The service's state the adapters share with its other entry points
/// (IGDB token and queue, Steam caches, parsed store libraries, Playnite).
pub(crate) struct Shared {
    pub igdb: Arc<Igdb>,
    pub steam: Arc<SteamState>,
    pub stores: Arc<StoreLibraries>,
    pub playnite: Arc<RwLock<Option<Arc<dyn PlayniteSource>>>>,
    pub launchbox: Arc<LaunchBox>,
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
        asked: Asked::Always,
        game_kind: None,
        about: None,
        download: None,
        also_for: None,
        leads_for: None,
        extension: None,
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
                    "coverWide",
                ],
            )
        },
        SourceDescriptor {
            finds_by: vec![Game],
            network: false,
            ..source(STORE, n_!("Store"), Details, &["description", "releaseDate", "developers", "genres", "cover", "hero", "coverWide"])
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
        // Asked from a database downloaded once (about 105 MB), never before
        // the user asks for it. For emulated games: off for PC games unless
        // the user turns that on (Steam and IGDB know them better, and
        // LaunchBox's Windows list has fan ports named like the console
        // games). An emulated game it finds by platform and dump name goes
        // ahead of the sources that found it only by title.
        SourceDescriptor {
            applies_to: AppliesTo::Emulated,
            finds_by: vec![SourceId, SteamId, ExactTitle, FuzzyTitle],
            needs: Needs::Download,
            network: false,
            searchable: true,
            download: Some(SourceDownload { size_mb: 105, refresh_days: 7 }),
            also_for: Some(AlsoFor { option: PC_GAMES.into(), applies_to: AppliesTo::Pc }),
            leads_for: Some(AppliesTo::Emulated),
            about: Some(n_!(
                "The LaunchBox Games Database (gamesdb.launchbox-app.com), made for emulated games: it knows each platform's games by their dump names, with box art by region. Downloaded once to this PC and refreshed every week while it is on. Heronite is not affiliated with LaunchBox."
            )
            .into()),
            options: vec![
                SourceOption {
                    choices: std::iter::once(OptionChoice { value: lb_map::AUTO_REGION.into(), label: n_!("Automatic").into() })
                        .chain(lb_map::REGIONS.iter().map(|r| OptionChoice { value: (*r).into(), label: region_label(r).into() }))
                        .collect(),
                    help: Some(n_!("Automatic follows the interface language. Box art made for every region comes next.").into()),
                    ..option(BOX_REGION, n_!("Box art region"), OptionKind::Select)
                },
                SourceOption {
                    help: Some(n_!("Look up games that are not emulated in its Windows and DOS lists too.").into()),
                    ..option(PC_GAMES, n_!("Also for PC games"), OptionKind::Switch)
                },
            ],
            ..source(
                LAUNCHBOX,
                "LaunchBox",
                Details,
                &[
                    "description",
                    "releaseDate",
                    "developers",
                    "publishers",
                    "genres",
                    "gameModes",
                    "platforms",
                    "criticScore",
                    "cover",
                    "hero",
                    "coverSquare",
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
            ..source(STEAMGRIDDB, "SteamGridDB", Pictures, &["cover", "hero", "coverSquare", "coverWide"])
        },
        // Box art by the dump's own file name: for the emulated games of the
        // platforms that have a folder on the server. It goes ahead of the
        // sources that found such a game only by its title. It has no
        // background: its screenshots are 240 to 512 px wide.
        SourceDescriptor {
            applies_to: AppliesTo::Platforms(libretro_platforms()),
            finds_by: vec![ExactTitle, FuzzyTitle],
            leads_for: Some(AppliesTo::Emulated),
            about: Some(n_!(
                "The box art RetroArch uses (thumbnails.libretro.com), found by each game's dump file name on its platform. No key or account. Box art keeps its own shape: landscape boxes fill the wide cover and CD cases the square one. Heronite is not affiliated with libretro."
            )
            .into()),
            ..source(LIBRETRO, "Libretro", Pictures, &["cover", "coverSquare", "coverWide"])
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

/// The platforms that have a system folder on libretro's server.
fn libretro_platforms() -> Vec<String> {
    crate::platforms::PLATFORMS
        .iter()
        .filter(|p| !p.names_in(crate::platforms::Database::Libretro).is_empty())
        .map(|p| p.id.to_owned())
        .collect()
}

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
            IGDB => Entry::Lookup(Arc::new(IgdbSource { d, igdb: shared.igdb.clone() })),
            STEAM => Entry::Lookup(Arc::new(SteamSource { d, state: shared.steam.clone() })),
            STORE => Entry::Lookup(Arc::new(StoreSource { d, stores: shared.stores.clone() })),
            LUTRIS => Entry::Lookup(Arc::new(LutrisSource { d })),
            PLAYNITE => Entry::Lookup(Arc::new(PlayniteDump { d, source: shared.playnite.clone() })),
            LAUNCHBOX => Entry::Lookup(Arc::new(LaunchBoxSource { d, db: shared.launchbox.clone() })),
            STEAMGRIDDB => Entry::Lookup(Arc::new(SteamGridSource { d })),
            LIBRETRO => Entry::Lookup(Arc::new(LibretroSource { d, client: libretro::Libretro::new() })),
            _ => Entry::DescriptorOnly(Arc::new(d.clone())),
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

/// SteamGridDB as a pictures source: the game by its Steam id or its title,
/// then the first picture it ranks of each field asked (the covers by
/// dimension, see [`crate::covers`]). It is asked only for the fields it is
/// in the order of, so a fetch with the default orders asks it for the
/// square and wide covers only when those are kept.
struct SteamGridSource {
    d: &'static SourceDescriptor,
}

impl MetadataSource for SteamGridSource {
    fn descriptor(&self) -> &SourceDescriptor {
        self.d
    }

    fn lookup(&self, ctx: &SourceContext<'_>, game: &SourceGame) -> Lookup<MetadataCandidate> {
        let Some(key) = steamgrid_api_key(ctx.paths) else { return Lookup::Skipped };
        let wanted = |f: &String| game.fields.as_ref().is_none_or(|list| list.contains(f));
        let kinds: Vec<ArtKind> = self.d.provides.iter().filter(|f| wanted(f)).filter_map(|f| ArtKind::from_field(f)).collect();
        if kinds.is_empty() {
            return Lookup::Skipped;
        }
        let found = match find_steamgrid_game(ctx.http, &key, game.steam_app_id.as_deref(), &game.title) {
            Lookup::Found(found) => found,
            Lookup::NotFound => return Lookup::NotFound,
            Lookup::Unreachable => return Lookup::Unreachable,
            Lookup::Skipped => return Lookup::Skipped,
        };
        steamgrid_pictures(ctx.http, &key, found.id, &kinds).map(|pictures| {
            // No title: the record keeps the one the details sources give.
            let mut md = crate::util::blank_metadata(&game.app_name, &game.runner);
            for (kind, url) in pictures {
                *kind.of_record_mut(&mut md) = Some(url);
            }
            MetadataCandidate {
                source: self.d.id.clone(),
                match_name: found.name.clone(),
                match_year: found.year,
                matched_by: Some(found.matched_by),
                match_id: Some(found.id.to_string()),
                metadata: md,
                adult_pictures: Vec::new(),
            }
        })
    }
}
