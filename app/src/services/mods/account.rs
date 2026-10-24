//! The Nexus Mods account (the user's personal API key), and fuse-overlayfs.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use gpui_kit::*;
use heronite_i18n::tr;
use heronite_mods::nexus::{self, StoredAccount};
use heronite_mods::{tool, Progress};

use super::jobs::{Change, Msg, Outcome};
use super::{Job, JobStage, ModsEvent, ModsService};

impl ModsService {
    /// Reads `nexus.json` at start.
    pub(super) fn read_account(&mut self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        cx.spawn(async move |this, cx| {
            let account = cx.background_spawn(async move { nexus::load_account(&root) }).await;
            this.update(cx, |s, cx| {
                if account.is_some() {
                    s.account = account;
                    cx.emit(ModsEvent::Account);
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn validate_key(&mut self, key: String, cx: &mut Context<Self>) {
        let key = key.trim().to_string();
        let root = self.root.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if key.is_empty() {
                        return Err(tr!("Paste your Nexus Mods API key first").to_string());
                    }
                    let account = nexus::validate(&key).map_err(|e| tr!("Nexus Mods did not accept the key: {error}", error = e))?;
                    let stored = StoredAccount::new(&key, account);
                    nexus::save_account(&root, &stored).map_err(|e| tr!("Could not keep the key: {error}", error = e))?;
                    Ok(stored)
                })
                .await;
            this.update(cx, |s, cx| {
                match result {
                    Ok(stored) => {
                        let message = tr!("Connected to Nexus Mods as {name}", name = stored.account.name.clone());
                        s.account = Some(stored);
                        cx.emit(ModsEvent::Notice { game_key: String::new(), message, error: false });
                    }
                    Err(message) => cx.emit(ModsEvent::Notice { game_key: String::new(), message, error: true }),
                }
                cx.emit(ModsEvent::Account);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn forget_account(&mut self, cx: &mut Context<Self>) {
        self.account = None;
        cx.emit(ModsEvent::Account);
        let root = self.root.clone();
        cx.background_spawn(async move { nexus::forget_account(&root) }).detach();
    }

    /// Looks for fuse-overlayfs and FUSE again (Settings → Mods).
    pub fn refresh_support(&mut self, cx: &mut Context<Self>) {
        let bin = self.data_paths.root.join("tools/bin");
        cx.spawn(async move |this, cx| {
            let support = cx.background_spawn(async move { tool::support(&bin) }).await;
            this.update(cx, |s, cx| {
                if s.support.as_ref() != Some(&support) {
                    s.support = Some(support);
                    cx.emit(ModsEvent::Account);
                }
            })
            .ok();
        })
        .detach();
    }

    /// A job without a game that downloads fuse-overlayfs; its progress shows
    /// in `jobs(None)` with the `Tool` stage.
    pub(super) fn download_tool(&mut self, cx: &mut Context<Self>) {
        if self.jobs.iter().any(|j| j.stage == JobStage::Tool) {
            return;
        }
        let id = self.next_job_id();
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.work.cancels_insert(id, cancel.clone());
        self.jobs.push(Job {
            id,
            game_key: String::new(),
            name: "fuse-overlayfs".into(),
            stage: JobStage::Tool,
            progress: Progress::default(),
            error: None,
            dependencies: 0,
        });
        let (tx, bin) = (self.work.tx.clone(), self.data_paths.root.join("tools/bin"));
        let spawned = std::thread::Builder::new().name("mods-tool".into()).spawn({
            let tx = tx.clone();
            move || {
                let mut progress = |p: Progress| {
                    let _ = tx.unbounded_send(Msg::Update { job: id, change: Change::Progress(p) });
                };
                let outcome = match tool::ensure(&bin, &mut progress, &cancel) {
                    Ok(_) if cancel.load(Ordering::Relaxed) => Outcome::Cancelled,
                    Ok(_) => Outcome::Finished(tr!("fuse-overlayfs is ready: mods can be applied").to_string()),
                    Err(e) => Outcome::Failed(e),
                };
                let _ = tx.unbounded_send(Msg::Finished { job: id, outcome });
            }
        });
        if let Err(e) = spawned {
            let _ = tx.unbounded_send(Msg::Finished { job: id, outcome: Outcome::Failed(e.to_string()) });
        }
        cx.emit(ModsEvent::Jobs);
    }
}
