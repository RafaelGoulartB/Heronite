//! Statistics in console mode: the same numbers as the desktop page, laid
//! out for a TV — LB / RB pick the span, ◀ ▶ the period, ↑ ↓ (or the right
//! stick) scroll, Y shows every game.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::stats::{fmt_hours_short, fmt_secs, Span, Stats, TOP_N};

use super::{axis_label, compare_line, nice_axis, series_color, StatsPage, DAY_PARTS, WEEKDAYS};
use crate::console::kit::{self, u};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::icons::Icon;
use crate::theme;

const SPANS: [Span; 4] = [Span::Week, Span::Month, Span::Year, Span::All];

fn tile(label: &str, value: String, sub: String, sub_color: Option<Rgba>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(u(6.))
        .flex_1()
        .min_w_0()
        .p(u(20.))
        .rounded(u(18.))
        .bg(theme::black_a(0.3))
        .border_1()
        .border_color(theme::white_a(0.08))
        .child(div().text_size(u(14.)).text_color(theme::text_muted()).child(label.to_string()))
        .child(kit::title(value, 34.))
        .child(div().text_size(u(14.)).truncate().text_color(sub_color.unwrap_or(theme::text_dim())).child(sub))
}

fn panel(title: &str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(u(14.))
        .p(u(22.))
        .rounded(u(18.))
        .bg(theme::black_a(0.3))
        .border_1()
        .border_color(theme::white_a(0.08))
        .child(div().text_size(u(18.)).font_weight(theme::W_BOLD).child(title.to_string()))
}

impl StatsPage {
    fn console_chart(&self, st: &Stats) -> Div {
        let max = st.buckets.iter().map(|b| b.secs).fold(0., f64::max);
        let (step, top) = nice_axis(max / 3600.);
        let top_secs = top * 3600.;
        let h = 190.;
        let lines = (top / step).round() as usize;
        let today = chrono::Local::now().date_naive();
        let n = st.buckets.len();
        let bars = st.buckets.iter().map(|b| {
            let mut bar = div().w_full().max_w(u(if n <= 12 { 40. } else { 22. })).flex().flex_col().rounded_t(u(5.)).overflow_hidden();
            let other = (b.secs - b.top.iter().sum::<f64>()).max(0.);
            for (secs, color) in
                std::iter::once((other, theme::chart_other())).chain((0..TOP_N).rev().map(|r| (b.top[r], series_color(Some(r)))))
            {
                let px_h = (secs / top_secs) as f32 * h;
                if px_h >= 0.5 {
                    bar = bar.child(div().w_full().h(u(px_h)).bg(color));
                }
            }
            div().flex_1().min_w_0().h_full().flex().flex_col().justify_end().items_center().px(u(if n > 20 { 2. } else { 6. })).child(bar)
        });
        let labels = st.buckets.iter().map(|b| {
            let current = b.start <= today && today < b.end;
            let text = match st.period.span {
                Span::Week => super::WEEKDAYS[chrono::Datelike::weekday(&b.start).num_days_from_monday() as usize].to_string(),
                Span::Month => chrono::Datelike::day(&b.start).to_string(),
                _ if (b.end - b.start).num_days() > 31 => chrono::Datelike::year(&b.start).to_string(),
                _ => heronite_data::stats::month_short(chrono::Datelike::month(&b.start)).to_string(),
            };
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .justify_center()
                .text_size(u(12.))
                .text_color(if current { theme::accent() } else { theme::text_dim() })
                .child(text)
        });
        let grid = (0..=lines).map(|i| {
            div().absolute().left_0().right_0().bottom(u(h * i as f32 / lines as f32)).h(px(1.)).bg(theme::white_a(if i == 0 {
                0.14
            } else {
                0.06
            }))
        });
        let axis = (0..=lines).map(|i| {
            div()
                .absolute()
                .right(u(8.))
                .bottom(u(h * i as f32 / lines as f32 - 8.))
                .text_size(u(12.))
                .text_color(theme::text_dim())
                .child(axis_label(step * i as f64))
        });
        let legend = div().flex().flex_wrap().gap(u(18.)).pl(u(44.)).children(st.series.iter().enumerate().map(|(r, &g)| {
            div()
                .flex()
                .items_center()
                .gap(u(8.))
                .text_size(u(14.))
                .text_color(theme::text_muted())
                .child(div().size(u(10.)).rounded_full().bg(series_color(Some(r))))
                .child(self.game_title(g))
        }));
        panel("Activity")
            .child(div().flex().child(div().relative().flex_none().w(u(44.)).h(u(h)).children(axis)).child(
                div().relative().flex_1().h(u(h)).children(grid).child(div().absolute().inset_0().flex().items_end().children(bars)),
            ))
            .child(div().flex().pl(u(44.)).children(labels))
            .child(legend)
    }

