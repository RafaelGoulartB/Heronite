//! Data for the emulator manager page: every configured emulator with its
//! health, the games that use it and its ROM folders.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use heronite_data::models::LocalMetaFile;
use heronite_data::{json_store, DataPaths};
use heronite_i18n::tr;
use heronite_launch::emulators::UserEmulator;

use crate::detect::DetectEnv;
use crate::store;
use crate::types::RomScanner;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Ok,
    /// The executable does not exist.
    MissingExecutable,
    /// It exists but is not executable (AppImages that were not `chmod +x`).
    NotExecutable,
    /// A Flatpak emulator whose app is not installed.
    FlatpakMissing,
}

impl Health {
    pub fn is_ok(self) -> bool {
        self == Health::Ok
    }

    pub fn label(self) -> &'static str {
        match self {
            Health::Ok => tr!("Ready"),
            Health::MissingExecutable => tr!("Executable not found"),
            Health::NotExecutable => tr!("Not executable (chmod +x)"),
            Health::FlatpakMissing => tr!("Flatpak not installed"),
        }
    }
}

/// Whether `emulator` can be launched. Runs `flatpak info` for Flatpak
/// emulators, so call it off the UI thread.
pub fn emulator_health(env: &dyn DetectEnv, emulator: &UserEmulator) -> Health {
    if emulator.launch_kind == "flatpak" {
        return match emulator.flatpak_id.as_deref().filter(|id| !id.is_empty()) {
            Some(id) if env.flatpak_installed(id) => Health::Ok,
            _ => Health::FlatpakMissing,
        };
    }
    let exe = emulator.executable.trim();
    if exe.is_empty() {
        return Health::MissingExecutable;
    }
    let path = if exe.contains('/') { Some(exe.to_owned()) } else { env.which(exe) };
    let Some(path) = path else { return Health::MissingExecutable };
    match std::fs::metadata(Path::new(&path)) {
        Ok(m) if m.is_file() => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if m.permissions().mode() & 0o111 == 0 && emulator.launch_kind != "wine" {
                    return Health::NotExecutable;
                }
            }
            Health::Ok
        }
        _ => Health::MissingExecutable,
    }
}

/// Games per emulator id (`local_library/library.json` metas).
pub fn count_games_by_emulator(paths: &DataPaths) -> Result<HashMap<String, usize>> {
    let metas: LocalMetaFile = json_store::read(&paths.local_meta())?;
    let mut counts: HashMap<String, usize> = HashMap::new();
    for meta in metas.games.values() {
        if let Some(id) = meta.emulator_id.as_deref().filter(|s| !s.is_empty()) {
            *counts.entry(id.to_owned()).or_default() += 1;
        }
    }
    Ok(counts)
}

#[derive(Debug, Clone)]
pub struct EmulatorSummary {
    pub emulator: UserEmulator,
    pub health: Health,
    pub games: usize,
    pub scanners: Vec<RomScanner>,
    /// Directories of `scanners` that do not exist (unmounted drive, moved).
    pub missing_folders: Vec<String>,
}

/// Everything the manager page shows.
pub fn overview(paths: &DataPaths, env: &dyn DetectEnv) -> Result<Vec<EmulatorSummary>> {
    let emulators = store::list_user_emulators(paths)?;
    let scanners = store::list_rom_scanners(paths)?;
    let counts = count_games_by_emulator(paths)?;
    Ok(emulators
        .into_iter()
        .map(|emulator| {
            let scanners: Vec<RomScanner> = scanners.iter().filter(|s| s.emulator_id == emulator.id).cloned().collect();
            EmulatorSummary {
                health: emulator_health(env, &emulator),
                games: counts.get(&emulator.id).copied().unwrap_or(0),
                missing_folders: scanners.iter().filter(|s| !Path::new(&s.directory).is_dir()).map(|s| s.directory.clone()).collect(),
                scanners,
                emulator,
            }
        })
        .collect())
}

/// Scanners that point at an emulator that no longer exists (left behind by
/// hand edits; `remove_user_emulator` removes its own).
pub fn orphan_scanners(paths: &DataPaths) -> Result<Vec<RomScanner>> {
    let emulators = store::list_user_emulators(paths)?;
    Ok(store::list_rom_scanners(paths)?.into_iter().filter(|s| !emulators.iter().any(|e| e.id == s.emulator_id)).collect())
}

/// How many saved ROM folders use a profile of an emulator.
pub fn scanners_using_profile(scanners: &[RomScanner], emulator_id: &str, profile_id: &str) -> usize {
    scanners.iter().filter(|s| s.emulator_id == emulator_id && s.profile_id == profile_id).count()
}

