//! A made-up review (`HERONITE_DEBUG_DIALOG=review…`, or while
//! `HERONITE_DEBUG_METADATA_JOB` makes jobs up): proposals and searches are
//! invented from the game's title, nothing is asked or written (the first
//! game also has a proposal of a made-up extension, `novels`, with its
//! original title). The pictures are covers the Collection already has.

use heronite_metadata::service::SourceSearch;
use heronite_metadata::{MatchChoice, MatchedBy, ReviewReason, SourceHit};

use super::ReviewGame;

pub(super) struct FakeReview {
    thumbs: Vec<String>,
    /// Every game comes back with nothing found (`review:empty`).
    pub(super) empty: bool,
}

fn hit(source: &str, id: &str, title: &str, year: i32, matched_by: Option<MatchedBy>) -> SourceHit {
    SourceHit {
        source: source.into(),
        id: Some(id.into()),
        title: Some(title.into()),
        year: Some(year),
        platform: None,
        thumb_url: None,
        matched_by,
        alt_title: None,
        thumb_adult: false,
    }
}

fn choice(title: &str, year: i32, platform: &str, thumb: Option<&String>, hits: Vec<SourceHit>) -> MatchChoice {
    MatchChoice {
        title: Some(title.into()),
        year: Some(year),
        platform: Some(platform.into()),
        alt_title: None,
        thumb_url: thumb.cloned(),
        hits,
    }
}

/// What a made-up search looks for: the title up to its subtitle.
pub(super) fn search_query(title: &str) -> String {
    title.split([':', '-']).next().unwrap_or(title).trim().to_string()
}

impl FakeReview {
    pub(super) fn new(thumbs: Vec<String>) -> Self {
        Self { thumbs, empty: false }
    }

    /// Game `i` of the review: three choices, two, or one, in turn.
    pub(super) fn entry(&self, i: usize, game: &ReviewGame) -> (ReviewReason, Vec<MatchChoice>) {
        let t = game.item.title.as_str();
        let thumb = |k: usize| self.thumbs.get((i + k) % self.thumbs.len().max(1));
        if self.empty {
            return (ReviewReason::Other, Vec::new());
        }
        match i % 3 {
            0 => (
                ReviewReason::SourcesDisagree,
                vec![
                    choice(
                        t,
                        1997,
                        "PlayStation",
                        thumb(0),
                        vec![
                            hit("igdb", "1128", t, 1997, Some(MatchedBy::SteamId)),
                            hit("steam", "620", t, 1997, Some(MatchedBy::SteamId)),
                            hit("lutris", "game", t, 1997, Some(MatchedBy::ExactTitle)),
                        ],
                    ),
                    // A visual novel of the same name, with its original title.
                    MatchChoice {
                        alt_title: Some("ビジュアルノベル版".into()),
                        ..choice(
                            t,
                            2009,
                            "Windows, PS3",
                            thumb(2),
                            vec![hit("novels", "n2002", t, 2009, Some(MatchedBy::ExactTitle)), hit("lutris", "game-vn", t, 2009, None)],
                        )
                    },
                    choice(
                        t,
                        1998,
                        "Sega Saturn",
                        None,
                        vec![hit("igdb", "9071", t, 1998, Some(MatchedBy::FuzzyTitle)), hit("lutris", "game-ss", t, 1998, None)],
                    ),
                    choice(
                        t,
                        2007,
                        "Xbox 360, PSP",
                        thumb(1),
                        vec![hit("igdb", "20011", t, 2007, Some(MatchedBy::FuzzyTitle)), hit("steam", "1203", t, 2007, None)],
                    ),
                ],
            ),
            1 => {
                let other = format!("{t}: Remastered");
                (
                    ReviewReason::SourcesDisagree,
                    vec![
                        choice(t, 2011, "PC", thumb(2), vec![hit("steam", "4410", t, 2011, Some(MatchedBy::ExactTitle))]),
                        choice(
                            &other,
                            2020,
                            "PC, PlayStation 4",
                            None,
                            vec![hit("igdb", "7712", &other, 2020, Some(MatchedBy::FuzzyTitle))],
                        ),
                    ],
                )
            }
            _ => {
                let close = format!("{t} II");
                (
                    ReviewReason::LoneGuess,
                    vec![choice(&close, 2004, "PC", thumb(3), vec![hit("igdb", "3305", &close, 2004, Some(MatchedBy::FuzzyTitle))])],
                )
            }
        }
    }

    /// A search by `query`: two games, and Lutris not answering.
    pub(super) fn search(&self, query: &str) -> SourceSearch {
        let deluxe = format!("{query} Deluxe");
        let thumb = self.thumbs.first();
        SourceSearch {
            choices: vec![
                choice(
                    query,
                    1994,
                    "Super Nintendo",
                    thumb,
                    vec![hit("igdb", "501", query, 1994, None), hit("steam", "502", query, 1994, None)],
                ),
                choice(&deluxe, 2009, "PC", None, vec![hit("igdb", "503", &deluxe, 2009, None)]),
            ],
            asked: vec!["igdb".into(), "steam".into(), "lutris".into()],
            unreachable: vec!["lutris".into()],
            needs_key: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_search_looks_for_the_title_before_its_subtitle() {
        assert_eq!(search_query("Castlevania: Symphony of the Night"), "Castlevania");
        assert_eq!(search_query("Castlevania - Symphony of the Night"), "Castlevania");
        assert_eq!(search_query("Portal"), "Portal");
    }
}
