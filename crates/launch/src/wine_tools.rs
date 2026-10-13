//! Wine tools of the game settings (winecfg, winetricks, "run EXE on prefix"):
//! ports of `callTool`, `runWineCommand` and `Winetricks.runWithArgs`
//! (`backend/tools/index.ts`, `backend/launcher.ts`). Only builds the command
//! (pure, unit-tested); the caller spawns it.

use heronite_i18n::tr;
use std::path::Path;

use crate::emulators::node_dirname;
use crate::env::{join, setup_env_vars, setup_wine_env_vars, setup_wrapper_env_vars, Env};
use crate::plan::{Command, PlanContext, WineRuntime};
use crate::probe::Probe;
use crate::settings::GameSettings;
use crate::wine::{prepare, umu_path, valid_wine, WineEnv};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WineTool {
    Winecfg,
    /// The winetricks GUI.
    Winetricks,
    /// Runs this program inside the game's prefix.
    RunExe(String),
}

/// A tool command ready to spawn, with what the user should be told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCommand {
    pub command: Command,
    pub notes: Vec<String>,
}

/// Heroic's `winetricks` script: `<tools>/winetricks`.
pub fn winetricks_path(ctx: &PlanContext) -> String {
    join(&ctx.tools_dir(), "winetricks")
}

/// Builds the command for `tool` against the game's effective `settings`.
/// `Err` explains why nothing can run (no usable Wine, missing winetricks...).
pub fn build_tool_command(ctx: &PlanContext, probe: &dyn Probe, settings: &GameSettings, tool: &WineTool) -> Result<ToolCommand, String> {
    let wine = prepare(ctx, probe, settings)?;
    crate::shared_folders::map_for(&wine.settings);
    let mut notes = wine.notes.clone();
    if !probe.exists(&wine.settings.wine_prefix) {
        notes.push(tr!("The Wine prefix {prefix} does not exist yet; Wine creates it on first run", prefix = wine.settings.wine_prefix));
    }
    let command = match tool {
        WineTool::Winecfg => run_in_prefix(ctx, probe, &wine, vec!["winecfg".into()], None),
        WineTool::RunExe(exe) => {
            if exe.trim().is_empty() {
                return Err(tr!("No program selected").into());
            }
            if !probe.exists(exe) {
                return Err(tr!("File not found: {exe}", exe));
            }
            run_in_prefix(ctx, probe, &wine, vec![exe.clone()], Some(node_dirname(exe)))
        }
        WineTool::Winetricks => winetricks(ctx, probe, &wine, None, &mut notes)?,
    };
    Ok(ToolCommand { command, notes })
}

/// `Winetricks.install`: `winetricks -q <component>` without the GUI, in the
/// game's prefix.
pub fn winetricks_install_command(
    ctx: &PlanContext,
    probe: &dyn Probe,
    settings: &GameSettings,
    component: &str,
) -> Result<ToolCommand, String> {
    let wine = prepare(ctx, probe, settings)?;
    let mut notes = wine.notes.clone();
    let command = winetricks(ctx, probe, &wine, Some(component), &mut notes)?;
    Ok(ToolCommand { command, notes })
}

/// `runWineCommand({ commandParts, protonVerb: 'run' })` for the game's Wine
/// (used by the pre-launch preparation: DXVK overrides, `wineboot -u`).
pub fn wine_command(ctx: &PlanContext, probe: &dyn Probe, settings: &GameSettings, parts: Vec<String>) -> Result<Command, String> {
    wine_command_for(ctx, probe, settings, parts, None)
}

/// [`wine_command`] for a game: `gameInstallPath` also goes into the
/// environment (`STEAM_COMPAT_INSTALL_PATH`).
pub fn wine_command_for(
    ctx: &PlanContext,
    probe: &dyn Probe,
    settings: &GameSettings,
    parts: Vec<String>,
    install_path: Option<&str>,
) -> Result<Command, String> {
    let wine = prepare(ctx, probe, settings)?;
    Ok(run_in_prefix_at(ctx, probe, &wine, parts, None, install_path))
}

