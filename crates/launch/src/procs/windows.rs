//! [`Procs`] from a process snapshot (`heronite_platform::process`).
//!
//! Start times are Windows' creation times, in milliseconds since 1601:
//! unlike Linux's ticks since boot they name one process across reboots too.
//! Nothing runs under Wine here, so the prefix questions have no answer.

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

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemProcs;

impl Procs for SystemProcs {
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
        let me = std::process::id();
        os::snapshot()
            .into_iter()
            .filter(|p| p.pid > 4 && p.pid != me)
            .filter(|p| os::exe_path(p.pid).is_some_and(|exe| under(&exe, folder)))
            .map(|p| p.pid)
            .collect()
    }

    fn user_ns(&self, _pid: u32) -> Option<String> {
        None
    }

    fn prefix_programs(&self, _prefix: &str, _folder: Option<&str>) -> Vec<WinProgram> {
        Vec::new()
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
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn paths_under_a_folder_ignore_case_and_separators() {
        assert!(under(Path::new(r"C:\Games\Hades\Hades.exe"), "c:/games/hades/"));
        assert!(under(Path::new(r"C:\Games\Hades"), r"C:\Games\Hades"));
        assert!(!under(Path::new(r"C:\Games\Hades II\Hades.exe"), r"C:\Games\Hades"));
        assert!(!under(Path::new(r"C:\Games\Hades.exe"), ""));
    }
}
