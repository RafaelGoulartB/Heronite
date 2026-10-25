//! The Steam client's stats files, in `<steam root>/appcache/stats`:
//! `UserGameStatsSchema_<app>.bin` (the achievements of the game) and
//! `UserGameStats_<account>_<app>.bin` (which of them the account has).
//!
//! In the schema, every stat under `stats` that has `bits` holds up to 32
//! achievements, one per bit. The user file has the same stat ids; its `data`
//! is the bitmask and `AchievementTimes/<bit>` the unlock time.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use super::kv::{self, Value};

/// One achievement of the schema.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entry {
    /// Steam's API name.
    pub api: String,
    pub name: String,
    pub description: String,
    pub hidden: bool,
    /// File name (the client's files) or full URL (the Web API).
    pub icon: String,
    pub icon_gray: String,
    /// Which bit of which stat holds it; `None` for the Web API's schema.
    pub slot: Option<(String, u32)>,
    /// The stat that counts towards it and the value that unlocks it.
    pub progress: Option<(String, u32)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Schema {
    pub entries: Vec<Entry>,
    /// Stat id → stat name, for the stats the achievements count.
    stat_names: HashMap<String, String>,
}

impl Schema {
    pub fn from_entries(entries: Vec<Entry>) -> Self {
        Self { entries, stat_names: HashMap::new() }
    }
}

/// What an account has done in a game.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Unlocks {
    /// API name → when; present means unlocked.
    pub done: HashMap<String, Option<DateTime<Utc>>>,
    /// Stat name → value.
    pub stats: HashMap<String, i64>,
}

/// Where the client keeps the stats files of a Steam root.
pub fn stats_dir(root: &Path) -> PathBuf {
    root.join("appcache").join("stats")
}

pub fn schema_file(root: &Path, app_id: &str) -> PathBuf {
    stats_dir(root).join(format!("UserGameStatsSchema_{app_id}.bin"))
}

pub fn user_file(root: &Path, account: &str, app_id: &str) -> PathBuf {
    stats_dir(root).join(format!("UserGameStats_{account}_{app_id}.bin"))
}

/// The language keys of the schema to try, best first, for the interface
/// language. `english` always closes the list.
pub fn language_keys(lang: &str) -> Vec<&'static str> {
    let lang = lang.to_ascii_lowercase();
    let mut keys: Vec<&'static str> = Vec::new();
    if lang.starts_with("pt") {
        keys.extend(["brazilian", "portuguese"]);
    } else if lang.starts_with("es") {
        keys.extend(["latam", "spanish"]);
    } else if lang.starts_with("de") {
        keys.push("german");
    }
    keys.push("english");
    keys
}

/// The Web API's `l=` value for the interface language.
pub fn web_language(lang: &str) -> &'static str {
    language_keys(lang)[0]
}

/// The text of a `{language: text}` group, in the best language it has.
fn localized(group: Option<&Value>, keys: &[&str]) -> String {
    let Some(group) = group else { return String::new() };
    if let Some(text) = group.as_str() {
        return text.to_string();
    }
    let pick = |key: &str| group.get(key).and_then(Value::as_str).filter(|t| !t.is_empty());
    keys.iter()
        .find_map(|key| pick(key))
        .or_else(|| group.children().iter().filter(|(k, _)| k != "token").find_map(|(_, v)| v.as_str().filter(|t| !t.is_empty())))
        .unwrap_or_default()
        .to_string()
}

fn text_of(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).unwrap_or_default().to_string()
}

/// The schema out of `UserGameStatsSchema_<app>.bin`, with the texts in the
/// interface language.
pub fn parse_schema(bytes: &[u8], lang: &str) -> Result<Schema, String> {
    let doc = kv::parse(bytes)?;
    let keys = language_keys(lang);
    let stats = doc.children().first().and_then(|(_, game)| game.get("stats")).ok_or("no stats in the schema")?;
    let mut schema = Schema::default();
    for (stat_id, stat) in stats.children() {
        if let (Some(name), None) = (stat.get("name").and_then(Value::as_str), stat.get("bits")) {
            schema.stat_names.insert(stat_id.clone(), name.to_string());
        }
        let Some(bits) = stat.get("bits") else { continue };
        for (bit, item) in bits.children() {
            let (Ok(bit), Some(api)) = (bit.parse::<u32>(), item.get("name").and_then(Value::as_str)) else { continue };
            let display = item.get("display");
            let progress = item.get("progress").and_then(|p| {
                let max = u32::try_from(p.get("max_val")?.as_i64()?).ok().filter(|m| *m > 0)?;
                let value = p.get("value")?;
                (value.get("operation")?.as_str()? == "statvalue").then_some(())?;
                Some((value.get("operand1")?.as_str()?.to_string(), max))
            });
            schema.entries.push(Entry {
                api: api.to_string(),
                name: localized(display.and_then(|d| d.get("name")), &keys),
                description: localized(display.and_then(|d| d.get("desc")), &keys),
                hidden: display.and_then(|d| d.get("hidden")).and_then(Value::as_i64).is_some_and(|h| h != 0),
                icon: text_of(display.and_then(|d| d.get("icon"))),
                icon_gray: text_of(display.and_then(|d| d.get("icon_gray"))),
                slot: Some((stat_id.clone(), bit)),
                progress,
            });
        }
    }
    Ok(schema)
}