/// A Windows program run while the game runs (see [`crate::companions`]):
/// the game's Wine, prefix and environment. `beside_the_game`: the game itself
/// runs in that prefix, so the program joins it (`runinprefix`, and with umu
/// the game's container, see `UMU_CONTAINER_NSENTER`) instead of preparing
/// the prefix again.
pub fn alongside_command(
    ctx: &PlanContext,
    probe: &dyn Probe,
    app_name: &str,
    runner: &str,
    settings: &GameSettings,
    exe: &str,
    beside_the_game: bool,
) -> Result<Command, String> {
    let wine = prepare(ctx, probe, settings)?;
    let s = &wine.settings;
    let mut env = setup_wrapper_env_vars(app_name, runner);
    env.extend(setup_env_vars(s, None, probe));
    env.extend(setup_wine_env_vars(s, "0", &ctx.runtime_dir(), probe));
    quiet_wine_debug(&mut env, s);
    let verb = if beside_the_game { "runinprefix" } else { "run" };
    env.insert("PROTON_VERB".into(), verb.into());
    let mut args = Vec::new();
    match wine.runtime {
        WineRuntime::Umu => {
            // The game already brought the runtime up to date.
            env.insert("UMU_RUNTIME_UPDATE".into(), "0".into());
            if beside_the_game {
                env.insert(UMU_NSENTER.into(), "1".into());
            }
        }
        WineRuntime::Proton => args.push(verb.to_string()),
        WineRuntime::Wine => {}
    }
    args.push(exe.to_string());
    Ok(Command { program: wine.runner_bin(), args, env, cwd: Some(node_dirname(exe)) })
}

/// Set on a game run through umu, it starts a service in the game's container
/// that a second `umu-run` with the same prefix and this variable uses to run
/// its program inside that container (same wineserver), as `runinprefix`.
pub const UMU_NSENTER: &str = "UMU_CONTAINER_NSENTER";

/// `runWineCommand` with `protonVerb: 'run'`.
fn run_in_prefix(ctx: &PlanContext, probe: &dyn Probe, wine: &WineEnv, parts: Vec<String>, cwd: Option<String>) -> Command {
    run_in_prefix_at(ctx, probe, wine, parts, cwd, None)
}

fn run_in_prefix_at(
    ctx: &PlanContext,
    probe: &dyn Probe,
    wine: &WineEnv,
    mut parts: Vec<String>,
    cwd: Option<String>,
    install_path: Option<&str>,
) -> Command {
    let s = &wine.settings;
    let mut env = setup_env_vars(s, install_path, probe);
    env.extend(setup_wine_env_vars(s, "0", &ctx.runtime_dir(), probe));
    quiet_wine_debug(&mut env, s);
    env.insert("PROTON_VERB".into(), "run".into());
    if wine.runtime == WineRuntime::Proton {
        parts.insert(0, "run".into());
    }
    Command { program: wine.runner_bin(), args: parts, env, cwd }
}

/// Programs run in the prefix (installers, winecfg, winetricks, wineboot)
/// get no `WINEDEBUG=+fixme` from "Enable verbose logs" (games still do):
/// with it, repack installers fail while unpacking ("Unarc.dll returned an
/// error code: -2 / unsupported compression method") and the same setup
/// works without it. Proton then uses its own default (`-all`). A WINEDEBUG
/// the user set in the environment variables is kept.
fn quiet_wine_debug(env: &mut Env, s: &GameSettings) {
    if !s.env_options.iter().any(|(k, _)| k == "WINEDEBUG") {
        env.remove("WINEDEBUG");
    }
}

/// `getWineFromProton`: with plain Proton (no umu), winetricks needs the
/// `wine` inside Proton and the `pfx` folder of the prefix.
fn wine_from_proton(wine: &WineEnv, probe: &dyn Probe) -> (String, crate::settings::WineVersion) {
    let s = &wine.settings;
    if s.wine_version.kind != "proton" || wine.runtime == WineRuntime::Umu {
        return (s.wine_prefix.clone(), s.wine_version.clone());
    }
    let base = node_dirname(&s.wine_version.bin);
    for dist in ["dist", "files"] {
        let bin = join(&base, &format!("{dist}/bin/wine"));
        if !probe.exists(&bin) {
            continue;
        }
        let server = join(&base, &format!("{dist}/bin/wineserver"));
        let mut w = s.wine_version.clone();
        w.kind = "wine".into();
        w.bin = bin;
        w.wineserver = probe.exists(&server).then_some(server);
        return (join(&s.wine_prefix, "pfx"), w);
    }
    (s.wine_prefix.clone(), s.wine_version.clone())
}

