//! The game's public achievements page on Steam Community
//! (`/stats/<appid>/achievements/`): its list without a key, for the games
//! the client keeps no stats files of (never played on this PC). The page
//! is rendered on the server with its data as JSON in
//! `<script id="valve-ssr-data">`, whose `renderContext.queryData` (JSON in a
//! string) holds the `["achievements", <appid>, "schema_v2"]` query
//! (`"schema"` on older pages). The language follows the `Steam_Language`
//! cookie.
//!
//! The query's `groups` are Steam's Achievement Sets: `id`, `name` (empty
//! for the default group), `order`, `dlcappid` (0 for the base game) and
//! `archived`, each with its achievements. A game without sets has one
//! unnamed group `0`. Fixtures: `fixtures/community_*.json`, trimmed from
//! the live pages.

use heronite_metadata::Http;
use serde_json::Value;

use super::local::{sort_groups, Entry, Schema};
use crate::model::AchievementGroup;

pub const COMMUNITY_BASE: &str = "https://steamcommunity.com";

/// The JSON of the page's `valve-ssr-data` script.
fn ssr_data(html: &str) -> Option<Value> {
    let start = html.find("id=\"valve-ssr-data\"")?;
    let open = start + html[start..].find('>')? + 1;
    let close = open + html[open..].find("</script>")?;
    serde_json::from_str(&html[open..close]).ok()
}

/// The schema out of the page; `None` when it has no achievements data.
pub fn parse_page(html: &str) -> Option<Schema> {
    let doc = ssr_data(html)?;
    let queries: Value = serde_json::from_str(doc.pointer("/renderContext/queryData")?.as_str()?).ok()?;
    let schema = queries.get("queries")?.as_array()?.iter().find_map(|q| {
        let key = q.get("queryKey")?.as_array()?;
        let is_schema = key.first()?.as_str() == Some("achievements") && matches!(key.get(2)?.as_str(), Some("schema_v2" | "schema"));
        is_schema.then(|| q.pointer("/state/data")).flatten()
    })?;
    parse_schema(schema)
}

/// The query's data: every group's achievements, flat, and the groups.
fn parse_schema(schema: &Value) -> Option<Schema> {
    let text = |item: &Value, key: &str| item.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let flag = |item: &Value, key: &str| item.get(key).and_then(Value::as_bool).unwrap_or(false);
    let mut entries = Vec::new();
    let mut groups = Vec::new();
    for g in schema.get("groups")?.as_array()? {
        let id = g.get("id").and_then(Value::as_i64).map(|id| id.to_string());
        if let Some(id) = &id {
            groups.push(AchievementGroup {
                id: id.clone(),
                name: text(g, "name"),
                order: g.get("order").and_then(Value::as_i64),
                dlc_app_id: g.get("dlcappid").and_then(Value::as_u64).and_then(|d| u32::try_from(d).ok()),
                archived: flag(g, "archived"),
                ..Default::default()
            });
        }
        let Some(list) = g.get("achievements").and_then(Value::as_array) else { continue };
        entries.extend(list.iter().filter(|a| !flag(a, "archived")).filter_map(|a| {
            Some(Entry {
                api: a.get("api_name")?.as_str()?.to_string(),
                name: text(a, "name"),
                description: text(a, "description"),
                hidden: flag(a, "hidden"),
                icon: text(a, "icon_achieved"),
                icon_gray: text(a, "icon_unachieved"),
                group: id.clone(),
                ..Default::default()
            })
        }));
    }
    sort_groups(&mut groups);
    let mut schema = Schema::from_entries(entries);
    schema.groups = groups;
    Some(schema)
}

