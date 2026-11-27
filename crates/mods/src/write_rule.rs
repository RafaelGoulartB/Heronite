//! Which of the files a game writes while modded go to the overwrite folder,
//! and which stay in the game folder (`docs/MODS.md`, "What the game
//! writes"). Saves must stay with the game: a save made while modded must
//! still be there in unmodded play, and Steam Cloud must see it.
//!
//! * Only files under a **mod's own folders** go to overwrite: the top-level
//!   folders of the enabled mods (`BepInEx`, `Data`, `r6`, …). A file the
//!   game makes anywhere else, or at the root of its folder, stays.
//! * Folders the known-games table lists as the game's own
//!   ([`crate::games::KnownGame::keep`]), and folders named like saves
//!   (`Saves`, `SaveGames`, `Saved`, …), stay even inside a mod's folder.
//!
//! On Windows [`crate::journal_deploy`] applies the rule when it takes the
//! mods out. On Linux the overlay writes everything into the overwrite
//! folder; [`settle`] moves what the rule keeps in the game folder back into
//! it once the overlay is unmounted. A session file (the rule and a list of
//! what the overwrite folder held before) tells the files of this session
//! from older ones.
//!
//! Older ones are the [`migrate`] offer: on Linux every file went to the
//! overwrite folder before this rule, saves included. The manager offers once
//! to move back the files that lie outside the mods' folders.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use heronite_data::temps::write_atomic;
use heronite_i18n::tr;
use serde::{Deserialize, Serialize};

use crate::fsutil::{entries, walk_files};
use crate::journal_fs::{is_partial, move_file};
use crate::model::ModList;
use crate::overwrite::is_whiteout;
use crate::paths::{GameDirs, ModsRoot};

/// Folder names that hold saves, compared lower case with only letters and
/// digits kept (`Save Games` = `savegames`).
const SAVE_NAMES: &[&str] = &["save", "saves", "saved", "savegame", "savegames", "savedgames", "savedata", "savefiles", "saveslots"];

/// The rule of one deployment.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WriteRule {
    /// The top-level folders of the mods, lower case.
    pub mod_folders: BTreeSet<String>,
    /// Folders that stay in the game folder, lower case, relative, with `/`.
    pub keep: Vec<String>,
}

impl WriteRule {
    /// The rule for `layers` (mod folders; not the overwrite folder, whose
    /// own folders say nothing about the mods) and the game's `keep` list.
    pub fn new(layers: &[PathBuf], keep: &[&str]) -> Self {
        let mod_folders = layers.iter().flat_map(|l| entries(l)).filter(|(_, is_dir)| *is_dir).map(|(n, _)| n.to_lowercase()).collect();
        let keep = keep.iter().map(|k| k.replace('\\', "/").trim_matches('/').to_lowercase()).filter(|k| !k.is_empty()).collect();
        Self { mod_folders, keep }
    }

    /// The rule over every installed mod, enabled or not: what the migration
    /// leaves in the overwrite folder, so a disabled mod's configuration is
    /// not taken for the game's.
    pub fn for_list(dirs: &GameDirs, list: &ModList, keep: &[&str]) -> Self {
        let layers: Vec<PathBuf> = list.mods.iter().map(|m| dirs.mod_dir(&m.id)).collect();
        Self::new(&layers, keep)
    }

    /// Whether a file the game wrote at `rel` (relative to the game folder,
    /// with `/`) goes to the overwrite folder.
    pub fn to_overwrite(&self, rel: &str) -> bool {
        let lower = rel.replace('\\', "/").to_lowercase();
        let parts: Vec<&str> = lower.split('/').filter(|p| !p.is_empty()).collect();
        // A file at the root is the game's, whatever its name.
        let Some((_, folders)) = parts.split_last() else { return false };
        let Some(first) = folders.first() else { return false };
        if !self.mod_folders.contains(*first) {
            return false;
        }
        if self.keep.iter().any(|k| lower == *k || lower.starts_with(&format!("{k}/"))) {
            return false;
        }
        !folders.iter().any(|f| looks_like_saves(f))
    }
}

/// Whether a folder's name says it holds saves.
pub fn looks_like_saves(name: &str) -> bool {
    let n: String = name.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
    SAVE_NAMES.contains(&n.as_str())
}

