//! Steam: the stats files the Steam client keeps on this PC (no key, works
//! offline), the Web API for the share of players that have each
//! achievement, and, with the user's Web API key, for the games the client
//! has no stats of.

mod kv;
mod local;
mod web;

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use chrono::Utc;
use heronite_launch::probe::SystemProbe;
use heronite_launch::settings::GlobalSettings;
use heronite_launch::steam::SteamLocator;

use self::local::{Entry, Schema, Unlocks};
use super::{Ctx, Fetch};
use crate::model::{Achievement, AchievementSet, Progress, Source};

pub const API_BASE: &str = "https://api.steampowered.com";
pub const CDN_BASE: &str = "https://cdn.cloudflare.steamstatic.com";

/// The servers to ask; tests point them nowhere.
struct Bases<'a> {
    api: &'a str,
    cdn: &'a str,
}

/// A Steam install and the account whose stats it keeps.
struct Install {
    root: PathBuf,
    account: Option<String>,
}

pub fn fetch(ctx: &Ctx, app_id: &str) -> Fetch {
    let locator = SteamLocator::detect(&GlobalSettings::load(ctx.paths), &SystemProbe);
    fetch_from(ctx, &Bases { api: API_BASE, cdn: CDN_BASE }, &locator, app_id)
}

fn fetch_from(ctx: &Ctx, bases: &Bases, locator: &SteamLocator, app_id: &str) -> Fetch {
    let installs: Vec<Install> = locator
        .installs()
        .into_iter()
        .map(|root| Install { account: heronite_steam::files::current_user(&root, &locator.home), root })
        .collect();
    let account = installs.iter().find_map(|i| i.account.as_deref());
    let key = ctx.accounts.steam_web_api_key.as_deref().filter(|k| !k.is_empty());
    let web_lang = local::web_language(ctx.lang);

    let (schema, unlocks) = match read_schema(&installs, app_id, ctx.lang) {
        Some(schema) => {
            let mut unlocks = read_unlocks(&installs, &schema, app_id);
            if let (None, Some(key), Some(account)) = (&unlocks, key, account) {
                match web::player(ctx.http, bases.api, key, account, app_id, web_lang) {
                    Ok(web) => unlocks = Some(web),
                    Err(e) => return Fetch::Failed(format!("Steam: {e}")),
                }
            }
            (schema, unlocks)
        }
        None => {
            let (Some(key), Some(account)) = (key, account) else { return Fetch::None };
            let schema = match web::schema(ctx.http, bases.api, key, app_id, web_lang) {
                Ok(schema) if schema.entries.is_empty() => return Fetch::None,
                Ok(schema) => schema,
                Err(e) => return Fetch::Failed(format!("Steam: {e}")),
            };
            match web::player(ctx.http, bases.api, key, account, app_id, web_lang) {
                Ok(unlocks) => (schema, Some(unlocks)),
                Err(e) => return Fetch::Failed(format!("Steam: {e}")),
            }
        }
    };
    if schema.entries.is_empty() {
        return Fetch::None;
    }
    let rarity = web::rarity(ctx.http, bases.api, app_id);
    Fetch::Set(build(app_id, &schema, unlocks.as_ref(), &rarity, bases.cdn))
}

/// The first readable schema of the game among the installs.
fn read_schema(installs: &[Install], app_id: &str, lang: &str) -> Option<Schema> {
    installs.iter().find_map(|i| {
        let file = local::schema_file(&i.root, app_id);
        let bytes = fs::read(&file).ok()?;
        match local::parse_schema(&bytes, lang) {
            Ok(schema) => Some(schema),
            Err(e) => {
                log::warn!("{}: {e}", file.display());
                None
            }
        }
    })
}

/// What the account did, when the client has a stats file of it.
fn read_unlocks(installs: &[Install], schema: &Schema, app_id: &str) -> Option<Unlocks> {
    installs.iter().find_map(|i| {
        let file = local::user_file(&i.root, i.account.as_deref()?, app_id);
        let bytes = fs::read(&file).ok()?;
        match local::parse_user(&bytes, schema) {
            Ok(unlocks) => Some(unlocks),
            Err(e) => {
                log::warn!("{}: {e}", file.display());
                None
            }
        }
    })
}

/// An icon file name of the client's schema becomes its CDN URL; the Web API
/// already gives URLs.
fn icon_url(cdn: &str, app_id: &str, icon: &str) -> String {
    if icon.is_empty() || icon.starts_with("http") {
        icon.to_string()
    } else {
        format!("{cdn}/steamcommunity/public/images/apps/{app_id}/{icon}")
    }
}

fn achievement(entry: &Entry, app_id: &str, unlocks: Option<&Unlocks>, rarity: &HashMap<String, f32>, cdn: &str) -> Achievement {
    let done = unlocks.and_then(|u| u.done.get(&entry.api));
    let progress = entry.progress.as_ref().map(|(stat, max)| {
        let current =
            if done.is_some() { *max } else { unlocks.and_then(|u| u.stats.get(stat)).map_or(0, |v| (*v).clamp(0, *max as i64) as u32) };
        Progress { current, max: *max }
    });
    Achievement {
        id: entry.api.clone(),
        name: entry.name.clone(),
        description: entry.description.clone(),
        hidden: entry.hidden,
        icon: icon_url(cdn, app_id, &entry.icon),
        icon_locked: icon_url(cdn, app_id, &entry.icon_gray),
        unlocked: done.is_some(),
        unlocked_at: done.copied().flatten(),
        rarity: rarity.get(&entry.api).copied(),
        progress,
        ..Default::default()
    }
}

