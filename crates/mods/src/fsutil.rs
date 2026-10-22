//! Small file-system helpers shared by the install, conflict and overwrite
//! code. None of them follows a symbolic link.

use std::fs;
use std::io;
use std::path::Path;

/// Calls `f(relative path with '/', metadata)` for every entry of `root` that
/// is not a folder (files, links, devices), depth first. Links are reported,
/// never entered.
pub(crate) fn walk_files(root: &Path, f: &mut dyn FnMut(&str, &fs::Metadata)) {
    fn rec(dir: &Path, rel: &str, f: &mut dyn FnMut(&str, &fs::Metadata)) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let Ok(meta) = e.path().symlink_metadata() else { continue };
            let name = e.file_name().to_string_lossy().into_owned();
            let path = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            if meta.is_dir() {
                rec(&e.path(), &path, f);
            } else {
                f(&path, &meta);
            }
        }
    }
    rec(root, "", f);
}

/// Names equal ignoring case (Windows mods against Linux folders).
pub(crate) fn same_name(a: &str, b: &str) -> bool {
    a == b || a.to_lowercase() == b.to_lowercase()
}

/// The extension of a file name without the dot, lower case; `""` when there
/// is none.
pub(crate) fn extension(name: &str) -> String {
    name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default()
}

/// The entry of `dir` called `name` (exact spelling first, then ignoring
/// case) and whether it is a real folder. Links are not entries.
pub(crate) fn find_ci(dir: &Path, name: &str) -> Option<(String, bool)> {
    let mut found = None;
    for e in fs::read_dir(dir).ok()?.flatten() {
        let Ok(t) = e.file_type() else { continue };
        if t.is_symlink() {
            continue;
        }
        let n = e.file_name().to_string_lossy().into_owned();
        if n == name {
            return Some((n, t.is_dir()));
        }
        if found.is_none() && same_name(&n, name) {
            found = Some((n, t.is_dir()));
        }
    }
    found
}

/// Entries of `dir` as `(name, is_dir)`, links left out, sorted by name.
pub(crate) fn entries(dir: &Path) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let t = e.file_type().ok()?;
            (!t.is_symlink()).then(|| (e.file_name().to_string_lossy().into_owned(), t.is_dir()))
        })
        .collect();
    out.sort();
    out
}

/// Removes `path` whatever it is; a link is removed, not followed. Missing is
/// fine.
pub(crate) fn remove_any(path: &Path) -> io::Result<()> {
    match path.symlink_metadata() {
        Ok(m) if m.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Files and bytes under `root`.
pub(crate) fn totals(root: &Path) -> (u64, u64) {
    let (mut files, mut bytes) = (0u64, 0u64);
    walk_files(root, &mut |_, m| {
        files += 1;
        bytes += m.len();
    });
    (files, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_ignoring_case_and_skips_links() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir(t.path().join("Textures")).unwrap();
        fs::write(t.path().join("a.esp"), b"x").unwrap();
        std::os::unix::fs::symlink("/etc", t.path().join("evil")).unwrap();
        assert_eq!(find_ci(t.path(), "textures"), Some(("Textures".into(), true)));
        assert_eq!(find_ci(t.path(), "A.ESP"), Some(("a.esp".into(), false)));
        assert_eq!(find_ci(t.path(), "evil"), None);
        assert_eq!(entries(t.path()), vec![("Textures".to_string(), true), ("a.esp".to_string(), false)]);
        assert_eq!(totals(t.path()).0, 2, "the link counts as a file and is not entered");
    }
}
