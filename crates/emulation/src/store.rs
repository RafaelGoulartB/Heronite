//! User emulators (`local_library/emulators.json`) and ROM scanners
//! (`local_library/rom-scanners.json`): port of `emulation/store.ts`.

use anyhow::Result;
use heronite_data::json_store;
use heronite_data::HeroicPaths;
use heronite_launch::emulators::{EmulatorsFile, UserEmulator, UserEmulatorProfile};
use uuid::Uuid;

use crate::catalog::{definition_profile, emulator_definition};
use crate::types::{RomScanner, RomScannersFile, UpsertRomScannerArgs, UpsertUserEmulatorArgs};

pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn list_user_emulators(paths: &HeroicPaths) -> Result<Vec<UserEmulator>> {
    let f: EmulatorsFile = json_store::read(&paths.emulators())?;
    Ok(f.emulators)
}

pub fn get_user_emulator(paths: &HeroicPaths, id: &str) -> Result<Option<UserEmulator>> {
    Ok(list_user_emulators(paths)?.into_iter().find(|e| e.id == id))
}

pub fn find_user_emulator_by_playnite_id(
    paths: &HeroicPaths,
    playnite_id: &str,
) -> Result<Option<UserEmulator>> {
    Ok(list_user_emulators(paths)?
        .into_iter()
        .find(|e| e.playnite_id.as_deref() == Some(playnite_id)))
}

pub fn find_user_emulator_by_definition(
    paths: &HeroicPaths,
    definition_id: &str,
) -> Result<Option<UserEmulator>> {
    Ok(list_user_emulators(paths)?
        .into_iter()
        .find(|e| e.definition_id == definition_id))
}

/// `defaultProfiles`: the given profiles, or one per catalog profile with
/// fresh ids (a single "Custom" profile for unknown definitions).
fn default_profiles(
    definition_id: &str,
    incoming: Option<Vec<UserEmulatorProfile>>,
) -> Vec<UserEmulatorProfile> {
    if let Some(p) = incoming.filter(|p| !p.is_empty()) {
        return p;
    }
    let Some(def) = emulator_definition(definition_id) else {
        return vec![UserEmulatorProfile {
            id: new_id(),
            definition_profile_id: Some("custom".into()),
            name: "Custom".into(),
            platform_id: Some("other".into()),
            platform_name: Some("Other".into()),
            arguments: "{ImagePath}".into(),
            extensions: vec![],
            ..Default::default()
        }];
    };
    def.profiles
        .iter()
        .map(|p| UserEmulatorProfile {
            id: new_id(),
            definition_profile_id: Some(p.id.clone()),
            name: p.name.clone(),
            platform_id: Some(p.platform_id.clone()),
            platform_name: Some(p.platform_name.clone()),
            arguments: p.arguments.clone(),
            extensions: p.extensions.clone(),
            core_path: None,
            folder_indicator: p.folder_indicator.clone(),
            playlist_extensions: p.playlist_extensions.clone(),
            working_dir: p.working_dir.clone(),
            extra: Default::default(),
        })
        .collect()
}

fn non_empty(s: Option<&str>) -> Option<String> {
    s.map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned)
}

/// `upsertUserEmulator` on an in-memory list.
pub fn upsert_emulator_in(
    emulators: &mut Vec<UserEmulator>,
    args: UpsertUserEmulatorArgs,
) -> UserEmulator {
    let existing: Option<UserEmulator> = if let Some(id) = &args.id {
        emulators.iter().find(|e| &e.id == id).cloned()
    } else if let Some(pid) = &args.playnite_id {
        emulators
            .iter()
            .find(|e| e.playnite_id.as_deref() == Some(pid.as_str()))
            .cloned()
    } else if args.definition_id != "custom" {
        emulators
            .iter()
            .find(|e| e.definition_id == args.definition_id && e.executable == args.executable.trim())
            .cloned()
    } else {
        None
    };

    let name = non_empty(Some(&args.name))
        .or_else(|| existing.as_ref().map(|e| e.name.clone()).filter(|n| !n.is_empty()))
        .unwrap_or_else(|| args.definition_id.clone());
    let profiles_in = args
        .profiles
        .clone()
        .or_else(|| existing.as_ref().map(|e| e.profiles.clone()));
    let next = UserEmulator {
        id: existing
            .as_ref()
            .map(|e| e.id.clone())
            .or_else(|| args.id.clone())
            .unwrap_or_else(new_id),
        definition_id: args.definition_id.clone(),
        name,
        executable: args.executable.trim().to_owned(),
        install_dir: non_empty(args.install_dir.as_deref())
            .or_else(|| existing.as_ref().and_then(|e| e.install_dir.clone())),
        launch_kind: args.launch_kind.clone(),
        flatpak_id: args
            .flatpak_id
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| existing.as_ref().and_then(|e| e.flatpak_id.clone())),
        playnite_id: args
            .playnite_id
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| existing.as_ref().and_then(|e| e.playnite_id.clone())),
        profiles: default_profiles(&args.definition_id, profiles_in),
        // Keep fields this app does not model.
        extra: existing.map(|e| e.extra).unwrap_or_default(),
    };
    match emulators.iter().position(|e| e.id == next.id) {
        Some(i) => emulators[i] = next.clone(),
        None => emulators.push(next.clone()),
    }
    next
}

