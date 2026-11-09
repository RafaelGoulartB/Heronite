//! Restoring a Ludusavi backup into the game's prefix as it is now (Linux),
//! or into the game's own folders (Windows, where games run natively).
//!
//! `ludusavi restore` writes each file back to the absolute path it was
//! backed up from and has no `--wine-prefix`. A backup made in another
//! prefix (another Steam library, a recreated prefix, Windows) is retargeted
//! with restore redirects written to a throwaway config folder, so Ludusavi's
//! own config is never touched. On Windows only what moved is redirected
//! (another Steam library, another Windows user, a backup made on Linux).
//! Files left out go to that config's `restore.toggledPaths`.
//!
//! Flow: [`list_backups`] → [`plan_restore`] (a preview without redirects to
//! read the stored paths, then one with them) → [`apply_restore`] (undo copy,
//! then the real restore) → [`undo::undo`] if the user changes their mind.

mod config;
mod list;
mod manage;
mod plan;
mod target;
pub mod undo;

use std::path::PathBuf;

use heronite_data::models::LudusaviSettings;
use heronite_data::settings;
use heronite_i18n::tr;

pub use list::{backup_size, list_backups, parse_backups, restore_sources, BackupEntry, BackupSource, GameBackups, SourceKind};
pub use manage::{delete_backup, set_name};
pub use plan::{classify, is_windows_path, redirects, Change, FileKind, PlannedFile, Redirect};
pub use target::{steam_game_running, wine_user, RestoreTarget};

use super::backup::InFlight;
use super::detect::{detect_ludusavi, LudusaviDetected};
use super::{parse_json, CommandOutput, LudusaviEnv};
use config::TempConfig;
use plan::{parse_reply, planned_files};
use undo::UndoRecord;

/// The Ludusavi binary and the settings a restore runs with.
#[derive(Debug, Clone)]
pub struct Tool {
    pub binary: String,
    pub detected: Option<LudusaviDetected>,
    pub settings: LudusaviSettings,
}

/// Why there is no [`Tool`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolError {
    /// Ludusavi is not installed (or not where Heronite looks).
    NotFound,
    /// The settings could not be read.
    Settings(String),
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolError::NotFound => f.write_str(tr!("Ludusavi was not found")),
            ToolError::Settings(e) => f.write_str(e),
        }
    }
}

impl From<ToolError> for String {
    fn from(e: ToolError) -> Self {
        e.to_string()
    }
}

impl Tool {
    /// Like the backup: the binary set in Settings, else the one found.
    pub fn resolve(env: &LudusaviEnv) -> Result<Tool, ToolError> {
        let settings = settings::load(env.paths).map_err(|e| ToolError::Settings(format!("{e:#}")))?.ludusavi;
        let preferred = Some(settings.binary_path.trim()).filter(|p| !p.is_empty());
        let detected = detect_ludusavi(env.runner, env.probe, preferred);
        let binary = preferred.map(str::to_string).or_else(|| detected.as_ref().map(|d| d.binary.clone()));
        match binary.filter(|b| env.probe.exists(b)) {
            Some(binary) => Ok(Tool { binary, detected, settings }),
            None => Err(ToolError::NotFound),
        }
    }
}

/// What to restore and where.
#[derive(Debug, Clone)]
pub struct RestoreRequest {
    pub source_root: PathBuf,
    /// Ludusavi's name of the game in that folder.
    pub game_name: String,
    pub backup_id: String,
    pub target: RestoreTarget,
}

/// A previewed restore. Toggle `files[i].selected`, then [`apply_restore`].
#[derive(Debug, Clone)]
pub struct RestorePlan {
    pub request: RestoreRequest,
    pub redirects: Vec<Redirect>,
    pub files: Vec<PlannedFile>,
}

impl RestorePlan {
    pub fn selected(&self) -> impl Iterator<Item = &PlannedFile> {
        self.files.iter().filter(|f| f.selected)
    }

