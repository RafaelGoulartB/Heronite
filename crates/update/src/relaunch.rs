//! Starting the updated AppImage as the app quits.

use std::io;
use std::os::unix::process::CommandExt;
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
    mark_fds_cloexec();
    let home = std::env::var_os("HOME").filter(|h| Path::new(h).is_dir()).unwrap_or_else(|| "/".into());
    let mut cmd = Command::new(appimage);
    cmd.current_dir(home).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    // SAFETY: `setsid` is async-signal-safe and touches no memory of the parent.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn().map(|child| child.id())
}

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

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn child_runs_detached_without_inherited_files() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("out");
        let script = tmp.path().join("app");
        // Lists the descriptors the child got and its session id.
        std::fs::write(
            &script,
            format!("#!/bin/sh\nls /proc/$$/fd > {0}.tmp; ps -o sid= -p $$ >> {0}.tmp; mv {0}.tmp {0}\n", out.display()),
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
        // stdin/stdout/stderr, and the shell's own script descriptor at most.
        assert!(!lines.contains(&r.to_string().as_str()) && !lines.contains(&w.to_string().as_str()), "{lines:?}");
        // SAFETY: closing the pipe made above.
        unsafe {
            libc::close(r);
            libc::close(w);
        }
    }
}
