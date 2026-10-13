//! Effective per-game settings: Heroic's global defaults (`config.json` ->
//! `defaultSettings`) overlaid with `GamesConfig/<appName>.json[<appName>]`,
//! like `GameConfigV0.getSettings()`.

use heronite_data::DataPaths;
use serde_json::{Map, Value};

use crate::probe::Probe;

pub type JsonMap = Map<String, Value>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WineVersion {
    pub bin: String,
    pub name: String,
    /// 'wine' | 'proton' | 'crossover' | 'toolkit'
    pub kind: String,
    pub wineserver: Option<String>,
    pub lib: Option<String>,
    pub lib32: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wrapper {
    pub exe: String,
    pub args: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Gamescope {
    pub enable_upscaling: bool,
    pub enable_limiter: bool,
    pub enable_force_grab_cursor: bool,
    pub window_type: String,
    pub game_width: String,
    pub game_height: String,
    pub upscale_width: String,
    pub upscale_height: String,
    pub upscale_method: String,
    pub fps_limiter: String,
    pub fps_limiter_no_focus: String,
    pub additional_options: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GameSettings {
    pub wine_version: WineVersion,
    pub wine_prefix: String,
    /// `enviromentOptions` (sic), in order.
    pub env_options: Vec<(String, String)>,
    /// `heroniteSharedFolders`: folders the Steam runtime container mounts.
    pub shared_folders: Vec<String>,
    /// `heroniteCompanions`: programs run alongside the game.
    pub companions: Vec<crate::companions::Companion>,
    pub wrapper_options: Vec<Wrapper>,
    pub launcher_args: String,
    pub target_exe: String,
    /// `beforeLaunchScriptPath`: run to completion before the game starts.
    pub before_launch_script: String,
    /// `afterLaunchScriptPath`: run to completion after the game exits.
    pub after_launch_script: String,
    pub use_game_mode: bool,
    pub show_fps: bool,
    pub show_mangohud: bool,
    /// `heroniteMangohudFpsLimit`: MangoHud's `fps_limit`; empty = no limit.
    pub mangohud_fps_limit: String,
    pub nvidia_prime: bool,
    pub enable_esync: bool,
    pub enable_fsync: bool,
    pub enable_wine_wayland: bool,
    pub enable_hdr: bool,
    pub enable_wow64: bool,
    pub enable_fsr: bool,
    pub max_sharpness: Option<i64>,
    pub enable_dxvk_fps_limit: bool,
    pub dxvk_fps_cap: String,
    /// `autoInstallDxvk`: copy DXVK into Wine prefixes before launching.
    pub auto_install_dxvk: bool,
    pub auto_install_dxvk_nvapi: bool,
    /// `autoInstallVkd3d`.
    pub auto_install_vkd3d: bool,
    /// `autoSyncSaves` + `savesPath`: cloud saves around a launch.
    pub auto_sync_saves: bool,
    pub saves_path: String,
    pub eac_runtime: bool,
    pub battleye_runtime: bool,
    pub prefer_system_libs: bool,
    pub verbose_logs: bool,
    pub offline_mode: bool,
    pub language: String,
    /// `steamRuntime`: `false` or a runtime name/path.
    pub steam_runtime: Option<String>,
    /// `disableUMU`: only `Some(true)` disables umu.
    pub disable_umu: Option<bool>,
    pub gamescope: Gamescope,
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().map(|x| x != 0.0).unwrap_or(false),
        Some(Value::String(s)) => !s.is_empty(),
        _ => false,
    }
}

fn text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn parse_wine_version(v: Option<&Value>) -> WineVersion {
    let Some(Value::Object(o)) = v else {
        return WineVersion::default();
    };
    let opt = |k: &str| {
        let s = text(o.get(k));
        (!s.is_empty()).then_some(s)
    };
    WineVersion {
        // Old configs stored the path wrapped in single quotes.
        bin: text(o.get("bin")).replace('\'', ""),
        name: text(o.get("name")),
        kind: text(o.get("type")),
        wineserver: opt("wineserver"),
        lib: opt("lib"),
        lib32: opt("lib32"),
    }
}

impl GameSettings {
    /// Builds settings from an already merged JSON map. Never fails: missing or
    /// oddly typed values fall back to Heroic's "off/empty".
    pub fn from_map(m: &JsonMap, home: &str) -> Self {
        let b = |k: &str| truthy(m.get(k));
        let mut env_options = Vec::new();
        if let Some(Value::Array(a)) = m.get("enviromentOptions") {
            for e in a {
                let key = text(e.get("key"));
                if key.is_empty() {
                    continue;
                }
                env_options.push((key, remove_quote_if_necessary(&text(e.get("value")))));
            }
        }
        let mut wrapper_options = Vec::new();
        if let Some(Value::Array(a)) = m.get("wrapperOptions") {
            for e in a {
                let exe = text(e.get("exe"));
                if exe.is_empty() {
                    continue;
                }
                wrapper_options.push(Wrapper { exe, args: text(e.get("args")) });
            }
        }
        let gs = match m.get("gamescope") {
            Some(Value::Object(g)) => Gamescope {
                enable_upscaling: truthy(g.get("enableUpscaling")),
                enable_limiter: truthy(g.get("enableLimiter")),
                enable_force_grab_cursor: truthy(g.get("enableForceGrabCursor")),
                window_type: text(g.get("windowType")),
                game_width: text(g.get("gameWidth")),
                game_height: text(g.get("gameHeight")),
                upscale_width: text(g.get("upscaleWidth")),
                upscale_height: text(g.get("upscaleHeight")),
                upscale_method: text(g.get("upscaleMethod")),
                fps_limiter: text(g.get("fpsLimiter")),
                fps_limiter_no_focus: text(g.get("fpsLimiterNoFocus")),
                additional_options: text(g.get("additionalOptions")),
            },
            _ => Gamescope::default(),
        };
        let mut wine_prefix = text(m.get("winePrefix"));
        if wine_prefix.is_empty() {
            wine_prefix = format!("{}/default", heronite_data::game_config::default_wine_prefix_dir(home));
        } else if wine_prefix.contains('~') {
            wine_prefix = wine_prefix.replacen('~', home, 1);
        }
        let steam_runtime = match m.get("steamRuntime") {
            Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
            _ => None,
        };
        Self {
            wine_version: parse_wine_version(m.get("wineVersion")),
            wine_prefix,
            env_options,
            shared_folders: crate::shared_folders::parse(m.get(crate::shared_folders::KEY)),
            companions: crate::companions::parse(m.get(crate::companions::KEY)),
            wrapper_options,
            launcher_args: text(m.get("launcherArgs")),
            target_exe: text(m.get("targetExe")),
            before_launch_script: text(m.get("beforeLaunchScriptPath")).trim().to_string(),
            after_launch_script: text(m.get("afterLaunchScriptPath")).trim().to_string(),
            use_game_mode: b("useGameMode"),
            show_fps: b("showFps"),
            show_mangohud: b("showMangohud"),
            mangohud_fps_limit: text(m.get("heroniteMangohudFpsLimit")).trim().to_string(),
            nvidia_prime: b("nvidiaPrime"),
            enable_esync: b("enableEsync"),
            enable_fsync: b("enableFsync"),
            enable_wine_wayland: b("enableWineWayland"),
            enable_hdr: b("enableHDR"),
            enable_wow64: b("enableWoW64"),
            enable_fsr: b("enableFSR"),
            max_sharpness: m.get("maxSharpness").and_then(Value::as_i64),
            enable_dxvk_fps_limit: b("enableDXVKFpsLimit"),
            dxvk_fps_cap: text(m.get("DXVKFpsCap")),
            auto_install_dxvk: b("autoInstallDxvk"),
            auto_install_dxvk_nvapi: b("autoInstallDxvkNvapi"),
            auto_install_vkd3d: b("autoInstallVkd3d"),
            auto_sync_saves: b("autoSyncSaves"),
            saves_path: text(m.get("savesPath")),
            eac_runtime: b("eacRuntime"),
            battleye_runtime: b("battlEyeRuntime"),
            prefer_system_libs: b("preferSystemLibs"),
            verbose_logs: b("verboseLogs"),
            offline_mode: b("offlineMode"),
            language: text(m.get("language")),
            steam_runtime,
            disable_umu: match m.get("disableUMU") {
                Some(Value::Bool(v)) => Some(*v),
                _ => None,
            },
            gamescope: gs,
        }
    }
}

/// Heroic's `removeQuoteIfNecessary`.
pub fn remove_quote_if_necessary(s: &str) -> String {
    if s.starts_with('"') && s.ends_with('"') {
        s.trim_start_matches('"').trim_end_matches('"').to_string()
    } else {
        s.to_string()
    }
}

/// `<root>/config.json` -> `defaultSettings`: the defaults every game inherits.
#[derive(Debug, Clone, Default)]
pub struct GlobalSettings {
    pub defaults: JsonMap,
}

impl GlobalSettings {
    /// A missing or unreadable file gives empty defaults.
    pub fn load(paths: &DataPaths) -> Self {
        let path = paths.global_config();
        let value = heronite_data::json_store::read_value(&path).unwrap_or(Value::Null);
        let defaults = match value.get("defaultSettings") {
            Some(Value::Object(o)) => o.clone(),
            _ => JsonMap::new(),
        };
        Self { defaults }
    }

    pub fn from_defaults(defaults: JsonMap) -> Self {
        Self { defaults }
    }

    pub fn string(&self, key: &str) -> Option<String> {
        match self.defaults.get(key) {
            Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
            _ => None,
        }
    }

    pub fn wine_version(&self) -> WineVersion {
        parse_wine_version(self.defaults.get("wineVersion"))
    }

    /// `defaultSteamPath` without Heroic's legacy quotes.
    pub fn default_steam_path(&self) -> Option<String> {
        self.string("defaultSteamPath").map(|s| s.replace('\'', ""))
    }

    pub fn alt_legendary_bin(&self) -> Option<String> {
        self.string("altLegendaryBin")
    }
}

/// Reads `GamesConfig/<appName>.json` (the section named after the game) and
/// merges it over the global defaults.
pub fn load_game_settings(paths: &DataPaths, app_name: &str, global: &GlobalSettings, probe: &dyn Probe) -> GameSettings {
    let mut merged = global.defaults.clone();
    let file = heronite_data::json_store::read_value(&paths.game_config(app_name)).unwrap_or(Value::Null);
    if let Some(Value::Object(section)) = file.get(app_name) {
        for (k, v) in section {
            merged.insert(k.clone(), v.clone());
        }
    }
    GameSettings::from_map(&merged, &probe.home())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_sic_env_options_and_quotes() {
        let m = json!({
            "enviromentOptions": [{"key":"A","value":"\"x y\""},{"key":"B","value":"1"}],
            "wrapperOptions": [{"exe":"mangohud","args":""}],
            "wineVersion": {"bin":"'/p/proton'","name":"P","type":"proton"},
            "winePrefix": "~/pfx",
            "steamRuntime": false,
            "disableUMU": true
        });
        let s = GameSettings::from_map(m.as_object().unwrap(), "/home/u");
        assert_eq!(s.env_options, vec![("A".into(), "x y".into()), ("B".into(), "1".into())]);
        assert_eq!(s.wrapper_options[0].exe, "mangohud");
        assert_eq!(s.wine_version.bin, "/p/proton");
        assert_eq!(s.wine_prefix, "/home/u/pfx");
        assert_eq!(s.steam_runtime, None);
        assert_eq!(s.disable_umu, Some(true));
    }

    #[test]
    fn game_section_overrides_global_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(paths.games_config_dir()).unwrap();
        std::fs::write(
            paths.root.join("config.json"),
            r#"{"defaultSettings":{"useGameMode":true,"winePrefix":"/g","wineVersion":{"bin":"/w","name":"W","type":"wine"}},"version":"v0"}"#,
        )
        .unwrap();
        std::fs::write(paths.game_config("g1"), r#"{"g1":{"useGameMode":false,"winePrefix":"/mine"},"version":"v0","explicit":true}"#)
            .unwrap();
        let global = GlobalSettings::load(&paths);
        let probe = crate::probe::FakeProbe::new("/h");
        let s = load_game_settings(&paths, "g1", &global, &probe);
        assert!(!s.use_game_mode);
        assert_eq!(s.wine_prefix, "/mine");
        assert_eq!(s.wine_version.name, "W");
        let other = load_game_settings(&paths, "nope", &global, &probe);
        assert!(other.use_game_mode);
    }
}
