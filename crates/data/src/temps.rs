//! Atomic file writes with unique temp names, and the sweep of the temp files
//! a killed process leaves behind.
//!
//! Every temp file this app creates is named
//! `.<target name>.heronite-rs-<pid>-<counter>.tmp` next to its target
//! (older versions used `.<name>.heronite-rs-<pid>.tmp`, `shortcuts.vdf.tmp`
//! and `<x>.desktop.tmp`). The sweep only ever removes files matching those
//! exact shapes.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// Marker between the target name and the pid.
const MARK: &str = ".heronite-rs-";
/// Leftovers older than this are removed even if their pid is alive (reused).
pub const STALE_AFTER: Duration = Duration::from_secs(3600);

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// `.<name>.heronite-rs-<pid>-<n>.tmp` next to `path`; unique per call, so two
/// writers of the same file in one process never share a temp file.
pub fn unique_tmp_path(path: &Path) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(".{name}{MARK}{}-{n}.tmp", std::process::id()))
}

/// Writes `bytes` to a unique temp file next to `path`, fsyncs it and renames
/// it over `path`. The temp file is removed if anything fails. `mode` sets the
/// Unix permissions (the target keeps the temp file's, not the old one's).
/// The temp file is made new, never written through a link left in its place
/// (a Wine prefix is writable by the programs in the sandbox).
pub fn write_atomic(path: &Path, bytes: &[u8], mode: Option<u32>) -> io::Result<()> {
    let tmp = unique_tmp_path(path);
    let _ = fs::remove_file(&tmp);
    let result = (|| {
        let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(bytes)?;
        #[cfg(unix)]
        if let Some(mode) = mode {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(fs::Permissions::from_mode(mode))?;
        }
        #[cfg(not(unix))]
        let _ = mode;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// The pid in a temp name of our shape, if `name` has it.
pub fn tmp_owner_pid(name: &str) -> Option<i32> {
    let rest = name.strip_prefix('.')?.strip_suffix(".tmp")?;
    let at = rest.rfind(MARK)?;
    if at == 0 {
        return None;
    }
    let tail = &rest[at + MARK.len()..];
    let (pid, counter) = match tail.split_once('-') {
        Some((p, c)) => (p, Some(c)),
        None => (tail, None),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(pid) || counter.is_some_and(|c| !digits(c)) {
        return None;
    }
    pid.parse().ok()
}

#[cfg(unix)]
fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // SAFETY: signal 0 only checks that the process exists.
    let r = unsafe { libc::kill(pid, 0) };
    r == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn pid_alive(_pid: i32) -> bool {
    true
}

fn age(path: &Path, now: SystemTime) -> Option<Duration> {
    let modified = fs::symlink_metadata(path).and_then(|m| m.modified()).ok()?;
    Some(now.duration_since(modified).unwrap_or_default())
}

fn remove_file(path: &Path, removed: &mut Vec<PathBuf>) {
    if fs::remove_file(path).is_ok() {
        log::info!("removed stale temp file {}", path.display());
        removed.push(path.to_path_buf());
    }
}

/// Removes our own leftover temp files directly inside `dir` (not recursive):
/// `.<x>.heronite-rs-<pid>[-<n>].tmp` whose pid is not alive or that are older
/// than [`STALE_AFTER`]. Anything else is left alone. Returns what was removed.
pub fn sweep_dir(dir: &Path) -> Vec<PathBuf> {
    sweep_dir_at(dir, SystemTime::now())
}

pub(crate) fn sweep_dir_at(dir: &Path, now: SystemTime) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else { return removed };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(pid) = tmp_owner_pid(&name) else { continue };
        let path = entry.path();
        if !fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
            continue;
        }
        let old = age(&path, now).is_some_and(|a| a > STALE_AFTER);
        if !pid_alive(pid) || old {
            remove_file(&path, &mut removed);
        }
    }
    removed
}

/// Removes `path` (a plain file) when it is older than [`STALE_AFTER`]: for the
/// old fixed temp names such as `shortcuts.vdf.tmp`, whose owner is unknown.
pub fn sweep_legacy_file(path: &Path, removed: &mut Vec<PathBuf>) {
    sweep_legacy_file_at(path, SystemTime::now(), removed)
}

pub(crate) fn sweep_legacy_file_at(path: &Path, now: SystemTime, removed: &mut Vec<PathBuf>) {
    if fs::symlink_metadata(path).is_ok_and(|m| m.is_file()) && age(path, now).is_some_and(|a| a > STALE_AFTER) {
        remove_file(path, removed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(p: &Path) {
        fs::write(p, "x").unwrap();
    }

    #[test]
    fn temp_names_are_unique_and_parse_back() {
        let a = unique_tmp_path(Path::new("/d/config.json"));
        let b = unique_tmp_path(Path::new("/d/config.json"));
        assert_ne!(a, b);
        let name = a.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with(".config.json.heronite-rs-"), "{name}");
        assert_eq!(tmp_owner_pid(&name), Some(std::process::id() as i32));
        assert_eq!(tmp_owner_pid(".x.json.heronite-rs-123.tmp"), Some(123));
        assert_eq!(tmp_owner_pid(".x.json.heronite-rs-123-4.tmp"), Some(123));
    }

    #[test]
    fn only_exact_shapes_parse() {
        for bad in [
            "x.heronite-rs-1.tmp",
            ".heronite-rs-1.tmp",
            ".x.heronite-rs-.tmp",
            ".x.heronite-rs-1x.tmp",
            ".x.heronite-rs-1-.tmp",
            ".x.heronite-rs-1-a.tmp",
            ".x.heronite-rs-1.temp",
            ".x.heronite-rs-1-2-3.tmp",
            "config.json",
            ".config.json.bak",
        ] {
            assert_eq!(tmp_owner_pid(bad), None, "{bad}");
        }
    }

    #[test]
    fn write_atomic_replaces_and_leaves_no_temp() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        write_atomic(&p, b"one", Some(0o755)).unwrap();
        write_atomic(&p, b"two", Some(0o755)).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"two");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o755);
        }
        // A target that cannot be replaced (a non-empty directory) leaves no temp file behind.
        let blocked = dir.path().join("blocked");
        fs::create_dir_all(blocked.join("inner")).unwrap();
        assert!(write_atomic(&blocked, b"x", None).is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn sweep_removes_only_dead_or_old_temps_with_our_names() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        // A pid that cannot exist (above pid_max).
        let dead_old = d.join(".a.json.heronite-rs-4194999.tmp");
        let dead_new = d.join(".b.json.heronite-rs-4194999-7.tmp");
        let mine = unique_tmp_path(&d.join("c.json"));
        let stranger = d.join(".d.json.tmp");
        let other_prog = d.join(".e.json.other-1234.tmp");
        let plain = d.join("f.json");
        let tmp_dir = d.join(".g.json.heronite-rs-4194999.tmp");
        for p in [&dead_old, &dead_new, &mine, &stranger, &other_prog, &plain] {
            touch(p);
        }
        fs::create_dir(&tmp_dir).unwrap();
        let removed = sweep_dir(d);
        assert_eq!(removed.len(), 2, "{removed:?}");
        assert!(!dead_old.exists() && !dead_new.exists());
        for p in [&mine, &stranger, &other_prog, &plain, &tmp_dir] {
            assert!(p.exists(), "{} must be kept", p.display());
        }
        // A live pid's temp goes only once it is stale (pid reuse).
        let later = SystemTime::now() + STALE_AFTER + Duration::from_secs(60);
        assert_eq!(sweep_dir_at(d, later), vec![mine.clone()]);
        assert!(stranger.exists() && other_prog.exists());
    }

    #[test]
    fn legacy_files_need_to_be_old() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("shortcuts.vdf.tmp");
        touch(&p);
        let mut removed = Vec::new();
        sweep_legacy_file(&p, &mut removed);
        assert!(p.exists() && removed.is_empty());
        sweep_legacy_file_at(&p, SystemTime::now() + STALE_AFTER + Duration::from_secs(60), &mut removed);
        assert!(!p.exists());
        assert_eq!(removed.len(), 1);
    }
}
