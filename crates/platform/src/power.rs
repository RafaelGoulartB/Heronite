//! Keeping the computer awake (Windows).
//!
//! [`KeepAwake::hold`] keeps Windows from sleeping and the screen from
//! turning off until the guard is dropped, as a video player does. Useful
//! with a controller, whose input Windows does not count as activity.
//!
//! `SetThreadExecutionState` belongs to the thread that calls it and ends
//! with it, so each guard has a thread of its own that sets the state and
//! waits. Guards are independent: while one lives, the computer stays awake,
//! whatever the others do.

use std::io;
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;

use windows_sys::Win32::System::Power::{
    CallNtPowerInformation, SetThreadExecutionState, SystemExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED,
};

/// The computer stays awake while this lives. Dropping it ends the hold.
#[derive(Debug)]
pub struct KeepAwake {
    /// The thread waits on the other end; closing it (or sending) ends the hold.
    release: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl KeepAwake {
    /// Starts holding. `reason` names the hold in the thread's name and in the
    /// logs. Fails when the thread cannot be made or Windows refuses the state.
    pub fn hold(reason: &str) -> io::Result<KeepAwake> {
        let (release, wait) = mpsc::channel::<()>();
        let (ready_tx, ready) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new().name(format!("keep-awake: {reason}")).spawn(move || {
            // SAFETY: a plain call; it changes this thread's state only. 0 is a failure.
            let set = unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED) } != 0;
            let _ = ready_tx.send(set);
            if set {
                // Ends on a message or when the guard is gone.
                let _ = wait.recv();
                // SAFETY: as above; ES_CONTINUOUS alone clears what this thread asked for.
                unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
            }
        })?;
        match ready.recv() {
            Ok(true) => Ok(KeepAwake { release: Some(release), thread: Some(thread) }),
            _ => {
                let _ = thread.join();
                Err(io::Error::other("Windows did not accept the request to stay awake"))
            }
        }
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        drop(self.release.take());
        if let Some(thread) = self.thread.take() {
            // The thread only clears its state and ends.
            let _ = thread.join();
        }
    }
}

/// What the programs on the computer ask for together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requested {
    /// The computer is not to sleep.
    pub system: bool,
    /// The screen is not to turn off.
    pub display: bool,
}

/// What every program holds the computer awake for now, [`KeepAwake`] and
/// others alike. `None` when Windows does not say.
pub fn requested() -> Option<Requested> {
    let mut state = 0u32;
    // SAFETY: the output is one `u32`, as the information level says.
    let status = unsafe { CallNtPowerInformation(SystemExecutionState, std::ptr::null(), 0, (&mut state as *mut u32).cast(), 4) };
    (status == 0).then_some(Requested { system: state & ES_SYSTEM_REQUIRED != 0, display: state & ES_DISPLAY_REQUIRED != 0 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    const BOTH: Requested = Requested { system: true, display: true };

    fn held() -> bool {
        requested().expect("Windows says what is requested") == BOTH
    }

    #[test]
    fn the_computer_stays_awake_while_any_guard_lives() {
        // Another program may hold the computer awake too: then the end of
        // the hold cannot be seen.
        let others = held();

        let first = KeepAwake::hold("one game").unwrap();
        assert!(held());
        let second = KeepAwake::hold("another game").unwrap();
        drop(first);
        assert!(held(), "the second still holds");
        drop(second);
        if !others {
            let deadline = Instant::now() + Duration::from_secs(5);
            while held() {
                assert!(Instant::now() < deadline, "the hold did not end");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}
