//! Resolving a game to its emulator engine and listing its states.

use std::collections::HashSet;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use heronite_data::models::{LocalGameMeta, LocalMetaFile};
use heronite_data::{json_store, HeroicPaths};
use heronite_launch::emulators::EmulatorsFile;
use heronite_launch::plan::pick_rom_path;
use heronite_launch::probe::SystemProbe;

use super::{
    engine_for, engine_id_for, thumbnail_for, Engine, EngineContext, EmulatorSaveState,
    EmulatorSaveStateList, MAX_STATES,
};
use crate::catalog::emulator_definition;
use crate::iso::iso_millis;
use crate::pathutil::{join, unique};
use crate::store::resolve_template;
use crate::sysfiles::HostEnv;

pub(super) struct Resolved {
    meta: LocalGameMeta,
    pub(super) engine: Engine,
    pub(super) ctx: EngineContext,
    pub(super) rom_path: String,
    emulator_name: String,
}

pub(super) fn resolve(paths: &HeroicPaths, app_name: &str, rom_path: Option<&str>) -> Result<Option<Resolved>> {
    let metas: LocalMetaFile = json_store::read(&paths.local_meta())?;
    let Some(meta) = metas.games.get(app_name).cloned() else {
        return Ok(None);
    };
    if meta.launch_kind != "emulator" {
        return Ok(None);
    }
    let Some(emulator_id) = meta.emulator_id.clone().filter(|i| !i.is_empty()) else {
        return Ok(None);
    };
    let emulators: EmulatorsFile = json_store::read(&paths.emulators())?;
    let Some(emulator) = emulators.get(&emulator_id).cloned() else {
        return Ok(None);
    };
    if emulator.launch_kind == "wine" {
        return Ok(None);
    }
    let engine_id = engine_id_for(&emulator);
    let Some(engine) = engine_for(&engine_id) else {
        return Ok(None);
    };
    let requested: Vec<String> = rom_path.map(|r| vec![r.to_string()]).unwrap_or_default();
    let Some(rom) = pick_rom_path(&meta, &requested, &SystemProbe) else {
        return Ok(None);
    };
    let profile = emulator.profile(meta.emulator_profile_id.as_deref().unwrap_or(""));
    let flatpak_id = (emulator.launch_kind == "flatpak")
        .then(|| {
            emulator
                .flatpak_id
                .clone()
                .filter(|f| !f.is_empty())
                .or_else(|| emulator_definition(&engine_id).and_then(|d| d.flatpak_id.clone()))
        })
        .flatten();
    let executable = if flatpak_id.is_some() {
        None
    } else {
        Some(emulator.executable.clone())
            .filter(|e| !e.is_empty())
            .or_else(|| meta.remapped_executable.clone().filter(|e| !e.is_empty()))
    };
    let core_path = profile.and_then(|p| resolve_template(&emulator, p).core_path);
    Ok(Some(Resolved {
        emulator_name: emulator.name.clone(),
        meta,
        engine,
        ctx: EngineContext { executable, flatpak_id, core_path },
        rom_path: rom,
    }))
}

pub(super) struct Found {
    pub(super) state: EmulatorSaveState,
    key: String,
}

pub(super) fn collect_states(r: &Resolved, env: &HostEnv, keys: &[String], modified_since: Option<SystemTime>) -> Vec<Found> {
    let mut found: Vec<Found> = Vec::new();
    let mut seen = HashSet::new();
    for dir in unique(r.engine.state_dirs(env, &r.ctx, &r.rom_path, keys).into_iter().map(Some)) {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut names: Vec<String> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        for name in names {
            let Some(parsed) = r.engine.parse(&name) else { continue };
            if !keys.is_empty() && !keys.iter().any(|k| k.eq_ignore_ascii_case(&parsed.key)) {
                continue;
            }
            let path = join(&dir, &name);
            if seen.contains(&path) {
                continue;
            }
            let Ok(info) = std::fs::metadata(&path) else { continue };
            if !info.is_file() {
                continue;
            }
            let Ok(mtime) = info.modified() else { continue };
            if modified_since.is_some_and(|since| mtime < since) {
                continue;
            }
            seen.insert(path.clone());
            found.push(Found {
                key: parsed.key,
                state: EmulatorSaveState {
                    path,
                    kind: parsed.kind,
                    slot: parsed.slot,
                    modified_at: iso_millis(mtime),
                    thumbnail: None,
                },
            });
        }
    }
    found.sort_by(|a, b| b.state.modified_at.cmp(&a.state.modified_at));
    found
}

