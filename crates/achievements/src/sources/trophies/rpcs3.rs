//! RPCS3: a game's trophies are in
//! `<config>/dev_hdd0/home/<user>/trophy/<NP comm id>/`, where the emulator
//! installs the game's trophy pack the first time it plays: `TROPCONF.SFM`
//! (the list, in the console's language), `TROPUSR.DAT` (what is unlocked,
//! and when) and `TROP000.PNG`… (the pictures).
//!
//! The game says its comm id in its `TROPDIR` folder
//! (`PS3_GAME/TROPDIR/NPWR01234_00/TROPHY.TRP`); without it (an ISO image,
//! a game whose folder moved), the list whose name is the game's is used.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::psxml::{self, TrophyConf};
use super::{entries, from_rtc_tick, game_dirs, modified, read_capped, same_title, Env, Install, MAX_XML};
use crate::model::Source;
use crate::sources::Fetch;
use crate::target::TrophyTarget;

/// `TROPUSR.DAT` of a game with the most trophies is a few KiB.
const MAX_TROPUSR: u64 = 1 << 20;
const FLATPAK_ID: &str = "net.rpcs3.RPCS3";

/// The folders RPCS3 may keep its files in: a portable copy's own folder
/// (Windows always), then its Flatpak's, then `~/.config/rpcs3`.
pub fn config_roots(env: &Env, installs: &[Install]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if !out.contains(&p) {
            out.push(p);
        }
    };
    for i in installs {
        let portable = |d: &&PathBuf| d.join("dev_hdd0").is_dir() || d.join("config/vfs.yml").is_file() || d.join("vfs.yml").is_file();
        if let Some(dir) = i.exe_dir.as_ref().filter(portable) {
            push(dir.clone());
        }
        if let Some(id) = &i.flatpak_id {
            push(env.flatpak(id, "config").join("rpcs3"));
        }
    }
    push(env.flatpak(FLATPAK_ID, "config").join("rpcs3"));
    for c in &env.config_homes {
        push(c.join("rpcs3"));
    }
    out
}

/// `dev_hdd0` of a config folder: where `vfs.yml` (in `config/` on
/// Windows) puts it, else inside.
fn hdd0_of(root: &Path) -> PathBuf {
    let default = root.join("dev_hdd0");
    let vfs = [root.join("config/vfs.yml"), root.join("vfs.yml")];
    let Some(bytes) = vfs.iter().find_map(|f| read_capped(f, 1 << 16).ok()) else { return default };
    let text = String::from_utf8_lossy(&bytes);
    let Some(value) = text.lines().find_map(|l| l.trim().strip_prefix("/dev_hdd0/:")) else { return default };
    let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
    if value.is_empty() {
        return default;
    }
    let mut emulator_dir = root.to_string_lossy().into_owned();
    if !emulator_dir.ends_with(std::path::MAIN_SEPARATOR) {
        emulator_dir.push(std::path::MAIN_SEPARATOR);
    }
    let path = PathBuf::from(value.replace("$(EmulatorDir)", &emulator_dir));
    if path.is_absolute() {
        path
    } else {
        default
    }
}

/// Every user's trophy folder (`home/00000001/trophy`) of every config folder.
fn trophy_dirs(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in roots {
        for user in entries(&hdd0_of(root).join("home")) {
            let is_user = user.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.len() == 8 && n.bytes().all(|b| b.is_ascii_digit()));
            let dir = user.join("trophy");
            if is_user && dir.is_dir() {
                out.push(dir);
            }
        }
    }
    out
}

/// The comm ids the game's `TROPDIR` names.
pub fn game_comm_ids(roms: &[PathBuf]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for dir in game_dirs(roms, 3) {
        for tropdir in [dir.join("TROPDIR"), dir.join("PS3_GAME/TROPDIR")] {
            for e in entries(&tropdir) {
                if let Some(name) = e.file_name().and_then(|n| n.to_str()).filter(|n| psxml::is_comm_id(n)) {
                    if !out.iter().any(|o| o == name) {
                        out.push(name.to_string());
                    }
                }
            }
        }
    }
    out
}

