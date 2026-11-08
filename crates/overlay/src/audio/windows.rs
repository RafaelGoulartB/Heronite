use std::collections::{HashMap, HashSet};
use std::io;

use heronite_platform::audio::{self as native, Session, Snapshot};
use heronite_platform::process::{self, ProcessEntry};

use super::{AppAudio, AudioState, Output, StreamId};

/// The current snapshot; `None` when Core Audio cannot be queried.
pub fn read(game_pid: Option<u32>) -> Option<AudioState> {
    let processes = process_snapshot();
    let snapshot = reported("reading audio", native::read(&processes))?;
    let game = match game_pid {
        Some(game_pid) => {
            let mut is_game = game_matcher(game_pid, &processes);
            snapshot.sessions.iter().map(|s| s.pid).filter(|&pid| is_game(pid)).collect()
        }
        None => HashSet::new(),
    };
    Some(map_snapshot(snapshot, &game))
}

/// The game row counts its silent sessions too, so it stays while the game is
/// quiet (a menu, a pause); the other apps are the ones playing, as on Linux,
/// where a paused stream leaves the list.
fn map_snapshot(snapshot: Snapshot, game: &HashSet<u32>) -> AudioState {
    let game_sessions: Vec<_> = snapshot.sessions.iter().filter(|s| game.contains(&s.pid)).collect();
    // What is heard, when the game plays anything.
    let game_volume = game_sessions.iter().find(|s| s.active).or(game_sessions.first()).map(|s| s.volume);
    let output = |device: native::Device| Output { name: device.id, description: device.name };
    AudioState {
        volume: snapshot.volume,
        muted: snapshot.muted,
        default_output: snapshot.outputs.iter().position(|d| Some(&d.id) == snapshot.default_output.as_ref()),
        outputs: snapshot.outputs.into_iter().map(output).collect(),
        mic_muted: snapshot.mic_muted,
        default_input: snapshot.inputs.iter().position(|d| Some(&d.id) == snapshot.default_input.as_ref()),
        inputs: snapshot.inputs.into_iter().map(output).collect(),
        game_volume,
        game_muted: !game_sessions.is_empty() && game_sessions.iter().all(|s| s.muted),
        apps: group_apps(snapshot.sessions.iter().filter(|s| s.active && !game.contains(&s.pid))),
    }
}

fn group_apps<'a>(sessions: impl Iterator<Item = &'a Session>) -> Vec<AppAudio> {
    let mut apps: Vec<AppAudio> = Vec::new();
    for session in sessions {
        let same_app = |app: &&mut AppAudio| {
            if session.binary.is_empty() && app.binary.is_empty() {
                app.name == session.name
            } else {
                !session.binary.is_empty() && app.binary == session.binary
            }
        };
        match apps.iter_mut().find(same_app) {
            Some(app) => {
                app.streams.push(session.id.clone());
                app.muted &= session.muted;
            }
            None => apps.push(AppAudio {
                name: session.name.clone(),
                binary: session.binary.clone(),
                volume: session.volume,
                muted: session.muted,
                streams: vec![session.id.clone()],
            }),
        }
    }
    apps
}

pub fn set_volume(percent: u16) {
    reported("setting output volume", native::set_volume(percent));
}

pub fn step_volume(delta: i32) -> Option<u16> {
    reported("stepping output volume", native::step_volume(delta))
}

pub fn set_muted(muted: bool) {
    reported("setting output mute", native::set_muted(muted));
}

pub fn set_mic_muted(muted: bool) {
    reported("setting microphone mute", native::set_mic_muted(muted));
}

pub fn set_streams_volume(streams: &[StreamId], percent: u16) {
    reported("setting app volume", native::set_sessions_volume(streams, percent));
}

pub fn set_streams_muted(streams: &[StreamId], muted: bool) {
    reported("setting app mute", native::set_sessions_muted(streams, muted));
}

/// Sets every session of the game, silent ones included, in one pass.
pub fn set_game_volume(game_pid: u32, percent: u16) {
    let is_game = game_matcher(game_pid, &process_snapshot());
    game_changed(game_pid, reported("setting game volume", native::set_process_sessions_volume(is_game, percent)));
}

