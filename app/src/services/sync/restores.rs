//! Bringing things back from the cloud account (Settings → Cloud, "Restore
//! from the cloud"), for a new or formatted PC: what is there, listed when
//! the user asks; a PC's Heronite backup (into the backup folder, then
//! Settings → Backups asks to restore it); its game saves (into Ludusavi's
//! backup folder only: a game's own save folder is never touched, restoring
//! one is Ludusavi's); the screenshots (into the captures folder, the ones
//! here kept); or all of it, the backup last since its restore restarts.
//! One at a time, on a thread of its own.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::channel::mpsc::unbounded;
use futures::StreamExt;
use gpui_kit::*;
use heronite_data::proc::Cancel;
use heronite_data::{settings, AppPaths, DataPaths};
use heronite_emulation::ludusavi::{detect_ludusavi, SystemRunner};
use heronite_i18n::tr;
use heronite_launch::SystemProbe;
use heronite_sync::cloud::{self, CloudBackup, InCloud};
use heronite_sync::SyncError;

use super::LibrarySync;

/// What is being brought back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bring {
    Backup(CloudBackup),
    /// The saves of a PC.
    Saves(String),
    Captures,
    /// A PC's saves and newest backup, and the screenshots.
    Everything(String),
}

/// What runs now.
pub struct Bringing {
    pub what: Bring,
    /// The step running (`Everything`: saves, screenshots, then the backup).
    pub step: Step,
    pub percent: Option<u8>,
    cancel: Arc<Cancel>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Backup,
    Saves,
    Captures,
}

#[derive(Default)]
pub(super) struct Restores {
    /// The last listing, or why it failed.
    listed: Option<Result<InCloud, String>>,
    listing: bool,
    current: Option<Bringing>,
    /// How the last one went (`true`: an error).
    notice: Option<(String, bool)>,
    /// Brought to be restored: Settings → Backups takes it.
    to_restore: Option<PathBuf>,
}

enum Msg {
    Step(Step),
    Percent(u8),
    Done(Result<Option<PathBuf>, String>),
}

impl LibrarySync {
    pub fn in_cloud(&self) -> Option<&Result<InCloud, String>> {
        self.restores.listed.as_ref()
    }

    pub fn listing_cloud(&self) -> bool {
        self.restores.listing
    }

    pub fn bringing(&self) -> Option<&Bringing> {
        self.restores.current.as_ref()
    }

    pub fn restore_notice(&self) -> Option<&(String, bool)> {
        self.restores.notice.as_ref()
    }

    /// The backup brought to be restored, once (Settings → Backups asks).
    pub fn take_backup_to_restore(&mut self) -> Option<PathBuf> {
        self.restores.to_restore.take()
    }

    /// Another account: what was listed was of the old one.
    pub(super) fn forget_cloud_backups(&mut self) {
        self.restores.listed = None;
        self.restores.notice = None;
    }

