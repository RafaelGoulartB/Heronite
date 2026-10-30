//! Entry points of plan building: sideload games (executables, Steam URIs) and runner dispatch.

use heronite_data::models::{GameInfo, LocalGameMeta};
use heronite_i18n::tr;

use super::emulator::{emulator_plan, EmulatorOutcome};
use super::{Command, LaunchOptions, LaunchPlan, PlanContext, PrepSpec, ProcessPlan, StopSpec};
use crate::emulators::{node_basename, node_dirname};
use crate::env::{build_wrappers_in, setup_env_vars, setup_wrapper_env_vars};
use crate::native::{effective_platform, is_executable_game, Platform};
use crate::probe::Probe;
use crate::settings::{load_game_settings, GameSettings};
use crate::steam;
use heronite_platform::Feature;

/// Builds the plan for a game, reading its `GamesConfig` from disk.
pub fn build_plan(ctx: &PlanContext, probe: &dyn Probe, game: &GameInfo, meta: Option<&LocalGameMeta>, opts: &LaunchOptions) -> LaunchPlan {
    let settings = load_game_settings(&ctx.paths, &game.app_name, &ctx.global, probe);
    build_plan_with_settings(ctx, probe, game, meta, &settings, opts)
}

pub fn build_plan_with_settings(
    ctx: &PlanContext,
    probe: &dyn Probe,
    game: &GameInfo,
    meta: Option<&LocalGameMeta>,
    settings: &GameSettings,
    opts: &LaunchOptions,
) -> LaunchPlan {
    // A store client starts these games, with the client's settings (its
    // sandbox and Wine), not the game's. Their umu id stays `umu-0`.
    if let Some(m) = meta.filter(|m| game.runner == "sideload" && m.launch_kind == heronite_data::clients::LAUNCH_KIND) {
        let mut plan = crate::clients::build(ctx, probe, game, m);
        crate::keep_awake::attach(&mut plan, probe, &game.title, settings);
        crate::scripts::attach(&mut plan, probe, game, settings);
        crate::clients::attach_companions(&mut plan, ctx, probe, game, settings);
        return plan;
    }
    let mut plan = match game.runner.as_str() {
        "sideload" => build_sideload(ctx, probe, game, meta, settings, opts),
        "legendary" => crate::legendary::build(ctx, probe, game, settings, opts),
        "gog" => crate::gog::build(ctx, probe, game, settings, opts),
        "nile" => crate::nile::build(ctx, probe, game, settings, opts),
        other => LaunchPlan::Unavailable(tr!("Launching {other} games is not supported yet", other)),
    };
    // Every way to start a game puts it in the sandbox itself; one that does
    // not (Steam, or a new one that forgot) never runs outside instead. A
    // system without the sandbox ignores the setting (a restored backup).
    let outside = match &plan {
        LaunchPlan::SteamUri { .. } => true,
        p => p.process().is_some_and(|p| !p.sandboxed),
    };
    if settings.sandbox.enabled && heronite_platform::supported(Feature::Sandbox) && outside {
        return LaunchPlan::Unavailable(tr!("This game cannot run in the sandbox. Turn it off for this game.").into());
    }
    // Before the companions: they join the game's container with its GAMEID.
    let keys = crate::umu_ids::GameKeys {
        runner: &game.runner,
        app_name: &game.app_name,
        title: &game.title,
        steam_app_id: meta.and_then(|m| m.steam_app_id.as_deref()),
        user_id: &settings.umu_id,
    };
    crate::umu_ids::attach(&mut plan, ctx, keys);
    // logind's lock, `sh` scripts and programs started alongside are Linux's.
    if heronite_platform::supported(Feature::LaunchExtras) {
        crate::keep_awake::attach(&mut plan, probe, &game.title, settings);
        crate::scripts::attach(&mut plan, probe, game, settings);
        crate::companions::attach(&mut plan, ctx, probe, game, settings);
    }
    plan
}

fn is_browser_game(game: &GameInfo) -> bool {
    game.rest.get("browserUrl").map(|v| !v.is_null() && v != "").unwrap_or(false) || game.install.platform.as_deref() == Some("Browser")
}

