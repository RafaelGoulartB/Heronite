//! What an install, update or repair has to do (legendary's
//! `DLManager.run_analysis`): the files to write and where each byte comes
//! from, the chunks to download, the files to delete.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use super::{ChunkInfo, FileManifest, Guid, Manifest};

/// What to install.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlanOptions<'a> {
    /// Install tags to install (`""` is the untagged files); `None` installs
    /// every file.
    pub install_tags: Option<&'a [String]>,
    /// Only these files are written (a repair of the files found broken).
    pub only: Option<&'a BTreeSet<String>>,
    /// The install folder: a file missing from it is written again even when
    /// the build did not change it.
    pub root: Option<&'a Path>,
    /// Read the bytes a changed file keeps from its installed version instead
    /// of downloading them.
    pub reuse: bool,
}

/// Where the bytes of a piece of a file come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannedPart {
    /// `size` bytes at `offset` of the decoded chunk `guid`.
    Chunk { guid: Guid, offset: u32, size: u32 },
    /// `size` bytes at `offset` of the file installed now at `path` (its old
    /// version), before it is replaced.
    Local { path: String, offset: u64, size: u64 },
}

/// A file to write.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedFile {
    /// Relative, with `/`.
    pub path: String,
    pub size: u64,
    /// SHA-1 of the finished file.
    pub sha1: [u8; 20],
    pub executable: bool,
    pub symlink: Option<String>,
    pub parts: Vec<PlannedPart>,
}

/// The work of one run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InstallPlan {
    /// In the order legendary writes them (by lower-case path).
    pub files: Vec<PlannedFile>,
    /// Relative paths to delete once every file is written: files the new
    /// build no longer has, and files of install tags not selected.
    pub remove: Vec<String>,
    /// Chunks to download, each once, in the order they are first needed.
    pub chunks: Vec<ChunkInfo>,
    /// Bytes on disk of the selected files once installed.
    pub install_size: u64,
    /// Bytes to download.
    pub download_size: u64,
    /// Bytes to write.
    pub write_size: u64,
    /// Bytes read from installed files instead of downloaded.
    pub reuse_size: u64,
    pub added: usize,
    pub changed: usize,
    pub unchanged: usize,
}

impl InstallPlan {
    /// Nothing to write or delete.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.remove.is_empty()
    }
}

/// Plans installing `new` over `old` (the manifest of what is installed now;
/// `None` for a new install or a repair against the same build).
pub fn plan(new: &Manifest, old: Option<&Manifest>, opts: &PlanOptions) -> InstallPlan {
    let old_hashes: HashMap<&str, &FileManifest> =
        old.map(|o| o.files.iter().map(|f| (f.filename.as_str(), f)).collect()).unwrap_or_default();
    let mut out = InstallPlan::default();

    let selected = |f: &FileManifest| opts.install_tags.is_none_or(|tags| f.selected_by(tags));
    let mut files: Vec<&FileManifest> = new.files.iter().collect();
    files.sort_by_key(|f| f.filename.to_lowercase());

    let mut write: Vec<(&FileManifest, Option<&FileManifest>)> = Vec::new();
    for f in &files {
        if !selected(f) {
            // Deselected files go (a no-op when they were never installed).
            out.remove.push(f.filename.clone());
            continue;
        }
        out.install_size += f.size();
        if let Some(only) = opts.only {
            if only.contains(&f.filename) {
                out.changed += 1;
                write.push((f, None));
            } else {
                out.unchanged += 1;
            }
            continue;
        }
        let missing = opts.root.is_some_and(|root| std::fs::symlink_metadata(root.join(&f.filename)).is_err());
        match old_hashes.get(f.filename.as_str()) {
            Some(prev) if prev.hash == f.hash && !missing => out.unchanged += 1,
            Some(prev) if !missing => {
                out.changed += 1;
                write.push((f, Some(*prev)));
            }
            _ => {
                out.added += 1;
                write.push((f, None));
            }
        }
    }
    if opts.only.is_none() {
        if let Some(old) = old {
            let kept: HashSet<&str> = new.files.iter().map(|f| f.filename.as_str()).collect();
            out.remove.extend(old.files.iter().filter(|f| !kept.contains(f.filename.as_str())).map(|f| f.filename.clone()));
        }
    }

    let chunks: HashMap<Guid, &ChunkInfo> = new.chunks.iter().map(|c| (c.guid, c)).collect();
    let mut queued: HashSet<Guid> = HashSet::new();
    for (f, prev) in write {
        // Where each chunk part of the old version sits in the old file.
        let mut old_parts: HashMap<Guid, Vec<(u64, u32, u32)>> = HashMap::new();
        if let (true, Some(prev)) = (opts.reuse, prev) {
            let mut at = 0u64;
            for p in &prev.chunk_parts {
                old_parts.entry(p.guid).or_default().push((at, p.offset, p.offset + p.size));
                at += p.size as u64;
            }
        }
        let mut parts = Vec::with_capacity(f.chunk_parts.len());
        for p in &f.chunk_parts {
            let local = old_parts.get(&p.guid).and_then(|spans| {
                spans
                    .iter()
                    .find(|(_, start, end)| *start <= p.offset && p.offset + p.size <= *end)
                    .map(|(at, start, _)| at + (p.offset - start) as u64)
            });
            match local {
                Some(offset) => {
                    out.reuse_size += p.size as u64;
                    parts.push(PlannedPart::Local { path: f.filename.clone(), offset, size: p.size as u64 });
                }
                None => {
                    if queued.insert(p.guid) {
                        if let Some(c) = chunks.get(&p.guid) {
                            out.download_size += c.file_size;
                            out.chunks.push((*c).clone());
                        }
                    }
                    parts.push(PlannedPart::Chunk { guid: p.guid, offset: p.offset, size: p.size });
                }
            }
        }
        out.write_size += f.size();
        out.files.push(PlannedFile {
            path: f.filename.clone(),
            size: f.size(),
            sha1: f.hash,
            executable: f.executable(),
            symlink: Some(f.symlink_target.clone()).filter(|s| !s.is_empty()),
            parts,
        });
    }
    out
}

