//! What an install, update or repair writes: the files, the pieces each is
//! made of and where those pieces come from.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::hash::Hash;

/// Everything one run writes into an install folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// The chunks [`Part::Chunk`] refers to, by index.
    pub chunks: Vec<Chunk>,
    /// Files to write (new or changed), in the order they are written.
    pub files: Vec<FileTask>,
    /// Files of the old version that the new one no longer has, relative to
    /// the install folder. Deleted once every file is in place.
    pub remove: Vec<String>,
    /// Folders to create even when no file goes in them.
    pub dirs: Vec<String>,
}

/// A piece of data the store serves, decoded by the [`crate::Source`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Chunk {
    /// Opaque key for the source (a URL path, a GUID, a byte range...).
    pub id: String,
    /// Bytes fetched over the network.
    pub download_size: u64,
    /// Bytes once decoded: the length the source must return.
    pub size: u64,
}

/// One file of the install.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileTask {
    /// Relative to the install folder, `/` between folders.
    pub path: String,
    pub size: u64,
    /// The file's bytes, in order; their sizes add up to `size`.
    pub parts: Vec<Part>,
    /// Checked once the file is written; `None` trusts the chunks' own checks.
    pub hash: Option<Hash>,
    pub executable: bool,
    /// A symbolic link to this target (relative to the link's folder)
    /// instead of a file: `size` is 0 and `parts` empty.
    pub symlink: Option<String>,
}

/// A run of bytes of a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Part {
    /// `size` bytes at `offset` of the decoded chunk `chunk` (an index into
    /// [`Plan::chunks`]).
    Chunk { chunk: usize, offset: u64, size: u64 },
    /// `size` bytes at `offset` of the file installed now at `path` (its old
    /// version, during an update), so unchanged data is not downloaded again.
    Local { path: String, offset: u64, size: u64 },
}

impl Part {
    pub fn size(&self) -> u64 {
        match self {
            Part::Chunk { size, .. } | Part::Local { size, .. } => *size,
        }
    }
}

impl Plan {
    /// Bytes to download: every chunk once.
    pub fn download_size(&self) -> u64 {
        self.used_chunks().map(|c| self.chunks[c].download_size).sum()
    }

