use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use heronite_data::json_store;
use heronite_data::paths::DataPaths;
use serde::{Deserialize, Serialize};

use crate::confidence::YEAR_SLACK;
use crate::covers::search_exact_app;
use crate::http::Http;
use crate::types::Lookup;
use crate::util::normalize_title;

/// A Steam id found by title is searched again after this long.
const HIT_MAX_AGE_SECS: i64 = 30 * 24 * 3600;
/// "Not on Steam" is kept a shorter time: the game may be released since.
const MISS_MAX_AGE_SECS: i64 = 3 * 24 * 3600;

/// Until when no search by name is made at a Steam store (by address): it said to wait (429, 5xx).
static BUSY_UNTIL: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(Default::default);

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Entry {
    /// The normalized title: names the file by a hash, so this tells a collision.
    title: String,
    id: Option<String>,
    fetched_at: String,
}

/// One small file per title (`titles/<hash>.json`), so remembering a title
/// writes only its own file.
fn file(paths: &DataPaths, key: &str) -> std::path::PathBuf {
    // FNV-1a: stable across builds, unlike the standard hasher.
    let hash = key.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3));
    paths.source_cache_dir("steam").join("titles").join(format!("{hash:016x}.json"))
}

/// `Some(Some(id))` found, `Some(None)` searched and not on Steam, `None`
/// not kept (or too old). No network.
fn read(paths: &DataPaths, title: &str, now: DateTime<Utc>) -> Option<Option<String>> {
    let key = normalize_title(title);
    let entry: Entry = json_store::read(&file(paths, &key)).ok()?;
    if entry.title != key {
        return None;
    }
    let age = now.signed_duration_since(DateTime::parse_from_rfc3339(&entry.fetched_at).ok()?).num_seconds();
    let max = if entry.id.is_some() { HIT_MAX_AGE_SECS } else { MISS_MAX_AGE_SECS };
    (0..max).contains(&age).then(|| entry.id.clone().filter(|id| id.bytes().all(|b| b.is_ascii_digit())))
}

fn write(paths: &DataPaths, title: &str, id: Option<&str>, now: DateTime<Utc>) {
    let key = normalize_title(title);
    let entry = Entry { title: key.clone(), id: id.map(str::to_owned), fetched_at: now.to_rfc3339() };
    let path = file(paths, &key);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = json_store::write(&path, &entry) {
        log::warn!("Steam title cache write failed for {title}: {e:#}");
    }
}

fn busy_now(http: &Http) -> bool {
    BUSY_UNTIL.lock().is_ok_and(|m| m.get(&http.endpoints.steam_store).is_some_and(|t| *t > Instant::now()))
}

fn be_busy_for(http: &Http, wait: Duration) {
    if let Ok(mut m) = BUSY_UNTIL.lock() {
        m.insert(http.endpoints.steam_store.clone(), Instant::now() + wait);
    }
}

/// The Steam app whose name is `title`, from the memory or the search. A
/// Steam that said to wait is left alone for that long (`Unreachable`, as a
/// bulk run would otherwise keep asking it); only an answer that is
/// complete is remembered, a "no" included.
pub fn app_id(paths: &DataPaths, http: &Http, title: &str) -> Lookup<String> {
    if normalize_title(title).is_empty() {
        return Lookup::NotFound;
    }
    match read(paths, title, Utc::now()) {
        Some(Some(id)) => return Lookup::Found(id),
        Some(None) => return Lookup::NotFound,
        None => {}
    }
    if busy_now(http) {
        return Lookup::Unreachable;
    }
    let searched = search_exact_app(http, title);
    if let Some(wait) = searched.busy {
        be_busy_for(http, wait);
    }
    match &searched.lookup {
        Lookup::Found(id) => write(paths, title, Some(id), Utc::now()),
        Lookup::NotFound if searched.complete => write(paths, title, None, Utc::now()),
        _ => {}
    }
    searched.lookup
}

/// Whether the Steam game named `name`, released in `released`, is the game
/// titled `title`, whose year is `year` when the record has one: the same
/// name, not one with edition words or another game's, and not a game of
/// another time (Doom, 1993 and 2016).
pub fn is_the_game(title: &str, year: Option<i32>, name: &str, released: Option<i32>) -> bool {
    let (wanted, got) = (normalize_title(title), normalize_title(name));
    if wanted.is_empty() || wanted != got {
        return false;
    }
    match (year, released) {
        (Some(a), Some(b)) => (a - b).abs() <= YEAR_SLACK,
        _ => true,
    }
}

/// Whether `found`, the game a search by name matched before, is still the game
/// titled `title` (the record's year `year`): after a rename, or once a year is
/// known, it may not be.
pub fn match_is_current(found: &crate::record::SourceMatch, title: &str, year: Option<i32>) -> bool {
    found.title.as_deref().is_none_or(|name| is_the_game(title, year, name, found.year))
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
    fn a_title_is_a_file_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        write(&paths, "Hades", Some("1"), Utc::now());
        write(&paths, "Celeste", None, Utc::now());
        let files = std::fs::read_dir(paths.source_cache_dir("steam").join("titles")).unwrap().count();
        assert_eq!(files, 2);
        // A file read for another title (a hash collision) is not its answer.
        let other = file(&paths, &normalize_title("Hades"));
        std::fs::copy(&other, file(&paths, &normalize_title("Celeste"))).unwrap();
        assert_eq!(read(&paths, "Celeste", Utc::now()), None);
    }

    #[test]
    fn only_the_same_game_is_taken() {
        assert!(is_the_game("Hades", None, "HADES™", Some(2020)));
        assert!(is_the_game("Doom", Some(2016), "DOOM", Some(2016)));
        assert!(is_the_game("Doom", Some(2015), "DOOM", Some(2016)), "a year apart is the same game");
        assert!(!is_the_game("Doom", Some(1993), "DOOM", Some(2016)), "another game of the same title");
        assert!(
            !is_the_game("Sekiro: Shadows Die Twice", None, "Sekiro: Shadows Die Twice - GOTY Edition", None),
            "an edition is not the title"
        );
        assert!(!is_the_game("Portal", None, "Portal 2", None));
        assert!(!is_the_game("", None, "", None));
    }
}
