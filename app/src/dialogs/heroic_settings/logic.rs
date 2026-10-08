//! Pure rules of the Heroic settings tabs (no UI): which tabs a game gets,
//! how Wine versions are named and which toggles depend on which. Ports of
//! the conditions in `Settings/sections/GamesSettings` and `components/*`.

use heronite_data::collection::Game;
use heronite_launch::native::{effective_platform, Platform};
use heronite_launch::SystemProbe;
use heronite_wine::WineInstallation;
use serde_json::{json, Value};

/// Tabs of the Heroic settings (the General one only exists for the global
/// dialog).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Section {
    General,
    Wine,
    Other,
    Advanced,
    Gamescope,
    Saves,
}

impl Section {
    pub fn label(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::Wine => "Wine",
            Section::Other => "Other",
            Section::Advanced => "Advanced",
            Section::Gamescope => "Gamescope",
            Section::Saves => "Cloud saves",
        }
    }
}

/// How a game runs, which decides the tabs it gets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RunKind {
    /// Windows program through Wine/Proton/umu.
    Wine,
    /// Native Linux program.
    Native,
    /// Browser games, Steam URIs, emulated ROMs, not installed: no Heroic settings.
    None,
}

/// What the settings need to know about one game.
#[derive(Clone, Debug)]
pub struct GameFacts {
    pub app_name: String,
    pub runner: String,
    pub run_kind: RunKind,
    pub cloud_saves: bool,
}

impl GameFacts {
    /// Reads what is needed from a Collection game. Touches the disk lightly
    /// (the first bytes of the executable of sideloaded games).
    pub fn of(game: &Game) -> Self {
        let g = &game.game;
        let platform = g.install.platform.clone().unwrap_or_default().to_ascii_lowercase();
        let launch_kind = game.launch_kind.as_deref().unwrap_or("");
        let run_kind = if !game.installed
            || matches!(launch_kind, "emulator" | "steam-uri")
            || platform == "browser"
            || g.rest.get("browserUrl").is_some_and(|v| !v.is_null() && v != "")
        {
            RunKind::None
        } else if g.runner == "sideload" || g.runner == "zoom" {
            let exe = g.install.executable.clone().unwrap_or_default();
            match effective_platform(g, game.meta.as_deref(), &exe, &SystemProbe) {
                Platform::Linux => RunKind::Native,
                Platform::Windows => RunKind::Wine,
            }
        } else if platform.contains("linux") || g.is_linux_native == Some(true) {
            RunKind::Native
        } else if platform.contains("mac") || platform == "osx" {
            RunKind::None
        } else {
            RunKind::Wine
        };
        Self {
            app_name: game.app_name.clone(),
            runner: game.runner.clone(),
            run_kind,
            cloud_saves: g.rest.get("cloud_save_enabled").and_then(Value::as_bool).unwrap_or(false),
        }
    }

    pub fn is_sideload(&self) -> bool {
        self.runner == "sideload" || self.runner == "zoom"
    }

    /// Tabs shown for this game, in Heroic's order (`GamesSettings`).
    pub fn sections(&self) -> Vec<Section> {
        sections_for(self.run_kind, &self.runner, self.cloud_saves)
    }

    /// The tab Heroic opens first (`getStartingTab`).
    pub fn start_section(&self) -> Option<Section> {
        match self.run_kind {
            RunKind::Wine => Some(Section::Wine),
            RunKind::Native => Some(Section::Other),
            RunKind::None => None,
        }
    }
}

pub fn sections_for(run_kind: RunKind, runner: &str, cloud_saves_tab: bool) -> Vec<Section> {
    let mut out = Vec::new();
    match run_kind {
        RunKind::None => return out,
        RunKind::Wine => {
            out.push(Section::Wine);
            out.push(Section::Other);
        }
        RunKind::Native => out.push(Section::Other),
    }
    out.push(Section::Advanced);
    if runner == "legendary" && cloud_saves_tab {
        out.push(Section::Saves);
    }
    out.push(Section::Gamescope);
    out
}

/// Tabs of the global "Game defaults" editor.
pub const DEFAULT_SECTIONS: [Section; 4] = [Section::Wine, Section::Other, Section::Advanced, Section::Gamescope];

