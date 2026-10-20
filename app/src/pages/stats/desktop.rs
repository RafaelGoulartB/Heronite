//! Desktop layout of the Statistics page: a dark panel whose top is the
//! hero (the period's number one game, the period switch, the total and the
//! Year in review button), then a column of glass panels.

use chrono::Datelike;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::stats::{fmt_hours_short, fmt_secs, month_in_sentence, Span, Stats};
use heronite_i18n::{n_, tr};

use super::tabs::Tab;
use super::{change_pct, today, StatsPage};
use crate::icons::Icon;
use crate::pages::page_close;
use crate::theme;
use crate::ui::icon;

/// Height of the hero band.
const HERO_H: f32 = 372.;
/// Side padding of the page column.
const PAD: f32 = 32.;
/// Widest the column gets (ultrawide windows keep even margins).
const MAX_W: f32 = 1840.;
/// Height of the bar that keeps the tabs and the period in reach while the
/// page scrolls.
const BAR_H: f32 = 76.;
/// Scroll (px) past which the bar shows: the hero's own controls are gone.
const BAR_AT: f32 = 84.;
/// Gap between panels.
pub(super) const GAP: f32 = 16.;
/// Panel corner radius.
pub(super) const R_PANEL: f32 = 20.;

/// Glass panel of the page (padding 22 × 24).
pub(super) fn panel() -> Div {
    div()
        .relative()
        .flex()
        .flex_col()
        .min_w_0()
        .px(px(24.))
        .py(px(22.))
        .rounded(px(R_PANEL))
        .bg(rgba(0x161b24d6))
        .border_1()
        .border_color(theme::white_a(0.08))
        .shadow(vec![BoxShadow {
            color: theme::black_a(0.28),
            offset: point(px(0.), px(12.)),
            blur_radius: px(40.),
            spread_radius: px(0.),
            inset: false,
        }])
}

/// Panel title row: title on the left, `right` (hint or action) on the right.
pub(super) fn panel_head(title: impl Into<SharedString>, right: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.))
        .mb(px(14.))
        .child(
            div()
                .font_family(theme::font_heading())
                .font_weight(theme::W_BOLD)
                .text_size(px(17.))
                .text_color(theme::text_primary())
                .child(title.into()),
        )
        .child(right)
}

/// Muted hint text for [`panel_head`].
pub(super) fn hint(text: impl Into<SharedString>) -> Div {
    div().text_size(px(12.5)).text_color(theme::text_muted()).whitespace_nowrap().child(text.into())
}

/// Uppercase small label ("TIME PLAYED · 2026").
pub(super) fn eyebrow(text: impl Into<SharedString>, alpha: f32) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .font_family(theme::font_heading())
        .font_weight(theme::W_BOLD)
        .text_size(px(12.))
        .text_color(theme::white_a(alpha))
        .child(div().w(px(26.)).h(px(1.)).bg(theme::white_a(alpha * 0.7)))
        .child(text.into().to_uppercase())
}

/// Pill-shaped glass capsule (period switch, year arrows, buttons).
fn capsule() -> Div {
    div().flex().items_center().p(px(3.)).gap(px(2.)).rounded_full().bg(rgba(0x161b24d6)).border_1().border_color(theme::white_a(0.08))
}

/// "711h" and "20m" of a big total (all time: "1,402h" alone).
fn total_parts(secs: f64, all: bool) -> (String, Option<String>) {
    if all && secs >= 100. * 3600. {
        return (fmt_hours_short(secs), None);
    }
    let mins = (secs / 60.).round() as u64;
    let (h, m) = (mins / 60, mins % 60);
    if h == 0 {
        (tr!("{m}m", m), None)
    } else {
        (tr!("{h}h", h), Some(tr!("{m}m", m)))
    }
}

