//! File helpers for save states (port of `emulation/save-state-files.ts`):
//! XDG/Flatpak folders, INI files, ZIP entries and embedded PNGs.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::pathutil::{join, resolve_from};

/// Where an emulator's per-user folders live. [`HostEnv::system`] reads the
/// process environment; tests build one around a temp directory.
#[derive(Debug, Clone)]
pub struct HostEnv {
    pub home: PathBuf,
    /// `$XDG_CONFIG_HOME` exactly as set (DuckStation checks the variable).
    pub xdg_config_home_var: Option<PathBuf>,
    pub xdg_data_home_var: Option<PathBuf>,
    /// `$DOLPHIN_EMU_USERPATH`.
    pub dolphin_userpath: Option<PathBuf>,
    /// Where one-shot config files for RetroArch are written.
    pub tmp_dir: PathBuf,
}

impl HostEnv {
    pub fn system() -> Self {
        let var = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Self {
            home: dirs_home(),
            xdg_config_home_var: var("XDG_CONFIG_HOME"),
            xdg_data_home_var: var("XDG_DATA_HOME"),
            dolphin_userpath: var("DOLPHIN_EMU_USERPATH"),
            tmp_dir: std::env::temp_dir(),
        }
    }

    /// A sandbox: everything under `home`, no environment overrides.
    pub fn sandbox(home: impl Into<PathBuf>, tmp: impl Into<PathBuf>) -> Self {
        Self {
            home: home.into(),
            xdg_config_home_var: None,
            xdg_data_home_var: None,
            dolphin_userpath: None,
            tmp_dir: tmp.into(),
        }
    }

    /// `xdgConfigHome()`.
    pub fn xdg_config_home(&self) -> PathBuf {
        self.xdg_config_home_var
            .clone()
            .unwrap_or_else(|| self.home.join(".config"))
    }

    /// `xdgDataHome()`.
    pub fn xdg_data_home(&self) -> PathBuf {
        self.xdg_data_home_var
            .clone()
            .unwrap_or_else(|| self.home.join(".local/share"))
    }

    /// `flatpakDir`.
    pub fn flatpak_dir(&self, flatpak_id: &str, kind: &str, rest: &[&str]) -> PathBuf {
        let mut p = self.home.join(".var/app").join(flatpak_id).join(kind);
        for r in rest {
            p.push(r);
        }
        p
    }
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// `key = value` INI and RetroArch-style `key = "value"` files. Keys before
/// the first section live in section `""`.
#[derive(Debug, Default, Clone)]
pub struct Ini {
    sections: HashMap<String, HashMap<String, String>>,
}

impl Ini {
    pub fn parse(text: &str) -> Self {
        let mut ini = Ini::default();
        let mut current = String::new();
        ini.sections.entry(current.clone()).or_default();
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if line.len() > 2 && line.starts_with('[') && line.ends_with(']') {
                current = line[1..line.len() - 1].to_string();
                ini.sections.entry(current.clone()).or_default();
                continue;
            }
            let Some(eq) = line.find('=') else { continue };
            let key = line[..eq].trim().to_string();
            let mut value = line[eq + 1..].trim();
            if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
                value = &value[1..value.len() - 1];
            }
            ini.sections
                .entry(current.clone())
                .or_default()
                .insert(key, value.to_string());
        }
        ini
    }

    /// `readIni`.
    pub fn read(path: &Path) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        Some(Self::parse(&String::from_utf8_lossy(&bytes)))
    }

    /// `iniValue`: `None` for missing or empty values.
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections
            .get(section)
            .and_then(|s| s.get(key))
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }
}

/// `iniValue(readIni(path), section, key)`.
pub fn ini_value(path: &Path, section: &str, key: &str) -> Option<String> {
    Ini::read(path)?.get(section, key).map(str::to_string)
}

