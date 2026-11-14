//! MangoHud's HUD in a running game whose MangoHud config Heronite did not
//! write (a Steam game with `MANGOHUD=1` or `mangohud %command%` in its
//! launch options, a game with a `MANGOHUD_CONFIG` of the user's own...),
//! shown and hidden without touching how the game was started.
//!
//! MangoHud watches (inotify) the one config file it picked when the game
//! started (`MANGOHUD_CONFIGFILE`, else the first of `<program>.conf`,
//! `wine-<exe>.conf`, `MangoHud.conf`...) and rereads it on every change. The
//! game's `/proc/<pid>/fdinfo` names that file's inode, so the file is found
//! from what the game really watches, never guessed. Hiding appends a block
//! of ours (`no_display=1`) to it; showing again removes it. The block is
//! also removed when the game ends, when the app quits and when it starts
//! again (after a crash), from a list of the files changed.
//!
//! Anything uncertain leaves the HUD as it is and the toggle out: no
//! MangoHud in the game, a `MANGOHUD_CONFIG` that wins over the file or
//! does not read it, a watched file that is a link or cannot be written.
//! Only `/proc` is read and only MangoHud's own config file is written: the
//! game and its launch stay as they are.

use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// The line our block starts with; the `no_display` line follows it.
const MARK: &str = "# Heronite: HUD shown or hidden by the in-game overlay (removed when the game ends)";

/// MangoHud's config files of a running game, as it watches them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchedHud {
    /// Usually one; a launcher and its game may each watch their own.
    pub files: Vec<PathBuf>,
    /// The HUD shows now.
    pub shown: bool,
}

/// The config files MangoHud watches in the game whose process (group
/// leader) is `game_pid`; `None` when the HUD cannot be shown or hidden this
/// way. Blocking (`/proc`).
#[cfg(target_os = "linux")]
pub fn find(game_pid: u32) -> Option<WatchedHud> {
    let mut files: Vec<PathBuf> = Vec::new();
    for pid in crate::procs::game_processes(game_pid) {
        if !loads_mangohud(pid) {
            continue;
        }
        let env = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
        let var = |key: &str| crate::procs::environ_value(&env, key);
        // Applied over the file on every reload, or the file is not read.
        if !file_counts(var("MANGOHUD_CONFIG").as_deref()) {
            return None;
        }
        let watched = inotify_watches(pid);
        if watched.is_empty() {
            continue;
        }
        let found = candidates(&var).into_iter().find(|f| is_watched(f, &watched));
        // MangoHud runs and watches a file we cannot place: leave it.
        let file = found?;
        if !files.contains(&file) {
            files.push(file);
        }
    }
    if files.is_empty() {
        return None;
    }
    let shown = files.iter().all(|f| std::fs::read_to_string(f).is_ok_and(|t| !hides_hud(&t)));
    Some(WatchedHud { files, shown })
}

#[cfg(not(target_os = "linux"))]
pub fn find(_game_pid: u32) -> Option<WatchedHud> {
    None
}

/// Shows or hides the HUD: writes or removes our block in each file
/// (blocking). `record` first lists the files, so a block left by a crash is
/// removed at the next start ([`restore_recorded`]).
pub fn set_shown(hud: &WatchedHud, shown: bool, record: &Path) -> Result<(), String> {
    remember(record, &hud.files).map_err(|e| format!("{}: {e}", record.display()))?;
    for file in &hud.files {
        let text = read_plain(file)?;
        let base = without_block(&text);
        let new = if hides_hud(&base) == !shown { base } else { with_block(&base, shown) };
        write_in_place(file, &text, &new).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    Ok(())
}

/// Removes our block from the files `record` lists, then the list itself
/// (at start, after a crash, and when the app quits). Blocking.
pub fn restore_recorded(record: &Path) {
    restore_listed(record, None);
}

/// Removes our block from `files` (their game ended) and takes them off
/// `record`. Blocking.
pub fn restore_files(files: &[PathBuf], record: &Path) {
    restore_listed(record, Some(files));
}

/// Files that changed into something else (a link) are left alone; gone
/// ones are dropped from the list.
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
            // Kept to try again next time, unless it cannot come back.
            if file.exists() {
                kept.push(line.to_string());
            }
        }
    }
    let result = match kept.is_empty() {
        true => std::fs::remove_file(record),
        false => std::fs::write(record, kept.join("\n") + "\n"),
    };
    if let Err(e) = result {
        log::warn!("MangoHud: {}: {e}", record.display());
    }
}

