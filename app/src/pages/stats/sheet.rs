//! The game sheet: a drawer over the right of the Statistics page with one
//! game inside the period (its time, a bar per day or week, its best session,
//! its habits and its screenshots). Every cover and game name of the page
//! opens it; "Open in Collection" goes to the game there.

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::stats::{fmt_hours_short, fmt_secs, game_sheet, GameSheet, Span};
use heronite_i18n::{tr, trn};

use super::desktop::R_PANEL;
use super::{axis_label, day_label, full_date, nice_axis, series_color, StatsPage};
use crate::icons::Icon;
use crate::pages::PageEvent;
use crate::theme;
use crate::ui::controls::{button_sm, close_button, ButtonKind};
use crate::ui::icon;

/// Width of the drawer.
const SHEET_W: f32 = 540.;
const ART_H: f32 = 220.;
const CHART_H: f32 = 120.;

impl StatsPage {
    /// Opens the sheet of `game` for the period shown.
    pub(super) fn open_sheet(&mut self, game: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(h) = self.history.clone() else { return };
        self.sheet = Some(Arc::new(game_sheet(&h, game, self.period, super::today())));
        self.sheet_scroll.set_offset(point(px(0.), px(0.)));
        self.hover = None;
        self.focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn close_sheet(&mut self, cx: &mut Context<Self>) {
        if self.sheet.take().is_some() {
            cx.notify();
        }
    }

    /// Status name and colour of `game`.
    pub(super) fn game_status(&self, game: usize) -> Option<(String, Rgba)> {
        let lib = self.lib.as_ref()?;
        let g = lib.games.get(game)?;
        let s = lib.statuses.iter().find(|s| s.id == g.status_id)?;
        Some((heronite_data::status::status_label(&s.name).to_string(), theme::status_color(&s.slug)))
    }

    /// "FromSoftware · 2022 · Role-playing (RPG) · Adventure".
    fn game_meta_line(&self, game: usize) -> String {
        let Some(g) = self.lib.as_ref().and_then(|l| l.games.get(game)) else { return String::new() };
        let d = &g.details;
        let year = d.release_date.as_deref().and_then(|r| r.get(..4)).filter(|y| y.bytes().all(|b| b.is_ascii_digit()));
        d.developers
            .first()
            .map(String::as_str)
            .into_iter()
            .chain(year)
            .chain(d.genres.iter().take(2).map(String::as_str))
            .collect::<Vec<_>>()
            .join(" \u{b7} ")
    }

    /// The drawer and the scrim under it, over the page panel.
    pub(super) fn render_sheet(&mut self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sh = self.sheet.clone()?;
        let st = self.stats.clone()?;
        let game = sh.game;
        let art = self.art(game, SHEET_W, ART_H, window);
        let cover = self.cover(game, 70., 94., window);
        let key = self.game_key(game).map(|k| k.to_string()).unwrap_or_default();
        let color = match st.series_of(game) {
            Some(slot) => series_color(Some(slot)),
            None => theme::accent().into(),
        };
        let fill = rgb(0x0f131a);
        let ink = |a: f32| Hsla { a, ..Hsla::from(fill) };

        let mut head = div().relative().h(px(ART_H)).flex_none().overflow_hidden();
        if let Some(src) = art {
            head = head.child(img(src).absolute().inset_0().size_full().object_fit(ObjectFit::Cover));
        }
        head = head
            .child(div().absolute().inset_0().bg(linear_gradient(180., linear_color_stop(ink(0.15), 0.), linear_color_stop(ink(1.), 1.))))
            .child(
                close_button("stats-sheet-close")
                    .absolute()
                    .top(px(14.))
                    .right(px(14.))
                    .on_click(cx.listener(|this, _, _, cx| this.close_sheet(cx))),
            );
        let status = self.game_status(game);
        let title = div()
            .flex()
            .items_end()
            .gap(px(16.))
            .mt(px(-64.))
            .px(px(24.))
            .child(
                div()
                    .flex_none()
                    .w(px(70.))
                    .h(px(94.))
                    .rounded(px(10.))
                    .overflow_hidden()
                    .bg(theme::white_a(0.06))
                    .border_1()
                    .border_color(theme::white_a(0.12))
                    .children(cover.map(|s| img(s).size_full().object_fit(ObjectFit::Cover))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .font_family(theme::font_heading())
                            .font_weight(theme::W_BOLD)
                            .text_size(px(24.))
                            .line_height(px(28.))
                            .child(self.game_title(game)),
                    )
                    .child(div().text_size(px(12.5)).text_color(theme::text_muted()).truncate().child(self.game_meta_line(game)))
                    .children(status.map(|(name, c)| div().flex().child(super::super::status_pill(name, c)))),
            );
        let actions = div().flex().gap(px(8.)).px(px(24.)).mt(px(16.)).child(
            button_sm("stats-sheet-open", ButtonKind::Glass, tr!("Open in Collection")).on_click(cx.listener(move |this, _, _, cx| {
                this.sheet = None;
                cx.emit(PageEvent::ShowGame(key.clone()));
            })),
        );

        let rank = st.games.iter().position(|g| g.game == game);
        let share = sh.secs / st.total_secs.max(1.) * 100.;
        let period = match st.period.span {
            Span::All => tr!("all time").to_string(),
            _ => st.period.label(),
        };
        let big = div()
            .px(px(24.))
            .mt(px(22.))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(10.))
                    .child(div().font_family(theme::font_heading()).font_weight(theme::W_BOLD).text_size(px(44.)).child(fmt_secs(sh.secs)))
                    .child(div().text_size(px(14.)).text_color(theme::text_muted()).child(if st.period.span == Span::All {
                        tr!("all time").to_string()
                    } else {
                        tr!("in {period} \u{b7} {total} all time", period, total = fmt_hours_short(sh.life_secs))
                    })),
            )
            .children(rank.filter(|_| sh.secs > 0.).map(|r| {
                div().text_size(px(13.)).text_color(theme::text_muted()).child(tr!(
                    "#{rank} game of {period} \u{b7} {pct}% of your time",
                    rank = r + 1,
                    period,
                    pct = format!("{share:.0}")
                ))
            }));

