//! Statistics page (☰ → Statistics): play time of a week, month, year or
//! all time — four totals, an activity chart stacked by game, the most
//! played games and when in the week the playing happens. The numbers come
//! from `heronite_data::stats` over the Collection's loaded library (handed
//! in with [`StatsPage::set_library`]); nothing is read from disk here.

mod console;

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{Datelike, Local, NaiveDate, NaiveDateTime};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::stats::{self, fmt_hours_short, fmt_secs, month_short, GameInput, History, Period, Span, Stats, TOP_N};
use heronite_data::Library;
use heronite_images::ImageRef;

use super::{caption_with, columns, content_width, icon_action, list_item, page_close, sheet, status_pill, PageCtx, PageEvent};
use crate::icons::Icon;
use crate::services::widgets::{cover_box, Covers};
use crate::theme;
use crate::ui::controls::{button_sm, caption, card, card_box, row, tabs, ButtonKind};
use crate::ui::icon;

/// Games listed before "Show all".
const SHORT_LIST: usize = 8;
const CHART_H: f32 = 168.;
const AXIS_W: f32 = 40.;
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const WEEKDAYS_LONG: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const DAY_PARTS: [(&str, &str); 4] = [("Night", "0–6h"), ("Morning", "6–12h"), ("Afternoon", "12–18h"), ("Evening", "18–24h")];

pub struct StatsPage {
    covers: Covers,
    lib: Option<Arc<Library>>,
    chains: HashMap<SharedString, Vec<ImageRef>>,
    history: Option<Arc<History>>,
    period: Period,
    stats: Option<Stats>,
    hover_bar: Option<usize>,
    hover_cell: Option<(usize, usize)>,
    show_all: bool,
    /// Scroll of the console layout.
    console_scroll: ScrollHandle,
}

impl EventEmitter<PageEvent> for StatsPage {}

fn today() -> NaiveDate {
    Local::now().date_naive()
}

fn build_history(lib: &Library) -> History {
    stats::history(lib.games.iter().map(|g| GameInput {
        sessions: g.sessions.as_deref().map_or(&[], Vec::as_slice),
        lifetime_minutes: g.playtime_minutes,
        first_played: g.first_played.as_deref(),
    }))
}

impl StatsPage {
    pub fn new(ctx: PageCtx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let covers = Covers::new(&ctx.heroic, &ctx.app, cx, |this: &mut Self| &mut this.covers);
        // `HERONITE_DEBUG_PAGE=stats:year` opens on another span.
        let span = match crate::env::value("HERONITE_DEBUG_PAGE").as_deref().and_then(|p| p.strip_prefix("stats:")) {
            Some("week") => Span::Week,
            Some("year") => Span::Year,
            Some("all") => Span::All,
            _ => Span::Month,
        };
        Self {
            covers,
            lib: None,
            chains: HashMap::new(),
            history: None,
            period: Period::of(span, today()),
            stats: None,
            hover_bar: None,
            hover_cell: None,
            show_all: false,
            console_scroll: ScrollHandle::new(),
        }
    }

