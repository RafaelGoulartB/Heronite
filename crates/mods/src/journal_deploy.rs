//! Windows has no overlay file system, and hooking the game's file calls
//! (MO2's usvfs) is not something Heronite does. So while a game runs, the
//! enabled mods' files are **put into its folder**, and they are taken out
//! again when it ends (`docs/MODS.md`, "Windows").
//!
//! * Each file is a hard link to the mod's copy when the mods and the game
//!   are on the same volume, and a copy otherwise. Files a game tends to
//!   rewrite (`.ini`, `.cfg`, `.json`, small files) are always copied, so the
//!   game never writes into a mod through a link.
//! * A game file that a mod replaces is moved aside to `<game>/.backup/`.
//! * `deploy.json` lists every change **before** the game folder is
//!   touched, so [`restore`] can undo a deployment that a crash, a kill or a
//!   power cut cut short, at any step. It runs when the guard drops, when an
//!   adopted game ends, at the app's start, and before the next deployment.
//! * What the game wrote goes to the overwrite folder, the same as Linux's
//!   upper layer: files it created (anything not in `deploy.snapshot`, the
//!   folder's list from before), and the copies of ours it rewrote. The
//!   overwrite folder is the top layer of the next deployment.
//!
//! The module builds on every system, so its tests run on Linux too; only
//! Windows uses it at launch.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use heronite_data::temps::write_atomic;
use heronite_i18n::tr;
use serde::{Deserialize, Serialize};

use crate::fsutil::{extension, remove_any};
use crate::overwrite::is_whiteout;
use crate::paths::GameDirs;

/// Below this size a copy costs nothing, and the game can never write into
/// the mod's own file through a link.
const COPY_BELOW: u64 = 64 * 1024;
/// Text files a game rewrites as it runs; always copied.
const COPY_EXTENSIONS: [&str; 9] = ["ini", "cfg", "json", "xml", "txt", "toml", "yaml", "yml", "log"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Phase {
    /// The entries are planned and being put in place. Their sizes and
    /// times are not known yet, and the game has not run.
    Deploying,
    /// Everything is in place. From here on the game may have written.
    Deployed,
}

/// One file of a mod in the game folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    /// Relative to the game folder, with `/`.
    rel: String,
    /// The game had a file there; it is in `.backup/<rel>` while deployed.
    replaced: bool,
    /// A hard link, not a copy.
    #[serde(default)]
    linked: bool,
    /// The mod's file (a link shares its data with it).
    #[serde(default)]
    source: PathBuf,
    /// Size and modification time (ns since 1970) once in place.
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    modified: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Journal {
    game_dir: PathBuf,
    phase: Phase,
    entries: Vec<Entry>,
}

/// What a deployment did, for the game's log.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeploySummary {
    pub files: usize,
    pub linked: usize,
    pub copied: usize,
    /// Game files that a mod replaced (moved aside until the game ends).
    pub replaced: usize,
}

/// What a restore did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RestoreSummary {
    pub removed: usize,
    pub restored: usize,
    /// Files the game wrote, moved to the overwrite folder.
    pub kept: usize,
}

/// The mods in a game folder. Dropping it takes them out again.
#[derive(Debug)]
pub struct Deployed {
    dirs: GameDirs,
    game_dir: PathBuf,
    summary: DeploySummary,
    done: bool,
}

impl Deployed {
    pub fn game_dir(&self) -> &Path {
        &self.game_dir
    }

    pub fn summary(&self) -> DeploySummary {
        self.summary
    }

    /// Takes the mods out now.
    pub fn restore(mut self) -> Result<RestoreSummary, String> {
        self.done = true;
        restore(&self.dirs)
    }
}

impl Drop for Deployed {
    fn drop(&mut self) {
        if !std::mem::replace(&mut self.done, true) {
            if let Err(e) = restore(&self.dirs) {
                // The journal stays: the next start or deployment tries again.
                log::warn!("mods: could not take the mods out of {}: {e}", self.game_dir.display());
            }
        }
    }
}

/// Whether mods of ours are in this game's folder now (a game runs with
/// them, or a crash left them there for the next start to take out).
pub fn is_deployed(dirs: &GameDirs) -> bool {
    dirs.deploy_journal().is_file()
}

/// The game folder a deployment that is still there was made into.
pub fn deployed_game_dir(dirs: &GameDirs) -> Option<PathBuf> {
    read_journal(dirs).map(|j| j.game_dir)
}