/// `resolveConfiguredDir`: `default`/empty -> `None`, `~` expanded, relative
/// paths resolved against `base`.
pub fn resolve_configured_dir(value: Option<&str>, base: &str, home: &Path) -> Option<String> {
    let value = value.filter(|v| !v.is_empty() && *v != "default")?;
    let expanded = if value == "~" {
        home.to_string_lossy().into_owned()
    } else if let Some(rest) = value.strip_prefix("~/") {
        join(&home.to_string_lossy(), rest)
    } else {
        value.to_string()
    };
    Some(if expanded.starts_with('/') {
        expanded
    } else {
        resolve_from(base, &expanded)
    })
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

fn le32(d: &[u8], at: usize) -> Option<u32> {
    d.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn le16(d: &[u8], at: usize) -> Option<u16> {
    d.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

/// `readZipEntry`: one stored or deflated entry from a ZIP archive (PCSX2
/// `.p2s` keeps its `Screenshot.png` stored next to zstd-compressed memory).
pub fn read_zip_entry(path: &Path, entry_name: &str) -> Option<Vec<u8>> {
    let mut file = File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let tail_len = size.min(65557) as usize;
    let tail = read_at(&mut file, size - tail_len as u64, tail_len);
    let eocd = tail.windows(4).rposition(|w| w == [0x50, 0x4b, 0x05, 0x06])?;
    let dir_size = le32(&tail, eocd + 12)? as usize;
    let dir_offset = le32(&tail, eocd + 16)? as u64;
    let dir = read_at(&mut file, dir_offset, dir_size);
    let mut pos = 0usize;
    while pos + 46 <= dir.len() && le32(&dir, pos)? == 0x0201_4b50 {
        let method = le16(&dir, pos + 10)?;
        let compressed = le32(&dir, pos + 20)? as usize;
        let name_len = le16(&dir, pos + 28)? as usize;
        let extra_len = le16(&dir, pos + 30)? as usize;
        let comment_len = le16(&dir, pos + 32)? as usize;
        let local = le32(&dir, pos + 42)? as u64;
        let name = String::from_utf8_lossy(dir.get(pos + 46..pos + 46 + name_len)?);
        if name == entry_name {
            let head = read_at(&mut file, local, 30);
            if head.len() < 30 || le32(&head, 0)? != 0x0403_4b50 {
                return None;
            }
            let start = local + 30 + le16(&head, 26)? as u64 + le16(&head, 28)? as u64;
            let data = read_at(&mut file, start, compressed);
            return match method {
                0 => Some(data),
                8 => {
                    let mut out = Vec::new();
                    flate2::read::DeflateDecoder::new(&data[..])
                        .read_to_end(&mut out)
                        .ok()?;
                    Some(out)
                }
                _ => None,
            };
        }
        pos += 46 + name_len + extra_len + comment_len;
    }
    None
}

const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

/// `findEmbeddedPng`: a PNG inside the first 4 MiB of a state file (DuckStation
/// and mGBA states embed their screenshot).
pub fn find_embedded_png(path: &Path) -> Option<Vec<u8>> {
    let mut file = File::open(path).ok()?;
    let data = read_at(&mut file, 0, 4 * 1024 * 1024);
    let start = crate::disc_id::find_bytes(&data, &PNG_SIGNATURE, 0)?;
    let end = crate::disc_id::find_bytes(&data, b"IEND", start)?;
    // `IEND` + 4 bytes of CRC.
    data.get(start..(end + 8).min(data.len())).map(<[u8]>::to_vec)
}

/// Mime type from the magic bytes of an image.
pub fn image_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&PNG_SIGNATURE) {
        "image/png"
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "image/jpeg"
    } else if bytes.starts_with(b"BM") {
        "image/bmp"
    } else {
        "application/octet-stream"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn ini_parsing_matches_heronite() {
        let ini = Ini::parse(
            "top = 1\n; comment\n[Folders]\nSavestates = /data/states\nEmpty =\n[ports.qt]\nsavestatePath = \"~/states\"\n",
        );
        assert_eq!(ini.get("", "top"), Some("1"));
        assert_eq!(ini.get("Folders", "Savestates"), Some("/data/states"));
        assert_eq!(ini.get("Folders", "Empty"), None);
        assert_eq!(ini.get("ports.qt", "savestatePath"), Some("~/states"));
        assert_eq!(ini.get("nope", "x"), None);
    }

    #[test]
    fn configured_dirs() {
        let home = Path::new("/home/u");
        assert_eq!(resolve_configured_dir(Some("default"), "/b", home), None);
        assert_eq!(resolve_configured_dir(None, "/b", home), None);
        assert_eq!(resolve_configured_dir(Some("~/st"), "/b", home).as_deref(), Some("/home/u/st"));
        assert_eq!(resolve_configured_dir(Some("/abs"), "/b", home).as_deref(), Some("/abs"));
        assert_eq!(resolve_configured_dir(Some("rel/x"), "/b", home).as_deref(), Some("/b/rel/x"));
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &b in data {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
            }
        }
        !crc
    }

    /// Builds a zip with one stored and one deflated entry.
    fn build_zip(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data, deflate) in entries {
            let (method, payload) = if *deflate {
                let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                enc.write_all(data).unwrap();
                (8u16, enc.finish().unwrap())
            } else {
                (0u16, data.to_vec())
            };
            let offset = out.len() as u32;
            let crc = crc32(data);
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&[20, 0, 0, 0]);
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0, 0, 0, 0]);
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&payload);

            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            central.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0, 0, 0, 0]);
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 12]);
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let dir_offset = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&dir_offset.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    }

    #[test]
    fn reads_stored_and_deflated_zip_entries() {
        let d = tempfile::tempdir().unwrap();
        let png = [&PNG_SIGNATURE[..], b"payload"].concat();
        let big = vec![b'a'; 5000];
        let zip = build_zip(&[("memory.bin", &big, true), ("Screenshot.png", &png, false)]);
        let p = d.path().join("s.p2s");
        std::fs::write(&p, zip).unwrap();
        assert_eq!(read_zip_entry(&p, "Screenshot.png").unwrap(), png);
        assert_eq!(read_zip_entry(&p, "memory.bin").unwrap(), big);
        assert_eq!(read_zip_entry(&p, "missing"), None);
        assert_eq!(read_zip_entry(&d.path().join("none"), "x"), None);
        std::fs::write(d.path().join("junk"), b"not a zip at all").unwrap();
        assert_eq!(read_zip_entry(&d.path().join("junk"), "x"), None);
    }

    #[test]
    fn finds_embedded_png() {
        let d = tempfile::tempdir().unwrap();
        let png = [&PNG_SIGNATURE[..], b"\0\0\0\rIHDRxxxx", b"\0\0\0\0IEND\xaeB`\x82"].concat();
        let mut blob = vec![7u8; 100];
        blob.extend_from_slice(&png);
        blob.extend_from_slice(&[9u8; 50]);
        let p = d.path().join("state.sav");
        std::fs::write(&p, &blob).unwrap();
        let found = find_embedded_png(&p).unwrap();
        assert_eq!(found, png);
        assert_eq!(image_mime(&found), "image/png");
        std::fs::write(&p, [1u8; 64]).unwrap();
        assert_eq!(find_embedded_png(&p), None);
        assert_eq!(image_mime(&[0xff, 0xd8, 0xff, 0xe0]), "image/jpeg");
    }
}
