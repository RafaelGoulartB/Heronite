//! Phase one of an install: the archive is unpacked into a scratch folder (a
//! folder is used where it is), so a FOMOD installer can be shown before
//! anything is copied.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use heronite_i18n::tr;
use heronite_installer::archive;
use heronite_installer::exec::Io;

use crate::fsutil::{entries, find_ci, remove_any};
use crate::paths::GameDirs;

/// What is being installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Archive(PathBuf),
    Folder(PathBuf),
}

/// The files of a [`Source`], ready to be read.
#[derive(Debug)]
pub struct Staged {
    /// Folder holding the files.
    pub root: PathBuf,
    /// The folder is a scratch copy: it is moved out of and removed on drop.
    /// A user's folder is never owned.
    pub owned: bool,
    /// The folder (`root`, or one or two single-folder wrappers down) that
    /// holds `fomod/ModuleConfig.xml`. FOMOD sources are relative to it.
    pub fomod: Option<PathBuf>,
}

impl Drop for Staged {
    fn drop(&mut self) {
        if self.owned {
            if let Err(e) = remove_any(&self.root) {
                log::warn!("mods: could not remove {}: {e}", self.root.display());
            }
        }
    }
}

/// Loose files that are mods by themselves ("Add from file" on a plugin).
const LOOSE: &[&str] = &["esp", "esm", "esl", "bsa", "ba2", "dll", "asi", "pak", "archive"];

static COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn stage(dirs: &GameDirs, source: &Source, cancel: &AtomicBool) -> Result<Staged, String> {
    match source {
        Source::Folder(path) => {
            if !path.is_dir() {
                return Err(tr!("Cannot use {path}: it is not a folder", path = path.display()));
            }
            Ok(Staged { fomod: find_fomod(path), root: path.clone(), owned: false })
        }
        Source::Archive(path) => {
            if !path.is_file() {
                return Err(tr!("{path} does not exist", path = path.display()));
            }
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dest = dirs.staging().join(format!("{}-{}-{n}", std::process::id(), crate::store::unix_now()));
            fs::create_dir_all(&dest).map_err(|e| format!("{}: {e}", dest.display()))?;
            // Owned from here on: a failed or cancelled unpack removes it.
            let mut staged = Staged { root: dest, owned: true, fomod: None };
            unpack(path, &staged.root, cancel)?;
            staged.fomod = find_fomod(&staged.root);
            Ok(staged)
        }
    }
}

fn unpack(path: &Path, dest: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if LOOSE.contains(&ext.as_str()) {
        let name = path.file_name().unwrap_or_default();
        return fs::copy(path, dest.join(name)).map(|_| ()).map_err(|e| format!("{}: {e}", path.display()));
    }
    let is_cancelled = || cancel.load(Ordering::Relaxed);
    let log = |line: &str| log::debug!("mods: extract: {line}");
    let io = Io { cancel: &is_cancelled, log: &log };
    archive::extract(path, dest, None, false, &io)
}

/// The folder holding `fomod/ModuleConfig.xml`, looking through up to two
/// folders that hold nothing else.
pub fn find_fomod(root: &Path) -> Option<PathBuf> {
    let mut cur = root.to_path_buf();
    for _ in 0..3 {
        if let Some((fomod, true)) = find_ci(&cur, "fomod") {
            if find_ci(&cur.join(fomod), "ModuleConfig.xml").is_some_and(|(_, dir)| !dir) {
                return Some(cur);
            }
        }
        match entries(&cur).as_slice() {
            [(name, true)] => cur = cur.join(name),
            _ => return None,
        }
    }
    None
}

