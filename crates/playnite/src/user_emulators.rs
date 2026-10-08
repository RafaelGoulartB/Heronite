//! `ensurePlayniteEmulator` (`emulation/playnite.ts`) with the
//! `upsertUserEmulator` / `guessDefinitionId` pieces it uses, writing
//! `local_library/emulators.json`.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use heronite_data::{json_store, HeroicPaths};
use heronite_launch::emulators::{emulator_definition, EmulatorsFile, UserEmulator, UserEmulatorProfile};

use crate::reader::PlayniteEmulator;
use crate::remap::{remap_windows_path, DriveRemap};

/// A random version-4 UUID (no `uuid` dependency: two SipHash keys from the
/// std `RandomState`, the clock and a counter).
pub fn random_uuid() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut parts = [0u64; 2];
    for (i, part) in parts.iter_mut().enumerate() {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed) + i as u64);
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default(),
        );
        *part = h.finish();
    }
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&parts[0].to_le_bytes());
    b[8..].copy_from_slice(&parts[1].to_le_bytes());
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// `guessDefinitionId`.
pub fn guess_definition_id(name: Option<&str>) -> &'static str {
    let normalized: String = name.unwrap_or_default().to_lowercase().chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit()).collect();
    if normalized.is_empty() {
        return "custom";
    }
    const ALIASES: [(&str, &str); 18] = [
        ("retroarch", "retroarch"),
        ("dolphin", "dolphin"),
        ("pcsx2", "pcsx2"),
        ("duckstation", "duckstation"),
        ("ppsspp", "ppsspp"),
        ("rpcs3", "rpcs3"),
        ("cemu", "cemu"),
        ("ryujinx", "ryujinx"),
        ("xemu", "xemu"),
        ("melonds", "melonds"),
        ("mgba", "mgba"),
        ("flycast", "flycast"),
        ("mame", "mame"),
        ("scummvm", "scummvm"),
        ("vita3k", "vita3k"),
        ("shadps4", "shadps4"),
        ("azahar", "azahar"),
        ("citra", "azahar"),
    ];
    ALIASES.iter().find(|(needle, _)| normalized.contains(needle)).map(|(_, id)| *id).unwrap_or("custom")
}

/// `defaultProfiles` when no profiles come in. The Rust launch catalog has no
/// display names / platforms / extensions, so catalog profiles are named after
/// their id and carry empty extensions.
fn default_profiles(definition_id: &str) -> Vec<UserEmulatorProfile> {
    match emulator_definition(definition_id) {
        Some(def) if !def.profiles.is_empty() => def
            .profiles
            .iter()
            .map(|p| UserEmulatorProfile {
                id: random_uuid(),
                definition_profile_id: Some(p.id.to_owned()),
                name: p.id.to_owned(),
                arguments: p.arguments.to_owned(),
                extensions: Vec::new(),
                working_dir: p.working_dir.map(str::to_owned),
                ..Default::default()
            })
            .collect(),
        _ => vec![UserEmulatorProfile {
            id: random_uuid(),
            definition_profile_id: Some("custom".into()),
            name: "Custom".into(),
            platform_id: Some("other".into()),
            platform_name: Some("Other".into()),
            arguments: "{ImagePath}".into(),
            extensions: Vec::new(),
            ..Default::default()
        }],
    }
}

fn non_empty(s: Option<&str>) -> Option<&str> {
    s.filter(|v| !v.is_empty())
}

