//! The backup flow: one game, or after a play session.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use heronite_data::models::{LocalGameMeta, LocalMetaFile};
use heronite_data::{json_store, settings, DataPaths};
use heronite_i18n::tr;
use heronite_launch::emulators::EmulatorsFile;
use heronite_launch::probe::Probe;
use heronite_launch::settings::{load_game_settings, GlobalSettings};
use serde::{Deserialize, Serialize};

use super::commands::{
    backup_command, best_find_match, find_commands, game_names, parse_json, total_games, wine_prefix_for_ludusavi, BackupCommand,
};
use super::detect::detect_ludusavi;
use super::runner::{CommandOutput, CommandRunner, LUDUSAVI_TIMEOUT};
use super::titles::emulator_names;
use crate::iso::iso_now;
use crate::pathutil::join;
use crate::settings_ext::{record_ludusavi_backup, LudusaviRecord};

pub const SKIP_SESSION_SECONDS: u64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupReason {
    /// After a play session: waits up to [`AFTER_PLAY_WAIT`] for a backup
    /// already running (a periodic one of another game, one by hand).
    Auto,
    /// While the game runs (`ludusavi.intervalMinutes`): skipped when
    /// another backup is running, the next one comes an interval later.
    Periodic,
    Manual,
}

impl BackupReason {
    /// Follows the automatic backup settings (on, off for the game).
    fn automatic(self) -> bool {
        self != BackupReason::Manual
    }
}

/// How long the after-play backup waits for one already running: about one
/// Ludusavi run, so the last backup of a session is not lost to a periodic
/// one still copying.
pub const AFTER_PLAY_WAIT: Duration = LUDUSAVI_TIMEOUT;

/// Arguments of `runLudusaviBackup`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LudusaviGame {
    pub app_name: String,
    pub title: String,
    pub runner: String,
    #[serde(default)]
    pub steam_app_id: Option<String>,
    #[serde(default)]
    pub store_game_id: Option<String>,
    /// Id of the user's emulator that runs the game (an `emulators.json`
    /// entry, the meta's `emulatorId`); `None` when the game is not emulated.
    #[serde(default)]
    pub emulator_id: Option<String>,
    /// The ROM that runs the game (emulated games): the name the emulator
    /// manifests build their entries from. `None` when the game is not
    /// emulated or its meta lists no ROM.
    #[serde(default)]
    pub rom_path: Option<String>,
}

impl LudusaviGame {
    /// A launched game, with the store ids kept in its overlay meta (Ludusavi
    /// finds a game by its Steam/GOG id when the title differs).
    pub fn of_session(paths: &DataPaths, app_name: &str, title: &str, runner: &str) -> Self {
        let metas: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap_or_default();
        let meta = metas.games.get(app_name);
        LudusaviGame {
            app_name: app_name.to_owned(),
            title: title.to_owned(),
            runner: runner.to_owned(),
            steam_app_id: meta.and_then(|m| m.steam_app_id.clone()),
            store_game_id: meta.and_then(|m| m.store_game_id.clone()),
            emulator_id: Self::emulator_of(meta),
            rom_path: Self::rom_path_of(meta),
        }
    }

    /// The emulator of an emulated game. Emulated games are sideloaded
    /// (`runner` is `sideload`), so the emulator lives in the overlay meta.
    pub fn emulator_of(meta: Option<&LocalGameMeta>) -> Option<String> {
        meta.filter(|m| m.launch_kind == "emulator").and_then(|m| m.emulator_id.clone()).filter(|id| !id.is_empty())
    }

    /// The ROM a meta lists: the one selected for play, else the first
    /// non-empty `roms[].path`. Emulated games carry one; other games simply
    /// have none. The manifests name an entry after the ROM's title, so this
    /// is what covers a Collection title that was renamed, stripped of
    /// punctuation or cut short.
    pub fn rom_path_of(meta: Option<&LocalGameMeta>) -> Option<String> {
        let meta = meta?;
        if let Some(path) = meta.selected_rom_path.as_ref().filter(|path| !path.is_empty()) {
            return Some(path.clone());
        }
        meta.roms.as_ref()?.iter().map(|rom| rom.path.as_str()).find(|path| !path.is_empty()).map(str::to_owned)
    }
}

/// `LudusaviBackupResult`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LudusaviBackupResult {
    pub ran: bool,
    /// 'disabled' | 'not-found' | 'no-match' | 'already-running'
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_games: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

static IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// One Ludusavi backup or restore at a time.
pub(super) struct InFlight;

impl InFlight {
    pub(super) fn acquire() -> Option<Self> {
        IN_FLIGHT.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).ok().map(|_| InFlight)
    }

    /// [`Self::acquire`], trying again for up to `wait`.
    pub(super) fn acquire_within(wait: Duration) -> Option<Self> {
        let deadline = Instant::now() + wait;
        loop {
            if let Some(g) = Self::acquire() {
                return Some(g);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            std::thread::sleep(left.min(Duration::from_millis(100)));
        }
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        IN_FLIGHT.store(false, Ordering::SeqCst);
    }
}

/// Tests that take the in-flight guard (backup and restore ones) run one at
/// a time, so none of them finds it taken by another.
#[cfg(test)]
pub(super) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static L: std::sync::Mutex<()> = std::sync::Mutex::new(());
    L.lock().unwrap_or_else(|e| e.into_inner())
}

/// Everything the flow reads from or runs on the machine.
pub struct LudusaviEnv<'a> {
    pub paths: &'a DataPaths,
    pub runner: &'a dyn CommandRunner,
    pub probe: &'a dyn Probe,
}

