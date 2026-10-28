//! Whether the Steam client runs. One answer for every crate that asks: the
//! `steam.pid` files of the native, Flatpak and Snap installs, each checked
//! against the process it names, with a scan for a `steam` process when no
//! file tells.
//!
//! A pid file alone is not enough: a crash leaves it behind, and its pid may
//! belong to something else by the next boot. So the pid must be alive and be
//! `steam`. The Flatpak's pid is from the sandbox's pid namespace and may
//! name nothing (or something else) on the host; the scan covers that.

use std::path::{Path, PathBuf};

/// Flatpak id of the Steam client.
const FLATPAK_ID: &str = "com.valvesoftware.Steam";

/// Name (`comm`) of the Steam client's main process.
const CLIENT_COMM: &str = "steam";

/// The processes of the machine, behind a trait so callers can test with
/// fake ones.
pub trait ProcTable {
    /// Short name of a process that is alive and not a zombie.
    fn live_comm(&self, pid: u32) -> Option<String>;
    /// Every live pid, read at once.
    fn live_pids(&self) -> Vec<u32>;
}

/// The processes read from a `/proc`-shaped folder (`/proc` itself, or a fake
/// one in tests).
pub struct ProcFs<'a>(pub &'a Path);

impl ProcFs<'static> {
    pub fn system() -> Self {
        ProcFs(Path::new("/proc"))
    }
}

impl ProcTable for ProcFs<'_> {
    fn live_comm(&self, pid: u32) -> Option<String> {
        if pid <= 1 {
            return None;
        }
        let stat = std::fs::read_to_string(self.0.join(pid.to_string()).join("stat")).ok()?;
        // "pid (comm, maybe with spaces or ')') S ppid ..."
        let (start, end) = (stat.find('(')?, stat.rfind(')')?);
        let state = stat.get(end + 1..)?.split_whitespace().next()?;
        (start < end && state != "Z" && state != "X").then(|| stat[start + 1..end].to_string())
    }

    fn live_pids(&self) -> Vec<u32> {
        let Ok(rd) = std::fs::read_dir(self.0) else {
            return Vec::new();
        };
        rd.flatten().filter_map(|e| e.file_name().to_str()?.parse().ok()).collect()
    }
}

/// The `steam.pid` files Steam may write, most likely first: the one beside
/// `~/.steam/steam` (where Steam always puts it, whatever the configured
/// folder is), the configured folder's and its parent's, then the Flatpak's
/// and the Snap's.
pub fn pid_files(home: &Path, steam_root: &Path) -> Vec<PathBuf> {
    let flatpak = home.join(".var/app").join(FLATPAK_ID);
    let mut files = vec![home.join(".steam/steam.pid"), steam_root.join("steam.pid")];
    files.extend(steam_root.parent().map(|p| p.join("steam.pid")));
    files.extend([
        flatpak.join(".steam/steam.pid"),
        flatpak.join(".local/share/Steam/steam.pid"),
        home.join("snap/steam/common/.steam/steam.pid"),
    ]);
    let mut seen = Vec::new();
    files.retain(|f| {
        let new = !seen.contains(f);
        seen.push(f.clone());
        new
    });
    files
}

fn is_client(procs: &dyn ProcTable, pid: u32) -> bool {
    procs.live_comm(pid).is_some_and(|c| c.trim() == CLIENT_COMM)
}

/// The first pid file that names a live `steam` process.
pub fn live_pid_file(procs: &dyn ProcTable, home: &Path, steam_root: &Path) -> Option<PathBuf> {
    pid_files(home, steam_root)
        .into_iter()
        .find(|file| std::fs::read_to_string(file).ok().and_then(|s| s.trim().parse::<u32>().ok()).is_some_and(|pid| is_client(procs, pid)))
}

/// Whether the Steam client runs: a pid file names it, or (the file missing,
/// stale or from another pid namespace) a `steam` process exists. Steam Deck
/// game mode (`gamescope`) counts as running, as Steam is the session there.
pub fn is_running(procs: &dyn ProcTable, home: &Path, steam_root: &Path) -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").as_deref() == Ok("gamescope")
        || live_pid_file(procs, home, steam_root).is_some()
        || procs.live_pids().into_iter().any(|pid| is_client(procs, pid))
}

