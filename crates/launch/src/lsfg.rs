//! Lossless Scaling frame generation on Linux through lsfg-vk
//! (<https://lsfg-vk.dev>): a Vulkan layer that runs the frame generation
//! of the Windows app Lossless Scaling, whose DLL it loads from the app's
//! Steam folder. It works for Vulkan games and DirectX games through
//! DXVK / VKD3D-Proton; OpenGL is not supported.
//!
//! The layer is implicit: installed, it loads into every Vulkan program and
//! only turns on for a program it identifies. The game's settings
//! (`heroniteLsfg*`, Heronite keys) pick one of two ways:
//!
//! * Values set here: the environment carries them (lsfg-vk 2:
//!   `LSFGVK_ENV=1` + `LSFGVK_*`; lsfg-vk 1: `LSFG_LEGACY=1` + `LSFG_*`).
//! * A profile of lsfg-vk's own `conf.toml` (lsfg-vk 2 only), picked by
//!   name with `LSFGVK_PROFILE`. lsfg-vk watches that file, so changes made
//!   in `lsfg-vk-ui` apply while the game runs.
//!
//! The variables reach every Vulkan program of the game's process tree.

use std::path::{Path, PathBuf};

use heronite_i18n::tr;
use serde_json::{Map, Value};

use crate::env::Env;
use crate::probe::Probe;
use crate::settings::GlobalSettings;
use crate::steam::SteamLocator;

/// On/off.
pub const KEY_ENABLED: &str = "heroniteLsfg";
/// Frames shown per real frame, [`MULTIPLIERS`].
pub const KEY_MULTIPLIER: &str = "heroniteLsfgMultiplier";
/// Resolution of the motion estimation, in percent ([`FLOW_SCALES`]).
pub const KEY_FLOW_SCALE: &str = "heroniteLsfgFlowScale";
pub const KEY_PERFORMANCE: &str = "heroniteLsfgPerformance";
pub const KEY_HDR: &str = "heroniteLsfgHdr";
/// Name of a `conf.toml` profile; empty: the values above.
pub const KEY_PROFILE: &str = "heroniteLsfgProfile";
/// Steam games only: the DLL the launch options name (see
/// `heronite_steam::options`). Other games find it at launch.
pub const KEY_DLL: &str = "heroniteLsfgDll";
/// The keys new games inherit from the global defaults.
pub const INHERITED_KEYS: [&str; 6] = [KEY_ENABLED, KEY_MULTIPLIER, KEY_FLOW_SCALE, KEY_PERFORMANCE, KEY_HDR, KEY_PROFILE];

pub const MULTIPLIERS: [u32; 7] = [2, 3, 4, 5, 6, 7, 8];
pub const FLOW_SCALES: [u32; 16] = [25, 30, 35, 40, 45, 50, 55, 60, 65, 70, 75, 80, 85, 90, 95, 100];
pub const DEFAULT_MULTIPLIER: u32 = 2;
pub const DEFAULT_FLOW_SCALE: u32 = 100;

/// Steam app id of Lossless Scaling.
pub const STEAM_APP_ID: &str = "993090";
pub const WEBSITE: &str = "https://lsfg-vk.dev";

/// The game's frame generation settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lsfg {
    pub enabled: bool,
    /// `2..=8`.
    pub multiplier: u32,
    /// Percent, `25..=100`.
    pub flow_scale: u32,
    pub performance: bool,
    pub hdr: bool,
    /// Empty: the values above, not a profile.
    pub profile: String,
}

impl Default for Lsfg {
    fn default() -> Self {
        Self {
            enabled: false,
            multiplier: DEFAULT_MULTIPLIER,
            flow_scale: DEFAULT_FLOW_SCALE,
            performance: false,
            hdr: false,
            profile: String::new(),
        }
    }
}