/// The game's trophy folder: one its `TROPDIR` names, else the one whose
/// list has the game's name; the one played last when there are several.
fn locate(trophy_dirs: &[PathBuf], target: &TrophyTarget) -> Option<PathBuf> {
    let ids = game_comm_ids(&target.roms);
    let by_id = trophy_dirs.iter().flat_map(|d| ids.iter().map(move |id| d.join(id))).filter(|d| d.join("TROPCONF.SFM").is_file());
    let found: Vec<PathBuf> = by_id.collect();
    let found = if found.is_empty() && !target.title.trim().is_empty() {
        trophy_dirs
            .iter()
            .flat_map(|d| entries(d))
            .filter(|d| d.file_name().and_then(|n| n.to_str()).is_some_and(psxml::is_comm_id))
            .filter(|d| {
                read_capped(&d.join("TROPCONF.SFM"), MAX_XML)
                    .ok()
                    .and_then(|b| psxml::parse(&b).ok())
                    .is_some_and(|c| same_title(&c.title, &target.title))
            })
            .collect()
    } else {
        found
    };
    let last = |d: &PathBuf| modified(&d.join("TROPUSR.DAT")).or_else(|| modified(&d.join("TROPCONF.SFM")));
    found.into_iter().max_by_key(last)
}

/// One trophy's record in `TROPUSR.DAT`: unlocked, and when.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unlock {
    pub unlocked: bool,
    /// The two times RPCS3 writes (the same, by its code), the second first.
    pub ticks: [u64; 2],
}

fn be_u32(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes(s.try_into().ok()?))
}

fn be_u64(b: &[u8], at: usize) -> Option<u64> {
    let s = b.get(at..at.checked_add(8)?)?;
    Some(u64::from_be_bytes(s.try_into().ok()?))
}

/// `TROPUSR.DAT` (big-endian, RPCS3's `TROPUSR.h`): a 0x30-byte header
/// (magic `818F54AD`, the table count at 0x08), then 0x20-byte table
/// headers (type, entry size, ?, count, offset: u64). Table 6 has one
/// 0x60-byte entry per trophy: its id at 0x10, its state at 0x14 (1 =
/// unlocked) and its time at 0x28 (a `CellRtcTick`).
pub fn parse_tropusr(b: &[u8]) -> Result<HashMap<u32, Unlock>, String> {
    const MAGIC: u32 = 0x818F_54AD;
    const ENTRY6: usize = 0x60;
    const MAX_TABLES: u32 = 64;
    const MAX_TROPHIES: u32 = 4096;
    if be_u32(b, 0) != Some(MAGIC) {
        return Err("TROPUSR.DAT: not a trophy record".into());
    }
    let tables = be_u32(b, 8).ok_or("TROPUSR.DAT: cut short")?;
    if tables > MAX_TABLES {
        return Err(format!("TROPUSR.DAT: {tables} tables"));
    }
    let mut out = HashMap::new();
    for t in 0..tables as usize {
        let at = 0x30 + t * 0x20;
        let kind = be_u32(b, at).ok_or("TROPUSR.DAT: cut short")?;
        if kind != 6 {
            continue;
        }
        let count = be_u32(b, at + 0x0C).ok_or("TROPUSR.DAT: cut short")?;
        if count > MAX_TROPHIES {
            return Err(format!("TROPUSR.DAT: {count} trophies"));
        }
        let offset = usize::try_from(be_u64(b, at + 0x10).ok_or("TROPUSR.DAT: cut short")?).map_err(|_| "TROPUSR.DAT: bad offset")?;
        for i in 0..count as usize {
            // Past the end (and an offset near `usize::MAX`) is a short read.
            let entry =
                i.checked_mul(ENTRY6).and_then(|o| o.checked_add(offset)).filter(|e| *e < b.len()).ok_or("TROPUSR.DAT: bad offset")?;
            let (Some(id), Some(state), Some(t1), Some(t2)) =
                (be_u32(b, entry + 0x10), be_u32(b, entry + 0x14), be_u64(b, entry + 0x20), be_u64(b, entry + 0x28))
            else {
                return Err("TROPUSR.DAT: cut short".into());
            };
            out.insert(id, Unlock { unlocked: state == 1, ticks: [t2, t1] });
        }
    }
    Ok(out)
}

/// The set of a trophy folder.
pub fn read(dir: &Path, game_title: &str) -> Fetch {
    let conf: TrophyConf = match read_capped(&dir.join("TROPCONF.SFM"), MAX_XML).and_then(|b| psxml::parse(&b)) {
        Ok(c) => c,
        Err(e) => return Fetch::Failed(format!("{}: {e}", dir.display())),
    };
    if conf.trophies.is_empty() {
        return Fetch::None;
    }
    let usr_file = dir.join("TROPUSR.DAT");
    // RPCS3 writes the record when the game first opens its trophies:
    // until then, nothing is unlocked.
    let unlocks = if usr_file.is_file() {
        match read_capped(&usr_file, MAX_TROPUSR).and_then(|b| parse_tropusr(&b)) {
            Ok(u) => u,
            Err(e) => return Fetch::Failed(format!("{}: {e}", dir.display())),
        }
    } else {
        HashMap::new()
    };
    let set = psxml::build(
        Source::Rpcs3,
        &conf,
        game_title,
        |t| unlocks.get(&t.id).filter(|u| u.unlocked).map_or((false, None), |u| (true, u.ticks.iter().find_map(|t| from_rtc_tick(*t)))),
        |id| psxml::icon_in(dir, id),
    );
    Fetch::Set(set)
}

