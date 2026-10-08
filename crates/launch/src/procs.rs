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
            Ok(stat) => parse_stat(&stat)
                .map(|(s, _)| s != 'Z' && s != 'X')
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    fn comm(&self, pid: u32) -> Option<String> {
        std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .ok()
            .map(|s| s.trim().to_string())
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
                if parse_stat(&stat)
                    .map(|(_, ppid)| ppid == pid)
                    .unwrap_or(false)
                {
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
        let mut v: Vec<u32> = s
            .parents
            .iter()
            .filter(|(c, p)| **p == pid && s.alive.contains(c))
            .map(|(c, _)| *c)
            .collect();
        v.sort_unstable();
        v
    }
    fn signal(&self, pid: u32, sig: Sig) {
        self.inner.lock().unwrap().signals.push((pid, sig));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stat_with_odd_comm() {
        assert_eq!(
            parse_stat("42 (my (weird) app) S 7 42 42 0"),
            Some(('S', 7))
        );
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
    fn system_procs_sees_itself() {
        let me = std::process::id();
        assert!(SystemProcs.is_alive(me));
        assert!(SystemProcs.comm(me).is_some());
        assert!(!SystemProcs.is_alive(u32::MAX - 1));
    }
}
