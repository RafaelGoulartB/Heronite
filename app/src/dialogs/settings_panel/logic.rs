//! Pure rules of the Heroic settings tabs (no UI): which tabs a game gets,
//! how Wine versions are named and which toggles depend on which. Ports of
//! the conditions in `Settings/sections/GamesSettings` and `components/*`.

use heronite_data::collection::Game;
use heronite_i18n::tr;
use heronite_launch::native::{effective_platform, Platform};
use heronite_launch::SystemProbe;
use heronite_wine::WineInstallation;
use serde_json::{json, Value};

/// Groups of the game settings, by the question the player has (the General
/// one only exists for the global settings). Heroic's tabs are spread over
/// them: Wine (or a Steam game's Proton) is Compatibility, Other and Gamescope
/// are Graphics and Performance, Advanced is Start.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Section {
    General,
    /// What runs when Play is pressed, and with what: arguments, scripts,
    /// programs alongside, variables (the game settings add Play and the
    /// actions).
    Start,
    /// How a Windows game runs here: Wine / Proton, the prefix, DXVK, sync,
    /// anti-cheat, the sandbox.
    Compatibility,
    /// How it looks: the graphics card, HDR, VSync, every upscaler,
    /// gamescope, ReShade.
    Graphics,
    /// How it runs: overlays, the FPS limit, frame generation, latency,
    /// system tweaks.
    Performance,
    /// The store's cloud saves (the game settings add the backups).
    Saves,
}

