//! A file of the Wine prefix (`Plugins.txt`, a game's `.ini`) that has other
//! content while the game runs; the user's own comes back when the guard is
//! dropped, or at the next run if a crash got in the way.

use std::fs;
use std::path::{Path, PathBuf};

use heronite_data::temps::write_atomic;
use heronite_i18n::tr;

use crate::fsutil::remove_any;

const BACKUP_SUFFIX: &str = ".heronite-backup";
/// Left when there was no file before ours, so a crash can be undone.
const NEW_SUFFIX: &str = ".heronite-new";

/// Puts the user's file back when dropped.
#[derive(Debug)]
pub(crate) struct Swap {
    file: PathBuf,
    backup: PathBuf,
    marker: PathBuf,
    had_original: bool,
    armed: bool,
}

fn sibling(file: &Path, suffix: &str) -> PathBuf {
    let mut name = file.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    file.with_file_name(name)
}

fn write_error(path: &Path, e: std::io::Error) -> String {
    tr!("Could not write {path}: {error}", path = path.display(), error = e)
}

impl Swap {
    /// Undoes what a crash left behind: the user's file comes back before
    /// anything else is read.
    pub(crate) fn recover(file: &Path) -> Result<(), String> {
        let (backup, marker) = (sibling(file, BACKUP_SUFFIX), sibling(file, NEW_SUFFIX));
        if backup.exists() {
            log::warn!("mods: restoring the {} a crash left behind", file.display());
            fs::rename(&backup, file).map_err(|e| write_error(file, e))?;
        } else if marker.exists() {
            let _ = remove_any(file);
        }
        let _ = remove_any(&marker);
        Ok(())
    }

    /// `file` now holds `content`. `original` is what [`Swap::recover`] left
    /// there (`None`: no file).
    pub(crate) fn replace(file: &Path, original: Option<&[u8]>, content: &[u8]) -> Result<Swap, String> {
        let mut swap = Swap {
            file: file.to_path_buf(),
            backup: sibling(file, BACKUP_SUFFIX),
            marker: sibling(file, NEW_SUFFIX),
            had_original: original.is_some(),
            armed: false,
        };
        match original {
            Some(_) => fs::rename(file, &swap.backup).map_err(|e| write_error(&swap.backup, e))?,
            None => fs::write(&swap.marker, b"Heronite wrote this file; this marks it as not the user's.\n")
                .map_err(|e| write_error(&swap.marker, e))?,
        }
        swap.armed = true;
        write_atomic(file, content, Some(0o644)).map_err(|e| write_error(file, e))?;
        Ok(swap)
    }
}

impl Drop for Swap {
    fn drop(&mut self) {
        if !self.armed {
            let _ = remove_any(&self.marker);
            return;
        }
        let result = if self.had_original {
            fs::rename(&self.backup, &self.file)
        } else {
            remove_any(&self.file).and_then(|_| remove_any(&self.marker))
        };
        match result {
            Ok(()) => log::info!("mods: {} restored", self.file.display()),
            Err(e) => log::warn!("mods: could not restore {}: {e}", self.file.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_users_file_comes_back_and_a_crash_is_undone() {
        let t = tempfile::tempdir().unwrap();
        let file = t.path().join("a.ini");
        fs::write(&file, "mine").unwrap();
        let swap = Swap::replace(&file, Some(b"mine"), b"ours").unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"ours");
        drop(swap);
        assert_eq!(fs::read(&file).unwrap(), b"mine");

        std::mem::forget(Swap::replace(&file, Some(b"mine"), b"ours").unwrap());
        Swap::recover(&file).unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"mine");

        fs::remove_file(&file).unwrap();
        std::mem::forget(Swap::replace(&file, None, b"ours").unwrap());
        Swap::recover(&file).unwrap();
        assert!(!file.exists() && !sibling(&file, NEW_SUFFIX).exists());
    }
}
