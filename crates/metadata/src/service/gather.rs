//! Asking the sources about one game ([`MetadataService::gather`]): which
//! are asked (on, chosen, for this game, and for a fallback source, needed),
//! all at once on their own threads, then the fallback sources the first
//! answers call for; the pictures flagged adult are dropped here unless the
//! user allows them.
//!
//! A fallback source ([`Asked::AsFallback`], an extension such as VNDB) is
//! asked with the others when the game is identified with it (a pinned or
//! stored match), when the request names it, or when the record or the game's Steam tags say the
//! game is of its kind ([`GameKind`]). Otherwise it waits for the others:
//! it is asked after them when none of the online details sources found
//! the game for sure, or when one of their candidates says the game is of
//! its kind. A game the others identify costs no request to it, and when
//! they could not be reached it is not asked either (the fetch is offline).

use std::collections::HashSet;
use std::sync::Arc;
use std::time::SystemTime;

use heronite_data::json_store;
use heronite_data::models::{CollectionGameMetadata, LocalMetaFile, MetadataSettings};
use heronite_data::steam_tags::SteamTagsFile;
use indexmap::IndexMap;

use super::fetch::{asks_details, is_picture_source};
use super::MetadataService;
use crate::record;
use crate::sources::{is_on, GameKind, MetadataSource, Needs, SourceContext, SourceDescriptor, SourceGame, SourceKind, SourceRegistry};
use crate::store::{as_runner, get_game_metadata};
use crate::types::*;

/// What the providers returned and whether the online ones answered at all.
pub(super) struct Gathered {
    pub(super) candidates: Vec<MetadataCandidate>,
    pub(super) offline: bool,
    /// Some source was asked (not every one off, left out or not for this game).
    pub(super) asked: bool,
    /// Every source with a lookup, in registry order (hidden ones only when
    /// they found the game).
    pub(super) states: Vec<SourceState>,
    /// The registry the sources were asked from: what is decided about
    /// their answers reads their kinds there, whatever was installed or
    /// removed meanwhile.
    pub(super) sources: Arc<SourceRegistry>,
    /// The online sources asked that did not answer (offline, or throttled).
    pub(super) unreachable: Vec<String>,
}

impl Gathered {
    /// Whether a source that could have answered a fetch of `fields` did
    /// not answer ([`missed`]).
    pub(super) fn missed(&self, fields: Option<&[String]>) -> bool {
        missed(&self.sources, &self.unreachable, fields)
    }
}

/// What a preview passes on of its gather to the fetch that saves it.
pub(super) struct Asked {
    /// Some source was asked.
    pub(super) any: bool,
    pub(super) sources: Arc<SourceRegistry>,
    pub(super) unreachable: Vec<String>,
    /// The decision uses some candidate.
    pub(super) used: bool,
}

impl Asked {
    /// See [`missed`].
    pub(super) fn missed(&self, fields: Option<&[String]>) -> bool {
        missed(&self.sources, &self.unreachable, fields)
    }
}

/// Whether one of `unreachable` could have answered a fetch of `fields`: a
/// details source for a fetch of the details ([`asks_details`]), any source
/// for one of pictures only. A pictures source that did not answer does not
/// keep a game from being marked as found in no source.
fn missed(sources: &SourceRegistry, unreachable: &[String], fields: Option<&[String]>) -> bool {
    let details = asks_details(fields);
    unreachable.iter().any(|id| !details || !is_picture_source(sources, id))
}

/// `store_cache/steam_tags.json` as last read, with its modification time.
pub(super) type SteamTagsCache = parking_lot::Mutex<Option<(Option<SystemTime>, Arc<SteamTagsFile>)>>;

/// The games of each source the lookups must use: the record's pinned
/// ones, changed by the arguments' `pins`.
fn pins(args: &MetadataApplyArgs, current: Option<&CollectionGameMetadata>) -> IndexMap<String, String> {
    let mut pins = current.map(record::pinned_ids).unwrap_or_default();
    for (source, pin) in args.pins.iter().flatten() {
        match pin.as_deref().map(str::trim).filter(|id| !id.is_empty()) {
            Some(id) => pins.insert(source.clone(), id.to_owned()),
            None => pins.shift_remove(source),
        };
    }
    pins
}

/// How one source is asked, or why it is not.
enum Plan<'a> {
    /// Its lookup of the game, or of this game of it.
    Ask(Option<&'a str>),
    /// A fallback source waiting for the others' answers.
    Later,
    Not(NotAsked),
}

/// Whether `record`'s genres or themes say it is a game of `kind`.
fn kind_in(kind: &GameKind, md: &CollectionGameMetadata) -> bool {
    kind.said_by(md.genres.iter().chain(&md.themes))
}

