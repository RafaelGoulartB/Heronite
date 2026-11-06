use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;
use std::ptr;

use windows::core::{Error, Interface, Result, HSTRING};
use windows::Win32::Foundation::{E_FAIL, S_OK};
use windows::Win32::Media::Audio::{
    eRender, AudioSessionState, AudioSessionStateActive, AudioSessionStateExpired, IAudioSessionControl2, IAudioSessionManager2, IMMDevice,
    ISimpleAudioVolume,
};
use windows::Win32::System::Com::CLSCTX_ALL;

use super::{devices, not_found, percent, scalar, take_string, with_audio, Device};
use crate::process::ProcessEntry;

/// Native endpoint and session-instance identities, never enumeration indexes.
/// A restarted process/session gets a new identity, even when its PID repeats.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionId {
    pub endpoint: String,
    pub instance: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub id: SessionId,
    pub pid: u32,
    pub name: String,
    pub binary: String,
    pub volume: u16,
    pub muted: bool,
}

pub(super) fn read(devices: &[(Device, IMMDevice)], processes: &[ProcessEntry]) -> Vec<Session> {
    let processes: HashMap<u32, &str> = processes.iter().map(|p| (p.pid, p.name.as_str())).collect();
    let mut sessions = Vec::new();
    for (info, device) in devices {
        let result = visit(device, |control| {
            // SAFETY: every interface stays local to the initialized worker.
            unsafe {
                if control.GetState()? != AudioSessionStateActive {
                    return Ok(());
                }
                let system_sounds = control.IsSystemSoundsSession();
                system_sounds.ok()?;
                if system_sounds == S_OK {
                    return Ok(());
                }
                let id = identity(&info.id, control)?;
                let pid = control.GetProcessId()?;
                let display = match control.GetDisplayName() {
                    Ok(name) => take_string(name),
                    Err(error) => {
                        log::debug!("Core Audio: reading session name for PID {pid}: {error}");
                        String::new()
                    }
                };
                let executable = processes
                    .get(&pid)
                    .map(|name| String::from(*name))
                    .or_else(|| crate::process::exe_path(pid).and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned())));
                let (name, binary) = app_name(&display, executable.as_deref(), pid);
                let volume: ISimpleAudioVolume = control.cast()?;
                sessions.push(Session {
                    id,
                    pid,
                    name,
                    binary,
                    volume: percent(volume.GetMasterVolume()?),
                    muted: volume.GetMute()?.as_bool(),
                });
                Ok(())
            }
        });
        if let Err(error) = result {
            log::warn!("Core Audio: reading sessions on {}: {error}", info.name);
        }
    }
    sessions
}

/// Sets exactly the identified sessions and unmutes them. Stale IDs never
/// target a new session occupying the same enumeration position.
pub fn set_sessions_volume(ids: &[SessionId], percent: u16) -> io::Result<()> {
    change_sessions(ids, |volume| {
        // SAFETY: the interface stays on the initialized calling worker.
        unsafe {
            volume.SetMasterVolume(scalar(percent), ptr::null())?;
            volume.SetMute(false, ptr::null())
        }
    })
}

pub fn set_sessions_muted(ids: &[SessionId], muted: bool) -> io::Result<()> {
    change_sessions(ids, |volume| {
        // SAFETY: the interface stays on the initialized calling worker.
        unsafe { volume.SetMute(muted, ptr::null()) }
    })
}

fn change_sessions(ids: &[SessionId], mut change: impl FnMut(&ISimpleAudioVolume) -> Result<()>) -> io::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    with_audio(|enumerator| {
        let mut pass = Targets::new(ids);
        let endpoints: HashSet<_> = ids.iter().map(|id| &id.endpoint).collect();
        for endpoint in endpoints {
            // SAFETY: the enumerator is local; the UTF-16 ID lives through the call.
            let device = match unsafe { enumerator.GetDevice(&HSTRING::from(endpoint.as_str())) } {
                Ok(device) => device,
                Err(error) => {
                    log::warn!("Core Audio: opening endpoint {endpoint}: {error}");
                    pass.unreadable(error);
                    continue;
                }
            };
            let result = visit(&device, |control| {
                // SAFETY: the control is local to this initialized call.
                if !live(unsafe { control.GetState()? }) {
                    return Ok(());
                }
                let id = identity(endpoint, control)?;
                if pass.take(&id) {
                    if let Err(error) = control.cast::<ISimpleAudioVolume>().and_then(|volume| change(&volume)) {
                        log::warn!("Core Audio: changing session {} on {endpoint}: {error}", id.instance);
                        pass.failed(error);
                    }
                }
                Ok(())
            });
            // A session that cannot be read may be another app's: it counts
            // only when a target was not found.
            if let Err(error) = result {
                log::debug!("Core Audio: reading sessions on {endpoint}: {error}");
                pass.unreadable(error);
            }
        }
        pass.finish(|| not_found("Audio session ended or its endpoint is no longer available"))
    })
}

