//! Xenia (Canary and its forks): each profile keeps, per game, a GPD file
//! with the game's achievements, their pictures and what it unlocked:
//! `<content>/<XUID>/FFFE07D1/00010000/<XUID>/<title id>.gpd`, written the
//! first time the game runs. `<content>` is `content` inside the storage
//! root: the executable's folder for a portable copy (`portable.txt`, and
//! the Windows build's default), else `~/.local/share/Xenia` or
//! `Documents\Xenia`.
//!
//! The game's title id comes from its `default.xex`; without it (a disc
//! image), from the profile's list of played games (`FFFE07D1.gpd`) by name.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::Map;

use super::{entries, game_dirs, newest, plausible, read_capped, same_title, Env, Install};
use crate::model::{Achievement, AchievementSet, Source};
use crate::sources::Fetch;
use crate::target::TrophyTarget;

/// A game's GPD with every picture is a few MiB at most.
const MAX_GPD: u64 = 32 << 20;
/// The dashboard's title id: its GPD lists the games played.
const DASHBOARD: u32 = 0xFFFE_07D1;
const SECTION_ACHIEVEMENT: u16 = 1;
const SECTION_IMAGE: u16 = 2;
const SECTION_TITLE: u16 = 4;
const ACHIEVED: u32 = 0x2_0000;
const ACHIEVED_ONLINE: u32 = 0x1_0000;
const SHOW_UNACHIEVED: u32 = 0x8;
/// Entry slots a GPD may have: a game has at most a few hundred entries.
const MAX_SLOTS: u32 = 16_384;
/// Achievements (or played games) read from one GPD.
const MAX_ITEMS: usize = 4_096;
/// UTF-16 units of one string (names and descriptions are short).
const MAX_STRING: usize = 4_096;
/// A picture bigger than this is not an achievement's icon.
const MAX_ICON: usize = 2 << 20;

fn be_u16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn be_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn be_u64(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(at..at.checked_add(8)?)?.try_into().ok()?))
}

/// A NUL-ended UTF-16BE string at `at` (at most [`MAX_STRING`] units kept),
/// and where the next one starts.
fn utf16_at(b: &[u8], at: usize) -> (String, usize) {
    let mut units = Vec::new();
    let mut i = at;
    while let Some(u) = be_u16(b, i) {
        i += 2;
        if u == 0 {
            break;
        }
        if units.len() < MAX_STRING {
            units.push(u);
        }
    }
    (String::from_utf16_lossy(&units), i)
}

/// Sync list and sync data entries (`0x1_0000_0000`, `0x2_0000_0000`) share
/// the achievement and title sections: real entries have 32-bit ids.
fn is_record(id: u64) -> bool {
    id <= u64::from(u32::MAX)
}

/// An XDBF file (Xenia's `xdbf_io.h`, big-endian): a 0x18-byte header
/// (`XDBF`, version, entry slots, entries used, free slots, free used),
/// the entry table (0x12 bytes each: section u16, id u64, offset u32, size
/// u32), the free table (8 bytes per slot), then the data.
#[derive(Debug)]
pub struct Xdbf<'a> {
    /// `(section, id, data)`, in the file's order.
    entries: Vec<(u16, u64, &'a [u8])>,
    /// The pictures by id (the first of a repeated id).
    images: HashMap<u64, &'a [u8]>,
}

