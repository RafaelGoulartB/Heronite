//! Finding and following a Steam game's process (port of
//! `playtime-watch.ts`): Steam logs `Game process added : AppID <id> ...
//! ProcID <pid>` in `logs/console_log.txt`; we read those lines and then
//! watch the pids in `/proc` until they are gone.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use regex::Regex;

use crate::procs::Procs;

pub const LOG_TAIL_BYTES: u64 = 64 * 1024;

struct LogRegexes {
    added: Regex,
    removed: Regex,
    removing: Regex,
}

fn regexes() -> &'static LogRegexes {
    static RE: OnceLock<LogRegexes> = OnceLock::new();
    RE.get_or_init(|| LogRegexes {
        added: Regex::new(
            r"(?i)Game process (?:added|updated)\s*:\s*AppID\s+(\d+).*ProcID\s+(\d+)",
        )
        .unwrap(),
        removed: Regex::new(r"(?i)Game process removed:\s*AppID\s+(\d+).*ProcID\s+(\d+)").unwrap(),
        removing: Regex::new(r"(?i)Removing process (\d+) for gameID (\d+)").unwrap(),
    })
}

/// Steam logs 64-bit game ids for non-Steam shortcuts; the low 32 bits are the app id.
fn matches_steam_app_id(captured: &str, steam_app_id: &str) -> bool {
    if captured == steam_app_id {
        return true;
    }
    captured
        .parse::<u64>()
        .map(|n| (n % 4_294_967_296).to_string() == steam_app_id)
        .unwrap_or(false)
}

/// `(added, removed)` pids mentioned for `steam_app_id` in a chunk of log text.
pub fn parse_steam_log_pids(chunk: &str, steam_app_id: &str) -> (Vec<u32>, Vec<u32>) {
    let re = regexes();
    let mut added = Vec::new();
    let mut removed = Vec::new();
    for c in re.added.captures_iter(chunk) {
        if matches_steam_app_id(&c[1], steam_app_id) {
            if let Ok(pid) = c[2].parse() {
                added.push(pid);
            }
        }
    }
    for c in re.removed.captures_iter(chunk) {
        if matches_steam_app_id(&c[1], steam_app_id) {
            if let Ok(pid) = c[2].parse() {
                removed.push(pid);
            }
        }
    }
    for c in re.removing.captures_iter(chunk) {
        if matches_steam_app_id(&c[2], steam_app_id) {
            if let Ok(pid) = c[1].parse() {
                removed.push(pid);
            }
        }
    }
    (added, removed)
}

/// Pids tracked for one Steam game.
#[derive(Debug, Clone)]
pub struct SteamWatch {
    app_id: String,
    self_pid: u32,
    pids: BTreeSet<u32>,
    started: bool,
}

/// Steam helper processes that are never the game.
fn is_steam_helper(comm: &str) -> bool {
    matches!(
        comm.to_lowercase().as_str(),
        "steam" | "steamwebhelper" | "steam-runtime"
    )
}

impl SteamWatch {
    pub fn new(app_id: &str) -> Self {
        Self {
            app_id: app_id.to_string(),
            self_pid: std::process::id(),
            pids: BTreeSet::new(),
            started: false,
        }
    }

    fn ignorable(&self, pid: u32, procs: &dyn Procs) -> bool {
        pid == self.self_pid
            || !procs.is_alive(pid)
            || procs
                .comm(pid)
                .map(|c| is_steam_helper(&c))
                .unwrap_or(false)
    }

    /// Applies new log text. Returns the pids that were newly tracked.
    pub fn apply_chunk(&mut self, chunk: &str, procs: &dyn Procs) -> Vec<u32> {
        let (added, removed) = parse_steam_log_pids(chunk, &self.app_id);
        let mut new = Vec::new();
        for pid in added {
            if self.ignorable(pid, procs) {
                continue;
            }
            if self.pids.insert(pid) {
                new.push(pid);
            }
            self.started = true;
        }
        for pid in removed {
            self.pids.remove(&pid);
        }
        new
    }