fn build_sideload(
    ctx: &PlanContext,
    probe: &dyn Probe,
    game: &GameInfo,
    meta: Option<&LocalGameMeta>,
    settings: &GameSettings,
    opts: &LaunchOptions,
) -> LaunchPlan {
    // 1. Emulators (tryLaunchEmulatedGame).
    if let Some(m) = meta.filter(|m| m.launch_kind == "emulator") {
        if settings.sandbox.enabled {
            return LaunchPlan::Unavailable(tr!("The sandbox does not run emulators. Turn it off for this game.").into());
        }
        return match emulator_plan(ctx, probe, game, m, settings, opts) {
            EmulatorOutcome::Plan(p) => p,
            // `.exe` emulators run through Wine like any Windows program.
            EmulatorOutcome::Wine { executable, args } => crate::wine::build_wine_plan(ctx, probe, game, settings, &executable, &args),
        };
    }

    // Games another launcher (Lutris, Bottles, Faugus) starts.
    if let Some(m) = meta.filter(|m| m.launch_kind == heronite_data::launchers::LAUNCH_KIND) {
        return launcher_plan(probe, game, m);
    }

    // 2. Local executables (tryLaunchNativeLocalGame, else Heroic's launchGame).
    if is_executable_game(game, meta) {
        if is_browser_game(game) {
            return LaunchPlan::Unavailable(tr!("Browser games are not supported").into());
        }
        return executable_plan(ctx, probe, game, meta, settings, opts);
    }

    // 3. Steam URI games.
    if let Some(m) = meta.filter(|m| m.launch_kind == "steam-uri") {
        if let Some(app_id) = m.steam_app_id.as_deref().filter(|s| !s.is_empty()) {
            if steam::find_steam_binary(probe).is_none() {
                return LaunchPlan::Unavailable(tr!("Steam client was not found on PATH").into());
            }
            return LaunchPlan::SteamUri { app_id: app_id.to_string(), uri: steam::steam_uri("rungameid", app_id) };
        }
    }
    LaunchPlan::Unavailable(tr!("This game has no launch method").into())
}

/// Asks the launcher that has the game to start it (`launcherArgs`). The
/// launcher sets the game up (Wine, wrappers); the session follows it, and
/// the game itself when the launcher hands it to its open window.
fn launcher_plan(probe: &dyn Probe, game: &GameInfo, meta: &LocalGameMeta) -> LaunchPlan {
    use heronite_data::launchers::{Launcher, PREFIX_KEY};
    let mut args = split_args(meta.launcher_args.as_deref().unwrap_or_default());
    if args.is_empty() {
        return LaunchPlan::Unavailable(tr!("This game has no launch method").into());
    }
    let program = args.remove(0);
    if probe.which(&program).is_none() && !probe.exists(&program) {
        let launcher = Launcher::from_source(&meta.source).map_or(program.as_str(), |l| l.name());
        return LaunchPlan::Unavailable(tr!("{launcher} was not found on this PC", launcher));
    }
    let folder = game.install.install_path.clone().filter(|f| !f.is_empty() && probe.exists(f));
    let prefix = meta.extra.get(PREFIX_KEY).and_then(serde_json::Value::as_str).filter(|p| !p.is_empty()).map(str::to_owned);
    LaunchPlan::Native(ProcessPlan {
        command: Command { program, args, env: Default::default(), cwd: folder.clone() },
        // Nothing to find the game by: the session ends with the command.
        handoff: (prefix.is_some() || folder.is_some()).then_some(crate::plan::Handoff { prefix, folder, client: None }),
        ..Default::default()
    })
}

fn first_existing<'a>(probe: &dyn Probe, paths: &[Option<&'a str>]) -> Option<&'a str> {
    paths.iter().flatten().copied().find(|p| !p.is_empty() && probe.exists(p))
}

pub(crate) fn split_args(s: &str) -> Vec<String> {
    shlex::split(s).unwrap_or_else(|| s.split_whitespace().map(str::to_string).collect())
}

