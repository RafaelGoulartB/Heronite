//! A client's installed games in the Collection: sideload entries
//! (`sideload_apps/library.json`) and their Collection metadata
//! (`local_library/library.json`) of `launchKind` `client`, whose `source`
//! is the client's id and `launcherArgs` what the client is asked.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use anyhow::Result;
use heronite_data::clients::{ClientPrefs, LastSync, LAUNCH_KIND};
use heronite_data::models::{GameInfo, InstallInfo, LocalGameMeta, LocalMetaFile};
use heronite_data::{json_store, DataPaths};
use serde_json::{json, Map, Value};

use crate::{Client, ClientGame};

/// What a sync changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    pub added: usize,
    pub updated: usize,
}

/// The Collection's app name for game `id` of `client`: `battlenet_wow`
/// (characters other than ASCII letters, digits and `-` become `_`, and
/// letters are lowercase, so the name does not depend on how the client
/// writes the id).
pub fn app_name(client: Client, id: &str) -> String {
    let id: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c.to_ascii_lowercase() } else { '_' }).collect();
    format!("{}_{id}", client.id())
}

/// The sideload entry of a game the client has.
fn entry(app: &str, game: &ClientGame) -> GameInfo {
    let mut info = GameInfo {
        runner: "sideload".into(),
        app_name: app.to_string(),
        title: game.title.clone(),
        is_installed: true,
        is_linux_native: Some(false),
        install: InstallInfo {
            executable: Some(String::new()),
            platform: Some("windows".into()),
            install_path: game.folder.as_ref().map(|f| f.to_string_lossy().into_owned()),
            is_dlc: Some(false),
            ..Default::default()
        },
        ..Default::default()
    };
    info.rest.insert("canRunOffline".into(), Value::Bool(true));
    info
}

fn meta(client: Client, app: &str, game: &ClientGame, status: &str) -> LocalGameMeta {
    LocalGameMeta {
        app_name: app.to_string(),
        runner: "sideload".into(),
        source: client.id().into(),
        launch_kind: LAUNCH_KIND.into(),
        store_game_id: Some(game.id.clone()),
        title: game.title.clone(),
        launcher_args: Some(game.launch.clone()),
        completion_status_id: Some(status.to_string()),
        ..Default::default()
    }
}

/// Sets on an existing sideload entry what the client may have changed.
fn refresh(entry: &mut Map<String, Value>, fresh: &GameInfo) {
    entry.insert("title".into(), json!(fresh.title));
    entry.insert("is_installed".into(), json!(true));
    entry.insert("is_linux_native".into(), json!(false));
    let install = entry.entry("install").or_insert_with(|| json!({}));
    if !install.is_object() {
        *install = json!({});
    }
    // A folder the client no longer names (a drive that is not mounted) is
    // kept, for the games that start from it.
    if let Some(path) = &fresh.install.install_path {
        install["install_path"] = json!(path);
    }
    install["platform"] = json!("windows");
    install["executable"] = json!("");
}