impl<'a> Xdbf<'a> {
    pub fn parse(b: &'a [u8]) -> Result<Xdbf<'a>, String> {
        if b.get(..4) != Some(b"XDBF".as_slice()) {
            return Err("not a GPD file".into());
        }
        let field = |at| be_u32(b, at).ok_or_else(|| "GPD: cut short".to_string());
        let (slots, used, free_slots) = (field(8)?, field(12)?, field(16)?);
        if slots > MAX_SLOTS || free_slots > MAX_SLOTS || used > slots {
            return Err("GPD: a broken header".into());
        }
        let data_start = 0x18 + 0x12 * slots as usize + 8 * free_slots as usize;
        let mut entries = Vec::new();
        for i in 0..used as usize {
            let at = 0x18 + i * 0x12;
            let (Some(section), Some(id), Some(offset), Some(size)) =
                (be_u16(b, at), be_u64(b, at + 2), be_u32(b, at + 10), be_u32(b, at + 14))
            else {
                return Err("GPD: cut short".into());
            };
            let start = data_start.checked_add(offset as usize).ok_or("GPD: bad offset")?;
            let data = start.checked_add(size as usize).and_then(|end| b.get(start..end)).ok_or("GPD: an entry past the end")?;
            entries.push((section, id, data));
        }
        let mut images = HashMap::new();
        for (_, id, data) in entries.iter().filter(|(s, _, _)| *s == SECTION_IMAGE) {
            images.entry(*id).or_insert(*data);
        }
        Ok(Xdbf { entries, images })
    }

    fn section(&self, section: u16) -> impl Iterator<Item = (u64, &'a [u8])> + '_ {
        self.entries.iter().filter(move |(s, _, _)| *s == section).map(|(_, id, data)| (*id, *data))
    }

    pub fn image(&self, id: u64) -> Option<&'a [u8]> {
        self.images.get(&id).copied()
    }

    /// The games a dashboard GPD lists: `(title id, name)`. Each entry is
    /// a 0x28-byte record (the title id first), then its name.
    pub fn titles(&self) -> Vec<(u32, String)> {
        self.section(SECTION_TITLE)
            .filter(|(id, _)| is_record(*id))
            .take(MAX_ITEMS)
            .filter_map(|(_, d)| Some((be_u32(d, 0)?, utf16_at(d, 0x28).0)))
            .collect()
    }
}

/// One achievement of a game's GPD (`X_XDBF_GPD_ACHIEVEMENT`): a 0x1C-byte
/// record (size, id, image id, gamerscore, flags, unlock time as a
/// FILETIME), then its name, its description and its locked description.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GpdAchievement {
    pub id: u32,
    pub image_id: u32,
    pub gamerscore: u32,
    pub flags: u32,
    pub unlock_time: u64,
    pub name: String,
    pub description: String,
    pub locked_description: String,
}

pub fn achievements(x: &Xdbf) -> Vec<GpdAchievement> {
    let mut seen = HashSet::new();
    x.section(SECTION_ACHIEVEMENT)
        .filter(|(id, d)| is_record(*id) && be_u32(d, 0) == Some(0x1C))
        .take(MAX_ITEMS)
        .filter_map(|(_, d)| {
            let (name, at) = utf16_at(d, 0x1C);
            let (description, at) = utf16_at(d, at);
            let (locked_description, _) = utf16_at(d, at);
            Some(GpdAchievement {
                id: be_u32(d, 4)?,
                image_id: be_u32(d, 8)?,
                gamerscore: be_u32(d, 12)?,
                flags: be_u32(d, 16)?,
                unlock_time: be_u64(d, 20)?,
                name,
                description,
                locked_description,
            })
        })
        .filter(|a| seen.insert(a.id))
        .collect()
}

/// A FILETIME (100 ns since 1601-01-01 UTC); `0` is no time.
fn from_filetime(ft: u64) -> Option<DateTime<Utc>> {
    const UNIX_EPOCH_SECS: u64 = 11_644_473_600;
    let secs = (ft / 10_000_000).checked_sub(UNIX_EPOCH_SECS)?;
    plausible(DateTime::from_timestamp(i64::try_from(secs).ok()?, 0)?)
}

/// The title id of an XEX2 executable: in its execution info (optional
/// header `0x00040006`), at 0x0C. The header count is at 0x14, the
/// `(key, value)` pairs from 0x18.
pub fn xex_title_id(b: &[u8]) -> Option<u32> {
    if b.get(..4) != Some(b"XEX2".as_slice()) {
        return None;
    }
    let count = be_u32(b, 0x14)?.min(256) as usize;
    (0..count).find_map(|i| {
        let at = 0x18 + i * 8;
        (be_u32(b, at)? == 0x0004_0006).then_some(())?;
        let info = be_u32(b, at + 4)? as usize;
        be_u32(b, info.checked_add(0x0C)?).filter(|id| *id != 0)
    })
}

/// The first `max` bytes of a file.
fn read_prefix(path: &Path, max: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(max).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// The title id of the game's `default.xex` (or the `.xex` played).
pub fn game_title_id(roms: &[PathBuf]) -> Option<u32> {
    let xex = roms.iter().filter(|r| r.extension().is_some_and(|e| e.eq_ignore_ascii_case("xex"))).cloned();
    let defaults = game_dirs(roms, 1).into_iter().map(|d| d.join("default.xex"));
    xex.chain(defaults).filter(|f| f.is_file()).find_map(|f| xex_title_id(&read_prefix(&f, 1 << 20)?))
}

/// The storage roots Xenia may use: a portable copy's folder (its
/// `content`), then the user folders.
pub fn content_roots(env: &Env, installs: &[Install]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if !out.contains(&p) && p.is_dir() {
            out.push(p);
        }
    };
    for i in installs {
        if let Some(dir) = &i.exe_dir {
            push(dir.join("content"));
        }
    }
    for d in &env.data_homes {
        push(d.join("Xenia/content"));
    }
    if let Some(docs) = &env.documents {
        push(docs.join("Xenia/content"));
    }
    out
}

