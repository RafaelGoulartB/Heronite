//! The account's Steam library, read from the running client (no Steam
//! login or API key: the client is signed in already), cached in
//! `store_cache/steam_library.json` and joined to the Collection:
//!
//! * only games: no tools, soundtracks, demos, test servers or playtests;
//! * a game hidden in Steam comes in hidden in the Collection too (shown
//!   with "Show hidden games"). Only when its entry is added: unhidden here
//!   later, it stays unhidden;
//! * a game the Collection has from another source becomes that entry's
//!   Steam twin (`heronite_data::steam_owned`) instead of a new entry;
//! * the others are added as Steam entries, installed or not;
//! * games another member of the Steam Family shares (the client lists them
//!   with their owner's account) are added as Steam entries too, marked
//!   ([`FAMILY_KEY`]) so they leave once the game is neither shared nor
//!   owned any more; games Steam does not share are left out;
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
use heronite_data::steam_owned::{steamid64, unix_to_iso, FamilyShare, SteamOwnedFile, SteamOwnedGame, SteamSource, FAMILY_KEY};
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
    /// New Steam entries in the Collection, not counting `hidden_added`.
    pub added: usize,
    /// Steam entries whose play time or last played date moved up.
    pub playtime: usize,
    /// Steam entries that went from "Not played" to "Played".
    pub promoted: usize,
    /// Games the Steam Family shares (after the filters).
    pub family: usize,
    /// New Steam entries for games the Steam Family shares (not in `added`
    /// nor in `hidden_added`).
    pub family_added: usize,
    /// New Steam entries, the account's or the Family's, for games hidden in
    /// Steam: added hidden in the Collection.
    pub hidden_added: usize,
    /// Family entries removed: the game is neither shared nor owned any more.
    pub family_removed: usize,
}

impl ImportReport {
    /// Whether the Collection has to be read again.
    pub fn changed(&self) -> bool {
        self.added > 0
            || self.playtime > 0
            || self.promoted > 0
            || self.family_added > 0
            || self.hidden_added > 0
            || self.family_removed > 0
    }
}

/// What a read of the client found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LibraryRead {
    /// The account's own games.
    pub owned: Vec<SteamOwnedGame>,
    /// Games another member of the Steam Family shares with the account,
    /// each with its [`SteamOwnedGame::share`].
    pub family: Vec<SteamOwnedGame>,
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
/// Steam library shows): the account's own, and the ones its Steam Family
/// shares. A shared app names its owner (`owner_account_id`, what the
/// client's own `BIsOwnedByAnotherUser` checks) and is not `subscribed_to`;
/// its details tell whether Steam shares it at all and whether someone
/// holds the family's copy (`strLockedBySteamID`, account 0 when free). The
/// owners' names come from the friends list's player store. No Steam login
/// or token: everything is what the client shows already. Fails while the
/// client is still loading the library.
pub fn read_library(client: &SteamClient) -> Result<LibraryRead> {
    let script = r#"(async () => {
      if (!LOADED) return null
      const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
      const hidden = (id) => { try { return collectionStore.BIsHidden(id) } catch (e) { return false } }
      const row = (a) => ({
        appId: String(a.appid),
        name: a.display_name ?? '',
        installed: !!a.installed,
        hidden: hidden(a.appid),
        minutes: a.minutes_playtime_forever ?? 0,
        lastPlayed: a.rt_last_time_played ?? 0,
      })
      const games = appStore.allApps.filter((a) => a.app_type === 1 && !a.BIsShortcut() && !a.BIsDemo())
      const owned = games.filter((a) => a.BIsOwned() && !a.owner_account_id)
      const shared = games.filter((a) => a.visible_in_game_list && !!a.owner_account_id)
      const details = (id) => { try { return appDetailsStore.GetAppDetails(id) } catch (e) { return null } }
      for (const a of shared) if (!details(a.appid)) try { appDetailsStore.RequestAppDetails(a.appid) } catch (e) {}
      for (let i = 0; i < 30 && shared.some((a) => !details(a.appid)); i++) await sleep(100)
      const players = (typeof g_FriendsUIApp === 'object' && g_FriendsUIApp && g_FriendsUIApp.FriendStore) || null
      const names = {}
      // All owners at once: one second at most, however many there are.
      await Promise.all([...new Set(shared.map((a) => a.owner_account_id))].map(async (id) => {
        try {
          const p = players && players.GetPlayer(id)
          for (let i = 0; i < 10 && p && !p.is_ready; i++) await sleep(100)
          names[id] = (p && p.is_ready && p.display_name) || ''
        } catch (e) { names[id] = '' }
      }))
      const me = (typeof App === 'object' && App && App.m_CurrentUser && App.m_CurrentUser.strSteamID) || ''
      return {
        owned: owned.map(row),
        family: shared.map((a) => {
          const d = details(a.appid)
          return Object.assign(row(a), {
            ownerAccount: a.owner_account_id,
            ownerName: names[a.owner_account_id] ?? '',
            excluded: !!(d && d.bIsExcludedFromSharing),
            lockedBy: (d && d.strLockedBySteamID) || '',
            me,
          })
        }),
      }
    })()"#;
    let value = client.eval(&script.replacen("LOADED", LOADED, 1))?;
    if value.is_null() {
        bail!("{}", tr!("Steam is still loading your library. It is read by itself as soon as Steam has it."));
    }
    let list = |key: &str| value.get(key).and_then(Value::as_array).cloned();
    let owned = list("owned").ok_or_else(|| anyhow!("the Steam client did not return its library"))?;
    Ok(LibraryRead {
        owned: owned.iter().filter_map(game_of).collect(),
        family: list("family").unwrap_or_default().iter().filter_map(shared_game_of).collect(),
    })
}

