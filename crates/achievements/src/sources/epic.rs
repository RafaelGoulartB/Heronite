//! Epic Games Store: the store's own GraphQL (`launcher.store.epicgames.com`),
//! the one the Epic launcher and legendary (`get_game_achievements*`) use.
//! Epic's public EOS achievements API needs per-game developer credentials,
//! so it is not an option.
//!
//! * Definitions (public): `Achievement.productAchievementsRecordBySandbox`
//!   with the game's sandbox id, which is its `namespace` in legendary's
//!   metadata.
//! * Progress: `PlayerAchievement.playerAchievementGameRecordsBySandbox`
//!   with the account id and the access token of legendary's `user.json`.
//!   `records` is `null` for a game the account never earned anything in.
//!
//! The endpoint answers browsers' user agents with a redirect and other
//! agents with 403: it only takes the launcher's own.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use heronite_data::json_store;
use heronite_data::paths::DataPaths;
use heronite_epic::cmd::Legendary;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{Ctx, Fetch};
use crate::model::{Achievement, AchievementSet, Source};

pub const STORE_BASE: &str = "https://launcher.store.epicgames.com";

/// What legendary sends to this endpoint (`_store_user_agent`).
const LAUNCHER_USER_AGENT: &str = "EpicGamesLauncher/14.0.8-22004686+++Portal+Release-Live";

const DEFINITIONS_QUERY: &str = "query Achievement($sandboxId: String!, $locale: String!) { \
    Achievement { productAchievementsRecordBySandbox(sandboxId: $sandboxId, locale: $locale) { \
    sandboxId totalAchievements achievements { achievement { \
    name hidden unlockedDisplayName lockedDisplayName unlockedDescription lockedDescription \
    unlockedIconLink lockedIconLink XP rarity { percent } } } } } }";

const PLAYER_QUERY: &str = "query PlayerAchievement($epicAccountId: String!, $sandboxId: String!) { \
    PlayerAchievement { playerAchievementGameRecordsBySandbox(epicAccountId: $epicAccountId, sandboxId: $sandboxId) { \
    records { playerAchievements { playerAchievement { achievementName unlocked unlockDate } } } } } }";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Definition {
    name: String,
    #[serde(default)]
    hidden: bool,
    unlocked_display_name: Option<String>,
    locked_display_name: Option<String>,
    unlocked_description: Option<String>,
    locked_description: Option<String>,
    unlocked_icon_link: Option<String>,
    locked_icon_link: Option<String>,
    #[serde(rename = "XP")]
    xp: Option<f64>,
    rarity: Option<Rarity>,
}