/// Removes our block from `file` (nothing when it has none).
fn restore(file: &Path) -> Result<(), String> {
    let text = match read_plain(file) {
        Ok(t) => t,
        Err(_) if !file.exists() => return Ok(()),
        Err(e) => return Err(e),
    };
    let base = without_block(&text);
    write_in_place(file, &text, &base).map_err(|e| e.to_string())
}

fn remember(record: &Path, files: &[PathBuf]) -> std::io::Result<()> {
    let list = std::fs::read_to_string(record).unwrap_or_default();
    let missing: Vec<String> = files.iter().map(|f| f.to_string_lossy().into_owned()).filter(|f| !list.lines().any(|l| l == f)).collect();
    if missing.is_empty() {
        return Ok(());
    }
    if let Some(dir) = record.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut out = std::fs::OpenOptions::new().create(true).append(true).open(record)?;
    out.write_all((missing.join("\n") + "\n").as_bytes())?;
    out.sync_all()
}

/// The text of a regular file (never through a link: it could be someone's
/// dotfiles repository).
fn read_plain(file: &Path) -> Result<String, String> {
    let meta = std::fs::symlink_metadata(file).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() {
        return Err("not a regular file".into());
    }
    std::fs::read_to_string(file).map_err(|e| e.to_string())
}

/// Writes `new` over `old` in the same file (MangoHud watches its inode, so
/// it is never replaced), from the first byte that differs, then cuts what
/// is left. The file never reads empty in between: a block at the end comes
/// and goes without the rest being written again.
fn write_in_place(file: &Path, old: &str, new: &str) -> std::io::Result<()> {
    if old == new {
        return Ok(());
    }
    let same = old.bytes().zip(new.bytes()).take_while(|(a, b)| a == b).count();
    let mut f = std::fs::OpenOptions::new().write(true).open(file)?;
    if same < new.len() {
        f.seek(SeekFrom::Start(same as u64))?;
        f.write_all(&new.as_bytes()[same..])?;
    }
    if new.len() < old.len() {
        f.set_len(new.len() as u64)?;
    }
    f.flush()
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
/// or not `0`.
fn hides_hud(config: &str) -> bool {
    config
        .lines()
        .rev()
        .find_map(|l| {
            let (key, value) = l.split_once('=').map_or((l, None), |(k, v)| (k, Some(v)));
            (key.trim() == "no_display").then(|| value.is_none_or(|v| v.trim() != "0"))
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
    let config_home = |xdg: Option<String>, home: Option<String>| {
        xdg.filter(|d| !d.is_empty()).map(PathBuf::from).or_else(|| home.filter(|h| !h.is_empty()).map(|h| Path::new(&h).join(".config")))
    };
    let mut dirs: Vec<PathBuf> = Vec::new();
    for dir in
        [config_home(var("XDG_CONFIG_HOME"), var("HOME")), config_home(std::env::var("XDG_CONFIG_HOME").ok(), std::env::var("HOME").ok())]
            .into_iter()
            .flatten()
    {
        let dir = dir.join("MangoHud");
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    let mut files = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        files.extend(entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "conf")));
    }
    files
}

/// Whether `file` is one of `watched` (inode and device), as a regular file
/// this user can write.
#[cfg(target_os = "linux")]
fn is_watched(file: &Path, watched: &[(u64, u64)]) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(meta) = std::fs::symlink_metadata(file) else { return false };
    meta.file_type().is_file()
        && watched.iter().any(|(ino, dev)| meta.ino() == *ino && meta.dev() == *dev)
        // Opening for writing changes nothing (MangoHud waits for a modification).
        && std::fs::OpenOptions::new().append(true).open(file).is_ok()
}

/// Whether MangoHud's library is loaded in the process.
#[cfg(target_os = "linux")]
fn loads_mangohud(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/maps")).is_ok_and(|m| m.contains("libMangoHud"))
}

