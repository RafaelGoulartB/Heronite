//! The game page of a picture, in the theme's `game_page` style (the console's
//! `collection/console/game/`): Classic, Hero, Tabs or the Sheet floating over
//! the tab. The first game of the library, the Play button on the focus colour
//! with the theme's text on it, and the actions as the console lists them.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{Clock, GamePage, HeroTitle};
use heronite_i18n::tr;

use super::layouts;
use super::{Pic, CANVAS_H};
use crate::icons::Icon;
use crate::theme::{self, W_BOLD};
use crate::ui::icon;

/// The value at the right of an action.
enum Val {
    None,
    /// Status: `◀ … ▶` around the current one.
    Choice,
    /// A switch that is on.
    On,
}

/// Width of the sheet.
const SHEET_W: f32 = 220.;
/// Its space from the edges.
const MARGIN: f32 = 9.;

/// The page behind the page's own parts: the tab under the Sheet, else the
/// art with the style's shading.
pub(super) fn content(p: &mut Pic) -> Div {
    let layer = div().absolute().inset_0();
    match p.t.game_page {
        GamePage::Classic => classic(p, layer.child(p.cover_all().bg(p.shade(0.55)))),
        GamePage::Hero => hero(p, layer.child(p.scrim_left(0.6)).child(p.scrim_bottom(0.7))),
        GamePage::Tabs => tabs(p, layer.child(p.cover_all().bg(p.shade(0.62)))),
        GamePage::Sheet => layouts::content(p),
    }
}

/// What floats over the whole picture: the Sheet's window and the dim behind it.
pub(super) fn sheet(p: &mut Pic) -> Option<Div> {
    if p.t.game_page != GamePage::Sheet {
        return None;
    }
    let cover = p.picture_box(0, false, 46., 66., 6.).shadow(vec![BoxShadow {
        color: theme::black_a(0.5),
        offset: point(px(0.), p.u(3.)),
        blur_radius: p.u(8.),
        spread_radius: px(0.),
        inset: false,
    }]);
    let badge = div()
        .flex()
        .items_center()
        .gap(p.u(3.))
        .px(p.u(5.))
        .py(p.u(2.))
        .rounded_full()
        .bg(Rgba { a: 0.14, ..p.text })
        .child(div().size(p.u(4.)).rounded_full().bg(p.on))
        .child(p.line(22., 3.5, 0.7));
    let head = div().flex().items_end().gap(p.u(9.)).px(p.u(11.)).pt(p.u(12.)).pb(p.u(7.)).child(cover).child(
        div()
            .flex()
            .flex_col()
            .gap(p.u(4.))
            .min_w_0()
            .child(p.game_name(0, 12., 120.))
            .child(p.line(80., 3.5, 0.5))
            .child(div().flex().child(badge)),
    );
    let rows = p.actions(true);
    let panel = p
        .sheet_surface()
        .absolute()
        .top(p.u(MARGIN))
        .bottom(p.u(MARGIN))
        .left(p.u(MARGIN))
        .w(p.u(SHEET_W))
        .overflow_hidden()
        .child(head)
        .child(div().px(p.u(6.)).child(rows));
    Some(p.full().child(p.full().bg(p.shade(0.45))).child(panel))
}

impl Pic<'_, '_> {
    fn sheet_surface(&self) -> Div {
        div().rounded(self.u(13.)).bg(self.sidebar).border_1().border_color(Rgba { a: 0.1, ..self.text }).shadow(vec![BoxShadow {
            color: theme::black_a(0.55),
            offset: point(px(0.), self.u(8.)),
            blur_radius: self.u(24.),
            spread_radius: px(0.),
            inset: false,
        }])
    }

    /// A soft card over the art (the Classic page's right side).
    fn card(&self) -> Div {
        div().rounded(self.u(10.)).bg(Rgba { a: 0.07, ..self.text }).border_1().border_color(Rgba { a: 0.06, ..self.text })
    }

