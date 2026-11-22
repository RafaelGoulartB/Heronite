//! A game's mods kept on the game's own drive (`docs/MODS.md`, "On the
//! game's drive").
//!
//! On Windows the mods' files go into the game folder while it runs
//! ([`crate::journal_deploy`]): a hard link when the mods are on the game's
//! volume, a copy otherwise, at every launch. So a game whose folder is on
//! another volume than the mods root can keep its whole mods folder (list,
//! mods, downloads, overwrite, backups) in `<volume>/.heronite-mods/<game key>`
//! instead, and every launch links.
//!
//! The answer is given once per volume ([`Choice`], kept in
//! `<mods root>/drives.json`). The game's folder under the mods root stays,
//! holding only `elsewhere.json`, which names the folder in use;
//! [`crate::ModsRoot::game`] follows it, so the rest of the crate never knows.
//!
//! A move ([`relocate`]) writes `moving.json` (from, to) first, copies,
//! switches `elsewhere.json`, then deletes the old copy. A crash at any step
//! is put right by [`sweep`] at the next start: before the switch the new
//! copy is deleted, after it the old one. A launch is refused while a move
//! runs ([`moving`]).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use heronite_data::temps::write_atomic;
use heronite_i18n::tr;
use serde::{Deserialize, Serialize};

use crate::fsutil::remove_any;
use crate::model::Progress;
use crate::paths::{safe_name, ModsRoot};

/// The folder made at the root of a volume for the mods of its games.
pub const STORE_NAME: &str = ".heronite-mods";
/// In a game's folder of the mods root: where its mods are instead.
pub(crate) const POINTER: &str = "elsewhere.json";
/// In a game's folder of the mods root: a move under way.
const MOVING: &str = "moving.json";
/// In the mods root: the answer of each volume.
const CHOICES: &str = "drives.json";

/// `<volume>/.heronite-mods`.
pub fn store_of(volume: &Path) -> PathBuf {
    volume.join(STORE_NAME)
}

/// The volume `path` is on (see [`heronite_platform::disks::volume_root`]).
pub fn volume_of(path: &Path) -> Option<PathBuf> {
    heronite_platform::disks::volume_root(path)
}

fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.as_os_str().to_string_lossy().to_lowercase().trim_end_matches(['\\', '/'])
            == b.as_os_str().to_string_lossy().to_lowercase().trim_end_matches(['\\', '/'])
    } else {
        a == b
    }
}

// ---- The answer of each volume ----

/// What the user said for the games of a volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Choice {
    /// Their mods live in `<volume>/.heronite-mods`.
    Store,
    /// Their mods stay in the mods root and are copied at each launch.
    Copy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveChoice {
    pub volume: PathBuf,
    pub choice: Choice,
}

/// Every answer given, in the order given.
pub fn choices(root: &ModsRoot) -> Vec<DriveChoice> {
    fs::read(root.path().join(CHOICES)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn choice(root: &ModsRoot, volume: &Path) -> Option<Choice> {
    choices(root).into_iter().find(|c| same_path(&c.volume, volume)).map(|c| c.choice)
}

pub fn set_choice(root: &ModsRoot, volume: &Path, choice: Choice) -> io::Result<()> {
    let mut all = choices(root);
    all.retain(|c| !same_path(&c.volume, volume));
    all.push(DriveChoice { volume: volume.to_path_buf(), choice });
    fs::create_dir_all(root.path())?;
    let mut json = serde_json::to_vec_pretty(&all).map_err(io::Error::other)?;
    json.push(b'\n');
    write_atomic(&root.path().join(CHOICES), &json, Some(0o644))
}

// ---- Where a game's mods are, and where they should be ----

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pointer {
    path: PathBuf,
}

/// The folder `elsewhere.json` in `home` names, when it is one this module
/// could have made: `<something>/.heronite-mods/<name>`.
pub(crate) fn pointed(home: &Path) -> Option<PathBuf> {
    let bytes = fs::read(home.join(POINTER)).ok()?;
    let p: Pointer = serde_json::from_slice(&bytes).ok()?;
    is_store_folder(&p.path).then_some(p.path)
}

/// `<volume>/.heronite-mods/<name>`, absolute.
fn is_store_folder(path: &Path) -> bool {
    path.is_absolute()
        && path.file_name().is_some_and(|n| !n.is_empty() && n != "." && n != "..")
        && path.parent().and_then(Path::file_name).is_some_and(|n| n == STORE_NAME)
}

/// Where the mods should go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// The game's folder of the mods root.
    Home,
    /// A folder in this `.heronite-mods`.
    Store(PathBuf),
}

/// Where a game's mods are against where its folder is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    /// On the game's volume (or the game's folder is unknown): every launch
    /// links.
    Linked,
    /// On another volume, and that volume has no answer yet.
    Ask { volume: PathBuf, store: PathBuf },
    /// On another volume, which the user keeps them on: copied at each launch.
    Copied { volume: PathBuf },
    /// Elsewhere than the answer says (just accepted, or the game moved).
    Move(Destination),
}