fn build(app_id: &str, schema: &Schema, unlocks: Option<&Unlocks>, rarity: &HashMap<String, f32>, cdn: &str) -> AchievementSet {
    let achievements = schema.entries.iter().map(|e| achievement(e, app_id, unlocks, rarity, cdn)).collect();
    AchievementSet { source: Source::Steam, external_id: app_id.into(), title: None, fetched_at: Utc::now(), achievements }
}

#[cfg(test)]
mod tests {
    use super::kv::build::{file, group, int, string};
    use super::*;
    use crate::accounts::Accounts;
    use heronite_data::paths::DataPaths;
    use heronite_metadata::Http;
    use std::path::Path;

    const ACCOUNT: &str = "314512607";

    fn schema_bytes() -> Vec<u8> {
        let item = |bit: &str, api: &str, name: &str, hidden: i32, progress: Vec<Vec<u8>>| {
            let mut children = vec![
                string("name", api),
                group(
                    "display",
                    &[
                        group("name", &[string("english", name)]),
                        group("desc", &[string("english", "d")]),
                        int("hidden", hidden),
                        string("icon", "on.jpg"),
                        string("icon_gray", "off.jpg"),
                    ],
                ),
            ];
            children.extend(progress);
            group(bit, &children)
        };
        let counted =
            vec![group("progress", &[int("max_val", 10), group("value", &[string("operation", "statvalue"), string("operand1", "runs")])])];
        let bits = group("bits", &[item("0", "A", "Alpha", 0, vec![]), item("1", "B", "Beta", 1, counted)]);
        let runs = group("2", &[string("type", "INT"), string("name", "runs")]);
        file(group("620", &[group("stats", &[group("1", &[bits]), runs])]))
    }

    fn user_bytes(mask: i32) -> Vec<u8> {
        let times = group("AchievementTimes", &[int("0", 1_700_000_000)]);
        file(group("cache", &[group("1", &[int("data", mask), times]), group("2", &[int("data", 6)])]))
    }

    /// A Steam folder with the client's stats files of app 620.
    fn steam_with(user_mask: Option<i32>) -> (tempfile::TempDir, SteamLocator) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("steam");
        fs::create_dir_all(root.join("config")).unwrap();
        fs::create_dir_all(root.join("userdata").join(ACCOUNT).join("config")).unwrap();
        fs::create_dir_all(local::stats_dir(&root)).unwrap();
        fs::write(local::schema_file(&root, "620"), schema_bytes()).unwrap();
        if let Some(mask) = user_mask {
            fs::write(local::user_file(&root, ACCOUNT, "620"), user_bytes(mask)).unwrap();
        }
        let locator = SteamLocator { home: dir.path().to_path_buf(), steam_path: root, system_roots: vec![] };
        (dir, locator)
    }

    /// Nothing listens there: the Web API is unreachable.
    const OFFLINE: Bases = Bases { api: "http://127.0.0.1:9", cdn: "https://cdn.test" };

    fn run(dir: &Path, locator: &SteamLocator, key: Option<&str>, app_id: &str) -> Fetch {
        let paths = DataPaths::at(dir);
        let http = Http::default();
        let accounts = Accounts { steam_web_api_key: key.map(str::to_string), ..Default::default() };
        fetch_from(&Ctx { paths: &paths, http: &http, accounts: &accounts, lang: "en" }, &OFFLINE, locator, app_id)
    }

    #[test]
    fn reads_the_clients_files_and_survives_a_failing_rarity_call() {
        let (dir, locator) = steam_with(Some(1));
        let Fetch::Set(set) = run(dir.path(), &locator, None, "620") else { panic!("expected a set") };
        assert_eq!((set.source, set.external_id.as_str(), set.total(), set.unlocked()), (Source::Steam, "620", 2, 1));
        let (a, b) = (&set.achievements[0], &set.achievements[1]);
        assert_eq!((a.id.as_str(), a.name.as_str(), a.unlocked, a.rarity), ("A", "Alpha", true, None));
        assert_eq!(a.unlocked_at.unwrap().timestamp(), 1_700_000_000);
        assert_eq!(a.icon, "https://cdn.test/steamcommunity/public/images/apps/620/on.jpg");
        assert_eq!(a.icon_locked, "https://cdn.test/steamcommunity/public/images/apps/620/off.jpg");
        assert_eq!((b.unlocked, b.hidden, b.progress), (false, true, Some(Progress { current: 6, max: 10 })));
    }

    #[test]
    fn without_a_user_file_and_a_key_everything_is_locked() {
        let (dir, locator) = steam_with(None);
        let Fetch::Set(set) = run(dir.path(), &locator, None, "620") else { panic!("expected a set") };
        assert_eq!((set.total(), set.unlocked()), (2, 0));
        assert_eq!(set.achievements[1].progress, Some(Progress { current: 0, max: 10 }));
    }

    #[test]
    fn a_failing_web_fallback_fails_without_leaking_the_key() {
        let (dir, locator) = steam_with(None);
        let Fetch::Failed(message) = run(dir.path(), &locator, Some("SECRETKEY"), "620") else { panic!("expected a failure") };
        assert!(message.starts_with("Steam: "), "{message}");
        assert!(!message.contains("SECRETKEY"), "{message}");
    }

    #[test]
    fn no_schema_anywhere_is_none() {
        let (dir, locator) = steam_with(Some(1));
        assert_eq!(run(dir.path(), &locator, None, "999"), Fetch::None);
    }

    #[test]
    fn icon_urls() {
        assert_eq!(icon_url("https://c", "1", ""), "");
        assert_eq!(icon_url("https://c", "1", "https://x/y.jpg"), "https://x/y.jpg");
        assert_eq!(icon_url("https://c", "1", "z.jpg"), "https://c/steamcommunity/public/images/apps/1/z.jpg");
    }
}