        let chart = self.sheet_chart(&sh, st.period.span, color);
        let facts = self.sheet_facts(&sh);
        let shots = self.sheet_shots(game, st.period, window, cx);

        let body = div().id("stats-sheet-scroll").size_full().overflow_y_scroll().track_scroll(&self.sheet_scroll).child(
            div()
                .flex()
                .flex_col()
                .pb(px(28.))
                .child(head)
                .child(title)
                .child(actions)
                .child(big)
                .child(chart)
                .child(facts)
                .children(shots),
        );
        let drawer = div()
            .id("stats-sheet")
            .occlude()
            .absolute()
            .top(px(10.))
            .right(px(10.))
            .bottom(px(10.))
            .w(px(SHEET_W))
            .rounded(px(R_PANEL))
            .overflow_hidden()
            .bg(fill)
            .border_1()
            .border_color(theme::white_a(0.1))
            .shadow(vec![BoxShadow {
                color: theme::black_a(0.5),
                offset: point(px(-12.), px(0.)),
                blur_radius: px(50.),
                spread_radius: px(0.),
                inset: false,
            }])
            .child(body);
        let scrim = div()
            .id("stats-sheet-scrim")
            .absolute()
            .inset_0()
            .bg(theme::black_a(0.42))
            .on_click(cx.listener(|this, _, _, cx| this.close_sheet(cx)));
        Some(div().absolute().inset_0().child(scrim).child(drawer).into_any_element())
    }

    /// Bars of the game across the period (days or weeks).
    fn sheet_chart(&self, sh: &GameSheet, span: Span, color: Hsla) -> Div {
        let max = sh.bars.iter().map(|b| b.1).fold(0., f64::max);
        let (step, top) = nice_axis(max / 3600.);
        let lines = (top / step).round() as usize;
        let top_secs = top * 3600.;
        let grid = (0..=lines).map(|i| {
            let y = CHART_H * i as f32 / lines as f32;
            div().absolute().left_0().right_0().bottom(px(y)).h(px(1.)).bg(theme::white_a(if i == 0 { 0.12 } else { 0.05 }))
        });
        let labels = (0..=lines).map(|i| {
            let y = CHART_H * i as f32 / lines as f32;
            div()
                .absolute()
                .right(px(6.))
                .bottom(px(y - 7.))
                .text_size(px(10.5))
                .text_color(theme::text_dim())
                .child(axis_label(step * i as f64))
        });
        let bars = sh.bars.iter().map(|(_, secs)| {
            let h = (*secs / top_secs) as f32 * CHART_H;
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .justify_end()
                .px(px(1.))
                .child(div().w_full().h(px(if *secs > 0. { h.max(2.) } else { 0. })).rounded_t(px(3.)).bg(color))
        });
        let (first, last) = (sh.bars.first().map(|b| b.0), sh.bars.last().map(|b| b.0));
        let ends = div()
            .flex()
            .justify_between()
            .mt(px(6.))
            .ml(px(36.))
            .text_size(px(11.))
            .text_color(theme::text_dim())
            .children(first.map(super::short_date))
            .children(last.map(super::short_date));
        let title = match span {
            Span::Week | Span::Month => tr!("Day by day"),
            _ => tr!("Week by week"),
        };
        div()
            .px(px(24.))
            .mt(px(26.))
            .child(div().font_family(theme::font_heading()).font_weight(theme::W_BOLD).text_size(px(15.)).mb(px(12.)).child(title))
            .child(div().flex().child(div().relative().flex_none().w(px(36.)).h(px(CHART_H)).children(labels)).child(
                div().relative().flex_1().min_w_0().h(px(CHART_H)).children(grid).child(div().absolute().inset_0().flex().children(bars)),
            ))
            .child(ends)
    }

    fn sheet_facts(&self, sh: &GameSheet) -> Div {
        let cell = |label: String, value: String, sub: Option<String>| {
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .p(px(14.))
                .rounded(px(12.))
                .bg(theme::white_a(0.035))
                .border_1()
                .border_color(theme::white_a(0.06))
                .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(label))
                .child(div().font_family(theme::font_heading()).font_weight(theme::W_BOLD).text_size(px(18.)).child(value))
                .children(sub.map(|s| div().text_size(px(12.)).text_color(theme::text_dim()).truncate().child(s)))
        };
        let mut cells =
            vec![cell(tr!("Sessions").into(), sh.sessions.to_string(), Some(trn!(sh.days_played, "{n} day played", "{n} days played")))];
        if sh.sessions > 0 {
            cells.push(cell(tr!("Average session").into(), fmt_secs(sh.secs / sh.sessions as f64), None));
        }
        if let Some(b) = sh.best {
            cells.push(cell(tr!("Longest session").into(), fmt_secs(b.secs), Some(day_label(b.start.date()))));
        }
        if let Some(h) = sh.usual_hour {
            cells.push(cell(tr!("Usually starts").into(), tr!("around {h}:00", h), None));
        }
        if sh.secs > 0. {
            cells.push(cell(tr!("On weekends").into(), format!("{:.0}%", sh.weekend * 100.), Some(tr!("of its time").into())));
        }
        if let Some(f) = sh.first {
            cells.push(cell(tr!("First played").into(), full_date(f), None));
        }
        if let Some(l) = sh.last {
            let recent = (super::today() - l.date()).num_days() < 7;
            cells.push(cell(tr!("Last session").into(), full_date(l.date()), recent.then(|| super::when_label(l))));
        }
        div().grid().grid_cols(2).gap(px(10.)).px(px(24.)).mt(px(24.)).children(cells)
    }

    /// Screenshots of the game: those of the period, else its latest.
    fn sheet_shots(&mut self, game: usize, period: heronite_data::stats::Period, window: &Window, cx: &mut Context<Self>) -> Option<Div> {
        let all = self.shots.as_ref()?.get(&game)?.clone();
        let inside: Vec<_> = all.iter().filter(|s| period.contains(s.at.date())).cloned().collect();
        let (list, label) = if inside.is_empty() {
            (all.iter().rev().take(6).cloned().collect::<Vec<_>>(), tr!("Latest screenshots").to_string())
        } else {
            let n = inside.len();
            (inside.into_iter().rev().take(6).collect(), trn!(n, "{n} screenshot in this period", "{n} screenshots in this period"))
        };
        let w = (SHEET_W - 48. - 16.) / 3.;
        let h = w * 9. / 16.;
        let thumbs: Vec<Div> = list
            .iter()
            .map(|s| {
                let src = self.shot(&s.path, w, h, window, cx);
                div()
                    .w(px(w))
                    .h(px(h))
                    .rounded(px(8.))
                    .overflow_hidden()
                    .bg(theme::white_a(0.05))
                    .children(src.map(|s| img(s).size_full().object_fit(ObjectFit::Cover)))
            })
            .collect();
        Some(
            div()
                .px(px(24.))
                .mt(px(26.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .mb(px(12.))
                        .child(icon(Icon::PhotoCamera, px(15.), theme::text_muted()))
                        .child(div().font_family(theme::font_heading()).font_weight(theme::W_BOLD).text_size(px(15.)).child(label)),
                )
                .child(div().flex().flex_wrap().gap(px(8.)).children(thumbs)),
        )
    }
}
