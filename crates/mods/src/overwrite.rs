//! The overwrite folder: what the game wrote into its own folder while the
//! mods were mounted (the overlay's upper layer).

use std::fs;
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::Path;

use heronite_i18n::tr;

use crate::fsutil::{remove_any, walk_files};
use crate::install::scan;
use crate::model::{ModEntry, ModList, ModSource};
use crate::paths::GameDirs;
use crate::store;

/// An entry that says "this file of a lower layer was deleted": a character
/// device 0/0, or a `.wh.<name>` file.
fn is_whiteout(name: &str, meta: &fs::Metadata) -> bool {
    (meta.file_type().is_char_device() && meta.rdev() == 0) || name.rsplit('/').next().is_some_and(|n| n.starts_with(".wh."))
}

/// Files and bytes the game wrote; deletions are not counted.
pub fn summary(dirs: &GameDirs) -> (u64, u64) {
    let (mut files, mut bytes) = (0, 0);
    walk_files(&dirs.overwrite(), &mut |rel, meta| {
        if !is_whiteout(rel, meta) {
            files += 1;
            bytes += meta.len();
        }
    });
    (files, bytes)
}

/// Up to `limit` of the written files, sorted, relative to the game folder.
pub fn list_files(dirs: &GameDirs, limit: usize) -> Vec<String> {
    let mut v = Vec::new();
    walk_files(&dirs.overwrite(), &mut |rel, meta| {
        if !is_whiteout(rel, meta) {
            v.push(rel.to_string());
        }
    });
    v.sort();
    v.truncate(limit);
    v
}

/// Empties the overwrite folder (deletions included), keeping the folder.
pub fn clear(dirs: &GameDirs) -> io::Result<()> {
    let dir = dirs.overwrite();
    for e in fs::read_dir(&dir).into_iter().flatten().flatten() {
        remove_any(&e.path())?;
    }
    Ok(())
}

