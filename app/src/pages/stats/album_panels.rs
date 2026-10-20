//! The Album tab's panels under the moments: the most photographed games,
//! when you capture against when you play, a photo per month and the
//! captures per session.

use std::sync::Arc;

use chrono::Datelike;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::stats::{fmt_secs, month_name, Span, Stats};
use heronite_i18n::{tr, trn};

use super::album::AlbumView;
use super::desktop::{hint, panel, panel_head};
use super::sections::sentence;
use super::{short_date, StatsPage};
use crate::theme;

impl StatsPage {
    /// Captures per hour played, per game.
    pub(super) fn album_rate(&mut self, a: &AlbumView, st: &Stats, window: &Window, cx: &mut Context<Self>) -> Div {
        let head = panel_head(tr!("Most photographed"), hint(tr!("captures per hour played")));
        let Some(first) = a.rate.first() else {
            return panel().child(head).child(
                div()
                    .text_size(px(13.))
                    .text_color(theme::text_muted())
                    .child(tr!("Not enough captures yet: 5 or more in a game played 1h or more.")),
            );
        };
        let max = first.1 as f64 / (first.2 / 3600.);
        let mut list = div().flex().flex_col().gap(px(10.));
        for (g, n, secs) in a.rate.iter().take(7) {
            let game = *g;
            let r = *n as f64 / (secs / 3600.);
            let cover = self.cover(game, 26., 34., window);
            let color: Hsla = match st.series_of(game) {
                Some(s) => super::series_color(Some(s)),
                None => theme::chart_ramp(0.7).into(),
            };
            list = list.child(
                div()
                    .id(("album-rate", game))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| this.open_sheet(game, window, cx)))
                    .child(
                        div()
                            .flex_none()
                            .w(px(26.))
                            .h(px(34.))
                            .rounded(px(4.))
                            .overflow_hidden()
                            .bg(theme::white_a(0.06))
                            .children(cover.map(|s| img(s).size_full().object_fit(ObjectFit::Cover))),
                    )
                    .child(
                        div()
                            .w(px(190.))
                            .flex_none()
                            .flex()
                            .flex_col()
                            .child(div().text_size(px(13.)).truncate().child(self.game_title(game)))
                            .child(div().text_size(px(11.5)).text_color(theme::text_dim()).child(tr!(
                                "{n} captures \u{b7} {time}",
                                n,
                                time = fmt_secs(*secs)
                            ))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h(px(10.))
                            .rounded_full()
                            .bg(theme::white_a(0.06))
                            .child(div().h_full().w(relative((r / max) as f32)).rounded_full().bg(color)),
                    )
                    .child(
                        div()
                            .w(px(70.))
                            .flex_none()
                            .flex()
                            .justify_end()
                            .items_baseline()
                            .gap(px(3.))
                            .child(
                                div()
                                    .font_family(theme::font_heading())
                                    .font_weight(theme::W_BOLD)
                                    .text_size(px(15.))
                                    .child(heronite_i18n::decimal(r, 1)),
                            )
                            .child(div().text_size(px(11.)).text_color(theme::text_dim()).child(tr!("/h"))),
                    ),
            );
        }
        let lead = tr!(
            "**{game}** makes you stop to capture {n} times an hour.",
            game = self.game_title(first.0),
            n = heronite_i18n::decimal(max, 1)
        );
        panel()
            .flex_1()
            .child(head)
            .child(div().mt(px(-6.)).mb(px(14.)).text_size(px(13.)).text_color(theme::text_muted()).child(sentence(lead)))
            .child(list)
    }

    /// Captures and play time across the 24 hours.
    pub(super) fn album_hours(&self, a: &AlbumView) -> Div {
        let (caps, play) = a.hours;
        let max = caps.iter().chain(play.iter()).copied().fold(0.01, f64::max);
        let h = 150.;
        let argmax = |v: &[f64; 24]| v.iter().enumerate().max_by(|x, y| x.1.total_cmp(y.1)).map_or(0, |(i, _)| i);
        // The hour that is most photogenic: the biggest share of captures over play.
        let photogenic = (0..24).filter(|i| play[*i] >= 0.02).max_by(|x, y| (caps[*x] / play[*x]).total_cmp(&(caps[*y] / play[*y])));
        let cols = (0..24).map(|i| {
            let bar = |v: f64, c: Hsla| div().flex_1().h(px((v / max) as f32 * h)).rounded_t(px(2.)).bg(c);
            div()
                .flex_1()
                .h_full()
                .flex()
                .items_end()
                .gap(px(1.))
                .px(px(1.))
                .child(bar(caps[i], theme::chart_series(1).into()))
                .child(bar(play[i], theme::chart_ramp(0.55).into()))
        });
        let ticks = div()
            .flex()
            .mt(px(6.))
            .children((0..4).map(|q| div().flex_1().text_size(px(11.)).text_color(theme::text_dim()).child(format!("{:02}h", q * 6))));
        let legend = div()
            .flex()
            .gap(px(14.))
            .text_size(px(12.))
            .text_color(theme::text_muted())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().size(px(9.)).rounded(px(2.)).bg(theme::chart_series(1)))
                    .child(tr!("captures")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().size(px(9.)).rounded(px(2.)).bg(theme::chart_ramp(0.55)))
                    .child(tr!("play time")),
            );
        let mut lead = tr!("You play most at **{h}h** and capture most at **{c}h**.", h = argmax(&play), c = argmax(&caps));
        if let Some(p) = photogenic {
            lead = format!("{lead} {}", tr!("The most photogenic hour is **{h}h**.", h = p));
        }
        panel()
            .flex_1()
            .child(panel_head(tr!("When you capture \u{d7} when you play"), legend))
            .child(div().mt(px(-6.)).mb(px(14.)).text_size(px(13.)).text_color(theme::text_muted()).child(sentence(lead)))
            .child(div().flex().h(px(h)).border_b_1().border_color(theme::white_a(0.12)).children(cols))
            .child(ticks)
    }

    /// A photo per month (year and all time), else the latest captures.
    pub(super) fn album_months(&mut self, a: &AlbumView, st: &Stats, window: &Window, cx: &mut Context<Self>) -> Div {
        let list = Arc::new(a.photos.clone());
        if matches!(st.period.span, Span::Year | Span::All) {
            // Each month: the middle capture of its most photographed game.
            let mut months: std::collections::BTreeMap<(i32, u32), Vec<usize>> = Default::default();
            for (i, p) in a.photos.iter().enumerate() {
                months.entry((p.at.year(), p.at.month())).or_default().push(i);
            }
            let mut picks: Vec<((i32, u32), usize)> = Vec::new();
            for (k, idx) in &months {
                let mut per: std::collections::HashMap<usize, Vec<usize>> = Default::default();
                for i in idx {
                    per.entry(a.photos[*i].game).or_default().push(*i);
                }
                if let Some(best) = per.values().max_by_key(|v| v.len()) {
                    picks.push((*k, best[best.len() / 2]));
                }
            }
            let tiles: Vec<Div> = picks
                .iter()
                .rev()
                .take(12)
                .rev()
                .map(|((y, m), i)| {
                    let label = if st.period.span == Span::All { format!("{} {}", month_name(*m), y) } else { month_name(*m).to_string() };
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(self.photo_tile(
                            SharedString::from(format!("month-photo-{y}-{m}")),
                            list.clone(),
                            *i,
                            196.,
                            110.,
                            None,
                            window,
                            cx,
                        ))
                        .child(div().text_size(px(12.)).font_weight(theme::W_BOLD).child(label))
                        .child(
                            div()
                                .mt(px(-4.))
                                .text_size(px(11.5))
                                .text_color(theme::text_dim())
                                .truncate()
                                .max_w(px(196.))
                                .child(self.game_title(a.photos[*i].game)),
                        )
                })
                .collect();
            return panel()
                .flex_1()
                .child(panel_head(tr!("Photo of the month"), hint(tr!("from the most captured game of each month"))))
                .child(div().flex().flex_wrap().gap(px(12.)).children(tiles));
        }
        let n = list.len();
        let tiles: Vec<Stateful<Div>> = (n.saturating_sub(12)..n)
            .rev()
            .map(|i| {
                let label = Some(short_date(list[i].at.date()));
                self.photo_tile(SharedString::from(format!("latest-{i}")), list.clone(), i, 196., 110., label, window, cx)
            })
            .collect();
        panel()
            .flex_1()
            .child(panel_head(tr!("Latest captures"), hint(trn!(n, "{n} in this period", "{n} in this period"))))
            .child(div().flex().flex_wrap().gap(px(12.)).children(tiles))
    }

    /// Sessions by how many captures they left.
    pub(super) fn album_sessions(&self, a: &AlbumView) -> Div {
        let total = a.per_session.iter().sum::<usize>().max(1);
        let labels = [tr!("none"), "1", "2\u{2013}4", "5\u{2013}9", "10+"];
        let max = a.per_session.iter().copied().max().unwrap_or(1).max(1);
        let rows = (0..5).map(|i| {
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(div().w(px(46.)).flex_none().text_size(px(12.5)).text_color(theme::text_muted()).child(labels[i]))
                .child(
                    div().flex_1().h(px(12.)).child(
                        div()
                            .h_full()
                            .w(relative(a.per_session[i] as f32 / max as f32))
                            .rounded_r(px(4.))
                            .bg(theme::chart_ramp(0.35 + 0.15 * i as f32)),
                    ),
                )
                .child(div().w(px(70.)).flex_none().flex().justify_end().text_size(px(12.5)).child(format!(
                    "{} \u{b7} {}%",
                    a.per_session[i],
                    a.per_session[i] * 100 / total
                )))
        });
        let none = a.per_session[0] * 100 / total;
        panel()
            .flex_1()
            .child(panel_head(tr!("Captures per session"), hint(trn!(total, "{n} session", "{n} sessions"))))
            .child(
                div()
                    .mt(px(-6.))
                    .mb(px(14.))
                    .text_size(px(13.))
                    .text_color(theme::text_muted())
                    .child(sentence(tr!("**{pct}%** of your sessions end without a capture.", pct = none))),
            )
            .child(div().flex().flex_col().gap(px(10.)).children(rows))
    }
}