/// `Winetricks.runWithArgs(['-q', '--gui'])`, or `['-q', component]` to
/// install `component` without the GUI.
fn winetricks(
    ctx: &PlanContext,
    probe: &dyn Probe,
    wine: &WineEnv,
    component: Option<&str>,
    notes: &mut Vec<String>,
) -> Result<Command, String> {
    let (prefix, wine_version) = wine_from_proton(wine, probe);
    if !valid_wine(&wine.settings.wine_version, probe) && !valid_wine(&wine_version, probe) {
        return Err(tr!("The configured Wine version is not valid").into());
    }
    let mut s = wine.settings.clone();
    s.wine_version = wine_version.clone();

    let mut env = setup_env_vars(&s, None, probe);
    env.extend(setup_wine_env_vars(&s, "0", &ctx.runtime_dir(), probe));
    quiet_wine_debug(&mut env, &s);
    env.insert("WINEPREFIX".into(), prefix);

    if wine.runtime == WineRuntime::Umu {
        // umu runs winetricks itself; an empty program opens its GUI.
        let umu = umu_path(ctx, probe);
        env.insert("UMU_RUNTIME_UPDATE".into(), "0".into());
        let args = match component {
            Some(c) => {
                env.insert("GAMEID".into(), "umu-0".into());
                vec!["winetricks".to_string(), c.to_string()]
            }
            None => {
                env.insert("GAMEID".into(), "winetricks-gui".into());
                vec![String::new()]
            }
        };
        return Ok(Command { program: umu, args, env, cwd: None });
    }

    let script = winetricks_path(ctx);
    if !probe.exists(&script) {
        return Err(tr!("winetricks was not found at {script}. It is downloaded with the Wine tools updates", script));
    }
    let wine_dir = node_dirname(&wine_version.bin);
    let path = probe.env_var("PATH").unwrap_or_default();
    env.insert("PATH".into(), format!("{wine_dir}:{path}"));
    env.insert("GAMEID".into(), "umu-0".into());
    env.insert("UMU_RUNTIME_UPDATE".into(), "0".into());
    notes.extend(missing_dependencies(probe));
    let args = match component {
        Some(c) => vec!["-q".to_string(), c.to_string()],
        None => vec!["-q".to_string(), "--gui".to_string()],
    };
    Ok(Command { program: script, args, env, cwd: None })
}

/// `Winetricks.checkDependencies`.
fn missing_dependencies(probe: &dyn Probe) -> Vec<String> {
    ["7z", "cabextract", "zenity", "unzip", "curl"]
        .iter()
        .filter(|d| probe.which(d).is_none())
        .map(|d| tr!("{dependency} is not installed; winetricks might fail to install some packages or even open", dependency = d))
        .collect()
}

/// The environment of `command` on top of the process one, as `KEY=value`
/// pairs (debug / log output).
pub fn display_env(env: &Env) -> Vec<String> {
    env.iter().map(|(k, v)| format!("{k}={v}")).collect()
}

