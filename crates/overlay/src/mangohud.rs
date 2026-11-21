//! MangoHud's HUD in a running game whose MangoHud config Heronite did not
//! write (a Steam game with `MANGOHUD=1` or `mangohud %command%` in its
//! launch options, a game with a `MANGOHUD_CONFIG` of the user's own...),
//! shown and hidden without touching how the game was started.
//!
//! MangoHud watches (inotify) the one config file it picked when the game
//! started (`MANGOHUD_CONFIGFILE`, else the first of `<program>.conf`,
//! `wine-<exe>.conf`, `MangoHud.conf`...) and rereads it on every change. The
//! game's `/proc/<pid>/fdinfo` names that file as the kernel knows it (inode
//! and device), so the file is found from what the game really watches,
//! never guessed. Hiding appends a block of ours (`no_display=1`) to it;
//! showing again removes it. The block is also removed when the game ends,
//! when the app quits and when it starts again (after a crash), from a list
//! of the files changed and a look through MangoHud's config folder.
//!
//! Anything uncertain leaves the HUD as it is and the toggle out: no
//! MangoHud in the game, a `MANGOHUD_CONFIG` that wins over the file or
//! does not read it, a file reached through a link (someone's dotfiles), a
//! hard-linked one, one that cannot be written or is not text. Only `/proc`
//! is read and only MangoHud's own config file is written: the game and its
//! launch stay as they are.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The line our block starts with; the `no_display` line follows it.
const MARK: &str = "# Heronite: HUD shown or hidden by the in-game overlay (removed when the game ends)";

/// One change at a time: a toggle and a restore (game ended, app quitting)
/// must not interleave, or a block could outlive the list of changed files.
static LOCK: Mutex<()> = Mutex::new(());

/// MangoHud's config files of a running game, as it watches them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchedHud {
    /// Usually one; a launcher and its game may each watch their own.
    pub files: Vec<PathBuf>,
    /// The HUD shows now.
    pub shown: bool,
}

/// The config files MangoHud watches in the game whose process (group
/// leader) is `game_pid`, and for a Steam game (`steam_app_id`) in what
/// Steam runs under its `reaper` (the pid Steam names is its runtime's
/// launch client, apart from the game); `None` when the HUD cannot be shown
/// or hidden this way. Blocking (`/proc`).
#[cfg(target_os = "linux")]
pub fn find(game_pid: u32, steam_app_id: Option<&str>) -> Option<WatchedHud> {
    let mut procs = crate::procs::game_processes(game_pid);
    if let Some(id) = steam_app_id.filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())) {
        for p in crate::procs::steam_game_processes(id) {
            if !procs.contains(&p) {
                procs.push(p);
            }
        }
    }
    let mut files: Vec<PathBuf> = Vec::new();
    let mut seen = 0;
    for pid in procs.iter().copied() {
        if !loads_mangohud(pid) {
            continue;
        }
        seen += 1;
        let Ok(env) = std::fs::read(format!("/proc/{pid}/environ")) else {
            log::debug!("MangoHud toggle: game {game_pid}: the environment of {pid} cannot be read");
            return None;
        };
        let var = |key: &str| crate::procs::environ_value(&env, key);
        // Applied over the file on every reload, or the file is not read.
        if !file_counts(var("MANGOHUD_CONFIG").as_deref()) {
            log::debug!("MangoHud toggle: game {game_pid}: {pid}'s MANGOHUD_CONFIG decides the HUD");
            return None;
        }
        // MangoHud's watch waits for modifications; other inotify users of
        // the process (SDL's input devices...) do not count.
        let watched: Vec<(u64, u64)> =
            inotify_watches(pid).into_iter().filter(|w| w.mask & IN_MODIFY != 0).map(|w| (w.ino, w.sdev)).collect();
        if watched.is_empty() {
            log::debug!("MangoHud toggle: game {game_pid}: {pid} loads MangoHud but watches no file");
            continue;
        }
        let candidates = candidates(&var);
        // A process whose file cannot be placed (or used) keeps its HUD as it is.
        let Some(file) = candidates.iter().filter(|f| usable(f)).find(|f| kernel_id(f).is_some_and(|id| watched.contains(&id))).cloned()
        else {
            log::debug!(
                "MangoHud toggle: game {game_pid}: none of {candidates:?} is what {pid} watches ({watched:x?}; usable: {:?})",
                candidates.iter().filter(|f| usable(f)).map(|f| (f, kernel_id(f))).collect::<Vec<_>>()
            );
            continue;
        };
        if !files.contains(&file) {
            files.push(file);
        }
    }
    if files.is_empty() {
        log::info!("MangoHud toggle: game {game_pid}: {} processes, {seen} with MangoHud, no config file to use", procs.len());
        return None;
    }
    let shown = files.iter().all(|f| std::fs::read_to_string(f).is_ok_and(|t| !hides_hud(&t)));
    log::info!("MangoHud toggle: game {game_pid}: {files:?} (shown: {shown})");
    Some(WatchedHud { files, shown })
}

