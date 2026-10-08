//! Environment variables Heroic sets for games (Linux only): ports of
//! `setupEnvVars`, `setupWrapperEnvVars`, `setupWineEnvVars` and
//! `setupWrappers` from `backend/launcher.ts`.
//!
//! The maps returned here are *additions* on top of the inherited process
//! environment (the spawner merges them).

use std::collections::BTreeMap;
use std::path::Path;

use regex::Regex;

use crate::probe::Probe;
use crate::settings::GameSettings;

pub type Env = BTreeMap<String, String>;

pub fn setup_env_vars(s: &GameSettings, install_path: Option<&str>, probe: &dyn Probe) -> Env {
    let mut ret = Env::new();
    if s.nvidia_prime {
        ret.insert("DRI_PRIME".into(), "1".into());
        ret.insert("__NV_PRIME_RENDER_OFFLOAD".into(), "1".into());
        ret.insert("__GLX_VENDOR_LIBRARY_NAME".into(), "nvidia".into());
    }
    if let Some(p) = install_path.filter(|p| !p.is_empty()) {
        // Used by the Steam runtime to mount the game directory.
        ret.insert("STEAM_COMPAT_INSTALL_PATH".into(), p.into());
    }
    if let Some(rt) = &s.steam_runtime {
        // umu uses PROTONPATH to pick the runtime.
        ret.insert("PROTONPATH".into(), rt.clone());
    }
    for (k, v) in &s.env_options {
        ret.insert(k.clone(), v.clone());
    }
    // Fixes the std::log_error crash in Fall Guys (Heroic does the same).
    if probe.env_var("LD_PRELOAD").is_none() && !ret.contains_key("LD_PRELOAD") {
        ret.insert("LD_PRELOAD".into(), String::new());
    }
    ret
}

pub fn setup_wrapper_env_vars(app_name: &str, runner: &str) -> Env {
    let mut ret = Env::new();
    ret.insert("HEROIC_APP_NAME".into(), app_name.into());
    ret.insert("HEROIC_APP_RUNNER".into(), runner.into());
    ret.insert("GAMEID".into(), "umu-0".into());
    let (source, store) = match runner {
        "gog" => (Some("gog"), Some("gog")),
        "legendary" => (Some("epic"), Some("egs")),
        "nile" => (Some("amazon"), Some("amazon")),
        "sideload" => (Some("sideload"), None),
        "zoom" => (Some("zoom"), Some("zoomplatform")),
        _ => (None, None),
    };
    if let Some(s) = source {
        ret.insert("HEROIC_APP_SOURCE".into(), s.into());
    }
    if let Some(s) = store {
        ret.insert("STORE".into(), s.into());
    }
    ret
}

/// Like Node's `path.join`: lexically resolves `.` and `..`.
pub fn join(a: &str, b: &str) -> String {
    use std::path::Component;
    let mut out: Vec<String> = Vec::new();
    let absolute = a.starts_with('/');
    for comp in Path::new(a).join(b).components() {
        match comp {
            Component::Normal(c) => out.push(c.to_string_lossy().into_owned()),
            Component::ParentDir => {
                if out.last().map(|l| l != "..").unwrap_or(false) {
                    out.pop();
                } else if !absolute {
                    out.push("..".into());
                }
            }
            _ => {}
        }
    }
    let joined = out.join("/");
    if absolute {
        format!("/{joined}")
    } else if joined.is_empty() {
        ".".into()
    } else {
        joined
    }
}

fn dirname(p: &str) -> String {
    Path::new(p)
        .parent()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".into())
}

/// `lib`/`lib32` of a Wine build as Heroic's `getWineLibs` finds them.
pub fn wine_libs(s: &GameSettings, probe: &dyn Probe) -> (Option<String>, Option<String>) {
    let w = &s.wine_version;
    if w.lib.is_some() && w.lib32.is_some() {
        return (w.lib.clone(), w.lib32.clone());
    }
    let dir = dirname(&w.bin);
    let lib32 = join(&dir, "../lib");
    let lib = join(&dir, "../lib64");
    (
        probe.exists(&lib).then_some(lib),
        probe.exists(&lib32).then_some(lib32),
    )
}