/// `upsertUserEmulator` (`UpsertUserEmulatorArgs` of the UI).
pub fn upsert_user_emulator(
    paths: &HeroicPaths,
    args: UpsertUserEmulatorArgs,
) -> Result<UserEmulator> {
    json_store::update::<EmulatorsFile, _>(&paths.emulators(), |f| {
        Ok(upsert_emulator_in(&mut f.emulators, args))
    })
}

/// `removeUserEmulator`: also drops the scanners that used it. Returns the
/// remaining emulators.
pub fn remove_user_emulator(paths: &HeroicPaths, id: &str) -> Result<Vec<UserEmulator>> {
    let remaining = json_store::update::<EmulatorsFile, _>(&paths.emulators(), |f| {
        f.emulators.retain(|e| e.id != id);
        Ok(f.emulators.clone())
    })?;
    json_store::update::<RomScannersFile, _>(&paths.rom_scanners(), |f| {
        f.scanners.retain(|s| s.emulator_id != id);
        Ok(())
    })?;
    Ok(remaining)
}

/// What `resolveProfileTemplate` yields (everything scanning and launching
/// need), falling back to the catalog for empty fields.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTemplate {
    pub arguments: String,
    pub extensions: Vec<String>,
    pub folder_indicator: Option<String>,
    pub playlist_extensions: Option<Vec<String>>,
    pub working_dir: Option<String>,
    pub platform_id: Option<String>,
    pub platform_name: Option<String>,
    pub core_path: Option<String>,
}

pub fn resolve_template(emulator: &UserEmulator, profile: &UserEmulatorProfile) -> ResolvedTemplate {
    let catalog = profile
        .definition_profile_id
        .as_deref()
        .and_then(|pid| definition_profile(&emulator.definition_id, pid));
    ResolvedTemplate {
        arguments: if !profile.arguments.is_empty() {
            profile.arguments.clone()
        } else {
            catalog
                .map(|c| c.arguments.clone())
                .filter(|a| !a.is_empty())
                .unwrap_or_else(|| "{ImagePath}".into())
        },
        extensions: if !profile.extensions.is_empty() {
            profile.extensions.clone()
        } else {
            catalog.map(|c| c.extensions.clone()).unwrap_or_default()
        },
        folder_indicator: profile
            .folder_indicator
            .clone()
            .or_else(|| catalog.and_then(|c| c.folder_indicator.clone())),
        playlist_extensions: profile
            .playlist_extensions
            .clone()
            .or_else(|| catalog.and_then(|c| c.playlist_extensions.clone())),
        working_dir: profile
            .working_dir
            .clone()
            .or_else(|| catalog.and_then(|c| c.working_dir.clone())),
        platform_id: profile
            .platform_id
            .clone()
            .or_else(|| catalog.map(|c| c.platform_id.clone())),
        platform_name: profile
            .platform_name
            .clone()
            .or_else(|| catalog.map(|c| c.platform_name.clone())),
        core_path: profile.core_path.clone(),
    }
}

// ---------------------------------------------------------------------------
// Scanners
// ---------------------------------------------------------------------------

