//! What goes into a launch log: the switches, the settings shown and the header.

use std::path::Path;

use heronite_data::DataPaths;
use serde_json::{Map, Value};

use super::GameLog;

/// Whether launches of `app_name` are logged, from the global
/// `disableLogs` and the per-game `verboseLogs` (default on). Returns the
/// merged settings too, for the header.
pub fn launch_log_switches(data_paths: &DataPaths, app_name: &str) -> (bool, bool, Map<String, Value>) {
    let global = crate::settings::GlobalSettings::load(data_paths);
    let merged = merged_settings(data_paths, app_name, &global);
    let disabled_globally = truthy(global.defaults.get("disableLogs"));
    let verbose = merged.get("verboseLogs").map(|v| truthy(Some(v))).unwrap_or(true);
    (disabled_globally, verbose, merged)
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().map(|x| x != 0.0).unwrap_or(false),
        Some(Value::String(s)) => !s.is_empty(),
        _ => false,
    }
}

/// Global defaults overlaid with the game's section (like Heroic's
/// `GameConfig.getSettings()`).
pub fn merged_settings(data_paths: &DataPaths, app_name: &str, global: &crate::settings::GlobalSettings) -> Map<String, Value> {
    let mut merged = global.defaults.clone();
    let file = heronite_data::json_store::read_value(&data_paths.game_config(app_name)).unwrap_or(Value::Null);
    if let Some(Value::Object(section)) = file.get(app_name) {
        for (k, v) in section {
            merged.insert(k.clone(), v.clone());
        }
    }
    merged
}

fn is_secret_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    ["apikey", "api_key", "token", "secret", "password", "passwd", "credential"].iter().any(|s| k.contains(s))
}

/// `filterGameSettingsForLog`: what is useful for support, nothing secret.
pub fn settings_for_log(merged: &Map<String, Value>, native: bool) -> Value {
    let mut s = merged.clone();
    for k in ["enableQuickSavesMenu", "verboseLogs", "enableMsync", "wineCrossoverBottle", "advertiseAvxForRosetta"] {
        s.remove(k);
    }
    if let Some(Value::Object(g)) = s.get_mut("gamescope") {
        let enabled = |g: &Map<String, Value>, k: &str| truthy(g.get(k));
        if !enabled(g, "enableLimiter") {
            g.remove("fpsLimiter");
            g.remove("fpsLimiterNoFocus");
        }
        if !enabled(g, "enableUpscaling") {
            for k in ["upscaleMethod", "upscaleHeight", "upscaleWidth", "gameHeight", "gameWidth", "windowType"] {
                g.remove(k);
            }
        }
    }
    if native {
        for k in [
            "wineVersion",
            "winePrefix",
            "autoInstallDxvk",
            "autoInstallDxvkNvapi",
            "autoInstallVkd3d",
            "enableDXVKFpsLimit",
            "DXVKFpsCap",
            "battlEyeRuntime",
            "eacRuntime",
            "enableEsync",
            "enableFsync",
            "enableWineWayland",
            "enableHDR",
            "enableWoW64",
        ] {
            s.remove(k);
        }
    }
    for (k, v) in s.iter_mut() {
        if is_secret_key(k) && v.as_str().is_some_and(|x| !x.is_empty()) {
            *v = Value::String("<hidden>".into());
        }
    }
    Value::Object(s)
}

/// Heroic prints `ENV=value ... program args`; values of secret-looking
/// variables are replaced.
pub fn command_line_for_log(cmd: &crate::plan::Command) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (k, v) in &cmd.env {
        let shown = if is_secret_key(k) { "<hidden>".to_string() } else { v.clone() };
        let pair = format!("{k}={shown}");
        parts.push(shlex::try_quote(&pair).map(|c| c.into_owned()).unwrap_or(pair));
    }
    parts.push(cmd.display());
    parts.join(" ")
}

/// What the header of a launch log is made of.
pub struct HeaderInfo<'a> {
    pub title: &'a str,
    pub runner: &'a str,
    pub app_name: &'a str,
    pub native: bool,
    pub installed_in: Option<&'a str>,
    /// `formatSystemInfo` text, when available.
    pub system_info: Option<&'a str>,
    pub settings: &'a Map<String, Value>,
    pub settings_path: &'a Path,
    pub notes: &'a [String],
}