#[cfg(not(target_os = "linux"))]
pub fn find(_game_pid: u32, _steam_app_id: Option<&str>) -> Option<WatchedHud> {
    None
}

/// Shows or hides the HUD: writes or removes our block in each file
/// (blocking). `record` first lists the files, so a block left by a crash is
/// removed at the next start ([`restore_recorded`]).
pub fn set_shown(hud: &WatchedHud, shown: bool, record: &Path) -> Result<(), String> {
    let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    remember(record, &hud.files).map_err(|e| format!("{}: {e}", record.display()))?;
    for file in &hud.files {
        edit(file, |base| {
            let base = without_block(base);
            if hides_hud(&base) == !shown {
                base
            } else {
                with_block(&base, shown)
            }
        })
        .map_err(|e| format!("{}: {e}", file.display()))?;
    }
    Ok(())
}

/// Removes our block from the files `record` lists and from those of
/// MangoHud's config folder, then the list itself (at start, after a crash,
/// and when the app quits). Blocking.
pub fn restore_recorded(record: &Path) {
    let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    restore_listed(record, None);
    #[cfg(target_os = "linux")]
    for file in default_folder_files() {
        if std::fs::read_to_string(&file).is_ok_and(|t| t.contains(MARK)) {
            if let Err(e) = restore(&file) {
                log::warn!("MangoHud: could not restore {}: {e}", file.display());
            }
        }
    }
}

/// Removes our block from `files` (their game ended), listed or not, and
/// takes them off `record`. Blocking.
pub fn restore_files(files: &[PathBuf], record: &Path) {
    let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for file in files {
        if let Err(e) = restore(file) {
            log::warn!("MangoHud: could not restore {}: {e}", file.display());
        }
    }
    restore_listed(record, Some(files));
}

/// Restores the listed files (`only` those, when given) and rewrites the
/// list without them. One that cannot be restored stays listed while it
/// exists, to be tried again.
fn restore_listed(record: &Path, only: Option<&[PathBuf]>) {
    let Ok(list) = std::fs::read_to_string(record) else { return };
    let mut kept = Vec::new();
    for line in list.lines().filter(|l| !l.trim().is_empty()) {
        let file = PathBuf::from(line);
        if only.is_some_and(|o| !o.contains(&file)) {
            kept.push(line.to_string());
            continue;
        }
        if let Err(e) = restore(&file) {
            log::warn!("MangoHud: could not restore {}: {e}", file.display());
            if file.exists() {
                kept.push(line.to_string());
            }
        }
    }
    let result = match kept.is_empty() {
        true => std::fs::remove_file(record),
        false => heronite_data::temps::write_atomic(record, (kept.join("\n") + "\n").as_bytes(), None),
    };
    if let Err(e) = result {
        log::warn!("MangoHud: {}: {e}", record.display());
    }
}

/// Removes our block from `file` (nothing when it has none or is gone).
fn restore(file: &Path) -> Result<(), String> {
    if std::fs::symlink_metadata(file).is_err() {
        return Ok(());
    }
    edit(file, without_block)
}

