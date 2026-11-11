//! The file moves of [`crate::journal_deploy`], made to survive two things a
//! game folder on Windows does:
//!
//! * **A file that is still open.** Right after a game ends, its crash
//!   reporter, an anti-virus scan or the store client can hold a file for a
//!   moment, and Windows refuses to delete or rename it. [`Patience`] retries
//!   with a growing pause until one shared deadline, and [`probe`] tells
//!   whether a file can be taken before anything is touched.
//! * **A copy cut short.** A move to another drive is a copy and a delete. The
//!   copy goes to `<name>.heronite-partial` beside the target (so on the
//!   target's drive), is flushed, gets the source's modification time, and is
//!   renamed into place; only then is the source removed. A crash leaves the
//!   target missing or whole, never half written, and the source whole. A
//!   leftover `*.heronite-partial` is therefore never needed: [`sweep_partials`]
//!   removes it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The suffix of a copy that is not in place yet.
pub(crate) const PARTIAL: &str = ".heronite-partial";

/// Whether a file name is a copy that never got into place.
pub(crate) fn is_partial(name: &str) -> bool {
    name.ends_with(PARTIAL)
}

/// Where the copy to `to` is written before it is renamed into place.
pub(crate) fn partial_of(to: &Path) -> PathBuf {
    let mut name = to.file_name().unwrap_or_default().to_os_string();
    name.push(PARTIAL);
    to.with_file_name(name)
}

/// Removes every `*.heronite-partial` under `root`. Its source is still whole
/// (it is only removed after the rename), so nothing is lost.
pub(crate) fn sweep_partials(root: &Path) -> usize {
    let mut found = Vec::new();
    crate::fsutil::walk_files(root, &mut |rel, meta| {
        if meta.is_file() && is_partial(rel) {
            found.push(root.join(rel));
        }
    });
    let mut removed = 0;
    for path in found {
        match remove_file(&path) {
            Ok(()) => {
                log::info!("mods: removed {}, a copy a crash cut short", path.display());
                removed += 1;
            }
            Err(e) => log::warn!("mods: could not remove {}: {e}", path.display()),
        }
    }
    removed
}

/// Copies `from` to `to` through [`partial_of`]`(to)`: flushed, given the
/// source's modification time, then renamed over `to`. `to` is never seen
/// half written.
pub(crate) fn copy_atomic(from: &Path, to: &Path) -> io::Result<u64> {
    let tmp = partial_of(to);
    let result = (|| {
        let size = fs::copy(from, &tmp)?;
        // The copy takes the source's attributes; a read-only one is opened
        // for the flush and set read-only again after.
        let perms = fs::metadata(&tmp)?.permissions();
        if perms.readonly() {
            let mut writable = perms.clone();
            #[allow(clippy::permissions_set_readonly_false)]
            writable.set_readonly(false);
            fs::set_permissions(&tmp, writable)?;
        }
        let file = fs::OpenOptions::new().write(true).open(&tmp)?;
        if let Ok(modified) = from.metadata().and_then(|m| m.modified()) {
            file.set_modified(modified)?;
        }
        file.sync_all()?;
        drop(file);
        if perms.readonly() {
            fs::set_permissions(&tmp, perms)?;
        }
        fs::rename(&tmp, to)?;
        Ok(size)
    })();
    if result.is_err() {
        let _ = remove_file(&tmp);
    }
    result
}

/// Removes a file, also a read-only one (a copy keeps the attribute of the
/// mod's file, and Windows refuses to delete a read-only file).
pub(crate) fn remove_file(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            let Ok(meta) = fs::symlink_metadata(path) else { return Err(e) };
            let mut perms = meta.permissions();
            if !meta.is_file() || !perms.readonly() {
                return Err(e);
            }
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            fs::set_permissions(path, perms)?;
            fs::remove_file(path)
        }
        other => other,
    }
}

/// Rename, else (another drive) [`move_by_copy`].
pub(crate) fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        // A file that is open is not taken by a copy either.
        Err(e) if is_locked(&e) => Err(e),
        Err(_) => move_by_copy(from, to),
    }
}

/// The move between drives: [`copy_atomic`], then the source is removed. A
/// crash in between leaves two whole copies, and the journal's next run
/// treats either as good.
pub(crate) fn move_by_copy(from: &Path, to: &Path) -> io::Result<()> {
    copy_atomic(from, to)?;
    remove_file(from)
}

/// Whether an error is a file held open by another program, which goes away
/// by itself: a sharing or lock violation, or a delete that is still pending
/// (access denied). On Linux nothing holds a file like that, but a busy
/// mount point or running program can.
pub(crate) fn is_locked(e: &io::Error) -> bool {
    #[cfg(windows)]
    {
        // ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION.
        matches!(e.raw_os_error(), Some(5 | 32 | 33))
    }
    #[cfg(not(windows))]
    {
        // EBUSY, ETXTBSY.
        matches!(e.raw_os_error(), Some(16 | 26))
    }
}

