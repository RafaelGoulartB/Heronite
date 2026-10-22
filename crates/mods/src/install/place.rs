//! Putting files into the mod folder being built. Every path component takes
//! the spelling of an entry that is already there, else of the game folder's
//! entry at the same place, so `data/textures` lands in `Data/Textures` when
//! the game has those, and two spellings of one folder become one.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use heronite_installer::engine::CANCELLED;

use crate::fsutil::{entries, find_ci, remove_any};

/// How a file gets from the staged tree to the mod folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Transfer {
    /// The staged tree is ours and each file is used once.
    Move,
    /// The staged tree is ours and a file may be used twice (FOMOD rules).
    Link,
    /// The staged tree is the user's own folder.
    Copy,
}

#[derive(Default)]
struct DirInfo {
    /// Lower-cased name to the spelling in the mod folder.
    mine: HashMap<String, String>,
    /// The game's folder at the same place.
    game: Option<PathBuf>,
    /// The game folder's entries, read when first needed.
    game_names: Option<HashMap<String, (String, bool)>>,
}

pub(super) struct Placer<'a> {
    root: PathBuf,
    dirs: HashMap<PathBuf, DirInfo>,
    mode: Transfer,
    cancel: &'a AtomicBool,
}

impl<'a> Placer<'a> {
    pub(super) fn new(root: &Path, game_dir: Option<&Path>, mode: Transfer, cancel: &'a AtomicBool) -> Self {
        let mut dirs = HashMap::new();
        dirs.insert(root.to_path_buf(), DirInfo { game: game_dir.map(Path::to_path_buf), ..Default::default() });
        Self { root: root.to_path_buf(), dirs, mode, cancel }
    }

    fn info(&mut self, dir: &Path) -> &mut DirInfo {
        let info = self.dirs.entry(dir.to_path_buf()).or_default();
        if info.mine.is_empty() {
            // Entries from an earlier visit of the folder (a second rule).
            for (name, _) in entries(dir) {
                info.mine.insert(name.to_lowercase(), name);
            }
        }
        info
    }

    /// The spelling `name` gets in `dir`: the one already there, else the
    /// game's, else its own. The name is remembered.
    fn spell(&mut self, dir: &Path, name: &str, is_dir: bool) -> String {
        let lower = name.to_lowercase();
        let info = self.info(dir);
        if let Some(existing) = info.mine.get(&lower) {
            return existing.clone();
        }
        let game_names = info.game_names.get_or_insert_with(|| match &info.game {
            Some(g) => entries(g).into_iter().map(|(n, d)| (n.to_lowercase(), (n, d))).collect(),
            None => HashMap::new(),
        });
        let spelled = match game_names.get(&lower) {
            Some((n, d)) if *d == is_dir => n.clone(),
            _ => name.to_string(),
        };
        info.mine.insert(lower, spelled.clone());
        spelled
    }

