//! RetroAchievements' client API (`dorequest.php`), the one emulators use
//! (rcheevos' `rc_api_*` requests, form-encoded POSTs): the login with the
//! username and password, which gives the token every later call and the
//! emulators use; the games of a console with their accepted hashes (no
//! login); a game's achievements (with their rarity) and which ones the user
//! has. It gives no unlock times: a new unlock gets the time it is noticed.

use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use heronite_metadata::Http;
use serde_json::Value;

use super::api::{kind_of, Listed, MEDIA};
use crate::accounts::RetroAccount;
use crate::model::{Achievement, AchievementSet, Source};

/// The site asks clients to say who they are.
const USER_AGENT: &str = concat!("Heronite/", env!("CARGO_PKG_VERSION"), " (Linux)");

/// Achievements of the core set (`Flags` 5 are unofficial ones).
const CORE: u64 = 3;

/// `<host>/dorequest.php` for a Web API base (`<host>/API/`).
pub fn dorequest_url(base: &str) -> String {
    let host = base.strip_suffix("API/").unwrap_or(base);
    format!("{}/dorequest.php", host.trim_end_matches('/'))
}

/// `application/x-www-form-urlencoded`.
fn form(params: &[(&str, &str)]) -> String {
    let encode = |s: &str| -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
                _ => format!("%{b:02X}"),
            })
            .collect()
    };
    params.iter().map(|(k, v)| format!("{}={}", encode(k), encode(v))).collect::<Vec<_>>().join("&")
}

/// One request; `secrets` (password, token) never show in an error.
fn call(http: &Http, base: &str, params: &[(&str, &str)], secrets: &[&str], big: bool) -> Result<Value> {
    let hide = |text: String| secrets.iter().filter(|s| !s.is_empty()).fold(text, |t, s| t.replace(s, "***"));
    let mut request = http
        .post(dorequest_url(base))
        .header("User-Agent", USER_AGENT)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(form(params));
    if big {
        request = request.timeout_secs(120).max_bytes(128 * 1024 * 1024);
    }
    let response = request.send().map_err(|e| anyhow!(hide(format!("{e:#}"))))?;
    let doc: Option<Value> = response.json().ok();
    let error = doc.as_ref().and_then(|d| d.get("Error")).and_then(Value::as_str).filter(|e| !e.is_empty()).map(str::to_owned);
    match response.status {
        401 | 403 => bail!(Rejected(error.unwrap_or_else(|| "RetroAchievements did not accept the login".into()))),
        429 => bail!("RetroAchievements asks to slow down (HTTP 429)"),
        s if !(200..300).contains(&s) => bail!("RetroAchievements: HTTP {s}"),
        _ => {}
    }
    let doc = doc.context("RetroAchievements answered something unexpected")?;
    if doc.get("Success").and_then(Value::as_bool) == Some(false) {
        let message = error.unwrap_or_else(|| "RetroAchievements refused the request".into());
        // The site answers a wrong password or a stale token with `Success: false`.
        if doc.get("Code").and_then(Value::as_str).is_some_and(|c| c.contains("credentials") || c.contains("token")) {
            bail!(Rejected(hide(message)));
        }
        bail!(hide(message));
    }
    Ok(doc)
}

/// The site said no to the username, password or token.
#[derive(Debug)]
pub struct Rejected(pub String);

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Rejected {}

/// Whether `err` is the site refusing the login (as opposed to no network).
pub fn is_rejected(err: &anyhow::Error) -> bool {
    err.downcast_ref::<Rejected>().is_some()
}

/// A signed-in account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// As the site writes it (its capitals).
    pub username: String,
    pub token: String,
    /// Hardcore points.
    pub points: u32,
    pub softcore_points: u32,
}

