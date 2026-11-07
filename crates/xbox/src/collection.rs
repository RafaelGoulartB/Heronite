//! The Xbox app's installed games in the Collection: sideload entries
//! (`sideload_apps/library.json`) and their Collection metadata
//! (`local_library/library.json`) of `launchKind` `xbox`, whose `source` is
//! `xbox` and `launcherArgs` the AUMID Windows starts.

use std::collections::{BTreeSet, HashSet};

use anyhow::Result;
use heronite_data::models::{GameInfo, InstallInfo, LocalGameMeta, LocalMetaFile};
use heronite_data::xbox::{LastSync, XboxPrefs, LAUNCH_KIND, SOURCE, TITLE_ID_KEY};
use heronite_data::{json_store, DataPaths};
use serde_json::{json, Map, Value};

use crate::XboxGame;

/// What a sync changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    pub added: usize,
    pub updated: usize,
}

/// The Collection's app name of a package family: `xbox_<family>`, lowercase,
/// characters other than ASCII letters, digits and `-` as `_`.
pub fn app_name(family: &str) -> String {
    let id: String = family.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c.to_ascii_lowercase() } else { '_' }).collect();
    format!("{SOURCE}_{id}")
}

fn entry(app: &str, game: &XboxGame) -> GameInfo {
    let mut info = GameInfo {
        runner: "sideload".into(),
        app_name: app.to_string(),
        title: game.title.clone(),
        is_installed: true,
        is_linux_native: Some(false),
        install: InstallInfo {
            executable: Some(String::new()),
            platform: Some("windows".into()),
            install_path: Some(game.folder.to_string_lossy().into_owned()),
            is_dlc: Some(false),
            ..Default::default()
        },
        ..Default::default()
    };
    info.rest.insert("canRunOffline".into(), Value::Bool(true));
    info
}

fn meta(app: &str, game: &XboxGame, status: &str) -> LocalGameMeta {
    let mut meta = LocalGameMeta {
        app_name: app.to_string(),
        runner: "sideload".into(),
        source: SOURCE.into(),
        launch_kind: LAUNCH_KIND.into(),
        store_game_id: Some(game.family.clone()),
        title: game.title.clone(),
        launcher_args: Some(game.aumid()),
        completion_status_id: Some(status.to_string()),
        ..Default::default()
    };
    set_title_id(&mut meta, game);
    meta
}

/// Writes the game's title id into its metadata (or takes out one its files
/// no longer name). Whether that changed anything.
fn set_title_id(meta: &mut LocalGameMeta, game: &XboxGame) -> bool {
    let current = meta.extra.get(TITLE_ID_KEY).and_then(Value::as_str);
    if current == game.title_id.as_deref() {
        return false;
    }
    match &game.title_id {
        Some(id) => meta.extra.insert(TITLE_ID_KEY.into(), json!(id)),
        None => meta.extra.remove(TITLE_ID_KEY),
    };
    true
}

/// Sets on an existing sideload entry what an update may have changed.
fn refresh(entry: &mut Map<String, Value>, fresh: &GameInfo) {
    entry.insert("title".into(), json!(fresh.title));
    entry.insert("is_installed".into(), json!(true));
    entry.insert("is_linux_native".into(), json!(false));
    let install = entry.entry("install").or_insert_with(|| json!({}));
    if !install.is_object() {
        *install = json!({});
    }
    install["install_path"] = json!(fresh.install.install_path);
    install["platform"] = json!("windows");
    install["executable"] = json!("");
}

