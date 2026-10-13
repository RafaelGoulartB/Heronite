//! The account's Steam library, read from the running client (no Steam
//! login or API key: the client is signed in already), cached in
//! `store_cache/steam_library.json` and joined to the Collection:
//!
//! * only games: no tools, soundtracks, demos, test servers, playtests or
//!   games hidden in Steam;
//! * a game the Collection has from another source becomes that entry's
//!   Steam twin (`heronite_data::steam_owned`) instead of a new entry;
//! * the others are added as Steam entries, installed or not;
//! * Steam's play time and last played date go to the Steam entries when
//!   they are ahead (Steam counts the sessions started here too).

use std::collections::HashSet;
use std::sync::OnceLock;

use anyhow::{anyhow, Result};
use heronite_data::models::{LocalMetaFile, TimestampFile};
use heronite_data::steam_owned::{unix_to_iso, SteamOwnedFile, SteamOwnedGame};
use heronite_data::{json_store, DataPaths, Library};
use heronite_launch::install_state::steam_sideload_entry;
use heronite_launch::steam::{is_non_game_steam_app, SteamInstalledGame};
use heronite_shortcuts::steam_client::SteamClient;
use regex::Regex;
use serde_json::{json, Map, Value};

/// What an import did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Games in the Steam library (after the filters).
    pub owned: usize,
    /// New Steam entries in the Collection.
    pub added: usize,
    /// Steam entries whose play time or last played date moved up.
    pub playtime: usize,
}

/// Steam names of things that are not games to play: test servers,
/// playtests, benchmarks (plus [`is_non_game_steam_app`]).
fn not_a_game(name: &str, app_id: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?i)(\btest server\b|\bplaytest\b|\bbenchmark\b|\bpublic test\b)").unwrap());
    is_non_game_steam_app(name, app_id) || re.is_match(name)
}

/// The games of the client's library (`appStore.allApps`, the list the
/// Steam library shows).
pub fn read_owned(client: &SteamClient) -> Result<Vec<SteamOwnedGame>> {
    let value = client.eval(
        r#"(async () => {
      const hidden = (id) => { try { return collectionStore.BIsHidden(id) } catch (e) { return false } }
      return appStore.allApps
        .filter((a) => a.app_type === 1 && !a.BIsShortcut() && (a.BIsOwned() || a.BIsBorrowed()) && !a.BIsDemo() && !hidden(a.appid))
        .map((a) => ({
          appId: String(a.appid),
          name: a.display_name ?? '',
          installed: !!a.installed,
          minutes: a.minutes_playtime_forever ?? 0,
          lastPlayed: a.rt_last_time_played ?? 0,
        }))
    })()"#,
    )?;
    let list = value.as_array().ok_or_else(|| anyhow!("the Steam client did not return its library"))?;
    Ok(list
        .iter()
        .filter_map(|g| {
            let app_id = g.get("appId")?.as_str()?.to_string();
            let name = g.get("name").and_then(Value::as_str).unwrap_or_default().trim().to_string();
            if app_id.is_empty() || name.is_empty() || not_a_game(&name, &app_id) {
                return None;
            }
            Some(SteamOwnedGame {
                app_id,
                name,
                installed: g.get("installed").and_then(Value::as_bool).unwrap_or(false),
                playtime_minutes: g.get("minutes").and_then(Value::as_u64).unwrap_or(0),
                last_played: g.get("lastPlayed").and_then(Value::as_i64).filter(|t| *t > 0),
                extra: Default::default(),
            })
        })
        .collect())
}

