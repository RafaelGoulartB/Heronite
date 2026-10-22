//! Mounting the enabled mods over the game's folder with `fuse-overlayfs`,
//! and taking the mount away again (`docs/MODS.md`, "Mounting").

use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use heronite_data::temps::write_atomic;
use heronite_i18n::tr;

use crate::fsutil::remove_any;
use crate::paths::{GameDirs, ModsRoot};
use crate::tool;

const MOUNT_WAIT: Duration = Duration::from_secs(5);
const BUSY_RETRY: Duration = Duration::from_millis(250);
const BUSY_FOR: Duration = Duration::from_secs(5);
/// The fstype of a mount made by the tool is `fuse.<name of the binary>`.
const FSTYPE_PREFIX: &str = "fuse.fuse-overlayfs";

/// A live overlay over a game folder. Dropping it unmounts.
#[derive(Debug)]
pub struct Mount {
    game_dir: PathBuf,
    mods_root: PathBuf,
    fusermount: PathBuf,
    done: bool,
}

impl Mount {
    pub fn game_dir(&self) -> &Path {
        &self.game_dir
    }

    /// Unmounts now; whether the folder is free of our mount afterwards.
    pub fn unmount(mut self) -> bool {
        self.release()
    }

    fn release(&mut self) -> bool {
        if self.done {
            return true;
        }
        self.done = true;
        let gone = unmount_dir(&self.fusermount, &self.game_dir);
        if gone {
            record_remove(&self.mods_root, &self.game_dir);
        }
        gone
    }
}

impl Drop for Mount {
    fn drop(&mut self) {
        self.release();
    }
}

/// Mounts `layers` (mod folders, lowest priority first) over `game_dir`; what
/// the game writes lands in `dirs.overwrite()`. The game folder itself is the
/// bottom layer: `fuse-overlayfs` opens its layers before it mounts, so they
/// stay reachable and the folder is never written.
pub fn mount(overlay_bin: &Path, dirs: &GameDirs, game_dir: &Path, layers: &[PathBuf]) -> Result<Mount, String> {
    let fusermount =
        tool::fusermount().ok_or_else(|| tr!("Heronite needs FUSE (fusermount3) to apply mods. Install the fuse3 package.").to_string())?;
    let mods_root = dirs.root().parent().map(Path::to_path_buf).unwrap_or_else(|| dirs.root().to_path_buf());

    // A mount left by a crash makes the folder unreadable until it is removed.
    if is_overlay_at(game_dir) {
        log::warn!("mods: {} is still mounted from before; unmounting it", game_dir.display());
        if !unmount_dir(&fusermount, game_dir) {
            return Err(tr!("{path} is still mounted from an earlier run and could not be unmounted", path = game_dir.display()));
        }
        record_remove(&mods_root, game_dir);
    }
    let game = game_dir.canonicalize().map_err(|_| tr!("Cannot use {path}: it is not a folder", path = game_dir.display()))?;
    if !game.is_dir() {
        return Err(tr!("Cannot use {path}: it is not a folder", path = game_dir.display()));
    }
    fs::create_dir_all(dirs.root()).map_err(|e| format!("{}: {e}", dirs.root().display()))?;
    check_places(&game, &dirs.root().canonicalize().map_err(|e| e.to_string())?)?;

    let layer_dir = prepare_layers(dirs, &game, layers)?;
    let log_file = dirs.root().join(".mount.log");
    let stderr = fs::File::create(&log_file).map(Stdio::from).unwrap_or_else(|_| Stdio::null());
    let options = options(layers.len());
    log::info!("mods: mounting {} layers over {}", layers.len(), game.display());
    let status = Command::new(overlay_bin)
        .current_dir(&layer_dir)
        .arg("-o")
        .arg(&options)
        .arg(&game)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr)
        .status()
        .map_err(|e| tr!("Could not start {program}: {error}", program = overlay_bin.display(), error = e))?;
    let said = fs::read_to_string(&log_file).unwrap_or_default();
    let _ = fs::remove_file(&log_file);
    if !status.success() {
        return Err(tr!("fuse-overlayfs failed: {error}", error = first_lines(&said, status.code())));
    }
    if !wait_until(|| is_overlay_at(&game)) {
        let _ = unmount_dir(&fusermount, &game);
        return Err(tr!("fuse-overlayfs did not mount {path}: {error}", path = game.display(), error = first_lines(&said, None)));
    }
    record_add(&mods_root, &game);
    Ok(Mount { game_dir: game, mods_root, fusermount, done: false })
}

