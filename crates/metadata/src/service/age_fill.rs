//! Age ratings for games whose metadata was stored before ratings were read:
//! only the ratings are asked for (IGDB in batches, the Steam store page),
//! the rest of each record stays as it is.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;
use heronite_data::age_rating::{self, AgeRating};
use heronite_data::json_store;
use heronite_data::models::{CollectionGameMetadata, MetadataFile};
use serde_json::Value;

use super::MetadataService;
use crate::igdb::{IgdbCredentials, RATINGS_BATCH};
use crate::merge::{AGE_CHECKED, AGE_RATINGS};
use crate::record;
use crate::steam;
use crate::store::get_all_game_metadata;
use crate::types::Lookup;
use crate::util::now_iso;

/// Steam store pages read per pass (the store answers about 200 requests
/// in five minutes, and the Focus panel asks it too).
const STEAM_PER_PASS: usize = 100;
const STEAM_GAP: Duration = Duration::from_millis(400);

/// A record never asked for its ratings that has an id to ask with (and
/// whose ratings are not locked).
fn needs_age_ratings(md: &CollectionGameMetadata) -> bool {
    md.age_ratings.is_empty()
        && !md.extra.contains_key(AGE_CHECKED)
        && md.field_sources.get(AGE_RATINGS).map(String::as_str) != Some("manual")
        && !record::is_locked(md, AGE_RATINGS)
        && (igdb_id(md).is_some() || md.steam_app_id.as_deref().is_some_and(|s| !s.is_empty()))
}

fn igdb_id(md: &CollectionGameMetadata) -> Option<i64> {
    md.igdb_id.map(|i| i.get() as i64).filter(|i| *i != 0)
}

impl MetadataService {
    /// Fills the age ratings of stored records that have none and were
    /// never asked (when the background fill is on), checking `should_stop`
    /// before each game. Returns how many got a rating. A record whose
    /// sources answered is marked asked, rated or not; one whose source
    /// could not be reached waits for the next pass.
    /// Gives way to a bulk job like the other idle fills.
    pub fn fill_missing_age_ratings_while(&self, should_stop: &dyn Fn() -> bool) -> Result<usize> {
        if self.rating_fill.swap(true, Ordering::SeqCst) {
            return Ok(0);
        }
        let result = if self.job_pending() { Ok(0) } else { self.fill_age_ratings(&|| should_stop() || self.job_pending()) };
        self.rating_fill.store(false, Ordering::SeqCst);
        result
    }

    fn fill_age_ratings(&self, should_stop: &dyn Fn() -> bool) -> Result<usize> {
        let settings = self.metadata_settings();
        if !settings.auto_fill_missing {
            return Ok(0);
        }
        // The sources age ratings are taken from, in order: IGDB and Steam
        // unless the user turned them, or their age ratings, off.
        let order = self.sources().field_order(&settings, AGE_RATINGS);
        let (ask_igdb, ask_steam) = (order.iter().any(|s| s == "igdb"), order.iter().any(|s| s == "steam"));
        if !ask_igdb && !ask_steam {
            return Ok(0);
        }
        let todo: Vec<(String, CollectionGameMetadata)> =
            get_all_game_metadata(&self.paths)?.into_iter().filter(|(_, md)| needs_age_ratings(md)).collect();
        if todo.is_empty() {
            return Ok(0);
        }
        log::info!("Reading age ratings for {} game(s)", todo.len());

        let creds = if ask_igdb { self.credentials() } else { IgdbCredentials::new("", "") };
        let mut from_igdb: HashMap<i64, Vec<AgeRating>> = HashMap::new();
        let mut igdb_answered = creds.configured();
        if igdb_answered {
            let ids: Vec<i64> = todo.iter().filter_map(|(_, md)| igdb_id(md)).collect();
            for chunk in ids.chunks(RATINGS_BATCH) {
                if should_stop() {
                    break;
                }
                match self.igdb.age_ratings_by_id(&self.http, &creds, chunk) {
                    Ok(rows) => from_igdb.extend(rows),
                    Err(e) => {
                        log::warn!("IGDB age ratings failed: {e}");
                        igdb_answered = false;
                        break;
                    }
                }
            }
        }

        let mut found: Vec<(String, Option<(String, Vec<AgeRating>)>)> = Vec::new();
        let mut steam_asked = 0;
        for (key, md) in &todo {
            if should_stop() {
                break;
            }
            let igdb = match igdb_id(md) {
                Some(_) if creds.configured() && !igdb_answered => continue,
                Some(id) if creds.configured() => Some(from_igdb.get(&id).cloned().unwrap_or_default()),
                _ => None,
            };
            let steam = match md.steam_app_id.as_deref().filter(|s| ask_steam && !s.is_empty()) {
                Some(_) if steam_asked >= STEAM_PER_PASS => continue,
                Some(id) => {
                    if steam_asked > 0 {
                        std::thread::sleep(STEAM_GAP);
                    }
                    steam_asked += 1;
                    match steam::lookup_steam_app_details(&self.paths, &self.http, &self.steam, id, None) {
                        Lookup::Found(d) => Some(d.age_ratings.unwrap_or_default()),
                        // The other pages would not answer either.
                        Lookup::Unreachable => break,
                        _ => Some(Vec::new()),
                    }
                }
                None => None,
            };
            let lists: Vec<(&str, Vec<AgeRating>)> = order
                .iter()
                .filter_map(|source| match source.as_str() {
                    "igdb" => igdb.clone().map(|l| ("igdb", l)),
                    "steam" => steam.clone().map(|l| ("steam", l)),
                    _ => None,
                })
                .filter(|(_, l)| !l.is_empty())
                .collect();
            let rated = lists.first().map(|(source, _)| (source.to_string(), age_rating::merge(lists.iter().map(|(_, l)| l.as_slice()))));
            found.push((key.clone(), rated));
        }
        if found.is_empty() {
            return Ok(0);
        }

        let now = Value::String(now_iso());
        json_store::update::<MetadataFile, _>(&self.paths.metadata(), |file| {
            let mut rated = 0;
            for (key, result) in found {
                // A fetch or an edit may have written the record meanwhile.
                let Some(md) = file.games.get_mut(&key).filter(|md| needs_age_ratings(md)) else { continue };
                if let Some((source, ratings)) = result {
                    md.age_ratings = ratings;
                    md.field_sources.insert(AGE_RATINGS.to_owned(), source);
                    rated += 1;
                }
                md.extra.insert(AGE_CHECKED.to_owned(), now.clone());
            }
            Ok(rated)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::blank_metadata;

    #[test]
    fn a_record_with_its_ratings_locked_is_not_asked() {
        let mut md = blank_metadata("a", "sideload");
        md.igdb_id = Some(heronite_data::num::JsNum(72.0));
        assert!(needs_age_ratings(&md));
        record::set_locked(&mut md, AGE_RATINGS, true);
        assert!(!needs_age_ratings(&md));
    }
}