impl MetadataService {
    /// The fields of `fields` a pictures source is asked for: those it gives
    /// and is in the order of.
    fn picture_fields(sources: &SourceRegistry, d: &SourceDescriptor, fields: &[String], settings: &MetadataSettings) -> Vec<String> {
        fields.iter().filter(|f| d.provides(f) && sources.field_order(settings, f).contains(&d.id)).cloned().collect()
    }

    /// `game` as the sources see it. The overlay is read only when needed:
    /// for the Steam id the arguments lack, or for a source's audience (an
    /// emulated game's platform and ROM). `current`: its record, whose
    /// platforms are the last word on an emulated game's platform and which
    /// gives the release year.
    pub(super) fn source_game(
        &self,
        sources: &SourceRegistry,
        args: &MetadataApplyArgs,
        current: Option<&CollectionGameMetadata>,
    ) -> SourceGame {
        let has_steam_id = args.steam_app_id.as_deref().is_some_and(|s| !s.is_empty());
        let overlay = (!has_steam_id || sources.needs_overlay())
            .then(|| json_store::read::<LocalMetaFile>(&self.paths.local_meta()).ok())
            .flatten()
            .and_then(|mut file| file.games.shift_remove(&args.app_name));
        let mut game = SourceGame::new(args, overlay.as_ref(), current.map_or(&[], |md| md.platforms.as_slice()));
        game.year = current.and_then(|md| release_year(md.release_date.as_deref()));
        game
    }

    /// `gatherCandidates`: asks every provider that is on and applies to the
    /// game at once (IGDB calls queue among themselves) and keeps those that
    /// answered, in provider order.
    pub fn gather_candidates(&self, game: &MetadataApplyArgs) -> Vec<MetadataCandidate> {
        self.gather(game).candidates
    }

    /// The names of the Steam tags of `steam_app_id` in every language
    /// cached (no network; the file is read again only when it changed).
    fn steam_tag_names(&self, steam_app_id: &str) -> Vec<String> {
        let path = self.paths.steam_tags();
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let file = {
            let mut cache = self.steam_tags.lock();
            match cache.as_ref() {
                Some((at, file)) if *at == modified => file.clone(),
                _ => {
                    let file = Arc::new(SteamTagsFile::load(&self.paths));
                    *cache = Some((modified, file.clone()));
                    file
                }
            }
        };
        let Some(app) = file.apps.get(steam_app_id) else { return Vec::new() };
        app.tags.iter().flat_map(|t| file.names.values().filter_map(move |n| n.names.get(&t.tagid).cloned())).collect()
    }

