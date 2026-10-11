//! Year in review ("Wrapped"): a year told in seven full-page stories over
//! the blurred art of its games: the intro, the big number, the number one
//! game, the top five, the habits, the records and a closing card with the
//! screenshots, the games beaten and a summary to copy.
//!
//! Drawn at a 1600×1000 design size scaled to the page. ← → (or the
//! controller's ◀ ▶), Esc / B closes.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{Datelike, Duration, NaiveDate};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::stats::{self, fmt_secs, month_name, month_short, History, Period, Span, Stats};
use heronite_data::Library;

use super::desktop::eyebrow;
use super::{derive, hours_text, plural, series_color, short_date, today, Derived, Shot, StatsPage, WEEKDAYS, WEEKDAYS_LONG};
use crate::console::motion;
use crate::icons::Icon;
use crate::pages::PageEvent;
use crate::theme;
use crate::ui::icon;

pub(super) const SLIDES: usize = 7;

thread_local! {
    static DEBUG_SLIDE: Cell<usize> = const { Cell::new(0) };
}

/// `HERONITE_DEBUG_PAGE=stats:year:review:N` opens on slide N (1-based).
pub(super) fn set_debug_slide(n: usize) {
    DEBUG_SLIDE.with(|s| s.set(n.clamp(1, SLIDES) - 1));
}

pub(super) struct Review {
    year: i32,
    st: Arc<Stats>,
    derived: Arc<Derived>,
    slide: usize,
    /// Slide shown before (its background stays under the new one while it fades in).
    prev: Option<usize>,
    /// Bumped on every change: restarts the entrance animations.
    epoch: usize,
    focused: bool,
}

impl Review {
    pub fn new(year: i32, h: &History, lib: Option<&Library>, shots: Option<&HashMap<usize, Vec<Shot>>>) -> Self {
        let mut r = Review {
            year,
            st: Arc::new(Stats::empty_like()),
            derived: Arc::default(),
            slide: DEBUG_SLIDE.with(|s| s.replace(0)),
            prev: None,
            epoch: 0,
            focused: false,
        };
        r.refresh(h, lib, shots);
        r
    }

    pub fn refresh(&mut self, h: &History, lib: Option<&Library>, shots: Option<&HashMap<usize, Vec<Shot>>>) {
        let Some(jan1) = NaiveDate::from_ymd_opt(self.year, 1, 1) else { return };
        let st = stats::compute(h, Period::of(Span::Year, jan1), today());
        self.derived = Arc::new(lib.map(|l| derive(&st, l, shots)).unwrap_or_default());
        self.st = Arc::new(st);
    }

    fn go(&mut self, slide: usize) {
        if slide != self.slide && slide < SLIDES {
            self.prev = Some(self.slide);
            self.slide = slide;
            self.epoch += 1;
        }
    }
}

trait EmptyStats {
    fn empty_like() -> Self;
}

impl EmptyStats for Stats {
    fn empty_like() -> Self {
        stats::compute(&History::default(), Period::of(Span::Year, today()), today())
    }
}

/// Design px scaled to the page (`.0`), and how far the 1600px stage sits
/// from the page's left edge, in design px (`.1`; art bleeds to the edges).
#[derive(Clone, Copy)]
struct Z(f32, f32);

impl Z {
    fn p(self, v: f32) -> Pixels {
        px(v * self.0)
    }
}

fn ease(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(4)
}

/// Fades `el` in while it rises 28 design px, after `delay` ms.
fn rise(id: impl Into<ElementId>, el: Div, delay: u64, z: Z) -> AnyElement {
    let el = el.relative();
    if !motion::on() {
        return el.into_any_element();
    }
    let total = delay + 900;
    let dy = z.p(28.);
    el.with_animation(id, Animation::new(StdDuration::from_millis(total)), move |el, t| {
        let local = ease((t * total as f32 - delay as f32) / 900.);
        el.opacity(local).top(dy * (1. - local))
    })
    .into_any_element()
}

/// A bar that grows to `h` (design px) from the bottom, after `delay` ms.
fn grow(id: impl Into<ElementId>, bar: Div, h: f32, delay: u64, z: Z) -> AnyElement {
    if !motion::on() {
        return bar.h(z.p(h)).into_any_element();
    }
    let total = delay + 1100;
    bar.h(z.p(h))
        .with_animation(id, Animation::new(StdDuration::from_millis(total)), move |el, t| {
            let local = ease((t * total as f32 - delay as f32) / 1100.);
            el.h(z.p(h * local))
        })
        .into_any_element()
}

fn heading(text: impl Into<SharedString>, size: f32, z: Z) -> Div {
    div().font_family(theme::FONT_HEADING).font_weight(theme::W_BOLD).text_size(z.p(size)).line_height(z.p(size * 1.02)).text_color(theme::text_primary()).child(text.into())
}

fn kick(text: &str, z: Z) -> Div {
    eyebrow(text, 0.72).text_size(z.p(14.))
}

fn lead(z: Z) -> Div {
    div().text_size(z.p(22.)).line_height(z.p(31.)).text_color(theme::white_a(0.82))
}

/// Plain run of a [`rich`] text.
const PLAIN: u8 = 0;
/// Bold, bright run.
const BOLD: u8 = 1;
/// Accent-coloured run.
const ACCENT: u8 = 2;

/// One paragraph with bold and accent runs (it wraps as a whole).
fn rich(parts: Vec<(String, u8)>) -> StyledText {
    let mut text = String::new();
    let mut runs = Vec::new();
    for (part, kind) in parts {
        let start = text.len();
        text.push_str(&part);
        let style = match kind {
            BOLD => HighlightStyle { font_weight: Some(theme::W_BOLD), color: Some(theme::text_primary().into()), ..Default::default() },
            ACCENT => HighlightStyle { color: Some(theme::accent().into()), ..Default::default() },
            _ => continue,
        };
        runs.push((start..text.len(), style));
    }
    StyledText::new(text).with_highlights(runs)
}

fn p(text: impl Into<String>) -> (String, u8) {
    (text.into(), PLAIN)
}

fn b(text: impl Into<String>) -> (String, u8) {
    (text.into(), BOLD)
}

fn two(a: String, c: String) -> Div {
    div().flex().flex_col().child(div().font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(a)).child(div().child(c))
}

fn glass_card(z: Z) -> Div {
    div().relative().rounded(z.p(22.)).bg(rgba(0x12161ec7)).border_1().border_color(theme::white_a(0.14)).shadow(vec![BoxShadow {
        color: theme::black_a(0.4),
        offset: point(px(0.), z.p(20.)),
        blur_radius: z.p(60.),
        spread_radius: px(0.),
        inset: false,
    }])
}

fn chip(z: Z) -> Div {
    div()
        .flex()
        .items_center()
        .gap(z.p(8.))
        .h(z.p(32.))
        .px(z.p(14.))
        .rounded_full()
        .bg(rgba(0x0a0c108c))
        .border_1()
        .border_color(theme::white_a(0.14))
        .font_family(theme::FONT_HEADING)
        .font_weight(theme::W_BOLD)
        .text_size(z.p(13.))
}