/// Puts `layers` (mod folders, lowest priority first) into `game_dir`, with
/// the overwrite folder on top. A deployment left by a crash is restored
/// first. Fails, leaving the folder as it was, when a file cannot be placed.
pub fn deploy(dirs: &GameDirs, game_dir: &Path, layers: &[PathBuf]) -> Result<Deployed, String> {
    if is_deployed(dirs) {
        log::warn!("mods: mods from an earlier run are still in the game folder; taking them out first");
        restore(dirs).map_err(|e| tr!("The mods of an earlier run could not be taken out of the game folder: {error}", error = e))?;
    }
    let game = game_dir.canonicalize().map_err(|_| tr!("Cannot use {path}: it is not a folder", path = game_dir.display()))?;
    if !game.is_dir() {
        return Err(tr!("Cannot use {path}: it is not a folder", path = game_dir.display()));
    }
    fs::create_dir_all(dirs.root()).map_err(|e| format!("{}: {e}", dirs.root().display()))?;
    check_places(&game, &dirs.root().canonicalize().map_err(|e| e.to_string())?)?;

    let snapshot = list_tree(&game);
    let mut all_layers = layers.to_vec();
    if dirs.overwrite().is_dir() {
        all_layers.push(dirs.overwrite());
    }
    let mut entries = plan(&all_layers, &snapshot.files);
    let summary_files = entries.len();
    write_snapshot(dirs, &snapshot)?;
    let mut journal = Journal { game_dir: game.clone(), phase: Phase::Deploying, entries: entries.clone() };
    write_journal(dirs, &journal)?;

    let mut links = true;
    let mut summary = DeploySummary { files: summary_files, ..Default::default() };
    for entry in &mut entries {
        if let Err(e) = place(dirs, &game, entry, &mut links) {
            let what = tr!("Could not put {file} into the game folder: {error}", file = entry.rel.clone(), error = e);
            if let Err(again) = restore(dirs) {
                log::warn!("mods: undoing a deployment that failed: {again}");
            }
            return Err(what);
        }
        if entry.linked {
            summary.linked += 1;
        } else {
            summary.copied += 1;
        }
        if entry.replaced {
            summary.replaced += 1;
        }
    }
    journal.entries = entries;
    journal.phase = Phase::Deployed;
    write_journal(dirs, &journal)?;
    log::info!(
        "mods: {} files put into {} ({} linked, {} copied, {} game files set aside)",
        summary.files,
        game.display(),
        summary.linked,
        summary.copied,
        summary.replaced
    );
    Ok(Deployed { dirs: dirs.clone(), game_dir: game, summary, done: false })
}

/// The winner of each path: the highest layer that has it. The path is
/// spelled like the game's file, if the game has one, else like the first
/// layer that brought it. Paths are matched ignoring case, the way Windows
/// does.
fn plan(layers: &[PathBuf], game_files: &BTreeSet<String>) -> Vec<Entry> {
    let game_spelling: HashMap<String, &String> = game_files.iter().map(|f| (f.to_lowercase(), f)).collect();
    let mut order: Vec<String> = Vec::new();
    let mut winners: HashMap<String, (String, PathBuf)> = HashMap::new();
    for layer in layers {
        crate::fsutil::walk_files(layer, &mut |rel, meta| {
            if !meta.is_file() || is_whiteout(rel, meta) {
                return;
            }
            let key = rel.to_lowercase();
            let source = layer.join(rel);
            match winners.get_mut(&key) {
                Some(w) => w.1 = source,
                None => {
                    let spelled = game_spelling.get(&key).map(|s| s.to_string()).unwrap_or_else(|| rel.to_string());
                    winners.insert(key.clone(), (spelled, source));
                    order.push(key);
                }
            }
        });
    }
    order.sort();
    order
        .into_iter()
        .filter_map(|key| {
            let (rel, source) = winners.remove(&key)?;
            Some(Entry { replaced: game_spelling.contains_key(&key), rel, linked: false, source, size: None, modified: None })
        })
        .collect()
}

/// Whether `rel` is copied whatever the volume.
fn always_copied(rel: &str, size: u64) -> bool {
    size < COPY_BELOW || COPY_EXTENSIONS.contains(&extension(rel).as_str())
}

