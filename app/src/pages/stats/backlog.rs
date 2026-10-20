//! The Backlog tab: what gets finished. The year's beaten games against a
//! goal, beaten and abandoned games on the year's line, the hours they took
//! next to How Long To Beat, the whole Collection by status, the queue
//! (Plan to play) with when each would be beaten at your pace, the games on
//! hold, and the pile: games added, started and beaten over time.

use std::collections::HashMap;

use chrono::{Datelike, Duration, NaiveDate};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::hltb::HltbEntry;
use heronite_data::stats::{fmt_hours_short, fmt_secs, goal, median, month_ends, month_short, Period, Span, Stats};
use heronite_data::status_history::Change;
use heronite_i18n::{tr, trc, trn};

use super::desktop::{eyebrow, hint, panel, panel_head, GAP};
use super::facts::BEATEN;
use super::sections::sentence;
use super::{full_date, short_date, today, StatsPage};
use crate::pages::PageEvent;
use crate::theme;

/// Days without a session after which a game in play is "on the shelf".
const STALE_DAYS: i64 = 30;
const COVER_W: f32 = 46.;
const COVER_H: f32 = 62.;
/// Covers of beaten (and of abandoned) games on the line, the latest.
const YEAR_COVERS_MAX: usize = 60;

/// Library facts the tab needs besides the sessions, read once per library
/// off the UI thread.
#[derive(Default)]
pub(super) struct BacklogData {
    /// Status changes by game index ([`heronite_data::status_history`]).
    pub changes: HashMap<usize, Vec<Change>>,
    /// Cached How Long To Beat times by game index.
    pub hltb: HashMap<usize, HltbEntry>,
    /// Day each game entered the library.
    pub added: HashMap<usize, NaiveDate>,
}

impl BacklogData {
    pub fn read(lib: &heronite_data::Library) -> Self {
        let changes = heronite_data::status_history::load(&lib.paths);
        let wiki = heronite_data::hltb::WikiCache::load(&lib.paths);
        let added = heronite_data::added::load(&lib.paths);
        let mut out = Self::default();
        for (i, g) in lib.games.iter().enumerate() {
            if let Some(c) = changes.get(&g.app_name) {
                out.changes.insert(i, c.clone());
            }
            if let Some(h) = wiki.hltb(&g.title, &g.game.title) {
                out.hltb.insert(i, h);
            }
            if let Some(d) = added.get(&g.key).and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()) {
                out.added.insert(i, d);
            }
        }
        out
    }
}

/// How the queue is ordered.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) enum QueueSort {
    #[default]
    Shortest,
    Oldest,
    Started,
}

impl StatsPage {
    /// Games beaten in `year`, oldest first.
    pub(super) fn beaten_in_year(&self, year: i32) -> Vec<(usize, NaiveDate)> {
        let jan1 = NaiveDate::from_ymd_opt(year, 1, 1).unwrap_or_else(today);
        self.facts.beaten_in(&Period::of(Span::Year, jan1))
    }

    fn year_goal(&self, year: i32) -> u32 {
        self.prefs.goals.get(&year).copied().unwrap_or(12)
    }

    fn set_goal(&mut self, year: i32, n: u32, cx: &mut Context<Self>) {
        self.prefs.goals.insert(year, n.clamp(1, 999));
        self.save_prefs(cx);
        cx.notify();
    }

