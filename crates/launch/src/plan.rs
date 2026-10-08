//! Launch plans: pure data describing *how* a game would be started.
//!
//! Building a plan never spawns anything. The [`crate::runtime::Launcher`]
//! executes plans through a [`crate::runtime::Spawner`].

use heronite_data::models::{GameInfo, LocalGameMeta};
use heronite_data::HeroicPaths;
use serde_json::Value;

use crate::emulators::{
    emulator_definition, expand_argv, node_basename, node_dirname, resolve_profile_template,
    vars_for_rom, EmulatorsFile,
};
use crate::env::{build_wrappers, setup_env_vars, setup_wrapper_env_vars, Env};
use crate::native::{effective_platform, is_executable_game, Platform};
use crate::probe::Probe;
use crate::settings::{load_game_settings, GameSettings, GlobalSettings};
use crate::steam;

/// A process to spawn. `env` holds only *additions* to the inherited
/// environment; `program` is run as given (resolved through `$PATH` if bare).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Command {
    pub program: String,
    pub args: Vec<String>,
    pub env: Env,
    pub cwd: Option<String>,
}

impl Command {
    /// Shell-quoted one-liner for logs.
    pub fn display(&self) -> String {
        let mut parts = vec![self.program.clone()];
        parts.extend(self.args.iter().cloned());
        shlex::try_join(parts.iter().map(String::as_str)).unwrap_or_else(|_| parts.join(" "))
    }
}

/// What "stop" does besides killing the tracked process tree: Heroic runs
/// `pkill -f <pattern>` and shuts the Wine prefix down.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StopSpec {
    pub kill_patterns: Vec<String>,
    pub commands: Vec<Command>,
}

/// How a Windows program is run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WineRuntime {
    /// `umu-run` with `PROTONPATH` (Proton inside the Steam Runtime).
    Umu,
    /// `proton waitforexitandrun <exe>`.
    Proton,
    /// `wine <exe>`.
    Wine,
}

/// Anything that is spawned as a child process.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProcessPlan {
    pub command: Command,
    pub stop: StopSpec,
    /// Directories to create first (the Wine prefix).
    pub create_dirs: Vec<String>,
    /// Run to completion, in order, before `command` (prefix creation).
    pub pre_commands: Vec<Command>,
    /// Run after `command` exits (`wineserver --wait`).
    pub wait_after_exit: Option<Command>,
    /// Program to `chmod +x` before launching (Heroic does this too).
    pub make_executable: Option<String>,
    /// Human-readable decisions worth logging (e.g. fell back to default Wine).
    pub notes: Vec<String>,
    /// What Heroic's `prepareLaunch` / `prepareWineLaunch` and the cloud-save
    /// sync around a launch need before and after the process runs.
    pub prep: PrepSpec,
}

/// Work around a launch that needs the network, the tools folder or the
/// Wine prefix (run by [`crate::prepare`] on the launch thread).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrepSpec {
    /// The umu runtime is wanted but not installed: download it first.
    pub umu: bool,
    /// Effective settings of a game run through Wine/Proton: drives the
    /// DXVK / DXVK-NVAPI / VKD3D install into the prefix and the EAC /
    /// BattlEye runtime downloads.
    pub wine: Option<GameSettings>,
    /// `savesPath` when `autoSyncSaves` is on: cloud saves are downloaded
    /// before the game starts and uploaded after it exits.
    pub cloud_saves: Option<String>,
}

