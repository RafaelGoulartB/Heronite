//! The compatibility tools in `<data>/tools`: DXVK, VKD3D-Proton,
//! DXVK-NVAPI and winetricks (port of `installOrUpdateTool`, `DXVK.getLatest`
//! and `Winetricks.download` in `backend/tools/index.ts`).
//!
//! Layout (Heroic's):
//! `tools/<tool>/<release>/...` plus `tools/<tool>/latest_<tool>` holding the
//! name of the newest release folder.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use heronite_data::DataPaths;
use heronite_i18n::tr;
use regex::Regex;
use serde_json::Value;

use crate::net::{extract_tar, Cancel, Net, CANCELLED};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Dxvk,
    DxvkNvapi,
    Vkd3d,
}

impl Tool {
    pub const ALL: [Tool; 3] = [Tool::Vkd3d, Tool::Dxvk, Tool::DxvkNvapi];

    /// Folder name under `tools/` (also Heroic's tool name).
    pub fn name(self) -> &'static str {
        match self {
            Tool::Dxvk => "dxvk",
            Tool::DxvkNvapi => "dxvk-nvapi",
            Tool::Vkd3d => "vkd3d",
        }
    }

    /// Name shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            Tool::Dxvk => "DXVK",
            Tool::DxvkNvapi => "DXVK-NVAPI",
            Tool::Vkd3d => "VKD3D",
        }
    }

    /// Release folders have a `x32` (`x86` for VKD3D) and a `x64` folder.
    pub fn dir32(self) -> &'static str {
        if self == Tool::Vkd3d {
            "x86"
        } else {
            "x32"
        }
    }

    /// Leading path components of the archive to drop (`strip` in Heroic).
    fn strip(self) -> u32 {
        if self == Tool::DxvkNvapi {
            0
        } else {
            1
        }
    }

    /// GitHub release endpoint, depending on the Vulkan support of the GPUs.
    pub fn release_url(self, vulkan: Vulkan) -> &'static str {
        match (self, vulkan) {
            (Tool::Dxvk, Vulkan::Legacy) => "https://api.github.com/repos/doitsujin/dxvk/releases/tags/v1.10.3",
            (Tool::Dxvk, _) => "https://api.github.com/repos/doitsujin/dxvk/releases/latest",
            (Tool::Vkd3d, Vulkan::Legacy) => "https://api.github.com/repos/Heroic-Games-Launcher/vkd3d-proton/releases/tags/v2.6",
            (Tool::Vkd3d, _) => "https://api.github.com/repos/Heroic-Games-Launcher/vkd3d-proton/releases/latest",
            (Tool::DxvkNvapi, _) => "https://api.github.com/repos/jp7677/dxvk-nvapi/releases/latest",
        }
    }
}

/// How modern the GPUs' Vulkan is (`any_gpu_supports_version`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vulkan {
    /// 1.3 or newer, or unknown: current releases.
    Modern,
    /// Only 1.1/1.2: DXVK 1.10.3 and VKD3D 2.6.
    Legacy,
}

/// Highest `apiVersion` of every GPU in `vulkaninfo --summary` output.
pub fn parse_vulkan_summary(text: &str) -> Vulkan {
    let re = Regex::new(r"apiVersion\s*=\s*(?:\d+\s*\()?(\d+)\.(\d+)\.\d+").unwrap();
    let mut best: Option<(u32, u32)> = None;
    for c in re.captures_iter(text) {
        let v = (c[1].parse().unwrap_or(0), c[2].parse().unwrap_or(0));
        best = Some(best.map_or(v, |b| b.max(v)));
    }
    match best {
        Some(v) if ((1, 1)..(1, 3)).contains(&v) => Vulkan::Legacy,
        _ => Vulkan::Modern,
    }
}

