//! Finding a game on Steam by its exact title (Steam's option "Find games
//! not on Steam by name"): a search whose answers are remembered, and the
//! rule of which game it accepts.
//!
//! The search of a title is kept in `store_cache/sources/steam/titles.json`
//! (`{"<normalized title>": {"id": "620", "fetchedAt": "<rfc3339>"}}`), the
//! way ProtonDB keeps its own: a game found is asked again after a month, a
//! title Steam has no game for after three days. A Steam that did not answer
//! (offline, throttling or failing) is never kept.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use heronite_data::json_store;
use heronite_data::paths::DataPaths;
use serde::{Deserialize, Serialize};

use crate::confidence::YEAR_SLACK;
use crate::covers::lookup_steam_app_id_exact;
use crate::http::Http;
use crate::steam::year_of_date;
use crate::types::Lookup;
use crate::util::normalize_title;

/// A Steam id found by title is searched again after this long.
const HIT_MAX_AGE_SECS: i64 = 30 * 24 * 3600;
/// "Not on Steam" is kept a shorter time: the game may be released since.
const MISS_MAX_AGE_SECS: i64 = 3 * 24 * 3600;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Entry {
    id: Option<String>,
    fetched_at: String,
}

fn file(paths: &DataPaths) -> std::path::PathBuf {
    paths.source_cache_dir("steam").join("titles.json")
}

/// `Some(Some(id))` found, `Some(None)` searched and not on Steam, `None`
/// not kept (or too old). No network.
fn read(paths: &DataPaths, title: &str, now: DateTime<Utc>) -> Option<Option<String>> {
    let titles: BTreeMap<String, Entry> = json_store::read(&file(paths)).ok()?;
    let entry = titles.get(&normalize_title(title))?;
    let age = now.signed_duration_since(DateTime::parse_from_rfc3339(&entry.fetched_at).ok()?).num_seconds();
    let max = if entry.id.is_some() { HIT_MAX_AGE_SECS } else { MISS_MAX_AGE_SECS };
    (0..max).contains(&age).then(|| entry.id.clone().filter(|id| id.bytes().all(|b| b.is_ascii_digit())))
}

fn write(paths: &DataPaths, title: &str, id: Option<&str>, now: DateTime<Utc>) {
    let entry = Entry { id: id.map(str::to_owned), fetched_at: now.to_rfc3339() };
    let res = json_store::update::<BTreeMap<String, Entry>, _>(&file(paths), |titles| {
        titles.insert(normalize_title(title), entry);
        Ok(())
    });
    if let Err(e) = res {
        log::warn!("Steam title cache write failed for {title}: {e:#}");
    }
}

/// The Steam app whose name is `title`, from the memory or the search.
pub fn app_id(paths: &DataPaths, http: &Http, title: &str) -> Lookup<String> {
    if normalize_title(title).is_empty() {
        return Lookup::NotFound;
    }
    match read(paths, title, Utc::now()) {
        Some(Some(id)) => return Lookup::Found(id),
        Some(None) => return Lookup::NotFound,
        None => {}
    }
    let found = lookup_steam_app_id_exact(http, title);
    match &found {
        Lookup::Found(id) => write(paths, title, Some(id), Utc::now()),
        Lookup::NotFound => write(paths, title, None, Utc::now()),
        Lookup::Unreachable | Lookup::Skipped => {}
    }
    found
}

/// Whether the Steam game named `name`, released `released` (Steam's own
/// wording of the date), is the game titled `title`, whose year is `year`
/// when the record has one: the same name, not one with edition words or
/// another game's, and not a game of another time (Doom, 1993 and 2016).
pub fn is_the_game(title: &str, year: Option<i32>, name: &str, released: Option<&str>) -> bool {
    let (wanted, got) = (normalize_title(title), normalize_title(name));
    if wanted.is_empty() || wanted != got {
        return false;
    }
    match (year, released.and_then(year_of_date)) {
        (Some(a), Some(b)) => (a - b).abs() <= YEAR_SLACK,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn the_search_is_remembered_longer_for_a_game_than_for_none() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let now = Utc::now();
        assert_eq!(read(&paths, "Hades", now), None);
        write(&paths, "Hades", Some("1145360"), now - Duration::days(10));
        write(&paths, "Not A Steam Game", None, now - Duration::days(1));
        assert_eq!(read(&paths, "HADES™", now), Some(Some("1145360".into())));
        assert_eq!(read(&paths, "Not a Steam game", now), Some(None));
        write(&paths, "Not A Steam Game", None, now - Duration::days(4));
        assert_eq!(read(&paths, "Not A Steam Game", now), None, "a miss is asked again sooner");
        write(&paths, "Hades", Some("1145360"), now - Duration::days(40));
        assert_eq!(read(&paths, "Hades", now), None);
    }

    #[test]
    fn only_the_same_game_is_taken() {
        assert!(is_the_game("Hades", None, "HADES™", Some("17 Sep, 2020")));
        assert!(is_the_game("Doom", Some(2016), "DOOM", Some("12 May, 2016")));
        assert!(is_the_game("Doom", Some(2015), "DOOM", Some("12 May, 2016")), "a year apart is the same game");
        assert!(!is_the_game("Doom", Some(1993), "DOOM", Some("12 May, 2016")), "another game of the same title");
        assert!(
            !is_the_game("Sekiro: Shadows Die Twice", None, "Sekiro: Shadows Die Twice - GOTY Edition", None),
            "an edition is not the title"
        );
        assert!(!is_the_game("Portal", None, "Portal 2", None));
        assert!(!is_the_game("", None, "", None));
    }
}