/// `(tag, bytes on disk, file count)` and `(tag, bytes to download, chunk
/// count)` of every install tag (legendary's `tag_disk_size` and
/// `tag_download_size`); empty when the build has no tags.
pub fn tag_sizes(m: &Manifest) -> (Vec<(String, u64, usize)>, Vec<(String, u64, usize)>) {
    let tags = m.install_tags();
    if tags.len() <= 1 {
        return (Vec::new(), Vec::new());
    }
    let sizes: HashMap<Guid, u64> = m.chunks.iter().map(|c| (c.guid, c.file_size)).collect();
    let mut disk = Vec::new();
    let mut download = Vec::new();
    for tag in tags {
        let files: Vec<&FileManifest> = m.files.iter().filter(|f| f.selected_by(std::slice::from_ref(&tag))).collect();
        disk.push((tag.clone(), files.iter().map(|f| f.size()).sum(), files.len()));
        let guids: HashSet<Guid> = files.iter().flat_map(|f| f.chunk_parts.iter().map(|p| p.guid)).collect();
        download.push((tag, guids.iter().filter_map(|g| sizes.get(g)).sum(), guids.len()));
    }
    (disk, download)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::tests::sample;
    use crate::manifest::ChunkPart;

    #[test]
    fn a_new_install_writes_every_selected_file_and_fetches_each_chunk_once() {
        let m = sample();
        let p = plan(&m, None, &PlanOptions::default());
        assert_eq!(p.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["Control.exe", "data/a.pak", "lang/de.pak"]);
        assert_eq!(p.chunks.len(), 2);
        assert_eq!(p.download_size, 1500);
        assert_eq!(p.install_size, 250);
        assert!(p.remove.is_empty());
        assert_eq!(p.added, 3);

        let untagged = vec![String::new()];
        let p = plan(&m, None, &PlanOptions { install_tags: Some(&untagged), ..Default::default() });
        assert_eq!(p.files.len(), 2);
        assert_eq!(p.remove, ["lang/de.pak"], "deselected files are removed");
        assert_eq!(p.install_size, 220);
    }

    #[test]
    fn an_update_keeps_unchanged_files_and_reuses_old_bytes() {
        let old = sample();
        let mut new = sample();
        // a.pak changes: its first part is the same, the second comes from a new chunk.
        let c3 = ChunkInfo { guid: Guid([9, 9, 9, 9]), file_size: 333, ..Default::default() };
        new.chunks.push(c3.clone());
        new.files[1].hash = [0xAA; 20];
        new.files[1].chunk_parts[1] = ChunkPart { guid: c3.guid, offset: 0, size: 70 };
        new.files.remove(2);
        let p = plan(&new, Some(&old), &PlanOptions { reuse: true, ..Default::default() });
        assert_eq!(p.unchanged, 1);
        assert_eq!(p.changed, 1);
        assert_eq!(p.remove, ["lang/de.pak"]);
        assert_eq!(p.files.len(), 1);
        assert_eq!(
            p.files[0].parts,
            [
                PlannedPart::Local { path: "data/a.pak".into(), offset: 0, size: 50 },
                PlannedPart::Chunk { guid: c3.guid, offset: 0, size: 70 }
            ]
        );
        assert_eq!(p.chunks, [c3]);
        assert_eq!(p.reuse_size, 50);
        assert_eq!(p.download_size, 333);
        // Without reuse every part is downloaded.
        let p = plan(&new, Some(&old), &PlanOptions::default());
        assert_eq!(p.chunks.len(), 2);
    }

    #[test]
    fn missing_files_are_written_again_and_repairs_write_only_what_is_asked() {
        let m = sample();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Control.exe"), b"x").unwrap();
        let p = plan(&m, Some(&m), &PlanOptions { root: Some(dir.path()), ..Default::default() });
        assert_eq!(p.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["data/a.pak", "lang/de.pak"]);

        let only: BTreeSet<String> = ["Control.exe".to_string()].into();
        let p = plan(&m, Some(&m), &PlanOptions { only: Some(&only), ..Default::default() });
        assert_eq!(p.files.len(), 1);
        assert_eq!(p.chunks.len(), 1);
        assert!(p.remove.is_empty());
    }

    #[test]
    fn sizes_per_install_tag() {
        let (disk, download) = tag_sizes(&sample());
        assert_eq!(disk, [("".to_string(), 220, 2), ("de".to_string(), 30, 1)]);
        assert_eq!(download, [("".to_string(), 1500, 2), ("de".to_string(), 500, 1)]);
        let mut plain = sample();
        plain.files.remove(2);
        assert_eq!(tag_sizes(&plain), (vec![], vec![]));
    }
}
