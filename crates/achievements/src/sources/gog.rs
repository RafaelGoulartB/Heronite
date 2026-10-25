//! GOG: `gameplay.gog.com`, with the signed-in user's token (gogdl
//! refreshes it). Heroic's `getAchievements` reads the same endpoint.

use chrono::{DateTime, Utc};
use heronite_data::paths::DataPaths;
use heronite_gog::cmd::Gogdl;
use serde::Deserialize;
use serde_json::Value;

use super::{Ctx, Fetch};
use crate::model::{Achievement, AchievementSet, Source};

pub const GAMEPLAY_BASE: &str = "https://gameplay.gog.com";

/// One item of the endpoint's `items`.
#[derive(Debug, Deserialize)]
struct Item {
    #[serde(default)]
    achievement_id: String,
    #[serde(default)]
    achievement_key: String,
    #[serde(default)]
    visible: bool,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    image_url_unlocked: String,
    #[serde(default)]
    image_url_locked: String,
    #[serde(default)]
    rarity: Option<f64>,
    #[serde(default)]
    date_unlocked: Option<String>,
}

/// GOG's `2026-01-01T00:00:00+0000`.
fn parse_date(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%z")
        .or_else(|_| DateTime::parse_from_rfc3339(text))
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// The set out of the endpoint's answer.
pub fn parse(product_id: &str, doc: &Value) -> Option<AchievementSet> {
    let items: Vec<Item> = serde_json::from_value(doc.get("items")?.clone()).ok()?;
    let achievements = items
        .into_iter()
        .map(|i| {
            let unlocked_at = i.date_unlocked.as_deref().filter(|d| !d.is_empty()).and_then(parse_date);
            let unlocked = i.date_unlocked.as_deref().is_some_and(|d| !d.is_empty());
            Achievement {
                id: if i.achievement_key.is_empty() { i.achievement_id } else { i.achievement_key },
                name: i.name,
                description: i.description,
                hidden: !i.visible,
                icon: i.image_url_unlocked,
                icon_locked: i.image_url_locked,
                unlocked,
                unlocked_at,
                rarity: i.rarity.map(|r| r as f32),
                ..Default::default()
            }
        })
        .collect();
    Some(AchievementSet { source: Source::Gog, external_id: product_id.into(), title: None, fetched_at: Utc::now(), achievements })
}

/// `X-Gog-Lc` for the interface language.
fn locale(lang: &str) -> &'static str {
    match lang {
        l if l.starts_with("pt") => "pt-BR",
        l if l.starts_with("es") => "es-ES",
        _ => "en-US",
    }
}

/// A fresh token and the user id: gogdl refreshes an expired token.
fn credentials(paths: &DataPaths) -> Result<(String, String), String> {
    let gogdl = Gogdl::locate(paths).map_err(|e| e.to_string())?;
    let c = heronite_gog::user::credentials(paths, &gogdl).map_err(|e| format!("{e:#}"))?;
    Ok((c.access_token, c.user_id))
}

pub fn fetch(ctx: &Ctx, product_id: &str) -> Fetch {
    fetch_from(ctx, GAMEPLAY_BASE, product_id, credentials)
}

fn fetch_from(ctx: &Ctx, base: &str, product_id: &str, creds: impl FnOnce(&DataPaths) -> Result<(String, String), String>) -> Fetch {
    if !heronite_gog::user::is_logged_in(ctx.paths) {
        return Fetch::NeedsAccount;
    }
    let (token, user_id) = match creds(ctx.paths) {
        Ok(c) => c,
        Err(e) => return Fetch::Failed(e),
    };
    let url = format!("{base}/clients/{product_id}/users/{user_id}/achievements");
    let response = ctx.http.get(url).header("Authorization", &format!("Bearer {token}")).header("X-Gog-Lc", locale(ctx.lang)).send();
    match response {
        Ok(r) if r.ok() => match r.json::<Value>().ok().as_ref().and_then(|doc| parse(product_id, doc)) {
            Some(set) if set.achievements.is_empty() => Fetch::None,
            Some(set) => Fetch::Set(set),
            None => Fetch::Failed("GOG answered something unexpected".into()),
        },
        Ok(r) => Fetch::Failed(format!("GOG: HTTP {}", r.status)),
        Err(e) => Fetch::Failed(format!("GOG: {e:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_the_gameplay_answer() {
        let doc = json!({"total_count": 2, "items": [
            {"achievement_id":"1","achievement_key":"WIN","visible":true,"name":"Win","description":"Win it",
             "image_url_unlocked":"u","image_url_locked":"l","rarity":12.5,"date_unlocked":"2026-01-02T03:04:05+0000"},
            {"achievement_id":"2","achievement_key":"","visible":false,"name":"Secret","description":"",
             "image_url_unlocked":"u2","image_url_locked":"l2","rarity":1.0,"date_unlocked":null}]});
        let set = parse("42", &doc).unwrap();
        assert_eq!(set.external_id, "42");
        let (a, b) = (&set.achievements[0], &set.achievements[1]);
        assert_eq!((a.id.as_str(), a.unlocked, a.hidden, a.rarity), ("WIN", true, false, Some(12.5)));
        assert_eq!(a.unlocked_at.unwrap().to_rfc3339(), "2026-01-02T03:04:05+00:00");
        assert_eq!((b.id.as_str(), b.unlocked, b.hidden, b.icon_locked.as_str()), ("2", false, true, "l2"));
    }

    #[test]
    fn locales() {
        assert_eq!(locale("pt-BR"), "pt-BR");
        assert_eq!(locale("es"), "es-ES");
        assert_eq!(locale("en"), "en-US");
    }
}