pub fn parse_login(doc: &Value) -> Option<Session> {
    let text = |k: &str| doc.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
    let number = |k: &str| doc.get(k).and_then(Value::as_u64).unwrap_or(0).min(u32::MAX as u64) as u32;
    Some(Session { username: text("User")?, token: text("Token")?, points: number("Score"), softcore_points: number("SoftcoreScore") })
}

/// Signs in with the password (never kept): the session has the token.
pub fn login(http: &Http, base: &str, username: &str, password: &str) -> Result<Session> {
    let doc = call(http, base, &[("r", "login2"), ("u", username.trim()), ("p", password)], &[password], false)?;
    parse_login(&doc).context("RetroAchievements answered the login in an unknown shape")
}

/// Checks a kept token (and reads the points again).
pub fn check(http: &Http, base: &str, username: &str, token: &str) -> Result<Session> {
    let doc = call(http, base, &[("r", "login2"), ("u", username), ("t", token)], &[token], false)?;
    parse_login(&doc).context("RetroAchievements answered the login in an unknown shape")
}

/// `r=systemgames`: the games of a console with the hashes they accept;
/// only those with achievements.
pub fn parse_system_games(doc: &Value) -> Option<Vec<Listed>> {
    let items = doc.get("Response")?.as_array()?;
    Some(
        items
            .iter()
            .filter_map(|g| {
                let achievements = g.get("NumAchievements").and_then(Value::as_u64).unwrap_or(0) as u32;
                Some(Listed {
                    id: u32::try_from(g.get("ID")?.as_u64()?).ok()?,
                    title: g.get("Title")?.as_str()?.to_owned(),
                    achievements,
                    hashes: g
                        .get("SupportedHashes")
                        .and_then(Value::as_array)
                        .map(|h| h.iter().filter_map(Value::as_str).map(str::to_ascii_lowercase).collect())
                        .unwrap_or_default(),
                })
            })
            .filter(|g| g.achievements > 0)
            .collect(),
    )
}

pub fn system_games(http: &Http, base: &str, console: u32) -> Result<Vec<Listed>> {
    let console = console.to_string();
    let doc = call(http, base, &[("r", "systemgames"), ("s", &console)], &[], true)?;
    parse_system_games(&doc).context("RetroAchievements sent a game list in an unknown shape")
}

/// `r=patch`'s `PatchData`: the game's core achievements, every one locked.
pub fn parse_patch(doc: &Value, game_id: u32) -> Option<AchievementSet> {
    let data = doc.get("PatchData")?;
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    let achievements = data
        .get("Achievements")?
        .as_array()?
        .iter()
        .filter(|a| a.get("Flags").and_then(Value::as_u64).unwrap_or(CORE) == CORE)
        .filter_map(|a| {
            let id = a.get("ID")?.as_u64()?;
            let badge = text(a, "BadgeName");
            let url = |k: &str, fallback: String| Some(text(a, k)).filter(|u| !u.is_empty()).unwrap_or(fallback);
            Some(Achievement {
                id: id.to_string(),
                name: text(a, "Title"),
                description: text(a, "Description"),
                icon: url("BadgeURL", format!("{MEDIA}/Badge/{badge}.png")),
                icon_locked: url("BadgeLockedURL", format!("{MEDIA}/Badge/{badge}_lock.png")),
                rarity: a.get("Rarity").and_then(Value::as_f64).map(|r| (r as f32).clamp(0., 100.)),
                points: a.get("Points").and_then(Value::as_u64).map(|p| p as u32),
                kind: a.get("Type").and_then(Value::as_str).and_then(kind_of),
                ..Default::default()
            })
        })
        .collect();
    Some(AchievementSet {
        source: Source::RetroAchievements,
        external_id: game_id.to_string(),
        title: Some(text(data, "Title")).filter(|t| !t.is_empty()),
        fetched_at: Utc::now(),
        achievements,
    })
}

/// `r=unlocks`' `UserUnlocks`: achievement ids.
pub fn parse_unlocks(doc: &Value) -> Vec<String> {
    doc.get("UserUnlocks").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64).map(|id| id.to_string()).collect()
}

