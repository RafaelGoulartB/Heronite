//! `HERONITE_DEBUG_UPDATE=<state>`: the update UI in a given state, with a
//! made-up release, for screenshots. Nothing is requested or written.

use heronite_update::{Asset, Progress, Release, Target};

use super::{Phase, Updater};

impl Updater {
    pub(super) fn debug_state(&mut self, state: &str) {
        self.debug = true;
        self.target = Ok(Target { appimage: "/tmp/Heronite-x86_64.AppImage".into() });
        let next = super::Version { patch: self.current.patch + 1, ..self.current.clone() };
        let name = "Heronite-x86_64.AppImage".to_string();
        let release = Release {
            version: next.to_string(),
            name: format!("Heronite v{next}"),
            notes: "### Features\n- **collection:** Update Heronite from the app ([1a2b3c4](https://x))\n- Release notes in Settings\n\n### Bug fixes\n- The welcome screen keeps its layout on narrow windows ([5d6e7f8](https://x))".into(),
            page_url: heronite_update::RELEASES_PAGE.into(),
            published_at: "2026-10-11T12:00:00Z".into(),
            appimage: Some(Asset { name: name.clone(), url: String::new(), size: 15_400_000 }),
            signature: Some(Asset { name: format!("{name}.minisig"), url: String::new(), size: 0 }),
        };
        self.prefs.last_check = Some(super::now_secs().saturating_sub(7200));
        self.phase = match state {
            "checking" => Phase::Checking,
            "uptodate" => Phase::UpToDate,
            "downloading" => Phase::Downloading(Progress { downloaded: 6_200_000, total: 15_400_000, bytes_per_sec: 2_400_000. }),
            "ready" => Phase::Ready,
            "failed" => Phase::InstallFailed("The downloaded file does not match its signature".into()),
            _ => Phase::Available,
        };
        if !matches!(self.phase, Phase::Checking | Phase::UpToDate) {
            self.release = Some(release);
        }
    }
}