    /// Hero of the tab: games beaten in the period, and the goal of the year.
    pub(super) fn render_backlog_total(&self, st: &Stats, cx: &mut Context<Self>) -> Div {
        let p = st.period;
        let beaten = self.facts.beaten_in(&p);
        let abandoned = self.facts.abandoned_in(&p);
        let all = p.span == Span::All;
        let period = if all { tr!("all time").to_string() } else { p.label() };
        let total = self.lib.as_ref().map_or(0, |l| l.games.len());
        let number = div()
            .flex()
            .items_baseline()
            .gap(px(14.))
            .mt(px(8.))
            .mb(px(12.))
            .font_family(theme::font_heading())
            .font_weight(theme::W_BOLD)
            .child(div().text_size(px(96.)).line_height(px(96.)).child(beaten.len().to_string()))
            .child(div().text_size(px(44.)).text_color(theme::white_a(0.85)).child(if beaten.len() == 1 {
                tr!("game beaten")
            } else {
                tr!("games beaten")
            }));
        let sep = || div().size(px(4.)).rounded_full().bg(theme::white_a(0.35));
        let fact = |n: String, rest: String| {
            div()
                .flex()
                .items_baseline()
                .gap(px(5.))
                .whitespace_nowrap()
                .child(div().font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(n))
                .child(rest)
        };
        let story = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(14.))
            .text_size(px(16.))
            .text_color(theme::white_a(0.78))
            .child(fact(abandoned.len().to_string(), tr!("abandoned").into()))
            .child(sep())
            .child(fact(total.to_string(), tr!("games in the Collection").into()));
        let main = div().flex_1().child(eyebrow(tr!("Backlog \u{b7} {period}", period), 0.7)).child(number).child(story);
        let side = (!all).then(|| self.goal_card(p.start.year(), beaten.len() as u32, cx));
        div().mt_auto().pb(px(30.)).flex().items_end().gap(px(24.)).child(main).children(side)
    }

    fn goal_card(&self, year: i32, done: u32, cx: &mut Context<Self>) -> Div {
        let target = self.year_goal(year);
        let g = goal(target, done, year, today());
        let ring_color: Hsla = if done >= target { theme::success().into() } else { theme::accent().into() };
        let progress = (done as f32 / target.max(1) as f32).min(1.);
        let expected = (g.expected / target.max(1) as f64).min(1.) as f32;
        let ring = div()
            .relative()
            .flex_none()
            .size(px(112.))
            .child(
                canvas(|_, _, _| (), move |bounds, _, window, _| paint_ring(bounds, progress, expected, ring_color, window))
                    .absolute()
                    .inset_0(),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .font_family(theme::font_heading())
                            .font_weight(theme::W_BOLD)
                            .child(div().text_size(px(30.)).child(done.to_string()))
                            .child(div().text_size(px(16.)).text_color(theme::text_muted()).child(format!("/{target}"))),
                    )
                    .child(div().text_size(px(11.5)).text_color(theme::text_muted()).child(tr!("beaten"))),
            );
        let step = |id: &'static str, label: &'static str, delta: i32| {
            div()
                .id(id)
                .flex()
                .items_center()
                .justify_center()
                .size(px(26.))
                .rounded_full()
                .bg(theme::white_a(0.08))
                .cursor_pointer()
                .hover(|s| s.bg(theme::white_a(0.16)))
                .font_weight(theme::W_BOLD)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let n = this.year_goal(year) as i32 + delta;
                    this.set_goal(year, n.max(1) as u32, cx);
                }))
                .child(label)
        };
        let ongoing = year == today().year();
        let mut lines = div().flex().flex_col().gap(px(6.)).text_size(px(13.)).text_color(theme::white_a(0.82));
        if done >= target {
            lines = lines.child(sentence(tr!("**Goal reached.** Raise it?")));
        } else if ongoing {
            let ahead = done as f64 >= g.expected;
            lines = lines.child(sentence(if ahead {
                tr!("**On track** \u{b7} expected by today: {n}", n = heronite_i18n::decimal(g.expected, 1))
            } else {
                tr!("**A little behind** \u{b7} expected by today: {n}", n = heronite_i18n::decimal(g.expected, 1))
            }));
            lines = lines.child(sentence(tr!("At this pace, **~{n}** by Dec 31", n = g.projected.round() as u32)));
            if let Some(line) = self.plans_line(year) {
                lines = lines.child(sentence(line));
            }
            if let Some(every) = g.every_days {
                lines = lines.child(sentence(tr!(
                    "To make it: **one every {n} days** ({left} to go)",
                    n = every.round() as u32,
                    left = g.left()
                )));
            }
        } else {
            lines = lines.child(sentence(tr!("{year} ended with **{done} of {target}**", year, done, target)));
        }
        div()
            .flex()
            .items_center()
            .gap(px(20.))
            .p(px(18.))
            .rounded(px(20.))
            .bg(rgba(0x0e1218d9))
            .border_1()
            .border_color(theme::white_a(0.1))
            .child(ring)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .font_family(theme::font_heading())
                                    .font_weight(theme::W_BOLD)
                                    .text_size(px(15.))
                                    .child(tr!("Goal for {year}", year)),
                            )
                            .child(step("goal-minus", "\u{2212}", -1))
                            .child(div().min_w(px(26.)).flex().justify_center().font_weight(theme::W_BOLD).child(target.to_string()))
                            .child(step("goal-plus", "+", 1)),
                    )
                    .child(lines),
            )
    }

    pub(super) fn render_backlog(
        &mut self,
        st: &Stats,
        col_w: f32,
        wide: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let p = st.period;
        let summary = self.backlog_summary(&p);
        let mut out = vec![self.backlog_year(&p, summary, col_w, window, cx)];
        let coll = self.backlog_collection();
        // Without cached How Long To Beat times, its panel is a one-line note.
        match self.backlog_hltb(&p, window, cx) {
            Some(hltb) => out.push(super::sections::row(wide, vec![(hltb, 7), (coll, 5)])),
            None => out.push(coll.into_any_element()),
        }
        let queue = self.backlog_queue(window, cx);
        let shelf = self.backlog_shelf(window, cx);
        out.push(super::sections::row(wide, vec![(queue, 7), (shelf, 5)]));
        out.push(self.backlog_pile());
        out
    }

    /// The numbers behind the year's beaten and abandoned games, as a strip
    /// at the top of [`Self::backlog_year`].
    fn backlog_summary(&self, p: &Period) -> Div {
        let beaten = self.facts.beaten_in(p);
        let abandoned = self.facts.abandoned_in(p);
        let life = |g: usize| self.history.as_ref().map_or(0., |h| h.total_secs(g));
        let beaten_secs: f64 = beaten.iter().map(|(g, _)| life(*g)).sum();
        let abandoned_secs: f64 = abandoned.iter().map(|(g, _)| life(*g)).sum();
        let first = |g: usize| self.history.as_ref().and_then(|h| h.first_day(g));
        let days = median(beaten.iter().filter_map(|(g, d)| Some((*d - first(*g)?).num_days())).collect());
        let ratios: Vec<f64> = beaten
            .iter()
            .filter_map(|(g, _)| {
                let h = self.backlog.hltb.get(g)?;
                let reference = if h.main_extra > 0. { h.main_extra } else { h.main_story };
                (reference > 0.).then(|| life(*g) / 3600. / reference)
            })
            .collect();
        let tile = |value: String, label: String| {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(2.))
                .pl(px(18.))
                .border_l_1()
                .border_color(theme::white_a(0.08))
                .child(div().font_family(theme::font_heading()).font_weight(theme::W_BOLD).text_size(px(22.)).child(value))
                .child(div().text_size(px(12.5)).text_color(theme::text_muted()).child(label))
        };
        let mut tiles = vec![tile(fmt_hours_short(beaten_secs), tr!("in games you beat").into())];
        if let Some(d) = days {
            tiles.push(tile(trn!(d.max(0) as usize, "{n} day", "{n} days"), tr!("from first session to beaten (median)").into()));
        }
        if !ratios.is_empty() {
            let avg = ratios.iter().sum::<f64>() / ratios.len() as f64;
            tiles.push(tile(
                format!("{}\u{d7}", heronite_i18n::decimal(avg, 1)),
                tr!("the time of How Long To Beat (main + extras)").into(),
            ));
        }
        tiles.push(tile(fmt_hours_short(abandoned_secs), tr!("in games you abandoned").into()));
        div().flex().gap(px(GAP)).mb(px(22.)).children(tiles)
    }

    /// Beaten games above a line of months, abandoned ones under it, and
    /// empty slots for what the goal still asks.
    fn backlog_year(&mut self, p: &Period, summary: Div, col_w: f32, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        // All time keeps the latest covers: hundreds would stack row on row.
        let latest = |mut v: Vec<(usize, NaiveDate)>| {
            v.drain(..v.len().saturating_sub(YEAR_COVERS_MAX));
            v
        };
        let (all_beaten, all_abandoned) = (self.facts.beaten_in(p), self.facts.abandoned_in(p));
        let cut = all_beaten.len() > YEAR_COVERS_MAX || all_abandoned.len() > YEAR_COVERS_MAX;
        let (beaten, abandoned) = (latest(all_beaten), latest(all_abandoned));
        let start = if p.span == Span::All { self.history.as_ref().and_then(|h| h.since).unwrap_or(p.start) } else { p.start };
        let end = if p.span == Span::All { today() + Duration::days(1) } else { p.end };
        let w = col_w - 48.;
        let x_of = |d: NaiveDate| ((d - start).num_days() as f32 / (end - start).num_days().max(1) as f32).clamp(0., 1.) * (w - COVER_W);
        // Greedy rows so covers do not overlap.
        let place = |items: &[(usize, NaiveDate)], cw: f32| {
            let mut rows: Vec<f32> = Vec::new();
            items
                .iter()
                .map(|(g, d)| {
                    let x = x_of(*d);
                    let row = rows.iter().position(|r| *r <= x).unwrap_or_else(|| {
                        rows.push(f32::MIN);
                        rows.len() - 1
                    });
                    rows[row] = x + cw + 6.;
                    (*g, *d, x, row)
                })
                .collect::<Vec<_>>()
        };
        let up = place(&beaten, COVER_W);
        let down = place(&abandoned, COVER_W * 0.75);
        let up_rows = up.iter().map(|x| x.3 + 1).max().unwrap_or(1);
        let down_rows = down.iter().map(|x| x.3 + 1).max().unwrap_or(0);
        let axis_y = up_rows as f32 * (COVER_H + 8.) + 14.;
        let height = axis_y + 30. + down_rows as f32 * (COVER_H * 0.75 + 8.);
        let mut area = div().relative().w(px(w)).h(px(height));
        // Months.
        for m in month_ends(start, end - Duration::days(1)) {
            let first = NaiveDate::from_ymd_opt(m.year(), m.month(), 1).unwrap_or(m).max(start);
            let x = x_of(first) + COVER_W / 2.;
            area = area.child(div().absolute().left(px(x)).top(px(axis_y - 4.)).w(px(1.)).h(px(8.)).bg(theme::white_a(0.18))).child(
                div()
                    .absolute()
                    .left(px(x + 4.))
                    .top(px(axis_y + 6.))
                    .text_size(px(11.5))
                    .text_color(theme::text_dim())
                    .child(month_short(m.month())),
            );
        }
        area =
            area.child(div().absolute().left(px(COVER_W / 2.)).right(px(COVER_W / 2.)).top(px(axis_y)).h(px(1.)).bg(theme::white_a(0.16)));
        let t = today();
        if p.contains(t) {
            area =
                area.child(div().absolute().left(px(x_of(t) + COVER_W / 2.)).top_0().bottom_0().w(px(1.)).bg(theme::white_a(0.2))).child(
                    div()
                        .absolute()
                        .left(px(x_of(t) + COVER_W / 2. + 5.))
                        .top_0()
                        .text_size(px(11.5))
                        .text_color(theme::text_muted())
                        .child(tr!("today")),
                );
        }
        let beaten_c = theme::status_color("beaten");
        for (g, d, x, row) in up {
            let src = self.cover(g, COVER_W, COVER_H, window);
            let y = axis_y - 10. - COVER_H - row as f32 * (COVER_H + 8.);
            area = area
                .child(
                    div()
                        .absolute()
                        .left(px(x + COVER_W / 2.))
                        .top(px(y + COVER_H))
                        .w(px(1.))
                        .h(px(axis_y - y - COVER_H))
                        .bg(Hsla { a: 0.5, ..beaten_c.into() }),
                )
                .child(div().absolute().left(px(x + COVER_W / 2. - 4.)).top(px(axis_y - 4.)).size(px(8.)).rounded_full().bg(beaten_c))
                .child(self.cover_tile(("bk-up", g), g, src, COVER_W, COVER_H, Some(short_date(d)), cx).absolute().left(px(x)).top(px(y)));
        }
        let (aw, ah) = (COVER_W * 0.75, COVER_H * 0.75);
        for (g, d, x, row) in down {
            let src = self.cover(g, aw, ah, window);
            let y = axis_y + 26. + row as f32 * (ah + 8.);
            area = area
                .child(
                    div()
                        .absolute()
                        .left(px(x + COVER_W / 2. - 4.))
                        .top(px(axis_y - 4.))
                        .size(px(8.))
                        .rounded_full()
                        .bg(theme::status_color("abandoned")),
                )
                .child(
                    self.cover_tile(("bk-down", g), g, src, aw, ah, Some(short_date(d)), cx)
                        .absolute()
                        .left(px(x + (COVER_W - aw) / 2.))
                        .top(px(y))
                        .opacity(0.7),
                );
        }
        // What the goal still asks, spread over the rest of the year: each
        // slot holds a planned game or opens the picker ([`super::plans`]).
        if p.span == Span::Year && p.contains(t) {
            let year = p.start.year();
            let planned = self.plans_of(year);
            let left = (self.year_goal(year).saturating_sub(beaten.len() as u32) as usize).max(planned.len()).min(8);
            for i in 0..left {
                let d = t + Duration::days(((p.end - t).num_days() as f32 * (i as f32 + 0.6) / left as f32) as i64);
                let slot = self.plan_slot(year, i, planned.get(i).copied(), COVER_W, COVER_H, window, cx);
                area = area.child(slot.absolute().left(px(x_of(d))).top(px(axis_y - 10. - COVER_H)));
            }
        }
        let legend = div()
            .flex()
            .items_center()
            .gap(px(14.))
            .text_size(px(12.))
            .text_color(theme::text_muted())
            .child(div().flex().items_center().gap(px(6.)).child(div().size(px(8.)).rounded_full().bg(beaten_c)).child(tr!("Beaten")))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().size(px(8.)).rounded_full().bg(theme::status_color("abandoned")))
                    .child(tr!("Abandoned")),
            );
        let has_dates = !self.backlog.changes.is_empty();
        let mut note = if has_dates {
            tr!("Beaten above, abandoned below").to_string()
        } else {
            tr!("Beaten above, abandoned below \u{b7} the day is the last session until a status changes").to_string()
        };
        if cut {
            note = format!("{note} \u{b7} {}", tr!("the latest {n}", n = YEAR_COVERS_MAX));
        }
        let title = if p.span == Span::All { tr!("Every beaten game") } else { tr!("The year") };
        panel()
            .child(panel_head(title, legend))
            .child(div().mt(px(-8.)).mb(px(18.)).child(hint(note)))
            .child(summary)
            .child(area)
            .into_any_element()
    }

    /// A clickable cover (opens the game sheet), with an optional caption.
    fn cover_tile(
        &self,
        id: (&'static str, usize),
        g: usize,
        src: Option<ImageSource>,
        w: f32,
        h: f32,
        caption: Option<String>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let title = self.game_title(g);
        div()
            .id(SharedString::from(format!("{}-{}", id.0, id.1)))
            .w(px(w))
            .h(px(h))
            .rounded(px(8.))
            .overflow_hidden()
            .bg(theme::white_a(0.06))
            .border_1()
            .border_color(theme::white_a(0.1))
            .cursor_pointer()
            .hover(|s| s.border_color(Hsla { a: 0.6, ..Hsla::from(theme::accent()) }))
            .on_click(cx.listener(move |this, _, window, cx| this.open_sheet(g, window, cx)))
            .children(src.map(|s| img(s).size_full().object_fit(ObjectFit::Cover)))
            .when(caption.is_none(), |d| d.child(div().p(px(4.)).text_size(px(9.)).text_color(theme::text_muted()).child(title)))
    }

    /// Hours each beaten game took next to its How Long To Beat range.
    fn backlog_hltb(&mut self, p: &Period, window: &Window, cx: &mut Context<Self>) -> Option<Div> {
        let beaten = self.facts.beaten_in(p);
        let rows: Vec<(usize, HltbEntry, f64)> = beaten
            .iter()
            .rev()
            .filter_map(|(g, _)| {
                let h = self.backlog.hltb.get(g)?.clone();
                let mine = self.history.as_ref().map_or(0., |hh| hh.total_secs(*g)) / 3600.;
                Some((*g, h, mine))
            })
            .take(10)
            .collect();
        let head = panel_head(tr!("Your hours vs. How Long To Beat"), hint(tr!("beaten in this period")));
        if rows.is_empty() {
            return None;
        }
        let max = rows.iter().map(|(_, h, m)| h.completionist.max(h.main_extra).max(*m)).fold(1., f64::max) * 1.05;
        let x = |v: f64| relative((v / max) as f32);
        let mut list = div().flex().flex_col().gap(px(10.));
        for (g, h, mine) in rows {
            let cover = self.cover(g, 22., 30., window);
            let reference = if h.main_extra > 0. { h.main_extra } else { h.main_story };
            let diff = (mine / reference - 1.) * 100.;
            let (lo, hi) = (h.main_story.max(0.), h.completionist.max(h.main_extra));
            let mut bar = div().relative().flex_1().h(px(18.));
            bar = bar
                .child(div().absolute().top(px(7.)).left(x(lo)).w(x((hi - lo).max(0.))).h(px(4.)).rounded_full().bg(theme::white_a(0.16)))
                .child(div().absolute().top(px(3.)).left(x(reference)).w(px(2.)).h(px(12.)).bg(theme::white_a(0.6)))
                .child(
                    div()
                        .absolute()
                        .top(px(3.))
                        .left(x(mine))
                        .ml(px(-6.))
                        .size(px(12.))
                        .rounded_full()
                        .bg(theme::accent())
                        .border_2()
                        .border_color(rgb(0x161b24)),
                );
            let game = g;
            list = list.child(
                div()
                    .id(("bk-hltb", g))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| this.open_sheet(game, window, cx)))
                    .child(
                        div()
                            .flex_none()
                            .w(px(22.))
                            .h(px(30.))
                            .rounded(px(4.))
                            .overflow_hidden()
                            .bg(theme::white_a(0.06))
                            .children(cover.map(|s| img(s).size_full().object_fit(ObjectFit::Cover))),
                    )
                    .child(div().w(px(170.)).flex_none().truncate().text_size(px(13.)).child(self.game_title(g)))
                    .child(bar)
                    .child(div().w(px(120.)).flex_none().flex().justify_end().text_size(px(12.5)).text_color(theme::text_muted()).child(
                        if diff >= 0. {
                            tr!("{pct}% longer", pct = format!("+{diff:.0}"))
                        } else {
                            tr!("{pct}% faster", pct = format!("{:.0}", -diff))
                        },
                    )),
            );
        }
        let legend = div()
            .flex()
            .gap(px(16.))
            .mt(px(14.))
            .text_size(px(12.))
            .text_color(theme::text_muted())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().w(px(18.)).h(px(4.)).rounded_full().bg(theme::white_a(0.16)))
                    .child(tr!("main story to completionist")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().w(px(2.)).h(px(12.)).bg(theme::white_a(0.6)))
                    .child(tr!("main + extras")),
            )
            .child(div().flex().items_center().gap(px(6.)).child(div().size(px(10.)).rounded_full().bg(theme::accent())).child(tr!("you")));
        Some(panel().child(head).child(list).child(legend))
    }

    /// Every game by status, and how far games get.
    fn backlog_collection(&self) -> Div {
        let Some(lib) = self.lib.clone() else { return panel() };
        let total = lib.games.len().max(1);
        let mut counts: Vec<(String, String, usize)> = lib
            .statuses
            .iter()
            .map(|s| (heronite_data::status::status_label(&s.name).to_string(), s.slug.clone(), self.facts.in_status(&s.id)))
            .filter(|c| c.2 > 0)
            .collect();
        counts.sort_by_key(|c| c.1 == "not-played");
        let bar = div().flex().gap(px(2.)).h(px(14.)).children(counts.iter().map(|(_, slug, n)| {
            div().h_full().flex_grow(1.).flex_basis(relative(*n as f32 / total as f32)).rounded(px(3.)).bg(theme::status_color(slug))
        }));
        let legend =
            div().flex().flex_wrap().gap_x(px(14.)).gap_y(px(6.)).mt(px(12.)).text_size(px(12.)).text_color(theme::text_muted()).children(
                counts.iter().map(|(name, slug, n)| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(div().size(px(8.)).rounded_full().bg(theme::status_color(slug)))
                        .child(name.clone())
                        .child(div().font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(n.to_string()))
                }),
            );
        let (opened, five) = (self.facts.opened, self.facts.played_5h);
        let beaten = self.facts.count(&BEATEN);
        let steps = [
            (tr!("In the Collection").to_string(), total, None),
            (tr!("Opened").to_string(), opened, Some(tr!("{pct}% of the Collection", pct = opened * 100 / total))),
            (tr!("Played 5h or more").to_string(), five, Some(tr!("{pct}% of those opened", pct = five * 100 / opened.max(1)))),
            (tr!("Beaten").to_string(), beaten, Some(tr!("{pct}% of those played 5h+", pct = beaten * 100 / five.max(1)))),
        ];
        let funnel = div().flex().flex_col().gap(px(10.)).mt(px(20.)).pt(px(16.)).border_t_1().border_color(theme::white_a(0.08)).children(
            steps.into_iter().map(|(label, n, note)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .w(px(150.))
                            .flex_none()
                            .flex()
                            .flex_col()
                            .child(
                                div().font_family(theme::font_heading()).font_weight(theme::W_BOLD).text_size(px(18.)).child(n.to_string()),
                            )
                            .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(label)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h(px(12.))
                            .child(div().h_full().w(relative(n as f32 / total as f32)).rounded_r(px(4.)).bg(theme::chart_ramp(0.7))),
                    )
                    .child(div().w(px(150.)).flex_none().text_size(px(12.)).text_color(theme::text_muted()).children(note))
            }),
        );
        panel()
            .child(panel_head(tr!("The whole Collection"), hint(trn!(total, "{n} game", "{n} games"))))
            .child(bar)
            .child(legend)
            .child(funnel)
    }

    /// Plan to play, with when each would be beaten at your pace.
    fn backlog_queue(&mut self, window: &Window, cx: &mut Context<Self>) -> Div {
        let pace = self.facts.pace;
        let mut queue = self.facts.with(&["plan-to-play"]);
        let main = |this: &Self, g: usize| this.backlog.hltb.get(&g).map(|h| h.main_story).filter(|h| *h > 0.);
        let played = |this: &Self, g: usize| this.lib.as_ref().and_then(|l| l.games.get(g)).map_or(0, |x| x.playtime_minutes);
        match self.queue_sort {
            QueueSort::Shortest => queue.sort_by(|a, b| main(self, *a).unwrap_or(f64::MAX).total_cmp(&main(self, *b).unwrap_or(f64::MAX))),
            QueueSort::Oldest => queue.sort_by_key(|g| self.backlog.added.get(g).copied().unwrap_or(NaiveDate::MAX)),
            QueueSort::Started => queue.sort_by_key(|g| std::cmp::Reverse(played(self, *g))),
        }
        let sorts = [
            (QueueSort::Shortest, tr!("Shortest")),
            (QueueSort::Oldest, tr!("Longest in the queue")),
            (QueueSort::Started, tr!("Started")),
        ];
        let mut seg = div().flex().gap(px(6.)).mb(px(12.));
        for (s, label) in sorts {
            let on = self.queue_sort == s;
            seg = seg.child(
                div()
                    .id(SharedString::from(format!("qsort-{s:?}")))
                    .h(px(30.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .rounded_full()
                    .text_size(px(12.5))
                    .font_weight(theme::W_BOLD)
                    .when(on, |d| d.bg(theme::accent()).text_color(theme::on_accent()))
                    .when(!on, |d| d.bg(theme::white_a(0.06)).text_color(theme::text_muted()).cursor_pointer())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.queue_sort = s;
                        cx.notify();
                    }))
                    .child(label),
            );
        }
        // Games planned for this year's goal ([`super::plans`]).
        let planned = self.plans_of(today().year());
        let mut list = div().flex().flex_col();
        let mut weeks_so_far = 0.;
        for g in queue.iter().take(8) {
            let g = *g;
            let cover = self.cover(g, 30., 40., window);
            let hours = main(self, g);
            let mine = played(self, g) as f64 / 60.;
            let since = self.backlog.added.get(&g).map(|d| tr!("in the queue since {day}", day = full_date(*d)));
            let estimate = hours.filter(|_| pace > 0.).map(|h| {
                let left = (h - mine).max(0.5);
                weeks_so_far += left / pace;
                (left / pace, weeks_so_far)
            });
            list = list.child(
                div()
                    .id(("bk-queue", g))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .py(px(8.))
                    .border_b_1()
                    .border_color(theme::white_a(0.05))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::white_a(0.03)))
                    .on_click(cx.listener(move |this, _, window, cx| this.open_sheet(g, window, cx)))
                    .child(
                        div()
                            .flex_none()
                            .w(px(30.))
                            .h(px(40.))
                            .rounded(px(5.))
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
                            .gap(px(2.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(div().text_size(px(13.5)).font_weight(theme::W_BOLD).truncate().child(self.game_title(g)))
                                    .when(planned.contains(&g), |d| d.child(super::super::status_pill(tr!("Planned"), theme::accent()))),
                            )
                            .child(div().text_size(px(12.)).text_color(theme::text_dim()).truncate().children(since)),
                    )
                    // Unknown times stay blank: a column repeating "unknown" says nothing.
                    .child(
                        div()
                            .w(px(150.))
                            .flex_none()
                            .text_size(px(12.5))
                            .text_color(theme::text_muted())
                            .children(hours.map(|h| tr!("{h}h main story", h = h.round()))),
                    )
                    .child(div().w(px(130.)).flex_none().flex().flex_col().items_end().children(estimate.map(|(w, total)| {
                        div()
                            .flex()
                            .flex_col()
                            .items_end()
                            .child(div().font_weight(theme::W_BOLD).text_size(px(13.)).child(if w < 1. {
                                tr!("under a week").to_string()
                            } else {
                                trn!(w.round() as usize, "~{n} week", "~{n} weeks")
                            }))
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(theme::text_dim())
                                    .child(tr!("done ~{day} if in order", day = short_date(today() + Duration::days((total * 7.) as i64)))),
                            )
                    }))),
            );
        }
        if queue.is_empty() {
            list = list.child(div().text_size(px(13.)).text_color(theme::text_muted()).child(tr!("No game is in Plan to play.")));
        }
        let lead =
            tr!("Plan to play, at your pace of {h}h a week (last 6 months, without endless games)", h = heronite_i18n::decimal(pace, 1));
        panel()
            .child(panel_head(tr!("The queue"), hint(trn!(queue.len(), "{n} game", "{n} games"))))
            .child(div().mt(px(-6.)).mb(px(12.)).child(hint(lead)))
            .child(seg)
            .child(list)
    }

    /// Games in play or on hold without a session for a while.
    fn backlog_shelf(&mut self, window: &Window, cx: &mut Context<Self>) -> Div {
        let t = today();
        let mut stale: Vec<(usize, Option<NaiveDate>)> = self
            .facts
            .with(&["playing", "on-hold"])
            .into_iter()
            .map(|g| (g, self.history.as_ref().and_then(|h| h.last_day(g))))
            .filter(|(_, d)| d.is_none_or(|d| (t - d).num_days() >= STALE_DAYS))
            .collect();
        stale.sort_by_key(|(_, d)| *d);
        let mut list = div().flex().flex_col();
        for (g, last) in stale.iter().take(7) {
            let g = *g;
            let cover = self.cover(g, 30., 40., window);
            let status = self.game_status(g);
            let key = self.game_key(g).map(|k| k.to_string()).unwrap_or_default();
            let played = self.lib.as_ref().and_then(|l| l.games.get(g)).map_or(0, |x| x.playtime_minutes);
            let detail = match last {
                Some(d) => {
                    tr!("{n} days without playing \u{b7} {time} played", n = (t - *d).num_days(), time = fmt_secs(played as f64 * 60.))
                }
                None => tr!("no recorded session \u{b7} {time} played", time = fmt_secs(played as f64 * 60.)),
            };
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .py(px(8.))
                    .border_b_1()
                    .border_color(theme::white_a(0.05))
                    .child(
                        div()
                            .flex_none()
                            .w(px(30.))
                            .h(px(40.))
                            .rounded(px(5.))
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
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(div().text_size(px(13.5)).font_weight(theme::W_BOLD).truncate().child(self.game_title(g)))
                                    .children(status.map(|(n, c)| super::super::status_pill(n, c))),
                            )
                            .child(div().text_size(px(12.)).text_color(theme::text_dim()).truncate().child(detail)),
                    )
                    .child(
                        crate::ui::controls::button_sm(("bk-open", g), crate::ui::controls::ButtonKind::Glass, tr!("Open in Collection"))
                            .on_click(cx.listener(move |_, _, _, cx| cx.emit(PageEvent::ShowGame(key.clone())))),
                    ),
            );
        }
        if stale.is_empty() {
            list = list.child(
                div().text_size(px(13.)).text_color(theme::text_muted()).child(tr!("Every game you are playing had a session this month.")),
            );
        }
        panel()
            .child(panel_head(tr!("On the shelf"), hint(trn!(stale.len(), "{n} game", "{n} games"))))
            .child(div().mt(px(-6.)).mb(px(12.)).child(hint(tr!("Playing or on hold, without a session for 30 days or more"))))
            .child(list)
    }

    /// Games added, started and beaten over time (the backlog grows when the
    /// first line runs away from the others).
    fn backlog_pile(&self) -> AnyElement {
        // Counted once per library ([`super::facts::Pile`]).
        let facts = self.facts.clone();
        let Some(pile) = facts.pile.as_ref() else { return div().into_any_element() };
        let from = pile.from;
        let series: [(String, Hsla, &Vec<usize>); 3] = [
            (trc!("stats", "Added").into(), theme::chart_series(0).into(), &pile.added),
            (tr!("Started").into(), theme::chart_series(1).into(), &pile.started),
            (tr!("Beaten").into(), theme::chart_series(2).into(), &pile.beaten),
        ];
        let max = series.iter().flat_map(|s| s.2.iter().copied()).max().unwrap_or(1).max(1);
        let lines: Vec<(Hsla, Vec<f32>)> =
            series.iter().map(|(_, c, v)| (*c, v.iter().map(|n| *n as f32 / max as f32).collect())).collect();
        let chart_h = 180.;
        let plot = div()
            .relative()
            .h(px(chart_h))
            .border_b_1()
            .border_color(theme::white_a(0.12))
            .child(canvas(|_, _, _| (), move |bounds, _, window, _| paint_lines(bounds, &lines, window)).absolute().inset_0());
        let first_label = format!("{} {}", month_short(from.month()), from.year());
        let ends = div()
            .flex()
            .justify_between()
            .mt(px(6.))
            .text_size(px(11.5))
            .text_color(theme::text_dim())
            .child(first_label)
            .child(tr!("today"));
        let legend =
            div().flex().gap(px(18.)).text_size(px(12.5)).text_color(theme::text_muted()).children(series.iter().map(|(name, c, v)| {
                div().flex().items_center().gap(px(7.)).child(div().w(px(14.)).h(px(3.)).rounded_full().bg(*c)).child(name.clone()).child(
                    div().font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(v.last().copied().unwrap_or(0).to_string()),
                )
            }));
        let gap = series[0].2.last().copied().unwrap_or(0).saturating_sub(series[1].2.last().copied().unwrap_or(0));
        panel()
            .child(panel_head(tr!("The pile"), legend))
            .child(div().mt(px(-6.)).mb(px(14.)).child(hint(tr!("{n} games added and never started", n = gap))))
            .child(plot)
            .child(ends)
            .into_any_element()
    }
}