fn place(dirs: &GameDirs, game: &Path, entry: &mut Entry, links: &mut bool) -> io::Result<()> {
    let target = game.join(&entry.rel);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    if entry.replaced {
        let backup = dirs.deploy_backup().join(&entry.rel);
        if let Some(parent) = backup.parent() {
            fs::create_dir_all(parent)?;
        }
        move_file(&target, &backup)?;
    }
    let size = entry.source.metadata()?.len();
    entry.linked = false;
    if *links && !always_copied(&entry.rel, size) {
        match fs::hard_link(&entry.source, &target) {
            Ok(()) => entry.linked = true,
            Err(e) => {
                // Another volume, FAT, a share: copies from now on.
                log::info!("mods: no hard links into {} ({e}); copying", game.display());
                *links = false;
            }
        }
    }
    if !entry.linked {
        fs::copy(&entry.source, &target)?;
    }
    let (size, modified) = stamp(&target).unwrap_or((size, 0));
    entry.size = Some(size);
    entry.modified = Some(modified);
    Ok(())
}

/// Size and modification time (ns since 1970).
fn stamp(path: &Path) -> Option<(u64, u64)> {
    let meta = path.symlink_metadata().ok()?;
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((meta.len(), u64::try_from(modified.as_nanos()).unwrap_or(u64::MAX)))
}

/// Rename, else (another volume) copy and delete.
fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to).or_else(|_| fs::copy(from, to).and_then(|_| fs::remove_file(from)))
}

/// Takes a deployment out of its game folder. It is safe to run after a
/// crash at any step of [`deploy`], and again after a restore that failed
/// halfway. Without a journal there is nothing to do.
pub fn restore(dirs: &GameDirs) -> Result<RestoreSummary, String> {
    let Some(journal) = read_journal(dirs) else {
        // A snapshot or backups without a journal: the journal is written
        // after them, so nothing was placed yet.
        let _ = fs::remove_file(dirs.deploy_snapshot());
        if dirs.deploy_backup().is_dir() && fs::read_dir(dirs.deploy_backup()).map(|mut d| d.next().is_none()).unwrap_or(false) {
            let _ = fs::remove_dir(dirs.deploy_backup());
        }
        return Ok(RestoreSummary::default());
    };
    let game = journal.game_dir.clone();
    if !game.is_dir() {
        return Err(tr!("The game folder {path} is not there (an unplugged drive?)", path = game.display()));
    }
    let snapshot = read_snapshot(dirs);
    let mut summary = RestoreSummary::default();
    let mut touched_dirs: BTreeSet<String> = BTreeSet::new();
    let mut moved: HashSet<String> = HashSet::new();

    if journal.phase == Phase::Deployed {
        let ours: HashSet<String> = journal.entries.iter().map(|e| e.rel.to_lowercase()).collect();
        // Files the game made while it ran.
        let now = list_tree(&game);
        let known: HashSet<String> = snapshot.as_ref().map(|s| s.files.iter().map(|f| f.to_lowercase()).collect()).unwrap_or_default();
        if snapshot.is_some() {
            for rel in &now.files {
                let key = rel.to_lowercase();
                if !known.contains(&key) && !ours.contains(&key) {
                    keep_written(dirs, &game, rel).map_err(|e| format!("{rel}: {e}"))?;
                    touched_dirs.insert(rel.clone());
                    summary.kept += 1;
                }
            }
        }
        // Files of ours the game wrote over. A replaced file whose backup is
        // gone was put back by an earlier restore that failed further on: what
        // is there is the game's own file, and it stays.
        for e in &journal.entries {
            if e.replaced && !dirs.deploy_backup().join(&e.rel).exists() {
                continue;
            }
            if rewritten(&game, e) {
                keep_written(dirs, &game, &e.rel).map_err(|err| format!("{}: {err}", e.rel))?;
                moved.insert(e.rel.clone());
                summary.kept += 1;
            }
        }
    }

    let mut failed: Vec<String> = Vec::new();
    for e in journal.entries.iter().rev() {
        let target = game.join(&e.rel);
        let backup = dirs.deploy_backup().join(&e.rel);
        touched_dirs.insert(e.rel.clone());
        if e.replaced && !backup.exists() {
            // Not reached before a crash: the game's own file never moved.
            continue;
        }
        // Only a file: a folder there is the game's (a mod file that could not be placed).
        if !moved.contains(&e.rel) && target.symlink_metadata().is_ok_and(|m| !m.is_dir()) {
            match fs::remove_file(&target) {
                Ok(()) => summary.removed += 1,
                Err(err) => {
                    failed.push(format!("{}: {err}", e.rel));
                    continue;
                }
            }
        }
        if e.replaced {
            match move_file(&backup, &target) {
                Ok(()) => summary.restored += 1,
                Err(err) => failed.push(format!("{}: {err}", e.rel)),
            }
        }
    }
    if !failed.is_empty() {
        // The journal stays, so the next try picks up from here.
        return Err(failed.into_iter().take(3).collect::<Vec<_>>().join("; "));
    }
    remove_new_dirs(&game, &touched_dirs, snapshot.as_ref());
    let _ = remove_any(&dirs.deploy_backup());
    let _ = fs::remove_file(dirs.deploy_snapshot());
    fs::remove_file(dirs.deploy_journal()).map_err(|e| format!("{}: {e}", dirs.deploy_journal().display()))?;
    log::info!(
        "mods: taken out of {}: {} removed, {} game files put back, {} written by the game kept in overwrite",
        game.display(),
        summary.removed,
        summary.restored,
        summary.kept
    );
    Ok(summary)
}