/// A game of the client's answer, unless it is not a game to play.
fn game_of(g: &Value) -> Option<SteamOwnedGame> {
    let app_id = g.get("appId")?.as_str()?.to_string();
    let name = g.get("name").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    if app_id.is_empty() || name.is_empty() || not_a_game(&name, &app_id) {
        return None;
    }
    Some(SteamOwnedGame {
        app_id,
        name,
        installed: g.get("installed").and_then(Value::as_bool).unwrap_or(false),
        hidden: g.get("hidden").and_then(Value::as_bool).unwrap_or(false),
        playtime_minutes: g.get("minutes").and_then(Value::as_u64).unwrap_or(0),
        last_played: g.get("lastPlayed").and_then(Value::as_i64).filter(|t| *t > 0),
        share: None,
        extra: Default::default(),
    })
}

/// A shared game of the client's answer, unless Steam does not share it.
fn shared_game_of(g: &Value) -> Option<SteamOwnedGame> {
    if g.get("excluded").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    let owner = u32::try_from(g.get("ownerAccount").and_then(Value::as_u64)?).ok().filter(|id| *id > 0)?;
    let me = g.get("me").and_then(Value::as_str).unwrap_or_default();
    let locked = g.get("lockedBy").and_then(Value::as_str).unwrap_or_default();
    let mut game = game_of(g)?;
    game.share = Some(FamilyShare {
        owner_id: steamid64(owner),
        owner_name: g.get("ownerName").and_then(Value::as_str).unwrap_or_default().trim().to_string(),
        in_use: held_by_another(locked, me),
    });
    Some(game)
}

/// Whether `locked` (the Steam id holding the family's copy) is someone
/// other than `me`: the id of account 0 (or none) means the copy is free.
fn held_by_another(locked: &str, me: &str) -> bool {
    let locked = locked.trim();
    locked.parse::<u64>().is_ok_and(|id| id & 0xFFFF_FFFF != 0) && locked != me.trim()
}

/// Saves what the client showed as the cached library and joins it to the
/// Collection. `steam_bin` is the `steam` command installed entries point
/// at. Blocking.
pub fn import(paths: &DataPaths, read: LibraryRead, steam_bin: Option<&str>) -> Result<ImportReport> {
    import_from(paths, read, steam_bin, SteamSource::Client)
}