/// What `UserGameStats_<account>_<app>.bin` says, for the entries of
/// `schema`.
pub fn parse_user(bytes: &[u8], schema: &Schema) -> Result<Unlocks, String> {
    let doc = kv::parse(bytes)?;
    let stats = doc.children().first().map(|(_, cache)| cache).ok_or("no stats in the user file")?;
    let mut unlocks = Unlocks::default();
    for entry in &schema.entries {
        let Some((stat_id, bit)) = &entry.slot else { continue };
        let Some(stat) = stats.get(stat_id) else { continue };
        // The mask is a signed int32 in the file; the bit is what counts.
        let mask = stat.get("data").and_then(Value::as_i64).unwrap_or(0) as u32;
        if *bit < 32 && mask & (1 << bit) != 0 {
            let at = stat.get("AchievementTimes").and_then(|t| t.get(&bit.to_string())).and_then(Value::as_i64);
            unlocks.done.insert(entry.api.clone(), at.filter(|s| *s > 0).and_then(|s| DateTime::from_timestamp(s, 0)));
        }
    }
    for (stat_id, name) in &schema.stat_names {
        if let Some(value) = stats.get(stat_id).and_then(|s| s.get("data")).and_then(Value::as_i64) {
            unlocks.stats.insert(name.clone(), value);
        }
    }
    Ok(unlocks)
}

#[cfg(test)]
mod tests {
    use super::super::kv::build::*;
    use super::*;

    fn names(pairs: &[(&str, &str)]) -> Vec<u8> {
        group("name", &pairs.iter().map(|(k, v)| string(k, v)).collect::<Vec<_>>())
    }

    pub(crate) fn schema_bytes() -> Vec<u8> {
        let first = group(
            "0",
            &[
                string("name", "ACH.FIRST"),
                group(
                    "display",
                    &[
                        names(&[
                            ("english", "First"),
                            ("brazilian", "Primeira"),
                            ("LATAM", "Primera"),
                            ("spanish", "Primera (ES)"),
                            ("token", "T_NAME"),
                        ]),
                        group("desc", &[string("english", "Do it"), string("brazilian", "Faca isso")]),
                        int("hidden", 0),
                        string("icon", "first.jpg"),
                        string("icon_gray", "first_bw.jpg"),
                    ],
                ),
            ],
        );
        let counted = group(
            "3",
            &[
                string("name", "ACH.COUNT"),
                group(
                    "display",
                    &[
                        names(&[("english", "Count")]),
                        group("desc", &[string("english", "Do it 10 times")]),
                        int("hidden", 1),
                        string("icon", "count.jpg"),
                    ],
                ),
                group(
                    "progress",
                    &[
                        int("min_val", 0),
                        int("max_val", 10),
                        group("value", &[string("operation", "statvalue"), string("operand1", "runs")]),
                    ],
                ),
            ],
        );
        let achievements = group("1", &[group("bits", &[first, counted]), string("type", "ACHIEVEMENTS")]);
        let runs = group("2", &[string("type", "INT"), string("name", "runs"), group("display", &[string("name", "Runs")])]);
        file(group("620", &[string("gamename", "Test"), group("stats", &[achievements, runs])]))
    }

    pub(crate) fn user_bytes() -> Vec<u8> {
        let ach = group("1", &[int("data", -2147483639), group("AchievementTimes", &[int("3", 1_700_000_000), int("0", 0)])]);
        let runs = group("2", &[int("data", 4)]);
        file(group("cache", &[int("crc", 1), ach, runs]))
    }

    #[test]
    fn schema_picks_the_interface_language() {
        let bytes = schema_bytes();
        let pt = parse_schema(&bytes, "pt-BR").unwrap();
        assert_eq!(pt.entries.len(), 2);
        let first = &pt.entries[0];
        assert_eq!((first.api.as_str(), first.name.as_str(), first.description.as_str()), ("ACH.FIRST", "Primeira", "Faca isso"));
        assert_eq!((first.icon.as_str(), first.icon_gray.as_str(), first.hidden), ("first.jpg", "first_bw.jpg", false));
        assert_eq!(first.slot, Some(("1".into(), 0)));
        assert_eq!(parse_schema(&bytes, "es").unwrap().entries[0].name, "Primera");
        let en = parse_schema(&bytes, "en").unwrap();
        assert_eq!(en.entries[0].name, "First");
        let counted = &en.entries[1];
        assert_eq!((counted.hidden, counted.icon_gray.as_str(), counted.progress.clone()), (true, "", Some(("runs".into(), 10))));
    }

    #[test]
    fn a_missing_language_falls_back_to_english_then_to_anything() {
        let schema = parse_schema(&schema_bytes(), "pt").unwrap();
        assert_eq!(schema.entries[1].name, "Count");
        let only = group("name", &[string("token", "T"), string("german", "Nur")]);
        assert_eq!(localized(Some(&kv::parse(&only).unwrap().children()[0].1), &["english"]), "Nur");
    }

    #[test]
    fn user_file_gives_unlocks_times_and_counters() {
        let schema = parse_schema(&schema_bytes(), "en").unwrap();
        let unlocks = parse_user(&user_bytes(), &schema).unwrap();
        assert_eq!(unlocks.done.len(), 2);
        assert_eq!(unlocks.done["ACH.COUNT"].unwrap().timestamp(), 1_700_000_000);
        assert_eq!(unlocks.done["ACH.FIRST"], None);
        assert_eq!(unlocks.stats["runs"], 4);
    }

    #[test]
    fn language_choice() {
        assert_eq!(language_keys("pt-BR"), ["brazilian", "portuguese", "english"]);
        assert_eq!(language_keys("es-MX"), ["latam", "spanish", "english"]);
        assert_eq!(language_keys("de"), ["german", "english"]);
        assert_eq!(language_keys("en"), ["english"]);
        assert_eq!(web_language("es"), "latam");
    }
}
