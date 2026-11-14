//! Native Linux builds come as GOG's installers: a shell script (MojoSetup)
//! with a zip archive appended. The game's files are the archive's
//! `data/noarch/` entries, read straight out of the installer with HTTP
//! ranges: the central directory at the end says where each one is, so the
//! installer itself is never downloaded whole or run.
//!
//! An install keeps gogdl's `.gogdl-linux-manifest` in the game's folder
//! (each entry's name and crc32, for updates) and the installer's
//! `gameinfo` file, which the launch and imports read.

use std::io::Read;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

use crate::api::{self, Client};
use crate::files::{Checksum, Diff, FileEntry, Link, Root, Source};
use crate::language;

const LOCAL_HEADER: &[u8] = b"PK\x03\x04";
const CENTRAL_HEADER: &[u8] = b"PK\x01\x02";
const END_OF_CD: &[u8] = b"PK\x05\x06";
const ZIP64_LOCATOR: &[u8] = b"PK\x06\x07";
const ZIP64_END_OF_CD: &[u8] = b"PK\x06\x06";
/// The script before the archive is smaller than this.
const SCRIPT_SEARCH: u64 = 2 * 1024 * 1024;
/// The files that matter, inside the archive.
const DATA_PREFIX: &str = "data/noarch/";

/// One entry of the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub crc32: u32,
    pub compressed_size: u64,
    pub size: u64,
    /// 0 stored, 8 deflated.
    pub method: u16,
    /// Of its local header, from the start of the archive.
    pub local_offset: u64,
    /// External attributes: the Unix mode is the high 16 bits.
    pub attributes: u32,
    /// Where its data starts, from the start of the archive.
    pub data_offset: u64,
}

impl Entry {
    fn mode(&self) -> u32 {
        self.attributes >> 16
    }

    pub fn is_symlink(&self) -> bool {
        self.mode() & 0o170000 == 0o120000
    }

    pub fn is_dir(&self) -> bool {
        self.name.ends_with('/')
    }

    pub fn is_executable(&self) -> bool {
        self.mode() & 0o111 != 0
    }

    /// The path in the game's folder, for an entry of `data/noarch/`.
    pub fn game_path(&self) -> Option<&str> {
        self.name.strip_prefix(DATA_PREFIX).filter(|p| !p.is_empty())
    }

    /// gogdl's record of it in `.gogdl-linux-manifest`.
    pub fn to_record(&self) -> Value {
        json!({"file_name": self.name, "crc32": self.crc32, "compressed_size": self.compressed_size, "size": self.size, "is_symlink": self.is_symlink()})
    }
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    b.get(at..at + 2).map_or(0, |s| u16::from_le_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    b.get(at..at + 4).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    b.get(at..at + 8).map_or(0, |s| u64::from_le_bytes(s.try_into().unwrap_or_default()))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).rposition(|w| w == needle)
}

/// Where the central directory is: `(offset, size, entries)`, from the
/// archive's last bytes. `zip64_record` gives the 56 bytes of the zip64 end
/// record at an offset, for archives over 4 GiB.
pub fn central_directory_location(tail: &[u8], zip64_record: impl FnOnce(u64) -> Result<Vec<u8>>) -> Result<(u64, u64, u64)> {
    let at = find(tail, END_OF_CD).ok_or_else(|| anyhow!("the installer has no zip directory"))?;
    let eocd = &tail[at..];
    let (entries, size, offset) = (u16_at(eocd, 10) as u64, u32_at(eocd, 12) as u64, u32_at(eocd, 16) as u64);
    if offset != 0xFFFF_FFFF && entries != 0xFFFF {
        return Ok((offset, size, entries));
    }
    let loc = find(&tail[..at], ZIP64_LOCATOR).ok_or_else(|| anyhow!("the installer's zip64 directory is missing"))?;
    let record = zip64_record(u64_at(&tail[loc..], 8))?;
    if !record.starts_with(ZIP64_END_OF_CD) {
        bail!("the installer's zip64 directory is damaged");
    }
    Ok((u64_at(&record, 48), u64_at(&record, 40), u64_at(&record, 32)))
}