/// Whether `rel` in the overwrite folder is a deletion or a marker of the
/// overlay, not a file of the game.
fn is_overlay_entry(rel: &str, meta: &fs::Metadata) -> bool {
    is_whiteout(rel, meta) || !meta.is_file()
}

/// Size and modification time (ns since 1970).
fn stamp(meta: &fs::Metadata) -> (u64, u64) {
    let modified = meta.modified().ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
    (meta.len(), u64::try_from(modified).unwrap_or(u64::MAX))
}

/// What a mount on Linux left to sort out once it is gone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    game_dir: PathBuf,
    rule: WriteRule,
    /// The overwrite folder's files before the game ran, with their size and
    /// time: those are not this session's.
    before: BTreeMap<String, (u64, u64)>,
}

fn read_session(dirs: &GameDirs) -> Option<Session> {
    let bytes = fs::read(dirs.write_session()).ok()?;
    serde_json::from_slice(&bytes).map_err(|e| log::warn!("mods: {} cannot be read: {e}", dirs.write_session().display())).ok()
}

/// Notes what the overwrite folder holds before the overlay is mounted, and
/// the rule. Call it before mounting.
pub fn begin_session(dirs: &GameDirs, game_dir: &Path, rule: &WriteRule) -> Result<(), String> {
    let mut before = BTreeMap::new();
    walk_files(&dirs.overwrite(), &mut |rel, meta| {
        if !is_overlay_entry(rel, meta) {
            before.insert(rel.to_string(), stamp(meta));
        }
    });
    let session = Session { game_dir: game_dir.to_path_buf(), rule: rule.clone(), before };
    let file = dirs.write_session();
    let bytes = serde_json::to_vec_pretty(&session).map_err(|e| e.to_string())?;
    write_atomic(&file, &bytes, None).map_err(|e| format!("{}: {e}", file.display()))
}

/// Forgets a session that never started (the mount failed).
pub fn drop_session(dirs: &GameDirs) {
    let _ = fs::remove_file(dirs.write_session());
}

/// The game folder of a session not settled yet.
pub fn session_game_dir(dirs: &GameDirs) -> Option<PathBuf> {
    read_session(dirs).map(|s| s.game_dir)
}

/// What [`settle`] or [`migrate`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Moved {
    /// Files moved into the game folder (or already there, the same).
    pub moved: usize,
    /// Files left in the overwrite folder: the game folder has a different
    /// file there, or a folder.
    pub left: Vec<String>,
}

/// After the overlay is gone (Linux): moves the files this session wrote
/// that the rule keeps with the game from the overwrite folder into the game
/// folder, replacing the game's copy (the game wrote over it, as it would
/// unmodded). Only call it once nothing is mounted over the game folder.
pub fn settle(dirs: &GameDirs) -> Result<Moved, String> {
    let Some(session) = read_session(dirs) else { return Ok(Moved::default()) };
    let game = session.game_dir.clone();
    if !game.is_dir() {
        return Err(tr!("The game folder {path} is not there (an unplugged drive?)", path = game.display()));
    }
    let mut wanted = Vec::new();
    walk_files(&dirs.overwrite(), &mut |rel, meta| {
        if is_overlay_entry(rel, meta) || is_partial(rel) || session.rule.to_overwrite(rel) {
            return;
        }
        if session.before.get(rel) != Some(&stamp(meta)) {
            wanted.push(rel.to_string());
        }
    });
    let done = move_out(dirs, &game, &wanted, true);
    drop_session(dirs);
    if done.moved > 0 || !done.left.is_empty() {
        log::info!("mods: {} files the game wrote moved back into {} ({} left in overwrite)", done.moved, game.display(), done.left.len());
    }
    Ok(done)
}

/// Settles the session of the game whose folder is `dir`, if one waits
/// (an overlay unmounted after the app restarted, or at start after a crash).
pub fn settle_folder(root: &ModsRoot, dir: &Path) {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    for e in fs::read_dir(root.path()).into_iter().flatten().flatten() {
        if !e.path().is_dir() {
            continue;
        }
        let dirs = root.game(&e.file_name().to_string_lossy());
        if session_game_dir(&dirs).is_some_and(|g| g == dir || canon(&g) == canon(dir)) {
            if let Err(e) = settle(&dirs) {
                log::warn!("mods: {e}");
            }
        }
    }
}

