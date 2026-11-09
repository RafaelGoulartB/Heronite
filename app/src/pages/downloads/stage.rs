//! The top of the Downloads page: the running element over its wide art
//! (progress, speeds, when it ends, the speed chart, pause and stop), the
//! game that just finished installing, or what to do when nothing runs.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_epic::manager::DmState;
use heronite_epic::queue::{DmElement, DmStatus, ElementKind};
use heronite_i18n::{tr, trc};

use super::timing::{clock_in, relative_date, size_bytes, size_text};
use super::{folder_of, meta_line, play_key, poster, DownloadsPage, Overview};
use crate::icons::Icon;
use crate::pages::{icon_action, status_pill, PageEvent};
use crate::services::widgets::{duration_text, fmt_percent, progress_bar, sparkline};
use crate::theme;
use crate::ui::controls::{button_sm, icon_button_sm, ButtonKind};
use crate::ui::{icon, tooltip};

/// Height the wide art is requested at (the stage grows with its content).
const ART_H: f32 = 230.;
/// A finished install keeps the stage this long while nothing else runs.
const READY_FOR_MS: u64 = 30 * 60 * 1000;

impl DownloadsPage {
    pub(super) fn render_stage(&mut self, o: &Overview, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if let Some(cur) = o.snap.queue.first() {
            return self.render_running(cur, o, window, cx);
        }
        let ready = o.snap.finished.iter().max_by_key(|e| e.end_time).filter(|e| {
            e.status == Some(DmStatus::Done)
                && e.kind == ElementKind::Install
                && heronite_epic::queue::now_ms().saturating_sub(e.end_time) < READY_FOR_MS
        });
        match ready {
            Some(el) => self.render_ready(el, window, cx),
            None => idle_strip(cx).into_any_element(),
        }
    }

    /// The art of `el` cropped for the stage, under a fade from the panel's
    /// colour (left, where the text sits) to the art.
    fn stage_frame(&mut self, el: &DmElement, dim: bool, window: &Window) -> Div {
        // One size for every window width (each size is a texture of its
        // own); the image covers the stage whatever its width.
        let art = self.covers.get(el.params.art().as_deref(), super::MAX_W, ART_H, window.scale_factor());
        let (r, g, b) = theme::panel_rgb();
        div()
            .relative()
            .w_full()
            .rounded(px(20.))
            .overflow_hidden()
            .bg(theme::card_fill())
            .border_1()
            .border_color(theme::card_border())
            .children(art.map(|src| img(src).absolute().inset_0().size_full().object_fit(ObjectFit::Cover).rounded(px(20.))))
            .child(div().absolute().inset_0().rounded(px(20.)).bg(linear_gradient(
                90.,
                linear_color_stop(theme::rgba_hsla(r, g, b, 0.97), 0.38),
                linear_color_stop(theme::rgba_hsla(r, g, b, if dim { 0.82 } else { 0.66 }), 1.),
            )))
    }

