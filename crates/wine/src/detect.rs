//! Detection of usable Wine/Proton installations (port of
//! `getLinuxWineSet` and helpers in `backend/utils/compatibility_layers.ts`).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use heronite_data::DataPaths;

use crate::types::{WineConfig, WineInstallation};
use crate::vdf;

/// Where detection looks outside the data folder; injectable for tests.
#[derive(Debug, Clone)]
pub struct DetectEnv {
    pub home: PathBuf,
    /// Steam's system-wide library (Heroic always includes `/usr/share/steam`).
    pub system_steam: Option<PathBuf>,
    /// Look for `wine` on `PATH` (runs `wine --version`).
    pub system_wine: bool,
    /// Look in the configured Steam's libraries.
    pub steam: bool,
}

impl DetectEnv {
    /// This PC. `HERONITE_WINE_ONLY_TOOLS=1` (tests and screenshots with a
    /// copy of the data folder) counts only the builds of the data folder,
    /// not the system's Wine nor Steam's or Lutris' builds.
    pub fn real() -> Self {
        if std::env::var_os("HERONITE_WINE_ONLY_TOOLS").is_some_and(|v| v != "0") {
            return Self { home: PathBuf::from("/nonexistent"), system_steam: None, system_wine: false, steam: false };
        }
        Self {
            home: dirs::home_dir().unwrap_or_default(),
            system_steam: Some(PathBuf::from("/usr/share/steam")),
            system_wine: true,
            steam: true,
        }
    }
}

/// Every Wine/Proton the game settings may pick (Heroic's `getWineList` on
/// Linux), in Heroic's order. Blocking (scans folders): call off the UI thread.
pub fn list_installations(data_paths: &DataPaths) -> Vec<WineInstallation> {
    list_installations_with(data_paths, &WineConfig::load(data_paths), &DetectEnv::real())
}

pub fn list_installations_with(data_paths: &DataPaths, cfg: &WineConfig, env: &DetectEnv) -> Vec<WineInstallation> {
    let tools = data_paths.tools_dir();

    // tools/wine/<version>/bin/wine and Lutris runners.
    let mut alt_wine = Vec::new();
    let mut wine_dirs = vec![tools.join("wine")];
    wine_dirs.push(env.home.join(".local/share/lutris/runners/wine"));
    for dir in wine_dirs {
        for version in sorted_children(&dir) {
            if version.starts_with('.') {
                continue;
            }
            let bin = dir.join(&version).join("bin").join("wine");
            if !bin.exists() {
                continue;
            }
            alt_wine.push(wine_install(&bin, version));
        }
    }

    // Proton: tools/proton, Steam compatibilitytools.d (+ Valve's own with showValveProton).
    let mut proton_dirs = vec![tools.join("proton")];
    for lib in steam_libraries(cfg, env) {
        let lib = PathBuf::from(lib);
        if cfg.show_valve_proton {
            proton_dirs.push(lib.join("steam/steamapps/common"));
            proton_dirs.push(lib.join("steamapps/common"));
        }
        proton_dirs.push(lib.join("root/compatibilitytools.d"));
        proton_dirs.push(lib.join("compatibilitytools.d"));
    }
    let mut proton = Vec::new();
    for dir in &proton_dirs {
        for version in sorted_children(dir) {
            // Only relevant to Lutris; hidden folders are our half-extracted installs.
            if version.starts_with("UMU-Latest") || version.starts_with('.') {
                continue;
            }
            let bin = dir.join(&version).join("proton");
            if bin.exists() {
                proton.push(WineInstallation {
                    bin: bin.to_string_lossy().into_owned(),
                    name: version,
                    kind: "proton".into(),
                    ..Default::default()
                });
            }
        }
    }

    let mut out = proton;
    out.extend(alt_wine);
    if env.system_wine {
        if let Some(w) = default_wine() {
            out.push(w);
        }
    }
    for path in &cfg.custom_wine_paths {
        if path.ends_with("proton") {
            out.push(WineInstallation {
                bin: path.clone(),
                name: format!("Custom Proton - {path}"),
                kind: "proton".into(),
                ..Default::default()
            });
        } else {
            out.push(WineInstallation {
                bin: path.clone(),
                name: format!("Custom Wine - {path}"),
                kind: "wine".into(),
                wineserver: wineserver_of(Path::new(path)),
                ..Default::default()
            });
        }
    }

    let mut seen = HashSet::new();
    out.retain(|w| seen.insert((w.bin.clone(), w.name.clone())));
    out
}

