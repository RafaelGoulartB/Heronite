//! GOG games. Windows builds run their primary play task (from the
//! `goggame-<id>.info` file GOG puts in the install folder) through
//! Wine/Proton; Linux builds start the `start.sh` of their folder.

use heronite_data::models::GameInfo;
use heronite_i18n::tr;
use serde_json::Value;

use crate::env::join;
use crate::legendary::{cached_umu_id, known_fixes_env, known_fixes_steps};
use crate::plan::{native_plan, split_args, CloudSaves, FirstLaunch, LaunchOptions, LaunchPlan, PlanContext, WineRuntime};
use crate::probe::Probe;
use crate::settings::GameSettings;
use crate::wine;

/// The task a GOG game starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayTask {
    /// Relative to the install folder, `/`-separated.
    pub path: String,
    pub arguments: String,
    /// Relative to the install folder; `None` = the install folder.
    pub working_dir: Option<String>,
}

/// `<install>/goggame-<id>.info`.
pub fn info_file(install_path: &str, app_name: &str) -> String {
    join(install_path, &format!("goggame-{app_name}.info"))
}

/// The primary file task of a `goggame-<id>.info` (else the first file task
/// of the `game` category).
pub fn primary_task(info_text: &str) -> Option<PlayTask> {
    let doc: Value = serde_json::from_str(info_text).ok()?;
    let tasks = doc.get("playTasks")?.as_array()?;
    let is_file = |t: &&Value| t.get("type").and_then(Value::as_str).is_none_or(|k| k == "FileTask") && t.get("path").is_some();
    let task = tasks
        .iter()
        .filter(is_file)
        .find(|t| t.get("isPrimary").and_then(Value::as_bool) == Some(true))
        .or_else(|| tasks.iter().filter(is_file).find(|t| t.get("category").and_then(Value::as_str) == Some("game")))?;
    let text = |k: &str| task.get(k).and_then(Value::as_str).map(|s| s.replace('\\', "/"));
    Some(PlayTask {
        path: text("path").filter(|p| !p.is_empty())?,
        arguments: text("arguments").unwrap_or_default(),
        working_dir: text("workingDir").filter(|d| !d.is_empty()),
    })
}

/// `rel` under `base`; when that exact path is missing, the same path
/// matched case-insensitively on disk (GOG's tasks and the files often
/// differ in case).
fn resolve(probe: &dyn Probe, base: &str, rel: &str) -> String {
    let exact = join(base, rel);
    if probe.exists(&exact) {
        return exact;
    }
    let mut dir = std::path::PathBuf::from(base);
    for part in rel.split('/').filter(|p| !p.is_empty()) {
        let found = std::fs::read_dir(&dir)
            .ok()
            .and_then(|entries| entries.flatten().map(|e| e.file_name()).find(|n| n.to_string_lossy().eq_ignore_ascii_case(part)));
        match found {
            Some(name) => dir.push(name),
            None => return exact,
        }
    }
    dir.to_string_lossy().into_owned()
}

/// The Linux build's platform name in `installed.json`.
fn is_linux(game: &GameInfo) -> bool {
    game.install.platform.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("linux"))
}

