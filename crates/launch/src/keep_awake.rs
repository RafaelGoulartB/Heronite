//! "Keep the computer awake" (`heroniteKeepAwake`, on unless turned off):
//! while the game runs, the system is told not to suspend or go idle, as a
//! video player does. Useful with a controller, whose input the desktop does
//! not count as activity.
//!
//! On Linux the plan gets a command the runtime starts with the game and
//! stops after it: `systemd-inhibit` (or elogind's `elogind-inhibit`) holding
//! an `idle:sleep` block lock around `tail --pid=<launcher>`, so the lock also
//! goes away if the launcher dies.
//!
//! On Windows the plan only says so (`ProcessPlan::hold_awake`) and the
//! runtime keeps a [`Hold`] for the session; the hold ends with the app too.

use std::io;

use crate::plan::{Command, LaunchPlan};
use crate::probe::Probe;
use crate::settings::GameSettings;

pub const KEY: &str = "heroniteKeepAwake";

/// The lock holder for `title`, `None` without logind's tool.
pub fn command(probe: &dyn Probe, title: &str, launcher_pid: u32) -> Option<Command> {
    let program = ["systemd-inhibit", "elogind-inhibit"].into_iter().find_map(|t| probe.which(t))?;
    let args = [
        "--what=idle:sleep".to_string(),
        "--who=Heronite".into(),
        format!("--why={title} is running"),
        "--mode=block".into(),
        "tail".into(),
        format!("--pid={launcher_pid}"),
        "-f".into(),
        "/dev/null".into(),
    ];
    Some(Command { program, args: args.into(), ..Default::default() })
}

/// The computer kept awake from inside the app, until dropped (Windows; a
/// plan with [`ProcessPlan::hold_awake`](crate::plan::ProcessPlan::hold_awake)).
pub struct Hold {
    #[cfg(windows)]
    _awake: heronite_platform::power::KeepAwake,
}

impl Hold {
    /// Starts holding for `title`'s game. Fails when Windows refuses; always
    /// fails elsewhere, where the lock is [`command`].
    pub fn new(title: &str) -> io::Result<Hold> {
        #[cfg(windows)]
        {
            heronite_platform::power::KeepAwake::hold(&format!("{title} is running")).map(|_awake| Hold { _awake })
        }
        #[cfg(not(windows))]
        {
            let _ = title;
            Err(io::ErrorKind::Unsupported.into())
        }
    }

    /// Lets the computer sleep again (dropping the hold does the same).
    pub fn release(self) {}
}

/// Adds the lock (Linux) or the request to hold (Windows) to a plan that runs
/// a process, when the game asks for it.
pub fn attach(plan: &mut LaunchPlan, probe: &dyn Probe, title: &str, settings: &GameSettings) {
    if !settings.keep_awake {
        return;
    }
    let Some(process) = plan.process_mut() else { return };
    if cfg!(windows) {
        process.hold_awake = true;
        return;
    }
    match command(probe, title, std::process::id()) {
        Some(cmd) => process.keep_awake = Some(cmd),
        None => process.notes.push("keep awake: neither systemd-inhibit nor elogind-inhibit is installed".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::FakeProbe;

    #[test]
    fn the_lock_waits_for_the_launcher() {
        let probe = FakeProbe::new("/h").with_bin("systemd-inhibit", "/usr/bin/systemd-inhibit");
        let c = command(&probe, "Control", 42).unwrap();
        assert_eq!(c.program, "/usr/bin/systemd-inhibit");
        assert_eq!(c.args[0], "--what=idle:sleep");
        assert_eq!(c.args[2], "--why=Control is running");
        assert_eq!(&c.args[4..], ["tail", "--pid=42", "-f", "/dev/null"]);
        let elogind = FakeProbe::new("/h").with_bin("elogind-inhibit", "/usr/bin/elogind-inhibit");
        assert_eq!(command(&elogind, "x", 1).unwrap().program, "/usr/bin/elogind-inhibit");
        assert!(command(&FakeProbe::new("/h"), "x", 1).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn windows_asks_the_runtime_to_hold_instead_of_starting_a_lock() {
        use crate::plan::ProcessPlan;
        let probe = FakeProbe::new(r"C:\Users\u").with_bin("systemd-inhibit", r"C:\bin\systemd-inhibit.exe");
        let on = GameSettings::from_map(&serde_json::Map::new(), r"C:\Users\u");
        let mut plan = LaunchPlan::Native(ProcessPlan::default());
        attach(&mut plan, &probe, "Control", &on);
        let process = plan.process().unwrap();
        assert!(process.hold_awake, "on unless turned off");
        assert!(process.keep_awake.is_none() && process.notes.is_empty());

        let off = GameSettings::from_map(serde_json::json!({ KEY: false }).as_object().unwrap(), r"C:\Users\u");
        let mut plan = LaunchPlan::Native(ProcessPlan::default());
        attach(&mut plan, &probe, "Control", &off);
        assert!(!plan.process().unwrap().hold_awake);
        // Nothing to hold for a game Steam starts.
        let mut steam = LaunchPlan::Unavailable(String::new());
        attach(&mut steam, &probe, "Control", &on);
        assert_eq!(steam, LaunchPlan::Unavailable(String::new()));
    }

    #[test]
    fn only_windows_holds_from_inside_the_app() {
        assert_eq!(Hold::new("Control").is_ok(), cfg!(windows));
    }
}
