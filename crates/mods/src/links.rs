//! Which Nexus Mods game and which Thunderstore community a Collection game
//! is. The table in [`games`] answers first; the sites' own lists answer the
//! rest. A network failure is never an error here, just "unknown".

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Mutex;

use heronite_data::temps::write_atomic;
use serde::{Deserialize, Serialize};

use crate::games::{self, normalize_title};
use crate::http::{self, null_default};
use crate::{nexus, thunderstore};

const NEXUS_CACHE: &str = "nexus-domains.json";
const MAX_AGE_SECS: u64 = 7 * 24 * 3600;

/// One remembered answer; `value` None = the site has no such game.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Entry {
    value: Option<String>,
    /// Unix seconds.
    checked_at: u64,
}

/// `nexus-domains.json`: title -> domain, and domain -> name.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct NexusCache {
    #[serde(deserialize_with = "null_default")]
    domains: BTreeMap<String, Entry>,
    #[serde(deserialize_with = "null_default")]
    names: BTreeMap<String, Entry>,
}

impl NexusCache {
    fn load(dir: &Path) -> Self {
        std::fs::read(dir.join(NEXUS_CACHE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn save(&self, dir: &Path) {
        let saved = std::fs::create_dir_all(dir)
            .and_then(|()| write_atomic(&dir.join(NEXUS_CACHE), &serde_json::to_vec(self).map_err(std::io::Error::other)?, None));
        if let Err(e) = saved {
            log::debug!("links: Nexus cache not saved: {e}");
        }
    }
}

fn fresh(map: &BTreeMap<String, Entry>, key: &str, now: u64) -> Option<Option<String>> {
    map.get(key).filter(|e| now.saturating_sub(e.checked_at) < MAX_AGE_SECS).map(|e| e.value.clone())
}

/// The Nexus Mods game domain of a game: the known-games table by Steam app
/// id or title, else the best exact title match on Nexus. Answers (also "no
/// such game") are kept for a week in `cache_dir`.
pub fn nexus_domain_for(cache_dir: &Path, steam_app_id: Option<&str>, title: &str) -> Option<String> {
    domain_for(cache_dir, http::now_secs(), steam_app_id, title, &nexus::games_matching)
}

type GameSearch<'a> = &'a dyn Fn(&str) -> Result<Vec<(String, String)>, String>;

fn domain_for(cache_dir: &Path, now: u64, steam_app_id: Option<&str>, title: &str, search: GameSearch) -> Option<String> {
    if let Some(domain) = games::find(steam_app_id, title).and_then(|g| g.nexus_domain) {
        return Some(domain.to_string());
    }
    let key = normalize_title(title);
    if key.is_empty() {
        return None;
    }
    let mut cache = NexusCache::load(cache_dir);
    if let Some(answer) = fresh(&cache.domains, &key, now) {
        return answer;
    }
    let mut found = None;
    for query in title_queries(title) {
        match search(&query) {
            Ok(candidates) => {
                found = candidates.into_iter().find(|(_, name)| normalize_title(name) == key).map(|(domain, _)| domain);
                if found.is_some() {
                    break;
                }
            }
            Err(e) => {
                log::debug!("links: Nexus game search for {title:?}: {e}");
                return None;
            }
        }
    }
    cache.domains.insert(key, Entry { value: found.clone(), checked_at: now });
    cache.save(cache_dir);
    found
}

/// What to ask Nexus for: the title's words, then its first two (editions
/// and subtitles keep a store's title from matching as a whole).
fn title_queries(title: &str) -> Vec<String> {
    let cleaned = title.to_lowercase().replace(['\'', '’', '™', '®'], "");
    let words: Vec<&str> = cleaned.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty() && *w != "the").collect();
    let mut queries = Vec::new();
    if !words.is_empty() {
        queries.push(words.join(" "));
    }
    if words.len() > 2 {
        queries.push(words[..2].join(" "));
    }
    queries
}

/// The name of a Nexus Mods game, to match an incoming `nxm://` link with
/// the Collection's titles.
pub fn nexus_game_name(cache_dir: &Path, domain: &str) -> Option<String> {
    game_name(cache_dir, http::now_secs(), domain, &nexus::game_name)
}

fn game_name(cache_dir: &Path, now: u64, domain: &str, ask: &dyn Fn(&str) -> Result<Option<String>, String>) -> Option<String> {
    let domain = domain.trim().to_ascii_lowercase();
    if let Some(known) = games::by_nexus_domain(&domain) {
        return Some(known.name.to_string());
    }
    let mut cache = NexusCache::load(cache_dir);
    if let Some(answer) = fresh(&cache.names, &domain, now) {
        return answer;
    }
    match ask(&domain) {
        Ok(name) => {
            cache.names.insert(domain, Entry { value: name.clone(), checked_at: now });
            cache.save(cache_dir);
            name
        }
        Err(e) => {
            log::debug!("links: Nexus game {domain}: {e}");
            None
        }
    }
}

/// The Thunderstore community of a game: the table, else the community
/// whose name is the game's title.
pub fn thunderstore_for(cache_dir: &Path, steam_app_id: Option<&str>, title: &str) -> Option<String> {
    if let Some(community) = games::find(steam_app_id, title).and_then(|g| g.thunderstore) {
        return Some(community.to_string());
    }
    if normalize_title(title).is_empty() {
        return None;
    }
    match thunderstore::communities(cache_dir) {
        Ok(list) => community_named(&list, title),
        Err(e) => {
            log::debug!("links: Thunderstore communities: {e}");
            None
        }
    }
}

fn community_named(list: &[thunderstore::Community], title: &str) -> Option<String> {
    let key = normalize_title(title);
    list.iter().find(|c| normalize_title(&c.name) == key).map(|c| c.identifier.clone())
}

/// A community's look on the site.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommunityInfo {
    pub name: String,
    /// Square icon URL.
    pub icon: Option<String>,
    /// Portrait cover image URL.
    pub cover: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawCommunity {
    #[serde(deserialize_with = "null_default")]
    name: String,
    community_icon_url: Option<String>,
    cover_image_url: Option<String>,
}

impl From<RawCommunity> for CommunityInfo {
    fn from(r: RawCommunity) -> Self {
        let url = |u: Option<String>| u.filter(|u| !u.trim().is_empty());
        CommunityInfo { name: r.name, icon: url(r.community_icon_url), cover: url(r.cover_image_url) }
    }
}

/// The icon and cover of a Thunderstore community, remembered while the app
/// runs (a failed lookup is retried).
pub fn community_art(community: &str) -> Option<CommunityInfo> {
    static SEEN: Mutex<Option<HashMap<String, CommunityInfo>>> = Mutex::new(None);
    if let Some(info) = SEEN.lock().ok()?.as_ref().and_then(|m| m.get(community)) {
        return Some(info.clone());
    }
    let url = format!("https://thunderstore.io/api/cyberstorm/community/{}/", http::encode(community));
    match http::get_json::<RawCommunity>(&url, &[]) {
        Ok(raw) => {
            let info = CommunityInfo::from(raw);
            SEEN.lock().ok()?.get_or_insert_with(HashMap::new).insert(community.to_string(), info.clone());
            Some(info)
        }
        Err(e) => {
            log::debug!("links: Thunderstore community {community}: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    fn nexus_games(rows: &[(&str, &str)]) -> Vec<(String, String)> {
        rows.iter().map(|(d, n)| (d.to_string(), n.to_string())).collect()
    }

    #[test]
    fn queries_drop_punctuation_and_shorten() {
        assert_eq!(title_queries("The Witcher 3: Wild Hunt"), ["witcher 3 wild hunt", "witcher 3"]);
        assert_eq!(title_queries("Baldur's Gate 3"), ["baldurs gate 3", "baldurs gate"]);
        assert_eq!(title_queries("Hades II"), ["hades ii"]);
        assert!(title_queries("™").is_empty());
    }

    #[test]
    fn the_table_answers_without_asking() {
        let dir = tempfile::tempdir().unwrap();
        let search = |_: &str| -> Result<Vec<(String, String)>, String> { panic!("searched") };
        assert_eq!(domain_for(dir.path(), 1, Some("489830"), "whatever", &search).as_deref(), Some("skyrimspecialedition"));
    }

    #[test]
    fn an_exact_title_match_is_found_cached_and_a_miss_is_cached_too() {
        let dir = tempfile::tempdir().unwrap();
        let asked = RefCell::new(Vec::new());
        let search = |q: &str| {
            asked.borrow_mut().push(q.to_string());
            Ok(nexus_games(&[("fallout4vr", "Fallout 4 VR"), ("zork", "Zork Zero: The Revenge of Megaboz")]))
        };
        assert_eq!(domain_for(dir.path(), 100, None, "Zork Zero - The Revenge of Megaboz", &search).as_deref(), Some("zork"));
        assert_eq!(asked.borrow().len(), 1);
        // Cached: no search.
        let never = |_: &str| -> Result<Vec<(String, String)>, String> { panic!("searched again") };
        assert_eq!(domain_for(dir.path(), 200, None, "zork zero the revenge of megaboz", &never).as_deref(), Some("zork"));
        // No match: both queries tried, then remembered as absent.
        assert_eq!(domain_for(dir.path(), 100, None, "Nothing Like It Anywhere", &search), None);
        assert_eq!(asked.borrow().len(), 3);
        assert_eq!(domain_for(dir.path(), 300, None, "Nothing Like It Anywhere", &never), None);
        // A week later it asks again.
        let found = |_: &str| Ok(nexus_games(&[("nothing", "Nothing Like It Anywhere")]));
        assert_eq!(domain_for(dir.path(), 100 + 8 * 24 * 3600, None, "Nothing Like It Anywhere", &found).as_deref(), Some("nothing"));
    }

    #[test]
    fn a_failed_search_is_not_remembered() {
        let dir = tempfile::tempdir().unwrap();
        let offline = |_: &str| -> Result<Vec<(String, String)>, String> { Err("offline".into()) };
        assert_eq!(domain_for(dir.path(), 1, None, "Some Game Title", &offline), None);
        let online = |_: &str| Ok(nexus_games(&[("some", "Some Game Title")]));
        assert_eq!(domain_for(dir.path(), 2, None, "Some Game Title", &online).as_deref(), Some("some"));
    }

    #[test]
    fn game_names_come_from_the_table_then_the_site() {
        let dir = tempfile::tempdir().unwrap();
        let never = |_: &str| -> Result<Option<String>, String> { panic!("asked") };
        assert_eq!(game_name(dir.path(), 1, "SkyrimSpecialEdition", &never).as_deref(), Some("Skyrim Special Edition"));
        let site = |_: &str| Ok(Some("Obscure Game".to_string()));
        assert_eq!(game_name(dir.path(), 1, "obscure", &site).as_deref(), Some("Obscure Game"));
        assert_eq!(game_name(dir.path(), 5, "obscure", &never).as_deref(), Some("Obscure Game"));
        let none = |_: &str| Ok(None);
        assert_eq!(game_name(dir.path(), 1, "nothing", &none), None);
        assert_eq!(game_name(dir.path(), 5, "nothing", &never), None);
        let offline = |_: &str| -> Result<Option<String>, String> { Err("offline".into()) };
        assert_eq!(game_name(dir.path(), 1, "other", &offline), None);
    }

    #[test]
    fn communities_match_by_title() {
        let list = vec![
            thunderstore::Community { identifier: "lethal-company".into(), name: "Lethal Company".into() },
            thunderstore::Community { identifier: "ror2".into(), name: "Risk of Rain 2".into() },
        ];
        assert_eq!(community_named(&list, "LETHAL COMPANY").as_deref(), Some("lethal-company"));
        assert_eq!(community_named(&list, "Risk of Rain 2").as_deref(), Some("ror2"));
        assert_eq!(community_named(&list, "Valheim"), None);
    }

    #[test]
    fn community_pages_give_art() {
        let raw: RawCommunity = serde_json::from_str(
            r#"{"name":"Lethal Company","identifier":"lethal-company","cover_image_url":"https://c/cover.webp","icon_url":null,
                "community_icon_url":"https://c/icon.webp","has_mod_manager_support":true}"#,
        )
        .unwrap();
        let info = CommunityInfo::from(raw);
        assert_eq!(
            (info.name.as_str(), info.icon.as_deref(), info.cover.as_deref()),
            ("Lethal Company", Some("https://c/icon.webp"), Some("https://c/cover.webp"))
        );
    }

    #[test]
    #[ignore = "network"]
    fn live_lookups() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(nexus_domain_for(dir.path(), None, "The Witcher 3: Wild Hunt").as_deref(), Some("witcher3"));
        assert_eq!(nexus_domain_for(dir.path(), None, "Baldur's Gate 3").as_deref(), Some("baldursgate3"));
        assert_eq!(nexus_game_name(dir.path(), "witcher3").as_deref(), Some("The Witcher 3: Wild Hunt"));
        assert_eq!(thunderstore_for(dir.path(), None, "Raft").as_deref(), Some("raft"));
        let art = community_art("lethal-company").unwrap();
        assert!(art.icon.is_some() && art.cover.is_some());
    }
}
