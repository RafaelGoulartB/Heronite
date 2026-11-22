//! AntiMicroX profiles of a game (`heroniteAntimicrox`, a Heronite key of
//! the game's settings): one `.amgp` file per player, loaded on that
//! player's controller while the game runs.
//!
//! Every profile gets its own `antimicrox --profile <file>
//! --profile-controller <n>`. AntiMicroX's `--next`, meant for several at
//! once, reads every option with `QCommandLineParser::value`, which gives
//! the last one passed: every controller would get the last file.
//! AntiMicroX runs once per session: a second start passes its options to
//! the running one and exits. So the first command starts AntiMicroX
//! without its window and the next ones, [`STEP`] apart so that it is
//! listening by then, hand it their profiles.
//!
//! When the game ends Heronite closes the AntiMicroX it started. When
//! AntiMicroX was already running, the commands handed their profiles over
//! and exited: `antimicrox --unload`, once, then takes the game's profiles
//! off the running one, which stays. Every command carries that unload, so
//! it runs even when the first one could not; the runtime skips it while an
//! AntiMicroX Heronite started is running or after one crashed (nothing was
//! handed over then). It is `--hidden`, so that if it ever finds none
//! running it starts one without its window.
//!
//! It runs on the host, never in the game's prefix or sandbox: it needs the
//! controllers and `/dev/uinput`.

use std::time::Duration;

use heronite_data::models::GameInfo;
use serde_json::Value;

use crate::plan::{Command, CompanionPlan, ProcessPlan};
use crate::probe::Probe;
use crate::settings::GameSettings;

pub const KEY: &str = "heroniteAntimicrox";
pub const FLATPAK_ID: &str = "io.github.antimicrox.antimicrox";
/// Time between one player's profile and the next.
pub const STEP: Duration = Duration::from_secs(3);

/// The profiles of the setting (`["/p/player1.amgp", "/p/player2.amgp"]`),
/// player 1 first; empty entries are skipped.
pub fn parse(v: Option<&Value>) -> Vec<String> {
    let Some(Value::Array(list)) = v else { return Vec::new() };
    list.iter().filter_map(Value::as_str).map(str::trim).filter(|p| !p.is_empty()).map(String::from).collect()
}

pub fn to_value(profiles: &[String]) -> Value {
    Value::Array(profiles.iter().cloned().map(Value::String).collect())
}

/// Whether `path` is an AntiMicroX profile (`.amgp`), not a program.
pub fn is_profile(path: &str) -> bool {
    path.trim().to_ascii_lowercase().ends_with(".amgp")
}

/// How AntiMicroX starts here: its program and the arguments that go
/// before its own (see [`find_in`]).
pub fn find(probe: &dyn Probe) -> Option<(String, Vec<String>)> {
    find_in(probe, heronite_platform::flatpak::active())
}

/// The one on the `PATH` first, then its Flatpak, then the folder its
/// Windows installer uses. `in_flatpak`: Heronite runs as a Flatpak, which
/// sees neither the host's `PATH` nor its Flatpaks: the AntiMicroX Flatpak
/// is asked for anyway, and one that is not installed fails to start like
/// any missing program.
fn find_in(probe: &dyn Probe, in_flatpak: bool) -> Option<(String, Vec<String>)> {
    if let Some(bin) = probe.which("antimicrox") {
        return Some((bin, Vec::new()));
    }
    if cfg!(windows) {
        return ["ProgramFiles", "ProgramFiles(x86)"]
            .iter()
            .filter_map(|var| probe.env_var(var))
            .map(|dir| format!("{dir}\\AntiMicroX\\bin\\antimicrox.exe"))
            .find(|exe| probe.exists(exe))
            .map(|exe| (exe, Vec::new()));
    }
    let home = probe.home();
    let exports = [format!("{home}/.local/share/flatpak/exports/bin/{FLATPAK_ID}"), format!("/var/lib/flatpak/exports/bin/{FLATPAK_ID}")];
    // `flatpak` by name: the spawner runs it on the host from Heronite's own Flatpak.
    (in_flatpak || exports.iter().any(|p| probe.exists(p))).then(|| ("flatpak".to_string(), vec!["run".into(), FLATPAK_ID.into()]))
}

/// The programs that load `profiles`, one per player (see the module).
/// `Err`: AntiMicroX is not installed.
pub fn plans(probe: &dyn Probe, profiles: &[String]) -> Result<Vec<CompanionPlan>, String> {
    plans_with(find(probe), profiles)
}