/// Whether `path` looks like a Windows program the user may run in a prefix.
pub fn is_windows_program(path: &str) -> bool {
    let ext = Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    matches!(ext.as_str(), "exe" | "msi" | "bat" | "lnk" | "cmd")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emulators::EmulatorsFile;
    use crate::probe::FakeProbe;
    use crate::settings::{GlobalSettings, WineVersion};
    use heronite_data::DataPaths;

    fn ctx() -> PlanContext {
        PlanContext {
            paths: DataPaths::at("/cfg/heroic"),
            global: GlobalSettings::default(),
            emulators: EmulatorsFile::default(),
            ui_language: None,
            known_fixes: None,
            umu_ids: None,
        }
    }

    fn settings(kind: &str, bin: &str) -> GameSettings {
        GameSettings {
            wine_version: WineVersion { bin: bin.into(), name: "W".into(), kind: kind.into(), ..Default::default() },
            wine_prefix: "/pfx".into(),
            enable_esync: true,
            enable_fsync: true,
            ..Default::default()
        }
    }

    fn probe() -> FakeProbe {
        FakeProbe::new("/home/u")
            .with_path("/w/bin/wine")
            .with_path("/w/bin/wineserver")
            .with_path("/pfx")
            .with_path("/p/GE/proton")
            .with_path("/p/GE/files/bin/wine")
            .with_path("/p/GE/files/bin/wineserver")
            .with_path("/cfg/heroic/tools/winetricks")
            .with_path("/games/g.exe")
            .with_bin("umu-run", "/usr/bin/umu-run")
    }

    #[test]
    fn winecfg_with_wine_runs_winecfg_in_the_prefix() {
        let t = build_tool_command(&ctx(), &probe(), &settings("wine", "/w/bin/wine"), &WineTool::Winecfg).unwrap();
        assert_eq!(t.command.program, "/w/bin/wine");
        assert_eq!(t.command.args, ["winecfg"]);
        assert_eq!(t.command.env["WINEPREFIX"], "/pfx");
        assert_eq!(t.command.env["WINEESYNC"], "1");
        assert_eq!(t.command.env["PROTON_VERB"], "run");
        assert_eq!(t.command.cwd, None);
        assert!(t.notes.is_empty());
    }

    #[test]
    fn run_exe_uses_the_program_folder_and_proton_run_verb() {
        let t = build_tool_command(
            &ctx(),
            &FakeProbe::new("/home/u").with_path("/p/GE/proton").with_path("/games/g.exe").with_path("/pfx"),
            &{
                let mut s = settings("proton", "/p/GE/proton");
                s.disable_umu = Some(true);
                s
            },
            &WineTool::RunExe("/games/g.exe".into()),
        )
        .unwrap();
        assert_eq!(t.command.program, "/p/GE/proton");
        assert_eq!(t.command.args, ["run", "/games/g.exe"]);
        assert_eq!(t.command.cwd.as_deref(), Some("/games"));
        assert_eq!(t.command.env["STEAM_COMPAT_DATA_PATH"], "/pfx");
        assert_eq!(t.command.env["PROTON_VERB"], "run");
    }

    #[test]
    fn verbose_logs_do_not_reach_programs_run_in_the_prefix() {
        let mut s = settings("proton", "/p/GE/proton");
        s.verbose_logs = true;
        let run = |s: &GameSettings| build_tool_command(&ctx(), &probe(), s, &WineTool::RunExe("/games/g.exe".into())).unwrap();
        assert!(!run(&s).command.env.contains_key("WINEDEBUG"));
        // The user's own value stays.
        s.env_options.push(("WINEDEBUG".into(), "+relay".into()));
        assert_eq!(run(&s).command.env["WINEDEBUG"], "+relay");
    }

    #[test]
    fn umu_is_preferred_for_proton() {
        let t =
            build_tool_command(&ctx(), &probe(), &settings("proton", "/p/GE/proton"), &WineTool::RunExe("/games/g.exe".into())).unwrap();
        assert_eq!(t.command.program, "/usr/bin/umu-run");
        assert_eq!(t.command.args, ["/games/g.exe"]);
        assert_eq!(t.command.env["PROTONPATH"], "/p/GE");
    }

    #[test]
    fn run_exe_validates_the_program() {
        let s = settings("wine", "/w/bin/wine");
        assert!(build_tool_command(&ctx(), &probe(), &s, &WineTool::RunExe("".into())).is_err());
        let e = build_tool_command(&ctx(), &probe(), &s, &WineTool::RunExe("/nope.exe".into())).unwrap_err();
        assert!(e.contains("not found"), "{e}");
    }

    #[test]
    fn invalid_wine_is_reported() {
        let e = build_tool_command(&ctx(), &probe(), &settings("wine", "/missing/wine"), &WineTool::Winecfg).unwrap_err();
        assert!(e.contains("not valid"), "{e}");
    }

    #[test]
    fn missing_prefix_is_noted_not_fatal() {
        let p = FakeProbe::new("/h").with_path("/w/bin/wine").with_path("/w/bin/wineserver");
        let t = build_tool_command(&ctx(), &p, &settings("wine", "/w/bin/wine"), &WineTool::Winecfg).unwrap();
        assert!(t.notes.iter().any(|n| n.contains("does not exist yet")));
    }

    #[test]
    fn winetricks_with_wine_uses_the_script_and_wine_on_path() {
        let probe = probe().with_env("PATH", "/usr/bin");
        let t = build_tool_command(&ctx(), &probe, &settings("wine", "/w/bin/wine"), &WineTool::Winetricks).unwrap();
        assert_eq!(t.command.program, "/cfg/heroic/tools/winetricks");
        assert_eq!(t.command.args, ["-q", "--gui"]);
        assert_eq!(t.command.env["WINEPREFIX"], "/pfx");
        assert_eq!(t.command.env["PATH"], "/w/bin:/usr/bin");
        assert_eq!(t.command.env["GAMEID"], "umu-0");
        assert_eq!(t.command.env["UMU_RUNTIME_UPDATE"], "0");
        assert!(t.notes.iter().any(|n| n.starts_with("zenity")));
    }

    #[test]
    fn winetricks_with_proton_but_no_umu_uses_the_wine_inside_proton() {
        let mut s = settings("proton", "/p/GE/proton");
        s.disable_umu = Some(true);
        let t = build_tool_command(&ctx(), &probe().with_env("PATH", "/usr/bin"), &s, &WineTool::Winetricks).unwrap();
        assert_eq!(t.command.program, "/cfg/heroic/tools/winetricks");
        assert_eq!(t.command.env["WINEPREFIX"], "/pfx/pfx");
        assert_eq!(t.command.env["PATH"], "/p/GE/files/bin:/usr/bin");
        // It is Wine now, so the Wine style sync variables apply.
        assert_eq!(t.command.env["WINEESYNC"], "1");
        assert!(!t.command.env.contains_key("PROTON_NO_ESYNC"));
    }

    #[test]
    fn winetricks_with_umu_opens_the_gui_through_umu() {
        let t = build_tool_command(&ctx(), &probe(), &settings("proton", "/p/GE/proton"), &WineTool::Winetricks).unwrap();
        assert_eq!(t.command.program, "/usr/bin/umu-run");
        assert_eq!(t.command.args, [""]);
        assert_eq!(t.command.env["GAMEID"], "winetricks-gui");
        assert_eq!(t.command.env["WINEPREFIX"], "/pfx");
    }

    #[test]
    fn winetricks_install_runs_the_component_without_the_gui() {
        let probe = probe().with_env("PATH", "/usr/bin");
        let t = winetricks_install_command(&ctx(), &probe, &settings("wine", "/w/bin/wine"), "vcrun2022").unwrap();
        assert_eq!(t.command.program, "/cfg/heroic/tools/winetricks");
        assert_eq!(t.command.args, ["-q", "vcrun2022"]);
        assert_eq!(t.command.env["WINEPREFIX"], "/pfx");
        assert_eq!(t.command.env["GAMEID"], "umu-0");

        // With umu the script is umu's own `winetricks` verb and `-q` is dropped.
        let t = winetricks_install_command(&ctx(), &probe, &settings("proton", "/p/GE/proton"), "vcrun2022").unwrap();
        assert_eq!(t.command.program, "/usr/bin/umu-run");
        assert_eq!(t.command.args, ["winetricks", "vcrun2022"]);
        assert_eq!(t.command.env["GAMEID"], "umu-0");
        assert_eq!(t.command.env["UMU_RUNTIME_UPDATE"], "0");
    }

    #[test]
    fn wine_command_for_a_game_exports_its_install_path() {
        let s = settings("wine", "/w/bin/wine");
        let cmd = wine_command_for(&ctx(), &probe(), &s, vec!["x.exe".into()], Some("/games/g")).unwrap();
        assert_eq!(cmd.env["STEAM_COMPAT_INSTALL_PATH"], "/games/g");
        assert_eq!(cmd.args, ["x.exe"]);
        let plain = wine_command(&ctx(), &probe(), &s, vec!["x.exe".into()]).unwrap();
        assert!(!plain.env.contains_key("STEAM_COMPAT_INSTALL_PATH"));
    }

    #[test]
    fn winetricks_script_missing_is_an_error() {
        let p = FakeProbe::new("/h").with_path("/w/bin/wine").with_path("/w/bin/wineserver").with_path("/pfx");
        let e = build_tool_command(&ctx(), &p, &settings("wine", "/w/bin/wine"), &WineTool::Winetricks).unwrap_err();
        assert!(e.contains("winetricks was not found"), "{e}");
    }

    #[test]
    fn windows_program_extensions() {
        assert!(is_windows_program("/a/Setup.EXE"));
        assert!(!is_windows_program("/a/readme.txt"));
    }
}
