//! Emulator detection (PATH, Flatpak, libretro cores): port of
//! `emulation/detect.ts` plus `addDetectedEmulators` of `emulation/index.ts`.
//! Windows has no Flatpak; its emulators are `.exe` files and RetroArch's
//! cores `.dll` files.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::Result;
use heronite_data::DataPaths;
use heronite_launch::emulators::{UserEmulator, UserEmulatorProfile};
use heronite_launch::probe::{Probe, SystemProbe};

use crate::catalog::{emulator_definition, emulator_definitions, EmulatorDefinition};
use crate::pathutil::{dirname, has_dir, join, join_all, unique};
use crate::store::{self, upsert_emulator_in};
use crate::types::{DetectedEmulator, DetectedEmulatorProfile, UpsertUserEmulatorArgs};

/// What detection needs from the machine (replaceable in tests).
pub trait DetectEnv {
    /// Absolute path of `name` on `$PATH`.
    fn which(&self, name: &str) -> Option<String>;
    /// `flatpak info <id>` succeeds.
    fn flatpak_installed(&self, id: &str) -> bool;
    fn exists(&self, path: &str) -> bool;
    /// File names in a directory (empty when unreadable).
    fn read_dir(&self, dir: &str) -> Vec<String>;
    fn home(&self) -> String;
    /// A variable of the environment, `None` when unset or empty
    /// (`ProgramFiles`, `LOCALAPPDATA`... on Windows).
    fn var(&self, _key: &str) -> Option<String> {
        None
    }
    /// Windows: the roots of the fixed and removable drives (`D:\`), none
    /// on other systems or when unknown.
    fn fixed_drives(&self) -> Vec<String> {
        vec![]
    }
}

/// The real machine (read-only; `flatpak info` is the only process it spawns).
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemEnv;

impl DetectEnv for SystemEnv {
    fn which(&self, name: &str) -> Option<String> {
        let found = SystemProbe.which(name)?;
        Some(scoop_shim_target(&found).unwrap_or(found))
    }

    fn flatpak_installed(&self, id: &str) -> bool {
        if cfg!(windows) {
            return false;
        }
        let Ok(mut child) =
            Command::new("flatpak").args(["info", id]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
        else {
            return false;
        };
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return status.success(),
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
            }
        }
    }

    fn exists(&self, path: &str) -> bool {
        Path::new(path).exists()
    }

    fn read_dir(&self, dir: &str) -> Vec<String> {
        std::fs::read_dir(dir)
            .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default()
    }

    fn home(&self) -> String {
        SystemProbe.home()
    }

    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|v| !v.is_empty())
    }

    fn fixed_drives(&self) -> Vec<String> {
        heronite_platform::fs::fixed_drives().iter().map(|d| d.to_string_lossy().into_owned()).collect()
    }
}

/// The program a Scoop shim (`scoop\shims\<name>.exe`) starts: the `path`
/// line of the `<name>.shim` next to it. Its folder is the emulator's, where
/// its cores and portable files are.
fn scoop_shim_target(exe: &str) -> Option<String> {
    let shim = Path::new(exe).with_extension("shim");
    let text = std::fs::read_to_string(shim).ok()?;
    let line = text.lines().find_map(|l| l.trim().strip_prefix("path"))?;
    let target = line.trim_start().strip_prefix('=')?.trim().trim_matches('"');
    Path::new(target).is_file().then(|| target.to_string())
}

/// Where Scoop keeps its apps: the user's (`%SCOOP%`, else `~\scoop`) and
/// the global one (`%SCOOP_GLOBAL%`, else `%ProgramData%\scoop`).
pub(crate) fn scoop_apps(env: &dyn DetectEnv) -> Vec<String> {
    let user = env.var("SCOOP").unwrap_or_else(|| join(&env.home(), "scoop"));
    let global = env.var("SCOOP_GLOBAL").or_else(|| env.var("ProgramData").map(|d| join(&d, "scoop")));
    unique([Some(join(&user, "apps")), global.map(|g| join(&g, "apps"))])
}

