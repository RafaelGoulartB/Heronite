//! RetroAchievements' Web API (https://api-docs.retroachievements.org):
//! the account check, the game list of a console (with the hashes of its
//! games, cached on disk) and a game's achievements with the user's progress.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use heronite_data::paths::DataPaths;
use heronite_metadata::Http;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::consoles;
use crate::accounts::RetroAccount;
use crate::model::{Achievement, AchievementSet, Kind, Source};
use crate::sources::Ctx;

pub const BASE: &str = "https://retroachievements.org/API/";
pub const MEDIA: &str = "https://media.retroachievements.org";

/// The site asks for an identifiable client.
const USER_AGENT: &str = concat!("Heronite/", env!("CARGO_PKG_VERSION"));

/// How long a console's game list is trusted: it changes slowly and is big.
const LIST_MAX_AGE_DAYS: i64 = 7;

/// A request to `endpoint` with the key; the answer as JSON.
fn get(http: &Http, base: &str, account: &RetroAccount, endpoint: &str, params: &[(&str, &str)], big: bool) -> Result<Value> {
    let mut all = vec![("y", account.api_key.as_str())];
    all.extend_from_slice(params);
    let mut request = http.get(format!("{base}{endpoint}")).query(&all).header("User-Agent", USER_AGENT);
    if big {
        request = request.timeout_secs(120).max_bytes(128 * 1024 * 1024);
    }
    let response = request.send().map_err(|e| anyhow!(scrub(&format!("{e:#}"), &account.api_key)))?;
    match response.status {
        s if (200..300).contains(&s) => response.json().map_err(|e| anyhow!("RetroAchievements answered something unexpected ({e:#})")),
        401 | 403 => bail!("RetroAchievements rejected the username or the API key"),
        404 => bail!("RetroAchievements does not know it"),
        429 => bail!("RetroAchievements asks to slow down (HTTP 429)"),
        s => bail!("RetroAchievements: HTTP {s}"),
    }
}

/// The key is part of every URL: an error message must not carry it.
fn scrub(text: &str, key: &str) -> String {
    if key.is_empty() {
        text.to_owned()
    } else {
        text.replace(key, "***")
    }
}

/// The user's public profile, as Settings shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub username: String,
    /// `TotalPoints`: the points won in hardcore mode.
    pub hardcore_points: u32,
    /// `TotalSoftcorePoints`.
    pub softcore_points: u32,
    /// `TotalTruePoints`: the site's weighted score (RetroPoints).
    pub true_points: u32,
    pub avatar_url: String,
}

/// `API_GetUserProfile`'s answer.
pub fn parse_profile(doc: &Value) -> Option<Profile> {
    let username = doc.get("User")?.as_str().filter(|u| !u.is_empty())?.to_owned();
    let number = |key: &str| doc.get(key).and_then(Value::as_u64).unwrap_or(0).min(u32::MAX as u64) as u32;
    let avatar = doc.get("UserPic").and_then(Value::as_str).unwrap_or_default();
    Some(Profile {
        avatar_url: if avatar.is_empty() { String::new() } else { format!("{MEDIA}{avatar}") },
        hardcore_points: number("TotalPoints"),
        softcore_points: number("TotalSoftcorePoints"),
        true_points: number("TotalTruePoints"),
        username,
    })
}

/// Checks a username and Web API key (Settings): the profile, or why not.
pub fn validate_account(http: &Http, username: &str, api_key: &str) -> Result<Profile> {
    validate_account_at(http, BASE, username, api_key)
}

fn validate_account_at(http: &Http, base: &str, username: &str, api_key: &str) -> Result<Profile> {
    let account = RetroAccount { username: username.trim().to_owned(), api_key: api_key.trim().to_owned() };
    let doc = get(http, base, &account, "API_GetUserProfile.php", &[("u", account.username.as_str())], false)?;
    parse_profile(&doc).ok_or_else(|| anyhow!("RetroAchievements does not know the user {}", account.username))
}

/// One game of a console's list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listed {
    pub id: u32,
    pub title: String,
    #[serde(default)]
    pub achievements: u32,
    /// Lowercase MD5s of the ROMs the site accepts for it.
    #[serde(default)]
    pub hashes: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListFile {
    fetched_at: DateTime<Utc>,
    games: Vec<Listed>,
}

/// `API_GetGameList`'s answer.
pub fn parse_game_list(doc: &Value) -> Option<Vec<Listed>> {
    let items = doc.as_array()?;
    Some(
        items
            .iter()
            .filter_map(|g| {
                Some(Listed {
                    id: u32::try_from(g.get("ID")?.as_u64()?).ok()?,
                    title: g.get("Title")?.as_str()?.to_owned(),
                    achievements: g.get("NumAchievements").and_then(Value::as_u64).unwrap_or(0) as u32,
                    hashes: g
                        .get("Hashes")
                        .and_then(Value::as_array)
                        .map(|h| h.iter().filter_map(Value::as_str).map(str::to_ascii_lowercase).collect())
                        .unwrap_or_default(),
                })
            })
            .collect(),
    )
}