/// The lines Heroic's `prepareLaunch` writes before the game starts.
pub fn write_launch_header(log: &GameLog, h: &HeaderInfo<'_>) {
    log.info(&format!("Launching \"{}\" ({})", h.title, h.runner));
    log.info(&format!("Native? {}", h.native));
    if let Some(dir) = h.installed_in.filter(|d| !d.is_empty()) {
        log.info(&format!("Installed in: {dir}\n"));
    }
    if let Some(info) = h.system_info {
        log.info(&format!("System Info: \n{}\n", info.trim_end()));
    }
    let json = serde_json::to_string_pretty(&settings_for_log(h.settings, h.native)).unwrap_or_default();
    log.info(&format!("Game Settings: {json} \n Stored at: {}\n", h.settings_path.display()));
    for note in h.notes {
        log.info(note);
    }
}

/// The "not verbose" notice at the top of a launch log (forced).
pub fn write_disabled_notice(log: &GameLog) {
    log.forced_warning("IMPORTANT: Logs are disabled");
    log.forced_warning("Enable verbose logs in Game's settings > Advanced tab > 'Enable verbose logs' before reporting an issue.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn switches_follow_global_disable_and_game_verbose() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(data_paths.games_config_dir()).unwrap();
        std::fs::write(
            data_paths.root.join("config.json"),
            r#"{"defaultSettings":{"disableLogs":false,"verboseLogs":true,"steamGridDbApiKey":"K"},"version":"v0"}"#,
        )
        .unwrap();
        std::fs::write(data_paths.game_config("quiet"), r#"{"quiet":{"verboseLogs":false},"version":"v0"}"#).unwrap();
        let (off, verbose, merged) = launch_log_switches(&data_paths, "loud");
        assert!(!off && verbose);
        let shown = settings_for_log(&merged, true).to_string();
        assert!(!shown.contains("\"K\"") && shown.contains("<hidden>"), "{shown}");
        let (_, verbose, _) = launch_log_switches(&data_paths, "quiet");
        assert!(!verbose);
        std::fs::write(data_paths.root.join("config.json"), r#"{"defaultSettings":{"disableLogs":true},"version":"v0"}"#).unwrap();
        let (off, verbose, _) = launch_log_switches(&data_paths, "anything");
        assert!(off, "disableLogs wins");
        assert!(verbose, "verboseLogs defaults to on like Heroic's defaults");
    }

    #[test]
    fn settings_for_log_drops_noise() {
        let m = json!({"verboseLogs":true,"enableQuickSavesMenu":true,"winePrefix":"/p","useGameMode":false,
            "gamescope":{"enableLimiter":false,"fpsLimiter":"30","enableUpscaling":true,"upscaleMethod":"fsr"}});
        let v = settings_for_log(m.as_object().unwrap(), false);
        assert!(v.get("verboseLogs").is_none() && v.get("enableQuickSavesMenu").is_none());
        assert_eq!(v["winePrefix"], "/p");
        assert!(v["gamescope"].get("fpsLimiter").is_none());
        assert_eq!(v["gamescope"]["upscaleMethod"], "fsr");
        assert!(settings_for_log(m.as_object().unwrap(), true).get("winePrefix").is_none());
    }

    #[test]
    fn command_lines_hide_secrets() {
        let mut env = crate::plan::Command { program: "legendary".into(), args: vec!["launch".into(), "x y".into()], ..Default::default() };
        env.env.insert("WINEPREFIX".into(), "/p".into());
        env.env.insert("MY_TOKEN".into(), "abc".into());
        let line = command_line_for_log(&env);
        assert!(
            line.contains("WINEPREFIX=/p") && line.contains("MY_TOKEN=\\<hidden\\>")
                || line.contains("MY_TOKEN='<hidden>'")
                || line.contains("MY_TOKEN=<hidden>"),
            "{line}"
        );
        assert!(!line.contains("abc"));
        assert!(line.ends_with("legendary launch 'x y'"), "{line}");
    }
}