pub fn set_game_muted(game_pid: u32, muted: bool) {
    let is_game = game_matcher(game_pid, &process_snapshot());
    game_changed(game_pid, reported("setting game mute", native::set_process_sessions_muted(is_game, muted)));
}

fn game_changed(game_pid: u32, changed: Option<usize>) {
    if changed == Some(0) {
        log::debug!("Core Audio: no audio sessions for game PID {game_pid}");
    }
}

/// Makes `id` (an [`Output::name`] of [`AudioState::outputs`]) the default output.
pub fn set_default_output(id: &str) {
    reported("setting the default output", native::set_default_device(id));
}

/// Makes `id` (an [`Output::name`] of [`AudioState::inputs`]) the microphone.
pub fn set_default_input(id: &str) {
    reported("setting the default microphone", native::set_default_device(id));
}

/// Recorder stream-name selection is a PulseAudio feature, not a WASAPI one.
pub fn game_stream_names(_game_pid: u32) -> Vec<String> {
    log::debug!("Core Audio: recorder stream-name selection is unsupported on Windows");
    Vec::new()
}

fn reported<T>(action: &str, result: io::Result<T>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            log::warn!("Core Audio: {action}: {error}");
            None
        }
    }
}

fn process_snapshot() -> Vec<ProcessEntry> {
    let processes = process::snapshot();
    if processes.is_empty() {
        log::warn!("Core Audio: process snapshot unavailable; the game matches by its PID and job only");
    }
    processes
}

/// Whether a session's process is the game's: one in a job this app started
/// (how screen capture finds the game too; the job keeps whatever the game
/// starts, even once a process in between has exited), or one under the
/// game's PID in `processes`, kept for processes outside any job (a game a
/// store client started). Toolhelp parent PIDs can be stale after a PID is
/// reused, so the tree alone is not trusted to find the game.
fn game_matcher(game_pid: u32, processes: &[ProcessEntry]) -> impl FnMut(u32) -> bool {
    let tree = descendants(game_pid, processes);
    let own = std::process::id();
    let mut in_job: HashMap<u32, bool> = HashMap::new();
    move |pid| {
        game_pid != 0
            && belongs_to_game(pid, &tree, |pid| *in_job.entry(pid).or_insert_with(|| pid != own && process::in_tracked_group(pid)))
    }
}

fn belongs_to_game(pid: u32, tree: &HashSet<u32>, in_job: impl FnOnce(u32) -> bool) -> bool {
    pid != 0 && (tree.contains(&pid) || in_job(pid))
}