/// `(inode, device)` of every file the process watches with inotify, the
/// device as `stat` gives it.
#[cfg(target_os = "linux")]
fn inotify_watches(pid: u32) -> Vec<(u64, u64)> {
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

/// The `inotify ... ino:<hex> sdev:<hex>` lines of an fdinfo. `sdev` is the
/// kernel's device number (`major << 20 | minor`).
#[cfg(target_os = "linux")]
fn parse_fdinfo(info: &str) -> Vec<(u64, u64)> {
    info.lines()
        .filter_map(|l| {
            let l = l.strip_prefix("inotify")?;
            let field = |name: &str| l.split_whitespace().find_map(|w| w.strip_prefix(name)).and_then(|v| u64::from_str_radix(v, 16).ok());
            let (ino, sdev) = (field("ino:")?, field("sdev:")?);
            let (major, minor) = ((sdev >> 20) as libc::c_uint, (sdev & 0xfffff) as libc::c_uint);
            Some((ino, libc::makedev(major, minor)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("MangoHud.conf");
        let record = tmp.path().join("state/mangohud-hidden");
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
        assert_eq!(std::fs::read_to_string(&record).unwrap().lines().count(), 1, "listed once");
        set_shown(&hud, true, &record).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "fps\n", "shown: the user's file as it was");
        #[cfg(unix)]
        assert_eq!(inode(), before, "MangoHud watches this inode: never replaced");
    }

    #[test]
    fn a_hud_the_user_hid_shows_with_our_block_and_hides_again_without_it() {
        let tmp = tempfile::tempdir().unwrap();
        let (file, record) = (tmp.path().join("wine-Game.conf"), tmp.path().join("rec"));
        std::fs::write(&file, "no_display\n").unwrap();
        let hud = WatchedHud { files: vec![file.clone()], shown: false };
        set_shown(&hud, true, &record).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), format!("no_display\n{MARK}\nno_display=0\n"));
        set_shown(&hud, false, &record).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "no_display\n");
    }

    #[test]
    fn a_block_left_behind_is_removed_at_the_next_start() {
        let tmp = tempfile::tempdir().unwrap();
        let (file, record) = (tmp.path().join("MangoHud.conf"), tmp.path().join("rec"));
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
    fn a_game_that_ends_restores_its_files_only() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b, record) = (tmp.path().join("a.conf"), tmp.path().join("b.conf"), tmp.path().join("rec"));
        std::fs::write(&a, "fps\n").unwrap();
        std::fs::write(&b, "fps\n").unwrap();
        set_shown(&WatchedHud { files: vec![a.clone()], shown: true }, false, &record).unwrap();
        set_shown(&WatchedHud { files: vec![b.clone()], shown: true }, false, &record).unwrap();
        restore_files(std::slice::from_ref(&a), &record);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "fps\n");
        assert!(hides_hud(&std::fs::read_to_string(&b).unwrap()), "the other game's stays");
        assert_eq!(std::fs::read_to_string(&record).unwrap(), format!("{}\n", b.display()));
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_never_written() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("dotfiles.conf");
        std::fs::write(&real, "fps\n").unwrap();
        let link = tmp.path().join("MangoHud.conf");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let hud = WatchedHud { files: vec![link.clone()], shown: true };
        assert!(set_shown(&hud, false, &tmp.path().join("rec")).is_err());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "fps\n");
        // Listed by a crashed run, then turned into a link: left alone.
        std::fs::write(tmp.path().join("rec2"), format!("{}\n", link.display())).unwrap();
        restore_recorded(&tmp.path().join("rec2"));
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
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reads_inotify_watches_from_fdinfo() {
        let info = "pos:\t0\nflags:\t02004000\nmnt_id:\t15\nino:\t1057\n\
                    inotify wd:1 ino:6a451 sdev:39 mask:402 ignored_mask:0 fhandle-bytes:c fhandle-type:1 f_handle:5fdc\n\
                    inotify wd:2 ino:10 sdev:800003 mask:2 ignored_mask:0\n";
        assert_eq!(parse_fdinfo(info), [(0x6a451, libc::makedev(0, 0x39)), (0x10, libc::makedev(8, 3))]);
    }

    /// The real MangoHud: `HERONITE_TEST_MANGOHUD=1`, with `vkcube` and a
    /// display. Finds the file vkcube's MangoHud watches, hides the HUD and
    /// shows it again.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "starts vkcube with MangoHud"]
    fn finds_the_file_a_real_mangohud_watches() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("MangoHud");
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("MangoHud.conf");
        std::fs::write(&conf, "fps\n").unwrap();
        let mut child = std::process::Command::new("vkcube")
            .env("MANGOHUD", "1")
            .env("XDG_CONFIG_HOME", tmp.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_secs(3));
        let found = find(child.id());
        let record = tmp.path().join("rec");
        if let Some(hud) = &found {
            set_shown(hud, false, &record).unwrap();
        }
        let after = find(child.id());
        child.kill().ok();
        child.wait().ok();
        assert_eq!(found, Some(WatchedHud { files: vec![conf.clone()], shown: true }));
        assert_eq!(after.map(|h| h.shown), Some(false));
        restore_recorded(&record);
        assert_eq!(std::fs::read_to_string(&conf).unwrap(), "fps\n");
    }
}
