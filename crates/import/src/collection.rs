//! The Collection entries of the games Lutris, Bottles and Faugus start: a sideload
//! entry (`sideload_apps/library.json`) and its Collection metadata
//! (`local_library/library.json`) with the launcher's command line
//! ([`LAUNCH_KIND`]), plus the launcher's play time when it is ahead.

use std::collections::{BTreeSet, HashSet};

use anyhow::Result;
use heronite_data::launchers::{Launcher, LAUNCH_KIND, PREFIX_KEY};
use heronite_data::models::{GameInfo, InstallInfo, LocalGameMeta, LocalMetaFile, TimestampFile};
use heronite_data::{json_store, DataPaths};
use serde_json::{json, Map, Value};

use crate::{Game, Report};

/// The Collection's name for `game` of `launcher`: `lutris_12`.
pub fn app_name(launcher: Launcher, id: &str) -> String {
    let id: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    format!("{}_{id}", launcher.id())
}

fn entry(launcher: Launcher, game: &Game) -> GameInfo {
    let mut info = GameInfo {
        runner: "sideload".into(),
        app_name: app_name(launcher, &game.id),
        title: game.title.clone(),
        art_cover: game.banner.clone().or_else(|| game.cover.clone()),
        art_square: game.cover.clone(),
        is_installed: true,
        is_linux_native: Some(!game.windows),
        install: InstallInfo {
            executable: Some(String::new()),
            platform: Some(if game.windows { "windows" } else { "linux" }.into()),
            install_path: game.folder.clone(),
            is_dlc: Some(false),
            ..Default::default()
        },
        ..Default::default()
    };
    info.rest.insert("canRunOffline".into(), Value::Bool(true));
    info
}

fn meta(launcher: Launcher, game: &Game, status: &str) -> LocalGameMeta {
    let mut meta = LocalGameMeta {
        app_name: app_name(launcher, &game.id),
        runner: "sideload".into(),
        source: launcher.id().into(),
        launch_kind: LAUNCH_KIND.into(),
        store_game_id: Some(game.id.clone()),
        title: game.title.clone(),
        launcher_args: Some(game.command.clone()),
        completion_status_id: Some(status.to_string()),
        ..Default::default()
    };
    if let Some(prefix) = &game.prefix {
        meta.extra.insert(PREFIX_KEY.into(), Value::String(prefix.clone()));
    }
    meta
}

/// Sets on an existing entry what the launcher may have changed.
fn refresh(entry: &mut Map<String, Value>, fresh: &GameInfo) -> bool {
    let before = entry.clone();
    entry.insert("title".into(), json!(fresh.title));
    entry.insert("is_installed".into(), json!(true));
    entry.insert("is_linux_native".into(), json!(fresh.is_linux_native));
    if fresh.art_square.is_some() {
        entry.insert("art_square".into(), json!(fresh.art_square));
        entry.insert("art_cover".into(), json!(fresh.art_cover));
    }
    let install = entry.entry("install").or_insert_with(|| json!({}));
    if !install.is_object() {
        *install = json!({});
    }
    install["install_path"] = json!(fresh.install.install_path);
    install["platform"] = json!(fresh.install.platform);
    install["executable"] = json!("");
    *entry != before
}

