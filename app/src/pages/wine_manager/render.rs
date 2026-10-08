//! Rendering of the Wine Manager page: toolbar, tabs, the list of versions
//! and one row per version with its progress and actions.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_wine::{Job, JobKind, JobPhase, Repository, WineVersionInfo};

use super::WineManagerPage;
use crate::icons::Icon;
use crate::pages::{icon_action, status_pill};
use crate::services::widgets::progress_bar;
use crate::theme;
use crate::ui::controls::{button_sm, card, card_box, tabs, toggle_row, ButtonKind};
use crate::ui::fields;
use crate::ui::icon;

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

/// Progress bar percentage under a row (Heroic's `--progress`); `None` = no job.
fn job_fraction(job: Option<&Job>) -> Option<f32> {
    match job.map(|j| &j.phase) {
        Some(JobPhase::Downloading { percentage, .. }) => Some(*percentage as f32),
        Some(JobPhase::Unzipping) | Some(JobPhase::Removing) => Some(100.),
        Some(JobPhase::Starting) => Some(0.),
        None => None,
    }
}

/// Text of the size column: the job's progress, or the size on disk / to download.
fn size_text(job: Option<&Job>, r: &WineVersionInfo) -> SharedString {
    match job.map(|j| &j.phase) {
        Some(JobPhase::Downloading { percentage, speed, eta_secs, .. }) => {
            let mut s = format!("{percentage:.1}%  \u{b7}  {}/s", fmt_size(*speed as u64));
            if let Some(eta) = eta_secs {
                s.push_str(&format!("  \u{b7}  ETA {}", fmt_eta(*eta)));
            }
            s.into()
        }
        Some(JobPhase::Unzipping) => "Unzipping...".into(),
        Some(JobPhase::Starting) => "Starting...".into(),
        Some(JobPhase::Removing) => "Removing...".into(),
        None => if r.installed() { fmt_size(r.disksize) } else { fmt_size(r.downsize.unwrap_or(0)) }.into(),
    }
}

