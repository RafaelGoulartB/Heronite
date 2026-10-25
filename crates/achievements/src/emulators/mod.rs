//! Signs the user in to RetroAchievements in every installed emulator that
//! supports it, from the one login the app holds (a username and the Connect
//! token, the one `dorequest.php?r=login2` gives), so nobody signs in inside
//! each emulator. Signing out clears it again.
//!
//! The login goes into the emulator's own config file, edited in place (see
//! `ini`), for the native install and the Flatpak when both exist. A config
//! file that does not exist is never created: the emulator was not opened yet.
//! An emulator that runs is left alone: it rewrites its config when it
//! closes and would undo the change.
//!
//! The token is never logged.

mod file;
mod ini;
mod procs;
mod secret;
mod spec;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

/// The RetroAchievements login to write.
pub struct Login<'a> {
    pub username: &'a str,
    /// The Connect API token (not the Web API key).
    pub token: &'a str,
    /// Hardcore (challenge) mode on.
    pub hardcore: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Emulator {
    RetroArch,
    DuckStation,
    Pcsx2,
    Ppsspp,
    Dolphin,
    Flycast,
}

impl Emulator {
    pub const ALL: [Emulator; 6] =
        [Emulator::RetroArch, Emulator::DuckStation, Emulator::Pcsx2, Emulator::Ppsspp, Emulator::Dolphin, Emulator::Flycast];

    pub fn name(self) -> &'static str {
        match self {
            Emulator::RetroArch => "RetroArch",
            Emulator::DuckStation => "DuckStation",
            Emulator::Pcsx2 => "PCSX2",
            Emulator::Ppsspp => "PPSSPP",
            Emulator::Dolphin => "Dolphin",
            Emulator::Flycast => "Flycast",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// The file now holds this login.
    SignedIn,
    /// The login was removed (or there was none).
    SignedOut,
    /// No config file found: the emulator was never opened (or is not installed).
    NotSetUp,
    /// The emulator runs now; it would overwrite the file when it closes, so
    /// nothing was written.
    Running,
    Failed(String),
}

/// The outcome for one config file. An emulator with a native and a Flatpak
/// install has one `Status` for each.
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub emulator: Emulator,
    /// `None` when `state` is `NotSetUp`.
    pub config: Option<PathBuf>,
    pub state: State,
}

/// Writes `login` (`None` = sign out) into every emulator found under `home`.
pub fn apply(home: &Path, login: Option<&Login>) -> Vec<Status> {
    apply_to(home, &Emulator::ALL, login)
}

/// Like `apply`, for the listed emulators only. The app signs out only the
/// emulators whose file holds its own username (`status`), so a login the
/// user made by hand with another account is never wiped.
pub fn apply_to(home: &Path, only: &[Emulator], login: Option<&Login>) -> Vec<Status> {
    run(home, only, login, &procs::running_from_proc(), now(), &secret::machine_key)
}

/// What each emulator's file holds now for `username`: `SignedIn` when it
/// has that login (same username, a token, achievements on), `SignedOut`
/// otherwise, whatever else the file holds. Writes nothing.
pub fn status(home: &Path, username: Option<&str>) -> Vec<Status> {
    check(home, username, &secret::machine_key)
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The config files that exist for `emulator`.
fn found(home: &Path, emulator: Emulator) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = Vec::new();
    for path in spec::candidates(home, emulator) {
        if path.is_file() && !files.contains(&path) {
            files.push(path);
        }
    }
    files
}

fn run(
    home: &Path,
    only: &[Emulator],
    login: Option<&Login>,
    running: &dyn Fn(Emulator) -> bool,
    now: u64,
    machine_key: file::MachineKey,
) -> Vec<Status> {
    let mut out = Vec::new();
    for (i, &emulator) in only.iter().enumerate() {
        if only[..i].contains(&emulator) {
            continue;
        }
        let files = found(home, emulator);
        if files.is_empty() {
            out.push(Status { emulator, config: None, state: State::NotSetUp });
            continue;
        }
        let busy = running(emulator);
        let spec = spec::spec(emulator);
        for config in files {
            let state = if busy {
                State::Running
            } else {
                let result = match login {
                    Some(login) => file::sign_in(&spec, &config, login, now, machine_key).map(|()| State::SignedIn),
                    None => file::sign_out(&spec, &config).map(|()| State::SignedOut),
                };
                result.unwrap_or_else(State::Failed)
            };
            out.push(Status { emulator, config: Some(config), state });
        }
    }
    out
}

fn check(home: &Path, username: Option<&str>, machine_key: file::MachineKey) -> Vec<Status> {
    let mut out = Vec::new();
    for emulator in Emulator::ALL {
        let files = found(home, emulator);
        if files.is_empty() {
            out.push(Status { emulator, config: None, state: State::NotSetUp });
        }
        let spec = spec::spec(emulator);
        for config in files {
            let held = match username {
                Some(name) => file::holds(&spec, &config, name, machine_key),
                None => Ok(false),
            };
            let state = match held {
                Ok(true) => State::SignedIn,
                Ok(false) => State::SignedOut,
                Err(e) => State::Failed(e),
            };
            out.push(Status { emulator, config: Some(config), state });
        }
    }
    out
}
