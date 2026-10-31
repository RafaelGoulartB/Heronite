//! GOG games. Windows builds run their primary play task (from the
//! `goggame-<id>.info` file GOG puts in the install folder): as they are on
//! Windows, through Wine/Proton on Linux. Linux builds start the `start.sh`
//! of their folder. A build this system cannot run is unavailable
//! ([`crate::host::run_mode`]).
//!
//! On Windows the task is started directly rather than through `gogdl
//! launch`: gogdl would only read the same file and start the same program,
//! one process later, and the session follows the game's own process tree
//! (its job object) from the start.

use heronite_data::models::GameInfo;
use heronite_i18n::tr;
use serde_json::Value;

use crate::env::join;
use crate::host::{run_mode, unavailable_reason, Os, RunMode};
use crate::legendary::{cached_umu_id, known_fixes_env, known_fixes_steps};
use crate::plan::{native_plan, split_args, windows_plan, CloudSaves, FirstLaunch, LaunchOptions, LaunchPlan, PlanContext, WineRuntime};
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
    under(install_path, &format!("goggame-{app_name}.info"))
}

/// `rel` (`/`-separated) below the folder `base`: lexically joined on Linux
/// ([`join`]), a Windows path with `\` on Windows (`join` drops drive letters).
fn under(base: &str, rel: &str) -> String {
    if cfg!(windows) {
        let mut path = std::path::PathBuf::from(base);
        path.extend(rel.split(['/', '\\']).filter(|p| !p.is_empty() && *p != "."));
        path.to_string_lossy().into_owned()
    } else {
        join(base, rel)
    }
}

/// Splits a Windows command line the way programs do (`CommandLineToArgvW`):
/// spaces separate arguments, quotes group them, `\"` is a quote and other
/// backslashes stay as they are (paths keep their separators).
pub fn split_windows_args(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let (mut in_word, mut quoted, mut backslashes) = (false, false, 0usize);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                backslashes += 1;
                in_word = true;
                continue;
            }
            '"' => {
                word.extend(std::iter::repeat_n('\\', backslashes / 2));
                if backslashes % 2 == 1 {
                    word.push('"');
                } else if quoted && chars.peek() == Some(&'"') {
                    // `""` inside quotes is a quote.
                    chars.next();
                    word.push('"');
                } else {
                    quoted = !quoted;
                }
                in_word = true;
            }
            c if c.is_whitespace() && !quoted => {
                word.extend(std::iter::repeat_n('\\', backslashes));
                if in_word {
                    out.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            c => {
                word.extend(std::iter::repeat_n('\\', backslashes));
                word.push(c);
                in_word = true;
            }
        }
        backslashes = 0;
    }
    word.extend(std::iter::repeat_n('\\', backslashes));
    if in_word {
        out.push(word);
    }
    out
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
    // Windows reads the arguments as they are written (backslashes in paths);
    // Wine gets them with `/`.
    let arguments = if cfg!(windows) { task.get("arguments").and_then(Value::as_str).map(str::to_string) } else { text("arguments") };
    Some(PlayTask {
        path: text("path").filter(|p| !p.is_empty())?,
        arguments: arguments.unwrap_or_default(),
        working_dir: text("workingDir").filter(|d| !d.is_empty()),
    })
}