/// Moves what the game wrote into a new mod called `name`, enabled, last in
/// the load order (so it wins what it overlaps). Deletions stay in the
/// overwrite folder. Returns the mod's id; the list is saved.
pub fn into_mod(dirs: &GameDirs, list: &mut ModList, name: &str) -> Result<String, String> {
    let id = store::unique_id(list, name);
    let partial = dirs.mods_dir().join(format!("{id}.partial"));
    remove_any(&partial).map_err(|e| format!("{}: {e}", partial.display()))?;
    fs::create_dir_all(&partial).map_err(|e| format!("{}: {e}", partial.display()))?;
    let result = move_files(&dirs.overwrite(), &partial, "").map_err(|e| e.to_string());
    let found = scan(&partial, Some("Data"), &[]);
    if let Err(e) = result.and_then(|_| if found.files == 0 { Err(tr!("The overwrite folder is empty").to_string()) } else { Ok(()) }) {
        // What was moved goes back, so nothing is lost.
        let _ = move_files(&partial, &dirs.overwrite(), "");
        let _ = remove_any(&partial);
        return Err(e);
    }
    let dest = dirs.mod_dir(&id);
    fs::rename(&partial, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    list.mods.push(ModEntry {
        id: id.clone(),
        name: name.trim().to_string(),
        enabled: true,
        source: ModSource::File,
        installed_at: store::now_rfc3339(),
        files: found.files,
        size: found.size,
        plugins: found.plugins,
        dll_overrides: found.dll_overrides,
        ..Default::default()
    });
    store::save(dirs, list).map_err(|e| format!("{}: {e}", dirs.list_file().display()))?;
    Ok(id)
}

/// Moves the files under `from` to `to`, leaving whiteouts and links.
fn move_files(from: &Path, to: &Path, rel: &str) -> io::Result<()> {
    for e in fs::read_dir(from)?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let meta = e.path().symlink_metadata()?;
        let path = format!("{rel}{name}");
        if meta.is_dir() {
            let sub = to.join(&name);
            fs::create_dir_all(&sub)?;
            move_files(&e.path(), &sub, &format!("{path}/"))?;
            // Gone when nothing stayed behind.
            let _ = fs::remove_dir(e.path());
            let _ = fs::remove_dir(&sub);
        } else if meta.is_file() && !is_whiteout(&name, &meta) {
            let target = to.join(&name);
            fs::rename(e.path(), &target).or_else(|_| fs::copy(e.path(), &target).map(|_| ()).and_then(|_| fs::remove_file(e.path())))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::ModsRoot;
    use std::process::Command;

    fn put(path: std::path::PathBuf, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn setup() -> (tempfile::TempDir, GameDirs) {
        let t = tempfile::tempdir().unwrap();
        let dirs = ModsRoot::at(t.path()).game("g");
        put(dirs.overwrite().join("Data/Config/game.ini"), "written");
        put(dirs.overwrite().join("save.txt"), "12345");
        put(dirs.overwrite().join("Data/.wh.removed.esp"), "");
        // A real overlay marks deletions with a 0/0 character device; mknod needs
        // root, so only the `.wh.` form is made here.
        (t, dirs)
    }

    #[test]
    fn whiteouts_are_not_counted() {
        let (t, dirs) = setup();
        assert_eq!(summary(&dirs), (2, 12));
        assert_eq!(list_files(&dirs, 10), ["Data/Config/game.ini", "save.txt"]);
        assert_eq!(list_files(&dirs, 1).len(), 1);
        let empty = ModsRoot::at(t.path()).game("none");
        assert_eq!(summary(&empty), (0, 0));
    }

    #[test]
    fn clearing_keeps_the_folder() {
        let (_t, dirs) = setup();
        std::os::unix::fs::symlink("/etc", dirs.overwrite().join("link")).unwrap();
        clear(&dirs).unwrap();
        assert!(dirs.overwrite().is_dir() && fs::read_dir(dirs.overwrite()).unwrap().next().is_none());
        assert!(Path::new("/etc").exists());
    }

    #[test]
    fn becomes_a_mod_at_the_end() {
        let (_t, dirs) = setup();
        put(dirs.overwrite().join("Data/Patch.esp"), "p");
        let mut list = ModList::new("g", "G");
        list.mods.push(ModEntry { id: "first".into(), enabled: true, ..Default::default() });
        let id = into_mod(&dirs, &mut list, "Game output").unwrap();
        assert_eq!(id, "game-output");
        assert_eq!(list.mods.last().unwrap().id, id);
        let m = list.get(&id).unwrap();
        assert!(m.enabled && m.files == 3 && m.plugins == ["Patch.esp"], "{m:?}");
        assert_eq!(fs::read_to_string(dirs.mod_dir(&id).join("Data/Config/game.ini")).unwrap(), "written");
        assert!(!dirs.mod_dir(&id).join("Data/.wh.removed.esp").exists());
        assert!(dirs.overwrite().join("Data/.wh.removed.esp").exists(), "deletions stay");
        assert!(!dirs.overwrite().join("save.txt").exists() && !dirs.overwrite().join("Data/Config").exists());
        assert_eq!(store::load(&dirs), list);
        assert_eq!(summary(&dirs), (0, 0));
        assert_eq!(into_mod(&dirs, &mut list, "Again").unwrap_err(), "The overwrite folder is empty");
        assert!(!dirs.mods_dir().join("again.partial").exists());
    }

    #[test]
    fn a_real_whiteout_device_is_recognised() {
        // `mknod` works for 0/0 character devices only as root; try, skip otherwise.
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("gone");
        let ok = Command::new("mknod").arg(&p).args(["c", "0", "0"]).status().map(|s| s.success()).unwrap_or(false);
        if ok {
            let meta = p.symlink_metadata().unwrap();
            assert!(is_whiteout("gone", &meta));
        }
    }
}
