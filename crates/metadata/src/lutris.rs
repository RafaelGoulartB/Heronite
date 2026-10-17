//! Lutris provider: port of `fetchLutrisMetadata`.

use heronite_data::models::CollectionGameMetadata;
use serde_json::Value;

use crate::http::Http;
use crate::types::Lookup;
use crate::util::{blank_metadata, clip, pick_best_match, strip_html, title_queries, unique_names, unique_names12, MIN_MATCH_SCORE};

fn named_list(v: Option<&Value>) -> Vec<String> {
    let items: Vec<&str> = v
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|i| match i {
                    Value::String(s) => Some(s.as_str()),
                    other => other.get("name").and_then(Value::as_str),
                })
                .collect()
        })
        .unwrap_or_default();
    unique_names12(items)
}

fn s<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|x| !x.is_empty())
}

/// Maps a `/api/games/{slug}` response. `None` when it has no name.
pub fn map_lutris_detail(app_name: &str, runner: &str, slug: &str, game: &Value) -> Option<CollectionGameMetadata> {
    let name = s(game, "name")?;
    let cover = s(game, "coverart").or_else(|| s(game, "cover_url"));
    let hero = s(game, "banner_url").or_else(|| s(game, "background_url"));
    // `game.year ? String(game.year) : undefined`
    let year = match game.get("year") {
        Some(Value::Number(n)) if n.as_f64().is_some_and(|y| y != 0.0) => Some(n.to_string()),
        Some(Value::String(y)) if !y.is_empty() => Some(y.clone()),
        _ => None,
    };
    let http_only = |u: Option<&str>| u.filter(|u| u.starts_with("http")).map(str::to_owned);
    let mut md = blank_metadata(app_name, runner);
    md.lutris_slug = Some(s(game, "slug").unwrap_or(slug).to_owned());
    md.title = Some(name.to_owned());
    md.description = clip(Some(&strip_html(game.get("description").and_then(Value::as_str))));
    md.release_date = year;
    md.platforms = named_list(game.get("platforms"));
    md.genres = named_list(game.get("genres"));
    md.websites = unique_names([s(game, "website")], 2);
    md.cover_url = http_only(cover);
    md.hero_url = http_only(hero);
    Some(md)
}

/// `fetchLutrisMetadata`: by known slug, else by searching the title.
pub fn fetch_lutris_metadata(
    http: &Http,
    app_name: &str,
    runner: &str,
    title: &str,
    lutris_slug: Option<&str>,
) -> Lookup<CollectionGameMetadata> {
    let run = || -> anyhow::Result<Option<CollectionGameMetadata>> {
        let mut slug = lutris_slug.filter(|s| !s.is_empty()).map(str::to_owned);
        if slug.is_none() {
            for query in title_queries(title) {
                let resp = http
                    .get(format!("{}/api/games", http.endpoints.lutris))
                    .query(&[("search", query.as_str())])
                    .timeout_secs(12)
                    .send_ok()?;
                let body: Value = resp.json()?;
                let results: Vec<Value> = body.get("results").and_then(Value::as_array).cloned().unwrap_or_default();
                let best =
                    pick_best_match(title, &results, |r| r.get("name").and_then(Value::as_str).unwrap_or("").to_owned(), MIN_MATCH_SCORE);
                if let Some(found) = best.and_then(|b| s(b, "slug")) {
                    slug = Some(found.to_owned());
                    break;
                }
            }
        }
        let Some(slug) = slug else { return Ok(None) };
        let detail: Value = http.get(format!("{}/api/games/{slug}", http.endpoints.lutris)).timeout_secs(15).send_ok()?.json()?;
        Ok(map_lutris_detail(app_name, runner, &slug, &detail))
    };
    match run() {
        Ok(Some(md)) => Lookup::Found(md),
        Ok(None) => Lookup::NotFound,
        Err(e) => {
            log::warn!("Lutris metadata failed for {title}: {e}");
            Lookup::from_error(&e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        serde_json::from_str(&std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap()
    }

    #[test]
    fn maps_recorded_lutris_detail() {
        let detail = fixture("lutris_detail_portal.json");
        let md = map_lutris_detail("steam_400", "sideload", "portal", &detail).unwrap();
        assert_eq!(md.lutris_slug.as_deref(), Some("portal"));
        assert_eq!(md.title.as_deref(), Some("Portal"));
        assert_eq!(md.release_date.as_deref(), Some("2007"));
        assert!(md.genres.contains(&"Puzzle".to_string()));
        assert!(md.platforms.contains(&"Linux".to_string()));
        assert!(md.cover_url.unwrap().starts_with("https://"));
        assert!(md.hero_url.unwrap().starts_with("https://"));
        assert!(md.description.is_some());
    }

    #[test]
    fn search_picks_the_exact_title() {
        let list = fixture("lutris_search_portal.json");
        let results: Vec<Value> = list["results"].as_array().unwrap().clone();
        let best = pick_best_match("Portal", &results, |r| r["name"].as_str().unwrap_or("").into(), MIN_MATCH_SCORE).unwrap();
        assert_eq!(best["slug"], "portal");
    }
}