    /// Files that would replace different ones on disk.
    pub fn overwrites(&self) -> usize {
        self.selected().filter(|f| f.change == Change::Different).count()
    }
}

/// How a restore went.
#[derive(Debug, Clone)]
pub struct RestoreOutcome {
    /// Files written (new or replaced).
    pub written: usize,
    /// Already identical, left alone.
    pub unchanged: usize,
    /// `(path, error)` of files Ludusavi could not write.
    pub failed: Vec<(String, String)>,
    /// The copy to undo it with (`None` when nothing was written).
    pub undo: Option<UndoRecord>,
}

fn restore_args(cfg: &TempConfig, req: &RestoreRequest, preview: bool) -> Vec<String> {
    let mut args = cfg.args();
    args.extend(["restore".into(), "--api".into(), "--no-cloud-sync".into()]);
    args.push(if preview { "--preview".into() } else { "--force".into() });
    args.extend(["--path".into(), req.source_root.to_string_lossy().into_owned()]);
    args.extend(["--backup".into(), req.backup_id.clone(), req.game_name.clone()]);
    args
}

fn reply_error(out: &CommandOutput, code: &str, req: &RestoreRequest) -> String {
    let stderr = out.stderr.trim();
    if code == "unknown-game" {
        tr!("Ludusavi has no backup of {game} in {folder}", game = req.game_name, folder = req.source_root.display().to_string())
    } else if !stderr.is_empty() {
        stderr.to_string()
    } else {
        tr!("Ludusavi did not answer as expected").into()
    }
}

fn preview(env: &LudusaviEnv, tool: &Tool, req: &RestoreRequest, redirects: &[Redirect]) -> Result<Vec<plan::ReplyFile>, String> {
    let cfg = TempConfig::write(&req.game_name, redirects, &[]).map_err(|e| e.to_string())?;
    let out = env.runner.run(&tool.binary, &restore_args(&cfg, req, true))?;
    parse_reply(&parse_json(&out.stdout), &req.game_name).map_err(|code| reply_error(&out, &code, req))
}

/// Previews the restore: where each file of the backup lands in `target`.
pub fn plan_restore(env: &LudusaviEnv, tool: &Tool, request: RestoreRequest) -> Result<RestorePlan, String> {
    if request.target.missing_prefix() {
        return Err(tr!("The game's prefix does not exist yet. Start the game once, then restore.").into());
    }
    let stored = preview(env, tool, &request, &[])?;
    let originals: Vec<String> = stored.iter().map(|f| f.original.clone()).collect();
    let redirects = redirects(&originals, &request.target);
    let reply = if redirects.is_empty() { stored } else { preview(env, tool, &request, &redirects)? };
    let files = planned_files(reply, &request.target);
    log::info!("Ludusavi restore plan for {} ({}): {} files, redirects {:?}", request.game_name, request.backup_id, files.len(), redirects);
    Ok(RestorePlan { request, redirects, files })
}