fn record_error(paths: &DataPaths, error: &str) {
    if let Err(e) = record_ludusavi_backup(paths, &LudusaviRecord { error: Some(error.to_string()), ..Default::default() }) {
        log::error!("could not record Ludusavi error: {e:#}");
    }
}

/// The game's own Wine prefix (the folder holding `drive_c`), from its game
/// settings. `None` for Steam-launched games: their prefix is Steam's. `None`
/// on Windows, where games run as they are (a Wine prefix in settings
/// restored from Linux means nothing there).
pub fn game_wine_prefix(env: &LudusaviEnv, app_name: &str) -> Option<String> {
    if cfg!(windows) {
        return None;
    }
    let metas: LocalMetaFile = json_store::read(&env.paths.local_meta()).ok()?;
    if metas.games.get(app_name).is_some_and(|m| m.launch_kind == "steam-uri") {
        return None;
    }
    let global = GlobalSettings::load(env.paths);
    let game = load_game_settings(env.paths, app_name, &global, env.probe);
    wine_prefix_for_ludusavi(&game.wine_prefix, env.probe)
}

/// `resolveLudusaviTitle`: Ludusavi's own name for the game (Steam/GOG id
/// first, then the title), falling back to the title.
pub(super) fn resolve_title(env: &LudusaviEnv, binary: &str, game: &LudusaviGame) -> String {
    for cmd in find_commands(&game.title, game.steam_app_id.as_deref(), &game.runner, game.store_game_id.as_deref()) {
        if let Ok(out) = env.runner.run(binary, &cmd) {
            if let Some(m) = best_find_match(&parse_json(&out.stdout)) {
                return m;
            }
        }
    }
    game.title.clone()
}

/// What the user's emulator says about the Ludusavi name of a game it runs.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct EmulatorNaming {
    /// The catalog id (`retroarch`, `pcsx2`); a custom emulator is recognised
    /// by its name or program.
    pub engine: Option<String>,
    /// The one Ludusavi entry that holds the saves of all its games
    /// (`UserEmulator::ludusavi_entry`): when set, it is the only name tried.
    pub entry: Option<String>,
}

/// [`EmulatorNaming`] of the user's emulator `emulator_id` (an
/// `emulators.json` id); empty when the game is not emulated.
pub(crate) fn emulator_naming(paths: &DataPaths, emulator_id: Option<&str>) -> EmulatorNaming {
    let Some(id) = emulator_id.filter(|id| !id.is_empty()) else { return EmulatorNaming::default() };
    let emulators: EmulatorsFile = json_store::read(&paths.emulators()).unwrap_or_default();
    let Some(emulator) = emulators.get(id) else { return EmulatorNaming::default() };
    EmulatorNaming { engine: Some(crate::save_states::engine_id_for(emulator)), entry: emulator.ludusavi_entry().map(str::to_owned) }
}

/// The catalog id (`retroarch`, `pcsx2`) of the user's emulator `emulator_id`
/// ([`EmulatorNaming::engine`]).
pub(crate) fn emulator_engine(paths: &DataPaths, emulator_id: Option<&str>) -> Option<String> {
    emulator_naming(paths, emulator_id).engine
}

/// `backupLudusaviForGame`.
pub fn backup_for_game(env: &LudusaviEnv, game: &LudusaviGame, reason: BackupReason) -> LudusaviBackupResult {
    backup_for_game_within(env, game, reason, AFTER_PLAY_WAIT)
}

/// [`backup_for_game`]; an [`BackupReason::Auto`] backup waits up to `wait`
/// for one already running.
pub(super) fn backup_for_game_within(env: &LudusaviEnv, game: &LudusaviGame, reason: BackupReason, wait: Duration) -> LudusaviBackupResult {
    log::info!("Ludusavi {reason:?} backup requested for {}", game.title);
    let guard = match reason {
        BackupReason::Auto => InFlight::acquire_within(wait),
        BackupReason::Periodic | BackupReason::Manual => InFlight::acquire(),
    };
    let Some(_guard) = guard else {
        log::info!("Ludusavi {reason:?} backup of {} skipped: another backup is running", game.title);
        return LudusaviBackupResult { skipped_reason: Some("already-running".into()), ..Default::default() };
    };
    match backup_inner(env, game, reason) {
        Ok(r) => r,
        Err(e) => {
            let message = format!("{e:#}");
            record_error(env.paths, &message);
            log::error!("Ludusavi backup failed: {message}");
            LudusaviBackupResult { error: Some(message), ..Default::default() }
        }
    }
}