impl Lsfg {
    /// Out-of-range numbers are clamped (lsfg-vk refuses to start with them).
    pub fn from_map(m: &Map<String, Value>) -> Self {
        let number = |key: &str| m.get(key).and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())));
        let flag = |key: &str| m.get(key) == Some(&Value::Bool(true));
        Self {
            enabled: flag(KEY_ENABLED),
            multiplier: number(KEY_MULTIPLIER).map_or(DEFAULT_MULTIPLIER, |n| n.clamp(2, 8) as u32),
            flow_scale: number(KEY_FLOW_SCALE).map_or(DEFAULT_FLOW_SCALE, |n| n.clamp(25, 100) as u32),
            performance: flag(KEY_PERFORMANCE),
            hdr: flag(KEY_HDR),
            profile: m.get(KEY_PROFILE).and_then(Value::as_str).map(str::trim).unwrap_or("").to_string(),
        }
    }
}

/// Major version of the installed layer: they read different variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    V1,
    V2,
}

/// What this machine has for frame generation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Install {
    /// `None`: lsfg-vk is not installed.
    pub layer: Option<Layer>,
    /// The DLL the layer needs, in a Steam library.
    pub dll: Option<String>,
    /// Lossless Scaling is installed but has no DLL for this layer (an old
    /// copy without `lsfg-vk.dll`).
    pub dll_outdated: bool,
    /// Profile names of lsfg-vk's `conf.toml` (lsfg-vk 2), in order.
    pub profiles: Vec<String>,
}

impl Install {
    /// Looks at the layer manifests, the Steam libraries and `conf.toml`.
    pub fn detect(global: &GlobalSettings, probe: &dyn Probe) -> Self {
        let Some(layer) = layer(probe) else { return Self::default() };
        let folders = app_folders(&SteamLocator::detect(global, probe).library_roots());
        let dll = folders.iter().map(|f| f.join(dll_name(layer))).find(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned());
        let profiles = match layer {
            Layer::V2 => std::fs::read_to_string(config_path(probe)).map(|t| profile_names(&t)).unwrap_or_default(),
            Layer::V1 => Vec::new(),
        };
        Self { layer: Some(layer), dll_outdated: dll.is_none() && !folders.is_empty(), dll, profiles }
    }

    /// Whether `profile` will be used: lsfg-vk 2 and a profile of its file.
    pub fn uses_profile(&self, profile: &str) -> bool {
        self.layer == Some(Layer::V2) && !profile.is_empty() && self.profiles.iter().any(|p| p == profile)
    }
}

/// File names of the layer manifests, newest major first.
const MANIFESTS: [(&str, Layer); 3] = [
    ("VkLayer_LSFGVK_frame_generation.json", Layer::V2),
    ("VkLayer_LSFGVK_frame_generation.x86_64.json", Layer::V2),
    ("VkLayer_LS_frame_generation.json", Layer::V1),
];

/// The installed layer, from the implicit layer folders the Vulkan loader
/// reads (`$XDG_CONFIG_HOME`, `/etc`, `$XDG_DATA_HOME`, `$XDG_DATA_DIRS`).
pub fn layer(probe: &dyn Probe) -> Option<Layer> {
    let home = probe.home();
    let var = |k: &str| probe.env_var(k).filter(|v| !v.is_empty());
    let mut dirs: Vec<String> = vec![
        var("XDG_CONFIG_HOME").unwrap_or_else(|| format!("{home}/.config")),
        "/etc/xdg".into(),
        "/etc".into(),
        var("XDG_DATA_HOME").unwrap_or_else(|| format!("{home}/.local/share")),
    ];
    // An AppImage may replace XDG_DATA_DIRS: the system folders are always looked at.
    let data_dirs = var("XDG_DATA_DIRS").unwrap_or_default();
    dirs.extend(data_dirs.split(':').filter(|d| !d.is_empty()).map(str::to_string));
    dirs.extend(["/usr/local/share".to_string(), "/usr/share".to_string()]);
    MANIFESTS.iter().find_map(|(name, layer)| {
        dirs.iter().any(|d| probe.exists(&format!("{}/vulkan/implicit_layer.d/{name}", d.trim_end_matches('/')))).then_some(*layer)
    })
}

fn dll_name(layer: Layer) -> &'static str {
    match layer {
        Layer::V2 => "lsfg-vk.dll",
        Layer::V1 => "Lossless.dll",
    }
}

/// `steamapps/common/Lossless Scaling` of each library that has it.
fn app_folders(roots: &[PathBuf]) -> Vec<PathBuf> {
    roots.iter().map(|r| r.join("steamapps/common/Lossless Scaling")).filter(|p| p.is_dir()).collect()
}

