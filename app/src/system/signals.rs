//! SIGTERM / SIGINT / SIGHUP are a quit: the desktop session ending, `kill`,
//! Ctrl-C in the terminal that started the app. GPUI installs no handler, so
//! without this the process would just die and lose the running games'
//! playtime and the Epic queue's orderly pause.
//!
//! A thread reads the signals (`signal-hook` iterator, nothing runs inside the
//! handler) and forwards them to the foreground loop, which runs the graceful
//! quit of the hub. The thread alone decides the escalation, so it works even
//! if the UI thread is stuck: a signal while a quit is already under way (a
//! second signal, or one after a menu Quit) ends the process at once with
//! `128 + signal`.

use std::sync::atomic::{AtomicBool, Ordering};

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

/// Set by the hub when its graceful quit starts.
static QUITTING: AtomicBool = AtomicBool::new(false);

/// The quit has begun (called by the hub): the next signal ends the process.
pub fn quit_started() {
    QUITTING.store(true, Ordering::SeqCst);
}

/// Exit status after a signal, as a shell reports it.
pub fn exit_code_for(signal: i32) -> i32 {
    128 + signal
}

/// What to do with a signal given what happened before. Separate from the
/// thread so it can be tested.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// Start the graceful quit.
    Quit,
    /// A quit is already under way: leave now.
    ExitNow(i32),
}

pub fn decide(signal: i32, quit_under_way: bool, already_signalled: bool) -> Action {
    if quit_under_way || already_signalled {
        Action::ExitNow(exit_code_for(signal))
    } else {
        Action::Quit
    }
}

/// Starts the signal thread. The receiver gets the signal numbers that ask
/// for a graceful quit. `None` if the signals cannot be registered (the app
/// then runs without them).
pub fn spawn() -> Option<UnboundedReceiver<i32>> {
    let mut signals = match Signals::new([SIGTERM, SIGINT, SIGHUP]) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("signal handlers not installed: {e}");
            return None;
        }
    };
    let (tx, rx) = unbounded();
    let started = std::thread::Builder::new().name("signals".into()).spawn(move || run(&mut signals, tx));
    match started {
        Ok(_) => Some(rx),
        Err(e) => {
            log::warn!("signal thread not started: {e}");
            None
        }
    }
}

fn run(signals: &mut Signals, tx: UnboundedSender<i32>) {
    let mut signalled = false;
    for signal in signals.forever() {
        match decide(signal, QUITTING.load(Ordering::SeqCst), signalled) {
            Action::Quit => {
                signalled = true;
                log::info!("signal {signal} received: quitting gracefully");
                if tx.unbounded_send(signal).is_err() {
                    // Nobody listens any more: the loop is gone.
                    std::process::exit(exit_code_for(signal));
                }
            }
            Action::ExitNow(code) => {
                log::warn!("signal {signal} while already quitting: exiting now");
                std::process::exit(code);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_signal_quits_gracefully_the_second_exits() {
        assert_eq!(decide(SIGTERM, false, false), Action::Quit);
        assert_eq!(decide(SIGTERM, false, true), Action::ExitNow(143));
        assert_eq!(decide(SIGINT, false, true), Action::ExitNow(130));
        assert_eq!(decide(SIGHUP, true, false), Action::ExitNow(129));
    }
}