/// The game's list in `lang` (Steam's language name, `brazilian`).
pub fn schema(http: &Http, base: &str, app_id: &str, lang: &str) -> Result<Schema, String> {
    let url = format!("{base}/stats/{app_id}/achievements/");
    let response = http.get(url).header("Cookie", &format!("Steam_Language={lang}")).send().map_err(|e| format!("{e:#}"))?;
    if !response.ok() {
        return Err(format!("HTTP {}", response.status));
    }
    Ok(parse_page(&response.text()).unwrap_or_else(|| Schema::from_entries(Vec::new())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn page(schema: Value) -> String {
        page_with("schema", schema)
    }

    fn page_with(query: &str, schema: Value) -> String {
        let queries = json!({"mutations": [], "queries": [
            {"queryKey": ["AOWarningCookie"], "state": {"data": null}},
            {"queryKey": ["achievements", 400, query], "state": {"data": schema}},
        ]});
        let data = json!({"route": "/achievements/:appid", "renderContext": {"queryData": queries.to_string()}});
        format!("<html><script type=\"application/json\" id=\"valve-ssr-data\" nonce=\"x\">{data}</script><script>1</script></html>")
    }

    #[test]
    fn reads_the_schema_query_of_the_page() {
        let html = page(json!({"appid": 400, "language": "english", "groups": [{"id": 0, "achievements": [
            {"internal_key": 0, "api_name": "PORTAL_GET_PORTALGUNS", "name": "Lab Rat", "description": "Acquire it.",
             "hidden": false, "archived": false, "icon_achieved": "https://x/on.jpg", "icon_unachieved": "https://x/off.jpg"},
            {"internal_key": 1, "api_name": "SECRET", "name": "S", "description": "", "hidden": true, "archived": false,
             "icon_achieved": "a", "icon_unachieved": "b"},
            {"internal_key": 2, "api_name": "OLD", "name": "Old", "hidden": false, "archived": true}]}]}));
        let schema = parse_page(&html).unwrap();
        let apis: Vec<_> = schema.entries.iter().map(|e| e.api.as_str()).collect();
        assert_eq!(apis, ["PORTAL_GET_PORTALGUNS", "SECRET"]);
        let lab = &schema.entries[0];
        assert_eq!((lab.name.as_str(), lab.icon.as_str(), lab.icon_gray.as_str()), ("Lab Rat", "https://x/on.jpg", "https://x/off.jpg"));
        assert!(schema.entries[1].hidden);
    }

    /// `fixtures/<name>.json`, as the live page's `schema_v2` query.
    fn fixture(name: &str) -> Schema {
        let json = match name {
            "dredge" => include_str!("fixtures/community_1562430_dredge.json"),
            "portal2" => include_str!("fixtures/community_620_portal2.json"),
            "portal2_brazilian" => include_str!("fixtures/community_620_portal2_brazilian.json"),
            "portal" => include_str!("fixtures/community_400_portal.json"),
            _ => unreachable!(),
        };
        parse_page(&page_with("schema_v2", serde_json::from_str(json).unwrap())).unwrap()
    }

    fn group_view(schema: &Schema) -> Vec<(&str, &str, Option<i64>, Option<u32>)> {
        schema.groups.iter().map(|g| (g.id.as_str(), g.name.as_str(), g.order, g.dlc_app_id)).collect()
    }

    #[test]
    fn base_game_and_dlc_sets_keep_ids_dlc_and_membership() {
        // https://steamcommunity.com/stats/1562430/achievements/ (DREDGE)
        let schema = fixture("dredge");
        assert_eq!(
            group_view(&schema),
            [("0", "", Some(0), Some(0)), ("1", "The Pale Reach", Some(0), Some(2561440)), ("2", "The Iron Rig", Some(0), Some(2561450))]
        );
        let membership: Vec<_> = schema.entries.iter().map(|e| (e.api.as_str(), e.group.as_deref(), e.hidden)).collect();
        assert_eq!(
            membership,
            [
                ("CATCH_FISH_ROD_1", Some("0"), false),
                ("COMPLETE_CHAPTER_2", Some("0"), true),
                ("DLC_3_1", Some("1"), false),
                ("DLC_3_3", Some("1"), true),
                ("DLC_4_1", Some("2"), false),
                ("DLC_4_4", Some("2"), true),
            ]
        );
        assert!(!schema.groups[0].is_dlc() && schema.groups[1].is_dlc());
        // Icons are file names here; `build` turns them into CDN URLs.
        assert_eq!(schema.entries[0].icon, "12ee49fe9ad45969bb4d106c099517279a940521.jpg");
    }

    #[test]
    fn named_sets_follow_their_order_and_names_follow_the_language() {
        // https://steamcommunity.com/stats/620/achievements/ (Portal 2), English and Steam_Language=brazilian
        let en = fixture("portal2");
        assert_eq!(group_view(&en), [("2", "Single Player", Some(0), Some(0)), ("1", "Cooperative", Some(1), Some(0))]);
        let pt = fixture("portal2_brazilian");
        assert_eq!(group_view(&pt), [("2", "Um jogador", Some(0), Some(0)), ("1", "Cooperativo", Some(1), Some(0))]);
        let ids = |s: &Schema| s.entries.iter().map(|e| (e.api.clone(), e.group.clone())).collect::<Vec<_>>();
        assert_eq!(ids(&en), ids(&pt));
    }

    #[test]
    fn groups_sort_by_order_keeping_the_page_order_on_ties() {
        let html = page_with(
            "schema_v2",
            json!({"groups": [
                {"id": 7, "name": "B", "order": 1, "achievements": []},
                {"id": 3, "name": "A", "order": 0, "achievements": []},
                {"id": 5, "name": "C", "order": 1, "achievements": []}]}),
        );
        let ids: Vec<_> = parse_page(&html).unwrap().groups.into_iter().map(|g| g.id).collect();
        assert_eq!(ids, ["3", "7", "5"]);
    }

    #[test]
    fn a_game_without_sets_has_one_unnamed_group() {
        // https://steamcommunity.com/stats/400/achievements/ (Portal)
        let schema = fixture("portal");
        assert_eq!(group_view(&schema), [("0", "", Some(0), Some(0))]);
        assert!(schema.entries.iter().all(|e| e.group.as_deref() == Some("0")));
    }

    #[test]
    fn missing_group_fields_stay_unknown() {
        let html = page_with("schema_v2", json!({"groups": [{"id": 1, "achievements": [{"api_name": "A"}]}]}));
        let schema = parse_page(&html).unwrap();
        assert_eq!(group_view(&schema), [("1", "", None, None)]);
        assert!(!schema.groups[0].archived);
    }

    #[test]
    fn a_page_without_data_has_no_schema() {
        assert!(parse_page("<html>nothing</html>").is_none());
        assert!(parse_page(&page(json!(null))).is_none());
    }
}