/// `ensurePlayniteEmulator`: creates or refreshes the user emulator that
/// mirrors a Playnite one. `None` when there is no executable to point at and
/// nothing existed yet (nothing is written then).
pub fn ensure_playnite_emulator(
    paths: &HeroicPaths,
    playnite: &PlayniteEmulator,
    drive_map: &[DriveRemap],
) -> Result<Option<UserEmulator>> {
    json_store::update::<EmulatorsFile, _>(&paths.emulators(), |file| {
        let existing_idx = file.emulators.iter().position(|e| e.playnite_id.as_deref() == Some(playnite.id.as_str()));
        let existing = existing_idx.map(|i| file.emulators[i].clone());

        let install_dir = remap_windows_path(playnite.install_dir.as_deref(), drive_map);
        let profile = playnite.profiles.first();
        let raw_exe = profile
            .and_then(|p| p.executable.clone().or_else(|| p.startup_executable.clone()))
            .or_else(|| playnite.install_dir.clone());
        let executable = remap_windows_path(raw_exe.as_deref(), drive_map).or_else(|| {
            if raw_exe.as_deref().is_some_and(|e| e.starts_with('/')) { raw_exe.clone() } else { install_dir.clone() }
        });
        if non_empty(executable.as_deref()).is_none() && existing.is_none() {
            return Ok(None);
        }

        let definition_id = guess_definition_id(playnite.name.as_deref());
        let wine = executable.as_deref().is_some_and(|e| e.to_lowercase().ends_with(".exe"));
        let name = non_empty(playnite.name.as_deref()).unwrap_or(definition_id);

        let incoming: Option<Vec<UserEmulatorProfile>> = if playnite.profiles.is_empty() {
            existing.as_ref().map(|e| e.profiles.clone())
        } else {
            let ext = existing.as_ref().and_then(|e| e.profiles.first()).map(|p| p.extensions.clone()).unwrap_or_default();
            Some(
                playnite
                    .profiles
                    .iter()
                    .map(|item| UserEmulatorProfile {
                        id: non_empty(item.id.as_deref())
                            .or(non_empty(item.name.as_deref()))
                            .unwrap_or("default")
                            .to_owned(),
                        definition_profile_id: item.id.clone(),
                        name: non_empty(item.name.as_deref()).unwrap_or("Default").to_owned(),
                        arguments: non_empty(item.arguments.as_deref())
                            .or(non_empty(item.custom_arguments.as_deref()))
                            .unwrap_or("{ImagePath}")
                            .to_owned(),
                        extensions: ext.clone(),
                        ..Default::default()
                    })
                    .collect(),
            )
        };
        let profiles = match incoming {
            Some(p) if !p.is_empty() => p,
            _ => default_profiles(definition_id),
        };

        let trimmed_name = name.trim();
        let mut next = existing.clone().unwrap_or_default();
        next.id = existing.as_ref().map(|e| e.id.clone()).unwrap_or_else(random_uuid);
        next.definition_id = definition_id.to_owned();
        next.name = if !trimmed_name.is_empty() {
            trimmed_name.to_owned()
        } else {
            existing.as_ref().map(|e| e.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| definition_id.to_owned())
        };
        let exe = non_empty(executable.as_deref())
            .or(existing.as_ref().map(|e| e.executable.as_str()).filter(|e| !e.is_empty()))
            .unwrap_or("");
        next.executable = exe.trim().to_owned();
        next.install_dir = non_empty(install_dir.as_deref().map(str::trim))
            .map(str::to_owned)
            .or_else(|| existing.as_ref().and_then(|e| e.install_dir.clone()));
        next.launch_kind = if wine { "wine" } else { "native" }.to_owned();
        next.playnite_id = Some(playnite.id.clone());
        next.profiles = profiles;

        match existing_idx {
            Some(i) => file.emulators[i] = next.clone(),
            None => file.emulators.push(next.clone()),
        }
        Ok(Some(next))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::PlayniteEmulatorProfile;

    #[test]
    fn uuid_shape_and_uniqueness() {
        let a = random_uuid();
        let b = random_uuid();
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        assert_eq!(&a[14..15], "4");
    }

    #[test]
    fn guesses_definitions() {
        assert_eq!(guess_definition_id(Some("PCSX2 (Nightly)")), "pcsx2");
        assert_eq!(guess_definition_id(Some("Citra")), "azahar");
        assert_eq!(guess_definition_id(Some("???")), "custom");
        assert_eq!(guess_definition_id(None), "custom");
    }

    #[test]
    fn creates_then_refreshes_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let paths = HeroicPaths::at(dir.path());
        let map = [DriveRemap { windows_root: "E:\\".into(), linux_path: "/mnt/e".into() }];
        let pe = PlayniteEmulator {
            id: "pe-1".into(),
            name: Some("PCSX2".into()),
            install_dir: Some("E:\\emu".into()),
            profiles: vec![PlayniteEmulatorProfile {
                id: Some("p1".into()),
                name: Some("PS2".into()),
                executable: Some("E:\\emu\\pcsx2.exe".into()),
                ..Default::default()
            }],
        };
        let first = ensure_playnite_emulator(&paths, &pe, &map).unwrap().unwrap();
        assert_eq!(first.definition_id, "pcsx2");
        assert_eq!(first.executable, "/mnt/e/emu/pcsx2.exe");
        assert_eq!(first.launch_kind, "wine");
        assert_eq!(first.install_dir.as_deref(), Some("/mnt/e/emu"));
        assert_eq!(first.profiles[0].id, "p1");
        assert_eq!(first.profiles[0].arguments, "{ImagePath}");
        let second = ensure_playnite_emulator(&paths, &pe, &map).unwrap().unwrap();
        assert_eq!(first.id, second.id);
        let file: EmulatorsFile = json_store::read(&paths.emulators()).unwrap();
        assert_eq!(file.emulators.len(), 1);

        // no executable and nothing existing: nothing written
        let nothing = PlayniteEmulator { id: "pe-2".into(), name: Some("X".into()), install_dir: None, profiles: vec![] };
        assert!(ensure_playnite_emulator(&paths, &nothing, &[]).unwrap().is_none());
        let file: EmulatorsFile = json_store::read(&paths.emulators()).unwrap();
        assert_eq!(file.emulators.len(), 1);
    }
}
