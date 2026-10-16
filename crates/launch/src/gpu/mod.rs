//! A game's graphics card settings (`heroniteGpu*`, Heronite keys): which
//! card draws it, the NVIDIA driver's frame generation (Smooth Motion), the
//! upscaler upgrades of GE-Proton and Proton-CachyOS, Mesa's switches and
//! what DXVK tells DirectX games about the card. Every one of them is
//! environment variables; [`detect`] says what this computer has.
//!
//! Steam games carry the same variables in their launch options:
//! [`launch_options_vars`] writes them and [`read_launch_options`] reads
//! them back.

pub mod detect;

use serde_json::{json, Map, Value};

use crate::env::Env;
use crate::probe::Probe;

/// PCI address of the card that draws the game (`0000:01:00.0`). Empty: the
/// system's choice, or Heroic's `nvidiaPrime` ("use the dedicated GPU").
pub const KEY_DEVICE: &str = "heroniteGpuDevice";
/// Heroic's switch for the NVIDIA card of a hybrid laptop.
pub const KEY_PRIME: &str = "nvidiaPrime";
/// NVIDIA Smooth Motion: the driver's frame generation for Vulkan and
/// DirectX games (RTX 40 and newer, driver 575 and newer).
pub const KEY_SMOOTH_MOTION: &str = "heroniteGpuSmoothMotion";
/// Reflex in Vulkan games, through dxvk-nvapi's layer (GE-Proton and
/// Proton-CachyOS ship it; DirectX games have Reflex without it).
pub const KEY_REFLEX: &str = "heroniteGpuReflex";
/// A [`SHADER_CACHE_BYTES`] NVIDIA shader cache that is never trimmed.
pub const KEY_SHADER_CACHE: &str = "heroniteGpuShaderCache";
/// GE-Proton / Proton-CachyOS download the newest DLSS into the prefix.
pub const KEY_DLSS_UPGRADE: &str = "heroniteGpuDlssUpgrade";
/// [`DlssPreset::id`]; only used with [`KEY_DLSS_UPGRADE`].
pub const KEY_DLSS_PRESET: &str = "heroniteGpuDlssPreset";
pub const KEY_DLSS_INDICATOR: &str = "heroniteGpuDlssIndicator";
/// FSR 3.1 games get FSR 4 (RDNA 4 cards).
pub const KEY_FSR4_UPGRADE: &str = "heroniteGpuFsr4Upgrade";
pub const KEY_FSR4_INDICATOR: &str = "heroniteGpuFsr4Indicator";
pub const KEY_XESS_UPGRADE: &str = "heroniteGpuXessUpgrade";
/// Mesa's Anti-Lag layer (`VK_AMD_anti_lag` on any Mesa driver).
pub const KEY_ANTI_LAG: &str = "heroniteGpuAntiLag";
/// `mesa_glthread`: OpenGL work on a second thread (Mesa drivers).
pub const KEY_GLTHREAD: &str = "heroniteGpuGlthread";
/// [`Vsync::id`].
pub const KEY_VSYNC: &str = "heroniteGpuVsync";
/// [`Vendor::id`] DirectX games are told the card is from; empty: the real one.
pub const KEY_VENDOR: &str = "heroniteGpuVendor";
/// Video memory DirectX games are told the card has, in MiB; `0` or empty:
/// all of it.
pub const KEY_VRAM: &str = "heroniteGpuVram";
/// The keys new games inherit from the global defaults (besides Heroic's
/// `nvidiaPrime`).
pub const INHERITED_KEYS: [&str; 16] = [
    KEY_DEVICE,
    KEY_SMOOTH_MOTION,
    KEY_REFLEX,
    KEY_SHADER_CACHE,
    KEY_DLSS_UPGRADE,
    KEY_DLSS_PRESET,
    KEY_DLSS_INDICATOR,
    KEY_FSR4_UPGRADE,
    KEY_FSR4_INDICATOR,
    KEY_XESS_UPGRADE,
    KEY_ANTI_LAG,
    KEY_GLTHREAD,
    KEY_VSYNC,
    KEY_VENDOR,
    KEY_VRAM,
    KEY_PRIME,
];

/// Size of the bigger NVIDIA shader cache: 10 GiB (the driver's own is 1 GiB
/// and is trimmed, so big games compile their shaders again).
pub const SHADER_CACHE_BYTES: &str = "10737418240";
/// The video memory choices, in MiB.
pub const VRAM_CHOICES: [u32; 8] = [2048, 3072, 4096, 6144, 8192, 10240, 12288, 16384];