/// Brings `games` (all the client has installed now) into the Collection:
/// new ones are added, known ones refreshed (title, folder, launcherArgs),
/// the ones the client no longer has stay as not installed. Games the user
/// took out (`ClientPrefs::removed`) are left out while the client keeps
/// them; one that is gone from the client leaves that list, so installing
/// it again brings it back. Records `ClientPrefs::last` (when anything
/// changed, or the number of games did). Writes nothing when nothing
/// changed.
pub fn sync(paths: &DataPaths, client: Client, games: &[ClientGame]) -> Result<Report> {
    let removed = ClientPrefs::load(paths).removed.remove(&client).unwrap_or_default();
    let mut names: Vec<(String, &ClientGame)> = Vec::new();
    for game in games.iter().filter(|g| !g.id.trim().is_empty()) {
        let app = app_name(client, &game.id);
        if !names.iter().any(|(a, _)| *a == app) {
            names.push((app, game));
        }
    }
    let present: BTreeSet<String> = names.iter().map(|(a, _)| a.clone()).collect();
    let fresh: Vec<(GameInfo, &ClientGame)> = names.iter().filter(|(a, _)| !removed.contains(a)).map(|(a, g)| (entry(a, g), *g)).collect();
    let wanted: HashSet<&str> = fresh.iter().map(|(i, _)| i.app_name.as_str()).collect();

    let metas: LocalMetaFile = json_store::read(&paths.local_meta())?;
    let ours: HashSet<String> = metas.games.values().filter(|m| m.source == client.id()).map(|m| m.app_name.clone()).collect();
    let statuses = heronite_data::status::load_statuses(paths).unwrap_or_else(|_| heronite_data::status::default_statuses());
    let mut changed: BTreeSet<String> = BTreeSet::new();
    json_store::update_if_changed::<LocalMetaFile, _>(&paths.local_meta(), |file| {
        for (info, game) in &fresh {
            match file.games.get_mut(&info.app_name) {
                Some(m) => {
                    if m.launcher_args.as_deref() != Some(game.launch.as_str()) {
                        m.launcher_args = Some(game.launch.clone());
                        changed.insert(info.app_name.clone());
                    }
                }
                None => {
                    let status = heronite_data::status::inferred_status_id(0.0, &statuses);
                    file.games.insert(info.app_name.clone(), meta(client, &info.app_name, game, &status));
                }
            }
        }
        Ok(())
    })?;

    let mut report = Report::default();
    // A client without games has nothing to put in the file (or to take out).
    let nothing = fresh.is_empty() && ours.is_empty();
    json_store::update_value_if_changed(&paths.sideload_library(), |doc| {
        if nothing {
            return Ok(());
        }
        if !doc.is_object() {
            *doc = json!({});
        }
        let games = doc.as_object_mut().expect("object").entry("games").or_insert_with(|| json!([]));
        if !games.is_array() {
            *games = json!([]);
        }
        let list = games.as_array_mut().expect("array");
        let mut seen = HashSet::new();
        for item in list.iter_mut() {
            let Some(obj) = item.as_object_mut() else { continue };
            let Some(app) = obj.get("app_name").and_then(Value::as_str).map(str::to_owned) else { continue };
            if let Some((info, _)) = fresh.iter().find(|(i, _)| i.app_name == app) {
                seen.insert(app.clone());
                let before = obj.clone();
                refresh(obj, info);
                if *obj != before {
                    changed.insert(app);
                }
            } else if ours.contains(&app)
                && !wanted.contains(app.as_str())
                && !removed.contains(&app)
                && obj.get("is_installed") != Some(&json!(false))
            {
                obj.insert("is_installed".into(), json!(false));
                changed.insert(app);
            }
        }
        for (info, _) in fresh.iter().filter(|(i, _)| !seen.contains(&i.app_name)) {
            list.push(serde_json::to_value(info)?);
            report.added += 1;
            changed.remove(&info.app_name);
        }
        Ok(())
    })?;
    report.updated = changed.len();

    // The user's list loses what the client no longer has; the date moves
    // when the Collection did or the client has another number of games.
    let touched = report != Report::default();
    json_store::update_if_changed::<ClientPrefs, _>(&paths.clients(), |prefs| {
        if let Some(list) = prefs.removed.get_mut(&client) {
            list.retain(|a| present.contains(a));
            if list.is_empty() {
                prefs.removed.remove(&client);
            }
        }
        if touched || prefs.last.get(&client).map(|l| l.games) != Some(games.len()) {
            prefs.last.insert(client, LastSync { at: now(), games: games.len() });
        }
        Ok(())
    })?;
    if report.added > 0 {
        log::info!("Added {} game(s) from {} to the Collection", report.added, client.name());
    }
    Ok(report)
}

/// RFC 3339, to the second.
fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// How many Collection entries came from each client (one read of the
/// Collection's file).
pub fn counts(paths: &DataPaths) -> BTreeMap<Client, usize> {
    let metas: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap_or_default();
    let mut out: BTreeMap<Client, usize> = Client::ALL.iter().map(|c| (*c, 0)).collect();
    for client in metas.games.values().filter_map(|m| Client::from_id(&m.source)) {
        *out.entry(client).or_default() += 1;
    }
    out
}

/// Takes every entry of `client` out of the Collection (their play time,
/// sessions and journal stay in their own files). How many were removed.
pub fn remove(paths: &DataPaths, client: Client) -> Result<usize> {
    let mut gone = HashSet::new();
    json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |file| {
        file.games.retain(|app, m| {
            let ours = m.source == client.id();
            if ours {
                gone.insert(app.clone());
            }
            !ours
        });
        Ok(())
    })?;
    if gone.is_empty() {
        return Ok(0);
    }
    json_store::update_value(&paths.sideload_library(), |doc| {
        if let Some(list) = doc.get_mut("games").and_then(Value::as_array_mut) {
            list.retain(|g| !g.get("app_name").and_then(Value::as_str).is_some_and(|a| gone.contains(a)));
        }
        Ok(())
    })?;
    Ok(gone.len())
}