/// One pass of [`change_sessions`] over the sessions asked for.
struct Targets<E> {
    remaining: HashSet<SessionId>,
    /// A target session found whose change failed.
    failed: Option<E>,
    /// An endpoint or session that could not be read: maybe a target.
    unreadable: Option<E>,
}

impl<E> Targets<E> {
    fn new(ids: &[SessionId]) -> Self {
        Targets { remaining: ids.iter().cloned().collect(), failed: None, unreadable: None }
    }

    /// Whether `id` is a target not changed yet; it is not one afterwards.
    fn take(&mut self, id: &SessionId) -> bool {
        self.remaining.remove(id)
    }

    fn failed(&mut self, error: E) {
        self.failed.get_or_insert(error);
    }

    fn unreadable(&mut self, error: E) {
        self.unreadable.get_or_insert(error);
    }

    /// A failed change is the error; then a missing target, explained by a
    /// read failure when there was one. Other apps' unreadable sessions do
    /// not fail a pass that changed every target.
    fn finish(self, not_found: impl FnOnce() -> E) -> std::result::Result<(), E> {
        if let Some(error) = self.failed {
            return Err(error);
        }
        if self.remaining.is_empty() {
            return Ok(());
        }
        Err(self.unreadable.unwrap_or_else(not_found))
    }
}

/// Sets every output session of a process `matches` takes, silent ones
/// included (a game in a menu or paused), and unmutes them, in one pass.
/// Returns how many were changed.
pub fn set_process_sessions_volume(matches: impl FnMut(u32) -> bool, percent: u16) -> io::Result<usize> {
    change_process_sessions(matches, |volume| {
        // SAFETY: the interface stays on the initialized calling worker.
        unsafe {
            volume.SetMasterVolume(scalar(percent), ptr::null())?;
            volume.SetMute(false, ptr::null())
        }
    })
}

/// [`set_process_sessions_volume`] for the mute.
pub fn set_process_sessions_muted(matches: impl FnMut(u32) -> bool, muted: bool) -> io::Result<usize> {
    change_process_sessions(matches, |volume| {
        // SAFETY: the interface stays on the initialized calling worker.
        unsafe { volume.SetMute(muted, ptr::null()) }
    })
}

fn change_process_sessions(
    mut matches: impl FnMut(u32) -> bool,
    mut change: impl FnMut(&ISimpleAudioVolume) -> Result<()>,
) -> io::Result<usize> {
    with_audio(|enumerator| {
        let mut changed = 0;
        let mut failed = None;
        for (info, device) in devices(enumerator, eRender)? {
            let result = visit(&device, |control| {
                // SAFETY: every interface stays local to the initialized worker.
                unsafe {
                    if !live(control.GetState()?) {
                        return Ok(());
                    }
                    let system_sounds = control.IsSystemSoundsSession();
                    system_sounds.ok()?;
                    if system_sounds == S_OK {
                        return Ok(());
                    }
                    let pid = control.GetProcessId()?;
                    if pid == 0 || !matches(pid) {
                        return Ok(());
                    }
                }
                match control.cast::<ISimpleAudioVolume>().and_then(|volume| change(&volume)) {
                    Ok(()) => changed += 1,
                    Err(error) => {
                        log::warn!("Core Audio: changing a session on {}: {error}", info.name);
                        failed.get_or_insert(error);
                    }
                }
                Ok(())
            });
            // Another app's session that cannot be read is not this change's.
            if let Err(error) = result {
                log::debug!("Core Audio: reading sessions on {}: {error}", info.name);
            }
        }
        failed.map_or(Ok(changed), Err)
    })
}

/// A session that can still be changed: playing (active) or silent for now
/// (inactive), not ended (expired).
fn live(state: AudioSessionState) -> bool {
    state != AudioSessionStateExpired
}

