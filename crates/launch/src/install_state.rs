//! Which local games count as installed (port of `install-state.ts`):
//! the pure rules, and [`refresh_local_install_states`] that applies them to
//! `sideload_apps/library.json` (and to the cached Steam library).

use std::collections::{HashMap, HashSet};

use heronite_data::models::{GameInfo, InstallInfo, LocalGameMeta, LocalMetaFile};
use heronite_data::{json_store, DataPaths};
use serde_json::Value;

use crate::plan::{emulator_executable, PlanContext};
use crate::probe::Probe;
use crate::steam::{find_steam_binary, installed_app_ids, list_installed_games, steam_cdn_covers, SteamInstalledGame, SteamLocator};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallState {
    pub is_installed: bool,
    pub executable: String,
}

fn first_existing(probe: &dyn Probe, paths: &[Option<&str>]) -> String {
    paths.iter().flatten().find(|p| !p.is_empty() && probe.exists(p)).map(|p| p.to_string()).unwrap_or_default()
}

/// `emulatorRomInstalled`.
pub fn emulator_rom_installed(meta: &LocalGameMeta, probe: &dyn Probe) -> bool {
    match meta.roms.as_ref().filter(|r| !r.is_empty()) {
        None => meta.remapped_executable.as_deref().map(|p| probe.exists(p)).unwrap_or(false),
        Some(roms) => roms.iter().any(|r| probe.exists(&r.path)),
    }
}

/// `nextInstallState`: what `is_installed` / `install.executable` should be.
pub fn next_install_state(
    ctx: &PlanContext,
    probe: &dyn Probe,
    game: &GameInfo,
    meta: &LocalGameMeta,
    installed_steam: &HashSet<String>,
    steam_bin: Option<&str>,
) -> InstallState {
    let local_exe = || first_existing(probe, &[meta.remapped_executable.as_deref(), game.install.executable.as_deref()]);

    if meta.launch_kind == "steam-uri" {
        let installed = meta.steam_app_id.as_deref().map(|id| installed_steam.contains(id)).unwrap_or(false);
        return InstallState {
            is_installed: installed,
            executable: if installed { steam_bin.unwrap_or("steam").to_string() } else { String::new() },
        };
    }

    // Installed in the launcher or store client it came from (their sync says
    // so; there is no executable), while its folder is there.
    if matches!(meta.launch_kind.as_str(), heronite_data::launchers::LAUNCH_KIND | heronite_data::clients::LAUNCH_KIND) {
        let folder = game.install.install_path.as_deref().filter(|f| !f.is_empty());
        return InstallState { is_installed: game.is_installed && folder.is_none_or(|f| probe.exists(f)), executable: String::new() };
    }

    if meta.launch_kind == "emulator" {
        let executable = emulator_executable(ctx, meta).unwrap_or_else(local_exe);
        let roms_ok = emulator_rom_installed(meta, probe);
        let exe_ok = if meta.emulator_id.as_deref().map(|s| !s.is_empty()).unwrap_or(false) && executable == "flatpak" {
            true
        } else {
            !executable.is_empty() && probe.exists(&executable)
        };
        return InstallState { is_installed: exe_ok && roms_ok, executable: if exe_ok { executable } else { String::new() } };
    }

    // Started by a command: installed while its folder is there.
    if let Some((_, folder)) = heronite_data::sideload::command_of(game) {
        return InstallState { is_installed: probe.exists(folder), executable: game.install.executable.clone().unwrap_or_default() };
    }
    let executable = local_exe();
    if meta.launch_kind == "executable" || !executable.is_empty() {
        return InstallState { is_installed: !executable.is_empty(), executable };
    }
    InstallState { is_installed: false, executable: String::new() }
}

