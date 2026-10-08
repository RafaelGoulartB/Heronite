//! Save states per emulator engine (port of `emulation/save-states.ts`,
//! `save-state-engines.ts`): where the files live, how a ROM maps to the file
//! names, thumbnails as raw image bytes, and the one-shot launch arguments
//! that boot into a chosen state.
//!
//! Wiring into launching (the launch crate builds plans, this crate only
//! produces data): take the [`LaunchStateArgs`] returned by
//! [`PendingLaunchStates::take`] and call [`LaunchStateArgs::apply`] on the
//! emulator argv (before the Flatpak `run <id>` wrapper), skipping Wine
//! emulators, exactly where `tryLaunchEmulatedGame` does.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;
use chrono::{DateTime, SecondsFormat, Utc};
use heronite_data::models::LocalMetaFile;
use heronite_data::{json_store, HeroicPaths};
use heronite_launch::emulators::{EmulatorsFile, UserEmulator};
use heronite_launch::plan::pick_rom_path;
use heronite_launch::probe::SystemProbe;
use regex::Regex;
use serde::Serialize;

use crate::catalog::{emulator_definition, guess_definition_id};
use crate::disc_id::{
    read_nintendo_disc_id, read_playstation_serial, read_ps3_title_id, read_psp_disc_id,
    serial_from_game_list_cache,
};
use crate::pathutil::{base_no_ext, basename, dirname, join};
use crate::store::resolve_template;
use crate::sysfiles::{
    find_embedded_png, image_mime, read_zip_entry, resolve_configured_dir, HostEnv, Ini,
};

pub const MAX_STATES: usize = 24;
pub const PENDING_TTL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SaveStateKind {
    Slot,
    Resume,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedState {
    pub key: String,
    pub kind: SaveStateKind,
    /// Slot number as the emulator's own menu shows it.
    pub slot: Option<u32>,
}

/// Raw screenshot of a state (PNG or JPEG bytes); decode it in the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumbnail {
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmulatorSaveState {
    pub path: String,
    pub kind: SaveStateKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot: Option<u32>,
    /// ISO 8601 UTC, like `Date#toISOString`.
    pub modified_at: String,
    #[serde(skip)]
    pub thumbnail: Option<Arc<Thumbnail>>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmulatorSaveStateList {
    pub supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emulator_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rom_path: Option<String>,
    pub states: Vec<EmulatorSaveState>,
}

/// Extra emulator arguments that boot into a state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchStateArgs {
    pub args: Vec<String>,
    /// The state boots the game by itself; drop the ROM path from argv.
    pub replaces_image: bool,
}

impl LaunchStateArgs {
    /// Prepends the state arguments to the emulator argv (dropping the ROM
    /// path when the state replaces the image).
    pub fn apply(&self, argv: Vec<String>, rom_path: Option<&str>) -> Vec<String> {
        let rest: Vec<String> = if self.replaces_image {
            argv.into_iter().filter(|p| Some(p.as_str()) != rom_path).collect()
        } else {
            argv
        };
        self.args.iter().cloned().chain(rest).collect()
    }
}

#[derive(Debug, Clone, Default)]
pub struct EngineContext {
    /// Emulator binary on the host; `None` for Flatpak installs.
    pub executable: Option<String>,
    pub flatpak_id: Option<String>,
    /// RetroArch core of the game's profile.
    pub core_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Pcsx2,
    DuckStation,
    Ppsspp,
    Dolphin,
    RetroArch,
    Mgba,
    Rpcs3,
}

/// Emulators without a CLI way to load a state at boot (melonDS, Azahar,
/// Cemu, Ryujinx, Vita3K, shadPS4, xemu) have no engine.
pub fn engine_for(id: &str) -> Option<Engine> {
    Some(match id {
        "pcsx2" => Engine::Pcsx2,
        "duckstation" => Engine::DuckStation,
        "ppsspp" => Engine::Ppsspp,
        "dolphin" => Engine::Dolphin,
        "retroarch" => Engine::RetroArch,
        "mgba" => Engine::Mgba,
        "rpcs3" => Engine::Rpcs3,
        _ => return None,
    })
}

/// `engineIdFor`: custom emulators are matched by name/binary/flatpak id.
pub fn engine_id_for(emulator: &UserEmulator) -> String {
    if emulator.definition_id != "custom" {
        return emulator.definition_id.clone();
    }
    let hints = [
        Some(emulator.name.clone()),
        Some(emulator.executable.clone()).filter(|e| !e.is_empty()).map(|e| basename(&e)),
        emulator.flatpak_id.clone(),
    ];
    for hint in hints {
        let id = guess_definition_id(hint.as_deref());
        if id != "custom" {
            return id;
        }
    }
    "custom".into()
}

fn unique(values: impl IntoIterator<Item = Option<String>>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for v in values.into_iter().flatten() {
        if !v.is_empty() && !out.contains(&v) {
            out.push(v);
        }
    }
    out
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn portable_dir(ctx: &EngineContext, markers: &[&str]) -> Option<PathBuf> {
    let exe = ctx.executable.as_deref()?;
    let dir = dirname(exe);
    markers
        .iter()
        .any(|m| Path::new(&dir).join(m).exists())
        .then(|| PathBuf::from(dir))
}

fn child_dirs(root: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(root)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|e| s(&e.path()))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).unwrap())
}

fn slot(key: &str, n: u32) -> ParsedState {
    ParsedState { key: key.into(), kind: SaveStateKind::Slot, slot: Some(n) }
}

fn resume(key: &str) -> ParsedState {
    ParsedState { key: key.into(), kind: SaveStateKind::Resume, slot: None }
}

// --- PCSX2 -------------------------------------------------------------------
// `{Serial} ({CRC}).{NN}.p2s`, `{Serial} ({CRC}).resume.p2s` in
// inis/PCSX2.ini [Folders] Savestates. `-statefile <file>` boots and loads.

fn pcsx2_data_dirs(env: &HostEnv, ctx: &EngineContext) -> Vec<PathBuf> {
    if let Some(id) = &ctx.flatpak_id {
        return vec![env.flatpak_dir(id, "config", &["PCSX2"])];
    }
    match portable_dir(ctx, &["portable.ini", "portable.txt"]) {
        Some(p) => vec![p],
        None => vec![env.xdg_config_home().join("PCSX2")],
    }
}

// --- DuckStation -------------------------------------------------------------
// `{Serial}_{N}.sav`, `{Serial}_resume.sav` in settings.ini [Folders]
// SaveStates. Data root: $XDG_CONFIG_HOME/duckstation when that variable is
// set, otherwise ~/.local/share/duckstation. `-statefile <file>` loads.