/// Adds `files` to the list (written whole and replaced: Heronite's own file).
fn remember(record: &Path, files: &[PathBuf]) -> std::io::Result<()> {
    let list = std::fs::read_to_string(record).unwrap_or_default();
    let mut lines: Vec<String> = list.lines().filter(|l| !l.trim().is_empty()).map(String::from).collect();
    let before = lines.len();
    for f in files {
        // `plain_path` ones are UTF-8 without line breaks.
        let Some(f) = f.to_str() else { continue };
        if !lines.iter().any(|l| l == f) {
            lines.push(f.to_string());
        }
    }
    if lines.len() == before {
        return Ok(());
    }
    if let Some(dir) = record.parent() {
        std::fs::create_dir_all(dir)?;
    }
    heronite_data::temps::write_atomic(record, (lines.join("\n") + "\n").as_bytes(), None)
}

/// Rewrites `file` with `change` of its text, through one open file: a
/// regular file, never through a link (its own name or a folder's: it could
/// be someone's dotfiles repository) nor a hard-linked one. MangoHud watches
/// its inode, so it is written in place, never replaced.
fn edit(file: &Path, change: impl FnOnce(&str) -> String) -> Result<(), String> {
    if !plain_path(file) {
        return Err("not a plain file (a link, or a path with a link in it)".into());
    }
    let mut f = open_plain(file).map_err(|e| e.to_string())?;
    let meta = f.metadata().map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || hard_linked(&meta) {
        return Err("not a plain file".into());
    }
    let mut old = String::new();
    f.read_to_string(&mut old).map_err(|e| e.to_string())?;
    let new = change(&old);
    write_in_place(&mut f, &old, &new).map_err(|e| e.to_string())
}

#[cfg(target_os = "linux")]
fn open_plain(file: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().read(true).write(true).custom_flags(libc::O_NOFOLLOW).open(file)
}

#[cfg(not(target_os = "linux"))]
fn open_plain(file: &Path) -> std::io::Result<File> {
    std::fs::OpenOptions::new().read(true).write(true).open(file)
}

#[cfg(unix)]
fn hard_linked(meta: &std::fs::Metadata) -> bool {
    std::os::unix::fs::MetadataExt::nlink(meta) > 1
}

#[cfg(not(unix))]
fn hard_linked(_meta: &std::fs::Metadata) -> bool {
    false
}

/// An absolute path to a regular file with no link in it, as UTF-8 without
/// line breaks (it goes into the list of changed files, one per line).
fn plain_path(file: &Path) -> bool {
    file.is_absolute()
        && file.to_str().is_some_and(|s| !s.contains(['\n', '\r']))
        && std::fs::canonicalize(file).is_ok_and(|c| c == file)
        && std::fs::symlink_metadata(file).is_ok_and(|m| m.file_type().is_file())
}

/// Writes `new` over `old` in the open file, from the first byte that
/// differs, then cuts what is left. The file never reads empty in between:
/// a block at the end comes and goes without the rest being written again.
fn write_in_place(f: &mut File, old: &str, new: &str) -> std::io::Result<()> {
    if old == new {
        return Ok(());
    }
    let same = old.bytes().zip(new.bytes()).take_while(|(a, b)| a == b).count();
    if same < new.len() {
        f.seek(SeekFrom::Start(same as u64))?;
        f.write_all(&new.as_bytes()[same..])?;
    }
    if new.len() < old.len() {
        f.set_len(new.len() as u64)?;
    }
    Ok(())
}

/// `text` with our block at the end, saying whether the HUD shows.
fn with_block(text: &str, shown: bool) -> String {
    let mut out = text.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(MARK);
    out.push('\n');
    out.push_str(&format!("no_display={}\n", u8::from(!shown)));
    out
}

/// `text` without our blocks (the mark and the `no_display` line after it).
fn without_block(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut lines = text.split_inclusive('\n').peekable();
    while let Some(line) = lines.next() {
        if line.trim_end() == MARK {
            lines.next_if(|l| l.trim_start().starts_with("no_display"));
            continue;
        }
        out.push_str(line);
    }
    out
}