/// The system drive's root (`C:\`).
pub(crate) fn system_drive(env: &dyn DetectEnv) -> String {
    format!("{}\\", env.var("SystemDrive").unwrap_or_else(|| "C:".into()))
}

/// Windows: the drives to look for emulators and ROMs on: the system drive
/// first, then the other fixed and removable drives, each once. A root that
/// cannot be read (an empty card reader) is left out, so every drive costs
/// one check here and a few folder names where the caller looks.
pub(crate) fn drives(env: &dyn DetectEnv) -> Vec<String> {
    let mut all = vec![system_drive(env)];
    for drive in env.fixed_drives() {
        if !all.iter().any(|d| d.eq_ignore_ascii_case(&drive)) && env.exists(&drive) {
            all.push(drive);
        }
    }
    all
}

/// RetroArch's cores: `.so` files on Linux, `.dll` on Windows. A core named
/// with the other system's extension (a backup from Linux) gets this one's.
fn core_file_name(core: &str) -> String {
    let (ext, other) = if cfg!(windows) { (".dll", ".so") } else { (".so", ".dll") };
    if core.ends_with(ext) {
        return core.to_string();
    }
    format!("{}{ext}", core.strip_suffix(other).filter(|_| cfg!(windows)).unwrap_or(core))
}

/// `retroArchCoreDirs`.
pub fn retroarch_core_dirs(env: &dyn DetectEnv, install_dir: Option<&str>) -> Vec<String> {
    let home = env.home();
    if cfg!(windows) {
        // RetroArch keeps its cores next to it; the installer's folder and
        // Scoop's when the executable is elsewhere (a shortcut, `$PATH`).
        let mut dirs = vec![install_dir.map(|d| join(d, "cores"))];
        dirs.extend(drives(env).iter().map(|drive| Some(join_all(drive, &["RetroArch-Win64", "cores"]))));
        dirs.extend(scoop_apps(env).iter().map(|apps| Some(join_all(apps, &["retroarch", "current", "cores"]))));
        dirs.push(env.var("APPDATA").map(|d| join_all(&d, &["RetroArch", "cores"])));
        return unique(dirs);
    }
    unique([
        install_dir.map(|d| join(d, "cores")),
        Some(join(&home, ".config/retroarch/cores")),
        Some(join(&home, ".local/share/flatpak/exports/share/libretro/cores")),
        Some("/usr/lib/libretro".into()),
        Some("/usr/lib64/libretro".into()),
        Some("/usr/lib/x86_64-linux-gnu/libretro".into()),
        Some("/usr/share/libretro/cores".into()),
        Some("/var/lib/flatpak/app/org.libretro.RetroArch/current/active/files/lib/libretro".into()),
    ])
}

/// `findCorePath`.
pub fn find_core_path(env: &dyn DetectEnv, core: &str, install_dir: Option<&str>) -> Option<String> {
    let file = core_file_name(core);
    retroarch_core_dirs(env, install_dir).into_iter().map(|d| join(&d, &file)).find(|full| env.exists(full))
}

/// `existingProfiles` / `attachDetectedCores`: the catalog profiles usable
/// with this install (core-based ones only when the core file exists).
pub fn existing_profiles(env: &dyn DetectEnv, definition_id: &str, executable: &str, launch_kind: &str) -> Vec<DetectedEmulatorProfile> {
    let Some(def) = emulator_definition(definition_id) else {
        return vec![];
    };
    let install_dir = (launch_kind != "flatpak").then(|| dirname(executable));
    let mut out = Vec::new();
    for p in &def.profiles {
        let core_path = p.core.as_deref().and_then(|c| find_core_path(env, c, install_dir.as_deref()));
        if p.core.is_some() && core_path.is_none() {
            continue;
        }
        out.push(DetectedEmulatorProfile {
            id: p.id.clone(),
            name: p.name.clone(),
            platform_name: p.platform_name.clone(),
            core_path,
            ..Default::default()
        });
    }
    out
}