/// The entries of a central directory (`count` records), with the place of
/// each one's data. Like gogdl, an entry's data is taken to end where the
/// next local header starts (GOG's archives have no data descriptors);
/// `cd_offset` closes the last one.
pub fn parse_central_directory(data: &[u8], count: u64, cd_offset: u64) -> Result<Vec<Entry>> {
    let mut out = Vec::with_capacity(count as usize);
    let mut at = 0usize;
    for _ in 0..count {
        let rec =
            data.get(at..).filter(|r| r.starts_with(CENTRAL_HEADER)).ok_or_else(|| anyhow!("the installer's zip directory is damaged"))?;
        let name_len = u16_at(rec, 28) as usize;
        let extra_len = u16_at(rec, 30) as usize;
        let comment_len = u16_at(rec, 32) as usize;
        let name = String::from_utf8_lossy(rec.get(46..46 + name_len).ok_or_else(|| anyhow!("truncated zip directory"))?).into_owned();
        let mut e = Entry {
            name,
            crc32: u32_at(rec, 16),
            compressed_size: u32_at(rec, 20) as u64,
            size: u32_at(rec, 24) as u64,
            method: u16_at(rec, 10),
            local_offset: u32_at(rec, 42) as u64,
            attributes: u32_at(rec, 38),
            data_offset: 0,
        };
        apply_zip64(&mut e, rec.get(46 + name_len..46 + name_len + extra_len).unwrap_or_default());
        out.push(e);
        at += 46 + name_len + extra_len + comment_len;
    }
    let mut order: Vec<usize> = (0..out.len()).collect();
    order.sort_by_key(|&i| out[i].local_offset);
    for (n, &i) in order.iter().enumerate() {
        let end = order.get(n + 1).map_or(cd_offset, |&j| out[j].local_offset);
        out[i].data_offset = end.saturating_sub(out[i].compressed_size);
    }
    Ok(out)
}

/// The zip64 extra field (id 1) holds the sizes and offset too big for
/// 32 bits, in this order, only for those that are.
fn apply_zip64(e: &mut Entry, mut extra: &[u8]) {
    while extra.len() >= 4 {
        let (id, len) = (u16_at(extra, 0), u16_at(extra, 2) as usize);
        let body = extra.get(4..4 + len).unwrap_or_default();
        if id == 1 {
            let mut at = 0;
            for field in [&mut e.size, &mut e.compressed_size, &mut e.local_offset] {
                if *field == 0xFFFF_FFFF && at + 8 <= body.len() {
                    *field = u64_at(body, at);
                    at += 8;
                }
            }
            return;
        }
        extra = extra.get(4 + len..).unwrap_or_default();
    }
}

/// An installer, opened: its address, where its archive starts and the
/// archive's entries.
#[derive(Debug, Clone)]
pub struct Installer {
    /// The game or DLC id.
    pub product: String,
    /// The CDN address (redirects followed).
    pub url: String,
    pub size: u64,
    /// Offset of the archive in the installer.
    pub archive_start: u64,
    pub entries: Vec<Entry>,
}

impl Installer {
    /// The entries of the game's files (`data/noarch/`, no folders).
    pub fn data_entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.game_path().is_some() && !e.is_dir())
    }

    /// `(download, disk)` bytes of the game's files.
    pub fn sizes(&self) -> (u64, u64) {
        self.data_entries().fold((0, 0), |(a, b), e| (a + e.compressed_size, b + e.size))
    }

    /// The file model of an entry (links are [`Client::link_of`]).
    pub fn file_of(&self, e: &Entry) -> Option<FileEntry> {
        let path = e.game_path()?.to_string();
        let source = if e.size == 0 {
            Source::Empty
        } else {
            Source::Zip {
                product: self.product.clone(),
                offset: self.archive_start + e.data_offset,
                compressed_size: e.compressed_size,
                deflated: e.method == 8,
            }
        };
        Some(FileEntry {
            path,
            root: Root::Game,
            size: e.size,
            source,
            checksum: Some(Checksum::Crc32(e.crc32)),
            executable: e.is_executable(),
        })
    }
}