/// Whether a MangoHud config hides the HUD: its last `no_display` line, bare
/// or not `0` (a `#` starts a comment).
fn hides_hud(config: &str) -> bool {
    config
        .lines()
        .rev()
        .find_map(|l| {
            let l = l.split('#').next().unwrap_or("");
            let (key, value) = l.split_once('=').map_or((l, None), |(k, v)| (k, Some(v)));
            (key.trim() == "no_display").then(|| value.is_none_or(|v| v.trim().parse::<i64>().ok().is_none_or(|n| n != 0)))
        })
        .unwrap_or(false)
}

/// Whether MangoHud's config file decides the HUD with `MANGOHUD_CONFIG` as
/// it is: none, or one that reads the file (`read_cfg`) and does not set
/// `no_display` itself (it is applied over the file on every reload).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn file_counts(config: Option<&str>) -> bool {
    let Some(config) = config else { return true };
    let keys: Vec<&str> = config.split([',', ':', ';']).map(|o| o.split('=').next().unwrap_or("").trim()).collect();
    keys.contains(&"read_cfg") && !keys.contains(&"no_display")
}

/// MangoHud's config files the game may be watching: its
/// `MANGOHUD_CONFIGFILE`, else the `.conf` files of its MangoHud config
/// folders (and of this app's own, the same folder outside a container).
#[cfg(target_os = "linux")]
fn candidates(var: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    if let Some(file) = var("MANGOHUD_CONFIGFILE").filter(|f| !f.is_empty()) {
        return vec![PathBuf::from(file)];
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    for dir in [config_home(var("XDG_CONFIG_HOME"), var("HOME")), own_config_home()].into_iter().flatten() {
        let dir = dir.join("MangoHud");
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs.iter().flat_map(|d| conf_files(d)).collect()
}

/// The `.conf` files of this app's MangoHud config folder (where a block
/// left by a crash is looked for).
#[cfg(target_os = "linux")]
fn default_folder_files() -> Vec<PathBuf> {
    own_config_home().map(|d| conf_files(&d.join("MangoHud"))).unwrap_or_default().into_iter().filter(|f| plain_path(f)).collect()
}

#[cfg(target_os = "linux")]
fn config_home(xdg: Option<String>, home: Option<String>) -> Option<PathBuf> {
    xdg.filter(|d| !d.is_empty()).map(PathBuf::from).or_else(|| home.filter(|h| !h.is_empty()).map(|h| Path::new(&h).join(".config")))
}

#[cfg(target_os = "linux")]
fn own_config_home() -> Option<PathBuf> {
    config_home(std::env::var("XDG_CONFIG_HOME").ok(), std::env::var("HOME").ok())
}

#[cfg(target_os = "linux")]
fn conf_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "conf")).collect()
}

/// A file the HUD can be shown and hidden through: [`plain_path`], not hard
/// linked, writable by this user and text.
#[cfg(target_os = "linux")]
fn usable(file: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    if !plain_path(file) || std::fs::symlink_metadata(file).is_ok_and(|m| hard_linked(&m)) {
        return false;
    }
    let Ok(c) = std::ffi::CString::new(file.as_os_str().as_bytes()) else { return false };
    // SAFETY: `c` is a valid NUL-terminated path; `access` only checks it.
    let writable = unsafe { libc::access(c.as_ptr(), libc::W_OK) } == 0;
    writable && std::fs::read_to_string(file).is_ok()
}

/// The file as the kernel names it in inotify's fdinfo (inode, device): a
/// watch of our own on it, read back and dropped. Adding a watch sends no
/// event to anyone. `stat`'s device differs on btrfs subvolumes, so the
/// game's watches are compared with these, never with `stat`.
#[cfg(target_os = "linux")]
fn kernel_id(file: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(file.as_os_str().as_bytes()).ok()?;
    // SAFETY: a new inotify fd of ours, closed below.
    let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC | libc::IN_NONBLOCK) };
    if fd < 0 {
        return None;
    }
    // SAFETY: `fd` is ours and `c` is a valid path.
    let wd = unsafe { libc::inotify_add_watch(fd, c.as_ptr(), libc::IN_DELETE_SELF | libc::IN_DONT_FOLLOW) };
    let id = (wd >= 0)
        .then(|| std::fs::read_to_string(format!("/proc/self/fdinfo/{fd}")).ok())
        .flatten()
        .and_then(|info| parse_fdinfo(&info).into_iter().next())
        .map(|w| (w.ino, w.sdev));
    // SAFETY: closes the fd opened above.
    unsafe { libc::close(fd) };
    id
}