/// The profiles a new emulator of `definition_id` starts with: the catalog's,
/// with RetroArch cores resolved next to `executable`. When the definition
/// has core-based profiles and some core is found, only those with a core are
/// kept; when none is found they all stay (the user sets the core paths).
pub fn profiles_for_definition(env: &dyn DetectEnv, definition_id: &str, executable: &str, launch_kind: &str) -> Vec<UserEmulatorProfile> {
    let mut profiles = store::default_profiles(definition_id, None);
    let Some(def) = emulator_definition(definition_id) else {
        return profiles;
    };
    let install_dir = (launch_kind != "flatpak" && has_dir(executable)).then(|| dirname(executable));
    let mut any_core = false;
    for p in &mut profiles {
        let core =
            p.definition_profile_id.as_deref().and_then(|pid| def.profiles.iter().find(|d| d.id == pid)).and_then(|d| d.core.as_deref());
        if let Some(core) = core {
            p.core_path = find_core_path(env, core, install_dir.as_deref());
            any_core |= p.core_path.is_some();
        }
    }
    if any_core {
        profiles.retain(|p| {
            let has_core = p
                .definition_profile_id
                .as_deref()
                .and_then(|pid| def.profiles.iter().find(|d| d.id == pid))
                .is_some_and(|d| d.core.is_some());
            !has_core || p.core_path.is_some()
        });
    }
    profiles
}

/// Windows: an install of `def` where it usually is
/// (`Program Files\PCSX2\pcsx2-qt.exe`, Scoop's
/// `apps\pcsx2\current\pcsx2-qt.exe`), with where it was found.
fn windows_install(env: &dyn DetectEnv, def: &EmulatorDefinition) -> Option<(String, String)> {
    let installs = windows_install_bases(env).into_iter().map(|(base, label)| (base, None, label));
    let scoop = scoop_apps(env).into_iter().map(|apps| (apps, Some("current"), "Scoop".to_string()));
    for (base, sub, label) in installs.chain(scoop) {
        for folder in &def.install_folders {
            // Scoop names its apps in lower case.
            let folder = if sub.is_some() { folder.to_lowercase() } else { folder.clone() };
            for bin in &def.binaries {
                let path = join_all(&base, &[&folder, sub.unwrap_or(""), bin]);
                if env.exists(&path) {
                    return Some((path, label));
                }
            }
        }
    }
    None
}

/// Windows: where programs are installed, with the name shown for it:
/// Program Files, Program Files (x86), `%LOCALAPPDATA%\Programs`,
/// `%LOCALAPPDATA%`, `%APPDATA%` and the root of each drive ([`drives`]).
pub(crate) fn windows_install_bases(env: &dyn DetectEnv) -> Vec<(String, String)> {
    let local = env.var("LOCALAPPDATA");
    let mut bases = vec![
        (env.var("ProgramFiles"), "Program Files".to_string()),
        (env.var("ProgramFiles(x86)"), "Program Files (x86)".to_string()),
        (local.as_deref().map(|d| join(d, "Programs")), r"AppData\Local\Programs".to_string()),
        (local, r"AppData\Local".to_string()),
        (env.var("APPDATA"), r"AppData\Roaming".to_string()),
    ];
    bases.extend(drives(env).into_iter().map(|drive| (Some(drive.clone()), drive)));
    bases.into_iter().filter_map(|(base, label)| base.map(|b| (b, label))).collect()
}

/// `detectEmulators`: `$PATH`, then Flatpak on Linux, the usual install
/// folders on Windows.
pub fn detect_emulators(env: &dyn DetectEnv, existing: &[UserEmulator]) -> Vec<DetectedEmulator> {
    let mut found = Vec::new();
    for def in emulator_definitions() {
        if def.id == "custom" {
            continue;
        }
        if let Some(d) = detect_one(env, def, existing) {
            found.push(d);
        }
    }
    found
}

