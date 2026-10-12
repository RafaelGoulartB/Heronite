//! "Month by month": a horizontal strip with the year as a river (one band
//! per top game, see [`super::river`]) and one card per month (its game of
//! the month, a highlight, the top three, every day and screenshots), with
//! a ruler of months under it. All time with a long history gets one card
//! per year instead.

use chrono::{Datelike, Duration, NaiveDate};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::stats::{fmt_secs, month_name, month_short, Bucket, Span, Stats, TOP_N};
use heronite_i18n::{tr, trn};

use super::desktop::{eyebrow, hint, panel, panel_head};
use super::review::marked;
use super::river::{self, River, ORDER};
use super::tip::hover;
use super::{day_label, hours_text, in_text, series_color, short_date, today, Hover, StatsPage};
use crate::console::motion::Goal;
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// Card height.
const MH: f32 = 664.;
const MONTH_W: f32 = 470.;
const FUTURE_W: f32 = 330.;
const RIVER_W: f32 = 640.;
/// River plot inside its card.
const PLOT_W: f32 = RIVER_W - 52.;
const PLOT_H: f32 = 300.;

fn card_fill() -> Rgba {
    theme::river_gap()
}

/// "February" for a month bucket, "2025" for a year bucket.
fn bucket_name(b: &Bucket) -> String {
    if (b.end - b.start).num_days() > 31 {
        b.start.year().to_string()
    } else {
        month_name(b.start.month()).to_string()
    }
}

fn bucket_short(b: &Bucket) -> String {
    if (b.end - b.start).num_days() > 31 {
        b.start.year().to_string()
    } else {
        month_short(b.start.month()).to_string()
    }
}

/// Gradient layer between `from` and `to` (fractions of the parent height).
fn band(from: f32, to: f32, a0: f32, a1: f32, color: Rgba) -> Div {
    let c: Hsla = color.into();
    div().absolute().left_0().right_0().top(relative(from)).h(relative(to - from)).bg(linear_gradient(
        180.,
        linear_color_stop(Hsla { a: a0, ..c }, 0.),
        linear_color_stop(Hsla { a: a1, ..c }, 1.),
    ))
}

/// Small dark pill over art.
fn art_pill() -> Div {
    div().flex().items_center().gap(px(8.)).rounded_full().bg(rgba(0x0a0e14b3)).border_1().border_color(theme::white_a(0.08))
}

impl StatsPage {
    pub(super) fn render_months(&mut self, st: &Stats, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let mut cards: Vec<AnyElement> = vec![self.river_card(st, window, cx)];
        for (i, b) in st.buckets.iter().enumerate() {
            cards.push(self.month_card(st, i, b, window, cx));
        }
        let active = self.active_card(cards.len());
        let track = div()
            .id("stats-strip")
            .relative()
            .flex()
            .gap(px(16.))
            .mx(px(-24.))
            .px(px(24.))
            .pt(px(2.))
            .pb(px(8.))
            .overflow_x_scroll()
            .restrict_scroll_to_axis()
            .track_scroll(self.strip.handle())
            .children(cards)
            .child(self.strip.driver());
        let years = st.buckets.first().is_some_and(|b| (b.end - b.start).num_days() > 31);
        panel()
            .pb_0()
            .child(panel_head(
                if years { tr!("Year by year") } else { tr!("Month by month") },
                hint(if years {
                    tr!("One card per year \u{b7} scroll sideways or pick one on the ruler")
                } else {
                    tr!("One card per month \u{b7} scroll sideways or pick one on the ruler")
                }),
            ))
            .child(track)
            .child(self.ruler(st, active, cx))
            .into_any_element()
    }

    /// Card nearest to the strip's scroll position.
    fn active_card(&self, n: usize) -> usize {
        let h = self.strip.handle();
        let x = -f32::from(h.offset().x);
        let Some(first) = h.bounds_for_item(0) else { return 0 };
        let max = f32::from(h.max_offset().x);
        if max > 0. && x >= max - 2. {
            return n - 1;
        }
        (0..n)
            .filter_map(|i| h.bounds_for_item(i).map(|b| (i, (f32::from(b.left() - first.left()) - x).abs())))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(0, |(i, _)| i)
    }