pub fn list_rom_scanners(paths: &HeroicPaths) -> Result<Vec<RomScanner>> {
    let f: RomScannersFile = json_store::read(&paths.rom_scanners())?;
    Ok(f.scanners)
}

pub fn get_rom_scanner(paths: &HeroicPaths, id: &str) -> Result<Option<RomScanner>> {
    Ok(list_rom_scanners(paths)?.into_iter().find(|s| s.id == id))
}

/// `upsertRomScanner`.
pub fn upsert_rom_scanner(paths: &HeroicPaths, args: UpsertRomScannerArgs) -> Result<RomScanner> {
    let emulator = get_user_emulator(paths, &args.emulator_id)?;
    json_store::update::<RomScannersFile, _>(&paths.rom_scanners(), |f| {
        let existing = args
            .id
            .as_ref()
            .and_then(|id| f.scanners.iter().find(|s| &s.id == id))
            .or_else(|| {
                f.scanners.iter().find(|s| {
                    s.emulator_id == args.emulator_id
                        && s.profile_id == args.profile_id
                        && s.directory == args.directory
                })
            })
            .cloned();
        let profile = emulator
            .as_ref()
            .and_then(|e| e.profile(&args.profile_id).cloned());
        let default_name = [
            emulator.as_ref().map(|e| e.name.as_str()),
            profile.as_ref().map(|p| p.name.as_str()),
        ]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" \u{b7} ");
        let next = RomScanner {
            id: existing
                .as_ref()
                .map(|s| s.id.clone())
                .or_else(|| args.id.clone())
                .unwrap_or_else(new_id),
            name: non_empty(args.name.as_deref())
                .or_else(|| existing.as_ref().map(|s| s.name.clone()).filter(|n| !n.is_empty()))
                .or_else(|| Some(default_name).filter(|n| !n.is_empty()))
                .unwrap_or_else(|| "ROM folder".into()),
            emulator_id: args.emulator_id.clone(),
            profile_id: args.profile_id.clone(),
            directory: args.directory.trim().to_owned(),
            scan_subfolders: args
                .scan_subfolders
                .or(existing.as_ref().map(|s| s.scan_subfolders))
                .unwrap_or(true),
            merge_related_files: args
                .merge_related_files
                .or(existing.as_ref().map(|s| s.merge_related_files))
                .unwrap_or(true),
            auto_scan_on_refresh: args
                .auto_scan_on_refresh
                .or(existing.as_ref().map(|s| s.auto_scan_on_refresh))
                .unwrap_or(true),
            extra: existing.map(|s| s.extra).unwrap_or_default(),
        };
        match f.scanners.iter().position(|s| s.id == next.id) {
            Some(i) => f.scanners[i] = next.clone(),
            None => f.scanners.push(next.clone()),
        }
        Ok(next)
    })
}