/// Blocking: asks the volumes of the game's folder and of the mods.
pub fn placement(root: &ModsRoot, game_key: &str, game_dir: &Path) -> Placement {
    let Some(game_vol) = volume_of(game_dir) else { return Placement::Linked };
    let current = root.game(game_key).root().to_path_buf();
    let volumes =
        Volumes { current: volume_of(&current), home: volume_of(root.path()), at_home: same_path(&current, &root.home_of(game_key)) };
    let choice = choice(root, &game_vol);
    decide(game_vol, &volumes, choice)
}

struct Volumes {
    /// Of the folder the mods are in now.
    current: Option<PathBuf>,
    /// Of the mods root.
    home: Option<PathBuf>,
    /// Whether the mods are in the mods root now.
    at_home: bool,
}

fn decide(game_vol: PathBuf, v: &Volumes, choice: Option<Choice>) -> Placement {
    let on = |vol: &Option<PathBuf>| vol.as_deref().is_some_and(|v| same_path(v, &game_vol));
    if on(&v.current) {
        return Placement::Linked;
    }
    if on(&v.home) {
        // The game moved onto the mods root's volume: back home.
        return Placement::Move(Destination::Home);
    }
    match choice {
        Some(Choice::Store) => Placement::Move(Destination::Store(store_of(&game_vol))),
        Some(Choice::Copy) if v.at_home => Placement::Copied { volume: game_vol },
        // In the store of a volume the game left: the new one keeps to copies.
        Some(Choice::Copy) => Placement::Move(Destination::Home),
        None => Placement::Ask { store: store_of(&game_vol), volume: game_vol },
    }
}

