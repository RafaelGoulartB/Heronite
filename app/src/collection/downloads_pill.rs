//! The downloads pill of the Collection's header. While the queue has
//! something: a progress ring, the running game and its percent and speed;
//! hovering it shows the queue with pause / resume. Once the queue is empty,
//! what last finished (ready to play, failed) until the Downloads page is
//! opened.

use std::sync::OnceLock;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_epic::manager::DmState;
use heronite_epic::queue::{now_ms, DmElement, DmStatus, ElementKind};
use heronite_i18n::tr;

use super::header::Menu;
use super::CollectionView;
use crate::icons::Icon;
use crate::pages::{icon_action, status_pill, Page};
use crate::services::widgets::{duration_text, eta_secs, fmt_percent, progress_bar, progress_ring};
use crate::theme;
use crate::ui::controls::{icon_button_sm, ButtonKind};
use crate::ui::dropdown::Align;
use crate::ui::{glass, icon};

/// Waiting elements the popover lists.
const POPOVER_NEXT: usize = 3;
/// The pill shows this many characters of a title at most (the popover has
/// the whole one), and never wider than [`TITLE_W`].
const TITLE_CHARS: usize = 16;
const TITLE_W: f32 = 130.;

/// When the app started (ms): what finished before is old news.
fn started_at() -> u64 {
    static AT: OnceLock<u64> = OnceLock::new();
    *AT.get_or_init(now_ms)
}

/// The Collection's key of an element's game (`runner_appName`).
fn key_of(el: &DmElement) -> String {
    let runner = if el.params.runner.is_empty() { "legendary" } else { el.params.runner.as_str() };
    format!("{runner}_{}", el.params.app_name)
}

/// The queue as the popover lists it, read before its body is built.
struct QueueView {
    cur: DmElement,
    state: DmState,
    offline: bool,
    percent: Option<f64>,
    speed: Option<f64>,
    /// Seconds left of the running element.
    eta: Option<u64>,
    cover: Option<ImageSource>,
    next: Vec<(DmElement, Option<ImageSource>)>,
    more: usize,
}

impl CollectionView {
    pub(super) fn downloads_pill(&mut self, window: &Window, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        // Never starts the service: nothing to show before it runs.
        crate::services::try_downloads(cx)?;
        // Repaints the header on its progress.
        let svc = self.epic_service(cx);
        let s = svc.read(cx);
        let snap = s.snapshot();
        let Some(cur) = snap.queue.first().cloned() else {
            let seen = self.pages.downloads_seen.max(started_at());
            let last = snap.finished.iter().max_by_key(|e| e.end_time).filter(|e| e.end_time > seen).cloned()?;
            return self.downloads_notice(last, cx);
        };
        let progress = s.progress_of(&cur.params.app_name).cloned();
        let offline = s.paused_offline();
        let state = snap.state;
        let waiting: Vec<DmElement> = snap.queue[1..].to_vec();
        let scale = window.scale_factor();
        let cover = self.cover(&key_of(&cur).into(), 44., 58., scale, 50);
        let next = waiting.iter().take(POPOVER_NEXT).map(|e| (e.clone(), self.cover(&key_of(e).into(), 26., 35., scale, 40))).collect();
        let q = QueueView {
            percent: progress.as_ref().map(|p| p.percent),
            speed: progress.as_ref().map(|p| p.down_speed).filter(|_| state == DmState::Running),
            eta: progress.and_then(|p| eta_secs(&p.eta)).filter(|s| *s > 0),
            cur,
            state,
            offline,
            cover,
            next,
            more: waiting.len().saturating_sub(POPOVER_NEXT),
        };

        let (color, glyph): (Hsla, Option<Icon>) = match (q.state, q.offline) {
            (DmState::Running, _) => (theme::success().into(), None),
            (DmState::Paused, true) => (theme::warning().into(), Some(Icon::WifiOff)),
            (DmState::Paused, false) => (theme::accent().into(), Some(Icon::Pause)),
            (DmState::Idle, _) => (theme::accent().into(), Some(Icon::PlayArrow)),
        };
        let detail = match (q.state, q.offline) {
            (DmState::Running, _) => match (q.percent, q.speed) {
                (Some(p), Some(v)) => format!("{} \u{b7} {}", fmt_percent(p), speed_text(v)),
                _ => tr!("Starting...").to_string(),
            },
            (DmState::Paused, true) => tr!("Waiting for the connection").to_string(),
            (DmState::Paused, false) => tr!("Paused").to_string(),
            (DmState::Idle, _) => tr!("Waiting").to_string(),
        };
        let open = self.open_menu == Some(Menu::Downloads);
        let pill = pill_base("hdr-downloads", open)
            .child(ring(q.percent.unwrap_or(0.) as f32, color, glyph))
            .child(
                div()
                    .max_w(px(TITLE_W))
                    .truncate()
                    .text_size(px(13.))
                    .font_weight(theme::W_BOLD)
                    .text_color(theme::text_primary())
                    .child(clip_title(&q.cur.params.title())),
            )
            .child(div().flex_none().text_size(px(12.)).text_color(theme::text_dim()).child(detail))
            .when(!waiting.is_empty(), |d| {
                d.child(div().flex_none().pr(px(4.)).text_size(px(12.)).text_color(theme::text_dim()).child(format!("+{}", waiting.len())))
            });
        Some(self.hover_dropdown(Menu::Downloads, pill, Align::Right, move |_, cx| queue_popover(q, cx), cx))
    }