/// Restores the selected files of `plan`, after copying what they replace
/// to the undo folder (`undo_root`, under `app_name`).
pub fn apply_restore(
    env: &LudusaviEnv,
    tool: &Tool,
    plan: &RestorePlan,
    undo_root: &std::path::Path,
    app_name: &str,
) -> Result<RestoreOutcome, String> {
    let Some(_guard) = InFlight::acquire() else {
        return Err(tr!("Ludusavi is busy with another backup or restore. Try again in a moment.").into());
    };
    let req = &plan.request;
    let selected: Vec<String> = plan.selected().map(|f| f.target.clone()).collect();
    if selected.is_empty() {
        return Err(tr!("Choose at least one file to restore.").into());
    }
    let record = undo::snapshot(undo_root, app_name, &req.game_name, &req.backup_id, &selected).map_err(|e| format!("{e:#}"))?;
    let skipped: Vec<&str> = plan.files.iter().filter(|f| !f.selected).map(|f| f.target.as_str()).collect();
    let cfg = match TempConfig::write(&req.game_name, &plan.redirects, &skipped) {
        Ok(cfg) => cfg,
        Err(e) => {
            undo::discard(&record);
            return Err(e.to_string());
        }
    };
    let args = restore_args(&cfg, req, false);
    log::info!("Running {} {}", tool.binary, args.join(" "));
    let reply = env
        .runner
        .run(&tool.binary, &args)
        .and_then(|out| parse_reply(&parse_json(&out.stdout), &req.game_name).map_err(|code| reply_error(&out, &code, req)));
    let reply = match reply {
        Ok(reply) => reply,
        // Ludusavi ran (or was stopped part-way through): files may have
        // changed, so the copy stays for the undo.
        Err(e) => {
            log::warn!("Ludusavi restore failed after it started; keeping the undo copy {}", record.dir.display());
            return Err(format!("{e}\n{}", tr!("A copy of what the restore may have replaced is kept: Undo puts it back.")));
        }
    };
    let mut outcome = RestoreOutcome { written: 0, unchanged: 0, failed: Vec::new(), undo: None };
    for f in reply.into_iter().filter(|f| !f.ignored) {
        if f.failed {
            outcome.failed.push((f.target, f.error.unwrap_or_default()));
        } else if f.change == Change::Same {
            outcome.unchanged += 1;
        } else {
            outcome.written += 1;
        }
    }
    if outcome.written > 0 {
        outcome.undo = Some(record);
    } else {
        undo::discard(&record);
    }
    Ok(outcome)
}

fn edit_args(root: &std::path::Path, game: &str, backup_id: &str, locked: bool) -> Vec<String> {
    let mut args: Vec<String> = ["--no-manifest-update", "backups", "edit", "--path"].iter().map(|s| s.to_string()).collect();
    args.push(root.to_string_lossy().into_owned());
    args.extend(["--backup".into(), backup_id.into(), if locked { "--lock".into() } else { "--unlock".into() }, game.into()]);
    args
}

/// Locks a backup (the retention never removes it) or unlocks it.
pub fn set_locked(env: &LudusaviEnv, tool: &Tool, root: &std::path::Path, game: &str, backup_id: &str, locked: bool) -> Result<(), String> {
    // Ludusavi rewrites the game's mapping.yaml, like a backup does.
    let Some(_guard) = InFlight::acquire() else {
        return Err(tr!("Ludusavi is busy with another backup or restore. Try again in a moment.").into());
    };
    let out = env.runner.run(&tool.binary, &edit_args(root, game, backup_id, locked))?;
    if out.code == 0 {
        Ok(())
    } else {
        Err(Some(out.stderr.trim().to_string())
            .filter(|e| !e.is_empty())
            .unwrap_or_else(|| tr!("Ludusavi could not change the backup").into()))
    }
}