/// [`import`] of a library read from `source`. Only the client shows the
/// Steam Family's games: a read of the account keeps the ones cached
/// (`read.family` is not looked at) and removes none of their entries.
pub fn import_from(paths: &DataPaths, read: LibraryRead, steam_bin: Option<&str>, source: SteamSource) -> Result<ImportReport> {
    let LibraryRead { owned, family } = read;
    let family = if source == SteamSource::Client { family } else { SteamOwnedFile::read(paths).family };
    let owned_ids: HashSet<String> = owned.iter().map(|g| g.app_id.clone()).collect();
    // A game the account owns too is its own, not the family's.
    let family: Vec<SteamOwnedGame> = family.into_iter().filter(|g| g.share.is_some() && !owned_ids.contains(&g.app_id)).collect();
    let family_ids: HashSet<String> = family.iter().map(|g| g.app_id.clone()).collect();
    let mut report = ImportReport { owned: owned.len(), family: family.len(), ..Default::default() };
    json_store::update::<SteamOwnedFile, _>(&paths.steam_owned(), |f| {
        f.games = owned.clone();
        f.family = family.clone();
        f.updated_at = Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
        f.source = Some(source);
        Ok(())
    })?;

    let metas: LocalMetaFile = json_store::read(&paths.local_meta())?;
    // Family entries whose game is neither shared nor owned any more.
    let gone: HashSet<String> = metas
        .games
        .values()
        .filter(|m| m.launch_kind == "steam-uri" && m.extra.get(FAMILY_KEY) == Some(&Value::Bool(true)))
        .filter(|m| m.steam_app_id.as_ref().is_none_or(|id| !owned_ids.contains(id) && !family_ids.contains(id)))
        .map(|m| m.app_name.clone())
        .collect();
    // Ids the Collection has already: Steam entries, ids other entries name,
    // and the twins found by title (a game hidden in the Collection counts).
    let mut known: HashSet<String> =
        metas.games.values().filter(|m| !gone.contains(&m.app_name)).filter_map(|m| m.steam_app_id.clone()).collect();
    known.extend(metas.games.keys().filter(|k| !gone.contains(*k)).filter_map(|k| k.strip_prefix("steam_")).map(str::to_string));
    let lib = Library::load_with(paths, true)?;
    known.extend(lib.games.iter().filter_map(|g| g.steam_link.as_ref().map(|l| l.app_id.clone())));
    known.extend(
        lib.games.iter().filter(|g| !gone.contains(&g.app_name)).filter_map(|g| g.app_name.strip_prefix("steam_").map(str::to_string)),
    );

    let statuses = heronite_data::status::load_statuses(paths).unwrap_or_else(|_| heronite_data::status::default_statuses());
    // A Family game shared again gets back the status it left with: its
    // entry went when sharing ended, the status history stayed.
    let history =
        if family.iter().any(|g| !known.contains(&g.app_id)) { heronite_data::status_history::load(paths) } else { Default::default() };
    let left_with = |g: &SteamOwnedGame| {
        let changes = history.get(&format!("steam_{}", g.app_id))?;
        changes.last().map(|c| c.status.clone()).filter(|id| statuses.iter().any(|s| s.id == *id))
    };
    let entry = |g: &SteamOwnedGame| {
        let status = g
            .share
            .as_ref()
            .and_then(|_| left_with(g))
            .unwrap_or_else(|| heronite_data::status::inferred_status_id(g.playtime_minutes as f64, &statuses));
        let (mut info, mut meta) =
            steam_sideload_entry(&SteamInstalledGame { app_id: g.app_id.clone(), name: g.name.clone() }, steam_bin, &status);
        if !g.installed {
            info.is_installed = false;
            info.install.executable = Some(String::new());
        }
        if g.share.is_some() {
            meta.extra.insert(FAMILY_KEY.into(), Value::Bool(true));
        }
        (info, meta)
    };
    let new_games: Vec<&SteamOwnedGame> = owned.iter().chain(&family).filter(|g| !known.contains(&g.app_id)).collect();
    let new: Vec<_> = new_games.iter().map(|g| entry(g)).collect();
    // Hidden in Steam: hidden here too, from the start.
    let hide: Vec<(String, String, bool)> = new_games
        .iter()
        .zip(&new)
        .filter(|(g, _)| g.hidden)
        .map(|(_, (info, meta))| (meta.app_name.clone(), info.title.clone(), true))
        .collect();
    report.hidden_added = hide.len();
    report.family_added = new_games.iter().filter(|g| g.share.is_some() && !g.hidden).count();
    report.added = new.len() - report.family_added - report.hidden_added;
    report.family_removed = gone.len();
    if !new.is_empty() || !gone.is_empty() {
        // Before the entries: a game is hidden only when its entry is added,
        // so an entry written without its mark would stay shown for good. A
        // mark without its entry is set again by the next read.
        heronite_data::config_games::set_hidden(paths, &hide)?;
        json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |file| {
            file.games.retain(|app, _| !gone.contains(app));
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
            list.retain(|g| g.get("app_name").and_then(Value::as_str).is_none_or(|app| !gone.contains(app)));
            for (info, _) in &new {
                list.push(serde_json::to_value(info)?);
            }
            Ok(())
        })?;
        if report.added > 0 {
            log::info!("Added {} game(s) from the Steam library to the Collection", report.added);
        }
        if report.family_added > 0 {
            log::info!("Added {} game(s) the Steam Family shares to the Collection", report.family_added);
        }
        if report.hidden_added > 0 {
            log::info!("Added {} game(s) hidden in Steam to the Collection, hidden", report.hidden_added);
        }
        if !gone.is_empty() {
            log::info!("Removed {} Steam Family game(s) that are no longer shared", gone.len());
        }
    }

    // Play time of the Steam entries (new ones included): the account's own
    // time, shared games included.
    let metas: LocalMetaFile = json_store::read(&paths.local_meta())?;
    let entries: Vec<(String, &SteamOwnedGame)> = metas
        .games
        .values()
        .filter(|m| m.launch_kind == "steam-uri")
        .filter_map(|m| {
            let id = m.steam_app_id.as_ref()?;
            Some((m.app_name.clone(), owned.iter().chain(&family).find(|g| &g.app_id == id)?))
        })
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
            share: None,
            hidden: false,
            extra: Default::default(),
        }
    }

    /// An import of a read with only the account's own games.
    fn import_owned(paths: &DataPaths, games: Vec<SteamOwnedGame>, steam_bin: Option<&str>) -> Result<ImportReport> {
        import(paths, LibraryRead { owned: games, family: Vec::new() }, steam_bin)
    }

    /// A game the Steam Family shares (fabricated owner).
    fn shared(id: &str, name: &str, installed: bool, minutes: u64) -> SteamOwnedGame {
        SteamOwnedGame {
            share: Some(FamilyShare { owner_id: steamid64(1001), owner_name: "Riley".into(), in_use: false }),
            ..owned(id, name, installed, minutes, None)
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
        let report = import_owned(&paths, games.clone(), Some("/usr/bin/steam")).unwrap();
        assert_eq!(report, ImportReport { owned: 4, added: 2, playtime: 2, promoted: 1, ..Default::default() });

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
        assert_eq!(import_owned(&paths, games, Some("/usr/bin/steam")).unwrap(), ImportReport { owned: 4, ..Default::default() });
    }

    #[test]
    fn games_hidden_in_steam_come_in_hidden_once() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let hidden = |g: SteamOwnedGame| SteamOwnedGame { hidden: true, ..g };
        let read = || LibraryRead {
            owned: vec![hidden(owned("620", "Portal 2", false, 0, None)), owned("400", "Portal", false, 0, None)],
            family: vec![hidden(shared("10", "Counter-Strike", false, 0))],
        };
        let report = import(&paths, read(), Some("/usr/bin/steam")).unwrap();
        assert_eq!(report, ImportReport { owned: 2, family: 1, added: 1, hidden_added: 2, ..Default::default() });
        assert!(report.changed());
        let shown: Vec<_> = Library::load(&paths).unwrap().games.iter().map(|g| g.app_name.clone()).collect();
        assert_eq!(shown, ["steam_400"]);
        let all = Library::load_with(&paths, true).unwrap();
        let flags: Vec<_> = all.games.iter().map(|g| (g.app_name.as_str(), g.hidden)).collect();
        assert_eq!(flags, [("steam_620", true), ("steam_400", false), ("steam_10", true)]);
        let list = heronite_data::config_games::list_entries(&paths, "hidden");
        assert_eq!(list, [("steam_620".to_string(), "Portal 2".to_string()), ("steam_10".to_string(), "Counter-Strike".to_string())]);
        // The cache keeps the mark, for the scan of the installed games.
        assert!(SteamOwnedFile::read(&paths).get("620").unwrap().hidden);

        // Unhidden here: the next read leaves it shown.
        heronite_data::config_games::unhide_game(&paths, "steam_620").unwrap();
        let again = import(&paths, read(), Some("/usr/bin/steam")).unwrap();
        assert_eq!(again, ImportReport { owned: 2, family: 1, ..Default::default() });
        assert_eq!(heronite_data::config_games::list_entries(&paths, "hidden").len(), 1);
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

        let report = import_owned(&paths, games, None).unwrap();
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

        let report = import_owned(&paths, games, None).unwrap();
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
        assert_eq!(import_owned(&paths, vec![owned("1", "A", true, 90, None)], None).unwrap().promoted, 1);
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
        let report = import_owned(&paths, vec![owned("1", "A", true, 90, None)], None).unwrap();
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
            let report = import_owned(&paths, vec![owned("1", "A", true, 90, None), owned("2", "B", true, 90, None)], None).unwrap();
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
        assert_eq!(import_owned(&paths, games.clone(), None).unwrap().promoted, 1);

        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(86_400);
        let files = [paths.local_meta(), paths.timestamps()];
        for file in &files {
            fs::File::options().write(true).open(file).unwrap().set_modified(old).unwrap();
        }
        assert!(!import_owned(&paths, games, None).unwrap().changed());
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
                json!({
                    "owned": [{"appId": "620", "name": "Portal 2", "installed": true, "minutes": 125, "lastPlayed": 1_499_373_907}],
                    "family": [
                        {"appId": "400", "name": "Portal", "installed": false, "minutes": 30, "lastPlayed": 0,
                         "ownerAccount": 1001, "ownerName": "Riley", "excluded": false, "lockedBy": "76561197960265728", "me": ME},
                        {"appId": "500", "name": "Left 4 Dead", "installed": true, "minutes": 0, "lastPlayed": 0,
                         "ownerAccount": 1002, "ownerName": "", "excluded": false, "lockedBy": "76561197960266730", "me": ME},
                        {"appId": "550", "name": "Left 4 Dead 2", "installed": false, "minutes": 0, "lastPlayed": 0,
                         "ownerAccount": 1001, "ownerName": "Riley", "excluded": true, "lockedBy": "", "me": ME},
                        {"appId": "70", "name": "Half-Life", "installed": false, "minutes": 0, "lastPlayed": 0,
                         "ownerAccount": 1001, "ownerName": "Riley", "excluded": false, "lockedBy": ME, "me": ME},
                        {"appId": "80", "name": "No Owner", "installed": false, "minutes": 0, "lastPlayed": 0, "ownerAccount": 0}
                    ]
                })
            }
        }));
        let client = SteamClient::new(fake.addr.clone());
        (fake, client)
    }

    #[test]
    fn a_library_still_loading_is_not_read_as_empty() {
        let (fake, client) = client(false);
        assert!(!library_loaded(&client).unwrap());
        let err = read_library(&client).unwrap_err().to_string();
        assert!(err.contains("still loading"), "{err}");
        // The read itself checks too: the client may restart between the two.
        let exprs = fake.expressions.lock().unwrap();
        assert!(exprs.last().unwrap().contains(&format!("if (!{LOADED}) return null")), "{}", exprs.last().unwrap());
    }

    #[test]
    fn a_loaded_library_is_read() {
        let (_fake, client) = client(true);
        assert!(library_loaded(&client).unwrap());
        let read = read_library(&client).unwrap();
        let games = &read.owned;
        assert_eq!(games.len(), 1);
        assert_eq!((games[0].app_id.as_str(), games[0].playtime_minutes, games[0].installed), ("620", 125, true));
        assert_eq!(games[0].share, None);
        // Shared games: Steam's exclusions and ownerless rows are left out.
        let family: Vec<_> = read.family.iter().map(|g| (g.app_id.as_str(), g.share.clone().unwrap())).collect();
        let share = |owner: u32, name: &str, in_use: bool| FamilyShare { owner_id: steamid64(owner), owner_name: name.into(), in_use };
        assert_eq!(family, [("400", share(1001, "Riley", false)), ("500", share(1002, "", true)), ("70", share(1001, "Riley", false))]);
        assert_eq!(read.family[0].playtime_minutes, 30);
        assert!(read.family[1].installed);
    }

    #[test]
    fn only_another_account_holding_the_copy_puts_it_in_use() {
        assert!(!held_by_another("", ME));
        assert!(!held_by_another("76561197960265728", ME), "account 0: free");
        assert!(!held_by_another(ME, ME));
        assert!(held_by_another("76561197960266730", ME));
        assert!(!held_by_another("not a number", ME));
    }

    /// The signed-in account of the fake client (fabricated).
    const ME: &str = "76561197960266729";

    fn family_meta(paths: &DataPaths, app: &str) -> Option<Value> {
        let doc = json_store::read_value(&paths.local_meta()).unwrap();
        doc["games"].get(app).cloned()
    }

    #[test]
    fn family_games_join_apart_and_leave_when_sharing_ends() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        w(
            paths.legendary_library(),
            r#"{"library":[{"runner":"legendary","app_name":"ctl","title":"Control","install":{}}],"__timestamp":{"library":"x"}}"#,
        );
        let read = LibraryRead {
            owned: vec![owned("620", "Portal 2", true, 60, None)],
            family: vec![
                shared("400", "Portal", false, 90),
                // Owned as well: the account's own.
                shared("620", "Portal 2", true, 0),
                // On Epic already: a Family game never joins it, and needs no entry.
                shared("870780", "Control", false, 0),
                shared("10", "Counter-Strike", true, 0),
            ],
        };
        let report = import(&paths, read.clone(), None).unwrap();
        assert_eq!((report.owned, report.added, report.family, report.family_added, report.family_removed), (1, 1, 3, 3, 0));
        let file = SteamOwnedFile::read(&paths);
        assert_eq!(file.family.iter().map(|g| g.app_id.as_str()).collect::<Vec<_>>(), ["400", "870780", "10"]);
        assert_eq!(family_meta(&paths, "steam_400").unwrap()[FAMILY_KEY], true);
        assert_eq!(family_meta(&paths, "steam_620").unwrap().get(FAMILY_KEY), None);

        let lib = Library::load(&paths).unwrap();
        let portal = lib.get("sideload_steam_400").unwrap();
        assert_eq!(portal.family.as_ref().map(|f| f.owner()), Some("Riley"));
        assert_eq!(portal.source_label, heronite_data::library::FAMILY_LABEL);
        assert_eq!(portal.steam_launch_id.as_deref(), Some("400"));
        assert_eq!(portal.playtime_minutes, 90, "the account's own time on a shared game");
        assert!(lib.get("sideload_steam_10").unwrap().installed);
        assert_eq!(lib.get("sideload_steam_620").unwrap().family, None);
        let control = lib.get("legendary_ctl").unwrap();
        assert_eq!((control.family.as_ref(), control.steam_link.as_ref()), (None, None));
        assert!(lib.get("sideload_steam_870780").is_some_and(|g| g.family.is_some()));

        // Again: nothing new.
        assert!(!import(&paths, read, None).unwrap().changed());

        // Sharing ends for Portal; Counter-Strike is bought. The play time of
        // Portal stays (its own file), should it be shared again.
        heronite_data::status::set_game_status(&paths, "steam_400", "beaten", &Default::default()).unwrap();
        let read = LibraryRead {
            owned: vec![owned("620", "Portal 2", true, 60, None), owned("10", "Counter-Strike", true, 5, None)],
            family: vec![shared("870780", "Control", false, 0)],
        };
        let report = import(&paths, read, None).unwrap();
        assert_eq!((report.added, report.family_added, report.family_removed), (0, 0, 1));
        let lib = Library::load(&paths).unwrap();
        assert!(lib.get("sideload_steam_400").is_none());
        let sideload = json_store::read_value(&paths.sideload_library()).unwrap();
        assert!(!sideload["games"].as_array().unwrap().iter().any(|g| g["app_name"] == "steam_400"));
        assert_eq!(lib.get("sideload_steam_10").unwrap().family, None, "bought: the account's own now");
        let t: Value = json_store::read_value(&paths.timestamps()).unwrap();
        assert_eq!(t["steam_400"]["totalPlayed"], 90);

        // Shared again: back with its play time and the status it left with.
        let read = LibraryRead { owned: vec![owned("620", "Portal 2", true, 60, None)], family: vec![shared("400", "Portal", false, 90)] };
        let report = import(&paths, read, None).unwrap();
        assert_eq!(report.family_added, 1);
        // Counter-Strike and Control have entries of the family: neither is owned or shared now.
        assert_eq!(report.family_removed, 2);
        let lib = Library::load(&paths).unwrap();
        let portal = lib.get("sideload_steam_400").unwrap();
        assert_eq!((portal.playtime_minutes, portal.status_id.as_str()), (90, "beaten"));
        // Never given a status: the one its play time gives.
        let read = LibraryRead { owned: Vec::new(), family: vec![shared("870780", "Control", false, 0)] };
        import(&paths, read, None).unwrap();
        assert_eq!(Library::load(&paths).unwrap().get("sideload_steam_870780").unwrap().status_id, "not-played");
    }

    #[test]
    fn a_read_of_the_account_keeps_the_family_games() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let read = LibraryRead { owned: vec![owned("620", "Portal 2", true, 60, None)], family: vec![shared("400", "Portal", false, 90)] };
        assert_eq!(import(&paths, read, None).unwrap().family_added, 1);
        // The Web API does not show the family's games: nothing is taken for gone.
        let account = LibraryRead { owned: vec![owned("620", "Portal 2", true, 60, None)], family: Vec::new() };
        let report = import_from(&paths, account, None, SteamSource::Account).unwrap();
        assert_eq!((report.family, report.family_removed), (1, 0));
        assert_eq!(SteamOwnedFile::read(&paths).family.len(), 1);
        assert!(Library::load(&paths).unwrap().get("sideload_steam_400").is_some_and(|g| g.family.is_some()));
    }

    #[test]
    fn a_steam_entry_not_added_for_the_family_stays_when_sharing_ends() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        // Found installed by the scan, or added before Family games were read.
        steam_entries(&paths, &[("400", None)]);
        w(paths.sideload_library(), r#"{"games":[{"runner":"sideload","app_name":"steam_400","title":"Portal","install":{}}]}"#);
        let read = LibraryRead { owned: Vec::new(), family: vec![shared("400", "Portal", true, 0)] };
        assert_eq!(import(&paths, read, None).unwrap().family_added, 0);
        assert!(Library::load(&paths).unwrap().get("sideload_steam_400").is_some_and(|g| g.family.is_some()));
        let report = import(&paths, LibraryRead::default(), None).unwrap();
        assert_eq!(report.family_removed, 0);
        assert!(family_meta(&paths, "steam_400").is_some());
    }
}