    /// One action of the list: an icon, its name and its value; the focused
    /// one on the focus colour with the theme's text on it.
    fn action(&self, ic: Icon, name: &'static str, val: Val, focused: bool) -> Div {
        let (fg, mute) = if focused { (self.focus_text, self.focus_text) } else { (self.text, self.muted) };
        let value = match val {
            Val::None => None,
            Val::Choice => Some(
                div()
                    .flex()
                    .items_center()
                    .gap(self.u(4.))
                    .child(self.label("\u{25c0}", 5.5, mute))
                    .child(self.line(24., 3.5, 0.6))
                    .child(self.label("\u{25b6}", 5.5, mute)),
            ),
            Val::On => Some(
                div()
                    .flex_none()
                    .px(self.u(6.))
                    .py(self.u(1.5))
                    .rounded_full()
                    .bg(self.on)
                    .child(self.label(tr!("On"), 6., self.on_fill(self.on)).font_weight(W_BOLD)),
            ),
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(self.u(7.))
            .h(self.u(20.))
            .px(self.u(9.))
            .rounded(self.u(7.))
            .when(focused, |d| d.bg(self.accent).border_1().border_color(theme::white_a(0.5)))
            .child(icon(ic, self.u(10.), mute))
            .child(self.label(name, 8.5, fg).flex_1().min_w_0())
            .children(value)
    }

    /// The game's actions: Play (focused), Status, Favourite (on) and Game
    /// settings, and the Sheet's "Full page" when `full_page`.
    fn actions(&self, full_page: bool) -> Div {
        let mut list = div()
            .flex()
            .flex_col()
            .gap(self.u(1.))
            .child(self.action(Icon::PlayArrow, tr!("Play"), Val::None, true))
            .child(self.action(Icon::Flag, tr!("Status"), Val::Choice, false))
            .child(self.action(Icon::Favorite, tr!("Favourite"), Val::On, false))
            .child(self.action(Icon::Tune, tr!("Game settings"), Val::None, false));
        if full_page {
            list = list.child(self.action(Icon::OpenInNew, tr!("Full page"), Val::None, false));
        }
        list
    }

    /// A round button of the Hero page: an icon, and a name for the wide ones.
    fn pill(&self, ic: Icon, name: Option<&'static str>, focused: bool) -> Div {
        let (fg, fill) = if focused { (self.focus_text, self.accent) } else { (self.text, Rgba { a: 0.14, ..self.text }) };
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(self.u(4.))
            .h(self.u(20.))
            .min_w(self.u(20.))
            .px(self.u(if name.is_some() { 9. } else { 5. }))
            .rounded_full()
            .bg(fill)
            .when(focused, |d| d.border_1().border_color(theme::white_a(0.5)))
            .child(icon(ic, self.u(10.), fg))
            .children(name.map(|n| self.label(n, 8.5, fg).font_weight(W_BOLD)))
    }

    /// The page's tabs: LB, the names, RB; the active one under a bar in the focus colour.
    fn page_tabs(&self, names: &[&'static str], active: usize) -> Div {
        let grey = Rgba { a: 0.5, ..self.text };
        let mut row = div().flex().items_center().gap(self.u(12.)).child(self.glyph("LB", grey));
        for (i, name) in names.iter().enumerate() {
            let on = i == active;
            row = row.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(self.u(2.))
                    .child(self.label(*name, 8.5, if on { self.text } else { self.muted }).when(on, |d| d.font_weight(W_BOLD)))
                    .child(div().h(self.u(1.5)).w(self.u(if on { 14. } else { 0. })).rounded_full().bg(self.accent)),
            );
        }
        row.child(self.glyph("RB", grey))
    }
}

/// The game large over its art, the actions on the left and a card with the
/// numbers, the description and the facts on the right.
fn classic(p: &mut Pic, layer: Div) -> Div {
    let cover = p.picture_box(0, false, 62., 90., 6.).shadow(vec![BoxShadow {
        color: theme::black_a(0.6),
        offset: point(px(0.), p.u(5.)),
        blur_radius: p.u(14.),
        spread_radius: px(0.),
        inset: false,
    }]);
    let head = div().flex().items_end().gap(p.u(14.)).child(cover).child(
        div().flex().flex_col().gap(p.u(5.)).pb(p.u(3.)).child(p.game_name(0, 24., 360.)).child(p.facts()).child(p.line(110., 3.5, 0.4)),
    );
    let list = p.actions(false).w(p.u(280.)).child(p.action(Icon::AccessTime, tr!("Playtime"), Val::None, false)).child(p.action(
        Icon::Edit,
        tr!("Manage"),
        Val::None,
        false,
    ));
    let y = 134.;
    let mut numbers = div().flex().gap(p.u(6.));
    for name in [tr!("Playtime"), tr!("Last played"), tr!("Main story")] {
        numbers = numbers.child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(p.u(3.))
                .px(p.u(7.))
                .py(p.u(5.))
                .rounded(p.u(6.))
                .bg(Rgba { a: 0.06, ..p.text })
                .child(p.label(name, 5.5, p.muted))
                .child(p.line(20., 6., 0.85)),
        );
    }
    let mut facts = div().flex().flex_col().gap(p.u(5.));
    for name in [tr!("Status"), tr!("Library"), tr!("Genre"), tr!("Developer")] {
        facts = facts.child(
            div()
                .flex()
                .items_center()
                .gap(p.u(8.))
                .child(div().w(p.u(46.)).child(p.label(name, 6., p.muted)))
                .child(p.line(60., 3.5, 0.6)),
        );
    }
    let card = p
        .card()
        .w(p.u(p.width() - 346.))
        .h(p.u((p.bottom() - y).max(90.)))
        .overflow_hidden()
        .p(p.u(10.))
        .flex()
        .flex_col()
        .gap(p.u(9.))
        .child(numbers)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(p.u(3.))
                .child(p.line(190., 3.5, 0.45))
                .child(p.line(170., 3.5, 0.45))
                .child(p.line(120., 3.5, 0.45)),
        )
        .child(div().h(px(1.)).w_full().bg(Rgba { a: 0.1, ..p.text }))
        .child(facts);
    layer.child(p.at(23., 30., head)).child(p.at(23., y, list)).child(p.at(323., y, card))
}