impl PrepSpec {
    pub fn is_empty(&self) -> bool {
        *self == PrepSpec::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchPlan {
    /// Open `steam://rungameid/<id>`; the game process is found in Steam's logs.
    SteamUri { app_id: String, uri: String },
    /// A Linux program started directly (with Heroic's wrappers and env).
    Native(ProcessPlan),
    /// A Windows program through umu/Proton/Wine.
    Wine {
        process: ProcessPlan,
        runtime: WineRuntime,
        prefix: String,
    },
    /// An emulator with its ROM.
    Emulator {
        process: ProcessPlan,
        emulator: String,
        rom: Option<String>,
    },
    /// `legendary launch <app>` (Epic).
    Legendary {
        process: ProcessPlan,
        app_name: String,
    },
    /// Cannot be launched; the string says why.
    Unavailable(String),
}

impl LaunchPlan {
    pub fn process(&self) -> Option<&ProcessPlan> {
        match self {
            LaunchPlan::Native(p)
            | LaunchPlan::Wine { process: p, .. }
            | LaunchPlan::Emulator { process: p, .. }
            | LaunchPlan::Legendary { process: p, .. } => Some(p),
            LaunchPlan::SteamUri { .. } | LaunchPlan::Unavailable(_) => None,
        }
    }

    pub fn unavailable_reason(&self) -> Option<&str> {
        match self {
            LaunchPlan::Unavailable(r) => Some(r),
            _ => None,
        }
    }

    /// Short stable label for logs and UI ("steam", "native", ...).
    pub fn kind(&self) -> &'static str {
        match self {
            LaunchPlan::SteamUri { .. } => "steam",
            LaunchPlan::Native(_) => "native",
            LaunchPlan::Wine { .. } => "wine",
            LaunchPlan::Emulator { .. } => "emulator",
            LaunchPlan::Legendary { .. } => "legendary",
            LaunchPlan::Unavailable(_) => "unavailable",
        }
    }
}

/// Everything read from disk that plan building needs.
#[derive(Debug, Clone)]
pub struct PlanContext {
    pub paths: HeroicPaths,
    pub global: GlobalSettings,
    pub emulators: EmulatorsFile,
    /// `language` from `store/config.json` (legendary's `--language`).
    pub ui_language: Option<String>,
    /// `<heroic>/known_fixes.json`, if downloaded.
    pub known_fixes: Option<Value>,
    /// `store_cache/umu.json` (cached umu ids), if present.
    pub umu_ids: Option<Value>,
}

impl PlanContext {
    pub fn load(paths: &HeroicPaths) -> Self {
        use heronite_data::json_store::read_value;
        let opt = |p: std::path::PathBuf| read_value(&p).ok().filter(|v| !v.is_null());
        let config = opt(paths.config());
        Self {
            paths: paths.clone(),
            global: GlobalSettings::load(paths),
            emulators: heronite_data::json_store::read(&paths.emulators()).unwrap_or_default(),
            ui_language: config
                .as_ref()
                .and_then(|c| c.get("language"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            known_fixes: opt(paths.root.join("known_fixes.json")),
            umu_ids: opt(paths.root.join("store_cache/umu.json")),
        }
    }

    pub fn tools_dir(&self) -> String {
        self.paths.tools_dir().to_string_lossy().into_owned()
    }

    pub fn runtime_dir(&self) -> String {
        crate::env::join(&self.tools_dir(), "runtimes")
    }
}

/// Per-launch inputs from the caller.
#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    /// Extra arguments after the game's configured `launcherArgs`. For
    /// emulators the first one may name the ROM to run.
    pub extra_args: Vec<String>,
    /// legendary `--skip-version-check`.
    pub skip_version_check: bool,
    /// legendary `--override-exe` for an alternative executable.
    pub alt_exe: Option<String>,
    /// Emulators: arguments that boot into a save state, put before the rest
    /// of argv.
    pub state_args: Vec<String>,
    /// Emulators: the state boots the game by itself, so the ROM argument is
    /// dropped from argv.
    pub state_replaces_image: bool,
}

/// Builds the plan for a game, reading its `GamesConfig` from disk.
pub fn build_plan(
    ctx: &PlanContext,
    probe: &dyn Probe,
    game: &GameInfo,
    meta: Option<&LocalGameMeta>,
    opts: &LaunchOptions,
) -> LaunchPlan {
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
        other => LaunchPlan::Unavailable(format!("Launching {other} games is not supported yet")),
    }
}

fn is_browser_game(game: &GameInfo) -> bool {
    game.rest
        .get("browserUrl")
        .map(|v| !v.is_null() && v != "")
        .unwrap_or(false)
        || game.install.platform.as_deref() == Some("Browser")
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
            EmulatorOutcome::Wine { executable, args } => {
                crate::wine::build_wine_plan(ctx, probe, game, settings, &executable, &args)
            }
        };
    }

    // 2. Local executables (tryLaunchNativeLocalGame, else Heroic's launchGame).
    if is_executable_game(game, meta) {
        if is_browser_game(game) {
            return LaunchPlan::Unavailable("Browser games are not supported".into());
        }
        return executable_plan(ctx, probe, game, meta, settings, opts);
    }

    // 3. Steam URI games.
    if let Some(m) = meta.filter(|m| m.launch_kind == "steam-uri") {
        if let Some(app_id) = m.steam_app_id.as_deref().filter(|s| !s.is_empty()) {
            if steam::find_steam_binary(probe).is_none() {
                return LaunchPlan::Unavailable("Steam client was not found on PATH".into());
            }
            return LaunchPlan::SteamUri {
                app_id: app_id.to_string(),
                uri: steam::steam_uri("rungameid", app_id),
            };
        }
    }
    LaunchPlan::Unavailable("This game has no launch method".into())
}