impl Section {
    pub fn label(self) -> &'static str {
        match self {
            Section::General => tr!("General"),
            Section::Start => tr!("Start"),
            Section::Compatibility => tr!("Compatibility"),
            Section::Graphics => tr!("Graphics"),
            Section::Performance => tr!("Performance"),
            Section::Saves => tr!("Saves"),
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
    /// Installed Steam game: the settings are Steam's own (`heronite_steam`).
    Steam,
    /// Installed Steam game that Steam starts as it is (Windows): what
    /// Heronite runs around it, the scripts and the programs alongside
    /// (`heronite_launch::steam_extras`), and Steam's launch options as text
    /// ([`steam_line`]).
    SteamDirect,
    /// Browser games, emulated ROMs, not installed: no Heroic settings.
    None,
}

impl RunKind {
    /// Whether ReShade can go into the game (`Some`), and whether Wine or
    /// Proton runs it (`true`: the game needs `WINEDLLOVERRIDES`). A Steam
    /// game on Linux may be its Linux build: the files say, once read
    /// (`heronite_launch::reshade::steam`); under Proton the override goes in
    /// its launch options. Emulated games have none: ReShade goes next to a
    /// program, and the emulator's is every game's it runs.
    pub fn reshade(self) -> Option<bool> {
        match self {
            RunKind::Wine => Some(true),
            RunKind::Steam => Some(cfg!(not(windows))),
            RunKind::SteamDirect | RunKind::Native if cfg!(windows) => Some(false),
            _ => None,
        }
    }
}

/// What the settings need to know about one game.
#[derive(Clone, Debug)]
pub struct GameFacts {
    pub app_name: String,
    pub title: String,
    pub runner: String,
    pub run_kind: RunKind,
    pub cloud_saves: bool,
    /// Steam's app id of a [`RunKind::Steam`] game, or of a
    /// [`RunKind::SteamDirect`] one whose launch options show ([`steam_line`]).
    pub steam_app_id: Option<String>,
    /// The Steam id ProtonDB knows the game by (its own or the metadata's).
    pub protondb_id: Option<String>,
    /// The game itself, to show how it starts.
    pub game: Option<Game>,
    /// The game was put in the sandbox (when it was added or installed),
    /// where there is one: Compatibility ends with its switches. Decided
    /// once, so turning it off there keeps them.
    pub sandbox: bool,
}

impl GameFacts {
    /// Reads what is needed from a Collection game. Touches the disk lightly
    /// (the first bytes of the executable of sideloaded games).
    pub fn of(game: &Game) -> Self {
        let g = &game.game;
        let platform = g.install.platform.clone().unwrap_or_default().to_ascii_lowercase();
        let launch_kind = game.launch_kind.as_deref().unwrap_or("");
        let steam_app_id = game.steam_launch_id.clone().filter(|_| game.installed && launch_kind == "steam-uri");
        let run_kind = if steam_app_id.is_some() {
            steam_run_kind(
                heronite_steam::PROTON,
                heronite_platform::supported(heronite_platform::Feature::SteamGameSettings),
                heronite_launch::steam_extras::supported(),
            )
        } else if !game.installed
            || matches!(
                launch_kind,
                "emulator"
                    | "steam-uri"
                    | heronite_data::launchers::LAUNCH_KIND
                    | heronite_data::clients::LAUNCH_KIND
                    | heronite_data::xbox::LAUNCH_KIND
            )
            || platform == "browser"
            || g.rest.get("browserUrl").is_some_and(|v| !v.is_null() && v != "")
        {
            RunKind::None
        } else if g.runner == "sideload" || g.runner == "zoom" {
            let exe = g.install.executable.clone().unwrap_or_default();
            // On Windows a Windows program is the native one, and a Linux
            // one does not run at all.
            match effective_platform(g, game.meta.as_deref(), &exe, &SystemProbe) {
                Platform::Linux if cfg!(windows) => RunKind::None,
                Platform::Windows if cfg!(windows) => RunKind::Native,
                Platform::Linux => RunKind::Native,
                Platform::Windows => RunKind::Wine,
            }
        } else if platform.contains("linux") || (platform.is_empty() && g.is_linux_native == Some(true)) {
            // The installed build decides: a GOG game with a Linux build may
            // be installed as its Windows build. Linux builds do not run on Windows.
            if cfg!(windows) {
                RunKind::None
            } else {
                RunKind::Native
            }
        } else if platform.contains("mac") || platform == "osx" {
            RunKind::None
        } else if cfg!(windows) {
            // A store's Windows build runs as it is on Windows.
            RunKind::Native
        } else {
            RunKind::Wine
        };
        let cloud_saves = if game.runner == "gog" {
            // Two small files, no network: the tab list is built on the UI thread.
            run_kind != RunKind::None && heronite_gog::saves::supported(&heronite_data::DataPaths::detect(), &game.app_name)
        } else {
            g.rest.get("cloud_save_enabled").and_then(Value::as_bool).unwrap_or(false)
        };
        let protondb_id =
            [steam_app_id.as_deref(), game.steam_app_id.as_deref(), game.metadata.as_ref().and_then(|m| m.steam_app_id.as_deref())]
                .into_iter()
                .flatten()
                .find(|s| !s.is_empty())
                .map(str::to_owned);
        // Only a game whose settings (or launch options) are Steam's reads them from Steam.
        let steam_app_id = steam_app_id.filter(|_| run_kind == RunKind::Steam || (run_kind == RunKind::SteamDirect && steam_line()));
        // Steam starts its games itself: they cannot run in the sandbox.
        let sandbox = matches!(run_kind, RunKind::Wine | RunKind::Native)
            && heronite_platform::supported(heronite_platform::Feature::Sandbox)
            && sandboxed(&game.app_name);
        Self {
            app_name: game.app_name.clone(),
            title: game.title.clone(),
            runner: game.runner.clone(),
            run_kind,
            cloud_saves,
            protondb_id,
            steam_app_id,
            game: Some(game.clone()),
            sandbox,
        }
    }

    pub fn is_gog(&self) -> bool {
        self.runner == "gog"
    }

    pub fn is_sideload(&self) -> bool {
        self.runner == "sideload" || self.runner == "zoom"
    }

    /// Groups shown for this game, in order.
    pub fn sections(&self) -> Vec<Section> {
        sections_for(self.run_kind, &self.runner, self.cloud_saves, self.sandbox)
    }