/// Asks `vulkaninfo` (when installed). Unknown counts as modern.
pub fn detect_vulkan() -> Vulkan {
    std::process::Command::new("vulkaninfo")
        .arg("--summary")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| parse_vulkan_summary(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or(Vulkan::Modern)
}

pub fn tool_dir(paths: &DataPaths, tool: Tool) -> PathBuf {
    paths.tools_dir().join(tool.name())
}

fn latest_file(paths: &DataPaths, tool: Tool) -> PathBuf {
    tool_dir(paths, tool).join(format!("latest_{}", tool.name()))
}

/// Release folder the `latest_<tool>` file points to, when it exists.
pub fn installed_version(paths: &DataPaths, tool: Tool) -> Option<String> {
    let text = fs::read_to_string(latest_file(paths, tool)).ok()?;
    let version = text.lines().next()?.trim().to_string();
    (is_safe_name(&version) && tool_dir(paths, tool).join(&version).is_dir()).then_some(version)
}

/// Folder of the newest installed release.
pub fn installed_dir(paths: &DataPaths, tool: Tool) -> Option<PathBuf> {
    installed_version(paths, tool).map(|v| tool_dir(paths, tool).join(v))
}

fn is_safe_name(n: &str) -> bool {
    !n.is_empty() && n != "." && n != ".." && !n.contains('/') && !n.contains('\0')
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Updated {
    UpToDate(String),
    Installed(String),
}

/// Archive file name to the folder name Heroic uses.
fn release_name(asset: &str) -> String {
    asset.replace(".tar.gz", "").replace(".tar.xz", "").replace(".tar.zst", "")
}

/// `installOrUpdateTool`: installs the newest release unless it is already
/// there. Downloads into a temporary file and extracts into a hidden folder
/// that is renamed when complete, so an interruption never leaves a broken
/// release behind.
pub fn install_or_update(paths: &DataPaths, net: &dyn Net, tool: Tool, vulkan: Vulkan, cancel: Cancel) -> Result<Updated, String> {
    let release = net
        .get_json(tool.release_url(vulkan))
        .map_err(|e| tr!("could not fetch the {tool} release: {error}", tool = tool.label(), error = e))?;
    let asset = release
        .get("assets")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .ok_or_else(|| tr!("the {tool} release has no assets", tool = tool.label()))?;
    let name = asset.get("name").and_then(Value::as_str).unwrap_or_default();
    let url = asset.get("browser_download_url").and_then(Value::as_str).unwrap_or_default();
    let version = release_name(name);
    if !is_safe_name(&version) || url.is_empty() {
        return Err(tr!("the {tool} release has an unusable asset ({name})", tool = tool.label(), name = format!("{name:?}")));
    }

    let dir = tool_dir(paths, tool);
    if installed_version(paths, tool).as_deref() == Some(version.as_str()) {
        return Ok(Updated::UpToDate(version));
    }
    fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;

    let archive = dir.join(format!("{name}.part"));
    let _ = fs::remove_file(&archive);
    if let Err(e) = net.download(url, &archive, cancel) {
        let _ = fs::remove_file(&archive);
        return Err(if e == CANCELLED { e } else { tr!("downloading {tool} failed: {error}", tool = tool.label(), error = e) });
    }
    let final_dir = dir.join(&version);
    let tmp = dir.join(format!(".{version}.extracting"));
    let _ = fs::remove_dir_all(&tmp);
    let extracted = extract_tar(&archive, &tmp, tool.strip(), cancel);
    let _ = fs::remove_file(&archive);
    if let Err(e) = extracted {
        let _ = fs::remove_dir_all(&tmp);
        return Err(if e == CANCELLED { e } else { tr!("unpacking {tool} failed: {error}", tool = tool.label(), error = e) });
    }
    // Same version re-installed (a broken folder): replace it.
    if final_dir.exists() {
        let _ = fs::remove_dir_all(&final_dir);
    }
    fs::rename(&tmp, &final_dir).map_err(|e| {
        let _ = fs::remove_dir_all(&tmp);
        tr!("could not move {tool} into place: {error}", tool = tool.label(), error = e)
    })?;
    write_atomic(&latest_file(paths, tool), &version)?;
    Ok(Updated::Installed(version))
}

/// Makes sure some release of `tool` is installed (downloads when none is).
pub fn ensure(paths: &DataPaths, net: &dyn Net, tool: Tool, vulkan: Vulkan, cancel: Cancel) -> Result<PathBuf, String> {
    if let Some(dir) = installed_dir(paths, tool) {
        return Ok(dir);
    }
    if !net.is_online() {
        return Err(tr!("{tool} is not installed and there is no internet connection", tool = tool.label()));
    }
    install_or_update(paths, net, tool, vulkan, cancel)?;
    installed_dir(paths, tool).ok_or_else(|| tr!("{tool} was not installed", tool = tool.label()))
}

/// Writes a file next to its final place and renames it over. The file next
/// to it is made new, never written through: in a prefix, a program in the
/// sandbox could leave a link there to any file of the user.
pub fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
    use std::io::Write;
    let tmp = path.with_extension("tmp-write");
    let _ = fs::remove_file(&tmp);
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .and_then(|mut f| f.write_all(contents.as_bytes()))
        .map_err(|e| format!("writing {}: {e}", path.display()))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("writing {}: {e}", path.display())
    })
}