    fn render_running(&mut self, cur: &DmElement, o: &Overview, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let scale = window.scale_factor();
        let running = o.snap.state == DmState::Running;
        let (status, color) = match o.snap.state {
            DmState::Running => (tr!("Downloading"), theme::success()),
            DmState::Paused if o.offline => (tr!("Waiting for the connection"), theme::warning()),
            DmState::Paused => (tr!("Paused"), theme::accent()),
            DmState::Idle => (tr!("Waiting"), theme::accent()),
        };
        let size = size_bytes(cur);
        let progress = o.progress.as_ref();
        let percent = progress.map(|p| p.percent);

        let mut main = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(status_pill(status, color))
                    .child(div().min_w_0().truncate().text_size(px(12.5)).text_color(theme::text_dim()).child(meta_line(cur))),
            )
            .child(
                div()
                    .mt(px(8.))
                    .mb(px(10.))
                    .truncate()
                    .font_family(theme::font_heading())
                    .text_size(px(26.))
                    .line_height(px(32.))
                    .font_weight(theme::W_BOLD)
                    .child(cur.params.title()),
            )
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(12.))
                    .mb(px(9.))
                    .child(
                        div()
                            .font_family(theme::font_heading())
                            .text_size(px(34.))
                            .line_height(px(38.))
                            .font_weight(theme::W_BOLD)
                            .child(percent.map_or_else(|| "\u{2014}".to_string(), fmt_percent)),
                    )
                    .children(size.zip(percent).map(|(s, p)| {
                        div().text_size(px(13.)).text_color(theme::text_muted()).child(tr!(
                            "{done} of {total}",
                            done = size_text(s * p / 100.),
                            total = size_text(s)
                        ))
                    })),
            )
            .child(progress_bar(percent.map(|p| p as f32), 10., if running { theme::success() } else { color }));

        main = match (o.snap.state, progress) {
            (DmState::Running, Some(p)) => main.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_x(px(24.))
                    .gap_y(px(8.))
                    .mt(px(14.))
                    .child(stat(trc!("speed", "Download"), speed_text(p.down_speed)))
                    .child(stat(tr!("Disk"), speed_text(p.disk_speed)))
                    .children(o.current_eta.map(|s| stat(trc!("time", "Left"), duration_text(s))))
                    .children(o.current_eta.map(|s| stat(tr!("Ends at"), clock_in(s)))),
            ),
            (DmState::Running, None) => main.child(message(Icon::Downloading, theme::success(), tr!("Starting..."))),
            (DmState::Paused, _) if o.offline => main.child(
                message(Icon::WifiOff, theme::warning(), tr!("The connection dropped. The download goes on by itself when it is back."))
                    .child(icon_button_sm("dm-try-now", ButtonKind::Glass, Icon::Refresh, tr!("Try now"), true, {
                        let svc = self.svc.downgrade();
                        move |_, _, cx| {
                            svc.update(cx, |s, _| s.resume()).ok();
                        }
                    })),
            ),
            (DmState::Paused, _) => main.child(message(
                Icon::Pause,
                theme::accent(),
                tr!("Paused. What is downloaded stays: it goes on where it stopped, even after closing the app."),
            )),
            (DmState::Idle, _) => main.child(message(Icon::PlayArrow, theme::accent(), tr!("Press play to start the queue"))),
        };

        let samples: Vec<f32> = self.svc.read(cx).speed_samples(&cur.params.app_name).iter().copied().collect();
        let peak = samples.iter().copied().fold(0., f32::max);
        let chart = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .w_full()
            .px(px(12.))
            .pt(px(9.))
            .pb(px(6.))
            .rounded(px(12.))
            .bg(theme::recess_a(0.25))
            .border_1()
            .border_color(theme::card_border())
            .child(
                div()
                    .flex()
                    .justify_between()
                    .gap(px(8.))
                    .text_size(px(11.5))
                    .text_color(theme::text_dim())
                    .child(tr!("Download speed"))
                    .when(peak > 0., |d| d.child(tr!("peak {speed}", speed = speed_text(peak as f64)))),
            )
            .child(
                div()
                    .h(px(56.))
                    .w_full()
                    .child(sparkline(samples, if running { theme::accent().into() } else { theme::text_dim().into() }).size_full()),
            );

        let toggle = div()
            .id("dm-pause")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(52.))
            .rounded_full()
            .bg(theme::accent())
            .cursor_pointer()
            .hover(|s| s.bg(theme::accent_overlay()))
            .tooltip(tooltip(if running { tr!("Pause") } else { tr!("Resume") }))
            .child(icon(if running { Icon::Pause } else { Icon::PlayArrow }, px(26.), theme::on_accent()))
            .on_click(cx.listener(|this, _, _, cx| {
                this.svc.update(cx, |s, _| if s.is_running() { s.pause() } else { s.resume() });
                cx.notify();
            }));
        let folder = folder_of(cur).map(|path| {
            icon_action("dm-folder", Icon::FolderOpen, true)
                .tooltip(tooltip(tr!("Open the folder")))
                .on_click(move |_, _, _| crate::ui::open_external(&path))
        });
        let asking = self.confirm_stop.as_deref() == Some(cur.params.app_name.as_str());
        let app = cur.params.app_name.clone();
        let stop =
            icon_action("dm-stop", Icon::Stop, true).tooltip(tooltip(tr!("Stop\u{2026}"))).on_click(cx.listener(move |this, _, _, cx| {
                this.confirm_stop = if asking { None } else { Some(app.clone()) };
                cx.notify();
            }));
        let side = div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(270.))
            .gap(px(16.))
            .items_end()
            .child(chart)
            .child(div().flex().items_center().gap(px(10.)).children(folder).child(stop).child(toggle));

        let cover = self.covers.get(cur.params.cover().as_deref(), 104., 139., scale);
        let body =
            div().relative().flex().items_center().gap(px(24.)).p(px(24.)).child(poster(cover, 104., 139., 10.)).child(main).child(side);
        let mut stage = self.stage_frame(cur, !running, window).child(body);
        if asking {
            let done = size.zip(percent).map(|(s, p)| size_text(s * p / 100.));
            stage = stage.child(self.stop_question(&cur.params.title(), done, cx));
        }
        stage.into_any_element()
    }

    /// "Stop the installation?" under the running element: keep it going,
    /// stop and keep the files (it resumes later) or stop and delete them.
    fn stop_question(&self, title: &str, done: Option<String>, cx: &mut Context<Self>) -> Div {
        let (r, g, b) = theme::panel_rgb();
        let detail = match done {
            Some(done) => tr!("{done} downloaded so far. Keeping the files, it can go on later where it stopped.", done),
            None => tr!("Keeping the files, it can go on later where it stopped.").to_string(),
        };
        div()
            .relative()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .px(px(24.))
            .py(px(14.))
            .border_t_1()
            .border_color(theme::card_border())
            .bg(theme::rgba_hsla(r, g, b, 0.96))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(260.))
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(theme::W_BOLD)
                            .text_color(theme::text_primary())
                            .child(tr!("Stop {title}?", title = title.to_string())),
                    )
                    .child(div().text_size(px(12.5)).text_color(theme::text_dim()).child(detail)),
            )
            .child(button_sm("dm-keep-running", ButtonKind::Ghost, tr!("Keep installing")).on_click(cx.listener(|this, _, _, cx| {
                this.confirm_stop = None;
                cx.notify();
            })))
            .child(button_sm("dm-stop-keep", ButtonKind::Glass, tr!("Stop, keep files")).on_click(cx.listener(|this, _, _, cx| {
                this.confirm_stop = None;
                this.svc.update(cx, |s, _| s.cancel_current(false));
                cx.notify();
            })))
            .child(button_sm("dm-stop-delete", ButtonKind::Danger, tr!("Stop, delete files")).on_click(cx.listener(|this, _, _, cx| {
                this.confirm_stop = None;
                this.svc.update(cx, |s, _| s.cancel_current(true));
                cx.notify();
            })))
    }

    /// The game that just finished installing, with Play.
    fn render_ready(&mut self, el: &DmElement, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let cover = self.covers.get(el.params.cover().as_deref(), 104., 139., window.scale_factor());
        let key = play_key(el);
        let mut facts = vec![tr!("Installed {when}", when = relative_date(el.end_time))];
        facts.extend(size_bytes(el).map(size_text));
        let folder = folder_of(el);
        facts.extend(folder.clone());
        let buttons = div()
            .flex()
            .flex_wrap()
            .gap(px(8.))
            .mt(px(18.))
            .child(icon_button_sm("dm-ready-play", ButtonKind::Primary, Icon::PlayArrow, tr!("Play"), true, {
                let page = cx.entity().downgrade();
                move |_, _, cx| {
                    page.update(cx, |_, cx| cx.emit(PageEvent::Play(key.clone()))).ok();
                }
            }))
            .children(folder.map(|path| {
                icon_button_sm("dm-ready-folder", ButtonKind::Glass, Icon::FolderOpen, tr!("Open the folder"), true, move |_, _, _| {
                    crate::ui::open_external(&path)
                })
            }));
        let main = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(div().flex().child(status_pill(tr!("Ready to play"), theme::success())))
            .child(
                div()
                    .mt(px(8.))
                    .mb(px(6.))
                    .truncate()
                    .font_family(theme::font_heading())
                    .text_size(px(26.))
                    .line_height(px(32.))
                    .font_weight(theme::W_BOLD)
                    .child(el.params.title()),
            )
            .child(div().truncate().text_size(px(12.5)).text_color(theme::text_dim()).child(facts.join(" \u{b7} ")))
            .child(buttons);
        self.stage_frame(el, false, window)
            .child(div().relative().flex().items_center().gap(px(24.)).p(px(24.)).child(poster(cover, 104., 139., 10.)).child(main))
            .into_any_element()
    }
}