fn detect_one(env: &dyn DetectEnv, def: &EmulatorDefinition, existing: &[UserEmulator]) -> Option<DetectedEmulator> {
    let mut executable: Option<String> = def.binaries.iter().find_map(|b| env.which(b));
    let mut source = "PATH".to_string();
    let mut launch_kind = "native";
    let mut flatpak_id: Option<String> = None;
    if executable.is_none() && cfg!(windows) {
        if let Some((path, found_in)) = windows_install(env, def) {
            executable = Some(path);
            source = found_in;
        }
    }
    if executable.is_none() && !cfg!(windows) {
        if let Some(id) = def.flatpak_id.as_deref() {
            if env.flatpak_installed(id) {
                executable = Some("flatpak".into());
                launch_kind = "flatpak";
                flatpak_id = Some(id.to_string());
            }
        }
    }
    let executable = executable?;
    let profiles = existing_profiles(env, &def.id, &executable, launch_kind);
    if profiles.is_empty() {
        return None;
    }
    let already_added = existing.iter().any(|e| {
        e.definition_id == def.id
            && e.executable == executable
            && e.flatpak_id.clone().unwrap_or_default() == flatpak_id.clone().unwrap_or_default()
    });
    Some(DetectedEmulator {
        definition_id: def.id.clone(),
        name: def.name.clone(),
        executable,
        launch_kind: launch_kind.into(),
        flatpak_id,
        profiles,
        already_added,
        source: if launch_kind == "flatpak" { "Flatpak".into() } else { source },
    })
}

/// `listCorePaths`: every `*_libretro.so` (`.dll` on Windows) in the known
/// core folders.
pub fn list_core_paths(env: &dyn DetectEnv, install_dir: Option<&str>) -> Vec<String> {
    let suffix = core_file_name("_libretro");
    let mut files = Vec::new();
    for dir in retroarch_core_dirs(env, install_dir) {
        if !env.exists(&dir) {
            continue;
        }
        let mut names = env.read_dir(&dir);
        names.sort();
        for name in names {
            if name.ends_with(&suffix) {
                files.push(join(&dir, &name));
            }
        }
    }
    files
}

/// `addDetectedEmulators`: registers every detected emulator that is not
/// yet known (and refreshes the core paths of known ones). Returns the full
/// emulator list.
pub fn add_detected_emulators(paths: &DataPaths, env: &dyn DetectEnv) -> Result<Vec<UserEmulator>> {
    let current = store::list_user_emulators(paths)?;
    let detected = detect_emulators(env, &current);
    heronite_data::json_store::update::<heronite_launch::emulators::EmulatorsFile, _>(&paths.emulators(), |file| {
        apply_detected(&mut file.emulators, &current, &detected);
        Ok(())
    })?;
    store::list_user_emulators(paths)
}