/// The art full screen with the name, the facts, a row of buttons and the
/// page's tabs under them.
fn hero(p: &mut Pic, layer: Div) -> Div {
    let logo = p.t.hero.title == HeroTitle::Logo;
    let buttons = div()
        .flex()
        .gap(p.u(6.))
        .mt(p.u(2.))
        .child(p.pill(Icon::PlayArrow, Some(tr!("Play")), true))
        .child(p.pill(Icon::Flag, Some(tr!("Status")), false))
        .child(p.pill(Icon::FavoriteBorder, None, false))
        .child(p.pill(Icon::Tune, None, false))
        .child(p.pill(Icon::Menu, None, false));
    let about = div()
        .flex()
        .flex_col()
        .gap(p.u(4.))
        .mt(p.u(2.))
        .child(p.line(300., 3.5, 0.45))
        .child(p.line(260., 3.5, 0.45))
        .child(p.line(200., 3.5, 0.45));
    let page = div()
        .absolute()
        .left(p.u(28.))
        .bottom(p.u(CANVAS_H - p.bottom() + 6.))
        .flex()
        .flex_col()
        .gap(p.u(6.))
        .child(p.game_name(0, if logo { 38. } else { 29. }, 360.))
        .child(p.facts())
        .child(buttons)
        .child(div().mt(p.u(5.)).child(p.page_tabs(&[tr!("Overview"), tr!("Details"), tr!("Screenshots"), tr!("Journal")], 0)))
        .child(about);
    layer.child(page)
}

/// A bar with the small cover, the name, the page's tabs and the clock; Play
/// shows the big cover and the actions.
fn tabs(p: &mut Pic, layer: Div) -> Div {
    let small = p.picture_box(0, false, 15., 22., 3.);
    let clock = match p.t.interface.clock {
        Clock::H24 => Some("21:30"),
        Clock::H12 => Some("9:30"),
        Clock::Off => None,
    };
    let bar = div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(p.u(38.))
        .px(p.u(23.))
        .flex()
        .items_center()
        .gap(p.u(18.))
        .child(div().flex().items_center().gap(p.u(6.)).child(small).child(p.game_name(0, 9., 110.)))
        .child(p.page_tabs(&[tr!("Play"), tr!("About"), tr!("Manage")], 0))
        .child(div().flex_1())
        .children(clock.map(|c| p.label(c, 10., p.text)));
    let big = p.picture_box(0, false, 100., 146., 8.).shadow(vec![BoxShadow {
        color: theme::black_a(0.55),
        offset: point(px(0.), p.u(6.)),
        blur_radius: p.u(16.),
        spread_radius: px(0.),
        inset: false,
    }]);
    let left =
        div().flex().flex_col().gap(p.u(6.)).child(big).child(
            div().flex().items_center().gap(p.u(4.)).child(div().size(p.u(5.)).rounded_full().bg(p.on)).child(p.line(40., 3.5, 0.5)),
        );
    let right = div()
        .flex()
        .flex_col()
        .gap(p.u(6.))
        .w(p.u(290.))
        .child(p.game_name(0, 15., 290.))
        .child(p.facts())
        .child(div().mt(p.u(4.)).child(p.actions(false)));
    layer.child(bar).child(p.at(23., 52., div().flex().gap(p.u(20.)).child(left).child(right)))
}