#[derive(Debug, Deserialize)]
struct Rarity {
    percent: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Earned {
    achievement_name: String,
    #[serde(default)]
    unlocked: bool,
    /// An ISO time, or `"N/A"` while locked.
    unlock_date: Option<String>,
}

/// The `data` of a GraphQL answer, or why there is none.
fn data(doc: &Value) -> Result<&Value, String> {
    let errors = doc.get("errors").and_then(Value::as_array).filter(|e| !e.is_empty());
    match (doc.get("data").filter(|d| !d.is_null()), errors) {
        (Some(data), None) => Ok(data),
        (_, Some(_)) => Err("Epic refused the request".into()),
        (None, None) => Err("Epic answered something unexpected".into()),
    }
}

/// The game's achievements; `None` when it has none.
fn parse_definitions(doc: &Value) -> Result<Option<Vec<Definition>>, String> {
    let record = data(doc)?.pointer("/Achievement/productAchievementsRecordBySandbox").filter(|r| !r.is_null());
    let Some(list) = record.and_then(|r| r.get("achievements")).filter(|l| !l.is_null()) else { return Ok(None) };
    let items = list.as_array().ok_or("Epic answered something unexpected")?;
    let defs = items
        .iter()
        .filter_map(|item| item.get("achievement"))
        .map(|a| Definition::deserialize(a).map_err(|e| format!("Epic answered something unexpected: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((!defs.is_empty()).then_some(defs))
}

/// What the account earned, by achievement name. Empty for a game it never
/// earned anything in.
fn parse_progress(doc: &Value) -> Result<HashMap<String, Earned>, String> {
    let records = data(doc)?.pointer("/PlayerAchievement/playerAchievementGameRecordsBySandbox/records").and_then(Value::as_array);
    let mut earned: HashMap<String, Earned> = HashMap::new();
    for item in records.into_iter().flatten().filter_map(|r| r.get("playerAchievements")).filter_map(Value::as_array).flatten() {
        let Some(entry) = item.get("playerAchievement") else { continue };
        let e = Earned::deserialize(entry).map_err(|e| format!("Epic answered something unexpected: {e}"))?;
        let already = earned.get(&e.achievement_name).is_some_and(|old| old.unlocked);
        if !already {
            earned.insert(e.achievement_name.clone(), e);
        }
    }
    Ok(earned)
}

fn parse_date(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text).ok().map(|d| d.with_timezone(&Utc))
}

/// Non-empty text of the first of the two.
fn first_text(a: Option<String>, b: Option<String>) -> String {
    [a, b].into_iter().flatten().find(|t| !t.is_empty()).unwrap_or_default()
}

fn build_set(namespace: &str, defs: Vec<Definition>, mut earned: HashMap<String, Earned>) -> AchievementSet {
    let achievements = defs
        .into_iter()
        .map(|d| {
            let got = earned.remove(&d.name).filter(|e| e.unlocked);
            Achievement {
                name: first_text(d.unlocked_display_name, d.locked_display_name),
                description: first_text(d.unlocked_description, d.locked_description),
                hidden: d.hidden,
                icon: d.unlocked_icon_link.unwrap_or_default(),
                icon_locked: d.locked_icon_link.unwrap_or_default(),
                unlocked: got.is_some(),
                unlocked_at: got.and_then(|e| e.unlock_date).as_deref().and_then(parse_date),
                rarity: d.rarity.and_then(|r| r.percent).map(|p| p as f32),
                points: d.xp.filter(|x| *x >= 0.).map(|x| x.round() as u32),
                id: d.name,
                ..Default::default()
            }
        })
        .collect();
    AchievementSet { source: Source::Epic, external_id: namespace.into(), title: None, fetched_at: Utc::now(), achievements }
}

/// `en-US`, `pt-BR`, `es-ES` or `de-DE` for the interface language.
fn locale(lang: &str) -> &'static str {
    match lang {
        l if l.starts_with("pt") => "pt-BR",
        l if l.starts_with("es") => "es-ES",
        l if l.starts_with("de") => "de-DE",
        _ => "en-US",
    }
}

/// The game's sandbox: its `namespace` in legendary's metadata.
fn namespace(paths: &DataPaths, app_name: &str) -> Option<String> {
    let file = paths.legendary_config_dir().join("metadata").join(format!("{app_name}.json"));
    let doc = json_store::read_value(&file).ok()?;
    doc.pointer("/metadata/namespace").and_then(Value::as_str).filter(|n| !n.is_empty()).map(str::to_string)
}

struct Session {
    account_id: String,
    token: String,
    /// Valid for at least another minute.
    fresh: bool,
}

/// The account and token of legendary's `user.json`.
fn read_session(paths: &DataPaths) -> Option<Session> {
    let doc = json_store::read_value(&paths.legendary_config_dir().join("user.json")).ok()?;
    let text = |k: &str| doc.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    let fresh = text("expires_at").and_then(|t| parse_date(&t)).is_none_or(|at| at > Utc::now() + chrono::Duration::seconds(60));
    Some(Session { account_id: text("account_id")?, token: text("access_token")?, fresh })
}

/// Account id and a valid token. `legendary status` renews an expired token
/// (and removes `user.json` when Epic refuses the stored sign-in): `Ok(None)`
/// then, as for a signed-out user.
fn credentials_with(paths: &DataPaths, renew: impl FnOnce() -> Result<(), String>) -> Result<Option<(String, String)>, String> {
    let mut session = read_session(paths);
    if session.as_ref().is_some_and(|s| !s.fresh) {
        renew()?;
        session = read_session(paths);
    }
    Ok(session.filter(|s| s.fresh).map(|s| (s.account_id, s.token)))
}

fn credentials(paths: &DataPaths) -> Result<Option<(String, String)>, String> {
    credentials_with(paths, || {
        let legendary = Legendary::locate(paths).map_err(|e| e.to_string())?;
        legendary.run(&["status"], &[]).map(drop).map_err(|e| format!("{e:#}"))
    })
}

/// One GraphQL request, answered as JSON.
fn post(ctx: &Ctx, base: &str, token: &str, query: &str, variables: Value) -> Result<Value, String> {
    let body = json!({ "query": query, "variables": variables }).to_string();
    let response = ctx
        .http
        .post(format!("{base}/graphql"))
        .header("Content-Type", "application/json")
        .header("User-Agent", LAUNCHER_USER_AGENT)
        .header("Authorization", &format!("Bearer {token}"))
        .body(body)
        .send()
        .map_err(|e| format!("Epic: {e:#}"))?;
    if !response.ok() {
        return Err(format!("Epic: HTTP {}", response.status));
    }
    response.json::<Value>().map_err(|_| "Epic answered something unexpected".into())
}

pub fn fetch(ctx: &Ctx, app_name: &str) -> Fetch {
    fetch_from(ctx, STORE_BASE, app_name, credentials)
}

fn fetch_from(ctx: &Ctx, base: &str, app_name: &str, creds: impl FnOnce(&DataPaths) -> Result<Option<(String, String)>, String>) -> Fetch {
    if !heronite_epic::user::is_logged_in(ctx.paths) {
        return Fetch::NeedsAccount;
    }
    let Some(namespace) = namespace(ctx.paths, app_name) else { return Fetch::None };
    let (account_id, token) = match creds(ctx.paths) {
        Ok(Some(c)) => c,
        Ok(None) => return Fetch::NeedsAccount,
        Err(e) => return Fetch::Failed(e),
    };
    let result = (|| {
        let doc = post(ctx, base, &token, DEFINITIONS_QUERY, json!({ "sandboxId": namespace, "locale": locale(ctx.lang) }))?;
        let Some(defs) = parse_definitions(&doc)? else { return Ok(None) };
        let doc = post(ctx, base, &token, PLAYER_QUERY, json!({ "sandboxId": namespace, "epicAccountId": account_id }))?;
        Ok::<_, String>(Some(build_set(&namespace, defs, parse_progress(&doc)?)))
    })();
    match result {
        Ok(Some(set)) => Fetch::Set(set),
        Ok(None) => Fetch::None,
        Err(e) => Fetch::Failed(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::Accounts;
    use chrono::TimeZone;
    use heronite_metadata::Http;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// A trimmed answer of the definitions query (Fall Guys, `pt-BR`).
    fn definitions() -> Value {
        json!({"data": {"Achievement": {"productAchievementsRecordBySandbox": {
            "sandboxId": "sb", "totalAchievements": 3, "achievements": [
            {"achievement": {"name": "ACH_A", "hidden": false,
                "unlockedDisplayName": "Azarao", "lockedDisplayName": "Azarao Locked",
                "unlockedDescription": "Qualify with 1 point", "lockedDescription": "Locked description",
                "unlockedIconLink": "https://cdn/a-on", "lockedIconLink": "https://cdn/a-off",
                "XP": 15, "rarity": {"percent": 3}}},
            {"achievement": {"name": "ACH_B", "hidden": true,
                "unlockedDisplayName": "Secret", "lockedDisplayName": "",
                "unlockedDescription": "Do the secret thing", "lockedDescription": "",
                "unlockedIconLink": "https://cdn/b-on", "lockedIconLink": "https://cdn/b-off",
                "XP": 50, "rarity": {"percent": 0.1}}},
            {"achievement": {"name": "ACH_C", "hidden": false,
                "unlockedDisplayName": "Plain", "lockedDisplayName": "Plain",
                "unlockedDescription": null, "lockedDescription": null,
                "unlockedIconLink": null, "lockedIconLink": null,
                "XP": null, "rarity": null}}]}}}})
    }

    fn progress() -> Value {
        json!({"data": {"PlayerAchievement": {"playerAchievementGameRecordsBySandbox": {"records": [
            {"playerAchievements": [
                {"playerAchievement": {"achievementName": "ACH_A", "unlocked": true, "unlockDate": "2022-07-16T21:57:28.807Z"}},
                {"playerAchievement": {"achievementName": "ACH_B", "unlocked": false, "unlockDate": "N/A"}}]}]}}}})
    }

    #[test]
    fn builds_the_set_from_both_answers() {
        let defs = parse_definitions(&definitions()).unwrap().unwrap();
        let set = build_set("sb", defs, parse_progress(&progress()).unwrap());
        assert_eq!((set.source, set.external_id.as_str(), set.total(), set.unlocked()), (Source::Epic, "sb", 3, 1));
        let [a, b, c] = &set.achievements[..] else { panic!("three achievements") };
        assert_eq!((a.id.as_str(), a.name.as_str(), a.description.as_str()), ("ACH_A", "Azarao", "Qualify with 1 point"));
        assert_eq!((a.icon.as_str(), a.icon_locked.as_str()), ("https://cdn/a-on", "https://cdn/a-off"));
        assert_eq!((a.unlocked, a.hidden, a.rarity, a.points), (true, false, Some(3.), Some(15)));
        assert_eq!(a.unlocked_at, Some(Utc.with_ymd_and_hms(2022, 7, 16, 21, 57, 28).unwrap() + chrono::Duration::milliseconds(807)));
        assert_eq!((b.name.as_str(), b.hidden, b.unlocked, b.unlocked_at, b.rarity), ("Secret", true, false, None, Some(0.1)));
        assert_eq!((c.description.as_str(), c.icon.as_str(), c.points, c.rarity, c.unlocked), ("", "", None, None, false));
        assert_eq!(set.points(), Some((15, 65)));
    }

    #[test]
    fn a_game_never_earned_anything_in_has_no_records() {
        let none = json!({"data": {"PlayerAchievement": {"playerAchievementGameRecordsBySandbox": {"records": null}}}});
        assert!(parse_progress(&none).unwrap().is_empty());
        let defs = parse_definitions(&definitions()).unwrap().unwrap();
        assert_eq!(build_set("sb", defs, HashMap::new()).unlocked(), 0);
    }

    #[test]
    fn games_without_achievements() {
        let nulls = json!({"data": {"Achievement": {"productAchievementsRecordBySandbox":
            {"sandboxId": null, "totalAchievements": null, "achievements": null}}}});
        assert!(parse_definitions(&nulls).unwrap().is_none());
        let record = json!({"data": {"Achievement": {"productAchievementsRecordBySandbox": null}}});
        assert!(parse_definitions(&record).unwrap().is_none());
        let empty = json!({"data": {"Achievement": {"productAchievementsRecordBySandbox": {"achievements": []}}}});
        assert!(parse_definitions(&empty).unwrap().is_none());
    }

    #[test]
    fn graphql_errors_are_failures() {
        let denied = json!({"errors": [{"message": "nope"}], "data": null});
        assert!(parse_definitions(&denied).is_err());
        assert!(parse_progress(&denied).is_err());
        assert!(parse_progress(&json!({"unexpected": true})).is_err());
    }

    #[test]
    fn locales() {
        assert_eq!(locale("pt-BR"), "pt-BR");
        assert_eq!(locale("es"), "es-ES");
        assert_eq!(locale("de"), "de-DE");
        assert_eq!(locale("en"), "en-US");
    }

    fn write_user(paths: &DataPaths, expires_at: &str) {
        std::fs::create_dir_all(paths.legendary_config_dir().join("metadata")).unwrap();
        let user = json!({"account_id": "acc1", "access_token": "tok", "expires_at": expires_at, "displayName": "P"});
        std::fs::write(paths.legendary_config_dir().join("user.json"), user.to_string()).unwrap();
        let meta = json!({"app_name": "Fall", "metadata": {"namespace": "sb"}});
        std::fs::write(paths.legendary_config_dir().join("metadata/Fall.json"), meta.to_string()).unwrap();
    }

    fn future() -> String {
        (Utc::now() + chrono::Duration::hours(2)).to_rfc3339()
    }

    #[test]
    fn the_namespace_comes_from_legendarys_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        write_user(&paths, &future());
        assert_eq!(namespace(&paths, "Fall").as_deref(), Some("sb"));
        assert_eq!(namespace(&paths, "Missing"), None);
    }

    #[test]
    fn an_expired_token_is_renewed_through_legendary() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        write_user(&paths, "2020-01-01T00:00:00.000Z");
        let renewed = credentials_with(&paths, || {
            write_user(&paths, &future());
            Ok(())
        });
        assert_eq!(renewed, Ok(Some(("acc1".into(), "tok".into()))));
        // Epic refused the stored sign-in: legendary deleted user.json.
        write_user(&paths, "2020-01-01T00:00:00.000Z");
        let refused =
            credentials_with(&paths, || std::fs::remove_file(paths.legendary_config_dir().join("user.json")).map_err(|e| e.to_string()));
        assert_eq!(refused, Ok(None));
        // A token that is still valid does not call legendary.
        write_user(&paths, &future());
        assert!(credentials_with(&paths, || Err("must not run".into())).unwrap().is_some());
    }

    /// Answers `bodies` in turn, one connection each; the raw requests come
    /// back through the receiver.
    fn serve(bodies: Vec<String>) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for (mut stream, body) in listener.incoming().filter_map(Result::ok).zip(bodies) {
                let mut request = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let n = stream.read(&mut chunk).unwrap_or(0);
                    request.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&request).into_owned();
                    let Some((head, rest)) = text.split_once("\r\n\r\n") else {
                        if n == 0 {
                            break;
                        }
                        continue;
                    };
                    let length = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if rest.len() >= length || n == 0 {
                        break;
                    }
                }
                let _ = tx.send(String::from_utf8_lossy(&request).into_owned());
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        (format!("http://{addr}"), rx)
    }

