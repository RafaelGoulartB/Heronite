//! Emulator detection (PATH, Flatpak, libretro cores): port of
//! `emulation/detect.ts` plus `addDetectedEmulators` of `emulation/index.ts`.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::Result;
use heronite_data::DataPaths;
use heronite_launch::emulators::{UserEmulator, UserEmulatorProfile};
use heronite_launch::probe::{Probe, SystemProbe};

use crate::catalog::{emulator_definition, emulator_definitions, EmulatorDefinition};
use crate::pathutil::{dirname, join, unique};
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
}

/// The real machine (read-only; `flatpak info` is the only process it spawns).
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemEnv;

impl DetectEnv for SystemEnv {
    fn which(&self, name: &str) -> Option<String> {
        SystemProbe.which(name)
    }

    fn flatpak_installed(&self, id: &str) -> bool {
        let Ok(mut child) = Command::new("flatpak")
            .args(["info", id])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return false;
        };
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return status.success(),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20))
                }
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
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn home(&self) -> String {
        SystemProbe.home()
    }
}

fn core_file_name(core: &str) -> String {
    if core.ends_with(".so") {
        core.to_string()
    } else {
        format!("{core}.so")
    }
}

/// `retroArchCoreDirs`.
pub fn retroarch_core_dirs(env: &dyn DetectEnv, install_dir: Option<&str>) -> Vec<String> {
    let home = env.home();
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
    retroarch_core_dirs(env, install_dir)
        .into_iter()
        .map(|d| join(&d, &file))
        .find(|full| env.exists(full))
}