fn list_file(paths: &DataPaths, console: u32) -> PathBuf {
    paths.store_cache(&format!("ra_games_{console}"))
}

pub fn is_fresh(fetched_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    now.signed_duration_since(fetched_at) < Duration::days(LIST_MAX_AGE_DAYS)
}

fn read_list(file: &Path) -> Option<ListFile> {
    heronite_data::json_store::read::<Option<ListFile>>(file).ok().flatten()
}

/// The games of a console that have achievements, with their hashes: from
/// the cache while it is fresh, else from the site (a stale cache is used
/// when the site cannot be reached).
pub fn game_list(ctx: &Ctx, console: u32) -> Result<Vec<Listed>> {
    game_list_at(ctx, BASE, console, Utc::now())
}

pub(super) fn game_list_at(ctx: &Ctx, base: &str, console: u32, now: DateTime<Utc>) -> Result<Vec<Listed>> {
    let file = list_file(ctx.paths, console);
    let cached = read_list(&file);
    if let Some(c) = cached.as_ref().filter(|c| is_fresh(c.fetched_at, now)) {
        return Ok(c.games.clone());
    }
    let fresh = fetch_list(ctx, base, console).and_then(|games| {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        heronite_data::json_store::write(&file, &ListFile { fetched_at: now, games: games.clone() })?;
        Ok(games)
    });
    match (fresh, cached) {
        (Ok(games), _) => Ok(games),
        (Err(e), Some(stale)) => {
            log::warn!("RetroAchievements game list of console {console}: {e:#}; using the cached one");
            Ok(stale.games)
        }
        (Err(e), None) => Err(e),
    }
}

fn fetch_list(ctx: &Ctx, base: &str, console: u32) -> Result<Vec<Listed>> {
    let account = ctx.accounts.retro_achievements.as_ref().ok_or_else(|| anyhow!("no RetroAchievements account"))?;
    let console_id = console.to_string();
    let doc = get(ctx.http, base, account, "API_GetGameList.php", &[("i", &console_id), ("h", "1"), ("f", "1")], true)?;
    parse_game_list(&doc).context("RetroAchievements sent a game list in an unknown shape")
}

/// A game picked from the lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameHit {
    pub id: u32,
    pub title: String,
    pub console_id: u32,
    /// RA's name for the console.
    pub console: String,
}

/// Words of a title, lowercase, without punctuation.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// Games of `console_ids` (the cached or fetched lists) whose title has every
/// word of `query`, the closest titles first. An empty `console_ids` searches
/// the lists already on disk.
pub fn search_games(ctx: &Ctx, console_ids: &[u32], query: &str) -> Vec<GameHit> {
    let wanted = words(query);
    if wanted.is_empty() {
        return Vec::new();
    }
    let consoles = if console_ids.is_empty() { cached_consoles(ctx.paths) } else { console_ids.to_vec() };
    let mut hits: Vec<(usize, GameHit)> = Vec::new();
    for console in consoles {
        let Ok(list) = game_list(ctx, console) else { continue };
        for game in list {
            let title_words = words(&game.title);
            if wanted.iter().all(|w| title_words.iter().any(|t| t.contains(w.as_str()))) {
                // Fewer extra words = closer.
                let rank = title_words.len().saturating_sub(wanted.len());
                hits.push((rank, GameHit { id: game.id, title: game.title, console_id: console, console: consoles::name(console) }));
            }
        }
    }
    hits.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.title.cmp(&b.1.title)));
    hits.into_iter().map(|(_, h)| h).take(50).collect()
}

/// Consoles that have a game list on disk.
fn cached_consoles(paths: &DataPaths) -> Vec<u32> {
    let dir = list_file(paths, 0).parent().map(Path::to_path_buf).unwrap_or_default();
    let mut out: Vec<u32> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_prefix("ra_games_")?.strip_suffix(".json")?.parse().ok())
        .collect();
    out.sort_unstable();
    out
}

/// A game's achievements with the user's progress
/// (`API_GetGameInfoAndUserProgress`).
pub fn game_progress(ctx: &Ctx, base: &str, account: &RetroAccount, game_id: u32) -> Result<AchievementSet> {
    let id = game_id.to_string();
    let doc =
        get(ctx.http, base, account, "API_GetGameInfoAndUserProgress.php", &[("g", &id), ("u", &account.username), ("a", "1")], false)?;
    parse_set(&doc, game_id, Utc::now()).context("RetroAchievements sent a game in an unknown shape")
}

/// `2026-01-02 03:04:05`, UTC.
fn parse_date(text: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").ok().map(|d| d.and_utc())
}

