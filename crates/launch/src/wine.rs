//! Windows programs: Wine/Proton/umu command assembly, ported from
//! `runWineCommand`, `verifyWinePrefix` and `shutdownWine` in
//! `backend/launcher.ts`.

use heronite_data::models::GameInfo;
use heronite_i18n::tr;

use crate::emulators::{node_basename, node_dirname};
use crate::env::{build_wrappers, join, setup_env_vars, setup_wine_env_vars, setup_wrapper_env_vars, Env};
use crate::plan::{Command, LaunchPlan, PlanContext, PrepSpec, ProcessPlan, StopSpec, WineRuntime};
use crate::probe::Probe;
use crate::settings::{GameSettings, WineVersion};

/// Heroic's `getUmuPath`: `umu-run` on `$PATH`, else its bundled copy.
pub fn umu_path(ctx: &PlanContext, probe: &dyn Probe) -> String {
    probe.which("umu-run").unwrap_or_else(|| join(&join(&ctx.tools_dir(), "runtimes"), "umu/umu_run.py"))
}

/// `wineserver` next to the Wine binary (or the one stored in the config).
pub fn wine_server_of(w: &WineVersion, probe: &dyn Probe) -> Option<String> {
    if let Some(s) = w.wineserver.as_ref().filter(|s| probe.exists(s)) {
        return Some(s.clone());
    }
    let candidate = join(&node_dirname(&w.bin), "wineserver");
    probe.exists(&candidate).then_some(candidate)
}

/// Heroic's `validWine`: Wine needs `wine` and `wineserver`; others the binary.
pub fn valid_wine(w: &WineVersion, probe: &dyn Probe) -> bool {
    if w.bin.is_empty() || !probe.exists(&w.bin) {
        return false;
    }
    match w.kind.as_str() {
        "wine" => wine_server_of(w, probe).is_some(),
        "proton" => true,
        _ => false,
    }
}

/// The error of [`prepare`] when no Wine/Proton can run the game.
pub fn no_wine_message(name: &str) -> String {
    tr!("Wine version '{name}' is not valid and no valid default Wine/Proton is configured", name)
}

/// Whether a launch failed with [`no_wine_message`] (in the current language).
pub fn is_no_wine_error(reason: &str) -> bool {
    let template = no_wine_message("\u{1}");
    let (pre, post) = template.split_once('\u{1}').unwrap_or((&template, ""));
    reason.len() >= pre.len() + post.len() && reason.starts_with(pre) && reason.ends_with(post)
}

/// Settings with a usable Wine version, an explanation if it had to change.
pub struct WineEnv {
    pub settings: GameSettings,
    pub runtime: WineRuntime,
    pub umu: Option<String>,
    /// `umu` is used but not installed yet: the launch downloads it first.
    pub umu_download: bool,
    pub server: Option<String>,
    pub notes: Vec<String>,
}

/// Picks the game's Wine, falling back to the global default like
/// `checkWineBeforeLaunch` (minus its dialog). Err = nothing usable.
pub fn prepare(ctx: &PlanContext, probe: &dyn Probe, settings: &GameSettings) -> Result<WineEnv, String> {
    prepare_with(ctx, probe, settings, false)
}

/// Like [`prepare`] for an actual launch: a missing umu runtime is planned
/// for (the launch downloads it, as Heroic does) instead of falling back to
/// running Proton directly.
pub fn prepare_for_launch(ctx: &PlanContext, probe: &dyn Probe, settings: &GameSettings) -> Result<WineEnv, String> {
    prepare_with(ctx, probe, settings, true)
}

fn prepare_with(ctx: &PlanContext, probe: &dyn Probe, settings: &GameSettings, download_umu: bool) -> Result<WineEnv, String> {
    let mut settings = settings.clone();
    let mut notes = Vec::new();
    if !valid_wine(&settings.wine_version, probe) {
        let default = ctx.global.wine_version();
        if valid_wine(&default, probe) {
            notes.push(tr!(
                "Wine version '{name}' is not valid; using the default '{default}'",
                name = settings.wine_version.name,
                default = default.name
            ));
            settings.wine_version = default;
        } else {
            return Err(no_wine_message(&settings.wine_version.name));
        }
    }
    let kind = settings.wine_version.kind.clone();
    let umu_file = umu_path(ctx, probe);
    let wants_umu = kind == "proton" && settings.disable_umu != Some(true);
    let umu_present = probe.exists(&umu_file);
    let umu_download = wants_umu && !umu_present && download_umu;
    let umu_ok = wants_umu && (umu_present || download_umu);
    if umu_download {
        notes.push(tr!("umu-run was not found; it will be downloaded").to_string());
    } else if wants_umu && !umu_ok {
        notes.push(tr!("umu-run was not found; running Proton directly").to_string());
    }
    let runtime = if umu_ok {
        WineRuntime::Umu
    } else if kind == "proton" {
        WineRuntime::Proton
    } else {
        WineRuntime::Wine
    };
    let server = if kind == "wine" { wine_server_of(&settings.wine_version, probe) } else { None };
    Ok(WineEnv { settings, runtime, umu: umu_ok.then_some(umu_file), umu_download, server, notes })
}

impl WineEnv {
    /// What the launch must do around this Wine setup (see [`PrepSpec`]).
    pub(crate) fn prep_spec(&self) -> PrepSpec {
        PrepSpec { umu: self.umu_download, wine: Some(self.settings.clone()), cloud_saves: None, ..Default::default() }
    }