/// The user took `app_name` (an entry of `client`) out of the Collection:
/// the sync leaves it out from now on (`ClientPrefs::removed`).
pub fn mark_removed(paths: &DataPaths, client: Client, app_name: &str) -> Result<()> {
    json_store::update_if_changed::<ClientPrefs, _>(&paths.clients(), |prefs| {
        prefs.removed.entry(client).or_default().insert(app_name.to_string());
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::Library;

    fn game(id: &str, title: &str) -> ClientGame {
        ClientGame {
            id: id.into(),
            title: title.into(),
            folder: Some(format!("/prefix/drive_c/Games/{id}").into()),
            launch: format!("--exec=\"launch {id}\""),
        }
    }

    fn times(paths: &DataPaths) -> [Option<std::time::SystemTime>; 3] {
        [json_store::mtime(&paths.local_meta()), json_store::mtime(&paths.sideload_library()), json_store::mtime(&paths.clients())]
    }

    fn pause() {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    #[test]
    fn names_are_lowercase_and_safe_for_file_names() {
        assert_eq!(app_name(Client::BattleNet, "WoW"), "battlenet_wow");
        assert_eq!(app_name(Client::Ea, "Origin.OFR.50.0001"), "ea_origin_ofr_50_0001");
        assert_eq!(app_name(Client::Ubisoft, "635"), "ubisoft_635");
        assert_eq!(app_name(Client::Rockstar, "Some Other-Game/1"), "rockstar_some_other-game_1");
    }

    #[test]
    fn adds_refreshes_and_keeps_the_games_a_client_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let first = [game("WoW", "World of Warcraft"), game("D3", "Diablo III")];
        assert_eq!(sync(&paths, Client::BattleNet, &first).unwrap(), Report { added: 2, updated: 0 });

        let lib = Library::load(&paths).unwrap();
        let wow = lib.get("sideload_battlenet_wow").unwrap();
        assert!(wow.installed);
        assert_eq!(wow.title, "World of Warcraft");
        assert_eq!(wow.launch_kind.as_deref(), Some(LAUNCH_KIND));
        let m = wow.meta.as_ref().unwrap();
        assert_eq!((m.source.as_str(), m.runner.as_str()), ("battlenet", "sideload"));
        assert_eq!(m.store_game_id.as_deref(), Some("WoW"));
        assert_eq!(m.launcher_args.as_deref(), Some(r#"--exec="launch WoW""#));
        assert_eq!(m.completion_status_id.as_deref(), Some("not-played"));

        let sideload: Value = json_store::read_value(&paths.sideload_library()).unwrap();
        let first_entry = &sideload["games"][0];
        assert_eq!(first_entry["install"]["platform"], "windows");
        assert_eq!(first_entry["install"]["executable"], "");
        assert_eq!(first_entry["install"]["install_path"], "/prefix/drive_c/Games/WoW");
        assert_eq!(first_entry["install"]["is_dlc"], false);
        assert_eq!(first_entry["is_linux_native"], false);
        assert_eq!(first_entry["canRunOffline"], true);
        assert_eq!(first_entry["runner"], "sideload");

        let prefs = ClientPrefs::load(&paths);
        assert_eq!(prefs.last[&Client::BattleNet].games, 2);
        assert!(prefs.last[&Client::BattleNet].at.ends_with('Z'));

        // Again: nothing new, and nothing written (the Collection would reload).
        let before = times(&paths);
        pause();
        assert_eq!(sync(&paths, Client::BattleNet, &first).unwrap(), Report::default());
        assert_eq!(times(&paths), before);

        // Diablo III left the client, World of Warcraft was renamed and moved.
        let mut renamed = game("WoW", "World of Warcraft Classic");
        renamed.folder = Some("/prefix/drive_c/Games/Elsewhere".into());
        renamed.launch = r#"--exec="launch WoWC""#.into();
        assert_eq!(sync(&paths, Client::BattleNet, &[renamed]).unwrap(), Report { added: 0, updated: 2 });
        let lib = Library::load(&paths).unwrap();
        assert_eq!(lib.get("sideload_battlenet_wow").unwrap().title, "World of Warcraft Classic");
        let m = lib.get("sideload_battlenet_wow").unwrap().meta.clone().unwrap();
        assert_eq!(m.launcher_args.as_deref(), Some(r#"--exec="launch WoWC""#));
        assert!(!lib.get("sideload_battlenet_d3").unwrap().installed, "a game the client dropped stays, not installed");
        assert_eq!(ClientPrefs::load(&paths).last[&Client::BattleNet].games, 1);

        // It comes back.
        assert_eq!(sync(&paths, Client::BattleNet, &first).unwrap(), Report { added: 0, updated: 2 });
        assert!(Library::load(&paths).unwrap().get("sideload_battlenet_d3").unwrap().installed);
    }

    #[test]
    fn a_game_the_client_names_without_a_folder_keeps_the_one_it_had() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        sync(&paths, Client::Ubisoft, &[game("635", "Far Cry 3")]).unwrap();
        let mut unplaced = game("635", "Far Cry 3");
        unplaced.folder = None;
        assert_eq!(sync(&paths, Client::Ubisoft, &[unplaced]).unwrap(), Report::default());
        let sideload: Value = json_store::read_value(&paths.sideload_library()).unwrap();
        assert_eq!(sideload["games"][0]["install"]["install_path"], "/prefix/drive_c/Games/635");
    }

    #[test]
    fn clients_keep_to_their_own_entries() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        sync(&paths, Client::BattleNet, &[game("WoW", "World of Warcraft")]).unwrap();
        sync(&paths, Client::Ubisoft, &[game("635", "Far Cry 3"), game("635", "Duplicate")]).unwrap();
        // The other client's games are none of Ubisoft's to mark as gone.
        assert_eq!(sync(&paths, Client::Ubisoft, &[]).unwrap(), Report { added: 0, updated: 1 });
        let lib = Library::load(&paths).unwrap();
        assert!(lib.get("sideload_battlenet_wow").unwrap().installed);
        assert!(!lib.get("sideload_ubisoft_635").unwrap().installed);
        assert_eq!(lib.get("sideload_ubisoft_635").unwrap().title, "Far Cry 3");

        let by_client = counts(&paths);
        assert_eq!(
            (by_client[&Client::BattleNet], by_client[&Client::Ubisoft], by_client[&Client::Ea], by_client[&Client::Rockstar]),
            (1, 1, 0, 0)
        );
        assert_eq!(remove(&paths, Client::Ubisoft).unwrap(), 1);
        assert_eq!(remove(&paths, Client::Ubisoft).unwrap(), 0);
        let lib = Library::load(&paths).unwrap();
        assert!(lib.get("sideload_ubisoft_635").is_none() && lib.get("sideload_battlenet_wow").is_some());
        assert_eq!(counts(&paths)[&Client::BattleNet], 1);
    }

    #[test]
    fn leaves_out_the_games_the_user_removed_until_the_client_drops_them() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let games = [game("1", "One"), game("2", "Two")];
        mark_removed(&paths, Client::Ea, "ea_2").unwrap();
        mark_removed(&paths, Client::Ea, "ea_9").unwrap();
        assert_eq!(sync(&paths, Client::Ea, &games).unwrap(), Report { added: 1, updated: 0 });
        let lib = Library::load(&paths).unwrap();
        assert!(lib.get("sideload_ea_1").is_some() && lib.get("sideload_ea_2").is_none());
        // Still installed: still out. ea_9 is not installed any more: forgotten.
        let removed = &ClientPrefs::load(&paths).removed[&Client::Ea];
        assert_eq!(removed.iter().map(String::as_str).collect::<Vec<_>>(), ["ea_2"]);

        // Uninstalled, then installed again: it comes back.
        assert_eq!(sync(&paths, Client::Ea, &games[..1]).unwrap(), Report::default());
        assert!(ClientPrefs::load(&paths).removed.is_empty());
        assert_eq!(sync(&paths, Client::Ea, &games).unwrap(), Report { added: 1, updated: 0 });
        assert!(Library::load(&paths).unwrap().get("sideload_ea_2").is_some());
    }

    #[test]
    fn marking_a_game_twice_writes_once() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        mark_removed(&paths, Client::Rockstar, "rockstar_gta5").unwrap();
        let before = json_store::mtime(&paths.clients());
        pause();
        mark_removed(&paths, Client::Rockstar, "rockstar_gta5").unwrap();
        assert_eq!(json_store::mtime(&paths.clients()), before);
        assert!(ClientPrefs::load(&paths).removed[&Client::Rockstar].contains("rockstar_gta5"));
    }
}
