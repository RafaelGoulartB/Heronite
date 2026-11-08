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
//!   they are ahead (Steam counts the sessions started here too);
//! * a Steam entry that is still "Not played", the status the scan of the
//!   installed games gave it before this library was read, becomes "Played"
//!   once Steam has play time for it, unless the user ever chose its status.

use std::collections::HashSet;
use std::sync::OnceLock;

use anyhow::{anyhow, bail, Result};
use heronite_data::models::{LocalMetaFile, TimestampFile};
use heronite_data::status::PlayedGame;
use heronite_data::steam_owned::{unix_to_iso, SteamOwnedFile, SteamOwnedGame, SteamSource};
use heronite_data::{json_store, DataPaths, Library};
use heronite_i18n::tr;
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
    /// Steam entries that went from "Not played" to "Played".
    pub promoted: usize,
}

impl ImportReport {
    /// Whether the Collection has to be read again.
    pub fn changed(&self) -> bool {
        self.added > 0 || self.playtime > 0 || self.promoted > 0
    }
}

/// Steam names of things that are not games to play: test servers,
/// playtests, benchmarks (plus [`is_non_game_steam_app`]).
pub(crate) fn not_a_game(name: &str, app_id: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?i)(\btest server\b|\bplaytest\b|\bbenchmark\b|\bpublic test\b)").unwrap());
    is_non_game_steam_app(name, app_id) || re.is_match(name)
}

/// Whether the client has loaded the account's library. Its shared context
/// answers a few seconds before that (right after Steam starts, and while
/// it signs in), with `appStore.allApps` still empty: a read then takes the
/// library for empty. Clients without the flag count as loaded once they
/// list an app.
const LOADED: &str = "(typeof appStore.m_bIsInitialized === 'boolean' ? appStore.m_bIsInitialized : appStore.allApps.length > 0)";

/// Whether the client has loaded the account's library yet ([`LOADED`]).
pub fn library_loaded(client: &SteamClient) -> Result<bool> {
    client.eval(LOADED)?.as_bool().ok_or_else(|| anyhow!("the Steam client did not say whether its library is loaded"))
}

/// The games of the client's library (`appStore.allApps`, the list the
/// Steam library shows). Fails while the client is still loading it.
pub fn read_owned(client: &SteamClient) -> Result<Vec<SteamOwnedGame>> {
    let script = r#"(async () => {
      if (!LOADED) return null
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
    })()"#;
    let value = client.eval(&script.replacen("LOADED", LOADED, 1))?;
    if value.is_null() {
        bail!("{}", tr!("Steam is still loading your library. It is read by itself as soon as Steam has it."));
    }
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

/// Saves `owned` (read from the client) as the cached library and joins it
/// to the Collection. `steam_bin` is the `steam` command installed entries
/// point at. Blocking.
pub fn import(paths: &DataPaths, owned: Vec<SteamOwnedGame>, steam_bin: Option<&str>) -> Result<ImportReport> {
    import_from(paths, owned, steam_bin, SteamSource::Client)
}

