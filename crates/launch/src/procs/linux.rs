//! [`Procs`] from `/proc`.

use std::collections::HashMap;

use super::{program_matches, same_prefix, Procs, Sig, WinProgram, WINE_SERVICES};

/// `path` is `folder` or inside it.
fn under(path: &std::path::Path, folder: &str) -> bool {
    let folder = folder.trim_end_matches('/');
    !folder.is_empty() && path.starts_with(folder)
}

/// The file name of a Windows path Wine gives a process as its name
/// (`C:\windows\system32\services.exe`, `Z:\games\Game.exe`).
fn windows_file_name(argv0: &str) -> Option<String> {
    let b = argv0.as_bytes();
    if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' || b[2] != b'\\' {
        return None;
    }
    argv0.rsplit('\\').next().filter(|n| !n.is_empty()).map(str::to_lowercase)
}

/// The names a process goes by for [`program_matches`]: its comm and the
/// file names of its first two words (`bash run.sh`, `Z:\games\Game.exe`).
fn program_names(comm: &str, cmdline: &[u8]) -> Vec<String> {
    let mut names = vec![comm.trim().to_string()];
    for word in cmdline.split(|b| *b == 0).filter(|w| !w.is_empty()).take(2) {
        let word = String::from_utf8_lossy(word);
        if let Some(name) = word.rsplit(['/', '\\']).next().filter(|n| !n.is_empty()) {
            names.push(name.to_string());
        }
    }
    names
}

/// Start time of a process from its `stat`, in clock ticks since boot.
fn parse_start_ticks(stat: &str) -> Option<u64> {
    let end = stat.rfind(')')?;
    // Field 22 of stat; the fields after the comm start at field 3.
    stat[end + 1..].split_whitespace().nth(19)?.parse().ok()
}

fn clock_ticks_per_second() -> u64 {
    // SAFETY: sysconf has no preconditions.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if hz > 0 {
        hz as u64
    } else {
        100
    }
}

/// Now, in the clock `/proc/<pid>/stat` measures start times with (ticks
/// since boot), minus a second of slack.
pub fn boot_ticks_now() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: clock_gettime writes into the timespec we own.
    if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut ts) } != 0 {
        return 0;
    }
    let hz = clock_ticks_per_second();
    (ts.tv_sec as u64).saturating_sub(1) * hz + ts.tv_nsec as u64 * hz / 1_000_000_000
}

/// Our processes started at or after `since` (see [`boot_ticks_now`]) whose
/// program [`program_matches`] `pattern`. What the game started that left its
/// process group; anything that was running before the game is never named.
pub fn processes_matching(pattern: &str, since: u64) -> Vec<u32> {
    let Ok(rd) = std::fs::read_dir("/proc") else { return Vec::new() };
    let me = std::process::id();
    let mut out = Vec::new();
    for e in rd.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        if pid <= 1 || pid == me {
            continue;
        }
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { continue };
        if parse_start_ticks(&stat).is_none_or(|t| t < since) {
            continue;
        }
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
        let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        if program_names(&comm, &cmdline).iter().any(|n| program_matches(n, pattern)) {
            out.push(pid);
        }
    }
    out
}

/// `/proc` on Linux.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemProcs;

fn parse_stat(stat: &str) -> Option<(char, u32)> {
    // "pid (comm with spaces) S ppid ..."; comm may contain ')'.
    let end = stat.rfind(')')?;
    let mut it = stat[end + 1..].split_whitespace();
    let state = it.next()?.chars().next()?;
    let ppid = it.next()?.parse().ok()?;
    Some((state, ppid))
}

