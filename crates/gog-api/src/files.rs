//! The files of a build, whatever generation or installer they come from,
//! and what changes between two builds. Nothing here downloads or writes:
//! the download engine turns a [`Diff`] into work.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use anyhow::Result;
use md5::{Digest, Md5};
use sha2::Sha256;

/// The folder a file goes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Root {
    /// The game's folder.
    Game,
    /// The game's support folder (installer scripts, its own redist); the
    /// path starts with the product id.
    Support,
}

/// One chunk of the content system (v2): zlib data stored under its
/// `compressed_md5`, `size` bytes once inflated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkRef {
    pub md5: String,
    pub compressed_md5: String,
    pub size: u64,
    pub compressed_size: u64,
}

/// A piece of a file: `size` bytes from `offset` of a decoded chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkPart {
    pub chunk: ChunkRef,
    pub offset: u64,
    pub size: u64,
    /// During an update: where the same bytes are in the installed file of
    /// the same path (read there instead of downloaded).
    pub old_offset: Option<u64>,
}

/// Where a file's bytes come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Chunks of the content system, through the secure link of `product`
    /// (`redist` for redistributables).
    Chunks { product: String, parts: Vec<ChunkPart> },
    /// v1: `size` bytes of `product`'s `main.bin` from `offset`.
    Range { product: String, offset: u64 },
    /// A Linux installer: the entry's data at `offset` of `product`'s
    /// installer, `compressed_size` bytes, deflated or stored.
    Zip { product: String, offset: u64, compressed_size: u64, deflated: bool },
    /// An empty file.
    Empty,
}

/// A whole-file checksum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Checksum {
    /// Lower-case hex.
    Md5(String),
    /// Lower-case hex.
    Sha256(String),
    Crc32(u32),
}

/// One file of a build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Relative to its [`Root`], `/` separated.
    pub path: String,
    pub root: Root,
    /// Bytes on disk.
    pub size: u64,
    pub source: Source,
    pub checksum: Option<Checksum>,
    pub executable: bool,
}

impl FileEntry {
    /// Bytes to download for it.
    pub fn download_size(&self) -> u64 {
        match &self.source {
            Source::Chunks { parts, .. } => {
                let mut seen = std::collections::HashSet::new();
                parts
                    .iter()
                    .filter(|p| p.old_offset.is_none() && seen.insert(&p.chunk.compressed_md5))
                    .map(|p| p.chunk.compressed_size)
                    .sum()
            }
            Source::Range { .. } => self.size,
            Source::Zip { compressed_size, .. } => *compressed_size,
            Source::Empty => 0,
        }
    }

    /// The key gogdl compared files by: the path in lower case.
    pub fn key(&self) -> String {
        self.path.to_lowercase()
    }

    /// The md5 of each piece, for files with no whole-file checksum (v2
    /// files of several chunks): `(size, md5)` in order.
    pub fn piece_sums(&self) -> Vec<(u64, String)> {
        match &self.source {
            Source::Chunks { parts, .. } if parts.iter().all(|p| p.offset == 0 && p.size == p.chunk.size) => {
                parts.iter().map(|p| (p.size, p.chunk.md5.clone())).collect()
            }
            _ => Vec::new(),
        }
    }
}

/// A symbolic link (Unix only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// Relative to the game's folder.
    pub path: String,
    /// As stored: relative to the link's folder.
    pub target: String,
}

/// The items of a build's depots.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Items {
    pub files: Vec<FileEntry>,
    pub links: Vec<Link>,
    pub dirs: Vec<String>,
}

/// What an install or update does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    /// Files to write (new or changed).
    pub files: Vec<FileEntry>,
    /// Files of the old build that are gone.
    pub deleted: Vec<(Root, String)>,
    pub links: Vec<Link>,
    /// Folders the build has even when empty, relative to the game's folder.
    pub dirs: Vec<String>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.deleted.is_empty() && self.links.is_empty()
    }

    /// Bytes to download and bytes written.
    pub fn sizes(&self) -> (u64, u64) {
        (self.files.iter().map(FileEntry::download_size).sum(), self.files.iter().map(|f| f.size).sum())
    }
}