/// Whether lsfg-vk 2 finds `dll` by itself: it only looks in the usual
/// Steam folders of the home.
pub fn found_by_layer(dll: &str, home: &str) -> bool {
    const STEAM: [&str; 5] = [
        ".local/share/Steam",
        ".steam/steam",
        ".steam/debian-installation",
        ".var/app/com.valvesoftware.Steam/.local/share/Steam",
        "snap/steam/common/.local/share/Steam",
    ];
    let home = home.trim_end_matches('/');
    STEAM.iter().any(|s| dll == format!("{home}/{s}/steamapps/common/Lossless Scaling/lsfg-vk.dll"))
}

/// lsfg-vk 2's `conf.toml`, where lsfg-vk looks for it.
pub fn config_path(probe: &dyn Probe) -> PathBuf {
    if let Some(p) = probe.env_var("LSFGVK_CONFIG").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    let base = probe.env_var("XDG_CONFIG_HOME").filter(|p| !p.is_empty()).unwrap_or_else(|| format!("{}/.config", probe.home()));
    Path::new(&base).join("lsfg-vk/conf.toml")
}

/// The `[[profile]]` names of a `conf.toml`; nothing when it does not parse.
pub fn profile_names(text: &str) -> Vec<String> {
    let Ok(table) = text.parse::<toml::Table>() else { return Vec::new() };
    let Some(toml::Value::Array(list)) = table.get("profile") else { return Vec::new() };
    list.iter().filter_map(|p| p.get("name")?.as_str()).filter(|n| !n.is_empty()).map(str::to_string).collect()
}

/// The variables for `s` with `layer`. `dll`: the DLL to name; without one
/// lsfg-vk looks in the usual Steam folders of the home itself.
pub fn env_for(s: &Lsfg, layer: Layer, dll: Option<&str>, use_profile: bool) -> Env {
    let mut env = Env::new();
    let mut set = |k: &str, v: String| {
        env.insert(k.to_string(), v);
    };
    let bit = |b: bool| if b { "1" } else { "0" }.to_string();
    let flow = format!("{:.2}", s.flow_scale as f32 / 100.0);
    match layer {
        Layer::V2 => {
            if let Some(d) = dll {
                set("LSFGVK_DLL_PATH", d.to_string());
            }
            if use_profile {
                set("LSFGVK_PROFILE", s.profile.clone());
                return env;
            }
            set("LSFGVK_ENV", "1".into());
            set("LSFGVK_MULTIPLIER", s.multiplier.to_string());
            set("LSFGVK_FLOW_SCALE", flow);
            set("LSFGVK_PERFORMANCE_MODE", bit(s.performance));
            if s.hdr {
                set("LSFGVK_COLOR_SPACE", "hdr10".into());
                set("LSFGVK_TRANSFER_FUNCTION", "pq".into());
            }
        }
        Layer::V1 => {
            if let Some(d) = dll {
                set("LSFG_DLL_PATH", d.to_string());
            }
            set("LSFG_LEGACY", "1".into());
            set("LSFG_MULTIPLIER", s.multiplier.to_string());
            set("LSFG_FLOW_SCALE", flow);
            set("LSFG_PERFORMANCE_MODE", bit(s.performance));
            set("LSFG_HDR_MODE", bit(s.hdr));
        }
    }
    env
}

/// Folders the Steam Linux Runtime's container shares without being told.
const CONTAINER_SHARED: [&str; 4] = ["/mnt/", "/media/", "/run/media/", "/tmp/"];