/// The sideload entry + Collection metadata created for a Steam game found
/// by scanning manifests (`steamSideloadEntry`). `not_played_status_id` is
/// the id of the "not played" completion status.
pub fn steam_sideload_entry(game: &SteamInstalledGame, steam_bin: Option<&str>, not_played_status_id: &str) -> (GameInfo, LocalGameMeta) {
    let app_name = format!("steam_{}", game.app_id);
    let (art_cover, art_square) = steam_cdn_covers(&game.app_id);
    let mut info = GameInfo {
        runner: "sideload".into(),
        app_name: app_name.clone(),
        title: game.name.clone(),
        art_cover: Some(art_cover),
        art_square: Some(art_square),
        is_installed: true,
        // Steam runs it: native to the system Heronite runs on.
        is_linux_native: Some(cfg!(not(windows))),
        install: InstallInfo {
            executable: Some(steam_bin.unwrap_or("steam").to_string()),
            platform: Some(if cfg!(windows) { "Windows" } else { "linux" }.into()),
            is_dlc: Some(false),
            ..Default::default()
        },
        ..Default::default()
    };
    info.rest.insert("canRunOffline".into(), serde_json::Value::Bool(false));
    let meta = LocalGameMeta {
        app_name,
        runner: "sideload".into(),
        playnite_id: format!("steam_{}", game.app_id),
        source: "steam".into(),
        launch_kind: "steam-uri".into(),
        steam_app_id: Some(game.app_id.clone()),
        store_game_id: Some(game.app_id.clone()),
        title: game.name.clone(),
        launcher_args: Some(format!("steam://rungameid/{}", game.app_id)),
        completion_status_id: Some(not_played_status_id.to_string()),
        ..Default::default()
    };
    let mut meta = meta;
    meta.extra.insert(heronite_data::steam_owned::AUTO_KEY.into(), Value::Bool(true));
    (info, meta)
}

/// Brings `installed` of the cached Steam library in line with the
/// manifests (a game installed or uninstalled since the client was read).
/// Whether the file changed.
fn refresh_owned_installs(paths: &DataPaths, installed_steam: &HashSet<String>) -> anyhow::Result<bool> {
    use heronite_data::steam_owned::SteamOwnedFile;
    let file = SteamOwnedFile::read(paths);
    if !file.games.iter().any(|g| g.installed != installed_steam.contains(&g.app_id)) {
        return Ok(false);
    }
    json_store::update::<SteamOwnedFile, _>(&paths.steam_owned(), |f| {
        for g in &mut f.games {
            g.installed = installed_steam.contains(&g.app_id);
        }
        Ok(())
    })?;
    Ok(true)
}

/// What [`refresh_local_install_states`] changed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RefreshReport {
    /// Games whose `is_installed` / executable changed.
    pub updated: usize,
    /// Steam games installed outside the Collection, added to it.
    pub added: usize,
}

impl RefreshReport {
    pub fn changed(&self) -> bool {
        self.updated > 0 || self.added > 0
    }
}

/// `launchKindForLocalExe`.
fn launch_kind_for_local_exe(existing: &str) -> &'static str {
    if existing == "emulator" {
        "emulator"
    } else {
        "executable"
    }
}

