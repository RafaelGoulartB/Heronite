//! [`Procs`] from a process snapshot (`heronite_platform::process`).
//!
//! Start times are Windows' creation times, in milliseconds since 1601:
//! unlike Linux's ticks since boot they name one process across reboots too.
//! Nothing runs under Wine here: the questions about a prefix are about the
//! whole PC ([`Procs::prefix_programs`]), or have no answer.

use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use heronite_platform::process::{self as os, Signal};

use super::{program_matches, Procs, Sig, WinProgram};

/// Milliseconds between 1601-01-01 (Windows' epoch) and 1970-01-01.
const EPOCH_1601_MS: u64 = 11_644_473_600_000;

/// Now, in the clock of [`Procs::start_ticks`] (milliseconds since 1601),
/// minus a second of slack.
pub fn boot_ticks_now() -> u64 {
    let since_1970 = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    (since_1970 + EPOCH_1601_MS).saturating_sub(1000)
}

/// Our processes started at or after `since` (see [`boot_ticks_now`]) whose
/// program [`program_matches`] `pattern`.
pub fn processes_matching(pattern: &str, since: u64) -> Vec<u32> {
    let me = std::process::id();
    os::snapshot()
        .into_iter()
        .filter(|p| p.pid > 4 && p.pid != me && program_matches(&p.name, pattern))
        .filter(|p| os::start_time(p.pid).is_some_and(|t| t >= since))
        .map(|p| p.pid)
        .collect()
}

fn sig(sig: Sig) -> Signal {
    match sig {
        Sig::Term => Signal::Term,
        Sig::Kill => Signal::Kill,
    }
}

/// `path` is `folder` or inside it; Windows paths ignore case.
fn under(path: &Path, folder: &str) -> bool {
    let folder = folder.trim_end_matches(['/', '\\']).replace('/', "\\").to_lowercase();
    let path = path.to_string_lossy().replace('/', "\\").to_lowercase();
    !folder.is_empty() && (path == folder || path.starts_with(&format!("{folder}\\")))
}

/// `folder` and, when it is a link, the folder it leads to: Windows names a
/// program by its real path, and a GDK game's folder under `WindowsApps` is
/// a link to `<drive>\XboxGames\<game>\Content`.
fn folder_and_real(folder: &str) -> Vec<String> {
    let mut out = vec![folder.to_string()];
    if let Ok(real) = heronite_platform::disks::canonical(Path::new(folder)) {
        let real = real.to_string_lossy().into_owned();
        if !under(Path::new(&real), folder) {
            out.push(real);
        }
    }
    out
}