/// The Linux installers of a product document (`downloads.installers`).
pub fn linux_installers(product: &Value) -> Vec<&Value> {
    product
        .pointer("/downloads/installers")
        .and_then(Value::as_array)
        .map(|l| l.iter().filter(|i| i.get("os").and_then(Value::as_str) == Some("linux")).collect())
        .unwrap_or_default()
}

/// gogdl's pick: the only one, else the install's language, else English
/// (English installers have every language).
pub fn pick_installer<'a>(installers: &[&'a Value], lang: &str) -> Option<&'a Value> {
    if installers.len() == 1 {
        return installers.first().copied();
    }
    let target = language::parse(lang);
    let of = |i: &Value| i.get("language").and_then(Value::as_str).unwrap_or_default().to_string();
    installers.iter().find(|i| target.is_some_and(|t| t.matches(&of(i)))).or_else(|| installers.iter().find(|i| of(i) == "en")).copied()
}

impl Client {
    /// The product document with its downloads and DLCs.
    pub fn product_downloads(&self, game_id: &str) -> Result<Value> {
        self.get_json(&api::product_url(game_id, &["downloads", "expanded_dlcs"]))
    }

    /// Opens `installer` (an element of `downloads.installers`) of `product`:
    /// follows its download link and reads its zip directory.
    pub fn open_installer(&self, installer: &Value, product: &str) -> Result<Installer> {
        let link =
            installer.pointer("/files/0/downlink").and_then(Value::as_str).ok_or_else(|| anyhow!("the installer has no download link"))?;
        let downlink = self.get_json(link)?;
        let target = downlink.get("downlink").and_then(Value::as_str).ok_or_else(|| anyhow!("GOG sent no download address"))?;
        let (url, size) = self.resolve(target)?;
        let cdn = Client::anonymous();
        let head = cdn.get_range(&url, 0, SCRIPT_SEARCH.min(size))?;
        let archive_start = head.windows(4).position(|w| w == LOCAL_HEADER).ok_or_else(|| anyhow!("the installer has no archive"))? as u64;
        let tail_len = 200.min(size);
        let tail = cdn.get_range(&url, size - tail_len, tail_len)?;
        let (cd_offset, cd_size, count) = central_directory_location(&tail, |at| cdn.get_range(&url, archive_start + at, 56))?;
        let cd = cdn.get_range(&url, archive_start + cd_offset, cd_size).context("reading the installer's zip directory")?;
        let entries = parse_central_directory(&cd, count, cd_offset)?;
        Ok(Installer { product: product.to_string(), url, size, archive_start, entries })
    }

    /// A link entry: its target is its data.
    pub fn link_of(&self, inst: &Installer, e: &Entry) -> Result<Link> {
        let data = self.read_entry(inst, e)?;
        Ok(Link { path: e.game_path().unwrap_or_default().to_string(), target: String::from_utf8_lossy(&data).into_owned() })
    }

    /// An entry's bytes, inflated and checked (small entries).
    pub fn read_entry(&self, inst: &Installer, e: &Entry) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(e.size as usize);
        copy_entry(self, inst, e, &mut out, &mut |_| {})?;
        Ok(out)
    }
}

