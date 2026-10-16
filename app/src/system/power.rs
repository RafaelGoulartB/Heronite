//! Suspend, restart and shut down the computer (console mode's Power menu).
//!
//! Goes through logind like a desktop's own power menu: `systemctl`
//! (systemd), else `loginctl` (elogind). Neither needs root for the active
//! local session: polkit decides, and a blocked request (another user logged
//! in, an app inhibiting shutdown) fails with logind's message. A shutdown
//! ends the app with SIGTERM, which runs the graceful quit (`signals`), so
//! the playtime of running games is saved.

use std::process::{Command, Stdio};

use heronite_i18n::tr;
use heronite_launch::Probe;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerAction {
    Suspend,
    Reboot,
    PowerOff,
}

impl PowerAction {
    /// The `systemctl` / `loginctl` command.
    fn verb(self) -> &'static str {
        match self {
            Self::Suspend => "suspend",
            Self::Reboot => "reboot",
            Self::PowerOff => "poweroff",
        }
    }
}

/// The command for `action` with the first tool found on `$PATH`.
fn command(action: PowerAction, has: impl Fn(&str) -> bool) -> Option<(&'static str, &'static str)> {
    ["systemctl", "loginctl"].into_iter().find(|t| has(t)).map(|t| (t, action.verb()))
}

/// Blocking: runs the request and waits for logind's answer. `Err`: why it
/// was refused, for the user.
///
/// `HERONITE_DEBUG_POWER_DRY=1` only logs the command (for trying the menu).
pub fn run(action: PowerAction) -> Result<(), String> {
    let probe = heronite_launch::SystemProbe;
    let Some((tool, verb)) = command(action, |t| probe.which(t).is_some()) else {
        return Err(tr!("Neither systemctl nor loginctl was found, so Heronite cannot ask the system to do this.").into());
    };
    if crate::env::flag("HERONITE_DEBUG_POWER_DRY") {
        log::info!("power (dry run): {tool} {verb}");
        return Ok(());
    }
    log::info!("power: {tool} {verb}");
    let out = Command::new(tool).arg(verb).stdin(Stdio::null()).output().map_err(|e| format!("{tool}: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(if err.is_empty() { format!("{tool} {verb}: {}", out.status) } else { err })
}

#[cfg(test)]
mod tests {
    use super::{command, PowerAction};

    #[test]
    fn systemctl_first_then_loginctl() {
        assert_eq!(command(PowerAction::PowerOff, |_| true), Some(("systemctl", "poweroff")));
        assert_eq!(command(PowerAction::Suspend, |t| t == "loginctl"), Some(("loginctl", "suspend")));
        assert_eq!(command(PowerAction::Reboot, |_| false), None);
    }
}