/// `removeRomScanner`; returns the remaining scanners.
pub fn remove_rom_scanner(paths: &HeroicPaths, id: &str) -> Result<Vec<RomScanner>> {
    json_store::update::<RomScannersFile, _>(&paths.rom_scanners(), |f| {
        f.scanners.retain(|s| s.id != id);
        Ok(f.scanners.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> (tempfile::TempDir, HeroicPaths) {
        let d = tempfile::tempdir().unwrap();
        let p = HeroicPaths::at(d.path());
        (d, p)
    }

    fn args(def: &str, exe: &str) -> UpsertUserEmulatorArgs {
        UpsertUserEmulatorArgs {
            definition_id: def.into(),
            name: "  Dolphin  ".into(),
            executable: format!(" {exe} "),
            launch_kind: "native".into(),
            ..Default::default()
        }
    }

    #[test]
    fn upsert_creates_catalog_profiles_and_dedups() {
        let (_d, paths) = temp();
        let a = upsert_user_emulator(&paths, args("dolphin", "/usr/bin/dolphin-emu")).unwrap();
        assert_eq!(a.name, "Dolphin");
        assert_eq!(a.executable, "/usr/bin/dolphin-emu");
        assert_eq!(a.profiles.len(), 2);
        assert_eq!(a.profiles[0].definition_profile_id.as_deref(), Some("gamecube"));
        assert_eq!(a.profiles[0].arguments, "--exec={ImagePath} --batch");
        assert_eq!(a.profiles[0].playlist_extensions.as_deref(), Some(&["m3u".to_string()][..]));
        // Same definition + executable updates in place.
        let mut again = args("dolphin", "/usr/bin/dolphin-emu");
        again.name = String::new();
        let b = upsert_user_emulator(&paths, again).unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(b.name, "Dolphin");
        assert_eq!(b.profiles[0].id, a.profiles[0].id);
        assert_eq!(list_user_emulators(&paths).unwrap().len(), 1);
        // Custom never dedups on executable.
        upsert_user_emulator(&paths, args("custom", "/bin/x")).unwrap();
        let c = upsert_user_emulator(&paths, args("custom", "/bin/x")).unwrap();
        assert_eq!(c.profiles.len(), 1);
        assert_eq!(c.profiles[0].definition_profile_id.as_deref(), Some("custom"));
        assert_eq!(list_user_emulators(&paths).unwrap().len(), 3);
    }

    #[test]
    fn unknown_definition_gets_custom_profile() {
        let (_d, paths) = temp();
        let e = upsert_user_emulator(&paths, args("nope", "/x")).unwrap();
        assert_eq!(e.profiles[0].name, "Custom");
        assert_eq!(e.profiles[0].platform_id.as_deref(), Some("other"));
    }

    #[test]
    fn explicit_id_updates_and_keeps_optional_fields() {
        let (_d, paths) = temp();
        let mut a = args("pcsx2", "/x/pcsx2");
        a.install_dir = Some("/x".into());
        a.flatpak_id = Some("net.pcsx2.PCSX2".into());
        let first = upsert_user_emulator(&paths, a).unwrap();
        let mut b = args("pcsx2", "/y/pcsx2");
        b.id = Some(first.id.clone());
        let second = upsert_user_emulator(&paths, b).unwrap();
        assert_eq!(second.id, first.id);
        assert_eq!(second.executable, "/y/pcsx2");
        assert_eq!(second.install_dir.as_deref(), Some("/x"));
        assert_eq!(second.flatpak_id.as_deref(), Some("net.pcsx2.PCSX2"));
    }

    #[test]
    fn scanners_crud_and_cascade() {
        let (_d, paths) = temp();
        let e = upsert_user_emulator(&paths, args("pcsx2", "/x/pcsx2")).unwrap();
        let s = upsert_rom_scanner(
            &paths,
            UpsertRomScannerArgs {
                emulator_id: e.id.clone(),
                profile_id: e.profiles[0].id.clone(),
                directory: " /roms ".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(s.name, "Dolphin \u{b7} PlayStation 2");
        assert_eq!(s.directory, "/roms");
        assert!(s.scan_subfolders && s.merge_related_files && s.auto_scan_on_refresh);
        // Same triple (untrimmed directory matches the stored trimmed one only
        // when passed trimmed): updates in place.
        let s2 = upsert_rom_scanner(
            &paths,
            UpsertRomScannerArgs {
                emulator_id: e.id.clone(),
                profile_id: e.profiles[0].id.clone(),
                directory: "/roms".into(),
                auto_scan_on_refresh: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(s2.id, s.id);
        assert!(!s2.auto_scan_on_refresh);
        assert_eq!(list_rom_scanners(&paths).unwrap().len(), 1);
        remove_user_emulator(&paths, &e.id).unwrap();
        assert!(list_rom_scanners(&paths).unwrap().is_empty());
        assert!(list_user_emulators(&paths).unwrap().is_empty());
    }

    #[test]
    fn template_falls_back_to_catalog() {
        let (_d, paths) = temp();
        let e = upsert_user_emulator(&paths, args("retroarch", "/usr/bin/retroarch")).unwrap();
        let mut p = e.profiles.iter().find(|p| p.definition_profile_id.as_deref() == Some("snes")).unwrap().clone();
        p.arguments = String::new();
        p.extensions = vec![];
        p.core_path = Some("/c/snes9x_libretro.so".into());
        let t = resolve_template(&e, &p);
        assert_eq!(t.arguments, "-L {CorePath} {ImagePath}");
        assert!(t.extensions.contains(&"sfc".to_string()));
        assert_eq!(t.core_path.as_deref(), Some("/c/snes9x_libretro.so"));
        assert_eq!(t.platform_name.as_deref(), Some("Super Nintendo"));
    }
}