/// The files of the overwrite folder that lie outside the mods' folders
/// (saves, mostly), sorted.
pub fn outside_files(dirs: &GameDirs, rule: &WriteRule) -> Vec<String> {
    let mut v = Vec::new();
    walk_files(&dirs.overwrite(), &mut |rel, meta| {
        if !is_overlay_entry(rel, meta) && !is_partial(rel) && !rule.to_overwrite(rel) {
            v.push(rel.to_string());
        }
    });
    v.sort();
    v
}

/// The one-time offer: moves the files of the overwrite folder that lie
/// outside the mods' folders into the game folder. A file the game folder
/// has already is left where it is unless both are the same, so nothing the
/// game has is replaced. The mods must not be applied meanwhile.
pub fn migrate(dirs: &GameDirs, game_dir: &Path, rule: &WriteRule) -> Result<Moved, String> {
    if crate::journal_deploy::is_deployed(dirs) || dirs.write_session().is_file() {
        return Err(tr!("The game is running with mods; close it first").to_string());
    }
    if !game_dir.is_dir() {
        return Err(tr!("The game folder {path} is not there (an unplugged drive?)", path = game_dir.display()));
    }
    let wanted = outside_files(dirs, rule);
    let done = move_out(dirs, game_dir, &wanted, false);
    log::info!("mods: {} files moved from overwrite into {} ({} left)", done.moved, game_dir.display(), done.left.len());
    Ok(done)
}

/// Moves `rels` from the overwrite folder into `game`. `replace`: a file the
/// game folder has is replaced; else it is left unless it is the same. Then
/// removes the folders it emptied, and the overlay's "replaced folder" mark
/// of each folder it took files from, which would otherwise hide the game's
/// folder (and the files just moved into it) in the next modded run.
fn move_out(dirs: &GameDirs, game: &Path, rels: &[String], replace: bool) -> Moved {
    let ow = dirs.overwrite();
    let mut done = Moved::default();
    let mut from_dirs: HashSet<PathBuf> = HashSet::new();
    for rel in rels {
        let (from, to) = (ow.join(rel), game.join(rel));
        let result = match to.symlink_metadata() {
            Ok(m) if m.is_dir() => Ok(false),
            Ok(_) if !replace => same_file(&from, &to).and_then(|same| if same { fs::remove_file(&from).map(|_| true) } else { Ok(false) }),
            _ => to.parent().map_or(Ok(()), fs::create_dir_all).and_then(|_| move_file(&from, &to)).map(|_| true),
        };
        match result {
            Ok(true) => {
                done.moved += 1;
                if let Some(p) = from.parent() {
                    from_dirs.insert(p.to_path_buf());
                }
            }
            Ok(false) => done.left.push(rel.clone()),
            Err(e) => {
                log::warn!("mods: could not move {rel} into the game folder: {e}");
                done.left.push(rel.clone());
            }
        }
    }
    for d in &from_dirs {
        let mut dir = d.clone();
        while dir.starts_with(&ow) && dir != ow {
            clear_opaque(&dir);
            if fs::remove_dir(&dir).is_err() && dir.exists() {
                break;
            }
            let Some(parent) = dir.parent() else { break };
            dir = parent.to_path_buf();
        }
    }
    done
}

/// Whether two files hold the same bytes.
fn same_file(a: &Path, b: &Path) -> io::Result<bool> {
    if fs::metadata(a)?.len() != fs::metadata(b)?.len() {
        return Ok(false);
    }
    Ok(fs::read(a)? == fs::read(b)?)
}