/// `rel` under `base`; when that exact path is missing, the same path
/// matched case-insensitively on disk (GOG's tasks and the files often
/// differ in case).
fn resolve(probe: &dyn Probe, base: &str, rel: &str) -> String {
    let exact = under(base, rel);
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

/// The play task of a Windows build: program, arguments and working folder.
fn play_task(probe: &dyn Probe, install_path: &str, app: &str, windows: bool) -> Result<(String, Vec<String>, String), String> {
    let info = info_file(install_path, app);
    let Some(task) = probe.read_head(&info, 4 << 20).and_then(|b| primary_task(&String::from_utf8_lossy(&b))) else {
        return Err(tr!("The game's launch information was not found: {path}", path = info));
    };
    let exe = resolve(probe, install_path, &task.path);
    let cwd = task.working_dir.as_deref().map(|d| resolve(probe, install_path, d)).unwrap_or_else(|| install_path.to_string());
    let args = if windows { split_windows_args(&task.arguments) } else { split_args(&task.arguments) };
    Ok((exe, args, cwd))
}

/// A Windows build on Windows: its play task started as it is, from its
/// working folder, like a game added by hand. Cloud saves around the launch
/// and the store's setup the first time (redistributables, install scripts)
/// still apply; Wine's known fixes do not.
fn native_windows(
    probe: &dyn Probe,
    game: &GameInfo,
    settings: &GameSettings,
    install_path: &str,
    override_exe: Option<String>,
    extra: Vec<String>,
) -> LaunchPlan {
    let app = game.app_name.as_str();
    let (exe, mut args, cwd) = match override_exe {
        Some(exe) => (exe, Vec::new(), None),
        None => match play_task(probe, install_path, app, true) {
            Ok((exe, args, cwd)) => (exe, args, Some(cwd)),
            Err(why) => return LaunchPlan::Unavailable(why),
        },
    };
    args.extend(extra);
    let mut plan = windows_plan(probe, game, settings, &exe, args, "gog");
    if let LaunchPlan::Native(process) = &mut plan {
        if let Some(cwd) = cwd.filter(|c| probe.exists(c)) {
            process.command.cwd = Some(cwd);
        }
        if settings.auto_sync_saves {
            process.prep.cloud_saves = Some(CloudSaves::new("gog", ""));
        }
        process.prep.first_launch = Some(FirstLaunch {
            runner: "gog".into(),
            app_name: app.to_string(),
            install_path: install_path.to_string(),
            platform: game.install.platform.clone().unwrap_or_default(),
            ..Default::default()
        });
    }
    plan
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

    let os = if is_linux(game) { Os::Linux } else { Os::from_name(game.install.platform.as_deref().unwrap_or("")).unwrap_or(Os::Windows) };
    match (run_mode(os), os) {
        (RunMode::Unavailable, _) => return LaunchPlan::Unavailable(unavailable_reason(os)),
        (RunMode::Native, Os::Linux) => {
            let exe = override_exe.unwrap_or_else(|| join(install_path, "start.sh"));
            return native_plan(ctx, probe, game, settings, &exe, extra, "gog");
        }
        (RunMode::Native, _) => return native_windows(probe, game, settings, install_path, override_exe, extra),
        (RunMode::Wine, _) => {}
    }

    let (exe, mut args, cwd) = match override_exe {
        Some(exe) => {
            let cwd = crate::emulators::node_dirname(&exe);
            (exe, Vec::new(), cwd)
        }
        None => match play_task(probe, install_path, app, false) {
            Ok(task) => task,
            Err(why) => return LaunchPlan::Unavailable(why),
        },
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
        if cfg!(windows) {
            assert_eq!(split_windows_args(&t.arguments), ["-nolauncher", "-data", r"C:\x"]);
        } else {
            assert_eq!(split_args(&t.arguments), ["-nolauncher", "-data", "C:/x"]);
        }
    }

    #[test]
    fn windows_command_lines_split_like_windows_programs_do() {
        assert_eq!(split_windows_args(r#"-a "C:\Program Files\G" b"#), ["-a", r"C:\Program Files\G", "b"]);
        assert_eq!(split_windows_args(r#"/path=C:\x\ "q\"uote" "" end\"#), [r"/path=C:\x\", r#"q"uote"#, "", r"end\"]);
        assert_eq!(split_windows_args(r#""a\\" "x""y""#), [r"a\", r#"x"y"#]);
        assert!(split_windows_args("   ").is_empty());
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
        // Windows' own folders do not mind the case.
        let found = if cfg!(windows) { "Bin/Game.exe" } else { "BIN/game.EXE" };
        assert_eq!(resolve(&probe, base, "Bin/Game.exe"), under(base, found));
        assert_eq!(resolve(&probe, base, "Other/Game.exe"), under(base, "Other/Game.exe"));
    }

    #[cfg(windows)]
    mod windows {
        use super::*;
        use crate::plan::build_plan_with_settings;
        use crate::probe::FakeProbe;
        use heronite_data::models::InstallInfo;

        fn game(platform: &str) -> GameInfo {
            GameInfo {
                runner: "gog".into(),
                app_name: "42".into(),
                title: "The Game".into(),
                is_installed: true,
                install: InstallInfo {
                    install_path: Some(r"C:\Games\The Game".into()),
                    platform: Some(platform.into()),
                    ..Default::default()
                },
                ..Default::default()
            }
        }

        fn ctx() -> PlanContext {
            PlanContext::load(&heronite_data::DataPaths::at(r"C:\hr"))
        }

        const TASK: &[u8] = br#"{"playTasks": [{"category": "game", "isPrimary": true, "type": "FileTask", "path": "bin\\game.exe",
            "arguments": "-data \"C:\\Games\\The Game\\data\" -x", "workingDir": "bin"}]}"#;

        fn probe() -> FakeProbe {
            FakeProbe::new(r"C:\Users\u")
                .with_path(r"C:\Games\The Game")
                .with_path(r"C:\Games\The Game\bin")
                .with_path(r"C:\Games\The Game\bin\game.exe")
                .with_file(r"C:\Games\The Game\goggame-42.info", TASK)
        }

        #[test]
        fn windows_builds_run_their_play_task_natively() {
            let settings = GameSettings { launcher_args: "-windowed".into(), ..Default::default() };
            let plan = build_plan_with_settings(&ctx(), &probe(), &game("windows"), None, &settings, &Default::default());
            let LaunchPlan::Native(p) = plan else { panic!("{plan:?}") };
            assert_eq!(p.command.program, r"C:\Games\The Game\bin\game.exe");
            assert_eq!(p.command.args, ["-data", r"C:\Games\The Game\data", "-x", "-windowed"]);
            assert_eq!(p.command.cwd.as_deref(), Some(r"C:\Games\The Game\bin"));
            assert_eq!(p.command.env.get("HEROIC_APP_SOURCE").map(String::as_str), Some("gog"), "{:?}", p.command.env);
            assert_eq!(p.stop.kill_patterns, ["game.exe"]);
            assert_eq!(p.prep.cloud_saves, None);
            assert!(p.prep.wine.is_none());
            let first = p.prep.first_launch.expect("the store setup");
            assert_eq!((first.runner.as_str(), first.app_name.as_str(), first.install_path.as_str()), ("gog", "42", r"C:\Games\The Game"));
            assert!(first.winetricks.is_empty() && first.run_in_prefix.is_empty());

            let sync = GameSettings { auto_sync_saves: true, ..Default::default() };
            let plan = build_plan_with_settings(&ctx(), &probe(), &game("windows"), None, &sync, &Default::default());
            assert_eq!(plan.process().unwrap().prep.cloud_saves, Some(CloudSaves::new("gog", "")));
        }

        #[test]
        fn another_program_can_be_picked_and_missing_files_say_so() {
            let settings = GameSettings { target_exe: r"C:\Games\The Game\tools\editor.exe".into(), ..Default::default() };
            let probe = probe().with_path(r"C:\Games\The Game\tools\editor.exe");
            let plan = build_plan_with_settings(&ctx(), &probe, &game("windows"), None, &settings, &Default::default());
            let p = plan.process().unwrap();
            assert_eq!(p.command.program, r"C:\Games\The Game\tools\editor.exe");
            assert_eq!(p.command.cwd.as_deref(), Some(r"C:\Games\The Game\tools"));

            let bare = FakeProbe::new(r"C:\Users\u").with_path(r"C:\Games\The Game");
            let plan = build_plan_with_settings(&ctx(), &bare, &game("windows"), None, &GameSettings::default(), &Default::default());
            assert!(plan.unavailable_reason().unwrap().contains(r"C:\Games\The Game\goggame-42.info"), "{plan:?}");
        }

        #[test]
        fn linux_and_mac_builds_do_not_run_on_windows() {
            for platform in ["linux", "osx"] {
                let plan = build_plan_with_settings(&ctx(), &probe(), &game(platform), None, &GameSettings::default(), &Default::default());
                assert!(plan.unavailable_reason().is_some(), "{platform}: {plan:?}");
            }
            assert_eq!(info_file(r"C:\Games\G", "7"), r"C:\Games\G\goggame-7.info");
        }
    }
}