/// The pure part of `addDetectedEmulators`.
pub fn apply_detected(emulators: &mut Vec<UserEmulator>, snapshot: &[UserEmulator], detected: &[DetectedEmulator]) {
    for item in detected {
        let existing = snapshot.iter().find(|e| {
            e.definition_id == item.definition_id
                && e.executable == item.executable
                && e.flatpak_id.clone().unwrap_or_default() == item.flatpak_id.clone().unwrap_or_default()
        });
        let def = emulator_definition(&item.definition_id);
        let incoming: Vec<UserEmulatorProfile> = item
            .profiles
            .iter()
            .map(|p| {
                let cat = def.and_then(|d| d.profiles.iter().find(|c| c.id == p.id));
                UserEmulatorProfile {
                    id: p.id.clone(),
                    definition_profile_id: Some(p.id.clone()),
                    name: p.name.clone(),
                    platform_id: p.platform_id.clone().or_else(|| cat.map(|c| c.platform_id.clone())),
                    platform_name: Some(p.platform_name.clone()),
                    arguments: p.arguments.clone().or_else(|| cat.map(|c| c.arguments.clone())).unwrap_or_else(|| "{ImagePath}".into()),
                    extensions: p.extensions.clone().or_else(|| cat.map(|c| c.extensions.clone())).unwrap_or_default(),
                    core_path: p.core_path.clone(),
                    folder_indicator: cat.and_then(|c| c.folder_indicator.clone()),
                    playlist_extensions: cat.and_then(|c| c.playlist_extensions.clone()),
                    working_dir: cat.and_then(|c| c.working_dir.clone()),
                    extra: Default::default(),
                }
            })
            .collect();
        let profiles = match existing {
            Some(ex) => {
                let mut merged: Vec<UserEmulatorProfile> = ex
                    .profiles
                    .iter()
                    .map(|profile| match incoming.iter().find(|i| i.definition_profile_id == profile.definition_profile_id) {
                        Some(fresh) => UserEmulatorProfile { core_path: fresh.core_path.clone(), ..profile.clone() },
                        None => profile.clone(),
                    })
                    .collect();
                merged.extend(
                    incoming.iter().filter(|i| !ex.profiles.iter().any(|p| p.definition_profile_id == i.definition_profile_id)).cloned(),
                );
                merged
            }
            None => incoming,
        };
        upsert_emulator_in(
            emulators,
            UpsertUserEmulatorArgs {
                id: existing.map(|e| e.id.clone()),
                definition_id: item.definition_id.clone(),
                name: existing.map(|e| e.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| item.name.clone()),
                executable: item.executable.clone(),
                install_dir: None,
                launch_kind: item.launch_kind.clone(),
                flatpak_id: item.flatpak_id.clone(),
                playnite_id: None,
                profiles: Some(profiles),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    #[derive(Default)]
    struct Fake {
        bins: HashMap<String, String>,
        flatpaks: HashSet<String>,
        files: HashSet<String>,
        dirs: HashMap<String, Vec<String>>,
        vars: HashMap<String, String>,
        fixed: Vec<String>,
    }

    impl DetectEnv for Fake {
        fn which(&self, name: &str) -> Option<String> {
            self.bins.get(name).cloned()
        }
        fn flatpak_installed(&self, id: &str) -> bool {
            self.flatpaks.contains(id)
        }
        fn exists(&self, path: &str) -> bool {
            self.files.contains(path) || self.dirs.contains_key(path)
        }
        fn read_dir(&self, dir: &str) -> Vec<String> {
            self.dirs.get(dir).cloned().unwrap_or_default()
        }
        fn home(&self) -> String {
            "/home/u".into()
        }
        fn var(&self, key: &str) -> Option<String> {
            self.vars.get(key).cloned()
        }
        fn fixed_drives(&self) -> Vec<String> {
            self.fixed.clone()
        }
    }

    /// `rest` under the root of this system (`/opt/ra`, `C:\opt\ra`).
    fn at(rest: &str) -> String {
        join(if cfg!(windows) { "C:\\" } else { "/" }, rest)
    }

    /// An executable name of this system (`retroarch`, `retroarch.exe`).
    fn exe(name: &str) -> String {
        if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        }
    }

    /// A RetroArch core file of this system.
    fn core(stem: &str) -> String {
        core_file_name(&format!("{stem}_libretro"))
    }

    // Flatpak and the system's libretro folders are Linux's.
    #[cfg(unix)]
    #[test]
    fn detects_path_flatpak_and_cores() {
        let mut env = Fake::default();
        env.bins.insert("pcsx2-qt".into(), "/usr/bin/pcsx2-qt".into());
        env.flatpaks.insert("org.libretro.RetroArch".into());
        env.flatpaks.insert("org.DolphinEmu.dolphin-emu".into());
        // RetroArch flatpak core: only SNES + PS1 cores exist.
        env.files.insert("/var/lib/flatpak/app/org.libretro.RetroArch/current/active/files/lib/libretro/snes9x_libretro.so".into());
        env.files.insert("/usr/lib/libretro/pcsx_rearmed_libretro.so".into());
        let existing = vec![UserEmulator { definition_id: "pcsx2".into(), executable: "/usr/bin/pcsx2-qt".into(), ..Default::default() }];
        let found = detect_emulators(&env, &existing);
        let ids: Vec<_> = found.iter().map(|d| d.definition_id.as_str()).collect();
        assert_eq!(ids, ["dolphin", "pcsx2", "retroarch"]);
        let pcsx2 = &found[1];
        assert!(pcsx2.already_added);
        assert_eq!(pcsx2.launch_kind, "native");
        let dolphin = &found[0];
        assert_eq!(dolphin.executable, "flatpak");
        assert_eq!(dolphin.flatpak_id.as_deref(), Some("org.DolphinEmu.dolphin-emu"));
        assert!(!dolphin.already_added);
        let ra = &found[2];
        let profile_ids: Vec<_> = ra.profiles.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(profile_ids, ["snes", "ps1"]);
        assert_eq!(
            ra.profiles[0].core_path.as_deref(),
            Some("/var/lib/flatpak/app/org.libretro.RetroArch/current/active/files/lib/libretro/snes9x_libretro.so")
        );
    }

    #[test]
    fn new_emulator_profiles_follow_the_cores_found() {
        let mut env = Fake::default();
        let retroarch = at("opt/ra/retroarch");
        let all = profiles_for_definition(&env, "retroarch", &retroarch, "native");
        assert_eq!(all.len(), 20);
        assert!(all.iter().all(|p| p.core_path.is_none()));
        let snes = at(&format!("opt/ra/cores/{}", core("snes9x")));
        env.files.insert(snes.clone());
        let some = profiles_for_definition(&env, "retroarch", &retroarch, "native");
        assert_eq!(some.len(), 1);
        assert_eq!(some[0].core_path.as_deref(), Some(snes.as_str()));
        let pcsx2 = profiles_for_definition(&env, "pcsx2", &at("x/pcsx2"), "native");
        assert_eq!(pcsx2.len(), 1);
        assert_eq!(profiles_for_definition(&env, "custom", "", "native")[0].name, "Custom");
    }

    #[test]
    fn retroarch_without_cores_is_not_detected() {
        let mut env = Fake::default();
        env.bins.insert(exe("retroarch"), at(&format!("emu/{}", exe("retroarch"))));
        assert!(detect_emulators(&env, &[]).is_empty());
        env.files.insert(at(&format!("emu/cores/{}", core("nestopia"))));
        let found = detect_emulators(&env, &[]);
        assert_eq!(found[0].profiles[0].id, "nes");
        assert_eq!((found[0].launch_kind.as_str(), found[0].source.as_str()), ("native", "PATH"));
    }

    #[test]
    fn list_cores_filters_names() {
        let mut env = Fake::default();
        let cores = at("opt/ra/cores");
        let other = if cfg!(windows) { "d_libretro.so" } else { "d_libretro.dll" };
        env.dirs.insert(cores.clone(), vec![core("b"), core("a"), "readme.txt".into(), "c_libretro.info".into(), other.into()]);
        let found = list_core_paths(&env, Some(&at("opt/ra")));
        assert_eq!(found, [join(&cores, &core("a")), join(&cores, &core("b"))]);
    }

    #[test]
    fn add_detected_registers_new_and_refreshes_existing() {
        let d = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(d.path());
        let mut env = Fake::default();
        env.bins.insert(exe("pcsx2-qt"), at(&format!("usr/bin/{}", exe("pcsx2-qt"))));
        env.bins.insert(exe("retroarch"), at(&format!("opt/ra/{}", exe("retroarch"))));
        let snes = at(&format!("opt/ra/cores/{}", core("snes9x")));
        env.files.insert(snes.clone());
        let list = add_detected_emulators(&paths, &env).unwrap();
        assert_eq!(list.len(), 2);
        let ra = list.iter().find(|e| e.definition_id == "retroarch").unwrap();
        assert_eq!(ra.profiles.len(), 1);
        assert_eq!(ra.profiles[0].id, "snes");
        assert_eq!(ra.profiles[0].core_path.as_deref(), Some(snes.as_str()));
        // Core moves + a new core appears: existing profile is refreshed, new
        // one appended, user edits survive, no duplicates.
        let mut tweak = ra.clone();
        tweak.profiles[0].arguments = "-L {CorePath} --custom {ImagePath}".into();
        tweak.name = "My RA".into();
        heronite_data::json_store::update::<heronite_launch::emulators::EmulatorsFile, _>(&paths.emulators(), |f| {
            let i = f.emulators.iter().position(|e| e.id == tweak.id).unwrap();
            f.emulators[i] = tweak.clone();
            Ok(())
        })
        .unwrap();
        env.files.clear();
        env.files.insert(snes);
        env.files.insert(at(&format!("opt/ra/cores/{}", core("nestopia"))));
        let list = add_detected_emulators(&paths, &env).unwrap();
        assert_eq!(list.len(), 2);
        let ra = list.iter().find(|e| e.definition_id == "retroarch").unwrap();
        assert_eq!(ra.name, "My RA");
        let ids: Vec<_> = ra.profiles.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["snes", "nes"]);
        assert!(ra.profiles[0].arguments.contains("--custom"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_has_dll_cores_and_no_flatpak() {
        assert_eq!(core_file_name("snes9x_libretro"), "snes9x_libretro.dll");
        // A core path from Linux (a restored backup) names this system's file.
        assert_eq!(core_file_name("snes9x_libretro.so"), "snes9x_libretro.dll");
        assert_eq!(core_file_name("snes9x_libretro.dll"), "snes9x_libretro.dll");
        let mut env = Fake::default();
        env.flatpaks.insert("org.DolphinEmu.dolphin-emu".into());
        env.vars.insert("SystemDrive".into(), "D:".into());
        env.vars.insert("APPDATA".into(), r"C:\Users\u\AppData\Roaming".into());
        env.vars.insert("SCOOP".into(), r"C:\Scoop".into());
        assert!(detect_emulators(&env, &[]).is_empty(), "no Flatpak on Windows");
        assert_eq!(
            retroarch_core_dirs(&env, Some(r"C:\Emu\RetroArch")),
            [
                r"C:\Emu\RetroArch\cores",
                r"D:\RetroArch-Win64\cores",
                r"C:\Scoop\apps\retroarch\current\cores",
                r"C:\Users\u\AppData\Roaming\RetroArch\cores",
            ]
        );
        // RetroArch from the installer, found through its cores folder.
        env.bins.insert("retroarch.exe".into(), r"D:\RetroArch-Win64\retroarch.exe".into());
        env.files.insert(r"D:\RetroArch-Win64\cores\snes9x_libretro.dll".into());
        let found = detect_emulators(&env, &[]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].profiles[0].core_path.as_deref(), Some(r"D:\RetroArch-Win64\cores\snes9x_libretro.dll"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_installs_are_found_in_their_usual_folders() {
        let mut env = Fake::default();
        env.vars.insert("ProgramFiles".into(), r"C:\Program Files".into());
        env.vars.insert("LOCALAPPDATA".into(), r"C:\Users\u\AppData\Local".into());
        env.vars.insert("SCOOP".into(), r"D:\Scoop".into());
        env.files.insert(r"C:\Program Files\PCSX2\pcsx2-qt.exe".into());
        env.files.insert(r"C:\Users\u\AppData\Local\Programs\Ryujinx\Ryujinx.exe".into());
        env.files.insert(r"D:\Scoop\apps\duckstation\current\duckstation-qt-x64-ReleaseLTCG.exe".into());
        // `$PATH` comes first.
        env.bins.insert("Cemu.exe".into(), r"C:\Tools\Cemu\Cemu.exe".into());
        env.files.insert(r"C:\Program Files\Cemu\Cemu.exe".into());
        let found: Vec<(String, String)> = detect_emulators(&env, &[]).into_iter().map(|d| (d.executable, d.source)).collect();
        let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(
            found,
            [
                pair(r"C:\Program Files\PCSX2\pcsx2-qt.exe", "Program Files"),
                pair(r"D:\Scoop\apps\duckstation\current\duckstation-qt-x64-ReleaseLTCG.exe", "Scoop"),
                pair(r"C:\Tools\Cemu\Cemu.exe", "PATH"),
                pair(r"C:\Users\u\AppData\Local\Programs\Ryujinx\Ryujinx.exe", r"AppData\Local\Programs"),
            ]
        );
    }

    #[test]
    fn the_system_drive_leads_and_the_other_readable_drives_follow() {
        let mut env = Fake::default();
        assert_eq!(drives(&env), [r"C:\"]);
        env.vars.insert("SystemDrive".into(), "D:".into());
        env.fixed = [r"C:\", r"d:\", r"E:\", r"F:\"].map(String::from).to_vec();
        // E: reads, F: is a card reader with nothing in it.
        env.dirs.insert(r"C:\".into(), vec![]);
        env.dirs.insert(r"E:\".into(), vec![]);
        assert_eq!(drives(&env), [r"D:\", r"C:\", r"E:\"]);
    }

    #[cfg(windows)]
    #[test]
    fn windows_installs_and_cores_are_found_on_every_drive() {
        let mut env = Fake { fixed: [r"C:\", r"D:\", r"E:\"].map(String::from).to_vec(), ..Default::default() };
        for root in [r"C:\", r"D:\"] {
            env.dirs.insert(root.into(), vec![]);
        }
        env.files.insert(r"D:\PCSX2\pcsx2-qt.exe".into());
        env.files.insert(r"C:\DuckStation\duckstation-qt.exe".into());
        env.files.insert(r"D:\DuckStation\duckstation-qt.exe".into());
        // Nothing is looked for on a drive that cannot be read.
        env.files.insert(r"E:\Cemu\Cemu.exe".into());
        env.files.insert(r"D:\RetroArch-Win64\retroarch.exe".into());
        env.files.insert(r"D:\RetroArch-Win64\cores\snes9x_libretro.dll".into());
        let found: Vec<(String, String)> = detect_emulators(&env, &[]).into_iter().map(|d| (d.executable, d.source)).collect();
        let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(
            found,
            [
                pair(r"D:\PCSX2\pcsx2-qt.exe", r"D:\"),
                pair(r"C:\DuckStation\duckstation-qt.exe", r"C:\"),
                pair(r"D:\RetroArch-Win64\retroarch.exe", r"D:\"),
            ]
        );
        assert_eq!(retroarch_core_dirs(&env, None)[..2], [r"C:\RetroArch-Win64\cores", r"D:\RetroArch-Win64\cores"]);
        assert_eq!(find_core_path(&env, "snes9x_libretro", None).as_deref(), Some(r"D:\RetroArch-Win64\cores\snes9x_libretro.dll"));
    }

    #[cfg(windows)]
    #[test]
    fn a_scoop_shim_leads_to_its_program() {
        let d = tempfile::tempdir().unwrap();
        let program = d.path().join(r"apps\pcsx2\current\pcsx2-qt.exe");
        std::fs::create_dir_all(program.parent().unwrap()).unwrap();
        std::fs::write(&program, "x").unwrap();
        let shims = d.path().join("shims");
        std::fs::create_dir_all(&shims).unwrap();
        std::fs::write(shims.join("pcsx2-qt.exe"), "x").unwrap();
        std::fs::write(shims.join("pcsx2-qt.shim"), format!("path = \"{}\"\r\n", program.display())).unwrap();
        let shim = shims.join("pcsx2-qt.exe").to_string_lossy().into_owned();
        assert_eq!(scoop_shim_target(&shim).as_deref(), program.to_str());
        // Not a shim (or one whose program is gone): the file itself.
        assert_eq!(scoop_shim_target(program.to_str().unwrap()), None);
        std::fs::remove_file(&program).unwrap();
        assert_eq!(scoop_shim_target(&shim), None);
    }
}
