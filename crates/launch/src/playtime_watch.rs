//! Finding and following a Steam game's process (port of
//! `playtime-watch.ts`): Steam logs `Game process added : AppID <id> ...
//! ProcID <pid>` in `logs/console_log.txt`; we read those lines and then
//! watch the pids in `/proc` until they are gone.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use regex::Regex;

use crate::procs::Procs;
use crate::wake::Wake;

pub const LOG_TAIL_BYTES: u64 = 64 * 1024;

struct LogRegexes {
    added: Regex,
    removed: Regex,
    removing: Regex,
    action_task: Regex,
    action_waiting: Regex,
    action_continues: Regex,
    action_failed: Regex,
    content_state: Regex,
    content_update: Regex,
    content_ended: Regex,
}

fn regexes() -> &'static LogRegexes {
    static RE: OnceLock<LogRegexes> = OnceLock::new();
    RE.get_or_init(|| LogRegexes {
        added: Regex::new(r"(?i)Game process (?:added|updated)\s*:\s*AppID\s+(\d+).*ProcID\s+(\d+)").unwrap(),
        removed: Regex::new(r"(?i)Game process removed:\s*AppID\s+(\d+).*ProcID\s+(\d+)").unwrap(),
        removing: Regex::new(r"(?i)Removing process (\d+) for gameID (\d+)").unwrap(),
        action_task: Regex::new(
            r"(?i)GameAction\s*\[AppID\s+(\d+),\s*ActionID\s+(\d+)\]\s*:\s*LaunchApp\s+changed task to\s+([A-Za-z0-9_]+)\s+with\b",
        )
        .unwrap(),
        action_waiting: Regex::new(
            r"(?i)GameAction\s*\[AppID\s+(\d+),\s*ActionID\s+(\d+)\]\s*:\s*LaunchApp\s+waiting for user response to\s+([A-Za-z0-9_]+)",
        )
        .unwrap(),
        action_continues: Regex::new(
            r#"(?i)GameAction\s*\[AppID\s+(\d+),\s*ActionID\s+(\d+)\]\s*:\s*LaunchApp\s+continues with user response\s+"([^"]*)""#,
        )
        .unwrap(),
        action_failed: Regex::new(
            r"(?i)GameAction\s*\[AppID\s+(\d+),\s*ActionID\s+(\d+)\]\s*:\s*LaunchApp\s+(?:failed|canceled|cancelled|aborted)\b",
        )
        .unwrap(),
        content_state: Regex::new(r"(?i)\bAppID\s+(\d+)\s+state changed\s*:\s*(.*)$").unwrap(),
        content_update: Regex::new(r"(?i)\bAppID\s+(\d+)\s+App update changed\s*:\s*(.*)$").unwrap(),
        content_ended: Regex::new(r"(?i)\bAppID\s+(\d+)\s+(?:scheduler finished|update canceled|update cancelled)\b").unwrap(),
    })
}

/// Steam logs 64-bit game ids for non-Steam shortcuts; the low 32 bits are the app id.
fn matches_steam_app_id(captured: &str, steam_app_id: &str) -> bool {
    if captured == steam_app_id {
        return true;
    }
    captured.parse::<u64>().map(|n| (n % 4_294_967_296).to_string() == steam_app_id).unwrap_or(false)
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
    matches!(comm.to_lowercase().as_str(), "steam" | "steamwebhelper" | "steam-runtime")
}

impl SteamWatch {
    pub fn new(app_id: &str) -> Self {
        Self { app_id: app_id.to_string(), self_pid: std::process::id(), pids: BTreeSet::new(), started: false }
    }

    /// A game already seen running with these pids (a session picked up again).
    pub fn resume(app_id: &str, pids: BTreeSet<u32>) -> Self {
        Self { pids, started: true, ..Self::new(app_id) }
    }

