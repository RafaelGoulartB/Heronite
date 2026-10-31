//! Where each emulator keeps its data, config and state files: the XDG and
//! Flatpak folders on Linux; on Windows Documents, AppData or the
//! emulator's own folder (portable installs, RetroArch, RPCS3).

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::engine::EngineContext;
use super::s;
use crate::pathutil::{base_no_ext, dirname, join, unique};
use crate::sysfiles::{resolve_configured_dir, HostEnv, Ini};

/// The executable's folder.
fn exe_dir(ctx: &EngineContext) -> Option<PathBuf> {
    ctx.executable.as_deref().map(|exe| PathBuf::from(dirname(exe)))
}

/// The executable's folder when it holds one of the portable-mode marker files.
fn portable_dir(ctx: &EngineContext, markers: &[&str]) -> Option<PathBuf> {
    let exe = ctx.executable.as_deref()?;
    let dir = dirname(exe);
    markers.iter().any(|m| Path::new(&dir).join(m).exists()).then(|| PathBuf::from(dir))
}

/// Sorted full paths of the sub-folders of `root`.
pub(super) fn child_dirs(root: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(root)
        .map(|rd| rd.filter_map(|e| e.ok()).filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false)).map(|e| s(&e.path())).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// PCSX2: `{Serial} ({CRC}).{NN}.p2s`, `{Serial} ({CRC}).resume.p2s` in
/// inis/PCSX2.ini [Folders] Savestates. `-statefile <file>` boots and loads.
pub(super) fn pcsx2_data_dirs(env: &HostEnv, ctx: &EngineContext) -> Vec<PathBuf> {
    if let Some(id) = &ctx.flatpak_id {
        return vec![env.flatpak_dir(id, "config", &["PCSX2"])];
    }
    match portable_dir(ctx, &["portable.ini", "portable.txt"]) {
        Some(p) => vec![p],
        None if cfg!(windows) => vec![env.documents.join("PCSX2")],
        None => vec![env.xdg_config_home().join("PCSX2")],
    }
}

