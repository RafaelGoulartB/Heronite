//! Starting the updated app (or the Windows setup that installs it) as the
//! app quits.

use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

/// Starts `appimage` in its own session, detached from this process, which
/// is about to exit.
///
/// The child must not inherit this process's files: the AppImage runtime
/// that started us keeps its FUSE mount alive through an open pipe, and an
/// inherited copy would keep the old version mounted for as long as the new
/// one runs. So every descriptor above stderr is marked close-on-exec first.
pub fn relaunch(appimage: &Path) -> io::Result<u32> {
    launch(appimage, &[])
}

/// Starts `program` with `args` like [`relaunch`]: detached, in the home
/// folder, without this process's input and output. On Windows it also
/// leaves the job object this process may run in (a launcher, a terminal),
/// when that job allows it, so closing the job does not end it.
pub fn launch(program: &Path, args: &[OsString]) -> io::Result<u32> {
    #[cfg(target_os = "linux")]
    mark_fds_cloexec();
    let home = heronite_platform::dirs::home().filter(|h| h.is_dir()).unwrap_or_else(|| "/".into());
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(home).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    heronite_platform::process::detach(&mut cmd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // `detach`'s flags (DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP) and
        // CREATE_BREAKAWAY_FROM_JOB. A job that forbids breaking away makes
        // CreateProcess fail with ERROR_ACCESS_DENIED: then it starts inside.
        const DETACHED: u32 = 0x0000_0008 | 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        cmd.creation_flags(DETACHED | CREATE_BREAKAWAY_FROM_JOB);
        match cmd.spawn() {
            Err(e) if e.raw_os_error() == Some(5) => {
                cmd.creation_flags(DETACHED);
            }
            other => return other.map(|child| child.id()),
        }
    }
    cmd.spawn().map(|child| child.id())
}

#[cfg(target_os = "linux")]
fn mark_fds_cloexec() {
    let Ok(entries) = std::fs::read_dir("/proc/self/fd") else { return };
    for fd in entries.flatten().filter_map(|e| e.file_name().to_str()?.parse::<libc::c_int>().ok()) {
        if fd <= 2 {
            continue;
        }
        // SAFETY: F_GETFD / F_SETFD only change this descriptor's flags; a
        // descriptor closed meanwhile (the directory listing's own) just fails.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags >= 0 {
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn child_runs_detached_without_inherited_files() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("out");
        let script = tmp.path().join("app");
        // Lists what the child's descriptors point to, and its session id.
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nfor f in /proc/$$/fd/*; do readlink $f; done > {0}.tmp; ps -o sid= -p $$ >> {0}.tmp; mv {0}.tmp {0}\n",
                out.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        // A descriptor without close-on-exec, like the AppImage runtime's pipe.
        let (r, w) = {
            let mut fds = [0; 2];
            // SAFETY: `fds` has room for the two descriptors.
            assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
            (fds[0], fds[1])
        };
        let pid = super::relaunch(&script).unwrap();
        for _ in 0..200 {
            if out.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let text = std::fs::read_to_string(&out).unwrap();
        let mut lines: Vec<&str> = text.lines().collect();
        let sid: u32 = lines.pop().unwrap().trim().parse().unwrap();
        assert_eq!(sid, pid, "the child leads its own session");
        // Compared by what they point to, not by number: the shell opens its
        // own descriptors (dash puts the script at 10) that may reuse the
        // pipe's numbers.
        let pipe = std::fs::read_link(format!("/proc/self/fd/{r}")).unwrap();
        assert!(!lines.contains(&pipe.to_str().unwrap()), "{pipe:?} in {lines:?}");
        // SAFETY: closing the pipe made above.
        unsafe {
            libc::close(r);
            libc::close(w);
        }
    }
}