/// `path` is in one of `folders` (see [`folder_and_real`]).
fn under_any(path: &Path, folders: &[String]) -> bool {
    folders.iter().any(|f| under(path, f))
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemProcs;

impl Procs for SystemProcs {
    fn steam_client_running(&self) -> bool {
        use heronite_platform::registry::{dword, Hive};
        dword(Hive::CurrentUser, r"Software\Valve\Steam\ActiveProcess", "pid").is_some_and(|pid| pid != 0 && self.is_alive(pid))
            // Steam does not always keep ActiveProcess up to date.
            || os::snapshot().iter().any(|p| p.name.eq_ignore_ascii_case("steam.exe"))
    }

    fn is_alive(&self, pid: u32) -> bool {
        pid > 1 && os::is_alive(pid)
    }

    fn comm(&self, pid: u32) -> Option<String> {
        os::snapshot().into_iter().find(|p| p.pid == pid).map(|p| p.name)
    }

    fn children(&self, pid: u32) -> Vec<u32> {
        // A parent id can be a reused pid: a child is never older than its parent.
        let born = os::start_time(pid);
        os::snapshot()
            .into_iter()
            .filter(|p| p.parent == pid && p.pid != pid)
            .filter(|p| born.is_none_or(|b| os::start_time(p.pid).is_none_or(|t| t >= b)))
            .map(|p| p.pid)
            .collect()
    }

    fn parents(&self) -> HashMap<u32, u32> {
        os::snapshot().into_iter().map(|p| (p.pid, p.parent)).collect()
    }

    fn signal(&self, pid: u32, s: Sig) {
        if pid > 1 {
            os::signal(pid, sig(s));
        }
    }

    fn windows_programs(&self, _prefix: &str) -> Vec<String> {
        Vec::new()
    }

    fn start_ticks(&self, pid: u32) -> Option<u64> {
        os::start_time(pid)
    }

    fn group_alive(&self, pgid: u32) -> bool {
        os::group_alive(pgid)
    }

    fn signal_group(&self, pgid: u32, s: Sig) {
        os::signal_group(pgid, sig(s));
    }

    fn boot_id(&self) -> Option<String> {
        // Creation times are wall-clock times, so a pid and its start time
        // never name another process after a reboot: one id fits every boot.
        Some("windows".into())
    }

    fn game_pids(&self, _prefix: Option<&str>, folder: Option<&str>) -> Vec<u32> {
        let Some(folder) = folder else { return Vec::new() };
        let folders = folder_and_real(folder);
        let me = std::process::id();
        os::snapshot()
            .into_iter()
            .filter(|p| p.pid > 4 && p.pid != me)
            .filter(|p| os::exe_path(p.pid).is_some_and(|exe| under_any(&exe, &folders)))
            .map(|p| p.pid)
            .collect()
    }

    fn package_pids(&self, family: &str) -> Vec<u32> {
        let me = std::process::id();
        os::snapshot()
            .into_iter()
            .filter(|p| p.pid > 4 && p.pid != me)
            .filter(|p| os::package_family(p.pid).is_some_and(|f| f.eq_ignore_ascii_case(family)))
            .map(|p| p.pid)
            .collect()
    }

    fn user_ns(&self, _pid: u32) -> Option<String> {
        None
    }

    fn prefix_programs(&self, _prefix: &str, folder: Option<&str>) -> Vec<WinProgram> {
        // Without Wine every program is a Windows one: the whole PC, the
        // ones running from `folder` told apart by where their program is.
        let me = std::process::id();
        let folders = folder.map(folder_and_real).unwrap_or_default();
        os::snapshot()
            .into_iter()
            .filter(|p| p.pid > 4 && p.pid != me)
            .map(|p| {
                let exe = os::exe_path(p.pid);
                let from_folder = exe.as_deref().is_some_and(|e| under_any(e, &folders));
                let path = exe.map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
                WinProgram { pid: p.pid, name: p.name.to_lowercase(), path, from_folder }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_procs_sees_itself_and_its_children() {
        let me = std::process::id();
        assert!(SystemProcs.is_alive(me));
        assert!(SystemProcs.comm(me).is_some_and(|n| n.ends_with(".exe")));
        assert!(SystemProcs.start_ticks(me).is_some_and(|t| t <= boot_ticks_now() + 1000));
        assert!(!SystemProcs.is_alive(u32::MAX - 1));
        let mut child =
            std::process::Command::new("ping").args(["-n", "5", "127.0.0.1"]).stdout(std::process::Stdio::null()).spawn().unwrap();
        assert!(SystemProcs.children(me).contains(&child.id()));
        assert!(processes_matching("ping", 0).contains(&child.id()));
        assert!(processes_matching("ping", boot_ticks_now() + 10_000).is_empty());
        let exe = os::exe_path(child.id()).unwrap();
        let folder = exe.parent().unwrap().to_string_lossy().to_uppercase();
        assert!(SystemProcs.game_pids(None, Some(&folder)).contains(&child.id()));
        // Every program of the PC, the prefix aside; the child is from its folder.
        let programs = SystemProcs.prefix_programs("", Some(&folder));
        let ping = programs.iter().find(|p| p.pid == child.id()).unwrap();
        assert_eq!((ping.name.as_str(), ping.from_folder), ("ping.exe", true));
        assert!(ping.path.to_lowercase().ends_with("\\ping.exe"), "{}", ping.path);
        assert!(programs.iter().all(|p| p.pid != me), "never this process");
        assert!(SystemProcs.prefix_programs("", None).iter().all(|p| !p.from_folder));
        let _ = child.kill();
        let _ = child.wait();
    }

    /// A GDK game's folder under `WindowsApps` is a link to the folder its
    /// programs run from: the game is found through the link.
    #[test]
    fn a_game_is_found_through_a_link_to_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("XboxGames").join("Game").join("Content");
        std::fs::create_dir_all(&real).unwrap();
        let system = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        std::fs::copy(Path::new(&system).join("System32").join("ping.exe"), real.join("game.exe")).unwrap();
        let link = dir.path().join("WindowsApps").join("Studio.Game_1.0.0.0_x64__abc");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        // A junction, as Windows makes them; no privilege needed.
        let made = std::process::Command::new("cmd").arg("/C").arg("mklink").arg("/J").arg(&link).arg(&real).output().unwrap();
        assert!(made.status.success(), "{made:?}");
        let mut child = std::process::Command::new(real.join("game.exe"))
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let link = link.to_string_lossy().into_owned();
        let found = SystemProcs.game_pids(None, Some(&link)).contains(&child.id());
        let from_folder = SystemProcs.prefix_programs("", Some(&link)).iter().any(|p| p.pid == child.id() && p.from_folder);
        // Not from a package: a test runs none.
        let packaged = SystemProcs.package_pids("Studio.Game_abc").contains(&child.id());
        let _ = child.kill();
        let _ = child.wait();
        assert!(found, "found from the link's folder");
        assert!(from_folder);
        assert!(!packaged);
    }

    #[test]
    fn paths_under_a_folder_ignore_case_and_separators() {
        assert!(under(Path::new(r"C:\Games\Hades\Hades.exe"), "c:/games/hades/"));
        assert!(under(Path::new(r"C:\Games\Hades"), r"C:\Games\Hades"));
        assert!(!under(Path::new(r"C:\Games\Hades II\Hades.exe"), r"C:\Games\Hades"));
        assert!(!under(Path::new(r"C:\Games\Hades.exe"), ""));
    }
}