    /// Bytes of the files the plan writes.
    pub fn write_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// The chunks some file uses, each once.
    fn used_chunks(&self) -> impl Iterator<Item = usize> + '_ {
        let mut seen = HashSet::new();
        self.files
            .iter()
            .flat_map(|f| f.parts.iter())
            .filter_map(|p| match p {
                Part::Chunk { chunk, .. } => Some(*chunk),
                Part::Local { .. } => None,
            })
            .filter(move |c| seen.insert(*c))
    }

    /// Checks that the plan is safe to write and holds together: paths stay
    /// inside the install folder, parts fit their chunks and add up to their
    /// file, no path is written twice, no file goes through a link the plan
    /// makes, and link targets stay inside the folder.
    pub fn validate(&self) -> Result<()> {
        let mut paths = HashSet::new();
        let mut links = HashSet::new();
        for f in &self.files {
            check_path(&f.path)?;
            if !paths.insert(key(&f.path)) {
                bail!("{} is written twice", f.path);
            }
            if let Some(target) = &f.symlink {
                if !f.parts.is_empty() || f.size != 0 {
                    bail!("the link {} has data", f.path);
                }
                check_link_target(&f.path, target)?;
                links.insert(key(&f.path));
                continue;
            }
            let mut total = 0u64;
            for p in &f.parts {
                match p {
                    Part::Chunk { chunk, offset, size } => {
                        let Some(c) = self.chunks.get(*chunk) else { bail!("{} uses chunk {chunk}, which the plan does not have", f.path) };
                        if offset.checked_add(*size).is_none_or(|end| end > c.size) {
                            bail!("{} reads past the end of chunk {}", f.path, c.id);
                        }
                    }
                    Part::Local { path, .. } => check_path(path)?,
                }
                total = total.checked_add(p.size()).ok_or_else(|| anyhow::anyhow!("{} is too big", f.path))?;
            }
            if total != f.size {
                bail!("the parts of {} add up to {total} bytes, not {}", f.path, f.size);
            }
        }
        for f in &self.files {
            let k = key(&f.path);
            let mut parent = k.as_str();
            while let Some((up, _)) = parent.rsplit_once('/') {
                if links.contains(up) {
                    bail!("{} goes through the link {up}", f.path);
                }
                parent = up;
            }
        }
        for r in &self.remove {
            check_path(r)?;
            if paths.contains(&key(r)) {
                bail!("{r} is both written and removed");
            }
        }
        for d in &self.dirs {
            check_path(d)?;
        }
        Ok(())
    }

    /// SHA-256 of everything the plan writes, in hex: a resume journal only
    /// applies to the very same plan.
    pub fn fingerprint(&self) -> String {
        let mut h = sha2::Sha256::new();
        let mut put = |bytes: &[u8]| {
            h.update((bytes.len() as u64).to_le_bytes());
            h.update(bytes);
        };
        for c in &self.chunks {
            put(c.id.as_bytes());
            put(&c.download_size.to_le_bytes());
            put(&c.size.to_le_bytes());
        }
        for f in &self.files {
            put(f.path.as_bytes());
            put(&f.size.to_le_bytes());
            put(&[f.executable as u8]);
            put(f.symlink.as_deref().unwrap_or("\0").as_bytes());
            put(f.hash.as_ref().map(Hash::bytes).unwrap_or(&[]));
            for p in &f.parts {
                match p {
                    Part::Chunk { chunk, offset, size } => {
                        put(b"c");
                        put(&(*chunk as u64).to_le_bytes());
                        put(&offset.to_le_bytes());
                        put(&size.to_le_bytes());
                    }
                    Part::Local { path, offset, size } => {
                        put(b"l");
                        put(path.as_bytes());
                        put(&offset.to_le_bytes());
                        put(&size.to_le_bytes());
                    }
                }
            }
        }
        for r in &self.remove {
            put(b"r");
            put(r.as_bytes());
        }
        for d in &self.dirs {
            put(b"d");
            put(d.as_bytes());
        }
        hex(&h.finalize())
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The separators of a relative path: `\` is one too on Windows, where it
/// would otherwise slip a `..` past the checks.
const SEPARATORS: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };

/// How two paths are told apart: as the file system does (without case on
/// Windows), with one kind of separator.
pub(crate) fn key(path: &str) -> String {
    let joined = path.split(SEPARATORS).collect::<Vec<_>>().join("/");
    if cfg!(windows) {
        joined.to_lowercase()
    } else {
        joined
    }
}

/// A path relative to the install folder that stays inside it.
pub fn check_path(path: &str) -> Result<()> {
    if path.is_empty() {
        bail!("an empty path");
    }
    if path.starts_with(SEPARATORS) || path.contains('\0') || (path.len() >= 2 && path.as_bytes()[1] == b':') {
        bail!("{path:?} is not a relative path");
    }
    for part in path.split(SEPARATORS) {
        if part.is_empty() || part == "." || part == ".." {
            bail!("{path:?} is not a plain relative path");
        }
        if cfg!(windows) && !windows_name_ok(part) {
            bail!("{path:?} has a name Windows does not allow");
        }
    }
    Ok(())
}

/// `name` as the name of a game's own folder, when it is one: a single
/// plain name (no separator of any system, not `.` or `..`, no drive, no
/// NUL, and a name Windows allows on Windows). Store catalogs name the
/// folder; anything else falls back to the game's id.
pub fn folder_name(name: &str) -> Option<&str> {
    let name = name.trim();
    (!name.contains(['/', '\\']) && check_path(name).is_ok()).then_some(name)
}

fn windows_name_ok(name: &str) -> bool {
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4",
        "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if name.chars().any(|c| c < ' ' || "<>:\"|?*".contains(c)) || name.ends_with(['.', ' ']) {
        return false;
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    !RESERVED.contains(&stem.as_str())
}

/// A link's target, relative to the link's folder, that resolves inside the
/// install folder. `..` may only lead the target: after a name it would step
/// back from wherever that name leads, and the name may be another link
/// (`d/y -> ..` makes `d/x -> y/../..` leave the folder).
fn check_link_target(link: &str, target: &str) -> Result<()> {
    if target.is_empty() || target.starts_with(SEPARATORS) || target.contains('\0') || (target.len() >= 2 && target.as_bytes()[1] == b':') {
        bail!("the link {link} points outside the install folder");
    }
    let mut depth: i64 = link.split(SEPARATORS).count() as i64 - 1;
    let mut named = false;
    for part in target.split(SEPARATORS) {
        match part {
            "" | "." => {}
            ".." if named => bail!("the link {link} steps back after a name ({target})"),
            ".." => depth -= 1,
            _ => {
                depth += 1;
                named = true;
            }
        }
        if depth < 0 {
            bail!("the link {link} points outside the install folder");
        }
    }
    Ok(())
}

/// `root` joined with the checked relative `path`.
pub(crate) fn join(root: &Path, path: &str) -> Result<PathBuf> {
    check_path(path)?;
    let mut out = root.to_path_buf();
    for part in path.split(SEPARATORS) {
        out.push(part);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_targets_cannot_step_back_through_other_links() {
        assert!(check_link_target("d/y", "..").is_ok());
        assert!(check_link_target("d/x", "y/../..").is_err());
        assert!(check_link_target("a/b/c", "../../lib/x.so").is_ok());
        assert!(check_link_target("a/b", "x/./y").is_ok());
        assert!(check_link_target("a", "../x").is_err());
    }

    #[test]
    fn store_folder_names_are_one_plain_name() {
        assert_eq!(folder_name(" Control "), Some("Control"));
        assert_eq!(folder_name("Game: The Sequel").is_some(), cfg!(unix));
        for bad in ["", ".", "..", "../x", "a/b", "a\\b", "/abs", "C:", "C:\\Games", "x\0"] {
            assert_eq!(folder_name(bad), None, "{bad:?}");
        }
    }

    fn file(path: &str, parts: Vec<Part>) -> FileTask {
        FileTask { path: path.into(), size: parts.iter().map(Part::size).sum(), parts, ..Default::default() }
    }

    fn chunk(size: u64) -> Chunk {
        Chunk { id: format!("c{size}"), download_size: size / 2, size }
    }

    #[test]
    fn paths_must_stay_inside_the_folder() {
        for bad in ["", "/etc/passwd", "../x", "a/../../x", "a//b", "./a", "a/", "C:/x", "a\0b"] {
            assert!(check_path(bad).is_err(), "{bad:?}");
        }
        for good in ["a", "a/b/c.txt", "Engine/Binaries/Win64/Game.exe", ".hidden/x", "a..b"] {
            check_path(good).unwrap();
        }
    }

    #[test]
    fn parts_must_fit_their_chunks_and_add_up() {
        let mut plan = Plan {
            chunks: vec![chunk(10)],
            files: vec![file("a", vec![Part::Chunk { chunk: 0, offset: 4, size: 6 }])],
            ..Default::default()
        };
        plan.validate().unwrap();
        plan.files[0].parts[0] = Part::Chunk { chunk: 0, offset: 5, size: 6 };
        assert!(plan.validate().is_err());
        plan.files[0].parts[0] = Part::Chunk { chunk: 1, offset: 0, size: 6 };
        assert!(plan.validate().is_err());
        plan.files[0].parts[0] = Part::Chunk { chunk: 0, offset: 0, size: 6 };
        plan.files[0].size = 7;
        assert!(plan.validate().is_err());
    }

    #[test]
    fn duplicates_links_and_removals_are_checked() {
        let twice = Plan { files: vec![file("a", vec![]), file("a", vec![])], ..Default::default() };
        assert!(twice.validate().is_err());
        let link = |target: &str| FileTask { path: "lib/libx.so".into(), symlink: Some(target.into()), ..Default::default() };
        Plan { files: vec![link("libx.so.1")], ..Default::default() }.validate().unwrap();
        Plan { files: vec![link("../bin/x")], ..Default::default() }.validate().unwrap();
        assert!(Plan { files: vec![link("../../x")], ..Default::default() }.validate().is_err());
        assert!(Plan { files: vec![link("/etc")], ..Default::default() }.validate().is_err());
        let through = Plan {
            files: vec![FileTask { path: "d".into(), symlink: Some("e".into()), ..Default::default() }, file("d/x", vec![])],
            ..Default::default()
        };
        assert!(through.validate().is_err());
        let both = Plan { files: vec![file("a", vec![])], remove: vec!["a".into()], ..Default::default() };
        assert!(both.validate().is_err());
    }

    #[test]
    fn sizes_count_every_chunk_once() {
        let plan = Plan {
            chunks: vec![chunk(10), chunk(20)],
            files: vec![
                file("a", vec![Part::Chunk { chunk: 0, offset: 0, size: 10 }, Part::Chunk { chunk: 1, offset: 0, size: 5 }]),
                file("b", vec![Part::Chunk { chunk: 1, offset: 5, size: 15 }, Part::Local { path: "old".into(), offset: 0, size: 3 }]),
            ],
            ..Default::default()
        };
        assert_eq!(plan.download_size(), 15);
        assert_eq!(plan.write_size(), 33);
    }

    #[test]
    fn fingerprints_follow_the_content() {
        let a = Plan {
            chunks: vec![chunk(4)],
            files: vec![file("a", vec![Part::Chunk { chunk: 0, offset: 0, size: 4 }])],
            ..Default::default()
        };
        let mut b = a.clone();
        assert_eq!(a.fingerprint(), b.fingerprint());
        b.files[0].executable = true;
        assert_ne!(a.fingerprint(), b.fingerprint());
    }
}