/// Whether the game wrote over a file of ours. A link whose mod file changed
/// in step was written in place: the change is already in the mod's file,
/// and moving it would only link the overwrite folder to the mod.
fn rewritten(game: &Path, e: &Entry) -> bool {
    let (Some(size), Some(modified)) = (e.size, e.modified) else { return false };
    let Some(now) = stamp(&game.join(&e.rel)) else { return false };
    if now == (size, modified) {
        return false;
    }
    !(e.linked && stamp(&e.source) == Some(now))
}

/// Moves a file the game wrote into the overwrite folder, replacing what was
/// there.
fn keep_written(dirs: &GameDirs, game: &Path, rel: &str) -> io::Result<()> {
    let to = dirs.overwrite().join(rel);
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    if to.symlink_metadata().is_ok() {
        remove_any(&to)?;
    }
    move_file(&game.join(rel), &to)
}

/// Removes the empty folders that were not in the game before: those the
/// mods' files needed, and those the game made for files now in overwrite.
fn remove_new_dirs(game: &Path, files: &BTreeSet<String>, snapshot: Option<&Tree>) {
    let Some(snapshot) = snapshot else { return };
    let before: HashSet<String> = snapshot.dirs.iter().map(|d| d.to_lowercase()).collect();
    let mut candidates: BTreeSet<String> = BTreeSet::new();
    for rel in files {
        let mut parent = Path::new(rel).parent();
        while let Some(p) = parent.filter(|p| !p.as_os_str().is_empty()) {
            candidates.insert(p.to_string_lossy().replace('\\', "/"));
            parent = p.parent();
        }
    }
    // Deepest first, so a parent is empty by the time it is reached.
    let mut ordered: Vec<String> = candidates.into_iter().filter(|d| !before.contains(&d.to_lowercase())).collect();
    ordered.sort_by_key(|d| std::cmp::Reverse(d.matches('/').count()));
    for d in ordered {
        let _ = fs::remove_dir(game.join(&d));
    }
}

/// Refuses folders the mods must never be put into.
fn check_places(game: &Path, mods_dir: &Path) -> Result<(), String> {
    let home = user_home().and_then(|h| h.canonicalize().ok());
    if game.parent().is_none() || home.as_deref() == Some(game) {
        return Err(tr!("Mods cannot be applied over {path}", path = game.display()));
    }
    if mods_dir.starts_with(game) || game.starts_with(mods_dir) {
        return Err(tr!("The mods folder and the game folder cannot be inside each other").to_string());
    }
    Ok(())
}

fn user_home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// The files and folders of a tree, relative, with `/`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Tree {
    files: BTreeSet<String>,
    dirs: BTreeSet<String>,
}

fn list_tree(root: &Path) -> Tree {
    fn rec(dir: &Path, rel: &str, out: &mut Tree) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let Ok(meta) = e.path().symlink_metadata() else { continue };
            let name = e.file_name().to_string_lossy().into_owned();
            let path = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            if meta.is_dir() {
                rec(&e.path(), &path, out);
                out.dirs.insert(path);
            } else {
                out.files.insert(path);
            }
        }
    }
    let mut out = Tree::default();
    rec(root, "", &mut out);
    out
}