fn backup_inner(env: &LudusaviEnv, game: &LudusaviGame, reason: BackupReason) -> Result<LudusaviBackupResult> {
    let cfg = settings::load(env.paths)?.ludusavi;
    if reason.automatic() && !cfg.enabled {
        return Ok(LudusaviBackupResult { skipped_reason: Some("disabled".into()), ..Default::default() });
    }
    if reason.automatic() && !cfg.auto_backup_for(&game.runner, &game.app_name) {
        log::info!("Ludusavi auto backup is off for {}", game.title);
        return Ok(LudusaviBackupResult { skipped_reason: Some("off-for-game".into()), ..Default::default() });
    }
    let preferred = Some(cfg.binary_path.as_str()).filter(|p| !p.is_empty());
    let detected = detect_ludusavi(env.runner, env.probe, preferred);
    let binary = Some(cfg.binary_path.trim().to_string()).filter(|b| !b.is_empty()).or_else(|| detected.as_ref().map(|d| d.binary.clone()));
    let Some(binary) = binary.filter(|b| env.probe.exists(b)) else {
        let error = "Ludusavi was not found".to_string();
        record_error(env.paths, &error);
        return Ok(LudusaviBackupResult {
            skipped_reason: Some("not-found".into()),
            error: Some(tr!("Ludusavi was not found").to_string()),
            ..Default::default()
        });
    };

    let use_installed = cfg.use_installed_config && detected.as_ref().is_some_and(|d| d.config_path.is_some());
    let configured = cfg.backup_path.trim().to_string();
    let detected_backup = detected.as_ref().and_then(|d| d.backup_path.clone());
    let backup_root = if use_installed { detected_backup } else { Some(configured.clone()).filter(|c| !c.is_empty()).or(detected_backup) };
    if !use_installed && configured.is_empty() {
        let error = "Set a Ludusavi backup folder in Collection settings".to_string();
        record_error(env.paths, &error);
        return Ok(LudusaviBackupResult {
            error: Some(tr!("Set a Ludusavi backup folder in Collection settings").to_string()),
            ..Default::default()
        });
    }

    let wine_prefix = game_wine_prefix(env, &game.app_name);
    let run_backup = |name: &str| -> Result<CommandOutput> {
        let cmd = backup_command(&BackupCommand {
            use_installed,
            configured_path: &configured,
            format: &cfg.format,
            compression: &cfg.compression,
            wine_prefix: wine_prefix.as_deref(),
            full_limit: cfg.full_limit,
            differential_limit: cfg.differential_limit,
            game_name: name,
        });
        log::info!("Running {binary} {}", cmd.join(" "));
        env.runner.run(&binary, &cmd).map_err(anyhow::Error::msg)
    };

    // An emulator whose games all keep their saves under one Ludusavi entry
    // (a custom entry for RetroArch's saves folder) is backed up under that
    // entry, whatever the game: the per-game names are never tried.
    let naming = emulator_naming(env.paths, game.emulator_id.as_deref());
    let looked_up = naming.entry.clone().unwrap_or_else(|| game.title.clone());
    let mut game_name = looked_up.clone();
    let mut result = run_backup(&game_name)?;
    let mut payload = parse_json(&result.stdout);
    let mut total = total_games(&payload);

    // A secondary manifest may name an emulated game after its platform
    // ("… (RetroArch)"), which the library title does not carry. The bare
    // title is tried first, because that is the primary manifest's convention;
    // only when it found nothing are the suffixed names tried, one run each,
    // under the usual timeout and the single in-flight guard. A suffixed name
    // without saves leaves no backup behind, so the order is safe.
    if total == 0 && naming.entry.is_none() {
        for name in emulator_names(naming.engine.as_deref(), &game.title, game.rom_path.as_deref()) {
            if name == game_name {
                continue;
            }
            let attempt = run_backup(&name)?;
            let attempt_payload = parse_json(&attempt.stdout);
            if total_games(&attempt_payload) > 0 {
                game_name = name;
                result = attempt;
                payload = attempt_payload;
                total = total_games(&payload);
                break;
            }
        }
    }

    if naming.entry.is_none()
        && result.code != 0
        && total == 0
        && (game.steam_app_id.as_deref().is_some_and(|s| !s.is_empty()) || game.runner == "gog")
    {
        game_name = resolve_title(env, &binary, game);
        if game_name != game.title {
            result = run_backup(&game_name)?;
            payload = parse_json(&result.stdout);
            total = total_games(&payload);
        }
    }

    if let Some(first) = game_names(&payload).into_iter().next() {
        game_name = first;
    }
    let backup_path = backup_root.as_deref().map(|r| join(r, &game_name));
    let stderr = result.stderr.trim().to_string();

    if result.code != 0 && total == 0 {
        let recorded = if stderr.is_empty() { format!("Ludusavi did not find saves for {looked_up}") } else { stderr.clone() };
        let error = if stderr.is_empty() { tr!("Ludusavi did not find saves for {title}", title = looked_up) } else { stderr };
        record_error(env.paths, &recorded);
        log::error!("Ludusavi backup failed: {recorded}");
        return Ok(LudusaviBackupResult {
            skipped_reason: Some("no-match".into()),
            game_name: Some(game_name),
            error: Some(error),
            ..Default::default()
        });
    }

    record_ludusavi_backup(
        env.paths,
        &LudusaviRecord { at: Some(iso_now()), game: Some(game_name.clone()), path: backup_path.clone(), error: None },
    )?;
    Ok(LudusaviBackupResult { ran: true, game_name: Some(game_name), backup_path, total_games: Some(total), ..Default::default() })
}

/// Arguments of `backupLudusaviAfterPlay`.
#[derive(Debug, Clone)]
pub struct AfterPlay {
    pub app_name: String,
    pub title: String,
    pub runner: String,
    pub elapsed_seconds: u64,
}

/// `backupLudusaviAfterPlay`: skips sessions shorter than
/// [`SKIP_SESSION_SECONDS`] (returns `None`), otherwise runs an automatic
/// backup with the ids stored in the game's overlay meta.
pub fn backup_after_play(env: &LudusaviEnv, play: &AfterPlay) -> Option<LudusaviBackupResult> {
    if play.elapsed_seconds < SKIP_SESSION_SECONDS {
        log::info!("Skipping Ludusavi auto backup; session was only {}s", play.elapsed_seconds);
        return None;
    }
    let game = LudusaviGame::of_session(env.paths, &play.app_name, &play.title, &play.runner);
    let result = backup_for_game(env, &game, BackupReason::Auto);
    if !result.ran {
        if let Some(e) = &result.error {
            log::error!("Ludusavi save backup after play failed: {e}");
        }
    }
    Some(result)
}