/// "· your #1 game of October 2026".
fn top_game_line(st: &Stats) -> String {
    match st.period.span {
        Span::Week => tr!("\u{b7} your #1 game of this week").into(),
        Span::Month => {
            tr!("\u{b7} your #1 game of {month} {year}", month = month_in_sentence(st.period.start.month()), year = st.period.start.year())
        }
        Span::Year => tr!("\u{b7} your #1 game of {year}", year = st.period.start.year()),
        Span::All => tr!("\u{b7} your #1 game of all time").into(),
    }
}

impl StatsPage {
    /// Year the review button opens: the period's year (all time: this year).
    pub(super) fn review_year(&self) -> i32 {
        match self.period.span {
            Span::All => today().year(),
            _ => self.period.start.year(),
        }
    }

    fn render_switch(&self, cx: &mut Context<Self>) -> Div {
        let spans = [(Span::Week, n_!("Week")), (Span::Month, n_!("Month")), (Span::Year, n_!("Year")), (Span::All, n_!("All time"))];
        let mut seg = capsule();
        for (span, label) in spans {
            let on = self.period.span == span;
            // Spans the tab cannot show are dimmed (the tab shows the year).
            let allowed = self.tab.spans().contains(&span);
            seg = seg.child(
                div()
                    .id(SharedString::from(format!("stats-span-{label}")))
                    .flex()
                    .items_center()
                    .h(px(34.))
                    .px(px(16.))
                    .rounded_full()
                    .font_family(theme::font_heading())
                    .font_weight(theme::W_BOLD)
                    .text_size(px(13.))
                    .when(on, |d| d.bg(theme::accent()).text_color(theme::on_accent()))
                    .when(!on && allowed, |d| {
                        d.text_color(theme::text_muted())
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::white_a(0.06)).text_color(theme::text_primary()))
                            .on_click(cx.listener(move |this, _, _, cx| this.set_span(span, cx)))
                    })
                    .when(!on && !allowed, |d| d.text_color(theme::text_muted()).opacity(0.35))
                    .child(heronite_i18n::tr(label)),
            );
        }
        let all = self.period.span == Span::All;
        let label = match self.period.span {
            Span::All => self
                .history
                .as_ref()
                .and_then(|h| h.since)
                .map_or_else(String::new, |d| tr!("Since {month} {year}", month = month_in_sentence(d.month()), year = d.year())),
            _ => self.period.label(),
        };
        let arrow = |id: &'static str, ic: Icon, enabled: bool| {
            div()
                .id(id)
                .flex()
                .items_center()
                .justify_center()
                .size(px(34.))
                .rounded_full()
                .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(theme::white_a(0.08))))
                .when(!enabled, |d| d.opacity(0.3))
                .child(icon(ic, px(18.), theme::text_muted()))
        };
        let (can_prev, can_next) = (self.can_prev(), self.can_next());
        let mut prev = arrow("stats-prev", Icon::ChevronLeft, can_prev);
        if can_prev {
            prev = prev.on_click(cx.listener(|this, _, _, cx| {
                if let Some(p) = this.period.prev() {
                    this.go(p, cx);
                }
            }));
        }
        let mut next = arrow("stats-next", Icon::ChevronRight, can_next);
        if can_next {
            next = next.on_click(cx.listener(|this, _, _, cx| {
                if let Some(p) = this.period.next() {
                    this.go(p, cx);
                }
            }));
        }
        let year = capsule()
            .when(!all, |d| d.child(prev))
            .child(
                div()
                    .px(px(14.))
                    .min_w(px(70.))
                    .flex()
                    .justify_center()
                    .font_family(theme::font_heading())
                    .font_weight(theme::W_BOLD)
                    .text_size(px(15.))
                    .text_color(theme::text_primary())
                    .whitespace_nowrap()
                    .child(label),
            )
            .when(!all, |d| d.child(next));
        div().flex().items_center().gap(px(10.)).child(seg).child(year)
    }

    /// "Your 2026 in review is ready ▶ Watch" (only for a year with play).
    fn render_review_button(&mut self, window: &Window, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let (year, total, thumbs) = self.year_summary.clone()?;
        if total < 3600. {
            return None;
        }
        let mut mosaic = div().flex();
        for (i, g) in thumbs.into_iter().enumerate() {
            let src = self.art(g, 50., 34., window);
            mosaic = mosaic.child(
                div()
                    .w(px(50.))
                    .h(px(34.))
                    .when(i > 0, |d| d.ml(px(-14.)))
                    .rounded(px(9.))
                    .border_2()
                    .border_color(rgb(0x161b24))
                    .overflow_hidden()
                    .bg(theme::white_a(0.06))
                    .children(src.map(|s| img(s).size_full().object_fit(ObjectFit::Cover))),
            );
        }
        let ongoing = year == today().year();
        let title = if ongoing { tr!("Your {year} so far, in review", year) } else { tr!("Your {year} in review is ready", year) };
        Some(
            div()
                .id("stats-review")
                .flex()
                .items_center()
                .gap(px(14.))
                .p(px(8.))
                .rounded_full()
                .bg(rgba(0x161b24d6))
                .border_1()
                .border_color(theme::white_a(0.14))
                .shadow(vec![BoxShadow {
                    color: theme::black_a(0.35),
                    offset: point(px(0.), px(10.)),
                    blur_radius: px(30.),
                    spread_radius: px(0.),
                    inset: false,
                }])
                .cursor_pointer()
                .hover(|s| s.border_color(Hsla { a: 0.5, ..Hsla::from(theme::accent()) }))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.focus.focus(window, cx);
                    this.open_review(year, cx);
                }))
                .child(mosaic)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .font_family(theme::font_heading())
                                .font_weight(theme::W_BOLD)
                                .text_size(px(14.))
                                .text_color(theme::text_primary())
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme::text_muted())
                                .child(tr!("7 stories \u{b7} Ctrl K \u{2192} Year in review")),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .h(px(34.))
                        .px(px(16.))
                        .rounded_full()
                        .bg(theme::accent())
                        .text_color(theme::on_accent())
                        .font_family(theme::font_heading())
                        .font_weight(theme::W_BOLD)
                        .text_size(px(13.))
                        .child(icon(Icon::PlayArrow, px(16.), theme::on_accent()))
                        .child(tr!("Watch")),
                ),
        )
    }

    fn render_hero(&mut self, st: Option<&Stats>, panel_w: f32, col_w: f32, window: &Window, cx: &mut Context<Self>) -> Div {
        let top = st.and_then(|s| s.games.first()).map(|g| g.game);
        // Discoveries show the art of the game their lead is about.
        let art_game = match self.tab {
            Tab::Discoveries => self.lead_game().or(top),
            Tab::Album => self.album_lead().or(top),
            _ => top,
        };
        let art = art_game.and_then(|g| self.art(g, panel_w, HERO_H, window));
        let page = theme::stats_page_bg();
        let ink = |a: f32| Hsla { a, ..Hsla::from(page) };
        let mut hero = div().relative().w_full().h(px(HERO_H)).flex_none();
        if let Some(src) = art {
            hero = hero.child(img(src).absolute().inset_0().size_full().object_fit(ObjectFit::Cover).rounded_t(px(theme::R_SHEET - 1.)));
        }
        // Darken the left (text side) and fade the bottom into the page.
        hero = hero
            .child(div().absolute().top_0().bottom_0().left_0().w(relative(0.72)).bg(linear_gradient(
                90.,
                linear_color_stop(ink(0.9), 0.),
                linear_color_stop(ink(0.16), 1.),
            )))
            .child(div().absolute().top_0().bottom_0().right_0().w(relative(0.28)).bg(linear_gradient(
                90.,
                linear_color_stop(ink(0.16), 0.),
                linear_color_stop(ink(0.3), 1.),
            )))
            .child(div().absolute().left_0().right_0().bottom(px(118.)).h(px(144.)).bg(linear_gradient(
                180.,
                linear_color_stop(ink(0.), 0.),
                linear_color_stop(ink(0.7), 1.),
            )))
            .child(div().absolute().left_0().right_0().bottom(px(-1.)).h(px(119.)).bg(linear_gradient(
                180.,
                linear_color_stop(ink(0.7), 0.),
                linear_color_stop(ink(1.), 1.),
            )));

        let review = if self.tab == Tab::Overview { self.render_review_button(window, cx) } else { None };
        let controls = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(12.))
            .child(self.render_tabs(cx))
            .child(self.render_switch(cx))
            .child(div().flex_1())
            .children(review);
        let mut column = div().relative().flex().flex_col().w(px(col_w)).h_full().mx_auto().pt(px(28.)).child(controls);
        if let Some(st) = st.filter(|s| s.total_secs > 0.) {
            column = match self.tab {
                Tab::Overview => column.child(self.render_total(st)),
                Tab::Journeys => column.children(self.render_journeys_total(st)),
                Tab::Backlog => column.child(self.render_backlog_total(st, cx)),
                Tab::Discoveries => column.child(self.render_discoveries_total(st, window, cx)),
                Tab::Album => column.child(self.render_album_total(st, window, cx)),
            };
            // The Backlog's goal card takes that corner.
            if let Some(g) = top.filter(|_| matches!(self.tab, Tab::Overview | Tab::Journeys)) {
                let cover = self.cover(g, 24., 32., window);
                let title = self.game_title(g);
                column = column.child(
                    div()
                        .absolute()
                        .right_0()
                        .bottom(px(34.))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .pl(px(5.))
                        .pr(px(14.))
                        .py(px(5.))
                        .rounded_full()
                        .bg(rgba(0x0a0c1099))
                        .border_1()
                        .border_color(theme::white_a(0.08))
                        .text_size(px(12.))
                        .text_color(theme::text_muted())
                        .child(
                            div()
                                .w(px(24.))
                                .h(px(32.))
                                .rounded(px(6.))
                                .overflow_hidden()
                                .bg(theme::white_a(0.06))
                                .children(cover.map(|s| img(s).size_full().object_fit(ObjectFit::Cover))),
                        )
                        .child(tr!("Art:"))
                        .child(div().font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(title))
                        .child(top_game_line(st)),
                );
            }
        }
        hero.child(column)
    }

    /// The tabs and the period switch pinned to the top of the panel once the
    /// hero scrolled away, so a long tab (Journeys, Album) never needs the
    /// trip back up to change tab or period.
    fn render_bar(&self, col_w: f32, cx: &mut Context<Self>) -> Div {
        let page = Hsla::from(theme::stats_page_bg());
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(BAR_H))
            .bg(page)
            .border_b_1()
            .border_color(theme::white_a(0.08))
            .shadow(vec![BoxShadow {
                color: theme::black_a(0.4),
                offset: point(px(0.), px(8.)),
                blur_radius: px(24.),
                spread_radius: px(0.),
                inset: false,
            }])
            .child(
                div()
                    .w(px(col_w))
                    .h_full()
                    .mx_auto()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .overflow_hidden()
                    .child(self.render_tabs(cx))
                    .child(self.render_switch(cx)),
            )
    }

    /// Eyebrow, the big total and the story line under it.
    fn render_total(&self, st: &Stats) -> Div {
        let all = st.period.span == Span::All;
        let eyebrow_text = match st.period.span {
            Span::All => tr!("Time played \u{b7} all time").into(),
            _ => tr!("Time played \u{b7} {period}", period = st.period.label()),
        };
        let (main, small) = total_parts(st.total_secs, all);
        let number = div()
            .flex()
            .items_baseline()
            .gap(px(6.))
            .mt(px(8.))
            .mb(px(12.))
            .font_family(theme::font_heading())
            .font_weight(theme::W_BOLD)
            .text_color(theme::text_primary())
            .child(div().text_size(px(96.)).line_height(px(96.)).child(main))
            .children(small.map(|s| div().text_size(px(48.)).text_color(theme::white_a(0.78)).child(s)));

        let mut story = div().flex().flex_wrap().items_center().gap(px(14.)).text_size(px(16.)).text_color(theme::white_a(0.78));
        let prev_name = st.period.prev().map(|p| p.short_name()).unwrap_or_default();
        let over = st.period.end <= today();
        match (change_pct(st), st.prev_secs) {
            (Some(c), _) if c.abs() >= 1. && (over || c > 0.) => {
                let (text, color) = if c > 0. {
                    (tr!("\u{25b2} +{pct}% vs {name}", pct = format!("{c:.0}"), name = prev_name), theme::success())
                } else {
                    (tr!("\u{25bc} {pct}% vs {name}", pct = format!("{c:.0}"), name = prev_name), theme::danger_text())
                };
                let tint: Hsla = color.into();
                story = story.child(
                    div()
                        .flex()
                        .items_center()
                        .h(px(28.))
                        .px(px(12.))
                        .rounded_full()
                        .bg(Hsla { a: 0.14, ..tint })
                        .border_1()
                        .border_color(Hsla { a: 0.3, ..tint })
                        .text_color(color)
                        .font_family(theme::font_heading())
                        .font_weight(theme::W_BOLD)
                        .text_size(px(13.))
                        .child(text),
                );
            }
            (_, Some(prev)) if prev > 0. => story = story.child(fact(fmt_secs(prev), super::capitalised(&prev_name), true)),
            _ => {}
        }
        let sep = || div().size(px(4.)).rounded_full().bg(theme::white_a(0.35));
        story = story.child(if all {
            fact(st.days_played.to_string(), tr!("days played").into(), false)
        } else {
            fact(st.days_played.to_string(), tr!("of {days} days", days = st.days_elapsed), false)
        });
        if st.streak > 1 {
            story = story.child(sep()).child(fact(tr!("{n}-day", n = st.streak), tr!("streak").into(), false));
        }
        story = story.child(sep()).child(fact(
            heronite_data::stats::group_thousands(st.sessions as u64),
            if st.sessions == 1 { tr!("session").into() } else { tr!("sessions").into() },
            false,
        ));
        div().mt_auto().pb(px(30.)).child(eyebrow(eyebrow_text, 0.7)).child(number).child(story)
    }

    pub(super) fn render_desktop(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let vw = f32::from(window.viewport_size().width);
        let panel_w = vw - 24.;
        let col_w = (panel_w - 2. * PAD).clamp(320., MAX_W);
        let wide = col_w >= 1100.;
        let st = self.stats.clone();
        let hero = self.render_hero(st.as_deref(), panel_w, col_w, window, cx);
        let body: Vec<AnyElement> = match (&self.history, &st) {
            (None, _) | (_, None) => vec![notice(Icon::BarChart, tr!("Reading your play history\u{2026}"), "")],
            (Some(h), Some(_)) if h.is_empty() => {
                vec![notice(
                    Icon::BarChart,
                    tr!("No play time yet"),
                    tr!("Play a game from the Collection and your statistics show up here."),
                )]
            }
            (Some(_), Some(st)) if st.total_secs <= 0. => {
                vec![notice(
                    Icon::AccessTime,
                    &tr!("Nothing played in {name}", name = self.period.label()),
                    tr!("Pick another period with the arrows above."),
                )]
            }
            (Some(_), Some(st)) => match self.tab {
                Tab::Overview => self.render_sections(st, col_w, wide, window, cx),
                Tab::Journeys => self.render_journeys(st, window, cx),
                Tab::Backlog => self.render_backlog(st, col_w, wide, window, cx),
                Tab::Discoveries => self.render_discoveries(st, col_w, window, cx),
                Tab::Album => self.render_album(st, col_w, wide, window, cx),
            },
        };
        // Blurred art of the number one game under the panels.
        let backdrop = st.as_ref().and_then(|s| s.games.first()).and_then(|g| self.blurred(g.game, panel_w / 4., 300.));
        let page = theme::stats_page_bg();
        let ink = |a: f32| Hsla { a, ..Hsla::from(page) };
        let mut content = div().relative().flex().flex_col().w_full().pb(px(36.));
        if let Some(src) = backdrop {
            content = content.child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(HERO_H))
                    .bottom_0()
                    .child(img(src).size_full().object_fit(ObjectFit::Cover).opacity(0.5))
                    .child(div().absolute().left_0().right_0().top_0().h(px(330.)).bg(linear_gradient(
                        180.,
                        linear_color_stop(ink(1.), 0.),
                        linear_color_stop(ink(0.74), 1.),
                    )))
                    .child(div().absolute().left_0().right_0().top(px(330.)).bottom_0().bg(ink(0.8))),
            );
        }
        content =
            content.child(hero).child(div().relative().flex().flex_col().gap(px(GAP)).w(px(col_w)).mx_auto().pt(px(GAP)).children(body));
        let bar = (-f32::from(self.scroll.offset().y) > BAR_AT).then(|| self.render_bar(col_w, cx));
        let tip = st.as_ref().and_then(|st| self.render_tip(st, window));
        let sheet = self.render_sheet(window, cx);
        let lightbox = self.render_lightbox(window, cx);
        div()
            .size_full()
            .p(px(12.))
            .pt(px(2.))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                if this.lightbox.is_some() {
                    match ev.keystroke.key.as_str() {
                        "escape" => {
                            this.lightbox = None;
                            cx.notify();
                        }
                        "left" => this.step_photo(-1, cx),
                        "right" => this.step_photo(1, cx),
                        _ => return,
                    }
                    cx.stop_propagation();
                } else if ev.keystroke.key == "escape" && this.sheet.is_some() {
                    this.close_sheet(cx);
                    cx.stop_propagation();
                }
            }))
            .on_mouse_move(cx.listener(|this, _, _, cx| {
                if this.hover.is_some() {
                    cx.notify();
                }
            }))
            .child(
                div()
                    .relative()
                    .size_full()
                    .rounded(px(theme::R_SHEET))
                    .overflow_hidden()
                    .bg(page)
                    .border_1()
                    .border_color(theme::glass_border())
                    .text_color(theme::text_primary())
                    .font_family(theme::font_body())
                    .child(div().id("stats-page").size_full().overflow_y_scroll().track_scroll(&self.scroll).child(content))
                    .children(bar)
                    .child(page_close(cx).absolute().top(px(20.)).right(px(20.)))
                    .children(sheet)
                    .children(lightbox),
            )
            .children(tip.filter(|_| self.sheet.is_none() && self.lightbox.is_none()))
            .into_any_element()
    }
}

/// "**213** of 284 days" (bold number, dimmer rest); `rev` puts the label first.
fn fact(strong: String, rest: String, rev: bool) -> Div {
    let b = div().font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(strong);
    let d = div().flex().items_baseline().gap(px(5.)).whitespace_nowrap();
    if rev {
        d.child(div().child(format!("{rest}:"))).child(b)
    } else {
        d.child(b).child(div().child(rest))
    }
}

fn notice(ic: Icon, title: &str, body: &str) -> AnyElement {
    panel()
        .items_center()
        .gap(px(8.))
        .py(px(56.))
        .child(icon(ic, px(28.), theme::text_dim()))
        .child(div().text_size(px(15.)).text_color(theme::text_primary()).child(title.to_string()))
        .child(div().text_size(px(13.)).text_color(theme::text_dim()).child(body.to_string()))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::total_parts;

    #[test]
    fn totals_split_hours_and_minutes() {
        assert_eq!(total_parts(711. * 3600. + 1200., false), ("711h".into(), Some("20m".into())));
        assert_eq!(total_parts(38. * 60., false), ("38m".into(), None));
        assert_eq!(total_parts(1401.6 * 3600., true), ("1,402h".into(), None));
    }
}
