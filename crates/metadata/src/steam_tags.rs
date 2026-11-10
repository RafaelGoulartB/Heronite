//! Steam store tags from the keyless Steam Web API, written to
//! `store_cache/steam_tags.json` ([`heronite_data::steam_tags`] reads it).
//!
//! - `IStoreBrowseService/GetItems` gives the tag ids and weights of many
//!   apps per request (the store page's "popular user-defined tags");
//! - `IStoreService/GetTagList` gives the id -> name list of a language.
//!
//! Apps are asked again after 30 days, tag names after 7. Requests go one
//! at a time, a second apart, and say they come from Heronite.

use std::collections::HashSet;
use std::time::Duration;

use anyhow::{anyhow, Result};
use heronite_data::json_store;
use heronite_data::paths::DataPaths;
use heronite_data::steam_tags::{AppTags, SteamTagsFile, TagNames, TagWeight, MAX_LIMIT};
use serde_json::{json, Value};

use crate::http::Http;
use crate::util::now_ms;

/// Apps per `GetItems` request.
pub const BATCH: usize = 50;
/// Pause between two requests.
pub const GAP: Duration = Duration::from_secs(1);
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const APP_TTL_MS: i64 = 30 * DAY_MS;
const NAMES_TTL_MS: i64 = 7 * DAY_MS;

/// Sent with every request here: who is asking.
pub const USER_AGENT: &str = concat!("Heronite/", env!("CARGO_PKG_VERSION"), " (+https://github.com/RafaelGoulartB/Heronite)");

/// The tags of each app `GetItems` returned (`success == 1`), heaviest
/// first. Apps Steam does not list are absent.
pub fn parse_items(body: &Value) -> Vec<(String, Vec<TagWeight>)> {
    let items = body.pointer("/response/store_items").and_then(Value::as_array);
    items
        .into_iter()
        .flatten()
        .filter(|i| i.get("success").and_then(Value::as_i64) == Some(1))
        .filter_map(|i| {
            let id = i.get("appid").or_else(|| i.get("id")).and_then(Value::as_u64).filter(|id| *id != 0)?;
            let mut tags: Vec<TagWeight> = i
                .get("tags")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|t| {
                    let tagid = u32::try_from(t.get("tagid")?.as_u64()?).ok()?;
                    let weight = u32::try_from(t.get("weight").and_then(Value::as_u64).unwrap_or(0)).unwrap_or(u32::MAX);
                    Some(TagWeight { tagid, weight })
                })
                .collect();
            tags.sort_by_key(|t| std::cmp::Reverse(t.weight));
            Some((id.to_string(), tags))
        })
        .collect()
}

/// `GetTagList`'s answer; `None` when it holds no tag.
pub fn parse_tag_list(body: &Value, fetched_at: i64) -> Option<TagNames> {
    let response = body.get("response")?;
    let names: std::collections::BTreeMap<u32, String> = response
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let id = u32::try_from(t.get("tagid")?.as_u64()?).ok()?;
            let name = t.get("name")?.as_str()?.trim();
            (!name.is_empty()).then(|| (id, name.to_owned()))
        })
        .collect();
    if names.is_empty() {
        return None;
    }
    let version_hash = response.get("version_hash").and_then(Value::as_str).unwrap_or_default().to_owned();
    Some(TagNames { version_hash, fetched_at, names })
}

fn get_json(http: &Http, path: &str, query: &[(&str, &str)]) -> Result<Value> {
    let url = format!("{}/{path}", http.endpoints.steam_api);
    http.get(url).query(query).header("User-Agent", USER_AGENT).timeout_secs(20).send_ok()?.json()
}

fn fetch_items(http: &Http, ids: &[String]) -> Result<Vec<(String, Vec<TagWeight>)>> {
    let ids: Vec<Value> = ids.iter().filter_map(|id| id.parse::<u64>().ok()).map(|id| json!({ "appid": id })).collect();
    let input = json!({
        "ids": ids,
        // Tags are ids: the language and country only matter to other fields.
        "context": { "language": "english", "country_code": "US" },
        "data_request": { "include_tag_count": MAX_LIMIT },
    });
    let body = get_json(http, "IStoreBrowseService/GetItems/v1/", &[("input_json", &input.to_string())])?;
    if body.pointer("/response").is_none() {
        return Err(anyhow!("GetItems answered without a response"));
    }
    Ok(parse_items(&body))
}

fn fetch_tag_list(http: &Http, steam_language: &str) -> Result<TagNames> {
    let body = get_json(http, "IStoreService/GetTagList/v1/", &[("language", steam_language)])?;
    parse_tag_list(&body, now_ms()).ok_or_else(|| anyhow!("GetTagList answered without tags"))
}

/// The ids of `ids` (Steam app ids, digits only) without a fresh entry.
pub fn stale_ids(file: &SteamTagsFile, ids: &[String], now: i64) -> Vec<String> {
    let mut seen = HashSet::new();
    ids.iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
        .filter(|id| seen.insert(id.to_string()))
        .filter(|id| file.apps.get(*id).is_none_or(|a| now - a.fetched_at >= APP_TTL_MS))
        .map(str::to_owned)
        .collect()
}

/// What [`refresh`] changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Refreshed {
    /// Apps written (with or without tags).
    pub apps: usize,
    /// Tag name lists written.
    pub names: usize,
    /// Steam could not be reached: the rest waits for the next pass.
    pub unreachable: bool,
}

impl Refreshed {
    pub fn changed(&self) -> bool {
        self.apps + self.names > 0
    }
}

