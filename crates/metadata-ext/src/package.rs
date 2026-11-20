//! Reading an extension's package: its folder, or a zip of it (the files at
//! the top, or in one folder at the top). Nothing outside the expected
//! files is accepted: no other names, no folders, no links, no path that
//! leaves the package, nothing over its size.

use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::manifest::Manifest;
use crate::ExtensionError;

pub const MANIFEST: &str = "source.toml";
pub const SCRIPT: &str = "source.js";
pub const README: &str = "README.md";
pub const ICONS: [&str; 2] = ["icon.png", "icon.svg"];

/// The largest zip file read.
pub const MAX_ARCHIVE: u64 = 2 * 1024 * 1024;

/// The largest size of each file a package may have.
fn cap(name: &str) -> Option<u64> {
    Some(match name {
        MANIFEST => 64 * 1024,
        SCRIPT => 512 * 1024,
        README => 64 * 1024,
        n if ICONS.contains(&n) => 256 * 1024,
        _ => return None,
    })
}

/// A package's files, read and checked (the manifest parsed; the script
/// is compiled by whoever runs it).
#[derive(Debug, Clone)]
pub struct Package {
    pub manifest: Manifest,
    pub manifest_text: String,
    pub script: String,
    pub readme: Option<String>,
    /// `icon.png` or `icon.svg` and its bytes.
    pub icon: Option<(String, Vec<u8>)>,
    /// SHA-256 (hex) of the zip file as given, or for a folder of its
    /// files: each name, a zero byte, its length (8 bytes, little endian)
    /// and its bytes, in name order.
    pub sha256: String,
}

