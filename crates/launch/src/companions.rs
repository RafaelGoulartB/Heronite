//! "Programs alongside the game" (`heroniteCompanions`, a Heronite key of
//! the game's settings): programs started a while after the game and closed
//! with it, such as trainers and mod tools.
//!
//! A trainer has to see the game's processes, so a Windows program runs in
//! the game's prefix with the game's Wine. With Proton it uses `runinprefix`,
//! which joins the running prefix instead of preparing it again. With umu the
//! game also gets `UMU_CONTAINER_NSENTER`, so the program runs inside the
//! game's container and its wineserver; the container ends with the game, and
//! so does the program. Linux programs run directly.
//!
//! Only builds the commands; the runtime starts and stops them.

use std::time::Duration;

use heronite_data::models::GameInfo;
use serde_json::{json, Value};

use crate::emulators::node_dirname;
use crate::native::{detect_executable_kind, ExecutableKind};
use crate::plan::{Command, CompanionPlan, LaunchPlan, PlanContext, WineRuntime};
use crate::probe::Probe;
use crate::scripts::game_env;
use crate::settings::GameSettings;
use crate::wine_tools::{alongside_command, UMU_NSENTER};

pub const KEY: &str = "heroniteCompanions";
/// Longest wait before a program starts, in seconds.
pub const MAX_DELAY_SECS: u32 = 600;

/// One program of the setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Companion {
    pub path: String,
    /// Seconds after the game started, `0..=MAX_DELAY_SECS`.
    pub delay_secs: u32,
}

/// The programs of the setting (`[{"path": "...", "delay": 10}]`), in order;
/// entries without a path are skipped.
pub fn parse(v: Option<&Value>) -> Vec<Companion> {
    let Some(Value::Array(list)) = v else { return Vec::new() };
    list.iter()
        .filter_map(|item| {
            let path = item.get("path")?.as_str()?.trim();
            if path.is_empty() {
                return None;
            }
            let delay = item.get("delay").and_then(Value::as_u64).unwrap_or(0).min(MAX_DELAY_SECS as u64) as u32;
            Some(Companion { path: path.to_string(), delay_secs: delay })
        })
        .collect()
}

pub fn to_value(list: &[Companion]) -> Value {
    Value::Array(list.iter().map(|c| json!({ "path": c.path, "delay": c.delay_secs })).collect())
}