/// Wine name as Heroic lists it (`WineVersionListItem`).
pub fn display_wine_name(name: &str) -> String {
    for prefix in ["Proton-GE-Proton", "Proton-GE"] {
        if let Some(i) = name.find(prefix) {
            return format!("{}GE-Proton{}", &name[..i], &name[i + prefix.len()..]);
        }
    }
    name.to_string()
}

/// Heroic's `defaultWineVersion` (shown when nothing is configured).
pub fn default_wine() -> WineInstallation {
    WineInstallation { bin: "/usr/bin/wine".into(), name: "Wine Default".into(), kind: "wine".into(), ..Default::default() }
}

/// `wineVersion` of a settings map.
pub fn wine_of(settings: &serde_json::Map<String, Value>) -> WineInstallation {
    settings
        .get("wineVersion")
        .and_then(|v| serde_json::from_value::<WineInstallation>(v.clone()).ok())
        .filter(|w| !w.name.is_empty() || !w.bin.is_empty())
        .unwrap_or_else(default_wine)
}

/// The Wine list plus the configured version when it is not in it (Heroic
/// adds it when it is still valid).
pub fn wine_options(list: &[WineInstallation], current: &WineInstallation) -> Vec<WineInstallation> {
    let mut out = list.to_vec();
    if !out.iter().any(|w| w.name == current.name || (!w.bin.is_empty() && w.bin == current.bin)) {
        out.push(current.clone());
    }
    out
}

pub fn wine_to_value(w: &WineInstallation) -> Value {
    serde_json::to_value(w).unwrap_or_else(|_| json!({}))
}

/// `validateEnvKey`; empty = valid.
pub fn validate_env_key(key: &str) -> &'static str {
    let key = key.trim();
    if key.is_empty() {
        "Variable names can't be empty"
    } else if key.contains('=') {
        "Variable names can't contain the \"=\" sign"
    } else if key.contains(' ') {
        "Variable names can't contain spaces"
    } else {
        ""
    }
}

/// `LauncherArgs` warnings.
pub fn launcher_args_error(args: &str) -> &'static str {
    if args.contains("%command") {
        "The %command% syntax from Steam is not valid as game arguments."
    } else if has_env_assignment(args) {
        "Environment variables must be configured in the table below."
    } else {
        ""
    }
}

