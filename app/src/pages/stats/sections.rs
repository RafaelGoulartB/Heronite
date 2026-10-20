//! Panels below the hero: the day-by-day chart (week and month), the top
//! games, when you play, the records, every day of the year and the genres.

use chrono::{Datelike, Duration, NaiveDate};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::stats::{fmt_secs, month_name, month_short, Span, Stats, TOP_N};

use super::desktop::{hint, panel, panel_head, GAP, R_PANEL};
use super::marks::{marks, Mark};
use super::review::marked;
use super::tip::hover;
use super::{axis_label, day_label, day_part, nice_axis, series_color, short_date, today, weekday, weekday_on, Hover, StatsPage};
use heronite_i18n::{tr, trc, trn};

use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

const CHART_H: f32 = 200.;
/// Covers on the shelf.
const SHELF: usize = 8;

/// `n` of 12 columns of the page grid (`w` is the column width).
fn span(w: f32, n: usize) -> f32 {
    (w - GAP * 11.) / 12. * n as f32 + GAP * (n - 1) as f32
}

fn bold(text: impl Into<SharedString>) -> Div {
    div().font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(text.into())
}

/// Text made of plain and bold parts on one line.
fn line(parts: Vec<Div>) -> Div {
    div().flex().flex_wrap().items_baseline().gap_x(px(4.)).children(parts)
}

/// A sentence with `**bold**` runs.
/// Rows of the "Show all" list of games.
const LIST_MAX: usize = 100;

pub(super) fn sentence(text: impl AsRef<str>) -> Div {
    div().child(marked(text))
}

impl StatsPage {
    pub(super) fn render_sections(
        &mut self,
        st: &Stats,
        col_w: f32,
        wide: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut out = vec![self.render_portals(st, window, cx)];
        if matches!(st.period.span, Span::Year | Span::All) && st.buckets.len() > 1 {
            out.push(self.render_months(st, window, cx));
        } else {
            out.push(self.render_activity(st, cx));
        }
        out.push(self.render_shelf(st, col_w, window, cx));
        let habits = self.render_habits(st, cx);
        let records = self.render_records(st, if wide { span(col_w, 8) } else { col_w }, window, cx);
        out.push(row(wide, vec![(habits, 4), (records, 8)]));
        let genres = self.render_genres(st, cx);
        if st.period.span == Span::Year {
            let calendar = self.render_calendar(st, if wide { span(col_w, 8) } else { col_w }, cx);
            out.push(row(wide, vec![(calendar, 8), (genres, 4)]));
        } else {
            out.push(genres.into_any_element());
        }
        out
    }