/// [`is_running`] on this machine's processes.
pub fn is_running_here(home: &Path, steam_root: &Path) -> bool {
    is_running(&ProcFs::system(), home, steam_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Rig {
        _dir: tempfile::TempDir,
        home: PathBuf,
        proc: PathBuf,
    }

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let proc = dir.path().join("proc");
        fs::create_dir_all(home.join(".steam")).unwrap();
        fs::create_dir_all(&proc).unwrap();
        Rig { _dir: dir, home, proc }
    }

    impl Rig {
        fn root(&self) -> PathBuf {
            self.home.join(".local/share/Steam")
        }
        fn process(&self, pid: u32, comm: &str, state: char) {
            let d = self.proc.join(pid.to_string());
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("stat"), format!("{pid} ({comm}) {state} 1 {pid} 0 0\n")).unwrap();
        }
        fn running(&self) -> bool {
            is_running(&ProcFs(&self.proc), &self.home, &self.root())
        }
    }

    #[test]
    fn a_pid_file_of_a_live_steam_counts_wherever_the_install_is() {
        let r = rig();
        r.process(4242, "steam", 'S');
        // The configured folder is not where the pid file is: `~/.steam/steam` is.
        fs::write(r.home.join(".steam/steam.pid"), "4242\n").unwrap();
        assert!(r.running());
        assert_eq!(live_pid_file(&ProcFs(&r.proc), &r.home, &r.root()), Some(r.home.join(".steam/steam.pid")));
    }

    #[test]
    fn flatpak_snap_and_configured_folders_are_checked() {
        for rel in [
            ".var/app/com.valvesoftware.Steam/.steam/steam.pid",
            ".var/app/com.valvesoftware.Steam/.local/share/Steam/steam.pid",
            "snap/steam/common/.steam/steam.pid",
            ".local/share/Steam/steam.pid",
            ".local/share/steam.pid",
        ] {
            let r = rig();
            r.process(77, "steam", 'S');
            let file = r.home.join(rel);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, "77").unwrap();
            assert_eq!(live_pid_file(&ProcFs(&r.proc), &r.home, &r.root()), Some(file), "{rel}");
        }
    }

    #[test]
    fn a_dead_pid_is_not_running() {
        let r = rig();
        fs::write(r.home.join(".steam/steam.pid"), "999999").unwrap();
        assert!(!r.running());
    }

    #[test]
    fn a_pid_reused_by_another_program_is_not_running() {
        let r = rig();
        r.process(4242, "firefox", 'S');
        fs::write(r.home.join(".steam/steam.pid"), "4242").unwrap();
        assert!(!r.running());
    }

    #[test]
    fn this_very_process_is_not_steam() {
        let r = rig();
        fs::write(r.home.join(".steam/steam.pid"), std::process::id().to_string()).unwrap();
        assert_eq!(live_pid_file(&ProcFs::system(), &r.home, &r.root()), None);
    }

    #[test]
    fn a_zombie_steam_is_not_running() {
        let r = rig();
        r.process(4242, "steam", 'Z');
        fs::write(r.home.join(".steam/steam.pid"), "4242").unwrap();
        assert!(!r.running());
    }

    #[test]
    fn garbage_in_the_pid_file_is_ignored() {
        let r = rig();
        fs::write(r.home.join(".steam/steam.pid"), "not a pid").unwrap();
        assert!(!r.running());
    }

    #[test]
    fn a_steam_process_counts_without_a_usable_pid_file() {
        let r = rig();
        // The Flatpak's pid file holds a pid from its own namespace.
        let file = r.home.join(".var/app/com.valvesoftware.Steam/.steam/steam.pid");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, "3").unwrap();
        assert!(!r.running());
        r.process(900, "steamwebhelper", 'S');
        assert!(!r.running());
        r.process(901, "steam", 'S');
        assert!(r.running());
    }

    #[test]
    fn comm_with_parentheses_and_spaces_is_read_whole() {
        let r = rig();
        r.process(5, "a (b) c", 'S');
        assert_eq!(ProcFs(&r.proc).live_comm(5).as_deref(), Some("a (b) c"));
        assert_eq!(ProcFs(&r.proc).live_comm(6), None);
    }
}
