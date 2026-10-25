//! Why a game closed right after starting, for the UI: [`gather`] reads
//! what `heronite_launch::diagnose` needs for one game and runs it, [`apply`]
//! applies a fix. Both block (files, processes): call them off the UI thread.
//!
//! The Collection runs [`gather`] when a game ends within
//! `FAILED_LAUNCH_WINDOW` and opens `dialogs::launch_problem` by itself only
//! for a known cause with a fix ([`Report::opens_by_itself`]); without a
//! window that cause goes into the notification (`system::hub::events`).
//! "Troubleshoot" (the game's context menu, always there) shows every
//! finding of the last session, however long the game ran (a .NET error
//! after a minute, a crash after an hour), including the ones nothing fixes.
//! The fixes applied this run are remembered ([`remember_tried`]), so a fix
//! that did not help is not offered as the answer again.

mod apply;
mod gather;
pub mod text;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use heronite_launch::diagnose::{Finding, Fix, RunKind};
use heronite_launch::ExitInfo;

pub use apply::apply;
pub use gather::gather;

/// How the run that is diagnosed ended.
#[derive(Debug, Clone, Copy)]
pub struct Ended {
    pub elapsed: Duration,
    pub exit: Option<ExitInfo>,
}

impl Ended {
    /// A non-zero exit code or a signal.
    pub fn failed(&self) -> bool {
        self.exit.is_some_and(|e| e.signal.is_some() || e.code.is_some_and(|c| c != 0))
    }
}

/// What the dialog shows about one game.
#[derive(Debug, Clone)]
pub struct Report {
    pub key: String,
    pub app_name: String,
    pub title: String,
    pub kind: RunKind,
    /// `None` when the user asked ("Troubleshoot") about the last session.
    pub ended: Option<Ended>,
    /// Asked about later: how the last session ended, as its log says.
    pub last_exit: Option<ExitInfo>,
    pub findings: Vec<Finding>,
    /// There is a launch log to read.
    pub has_log: bool,
    /// For ProtonDB.
    pub steam_app_id: Option<String>,
    /// The Proton [`Fix::UseProton`] switches to.
    pub best_proton: Option<String>,
    /// The Proton [`Fix::OtherProton`] switches to.
    pub other_proton: Option<String>,
}

impl Report {
    /// Worth opening the dialog for right after a failed start (see
    /// `heronite_launch::diagnose::opens_by_itself`).
    pub fn opens_by_itself(&self) -> bool {
        heronite_launch::diagnose::opens_by_itself(&self.findings)
    }

    /// The finding the dialog leads with.
    pub fn main(&self) -> Option<&Finding> {
        self.findings.first()
    }
}

static TRIED: Mutex<Option<HashMap<String, Vec<Fix>>>> = Mutex::new(None);

/// `fix` was applied to `app_name` (this run of the app).
pub fn remember_tried(app_name: &str, fix: &Fix) {
    let mut tried = TRIED.lock().unwrap();
    let list = tried.get_or_insert_with(HashMap::new).entry(app_name.to_string()).or_default();
    if !list.contains(fix) {
        list.push(fix.clone());
    }
}

/// Whether `fix` was applied to `app_name` before (this run of the app).
pub fn was_tried(app_name: &str, fix: &Fix) -> bool {
    TRIED.lock().unwrap().as_ref().and_then(|t| t.get(app_name)).is_some_and(|l| l.contains(fix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tried_fixes_are_remembered_per_game() {
        let fix = Fix::Winetricks(vec!["vcrun2022".into()]);
        assert!(!was_tried("tried-a", &fix));
        remember_tried("tried-a", &fix);
        remember_tried("tried-a", &fix);
        assert!(was_tried("tried-a", &fix));
        assert!(!was_tried("tried-b", &fix));
        assert!(!was_tried("tried-a", &Fix::StopWineserver));
    }

    #[test]
    fn failed_means_a_bad_code_or_a_signal() {
        let ended = |code, signal| Ended { elapsed: Duration::from_secs(2), exit: Some(ExitInfo { code, signal }) };
        assert!(ended(Some(53), None).failed());
        assert!(ended(None, Some(11)).failed());
        assert!(!ended(Some(0), None).failed());
        assert!(!Ended { elapsed: Duration::ZERO, exit: None }.failed());
    }
}