/// Nothing runs: a slim card that points to where installs start.
fn idle_strip(cx: &mut Context<DownloadsPage>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(16.))
        .px(px(20.))
        .py(px(16.))
        .rounded(px(18.))
        .bg(theme::card_fill())
        .border_1()
        .border_color(theme::card_border())
        .child(div().flex().flex_none().items_center().justify_center().size(px(44.)).rounded(px(14.)).bg(theme::ink_a(0.05)).child(icon(
            Icon::Download,
            px(22.),
            theme::text_dim(),
        )))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    div()
                        .font_family(theme::font_heading())
                        .text_size(px(16.))
                        .font_weight(theme::W_BOLD)
                        .child(tr!("Nothing is downloading")),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(theme::text_dim())
                        .child(tr!("Install a game from the Collection or a store page and it shows up here.")),
                ),
        )
        .child(icon_button_sm("dm-to-collection", ButtonKind::Glass, Icon::GridView, tr!("Go to the Collection"), true, {
            let page = cx.entity().downgrade();
            move |_, _, cx| {
                page.update(cx, |_, cx| cx.emit(PageEvent::Navigate(crate::pages::Page::Collection))).ok();
            }
        }))
}

/// Empty queue and history: what the page is for and where to start.
pub(super) fn empty_page(cx: &mut Context<DownloadsPage>) -> Div {
    let tip = |ic: Icon, title: &'static str, text: &'static str| {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(200.))
            .max_w(px(260.))
            .gap(px(4.))
            .px(px(16.))
            .py(px(14.))
            .rounded(px(theme::R_TILE))
            .bg(theme::card_fill())
            .border_1()
            .border_color(theme::card_border())
            .child(icon(ic, px(20.), theme::accent()))
            .child(div().mt(px(4.)).text_size(px(13.5)).font_weight(theme::W_BOLD).child(title))
            .child(div().text_size(px(12.5)).text_color(theme::text_dim()).child(text))
    };
    let page = cx.entity().downgrade();
    let settings = cx.entity().downgrade();
    div()
        .flex()
        .flex_col()
        .items_center()
        .pt(px(56.))
        .pb(px(24.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(112.))
                .rounded_full()
                .bg(theme::accent_alpha(0.12))
                .border_1()
                .border_color(theme::accent_alpha(0.2))
                .child(icon(Icon::Download, px(44.), theme::accent())),
        )
        .child(
            div()
                .mt(px(26.))
                .font_family(theme::font_heading())
                .text_size(px(22.))
                .font_weight(theme::W_BOLD)
                .child(tr!("Nothing to download")),
        )
        .child(div().mt(px(6.)).max_w(px(470.)).text_center().text_size(px(13.5)).text_color(theme::text_dim()).child(tr!(
            "Install or update a game from your Collection and it shows up here, with its progress, its speed and when it is ready."
        )))
        .child(
            div()
                .flex()
                .gap(px(8.))
                .mt(px(20.))
                .child(icon_button_sm("dm-empty-collection", ButtonKind::Primary, Icon::GridView, tr!("Go to the Collection"), true, {
                    move |_, _, cx| {
                        page.update(cx, |_, cx| cx.emit(PageEvent::Navigate(crate::pages::Page::Collection))).ok();
                    }
                }))
                .child(icon_button_sm("dm-empty-settings", ButtonKind::Ghost, Icon::Tune, tr!("Download settings"), true, {
                    move |_, _, cx| {
                        crate::pages::settings::request_section(crate::pages::settings::Section::Downloads, cx);
                        settings.update(cx, |_, cx| cx.emit(PageEvent::Navigate(crate::pages::Page::Settings))).ok();
                    }
                })),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .justify_center()
                .gap(px(12.))
                .mt(px(40.))
                .child(tip(
                    Icon::Downloading,
                    tr!("One queue for everything"),
                    tr!("Epic Games, GOG and Amazon Games, one game at a time, in the order you choose."),
                ))
                .child(tip(
                    Icon::DragIndicator,
                    tr!("Drag to reorder"),
                    tr!("Want to play something first? Drag it to the top of the queue."),
                ))
                .child(tip(Icon::Pause, tr!("Pausing loses nothing"), tr!("Paused or closed the app: it goes on where it stopped."))),
        )
}

/// A labelled figure of the running element (speed, time left).
fn stat(label: &'static str, value: String) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(1.))
        .child(div().text_size(px(11.5)).text_color(theme::text_dim()).child(label))
        .child(div().font_family(theme::font_heading()).text_size(px(15.)).font_weight(theme::W_BOLD).child(value))
}

/// A line under the bar saying why nothing moves (paused, offline).
fn message(ic: Icon, color: Rgba, text: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(10.))
        .mt(px(12.))
        .child(icon(ic, px(18.), theme::readable(color)))
        .child(div().flex_1().min_w(px(200.)).text_size(px(13.)).text_color(theme::text_muted()).child(text.into()))
}

/// `42.3 MiB/s`.
pub(super) fn speed_text(mib_s: f64) -> String {
    tr!("{speed} MiB/s", speed = heronite_i18n::decimal(mib_s, 1))
}