impl Procs for SystemProcs {
    fn is_alive(&self, pid: u32) -> bool {
        if pid <= 1 {
            return false;
        }
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => parse_stat(&stat).map(|(s, _)| s != 'Z' && s != 'X').unwrap_or(false),
            Err(_) => false,
        }
    }

    fn comm(&self, pid: u32) -> Option<String> {
        std::fs::read_to_string(format!("/proc/{pid}/comm")).ok().map(|s| s.trim().to_string())
    }

    fn children(&self, pid: u32) -> Vec<u32> {
        let Ok(rd) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for e in rd.flatten() {
            let Some(child) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else {
                continue;
            };
            if let Ok(stat) = std::fs::read_to_string(format!("/proc/{child}/stat")) {
                if parse_stat(&stat).map(|(_, ppid)| ppid == pid).unwrap_or(false) {
                    out.push(child);
                }
            }
        }
        out
    }

    fn parents(&self) -> HashMap<u32, u32> {
        let Ok(rd) = std::fs::read_dir("/proc") else {
            return HashMap::new();
        };
        rd.flatten()
            .filter_map(|e| {
                let pid = e.file_name().to_str()?.parse::<u32>().ok()?;
                let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
                parse_stat(&stat).map(|(_, ppid)| (pid, ppid))
            })
            .collect()
    }

    fn signal(&self, pid: u32, sig: Sig) {
        if pid <= 1 {
            return;
        }
        let s = match sig {
            Sig::Term => libc::SIGTERM,
            Sig::Kill => libc::SIGKILL,
        };
        // SAFETY: plain kill(2) on a pid we were asked to signal.
        unsafe {
            libc::kill(pid as libc::pid_t, s);
        }
    }

    fn windows_programs(&self, prefix: &str) -> Vec<String> {
        let Ok(rd) = std::fs::read_dir("/proc") else { return Vec::new() };
        let mut out = Vec::new();
        for e in rd.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
            let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) else { continue };
            let argv0 = String::from_utf8_lossy(cmdline.split(|b| *b == 0).next().unwrap_or_default()).into_owned();
            let Some(name) = windows_file_name(&argv0) else { continue };
            if in_wine_prefix(pid, prefix) && self.is_alive(pid) {
                out.push(name);
            }
        }
        out
    }

    fn start_ticks(&self, pid: u32) -> Option<u64> {
        if pid <= 1 {
            return None;
        }
        parse_start_ticks(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
    }

    fn group_alive(&self, pgid: u32) -> bool {
        if pgid <= 1 {
            return false;
        }
        // SAFETY: signal 0 only checks that the group exists.
        let r = unsafe { libc::kill(-(pgid as libc::pid_t), 0) };
        r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    fn signal_group(&self, pgid: u32, sig: Sig) {
        if pgid <= 1 {
            return;
        }
        let s = match sig {
            Sig::Term => libc::SIGTERM,
            Sig::Kill => libc::SIGKILL,
        };
        // SAFETY: kill(2) on a process group we were asked to signal.
        unsafe {
            libc::kill(-(pgid as libc::pid_t), s);
        }
    }

    fn boot_id(&self) -> Option<String> {
        std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    }

    fn game_pids(&self, prefix: Option<&str>, folder: Option<&str>) -> Vec<u32> {
        if prefix.is_none() && folder.is_none() {
            return Vec::new();
        }
        let me = std::process::id();
        let Ok(rd) = std::fs::read_dir("/proc") else { return Vec::new() };
        let mut out = Vec::new();
        for e in rd.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
            if pid == me || !self.is_alive(pid) {
                continue;
            }
            let link = |what: &str| std::fs::read_link(format!("/proc/{pid}/{what}")).ok();
            if let Some(folder) = folder {
                if link("exe").is_some_and(|exe| under(&exe, folder)) {
                    out.push(pid);
                    continue;
                }
            }
            let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) else { continue };
            let argv0 = String::from_utf8_lossy(cmdline.split(|b| *b == 0).next().unwrap_or_default()).into_owned();
            let Some(name) = windows_file_name(&argv0) else { continue };
            if WINE_SERVICES.contains(&name.as_str()) {
                continue;
            }
            let from_folder = folder.is_some_and(|f| link("cwd").is_some_and(|cwd| under(&cwd, f)));
            if from_folder || prefix.is_some_and(|prefix| in_wine_prefix(pid, prefix)) {
                out.push(pid);
            }
        }
        out
    }

    fn user_ns(&self, pid: u32) -> Option<String> {
        std::fs::read_link(format!("/proc/{pid}/ns/user")).ok().map(|l| l.to_string_lossy().into_owned())
    }

    fn prefix_programs(&self, prefix: &str, folder: Option<&str>) -> Vec<WinProgram> {
        let me = std::process::id();
        let Ok(rd) = std::fs::read_dir("/proc") else { return Vec::new() };
        let mut out = Vec::new();
        for e in rd.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
            if pid == me {
                continue;
            }
            let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) else { continue };
            let argv0 = String::from_utf8_lossy(cmdline.split(|b| *b == 0).next().unwrap_or_default()).into_owned();
            let Some(name) = windows_file_name(&argv0).filter(|n| !WINE_SERVICES.contains(&n.as_str())) else { continue };
            let from_folder = folder.is_some_and(|f| std::fs::read_link(format!("/proc/{pid}/cwd")).is_ok_and(|cwd| under(&cwd, f)));
            if (from_folder || in_wine_prefix(pid, prefix)) && self.is_alive(pid) {
                out.push(WinProgram { pid, name, path: argv0, from_folder });
            }
        }
        out
    }
}