    /// What last finished, until the Downloads page is opened: a click opens it.
    fn downloads_notice(&mut self, el: DmElement, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let title = el.params.title();
        let (color, glyph, detail) = match el.status? {
            DmStatus::Done if el.kind == ElementKind::Install => (theme::success(), Icon::Check, tr!("ready to play")),
            DmStatus::Done => (theme::success(), Icon::Check, tr!("updated")),
            DmStatus::Error => (theme::danger_text(), Icon::Warning, tr!("failed \u{b7} see why")),
            DmStatus::Abort | DmStatus::Paused => return None,
        };
        let play = (el.status == Some(DmStatus::Done) && el.kind == ElementKind::Install).then(|| {
            let key = key_of(&el);
            icon_button_sm("hdr-downloads-play", ButtonKind::Primary, Icon::PlayArrow, tr!("Play"), true, {
                let view = cx.entity().downgrade();
                move |_, _, cx| {
                    cx.stop_propagation();
                    view.update(cx, |this, cx| {
                        this.pages.downloads_seen = now_ms();
                        this.primary_action(&key, cx);
                    })
                    .ok();
                }
            })
            .h(px(26.))
        });
        Some(
            pill_base("hdr-downloads", false)
                .border_color(Hsla { a: 0.4, ..color.into() })
                .child(ring(100., color.into(), Some(glyph)))
                .child(
                    div()
                        .max_w(px(TITLE_W))
                        .truncate()
                        .text_size(px(13.))
                        .font_weight(theme::W_BOLD)
                        .text_color(theme::text_primary())
                        .child(clip_title(&title)),
                )
                .child(div().flex_none().pr(px(4.)).text_size(px(12.)).text_color(theme::readable(color)).child(detail))
                .children(play)
                .tooltip(crate::ui::tooltip(tr!("Open Downloads")))
                .on_click(cx.listener(|this, _, window, cx| this.navigate(Page::Downloads, window, cx))),
        )
    }
}

pub(super) fn pill_base(id: &'static str, open: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .h(theme::TOOL_BTN)
        .pl(px(4.))
        .pr(px(10.))
        .rounded_full()
        .bg(if open { theme::search_fill_hover() } else { theme::search_fill() })
        .border_1()
        .border_color(theme::ink_a(0.1))
        .cursor_pointer()
        .hover(|s| s.bg(theme::search_fill_hover()))
}

/// 24px progress ring, with `glyph` in its middle.
pub(super) fn ring(percent: f32, color: Hsla, glyph: Option<Icon>) -> Div {
    div()
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(24.))
        .child(progress_ring(percent, 2.5, color).absolute().inset_0())
        .children(glyph.map(|g| icon(g, px(12.), color)))
}

/// `title` cut to [`TITLE_CHARS`] characters with an ellipsis.
fn clip_title(title: &str) -> String {
    if title.chars().count() <= TITLE_CHARS {
        return title.to_string();
    }
    let cut: String = title.chars().take(TITLE_CHARS - 1).collect();
    format!("{}\u{2026}", cut.trim_end())
}

fn speed_text(mib_s: f64) -> String {
    tr!("{speed} MiB/s", speed = heronite_i18n::decimal(mib_s, 1))
}