    /// The folder `dir/name` (created), spelled as above.
    fn subdir(&mut self, dir: &Path, name: &str) -> Result<PathBuf, String> {
        let spelled = self.spell(dir, name, true);
        let path = dir.join(&spelled);
        match path.symlink_metadata() {
            Ok(m) if m.is_dir() => {}
            Ok(_) => {
                remove_any(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                fs::create_dir(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            }
            Err(_) => fs::create_dir(&path).map_err(|e| format!("{}: {e}", path.display()))?,
        }
        if !self.dirs.contains_key(&path) {
            let game = {
                let info = self.info(dir);
                info.game_names
                    .as_ref()
                    .and_then(|g| g.get(&spelled.to_lowercase()))
                    .filter(|(_, d)| *d)
                    .and_then(|(n, _)| info.game.as_ref().map(|g| g.join(n)))
            };
            self.dirs.insert(path.clone(), DirInfo { game, ..Default::default() });
        }
        Ok(path)
    }

    fn dir_at(&mut self, comps: &[String]) -> Result<PathBuf, String> {
        let mut cur = self.root.clone();
        for c in comps {
            cur = self.subdir(&cur, c)?;
        }
        Ok(cur)
    }

    /// Copies what `src` is (a folder: its contents) to `dest`, given as
    /// components under the mod folder. A file needs its name as the last one.
    pub(super) fn place(&mut self, src: &Path, src_is_dir: bool, dest: &[String]) -> Result<(), String> {
        if src_is_dir {
            let dir = self.dir_at(dest)?;
            return self.copy_dir(src, &dir);
        }
        let Some((name, parents)) = dest.split_last() else { return Ok(()) };
        let dir = self.dir_at(parents)?;
        self.put_file(src, &dir, name)
    }

    fn copy_dir(&mut self, src: &Path, dst: &Path) -> Result<(), String> {
        for (name, is_dir) in entries(src) {
            if self.cancel.load(Ordering::Relaxed) {
                return Err(CANCELLED.to_string());
            }
            if is_dir {
                let sub = self.subdir(dst, &name)?;
                self.copy_dir(&src.join(&name), &sub)?;
            } else {
                self.put_file(&src.join(&name), dst, &name)?;
            }
        }
        Ok(())
    }

    fn put_file(&mut self, src: &Path, dir: &Path, name: &str) -> Result<(), String> {
        let spelled = self.spell(dir, name, false);
        let target = dir.join(spelled);
        let err = |e: std::io::Error| format!("{}: {e}", target.display());
        // Later rules win; a folder in the way goes.
        if target.symlink_metadata().is_ok() {
            remove_any(&target).map_err(err)?;
        }
        transfer(src, &target, self.mode).map_err(err)?;
        make_writable(&target);
        Ok(())
    }
}

fn transfer(src: &Path, dst: &Path, mode: Transfer) -> std::io::Result<()> {
    match mode {
        Transfer::Move => fs::rename(src, dst).or_else(|_| fs::copy(src, dst).map(|_| ()).and_then(|_| fs::remove_file(src))),
        Transfer::Link => fs::hard_link(src, dst).or_else(|_| fs::copy(src, dst).map(|_| ())),
        Transfer::Copy => fs::copy(src, dst).map(|_| ()),
    }
}

/// Archives carry read-only files; the owner can always change a mod's files.
fn make_writable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(m) = path.metadata() {
        let mode = m.permissions().mode();
        if mode & 0o200 == 0 {
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode | 0o200));
        }
    }
}

/// `a/b\c` as components; `..` is refused, `.` and empty parts are dropped.
pub(super) fn components(path: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for c in path.split(['/', '\\']) {
        match c {
            "" | "." => {}
            ".." => return Err(heronite_i18n::tr!("The path {path} is not allowed", path = path)),
            c => out.push(c.to_string()),
        }
    }
    Ok(out)
}