/// Adds the frame generation variables of a launch to `env` when the game
/// turns it on. `Err`: why the game cannot start with it (lsfg-vk or the
/// DLL is missing), like a missing MangoHud stops a launch.
pub fn apply(env: &mut Env, s: &Lsfg, global: &GlobalSettings, probe: &dyn Probe) -> Result<(), String> {
    if !s.enabled {
        return Ok(());
    }
    let install = Install::detect(global, probe);
    let Some(layer) = install.layer else {
        return Err(tr!("Lossless Scaling frame generation is on, but lsfg-vk is not installed").to_string());
    };
    let use_profile = install.uses_profile(&s.profile);
    // A profile run may still find the DLL through `conf.toml`.
    if install.dll.is_none() && !use_profile {
        return Err(match install.dll_outdated {
            true => tr!(
                "Lossless Scaling frame generation is on, but this copy of Lossless Scaling is too old for lsfg-vk. Update it in Steam."
            ),
            false => tr!("Lossless Scaling frame generation is on, but Lossless Scaling was not found in your Steam libraries"),
        }
        .to_string());
    }
    if !s.profile.is_empty() && !use_profile {
        log::warn!("lsfg-vk profile {:?} not found; using the game's own frame generation values", s.profile);
    }
    env.extend(env_for(s, layer, install.dll.as_deref(), use_profile));
    if let Some(dll) = &install.dll {
        share_with_container(env, dll, probe);
    }
    Ok(())
}

