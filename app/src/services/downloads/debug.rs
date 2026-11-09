//! `HERONITE_DEBUG_*` hooks that drive the Epic service without the UI.

use std::time::Duration;

use gpui_kit::*;
use heronite_data::json_store;
use heronite_epic::cmd::Cancel;
use heronite_epic::manager::{Job, JobResult};
use heronite_epic::progress::Progress;
use heronite_epic::queue::{DmElement, DmStatus, ElementKind, InstallParams};
use serde_json::Value;

use super::eos::EosEntry;
use super::{library_entry, Downloads, EosGame, InstallRequest};

impl Downloads {
    /// `HERONITE_DEBUG_EPIC_OP=verify:<app>|uninstall:<app>[:prefix][:settings]|move:<app>:<dir>|import:<app>:<dir>|sync:<app>|install-dlc:<dlc>:<dir>|eos-install|eos-remove|eos-enable:<app>|eos-disable:<app>`:
    /// runs one operation at startup (for end-to-end checks with a fake legendary).
    pub(super) fn debug_op(&mut self, op: &str, cx: &mut Context<Self>) {
        let parts: Vec<&str> = op.split(':').collect();
        let app = parts.get(1).copied().unwrap_or_default().to_string();
        let install_path = library_entry(&self.data_paths, &app)
            .and_then(|g| g.pointer("/install/install_path").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default();
        match parts.first().copied() {
            Some("verify") => self.verify(&app, cx),
            Some("uninstall") => self.uninstall(&app, parts.contains(&"prefix"), parts.contains(&"settings"), cx),
            Some("move") => self.move_install(&app, &install_path, parts.get(2).copied().unwrap_or_default(), cx),
            Some("import") => self.import_game(&app, parts.get(2).copied().unwrap_or_default(), "Windows", None, None, cx),
            Some("sync") => self.sync_saves(&app, &app, cx),
            Some("install-dlc") => {
                // `install-dlc:<dlcApp>:<base folder>`
                if let Some(info) = library_entry(&self.data_paths, &app) {
                    self.install_dlc(info, parts.get(2).copied().unwrap_or_default(), "Windows", Some("2 GiB".into()), cx);
                }
            }
            Some("eos-install") => self.eos_install(cx),
            Some("eos-remove") => self.eos_remove(cx),
            Some("eos-enable") => {
                self.eos_games.insert(app.clone(), EosEntry::set(EosGame::Disabled));
                self.eos_toggle_game(&app, cx)
            }
            Some("eos-disable") => {
                self.eos_games.insert(app.clone(), EosEntry::set(EosGame::Enabled));
                self.eos_toggle_game(&app, cx)
            }
            _ => log::warn!("unknown HERONITE_DEBUG_EPIC_OP {op}"),
        }
    }

    /// `HERONITE_DEBUG_QUEUE_INSTALL=<appName>`: queues an install of that game
    /// into `$HERONITE_DEBUG_INSTALL_DIR` the way the install dialog would
    /// (any store's game; GOG / Amazon with a fake gogdl / nile through
    /// `HERONITE_GOGDL_BIN` / `HERONITE_NILE_BIN`).
    pub(super) fn debug_queue_install(&mut self, app_name: &str, cx: &mut Context<Self>) {
        let Some(game_info) = super::stores::any_library_entry(&self.data_paths, app_name) else {
            log::warn!("debug install: {app_name} is not in a store library");
            return;
        };
        let platform = if game_info.get("runner").and_then(Value::as_str) == Some("gog") { "windows" } else { "Windows" };
        let path = crate::env::value("HERONITE_DEBUG_INSTALL_DIR").unwrap_or_else(|| "/tmp/heronite-fake-games".into());
        let dlcs = crate::env::value("HERONITE_DEBUG_INSTALL_DLCS")
            .map(|d| d.split(',').filter(|s| !s.is_empty()).map(str::to_string).collect())
            .unwrap_or_default();
        self.queue_install(
            InstallRequest {
                game_info,
                path,
                platform: platform.into(),
                sdl_tags: Vec::new(),
                dlcs,
                language: String::new(),
                size: Some("1 GiB".into()),
                wine_prefix: None,
                wine_version: None,
                ignore_free_space: false,
            },
            cx,
        );
    }

    /// `HERONITE_DEBUG_FAKE_DOWNLOAD=<appName>`: a queue with a fake running
    /// install (no legendary), one waiting and one finished, for screenshots.
    pub(super) fn seed_fake(&mut self, app_name: &str) {
        let games = json_store::read_value(&self.data_paths.legendary_library()).unwrap_or(Value::Null);
        let list: Vec<Value> = games.get("library").and_then(Value::as_array).cloned().unwrap_or_default();
        let find = |app: &str| list.iter().find(|g| g.get("app_name").and_then(Value::as_str) == Some(app)).cloned();
        let mut chosen: Vec<Value> = Vec::new();
        if let Some(g) = find(app_name) {
            chosen.push(g);
        }
        for g in list.iter().filter(|g| g.get("app_name").and_then(Value::as_str) != Some(app_name)).take(3) {
            chosen.push(g.clone());
        }
        let mk = |g: &Value, kind: ElementKind, size: &str| {
            DmElement::new(
                kind,
                InstallParams {
                    app_name: g.get("app_name").and_then(Value::as_str).unwrap_or_default().to_string(),
                    path: "/tmp/heronite-fake/Games".into(),
                    runner: "legendary".into(),
                    platform_to_install: "Windows".into(),
                    game_info: g.clone(),
                    size: Some(size.into()),
                    ..Default::default()
                },
            )
        };
        let manager = self.manager.clone();
        if let Some(g) = chosen.first() {
            manager.add(mk(g, ElementKind::Install, "25.87 GiB"));
        }
        for (g, size) in chosen.iter().skip(1).take(2).zip(["4.2 GiB", "812.5 MiB"]) {
            manager.add(mk(g, ElementKind::Update, size));
        }
        // `HERONITE_DEBUG_FAKE_PAUSED=1`: the fake queue paused, as by the user.
        if crate::env::flag("HERONITE_DEBUG_FAKE_PAUSED") {
            manager.pause_by_user();
        }
    }
}

/// Debug job: slowly advances a percentage and never finishes.
pub(super) struct FakeJob;

impl Job for FakeJob {
    fn run(&self, _element: &DmElement, cancel: &Cancel, on_progress: &mut dyn FnMut(Progress)) -> JobResult {
        let mut percent = 37.4;
        while !cancel.is_requested() {
            on_progress(Progress { percent, downloaded_mib: percent * 120., eta: "00:08:20".into(), down_speed: 5.43, disk_speed: 5.12 });
            percent = (percent + 0.1).min(99.);
            std::thread::sleep(Duration::from_millis(1000));
        }
        JobResult { status: DmStatus::Abort, error: None }
    }
}