fn wine_install(bin: &Path, name: String) -> WineInstallation {
    let dir = bin.parent().unwrap_or(Path::new(""));
    let lib32 = dir.join("../lib");
    let lib = dir.join("../lib64");
    WineInstallation {
        bin: bin.to_string_lossy().into_owned(),
        name,
        kind: "wine".into(),
        lib: lib.exists().then(|| lib.to_string_lossy().into_owned()),
        lib32: lib32.exists().then(|| lib32.to_string_lossy().into_owned()),
        wineserver: wineserver_of(bin),
    }
}

fn wineserver_of(wine_bin: &Path) -> Option<String> {
    let p = wine_bin.parent()?.join("wineserver");
    p.exists().then(|| p.to_string_lossy().into_owned())
}

/// Child names of `dir`, sorted (readdir order is arbitrary; Heroic shows
/// whatever the filesystem returns, sorted is friendlier and stable).
fn sorted_children(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// `getSteamLibraries`: the system library plus every `path` in the
/// `libraryfolders.vdf` of the default Steam, then of the other Steam
/// installs in the home folder (native, Flatpak, Snap): the default one can
/// be a stale install while Steam runs from another, whose extra libraries
/// (a second drive) only its own list names. Existing folders, each once.
pub fn steam_libraries(cfg: &WineConfig, env: &DetectEnv) -> Vec<String> {
    if !env.steam {
        return Vec::new();
    }
    let system = env.system_steam.iter().map(|p| p.to_string_lossy().into_owned());
    let installs = std::iter::once(PathBuf::from(&cfg.default_steam_path)).chain(
        [
            ".steam/steam",
            ".local/share/Steam",
            ".var/app/com.valvesoftware.Steam/.local/share/Steam",
            "snap/steam/common/.local/share/Steam",
        ]
        .iter()
        .map(|rel| env.home.join(rel))
        .filter(|p| p.join("config").is_dir() || p.join("steam.sh").is_file()),
    );
    let listed = installs.flat_map(|install| {
        fs::read_to_string(install.join("steamapps/libraryfolders.vdf")).ok().and_then(|text| vdf::library_paths(&text)).unwrap_or_default()
    });
    let mut seen = HashSet::new();
    system
        .chain(listed)
        .filter(|p| Path::new(p).exists())
        .filter(|p| seen.insert(fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p))))
        .collect()
}