/// [`import`] of a library read from `source`.
pub fn import_from(paths: &DataPaths, owned: Vec<SteamOwnedGame>, steam_bin: Option<&str>, source: SteamSource) -> Result<ImportReport> {
    let mut report = ImportReport { owned: owned.len(), ..Default::default() };
    json_store::update::<SteamOwnedFile, _>(&paths.steam_owned(), |f| {
        f.games = owned.clone();
        f.updated_at = Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
        f.source = Some(source);
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
    report.promoted = promote_played(paths, &entries)?;
    if report.promoted > 0 {
        log::info!("Marked {} Steam game(s) as Played: Steam has play time for them", report.promoted);
    }
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

/// Moves the entries that are still "Not played" to "Played" when Steam has
/// play time for them: the status they would have got had this library been
/// read before the scan of the installed games added them (the rule is
/// [`heronite_data::status::promote_played`]). How many entries changed.
fn promote_played(paths: &DataPaths, entries: &[(String, &SteamOwnedGame)]) -> Result<usize> {
    let games: Vec<PlayedGame> = entries
        .iter()
        .map(|(app, g)| PlayedGame { app_name: app.clone(), minutes: g.playtime_minutes, extras: Default::default() })
        .collect();
    heronite_data::status::promote_played(paths, &games)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

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
        assert_eq!(report, ImportReport { owned: 4, added: 2, playtime: 2, promoted: 1 });

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
        assert_eq!(import(&paths, games, Some("/usr/bin/steam")).unwrap(), ImportReport { owned: 4, added: 0, playtime: 0, promoted: 0 });
    }

    /// Steam entries as the scan of the installed games left them: `(app id,
    /// stored status)`.
    fn steam_entries(paths: &DataPaths, entries: &[(&str, Option<&str>)]) {
        let games: Map<String, Value> = entries
            .iter()
            .map(|(id, status)| {
                let app = format!("steam_{id}");
                let mut meta =
                    json!({"appName": app, "runner": "sideload", "launchKind": "steam-uri", "steamAppId": id, "title": app, "zz": 1});
                if let Some(status) = status {
                    meta["completionStatusId"] = json!(status);
                }
                (app, meta)
            })
            .collect();
        w(paths.local_meta(), &json!({ "games": games }).to_string());
    }

    fn status_of(paths: &DataPaths, app_id: &str) -> Option<String> {
        let metas: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap();
        metas.games[&format!("steam_{app_id}")].completion_status_id.clone()
    }

    #[test]
    fn an_entry_still_not_played_becomes_played_once_steam_has_play_time() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        steam_entries(&paths, &[("1", Some("not-played")), ("2", None), ("3", Some("")), ("4", Some("not-played"))]);
        let games = vec![
            owned("1", "A", true, 3748, None),
            owned("2", "B", true, 10, None),
            owned("3", "C", false, 1, None),
            owned("4", "D", true, 0, None),
        ];

        let report = import(&paths, games, None).unwrap();
        assert_eq!((report.added, report.promoted), (0, 3));
        assert!(report.changed());
        for id in ["1", "2", "3"] {
            assert_eq!(status_of(&paths, id).as_deref(), Some("played"), "{id}");
        }
        assert_eq!(status_of(&paths, "4").as_deref(), Some("not-played"), "no play time on Steam either");
        // The rest of the entry stays as it was, and no status change is recorded.
        let doc = json_store::read_value(&paths.local_meta()).unwrap();
        assert_eq!(doc["games"]["steam_1"]["zz"], 1);
        assert_eq!(doc["games"]["steam_1"]["title"], "steam_1");
        assert!(heronite_data::status_history::load(&paths).is_empty());
    }

    #[test]
    fn a_status_the_user_chose_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let custom = heronite_data::status::new_custom_status("Backlog", &heronite_data::status::default_statuses());
        heronite_data::status::upsert_status(&paths, custom.clone()).unwrap();
        steam_entries(&paths, &[("1", Some("beaten")), ("2", Some(custom.id.as_str())), ("3", Some("played"))]);
        // Back to Not played by hand: the status history remembers the change.
        heronite_data::status::set_game_status(&paths, "steam_3", "not-played", &Default::default()).unwrap();
        let games = vec![owned("1", "A", true, 600, None), owned("2", "B", true, 600, None), owned("3", "C", true, 600, None)];

        let report = import(&paths, games, None).unwrap();
        assert_eq!(report.promoted, 0);
        assert_eq!(status_of(&paths, "1").as_deref(), Some("beaten"));
        assert_eq!(status_of(&paths, "2"), Some(custom.id));
        assert_eq!(status_of(&paths, "3").as_deref(), Some("not-played"));
    }

    #[test]
    fn the_played_and_not_played_statuses_are_the_ones_of_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        w(
            paths.statuses(),
            r#"{"statuses":[{"id":"s1","name":"Never","slug":"not-played","sortOrder":10},{"id":"s2","name":"Tried","slug":"played","sortOrder":20}]}"#,
        );
        steam_entries(&paths, &[("1", Some("s1"))]);
        assert_eq!(import(&paths, vec![owned("1", "A", true, 90, None)], None).unwrap().promoted, 1);
        assert_eq!(status_of(&paths, "1").as_deref(), Some("s2"));
    }

    #[test]
    fn nothing_moves_to_a_played_the_user_does_not_have() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        // The inference would fall back to the first status, Beaten.
        w(
            paths.statuses(),
            r#"{"statuses":[{"id":"beaten","name":"Beaten","slug":"beaten","sortOrder":10},{"id":"not-played","name":"Not Played","slug":"not-played","sortOrder":20}]}"#,
        );
        steam_entries(&paths, &[("1", Some("not-played"))]);
        let report = import(&paths, vec![owned("1", "A", true, 90, None)], None).unwrap();
        assert_eq!(report.promoted, 0);
        assert_eq!(status_of(&paths, "1").as_deref(), Some("not-played"));
    }

    #[test]
    fn nothing_moves_without_a_not_played_in_the_list() {
        // Deleting "Not played" moves its games to the first status left,
        // which the inference would then take for the automatic one.
        for (first, second) in [("beaten", "played"), ("played", "beaten")] {
            let dir = tempfile::tempdir().unwrap();
            let paths = DataPaths::at(dir.path());
            let statuses = json!({"statuses": [
                {"id": first, "name": first, "slug": first, "sortOrder": 10},
                {"id": second, "name": second, "slug": second, "sortOrder": 20},
            ]});
            w(paths.statuses(), &statuses.to_string());
            steam_entries(&paths, &[("1", Some("beaten")), ("2", None)]);
            let report = import(&paths, vec![owned("1", "A", true, 90, None), owned("2", "B", true, 90, None)], None).unwrap();
            assert_eq!(report.promoted, 0, "{first} first");
            assert_eq!(status_of(&paths, "1").as_deref(), Some("beaten"), "{first} first");
            assert_eq!(status_of(&paths, "2"), None, "{first} first");
        }
    }

    #[test]
    fn importing_again_does_not_rewrite_the_entries_or_their_play_time() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        steam_entries(&paths, &[("1", Some("not-played"))]);
        let games = vec![owned("1", "A", true, 90, Some(1_499_373_907))];
        assert_eq!(import(&paths, games.clone(), None).unwrap().promoted, 1);

        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(86_400);
        let files = [paths.local_meta(), paths.timestamps()];
        for file in &files {
            fs::File::options().write(true).open(file).unwrap().set_modified(old).unwrap();
        }
        assert!(!import(&paths, games, None).unwrap().changed());
        for file in &files {
            assert_eq!(json_store::mtime(file), Some(old), "{}", file.display());
        }
    }

    /// A fake client whose library is loaded or not: what a real one
    /// answers right after its shared context comes up, and seconds later.
    fn client(loaded: bool) -> (heronite_shortcuts::steam_client::testing::FakeSteam, SteamClient) {
        let fake = heronite_shortcuts::steam_client::testing::start(Box::new(move |expr: &str| {
            if expr == LOADED {
                json!(loaded)
            } else if !loaded {
                Value::Null
            } else {
                json!([{"appId": "620", "name": "Portal 2", "installed": true, "minutes": 125, "lastPlayed": 1_499_373_907}])
            }
        }));
        let client = SteamClient::new(fake.addr.clone());
        (fake, client)
    }

    #[test]
    fn a_library_still_loading_is_not_read_as_empty() {
        let (fake, client) = client(false);
        assert!(!library_loaded(&client).unwrap());
        let err = read_owned(&client).unwrap_err().to_string();
        assert!(err.contains("still loading"), "{err}");
        // The read itself checks too: the client may restart between the two.
        let exprs = fake.expressions.lock().unwrap();
        assert!(exprs.last().unwrap().contains(&format!("if (!{LOADED}) return null")), "{}", exprs.last().unwrap());
    }

    #[test]
    fn a_loaded_library_is_read() {
        let (_fake, client) = client(true);
        assert!(library_loaded(&client).unwrap());
        let games = read_owned(&client).unwrap();
        assert_eq!(games.len(), 1);
        assert_eq!((games[0].app_id.as_str(), games[0].playtime_minutes, games[0].installed), ("620", 125, true));
    }
}