pub const NVIDIA: u16 = 0x10de;
pub const AMD: u16 = 0x1002;
pub const INTEL: u16 = 0x8086;

/// The DLSS model the game uses, through the driver's preset override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DlssPreset {
    /// The game's own.
    #[default]
    Game,
    /// The newest model of the DLSS file.
    Latest,
    /// The transformer model (DLSS 4).
    K,
    /// The last CNN model: lighter on the card.
    E,
}

impl DlssPreset {
    pub const ALL: [DlssPreset; 4] = [DlssPreset::Game, DlssPreset::Latest, DlssPreset::K, DlssPreset::E];

    pub fn id(self) -> &'static str {
        match self {
            DlssPreset::Game => "",
            DlssPreset::Latest => "latest",
            DlssPreset::K => "k",
            DlssPreset::E => "e",
        }
    }

    pub fn parse(s: &str) -> Self {
        Self::ALL.into_iter().find(|p| p.id().eq_ignore_ascii_case(s.trim())).unwrap_or_default()
    }

    /// The value of dxvk-nvapi's setting (`NvApiDriverSettings.h` names,
    /// read without case).
    fn drs(self) -> Option<&'static str> {
        match self {
            DlssPreset::Game => None,
            DlssPreset::Latest => Some("render_preset_latest"),
            DlssPreset::K => Some("render_preset_k"),
            DlssPreset::E => Some("render_preset_e"),
        }
    }
}

/// Whether frames wait for the screen's refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Vsync {
    /// The game's own setting.
    #[default]
    Game,
    On,
    Off,
}

impl Vsync {
    pub const ALL: [Vsync; 3] = [Vsync::Game, Vsync::On, Vsync::Off];

    pub fn id(self) -> &'static str {
        match self {
            Vsync::Game => "",
            Vsync::On => "on",
            Vsync::Off => "off",
        }
    }

    pub fn parse(s: &str) -> Self {
        Self::ALL.into_iter().find(|v| v.id() == s.trim()).unwrap_or_default()
    }
}

/// A card maker, as DirectX games see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
}

impl Vendor {
    pub const ALL: [Vendor; 3] = [Vendor::Nvidia, Vendor::Amd, Vendor::Intel];

    pub fn id(self) -> &'static str {
        match self {
            Vendor::Nvidia => "nvidia",
            Vendor::Amd => "amd",
            Vendor::Intel => "intel",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Vendor::Nvidia => "NVIDIA",
            Vendor::Amd => "AMD",
            Vendor::Intel => "Intel",
        }
    }

    pub fn pci_id(self) -> u16 {
        match self {
            Vendor::Nvidia => NVIDIA,
            Vendor::Amd => AMD,
            Vendor::Intel => INTEL,
        }
    }

    pub fn of_pci(id: u16) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.pci_id() == id)
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.id() == s.trim())
    }
}

/// The game's graphics card settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Gpu {
    /// PCI address; empty: the system's choice (or [`Gpu::dedicated`]).
    pub device: String,
    /// Heroic's `nvidiaPrime`, used when no card is picked.
    pub dedicated: bool,
    pub smooth_motion: bool,
    pub reflex: bool,
    pub shader_cache: bool,
    pub dlss_upgrade: bool,
    pub dlss_preset: DlssPreset,
    pub dlss_indicator: bool,
    pub fsr4_upgrade: bool,
    pub fsr4_indicator: bool,
    pub xess_upgrade: bool,
    pub anti_lag: bool,
    pub glthread: bool,
    pub vsync: Vsync,
    pub vendor: Option<Vendor>,
    /// MiB; `0`: all of it.
    pub vram: u32,
}

impl Gpu {
    pub fn from_map(m: &Map<String, Value>) -> Self {
        let flag = |k: &str| m.get(k) == Some(&Value::Bool(true));
        let text = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
        let vram = m.get(KEY_VRAM).and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))).unwrap_or(0);
        Self {
            device: text(KEY_DEVICE),
            dedicated: flag(KEY_PRIME),
            smooth_motion: flag(KEY_SMOOTH_MOTION),
            reflex: flag(KEY_REFLEX),
            shader_cache: flag(KEY_SHADER_CACHE),
            dlss_upgrade: flag(KEY_DLSS_UPGRADE),
            dlss_preset: DlssPreset::parse(&text(KEY_DLSS_PRESET)),
            dlss_indicator: flag(KEY_DLSS_INDICATOR),
            fsr4_upgrade: flag(KEY_FSR4_UPGRADE),
            fsr4_indicator: flag(KEY_FSR4_INDICATOR),
            xess_upgrade: flag(KEY_XESS_UPGRADE),
            anti_lag: flag(KEY_ANTI_LAG),
            glthread: flag(KEY_GLTHREAD),
            vsync: Vsync::parse(&text(KEY_VSYNC)),
            vendor: Vendor::parse(&text(KEY_VENDOR)),
            vram: u32::try_from(vram).unwrap_or(0),
        }
    }
}