/// Whether MangoHud's library is loaded in the process (`maps` may hold
/// paths that are not UTF-8: searched as bytes).
#[cfg(target_os = "linux")]
fn loads_mangohud(pid: u32) -> bool {
    const LIB: &[u8] = b"libMangoHud";
    std::fs::read(format!("/proc/{pid}/maps")).is_ok_and(|m| m.windows(LIB.len()).any(|w| w == LIB))
}

#[cfg(target_os = "linux")]
const IN_MODIFY: u32 = 0x2;

/// One inotify watch of an fdinfo.
#[cfg(target_os = "linux")]
#[derive(Debug, PartialEq, Eq)]
struct Watch {
    ino: u64,
    /// The kernel's device number, as fdinfo gives it.
    sdev: u64,
    mask: u32,
}

/// Every inotify watch of the process.
#[cfg(target_os = "linux")]
fn inotify_watches(pid: u32) -> Vec<Watch> {
    let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else { return Vec::new() };
    let mut out = Vec::new();
    for fd in fds.flatten() {
        if std::fs::read_link(fd.path()).ok().and_then(|l| l.to_str().map(|s| s == "anon_inode:inotify")) != Some(true) {
            continue;
        }
        let Ok(info) = std::fs::read_to_string(format!("/proc/{pid}/fdinfo/{}", fd.file_name().to_string_lossy())) else { continue };
        out.extend(parse_fdinfo(&info));
    }
    out
}