/// Brings `games` (all the Xbox app's games installed now) into the
/// Collection: new ones are added, known ones refreshed (title, folder,
/// AUMID), the ones no longer installed stay as not installed. Games the
/// user took out (`XboxPrefs::removed`) are left out while they are
/// installed; one that is uninstalled leaves that list, so installing it
/// again brings it back. Writes nothing when nothing changed.
pub fn sync(paths: &DataPaths, games: &[XboxGame]) -> Result<Report> {
    let removed = XboxPrefs::load(paths).removed;
    let mut names: Vec<(String, &XboxGame)> = Vec::new();
    for game in games.iter().filter(|g| !g.family.trim().is_empty()) {
        let app = app_name(&game.family);
        if !names.iter().any(|(a, _)| *a == app) {
            names.push((app, game));
        }
    }
    let present: BTreeSet<String> = names.iter().map(|(a, _)| a.clone()).collect();
    let fresh: Vec<(GameInfo, &XboxGame)> = names.iter().filter(|(a, _)| !removed.contains(a)).map(|(a, g)| (entry(a, g), *g)).collect();

    let metas: LocalMetaFile = json_store::read(&paths.local_meta())?;
    let ours: HashSet<String> =
        metas.games.values().filter(|m| m.source == SOURCE && m.launch_kind == LAUNCH_KIND).map(|m| m.app_name.clone()).collect();
    let statuses = heronite_data::status::load_statuses(paths).unwrap_or_else(|_| heronite_data::status::default_statuses());
    let mut changed: BTreeSet<String> = BTreeSet::new();
    json_store::update_if_changed::<LocalMetaFile, _>(&paths.local_meta(), |file| {
        for (info, game) in &fresh {
            match file.games.get_mut(&info.app_name) {
                Some(m) => {
                    let aumid = game.aumid();
                    if m.launcher_args.as_deref() != Some(aumid.as_str()) {
                        m.launcher_args = Some(aumid);
                        changed.insert(info.app_name.clone());
                    }
                    if set_title_id(m, game) {
                        changed.insert(info.app_name.clone());
                    }
                }
                None => {
                    let status = heronite_data::status::inferred_status_id(0.0, &statuses);
                    file.games.insert(info.app_name.clone(), meta(&info.app_name, game, &status));
                }
            }
        }
        Ok(())
    })?;

    let mut report = Report::default();
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
            } else if ours.contains(&app) && !present.contains(&app) && obj.get("is_installed") != Some(&json!(false)) {
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

    let touched = report != Report::default();
    json_store::update_if_changed::<XboxPrefs, _>(&paths.xbox(), |prefs| {
        prefs.removed.retain(|a| present.contains(a));
        if touched || prefs.last.as_ref().map(|l| l.games) != Some(games.len()) {
            prefs.last = Some(LastSync { at: now(), games: games.len() });
        }
        Ok(())
    })?;
    if report.added > 0 {
        log::info!("Added {} game(s) from the Xbox app to the Collection", report.added);
    }
    Ok(report)
}

/// RFC 3339, to the second.
fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// How many Collection entries came from the Xbox app.
pub fn count(paths: &DataPaths) -> usize {
    let metas: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap_or_default();
    metas.games.values().filter(|m| m.source == SOURCE && m.launch_kind == LAUNCH_KIND).count()
}