/// The profiles' folders: `<content>/<XUID>/FFFE07D1/00010000/<XUID>`.
fn profiles(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in roots {
        for user in entries(root) {
            let Some(xuid) =
                user.file_name().and_then(|n| n.to_str()).filter(|n| n.len() == 16 && n.bytes().all(|b| b.is_ascii_hexdigit()))
            else {
                continue;
            };
            let dir = user.join("FFFE07D1/00010000").join(xuid);
            if dir.is_dir() {
                out.push(dir);
            }
        }
    }
    out
}

fn gpd_name(title_id: u32) -> String {
    format!("{title_id:08X}.gpd")
}

/// The title id of the game named `title` in a profile's list of games.
fn title_id_by_name(profiles: &[PathBuf], title: &str) -> Option<u32> {
    profiles.iter().find_map(|p| {
        let bytes = read_capped(&p.join(gpd_name(DASHBOARD)), MAX_GPD).ok()?;
        let x = Xdbf::parse(&bytes).ok()?;
        x.titles().into_iter().find(|(_, name)| same_title(name, title)).map(|(id, _)| id)
    })
}

fn title_id_of(profiles: &[PathBuf], target: &TrophyTarget) -> Option<u32> {
    game_title_id(&target.roms).or_else(|| (!target.title.trim().is_empty()).then(|| title_id_by_name(profiles, &target.title)).flatten())
}

/// The picture of an achievement, written once under the app's
/// achievements folder (the GPD keeps it inside), when it is a PNG.
fn icon_file(icons: &Path, title_id: u32, x: &Xdbf, image_id: u32) -> Option<PathBuf> {
    let png = x.image(u64::from(image_id)).filter(|d| d.starts_with(b"\x89PNG") && d.len() <= MAX_ICON)?;
    let file = icons.join(format!("{title_id:08X}")).join(format!("{image_id}.png"));
    let same = std::fs::metadata(&file).ok().is_some_and(|m| m.len() == png.len() as u64);
    if !same {
        let dir = file.parent()?;
        std::fs::create_dir_all(dir).ok()?;
        heronite_data::temps::write_atomic(&file, png, None).map_err(|e| log::debug!("emulator trophies: {}: {e}", file.display())).ok()?;
    }
    Some(file)
}

pub fn read(gpd: &Path, title_id: u32, icons: &Path) -> Fetch {
    let bytes = match read_capped(gpd, MAX_GPD) {
        Ok(b) => b,
        Err(e) => return Fetch::Failed(e),
    };
    let x = match Xdbf::parse(&bytes) {
        Ok(x) => x,
        Err(e) => return Fetch::Failed(format!("{}: {e}", gpd.display())),
    };
    let list = achievements(&x);
    if list.is_empty() {
        return Fetch::None;
    }
    let achievements = list
        .iter()
        .map(|a| {
            let unlocked = a.flags & (ACHIEVED | ACHIEVED_ONLINE) != 0;
            Achievement {
                id: a.id.to_string(),
                name: if a.name.is_empty() { format!("#{}", a.id) } else { a.name.clone() },
                description: if a.description.is_empty() { a.locked_description.clone() } else { a.description.clone() },
                hidden: a.flags & SHOW_UNACHIEVED == 0,
                icon: icon_file(icons, title_id, &x, a.image_id).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
                unlocked,
                unlocked_at: if unlocked { from_filetime(a.unlock_time) } else { None },
                // Gamerscore: 0..=1000 a game.
                points: Some(a.gamerscore.min(1000)),
                ..Default::default()
            }
        })
        .collect();
    Fetch::Set(AchievementSet {
        source: Source::Xenia,
        external_id: format!("{title_id:08X}"),
        title: None,
        fetched_at: Utc::now(),
        achievements,
        groups: Vec::new(),
        unknown: Map::new(),
    })
}