/// Streams an entry's bytes into `out`, inflated and checked against its
/// crc32; `on_bytes` gets each count of compressed bytes read. For files too
/// big to hold in memory.
pub fn copy_entry(
    client: &Client,
    inst: &Installer,
    e: &Entry,
    out: &mut dyn std::io::Write,
    on_bytes: &mut dyn FnMut(u64),
) -> Result<u64> {
    let raw = client.open_range(&inst.url, inst.archive_start + e.data_offset, e.compressed_size)?;
    let mut counted = Counted { inner: raw, on_bytes };
    let mut reader: Box<dyn Read + '_> =
        if e.method == 8 { Box::new(flate2::read::DeflateDecoder::new(&mut counted)) } else { Box::new(&mut counted) };
    let mut crc = flate2::Crc::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut written = 0u64;
    loop {
        let n = reader.read(&mut buf).with_context(|| format!("reading {}", e.name))?;
        if n == 0 {
            break;
        }
        crc.update(&buf[..n]);
        out.write_all(&buf[..n])?;
        written += n as u64;
    }
    if written != e.size || crc.sum() != e.crc32 {
        bail!("{} came out damaged", e.name);
    }
    Ok(written)
}

struct Counted<'a, R> {
    inner: R,
    on_bytes: &'a mut dyn FnMut(u64),
}

impl<R: Read> Read for Counted<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        (self.on_bytes)(n as u64);
        Ok(n)
    }
}

/// What an install or update of the installers' files does, against the
/// game folder's `.gogdl-linux-manifest` (`old`): entries with another crc32
/// or new, and the names gone. Links are read separately.
pub fn diff(installers: &[Installer], old: Option<&Value>) -> (Diff, Vec<(String, String)>) {
    let mut out = Diff::default();
    let mut links = Vec::new();
    let previous: std::collections::HashMap<String, u64> = old
        .and_then(|m| m.get("files"))
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(|f| Some((f.get("file_name")?.as_str()?.to_string(), f.get("crc32")?.as_u64()?))).collect())
        .unwrap_or_default();
    let mut current = std::collections::HashSet::new();
    for inst in installers {
        for e in inst.data_entries() {
            current.insert(e.name.clone());
            if previous.get(&e.name).is_some_and(|crc| *crc == u64::from(e.crc32)) {
                continue;
            }
            if e.is_symlink() {
                links.push((inst.product.clone(), e.name.clone()));
            } else if let Some(f) = inst.file_of(e) {
                out.files.push(f);
            }
        }
    }
    for name in previous.keys().filter(|n| !current.contains(*n)) {
        if let Some(path) = name.strip_prefix(DATA_PREFIX) {
            out.deleted.push((Root::Game, path.to_string()));
        }
    }
    out.deleted.sort();
    (out, links)
}

/// The record an install leaves in the game's folder: every entry and the
/// md5 of `gameinfo` (if the installer has one).
pub fn record(installers: &[Installer], game_dir: &Path) -> Value {
    let files: Vec<Value> = installers.iter().flat_map(|i| i.data_entries().map(Entry::to_record)).collect();
    let mut doc = json!({"files": files});
    if let Ok(bytes) = std::fs::read(game_dir.join("gameinfo")) {
        use md5::Digest;
        doc["info_checksum"] = json!(hex::encode(md5::Md5::digest(&bytes)));
    }
    doc
}

/// Reads the folder's `.gogdl-linux-manifest`.
pub fn read_record(game_dir: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(game_dir.join(crate::stored::LINUX_MANIFEST)).ok()?).ok()
}

/// Writes it.
pub fn write_record(game_dir: &Path, doc: &Value) -> Result<()> {
    let path = game_dir.join(crate::stored::LINUX_MANIFEST);
    heronite_data::temps::write_atomic(&path, &serde_json::to_vec(doc)?, None).with_context(|| format!("writing {}", path.display()))
}

/// What a Linux installer's `gameinfo` says (lines: title, version, ...,
/// language, game id, ..., build id).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GameInfo {
    pub title: String,
    pub version: String,
    pub language: Option<String>,
    pub game_id: Option<String>,
    pub build_id: Option<String>,
}