fn plans_with(antimicrox: Option<(String, Vec<String>)>, profiles: &[String]) -> Result<Vec<CompanionPlan>, String> {
    if profiles.is_empty() {
        return Ok(Vec::new());
    }
    let (program, before) = antimicrox.ok_or("AntiMicroX was not found")?;
    let command = |args: &[&str]| Command {
        program: program.clone(),
        args: before.iter().cloned().chain(args.iter().map(|a| a.to_string())).collect(),
        ..Default::default()
    };
    Ok(profiles
        .iter()
        .enumerate()
        .map(|(i, profile)| {
            let player = (i + 1).to_string();
            CompanionPlan {
                path: profile.clone(),
                // `--hidden` on every one: a later one that finds no
                // AntiMicroX running starts it, still without its window.
                command: command(&["--hidden", "--profile", profile, "--profile-controller", &player]),
                delay: STEP * i as u32,
                // The same on every one: the runtime runs it once, and it
                // takes off every controller's profile.
                undo: Some(command(&["--hidden", "--unload"])),
                on_host: true,
            }
        })
        .collect())
}

/// Folders a game in the sandbox can write to, as `sandbox::mounts` gives
/// them: a profile there is left out. The game could change it, and
/// AntiMicroX, on the host, runs the programs a profile names. The game's
/// folder is the sandbox's (`wine::game_folder`), for its executable and
/// for the alternative one.
fn sandbox_writable(game: &GameInfo, settings: &GameSettings) -> Vec<String> {
    let mut dirs = vec![settings.wine_prefix.clone()];
    if settings.sandbox.game_folder {
        for exe in [game.install.executable.as_deref().unwrap_or_default(), &settings.target_exe] {
            dirs.push(crate::wine::game_folder(game, exe));
        }
    }
    if settings.sandbox.shared_folders {
        dirs.extend(settings.shared_folders.iter().cloned());
    }
    dirs.iter().filter_map(|d| crate::sandbox::clean(d)).collect()
}

/// Why a sandboxed game gets no `profile`, if it does not: a profile it
/// could change, or a path that cannot be told (relative, or with `..`).
fn sandbox_reason(profile: &str, writable: &[String]) -> Option<String> {
    let Some(path) = crate::sandbox::clean(profile) else {
        return Some(format!("{profile} will not load: the game runs in the sandbox and the path is not a plain absolute one"));
    };
    let dir = writable.iter().find(|dir| crate::sandbox::inside(&path, dir))?;
    Some(format!("{profile} will not load: the game's sandbox can change files in {dir}"))
}

