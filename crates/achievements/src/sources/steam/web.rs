//! Steam's Web API: the global unlock percentages (no key) and, with the
//! user's key, the schema and the unlocks of a game this PC has no stats
//! files for.

use std::collections::HashMap;

use chrono::DateTime;
use heronite_metadata::Http;
use serde_json::Value;

use super::local::{Entry, Schema, Unlocks};

/// Offset between an account id and the SteamID64 the Web API takes.
const STEAMID64_BASE: u64 = 76561197960265728;

pub fn steam_id64(account: &str) -> Option<u64> {
    account.parse::<u64>().ok().and_then(|id| id.checked_add(STEAMID64_BASE))
}

/// The answer of `url` as JSON; `secret` never shows in the error.
fn get_json(http: &Http, url: &str, query: &[(&str, &str)], secret: &str) -> Result<Value, String> {
    let hide = |text: String| if secret.is_empty() { text } else { text.replace(secret, "***") };
    let response = http.get(url).query(query).send().map_err(|e| hide(format!("{e:#}")))?;
    if !response.ok() {
        return Err(format!("HTTP {}", response.status));
    }
    response.json().map_err(|e| hide(format!("{e:#}")))
}

/// API name → share of the players (`0.0..=100.0`) that have it.
pub fn parse_rarity(doc: &Value) -> HashMap<String, f32> {
    let list = doc.pointer("/achievementpercentages/achievements").and_then(Value::as_array);
    list.into_iter()
        .flatten()
        .filter_map(|item| {
            let name = item.get("name")?.as_str()?;
            let percent = item.get("percent").and_then(|p| p.as_f64().or_else(|| p.as_str()?.parse().ok()))?;
            Some((name.to_string(), (percent as f32).clamp(0., 100.)))
        })
        .collect()
}

/// Empty when the call fails: rarity is a nicety, not a reason to fail.
pub fn rarity(http: &Http, api: &str, app_id: &str) -> HashMap<String, f32> {
    let url = format!("{api}/ISteamUserStats/GetGlobalAchievementPercentagesForApp/v0002/");
    match get_json(http, &url, &[("gameid", app_id)], "") {
        Ok(doc) => parse_rarity(&doc),
        Err(e) => {
            log::info!("Steam rarity of {app_id}: {e}");
            HashMap::new()
        }
    }
}

/// The schema out of `GetSchemaForGame/v2`.
pub fn parse_schema(doc: &Value) -> Schema {
    let list = doc.pointer("/game/availableGameStats/achievements").and_then(Value::as_array);
    let text = |item: &Value, key: &str| item.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let entries = list
        .into_iter()
        .flatten()
        .filter(|item| item.get("name").and_then(Value::as_str).is_some())
        .map(|item| Entry {
            api: text(item, "name"),
            name: text(item, "displayName"),
            description: text(item, "description"),
            hidden: item.get("hidden").and_then(Value::as_i64).is_some_and(|h| h != 0),
            icon: text(item, "icon"),
            icon_gray: text(item, "icongray"),
            ..Default::default()
        })
        .collect();
    Schema::from_entries(entries)
}

pub fn schema(http: &Http, api: &str, key: &str, app_id: &str, lang: &str) -> Result<Schema, String> {
    let url = format!("{api}/ISteamUserStats/GetSchemaForGame/v2/");
    let doc = get_json(http, &url, &[("key", key), ("appid", app_id), ("l", lang)], key)?;
    Ok(parse_schema(&doc))
}

/// The unlocks out of `GetPlayerAchievements/v1`.
pub fn parse_player(doc: &Value) -> Result<Unlocks, String> {
    let stats = doc.get("playerstats").ok_or("no player stats in the answer")?;
    if stats.get("success").and_then(Value::as_bool) == Some(false) {
        return Err(stats.get("error").and_then(Value::as_str).unwrap_or("the answer says it failed").to_string());
    }
    let mut unlocks = Unlocks::default();
    for item in stats.get("achievements").and_then(Value::as_array).into_iter().flatten() {
        let (Some(api), Some(1)) = (item.get("apiname").and_then(Value::as_str), item.get("achieved").and_then(Value::as_i64)) else {
            continue;
        };
        let at = item.get("unlocktime").and_then(Value::as_i64).filter(|t| *t > 0).and_then(|t| DateTime::from_timestamp(t, 0));
        unlocks.done.insert(api.to_string(), at);
    }
    Ok(unlocks)
}

pub fn player(http: &Http, api: &str, key: &str, account: &str, app_id: &str, lang: &str) -> Result<Unlocks, String> {
    let steam_id = steam_id64(account).ok_or("the Steam account id is not a number")?.to_string();
    let url = format!("{api}/ISteamUserStats/GetPlayerAchievements/v1/");
    let doc = get_json(http, &url, &[("key", key), ("steamid", &steam_id), ("appid", app_id), ("l", lang)], key)?;
    parse_player(&doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rarity_accepts_numbers_and_strings() {
        let doc = json!({"achievementpercentages": {"achievements": [
            {"name": "A", "percent": 12.5}, {"name": "B", "percent": "3.25"}, {"name": "C"}]}});
        let map = parse_rarity(&doc);
        assert_eq!((map["A"], map["B"], map.len()), (12.5, 3.25, 2));
        assert!(parse_rarity(&json!({})).is_empty());
    }

    #[test]
    fn schema_of_the_web_api() {
        let doc = json!({"game": {"availableGameStats": {"achievements": [
            {"name": "WIN", "defaultvalue": 0, "displayName": "Win", "hidden": 1, "description": "Win it", "icon": "https://x/a.jpg", "icongray": "https://x/b.jpg"},
            {"name": "TWO", "displayName": "Two"}]}}});
        let schema = parse_schema(&doc);
        assert_eq!(schema.entries.len(), 2);
        let win = &schema.entries[0];
        assert_eq!((win.api.as_str(), win.name.as_str(), win.hidden, win.icon_gray.as_str()), ("WIN", "Win", true, "https://x/b.jpg"));
        assert_eq!(schema.entries[1].description, "");
        assert!(parse_schema(&json!({"game": {}})).entries.is_empty());
    }

    #[test]
    fn player_unlocks_and_errors() {
        let doc = json!({"playerstats": {"success": true, "achievements": [
            {"apiname": "WIN", "achieved": 1, "unlocktime": 1700000000},
            {"apiname": "TWO", "achieved": 0, "unlocktime": 0},
            {"apiname": "NOTIME", "achieved": 1, "unlocktime": 0}]}});
        let unlocks = parse_player(&doc).unwrap();
        assert_eq!(unlocks.done.len(), 2);
        assert_eq!(unlocks.done["WIN"].unwrap().timestamp(), 1_700_000_000);
        assert_eq!(unlocks.done["NOTIME"], None);
        let private = json!({"playerstats": {"error": "Profile is not public", "success": false}});
        assert_eq!(parse_player(&private).unwrap_err(), "Profile is not public");
        assert!(parse_player(&json!({})).is_err());
    }

    #[test]
    fn steam_id() {
        assert_eq!(steam_id64("314512607"), Some(76561198274778335));
        assert_eq!(steam_id64("x"), None);
    }
}