/// Fetches the tags of the apps in `ids` that have none or old ones, and the
/// tag names in `steam_language` (and English, the fallback) when missing,
/// old, or lacking an id just fetched. `gap` is the pause between requests;
/// `should_stop` is checked before each.
pub fn refresh(
    paths: &DataPaths,
    http: &Http,
    ids: &[String],
    steam_language: &str,
    gap: Duration,
    should_stop: &dyn Fn() -> bool,
) -> Result<Refreshed> {
    fetch(paths, http, ids, steam_language, gap, false, should_stop)
}

/// [`refresh`] that asks for every app of `ids` again, fresh or not (a bulk
/// update that refreshes what sources gave).
pub fn refresh_all(
    paths: &DataPaths,
    http: &Http,
    ids: &[String],
    steam_language: &str,
    gap: Duration,
    should_stop: &dyn Fn() -> bool,
) -> Result<Refreshed> {
    fetch(paths, http, ids, steam_language, gap, true, should_stop)
}

fn fetch(
    paths: &DataPaths,
    http: &Http,
    ids: &[String],
    steam_language: &str,
    gap: Duration,
    all: bool,
    should_stop: &dyn Fn() -> bool,
) -> Result<Refreshed> {
    let file = SteamTagsFile::load(paths);
    let now = now_ms();
    // Against an empty file every valid id is stale.
    let empty = SteamTagsFile::default();
    let todo = stale_ids(if all { &empty } else { &file }, ids, now);
    let mut out = Refreshed::default();
    let mut apps: Vec<(String, AppTags)> = Vec::new();
    let mut requests = 0usize;
    let pause = |requests: &mut usize| {
        if *requests > 0 {
            std::thread::sleep(gap);
        }
        *requests += 1;
    };

    for chunk in todo.chunks(BATCH) {
        if should_stop() {
            break;
        }
        pause(&mut requests);
        match fetch_items(http, chunk) {
            Ok(found) => {
                let mut by_id: std::collections::HashMap<String, Vec<TagWeight>> = found.into_iter().collect();
                // An app Steam did not list is remembered without tags, so it
                // is not asked again before the others.
                apps.extend(chunk.iter().map(|id| (id.clone(), AppTags { tags: by_id.remove(id).unwrap_or_default(), fetched_at: now })));
            }
            Err(e) => {
                log::warn!("Steam tags: GetItems failed: {e:#}");
                out.unreachable = crate::http::is_unreachable(&e);
                break;
            }
        }
    }

    let mut languages = vec![steam_language.to_owned()];
    if steam_language != "english" {
        languages.push("english".to_owned());
    }
    let new_ids: HashSet<u32> = apps.iter().flat_map(|(_, a)| a.tags.iter().map(|t| t.tagid)).collect();
    let mut names: Vec<(String, TagNames)> = Vec::new();
    if !out.unreachable {
        for lang in languages {
            let current = file.names.get(&lang);
            let needed = match current {
                None => !file.apps.is_empty() || !apps.is_empty(),
                Some(n) => now - n.fetched_at >= NAMES_TTL_MS || new_ids.iter().any(|id| !n.names.contains_key(id)),
            };
            if !needed || should_stop() {
                continue;
            }
            pause(&mut requests);
            match fetch_tag_list(http, &lang) {
                Ok(list) => names.push((lang, list)),
                Err(e) => {
                    log::warn!("Steam tags: GetTagList ({lang}) failed: {e:#}");
                    if crate::http::is_unreachable(&e) {
                        out.unreachable = true;
                        break;
                    }
                }
            }
        }
    }

    if apps.is_empty() && names.is_empty() {
        return Ok(out);
    }
    out.apps = apps.len();
    out.names = names.len();
    json_store::update::<SteamTagsFile, _>(&paths.steam_tags(), |file| {
        file.apps.extend(apps);
        file.names.extend(names);
        Ok(())
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        let text = std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn reads_recorded_get_items() {
        let found = parse_items(&fixture("steam_getitems_tags.json"));
        let ids: Vec<&str> = found.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["620", "1145360"], "the unlisted app is left out");
        let portal = &found[0].1;
        assert_eq!(portal.len(), 20);
        assert_eq!(portal[0], TagWeight { tagid: 4182, weight: 604 });
        assert!(portal.windows(2).all(|w| w[0].weight >= w[1].weight));
    }

    #[test]
    fn reads_recorded_tag_lists() {
        let en = parse_tag_list(&fixture("steam_taglist_en.json"), 5).unwrap();
        assert_eq!(en.names[&4182], "Singleplayer");
        assert_eq!(en.names[&42804], "Action Roguelike");
        assert_eq!(en.fetched_at, 5);
        assert!(!en.version_hash.is_empty());
        let br = parse_tag_list(&fixture("steam_taglist_br.json"), 5).unwrap();
        assert_eq!(br.names[&1664], "Quebra-Cabeça");
        assert!(parse_tag_list(&json!({"response": {"tags": []}}), 1).is_none());
        assert!(parse_tag_list(&json!({}), 1).is_none());
    }

    #[test]
    fn only_missing_or_old_numeric_ids_are_asked() {
        let mut file = SteamTagsFile::default();
        let now = 100 * DAY_MS;
        file.apps.insert("1".into(), AppTags { tags: vec![], fetched_at: now - DAY_MS });
        file.apps.insert("2".into(), AppTags { tags: vec![], fetched_at: now - 31 * DAY_MS });
        let ids: Vec<String> = ["1", "2", "3", "3", " 4 ", "", "abc", "5x"].iter().map(|s| s.to_string()).collect();
        assert_eq!(stale_ids(&file, &ids, now), ["2", "3", "4"]);
    }
}