/// Whether a move of the game's mods is under way (or a crash cut one short).
pub fn moving(root: &ModsRoot, game_key: &str) -> bool {
    root.home_of(game_key).join(MOVING).exists()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Move {
    from: PathBuf,
    to: PathBuf,
}

/// The first `<store>/<name>`, `<name>-2`, … that does not exist.
fn free_folder(store: &Path, name: &str) -> PathBuf {
    let first = store.join(name);
    if !first.exists() {
        return first;
    }
    (2..).map(|n| store.join(format!("{name}-{n}"))).find(|p| !p.exists()).unwrap_or(first)
}

/// Bytes of the files under `dir` (links not followed).
pub fn size_of(dir: &Path) -> u64 {
    let mut total = 0;
    crate::fsutil::walk_files(dir, &mut |_, meta| total += meta.len());
    total
}

/// Moves the game's mods folder (list, mods, downloads, overwrite) to `to`.
/// Refused while its mods are applied to `game_dir`. On an error or a
/// cancel the mods stay where they were, whole. Blocking; can take minutes.
pub fn relocate(
    root: &ModsRoot,
    game_key: &str,
    game_dir: Option<&Path>,
    to: &Destination,
    progress: &mut dyn FnMut(Progress),
    cancel: &AtomicBool,
) -> Result<(), String> {
    let in_use = || game_dir.is_some_and(|d| crate::deployment::in_use(root, d));
    if in_use() {
        return Err(tr!("The game is running with mods; close it first").to_string());
    }
    if moving(root, game_key) {
        sweep_game(&root.home_of(game_key));
    }
    let home = root.home_of(game_key);
    let from = root.game(game_key).root().to_path_buf();
    let target = match to {
        Destination::Home => home.clone(),
        Destination::Store(store) => free_folder(store, &safe_name(game_key)),
    };
    if same_path(&from, &target) || matches!(to, Destination::Store(s) if from.parent() == Some(s.as_path())) {
        return Ok(());
    }
    let total = size_of(&from);
    if let Some((_, free)) = heronite_platform::fs::disk_space(target.ancestors().find(|p| p.exists()).unwrap_or(&target)) {
        // Room for the mods and a little more.
        if free < total.saturating_add(total / 20 + (64 << 20)) {
            return Err(tr!(
                "{drive} has {free} MB free and the mods take {size} MB",
                drive = volume_of(&target).unwrap_or_else(|| target.clone()).display(),
                free = free >> 20,
                size = total >> 20
            ));
        }
    }
    let failed = |e: io::Error| tr!("Could not move the mods to {path}: {error}", path = target.display(), error = e);
    fs::create_dir_all(&home).map_err(failed)?;
    // A game with no mods yet: its first ones go straight to the target.
    fs::create_dir_all(&from).map_err(failed)?;
    write_json(&home.join(MOVING), &Move { from: from.clone(), to: target.clone() }).map_err(failed)?;

    let mut done = 0u64;
    let copied = copy_tree(
        &from,
        &target,
        &mut |n| {
            done += n;
            progress(Progress { done, total });
        },
        cancel,
    );
    let copied = copied.map_err(failed).and_then(|()| {
        // A launch that slipped in while copying holds the old folder.
        if in_use() {
            Err(tr!("The game is running with mods; close it first").to_string())
        } else {
            Ok(())
        }
    });
    if let Err(e) = copied {
        sweep_game(&home);
        return Err(e);
    }
    let switched = match to {
        Destination::Home => remove_any(&home.join(POINTER)),
        Destination::Store(_) => write_json(&home.join(POINTER), &Pointer { path: target.clone() }),
    };
    if let Err(e) = switched {
        sweep_game(&home);
        return Err(failed(e));
    }
    // The new copy is the one in use: the rest is cleaning up, also at the next start.
    sweep_game(&home);
    Ok(())
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let mut json = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    json.push(b'\n');
    write_atomic(path, &json, Some(0o644))
}

/// Copies `from` into `to` (made if missing), keeping each file's time. In
/// the game's folder of the mods root the move's own files are left out.
fn copy_tree(from: &Path, to: &Path, progress: &mut dyn FnMut(u64), cancel: &AtomicBool) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == POINTER || name == MOVING {
            continue;
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, tr!("Cancelled")));
        }
        let (src, dst) = (entry.path(), to.join(&name));
        let meta = src.symlink_metadata()?;
        if meta.is_dir() {
            copy_tree(&src, &dst, progress, cancel)?;
        } else if meta.is_file() {
            fs::copy(&src, &dst)?;
            if let Ok(time) = meta.modified() {
                fs::File::options().write(true).open(&dst)?.set_modified(time)?;
            }
            progress(meta.len());
        }
        // Links (Linux's `.layers`) are remade by the next mount.
    }
    Ok(())
}

/// Deletes `dir`: the game's folder of the mods root keeps `elsewhere.json`
/// and `moving.json` (its other entries go); a store folder goes whole, and
/// its `.heronite-mods` too when nothing else is left in it.
fn delete_copy(home: &Path, dir: &Path) {
    if same_path(dir, home) {
        for entry in fs::read_dir(home).into_iter().flatten().flatten() {
            let name = entry.file_name();
            if name != POINTER && name != MOVING {
                if let Err(e) = remove_any(&entry.path()) {
                    log::warn!("mods: could not delete {}: {e}", entry.path().display());
                }
            }
        }
    } else if is_store_folder(dir) {
        if let Err(e) = remove_any(dir) {
            log::warn!("mods: could not delete {}: {e}", dir.display());
        }
        if let Some(store) = dir.parent() {
            // Only when empty.
            let _ = fs::remove_dir(store);
        }
    }
}

/// Puts a move of one game right: finished when `elsewhere.json` already
/// names its target, undone otherwise. Then, when the mods are elsewhere,
/// whatever is left beside `elsewhere.json` goes.
fn sweep_game(home: &Path) {
    let current = pointed(home).unwrap_or_else(|| home.to_path_buf());
    if let Some(m) = fs::read(home.join(MOVING)).ok().and_then(|b| serde_json::from_slice::<Move>(&b).ok()) {
        let stale = if same_path(&current, &m.to) { &m.from } else { &m.to };
        if !same_path(stale, &current) {
            delete_copy(home, stale);
        }
    }
    let _ = fs::remove_file(home.join(MOVING));
    if !same_path(&current, home) {
        delete_copy(home, home);
    }
}

