//! The files of the open-source Steam API emulators (Goldberg's and its
//! maintained fork, gbe_fork), read only:
//!
//! - the unlock state, `<saves>/<app id>/achievements.json`: an object keyed
//!   by each achievement's Steam API name, with whether it is earned, when
//!   (Unix seconds; 0 when unknown) and, in the fork, the progress of a
//!   counted one;
//! - the list of the game's achievements the emulator was set up with,
//!   `steam_settings/achievements.json`: an array of the API name, the name
//!   and description (one text, or one per language), whether it is hidden,
//!   the icons (files of `steam_settings`, or Steam's file names) and,
//!   sometimes, the share of players that have it.

use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::sources::steam::local::{language_keys, Entry, Schema, Unlocks};

/// What the state file says of one achievement.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Earned {
    pub earned: bool,
    pub at: Option<DateTime<Utc>>,
    /// Current and maximum of a counted one.
    pub progress: Option<(u32, u32)>,
}

fn number(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        _ => None,
    }
}

fn flag(v: Option<&Value>) -> bool {
    number(v).is_some_and(|n| n != 0.)
}

/// The unlock state, by API name.
pub fn parse_state(bytes: &[u8]) -> Result<HashMap<String, Earned>, String> {
    let doc: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let obj = doc.as_object().ok_or("not an object")?;
    let mut out = HashMap::new();
    for (api, v) in obj {
        let earned = flag(v.get("earned"));
        let at =
            number(v.get("earned_time")).filter(|t| *t > 0.).and_then(|t| DateTime::<Utc>::from_timestamp(t as i64, 0)).filter(|_| earned);
        let progress = match (number(v.get("progress")), number(v.get("max_progress"))) {
            (Some(cur), Some(max)) if max > 0. => Some((cur.clamp(0., max) as u32, max as u32)),
            _ => None,
        };
        out.insert(api.clone(), Earned { earned, at, progress });
    }
    Ok(out)
}

/// What is unlocked, by the schema's API names: an exact match first, then
/// one that only differs in case. Names the schema does not have are left out.
pub fn unlocks(state: &HashMap<String, Earned>, schema: &Schema) -> Unlocks {
    let lower: HashMap<String, &Earned> = state.iter().map(|(k, v)| (k.to_lowercase(), v)).collect();
    let mut out = Unlocks::default();
    for entry in &schema.entries {
        let Some(e) = state.get(&entry.api).or_else(|| lower.get(&entry.api.to_lowercase()).copied()) else { continue };
        if e.earned {
            out.done.insert(entry.api.clone(), e.at);
        }
        if let (Some((stat, _)), Some((current, _))) = (&entry.progress, e.progress) {
            out.stats.insert(stat.clone(), i64::from(current));
        }
    }
    let unknown = state.keys().filter(|k| !schema.entries.iter().any(|e| e.api.eq_ignore_ascii_case(k))).count();
    if unknown > 0 {
        log::debug!("game files: {unknown} unlocks the list of achievements does not have");
    }
    out
}

/// A text of the schema: a plain one, or the one of the interface language
/// (English, then any, when it is missing).
fn text(v: Option<&Value>, lang: &str) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Object(map)) => language_keys(lang)
            .into_iter()
            .find_map(|k| map.get(k).and_then(Value::as_str))
            .or_else(|| map.values().find_map(Value::as_str))
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    }
}

const IMAGES: [&str; 5] = ["jpg", "jpeg", "png", "webp", "gif"];

/// An icon of the schema: a file inside `settings` (never outside it), as
/// an absolute path; else Steam's file name, for its CDN; else nothing.
fn icon(v: Option<&Value>, settings: &Path) -> String {
    let Some(name) = v.and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) else { return String::new() };
    let rel = Path::new(name);
    let safe = !rel.is_absolute() && rel.components().all(|c| matches!(c, std::path::Component::Normal(_)));
    let image = rel.extension().and_then(|e| e.to_str()).is_some_and(|e| IMAGES.contains(&e.to_ascii_lowercase().as_str()));
    if !safe || !image {
        return String::new();
    }
    let file = rel.components().fold(settings.to_path_buf(), |p, c| p.join(c));
    if file.is_file() {
        return file.to_string_lossy().into_owned();
    }
    // A bare file name is Steam's (the CDN has it).
    if rel.components().count() == 1 {
        return name.to_string();
    }
    String::new()
}