    /// Three cards under the hero, one per other tab, each with what that tab
    /// has to say about the period.
    fn render_portals(&mut self, st: &Stats, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        use super::tabs::Tab;
        let mut cards: Vec<(Tab, Icon, String, String, Option<usize>)> = Vec::new();
        if let Some(j) = self.journeys.clone() {
            let text = match j.peak().filter(|p| p.1 > 1) {
                Some((_, n)) => tr!("Up to **{n} games** at the same time", n),
                None => trn!(j.lanes.len(), "**{n} journey** of 1h or more", "**{n} journeys** of 1h or more"),
            };
            let game = j.lanes.iter().max_by(|a, b| a.secs.total_cmp(&b.secs)).map(|l| l.game);
            cards.push((Tab::Journeys, Icon::History, tr!("Journeys").into(), text, game));
        }
        let year = match st.period.span {
            Span::All => today().year(),
            _ => st.period.start.year(),
        };
        let beaten = self.beaten_in_year(year);
        let target = self.prefs.goals.get(&year).copied().unwrap_or(12);
        let last = beaten.last().map(|b| b.0);
        cards.push((
            Tab::Backlog,
            Icon::Flag,
            tr!("Goal for {year}", year),
            tr!("**{done} of {target}** games beaten", done = beaten.len(), target),
            last,
        ));
        if let Some(lead) = self.shown_insights().into_iter().next() {
            cards.push((Tab::Discoveries, Icon::Lightbulb, tr!("Discovery").into(), self.insight_sentence(&lead.fact), lead.fact.game()));
        }
        // The period's screenshots per game, most first ([`super::Derived`]).
        let derived = self.derived.clone();
        if let Some((g, _)) = derived.shots.first() {
            let text = tr!(
                "**{n} screenshots**, most in {game}",
                n = heronite_data::stats::group_thousands(derived.shots_total as u64),
                game = self.game_title(*g)
            );
            cards.push((Tab::Album, Icon::PhotoCamera, tr!("Album").into(), text, Some(*g)));
        }
        let mut row = div().flex().gap(px(GAP));
        for (tab, ic, kicker, text, game) in cards {
            let cover = game.and_then(|g| self.cover(g, 30., 40., window));
            row = row.child(
                div()
                    .id(SharedString::from(format!("portal-{}", tab.id())))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .px(px(18.))
                    .py(px(14.))
                    .rounded(px(16.))
                    .bg(rgba(0x161b24d6))
                    .border_1()
                    .border_color(theme::white_a(0.08))
                    .cursor_pointer()
                    .hover(|s| s.border_color(Hsla { a: 0.45, ..Hsla::from(theme::accent()) }))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_tab(tab, cx)))
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
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .font_weight(theme::W_BOLD)
                                    .text_color(theme::text_muted())
                                    .child(kicker.to_uppercase()),
                            )
                            .child(div().text_size(px(13.5)).truncate().child(sentence(text))),
                    )
                    .children(cover.map(|s| {
                        div()
                            .flex_none()
                            .w(px(30.))
                            .h(px(40.))
                            .rounded(px(5.))
                            .overflow_hidden()
                            .child(img(s).size_full().object_fit(ObjectFit::Cover))
                    }))
                    .child(icon(Icon::ChevronRight, px(16.), theme::text_muted())),
            );
        }
        row.into_any_element()
    }

    /// Week and month: one bar per day, stacked by game.
    fn render_activity(&self, st: &Stats, cx: &mut Context<Self>) -> AnyElement {
        let today = today();
        let max = st.buckets.iter().map(|b| b.secs).fold(0., f64::max);
        let (step, top) = nice_axis(max / 3600.);
        let top_secs = top * 3600.;
        let lines = (top / step).round() as usize;
        let grid = (0..=lines).map(|i| {
            let y = CHART_H * i as f32 / lines as f32;
            div().absolute().left_0().right_0().bottom(px(y)).h(px(1.)).bg(theme::white_a(if i == 0 { 0.12 } else { 0.06 }))
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
        let bars = st.buckets.iter().enumerate().map(|(i, b)| {
            let on = self.hover == Some(Hover::Bar(i));
            let mut bar =
                div().w_full().max_w(px(if n <= 7 { 46. } else { 26. })).flex().flex_col().gap(px(2.)).rounded_t(px(4.)).overflow_hidden();
            let other = (b.secs - b.top.iter().sum::<f64>()).max(0.);
            let segs = std::iter::once((other, theme::chart_other())).chain((0..TOP_N).rev().map(|r| (b.top[r], series_color(Some(r)))));
            for (secs, color) in segs.collect::<Vec<_>>().into_iter().rev() {
                let h = (secs / top_secs) as f32 * CHART_H;
                if h >= 0.5 {
                    bar = bar.child(div().w_full().h(px(h)).bg(color));
                }
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
                .px(px(if n > 20 { 2. } else { 6. }))
                .rounded_t(px(8.))
                .when(on, |d| d.bg(theme::white_a(0.05)))
                .on_hover(hover(Hover::Bar(i), cx))
                .child(bar)
        });
        let x_labels = st.buckets.iter().enumerate().map(|(i, b)| {
            let current = b.start <= today && today < b.end;
            let text = match st.period.span {
                Span::Week => tr!("{weekday} {day}", weekday = weekday(i % 7), day = b.start.day()),
                _ => b.start.day().to_string(),
            };
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .justify_center()
                .text_size(px(11.5))
                .whitespace_nowrap()
                .text_color(if current { theme::accent() } else { theme::text_dim() })
                .when(current, |d| d.font_weight(theme::W_BOLD))
                .child(text)
        });
        let legend = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(16.))
            .mt(px(18.))
            .pt(px(14.))
            .border_t_1()
            .border_color(theme::white_a(0.08))
            .children(st.series.iter().enumerate().map(|(slot, &g)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .max_w(px(220.))
                    .text_size(px(12.5))
                    .text_color(theme::text_muted())
                    .child(div().flex_none().size(px(8.)).rounded_full().bg(series_color(Some(slot))))
                    .child(div().truncate().child(self.game_title(g)))
            }));
        let busiest = st.busiest.map(|(d, s)| tr!("Busiest day: {day} \u{b7} {time}", day = day_label(d), time = fmt_secs(s)));
        panel()
            .child(panel_head(tr!("Day by day"), hint(busiest.unwrap_or_default())))
            .child(
                div().flex().child(div().relative().flex_none().w(px(40.)).h(px(CHART_H)).children(labels)).child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(div().relative().h(px(CHART_H)).children(grid).child(div().absolute().inset_0().flex().children(bars)))
                        .child(div().flex().mt(px(8.)).children(x_labels)),
                ),
            )
            .child(legend)
            .into_any_element()
    }

    fn render_shelf(&mut self, st: &Stats, col_w: f32, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let cols = if col_w >= 1100. { SHELF } else { 4 };
        let inner = col_w - 48.;
        let cw = (inner - 16. * (cols - 1) as f32) / cols as f32;
        let ch = cw * 4. / 3.;
        let max = st.games.first().map_or(1., |g| g.secs.max(1.));
        let mut covers = Vec::new();
        for (rank, g) in st.games.iter().take(cols.max(SHELF).min(if cols == 4 { 4 } else { SHELF })).enumerate() {
            let src = self.cover(g.game, cw, ch, window);
            let game = g.game;
            let c = series_color(st.series_of(g.game));
            let first = rank == 0;
            covers.push(
                div()
                    .id(("shelf", g.game))
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .cursor_pointer()
                    .on_hover(hover(Hover::Game(g.game), cx))
                    .on_click(cx.listener(move |this, _, window, cx| this.open_sheet(game, window, cx)))
                    .child(
                        div()
                            .relative()
                            .w(px(cw))
                            .h(px(ch))
                            .rounded(px(16.))
                            .overflow_hidden()
                            .bg(rgb(0x11151c))
                            .when(first, |d| d.border_2().border_color(Hsla { a: 0.55, ..Hsla::from(theme::accent()) }))
                            .shadow(vec![BoxShadow {
                                color: theme::black_a(0.45),
                                offset: point(px(0.), px(10.)),
                                blur_radius: px(30.),
                                spread_radius: px(0.),
                                inset: false,
                            }])
                            .children(src.map(|s| img(s).absolute().inset_0().size_full().object_fit(ObjectFit::Cover)))
                            .child(div().absolute().left_0().right_0().bottom_0().h(relative(0.55)).bg(linear_gradient(
                                180.,
                                linear_color_stop(theme::black_a(0.), 0.),
                                linear_color_stop(Hsla { a: 0.88, ..Hsla::from(theme::stats_page_bg()) }, 1.),
                            )))
                            .when(g.new, |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .right(px(10.))
                                        .top(px(10.))
                                        .h(px(22.))
                                        .px(px(9.))
                                        .flex()
                                        .items_center()
                                        .rounded_full()
                                        .bg(rgba(0x0a0c10b3))
                                        .border_1()
                                        .border_color(theme::white_a(0.18))
                                        .font_family(theme::font_heading())
                                        .font_weight(theme::W_BOLD)
                                        .text_size(px(11.))
                                        .child(tr!("New")),
                                )
                            })
                            .child(
                                div()
                                    .absolute()
                                    .left(px(12.))
                                    .bottom(px(4.))
                                    .font_family(theme::font_heading())
                                    .font_weight(theme::W_BOLD)
                                    .text_size(px((cw * 0.3).clamp(30., 58.)))
                                    .line_height(px((cw * 0.3).clamp(30., 58.)))
                                    .child((rank + 1).to_string()),
                            ),
                    )
                    .child(
                        div()
                            .h(px(4.))
                            .mt(px(12.))
                            .mb(px(10.))
                            .rounded_full()
                            .bg(theme::white_a(0.08))
                            .child(div().h_full().w(relative((g.secs / max) as f32)).rounded_full().bg(c)),
                    )
                    .child(div().text_size(px(13.)).font_weight(theme::W_BOLD).truncate().child(self.game_title(g.game)))
                    .child(
                        div()
                            .mt(px(2.))
                            .font_family(theme::font_heading())
                            .font_weight(theme::W_BOLD)
                            .text_size(px(20.))
                            .child(fmt_secs(g.secs)),
                    )
                    .child(div().mt(px(1.)).text_size(px(12.)).text_color(theme::text_muted()).truncate().child({
                        let sessions = trn!(g.sessions, "{n} session", "{n} sessions");
                        let pct = format!("{:.0}", g.secs / st.total_secs.max(1.) * 100.);
                        match st.period.span {
                            Span::Week => tr!("{sessions} \u{b7} {pct}% of the week", sessions, pct),
                            Span::Month => tr!("{sessions} \u{b7} {pct}% of the month", sessions, pct),
                            Span::Year => tr!("{sessions} \u{b7} {pct}% of the year", sessions, pct),
                            Span::All => tr!("{sessions} \u{b7} {pct}% of the total", sessions, pct),
                        }
                    })),
            );
        }
        let new = st.new_games();
        let beaten = self.derived.beaten.len();
        let mut facts = Vec::new();
        if new > 0 {
            facts.push(if new == 1 { tr!("{n} new", n = new) } else { trc!("plural", "{n} new", n = new) });
        }
        if beaten > 0 {
            facts.push(if beaten == 1 { tr!("{n} beaten", n = beaten) } else { trc!("plural", "{n} beaten", n = beaten) });
        }
        let toggle = (st.games.len() > covers.len()).then(|| {
            let label = if self.show_all { tr!("Show less").to_string() } else { tr!("Show all {n}", n = st.games.len()) };
            div()
                .id("stats-show-all")
                .px(px(12.))
                .py(px(5.))
                .rounded_full()
                .bg(Hsla { a: 0.1, ..Hsla::from(theme::accent()) })
                .hover(|s| s.bg(Hsla { a: 0.2, ..Hsla::from(theme::accent()) }))
                .cursor_pointer()
                .font_family(theme::font_heading())
                .font_weight(theme::W_BOLD)
                .text_size(px(12.5))
                .text_color(theme::accent())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.show_all = !this.show_all;
                    cx.notify();
                }))
                .child(label)
        });
        let journeys = div()
            .id("stats-to-journeys")
            .flex()
            .items_center()
            .gap(px(4.))
            .text_size(px(12.5))
            .font_weight(theme::W_BOLD)
            .text_color(theme::accent())
            .cursor_pointer()
            .hover(|s| s.opacity(0.8))
            .on_click(cx.listener(|this, _, _, cx| this.set_tab(super::tabs::Tab::Journeys, cx)))
            .child(tr!("Every journey"))
            .child(icon(Icon::ChevronRight, px(13.), theme::accent()));
        let head = div().flex().items_center().gap(px(14.)).child(hint(facts.join(" \u{b7} "))).children(toggle).child(journeys);
        let mut p =
            panel().child(panel_head(tr!("Top games"), head)).child(div().grid().grid_cols(cols as u16).gap(px(16.)).children(covers));
        if self.show_all {
            p = p.child(self.render_list(st, cols, window, cx));
        }
        p.into_any_element()
    }

    /// Every game of the period after the shelf ("Show all").
    fn render_list(&mut self, st: &Stats, skip: usize, window: &Window, cx: &mut Context<Self>) -> Div {
        let max = st.games.first().map_or(1., |g| g.secs.max(1.));
        let mut list = div().flex().flex_col().mt(px(18.)).pt(px(8.)).border_t_1().border_color(theme::white_a(0.08));
        // All time on a big library holds thousands of games: the list stops
        // at [`LIST_MAX`] (the rest are the ones played least).
        for (rank, g) in st.games.iter().enumerate().skip(skip).take(LIST_MAX) {
            let cover = self.cover(g.game, 30., 40., window);
            let game = g.game;
            let meta = match g.last {
                Some(last) => {
                    tr!(
                        "{sessions} \u{b7} last {when}",
                        sessions = trn!(g.sessions, "{n} session", "{n} sessions"),
                        when = super::when_label(last)
                    )
                }
                None => tr!("No recorded sessions").into(),
            };
            list = list.child(
                div()
                    .id(("stats-row", g.game))
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .py(px(8.))
                    .px(px(8.))
                    .mx(px(-8.))
                    .rounded(px(10.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::white_a(0.04)))
                    .on_click(cx.listener(move |this, _, window, cx| this.open_sheet(game, window, cx)))
                    .child(div().w(px(24.)).text_size(px(12.)).text_color(theme::text_dim()).child((rank + 1).to_string()))
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
                            .gap(px(5.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(div().truncate().text_size(px(13.5)).child(self.game_title(g.game)))
                                    .when(g.new, |d| d.child(super::super::status_pill(tr!("New"), theme::accent()))),
                            )
                            .child(div().h(px(4.)).rounded_full().bg(theme::white_a(0.06)).child(
                                div().h_full().w(relative((g.secs / max) as f32)).rounded_full().bg(series_color(st.series_of(g.game))),
                            ))
                            .child(div().text_size(px(12.)).text_color(theme::text_dim()).truncate().child(meta)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .min_w(px(80.))
                            .flex()
                            .justify_end()
                            .font_family(theme::font_heading())
                            .font_weight(theme::W_BOLD)
                            .text_size(px(14.))
                            .child(fmt_secs(g.secs)),
                    ),
            );
        }
        let rest = st.games.len().saturating_sub(skip + LIST_MAX);
        if rest > 0 {
            list = list.child(div().pt(px(12.)).text_size(px(12.5)).text_color(theme::text_muted()).child(trn!(
                rest,
                "and {n} more game",
                "and {n} more games"
            )));
        }
        list
    }

    fn render_habits(&self, st: &Stats, cx: &mut Context<Self>) -> Div {
        let day_totals: Vec<f64> = st.heat.iter().map(|r| r.iter().sum()).collect();
        let total: f64 = day_totals.iter().sum();
        let max_cell = st.heat.iter().flatten().copied().fold(0., f64::max).max(1.);
        let argmax = |v: &[f64]| v.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map_or(0, |(i, _)| i);
        let best_day = argmax(&day_totals);
        let best_part = argmax(&st.day_parts);
        // 168 cells painted as one element ([`super::marks`]), the weekdays beside.
        let (heat_values, on) = (st.heat, self.hover);
        let cells = marks(
            "heat",
            move |w, _| {
                let cw = (w - 2. * 23.) / 24.;
                let mut out = Vec::with_capacity(168);
                for (d, hours) in heat_values.iter().enumerate() {
                    for (h, v) in hours.iter().enumerate() {
                        let color: Hsla =
                            if *v <= 0. { theme::chart_empty() } else { theme::chart_ramp(((*v / max_cell) as f32).powf(0.7)).into() };
                        let hover = Hover::Heat(d, h);
                        out.push(
                            Mark::new(h as f32 * (cw + 2.), d as f32 * 23., cw, 21., color)
                                .radius(4.)
                                .ring(on == Some(hover))
                                .hover(Some(hover)),
                        );
                    }
                }
                out
            },
            cx,
        )
        .flex_1()
        .h(px(7. * 23. - 2.));
        let labels = div().flex_none().w(px(32.)).flex().flex_col().gap(px(2.)).children(
            (0..7).map(|d| div().h(px(21.)).flex().items_center().text_size(px(11.)).text_color(theme::text_dim()).child(weekday(d))),
        );
        let heat = div().flex().child(labels).child(cells);
        let hours = div()
            .flex()
            .mt(px(4.))
            .child(div().flex_none().w(px(32.)))
            .children((0..4).map(|q| div().flex_1().text_size(px(11.)).text_color(theme::text_dim()).child(format!("{:02}", q * 6))));
        let scale = div()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(3.))
            .mt(px(8.))
            .text_size(px(11.))
            .text_color(theme::text_dim())
            .child(tr!("Less"))
            .children((0..10).map(|i| div().w(px(14.)).h(px(8.)).rounded(px(2.)).bg(theme::chart_ramp(i as f32 / 9.))))
            .child(tr!("More"));
        let part_colors = [theme::chart_ramp(0.2), theme::chart_ramp(0.), theme::chart_ramp(0.66), theme::chart_ramp(1.)];
        let part_icons = [Icon::Moon, Icon::Sunrise, Icon::Sun, Icon::Sunset];
        let pbar = div().flex().gap(px(2.)).h(px(14.)).children((0..4).filter(|i| st.day_parts[*i] > 0.).map(|i| {
            div()
                .id(("part-bar", i))
                .h_full()
                .flex_grow(1.)
                .flex_basis(relative((st.day_parts[i] / total.max(1.)) as f32))
                .rounded(px(3.))
                .bg(part_colors[i])
                .on_hover(hover(Hover::Part(i), cx))
        }));
        let plist = div().grid().grid_cols(4).gap(px(8.)).mt(px(12.)).children((0..4).map(|i| {
            let share = st.day_parts[i] / total.max(1.) * 100.;
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(12.))
                        .text_color(theme::text_muted())
                        .child(icon(part_icons[i], px(14.), theme::text_muted()))
                        .child(day_part(i).0),
                )
                .child(
                    div()
                        .font_family(theme::font_heading())
                        .font_weight(theme::W_BOLD)
                        .text_size(px(22.))
                        .text_color(if i == best_part { theme::accent() } else { theme::text_primary() })
                        .child(format!("{share:.0}%")),
                )
                .child(div().text_size(px(11.)).text_color(theme::text_dim()).child(day_part(i).1))
        }));
        let weekend = st.weekend_secs / total.max(1.);
        let wk = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .mt(px(14.))
            .text_size(px(12.5))
            .text_color(theme::text_muted())
            .child(tr!("Weekends"))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .gap(px(2.))
                    .h(px(8.))
                    .child(div().h_full().flex_basis(relative(weekend as f32)).flex_grow(1.).rounded(px(4.)).bg(theme::chart_ramp(1.)))
                    .child(
                        div().h_full().flex_basis(relative(1. - weekend as f32)).flex_grow(1.).rounded(px(4.)).bg(theme::chart_ramp(0.2)),
                    ),
            )
            .child(sentence(tr!("**{pct}%** of your time", pct = format!("{:.0}", weekend * 100.))));
        let day = weekday_on(best_day);
        let lead = sentence(match best_part {
            0 => tr!("You play most **{day}**, usually in the **night**.", day),
            1 => tr!("You play most **{day}**, usually in the **morning**.", day),
            2 => tr!("You play most **{day}**, usually in the **afternoon**.", day),
            _ => tr!("You play most **{day}**, usually in the **evening**.", day),
        });
        panel()
            .flex_1()
            .child(panel_head(tr!("When you play"), div()))
            .child(lead.mt(px(-6.)).mb(px(14.)).text_size(px(13.)).text_color(theme::text_muted()))
            .child(heat)
            .child(hours)
            .child(scale)
            .child(div().mt(px(18.)).pt(px(18.)).border_t_1().border_color(theme::white_a(0.08)).child(pbar).child(plist).child(wk))
    }

    /// One record card: art on the right, label, value and detail.
    fn record(&mut self, game: Option<usize>, w: f32, label: &str, value: String, detail: Div, extra: Option<Div>, window: &Window) -> Div {
        let art = game.and_then(|g| self.art(g, w * 0.78, 240., window));
        let fill = rgb(0x161b24);
        let ink = |a: f32| Hsla { a, ..Hsla::from(fill) };
        let mut card = div()
            .relative()
            .flex()
            .flex_col()
            .min_h(px(208.))
            .rounded(px(R_PANEL))
            .overflow_hidden()
            .bg(Hsla { a: 0.94, ..Hsla::from(fill) })
            .border_1()
            .border_color(theme::white_a(0.08));
        if let Some(src) = art {
            card = card
                .child(
                    img(src).absolute().top_0().bottom_0().right_0().w(relative(0.78)).h_full().object_fit(ObjectFit::Cover).opacity(0.9),
                )
                .child(div().absolute().top_0().bottom_0().left_0().w(relative(0.32)).bg(linear_gradient(
                    90.,
                    linear_color_stop(ink(0.97), 0.),
                    linear_color_stop(ink(0.9), 1.),
                )))
                .child(div().absolute().top_0().bottom_0().left(relative(0.32)).right_0().bg(linear_gradient(
                    90.,
                    linear_color_stop(ink(0.9), 0.),
                    linear_color_stop(ink(0.35), 1.),
                )));
        }
        card.child(
            div()
                .relative()
                .flex()
                .flex_col()
                .flex_1()
                .px(px(22.))
                .py(px(20.))
                .child(
                    div()
                        .font_family(theme::font_heading())
                        .font_weight(theme::W_BOLD)
                        .text_size(px(11.5))
                        .text_color(theme::text_muted())
                        .child(label.to_uppercase()),
                )
                .child(
                    div()
                        .mt_auto()
                        .font_family(theme::font_heading())
                        .font_weight(theme::W_BOLD)
                        .text_size(px(46.))
                        .line_height(px(48.))
                        .child(value),
                )
                .child(detail.mt(px(8.)).text_size(px(13.)).text_color(theme::white_a(0.82)))
                .children(extra),
        )
    }

    fn render_records(&mut self, st: &Stats, w: f32, window: &Window, cx: &mut Context<Self>) -> Div {
        let card_w = (w - GAP) / 2.;
        let mut cards: Vec<(Option<usize>, Div)> = Vec::new();
        // A game's art shows once: the longest session and the streak are
        // often the same game, and two equal cards read as a glitch.
        let mut shown: Vec<usize> = Vec::new();
        let mut art = |g: Option<usize>| {
            let g = g.filter(|g| !shown.contains(g));
            shown.extend(g);
            g
        };
        if let Some(l) = st.longest {
            let detail = sentence(tr!("**{game}** \u{b7} {day}", game = self.game_title(l.game), day = day_label(l.start.date())));
            cards.push((
                Some(l.game),
                self.record(art(Some(l.game)), card_w, tr!("Longest session"), fmt_secs(l.secs), detail, None, window),
            ));
        }
        if let Some((d, secs)) = st.busiest {
            let same = st.longest.is_some_and(|l| l.start.date() == d);
            let day = day_label(d);
            let detail = sentence(if same {
                tr!("**{day}** \u{b7} same day as your longest session", day)
            } else {
                tr!("**{day}** \u{b7} your most intense day", day)
            });
            // That month, day by day.
            let first = NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap_or(d);
            let next = if d.month() == 12 {
                NaiveDate::from_ymd_opt(d.year() + 1, 1, 1)
            } else {
                NaiveDate::from_ymd_opt(d.year(), d.month() + 1, 1)
            }
            .unwrap_or(d);
            let mut bars = div().flex().items_end().gap(px(2.)).h(px(64.));
            let mut day = first;
            while day < next {
                let v = st.day_secs(day);
                bars = bars.child(
                    div().flex_1().h(px(((v / secs) as f32 * 64.).max(if v > 0. { 2. } else { 0. }))).rounded_t(px(2.)).bg(if day == d {
                        theme::chart_ramp(1.).into()
                    } else {
                        Hsla::from(theme::chart_ramp(0.55))
                    }),
                );
                day += Duration::days(1);
            }
            let mini = div().absolute().right(px(22.)).top(px(44.)).w(relative(0.38)).child(bars).child(
                div()
                    .flex()
                    .justify_between()
                    .mt(px(5.))
                    .text_size(px(11.))
                    .text_color(theme::text_muted())
                    .child(tr!("{month} 1", month = month_short(d.month())))
                    .child(short_date(next - Duration::days(1))),
            );
            cards.push((None, self.record(None, card_w, tr!("Busiest day"), fmt_secs(secs), detail, None, window).child(mini)));
        }
        if let (Some((a, z)), true) = (st.streak_range, st.streak > 1) {
            let detail = sentence(tr!("**{from} \u{2013} {to}** \u{b7} every single day", from = short_date(a), to = short_date(z)));
            let dots = div().flex().flex_wrap().gap(px(2.)).mt(px(10.)).children(
                (0..st.streak.min(60)).map(|_| div().size(px(6.)).rounded(px(2.)).bg(Hsla { a: 0.85, ..Hsla::from(theme::accent()) })),
            );
            cards.push((
                st.streak_game,
                self.record(
                    art(st.streak_game),
                    card_w,
                    tr!("Longest streak"),
                    trn!(st.streak, "{n} day", "{n} days"),
                    detail,
                    Some(dots),
                    window,
                ),
            ));
        }
        match st.longest_break.filter(|b| b.days >= 3) {
            Some(b) => {
                let off = (b.last + Duration::days(1), b.back - Duration::days(1));
                let back_game = st
                    .buckets
                    .iter()
                    .position(|x| x.start <= b.back && b.back < x.end)
                    .and_then(|i| st.bucket_games.get(i)?.first().map(|g| g.0));
                let (from, to, back) = (short_date(off.0), short_date(off.1), short_date(b.back));
                let detail = sentence(match back_game {
                    Some(g) => tr!("**{from} \u{2013} {to}** \u{b7} back on {back} with {game}", from, to, back, game = self.game_title(g)),
                    None => tr!("**{from} \u{2013} {to}** \u{b7} back on {back}", from, to, back),
                });
                cards.push((
                    back_game,
                    self.record(art(back_game), card_w, tr!("Longest break"), trn!(b.days, "{n} day", "{n} days"), detail, None, window),
                ));
            }
            None => {
                let new = st.new_games();
                let detail = sentence(trn!(new, "**{n} game** played for the first time", "**{n} games** played for the first time"));
                let first_new = st.games.iter().find(|g| g.new).map(|g| g.game);
                cards.push((
                    first_new,
                    self.record(art(first_new), card_w, tr!("Games played"), st.games.len().to_string(), detail, None, window),
                ));
            }
        }
        // A card with a game's art opens that game's sheet.
        let cards = cards.into_iter().enumerate().map(|(i, (game, card))| match game {
            Some(g) => div()
                .id(("record", i))
                .flex()
                .flex_col()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, window, cx| this.open_sheet(g, window, cx)))
                .child(card.flex_1())
                .into_any_element(),
            None => card.into_any_element(),
        });
        div().grid().grid_cols(2).gap(px(GAP)).flex_1().children(cards)
    }

    fn render_calendar(&self, st: &Stats, w: f32, cx: &mut Context<Self>) -> Div {
        let year = st.period.start.year();
        let jan1 = st.period.start;
        let offset = jan1.weekday().num_days_from_monday() as i64;
        let first_monday = jan1 - Duration::days(offset);
        let dec31 = st.period.end - Duration::days(1);
        let weeks = ((dec31 - first_monday).num_days() / 7 + 1) as usize;
        let inner = w - 48. - 34.;
        let gap = 3.;
        let cell = ((inner - gap * (weeks - 1) as f32) / weeks as f32).floor();
        let max = st.busiest.map_or(1., |b| b.1.max(1.));
        let today = today();
        let col_of = |d: NaiveDate| ((d - first_monday).num_days() / 7) as usize;
        // 371 cells painted as one element ([`super::marks`]).
        let mut cells = Vec::with_capacity(weeks * 7);
        for wk in 0..weeks {
            for wd in 0..7 {
                let d = first_monday + Duration::days((wk * 7 + wd) as i64);
                if d.year() != year {
                    continue;
                }
                let v = st.day_secs(d);
                let color: Hsla = if d > today {
                    theme::white_a(0.022)
                } else if v <= 0. {
                    theme::chart_empty()
                } else {
                    theme::chart_ramp(((v / max) as f32).powf(0.6)).into()
                };
                let (x, y) = (wk as f32 * (cell + gap), wd as f32 * (cell + gap));
                cells.push(
                    Mark::new(x, y, cell, cell, color)
                        .radius(3.)
                        .ring(self.hover == Some(Hover::Day(d)))
                        .hover((d <= today).then_some(Hover::Day(d))),
                );
            }
        }
        let grid = marks("calendar", move |_, _| cells, cx).w(px(weeks as f32 * (cell + gap) - gap)).h(px(7. * (cell + gap) - gap));
        let step = cell + gap;
        let months = div().relative().h(px(18.)).ml(px(34.)).children((1..=12).filter_map(|m| {
            let d = NaiveDate::from_ymd_opt(year, m, 1)?;
            Some(
                div()
                    .absolute()
                    .top_0()
                    .left(px(col_of(d) as f32 * step))
                    .text_size(px(11.5))
                    .text_color(theme::text_muted())
                    .child(month_short(m)),
            )
        }));
        let wdays = div().flex().flex_col().gap(px(gap)).w(px(34.)).children((0..7).map(|i| {
            div()
                .h(px(cell))
                .flex()
                .items_center()
                .text_size(px(11.))
                .text_color(theme::text_dim())
                .children((i % 2 == 0).then(|| weekday(i)))
        }));
        // Brackets under the streak and the longest break.
        let mut notes = div().relative().h(px(36.)).mt(px(8.)).ml(px(34.));
        let bracket = |a: NaiveDate, z: NaiveDate, label: Div| {
            let (x0, x1) = (col_of(a) as f32 * step, (col_of(z) + 1) as f32 * step - gap);
            div()
                .absolute()
                .top_0()
                .left(px(x0))
                .w(px((x1 - x0).max(cell)))
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(div().h(px(5.)).border_1().border_t_0().rounded_b(px(4.)).border_color(theme::white_a(0.3)))
                .child(label.text_size(px(12.)).text_color(theme::text_muted()).whitespace_nowrap())
        };
        if let (Some((a, z)), true) = (st.streak_range, st.streak > 2) {
            notes = notes.child(bracket(a, z, line(vec![bold(tr!("{n}-day streak", n = st.streak))])));
        }
        if let Some(b) = st.longest_break.filter(|b| b.days >= 7) {
            let (a, z) = (b.last + Duration::days(1), b.back - Duration::days(1));
            notes = notes.child(bracket(a, z, line(vec![bold(tr!("{n} days off", n = b.days))])));
        }
        let scale = div()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(3.))
            .text_size(px(11.))
            .text_color(theme::text_dim())
            .child(tr!("Less"))
            .children((0..10).map(|i| div().w(px(14.)).h(px(8.)).rounded(px(2.)).bg(theme::chart_ramp(i as f32 / 9.))))
            .child(tr!("More"));
        // Facts under it.
        let played = st.days_played.max(1);
        let long_days = st.days.iter().filter(|(_, s)| *s >= 4. * 3600.).count();
        let best_month = st.buckets.iter().enumerate().max_by(|a, b| a.1.secs.total_cmp(&b.1.secs));
        let fact = |big: String, small: String| {
            div()
                .flex()
                .flex_col()
                .child(div().font_family(theme::font_heading()).font_weight(theme::W_BOLD).text_size(px(24.)).child(big))
                .child(div().text_size(px(12.5)).text_color(theme::text_muted()).child(small))
        };
        let mut facts = div().grid().grid_cols(4).gap(px(16.)).mt(px(22.)).pt(px(18.)).border_t_1().border_color(theme::white_a(0.08));
        facts = facts.child(fact(fmt_secs(st.total_secs / played as f64), tr!("on a typical day you play").into()));
        facts = facts.child(fact((st.days_elapsed.saturating_sub(st.days_played)).to_string(), tr!("days without a game").into()));
        if let Some((_, b)) = best_month {
            let (days, _) = st.days_between(b.start, b.end);
            facts = facts.child(fact(
                month_name(b.start.month()).to_string(),
                tr!("{days} of {total} days played", days, total = (b.end - b.start).num_days()),
            ));
        }
        facts = facts.child(fact(format!("{:.0}%", long_days as f64 / played as f64 * 100.), tr!("of play days lasted 4h+").into()));
        panel()
            .child(panel_head(
                tr!("Every day of {year}", year),
                line(vec![bold(st.days_played.to_string()), div().child(tr!("days played \u{b7} darker means shorter"))])
                    .text_size(px(12.5))
                    .text_color(theme::text_muted()),
            ))
            .child(months)
            .child(div().flex().child(wdays).child(grid))
            .child(notes)
            .child(scale.mt(px(-6.)))
            .child(facts)
    }

    fn render_genres(&self, st: &Stats, cx: &mut Context<Self>) -> Div {
        let d = self.derived.clone();
        let gmax = d.genres.first().map_or(1., |g| g.1.max(1.));
        let genres = div().flex().flex_col().gap(px(11.)).children(d.genres.iter().enumerate().map(|(i, (name, secs))| {
            div()
                .id(("genre", i))
                .flex()
                .items_center()
                .gap(px(12.))
                .text_size(px(13.))
                .on_hover(hover(Hover::Genre(i), cx))
                .child(div().w(px(150.)).flex_none().truncate().child(name.clone()))
                .child(div().flex_1().h(px(8.)).rounded_full().bg(theme::white_a(0.07)).child(
                    div().h_full().w(relative((*secs / gmax) as f32)).rounded_full().bg(linear_gradient(
                        90.,
                        linear_color_stop(theme::chart_ramp(0.55), 0.),
                        linear_color_stop(theme::chart_ramp(1.), 1.),
                    )),
                ))
                .child(
                    div()
                        .w(px(62.))
                        .flex()
                        .justify_end()
                        .text_size(px(12.5))
                        .text_color(theme::text_muted())
                        .child(tr!("{h}h", h = (secs / 3600.).round())),
                )
        }));
        let total: f64 = d.sources.iter().map(|s| s.1).sum::<f64>().max(1.);
        let alphas = [0.95, 0.6, 0.34, 0.2, 0.12, 0.08];
        let accent: Hsla = theme::accent().into();
        let bar = div().flex().gap(px(2.)).h(px(10.)).my(px(10.)).children(d.sources.iter().enumerate().map(|(i, (_, secs))| {
            div()
                .id(("source", i))
                .h_full()
                .flex_grow(1.)
                .flex_basis(relative((*secs / total) as f32))
                .rounded(px(3.))
                .bg(Hsla { a: alphas[i.min(5)], ..accent })
                .on_hover(hover(Hover::Source(i), cx))
        }));
        let labels = div().flex().flex_wrap().gap_x(px(16.)).gap_y(px(6.)).text_size(px(12.)).text_color(theme::text_muted()).children(
            d.sources.iter().enumerate().map(|(i, (name, secs))| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().size(px(8.)).rounded_full().bg(Hsla { a: alphas[i.min(5)], ..accent }))
                    .child(name.to_string())
                    .child(div().text_color(theme::text_primary()).child(format!("{:.0}%", secs / total * 100.)))
            }),
        );
        let sources = div()
            .mt(px(20.))
            .pt(px(16.))
            .border_t_1()
            .border_color(theme::white_a(0.08))
            .child(div().text_size(px(13.)).text_color(theme::text_muted()).child(tr!("Where you play from")))
            .child(bar)
            .child(labels);
        let _ = st;
        panel()
            .flex_1()
            .child(panel_head(
                if d.genres.is_empty() { tr!("Where you play from") } else { tr!("Genres") },
                hint(if st.period.span == Span::Year { tr!("of the year") } else { tr!("of this period") }),
            ))
            .when(!d.genres.is_empty(), |p| p.child(genres))
            .child(sources)
    }
}

/// Panels side by side in 12-column spans (`wide`), else stacked.
pub(super) fn row(wide: bool, items: Vec<(Div, usize)>) -> AnyElement {
    if !wide {
        return div().flex().flex_col().gap(px(GAP)).children(items.into_iter().map(|(d, _)| d)).into_any_element();
    }
    div()
        .flex()
        .items_stretch()
        .gap(px(GAP))
        .children(items.into_iter().map(|(d, n)| {
            div().flex().flex_col().flex_grow(1.).flex_shrink(1.).flex_basis(relative(n as f32 / 12.)).min_w_0().child(d.flex_1())
        }))
        .into_any_element()
}
