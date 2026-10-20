//! The Journeys tab: each game of the period as a lane in time (a dot per
//! day played, sized by the hours, from its first to its last day), with
//! when it started, was beaten or abandoned, and how many games were going
//! at once. A lane opens in place with the game's weeks on the same axis.

use chrono::{Datelike, Duration, NaiveDate};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::stats::{fmt_secs, game_sheet, median, month_short, Journeys, Lane, Period, Span, Stats};
use heronite_i18n::{tr, trn};

use super::desktop::{eyebrow, hint, panel, panel_head, GAP, R_PANEL};
use super::{series_color, short_date, today, StatsPage};
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// The title column of the lanes.
const LEFT_W: f32 = 280.;
/// The number columns on the right: total, sessions, to beat, pace.
const COLS: [f32; 4] = [96., 72., 112., 92.];
const LANE_H: f32 = 30.;
const PARALLEL_H: f32 = 96.;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) enum Sort {
    #[default]
    Start,
    Time,
    Recent,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) enum Filter {
    #[default]
    All,
    Beaten,
    Ongoing,
    Abandoned,
    New,
}

/// What the tab shows of the lanes (kept across periods).
#[derive(Default)]
pub(super) struct JourneyView {
    pub sort: Sort,
    pub filter: Filter,
    /// The lane open in place.
    pub open: Option<usize>,
    /// The "only tried" games are listed.
    pub tried: bool,
}

/// Where `d` falls on the axis of `p` (0..=1).
fn frac(p: &Period, d: NaiveDate) -> f32 {
    let total = (p.end - p.start).num_days().max(1) as f32;
    ((d - p.start).num_days() as f32 / total).clamp(0., 1.)
}

/// Ticks of the axis: `(day, label)`.
fn ticks(p: &Period) -> Vec<(NaiveDate, String)> {
    match p.span {
        Span::Week | Span::Month => (0..5)
            .filter_map(|w| {
                let d = p.start + Duration::days(w * 7);
                (d < p.end).then(|| (d, d.day().to_string()))
            })
            .collect(),
        _ => {
            let months = (p.end.year() - p.start.year()) * 12 + p.end.month() as i32 - p.start.month() as i32;
            let every = (months / 13 + 1).max(1) as u32;
            let mut out = Vec::new();
            let mut d = NaiveDate::from_ymd_opt(p.start.year(), p.start.month(), 1).unwrap_or(p.start);
            let mut i = 0;
            while d < p.end {
                if d >= p.start && i % every == 0 {
                    let label = if d.month() == 1 || out.is_empty() {
                        format!("{} {}", month_short(d.month()), d.year() % 100)
                    } else {
                        month_short(d.month()).to_string()
                    };
                    out.push((d, label));
                }
                i += 1;
                d = if d.month() == 12 {
                    NaiveDate::from_ymd_opt(d.year() + 1, 1, 1)
                } else {
                    NaiveDate::from_ymd_opt(d.year(), d.month() + 1, 1)
                }
                .unwrap_or(p.end);
            }
            out
        }
    }
}

impl StatsPage {
    /// Status slug of `game` (`beaten`, `playing`...).
    pub(super) fn game_slug(&self, game: usize) -> String {
        let Some(lib) = self.lib.as_ref() else { return String::new() };
        let Some(g) = lib.games.get(game) else { return String::new() };
        lib.statuses.iter().find(|s| s.id == g.status_id).map_or_else(|| g.status_id.clone(), |s| s.slug.clone())
    }

    /// The day `game` was beaten (or completed): its last recorded session
    /// until the status change has a date of its own.
    pub(super) fn finished_on(&self, game: usize) -> Option<NaiveDate> {
        matches!(self.game_slug(game).as_str(), "beaten" | "completed").then(|| self.history.as_ref()?.last_day(game)).flatten()
    }

    fn abandoned_on(&self, game: usize) -> Option<NaiveDate> {
        (self.game_slug(game) == "abandoned").then(|| self.history.as_ref()?.last_day(game)).flatten()
    }

    fn lane_matches(&self, lane: &Lane, p: &Period) -> bool {
        let slug = self.game_slug(lane.game);
        match self.jv.filter {
            Filter::All => true,
            Filter::Beaten => matches!(slug.as_str(), "beaten" | "completed"),
            Filter::Ongoing => matches!(slug.as_str(), "playing" | "endless" | "on-hold"),
            Filter::Abandoned => slug == "abandoned",
            Filter::New => !lane.from_before(p),
        }
    }