    fn ignorable(&self, pid: u32, procs: &dyn Procs) -> bool {
        pid == self.self_pid || !procs.is_alive(pid) || procs.comm(pid).map(|c| is_steam_helper(&c)).unwrap_or(false)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionPhase {
    /// `LaunchApp` is in `DownloadingDepots` (game, shader cache, Proton, or
    /// Steam Linux Runtime update).
    DownloadingDepots,
    /// `LaunchApp` is in another pre-launch task (install scripts, shader
    /// processing, cloud sync, launch option / interstitial dialogs).
    Preparing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentUpdateState {
    Unknown,
    Active,
    Ended,
}

/// Tracks whether Steam is actively preparing a launch for one app (updates,
/// shader pre-caching, runtime downloads, install scripts, cloud sync, or
/// launch dialogs) from logs written after the launch URI was sent.
#[derive(Debug, Clone)]
pub struct SteamLaunchProgress {
    app_id: String,
    action: Option<(u64, ActionPhase)>,
    last_action_id: Option<u64>,
    failed_action: Option<u64>,
    /// True once the newest `LaunchApp` action ended (failed, canceled,
    /// completed, or handed off to `CreatingProcess` / `WaitingGameWindow`),
    /// so background update or manifest flags cannot hold the timeout open.
    latest_ended: bool,
    /// Actions that already finished, failed, or reached `CreatingProcess`, so
    /// a second console log flushing older lines later cannot resurrect them.
    ended_actions: HashSet<u64>,
    content: ContentUpdateState,
    /// Other AppIDs currently in `Update Running` / `Running Update` (Proton,
    /// Steam Linux Runtime, redistributables) while `DownloadingDepots` waits.
    active_dep_updates: HashSet<String>,
    saw_activity: bool,
}

impl SteamLaunchProgress {
    pub fn new(app_id: &str) -> Self {
        Self {
            app_id: app_id.to_string(),
            action: None,
            last_action_id: None,
            failed_action: None,
            latest_ended: false,
            ended_actions: HashSet::new(),
            content: ContentUpdateState::Unknown,
            active_dep_updates: HashSet::new(),
            saw_activity: false,
        }
    }

    /// Whether any post-mark Steam log output has been observed since launch.
    pub fn saw_activity(&self) -> bool {
        self.saw_activity
    }

    /// Whether the newest `LaunchApp` action for this app failed or was canceled.
    pub fn is_failed(&self) -> bool {
        self.failed_action.is_some()
    }

    /// Whether the newest `LaunchApp` action has already ended or handed off.
    pub fn is_latest_ended(&self) -> bool {
        self.latest_ended
    }

    fn record_failure(&mut self, aid: u64, progressed: &mut bool) {
        self.ended_actions.insert(aid);
        if self.action.is_some_and(|(cur, _)| cur == aid) {
            self.action = None;
        }
        if self.last_action_id.is_none_or(|cur| aid >= cur) {
            self.last_action_id = Some(aid);
            self.failed_action = Some(aid);
            self.latest_ended = true;
            *progressed = false;
        }
    }

    fn is_stale_action(&self, aid: u64) -> bool {
        self.ended_actions.contains(&aid) || self.last_action_id.is_some_and(|cur| aid < cur)
    }

    /// Parses newly written console log text (`console_log.txt`,
    /// `console-linux.txt`, `console-windows.txt`). Returns `true` when Steam
    /// reported forward launch progress for this app.
    pub fn apply_console_chunk(&mut self, chunk: &str) -> bool {
        if !chunk.is_empty() {
            self.saw_activity = true;
        }
        let re = regexes();
        let mut progressed = false;
        for line in chunk.lines() {
            if let Some(c) = re.action_task.captures(line) {
                if !matches_steam_app_id(&c[1], &self.app_id) {
                    continue;
                }
                let Ok(aid) = c[2].parse::<u64>() else { continue };
                match c[3].to_ascii_lowercase().as_str() {
                    "failed" | "canceled" | "cancelled" | "aborted" => self.record_failure(aid, &mut progressed),
                    "completed" | "waitinggamewindow" | "none" => {
                        self.ended_actions.insert(aid);
                        if self.action.is_some_and(|(cur, _)| cur == aid) {
                            self.action = None;
                        }
                        if self.last_action_id.is_none_or(|cur| aid >= cur) {
                            self.last_action_id = Some(aid);
                            self.latest_ended = true;
                        }
                    }
                    "creatingprocess" => {
                        if self.is_stale_action(aid) {
                            continue;
                        }
                        // Steam is about to spawn the game: end any long-running
                        // prep phase so a silent spawn failure still times out,
                        // while granting a fresh window for the process to appear.
                        self.ended_actions.insert(aid);
                        if self.action.is_some_and(|(cur, _)| cur == aid) {
                            self.action = None;
                        }
                        self.last_action_id = Some(aid);
                        self.latest_ended = true;
                        if self.failed_action.is_some_and(|f| aid > f) {
                            self.failed_action = None;
                        }
                        progressed = true;
                    }
                    "downloadingdepots" => {
                        if self.is_stale_action(aid) {
                            continue;
                        }
                        if self.last_action_id.is_some_and(|prev| aid > prev) && self.content == ContentUpdateState::Ended {
                            self.content = ContentUpdateState::Unknown;
                        }
                        self.last_action_id = Some(aid);
                        self.latest_ended = false;
                        if self.failed_action.is_some_and(|f| aid > f) {
                            self.failed_action = None;
                        }
                        self.action = Some((aid, ActionPhase::DownloadingDepots));
                        progressed = true;
                    }
                    _ => {
                        if self.is_stale_action(aid) {
                            continue;
                        }
                        self.last_action_id = Some(aid);
                        self.latest_ended = false;
                        if self.failed_action.is_some_and(|f| aid > f) {
                            self.failed_action = None;
                        }
                        self.action = Some((aid, ActionPhase::Preparing));
                        progressed = true;
                    }
                }
            } else if let Some(c) = re.action_failed.captures(line) {
                if matches_steam_app_id(&c[1], &self.app_id) {
                    if let Ok(aid) = c[2].parse::<u64>() {
                        self.record_failure(aid, &mut progressed);
                    }
                }
            } else if let Some(c) = re.action_waiting.captures(line) {
                if matches_steam_app_id(&c[1], &self.app_id) {
                    if let Ok(aid) = c[2].parse::<u64>() {
                        if !self.is_stale_action(aid) && !c[3].eq_ignore_ascii_case("creatingprocess") {
                            if self.action.is_none_or(|(cur, _)| cur != aid) {
                                self.action = Some((aid, ActionPhase::Preparing));
                            }
                            self.last_action_id = Some(aid);
                            self.latest_ended = false;
                            if self.failed_action.is_some_and(|f| aid > f) {
                                self.failed_action = None;
                            }
                            progressed = true;
                        }
                    }
                }
            } else if let Some(c) = re.action_continues.captures(line) {
                if matches_steam_app_id(&c[1], &self.app_id) {
                    if let Ok(aid) = c[2].parse::<u64>() {
                        if matches!(c[3].to_ascii_lowercase().as_str(), "cancel" | "canceled" | "cancelled") {
                            self.record_failure(aid, &mut progressed);
                        } else if !self.is_stale_action(aid) {
                            progressed = true;
                        }
                    }
                }
            }
        }
        progressed
    }

    /// Parses newly written `content_log.txt` text. Returns `true` when Steam
    /// reported the app (or a dependency while in `DownloadingDepots`) entering
    /// or advancing an active update.
    pub fn apply_content_chunk(&mut self, chunk: &str) -> bool {
        if !chunk.is_empty() {
            self.saw_activity = true;
        }
        let re = regexes();
        let mut progressed = false;
        for line in chunk.lines() {
            if let Some(c) = re.content_state.captures(line) {
                let lower = c[2].to_ascii_lowercase();
                let stopped = lower.contains("suspended")
                    || lower.contains("paused")
                    || lower.contains("stopping")
                    || lower.contains("uninstalling")
                    || lower.contains("terminating");
                let running_now = !stopped && (lower.contains("update running") || lower.contains("moving folder"));
                let queued_now =
                    !stopped && !lower.contains('(') && (lower.contains("update queued") || lower.contains("prefetching info"));
                if matches_steam_app_id(&c[1], &self.app_id) {
                    let active = running_now || queued_now;
                    self.content = if active { ContentUpdateState::Active } else { ContentUpdateState::Ended };
                    if !self.latest_ended {
                        progressed = active;
                    }
                } else if running_now {
                    self.active_dep_updates.insert(c[1].to_string());
                    if !self.latest_ended && matches!(self.action, Some((_, ActionPhase::DownloadingDepots))) {
                        progressed = true;
                    }
                } else {
                    self.active_dep_updates.remove(&c[1]);
                }
            } else if let Some(c) = re.content_update.captures(line) {
                let lower = c[2].to_ascii_lowercase();
                let active = lower.contains("running update")
                    && !lower.contains("stopping")
                    && !lower.contains("paused")
                    && !lower.contains("suspended");
                if matches_steam_app_id(&c[1], &self.app_id) {
                    self.content = if active { ContentUpdateState::Active } else { ContentUpdateState::Ended };
                    if !self.latest_ended {
                        progressed = active;
                    }
                } else if active {
                    self.active_dep_updates.insert(c[1].to_string());
                    if !self.latest_ended && matches!(self.action, Some((_, ActionPhase::DownloadingDepots))) {
                        progressed = true;
                    }
                } else {
                    self.active_dep_updates.remove(&c[1]);
                }
            } else if let Some(c) = re.content_ended.captures(line) {
                if matches_steam_app_id(&c[1], &self.app_id) {
                    self.content = ContentUpdateState::Ended;
                    progressed = false;
                } else {
                    self.active_dep_updates.remove(&c[1]);
                }
            }
        }
        progressed
    }

    /// Whether Steam's `content_log.txt` has already explicitly reported the
    /// app's update as ended/paused/stopped (so a lazily rewritten manifest
    /// should not override it).
    pub fn content_ended(&self) -> bool {
        self.content == ContentUpdateState::Ended
    }

    /// Whether Steam is actively preparing the game right now (so the idle
    /// start timeout must wait instead of giving up).
    pub fn is_preparing(&self, manifest_updating: bool) -> bool {
        if self.latest_ended {
            return false;
        }
        if self.content == ContentUpdateState::Active || (manifest_updating && self.content == ContentUpdateState::Unknown) {
            return true;
        }
        match self.action {
            Some((_, ActionPhase::Preparing)) => true,
            // Stay active in `DownloadingDepots` while the game's update has
            // not ended or a dependency update (Proton, Steam Linux Runtime,
            // redistributables) is still running. Once both finish or stop,
            // the idle timeout resumes so a dropped or paused launch still ends.
            Some((_, ActionPhase::DownloadingDepots)) => self.content != ContentUpdateState::Ended || !self.active_dep_updates.is_empty(),
            None => false,
        }
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

/// Reads complete `\n`-terminated lines in `start..start + len`, returning
/// `(text, bytes_consumed)`. A trailing partial line is left unread so the
/// next poll sees it whole instead of parsing a mid-write fragment.
fn read_complete_lines(path: &Path, start: u64, len: u64) -> Option<(String, u64)> {
    let mut f = fs::File::open(path).ok()?;
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::with_capacity(len as usize);
    f.take(len).read_to_end(&mut buf).ok()?;
    let Some(last_nl) = buf.iter().rposition(|&b| b == b'\n') else {
        return Some((String::new(), 0));
    };
    let consumed = last_nl + 1;
    Some((String::from_utf8_lossy(&buf[..consumed]).into_owned(), consumed as u64))
}

/// Where Steam's logs end at one moment: what they gain afterwards is new.
#[derive(Debug, Default)]
pub struct LogMarks(HashMap<PathBuf, u64>);

/// At most this much of a log is read after its mark.
const MARK_SCAN_BYTES: u64 = 1024 * 1024;

impl LogMarks {
    pub fn capture(files: &[PathBuf]) -> Self {
        Self(files.iter().map(|f| (f.clone(), fs::metadata(f).map(|m| m.len()).unwrap_or(0))).collect())
    }

    /// Byte offset captured for `file`, or `None` if it did not exist then.
    pub fn offset(&self, file: &Path) -> Option<u64> {
        self.0.get(file).copied()
    }

    /// Whether Steam logged getting `uri` (`ExecuteSteamURL: "<uri>"`) in
    /// `files` since the capture. A log that appeared later, or was rotated,
    /// counts from its start.
    pub fn steam_received(&self, files: &[PathBuf], uri: &str) -> bool {
        let quoted = format!("\"{uri}\"");
        files.iter().any(|f| {
            let size = fs::metadata(f).map(|m| m.len()).unwrap_or(0);
            let mark = self.0.get(f).copied().filter(|m| *m <= size).unwrap_or(0);
            let from = mark.max(size.saturating_sub(MARK_SCAN_BYTES));
            size > from
                && read_range(f, from, size - from).is_some_and(|t| t.lines().any(|l| l.contains("ExecuteSteamURL") && l.contains(&quoted)))
        })
    }
}

impl LogTail {
    /// Starts following `path`; also returns its last 64 KiB (what happened
    /// just before we started looking), like `readFileTail`.
    pub fn open(path: &Path) -> (Self, String) {
        let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let start = size.saturating_sub(LOG_TAIL_BYTES);
        let initial = if size > start { read_range(path, start, size - start).unwrap_or_default() } else { String::new() };
        (Self { path: path.to_path_buf(), offset: size }, initial)
    }

    /// Starts following `path`, returning both its last 64 KiB (for live PID
    /// discovery) and only the complete lines appended after `mark` (or from 0
    /// if the file appeared later or was rotated, capped at [`MARK_SCAN_BYTES`]).
    pub fn open_with_mark(path: &Path, mark: Option<u64>) -> (Self, String, String) {
        let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let pid_start = size.saturating_sub(LOG_TAIL_BYTES);
        let pid_initial = if size > pid_start {
            read_complete_lines(path, pid_start, size - pid_start).map(|(t, _)| t).unwrap_or_default()
        } else {
            String::new()
        };
        let from = mark.filter(|m| *m <= size).unwrap_or(0).max(size.saturating_sub(MARK_SCAN_BYTES));
        let (since_mark, offset) = if size > from {
            let (text, consumed) = read_complete_lines(path, from, size - from).unwrap_or_default();
            (text, from + consumed)
        } else {
            (String::new(), size)
        };
        (Self { path: path.to_path_buf(), offset }, pid_initial, since_mark)
    }

    /// Starts following `path`, returning only complete lines written after
    /// `mark` (or from 0 if the file appeared later or was rotated).
    pub fn open_from_mark(path: &Path, mark: Option<u64>) -> (Self, String) {
        let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let from = mark.filter(|m| *m <= size).unwrap_or(0).max(size.saturating_sub(MARK_SCAN_BYTES));
        let (initial, offset) = if size > from {
            let (text, consumed) = read_complete_lines(path, from, size - from).unwrap_or_default();
            (text, from + consumed)
        } else {
            (String::new(), size)
        };
        (Self { path: path.to_path_buf(), offset }, initial)
    }

    /// New complete lines since the last call. A truncated/rotated file
    /// restarts at 0; a trailing partial line waits for its `\n`.
    pub fn read_new(&mut self) -> Option<String> {
        let size = fs::metadata(&self.path).ok()?.len();
        if size < self.offset {
            self.offset = 0;
        }
        if size == self.offset {
            return None;
        }
        let (text, consumed) = read_complete_lines(&self.path, self.offset, size - self.offset)?;
        if consumed == 0 {
            return None;
        }
        self.offset += consumed;
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
    /// How long after asking Steam to look whether it got the URI (else it
    /// is asked again, directly).
    pub delivery_check: Duration,
    /// Added to `start_timeout` when the launch opens Steam: it may update
    /// and sign in before it starts the game.
    pub cold_start_grace: Duration,
}

impl Default for SteamWaitConfig {
    fn default() -> Self {
        Self {
            start_timeout: Duration::from_secs(90),
            poll_interval: Duration::from_millis(500),
            resolve_interval: Duration::from_secs(2),
            delivery_check: Duration::from_secs(10),
            cold_start_grace: Duration::from_secs(120),
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

/// Live Steam logs and state checked while waiting for a game to start.
pub struct SteamWaitSources<'a> {
    /// Console logs (`console_log.txt`, `console-linux.txt`, `console-windows.txt`).
    pub console_logs: &'a dyn Fn() -> Vec<PathBuf>,
    /// Content logs (`content_log.txt`) reporting updates and queuing.
    pub content_logs: &'a dyn Fn() -> Vec<PathBuf>,
    /// Console log offsets captured right before the launch URI was sent.
    pub console_marks: &'a LogMarks,
    /// Content log offsets captured right before the launch URI was sent.
    pub content_marks: &'a LogMarks,
    /// Whether `appmanifest_<id>.acf` currently reports an active update.
    pub manifest_updating: &'a dyn Fn() -> bool,
    /// Whether the Steam client process is currently running.
    pub client_running: &'a dyn Fn() -> bool,
}

/// Blocks until the Steam game started via `steam://rungameid/<id>` exits.
/// Setting `abort` must be followed by `wake.notify()`.
///
/// While Steam is actively preparing the game (downloading/staging updates,
/// shaders, or runtimes, running install scripts, syncing cloud saves, or
/// waiting on a Steam launch dialog), the start timeout waits; only when Steam
/// is idle, fails, or exits without starting the game does `cfg.start_timeout`
/// elapse. `shared` mirrors the tracked pids (so a caller can kill them);
/// `on_first_pid` fires once when the game process is first seen; `on_waiting`
/// fires on every poll until then with whether Steam is actively preparing;
/// `on_tick` fires on every poll while the game runs.
#[allow(clippy::too_many_arguments)]
pub fn wait_for_steam_session(
    app_id: &str,
    procs: &dyn Procs,
    sources: &SteamWaitSources<'_>,
    cfg: &SteamWaitConfig,
    abort: &AtomicBool,
    wake: &Wake,
    shared: &Mutex<BTreeSet<u32>>,
    on_first_pid: &mut dyn FnMut(u32),
    on_waiting: &mut dyn FnMut(bool),
    on_tick: &mut dyn FnMut(),
) -> SteamWaitOutcome {
    let mut watch = SteamWatch::new(app_id);
    let mut progress = SteamLaunchProgress::new(app_id);
    let mut tails: HashMap<PathBuf, LogTail> = HashMap::new();
    let mut content_tails: HashMap<PathBuf, LogTail> = HashMap::new();
    let mut announced = false;
    let mut cached_running = (sources.client_running)();
    let mut saw_client = cached_running;

    let attach_console = |watch: &mut SteamWatch,
                          progress: &mut SteamLaunchProgress,
                          tails: &mut HashMap<PathBuf, LogTail>,
                          files: Vec<PathBuf>|
     -> (Vec<u32>, bool) {
        let mut new = Vec::new();
        let mut progressed = false;
        for f in files {
            if tails.contains_key(&f) {
                continue;
            }
            let (tail, pid_initial, progress_initial) = LogTail::open_with_mark(&f, sources.console_marks.offset(&f));
            new.extend(watch.apply_chunk(&pid_initial, procs));
            progressed |= progress.apply_console_chunk(&progress_initial);
            tails.insert(f, tail);
        }
        (new, progressed)
    };
    let attach_content = |progress: &mut SteamLaunchProgress, tails: &mut HashMap<PathBuf, LogTail>, files: Vec<PathBuf>| -> bool {
        let mut progressed = false;
        for f in files {
            if tails.contains_key(&f) {
                continue;
            }
            let (tail, initial) = LogTail::open_from_mark(&f, sources.content_marks.offset(&f));
            progressed |= progress.apply_content_chunk(&initial);
            tails.insert(f, tail);
        }
        progressed
    };
    let sync = |watch: &SteamWatch, new: &[u32], announced: &mut bool, on_first_pid: &mut dyn FnMut(u32)| {
        *shared.lock().unwrap() = watch.pids().clone();
        if !*announced {
            if let Some(pid) = new.first().or_else(|| watch.pids().iter().next()) {
                *announced = true;
                on_first_pid(*pid);
            }
        }
    };
    // Only read manifests when Steam is actually alive (running or logging),
    // `content_log.txt` has not already reported the update ended, and logs do
    // not already show active preparation.
    let check_manifest = |progress: &SteamLaunchProgress, running: bool, exited: bool| -> bool {
        if exited
            || progress.is_latest_ended()
            || progress.content_ended()
            || !(running || progress.saw_activity())
            || progress.is_preparing(false)
        {
            false
        } else {
            (sources.manifest_updating)()
        }
    };

    let (new, _) = attach_console(&mut watch, &mut progress, &mut tails, (sources.console_logs)());
    let _ = attach_content(&mut progress, &mut content_tails, (sources.content_logs)());
    sync(&watch, &new, &mut announced, on_first_pid);

    let mut cached_manifest_updating = check_manifest(&progress, cached_running, false);
    let mut deadline = Instant::now() + cfg.start_timeout;
    let mut last_resolve = Instant::now();
    while !watch.started() {
        if abort.load(Ordering::SeqCst) {
            return SteamWaitOutcome::Aborted;
        }
        if Instant::now() >= deadline {
            return SteamWaitOutcome::NotStarted;
        }
        wake.wait_while(cfg.poll_interval, || !abort.load(Ordering::SeqCst));
        let mut new = Vec::new();
        let mut progressed = false;
        for t in tails.values_mut() {
            if let Some(chunk) = t.read_new() {
                new.extend(watch.apply_chunk(&chunk, procs));
                progressed |= progress.apply_console_chunk(&chunk);
            }
        }
        for t in content_tails.values_mut() {
            if let Some(chunk) = t.read_new() {
                progressed |= progress.apply_content_chunk(&chunk);
            }
        }
        if last_resolve.elapsed() >= cfg.resolve_interval {
            last_resolve = Instant::now();
            let (added, p) = attach_console(&mut watch, &mut progress, &mut tails, (sources.console_logs)());
            new.extend(added);
            progressed |= p;
            progressed |= attach_content(&mut progress, &mut content_tails, (sources.content_logs)());
            cached_running = (sources.client_running)();
            if cached_running {
                saw_client = true;
            }
            let exited = saw_client && !cached_running;
            cached_manifest_updating = check_manifest(&progress, cached_running, exited);
        }
        sync(&watch, &new, &mut announced, on_first_pid);
        if !watch.started() {
            let exited = saw_client && !cached_running;
            let preparing = !exited && progress.is_preparing(cached_manifest_updating);
            if !exited && !progress.is_failed() && (preparing || progressed) {
                deadline = Instant::now() + cfg.start_timeout;
            }
            on_waiting(preparing);
        }
    }

    follow(&mut watch, &mut tails, procs, cfg, abort, wake, shared, on_tick)
}

/// Follows a Steam game seen running with `pids` until it exits, as
/// [`wait_for_steam_session`] does once the game started (a session picked
/// up again after the app restarted). Only what Steam logs from now on counts.
pub fn follow_steam_session(
    app_id: &str,
    procs: &dyn Procs,
    log_files: &dyn Fn() -> Vec<PathBuf>,
    cfg: &SteamWaitConfig,
    abort: &AtomicBool,
    wake: &Wake,
    shared: &Mutex<BTreeSet<u32>>,
    pids: BTreeSet<u32>,
) -> SteamWaitOutcome {
    let mut watch = SteamWatch::resume(app_id, pids);
    let mut tails: HashMap<PathBuf, LogTail> = log_files().into_iter().map(|f| (f.clone(), LogTail::open(&f).0)).collect();
    follow(&mut watch, &mut tails, procs, cfg, abort, wake, shared, &mut || {})
}

/// Until every tracked pid is gone (or tracking is aborted).
#[allow(clippy::too_many_arguments)]
fn follow(
    watch: &mut SteamWatch,
    tails: &mut HashMap<PathBuf, LogTail>,
    procs: &dyn Procs,
    cfg: &SteamWaitConfig,
    abort: &AtomicBool,
    wake: &Wake,
    shared: &Mutex<BTreeSet<u32>>,
    on_tick: &mut dyn FnMut(),
) -> SteamWaitOutcome {
    loop {
        if abort.load(Ordering::SeqCst) {
            return SteamWaitOutcome::Aborted;
        }
        watch.prune_dead(procs);
        *shared.lock().unwrap() = watch.pids().clone();
        if watch.finished() {
            return SteamWaitOutcome::Finished;
        }
        on_tick();
        wake.wait_while(cfg.poll_interval, || !abort.load(Ordering::SeqCst));
        for t in tails.values_mut() {
            if let Some(chunk) = t.read_new() {
                watch.apply_chunk(&chunk, procs);
            }
        }
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
        let (a, _) = parse_steam_log_pids("Game process added : AppID 4294967696 x ProcID 5", "400");
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
        fs::OpenOptions::new().append(true).open(&p).unwrap().write_all(b"new line\n").unwrap();
        assert_eq!(tail.read_new().as_deref(), Some("new line\n"));
        // Partial lines stay unread until their trailing newline arrives.
        fs::OpenOptions::new().append(true).open(&p).unwrap().write_all(b"partial").unwrap();
        assert_eq!(tail.read_new(), None);
        fs::OpenOptions::new().append(true).open(&p).unwrap().write_all(b" complete\n").unwrap();
        assert_eq!(tail.read_new().as_deref(), Some("partial complete\n"));
        fs::write(&p, "x\n").unwrap();
        assert_eq!(tail.read_new().as_deref(), Some("x\n"));
        let (_, none) = LogTail::open(&dir.path().join("missing"));
        assert!(none.is_empty());
    }

    #[test]
    fn log_marks_see_only_what_steam_got_after_them() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("console_log.txt");
        let uri = "steam://rungameid/945360";
        // A URL Steam got before the mark does not count.
        fs::write(&log, format!("[t] ExecuteSteamURL: \"{uri}\"\n")).unwrap();
        let later = dir.path().join("console-linux.txt");
        let marks = LogMarks::capture(std::slice::from_ref(&log));
        let files = vec![log.clone(), later.clone()];
        assert!(!marks.steam_received(&files, uri));
        // Another game whose id starts with this one's is not it.
        let append =
            |f: &Path, text: &str| fs::OpenOptions::new().create(true).append(true).open(f).unwrap().write_all(text.as_bytes()).unwrap();
        append(&log, "[t] ExecuteSteamURL: \"steam://rungameid/9453601\"\n");
        assert!(!marks.steam_received(&files, uri));
        append(&log, &format!("[t] ExecCommandLine: \"'steam' '{uri}'\"\n[t] ExecuteSteamURL: \"{uri}\"\n"));
        assert!(marks.steam_received(&files, uri));
        // A log that appeared after the mark counts from its start.
        let marks = LogMarks::capture(std::slice::from_ref(&log));
        assert!(!marks.steam_received(&files, uri));
        append(&later, &format!("ExecuteSteamURL: \"{uri}\"\n"));
        assert!(marks.steam_received(&files, uri));
    }

    fn fast_cfg() -> SteamWaitConfig {
        SteamWaitConfig {
            start_timeout: Duration::from_secs(3),
            poll_interval: Duration::from_millis(10),
            resolve_interval: Duration::from_millis(20),
            delivery_check: Duration::from_millis(50),
            cold_start_grace: Duration::ZERO,
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
            let (mut first, mut ticks) = (None, 0);
            let console_logs = || vec![l2.clone()];
            let content_logs = Vec::new;
            let console_marks = LogMarks::capture(std::slice::from_ref(&l2));
            let content_marks = LogMarks::default();
            let sources = SteamWaitSources {
                console_logs: &console_logs,
                content_logs: &content_logs,
                console_marks: &console_marks,
                content_marks: &content_marks,
                manifest_updating: &|| false,
                client_running: &|| false,
            };
            let out = wait_for_steam_session(
                "400",
                &*p2,
                &sources,
                &fast_cfg(),
                &a2,
                &Wake::default(),
                &s2,
                &mut |pid| first = Some(pid),
                &mut |_| {},
                &mut || ticks += 1,
            );
            assert!(ticks > 0, "polled while the game ran");
            (out, first)
        });

        std::thread::sleep(Duration::from_millis(60));
        procs.start(4242, "reaper", None);
        fs::OpenOptions::new().append(true).open(&log).unwrap().write_all(b"Game process added : AppID 400 x ProcID 4242\n").unwrap();
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(shared.lock().unwrap().iter().copied().collect::<Vec<_>>(), vec![4242]);
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
        let cfg = SteamWaitConfig { start_timeout: Duration::from_millis(80), ..fast_cfg() };
        let console_logs = || vec![log.clone()];
        let content_logs = Vec::new;
        let console_marks = LogMarks::capture(std::slice::from_ref(&log));
        let content_marks = LogMarks::default();
        let sources = SteamWaitSources {
            console_logs: &console_logs,
            content_logs: &content_logs,
            console_marks: &console_marks,
            content_marks: &content_marks,
            manifest_updating: &|| false,
            client_running: &|| false,
        };
        let out =
            wait_for_steam_session("400", &procs, &sources, &cfg, &abort, &Wake::default(), &shared, &mut |_| {}, &mut |_| {}, &mut || {});
        assert_eq!(out, SteamWaitOutcome::NotStarted);

        abort.store(true, Ordering::SeqCst);
        let out = wait_for_steam_session(
            "400",
            &procs,
            &sources,
            &fast_cfg(),
            &abort,
            &Wake::default(),
            &shared,
            &mut |_| {},
            &mut |_| {},
            &mut || {},
        );
        assert_eq!(out, SteamWaitOutcome::Aborted);
    }

    #[test]
    fn launch_progress_tracks_game_action_and_content_log() {
        let mut p = SteamLaunchProgress::new("2073850");
        assert!(!p.is_preparing(false));
        assert!(p.is_preparing(true), "active manifest update counts even without log lines");

        // Unrelated app lines do not start preparation when our app has no launch action.
        assert!(!p.apply_console_chunk("[t] GameAction [AppID 999, ActionID 1] : LaunchApp changed task to DownloadingDepots with \"\"\n"));
        assert!(!p.apply_content_chunk("[t] AppID 999 state changed : Update Required,Fully Installed,Update Queued,Update Running,\n"));
        assert!(!p.is_preparing(false));
        assert!(!p.apply_content_chunk("[t] AppID 999 state changed : Fully Installed,\n"));

        // Duplicate DownloadingDepots lines (normal in Steam) keep waiting.
        assert!(p.apply_console_chunk(
            "[t] GameAction [AppID 2073850, ActionID 1] : LaunchApp changed task to DownloadingDepots with \"\"\n\
             [t] GameAction [AppID 2073850, ActionID 1] : LaunchApp changed task to DownloadingDepots with \"\"\n"
        ));
        assert!(p.is_preparing(false));

        // Runtime dependency updates leave the game in `Update Queued` (no delay).
        assert!(p.apply_content_chunk("[t] AppID 2073850 state changed : Fully Installed,Update Queued,\n"));
        assert!(p.is_preparing(false));

        // Delayed, paused, or errored queued states are not active; Update Running is.
        assert!(!p.apply_content_chunk(
            "[t] AppID 2073850 state changed : Update Required,Fully Installed,Update Queued, (Update delayed for 43148 secs)\n"
        ));
        assert!(!p.is_preparing(false), "ended content update stops DownloadingDepots from hanging");
        assert!(!p.apply_content_chunk(
            "[t] AppID 2073850 state changed : Update Required,Fully Installed,Update Queued, (Disk Write Failure)\n"
        ));
        assert!(!p.is_preparing(false));
        assert!(!p.apply_content_chunk("[t] AppID 2073850 state changed : Update Required,Fully Installed,Update Paused,Update Queued,\n"));
        assert!(!p.is_preparing(false));
        assert!(p.apply_content_chunk(
            "[t] AppID 2073850 state changed : Update Required,Fully Installed,Update Running, (Update delayed for 7972 secs)\n"
        ));
        assert!(p.is_preparing(false), "Update Running stays active even with a delayed suffix");

        // Dependency update (e.g. Steam Linux Runtime 4183110) keeps DownloadingDepots
        // active even after the game's own update ends, then releases once it ends.
        assert!(!p.apply_content_chunk("[t] AppID 2073850 state changed : Fully Installed,\n"));
        assert!(!p.is_preparing(false));
        assert!(p.apply_content_chunk("[t] AppID 4183110 state changed : Update Required,Fully Installed,Update Queued,Update Running,\n"));
        assert!(p.is_preparing(false));
        assert!(!p.apply_content_chunk(
            "[t] AppID 4183110 state changed : Fully Installed,\n\
             [t] AppID 4183110 scheduler finished : removed from schedule (result No Error, state 0xc)\n"
        ));
        assert!(!p.is_preparing(false));

        // Pausing / stopping ends the update even while LaunchApp stayed in DownloadingDepots.
        assert!(p.apply_content_chunk("[t] AppID 2073850 App update changed : Running Update,Downloading,Staging,\n"));
        assert!(p.is_preparing(false));
        assert!(!p.apply_content_chunk(
            "[t] AppID 2073850 App update changed : Running Update,Downloading,Staging,Stopping,\n\
             [t] AppID 2073850 App update changed : None\n\
             [t] AppID 2073850 state changed : Update Required,Fully Installed,Update Queued,Update Started, (Suspended)\n"
        ));
        assert!(!p.is_preparing(false));

        // Moving on to install scripts / shader processing / launch options keeps waiting.
        assert!(p.apply_console_chunk(
            "[t] GameAction [AppID 2073850, ActionID 1] : LaunchApp changed task to ProcessingInstallScript with \"\"\n"
        ));
        assert!(p.is_preparing(false));
        assert!(p.apply_console_chunk(
            "[t] GameAction [AppID 2073850, ActionID 1] : LaunchApp changed task to ProcessingShaderCache with \"\"\n"
        ));
        assert!(p.is_preparing(false));

        // CreatingProcess + WaitingGameWindow + Completed in one chunk preserves the fresh window
        // without holding forever, and late duplicate lines from a second console log cannot resurrect it.
        assert!(p.apply_console_chunk(
            "[t] GameAction [AppID 2073850, ActionID 1] : LaunchApp changed task to CreatingProcess with \"\"\n\
             [t] GameAction [AppID 2073850, ActionID 1] : LaunchApp waiting for user response to CreatingProcess \"\"\n\
             [t] GameAction [AppID 2073850, ActionID 1] : LaunchApp continues with user response \"CreatingProcess\"\n\
             [t] GameAction [AppID 2073850, ActionID 1] : LaunchApp changed task to WaitingGameWindow with \"\"\n\
             [t] GameAction [AppID 2073850, ActionID 1] : LaunchApp changed task to Completed with \"\"\n"
        ));
        assert!(!p.is_preparing(false));
        assert!(!p.apply_console_chunk(
            "[t] GameAction [AppID 2073850, ActionID 1] : LaunchApp changed task to ProcessingShaderCache with \"\"\n\
             [t] GameAction [AppID 2073850, ActionID 1] : LaunchApp continues with user response \"ShowInterstitials\"\n"
        ));
        assert!(!p.is_preparing(false));

        // A failed launch overrides background update/manifest signals until a newer ActionID starts.
        assert!(
            p.apply_console_chunk("[t] GameAction [AppID 2073850, ActionID 2] : LaunchApp changed task to ShowLaunchOption with \"\"\n")
        );
        assert!(p.is_preparing(false));
        assert!(!p.apply_console_chunk("[t] GameAction [AppID 2073850, ActionID 1] : LaunchApp changed task to Failed with \"\"\n"));
        assert!(p.is_preparing(false));
        assert!(!p.apply_console_chunk(
            "[t] GameAction [AppID 2073850, ActionID 2] : LaunchApp failed with AppError_22 with \"\"\n\
             [t] GameAction [AppID 2073850, ActionID 2] : LaunchApp changed task to Failed with \"\"\n"
        ));
        assert!(!p.is_preparing(true), "failed launch ignores manifest_updating");
        assert!(!p.apply_content_chunk("[t] AppID 2073850 App update changed : Running Update,Downloading,Staging,\n"));
        assert!(!p.is_preparing(true), "failed launch ignores background content updates");

        // A newer ActionID clears the earlier failure and Ended state.
        assert!(
            p.apply_console_chunk("[t] GameAction [AppID 2073850, ActionID 3] : LaunchApp changed task to DownloadingDepots with \"\"\n")
        );
        assert!(p.is_preparing(false));
        assert!(!p.apply_console_chunk("[t] GameAction [AppID 2073850, ActionID 3] : LaunchApp changed task to Completed with \"\"\n"));
        assert!(!p.is_preparing(false));

        // Cross-file flush race: content_log reports Ended before the first DownloadingDepots arrives,
        // and an explicit content_log Ended beats a lazily-rewritten manifest.
        let mut race = SteamLaunchProgress::new("413150");
        assert!(!race.apply_content_chunk(
            "[t] AppID 413150 state changed : Fully Installed,\n\
             [t] AppID 413150 scheduler finished : removed from schedule (result No Error, state 0xc)\n"
        ));
        assert!(!race.is_preparing(true), "explicit content_log Ended overrides stale manifest flag");
        assert!(
            race.apply_console_chunk("[t] GameAction [AppID 413150, ActionID 1] : LaunchApp changed task to DownloadingDepots with \"\"\n")
        );
        assert!(!race.is_preparing(true), "first DownloadingDepots after Ended must not hang");

        // A newer action's dialog replaces an older action's phase, and canceling it ends the wait.
        assert!(race.apply_console_chunk(
            "[t] GameAction [AppID 413150, ActionID 2] : LaunchApp waiting for user response to ShowLaunchOption \"\"\n"
        ));
        assert!(race.is_preparing(false));
        assert!(
            !race.apply_console_chunk("[t] GameAction [AppID 413150, ActionID 2] : LaunchApp continues with user response \"Cancel\"\n")
        );
        assert!(!race.is_preparing(true));
        assert!(race.is_failed());
    }

    #[test]
    fn waits_past_start_timeout_while_preparing_and_times_out_on_stall_or_exit() {
        let dir = tempfile::tempdir().unwrap();
        let console = dir.path().join("console_log.txt");
        let content = dir.path().join("content_log.txt");
        let append =
            |p: &Path, text: &str| fs::OpenOptions::new().create(true).append(true).open(p).unwrap().write_all(text.as_bytes()).unwrap();
        let cfg = SteamWaitConfig { start_timeout: Duration::from_millis(120), ..fast_cfg() };

        // 1. Stale DownloadingDepots written before the launch mark does not delay timeout.
        fs::write(&console, "[t] GameAction [AppID 400, ActionID 1] : LaunchApp changed task to DownloadingDepots with \"\"\n").unwrap();
        fs::write(&content, "").unwrap();
        let console_marks = LogMarks::capture(std::slice::from_ref(&console));
        let content_marks = LogMarks::capture(std::slice::from_ref(&content));
        let sources = SteamWaitSources {
            console_logs: &|| vec![console.clone()],
            content_logs: &|| vec![content.clone()],
            console_marks: &console_marks,
            content_marks: &content_marks,
            manifest_updating: &|| false,
            client_running: &|| true,
        };
        let started = Instant::now();
        let out = wait_for_steam_session(
            "400",
            &FakeProcs::new(),
            &sources,
            &cfg,
            &AtomicBool::new(false),
            &Wake::default(),
            &Mutex::new(BTreeSet::new()),
            &mut |_| {},
            &mut |_| {},
            &mut || {},
        );
        assert_eq!(out, SteamWaitOutcome::NotStarted);
        assert!(started.elapsed() < Duration::from_millis(500));

        // 2. Active update waits past start_timeout, then succeeds once the game starts.
        let procs = Arc::new(FakeProcs::new());
        let (p2, c2, ct2, cfg2) = (procs.clone(), console.clone(), content.clone(), cfg.clone());
        let c_marks = LogMarks::capture(std::slice::from_ref(&console));
        let ct_marks = LogMarks::capture(std::slice::from_ref(&content));
        let handle = std::thread::spawn(move || {
            let sources = SteamWaitSources {
                console_logs: &|| vec![c2.clone()],
                content_logs: &|| vec![ct2.clone()],
                console_marks: &c_marks,
                content_marks: &ct_marks,
                manifest_updating: &|| false,
                client_running: &|| true,
            };
            wait_for_steam_session(
                "400",
                &*p2,
                &sources,
                &cfg2,
                &AtomicBool::new(false),
                &Wake::default(),
                &Mutex::new(BTreeSet::new()),
                &mut |_| {},
                &mut |_| {},
                &mut || {},
            )
        });
        std::thread::sleep(Duration::from_millis(30));
        append(&console, "[t] GameAction [AppID 400, ActionID 2] : LaunchApp changed task to DownloadingDepots with \"\"\n");
        append(&content, "[t] AppID 400 state changed : Update Required,Fully Installed,Update Queued,Update Running,\n");
        // Wait well past the 120ms start_timeout while Steam is updating.
        std::thread::sleep(Duration::from_millis(260));
        append(&content, "[t] AppID 400 state changed : Fully Installed,\n[t] AppID 400 scheduler finished : removed from schedule (result No Error, state 0xc)\n");
        procs.start(4242, "reaper", None);
        append(&console, "[t] Game process added : AppID 400 x ProcID 4242\n");
        std::thread::sleep(Duration::from_millis(40));
        procs.exit(4242);
        assert_eq!(handle.join().unwrap(), SteamWaitOutcome::Finished);

        // 3. Steam exiting mid-update causes the timeout to fire instead of hanging forever.
        let client_alive = Arc::new(AtomicBool::new(true));
        let (ca2, c2, ct2, cfg2) = (client_alive.clone(), console.clone(), content.clone(), cfg.clone());
        let c_marks = LogMarks::capture(std::slice::from_ref(&console));
        let ct_marks = LogMarks::capture(std::slice::from_ref(&content));
        let handle = std::thread::spawn(move || {
            let sources = SteamWaitSources {
                console_logs: &|| vec![c2.clone()],
                content_logs: &|| vec![ct2.clone()],
                console_marks: &c_marks,
                content_marks: &ct_marks,
                manifest_updating: &|| true,
                client_running: &|| ca2.load(Ordering::SeqCst),
            };
            wait_for_steam_session(
                "400",
                &FakeProcs::new(),
                &sources,
                &cfg2,
                &AtomicBool::new(false),
                &Wake::default(),
                &Mutex::new(BTreeSet::new()),
                &mut |_| {},
                &mut |_| {},
                &mut || {},
            )
        });
        std::thread::sleep(Duration::from_millis(30));
        append(&console, "[t] GameAction [AppID 400, ActionID 3] : LaunchApp changed task to DownloadingDepots with \"\"\n");
        std::thread::sleep(Duration::from_millis(160));
        client_alive.store(false, Ordering::SeqCst);
        assert_eq!(handle.join().unwrap(), SteamWaitOutcome::NotStarted);
    }
}