    /// Binary that runs Windows programs: umu, proton or wine.
    pub fn runner_bin(&self) -> String {
        match &self.umu {
            Some(u) => u.clone(),
            None => self.settings.wine_version.bin.clone(),
        }
    }

    /// `shutdownWine`: kills everything in the prefix.
    pub fn shutdown_command(&self, env: &Env) -> Command {
        if let Some(server) = &self.server {
            let mut e = Env::new();
            e.insert("WINEPREFIX".into(), self.settings.wine_prefix.clone());
            return Command { program: server.clone(), args: vec!["-k".into()], env: e, cwd: None };
        }
        let mut e = env.clone();
        e.insert("PROTON_VERB".into(), "run".into());
        let mut args = Vec::new();
        if self.runtime == WineRuntime::Proton {
            args.push("run".to_string());
        }
        args.extend(["wineboot".to_string(), "-k".to_string()]);
        Command { program: self.runner_bin(), args, env: e, cwd: None }
    }

    /// `verifyWinePrefix`: files that must exist for the prefix to be usable.
    pub(crate) fn prefix_missing(&self, probe: &dyn Probe) -> bool {
        let prefix = &self.settings.wine_prefix;
        let mut required: Vec<String> =
            ["dosdevices", "drive_c", "system.reg", "user.reg", "userdef.reg"].iter().map(|s| s.to_string()).collect();
        let mut files = vec![prefix.clone()];
        if self.settings.wine_version.kind == "proton" {
            required = required.into_iter().map(|p| format!("pfx/{p}")).collect();
            for extra in ["pfx.lock", "tracked_files", "version", "config_info"] {
                files.push(join(prefix, extra));
            }
        }
        files.extend(required.iter().map(|p| join(prefix, p)));
        !files.iter().all(|f| probe.exists(f))
    }

    /// Commands creating the prefix when it is missing.
    pub(crate) fn prefix_creation(&self, env: &Env) -> Vec<Command> {
        let mut e = env.clone();
        e.insert("PROTON_VERB".into(), "run".into());
        let bin = self.runner_bin();
        let main = match self.runtime {
            WineRuntime::Umu => Command { program: bin, args: vec!["createprefix".into()], env: e.clone(), cwd: None },
            WineRuntime::Proton => {
                Command { program: bin, args: vec!["run".into(), "wineboot".into(), "--init".into()], env: e.clone(), cwd: None }
            }
            WineRuntime::Wine => Command { program: bin, args: vec!["wineboot".into(), "--init".into()], env: e.clone(), cwd: None },
        };
        let mut out = vec![main];
        if let Some(server) = &self.server {
            out.push(Command { program: server.clone(), args: vec!["--wait".into()], env: e, cwd: None });
        }
        out
    }
}

/// Plan for a Windows program (also used for `.exe` emulators).
pub fn build_wine_plan(
    ctx: &PlanContext,
    probe: &dyn Probe,
    game: &GameInfo,
    settings: &GameSettings,
    executable: &str,
    args: &[String],
) -> LaunchPlan {
    if executable.starts_with('/') && !probe.exists(executable) {
        return LaunchPlan::Unavailable(tr!("Executable not found: {executable}", executable));
    }
    let wine = match prepare_for_launch(ctx, probe, settings) {
        Ok(w) => w,
        Err(e) => return LaunchPlan::Unavailable(e),
    };
    let s = &wine.settings;
    let wp = build_wrappers(s, probe, None);
    if let Some(err) = wp.error {
        return LaunchPlan::Unavailable(tr!("Launch aborted: {error}", error = err));
    }

    let mut env = setup_wrapper_env_vars(&game.app_name, &game.runner);
    env.extend(setup_env_vars(s, None, probe));
    env.extend(setup_wine_env_vars(s, "0", &ctx.runtime_dir(), probe));
    env.insert("PROTON_VERB".into(), "waitforexitandrun".into());

    let runner_bin = wine.runner_bin();
    let mut parts: Vec<String> = Vec::new();
    if wine.runtime == WineRuntime::Proton {
        parts.push("waitforexitandrun".into());
    }
    parts.push(executable.to_string());
    parts.extend(args.iter().cloned());

    let mut full = wp.wrappers;
    full.push(runner_bin);
    full.extend(parts);
    let program = full.remove(0);

    let (create_dirs, pre_commands) =
        if wine.prefix_missing(probe) { (vec![s.wine_prefix.clone()], wine.prefix_creation(&env)) } else { (vec![], vec![]) };
    let wait_after_exit = wine.server.as_ref().map(|server| Command {
        program: server.clone(),
        args: vec!["--wait".into()],
        env: env.clone(),
        cwd: Some(node_dirname(executable)),
    });
    let stop = StopSpec { kill_patterns: vec![node_basename(executable)], commands: vec![wine.shutdown_command(&env)] };

    LaunchPlan::Wine {
        runtime: wine.runtime,
        prefix: s.wine_prefix.clone(),
        process: ProcessPlan {
            command: Command { program, args: full, env, cwd: Some(node_dirname(executable)) },
            stop,
            create_dirs,
            pre_commands,
            wait_after_exit,
            make_executable: None,
            prep: wine.prep_spec(),
            notes: wine.notes,
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{is_no_wine_error, no_wine_message};

    #[test]
    fn the_no_wine_message_is_recognised() {
        assert!(is_no_wine_error(&no_wine_message("GE-Proton9-1")));
        assert!(is_no_wine_error(&no_wine_message("")));
        assert!(!is_no_wine_error("Steam client was not found on PATH"));
    }
}