pub(super) fn rom_keys_for(r: &Resolved, env: &HostEnv) -> Vec<String> {
    unique(
        r.engine
            .rom_keys(env, &r.ctx, &r.rom_path)
            .into_iter()
            .map(Some)
            .chain([r.meta.save_state_keys.as_ref().and_then(|m| m.get(&r.rom_path)).cloned()]),
    )
}

/// `listEmulatorSaveStates`: the newest states of the game's ROM (at most
/// [`MAX_STATES`]); thumbnails are loaded when `with_thumbnails`.
pub fn list_emulator_save_states(
    paths: &HeroicPaths,
    env: &HostEnv,
    app_name: &str,
    rom_path: Option<&str>,
    with_thumbnails: bool,
) -> Result<EmulatorSaveStateList> {
    let Some(r) = resolve(paths, app_name, rom_path)? else {
        return Ok(EmulatorSaveStateList { supported: false, ..Default::default() });
    };
    let keys = rom_keys_for(&r, env);
    let mut list = EmulatorSaveStateList {
        supported: true,
        emulator_name: Some(r.emulator_name.clone()),
        rom_path: Some(r.rom_path.clone()),
        states: vec![],
    };
    if keys.is_empty() {
        return Ok(list);
    }
    list.states = collect_states(&r, env, &keys, None)
        .into_iter()
        .take(MAX_STATES)
        .map(|f| {
            let mut st = f.state;
            if with_thumbnails {
                st.thumbnail = thumbnail_for(r.engine, &st);
            }
            st
        })
        .collect();
    Ok(list)
}