fn small_cover(src: Option<ImageSource>, w: f32, h: f32) -> Div {
    div()
        .flex_none()
        .w(px(w))
        .h(px(h))
        .rounded(px(5.))
        .overflow_hidden()
        .bg(theme::ink_a(0.05))
        .children(src.map(|s| img(s).size_full().object_fit(ObjectFit::Cover).rounded(px(5.))))
}

/// The pill's popover: the running element with pause / resume, the next
/// ones and the way to the Downloads page.
fn queue_popover(q: QueueView, cx: &mut Context<CollectionView>) -> AnyElement {
    let running = q.state == DmState::Running;
    let (status, color) = match (q.state, q.offline) {
        (DmState::Running, _) => (tr!("Downloading"), theme::success()),
        (DmState::Paused, true) => (tr!("Waiting for the connection"), theme::warning()),
        (DmState::Paused, false) => (tr!("Paused"), theme::accent()),
        (DmState::Idle, _) => (tr!("Waiting"), theme::accent()),
    };
    let mut facts: Vec<String> = q.percent.map(fmt_percent).into_iter().collect();
    facts.extend(q.speed.map(speed_text));
    facts.extend(q.eta.filter(|_| running).map(|secs| tr!("{time} left", time = duration_text(secs))));
    let toggle = icon_action("hdr-dl-toggle", if running { Icon::Pause } else { Icon::PlayArrow }, true)
        .tooltip(crate::ui::tooltip(if running { tr!("Pause") } else { tr!("Resume") }))
        .on_click(cx.listener(|this, _, _, cx| {
            if let Some(svc) = crate::services::try_downloads(cx) {
                svc.update(cx, |s, _| if s.is_running() { s.pause() } else { s.resume() });
            }
            this.open_menu = None;
            cx.notify();
        }));
    let current = div()
        .flex()
        .items_center()
        .gap(px(12.))
        .child(small_cover(q.cover, 44., 58.))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(4.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .min_w_0()
                        .child(div().min_w_0().truncate().text_size(px(14.)).font_weight(theme::W_BOLD).child(q.cur.params.title()))
                        .child(status_pill(status, color)),
                )
                .when(!facts.is_empty(), |d| d.child(div().text_size(px(12.)).text_color(theme::text_dim()).child(facts.join(" \u{b7} "))))
                .child(progress_bar(q.percent.map(|p| p as f32), 4., if running { theme::success() } else { color })),
        )
        .child(toggle);
    let mut panel = glass::glass_panel(theme::R_POPOVER).w(px(360.)).p(px(14.)).flex().flex_col().gap(px(4.)).child(current);
    if !q.next.is_empty() {
        panel = panel.child(div().mt(px(10.)).mb(px(2.)).text_size(px(12.)).text_color(theme::text_dim()).child(tr!("Up next")));
        for (i, (el, cover)) in q.next.into_iter().enumerate() {
            panel = panel.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .py(px(4.))
                    .child(div().w(px(14.)).text_size(px(12.)).text_color(theme::text_dim()).child((i + 1).to_string()))
                    .children(cover.map(|c| small_cover(Some(c), 26., 35.)))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).child(el.params.title()))
                    .children(el.params.size.as_deref().map(|size| {
                        div().flex_none().text_size(px(12.)).text_color(theme::text_dim()).child(heronite_epic::util::localize_size(size))
                    })),
            );
        }
    }
    let more =
        (q.more > 0).then(|| div().text_size(px(12.)).text_color(theme::text_dim()).child(tr!("+{count} in the queue", count = q.more)));
    panel
        .child(div().flex().items_center().gap(px(8.)).mt(px(10.)).children(more).child(div().flex_1()).child(icon_button_sm(
            "hdr-dl-open",
            ButtonKind::Glass,
            Icon::Download,
            tr!("Open Downloads"),
            true,
            {
                let view = cx.entity().downgrade();
                move |_, window, cx| {
                    view.update(cx, |this, cx| this.navigate(Page::Downloads, window, cx)).ok();
                }
            },
        )))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::clip_title;

    #[test]
    fn long_titles_are_cut_with_an_ellipsis() {
        assert_eq!(clip_title("Control"), "Control");
        assert_eq!(clip_title("Sixteen chars ok"), "Sixteen chars ok");
        assert_eq!(clip_title("Sid Meier's Civilization VI"), "Sid Meier's Civ\u{2026}");
        assert_eq!(clip_title("Tiny Tina's Wonderlands"), "Tiny Tina's Won\u{2026}");
        assert_eq!(clip_title("A Plague Tale: Requiem"), "A Plague Tale:\u{2026}");
    }
}
