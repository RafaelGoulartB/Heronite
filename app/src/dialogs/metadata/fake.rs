//! A made-up preview for `HERONITE_DEBUG_DIALOG=metadata:<state>`: nothing
//! is asked or written. `fake` (and `change`, with "Change" open): IGDB
//! pinned, Steam found by its id, Lutris found another game (left out), the
//! store not for this game, a fallback source not asked (the others found
//! the game), genres locked and the release date picked from Steam;
//! `fallback`: as `fake`, with that source asked and found by its title
//! ([`FALLBACK`], a made-up extension id: not in the registry, so it shows
//! by its id and cannot be searched); `emulated`: as `fake` for an
//! emulated game, Steam not for it, the LaunchBox Games Database found
//! by the dump's name on its platform and libretro-thumbnails (a pictures
//! source: no match to show) giving its box art; `none`: no source found the
//! game; `offline`: none could be reached; `nokey`: as `fake`, without an
//! IGDB key; `covers`: as `fake`, scrolled to the cover rows. SteamGridDB
//! gives a cover of each shape. The pictures are covers the Collection
//! already has.

use heronite_data::collection::Game;
use heronite_data::models::CollectionGameMetadata;
use heronite_data::num::JsNum;
use heronite_metadata::service::SourceSearch;
use heronite_metadata::{MatchChoice, MatchedBy, MetadataCandidate, MetadataPreview, NotAsked, SourceAnswer, SourceHit, SourceState};
use indexmap::IndexMap;

/// The made-up fallback source of the made-up states.
const FALLBACK: &str = "novels";

pub(super) struct FakePreview {
    state: String,
    thumbs: Vec<String>,
}

