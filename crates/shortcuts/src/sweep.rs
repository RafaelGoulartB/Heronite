//! Startup sweep of the temp files a killed process left behind: the JSON
//! stores' and the shortcuts' (`.<x>.heronite-rs-<pid>-<n>.tmp`), `shortcuts.vdf.tmp` next to a
//! `shortcuts.vdf`, and `<x>.desktop.tmp` in the shortcut folders. Only those
//! exact names; see [`heronite_data::temps`].

use std::fs;
use std::path::{Path, PathBuf};

use heronite_data::temps;
use heronite_data::{game_config, json_store, AppPaths, DataPaths};

use crate::menu::Roots;
use crate::steam::{clean_steam_path, steam_root};

/// Everything [`sweep_stale_temps_at_startup`] looks at.
#[derive(Debug, Clone, Default)]
pub struct Targets {
    /// Folders holding our JSON stores.
    pub json_dirs: Vec<PathBuf>,
    /// Desktop and menu folders (application menu, Start menu).
    pub shortcut_dirs: Vec<PathBuf>,
    /// Steam install folder (`userdata/<user>/config/shortcuts.vdf`).
    pub steam: Option<PathBuf>,
}

/// Call once at startup, on a background thread. Returns how many files were
/// removed.
pub fn sweep_stale_temps_at_startup(data_paths: &DataPaths, app: &AppPaths) -> usize {
    let roots = Roots::detect();
    let home = dirs::home_dir().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default();
    let steam = std::env::var_os("HERONITE_STEAM_DIR").map(PathBuf::from).or_else(|| {
        let g = game_config::load_global(data_paths, &home);
        let raw = g.get("defaultSteamPath").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        (!raw.is_empty() || cfg!(windows)).then(|| steam_root(clean_steam_path(&raw)))
    });
    let mut shortcut_dirs = vec![roots.applications];
    shortcut_dirs.extend(roots.desktop);
    let removed = json_store::sweep_stale_temps_for(data_paths, app) + sweep(&Targets { json_dirs: vec![], shortcut_dirs, steam });
    if removed > 0 {
        log::info!("removed {removed} stale temp file(s) left by an interrupted run");
    }
    removed
}

/// The shortcut and Steam part of the sweep (plus `json_dirs`).
pub fn sweep(targets: &Targets) -> usize {
    let mut removed = json_store::sweep_stale_temps(&targets.json_dirs);
    for dir in &targets.shortcut_dirs {
        removed += temps::sweep_dir(dir).len();
        removed += sweep_legacy_desktop(dir);
    }
    if let Some(steam) = &targets.steam {
        if let Ok(users) = fs::read_dir(steam.join("userdata")) {
            for user in users.flatten() {
                let config = user.path().join("config");
                removed += temps::sweep_dir(&config).len();
                removed += sweep_legacy_vdf(&config);
            }
        }
    }
    removed
}

/// `<x>.desktop.tmp`, the temp name of older versions.
fn sweep_legacy_desktop(dir: &Path) -> usize {
    let mut removed = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.len() > ".desktop.tmp".len() && name.ends_with(".desktop.tmp") {
                temps::sweep_legacy_file(&entry.path(), &mut removed);
            }
        }
    }
    removed.len()
}

/// `shortcuts.vdf.tmp` next to a `shortcuts.vdf` (older versions' temp name).
fn sweep_legacy_vdf(config_dir: &Path) -> usize {
    let mut removed = Vec::new();
    if config_dir.join("shortcuts.vdf").is_file() {
        temps::sweep_legacy_file(&config_dir.join("shortcuts.vdf.tmp"), &mut removed);
    }
    removed.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn touch_old(p: &Path) {
        fs::write(p, "x").unwrap();
        let old = SystemTime::now() - Duration::from_secs(2 * 3600);
        fs::File::options().write(true).open(p).unwrap().set_modified(old).unwrap();
    }

    #[test]
    fn removes_only_our_leftovers_in_shortcut_and_steam_folders() {
        let dir = tempfile::tempdir().unwrap();
        let apps = dir.path().join("apps");
        let steam = dir.path().join("steam");
        let config = steam.join("userdata/1234/config");
        let orphan_config = steam.join("userdata/99/config");
        for d in [&apps, &config, &orphan_config] {
            fs::create_dir_all(d).unwrap();
        }
        // Ours.
        fs::write(apps.join(".Game.desktop.heronite-rs-4194999-3.tmp"), "x").unwrap();
        fs::write(apps.join(".Game.lnk.heronite-rs-4194999-4.tmp"), "x").unwrap();
        touch_old(&apps.join("Game.desktop.tmp"));
        fs::write(config.join(".shortcuts.vdf.heronite-rs-4194999-1.tmp"), "x").unwrap();
        fs::write(config.join("shortcuts.vdf"), "v").unwrap();
        touch_old(&config.join("shortcuts.vdf.tmp"));
        // Not ours / not stale: untouched.
        fs::write(apps.join("Game.desktop"), "d").unwrap();
        fs::write(apps.join("Game.lnk"), "l").unwrap();
        fs::write(apps.join("notes.tmp"), "n").unwrap();
        fs::write(apps.join(".hidden.tmp"), "n").unwrap();
        fs::write(config.join("shortcuts.vdf.heronite-bak"), "b").unwrap();
        fs::write(config.join("localconfig.vdf"), "l").unwrap();
        // A fresh legacy temp may belong to a running older version.
        fs::write(orphan_config.join("shortcuts.vdf"), "v").unwrap();
        fs::write(orphan_config.join("shortcuts.vdf.tmp"), "fresh").unwrap();

        let n = sweep(&Targets { json_dirs: vec![], shortcut_dirs: vec![apps.clone()], steam: Some(steam.clone()) });
        assert_eq!(n, 5);
        for kept in [
            apps.join("Game.desktop"),
            apps.join("Game.lnk"),
            apps.join("notes.tmp"),
            apps.join(".hidden.tmp"),
            config.join("shortcuts.vdf"),
            config.join("shortcuts.vdf.heronite-bak"),
            config.join("localconfig.vdf"),
            orphan_config.join("shortcuts.vdf.tmp"),
        ] {
            assert!(kept.exists(), "{} must be kept", kept.display());
        }
        // A legacy vdf temp without a shortcuts.vdf next to it is not ours to judge.
        fs::remove_file(orphan_config.join("shortcuts.vdf")).unwrap();
        touch_old(&orphan_config.join("shortcuts.vdf.tmp"));
        assert_eq!(sweep(&Targets { steam: Some(steam), ..Default::default() }), 0);
        assert!(orphan_config.join("shortcuts.vdf.tmp").exists());
    }
}
