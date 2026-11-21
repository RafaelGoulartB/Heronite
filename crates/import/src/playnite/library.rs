//! What Playnite knows of a game besides how it starts: its details and the
//! user's own (favourite, hidden, score, status, categories, notes, the day
//! it was added). A game holds the ids of its genres, companies, tags...;
//! their names are in a file of their own beside `games.db` (`genres.db`,
//! `companies.db`...), each item an `_id` and a `Name`.

use std::collections::HashMap;
use std::path::Path;

use heronite_metadata::providers::PlayniteGame;

use super::Install;
use crate::litedb::{self, Bson, Document};
use crate::transfer::{Start, Transfer};

/// The files of named items a game refers to.
const KINDS: [&str; 8] = ["genres", "companies", "tags", "features", "categories", "platforms", "series", "completionstatuses"];

/// The names of the items of a library, by kind and id. A file that cannot
/// be read gives none (the game then comes without them).
#[derive(Debug, Default)]
pub(super) struct Names(HashMap<&'static str, HashMap<String, String>>);

impl Names {
    pub(super) fn read(library: &Path) -> Self {
        let read = |kind: &str| -> HashMap<String, String> {
            let file = library.join(format!("{kind}.db"));
            let docs = std::fs::read(&file).map_err(anyhow::Error::from).and_then(|b| litedb::documents(&b));
            let docs = docs.unwrap_or_else(|e| {
                log::debug!("Reading {}: {e:#}", file.display());
                Vec::new()
            });
            docs.iter().filter_map(|d| Some((d.guid("_id")?, d.str("Name")?.trim().to_string()))).filter(|(_, n)| !n.is_empty()).collect()
        };
        Names(KINDS.into_iter().map(|k| (k, read(k))).collect())
    }

    /// The names of the items of `kind` the game's `field` lists, in its order.
    fn of(&self, kind: &str, d: &Document, field: &str) -> Vec<String> {
        let names = self.0.get(kind);
        ids(d, field).iter().filter_map(|id| names?.get(id).cloned()).collect()
    }

    /// The name of the first platform of the game.
    pub(super) fn platform(&self, d: &Document) -> Option<String> {
        self.of("platforms", d, "PlatformIds").into_iter().next()
    }
}

/// The ids a game's `field` lists.
fn ids(d: &Document, field: &str) -> Vec<String> {
    match d.get(field) {
        Some(Bson::Array(items)) => items
            .iter()
            .filter_map(|b| match b {
                Bson::Binary(4, bytes) => litedb::guid(bytes),
                Bson::String(s) => Some(s.to_ascii_lowercase()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// `YYYY-MM-DD` of milliseconds since 1970 (UTC).
fn day(ms: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(ms).map(|t| t.format("%Y-%m-%d").to_string())
}

/// Playnite's release date: `2020-1-5`, `2020-1` or `2020` (its
/// `ReleaseDate.Serialize`), as `2020-01-05`, `2020-01` or `2020`.
pub(super) fn release_date(d: &Document) -> Option<String> {
    match d.get("ReleaseDate")? {
        Bson::String(s) => {
            let parts: Vec<u32> = s.trim().split('-').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
            match parts.as_slice() {
                [y] => Some(format!("{y:04}")),
                [y, m] if (1..=12).contains(m) => Some(format!("{y:04}-{m:02}")),
                [y, m, d] if (1..=12).contains(m) && (1..=31).contains(d) => Some(format!("{y:04}-{m:02}-{d:02}")),
                _ => None,
            }
        }
        Bson::DateTime(ms) => day(*ms),
        _ => None,
    }
}

/// What the game `d` carries when it starts here by `start`.
pub(super) fn transfer(
    d: &Document,
    install: &Install,
    names: &Names,
    start: Start,
    actions: Vec<heronite_data::game_actions::Action>,
) -> Transfer {
    let text = |k: &str| d.str(k).map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
    let links = match d.get("Links") {
        Some(Bson::Array(items)) => items
            .iter()
            .filter_map(|l| match l {
                Bson::Document(l) => l.str("Url").map(str::trim).filter(|u| !u.is_empty()).map(str::to_string),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    let details = PlayniteGame {
        id: d.guid("_id").unwrap_or_default(),
        name: text("Name").unwrap_or_default(),
        description: text("Description"),
        release_date: release_date(d),
        developers: names.of("companies", d, "DeveloperIds"),
        publishers: names.of("companies", d, "PublisherIds"),
        genres: names.of("genres", d, "GenreIds"),
        tags: names.of("tags", d, "TagIds"),
        platforms: names.of("platforms", d, "PlatformIds"),
        series: names.of("series", d, "SeriesIds").into_iter().next(),
        features: names.of("features", d, "FeatureIds"),
        critic_score: d.int("CriticScore"),
        links,
        cover_image: text("CoverImage"),
        background_image: text("BackgroundImage"),
    };
    let status = d.guid("CompletionStatusId").and_then(|id| Some((id.clone(), names.0.get("completionstatuses")?.get(&id)?.clone())));
    Transfer {
        start,
        details,
        library: install.library.clone(),
        favourite: d.bool("Favorite").unwrap_or(false),
        hidden: d.bool("Hidden").unwrap_or(false),
        stars: d.int("UserScore").and_then(crate::transfer::stars),
        status,
        notes: text("Notes"),
        added: d.date("Added").and_then(day),
        categories: names.of("categories", d, "CategoryIds"),
        actions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(value: Bson) -> Document {
        Document(vec![("ReleaseDate".into(), value)])
    }

    #[test]
    fn release_dates_are_padded() {
        assert_eq!(release_date(&with(Bson::String("2020-1-5".into()))).as_deref(), Some("2020-01-05"));
        assert_eq!(release_date(&with(Bson::String("1997-9".into()))).as_deref(), Some("1997-09"));
        assert_eq!(release_date(&with(Bson::String("1998".into()))).as_deref(), Some("1998"));
        assert_eq!(release_date(&with(Bson::DateTime(1_767_323_045_123))).as_deref(), Some("2026-01-02"));
        assert_eq!(release_date(&with(Bson::String("2020-13-1".into()))), None);
        assert_eq!(release_date(&with(Bson::String("soon".into()))), None);
        assert_eq!(release_date(&Document::default()), None);
    }
}
