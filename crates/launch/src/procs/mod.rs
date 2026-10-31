//! Reading and signalling processes, behind a trait so Steam tracking can be
//! tested without real processes. [`SystemProcs`] reads `/proc` on Linux
//! (`linux.rs`) and a process snapshot on Windows (`windows.rs`).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Mutex;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{boot_ticks_now, processes_matching, SystemProcs};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{boot_ticks_now, processes_matching, SystemProcs};

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
    /// Every live process with its parent, read at once (one pass over
    /// `/proc`, where [`Procs::children`] makes one per process).
    fn parents(&self) -> HashMap<u32, u32>;
    fn signal(&self, pid: u32, sig: Sig);
    /// File names (lowercase, `game.exe`) of the Windows programs running in
    /// the Wine prefix `prefix`.
    fn windows_programs(&self, prefix: &str) -> Vec<String>;
    /// When the process started, in clock ticks since boot (see
    /// [`boot_ticks_now`]). With the pid and the boot, it names one process:
    /// a pid reused later starts later.
    fn start_ticks(&self, pid: u32) -> Option<u64>;
    /// Whether any process of group `pgid` is left. The kernel keeps a group's
    /// id from being given to a new process while the group lives.
    fn group_alive(&self, pgid: u32) -> bool;
    /// Signals every process of group `pgid`.
    fn signal_group(&self, pgid: u32, sig: Sig);
    /// This boot's id (`/proc/sys/kernel/random/boot_id`): pids and start
    /// ticks mean nothing across a reboot.
    fn boot_id(&self) -> Option<String>;
    /// Processes of a game started by another program: running a file
    /// under `folder`, or a Windows program (not Wine's own) in `prefix` or
    /// from `folder`. Never this process.
    fn game_pids(&self, prefix: Option<&str>, folder: Option<&str>) -> Vec<u32>;
    /// The user namespace of a process (`user:[4026531837]`), which tells a
    /// sandboxed one (see [`crate::sandbox::find_sandbox`]).
    fn user_ns(&self, pid: u32) -> Option<String>;
    /// The Windows programs (not Wine's own, see [`WINE_SERVICES`]) running in
    /// the Wine prefix `prefix` or from `folder`. A store client's game is
    /// told from the client by name and by where it runs from.
    fn prefix_programs(&self, prefix: &str, folder: Option<&str>) -> Vec<WinProgram>;
}

/// A Windows program running in a Wine prefix (see [`Procs::prefix_programs`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WinProgram {
    pub pid: u32,
    /// Lowercase file name (`game.exe`).
    pub name: String,
    /// The Windows path Wine gives it (`C:\games\Game.exe`).
    pub path: String,
    /// Its working folder is the folder asked for, or inside it.
    pub from_folder: bool,
}

/// Whether the Windows path `win` of a program in `prefix` is inside the
/// Linux folder `folder`: `C:` is the prefix's drive (`pfx/drive_c` with
/// Proton), `Z:` the root, as Wine sets them up. Wine's paths ignore case.
pub fn windows_path_under(win: &str, prefix: &str, folder: &str) -> bool {
    let b = win.as_bytes();
    if b.len() < 3 || b[1] != b':' || !matches!(b[2], b'\\' | b'/') {
        return false;
    }
    let rest = win[3..].replace('\\', "/").to_lowercase();
    let prefix = prefix.trim_end_matches('/');
    let roots: Vec<String> = match b[0].to_ascii_lowercase() {
        b'c' => vec![format!("{prefix}/pfx/drive_c"), format!("{prefix}/drive_c")],
        b'z' => vec![String::new()],
        _ => return false,
    };
    let folder = format!("{}/", folder.trim_end_matches('/').to_lowercase());
    roots.iter().any(|root| format!("{}/{rest}", root.to_lowercase()).starts_with(&folder))
}

/// Wine's own programs, running in every prefix in use, and Proton's
/// `steam.exe`, which starts the game and stays after it.
pub(crate) const WINE_SERVICES: &[&str] = &[
    "steam.exe",
    "services.exe",
    "winedevice.exe",
    "plugplay.exe",
    "svchost.exe",
    "explorer.exe",
    "rpcss.exe",
    "conhost.exe",
    "tabtip.exe",
    "wineboot.exe",
    "winemenubuilder.exe",
    "start.exe",
    // Proton's accessibility helper (`PROTON_USE_XALIA`).
    "xalia.exe",
];

