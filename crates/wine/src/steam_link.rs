//! Proton builds in `tools/proton` registered with Steam, so its games can
//! pick them like the ones in `compatibilitytools.d`: each gets a folder
//! there (`heronite-<build>`) holding only a `compatibilitytool.vdf` whose
//! `install_path` points at the build. Kept while "Install Proton into
//! Steam" is on; Steam reads the folder when it starts. A registration that
//! points into another data folder (a test copy run with `HERONITE_DATA_DIR`)
//! is that folder's: left alone, or the real one would come and go.

use std::fs;
use std::path::{Path, PathBuf};

use heronite_data::DataPaths;

use crate::types::WineConfig;

const PREFIX: &str = "heronite-";
const MANIFEST: &str = "compatibilitytool.vdf";
const MARK: &str = "// Written by Heronite";

/// Brings the registrations in line with `tools/proton` and the setting.
/// Blocking (a few small files).
pub fn sync(data_paths: &DataPaths) {
    let cfg = WineConfig::load(data_paths);
    let Some(steam_tools) = steam_tools_dir(&cfg) else { return };
    let proton = data_paths.tools_dir().join("proton");
    let builds = if cfg.download_proton_to_steam { builds(&proton) } else { Vec::new() };
    if let Err(e) = sync_into(&steam_tools, &proton, &builds) {
        log::warn!("Could not register Heronite's Proton builds with Steam in {}: {e}", steam_tools.display());
    }
}

/// `compatibilitytools.d` of the configured Steam. Not Steam's Flatpak: its
/// sandbox does not see Heronite's folder.
fn steam_tools_dir(cfg: &WineConfig) -> Option<PathBuf> {
    let steam = Path::new(&cfg.default_steam_path);
    let real = fs::canonicalize(steam).unwrap_or_else(|_| steam.to_path_buf());
    if !steam.join("steam.sh").exists() || real.to_string_lossy().contains("/.var/app/com.valvesoftware.Steam/") {
        return None;
    }
    Some(steam.join("compatibilitytools.d"))
}

/// `(folder name, path)` of every complete build (has `proton` and the
/// `toolmanifest.vdf` Steam runs it with).
fn builds(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(rd) = fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<(String, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            let path = e.path();
            (!name.starts_with('.') && path.join("proton").is_file() && path.join("toolmanifest.vdf").is_file()).then_some((name, path))
        })
        .collect();
    out.sort();
    out
}

/// `proton`: this data folder's `tools/proton`.
fn sync_into(steam_tools: &Path, proton: &Path, builds: &[(String, PathBuf)]) -> std::io::Result<()> {
    let wanted: Vec<(String, String)> = builds.iter().map(|(name, path)| (format!("{PREFIX}{name}"), manifest(name, path))).collect();
    if let Ok(rd) = fs::read_dir(steam_tools) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with(PREFIX) && !wanted.iter().any(|(w, _)| *w == name) && is_ours(&e.path()) && is_here(&e.path(), proton) {
                fs::remove_file(e.path().join(MANIFEST))?;
                fs::remove_dir(e.path())?;
            }
        }
    }
    for (name, text) in &wanted {
        let dir = steam_tools.join(name);
        let file = dir.join(MANIFEST);
        if fs::read_to_string(&file).is_ok_and(|t| t == *text) {
            continue;
        }
        if dir.exists() && !is_ours(&dir) {
            log::warn!("{} is not Heronite's; leaving it alone", dir.display());
            continue;
        }
        if dir.exists() && !is_here(&dir, proton) {
            log::info!("{} belongs to another Heronite data folder; leaving it alone", dir.display());
            continue;
        }
        fs::create_dir_all(&dir)?;
        heronite_data::temps::write_atomic(&file, text.as_bytes(), None)?;
    }
    Ok(())
}

/// A folder this module wrote: only the marked manifest in it.
fn is_ours(dir: &Path) -> bool {
    let Ok(rd) = fs::read_dir(dir) else { return false };
    let names: Vec<_> = rd.flatten().map(|e| e.file_name()).collect();
    names.len() == 1 && names[0] == MANIFEST && fs::read_to_string(dir.join(MANIFEST)).is_ok_and(|t| t.starts_with(MARK))
}