    /// Asks the sources. A source with a pinned game is asked for that game
    /// (whatever its audience); IGDB and Lutris otherwise start from the id
    /// the record has, unless the arguments unpin them, and a fallback
    /// source from the match the record has.
    pub(super) fn gather(&self, game: &MetadataApplyArgs) -> Gathered {
        // One registry for the whole gather, whatever is installed meanwhile.
        let registry = self.sources();
        let sources = registry.as_ref();
        let current = get_game_metadata(&self.paths, as_runner(&game.runner), &game.app_name).ok().flatten();
        let pins = pins(game, current.as_ref());
        let unpinned = |source: &str| game.pins.as_ref().is_some_and(|p| p.get(source).is_some_and(Option::is_none));
        let mut with_ids = game.clone();
        with_ids.igdb_id =
            game.igdb_id.or_else(|| current.as_ref().and_then(|c| c.igdb_id).map(|n| n.get() as i64).filter(|_| !unpinned("igdb")));
        with_ids.lutris_slug =
            game.lutris_slug.clone().or_else(|| current.as_ref().and_then(|c| c.lutris_slug.clone()).filter(|_| !unpinned("lutris")));
        let game_view = self.source_game(sources, &with_ids, current.as_ref());
        let ctx = SourceContext::new(&self.paths, &self.http);
        let settings = ctx.settings();
        let fields = self.fetch_fields(game);
        // A pictures source sees the game with the fields asked of it.
        let views: Vec<Option<SourceGame>> = sources
            .lookups()
            .map(|source| {
                let d = source.descriptor();
                (d.kind == SourceKind::Pictures)
                    .then(|| SourceGame { fields: Some(Self::picture_fields(sources, d, &fields, settings)), ..game_view.clone() })
            })
            .collect();
        // The match a fallback source has in the record (not unpinned): it
        // is asked for that game again.
        let stored: IndexMap<String, String> = current
            .as_ref()
            .map(record::source_matches)
            .unwrap_or_default()
            .into_iter()
            .filter(|(source, _)| sources.get(source).is_some_and(SourceDescriptor::is_fallback) && !unpinned(source))
            .map(|(source, m)| (source, m.id))
            .collect();
        let mut steam_tags: Option<Vec<String>> = None;
        let plans: Vec<Plan<'_>> = sources
            .lookups()
            .zip(&views)
            .map(|(source, view)| {
                let d = source.descriptor();
                let game_view = view.as_ref().unwrap_or(&game_view);
                let pin = pins.get(&d.id).filter(|_| d.searchable).map(String::as_str);
                if !is_on(settings, &d.id) {
                    return Plan::Not(NotAsked::Off);
                }
                if !source.health().is_ok() {
                    return Plan::Not(NotAsked::Failing);
                }
                if !Self::wanted(d, game, game_view) {
                    return Plan::Not(NotAsked::NotChosen);
                }
                if pin.is_none() && !sources.applies(&self.paths, settings, d, game_view) {
                    return Plan::Not(NotAsked::NotForThisGame);
                }
                // Never asked, not even for a pinned game, before the user downloads its database.
                if !sources.downloaded(&self.paths, &d.id) {
                    return Plan::Not(NotAsked::NeedsDownload);
                }
                if !d.is_fallback() || pin.is_some() {
                    return Plan::Ask(pin);
                }
                if let Some(id) = stored.get(&d.id) {
                    return Plan::Ask(Some(id));
                }
                let of_kind = d.game_kind.as_ref().is_some_and(|kind| {
                    current.as_ref().is_some_and(|md| kind_in(kind, md))
                        || game_view
                            .steam_app_id
                            .as_deref()
                            .is_some_and(|id| kind.said_by(steam_tags.get_or_insert_with(|| self.steam_tag_names(id)).iter()))
                });
                if of_kind || sources.names_fallback(&d.id, game.only_sources.as_deref()) {
                    Plan::Ask(None)
                } else {
                    Plan::Later
                }
            })
            .collect();
        let lookups: Vec<&dyn MetadataSource> = sources.lookups().collect();
        let view_of = |i: usize| views[i].as_ref().unwrap_or(&game_view);
        let first: Vec<(usize, Option<&str>)> =
            plans.iter().enumerate().filter_map(|(i, p)| if let Plan::Ask(pin) = p { Some((i, *pin)) } else { None }).collect();
        let mut answers: Vec<Option<Lookup<MetadataCandidate>>> = vec![None; lookups.len()];
        self.ask(&ctx, &lookups, &first, &view_of, &game.title, &mut answers);

        // The fallback sources that waited: asked now when the others did
        // not identify the game, or say it is of their kind.
        let later: Vec<usize> = plans.iter().enumerate().filter(|(_, p)| matches!(p, Plan::Later)).map(|(i, _)| i).collect();
        let mut called: HashSet<usize> = HashSet::new();
        if !later.is_empty() && !Self::all_unreachable(&lookups, &answers) {
            let found = |i: usize| match &answers[i] {
                Some(Lookup::Found(c)) => Some(c),
                _ => None,
            };
            let identified = (0..lookups.len()).any(|i| {
                let d = lookups[i].descriptor();
                d.kind == SourceKind::Details
                    && d.network
                    && !d.is_fallback()
                    && found(i).is_some_and(|c| c.matched_by.is_none_or(MatchedBy::is_sure))
            });
            for &i in &later {
                let kind = lookups[i].descriptor().game_kind.as_ref();
                let of_kind = kind.is_some_and(|k| (0..lookups.len()).filter_map(found).any(|c| kind_in(k, &c.metadata)));
                if !identified || of_kind {
                    called.insert(i);
                }
            }
            let second: Vec<(usize, Option<&str>)> = later.iter().filter(|i| called.contains(i)).map(|&i| (i, None)).collect();
            self.ask(&ctx, &lookups, &second, &view_of, &game.title, &mut answers);
        }

        let mut lookups_done: Vec<(&SourceDescriptor, Option<NotAsked>, Lookup<MetadataCandidate>)> = lookups
            .iter()
            .zip(plans)
            .zip(answers)
            .enumerate()
            .map(|(i, ((source, plan), answer))| {
                let why = match plan {
                    Plan::Not(why) => Some(why),
                    Plan::Later if !called.contains(&i) => Some(NotAsked::Fallback),
                    _ => None,
                };
                (source.descriptor(), why, answer.unwrap_or(Lookup::Skipped))
            })
            .collect();
        if !settings.allow_adult_covers {
            for (_, _, lookup) in &mut lookups_done {
                if let Lookup::Found(c) = lookup {
                    c.drop_adult();
                }
            }
        }
        // Offline: every online provider that was asked failed to connect.
        let mut asked = lookups_done.iter().filter(|(s, _, l)| s.network && !matches!(l, Lookup::Skipped)).peekable();
        let offline = asked.peek().is_some() && asked.all(|(_, _, l)| matches!(l, Lookup::Unreachable));
        let any_asked = lookups_done.iter().any(|(_, _, l)| !matches!(l, Lookup::Skipped));
        let unreachable =
            lookups_done.iter().filter(|(d, _, l)| d.network && matches!(l, Lookup::Unreachable)).map(|(d, _, _)| d.id.clone()).collect();
        let mut candidates = Vec::new();
        let mut states = Vec::new();
        for (d, why, lookup) in lookups_done {
            let answer = match lookup {
                Lookup::Found(c) => {
                    let answer = SourceAnswer::Found { hit: c.hit(), pinned: pins.contains_key(&d.id) && d.searchable, used: false };
                    candidates.push(c);
                    answer
                }
                Lookup::NotFound => SourceAnswer::NotFound,
                Lookup::Unreachable => SourceAnswer::Unreachable,
                Lookup::Skipped => SourceAnswer::NotAsked {
                    why: why.unwrap_or(match d.needs {
                        Needs::ApiKey | Needs::Account => NotAsked::NeedsKey,
                        Needs::Download => NotAsked::NeedsDownload,
                        Needs::Nothing => NotAsked::NotForThisGame,
                    }),
                },
            };
            // The per-game dialog lists the details sources: a pictures
            // source has no match to show or change.
            if d.kind == SourceKind::Details && (!d.hidden || matches!(answer, SourceAnswer::Found { .. })) {
                states.push(SourceState { source: d.id.clone(), answer });
            }
        }
        Gathered { candidates, offline, asked: any_asked, states, sources: registry.clone(), unreachable }
    }