fn executable_plan(
    ctx: &PlanContext,
    probe: &dyn Probe,
    game: &GameInfo,
    meta: Option<&LocalGameMeta>,
    settings: &GameSettings,
    opts: &LaunchOptions,
) -> LaunchPlan {
    // A Linux game started by a command in its folder (targetExe still wins).
    if settings.target_exe.is_empty() {
        if let Some((command, folder)) = heronite_data::sideload::command_of(game) {
            let mut extra = split_args(&settings.launcher_args);
            extra.extend(opts.extra_args.iter().cloned());
            return command_plan(ctx, probe, game, settings, command, folder, extra);
        }
    }
    // targetExe wins; otherwise what install-state would have stored:
    // the first existing of (remapped executable, install.executable).
    let install_exe = game.install.executable.as_deref();
    let executable = if !settings.target_exe.is_empty() {
        settings.target_exe.clone()
    } else {
        first_existing(probe, &[meta.and_then(|m| m.remapped_executable.as_deref()), install_exe]).or(install_exe).unwrap_or("").to_string()
    };
    if executable.is_empty() {
        return LaunchPlan::Unavailable(tr!("No executable is configured for this game").into());
    }
    let mut extra = split_args(&settings.launcher_args);
    extra.extend(opts.extra_args.iter().cloned());

    let platform = effective_platform(game, meta, &executable, probe);
    if cfg!(windows) {
        return match platform {
            Platform::Windows => windows_plan(probe, game, settings, &executable, extra, "sideload"),
            Platform::Linux => LaunchPlan::Unavailable(tr!("Linux programs do not run on Windows").into()),
        };
    }
    if platform == Platform::Windows {
        return crate::wine::build_wine_plan(ctx, probe, game, settings, &executable, &extra);
    }

    native_plan(ctx, probe, game, settings, &executable, extra, "sideload")
}

/// A Windows program on Windows: started as it is, from its folder. Linux's
/// wrappers, runtimes and sandbox have no counterpart there, so settings
/// brought from Linux (a restored backup) are left aside; the game's
/// arguments and environment variables still apply. The session follows
/// every process it starts (a job object, see `heronite_platform::process`).
pub(crate) fn windows_plan(
    probe: &dyn Probe,
    game: &GameInfo,
    settings: &GameSettings,
    executable: &str,
    extra: Vec<String>,
    runner: &str,
) -> LaunchPlan {
    if !probe.exists(executable) {
        return LaunchPlan::Unavailable(tr!("Executable not found: {executable}", executable));
    }
    let path = std::path::Path::new(executable);
    let folder = path.parent().map(|d| d.to_string_lossy().into_owned()).filter(|d| !d.is_empty());
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut env = setup_wrapper_env_vars(&game.app_name, runner);
    env.extend(settings.env_options.iter().cloned());
    LaunchPlan::Native(ProcessPlan {
        command: Command { program: executable.to_string(), args: extra, env, cwd: folder },
        stop: StopSpec { kill_patterns: vec![name].into_iter().filter(|n| !n.is_empty()).collect(), commands: vec![] },
        ..Default::default()
    })
}

/// A Linux game started by a shell command inside its folder: `sh -c` runs
/// the command with the game's arguments appended (`"$@"`), under the same
/// wrappers and environment as a native executable.
fn command_plan(
    ctx: &PlanContext,
    probe: &dyn Probe,
    game: &GameInfo,
    settings: &GameSettings,
    command: &str,
    folder: &str,
    extra: Vec<String>,
) -> LaunchPlan {
    let program_file = match crate::native::check_command(command, folder, probe) {
        Ok(file) => file,
        Err(why) => return LaunchPlan::Unavailable(why),
    };
    let mut notes = Vec::new();
    let umu = native_runtime(ctx, probe, settings, &mut notes);
    let umu_missing = umu.as_deref().is_some_and(|u| !probe.exists(u));
    let mut env = setup_wrapper_env_vars(&game.app_name, "sideload");
    env.extend(setup_env_vars(settings, Some(folder), probe));
    if let Err(err) = crate::lsfg::apply(&mut env, &settings.lsfg, &ctx.global, probe) {
        return LaunchPlan::Unavailable(tr!("Launch aborted: {error}", error = err));
    }
    let sandbox = match native_sandbox(ctx, probe, game, settings, folder, &env) {
        Ok(p) => p,
        Err(e) => return LaunchPlan::Unavailable(e),
    };
    let wp = build_wrappers_in(settings, probe, umu.as_deref(), sandbox.as_deref());
    if let Some(err) = wp.error {
        return LaunchPlan::Unavailable(tr!("Launch aborted: {error}", error = err));
    }
    let mut parts = wp.wrappers;
    parts.extend(["/bin/sh".to_string(), "-c".to_string(), format!("{command} \"$@\""), "sh".to_string()]);
    parts.extend(extra);
    let program = parts.remove(0);
    LaunchPlan::Native(ProcessPlan {
        command: Command { program, args: parts, env, cwd: Some(folder.to_string()) },
        stop: StopSpec { kill_patterns: command_program(command).into_iter().collect(), commands: vec![] },
        // An AppImage or script copied without the executable bit gets it, as executables do.
        make_executable: program_file.filter(|p| !probe.is_executable(p)),
        prep: PrepSpec { umu: umu_missing, ..Default::default() },
        notes,
        sandboxed: sandbox.is_some(),
        ..Default::default()
    })
}