fn visit(device: &IMMDevice, mut visit: impl FnMut(&IAudioSessionControl2) -> Result<()>) -> Result<()> {
    // SAFETY: manager, enumerator and controls are created and released on the
    // same initialized worker. A disappearing session must not hide the rest.
    unsafe {
        let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
        let sessions = manager.GetSessionEnumerator()?;
        let mut first_error = None;
        for index in 0..sessions.GetCount()? {
            let result = (|| {
                let control: IAudioSessionControl2 = sessions.GetSession(index)?.cast()?;
                visit(&control)
            })();
            if let Err(error) = result {
                log::debug!("Core Audio: visiting session {index}: {error}");
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

fn identity(endpoint: &str, control: &IAudioSessionControl2) -> Result<SessionId> {
    // SAFETY: the returned CoTaskMem string is copied and freed in this call.
    let instance = unsafe { take_string(control.GetSessionInstanceIdentifier()?) };
    if instance.is_empty() {
        return Err(Error::new(E_FAIL, "Audio session has no instance identifier"));
    }
    Ok(SessionId { endpoint: endpoint.to_string(), instance })
}

fn app_name(display: &str, executable: Option<&str>, pid: u32) -> (String, String) {
    let binary = executable.unwrap_or_default().to_lowercase();
    let display = display.trim();
    let name = if !display.is_empty() && !display.starts_with('@') {
        display.to_string()
    } else {
        executable
            .and_then(|name| Path::new(name).file_stem())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("Process {pid}"))
    };
    (name, binary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_identity_distinguishes_endpoints_and_restarts() {
        let first = SessionId { endpoint: "speakers".into(), instance: "instance-one".into() };
        let same = first.clone();
        let restarted = SessionId { instance: "instance-two".into(), ..first.clone() };
        let moved = SessionId { endpoint: "headphones".into(), ..first.clone() };
        let sessions: HashSet<_> = [first, same, restarted, moved].into_iter().collect();
        assert_eq!(sessions.len(), 3);
    }

    fn id(instance: &str) -> SessionId {
        SessionId { endpoint: "speakers".into(), instance: instance.into() }
    }

    #[test]
    fn unreadable_sessions_of_other_apps_do_not_fail_a_complete_change() {
        let mut pass = Targets::new(&[id("game"), id("chat")]);
        pass.unreadable("another app's session has no identity");
        assert!(!pass.take(&id("browser")), "not a target");
        assert!(pass.take(&id("game")));
        assert!(!pass.take(&id("game")), "changed once");
        assert!(pass.take(&id("chat")));
        assert_eq!(pass.finish(|| "not found"), Ok(()));
    }

    #[test]
    fn failed_or_missing_targets_fail_the_change() {
        let mut failed = Targets::new(&[id("game"), id("chat")]);
        failed.take(&id("game"));
        failed.failed("access denied");
        failed.unreadable("unreadable");
        failed.take(&id("chat"));
        assert_eq!(failed.finish(|| "not found"), Err("access denied"));

        let mut ended = Targets::<&str>::new(&[id("game"), id("chat")]);
        ended.take(&id("game"));
        assert_eq!(ended.finish(|| "not found"), Err("not found"));

        let mut unknown = Targets::new(&[id("game")]);
        unknown.unreadable("endpoint gone");
        assert_eq!(unknown.finish(|| "not found"), Err("endpoint gone"), "the read failure may be why");
    }

    #[test]
    fn silent_sessions_are_still_changed_ended_ones_are_not() {
        use windows::Win32::Media::Audio::AudioSessionStateInactive;
        assert!(live(AudioSessionStateActive));
        assert!(live(AudioSessionStateInactive), "a game in a menu or paused");
        assert!(!live(AudioSessionStateExpired));
    }

    #[test]
    fn session_names_do_not_show_unresolved_resource_strings() {
        assert_eq!(app_name("", Some("Chrome.exe"), 42), ("Chrome".into(), "chrome.exe".into()));
        assert_eq!(app_name("@%SystemRoot%\\resource.dll,-1", Some("Game.exe"), 42), ("Game".into(), "game.exe".into()));
        assert_eq!(app_name(" Music player ", Some("player.exe"), 42), ("Music player".into(), "player.exe".into()));
        assert_eq!(app_name("", None, 42), ("Process 42".into(), String::new()));
    }
}