fn deprecated_ge_regex() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(GE|Wine|Proton)-(Proton[7-8]|[4-7].*-GE|GE-4.[0-9]*)").unwrap())
}

/// `setupWineEnvVars`. `runtime_dir` is `<heroic>/tools/runtimes`.
pub fn setup_wine_env_vars(
    s: &GameSettings,
    game_id: &str,
    runtime_dir: &str,
    probe: &dyn Probe,
) -> Env {
    let w = &s.wine_version;
    let kind = w.kind.as_str();
    let home = probe.home();
    let mut ret = Env::new();

    match kind {
        "wine" => {
            ret.insert("WINEPREFIX".into(), s.wine_prefix.clone());
            // Don't let Wine mess with file associations.
            let wmb = "winemenubuilder.exe=d";
            if let Some((k, v)) = s
                .env_options
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("winedlloverrides"))
            {
                ret.insert(k.clone(), format!("{v};{wmb}"));
            } else {
                ret.insert("WINEDLLOVERRIDES".into(), wmb.into());
            }
        }
        "proton" => {
            ret.insert(
                "STEAM_COMPAT_CLIENT_INSTALL_PATH".into(),
                join(&home, ".steam/steam"),
            );
            ret.insert("WINEPREFIX".into(), s.wine_prefix.clone());
            ret.insert("STEAM_COMPAT_DATA_PATH".into(), s.wine_prefix.clone());
            ret.insert("PROTONPATH".into(), dirname(&w.bin));
        }
        "toolkit" => {
            ret.insert("WINEPREFIX".into(), s.wine_prefix.clone());
        }
        _ => {}
    }

    if s.show_fps {
        ret.insert("DXVK_HUD".into(), "fps".into());
    }
    if s.enable_dxvk_fps_limit {
        ret.insert("DXVK_FRAME_RATE".into(), s.dxvk_fps_cap.clone());
    }
    if s.enable_fsr {
        ret.insert("WINE_FULLSCREEN_FSR".into(), "1".into());
        ret.insert(
            "WINE_FULLSCREEN_FSR_STRENGTH".into(),
            s.max_sharpness.filter(|n| *n != 0).unwrap_or(2).to_string(),
        );
    } else {
        ret.insert("WINE_FULLSCREEN_FSR".into(), "0".into());
    }
    let is_proton = kind == "proton";
    if s.enable_esync && !is_proton {
        ret.insert("WINEESYNC".into(), "1".into());
    }
    if !s.enable_esync && is_proton {
        ret.insert("PROTON_NO_ESYNC".into(), "1".into());
    }
    if s.enable_fsync && !is_proton {
        ret.insert("WINEFSYNC".into(), "1".into());
    }
    if !s.enable_fsync && is_proton {
        ret.insert("PROTON_NO_FSYNC".into(), "1".into());
    }
    if s.enable_wine_wayland {
        if is_proton {
            ret.insert("PROTON_ENABLE_WAYLAND".into(), "1".into());
            if s.enable_hdr {
                ret.insert("PROTON_ENABLE_HDR".into(), "1".into());
            }
        } else {
            ret.insert("DISPLAY".into(), String::new());
            if s.enable_hdr {
                ret.insert("DXVK_HDR".into(), "1".into());
            }
        }
    }
    if s.enable_wow64 {
        if is_proton {
            ret.insert("PROTON_USE_WOW64".into(), "1".into());
        } else {
            ret.insert("WINEARCH".into(), "wow64".into());
        }
    }
    if is_proton {
        if s.auto_install_dxvk_nvapi {
            ret.insert("PROTON_ENABLE_NVAPI".into(), "1".into());
            ret.insert("DXVK_NVAPI_ALLOW_OTHER_DRIVERS".into(), "1".into());
        } else {
            ret.insert("PROTON_DISABLE_NVAPI".into(), "1".into());
        }
    }
    if s.auto_install_dxvk_nvapi && kind == "wine" {
        ret.insert("DXVK_ENABLE_NVAPI".into(), "1".into());
        ret.insert("DXVK_NVAPI_ALLOW_OTHER_DRIVERS".into(), "1".into());
    }
    if s.eac_runtime {
        ret.insert(
            "PROTON_EAC_RUNTIME".into(),
            join(runtime_dir, "eac_runtime"),
        );
    }
    if s.battleye_runtime {
        ret.insert(
            "PROTON_BATTLEYE_RUNTIME".into(),
            join(runtime_dir, "battleye_runtime"),
        );
    }
    if is_proton {
        // GE-Proton guesses the AppID from the prefix path otherwise.
        let compat = probe
            .env_var("STEAM_COMPAT_APP_ID")
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "0".into());
        ret.insert("STEAM_COMPAT_APP_ID".into(), compat.clone());
        ret.insert(
            "SteamAppId".into(),
            probe
                .env_var("SteamAppId")
                .filter(|v| !v.is_empty())
                .unwrap_or(compat),
        );
        ret.insert(
            "SteamGameId".into(),
            probe
                .env_var("SteamGameId")
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| format!("heroic-{game_id}")),
        );
        ret.insert("PROTON_LOG_DIR".into(), home.clone());
        if s.verbose_logs {
            let has = |k: &str| s.env_options.iter().any(|(key, _)| key == k);
            if !has("WINEDEBUG") {
                ret.insert("WINEDEBUG".into(), "+fixme".into());
            }
            if !has("DXVK_LOG_LEVEL") {
                ret.insert("DXVK_LOG_LEVEL".into(), "info".into());
            }
            if !has("VKD3D_DEBUG") {
                ret.insert("VKD3D_DEBUG".into(), "fixme".into());
            }
        }
    }
    if !s.prefer_system_libs && kind == "wine" {
        if probe.env_var("ORIG_LD_LIBRARY_PATH").is_none() {
            ret.insert(
                "ORIG_LD_LIBRARY_PATH".into(),
                probe.env_var("LD_LIBRARY_PATH").unwrap_or_default(),
            );
        }
        if let (Some(lib), Some(lib32)) = wine_libs(s, probe) {
            let ld = [
                Some(lib.clone()),
                Some(lib32.clone()),
                probe.env_var("LD_LIBRARY_PATH"),
            ]
            .into_iter()
            .flatten()
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join(":");
            ret.insert("LD_LIBRARY_PATH".into(), ld);
            let (g64, g32) = (join(&lib, "gstreamer-1.0"), join(&lib32, "gstreamer-1.0"));
            if probe.exists(&g64) && probe.exists(&g32) {
                ret.insert("GST_PLUGIN_SYSTEM_PATH_1_0".into(), format!("{g64}:{g32}"));
            }
            let (d64, d32) = (join(&lib, "wine"), join(&lib32, "wine"));
            if probe.exists(&d64) && probe.exists(&d32) {
                ret.insert("WINEDLLPATH".into(), format!("{d64}:{d32}"));
            }
        } else {
            log::error!(
                "Couldn't find all library folders of {}; falling back to system libraries",
                w.name
            );
        }
    }
    if deprecated_ge_regex().is_match(&w.name) {
        ret.insert("SteamVirtualGamepadInfo".into(), String::new());
    }
    ret
}