    fn ruler(&self, st: &Stats, active: usize, cx: &mut Context<Self>) -> Div {
        let n = st.buckets.len() + 1;
        let max = st.buckets.iter().map(|b| b.secs).fold(1., f64::max);
        let nav = |id: &'static str, ic: Icon, enabled: bool| {
            div()
                .id(id)
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(38.))
                .rounded_full()
                .bg(theme::white_a(0.07))
                .border_1()
                .border_color(theme::white_a(0.08))
                .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(theme::white_a(0.16))))
                .when(!enabled, |d| d.opacity(0.3))
                .child(icon(ic, px(16.), theme::text_muted()))
        };
        let mut prev = nav("strip-prev", Icon::ChevronLeft, active > 0);
        if active > 0 {
            prev = prev.on_click(cx.listener(move |this, _, _, cx| {
                this.strip.go(Goal::Start(active - 1));
                cx.notify();
            }));
        }
        let mut next = nav("strip-next", Icon::ChevronRight, active + 1 < n);
        if active + 1 < n {
            next = next.on_click(cx.listener(move |this, _, _, cx| {
                this.strip.go(Goal::Start(active + 1));
                cx.notify();
            }));
        }
        let tick = |i: usize, bar: Div, label: String| {
            let on = i == active;
            div()
                .id(("strip-tick", i))
                .flex()
                .flex_col()
                .items_center()
                .justify_end()
                .gap(px(5.))
                .w(px(62.))
                .h(px(68.))
                .pb(px(6.))
                .rounded(px(12.))
                .cursor_pointer()
                .hover(|s| s.bg(theme::white_a(0.06)))
                .when(on, |d| d.bg(theme::white_a(0.05)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.strip.go(Goal::Start(i));
                    cx.notify();
                }))
                .child(bar.when(on, |b| b.bg(theme::accent())))
                .child(
                    div()
                        .font_family(theme::FONT_HEADING)
                        .text_size(px(11.5))
                        .text_color(if on { theme::text_primary() } else { theme::text_dim() })
                        .when(on, |d| d.font_weight(theme::W_BOLD))
                        .child(label),
                )
        };
        let mut row = div()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .h(px(84.))
            .mx(px(-24.))
            .mt(px(6.))
            .border_t_1()
            .border_color(theme::white_a(0.08));
        row = row.child(prev.mr(px(8.)));
        let first = match st.period.span {
            Span::All => tr!("All").to_string(),
            _ => st.period.start.year().to_string(),
        };
        row = row.child(tick(0, div().size(px(20.)).rounded(px(7.)).bg(theme::white_a(0.16)), first));
        for (i, b) in st.buckets.iter().enumerate() {
            let h = if b.future { 3. } else { (b.secs / max * 34.).max(3.) as f32 };
            let bar = div().w(px(20.)).h(px(h)).rounded_t(px(4.)).rounded_b(px(2.)).bg(theme::white_a(if b.future { 0.08 } else { 0.2 }));
            row = row.child(tick(i + 1, bar, bucket_short(b)));
        }
        row.child(next.ml(px(8.)))
            .child(div().w(px(56.)).text_center().text_size(px(12.)).text_color(theme::text_dim()).child(format!("{} / {n}", active + 1)))
    }

    /// The period as a river, with its legend.
    fn river_card(&mut self, st: &Stats, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let n = st.buckets.len();
        let layers: Vec<(Option<usize>, Vec<f64>)> = ORDER
            .iter()
            .filter(|slot| slot.is_none_or(|s| s < st.series.len()))
            .map(|slot| {
                let v = st
                    .buckets
                    .iter()
                    .map(|b| match slot {
                        Some(s) => b.top[*s],
                        None => (b.secs - b.top.iter().sum::<f64>()).max(0.),
                    })
                    .collect();
                (*slot, v)
            })
            .collect();
        let max = st.buckets.iter().map(|b| b.secs).fold(1., f64::max);
        let y0 = PLOT_H / 2. - 6.;
        let rv = River::new(layers, (PLOT_H - 60.) / max as f32, y0);
        let bands = rv.bands(PLOT_W);
        let focus = match self.hover {
            Some(Hover::Slot(s)) => Some(if s < TOP_N { Some(s) } else { None }),
            _ => None,
        };
        let col = PLOT_W / n.max(1) as f32;
        let today = today();
        let first_future = st.buckets.iter().position(|b| b.future);
        let mut plot = div().relative().w(px(PLOT_W)).h(px(PLOT_H));
        for i in 1..n {
            plot = plot.child(div().absolute().top_0().bottom_0().left(px(i as f32 * col)).w(px(1.)).bg(theme::white_a(0.05)));
        }
        if let Some(f) = first_future {
            // Today: where in the current bucket it falls.
            let b = &st.buckets[f.saturating_sub(1)];
            let frac = ((today - b.start).num_days() as f32 + 0.5) / (b.end - b.start).num_days().max(1) as f32;
            let x = (f.saturating_sub(1) as f32 + frac.clamp(0., 1.)) * col;
            plot = plot
                .child(div().absolute().top_0().bottom_0().left(px(x)).right_0().bg(theme::black_a(0.2)))
                .child(div().absolute().top_0().bottom_0().left(px(x)).w(px(1.)).bg(theme::white_a(0.3)))
                .child(
                    div()
                        .absolute()
                        .top(px(6.))
                        .left(px(x + 8.))
                        .text_size(px(11.5))
                        .text_color(theme::text_dim())
                        .child(tr!("Today \u{b7} {date}", date = short_date(today))),
                )
                .when(PLOT_W - x > 70., |p| {
                    p.child(
                        div()
                            .absolute()
                            .top(px(y0 - 26.))
                            .left(px(x))
                            .right_0()
                            .flex()
                            .justify_center()
                            .text_size(px(12.))
                            .text_color(theme::text_dim())
                            .child(tr!("Not played yet")),
                    )
                });
        }
        plot = plot.child(
            canvas(|_, _, _| (), move |bounds, _, window, _| river::paint(&bands, bounds.origin, focus, window)).absolute().inset_0(),
        );
        // Months without play inside the elapsed part: a pause sign.
        for (i, b) in st.buckets.iter().enumerate() {
            if b.future || b.secs > 0. {
                continue;
            }
            let x = (i as f32 + 0.5) * col;
            plot = plot
                .child(
                    div()
                        .absolute()
                        .left(px(x - 9.))
                        .top(px(y0 - 12.))
                        .flex()
                        .gap(px(5.))
                        .child(div().w(px(6.)).h(px(24.)).rounded(px(2.)).bg(theme::white_a(0.55)))
                        .child(div().w(px(6.)).h(px(24.)).rounded(px(2.)).bg(theme::white_a(0.55))),
                )
                .child(div().absolute().left(px(x)).top(px(y0 + 18.)).w(px(1.)).h(px(PLOT_H - y0 - 66.)).bg(theme::white_a(0.3)))
                .child(
                    div()
                        .absolute()
                        .left(px(x - 70.))
                        .w(px(140.))
                        .top(px(PLOT_H - 46.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .text_size(px(12.))
                        .text_color(theme::text_muted())
                        .child(
                            div()
                                .font_weight(theme::W_BOLD)
                                .text_color(theme::text_primary())
                                .child(tr!("{name} \u{b7} 0 h", name = bucket_name(b))),
                        )
                        .child(tr!("a real break")),
                );
        }
        // Hover columns.
        for i in 0..n {
            let on = self.hover == Some(Hover::River(i));
            plot = plot.child(
                div()
                    .id(("river-col", i))
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(i as f32 * col))
                    .w(px(col))
                    .rounded(px(8.))
                    .when(on, |d| d.bg(theme::white_a(0.05)))
                    .on_hover(hover(Hover::River(i), cx)),
            );
        }
        let axis = div().flex().mt(px(6.)).children(st.buckets.iter().map(|b| {
            let now = b.start <= today && today < b.end;
            div()
                .flex_1()
                .text_center()
                .text_size(px(11.5))
                .text_color(if now {
                    theme::accent().into()
                } else if b.future {
                    theme::white_a(0.3)
                } else {
                    theme::text_muted().into()
                })
                .when(now, |d| d.font_weight(theme::W_BOLD))
                .child(bucket_short(b))
        }));

        // Legend: the top games and Other, two columns.
        let top_secs: f64 = st.series.iter().filter_map(|g| st.game(*g)).map(|g| g.secs).sum();
        let mut legend: Vec<(usize, Hsla, String, f64)> = st
            .series
            .iter()
            .enumerate()
            .map(|(slot, g)| (slot, series_color(Some(slot)), self.game_title(*g), st.game(*g).map_or(0., |s| s.secs)))
            .collect();
        legend.push((TOP_N, theme::chart_other(), tr!("Other games").into(), (st.total_secs - top_secs).max(0.)));
        let legend = div()
            .mt_auto()
            .pt(px(14.))
            .border_t_1()
            .border_color(theme::white_a(0.08))
            .grid()
            .grid_cols(2)
            .gap_x(px(18.))
            .gap_y(px(2.))
            .children(legend.into_iter().map(|(slot, c, name, secs)| {
                div()
                    .id(("river-legend", slot))
                    .flex()
                    .items_center()
                    .gap(px(9.))
                    .h(px(30.))
                    .px(px(8.))
                    .mx(px(-8.))
                    .rounded(px(9.))
                    .text_size(px(13.))
                    .min_w_0()
                    .hover(|s| s.bg(theme::white_a(0.06)))
                    .on_hover(hover(Hover::Slot(slot), cx))
                    .child(div().flex_none().size(px(8.)).rounded_full().bg(c))
                    .child(div().flex_1().min_w_0().truncate().child(name))
                    .child(
                        div()
                            .flex_none()
                            .font_family(theme::FONT_HEADING)
                            .font_weight(theme::W_BOLD)
                            .text_size(px(12.5))
                            .text_color(theme::text_muted())
                            .child(tr!("{h}h", h = (secs / 3600.).round())),
                    )
            }));

        let pause = st.buckets.iter().find(|b| !b.future && b.secs <= 0.).map(bucket_name);
        let (kick, title) = match st.period.span {
            Span::All => (tr!("All time"), tr!("Your history as a ~~river~~").to_string()),
            _ => (tr!("Year in review"), tr!("{period} as a ~~river~~", period = st.period.label())),
        };
        let years = st.buckets.first().is_some_and(|b| (b.end - b.start).num_days() > 31);
        let lead = div()
            .flex()
            .flex_wrap()
            .gap_x(px(4.))
            .mt(px(8.))
            .text_size(px(14.))
            .text_color(theme::white_a(0.82))
            .child(if years {
                tr!("Band thickness is the hours you played each year.")
            } else {
                tr!("Band thickness is the hours you played each month.")
            })
            .children(pause.map(|m| div().child(marked(tr!("**{name}** pinches shut: a real pause.", name = m)))));
        let top = st.games.first().map(|g| g.game);
        let bg = top.and_then(|g| self.art(g, RIVER_W, MH, window));
        let fill = card_fill();
        let mut card = div()
            .relative()
            .flex_none()
            .w(px(RIVER_W))
            .h(px(MH))
            .rounded(px(18.))
            .overflow_hidden()
            .bg(fill)
            .border_1()
            .border_color(theme::white_a(0.08));
        if let Some(src) = bg {
            card = card.child(img(src).absolute().inset_0().size_full().object_fit(ObjectFit::Cover).rounded(px(17.)));
        }
        card.child(band(0., 0.26, 0.55, 0.86, rgb(0x0e1219)))
            .child(band(0.26, 0.48, 0.86, 0.98, rgb(0x121721)))
            .child(div().absolute().left_0().right_0().top(relative(0.48)).bottom_0().bg(Hsla { a: 0.98, ..Hsla::from(fill) }))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .size_full()
                    .px(px(26.))
                    .pt(px(24.))
                    .pb(px(22.))
                    .child(eyebrow(kick, 0.7))
                    .child(
                        div()
                            .flex()
                            .mt(px(10.))
                            .font_family(theme::FONT_HEADING)
                            .font_weight(theme::W_BOLD)
                            .text_size(px(36.))
                            .line_height(px(40.))
                            .child(marked(title)),
                    )
                    .child(lead)
                    .child(div().mt(px(14.)).child(plot).child(axis))
                    .child(legend),
            )
            .into_any_element()
    }

    /// Highlight of bucket `i`: big text, title, detail.
    fn highlight(&self, st: &Stats, i: usize, b: &Bucket, best: Option<(NaiveDate, f64)>) -> (String, String, String) {
        let within = |d: NaiveDate| b.start <= d && d < b.end;
        let of = match st.period.span {
            Span::All => tr!("of all time").to_string(),
            _ => tr!("of {year}", year = st.period.start.year()),
        };
        let busiest = st.buckets.iter().enumerate().max_by(|a, c| a.1.secs.total_cmp(&c.1.secs)).map(|(j, _)| j);
        let top = st.bucket_games.get(i).and_then(|g| g.first().copied());
        if let Some((a, z)) = st.streak_range.filter(|(a, _)| within(*a) && st.streak > 2) {
            return (
                trn!(st.streak, "{n} day", "{n} days"),
                tr!("Longest streak {of}", of),
                tr!("{from} to {to}, not a single day missed", from = short_date(a), to = short_date(z)),
            );
        }
        if busiest == Some(i) {
            let detail = match (top, best) {
                (Some((g, s)), Some((d, ds))) => {
                    tr!(
                        "{pct}% of it was {game}, best day {time} on {date}",
                        pct = format!("{:.0}", s / b.secs * 100.),
                        game = self.game_title(g),
                        time = fmt_secs(ds),
                        date = short_date(d)
                    )
                }
                _ => String::new(),
            };
            let title = if (b.end - b.start).num_days() > 31 {
                tr!("Busiest year {of} \u{b7} {time}", of, time = hours_text(b.secs))
            } else {
                tr!("Busiest month {of} \u{b7} {time}", of, time = hours_text(b.secs))
            };
            return ("#1".into(), title, detail);
        }
        if let Some(l) = st.longest.filter(|l| within(l.start.date())) {
            return (
                fmt_secs(l.secs),
                tr!("Longest session {of}", of),
                tr!("{game} \u{b7} {day}", game = self.game_title(l.game), day = day_label(l.start.date())),
            );
        }
        let new = st.games.iter().filter(|g| g.new && g.first_played.is_some_and(within)).count();
        match best {
            Some((d, s)) => {
                let detail = if new > 0 {
                    trn!(new, "{day} \u{b7} {n} new game started", "{day} \u{b7} {n} new games started", day = day_label(d))
                } else {
                    day_label(d)
                };
                (fmt_secs(s), tr!("Best day").into(), detail)
            }
            None => (String::new(), String::new(), String::new()),
        }
    }

    fn month_card(&mut self, st: &Stats, i: usize, b: &Bucket, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let name = bucket_name(b);
        let num = if (b.end - b.start).num_days() > 31 { format!("{}", i + 1) } else { format!("{:02}", b.start.month()) };
        let head = |name: String| {
            div()
                .absolute()
                .left(px(20.))
                .top(px(18.))
                .flex()
                .items_center()
                .gap(px(12.))
                .child(
                    div()
                        .px(px(10.))
                        .py(px(4.))
                        .rounded_full()
                        .bg(rgba(0x0a0e14b3))
                        .border_1()
                        .border_color(theme::white_a(0.08))
                        .font_family(theme::FONT_HEADING)
                        .font_weight(theme::W_BOLD)
                        .text_size(px(12.))
                        .text_color(theme::text_muted())
                        .child(num.clone()),
                )
                .child(
                    div().font_family(theme::FONT_HEADING).font_weight(theme::W_BOLD).text_size(px(30.)).line_height(px(32.)).child(name),
                )
        };
        let shell = |w: f32| {
            div()
                .relative()
                .flex()
                .flex_col()
                .flex_none()
                .w(px(w))
                .h(px(MH))
                .rounded(px(18.))
                .overflow_hidden()
                .bg(card_fill())
                .border_1()
                .border_color(theme::white_a(0.08))
                .shadow(vec![BoxShadow {
                    color: theme::black_a(0.28),
                    offset: point(px(0.), px(10.)),
                    blur_radius: px(30.),
                    spread_radius: px(0.),
                    inset: false,
                }])
        };
        let today = today();
        if b.future {
            return shell(FUTURE_W)
                .opacity(0.9)
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .bg(linear_gradient(180., linear_color_stop(rgb(0x141922), 0.), linear_color_stop(rgb(0x10151c), 1.)))
                        .child(head(name.clone()))
                        .child(
                            div()
                                .absolute()
                                .left_0()
                                .right_0()
                                .top(relative(0.38))
                                .flex()
                                .justify_center()
                                .font_family(theme::FONT_HEADING)
                                .font_weight(theme::W_BOLD)
                                .text_size(px(170.))
                                .text_color(theme::white_a(0.05))
                                .child(num.clone()),
                        ),
                )
                .child(
                    div()
                        .px(px(22.))
                        .pb(px(26.))
                        .child(
                            div()
                                .font_family(theme::FONT_HEADING)
                                .font_weight(theme::W_BOLD)
                                .text_size(px(44.))
                                .text_color(theme::text_dim())
                                .child("0 h"),
                        )
                        .child(
                            div()
                                .mt(px(6.))
                                .text_size(px(12.5))
                                .text_color(theme::text_muted())
                                .child(tr!("Not played yet \u{b7} today is {date}", date = short_date(today))),
                        ),
                )
                .into_any_element();
        }
        let (days, best) = st.days_between(b.start, b.end.min(today + Duration::days(1)));
        let span_days = (b.end.min(today + Duration::days(1)) - b.start).num_days().max(0) as usize;
        let daily = self.daily_bars(st, b, best.map(|x| x.0), cx);
        let dlab = div()
            .flex()
            .justify_between()
            .mt(px(12.))
            .mb(px(6.))
            .font_family(theme::FONT_HEADING)
            .font_weight(theme::W_BOLD)
            .text_size(px(11.))
            .text_color(theme::text_muted())
            .child(tr!("DAY BY DAY"))
            .child(tr!("{days} OF {span_days} DAYS PLAYED", days, span_days));
        let hbox = |big: String, title: String, detail: String| {
            div()
                .flex()
                .items_center()
                .gap(px(14.))
                .px(px(14.))
                .py(px(10.))
                .rounded(px(16.))
                .bg(theme::white_a(0.05))
                .border_1()
                .border_color(Hsla { a: 0.26, ..Hsla::from(theme::accent()) })
                .child(
                    div()
                        .flex_none()
                        .min_w(px(76.))
                        .font_family(theme::FONT_HEADING)
                        .font_weight(theme::W_BOLD)
                        .text_size(px(24.))
                        .text_color(theme::accent())
                        .child(big),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(div().font_family(theme::FONT_HEADING).font_weight(theme::W_BOLD).text_size(px(13.)).truncate().child(title))
                        .child(div().text_size(px(12.)).text_color(theme::text_muted()).truncate().child(detail)),
                )
        };
        if b.secs <= 0. {
            let gap = st.longest_break.filter(|g| g.last < b.end && g.back > b.start);
            let (big, title, detail) = match gap {
                Some(g) => (
                    g.days.to_string(),
                    tr!("Days without a session").to_string(),
                    tr!(
                        "{from} to {to}, your longest break",
                        from = short_date(g.last + Duration::days(1)),
                        to = short_date(g.back - Duration::days(1))
                    ),
                ),
                None => ("0".into(), tr!("Days played").into(), tr!("Nothing played in {name}", name = name.clone())),
            };
            return shell(MONTH_W)
                .child(
                    div().relative().flex_1().bg(rgb(0x10151c)).child(head(name.clone())).child(
                        div()
                            .absolute()
                            .left(relative(0.5))
                            .top(relative(0.54))
                            .ml(px(-42.))
                            .mt(px(-42.))
                            .size(px(84.))
                            .rounded_full()
                            .border_2()
                            .border_color(theme::white_a(0.2))
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap(px(10.))
                            .child(div().w(px(10.)).h(px(32.)).rounded(px(4.)).bg(theme::white_a(0.55)))
                            .child(div().w(px(10.)).h(px(32.)).rounded(px(4.)).bg(theme::white_a(0.55))),
                    ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .px(px(22.))
                        .pb(px(20.))
                        .child(
                            div()
                                .font_family(theme::FONT_HEADING)
                                .font_weight(theme::W_BOLD)
                                .text_size(px(54.))
                                .line_height(px(56.))
                                .child("0 h"),
                        )
                        .child(
                            div()
                                .mt(px(6.))
                                .mb(px(12.))
                                .text_size(px(12.5))
                                .text_color(theme::text_muted())
                                .child(tr!("Time off. The controller stayed on the shelf.")),
                        )
                        .child(hbox(big, title, detail))
                        .child(dlab)
                        .child(daily),
                )
                .into_any_element();
        }

        let games = st.bucket_games.get(i).cloned().unwrap_or_default();
        let top = games.first().map(|g| g.0);
        let art = top.and_then(|g| self.art(g, MONTH_W, 300., window));
        let (big, title, detail) = self.highlight(st, i, b, best);
        let max = games.first().map_or(1., |g| g.1);
        let mut rows = div().flex().flex_col().gap(px(1.)).mt(px(10.));
        for (r, (g, secs)) in games.iter().take(3).enumerate() {
            let cover = self.cover(*g, 30., 40., window);
            let c = series_color(st.series_of(*g));
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .h(px(46.))
                    .child(
                        div()
                            .w(px(12.))
                            .font_family(theme::FONT_HEADING)
                            .font_weight(theme::W_BOLD)
                            .text_size(px(12.))
                            .text_color(theme::text_dim())
                            .child((r + 1).to_string()),
                    )
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
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(7.))
                                    .font_family(theme::FONT_HEADING)
                                    .font_weight(theme::W_BOLD)
                                    .text_size(px(13.))
                                    .child(div().flex_none().size(px(8.)).rounded_full().bg(c))
                                    .child(div().truncate().child(self.game_title(*g))),
                            )
                            .child(
                                div()
                                    .mt(px(6.))
                                    .h(px(5.))
                                    .rounded_full()
                                    .bg(theme::white_a(0.07))
                                    .child(div().h_full().w(relative((*secs / max) as f32)).rounded_full().bg(c)),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .min_w(px(70.))
                            .flex()
                            .justify_end()
                            .font_family(theme::FONT_HEADING)
                            .font_weight(theme::W_BOLD)
                            .text_size(px(14.))
                            .child(fmt_secs(*secs)),
                    ),
            );
        }
        let rest: f64 = games.iter().skip(3).map(|g| g.1).sum();
        if rest >= 60. {
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .h(px(26.))
                    .text_size(px(12.5))
                    .text_color(theme::text_muted())
                    .child(div().w(px(12.)))
                    .child(div().flex_none().size(px(8.)).rounded_full().bg(theme::chart_other()))
                    .child(div().flex_1().child(tr!("Other games")))
                    .child(div().child(fmt_secs(rest))),
            );
        }
        // Screenshots taken that month of its top games (any game if none).
        let mut shots: Vec<(usize, std::path::PathBuf)> = Vec::new();
        if let Some(all) = self.shots.clone() {
            let in_month =
                |g: usize| all.get(&g).and_then(|l| l.iter().filter(|s| b.start <= s.at.date() && s.at.date() < b.end).nth(0).cloned());
            for (g, _) in games.iter().take(3) {
                if let Some(s) = in_month(*g) {
                    shots.push((*g, s.path));
                }
                if shots.len() == 2 {
                    break;
                }
            }
            if shots.is_empty() {
                for (g, _) in games.iter().skip(3) {
                    if let Some(s) = in_month(*g) {
                        shots.push((*g, s.path));
                        break;
                    }
                }
            }
        }
        let shot_h = if shots.len() == 1 { 100. } else { 92. };
        let shot_w = if shots.len() == 1 { MONTH_W - 44. } else { (MONTH_W - 52.) / 2. };
        let shot_els: Vec<Div> = shots
            .iter()
            .map(|(g, path)| {
                let src = self.shot(path, shot_w, shot_h, window, cx);
                div()
                    .relative()
                    .flex_1()
                    .h(px(shot_h))
                    .rounded(px(12.))
                    .overflow_hidden()
                    .bg(theme::white_a(0.05))
                    .children(src.map(|s| img(s).absolute().inset_0().size_full().object_fit(ObjectFit::Cover)))
                    .child(
                        div()
                            .absolute()
                            .left(px(8.))
                            .bottom(px(7.))
                            .px(px(8.))
                            .py(px(3.))
                            .rounded_full()
                            .bg(rgba(0x0a0e14bf))
                            .font_family(theme::FONT_HEADING)
                            .font_weight(theme::W_BOLD)
                            .text_size(px(10.5))
                            .max_w(px(shot_w - 16.))
                            .truncate()
                            .child(self.game_title(*g)),
                    )
            })
            .collect();
        let partial = b.start <= today && today < b.end;
        let month_start = b.start;
        let is_month = (b.end - b.start).num_days() <= 31;
        let open = div()
            .id(("open-month", i))
            .absolute()
            .right(px(16.))
            .top(px(17.))
            .flex()
            .items_center()
            .gap(px(4.))
            .h(px(28.))
            .pl(px(13.))
            .pr(px(10.))
            .rounded_full()
            .bg(rgba(0x0a0e14b3))
            .border_1()
            .border_color(theme::white_a(0.14))
            .font_family(theme::FONT_HEADING)
            .font_weight(theme::W_BOLD)
            .text_size(px(12.))
            .cursor_pointer()
            .hover(|s| s.bg(Hsla { a: 0.2, ..Hsla::from(theme::accent()) }))
            .on_click(cx.listener(move |this, _, _, cx| {
                let span = if is_month { Span::Month } else { Span::Year };
                this.go(heronite_data::stats::Period::of(span, month_start), cx);
            }))
            .child(if is_month { tr!("Open month") } else { tr!("Open year") })
            .child(icon(Icon::ChevronRight, px(14.), theme::text_primary()));
        let mut art_box = div().relative().flex_1().min_h(px(110.)).overflow_hidden().bg(rgb(0x10151c));
        if let Some(src) = art {
            art_box = art_box.child(img(src).absolute().inset_0().size_full().object_fit(ObjectFit::Cover));
        }
        let fill = card_fill();
        art_box = art_box
            .child(band(0., 0.38, 0.68, 0., rgb(0x0a0d12)))
            .child(band(0.38, 0.76, 0., 0.6, fill))
            .child(band(0.76, 1.001, 0.6, 1., fill))
            .child(head(name.clone()))
            .child(open)
            .children(top.map(|g| {
                art_pill()
                    .absolute()
                    .left(px(20.))
                    .bottom(px(56.))
                    .pl(px(9.))
                    .pr(px(11.))
                    .py(px(4.))
                    .font_family(theme::FONT_HEADING)
                    .font_weight(theme::W_BOLD)
                    .text_size(px(11.5))
                    .child(div().size(px(8.)).rounded_full().bg(series_color(st.series_of(g))))
                    .child(if is_month { tr!("Game of the month") } else { tr!("Game of the year") })
                    .child(
                        div()
                            .font_family(theme::FONT_BODY)
                            .font_weight(theme::W_REGULAR)
                            .text_color(theme::text_muted())
                            .max_w(px(220.))
                            .truncate()
                            .child(self.game_title(g)),
                    )
            }));
        let body = div()
            .relative()
            .flex()
            .flex_col()
            .mt(px(-50.))
            .px(px(22.))
            .pb(px(20.))
            .child(
                div()
                    .font_family(theme::FONT_HEADING)
                    .font_weight(theme::W_BOLD)
                    .text_size(px(54.))
                    .line_height(px(56.))
                    .child(fmt_secs(b.secs)),
            )
            .child(div().mt(px(6.)).mb(px(12.)).text_size(px(12.5)).text_color(theme::text_muted()).child({
                let name = in_text(&name);
                if partial {
                    tr!("played in {name} so far \u{b7} {days} of {span_days} days", name, days, span_days)
                } else {
                    tr!("played in {name} \u{b7} {days} of {span_days} days", name, days, span_days)
                }
            }))
            .when(!big.is_empty(), |d| d.child(hbox(big, title, detail)))
            .child(rows)
            .child(dlab)
            .child(daily)
            .when(!shot_els.is_empty(), |d| d.child(div().flex().gap(px(8.)).mt(px(12.)).children(shot_els)));
        shell(MONTH_W).child(art_box).child(body).into_any_element()
    }

    /// One bar per day of bucket `b` (day scale shared by the whole period).
    fn daily_bars(&self, st: &Stats, b: &Bucket, best: Option<NaiveDate>, cx: &mut Context<Self>) -> Div {
        let max = st.busiest.map_or(1., |x| x.1.max(1.));
        let mut row = div().flex().items_end().gap(px(2.)).h(px(44.)).border_b_1().border_color(theme::white_a(0.1));
        let days = (b.end - b.start).num_days();
        // Year buckets: one bar per week.
        let step = if days > 31 { 7 } else { 1 };
        let mut d = b.start;
        while d < b.end {
            let end = (d + Duration::days(step)).min(b.end);
            let secs: f64 = if step == 1 { st.day_secs(d) } else { st.days.iter().filter(|(x, _)| d <= *x && *x < end).map(|x| x.1).sum() };
            let scale = if step == 1 { max } else { max * 7. };
            let on = self.hover == Some(Hover::Day(d));
            let color: Hsla = if on {
                gpui_kit::white()
            } else if Some(d) == best {
                theme::chart_ramp(1.).into()
            } else {
                theme::chart_series(0).into()
            };
            row = row.child(
                div()
                    .id(SharedString::from(format!("day-{d}")))
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_end()
                    .when(secs > 0. && step == 1, |el| el.on_hover(hover(Hover::Day(d), cx)))
                    .child(
                        div()
                            .w_full()
                            .h(px(if secs > 0. { ((secs / scale) as f32 * 44.).max(3.) } else { 0. }))
                            .rounded_t(px(3.))
                            .bg(color),
                    ),
            );
            d = end;
        }
        row
    }
}