    /// The Collection's library (and its cover chains); the history is
    /// rebuilt off the UI thread when the library changed.
    pub fn set_library(&mut self, lib: Arc<Library>, chains: &HashMap<SharedString, Vec<ImageRef>>, cx: &mut Context<Self>) {
        if self.lib.as_ref().is_some_and(|l| Arc::ptr_eq(l, &lib)) {
            // Same data, maybe a new day: keep "today" right.
            self.recompute();
            return;
        }
        self.lib = Some(lib.clone());
        self.chains = chains.clone();
        cx.spawn(async move |this, cx| {
            let built = lib.clone();
            let history = cx.background_spawn(async move { build_history(&built) }).await;
            this.update(cx, |this, cx| {
                if this.lib.as_ref().is_some_and(|l| Arc::ptr_eq(l, &lib)) {
                    this.history = Some(Arc::new(history));
                    this.recompute();
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn recompute(&mut self) {
        self.stats = self.history.as_ref().map(|h| stats::compute(h, self.period, today()));
        self.hover_bar = None;
        self.hover_cell = None;
    }

    fn go(&mut self, period: Period, cx: &mut Context<Self>) {
        self.period = period;
        self.show_all = false;
        self.recompute();
        cx.notify();
    }

    fn set_span(&mut self, span: Span, cx: &mut Context<Self>) {
        let today = today();
        let anchor = if self.period.span == Span::All || self.period.contains(today) { today } else { self.period.start };
        self.go(Period::of(span, anchor), cx);
    }

    fn can_prev(&self) -> bool {
        self.period.span != Span::All && self.history.as_ref().and_then(|h| h.since).is_some_and(|s| s < self.period.start)
    }

    fn can_next(&self) -> bool {
        self.period.span != Span::All && self.period.end <= today()
    }

    fn game_key(&self, game: usize) -> Option<SharedString> {
        self.lib.as_ref()?.games.get(game).map(|g| SharedString::from(g.key.clone()))
    }

    fn game_title(&self, game: usize) -> String {
        self.lib.as_ref().and_then(|l| l.games.get(game)).map_or_else(String::new, |g| g.title.clone())
    }
}

// ---------------------------------------------------------------- texts

fn day_label(d: NaiveDate) -> String {
    let wd = WEEKDAYS[d.weekday().num_days_from_monday() as usize];
    if d.year() == today().year() {
        format!("{wd}, {} {}", month_short(d.month()), d.day())
    } else {
        format!("{wd}, {} {}, {}", month_short(d.month()), d.day(), d.year())
    }
}

/// "today", "yesterday", "3 days ago", "Oct 2", "Oct 2, 2025".
fn when_label(at: NaiveDateTime) -> String {
    let (d, today) = (at.date(), today());
    match (today - d).num_days() {
        0 => "today".into(),
        1 => "yesterday".into(),
        n @ 2..=6 => format!("{n} days ago"),
        _ if d.year() == today.year() => format!("{} {}", month_short(d.month()), d.day()),
        _ => format!("{} {}, {}", month_short(d.month()), d.day(), d.year()),
    }
}

fn bucket_label(span: Span, start: NaiveDate, end: NaiveDate) -> String {
    match span {
        Span::Week | Span::Month => day_label(start),
        _ if (end - start).num_days() > 31 => start.year().to_string(),
        _ => format!("{} {}", stats::month_name(start.month()), start.year()),
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// Under "Time played": the change against the previous period once this
/// one is over, its total while this one is still running.
fn compare_line(st: &Stats) -> (String, Option<Rgba>) {
    let Some(prev) = st.prev_secs else {
        let recorded: f64 = st.heat.iter().flatten().sum();
        return (format!("{} in recorded sessions", fmt_hours_short(recorded)), None);
    };
    let name = st.period.prev().map(|p| p.short_name()).unwrap_or_default();
    let over = st.period.end <= today();
    if prev <= 0. {
        (format!("Nothing played in {name}"), None)
    } else if over {
        let change = (st.total_secs / prev - 1.) * 100.;
        if change.abs() < 1. {
            (format!("Same as {name}"), None)
        } else if change > 0. {
            (format!("\u{25b2} {change:.0}% vs {name}"), Some(theme::success()))
        } else {
            (format!("\u{25bc} {:.0}% vs {name}", -change), Some(theme::danger_text()))
        }
    } else {
        (format!("{}: {}", capitalised(&name), fmt_secs(prev)), None)
    }
}

/// Axis step and top (hours) for a chart whose tallest bar is `max_h`.
fn nice_axis(max_h: f64) -> (f64, f64) {
    const STEPS: [f64; 13] = [0.25, 0.5, 1., 2., 3., 5., 10., 20., 25., 50., 100., 200., 500.];
    let step = STEPS.into_iter().find(|s| (max_h / s).ceil() <= 4.).unwrap_or(1000.);
    (step, ((max_h / step).ceil()).max(1.) * step)
}

fn axis_label(h: f64) -> String {
    if h == 0. {
        "0".into()
    } else if h < 1. {
        format!("{}m", (h * 60.).round())
    } else if h.fract() == 0. {
        format!("{h}h")
    } else {
        format!("{h:.1}h")
    }
}

/// Colour of chart slot `slot` (see `Stats::series`); `None` = "Other".
fn series_color(slot: Option<usize>) -> Hsla {
    match slot {
        Some(i) if i < TOP_N => theme::chart_series(i).into(),
        _ => theme::chart_other(),
    }
}

fn dot(color: Hsla) -> Div {
    div().flex_none().size(px(8.)).rounded_full().bg(color)
}

// ---------------------------------------------------------------- pieces

fn kpi(label: &str, value: impl IntoElement, sub: String, sub_color: Option<Rgba>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .min_w_0()
        .px(px(18.))
        .py(px(16.))
        .rounded(px(theme::R_CARD))
        .bg(theme::card_fill())
        .border_1()
        .border_color(theme::card_border())
        .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(label.to_string()))
        .child(value)
        .child(div().text_size(px(12.)).truncate().text_color(sub_color.unwrap_or(theme::text_dim())).child(sub))
}

fn big_number(text: String, suffix: Option<String>) -> Div {
    div()
        .flex()
        .items_baseline()
        .gap(px(4.))
        .font_family(theme::FONT_HEADING)
        .font_weight(theme::W_BOLD)
        .child(div().text_size(px(28.)).line_height(px(34.)).text_color(theme::text_primary()).child(text))
        .children(suffix.map(|s| div().text_size(px(16.)).text_color(theme::text_dim()).child(s)))
}

impl StatsPage {
    fn render_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let (can_prev, can_next) = (self.can_prev(), self.can_next());
        let label = match self.period.span {
            Span::All => self.history.as_ref().and_then(|h| h.since).map_or_else(String::new, |d| format!("Since {} {}", month_short(d.month()), d.year())),
            _ => self.period.label(),
        };
        let this = cx.entity().downgrade();
        let span_tabs = tabs(
            "stats-span",
            &[(Span::Week, "Week"), (Span::Month, "Month"), (Span::Year, "Year"), (Span::All, "All time")],
            self.period.span,
            move |span, _, cx| {
                this.update(cx, |p, cx| p.set_span(span, cx)).ok();
            },
        );
        let mut prev = icon_action("stats-prev", Icon::ChevronLeft, can_prev);
        if can_prev {
            prev = prev.on_click(cx.listener(|this, _, _, cx| {
                if let Some(p) = this.period.prev() {
                    this.go(p, cx);
                }
            }));
        }
        let mut next = icon_action("stats-next", Icon::ChevronRight, can_next);
        if can_next {
            next = next.on_click(cx.listener(|this, _, _, cx| {
                if let Some(p) = this.period.next() {
                    this.go(p, cx);
                }
            }));
        }
        let all = self.period.span == Span::All;
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .when(!all, |d| d.child(prev))
                    .child(
                        div()
                            .min_w(px(if all { 0. } else { 168. }))
                            .flex()
                            .justify_center()
                            .text_size(px(14.))
                            .font_weight(theme::W_BOLD)
                            .text_color(theme::text_primary())
                            .whitespace_nowrap()
                            .child(label),
                    )
                    .when(!all, |d| d.child(next)),
            )
            .child(span_tabs)
            .into_any_element()
    }

    fn render_kpis(&self, st: &Stats, cols: u16) -> Div {
        let all = st.period.span == Span::All;
        let (cmp, cmp_color) = compare_line(st);
        let time = kpi("Time played", big_number(if all { fmt_hours_short(st.total_secs) } else { fmt_secs(st.total_secs) }, None), cmp, cmp_color);
        let streak = if st.streak > 1 { format!("Longest streak: {} days", st.streak) } else { "No streak yet".into() };
        let days = kpi(
            "Days played",
            big_number(st.days_played.to_string(), (!all).then(|| format!("/ {}", st.days_elapsed))),
            streak,
            None,
        );
        let library = self.lib.as_ref().map_or(0, |l| l.games.len());
        let new = st.new_games();
        let games_sub = if all {
            format!("of {library} in your library")
        } else if new > 0 {
            format!("{} for the first time", plural(new, "played", "played"))
        } else {
            "No new games".into()
        };
        let games = kpi("Games", big_number(st.games.len().to_string(), None), games_sub, None);
        let avg = st.avg_session_secs();
        let session = kpi(
            "Typical session",
            big_number(if st.sessions > 0 { fmt_secs(avg) } else { "\u{2014}".into() }, None),
            format!("{} {}", stats::group_thousands(st.sessions as u64), if st.sessions == 1 { "session" } else { "sessions" }),
            None,
        );
        div().grid().grid_cols(cols).gap(px(14.)).w_full().child(time).child(days).child(games).child(session)
    }

    fn render_activity(&self, st: &Stats, cx: &mut Context<Self>) -> Div {
        let span = st.period.span;
        let today = today();
        let max = st.buckets.iter().map(|b| b.secs).fold(0., f64::max);
        let (step, top) = nice_axis(max / 3600.);
        let top_secs = top * 3600.;
        let lines = (top / step).round() as usize;

        // Detail line: the hovered bar, or the busiest one.
        let shown = self.hover_bar.and_then(|i| st.buckets.get(i).map(|b| (b, true))).or_else(|| {
            st.buckets.iter().filter(|b| b.secs > 0.).max_by(|a, b| a.secs.total_cmp(&b.secs)).map(|b| (b, false))
        });
        let detail = match shown {
            Some((b, hovered)) => {
                let what = bucket_label(span, b.start, b.end);
                let lead = if hovered {
                    what
                } else {
                    let unit = match span {
                        Span::Week | Span::Month => "day",
                        _ if (b.end - b.start).num_days() > 31 => "year",
                        _ => "month",
                    };
                    format!("Busiest {unit}: {what}")
                };
                let mut parts: Vec<(Hsla, String)> = b
                    .top
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| **s >= 60.)
                    .map(|(i, s)| (series_color(Some(i)), format!("{} {}", self.game_title(st.series[i]), fmt_secs(*s))))
                    .collect();
                let other = b.secs - b.top.iter().sum::<f64>();
                if hovered && other >= 60. {
                    parts.push((theme::chart_other(), format!("Other {}", fmt_secs(other))));
                }
                let breakdown = div().flex().items_center().gap(px(12.)).min_w_0().overflow_hidden().children(
                    parts.into_iter().take(if hovered { 6 } else { 3 }).map(|(c, t)| {
                        div().flex().flex_none().items_center().gap(px(6.)).child(dot(c)).child(div().whitespace_nowrap().child(t))
                    }),
                );
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .min_w_0()
                    .text_size(px(12.))
                    .text_color(theme::text_muted())
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(13.))
                            .text_color(theme::text_primary())
                            .child(format!("{lead} \u{b7} {}", fmt_secs(b.secs))),
                    )
                    .when(b.secs > 0., |d| d.child(breakdown))
            }
            None => div(),
        };

        let axis_x = AXIS_W;
        let grid = (0..=lines).map(|i| {
            let y = CHART_H * i as f32 / lines as f32;
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(y))
                .h(px(1.))
                .bg(theme::white_a(if i == 0 { 0.12 } else { 0.05 }))
        });
        let labels = (0..=lines).map(|i| {
            let y = CHART_H * i as f32 / lines as f32;
            div()
                .absolute()
                .right(px(8.))
                .bottom(px(y - 7.))
                .text_size(px(11.))
                .line_height(px(14.))
                .text_color(theme::text_dim())
                .child(axis_label(step * i as f64))
        });

        let n = st.buckets.len();
        let bar_w = if n <= 12 { 34. } else { 22. };
        let bars = st.buckets.iter().enumerate().map(|(i, b)| {
            let hovered = self.hover_bar == Some(i);
            let mut bar = div().w_full().max_w(px(bar_w)).flex().flex_col().rounded_t(px(4.)).overflow_hidden();
            let other = (b.secs - b.top.iter().sum::<f64>()).max(0.);
            let segs = std::iter::once((other, theme::chart_other())).chain((0..TOP_N).rev().map(|r| (b.top[r], series_color(Some(r)))));
            for (secs, color) in segs {
                let h = (secs / top_secs) as f32 * CHART_H;
                if h >= 0.5 {
                    bar = bar.child(div().w_full().h(px(h)).bg(color));
                }
            }
            if b.secs <= 0. && !b.future {
                bar = bar.child(div().w_full().h(px(2.)).bg(theme::white_a(0.1)));
            }
            div()
                .id(("stats-bar", i))
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .justify_end()
                .items_center()
                .px(px(if n > 20 { 2. } else { 5. }))
                .rounded_t(px(6.))
                .when(hovered, |d| d.bg(theme::white_a(0.05)))
                .on_hover(cx.listener(move |this, on: &bool, _, cx| {
                    if *on {
                        this.hover_bar = Some(i);
                    } else if this.hover_bar == Some(i) {
                        this.hover_bar = None;
                    }
                    cx.notify();
                }))
                .child(bar)
        });

        let years = (n > 0 && (st.buckets[n - 1].end - st.buckets[0].start).num_days() > 400) || span == Span::All;
        let x_labels = st.buckets.iter().enumerate().map(|(i, b)| {
            let current = b.start <= today && today < b.end;
            let (line1, line2) = match span {
                Span::Week => (WEEKDAYS[i % 7].to_string(), Some(b.start.day().to_string())),
                Span::Month => (b.start.day().to_string(), None),
                _ if (b.end - b.start).num_days() > 31 => (b.start.year().to_string(), None),
                _ => (
                    month_short(b.start.month()).to_string(),
                    (years && (i == 0 || b.start.month() == 1)).then(|| format!("{}", b.start.year())),
                ),
            };
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .items_center()
                .text_size(px(11.))
                .line_height(px(14.))
                .whitespace_nowrap()
                .text_color(if current { theme::accent() } else { theme::text_dim() })
                .when(current, |d| d.font_weight(theme::W_BOLD))
                .child(line1)
                .children(line2.map(|t| div().text_color(theme::text_dim()).font_weight(theme::W_REGULAR).child(t)))
        });

        let legend = div().flex().flex_none().items_center().gap(px(14.)).children(st.series.iter().enumerate().map(|(r, &game)| {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .max_w(px(170.))
                .text_size(px(12.))
                .text_color(theme::text_muted())
                .child(dot(series_color(Some(r))))
                .child(div().truncate().child(self.game_title(game)))
        }));
        let has_other = st.buckets.iter().any(|b| b.secs - b.top.iter().sum::<f64>() >= 1.);

        card_box()
            .px(px(18.))
            .py(px(16.))
            .gap(px(14.))
            .child(div().flex().items_center().justify_between().gap(px(16.)).h(px(20.)).child(detail))
            .child(
                div()
                    .flex()
                    .child(div().relative().flex_none().w(px(axis_x)).h(px(CHART_H)).children(labels))
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .h(px(CHART_H))
                            .children(grid)
                            .child(div().absolute().inset_0().flex().items_end().children(bars)),
                    ),
            )
            .child(div().flex().pl(px(axis_x)).mt(px(-6.)).children(x_labels))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(14.))
                    .pl(px(axis_x))
                    .child(legend)
                    .when(has_other, |d| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .text_size(px(12.))
                                .text_color(theme::text_muted())
                                .child(dot(theme::chart_other()))
                                .child("Other games"),
                        )
                    }),
            )
    }

    fn render_games(&mut self, st: &Stats, window: &Window, cx: &mut Context<Self>) -> Div {
        let scale = window.scale_factor();
        let max = st.games.first().map_or(1., |g| g.secs.max(1.));
        let total = st.total_secs.max(1.);
        let limit = if self.show_all { st.games.len() } else { SHORT_LIST };
        let mut rows = Vec::new();
        for g in st.games.iter().take(limit) {
            let Some(key) = self.game_key(g.game) else { continue };
            let title = self.game_title(g.game);
            let cover = self.chains.get(&key).and_then(|c| self.covers.get_chain(&key, c, 36., 48., scale));
            let meta = match (g.sessions, g.last) {
                (0, _) => "No recorded sessions".to_string(),
                (n, Some(last)) => format!("{} \u{b7} last {}", plural(n, "session", "sessions"), when_label(last)),
                (n, None) => plural(n, "session", "sessions"),
            };
            let content = div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .min_w_0()
                        .child(div().truncate().text_size(px(14.)).text_color(theme::text_primary()).child(title))
                        .when(g.new, |d| d.child(status_pill("New", theme::accent()))),
                )
                .child(
                    div()
                        .h(px(5.))
                        .w_full()
                        .rounded_full()
                        .bg(theme::white_a(0.06))
                        .child(div().h_full().w(relative((g.secs / max) as f32)).rounded_full().bg(series_color(st.series_of(g.game)))),
                )
                .child(div().text_size(px(12.)).text_color(theme::text_dim()).truncate().child(meta));
            let trailing = div()
                .flex()
                .flex_col()
                .items_end()
                .min_w(px(64.))
                .child(div().text_size(px(14.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(fmt_secs(g.secs)))
                .child(div().text_size(px(12.)).text_color(theme::text_dim()).child(format!("{:.0}%", g.secs / total * 100.)));
            let k = key.clone();
            rows.push(
                div()
                    .id(SharedString::from(format!("stats-game-{key}")))
                    .mx(px(-8.))
                    .px(px(8.))
                    .rounded(px(10.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::white_a(0.04)))
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(PageEvent::ShowGame(k.to_string()))))
                    .child(list_item(Some(cover_box(cover, 36., 48.).into_any_element()), content, trailing))
                    .into_any_element(),
            );
        }
        let more = st.games.len().saturating_sub(SHORT_LIST);
        let toggle = (more > 0).then(|| {
            let label = if self.show_all { "Show less".to_string() } else { format!("Show all {}", st.games.len()) };
            button_sm("stats-show-all", ButtonKind::Ghost, label).on_click(cx.listener(|this, _, _, cx| {
                this.show_all = !this.show_all;
                cx.notify();
            }))
        });
        div().child(caption_with("Most played", div().children(toggle))).child(card(rows))
    }

    fn render_habits(&self, st: &Stats, cx: &mut Context<Self>) -> Div {
        let day_totals: Vec<f64> = st.heat.iter().map(|r| r.iter().sum()).collect();
        let in_sessions: f64 = day_totals.iter().sum();
        let max_cell = st.heat.iter().flatten().copied().fold(0., f64::max).max(1.);
        let argmax = |v: &[f64]| v.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map_or(0, |(i, _)| i);
        let best_day = argmax(&day_totals);
        let best_part = argmax(&st.day_parts);

        let summary = match self.hover_cell {
            Some((d, h)) => format!(
                "{}s, {h:02}:00\u{2013}{:02}:00 \u{b7} {}",
                WEEKDAYS_LONG[d],
                (h + 1) % 24,
                fmt_secs(st.heat[d][h])
            ),
            None => format!(
                "You play most on {}s, usually in the {}.",
                WEEKDAYS_LONG[best_day],
                DAY_PARTS[best_part].0.to_lowercase()
            ),
        };

        let accent: Hsla = theme::accent().into();
        let mut heat = div().flex().flex_col().gap(px(3.));
        for (d, hours) in st.heat.iter().enumerate() {
            let mut line = div().flex().items_center().gap(px(3.)).child(
                div().flex_none().w(px(34.)).text_size(px(11.)).text_color(theme::text_dim()).child(WEEKDAYS[d]),
            );
            for (h, v) in hours.iter().enumerate() {
                let t = (*v / max_cell) as f32;
                let color = if *v <= 0. { theme::white_a(0.04) } else { Hsla { a: 0.12 + 0.88 * t.powf(0.7), ..accent } };
                let hovered = self.hover_cell == Some((d, h));
                line = line.child(
                    div()
                        .id(("stats-cell", d * 24 + h))
                        .flex_1()
                        .h(px(18.))
                        .rounded(px(4.))
                        .bg(color)
                        .when(hovered, |c| c.border_1().border_color(theme::white_a(0.7)))
                        .on_hover(cx.listener(move |this, on: &bool, _, cx| {
                            if *on {
                                this.hover_cell = Some((d, h));
                            } else if this.hover_cell == Some((d, h)) {
                                this.hover_cell = None;
                            }
                            cx.notify();
                        })),
                );
            }
            heat = heat.child(line);
        }
        let hours = div().flex().gap(px(3.)).child(div().flex_none().w(px(34.))).children((0..24).map(|h| {
            div()
                .flex_1()
                .text_size(px(11.))
                .text_color(theme::text_dim())
                .children((h % 6 == 0).then(|| format!("{h:02}")))
        }));

        let parts = div().grid().grid_cols(4).gap(px(10.)).children(DAY_PARTS.iter().enumerate().map(|(i, (name, range))| {
            let share = if in_sessions > 0. { st.day_parts[i] / in_sessions } else { 0. };
            let best = i == best_part && in_sessions > 0.;
            div()
                .flex()
                .flex_col()
                .gap(px(5.))
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(6.))
                        .child(div().text_size(px(13.)).text_color(theme::text_primary()).child(name.to_string()))
                        .child(div().text_size(px(11.)).text_color(theme::text_dim()).child(range.to_string())),
                )
                .child(
                    div()
                        .font_family(theme::FONT_HEADING)
                        .font_weight(theme::W_BOLD)
                        .text_size(px(18.))
                        .text_color(if best { theme::accent() } else { theme::text_primary() })
                        .child(format!("{:.0}%", share * 100.)),
                )
                .child(
                    div()
                        .h(px(4.))
                        .w_full()
                        .rounded_full()
                        .bg(theme::white_a(0.06))
                        .child(div().h_full().w(relative(share as f32)).rounded_full().bg(if best { accent } else { theme::white_a(0.35) })),
                )
        }));

        let when = card_box()
            .px(px(18.))
            .py(px(16.))
            .gap(px(14.))
            .child(div().h(px(20.)).text_size(px(13.)).text_color(theme::text_primary()).truncate().child(summary))
            .child(div().flex().flex_col().gap(px(6.)).child(heat).child(hours))
            .child(div().h(px(1.)).bg(theme::row_separator()))
            .child(parts);

        // Highlights.
        let mut rows: Vec<AnyElement> = Vec::new();
        if let Some(l) = st.longest {
            let desc = format!("{} \u{b7} {}", self.game_title(l.game), day_label(l.start.date()));
            rows.push(row("Longest session", Some(&desc), value_text(fmt_secs(l.secs))).into_any_element());
        }
        if let Some((d, secs)) = st.busiest {
            rows.push(row("Busiest day", Some(&day_label(d)), value_text(fmt_secs(secs))).into_any_element());
        }
        if in_sessions > 0. {
            let share = st.weekend_secs / in_sessions * 100.;
            let desc = format!("{} on Saturday and Sunday \u{b7} {} on weekdays", fmt_secs(st.weekend_secs), fmt_secs(in_sessions - st.weekend_secs));
            rows.push(row("Weekends", Some(&desc), value_text(format!("{share:.0}%"))).into_any_element());
        }

        div()
            .child(caption("When you play"))
            .child(when)
            .when(!rows.is_empty(), |d| d.child(caption("Highlights")).child(card(rows)))
    }
}