fn duckstation_data_dirs(env: &HostEnv, ctx: &EngineContext) -> Vec<PathBuf> {
    if let Some(id) = &ctx.flatpak_id {
        return vec![env.flatpak_dir(id, "data", &["duckstation"])];
    }
    if let Some(p) = portable_dir(ctx, &["portable.txt"]) {
        return vec![p];
    }
    unique([
        env.xdg_config_home_var.as_ref().map(|d| s(&d.join("duckstation"))),
        Some(s(&env.xdg_data_home().join("duckstation"))),
        Some(s(&env.home.join(".local/share/duckstation"))),
    ])
    .into_iter()
    .map(PathBuf::from)
    .filter(|d| d.exists())
    .collect()
}

// --- PPSSPP ------------------------------------------------------------------
// `{DISC_ID}_{DISC_VERSION}_{slot}.ppst` (slot 0-based) with a `.jpg` next to
// it, in <memstick>/PSP/PPSSPP_STATE. `--state=<file>` loads after boot.

fn ppsspp_state_dir(env: &HostEnv, ctx: &EngineContext) -> PathBuf {
    let memstick = match &ctx.flatpak_id {
        Some(id) => env.flatpak_dir(id, "config", &["ppsspp"]),
        None => env.xdg_config_home().join("ppsspp"),
    };
    memstick.join("PSP").join("PPSSPP_STATE")
}

// --- Dolphin -----------------------------------------------------------------
// `{GameID}.s{NN}` in <user>/StateSaves. `--save_state <file>` loads at boot.

fn dolphin_user_dir(env: &HostEnv, ctx: &EngineContext) -> PathBuf {
    if let Some(id) = &ctx.flatpak_id {
        return env.flatpak_dir(id, "data", &["dolphin-emu"]);
    }
    if let Some(p) = &env.dolphin_userpath {
        return p.clone();
    }
    let legacy = env.home.join(".dolphin-emu");
    if legacy.exists() {
        legacy
    } else {
        env.xdg_data_home().join("dolphin-emu")
    }
}

// --- RetroArch ---------------------------------------------------------------
// `{rom}.state` (slot 0), `{rom}.state{N}`, `{rom}.state.auto` in
// savestate_directory, sorted into a folder per core by default.
// `-e N` loads slot N at boot (normal slot files need RetroArch 1.21+); the
// auto state needs `savestate_auto_load`, passed via --appendconfig.

fn retroarch_config_dir(env: &HostEnv, ctx: &EngineContext) -> PathBuf {
    match &ctx.flatpak_id {
        Some(id) => env.flatpak_dir(id, "config", &["retroarch"]),
        None => env.xdg_config_home().join("retroarch"),
    }
}

