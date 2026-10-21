//! The last stage: the installed game becomes a local game of the
//! Collection (`sideload_apps/library.json`), with its settings
//! (`GamesConfig/<app>.json`) and its other ways to start as the game's own
//! actions (`local_library/actions.json`).
//!
//! From the script's settings: `system.env` and the Wine DLL overrides become
//! environment variables, `dxvk` / `vkd3d` / `dxvk_nvapi` / `esync` /
//! `fsync` and `system.gamemode` / `mangohud` their switches, `game.args`
//! the launch arguments. Settings with no counterpart are listed in the
//! outcome's notes.

use std::path::Path;

use heronite_data::game_actions::{self, Action as GameAction, ActionKind};
use heronite_data::game_config;
use heronite_data::sideload::{self, SideloadPlatform, SideloadSpec};
use heronite_i18n::tr;
use heronite_launch::native::{detect_executable_kind, ExecutableKind};
use heronite_launch::SystemProbe;
use serde_json::{json, Value};

use crate::engine::{Outcome, State};
use crate::runners::{extra_launches, resolve, Launch, Runner};
use crate::text::{pairs, truthy};

/// Runner settings with a game setting of their own.
const WINE_SWITCHES: [(&str, &str); 5] = [
    ("dxvk", "autoInstallDxvk"),
    ("vkd3d", "autoInstallVkd3d"),
    ("dxvk_nvapi", "autoInstallDxvkNvapi"),
    ("esync", "enableEsync"),
    ("fsync", "enableFsync"),
];
const SYSTEM_SWITCHES: [(&str, &str); 2] = [("gamemode", "useGameMode"), ("mangohud", "showMangohud")];
/// Read elsewhere, or meaningless here.
const WINE_HANDLED: [&str; 4] = ["overrides", "version", "dxvk_version", "vkd3d_version"];
const SYSTEM_HANDLED: [&str; 4] = ["env", "disable_runtime", "prefer_system_libs", "exclude_processes"];