    /// The group the game settings open on (Heroic's `getStartingTab`).
    pub fn start_section(&self) -> Option<Section> {
        match self.run_kind {
            RunKind::Wine | RunKind::Steam => Some(Section::Compatibility),
            RunKind::Native if !wrappers() => Some(Section::Start),
            RunKind::SteamDirect => Some(Section::Start),
            RunKind::Native => Some(Section::Performance),
            RunKind::None => None,
        }
    }
}

/// The game's settings have the sandbox on (a tiny read of its config).
fn sandboxed(app_name: &str) -> bool {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    let settings = heronite_data::game_config::load_game(&heronite_data::DataPaths::detect(), app_name, &home);
    heronite_launch::sandbox::Sandbox::from_map(&settings).enabled
}

/// Whether the launch extras exist here (`heronite_platform::Feature::LaunchExtras`):
/// scripts, programs alongside the game and presets.
pub fn extras() -> bool {
    heronite_platform::supported(heronite_platform::Feature::LaunchExtras)
}

/// How an installed Steam game runs. With Proton (Linux) the game's settings
/// are Steam's: its launch options and compatibility tool. Without (Windows)
/// Steam starts the game as it is: Heronite runs the `extras` around it, and
/// Steam's own line (`settings`) is edited as text.
fn steam_run_kind(proton: bool, settings: bool, extras: bool) -> RunKind {
    match (proton, settings) {
        (true, true) => RunKind::Steam,
        (false, true) => RunKind::SteamDirect,
        _ if extras => RunKind::SteamDirect,
        _ => RunKind::None,
    }
}

/// Whether a [`RunKind::SteamDirect`] game shows Steam's launch options
/// (`heronite_platform::Feature::SteamGameSettings` on Windows): the line
/// Steam adds to the game's command, edited as text. No compatibility tool:
/// Steam starts Windows games as they are.
pub fn steam_line() -> bool {
    !heronite_steam::PROTON && heronite_platform::supported(heronite_platform::Feature::SteamGameSettings)
}

/// Whether Linux's wrappers exist here (`heronite_platform::Feature::Wrappers`):
/// without them a native game only has the Launch tab.
pub fn wrappers() -> bool {
    heronite_platform::supported(heronite_platform::Feature::Wrappers)
}

/// Whether the GPU tab picks Windows' per-program GPU preference instead of
/// Linux's variables (`heronite_launch::gpu::preference`): Windows, where
/// Heronite starts the game's program itself.
pub fn windows_gpu() -> bool {
    cfg!(windows)
}

/// A game Steam starts as it is, or one without Linux's tools (wrappers,
/// overlays, gamescope; Windows): no Performance group, and Graphics holds
/// what applies (ReShade, Windows' GPU preference).
pub fn reshade_only(run_kind: RunKind) -> bool {
    run_kind == RunKind::SteamDirect || (run_kind == RunKind::Native && !wrappers())
}

/// Whether a game gets cloud saves in its Saves group: an Epic or GOG game
/// with cloud saves whose Windows build runs through Wine, or as it is on
/// Windows (the stores resolve the save folders of this PC then).
pub fn saves_tab(run_kind: RunKind, runner: &str, cloud_saves: bool) -> bool {
    cloud_saves && matches!(runner, "legendary" | "gog") && (run_kind == RunKind::Wine || (run_kind == RunKind::Native && cfg!(windows)))
}

/// The panel's groups of a game, in order. `sandbox`: the game is in the
/// sandbox; its switches end Compatibility, which a native game gets then.
pub fn sections_for(run_kind: RunKind, runner: &str, cloud_saves_tab: bool, sandbox: bool) -> Vec<Section> {
    let mut out = vec![Section::Start];
    match run_kind {
        RunKind::None => return Vec::new(),
        _ if reshade_only(run_kind) => {
            // ReShade, and Windows' own GPU preference (Steam starts its
            // games itself: Heronite cannot set it for the program it runs).
            if run_kind.reshade().is_some() || (run_kind == RunKind::Native && windows_gpu()) {
                out.push(Section::Graphics);
            }
        }
        RunKind::Native if !sandbox => out.extend([Section::Graphics, Section::Performance]),
        // Steam and Wine (Steam's own start is `reshade_only`), and a native
        // game in the sandbox.
        _ => out.extend([Section::Compatibility, Section::Graphics, Section::Performance]),
    }
    if saves_tab(run_kind, runner, cloud_saves_tab) {
        out.push(Section::Saves);
    }
    out
}

/// Groups of the global "Game defaults" editor.
pub const DEFAULT_SECTIONS: [Section; 4] = [Section::Compatibility, Section::Graphics, Section::Performance, Section::Start];

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
/// adds it when it is still valid), in the order of [`wine_group`].
pub fn wine_options(list: &[WineInstallation], current: &WineInstallation) -> Vec<WineInstallation> {
    let mut out = list.to_vec();
    if !out.iter().any(|w| w.name == current.name || (!w.bin.is_empty() && w.bin == current.bin)) {
        out.push(current.clone());
    }
    out.sort_by_key(wine_group_rank);
    out
}

fn wine_group_rank(w: &WineInstallation) -> u8 {
    match w.kind.as_str() {
        "proton" if w.bin.contains("/steamapps/common/") => 1,
        "proton" => 0,
        "wine" => 2,
        _ => 3,
    }
}

/// Heading of the menu group `w` is listed under: the Proton builds you
/// installed (GE-Proton...), Valve's, Wine builds, the rest.
pub fn wine_group(w: &WineInstallation) -> &'static str {
    match wine_group_rank(w) {
        0 => tr!("Your versions"),
        1 => "Valve",
        2 => "Wine",
        _ => tr!("Other"),
    }
}

