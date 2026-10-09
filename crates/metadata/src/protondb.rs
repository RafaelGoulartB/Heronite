//! ProtonDB tier of a Steam game (`platinum`, `gold`, `silver`, `bronze`,
//! `borked`, `pending`), cached on disk for a week.
//!
//! One file per game, `<cache_dir>/<appid>.json`:
//! `{"tier": "gold", "fetchedAt": "<rfc3339>"}`. Games ProtonDB does not know
//! (404) are stored as `"unknown"` so they are not fetched again that week.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use heronite_data::json_store;
use serde::{Deserialize, Serialize};

use crate::http::Http;

/// A cached tier older than this is fetched again.
pub const MAX_AGE_SECS: u64 = 7 * 24 * 3600;

/// Stored for games ProtonDB has no summary for.
const UNKNOWN: &str = "unknown";

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct CacheEntry {
    tier: String,
    fetched_at: String,
}

#[derive(Deserialize)]
struct Summary {
    #[serde(default)]
    tier: String,
}

/// Steam app ids are digits only; this also keeps them safe as file names.
fn valid_app_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())
}

fn cache_file(cache_dir: &Path, steam_app_id: &str) -> PathBuf {
    cache_dir.join(format!("{steam_app_id}.json"))
}

/// The cache entry as `(tier, fresh)`, `"unknown"` included.
fn read_entry(cache_dir: &Path, steam_app_id: &str, now: DateTime<Utc>) -> Option<(String, bool)> {
    if !valid_app_id(steam_app_id) {
        return None;
    }
    let entry: CacheEntry = json_store::read(&cache_file(cache_dir, steam_app_id)).ok()?;
    let tier = entry.tier.trim().to_lowercase();
    if tier.is_empty() {
        return None;
    }
    let fetched = DateTime::parse_from_rfc3339(&entry.fetched_at).ok()?;
    let age = now.signed_duration_since(fetched).num_seconds();
    Some((tier, (0..MAX_AGE_SECS as i64).contains(&age)))
}

fn write_entry(cache_dir: &Path, steam_app_id: &str, tier: &str, now: DateTime<Utc>) {
    let entry = CacheEntry { tier: tier.to_owned(), fetched_at: now.to_rfc3339() };
    if let Err(e) = json_store::write(&cache_file(cache_dir, steam_app_id), &entry) {
        log::warn!("ProtonDB cache write failed for {steam_app_id}: {e:#}");
    }
}

fn known(tier: String) -> Option<String> {
    (tier != UNKNOWN).then_some(tier)
}

/// Cached tier (lowercase, e.g. `gold`) and whether it is still fresh. No
/// network. `None` when nothing is cached or the game is unknown to ProtonDB.
pub fn cached_tier(cache_dir: &Path, steam_app_id: &str) -> Option<(String, bool)> {
    let (tier, fresh) = read_entry(cache_dir, steam_app_id, Utc::now())?;
    known(tier).map(|tier| (tier, fresh))
}

/// Asks ProtonDB. `Ok(None)` is a 404; other failures are errors.
fn fetch(http: &Http, steam_app_id: &str) -> anyhow::Result<Option<String>> {
    let url = format!("{}/api/v1/reports/summaries/{steam_app_id}.json", http.endpoints.protondb);
    let resp = http.get(url).timeout_secs(10).max_bytes(256 * 1024).send()?;
    if resp.status == 404 {
        return Ok(None);
    }
    if !resp.ok() {
        return Err(resp.error());
    }
    let summary: Summary = resp.json()?;
    let tier = summary.tier.trim().to_lowercase();
    anyhow::ensure!(!tier.is_empty(), "ProtonDB summary without a tier");
    Ok(Some(tier))
}

/// The tier of a game: the cache when fresh, else fetched and cached. A
/// network error falls back to a stale cache. `None` for unknown games and
/// for ids that are not numeric.
pub fn tier(cache_dir: &Path, http: &Http, steam_app_id: &str) -> Option<String> {
    if !valid_app_id(steam_app_id) {
        return None;
    }
    let cached = read_entry(cache_dir, steam_app_id, Utc::now());
    if let Some((tier, true)) = cached {
        return known(tier);
    }
    match fetch(http, steam_app_id) {
        Ok(found) => {
            write_entry(cache_dir, steam_app_id, found.as_deref().unwrap_or(UNKNOWN), Utc::now());
            found
        }
        Err(e) => {
            log::warn!("ProtonDB lookup failed for {steam_app_id}: {e:#}");
            cached.and_then(|(tier, _)| known(tier))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn put(dir: &Path, id: &str, tier: &str, age_secs: i64) {
        write_entry(dir, id, tier, Utc::now() - Duration::seconds(age_secs));
    }

    #[test]
    fn cache_round_trip_and_freshness() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(cached_tier(dir.path(), "1245620"), None);
        put(dir.path(), "1245620", "gold", 60);
        assert_eq!(cached_tier(dir.path(), "1245620"), Some(("gold".into(), true)));
        put(dir.path(), "1245620", "Platinum", MAX_AGE_SECS as i64 + 60);
        assert_eq!(cached_tier(dir.path(), "1245620"), Some(("platinum".into(), false)));
    }

    #[test]
    fn unknown_is_cached_but_not_reported() {
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), "7", UNKNOWN, 0);
        assert_eq!(cached_tier(dir.path(), "7"), None);
        assert_eq!(read_entry(dir.path(), "7", Utc::now()), Some((UNKNOWN.into(), true)));
    }

    #[test]
    fn broken_entries_and_bad_ids_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("1.json"), "garbage").unwrap();
        std::fs::write(dir.path().join("2.json"), r#"{"tier":"gold","fetchedAt":"yesterday"}"#).unwrap();
        assert_eq!(cached_tier(dir.path(), "1"), None);
        assert_eq!(cached_tier(dir.path(), "2"), None);
        put(dir.path(), "3", "gold", 0);
        assert_eq!(cached_tier(dir.path(), "../3"), None);
        assert_eq!(cached_tier(dir.path(), ""), None);
        assert_eq!(tier(dir.path(), &Http::default(), "abc"), None);
    }

    #[test]
    fn future_timestamps_are_not_fresh() {
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), "5", "gold", -3600);
        assert_eq!(cached_tier(dir.path(), "5"), Some(("gold".into(), false)));
    }
}