/// Whether the Wine prefix variable of a process (`WINEPREFIX`) is `prefix`
/// (Proton keeps the Wine prefix in `<prefix>/pfx`).
pub(crate) fn same_prefix(wineprefix: &str, prefix: &str) -> bool {
    let a = wineprefix.trim_end_matches('/');
    let b = prefix.trim_end_matches('/');
    !b.is_empty() && (a == b || a.strip_prefix(b).is_some_and(|rest| rest == "/pfx"))
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

/// Milliseconds of a clock that survives a restart of the app (same boot
/// only): `heronite_platform::process::monotonic_ms`.
pub fn monotonic_ms() -> u64 {
    heronite_platform::process::monotonic_ms()
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
    group_signals: Vec<(u32, Sig)>,
    windows: HashMap<String, Vec<String>>,
    /// Start ticks; 0 for a process started without one.
    ticks: HashMap<u32, u64>,
    /// Process group of a pid; its own pid when not set.
    groups: HashMap<u32, u32>,
    /// `None`: "fake-boot".
    boot: Option<String>,
    /// What [`Procs::game_pids`] reports (alive ones only).
    game: Vec<u32>,
    /// User namespace of a pid; "host" when not set.
    user_ns: HashMap<u32, String>,
    /// What [`Procs::prefix_programs`] reports (alive ones only).
    programs: Vec<WinProgram>,
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
    /// The processes [`Procs::game_pids`] finds (while alive).
    pub fn set_game_pids(&self, pids: &[u32]) {
        self.inner.lock().unwrap().game = pids.to_vec();
    }
    /// What [`Procs::windows_programs`] reports for `prefix`.
    pub fn set_windows_programs(&self, prefix: &str, names: &[&str]) {
        self.inner.lock().unwrap().windows.insert(prefix.to_string(), names.iter().map(|n| n.to_string()).collect());
    }
    pub fn set_start_ticks(&self, pid: u32, ticks: u64) {
        self.inner.lock().unwrap().ticks.insert(pid, ticks);
    }
    /// Puts `pid` in process group `pgid`.
    pub fn set_group(&self, pid: u32, pgid: u32) {
        self.inner.lock().unwrap().groups.insert(pid, pgid);
    }
    pub fn set_boot_id(&self, id: &str) {
        self.inner.lock().unwrap().boot = Some(id.to_string());
    }
    pub fn group_signals(&self) -> Vec<(u32, Sig)> {
        self.inner.lock().unwrap().group_signals.clone()
    }
    /// Puts `pid` in another user namespace (a sandbox).
    pub fn set_user_ns(&self, pid: u32, ns: &str) {
        self.inner.lock().unwrap().user_ns.insert(pid, ns.to_string());
    }
    /// The programs [`Procs::prefix_programs`] finds, whatever the prefix
    /// (while alive), each running from the folder asked for.
    pub fn set_prefix_programs(&self, programs: &[(u32, &str)]) {
        let programs = programs
            .iter()
            .map(|(pid, name)| WinProgram { pid: *pid, name: name.to_string(), path: format!("C:\\games\\{name}"), from_folder: true })
            .collect();
        self.set_win_programs(programs);
    }
    /// [`set_prefix_programs`](Self::set_prefix_programs) with every detail.
    pub fn set_win_programs(&self, programs: Vec<WinProgram>) {
        self.inner.lock().unwrap().programs = programs;
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
    fn parents(&self) -> HashMap<u32, u32> {
        let s = self.inner.lock().unwrap();
        s.parents.iter().filter(|(c, _)| s.alive.contains(c)).map(|(c, p)| (*c, *p)).collect()
    }
    fn signal(&self, pid: u32, sig: Sig) {
        self.inner.lock().unwrap().signals.push((pid, sig));
    }
    fn windows_programs(&self, prefix: &str) -> Vec<String> {
        self.inner.lock().unwrap().windows.get(prefix).cloned().unwrap_or_default()
    }
    fn start_ticks(&self, pid: u32) -> Option<u64> {
        let s = self.inner.lock().unwrap();
        s.alive.contains(&pid).then(|| s.ticks.get(&pid).copied().unwrap_or(0))
    }
    fn group_alive(&self, pgid: u32) -> bool {
        let s = self.inner.lock().unwrap();
        s.alive.iter().any(|p| s.groups.get(p).copied().unwrap_or(*p) == pgid)
    }
    fn signal_group(&self, pgid: u32, sig: Sig) {
        self.inner.lock().unwrap().group_signals.push((pgid, sig));
    }
    fn boot_id(&self) -> Option<String> {
        Some(self.inner.lock().unwrap().boot.clone().unwrap_or_else(|| "fake-boot".into()))
    }

    fn game_pids(&self, _prefix: Option<&str>, _folder: Option<&str>) -> Vec<u32> {
        let s = self.inner.lock().unwrap();
        s.game.iter().copied().filter(|p| s.alive.contains(p)).collect()
    }

    fn user_ns(&self, pid: u32) -> Option<String> {
        Some(self.inner.lock().unwrap().user_ns.get(&pid).cloned().unwrap_or_else(|| "host".into()))
    }

    fn prefix_programs(&self, _prefix: &str, _folder: Option<&str>) -> Vec<WinProgram> {
        let s = self.inner.lock().unwrap();
        s.programs.iter().filter(|p| s.alive.contains(&p.pid)).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_windows_path_is_under_a_folder_of_the_prefix() {
        let wow = "/pfx/bnet/pfx/drive_c/Program Files (x86)/World of Warcraft";
        assert!(windows_path_under("C:\\Program Files (x86)\\World of Warcraft\\_retail_\\Wow.exe", "/pfx/bnet", wow));
        assert!(windows_path_under("c:/program files (x86)/world of warcraft/Wow.exe", "/pfx/bnet/", wow));
        assert!(!windows_path_under("C:\\Program Files (x86)\\World of Warcraft Classic\\Wow.exe", "/pfx/bnet", wow));
        assert!(!windows_path_under("C:\\Program Files (x86)\\Battle.net\\Battle.net.exe", "/pfx/bnet", wow));
        assert!(windows_path_under("Z:\\games\\wow\\Wow.exe", "/pfx/bnet", "/games/wow"));
        assert!(!windows_path_under("D:\\games\\wow\\Wow.exe", "/pfx/bnet", "/games/wow"));
        assert!(!windows_path_under("Wow.exe", "/pfx/bnet", "/games/wow"));
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
    }
}
