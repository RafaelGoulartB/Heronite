//! The EOS overlay (Epic Online Services' in-game overlay, legendary's
//! `eos-overlay`): a public build installed once like a game
//! (`overlay_install.json`), and turned on per Wine prefix (or for the user
//! on Windows) by the registry value the EOS SDK reads.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::api::ManifestInfo;
use crate::client::Client;
use crate::files;
use crate::installed::InstalledGame;
use crate::manifest::{plan, InstallPlan, Manifest, PlanOptions};
use crate::wine_reg;

pub const APP_NAME: &str = "98bc04bc842e4906993fd6d6644ffb8d";
pub const TITLE: &str = "Epic Online Services Overlay";
const NAMESPACE: &str = "302e5ede476149b1bc3e4fe6ae45e50e";
const CATALOG_ITEM_ID: &str = "cc15684f44d849e89e9bf4cec0508b68";
/// A folder with this file holds the overlay.
const MARKER: &str = "EOSOVH-Win64-Shipping.dll";
/// The key (escaped as in `user.reg`) and value the EOS SDK reads.
const WINE_KEY: &str = r"Software\\Epic Games\\EOS";
const VALUE: &str = "OverlayPath";
/// Registry keys of the overlay on Windows.
pub const WINDOWS_KEY: &str = r"SOFTWARE\Epic Games\EOS";
pub const VULKAN_LAYERS_KEY: &str = r"SOFTWARE\Khronos\Vulkan\ImplicitLayers";
const VERSION_MAX_AGE_SECS: f64 = 24.0 * 3600.0;

pub fn install_file(config_dir: &Path) -> PathBuf {
    config_dir.join("overlay_install.json")
}

pub fn version_file(config_dir: &Path) -> PathBuf {
    config_dir.join("overlay_version.json")
}

/// The installed overlay, if any.
pub fn installed(config_dir: &Path) -> Option<InstalledGame> {
    files::read_json(&install_file(config_dir)).and_then(|v| serde_json::from_value(v).ok())
}

/// Whether `path` holds the overlay.
pub fn is_overlay_install(path: &Path) -> bool {
    path.join(MARKER).exists()
}

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// The latest build version, from `overlay_version.json` when it is less
/// than a day old, else asked of Epic (anonymously) and stored like legendary
/// (`{"data": <manifest element>, "last_update": <unix time>}`).
pub fn latest_version(client: &Client, config_dir: &Path) -> Result<String> {
    let cached = files::read_json(&version_file(config_dir));
    let fresh =
        cached.as_ref().and_then(|c| c.get("last_update").and_then(Value::as_f64)).is_some_and(|t| now() - t < VERSION_MAX_AGE_SECS);
    if let (true, Some(v)) = (fresh, cached.as_ref().and_then(|c| c.pointer("/data/buildVersion")).and_then(Value::as_str)) {
        return Ok(v.to_string());
    }
    let token = client.anonymous_token()?;
    let url = format!(
        "{}/launcher/api/public/assets/v2/platform/Windows/namespace/{NAMESPACE}/catalogItem/{CATALOG_ITEM_ID}/app/{APP_NAME}/label/Live",
        client.endpoints.launcher
    );
    let doc = client.http.get_json(&client.get(&url).bearer(&token))?;
    let element = doc.pointer("/elements/0").cloned().unwrap_or(Value::Null);
    files::write_json(&version_file(config_dir), &json!({"data": element, "last_update": now()}), false)?;
    Ok(element.get("buildVersion").and_then(Value::as_str).unwrap_or_default().to_string())
}

/// The installed overlay is older than the latest build.
pub fn update_available(client: &Client, config_dir: &Path) -> Result<bool> {
    let Some(ig) = installed(config_dir) else { return Ok(false) };
    Ok(latest_version(client, config_dir)? != ig.version)
}

/// An overlay install to download.
#[derive(Debug, Clone)]
pub struct OverlayInstall {
    pub info: ManifestInfo,
    pub manifest: Manifest,
    pub plan: InstallPlan,
    /// What [`finish_install`] records once the files are in place.
    pub record: InstalledGame,
}