/// Downloads Ludusavi's newest list of games and save locations
/// (`manifest update --force`).
pub fn update_manifest(env: &LudusaviEnv, tool: &Tool) -> Result<(), String> {
    let out = env.runner.run(&tool.binary, &["manifest".into(), "update".into(), "--force".into()])?;
    if out.code == 0 {
        Ok(())
    } else {
        Err(Some(out.stderr.trim().to_string())
            .filter(|e| !e.is_empty())
            .unwrap_or_else(|| tr!("Ludusavi could not update its game list").into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ludusavi::fake::{out, Fake};
    use heronite_data::DataPaths;
    use heronite_launch::probe::FakeProbe;

    /// The in-flight guard is shared with the backup tests running alongside.
    fn apply(env: &LudusaviEnv, plan: &RestorePlan, root: &std::path::Path) -> Result<RestoreOutcome, String> {
        let _l = super::super::backup::test_lock();
        apply_restore(env, &tool(), plan, root, "app")
    }

    fn tool() -> Tool {
        Tool { binary: "/bin/ludusavi".into(), detected: None, settings: Default::default() }
    }

    const PFX: &str = "/lib/steamapps/compatdata/1245620/pfx";

    fn request(target: RestoreTarget) -> RestoreRequest {
        RestoreRequest { source_root: "/b".into(), game_name: "Elden Ring".into(), backup_id: "backup-1.zip".into(), target }
    }

    fn steam_target() -> RestoreTarget {
        RestoreTarget { prefix: Some(PFX.into()), user: Some("steamuser".into()), needs_prefix: true, ..Default::default() }
    }

    /// Answers the first preview with the stored paths and the second with
    /// the redirected ones (both start with `--config`, so by call count).
    struct TwoStep {
        calls: std::sync::Mutex<Vec<Vec<String>>>,
        replies: Vec<String>,
    }

    impl crate::ludusavi::CommandRunner for TwoStep {
        fn run(&self, _binary: &str, args: &[String]) -> Result<CommandOutput, String> {
            let mut calls = self.calls.lock().unwrap();
            let i = calls.len().min(self.replies.len() - 1);
            calls.push(args.to_vec());
            Ok(out(&self.replies[i], "", 0))
        }
    }

    // Linux's redirects into a Proton prefix.
    #[cfg(not(windows))]
    #[test]
    fn plans_with_redirects_from_the_stored_paths() {
        let d = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(d.path());
        let stored = serde_json::json!({"games": {"Elden Ring": {"files": {
            "C:/Users/rafael/AppData/Roaming/EldenRing/ER0000.sl2": {"change": "New", "bytes": 10}
        }}}});
        let redirected = serde_json::json!({"games": {"Elden Ring": {"files": {
            format!("{PFX}/drive_c/users/steamuser/AppData/Roaming/EldenRing/ER0000.sl2"):
                {"change": "Different", "bytes": 10, "originalPath": "C:/Users/rafael/AppData/Roaming/EldenRing/ER0000.sl2"}
        }}}});
        let runner = TwoStep { calls: Default::default(), replies: vec![stored.to_string(), redirected.to_string()] };
        let probe = FakeProbe::new("/home/u");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        let plan = plan_restore(&env, &tool(), request(steam_target())).unwrap();
        assert_eq!(plan.redirects.len(), 2);
        assert_eq!(plan.files.len(), 1);
        assert_eq!((plan.files[0].kind, plan.files[0].selected), (FileKind::Save, true));
        assert_eq!(plan.overwrites(), 1);
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        let first = &calls[0];
        assert_eq!(first[0], "--config");
        assert!(first.contains(&"--preview".to_string()));
        assert_eq!(&first[first.len() - 3..], ["--backup", "backup-1.zip", "Elden Ring"]);
    }

    /// Windows: a backup made by another user goes to this user's folder.
    #[cfg(windows)]
    #[test]
    fn windows_plans_with_redirects_to_this_users_folder() {
        let d = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(d.path());
        let stored = serde_json::json!({"games": {"Hollow Knight": {"files": {
            "C:/Users/bob/AppData/LocalLow/Team Cherry/Hollow Knight/user1.dat": {"change": "New", "bytes": 10},
            "/home/bob/.config/unity3d/Team Cherry/Hollow Knight/user1.dat": {"change": "New", "bytes": 10}
        }}}});
        let redirected = serde_json::json!({"games": {"Hollow Knight": {"files": {
            "C:/Users/rafael/AppData/LocalLow/Team Cherry/Hollow Knight/user1.dat":
                {"change": "Different", "bytes": 10, "originalPath": "C:/Users/bob/AppData/LocalLow/Team Cherry/Hollow Knight/user1.dat"},
            "/home/bob/.config/unity3d/Team Cherry/Hollow Knight/user1.dat": {"change": "New", "bytes": 10}
        }}}});
        let runner = TwoStep { calls: Default::default(), replies: vec![stored.to_string(), redirected.to_string()] };
        let probe = FakeProbe::new(r"C:\Users\rafael");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        let target = RestoreTarget { user_home: Some(r"C:\Users\rafael".into()), ..Default::default() };
        let mut req = request(target);
        req.game_name = "Hollow Knight".into();
        let plan = plan_restore(&env, &tool(), req).unwrap();
        assert_eq!(plan.redirects, [Redirect { source: "C:/Users/bob".into(), target: "C:/Users/rafael".into() }]);
        assert_eq!(
            plan.files.iter().map(|f| (f.kind, f.selected)).collect::<Vec<_>>(),
            [(FileKind::Save, true), (FileKind::Unmapped, false)]
        );
        assert_eq!(plan.overwrites(), 1);
        assert_eq!(runner.calls.lock().unwrap().len(), 2);
    }

    /// Windows: the files go back where they were, an undo copy keeps
    /// what they replace, and the undo puts it back.
    #[cfg(windows)]
    #[test]
    fn windows_applies_in_place_and_undoes() {
        let d = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(d.path().join("data"));
        let save = d.path().join(r"Saves\Game\slot1.sav");
        std::fs::create_dir_all(save.parent().unwrap()).unwrap();
        std::fs::write(&save, "before").unwrap();
        // As Ludusavi writes it: `/` separators.
        let save_s = save.to_string_lossy().replace('\\', "/");
        let created = d.path().join(r"Saves\Game\slot2.sav").to_string_lossy().replace('\\', "/");
        let reply = serde_json::json!({"games": {"Elden Ring": {"files": {
            save_s.clone(): {"change": "Different", "bytes": 6},
            created.clone(): {"change": "New", "bytes": 6}
        }}}});
        let runner = Fake { replies: vec![("--config".into(), out(&reply.to_string(), "", 0))], ..Default::default() };
        let probe = FakeProbe::new(r"C:\Users\u");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        let target = RestoreTarget::default();
        let files = planned_files(parse_reply(&reply, "Elden Ring").unwrap(), &target);
        assert!(files.iter().all(|f| f.kind == FileKind::Save && f.selected));
        let plan = RestorePlan { request: request(target), redirects: Vec::new(), files };
        let undo_root = paths.save_restore_undo();
        let outcome = apply(&env, &plan, &undo_root).unwrap();
        assert_eq!((outcome.written, outcome.failed.len()), (2, 0));
        // What Ludusavi would have written.
        std::fs::write(&save, "restored").unwrap();
        std::fs::write(&created, "restored").unwrap();
        let record = undo::latest(&undo_root, "app").expect("an undo copy");
        assert_eq!(record, outcome.undo.unwrap());
        assert_eq!(undo::undo(&record).unwrap(), 2);
        assert_eq!(std::fs::read_to_string(&save).unwrap(), "before");
        assert!(!std::path::Path::new(&created).exists());
    }

    #[test]
    fn refuses_a_missing_prefix_and_reports_unknown_games() {
        let d = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(d.path());
        let runner = Fake {
            replies: vec![("--config".into(), out(r#"{"errors":{"unknownGames":["Elden Ring"]},"games":{}}"#, "", 1))],
            ..Default::default()
        };
        let probe = FakeProbe::new("/home/u");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        let missing = RestoreTarget { needs_prefix: true, ..Default::default() };
        assert!(plan_restore(&env, &tool(), request(missing)).is_err());
        assert!(runner.calls.lock().unwrap().is_empty());
        let err = plan_restore(&env, &tool(), request(steam_target())).unwrap_err();
        assert!(err.contains("Elden Ring"), "{err}");
    }

    // Wine prefixes and Linux paths (`windows_applies_in_place_and_undoes` above).
    #[cfg(unix)]
    #[test]
    fn applies_only_the_selected_files_and_keeps_an_undo_copy() {
        let d = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(d.path());
        let save = d.path().join("pfx/drive_c/users/steamuser/save.sav");
        std::fs::create_dir_all(save.parent().unwrap()).unwrap();
        std::fs::write(&save, "before").unwrap();
        let save_s = save.to_string_lossy().into_owned();
        let reg = d.path().join("pfx/user.reg").to_string_lossy().into_owned();
        let reply = serde_json::json!({"games": {"Elden Ring": {"files": {
            save_s.clone(): {"change": "Different", "bytes": 6},
            reg.clone(): {"change": "Different", "bytes": 6, "ignored": true}
        }}}});
        let runner = Fake { replies: vec![("--config".into(), out(&reply.to_string(), "", 0))], ..Default::default() };
        let probe = FakeProbe::new("/home/u");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        let target = RestoreTarget { prefix: Some(d.path().join("pfx")), needs_prefix: true, ..Default::default() };
        let files = planned_files(parse_reply(&reply, "Elden Ring").unwrap(), &target);
        let plan = RestorePlan { request: request(target), redirects: Vec::new(), files };
        let undo_root = d.path().join("undo");
        let outcome = apply(&env, &plan, &undo_root).unwrap();
        assert_eq!((outcome.written, outcome.unchanged, outcome.failed.len()), (1, 0, 0));
        let record = outcome.undo.unwrap();
        assert_eq!(record.entries.len(), 1);
        assert_eq!(std::fs::read_to_string(record.dir.join("files/0")).unwrap(), "before");
        let calls = runner.calls.lock().unwrap();
        let args = &calls[0];
        assert!(args.contains(&"--force".to_string()) && !args.contains(&"--preview".to_string()));
        // The registry file went to the throwaway config as left out.
        drop(calls);

        let none =
            RestorePlan { files: plan.files.iter().cloned().map(|f| PlannedFile { selected: false, ..f }).collect(), ..plan.clone() };
        assert!(apply(&env, &none, &undo_root).unwrap_err().contains("at least one"));
    }

    #[test]
    fn nothing_written_drops_the_undo_copy() {
        let d = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(d.path());
        // A native path of this system: a save on Linux and on Windows.
        let same = d.path().join("s").to_string_lossy().into_owned();
        let reply = serde_json::json!({"games": {"Elden Ring": {"files": {same: {"change": "Same", "bytes": 1}}}}});
        let runner = Fake { replies: vec![("--config".into(), out(&reply.to_string(), "", 0))], ..Default::default() };
        let probe = FakeProbe::new("/home/u");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        let target = RestoreTarget::default();
        let files = planned_files(parse_reply(&reply, "Elden Ring").unwrap(), &target);
        let plan = RestorePlan { request: request(target), redirects: Vec::new(), files };
        let undo_root = d.path().join("undo");
        let outcome = apply(&env, &plan, &undo_root).unwrap();
        assert_eq!((outcome.written, outcome.unchanged), (0, 1));
        assert!(outcome.undo.is_none());
        assert!(undo::latest(&undo_root, "app").is_none());
    }

    #[test]
    fn a_restore_stopped_part_way_keeps_the_undo_copy() {
        let d = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(d.path());
        let save = d.path().join("save.sav");
        std::fs::write(&save, "before").unwrap();
        // No JSON back (killed on timeout, crashed): files may have changed.
        let runner = Fake { replies: vec![("--config".into(), out("", "killed", 1))], ..Default::default() };
        let probe = FakeProbe::new("/home/u");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        let file = PlannedFile {
            target: save.to_string_lossy().into_owned(),
            original: save.to_string_lossy().into_owned(),
            bytes: 6,
            change: Change::Different,
            kind: FileKind::Save,
            selected: true,
        };
        let plan = RestorePlan { request: request(RestoreTarget::default()), redirects: Vec::new(), files: vec![file] };
        let undo_root = d.path().join("undo");
        assert!(apply(&env, &plan, &undo_root).is_err());
        let kept = undo::latest(&undo_root, "app").expect("the undo copy stays");
        assert_eq!(std::fs::read_to_string(kept.dir.join("files/0")).unwrap(), "before");
    }

    #[test]
    fn lock_and_manifest_commands() {
        assert_eq!(
            edit_args(std::path::Path::new("/b"), "G", "backup-1.zip", true),
            ["--no-manifest-update", "backups", "edit", "--path", "/b", "--backup", "backup-1.zip", "--lock", "G"]
        );
        assert!(edit_args(std::path::Path::new("/b"), "G", "x", false).contains(&"--unlock".to_string()));
    }
}