/// `existingProfiles` / `attachDetectedCores`: the catalog profiles usable
/// with this install (core-based ones only when the core file exists).
pub fn existing_profiles(
    env: &dyn DetectEnv,
    definition_id: &str,
    executable: &str,
    launch_kind: &str,
) -> Vec<DetectedEmulatorProfile> {
    let Some(def) = emulator_definition(definition_id) else {
        return vec![];
    };
    let install_dir = (launch_kind != "flatpak").then(|| dirname(executable));
    let mut out = Vec::new();
    for p in &def.profiles {
        let core_path = p
            .core
            .as_deref()
            .and_then(|c| find_core_path(env, c, install_dir.as_deref()));
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
pub fn profiles_for_definition(
    env: &dyn DetectEnv,
    definition_id: &str,
    executable: &str,
    launch_kind: &str,
) -> Vec<UserEmulatorProfile> {
    let mut profiles = store::default_profiles(definition_id, None);
    let Some(def) = emulator_definition(definition_id) else {
        return profiles;
    };
    let install_dir = (launch_kind != "flatpak" && executable.contains('/')).then(|| dirname(executable));
    let mut any_core = false;
    for p in &mut profiles {
        let core = p
            .definition_profile_id
            .as_deref()
            .and_then(|pid| def.profiles.iter().find(|d| d.id == pid))
            .and_then(|d| d.core.as_deref());
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

/// `detectEmulators`.
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

fn detect_one(
    env: &dyn DetectEnv,
    def: &EmulatorDefinition,
    existing: &[UserEmulator],
) -> Option<DetectedEmulator> {
    let mut executable: Option<String> = def.binaries.iter().find_map(|b| env.which(b));
    let mut launch_kind = "native";
    let mut flatpak_id: Option<String> = None;
    if executable.is_none() {
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
        source: if launch_kind == "flatpak" { "Flatpak".into() } else { "PATH".into() },
    })
}

/// `listCorePaths`: every `*_libretro.so` in the known core folders.
pub fn list_core_paths(env: &dyn DetectEnv, install_dir: Option<&str>) -> Vec<String> {
    let mut files = Vec::new();
    for dir in retroarch_core_dirs(env, install_dir) {
        if !env.exists(&dir) {
            continue;
        }
        let mut names = env.read_dir(&dir);
        names.sort();
        for name in names {
            if name.ends_with("_libretro.so") {
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
    heronite_data::json_store::update::<heronite_launch::emulators::EmulatorsFile, _>(
        &paths.emulators(),
        |file| {
            apply_detected(&mut file.emulators, &current, &detected);
            Ok(())
        },
    )?;
    store::list_user_emulators(paths)
}

/// The pure part of `addDetectedEmulators`.
pub fn apply_detected(
    emulators: &mut Vec<UserEmulator>,
    snapshot: &[UserEmulator],
    detected: &[DetectedEmulator],
) {
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
                    arguments: p
                        .arguments
                        .clone()
                        .or_else(|| cat.map(|c| c.arguments.clone()))
                        .unwrap_or_else(|| "{ImagePath}".into()),
                    extensions: p
                        .extensions
                        .clone()
                        .or_else(|| cat.map(|c| c.extensions.clone()))
                        .unwrap_or_default(),
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
                    .map(|profile| {
                        match incoming
                            .iter()
                            .find(|i| i.definition_profile_id == profile.definition_profile_id)
                        {
                            Some(fresh) => UserEmulatorProfile {
                                core_path: fresh.core_path.clone(),
                                ..profile.clone()
                            },
                            None => profile.clone(),
                        }
                    })
                    .collect();
                merged.extend(
                    incoming
                        .iter()
                        .filter(|i| {
                            !ex.profiles
                                .iter()
                                .any(|p| p.definition_profile_id == i.definition_profile_id)
                        })
                        .cloned(),
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
                name: existing
                    .map(|e| e.name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| item.name.clone()),
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
    }

    #[test]
    fn detects_path_flatpak_and_cores() {
        let mut env = Fake::default();
        env.bins.insert("pcsx2-qt".into(), "/usr/bin/pcsx2-qt".into());
        env.flatpaks.insert("org.libretro.RetroArch".into());
        env.flatpaks.insert("org.DolphinEmu.dolphin-emu".into());
        // RetroArch flatpak core: only SNES + PS1 cores exist.
        env.files.insert("/var/lib/flatpak/app/org.libretro.RetroArch/current/active/files/lib/libretro/snes9x_libretro.so".into());
        env.files.insert("/usr/lib/libretro/pcsx_rearmed_libretro.so".into());
        let existing = vec![UserEmulator {
            definition_id: "pcsx2".into(),
            executable: "/usr/bin/pcsx2-qt".into(),
            ..Default::default()
        }];
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
        let all = profiles_for_definition(&env, "retroarch", "/opt/ra/retroarch", "native");
        assert_eq!(all.len(), 20);
        assert!(all.iter().all(|p| p.core_path.is_none()));
        env.files.insert("/opt/ra/cores/snes9x_libretro.so".into());
        let some = profiles_for_definition(&env, "retroarch", "/opt/ra/retroarch", "native");
        assert_eq!(some.len(), 1);
        assert_eq!(some[0].core_path.as_deref(), Some("/opt/ra/cores/snes9x_libretro.so"));
        let pcsx2 = profiles_for_definition(&env, "pcsx2", "/x/pcsx2", "native");
        assert_eq!(pcsx2.len(), 1);
        assert_eq!(profiles_for_definition(&env, "custom", "", "native")[0].name, "Custom");
    }

    #[test]
    fn retroarch_without_cores_is_not_detected() {
        let mut env = Fake::default();
        env.bins.insert("retroarch".into(), "/usr/bin/retroarch".into());
        assert!(detect_emulators(&env, &[]).is_empty());
        env.files.insert("/usr/bin/cores/nestopia_libretro.so".into());
        let found = detect_emulators(&env, &[]);
        assert_eq!(found[0].profiles[0].id, "nes");
    }

    #[test]
    fn list_cores_filters_names() {
        let mut env = Fake::default();
        env.dirs.insert(
            "/usr/lib/libretro".into(),
            vec!["b_libretro.so".into(), "a_libretro.so".into(), "readme.txt".into(), "c_libretro.info".into()],
        );
        assert_eq!(
            list_core_paths(&env, None),
            ["/usr/lib/libretro/a_libretro.so", "/usr/lib/libretro/b_libretro.so"]
        );
    }

    #[test]
    fn add_detected_registers_new_and_refreshes_existing() {
        let d = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(d.path());
        let mut env = Fake::default();
        env.bins.insert("pcsx2-qt".into(), "/usr/bin/pcsx2-qt".into());
        env.bins.insert("retroarch".into(), "/opt/ra/retroarch".into());
        env.files.insert("/opt/ra/cores/snes9x_libretro.so".into());
        let list = add_detected_emulators(&paths, &env).unwrap();
        assert_eq!(list.len(), 2);
        let ra = list.iter().find(|e| e.definition_id == "retroarch").unwrap();
        assert_eq!(ra.profiles.len(), 1);
        assert_eq!(ra.profiles[0].id, "snes");
        assert_eq!(ra.profiles[0].core_path.as_deref(), Some("/opt/ra/cores/snes9x_libretro.so"));
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
        env.files.insert("/opt/ra/cores/snes9x_libretro.so".into());
        env.files.insert("/opt/ra/cores/nestopia_libretro.so".into());
        let list = add_detected_emulators(&paths, &env).unwrap();
        assert_eq!(list.len(), 2);
        let ra = list.iter().find(|e| e.definition_id == "retroarch").unwrap();
        assert_eq!(ra.name, "My RA");
        let ids: Vec<_> = ra.profiles.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["snes", "nes"]);
        assert!(ra.profiles[0].arguments.contains("--custom"));
    }
}