/// Removes the overlay's mark that a folder replaces the lower ones: the
/// `.wh..wh..opq` file, and on Linux the extended attributes
/// `fuse-overlayfs` and the kernel use.
fn clear_opaque(dir: &Path) {
    let _ = fs::remove_file(dir.join(".wh..wh..opq"));
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let Ok(path) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else { return };
        for name in [c"user.fuseoverlayfs.opaque", c"user.overlay.opaque", c"trusted.overlay.opaque"] {
            // SAFETY: both are valid NUL-terminated strings for the call's duration.
            unsafe { libc::lremovexattr(path.as_ptr(), name.as_ptr()) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(path: PathBuf, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn read(path: PathBuf) -> String {
        fs::read_to_string(path).unwrap()
    }

    fn rule(folders: &[&str], keep: &[&str]) -> WriteRule {
        WriteRule { mod_folders: folders.iter().map(|f| f.to_lowercase()).collect(), ..WriteRule::new(&[], keep) }
    }

    #[test]
    fn only_the_mods_folders_go_to_overwrite() {
        let r = rule(&["BepInEx", "Data"], &[]);
        assert!(r.to_overwrite("BepInEx/config/mod.cfg"));
        assert!(r.to_overwrite("bepinex/LogOutput.log"), "case does not matter");
        assert!(r.to_overwrite("Data/SKSE/Plugins/x.ini"));
        assert!(!r.to_overwrite("Saves/slot1.sav"), "not a folder of the mods");
        assert!(!r.to_overwrite("output_log.txt"), "a file at the root is the game's");
        assert!(!r.to_overwrite("BepInEx"), "a file named like a mod folder");
    }

    #[test]
    fn listed_folders_and_folders_named_like_saves_stay() {
        let r = rule(&["Data", "Game"], &["Data/Profiles/", "game\\User"]);
        assert!(!r.to_overwrite("Data/Profiles/1/save.dat"));
        assert!(!r.to_overwrite("data/profiles"), "the listed name itself, as a file");
        assert!(!r.to_overwrite("Game/User/prefs.json"));
        assert!(r.to_overwrite("Data/ProfilesExtra/x"), "a prefix of a name is not the folder");
        assert!(!r.to_overwrite("Game/Saved/SaveGames/0.sav"));
        assert!(!r.to_overwrite("Data/Save Games/a.ess"));
        assert!(r.to_overwrite("Data/SaveManager.dll/x"), "named like saves only when it is the whole name");
        assert!(r.to_overwrite("Data/saves.txt"), "the file's own name is not a folder");
        for n in ["Saves", "SAVEGAMES", "save_data", "Saved Games", "SaveSlots"] {
            assert!(looks_like_saves(n), "{n}");
        }
        for n in ["Savior", "autosave2", "config"] {
            assert!(!looks_like_saves(n), "{n}");
        }
    }

    #[test]
    fn the_rule_takes_the_mods_top_folders_not_their_files() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        put(a.join("BepInEx/plugins/A/a.dll"), "a");
        put(a.join("winhttp.dll"), "dll");
        put(b.join("Data/b.esp"), "b");
        let r = WriteRule::new(&[a, b], &["Saves"]);
        assert_eq!(r.mod_folders.iter().map(String::as_str).collect::<Vec<_>>(), ["bepinex", "data"]);
        assert_eq!(r.keep, ["saves"]);
    }

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
        put(game.join("settings.cfg"), "game");
        fs::create_dir_all(dirs.overwrite()).unwrap();
        Env { _t: t, dirs, game }
    }

    #[test]
    fn after_the_overlay_the_games_files_go_back_and_older_ones_stay() {
        let e = env();
        // From before this rule: an old save and a mod's config.
        put(e.dirs.overwrite().join("Saves/old.sav"), "old");
        put(e.dirs.overwrite().join("BepInEx/config/a.cfg"), "cfg");
        let r = rule(&["BepInEx"], &[]);
        begin_session(&e.dirs, &e.game, &r).unwrap();
        assert_eq!(session_game_dir(&e.dirs).as_deref(), Some(e.game.as_path()));
        // What the overlay's upper layer holds after the game ran.
        put(e.dirs.overwrite().join("Saves/new.sav"), "new");
        put(e.dirs.overwrite().join("settings.cfg"), "changed by the game");
        put(e.dirs.overwrite().join("BepInEx/LogOutput.log"), "log");
        put(e.dirs.overwrite().join("Saves/.wh.gone.sav"), "");
        let done = settle(&e.dirs).unwrap();
        assert_eq!(done, Moved { moved: 2, left: vec![] });
        assert_eq!(read(e.game.join("Saves/new.sav")), "new");
        assert_eq!(read(e.game.join("settings.cfg")), "changed by the game", "written over, as it would be unmodded");
        assert_eq!(read(e.dirs.overwrite().join("BepInEx/LogOutput.log")), "log");
        assert_eq!(read(e.dirs.overwrite().join("Saves/old.sav")), "old", "older files wait for the offer");
        assert!(e.dirs.overwrite().join("Saves/.wh.gone.sav").exists(), "deletions stay");
        assert!(!e.game.join("Saves/old.sav").exists());
        assert!(!e.dirs.write_session().exists());
        assert_eq!(settle(&e.dirs).unwrap(), Moved::default(), "nothing waits any more");
    }

    #[test]
    fn a_file_changed_this_session_goes_back_even_if_it_was_there_before() {
        let e = env();
        put(e.dirs.overwrite().join("Saves/slot.sav"), "v1");
        begin_session(&e.dirs, &e.game, &rule(&[], &[])).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        put(e.dirs.overwrite().join("Saves/slot.sav"), "v2, longer");
        assert_eq!(settle(&e.dirs).unwrap().moved, 1);
        assert_eq!(read(e.game.join("Saves/slot.sav")), "v2, longer");
        assert!(!e.dirs.overwrite().join("Saves").exists(), "the emptied folder is gone");
    }

    #[test]
    fn the_replaced_folder_mark_goes_with_the_files() {
        let e = env();
        begin_session(&e.dirs, &e.game, &rule(&[], &[])).unwrap();
        put(e.dirs.overwrite().join("Saves/.wh..wh..opq"), "");
        put(e.dirs.overwrite().join("Saves/a.sav"), "a");
        settle(&e.dirs).unwrap();
        assert_eq!(read(e.game.join("Saves/a.sav")), "a");
        assert!(!e.dirs.overwrite().join("Saves").exists());
    }

    #[test]
    fn settling_finds_the_game_by_its_folder() {
        let e = env();
        begin_session(&e.dirs, &e.game, &rule(&[], &[])).unwrap();
        put(e.dirs.overwrite().join("save.dat"), "s");
        let root = ModsRoot::at(e.dirs.root().parent().unwrap());
        settle_folder(&root, &e.game.join("elsewhere"));
        assert!(e.dirs.write_session().exists(), "another folder");
        settle_folder(&root, &e.game);
        assert_eq!(read(e.game.join("save.dat")), "s");
    }

    #[test]
    fn the_offer_moves_what_lies_outside_and_never_replaces_the_games_files() {
        let e = env();
        put(e.dirs.mod_dir("bep").join("BepInEx/plugins/x.dll"), "x");
        put(e.dirs.overwrite().join("Saves/slot1.sav"), "s1");
        put(e.dirs.overwrite().join("BepInEx/config/x.cfg"), "cfg");
        put(e.dirs.overwrite().join("settings.cfg"), "a copy changed while modded");
        put(e.dirs.overwrite().join("game.exe"), "exe");
        put(e.dirs.overwrite().join(".wh.removed.txt"), "");
        let mut list = ModList::new("g", "G");
        list.mods.push(crate::model::ModEntry { id: "bep".into(), enabled: false, ..Default::default() });
        let r = WriteRule::for_list(&e.dirs, &list, &[]);
        assert_eq!(outside_files(&e.dirs, &r), ["Saves/slot1.sav", "game.exe", "settings.cfg"]);
        let done = migrate(&e.dirs, &e.game, &r).unwrap();
        assert_eq!(done, Moved { moved: 2, left: vec!["settings.cfg".into()] });
        assert_eq!(read(e.game.join("Saves/slot1.sav")), "s1");
        assert_eq!(read(e.game.join("settings.cfg")), "game", "the game's own file stays");
        assert_eq!(read(e.dirs.overwrite().join("settings.cfg")), "a copy changed while modded");
        assert!(!e.dirs.overwrite().join("game.exe").exists(), "the same file: the copy goes");
        assert_eq!(read(e.dirs.overwrite().join("BepInEx/config/x.cfg")), "cfg", "a disabled mod's folder counts");
        assert!(e.dirs.overwrite().join(".wh.removed.txt").exists());
    }

    #[test]
    fn the_offer_waits_while_the_mods_are_applied() {
        let e = env();
        begin_session(&e.dirs, &e.game, &rule(&[], &[])).unwrap();
        put(e.dirs.overwrite().join("save.dat"), "s");
        assert!(migrate(&e.dirs, &e.game, &rule(&[], &[])).is_err());
        assert!(e.dirs.overwrite().join("save.dat").exists());
    }
}