/// Plans installing (or updating) the overlay into `path` (the existing
/// install's folder wins).
pub fn prepare_install(client: &Client, config_dir: &Path, path: &Path) -> Result<OverlayInstall> {
    let token = client.anonymous_token()?;
    let info = client.manifest_info(&token, NAMESPACE, CATALOG_ITEM_ID, APP_NAME, "Windows", "Live")?;
    let bytes = client.download_manifest(&info)?;
    let mut manifest = Manifest::parse(&bytes)?;
    manifest.decrypt(&info.secrets)?;
    // The existing install's folder wins, when it is one the overlay may own.
    let path =
        installed(config_dir).map(|g| PathBuf::from(g.install_path)).filter(|p| is_own_folder(p)).unwrap_or_else(|| path.to_path_buf());
    if !is_own_folder(&path) {
        bail!("{} is not a folder the overlay can be installed in", path.display());
    }
    let plan = plan::plan(&manifest, None, &PlanOptions::default());
    let record = InstalledGame {
        app_name: APP_NAME.into(),
        title: TITLE.into(),
        version: manifest.meta.build_version.clone(),
        base_urls: info.base_urls.clone(),
        install_path: path.to_string_lossy().into_owned(),
        install_size: plan.install_size,
        ..Default::default()
    };
    Ok(OverlayInstall { info, manifest, plan, record })
}

/// Records the finished install.
pub fn finish_install(config_dir: &Path, record: &InstalledGame) -> Result<()> {
    files::write_json(&install_file(config_dir), &serde_json::to_value(record)?, false)
}

/// Whether `path` may hold the overlay's own files: an absolute folder at
/// least two levels deep, not the home folder or one above it, and either
/// missing, empty or holding the overlay already. Its folder is deleted
/// whole on removal, so it must never be a folder of other files.
pub fn is_own_folder(path: &Path) -> bool {
    use std::path::Component;
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return false;
    }
    if path.components().filter(|c| matches!(c, Component::Normal(_))).count() < 2 {
        return false;
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    if home.is_some_and(|h| h.starts_with(path)) {
        return false;
    }
    match std::fs::read_dir(path) {
        Ok(mut entries) => entries.next().is_none() || is_overlay_install(path),
        Err(_) => !path.exists(),
    }
}