fn cover_img(src: Option<ImageSource>, w: f32, h: f32, r: f32, z: Z) -> Div {
    div().flex_none().w(z.p(w)).h(z.p(h)).rounded(z.p(r)).overflow_hidden().bg(theme::white_a(0.06)).children(src.map(|s| img(s).size_full().object_fit(ObjectFit::Cover)))
}

const PERSONA: [&str; 4] = ["Night owl", "Early bird", "Afternoon", "Evening"];
const PART_RANGE: [&str; 4] = ["between midnight and 6 am", "between 6 am and noon", "between noon and 6 pm", "between 6 pm and midnight"];

impl StatsPage {
    fn review_go(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(r) = self.review.as_mut() {
            let next = r.slide as isize + delta;
            if (0..SLIDES as isize).contains(&next) {
                r.go(next as usize);
                cx.notify();
            }
        }
    }

    /// Controller / keys while the review is open.
    pub(super) fn review_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        match key {
            "left" => self.review_go(-1, cx),
            "right" | "space" | "enter" => self.review_go(1, cx),
            "escape" => self.close_review(cx),
            "home" => {
                if let Some(r) = self.review.as_mut() {
                    r.go(0);
                    cx.notify();
                }
            }
            _ => return false,
        }
        true
    }

    fn summary_text(&self, r: &Review) -> String {
        let st = &r.st;
        let top: Vec<String> = st.games.iter().take(3).map(|g| format!("{} {}", self.game_title(g.game), hours_text(g.secs))).collect();
        let mut s = format!(
            "My {} in games: {} played on {} days, {} sessions, {} games ({} new).",
            r.year,
            fmt_secs(st.total_secs),
            st.days_played,
            st.sessions,
            st.games.len(),
            st.new_games()
        );
        if !top.is_empty() {
            s.push_str(&format!(" Top {}: {}.", top.len(), top.join(", ")));
        }
        if let Some(l) = st.longest {
            s.push_str(&format!(" Longest session: {}.", fmt_secs(l.secs)));
        }
        if st.streak > 1 {
            s.push_str(&format!(" Longest streak: {} days.", st.streak));
        }
        s
    }

    pub(super) fn render_review(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(r) = self.review.as_mut() else { return div().into_any_element() };
        if !r.focused {
            r.focused = true;
            self.focus.focus(window, cx);
        }
        let (year, slide, prev, epoch) = (r.year, r.slide, r.prev, r.epoch);
        let st = r.st.clone();
        let derived = r.derived.clone();
        let vp = window.viewport_size();
        let (w, h) = (f32::from(vp.width) - 24., f32::from(vp.height) - f32::from(theme::HEADER_H) - 14.);
        let k = (w / 1600.).min(h / 1000.).max(0.4);
        let z = Z(k, ((w - 1600. * k) / 2.).max(0.) / k);

        // Backgrounds: blurred art of each slide's game.
        let games: Vec<usize> = st.games.iter().map(|g| g.game).collect();
        let pick = |i: usize| games.get(i).or(games.first()).copied();
        let bg_game = |s: usize| match s {
            1 => pick(1),
            3 => pick(2),
            4 => st.streak_game.or(pick(3)),
            5 => st.longest.map(|l| l.game).or(pick(4)),
            _ => pick(0),
        };
        let page = theme::stats_page_bg();
        let mut root = div().relative().size_full().rounded(px(theme::R_SHEET)).overflow_hidden().bg(page).border_1().border_color(theme::glass_border()).text_color(theme::text_primary()).font_family(theme::FONT_BODY);
        let bg_layer = |s: usize, fade: bool, this: &mut StatsPage| -> Option<AnyElement> {
            let g = bg_game(s)?;
            let src = this.blurred(g, w / 4., h / 4.)?;
            let el = div().absolute().inset_0().child(img(src).size_full().object_fit(ObjectFit::Cover).opacity(0.85));
            Some(if fade { motion::fade_in(("review-bg", epoch), el, 1200) } else { el.into_any_element() })
        };
        if let Some(p) = prev {
            if let Some(el) = bg_layer(p, false, self) {
                root = root.child(el);
            }
        }
        if let Some(el) = bg_layer(slide, true, self) {
            root = root.child(el);
        }
        let ink = |a: f32| Hsla { a, ..Hsla::from(page) };
        root = root
            .child(div().absolute().left_0().right_0().top_0().h(relative(0.4)).bg(linear_gradient(180., linear_color_stop(ink(0.5), 0.), linear_color_stop(ink(0.08), 1.))))
            .child(div().absolute().left_0().right_0().top(relative(0.4)).bottom_0().bg(linear_gradient(180., linear_color_stop(ink(0.08), 0.), linear_color_stop(ink(0.62), 1.))));

        let body = match slide {
            0 => self.slide_intro(&st, year, z, window),
            1 => self.slide_number(&st, year, z, epoch),
            2 => self.slide_top(&st, year, z, epoch, window),
            3 => self.slide_podium(&st, z, epoch, window),
            4 => self.slide_habits(&st, z, epoch),
            5 => self.slide_records(&st, z, epoch, window),
            _ => self.slide_wrap(&st, &derived, year, z, epoch, window, cx),
        };
        // The content area (design 1600×1000) centred in the page.
        let stage = div().absolute().top_0().bottom_0().left(px(((w - 1600. * z.0) / 2.).max(0.))).w(z.p(1600.)).child(body);
        let stage = if motion::on() {
            let dx = z.p(40.) * if prev.is_some_and(|p| p > slide) { -1. } else { 1. };
            stage
                .with_animation(("review-slide", epoch), Animation::new(StdDuration::from_millis(800)), move |el, t| {
                    let e = ease(t);
                    el.opacity(e).ml(dx * (1. - e))
                })
                .into_any_element()
        } else {
            stage.into_any_element()
        };
        root = root.child(stage);

        // Chrome: progress, title, close, arrows, footer.
        let progress = div().absolute().left(px(36.)).right(px(36.)).top(px(26.)).flex().gap(px(6.)).children((0..SLIDES).map(|i| {
            let fill = if i < slide { theme::white_a(0.9) } else if i == slide { theme::accent().into() } else { theme::white_a(0.2) };
            div()
                .id(("review-dot", i))
                .flex_1()
                .h(px(18.))
                .flex()
                .items_center()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(r) = this.review.as_mut() {
                        r.go(i);
                        cx.notify();
                    }
                }))
                .child(div().w_full().h(px(4.)).rounded(px(2.)).bg(fill))
        }));
        let head = div()
            .absolute()
            .left(px(36.))
            .right(px(36.))
            .top(px(56.))
            .flex()
            .items_center()
            .justify_between()
            .text_size(px(13.))
            .text_color(theme::text_muted())
            .child(div().flex().gap(px(8.)).child(div().font_family(theme::FONT_HEADING).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child("Heronite")).child(format!("{year} in review")))
            .child(
                div()
                    .id("review-close")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(34.))
                    .rounded_full()
                    .bg(rgba(0x161b24d6))
                    .border_1()
                    .border_color(theme::white_a(0.08))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::white_a(0.14)))
                    .on_click(cx.listener(|this, _, _, cx| this.close_review(cx)))
                    .child(icon(Icon::Close, px(15.), theme::text_primary())),
            );
        let arrow = |id: &'static str, ic: Icon, enabled: bool, delta: isize, cx: &mut Context<Self>| {
            div()
                .id(id)
                .absolute()
                .top(relative(0.5))
                .mt(px(-26.))
                .size(px(52.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(rgba(0x161b24d6))
                .border_1()
                .border_color(theme::white_a(0.14))
                .when(!enabled, |d| d.opacity(0.25))
                .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(Hsla { a: 0.2, ..Hsla::from(theme::accent()) })))
                .on_click(cx.listener(move |this, _, _, cx| this.review_go(delta, cx)))
                .child(icon(ic, px(22.), theme::text_primary()))
        };
        let foot = div()
            .absolute()
            .right(px(36.))
            .bottom(px(16.))
            .flex()
            .items_center()
            .gap(px(10.))
            .text_size(px(12.))
            .text_color(theme::text_dim())
            .child(keycap("\u{2190}"))
            .child(keycap("\u{2192}"))
            .child(format!("{} / {SLIDES}", slide + 1));
        root = root
            .child(div().absolute().left_0().right_0().top_0().h(px(150.)).bg(linear_gradient(180., linear_color_stop(ink(0.85), 0.), linear_color_stop(ink(0.), 1.))))
            .child(progress)
            .child(head)
            .child(arrow("review-prev", Icon::ChevronLeft, slide > 0, -1, cx).left(px(22.)))
            .child(arrow("review-next", Icon::ChevronRight, slide + 1 < SLIDES, 1, cx).right(px(22.)))
            .child(foot);
        div()
            .size_full()
            .p(px(12.))
            .pt(px(2.))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                if this.review_key(ev.keystroke.key.as_str(), cx) {
                    cx.stop_propagation();
                }
            }))
            .child(root)
            .into_any_element()
    }

    /// Text column of a slide: `left` and `top` design px into the stage.
    fn col(z: Z, left: f32, top: f32, w: Option<f32>) -> Div {
        let d = div().absolute().left(z.p(left)).top(z.p(top)).flex().flex_col();
        match w {
            Some(w) => d.w(z.p(w)),
            None => d.right(z.p(left)),
        }
    }

    fn slide_intro(&mut self, st: &Stats, year: i32, z: Z, window: &Window) -> AnyElement {
        let mut stage = div().relative().size_full();
        // Three drifting columns of covers.
        let picks: Vec<usize> = st.games.iter().take(9).map(|g| g.game).collect();
        let cols = [(930., -30., [0, 3, 6], 9000u64), (1180., 110., [1, 4, 7], 11000), (1430., -60., [2, 5, 8], 13000)];
        for (ci, (x, y, ix, period)) in cols.into_iter().enumerate() {
            let mut column = div().absolute().left(z.p(x)).top(z.p(y)).flex().flex_col().gap(z.p(22.));
            for i in ix {
                let Some(&g) = picks.get(i) else { continue };
                let src = self.cover(g, 230. * z.0, 307. * z.0, window);
                column = column.child(
                    cover_img(src, 230., 307., 18., z)
                        .border_1()
                        .border_color(theme::white_a(0.1))
                        .shadow(vec![BoxShadow { color: theme::black_a(0.6), offset: point(px(0.), z.p(24.)), blur_radius: z.p(60.), spread_radius: px(0.), inset: false }]),
                );
            }
            let column = if motion::on() {
                let amp = z.p(if ci == 1 { -16. } else { 18. });
                column
                    .with_animation(("review-drift", ci), Animation::new(StdDuration::from_millis(period)).repeat(), move |el, t| {
                        el.mt(amp * (t * std::f32::consts::TAU).sin())
                    })
                    .into_any_element()
            } else {
                column.into_any_element()
            };
            stage = stage.child(column);
        }
        let ink = |a: f32| Hsla { a, ..Hsla::from(theme::stats_page_bg()) };
        stage = stage
            .child(div().absolute().left(-z.p(z.1)).top_0().bottom_0().w(z.p(550. + z.1)).bg(ink(0.8)))
            .child(div().absolute().left(z.p(550.)).top_0().bottom_0().w(z.p(450.)).bg(linear_gradient(90., linear_color_stop(ink(0.8), 0.), linear_color_stop(ink(0.), 1.))));
        let end = if year == today().year() { today() } else { st.period.end - Duration::days(1) };
        let mut text = Self::col(z, 110., 170., Some(760.));
        text = text
            .child(rise("i-kick", kick("Year in review", z), 0, z))
            .child(rise("i-h1", div().mt(z.p(28.)).child(heading("Your", 150., z)).child(heading(year.to_string(), 150., z)).child(heading("in games", 150., z).text_color(theme::accent())), 120, z))
            .child(rise(
                "i-lead",
                lead(z).mt(z.p(34.)).child(rich(vec![
                    p(format!("{} to {}: ", short_date(st.period.start), short_date(end))),
                    b(plural(st.days_elapsed, "day", "days")),
                    p(", "),
                    b(plural(st.games.len(), "game", "games")),
                    p(" and "),
                    b(plural(st.sessions, "session", "sessions")),
                    p("."),
                ])),
                360,
                z,
            ))
            .child(rise("i-hint", div().mt(z.p(34.)).flex().items_center().gap(z.p(12.)).text_size(z.p(14.)).text_color(theme::text_muted()).child(keycap("\u{2192}")).child("Press next to start"), 520, z));
        stage.child(text).into_any_element()
    }

    fn slide_number(&mut self, st: &Stats, year: i32, z: Z, epoch: usize) -> AnyElement {
        let hours = (st.total_secs / 3600.).floor() as u64;
        let days = st.total_secs / 86400.;
        let (dd, hh) = (days.floor() as u64, ((days - days.floor()) * 24.).round() as u64);
        let ongoing = year == today().year();
        let compare = match st.prev_secs.filter(|p| *p > 0.) {
            Some(prev) => {
                let c = (st.total_secs / prev - 1.) * 100.;
                let sign = if c >= 0. { "+" } else { "" };
                format!("{sign}{c:.0}% on {} ({}){}", year - 1, hours_text(prev), if ongoing { ", and the year is not over." } else { "." })
            }
            None => "Your first year on record.".into(),
        };
        let card = glass_card(z)
            .w(z.p(480.))
            .px(z.p(30.))
            .py(z.p(26.))
            .child(
                div().font_family(theme::FONT_HEADING).font_weight(theme::W_BOLD).text_size(z.p(32.)).line_height(z.p(38.)).child(rich(vec![
                    p("That is "),
                    (format!("{} and {}", plural(dd as usize, "day", "days"), plural(hh as usize, "hour", "hours")), ACCENT),
                    p(" without ever putting the controller down."),
                ])),
            )
            .child(div().mt(z.p(14.)).text_size(z.p(15.)).line_height(z.p(22.)).text_color(theme::text_muted()).child(compare));
        // One bar per day of the year so far.
        let start = st.period.start;
        let end = if ongoing { today() } else { st.period.end - Duration::days(1) };
        let n = (end - start).num_days().max(0) as usize + 1;
        let max = st.busiest.map_or(1., |x| x.1.max(1.));
        let strip_w = 1380.;
        let bar_w = strip_w / n as f32;
        let mut strip = div().flex().items_end().h(z.p(80.)).gap(px(1.));
        for i in 0..n {
            let d = start + Duration::days(i as i64);
            let v = st.day_secs(d);
            let c: Hsla = if v > 0. { theme::chart_ramp(((v / max) as f32).powf(0.6)).into() } else { theme::white_a(0.07) };
            strip = strip.child(div().flex_1().h_full().rounded(px(2.)).bg(c));
        }
        let _ = bar_w;
        let months = div().relative().h(z.p(20.)).mt(z.p(10.)).children((1..=12).filter_map(|m| {
            let d = NaiveDate::from_ymd_opt(year, m, 1)?;
            (d <= end).then(|| div().absolute().top_0().left(z.p((d - start).num_days() as f32 / n as f32 * strip_w)).text_size(z.p(12.)).text_color(theme::text_muted()).child(month_short(m)))
        }));
        let gap_note = st.longest_break.filter(|g| g.days >= 7).map(|g| {
            let mid = g.last + Duration::days(g.days as i64 / 2 + 1);
            format!("See that empty stretch? That is your {}-day break in {}.", g.days, month_name(mid.month()))
        });
        let mut text = Self::col(z, 110., 225., None);
        text = text
            .child(rise(("n-kick", epoch), kick("The big number", z), 0, z))
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap(z.p(44.))
                    .mt(z.p(18.))
                    .child(rise(("n-mega", epoch), heading(hours.to_string(), 300., z).line_height(z.p(250.)), 150, z))
                    .child(rise(("n-h2", epoch), div().pb(z.p(14.)).child(heading("hours", 72., z)).child(heading("of play", 72., z)), 300, z))
                    .child(div().flex_1())
                    .child(rise(("n-card", epoch), card.mb(z.p(12.)), 480, z)),
            )
            .child(rise(
                ("n-strip", epoch),
                div()
                    .mt(z.p(80.))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .items_baseline()
                            .mb(z.p(14.))
                            .child(heading(format!("{} of {} days with a game running", st.days_played, n), 24., z))
                            .child(div().text_size(z.p(14.)).text_color(theme::text_muted()).child(format!("{} sessions \u{b7} {} each, on average", st.sessions, fmt_secs(st.total_secs / st.sessions.max(1) as f64)))),
                    )
                    .child(strip)
                    .child(months)
                    .children(gap_note.map(|n| div().mt(z.p(4.)).text_size(z.p(14.)).text_color(theme::text_muted()).child(n))),
                640,
                z,
            ));
        text.into_any_element()
    }

    fn slide_top(&mut self, st: &Stats, year: i32, z: Z, epoch: usize, window: &Window) -> AnyElement {
        let Some(top) = st.games.first() else { return div().into_any_element() };
        let g = top.game;
        let art = self.art(g, (1600. + 2. * z.1) * z.0, 560. * z.0, window);
        let cover = self.cover(g, 390. * z.0, 520. * z.0, window);
        let title = self.game_title(g);
        let size = match title.chars().count() {
            0..=14 => 120.,
            15..=24 => 92.,
            _ => 68.,
        };
        let lib = self.lib.clone();
        let game = lib.as_ref().and_then(|l| l.games.get(g)).cloned();
        let (status, status_color) = match (&lib, &game) {
            (Some(l), Some(gm)) => match l.statuses.iter().find(|s| s.id == gm.status_id) {
                Some(s) => (Some(s.name.clone()), theme::status_color(&s.slug)),
                None => (None, theme::status_color("")),
            },
            _ => (None, theme::status_color("")),
        };
        let genres: Vec<String> = game.as_ref().map(|gm| gm.details.genres.iter().take(3).cloned().collect()).unwrap_or_default();
        let mut chips = div().flex().flex_wrap().items_center().gap(z.p(12.)).mt(z.p(22.));
        if let Some(s) = status {
            chips = chips.child(chip(z).child(div().size(z.p(9.)).rounded_full().bg(status_color)).child(s));
        }
        if top.new {
            chips = chips.child(chip(z).child(format!("New in {year}")));
        }
        for genre in genres {
            chips = chips.child(chip(z).font_weight(theme::W_REGULAR).text_color(theme::text_muted()).child(genre));
        }
        let stat = |big: String, small: &str| div().flex().flex_col().gap(z.p(2.)).child(heading(big, 84., z)).child(div().text_size(z.p(14.)).text_color(theme::text_muted()).child(small.to_string()));
        // Months it was played in.
        let months: Vec<f64> = st.bucket_games.iter().map(|gs| gs.iter().find(|x| x.0 == g).map_or(0., |x| x.1)).collect();
        let active: Vec<usize> = months.iter().enumerate().filter(|(_, v)| **v > 0.).map(|(i, _)| i).collect();
        let sentence = match (active.first(), active.last(), top.first, top.last) {
            (Some(&a), Some(&l), Some(f), Some(last)) if a == l => lead(z).child(rich(vec![
                p("All of it in "),
                b(month_name(a as u32 + 1)),
                p(format!(": {} between ", plural(top.sessions, "session", "sessions"))),
                b(short_date(f.date())),
                p(" and "),
                b(short_date(last.date())),
                p("."),
            ])),
            (Some(&a), Some(&l), _, _) => lead(z).child(rich(vec![
                p(format!("{} across ", plural(top.sessions, "session", "sessions"))),
                b(plural(active.len(), "month", "months")),
                p(", from "),
                b(month_name(a as u32 + 1)),
                p(" to "),
                b(month_name(l as u32 + 1)),
                p("."),
            ])),
            _ => lead(z),
        };
        let mmax = months.iter().copied().fold(1., f64::max);
        let color = series_color(Some(0));
        let mut bars = div().flex().items_end().gap(z.p(6.)).h(z.p(64.));
        for (i, v) in months.iter().enumerate() {
            let h = if *v > 0. { ((*v / mmax) as f32 * 64.).max(3.) } else { 3. };
            bars = bars.child(grow(("t-bar", epoch * 100 + i), div().flex_1().rounded_t(z.p(3.)).bg(if *v > 0. { color } else { theme::white_a(0.1) }), h, 950 + i as u64 * 40, z));
        }
        let labels = div().flex().gap(z.p(6.)).mt(z.p(6.)).children((1..=12).map(|m| div().flex_1().text_center().text_size(z.p(11.5)).text_color(theme::text_muted()).child(month_short(m))));
        let ink = |a: f32| Hsla { a, ..Hsla::from(theme::stats_page_bg()) };
        let mut band = div().absolute().left(-z.p(z.1)).right(-z.p(z.1)).top_0().h(z.p(560.)).overflow_hidden();
        if let Some(src) = art {
            band = band.child(img(src).size_full().object_fit(ObjectFit::Cover));
        }
        band = band
            .child(div().absolute().top_0().bottom_0().left_0().w(relative(0.55)).bg(linear_gradient(90., linear_color_stop(ink(0.9), 0.), linear_color_stop(ink(0.35), 1.))))
            .child(div().absolute().top_0().bottom_0().left(relative(0.55)).right_0().bg(linear_gradient(90., linear_color_stop(ink(0.35), 0.), linear_color_stop(ink(0.1), 1.))))
            .child(div().absolute().left_0().right_0().bottom_0().h(z.p(300.)).bg(linear_gradient(180., linear_color_stop(ink(0.), 0.), linear_color_stop(ink(1.), 1.))));
        let mut stage = div().relative().size_full().child(band).child(div().absolute().left(-z.p(z.1)).right(-z.p(z.1)).top(z.p(558.)).bottom_0().bg(linear_gradient(180., linear_color_stop(ink(1.), 0.), linear_color_stop(ink(0.05), 1.))));
        let ring = Hsla { a: 0.7, ..Hsla::from(theme::accent()) };
        let content = Self::col(z, 110., 172., None)
            .child(rise(("t-kick", epoch), kick("Your number one", z), 0, z))
            .child(
                div()
                    .flex()
                    .gap(z.p(64.))
                    .mt(z.p(40.))
                    .items_start()
                    .child(rise(("t-cover", epoch), cover_img(cover, 390., 520., 24., z).border_3().border_color(ring), 150, z))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .pt(z.p(60.))
                            .child(rise(("t-title", epoch), heading(title, size, z), 260, z))
                            .child(rise(("t-chips", epoch), chips, 420, z))
                            .child(rise(
                                ("t-stats", epoch),
                                div().flex().gap(z.p(64.)).mt(z.p(46.)).child(stat(hours_text(top.secs), "time played")).child(stat(top.sessions.to_string(), "sessions")).child(stat(format!("{:.0}%", top.secs / st.total_secs.max(1.) * 100.), "of your whole year")),
                                580,
                                z,
                            ))
                            .child(rise(("t-lead", epoch), sentence.mt(z.p(32.)).max_w(z.p(760.)), 760, z))
                            .child(rise(("t-bars", epoch), div().mt(z.p(30.)).w(z.p(600.)).child(bars).child(labels), 900, z)),
                    ),
            );
        stage = stage.child(content);
        stage.into_any_element()
    }

    fn slide_podium(&mut self, st: &Stats, z: Z, epoch: usize, window: &Window) -> AnyElement {
        let n = st.games.len().min(5);
        let share: f64 = st.games.iter().take(n).map(|g| g.secs).sum::<f64>() / st.total_secs.max(1.) * 100.;
        let widths = [330., 270., 250., 230., 210.];
        let ranks = [150., 120., 110., 100., 92.];
        let max = st.games.first().map_or(1., |g| g.secs.max(1.));
        let mut pod = div().absolute().left_0().right_0().bottom(z.p(20.)).flex().items_end().justify_center().gap(z.p(18.));
        for (i, g) in st.games.iter().take(n).enumerate() {
            let (w, h) = (widths[i], widths[i] * 4. / 3.);
            let src = self.cover(g.game, w * z.0, h * z.0, window);
            let ring = if i == 0 { Some(Hsla { a: 0.7, ..Hsla::from(theme::accent()) }) } else { None };
            let card = div()
                .w(z.p(w))
                .flex()
                .flex_col()
                .child(
                    div()
                        .relative()
                        .w(z.p(w))
                        .h(z.p(h))
                        .rounded(z.p(20.))
                        .overflow_hidden()
                        .bg(theme::white_a(0.06))
                        .when_some(ring, |d, c| d.border_3().border_color(c))
                        .when(ring.is_none(), |d| d.border_1().border_color(theme::white_a(0.12)))
                        .children(src.map(|s| img(s).absolute().inset_0().size_full().object_fit(ObjectFit::Cover)))
                        .child(div().absolute().left_0().right_0().bottom_0().h(relative(0.5)).bg(linear_gradient(180., linear_color_stop(theme::black_a(0.), 0.), linear_color_stop(Hsla { a: 0.85, ..Hsla::from(theme::stats_page_bg()) }, 1.))))
                        .child(div().absolute().left(z.p(14.)).bottom(z.p(4.)).child(heading((i + 1).to_string(), ranks[i], z))),
                )
                .child(div().mt(z.p(14.)).font_family(theme::FONT_HEADING).font_weight(theme::W_BOLD).text_size(z.p(17.)).truncate().child(self.game_title(g.game)))
                .child(heading(hours_text(g.secs), 30., z).mt(z.p(2.)))
                .child(div().mt(z.p(2.)).text_size(z.p(13.)).text_color(theme::text_muted()).child(format!("{} \u{b7} {:.0}% of the year", plural(g.sessions, "session", "sessions"), g.secs / st.total_secs.max(1.) * 100.)))
                .child(div().mt(z.p(10.)).h(z.p(4.)).rounded_full().bg(theme::white_a(0.12)).child(div().h_full().w(relative((g.secs / max) as f32)).rounded_full().bg(series_color(st.series_of(g.game)))));
            pod = pod.child(rise(("p-card", epoch * 10 + i), card, 300 + i as u64 * 120, z));
        }
        let what = match n {
            1 => "One game".to_string(),
            2 => "Two games".into(),
            3 => "Three games".into(),
            4 => "Four games".into(),
            _ => "Five games".into(),
        };
        let head = div()
            .font_family(theme::FONT_HEADING)
            .font_weight(theme::W_BOLD)
            .text_size(z.p(72.))
            .line_height(z.p(74.))
            .max_w(z.p(1000.))
            .child(rich(vec![p(format!("{what} took ")), (format!("{share:.0}%"), ACCENT), p(" of your time.")]));
        Self::col(z, 110., 130., None)
            .bottom(z.p(60.))
            .child(rise(("p-kick", epoch), kick(&format!("Top {n} of the year"), z), 0, z))
            .child(rise(("p-h2", epoch), head.mt(z.p(18.)), 120, z))
            .child(div().relative().flex_1().child(pod))
            .into_any_element()
    }

    fn slide_habits(&self, st: &Stats, z: Z, epoch: usize) -> AnyElement {
        let total: f64 = st.heat.iter().flatten().sum::<f64>().max(1.);
        let parts = st.day_parts;
        let best_part = (0..4).max_by(|a, b| parts[*a].total_cmp(&parts[*b])).unwrap_or(3);
        let days: Vec<f64> = st.heat.iter().map(|r| r.iter().sum()).collect();
        let best_day = (0..7).max_by(|a, b| days[*a].total_cmp(&days[*b])).unwrap_or(5);
        let hours: Vec<f64> = (0..24).map(|h| st.heat.iter().map(|r| r[h]).sum()).collect();
        let hmax = hours.iter().copied().fold(1., f64::max);
        let wmax = days.iter().copied().fold(1., f64::max);
        let persona = match best_part {
            0 => "Night owl".to_string(),
            1 => "Early bird".to_string(),
            p => format!("{} player,", PERSONA[p]),
        };
        let h1 = div()
            .mt(z.p(30.))
            .child(heading(persona, 112., z))
            .child(heading(format!("{}s", WEEKDAYS_LONG[best_day]), 112., z).text_color(theme::accent()))
            .child(heading("mostly.", 112., z));
        let leadline = lead(z).mt(z.p(36.)).child(rich(vec![
            b(format!("{:.0}%", parts[best_part] / total * 100.)),
            p(format!(" of your play time happens {}, and {} alone is ", PART_RANGE[best_part], WEEKDAYS_LONG[best_day])),
            b(hours_text(days[best_day])),
            p("."),
        ]));
        let mut hbars = div().flex().items_end().gap(z.p(5.)).h(z.p(150.));
        for (i, v) in hours.iter().enumerate() {
            let c: Hsla = if *v > 0. { theme::chart_ramp((*v / hmax) as f32).into() } else { theme::white_a(0.07) };
            hbars = hbars.child(grow(("h-bar", epoch * 100 + i), div().flex_1().rounded_t(z.p(4.)).bg(c), ((*v / hmax) as f32 * 150.).max(2.), i as u64 * 18, z));
        }
        let hx = div().flex().mt(z.p(7.)).children((0..24).map(|h| div().flex_1().text_size(z.p(11.)).text_color(theme::text_dim()).children((h % 6 == 0).then(|| format!("{h:02}")))));
        let day_card = glass_card(z)
            .px(z.p(28.))
            .pt(z.p(24.))
            .pb(z.p(20.))
            .child(div().flex().justify_between().mb(z.p(16.)).child(heading("Your day", 18., z)).child(div().text_size(z.p(13.)).text_color(theme::text_muted()).child("time played by hour")))
            .child(hbars)
            .child(hx);
        let mut wbars = div().flex().items_end().gap(z.p(10.)).h(z.p(150.));
        for (i, v) in days.iter().enumerate() {
            let best = i == best_day;
            wbars = wbars.child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .items_center()
                    .gap(z.p(8.))
                    .text_size(z.p(12.))
                    .text_color(if best { theme::text_primary() } else { theme::text_muted() })
                    .when(best, |d| d.font_weight(theme::W_BOLD))
                    .child(format!("{}h", (v / 3600.).round()))
                    .child(grow(("w-bar", epoch * 10 + i), div().w_full().rounded_t(z.p(5.)).bg(if best { theme::accent() } else { theme::chart_ramp(0.55) }), ((*v / wmax) as f32 * 100.).max(2.), i as u64 * 90, z))
                    .child(WEEKDAYS[i]),
            );
        }
        let week_card = glass_card(z)
            .px(z.p(28.))
            .pt(z.p(24.))
            .pb(z.p(20.))
            .child(div().flex().justify_between().mb(z.p(16.)).child(heading("Your week", 18., z)).child(div().text_size(z.p(13.)).text_color(theme::text_muted()).child(format!("weekends hold {:.0}% of your time", st.weekend_secs / total * 100.))))
            .child(wbars);
        let night = parts[0] / total * 100.;
        let owl = glass_card(z)
            .px(z.p(28.))
            .py(z.p(22.))
            .flex()
            .items_center()
            .gap(z.p(22.))
            .child(div().size(z.p(52.)).rounded(z.p(16.)).bg(Hsla { a: 0.14, ..Hsla::from(theme::accent()) }).flex().items_center().justify_center().child(icon(Icon::Moon, z.p(26.), theme::accent())))
            .child(
                div()
                    .flex_1()
                    .child(heading(if night >= 20. { "A true night owl." } else if night >= 5. { "Night owl? Only a little." } else { "Not a night owl." }, 22., z))
                    .child(div().mt(z.p(3.)).text_size(z.p(14.)).text_color(theme::text_muted()).child(format!("{} between midnight and 6 am.", fmt_secs(parts[0])))),
            )
            .child(heading(format!("{night:.0}%"), 46., z));
        let left = Self::col(z, 110., 185., Some(700.))
            .child(rise(("hb-kick", epoch), kick("Your habits", z), 0, z))
            .child(rise(("hb-h1", epoch), h1, 140, z))
            .child(rise(("hb-lead", epoch), leadline, 380, z));
        let right = div().absolute().right(z.p(110.)).top(z.p(185.)).w(z.p(640.)).flex().flex_col().gap(z.p(20.)).child(rise(("hb-day", epoch), day_card, 300, z)).child(rise(("hb-week", epoch), week_card, 460, z)).child(rise(("hb-owl", epoch), owl, 620, z));
        div().relative().size_full().child(left).child(right).into_any_element()
    }

    /// Tall record card of the records slide.
    fn record_card(&mut self, game: Option<usize>, blurred: bool, label: &str, value: Div, detail: Div, extra: Option<AnyElement>, z: Z, window: &Window) -> Div {
        let (w, h) = (322., 700.);
        // Portrait cards: the cover fits better than wide art.
        let art = game.and_then(|g| if blurred { self.blurred(g, w * z.0 / 2., h * z.0 / 2.) } else { self.cover(g, w * z.0, h * z.0, window) });
        let ink = |a: f32| Hsla { a, ..Hsla::from(theme::stats_page_bg()) };
        let mut card = div().relative().size_full().rounded(z.p(26.)).overflow_hidden().bg(rgb(0x11151c)).border_1().border_color(theme::white_a(0.14));
        if let Some(src) = art {
            card = card.child(img(src).absolute().inset_0().size_full().object_fit(ObjectFit::Cover));
        }
        card.child(div().absolute().left_0().right_0().top_0().h(relative(0.32)).bg(linear_gradient(180., linear_color_stop(ink(0.55), 0.), linear_color_stop(ink(0.05), 1.))))
            .child(div().absolute().left_0().right_0().top(relative(0.32)).h(relative(0.3)).bg(linear_gradient(180., linear_color_stop(ink(0.05), 0.), linear_color_stop(ink(0.55), 1.))))
            .child(div().absolute().left_0().right_0().top(relative(0.62)).bottom_0().bg(linear_gradient(180., linear_color_stop(ink(0.55), 0.), linear_color_stop(ink(0.94), 1.))))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .flex_col()
                    .px(z.p(26.))
                    .pt(z.p(26.))
                    .pb(z.p(28.))
                    .child(div().font_family(theme::FONT_HEADING).font_weight(theme::W_BOLD).text_size(z.p(12.)).text_color(theme::white_a(0.85)).child(label.to_uppercase()))
                    .child(div().flex_1())
                    .children(extra)
                    .child(value.mt(z.p(18.)))
                    .child(detail.mt(z.p(12.)).text_size(z.p(16.)).line_height(z.p(22.)).text_color(theme::white_a(0.88))),
            )
    }

    fn slide_records(&mut self, st: &Stats, z: Z, epoch: usize, window: &Window) -> AnyElement {
        let big = |n: String, unit: Option<&str>| {
            div()
                .flex()
                .items_baseline()
                .gap(z.p(6.))
                .font_family(theme::FONT_HEADING)
                .font_weight(theme::W_BOLD)
                .child(div().text_size(z.p(60.)).line_height(z.p(58.)).child(n))
                .children(unit.map(|u| div().text_size(z.p(25.)).child(u.to_string())))
        };
        let mut cards: Vec<Div> = Vec::new();
        if let Some(l) = st.longest {
            let at = l.start.format("%H:%M").to_string();
            cards.push(self.record_card(Some(l.game), false, "Longest session", big(fmt_secs(l.secs), None), two(self.game_title(l.game), format!("{}, from {at}", super::day_label(l.start.date()))), None, z, window));
        }
        if let Some((d, secs)) = st.busiest {
            let same = st.longest.is_some_and(|l| l.start.date() == d);
            let first = NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap_or(d);
            let mut bars = div().flex().items_end().gap(z.p(3.)).h(z.p(150.));
            let mut day = first;
            while day.month() == d.month() {
                let v = st.day_secs(day);
                bars = bars.child(div().flex_1().h(z.p(((v / secs) as f32 * 150.).max(if v > 0. { 2. } else { 0. }))).rounded_t(z.p(2.)).bg(if day == d { theme::accent() } else { theme::chart_ramp(0.75) }));
                day += Duration::days(1);
            }
            let last = day - Duration::days(1);
            let extra = div().child(bars).child(div().flex().justify_between().mt(z.p(8.)).text_size(z.p(12.)).text_color(theme::white_a(0.7)).child(short_date(first)).child(short_date(last))).into_any_element();
            let game = st.longest.filter(|_| same).map(|l| l.game).or(st.games.first().map(|g| g.game));
            cards.push(self.record_card(game, true, "Busiest day", big(fmt_secs(secs), None), two(super::day_label(d), if same { "The same day as that marathon.".into() } else { "Your most intense day of the year.".into() }), Some(extra), z, window));
        }
        if let (Some((a, e)), true) = (st.streak_range, st.streak > 1) {
            let dots = div().flex().flex_wrap().gap(z.p(3.)).max_w(z.p(260.)).children((0..st.streak.min(60)).map(|_| div().size(z.p(9.)).rounded(z.p(3.)).bg(theme::accent()))).into_any_element();
            cards.push(self.record_card(st.streak_game, false, "Longest streak", big(st.streak.to_string(), Some("days")), two(format!("{} to {}", short_date(a), short_date(e)), "Not one day missed.".into()), Some(dots), z, window));
        }
        match st.longest_break.filter(|g| g.days >= 3) {
            Some(g) => {
                let back = st.buckets.iter().position(|x| x.start <= g.back && g.back < x.end).and_then(|i| st.bucket_games.get(i)?.first().map(|x| x.0));
                let month = st.buckets.iter().find(|x| x.start <= g.back && g.back < x.end).map_or(0., |x| x.secs);
                let detail = two(format!("{} to {}", short_date(g.last + Duration::days(1)), short_date(g.back - Duration::days(1))), format!("You came back on {} and played {} that month.", short_date(g.back), hours_text(month)));
                cards.push(self.record_card(back, false, "The longest break", big(g.days.to_string(), Some("days")), detail, None, z, window));
            }
            None => {
                if let Some((i, bk)) = st.buckets.iter().enumerate().max_by(|a, c| a.1.secs.total_cmp(&c.1.secs)) {
                    let game = st.bucket_games.get(i).and_then(|gs| gs.first()).map(|x| x.0);
                    cards.push(self.record_card(game, false, "Busiest month", big(hours_text(bk.secs), None), two(month_name(bk.start.month()).to_string(), game.map(|g| format!("Led by {}.", self.game_title(g))).unwrap_or_default()), None, z, window));
                }
            }
        }
        let n = cards.len().max(1) as f32;
        let card_w = (1420. - 24. * (n - 1.)) / n;
        let grid = div().absolute().left_0().top(z.p(140.)).bottom_0().w(z.p(1420.)).flex().gap(z.p(24.)).children(cards.into_iter().enumerate().map(|(i, c)| rise(("r-card", epoch * 10 + i), div().w(z.p(card_w)).h_full().child(c), 260 + i as u64 * 150, z)));
        Self::col(z, 90., 128., None)
            .bottom(z.p(60.))
            .child(rise(("r-kick", epoch), kick("Records", z), 0, z))
            .child(rise(("r-h2", epoch), heading("The moments that stood out.", 60., z).mt(z.p(14.)), 100, z))
            .child(grid)
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn slide_wrap(&mut self, st: &Stats, d: &Derived, year: i32, z: Z, epoch: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let has_shots = d.shots_total > 0;
        // Screenshot wall: the latest shot of each game with captures, up to six.
        let wall: Vec<(usize, std::path::PathBuf)> = d.shots.iter().take(6).filter_map(|(g, l)| l.last().map(|s| (*g, s.path.clone()))).collect();
        let mut left = div().w(z.p(800.)).flex().flex_col();
        if has_shots {
            let names: Vec<String> = d.shots.iter().take(2).map(|(g, _)| self.game_title(*g)).collect();
            let mut head = div().flex().flex_wrap().items_baseline().gap_x(z.p(12.)).mb(z.p(14.)).child(heading(d.shots_total.to_string(), 54., z)).child(div().text_size(z.p(17.)).text_color(theme::text_muted()).child("screenshots taken, mostly of"));
            head = match names.as_slice() {
                [a, c] => head.child(div().text_size(z.p(17.)).font_weight(theme::W_BOLD).child(a.clone())).child(div().text_size(z.p(17.)).text_color(theme::text_muted()).child("and")).child(div().text_size(z.p(17.)).font_weight(theme::W_BOLD).child(c.clone())),
                [a] => head.child(div().text_size(z.p(17.)).font_weight(theme::W_BOLD).child(a.clone())),
                _ => head,
            };
            let (tw, th) = ((800. - 20.) / 3., (800. - 20.) / 3. * 9. / 16.);
            let mut grid = div().flex().flex_wrap().gap(z.p(10.));
            for (i, (_, path)) in wall.iter().enumerate() {
                let src = self.shot(path, tw * z.0, th * z.0, window, cx);
                grid = grid.child(rise(("w-shot", epoch * 10 + i), cover_img(src, tw, th, 14., z).border_1().border_color(theme::white_a(0.1)), 320 + i as u64 * 70, z));
            }
            let cmax = d.shots.first().map_or(1, |x| x.1.len().max(1));
            let caps = div().mt(z.p(22.)).flex().flex_col().gap(z.p(9.)).children(d.shots.iter().take(5).map(|(g, l)| {
                div()
                    .flex()
                    .items_center()
                    .gap(z.p(14.))
                    .text_size(z.p(13.5))
                    .child(div().w(z.p(270.)).flex_none().truncate().child(self.game_title(*g)))
                    .child(div().flex_1().h(z.p(6.)).rounded_full().bg(theme::white_a(0.1)).child(div().h_full().w(relative(l.len() as f32 / cmax as f32)).rounded_full().bg(theme::accent())))
                    .child(div().w(z.p(36.)).flex().justify_end().text_color(theme::text_muted()).child(l.len().to_string()))
            }));
            left = left.child(head).child(grid).child(rise(("w-caps", epoch), caps, 1000, z));
        } else {
            let gmax = d.genres.first().map_or(1., |g| g.1.max(1.));
            left = left.child(heading("Genres of the year", 30., z).mb(z.p(18.))).children(d.genres.iter().map(|(n, s)| {
                div()
                    .flex()
                    .items_center()
                    .gap(z.p(14.))
                    .h(z.p(40.))
                    .text_size(z.p(16.))
                    .child(div().w(z.p(240.)).truncate().child(n.clone()))
                    .child(div().flex_1().h(z.p(10.)).rounded_full().bg(theme::white_a(0.1)).child(div().h_full().w(relative((*s / gmax) as f32)).rounded_full().bg(theme::chart_ramp(1.))))
                    .child(div().w(z.p(70.)).flex().justify_end().text_color(theme::text_muted()).child(format!("{}h", (s / 3600.).round())))
            }));
        }
        // Games beaten.
        let mut beaten = glass_card(z).w(z.p(310.)).px(z.p(24.)).pt(z.p(24.)).pb(z.p(22.));
        if d.beaten.is_empty() {
            beaten = beaten
                .child(div().flex().items_baseline().gap(z.p(10.)).child(heading(st.games.len().to_string(), 54., z)).child(div().text_size(z.p(17.)).text_color(theme::text_muted()).child("games played")))
                .child(div().mt(z.p(10.)).text_size(z.p(15.)).text_color(theme::text_muted()).child(format!("{} of them for the first time.", st.new_games())));
        } else {
            beaten = beaten.child(div().flex().items_baseline().gap(z.p(10.)).child(heading(d.beaten.len().to_string(), 54., z)).child(div().text_size(z.p(17.)).text_color(theme::text_muted()).child(if d.beaten.len() == 1 { "game beaten" } else { "games beaten" })));
            let mut list = div().flex().flex_col().gap(z.p(7.)).mt(z.p(14.));
            for g in d.beaten.iter().take(9) {
                let src = self.cover(*g, 28. * z.0, 37. * z.0, window);
                list = list.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(z.p(12.))
                        .text_size(z.p(14.))
                        .child(cover_img(src, 28., 37., 6., z))
                        .child(div().flex_1().min_w_0().truncate().child(self.game_title(*g)))
                        .child(icon(Icon::Check, z.p(16.), theme::success())),
                );
            }
            beaten = beaten.child(list);
        }
        // The poster.
        let top = st.games.first().map(|g| g.game);
        let art = top.and_then(|g| self.art(g, 380. * z.0, 300. * z.0, window));
        let mmax = st.buckets.iter().map(|x| x.secs).fold(1., f64::max);
        let mut months = div().flex().items_end().gap(z.p(4.)).h(z.p(70.)).mt_auto().mb(z.p(22.));
        for bk in &st.buckets {
            months = months.child(div().flex_1().h(z.p(((bk.secs / mmax) as f32 * 70.).max(3.))).rounded_t(z.p(3.)).bg(if bk.future { theme::white_a(0.08) } else { Hsla { a: 0.75, ..Hsla::from(theme::chart_ramp(0.9)) } }));
        }
        let mut t3 = div().flex().flex_col().gap(z.p(9.));
        for (i, g) in st.games.iter().take(3).enumerate() {
            let src = self.cover(g.game, 30. * z.0, 40. * z.0, window);
            t3 = t3.child(
                div()
                    .flex()
                    .items_center()
                    .gap(z.p(12.))
                    .text_size(z.p(14.))
                    .child(div().w(z.p(18.)).font_family(theme::FONT_HEADING).font_weight(theme::W_BOLD).text_size(z.p(20.)).text_color(theme::text_muted()).child((i + 1).to_string()))
                    .child(cover_img(src, 30., 40., 6., z))
                    .child(div().flex_1().min_w_0().truncate().child(self.game_title(g.game)))
                    .child(div().font_family(theme::FONT_HEADING).font_weight(theme::W_BOLD).child(hours_text(g.secs))),
            );
        }
        let ink = rgb(0x0e1218);
        let inkh = |a: f32| Hsla { a, ..Hsla::from(ink) };
        let mut poster = div().relative().w(z.p(380.)).h(z.p(612.)).rounded(z.p(26.)).overflow_hidden().bg(ink).border_1().border_color(theme::white_a(0.14));
        if let Some(src) = art {
            poster = poster.child(div().absolute().left_0().right_0().top_0().h(z.p(300.)).child(img(src).size_full().object_fit(ObjectFit::Cover)).child(div().absolute().inset_0().bg(linear_gradient(180., linear_color_stop(inkh(0.25), 0.), linear_color_stop(inkh(1.), 1.)))));
        }
        poster = poster.child(
            div()
                .relative()
                .flex()
                .flex_col()
                .size_full()
                .px(z.p(26.))
                .py(z.p(24.))
                .child(div().flex().items_center().gap(z.p(8.)).font_family(theme::FONT_HEADING).font_weight(theme::W_BOLD).text_size(z.p(12.)).text_color(theme::white_a(0.85)).child(div().size(z.p(18.)).rounded(z.p(6.)).bg(theme::accent())).child(format!("HERONITE \u{b7} {year} IN GAMES")))
                .child(heading("I played for", 21., z).mt(z.p(96.)).text_color(theme::white_a(0.9)))
                .child(heading(fmt_secs(st.total_secs), 64., z).mt(z.p(6.)))
                .child(div().mt(z.p(6.)).text_size(z.p(14.)).text_color(theme::text_muted()).child(format!("{} days \u{b7} {} sessions \u{b7} {} games", st.days_played, st.sessions, st.games.len())))
                .child(months)
                .child(t3),
        );
        let summary = self.summary_text(self.review.as_ref().expect("review open"));
        let button = |id: &'static str, label: &str, ic: Option<Icon>, primary: bool| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(z.p(8.))
                .h(z.p(40.))
                .px(z.p(20.))
                .rounded_full()
                .font_family(theme::FONT_HEADING)
                .font_weight(theme::W_BOLD)
                .text_size(z.p(14.))
                .cursor_pointer()
                .when(primary, |d| d.bg(theme::accent()).text_color(theme::on_accent()))
                .when(!primary, |d| d.bg(theme::white_a(0.09)).border_1().border_color(theme::white_a(0.14)).hover(|s| s.bg(theme::white_a(0.16))))
                .children(ic.map(|i| icon(i, z.p(16.), if primary { theme::on_accent().into() } else { Hsla::from(theme::text_primary()) })))
                .child(label.to_string())
        };
        let buttons = div()
            .flex()
            .gap(z.p(10.))
            .mt(z.p(18.))
            .child(button("review-copy", "Copy summary", Some(Icon::ContentCopy), true).on_click(cx.listener(move |_, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(summary.clone()));
                cx.emit(PageEvent::Toast("Summary copied".into(), false));
            })))
            .child(button("review-back", "Back to stats", None, false).on_click(cx.listener(|this, _, _, cx| this.close_review(cx))));
        let row = div()
            .absolute()
            .left_0()
            .right_0()
            .top(z.p(140.))
            .flex()
            .items_start()
            .gap(z.p(40.))
            .child(rise(("w-left", epoch), left, 260, z))
            .child(rise(("w-beaten", epoch), beaten, 560, z))
            .child(div().flex_1())
            .child(rise(("w-poster", epoch), div().child(poster).child(buttons), 760, z));
        Self::col(z, 110., 128., None)
            .bottom(z.p(40.))
            .child(rise(("w-kick", epoch), kick("That is a wrap", z), 0, z))
            .child(rise(("w-h2", epoch), heading(if has_shots { "Proof of every good moment." } else { "That was your year." }, 64., z).mt(z.p(14.)), 100, z))
            .child(row)
            .into_any_element()
    }
}

fn keycap(text: &str) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .min_w(px(24.))
        .h(px(22.))
        .px(px(6.))
        .rounded(px(6.))
        .border_1()
        .border_color(theme::white_a(0.14))
        .bg(theme::white_a(0.06))
        .font_family(theme::FONT_HEADING)
        .font_weight(theme::W_BOLD)
        .text_size(px(11.))
        .text_color(theme::text_muted())
        .child(text.to_string())
}