pub fn fetch(env: &Env, installs: &[Install], target: &TrophyTarget) -> Fetch {
    match locate(&trophy_dirs(&config_roots(env, installs)), target) {
        Some(dir) => read(&dir, &target.title),
        None => Fetch::None,
    }
}

pub fn watched_files(env: &Env, installs: &[Install], target: &TrophyTarget) -> Vec<PathBuf> {
    let dirs = trophy_dirs(&config_roots(env, installs));
    match locate(&dirs, target) {
        Some(dir) => vec![dir.join("TROPUSR.DAT")],
        // Not played yet: the folder appears with the first session, under
        // the comm id the game names.
        None => {
            let ids = game_comm_ids(&target.roms);
            dirs.iter().flat_map(|d| ids.iter().map(move |id| d.join(id).join("TROPUSR.DAT"))).collect()
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::Grade;
    use crate::target::TrophyEmulator;

    /// A `TROPUSR.DAT` with table 4 (ignored) and table 6 holding `(id,
    /// unlocked, tick)`.
    pub fn tropusr(trophies: &[(u32, bool, u64)]) -> Vec<u8> {
        let mut b = vec![0u8; 0x30];
        b[0..4].copy_from_slice(&0x818F_54ADu32.to_be_bytes());
        b[8..12].copy_from_slice(&2u32.to_be_bytes());
        let tables_end = 0x30 + 2 * 0x20;
        let t4 = tables_end;
        let t6 = t4 + trophies.len() * 0x50;
        let header = |kind: u32, size: u32, offset: usize| {
            let mut h = vec![0u8; 0x20];
            h[0..4].copy_from_slice(&kind.to_be_bytes());
            h[4..8].copy_from_slice(&size.to_be_bytes());
            h[8..12].copy_from_slice(&1u32.to_be_bytes());
            h[12..16].copy_from_slice(&(trophies.len() as u32).to_be_bytes());
            h[16..24].copy_from_slice(&(offset as u64).to_be_bytes());
            h
        };
        b.extend(header(4, 0x50, t4));
        b.extend(header(6, 0x60, t6));
        for (id, _, _) in trophies {
            let mut e = vec![0u8; 0x50];
            e[0..4].copy_from_slice(&4u32.to_be_bytes());
            e[0x10..0x14].copy_from_slice(&id.to_be_bytes());
            b.extend(e);
        }
        for (id, on, tick) in trophies {
            let mut e = vec![0u8; 0x60];
            e[0..4].copy_from_slice(&6u32.to_be_bytes());
            e[0x10..0x14].copy_from_slice(&id.to_be_bytes());
            e[0x14..0x18].copy_from_slice(&u32::from(*on).to_be_bytes());
            e[0x20..0x28].copy_from_slice(&tick.to_be_bytes());
            e[0x28..0x30].copy_from_slice(&tick.to_be_bytes());
            b.extend(e);
        }
        b
    }

    pub const CONF: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<trophyconf version="1.0" policy="large">
<npcommid>NPWR00001_00</npcommid>
<title-name>Demon's Souls</title-name>
<trophy id="000" hidden="no" ttype="P" pid="-1"><name>Demon's Souls</name><detail>All trophies.</detail></trophy>
<trophy id="001" hidden="no" ttype="B" pid="000"><name>Rescuer</name><detail>Rescue someone.</detail></trophy>
<trophy id="002" hidden="yes" ttype="G" pid="000"><name>Secret</name><detail>Spoiler.</detail></trophy>
</trophyconf>"#;

    const TICK_2023: u64 = 62_135_596_800_000_000 + 1_700_000_000_000_000;

    #[test]
    fn reads_the_record() {
        let b = tropusr(&[(0, false, 0), (1, true, TICK_2023), (2, false, 0)]);
        let u = parse_tropusr(&b).unwrap();
        assert_eq!(u.len(), 3);
        assert_eq!(u[&1], Unlock { unlocked: true, ticks: [TICK_2023, TICK_2023] });
        assert!(!u[&2].unlocked);
    }

    #[test]
    fn a_damaged_record_fails_and_never_panics() {
        let b = tropusr(&[(0, true, TICK_2023), (1, true, TICK_2023)]);
        for end in 0..b.len() {
            let _ = parse_tropusr(&b[..end]);
        }
        // Cut inside the last trophy's time.
        assert!(parse_tropusr(&b[..b.len() - 0x34]).is_err());
        assert!(parse_tropusr(b"").is_err());
        for m in crate::sources::trophies::mutants(&b, 5000) {
            let _ = parse_tropusr(&m);
        }
        let mut wild = b.clone();
        wild[0x30 + 0x20 + 0x10..0x30 + 0x20 + 0x18].copy_from_slice(&u64::MAX.to_be_bytes());
        assert!(parse_tropusr(&wild).is_err(), "an offset past the end");
        let mut many = b;
        many[0x30 + 0x20 + 0x0C..0x30 + 0x20 + 0x10].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(parse_tropusr(&many).is_err());
    }

    /// RPCS3's folders under a fake home: a game folder whose `TROPDIR`
    /// names the pack, and the pack installed for user 00000001.
    fn setup(home: &Path, record: Option<Vec<u8>>) -> (Env, TrophyTarget, PathBuf) {
        let env = Env { home: home.to_path_buf(), config_homes: vec![home.join(".config")], ..Default::default() };
        let game = home.join("games/BLUS30443");
        std::fs::create_dir_all(game.join("PS3_GAME/USRDIR")).unwrap();
        std::fs::create_dir_all(game.join("PS3_GAME/TROPDIR/NPWR00001_00")).unwrap();
        let eboot = game.join("PS3_GAME/USRDIR/EBOOT.BIN");
        std::fs::write(&eboot, b"x").unwrap();
        let pack = home.join(".config/rpcs3/dev_hdd0/home/00000001/trophy/NPWR00001_00");
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join("TROPCONF.SFM"), CONF).unwrap();
        std::fs::write(pack.join("TROP001.PNG"), b"png").unwrap();
        if let Some(r) = record {
            std::fs::write(pack.join("TROPUSR.DAT"), r).unwrap();
        }
        let target = TrophyTarget { emulator: TrophyEmulator::Rpcs3, roms: vec![eboot], title: "Demon's Souls".into(), emulator_id: None };
        (env, target, pack)
    }

    #[test]
    fn finds_the_games_pack_and_reads_it() {
        let d = tempfile::tempdir().unwrap();
        let (env, target, pack) = setup(d.path(), Some(tropusr(&[(0, false, 0), (1, true, TICK_2023), (2, false, 0)])));
        let Fetch::Set(set) = fetch(&env, &[], &target) else { panic!("expected a set") };
        assert_eq!((set.source, set.external_id.as_str(), set.total(), set.unlocked()), (Source::Rpcs3, "NPWR00001_00", 3, 1));
        let rescuer = &set.achievements[1];
        assert_eq!(rescuer.unlocked_at.unwrap().timestamp(), 1_700_000_000);
        assert_eq!(rescuer.grade, Some(Grade::Bronze));
        assert_eq!(Path::new(&rescuer.icon), pack.join("TROP001.PNG"));
        assert!(set.achievements[2].hidden && set.achievements[0].icon.is_empty());
        assert_eq!(watched_files(&env, &[], &target), [pack.join("TROPUSR.DAT")]);
    }

    #[test]
    fn before_the_first_unlock_everything_is_locked_and_a_moved_game_matches_by_name() {
        let d = tempfile::tempdir().unwrap();
        let (env, mut target, _) = setup(d.path(), None);
        target.roms = vec![d.path().join("elsewhere/game.iso")];
        let Fetch::Set(set) = fetch(&env, &[], &target) else { panic!("expected a set") };
        assert_eq!((set.total(), set.unlocked()), (3, 0));
        target.title = "Another game".into();
        assert_eq!(fetch(&env, &[], &target), Fetch::None);
    }

    #[test]
    fn a_damaged_record_is_a_failure_of_this_source() {
        let d = tempfile::tempdir().unwrap();
        let (env, target, _) = setup(d.path(), Some(b"garbage".to_vec()));
        assert!(matches!(fetch(&env, &[], &target), Fetch::Failed(_)));
    }

    #[test]
    fn vfs_moves_dev_hdd0() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("rpcs3");
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(hdd0_of(&root), root.join("dev_hdd0"));
        std::fs::write(root.join("vfs.yml"), "/games/: /g/\n/dev_hdd0/: \"$(EmulatorDir)hdd/\"\n").unwrap();
        assert_eq!(hdd0_of(&root), root.join("hdd/"));
        let elsewhere = d.path().join("ps3/hdd0");
        std::fs::write(root.join("vfs.yml"), format!("/dev_hdd0: x\n/dev_hdd0/: {}\n", elsewhere.display())).unwrap();
        assert_eq!(hdd0_of(&root), elsewhere);
        std::fs::write(root.join("vfs.yml"), "/dev_hdd0/: relative\n").unwrap();
        assert_eq!(hdd0_of(&root), root.join("dev_hdd0"));
    }
}