    /// Runs the lookups of `which` (source index, the game of it to look
    /// up) at once and stores their answers.
    fn ask<'a>(
        &self,
        ctx: &SourceContext<'_>,
        lookups: &[&dyn MetadataSource],
        which: &[(usize, Option<&str>)],
        view_of: &(dyn Fn(usize) -> &'a SourceGame + Sync),
        title: &str,
        answers: &mut [Option<Lookup<MetadataCandidate>>],
    ) {
        let done: Vec<(usize, Lookup<MetadataCandidate>)> = std::thread::scope(|scope| {
            let handles: Vec<_> = which
                .iter()
                .map(|&(i, pin)| {
                    let source = lookups[i];
                    let view = view_of(i);
                    (
                        i,
                        scope.spawn(move || match pin {
                            Some(id) => source.lookup_id(ctx, view, id),
                            None => source.lookup(ctx, view),
                        }),
                    )
                })
                .collect();
            handles
                .into_iter()
                .map(|(i, h)| match h.join() {
                    Ok(l) => (i, l),
                    Err(_) => {
                        log::warn!("Metadata source {} failed for {title}: panicked", lookups[i].descriptor().id);
                        (i, Lookup::NotFound)
                    }
                })
                .collect()
        });
        for (i, answer) in done {
            answers[i] = Some(answer);
        }
    }

    /// Whether every online source asked so far could not be reached.
    fn all_unreachable(lookups: &[&dyn MetadataSource], answers: &[Option<Lookup<MetadataCandidate>>]) -> bool {
        let mut asked = lookups
            .iter()
            .zip(answers)
            .filter(|(s, a)| s.descriptor().network && a.as_ref().is_some_and(|a| !matches!(a, Lookup::Skipped)))
            .peekable();
        asked.peek().is_some() && asked.all(|(_, a)| matches!(a, Some(Lookup::Unreachable)))
    }

    /// Whether the arguments' `only_sources` and `only_fields` leave `d` to
    /// ask; a pictures source only for the fields of its `view`.
    fn wanted(d: &SourceDescriptor, args: &MetadataApplyArgs, view: &SourceGame) -> bool {
        let fields = match &view.fields {
            Some(fields) => !fields.is_empty(),
            None => args.only_fields.as_ref().is_none_or(|f| f.iter().any(|field| d.provides(field))),
        };
        args.only_sources.as_ref().is_none_or(|s| s.contains(&d.id)) && fields
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_that_did_not_answer_holds_the_mark_only_when_it_could_have_found_the_game() {
        let sources = SourceRegistry::for_tests_with_fallback();
        let ids = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let pictures = ["coverSquare".to_string()];
        // A fetch of the details: a details source, not a pictures one.
        assert!(missed(&sources, &ids(&[crate::sources::TEST_FALLBACK]), None));
        assert!(!missed(&sources, &ids(&["steamgriddb", "libretro"]), None));
        assert!(!missed(&sources, &[], None));
        // A fetch of pictures only: any source.
        assert!(missed(&sources, &ids(&["steamgriddb"]), Some(&pictures)));
    }
}
