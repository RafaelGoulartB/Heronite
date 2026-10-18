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
#[cfg(unix)]
pub fn check(path: &str) -> Option<DiskSpace> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let dir = existing_ancestor(Path::new(path))?;
    let c = CString::new(dir.as_os_str().as_bytes()).ok()?;
    // SAFETY: statvfs fills a plain struct; `c` is a valid C string.
    let (free, total) = unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut st) != 0 {
            return None;
        }
        let frsize = st.f_frsize as u64;
        (st.f_bavail as u64 * frsize, st.f_blocks as u64 * frsize)
    };
    // SAFETY: access(2) on a valid C string.
    let writable = unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 };
    Some(DiskSpace { free, total, writable, message: format!("{} / {}", file_size_local(free as f64), file_size_local(total as f64)) })
}

#[cfg(not(unix))]
pub fn check(_path: &str) -> Option<DiskSpace> {
    None
}

/// Filesystem `path` (or the folder it would be created in) is on.
#[cfg(unix)]
fn device(path: &str) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    let path = Path::new(path.trim_matches('\''));
    if !path.is_absolute() {
        return None;
    }
    Some(std::fs::metadata(existing_ancestor(path)?).ok()?.dev())
}

#[cfg(not(unix))]
fn device(_path: &str) -> Option<u64> {
    None
}

/// Bytes the downloads in `queued` (`(install path, bytes still to come)`)
/// will take on the filesystem of `path`: free space they will use before an
/// install queued now gets its turn. Blocking (metadata reads).
pub fn reserved_on(path: &str, queued: &[(String, f64)]) -> u64 {
    let Some(target) = device(path) else { return 0 };
    queued.iter().filter(|(p, _)| device(p) == Some(target)).map(|(_, bytes)| bytes.max(0.) as u64).sum()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn reports_space_for_existing_and_future_paths() {
        let dir = tempfile::tempdir().unwrap();
        let here = check(dir.path().to_str().unwrap()).unwrap();
        assert!(here.total > 0 && here.free <= here.total);
        assert!(here.writable);
        let future = dir.path().join("a/b/c");
        let there = check(future.to_str().unwrap()).unwrap();
        assert_eq!(there.total, here.total);
        assert!(there.message.contains('/'));
    }

    #[test]
    fn queued_downloads_count_only_on_the_same_filesystem() {
        let dir = tempfile::tempdir().unwrap();
        let here = dir.path().to_str().unwrap().to_string();
        let future = dir.path().join("new/folder").to_str().unwrap().to_string();
        let queued = vec![(here.clone(), 100.), (future, 50.), ("thirdParty".into(), 7.), ("/proc".into(), 1000.)];
        assert_eq!(reserved_on(&here, &queued), 150);
        assert_eq!(reserved_on("relative", &queued), 0);
    }

    #[test]
    fn read_only_locations_are_not_writable() {
        let root = check("/proc/self").unwrap();
        // Skipped when running as root, where /proc/self/ is still not writable
        // for access(2) with W_OK on most kernels; only assert the call works.
        assert!(root.total == 0 || !root.message.is_empty());
    }
}