/// Whether the process runs in the Wine prefix `prefix` (its `WINEPREFIX`).
/// Other users' processes are not readable: they are not ours anyway.
fn in_wine_prefix(pid: u32, prefix: &str) -> bool {
    std::fs::read(format!("/proc/{pid}/environ")).is_ok_and(|env| {
        env.split(|b| *b == 0).filter_map(|kv| kv.strip_prefix(b"WINEPREFIX=")).any(|v| same_prefix(&String::from_utf8_lossy(v), prefix))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procs::monotonic_ms;

    #[test]
    fn windows_processes_are_matched_to_their_prefix() {
        assert_eq!(windows_file_name("C:\\windows\\system32\\services.exe").as_deref(), Some("services.exe"));
        assert_eq!(windows_file_name("Z:\\games\\My Game\\Game.EXE").as_deref(), Some("game.exe"));
        assert_eq!(windows_file_name("/usr/bin/wineserver"), None);
        assert_eq!(windows_file_name("C:"), None);
        assert!(same_prefix("/p/game/pfx/", "/p/game") && same_prefix("/p/game", "/p/game/"));
        assert!(!same_prefix("/p/game2", "/p/game") && !same_prefix("/p/game/other", "/p/game") && !same_prefix("/x", ""));
    }

    #[test]
    fn a_process_goes_by_its_comm_and_first_words() {
        let names = program_names("bash\n", b"/bin/bash\0/home/u/run.sh\0--fast\0");
        assert_eq!(names, ["bash", "bash", "run.sh"]);
        assert_eq!(program_names("Game.exe", b"Z:\\games\\Game.exe\0")[1], "Game.exe");
    }

    /// A program Wine would name `C:\\games\\Game.EXE` (the name is argv[0]).
    fn windows_program(argv0: &str, prefix: Option<&str>, cwd: Option<&std::path::Path>) -> std::process::Child {
        use std::os::unix::process::CommandExt;
        let mut c = std::process::Command::new("sleep");
        c.arg0(argv0).arg("5").env_remove("WINEPREFIX");
        if let Some(p) = prefix {
            c.env("WINEPREFIX", p);
        }
        if let Some(d) = cwd {
            c.current_dir(d);
        }
        c.spawn().unwrap()
    }

    #[test]
    fn the_windows_programs_of_a_prefix_come_with_their_names() {
        let dir = tempfile::tempdir().unwrap();
        let mut game = windows_program("C:\\games\\Game.EXE", Some("/pfx/store/pfx"), None);
        let mut service = windows_program("C:\\windows\\system32\\services.exe", Some("/pfx/store/pfx"), None);
        let mut elsewhere = windows_program("C:\\games\\Other.exe", Some("/pfx/other"), None);
        let mut from_folder = windows_program("Z:\\g\\Folder.exe", None, Some(dir.path()));
        let found: Vec<(u32, String, bool)> =
            SystemProcs.prefix_programs("/pfx/store", dir.path().to_str()).into_iter().map(|p| (p.pid, p.name, p.from_folder)).collect();
        assert!(found.contains(&(game.id(), "game.exe".to_string(), false)), "{found:?}");
        assert!(found.contains(&(from_folder.id(), "folder.exe".to_string(), true)), "{found:?}");
        assert!(found.iter().all(|(pid, ..)| *pid != service.id() && *pid != elsewhere.id()), "{found:?}");
        let by_prefix = SystemProcs.prefix_programs("/pfx/store", None);
        assert!(by_prefix.iter().any(|p| p.pid == game.id() && p.path == "C:\\games\\Game.EXE"));
        assert!(by_prefix.iter().all(|p| p.pid != from_folder.id()));
        for child in [&mut game, &mut service, &mut elsewhere, &mut from_folder] {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    #[test]
    fn parses_stat_with_odd_comm() {
        assert_eq!(parse_stat("42 (my (weird) app) S 7 42 42 0"), Some(('S', 7)));
        assert_eq!(parse_stat("42 (x) Z 1 0"), Some(('Z', 1)));
    }

    #[test]
    fn processes_started_before_the_game_are_never_matched() {
        let me = std::fs::read_to_string("/proc/self/stat").unwrap();
        assert!(parse_start_ticks(&me).is_some_and(|t| t <= boot_ticks_now() + clock_ticks_per_second()));
        assert_eq!(parse_start_ticks("42 (a b) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 777 9"), Some(777));
        // Started long before "now": nothing, not even with the widest pattern match.
        let mut child = std::process::Command::new("sleep").arg("5").spawn().unwrap();
        assert!(processes_matching("sleep", boot_ticks_now() + 10 * clock_ticks_per_second()).is_empty());
        assert!(processes_matching("sleep", 0).contains(&child.id()));
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn system_procs_sees_itself() {
        let me = std::process::id();
        assert!(SystemProcs.is_alive(me));
        assert!(SystemProcs.comm(me).is_some());
        assert!(SystemProcs.start_ticks(me).is_some_and(|t| t <= boot_ticks_now() + clock_ticks_per_second()));
        assert!(SystemProcs.boot_id().is_some_and(|b| b.len() > 8));
        assert!(monotonic_ms() > 0);
        assert!(!SystemProcs.is_alive(u32::MAX - 1));
    }
}
