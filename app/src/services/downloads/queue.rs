//! Install and update requests, queue control and the download manager's events.

use std::collections::HashSet;

use gpui_kit::*;
use heronite_data::HeroicPaths;
use heronite_epic::cmd::{Cancel, Legendary};
use heronite_epic::manager::{DmState, Event as DmEvent, Job, JobResult};
use heronite_epic::ops::{self, LegendaryJob};
use heronite_epic::progress::Progress;
use heronite_epic::queue::{DmElement, DmStatus, ElementKind, InstallParams};
use heronite_epic::util::file_size;
use heronite_epic::info;
use serde_json::Value;

use super::{library_entry, Downloads, DownloadsEvent, InstallRequest, QueueState};

impl Downloads {
    /// Where the game is in the queue.
    pub fn queue_state(&self, app_name: &str) -> Option<QueueState> {
        let (index, _) = self.queued(app_name)?;
        Some(match (index, self.snapshot.state) {
            (0, DmState::Running) => QueueState::Downloading,
            (0, DmState::Paused) => QueueState::Paused,
            _ => QueueState::Waiting,
        })
    }

    pub fn is_running(&self) -> bool {
        self.snapshot.state == DmState::Running
    }

    /// The app is quitting: pauses the queue the way the Pause button does
    /// (the element stays queued and resumes at the next start). Does not
    /// block; see [`queue_idle`](Self::queue_idle).
    pub fn pause_for_shutdown(&mut self) {
        self.manager.pause_for_shutdown();
    }

    /// The queue's worker is gone (nothing of it is running any more).
    pub fn queue_idle(&self) -> bool {
        self.manager.is_idle()
    }

    pub(super) fn on_manager_event(&mut self, ev: DmEvent, cx: &mut Context<Self>) {
        match ev {
            DmEvent::Progress { app_name, progress } => {
                self.progress.insert(app_name, progress);
            }
            DmEvent::Changed => {
                self.snapshot = self.manager.snapshot();
                let alive: HashSet<&str> = self.snapshot.queue.iter().map(|e| e.params.app_name.as_str()).collect();
                self.progress.retain(|app, _| alive.contains(app.as_str()));
            }
            DmEvent::Finished { element, error } => {
                let title = element.params.title();
                let verb = if element.kind == ElementKind::Install { "Installation" } else { "Update" };
                match element.status {
                    Some(DmStatus::Done) => {
                        self.auto_failed.remove(&element.params.app_name);
                        self.toast(cx, format!("{title}: {} finished", verb.to_lowercase()), false);
                        if !crate::system::has_window(cx) {
                            crate::system::notify::send(title.clone(), format!("{verb} Finished"));
                        }
                        self.progress.remove(&element.params.app_name);
                        self.emit(cx, DownloadsEvent::LibraryChanged);
                        self.reload_local(cx);
                        if element.kind == ElementKind::Install {
                            // Desktop / menu / Steam shortcuts from the General settings.
                            crate::system::shortcuts::spawn_on_game_added(cx, &self.heroic, "legendary", &element.params.app_name, |this, o, cx| {
                                if let Some(m) = o.summary() {
                                    this.toast(cx, m.text, m.error);
                                }
                            });
                        }
                    }
                    Some(DmStatus::Error) => {
                        if element.kind == ElementKind::Update {
                            self.auto_failed.insert(element.params.app_name.clone());
                        }
                        let why = error.unwrap_or_default();
                        let first = why.lines().next().unwrap_or("").to_string();
                        self.toast(cx, format!("{title}: {} failed{}", verb.to_lowercase(), if first.is_empty() { String::new() } else { format!(" ({first})") }), true);
                        if !crate::system::has_window(cx) {
                            crate::system::notify::send(title.clone(), format!("{verb} Failed"));
                        }
                    }
                    _ => {}
                }
            }
        }
        cx.notify();
    }