/// Deletes the overlay's folder and its record. A recorded folder that is
/// not the overlay's (no overlay in it, or a folder like the home folder)
/// keeps its files: only the record goes.
pub fn remove(config_dir: &Path) -> Result<()> {
    if let Some(ig) = installed(config_dir) {
        let dir = Path::new(&ig.install_path);
        if dir.exists() {
            if is_overlay_install(dir) && is_own_folder(dir) {
                std::fs::remove_dir_all(dir).with_context(|| format!("removing {}", dir.display()))?;
            } else {
                log::warn!("eos: {} does not hold only the overlay; its files are kept", dir.display());
            }
        }
    }
    match std::fs::remove_file(install_file(config_dir)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// The overlay folder a Wine prefix's registry names, as a host path.
pub fn prefix_overlay_path(prefix: &Path) -> Option<PathBuf> {
    let v = wine_reg::value(prefix, WINE_KEY, VALUE)?;
    Some(wine_reg::to_host_path(prefix, &v))
}

/// Turns the overlay on in a Wine prefix (only `OverlayPath`: Windows'
/// Vulkan layers do not work under Wine).
pub fn enable_in_prefix(prefix: &Path, overlay: &Path) -> Result<()> {
    let p = overlay.to_string_lossy().replace('\\', "/");
    let value = if p.starts_with('/') { format!("Z:{p}") } else { p };
    wine_reg::set_string(prefix, WINE_KEY, VALUE, &value)
}

/// Turns the overlay off in a Wine prefix.
pub fn disable_in_prefix(prefix: &Path) -> Result<()> {
    wine_reg::remove_value(prefix, WINE_KEY, VALUE).map(drop)
}

/// Whether the prefix's registry points at an overlay install.
pub fn enabled_in_prefix(prefix: &Path) -> bool {
    prefix_overlay_path(prefix).is_some_and(|p| is_overlay_install(&p))
}

/// A value of the Windows registry (current user) the overlay needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegValue {
    String { key: &'static str, name: String, value: String },
    Dword { key: &'static str, name: String, value: u32 },
}

/// What legendary writes on Windows to turn the overlay on: `OverlayPath`
/// and the two Vulkan layers of the user.
pub fn windows_values(overlay: &Path) -> Vec<RegValue> {
    let dir = overlay.to_string_lossy();
    let layer = |file: &str| format!("{}\\{file}", dir.replace('/', "\\").trim_end_matches('\\'));
    vec![
        RegValue::String { key: WINDOWS_KEY, name: VALUE.into(), value: dir.replace('\\', "/") },
        RegValue::Dword { key: VULKAN_LAYERS_KEY, name: layer("EOSOverlayVkLayer-Win32.json"), value: 0 },
        RegValue::Dword { key: VULKAN_LAYERS_KEY, name: layer("EOSOverlayVkLayer-Win64.json"), value: 0 },
    ]
}

/// Folders where an overlay may already be (legendary's
/// `search_overlay_installs`): ours, the Epic Games Launcher's and the EOS
/// helper's in `prefix` (or on this PC without one), and the one the
/// registry names.
pub fn search_installs(config_dir: &Path, prefix: Option<&Path>) -> Vec<PathBuf> {
    let mut places: Vec<PathBuf> = installed(config_dir).map(|g| PathBuf::from(g.install_path)).into_iter().collect();
    let program_files = match prefix {
        Some(p) => Some(p.join("drive_c/Program Files (x86)")),
        None => std::env::var_os("ProgramFiles(x86)").map(PathBuf::from),
    };
    if let Some(pf) = program_files {
        places.push(pf.join("Epic Games/Launcher/Portal/Extras/Overlay"));
        places.push(pf.join(format!("Epic Games/Epic Online Services/managedArtifacts/{APP_NAME}")));
    }
    if let Some(p) = prefix.and_then(prefix_overlay_path) {
        places.push(p);
    }
    let mut found: Vec<PathBuf> = Vec::new();
    for p in places {
        if is_overlay_install(&p) && !found.contains(&p) {
            found.push(p);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    // A Wine prefix maps host paths through `Z:`, which only Unix hosts have.
    #[cfg(unix)]
    #[test]
    fn the_overlay_is_switched_in_a_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let pfx = dir.path().join("pfx");
        std::fs::create_dir_all(&pfx).unwrap();
        std::fs::write(wine_reg::user_reg(&pfx), wine_reg::tests::USER_REG).unwrap();
        let overlay = dir.path().join("overlay");
        std::fs::create_dir_all(&overlay).unwrap();
        std::fs::write(overlay.join(MARKER), b"").unwrap();
        assert!(!enabled_in_prefix(&pfx));
        enable_in_prefix(&pfx, &overlay).unwrap();
        assert!(enabled_in_prefix(&pfx));
        assert!(wine_reg::value(&pfx, WINE_KEY, VALUE).unwrap().starts_with("Z:/"));
        assert_eq!(search_installs(dir.path(), Some(&pfx)), std::slice::from_ref(&overlay));
        disable_in_prefix(&pfx).unwrap();
        assert!(!enabled_in_prefix(&pfx));
    }

    #[test]
    fn only_the_overlays_own_folder_is_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let record = |path: &Path| {
            let ig = InstalledGame { app_name: APP_NAME.into(), install_path: path.to_string_lossy().into(), ..Default::default() };
            finish_install(&cfg, &ig).unwrap();
        };
        // A folder of other files, recorded by mistake, keeps them.
        let games = dir.path().join("Games");
        std::fs::create_dir_all(games.join("Control")).unwrap();
        record(&games);
        assert!(!is_own_folder(&games));
        remove(&cfg).unwrap();
        assert!(games.join("Control").exists());
        assert!(installed(&cfg).is_none());
        // The overlay's folder goes.
        let overlay = dir.path().join("overlay");
        std::fs::create_dir_all(&overlay).unwrap();
        std::fs::write(overlay.join(MARKER), b"").unwrap();
        record(&overlay);
        remove(&cfg).unwrap();
        assert!(!overlay.exists());
        assert!(!is_own_folder(Path::new("/")));
        assert!(!is_own_folder(Path::new("relative/dir")));
        assert!(is_own_folder(&dir.path().join("new/overlay")));
    }

    #[test]
    fn records_and_windows_values() {
        let dir = tempfile::tempdir().unwrap();
        assert!(installed(dir.path()).is_none());
        let record = InstalledGame {
            app_name: APP_NAME.into(),
            version: "1.0".into(),
            install_path: dir.path().join("o").to_string_lossy().into(),
            ..Default::default()
        };
        finish_install(dir.path(), &record).unwrap();
        assert_eq!(installed(dir.path()).unwrap().version, "1.0");
        remove(dir.path()).unwrap();
        assert!(installed(dir.path()).is_none());
        let values = windows_values(Path::new(r"C:\Epic\Overlay"));
        assert_eq!(values[0], RegValue::String { key: WINDOWS_KEY, name: "OverlayPath".into(), value: "C:/Epic/Overlay".into() });
        assert!(matches!(&values[2], RegValue::Dword { name, .. } if name == r"C:\Epic\Overlay\EOSOverlayVkLayer-Win64.json"));
    }
}