pub const WINETRICKS_URL: &str = "https://raw.githubusercontent.com/Winetricks/winetricks/master/src/winetricks";
const WINETRICKS_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

/// `Winetricks.download`: refreshes `tools/winetricks` when it is missing or
/// older than a week. Returns whether a download happened.
pub fn download_winetricks(paths: &DataPaths, net: &dyn Net, cancel: Cancel) -> Result<bool, String> {
    let target = paths.tools_dir().join("winetricks");
    let fresh = fs::metadata(&target)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .map(|age| age < WINETRICKS_MAX_AGE)
        .unwrap_or(false);
    if fresh {
        return Ok(false);
    }
    if !net.is_online() {
        return Err(tr!("winetricks cannot be downloaded without an internet connection").into());
    }
    fs::create_dir_all(paths.tools_dir()).map_err(|e| e.to_string())?;
    let part = paths.tools_dir().join("winetricks.part");
    net.download(WINETRICKS_URL, &part, cancel).map_err(|e| {
        let _ = fs::remove_file(&part);
        tr!("downloading winetricks failed: {error}", error = e)
    })?;
    heronite_platform::fs::make_executable(&part).map_err(|e| e.to_string())?;
    fs::rename(&part, &target).map_err(|e| format!("installing winetricks: {e}"))?;
    Ok(true)
}

/// What `DXVK.getLatest()` plus `Winetricks.download()` do at startup: update
/// every tool, reporting each failure but never stopping at the first.
pub fn update_all(paths: &DataPaths, net: &dyn Net, cancel: Cancel, report: &dyn Fn(&str)) {
    if !net.is_online() {
        log::info!("offline, skipping the tools update");
        return;
    }
    let vulkan = detect_vulkan();
    for tool in Tool::ALL {
        match install_or_update(paths, net, tool, vulkan, cancel) {
            Ok(Updated::Installed(v)) => {
                log::info!("{} updated to {v}", tool.label());
                report(&format!("{} updated to {v}", tool.label()));
            }
            Ok(Updated::UpToDate(_)) => {}
            Err(e) if e == CANCELLED => return,
            Err(e) => log::warn!("{e}"),
        }
    }
    if let Err(e) = download_winetricks(paths, net, cancel) {
        log::warn!("{e}");
    }
    crate::runtimes::update_installed(paths, net, cancel, report);
}

const CHECK_STAMP: &str = ".heronite-last-update-check";
const CHECK_INTERVAL: Duration = Duration::from_secs(12 * 3600);