/// `rememberSaveStateKey`: after a session, states written for this game
/// reveal the key the emulator uses; storing it lets later launches list
/// states for images whose ID cannot be read (CHD, CSO, GCZ...). Returns the
/// learned key.
pub fn remember_save_state_key(
    paths: &HeroicPaths,
    env: &HostEnv,
    app_name: &str,
    rom_path: Option<&str>,
    started_at: SystemTime,
) -> Result<Option<String>> {
    let Some(rom_path) = rom_path else { return Ok(None) };
    let Some(r) = resolve(paths, app_name, Some(rom_path))? else {
        return Ok(None);
    };
    if !r.engine.learns_keys() {
        return Ok(None);
    }
    // The disc or the emulator's cache is authoritative; only learn when both
    // are unreadable, so switching games inside the emulator cannot mislabel.
    if !r.engine.rom_keys(env, &r.ctx, &r.rom_path).is_empty() {
        return Ok(None);
    }
    let since = started_at.checked_sub(Duration::from_secs(2)).unwrap_or(started_at);
    let written = collect_states(&r, env, &[], Some(since));
    let keys = unique(written.iter().map(|f| Some(f.key.clone())));
    if keys.len() != 1 {
        return Ok(None);
    }
    let key = keys[0].clone();
    let rom = r.rom_path.clone();
    json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |file| {
        let Some(meta) = file.games.get_mut(app_name) else { return Ok(None) };
        let current = meta.save_state_keys.as_ref().and_then(|m| m.get(&rom)).cloned().unwrap_or_default();
        if current.eq_ignore_ascii_case(&key) {
            return Ok(None);
        }
        meta.save_state_keys.get_or_insert_with(Default::default).insert(rom.clone(), key.clone());
        Ok(Some(key.clone()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use crate::save_states::{s, set_pending_launch_state, PendingLaunchStates, SaveStateKind};


    /// End to end with a PCSX2 sandbox: ROM key from the game list cache,
    /// states listed newest first with thumbnails, pending args, and key
    /// learning.
    #[test]
    fn lists_states_end_to_end_for_pcsx2() {
        use crate::types::UpsertUserEmulatorArgs;
        let heroic = tempfile::tempdir().unwrap();
        let paths = HeroicPaths::at(heroic.path());
        let sandbox = tempfile::tempdir().unwrap();
        let home = sandbox.path().join("home");
        let env = HostEnv::sandbox(&home, sandbox.path().join("tmp"));
        let data = home.join(".config/PCSX2");
        let sstates = data.join("sstates");
        fs::create_dir_all(data.join("cache")).unwrap();
        fs::create_dir_all(&sstates).unwrap();
        let rom_dir = sandbox.path().join("roms");
        fs::create_dir_all(&rom_dir).unwrap();
        let rom = s(&rom_dir.join("Game (USA).iso"));
        fs::write(&rom, "not an iso").unwrap();
        let mut blob = vec![0u8; 9];
        blob.extend_from_slice(&(rom.len() as u32).to_le_bytes());
        blob.extend_from_slice(rom.as_bytes());
        blob.extend_from_slice(&10u32.to_le_bytes());
        blob.extend_from_slice(b"SLUS-20731");
        fs::write(data.join("cache/gamelist.cache"), blob).unwrap();

        let e = crate::store::upsert_user_emulator(
            &paths,
            UpsertUserEmulatorArgs {
                definition_id: "custom".into(),
                name: "PCSX2".into(),
                executable: "/opt/pcsx2.AppImage".into(),
                launch_kind: "native".into(),
                ..Default::default()
            },
        )
        .unwrap();
        json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |f| {
            f.games.insert(
                "emu_1".into(),
                heronite_data::models::LocalGameMeta {
                    app_name: "emu_1".into(),
                    runner: "sideload".into(),
                    launch_kind: "emulator".into(),
                    emulator_id: Some(e.id.clone()),
                    emulator_profile_id: Some(e.profiles[0].id.clone()),
                    roms: Some(vec![heronite_data::models::LocalGameRom { name: None, path: rom.clone(), extra: Default::default() }]),
                    ..Default::default()
                },
            );
            Ok(())
        })
        .unwrap();

        fs::write(sstates.join("SLUS-20731 (538ABF5D).resume.p2s"), "x").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let png = [&[0x89u8, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a][..], b"img"].concat();
        let zip = {
            // A stored-only zip with Screenshot.png, built by hand.
            let name = b"Screenshot.png";
            let mut out = Vec::new();
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&(png.len() as u32).to_le_bytes());
            out.extend_from_slice(&(png.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name);
            out.extend_from_slice(&png);
            let dir_offset = out.len() as u32;
            let mut c = Vec::new();
            c.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            c.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            c.extend_from_slice(&0u32.to_le_bytes());
            c.extend_from_slice(&(png.len() as u32).to_le_bytes());
            c.extend_from_slice(&(png.len() as u32).to_le_bytes());
            c.extend_from_slice(&(name.len() as u16).to_le_bytes());
            c.extend_from_slice(&[0; 12]);
            c.extend_from_slice(&0u32.to_le_bytes());
            c.extend_from_slice(name);
            out.extend_from_slice(&c);
            out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
            out.extend_from_slice(&[0; 4]);
            out.extend_from_slice(&1u16.to_le_bytes());
            out.extend_from_slice(&1u16.to_le_bytes());
            out.extend_from_slice(&(c.len() as u32).to_le_bytes());
            out.extend_from_slice(&dir_offset.to_le_bytes());
            out.extend_from_slice(&[0, 0]);
            out
        };
        fs::write(sstates.join("SLUS-20731 (538ABF5D).01.p2s"), zip).unwrap();
        fs::write(sstates.join("SLUS-99999 (11111111).01.p2s"), "other game").unwrap();

        let list = list_emulator_save_states(&paths, &env, "emu_1", None, true).unwrap();
        assert!(list.supported);
        assert_eq!(list.emulator_name.as_deref(), Some("PCSX2"));
        assert_eq!(list.rom_path.as_deref(), Some(rom.as_str()));
        assert_eq!(list.states.len(), 2);
        assert_eq!(list.states[0].kind, SaveStateKind::Slot);
        assert_eq!(list.states[0].slot, Some(1));
        assert_eq!(list.states[1].kind, SaveStateKind::Resume);
        let thumb = list.states[0].thumbnail.as_ref().unwrap();
        assert_eq!(thumb.mime, "image/png");
        assert_eq!(thumb.bytes, png);
        assert!(list.states[1].thumbnail.is_none());
        assert!(list.states[0].modified_at.ends_with('Z'));
        let json = serde_json::to_value(&list).unwrap();
        assert_eq!(json["states"][0]["kind"], "slot");
        assert!(json["states"][0].get("thumbnail").is_none());

        // Pending launch state.
        let pending = PendingLaunchStates::default();
        let st = list.states[0].path.clone();
        assert!(set_pending_launch_state(&paths, &env, &pending, "emu_1", None, Some(&st)).unwrap());
        let taken = pending.take("emu_1", Some(&rom)).unwrap();
        assert_eq!(taken.args, ["-statefile", st.as_str()]);
        assert!(!set_pending_launch_state(&paths, &env, &pending, "emu_1", None, Some("/gone")).unwrap());
        // Fresh start for PCSX2 needs no args.
        assert!(set_pending_launch_state(&paths, &env, &pending, "emu_1", None, None).unwrap());
        assert!(pending.take("emu_1", None).is_none());

        // Unsupported / unknown games.
        assert!(!list_emulator_save_states(&paths, &env, "nope", None, false).unwrap().supported);
    }

    #[test]
    fn learns_key_when_the_rom_is_unreadable() {
        use crate::types::UpsertUserEmulatorArgs;
        let heroic = tempfile::tempdir().unwrap();
        let paths = HeroicPaths::at(heroic.path());
        let sandbox = tempfile::tempdir().unwrap();
        let home = sandbox.path().join("home");
        let env = HostEnv::sandbox(&home, sandbox.path().join("tmp"));
        let states = home.join(".local/share/duckstation/savestates");
        fs::create_dir_all(&states).unwrap();
        let rom = s(&sandbox.path().join("Game.chd"));
        fs::write(&rom, "compressed").unwrap();
        let e = crate::store::upsert_user_emulator(
            &paths,
            UpsertUserEmulatorArgs {
                definition_id: "duckstation".into(),
                name: "DuckStation".into(),
                executable: "/opt/duck".into(),
                launch_kind: "native".into(),
                ..Default::default()
            },
        )
        .unwrap();
        json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |f| {
            f.games.insert(
                "g".into(),
                heronite_data::models::LocalGameMeta {
                    app_name: "g".into(),
                    runner: "sideload".into(),
                    launch_kind: "emulator".into(),
                    emulator_id: Some(e.id.clone()),
                    emulator_profile_id: Some(e.profiles[0].id.clone()),
                    roms: Some(vec![heronite_data::models::LocalGameRom { name: None, path: rom.clone(), extra: Default::default() }]),
                    ..Default::default()
                },
            );
            Ok(())
        })
        .unwrap();
        // Nothing listed before any session.
        let before = list_emulator_save_states(&paths, &env, "g", None, false).unwrap();
        assert!(before.supported && before.states.is_empty());
        let started = SystemTime::now();
        fs::write(states.join("SLUS-00594_resume.sav"), "x").unwrap();
        let key = remember_save_state_key(&paths, &env, "g", Some(&rom), started).unwrap();
        assert_eq!(key.as_deref(), Some("SLUS-00594"));
        let again = remember_save_state_key(&paths, &env, "g", Some(&rom), started).unwrap();
        assert_eq!(again, None);
        let after = list_emulator_save_states(&paths, &env, "g", None, false).unwrap();
        assert_eq!(after.states.len(), 1);
        assert_eq!(after.states[0].kind, SaveStateKind::Resume);
        // Two different keys in one session: ambiguous, nothing learned.
        fs::write(states.join("SLES-11111_1.sav"), "x").unwrap();
        let m: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap();
        assert_eq!(m.games["g"].save_state_keys.as_ref().unwrap()[&rom], "SLUS-00594");
    }
}