/// One line each: `f <path>` or `d <path>`.
fn write_snapshot(dirs: &GameDirs, tree: &Tree) -> Result<(), String> {
    let mut text = String::new();
    for d in &tree.dirs {
        text.push_str("d ");
        text.push_str(d);
        text.push('\n');
    }
    for f in &tree.files {
        text.push_str("f ");
        text.push_str(f);
        text.push('\n');
    }
    let file = dirs.deploy_snapshot();
    write_atomic(&file, text.as_bytes(), None).map_err(|e| format!("{}: {e}", file.display()))
}

fn read_snapshot(dirs: &GameDirs) -> Option<Tree> {
    let text = fs::read_to_string(dirs.deploy_snapshot()).ok()?;
    let mut tree = Tree::default();
    for line in text.lines() {
        match line.split_at_checked(2) {
            Some(("f ", rel)) => {
                tree.files.insert(rel.to_string());
            }
            Some(("d ", rel)) => {
                tree.dirs.insert(rel.to_string());
            }
            _ => {}
        }
    }
    Some(tree)
}

fn read_journal(dirs: &GameDirs) -> Option<Journal> {
    let bytes = fs::read(dirs.deploy_journal()).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(j) => Some(j),
        Err(e) => {
            log::warn!("mods: {} cannot be read: {e}", dirs.deploy_journal().display());
            None
        }
    }
}

