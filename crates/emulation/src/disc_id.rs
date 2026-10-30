//! Disc identifiers emulators use to name save states (port of
//! `emulation/disc-id.ts`): PS1/PS2 serial from `SYSTEM.CNF`, PSP disc id
//! from `PARAM.SFO`/`UMD_DATA.BIN`, PS3 `TITLE_ID`, GameCube/Wii game id, and
//! the emulator game-list cache fallback.
//!
//! Compressed formats (CHD, CSO, GCZ...) are not readable here and yield
//! `None`; callers fall back to the game-list cache or to a key learned
//! after a session ([`crate::save_states::remember_save_state_key`]).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use indexmap::IndexMap;
use regex::Regex;

use crate::pathutil::{dirname, join};

const SECTOR: usize = 2048;
const RAW_SECTOR: usize = 2352;
const SYNC: [u8; 12] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];

#[derive(Clone, Copy)]
struct Layout {
    size: usize,
    offset: usize,
}

fn read_at(file: &mut File, position: u64, length: usize) -> Vec<u8> {
    let mut buf = vec![0u8; length];
    if file.seek(SeekFrom::Start(position)).is_err() {
        return vec![];
    }
    let mut read = 0;
    while read < length {
        match file.read(&mut buf[read..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => read += n,
        }
    }
    buf.truncate(read);
    buf
}

fn detect_layout(file: &mut File) -> Layout {
    let head = read_at(file, 0, 16);
    if head.len() == 16 && head[..12] == SYNC {
        return Layout { size: RAW_SECTOR, offset: if head[15] == 2 { 24 } else { 16 } };
    }
    Layout { size: SECTOR, offset: 0 }
}

fn read_logical(file: &mut File, layout: Layout, lba: u32, length: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut remaining = length;
    let mut sector = lba as u64;
    while remaining > 0 {
        let take = remaining.min(SECTOR);
        let chunk = read_at(file, sector * layout.size as u64 + layout.offset as u64, take);
        if chunk.is_empty() {
            break;
        }
        out.extend_from_slice(&chunk);
        remaining -= take;
        sector += 1;
    }
    out
}

struct DirEntry {
    name: String,
    lba: u32,
    size: u32,
    is_dir: bool,
}

fn u32le(data: &[u8], at: usize) -> Option<u32> {
    data.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn u16le(data: &[u8], at: usize) -> Option<u16> {
    data.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn parse_directory(data: &[u8]) -> Vec<DirEntry> {
    let mut entries = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        let length = data[pos] as usize;
        if length == 0 {
            pos = (pos / SECTOR + 1) * SECTOR;
            continue;
        }
        let Some(&name_len) = data.get(pos + 32) else { break };
        let name_len = name_len as usize;
        let (Some(name_bytes), Some(lba), Some(size), Some(&flags)) =
            (data.get(pos + 33..pos + 33 + name_len), u32le(data, pos + 2), u32le(data, pos + 10), data.get(pos + 25))
        else {
            break;
        };
        let raw: String = name_bytes.iter().map(|&b| b as char).collect();
        let name = match raw.rfind(';') {
            Some(i) if raw[i + 1..].chars().all(|c| c.is_ascii_digit()) && i + 1 < raw.len() => raw[..i].to_string(),
            _ => raw,
        }
        .to_uppercase();
        entries.push(DirEntry { name, lba, size, is_dir: flags & 0x02 != 0 });
        pos += length;
    }
    entries
}

/// `readIsoFile`: a file (at most `max_bytes`) from an ISO9660 image given by
/// its path components, e.g. `["PSP_GAME", "PARAM.SFO"]`.
pub fn read_iso_file(image: &Path, file_path: &[&str], max_bytes: usize) -> Option<Vec<u8>> {
    let mut file = File::open(image).ok()?;
    let layout = detect_layout(&mut file);
    let pvd = read_logical(&mut file, layout, 16, SECTOR);
    if pvd.get(1..6)? != b"CD001" {
        return None;
    }
    let mut dir_lba = u32le(&pvd, 156 + 2)?;
    let mut dir_size = u32le(&pvd, 156 + 10)? as usize;
    for (index, want) in file_path.iter().enumerate() {
        let want = want.to_uppercase();
        let listing = read_logical(&mut file, layout, dir_lba, dir_size.min(256 * 1024));
        let entry = parse_directory(&listing).into_iter().find(|e| e.name == want)?;
        if index == file_path.len() - 1 {
            if entry.is_dir {
                return None;
            }
            return Some(read_logical(&mut file, layout, entry.lba, (entry.size as usize).min(max_bytes)));
        }
        if !entry.is_dir {
            return None;
        }
        dir_lba = entry.lba;
        dir_size = entry.size as usize;
    }
    None
}

fn first_cue_file_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"(?im)^\s*FILE\s+"?([^"\r\n]+?)"?\s+\w+\s*$"#).unwrap())
}

/// `resolveImagePath`: `.cue`/`.m3u` point at the real image; follow them to
/// the first data file.
pub fn resolve_image_path(rom: &Path) -> PathBuf {
    let s = rom.to_string_lossy();
    let ext = crate::pathutil::ext_lower(&s);
    if ext != "cue" && ext != "m3u" {
        return rom.to_path_buf();
    }
    let Ok(bytes) = std::fs::read(rom) else {
        return rom.to_path_buf();
    };
    let text = String::from_utf8_lossy(&bytes);
    let target = if ext == "cue" {
        first_cue_file_re().captures(&text).map(|c| c[1].to_string())
    } else {
        text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_string)
    };
    let Some(target) = target else {
        return rom.to_path_buf();
    };
    let resolved = if target.starts_with('/') { target } else { join(&dirname(&s), &target) };
    let next = PathBuf::from(&resolved);
    if next == rom {
        return next;
    }
    resolve_image_path(&next)
}

/// `serialFromSystemCnf`: `BOOT2 = cdrom0:\SLUS_209.46;1` -> `SLUS-20946`.
pub fn serial_from_system_cnf(text: &str) -> Option<String> {
    static BOOT: OnceLock<Regex> = OnceLock::new();
    static SERIAL: OnceLock<Regex> = OnceLock::new();
    let boot = BOOT.get_or_init(|| Regex::new(r"(?im)^\s*BOOT2?\s*=\s*(\S+)").unwrap());
    let serial = SERIAL.get_or_init(|| Regex::new(r"(?i)^([A-Z]{4})[_-](\d{3})\.(\d{2})").unwrap());
    let boot = boot.captures(text)?[1].to_string();
    let file = boot.rsplit(['\\', '/', ':']).next().unwrap_or("");
    let c = serial.captures(file)?;
    Some(format!("{}-{}{}", c[1].to_uppercase(), &c[2], &c[3]))
}

/// PS1/PS2 serial (`SLUS-20946`) of a disc image, following .cue/.m3u.
pub fn read_playstation_serial(rom: &Path) -> Option<String> {
    let image = resolve_image_path(rom);
    let cnf = read_iso_file(&image, &["SYSTEM.CNF"], 64 * 1024)?;
    serial_from_system_cnf(&latin1(&cnf))
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// `readSfo`: `PARAM.SFO` string entries (`\0PSF` header, key table, data
/// table, 16-byte index entries).
pub fn read_sfo(data: &[u8]) -> IndexMap<String, String> {
    let mut result = IndexMap::new();
    if data.len() < 20 || &data[..4] != b"\0PSF" {
        return result;
    }
    let (Some(key_table), Some(data_table), Some(count)) = (u32le(data, 8), u32le(data, 12), u32le(data, 16)) else {
        return result;
    };
    for index in 0..count as usize {
        let entry = 20 + index * 16;
        if entry + 16 > data.len() {
            break;
        }
        let (Some(key_off), Some(format), Some(length), Some(data_off)) =
            (u16le(data, entry), u16le(data, entry + 2), u32le(data, entry + 4), u32le(data, entry + 12))
        else {
            break;
        };
        let key_start = key_table as usize + key_off as usize;
        let value_start = data_table as usize + data_off as usize;
        let Some(key_end) = data.get(key_start..).and_then(|s| s.iter().position(|&b| b == 0)).map(|p| key_start + p) else {
            continue;
        };
        if format != 0x0204 {
            continue;
        }
        let end = (value_start + length as usize).min(data.len());
        let value = data.get(value_start..end).unwrap_or(&[]);
        let value = String::from_utf8_lossy(value);
        result.insert(latin1(&data[key_start..key_end]), value.trim_end_matches('\0').to_string());
    }
    result
}

/// PSP `DISC_ID` (`ULUS10041`).
pub fn read_psp_disc_id(rom: &Path) -> Option<String> {
    let image = resolve_image_path(rom);
    if let Some(sfo) = read_iso_file(&image, &["PSP_GAME", "PARAM.SFO"], 64 * 1024) {
        if let Some(id) = read_sfo(&sfo).get("DISC_ID") {
            if id.len() == 9 && id.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
                return Some(id.clone());
            }
        }
    }
    static UMD: OnceLock<Regex> = OnceLock::new();
    let umd = UMD.get_or_init(|| Regex::new(r"^([A-Z]{4})-?(\d{5})").unwrap());
    let data = read_iso_file(&image, &["UMD_DATA.BIN"], 64)?;
    let text = latin1(&data);
    let c = umd.captures(&text)?;
    Some(format!("{}{}", &c[1], &c[2]))
}

fn ps3_title(data: Option<Vec<u8>>) -> Option<String> {
    let id = read_sfo(&data?).get("TITLE_ID")?.clone();
    let ok = id.len() == 9 && id[..4].chars().all(|c| c.is_ascii_uppercase()) && id[4..].chars().all(|c| c.is_ascii_digit());
    ok.then_some(id)
}

/// RPCS3 `TITLE_ID` (`BCUS98114`): games are folders
/// (`.../PS3_GAME/USRDIR/EBOOT.BIN`) or ISO images.
pub fn read_ps3_title_id(rom: &Path) -> Option<String> {
    let s = rom.to_string_lossy().into_owned();
    if crate::pathutil::ext_lower(&s) == "iso" {
        return ps3_title(read_iso_file(rom, &["PS3_GAME", "PARAM.SFO"], 64 * 1024));
    }
    let mut dir = if s.to_lowercase().ends_with(".bin") { dirname(&s) } else { s };
    for _ in 0..3 {
        for candidate in [join(&dir, "PARAM.SFO"), join(&dir, "PS3_GAME/PARAM.SFO")] {
            if let Some(id) = ps3_title(std::fs::read(candidate).ok()) {
                return Some(id);
            }
        }
        dir = dirname(&dir);
    }
    None
}

/// GameCube/Wii 6-character game id (`GALE01`) from `.iso/.gcm/.rvz/.wia/.wbfs`.
pub fn read_nintendo_disc_id(rom: &Path) -> Option<String> {
    let s = rom.to_string_lossy();
    let ext = crate::pathutil::ext_lower(&s);
    let offset = match ext.as_str() {
        "rvz" | "wia" => 0x58,
        "wbfs" => 0x200,
        "iso" | "gcm" => 0,
        _ => return None,
    };
    let mut file = File::open(rom).ok()?;
    let id = latin1(&read_at(&mut file, offset, 6));
    (id.len() == 6 && id.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())).then_some(id)
}

pub(crate) fn find_bytes(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() || from > hay.len() - needle.len() {
        return None;
    }
    let first = needle[0];
    let mut i = from;
    let last = hay.len() - needle.len();
    while i <= last {
        {
            let p = hay[i..=last].iter().position(|&b| b == first)?;
            i += p;
            if &hay[i..i + needle.len()] == needle {
                return Some(i);
            }
            i += 1;
        }
    }
    None
}

/// `serialFromGameListCache`: PCSX2 and DuckStation keep a binary game list
/// cache where each entry starts with the u32-length-prefixed file path
/// followed by the serial. Matching the path and validating the next string
/// keeps this tolerant of format changes.
pub fn serial_from_game_list_cache(cache: &Path, rom: &str) -> Option<String> {
    let data = std::fs::read(cache).ok()?;
    let needle = rom.as_bytes();
    let serial_re = {
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| Regex::new(r"(?i)^[A-Z0-9][A-Z0-9_-]{3,}$").unwrap())
    };
    let mut from = 0usize;
    while from < data.len() {
        let at = find_bytes(&data, needle, from)?;
        if at < 4 {
            from = at + 1;
            continue;
        }
        let path_len = u32le(&data, at - 4)? as usize;
        let next = at + needle.len();
        if path_len == needle.len() && next + 4 <= data.len() {
            let serial_len = u32le(&data, next)? as usize;
            if serial_len > 0 && serial_len < 32 {
                if let Some(raw) = data.get(next + 4..next + 4 + serial_len) {
                    let serial = String::from_utf8_lossy(raw);
                    if serial_re.is_match(&serial) {
                        return Some(serial.into_owned());
                    }
                }
            }
        }
        from = at + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal ISO9660: PVD at sector 16, root dir at sector 18 with the given
    /// (name, is_dir, lba, size) records, file contents placed by the caller.
    pub(crate) fn dir_record(name: &str, is_dir: bool, lba: u32, size: u32) -> Vec<u8> {
        let mut rec = vec![0u8; 33 + name.len()];
        let mut len = rec.len();
        if len % 2 == 1 {
            len += 1;
        }
        rec.resize(len, 0);
        rec[0] = len as u8;
        rec[2..6].copy_from_slice(&lba.to_le_bytes());
        rec[10..14].copy_from_slice(&size.to_le_bytes());
        rec[25] = if is_dir { 2 } else { 0 };
        rec[32] = name.len() as u8;
        rec[33..33 + name.len()].copy_from_slice(name.as_bytes());
        rec
    }

    pub(crate) struct IsoBuilder {
        sectors: Vec<Vec<u8>>,
    }

    impl IsoBuilder {
        pub(crate) fn new() -> Self {
            let mut s = Self { sectors: vec![vec![0u8; SECTOR]; 20] };
            // Root directory lives in sector 18 (written on finish).
            s.sectors[16][0] = 1;
            s.sectors[16][1..6].copy_from_slice(b"CD001");
            s
        }
        /// Adds a file at the end, returns (lba, size).
        pub(crate) fn add_file(&mut self, data: &[u8]) -> (u32, u32) {
            let lba = self.sectors.len() as u32;
            for chunk in data.chunks(SECTOR) {
                let mut sector = vec![0u8; SECTOR];
                sector[..chunk.len()].copy_from_slice(chunk);
                self.sectors.push(sector);
            }
            if data.is_empty() {
                self.sectors.push(vec![0u8; SECTOR]);
            }
            (lba, data.len() as u32)
        }
        pub(crate) fn add_dir(&mut self, records: Vec<Vec<u8>>) -> (u32, u32) {
            let mut sector = vec![0u8; SECTOR];
            let mut pos = 0;
            for r in records {
                sector[pos..pos + r.len()].copy_from_slice(&r);
                pos += r.len();
            }
            let lba = self.sectors.len() as u32;
            self.sectors.push(sector);
            (lba, SECTOR as u32)
        }
        pub(crate) fn finish(mut self, root: Vec<Vec<u8>>, raw: bool) -> Vec<u8> {
            let mut sector = vec![0u8; SECTOR];
            let mut pos = 0;
            for r in root {
                sector[pos..pos + r.len()].copy_from_slice(&r);
                pos += r.len();
            }
            self.sectors[18] = sector;
            let root_rec = dir_record("\0", true, 18, SECTOR as u32);
            self.sectors[16][156..156 + root_rec.len()].copy_from_slice(&root_rec);
            let mut out = Vec::new();
            for s in &self.sectors {
                if raw {
                    let mut frame = Vec::new();
                    frame.extend_from_slice(&SYNC);
                    frame.extend_from_slice(&[0, 0, 0, 1]); // MSF + mode 1
                    frame.extend_from_slice(s);
                    frame.resize(RAW_SECTOR, 0);
                    out.extend_from_slice(&frame);
                } else {
                    out.extend_from_slice(s);
                }
            }
            out
        }
    }

    fn sfo(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut keys = Vec::new();
        let mut vals = Vec::new();
        let mut index = Vec::new();
        for (k, v) in entries {
            let mut e = Vec::new();
            e.extend_from_slice(&(keys.len() as u16).to_le_bytes());
            e.extend_from_slice(&0x0204u16.to_le_bytes());
            e.extend_from_slice(&((v.len() + 1) as u32).to_le_bytes());
            e.extend_from_slice(&((v.len() + 1) as u32).to_le_bytes());
            e.extend_from_slice(&(vals.len() as u32).to_le_bytes());
            index.extend(e);
            keys.extend_from_slice(k.as_bytes());
            keys.push(0);
            vals.extend_from_slice(v.as_bytes());
            vals.push(0);
        }
        let key_table = 20 + index.len();
        let data_table = key_table + keys.len();
        let mut out = b"\0PSF".to_vec();
        out.extend_from_slice(&0x101u32.to_le_bytes());
        out.extend_from_slice(&(key_table as u32).to_le_bytes());
        out.extend_from_slice(&(data_table as u32).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        out.extend(index);
        out.extend(keys);
        out.extend(vals);
        out
    }

    #[test]
    fn system_cnf_serials() {
        assert_eq!(serial_from_system_cnf("BOOT2 = cdrom0:\\SLUS_209.46;1\r\nVER = 1.00\r\n").as_deref(), Some("SLUS-20946"));
        assert_eq!(serial_from_system_cnf("BOOT = cdrom:\\SCUS_941.63;1\nTCB = 4").as_deref(), Some("SCUS-94163"));
        assert_eq!(serial_from_system_cnf("BOOT2 = cdrom0:\\NOPE.ELF"), None);
        assert_eq!(serial_from_system_cnf("junk"), None);
    }

    #[test]
    fn reads_ps2_serial_from_cooked_and_raw_images() {
        for raw in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut iso = IsoBuilder::new();
            let (lba, size) = iso.add_file(b"BOOT2 = cdrom0:\\SLES_523.60;1\r\nVER = 1.00\r\n");
            let image = iso.finish(vec![dir_record("SYSTEM.CNF;1", false, lba, size)], raw);
            let path = dir.path().join("game.iso");
            std::fs::write(&path, image).unwrap();
            assert_eq!(read_playstation_serial(&path).as_deref(), Some("SLES-52360"), "raw={raw}");
        }
    }

    // Linux emulator layouts and paths; emulation is off on Windows (`Feature::Emulation`).
    #[cfg(unix)]
    #[test]
    fn follows_cue_and_m3u_to_the_image() {
        let dir = tempfile::tempdir().unwrap();
        let mut iso = IsoBuilder::new();
        let (lba, size) = iso.add_file(b"BOOT = cdrom:\\SCUS_941.63;1\n");
        std::fs::write(dir.path().join("game.bin"), iso.finish(vec![dir_record("SYSTEM.CNF;1", false, lba, size)], true)).unwrap();
        std::fs::write(dir.path().join("game.cue"), "FILE \"game.bin\" BINARY\r\n  TRACK 01 MODE2/2352\r\n    INDEX 01 00:00:00\r\n")
            .unwrap();
        std::fs::write(dir.path().join("list.m3u"), "#EXTM3U\ngame.cue\n").unwrap();
        assert_eq!(read_playstation_serial(&dir.path().join("game.cue")).as_deref(), Some("SCUS-94163"));
        assert_eq!(read_playstation_serial(&dir.path().join("list.m3u")).as_deref(), Some("SCUS-94163"));
    }

    #[test]
    fn reads_psp_disc_id_from_sfo_and_umd_data() {
        let dir = tempfile::tempdir().unwrap();
        let mut iso = IsoBuilder::new();
        let (sl, ss) = iso.add_file(&sfo(&[("TITLE", "Game"), ("DISC_ID", "ULUS10041")]));
        let (psp_lba, psp_size) = iso.add_dir(vec![dir_record("PARAM.SFO;1", false, sl, ss)]);
        let image = iso.finish(vec![dir_record("PSP_GAME", true, psp_lba, psp_size)], false);
        let path = dir.path().join("psp.iso");
        std::fs::write(&path, image).unwrap();
        assert_eq!(read_psp_disc_id(&path).as_deref(), Some("ULUS10041"));

        let mut iso = IsoBuilder::new();
        let (ul, us) = iso.add_file(b"ULES-00123|0000000000|1|G");
        let image = iso.finish(vec![dir_record("UMD_DATA.BIN;1", false, ul, us)], false);
        let path = dir.path().join("umd.iso");
        std::fs::write(&path, image).unwrap();
        assert_eq!(read_psp_disc_id(&path).as_deref(), Some("ULES00123"));
    }

    #[test]
    fn reads_ps3_title_from_folder_and_iso() {
        let dir = tempfile::tempdir().unwrap();
        let usrdir = dir.path().join("Game/PS3_GAME/USRDIR");
        std::fs::create_dir_all(&usrdir).unwrap();
        std::fs::write(usrdir.join("EBOOT.BIN"), "x").unwrap();
        std::fs::write(dir.path().join("Game/PS3_GAME/PARAM.SFO"), sfo(&[("TITLE_ID", "BCUS98114")])).unwrap();
        assert_eq!(read_ps3_title_id(&usrdir.join("EBOOT.BIN")).as_deref(), Some("BCUS98114"));
        assert_eq!(read_ps3_title_id(&dir.path().join("Game")).as_deref(), Some("BCUS98114"));

        let mut iso = IsoBuilder::new();
        let (sl, ss) = iso.add_file(&sfo(&[("TITLE_ID", "NPUA80001")]));
        let (d_lba, d_size) = iso.add_dir(vec![dir_record("PARAM.SFO;1", false, sl, ss)]);
        let image = iso.finish(vec![dir_record("PS3_GAME", true, d_lba, d_size)], false);
        let p = dir.path().join("g.iso");
        std::fs::write(&p, image).unwrap();
        assert_eq!(read_ps3_title_id(&p).as_deref(), Some("NPUA80001"));
    }

    #[test]
    fn nintendo_ids() {
        let dir = tempfile::tempdir().unwrap();
        let mut gc = b"GALE01".to_vec();
        gc.resize(64, 0);
        std::fs::write(dir.path().join("a.iso"), &gc).unwrap();
        assert_eq!(read_nintendo_disc_id(&dir.path().join("a.iso")).as_deref(), Some("GALE01"));
        let mut rvz = vec![0u8; 0x58];
        rvz.extend_from_slice(b"RSBE01");
        std::fs::write(dir.path().join("b.rvz"), &rvz).unwrap();
        assert_eq!(read_nintendo_disc_id(&dir.path().join("b.rvz")).as_deref(), Some("RSBE01"));
        let mut wbfs = vec![0u8; 0x200];
        wbfs.extend_from_slice(b"R4QE01");
        std::fs::write(dir.path().join("c.wbfs"), &wbfs).unwrap();
        assert_eq!(read_nintendo_disc_id(&dir.path().join("c.wbfs")).as_deref(), Some("R4QE01"));
        std::fs::write(dir.path().join("d.iso"), b"lowerc").unwrap();
        assert_eq!(read_nintendo_disc_id(&dir.path().join("d.iso")), None);
        assert_eq!(read_nintendo_disc_id(&dir.path().join("e.chd")), None);
    }

    #[test]
    fn game_list_cache_lookup() {
        let dir = tempfile::tempdir().unwrap();
        let rom = "/roms/Game (USA).iso";
        let mut blob = vec![9u8; 13];
        blob.extend_from_slice(&(rom.len() as u32).to_le_bytes());
        blob.extend_from_slice(rom.as_bytes());
        blob.extend_from_slice(&10u32.to_le_bytes());
        blob.extend_from_slice(b"SLUS-20946");
        blob.extend_from_slice(&[1, 2, 3]);
        let cache = dir.path().join("gamelist.cache");
        std::fs::write(&cache, &blob).unwrap();
        assert_eq!(serial_from_game_list_cache(&cache, rom).as_deref(), Some("SLUS-20946"));
        assert_eq!(serial_from_game_list_cache(&cache, "/roms/other.iso"), None);
        // A bare occurrence without the length prefix is ignored.
        let mut fake = rom.as_bytes().to_vec();
        fake.extend_from_slice(&10u32.to_le_bytes());
        fake.extend_from_slice(b"SLUS-20946");
        std::fs::write(&cache, &fake).unwrap();
        assert_eq!(serial_from_game_list_cache(&cache, rom), None);
        assert_eq!(serial_from_game_list_cache(&dir.path().join("none"), rom), None);
    }

    #[test]
    fn sfo_ignores_garbage() {
        assert!(read_sfo(b"nope").is_empty());
        assert!(read_sfo(&[0; 40]).is_empty());
        let s = sfo(&[("A", "1"), ("B", "two")]);
        let m = read_sfo(&s);
        assert_eq!(m["A"], "1");
        assert_eq!(m["B"], "two");
    }
}