/// umu for a Linux game in the Steam runtime (`steamRuntime`); none in the
/// sandbox (said in `notes`, see [`crate::sandbox::without_umu`]).
fn native_runtime(ctx: &PlanContext, probe: &dyn Probe, settings: &GameSettings, notes: &mut Vec<String>) -> Option<String> {
    settings.steam_runtime.as_ref()?;
    if settings.sandbox.enabled {
        notes.push(tr!("In the sandbox, the game runs without the Steam runtime").into());
        return None;
    }
    Some(crate::wine::umu_path(ctx, probe))
}

/// The sandbox of a Linux game: its own private home, `folder` read-only.
fn native_sandbox(
    ctx: &PlanContext,
    probe: &dyn Probe,
    game: &GameInfo,
    settings: &GameSettings,
    folder: &str,
    env: &crate::env::Env,
) -> Result<Option<Vec<String>>, String> {
    let req = crate::sandbox::Request { folder: Some(folder), read: Vec::new(), env: Some(env) };
    crate::sandbox::prelude(ctx, probe, settings, crate::sandbox::Owner::Game(&game.app_name), &req)
}

/// The program a command starts, for stopping it: the first word that is not
/// `env` or a `NAME=value` assignment, without its folder.
fn command_program(command: &str) -> Option<String> {
    crate::native::command_program(command).map(|w| node_basename(&w)).filter(|w| !w.is_empty())
}

/// A Linux program started directly: Heroic's wrappers and environment, no
/// Wine. `runner` is the store named in the `HEROIC_APP_*` variables.
pub(crate) fn native_plan(
    ctx: &PlanContext,
    probe: &dyn Probe,
    game: &GameInfo,
    settings: &GameSettings,
    executable: &str,
    extra: Vec<String>,
    runner: &str,
) -> LaunchPlan {
    if !probe.exists(executable) {
        return LaunchPlan::Unavailable(tr!("Executable not found: {executable}", executable));
    }
    let mut notes = Vec::new();
    let umu = native_runtime(ctx, probe, settings, &mut notes);
    let umu_missing = umu.as_deref().is_some_and(|u| !probe.exists(u));
    let mut env = setup_wrapper_env_vars(&game.app_name, runner);
    env.extend(setup_env_vars(settings, game.install.install_path.as_deref(), probe));
    if let Err(err) = crate::lsfg::apply(&mut env, &settings.lsfg, &ctx.global, probe) {
        return LaunchPlan::Unavailable(tr!("Launch aborted: {error}", error = err));
    }
    let sandbox = match native_sandbox(ctx, probe, game, settings, &crate::wine::game_folder(game, executable), &env) {
        Ok(p) => p,
        Err(e) => return LaunchPlan::Unavailable(e),
    };
    let wp = build_wrappers_in(settings, probe, umu.as_deref(), sandbox.as_deref());
    if let Some(err) = wp.error {
        return LaunchPlan::Unavailable(tr!("Launch aborted: {error}", error = err));
    }

    let mut parts = wp.wrappers;
    parts.push(executable.to_string());
    parts.extend(extra);
    let program = parts.remove(0);
    LaunchPlan::Native(ProcessPlan {
        command: Command { program, args: parts, env, cwd: Some(node_dirname(executable)) },
        stop: StopSpec { kill_patterns: vec![node_basename(executable)], commands: vec![] },
        make_executable: (!probe.is_executable(executable)).then(|| executable.to_string()),
        prep: PrepSpec { umu: umu_missing, ..Default::default() },
        notes,
        sandboxed: sandbox.is_some(),
        ..Default::default()
    })
}
