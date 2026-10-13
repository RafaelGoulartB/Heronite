//! Install and update requests, queue control and the download manager's events.

use std::collections::HashSet;

use gpui_kit::*;
use heronite_amazon::cmd::Nile;
use heronite_amazon::ops::NileJob;
use heronite_data::DataPaths;
use heronite_epic::cmd::{Cancel, Legendary};
use heronite_epic::info;
use heronite_epic::manager::{DmState, Event as DmEvent, Job, JobResult};
use heronite_epic::ops::{self, LegendaryJob};
use heronite_epic::progress::Progress;
use heronite_epic::queue::{DmElement, DmStatus, ElementKind, InstallParams};
use heronite_epic::util::file_size;
use heronite_gog::cmd::Gogdl;
use heronite_gog::ops::GogJob;
use heronite_i18n::tr;
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
                let install = element.kind == ElementKind::Install;
                match element.status {
                    Some(DmStatus::Done) => {
                        self.auto_failed.remove(&element.params.app_name);
                        let done = if install {
                            tr!("{title}: installation finished", title = title.clone())
                        } else {
                            tr!("{title}: update finished", title = title.clone())
                        };
                        self.toast(cx, done, false);
                        if !crate::system::has_window(cx) {
                            crate::system::notify::send(
                                title.clone(),
                                if install { tr!("Installation finished") } else { tr!("Update finished") },
                            );
                        }
                        self.progress.remove(&element.params.app_name);
                        self.emit(cx, DownloadsEvent::LibraryChanged);
                        self.reload_local(cx);
                        if element.kind == ElementKind::Install {
                            // Desktop / menu / Steam shortcuts from the General settings.
                            let runner = if element.params.runner.is_empty() { "legendary" } else { element.params.runner.as_str() };
                            crate::system::shortcuts::spawn_on_game_added(
                                cx,
                                &self.data_paths,
                                runner,
                                &element.params.app_name,
                                |this, o, cx| {
                                    if let Some(m) = o.summary() {
                                        this.toast(cx, m.text, m.error);
                                    }
                                },
                            );
                        }
                    }
                    Some(DmStatus::Error) => {
                        if element.kind == ElementKind::Update {
                            self.auto_failed.insert(element.params.app_name.clone());
                        }
                        let why = error.unwrap_or_default();
                        let first = why.lines().next().unwrap_or("").to_string();
                        let failed = match (install, first.is_empty()) {
                            (true, true) => tr!("{title}: installation failed", title = title.clone()),
                            (true, false) => tr!("{title}: installation failed ({reason})", title = title.clone(), reason = first),
                            (false, true) => tr!("{title}: update failed", title = title.clone()),
                            (false, false) => tr!("{title}: update failed ({reason})", title = title.clone(), reason = first),
                        };
                        self.toast(cx, failed, true);
                        if !crate::system::has_window(cx) {
                            crate::system::notify::send(
                                title.clone(),
                                if install { tr!("Installation failed") } else { tr!("Update failed") },
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
        cx.notify();
    }

    /// Queues an install (and its DLCs) the way Heroic's `install` handler
    /// does. GOG installs the chosen DLCs with the game (gogdl takes them
    /// along); Epic DLCs are elements of their own.
    pub fn queue_install(&mut self, req: InstallRequest, cx: &mut Context<Self>) {
        let manager = self.manager.clone();
        let data_paths = self.data_paths.clone();
        let app_name = req.game_info.get("app_name").and_then(Value::as_str).unwrap_or_default().to_string();
        let runner = req.game_info.get("runner").and_then(Value::as_str).filter(|r| !r.is_empty()).unwrap_or("legendary").to_string();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if req.wine_prefix.is_some() || req.wine_version.is_some() {
                        ops::write_wine_config(
                            &data_paths,
                            &app_name,
                            req.wine_prefix.as_deref().unwrap_or(""),
                            req.wine_version.as_ref(),
                        )?;
                    }
                    let base = InstallParams {
                        app_name: app_name.clone(),
                        path: req.path.clone(),
                        install_dlcs: req.dlcs.clone(),
                        sdl_list: req.sdl_tags.clone(),
                        install_language: req.language.clone(),
                        runner: runner.clone(),
                        platform_to_install: req.platform.clone(),
                        game_info: req.game_info.clone(),
                        size: req.size.clone(),
                        ..Default::default()
                    };
                    manager.add(DmElement::new(ElementKind::Install, base.clone()));
                    let separate_dlcs = if runner == "legendary" { req.dlcs.as_slice() } else { &[] };
                    for dlc in separate_dlcs {
                        // The queue lists the DLC under its own title and cover.
                        let game_info = library_entry(&data_paths, dlc).unwrap_or_else(|| base.game_info.clone());
                        manager.add(DmElement::new(
                            ElementKind::Install,
                            InstallParams { app_name: dlc.clone(), install_dlcs: Vec::new(), game_info, size: None, ..base.clone() },
                        ));
                    }
                    anyhow::Ok(())
                })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(()) => this.toast(cx, tr!("Added to the download queue"), false),
                Err(err) => this.toast(cx, tr!("Could not queue the installation: {error}", error = format!("{err:#}")), true),
            })
            .ok();
        })
        .detach();
    }

    /// `updateGame`: queues an update of an installed game (any store).
    pub fn queue_update(&mut self, game_info: Value, cx: &mut Context<Self>) {
        let manager = self.manager.clone();
        let data_paths = self.data_paths.clone();
        let app = game_info.get("app_name").and_then(Value::as_str).unwrap_or_default().to_string();
        let runner = game_info.get("runner").and_then(Value::as_str).filter(|r| !r.is_empty()).unwrap_or("legendary").to_string();
        let path = game_info.pointer("/install/install_path").and_then(Value::as_str).unwrap_or_default().to_string();
        let platform = game_info.pointer("/install/platform").and_then(Value::as_str).unwrap_or("Windows").to_string();
        cx.background_spawn(async move {
            // GOG and Amazon only know the size of a full download, not of the update.
            let size = (runner == "legendary")
                .then(|| Legendary::locate(&data_paths).ok())
                .flatten()
                .and_then(|lg| info::install_info(&data_paths, &lg, &app, &platform).ok())
                .map(|i| file_size(i.download_size()));
            manager.add(DmElement::new(
                ElementKind::Update,
                InstallParams { app_name: app, path, runner, platform_to_install: platform, game_info, size, ..Default::default() },
            ));
        })
        .detach();
        self.toast(cx, tr!("Update added to the download queue"), false);
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

/// Locates the element's tool (legendary, gogdl or nile) when it runs (cheap
/// to create, and picks up a binary changed in the settings).
pub(super) struct LazyJob {
    pub(super) data_paths: DataPaths,
}

impl Job for LazyJob {
    fn run(&self, element: &DmElement, cancel: &Cancel, on_progress: &mut dyn FnMut(Progress)) -> JobResult {
        let paths = self.data_paths.clone();
        let failed = |err: anyhow::Error| JobResult { status: DmStatus::Error, error: Some(format!("{err:#}")) };
        match element.params.runner.as_str() {
            "gog" => match Gogdl::locate_or_install(&paths) {
                Ok(gogdl) => GogJob { paths, gogdl }.run(element, cancel, on_progress),
                Err(err) => failed(err),
            },
            "nile" => match Nile::locate_or_install(&paths) {
                Ok(nile) => NileJob { paths, nile }.run(element, cancel, on_progress),
                Err(err) => failed(err),
            },
            _ => match Legendary::locate(&paths) {
                Ok(legendary) => LegendaryJob { data_paths: paths, legendary }.run(element, cancel, on_progress),
                Err(err) => failed(err),
            },
        }
    }
}
