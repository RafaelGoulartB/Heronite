//! "Keep the computer awake" (`heroniteKeepAwake`, on unless turned off):
//! while the game runs,
//! logind is told not to suspend or go idle, as a video player does. Useful
//! with a controller, whose input the desktop does not count as activity.
//!
//! The plan gets a command the runtime starts with the game and stops after
//! it: `systemd-inhibit` (or elogind's `elogind-inhibit`) holding an
//! `idle:sleep` block lock around `tail --pid=<launcher>`, so the lock also
//! goes away if the launcher dies.

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

/// Adds the lock to a plan that runs a process, when the game asks for it.
pub fn attach(plan: &mut LaunchPlan, probe: &dyn Probe, title: &str, settings: &GameSettings) {
    if !settings.keep_awake {
        return;
    }
    let Some(process) = plan.process_mut() else { return };
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
}