/// [`update_all`] at most twice a day (a stamp file in `tools/`), so frequent
/// restarts do not hit GitHub's rate limit each time. Heroic checks on every
/// start; the result is the same, only later.
pub fn update_all_if_due(paths: &DataPaths, net: &dyn Net, cancel: Cancel, report: &dyn Fn(&str)) -> bool {
    let stamp = paths.tools_dir().join(CHECK_STAMP);
    let due = fs::metadata(&stamp)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .map(|age| age >= CHECK_INTERVAL)
        .unwrap_or(true);
    if !due || !net.is_online() {
        return false;
    }
    update_all(paths, net, cancel, report);
    if !cancel() {
        let _ = fs::create_dir_all(paths.tools_dir());
        let _ = fs::write(&stamp, "");
    }
    true
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use serde_json::json;
    use std::process::Command;

    /// A `.tar.gz` of `files` (relative path, contents) under one top folder.
    pub fn tarball(dir: &Path, top: Option<&str>, files: &[(&str, &str)]) -> Vec<u8> {
        let src = dir.join(format!("src-{}", files.len()));
        let _ = fs::remove_dir_all(&src);
        let root = match top {
            Some(t) => src.join(t),
            None => src.clone(),
        };
        for (rel, body) in files {
            let p = root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, body).unwrap();
        }
        let out = dir.join(format!("out-{}.tar.gz", files.len()));
        let target = top.unwrap_or(".");
        assert!(Command::new("tar").arg("-czf").arg(&out).arg("-C").arg(&src).arg(target).status().unwrap().success());
        fs::read(out).unwrap()
    }

    pub fn release(name: &str, url: &str) -> Value {
        json!({"assets": [{"name": name, "browser_download_url": url}]})
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use crate::net::FakeNet;

    #[test]
    fn vulkan_summary_levels() {
        assert_eq!(parse_vulkan_summary("apiVersion = 1.3.274"), Vulkan::Modern);
        assert_eq!(parse_vulkan_summary("apiVersion = 4206847 (1.3.255)"), Vulkan::Modern);
        assert_eq!(parse_vulkan_summary("apiVersion = 1.2.131\napiVersion = 1.1.0"), Vulkan::Legacy);
        // One modern GPU is enough.
        assert_eq!(parse_vulkan_summary("apiVersion = 1.2.1\napiVersion = 1.3.0"), Vulkan::Modern);
        assert_eq!(parse_vulkan_summary(""), Vulkan::Modern);
        assert_eq!(Tool::Dxvk.release_url(Vulkan::Legacy).rsplit('/').next(), Some("v1.10.3"));
        assert!(Tool::DxvkNvapi.release_url(Vulkan::Legacy).ends_with("/releases/latest"));
    }

    #[test]
    fn installs_a_release_in_heroics_layout() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heroic"));
        let tar = tarball(dir.path(), Some("dxvk-9.9"), &[("x64/d3d11.dll", "64"), ("x32/d3d11.dll", "32")]);
        let net = FakeNet::online()
            .with_json(Tool::Dxvk.release_url(Vulkan::Modern), release("dxvk-9.9.tar.gz", "http://x/dxvk-9.9.tar.gz"))
            .with_file("http://x/dxvk-9.9.tar.gz", tar);
        let got = install_or_update(&paths, &net, Tool::Dxvk, Vulkan::Modern, &|| false).unwrap();
        assert_eq!(got, Updated::Installed("dxvk-9.9".into()));
        let tools = paths.tools_dir().join("dxvk");
        assert_eq!(fs::read_to_string(tools.join("latest_dxvk")).unwrap(), "dxvk-9.9");
        assert_eq!(fs::read_to_string(tools.join("dxvk-9.9/x64/d3d11.dll")).unwrap(), "64");
        // No download leftovers.
        let names: Vec<String> = fs::read_dir(&tools).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into()).collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert_eq!(installed_version(&paths, Tool::Dxvk).as_deref(), Some("dxvk-9.9"));

        // Second time nothing is downloaded.
        let again = install_or_update(&paths, &net, Tool::Dxvk, Vulkan::Modern, &|| false).unwrap();
        assert_eq!(again, Updated::UpToDate("dxvk-9.9".into()));
        assert_eq!(net.requests().iter().filter(|u| u.ends_with(".tar.gz")).count(), 1);
    }

    #[test]
    fn nvapi_archives_are_not_stripped() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heroic"));
        let tar = tarball(dir.path(), None, &[("x64/nvapi64.dll", "n"), ("x32/nvapi.dll", "n")]);
        let net = FakeNet::online()
            .with_json(Tool::DxvkNvapi.release_url(Vulkan::Modern), release("dxvk-nvapi-v1.tar.gz", "http://x/n.tar.gz"))
            .with_file("http://x/n.tar.gz", tar);
        install_or_update(&paths, &net, Tool::DxvkNvapi, Vulkan::Modern, &|| false).unwrap();
        assert!(installed_dir(&paths, Tool::DxvkNvapi).unwrap().join("x64/nvapi64.dll").exists());
    }

    #[test]
    fn a_failed_download_leaves_the_installed_release_alone() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heroic"));
        let tools = paths.tools_dir().join("vkd3d");
        fs::create_dir_all(tools.join("vkd3d-proton-1")).unwrap();
        fs::write(tools.join("latest_vkd3d"), "vkd3d-proton-1\n").unwrap();
        let net =
            FakeNet::online().with_json(Tool::Vkd3d.release_url(Vulkan::Modern), release("vkd3d-proton-2.tar.zst", "http://x/missing"));
        let err = install_or_update(&paths, &net, Tool::Vkd3d, Vulkan::Modern, &|| false).unwrap_err();
        assert!(err.contains("VKD3D"), "{err}");
        assert_eq!(installed_version(&paths, Tool::Vkd3d).as_deref(), Some("vkd3d-proton-1"));
        assert!(!tools.join("vkd3d-proton-2").exists());
    }

    #[test]
    fn ensure_downloads_only_when_missing_and_online() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heroic"));
        let offline = FakeNet::offline();
        assert!(ensure(&paths, &offline, Tool::Dxvk, Vulkan::Modern, &|| false).unwrap_err().contains("internet"));
        assert!(offline.requests().is_empty());

        let tools = paths.tools_dir().join("dxvk");
        fs::create_dir_all(tools.join("dxvk-3.1.1")).unwrap();
        fs::write(tools.join("latest_dxvk"), "dxvk-3.1.1").unwrap();
        let online = FakeNet::online();
        assert_eq!(ensure(&paths, &online, Tool::Dxvk, Vulkan::Modern, &|| false).unwrap(), tools.join("dxvk-3.1.1"));
        assert!(online.requests().is_empty());
    }

    #[test]
    fn latest_pointing_to_a_missing_folder_counts_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heroic"));
        let tools = paths.tools_dir().join("dxvk");
        fs::create_dir_all(&tools).unwrap();
        fs::write(tools.join("latest_dxvk"), "../escape").unwrap();
        assert_eq!(installed_version(&paths, Tool::Dxvk), None);
        fs::write(tools.join("latest_dxvk"), "gone").unwrap();
        assert_eq!(installed_version(&paths, Tool::Dxvk), None);
    }

    #[test]
    fn the_startup_update_runs_at_most_twice_a_day() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heroic"));
        // Nothing is served: every tool fails, which is logged, never fatal.
        let net = FakeNet::online();
        assert!(update_all_if_due(&paths, &net, &|| false, &|_| {}));
        let asked = net.requests().len();
        assert!(asked >= 3, "{asked}");
        assert!(!update_all_if_due(&paths, &net, &|| false, &|_| {}));
        assert_eq!(net.requests().len(), asked);
        // An old stamp makes it due again; offline it is never attempted.
        let stamp = paths.tools_dir().join(CHECK_STAMP);
        let old = SystemTime::now() - Duration::from_secs(13 * 3600);
        fs::File::options().write(true).open(&stamp).unwrap().set_modified(old).unwrap();
        assert!(!update_all_if_due(&paths, &FakeNet::offline(), &|| false, &|_| {}));
        assert!(update_all_if_due(&paths, &net, &|| false, &|_| {}));
    }

    #[test]
    fn winetricks_is_refreshed_weekly() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heroic"));
        let net = FakeNet::online().with_file(WINETRICKS_URL, b"#!/bin/sh\n".to_vec());
        assert!(download_winetricks(&paths, &net, &|| false).unwrap());
        let file = paths.tools_dir().join("winetricks");
        assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o755);
        assert!(!download_winetricks(&paths, &net, &|| false).unwrap());
        assert!(download_winetricks(&paths, &FakeNet::offline(), &|| false).is_ok());
    }
}