/// The game's GPD of the profile that played it last.
fn locate(profiles: &[PathBuf], title_id: u32) -> Option<PathBuf> {
    newest(profiles.iter().map(|p| p.join(gpd_name(title_id))))
}

/// `achievements_dir`: the app's folder of achievement copies; the
/// pictures go in `_trophy-icons/xenia` inside it.
pub fn fetch(env: &Env, installs: &[Install], target: &TrophyTarget, achievements_dir: &Path) -> Fetch {
    let profiles = profiles(&content_roots(env, installs));
    let Some(title_id) = title_id_of(&profiles, target) else { return Fetch::None };
    match locate(&profiles, title_id) {
        Some(gpd) => read(&gpd, title_id, &achievements_dir.join("_trophy-icons/xenia")),
        None => Fetch::None,
    }
}

pub fn watched_files(env: &Env, installs: &[Install], target: &TrophyTarget) -> Vec<PathBuf> {
    let profiles = profiles(&content_roots(env, installs));
    match title_id_of(&profiles, target) {
        Some(id) => profiles.iter().map(|p| p.join(gpd_name(id))).collect(),
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::TrophyEmulator;

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().chain([0]).flat_map(u16::to_be_bytes).collect()
    }

    /// An XDBF with `entries` of `(section, id, data)`.
    pub fn xdbf(entries: &[(u16, u64, Vec<u8>)]) -> Vec<u8> {
        let slots = entries.len() as u32 + 2;
        let free_slots = 2u32;
        let mut b = b"XDBF".to_vec();
        for v in [0x10000u32, slots, entries.len() as u32, free_slots, 1] {
            b.extend(v.to_be_bytes());
        }
        let mut data: Vec<u8> = Vec::new();
        let mut table: Vec<u8> = Vec::new();
        for (section, id, d) in entries {
            table.extend(section.to_be_bytes());
            table.extend(id.to_be_bytes());
            table.extend((data.len() as u32).to_be_bytes());
            table.extend((d.len() as u32).to_be_bytes());
            data.extend(d);
        }
        table.resize(0x12 * slots as usize, 0);
        b.extend(table);
        b.extend(vec![0u8; 8 * free_slots as usize]);
        b.extend(data);
        b
    }

    fn achievement(id: u32, image: u32, score: u32, flags: u32, time: u64, name: &str, desc: &str, locked: &str) -> Vec<u8> {
        let mut d = Vec::new();
        for v in [0x1Cu32, id, image, score, flags] {
            d.extend(v.to_be_bytes());
        }
        d.extend(time.to_be_bytes());
        d.extend(utf16(name));
        d.extend(utf16(desc));
        d.extend(utf16(locked));
        d
    }

    /// FILETIME of 2023-11-14 22:13:20 UTC.
    const FT_2023: u64 = (11_644_473_600 + 1_700_000_000) * 10_000_000;

    fn game_gpd() -> Vec<u8> {
        xdbf(&[
            // The sync list and data share the section, and a record of
            // another kind.
            (1, 0x1_0000_0000, vec![0u8; 0x40]),
            (1, 0x2_0000_0000, [0x1Cu32.to_be_bytes().to_vec(), vec![0u8; 0x30]].concat()),
            (1, 9, vec![0u8; 0x30]),
            (1, 1, achievement(1, 10, 20, ACHIEVED | SHOW_UNACHIEVED | 1, FT_2023, "First", "Did it.", "Do it.")),
            (1, 2, achievement(2, 11, 50, 0, 0, "Secret", "", "Keep going.")),
            (2, 10, b"\x89PNG\r\n\x1a\nimage".to_vec()),
            (2, 11, b"not a png".to_vec()),
        ])
    }

    #[test]
    fn reads_a_games_gpd() {
        let d = tempfile::tempdir().unwrap();
        let gpd = d.path().join("4D5307E6.gpd");
        std::fs::write(&gpd, game_gpd()).unwrap();
        let icons = d.path().join("icons");
        let Fetch::Set(set) = read(&gpd, 0x4D53_07E6, &icons) else { panic!("expected a set") };
        assert_eq!((set.source, set.external_id.as_str(), set.total(), set.unlocked()), (Source::Xenia, "4D5307E6", 2, 1));
        assert_eq!(set.points(), Some((20, 70)));
        let (a, b) = (&set.achievements[0], &set.achievements[1]);
        assert_eq!((a.name.as_str(), a.description.as_str(), a.hidden), ("First", "Did it.", false));
        assert_eq!(a.unlocked_at.unwrap().timestamp(), 1_700_000_000);
        assert_eq!(std::fs::read(&a.icon).unwrap(), b"\x89PNG\r\n\x1a\nimage");
        assert!(b.hidden && !b.unlocked && b.icon.is_empty());
        assert_eq!(b.description, "Keep going.");
        // Read again: the picture is not written again.
        assert!(matches!(read(&gpd, 0x4D53_07E6, &icons), Fetch::Set(_)));
    }

    #[test]
    fn a_damaged_gpd_fails_and_never_panics() {
        let b = game_gpd();
        for end in 0..b.len() {
            if let Ok(x) = Xdbf::parse(&b[..end]) {
                let _ = achievements(&x);
                let _ = x.titles();
            }
        }
        for m in crate::sources::trophies::mutants(&b, 5000) {
            if let Ok(x) = Xdbf::parse(&m) {
                let _ = (achievements(&x), x.titles(), x.image(10));
            }
            let _ = xex_title_id(&m);
        }
        assert!(Xdbf::parse(b"XDBF").is_err());
        assert!(Xdbf::parse(b"nope").is_err());
        let mut wild = b.clone();
        wild[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(Xdbf::parse(&wild).is_err());
        let mut past = b;
        past[0x18 + 10..0x18 + 14].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(Xdbf::parse(&past).is_err());
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("x.gpd"), b"garbage").unwrap();
        assert!(matches!(read(&d.path().join("x.gpd"), 1, d.path()), Fetch::Failed(_)));
    }

    fn xex(title_id: u32) -> Vec<u8> {
        let mut b = b"XEX2".to_vec();
        b.extend([0u8; 0x10]);
        b.extend(2u32.to_be_bytes());
        b.extend(0x0002_0104u32.to_be_bytes());
        b.extend(0u32.to_be_bytes());
        b.extend(0x0004_0006u32.to_be_bytes());
        b.extend(0x30u32.to_be_bytes());
        b.resize(0x30, 0);
        b.extend([0u8; 0x0C]);
        b.extend(title_id.to_be_bytes());
        b
    }

    #[test]
    fn the_title_id_comes_from_the_xex() {
        assert_eq!(xex_title_id(&xex(0x4D53_07E6)), Some(0x4D53_07E6));
        let b = xex(0x4D53_07E6);
        for end in 0..b.len() {
            let _ = xex_title_id(&b[..end]);
        }
        assert_eq!(xex_title_id(b"XEX1"), None);
    }

    #[test]
    fn finds_the_profiles_gpd_by_the_xex_then_by_name() {
        let d = tempfile::tempdir().unwrap();
        let env = Env { home: d.path().into(), data_homes: vec![d.path().join("share")], ..Default::default() };
        let profile = d.path().join("share/Xenia/content/E030000012345678/FFFE07D1/00010000/E030000012345678");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::write(profile.join("4D5307E6.gpd"), game_gpd()).unwrap();
        let mut title = 0x4D53_07E6u32.to_be_bytes().to_vec();
        title.resize(0x28, 0);
        title.extend(utf16("Halo 3"));
        std::fs::write(profile.join("FFFE07D1.gpd"), xdbf(&[(4, 0x4D53_07E6, title)])).unwrap();
        let game = d.path().join("games/Halo 3");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("default.xex"), xex(0x4D53_07E6)).unwrap();
        let target = |rom: PathBuf, title: &str| TrophyTarget {
            emulator: TrophyEmulator::Xenia,
            roms: vec![rom],
            title: title.into(),
            emulator_id: None,
        };
        let data = d.path().join("data");
        let Fetch::Set(set) = fetch(&env, &[], &target(game.join("default.xex"), "Whatever"), &data) else { panic!("by the xex") };
        assert_eq!(set.unlocked(), 1);
        assert!(set.achievements[0].icon.starts_with(data.join("_trophy-icons/xenia").to_str().unwrap()));
        let Fetch::Set(_) = fetch(&env, &[], &target(d.path().join("halo3.iso"), "HALO 3"), &data) else { panic!("by name") };
        assert_eq!(fetch(&env, &[], &target(d.path().join("other.iso"), "Other"), &data), Fetch::None);
        assert_eq!(watched_files(&env, &[], &target(game.join("default.xex"), "")), [profile.join("4D5307E6.gpd")]);
    }
}
