//! The backup flow: one game, or after a play session.

use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use heronite_data::models::LocalMetaFile;
use heronite_data::{json_store, settings, DataPaths};
use heronite_i18n::tr;
use heronite_launch::probe::Probe;
use heronite_launch::settings::{load_game_settings, GlobalSettings};
use serde::{Deserialize, Serialize};

use super::commands::{
    backup_command, best_find_match, find_commands, game_names, parse_json, total_games, wine_prefix_for_ludusavi, BackupCommand,
};
use super::detect::detect_ludusavi;
use super::runner::{CommandOutput, CommandRunner};
use crate::iso::iso_now;
use crate::pathutil::join;
use crate::settings_ext::{record_ludusavi_backup, LudusaviRecord};

pub const SKIP_SESSION_SECONDS: u64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupReason {
    Auto,
    Manual,
}

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
/// settings. `None` for Steam-launched games: their prefix is Steam's.
pub fn game_wine_prefix(env: &LudusaviEnv, app_name: &str) -> Option<String> {
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

/// `backupLudusaviForGame`.
pub fn backup_for_game(env: &LudusaviEnv, game: &LudusaviGame, reason: BackupReason) -> LudusaviBackupResult {
    log::info!("Ludusavi {reason:?} backup requested for {}", game.title);
    let Some(_guard) = InFlight::acquire() else {
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
    if reason == BackupReason::Auto && !cfg.enabled {
        return Ok(LudusaviBackupResult { skipped_reason: Some("disabled".into()), ..Default::default() });
    }
    if reason == BackupReason::Auto && !cfg.auto_backup_for(&game.runner, &game.app_name) {
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

    let mut game_name = game.title.clone();
    let mut result = run_backup(&game_name)?;
    let mut payload = parse_json(&result.stdout);
    let mut total = total_games(&payload);

    if result.code != 0 && total == 0 && (game.steam_app_id.as_deref().is_some_and(|s| !s.is_empty()) || game.runner == "gog") {
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
        let recorded = if stderr.is_empty() { format!("Ludusavi did not find saves for {}", game.title) } else { stderr.clone() };
        let error = if stderr.is_empty() { tr!("Ludusavi did not find saves for {title}", title = game.title) } else { stderr };
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
    let metas: LocalMetaFile = json_store::read(&env.paths.local_meta()).unwrap_or_default();
    let meta = metas.games.get(&play.app_name);
    let game = LudusaviGame {
        app_name: play.app_name.clone(),
        title: play.title.clone(),
        runner: play.runner.clone(),
        steam_app_id: meta.and_then(|m| m.steam_app_id.clone()),
        store_game_id: meta.and_then(|m| m.store_game_id.clone()),
    };
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
        assert_eq!(r.backup_path.as_deref(), Some("/backups/Tony Hawk's Underground"));
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
        assert_eq!(l.last_backup_path.as_deref(), Some("/backups/Tony Hawk's Underground"));
        assert!(l.last_backup_at.is_some());
        assert_eq!(l.last_error, None);
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
        assert_eq!(body, "Saved G to /backups/G");
        assert!(notification(&LudusaviBackupResult::default(), "G").is_none());
    }
}