/// `refreshLocalInstallStates`: recomputes `is_installed` and the executable
/// of every sideloaded game with Collection metadata (a game uninstalled
/// from Steam, a moved executable, a missing ROM...) and adds the Steam games
/// installed since the last run, unless the game is already in the Collection
/// from another source (its Steam twin, `heronite_data::steam_owned`).
/// Blocking file I/O.
pub fn refresh_local_install_states(paths: &DataPaths, probe: &dyn Probe) -> anyhow::Result<RefreshReport> {
    let ctx = PlanContext::load(paths);
    let locator = SteamLocator::detect(&ctx.global, probe);
    let installed_steam = installed_app_ids(&locator);
    let steam_bin = find_steam_binary(probe);
    let metas: LocalMetaFile = json_store::read(&paths.local_meta())?;
    let doc = json_store::read_value(&paths.sideload_library())?;
    let entries: Vec<GameInfo> = doc
        .get("games")
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(|g| serde_json::from_value(g.clone()).ok()).collect())
        .unwrap_or_default();

    let mut states: HashMap<String, InstallState> = HashMap::new();
    // `syncMetaExecutable`: the executable that was found goes to the meta.
    let mut meta_exes: HashMap<String, String> = HashMap::new();
    for game in &entries {
        let Some(meta) = metas.games.get(&game.app_name) else { continue };
        let next = next_install_state(&ctx, probe, game, meta, &installed_steam, steam_bin.as_deref());
        if !next.executable.is_empty()
            && Some(next.executable.as_str()) != meta.remapped_executable.as_deref()
            && meta.launch_kind != "steam-uri"
        {
            meta_exes.insert(game.app_name.clone(), next.executable.clone());
        }
        if next.is_installed != game.is_installed || next.executable != game.install.executable.as_deref().unwrap_or("") {
            states.insert(game.app_name.clone(), next);
        }
    }

    let mut known: HashSet<String> = metas.games.values().filter_map(|m| m.steam_app_id.clone()).collect();
    for game in &entries {
        if let Some(id) = game.app_name.strip_prefix("steam_").filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())) {
            known.insert(id.to_string());
        }
    }
    let owned_changed = refresh_owned_installs(paths, &installed_steam)?;
    let mut new_steam = Vec::new();
    let mut installed_games = list_installed_games(&locator);
    // Once the client was read, its list says what is a game: tools and
    // games hidden in Steam stay out (a game bought since is added by the
    // next read of the library).
    let owned = heronite_data::steam_owned::SteamOwnedFile::read(paths);
    if owned.known() {
        installed_games.retain(|g| owned.get(&g.app_id).is_some());
    }
    if installed_games.iter().any(|g| !known.contains(&g.app_id)) {
        // Installed through Steam for a game the Collection has from another source.
        if let Ok(lib) = heronite_data::Library::load(paths) {
            known.extend(lib.games.iter().filter_map(|g| g.steam_link.as_ref().map(|l| l.app_id.clone())));
        }
    }
    if installed_games.iter().any(|g| !known.contains(&g.app_id)) {
        let statuses = heronite_data::status::load_statuses(paths).unwrap_or_else(|_| heronite_data::status::default_statuses());
        let not_played = heronite_data::status::inferred_status_id(0., &statuses);
        for g in installed_games {
            if known.insert(g.app_id.clone()) {
                new_steam.push(steam_sideload_entry(&g, steam_bin.as_deref(), &not_played));
            }
        }
    }

    if !meta_exes.is_empty() || !new_steam.is_empty() {
        json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |file| {
            for (app, exe) in &meta_exes {
                if let Some(m) = file.games.get_mut(app) {
                    m.launch_kind = launch_kind_for_local_exe(&m.launch_kind).into();
                    m.remapped_executable = Some(exe.clone());
                }
            }
            for (_, meta) in &new_steam {
                file.games.entry(meta.app_name.clone()).or_insert_with(|| meta.clone());
            }
            Ok(())
        })?;
    }
    let report = RefreshReport { updated: states.len() + usize::from(owned_changed), added: new_steam.len() };
    if states.is_empty() && new_steam.is_empty() {
        return Ok(report);
    }
    json_store::update_value(&paths.sideload_library(), |doc| {
        let obj = doc.as_object_mut().expect("update_value gives an object");
        let games = obj.entry("games").or_insert_with(|| Value::Array(Vec::new()));
        if !games.is_array() {
            *games = Value::Array(Vec::new());
        }
        let list = games.as_array_mut().expect("array");
        for g in list.iter_mut() {
            let Some(state) = g.get("app_name").and_then(Value::as_str).and_then(|a| states.get(a)) else { continue };
            let Some(entry) = g.as_object_mut() else { continue };
            entry.insert("is_installed".into(), Value::Bool(state.is_installed));
            let install = entry.entry("install").or_insert_with(|| Value::Object(Default::default()));
            if !install.is_object() {
                *install = Value::Object(Default::default());
            }
            install["executable"] = Value::String(state.executable.clone());
        }
        for (info, _) in &new_steam {
            list.push(serde_json::to_value(info)?);
        }
        Ok(())
    })?;
    if report.updated > 0 {
        log::info!("Updated install state for {} local game(s)", report.updated);
    }
    if report.added > 0 {
        log::info!("Added {} installed Steam game(s) to the local library", report.added);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::FakeProbe;
    use heronite_data::models::LocalGameRom;
    use heronite_data::DataPaths;

    fn ctx() -> PlanContext {
        let mut c = PlanContext::load(&DataPaths::at("/nonexistent-heroic-root"));
        c.emulators.emulators.push(crate::emulators::UserEmulator {
            id: "e1".into(),
            definition_id: "custom".into(),
            executable: "/emu/pcsx2".into(),
            launch_kind: "native".into(),
            ..Default::default()
        });
        c
    }

    fn game(exe: &str) -> GameInfo {
        GameInfo {
            runner: "sideload".into(),
            app_name: "g".into(),
            install: InstallInfo { executable: Some(exe.into()), ..Default::default() },
            ..Default::default()
        }
    }

    #[test]
    fn steam_games_follow_manifests() {
        let meta = LocalGameMeta { launch_kind: "steam-uri".into(), steam_app_id: Some("400".into()), ..Default::default() };
        let probe = FakeProbe::new("/h");
        let installed: HashSet<String> = ["400".to_string()].into();
        let on = next_install_state(&ctx(), &probe, &game(""), &meta, &installed, Some("/usr/bin/steam"));
        assert_eq!(on, InstallState { is_installed: true, executable: "/usr/bin/steam".into() });
        let off = next_install_state(&ctx(), &probe, &game("/usr/bin/steam"), &meta, &HashSet::new(), None);
        assert_eq!(off, InstallState { is_installed: false, executable: String::new() });
    }

    #[test]
    fn emulator_games_need_emulator_and_rom() {
        let probe = FakeProbe::new("/h").with_path("/emu/pcsx2").with_path("/roms/a.iso");
        let mut meta = LocalGameMeta {
            launch_kind: "emulator".into(),
            emulator_id: Some("e1".into()),
            roms: Some(vec![LocalGameRom { name: None, path: "/roms/a.iso".into(), extra: Default::default() }]),
            ..Default::default()
        };
        let s = next_install_state(&ctx(), &probe, &game(""), &meta, &HashSet::new(), None);
        assert_eq!(s, InstallState { is_installed: true, executable: "/emu/pcsx2".into() });
        meta.roms.as_mut().unwrap()[0].path = "/roms/missing.iso".into();
        let s = next_install_state(&ctx(), &probe, &game(""), &meta, &HashSet::new(), None);
        assert!(!s.is_installed);
        assert_eq!(s.executable, "/emu/pcsx2", "emulator itself is still present");
    }

    #[test]
    fn games_of_a_launcher_or_a_client_keep_the_state_their_sync_gave() {
        let probe = FakeProbe::new("/h").with_path("/pfx/drive_c/Games/Diablo IV");
        let installed = |folder: Option<&str>| GameInfo {
            is_installed: true,
            install: InstallInfo { install_path: folder.map(Into::into), ..Default::default() },
            ..game("")
        };
        for kind in [heronite_data::launchers::LAUNCH_KIND, heronite_data::clients::LAUNCH_KIND] {
            let meta = LocalGameMeta { launch_kind: kind.into(), ..Default::default() };
            let state = |g: &GameInfo| next_install_state(&ctx(), &probe, g, &meta, &HashSet::new(), None);
            let kept = state(&installed(Some("/pfx/drive_c/Games/Diablo IV")));
            assert_eq!(kept, InstallState { is_installed: true, executable: String::new() }, "{kind}");
            assert!(state(&installed(None)).is_installed, "{kind}: no folder to check");
            assert!(!state(&installed(Some("/pfx/drive_c/Games/Gone"))).is_installed, "{kind}: its folder is gone");
            assert!(!state(&game("")).is_installed, "{kind}: not installed in the sync");
        }
    }

    #[test]
    fn plain_executables_prefer_remapped_path() {
        let probe = FakeProbe::new("/h").with_path("/new/game").with_path("/old/game");
        let meta = LocalGameMeta { launch_kind: "executable".into(), remapped_executable: Some("/new/game".into()), ..Default::default() };
        assert_eq!(next_install_state(&ctx(), &probe, &game("/old/game"), &meta, &HashSet::new(), None).executable, "/new/game");
        let gone = LocalGameMeta { launch_kind: "executable".into(), ..Default::default() };
        assert!(!next_install_state(&ctx(), &probe, &game("/nope"), &gone, &HashSet::new(), None).is_installed);
    }

    #[test]
    fn new_steam_entries_match_heronite_shape() {
        let (info, meta) = steam_sideload_entry(&SteamInstalledGame { app_id: "400".into(), name: "Portal".into() }, None, "not-played");
        assert_eq!(info.app_name, "steam_400");
        assert_eq!(info.install.executable.as_deref(), Some("steam"));
        assert_eq!(info.rest["canRunOffline"], false);
        assert_eq!(meta.launcher_args.as_deref(), Some("steam://rungameid/400"));
        assert_eq!(meta.launch_kind, "steam-uri");
        assert_eq!(meta.extra[heronite_data::steam_owned::AUTO_KEY], true);
    }

    #[test]
    fn refresh_follows_steam_uninstalls_and_installs() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let steamapps = home.join(".steam/steam/steamapps");
        std::fs::create_dir_all(&steamapps).unwrap();
        std::fs::write(
            steamapps.join("appmanifest_620.acf"),
            "\"AppState\"\n{\n\t\"appid\"\t\t\"620\"\n\t\"name\"\t\t\"Portal 2\"\n\t\"StateFlags\"\t\t\"4\"\n}\n",
        )
        .unwrap();
        let paths = DataPaths::at(dir.path().join("heroic"));
        std::fs::create_dir_all(paths.sideload_library().parent().unwrap()).unwrap();
        std::fs::write(
            paths.sideload_library(),
            r#"{"games": [{"runner": "sideload", "app_name": "steam_400", "title": "Portal", "is_installed": true, "install": {"executable": "steam", "platform": "linux"}, "zz": 1}]}"#,
        )
        .unwrap();
        std::fs::create_dir_all(paths.local_meta().parent().unwrap()).unwrap();
        std::fs::write(
            paths.local_meta(),
            r#"{"games": {"steam_400": {"appName": "steam_400", "runner": "sideload", "launchKind": "steam-uri", "steamAppId": "400", "title": "Portal"}}}"#,
        )
        .unwrap();
        let probe = FakeProbe::new(home.to_str().unwrap());

        let report = refresh_local_install_states(&paths, &probe).unwrap();
        assert_eq!(report, RefreshReport { updated: 1, added: 1 });
        let doc = json_store::read_value(&paths.sideload_library()).unwrap();
        let games = doc["games"].as_array().unwrap();
        assert_eq!(games[0]["is_installed"], false);
        assert_eq!(games[0]["install"]["executable"], "");
        assert_eq!(games[0]["zz"], 1);
        assert_eq!(games[1]["app_name"], "steam_620");
        assert_eq!(games[1]["is_installed"], true);
        let metas: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap();
        assert_eq!(metas.games["steam_620"].launch_kind, "steam-uri");

        // Nothing changed since: the files are left alone.
        assert!(!refresh_local_install_states(&paths, &probe).unwrap().changed());

        // Installed through Steam for an Epic game of the Collection: no Steam
        // entry, and the cached library learns it is installed.
        std::fs::write(
            steamapps.join("appmanifest_870780.acf"),
            "\"AppState\"\n{\n\t\"appid\"\t\t\"870780\"\n\t\"name\"\t\t\"Control\"\n\t\"StateFlags\"\t\t\"4\"\n}\n",
        )
        .unwrap();
        std::fs::create_dir_all(paths.legendary_library().parent().unwrap()).unwrap();
        std::fs::write(
            paths.legendary_library(),
            r#"{"library":[{"runner":"legendary","app_name":"ctl","title":"Control","install":{}}],"__timestamp":{"library":"x"}}"#,
        )
        .unwrap();
        std::fs::write(paths.steam_owned(), r#"{"games":[{"appId":"870780","name":"Control"}],"updatedAt":"x"}"#).unwrap();
        let report = refresh_local_install_states(&paths, &probe).unwrap();
        assert_eq!(report, RefreshReport { updated: 1, added: 0 });
        let owned = heronite_data::steam_owned::SteamOwnedFile::read(&paths);
        assert!(owned.games[0].installed);
        let doc = json_store::read_value(&paths.sideload_library()).unwrap();
        assert_eq!(doc["games"].as_array().unwrap().len(), 2);
    }
}