/// Adds the game's programs to a plan that spawns a process. A Windows
/// program that cannot get a Wine is left out with a note in the log.
pub fn attach(plan: &mut LaunchPlan, ctx: &PlanContext, probe: &dyn Probe, game: &GameInfo, settings: &GameSettings) {
    let umu_game = matches!(plan, LaunchPlan::Wine { runtime: WineRuntime::Umu, .. });
    let Some(process) = plan.process_mut() else { return };
    if settings.companions.is_empty() {
        return;
    }
    // A game in a Wine prefix (Windows games, `.exe` emulators, Epic through Wine).
    let in_prefix = process.prep.wine.is_some();
    let mut joins_container = false;
    for c in &settings.companions {
        let command = match detect_executable_kind(&c.path, probe) {
            ExecutableKind::Windows => match alongside_command(ctx, probe, &game.app_name, &game.runner, settings, &c.path, in_prefix) {
                Ok(cmd) => {
                    joins_container |= cmd.env.contains_key(UMU_NSENTER);
                    cmd
                }
                Err(e) => {
                    process.notes.push(format!("{} will not start alongside the game: {e}", c.path));
                    continue;
                }
            },
            _ => Command { program: c.path.clone(), args: vec![], env: game_env(game, settings, &[]), cwd: Some(node_dirname(&c.path)) },
        };
        process.companions.push(CompanionPlan { path: c.path.clone(), command, delay: Duration::from_secs(c.delay_secs.into()) });
    }
    // Legendary runs umu itself as a wrapper and passes its environment on.
    if joins_container && (umu_game || matches!(plan, LaunchPlan::Legendary { .. })) {
        if let Some(process) = plan.process_mut() {
            process.command.env.insert(UMU_NSENTER.into(), "1".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{build_plan_with_settings, LaunchOptions};
    use crate::probe::FakeProbe;
    use heronite_data::DataPaths;

    #[test]
    fn parses_and_writes_the_setting() {
        let v = json!([{"path": " /t/trainer.exe ", "delay": 15}, {"path": "/t/tool.sh"}, {"path": ""}, {"delay": 3}, {"path": "/x", "delay": 99999}]);
        let list = parse(Some(&v));
        assert_eq!(
            list,
            [
                Companion { path: "/t/trainer.exe".into(), delay_secs: 15 },
                Companion { path: "/t/tool.sh".into(), delay_secs: 0 },
                Companion { path: "/x".into(), delay_secs: MAX_DELAY_SECS },
            ]
        );
        assert_eq!(parse(Some(&to_value(&list))), list);
        assert!(parse(None).is_empty() && parse(Some(&json!("nope"))).is_empty());
    }

    fn game(platform: &str, exe: &str) -> GameInfo {
        serde_json::from_value(json!({
            "runner": "sideload", "app_name": "g1", "title": "Game One", "is_installed": true,
            "install": {"executable": exe, "platform": platform, "install_path": "/games/one"}
        }))
        .unwrap()
    }

    fn plan_for(settings: serde_json::Value, game: &GameInfo, probe: &FakeProbe) -> LaunchPlan {
        let dir = tempfile::tempdir().unwrap();
        let ctx = PlanContext::load(&DataPaths::at(dir.path()));
        let s = GameSettings::from_map(settings.as_object().unwrap(), "/home/u");
        build_plan_with_settings(&ctx, probe, game, None, &s, &LaunchOptions::default())
    }

    #[test]
    fn a_trainer_joins_the_container_of_a_game_run_through_umu() {
        let probe = FakeProbe::new("/home/u")
            .with_file("/games/one/game.exe", b"MZ")
            .with_file("/t/trainer.exe", b"MZ")
            .with_file("/t/tool.sh", b"#!/bin/sh\n")
            .with_file("/p/GE/proton", b"#!/usr/bin/env python3\n")
            .with_bin("umu-run", "/usr/bin/umu-run")
            .with_path("/pfx");
        let settings = json!({
            "wineVersion": {"bin": "/p/GE/proton", "name": "GE", "type": "proton"}, "winePrefix": "/pfx",
            KEY: [{"path": "/t/trainer.exe", "delay": 10}, {"path": "/t/tool.sh"}]
        });
        let plan = plan_for(settings, &game("Windows", "/games/one/game.exe"), &probe);
        let LaunchPlan::Wine { runtime: WineRuntime::Umu, process, .. } = &plan else { panic!("{plan:?}") };
        assert_eq!(process.command.env.get(UMU_NSENTER).map(String::as_str), Some("1"), "the game starts the container service");
        let [trainer, tool] = &process.companions[..] else { panic!("{:?}", process.companions) };
        assert_eq!(trainer.delay, Duration::from_secs(10));
        assert_eq!(trainer.command.program, "/usr/bin/umu-run");
        assert_eq!(trainer.command.args, ["/t/trainer.exe"]);
        assert_eq!(trainer.command.env["PROTON_VERB"], "runinprefix");
        assert_eq!(trainer.command.env[UMU_NSENTER], "1");
        assert_eq!(trainer.command.env["WINEPREFIX"], "/pfx");
        assert_eq!(trainer.command.env["GAMEID"], process.command.env["GAMEID"]);
        assert_eq!(trainer.command.cwd.as_deref(), Some("/t"));
        // A Linux tool runs directly and is told about the game.
        assert_eq!(tool.command.program, "/t/tool.sh");
        assert_eq!(tool.command.env["HEROIC_GAME_PREFIX"], "/pfx");
        assert!(!tool.command.env.contains_key("SCRIPT_TYPE"));
    }

    #[test]
    fn plain_proton_uses_runinprefix_and_a_native_game_runs_windows_tools_on_their_own() {
        let probe = FakeProbe::new("/home/u")
            .with_file("/games/one/game.exe", b"MZ")
            .with_file("/games/one/game", b"\x7fELF")
            .with_file("/t/trainer.exe", b"MZ")
            .with_file("/p/GE/proton", b"#!/usr/bin/env python3\n")
            .with_path("/pfx");
        let proton = json!({"bin": "/p/GE/proton", "name": "GE", "type": "proton"});
        let settings = json!({"wineVersion": proton, "winePrefix": "/pfx", "disableUMU": true, KEY: [{"path": "/t/trainer.exe"}]});
        let plan = plan_for(settings.clone(), &game("Windows", "/games/one/game.exe"), &probe);
        let LaunchPlan::Wine { runtime: WineRuntime::Proton, process, .. } = &plan else { panic!("{plan:?}") };
        assert!(!process.command.env.contains_key(UMU_NSENTER));
        assert_eq!(process.companions[0].command.program, "/p/GE/proton");
        assert_eq!(process.companions[0].command.args, ["runinprefix", "/t/trainer.exe"]);

        let native = plan_for(settings, &game("linux", "/games/one/game"), &probe);
        let LaunchPlan::Native(process) = &native else { panic!("{native:?}") };
        assert_eq!(process.companions[0].command.args, ["run", "/t/trainer.exe"], "no game prefix to join");
    }

    #[test]
    fn without_programs_nothing_changes() {
        let probe = FakeProbe::new("/home/u").with_file("/games/one/game", b"\x7fELF");
        let plan = plan_for(json!({}), &game("linux", "/games/one/game"), &probe);
        assert!(plan.process().unwrap().companions.is_empty());
    }
}
