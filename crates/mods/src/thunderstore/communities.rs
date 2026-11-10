//! The list of Thunderstore communities (one per game), cached on disk.

use std::path::Path;

use heronite_data::temps::write_atomic;
use serde::{Deserialize, Serialize};

use crate::http::{self, null_default};

const FIRST_PAGE: &str = "https://thunderstore.io/api/experimental/community/";
const CACHE_FILE: &str = "thunderstore-communities.json";
const MAX_AGE_SECS: u64 = 7 * 24 * 3600;
/// Pages followed at most (the site has a few hundred communities, 100 a page).
const MAX_PAGES: usize = 30;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Community {
    /// `lethal-company`, what the listing URLs take.
    pub identifier: String,
    /// `Lethal Company`.
    pub name: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cache {
    /// Unix seconds.
    fetched_at: u64,
    communities: Vec<Community>,
}

#[derive(Deserialize)]
struct Page {
    #[serde(default)]
    pagination: Pagination,
    #[serde(default, deserialize_with = "null_default")]
    results: Vec<Community>,
}

#[derive(Deserialize, Default)]
struct Pagination {
    next_link: Option<String>,
}

/// Every community. Served from `cache_dir` for a week; when the site cannot
/// be reached an older cache is better than nothing.
pub fn communities(cache_dir: &Path) -> Result<Vec<Community>, String> {
    cached(cache_dir, http::now_secs(), fetch_all)
}

fn cached(cache_dir: &Path, now: u64, fetch: impl FnOnce() -> Result<Vec<Community>, String>) -> Result<Vec<Community>, String> {
    let path = cache_dir.join(CACHE_FILE);
    let old = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Cache>(&b).ok()).filter(|c| !c.communities.is_empty());
    if let Some(c) = old.as_ref().filter(|c| now.saturating_sub(c.fetched_at) < MAX_AGE_SECS) {
        return Ok(c.communities.clone());
    }
    match fetch() {
        Ok(list) => {
            let cache = Cache { fetched_at: now, communities: list.clone() };
            let saved = std::fs::create_dir_all(cache_dir)
                .and_then(|()| write_atomic(&path, &serde_json::to_vec(&cache).map_err(std::io::Error::other)?, None));
            if let Err(e) = saved {
                log::debug!("thunderstore: communities cache not saved: {e}");
            }
            Ok(list)
        }
        Err(e) => match old {
            Some(c) => {
                log::debug!("thunderstore: using the stale communities cache: {e}");
                Ok(c.communities)
            }
            None => Err(e),
        },
    }
}

fn fetch_all() -> Result<Vec<Community>, String> {
    let mut out = Vec::new();
    let mut url = Some(FIRST_PAGE.to_string());
    for _ in 0..MAX_PAGES {
        let Some(u) = url.take() else { break };
        let page: Page = http::get_json(&u, &[])?;
        out.extend(page.results.into_iter().filter(|c| !c.identifier.is_empty()));
        url = page.pagination.next_link.filter(|n| !n.is_empty());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn community(id: &str) -> Community {
        Community { identifier: id.into(), name: id.to_uppercase() }
    }

    #[test]
    fn pages_parse_and_ignore_extra_fields() {
        let p: Page = serde_json::from_str(
            r#"{"pagination":{"next_link":"https://thunderstore.io/api/experimental/community/?cursor=abc","previous_link":null},
                "results":[{"identifier":"raft","name":"Raft","discord_url":null,"wiki_url":null,"require_package_listing_approval":false}]}"#,
        )
        .unwrap();
        assert_eq!(p.results, vec![Community { identifier: "raft".into(), name: "Raft".into() }]);
        assert!(p.pagination.next_link.is_some());
        let last: Page = serde_json::from_str(r#"{"pagination":{"next_link":null,"previous_link":"x"},"results":[]}"#).unwrap();
        assert!(last.pagination.next_link.is_none());
    }

    #[test]
    fn the_cache_lasts_a_week_and_outlives_a_dead_network() {
        let dir = tempfile::tempdir().unwrap();
        let day = 24 * 3600;
        let first = cached(dir.path(), 1000, || Ok(vec![community("a")])).unwrap();
        assert_eq!(first, vec![community("a")]);
        // Fresh: the fetch is not called.
        let again = cached(dir.path(), 1000 + 6 * day, || panic!("fetched")).unwrap();
        assert_eq!(again, first);
        // Stale and online: refreshed.
        let fresh = cached(dir.path(), 1000 + 8 * day, || Ok(vec![community("b")])).unwrap();
        assert_eq!(fresh, vec![community("b")]);
        // Stale and offline: the old list.
        let offline = cached(dir.path(), 1000 + 20 * day, || Err("offline".into())).unwrap();
        assert_eq!(offline, vec![community("b")]);
    }

    #[test]
    fn no_cache_and_no_network_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(cached(&dir.path().join("none"), 5, || Err("offline".into())), Err("offline".to_string()));
    }

    #[test]
    #[ignore = "network"]
    fn live_all_pages() {
        let dir = tempfile::tempdir().unwrap();
        let all = communities(dir.path()).unwrap();
        assert!(all.len() > 100 && all.iter().any(|c| c.identifier == "lethal-company"), "{}", all.len());
    }
}