/// The desktop notification `(title, body)` for a finished after-play backup.
pub fn notification(result: &LudusaviBackupResult, title: &str) -> Option<(String, String)> {
    if !result.ran {
        return None;
    }
    let name = result.game_name.as_deref().unwrap_or(title);
    let body = match &result.backup_path {
        Some(p) => tr!("Saved {name} to {path}", name, path = p),
        None => tr!("Saved a backup for {name}", name),
    };
    Some(("Ludusavi".into(), body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ludusavi::fake::{out, Fake};
    use heronite_data::models::LocalGameRom;
    use heronite_launch::probe::FakeProbe;

    use super::test_lock as guard_lock;

    fn setup(settings_json: &str) -> (tempfile::TempDir, DataPaths) {
        let d = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(d.path());
        std::fs::create_dir_all(paths.local_library()).unwrap();
        std::fs::write(paths.collection_settings(), settings_json).unwrap();
        (d, paths)
    }

    const ENABLED: &str = r#"{"ludusavi":{"enabled":true,"useInstalledConfig":false,"binaryPath":"","backupPath":"/backups","format":"zip","compression":"zstd"}}"#;

    #[test]
    fn successful_backup_records_status() {
        let _l = guard_lock();
        let (_d, paths) = setup(ENABLED);
        let runner = Fake {
            replies: vec![("backup".into(), out(r#"{"overall":{"totalGames":1},"games":{"Tony Hawk's Underground":{}}}"#, "", 0))],
            ..Default::default()
        };
        let probe = FakeProbe::new("/h").with_bin("ludusavi", "/usr/bin/ludusavi");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        let game =
            LudusaviGame { app_name: "g".into(), title: "Tony Hawks Underground".into(), runner: "sideload".into(), ..Default::default() };
        let r = backup_for_game(&env, &game, BackupReason::Auto);
        assert!(r.ran, "{r:?}");
        assert_eq!(r.game_name.as_deref(), Some("Tony Hawk's Underground"));
        assert_eq!(r.backup_path, Some(join("/backups", "Tony Hawk's Underground")));
        assert_eq!(r.total_games, Some(1));
        let calls = runner.calls.lock().unwrap();
        let backup_call = calls.iter().find(|c| c[0] == "backup").unwrap();
        assert_eq!(
            backup_call[..10],
            ["backup", "--force", "--api", "--no-cloud-sync", "--path", "/backups", "--format", "zip", "--compression", "zstd"]
        );
        assert_eq!(backup_call.last().unwrap(), "Tony Hawks Underground");
        let l = settings::load(&paths).unwrap().ludusavi;
        assert_eq!(l.last_backup_game.as_deref(), Some("Tony Hawk's Underground"));
        assert_eq!(l.last_backup_path, Some(join("/backups", "Tony Hawk's Underground")));
        assert!(l.last_backup_at.is_some());
        assert_eq!(l.last_error, None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_backup_with_windows_paths_and_no_prefix() {
        let _l = guard_lock();
        let (_d, paths) = setup(
            r#"{"ludusavi":{"enabled":true,"useInstalledConfig":false,"binaryPath":"","backupPath":"D:\\Game Saves","format":"simple"}}"#,
        );
        let reply = r#"{"overall":{"totalGames":1},"games":{"Hollow Knight":{"files":{"C:/Users/u/AppData/LocalLow/Team Cherry/Hollow Knight/user1.dat":{"change":"New","bytes":3}}}}}"#;
        let runner = Fake { replies: vec![("backup".into(), out(reply, "", 0))], ..Default::default() };
        let exe = r"C:\Users\u\AppData\Local\Programs\Ludusavi\ludusavi.exe";
        let probe = FakeProbe::new(r"C:\Users\u").with_path(exe).with_env("LOCALAPPDATA", r"C:\Users\u\AppData\Local");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        let game = LudusaviGame {
            app_name: "hk".into(),
            title: "Hollow Knight".into(),
            runner: "sideload".into(),
            steam_app_id: Some("367520".into()),
            ..Default::default()
        };
        let r = backup_for_game(&env, &game, BackupReason::Manual);
        assert!(r.ran, "{r:?}");
        assert_eq!(r.backup_path.as_deref(), Some(r"D:\Game Saves\Hollow Knight"));
        let calls = runner.calls.lock().unwrap();
        let backup_call = calls.iter().find(|c| c[0] == "backup").unwrap();
        assert_eq!(backup_call[4..], ["--path", r"D:\Game Saves", "--format", "simple", "Hollow Knight"]);
        assert!(!backup_call.iter().any(|a| a == "--wine-prefix"));
        drop(calls);
        assert_eq!(settings::load(&paths).unwrap().ludusavi.last_backup_path.as_deref(), Some(r"D:\Game Saves\Hollow Knight"));
    }

    #[test]
    fn disabled_missing_binary_and_missing_folder() {
        let _l = guard_lock();
        let game = LudusaviGame { app_name: "g".into(), title: "T".into(), runner: "sideload".into(), ..Default::default() };
        let runner = Fake::default();
        let probe = FakeProbe::new("/h").with_bin("ludusavi", "/usr/bin/ludusavi");
        // Disabled: auto skipped, manual proceeds.
        let (_d, paths) = setup(r#"{"ludusavi":{"enabled":false,"useInstalledConfig":false,"backupPath":"/b"}}"#);
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        let r = backup_for_game(&env, &game, BackupReason::Auto);
        assert_eq!(r.skipped_reason.as_deref(), Some("disabled"));
        assert!(runner.calls.lock().unwrap().is_empty());
        // Off for this game only: auto skipped, manual proceeds.
        let (_d1, paths1) =
            setup(r#"{"ludusavi":{"enabled":true,"useInstalledConfig":false,"backupPath":"/b","autoBackupOff":["sideload:g"]}}"#);
        let env1 = LudusaviEnv { paths: &paths1, runner: &runner, probe: &probe };
        let r = backup_for_game(&env1, &game, BackupReason::Auto);
        assert_eq!(r.skipped_reason.as_deref(), Some("off-for-game"));
        assert!(runner.calls.lock().unwrap().is_empty());
        backup_for_game(&env1, &game, BackupReason::Manual);
        assert!(!runner.calls.lock().unwrap().is_empty());
        runner.calls.lock().unwrap().clear();
        // Binary not found.
        let (_d2, paths2) = setup(ENABLED);
        let none = FakeProbe::new("/h");
        let env2 = LudusaviEnv { paths: &paths2, runner: &runner, probe: &none };
        let r = backup_for_game(&env2, &game, BackupReason::Manual);
        assert_eq!(r.skipped_reason.as_deref(), Some("not-found"));
        assert_eq!(r.error.as_deref(), Some("Ludusavi was not found"));
        assert_eq!(settings::load(&paths2).unwrap().ludusavi.last_error.as_deref(), Some("Ludusavi was not found"));
        // Own config but no folder.
        let (_d3, paths3) = setup(r#"{"ludusavi":{"enabled":true,"useInstalledConfig":false,"backupPath":"  "}}"#);
        let env3 = LudusaviEnv { paths: &paths3, runner: &runner, probe: &probe };
        let r = backup_for_game(&env3, &game, BackupReason::Auto);
        assert_eq!(r.error.as_deref(), Some("Set a Ludusavi backup folder in Collection settings"));
        assert!(!r.ran);
    }

    #[test]
    fn failed_lookup_retries_with_find_and_reports_no_match() {
        let _l = guard_lock();
        let (_d, paths) = setup(ENABLED);
        let runner = Fake {
            replies: vec![
                ("find --api --steam-id 42".into(), out(r#"{"games":{"Real Name":{"score":1.0}}}"#, "", 0)),
                (
                    "backup --force --api --no-cloud-sync --path /backups --format zip --compression zstd Real Name".into(),
                    out("{}", "No info for these games:\n  - Real Name", 1),
                ),
                ("backup".into(), out("{}", "", 1)),
            ],
            ..Default::default()
        };
        let probe = FakeProbe::new("/h").with_bin("ludusavi", "/usr/bin/ludusavi");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        let game = LudusaviGame {
            app_name: "g".into(),
            title: "Title".into(),
            runner: "sideload".into(),
            steam_app_id: Some("42".into()),
            ..Default::default()
        };
        let r = backup_for_game(&env, &game, BackupReason::Manual);
        assert!(!r.ran);
        assert_eq!(r.skipped_reason.as_deref(), Some("no-match"));
        assert_eq!(r.game_name.as_deref(), Some("Real Name"));
        assert_eq!(r.error.as_deref(), Some("No info for these games:\n  - Real Name"));
        let calls = runner.calls.lock().unwrap();
        assert!(calls.iter().any(|c| c.join(" ") == "find --api --steam-id 42"));
        assert_eq!(calls.iter().filter(|c| c[0] == "backup").count(), 2);
        drop(calls);
        let l = settings::load(&paths).unwrap().ludusavi;
        assert_eq!(l.last_error.as_deref(), Some("No info for these games:\n  - Real Name"));
    }

    #[test]
    fn after_play_waits_for_a_running_backup_and_the_others_skip() {
        let _l = guard_lock();
        let (_d, paths) = setup(ENABLED);
        let runner =
            Fake { replies: vec![("backup".into(), out(r#"{"overall":{"totalGames":1},"games":{"G":{}}}"#, "", 0))], ..Default::default() };
        let probe = FakeProbe::new("/h").with_bin("ludusavi", "/usr/bin/ludusavi");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        let game = LudusaviGame { app_name: "g".into(), title: "G".into(), runner: "sideload".into(), ..Default::default() };
        let held = InFlight::acquire().expect("nothing else is running");
        for reason in [BackupReason::Periodic, BackupReason::Manual] {
            let r = backup_for_game_within(&env, &game, reason, Duration::from_secs(5));
            assert_eq!(r.skipped_reason.as_deref(), Some("already-running"), "{reason:?}");
        }
        let r = backup_for_game_within(&env, &game, BackupReason::Auto, Duration::from_millis(50));
        assert_eq!(r.skipped_reason.as_deref(), Some("already-running"), "gives up after its wait");
        assert!(runner.calls.lock().unwrap().is_empty());
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            drop(held);
        });
        let r = backup_for_game_within(&env, &game, BackupReason::Auto, Duration::from_secs(10));
        release.join().unwrap();
        assert!(r.ran, "the after-play backup runs once the other one ends: {r:?}");
        assert!(InFlight::acquire().is_some(), "and lets go of the guard");
    }

    #[test]
    fn periodic_backup_follows_the_automatic_settings() {
        let _l = guard_lock();
        let runner = Fake::default();
        let probe = FakeProbe::new("/h").with_bin("ludusavi", "/usr/bin/ludusavi");
        let game = LudusaviGame { app_name: "g".into(), title: "G".into(), runner: "sideload".into(), ..Default::default() };
        let (_d, paths) = setup(r#"{"ludusavi":{"enabled":false,"useInstalledConfig":false,"backupPath":"/b"}}"#);
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        assert_eq!(backup_for_game(&env, &game, BackupReason::Periodic).skipped_reason.as_deref(), Some("disabled"));
        let (_d1, paths1) =
            setup(r#"{"ludusavi":{"enabled":true,"useInstalledConfig":false,"backupPath":"/b","autoBackupOff":["sideload:g"]}}"#);
        let env1 = LudusaviEnv { paths: &paths1, runner: &runner, probe: &probe };
        assert_eq!(backup_for_game(&env1, &game, BackupReason::Periodic).skipped_reason.as_deref(), Some("off-for-game"));
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn after_play_skips_short_sessions_and_uses_meta_ids() {
        let _l = guard_lock();
        let (_d, paths) = setup(ENABLED);
        json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |f| {
            f.games.insert(
                "g".into(),
                heronite_data::models::LocalGameMeta {
                    app_name: "g".into(),
                    runner: "sideload".into(),
                    steam_app_id: Some("777".into()),
                    ..Default::default()
                },
            );
            Ok(())
        })
        .unwrap();
        let runner =
            Fake { replies: vec![("backup".into(), out(r#"{"overall":{"totalGames":0},"games":{"G":{}}}"#, "", 0))], ..Default::default() };
        let probe = FakeProbe::new("/h").with_bin("ludusavi", "/usr/bin/ludusavi");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        let short = AfterPlay { app_name: "g".into(), title: "G".into(), runner: "sideload".into(), elapsed_seconds: 4 };
        assert!(backup_after_play(&env, &short).is_none());
        assert!(runner.calls.lock().unwrap().is_empty());
        let long = AfterPlay { elapsed_seconds: 60, ..short };
        let r = backup_after_play(&env, &long).unwrap();
        assert!(r.ran);
        assert_eq!(r.total_games, Some(0));
        let (title, body) = notification(&r, "G").unwrap();
        assert_eq!(title, "Ludusavi");
        assert_eq!(body, format!("Saved G to {}", join("/backups", "G")));
        assert!(notification(&LudusaviBackupResult::default(), "G").is_none());
    }

    /// Adds a user emulator to `emulators.json`, under a generated id like
    /// the import writes into the meta.
    fn seed_emulator(paths: &DataPaths, id: &str, definition_id: &str, name: &str) {
        seed_emulator_with_entry(paths, id, definition_id, name, None);
    }

    fn seed_emulator_with_entry(paths: &DataPaths, id: &str, definition_id: &str, name: &str, entry: Option<&str>) {
        json_store::update::<EmulatorsFile, _>(&paths.emulators(), |f| {
            f.emulators.push(heronite_launch::emulators::UserEmulator {
                id: id.into(),
                definition_id: definition_id.into(),
                name: name.into(),
                ludusavi_entry: entry.map(str::to_owned),
                ..Default::default()
            });
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn the_catalog_id_comes_from_the_users_emulator() {
        let (_d, paths) = setup(ENABLED);
        seed_emulator(&paths, "7d0c2f1e", "retroarch", "RetroArch");
        seed_emulator(&paths, "a91b44c3", "custom", "Dolphin");
        let engine = |id| emulator_naming(&paths, id).engine;
        assert_eq!(engine(Some("7d0c2f1e")).as_deref(), Some("retroarch"));
        assert_eq!(engine(Some("a91b44c3")).as_deref(), Some("dolphin"));
        assert_eq!(engine(Some("missing")), None);
        assert_eq!(engine(None), None);
    }

    fn emulated_meta(launch_kind: &str, emulator_id: &str) -> LocalGameMeta {
        LocalGameMeta {
            app_name: "g".into(),
            runner: "sideload".into(),
            launch_kind: launch_kind.into(),
            emulator_id: Some(emulator_id.into()),
            ..Default::default()
        }
    }

    #[test]
    fn the_emulator_comes_from_an_emulated_games_meta() {
        assert_eq!(LudusaviGame::emulator_of(Some(&emulated_meta("emulator", "7d0c2f1e"))).as_deref(), Some("7d0c2f1e"));
        // A game turned into a plain executable keeps the stale id in its meta.
        assert_eq!(LudusaviGame::emulator_of(Some(&emulated_meta("executable", "7d0c2f1e"))), None);
        assert_eq!(LudusaviGame::emulator_of(Some(&emulated_meta("emulator", ""))), None);
        assert_eq!(LudusaviGame::emulator_of(None), None);
    }

    #[test]
    fn the_rom_comes_from_the_meta() {
        let mut meta = emulated_meta("emulator", "7d0c2f1e");
        meta.selected_rom_path = Some("/roms/selected.sfc".into());
        meta.roms = Some(vec![LocalGameRom { path: "/roms/first.sfc".into(), ..Default::default() }]);
        // The selected ROM wins, the first listed one is the fallback.
        assert_eq!(LudusaviGame::rom_path_of(Some(&meta)).as_deref(), Some("/roms/selected.sfc"));
        meta.selected_rom_path = None;
        assert_eq!(LudusaviGame::rom_path_of(Some(&meta)).as_deref(), Some("/roms/first.sfc"));
        // Neither present: no ROM, no ROM base.
        meta.roms = None;
        assert_eq!(LudusaviGame::rom_path_of(Some(&meta)), None);
        assert_eq!(LudusaviGame::rom_path_of(None), None);
    }

    #[test]
    fn emulated_game_is_backed_up_under_the_manifest_name() {
        let _l = guard_lock();
        let (_d, paths) = setup(ENABLED);
        let name = "Turbo Goose Racer (RetroArch)";
        let reply = format!(r#"{{"overall":{{"totalGames":1}},"games":{{"{name}":{{}}}}}}"#);
        let runner = Fake {
            replies: vec![(
                format!("backup --force --api --no-cloud-sync --path /backups --format zip --compression zstd {name}"),
                out(&reply, "", 0),
            )],
            ..Default::default()
        };
        json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |f| {
            f.games.insert("g".into(), emulated_meta("emulator", "7d0c2f1e"));
            Ok(())
        })
        .unwrap();
        seed_emulator(&paths, "7d0c2f1e", "retroarch", "RetroArch");
        let probe = FakeProbe::new("/h").with_bin("ludusavi", "/usr/bin/ludusavi");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        // As a session ends: emulated games are sideloaded, the emulator is in the meta.
        let play = AfterPlay { app_name: "g".into(), title: "Turbo Goose Racer".into(), runner: "sideload".into(), elapsed_seconds: 60 };
        let r = backup_after_play(&env, &play).unwrap();
        assert!(r.ran, "{r:?}");
        assert_eq!(r.game_name.as_deref(), Some(name));
        assert_eq!(r.backup_path, Some(join("/backups", name)));
        let calls = runner.calls.lock().unwrap();
        let backups: Vec<_> = calls.iter().filter(|c| c[0] == "backup").collect();
        assert_eq!(backups.len(), 2, "{calls:?}");
        assert_eq!(backups[0].last().unwrap(), "Turbo Goose Racer");
        assert_eq!(backups[1].last().unwrap(), name);
    }

    #[test]
    fn a_tagged_title_falls_back_to_the_stripped_manifest_name() {
        let _l = guard_lock();
        let (_d, paths) = setup(ENABLED);
        let name = "Turbo Goose Racer (RetroArch)";
        let reply = format!(r#"{{"overall":{{"totalGames":1}},"games":{{"{name}":{{}}}}}}"#);
        let runner = Fake {
            replies: vec![(
                format!("backup --force --api --no-cloud-sync --path /backups --format zip --compression zstd {name}"),
                out(&reply, "", 0),
            )],
            ..Default::default()
        };
        json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |f| {
            f.games.insert("g".into(), emulated_meta("emulator", "7d0c2f1e"));
            Ok(())
        })
        .unwrap();
        seed_emulator(&paths, "7d0c2f1e", "retroarch", "RetroArch");
        let probe = FakeProbe::new("/h").with_bin("ludusavi", "/usr/bin/ludusavi");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        // The title kept a tag the generator strips; the manifest key does not
        // have it. Bare title, then as-is with the platform, then the
        // tag-stripped name: three attempts, in that order.
        let play =
            AfterPlay { app_name: "g".into(), title: "Turbo Goose Racer (NA)".into(), runner: "sideload".into(), elapsed_seconds: 60 };
        let r = backup_after_play(&env, &play).unwrap();
        assert!(r.ran, "{r:?}");
        assert_eq!(r.game_name.as_deref(), Some(name));
        assert_eq!(r.backup_path, Some(join("/backups", name)));
        let calls = runner.calls.lock().unwrap();
        let backups: Vec<_> = calls.iter().filter(|c| c[0] == "backup").collect();
        assert_eq!(backups.len(), 3, "{calls:?}");
        assert_eq!(backups[0].last().unwrap(), "Turbo Goose Racer (NA)");
        assert_eq!(backups[1].last().unwrap(), "Turbo Goose Racer (NA) (RetroArch)");
        assert_eq!(backups[2].last().unwrap(), name);
    }

    #[test]
    fn a_renamed_title_falls_back_to_the_rom_name() {
        let _l = guard_lock();
        let (_d, paths) = setup(ENABLED);
        let name = "Great Adventure (RetroArch)";
        let reply = format!(r#"{{"overall":{{"totalGames":1}},"games":{{"{name}":{{}}}}}}"#);
        let runner = Fake {
            replies: vec![(
                format!("backup --force --api --no-cloud-sync --path /backups --format zip --compression zstd {name}"),
                out(&reply, "", 0),
            )],
            ..Default::default()
        };
        json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |f| {
            let mut meta = emulated_meta("emulator", "7d0c2f1e");
            meta.selected_rom_path = Some("/roms/Great Adventure (NA).sfc".into());
            f.games.insert("g".into(), meta);
            Ok(())
        })
        .unwrap();
        seed_emulator(&paths, "7d0c2f1e", "retroarch", "RetroArch");
        let probe = FakeProbe::new("/h").with_bin("ludusavi", "/usr/bin/ludusavi");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        // The Collection title was renamed; only the ROM still names the game
        // the way the manifest does.
        let play = AfterPlay { app_name: "g".into(), title: "MK3".into(), runner: "sideload".into(), elapsed_seconds: 60 };
        let r = backup_after_play(&env, &play).unwrap();
        assert!(r.ran, "{r:?}");
        assert_eq!(r.game_name.as_deref(), Some(name));
        let calls = runner.calls.lock().unwrap();
        let backups: Vec<_> = calls.iter().filter(|c| c[0] == "backup").collect();
        assert_eq!(backups.len(), 3, "{calls:?}");
        assert_eq!(backups[0].last().unwrap(), "MK3");
        assert_eq!(backups[1].last().unwrap(), "MK3 (RetroArch)");
        assert_eq!(backups[2].last().unwrap(), name);
    }

    #[test]
    fn a_pc_game_is_looked_up_by_title_only() {
        let _l = guard_lock();
        let (_d, paths) = setup(ENABLED);
        let name = "Turbo Goose Racer";
        let reply = format!(r#"{{"overall":{{"totalGames":1}},"games":{{"{name}":{{}}}}}}"#);
        let runner = Fake {
            replies: vec![(
                format!("backup --force --api --no-cloud-sync --path /backups --format zip --compression zstd {name}"),
                out(&reply, "", 0),
            )],
            ..Default::default()
        };
        // A PC game whose meta happens to carry a ROM path: no emulator, so no
        // platform-suffixed names and no ROM base are ever tried.
        json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |f| {
            let meta = LocalGameMeta {
                app_name: "g".into(),
                runner: "sideload".into(),
                launch_kind: "executable".into(),
                selected_rom_path: Some("/roms/stray.sfc".into()),
                ..Default::default()
            };
            f.games.insert("g".into(), meta);
            Ok(())
        })
        .unwrap();
        let probe = FakeProbe::new("/h").with_bin("ludusavi", "/usr/bin/ludusavi");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        let play = AfterPlay { app_name: "g".into(), title: name.into(), runner: "sideload".into(), elapsed_seconds: 60 };
        let r = backup_after_play(&env, &play).unwrap();
        assert!(r.ran, "{r:?}");
        assert_eq!(r.game_name.as_deref(), Some(name));
        let calls = runner.calls.lock().unwrap();
        let backups: Vec<_> = calls.iter().filter(|c| c[0] == "backup").collect();
        assert_eq!(backups.len(), 1, "{calls:?}");
        assert_eq!(backups[0].last().unwrap(), name);
    }

    #[test]
    fn a_platform_without_saves_falls_through_to_the_next_one() {
        let _l = guard_lock();
        let (_d, paths) = setup(ENABLED);
        let wii = "Moon Cheese Caper (Wii)";
        let reply = format!(r#"{{"overall":{{"totalGames":1}},"games":{{"{wii}":{{}}}}}}"#);
        let runner = Fake {
            replies: vec![(
                format!("backup --force --api --no-cloud-sync --path /backups --format zip --compression zstd {wii}"),
                out(&reply, "", 0),
            )],
            ..Default::default()
        };
        let probe = FakeProbe::new("/h").with_bin("ludusavi", "/usr/bin/ludusavi");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        let game = LudusaviGame {
            app_name: "g".into(),
            title: "Moon Cheese Caper".into(),
            runner: "sideload".into(),
            emulator_id: Some("a91b44c3".into()),
            ..Default::default()
        };
        seed_emulator(&paths, "a91b44c3", "custom", "Dolphin");
        let r = backup_for_game(&env, &game, BackupReason::Auto);
        assert!(r.ran, "{r:?}");
        assert_eq!(r.game_name.as_deref(), Some(wii));
        let calls = runner.calls.lock().unwrap();
        let backups: Vec<_> = calls.iter().filter(|c| c[0] == "backup").collect();
        assert_eq!(backups.len(), 3, "{calls:?}");
        assert_eq!(backups[0].last().unwrap(), "Moon Cheese Caper");
        assert_eq!(backups[2].last().unwrap(), wii);
    }

    #[test]
    fn the_naming_carries_the_emulators_entry_trimmed() {
        let (_d, paths) = setup(ENABLED);
        seed_emulator_with_entry(&paths, "e1", "custom", "Eden", Some("  Eden Saves "));
        seed_emulator_with_entry(&paths, "e2", "retroarch", "RetroArch", Some("   "));
        assert_eq!(emulator_naming(&paths, Some("e1")).entry.as_deref(), Some("Eden Saves"));
        assert_eq!(emulator_naming(&paths, Some("e2")), EmulatorNaming { engine: Some("retroarch".into()), entry: None });
    }

    #[test]
    fn an_emulators_shared_entry_is_backed_up_whatever_the_game() {
        let _l = guard_lock();
        let (_d, paths) = setup(ENABLED);
        let entry = "RetroArch Saves";
        let reply = format!(r#"{{"overall":{{"totalGames":1}},"games":{{"{entry}":{{}}}}}}"#);
        let runner = Fake {
            replies: vec![(
                format!("backup --force --api --no-cloud-sync --path /backups --format zip --compression zstd {entry}"),
                out(&reply, "", 0),
            )],
            ..Default::default()
        };
        json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |f| {
            f.games.insert("g".into(), emulated_meta("emulator", "7d0c2f1e"));
            Ok(())
        })
        .unwrap();
        seed_emulator_with_entry(&paths, "7d0c2f1e", "retroarch", "RetroArch", Some(entry));
        let probe = FakeProbe::new("/h").with_bin("ludusavi", "/usr/bin/ludusavi");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        let play = AfterPlay { app_name: "g".into(), title: "Turbo Goose Racer".into(), runner: "sideload".into(), elapsed_seconds: 60 };
        let r = backup_after_play(&env, &play).unwrap();
        assert!(r.ran, "{r:?}");
        assert_eq!(r.game_name.as_deref(), Some(entry));
        assert_eq!(r.backup_path, Some(join("/backups", entry)));
        // One run, under the entry: neither the title nor the manifest names.
        let calls = runner.calls.lock().unwrap();
        let backups: Vec<_> = calls.iter().filter(|c| c[0] == "backup").collect();
        assert_eq!(backups.len(), 1, "{calls:?}");
        assert_eq!(backups[0].last().unwrap(), entry);
    }

    #[test]
    fn a_missing_shared_entry_is_reported_by_its_name() {
        let _l = guard_lock();
        let (_d, paths) = setup(ENABLED);
        let runner = Fake { replies: vec![("backup".into(), out("{}", "", 1))], ..Default::default() };
        seed_emulator_with_entry(&paths, "e1", "custom", "Eden", Some("Eden Saves"));
        let probe = FakeProbe::new("/h").with_bin("ludusavi", "/usr/bin/ludusavi");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        let game = LudusaviGame {
            app_name: "g".into(),
            title: "Moon Cheese Caper".into(),
            runner: "sideload".into(),
            emulator_id: Some("e1".into()),
            ..Default::default()
        };
        let r = backup_for_game(&env, &game, BackupReason::Manual);
        assert!(!r.ran);
        assert_eq!(r.skipped_reason.as_deref(), Some("no-match"));
        assert_eq!(r.error.as_deref(), Some("Ludusavi did not find saves for Eden Saves"));
        assert_eq!(runner.calls.lock().unwrap().iter().filter(|c| c[0] == "backup").count(), 1);
    }
}