/// The emulator's list of the game's achievements, and the share of players
/// of each when it has it. `settings` is its `steam_settings` folder.
pub fn parse_schema(bytes: &[u8], lang: &str, settings: &Path) -> Result<(Schema, HashMap<String, f32>), String> {
    let doc: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let list = doc.as_array().ok_or("not a list")?;
    let mut entries = Vec::new();
    let mut rarity = HashMap::new();
    for v in list {
        let Some(api) = v.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()) else { continue };
        if let Some(p) = number(v.get("unlock_percentage")).filter(|p| (0. ..=100.).contains(p)) {
            rarity.insert(api.to_string(), p as f32);
        }
        let name = text(v.get("displayName"), lang);
        entries.push(Entry {
            api: api.to_string(),
            name: if name.is_empty() { api.to_string() } else { name },
            description: text(v.get("description"), lang),
            hidden: flag(v.get("hidden")),
            icon: icon(v.get("icon"), settings),
            icon_gray: icon(v.get("icon_gray").or_else(|| v.get("icongray")), settings),
            ..Default::default()
        });
    }
    Ok((Schema::from_entries(entries), rarity))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_gives_unlocks_times_and_progress() {
        let state = parse_state(
            br#"{
                "WIN": {"earned": true, "earned_time": 1700000000},
                "NO_TIME": {"earned": 1, "earned_time": 0},
                "LOCKED": {"earned": false, "earned_time": 0},
                "COUNT": {"earned": false, "earned_time": 0, "progress": 7, "max_progress": 10}
            }"#,
        )
        .unwrap();
        assert_eq!(state["WIN"].at.unwrap().timestamp(), 1_700_000_000);
        assert!(state["NO_TIME"].earned && state["NO_TIME"].at.is_none());
        assert!(!state["LOCKED"].earned);
        assert_eq!(state["COUNT"].progress, Some((7, 10)));
        assert!(parse_state(b"[]").is_err());
        assert!(parse_state(b"{").is_err());
    }

    #[test]
    fn unlocks_match_names_exactly_then_ignoring_case() {
        let state = parse_state(br#"{"Win": {"earned": true, "earned_time": 5}, "ghost": {"earned": true}}"#).unwrap();
        let schema = Schema::from_entries(vec![
            Entry { api: "WIN".into(), ..Default::default() },
            Entry { api: "OTHER".into(), ..Default::default() },
        ]);
        let u = unlocks(&state, &schema);
        assert_eq!(u.done.len(), 1);
        assert_eq!(u.done["WIN"].unwrap().timestamp(), 5);
    }

    #[test]
    fn schema_reads_languages_icons_hidden_and_rarity() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("steam_settings");
        std::fs::create_dir_all(settings.join("img")).unwrap();
        std::fs::write(settings.join("img/win.jpg"), b"x").unwrap();
        let (schema, rarity) = parse_schema(
            br#"[
                {"name": "WIN", "displayName": {"english": "Winner", "german": "Sieger"}, "description": "Win.",
                 "hidden": "1", "icon": "img/win.jpg", "icon_gray": "../outside.jpg", "unlock_percentage": 3.5},
                {"name": "CDN", "displayName": "From the CDN", "hidden": 0, "icon": "abc123.jpg", "icongray": "/etc/passwd"},
                {"displayName": "no api name"}
            ]"#,
            "de",
            &settings,
        )
        .unwrap();
        assert_eq!(schema.entries.len(), 2);
        let win = &schema.entries[0];
        assert_eq!((win.name.as_str(), win.description.as_str(), win.hidden), ("Sieger", "Win.", true));
        assert_eq!(Path::new(&win.icon), settings.join("img/win.jpg"));
        assert_eq!(win.icon_gray, "", "never a file outside steam_settings");
        let cdn = &schema.entries[1];
        assert_eq!((cdn.icon.as_str(), cdn.icon_gray.as_str(), cdn.hidden), ("abc123.jpg", "", false));
        assert_eq!(rarity["WIN"], 3.5);
        assert!(!rarity.contains_key("CDN"));
    }
}