pub fn wine_to_value(w: &WineInstallation) -> Value {
    serde_json::to_value(w).unwrap_or_else(|_| json!({}))
}

/// `validateEnvKey`; empty = valid.
pub fn validate_env_key(key: &str) -> &'static str {
    let key = key.trim();
    if key.is_empty() {
        tr!("Variable names can't be empty")
    } else if key.contains('=') {
        tr!("Variable names can't contain the \"=\" sign")
    } else if key.contains(' ') {
        tr!("Variable names can't contain spaces")
    } else {
        ""
    }
}

/// The variables as text, one `NAME=value` per line (the bulk editor).
pub fn env_block(rows: &[(String, String)]) -> String {
    rows.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n")
}

/// Reads variables pasted as text: one `NAME=value` per line, or several on
/// a line the way launch options are written (`A=1 B=2 %command%`, as
/// ProtonDB reports show them). `export` and blank or `#` lines are skipped;
/// a later line wins over an earlier one with the same name. `Err` names
/// the first line that is not an assignment.
pub fn parse_env_block(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut put = |k: String, v: String| match out.iter().position(|(n, _)| *n == k) {
        Some(i) => out[i].1 = v,
        None => out.push((k, v)),
    };
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        let line = line.strip_prefix("export ").map(str::trim).unwrap_or(line);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Several assignments on one line (everything up to %command%).
        let words: Vec<String> = shlex::split(line).unwrap_or_default();
        let before_command: Vec<&String> = words.iter().take_while(|w| !w.contains("%command%")).collect();
        let several = words.len() > 1
            && !before_command.is_empty()
            && before_command.iter().all(|w| w.split_once('=').is_some_and(|(k, _)| validate_env_key(k).is_empty()));
        if several {
            for w in before_command {
                let (k, v) = w.split_once('=').expect("checked above");
                put(k.to_string(), v.to_string());
            }
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            return Err(tr!("Line {n}: \"{line}\" is not NAME=value", n = n + 1, line));
        };
        let k = k.trim();
        let err = validate_env_key(k);
        if !err.is_empty() {
            return Err(format!("{} {err}", tr!("Line {n}:", n = n + 1)));
        }
        put(k.to_string(), remove_quotes(v.trim()).to_string());
    }
    Ok(out)
}

/// `"value"` or `'value'` -> `value`.
fn remove_quotes(v: &str) -> &str {
    for q in ['"', '\''] {
        if v.len() >= 2 && v.starts_with(q) && v.ends_with(q) {
            return &v[1..v.len() - 1];
        }
    }
    v
}