/// Wrappers Heroic puts in front of the game, resolved against `$PATH`.
pub struct WrapperPlan {
    pub wrappers: Vec<String>,
    /// Why a requested tool is missing (Heroic aborts the launch for these).
    pub error: Option<String>,
}

/// Port of the Linux half of `prepareLaunch` + `setupWrappers`.
/// `steam_runtime` is the umu binary when the wrapper list should include it
/// (native games with the Steam runtime, or legendary through Proton+umu).
pub fn build_wrappers(
    s: &GameSettings,
    probe: &dyn Probe,
    steam_runtime: Option<&str>,
) -> WrapperPlan {
    let mut error = None;
    let mut mangohud: Vec<String> = Vec::new();
    let mut gamemode: Option<String> = None;
    let mut gamescope: Vec<String> = Vec::new();

    if s.show_mangohud && probe.env_var("SteamDeck").as_deref() != Some("1") {
        match probe.which("mangohud") {
            Some(bin) => mangohud = vec![bin, "--dlsym".into()],
            None => {
                error = Some(
                    "Mangohud is enabled, but `mangohud` executable could not be found on $PATH"
                        .to_string(),
                )
            }
        }
    }
    if s.use_game_mode {
        match probe.which("gamemoderun") {
            Some(bin) => gamemode = Some(bin),
            None => {
                error.get_or_insert_with(|| {
                    "GameMode is enabled, but `gamemoderun` executable could not be found on $PATH".to_string()
                });
            }
        }
    }
    let g = &s.gamescope;
    if (g.enable_limiter || g.enable_upscaling)
        && probe.env_var("SteamDeck").as_deref() != Some("1")
    {
        if let Some(bin) = probe.which("gamescope") {
            let old = probe.gamescope_is_old(&bin);
            gamescope.push(bin);
            if g.enable_upscaling {
                if !g.game_width.is_empty() {
                    gamescope.extend(["-w".into(), g.game_width.clone()]);
                }
                if !g.game_height.is_empty() {
                    gamescope.extend(["-h".into(), g.game_height.clone()]);
                }
                if !g.upscale_width.is_empty() {
                    gamescope.extend(["-W".into(), g.upscale_width.clone()]);
                }
                if !g.upscale_height.is_empty() {
                    gamescope.extend(["-H".into(), g.upscale_height.clone()]);
                }
                match (g.upscale_method.as_str(), old) {
                    ("fsr", true) => gamescope.push("-U".into()),
                    ("fsr", false) => gamescope.extend(["-F".into(), "fsr".into()]),
                    ("nis", true) => gamescope.push("-Y".into()),
                    ("nis", false) => gamescope.extend(["-F".into(), "nis".into()]),
                    ("integer", true) => gamescope.push("-i".into()),
                    ("integer", false) => gamescope.extend(["-S".into(), "integer".into()]),
                    ("stretch", false) => gamescope.extend(["-S".into(), "stretch".into()]),
                    _ => {}
                }
                match g.window_type.as_str() {
                    "fullscreen" => gamescope.push("-f".into()),
                    "borderless" => gamescope.push("-b".into()),
                    _ => {}
                }
            }
            if g.enable_limiter {
                if !g.fps_limiter.is_empty() {
                    gamescope.extend(["-r".into(), g.fps_limiter.clone()]);
                }
                if !g.fps_limiter_no_focus.is_empty() {
                    gamescope.extend(["-o".into(), g.fps_limiter_no_focus.clone()]);
                }
            }
            if g.enable_force_grab_cursor {
                gamescope.push("--force-grab-cursor".into());
            }
            if s.show_mangohud {
                gamescope.push("--mangoapp".into());
            }
            gamescope.extend(shlex::split(&g.additional_options).unwrap_or_default());
            // Must be the last option.
            gamescope.push("--".into());
        } else {
            log::warn!(
                "Gamescope is enabled, but `gamescope` executable could not be found on $PATH"
            );
        }
    }

    let mut wrappers: Vec<String> = Vec::new();
    // Gamescope is always the first wrapper.
    wrappers.extend(gamescope.iter().cloned());
    for w in &s.wrapper_options {
        wrappers.push(w.exe.clone());
        wrappers.extend(shlex::split(&w.args).unwrap_or_default());
    }
    if !mangohud.is_empty() && gamescope.is_empty() {
        wrappers.extend(mangohud);
    }
    if let Some(gm) = gamemode {
        wrappers.push(gm);
    }
    if let Some(rt) = steam_runtime {
        wrappers.push(rt.to_string());
    }
    wrappers.retain(|w| !w.is_empty());
    WrapperPlan { wrappers, error }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::FakeProbe;
    use serde_json::json;

    fn settings(v: serde_json::Value) -> GameSettings {
        GameSettings::from_map(v.as_object().unwrap(), "/home/u")
    }

    #[test]
    fn proton_env_matches_heroic() {
        let s = settings(json!({
            "wineVersion": {"bin": "/t/GE-Proton9/proton", "name": "GE-Proton9", "type": "proton"},
            "winePrefix": "/pfx", "enableEsync": true, "enableFsync": false,
            "autoInstallDxvkNvapi": true, "verboseLogs": true, "showFps": true,
            "enviromentOptions": [{"key":"WINEDEBUG","value":"-all"}]
        }));
        let probe = FakeProbe::new("/home/u");
        let e = setup_wine_env_vars(&s, "0", "/tools/runtimes", &probe);
        assert_eq!(e["STEAM_COMPAT_DATA_PATH"], "/pfx");
        assert_eq!(e["PROTONPATH"], "/t/GE-Proton9");
        assert_eq!(
            e["STEAM_COMPAT_CLIENT_INSTALL_PATH"],
            "/home/u/.steam/steam"
        );
        assert_eq!(e["SteamGameId"], "heroic-0");
        assert_eq!(e["PROTON_NO_FSYNC"], "1");
        assert!(!e.contains_key("PROTON_NO_ESYNC"));
        assert_eq!(e["PROTON_ENABLE_NVAPI"], "1");
        assert_eq!(e["DXVK_HUD"], "fps");
        assert_eq!(e["WINE_FULLSCREEN_FSR"], "0");
        assert_eq!(e["DXVK_LOG_LEVEL"], "info");
        assert!(
            !e.contains_key("WINEDEBUG"),
            "user-set WINEDEBUG is left to the env options"
        );
        assert!(!e.contains_key("LD_LIBRARY_PATH"));
    }

    #[test]
    fn plain_wine_env_appends_to_user_dll_overrides() {
        let s = settings(json!({
            "wineVersion": {"bin": "/w/bin/wine", "name": "Wine", "type": "wine"},
            "winePrefix": "/pfx", "enableFsync": true,
            "enviromentOptions": [{"key":"WINEDLLOVERRIDES","value":"dxgi=n"}]
        }));
        let probe = FakeProbe::new("/h")
            .with_path("/w/lib64")
            .with_path("/w/lib")
            .with_path("/w/lib64/wine")
            .with_path("/w/lib/wine");
        let e = setup_wine_env_vars(&s, "0", "/r", &probe);
        assert_eq!(e["WINEDLLOVERRIDES"], "dxgi=n;winemenubuilder.exe=d");
        assert_eq!(e["WINEFSYNC"], "1");
        assert_eq!(e["LD_LIBRARY_PATH"], "/w/lib64:/w/lib");
        assert_eq!(e["WINEDLLPATH"], "/w/lib64/wine:/w/lib/wine");
        assert_eq!(e["ORIG_LD_LIBRARY_PATH"], "");
    }

    #[test]
    fn wrappers_follow_heroic_order() {
        let s = settings(json!({
            "useGameMode": true, "showMangohud": true,
            "wrapperOptions": [{"exe":"taskset","args":"-c 0-3"}]
        }));
        let probe = FakeProbe::new("/h")
            .with_bin("mangohud", "/usr/bin/mangohud")
            .with_bin("gamemoderun", "/usr/bin/gamemoderun");
        let w = build_wrappers(&s, &probe, Some("/umu"));
        assert!(w.error.is_none());
        assert_eq!(
            w.wrappers,
            [
                "taskset",
                "-c",
                "0-3",
                "/usr/bin/mangohud",
                "--dlsym",
                "/usr/bin/gamemoderun",
                "/umu"
            ]
        );
        let missing = build_wrappers(&s, &FakeProbe::new("/h"), None);
        assert!(missing.error.unwrap().contains("mangohud"));
    }

    #[test]
    fn env_vars_and_wrapper_env() {
        let s = settings(json!({"nvidiaPrime": true, "steamRuntime": "/rt"}));
        let e = setup_env_vars(&s, Some("/games/x"), &FakeProbe::new("/h"));
        assert_eq!(e["DRI_PRIME"], "1");
        assert_eq!(e["STEAM_COMPAT_INSTALL_PATH"], "/games/x");
        assert_eq!(e["PROTONPATH"], "/rt");
        assert_eq!(e["LD_PRELOAD"], "");
        let e2 = setup_env_vars(&s, None, &FakeProbe::new("/h").with_env("LD_PRELOAD", "x"));
        assert!(!e2.contains_key("LD_PRELOAD"));
        let we = setup_wrapper_env_vars("Calluna", "legendary");
        assert_eq!(we["STORE"], "egs");
        assert_eq!(we["HEROIC_APP_SOURCE"], "epic");
        assert_eq!(we["GAMEID"], "umu-0");
    }
}