/// `lowerdir` lists the layers by the short names of `.layers` (highest
/// priority first, the game last), so no path needs escaping.
fn options(count: usize) -> String {
    let lower: Vec<String> = (0..count).rev().map(|i| format!("{i:03}")).chain(std::iter::once("game".to_string())).collect();
    // SAFETY: getuid and getgid cannot fail.
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    format!("lowerdir={},upperdir=upper,workdir=work,squash_to_uid={uid},squash_to_gid={gid}", lower.join(":"))
}

/// Rebuilds `.layers` with a link for each layer, the game folder, the upper
/// and the work dir. Returns the folder to run the tool from.
fn prepare_layers(dirs: &GameDirs, game: &Path, layers: &[PathBuf]) -> Result<PathBuf, String> {
    let dir = dirs.layers();
    let err = |e: std::io::Error| format!("{}: {e}", dir.display());
    remove_any(&dir).map_err(err)?;
    fs::create_dir_all(&dir).map_err(err)?;
    for p in [dirs.overwrite(), dirs.work()] {
        fs::create_dir_all(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    let link = |target: &Path, name: &str| std::os::unix::fs::symlink(target, dir.join(name)).map_err(err);
    for (i, layer) in layers.iter().enumerate() {
        link(layer, &format!("{i:03}"))?;
    }
    link(game, "game")?;
    link(&dirs.overwrite(), "upper")?;
    link(&dirs.work(), "work")?;
    Ok(dir)
}

/// Refuses folders the overlay must never cover.
fn check_places(game: &Path, mods_dir: &Path) -> Result<(), String> {
    let home = std::env::var_os("HOME").and_then(|h| PathBuf::from(h).canonicalize().ok());
    if game == Path::new("/") || home.as_deref() == Some(game) {
        return Err(tr!("Mods cannot be applied over {path}", path = game.display()));
    }
    if mods_dir.starts_with(game) || game.starts_with(mods_dir) {
        return Err(tr!("The mods folder and the game folder cannot be inside each other").to_string());
    }
    Ok(())
}

fn first_lines(text: &str, code: Option<i32>) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).take(3).collect();
    match (lines.is_empty(), code) {
        (false, _) => lines.join(" "),
        (true, Some(c)) => format!("exit code {c}"),
        (true, None) => "no answer".to_string(),
    }
}