    /// Eyebrow, the number of journeys and the story line (hero of the tab).
    pub(super) fn render_journeys_total(&self, st: &Stats) -> Option<Div> {
        let j = self.journeys.clone()?;
        let p = j.period?;
        let started = j.lanes.iter().chain(&j.tried).filter(|l| !l.from_before(&p)).count();
        let beaten = j.lanes.iter().filter(|l| self.finished_on(l.game).is_some_and(|d| p.contains(d))).count();
        let abandoned = j.lanes.iter().chain(&j.tried).filter(|l| self.abandoned_on(l.game).is_some_and(|d| p.contains(d))).count();
        let period = if st.period.span == Span::All { tr!("all time").to_string() } else { st.period.label() };
        let number = div()
            .flex()
            .items_baseline()
            .gap(px(14.))
            .mt(px(8.))
            .mb(px(12.))
            .font_family(theme::font_heading())
            .font_weight(theme::W_BOLD)
            .child(div().text_size(px(96.)).line_height(px(96.)).child(j.lanes.len().to_string()))
            .child(div().text_size(px(44.)).text_color(theme::white_a(0.85)).child(if j.lanes.len() == 1 {
                tr!("journey")
            } else {
                tr!("journeys")
            }));
        let sep = || div().size(px(4.)).rounded_full().bg(theme::white_a(0.35));
        let fact = |n: usize, rest: String| {
            div()
                .flex()
                .items_baseline()
                .gap(px(5.))
                .whitespace_nowrap()
                .child(div().font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(n.to_string()))
                .child(rest)
        };
        let mut story = div().flex().flex_wrap().items_center().gap(px(14.)).text_size(px(16.)).text_color(theme::white_a(0.78));
        story = story.child(fact(j.lanes.len() + j.tried.len(), tr!("games opened").into()));
        story = story.child(sep()).child(fact(started, tr!("started in {period}", period = period.clone())));
        story = story.child(sep()).child(fact(beaten, tr!("beaten").into()));
        if abandoned > 0 {
            story = story.child(sep()).child(fact(abandoned, tr!("abandoned").into()));
        }
        if !j.tried.is_empty() {
            story = story.child(sep()).child(fact(j.tried.len(), tr!("only tried").into()));
        }
        Some(
            div()
                .mt_auto()
                .pb(px(30.))
                .child(eyebrow(tr!("Journeys \u{b7} {period} \u{b7} games with 1h or more", period), 0.7))
                .child(number)
                .child(story),
        )
    }

    pub(super) fn render_journeys(&mut self, st: &Stats, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(j) = self.journeys.clone() else { return Vec::new() };
        let Some(p) = j.period else { return Vec::new() };
        vec![self.journey_insights(&j, &p), self.journey_parallel(&j, &p), self.journey_lanes(st, &j, &p, window, cx)]
    }