/// Modification stamps of the files the manager shows (cheap change check).
pub fn data_stamp(paths: &DataPaths) -> Vec<Option<std::time::SystemTime>> {
    [paths.emulators(), paths.rom_scanners(), paths.local_meta()]
        .iter()
        .map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{UpsertRomScannerArgs, UpsertUserEmulatorArgs};
    use heronite_data::models::LocalGameMeta;
    use std::collections::HashSet;

    struct Env {
        flatpaks: HashSet<String>,
    }
    impl DetectEnv for Env {
        fn which(&self, name: &str) -> Option<String> {
            (name == "retroarch").then(|| "/bin/sh".to_owned())
        }
        fn flatpak_installed(&self, id: &str) -> bool {
            self.flatpaks.contains(id)
        }
        fn exists(&self, path: &str) -> bool {
            Path::new(path).exists()
        }
        fn read_dir(&self, _: &str) -> Vec<String> {
            vec![]
        }
        fn home(&self) -> String {
            "/h".into()
        }
    }

    fn emu(kind: &str, exe: &str, flatpak: Option<&str>) -> UserEmulator {
        UserEmulator {
            id: "e".into(),
            launch_kind: kind.into(),
            executable: exe.into(),
            flatpak_id: flatpak.map(Into::into),
            ..Default::default()
        }
    }

    #[test]
    fn health_covers_native_flatpak_and_permissions() {
        let env = Env { flatpaks: ["org.x.Y".to_owned()].into() };
        assert_eq!(emulator_health(&env, &emu("native", "/bin/sh", None)), Health::Ok);
        assert_eq!(emulator_health(&env, &emu("native", "retroarch", None)), Health::Ok);
        assert_eq!(emulator_health(&env, &emu("native", "/no/such/file", None)), Health::MissingExecutable);
        assert_eq!(emulator_health(&env, &emu("native", "", None)), Health::MissingExecutable);
        assert_eq!(emulator_health(&env, &emu("flatpak", "flatpak", Some("org.x.Y"))), Health::Ok);
        assert_eq!(emulator_health(&env, &emu("flatpak", "flatpak", Some("org.z.Z"))), Health::FlatpakMissing);
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("app.AppImage");
        std::fs::write(&f, "x").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(emulator_health(&env, &emu("native", f.to_str().unwrap(), None)), Health::NotExecutable);
            assert_eq!(emulator_health(&env, &emu("wine", f.to_str().unwrap(), None)), Health::Ok);
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(emulator_health(&env, &emu("native", f.to_str().unwrap(), None)), Health::Ok);
        // A directory is not an executable.
        assert_eq!(emulator_health(&env, &emu("native", d.path().to_str().unwrap(), None)), Health::MissingExecutable);
    }

    #[test]
    fn overview_counts_games_and_groups_folders() {
        let h = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(h.path());
        let a = store::upsert_user_emulator(
            &paths,
            UpsertUserEmulatorArgs {
                definition_id: "pcsx2".into(),
                name: "A".into(),
                executable: "/bin/sh".into(),
                launch_kind: "native".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let b =
            store::insert_user_emulator(&paths, UserEmulator { definition_id: "custom".into(), name: "B".into(), ..Default::default() })
                .unwrap();
        for dir in ["/r1", "/r2"] {
            store::upsert_rom_scanner(
                &paths,
                UpsertRomScannerArgs {
                    emulator_id: a.id.clone(),
                    profile_id: a.profiles[0].id.clone(),
                    directory: dir.into(),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |f| {
            for (k, e) in [("g1", Some(&a.id)), ("g2", Some(&a.id)), ("g3", None)] {
                f.games.insert(k.into(), LocalGameMeta { app_name: k.into(), emulator_id: e.cloned(), ..Default::default() });
            }
            Ok(())
        })
        .unwrap();
        let o = overview(&paths, &Env { flatpaks: Default::default() }).unwrap();
        assert_eq!(o.len(), 2);
        assert_eq!((o[0].games, o[0].scanners.len(), o[0].health), (2, 2, Health::Ok));
        assert_eq!((o[1].games, o[1].scanners.len(), o[1].health), (0, 0, Health::MissingExecutable));
        assert_eq!(o[1].emulator.id, b.id);
        assert_eq!(o[0].missing_folders, ["/r1", "/r2"]);
        assert_eq!(scanners_using_profile(&o[0].scanners, &a.id, &a.profiles[0].id), 2);
        assert!(orphan_scanners(&paths).unwrap().is_empty());
    }
}
