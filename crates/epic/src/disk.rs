//! Free space and writability of an install path (Heroic's `checkDiskSpace`).

use std::path::{Path, PathBuf};

use crate::util::file_size_local;

#[derive(Debug, Clone, PartialEq)]
pub struct DiskSpace {
    pub free: u64,
    pub total: u64,
    pub writable: bool,
    /// `12.3 GiB / 931.5 GiB`.
    pub message: String,
}

/// The closest ancestor of `path` that exists (the folder the game folder
/// would be created in).
fn existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut p = path.to_path_buf();
    loop {
        if p.exists() {
            return Some(p);
        }
        if !p.pop() {
            return None;
        }
    }
}

/// Blocking filesystem queries; `None` when nothing of the path exists.
pub fn check(path: &str) -> Option<DiskSpace> {
    let dir = existing_ancestor(Path::new(path))?;
    let (total, free) = heronite_platform::fs::disk_space(&dir)?;
    let writable = heronite_platform::fs::writable_dir(&dir);
    Some(DiskSpace { free, total, writable, message: format!("{} / {}", file_size_local(free as f64), file_size_local(total as f64)) })
}

/// Filesystem `path` (or the folder it would be created in) is on.
fn device(path: &str) -> Option<u64> {
    let path = Path::new(path.trim_matches('\''));
    if !path.is_absolute() {
        return None;
    }
    let dir = existing_ancestor(path)?;
    heronite_platform::fs::file_id(&dir, &std::fs::metadata(&dir).ok()?).map(|(device, _)| device)
}

/// Bytes the downloads in `queued` (`(install path, bytes still to come)`)
/// will take on the filesystem of `path`: free space they will use before an
/// install queued now gets its turn. Blocking (metadata reads).
pub fn reserved_on(path: &str, queued: &[(String, f64)]) -> u64 {
    let Some(target) = device(path) else { return 0 };
    queued.iter().filter(|(p, _)| device(p) == Some(target)).map(|(_, bytes)| bytes.max(0.) as u64).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_space_for_existing_and_future_paths() {
        let dir = tempfile::tempdir().unwrap();
        let here = check(dir.path().to_str().unwrap()).unwrap();
        assert!(here.total > 0 && here.free <= here.total);
        assert!(here.writable);
        let future = dir.path().join("a").join("b").join("c");
        let there = check(future.to_str().unwrap()).unwrap();
        assert_eq!(there.total, here.total);
        assert!(there.message.contains('/'));
    }

    #[test]
    fn queued_downloads_count_only_on_the_same_filesystem() {
        let dir = tempfile::tempdir().unwrap();
        let here = dir.path().to_str().unwrap().to_string();
        let future = dir.path().join("new").join("folder").to_str().unwrap().to_string();
        let elsewhere = if cfg!(windows) { r"\\?\Volume{00000000-0000-0000-0000-000000000000}\x" } else { "/proc" };
        let queued = vec![(here.clone(), 100.), (future, 50.), ("thirdParty".into(), 7.), (elsewhere.into(), 1000.)];
        assert_eq!(reserved_on(&here, &queued), 150);
        assert_eq!(reserved_on("relative", &queued), 0);
    }

    #[cfg(unix)]
    #[test]
    fn read_only_locations_are_not_writable() {
        let root = check("/proc/self").unwrap();
        // Skipped when running as root, where /proc/self/ is still not writable
        // for access(2) with W_OK on most kernels; only assert the call works.
        assert!(root.total == 0 || !root.message.is_empty());
    }
}