/// Removes what an install that never finished left: scratch folders and
/// `*.partial` mod folders. Call it at start, before any install runs; a mod
/// being replaced (`<id>.replaced`) whose new folder never arrived is put
/// back.
pub fn sweep(dirs: &GameDirs) {
    let staging = dirs.staging();
    for (name, _) in entries(&staging) {
        let _ = remove_any(&staging.join(name));
    }
    let _ = fs::remove_dir(&staging);
    for (name, _) in entries(&dirs.mods_dir()) {
        let path = dirs.mods_dir().join(&name);
        if name.ends_with(".partial") {
            let _ = remove_any(&path);
        } else if let Some(id) = name.strip_suffix(".replaced") {
            let original = dirs.mods_dir().join(id);
            if original.symlink_metadata().is_ok() {
                let _ = remove_any(&path);
            } else {
                let _ = fs::rename(&path, &original);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::ModsRoot;

    fn put(path: PathBuf) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "x").unwrap();
    }

    #[test]
    fn finds_the_installer_through_wrappers() {
        let t = tempfile::tempdir().unwrap();
        put(t.path().join("a/FOMOD/moduleconfig.xml"));
        put(t.path().join("a/data/x.esp"));
        assert_eq!(find_fomod(t.path().join("a").as_path()), Some(t.path().join("a")));
        assert_eq!(find_fomod(t.path()), Some(t.path().join("a")), "one folder down");
        put(t.path().join("b/w/Mod 1.0/fomod/ModuleConfig.xml"));
        assert_eq!(find_fomod(&t.path().join("b")), Some(t.path().join("b/w/Mod 1.0")), "two folders down");
        put(t.path().join("c/w1/w2/w3/fomod/ModuleConfig.xml"));
        assert_eq!(find_fomod(&t.path().join("c")), None, "too deep");
        put(t.path().join("d/readme.txt"));
        put(t.path().join("d/w/fomod/ModuleConfig.xml"));
        assert_eq!(find_fomod(&t.path().join("d")), None, "not a lone wrapper");
        put(t.path().join("e/fomod/info.xml"));
        assert_eq!(find_fomod(&t.path().join("e")), None);
    }

    #[test]
    fn a_folder_is_used_in_place_and_never_owned() {
        let t = tempfile::tempdir().unwrap();
        let dirs = ModsRoot::at(t.path().join("m")).game("g");
        put(t.path().join("mod/fomod/ModuleConfig.xml"));
        let s = stage(&dirs, &Source::Folder(t.path().join("mod")), &AtomicBool::new(false)).unwrap();
        assert!(!s.owned && s.fomod.is_some());
        drop(s);
        assert!(t.path().join("mod/fomod").exists());
        assert!(stage(&dirs, &Source::Folder(t.path().join("nope")), &AtomicBool::new(false)).is_err());
    }

    #[test]
    fn a_plugin_file_is_staged_as_it_is() {
        let t = tempfile::tempdir().unwrap();
        let dirs = ModsRoot::at(t.path().join("m")).game("g");
        put(t.path().join("My Mod.ESP"));
        let s = stage(&dirs, &Source::Archive(t.path().join("My Mod.ESP")), &AtomicBool::new(false)).unwrap();
        assert!(s.owned && s.root.join("My Mod.ESP").is_file());
        let root = s.root.clone();
        drop(s);
        assert!(!root.exists(), "the scratch folder goes with the guard");
    }

    #[test]
    fn unpacks_a_zip_when_unzip_exists() {
        if heronite_installer::exec::which_any(&["unzip", "7z", "7za", "7zz", "bsdtar"]).is_none()
            || heronite_installer::exec::which_any(&["zip"]).is_none()
        {
            return;
        }
        let t = tempfile::tempdir().unwrap();
        let dirs = ModsRoot::at(t.path().join("m")).game("g");
        put(t.path().join("src/Data/a.esp"));
        let zip = t.path().join("mod.zip");
        let ok = std::process::Command::new("zip")
            .arg("-qr")
            .arg(&zip)
            .arg("Data")
            .current_dir(t.path().join("src"))
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let s = stage(&dirs, &Source::Archive(zip), &AtomicBool::new(false)).unwrap();
        assert!(s.root.join("Data/a.esp").is_file());
        let cancelled = stage(&dirs, &Source::Archive(t.path().join("mod.zip")), &AtomicBool::new(true));
        assert!(cancelled.is_err());
        drop(s);
        assert!(entries(&dirs.staging()).is_empty(), "failed and finished stages leave nothing");
    }

    #[test]
    fn sweeping_removes_leftovers_and_restores_a_replaced_mod() {
        let t = tempfile::tempdir().unwrap();
        let dirs = ModsRoot::at(t.path().join("m")).game("g");
        put(dirs.staging().join("123-1/x.txt"));
        put(dirs.mods_dir().join("half.partial/x.txt"));
        put(dirs.mods_dir().join("lost.replaced/x.txt"));
        put(dirs.mods_dir().join("kept/x.txt"));
        put(dirs.mods_dir().join("kept.replaced/y.txt"));
        sweep(&dirs);
        assert!(!dirs.staging().exists() && !dirs.mods_dir().join("half.partial").exists());
        assert!(dirs.mods_dir().join("lost/x.txt").exists() && !dirs.mods_dir().join("lost.replaced").exists());
        assert!(dirs.mods_dir().join("kept/x.txt").exists() && !dirs.mods_dir().join("kept.replaced").exists());
    }
}
