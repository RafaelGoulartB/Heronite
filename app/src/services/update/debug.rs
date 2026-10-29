//! `HERONITE_DEBUG_UPDATE=<state>`: the update UI in a given state, with a
//! made-up release, for screenshots. `simulate` walks the whole flow (the
//! download is faked, Restart opens "What's new"). Nothing is requested,
//! written or restarted.

use std::time::Duration;

use gpui_kit::*;
use heronite_update::{Asset, Package, Progress, Release, Target};

use super::{Method, Phase, Updater};

/// The made-up release's notes: the shape of the real ones (`release.yml`).
const NOTES: &str = "**Download:** `Heronite-x86_64.AppImage` runs on any distribution (recommended). The `.minisig` and `.zsync` files are for the updaters; you do not need them.

## ✨ Highlights

- **New stores:** Battle.net, EA app, Ubisoft Connect and Rockstar Games Launcher, running under Wine
- **Console theme studio:** a full theme editor with undo and ready-made colour sets
- **Focus list rows:** switch the Focus list between covers and one game per row

## 🛠 Improvements

- Redesigned Stores and Emulators pages, with a sidebar list and real store logos ([1a2b3c4](https://x))
- Settings regrouped into six smaller groups

## 🐛 Fixes

- Proton's Xalia is no longer mistaken for the game ([5d6e7f8](https://x))

**Enjoying Heronite?** [Star it on GitHub](https://x) and share your feedback and ideas in [Discussions](https://x).";

/// Steps of the faked download (about 4 s).
const STEPS: u64 = 40;

impl Updater {
    pub(super) fn debug_state(&mut self, state: &str, cx: &mut Context<Self>) {
        self.debug = true;
        self.simulate = state.starts_with("simulate");
        if state == "simulate-auto" {
            // Update is pressed for you a second in (tests without clicks).
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                this.update(cx, |u, cx| u.install(cx)).ok();
            })
            .detach();
        }
        let package = crate::env::value("HERONITE_DEBUG_UPDATE_PACKAGE").and_then(|p| Package::parse(&p));
        self.method = Ok(match package {
            Some(kind) => Method::Package { kind, exe: "/usr/bin/heronite-rs".into() },
            None => Method::AppImage(Target { appimage: "/tmp/Heronite-x86_64.AppImage".into() }),
        });
        let next = super::Version { patch: self.current.patch + 1, ..self.current.clone() };
        let name = "Heronite-x86_64.AppImage".to_string();
        let release = Release {
            version: next.to_string(),
            name: format!("Heronite v{next}"),
            notes: NOTES.into(),
            page_url: heronite_update::RELEASES_PAGE.into(),
            published_at: "2026-10-11T12:00:00Z".into(),
            appimage: Some(Asset { name: name.clone(), url: String::new(), size: 15_400_000 }),
            signature: Some(Asset { name: format!("{name}.minisig"), url: String::new(), size: 0 }),
            packages: [
                format!("heronite_{next}-1_amd64.deb"),
                format!("heronite-{next}-1.x86_64.rpm"),
                format!("heronite-{next}-1-x86_64.pkg.tar.zst"),
            ]
            .into_iter()
            .flat_map(|n| [(format!("{n}.minisig"), 0), (n, 9_800_000)])
            .map(|(name, size)| Asset { name, url: String::new(), size })
            .collect(),
        };
        self.prefs.last_check = Some(super::now_secs().saturating_sub(7200));
        if state == "whatsnew" {
            self.phase = Phase::UpToDate;
            self.whats_new = Some(release);
            cx.notify();
            return;
        }
        self.phase = match state {
            "checking" => Phase::Checking,
            "uptodate" => Phase::UpToDate,
            "downloading" => Phase::Downloading(Progress { downloaded: 6_200_000, total: 15_400_000, bytes_per_sec: 2_400_000. }),
            "ready" => Phase::Ready,
            "installing" => Phase::Installing,
            "failed" => Phase::InstallFailed("The downloaded file does not match its signature".into()),
            _ => Phase::Available,
        };
        if !matches!(self.phase, Phase::Checking | Phase::UpToDate) {
            self.release = Some(release);
        }
        if let (Phase::InstallFailed(_), Some(kind)) = (&self.phase, package) {
            let file = format!("/home/you/.cache/heronite/updates/heronite_{next}-1_amd64.deb");
            let _ = kind;
            self.phase = Phase::InstallFailed("Not authorized (is a polkit agent running?)".into());
            self.downloaded = Some(file.into());
        }
    }
}

impl Updater {
    /// `simulate`: Update downloads nothing, the progress runs for a few
    /// seconds and ends as installed.
    pub(super) fn debug_install(&mut self, cx: &mut Context<Self>) {
        let total = self.download_asset().map(|a| a.size).unwrap_or(15_400_000);
        self.phase = Phase::Downloading(Progress { downloaded: 0, total, bytes_per_sec: 0. });
        cx.notify();
        self.job = Some(cx.spawn(async move |this, cx| {
            for step in 1..=STEPS {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                let p = Progress { downloaded: total * step / STEPS, total, bytes_per_sec: total as f64 / 4. };
                if this
                    .update(cx, |u, cx| {
                        u.phase = Phase::Downloading(p);
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
            }
            this.update(cx, |u, cx| u.install_finished(Ok(()), cx)).ok();
        }));
    }

    /// `simulate`: "Restart now" acts as the new version's first start.
    pub(super) fn debug_restart(&mut self, cx: &mut Context<Self>) {
        self.whats_new = self.release.take();
        self.phase = Phase::UpToDate;
        cx.notify();
    }
}
