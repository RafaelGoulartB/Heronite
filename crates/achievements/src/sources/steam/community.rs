//! The game's public achievements page on Steam Community
//! (`/stats/<appid>/achievements/`): its list without a key, for the games
//! the client keeps no stats files of (never played on this PC). The page
//! is rendered on the server with its data as JSON in
//! `<script id="valve-ssr-data">`, whose `renderContext.queryData` (JSON in a
//! string) holds the `["achievements", <appid>, "schema"]` query. The
//! language follows the `Steam_Language` cookie.

use heronite_metadata::Http;
use serde_json::Value;

use super::local::{Entry, Schema};

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
        let is_schema = key.first()?.as_str() == Some("achievements") && key.get(2)?.as_str() == Some("schema");
        is_schema.then(|| q.pointer("/state/data")).flatten()
    })?;
    let text = |item: &Value, key: &str| item.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let entries = schema
        .get("groups")?
        .as_array()?
        .iter()
        .filter_map(|g| g.get("achievements")?.as_array())
        .flatten()
        .filter(|a| !a.get("archived").and_then(Value::as_bool).unwrap_or(false))
        .filter_map(|a| {
            let api = a.get("api_name")?.as_str()?.to_string();
            Some(Entry {
                api,
                name: text(a, "name"),
                description: text(a, "description"),
                hidden: a.get("hidden").and_then(Value::as_bool).unwrap_or(false),
                icon: text(a, "icon_achieved"),
                icon_gray: text(a, "icon_unachieved"),
                ..Default::default()
            })
        })
        .collect();
    Some(Schema::from_entries(entries))
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
        let queries = json!({"mutations": [], "queries": [
            {"queryKey": ["AOWarningCookie"], "state": {"data": null}},
            {"queryKey": ["achievements", 400, "schema"], "state": {"data": schema}},
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

    #[test]
    fn a_page_without_data_has_no_schema() {
        assert!(parse_page("<html>nothing</html>").is_none());
        assert!(parse_page(&page(json!(null))).is_none());
    }
}
