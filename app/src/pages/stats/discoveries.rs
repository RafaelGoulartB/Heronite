//! The Discoveries tab: the period told in sentences instead of charts. The
//! most surprising fact leads (as the hero), the rest follow as cards that can
//! be pinned or hidden by kind and that link to where the numbers come from;
//! then the milestones of the whole history and memories of a year before.

use std::sync::Arc;

use chrono::{Datelike, Duration};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::stats::{
    compute, fmt_hours_short, fmt_secs, group_thousands, year_before, Fact, Insight, Milestone, MilestoneKind, Period, Span, Stats,
};
use heronite_i18n::{tr, trc, trn};

use super::desktop::{eyebrow, hint, panel, panel_head, GAP};
use super::journeys::Filter;
use super::review::marked;
use super::tabs::Tab;
use super::{day_label, full_date, in_text, short_date, today, weekday_plural, StatsPage};
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// Where a discovery's "See" link goes.
#[derive(Clone, Copy)]
enum Link {
    Game(usize),
    Tab(Tab),
    NewGames,
}

impl StatsPage {
    /// Discoveries of the period, pinned kinds first, hidden kinds left out.
    pub(super) fn shown_insights(&self) -> Vec<Insight> {
        let Some(all) = self.insights.clone() else { return Vec::new() };
        let hidden = |i: &Insight| self.prefs.hidden.iter().any(|k| k == i.fact.kind());
        let pinned = |i: &Insight| self.prefs.pinned.iter().any(|k| k == i.fact.kind());
        let mut v: Vec<Insight> = all.iter().filter(|i| self.show_hidden || !hidden(i)).cloned().collect();
        v.sort_by_key(|i| !pinned(i));
        v
    }

    /// The game whose art leads the tab.
    pub(super) fn lead_game(&self) -> Option<usize> {
        self.shown_insights().first().and_then(|i| i.fact.game())
    }

    /// The sentence of `f`, with `**bold**` runs.
    pub(super) fn insight_sentence(&self, f: &Fact) -> String {
        self.insight_text(f)
    }

    fn insight_text(&self, f: &Fact) -> String {
        let title = |g: usize| self.game_title(g);
        match f {
            Fact::Return { game, days, back, .. } => {
                tr!("Back to **{game}** after **{days} days**, on {day}", game = title(*game), days, day = short_date(*back))
            }
            Fact::Longest { game, secs, start, ratio } => tr!(
                "Longest session: **{time}** of **{game}** on {day} ({ratio}\u{d7} your average)",
                time = fmt_secs(*secs),
                game = title(*game),
                day = day_label(start.date()),
                ratio = heronite_i18n::decimal(*ratio, 1)
            ),
            Fact::Rising { game, secs, prev_secs } => {
                tr!(
                    "**{game}** grew the most: **{time}**, from {before}",
                    game = title(*game),
                    time = fmt_secs(*secs),
                    before = fmt_secs(*prev_secs)
                )
            }
            Fact::NewGames { n, prev, secs } => match prev {
                Some(p) if *p > 0 => tr!(
                    "You started **{n} new games** ({prev} before) and gave them **{time}**",
                    n,
                    prev = p,
                    time = fmt_hours_short(*secs)
                ),
                _ => tr!("You started **{n} new games** and gave them **{time}**", n, time = fmt_hours_short(*secs)),
            },
            Fact::Pause { days, last, back } => tr!(
                "The longest pause: **{days} days** without playing, {from} to {to}",
                days,
                from = short_date(*last),
                to = short_date(*back)
            ),
            Fact::Weekday { day, share } => {
                tr!("{days} are sacred: **{pct}%** of your time", days = weekday_plural(*day), pct = format!("{:.0}", share * 100.))
            }
            Fact::Streak { days, from, to, game } => match game {
                Some(g) => tr!(
                    "**{days} days in a row** playing, {from} to {to}, mostly **{game}**",
                    days,
                    from = short_date(*from),
                    to = short_date(*to),
                    game = title(*g)
                ),
                None => tr!("**{days} days in a row** playing, {from} to {to}", days, from = short_date(*from), to = short_date(*to)),
            },
            Fact::Marathon { day, secs, games } => tr!(
                "{day}: **{time}** in a single day, {games}",
                day = day_label(*day),
                time = fmt_secs(*secs),
                games = trn!(*games, "{n} game", "{n} games")
            ),
            Fact::Owl { sessions, share } => {
                tr!("Night owl: **{n} sessions** started after midnight ({pct}%)", n = sessions, pct = format!("{:.0}", share * 100.))
            }
            Fact::Focus { games, of } => tr!("**{n} games** hold 80% of your time, out of {of}", n = games, of),
            Fact::Compare { secs, prev_secs } => {
                let change = (secs / prev_secs - 1.) * 100.;
                let name = self.period.prev().map(|p| p.short_name()).unwrap_or_default();
                if change >= 0. {
                    tr!("You played **{pct}% more** than {name}: {time}", pct = format!("{change:.0}"), name, time = fmt_hours_short(*secs))
                } else {
                    tr!(
                        "You played **{pct}% less** than {name}: {time}",
                        pct = format!("{:.0}", -change),
                        name,
                        time = fmt_hours_short(*secs)
                    )
                }
            }
        }
    }