/// Settings that are one variable set to one value: `(key, variable,
/// value, Windows games only)`.
const SWITCHES: [(&str, &str, &str, bool); 10] = [
    (KEY_SMOOTH_MOTION, "NVPRESENT_ENABLE_SMOOTH_MOTION", "1", false),
    (KEY_ANTI_LAG, "ENABLE_LAYER_MESA_ANTI_LAG", "1", false),
    (KEY_GLTHREAD, "mesa_glthread", "true", false),
    (KEY_REFLEX, "DXVK_NVAPI_VKREFLEX", "1", true),
    (KEY_DLSS_UPGRADE, "PROTON_DLSS_UPGRADE", "1", true),
    (KEY_DLSS_INDICATOR, "PROTON_DLSS_INDICATOR", "1", true),
    (KEY_FSR4_UPGRADE, "PROTON_FSR4_UPGRADE", "1", true),
    (KEY_FSR4_INDICATOR, "PROTON_FSR4_INDICATOR", "1", true),
    (KEY_XESS_UPGRADE, "PROTON_XESS_UPGRADE", "1", true),
    // Read back only through the cache size (both are written together).
    (KEY_SHADER_CACHE, "__GL_SHADER_DISK_CACHE_SKIP_CLEANUP", "1", false),
];
const SHADER_CACHE_SIZE: &str = "__GL_SHADER_DISK_CACHE_SIZE";
/// dxvk-nvapi's dedicated variable for the DLSS preset (wins over the
/// `DXVK_NVAPI_DRS_SETTINGS` GE-Proton sets with the upgrade).
const DLSS_PRESET: &str = "DXVK_NVAPI_DRS_NGX_DLSS_SR_OVERRIDE_RENDER_PRESET_SELECTION";
pub const DXVK_CONFIG: &str = "DXVK_CONFIG";
/// What "use the dedicated GPU" sets (Heroic's `setupEnvVars` with `nvidiaPrime`).
pub const PRIME: [(&str, &str); 3] = [("DRI_PRIME", "1"), ("__NV_PRIME_RENDER_OFFLOAD", "1"), ("__GLX_VENDOR_LIBRARY_NAME", "nvidia")];
/// The variables that pick a card, besides `DRI_PRIME`.
const DEVICE_VARS: [&str; 4] = ["MESA_VK_DEVICE_SELECT", "__NV_PRIME_RENDER_OFFLOAD", "__VK_LAYER_NV_optimus", "__GLX_VENDOR_LIBRARY_NAME"];
/// Vsync for OpenGL (Mesa, NVIDIA) and Mesa's Vulkan drivers: `(variable,
/// on, off)`. NVIDIA's Vulkan driver has no such switch.
const VSYNC_VARS: [(&str, &str, &str); 3] =
    [("vblank_mode", "3", "0"), ("__GL_SYNC_TO_VBLANK", "1", "0"), ("MESA_VK_WSI_PRESENT_MODE", "fifo", "immediate")];

impl Gpu {
    /// The settings that apply on a card of `vendor` (PCI id; `None`:
    /// unknown, everything applies): the other makers' options are off, as
    /// the GPU tab hides them.
    pub fn for_card(&self, vendor: Option<u16>) -> Gpu {
        let mut g = self.clone();
        let Some(vendor) = vendor else { return g };
        if vendor != NVIDIA {
            (g.smooth_motion, g.reflex, g.shader_cache) = (false, false, false);
            (g.dlss_upgrade, g.dlss_indicator, g.dlss_preset) = (false, false, DlssPreset::Game);
        }
        if vendor == NVIDIA {
            (g.anti_lag, g.glthread) = (false, false);
        }
        if vendor != AMD {
            (g.fsr4_upgrade, g.fsr4_indicator) = (false, false);
        }
        g
    }
}