fn descendants(root: u32, processes: &[ProcessEntry]) -> HashSet<u32> {
    if root == 0 {
        return HashSet::new();
    }
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for process in processes {
        children.entry(process.parent).or_default().push(process.pid);
    }
    let mut found = HashSet::new();
    let mut pending = vec![root];
    while let Some(pid) = pending.pop() {
        if pid != 0 && found.insert(pid) {
            pending.extend(children.get(&pid).into_iter().flatten().copied());
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(endpoint: &str, instance: &str, pid: u32, binary: &str, volume: u16, muted: bool) -> Session {
        Session {
            id: StreamId { endpoint: endpoint.into(), instance: instance.into() },
            pid,
            name: binary.into(),
            binary: binary.into(),
            volume,
            muted,
            active: true,
        }
    }

    #[test]
    fn descendants_include_game_and_grandchildren_without_following_cycles() {
        let process = |pid, parent| ProcessEntry { pid, parent, name: String::new() };
        let processes = [process(30, 20), process(20, 10), process(40, 4), process(10, 30), process(0, 10)];
        assert_eq!(descendants(10, &processes), HashSet::from([10, 20, 30]));
        assert_eq!(descendants(99, &processes), HashSet::from([99]));
        assert!(descendants(0, &processes).is_empty());
    }

    #[test]
    fn game_sessions_match_by_job_or_by_tree() {
        let tree = HashSet::from([10, 20]);
        assert!(belongs_to_game(30, &tree, |_| true), "a job member whose parent left the tree");
        assert!(belongs_to_game(20, &tree, |_| false), "a child outside any job (store client)");
        assert!(!belongs_to_game(40, &tree, |_| false));
        assert!(!belongs_to_game(0, &tree, |_| true), "the system's idle process is nobody's");
        let process = |pid, parent| ProcessEntry { pid, parent, name: String::new() };
        let mut is_game = game_matcher(0, &[process(30, 0)]);
        assert!(!is_game(30), "no game, no match");
    }

    #[test]
    fn mapping_separates_game_children_and_groups_apps_across_endpoints() {
        let browser_one = session("speakers", "browser-one", 40, "browser.exe", 80, true);
        let browser_two = session("headphones", "browser-two", 41, "browser.exe", 50, false);
        let expected_ids = vec![browser_one.id.clone(), browser_two.id.clone()];
        let snapshot = Snapshot {
            volume: 70,
            outputs: vec![
                native::Device { id: "speakers".into(), name: "Desktop speakers".into() },
                native::Device { id: "headphones".into(), name: "USB headset".into() },
            ],
            default_output: Some("headphones".into()),
            inputs: vec![native::Device { id: "microphone".into(), name: "USB microphone".into() }],
            default_input: Some("microphone".into()),
            mic_muted: Some(true),
            sessions: vec![
                session("speakers", "launcher", 10, "launcher.exe", 60, true),
                session("speakers", "game", 20, "game.exe", 90, false),
                browser_one,
                browser_two,
            ],
            ..Snapshot::default()
        };
        let state = map_snapshot(snapshot, &HashSet::from([10, 20]));
        assert_eq!(state.game_volume, Some(60));
        assert!(!state.game_muted, "one unmuted game session keeps the game row unmuted");
        assert_eq!(state.default_output, Some(1));
        assert_eq!(state.outputs[1].description, "USB headset");
        assert_eq!(state.default_input, Some(0));
        assert_eq!(state.inputs[0].description, "USB microphone");
        assert_eq!(state.mic_muted, Some(true));
        assert_eq!(state.apps.len(), 1);
        assert_eq!(state.apps[0].volume, 80);
        assert!(!state.apps[0].muted);
        assert_eq!(state.apps[0].streams, expected_ids);
    }

    #[test]
    fn reordered_snapshots_keep_session_ids_and_missing_defaults_stay_unknown() {
        let first = session("speakers", "first-instance", 10, "first.exe", 60, false);
        let second = session("speakers", "second-instance", 20, "second.exe", 80, false);
        let before = group_apps([&first, &second].into_iter());
        let after = group_apps([&second, &first].into_iter());
        assert_eq!(before[0].streams, after[1].streams);
        assert_ne!(before[0].streams, after[0].streams);
        let state = map_snapshot(Snapshot::default(), &HashSet::new());
        assert_eq!(state.default_output, None);
        assert_eq!(state.default_input, None);
        assert_eq!(state.mic_muted, None);
        assert_eq!(state.game_volume, None);
        assert!(!state.game_muted);
    }

    #[test]
    fn a_silent_game_keeps_its_row_and_paused_apps_leave_the_list() {
        let mut menu = session("speakers", "menu", 20, "game.exe", 40, true);
        menu.active = false;
        let mut paused = session("speakers", "paused", 40, "player.exe", 80, false);
        paused.active = false;
        let browser = session("speakers", "browser", 50, "browser.exe", 70, false);
        let snapshot = Snapshot { sessions: vec![menu.clone(), paused, browser.clone()], ..Snapshot::default() };
        let state = map_snapshot(snapshot, &HashSet::from([20]));
        assert_eq!(state.game_volume, Some(40), "the game is in a menu, not gone");
        assert!(state.game_muted);
        assert_eq!(state.apps.len(), 1);
        assert_eq!(state.apps[0].streams, vec![browser.id]);

        let playing = session("headphones", "playing", 20, "game.exe", 90, false);
        let snapshot = Snapshot { sessions: vec![menu, playing], ..Snapshot::default() };
        let state = map_snapshot(snapshot, &HashSet::from([20]));
        assert_eq!(state.game_volume, Some(90), "the playing session is the one heard");
        assert!(!state.game_muted);
    }

    #[test]
    fn distinct_executables_with_the_same_display_name_are_not_grouped() {
        let first = session("speakers", "one", 10, "first.exe", 60, true);
        let mut second = session("speakers", "two", 20, "second.exe", 80, true);
        second.name = first.name.clone();
        assert_eq!(group_apps([&first, &second].into_iter()).len(), 2);
    }
}