/// One of our registrations that points into `proton`, or at a build that
/// is gone.
fn is_here(dir: &Path, proton: &Path) -> bool {
    let Ok(text) = fs::read_to_string(dir.join(MANIFEST)) else { return true };
    let doc = crate::vdf::parse(&text);
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let path =
        doc.get("compatibilitytools").and_then(|c| c.get("compat_tools")).and_then(|t| t.get(&name)).and_then(|t| t.get("install_path"));
    path.and_then(|p| p.as_str()).map(Path::new).is_none_or(|p| p.starts_with(proton) || !p.exists())
}

fn manifest(name: &str, path: &Path) -> String {
    let quote = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "{MARK} for its Wine Manager; removed with the build.\n\
         \"compatibilitytools\"\n{{\n  \"compat_tools\"\n  {{\n    \"{PREFIX}{tool}\"\n    {{\n      \
         \"install_path\" \"{path}\"\n      \"display_name\" \"{tool} (Heronite)\"\n      \
         \"from_oslist\" \"windows\"\n      \"to_oslist\" \"linux\"\n    }}\n  }}\n}}\n",
        tool = quote(name),
        path = quote(&path.to_string_lossy()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(tools: &Path, name: &str) -> PathBuf {
        let dir = tools.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("proton"), "").unwrap();
        fs::write(dir.join("toolmanifest.vdf"), "").unwrap();
        dir
    }

    #[test]
    fn registers_builds_and_removes_only_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let tools = dir.path().join("tools/proton");
        let steam = dir.path().join("Steam/compatibilitytools.d");
        let ge = build(&tools, "GE-Proton-latest");
        fs::create_dir_all(tools.join("half-extracted")).unwrap();
        // Someone else's folder with our prefix stays.
        fs::create_dir_all(steam.join("heronite-other")).unwrap();
        fs::write(steam.join("heronite-other").join(MANIFEST), "\"compatibilitytools\" {}").unwrap();

        sync_into(&steam, &tools, &builds(&tools)).unwrap();
        let text = fs::read_to_string(steam.join("heronite-GE-Proton-latest").join(MANIFEST)).unwrap();
        assert!(text.starts_with(MARK));
        assert!(text.contains("\"heronite-GE-Proton-latest\""));
        assert!(text.contains(&format!("\"install_path\" \"{}\"", ge.display())));
        assert!(text.contains("\"display_name\" \"GE-Proton-latest (Heronite)\""));
        assert!(!steam.join("heronite-half-extracted").exists());

        fs::remove_dir_all(&ge).unwrap();
        sync_into(&steam, &tools, &builds(&tools)).unwrap();
        assert!(!steam.join("heronite-GE-Proton-latest").exists());
        assert!(steam.join("heronite-other").join(MANIFEST).exists());
    }

    #[test]
    fn leaves_the_builds_of_another_data_folder_alone() {
        let dir = tempfile::tempdir().unwrap();
        let (real, copy) = (dir.path().join("real/tools/proton"), dir.path().join("copy/tools/proton"));
        let steam = dir.path().join("Steam/compatibilitytools.d");
        let ge = build(&real, "GE-Proton-latest");
        sync_into(&steam, &real, &builds(&real)).unwrap();
        let file = steam.join("heronite-GE-Proton-latest").join(MANIFEST);
        let before = fs::read_to_string(&file).unwrap();

        // A copy without builds, then with its own.
        sync_into(&steam, &copy, &builds(&copy)).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), before);
        build(&copy, "GE-Proton-latest");
        sync_into(&steam, &copy, &builds(&copy)).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), before);

        // Its build gone, any data folder may clean it up.
        fs::remove_dir_all(&ge).unwrap();
        fs::remove_dir_all(&copy).unwrap();
        sync_into(&steam, &copy, &builds(&copy)).unwrap();
        assert!(!file.exists());
    }

    #[test]
    fn manifest_parses_as_vdf() {
        let text = manifest("A \"B\"", Path::new("/x/A \"B\""));
        let doc = crate::vdf::parse(&text);
        let tool = doc.get("compatibilitytools").and_then(|c| c.get("compat_tools")).and_then(|t| t.get("heronite-A \"B\""));
        assert_eq!(tool.and_then(|t| t.get("install_path")).and_then(|p| p.as_str()), Some("/x/A \"B\""));
    }
}