pub fn parse_gameinfo(text: &str) -> GameInfo {
    let lines: Vec<&str> = text.split('\n').collect();
    let line = |i: usize| lines.get(i).map(|l| l.trim().to_string()).filter(|l| !l.is_empty());
    let long = lines.len() > 4;
    GameInfo {
        title: line(0).unwrap_or_default(),
        version: line(1).unwrap_or_default(),
        language: line(3),
        game_id: if long { line(4) } else { None },
        build_id: if long { line(6) } else { None },
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::io::Write;

    /// A zip with `(name, data, unix mode)` entries (deflated), preceded by
    /// `prefix` like an installer's script.
    pub fn installer(prefix: &[u8], entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
        let mut zip = Vec::new();
        let mut cd = Vec::new();
        for (name, data, mode) in entries {
            let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            z.write_all(data).unwrap();
            let comp = z.finish().unwrap();
            let mut crc = flate2::Crc::new();
            crc.update(data);
            let offset = zip.len() as u32;
            let header = |sig: &[u8], central: bool| {
                let mut h = sig.to_vec();
                if central {
                    h.extend(0x0314u16.to_le_bytes());
                }
                h.extend(20u16.to_le_bytes());
                h.extend(0u16.to_le_bytes());
                h.extend(8u16.to_le_bytes());
                h.extend([0u8; 4]);
                h.extend(crc.sum().to_le_bytes());
                h.extend((comp.len() as u32).to_le_bytes());
                h.extend((data.len() as u32).to_le_bytes());
                h.extend((name.len() as u16).to_le_bytes());
                h.extend(0u16.to_le_bytes());
                if central {
                    h.extend(0u16.to_le_bytes());
                    h.extend(0u16.to_le_bytes());
                    h.extend(0u16.to_le_bytes());
                    h.extend((mode << 16).to_le_bytes());
                    h.extend(offset.to_le_bytes());
                }
                h.extend(name.as_bytes());
                h
            };
            zip.extend(header(b"PK\x03\x04", false));
            zip.extend(&comp);
            cd.extend(header(b"PK\x01\x02", true));
        }
        let cd_offset = zip.len() as u32;
        zip.extend(&cd);
        zip.extend(b"PK\x05\x06");
        zip.extend([0u8; 4]);
        zip.extend((entries.len() as u16).to_le_bytes());
        zip.extend((entries.len() as u16).to_le_bytes());
        zip.extend((cd.len() as u32).to_le_bytes());
        zip.extend(cd_offset.to_le_bytes());
        zip.extend(0u16.to_le_bytes());
        let mut out = prefix.to_vec();
        out.extend(zip);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::testing::installer;
    use super::*;

    fn open(bytes: &[u8]) -> Installer {
        let start = bytes.windows(4).position(|w| w == LOCAL_HEADER).unwrap() as u64;
        let tail = &bytes[bytes.len().saturating_sub(200)..];
        let (cd_offset, cd_size, count) = central_directory_location(tail, |_| unreachable!()).unwrap();
        let cd = &bytes[(start + cd_offset) as usize..(start + cd_offset + cd_size) as usize];
        let entries = parse_central_directory(cd, count, cd_offset).unwrap();
        Installer { product: "1".into(), url: String::new(), size: bytes.len() as u64, archive_start: start, entries }
    }

    #[test]
    fn the_archive_is_read_after_the_script() {
        let bytes = installer(
            b"#!/bin/sh\necho mojo\n",
            &[
                ("data/noarch/start.sh", b"#!/bin/sh\nexec ./game\n", 0o100755),
                ("data/noarch/game/data.pak", &b"x".repeat(5000), 0o100644),
                ("data/noarch/game/lib.so", b"lib.so.1", 0o120777),
                ("scripts/config.lua", b"-- mojo", 0o100644),
            ],
        );
        let inst = open(&bytes);
        assert_eq!(inst.archive_start, 20);
        assert_eq!(inst.data_entries().count(), 3);
        let start = &inst.entries[0];
        assert!(start.is_executable() && !start.is_symlink());
        assert!(inst.entries[2].is_symlink());
        // The data offset found from the next header is where the bytes are.
        let e = &inst.entries[1];
        let at = (inst.archive_start + e.data_offset) as usize;
        let mut out = Vec::new();
        flate2::read::DeflateDecoder::new(&bytes[at..at + e.compressed_size as usize]).read_to_end(&mut out).unwrap();
        assert_eq!(out, b"x".repeat(5000));
        let f = inst.file_of(e).unwrap();
        assert_eq!((f.path.as_str(), f.size), ("game/data.pak", 5000));
        assert_eq!(f.checksum, Some(Checksum::Crc32(e.crc32)));
        assert!(matches!(f.source, Source::Zip { deflated: true, .. }));
        assert_eq!(inst.sizes().1, 5000 + 22 + 8);
    }

    #[test]
    fn updates_take_entries_with_another_crc() {
        let v1 = open(&installer(
            b"",
            &[("data/noarch/a", b"one", 0o100644), ("data/noarch/b", b"two", 0o100644), ("data/noarch/l", b"a", 0o120777)],
        ));
        let dir = tempfile::tempdir().unwrap();
        let rec = record(std::slice::from_ref(&v1), dir.path());
        let v2 = open(&installer(
            b"",
            &[("data/noarch/a", b"one", 0o100644), ("data/noarch/c", b"new", 0o100644), ("data/noarch/l", b"b", 0o120777)],
        ));
        let (d, links) = diff(std::slice::from_ref(&v2), Some(&rec));
        assert_eq!(d.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["c"]);
        assert_eq!(d.deleted, [(Root::Game, "b".to_string())]);
        assert_eq!(links, [("1".to_string(), "data/noarch/l".to_string())]);
        let (all, _) = diff(std::slice::from_ref(&v2), None);
        assert_eq!(all.files.len(), 2);
        write_record(dir.path(), &rec).unwrap();
        assert_eq!(read_record(dir.path()).unwrap(), rec);
    }

    #[test]
    fn gameinfo_and_installer_choice() {
        let info = parse_gameinfo("Some Game\n1.0.2\n2.0\nen-US\n1207658754\n1\n55555\n");
        assert_eq!(info.title, "Some Game");
        assert_eq!(
            (info.game_id.as_deref(), info.build_id.as_deref(), info.language.as_deref()),
            (Some("1207658754"), Some("55555"), Some("en-US"))
        );
        let short = parse_gameinfo("Old\n0.9\n\nen\n");
        assert_eq!((short.game_id, short.language.as_deref()), (None, Some("en")));
        let product = json!({"downloads": {"installers": [
            {"os": "windows", "language": "en"}, {"os": "linux", "language": "de"}, {"os": "linux", "language": "en"}
        ]}});
        let list = linux_installers(&product);
        assert_eq!(list.len(), 2);
        assert_eq!(pick_installer(&list, "de-DE").unwrap()["language"], "de");
        assert_eq!(pick_installer(&list, "fr-FR").unwrap()["language"], "en");
    }

    #[test]
    fn zip64_fields_replace_the_32_bit_ones() {
        let mut e = Entry {
            name: "x".into(),
            crc32: 0,
            compressed_size: 0xFFFF_FFFF,
            size: 0xFFFF_FFFF,
            method: 8,
            local_offset: 7,
            attributes: 0,
            data_offset: 0,
        };
        let mut extra = vec![1u8, 0, 16, 0];
        extra.extend(5_000_000_000u64.to_le_bytes());
        extra.extend(4_000_000_000u64.to_le_bytes());
        apply_zip64(&mut e, &extra);
        assert_eq!((e.size, e.compressed_size, e.local_offset), (5_000_000_000, 4_000_000_000, 7));
    }
}
