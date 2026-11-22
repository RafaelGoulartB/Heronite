//! The overwrite folder: what the game wrote into its own folder while the
//! mods were mounted (the overlay's upper layer).

use std::fs;
use std::io;
#[cfg(unix)]
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::Path;
use std::time::SystemTime;

use heronite_i18n::tr;

use crate::fsutil::{remove_any, walk_files};
use crate::install::scan;
use crate::model::{ModEntry, ModList, ModSource};
use crate::paths::GameDirs;
use crate::store;
use crate::write_rule::WriteRule;

/// How many of the newest files [`summary`] names.
pub const NEWEST: usize = 30;

/// An entry that says "this file of a lower layer was deleted": a character
/// device 0/0, or a `.wh.<name>` file. Windows has no devices here: an
/// overwrite folder brought from Linux (a backup) keeps the `.wh.` form only.
pub(crate) fn is_whiteout(name: &str, meta: &fs::Metadata) -> bool {
    #[cfg(unix)]
    if meta.file_type().is_char_device() && meta.rdev() == 0 {
        return true;
    }
    #[cfg(not(unix))]
    let _ = meta;
    name.rsplit('/').next().is_some_and(|n| n.starts_with(".wh."))
}

/// What is in the overwrite folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Written {
    /// Files the game wrote.
    pub files: u64,
    pub bytes: u64,
    /// Files of the game or of a mod that the game deleted.
    pub deleted: u64,
    /// The written files by top-level folder, sorted by name; files at the
    /// root of the game folder are the group `""`.
    pub groups: Vec<Group>,
    /// The newest written files (up to [`NEWEST`]), newest first.
    pub newest: Vec<String>,
    /// Written files that lie outside the mods' folders: saves or settings
    /// of the game, kept here before [`crate::write_rule`].
    pub outside: u64,
}

/// The written files under one top-level folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Group {
    pub folder: String,
    pub files: u64,
    pub bytes: u64,
    /// How many of them lie outside the mods' folders.
    pub outside: u64,
}

impl Written {
    pub fn is_empty(&self) -> bool {
        self.files == 0 && self.deleted == 0
    }
}

/// Whether `name` marks a whole folder as replaced (`.wh..wh..opq`) rather
/// than a file as deleted.
fn is_marker(name: &str) -> bool {
    name.rsplit('/').next().is_some_and(|n| n.starts_with(".wh..wh."))
}

/// What the game wrote and deleted, by folder. Deleting is counted apart
/// from writing: a folder with only deletions still has something to show.
/// `rule` (over every installed mod, [`WriteRule::for_list`]) tells which
/// files lie outside the mods' folders.
pub fn summary(dirs: &GameDirs, rule: &WriteRule) -> Written {
    let mut out = Written::default();
    let mut groups: std::collections::BTreeMap<String, Group> = Default::default();
    let mut times: Vec<(SystemTime, String)> = Vec::new();
    walk_files(&dirs.overwrite(), &mut |rel, meta| {
        if is_marker(rel) || crate::journal_fs::is_partial(rel) {
            return;
        }
        if is_whiteout(rel, meta) {
            out.deleted += 1;
            return;
        }
        out.files += 1;
        out.bytes += meta.len();
        let folder = rel.split_once('/').map_or("", |(f, _)| f).to_string();
        let g = groups.entry(folder.to_lowercase()).or_insert_with(|| Group { folder, ..Default::default() });
        g.files += 1;
        g.bytes += meta.len();
        if !rule.to_overwrite(rel) {
            g.outside += 1;
            out.outside += 1;
        }
        times.push((meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), rel.to_string()));
    });
    out.groups = groups.into_values().collect();
    times.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    out.newest = times.into_iter().take(NEWEST).map(|(_, rel)| rel).collect();
    out
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

    fn counts(w: &Written) -> (u64, u64, u64) {
        (w.files, w.bytes, w.deleted)
    }

    #[test]
    fn deletions_are_counted_apart_from_files() {
        let (t, dirs) = setup();
        let rule = WriteRule { mod_folders: ["data".to_string()].into(), keep: Vec::new() };
        let w = summary(&dirs, &rule);
        assert_eq!(counts(&w), (2, 12, 1));
        assert_eq!(list_files(&dirs, 10), ["Data/Config/game.ini", "save.txt"]);
        assert_eq!(list_files(&dirs, 1).len(), 1);
        let empty = ModsRoot::at(t.path()).game("none");
        assert_eq!(summary(&empty, &rule), Written::default());
        assert!(summary(&empty, &rule).is_empty() && !w.is_empty());
    }

    #[test]
    fn files_are_grouped_by_folder_and_the_newest_come_first() {
        let (_t, dirs) = setup();
        put(dirs.overwrite().join("Saves/slot.sav"), "sav");
        put(dirs.overwrite().join("Data/cache.bin"), "c");
        put(dirs.overwrite().join("Data/Config/game.ini.heronite-partial"), "half");
        let old = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        fs::File::options().write(true).open(dirs.overwrite().join("save.txt")).unwrap().set_modified(old).unwrap();
        let rule = WriteRule { mod_folders: ["data".to_string()].into(), keep: Vec::new() };
        let w = summary(&dirs, &rule);
        let groups: Vec<_> = w.groups.iter().map(|g| (g.folder.as_str(), g.files, g.bytes, g.outside)).collect();
        assert_eq!(groups, [("", 1, 5, 1), ("Data", 2, 8, 0), ("Saves", 1, 3, 1)]);
        assert_eq!(w.outside, 2);
        assert_eq!(w.files, 4, "a partial copy is not a file of the game");
        assert_eq!(w.newest.last().map(String::as_str), Some("save.txt"), "the oldest last");
        assert_eq!(w.newest.len(), 4);
    }

    #[test]
    fn only_deletions_still_count() {
        let t = tempfile::tempdir().unwrap();
        let dirs = ModsRoot::at(t.path()).game("g");
        put(dirs.overwrite().join("Data/.wh.a.esp"), "");
        put(dirs.overwrite().join(".wh.b.txt"), "");
        put(dirs.overwrite().join("Data/.wh..wh..opq"), "");
        let w = summary(&dirs, &WriteRule::default());
        assert_eq!(counts(&w), (0, 0, 2));
        assert!(!w.is_empty());
    }

    #[test]
    fn clearing_keeps_the_folder() {
        let (_t, dirs) = setup();
        crate::fsutil::test_symlink(Path::new("/etc"), &dirs.overwrite().join("link"));
        clear(&dirs).unwrap();
        assert!(dirs.overwrite().is_dir() && fs::read_dir(dirs.overwrite()).unwrap().next().is_none());
        #[cfg(unix)]
        assert!(Path::new("/etc").exists(), "the link was removed, not followed");
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
        assert_eq!(summary(&dirs, &WriteRule::default()).files, 0);
        assert_eq!(into_mod(&dirs, &mut list, "Again").unwrap_err(), "The overwrite folder is empty");
        assert!(!dirs.mods_dir().join("again.partial").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_real_whiteout_device_is_recognised() {
        // `mknod` works for 0/0 character devices only as root; try, skip otherwise.
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("gone");
        let ok = std::process::Command::new("mknod").arg(&p).args(["c", "0", "0"]).status().map(|s| s.success()).unwrap_or(false);
        if ok {
            let meta = p.symlink_metadata().unwrap();
            assert!(is_whiteout("gone", &meta));
        }
    }
}