/// The `inotify wd:.. ino:<hex> sdev:<hex> mask:<hex>` lines of an fdinfo.
#[cfg(target_os = "linux")]
fn parse_fdinfo(info: &str) -> Vec<Watch> {
    info.lines()
        .filter_map(|l| {
            let l = l.strip_prefix("inotify")?;
            let field = |name: &str| l.split_whitespace().find_map(|w| w.strip_prefix(name)).and_then(|v| u64::from_str_radix(v, 16).ok());
            Some(Watch { ino: field("ino:")?, sdev: field("sdev:")?, mask: u32::try_from(field("mask:")?).ok()? })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test folder by its real path (a tempdir may sit behind a link).
    fn tempdir() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let real = std::fs::canonicalize(tmp.path()).unwrap();
        (tmp, real)
    }

    #[test]
    fn the_block_comes_and_goes_and_leaves_the_users_config_as_it_was() {
        let user = "fps\nframetime\nno_display=0\n";
        let hidden = with_block(user, false);
        assert_eq!(hidden, format!("{user}{MARK}\nno_display=1\n"));
        assert!(hides_hud(&hidden));
        assert_eq!(without_block(&hidden), user);
        assert_eq!(with_block("fps", false), format!("fps\n{MARK}\nno_display=1\n"), "a last line without its newline");
        assert_eq!(without_block(&format!("fps\n{MARK}\nno_display=1\ncpu_temp\n")), "fps\ncpu_temp\n", "the user wrote after it");
        assert_eq!(without_block(&format!("{MARK}\n")), "", "a mark alone");
    }

    #[test]
    fn hiding_and_showing_writes_the_same_file_and_lists_it() {
        let (_tmp, dir) = tempdir();
        let file = dir.join("MangoHud.conf");
        let record = dir.join("state/mangohud-hidden");
        std::fs::write(&file, "fps\n").unwrap();
        #[cfg(unix)]
        let inode = || std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&file).unwrap());
        #[cfg(unix)]
        let before = inode();
        let hud = WatchedHud { files: vec![file.clone()], shown: true };
        set_shown(&hud, false, &record).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), format!("fps\n{MARK}\nno_display=1\n"));
        assert_eq!(std::fs::read_to_string(&record).unwrap(), format!("{}\n", file.display()));
        set_shown(&hud, false, &record).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), format!("fps\n{MARK}\nno_display=1\n"), "one block");
        assert_eq!(std::fs::read_to_string(&record).unwrap().lines().count(), 1, "listed once");
        set_shown(&hud, true, &record).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "fps\n", "shown: the user's file as it was");
        #[cfg(unix)]
        assert_eq!(inode(), before, "MangoHud watches this inode: never replaced");
    }

    #[test]
    fn a_hud_the_user_hid_shows_with_our_block_and_hides_again_without_it() {
        let (_tmp, dir) = tempdir();
        let (file, record) = (dir.join("wine-Game.conf"), dir.join("rec"));
        std::fs::write(&file, "no_display\n").unwrap();
        let hud = WatchedHud { files: vec![file.clone()], shown: false };
        set_shown(&hud, true, &record).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), format!("no_display\n{MARK}\nno_display=0\n"));
        set_shown(&hud, false, &record).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "no_display\n");
    }

    #[test]
    fn a_block_left_behind_is_removed_at_the_next_start() {
        let (_tmp, dir) = tempdir();
        let (file, record) = (dir.join("MangoHud.conf"), dir.join("rec"));
        std::fs::write(&file, "fps\n").unwrap();
        set_shown(&WatchedHud { files: vec![file.clone()], shown: true }, false, &record).unwrap();
        // The app crashed; the user also changed the file meanwhile.
        let mut f = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
        f.write_all(b"cpu_temp\n").unwrap();
        restore_recorded(&record);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "fps\ncpu_temp\n");
        assert!(!record.exists());
        restore_recorded(&record);
    }

    #[test]
    fn a_game_that_ends_restores_its_files_only_listed_or_not() {
        let (_tmp, dir) = tempdir();
        let (a, b, c, record) = (dir.join("a.conf"), dir.join("b.conf"), dir.join("c.conf"), dir.join("rec"));
        for f in [&a, &b, &c] {
            std::fs::write(f, "fps\n").unwrap();
        }
        set_shown(&WatchedHud { files: vec![a.clone()], shown: true }, false, &record).unwrap();
        set_shown(&WatchedHud { files: vec![b.clone()], shown: true }, false, &record).unwrap();
        // A block the list lost.
        std::fs::write(&c, with_block("fps\n", false)).unwrap();
        restore_files(&[a.clone(), c.clone()], &record);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "fps\n");
        assert_eq!(std::fs::read_to_string(&c).unwrap(), "fps\n");
        assert!(hides_hud(&std::fs::read_to_string(&b).unwrap()), "the other game's stays");
        assert_eq!(std::fs::read_to_string(&record).unwrap(), format!("{}\n", b.display()));
    }

    #[cfg(unix)]
    #[test]
    fn links_are_never_written() {
        let (_tmp, dir) = tempdir();
        let real = dir.join("dotfiles.conf");
        std::fs::write(&real, "fps\n").unwrap();
        let link = dir.join("MangoHud.conf");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let hud = WatchedHud { files: vec![link.clone()], shown: true };
        assert!(set_shown(&hud, false, &dir.join("rec")).is_err());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "fps\n");
        // Listed by a crashed run, then turned into a link: left alone.
        std::fs::write(dir.join("rec2"), format!("{}\n", link.display())).unwrap();
        restore_recorded(&dir.join("rec2"));
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "fps\n");

        // A linked folder (GNU stow's dotfiles): the file looks plain.
        let repo = dir.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(repo.join("MangoHud.conf"), "fps\n").unwrap();
        std::os::unix::fs::symlink(&repo, dir.join("MangoHud")).unwrap();
        let through = dir.join("MangoHud/MangoHud.conf");
        assert!(set_shown(&WatchedHud { files: vec![through], shown: true }, false, &dir.join("rec3")).is_err());
        assert_eq!(std::fs::read_to_string(repo.join("MangoHud.conf")).unwrap(), "fps\n");

        // A hard-linked one.
        let hard = dir.join("hard.conf");
        std::fs::hard_link(&real, &hard).unwrap();
        assert!(set_shown(&WatchedHud { files: vec![hard], shown: true }, false, &dir.join("rec4")).is_err());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "fps\n");
    }

    #[test]
    fn a_mangohud_config_that_decides_the_hud_leaves_the_toggle_out() {
        assert!(file_counts(None));
        assert!(file_counts(Some("read_cfg,fps_limit=60")));
        assert!(!file_counts(Some("fps,frametime")), "the file is not read");
        assert!(!file_counts(Some("read_cfg,no_display=1")));
        assert!(!file_counts(Some("read_cfg,no_display")));
    }

    #[test]
    fn user_hidden_configs() {
        assert!(!hides_hud("fps\n"));
        assert!(hides_hud("no_display\n"));
        assert!(hides_hud("no_display = 1\n"));
        assert!(!hides_hud("no_display\nno_display=0\n"));
        assert!(!hides_hud("no_display=0 # shown\n"));
        assert!(!hides_hud("no_display=00\n"));
        assert!(hides_hud("no_display # hidden\n"));
        assert!(!hides_hud("# no_display\n"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reads_inotify_watches_from_fdinfo() {
        let info = "pos:\t0\nflags:\t02004000\nmnt_id:\t15\nino:\t1057\n\
                    inotify wd:1 ino:6a451 sdev:39 mask:402 ignored_mask:0 fhandle-bytes:c fhandle-type:1 f_handle:5fdc\n\
                    inotify wd:2 ino:10 sdev:800003 mask:2 ignored_mask:0\n";
        assert_eq!(parse_fdinfo(info), [Watch { ino: 0x6a451, sdev: 0x39, mask: 0x402 }, Watch { ino: 0x10, sdev: 0x800003, mask: 0x2 }]);
    }

    /// Our own watch names the file as a game's watch does: same inode and
    /// device even where `stat`'s device differs (btrfs).
    #[cfg(target_os = "linux")]
    #[test]
    fn kernel_id_matches_a_watch_of_the_same_file() {
        let (_tmp, dir) = tempdir();
        let file = dir.join("MangoHud.conf");
        std::fs::write(&file, "fps\n").unwrap();
        let id = kernel_id(&file).expect("a watch can be added");
        assert_eq!(id.0, std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&file).unwrap()));
        assert_eq!(kernel_id(&file), Some(id));
        assert_eq!(kernel_id(&dir.join("missing.conf")), None);
    }

    /// A Steam game running now (`HERONITE_TEST_STEAM_APP_ID`): what the
    /// overlay would find. Only reads.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs a Steam game running with MangoHud"]
    fn finds_a_running_steam_game() {
        let id = std::env::var("HERONITE_TEST_STEAM_APP_ID").expect("HERONITE_TEST_STEAM_APP_ID");
        let found = find(u32::MAX, Some(&id));
        eprintln!("found: {found:?}");
        assert!(found.is_some());
    }

    /// The real MangoHud (`--include-ignored`, with `vkcube` and a display):
    /// finds the file vkcube's MangoHud watches, hides the HUD and shows it
    /// again. `HERONITE_TEST_MANGOHUD_DIR` puts it on another filesystem
    /// (btrfs: the user's home).
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "starts vkcube with MangoHud"]
    fn finds_the_file_a_real_mangohud_watches() {
        let tmp = match std::env::var_os("HERONITE_TEST_MANGOHUD_DIR") {
            Some(d) => tempfile::tempdir_in(d).unwrap(),
            None => tempfile::tempdir().unwrap(),
        };
        let home = std::fs::canonicalize(tmp.path()).unwrap();
        let dir = home.join("MangoHud");
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("MangoHud.conf");
        std::fs::write(&conf, "fps\n").unwrap();
        let mut child = std::process::Command::new("vkcube")
            .env("MANGOHUD", "1")
            .env("XDG_CONFIG_HOME", &home)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_secs(3));
        let found = find(child.id(), None);
        let record = home.join("rec");
        if let Some(hud) = &found {
            set_shown(hud, false, &record).unwrap();
        }
        let after = find(child.id(), None);
        child.kill().ok();
        child.wait().ok();
        assert_eq!(found, Some(WatchedHud { files: vec![conf.clone()], shown: true }));
        assert_eq!(after.map(|h| h.shown), Some(false));
        restore_recorded(&record);
        assert_eq!(std::fs::read_to_string(&conf).unwrap(), "fps\n");
    }
}