    fn insight_kind(f: &Fact) -> (Icon, &'static str) {
        match f {
            Fact::Return { .. } => (Icon::Replay, tr!("Return")),
            Fact::Longest { .. } => (Icon::AccessTime, tr!("Record")),
            Fact::Rising { .. } => (Icon::ArrowUpward, tr!("Rising")),
            Fact::NewGames { .. } => (Icon::Add, tr!("New games")),
            Fact::Pause { .. } => (Icon::Pause, trc!("stats", "Pause")),
            Fact::Weekday { .. } => (Icon::Event, tr!("Habit")),
            Fact::Streak { .. } => (Icon::History, tr!("Streak")),
            Fact::Marathon { .. } => (Icon::SportsEsports, tr!("Marathon")),
            Fact::Owl { .. } => (Icon::Moon, tr!("Night owl")),
            Fact::Focus { .. } => (Icon::Star, tr!("Focus")),
            Fact::Compare { .. } => (Icon::BarChart, tr!("Compared")),
        }
    }

    fn insight_link(f: &Fact) -> Link {
        match f {
            Fact::NewGames { .. } => Link::NewGames,
            Fact::Return { game, .. } | Fact::Longest { game, .. } | Fact::Rising { game, .. } => Link::Game(*game),
            Fact::Focus { .. } => Link::Tab(Tab::Journeys),
            _ => Link::Tab(Tab::Overview),
        }
    }

    fn follow(&mut self, link: Link, window: &mut Window, cx: &mut Context<Self>) {
        match link {
            Link::Game(g) => self.open_sheet(g, window, cx),
            Link::Tab(t) => self.set_tab(t, cx),
            Link::NewGames => {
                self.jv.filter = Filter::New;
                self.set_tab(Tab::Journeys, cx);
            }
        }
    }

    fn toggle_kind(&mut self, kind: &'static str, pin: bool, cx: &mut Context<Self>) {
        let list = if pin { &mut self.prefs.pinned } else { &mut self.prefs.hidden };
        match list.iter().position(|k| k == kind) {
            Some(i) => {
                list.remove(i);
            }
            None => list.push(kind.to_string()),
        }
        self.save_prefs(cx);
        cx.notify();
    }

    fn link_button(&self, id: impl Into<ElementId>, link: Link, cx: &mut Context<Self>) -> Stateful<Div> {
        let label = match link {
            Link::Game(_) => tr!("Open game sheet"),
            Link::Tab(Tab::Journeys) | Link::NewGames => tr!("See in Journeys"),
            Link::Tab(_) => tr!("See in Overview"),
        };
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(px(4.))
            .text_size(px(12.5))
            .font_weight(theme::W_BOLD)
            .text_color(theme::accent())
            .cursor_pointer()
            .hover(|s| s.opacity(0.8))
            .on_click(cx.listener(move |this, _, window, cx| this.follow(link, window, cx)))
            .child(label)
            .child(icon(Icon::ChevronRight, px(13.), theme::accent()))
    }

