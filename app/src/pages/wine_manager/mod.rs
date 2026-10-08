//! Wine Manager page: browse Wine/Proton builds (GE-Proton, Proton-CachyOS),
//! install / update / remove them. Port of Heroic's `screens/WineManager`
//! with this app's look. All work happens in `heronite_wine::WineManager`
//! (background threads); its state is owned by this entity, which the
//! Collection keeps alive, so downloads continue while another page is shown.
//!
//! Debug: `HERONITE_WINE_OFFLINE=1` disables every network request;
//! `HERONITE_DEBUG_WINE=progress,confirm,ge,install=<version>` fakes an install in progress, an
//! open removal confirmation, the GE-Proton tab, or a real install of `<version>`
//! (use a fixture dir!) for screenshots.

mod render;

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_wine::{Job, JobEvent, JobKind, JobPhase, Repository, WineConfig, WineManager, WineVersionInfo};

use super::{page_close, sheet, PageCtx, PageEvent};
use crate::ui::fields;

/// Refresh the release lists on open when the last fetch is older than this.
const STALE_AFTER_SECS: u64 = 24 * 3600;

pub struct WineManagerPage {
    ctx: PageCtx,
    manager: WineManager,
    repo: Repository,
    releases: Vec<WineVersionInfo>,
    loaded: bool,
    search: Entity<InputState>,
    query: String,
    /// Version whose removal awaits confirmation.
    confirm_remove: Option<String>,
    download_to_steam: bool,
    /// `WineManager::revision` last seen by `poll`.
    last_rev: u64,
    offline: bool,
    /// Screenshot helper: fake jobs shown instead of real ones.
    fake: HashMap<String, Job>,
}

impl EventEmitter<PageEvent> for WineManagerPage {}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn stamp_path(ctx: &PageCtx) -> std::path::PathBuf {
    ctx.app.config.join("wine-releases-fetched")
}


impl WineManagerPage {
    pub fn new(ctx: PageCtx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let manager = WineManager::new(ctx.heroic.clone());
        let search = fields::new_input(window, cx, "", "Search versions...");
        cx.subscribe_in(&search, window, |this, state, ev: &InputEvent, _window, cx| {
            if matches!(ev, InputEvent::Change) {
                this.query = state.read(cx).value().trim().to_lowercase();
                cx.notify();
            }
        })
        .detach();

        let debug = crate::env::value("HERONITE_DEBUG_WINE").unwrap_or_default();
        let debug_has = |k: &str| debug.split(',').any(|t| t == k);
        let mut this = Self {
            ctx,
            manager,
            repo: if debug_has("ge") { Repository::ProtonGe } else { Repository::LINUX[0] },
            releases: Vec::new(),
            loaded: false,
            search,
            query: String::new(),
            confirm_remove: None,
            download_to_steam: false,
            last_rev: 0,
            offline: crate::env::flag("HERONITE_WINE_OFFLINE"),
            fake: HashMap::new(),
        };
        let auto_install = debug.split(',').find_map(|t| t.strip_prefix("install=")).map(str::to_owned);
        this.init(debug_has("progress"), debug_has("confirm"), auto_install, cx);

        // Poll the manager (progress, finished jobs). Fast while busy.
        cx.spawn(async move |this, cx| loop {
            let busy = match this.update(cx, |this, cx| {
                this.poll(cx);
                this.manager.has_active_jobs()
            }) {
                Ok(b) => b,
                Err(_) => break,
            };
            let wait = if busy { 150 } else { 1000 };
            cx.background_executor().timer(Duration::from_millis(wait)).await;
        })
        .detach();
        this
    }