fn retroarch_core_name(env: &HostEnv, ctx: &EngineContext) -> Option<String> {
    let core = ctx.core_path.as_deref()?;
    let info_name = format!("{}.info", base_no_ext(core));
    let config_dir = retroarch_config_dir(env, ctx);
    let cfg_dir = s(&config_dir);
    let cfg = Ini::read(&config_dir.join("retroarch.cfg"));
    let dirs = unique([
        resolve_configured_dir(
            cfg.as_ref().and_then(|c| c.get("", "libretro_info_path")),
            &cfg_dir,
            &env.home,
        ),
        Some(join(&cfg_dir, "cores")),
        Some(join(&cfg_dir, "info")),
        Some(dirname(core)),
        Some("/usr/share/libretro/info".into()),
        Some("/usr/lib/libretro".into()),
    ]);
    for dir in dirs {
        if let Some(info) = Ini::read(&Path::new(&dir).join(&info_name)) {
            if let Some(name) = info.get("", "corename") {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Flatpak RetroArch cannot read the host /tmp; use the app's cache folder.
fn retroarch_append_config(env: &HostEnv, ctx: &EngineContext, auto_load: bool) -> Result<String> {
    let dir = match &ctx.flatpak_id {
        Some(id) => env.home.join(".var/app").join(id).join("cache"),
        None => env.tmp_dir.clone(),
    };
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(format!("heronite-retroarch-autoload-{auto_load}.cfg"));
    std::fs::write(&file, format!("savestate_auto_load = \"{auto_load}\"\n"))?;
    Ok(s(&file))
}

// --- mGBA --------------------------------------------------------------------
// `{rom}.ss{N}` next to the ROM or in [ports.qt] savestatePath. ss0 is the
// auto slot that mGBA-Qt loads on boot by default (`autoload`). States are PNG
// files with the emulator data embedded. `-t <file>` loads at boot.

fn mgba_config_path(env: &HostEnv, ctx: &EngineContext) -> PathBuf {
    match &ctx.flatpak_id {
        Some(id) => env.flatpak_dir(id, "config", &["mgba", "config.ini"]),
        None => env.xdg_config_home().join("mgba").join("config.ini"),
    }
}

// --- RPCS3 -------------------------------------------------------------------
// `<config>/savestates/{TITLE_ID}/{TITLE_ID}_*.SAVESTAT[.zst|.gz]`.
// `--savestate <file>` boots the state instead of the game.

fn rpcs3_states_root(env: &HostEnv, ctx: &EngineContext) -> PathBuf {
    match &ctx.flatpak_id {
        Some(id) => env.flatpak_dir(id, "config", &["rpcs3", "savestates"]),
        None => env.xdg_config_home().join("rpcs3").join("savestates"),
    }
}

impl Engine {
    /// Serial-keyed engines learn the key from states written in a session.
    pub fn learns_keys(self) -> bool {
        !matches!(self, Engine::RetroArch | Engine::Mgba)
    }

    /// Directories that hold state files for these keys / this ROM.
    pub fn state_dirs(self, env: &HostEnv, ctx: &EngineContext, rom: &str, keys: &[String]) -> Vec<String> {
        match self {
            Engine::Pcsx2 => pcsx2_data_dirs(env, ctx)
                .into_iter()
                .map(|data| {
                    let ini = Ini::read(&data.join("inis").join("PCSX2.ini"));
                    resolve_configured_dir(
                        ini.as_ref().and_then(|i| i.get("Folders", "Savestates")),
                        &s(&data),
                        &env.home,
                    )
                    .unwrap_or_else(|| s(&data.join("sstates")))
                })
                .collect(),
            Engine::DuckStation => duckstation_data_dirs(env, ctx)
                .into_iter()
                .map(|data| {
                    let ini = Ini::read(&data.join("settings.ini"));
                    resolve_configured_dir(
                        ini.as_ref().and_then(|i| i.get("Folders", "SaveStates")),
                        &s(&data),
                        &env.home,
                    )
                    .unwrap_or_else(|| s(&data.join("savestates")))
                })
                .collect(),
            Engine::Ppsspp => vec![s(&ppsspp_state_dir(env, ctx))],
            Engine::Dolphin => vec![s(&dolphin_user_dir(env, ctx).join("StateSaves"))],
            Engine::RetroArch => {
                let config_dir = retroarch_config_dir(env, ctx);
                let cfg_dir = s(&config_dir);
                let cfg = Ini::read(&config_dir.join("retroarch.cfg"));
                let configured = cfg
                    .as_ref()
                    .and_then(|c| c.get("", "savestate_directory"))
                    .map(|v| match v.strip_prefix(':') {
                        Some(rest) => format!("{cfg_dir}{rest}"),
                        None => v.to_string(),
                    });
                let root = resolve_configured_dir(configured.as_deref(), &cfg_dir, &env.home)
                    .unwrap_or_else(|| join(&cfg_dir, "states"));
                if let Some(core) = retroarch_core_name(env, ctx) {
                    if Path::new(&root).join(&core).exists() {
                        return vec![root.clone(), join(&root, &core)];
                    }
                }
                let mut out = vec![root.clone()];
                out.extend(child_dirs(Path::new(&root)));
                out
            }
            Engine::Mgba => {
                let ini = Ini::read(&mgba_config_path(env, ctx));
                let configured = resolve_configured_dir(
                    ini.as_ref().and_then(|i| i.get("ports.qt", "savestatePath")),
                    &dirname(rom),
                    &env.home,
                );
                unique([configured, Some(dirname(rom))])
            }
            Engine::Rpcs3 => {
                let root = rpcs3_states_root(env, ctx);
                if keys.is_empty() {
                    child_dirs(&root)
                } else {
                    keys.iter().map(|k| s(&root.join(k))).collect()
                }
            }
        }
    }

    /// Keys the emulator uses in state file names for this ROM.
    pub fn rom_keys(self, env: &HostEnv, ctx: &EngineContext, rom: &str) -> Vec<String> {
        let rom_path = Path::new(rom);
        match self {
            Engine::Pcsx2 => {
                let mut keys: Vec<Option<String>> = pcsx2_data_dirs(env, ctx)
                    .iter()
                    .map(|d| serial_from_game_list_cache(&d.join("cache").join("gamelist.cache"), rom))
                    .collect();
                keys.push(read_playstation_serial(rom_path));
                unique(keys)
            }
            Engine::DuckStation => {
                let mut keys: Vec<Option<String>> = duckstation_data_dirs(env, ctx)
                    .iter()
                    .map(|d| serial_from_game_list_cache(&d.join("cache").join("gamelist.cache"), rom))
                    .collect();
                keys.push(read_playstation_serial(rom_path));
                unique(keys)
            }
            Engine::Ppsspp => unique([read_psp_disc_id(rom_path)]),
            Engine::Dolphin => unique([read_nintendo_disc_id(rom_path)]),
            Engine::RetroArch | Engine::Mgba => vec![base_no_ext(rom)],
            Engine::Rpcs3 => unique([read_ps3_title_id(rom_path)]),
        }
    }

    /// Parses a file name in a state folder (`None` = not a state file).
    pub fn parse(self, file_name: &str) -> Option<ParsedState> {
        static PCSX2: OnceLock<Regex> = OnceLock::new();
        static DUCK: OnceLock<Regex> = OnceLock::new();
        static PSP: OnceLock<Regex> = OnceLock::new();
        static DOLPHIN: OnceLock<Regex> = OnceLock::new();
        static RA: OnceLock<Regex> = OnceLock::new();
        static MGBA: OnceLock<Regex> = OnceLock::new();
        static RPCS3: OnceLock<Regex> = OnceLock::new();
        match self {
            Engine::Pcsx2 => {
                let c = re(&PCSX2, r"(?i)^(.+?) \([0-9a-f]{8}\)\.(\d{2}|resume)\.p2s$").captures(file_name)?;
                Some(if c[2].eq_ignore_ascii_case("resume") {
                    resume(&c[1])
                } else {
                    slot(&c[1], c[2].parse().ok()?)
                })
            }
            Engine::DuckStation => {
                let c = re(&DUCK, r"(?i)^(.+)_(\d+|resume)\.sav$").captures(file_name)?;
                if &c[1] == "savestate" {
                    return None;
                }
                Some(if c[2].eq_ignore_ascii_case("resume") {
                    resume(&c[1])
                } else {
                    slot(&c[1], c[2].parse().ok()?)
                })
            }
            Engine::Ppsspp => {
                let c = re(&PSP, r"(?i)^([A-Z0-9]+)_[\d.]+_(\d+)\.ppst$").captures(file_name)?;
                Some(slot(&c[1], c[2].parse::<u32>().ok()? + 1))
            }
            Engine::Dolphin => {
                let c = re(&DOLPHIN, r"^([A-Z0-9]{6})\.s(\d{2})$").captures(file_name)?;
                Some(slot(&c[1], c[2].parse().ok()?))
            }
            Engine::RetroArch => {
                let c = re(&RA, r"^(.+)\.state(\d*|\.auto)$").captures(file_name)?;
                if &c[2] == ".auto" {
                    return Some(resume(&c[1]));
                }
                Some(slot(&c[1], if c[2].is_empty() { 0 } else { c[2].parse().ok()? }))
            }
            Engine::Mgba => {
                let c = re(&MGBA, r"^(.+)\.ss(\d)$").captures(file_name)?;
                Some(if &c[2] == "0" { resume(&c[1]) } else { slot(&c[1], c[2].parse().ok()?) })
            }
            Engine::Rpcs3 => {
                let c = re(&RPCS3, r"(?i)^([A-Z]{4}\d{5})_.*\.SAVESTAT(\.zst|\.gz)?$").captures(file_name)?;
                Some(ParsedState { key: c[1].to_string(), kind: SaveStateKind::Slot, slot: None })
            }
        }
    }

    /// Arguments that boot into this state.
    pub fn load_args(
        self,
        env: &HostEnv,
        ctx: &EngineContext,
        path: &str,
        kind: SaveStateKind,
        slot: Option<u32>,
    ) -> Result<LaunchStateArgs> {
        Ok(match self {
            Engine::Pcsx2 | Engine::DuckStation => LaunchStateArgs {
                args: vec!["-statefile".into(), path.into()],
                replaces_image: false,
            },
            Engine::Ppsspp => LaunchStateArgs { args: vec![format!("--state={path}")], replaces_image: false },
            Engine::Dolphin => LaunchStateArgs {
                args: vec!["--save_state".into(), path.into()],
                replaces_image: false,
            },
            Engine::RetroArch => {
                if kind == SaveStateKind::Resume {
                    let cfg = retroarch_append_config(env, ctx, true)?;
                    LaunchStateArgs { args: vec![format!("--appendconfig={cfg}")], replaces_image: false }
                } else {
                    LaunchStateArgs {
                        args: vec!["-e".into(), slot.unwrap_or(0).to_string()],
                        replaces_image: false,
                    }
                }
            }
            Engine::Mgba => LaunchStateArgs { args: vec!["-t".into(), path.into()], replaces_image: false },
            Engine::Rpcs3 => LaunchStateArgs {
                args: vec!["--savestate".into(), path.into()],
                replaces_image: true,
            },
        })
    }

    /// Arguments that stop the emulator from auto-loading a state.
    pub fn fresh_args(self, env: &HostEnv, ctx: &EngineContext) -> Result<Vec<String>> {
        Ok(match self {
            Engine::RetroArch => {
                let cfg = Ini::read(&retroarch_config_dir(env, ctx).join("retroarch.cfg"));
                if cfg.as_ref().and_then(|c| c.get("", "savestate_auto_load")) != Some("true") {
                    return Ok(vec![]);
                }
                vec![format!("--appendconfig={}", retroarch_append_config(env, ctx, false)?)]
            }
            Engine::Mgba => vec!["-C".into(), "autoload=0".into()],
            _ => vec![],
        })
    }

    /// The screenshot stored with the state, if the emulator keeps one.
    pub fn thumbnail(self, state_path: &str) -> Option<Vec<u8>> {
        let p = Path::new(state_path);
        match self {
            Engine::Pcsx2 => read_zip_entry(p, "Screenshot.png"),
            Engine::DuckStation | Engine::Mgba => find_embedded_png(p),
            Engine::Ppsspp => {
                let jpg = Regex::new(r"(?i)\.ppst$").unwrap().replace(state_path, ".jpg").into_owned();
                std::fs::read(jpg).ok()
            }
            Engine::RetroArch => std::fs::read(format!("{state_path}.png")).ok(),
            Engine::Dolphin | Engine::Rpcs3 => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Resolving a game and listing states
// ---------------------------------------------------------------------------

struct Resolved {
    meta: heronite_data::models::LocalGameMeta,
    engine: Engine,
    ctx: EngineContext,
    rom_path: String,
    emulator_name: String,
}

fn resolve(paths: &HeroicPaths, app_name: &str, rom_path: Option<&str>) -> Result<Option<Resolved>> {
    let metas: LocalMetaFile = json_store::read(&paths.local_meta())?;
    let Some(meta) = metas.games.get(app_name).cloned() else {
        return Ok(None);
    };
    if meta.launch_kind != "emulator" {
        return Ok(None);
    }
    let Some(emulator_id) = meta.emulator_id.clone().filter(|i| !i.is_empty()) else {
        return Ok(None);
    };
    let emulators: EmulatorsFile = json_store::read(&paths.emulators())?;
    let Some(emulator) = emulators.get(&emulator_id).cloned() else {
        return Ok(None);
    };
    if emulator.launch_kind == "wine" {
        return Ok(None);
    }
    let engine_id = engine_id_for(&emulator);
    let Some(engine) = engine_for(&engine_id) else {
        return Ok(None);
    };
    let requested: Vec<String> = rom_path.map(|r| vec![r.to_string()]).unwrap_or_default();
    let Some(rom) = pick_rom_path(&meta, &requested, &SystemProbe) else {
        return Ok(None);
    };
    let profile = emulator.profile(meta.emulator_profile_id.as_deref().unwrap_or(""));
    let flatpak_id = (emulator.launch_kind == "flatpak")
        .then(|| {
            emulator
                .flatpak_id
                .clone()
                .filter(|f| !f.is_empty())
                .or_else(|| emulator_definition(&engine_id).and_then(|d| d.flatpak_id.clone()))
        })
        .flatten();
    let executable = if flatpak_id.is_some() {
        None
    } else {
        Some(emulator.executable.clone())
            .filter(|e| !e.is_empty())
            .or_else(|| meta.remapped_executable.clone().filter(|e| !e.is_empty()))
    };
    let core_path = profile.and_then(|p| resolve_template(&emulator, p).core_path);
    Ok(Some(Resolved {
        emulator_name: emulator.name.clone(),
        meta,
        engine,
        ctx: EngineContext { executable, flatpak_id, core_path },
        rom_path: rom,
    }))
}

fn iso_millis(t: SystemTime) -> String {
    DateTime::<Utc>::from(t).to_rfc3339_opts(SecondsFormat::Millis, true)
}

struct Found {
    state: EmulatorSaveState,
    key: String,
}

fn collect_states(r: &Resolved, env: &HostEnv, keys: &[String], modified_since: Option<SystemTime>) -> Vec<Found> {
    let mut found: Vec<Found> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for dir in unique(r.engine.state_dirs(env, &r.ctx, &r.rom_path, keys).into_iter().map(Some)) {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut names: Vec<String> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        for name in names {
            let Some(parsed) = r.engine.parse(&name) else { continue };
            if !keys.is_empty() && !keys.iter().any(|k| k.eq_ignore_ascii_case(&parsed.key)) {
                continue;
            }
            let path = join(&dir, &name);
            if seen.contains(&path) {
                continue;
            }
            let Ok(info) = std::fs::metadata(&path) else { continue };
            if !info.is_file() {
                continue;
            }
            let Ok(mtime) = info.modified() else { continue };
            if modified_since.is_some_and(|since| mtime < since) {
                continue;
            }
            seen.insert(path.clone());
            found.push(Found {
                key: parsed.key,
                state: EmulatorSaveState {
                    path,
                    kind: parsed.kind,
                    slot: parsed.slot,
                    modified_at: iso_millis(mtime),
                    thumbnail: None,
                },
            });
        }
    }
    found.sort_by(|a, b| b.state.modified_at.cmp(&a.state.modified_at));
    found
}

type ThumbCache = Mutex<HashMap<String, Option<Arc<Thumbnail>>>>;

fn thumb_cache() -> &'static ThumbCache {
    static CACHE: OnceLock<ThumbCache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Loads (and caches by path + mtime) the state's screenshot as raw bytes.
pub fn thumbnail_for(engine: Engine, state: &EmulatorSaveState) -> Option<Arc<Thumbnail>> {
    let key = format!("{}:{}", state.path, state.modified_at);
    if let Some(hit) = thumb_cache().lock().unwrap().get(&key) {
        return hit.clone();
    }
    let value = engine
        .thumbnail(&state.path)
        .filter(|b| !b.is_empty())
        .map(|bytes| Arc::new(Thumbnail { mime: image_mime(&bytes), bytes }));
    thumb_cache().lock().unwrap().insert(key, value.clone());
    value
}

fn rom_keys_for(r: &Resolved, env: &HostEnv) -> Vec<String> {
    unique(
        r.engine
            .rom_keys(env, &r.ctx, &r.rom_path)
            .into_iter()
            .map(Some)
            .chain([r.meta.save_state_keys.as_ref().and_then(|m| m.get(&r.rom_path)).cloned()]),
    )
}

/// `listEmulatorSaveStates`: the newest states of the game's ROM (at most
/// [`MAX_STATES`]); thumbnails are loaded when `with_thumbnails`.
pub fn list_emulator_save_states(
    paths: &HeroicPaths,
    env: &HostEnv,
    app_name: &str,
    rom_path: Option<&str>,
    with_thumbnails: bool,
) -> Result<EmulatorSaveStateList> {
    let Some(r) = resolve(paths, app_name, rom_path)? else {
        return Ok(EmulatorSaveStateList { supported: false, ..Default::default() });
    };
    let keys = rom_keys_for(&r, env);
    let mut list = EmulatorSaveStateList {
        supported: true,
        emulator_name: Some(r.emulator_name.clone()),
        rom_path: Some(r.rom_path.clone()),
        states: vec![],
    };
    if keys.is_empty() {
        return Ok(list);
    }
    list.states = collect_states(&r, env, &keys, None)
        .into_iter()
        .take(MAX_STATES)
        .map(|f| {
            let mut st = f.state;
            if with_thumbnails {
                st.thumbnail = thumbnail_for(r.engine, &st);
            }
            st
        })
        .collect();
    Ok(list)
}

/// The choice made in the Collection launch dialog, consumed by the next
/// launch of that game. It expires so an abandoned launch cannot leak into a
/// later one started from elsewhere (official Library, tray, etc.).
#[derive(Debug)]
pub struct PendingLaunchStates {
    ttl: Duration,
    inner: Mutex<HashMap<String, (LaunchStateArgs, String, Instant)>>,
}

impl Default for PendingLaunchStates {
    fn default() -> Self {
        Self::with_ttl(PENDING_TTL)
    }
}

impl PendingLaunchStates {
    pub fn with_ttl(ttl: Duration) -> Self {
        Self { ttl, inner: Default::default() }
    }

    /// Process-wide instance for apps that do not carry their own.
    pub fn global() -> &'static PendingLaunchStates {
        static G: OnceLock<PendingLaunchStates> = OnceLock::new();
        G.get_or_init(Default::default)
    }

    pub fn clear(&self, app_name: &str) {
        self.inner.lock().unwrap().remove(app_name);
    }

    pub fn set(&self, app_name: &str, rom_path: &str, args: LaunchStateArgs) {
        self.inner
            .lock()
            .unwrap()
            .insert(app_name.into(), (args, rom_path.into(), Instant::now() + self.ttl));
    }

    /// `takePendingLaunchState`: consumes the entry (even when it does not
    /// apply, so it never leaks into a later launch).
    pub fn take(&self, app_name: &str, rom_path: Option<&str>) -> Option<LaunchStateArgs> {
        let (args, rom, expires) = self.inner.lock().unwrap().remove(app_name)?;
        if expires < Instant::now() {
            return None;
        }
        if rom_path.is_some_and(|r| !r.is_empty() && r != rom) {
            return None;
        }
        Some(args)
    }
}

/// `setPendingLaunchState`. With `state_path` the next launch boots into it;
/// without it, emulators that auto-load a state get "start fresh" arguments.
/// Returns `false` when the state no longer exists.
pub fn set_pending_launch_state(
    paths: &HeroicPaths,
    env: &HostEnv,
    pending: &PendingLaunchStates,
    app_name: &str,
    rom_path: Option<&str>,
    state_path: Option<&str>,
) -> Result<bool> {
    pending.clear(app_name);
    let Some(r) = resolve(paths, app_name, rom_path)? else {
        return Ok(state_path.is_none());
    };
    let launch = match state_path {
        Some(sp) => {
            let keys = rom_keys_for(&r, env);
            let states = if keys.is_empty() { vec![] } else { collect_states(&r, env, &keys, None) };
            let Some(state) = states.into_iter().find(|f| f.state.path == sp) else {
                return Ok(false);
            };
            r.engine
                .load_args(env, &r.ctx, &state.state.path, state.state.kind, state.state.slot)?
        }
        None => {
            let fresh = r.engine.fresh_args(env, &r.ctx)?;
            if fresh.is_empty() {
                return Ok(true);
            }
            LaunchStateArgs { args: fresh, replaces_image: false }
        }
    };
    pending.set(app_name, &r.rom_path, launch);
    Ok(true)
}

/// `rememberSaveStateKey`: after a session, states written for this game
/// reveal the key the emulator uses; storing it lets later launches list
/// states for images whose ID cannot be read (CHD, CSO, GCZ...). Returns the
/// learned key.
pub fn remember_save_state_key(
    paths: &HeroicPaths,
    env: &HostEnv,
    app_name: &str,
    rom_path: Option<&str>,
    started_at: SystemTime,
) -> Result<Option<String>> {
    let Some(rom_path) = rom_path else { return Ok(None) };
    let Some(r) = resolve(paths, app_name, Some(rom_path))? else {
        return Ok(None);
    };
    if !r.engine.learns_keys() {
        return Ok(None);
    }
    // The disc or the emulator's cache is authoritative; only learn when both
    // are unreadable, so switching games inside the emulator cannot mislabel.
    if !r.engine.rom_keys(env, &r.ctx, &r.rom_path).is_empty() {
        return Ok(None);
    }
    let since = started_at.checked_sub(Duration::from_secs(2)).unwrap_or(started_at);
    let written = collect_states(&r, env, &[], Some(since));
    let keys = unique(written.iter().map(|f| Some(f.key.clone())));
    if keys.len() != 1 {
        return Ok(None);
    }
    let key = keys[0].clone();
    let rom = r.rom_path.clone();
    json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |file| {
        let Some(meta) = file.games.get_mut(app_name) else { return Ok(None) };
        let current = meta.save_state_keys.as_ref().and_then(|m| m.get(&rom)).cloned().unwrap_or_default();
        if current.eq_ignore_ascii_case(&key) {
            return Ok(None);
        }
        meta.save_state_keys.get_or_insert_with(Default::default).insert(rom.clone(), key.clone());
        Ok(Some(key.clone()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn ctx() -> EngineContext {
        EngineContext::default()
    }

    #[test]
    fn parses_every_engine_file_name() {
        let p = |e: Engine, n: &str| e.parse(n);
        assert_eq!(p(Engine::Pcsx2, "SLUS-20731 (538ABF5D).resume.p2s"), Some(resume("SLUS-20731")));
        assert_eq!(p(Engine::Pcsx2, "SLUS-20731 (538ABF5D).03.p2s"), Some(slot("SLUS-20731", 3)));
        assert_eq!(p(Engine::Pcsx2, "memcard.ps2"), None);
        assert_eq!(p(Engine::DuckStation, "SLUS-00594_resume.sav"), Some(resume("SLUS-00594")));
        assert_eq!(p(Engine::DuckStation, "SLUS-00594_2.sav"), Some(slot("SLUS-00594", 2)));
        assert_eq!(p(Engine::DuckStation, "savestate_1.sav"), None);
        assert_eq!(p(Engine::Ppsspp, "ULUS10041_1.00_0.ppst"), Some(slot("ULUS10041", 1)));
        assert_eq!(p(Engine::Ppsspp, "ULUS10041_1.00_0.jpg"), None);
        assert_eq!(p(Engine::Dolphin, "GALE01.s01"), Some(slot("GALE01", 1)));
        assert_eq!(p(Engine::Dolphin, "GALE01.sav"), None);
        assert_eq!(p(Engine::RetroArch, "Zelda.state"), Some(slot("Zelda", 0)));
        assert_eq!(p(Engine::RetroArch, "Zelda.state3"), Some(slot("Zelda", 3)));
        assert_eq!(p(Engine::RetroArch, "Zelda.state.auto"), Some(resume("Zelda")));
        assert_eq!(p(Engine::RetroArch, "Zelda.state.png"), None);
        assert_eq!(p(Engine::Mgba, "Pokemon.ss0"), Some(resume("Pokemon")));
        assert_eq!(p(Engine::Mgba, "Pokemon.ss4"), Some(slot("Pokemon", 4)));
        assert_eq!(
            p(Engine::Rpcs3, "BCUS98114_2024-01-01.SAVESTAT.zst").map(|s| s.key),
            Some("BCUS98114".to_string())
        );
        assert_eq!(p(Engine::Rpcs3, "notes.txt"), None);
    }

    #[test]
    fn engine_ids_for_custom_emulators() {
        let mut e = UserEmulator {
            definition_id: "custom".into(),
            name: "PCSX2".into(),
            executable: "/x/pcsx2-v2.AppImage".into(),
            ..Default::default()
        };
        assert_eq!(engine_id_for(&e), "pcsx2");
        e.name = "My emu".into();
        assert_eq!(engine_id_for(&e), "pcsx2");
        e.executable = "/x/thing".into();
        e.flatpak_id = Some("org.DolphinEmu.dolphin-emu".into());
        assert_eq!(engine_id_for(&e), "dolphin");
        e.flatpak_id = None;
        assert_eq!(engine_id_for(&e), "custom");
        e.definition_id = "mgba".into();
        assert_eq!(engine_id_for(&e), "mgba");
        assert!(engine_for("melonds").is_none());
    }

    #[test]
    fn state_dirs_follow_each_emulator_layout() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let env = HostEnv::sandbox(&home, d.path().join("tmp"));
        // PCSX2: default and configured.
        let dirs = Engine::Pcsx2.state_dirs(&env, &ctx(), "/r/a.iso", &[]);
        assert_eq!(dirs, [s(&home.join(".config/PCSX2/sstates"))]);
        fs::create_dir_all(home.join(".config/PCSX2/inis")).unwrap();
        fs::write(home.join(".config/PCSX2/inis/PCSX2.ini"), "[Folders]\nSavestates = /data/states\n").unwrap();
        assert_eq!(Engine::Pcsx2.state_dirs(&env, &ctx(), "/r/a.iso", &[]), ["/data/states"]);
        // Flatpak PCSX2.
        let fp = EngineContext { flatpak_id: Some("net.pcsx2.PCSX2".into()), ..ctx() };
        assert_eq!(
            Engine::Pcsx2.state_dirs(&env, &fp, "/r/a.iso", &[]),
            [s(&home.join(".var/app/net.pcsx2.PCSX2/config/PCSX2/sstates"))]
        );
        // Portable AppImage dir.
        let portable = d.path().join("portable");
        fs::create_dir_all(&portable).unwrap();
        fs::write(portable.join("portable.txt"), "").unwrap();
        let pc = EngineContext { executable: Some(s(&portable.join("pcsx2.AppImage"))), ..ctx() };
        assert_eq!(Engine::Pcsx2.state_dirs(&env, &pc, "/r/a.iso", &[]), [s(&portable.join("sstates"))]);
        // PPSSPP / Dolphin / RPCS3 / mGBA.
        assert_eq!(
            Engine::Ppsspp.state_dirs(&env, &ctx(), "", &[]),
            [s(&home.join(".config/ppsspp/PSP/PPSSPP_STATE"))]
        );
        assert_eq!(
            Engine::Dolphin.state_dirs(&env, &ctx(), "", &[]),
            [s(&home.join(".local/share/dolphin-emu/StateSaves"))]
        );
        fs::create_dir_all(home.join(".dolphin-emu")).unwrap();
        assert_eq!(
            Engine::Dolphin.state_dirs(&env, &ctx(), "", &[]),
            [s(&home.join(".dolphin-emu/StateSaves"))]
        );
        assert_eq!(
            Engine::Rpcs3.state_dirs(&env, &ctx(), "", &["BCUS98114".into()]),
            [s(&home.join(".config/rpcs3/savestates/BCUS98114"))]
        );
        assert_eq!(
            Engine::Mgba.state_dirs(&env, &ctx(), "/roms/gba/p.gba", &[]),
            ["/roms/gba"]
        );
        fs::create_dir_all(home.join(".config/mgba")).unwrap();
        fs::write(home.join(".config/mgba/config.ini"), "[ports.qt]\nsavestatePath=states\n").unwrap();
        assert_eq!(
            Engine::Mgba.state_dirs(&env, &ctx(), "/roms/gba/p.gba", &[]),
            ["/roms/gba/states", "/roms/gba"]
        );
        // DuckStation: only existing candidate roots.
        assert!(Engine::DuckStation.state_dirs(&env, &ctx(), "", &[]).is_empty());
        fs::create_dir_all(home.join(".local/share/duckstation")).unwrap();
        assert_eq!(
            Engine::DuckStation.state_dirs(&env, &ctx(), "", &[]),
            [s(&home.join(".local/share/duckstation/savestates"))]
        );
    }

    #[test]
    fn retroarch_dirs_and_launch_args() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let tmp = d.path().join("tmp");
        let env = HostEnv::sandbox(&home, &tmp);
        let cfg = home.join(".config/retroarch");
        fs::create_dir_all(cfg.join("states/Snes9x")).unwrap();
        fs::create_dir_all(cfg.join("states/Other")).unwrap();
        fs::create_dir_all(cfg.join("cores")).unwrap();
        fs::write(cfg.join("cores/snes9x_libretro.info"), "corename = \"Snes9x\"\n").unwrap();
        fs::write(cfg.join("retroarch.cfg"), "savestate_auto_load = \"true\"\n").unwrap();
        let rc = EngineContext {
            core_path: Some(s(&cfg.join("cores/snes9x_libretro.so"))),
            ..ctx()
        };
        let states = s(&cfg.join("states"));
        assert_eq!(
            Engine::RetroArch.state_dirs(&env, &rc, "/r/z.sfc", &[]),
            [states.clone(), format!("{states}/Snes9x")]
        );
        // Without a core: root plus every child folder.
        assert_eq!(
            Engine::RetroArch.state_dirs(&env, &ctx(), "/r/z.sfc", &[]),
            [states.clone(), format!("{states}/Other"), format!("{states}/Snes9x")]
        );
        // Slot loads by number, auto state goes through appendconfig.
        let slot3 = Engine::RetroArch.load_args(&env, &rc, "/p", SaveStateKind::Slot, Some(3)).unwrap();
        assert_eq!(slot3.args, ["-e", "3"]);
        let auto = Engine::RetroArch.load_args(&env, &rc, "/p", SaveStateKind::Resume, None).unwrap();
        let file = auto.args[0].strip_prefix("--appendconfig=").unwrap();
        assert!(file.ends_with("heronite-retroarch-autoload-true.cfg"));
        assert_eq!(fs::read_to_string(file).unwrap(), "savestate_auto_load = \"true\"\n");
        let fresh = Engine::RetroArch.fresh_args(&env, &rc).unwrap();
        assert!(fresh[0].ends_with("heronite-retroarch-autoload-false.cfg"));
        fs::write(cfg.join("retroarch.cfg"), "savestate_auto_load = \"false\"\n").unwrap();
        assert!(Engine::RetroArch.fresh_args(&env, &rc).unwrap().is_empty());
    }

    #[test]
    fn launch_args_per_engine() {
        let env = HostEnv::sandbox("/h", "/t");
        let la = |e: Engine, kind, slot| e.load_args(&env, &ctx(), "/st/file", kind, slot).unwrap();
        assert_eq!(la(Engine::Pcsx2, SaveStateKind::Slot, Some(1)).args, ["-statefile", "/st/file"]);
        assert_eq!(la(Engine::DuckStation, SaveStateKind::Resume, None).args, ["-statefile", "/st/file"]);
        assert_eq!(la(Engine::Ppsspp, SaveStateKind::Slot, Some(1)).args, ["--state=/st/file"]);
        assert_eq!(la(Engine::Dolphin, SaveStateKind::Slot, Some(1)).args, ["--save_state", "/st/file"]);
        assert_eq!(la(Engine::Mgba, SaveStateKind::Slot, Some(1)).args, ["-t", "/st/file"]);
        let rp = la(Engine::Rpcs3, SaveStateKind::Slot, None);
        assert_eq!(rp.args, ["--savestate", "/st/file"]);
        assert!(rp.replaces_image);
        assert_eq!(Engine::Mgba.fresh_args(&env, &ctx()).unwrap(), ["-C", "autoload=0"]);
        assert!(Engine::Pcsx2.fresh_args(&env, &ctx()).unwrap().is_empty());
    }

    #[test]
    fn apply_prepends_and_can_drop_the_image() {
        let argv = vec!["-fullscreen".to_string(), "/r/a.iso".to_string()];
        let s1 = LaunchStateArgs { args: vec!["-statefile".into(), "/st".into()], replaces_image: false };
        assert_eq!(s1.apply(argv.clone(), Some("/r/a.iso")), ["-statefile", "/st", "-fullscreen", "/r/a.iso"]);
        let s2 = LaunchStateArgs { args: vec!["--savestate".into(), "/st".into()], replaces_image: true };
        assert_eq!(s2.apply(argv, Some("/r/a.iso")), ["--savestate", "/st", "-fullscreen"]);
    }

    #[test]
    fn pending_states_are_consumed_and_expire() {
        let p = PendingLaunchStates::with_ttl(Duration::from_millis(30));
        let a = LaunchStateArgs { args: vec!["x".into()], replaces_image: false };
        p.set("g", "/r/a.iso", a.clone());
        assert_eq!(p.take("g", Some("/r/a.iso")), Some(a.clone()));
        assert_eq!(p.take("g", Some("/r/a.iso")), None);
        p.set("g", "/r/a.iso", a.clone());
        assert_eq!(p.take("g", Some("/r/b.iso")), None);
        assert_eq!(p.take("g", None), None);
        p.set("g", "/r/a.iso", a.clone());
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(p.take("g", None), None);
        p.set("g", "/r/a.iso", a.clone());
        assert_eq!(p.take("g", None), Some(a));
    }

    /// End to end with a PCSX2 sandbox: ROM key from the game list cache,
    /// states listed newest first with thumbnails, pending args, and key
    /// learning.
    #[test]
    fn lists_states_end_to_end_for_pcsx2() {
        use crate::types::UpsertUserEmulatorArgs;
        let heroic = tempfile::tempdir().unwrap();
        let paths = HeroicPaths::at(heroic.path());
        let sandbox = tempfile::tempdir().unwrap();
        let home = sandbox.path().join("home");
        let env = HostEnv::sandbox(&home, sandbox.path().join("tmp"));
        let data = home.join(".config/PCSX2");
        let sstates = data.join("sstates");
        fs::create_dir_all(data.join("cache")).unwrap();
        fs::create_dir_all(&sstates).unwrap();
        let rom_dir = sandbox.path().join("roms");
        fs::create_dir_all(&rom_dir).unwrap();
        let rom = s(&rom_dir.join("Game (USA).iso"));
        fs::write(&rom, "not an iso").unwrap();
        let mut blob = vec![0u8; 9];
        blob.extend_from_slice(&(rom.len() as u32).to_le_bytes());
        blob.extend_from_slice(rom.as_bytes());
        blob.extend_from_slice(&10u32.to_le_bytes());
        blob.extend_from_slice(b"SLUS-20731");
        fs::write(data.join("cache/gamelist.cache"), blob).unwrap();

        let e = crate::store::upsert_user_emulator(
            &paths,
            UpsertUserEmulatorArgs {
                definition_id: "custom".into(),
                name: "PCSX2".into(),
                executable: "/opt/pcsx2.AppImage".into(),
                launch_kind: "native".into(),
                ..Default::default()
            },
        )
        .unwrap();
        json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |f| {
            f.games.insert(
                "emu_1".into(),
                heronite_data::models::LocalGameMeta {
                    app_name: "emu_1".into(),
                    runner: "sideload".into(),
                    launch_kind: "emulator".into(),
                    emulator_id: Some(e.id.clone()),
                    emulator_profile_id: Some(e.profiles[0].id.clone()),
                    roms: Some(vec![heronite_data::models::LocalGameRom { name: None, path: rom.clone(), extra: Default::default() }]),
                    ..Default::default()
                },
            );
            Ok(())
        })
        .unwrap();

        fs::write(sstates.join("SLUS-20731 (538ABF5D).resume.p2s"), "x").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let png = [&[0x89u8, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a][..], b"img"].concat();
        let zip = {
            // A stored-only zip with Screenshot.png, built by hand.
            let name = b"Screenshot.png";
            let mut out = Vec::new();
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&(png.len() as u32).to_le_bytes());
            out.extend_from_slice(&(png.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name);
            out.extend_from_slice(&png);
            let dir_offset = out.len() as u32;
            let mut c = Vec::new();
            c.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            c.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            c.extend_from_slice(&0u32.to_le_bytes());
            c.extend_from_slice(&(png.len() as u32).to_le_bytes());
            c.extend_from_slice(&(png.len() as u32).to_le_bytes());
            c.extend_from_slice(&(name.len() as u16).to_le_bytes());
            c.extend_from_slice(&[0; 12]);
            c.extend_from_slice(&0u32.to_le_bytes());
            c.extend_from_slice(name);
            out.extend_from_slice(&c);
            out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
            out.extend_from_slice(&[0; 4]);
            out.extend_from_slice(&1u16.to_le_bytes());
            out.extend_from_slice(&1u16.to_le_bytes());
            out.extend_from_slice(&(c.len() as u32).to_le_bytes());
            out.extend_from_slice(&dir_offset.to_le_bytes());
            out.extend_from_slice(&[0, 0]);
            out
        };
        fs::write(sstates.join("SLUS-20731 (538ABF5D).01.p2s"), zip).unwrap();
        fs::write(sstates.join("SLUS-99999 (11111111).01.p2s"), "other game").unwrap();

        let list = list_emulator_save_states(&paths, &env, "emu_1", None, true).unwrap();
        assert!(list.supported);
        assert_eq!(list.emulator_name.as_deref(), Some("PCSX2"));
        assert_eq!(list.rom_path.as_deref(), Some(rom.as_str()));
        assert_eq!(list.states.len(), 2);
        assert_eq!(list.states[0].kind, SaveStateKind::Slot);
        assert_eq!(list.states[0].slot, Some(1));
        assert_eq!(list.states[1].kind, SaveStateKind::Resume);
        let thumb = list.states[0].thumbnail.as_ref().unwrap();
        assert_eq!(thumb.mime, "image/png");
        assert_eq!(thumb.bytes, png);
        assert!(list.states[1].thumbnail.is_none());
        assert!(list.states[0].modified_at.ends_with('Z'));
        let json = serde_json::to_value(&list).unwrap();
        assert_eq!(json["states"][0]["kind"], "slot");
        assert!(json["states"][0].get("thumbnail").is_none());

        // Pending launch state.
        let pending = PendingLaunchStates::default();
        let st = list.states[0].path.clone();
        assert!(set_pending_launch_state(&paths, &env, &pending, "emu_1", None, Some(&st)).unwrap());
        let taken = pending.take("emu_1", Some(&rom)).unwrap();
        assert_eq!(taken.args, ["-statefile", st.as_str()]);
        assert!(!set_pending_launch_state(&paths, &env, &pending, "emu_1", None, Some("/gone")).unwrap());
        // Fresh start for PCSX2 needs no args.
        assert!(set_pending_launch_state(&paths, &env, &pending, "emu_1", None, None).unwrap());
        assert!(pending.take("emu_1", None).is_none());

        // Unsupported / unknown games.
        assert!(!list_emulator_save_states(&paths, &env, "nope", None, false).unwrap().supported);
    }

    #[test]
    fn learns_key_when_the_rom_is_unreadable() {
        use crate::types::UpsertUserEmulatorArgs;
        let heroic = tempfile::tempdir().unwrap();
        let paths = HeroicPaths::at(heroic.path());
        let sandbox = tempfile::tempdir().unwrap();
        let home = sandbox.path().join("home");
        let env = HostEnv::sandbox(&home, sandbox.path().join("tmp"));
        let states = home.join(".local/share/duckstation/savestates");
        fs::create_dir_all(&states).unwrap();
        let rom = s(&sandbox.path().join("Game.chd"));
        fs::write(&rom, "compressed").unwrap();
        let e = crate::store::upsert_user_emulator(
            &paths,
            UpsertUserEmulatorArgs {
                definition_id: "duckstation".into(),
                name: "DuckStation".into(),
                executable: "/opt/duck".into(),
                launch_kind: "native".into(),
                ..Default::default()
            },
        )
        .unwrap();
        json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |f| {
            f.games.insert(
                "g".into(),
                heronite_data::models::LocalGameMeta {
                    app_name: "g".into(),
                    runner: "sideload".into(),
                    launch_kind: "emulator".into(),
                    emulator_id: Some(e.id.clone()),
                    emulator_profile_id: Some(e.profiles[0].id.clone()),
                    roms: Some(vec![heronite_data::models::LocalGameRom { name: None, path: rom.clone(), extra: Default::default() }]),
                    ..Default::default()
                },
            );
            Ok(())
        })
        .unwrap();
        // Nothing listed before any session.
        let before = list_emulator_save_states(&paths, &env, "g", None, false).unwrap();
        assert!(before.supported && before.states.is_empty());
        let started = SystemTime::now();
        fs::write(states.join("SLUS-00594_resume.sav"), "x").unwrap();
        let key = remember_save_state_key(&paths, &env, "g", Some(&rom), started).unwrap();
        assert_eq!(key.as_deref(), Some("SLUS-00594"));
        let again = remember_save_state_key(&paths, &env, "g", Some(&rom), started).unwrap();
        assert_eq!(again, None);
        let after = list_emulator_save_states(&paths, &env, "g", None, false).unwrap();
        assert_eq!(after.states.len(), 1);
        assert_eq!(after.states[0].kind, SaveStateKind::Resume);
        // Two different keys in one session: ambiguous, nothing learned.
        fs::write(states.join("SLES-11111_1.sav"), "x").unwrap();
        let m: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap();
        assert_eq!(m.games["g"].save_state_keys.as_ref().unwrap()[&rom], "SLUS-00594");
    }
}