    fn run(base: &str, paths: &DataPaths, lang: &str) -> Fetch {
        let (http, accounts) = (Http::default(), Accounts::default());
        let ctx = Ctx { paths, http: &http, accounts: &accounts, lang };
        fetch_from(&ctx, base, "Fall", |p| Ok(read_session(p).map(|s| (s.account_id, s.token))))
    }

    #[test]
    fn fetches_definitions_then_progress_as_the_launcher() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        write_user(&paths, &future());
        let (base, requests) = serve(vec![definitions().to_string(), progress().to_string()]);
        let Fetch::Set(set) = run(&base, &paths, "pt-BR") else { panic!("a set") };
        assert_eq!((set.external_id.as_str(), set.total(), set.unlocked()), ("sb", 3, 1));
        let first = requests.recv().unwrap();
        let first_lower = first.to_ascii_lowercase();
        assert!(first.starts_with("POST /graphql "), "{first}");
        assert!(first_lower.contains(&format!("user-agent: {}", LAUNCHER_USER_AGENT.to_ascii_lowercase())), "{first}");
        assert!(first_lower.contains("authorization: bearer tok"), "{first}");
        assert!(first.contains(r#""sandboxId":"sb""#) && first.contains(r#""locale":"pt-BR""#), "{first}");
        let second = requests.recv().unwrap();
        assert!(second.contains(r#""epicAccountId":"acc1""#) && second.contains("playerAchievementGameRecordsBySandbox"), "{second}");
    }

    #[test]
    fn a_game_without_achievements_skips_the_progress_query() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        write_user(&paths, &future());
        let nulls = json!({"data": {"Achievement": {"productAchievementsRecordBySandbox": {"achievements": null}}}});
        let (base, requests) = serve(vec![nulls.to_string()]);
        assert_eq!(run(&base, &paths, "en"), Fetch::None);
        assert!(requests.recv().is_ok());
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn signed_out_and_unknown_games() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        assert_eq!(run("http://127.0.0.1:1", &paths, "en"), Fetch::NeedsAccount);
        write_user(&paths, &future());
        std::fs::remove_file(paths.legendary_config_dir().join("metadata/Fall.json")).unwrap();
        assert_eq!(run("http://127.0.0.1:1", &paths, "en"), Fetch::None);
    }

    #[test]
    fn an_unreachable_store_is_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        write_user(&paths, &future());
        assert!(matches!(run("http://127.0.0.1:1", &paths, "en"), Fetch::Failed(_)));
    }
}