fn text(doc: &Value, key: &str) -> String {
    doc.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn kind_of(name: &str) -> Option<Kind> {
    match name {
        "progression" => Some(Kind::Progression),
        "win_condition" => Some(Kind::WinCondition),
        "missable" => Some(Kind::Missable),
        _ => None,
    }
}

/// The set out of the answer. Only core achievements come in it. A game
/// without achievements has `"Achievements": []`.
pub fn parse_set(doc: &Value, game_id: u32, now: DateTime<Utc>) -> Option<AchievementSet> {
    let players = ["NumDistinctPlayers", "NumDistinctPlayersCasual"].iter().find_map(|k| doc.get(*k).and_then(Value::as_f64)).unwrap_or(0.);
    let mut items: Vec<(i64, u64, Achievement)> = match doc.get("Achievements")? {
        Value::Object(map) => map.values().filter_map(|a| parse_achievement(a, players)).collect(),
        _ => Vec::new(),
    };
    items.sort_by_key(|(order, id, _)| (*order, *id));
    Some(AchievementSet {
        source: Source::RetroAchievements,
        external_id: game_id.to_string(),
        title: Some(text(doc, "Title")).filter(|t| !t.is_empty()),
        fetched_at: now,
        achievements: items.into_iter().map(|(_, _, a)| a).collect(),
    })
}

fn parse_achievement(a: &Value, players: f64) -> Option<(i64, u64, Achievement)> {
    let id = a.get("ID")?.as_u64()?;
    let date = |key: &str| a.get(key).and_then(Value::as_str).filter(|d| !d.is_empty());
    let (earned, earned_hardcore) = (date("DateEarned"), date("DateEarnedHardcore"));
    let badge = text(a, "BadgeName");
    let awarded = a.get("NumAwarded").and_then(Value::as_f64).unwrap_or(0.);
    Some((
        a.get("DisplayOrder").and_then(Value::as_i64).unwrap_or(0),
        id,
        Achievement {
            id: id.to_string(),
            name: text(a, "Title"),
            description: text(a, "Description"),
            icon: format!("{MEDIA}/Badge/{badge}.png"),
            icon_locked: format!("{MEDIA}/Badge/{badge}_lock.png"),
            unlocked: earned.is_some() || earned_hardcore.is_some(),
            unlocked_at: earned.into_iter().chain(earned_hardcore).filter_map(parse_date).min(),
            hardcore: earned_hardcore.is_some(),
            rarity: (players > 0.).then(|| (awarded / players * 100.).clamp(0., 100.) as f32),
            points: a.get("Points").and_then(Value::as_u64).map(|p| p as u32),
            kind: a.get("type").and_then(Value::as_str).and_then(kind_of),
            ..Default::default()
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    #[test]
    fn profile_from_the_documented_answer() {
        let doc = json!({"User":"MaxMilyin","ULID":"00003EMFWR7XB8SDPEHB3K56ZQ","UserPic":"/UserPic/MaxMilyin.png",
            "TotalPoints":399597,"TotalSoftcorePoints":12,"TotalTruePoints":1599212,"Permissions":1,"ID":16446});
        let p = parse_profile(&doc).unwrap();
        assert_eq!(p.username, "MaxMilyin");
        assert_eq!((p.hardcore_points, p.softcore_points, p.true_points), (399597, 12, 1599212));
        assert_eq!(p.avatar_url, "https://media.retroachievements.org/UserPic/MaxMilyin.png");
        assert_eq!(parse_profile(&json!({"User": null})), None);
        assert_eq!(parse_profile(&json!([])), None);
    }

    #[test]
    fn game_list_from_the_documented_answer() {
        let doc = json!([{"Title":"Advanced Busterhawk: Gley Lancer","ID":3684,"ConsoleID":1,"ConsoleName":"Mega Drive",
            "ImageIcon":"/Images/020895.png","NumAchievements":44,"NumLeaderboards":33,"Points":595,
            "DateModified":"2022-11-20 03:44:12","ForumTopicID":1936,
            "Hashes":["8BD4A97783CDA077C342173DF0A9B51E","a13ab653a20fb383337fab1e52ddb0df"]},
            {"Title":"No hashes","ID":7,"NumAchievements":3,"Hashes":null}, {"nonsense": true}]);
        let list = parse_game_list(&doc).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, 3684);
        assert_eq!(list[0].hashes, ["8bd4a97783cda077c342173df0a9b51e", "a13ab653a20fb383337fab1e52ddb0df"]);
        assert!(list[1].hashes.is_empty());
        assert_eq!(parse_game_list(&json!({"error": 1})), None);
    }

    fn sample() -> Value {
        json!({"ID":1,"Title":"Sonic the Hedgehog","ConsoleID":1,"ConsoleName":"Mega Drive","NumDistinctPlayers":200,
            "NumAchievements":3,"Achievements":{
            "10":{"ID":10,"NumAwarded":50,"NumAwardedHardcore":20,"Title":"Second","Description":"Do it","Points":5,
                  "BadgeName":"250337","DisplayOrder":2,"type":"missable"},
            "9":{"ID":9,"NumAwarded":150,"NumAwardedHardcore":100,"Title":"That Was Easy","Description":"Complete the first act in Green Hill Zone",
                 "Points":3,"BadgeName":"250336","DisplayOrder":1,"type":"progression",
                 "DateEarnedHardcore":"2016-03-12 17:47:29","DateEarned":"2016-03-12 17:47:29"},
            "11":{"ID":11,"NumAwarded":0,"Title":"Soft","Description":null,"Points":10,"BadgeName":"1","DisplayOrder":3,"type":null,
                  "DateEarned":"2020-01-02 03:04:05"}},
            "NumAwardedToUser":2})
    }

    #[test]
    fn set_from_the_documented_answer() {
        let now = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
        let set = parse_set(&sample(), 1, now).unwrap();
        assert_eq!(
            (set.source, set.external_id.as_str(), set.title.as_deref()),
            (Source::RetroAchievements, "1", Some("Sonic the Hedgehog"))
        );
        let ids: Vec<_> = set.achievements.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["9", "10", "11"], "DisplayOrder decides");
        let a = &set.achievements[0];
        assert_eq!(
            (a.name.as_str(), a.points, a.kind, a.unlocked, a.hardcore),
            ("That Was Easy", Some(3), Some(Kind::Progression), true, true)
        );
        assert_eq!(a.rarity, Some(75.));
        assert_eq!(a.unlocked_at.unwrap().to_rfc3339(), "2016-03-12T17:47:29+00:00");
        assert_eq!(a.icon, "https://media.retroachievements.org/Badge/250336.png");
        assert_eq!(a.icon_locked, "https://media.retroachievements.org/Badge/250336_lock.png");
        let b = &set.achievements[1];
        assert_eq!((b.unlocked, b.hardcore, b.unlocked_at, b.kind, b.rarity), (false, false, None, Some(Kind::Missable), Some(25.)));
        let c = &set.achievements[2];
        assert_eq!((c.unlocked, c.hardcore, c.kind, c.description.as_str(), c.rarity), (true, false, None, "", Some(0.)));
        assert!(c.unlocked_at.is_some());
    }

    #[test]
    fn no_players_means_no_rarity_and_no_achievements_is_an_empty_set() {
        let mut doc = sample();
        doc["NumDistinctPlayers"] = json!(0);
        assert!(parse_set(&doc, 1, Utc::now()).unwrap().achievements.iter().all(|a| a.rarity.is_none()));
        let empty = json!({"ID":2,"Title":"Nothing","NumDistinctPlayers":5,"Achievements":[]});
        assert!(parse_set(&empty, 2, Utc::now()).unwrap().achievements.is_empty());
        assert!(parse_set(&json!({"ID": 3}), 3, Utc::now()).is_none());
    }

    #[test]
    fn cache_freshness() {
        let now = Utc.with_ymd_and_hms(2026, 10, 8, 0, 0, 0).unwrap();
        assert!(is_fresh(now - Duration::days(6), now));
        assert!(!is_fresh(now - Duration::days(7), now));
        assert!(is_fresh(now + Duration::hours(1), now));
    }

    #[test]
    fn the_account_check_reads_the_profile_and_reports_rejections() {
        use super::super::test_support::serve;
        let ok = json!({"User":"Me","UserPic":"/UserPic/Me.png","TotalPoints":10,"TotalSoftcorePoints":0,"TotalTruePoints":30});
        let server = serve(vec![("u=Me", 200, ok.to_string()), ("u=Ghost", 200, "{}".into()), ("u=Bad", 401, "{}".into())]);
        let http = Http::default();
        let p = validate_account_at(&http, &server.base, " Me ", " KEY ").unwrap();
        assert_eq!((p.username.as_str(), p.hardcore_points, p.true_points), ("Me", 10, 30));
        let first = &server.requests()[0];
        assert!(first.contains("GET /API/API_GetUserProfile.php?") && first.contains("y=KEY") && first.contains("u=Me"), "{first}");
        assert!(validate_account_at(&http, &server.base, "Ghost", "k").unwrap_err().to_string().contains("does not know"));
        assert!(validate_account_at(&http, &server.base, "Bad", "k").unwrap_err().to_string().contains("rejected"));
    }

    #[test]
    fn the_key_never_reaches_a_message() {
        assert_eq!(scrub("GET https://x/API?y=SECRET&u=me failed", "SECRET"), "GET https://x/API?y=***&u=me failed");
        assert_eq!(scrub("kept", ""), "kept");
    }
}