impl Package {
    /// Every file, as installed.
    pub fn files(&self) -> Vec<(String, Vec<u8>)> {
        let mut out =
            vec![(MANIFEST.to_owned(), self.manifest_text.clone().into_bytes()), (SCRIPT.to_owned(), self.script.clone().into_bytes())];
        if let Some(r) = &self.readme {
            out.push((README.to_owned(), r.clone().into_bytes()));
        }
        if let Some((name, bytes)) = &self.icon {
            out.push((name.clone(), bytes.clone()));
        }
        out
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn content_digest(files: &[(String, Vec<u8>)]) -> String {
    let mut sorted: Vec<&(String, Vec<u8>)> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut h = Sha256::new();
    for (name, bytes) in sorted {
        h.update(name.as_bytes());
        h.update([0]);
        h.update((bytes.len() as u64).to_le_bytes());
        h.update(bytes);
    }
    hex(&h.finalize())
}

/// What the host keeps beside an installed extension's files.
pub const RECORD: &str = "install.json";

/// Reads an installed extension's folder (its files and [`RECORD`]).
pub fn read_installed(dir: &Path) -> Result<Package, ExtensionError> {
    let files = read_folder(dir, &[RECORD])?;
    let digest = content_digest(&files);
    assemble(files, digest)
}

/// Reads the package at `path` (a folder or a `.zip`).
pub fn read(path: &Path) -> Result<Package, ExtensionError> {
    let meta = std::fs::metadata(path).map_err(|e| ExtensionError::NotAPackage(format!("{}: {e}", path.display())))?;
    if meta.is_dir() {
        let files = read_folder(path, &[])?;
        let digest = content_digest(&files);
        assemble(files, digest)
    } else {
        if meta.len() > MAX_ARCHIVE {
            return Err(ExtensionError::TooLarge(format!("the zip is larger than {MAX_ARCHIVE} bytes")));
        }
        let bytes = std::fs::read(path).map_err(|e| ExtensionError::NotAPackage(format!("{}: {e}", path.display())))?;
        read_zip(&bytes)
    }
}

/// Reads a package from the bytes of its zip.
pub fn read_zip(bytes: &[u8]) -> Result<Package, ExtensionError> {
    if bytes.len() as u64 > MAX_ARCHIVE {
        return Err(ExtensionError::TooLarge(format!("the zip is larger than {MAX_ARCHIVE} bytes")));
    }
    let digest = hex(&Sha256::digest(bytes));
    let files = unzip(bytes)?;
    assemble(files, digest)
}

fn check_name(name: &str) -> Result<u64, ExtensionError> {
    cap(name).ok_or_else(|| ExtensionError::UnexpectedFile(name.to_owned()))
}

fn read_folder(dir: &Path, skip: &[&str]) -> Result<Vec<(String, Vec<u8>)>, ExtensionError> {
    let entries = std::fs::read_dir(dir).map_err(|e| ExtensionError::NotAPackage(format!("{}: {e}", dir.display())))?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| ExtensionError::NotAPackage(e.to_string()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // Version control and editor files in a working folder.
        if name.starts_with('.') || skip.contains(&name.as_str()) {
            continue;
        }
        let kind = entry.file_type().map_err(|e| ExtensionError::NotAPackage(e.to_string()))?;
        if kind.is_symlink() {
            return Err(ExtensionError::UnexpectedFile(format!("{name} (a link)")));
        }
        if kind.is_dir() {
            return Err(ExtensionError::UnexpectedFile(format!("{name}/ (a folder)")));
        }
        let max = check_name(&name)?;
        let file = std::fs::File::open(entry.path()).map_err(|e| ExtensionError::NotAPackage(format!("{name}: {e}")))?;
        files.push((name.clone(), read_capped(file, max, &name)?));
    }
    Ok(files)
}

fn read_capped(r: impl Read, max: u64, name: &str) -> Result<Vec<u8>, ExtensionError> {
    let mut bytes = Vec::new();
    r.take(max + 1).read_to_end(&mut bytes).map_err(|e| ExtensionError::NotAPackage(format!("{name}: {e}")))?;
    if bytes.len() as u64 > max {
        return Err(ExtensionError::TooLarge(format!("{name} is larger than {max} bytes")));
    }
    Ok(bytes)
}

/// A path inside the zip as plain parts; refuses absolute paths, `..`,
/// drive letters and backslashes.
fn zip_parts(raw: &str) -> Result<Vec<&str>, ExtensionError> {
    let unsafe_path = || ExtensionError::UnsafePath(raw.to_owned());
    if raw.starts_with('/') || raw.contains('\\') || raw.contains(':') || raw.contains('\0') {
        return Err(unsafe_path());
    }
    let parts: Vec<&str> = raw.split('/').filter(|p| !p.is_empty()).collect();
    if parts.iter().any(|p| *p == ".." || *p == ".") {
        return Err(unsafe_path());
    }
    Ok(parts)
}

fn unzip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, ExtensionError> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| ExtensionError::NotAPackage(e.to_string()))?;
    if zip.len() > 32 {
        return Err(ExtensionError::UnexpectedFile(format!("{} entries", zip.len())));
    }
    let mut entries: Vec<(Vec<String>, usize)> = Vec::new();
    for i in 0..zip.len() {
        let file = zip.by_index_raw(i).map_err(|e| ExtensionError::NotAPackage(e.to_string()))?;
        let raw = file.name().map_err(|e| ExtensionError::NotAPackage(e.to_string()))?.into_owned();
        let parts: Vec<String> = zip_parts(&raw)?.into_iter().map(str::to_owned).collect();
        // What archivers on macOS add.
        if parts.first().is_some_and(|p| p == "__MACOSX") || parts.last().is_some_and(|p| p == ".DS_Store") {
            continue;
        }
        if file.is_symlink() {
            return Err(ExtensionError::UnexpectedFile(format!("{raw} (a link)")));
        }
        if file.encrypted() {
            return Err(ExtensionError::UnexpectedFile(format!("{raw} (encrypted)")));
        }
        if file.is_dir() || parts.is_empty() {
            continue;
        }
        entries.push((parts, i));
    }
    // The files at the top, or all in one folder at the top.
    let prefix = match entries.first() {
        Some((first, _)) if first.len() == 2 && entries.iter().all(|(p, _)| p.len() == 2 && p[0] == first[0]) => 1,
        _ => 0,
    };
    let mut files = Vec::new();
    for (parts, i) in entries {
        let rest = &parts[prefix..];
        if rest.len() != 1 {
            return Err(ExtensionError::UnexpectedFile(parts.join("/")));
        }
        let name = rest[0].clone();
        let max = check_name(&name)?;
        if files.iter().any(|(n, _): &(String, Vec<u8>)| *n == name) {
            return Err(ExtensionError::UnexpectedFile(format!("{name} (twice)")));
        }
        let file = zip.by_index(i).map_err(|e| ExtensionError::NotAPackage(e.to_string()))?;
        if file.size() > max {
            return Err(ExtensionError::TooLarge(format!("{name} is larger than {max} bytes")));
        }
        files.push((name.clone(), read_capped(file, max, &name)?));
    }
    Ok(files)
}

fn assemble(files: Vec<(String, Vec<u8>)>, sha256: String) -> Result<Package, ExtensionError> {
    let take = |name: &str| files.iter().find(|(n, _)| n == name).map(|(_, b)| b.clone());
    let utf8 = |name: &str, b: Vec<u8>| String::from_utf8(b).map_err(|_| ExtensionError::NotAPackage(format!("{name} is not UTF-8 text")));
    let manifest_text = utf8(MANIFEST, take(MANIFEST).ok_or_else(|| ExtensionError::MissingFile(MANIFEST.into()))?)?;
    let script = utf8(SCRIPT, take(SCRIPT).ok_or_else(|| ExtensionError::MissingFile(SCRIPT.into()))?)?;
    let readme = take(README).map(|b| utf8(README, b)).transpose()?;
    let icons: Vec<&str> = ICONS.iter().copied().filter(|n| take(n).is_some()).collect();
    if icons.len() > 1 {
        return Err(ExtensionError::UnexpectedFile("both icon.png and icon.svg".into()));
    }
    let icon = icons.first().and_then(|n| take(n).map(|b| (n.to_string(), b)));
    let manifest = Manifest::parse(&manifest_text)?;
    Ok(Package { manifest, manifest_text, script, readme, icon, sha256 })
}

#[cfg(test)]
#[path = "package_tests.rs"]
mod tests;