/// Adds the game's profiles to the programs that run alongside it. Without
/// AntiMicroX, or for a profile the sandboxed game could change, the log
/// says why they are left out.
pub fn attach(process: &mut ProcessPlan, probe: &dyn Probe, game: &GameInfo, settings: &GameSettings) {
    let plans = match plans(probe, &settings.antimicrox) {
        Ok(plans) => plans,
        Err(e) => return process.notes.push(format!("The AntiMicroX profiles will not load: {e}")),
    };
    let writable = if process.sandboxed { Some(sandbox_writable(game, settings)) } else { None };
    for plan in plans {
        match writable.as_deref().and_then(|w| sandbox_reason(&plan.path, w)) {
            Some(why) => process.notes.push(why),
            None => process.companions.push(plan),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::FakeProbe;
    use serde_json::json;

    fn profiles(list: &[&str]) -> Vec<String> {
        list.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn parses_and_writes_the_setting() {
        let list = parse(Some(&json!([" /p/one.amgp ", "", 3, "/p/two.amgp"])));
        assert_eq!(list, profiles(&["/p/one.amgp", "/p/two.amgp"]));
        assert_eq!(parse(Some(&to_value(&list))), list);
        assert!(parse(None).is_empty() && parse(Some(&json!("nope"))).is_empty());
    }

    #[test]
    fn a_profile_is_told_from_a_program_by_its_extension() {
        assert!(is_profile("/p/Player One.AMGP") && is_profile("/p/one.amgp "));
        assert!(!is_profile("/t/trainer.exe") && !is_profile("/t/amgp"));
    }

    #[test]
    fn each_player_gets_a_profile_on_their_controller_a_step_apart() {
        let probe = FakeProbe::new("/home/u").with_bin("antimicrox", "/usr/bin/antimicrox");
        let plans = plans(&probe, &profiles(&["/p/one.amgp", "/p/two.amgp", "/p/three.amgp"])).unwrap();
        let [one, two, three] = &plans[..] else { panic!("{plans:?}") };
        assert_eq!(one.command.program, "/usr/bin/antimicrox");
        assert_eq!(one.command.args, ["--hidden", "--profile", "/p/one.amgp", "--profile-controller", "1"]);
        assert_eq!(three.command.args, ["--hidden", "--profile", "/p/three.amgp", "--profile-controller", "3"]);
        assert_eq!([one.delay, two.delay, three.delay], [Duration::ZERO, STEP, STEP * 2]);
        assert_eq!(one.path, "/p/one.amgp");
        assert_eq!(one.undo.as_ref().unwrap().args, ["--hidden", "--unload"]);
        assert!(plans.iter().all(|p| p.undo == one.undo), "the same unload on each, run once");
        assert!(plans.iter().all(|p| p.on_host));
    }

    #[cfg(unix)]
    #[test]
    fn the_flatpak_runs_when_none_is_on_the_path() {
        let probe = FakeProbe::new("/home/u").with_path(&format!("/home/u/.local/share/flatpak/exports/bin/{FLATPAK_ID}"));
        let plans = plans(&probe, &profiles(&["/p/one.amgp"])).unwrap();
        assert_eq!(plans[0].command.program, "flatpak");
        assert_eq!(plans[0].command.args[..3], ["run", FLATPAK_ID, "--hidden"]);
        assert_eq!(plans[0].undo.as_ref().unwrap().args, ["run", FLATPAK_ID, "--hidden", "--unload"]);
    }

    #[cfg(unix)]
    #[test]
    fn heronites_own_flatpak_asks_for_the_antimicrox_flatpak_it_cannot_see() {
        let probe = FakeProbe::new("/home/u");
        assert_eq!(find_in(&probe, false), None);
        assert_eq!(find_in(&probe, true), Some(("flatpak".into(), vec!["run".into(), FLATPAK_ID.into()])));
        let native = probe.with_bin("antimicrox", "/usr/bin/antimicrox");
        assert_eq!(find_in(&native, true), Some(("/usr/bin/antimicrox".into(), vec![])), "one on the PATH still comes first");
    }

    #[test]
    fn without_antimicrox_the_profiles_cannot_load() {
        assert!(plans_with(None, &profiles(&["/p/one.amgp"])).is_err());
        assert_eq!(plans_with(None, &[]), Ok(Vec::new()), "no profiles: nothing to find");
    }

    #[test]
    fn a_sandboxed_game_gets_no_profile_it_could_change() {
        let probe = FakeProbe::new("/home/u").with_bin("antimicrox", "/usr/bin/antimicrox");
        let game: GameInfo = serde_json::from_value(json!({
            "runner": "sideload", "app_name": "g1", "title": "Game One", "is_installed": true,
            "install": {"executable": "/games/one/game", "platform": "linux", "install_path": "/games/one"}
        }))
        .unwrap();
        let settings = |game_folder: bool| GameSettings {
            antimicrox: profiles(&["/games/one/p1.amgp", "/pfx/p2.amgp", "/home/u/p3.amgp"]),
            wine_prefix: "/pfx".into(),
            sandbox: crate::sandbox::Sandbox { enabled: true, game_folder, ..Default::default() },
            ..Default::default()
        };
        let attached = |sandboxed: bool, game_folder: bool| {
            let mut process = ProcessPlan { sandboxed, ..Default::default() };
            attach(&mut process, &probe, &game, &settings(game_folder));
            let paths: Vec<String> = process.companions.iter().map(|c| c.path.clone()).collect();
            (paths, process.notes)
        };
        let (paths, notes) = attached(true, true);
        assert_eq!(paths, ["/home/u/p3.amgp"]);
        assert_eq!(notes.len(), 2, "{notes:?}");
        let (paths, _) = attached(true, false);
        assert_eq!(paths, ["/games/one/p1.amgp", "/home/u/p3.amgp"], "a read-only game folder is fine");
        let (paths, notes) = attached(false, true);
        assert_eq!(paths.len(), 3, "outside the sandbox every profile loads");
        assert!(notes.is_empty());
        // The players keep their controllers when one is left out.
        let mut process = ProcessPlan { sandboxed: true, ..Default::default() };
        attach(&mut process, &probe, &game, &settings(true));
        assert_eq!(process.companions[0].command.args.last().map(String::as_str), Some("3"));
    }

    #[test]
    fn a_sideloaded_game_without_an_install_path_shares_its_executables_folder() {
        let probe = FakeProbe::new("/home/u").with_bin("antimicrox", "/usr/bin/antimicrox");
        let game: GameInfo = serde_json::from_value(json!({
            "runner": "sideload", "app_name": "g2", "title": "Game Two", "is_installed": true,
            "install": {"executable": "/games/two/bin/game", "platform": "linux"}
        }))
        .unwrap();
        let settings = GameSettings {
            antimicrox: profiles(&["/games/two/bin/p1.amgp", "/games/two/bin/../../two/bin/p2.amgp", "/home/u/p3.amgp"]),
            sandbox: crate::sandbox::Sandbox { enabled: true, game_folder: true, ..Default::default() },
            ..Default::default()
        };
        let mut process = ProcessPlan { sandboxed: true, ..Default::default() };
        attach(&mut process, &probe, &game, &settings);
        let paths: Vec<&str> = process.companions.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths, ["/home/u/p3.amgp"], "{:?}", process.notes);
        assert_eq!(process.notes.len(), 2);
    }
}