fn wait_until(mut done: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if done() {
            return true;
        }
        if start.elapsed() > MOUNT_WAIT {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// `fusermount -u`, again every 250 ms while the folder is busy (a game's
/// last processes may still be leaving), then a lazy `-uz`. Whether the
/// folder is free of an overlay afterwards.
fn unmount_dir(fusermount: &Path, dir: &Path) -> bool {
    let run = |flag: &str| {
        Command::new(fusermount)
            .arg(flag)
            .arg(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map(|o| (o.status.success(), String::from_utf8_lossy(&o.stderr).trim().to_string()))
            .unwrap_or((false, "could not run fusermount".into()))
    };
    let start = Instant::now();
    loop {
        let (ok, said) = run("-u");
        if ok || !is_overlay_at(dir) {
            log::info!("mods: unmounted {}", dir.display());
            return true;
        }
        if start.elapsed() >= BUSY_FOR {
            log::warn!("mods: {} is busy ({said}); unmounting lazily", dir.display());
            break;
        }
        std::thread::sleep(BUSY_RETRY);
    }
    let (ok, said) = run("-uz");
    if !ok {
        log::warn!("mods: could not unmount {}: {said}", dir.display());
    }
    !is_overlay_at(dir)
}

/// A mount point and its file system type, from `/proc/self/mountinfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MountInfo {
    point: PathBuf,
    fstype: String,
}

fn parse_mountinfo(text: &str) -> Vec<MountInfo> {
    text.lines()
        .filter_map(|line| {
            let (before, after) = line.split_once(" - ")?;
            let point = before.split(' ').nth(4)?;
            let fstype = after.split(' ').next()?;
            Some(MountInfo { point: PathBuf::from(unescape(point)), fstype: fstype.to_string() })
        })
        .collect()
}

/// The kernel writes space, tab, newline and backslash as `\040`, `\011`,
/// `\012` and `\134`.
fn unescape(s: &str) -> OsString {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let octal = (b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)))
            .then(|| u32::from(b[i + 1] - b'0') * 64 + u32::from(b[i + 2] - b'0') * 8 + u32::from(b[i + 3] - b'0'))
            .and_then(|v| u8::try_from(v).ok());
        match octal {
            Some(v) => {
                out.push(v);
                i += 4;
            }
            None => {
                out.push(b[i]);
                i += 1;
            }
        }
    }
    OsString::from_vec(out)
}

fn mounts() -> Vec<MountInfo> {
    fs::read_to_string("/proc/self/mountinfo").map(|t| parse_mountinfo(&t)).unwrap_or_default()
}

/// Whether something is mounted at `dir` (any file system).
pub fn is_mounted(dir: &Path) -> bool {
    let canonical = dir.canonicalize().ok();
    mounts().iter().any(|m| m.point == dir || canonical.as_deref() == Some(m.point.as_path()))
}

/// Whether a `fuse-overlayfs` is mounted at `dir`.
fn is_overlay_at(dir: &Path) -> bool {
    // A dead mount makes `canonicalize` fail; the plain path still matches.
    let canonical = dir.canonicalize().ok();
    mounts().iter().any(|m| m.fstype.starts_with(FSTYPE_PREFIX) && (m.point == dir || canonical.as_deref() == Some(m.point.as_path())))
}

/// Every `fuse-overlayfs` mount point of the session.
pub fn overlay_mounts() -> Vec<PathBuf> {
    mounts().into_iter().filter(|m| m.fstype.starts_with(FSTYPE_PREFIX)).map(|m| m.point).collect()
}

/// The folders we mounted, kept in `<mods root>/.mounted.json`: the mount
/// table does not say which layers a mount has, so this is how a mount left
/// by a crash is known to be ours.
static RECORDS: Mutex<()> = Mutex::new(());

fn records_file(mods_root: &Path) -> PathBuf {
    mods_root.join(".mounted.json")
}