/// Brings `games` of `launcher` into the Collection: new ones are added,
/// known ones refreshed, the ones the launcher no longer has stay as not
/// installed. The ones in `removed` (app names: the user took them out) are
/// left out.
pub fn sync(paths: &DataPaths, launcher: Launcher, games: &[Game], removed: &BTreeSet<String>) -> Result<Report> {
    let mut report = Report::default();
    let games: Vec<&Game> = games.iter().filter(|g| !removed.contains(&app_name(launcher, &g.id))).collect();
    let metas: LocalMetaFile = json_store::read(&paths.local_meta())?;
    let ours: HashSet<String> = metas.games.values().filter(|m| m.source == launcher.id()).map(|m| m.app_name.clone()).collect();
    let fresh: Vec<(GameInfo, &Game)> = games.iter().map(|g| (entry(launcher, g), *g)).collect();
    let wanted: HashSet<&str> = fresh.iter().map(|(i, _)| i.app_name.as_str()).collect();

    let statuses = heronite_data::status::load_statuses(paths).unwrap_or_else(|_| heronite_data::status::default_statuses());
    json_store::update_if_changed::<LocalMetaFile, _>(&paths.local_meta(), |file| {
        for (info, game) in &fresh {
            match file.games.get_mut(&info.app_name) {
                Some(m) => {
                    m.launcher_args = Some(game.command.clone());
                    match &game.prefix {
                        Some(p) => m.extra.insert(PREFIX_KEY.into(), Value::String(p.clone())),
                        None => m.extra.remove(PREFIX_KEY),
                    };
                }
                None => {
                    let status = heronite_data::status::inferred_status_id(game.minutes as f64, &statuses);
                    file.games.insert(info.app_name.clone(), meta(launcher, game, &status));
                }
            }
        }
        Ok(())
    })?;

    json_store::update_value_if_changed(&paths.sideload_library(), |doc| {
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
                seen.insert(app);
                if refresh(obj, info) {
                    report.updated += 1;
                }
            } else if ours.contains(&app) && !wanted.contains(app.as_str()) && obj.get("is_installed") != Some(&json!(false)) {
                obj.insert("is_installed".into(), json!(false));
                report.updated += 1;
            }
        }
        for (info, _) in fresh.iter().filter(|(i, _)| !seen.contains(&i.app_name)) {
            list.push(serde_json::to_value(info)?);
            report.added += 1;
        }
        Ok(())
    })?;

    let played: Vec<_> = fresh.iter().map(|(i, g)| (i.app_name.clone(), g.minutes, g.last_played.clone())).collect();
    // A new entry's play time is part of adding it.
    report.updated += raise_playtime(paths, &played)?.iter().filter(|app| ours.contains(*app)).count();
    if report.added > 0 {
        log::info!("Added {} game(s) from {} to the Collection", report.added, launcher.name());
    }
    Ok(report)
}

/// Raises `totalPlayed` / `lastPlayed` of each `(app, minutes, last played)`
/// to the launcher's when it is ahead. The entries that changed.
pub(crate) fn raise_playtime(paths: &DataPaths, entries: &[(String, u64, Option<String>)]) -> Result<Vec<String>> {
    let current: TimestampFile = json_store::read(&paths.timestamps())?;
    let behind = |app: &str, minutes: u64, last: &Option<String>| {
        let t = current.get(app);
        let total = t.map(|t| t.total_played.get()).unwrap_or(0.0);
        let ours = t.and_then(|t| t.last_played.clone()).unwrap_or_default();
        minutes as f64 > total || last.as_deref().is_some_and(|l| l > ours.as_str())
    };
    let changed: Vec<_> = entries.iter().filter(|(app, m, l)| behind(app, *m, l)).collect();
    if changed.is_empty() {
        return Ok(Vec::new());
    }
    json_store::update_value(&paths.timestamps(), |root| {
        if !root.is_object() {
            *root = json!({});
        }
        let map = root.as_object_mut().expect("object");
        for (app, minutes, last) in &changed {
            let entry = map.entry(app.clone()).or_insert_with(|| json!({"firstPlayed": "", "lastPlayed": "", "totalPlayed": 0}));
            if !entry.is_object() {
                *entry = json!({});
            }
            if *minutes as f64 > entry.get("totalPlayed").and_then(Value::as_f64).unwrap_or(0.0) {
                entry["totalPlayed"] = json!(minutes);
            }
            if let Some(last) = last {
                if last.as_str() > entry.get("lastPlayed").and_then(Value::as_str).unwrap_or("") {
                    entry["lastPlayed"] = json!(last);
                }
                if entry.get("firstPlayed").and_then(Value::as_str).unwrap_or("").is_empty() {
                    entry["firstPlayed"] = json!(last);
                }
            }
        }
        Ok(())
    })?;
    Ok(changed.into_iter().map(|(app, _, _)| app.clone()).collect())
}

/// How many Collection entries came from each launcher (one read of the
/// Collection's file).
pub fn counts(paths: &DataPaths) -> std::collections::BTreeMap<Launcher, usize> {
    let metas: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap_or_default();
    let mut out: std::collections::BTreeMap<Launcher, usize> = Launcher::ALL.iter().map(|l| (*l, 0)).collect();
    for launcher in metas.games.values().filter_map(|m| Launcher::from_source(&m.source)) {
        *out.entry(launcher).or_default() += 1;
    }
    out
}