/// Saves `owned` as the cached library and joins it to the Collection.
/// `steam_bin` is the `steam` command installed entries point at. Blocking.
pub fn import(paths: &DataPaths, owned: Vec<SteamOwnedGame>, steam_bin: Option<&str>) -> Result<ImportReport> {
    let mut report = ImportReport { owned: owned.len(), ..Default::default() };
    json_store::update::<SteamOwnedFile, _>(&paths.steam_owned(), |f| {
        f.games = owned.clone();
        f.updated_at = Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
        Ok(())
    })?;

    let metas: LocalMetaFile = json_store::read(&paths.local_meta())?;
    // Ids the Collection has already: Steam entries, ids other entries name,
    // and the twins found by title (a game hidden in the Collection counts).
    let mut known: HashSet<String> = metas.games.values().filter_map(|m| m.steam_app_id.clone()).collect();
    known.extend(metas.games.keys().filter_map(|k| k.strip_prefix("steam_")).map(str::to_string));
    let lib = Library::load_with(paths, true)?;
    known.extend(lib.games.iter().filter_map(|g| g.steam_link.as_ref().map(|l| l.app_id.clone())));
    known.extend(lib.games.iter().filter_map(|g| g.app_name.strip_prefix("steam_").map(str::to_string)));

    let statuses = heronite_data::status::load_statuses(paths).unwrap_or_else(|_| heronite_data::status::default_statuses());
    let new: Vec<_> = owned
        .iter()
        .filter(|g| !known.contains(&g.app_id))
        .map(|g| {
            let status = heronite_data::status::inferred_status_id(g.playtime_minutes as f64, &statuses);
            let (mut info, meta) =
                steam_sideload_entry(&SteamInstalledGame { app_id: g.app_id.clone(), name: g.name.clone() }, steam_bin, &status);
            if !g.installed {
                info.is_installed = false;
                info.install.executable = Some(String::new());
            }
            (info, meta)
        })
        .collect();
    report.added = new.len();
    if !new.is_empty() {
        json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |file| {
            for (_, meta) in &new {
                file.games.entry(meta.app_name.clone()).or_insert_with(|| meta.clone());
            }
            Ok(())
        })?;
        json_store::update_value(&paths.sideload_library(), |doc| {
            if !doc.is_object() {
                *doc = Value::Object(Map::new());
            }
            let obj = doc.as_object_mut().expect("object");
            let games = obj.entry("games").or_insert_with(|| Value::Array(Vec::new()));
            if !games.is_array() {
                *games = Value::Array(Vec::new());
            }
            let list = games.as_array_mut().expect("array");
            for (info, _) in &new {
                list.push(serde_json::to_value(info)?);
            }
            Ok(())
        })?;
        log::info!("Added {} game(s) from the Steam library to the Collection", new.len());
    }

    // Play time of the Steam entries (new ones included).
    let metas: LocalMetaFile = json_store::read(&paths.local_meta())?;
    let entries: Vec<(String, &SteamOwnedGame)> = metas
        .games
        .values()
        .filter(|m| m.launch_kind == "steam-uri")
        .filter_map(|m| Some((m.app_name.clone(), owned.iter().find(|g| Some(&g.app_id) == m.steam_app_id.as_ref())?)))
        .collect();
    report.playtime = sync_playtime(paths, &entries)?;
    Ok(report)
}