impl WineManagerPage {
    pub(super) fn render_toolbar(&self, cx: &mut Context<Self>) -> Div {
        let refreshing = self.manager.refreshing();
        let refresh = {
            let b = div()
                .id("wine-refresh")
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(40.))
                .rounded_full()
                .bg(theme::button_fill())
                .border_1()
                .border_color(theme::button_border())
                .child(icon(Icon::Refresh, px(18.), if refreshing { theme::text_dim() } else { theme::text_primary() }));
            if refreshing || self.offline {
                b.cursor_not_allowed().opacity(0.6)
            } else {
                b.cursor_pointer().hover(|s| s.bg(theme::button_fill_hover())).on_click(cx.listener(|this, _, _, cx| {
                    this.manager.start_refresh(Repository::LINUX.to_vec());
                    cx.notify();
                }))
            }
        };
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(div().flex().flex_1().max_w(px(460.)).child(fields::search_box(&self.search)))
            .child(refresh)
            .when(refreshing, |d| d.child(div().text_size(px(13.)).text_color(theme::text_muted()).child("Fetching releases...")))
    }

    pub(super) fn render_tabs(&self, cx: &mut Context<Self>) -> Div {
        let this = cx.entity();
        let items: Vec<(Repository, &'static str)> = Repository::LINUX.iter().map(|r| (*r, r.kind())).collect();
        tabs("wine-tab", &items, self.repo, move |repo, _, cx| {
            this.update(cx, |this, cx| {
                this.repo = repo;
                this.confirm_remove = None;
                cx.notify();
            });
        })
    }

    pub(super) fn render_steam_row(&self, cx: &mut Context<Self>) -> Div {
        card([toggle_row(
            "wine-to-steam",
            "Install Proton into Steam",
            Some("Downloads into Steam's compatibilitytools.d folder as well."),
            self.download_to_steam,
        )
        .on_click(cx.listener(|this, _, _, cx| {
            let v = !this.download_to_steam;
            this.set_download_to_steam(v, cx);
        }))
        .into_any_element()])
    }

    /// What the selected repository offers.
    pub(super) fn render_info(&self) -> Div {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .mx(px(6.))
            .mt(px(14.))
            .text_size(px(13.))
            .text_color(theme::text_muted())
            .child(icon(Icon::Check, px(16.), theme::success()))
            .child(div().flex_1().min_w_0().child(self.repo.explanation()))
    }

    pub(super) fn render_list(&self, cx: &mut Context<Self>) -> Div {
        let rows = self.visible();
        let refreshing = self.manager.refreshing();
        if !self.loaded || (rows.is_empty() && refreshing) {
            return message("Loading wine versions...", false);
        }
        if rows.is_empty() {
            return if self.query.is_empty() {
                message("No Wine versions found. Please click the refresh icon to try again.", true)
            } else {
                message("No Wine versions match your search.", true)
            };
        }
        let header_cell = |t: &'static str| div().text_size(px(12.)).text_color(theme::text_muted()).child(t);
        let mut list = card_box().child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .py(px(10.))
                .child(header_cell("Version").flex_1())
                .child(header_cell("Release date").w(px(110.)).flex_none())
                .child(header_cell("Size").w(px(260.)).flex_none())
                .child(header_cell("").w(px(230.)).flex_none()),
        );
        for r in &rows {
            list = list.child(div().h(px(1.)).w_full().bg(theme::row_separator())).child(self.render_row(r, cx));
        }
        list
    }

    fn render_row(&self, r: &WineVersionInfo, cx: &mut Context<Self>) -> Stateful<Div> {
        let job = self.job(&r.version);
        let fraction = job_fraction(job.as_ref());
        let size_color = if fraction.is_some() { theme::accent() } else { theme::text_muted() };

        div()
            .id(row_id("row", r))
            .flex()
            .flex_col()
            .flex_none()
            .child(
                div()
                    .flex()
                    .items_center()
                    .min_h(px(52.))
                    .child(version_cell(r))
                    .child(div().w(px(110.)).flex_none().text_size(px(13.)).text_color(theme::text_dim()).child(r.date.clone()))
                    .child(div().w(px(260.)).flex_none().text_size(px(13.)).text_color(size_color).child(size_text(job.as_ref(), r)))
                    .child(self.render_row_actions(r, job.as_ref(), cx)),
            )
            .children(fraction.map(|p| div().pb(px(10.)).child(progress_bar(Some(p), 4., theme::accent()))))
    }

    /// Buttons at the end of a row, by state: removal confirmation, running
    /// job (cancel), installed (open folder, update, remove) or available (install).
    fn render_row_actions(&self, r: &WineVersionInfo, job: Option<&Job>, cx: &mut Context<Self>) -> Div {
        let confirming = self.confirm_remove.as_deref() == Some(r.version.as_str());
        let key = r.version.clone();
        let mut actions = div().flex().items_center().justify_end().gap(px(6.)).w(px(230.)).flex_none();
        if confirming {
            let k = key.clone();
            let info = r.clone();
            actions = actions
                .child(div().mr(px(2.)).text_size(px(13.)).text_color(theme::text_muted()).child("Remove?"))
                .child(button_sm(row_id("yes", r), ButtonKind::Danger, "Remove").on_click(cx.listener(move |this, _, _, cx| {
                    this.confirm_remove = None;
                    this.manager.start_remove(info.clone());
                    cx.notify();
                })))
                .child(button_sm(row_id("no", r), ButtonKind::Ghost, "No").on_click(cx.listener(move |this, _, _, cx| {
                    if this.confirm_remove.as_deref() == Some(k.as_str()) {
                        this.confirm_remove = None;
                    }
                    cx.notify();
                })));
        } else if job.is_some_and(|j| j.kind != JobKind::Remove) {
            let k = key.clone();
            actions = actions.child(button_sm(row_id("cancel", r), ButtonKind::Glass, "Cancel").on_click(cx.listener(move |this, _, _, cx| {
                this.manager.cancel(&k);
                cx.notify();
            })));
        } else if job.is_some() {
            // Removing: nothing to click.
        } else if r.installed() {
            if let Some(dir) = r.install_dir.clone() {
                actions = actions.child(icon_action(row_id("folder", r), Icon::FolderOpen, true).on_click(move |_, _, _| open_path(&dir)));
            }
            if r.has_update() {
                let info = r.clone();
                actions = actions.child(button_sm(row_id("update", r), ButtonKind::Primary, "Update").on_click(cx.listener(move |this, _, _, cx| {
                    this.manager.start_install(info.clone());
                    cx.notify();
                })));
            }
            let k = key.clone();
            actions = actions.child(icon_action(row_id("remove", r), Icon::FaTrash, true).on_click(cx.listener(move |this, _, _, cx| {
                this.confirm_remove = Some(k.clone());
                cx.notify();
            })));
        } else {
            let info = r.clone();
            actions = actions.child(button_sm(row_id("install", r), ButtonKind::Glass, "Install").on_click(cx.listener(move |this, _, _, cx| {
                this.manager.start_install(info.clone());
                cx.notify();
            })));
        }
        actions
    }
}

fn row_id(part: &str, r: &WineVersionInfo) -> SharedString {
    SharedString::from(format!("wine-{part}-{}", r.version))
}

/// Version name (opens the release notes) with its "Installed" / "Update available" pill.
fn version_cell(r: &WineVersionInfo) -> Div {
    let installed = r.installed();
    let notes = r.release_notes_link.clone();
    div()
        .flex()
        .flex_1()
        .min_w(px(180.))
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .id(row_id("notes", r))
                .cursor_pointer()
                .text_size(px(14.))
                .text_color(theme::text_primary())
                .hover(|s| s.text_color(theme::accent()))
                .when(installed, |d| d.font_weight(theme::W_BOLD))
                .child(r.version.clone())
                .when(!notes.is_empty(), |d| d.on_click(move |_, _, _| open_path(&notes))),
        )
        .when(installed && !r.has_update(), |d| d.child(status_pill("Installed", theme::success())))
        .when(r.has_update(), |d| d.child(status_pill("Update available", rgb(0xf2c76e))))
}

/// Full-width card with a centred message (loading, empty, no match).
fn message(text: &'static str, error: bool) -> Div {
    card_box()
        .items_center()
        .py(px(24.))
        .text_size(px(14.))
        .text_color(if error { theme::danger_text() } else { theme::text_muted() })
        .child(text)
}
