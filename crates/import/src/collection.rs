//! The Collection entries of the games Lutris, Bottles, Faugus and Playnite start: a sideload
//! entry (`sideload_apps/library.json`) and its Collection metadata
//! (`local_library/library.json`) with the launcher's command line
//! ([`LAUNCH_KIND`]), plus the launcher's play time when it is ahead.
//! Playnite's games that can start without it are brought over as this
//! app's own instead ([`crate::transfer`]).

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use anyhow::Result;
use heronite_data::launchers::{Launcher, LAUNCH_KIND, PREFIX_KEY, RUNS_FROM_KEY};
use heronite_data::models::{GameInfo, InstallInfo, LocalGameMeta, LocalMetaFile, TimestampFile};
use heronite_data::{json_store, DataPaths};
use serde_json::{json, Map, Value};

use crate::transfer::{self, Transfer};
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
    if let Some(t) = &game.transfer {
        transfer::entry(&mut info, game, t);
    }
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
    set_paths(&mut meta, game);
    meta
}

/// The game's Wine prefix and the folder its program runs from, as the
/// launcher has them now.
fn set_paths(meta: &mut LocalGameMeta, game: &Game) {
    for (key, value) in [(PREFIX_KEY, &game.prefix), (RUNS_FROM_KEY, &game.runs_from)] {
        match value {
            Some(v) => meta.extra.insert(key.into(), Value::String(v.clone())),
            None => meta.extra.remove(key),
        };
    }
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

/// Turns an entry that asked Playnite to start the game into the game's own
/// (`fresh`); its pictures stay when it has none of Playnite's.
fn take_over(entry: &mut Map<String, Value>, fresh: &GameInfo) -> Result<bool> {
    let before = entry.clone();
    let Value::Object(fresh) = serde_json::to_value(fresh)? else { return Ok(false) };
    for (k, v) in fresh.into_iter().filter(|(_, v)| !v.is_null()) {
        entry.insert(k, v);
    }
    Ok(*entry != before)
}

/// Brings `games` of `launcher` into the Collection: new ones are added,
/// known ones refreshed, the ones the launcher no longer has stay as not
/// installed. The ones in `removed` (app names: the user took them out) are
/// left out. A Playnite game brought over before is the user's: only its
/// play time moves. `home` is the user's home folder.
pub fn sync(paths: &DataPaths, home: &Path, launcher: Launcher, games: &[Game], removed: &BTreeSet<String>) -> Result<Report> {
    let mut report = Report::default();
    let games: Vec<&Game> = games.iter().filter(|g| !removed.contains(&app_name(launcher, &g.id))).collect();
    let metas: LocalMetaFile = json_store::read(&paths.local_meta())?;
    let ours: HashSet<String> = metas.games.values().filter(|m| m.source == launcher.id()).map(|m| m.app_name.clone()).collect();
    let own: HashSet<String> = metas
        .games
        .values()
        .filter(|m| launcher == Launcher::Playnite && m.source == launcher.id() && m.launch_kind != LAUNCH_KIND)
        .map(|m| m.app_name.clone())
        .collect();
    let fresh: Vec<(GameInfo, &Game)> =
        games.iter().filter(|g| !own.contains(&app_name(launcher, &g.id))).map(|g| (entry(launcher, g), *g)).collect();
    let wanted: HashSet<&str> = fresh.iter().map(|(i, _)| i.app_name.as_str()).collect();
    let copying: Vec<(String, &Game, &Transfer)> =
        fresh.iter().filter_map(|(i, g)| Some((i.app_name.clone(), *g, g.transfer.as_deref()?))).collect();
    transfer::record_added(paths, &copying);

    let statuses = heronite_data::status::load_statuses(paths).unwrap_or_else(|_| heronite_data::status::default_statuses());
    json_store::update_if_changed::<LocalMetaFile, _>(&paths.local_meta(), |file| {
        for (info, game) in &fresh {
            match (file.games.get_mut(&info.app_name), game.transfer.as_deref()) {
                (Some(m), Some(t)) => transfer::set_meta(m, game, t, &statuses, false),
                (Some(m), None) => {
                    m.launcher_args = Some(game.command.clone());
                    set_paths(m, game);
                }
                (None, t) => {
                    let status = heronite_data::status::inferred_status_id(game.minutes as f64, &statuses);
                    let mut m = meta(launcher, game, &status);
                    if let Some(t) = t {
                        transfer::set_meta(&mut m, game, t, &statuses, true);
                    }
                    file.games.insert(info.app_name.clone(), m);
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
            if let Some((info, game)) = fresh.iter().find(|(i, _)| i.app_name == app) {
                seen.insert(app);
                let changed = if game.transfer.is_some() { take_over(obj, info)? } else { refresh(obj, info) };
                if changed {
                    report.updated += 1;
                }
            } else if ours.contains(&app)
                && !own.contains(&app)
                && !wanted.contains(app.as_str())
                && obj.get("is_installed") != Some(&json!(false))
            {
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
    transfer::settle(paths, home, &copying);

    let played: Vec<_> = games.iter().map(|g| (app_name(launcher, &g.id), g.minutes, g.last_played.clone())).collect();
    // A new entry's play time is part of adding it.
    report.updated += raise_playtime(paths, &played)?.iter().filter(|app| ours.contains(*app)).count();
    if report.added > 0 {
        log::info!("Added {} game(s) from {} to the Collection", report.added, launcher.name());
    }
    if !copying.is_empty() {
        log::info!("Brought {} game(s) over from {}: they start here", copying.len(), launcher.name());
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
        assert_eq!(
            sync(&paths, Path::new("/home/u"), Launcher::Lutris, &first, &BTreeSet::new()).unwrap(),
            Report { added: 2, updated: 0 }
        );

        let lib = Library::load(&paths).unwrap();
        let celeste = lib.get("sideload_lutris_1").unwrap();
        assert!(celeste.installed);
        assert_eq!(celeste.source_label, "Lutris");
        assert_eq!(celeste.launch_kind.as_deref(), Some(LAUNCH_KIND));
        assert_eq!(celeste.playtime_minutes, 90);
        let m = celeste.meta.as_ref().unwrap();
        assert_eq!(m.launcher_args.as_deref(), Some("lutris lutris:rungameid/1"));
        assert_eq!(m.extra.get(PREFIX_KEY), Some(&json!("/prefixes/1")));
        assert_eq!(m.extra.get(RUNS_FROM_KEY), None);

        // Again: nothing new, and nothing written (the Collection would reload).
        let (meta_time, list_time) = (json_store::mtime(&paths.local_meta()), json_store::mtime(&paths.sideload_library()));
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(sync(&paths, Path::new("/home/u"), Launcher::Lutris, &first, &BTreeSet::new()).unwrap(), Report::default());
        assert_eq!((json_store::mtime(&paths.local_meta()), json_store::mtime(&paths.sideload_library())), (meta_time, list_time));

        // Hades was removed from Lutris, Celeste renamed and played more.
        let mut renamed = game("1", "Celeste (2018)", 120);
        renamed.last_played = Some("2026-02-01T00:00:00.000Z".into());
        assert_eq!(
            sync(&paths, Path::new("/home/u"), Launcher::Lutris, &[renamed], &BTreeSet::new()).unwrap(),
            Report { added: 0, updated: 3 }
        );
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
        assert_eq!(sync(&paths, Path::new("/home/u"), Launcher::Lutris, &games, &removed).unwrap(), Report { added: 1, updated: 0 });
        let lib = Library::load(&paths).unwrap();
        assert!(lib.get("sideload_lutris_1").is_some() && lib.get("sideload_lutris_2").is_none());
    }

    /// Playnite's games are labelled as its, and remember where their
    /// emulator runs from.
    #[test]
    fn playnites_games_are_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let mut castlevania = game("71581114-f5b5-44e2-b5c9-d2ff835b199e", "Castlevania", 129);
        castlevania.prefix = None;
        castlevania.runs_from = Some("/emulators/duckstation".into());
        assert_eq!(
            sync(&paths, Path::new("/home/u"), Launcher::Playnite, &[castlevania], &BTreeSet::new()).unwrap(),
            Report { added: 1, updated: 0 }
        );
        let lib = Library::load(&paths).unwrap();
        let g = lib.get("sideload_playnite_71581114-f5b5-44e2-b5c9-d2ff835b199e").unwrap();
        assert_eq!(g.source_label, "Playnite");
        let m = g.meta.as_ref().unwrap();
        assert_eq!((m.extra.get(RUNS_FROM_KEY), m.extra.get(PREFIX_KEY)), (Some(&json!("/emulators/duckstation")), None));
    }

    fn copied(id: &str, title: &str, start: transfer::Start) -> Game {
        use heronite_data::game_actions::{Action, ActionKind};
        let mut g = game(id, title, 129);
        g.prefix = None;
        g.command = format!("explorer.exe playnite://playnite/start/{id}");
        g.transfer = Some(Box::new(Transfer {
            start,
            details: heronite_metadata::providers::PlayniteGame {
                id: id.into(),
                name: title.into(),
                description: Some("<p>A <b>gothic</b> castle.</p>".into()),
                release_date: Some("1997-03-20".into()),
                developers: vec!["Konami".into()],
                genres: vec!["Metroidvania".into()],
                critic_score: Some(93),
                links: vec!["https://example.com/sotn".into()],
                ..Default::default()
            },
            library: "/playnite/library".into(),
            favourite: true,
            hidden: false,
            stars: Some(5),
            status: Some(("3c0b5e1f-0000-0000-0000-000000000001".into(), "Beaten".into())),
            notes: Some("Inverted castle next".into()),
            added: Some("2021-06-01".into()),
            categories: vec!["Couch".into()],
            actions: transfer::with_ids(vec![Action {
                name: "Manual".into(),
                kind: ActionKind::Open,
                target: "https://example.com/manual".into(),
                ..Default::default()
            }]),
        }));
        g
    }

    fn program(path: &str, args: &str, working_dir: Option<&str>) -> transfer::Start {
        transfer::Start::Program { path: path.into(), args: args.into(), working_dir: working_dir.map(str::to_string) }
    }

    /// A Playnite game that can start without Playnite comes over as this
    /// app's own, with what Playnite knew of it; from then on it is the
    /// user's.
    #[test]
    fn playnites_games_come_over_as_this_apps_own() {
        use heronite_data::sideload::WORKING_DIR_KEY;
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let home = Path::new("/home/u");
        let id = "71581114-f5b5-44e2-b5c9-d2ff835b199e";
        let app = format!("playnite_{id}");
        let sotn = copied(id, "Castlevania", program("/games/sotn/bin/sotn.exe", "-windowed -lang en", Some("/games/sotn")));
        assert_eq!(sync(&paths, home, Launcher::Playnite, std::slice::from_ref(&sotn), &BTreeSet::new()).unwrap().added, 1);

        let lib = Library::load(&paths).unwrap();
        let g = lib.get(&format!("sideload_{app}")).unwrap();
        assert_eq!(g.source_label, "Playnite");
        assert_eq!(g.launch_kind.as_deref(), Some("executable"));
        assert!(g.installed && g.favourite && !g.hidden);
        assert_eq!(g.playtime_minutes, 129);
        assert_eq!(g.status_id, "beaten");
        let m = g.meta.as_ref().unwrap();
        assert_eq!(m.remapped_executable.as_deref(), Some("/games/sotn/bin/sotn.exe"));
        assert_eq!((m.launcher_args.as_deref(), m.playnite_id.as_str()), (None, id));
        assert_eq!(m.notes.as_deref(), Some("Inverted castle next"));
        let info = &g.game;
        assert_eq!(info.install.executable.as_deref(), Some("/games/sotn/bin/sotn.exe"));
        assert_eq!(heronite_data::sideload::working_dir_of(info), Some("/games/sotn"));
        assert_eq!(info.rest.get(WORKING_DIR_KEY), Some(&json!("/games/sotn")));

        let key = format!("sideload_{app}");
        assert_eq!(heronite_data::game_config::game_overrides(&paths, &app).get("launcherArgs"), Some(&json!("-windowed -lang en")));
        assert_eq!(heronite_data::ratings::load(&paths).get(&key), Some(&5));
        assert_eq!(heronite_data::added::load(&paths).get(&key).map(String::as_str), Some("2021-06-01"));
        assert_eq!(heronite_data::game_actions::load(&paths)[&key][0].target, "https://example.com/manual");
        let store: Value = json_store::read_value(&paths.store_config()).unwrap();
        let couch = &store["games"]["customCategories"]["Couch"];
        assert_eq!(couch, &json!([heronite_data::categories::game_ref("sideload", &app)]));
        let md = heronite_metadata::store::get_game_metadata(&paths, "sideload", &app).unwrap().unwrap();
        assert_eq!(md.description.as_deref(), Some("A gothic castle."));
        assert_eq!((md.release_date.as_deref(), md.developers.as_slice()), (Some("1997-03-20"), &["Konami".to_string()][..]));
        assert_eq!(md.critic_score.map(|s| s.get()), Some(93.0));
        assert_eq!(md.websites, ["https://example.com/sotn"]);

        // The user changes it here; Playnite renames it and drops it later:
        // it stays as the user has it, only its play time follows Playnite.
        heronite_data::status::set_game_status(&paths, &app, "completed", &Default::default()).unwrap();
        let mut renamed = copied(id, "Castlevania: SOTN", program("/elsewhere/sotn.exe", "", None));
        renamed.minutes = 300;
        assert_eq!(sync(&paths, home, Launcher::Playnite, &[renamed], &BTreeSet::new()).unwrap(), Report { added: 0, updated: 1 });
        assert_eq!(sync(&paths, home, Launcher::Playnite, &[], &BTreeSet::new()).unwrap(), Report::default());
        let lib = Library::load(&paths).unwrap();
        let g = lib.get(&format!("sideload_{app}")).unwrap();
        assert_eq!((g.title.as_str(), g.installed, g.playtime_minutes), ("Castlevania", true, 300));
        assert_eq!(g.status_id, "completed");
        assert_eq!(g.meta.as_ref().unwrap().remapped_executable.as_deref(), Some("/games/sotn/bin/sotn.exe"));
    }

    /// An emulated game comes with its emulator's command line and ROMs; a
    /// game Playnite started before is brought over, keeping the status the
    /// user gave it.
    #[test]
    fn a_game_playnite_started_is_brought_over() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let home = Path::new("/home/u");
        let id = "00000002-0000-0000-0000-000000000002";
        let app = format!("playnite_{id}");
        let mut linked = copied(id, "Okami", program("/x.exe", "", None));
        linked.transfer = None;
        sync(&paths, home, Launcher::Playnite, &[linked], &BTreeSet::new()).unwrap();
        heronite_data::status::set_game_status(&paths, &app, "playing", &Default::default()).unwrap();
        assert_eq!(Library::load(&paths).unwrap().get(&format!("sideload_{app}")).unwrap().launch_kind.as_deref(), Some(LAUNCH_KIND));

        let rom = transfer::Rom { name: Some("Disc".into()), path: "/roms/okami.iso".into() };
        let start = transfer::Start::Emulator {
            name: "PCSX2".into(),
            program: "/emu/pcsx2/pcsx2-qt.exe".into(),
            args: r#"-batch -fullscreen "{ImagePath}""#.into(),
            roms: vec![rom],
        };
        let report = sync(&paths, home, Launcher::Playnite, &[copied(id, "Okami", start)], &BTreeSet::new()).unwrap();
        assert_eq!(report, Report { added: 0, updated: 1 });
        let lib = Library::load(&paths).unwrap();
        let g = lib.get(&format!("sideload_{app}")).unwrap();
        assert_eq!((g.launch_kind.as_deref(), g.status_id.as_str()), (Some("emulator"), "playing"));
        let m = g.meta.as_ref().unwrap();
        assert_eq!(m.remapped_executable.as_deref(), Some("/emu/pcsx2/pcsx2-qt.exe"));
        assert_eq!(m.launcher_args.as_deref(), Some(r#"-batch -fullscreen "{ImagePath}""#));
        assert_eq!(m.roms.as_ref().unwrap()[0].path, "/roms/okami.iso");
        assert_eq!(m.emulator_name.as_deref(), Some("PCSX2"));
        assert_eq!(m.extra.get(RUNS_FROM_KEY), None);
        assert_eq!(g.game.install.executable.as_deref(), Some("/emu/pcsx2/pcsx2-qt.exe"));
        // The day it was added to Playnite.
        assert_eq!(heronite_data::added::load(&paths).get(&format!("sideload_{app}")).map(String::as_str), Some("2021-06-01"));
    }

    #[test]
    fn names_are_safe_for_file_names() {
        assert_eq!(app_name(Launcher::Faugus, "my game/1"), "faugus_my_game_1");
    }
}