/// The maker (PCI id) of the card that draws the game: the picked one,
/// NVIDIA for Heroic's "dedicated GPU", else the card the screens start on.
/// `None`: not known.
pub fn target_vendor(g: &Gpu, probe: &dyn Probe) -> Option<u16> {
    if !g.device.is_empty() {
        if let Some(c) = detect::card_at(&g.device, probe) {
            return Some(c.vendor_id);
        }
    } else if g.dedicated {
        return Some(NVIDIA);
    }
    detect::primary_card(probe).map(|c| c.vendor_id)
}

/// [`Gpu::for_card`] for the card that draws the game on this computer.
pub fn for_launch(g: &Gpu, probe: &dyn Probe) -> Gpu {
    g.for_card(target_vendor(g, probe))
}

/// The variables for any game run here, Linux or Windows: the card, the
/// driver switches, vsync of OpenGL and Mesa's Vulkan.
pub fn env(g: &Gpu, probe: &dyn Probe) -> Env {
    let mut env = device_env(g, probe);
    let flags = Gpu::switches(g);
    for (key, var, value, wine_only) in SWITCHES {
        if !wine_only && flags.contains(&key) {
            env.insert(var.into(), value.into());
        }
    }
    if g.shader_cache {
        env.insert(SHADER_CACHE_SIZE.into(), SHADER_CACHE_BYTES.into());
    }
    for (var, on, off) in VSYNC_VARS {
        match g.vsync {
            Vsync::On => env.insert(var.into(), on.into()),
            Vsync::Off => env.insert(var.into(), off.into()),
            Vsync::Game => None,
        };
    }
    env
}

/// The variables for Windows games. `proton`: run through Proton (the
/// upgrades and Reflex are GE-Proton's and Proton-CachyOS's).
/// `user_dxvk_config`: the game's own `DXVK_CONFIG`, put after ours so it
/// wins.
pub fn wine_env(g: &Gpu, proton: bool, user_dxvk_config: Option<&str>) -> Env {
    let mut env = Env::new();
    if proton {
        let flags = Gpu::switches(g);
        for (key, var, value, wine_only) in SWITCHES {
            if wine_only && flags.contains(&key) {
                env.insert(var.into(), value.into());
            }
        }
        if let Some(preset) = g.dlss_preset.drs().filter(|_| g.dlss_upgrade) {
            env.insert(DLSS_PRESET.into(), preset.into());
        }
    }
    let ours = dxvk_config(g);
    let user = user_dxvk_config.map(str::trim).filter(|u| !u.is_empty());
    if !ours.is_empty() {
        let mut items = ours;
        items.extend(user.map(str::to_string));
        env.insert(DXVK_CONFIG.into(), items.join(";"));
    }
    env
}

impl Gpu {
    /// The keys of [`SWITCHES`] that are on.
    fn switches(&self) -> Vec<&'static str> {
        [
            (KEY_SMOOTH_MOTION, self.smooth_motion),
            (KEY_ANTI_LAG, self.anti_lag),
            (KEY_GLTHREAD, self.glthread),
            (KEY_REFLEX, self.reflex),
            (KEY_DLSS_UPGRADE, self.dlss_upgrade),
            (KEY_DLSS_INDICATOR, self.dlss_indicator && self.dlss_upgrade),
            (KEY_FSR4_UPGRADE, self.fsr4_upgrade),
            (KEY_FSR4_INDICATOR, self.fsr4_indicator && self.fsr4_upgrade),
            (KEY_XESS_UPGRADE, self.xess_upgrade),
            (KEY_SHADER_CACHE, self.shader_cache),
        ]
        .into_iter()
        .filter_map(|(k, on)| on.then_some(k))
        .collect()
    }
}

/// DXVK's options (`DXVK_CONFIG` items) for vsync, the maker and the memory.
fn dxvk_config(g: &Gpu) -> Vec<String> {
    let mut out = Vec::new();
    match g.vsync {
        Vsync::On => out.extend(["dxgi.syncInterval=1".to_string(), "d3d9.presentInterval=1".to_string()]),
        Vsync::Off => out.extend(["dxgi.syncInterval=0".to_string(), "d3d9.presentInterval=0".to_string()]),
        Vsync::Game => {}
    }
    if let Some(v) = g.vendor {
        out.push(format!("dxgi.customVendorId={:04x}", v.pci_id()));
    }
    if g.vram > 0 {
        out.push(format!("dxgi.maxDeviceMemory={}", g.vram));
    }
    out
}