/// The staged file or folder at `rel` (case-insensitive, `\` or `/`): its
/// path and whether it is a folder. `None` when it is not there.
pub(super) fn resolve(root: &Path, rel: &str) -> Result<Option<(PathBuf, bool)>, String> {
    let mut cur = root.to_path_buf();
    let mut is_dir = true;
    for c in components(rel)? {
        if !is_dir {
            return Ok(None);
        }
        let Some((name, d)) = find_ci(&cur, &c) else { return Ok(None) };
        cur = cur.join(name);
        is_dir = d;
    }
    Ok(Some((cur, is_dir)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(path: PathBuf, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn comps(s: &str) -> Vec<String> {
        components(s).unwrap()
    }

    #[test]
    fn components_refuse_parent_folders() {
        assert_eq!(comps("a\\b/./c//d"), ["a", "b", "c", "d"]);
        assert!(components("a/../b").is_err());
        assert!(components("..\\x").is_err());
        assert!(comps("").is_empty());
    }

    #[test]
    fn resolves_ignoring_case_and_separators() {
        let t = tempfile::tempdir().unwrap();
        put(t.path().join("Optional/Textures/a.dds"), "x");
        assert_eq!(resolve(t.path(), "optional\\textures").unwrap(), Some((t.path().join("Optional/Textures"), true)));
        assert_eq!(resolve(t.path(), "OPTIONAL/TEXTURES/A.DDS").unwrap(), Some((t.path().join("Optional/Textures/a.dds"), false)));
        assert_eq!(resolve(t.path(), "optional/missing").unwrap(), None);
        assert_eq!(resolve(t.path(), "optional/textures/a.dds/x").unwrap(), None);
        assert_eq!(resolve(t.path(), "").unwrap(), Some((t.path().to_path_buf(), true)));
        assert!(resolve(t.path(), "../etc").is_err());
    }

    #[test]
    fn takes_the_games_spelling_and_merges_spellings() {
        let t = tempfile::tempdir().unwrap();
        let (game, src, out) = (t.path().join("game"), t.path().join("src"), t.path().join("out"));
        put(game.join("Data/Textures/old.dds"), "g");
        put(game.join("Data/Skyrim.esm"), "g");
        put(src.join("1/a.dds"), "first");
        put(src.join("2/A.DDS"), "second");
        put(src.join("2/b.dds"), "second");
        put(src.join("p.esp"), "p");
        fs::create_dir_all(&out).unwrap();
        let cancel = AtomicBool::new(false);
        let mut p = Placer::new(&out, Some(&game), Transfer::Copy, &cancel);
        p.place(&src.join("1"), true, &comps("data/textures")).unwrap();
        p.place(&src.join("2"), true, &comps("DATA\\TEXTURES")).unwrap();
        p.place(&src.join("p.esp"), false, &comps("DATA/P.esp")).unwrap();
        let names: Vec<String> = {
            let mut v = Vec::new();
            crate::fsutil::walk_files(&out, &mut |rel, _| v.push(rel.to_string()));
            v.sort();
            v
        };
        assert_eq!(names, ["Data/P.esp", "Data/Textures/a.dds", "Data/Textures/b.dds"]);
        assert_eq!(fs::read_to_string(out.join("Data/Textures/a.dds")).unwrap(), "second", "the later rule wins");
        assert!(!game.join("Data/P.esp").exists(), "the game folder is only read");
    }

    #[test]
    fn a_folder_of_another_spelling_is_not_taken_from_a_game_file() {
        let t = tempfile::tempdir().unwrap();
        let (game, src, out) = (t.path().join("game"), t.path().join("src"), t.path().join("out"));
        put(game.join("meshes"), "a file in the game");
        put(src.join("f.nif"), "x");
        fs::create_dir_all(&out).unwrap();
        let cancel = AtomicBool::new(false);
        Placer::new(&out, Some(&game), Transfer::Copy, &cancel).place(&src.join("f.nif"), false, &comps("Meshes/f.nif")).unwrap();
        assert!(out.join("Meshes/f.nif").exists());
    }

    #[test]
    fn moves_links_and_never_follows_links() {
        let t = tempfile::tempdir().unwrap();
        let (src, out) = (t.path().join("src"), t.path().join("out"));
        put(src.join("d/a.txt"), "a");
        std::os::unix::fs::symlink("/etc", src.join("d/evil")).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", src.join("d/pw")).unwrap();
        fs::create_dir_all(&out).unwrap();
        let cancel = AtomicBool::new(false);
        Placer::new(&out, None, Transfer::Move, &cancel).place(&src.join("d"), true, &comps("x")).unwrap();
        assert!(out.join("x/a.txt").exists() && !src.join("d/a.txt").exists());
        assert!(!out.join("x/evil").exists() && !out.join("x/pw").exists());
        put(src.join("l.txt"), "l");
        let mut p = Placer::new(&out, None, Transfer::Link, &cancel);
        p.place(&src.join("l.txt"), false, &comps("one/l.txt")).unwrap();
        p.place(&src.join("l.txt"), false, &comps("two/l.txt")).unwrap();
        assert!(out.join("one/l.txt").exists() && out.join("two/l.txt").exists() && src.join("l.txt").exists());
    }

    #[test]
    fn stops_when_cancelled() {
        let t = tempfile::tempdir().unwrap();
        let (src, out) = (t.path().join("src"), t.path().join("out"));
        put(src.join("a.txt"), "a");
        fs::create_dir_all(&out).unwrap();
        let cancel = AtomicBool::new(true);
        let r = Placer::new(&out, None, Transfer::Copy, &cancel).place(&src, true, &[]);
        assert_eq!(r, Err(CANCELLED.to_string()));
    }
}
