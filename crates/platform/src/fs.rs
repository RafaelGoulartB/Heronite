//! Files: permission bits and identity.
//!
//! Windows has no Unix permission bits: changing them is a no-op there, and
//! a program is executable by its extension.

use std::fs::Metadata;
use std::io;
use std::path::Path;

/// Sets the Unix permission bits of `path` to `mode`.
pub fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Ok(())
    }
}

/// Adds the permission bits `bits` to `path` (`0o755`: anyone may run it).
pub fn add_mode(path: &Path, bits: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(perms.mode() | bits);
        std::fs::set_permissions(path, perms)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, bits);
        Ok(())
    }
}

/// The Unix permission bits (`0o7777` of the mode); `None` on Windows.
pub fn mode(meta: &Metadata) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(meta.permissions().mode() & 0o7777)
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        None
    }
}

/// Makes `path` runnable by its owner and readable by everyone (`0o755`).
pub fn make_executable(path: &Path) -> io::Result<()> {
    set_mode(path, 0o755)
}

/// Only the owner may open `path` (`0o700` for a folder, `0o600` for a file).
pub fn make_private(path: &Path) -> io::Result<()> {
    set_mode(path, if path.is_dir() { 0o700 } else { 0o600 })
}

/// Whether the file `meta` describes can be run: an execute bit on Unix, a
/// program's extension (`.exe`, `.bat`, `.cmd`, `.com`) on Windows.
pub fn is_executable(path: &Path, meta: &Metadata) -> bool {
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        let _ = path;
        mode(meta).is_some_and(|m| m & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.extension().and_then(|e| e.to_str()).is_some_and(|e| ["exe", "bat", "cmd", "com"].iter().any(|x| e.eq_ignore_ascii_case(x)))
    }
}

/// Makes `link` a symbolic link to `original`. Windows needs Developer Mode
/// or an administrator for it, and tells a link to a folder from one to a
/// file.
pub fn symlink(original: &Path, link: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(original, link)
    }
    #[cfg(windows)]
    {
        if original.is_dir() {
            std::os::windows::fs::symlink_dir(original, link)
        } else {
            std::os::windows::fs::symlink_file(original, link)
        }
    }
}

/// Whether the user may create files in the folder `dir`.
pub fn writable_dir(dir: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else { return false };
        // SAFETY: `c` is a valid NUL-terminated path for the duration of the call.
        unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
    }
    #[cfg(not(unix))]
    {
        // The read-only attribute means nothing for folders on Windows: try.
        let probe = dir.join(format!(".heronite-write-test-{}", std::process::id()));
        let ok = std::fs::File::create(&probe).is_ok();
        let _ = std::fs::remove_file(&probe);
        ok
    }
}

/// What tells one file from another, whatever its names (hard links share
/// it): the device and inode on Unix, the volume and file index on Windows.
pub fn file_id(path: &Path, meta: &Metadata) -> Option<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = path;
        Some((meta.dev(), meta.ino()))
    }
    #[cfg(windows)]
    {
        let _ = meta;
        windows_file_id(path)
    }
}

#[cfg(windows)]
fn windows_file_id(path: &Path) -> Option<(u64, u64)> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS};
    // Backup semantics: folders open too.
    let file = std::fs::OpenOptions::new().access_mode(0).custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(path).ok()?;
    // SAFETY: the handle is open for the duration of the call, and `info` is ours to write.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return None;
    }
    Some((info.dwVolumeSerialNumber as u64, (info.nFileIndexHigh as u64) << 32 | info.nFileIndexLow as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_links_share_their_id() {
        let d = tempfile::tempdir().unwrap();
        let (a, b, c) = (d.path().join("a"), d.path().join("b"), d.path().join("c"));
        std::fs::write(&a, "x").unwrap();
        std::fs::write(&c, "x").unwrap();
        std::fs::hard_link(&a, &b).unwrap();
        let id = |p: &Path| file_id(p, &std::fs::metadata(p).unwrap()).unwrap();
        assert_eq!(id(&a), id(&b));
        assert_ne!(id(&a), id(&c));
    }

    #[test]
    fn a_folder_of_ours_is_writable() {
        let d = tempfile::tempdir().unwrap();
        assert!(writable_dir(d.path()));
        assert!(!writable_dir(&d.path().join("missing")));
    }
}