/// The variables that make the game draw on the picked card. A card that
/// is gone adds nothing: the game gets the system's choice.
fn device_env(g: &Gpu, probe: &dyn Probe) -> Env {
    let mut env = Env::new();
    if g.device.is_empty() {
        if g.dedicated {
            env.extend(PRIME.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        }
        return env;
    }
    let Some(card) = detect::card_at(&g.device, probe) else {
        log::warn!("the graphics card {} picked for this game was not found; using the system's choice", g.device);
        return env;
    };
    // Mesa's OpenGL and its Vulkan device-select layer (which orders every
    // driver's cards, NVIDIA's too) put this card first.
    env.insert("DRI_PRIME".into(), dri_prime_tag(&g.device));
    env.insert("MESA_VK_DEVICE_SELECT".into(), format!("{:04x}:{:04x}", card.vendor_id, card.device_id));
    if card.vendor_id == NVIDIA {
        // NVIDIA's render offload, only for a card that does not drive the
        // screens (the offload variables are for hybrid setups).
        if !card.primary {
            env.insert("__NV_PRIME_RENDER_OFFLOAD".into(), "1".into());
            env.insert("__VK_LAYER_NV_optimus".into(), "NVIDIA_only".into());
            env.insert("__GLX_VENDOR_LIBRARY_NAME".into(), "nvidia".into());
        }
    } else {
        env.insert("__GLX_VENDOR_LIBRARY_NAME".into(), "mesa".into());
    }
    env
}

/// Mesa's `DRI_PRIME` tag of a PCI address: `0000:0d:00.0` -> `pci-0000_0d_00_0`.
fn dri_prime_tag(slot: &str) -> String {
    format!("pci-{}", slot.replace([':', '.'], "_"))
}

/// The PCI address of a `DRI_PRIME` tag (`None`: not a `pci-` tag).
fn slot_of_tag(tag: &str) -> Option<String> {
    let parts: Vec<&str> = tag.strip_prefix("pci-")?.split('_').collect();
    match parts.as_slice() {
        [domain, bus, dev, func] => Some(format!("{domain}:{bus}:{dev}.{func}")),
        _ => None,
    }
}

/// Every variable of the settings for a Steam game's launch options (Steam
/// runs Windows games through Proton). `user_dxvk_config`: the game's own
/// `DXVK_CONFIG`, merged into ours.
/// Heroic's PRIME variables come first, in its order; the rest by name.
pub fn launch_options_vars(g: &Gpu, probe: &dyn Probe, user_dxvk_config: Option<&str>) -> Vec<(String, String)> {
    let g = &for_launch(g, probe);
    let mut env = env(g, probe);
    env.extend(wine_env(g, true, user_dxvk_config));
    let mut out: Vec<(String, String)> = PRIME.iter().filter_map(|(k, _)| env.remove(*k).map(|v| (k.to_string(), v))).collect();
    out.extend(env);
    out
}

/// Takes the variables [`launch_options_vars`] writes out of a launch
/// line's `env` into settings keys of `s`. What does not match stays a
/// plain variable (a `DXVK_CONFIG` keeps the items that are not ours).
pub fn read_launch_options(env: &mut Vec<(String, String)>, s: &mut Map<String, Value>) {
    let get = |env: &Vec<(String, String)>, k: &str| env.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
    let take = |env: &mut Vec<(String, String)>, k: &str| env.retain(|(key, _)| key != k);
    let flag = |s: &Map<String, Value>, k: &str| s.get(k) == Some(&Value::Bool(true));
    for (key, var, value, _) in SWITCHES {
        let on = get(env, var).is_some_and(|v| v.eq_ignore_ascii_case(value))
            && match key {
                KEY_SHADER_CACHE => get(env, SHADER_CACHE_SIZE).as_deref() == Some(SHADER_CACHE_BYTES),
                // An indicator only counts with its upgrade (read before it).
                KEY_DLSS_INDICATOR => flag(s, KEY_DLSS_UPGRADE),
                KEY_FSR4_INDICATOR => flag(s, KEY_FSR4_UPGRADE),
                _ => true,
            };
        if on {
            take(env, var);
            if key == KEY_SHADER_CACHE {
                take(env, SHADER_CACHE_SIZE);
            }
        }
        s.insert(key.into(), Value::Bool(on));
    }
    let preset = get(env, DLSS_PRESET)
        .and_then(|v| DlssPreset::ALL.into_iter().find(|p| p.drs().is_some_and(|d| d.eq_ignore_ascii_case(v.trim()))))
        .filter(|_| flag(s, KEY_DLSS_UPGRADE));
    if preset.is_some() {
        take(env, DLSS_PRESET);
    }
    s.insert(KEY_DLSS_PRESET.into(), json!(preset.unwrap_or_default().id()));

    let mut vsync = Vsync::Game;
    let mut vendor: Option<Vendor> = None;
    let mut vram = 0u32;
    if let Some(config) = get(env, DXVK_CONFIG) {
        let mut rest = Vec::new();
        let mut sync = (None, None);
        for item in config.split(';').map(str::trim).filter(|i| !i.is_empty()) {
            let (k, v) = item.split_once('=').map(|(k, v)| (k.trim(), v.trim())).unwrap_or((item, ""));
            match k {
                "dxgi.syncInterval" if sync.0.is_none() && matches!(v, "0" | "1") => sync.0 = Some(v == "1"),
                "d3d9.presentInterval" if sync.1.is_none() && matches!(v, "0" | "1") => sync.1 = Some(v == "1"),
                "dxgi.customVendorId" if vendor.is_none() => match u16::from_str_radix(v, 16).ok().and_then(Vendor::of_pci) {
                    Some(found) => vendor = Some(found),
                    None => rest.push(item.to_string()),
                },
                "dxgi.maxDeviceMemory" if vram == 0 => match v.parse::<u32>() {
                    Ok(n) if n > 0 => vram = n,
                    _ => rest.push(item.to_string()),
                },
                _ => rest.push(item.to_string()),
            }
        }
        match sync {
            (Some(a), Some(b)) if a == b => vsync = if a { Vsync::On } else { Vsync::Off },
            (a, b) => {
                // Half of the pair is the user's own option.
                rest.extend(a.map(|on| format!("dxgi.syncInterval={}", u8::from(on))));
                rest.extend(b.map(|on| format!("d3d9.presentInterval={}", u8::from(on))));
            }
        }
        take(env, DXVK_CONFIG);
        if !rest.is_empty() {
            env.push((DXVK_CONFIG.into(), rest.join(";")));
        }
    }
    // The OpenGL / Mesa vsync variables written with DXVK's.
    if vsync != Vsync::Game {
        for (var, on, off) in VSYNC_VARS {
            let want = if vsync == Vsync::On { on } else { off };
            if get(env, var).as_deref() == Some(want) {
                take(env, var);
            }
        }
    }
    s.insert(KEY_VSYNC.into(), json!(vsync.id()));
    s.insert(KEY_VENDOR.into(), json!(vendor.map(Vendor::id).unwrap_or("")));
    s.insert(KEY_VRAM.into(), json!(vram));

    let device = get(env, "DRI_PRIME").and_then(|t| slot_of_tag(&t));
    if let Some(slot) = &device {
        take(env, "DRI_PRIME");
        for var in DEVICE_VARS {
            take(env, var);
        }
        s.insert(KEY_DEVICE.into(), json!(slot));
    } else {
        s.insert(KEY_DEVICE.into(), json!(""));
    }
}

/// The setting key a variable of [`env`] or [`wine_env`] comes from
/// (`DXVK_CONFIG` holds vsync, the maker and the memory: [`KEY_VSYNC`]).
pub fn var_setting(var: &str) -> Option<&'static str> {
    if let Some((key, ..)) = SWITCHES.iter().find(|(_, v, ..)| *v == var) {
        return Some(key);
    }
    match var {
        SHADER_CACHE_SIZE => Some(KEY_SHADER_CACHE),
        DLSS_PRESET => Some(KEY_DLSS_PRESET),
        DXVK_CONFIG => Some(KEY_VSYNC),
        "DRI_PRIME" | "MESA_VK_DEVICE_SELECT" | "__NV_PRIME_RENDER_OFFLOAD" | "__VK_LAYER_NV_optimus" | "__GLX_VENDOR_LIBRARY_NAME" => {
            Some(KEY_DEVICE)
        }
        _ => VSYNC_VARS.iter().any(|(v, ..)| *v == var).then_some(KEY_VSYNC),
    }
}

/// The settings of a launch line without any of these variables.
pub fn launch_options_defaults(s: &mut Map<String, Value>) {
    read_launch_options(&mut Vec::new(), s);
}

#[cfg(test)]
mod tests;