    pub fn prune_dead(&mut self, procs: &dyn Procs) {
        self.pids.retain(|p| procs.is_alive(*p));
    }

    pub fn pids(&self) -> &BTreeSet<u32> {
        &self.pids
    }

    /// A game process was seen at some point.
    pub fn started(&self) -> bool {
        self.started
    }

    /// Seen and all its processes are gone.
    pub fn finished(&self) -> bool {
        self.started && self.pids.is_empty()
    }
}

/// Reads whatever is appended to a file after it was opened.
#[derive(Debug)]
pub struct LogTail {
    path: PathBuf,
    offset: u64,
}

fn read_range(path: &Path, start: u64, len: u64) -> Option<String> {
    let mut f = fs::File::open(path).ok()?;
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::with_capacity(len as usize);
    f.take(len).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

impl LogTail {
    /// Starts following `path`; also returns its last 64 KiB (what happened
    /// just before we started looking), like `readFileTail`.
    pub fn open(path: &Path) -> (Self, String) {
        let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let start = size.saturating_sub(LOG_TAIL_BYTES);
        let initial = if size > start {
            read_range(path, start, size - start).unwrap_or_default()
        } else {
            String::new()
        };
        (
            Self {
                path: path.to_path_buf(),
                offset: size,
            },
            initial,
        )
    }

    /// New text since the last call. A truncated/rotated file restarts at 0.
    pub fn read_new(&mut self) -> Option<String> {
        let size = fs::metadata(&self.path).ok()?.len();
        if size < self.offset {
            self.offset = 0;
        }
        if size == self.offset {
            return None;
        }
        let text = read_range(&self.path, self.offset, size - self.offset)?;
        self.offset = size;
        Some(text)
    }
}

#[derive(Debug, Clone)]
pub struct SteamWaitConfig {
    /// Give up if no game process shows up in this time (Heroic: 90 s).
    pub start_timeout: Duration,
    pub poll_interval: Duration,
    /// How often to look for log files that appeared later.
    pub resolve_interval: Duration,
}

impl Default for SteamWaitConfig {
    fn default() -> Self {
        Self {
            start_timeout: Duration::from_secs(90),
            poll_interval: Duration::from_millis(500),
            resolve_interval: Duration::from_secs(2),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteamWaitOutcome {
    /// The game ran and all its processes are gone.
    Finished,
    /// No game process appeared before the timeout.
    NotStarted,
    /// Tracking was stopped (stop / force clear).
    Aborted,
}

pub fn sleep_abortable(total: Duration, abort: &AtomicBool) {
    let step = Duration::from_millis(25);
    let deadline = Instant::now() + total;
    while !abort.load(Ordering::SeqCst) {
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        std::thread::sleep(step.min(deadline - now));
    }
}

/// Blocks until the Steam game started via `steam://rungameid/<id>` exits.
///
/// `log_files` is called now and then to discover Steam's logs; `shared`
/// mirrors the tracked pids (so a caller can kill them); `on_first_pid` fires
/// once, when the game process is first seen.
pub fn wait_for_steam_session(
    app_id: &str,
    procs: &dyn Procs,
    log_files: &dyn Fn() -> Vec<PathBuf>,
    cfg: &SteamWaitConfig,
    abort: &AtomicBool,
    shared: &Mutex<BTreeSet<u32>>,
    on_first_pid: &mut dyn FnMut(u32),
) -> SteamWaitOutcome {
    let mut watch = SteamWatch::new(app_id);
    let mut tails: HashMap<PathBuf, LogTail> = HashMap::new();
    let mut announced = false;

    let attach = |watch: &mut SteamWatch,
                  tails: &mut HashMap<PathBuf, LogTail>,
                  files: Vec<PathBuf>|
     -> Vec<u32> {
        let mut new = Vec::new();
        for f in files {
            if tails.contains_key(&f) {
                continue;
            }
            let (tail, initial) = LogTail::open(&f);
            new.extend(watch.apply_chunk(&initial, procs));
            tails.insert(f, tail);
        }
        new
    };
    let sync = |watch: &SteamWatch,
                new: &[u32],
                announced: &mut bool,
                on_first_pid: &mut dyn FnMut(u32)| {
        *shared.lock().unwrap() = watch.pids().clone();
        if !*announced {
            if let Some(pid) = new.first().or_else(|| watch.pids().iter().next()) {
                *announced = true;
                on_first_pid(*pid);
            }
        }
    };

    let new = attach(&mut watch, &mut tails, log_files());
    sync(&watch, &new, &mut announced, on_first_pid);

    let deadline = Instant::now() + cfg.start_timeout;
    let mut last_resolve = Instant::now();
    while !watch.started() {
        if abort.load(Ordering::SeqCst) {
            return SteamWaitOutcome::Aborted;
        }
        if Instant::now() >= deadline {
            return SteamWaitOutcome::NotStarted;
        }
        sleep_abortable(cfg.poll_interval, abort);
        let mut new = Vec::new();
        for t in tails.values_mut() {
            if let Some(chunk) = t.read_new() {
                new.extend(watch.apply_chunk(&chunk, procs));
            }
        }
        if last_resolve.elapsed() >= cfg.resolve_interval {
            last_resolve = Instant::now();
            new.extend(attach(&mut watch, &mut tails, log_files()));
        }
        sync(&watch, &new, &mut announced, on_first_pid);
    }

    loop {
        if abort.load(Ordering::SeqCst) {
            return SteamWaitOutcome::Aborted;
        }
        watch.prune_dead(procs);
        *shared.lock().unwrap() = watch.pids().clone();
        if watch.finished() {
            return SteamWaitOutcome::Finished;
        }
        sleep_abortable(cfg.poll_interval, abort);
        let mut new = Vec::new();
        for t in tails.values_mut() {
            if let Some(chunk) = t.read_new() {
                new.extend(watch.apply_chunk(&chunk, procs));
            }
        }
        sync(&watch, &new, &mut announced, on_first_pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procs::FakeProcs;
    use std::io::Write;
    use std::sync::Arc;

    #[test]
    fn parses_all_three_line_shapes() {
        let log = "\
[2026-10-07 10:00:00] Game process added : AppID 400 \"/home/u/.steam/ubuntu12_32/reaper\" ProcID 4242, IP 0.0.0.0\n\
[2026-10-07 10:00:01] Game process updated : AppID 400 something ProcID 4243\n\
[2026-10-07 10:00:02] Game process added : AppID 999 other ProcID 1\n\
[2026-10-07 10:05:00] Game process removed: AppID 400 whatever ProcID 4242\n\
[2026-10-07 10:05:00] Removing process 4243 for gameID 400\n\
[2026-10-07 10:05:00] Removing process 77 for gameID 12\n";
        let (added, removed) = parse_steam_log_pids(log, "400");
        assert_eq!(added, vec![4242, 4243]);
        assert_eq!(removed, vec![4242, 4243]);
        // 64-bit ids (non-Steam shortcuts) reduce to their low 32 bits.
        let (a, _) =
            parse_steam_log_pids("Game process added : AppID 4294967696 x ProcID 5", "400");
        assert_eq!(a, vec![5]);
    }

    #[test]
    fn watch_ignores_dead_helpers_and_self() {
        let procs = FakeProcs::new();
        procs.start(10, "steamwebhelper", None);
        procs.start(11, "reaper", None);
        procs.start(std::process::id(), "me", None);
        let mut w = SteamWatch::new("400");
        let line = |pid: u32| format!("Game process added : AppID 400 x ProcID {pid}\n");
        assert!(w.apply_chunk(&line(10), &procs).is_empty());
        assert!(w.apply_chunk(&line(12), &procs).is_empty(), "dead pid");
        assert!(w.apply_chunk(&line(std::process::id()), &procs).is_empty());
        assert!(!w.started());
        assert_eq!(w.apply_chunk(&line(11), &procs), vec![11]);
        assert!(w.started() && !w.finished());
        procs.exit(11);
        w.prune_dead(&procs);
        assert!(w.finished());
    }

    #[test]
    fn log_tail_reads_appended_text_and_handles_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("console_log.txt");
        fs::write(&p, "old line\n").unwrap();
        let (mut tail, initial) = LogTail::open(&p);
        assert_eq!(initial, "old line\n");
        assert_eq!(tail.read_new(), None);
        fs::OpenOptions::new()
            .append(true)
            .open(&p)
            .unwrap()
            .write_all(b"new line\n")
            .unwrap();
        assert_eq!(tail.read_new().as_deref(), Some("new line\n"));
        fs::write(&p, "x\n").unwrap();
        assert_eq!(tail.read_new().as_deref(), Some("x\n"));
        let (_, none) = LogTail::open(&dir.path().join("missing"));
        assert!(none.is_empty());
    }

    fn fast_cfg() -> SteamWaitConfig {
        SteamWaitConfig {
            start_timeout: Duration::from_secs(3),
            poll_interval: Duration::from_millis(10),
            resolve_interval: Duration::from_millis(20),
        }
    }

    #[test]
    fn follows_a_session_from_log_line_to_exit() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("console_log.txt");
        fs::write(&log, "Game process added : AppID 400 stale ProcID 999\n").unwrap();
        let procs = Arc::new(FakeProcs::new());
        let abort = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Mutex::new(BTreeSet::new()));

        let (p2, a2, s2, l2) = (procs.clone(), abort.clone(), shared.clone(), log.clone());
        let handle = std::thread::spawn(move || {
            let mut first = None;
            let out = wait_for_steam_session(
                "400",
                &*p2,
                &move || vec![l2.clone()],
                &fast_cfg(),
                &a2,
                &s2,
                &mut |pid| first = Some(pid),
            );
            (out, first)
        });

        std::thread::sleep(Duration::from_millis(60));
        procs.start(4242, "reaper", None);
        fs::OpenOptions::new()
            .append(true)
            .open(&log)
            .unwrap()
            .write_all(b"Game process added : AppID 400 x ProcID 4242\n")
            .unwrap();
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(
            shared.lock().unwrap().iter().copied().collect::<Vec<_>>(),
            vec![4242]
        );
        procs.exit(4242);
        let (out, first) = handle.join().unwrap();
        assert_eq!(out, SteamWaitOutcome::Finished);
        assert_eq!(first, Some(4242));
    }

    #[test]
    fn gives_up_when_no_process_appears_and_honours_abort() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("console_log.txt");
        fs::write(&log, "").unwrap();
        let procs = FakeProcs::new();
        let abort = AtomicBool::new(false);
        let shared = Mutex::new(BTreeSet::new());
        let cfg = SteamWaitConfig {
            start_timeout: Duration::from_millis(80),
            ..fast_cfg()
        };
        let l = log.clone();
        let out = wait_for_steam_session(
            "400",
            &procs,
            &move || vec![l.clone()],
            &cfg,
            &abort,
            &shared,
            &mut |_| {},
        );
        assert_eq!(out, SteamWaitOutcome::NotStarted);

        abort.store(true, Ordering::SeqCst);
        let l = log.clone();
        let out = wait_for_steam_session(
            "400",
            &procs,
            &move || vec![l.clone()],
            &fast_cfg(),
            &abort,
            &shared,
            &mut |_| {},
        );
        assert_eq!(out, SteamWaitOutcome::Aborted);
    }
}