/// Raises `totalPlayed` / `lastPlayed` of each entry to Steam's when Steam's
/// is ahead. How many entries changed.
fn sync_playtime(paths: &DataPaths, entries: &[(String, &SteamOwnedGame)]) -> Result<usize> {
    let current: TimestampFile = json_store::read(&paths.timestamps())?;
    let behind = |app: &str, g: &SteamOwnedGame| {
        let t = current.get(app);
        let total = t.map(|t| t.total_played.get()).unwrap_or(0.0);
        let last = t.and_then(|t| t.last_played.clone()).unwrap_or_default();
        let steam_last = g.last_played.and_then(unix_to_iso).unwrap_or_default();
        (g.playtime_minutes as f64) > total || (!steam_last.is_empty() && steam_last > last)
    };
    let changed: Vec<&(String, &SteamOwnedGame)> = entries.iter().filter(|(app, g)| behind(app, g)).collect();
    if changed.is_empty() {
        return Ok(0);
    }
    json_store::update_value(&paths.timestamps(), |root| {
        if !root.is_object() {
            *root = Value::Object(Map::new());
        }
        let map = root.as_object_mut().expect("object");
        for (app, g) in &changed {
            let entry = map.entry(app.clone()).or_insert_with(|| json!({"firstPlayed": "", "lastPlayed": "", "totalPlayed": 0}));
            if !entry.is_object() {
                *entry = json!({});
            }
            let total = entry.get("totalPlayed").and_then(Value::as_f64).unwrap_or(0.0);
            if (g.playtime_minutes as f64) > total {
                entry["totalPlayed"] = json!(g.playtime_minutes);
            }
            if let Some(steam_last) = g.last_played.and_then(unix_to_iso) {
                let last = entry.get("lastPlayed").and_then(Value::as_str).unwrap_or("");
                if steam_last.as_str() > last {
                    entry["lastPlayed"] = json!(steam_last);
                }
                let first = entry.get("firstPlayed").and_then(Value::as_str).unwrap_or("");
                if first.is_empty() {
                    entry["firstPlayed"] = json!(steam_last);
                }
            }
        }
        Ok(())
    })?;
    Ok(changed.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn owned(id: &str, name: &str, installed: bool, minutes: u64, last: Option<i64>) -> SteamOwnedGame {
        SteamOwnedGame {
            app_id: id.into(),
            name: name.into(),
            installed,
            playtime_minutes: minutes,
            last_played: last,
            extra: Default::default(),
        }
    }

    fn w(p: PathBuf, s: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }

    #[test]
    fn filters_out_what_is_not_a_game() {
        assert!(not_a_game("Tom Clancy's Rainbow Six Siege - Test Server", "1"));
        assert!(not_a_game("Some Game Playtest", "2"));
        assert!(not_a_game("Proton 9.0", "3"));
        assert!(!not_a_game("Portal 2", "620"));
        assert!(!not_a_game("Demon's Souls", "5"));
    }

    #[test]
    fn imports_new_games_once_and_joins_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        w(
            paths.legendary_library(),
            r#"{"library":[{"runner":"legendary","app_name":"ctl","title":"Control","install":{}}],"__timestamp":{"library":"x"}}"#,
        );
        w(
            paths.sideload_library(),
            r#"{"games":[{"runner":"sideload","app_name":"steam_620","title":"Portal 2","is_installed":true,"install":{}}]}"#,
        );
        w(
            paths.local_meta(),
            r#"{"games":{"steam_620":{"appName":"steam_620","runner":"sideload","launchKind":"steam-uri","steamAppId":"620","title":"Portal 2"}}}"#,
        );
        w(
            paths.timestamps(),
            r#"{"steam_620":{"firstPlayed":"2016-01-01T00:00:00.000Z","lastPlayed":"2016-01-01T00:00:00.000Z","totalPlayed":90,"x":1}}"#,
        );
        let games = vec![
            owned("620", "Portal 2", true, 60, Some(1_499_373_907)),
            owned("870780", "Control", false, 0, None),
            owned("400", "Portal", false, 30, Some(1_400_000_000)),
            owned("10", "Counter-Strike", true, 0, None),
        ];
        let report = import(&paths, games.clone(), Some("/usr/bin/steam")).unwrap();
        assert_eq!(report, ImportReport { owned: 4, added: 2, playtime: 2 });

        let lib = Library::load(&paths).unwrap();
        let keys: Vec<_> = lib.games.iter().map(|g| g.key.as_str()).collect();
        assert_eq!(keys, ["sideload_steam_620", "sideload_steam_400", "sideload_steam_10", "legendary_ctl"]);
        let portal = lib.get("sideload_steam_400").unwrap();
        assert!(!portal.installed);
        assert_eq!(portal.steam_launch_id.as_deref(), Some("400"));
        assert_eq!(portal.playtime_minutes, 30);
        assert!(lib.get("sideload_steam_10").unwrap().installed);
        assert_eq!(lib.get("legendary_ctl").unwrap().steam_link.as_ref().map(|l| l.app_id.as_str()), Some("870780"));
        // Ours is ahead on minutes, Steam on the date.
        let t: Value = json_store::read_value(&paths.timestamps()).unwrap();
        assert_eq!(t["steam_620"]["totalPlayed"], 90);
        assert_eq!(t["steam_620"]["lastPlayed"], "2017-07-06T20:45:07.000Z");
        assert_eq!(t["steam_620"]["firstPlayed"], "2016-01-01T00:00:00.000Z");
        assert_eq!(t["steam_400"]["totalPlayed"], 30);
        assert_eq!(t["steam_620"]["x"], 1);

        // Again: nothing new.
        assert_eq!(import(&paths, games, Some("/usr/bin/steam")).unwrap(), ImportReport { owned: 4, added: 0, playtime: 0 });
    }
}