/// `LauncherArgs` warnings.
pub fn launcher_args_error(args: &str) -> &'static str {
    if args.contains("%command") {
        tr!("The %command% syntax from Steam is not valid as game arguments.")
    } else if has_env_assignment(args) {
        tr!("Environment variables must be configured in the table below.")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn wine(name: &str, bin: &str, kind: &str) -> WineInstallation {
        WineInstallation { name: name.into(), bin: bin.into(), kind: kind.into(), ..Default::default() }
    }

    #[test]
    fn groups_follow_what_the_game_has() {
        use Section::*;
        assert_eq!(sections_for(RunKind::Wine, "legendary", true, false), [Start, Compatibility, Graphics, Performance, Saves]);
        assert_eq!(sections_for(RunKind::Wine, "sideload", true, false), [Start, Compatibility, Graphics, Performance]);
        // The sandbox is a block of Compatibility, not a group of its own.
        assert_eq!(sections_for(RunKind::Wine, "sideload", false, true), [Start, Compatibility, Graphics, Performance]);
        // Without Linux's wrappers (Windows), a native game has Start, and
        // Graphics where ReShade or Windows' GPU preference applies.
        let native: &[Section] = match (wrappers(), cfg!(windows)) {
            (true, _) => &[Start, Graphics, Performance],
            (false, true) => &[Start, Graphics],
            (false, false) => &[Start],
        };
        assert_eq!(sections_for(RunKind::Native, "sideload", false, false), native);
        if wrappers() {
            // A sandboxed native game gets Compatibility for the sandbox.
            assert_eq!(sections_for(RunKind::Native, "sideload", false, true), [Start, Compatibility, Graphics, Performance]);
        }
        assert!(!sections_for(RunKind::Wine, "legendary", false, false).contains(&Saves));
        assert!(sections_for(RunKind::None, "legendary", true, true).is_empty());
        assert!(sections_for(RunKind::Wine, "gog", true, false).contains(&Saves));
        assert!(!sections_for(RunKind::Wine, "gog", false, false).contains(&Saves));
        assert_eq!(sections_for(RunKind::Steam, "sideload", true, false), [Start, Compatibility, Graphics, Performance]);
        let direct: &[Section] = if cfg!(windows) { &[Start, Graphics] } else { &[Start] };
        assert_eq!(sections_for(RunKind::SteamDirect, "sideload", true, false), direct);
        // Game defaults: no Saves (it is a game's own), Compatibility first.
        assert_eq!(DEFAULT_SECTIONS[0], Compatibility);
        assert!(!DEFAULT_SECTIONS.contains(&Saves));
    }

    /// Windows builds that run as they are on Windows sync their cloud saves
    /// too; Linux builds do not (the stores only list Windows folders).
    #[test]
    fn native_windows_games_get_cloud_saves() {
        for runner in ["legendary", "gog"] {
            assert_eq!(sections_for(RunKind::Native, runner, true, false).contains(&Section::Saves), cfg!(windows), "{runner}");
            assert!(!sections_for(RunKind::Native, runner, false, false).contains(&Section::Saves), "{runner}");
        }
        assert!(!sections_for(RunKind::Native, "sideload", true, false).contains(&Section::Saves));
        assert!(!sections_for(RunKind::SteamDirect, "legendary", true, false).contains(&Section::Saves));
        if cfg!(windows) && !wrappers() {
            use Section::*;
            assert_eq!(sections_for(RunKind::Native, "legendary", true, false), [Start, Graphics, Saves]);
        }
    }

    #[test]
    fn steam_games_get_proton_on_linux_and_the_line_alone_on_windows() {
        // Linux: the settings are Steam's, Proton included.
        assert_eq!(steam_run_kind(true, true, false), RunKind::Steam);
        assert_eq!(steam_run_kind(true, false, false), RunKind::None);
        // Windows: Steam starts the game as it is; its launch options show
        // with the extras, or alone.
        assert_eq!(steam_run_kind(false, true, true), RunKind::SteamDirect);
        assert_eq!(steam_run_kind(false, true, false), RunKind::SteamDirect);
        assert_eq!(steam_run_kind(false, false, true), RunKind::SteamDirect);
        assert_eq!(steam_run_kind(false, false, false), RunKind::None);
        // Only Linux gets the Proton tabs.
        assert_eq!(heronite_steam::PROTON, cfg!(target_os = "linux"));
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

    #[test]
    fn env_text_reads_lines_and_protondb_launch_options() {
        let pairs = |v: &[(&str, &str)]| v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();
        let text = "# from ProtonDB\nPROTON_USE_WINED3D=1\n\nexport DXVK_ASYNC=1\nWINEDLLOVERRIDES=\"winmm,version=n,b\"\nMY_PATH=/a b/c";
        assert_eq!(
            parse_env_block(text).unwrap(),
            pairs(&[("PROTON_USE_WINED3D", "1"), ("DXVK_ASYNC", "1"), ("WINEDLLOVERRIDES", "winmm,version=n,b"), ("MY_PATH", "/a b/c")])
        );
        assert_eq!(
            parse_env_block("PROTON_NO_ESYNC=1 PROTON_NO_FSYNC=1 %command% -dx11").unwrap(),
            pairs(&[("PROTON_NO_ESYNC", "1"), ("PROTON_NO_FSYNC", "1")])
        );
        assert_eq!(parse_env_block("A=1\nA=2").unwrap(), pairs(&[("A", "2")]), "the last one wins");
        assert!(parse_env_block("A=1\njust words").unwrap_err().contains("Line 2"));
        assert!(parse_env_block("BAD KEY=1").is_err());
        assert_eq!(parse_env_block("").unwrap(), Vec::new());
        assert_eq!(env_block(&pairs(&[("A", "1"), ("B", "x y")])), "A=1\nB=x y");
    }
}