/// `/[A-Z_]+=\S/`.
fn has_env_assignment(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_uppercase() || b[i] == b'_' {
            let mut j = i;
            while j < b.len() && (b[j].is_ascii_uppercase() || b[j] == b'_') {
                j += 1;
            }
            if j + 1 < b.len() && b[j] == b'=' && !b[j + 1].is_ascii_whitespace() {
                return true;
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    false
}

/// Digits only (gamescope sizes and limits).
pub fn digits_only(s: &str) -> String {
    s.chars().filter(char::is_ascii_digit).collect()
}

/// Value of the `env` / `wrapper` list entries as `(first, second)` pairs.
pub fn pairs(v: Option<&Value>, k1: &str, k2: &str) -> Vec<(String, String)> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|e| {
                    let s = |k: &str| e.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                    (s(k1), s(k2))
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn pairs_to_value(items: &[(String, String)], k1: &str, k2: &str) -> Value {
    Value::Array(items.iter().map(|(a, b)| json!({ k1: a, k2: b })).collect())
}

/// Steam Runtime ids and names (`SteamRuntime`).
pub const STEAM_RUNTIMES: [(&str, &str); 4] = [
    ("umu-scout", "Steam Linux Runtime 1.0 (scout)"),
    ("umu-soldier", "Steam Linux Runtime 2.0 (soldier)"),
    ("umu-sniper", "Steam Linux Runtime 3.0 (sniper)"),
    ("umu-steamrt4", "Steam Linux Runtime 4.0"),
];

/// Whether the Wine version offers the DXVK toggles (`AutoDXVK`).
pub fn offers_dxvk(w: &WineInstallation) -> bool {
    w.kind == "wine" && !w.bin.contains("toolkit") && !w.name.ends_with("-DXMT")
}

/// `PreferSystemLibs`: only for custom (non-system, non-Proton) Wine.
pub fn offers_system_libs(w: &WineInstallation) -> bool {
    w.kind != "proton" && w.kind != "crossover" && !w.bin.starts_with("/usr")
}

/// Number of GPUs (`/sys/class/drm/card<N>`), for the NVIDIA PRIME option.
pub fn count_gpus() -> usize {
    std::fs::read_dir("/sys/class/drm")
        .map(|d| {
            d.flatten()
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    n.strip_prefix("card").is_some_and(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()))
                })
                .count()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wine(name: &str, bin: &str, kind: &str) -> WineInstallation {
        WineInstallation { name: name.into(), bin: bin.into(), kind: kind.into(), ..Default::default() }
    }

    #[test]
    fn sections_follow_gamessettings() {
        assert_eq!(
            sections_for(RunKind::Wine, "legendary", true),
            [Section::Wine, Section::Other, Section::Advanced, Section::Saves, Section::Gamescope]
        );
        assert_eq!(sections_for(RunKind::Wine, "sideload", true), [Section::Wine, Section::Other, Section::Advanced, Section::Gamescope]);
        assert_eq!(sections_for(RunKind::Native, "sideload", false), [Section::Other, Section::Advanced, Section::Gamescope]);
        assert_eq!(sections_for(RunKind::Wine, "legendary", false).contains(&Section::Saves), false);
        assert!(sections_for(RunKind::None, "legendary", true).is_empty());
    }

    #[test]
    fn wine_names_and_options() {
        assert_eq!(display_wine_name("Proton-GE-Proton9-27"), "GE-Proton9-27");
        assert_eq!(display_wine_name("Proton-GE 8"), "GE-Proton 8");
        assert_eq!(display_wine_name("Wine-GE"), "Wine-GE");
        let list = vec![wine("A", "/a/wine", "wine")];
        let cur = wine("B", "/b/proton", "proton");
        let opts = wine_options(&list, &cur);
        assert_eq!(opts.len(), 2);
        assert_eq!(wine_options(&list, &wine("renamed", "/a/wine", "wine")).len(), 1);
        assert_eq!(wine_of(&serde_json::Map::new()).name, "Wine Default");
    }

    #[test]
    fn wine_value_roundtrip_skips_empty_extras() {
        let w = wine("GE", "/p/proton", "proton");
        assert_eq!(wine_to_value(&w), json!({"bin": "/p/proton", "name": "GE", "type": "proton"}));
        let mut full = w.clone();
        full.lib = Some("/l".into());
        assert_eq!(wine_to_value(&full)["lib"], "/l");
    }

    #[test]
    fn env_key_validation() {
        assert_eq!(validate_env_key(" "), "Variable names can't be empty");
        assert!(validate_env_key("A=B").contains("="));
        assert!(validate_env_key("A B").contains("spaces"));
        assert_eq!(validate_env_key("PROTON_LOG"), "");
    }

    #[test]
    fn launcher_args_warnings() {
        assert!(launcher_args_error("%command% -x").contains("%command%"));
        assert!(launcher_args_error("-a FOO=bar").contains("Environment"));
        assert_eq!(launcher_args_error("-windowed -w=800"), "");
        assert_eq!(launcher_args_error("A= x"), "");
    }

    #[test]
    fn pairs_roundtrip() {
        let v = json!([{"key": "A", "value": "1"}, {"key": "B"}]);
        let p = pairs(Some(&v), "key", "value");
        assert_eq!(p, [("A".to_string(), "1".to_string()), ("B".to_string(), String::new())]);
        assert_eq!(pairs_to_value(&p, "key", "value")[0], json!({"key": "A", "value": "1"}));
        assert!(pairs(None, "key", "value").is_empty());
    }

    #[test]
    fn dxvk_and_system_libs_rules() {
        assert!(offers_dxvk(&wine("W", "/w/wine", "wine")));
        assert!(!offers_dxvk(&wine("W", "/w/proton", "proton")));
        assert!(!offers_dxvk(&wine("W-DXMT", "/w/wine", "wine")));
        assert!(offers_system_libs(&wine("W", "/home/u/w/wine", "wine")));
        assert!(!offers_system_libs(&wine("W", "/usr/bin/wine", "wine")));
        assert!(!offers_system_libs(&wine("P", "/p/proton", "proton")));
        assert_eq!(digits_only("12a3"), "123");
    }
}