    /// Queues an install (and its DLCs) the way Heroic's `install` handler does.
    pub fn queue_install(&mut self, req: InstallRequest, cx: &mut Context<Self>) {
        let manager = self.manager.clone();
        let heroic = self.heroic.clone();
        let app_name = req.game_info.get("app_name").and_then(Value::as_str).unwrap_or_default().to_string();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if req.wine_prefix.is_some() || req.wine_version.is_some() {
                        ops::write_wine_config(&heroic, &app_name, req.wine_prefix.as_deref().unwrap_or(""), req.wine_version.as_ref())?;
                    }
                    let base = InstallParams {
                        app_name: app_name.clone(),
                        path: req.path.clone(),
                        install_dlcs: req.dlcs.clone(),
                        sdl_list: req.sdl_tags.clone(),
                        install_language: req.language.clone(),
                        runner: "legendary".into(),
                        platform_to_install: req.platform.clone(),
                        game_info: req.game_info.clone(),
                        size: req.size.clone(),
                        ..Default::default()
                    };
                    manager.add(DmElement::new(ElementKind::Install, base.clone()));
                    for dlc in &req.dlcs {
                        // The queue lists the DLC under its own title and cover.
                        let game_info = library_entry(&heroic, dlc).unwrap_or_else(|| base.game_info.clone());
                        manager.add(DmElement::new(
                            ElementKind::Install,
                            InstallParams { app_name: dlc.clone(), install_dlcs: Vec::new(), game_info, size: None, ..base.clone() },
                        ));
                    }
                    anyhow::Ok(())
                })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(()) => this.toast(cx, "Added to the download queue", false),
                Err(err) => this.toast(cx, format!("Could not queue the installation: {err:#}"), true),
            })
            .ok();
        })
        .detach();
    }

    /// `updateGame`: queues an update of an installed game.
    pub fn queue_update(&mut self, game_info: Value, cx: &mut Context<Self>) {
        let manager = self.manager.clone();
        let heroic = self.heroic.clone();
        let app = game_info.get("app_name").and_then(Value::as_str).unwrap_or_default().to_string();
        let path = game_info.pointer("/install/install_path").and_then(Value::as_str).unwrap_or_default().to_string();
        let platform = game_info.pointer("/install/platform").and_then(Value::as_str).unwrap_or("Windows").to_string();
        cx.background_spawn(async move {
            let size = Legendary::locate(&heroic)
                .ok()
                .and_then(|lg| info::install_info(&heroic, &lg, &app, &platform).ok())
                .map(|i| file_size(i.download_size()));
            manager.add(DmElement::new(
                ElementKind::Update,
                InstallParams {
                    app_name: app,
                    path,
                    runner: "legendary".into(),
                    platform_to_install: platform,
                    game_info,
                    size,
                    ..Default::default()
                },
            ));
        })
        .detach();
        self.toast(cx, "Update added to the download queue", false);
    }

    pub fn pause(&mut self) {
        self.manager.pause();
    }

    pub fn resume(&mut self) {
        self.manager.resume();
    }

    pub fn cancel_current(&mut self, remove_downloaded: bool) {
        self.manager.cancel_current(remove_downloaded);
    }

    pub fn remove_from_queue(&mut self, app_name: &str) {
        self.manager.remove(app_name);
    }

    pub fn move_in_queue(&mut self, app_name: &str, up: bool) {
        self.manager.move_element(app_name, up);
    }

    pub fn clear_finished(&mut self) {
        self.manager.clear_finished();
    }

    pub fn clear_finished_item(&mut self, app_name: &str) {
        self.manager.clear_finished_item(app_name);
    }

    /// Installs one owned DLC of an installed game into the game's folder
    /// (Heroic's `DLC` row: a normal install of the DLC with the base game's
    /// path and platform, no components of its own).
    pub fn install_dlc(&mut self, dlc_info: Value, base_path: &str, platform: &str, size: Option<String>, cx: &mut Context<Self>) {
        self.queue_install(
            InstallRequest {
                game_info: dlc_info,
                path: base_path.to_string(),
                platform: platform.to_string(),
                sdl_tags: Vec::new(),
                dlcs: Vec::new(),
                language: "en-US".into(),
                size,
                wine_prefix: None,
                wine_version: None,
            },
            cx,
        );
    }
}

/// Locates legendary when the first element runs (cheap to create, and picks
/// up an `altLegendaryBin` changed in the settings).
pub(super) struct LazyJob {
    pub(super) heroic: HeroicPaths,
}

impl Job for LazyJob {
    fn run(&self, element: &DmElement, cancel: &Cancel, on_progress: &mut dyn FnMut(Progress)) -> JobResult {
        match Legendary::locate(&self.heroic) {
            Ok(legendary) => LegendaryJob { heroic: self.heroic.clone(), legendary }.run(element, cancel, on_progress),
            Err(err) => JobResult { status: DmStatus::Error, error: Some(format!("{err:#}")) },
        }
    }
}
