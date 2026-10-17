//! Reading and signalling processes, behind a trait so Steam tracking can be
//! tested without real processes.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sig {
    Term,
    Kill,
}

pub trait Procs: Send + Sync {
    /// Alive and not a zombie.
    fn is_alive(&self, pid: u32) -> bool;
    /// Short command name (`/proc/<pid>/comm`).
    fn comm(&self, pid: u32) -> Option<String>;
    /// Direct children.
    fn children(&self, pid: u32) -> Vec<u32>;
    fn signal(&self, pid: u32, sig: Sig);
    /// File names (lowercase, `game.exe`) of the Windows programs running in
    /// the Wine prefix `prefix`.
    fn windows_programs(&self, prefix: &str) -> Vec<String>;
}

/// Whether the Wine prefix variable of a process (`WINEPREFIX`) is `prefix`
/// (Proton keeps the Wine prefix in `<prefix>/pfx`).
fn same_prefix(wineprefix: &str, prefix: &str) -> bool {
    let a = wineprefix.trim_end_matches('/');
    let b = prefix.trim_end_matches('/');
    !b.is_empty() && (a == b || a.strip_prefix(b).is_some_and(|rest| rest == "/pfx"))
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

/// Whether a program name (`/proc` comm, or the file name of a command's
/// first words) is what a stop pattern names: the pattern, maybe followed by
/// a version, an extension or a variant (`pcsx2-qt`, `Game.exe`,
/// `PPSSPPSDL`). Never a mere substring: `game` does not name `gamescope`.
pub fn program_matches(name: &str, pattern: &str) -> bool {
    let (name, pattern) = (name.as_bytes(), pattern.as_bytes());
    if pattern.is_empty() || name.len() < pattern.len() || !name[..pattern.len()].eq_ignore_ascii_case(pattern) {
        return false;
    }
    match name.get(pattern.len()) {
        None => true,
        Some(c) => !c.is_ascii_alphanumeric() || c.is_ascii_uppercase(),
    }
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
    if hz > 0 { hz as u64 } else { 100 }
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

/// All descendants of `root` (children before parents), `root` excluded.
pub fn descendants(procs: &dyn Procs, root: u32) -> Vec<u32> {
    let mut order = Vec::new();
    let mut seen = HashSet::new();
    seen.insert(root);
    let mut stack = vec![root];
    while let Some(p) = stack.pop() {
        for c in procs.children(p) {
            if seen.insert(c) {
                order.push(c);
                stack.push(c);
            }
        }
    }
    order.reverse();
    order
}

/// Signals `root` and everything below it. Never touches pids <= 1 or us.
pub fn signal_tree(procs: &dyn Procs, root: u32, sig: Sig) {
    let me = std::process::id();
    let mut all = descendants(procs, root);
    all.push(root);
    for pid in all {
        if pid > 1 && pid != me {
            procs.signal(pid, sig);
        }
    }
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
            // Other users' processes are not readable: they are not ours anyway.
            let Ok(environ) = std::fs::read(format!("/proc/{pid}/environ")) else { continue };
            let in_prefix = environ
                .split(|b| *b == 0)
                .filter_map(|kv| kv.strip_prefix(b"WINEPREFIX="))
                .any(|v| same_prefix(&String::from_utf8_lossy(v), prefix));
            if in_prefix && self.is_alive(pid) {
                out.push(name);
            }
        }
        out
    }
}

/// Scriptable process table for tests.
#[derive(Debug, Default)]
pub struct FakeProcs {
    inner: Mutex<FakeState>,
}

#[derive(Debug, Default)]
struct FakeState {
    alive: BTreeSet<u32>,
    comms: HashMap<u32, String>,
    parents: HashMap<u32, u32>,
    signals: Vec<(u32, Sig)>,
    windows: HashMap<String, Vec<String>>,
}

impl FakeProcs {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn start(&self, pid: u32, comm: &str, parent: Option<u32>) {
        let mut s = self.inner.lock().unwrap();
        s.alive.insert(pid);
        s.comms.insert(pid, comm.to_string());
        if let Some(p) = parent {
            s.parents.insert(pid, p);
        }
    }
    pub fn exit(&self, pid: u32) {
        self.inner.lock().unwrap().alive.remove(&pid);
    }
    pub fn signals(&self) -> Vec<(u32, Sig)> {
        self.inner.lock().unwrap().signals.clone()
    }
    /// What [`Procs::windows_programs`] reports for `prefix`.
    pub fn set_windows_programs(&self, prefix: &str, names: &[&str]) {
        self.inner.lock().unwrap().windows.insert(prefix.to_string(), names.iter().map(|n| n.to_string()).collect());
    }
}

impl Procs for FakeProcs {
    fn is_alive(&self, pid: u32) -> bool {
        self.inner.lock().unwrap().alive.contains(&pid)
    }
    fn comm(&self, pid: u32) -> Option<String> {
        self.inner.lock().unwrap().comms.get(&pid).cloned()
    }
    fn children(&self, pid: u32) -> Vec<u32> {
        let s = self.inner.lock().unwrap();
        let mut v: Vec<u32> = s.parents.iter().filter(|(c, p)| **p == pid && s.alive.contains(c)).map(|(c, _)| *c).collect();
        v.sort_unstable();
        v
    }
    fn signal(&self, pid: u32, sig: Sig) {
        self.inner.lock().unwrap().signals.push((pid, sig));
    }
    fn windows_programs(&self, prefix: &str) -> Vec<String> {
        self.inner.lock().unwrap().windows.get(prefix).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parses_stat_with_odd_comm() {
        assert_eq!(parse_stat("42 (my (weird) app) S 7 42 42 0"), Some(('S', 7)));
        assert_eq!(parse_stat("42 (x) Z 1 0"), Some(('Z', 1)));
    }

    #[test]
    fn tree_signalling_goes_leaves_first() {
        let p = FakeProcs::new();
        p.start(100, "sh", None);
        p.start(101, "wine", Some(100));
        p.start(102, "game", Some(101));
        signal_tree(&p, 100, Sig::Term);
        let order: Vec<u32> = p.signals().iter().map(|(pid, _)| *pid).collect();
        assert_eq!(order, vec![102, 101, 100]);
    }

    #[test]
    fn stop_patterns_name_programs_not_substrings() {
        for (name, pattern) in [
            ("pcsx2-qt", "pcsx2"),
            ("Game.exe", "game.exe"),
            ("Calluna.exe", "Calluna"),
            ("PPSSPPSDL", "PPSSPP"),
            ("retroarch", "RetroArch"),
            ("pcsx2-v2.8.2.AppImage", "pcsx2"),
        ] {
            assert!(program_matches(name, pattern), "{name} / {pattern}");
        }
        for (name, pattern) in [("gamescope", "game"), ("runuser", "run"), ("steam", "steamwebhelper"), ("x", "")] {
            assert!(!program_matches(name, pattern), "{name} / {pattern}");
        }
        let names = program_names("bash\n", b"/bin/bash\0/home/u/run.sh\0--fast\0");
        assert_eq!(names, ["bash", "bash", "run.sh"]);
        assert_eq!(program_names("Game.exe", b"Z:\\games\\Game.exe\0")[1], "Game.exe");
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
        assert!(!SystemProcs.is_alive(u32::MAX - 1));
    }
}