/// The user took `app_name` (an Xbox entry) out of the Collection: the sync
/// leaves it out from now on (`XboxPrefs::removed`).
pub fn mark_removed(paths: &DataPaths, app_name: &str) -> Result<()> {
    json_store::update_if_changed::<XboxPrefs, _>(&paths.xbox(), |prefs| {
        prefs.removed.insert(app_name.to_string());
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::Library;

    fn game(family: &str, title: &str) -> XboxGame {
        XboxGame {
            family: family.into(),
            app_id: "Game".into(),
            title: title.into(),
            folder: std::env::temp_dir().join("XboxGames").join(title).join("Content"),
            store_id: None,
            title_id: None,
            package: format!("{family}_1.0.0.0_x64__"),
        }
    }

    fn times(paths: &DataPaths) -> [Option<std::time::SystemTime>; 3] {
        [json_store::mtime(&paths.local_meta()), json_store::mtime(&paths.sideload_library()), json_store::mtime(&paths.xbox())]
    }

    #[test]
    fn names_are_lowercase_and_safe_for_file_names() {
        assert_eq!(app_name("Microsoft.624F8B84B80_8wekyb3d8bbwe"), "xbox_microsoft_624f8b84b80_8wekyb3d8bbwe");
    }

    #[test]
    fn adds_refreshes_and_keeps_the_uninstalled_games() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let first = [game("Studio.Forza_abc", "Forza Horizon 5"), game("Studio.Sea_abc", "Sea of Thieves")];
        assert_eq!(sync(&paths, &first).unwrap(), Report { added: 2, updated: 0 });

        let lib = Library::load(&paths).unwrap();
        let forza = lib.get("sideload_xbox_studio_forza_abc").unwrap();
        assert!(forza.installed);
        assert_eq!(forza.launch_kind.as_deref(), Some(LAUNCH_KIND));
        let m = forza.meta.as_ref().unwrap();
        assert_eq!((m.source.as_str(), m.runner.as_str()), ("xbox", "sideload"));
        assert_eq!(m.store_game_id.as_deref(), Some("Studio.Forza_abc"));
        assert_eq!(m.launcher_args.as_deref(), Some("Studio.Forza_abc!Game"));
        assert_eq!(m.completion_status_id.as_deref(), Some("not-played"));
        assert_eq!(XboxPrefs::load(&paths).last.unwrap().games, 2);
        assert_eq!(count(&paths), 2);

        // Again: nothing written.
        let before = times(&paths);
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(sync(&paths, &first).unwrap(), Report::default());
        assert_eq!(times(&paths), before);

        // Sea of Thieves was uninstalled; Forza's application changed.
        let mut forza = game("Studio.Forza_abc", "Forza Horizon 5");
        forza.app_id = "Launcher".into();
        assert_eq!(sync(&paths, &[forza]).unwrap(), Report { added: 0, updated: 2 });
        let lib = Library::load(&paths).unwrap();
        assert!(!lib.get("sideload_xbox_studio_sea_abc").unwrap().installed, "stays, not installed");
        let m = lib.get("sideload_xbox_studio_forza_abc").unwrap().meta.clone().unwrap();
        assert_eq!(m.launcher_args.as_deref(), Some("Studio.Forza_abc!Launcher"));
        assert!(!m.extra.contains_key(TITLE_ID_KEY));

        // Its files now name its title id.
        let mut forza = game("Studio.Forza_abc", "Forza Horizon 5");
        forza.app_id = "Launcher".into();
        forza.title_id = Some("79FC11E1".into());
        assert_eq!(sync(&paths, std::slice::from_ref(&forza)).unwrap(), Report { added: 0, updated: 1 });
        let m = Library::load(&paths).unwrap().get("sideload_xbox_studio_forza_abc").unwrap().meta.clone().unwrap();
        assert_eq!(m.extra.get(TITLE_ID_KEY), Some(&json!("79FC11E1")));
        assert_eq!(sync(&paths, &[forza]).unwrap(), Report::default());
    }

    #[test]
    fn leaves_out_the_games_the_user_removed_until_they_are_uninstalled() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let games = [game("A_x", "One"), game("B_x", "Two")];
        mark_removed(&paths, "xbox_b_x").unwrap();
        mark_removed(&paths, "xbox_gone_x").unwrap();
        assert_eq!(sync(&paths, &games).unwrap(), Report { added: 1, updated: 0 });
        assert!(Library::load(&paths).unwrap().get("sideload_xbox_b_x").is_none());
        assert_eq!(XboxPrefs::load(&paths).removed.into_iter().collect::<Vec<_>>(), ["xbox_b_x"]);
        assert_eq!(sync(&paths, &games[..1]).unwrap(), Report::default());
        assert!(XboxPrefs::load(&paths).removed.is_empty());
        assert_eq!(sync(&paths, &games).unwrap(), Report { added: 1, updated: 0 });
    }

    #[test]
    fn other_entries_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        json_store::write(
            &paths.sideload_library(),
            &json!({"games": [{"app_name": "mine", "runner": "sideload", "title": "Mine", "is_installed": true}]}),
        )
        .unwrap();
        sync(&paths, &[]).unwrap();
        sync(&paths, &[game("A_x", "One")]).unwrap();
        sync(&paths, &[]).unwrap();
        let lib = Library::load(&paths).unwrap();
        assert!(lib.get("sideload_mine").unwrap().installed);
        assert!(!lib.get("sideload_xbox_a_x").unwrap().installed);
    }
}
