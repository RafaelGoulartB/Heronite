use std::path::{Path, PathBuf};

/// Why a screenshot could not be trashed.
#[derive(Debug)]
pub enum TrashError {
    /// The file does not exist.
    NotFound,
    /// The path is outside the captures root.
    OutsideRoot,
    Trash(String),
}

impl std::fmt::Display for TrashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("Screenshot not found."),
            Self::OutsideRoot => f.write_str("Invalid screenshot path."),
            Self::Trash(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for TrashError {}

/// Moves a screenshot to the freedesktop trash (what `shell.trashItem` does)
/// and returns the real path that was trashed.
///
/// With `root`, the file must resolve (symlinks followed) to somewhere inside
/// the captures folder, as `deleteCollectionScreenshot` checks; without it
/// the path is trashed as given. Afterwards, call
/// [`ThumbnailCache::maintain`](crate::ThumbnailCache::maintain) to drop the
/// preview of the vanished original.
pub fn trash_screenshot_in(root: Option<&Path>, path: &Path) -> Result<PathBuf, TrashError> {
    let real = std::fs::canonicalize(path).map_err(|_| TrashError::NotFound)?;
    if let Some(root) = root {
        let real_root = std::fs::canonicalize(root).map_err(|_| TrashError::NotFound)?;
        if real == real_root || !real.starts_with(&real_root) {
            return Err(TrashError::OutsideRoot);
        }
    }
    trash::delete(&real).map_err(|e| TrashError::Trash(e.to_string()))?;
    Ok(real)
}

/// [`trash_screenshot_in`] without a root check.
pub fn trash_screenshot(path: &Path) -> Result<PathBuf, TrashError> {
    trash_screenshot_in(None, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_outside_root_and_missing() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let file = other.path().join("a.png");
        std::fs::write(&file, b"x").unwrap();
        assert!(matches!(trash_screenshot_in(Some(root.path()), &file), Err(TrashError::OutsideRoot)));
        assert!(file.exists(), "must not be touched");
        // `..` escape resolves outside as well.
        let sneaky = root.path().join("..").join(other.path().file_name().unwrap()).join("a.png");
        let _ = trash_screenshot_in(Some(root.path()), &sneaky);
        assert!(file.exists());
        assert!(matches!(trash_screenshot(&root.path().join("nope.png")), Err(TrashError::NotFound)));
        // The root itself is not a screenshot.
        assert!(matches!(trash_screenshot_in(Some(root.path()), root.path()), Err(TrashError::OutsideRoot)));
    }

    /// Really moves a temp file to a trash (`$XDG_DATA_HOME` is redirected to a
    /// temp dir first). Run with `cargo test -p heronite-media -- --ignored`.
    #[test]
    #[ignore = "touches a freedesktop trash"]
    fn moves_a_temp_file_to_the_trash() {
        let data = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_DATA_HOME", data.path());
        let root = tempfile::tempdir_in(data.path()).unwrap();
        let file = root.path().join("shot.png");
        std::fs::write(&file, b"png").unwrap();
        let real = trash_screenshot_in(Some(root.path()), &file).unwrap();
        assert!(!file.exists());
        assert_eq!(real, std::fs::canonicalize(root.path()).unwrap().join("shot.png"));
    }
}
