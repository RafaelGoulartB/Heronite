//! The games a client's account owns (the EA account's library,
//! [`crate::ea_account`]) in the Collection: the ones it does not have yet
//! come in as not installed, entries of the client like its installed
//! games, under the same app name ([`collection::app_name`]), so installing
//! one later makes that same entry installed. Nothing is ever taken out or
//! changed here: a game the account no longer lists, or a read that fails,
//! leaves the Collection as it is, and the installed games are always the
//! client's own sync's ([`collection::sync`]).

use std::collections::HashSet;

use anyhow::Result;
use heronite_data::clients::{ClientPrefs, LastSync, LAUNCH_KIND};
use heronite_data::models::{GameInfo, InstallInfo, LocalGameMeta, LocalMetaFile};
use heronite_data::{json_store, DataPaths};
use serde_json::{json, Value};

use crate::collection::{self, Report};
use crate::Client;

/// A game the account owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedGame {
    /// The client's id for the game, the one its installed copy has (EA's
    /// content id).
    pub id: String,
    pub title: String,
}

/// A title as it is compared: letters and digits only, lowercase
/// (`Mass Effect™ Legendary Edition` is `masseffectlegendaryedition`).
fn plain(title: &str) -> String {
    title.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn entry(app: &str, game: &OwnedGame) -> GameInfo {
    let mut info = GameInfo {
        runner: "sideload".into(),
        app_name: app.to_string(),
        title: game.title.clone(),
        is_installed: false,
        is_linux_native: Some(false),
        install: InstallInfo {
            executable: Some(String::new()),
            platform: Some("windows".into()),
            is_dlc: Some(false),
            ..Default::default()
        },
        ..Default::default()
    };
    info.rest.insert("canRunOffline".into(), Value::Bool(true));
    info
}

/// Adds the games of `owned` the Collection has no entry of `client` for
/// (by app name, by an id an installed copy lists, or by title), except the
/// ones the user took out ([`ClientPrefs::removed_owned`]), and records the
/// read ([`ClientPrefs::owned`]). Writes the Collection only when it adds.
pub fn sync(paths: &DataPaths, client: Client, owned: &[OwnedGame]) -> Result<Report> {
    let prefs = ClientPrefs::load(paths);
    let removed: HashSet<&String> =
        [prefs.removed.get(&client), prefs.removed_owned.get(&client)].into_iter().flatten().flatten().collect();
    let metas: LocalMetaFile = json_store::read(&paths.local_meta())?;
    let ours: Vec<&LocalGameMeta> = metas.games.values().filter(|m| m.source == client.id()).collect();
    let known_ids: HashSet<&str> = ours
        .iter()
        .flat_map(|m| m.store_game_id.as_deref().into_iter().chain(m.launcher_args.as_deref().into_iter().flat_map(crate::ea::linked_ids)))
        .collect();
    let known_titles: HashSet<String> = ours.iter().map(|m| plain(&m.title)).filter(|t| !t.is_empty()).collect();

    let mut new: Vec<(String, &OwnedGame)> = Vec::new();
    for game in owned {
        let app = collection::app_name(client, &game.id);
        let title = plain(&game.title);
        if game.id.trim().is_empty() || title.is_empty() || removed.contains(&app) || metas.games.contains_key(&app) {
            continue;
        }
        if known_ids.contains(game.id.as_str())
            || known_titles.contains(&title)
            || new.iter().any(|(a, g)| *a == app || plain(&g.title) == title)
        {
            continue;
        }
        new.push((app, game));
    }

    let mut report = Report::default();
    if !new.is_empty() {
        // The entry first: metadata alone would hide the game from the
        // next read without showing it.
        let mut added: HashSet<String> = HashSet::new();
        json_store::update_value_if_changed(&paths.sideload_library(), |doc| {
            if !doc.is_object() {
                *doc = json!({});
            }
            let games = doc.as_object_mut().expect("object").entry("games").or_insert_with(|| json!([]));
            if !games.is_array() {
                *games = json!([]);
            }
            let list = games.as_array_mut().expect("array");
            for (app, game) in &new {
                // The client's own sync may have added it meanwhile.
                if list.iter().any(|g| g.get("app_name").and_then(Value::as_str) == Some(app.as_str())) {
                    continue;
                }
                list.push(serde_json::to_value(entry(app, game))?);
                added.insert(app.clone());
            }
            Ok(())
        })?;
        let statuses = heronite_data::status::load_statuses(paths).unwrap_or_else(|_| heronite_data::status::default_statuses());
        json_store::update_if_changed::<LocalMetaFile, _>(&paths.local_meta(), |file| {
            for (app, game) in new.iter().filter(|(a, _)| added.contains(a)) {
                if file.games.contains_key(app) {
                    continue;
                }
                let meta = LocalGameMeta {
                    app_name: app.clone(),
                    runner: "sideload".into(),
                    source: client.id().into(),
                    launch_kind: LAUNCH_KIND.into(),
                    store_game_id: Some(game.id.clone()),
                    title: game.title.clone(),
                    launcher_args: (client == Client::Ea).then(|| crate::ea::launch_link(std::slice::from_ref(&game.id))),
                    completion_status_id: Some(heronite_data::status::inferred_status_id(0.0, &statuses)),
                    ..Default::default()
                };
                file.games.insert(app.clone(), meta);
            }
            Ok(())
        })?;
        report.added = added.len();
    }

    json_store::update_if_changed::<ClientPrefs, _>(&paths.clients(), |prefs| {
        prefs
            .owned
            .insert(client, LastSync { at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true), games: owned.len() });
        Ok(())
    })?;
    if report.added > 0 {
        log::info!("Added {} game(s) {} owns to the Collection, not installed", report.added, client.name());
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ClientGame;
    use heronite_data::Library;

    fn owned(id: &str, title: &str) -> OwnedGame {
        OwnedGame { id: id.into(), title: title.into() }
    }

    fn installed(ids: &[&str], title: &str) -> ClientGame {
        let ids: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
        ClientGame {
            id: ids[0].clone(),
            title: title.into(),
            folder: Some(format!("/pfx/drive_c/Program Files/EA Games/{title}").into()),
            launch: crate::ea::launch_link(&ids),
        }
    }

    fn lib(paths: &DataPaths) -> Library {
        Library::load(paths).unwrap()
    }

    #[test]
    fn owned_games_come_in_as_not_installed_and_install_into_the_same_entry() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        collection::sync(&paths, Client::Ea, &[installed(&["71067", "71068"], "Mass Effect\u{2122} Andromeda")]).unwrap();

        let list = [
            owned("71067", "Mass Effect Andromeda"),
            // An id the installed copy lists after its first.
            owned("71068", "Mass Effect Andromeda Deluxe"),
            // The same game as installed, under other ids.
            owned("9999", "MASS EFFECT: Andromeda"),
            owned("1002", "Unravel Two"),
            owned("1002", "Unravel Two (again)"),
            owned("1003", "\u{2122}"),
        ];
        assert_eq!(sync(&paths, Client::Ea, &list).unwrap(), Report { added: 1, updated: 0 });
        let l = lib(&paths);
        let unravel = l.get("sideload_ea_1002").unwrap();
        assert!(!unravel.installed);
        assert_eq!(unravel.title, "Unravel Two");
        let m = unravel.meta.as_ref().unwrap();
        assert_eq!((m.source.as_str(), m.launch_kind.as_str()), ("ea", LAUNCH_KIND));
        assert_eq!(m.store_game_id.as_deref(), Some("1002"));
        assert_eq!(m.launcher_args.as_deref(), Some("\"origin2://game/launch?offerIds=1002&autoDownload=1\""));
        assert_eq!(m.completion_status_id.as_deref(), Some("not-played"));
        assert!(l.get("sideload_ea_9999").is_none() && l.get("sideload_ea_71068").is_none());
        assert!(l.get("sideload_ea_71067").unwrap().installed);
        assert_eq!(ClientPrefs::load(&paths).owned[&Client::Ea].games, 6);

        // Installed later in the EA app: the same entry, installed.
        let both = [installed(&["71067", "71068"], "Mass Effect\u{2122} Andromeda"), installed(&["1002"], "Unravel Two")];
        assert_eq!(collection::sync(&paths, Client::Ea, &both).unwrap(), Report { added: 0, updated: 1 });
        let l = lib(&paths);
        assert!(l.get("sideload_ea_1002").unwrap().installed);
        assert_eq!(l.games.iter().filter(|g| g.app_name.starts_with("ea_")).count(), 2);

        // Uninstalled again: it stays, not installed, and the account adds nothing.
        collection::sync(&paths, Client::Ea, &both[..1]).unwrap();
        assert!(!lib(&paths).get("sideload_ea_1002").unwrap().installed);
        assert_eq!(sync(&paths, Client::Ea, &list).unwrap(), Report::default());
    }

    #[test]
    fn installing_a_game_the_account_brought_under_another_id_fills_its_entry() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        sync(&paths, Client::Ea, &[owned("1018221", "Mass Effect Andromeda Deluxe Edition")]).unwrap();
        // The installed copy lists that id after its first.
        let copy = installed(&["71067", "1018221"], "Mass Effect\u{2122} Andromeda");
        assert_eq!(collection::sync(&paths, Client::Ea, std::slice::from_ref(&copy)).unwrap(), Report { added: 0, updated: 1 });
        let l = lib(&paths);
        assert!(l.get("sideload_ea_1018221").unwrap().installed);
        assert!(l.get("sideload_ea_71067").is_none(), "no second entry");
        // Again, and once uninstalled: still the one entry.
        assert_eq!(collection::sync(&paths, Client::Ea, std::slice::from_ref(&copy)).unwrap(), Report::default());
        collection::sync(&paths, Client::Ea, &[]).unwrap();
        let l = lib(&paths);
        assert!(!l.get("sideload_ea_1018221").unwrap().installed && l.get("sideload_ea_71067").is_none());

        // Installed, then taken out of the Collection: it stays out under
        // either name while installed.
        collection::sync(&paths, Client::Ea, std::slice::from_ref(&copy)).unwrap();
        json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |f| {
            f.games.shift_remove("ea_1018221");
            Ok(())
        })
        .unwrap();
        json_store::update_value(&paths.sideload_library(), |doc| {
            doc["games"].as_array_mut().unwrap().retain(|g| g["app_name"] != "ea_1018221");
            Ok(())
        })
        .unwrap();
        collection::mark_removed(&paths, Client::Ea, "ea_1018221", std::slice::from_ref(&copy)).unwrap();
        for _ in 0..2 {
            assert_eq!(collection::sync(&paths, Client::Ea, std::slice::from_ref(&copy)).unwrap(), Report::default());
        }
        let l = lib(&paths);
        assert!(l.get("sideload_ea_1018221").is_none() && l.get("sideload_ea_71067").is_none());
        assert!(ClientPrefs::load(&paths).removed[&Client::Ea].contains("ea_1018221"));
    }

    #[test]
    fn without_the_account_installed_games_are_named_as_before() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        // Standard installed, then uninstalled; then a Deluxe copy that lists its id.
        collection::sync(&paths, Client::Ea, &[installed(&["1001"], "Game")]).unwrap();
        collection::sync(&paths, Client::Ea, &[]).unwrap();
        let deluxe = installed(&["2002", "1001"], "Game Deluxe");
        assert_eq!(collection::sync(&paths, Client::Ea, &[deluxe]).unwrap(), Report { added: 1, updated: 0 });
        let l = lib(&paths);
        assert!(l.get("sideload_ea_2002").unwrap().installed);
        assert!(!l.get("sideload_ea_1001").unwrap().installed);
    }

    #[test]
    fn a_read_without_a_game_takes_nothing_out() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        sync(&paths, Client::Ea, &[owned("1002", "Unravel Two")]).unwrap();
        let before = json_store::mtime(&paths.sideload_library());
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(sync(&paths, Client::Ea, &[]).unwrap(), Report::default());
        assert!(lib(&paths).get("sideload_ea_1002").is_some());
        assert_eq!(json_store::mtime(&paths.sideload_library()), before, "the Collection is not written");
        assert_eq!(ClientPrefs::load(&paths).owned[&Client::Ea].games, 0);
        // The client's own sync leaves it as it is too.
        collection::sync(&paths, Client::Ea, &[]).unwrap();
        assert!(!lib(&paths).get("sideload_ea_1002").unwrap().installed);
    }

    #[test]
    fn a_game_the_user_took_out_never_comes_back_from_the_account() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let list = [owned("1002", "Unravel Two"), owned("1003", "It Takes Two")];
        sync(&paths, Client::Ea, &list).unwrap();
        // Removed from the Collection (the app takes the entry out, then marks it).
        json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |f| {
            f.games.shift_remove("ea_1002");
            Ok(())
        })
        .unwrap();
        json_store::update_value(&paths.sideload_library(), |doc| {
            doc["games"].as_array_mut().unwrap().retain(|g| g["app_name"] != "ea_1002");
            Ok(())
        })
        .unwrap();
        collection::mark_removed(&paths, Client::Ea, "ea_1002", &[]).unwrap();
        // The client's sync forgets it (not installed), the account's does not.
        collection::sync(&paths, Client::Ea, &[]).unwrap();
        assert!(ClientPrefs::load(&paths).removed.is_empty());
        assert_eq!(sync(&paths, Client::Ea, &list).unwrap(), Report::default());
        assert!(lib(&paths).get("sideload_ea_1002").is_none());
        // Installed in the EA app, it comes back (the client's sync).
        collection::sync(&paths, Client::Ea, &[installed(&["1002"], "Unravel Two")]).unwrap();
        assert!(lib(&paths).get("sideload_ea_1002").unwrap().installed);
    }

    #[test]
    fn an_installed_game_the_user_took_out_does_not_come_back_under_another_id() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let copy = installed(&["71067", "1018221"], "Mass Effect\u{2122} Andromeda");
        collection::sync(&paths, Client::Ea, std::slice::from_ref(&copy)).unwrap();
        json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |f| {
            f.games.shift_remove("ea_71067");
            Ok(())
        })
        .unwrap();
        json_store::update_value(&paths.sideload_library(), |doc| {
            doc["games"].as_array_mut().unwrap().retain(|g| g["app_name"] != "ea_71067");
            Ok(())
        })
        .unwrap();
        collection::mark_removed(&paths, Client::Ea, "ea_71067", std::slice::from_ref(&copy)).unwrap();
        // The account is read before any sync of the EA app.
        assert_eq!(sync(&paths, Client::Ea, &[owned("1018221", "Mass Effect Andromeda Deluxe Edition")]).unwrap(), Report::default());
        for _ in 0..2 {
            collection::sync(&paths, Client::Ea, std::slice::from_ref(&copy)).unwrap();
            assert_eq!(sync(&paths, Client::Ea, &[owned("1018221", "Mass Effect Andromeda Deluxe Edition")]).unwrap(), Report::default());
        }
        let l = lib(&paths);
        assert!(l.get("sideload_ea_71067").is_none() && l.get("sideload_ea_1018221").is_none());
    }

    #[test]
    fn launch_links_name_their_ids() {
        assert_eq!(crate::ea::linked_ids("\"origin2://game/launch?offerIds=1,2&autoDownload=1\"").collect::<Vec<_>>(), ["1", "2"]);
        assert_eq!(crate::ea::linked_ids("--exec=\"launch WoW\"").count(), 0);
        assert_eq!(plain("Mass Effect\u{2122}: Legendary Edition"), "masseffectlegendaryedition");
    }
}