fn write_journal(dirs: &GameDirs, journal: &Journal) -> Result<(), String> {
    let file = dirs.deploy_journal();
    let bytes = serde_json::to_vec_pretty(journal).map_err(|e| e.to_string())?;
    write_atomic(&file, &bytes, None).map_err(|e| format!("{}: {e}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::ModsRoot;

    fn put(path: PathBuf, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn read(path: PathBuf) -> String {
        fs::read_to_string(path).unwrap()
    }

    /// A game, two mods and the mods root, all in one temp dir.
    struct Env {
        _t: tempfile::TempDir,
        dirs: GameDirs,
        game: PathBuf,
    }

    fn env() -> Env {
        let t = tempfile::tempdir().unwrap();
        let dirs = ModsRoot::at(t.path().join("mods")).game("g");
        let game = t.path().join("game");
        put(game.join("game.exe"), "exe");
        put(game.join("Data/base.txt"), "game");
        put(game.join("Data/keep.txt"), "game");
        put(dirs.mod_dir("a").join("Data/base.txt"), "A");
        put(dirs.mod_dir("a").join("Data/Textures/a.dds"), "A");
        put(dirs.mod_dir("b").join("data/BASE.txt"), "B");
        put(dirs.mod_dir("b").join("winhttp.dll"), "dll");
        let game = game.canonicalize().unwrap();
        Env { _t: t, dirs, game }
    }

    impl Env {
        fn layers(&self) -> Vec<PathBuf> {
            vec![self.dirs.mod_dir("a"), self.dirs.mod_dir("b")]
        }

        fn vanilla(&self) {
            assert_eq!(read(self.game.join("Data/base.txt")), "game");
            assert_eq!(read(self.game.join("Data/keep.txt")), "game");
            assert!(!self.game.join("winhttp.dll").exists());
            assert!(!self.game.join("Data/Textures").exists(), "a folder only the mods needed is gone");
            assert!(!is_deployed(&self.dirs) && !self.dirs.deploy_backup().exists() && !self.dirs.deploy_snapshot().exists());
        }
    }

    #[test]
    fn the_highest_layer_wins_ignoring_case_and_the_game_comes_back() {
        let e = env();
        let d = deploy(&e.dirs, &e.game, &e.layers()).unwrap();
        assert!(is_deployed(&e.dirs));
        assert_eq!(deployed_game_dir(&e.dirs).as_deref(), Some(e.game.as_path()));
        assert_eq!(read(e.game.join("Data/base.txt")), "B", "b is last in the load order");
        assert_eq!(read(e.game.join("Data/Textures/a.dds")), "A");
        assert_eq!(read(e.game.join("winhttp.dll")), "dll");
        assert_eq!(read(e.dirs.deploy_backup().join("Data/base.txt")), "game");
        let s = d.summary();
        assert_eq!((s.files, s.replaced, s.linked + s.copied), (3, 1, 3));
        let r = d.restore().unwrap();
        assert_eq!((r.removed, r.restored, r.kept), (3, 1, 0));
        e.vanilla();
        assert_eq!(read(e.dirs.mod_dir("b").join("data/BASE.txt")), "B", "the mods are untouched");
    }

    #[test]
    fn dropping_the_guard_restores() {
        let e = env();
        {
            let _d = deploy(&e.dirs, &e.game, &e.layers()).unwrap();
            assert_eq!(read(e.game.join("Data/base.txt")), "B");
        }
        e.vanilla();
    }

    #[test]
    fn what_the_game_writes_lands_in_overwrite_and_comes_back_next_time() {
        let e = env();
        let d = deploy(&e.dirs, &e.game, &e.layers()).unwrap();
        // The game makes a log and a folder, and rewrites a copied text file.
        put(e.game.join("Logs/run.log"), "log");
        put(e.game.join("settings.ini"), "mine");
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(e.game.join("Data/base.txt"), "rewritten by the game").unwrap();
        let r = d.restore().unwrap();
        assert_eq!(r.kept, 3);
        e.vanilla();
        assert!(!e.game.join("Logs").exists() && !e.game.join("settings.ini").exists());
        assert_eq!(read(e.dirs.overwrite().join("Logs/run.log")), "log");
        assert_eq!(read(e.dirs.overwrite().join("settings.ini")), "mine");
        assert_eq!(read(e.dirs.overwrite().join("Data/base.txt")), "rewritten by the game");
        assert_eq!(read(e.dirs.mod_dir("b").join("data/BASE.txt")), "B");

        // Next launch: the overwrite folder is the top layer.
        let d = deploy(&e.dirs, &e.game, &e.layers()).unwrap();
        assert_eq!(read(e.game.join("Data/base.txt")), "rewritten by the game");
        assert_eq!(read(e.game.join("Logs/run.log")), "log");
        drop(d);
        e.vanilla();
        assert!(!e.game.join("Logs").exists());
        assert_eq!(read(e.dirs.overwrite().join("Logs/run.log")), "log", "unchanged, still in overwrite");
    }

    #[test]
    fn a_crash_while_running_is_undone_at_the_next_start() {
        let e = env();
        std::mem::forget(deploy(&e.dirs, &e.game, &e.layers()).unwrap());
        put(e.game.join("save.dat"), "save");
        assert!(is_deployed(&e.dirs));
        restore(&e.dirs).unwrap();
        e.vanilla();
        assert_eq!(read(e.dirs.overwrite().join("save.dat")), "save");
        assert_eq!(restore(&e.dirs).unwrap(), RestoreSummary::default(), "nothing left to do");
    }

    #[test]
    fn a_crash_halfway_through_deploying_is_undone() {
        let e = env();
        // What a crash after the second of three files leaves: the full plan
        // in the journal, one game file moved aside, two files placed.
        let snapshot = list_tree(&e.game);
        write_snapshot(&e.dirs, &snapshot).unwrap();
        let mut entries = plan(&e.layers(), &snapshot.files);
        let journal = Journal { game_dir: e.game.clone(), phase: Phase::Deploying, entries: entries.clone() };
        write_journal(&e.dirs, &journal).unwrap();
        let mut links = true;
        for entry in entries.iter_mut().take(2) {
            place(&e.dirs, &e.game, entry, &mut links).unwrap();
        }
        assert_eq!(entries.iter().map(|x| x.rel.as_str()).collect::<Vec<_>>(), ["Data/base.txt", "Data/Textures/a.dds", "winhttp.dll"]);
        assert_eq!(read(e.game.join("Data/base.txt")), "B");
        // A file the game "made" meanwhile is not taken: the game never ran.
        put(e.game.join("unrelated.txt"), "x");

        // The next deployment restores first.
        let d = deploy(&e.dirs, &e.game, &e.layers()).unwrap();
        assert_eq!(read(e.game.join("Data/base.txt")), "B");
        drop(d);
        e.vanilla();
        assert_eq!(read(e.game.join("unrelated.txt")), "x");
        assert!(!e.dirs.overwrite().join("unrelated.txt").exists());
    }

    #[test]
    fn a_crash_before_a_replaced_file_was_reached_leaves_it_alone() {
        let e = env();
        let snapshot = list_tree(&e.game);
        write_snapshot(&e.dirs, &snapshot).unwrap();
        let entries = plan(&e.layers(), &snapshot.files);
        write_journal(&e.dirs, &Journal { game_dir: e.game.clone(), phase: Phase::Deploying, entries }).unwrap();
        restore(&e.dirs).unwrap();
        e.vanilla();
    }

    #[test]
    fn a_failure_undoes_what_was_placed() {
        let e = env();
        // A mod file where the game has a folder cannot be placed.
        put(e.dirs.mod_dir("b").join("Data/Sub"), "a file named like a folder");
        put(e.game.join("Data/Sub/inner.txt"), "game");
        let err = deploy(&e.dirs, &e.game, &e.layers()).unwrap_err();
        assert!(err.to_lowercase().contains("data/sub"), "{err}");
        e.vanilla();
        assert_eq!(read(e.game.join("Data/Sub/inner.txt")), "game");
    }

    #[test]
    fn links_where_it_can_and_copies_text_files() {
        let e = env();
        let big = "x".repeat(COPY_BELOW as usize + 1);
        put(e.dirs.mod_dir("a").join("Data/big.bsa"), &big);
        put(e.dirs.mod_dir("a").join("Data/big.ini"), &big);
        let d = deploy(&e.dirs, &e.game, &e.layers()).unwrap();
        let j = read_journal(&e.dirs).unwrap();
        let linked = |rel: &str| j.entries.iter().find(|x| x.rel == rel).unwrap().linked;
        // Same volume (one temp dir): the big archive is a link.
        assert!(linked("Data/big.bsa"));
        assert!(!linked("Data/big.ini"), "text files are copied");
        assert!(!linked("winhttp.dll"), "small files are copied");
        assert_eq!(d.summary().linked, 1);
        drop(d);
        assert!(!e.game.join("Data/big.bsa").exists());
        assert_eq!(read(e.dirs.mod_dir("a").join("Data/big.bsa")), big);
    }

    #[test]
    fn whiteouts_brought_from_linux_are_not_files() {
        let e = env();
        put(e.dirs.overwrite().join("Data/.wh.keep.txt"), "");
        let d = deploy(&e.dirs, &e.game, &e.layers()).unwrap();
        assert!(!e.game.join("Data/.wh.keep.txt").exists());
        drop(d);
        e.vanilla();
    }

    /// On Windows a file the game still holds open cannot be removed.
    #[cfg(unix)]
    #[test]
    fn a_restore_that_failed_halfway_is_finished_without_taking_the_games_files() {
        use std::os::unix::fs::PermissionsExt;
        let e = env();
        put(e.game.join("a.txt"), "game");
        put(e.game.join("sub/b.bin"), "game");
        put(e.dirs.mod_dir("m").join("a.txt"), "mod");
        put(e.dirs.mod_dir("m").join("sub/c.bin"), "mod");
        std::mem::forget(deploy(&e.dirs, &e.game, &[e.dirs.mod_dir("m")]).unwrap());
        let locked = |mode| fs::set_permissions(e.game.join("sub"), fs::Permissions::from_mode(mode)).unwrap();
        locked(0o555);
        assert!(restore(&e.dirs).is_err(), "sub/c.bin cannot be removed");
        assert_eq!(read(e.game.join("a.txt")), "game", "the other files are back already");
        assert!(is_deployed(&e.dirs));
        locked(0o755);
        let done = restore(&e.dirs).unwrap();
        assert_eq!((done.removed, done.kept), (1, 0));
        assert_eq!(read(e.game.join("a.txt")), "game", "the game's own file is not taken for one it wrote");
        assert!(!e.dirs.overwrite().join("a.txt").exists() && !e.game.join("sub/c.bin").exists());
        assert!(!is_deployed(&e.dirs));
    }

    #[test]
    fn the_game_folder_missing_keeps_the_journal() {
        let e = env();
        std::mem::forget(deploy(&e.dirs, &e.game, &e.layers()).unwrap());
        let journal = read_journal(&e.dirs).unwrap();
        write_journal(&e.dirs, &Journal { game_dir: e.game.join("gone"), ..journal }).unwrap();
        assert!(restore(&e.dirs).is_err());
        assert!(is_deployed(&e.dirs));
    }

    #[test]
    fn dangerous_places_are_refused() {
        let e = env();
        let root = e.dirs.root().parent().unwrap().to_path_buf();
        fs::create_dir_all(e.dirs.root()).unwrap();
        let mods = e.dirs.root().canonicalize().unwrap();
        assert!(check_places(Path::new(if cfg!(windows) { r"C:\" } else { "/" }), &mods).is_err());
        assert!(check_places(&root.parent().unwrap().canonicalize().unwrap(), &mods).is_err(), "the mods are inside");
        assert!(check_places(&e.game, &mods).is_ok());
    }
}