/// A game's achievements and which ones the account has (softcore and
/// hardcore are asked apart: hardcore ones are marked).
pub fn game_progress(http: &Http, base: &str, account: &RetroAccount, game_id: u32) -> Result<AchievementSet> {
    let (u, t) = (account.username.as_str(), account.token.as_str());
    let g = game_id.to_string();
    let patch = call(http, base, &[("r", "patch"), ("u", u), ("t", t), ("g", &g)], &[t], false)?;
    let mut set = parse_patch(&patch, game_id).context("RetroAchievements sent a game in an unknown shape")?;
    let unlocks = |hardcore: &str| -> Result<Vec<String>> {
        Ok(parse_unlocks(&call(http, base, &[("r", "unlocks"), ("u", u), ("t", t), ("g", &g), ("h", hardcore)], &[t], false)?))
    };
    let (softcore, hardcore) = (unlocks("0")?, unlocks("1")?);
    for a in &mut set.achievements {
        a.hardcore = hardcore.contains(&a.id);
        a.unlocked = a.hardcore || softcore.contains(&a.id);
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn urls_and_forms() {
        assert_eq!(dorequest_url("https://retroachievements.org/API/"), "https://retroachievements.org/dorequest.php");
        assert_eq!(dorequest_url("http://127.0.0.1:9/API/"), "http://127.0.0.1:9/dorequest.php");
        assert_eq!(form(&[("r", "login2"), ("p", "a b&c=é")]), "r=login2&p=a%20b%26c%3D%C3%A9");
    }

    #[test]
    fn login_answer() {
        let doc = json!({"Success":true,"User":"Rafa","Token":"tok","Score":980,"SoftcoreScore":260,"Messages":0});
        assert_eq!(parse_login(&doc), Some(Session { username: "Rafa".into(), token: "tok".into(), points: 980, softcore_points: 260 }));
        assert_eq!(parse_login(&json!({"Success":true,"User":"x"})), None);
    }

    #[test]
    fn system_games_keep_those_with_achievements() {
        let doc = json!({"Success":true,"Response":[
            {"ID":1,"Title":"Sonic","ImageIcon":"/i.png","ImageUrl":"u","NumAchievements":30,"NumLeaderboards":0,"Points":400,
             "SupportedHashes":["ABCDEF"]},
            {"ID":2,"Title":"Empty","NumAchievements":0,"SupportedHashes":["1"]}]});
        let list = parse_system_games(&doc).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].id, list[0].hashes[0].as_str()), (1, "abcdef"));
    }

    #[test]
    fn patch_and_unlocks_make_the_set() {
        let doc = json!({"Success":true,"PatchData":{"ID":6,"Title":"Super Test","Achievements":[
            {"ID":11,"Title":"First","Description":"d","Flags":3,"Points":5,"BadgeName":"7","Type":"progression","Rarity":42.5,
             "BadgeURL":"https://m/Badge/7.png","BadgeLockedURL":"https://m/Badge/7_lock.png"},
            {"ID":12,"Title":"Beta","Flags":5,"Points":1,"BadgeName":"8"},
            {"ID":13,"Title":"Old","Flags":3,"Points":10,"BadgeName":"9"}]}});
        let set = parse_patch(&doc, 6).unwrap();
        let ids: Vec<_> = set.achievements.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["11", "13"]);
        assert_eq!(set.title.as_deref(), Some("Super Test"));
        let first = &set.achievements[0];
        assert_eq!((first.rarity, first.points, first.icon_locked.as_str()), (Some(42.5), Some(5), "https://m/Badge/7_lock.png"));
        assert_eq!(set.achievements[1].icon, format!("{MEDIA}/Badge/9.png"));
        assert_eq!(parse_unlocks(&json!({"Success":true,"UserUnlocks":[11,13],"GameID":6})), ["11", "13"]);
    }
}
