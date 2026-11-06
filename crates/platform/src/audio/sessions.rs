use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;
use std::ptr;

use windows::core::{Error, Interface, Result, HSTRING};
use windows::Win32::Foundation::{E_FAIL, S_OK};
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, AudioSessionStateExpired, IAudioSessionControl2, IAudioSessionManager2, IMMDevice, ISimpleAudioVolume,
};
use windows::Win32::System::Com::CLSCTX_ALL;

use super::{not_found, percent, scalar, take_string, with_audio, Device};

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

pub(super) fn read(devices: &[(Device, IMMDevice)]) -> Vec<Session> {
    let processes: HashMap<u32, String> = crate::process::snapshot().into_iter().map(|p| (p.pid, p.name)).collect();
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
                    .cloned()
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
        let mut remaining: HashSet<_> = ids.iter().cloned().collect();
        let endpoints: HashSet<_> = ids.iter().map(|id| &id.endpoint).collect();
        let mut first_error = None;
        for endpoint in endpoints {
            let result = (|| {
                // SAFETY: the enumerator is local; the UTF-16 ID lives through the call.
                let device = unsafe { enumerator.GetDevice(&HSTRING::from(endpoint.as_str()))? };
                visit(&device, |control| {
                    // SAFETY: the control is local to this initialized call.
                    if unsafe { control.GetState()? } == AudioSessionStateExpired {
                        return Ok(());
                    }
                    let id = identity(endpoint, control)?;
                    if remaining.remove(&id) {
                        change(&control.cast()?)?;
                    }
                    Ok(())
                })
            })();
            if let Err(error) = result {
                log::warn!("Core Audio: changing sessions on {endpoint}: {error}");
                first_error.get_or_insert(error);
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        if !remaining.is_empty() {
            return Err(not_found("Audio session ended or its endpoint is no longer available"));
        }
        Ok(())
    })
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

    #[test]
    fn session_names_do_not_show_unresolved_resource_strings() {
        assert_eq!(app_name("", Some("Chrome.exe"), 42), ("Chrome".into(), "chrome.exe".into()));
        assert_eq!(app_name("@%SystemRoot%\\resource.dll,-1", Some("Game.exe"), 42), ("Game".into(), "game.exe".into()));
        assert_eq!(app_name(" Music player ", Some("player.exe"), 42), ("Music player".into(), "player.exe".into()));
        assert_eq!(app_name("", None, 42), ("Process 42".into(), String::new()));
    }
}