/// The path of an item as stored: `\` becomes `/`, no leading separator.
pub fn clean_path(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches('/').to_string()
}

/// Whether a file on disk still has the bytes of `entry`: whole checksum
/// first, else every chunk's md5, else just the size. `Ok(false)` when the
/// file is missing.
pub fn matches_on_disk(path: &Path, entry: &FileEntry) -> Result<bool> {
    let Ok(meta) = std::fs::metadata(path) else { return Ok(false) };
    if meta.len() != entry.size {
        return Ok(false);
    }
    let mut file = std::fs::File::open(path)?;
    match &entry.checksum {
        Some(Checksum::Md5(want)) => Ok(&digest::<Md5>(&mut file)? == want),
        Some(Checksum::Sha256(want)) => Ok(&digest::<Sha256>(&mut file)? == want),
        Some(Checksum::Crc32(want)) => {
            let mut crc = flate2::Crc::new();
            let mut buf = vec![0u8; 1 << 20];
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                crc.update(&buf[..n]);
            }
            Ok(crc.sum() == *want)
        }
        None => {
            for (size, want) in entry.piece_sums() {
                let mut piece = vec![0u8; size as usize];
                file.read_exact(&mut piece)?;
                if hex::encode(Md5::digest(&piece)) != want {
                    return Ok(false);
                }
            }
            Ok(true)
        }
    }
}

fn digest<D: Digest>(r: &mut impl Read) -> Result<String> {
    let mut h = D::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// Files by [`FileEntry::key`].
pub(crate) fn by_key(files: &[FileEntry]) -> HashMap<String, &FileEntry> {
    files.iter().map(|f| (f.key(), f)).collect()
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    pub fn chunk(md5: &str, size: u64) -> ChunkRef {
        ChunkRef { md5: md5.into(), compressed_md5: format!("z{md5}"), size, compressed_size: size / 2 }
    }

    pub fn file(path: &str, chunks: &[(&str, u64)]) -> FileEntry {
        let parts: Vec<ChunkPart> =
            chunks.iter().map(|(m, s)| ChunkPart { chunk: chunk(m, *s), offset: 0, size: *s, old_offset: None }).collect();
        FileEntry {
            path: path.into(),
            root: Root::Game,
            size: chunks.iter().map(|c| c.1).sum(),
            source: Source::Chunks { product: "1".into(), parts },
            checksum: None,
            executable: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    #[test]
    fn paths_are_cleaned() {
        assert_eq!(clean_path("\\bin\\game.exe"), "bin/game.exe");
        assert_eq!(clean_path("/data/x"), "data/x");
    }

    #[test]
    fn shared_chunks_download_once_and_reused_ones_not_at_all() {
        let mut f = file("a", &[("1", 100), ("1", 100), ("2", 50)]);
        assert_eq!(f.download_size(), 75);
        if let Source::Chunks { parts, .. } = &mut f.source {
            parts[2].old_offset = Some(0);
        }
        assert_eq!(f.download_size(), 50);
        let d = Diff { files: vec![f], ..Default::default() };
        assert_eq!(d.sizes(), (50, 250));
    }

    #[test]
    fn files_on_disk_are_checked_by_whole_sum_or_by_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        std::fs::write(&p, b"helloworld").unwrap();
        let md5 = |b: &[u8]| hex::encode(Md5::digest(b));
        let mut e = file("f", &[(&md5(b"hello"), 5), (&md5(b"world"), 5)]);
        assert!(matches_on_disk(&p, &e).unwrap());
        e.checksum = Some(Checksum::Md5(md5(b"helloworld")));
        assert!(matches_on_disk(&p, &e).unwrap());
        e.checksum = Some(Checksum::Md5(md5(b"nope")));
        assert!(!matches_on_disk(&p, &e).unwrap());
        let mut crc = flate2::Crc::new();
        crc.update(b"helloworld");
        e.checksum = Some(Checksum::Crc32(crc.sum()));
        assert!(matches_on_disk(&p, &e).unwrap());
        e.checksum = None;
        std::fs::write(&p, b"hellowXrld").unwrap();
        assert!(!matches_on_disk(&p, &e).unwrap());
        assert!(!matches_on_disk(&dir.path().join("missing"), &e).unwrap());
    }
}