/// DuckStation: `{Serial}_{N}.sav`, `{Serial}_resume.sav` in settings.ini
/// [Folders] SaveStates. Data root: $XDG_CONFIG_HOME/duckstation when that
/// variable is set, otherwise ~/.local/share/duckstation; on Windows
/// `%LOCALAPPDATA%\DuckStation` (older versions: `Documents\DuckStation`).
/// `-statefile <file>` loads.
pub(super) fn duckstation_data_dirs(env: &HostEnv, ctx: &EngineContext) -> Vec<PathBuf> {
    if let Some(id) = &ctx.flatpak_id {
        return vec![env.flatpak_dir(id, "data", &["duckstation"])];
    }
    if let Some(p) = portable_dir(ctx, &["portable.txt"]) {
        return vec![p];
    }
    if cfg!(windows) {
        return [env.local_appdata.join("DuckStation"), env.documents.join("DuckStation")].into_iter().filter(|d| d.exists()).collect();
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

/// PPSSPP: `{DISC_ID}_{DISC_VERSION}_{slot}.ppst` (slot 0-based) with a `.jpg`
/// next to it, in <memstick>/PSP/PPSSPP_STATE. `--state=<file>` loads after boot.
pub(super) fn ppsspp_state_dir(env: &HostEnv, ctx: &EngineContext) -> PathBuf {
    let memstick = match &ctx.flatpak_id {
        Some(id) => env.flatpak_dir(id, "config", &["ppsspp"]),
        None if cfg!(windows) => ppsspp_windows_memstick(env, ctx),
        None => env.xdg_config_home().join("ppsspp"),
    };
    memstick.join("PSP").join("PPSSPP_STATE")
}

/// PPSSPP on Windows: the `memstick` folder next to it (the zip build), the
/// folder its installer wrote in `installed.txt`, else `Documents\PPSSPP`.
fn ppsspp_windows_memstick(env: &HostEnv, ctx: &EngineContext) -> PathBuf {
    let dir = exe_dir(ctx);
    if let Some(memstick) = dir.as_ref().map(|d| d.join("memstick")).filter(|m| m.is_dir()) {
        return memstick;
    }
    let installed = dir.and_then(|d| std::fs::read_to_string(d.join("installed.txt")).ok());
    match installed.as_deref().and_then(|t| t.lines().next()).map(str::trim).filter(|l| !l.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => env.documents.join("PPSSPP"),
    }
}

/// Dolphin: `{GameID}.s{NN}` in <user>/StateSaves. `--save_state <file>` loads
/// at boot.
pub(super) fn dolphin_user_dir(env: &HostEnv, ctx: &EngineContext) -> PathBuf {
    if let Some(id) = &ctx.flatpak_id {
        return env.flatpak_dir(id, "data", &["dolphin-emu"]);
    }
    if let Some(p) = &env.dolphin_userpath {
        return p.clone();
    }
    if cfg!(windows) {
        return dolphin_windows_user_dir(env, ctx);
    }
    let legacy = env.home.join(".dolphin-emu");
    if legacy.exists() {
        legacy
    } else {
        env.xdg_data_home().join("dolphin-emu")
    }
}

/// Dolphin's `SetUserDirectory` on Windows, in its order: `User` next to a
/// portable copy (`portable.txt`, which never reads the registry), the folder
/// of the registry's `UserConfigPath` while it exists, `Documents\Dolphin
/// Emulator` when there (older versions), else `%APPDATA%\Dolphin Emulator`.
fn dolphin_windows_user_dir(env: &HostEnv, ctx: &EngineContext) -> PathBuf {
    if let Some(p) = portable_dir(ctx, &["portable.txt"]) {
        return p.join("User");
    }
    if let Some(p) = env.dolphin_registry_path.as_ref().filter(|p| p.is_dir()) {
        return p.clone();
    }
    let documents = env.documents.join("Dolphin Emulator");
    if documents.exists() {
        documents
    } else {
        env.appdata.join("Dolphin Emulator")
    }
}

// RetroArch: `{rom}.state` (slot 0), `{rom}.state{N}`, `{rom}.state.auto` in
// savestate_directory, sorted into a folder per core by default.
// `-e N` loads slot N at boot (normal slot files need RetroArch 1.21+); the
// auto state needs `savestate_auto_load`, passed via --appendconfig.

/// Where `retroarch.cfg` is: on Windows next to `retroarch.exe` (else
/// `%APPDATA%\RetroArch`).
pub(super) fn retroarch_config_dir(env: &HostEnv, ctx: &EngineContext) -> PathBuf {
    match &ctx.flatpak_id {
        Some(id) => env.flatpak_dir(id, "config", &["retroarch"]),
        None if cfg!(windows) => exe_dir(ctx).unwrap_or_else(|| env.appdata.join("RetroArch")),
        None => env.xdg_config_home().join("retroarch"),
    }
}

/// `corename` from the libretro `.info` file of the game's core.
pub(super) fn retroarch_core_name(env: &HostEnv, ctx: &EngineContext) -> Option<String> {
    let core = ctx.core_path.as_deref()?;
    let info_name = format!("{}.info", base_no_ext(core));
    let config_dir = retroarch_config_dir(env, ctx);
    let cfg_dir = s(&config_dir);
    let cfg = Ini::read(&config_dir.join("retroarch.cfg"));
    let dirs = unique([
        resolve_configured_dir(cfg.as_ref().and_then(|c| c.get("", "libretro_info_path")), &cfg_dir, &env.home),
        Some(join(&cfg_dir, "cores")),
        Some(join(&cfg_dir, "info")),
        Some(dirname(core)),
        (!cfg!(windows)).then_some("/usr/share/libretro/info".to_string()),
        (!cfg!(windows)).then_some("/usr/lib/libretro".to_string()),
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
pub(super) fn retroarch_append_config(env: &HostEnv, ctx: &EngineContext, auto_load: bool) -> Result<String> {
    let dir = match &ctx.flatpak_id {
        Some(id) => env.home.join(".var/app").join(id).join("cache"),
        None => env.tmp_dir.clone(),
    };
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(format!("heronite-retroarch-autoload-{auto_load}.cfg"));
    std::fs::write(&file, format!("savestate_auto_load = \"{auto_load}\"\n"))?;
    Ok(s(&file))
}

/// mGBA: `{rom}.ss{N}` next to the ROM or in [ports.qt] savestatePath. ss0 is
/// the auto slot that mGBA-Qt loads on boot by default (`autoload`). States are
/// PNG files with the emulator data embedded. `-t <file>` loads at boot. Its
/// config is in `%APPDATA%\mGBA` on Windows, next to it when portable.
pub(super) fn mgba_config_path(env: &HostEnv, ctx: &EngineContext) -> PathBuf {
    match &ctx.flatpak_id {
        Some(id) => env.flatpak_dir(id, "config", &["mgba", "config.ini"]),
        None if cfg!(windows) => portable_dir(ctx, &["portable.ini"]).unwrap_or_else(|| env.appdata.join("mGBA")).join("config.ini"),
        None => env.xdg_config_home().join("mgba").join("config.ini"),
    }
}

/// RPCS3: `<config>/savestates/{TITLE_ID}/{TITLE_ID}_*.SAVESTAT[.zst|.gz]`.
/// `--savestate <file>` boots the state instead of the game. On Windows its
/// config is its own folder.
pub(super) fn rpcs3_states_root(env: &HostEnv, ctx: &EngineContext) -> PathBuf {
    match &ctx.flatpak_id {
        Some(id) => env.flatpak_dir(id, "config", &["rpcs3", "savestates"]),
        None if cfg!(windows) => exe_dir(ctx).unwrap_or_else(|| env.appdata.join("rpcs3")).join("savestates"),
        None => env.xdg_config_home().join("rpcs3").join("savestates"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn dolphin_on_windows_follows_its_own_order() {
        let d = tempfile::tempdir().unwrap();
        let mut env = HostEnv::sandbox(d.path().join("home"), d.path().join("tmp"));
        let user = |env: &HostEnv, ctx: &EngineContext| dolphin_windows_user_dir(env, ctx);
        let none = EngineContext::default();
        assert_eq!(user(&env, &none), env.appdata.join("Dolphin Emulator"));
        // Documents of older versions, while it is there.
        fs::create_dir_all(env.documents.join("Dolphin Emulator")).unwrap();
        assert_eq!(user(&env, &none), env.documents.join("Dolphin Emulator"));
        // The registry's folder comes before both, only while it exists.
        let custom = d.path().join("Games").join("Dolphin User");
        env.dolphin_registry_path = Some(custom.clone());
        assert_eq!(user(&env, &none), env.documents.join("Dolphin Emulator"));
        fs::create_dir_all(&custom).unwrap();
        assert_eq!(user(&env, &none), custom);
        // A file is no folder.
        let file = d.path().join("file");
        fs::write(&file, "x").unwrap();
        env.dolphin_registry_path = Some(file);
        assert_eq!(user(&env, &none), env.documents.join("Dolphin Emulator"));
        // A portable copy has its own `User`, whatever the registry says.
        env.dolphin_registry_path = Some(custom.clone());
        let portable = d.path().join("Dolphin-x64");
        fs::create_dir_all(&portable).unwrap();
        let exe = EngineContext { executable: Some(s(&portable.join("Dolphin.exe"))), ..Default::default() };
        assert_eq!(user(&env, &exe), custom, "a copy without portable.txt is an install");
        fs::write(portable.join("portable.txt"), "").unwrap();
        assert_eq!(user(&env, &exe), portable.join("User"));
    }
}