/// Games run by umu or the Steam runtime see the home and a few folders; a
/// DLL in a Steam library elsewhere gets its folder shared too.
fn share_with_container(env: &mut Env, dll: &str, probe: &dyn Probe) {
    use crate::shared_folders::{container_env, ENV_KEY};
    let home = format!("{}/", probe.home().trim_end_matches('/'));
    if dll.starts_with(&home) || CONTAINER_SHARED.iter().any(|d| dll.starts_with(d)) {
        return;
    }
    let Some(dir) = Path::new(dll).parent().map(|d| d.to_string_lossy().into_owned()) else { return };
    let existing = [env.get(ENV_KEY).cloned().or_else(|| probe.env_var(ENV_KEY))];
    if let Some(v) = container_env(&[dir], &existing) {
        env.insert(ENV_KEY.into(), v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::FakeProbe;
    use serde_json::json;

    fn lsfg(v: Value) -> Lsfg {
        Lsfg::from_map(v.as_object().unwrap())
    }

    #[test]
    fn settings_read_with_defaults_and_clamps() {
        assert_eq!(lsfg(json!({})), Lsfg::default());
        let s = lsfg(json!({
            "heroniteLsfg": true, "heroniteLsfgMultiplier": 30, "heroniteLsfgFlowScale": "10",
            "heroniteLsfgPerformance": true, "heroniteLsfgProfile": " Fast "
        }));
        assert!(s.enabled && s.performance && !s.hdr);
        assert_eq!((s.multiplier, s.flow_scale, s.profile.as_str()), (8, 25, "Fast"));
    }

    #[test]
    fn layer_found_in_the_loader_folders() {
        let none = FakeProbe::new("/h");
        assert_eq!(layer(&none), None);
        let v2 = FakeProbe::new("/h").with_path("/usr/share/vulkan/implicit_layer.d/VkLayer_LSFGVK_frame_generation.json");
        assert_eq!(layer(&v2), Some(Layer::V2));
        let v1 = FakeProbe::new("/h").with_path("/h/.local/share/vulkan/implicit_layer.d/VkLayer_LS_frame_generation.json");
        assert_eq!(layer(&v1), Some(Layer::V1));
        let xdg = FakeProbe::new("/h")
            .with_env("XDG_DATA_DIRS", "/nix/profile/share/")
            .with_path("/nix/profile/share/vulkan/implicit_layer.d/VkLayer_LSFGVK_frame_generation.json");
        assert_eq!(layer(&xdg), Some(Layer::V2));
    }

    #[test]
    fn profile_names_of_the_config() {
        let text = r#"
version = 2
[global]
dll = "/x/lsfg-vk.dll"
[[profile]]
name = "4x FG"
multiplier = 4
[[profile]]
multiplier = 2
[[profile]]
name = "Control"
"#;
        assert_eq!(profile_names(text), ["4x FG", "Control"]);
        assert!(profile_names("not toml [").is_empty());
    }

    #[test]
    fn variables_for_each_layer() {
        let s = Lsfg { enabled: true, multiplier: 3, flow_scale: 75, hdr: true, ..Default::default() };
        let v2 = env_for(&s, Layer::V2, Some("/l/lsfg-vk.dll"), false);
        assert_eq!(v2["LSFGVK_ENV"], "1");
        assert_eq!(v2["LSFGVK_MULTIPLIER"], "3");
        assert_eq!(v2["LSFGVK_FLOW_SCALE"], "0.75");
        assert_eq!(v2["LSFGVK_PERFORMANCE_MODE"], "0");
        assert_eq!(v2["LSFGVK_COLOR_SPACE"], "hdr10");
        assert_eq!(v2["LSFGVK_TRANSFER_FUNCTION"], "pq");
        assert_eq!(v2["LSFGVK_DLL_PATH"], "/l/lsfg-vk.dll");
        let v1 = env_for(&s, Layer::V1, None, false);
        assert_eq!(v1["LSFG_LEGACY"], "1");
        assert_eq!(v1["LSFG_FLOW_SCALE"], "0.75");
        assert_eq!(v1["LSFG_HDR_MODE"], "1");
        assert!(!v1.contains_key("LSFG_DLL_PATH"));
        let profile = Lsfg { profile: "Control".into(), ..s };
        let p = env_for(&profile, Layer::V2, None, true);
        assert_eq!(p.into_iter().collect::<Vec<_>>(), [("LSFGVK_PROFILE".to_string(), "Control".to_string())]);
    }

    #[test]
    fn off_adds_nothing_and_missing_layer_stops_the_launch() {
        let mut env = Env::new();
        let probe = FakeProbe::new("/h");
        assert!(apply(&mut env, &Lsfg::default(), &GlobalSettings::default(), &probe).is_ok());
        assert!(env.is_empty());
        let on = Lsfg { enabled: true, ..Default::default() };
        let err = apply(&mut env, &on, &GlobalSettings::default(), &probe).unwrap_err();
        assert!(err.contains("lsfg-vk is not installed"));
    }

    #[test]
    fn dll_outside_the_home_is_shared_with_the_container() {
        let probe = FakeProbe::new("/home/u").with_env("PRESSURE_VESSEL_FILESYSTEMS_RW", "/srv");
        let mut env = Env::new();
        share_with_container(&mut env, "/data/Steam/steamapps/common/Lossless Scaling/lsfg-vk.dll", &probe);
        assert_eq!(env["PRESSURE_VESSEL_FILESYSTEMS_RW"], "/srv:/data/Steam/steamapps/common/Lossless Scaling");
        let mut home = Env::new();
        share_with_container(&mut home, "/home/u/.local/share/Steam/steamapps/common/Lossless Scaling/lsfg-vk.dll", &probe);
        share_with_container(&mut home, "/mnt/games/steamapps/common/Lossless Scaling/lsfg-vk.dll", &probe);
        assert!(home.is_empty());
    }

    #[test]
    fn the_layer_finds_dlls_of_the_home_steam_only() {
        assert!(found_by_layer("/h/.local/share/Steam/steamapps/common/Lossless Scaling/lsfg-vk.dll", "/h/"));
        assert!(!found_by_layer("/h/Games/SteamLibrary/steamapps/common/Lossless Scaling/lsfg-vk.dll", "/h"));
        assert!(!found_by_layer("/data/steamapps/common/Lossless Scaling/lsfg-vk.dll", "/h"));
    }

    #[test]
    fn detect_finds_the_dll_of_the_layer() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_string_lossy().into_owned();
        let app = dir.path().join(".local/share/Steam/steamapps/common/Lossless Scaling");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("Lossless.dll"), b"").unwrap();
        let manifest = format!("{home}/.local/share/vulkan/implicit_layer.d/VkLayer_LSFGVK_frame_generation.json");
        let probe = FakeProbe::new(&home).with_path(&format!("{home}/.local/share/Steam")).with_path(&manifest);
        let old = Install::detect(&GlobalSettings::default(), &probe);
        assert_eq!((old.layer, old.dll.as_deref(), old.dll_outdated), (Some(Layer::V2), None, true));
        std::fs::write(app.join("lsfg-vk.dll"), b"").unwrap();
        let new = Install::detect(&GlobalSettings::default(), &probe);
        assert_eq!(new.dll, Some(app.join("lsfg-vk.dll").to_string_lossy().into_owned()));
        assert!(!new.dll_outdated);
    }
}