/// Takes the entries that came from `launcher` out of the Collection, with
/// their status and notes (their play time, sessions and journal stay in
/// their own files and are there again if they come back). How many were
/// removed.
pub fn remove(paths: &DataPaths, launcher: Launcher) -> Result<usize> {
    let mut gone = HashSet::new();
    json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |file| {
        file.games.retain(|app, m| {
            let ours = m.source == launcher.id();
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

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::Library;

    fn game(id: &str, title: &str, minutes: u64) -> Game {
        Game {
            id: id.into(),
            title: title.into(),
            folder: Some(format!("/games/{id}")),
            windows: true,
            minutes,
            last_played: Some("2026-01-02T03:04:05.000Z".into()),
            command: format!("lutris lutris:rungameid/{id}"),
            prefix: Some(format!("/prefixes/{id}")),
            ..Default::default()
        }
    }

    #[test]
    fn adds_refreshes_and_keeps_the_games_a_launcher_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let first = [game("1", "Celeste", 90), game("2", "Hades", 0)];
        assert_eq!(sync(&paths, Launcher::Lutris, &first, &BTreeSet::new()).unwrap(), Report { added: 2, updated: 0 });

        let lib = Library::load(&paths).unwrap();
        let celeste = lib.get("sideload_lutris_1").unwrap();
        assert!(celeste.installed);
        assert_eq!(celeste.source_label, "Lutris");
        assert_eq!(celeste.launch_kind.as_deref(), Some(LAUNCH_KIND));
        assert_eq!(celeste.playtime_minutes, 90);
        let m = celeste.meta.as_ref().unwrap();
        assert_eq!(m.launcher_args.as_deref(), Some("lutris lutris:rungameid/1"));
        assert_eq!(m.extra.get(PREFIX_KEY), Some(&json!("/prefixes/1")));

        // Again: nothing new, and nothing written (the Collection would reload).
        let (meta_time, list_time) = (json_store::mtime(&paths.local_meta()), json_store::mtime(&paths.sideload_library()));
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(sync(&paths, Launcher::Lutris, &first, &BTreeSet::new()).unwrap(), Report::default());
        assert_eq!((json_store::mtime(&paths.local_meta()), json_store::mtime(&paths.sideload_library())), (meta_time, list_time));

        // Hades was removed from Lutris, Celeste renamed and played more.
        let mut renamed = game("1", "Celeste (2018)", 120);
        renamed.last_played = Some("2026-02-01T00:00:00.000Z".into());
        assert_eq!(sync(&paths, Launcher::Lutris, &[renamed], &BTreeSet::new()).unwrap(), Report { added: 0, updated: 3 });
        let lib = Library::load(&paths).unwrap();
        assert_eq!(lib.get("sideload_lutris_1").unwrap().title, "Celeste (2018)");
        assert_eq!(lib.get("sideload_lutris_1").unwrap().playtime_minutes, 120);
        assert!(!lib.get("sideload_lutris_2").unwrap().installed);

        assert_eq!(counts(&paths)[&Launcher::Lutris], 2);
        assert_eq!(remove(&paths, Launcher::Lutris).unwrap(), 2);
        assert!(Library::load(&paths).unwrap().games.is_empty());
    }

    /// A game the user took out of the Collection is not brought back.
    #[test]
    fn leaves_out_the_games_the_user_removed() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let removed: BTreeSet<String> = ["lutris_2".to_string()].into();
        let games = [game("1", "Celeste", 0), game("2", "Hades", 0)];
        assert_eq!(sync(&paths, Launcher::Lutris, &games, &removed).unwrap(), Report { added: 1, updated: 0 });
        let lib = Library::load(&paths).unwrap();
        assert!(lib.get("sideload_lutris_1").is_some() && lib.get("sideload_lutris_2").is_none());
    }

    #[test]
    fn names_are_safe_for_file_names() {
        assert_eq!(app_name(Launcher::Faugus, "my game/1"), "faugus_my_game_1");
    }
}