/// `getDefaultWine`: `wine` on `PATH` and its `--version`.
fn default_wine() -> Option<WineInstallation> {
    let path = std::env::var_os("PATH")?;
    let bin = std::env::split_paths(&path).map(|d| d.join("wine")).find(|p| p.is_file())?;
    let out =
        Command::new(&bin).arg("--version").stdin(Stdio::null()).stderr(Stdio::null()).output().ok().filter(|o| o.status.success())?;
    let name = String::from_utf8_lossy(&out.stdout).lines().next()?.trim().to_owned();
    if name.is_empty() {
        return None;
    }
    Some(WineInstallation {
        bin: bin.to_string_lossy().into_owned(),
        name,
        kind: "wine".into(),
        wineserver: wineserver_of(&bin),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn touch(p: &Path) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, "x").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(p, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn env(home: &Path) -> DetectEnv {
        DetectEnv { home: home.to_path_buf(), system_steam: None, system_wine: false, steam: true }
    }

    // Wine and Proton builds: Linux only.
    #[cfg(unix)]
    #[test]
    fn finds_tools_steam_lutris_and_custom() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("heroic");
        let home = tmp.path().join("home");
        let steam = home.join(".local/share/Steam");
        let lib2 = tmp.path().join("SteamLibrary");
        let data_paths = DataPaths::at(&root);

        touch(&root.join("tools/proton/GE-Proton-latest/proton"));
        fs::create_dir_all(root.join("tools/proton/empty-dir")).unwrap();
        touch(&root.join("tools/wine/Wine-GE-8/bin/wine"));
        touch(&root.join("tools/wine/Wine-GE-8/bin/wineserver"));
        fs::create_dir_all(root.join("tools/wine/Wine-GE-8/lib64")).unwrap();
        touch(&steam.join("compatibilitytools.d/Proton-CachyOS Latest/proton"));
        touch(&steam.join("compatibilitytools.d/UMU-Latest/proton"));
        touch(&lib2.join("compatibilitytools.d/Other/proton"));
        touch(&steam.join("steamapps/common/Proton 9.0/proton"));
        touch(&home.join(".local/share/lutris/runners/wine/lutris-9/bin/wine"));
        fs::write(
            steam.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\"0\"\n{{\n\"path\"\t\"{}\"\n}}\n\"1\"\n{{\n\"path\"\t\"{}\"\n}}\n\"2\"\n{{\n\"path\"\t\"/nonexistent\"\n}}\n}}",
                steam.display(),
                lib2.display()
            ),
        )
        .unwrap();

        let mut cfg = WineConfig {
            default_steam_path: steam.to_string_lossy().into_owned(),
            custom_wine_paths: vec!["/opt/w/bin/wine".into(), "/opt/p/proton".into()],
            ..Default::default()
        };
        let list = list_installations_with(&data_paths, &cfg, &env(&home));
        let names: Vec<_> = list.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "GE-Proton-latest",
                "Proton-CachyOS Latest",
                "Other",
                "Wine-GE-8",
                "lutris-9",
                "Custom Wine - /opt/w/bin/wine",
                "Custom Proton - /opt/p/proton"
            ]
        );
        let ge = &list[0];
        assert_eq!(ge.kind, "proton");
        assert_eq!(ge.bin, root.join("tools/proton/GE-Proton-latest/proton").to_string_lossy());
        let w = &list[3];
        assert_eq!(w.kind, "wine");
        assert!(w.wineserver.as_deref().unwrap().ends_with("bin/wineserver"));
        assert!(w.lib.as_deref().unwrap().ends_with("bin/../lib64"));
        assert!(w.lib32.is_none());

        cfg.show_valve_proton = true;
        let list = list_installations_with(&data_paths, &cfg, &env(&home));
        assert!(list.iter().any(|w| w.name == "Proton 9.0"));
    }

    /// Issue #318: the default Steam folder is a stale install while Steam
    /// runs from its Snap, whose list names a library on a second drive.
    #[cfg(unix)]
    #[test]
    fn builds_in_the_libraries_of_every_steam_install_are_found() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let stale = home.join(".steam/steam");
        let snap = home.join("snap/steam/common/.local/share/Steam");
        let hdd = tmp.path().join("media/HDD/SteamLibrary");
        let leftover = home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam");
        let old_drive = tmp.path().join("old/SteamLibrary");
        let list = |paths: &[&Path]| {
            let entries: String =
                paths.iter().enumerate().map(|(i, p)| format!("\"{i}\"\n{{\n\"path\"\t\"{}\"\n}}\n", p.display())).collect();
            format!("\"libraryfolders\"\n{{\n{entries}}}\n")
        };
        for (install, libs) in [(&stale, vec![stale.as_path()]), (&snap, vec![snap.as_path(), hdd.as_path()])] {
            fs::create_dir_all(install.join("config")).unwrap();
            fs::create_dir_all(install.join("steamapps")).unwrap();
            fs::write(install.join("steamapps/libraryfolders.vdf"), list(&libs)).unwrap();
        }
        // Not a Steam install (no `config`): its old list is not read.
        fs::create_dir_all(leftover.join("steamapps")).unwrap();
        fs::write(leftover.join("steamapps/libraryfolders.vdf"), list(&[old_drive.as_path()])).unwrap();
        touch(&snap.join("compatibilitytools.d/GE-Proton10-1/proton"));
        touch(&hdd.join("steamapps/common/Proton 9.0/proton"));
        touch(&old_drive.join("steamapps/common/Proton 8.0/proton"));

        let cfg = WineConfig { default_steam_path: stale.to_string_lossy().into_owned(), show_valve_proton: true, ..Default::default() };
        let libs = steam_libraries(&cfg, &env(&home));
        let expected: Vec<String> = [&stale, &snap, &hdd].iter().map(|p| p.to_string_lossy().into_owned()).collect();
        assert_eq!(libs, expected);
        let data_paths = DataPaths::at(tmp.path().join("heronite"));
        let names: Vec<_> = list_installations_with(&data_paths, &cfg, &env(&home)).into_iter().map(|w| w.name).collect();
        assert_eq!(names, ["GE-Proton10-1", "Proton 9.0"]);
    }

    #[test]
    fn empty_folders_give_empty_list() {
        let tmp = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(tmp.path().join("nothing"));
        let cfg = WineConfig { default_steam_path: "/nonexistent-steam".into(), ..Default::default() };
        assert!(list_installations_with(&data_paths, &cfg, &env(tmp.path())).is_empty());
    }
}