fn read_records(mods_root: &Path) -> Vec<PathBuf> {
    fs::read(records_file(mods_root)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn write_records(mods_root: &Path, records: &[PathBuf]) {
    let file = records_file(mods_root);
    let result = if records.is_empty() {
        remove_any(&file)
    } else {
        write_atomic(&file, &serde_json::to_vec_pretty(records).unwrap_or_default(), Some(0o644))
    };
    if let Err(e) = result {
        log::warn!("mods: could not update {}: {e}", file.display());
    }
}

fn record_add(mods_root: &Path, dir: &Path) {
    let _lock = RECORDS.lock().unwrap_or_else(|e| e.into_inner());
    let mut records = read_records(mods_root);
    if !records.iter().any(|r| r == dir) {
        records.push(dir.to_path_buf());
        write_records(mods_root, &records);
    }
}

fn record_remove(mods_root: &Path, dir: &Path) {
    let _lock = RECORDS.lock().unwrap_or_else(|e| e.into_inner());
    let mut records = read_records(mods_root);
    let before = records.len();
    records.retain(|r| r != dir);
    if records.len() != before {
        write_records(mods_root, &records);
    }
}

/// Unmounts the overlays we left mounted (a crash, a kill) except the game
/// folders in `keep` (games that are running now). Call it at start.
pub fn cleanup_stale(root: &ModsRoot, keep: &[PathBuf]) {
    let Some(fusermount) = tool::fusermount() else { return };
    let records = {
        let _lock = RECORDS.lock().unwrap_or_else(|e| e.into_inner());
        read_records(root.path())
    };
    for dir in records.iter().filter(|d| !keep.contains(d)) {
        if !is_overlay_at(dir) || unmount_dir(&fusermount, dir) {
            record_remove(root.path(), dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::ModsRoot;

    #[test]
    fn mountinfo_is_parsed_with_escapes() {
        let text = "\
41 27 0:38 / /sys/fs/fuse/connections rw,nosuid - fusectl fusectl rw
2159 59 0:209 / /tmp/My\\040Games/a\\134b rw,nosuid,nodev,noatime shared:751 - fuse.fuse-overlayfs-x86_64 fuse-overlayfs-x86_64 rw,user_id=1000
2160 59 0:210 / /mnt rw - ext4 /dev/sda1 rw
garbage";
        let m = parse_mountinfo(text);
        assert_eq!(m.len(), 3);
        assert_eq!(m[1], MountInfo { point: PathBuf::from("/tmp/My Games/a\\b"), fstype: "fuse.fuse-overlayfs-x86_64".into() });
        assert!(m[1].fstype.starts_with(FSTYPE_PREFIX) && !m[2].fstype.starts_with(FSTYPE_PREFIX));
    }

    #[test]
    fn lowerdir_has_the_highest_priority_first_and_the_game_last() {
        let o = options(3);
        assert!(o.starts_with("lowerdir=002:001:000:game,upperdir=upper,workdir=work,squash_to_uid="), "{o}");
        assert!(options(0).starts_with("lowerdir=game,"));
    }

    #[test]
    fn dangerous_places_are_refused() {
        let t = tempfile::tempdir().unwrap();
        let mods = t.path().join("mods").join("g");
        fs::create_dir_all(&mods).unwrap();
        assert!(check_places(Path::new("/"), &mods).is_err());
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/nonexistent-home".into()));
        if let Ok(h) = home.canonicalize() {
            assert!(check_places(&h, &mods).is_err());
        }
        assert!(check_places(t.path(), &mods).is_err(), "the mods are inside the game folder");
        assert!(check_places(&mods.join("mods").join("x"), &mods).is_err(), "the game folder is inside the mods");
        let game = tempfile::tempdir().unwrap();
        assert!(check_places(game.path(), &mods).is_ok());
    }

    #[test]
    fn layers_are_short_links() {
        let t = tempfile::tempdir().unwrap();
        let dirs = ModsRoot::at(t.path().join("m")).game("g");
        let game = t.path().join("game");
        let (a, b) = (t.path().join("a, b:c"), t.path().join("b"));
        for d in [&game, &a, &b] {
            fs::create_dir_all(d).unwrap();
        }
        fs::create_dir_all(dirs.layers()).unwrap();
        fs::write(dirs.layers().join("stale"), b"x").unwrap();
        let dir = prepare_layers(&dirs, &game, &[a.clone(), b.clone()]).unwrap();
        assert_eq!(fs::read_link(dir.join("000")).unwrap(), a);
        assert_eq!(fs::read_link(dir.join("001")).unwrap(), b);
        assert_eq!(fs::read_link(dir.join("game")).unwrap(), game);
        assert!(dir.join("upper").is_dir() && dir.join("work").is_dir() && !dir.join("stale").exists());
    }

    #[test]
    fn records_come_and_go() {
        let t = tempfile::tempdir().unwrap();
        record_add(t.path(), Path::new("/g/a"));
        record_add(t.path(), Path::new("/g/a"));
        record_add(t.path(), Path::new("/g/b"));
        assert_eq!(read_records(t.path()), [PathBuf::from("/g/a"), PathBuf::from("/g/b")]);
        record_remove(t.path(), Path::new("/g/a"));
        record_remove(t.path(), Path::new("/g/b"));
        assert!(!records_file(t.path()).exists());
    }

    /// The tool and `/dev/fuse` are needed; the test is skipped without them
    /// (`HERONITE_FUSE_OVERLAYFS=/path/to/fuse-overlayfs cargo test`).
    fn overlay() -> Option<PathBuf> {
        if !Path::new("/dev/fuse").exists() || tool::fusermount().is_none() {
            return None;
        }
        let bin = tool::find(Path::new("/nonexistent"))?;
        println!("using {}", bin.display());
        Some(bin)
    }

    fn put(path: PathBuf, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn mounts_layers_over_the_game_and_leaves_it_alone() {
        let Some(bin) = overlay() else { return };
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path().join("mods"));
        let dirs = root.game("g");
        let game = t.path().join("game");
        put(game.join("a.txt"), "game");
        put(game.join("b.txt"), "game");
        put(game.join("Data/old.txt"), "game");
        put(dirs.mod_dir("a").join("a.txt"), "A");
        put(dirs.mod_dir("a").join("c.txt"), "A");
        put(dirs.mod_dir("b").join("a.txt"), "B");

        let m = mount(&bin, &dirs, &game, &[dirs.mod_dir("a"), dirs.mod_dir("b")]).unwrap();
        assert!(is_mounted(&game));
        assert_eq!(overlay_mounts().iter().filter(|p| p.as_path() == m.game_dir()).count(), 1);
        let read = |n: &str| fs::read_to_string(game.join(n)).unwrap();
        assert_eq!((read("a.txt").as_str(), read("b.txt").as_str(), read("c.txt").as_str()), ("B", "game", "A"));
        assert_eq!(read("Data/old.txt"), "game");

        put(game.join("new.txt"), "written");
        fs::write(game.join("b.txt"), "changed").unwrap();
        fs::remove_file(game.join("Data/old.txt")).unwrap();
        assert!(m.unmount());
        assert!(!is_mounted(&game));
        assert_eq!(read("a.txt"), "game");
        assert_eq!(read("b.txt"), "game");
        assert_eq!(read("Data/old.txt"), "game");
        assert!(!game.join("new.txt").exists());
        assert_eq!(fs::read_to_string(dirs.overwrite().join("new.txt")).unwrap(), "written");
        assert_eq!(fs::read_to_string(dirs.overwrite().join("b.txt")).unwrap(), "changed");
        assert!(read_records(root.path()).is_empty());
    }

    #[test]
    fn dropping_unmounts_and_a_stale_mount_is_cleaned() {
        let Some(bin) = overlay() else { return };
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path().join("mods"));
        let dirs = root.game("g");
        let game = t.path().join("game");
        put(game.join("a.txt"), "game");
        put(dirs.mod_dir("a").join("a.txt"), "A");
        {
            let _m = mount(&bin, &dirs, &game, &[dirs.mod_dir("a")]).unwrap();
            assert_eq!(fs::read_to_string(game.join("a.txt")).unwrap(), "A");
        }
        assert!(!is_mounted(&game));

        // A crash leaves the mount and its record; the next mount takes it away.
        let m = mount(&bin, &dirs, &game, &[dirs.mod_dir("a")]).unwrap();
        std::mem::forget(m);
        assert!(is_mounted(&game));
        cleanup_stale(&root, &[game.canonicalize().unwrap()]);
        assert!(is_mounted(&game), "a game that is running is kept");
        cleanup_stale(&root, &[]);
        assert!(!is_mounted(&game));
        assert!(read_records(root.path()).is_empty());

        let m = mount(&bin, &dirs, &game, &[dirs.mod_dir("a")]).unwrap();
        std::mem::forget(m);
        let again = mount(&bin, &dirs, &game, &[dirs.mod_dir("a")]).unwrap();
        assert_eq!(fs::read_to_string(game.join("a.txt")).unwrap(), "A");
        assert!(again.unmount());
        assert_eq!(fs::read_to_string(game.join("a.txt")).unwrap(), "game");
    }
}