    /// Pin or hide a kind of discovery; `compact` draws the icon alone.
    fn kind_button(
        &self,
        id: String,
        ic: Icon,
        on: bool,
        kind: &'static str,
        pin: bool,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let tip = if pin {
            if on {
                tr!("Unpin")
            } else {
                tr!("Pin this kind")
            }
        } else if on {
            tr!("Show this kind")
        } else {
            tr!("Do not show this kind")
        };
        div()
            .id(SharedString::from(id))
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(28.))
            .px(px(if compact { 8. } else { 10. }))
            .rounded_full()
            .text_size(px(12.))
            .bg(if on { theme::white_a(0.16) } else { theme::white_a(0.06) })
            .text_color(if on { theme::text_primary() } else { theme::text_muted() })
            .cursor_pointer()
            .hover(|s| s.bg(theme::white_a(0.12)))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_kind(kind, pin, cx)))
            .child(icon(ic, px(13.), theme::text_muted()))
            .when(!compact, |d| d.child(tip))
    }

    /// Hero of the tab: the lead discovery, and a year ago on the side.
    pub(super) fn render_discoveries_total(&mut self, st: &Stats, window: &Window, cx: &mut Context<Self>) -> Div {
        let shown = self.shown_insights();
        let period = if st.period.span == Span::All { tr!("all time").to_string() } else { st.period.label() };
        let eyebrow_text = tr!("Discoveries \u{b7} {period} \u{b7} {n} things it told", period, n = shown.len());
        let mut main = div().flex_1().min_w_0().child(eyebrow(eyebrow_text, 0.7));
        if let Some(lead) = shown.first() {
            let kind = lead.fact.kind();
            let (ic, label) = Self::insight_kind(&lead.fact);
            let pinned = self.prefs.pinned.iter().any(|k| k == kind);
            main = main
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .mt(px(14.))
                        .text_size(px(12.5))
                        .font_weight(theme::W_BOLD)
                        .text_color(theme::accent())
                        .child(icon(ic, px(15.), theme::accent()))
                        .child(tr!("{kind} \u{b7} the biggest surprise", kind = label)),
                )
                .child(
                    div()
                        .mt(px(8.))
                        .max_w(px(900.))
                        .font_family(theme::font_heading())
                        .font_weight(theme::W_BOLD)
                        .text_size(px(40.))
                        .line_height(px(48.))
                        .child(marked(self.insight_text(&lead.fact))),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .mt(px(18.))
                        .child(self.kind_button(format!("lead-pin-{kind}"), Icon::Bookmark, pinned, kind, true, false, cx))
                        .child(self.kind_button(format!("lead-hide-{kind}"), Icon::VisibilityOff, false, kind, false, false, cx))
                        .child(self.link_button("lead-link", Self::insight_link(&lead.fact), cx)),
                );
        } else {
            main = main.child(div().mt(px(14.)).text_size(px(20.)).child(tr!("Not enough play in this period for a discovery.")));
        }
        let side = self.year_ago_card(st, window, cx);
        div().mt_auto().pb(px(30.)).flex().items_end().gap(px(24.)).child(main).children(side)
    }

    /// The same span a year before, if it had play.
    fn year_ago_card(&mut self, st: &Stats, window: &Window, cx: &mut Context<Self>) -> Option<Div> {
        let h = self.history.clone()?;
        let before = year_before(st.period);
        if before == st.period || h.since.is_none_or(|s| s >= before.end) {
            return None;
        }
        let then = compute(&h, before, today());
        if then.total_secs <= 0. {
            return None;
        }
        let covers: Vec<AnyElement> = then
            .games
            .iter()
            .take(3)
            .map(|g| {
                let game = g.game;
                let src = self.cover(game, 42., 56., window);
                div()
                    .id(("ago", game))
                    .w(px(42.))
                    .h(px(56.))
                    .rounded(px(6.))
                    .overflow_hidden()
                    .bg(theme::white_a(0.06))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| this.open_sheet(game, window, cx)))
                    .children(src.map(|s| img(s).size_full().object_fit(ObjectFit::Cover)))
                    .into_any_element()
            })
            .collect();
        let top = then.games.first().map(|g| self.game_title(g.game)).unwrap_or_default();
        Some(
            div()
                .flex_none()
                .w(px(360.))
                .flex()
                .flex_col()
                .gap(px(10.))
                .p(px(18.))
                .rounded(px(20.))
                .bg(rgba(0x0e1218d9))
                .border_1()
                .border_color(theme::white_a(0.1))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(px(12.))
                        .text_color(theme::text_muted())
                        .child(icon(Icon::History, px(14.), theme::text_muted()))
                        .child(tr!("A year before: {period}", period = before.label())),
                )
                .child(div().text_size(px(14.)).child(marked(tr!(
                    "**{time}** played, **{top}** in front",
                    time = fmt_hours_short(then.total_secs),
                    top
                ))))
                .child(div().flex().gap(px(8.)).children(covers)),
        )
    }

    pub(super) fn render_discoveries(&mut self, st: &Stats, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        vec![self.discovery_feed(window, cx), self.discovery_milestones(st, window), self.discovery_memories(window, cx)]
    }

    fn discovery_feed(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let shown = self.shown_insights();
        let hidden_n =
            self.insights.as_ref().map_or(0, |all| all.iter().filter(|i| self.prefs.hidden.iter().any(|k| k == i.fact.kind())).count());
        let mut cards = Vec::new();
        for (i, ins) in shown.iter().enumerate().skip(1) {
            let kind = ins.fact.kind();
            let (ic, label) = Self::insight_kind(&ins.fact);
            let pinned = self.prefs.pinned.iter().any(|k| k == kind);
            let hidden = self.prefs.hidden.iter().any(|k| k == kind);
            let cover = ins.fact.game().and_then(|g| self.cover(g, 40., 54., window));
            cards.push(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .p(px(20.))
                    .rounded(px(18.))
                    .bg(rgba(0x161b24d6))
                    .border_1()
                    .border_color(if pinned { Hsla { a: 0.4, ..Hsla::from(theme::accent()) } } else { theme::white_a(0.08) })
                    .when(hidden, |d| d.opacity(0.5))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .text_size(px(11.5))
                            .font_weight(theme::W_BOLD)
                            .text_color(theme::text_muted())
                            .child(icon(ic, px(14.), theme::text_muted()))
                            .child(label.to_uppercase())
                            .child(div().flex_1())
                            .child(self.kind_button(format!("pin-{i}"), Icon::Bookmark, pinned, kind, true, true, cx))
                            .child(self.kind_button(
                                format!("hide-{i}"),
                                if hidden { Icon::Visibility } else { Icon::VisibilityOff },
                                hidden,
                                kind,
                                false,
                                true,
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(14.))
                            .children(cover.map(|s| {
                                div()
                                    .flex_none()
                                    .w(px(40.))
                                    .h(px(54.))
                                    .rounded(px(6.))
                                    .overflow_hidden()
                                    .child(img(s).size_full().object_fit(ObjectFit::Cover))
                            }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .font_family(theme::font_heading())
                                    .font_weight(theme::W_BOLD)
                                    .text_size(px(17.))
                                    .line_height(px(23.))
                                    .child(marked(self.insight_text(&ins.fact))),
                            ),
                    )
                    .child(div().mt_auto().pt(px(10.)).border_t_1().border_color(theme::white_a(0.06)).child(self.link_button(
                        ("see", i),
                        Self::insight_link(&ins.fact),
                        cx,
                    ))),
            );
        }
        let toggle = (hidden_n > 0).then(|| {
            div()
                .id("show-hidden")
                .text_size(px(12.5))
                .font_weight(theme::W_BOLD)
                .text_color(theme::accent())
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.show_hidden = !this.show_hidden;
                    cx.notify();
                }))
                .child(if self.show_hidden {
                    tr!("Hide the hidden ones").to_string()
                } else {
                    trn!(hidden_n, "Show {n} hidden", "Show {n} hidden")
                })
        });
        let copy = div()
            .id("copy-summary")
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(30.))
            .px(px(12.))
            .rounded_full()
            .bg(theme::white_a(0.08))
            .text_size(px(12.5))
            .font_weight(theme::W_BOLD)
            .cursor_pointer()
            .hover(|s| s.bg(theme::white_a(0.14)))
            .on_click(cx.listener(|this, _, _, cx| {
                let text = this.discoveries_summary();
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                cx.emit(crate::pages::PageEvent::Toast(tr!("Summary copied").into(), false));
            }))
            .child(icon(Icon::ContentCopy, px(14.), theme::text_primary()))
            .child(tr!("Copy a summary"));
        let head = div().flex().items_center().gap(px(14.)).children(toggle).child(copy);
        let mut grid = div().grid().grid_cols(3).gap(px(GAP));
        for c in cards {
            grid = grid.child(c);
        }
        panel()
            .child(panel_head(tr!("More discoveries"), head))
            .child(div().mt(px(-6.)).mb(px(14.)).child(hint(tr!("Most surprising first \u{b7} pin a kind to keep it on top"))))
            .child(grid)
            .into_any_element()
    }

    /// The period in a few lines, for pasting anywhere.
    fn discoveries_summary(&self) -> String {
        let Some(st) = self.stats.clone() else { return String::new() };
        let period = if st.period.span == Span::All { tr!("all time").to_string() } else { st.period.label() };
        let mut lines = vec![tr!(
            "My {period} in games: {time}, {games} and {sessions}.",
            period,
            time = fmt_hours_short(st.total_secs),
            games = trn!(st.games.len(), "{n} game", "{n} games"),
            sessions = trn!(st.sessions, "{n} session", "{n} sessions")
        )];
        let top: Vec<String> =
            st.games.iter().take(3).map(|g| format!("{} ({})", self.game_title(g.game), fmt_hours_short(g.secs))).collect();
        if !top.is_empty() {
            lines.push(tr!("Most played: {list}.", list = top.join(", ")));
        }
        for ins in self.shown_insights().iter().take(3) {
            lines.push(self.insight_text(&ins.fact).replace("**", ""));
        }
        lines.join("\n")
    }

    fn discovery_milestones(&mut self, st: &Stats, window: &Window) -> AnyElement {
        let Some(all) = self.milestones.clone() else { return div().into_any_element() };
        let value_text = |kind: MilestoneKind, v: u32| match kind {
            MilestoneKind::Hours | MilestoneKind::GameHours => tr!("{h}h", h = group_thousands(v as u64)),
            _ => group_thousands(v as u64),
        };
        let label = |m: &Milestone, title: String| match m.kind {
            MilestoneKind::Hours => tr!("played in all").to_string(),
            MilestoneKind::Sessions => tr!("sessions").to_string(),
            MilestoneKind::Games => tr!("different games").to_string(),
            MilestoneKind::GameHours => tr!("in {game}", game = title),
            MilestoneKind::Streak => tr!("days in a row").to_string(),
        };
        let mut badges = Vec::new();
        let mut next = Vec::new();
        for m in all.iter() {
            let title = m.game.map(|g| self.game_title(g)).unwrap_or_default();
            if let Some((step, day)) = m.reached {
                // All time holds every day: nothing is new there.
                let new = st.period.span != Span::All && st.period.contains(day);
                let art = m.game.filter(|_| m.kind == MilestoneKind::GameHours).and_then(|g| self.cover(g, 52., 52., window));
                let ic = match m.kind {
                    MilestoneKind::Hours => Icon::AccessTime,
                    MilestoneKind::Sessions => Icon::PlayArrow,
                    MilestoneKind::Games => Icon::Collections,
                    MilestoneKind::GameHours => Icon::Star,
                    MilestoneKind::Streak => Icon::History,
                };
                badges.push(
                    div()
                        .relative()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(6.))
                        .p(px(18.))
                        .rounded(px(18.))
                        .bg(rgba(0x161b24d6))
                        .border_1()
                        .border_color(if new { Hsla { a: 0.45, ..Hsla::from(theme::accent()) } } else { theme::white_a(0.08) })
                        .child(
                            div()
                                .size(px(52.))
                                .rounded_full()
                                .overflow_hidden()
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(theme::white_a(0.06))
                                .border_2()
                                .border_color(theme::white_a(0.14))
                                .child(match art {
                                    Some(s) => img(s).size_full().object_fit(ObjectFit::Cover).into_any_element(),
                                    None => icon(ic, px(24.), theme::accent()).into_any_element(),
                                }),
                        )
                        .child(
                            div()
                                .font_family(theme::font_heading())
                                .font_weight(theme::W_BOLD)
                                .text_size(px(22.))
                                .child(value_text(m.kind, step)),
                        )
                        .child(div().text_size(px(12.)).text_color(theme::text_muted()).text_center().child(label(m, title.clone())))
                        .child(div().text_size(px(11.5)).text_color(theme::text_dim()).child(full_date(day)))
                        .when(new, |d| {
                            d.child(
                                div().absolute().top(px(10.)).right(px(10.)).child(super::super::status_pill(tr!("New"), theme::accent())),
                            )
                        }),
                );
            }
            if let Some(n) = m.next {
                let pct = (m.value / n as f64).clamp(0., 1.) as f32;
                let eta = (m.recent > 0. && m.kind != MilestoneKind::Streak).then(|| {
                    let days = ((n as f64 - m.value) / (m.recent / 90.)).ceil() as i64;
                    let when = today() + Duration::days(days);
                    if days <= 60 {
                        trn!(days as usize, "at your pace, in ~{n} day", "at your pace, in ~{n} days")
                    } else {
                        tr!(
                            "at your pace, around {month} {year}",
                            month = in_text(heronite_data::stats::month_name(when.month())),
                            year = when.year()
                        )
                    }
                });
                next.push(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .p(px(16.))
                        .rounded(px(16.))
                        .bg(theme::white_a(0.035))
                        .border_1()
                        .border_color(theme::white_a(0.06))
                        .child(
                            div()
                                .flex()
                                .justify_between()
                                .text_size(px(13.))
                                .child(div().font_weight(theme::W_BOLD).child(format!("{} {}", value_text(m.kind, n), label(m, title))))
                                .child(div().text_color(theme::text_muted()).child(format!(
                                    "{} / {}",
                                    group_thousands(m.value as u64),
                                    group_thousands(n as u64)
                                ))),
                        )
                        .child(
                            div()
                                .h(px(6.))
                                .rounded_full()
                                .bg(theme::white_a(0.08))
                                .child(div().h_full().w(relative(pct)).rounded_full().bg(theme::accent())),
                        )
                        .children(eta.map(|e| div().text_size(px(12.)).text_color(theme::text_dim()).child(e))),
                );
            }
        }
        let reached = badges.len();
        panel()
            .child(panel_head(tr!("Milestones"), hint(trn!(reached, "{n} reached in your history", "{n} reached in your history"))))
            .child(div().grid().grid_cols(5).gap(px(GAP)).children(badges))
            .child(div().mt(px(18.)).mb(px(10.)).text_size(px(13.)).text_color(theme::text_muted()).child(tr!("Next milestones")))
            .child(div().grid().grid_cols(5).gap(px(GAP)).children(next))
            .into_any_element()
    }

    fn discovery_memories(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(h) = self.history.clone() else { return div().into_any_element() };
        let t = today();
        let mut cards = Vec::new();
        // This month a year ago.
        let month = Period::of(Span::Month, t - Duration::days(365));
        let then = compute(&h, month, t);
        if then.total_secs > 0. {
            let covers: Vec<AnyElement> = then
                .games
                .iter()
                .take(5)
                .map(|g| {
                    let game = g.game;
                    let src = self.cover(game, 48., 64., window);
                    div()
                        .id(("mem-month", game))
                        .w(px(48.))
                        .h(px(64.))
                        .rounded(px(6.))
                        .overflow_hidden()
                        .bg(theme::white_a(0.06))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| this.open_sheet(game, window, cx)))
                        .children(src.map(|s| img(s).size_full().object_fit(ObjectFit::Cover)))
                        .into_any_element()
                })
                .collect();
            cards.push(memory(
                tr!("This month in {year}", year = month.start.year()),
                marked(tr!(
                    "**{period}**: {time} in {games}",
                    period = month.label(),
                    time = fmt_hours_short(then.total_secs),
                    games = trn!(then.games.len(), "{n} game", "{n} games")
                )),
                div().flex().gap(px(8.)).children(covers),
            ));
        }
        // Screenshots of the same week a year ago.
        let week = Period::of(Span::Week, t - Duration::days(364));
        let shots: Vec<(usize, std::path::PathBuf)> = self
            .shots
            .clone()
            .map(|all| {
                let mut v: Vec<(usize, std::path::PathBuf, chrono::NaiveDateTime)> = all
                    .iter()
                    .flat_map(|(g, l)| l.iter().filter(|s| week.contains(s.at.date())).map(move |s| (*g, s.path.clone(), s.at)))
                    .collect();
                v.sort_by_key(|x| x.2);
                v.into_iter().take(4).map(|x| (x.0, x.1)).collect()
            })
            .unwrap_or_default();
        if !shots.is_empty() {
            let thumbs: Vec<Div> = shots
                .iter()
                .map(|(_, path)| {
                    let src = self.shot(path, 150., 84., window, cx);
                    div()
                        .w(px(150.))
                        .h(px(84.))
                        .rounded(px(8.))
                        .overflow_hidden()
                        .bg(theme::white_a(0.05))
                        .children(src.map(|s| img(s).size_full().object_fit(ObjectFit::Cover)))
                })
                .collect();
            cards.push(memory(
                tr!("That week, a year ago"),
                marked(tr!(
                    "**{n} moments** from {from} to {to}",
                    n = shots.len(),
                    from = short_date(week.start),
                    to = short_date(week.end - Duration::days(1))
                )),
                div().flex().flex_wrap().gap(px(8.)).children(thumbs),
            ));
        }
        // Where the history begins.
        if let Some(since) = h.since {
            let first = compute(&h, Period::of(Span::Month, since), t);
            let game = first.games.first().map(|g| self.game_title(g.game)).unwrap_or_default();
            let all = compute(&h, Period::of(Span::All, t), t);
            cards.push(memory(
                tr!("Where it all began"),
                marked(tr!("The first recorded session was on **{day}**, with **{game}**", day = full_date(since), game)),
                div().text_size(px(13.)).text_color(theme::text_muted()).child(tr!(
                    "Since then: {time}, {sessions} and {games}.",
                    time = fmt_hours_short(all.total_secs),
                    sessions = trn!(all.sessions, "{n} session", "{n} sessions"),
                    games = trn!(all.games.len(), "{n} game", "{n} games")
                )),
            ));
        }
        if cards.is_empty() {
            return div().into_any_element();
        }
        panel()
            .child(panel_head(tr!("Memories"), hint(tr!("What was going on before"))))
            .child(div().grid().grid_cols(3).gap(px(GAP)).children(cards))
            .into_any_element()
    }
}

fn memory(title: impl Into<String>, lead: StyledText, body: impl IntoElement) -> Div {
    let title: String = title.into();
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .p(px(20.))
        .rounded(px(18.))
        .bg(theme::white_a(0.035))
        .border_1()
        .border_color(theme::white_a(0.06))
        .child(div().text_size(px(11.5)).font_weight(theme::W_BOLD).text_color(theme::text_muted()).child(title.to_uppercase()))
        .child(div().font_family(theme::font_heading()).font_weight(theme::W_BOLD).text_size(px(16.)).child(lead))
        .child(body)
}

/// Discoveries and milestones for the page state.
pub(super) fn compute_for(h: &heronite_data::stats::History, st: &Stats) -> (Arc<Vec<Insight>>, Arc<Vec<Milestone>>) {
    (Arc::new(heronite_data::stats::insights(h, st)), Arc::new(heronite_data::stats::milestones(h, today())))
}
