//! Entry points of plan building: sideload games (executables, Steam URIs) and runner dispatch.

use heronite_data::models::{GameInfo, LocalGameMeta};
use heronite_i18n::tr;

use super::emulator::{emulator_plan, EmulatorOutcome};
use super::{Command, LaunchOptions, LaunchPlan, PlanContext, PrepSpec, ProcessPlan, StopSpec};
use crate::emulators::{node_basename, node_dirname};
use crate::env::{build_wrappers, setup_env_vars, setup_wrapper_env_vars};
use crate::native::{effective_platform, is_executable_game, Platform};
use crate::probe::Probe;
use crate::settings::{load_game_settings, GameSettings};
use crate::steam;

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
    match game.runner.as_str() {
        "sideload" => build_sideload(ctx, probe, game, meta, settings, opts),
        "legendary" => crate::legendary::build(ctx, probe, game, settings, opts),
        other => LaunchPlan::Unavailable(tr!("Launching {other} games is not supported yet", other)),
    }
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
        return match emulator_plan(ctx, probe, game, m, settings, opts) {
            EmulatorOutcome::Plan(p) => p,
            // `.exe` emulators run through Wine like any Windows program.
            EmulatorOutcome::Wine { executable, args } => crate::wine::build_wine_plan(ctx, probe, game, settings, &executable, &args),
        };
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

fn first_existing<'a>(probe: &dyn Probe, paths: &[Option<&'a str>]) -> Option<&'a str> {
    paths.iter().flatten().copied().find(|p| !p.is_empty() && probe.exists(p))
}

fn split_args(s: &str) -> Vec<String> {
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

    if effective_platform(game, meta, &executable, probe) == Platform::Windows {
        return crate::wine::build_wine_plan(ctx, probe, game, settings, &executable, &extra);
    }

    // Linux program: Heroic's wrappers and environment, no Wine.
    if !probe.exists(&executable) {
        return LaunchPlan::Unavailable(tr!("Executable not found: {executable}", executable));
    }
    let umu = settings.steam_runtime.as_ref().map(|_| crate::wine::umu_path(ctx, probe));
    let umu_missing = umu.as_deref().is_some_and(|u| !probe.exists(u));
    let wp = build_wrappers(settings, probe, umu.as_deref());
    if let Some(err) = wp.error {
        return LaunchPlan::Unavailable(tr!("Launch aborted: {error}", error = err));
    }
    let mut env = setup_wrapper_env_vars(&game.app_name, "sideload");
    env.extend(setup_env_vars(settings, game.install.install_path.as_deref(), probe));

    let mut parts = wp.wrappers;
    parts.push(executable.clone());
    parts.extend(extra);
    let program = parts.remove(0);
    LaunchPlan::Native(ProcessPlan {
        command: Command { program, args: parts, env, cwd: Some(node_dirname(&executable)) },
        stop: StopSpec { kill_patterns: vec![node_basename(&executable)], commands: vec![] },
        make_executable: (!probe.is_executable(&executable)).then(|| executable.clone()),
        prep: PrepSpec { umu: umu_missing, ..Default::default() },
        ..Default::default()
    })
}