    /// Reads the cache off the UI thread and decides whether to refresh.
    fn init(&mut self, fake_progress: bool, fake_confirm: bool, auto_install: Option<String>, cx: &mut Context<Self>) {
        let heroic = self.ctx.heroic.clone();
        let stamp = stamp_path(&self.ctx);
        cx.spawn(async move |this, cx| {
            let (releases, cfg, age) = cx
                .background_spawn(async move {
                    let releases = heronite_wine::store::read_releases(&heroic);
                    let cfg = WineConfig::load(&heroic);
                    let fetched = std::fs::read_to_string(&stamp).ok().and_then(|s| s.trim().parse::<u64>().ok());
                    (releases, cfg, fetched.map(|t| now_secs().saturating_sub(t)))
                })
                .await;
            this.update(cx, |this, cx| {
                this.releases = releases;
                this.loaded = true;
                this.download_to_steam = cfg.download_proton_to_steam;
                if fake_progress {
                    this.install_fakes(fake_confirm);
                }
                if let Some(v) = &auto_install {
                    if let Some(r) = this.releases.iter().find(|r| &r.version == v) {
                        this.manager.start_install(r.clone());
                    }
                }
                // Heroic fetches when it has nothing cached; we also refresh
                // when the last fetch is stale. Never when offline.
                let stale = age.is_none_or(|a| a > STALE_AFTER_SECS);
                if !this.offline && (this.releases.is_empty() || stale) {
                    this.manager.start_refresh(Repository::LINUX.to_vec());
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn install_fakes(&mut self, confirm: bool) {
        let kind = self.repo.kind();
        let mut candidates = self.releases.iter().filter(|r| r.kind == kind && r.downloadable() && !r.installed());
        if let Some(r) = candidates.next() {
            let total = r.downsize.unwrap_or(500_000_000);
            self.fake.insert(
                r.version.clone(),
                Job {
                    kind: JobKind::Install,
                    phase: JobPhase::Downloading {
                        percentage: 42.3,
                        downloaded: (total as f64 * 0.423) as u64,
                        total,
                        speed: 18_400_000.,
                        eta_secs: Some(21),
                    },
                },
            );
        }
        if let Some(r) = candidates.next() {
            self.fake.insert(r.version.clone(), Job { kind: JobKind::Install, phase: JobPhase::Unzipping });
        }
        if confirm {
            if let Some(r) = self.releases.iter().find(|r| r.kind == kind && r.installed()) {
                self.confirm_remove = Some(r.version.clone());
            }
        }
    }

    fn toast(&self, text: impl Into<String>, error: bool, cx: &mut Context<Self>) {
        cx.emit(PageEvent::Toast(text.into(), error));
    }

    fn poll(&mut self, cx: &mut Context<Self>) {
        let rev = self.manager.revision();
        if rev == self.last_rev {
            return;
        }
        self.last_rev = rev;
        if log::log_enabled!(log::Level::Trace) {
            if let Some(r) = self.releases.iter().find_map(|r| self.manager.job(&r.version).map(|j| (r.version.clone(), j))) {
                log::trace!("wine manager poll: {} {:?}", r.0, r.1.phase);
            }
        }
        let mut reload = false;
        for ev in self.manager.take_events() {
            match ev {
                JobEvent::Installed { version, update } => {
                    self.toast(format!("{version} {}", if update { "updated" } else { "installed" }), false, cx);
                    reload = true;
                }
                JobEvent::Cancelled { version } => {
                    self.toast(format!("Installation of {version} canceled"), false, cx);
                }
                JobEvent::Removed { version } => {
                    self.toast(format!("{version} uninstalled"), false, cx);
                    reload = true;
                }
                JobEvent::Failed { version, message } => {
                    let first = message.lines().find(|l| !l.trim().is_empty()).unwrap_or("error").trim().to_owned();
                    self.toast(format!("{version}: {first}"), true, cx);
                    reload = true;
                }
                JobEvent::Refreshed { errors } => {
                    let _ = std::fs::create_dir_all(&self.ctx.app.config);
                    let _ = std::fs::write(stamp_path(&self.ctx), now_secs().to_string());
                    if let Some(e) = errors.first() {
                        self.toast(format!("Could not refresh everything: {e}"), true, cx);
                    }
                    reload = true;
                }
                JobEvent::RefreshFailed { message } => {
                    let first = message.lines().next().unwrap_or("error").to_owned();
                    self.toast(format!("Could not fetch wine versions: {first}"), true, cx);
                }
            }
        }
        if reload {
            self.reload(cx);
        }
        cx.notify();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let heroic = self.ctx.heroic.clone();
        cx.spawn(async move |this, cx| {
            let releases = cx.background_spawn(async move { heronite_wine::store::read_releases(&heroic) }).await;
            this.update(cx, |this, cx| {
                this.releases = releases;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn job(&self, version: &str) -> Option<Job> {
        self.manager.job(version).or_else(|| self.fake.get(version).cloned())
    }

    fn visible(&self) -> Vec<WineVersionInfo> {
        let kind = self.repo.kind();
        self.releases
            .iter()
            .filter(|r| r.kind == kind && r.downloadable())
            .filter(|r| self.query.is_empty() || r.version.to_lowercase().contains(&self.query))
            .cloned()
            .collect()
    }

    fn set_download_to_steam(&mut self, value: bool, cx: &mut Context<Self>) {
        self.download_to_steam = value;
        let heroic = self.ctx.heroic.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move { WineConfig::set_download_proton_to_steam(&heroic, value) })
                .await;
            if let Err(e) = res {
                this.update(cx, |this, cx| {
                    this.download_to_steam = !value;
                    this.toast(format!("Could not save the setting: {e:#}"), true, cx);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
        cx.notify();
    }
}

impl Render for WineManagerPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let toolbar = self.render_toolbar(cx);
        let tabs = self.render_tabs(cx);
        let steam = self.render_steam_row(cx);
        let info = self.render_info();
        let list = self.render_list(cx);
        let column = div()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(14.))
            .child(toolbar)
            .child(div().flex().flex_col().child(tabs).child(info))
            .child(steam)
            .child(list);
        let close = page_close(cx);
        sheet(
            "page-wine_manager",
            "Wine Manager",
            Some("Download and manage the Wine and Proton builds your games run on.".into()),
            None,
            1080.,
            close,
            column,
        )
    }
}