fn blank(game: &Game) -> CollectionGameMetadata {
    CollectionGameMetadata { app_name: game.app_name.clone(), runner: game.runner.clone(), ..Default::default() }
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

impl FakePreview {
    pub(super) fn new(state: &str, thumbs: Vec<String>) -> Self {
        Self { state: state.to_owned(), thumbs }
    }

    fn thumb(&self, i: usize) -> Option<String> {
        self.thumbs.get(i % self.thumbs.len().max(1)).cloned()
    }

    fn candidate(
        &self,
        source: &str,
        title: &str,
        year: &str,
        by: MatchedBy,
        id: &str,
        fill: impl Fn(&mut CollectionGameMetadata),
    ) -> MetadataCandidate {
        let mut md = CollectionGameMetadata { title: Some(title.into()), release_date: Some(year.into()), ..Default::default() };
        fill(&mut md);
        MetadataCandidate::new(source, md, Some(by)).with_id(Some(id.into()))
    }

    /// The preview, with the pins changed in the dialog applied.
    pub(super) fn preview(&self, game: &Game, pins: &IndexMap<String, Option<String>>) -> MetadataPreview {
        let t = game.title.as_str();
        let mut current = game.metadata.as_deref().cloned().unwrap_or_else(|| blank(game));
        let nothing = matches!(self.state.as_str(), "none" | "offline");
        if nothing {
            current = blank(game);
        }
        let candidates = if nothing {
            Vec::new()
        } else {
            vec![
                self.candidate("igdb", t, "2004-11-17", MatchedBy::SourceId, "1128", |m| {
                    m.description = Some(format!("{t}: the story, where it takes place and how it plays, as IGDB tells it. A second line, so the value is cut after two lines."));
                    m.developers = strings(&["North Studio"]);
                    m.genres = strings(&["Action", "Adventure"]);
                    m.series = Some(t.split(':').next().unwrap_or(t).trim().to_string());
                    m.critic_score = Some(JsNum(91.));
                    m.cover_url = self.thumb(0);
                    m.hero_url = self.thumb(1);
                }),
                self.candidate("steam", t, "2004-11-17", MatchedBy::SteamId, "2131630", |m| {
                    m.description = Some(format!("{t}, as its Steam page tells it."));
                    m.developers = strings(&["North Studio", "Port House"]);
                    m.genres = strings(&["Action"]);
                    m.critic_score = Some(JsNum(88.));
                    m.cover_url = self.thumb(2);
                }),
                // A pictures source: a cover of each shape.
                self.candidate("steamgriddb", t, "", MatchedBy::SteamId, "5245", |m| {
                    m.release_date = None;
                    m.cover_url = self.thumb(3);
                    m.cover_square_url = self.thumb(1);
                    m.cover_wide_url = self.thumb(1);
                }),
                self.candidate("lutris", &format!("{t}: Subsistence"), "2005", MatchedBy::FuzzyTitle, "subsistence", |m| {
                    m.genres = strings(&["Action"]);
                    m.critic_score = Some(JsNum(90.));
                }),
            ]
        };
        let mut candidates = candidates;
        let emulated = self.state == "emulated";
        if emulated {
            candidates.push(self.candidate("launchbox", t, "1997-03-20", MatchedBy::ExactTitle, "511", |m| {
                m.description = Some(format!("{t}, as the LaunchBox Games Database tells it."));
                m.developers = strings(&["North Studio"]);
                m.genres = strings(&["Action", "Platform"]);
                m.platforms = strings(&["PlayStation"]);
                m.critic_score = Some(JsNum(94.));
                m.cover_url = self.thumb(4);
            }));
            candidates.push(self.candidate("libretro", t, "", MatchedBy::ExactTitle, "", |m| {
                m.release_date = None;
                m.cover_url = self.thumb(2);
                m.cover_wide_url = self.thumb(3);
            }));
        }
        if self.state == "fallback" {
            candidates.push(self.candidate(FALLBACK, t, "2004-11", MatchedBy::ExactTitle, "n2002", |m| {
                m.description = Some(format!("{t}, as a database of visual novels tells it."));
                m.developers = strings(&["North Studio"]);
                m.genres = strings(&["Visual Novel"]);
                m.themes = strings(&["Time Travel", "Conspiracy", "Male Protagonist"]);
                m.critic_score = Some(JsNum(90.));
                m.cover_url = self.thumb(4);
                m.hero_url = self.thumb(5);
            }));
        }
        let found = |c: &MetadataCandidate, used: bool| SourceAnswer::Found {
            hit: c.hit(),
            pinned: pins.get(&c.source).map_or(c.source == "igdb", Option::is_some),
            used,
        };
        let sources = if nothing {
            let missing = if self.state == "offline" { SourceAnswer::Unreachable } else { SourceAnswer::NotFound };
            vec![
                SourceState { source: "igdb".into(), answer: missing.clone() },
                SourceState { source: "steam".into(), answer: SourceAnswer::NotAsked { why: NotAsked::NotForThisGame } },
                SourceState { source: "lutris".into(), answer: missing.clone() },
                SourceState { source: "store".into(), answer: SourceAnswer::NotAsked { why: NotAsked::NotForThisGame } },
                // Asked only when the others are reached and find nothing.
                SourceState {
                    source: FALLBACK.into(),
                    answer: if self.state == "offline" { SourceAnswer::NotAsked { why: NotAsked::Fallback } } else { missing },
                },
            ]
        } else {
            vec![
                SourceState { source: "igdb".into(), answer: found(&candidates[0], true) },
                SourceState { source: "steam".into(), answer: found(&candidates[1], true) },
                SourceState { source: "lutris".into(), answer: found(&candidates[3], false) },
                SourceState { source: "store".into(), answer: SourceAnswer::NotAsked { why: NotAsked::NotForThisGame } },
                SourceState {
                    source: FALLBACK.into(),
                    answer: match candidates.get(4).filter(|c| c.source == FALLBACK) {
                        Some(fallback) => found(fallback, true),
                        None => SourceAnswer::NotAsked { why: NotAsked::Fallback },
                    },
                },
                SourceState {
                    source: "launchbox".into(),
                    answer: match candidates.iter().find(|c| c.source == "launchbox") {
                        Some(lb) => found(lb, true),
                        None => SourceAnswer::NotAsked { why: NotAsked::NeedsDownload },
                    },
                },
            ]
        };
        let mut sources = sources;
        let mut candidates = candidates;
        if emulated {
            if let Some(steam) = sources.iter_mut().find(|s| s.source == "steam") {
                steam.answer = SourceAnswer::NotAsked { why: NotAsked::NotForThisGame };
            }
            candidates.retain(|c| c.source != "steam");
        }
        let mut proposed = current.clone();
        if let Some(igdb) = candidates.first() {
            let m = &igdb.metadata;
            for (field, set) in [
                ("description", m.description.is_some()),
                ("releaseDate", true),
                ("developers", true),
                ("series", true),
                ("criticScore", true),
                ("cover", true),
                ("hero", true),
            ] {
                if set {
                    proposed.field_sources.insert(field.into(), "igdb".into());
                }
            }
        }
        let mut locks = Vec::new();
        if !nothing {
            current.genres = strings(&["Action", "Adventure", "Stealth"]);
            locks.push("genres".to_string());
        }
        MetadataPreview {
            app_name: game.app_name.clone(),
            current: Some(current),
            proposed,
            candidates,
            igdb_configured: self.state != "nokey",
            offline: self.state == "offline",
            sources,
            locks,
            review: None,
        }
    }

    /// The picks the made-up state shows: the release date from Steam.
    pub(super) fn picks(&self) -> Vec<(&'static str, &'static str)> {
        match self.state.as_str() {
            "none" | "offline" | "emulated" => Vec::new(),
            _ => vec![("releaseDate", "steam")],
        }
    }

    /// A search of `source` alone.
    pub(super) fn search(&self, source: &str, query: &str) -> SourceSearch {
        let hit = |id: &str, title: String, year: i32, platform: &str| SourceHit {
            source: source.into(),
            id: Some(id.into()),
            title: Some(title),
            year: Some(year),
            platform: Some(platform.into()),
            thumb_url: None,
            matched_by: None,
            alt_title: None,
            thumb_adult: false,
        };
        let one = |h: SourceHit| MatchChoice {
            title: h.title.clone(),
            year: h.year,
            platform: h.platform.clone(),
            alt_title: h.alt_title.clone(),
            thumb_url: None,
            hits: vec![h],
        };
        SourceSearch {
            choices: vec![
                one(hit("1128", query.to_string(), 2004, "PlayStation 2")),
                one(hit("1129", format!("{query}: Subsistence"), 2005, "PlayStation 2")),
                one(hit("1130", format!("{query} 3D"), 2012, "Nintendo 3DS")),
                one(hit("1131", format!("{query} HD Collection"), 2011, "PlayStation 3, Xbox 360")),
            ],
            asked: vec![source.into()],
            ..Default::default()
        }
    }
}
