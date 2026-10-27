//! Windows paths of a Wine prefix (`C:\Program Files (x86)\...`) as the
//! Linux paths they are, the way Wine finds them: the drive letter through
//! `dosdevices/<letter>:`, each part matched without regard to case.

use std::path::{Component, Path, PathBuf};

/// The Linux path of `win` (`C:\Games\x` or `C:/Games/x`) in the prefix
/// whose registry and `dosdevices` are in `wine_prefix`. Parts that do not
/// exist are kept as written. `None` for a path without a drive letter.
pub fn to_unix(wine_prefix: &Path, win: &str) -> Option<PathBuf> {
    let win = win.trim().trim_matches('"');
    let b = win.as_bytes();
    if b.len() < 2 || !b[0].is_ascii_alphabetic() || b[1] != b':' {
        return None;
    }
    let letter = (b[0] as char).to_ascii_lowercase();
    let device = wine_prefix.join("dosdevices").join(format!("{letter}:"));
    let root = match std::fs::read_link(&device) {
        Ok(target) if target.is_absolute() => target,
        Ok(target) => wine_prefix.join("dosdevices").join(target),
        Err(_) if letter == 'c' => wine_prefix.join("drive_c"),
        Err(_) if letter == 'z' => PathBuf::from("/"),
        Err(_) => return None,
    };
    let mut out = root;
    for part in win[2..].split(['\\', '/']).filter(|p| !p.is_empty() && *p != ".") {
        out = if part == ".." { out.parent().map(Path::to_path_buf).unwrap_or(out) } else { child(&out, part) };
    }
    Some(normal(&out))
}

/// `rel` (parts separated by `/`) below `dir`, each part with the case the
/// folder has on disk. Parts that do not exist are kept as written.
pub(crate) fn join_ci(dir: &Path, rel: &str) -> PathBuf {
    rel.split('/').filter(|p| !p.is_empty()).fold(dir.to_path_buf(), |dir, part| child(&dir, part))
}

/// `dir/part`, with the existing entry's case when only the case differs.
fn child(dir: &Path, part: &str) -> PathBuf {
    let exact = dir.join(part);
    if exact.exists() {
        return exact;
    }
    let lower = part.to_lowercase();
    std::fs::read_dir(dir)
        .ok()
        .and_then(|rd| rd.flatten().map(|e| e.file_name()).find(|n| n.to_string_lossy().to_lowercase() == lower))
        .map_or(exact, |name| dir.join(name))
}

/// Drops `.` and folds `..` without touching the disk.
fn normal(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_the_drive_and_the_case_of_the_folders() {
        let dir = tempfile::tempdir().unwrap();
        let pfx = dir.path();
        std::fs::create_dir_all(pfx.join("drive_c/Program Files (x86)/Ubisoft/games/Far Cry 3")).unwrap();
        std::fs::create_dir_all(pfx.join("dosdevices")).unwrap();
        std::os::unix::fs::symlink("../drive_c", pfx.join("dosdevices/c:")).unwrap();
        std::os::unix::fs::symlink("/", pfx.join("dosdevices/z:")).unwrap();
        let got = to_unix(pfx, "C:\\Program Files (x86)\\ubisoft\\GAMES\\Far Cry 3\\").unwrap();
        assert_eq!(got.canonicalize().unwrap(), pfx.join("drive_c/Program Files (x86)/Ubisoft/games/Far Cry 3").canonicalize().unwrap());
        assert_eq!(to_unix(pfx, "C:/Program Files (x86)/New Game").unwrap().file_name().unwrap(), "New Game");
        assert_eq!(to_unix(pfx, "Z:\\home\\u\\Games").unwrap(), PathBuf::from("/home/u/Games"));
        assert_eq!(to_unix(pfx, "D:\\x"), None);
        assert_eq!(to_unix(pfx, "relative\\x"), None);
    }

    #[test]
    fn without_dosdevices_c_is_the_prefix_drive() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(to_unix(dir.path(), "c:\\Games\\x").unwrap(), dir.path().join("drive_c/Games/x"));
    }
}