/// At start: finishes or undoes the moves a crash cut short.
pub fn sweep(root: &ModsRoot) {
    for entry in fs::read_dir(root.path()).into_iter().flatten().flatten() {
        let home = entry.path();
        if home.is_dir() && (home.join(MOVING).exists() || home.join(POINTER).exists()) {
            sweep_game(&home);
        }
    }
}

/// Deletes the game's mods kept on another drive, if any (the folder there,
/// and `.heronite-mods` once empty). The game's folder of the mods root is
/// the caller's.
pub fn forget(root: &ModsRoot, game_key: &str) -> io::Result<()> {
    let home = root.home_of(game_key);
    if let Some(dir) = pointed(&home) {
        remove_any(&dir)?;
        if let Some(store) = dir.parent() {
            let _ = fs::remove_dir(store);
        }
    }
    Ok(())
}

/// A game whose mods are on another drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    /// The game's folder name in the mods root (its key, made safe).
    pub key: String,
    pub dir: PathBuf,
}

/// Every game whose mods live in a `.heronite-mods` folder.
pub fn stored(root: &ModsRoot) -> Vec<Stored> {
    let mut out: Vec<Stored> = fs::read_dir(root.path())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let dir = pointed(&e.path())?;
            Some(Stored { key: e.file_name().to_string_lossy().into_owned(), dir })
        })
        .collect();
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ModEntry, ModList};
    use crate::store;

    fn put(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// A game `g` with one mod, a download and a file in overwrite.
    fn game_with_mods(root: &ModsRoot) {
        let dirs = root.game("g");
        let mut list = ModList::new("g", "Game");
        list.mods.push(ModEntry { id: "m".into(), name: "M".into(), enabled: true, ..Default::default() });
        store::save(&dirs, &list).unwrap();
        put(&dirs.mod_dir("m").join("Data/a.esp"), "plugin");
        put(&dirs.downloads().join("m.zip"), "zip");
        put(&dirs.overwrite().join("save.dat"), "save");
    }

    fn no_cancel() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn choices_are_kept_per_volume() {
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path().join("mods"));
        assert_eq!(choice(&root, Path::new("/d")), None);
        set_choice(&root, Path::new("/d"), Choice::Copy).unwrap();
        set_choice(&root, Path::new("/e"), Choice::Store).unwrap();
        set_choice(&root, Path::new("/d"), Choice::Store).unwrap();
        assert_eq!(choice(&root, Path::new("/d")), Some(Choice::Store));
        assert_eq!(choice(&root, Path::new("/e")), Some(Choice::Store));
        assert_eq!(choices(&root).len(), 2);
    }

    #[test]
    fn decides_by_volume_and_answer() {
        let (c, d, e) = (PathBuf::from("/c"), PathBuf::from("/d"), PathBuf::from("/e"));
        let home = |current: &Path| Volumes { current: Some(current.into()), home: Some(c.clone()), at_home: current == c };
        let stored_on = |vol: &Path| Volumes { current: Some(vol.into()), home: Some(c.clone()), at_home: false };
        // Mods root on C:, game on D:.
        assert_eq!(decide(d.clone(), &home(&c), None), Placement::Ask { volume: d.clone(), store: d.join(STORE_NAME) });
        assert_eq!(decide(d.clone(), &home(&c), Some(Choice::Copy)), Placement::Copied { volume: d.clone() });
        assert_eq!(decide(d.clone(), &home(&c), Some(Choice::Store)), Placement::Move(Destination::Store(d.join(STORE_NAME))));
        // Already on D:, whatever the answer.
        assert_eq!(decide(d.clone(), &stored_on(&d), Some(Choice::Copy)), Placement::Linked);
        // Game on C: with the mods root.
        assert_eq!(decide(c.clone(), &home(&c), None), Placement::Linked);
        // The game moved from D: to C: (its mods are on D:) or to E:.
        assert_eq!(decide(c.clone(), &stored_on(&d), None), Placement::Move(Destination::Home));
        assert_eq!(decide(e.clone(), &stored_on(&d), Some(Choice::Store)), Placement::Move(Destination::Store(e.join(STORE_NAME))));
        assert_eq!(decide(e.clone(), &stored_on(&d), Some(Choice::Copy)), Placement::Move(Destination::Home));
        assert!(matches!(decide(e.clone(), &stored_on(&d), None), Placement::Ask { .. }));
    }

    #[test]
    fn a_game_on_the_mods_volume_is_linked() {
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path().join("mods"));
        fs::create_dir_all(t.path().join("game")).unwrap();
        assert_eq!(placement(&root, "g", &t.path().join("game")), Placement::Linked);
    }

    #[test]
    fn moves_out_and_back_with_everything() {
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path().join("mods"));
        game_with_mods(&root);
        let store = t.path().join("vol").join(STORE_NAME);
        let mut seen = Vec::new();
        relocate(&root, "g", None, &Destination::Store(store.clone()), &mut |p| seen.push(p), &no_cancel()).unwrap();

        let dirs = root.game("g");
        assert_eq!(dirs.root(), store.join("g"));
        assert_eq!(store::load(&dirs).mods.len(), 1);
        assert_eq!(fs::read_to_string(dirs.mod_dir("m").join("Data/a.esp")).unwrap(), "plugin");
        assert_eq!(fs::read_to_string(dirs.overwrite().join("save.dat")).unwrap(), "save");
        assert!(dirs.downloads().join("m.zip").is_file());
        // Only the pointer is left home.
        let left: Vec<_> = fs::read_dir(root.home_of("g")).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from(POINTER)]);
        assert!(!moving(&root, "g"));
        let last = seen.last().copied().unwrap();
        assert!(last.done == last.total && last.total > 15, "{last:?}");
        assert_eq!(store::game_keys(&root), vec!["g".to_string()]);
        assert_eq!(stored(&root), vec![Stored { key: "g".into(), dir: store.join("g") }]);

        relocate(&root, "g", None, &Destination::Home, &mut |_| {}, &no_cancel()).unwrap();
        let dirs = root.game("g");
        assert_eq!(dirs.root(), root.home_of("g"));
        assert_eq!(fs::read_to_string(dirs.mod_dir("m").join("Data/a.esp")).unwrap(), "plugin");
        assert!(!root.home_of("g").join(POINTER).exists());
        assert!(!store.exists(), "the empty .heronite-mods goes too");
        assert!(stored(&root).is_empty());
    }

    #[test]
    fn a_game_without_mods_yet_points_at_its_drive() {
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path().join("mods"));
        let store = t.path().join("vol").join(STORE_NAME);
        relocate(&root, "new", None, &Destination::Store(store.clone()), &mut |_| {}, &no_cancel()).unwrap();
        let dirs = root.game("new");
        assert_eq!(dirs.root(), store.join("new"));
        store::save(&dirs, &ModList::new("new", "New")).unwrap();
        assert!(store.join("new/mods.json").is_file());
        assert_eq!(store::game_keys(&root), vec!["new".to_string()]);
    }

    #[test]
    fn keeps_file_times() {
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path().join("mods"));
        game_with_mods(&root);
        let file = root.game("g").overwrite().join("save.dat");
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        fs::File::options().write(true).open(&file).unwrap().set_modified(old).unwrap();
        let store = t.path().join("vol").join(STORE_NAME);
        relocate(&root, "g", None, &Destination::Store(store), &mut |_| {}, &no_cancel()).unwrap();
        let moved = root.game("g").overwrite().join("save.dat");
        assert_eq!(fs::metadata(moved).unwrap().modified().unwrap(), old);
    }

    #[test]
    fn a_taken_name_gets_a_number() {
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path().join("mods"));
        game_with_mods(&root);
        let store = t.path().join("vol").join(STORE_NAME);
        // Another Heronite's mods for the same game.
        put(&store.join("g/mods.json"), "{}");
        relocate(&root, "g", None, &Destination::Store(store.clone()), &mut |_| {}, &no_cancel()).unwrap();
        assert_eq!(root.game("g").root(), store.join("g-2"));
        assert_eq!(fs::read_to_string(store.join("g/mods.json")).unwrap(), "{}", "not ours: untouched");
    }

    #[test]
    fn a_cancel_leaves_the_mods_where_they_were() {
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path().join("mods"));
        game_with_mods(&root);
        let store = t.path().join("vol").join(STORE_NAME);
        let err = relocate(&root, "g", None, &Destination::Store(store.clone()), &mut |_| {}, &AtomicBool::new(true)).unwrap_err();
        assert!(!err.is_empty());
        assert_eq!(root.game("g").root(), root.home_of("g"));
        assert!(root.game("g").mod_dir("m").join("Data/a.esp").is_file());
        assert!(!store.join("g").exists());
        assert!(!moving(&root, "g"));
    }

    #[test]
    fn a_crash_before_the_switch_is_undone() {
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path().join("mods"));
        game_with_mods(&root);
        let home = root.home_of("g");
        let to = t.path().join("vol").join(STORE_NAME).join("g");
        write_json(&home.join(MOVING), &Move { from: home.clone(), to: to.clone() }).unwrap();
        put(&to.join("mods/m/Data/a.esp"), "half");
        assert!(moving(&root, "g"));
        sweep(&root);
        assert!(!to.exists());
        assert!(!moving(&root, "g"));
        assert_eq!(fs::read_to_string(root.game("g").mod_dir("m").join("Data/a.esp")).unwrap(), "plugin");
    }

    #[test]
    fn a_crash_after_the_switch_is_finished() {
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path().join("mods"));
        game_with_mods(&root);
        let home = root.home_of("g");
        let to = t.path().join("vol").join(STORE_NAME).join("g");
        copy_tree(&home, &to, &mut |_| {}, &no_cancel()).unwrap();
        write_json(&home.join(MOVING), &Move { from: home.clone(), to: to.clone() }).unwrap();
        write_json(&home.join(POINTER), &Pointer { path: to.clone() }).unwrap();
        sweep(&root);
        assert_eq!(root.game("g").root(), to);
        assert!(!home.join("mods").exists(), "the old copy goes");
        assert!(!home.join("mods.json").exists());
        assert!(to.join("mods/m/Data/a.esp").is_file());
    }

    #[test]
    fn a_pointer_out_of_a_store_is_ignored() {
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path().join("mods"));
        let home = root.home_of("g");
        fs::create_dir_all(&home).unwrap();
        write_json(&home.join(POINTER), &Pointer { path: t.path().join("elsewhere") }).unwrap();
        assert_eq!(root.game("g").root(), home);
        write_json(&home.join(POINTER), &Pointer { path: PathBuf::from("rel").join(STORE_NAME).join("g") }).unwrap();
        assert_eq!(root.game("g").root(), home);
    }

    /// Linux tells "in use" by its mount table, not by the journal.
    #[cfg(windows)]
    #[test]
    fn mods_applied_from_the_other_drive_are_in_use_and_stay_put() {
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path().join("mods"));
        game_with_mods(&root);
        let store = t.path().join("vol").join(STORE_NAME);
        relocate(&root, "g", None, &Destination::Store(store.clone()), &mut |_| {}, &no_cancel()).unwrap();
        let game = t.path().join("game");
        put(&game.join("Game.exe"), "exe");
        let dirs = root.game("g");
        let deployed = crate::journal_deploy::deploy(&dirs, &game, &[dirs.mod_dir("m")]).unwrap();
        assert_eq!(fs::read_to_string(game.join("Data/a.esp")).unwrap(), "plugin");
        assert!(crate::deployment::in_use(&root, &game), "found through the pointer");
        let err = relocate(&root, "g", Some(&game), &Destination::Home, &mut |_| {}, &no_cancel()).unwrap_err();
        assert!(err.contains("close it"), "{err}");
        assert_eq!(root.game("g").root(), store.join("g"));
        deployed.restore().unwrap();
        assert!(!game.join("Data").exists());
        assert!(!crate::deployment::in_use(&root, &game));
    }

    #[test]
    fn a_launch_waits_for_a_move() {
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path().join("mods"));
        game_with_mods(&root);
        let home = root.home_of("g");
        write_json(&home.join(MOVING), &Move { from: home.clone(), to: t.path().join("vol").join(STORE_NAME).join("g") }).unwrap();
        let game = t.path().join("game");
        fs::create_dir_all(&game).unwrap();
        let req = crate::apply::ApplyRequest {
            root: &root,
            tools_bin: &t.path().join("bin"),
            game_key: "g",
            game_dir: &game,
            home: None,
            known: None,
        };
        let err = crate::apply::apply(&req, &mut |_| {}, &no_cancel()).unwrap_err();
        assert!(err.contains("being moved"), "{err}");
    }

    #[test]
    fn forgetting_deletes_the_folder_on_the_other_drive() {
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path().join("mods"));
        game_with_mods(&root);
        let store = t.path().join("vol").join(STORE_NAME);
        relocate(&root, "g", None, &Destination::Store(store.clone()), &mut |_| {}, &no_cancel()).unwrap();
        store::forget_game(&root, "g", None).unwrap();
        assert!(!store.exists());
        assert!(!root.home_of("g").exists());
        assert!(store::game_keys(&root).is_empty());
    }
}
