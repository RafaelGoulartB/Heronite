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

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_wine::{Job, JobEvent, JobKind, JobPhase, Repository, WineConfig, WineManager, WineVersionInfo};

use super::{page_body, page_column, PageCtx, PageEvent};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button, checkbox, ButtonKind};
use crate::ui::fields;
use crate::ui::{icon, tool_button};

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

fn fmt_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024. && i < UNITS.len() - 1 {
        v /= 1024.;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

fn fmt_eta(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn open_path(target: &str) {
    if let Err(err) = std::process::Command::new("xdg-open").arg(target).spawn() {
        log::warn!("xdg-open {target}: {err}");
    }
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

        let debug = std::env::var("HERONITE_DEBUG_WINE").unwrap_or_default();
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
            offline: std::env::var_os("HERONITE_WINE_OFFLINE").is_some(),
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
                let stale = age.map_or(true, |a| a > STALE_AFTER_SECS);
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

    // ---- rendering -------------------------------------------------------

    fn render_toolbar(&self, cx: &mut Context<Self>) -> Div {
        let refreshing = self.manager.refreshing();
        let refresh = {
            let b = div()
                .id("wine-refresh")
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(40.))
                .rounded(px(8.))
                .border_1()
                .border_color(theme::white_a(0.2))
                .bg(theme::rgb_a(25, 28, 34, 0.8))
                .child(icon(Icon::Refresh, px(20.), if refreshing { theme::white_a(0.4) } else { rgb(0xcaf3fd).into() }));
            if refreshing || self.offline {
                b.cursor_not_allowed().opacity(0.8)
            } else {
                b.cursor_pointer()
                    .hover(|s| s.border_color(theme::accent()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.manager.start_refresh(Repository::LINUX.to_vec());
                        cx.notify();
                    }))
            }
        };
        div()
            .flex()
            .items_center()
            .gap(px(16.))
            .child(div().flex_1().max_w(px(420.)).child(fields::input_box(&self.search, false)))
            .child(refresh)
            .when(refreshing, |d| {
                d.child(div().text_size(px(13.33)).text_color(theme::white_a(0.7)).child("Fetching releases..."))
            })
            .child(div().flex_1())
            .child(
                checkbox("wine-to-steam", self.download_to_steam, "Install Proton into Steam (compatibilitytools.d)")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let v = !this.download_to_steam;
                        this.set_download_to_steam(v, cx);
                    })),
            )
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> Div {
        let mut row = div()
            .flex()
            .w_full()
            .gap(px(4.))
            .p(px(4.))
            .rounded(px(12.))
            .bg(theme::black_a(0.3));
        for repo in Repository::LINUX {
            let active = repo == self.repo;
            row = row.child(
                div()
                    .id(SharedString::from(format!("wine-tab-{}", repo.kind())))
                    .px(px(16.))
                    .py(px(8.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .text_size(px(14.5))
                    .font_weight(theme::W_BOLD)
                    .when(active, |d| d.bg(theme::background()).text_color(white()))
                    .when(!active, |d| d.text_color(theme::white_a(0.6)).hover(|s| s.text_color(white())))
                    .child(repo.kind())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.repo = repo;
                        this.confirm_remove = None;
                        cx.notify();
                    })),
            );
        }
        row
    }

    fn info_box(&self) -> Div {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(16.))
            .py(px(12.))
            .rounded(px(8.))
            .bg(theme::rgb_a(25, 28, 34, 0.8))
            .text_size(px(14.5))
            .text_color(rgb(0xcaf3fd))
            .child(icon(Icon::Check, px(16.), theme::success()))
            .child(self.repo.explanation())
    }

    fn render_row(&self, r: &WineVersionInfo, cx: &mut Context<Self>) -> Stateful<Div> {
        let job = self.job(&r.version);
        let installed = r.installed();
        let confirming = self.confirm_remove.as_deref() == Some(r.version.as_str());
        let key = r.version.clone();

        // Progress bar behind the row (Heroic's `--progress`).
        let (fraction, bar_alpha) = match job.as_ref().map(|j| &j.phase) {
            Some(JobPhase::Downloading { percentage, .. }) => ((*percentage / 100.) as f32, 0.30),
            Some(JobPhase::Unzipping) | Some(JobPhase::Removing) => (1.0, 0.30),
            Some(JobPhase::Starting) => (0.0, 0.30),
            None => (0.0, 0.0),
        };

        // Size / status column.
        let size_text: SharedString = match job.as_ref().map(|j| &j.phase) {
            Some(JobPhase::Downloading { percentage, speed, eta_secs, .. }) => {
                let mut s = format!("{percentage:.1}%  ·  {}/s", fmt_size(*speed as u64));
                if let Some(eta) = eta_secs {
                    s.push_str(&format!("  ·  ETA {}", fmt_eta(*eta)));
                }
                s.into()
            }
            Some(JobPhase::Unzipping) => "Unzipping...".into(),
            Some(JobPhase::Starting) => "Starting...".into(),
            Some(JobPhase::Removing) => "Removing...".into(),
            None => if installed { fmt_size(r.disksize) } else { fmt_size(r.downsize.unwrap_or(0)) }.into(),
        };

        let id = |p: &str| SharedString::from(format!("wine-{p}-{}", r.version));
        let mut actions = div().flex().items_center().justify_end().gap(px(6.)).w(px(210.)).flex_none();
        if confirming {
            let k = key.clone();
            let info = r.clone();
            actions = actions
                .child(div().text_size(px(13.)).text_color(theme::white_a(0.8)).child("Remove?"))
                .child(
                    div()
                        .id(id("yes"))
                        .px(px(10.))
                        .py(px(6.))
                        .rounded(px(4.))
                        .cursor_pointer()
                        .bg(theme::danger())
                        .text_color(white())
                        .text_size(px(12.5))
                        .font_weight(theme::W_BOLD)
                        .child("YES")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.confirm_remove = None;
                            this.manager.start_remove(info.clone());
                            cx.notify();
                        })),
                )
                .child(
                    button(id("no"), ButtonKind::Tertiary, "No").on_click(cx.listener(move |this, _, _, cx| {
                        if this.confirm_remove.as_deref() == Some(k.as_str()) {
                            this.confirm_remove = None;
                        }
                        cx.notify();
                    })),
                );
        } else if job.as_ref().map_or(false, |j| j.kind != JobKind::Remove) {
            let k = key.clone();
            actions = actions.child(
                button(id("cancel"), ButtonKind::Tertiary, "Cancel").on_click(cx.listener(move |this, _, _, cx| {
                    this.manager.cancel(&k);
                    cx.notify();
                })),
            );
        } else if job.is_some() {
            // removing: nothing to click
        } else if installed {
            if let Some(dir) = r.install_dir.clone() {
                actions = actions.child(
                    tool_button(id("folder"), Icon::FolderOpen, false)
                        .on_click(move |_, _, _| open_path(&dir)),
                );
            }
            if r.has_update() {
                let info = r.clone();
                actions = actions.child(
                    button(id("update"), ButtonKind::Secondary, "Update").on_click(cx.listener(move |this, _, _, cx| {
                        this.manager.start_install(info.clone());
                        cx.notify();
                    })),
                );
            }
            let k = key.clone();
            actions = actions.child(tool_button(id("remove"), Icon::FaTrash, false).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.confirm_remove = Some(k.clone());
                    cx.notify();
                },
            )));
        } else {
            let info = r.clone();
            actions = actions.child(
                button(id("install"), ButtonKind::Secondary, "Install").on_click(cx.listener(move |this, _, _, cx| {
                    this.manager.start_install(info.clone());
                    cx.notify();
                })),
            );
        }

        let notes = r.release_notes_link.clone();
        let version_cell = div()
            .flex()
            .flex_1()
            .min_w(px(180.))
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .id(id("notes"))
                    .cursor_pointer()
                    .text_size(px(15.2))
                    .text_color(rgb(0xcaf3fd))
                    .hover(|s| s.text_color(theme::accent()))
                    .when(installed, |d| d.font_weight(theme::W_BOLD))
                    .child(r.version.clone())
                    .when(!notes.is_empty(), |d| d.on_click(move |_, _, _| open_path(&notes))),
            )
            .when(installed && !r.has_update(), |d| {
                d.child(div().text_size(px(11.5)).text_color(theme::success()).child("INSTALLED"))
            })
            .when(r.has_update(), |d| {
                d.child(div().text_size(px(11.5)).text_color(rgb(0xf2c76e)).child("UPDATE AVAILABLE"))
            });

        div()
            .id(id("row"))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .h(px(52.))
            .px(px(24.))
            .overflow_hidden()
            .border_b_1()
            .border_color(theme::rgb_a(165, 220, 235, 0.25))
            .hover(|s| s.bg(theme::navbar_active_background()))
            .when(bar_alpha > 0., |d| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .w(relative(fraction))
                        .bg(theme::rgb_a(165, 220, 235, bar_alpha)),
                )
            })
            .child(version_cell)
            .child(div().w(px(130.)).flex_none().text_size(px(14.5)).text_color(theme::white_a(0.7)).child(r.date.clone()))
            .child(
                div()
                    .w(px(300.))
                    .flex_none()
                    .text_size(px(14.5))
                    .font_weight(theme::W_BOLD)
                    .text_color(theme::accent())
                    .child(size_text),
            )
            .child(actions)
    }

    fn render_list(&self, cx: &mut Context<Self>) -> Div {
        let rows = self.visible();
        let refreshing = self.manager.refreshing();
        if !self.loaded || (rows.is_empty() && refreshing) {
            return self.message("Loading wine versions...", false);
        }
        if rows.is_empty() {
            return if self.query.is_empty() {
                self.message("No Wine versions found. Please click the refresh icon to try again.", true)
            } else {
                self.message("No Wine versions match your search.", true)
            };
        }
        let header_cell = |t: &'static str| div().text_size(px(12.)).font_weight(theme::W_BOLD).text_color(white()).child(t);
        let mut list = div()
            .flex()
            .flex_col()
            .w_full()
            .rounded(px(16.))
            .overflow_hidden()
            .bg(theme::rgb_a(25, 28, 34, 0.8))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .px(px(24.))
                    .py(px(12.))
                    .bg(theme::black_a(0.25))
                    .border_b_1()
                    .border_color(theme::white_a(0.1))
                    .child(header_cell("WINE VERSION").flex_1())
                    .child(header_cell("RELEASE DATE").w(px(130.)).flex_none())
                    .child(header_cell("SIZE").w(px(300.)).flex_none())
                    .child(header_cell("ACTION").w(px(210.)).flex_none().flex().justify_end()),
            );
        for r in &rows {
            list = list.child(self.render_row(r, cx));
        }
        list
    }

    fn message(&self, text: &'static str, error: bool) -> Div {
        div()
            .flex()
            .justify_center()
            .px(px(16.))
            .py(px(24.))
            .rounded(px(8.))
            .bg(theme::rgb_a(25, 28, 34, 0.8))
            .text_size(px(15.2))
            .text_color(if error { theme::danger().into() } else { theme::white_a(0.8) })
            .child(text)
    }
}

impl Render for WineManagerPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let toolbar = self.render_toolbar(cx);
        let tabs = self.render_tabs(cx);
        let info = self.info_box();
        let list = self.render_list(cx);
        page_body("page-wine_manager").child(page_column().child(toolbar).child(tabs).child(info).child(list))
    }
}