/// Fails, the way a delete or rename would, while another program has
/// `path` open. A missing file (or a folder) is free.
pub(crate) fn probe(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        if path.symlink_metadata().map_or(true, |m| m.is_dir()) {
            return Ok(());
        }
        use std::os::windows::fs::OpenOptionsExt;
        // DELETE access with no sharing: refused while any other handle is open.
        const DELETE: u32 = 0x0001_0000;
        const FILE_READ_ATTRIBUTES: u32 = 0x80;
        match fs::OpenOptions::new().access_mode(DELETE | FILE_READ_ATTRIBUTES).share_mode(0).open(path) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Ok(())
    }
}

/// How long a restore waits, all files together, for files to be let go.
pub(crate) const RESTORE_PATIENCE: Duration = Duration::from_secs(15);

/// Retries an operation on a locked file with a growing pause (25 ms, doubled
/// up to 1 s), until one deadline shared by every file of a restore. So a
/// few locked files cost a few seconds, and many never cost more than the
/// whole budget.
#[derive(Debug)]
pub(crate) struct Patience {
    deadline: Instant,
}

impl Patience {
    pub(crate) fn new(budget: Duration) -> Self {
        Self { deadline: Instant::now() + budget }
    }

    pub(crate) fn retry<T>(&self, mut op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
        let mut pause = Duration::from_millis(25);
        loop {
            match op() {
                Err(e) if is_locked(&e) && Instant::now() + pause < self.deadline => {
                    std::thread::sleep(pause);
                    pause = (pause * 2).min(Duration::from_secs(1));
                }
                other => return other,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_appears_whole_with_the_sources_time_and_leaves_nothing_beside() {
        let t = tempfile::tempdir().unwrap();
        let from = t.path().join("a.bin");
        fs::write(&from, vec![1u8; 200_000]).unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        fs::OpenOptions::new().write(true).open(&from).unwrap().set_modified(old).unwrap();
        let to = t.path().join("b.bin");
        fs::write(&to, "older content").unwrap();
        assert_eq!(copy_atomic(&from, &to).unwrap(), 200_000);
        assert_eq!(fs::read(&to).unwrap(), vec![1u8; 200_000]);
        assert_eq!(fs::metadata(&to).unwrap().modified().unwrap(), old);
        assert!(!partial_of(&to).exists());
    }

    #[test]
    fn a_move_by_copy_removes_the_source_only_once_the_target_is_whole() {
        let t = tempfile::tempdir().unwrap();
        let from = t.path().join("a.txt");
        fs::write(&from, "game").unwrap();
        let to = t.path().join("sub/a.txt");
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        move_by_copy(&from, &to).unwrap();
        assert!(!from.exists() && fs::read_to_string(&to).unwrap() == "game");
        // A failed copy (no source) leaves the target and nothing beside it.
        assert!(move_by_copy(&from, &to).is_err());
        assert_eq!(fs::read_to_string(&to).unwrap(), "game");
        assert!(!partial_of(&to).exists());
    }

    #[test]
    fn partials_are_swept_and_nothing_else() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir_all(t.path().join("Data")).unwrap();
        fs::write(t.path().join("Data/a.esp.heronite-partial"), "half").unwrap();
        fs::write(t.path().join("Data/a.esp"), "whole").unwrap();
        assert_eq!(sweep_partials(t.path()), 1);
        assert!(!t.path().join("Data/a.esp.heronite-partial").exists());
        assert!(t.path().join("Data/a.esp").exists());
        assert_eq!(sweep_partials(&t.path().join("missing")), 0);
    }

    #[test]
    fn patience_retries_a_lock_until_the_deadline_and_no_other_error() {
        let locked = || io::Error::from_raw_os_error(if cfg!(windows) { 32 } else { 16 });
        let p = Patience::new(Duration::from_millis(300));
        let mut tries = 0;
        let started = Instant::now();
        assert!(p
            .retry(|| -> io::Result<()> {
                tries += 1;
                Err(locked())
            })
            .is_err());
        assert!(tries > 2, "{tries}");
        assert!(started.elapsed() < Duration::from_secs(2));
        let mut tries = 0;
        assert!(Patience::new(Duration::from_secs(5))
            .retry(|| -> io::Result<()> {
                tries += 1;
                Err(io::Error::new(io::ErrorKind::NotFound, "gone"))
            })
            .is_err());
        assert_eq!(tries, 1, "only locks are waited for");
        let mut tries = 0;
        let ok = Patience::new(Duration::from_secs(5)).retry(|| {
            tries += 1;
            if tries < 3 {
                Err(locked())
            } else {
                Ok(tries)
            }
        });
        assert_eq!(ok.unwrap(), 3);
    }

    /// A file another program holds open is seen before anything is moved.
    #[cfg(windows)]
    #[test]
    fn an_open_file_is_seen_as_locked() {
        use std::os::windows::fs::OpenOptionsExt;
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("game.log");
        fs::write(&f, "x").unwrap();
        assert!(probe(&f).is_ok());
        let held = fs::OpenOptions::new().read(true).share_mode(0).open(&f).unwrap();
        let e = probe(&f).unwrap_err();
        assert!(is_locked(&e), "{e:?}");
        assert!(is_locked(&fs::remove_file(&f).unwrap_err()));
        drop(held);
        assert!(probe(&f).is_ok() && probe(&t.path().join("missing")).is_ok());
    }
}