fn value_text(text: String) -> Div {
    div().text_size(px(14.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).whitespace_nowrap().child(text)
}

fn notice(ic: Icon, title: String, body: &str) -> Div {
    card_box().items_center().gap(px(8.)).py(px(48.)).child(icon(ic, px(28.), theme::text_dim())).child(
        div().text_size(px(15.)).text_color(theme::text_primary()).child(title),
    )
    .child(div().text_size(px(13.)).text_color(theme::text_dim()).child(body.to_string()))
}

impl Render for StatsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            return self.render_console(window, cx);
        }
        self.covers.drop_evicted(window);
        let cw = content_width(window);
        let wide = cw >= 1100.;
        let controls = self.render_controls(cx);
        let lead = self.history.as_ref().and_then(|h| h.since).map(|d| {
            SharedString::from(format!("Your play time and habits, from the sessions recorded since {} {}.", stats::month_name(d.month()), d.year()))
        });
        let mut column = div().flex().flex_col();
        match (self.history.clone(), self.stats.clone()) {
            (None, _) | (_, None) => {
                column = column.child(notice(Icon::BarChart, "Reading your play history\u{2026}".into(), ""));
            }
            (Some(h), Some(_)) if h.is_empty() => {
                column = column.child(notice(
                    Icon::BarChart,
                    "No play time yet".into(),
                    "Play a game from the Collection and your statistics show up here.",
                ));
            }
            (Some(_), Some(st)) if st.total_secs <= 0. => {
                column = column.child(notice(
                    Icon::AccessTime,
                    format!("Nothing played in {}", self.period.label()),
                    "Pick another period with the arrows above.",
                ));
            }
            (Some(_), Some(st)) => {
                let kpi_cols = if cw >= 900. { 4 } else { 2 };
                column = column
                    .child(self.render_kpis(&st, kpi_cols))
                    .child(caption("Activity"))
                    .child(self.render_activity(&st, cx));
                let games = self.render_games(&st, window, cx);
                let habits = self.render_habits(&st, cx);
                column = column.child(columns(wide, [games, habits]));
            }
        }
        sheet("stats-body", "Statistics", lead, Some(controls), 1560., window, page_close(cx), column).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{axis_label, capitalised, nice_axis};

    #[test]
    fn axis_steps_are_round() {
        assert_eq!(nice_axis(0.), (0.25, 0.25));
        assert_eq!(nice_axis(0.9), (0.25, 1.));
        assert_eq!(nice_axis(3.2), (1., 4.));
        assert_eq!(nice_axis(7.5), (2., 8.));
        assert_eq!(nice_axis(130.), (50., 150.));
        assert_eq!(axis_label(0.5), "30m");
        assert_eq!(axis_label(4.), "4h");
        assert_eq!(axis_label(1.5), "1.5h");
    }

    #[test]
    fn capitalise_first_letter() {
        assert_eq!(capitalised("previous week"), "Previous week");
        assert_eq!(capitalised(""), "");
    }
}