fn first_existing<'a>(probe: &dyn Probe, paths: &[Option<&'a str>]) -> Option<&'a str> {
    paths
        .iter()
        .flatten()
        .copied()
        .find(|p| !p.is_empty() && probe.exists(p))
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
        first_existing(
            probe,
            &[
                meta.and_then(|m| m.remapped_executable.as_deref()),
                install_exe,
            ],
        )
        .or(install_exe)
        .unwrap_or("")
        .to_string()
    };
    if executable.is_empty() {
        return LaunchPlan::Unavailable("No executable is configured for this game".into());
    }
    let mut extra = split_args(&settings.launcher_args);
    extra.extend(opts.extra_args.iter().cloned());

    if effective_platform(game, meta, &executable, probe) == Platform::Windows {
        return crate::wine::build_wine_plan(ctx, probe, game, settings, &executable, &extra);
    }

    // Linux program: Heroic's wrappers and environment, no Wine.
    if !probe.exists(&executable) {
        return LaunchPlan::Unavailable(format!("Executable not found: {executable}"));
    }
    let umu = settings
        .steam_runtime
        .as_ref()
        .map(|_| crate::wine::umu_path(ctx, probe));
    let umu_missing = umu.as_deref().is_some_and(|u| !probe.exists(u));
    let wp = build_wrappers(settings, probe, umu.as_deref());
    if let Some(err) = wp.error {
        return LaunchPlan::Unavailable(format!("Launch aborted: {err}"));
    }
    let mut env = setup_wrapper_env_vars(&game.app_name, "sideload");
    env.extend(setup_env_vars(
        settings,
        game.install.install_path.as_deref(),
        probe,
    ));

    let mut parts = wp.wrappers;
    parts.push(executable.clone());
    parts.extend(extra);
    let program = parts.remove(0);
    LaunchPlan::Native(ProcessPlan {
        command: Command {
            program,
            args: parts,
            env,
            cwd: Some(node_dirname(&executable)),
        },
        stop: StopSpec {
            kill_patterns: vec![node_basename(&executable)],
            commands: vec![],
        },
        make_executable: (!probe.is_executable(&executable)).then(|| executable.clone()),
        prep: PrepSpec {
            umu: umu_missing,
            ..Default::default()
        },
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------
// Emulators (port of emulation/launch.ts)
// ---------------------------------------------------------------------------

#[allow(clippy::large_enum_variant)]
enum EmulatorOutcome {
    Plan(LaunchPlan),
    Wine {
        executable: String,
        args: Vec<String>,
    },
}

/// `pickRomPath`.
pub fn pick_rom_path(
    meta: &LocalGameMeta,
    extra_args: &[String],
    probe: &dyn Probe,
) -> Option<String> {
    let requested = extra_args
        .first()
        .filter(|s| !s.is_empty())
        .cloned()
        .or_else(|| meta.selected_rom_path.clone().filter(|s| !s.is_empty()));
    if let Some(req) = &requested {
        if meta
            .roms
            .as_ref()
            .map(|r| r.iter().any(|rom| &rom.path == req))
            .unwrap_or(false)
        {
            return Some(req.clone());
        }
        if probe.exists(req) {
            return Some(req.clone());
        }
    }
    meta.roms
        .as_ref()?
        .iter()
        .find(|r| probe.exists(&r.path))
        .map(|r| r.path.clone())
}

/// `emulatorExecutablePath`: `flatpak` for flatpak emulators, else the
/// emulator's executable, else the game's remapped executable.
pub fn emulator_executable(ctx: &PlanContext, meta: &LocalGameMeta) -> Option<String> {
    let emulator = meta
        .emulator_id
        .as_deref()
        .and_then(|id| ctx.emulators.get(id));
    if let Some(e) = emulator {
        if e.launch_kind == "flatpak" {
            return Some("flatpak".into());
        }
        if !e.executable.is_empty() {
            return Some(e.executable.clone());
        }
    }
    meta.remapped_executable.clone().filter(|s| !s.is_empty())
}

fn emulator_plan(
    ctx: &PlanContext,
    probe: &dyn Probe,
    game: &GameInfo,
    meta: &LocalGameMeta,
    _settings: &GameSettings,
    opts: &LaunchOptions,
) -> EmulatorOutcome {
    let unavailable = |s: &str| EmulatorOutcome::Plan(LaunchPlan::Unavailable(s.to_string()));
    let emulator = meta
        .emulator_id
        .as_deref()
        .and_then(|id| ctx.emulators.get(id));
    let profile =
        emulator.and_then(|e| e.profile(meta.emulator_profile_id.as_deref().unwrap_or("")));
    let template = emulator
        .zip(profile)
        .map(|(e, p)| resolve_profile_template(e, p));

    let rom_path = pick_rom_path(meta, &opts.extra_args, probe);
    let is_flatpak = emulator
        .map(|e| e.launch_kind == "flatpak")
        .unwrap_or(false);
    let Some(executable) = emulator_executable(ctx, meta) else {
        return unavailable("Emulator executable was not found");
    };
    if !is_flatpak && !probe.exists(&executable) {
        return unavailable("Emulator executable was not found");
    }
    let has_roms = meta.roms.as_ref().map(|r| !r.is_empty()).unwrap_or(false);
    if has_roms && rom_path.is_none() {
        return unavailable("ROM file was not found");
    }

    let emulator_dir = emulator
        .and_then(|e| e.install_dir.clone().filter(|d| !d.is_empty()))
        .unwrap_or_else(|| node_dirname(&executable));
    let core_path = template
        .as_ref()
        .and_then(|t| t.core_path.clone())
        .or_else(|| profile.and_then(|p| p.core_path.clone()));
    let vars = vars_for_rom(
        rom_path.as_deref().unwrap_or(""),
        Some(&emulator_dir),
        core_path.as_deref(),
        &meta.title,
    );
    let argv_template = template
        .as_ref()
        .map(|t| t.arguments.clone())
        .filter(|s| !s.is_empty())
        .or_else(|| meta.launcher_args.clone().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "{ImagePath}".to_string());
    let mut argv = expand_argv(&argv_template, &vars);
    if let Some(rom) = &rom_path {
        for r in meta.roms.iter().flatten() {
            for part in argv.iter_mut() {
                if *part == r.path {
                    *part = rom.clone();
                }
            }
        }
        if !argv.contains(rom) && !argv_template.contains("{Image") {
            argv.push(rom.clone());
        }
    }

    if !opts.state_args.is_empty() || opts.state_replaces_image {
        if opts.state_replaces_image {
            argv.retain(|a| Some(a) != rom_path.as_ref());
        }
        argv.splice(0..0, opts.state_args.iter().cloned());
    }

    let use_wine = emulator.map(|e| e.launch_kind == "wine").unwrap_or(false)
        || executable.to_lowercase().ends_with(".exe");
    if use_wine {
        return EmulatorOutcome::Wine {
            executable,
            args: argv,
        };
    }

    let (program, args, dir) = match emulator {
        Some(e)
            if e.launch_kind == "flatpak"
                && e.flatpak_id
                    .as_deref()
                    .map(|f| !f.is_empty())
                    .unwrap_or(false) =>
        {
            let mut a = vec!["run".to_string(), e.flatpak_id.clone().unwrap()];
            a.extend(argv);
            ("flatpak".to_string(), a, String::new())
        }
        _ => {
            let dir = node_dirname(&executable);
            (executable.clone(), argv, dir)
        }
    };
    let cwd = if template.as_ref().and_then(|t| t.working_dir.as_deref()) == Some("rom")
        && rom_path.is_some()
    {
        Some(node_dirname(rom_path.as_deref().unwrap()))
    } else {
        emulator
            .and_then(|e| e.install_dir.clone())
            .filter(|d| !d.is_empty())
            .or_else(|| (!dir.is_empty()).then_some(dir))
    };

    // stopPattern: catalog pattern, else the emulator binary's name.
    let definition = emulator.and_then(|e| emulator_definition(&e.definition_id));
    let pattern = definition
        .and_then(|d| d.stop_pattern.map(str::to_string))
        .or_else(|| {
            emulator
                .filter(|e| !e.executable.is_empty())
                .map(|e| node_basename(&e.executable))
        });
    let mut kill_patterns: Vec<String> = pattern.into_iter().collect();
    if let Some(e) = emulator.filter(|e| e.launch_kind == "flatpak") {
        if let Some(last) = e
            .flatpak_id
            .as_deref()
            .and_then(|f| f.rsplit('.').next())
            .filter(|s| !s.is_empty())
        {
            kill_patterns.push(last.to_string());
        }
    }

    EmulatorOutcome::Plan(LaunchPlan::Emulator {
        process: ProcessPlan {
            command: Command {
                program,
                args,
                env: Env::new(),
                cwd,
            },
            stop: StopSpec {
                kill_patterns,
                commands: vec![],
            },
            ..Default::default()
        },
        emulator: emulator
            .map(|e| e.name.clone())
            .filter(|n| !n.is_empty())
            .or_else(|| meta.emulator_name.clone().filter(|n| !n.is_empty()))
            .unwrap_or_else(|| game.title.clone()),
        rom: rom_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emulators::{UserEmulator, UserEmulatorProfile};
    use crate::probe::FakeProbe;
    use heronite_data::models::{InstallInfo, LocalGameRom};
    use serde_json::json;

    const UMU: &str = "/hr/tools/runtimes/umu/umu_run.py";

    pub(crate) fn ctx(global: Value) -> PlanContext {
        let mut c = PlanContext::load(&HeroicPaths::at("/hr"));
        c.global = GlobalSettings::from_defaults(global.as_object().cloned().unwrap_or_default());
        c
    }

    fn settings(v: Value) -> GameSettings {
        GameSettings::from_map(v.as_object().unwrap(), "/home/u")
    }

    fn sideload(name: &str, exe: &str, platform: &str) -> GameInfo {
        GameInfo {
            runner: "sideload".into(),
            app_name: name.into(),
            title: format!("Title of {name}"),
            install: InstallInfo {
                executable: Some(exe.into()),
                platform: Some(platform.into()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn meta(kind: &str) -> LocalGameMeta {
        LocalGameMeta {
            app_name: "g".into(),
            runner: "sideload".into(),
            launch_kind: kind.into(),
            title: "Game".into(),
            ..Default::default()
        }
    }

    fn proton_probe() -> FakeProbe {
        FakeProbe::new("/home/u")
            .with_path("/p/GE/proton")
            .with_path(UMU)
            .with_path("/games/win/game.exe")
            .with_path("/pfx")
    }

    fn proton_settings() -> GameSettings {
        settings(json!({
            "wineVersion": {"bin": "/p/GE/proton", "name": "GE-Proton9", "type": "proton"},
            "winePrefix": "/pfx",
            "enviromentOptions": [{"key": "PROTON_PREFER_SDL", "value": "1"}],
            "launcherArgs": "-windowed -w 1920"
        }))
    }

    #[test]
    fn linux_script_runs_native_with_wrappers_and_env() {
        let probe = FakeProbe::new("/home/u")
            .with_file("/g/run.sh", b"#!/bin/sh\n")
            .with_bin("gamemoderun", "/usr/bin/gamemoderun");
        let s = settings(
            json!({"useGameMode": true, "launcherArgs": "--fast \"a b\"", "nvidiaPrime": true}),
        );
        let game = sideload("g", "/g/run.sh", "linux");
        let plan = build_plan_with_settings(
            &ctx(json!({})),
            &probe,
            &game,
            Some(&meta("executable")),
            &s,
            &LaunchOptions {
                extra_args: vec!["--extra".into()],
                ..Default::default()
            },
        );
        let LaunchPlan::Native(p) = plan else {
            panic!("{plan:?}")
        };
        assert_eq!(p.command.program, "/usr/bin/gamemoderun");
        assert_eq!(p.command.args, ["/g/run.sh", "--fast", "a b", "--extra"]);
        assert_eq!(p.command.cwd.as_deref(), Some("/g"));
        assert_eq!(p.command.env["HEROIC_APP_NAME"], "g");
        assert_eq!(p.command.env["DRI_PRIME"], "1");
        assert_eq!(p.stop.kill_patterns, ["run.sh"]);
        assert_eq!(p.make_executable, None);
        let locked = FakeProbe::new("/home/u")
            .with_non_executable("/g/run.sh")
            .with_file("/g/run.sh", b"#!/bin/sh\n");
        let m = LocalGameMeta {
            run_as: Some("linux".into()),
            ..meta("executable")
        };
        let LaunchPlan::Native(p) = build_plan_with_settings(
            &ctx(json!({})),
            &locked,
            &game,
            Some(&m),
            &settings(json!({})),
            &Default::default(),
        ) else {
            panic!()
        };
        assert_eq!(p.make_executable.as_deref(), Some("/g/run.sh"));
    }

    #[test]
    fn native_missing_executable_and_missing_tool_are_unavailable() {
        let game = sideload("g", "/g/run.sh", "linux");
        let m = LocalGameMeta {
            run_as: Some("linux".into()),
            ..meta("executable")
        };
        let plan = build_plan_with_settings(
            &ctx(json!({})),
            &FakeProbe::new("/h"),
            &game,
            Some(&m),
            &settings(json!({})),
            &Default::default(),
        );
        assert_eq!(
            plan,
            LaunchPlan::Unavailable("Executable not found: /g/run.sh".into())
        );

        let probe = FakeProbe::new("/h").with_file("/g/run.sh", b"#!/bin/sh");
        let plan = build_plan_with_settings(
            &ctx(json!({})),
            &probe,
            &game,
            Some(&m),
            &settings(json!({"useGameMode": true})),
            &Default::default(),
        );
        assert!(plan.unavailable_reason().unwrap().contains("gamemoderun"));

        let none = sideload("g", "", "Windows");
        let plan = build_plan_with_settings(
            &ctx(json!({})),
            &FakeProbe::new("/h"),
            &none,
            Some(&meta("unavailable")),
            &settings(json!({})),
            &Default::default(),
        );
        assert_eq!(
            plan,
            LaunchPlan::Unavailable("No executable is configured for this game".into())
        );
    }

    #[test]
    fn windows_exe_uses_umu_with_proton_path() {
        let game = sideload("g", "/games/win/game.exe", "Windows");
        let plan = build_plan_with_settings(
            &ctx(json!({})),
            &proton_probe(),
            &game,
            Some(&meta("executable")),
            &proton_settings(),
            &Default::default(),
        );
        let LaunchPlan::Wine {
            process,
            runtime,
            prefix,
        } = plan
        else {
            panic!("{plan:?}")
        };
        assert_eq!(runtime, WineRuntime::Umu);
        assert_eq!(prefix, "/pfx");
        let c = &process.command;
        assert_eq!(c.program, UMU);
        assert_eq!(c.args, ["/games/win/game.exe", "-windowed", "-w", "1920"]);
        assert_eq!(c.cwd.as_deref(), Some("/games/win"));
        assert_eq!(c.env["PROTONPATH"], "/p/GE");
        assert_eq!(c.env["WINEPREFIX"], "/pfx");
        assert_eq!(c.env["GAMEID"], "umu-0");
        assert_eq!(c.env["PROTON_VERB"], "waitforexitandrun");
        assert_eq!(c.env["PROTON_PREFER_SDL"], "1");
    }

    #[test]
    fn missing_prefix_files_trigger_prefix_creation() {
        let game = sideload("g", "/games/win/game.exe", "Windows");
        let probe = proton_probe();
        let plan = build_plan_with_settings(
            &ctx(json!({})),
            &probe,
            &game,
            Some(&meta("executable")),
            &proton_settings(),
            &Default::default(),
        );
        let LaunchPlan::Wine { process, .. } = plan else {
            panic!()
        };
        // proton_probe has /pfx but none of the files proton needs.
        assert_eq!(process.create_dirs, ["/pfx"]);
        assert_eq!(process.pre_commands.len(), 1);
        assert_eq!(process.pre_commands[0].args, ["createprefix"]);
        assert_eq!(process.pre_commands[0].env["PROTON_VERB"], "run");
    }

    #[test]
    fn disabled_umu_runs_proton_directly() {
        let probe = FakeProbe::new("/home/u")
            .with_path("/p/GE/proton")
            .with_path("/games/win/game.exe");
        let game = sideload("g", "/games/win/game.exe", "Windows");
        // umu is wanted but not installed: the launch downloads it (Heroic
        // does too) instead of silently running Proton without the runtime.
        let LaunchPlan::Wine {
            process, runtime, ..
        } = build_plan_with_settings(
            &ctx(json!({})),
            &probe,
            &game,
            Some(&meta("executable")),
            &proton_settings(),
            &Default::default(),
        )
        else {
            panic!()
        };
        assert_eq!(runtime, WineRuntime::Umu);
        assert_eq!(process.command.program, UMU);
        assert!(process.prep.umu);
        assert!(process.notes.iter().any(|n| n.contains("umu-run")));
        assert_eq!(process.prep.wine.as_ref().unwrap().wine_prefix, "/pfx");
        // With umu in place there is nothing to download.
        let LaunchPlan::Wine { process, .. } = build_plan_with_settings(
            &ctx(json!({})),
            &proton_probe(),
            &game,
            Some(&meta("executable")),
            &proton_settings(),
            &Default::default(),
        ) else {
            panic!()
        };
        assert!(!process.prep.umu);
        // disableUMU runs Proton directly.
        let probe2 = proton_probe();
        let mut s = proton_settings();
        s.disable_umu = Some(true);
        let LaunchPlan::Wine { runtime, .. } = build_plan_with_settings(
            &ctx(json!({})),
            &probe2,
            &game,
            Some(&meta("executable")),
            &s,
            &Default::default(),
        ) else {
            panic!()
        };
        assert_eq!(runtime, WineRuntime::Proton);
        let LaunchPlan::Wine { process, .. } = build_plan_with_settings(
            &ctx(json!({})),
            &probe2,
            &game,
            Some(&meta("executable")),
            &s,
            &Default::default(),
        ) else {
            panic!()
        };
        assert_eq!(process.command.program, "/p/GE/proton");
        assert_eq!(
            process.command.args[..2],
            ["waitforexitandrun", "/games/win/game.exe"]
        );
        assert!(!process.prep.umu);
    }

    #[test]
    fn plain_wine_waits_for_wineserver() {
        let probe = FakeProbe::new("/home/u")
            .with_path("/w/bin/wine")
            .with_path("/w/bin/wineserver")
            .with_path("/games/win/game.exe");
        let s = settings(
            json!({"wineVersion": {"bin": "/w/bin/wine", "name": "Wine", "type": "wine"}, "winePrefix": "/pfx"}),
        );
        let game = sideload("g", "/games/win/game.exe", "Windows");
        let LaunchPlan::Wine {
            process, runtime, ..
        } = build_plan_with_settings(
            &ctx(json!({})),
            &probe,
            &game,
            None,
            &s,
            &Default::default(),
        )
        else {
            panic!()
        };
        assert_eq!(runtime, WineRuntime::Wine);
        assert_eq!(process.command.program, "/w/bin/wine");
        assert_eq!(process.command.args, ["/games/win/game.exe"]);
        assert_eq!(
            process.wait_after_exit.as_ref().unwrap().program,
            "/w/bin/wineserver"
        );
        assert_eq!(process.wait_after_exit.as_ref().unwrap().args, ["--wait"]);
        assert_eq!(process.stop.commands[0].program, "/w/bin/wineserver");
        assert_eq!(process.stop.commands[0].args, ["-k"]);
        assert_eq!(process.pre_commands[0].args, ["wineboot", "--init"]);
    }

    #[test]
    fn invalid_game_wine_falls_back_to_global_default() {
        let game = sideload("g", "/games/win/game.exe", "Windows");
        let s = settings(
            json!({"wineVersion": {"bin": "", "name": "Default Wine - Not Found", "type": "wine"}, "winePrefix": "/pfx"}),
        );
        let global =
            json!({"wineVersion": {"bin": "/p/GE/proton", "name": "GE-Proton9", "type": "proton"}});
        let LaunchPlan::Wine {
            process, runtime, ..
        } = build_plan_with_settings(
            &ctx(global),
            &proton_probe(),
            &game,
            None,
            &s,
            &Default::default(),
        )
        else {
            panic!()
        };
        assert_eq!(runtime, WineRuntime::Umu);
        assert!(process.notes[0].contains("Default Wine - Not Found"));
        let none = build_plan_with_settings(
            &ctx(json!({})),
            &proton_probe(),
            &game,
            None,
            &s,
            &Default::default(),
        );
        assert!(none.unavailable_reason().unwrap().contains("not valid"));
    }

    #[test]
    fn run_as_windows_sends_an_elf_through_wine() {
        let probe = proton_probe().with_file("/games/win/game.exe", b"\x7fELF");
        let game = sideload("g", "/games/win/game.exe", "linux");
        let m = LocalGameMeta {
            run_as: Some("windows".into()),
            ..meta("executable")
        };
        assert_eq!(
            build_plan_with_settings(
                &ctx(json!({})),
                &probe,
                &game,
                Some(&m),
                &proton_settings(),
                &Default::default()
            )
            .kind(),
            "wine"
        );
        let linux = LocalGameMeta {
            run_as: Some("linux".into()),
            ..meta("executable")
        };
        assert_eq!(
            build_plan_with_settings(
                &ctx(json!({})),
                &probe,
                &game,
                Some(&linux),
                &proton_settings(),
                &Default::default()
            )
            .kind(),
            "native"
        );
    }

    #[test]
    fn steam_games_become_uris_when_steam_exists() {
        let m = LocalGameMeta {
            steam_app_id: Some("400".into()),
            ..meta("steam-uri")
        };
        let game = sideload("steam_400", "/usr/bin/steam", "linux");
        let with_steam = FakeProbe::new("/h").with_bin("steam", "/usr/bin/steam");
        assert_eq!(
            build_plan_with_settings(
                &ctx(json!({})),
                &with_steam,
                &game,
                Some(&m),
                &settings(json!({})),
                &Default::default()
            ),
            LaunchPlan::SteamUri {
                app_id: "400".into(),
                uri: "steam://rungameid/400".into()
            }
        );
        let plan = build_plan_with_settings(
            &ctx(json!({})),
            &FakeProbe::new("/h"),
            &game,
            Some(&m),
            &settings(json!({})),
            &Default::default(),
        );
        assert_eq!(
            plan,
            LaunchPlan::Unavailable("Steam client was not found on PATH".into())
        );
    }

    fn emu_ctx(emu: UserEmulator) -> PlanContext {
        let mut c = ctx(json!({}));
        c.emulators.emulators.push(emu);
        c
    }

    fn profile(id: &str, args: &str) -> UserEmulatorProfile {
        UserEmulatorProfile {
            id: id.into(),
            definition_profile_id: Some(id.into()),
            arguments: args.into(),
            ..Default::default()
        }
    }

    fn emu_meta(roms: &[&str]) -> LocalGameMeta {
        LocalGameMeta {
            emulator_id: Some("e1".into()),
            emulator_profile_id: Some("custom".into()),
            roms: Some(
                roms.iter()
                    .map(|r| LocalGameRom {
                        name: None,
                        path: r.to_string(),
                        extra: Default::default(),
                    })
                    .collect(),
            ),
            ..meta("emulator")
        }
    }

    #[test]
    fn native_emulator_gets_its_rom_and_stop_pattern() {
        let emu = UserEmulator {
            id: "e1".into(),
            definition_id: "custom".into(),
            name: "PCSX2".into(),
            executable: "/emu/pcsx2.AppImage".into(),
            launch_kind: "native".into(),
            profiles: vec![profile("custom", "{ImagePath}")],
            ..Default::default()
        };
        let probe = FakeProbe::new("/h")
            .with_path("/emu/pcsx2.AppImage")
            .with_path("/roms/a.iso")
            .with_path("/roms/b.iso");
        let m = LocalGameMeta {
            selected_rom_path: Some("/roms/b.iso".into()),
            ..emu_meta(&["/roms/a.iso", "/roms/b.iso"])
        };
        let game = sideload("g", "/emu/pcsx2.AppImage", "linux");
        let plan = build_plan_with_settings(
            &emu_ctx(emu),
            &probe,
            &game,
            Some(&m),
            &settings(json!({})),
            &Default::default(),
        );
        let LaunchPlan::Emulator {
            process,
            emulator,
            rom,
        } = plan
        else {
            panic!("{plan:?}")
        };
        assert_eq!(emulator, "PCSX2");
        assert_eq!(rom.as_deref(), Some("/roms/b.iso"));
        assert_eq!(process.command.program, "/emu/pcsx2.AppImage");
        assert_eq!(process.command.args, ["/roms/b.iso"]);
        assert_eq!(process.command.cwd.as_deref(), Some("/emu"));
        assert!(process.command.env.is_empty());
        assert_eq!(process.stop.kill_patterns, ["pcsx2.AppImage"]);
    }

    #[test]
    fn save_state_args_go_first_and_can_replace_the_image() {
        let emu = UserEmulator {
            id: "e1".into(),
            definition_id: "custom".into(),
            executable: "/emu/x".into(),
            launch_kind: "native".into(),
            profiles: vec![profile("custom", "-batch {ImagePath}")],
            ..Default::default()
        };
        let probe = FakeProbe::new("/h").with_path("/emu/x").with_path("/roms/a.iso");
        let m = emu_meta(&["/roms/a.iso"]);
        let game = sideload("g", "/emu/x", "linux");
        let args_for = |opts: LaunchOptions| {
            let LaunchPlan::Emulator { process, .. } = build_plan_with_settings(
                &emu_ctx(emu.clone()),
                &probe,
                &game,
                Some(&m),
                &settings(json!({})),
                &opts,
            ) else {
                panic!()
            };
            process.command.args
        };
        let with_state = args_for(LaunchOptions {
            state_args: vec!["-statefile".into(), "/s/1.state".into()],
            ..Default::default()
        });
        assert_eq!(with_state, ["-statefile", "/s/1.state", "-batch", "/roms/a.iso"]);
        let replaced = args_for(LaunchOptions {
            state_args: vec!["--load".into(), "/s/1.state".into()],
            state_replaces_image: true,
            ..Default::default()
        });
        assert_eq!(replaced, ["--load", "/s/1.state", "-batch"]);
        assert_eq!(args_for(Default::default()), ["-batch", "/roms/a.iso"]);
    }

    #[test]
    fn requested_rom_wins_and_missing_roms_are_unavailable() {
        let emu = UserEmulator {
            id: "e1".into(),
            definition_id: "custom".into(),
            executable: "/emu/x".into(),
            launch_kind: "native".into(),
            profiles: vec![profile("custom", "{ImagePath}")],
            ..Default::default()
        };
        let probe = FakeProbe::new("/h")
            .with_path("/emu/x")
            .with_path("/roms/a.iso")
            .with_path("/roms/b.iso");
        let m = emu_meta(&["/roms/a.iso", "/roms/b.iso"]);
        let game = sideload("g", "/emu/x", "linux");
        let opts = LaunchOptions {
            extra_args: vec!["/roms/b.iso".into()],
            ..Default::default()
        };
        let LaunchPlan::Emulator { process, .. } = build_plan_with_settings(
            &emu_ctx(emu.clone()),
            &probe,
            &game,
            Some(&m),
            &settings(json!({})),
            &opts,
        ) else {
            panic!()
        };
        assert_eq!(process.command.args, ["/roms/b.iso"]);

        let gone = emu_meta(&["/roms/zzz.iso"]);
        let plan = build_plan_with_settings(
            &emu_ctx(emu.clone()),
            &probe,
            &game,
            Some(&gone),
            &settings(json!({})),
            &Default::default(),
        );
        assert_eq!(
            plan,
            LaunchPlan::Unavailable("ROM file was not found".into())
        );

        let plan = build_plan_with_settings(
            &emu_ctx(emu),
            &FakeProbe::new("/h"),
            &game,
            Some(&m),
            &settings(json!({})),
            &Default::default(),
        );
        assert_eq!(
            plan,
            LaunchPlan::Unavailable("Emulator executable was not found".into())
        );
    }

    #[test]
    fn retroarch_core_flatpak_and_rom_working_dir() {
        let mut p = profile("snes", "");
        p.core_path = Some("/cores/snes9x_libretro.so".into());
        let emu = UserEmulator {
            id: "e1".into(),
            definition_id: "retroarch".into(),
            name: "RetroArch".into(),
            launch_kind: "flatpak".into(),
            flatpak_id: Some("org.libretro.RetroArch".into()),
            profiles: vec![p],
            ..Default::default()
        };
        let probe = FakeProbe::new("/h").with_path("/roms/zelda.sfc");
        let m = LocalGameMeta {
            emulator_profile_id: Some("snes".into()),
            ..emu_meta(&["/roms/zelda.sfc"])
        };
        let game = sideload("g", "flatpak", "linux");
        let LaunchPlan::Emulator { process, .. } = build_plan_with_settings(
            &emu_ctx(emu),
            &probe,
            &game,
            Some(&m),
            &settings(json!({})),
            &Default::default(),
        ) else {
            panic!()
        };
        assert_eq!(process.command.program, "flatpak");
        assert_eq!(
            process.command.args,
            [
                "run",
                "org.libretro.RetroArch",
                "-L",
                "/cores/snes9x_libretro.so",
                "/roms/zelda.sfc"
            ]
        );
        assert_eq!(process.command.cwd, None);
        assert_eq!(process.stop.kill_patterns, ["retroarch", "RetroArch"]);

        let mut mame_profile = profile("arcade", "");
        mame_profile.definition_profile_id = Some("arcade".into());
        let mame = UserEmulator {
            id: "e1".into(),
            definition_id: "mame".into(),
            executable: "/usr/bin/mame".into(),
            launch_kind: "native".into(),
            profiles: vec![mame_profile],
            ..Default::default()
        };
        let probe = FakeProbe::new("/h")
            .with_path("/usr/bin/mame")
            .with_path("/roms/arcade/pacman.zip");
        let m = LocalGameMeta {
            emulator_profile_id: Some("arcade".into()),
            ..emu_meta(&["/roms/arcade/pacman.zip"])
        };
        let LaunchPlan::Emulator { process, .. } = build_plan_with_settings(
            &emu_ctx(mame),
            &probe,
            &game,
            Some(&m),
            &settings(json!({})),
            &Default::default(),
        ) else {
            panic!()
        };
        assert_eq!(process.command.args, ["pacman"]);
        assert_eq!(process.command.cwd.as_deref(), Some("/roms/arcade"));
    }

    #[test]
    fn exe_emulators_run_through_wine() {
        let emu = UserEmulator {
            id: "e1".into(),
            definition_id: "custom".into(),
            executable: "/games/win/game.exe".into(),
            launch_kind: "wine".into(),
            profiles: vec![profile("custom", "-fullscreen {ImagePath}")],
            ..Default::default()
        };
        let probe = proton_probe().with_path("/roms/a.iso");
        let m = emu_meta(&["/roms/a.iso"]);
        let game = sideload("g", "/games/win/game.exe", "Windows");
        let LaunchPlan::Wine { process, .. } = build_plan_with_settings(
            &emu_ctx(emu),
            &probe,
            &game,
            Some(&m),
            &proton_settings(),
            &Default::default(),
        ) else {
            panic!()
        };
        assert_eq!(
            process.command.args,
            ["/games/win/game.exe", "-fullscreen", "/roms/a.iso"]
        );
    }

    #[test]
    fn browser_and_unknown_runners_are_unavailable() {
        let mut browser = sideload("b", "", "Browser");
        browser.install.platform = Some("Browser".into());
        let plan = build_plan_with_settings(
            &ctx(json!({})),
            &FakeProbe::new("/h"),
            &browser,
            None,
            &settings(json!({})),
            &Default::default(),
        );
        assert!(plan.unavailable_reason().unwrap().contains("Browser"));
        let gog = GameInfo {
            runner: "gog".into(),
            app_name: "x".into(),
            ..Default::default()
        };
        let plan = build_plan_with_settings(
            &ctx(json!({})),
            &FakeProbe::new("/h"),
            &gog,
            None,
            &settings(json!({})),
            &Default::default(),
        );
        assert!(plan.unavailable_reason().unwrap().contains("gog"));
    }
}