    /// Lists what the account holds to bring back.
    pub fn list_cloud(&mut self, cx: &mut Context<Self>) {
        if self.restores.listing {
            return;
        }
        self.restores.listing = true;
        cx.notify();
        let (data, app) = (self.data_paths.clone(), self.app_paths.clone());
        cx.spawn(async move |this, cx| {
            let listed = cx.background_spawn(async move { cloud::in_cloud(&data, &app) }).await;
            this.update(cx, |s, cx| {
                s.restores.listing = false;
                s.restores.listed = Some(listed.map_err(|e| e.localized()));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Stops what is being brought back.
    pub fn stop_bringing(&mut self, cx: &mut Context<Self>) {
        if let Some(b) = &self.restores.current {
            b.cancel.request();
        }
        cx.notify();
    }

    /// Brings `what` back.
    pub fn bring(&mut self, what: Bring, cx: &mut Context<Self>) {
        if self.restores.current.is_some() {
            return;
        }
        let in_cloud = self.restores.listed.clone().and_then(Result::ok).unwrap_or_default();
        let cancel = Cancel::new();
        let first = match &what {
            Bring::Backup(_) => Step::Backup,
            Bring::Captures => Step::Captures,
            Bring::Saves(_) | Bring::Everything(_) => Step::Saves,
        };
        self.restores.current = Some(Bringing { what: what.clone(), step: first, percent: None, cancel: cancel.clone() });
        self.restores.notice = None;
        cx.notify();
        let (tx, mut rx) = unbounded::<Msg>();
        let (data, app) = (self.data_paths.clone(), self.app_paths.clone());
        // Bringing gigabytes of screenshots takes a while: a thread of its own.
        std::thread::spawn(move || {
            let result = run(&data, &app, &what, &in_cloud, &cancel, &tx);
            tx.unbounded_send(Msg::Done(result)).ok();
        });
        cx.spawn(async move |this, cx| {
            while let Some(msg) = rx.next().await {
                let done = matches!(msg, Msg::Done(_));
                this.update(cx, |s, cx| {
                    match msg {
                        Msg::Step(step) => {
                            if let Some(c) = &mut s.restores.current {
                                c.step = step;
                                c.percent = None;
                            }
                        }
                        Msg::Percent(p) => {
                            if let Some(c) = &mut s.restores.current {
                                c.percent = Some(p);
                            }
                        }
                        Msg::Done(result) => {
                            let what = s.restores.current.take().map(|c| c.what);
                            s.changed();
                            match result {
                                Ok(Some(backup)) => s.restores.to_restore = Some(backup),
                                Ok(None) => s.restores.notice = what.map(|w| (done_text(&w), false)),
                                Err(e) => s.restores.notice = Some((e, true)),
                            }
                        }
                    }
                    cx.notify();
                })
                .ok();
                if done {
                    break;
                }
            }
        })
        .detach();
    }
}

fn done_text(what: &Bring) -> String {
    match what {
        Bring::Saves(_) => {
            tr!("The game saves are in Ludusavi's backup folder. Restore a game from its Saves tab when you want it back.").into()
        }
        Bring::Captures => tr!("The screenshots are back in the screenshots folder.").into(),
        Bring::Backup(_) | Bring::Everything(_) => tr!("Brought back.").into(),
    }
}

/// Brings `what` back (blocking). `Ok(Some(path))`: a backup to restore.
fn run(
    data: &DataPaths,
    app: &AppPaths,
    what: &Bring,
    in_cloud: &InCloud,
    cancel: &Cancel,
    tx: &futures::channel::mpsc::UnboundedSender<Msg>,
) -> Result<Option<PathBuf>, String> {
    let localized = |e: SyncError| e.localized();
    let mut on_percent = |p: u8| {
        tx.unbounded_send(Msg::Percent(p)).ok();
    };
    let step = |s: Step| {
        tx.unbounded_send(Msg::Step(s)).ok();
    };
    match what {
        Bring::Backup(b) => backup(data, app, b, cancel).map(Some),
        Bring::Saves(pc) => {
            cloud::download_saves(data, app, pc, &saves_folder(data)?, cancel, &mut on_percent).map_err(localized)?;
            Ok(None)
        }
        Bring::Captures => {
            cloud::download_captures(data, app, &captures_folder(data)?, cancel, &mut on_percent).map_err(localized)?;
            Ok(None)
        }
        Bring::Everything(pc) => {
            if in_cloud.saves.contains(pc) {
                step(Step::Saves);
                cloud::download_saves(data, app, pc, &saves_folder(data)?, cancel, &mut on_percent).map_err(localized)?;
            }
            if in_cloud.captures {
                step(Step::Captures);
                cloud::download_captures(data, app, &captures_folder(data)?, cancel, &mut on_percent).map_err(localized)?;
            }
            // Last: its restore restarts the app.
            match in_cloud.backups.iter().find(|b| &b.pc == pc) {
                Some(b) => {
                    step(Step::Backup);
                    backup(data, app, b, cancel).map(Some)
                }
                None => Ok(None),
            }
        }
    }
}

/// Brings a backup into the backup folder.
fn backup(data: &DataPaths, app: &AppPaths, b: &CloudBackup, cancel: &Cancel) -> Result<PathBuf, String> {
    let folder = settings::load(data).map(|s| s.backup.folder).unwrap_or_default();
    if folder.trim().is_empty() {
        return Err(tr!("Choose a backup folder in Settings → Backups first.").into());
    }
    cloud::download_backup(data, app, b, Path::new(folder.trim()), cancel).map_err(|e| e.localized())
}

/// Ludusavi's backup folder: the one set in Settings → Saves, else the one
/// of the Ludusavi installed, else where its last backup went.
fn saves_folder(data: &DataPaths) -> Result<PathBuf, String> {
    let s = settings::load(data).map_err(|e| format!("{e:#}"))?;
    let l = &s.ludusavi;
    let configured = Some(l.backup_path.trim()).filter(|p| !p.is_empty() && !l.use_installed_config).map(PathBuf::from);
    let found = || {
        let preferred = Some(l.binary_path.as_str()).filter(|p| !p.is_empty());
        detect_ludusavi(&SystemRunner::default(), &SystemProbe, preferred).and_then(|d| d.backup_path).map(PathBuf::from)
    };
    let last = || l.last_backup_path.as_deref().and_then(|p| Path::new(p).parent()).map(Path::to_path_buf);
    configured.or_else(found).or_else(last).ok_or_else(|| tr!("Set Ludusavi's backup folder in Settings → Saves first.").to_string())
}

fn captures_folder(data: &DataPaths) -> Result<PathBuf, String> {
    let s = settings::load(data).map_err(|e| format!("{e:#}"))?;
    s.screenshots_folder()
        .map(Path::to_path_buf)
        .or_else(|| data.default_screenshots())
        .ok_or_else(|| tr!("Choose a screenshots folder in Settings → Screenshots first.").to_string())
}