/// The goal ring: a track, the progress arc and a tick where today's pace is.
fn paint_ring(bounds: Bounds<Pixels>, progress: f32, expected: f32, color: Hsla, window: &mut Window) {
    let c = bounds.center();
    let r = f32::from(bounds.size.width.min(bounds.size.height)) / 2. - 6.;
    let arc = |from: f32, to: f32, width: f32, color: Hsla, window: &mut Window| {
        if to <= from {
            return;
        }
        let mut b = PathBuilder::stroke(px(width));
        let steps = ((to - from) * 96.).ceil().max(2.) as usize;
        for i in 0..=steps {
            let t = from + (to - from) * i as f32 / steps as f32;
            let a = t * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
            let p = point(c.x + px(r * a.cos()), c.y + px(r * a.sin()));
            if i == 0 {
                b.move_to(p);
            } else {
                b.line_to(p);
            }
        }
        if let Ok(path) = b.build() {
            window.paint_path(path, color);
        }
    };
    arc(0., 1., 9., theme::white_a(0.1), window);
    arc(0., progress, 9., color, window);
    if expected > 0. && expected < 1. {
        arc(expected - 0.004, expected + 0.004, 15., theme::white_a(0.85), window);
    }
}

/// Lines of values in 0..=1 across the bounds.
fn paint_lines(bounds: Bounds<Pixels>, lines: &[(Hsla, Vec<f32>)], window: &mut Window) {
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    for (color, values) in lines {
        if values.len() < 2 {
            continue;
        }
        let mut b = PathBuilder::stroke(px(2.));
        for (i, v) in values.iter().enumerate() {
            let p = bounds.origin + point(px(w * i as f32 / (values.len() - 1) as f32), px(h - v * (h - 4.) - 1.));
            if i == 0 {
                b.move_to(p);
            } else {
                b.line_to(p);
            }
        }
        if let Ok(path) = b.build() {
            window.paint_path(path, *color);
        }
    }
}
