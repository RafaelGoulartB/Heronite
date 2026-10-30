//! Folder helpers: sizes, copying across file systems, and the guard that
//! keeps an uninstall from deleting anything but a game folder.

use std::fs;
use std::io;
use std::path::{Component, Path};

use heronite_data::proc::Cancel;

/// Bytes of the regular files below `path` (links are not followed).
pub fn dir_size(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else { return 0 };
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    fs::read_dir(path).map(|rd| rd.flatten().map(|e| dir_size(&e.path())).sum()).unwrap_or(0)
}

/// Whether `path` may be deleted as a game folder: absolute, no `..`, at
/// least two levels deep, and neither the home folder nor one of its parents.
pub fn safe_to_remove(path: &Path, home: Option<&Path>) -> bool {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return false;
    }
    let depth = path.components().filter(|c| matches!(c, Component::Normal(_))).count();
    if depth < 2 {
        return false;
    }
    !home.is_some_and(|h| h.starts_with(path))
}

/// Removes a folder, or just the link when `path` is a symlink.
pub fn remove_tree(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Copies `src` to `dst` (links are recreated, permissions kept), calling
/// `on_bytes` with the size of every copied file. `Ok(false)` when `cancel`
/// stopped it half way.
pub fn copy_tree(src: &Path, dst: &Path, cancel: &Cancel, on_bytes: &mut dyn FnMut(u64)) -> io::Result<bool> {
    let meta = fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        heronite_platform::fs::symlink(&fs::read_link(src)?, dst)?;
    } else if meta.is_dir() {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            if !copy_tree(&entry.path(), &dst.join(entry.file_name()), cancel, on_bytes)? {
                return Ok(false);
            }
        }
        fs::set_permissions(dst, meta.permissions())?;
    } else {
        if cancel.is_requested() {
            return Ok(false);
        }
        let n = fs::copy(src, dst)?;
        on_bytes(n);
    }
    Ok(true)
}

// Symlinks and Linux paths; GOG is off on Windows (`Feature::StoreAccounts`).
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn sizes_add_up_and_links_are_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("a/b")).unwrap();
        fs::write(dir.path().join("a/one"), [0u8; 10]).unwrap();
        fs::write(dir.path().join("a/b/two"), [0u8; 5]).unwrap();
        std::os::unix::fs::symlink(dir.path().join("a"), dir.path().join("a/loop")).unwrap();
        assert_eq!(dir_size(&dir.path().join("a")), 15);
        assert_eq!(dir_size(&dir.path().join("missing")), 0);
    }

    #[test]
    fn dangerous_paths_are_refused() {
        let home = Path::new("/home/me");
        for bad in ["", "/", "/home", "/home/me", "relative/game", "/games/../etc", "/games"] {
            assert!(!safe_to_remove(Path::new(bad), Some(home)), "{bad}");
        }
        assert!(safe_to_remove(Path::new("/games/The Game"), Some(home)));
        assert!(safe_to_remove(Path::new("/home/me/Games/g"), Some(home)));
    }

    #[test]
    fn copies_a_tree_and_stops_on_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("f"), [1u8; 7]).unwrap();
        fs::write(src.join("sub/g"), [2u8; 3]).unwrap();
        std::os::unix::fs::symlink("f", src.join("link")).unwrap();
        let mut copied = 0;
        let cancel = Cancel::new();
        assert!(copy_tree(&src, &dir.path().join("dst"), &cancel, &mut |n| copied += n).unwrap());
        assert_eq!(copied, 10);
        assert_eq!(fs::read(dir.path().join("dst/sub/g")).unwrap(), [2u8; 3]);
        assert_eq!(fs::read_link(dir.path().join("dst/link")).unwrap(), Path::new("f"));
        cancel.request();
        assert!(!copy_tree(&src, &dir.path().join("dst2"), &cancel, &mut |_| {}).unwrap());
    }
}