    /// Four sentences about the period's journeys.
    fn journey_insights(&self, j: &Journeys, p: &Period) -> AnyElement {
        let finished: Vec<i64> = j
            .lanes
            .iter()
            .filter_map(|l| Some((self.finished_on(l.game).filter(|d| p.contains(*d))? - l.first_ever?).num_days()))
            .collect();
        let card = |ic: Icon, text: String| {
            div()
                .flex()
                .items_center()
                .gap(px(14.))
                .flex_1()
                .min_w_0()
                .px(px(18.))
                .py(px(16.))
                .rounded(px(16.))
                .bg(rgba(0x161b24d6))
                .border_1()
                .border_color(theme::white_a(0.08))
                .child(
                    div()
                        .flex_none()
                        .size(px(36.))
                        .rounded(px(10.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(theme::white_a(0.06))
                        .child(icon(ic, px(18.), theme::accent())),
                )
                .child(div().text_size(px(13.5)).text_color(theme::white_a(0.82)).child(super::sections::sentence(text)))
        };
        let mut cards = Vec::new();
        if let Some(m) = median(finished.clone()) {
            cards.push(card(Icon::Flag, tr!("You beat a game in **~{days} days** (median of {n})", days = m, n = finished.len())));
        }
        if let Some((d, n)) = j.peak().filter(|x| x.1 > 1) {
            let when = if Journeys::step(p.span) == 1 { super::day_label(d) } else { tr!("the week of {day}", day = short_date(d)) };
            cards.push(card(Icon::GridView, tr!("Up to **{n} games** at the same time, {when}", n, when)));
        }
        if let Some(l) = j
            .lanes
            .iter()
            .filter(|l| l.days.len() > 1)
            .max_by_key(|l| (l.last().unwrap_or(p.start) - l.first().unwrap_or(p.start)).num_days())
        {
            let days = (l.last().unwrap_or(p.start) - l.first().unwrap_or(p.start)).num_days();
            cards.push(card(Icon::History, tr!("Longest journey: **{game}**, {days} days", game = self.game_title(l.game), days)));
        }
        if let Some(l) = j.lanes.iter().filter(|l| l.weeks >= 2).max_by(|a, b| a.hours_per_week().total_cmp(&b.hours_per_week())) {
            cards.push(card(
                Icon::AccessTime,
                tr!(
                    "Most intense: **{game}**, {h}h a week",
                    game = self.game_title(l.game),
                    h = heronite_i18n::decimal(l.hours_per_week(), 1)
                ),
            ));
        }
        div().flex().gap(px(GAP)).children(cards).into_any_element()
    }

    /// A row split like the lanes: the title column, the time axis, the number columns.
    fn lane_row(left: impl IntoElement, axis: impl IntoElement, right: impl IntoElement) -> Div {
        div()
            .flex()
            .items_center()
            .child(div().flex_none().w(px(LEFT_W)).min_w_0().child(left))
            .child(div().relative().flex_1().min_w_0().mx(px(14.)).child(axis))
            .child(div().flex_none().w(px(COLS.iter().sum::<f32>())).child(right))
    }

    fn axis_ticks(p: &Period, today_line: bool) -> Div {
        let mut d = div().relative().h(px(18.));
        for (day, label) in ticks(p) {
            d = d.child(
                div().absolute().top_0().left(relative(frac(p, day))).text_size(px(11.5)).text_color(theme::text_dim()).child(label),
            );
        }
        let t = today();
        if today_line && p.contains(t) {
            d = d.child(
                div()
                    .absolute()
                    .top_0()
                    .left(relative(frac(p, t)))
                    .ml(px(-14.))
                    .text_size(px(11.5))
                    .font_weight(theme::W_BOLD)
                    .text_color(theme::accent())
                    .child(tr!("today")),
            );
        }
        d
    }

    /// How many games were going in each week (each day of a month).
    fn journey_parallel(&self, j: &Journeys, p: &Period) -> AnyElement {
        let max = j.parallel.iter().map(|x| x.1).max().unwrap_or(1).max(1);
        let step = Journeys::step(p.span);
        let w = step as f32 / (p.end - p.start).num_days().max(1) as f32;
        let peak = j.peak();
        let cols = j.parallel.iter().map(|(d, n)| {
            let h = *n as f32 / max as f32 * PARALLEL_H;
            let top = peak.is_some_and(|x| x.0 == *d);
            div()
                .absolute()
                .bottom_0()
                .left(relative(frac(p, *d)))
                .w(relative(w))
                .h(px(h))
                .px(px(1.))
                .child(div().size_full().rounded_t(px(3.)).bg(if top { theme::chart_ramp(1.) } else { theme::chart_ramp(0.62) }))
        });
        let active: Vec<usize> = j.parallel.iter().map(|x| x.1).filter(|n| *n > 0).collect();
        let avg = active.iter().sum::<usize>() as f64 / active.len().max(1) as f64;
        let idle = j.parallel.iter().filter(|x| x.1 == 0).count();
        let unit = if step == 1 { tr!("a day with play, on average") } else { tr!("a week with play, on average") };
        let left = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div().font_family(theme::font_heading()).font_weight(theme::W_BOLD).text_size(px(17.)).child(tr!("Games at the same time")),
            )
            .child(div().text_size(px(12.5)).text_color(theme::text_muted()).child(if step == 1 {
                tr!("Each day, games with 15 min or more")
            } else {
                tr!("Each week, games with 15 min or more")
            }))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(8.))
                    .mt(px(6.))
                    .child(
                        div()
                            .font_family(theme::font_heading())
                            .font_weight(theme::W_BOLD)
                            .text_size(px(28.))
                            .child(heronite_i18n::decimal(avg, 1)),
                    )
                    .child(div().text_size(px(12.5)).text_color(theme::text_muted()).child(unit)),
            );
        let right = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .pl(px(10.))
            .text_size(px(12.5))
            .text_color(theme::text_muted())
            .children(peak.map(|(_, n)| super::sections::sentence(tr!("Peak: **{n} games**", n))))
            .child(super::sections::sentence(if step == 1 {
                tr!("Days without play: **{n}**", n = idle)
            } else {
                tr!("Weeks without play: **{n}**", n = idle)
            }));
        let chart = div()
            .flex()
            .flex_col()
            .child(div().relative().h(px(PARALLEL_H)).border_b_1().border_color(theme::white_a(0.12)).children(cols))
            .child(Self::axis_ticks(p, false).mt(px(6.)));
        panel().child(Self::lane_row(left, chart, right)).into_any_element()
    }

    fn journey_lanes(&mut self, st: &Stats, j: &Journeys, p: &Period, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let mut lanes: Vec<&Lane> = j.lanes.iter().filter(|l| self.lane_matches(l, p)).collect();
        match self.jv.sort {
            Sort::Start => {}
            Sort::Time => lanes.sort_by(|a, b| b.secs.total_cmp(&a.secs)),
            Sort::Recent => lanes.sort_by_key(|l| std::cmp::Reverse(l.last())),
        }
        let day_max = j.lanes.iter().flat_map(|l| l.days.iter().map(|d| d.1)).fold(1., f64::max);
        let count = |f: Filter| {
            j.lanes
                .iter()
                .filter(|l| {
                    let slug = self.game_slug(l.game);
                    match f {
                        Filter::All => true,
                        Filter::Beaten => matches!(slug.as_str(), "beaten" | "completed"),
                        Filter::Ongoing => matches!(slug.as_str(), "playing" | "endless" | "on-hold"),
                        Filter::Abandoned => slug == "abandoned",
                        Filter::New => !l.from_before(p),
                    }
                })
                .count()
        };
        let sorts = [(Sort::Start, tr!("By start")), (Sort::Time, tr!("By time")), (Sort::Recent, tr!("Most recent"))];
        let filters = [
            (Filter::All, tr!("All")),
            (Filter::Beaten, tr!("Beaten")),
            (Filter::Ongoing, tr!("Ongoing")),
            (Filter::Abandoned, tr!("Abandoned")),
            (Filter::New, tr!("Started here")),
        ];
        let pill = |id: String, label: String, on: bool| {
            div()
                .id(SharedString::from(id))
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(30.))
                .px(px(12.))
                .rounded_full()
                .text_size(px(12.5))
                .font_weight(theme::W_BOLD)
                .when(on, |d| d.bg(theme::accent()).text_color(theme::on_accent()))
                .when(!on, |d| {
                    d.bg(theme::white_a(0.06))
                        .text_color(theme::text_muted())
                        .cursor_pointer()
                        .hover(|s| s.text_color(theme::text_primary()))
                })
                .child(label)
        };
        let mut controls = div().flex().flex_wrap().items_center().gap(px(6.)).mb(px(14.));
        for (s, label) in sorts {
            controls = controls.child(pill(format!("jsort-{s:?}"), label.into(), self.jv.sort == s).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.jv.sort = s;
                    cx.notify();
                },
            )));
        }
        controls = controls.child(div().w(px(14.)));
        for (f, label) in filters {
            let n = count(f);
            if n == 0 && f != Filter::All {
                continue;
            }
            controls = controls.child(pill(format!("jfilter-{f:?}"), format!("{label} {n}"), self.jv.filter == f).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.jv.filter = f;
                    cx.notify();
                },
            )));
        }
        let legend = div()
            .ml_auto()
            .flex()
            .items_center()
            .gap(px(14.))
            .text_size(px(12.))
            .text_color(theme::text_muted())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().size(px(9.)).rounded_full().bg(theme::chart_ramp(0.7)))
                    .child(tr!("day played (size = hours)")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().size(px(10.)).rounded_full().border_2().border_color(theme::text_primary()))
                    .child(tr!("started")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(icon(Icon::Flag, px(13.), theme::status_color("beaten")))
                    .child(tr!("beaten")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(icon(Icon::Close, px(13.), theme::status_color("abandoned")))
                    .child(tr!("abandoned")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(icon(Icon::ChevronLeft, px(13.), theme::text_muted()))
                    .child(tr!("from before")),
            );
        let heads =
            div().flex().children(COLS.iter().zip([tr!("Total"), tr!("Sessions"), tr!("To beat"), tr!("Pace")]).map(|(w, t)| {
                div().w(px(*w)).flex().justify_end().text_size(px(11.5)).text_color(theme::text_dim()).child(t.to_uppercase())
            }));
        let header = Self::lane_row(
            div().text_size(px(11.5)).text_color(theme::text_dim()).child(trn!(lanes.len(), "{n} game", "{n} games").to_uppercase()),
            Self::axis_ticks(p, true),
            heads,
        )
        .pb(px(8.))
        .border_b_1()
        .border_color(theme::white_a(0.08));

        let mut rows = div().flex().flex_col();
        for lane in lanes {
            rows = rows.child(self.lane(st, lane, p, day_max, window, cx));
            if self.jv.open == Some(lane.game) {
                rows = rows.child(self.lane_open(lane, p, window, cx));
            }
        }
        if !j.tried.is_empty() {
            let open = self.jv.tried;
            let total: f64 = j.tried.iter().map(|l| l.secs).sum();
            rows = rows.child(
                div()
                    .id("jtried")
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .py(px(12.))
                    .cursor_pointer()
                    .text_size(px(13.))
                    .text_color(theme::text_muted())
                    .hover(|s| s.text_color(theme::text_primary()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.jv.tried = !this.jv.tried;
                        cx.notify();
                    }))
                    .child(icon(if open { Icon::FaChevronUp } else { Icon::FaChevronDown }, px(12.), theme::text_muted()))
                    .child(tr!("{n} games only tried, under 1h each \u{b7} {time} in all", n = j.tried.len(), time = fmt_secs(total))),
            );
            if open {
                rows = rows.child(div().flex().flex_wrap().gap(px(8.)).pb(px(8.)).children(j.tried.iter().map(|l| {
                    let g = l.game;
                    div()
                        .id(("jtried-game", g))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(10.))
                        .py(px(5.))
                        .rounded_full()
                        .bg(theme::white_a(0.05))
                        .text_size(px(12.5))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::white_a(0.09)))
                        .on_click(cx.listener(move |this, _, window, cx| this.open_sheet(g, window, cx)))
                        .child(self.game_title(g))
                        .child(div().text_color(theme::text_dim()).child(fmt_secs(l.secs)))
                })));
            }
        }
        panel()
            .child(panel_head(tr!("Each game in time"), hint(tr!("Click a game to open its weeks"))))
            .child(div().flex().items_center().child(controls).child(legend))
            .child(header)
            .child(rows)
            .into_any_element()
    }

    fn lane(&mut self, st: &Stats, lane: &Lane, p: &Period, day_max: f64, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let g = lane.game;
        let color: Hsla = match st.series_of(g) {
            Some(slot) => series_color(Some(slot)),
            None => theme::chart_ramp(0.7).into(),
        };
        let cover = self.cover(g, 30., 40., window);
        let status = self.game_status(g);
        let new = !lane.from_before(p);
        let left = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .flex_none()
                    .w(px(30.))
                    .h(px(40.))
                    .rounded(px(6.))
                    .overflow_hidden()
                    .bg(theme::white_a(0.06))
                    .children(cover.map(|s| img(s).size_full().object_fit(ObjectFit::Cover))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(div().text_size(px(13.5)).font_weight(theme::W_BOLD).truncate().child(self.game_title(g)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(11.5))
                            .text_color(theme::text_muted())
                            .children(status.map(|(name, c)| {
                                div().flex().items_center().gap(px(5.)).child(div().size(px(7.)).rounded_full().bg(c)).child(name)
                            }))
                            .when(new, |d| d.child(super::super::status_pill(tr!("New"), theme::accent()))),
                    ),
            );
        let (first, last) = (lane.first().unwrap_or(p.start), lane.last().unwrap_or(p.start));
        let (x0, x1) = (frac(p, first), frac(p, last));
        let mut axis = div().relative().h(px(LANE_H));
        if lane.from_before(p) {
            axis = axis
                .child(div().absolute().top(px(LANE_H / 2. - 1.)).left_0().w(relative(x0)).h(px(2.)).bg(Hsla { a: 0.25, ..color }))
                .child(div().absolute().top(px(LANE_H / 2. - 7.)).left(px(-16.)).child(icon(
                    Icon::ChevronLeft,
                    px(14.),
                    theme::text_muted(),
                )));
        }
        axis = axis.child(
            div()
                .absolute()
                .top(px(LANE_H / 2. - 4.))
                .left(relative(x0))
                .w(relative((x1 - x0).max(0.)))
                .h(px(8.))
                .ml(px(-4.))
                .rounded_full()
                .bg(Hsla { a: 0.18, ..color }),
        );
        for (d, secs) in &lane.days {
            let size = 5. + 9. * ((*secs / day_max) as f32).sqrt();
            axis = axis.child(
                div()
                    .absolute()
                    .top(px(LANE_H / 2. - size / 2.))
                    .left(relative(frac(p, *d)))
                    .ml(px(-size / 2.))
                    .size(px(size))
                    .rounded_full()
                    .bg(color),
            );
        }
        if new {
            axis = axis.child(
                div()
                    .absolute()
                    .top(px(LANE_H / 2. - 7.))
                    .left(relative(x0))
                    .ml(px(-7.))
                    .size(px(14.))
                    .rounded_full()
                    .border_2()
                    .border_color(theme::text_primary())
                    .bg(theme::stats_page_bg()),
            );
        }
        if let Some(d) = self.finished_on(g).filter(|d| p.contains(*d)) {
            axis = axis.child(div().absolute().top(px(-6.)).left(relative(frac(p, d))).child(icon(
                Icon::Flag,
                px(16.),
                theme::status_color("beaten"),
            )));
        } else if let Some(d) = self.abandoned_on(g).filter(|d| p.contains(*d)) {
            axis = axis.child(div().absolute().top(px(LANE_H / 2. - 8.)).left(relative(frac(p, d))).ml(px(6.)).child(icon(
                Icon::Close,
                px(15.),
                theme::status_color("abandoned"),
            )));
        }

        let slug = self.game_slug(g);
        let to_beat = match (self.finished_on(g).filter(|d| p.contains(*d)), lane.first_ever) {
            (Some(f), Some(start)) => (
                trn!((f - start).num_days().max(0) as usize, "{n} day", "{n} days"),
                Some(trn!(lane.days.len(), "{n} day played", "{n} days played")),
            ),
            _ if matches!(slug.as_str(), "playing" | "endless" | "on-hold") => (tr!("ongoing").to_string(), None),
            _ => ("\u{2014}".to_string(), None),
        };
        let col = |w: f32, big: String, small: Option<String>, dim: bool| {
            div()
                .w(px(w))
                .flex()
                .flex_col()
                .items_end()
                .child(
                    div()
                        .text_size(px(13.5))
                        .font_weight(theme::W_BOLD)
                        .text_color(if dim { theme::text_dim() } else { theme::text_primary() })
                        .child(big),
                )
                .children(small.map(|s| div().text_size(px(11.)).text_color(theme::text_dim()).child(s)))
        };
        let right = div()
            .flex()
            .child(col(COLS[0], fmt_secs(lane.secs), None, false))
            .child(col(COLS[1], lane.sessions.to_string(), None, false))
            .child(col(COLS[2], to_beat.0.clone(), to_beat.1, to_beat.0 == "\u{2014}" || to_beat.0 == tr!("ongoing")))
            .child(col(COLS[3], tr!("{h}h", h = heronite_i18n::decimal(lane.hours_per_week(), 1)), Some(tr!("a week").into()), false));
        let open = self.jv.open == Some(g);
        Self::lane_row(left, axis, right)
            .id(("lane", g))
            .py(px(7.))
            .px(px(8.))
            .mx(px(-8.))
            .rounded(px(10.))
            .border_b_1()
            .border_color(theme::white_a(0.04))
            .cursor_pointer()
            .when(open, |d| d.bg(theme::white_a(0.05)))
            .hover(|s| s.bg(theme::white_a(0.035)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.jv.open = if this.jv.open == Some(g) { None } else { Some(g) };
                cx.notify();
            }))
            .into_any_element()
    }

    /// The open lane: the game's days or weeks on the same axis and its facts.
    fn lane_open(&mut self, lane: &Lane, p: &Period, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(h) = self.history.clone() else { return div().into_any_element() };
        let g = lane.game;
        let sh = game_sheet(&h, g, *p, today());
        let max = sh.bars.iter().map(|b| b.1).fold(1., f64::max);
        let step = if matches!(p.span, Span::Week | Span::Month) { 1 } else { 7 };
        let w = step as f32 / (p.end - p.start).num_days().max(1) as f32;
        let color: Hsla = theme::chart_ramp(0.75).into();
        let bars = sh.bars.iter().filter(|b| b.1 > 0.).map(|(d, secs)| {
            div()
                .absolute()
                .bottom_0()
                .left(relative(frac(p, (*d).max(p.start))))
                .w(relative(w))
                .h(px((*secs / max) as f32 * 90.))
                .px(px(1.))
                .child(div().size_full().rounded_t(px(3.)).bg(color))
        });
        let fact = |label: String, value: String| {
            div()
                .flex()
                .justify_between()
                .gap(px(10.))
                .text_size(px(12.5))
                .child(div().text_color(theme::text_muted()).child(label))
                .child(div().font_weight(theme::W_BOLD).child(value))
        };
        let mut facts = div().flex().flex_col().gap(px(7.)).pr(px(10.));
        if let Some(b) = sh.best {
            facts = facts.child(fact(tr!("Longest session").into(), format!("{} \u{b7} {}", fmt_secs(b.secs), short_date(b.start.date()))));
        }
        if sh.sessions > 0 {
            facts = facts.child(fact(tr!("Average session").into(), fmt_secs(sh.secs / sh.sessions as f64)));
        }
        if let Some(hh) = sh.usual_hour {
            facts = facts.child(fact(tr!("Usually starts").into(), tr!("around {h}:00", h = hh)));
        }
        facts = facts.child(fact(tr!("On weekends").into(), format!("{:.0}%", sh.weekend * 100.)));
        if let Some(f) = lane.first_ever {
            facts = facts.child(fact(tr!("First played").into(), super::full_date(f)));
        }
        let peak = sh.bars.iter().max_by(|a, b| a.1.total_cmp(&b.1)).filter(|b| b.1 > 0.).map(|b| fmt_secs(b.1)).unwrap_or_default();
        let chart = div()
            .flex()
            .flex_col()
            .child(div().text_size(px(11.5)).text_color(theme::text_dim()).mb(px(6.)).child(if step == 1 {
                tr!("Hours per day \u{b7} best {time}", time = peak)
            } else {
                tr!("Hours per week \u{b7} best {time}", time = peak)
            }))
            .child(div().relative().h(px(90.)).border_b_1().border_color(theme::white_a(0.1)).children(bars));
        let _ = window;
        let button = crate::ui::controls::button_sm(("jsheet", g), crate::ui::controls::ButtonKind::Glass, tr!("Open game sheet"))
            .on_click(cx.listener(move |this, _, window, cx| this.open_sheet(g, window, cx)));
        Self::lane_row(facts, chart, div().flex().justify_end().child(button))
            .py(px(16.))
            .px(px(8.))
            .mx(px(-8.))
            .mb(px(6.))
            .rounded(px(R_PANEL - 6.))
            .bg(theme::white_a(0.03))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{frac, ticks};
    use chrono::NaiveDate;
    use heronite_data::stats::{month_short, Period, Span};

    #[test]
    fn axis_ticks_and_positions() {
        let p = Period::of(Span::Year, NaiveDate::from_ymd_opt(2026, 5, 1).unwrap());
        assert_eq!(frac(&p, p.start), 0.);
        assert!((frac(&p, NaiveDate::from_ymd_opt(2026, 7, 2).unwrap()) - 0.5).abs() < 0.01);
        let t = ticks(&p);
        assert_eq!(t.len(), 12);
        assert_eq!(t[0].1, format!("{} 26", month_short(1)));
        let m = Period::of(Span::Month, NaiveDate::from_ymd_opt(2026, 2, 1).unwrap());
        assert_eq!(ticks(&m).iter().map(|t| t.1.clone()).collect::<Vec<_>>(), ["1", "8", "15", "22"]);
    }
}