pub fn build(ctx: &PlanContext, probe: &dyn Probe, game: &GameInfo, settings: &GameSettings, opts: &LaunchOptions) -> LaunchPlan {
    let app = game.app_name.as_str();
    if !game.is_installed {
        return LaunchPlan::Unavailable(tr!("This GOG game is not installed").into());
    }
    let install_path = game.install.install_path.as_deref().unwrap_or("");
    if install_path.is_empty() || !probe.exists(install_path) {
        return LaunchPlan::Unavailable(tr!("Install folder not found: {install_path}", install_path));
    }
    let mut extra = split_args(&settings.launcher_args);
    extra.extend(opts.extra_args.iter().cloned());
    let override_exe =
        opts.alt_exe.clone().filter(|e| !e.is_empty()).or_else(|| Some(settings.target_exe.clone()).filter(|e| !e.is_empty()));

    if is_linux(game) {
        let exe = override_exe.unwrap_or_else(|| join(install_path, "start.sh"));
        return native_plan(ctx, probe, game, settings, &exe, extra, "gog");
    }

    let (exe, mut args, cwd) = match override_exe {
        Some(exe) => {
            let cwd = crate::emulators::node_dirname(&exe);
            (exe, Vec::new(), cwd)
        }
        None => {
            let info = info_file(install_path, app);
            let Some(task) = probe.read_head(&info, 4 << 20).and_then(|b| primary_task(&String::from_utf8_lossy(&b))) else {
                return LaunchPlan::Unavailable(tr!("The game's launch information was not found: {path}", path = info));
            };
            let exe = resolve(probe, install_path, &task.path);
            let cwd = task.working_dir.as_deref().map(|d| resolve(probe, install_path, d)).unwrap_or_else(|| install_path.to_string());
            (exe, split_args(&task.arguments), cwd)
        }
    };
    args.extend(extra);

    let mut plan = wine::build_wine_plan(ctx, probe, game, settings, &exe, &args);
    if let LaunchPlan::Wine { process, runtime, .. } = &mut plan {
        process.command.cwd = Some(cwd);
        process.command.env.extend(known_fixes_env(ctx, "gog", app));
        if *runtime == WineRuntime::Umu {
            if let Some(id) = cached_umu_id(ctx, "gog", app) {
                process.command.env.insert("GAMEID".into(), id);
            }
        }
        // GOG keeps the saves itself (its own locations): synced around the
        // launch when `autoSyncSaves` is on.
        if settings.auto_sync_saves {
            process.prep.cloud_saves = Some(CloudSaves::new("gog", ""));
        }
        // Redistributables, the game's install scripts and known fixes: once per game and prefix.
        let (winetricks, run_in_prefix) = known_fixes_steps(ctx, "gog", app);
        process.prep.first_launch = Some(FirstLaunch {
            runner: "gog".into(),
            app_name: app.to_string(),
            install_path: install_path.to_string(),
            platform: game.install.platform.clone().unwrap_or_default(),
            winetricks,
            run_in_prefix,
            ..Default::default()
        });
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO: &str = r#"{
        "gameId": "1207658924", "name": "Game",
        "playTasks": [
            {"category": "document", "type": "FileTask", "path": "manual.pdf", "name": "Manual"},
            {"category": "game", "isPrimary": true, "type": "FileTask", "path": "Bin\\Game.exe",
             "arguments": "-nolauncher -data \"C:\\x\"", "workingDir": "Bin", "name": "Game"},
            {"category": "launcher", "type": "URLTask", "link": "https://gog.com"}
        ]}"#;

    #[test]
    fn the_primary_task_is_picked() {
        let t = primary_task(INFO).unwrap();
        assert_eq!(t.path, "Bin/Game.exe");
        assert_eq!(t.working_dir.as_deref(), Some("Bin"));
        assert_eq!(split_args(&t.arguments), ["-nolauncher", "-data", "C:/x"]);
    }

    #[test]
    fn without_a_primary_task_the_game_task_is_used() {
        let info = r#"{"playTasks": [{"category": "document", "path": "a.pdf"}, {"category": "game", "path": "Game.exe"}]}"#;
        let t = primary_task(info).unwrap();
        assert_eq!((t.path.as_str(), t.working_dir), ("Game.exe", None));
        assert!(primary_task(r#"{"playTasks": []}"#).is_none());
        assert!(primary_task("not json").is_none());
    }

    #[test]
    fn paths_resolve_case_insensitively_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("BIN")).unwrap();
        std::fs::write(dir.path().join("BIN/game.EXE"), "").unwrap();
        let base = dir.path().to_str().unwrap();
        let probe = crate::probe::SystemProbe;
        assert_eq!(resolve(&probe, base, "Bin/Game.exe"), join(base, "BIN/game.EXE"));
        assert_eq!(resolve(&probe, base, "Other/Game.exe"), join(base, "Other/Game.exe"));
    }
}
