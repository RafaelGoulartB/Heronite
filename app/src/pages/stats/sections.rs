//! Panels below the hero: the day-by-day chart (week and month), the top
//! games, when you play, the records, every day of the year and the genres.

use chrono::{Datelike, Duration, NaiveDate};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::stats::{fmt_secs, month_name, month_short, Span, Stats, TOP_N};

use super::desktop::{hint, panel, panel_head, GAP, R_PANEL};
use super::tip::hover;
use super::{
    axis_label, day_label, nice_axis, plural, series_color, short_date, today, Hover, StatsPage, DAY_PARTS, WEEKDAYS, WEEKDAYS_LONG,
};
use crate::icons::Icon;
use crate::pages::PageEvent;
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

impl StatsPage {
    pub(super) fn render_sections(
        &mut self,
        st: &Stats,
        col_w: f32,
        wide: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if matches!(st.period.span, Span::Year | Span::All) && st.buckets.len() > 1 {
            out.push(self.render_months(st, window, cx));
        } else {
            out.push(self.render_activity(st, cx));
        }
        out.push(self.render_shelf(st, col_w, window, cx));
        let habits = self.render_habits(st, cx);
        let records = self.render_records(st, if wide { span(col_w, 8) } else { col_w }, window);
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
                Span::Week => format!("{} {}", WEEKDAYS[i % 7], b.start.day()),
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
        let busiest = st.busiest.map(|(d, s)| format!("Busiest day: {} \u{b7} {}", day_label(d), fmt_secs(s)));
        panel()
            .child(panel_head("Day by day", hint(busiest.unwrap_or_default())))
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
        let year_word = match st.period.span {
            Span::Week => "week",
            Span::Month => "month",
            Span::Year => "year",
            Span::All => "total",
        };
        let mut covers = Vec::new();
        for (rank, g) in st.games.iter().take(cols.max(SHELF).min(if cols == 4 { 4 } else { SHELF })).enumerate() {
            let src = self.cover(g.game, cw, ch, window);
            let key = self.game_key(g.game).map(|k| k.to_string()).unwrap_or_default();
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
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(PageEvent::ShowGame(key.clone()))))
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
                                        .font_family(theme::FONT_HEADING)
                                        .font_weight(theme::W_BOLD)
                                        .text_size(px(11.))
                                        .child("New"),
                                )
                            })
                            .child(
                                div()
                                    .absolute()
                                    .left(px(12.))
                                    .bottom(px(4.))
                                    .font_family(theme::FONT_HEADING)
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
                            .font_family(theme::FONT_HEADING)
                            .font_weight(theme::W_BOLD)
                            .text_size(px(20.))
                            .child(fmt_secs(g.secs)),
                    )
                    .child(div().mt(px(1.)).text_size(px(12.)).text_color(theme::text_muted()).truncate().child(format!(
                        "{} \u{b7} {:.0}% of {year_word}",
                        plural(g.sessions, "session", "sessions"),
                        g.secs / st.total_secs.max(1.) * 100.
                    ))),
            );
        }
        let new = st.new_games();
        let beaten = self.derived.beaten.len();
        let mut facts = Vec::new();
        if new > 0 {
            facts.push(format!("{new} new"));
        }
        if beaten > 0 {
            facts.push(format!("{beaten} beaten"));
        }
        let toggle = (st.games.len() > covers.len()).then(|| {
            let label = if self.show_all { "Show less".to_string() } else { format!("Show all {}", st.games.len()) };
            div()
                .id("stats-show-all")
                .px(px(12.))
                .py(px(5.))
                .rounded_full()
                .bg(Hsla { a: 0.1, ..Hsla::from(theme::accent()) })
                .hover(|s| s.bg(Hsla { a: 0.2, ..Hsla::from(theme::accent()) }))
                .cursor_pointer()
                .font_family(theme::FONT_HEADING)
                .font_weight(theme::W_BOLD)
                .text_size(px(12.5))
                .text_color(theme::accent())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.show_all = !this.show_all;
                    cx.notify();
                }))
                .child(label)
        });
        let head = div().flex().items_center().gap(px(14.)).child(hint(facts.join(" \u{b7} "))).children(toggle);
        let mut p = panel().child(panel_head("Top games", head)).child(div().grid().grid_cols(cols as u16).gap(px(16.)).children(covers));
        if self.show_all {
            p = p.child(self.render_list(st, cols, window, cx));
        }
        p.into_any_element()
    }

    /// Every game of the period after the shelf ("Show all").
    fn render_list(&mut self, st: &Stats, skip: usize, window: &Window, cx: &mut Context<Self>) -> Div {
        let max = st.games.first().map_or(1., |g| g.secs.max(1.));
        let mut list = div().flex().flex_col().mt(px(18.)).pt(px(8.)).border_t_1().border_color(theme::white_a(0.08));
        for (rank, g) in st.games.iter().enumerate().skip(skip) {
            let cover = self.cover(g.game, 30., 40., window);
            let key = self.game_key(g.game).map(|k| k.to_string()).unwrap_or_default();
            let meta = match g.last {
                Some(last) => format!("{} \u{b7} last {}", plural(g.sessions, "session", "sessions"), super::when_label(last)),
                None => "No recorded sessions".into(),
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
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(PageEvent::ShowGame(key.clone()))))
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
                                    .when(g.new, |d| d.child(super::super::status_pill("New", theme::accent()))),
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
                            .font_family(theme::FONT_HEADING)
                            .font_weight(theme::W_BOLD)
                            .text_size(px(14.))
                            .child(fmt_secs(g.secs)),
                    ),
            );
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
        let mut heat = div().flex().flex_col().gap(px(2.));
        for (d, hours) in st.heat.iter().enumerate() {
            let mut line = div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(div().flex_none().w(px(30.)).text_size(px(11.)).text_color(theme::text_dim()).child(WEEKDAYS[d]));
            for (h, v) in hours.iter().enumerate() {
                let on = self.hover == Some(Hover::Heat(d, h));
                let color: Hsla =
                    if *v <= 0. { theme::chart_empty() } else { theme::chart_ramp(((*v / max_cell) as f32).powf(0.7)).into() };
                line = line.child(
                    div()
                        .id(("heat", d * 24 + h))
                        .flex_1()
                        .h(px(21.))
                        .rounded(px(4.))
                        .bg(color)
                        .when(on, |c| c.border_2().border_color(gpui_kit::white()))
                        .on_hover(hover(Hover::Heat(d, h), cx)),
                );
            }
            heat = heat.child(line);
        }
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
            .child("Less")
            .children((0..10).map(|i| div().w(px(14.)).h(px(8.)).rounded(px(2.)).bg(theme::chart_ramp(i as f32 / 9.))))
            .child("More");
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
                        .child(DAY_PARTS[i].0),
                )
                .child(
                    div()
                        .font_family(theme::FONT_HEADING)
                        .font_weight(theme::W_BOLD)
                        .text_size(px(22.))
                        .text_color(if i == best_part { theme::accent() } else { theme::text_primary() })
                        .child(format!("{share:.0}%")),
                )
                .child(div().text_size(px(11.)).text_color(theme::text_dim()).child(DAY_PARTS[i].1))
        }));
        let weekend = st.weekend_secs / total.max(1.);
        let wk = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .mt(px(14.))
            .text_size(px(12.5))
            .text_color(theme::text_muted())
            .child("Weekends")
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
            .child(line(vec![bold(format!("{:.0}%", weekend * 100.)), div().child("of your time")]));
        let lead = line(vec![
            div().child("You play most on"),
            bold(format!("{}s,", WEEKDAYS_LONG[best_day])),
            div().child("usually in the"),
            bold(format!("{}.", DAY_PARTS[best_part].0.to_lowercase())),
        ]);
        panel()
            .flex_1()
            .child(panel_head("When you play", div()))
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
                        .font_family(theme::FONT_HEADING)
                        .font_weight(theme::W_BOLD)
                        .text_size(px(11.5))
                        .text_color(theme::text_muted())
                        .child(label.to_uppercase()),
                )
                .child(
                    div()
                        .mt_auto()
                        .font_family(theme::FONT_HEADING)
                        .font_weight(theme::W_BOLD)
                        .text_size(px(46.))
                        .line_height(px(48.))
                        .child(value),
                )
                .child(detail.mt(px(8.)).text_size(px(13.)).text_color(theme::white_a(0.82)))
                .children(extra),
        )
    }

    fn render_records(&mut self, st: &Stats, w: f32, window: &Window) -> Div {
        let card_w = (w - GAP) / 2.;
        let mut cards: Vec<Div> = Vec::new();
        if let Some(l) = st.longest {
            let detail = line(vec![bold(self.game_title(l.game)), div().child(format!("\u{b7} {}", day_label(l.start.date())))]);
            cards.push(self.record(Some(l.game), card_w, "Longest session", fmt_secs(l.secs), detail, None, window));
        }
        if let Some((d, secs)) = st.busiest {
            let same = st.longest.is_some_and(|l| l.start.date() == d);
            let detail = line(vec![
                bold(day_label(d)),
                div().child(if same { "\u{b7} same day as your longest session" } else { "\u{b7} your most intense day" }),
            ]);
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
                    .child(format!("{} 1", month_short(d.month())))
                    .child(short_date(next - Duration::days(1))),
            );
            cards.push(self.record(None, card_w, "Busiest day", fmt_secs(secs), detail, None, window).child(mini));
        }
        if let (Some((a, z)), true) = (st.streak_range, st.streak > 1) {
            let detail = line(vec![bold(format!("{} \u{2013} {}", short_date(a), short_date(z))), div().child("\u{b7} every single day")]);
            let dots = div().flex().flex_wrap().gap(px(2.)).mt(px(10.)).children(
                (0..st.streak.min(60)).map(|_| div().size(px(6.)).rounded(px(2.)).bg(Hsla { a: 0.85, ..Hsla::from(theme::accent()) })),
            );
            cards.push(self.record(st.streak_game, card_w, "Longest streak", format!("{} days", st.streak), detail, Some(dots), window));
        }
        match st.longest_break.filter(|b| b.days >= 3) {
            Some(b) => {
                let off = (b.last + Duration::days(1), b.back - Duration::days(1));
                let back_game = st
                    .buckets
                    .iter()
                    .position(|x| x.start <= b.back && b.back < x.end)
                    .and_then(|i| st.bucket_games.get(i)?.first().map(|g| g.0));
                let detail = line(vec![
                    bold(format!("{} \u{2013} {}", short_date(off.0), short_date(off.1))),
                    div().child(match back_game {
                        Some(g) => format!("\u{b7} back on {} with {}", short_date(b.back), self.game_title(g)),
                        None => format!("\u{b7} back on {}", short_date(b.back)),
                    }),
                ]);
                cards.push(self.record(back_game, card_w, "Longest break", plural(b.days, "day", "days"), detail, None, window));
            }
            None => {
                let new = st.new_games();
                let detail = line(vec![bold(plural(new, "game", "games")), div().child("played for the first time")]);
                cards.push(self.record(
                    st.games.iter().find(|g| g.new).map(|g| g.game),
                    card_w,
                    "Games played",
                    st.games.len().to_string(),
                    detail,
                    None,
                    window,
                ));
            }
        }
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
        let mut grid = div().flex().gap(px(gap));
        for wk in 0..weeks {
            let mut col = div().flex().flex_col().gap(px(gap));
            for wd in 0..7 {
                let d = first_monday + Duration::days((wk * 7 + wd) as i64);
                let inside = d.year() == year;
                let v = st.day_secs(d);
                let on = self.hover == Some(Hover::Day(d));
                let color: Hsla = if !inside {
                    theme::black_a(0.)
                } else if d > today {
                    theme::white_a(0.022)
                } else if v <= 0. {
                    theme::chart_empty()
                } else {
                    theme::chart_ramp(((v / max) as f32).powf(0.6)).into()
                };
                col = col.child(
                    div()
                        .id(SharedString::from(format!("cal-{d}")))
                        .size(px(cell))
                        .rounded(px(3.))
                        .bg(color)
                        .when(on, |c| c.border_2().border_color(gpui_kit::white()))
                        .when(inside && d <= today, |c| c.on_hover(hover(Hover::Day(d), cx))),
                );
            }
            grid = grid.child(col);
        }
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
                .children((i % 2 == 0).then(|| WEEKDAYS[i]))
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
            notes = notes.child(bracket(a, z, line(vec![bold(format!("{}-day streak", st.streak))])));
        }
        if let Some(b) = st.longest_break.filter(|b| b.days >= 7) {
            let (a, z) = (b.last + Duration::days(1), b.back - Duration::days(1));
            notes = notes.child(bracket(a, z, line(vec![bold(format!("{} days off", b.days))])));
        }
        let scale = div()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(3.))
            .text_size(px(11.))
            .text_color(theme::text_dim())
            .child("Less")
            .children((0..10).map(|i| div().w(px(14.)).h(px(8.)).rounded(px(2.)).bg(theme::chart_ramp(i as f32 / 9.))))
            .child("More");
        // Facts under it.
        let played = st.days_played.max(1);
        let long_days = st.days.iter().filter(|(_, s)| *s >= 4. * 3600.).count();
        let best_month = st.buckets.iter().enumerate().max_by(|a, b| a.1.secs.total_cmp(&b.1.secs));
        let fact = |big: String, small: String| {
            div()
                .flex()
                .flex_col()
                .child(div().font_family(theme::FONT_HEADING).font_weight(theme::W_BOLD).text_size(px(24.)).child(big))
                .child(div().text_size(px(12.5)).text_color(theme::text_muted()).child(small))
        };
        let mut facts = div().grid().grid_cols(4).gap(px(16.)).mt(px(22.)).pt(px(18.)).border_t_1().border_color(theme::white_a(0.08));
        facts = facts.child(fact(fmt_secs(st.total_secs / played as f64), "on a typical day you play".into()));
        facts = facts.child(fact((st.days_elapsed.saturating_sub(st.days_played)).to_string(), "days without a game".into()));
        if let Some((_, b)) = best_month {
            let (days, _) = st.days_between(b.start, b.end);
            facts = facts
                .child(fact(month_name(b.start.month()).to_string(), format!("{days} of {} days played", (b.end - b.start).num_days())));
        }
        facts = facts.child(fact(format!("{:.0}%", long_days as f64 / played as f64 * 100.), "of play days lasted 4h+".into()));
        panel()
            .child(panel_head(
                format!("Every day of {year}"),
                line(vec![bold(st.days_played.to_string()), div().child("days played \u{b7} darker means shorter")])
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
                        .child(format!("{}h", (secs / 3600.).round())),
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
            .child(div().text_size(px(13.)).text_color(theme::text_muted()).child("Where you play from"))
            .child(bar)
            .child(labels);
        let _ = st;
        panel()
            .flex_1()
            .child(panel_head(
                if d.genres.is_empty() { "Where you play from" } else { "Genres" },
                hint(if st.period.span == Span::Year { "of the year" } else { "of this period" }),
            ))
            .when(!d.genres.is_empty(), |p| p.child(genres))
            .child(sources)
    }
}

/// Panels side by side in 12-column spans (`wide`), else stacked.
fn row(wide: bool, items: Vec<(Div, usize)>) -> AnyElement {
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