    fn console_games(&mut self, st: &Stats, window: &Window) -> Div {
        let scale = window.scale_factor();
        let max = st.games.first().map_or(1., |g| g.secs.max(1.));
        let total = st.total_secs.max(1.);
        let limit = if self.show_all { st.games.len() } else { 6 };
        let s = kit::scale();
        let mut p = panel("Most played");
        for g in st.games.iter().take(limit) {
            let Some(key) = self.game_key(g.game) else { continue };
            let cover = self.chains.get(&key).and_then(|c| self.covers.get_chain(&key, c, 40. * s, 54. * s, scale));
            p = p.child(
                div()
                    .flex()
                    .items_center()
                    .gap(u(16.))
                    .child(crate::services::widgets::cover_box(cover, f32::from(u(40.)), f32::from(u(54.))))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(u(6.))
                            .child(div().text_size(u(17.)).truncate().child(self.game_title(g.game)))
                            .child(div().h(u(6.)).w_full().rounded_full().bg(theme::white_a(0.07)).child(
                                div().h_full().w(relative((g.secs / max) as f32)).rounded_full().bg(series_color(st.series_of(g.game))),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_end()
                            .min_w(u(90.))
                            .child(div().text_size(u(17.)).font_weight(theme::W_BOLD).child(fmt_secs(g.secs)))
                            .child(div().text_size(u(13.)).text_color(theme::text_dim()).child(format!("{:.0}%", g.secs / total * 100.))),
                    ),
            );
        }
        if st.games.len() > limit {
            p = p.child(
                div()
                    .flex()
                    .items_center()
                    .gap(u(8.))
                    .text_size(u(14.))
                    .text_color(theme::text_muted())
                    .child(kit::glyph(Btn::Y))
                    .child(format!("Show all {}", st.games.len())),
            );
        }
        p
    }

    fn console_habits(&self, st: &Stats) -> Div {
        let max_cell = st.heat.iter().flatten().copied().fold(0., f64::max).max(1.);
        let in_sessions: f64 = st.heat.iter().flatten().sum();
        let accent: Hsla = theme::accent().into();
        let mut heat = div().flex().flex_col().gap(u(4.));
        for (d, hours) in st.heat.iter().enumerate() {
            heat = heat.child(
                div()
                    .flex()
                    .items_center()
                    .gap(u(4.))
                    .child(div().flex_none().w(u(40.)).text_size(u(13.)).text_color(theme::text_dim()).child(WEEKDAYS[d]))
                    .children(hours.iter().map(|v| {
                        let t = (*v / max_cell) as f32;
                        div().flex_1().h(u(20.)).rounded(u(4.)).bg(if *v <= 0. {
                            theme::white_a(0.05)
                        } else {
                            Hsla { a: 0.12 + 0.88 * t.powf(0.7), ..accent }
                        })
                    })),
            );
        }
        let parts = div().flex().gap(u(16.)).children(DAY_PARTS.iter().enumerate().map(|(i, (name, range))| {
            let share = if in_sessions > 0. { st.day_parts[i] / in_sessions } else { 0. };
            div()
                .flex()
                .flex_col()
                .flex_1()
                .gap(u(4.))
                .child(div().text_size(u(14.)).text_color(theme::text_muted()).child(format!("{name} {range}")))
                .child(kit::title(format!("{:.0}%", share * 100.), 24.))
        }));
        panel("When you play").child(heat).child(parts)
    }

    pub(super) fn render_console(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        self.covers.drop_evicted(window);
        let span_ix = SPANS.iter().position(|s| *s == self.period.span).unwrap_or(1);
        let tabs: Vec<SharedString> = ["Week", "Month", "Year", "All time"].into_iter().map(Into::into).collect();
        let label = match self.period.span {
            Span::All => "All time".to_string(),
            _ => self.period.label(),
        };
        let top = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(u(36.))
            .h(u(76.))
            .px(u(46.))
            .child(kit::wordmark("STATISTICS"))
            .child(kit::tabs(&tabs, span_ix))
            .child(div().flex_1())
            .child(kit::choice(label, false, self.can_prev(), self.can_next()))
            .child(kit::clock());
        let body: AnyElement = match self.stats.clone() {
            None => kit::empty(Icon::BarChart, "Reading your play history\u{2026}", "").into_any_element(),
            Some(st) if st.total_secs <= 0. => kit::empty(
                Icon::AccessTime,
                format!("Nothing played in {}", self.period.label()),
                "Pick another period with \u{25c0} \u{25b6}.",
            )
            .into_any_element(),
            Some(st) => {
                let all = st.period.span == Span::All;
                let (cmp, cmp_color) = compare_line(&st);
                let kpis = div()
                    .flex()
                    .gap(u(16.))
                    .child(tile("Time played", if all { fmt_hours_short(st.total_secs) } else { fmt_secs(st.total_secs) }, cmp, cmp_color))
                    .child(tile(
                        "Days played",
                        if all { st.days_played.to_string() } else { format!("{} / {}", st.days_played, st.days_elapsed) },
                        if st.streak > 1 { format!("Longest streak: {} days", st.streak) } else { "No streak yet".into() },
                        None,
                    ))
                    .child(tile("Games", st.games.len().to_string(), format!("{} new", st.new_games()), None))
                    .child(tile(
                        "Typical session",
                        if st.sessions > 0 { fmt_secs(st.avg_session_secs()) } else { "\u{2014}".into() },
                        format!("{} sessions", heronite_data::stats::group_thousands(st.sessions as u64)),
                        None,
                    ));
                let chart = self.console_chart(&st);
                let games = self.console_games(&st, window);
                let habits = self.console_habits(&st);
                div()
                    .id("console-stats")
                    .flex()
                    .flex_col()
                    .gap(u(16.))
                    .px(u(46.))
                    .pb(u(24.))
                    .overflow_y_scroll()
                    .track_scroll(&self.console_scroll)
                    .child(kpis)
                    .child(chart)
                    .child(div().flex().items_start().gap(u(16.)).child(games.flex_1().min_w_0()).child(habits.flex_1().min_w_0()))
                    .into_any_element()
            }
        };
        div().flex().flex_col().size_full().child(top).child(div().flex_1().min_h_0().flex().flex_col().child(body)).into_any_element()
    }
}

impl PadTarget for StatsPage {
    fn on_pad(&mut self, pad: Pad, _: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.review.is_some() {
            match pad {
                Pad::Left => self.review_key("left", cx),
                Pad::Right | Pad::A => self.review_key("right", cx),
                Pad::B => self.review_key("escape", cx),
                _ => false,
            };
            return true;
        }
        if pad == Pad::X {
            let year = self.review_year();
            self.open_review(year, cx);
            return true;
        }
        let span_ix = SPANS.iter().position(|s| *s == self.period.span).unwrap_or(1);
        match pad {
            Pad::LB => self.set_span(SPANS[(span_ix + SPANS.len() - 1) % SPANS.len()], cx),
            Pad::RB => self.set_span(SPANS[(span_ix + 1) % SPANS.len()], cx),
            Pad::Left if self.can_prev() => {
                if let Some(p) = self.period.prev() {
                    self.go(p, cx);
                }
            }
            Pad::Right if self.can_next() => {
                if let Some(p) = self.period.next() {
                    self.go(p, cx);
                }
            }
            Pad::Up | Pad::Down | Pad::ScrollUp | Pad::ScrollDown => {
                let h = &self.console_scroll;
                let off = h.offset();
                let step = u(if matches!(pad, Pad::Up | Pad::Down) { 140. } else { 220. });
                let y = if matches!(pad, Pad::Down | Pad::ScrollDown) { off.y - step } else { off.y + step };
                h.set_offset(point(off.x, y.clamp(-h.max_offset().y, px(0.))));
            }
            Pad::Y => self.show_all = !self.show_all,
            Pad::Left | Pad::Right => {}
            _ => return false,
        }
        cx.notify();
        true
    }

    fn hints(&self, _: &App) -> Vec<Hint> {
        if self.review.is_some() {
            return vec![hint(Btn::LeftRight, "Story"), hint(Btn::B, "Close")];
        }
        vec![
            hint(Btn::X, "Year in review"),
            hint(Btn::Bumpers, "Span"),
            hint(Btn::LeftRight, "Period"),
            hint(Btn::Y, if self.show_all { "Top games" } else { "All games" }),
        ]
    }
}