/// `WINEDLLOVERRIDES` from a map of DLL -> mode (`n,b`, `native`,
/// `builtin`, `disabled`, empty = disabled). Same modes grouped, in a fixed
/// order; unknown modes are left out.
pub fn dll_overrides(overrides: &[(String, String)]) -> String {
    const ORDER: [&str; 6] = ["n,b", "b,n", "b", "n", "d", ""];
    let mut groups: Vec<Vec<String>> = vec![Vec::new(); ORDER.len()];
    for (dll, mode) in overrides {
        let m = mode.replace(' ', "").to_ascii_lowercase().replace("builtin", "b").replace("native", "n").replace("disabled", "");
        match ORDER.iter().position(|o| *o == m) {
            Some(i) => groups[i].push(dll.trim().to_string()),
            None => log::warn!("unknown DLL override mode {mode:?} for {dll}"),
        }
    }
    ORDER
        .iter()
        .zip(groups)
        .filter(|(_, dlls)| !dlls.is_empty())
        .map(|(mode, mut dlls)| {
            dlls.sort();
            format!("{}={mode}", dlls.join(","))
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn env_value(env: &[(String, String)]) -> Value {
    Value::Array(env.iter().map(|(k, v)| json!({ "key": k, "value": v })).collect())
}

/// `runAs` that pins the platform when the file itself says otherwise (a
/// `.bat` started through Wine).
fn run_as(platform: SideloadPlatform, exe: &str) -> Option<String> {
    let detected = match detect_executable_kind(exe, &SystemProbe) {
        ExecutableKind::Linux => Some(SideloadPlatform::Linux),
        ExecutableKind::Windows => Some(SideloadPlatform::Windows),
        ExecutableKind::Unknown => None,
    };
    match (platform, detected) {
        (SideloadPlatform::Windows, Some(d)) if d != platform => Some("windows".into()),
        (SideloadPlatform::Windows, None) => Some("windows".into()),
        (SideloadPlatform::Linux, Some(d)) if d != platform => Some("linux".into()),
        _ => None,
    }
}

pub(crate) fn add_game(st: &mut State, runner: Runner) -> Result<Outcome, String> {
    let job = st.job;
    let script = &job.script;
    let sub = |s: &str| st.sub(s);
    let launch = resolve(runner, &script.game, &sub, &job.game_dir)?;
    let extras = extra_launches(&script.game, &sub, &job.game_dir);
    let mut notes = Vec::new();
    let folder = job.game_dir.to_string_lossy().into_owned();

    let mut spec = SideloadSpec {
        app_name: job.app_name.clone(),
        title: job.title.trim().to_string(),
        executable: String::new(),
        platform: SideloadPlatform::Linux,
        art_cover: String::new(),
        art_square: String::new(),
        browser_url: String::new(),
        custom_user_agent: String::new(),
        launch_full_screen: false,
        run_as: None,
        install: false,
        install_folder: folder.clone(),
        command: String::new(),
    };
    let mut args = String::new();
    match &launch {
        Launch::Windows { exe, args: a, .. } | Launch::Linux { exe, args: a } => {
            spec.platform = if matches!(launch, Launch::Windows { .. }) { SideloadPlatform::Windows } else { SideloadPlatform::Linux };
            spec.executable = exe.clone();
            spec.run_as = run_as(spec.platform, exe);
            args = a.clone();
            if !Path::new(exe).exists() {
                notes.push(tr!(
                    "The program that starts the game was not found: {path}. Pick the right one in Edit game.",
                    path = exe.clone()
                ));
            }
        }
        Launch::Command { folder, command } => {
            spec.install_folder = folder.clone();
            spec.command = command.clone();
        }
        Launch::Browser { url } => {
            spec.platform = SideloadPlatform::Browser;
            spec.browser_url = url.clone();
            spec.install_folder = String::new();
        }
    }
    sideload::save_game(&job.paths, &spec).map_err(|e| format!("{e:#}"))?;

    let mut changes: Vec<(String, Value)> = Vec::new();
    let mut env = st.script_env();
    if let Launch::Windows { prefix, .. } = &launch {
        changes.push(("winePrefix".into(), json!(prefix)));
        if let Some(v) = &job.wine_version {
            changes.push(("wineVersion".into(), v.clone()));
        }
        let overrides = pairs(script.runner_config.get("overrides"));
        if !overrides.is_empty() {
            let text = dll_overrides(&overrides);
            match env.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case("WINEDLLOVERRIDES")) {
                Some((_, v)) => *v = format!("{v};{text}"),
                None => env.push(("WINEDLLOVERRIDES".into(), text)),
            }
        }
        let wine_kind = job.wine_version.as_ref().and_then(|v| v.get("type")).and_then(Value::as_str).unwrap_or("");
        if st.win32() && wine_kind == "wine" {
            env.push(("WINEARCH".into(), "win32".into()));
        }
        for (key, setting) in WINE_SWITCHES {
            if let Some(on) = truthy(script.runner_config.get(key)) {
                changes.push((setting.into(), json!(on)));
            }
        }
        if let Some(wanted) = script.runner_config.get("version").and_then(crate::text::scalar).filter(|v| !v.is_empty()) {
            let chosen = job.wine_version.as_ref().map(crate::tasks::wine_name).unwrap_or_default();
            if !chosen.eq_ignore_ascii_case(&wanted) {
                notes.push(tr!("The script was made for Wine {wanted}; the game uses {chosen}.", wanted, chosen));
            }
        }
        let ignored: Vec<&str> = script
            .runner_config
            .keys()
            .map(String::as_str)
            .filter(|k| !WINE_HANDLED.contains(k) && !WINE_SWITCHES.iter().any(|(s, _)| s == k))
            .collect();
        if !ignored.is_empty() {
            notes.push(tr!("Wine settings of the script that were not applied: {settings}.", settings = ignored.join(", ")));
        }
    }
    for (key, setting) in SYSTEM_SWITCHES {
        if let Some(on) = truthy(script.system.get(key)) {
            changes.push((setting.into(), json!(on)));
        }
    }
    let ignored: Vec<&str> = script
        .system
        .keys()
        .map(String::as_str)
        .filter(|k| !SYSTEM_HANDLED.contains(k) && !SYSTEM_SWITCHES.iter().any(|(s, _)| s == k))
        .collect();
    if !ignored.is_empty() {
        notes.push(tr!("System settings of the script that were not applied: {settings}.", settings = ignored.join(", ")));
    }
    if !env.is_empty() {
        changes.push(("enviromentOptions".into(), env_value(&env)));
    }
    if job.sandbox.enabled && !matches!(launch, Launch::Browser { .. }) {
        changes.push((heronite_launch::sandbox::KEY.into(), job.sandbox.to_value()));
    }
    if !args.trim().is_empty() {
        changes.push(("launcherArgs".into(), json!(args.trim())));
    }
    if !changes.is_empty() {
        game_config::set_game_settings(&job.paths, &job.app_name, &job.home, &changes).map_err(|e| format!("{e:#}"))?;
    }

    if !extras.is_empty() && !matches!(launch, Launch::Browser { .. }) {
        let actions: Vec<GameAction> = extras
            .iter()
            .enumerate()
            .map(|(i, x)| GameAction {
                id: format!("script-{}", i + 1),
                name: x.name.clone(),
                kind: ActionKind::Program,
                target: x.exe.clone(),
                args: x.args.clone(),
                counts_as_playing: true,
            })
            .collect();
        game_actions::save(&job.paths, &format!("sideload_{}", job.app_name), &actions).map_err(|e| format!("{e:#}"))?;
    }
    notes.append(&mut st.notes);
    Ok(Outcome { app_name: job.app_name.clone(), title: spec.title, notes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_are_grouped_by_mode() {
        let o = |pairs: &[(&str, &str)]| dll_overrides(&pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>());
        assert_eq!(o(&[("steam_api64", "n,b"), ("ddraw", "n,b"), ("lsteamclient", "d----")]), "ddraw,steam_api64=n,b");
        assert_eq!(o(&[("d3d9", "native"), ("mscoree", "disabled"), ("x", ""), ("y", "builtin, native")]), "y=b,n;d3d9=n;mscoree,x=");
        assert_eq!(o(&[]), "");
    }
}
