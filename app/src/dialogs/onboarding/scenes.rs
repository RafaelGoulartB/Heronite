//! The looping previews of the getting-started checklist: one short scene per
//! step (a cursor logging in to a store, Proton downloading until it becomes
//! the default, F10 sending a picture to the timeline...), drawn from a clock
//! `t` in 0..1 that loops every [`LOOP`]. They use the user's own covers and
//! titles. Scenes are laid out on a 520x320 board scaled by `k`; with the
//! animations turned off they show one still frame.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};

use super::steps::Step;
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

pub const W: f32 = 520.;
pub const H: f32 = 320.;
const LOOP: Duration = Duration::from_millis(6500);
/// Where a scene stops when animations are off: everything has happened.
const STILL: f32 = 0.8;

/// What the scenes show from this setup.
#[derive(Clone, Default)]
pub struct Assets {
    /// Covers of the most played games (`None` while loading or missing).
    pub covers: Vec<Option<ImageSource>>,
    /// Background art of the most played game.
    pub stage: Option<ImageSource>,
    pub titles: Vec<SharedString>,
    pub shot_key: SharedString,
    pub steam_games: usize,
}

impl Assets {
    fn title(&self, i: usize) -> SharedString {
        self.titles.get(i).cloned().unwrap_or_else(|| ["Hades II", "Celeste", "Hollow Knight", "Dead Cells", "Balatro"][i % 5].into())
    }
}

/// The scene of `step`, `k` times the 520x320 board.
pub fn scene(step: Step, k: f32, assets: Assets) -> AnyElement {
    let base = div().w(px(W * k)).h(px(H * k));
    if !theme::animations() {
        return base.child(build(step, &Sc { k, t: STILL, a: &assets })).into_any_element();
    }
    base.with_animation(
        SharedString::from(format!("onboarding-scene-{}", step.id())),
        Animation::new(LOOP).repeat().with_max_fps(30.),
        move |el, t| el.child(build(step, &Sc { k, t, a: &assets })),
    )
    .into_any_element()
}

/// 0 before `a`, 1 after `b`, eased in between.
fn ramp(t: f32, a: f32, b: f32) -> f32 {
    let x = ((t - a) / (b - a)).clamp(0., 1.);
    x * x * (3. - 2. * x)
}

/// Fades in over `a..b` and out over `c..d`.
fn fade(t: f32, a: f32, b: f32, c: f32, d: f32) -> f32 {
    ramp(t, a, b) * (1. - ramp(t, c, d))
}

fn lerp(a: f32, b: f32, x: f32) -> f32 {
    a + (b - a) * x
}

fn violet() -> Rgba {
    rgb(0xb7a8ff)
}
fn warm() -> Rgba {
    rgb(0xffcf9c)
}

struct Sc<'a> {
    k: f32,
    t: f32,
    a: &'a Assets,
}

impl Sc<'_> {
    fn p(&self, v: f32) -> Pixels {
        px(v * self.k)
    }

    fn at(&self, x: f32, y: f32) -> Div {
        div().absolute().left(self.p(x)).top(self.p(y))
    }

    fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> Div {
        self.at(x, y).w(self.p(w)).h(self.p(h))
    }

    fn text(&self, size: f32, color: impl Into<Hsla>, s: impl Into<SharedString>) -> Div {
        div().text_size(self.p(size)).text_color(color.into()).whitespace_nowrap().child(s.into())
    }

    fn heading(&self, size: f32, s: impl Into<SharedString>) -> Div {
        self.text(size, theme::text_primary(), s).font_family(theme::FONT_HEADING).font_weight(theme::W_BOLD)
    }

    fn ic(&self, ic: Icon, size: f32, color: impl Into<Hsla>) -> Svg {
        icon(ic, self.p(size), color)
    }

    /// A cover of the user's games (a tinted block while it loads).
    fn cover(&self, i: usize, x: f32, y: f32, w: f32, h: f32) -> Div {
        let hue = (i as f32 * 0.137 + 0.55) % 1.;
        let d = self.rect(x, y, w, h).rounded(self.p(7.)).overflow_hidden().bg(hsla(hue, 0.35, 0.32, 1.)).shadow(vec![BoxShadow {
            color: theme::black_a(0.4),
            offset: point(px(0.), self.p(6.)),
            blur_radius: self.p(16.),
            spread_radius: px(0.),
            inset: false,
        }]);
        match self.a.covers.get(i).cloned().flatten() {
            Some(src) => d.child(img(src).size_full().object_fit(ObjectFit::Cover)),
            None => d,
        }
    }

    fn picture(&self, src: Option<ImageSource>, x: f32, y: f32, w: f32, h: f32) -> Div {
        let d = self.rect(x, y, w, h).overflow_hidden().bg(rgb(0x1d2a3a));
        match src {
            Some(src) => d.child(img(src).size_full().object_fit(ObjectFit::Cover)),
            None => d,
        }
    }

    fn panel(&self, x: f32, y: f32, w: f32, h: f32) -> Div {
        self.rect(x, y, w, h).rounded(self.p(12.)).bg(theme::white_a(0.05)).border_1().border_color(theme::white_a(0.09))
    }

    fn pill(&self, x: f32, y: f32, s: impl Into<SharedString>, dot: bool) -> Div {
        self.at(x, y)
            .flex()
            .items_center()
            .gap(self.p(8.))
            .px(self.p(12.))
            .py(self.p(5.))
            .rounded_full()
            .bg(theme::black_a(0.45))
            .border_1()
            .border_color(theme::white_a(0.12))
            .when(dot, |d| d.child(div().size(self.p(8.)).rounded_full().bg(theme::success())))
            .child(self.text(12., rgb(0xd6dde7), s))
    }

    /// A notification centred at the bottom (or the top), fading in from below.
    fn toast(&self, ic: Icon, color: impl Into<Hsla>, s: impl Into<SharedString>, alpha: f32, top: bool) -> Div {
        let shift = (1. - alpha) * 14.;
        let row = div().absolute().left_0().right_0().flex().justify_center().opacity(alpha).child(
            div()
                .flex()
                .items_center()
                .gap(self.p(8.))
                .px(self.p(14.))
                .py(self.p(8.))
                .rounded(self.p(12.))
                .bg(theme::rgb_a(18, 22, 28, 0.95))
                .border_1()
                .border_color(theme::white_a(0.14))
                .child(self.ic(ic, 15., color))
                .child(self.text(12.5, theme::text_primary(), s)),
        );
        if top {
            row.top(self.p(54. - shift))
        } else {
            row.bottom(self.p(14. - shift))
        }
    }

    fn cursor(&self, x: f32, y: f32, press: f32, alpha: f32) -> Div {
        let s = 22. * (1. - 0.18 * press);
        self.at(x, y)
            .opacity(alpha)
            .child(div().absolute().left(self.p(1.)).top(self.p(1.5)).child(self.ic(Icon::Cursor, s + 2., theme::black_a(0.85))))
            .child(div().absolute().child(self.ic(Icon::Cursor, s, white())))
    }

    fn key(&self, x: f32, y: f32, label: impl Into<SharedString>, press: f32) -> Div {
        self.at(x, y + 3. * press)
            .px(self.p(12.))
            .py(self.p(6.))
            .rounded(self.p(8.))
            .bg(rgb(0x232a34))
            .border_1()
            .border_color(if press > 0.5 { theme::accent().into() } else { theme::white_a(0.2) })
            .shadow(vec![BoxShadow {
                color: if press > 0.5 { theme::accent().into() } else { theme::black_a(0.7) },
                offset: point(px(0.), self.p(if press > 0.5 { 0. } else { 3. })),
                blur_radius: self.p(if press > 0.5 { 16. } else { 0. }),
                spread_radius: px(0.),
                inset: false,
            }])
            .child(self.heading(13., label))
    }

    fn button(&self, s: impl Into<SharedString>, primary: bool) -> Div {
        div()
            .flex()
            .items_center()
            .justify_center()
            .gap(self.p(6.))
            .px(self.p(14.))
            .h(self.p(30.))
            .rounded_full()
            .bg(if primary { theme::accent().into() } else { theme::white_a(0.08) })
            .child(self.text(12.5, if primary { theme::on_accent() } else { theme::text_primary() }, s).font_weight(theme::W_BOLD))
    }

    fn bar(&self, x: f32, y: f32, w: f32, h: f32, fill: f32) -> Div {
        self.rect(x, y, w, h).rounded_full().bg(theme::white_a(0.1)).overflow_hidden().child(
            div().h_full().w(self.p(w * fill)).rounded_full().bg(linear_gradient(
                90.,
                linear_color_stop(theme::accent(), 0.),
                linear_color_stop(violet(), 1.),
            )),
        )
    }

    fn spinner(&self, size: f32) -> Svg {
        self.ic(Icon::FaSpinner, size, theme::accent()).with_transformation(Transformation::rotate(percentage((self.t * 8.) % 1.)))
    }

    fn check(&self, size: f32, alpha: f32) -> Div {
        div()
            .size(self.p(size))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::rgb_a(95, 211, 155, 0.18))
            .opacity(alpha)
            .child(self.ic(Icon::Check, size * 0.7, theme::success()))
    }

    fn menu(&self, x: f32, y: f32, w: f32, rows: Vec<Div>, alpha: f32) -> Div {
        self.at(x, y)
            .w(self.p(w))
            .opacity(alpha)
            .p(self.p(6.))
            .rounded(self.p(12.))
            .bg(theme::rgb_a(22, 27, 35, 0.97))
            .border_1()
            .border_color(theme::white_a(0.13))
            .flex()
            .flex_col()
            .children(rows)
    }

    fn menu_row(&self, ic: Option<Icon>, s: impl Into<SharedString>, sub: bool, hl: Option<Hsla>) -> Div {
        div()
            .flex()
            .items_center()
            .gap(self.p(9.))
            .h(self.p(30.))
            .px(self.p(10.))
            .rounded(self.p(8.))
            .when_some(hl, |d, c| d.bg(c))
            .children(ic.map(|i| self.ic(i, 14., theme::text_dim())))
            .child(self.text(12.5, rgb(0xd6dde7), s))
            .when(sub, |d| d.child(div().ml_auto().child(self.ic(Icon::ChevronRight, 14., theme::text_dim()))))
    }
}

fn build(step: Step, s: &Sc) -> Div {
    let board = div()
        .relative()
        .w(s.p(W))
        .h(s.p(H))
        .overflow_hidden()
        .rounded(s.p(16.))
        .border_1()
        .border_color(theme::white_a(0.08))
        .bg(linear_gradient(160., linear_color_stop(rgb(0x18232f), 0.), linear_color_stop(rgb(0x0a0e13), 0.7)))
        .text_color(theme::text_primary())
        .font_family(theme::FONT_BODY);
    match step {
        Step::Steam => steam(board, s),
        Step::Stores => stores(board, s),
        Step::Emulators => emulators(board, s),
        Step::Proton => proton(board, s),
        Step::Screenshots => screenshots(board, s),
        Step::Saves => saves(board, s),
        Step::Tray => tray(board, s),
        Step::Palette => palette(board, s),
        Step::Statuses => statuses(board, s),
        Step::Appearance => appearance(board, s),
        Step::Stats => stats(board, s),
        Step::Console => console(board, s),
    }
}

fn steam(board: Div, s: &Sc) -> Div {
    let t = s.t;
    let n = if s.a.steam_games > 0 { s.a.steam_games } else { 27 };
    let covers = (0..12).map(|i| {
        let start = 0.03 + i as f32 * 0.022;
        let a = fade(t, start, start + 0.09, 0.88, 0.95);
        s.cover(i, 38. + (i % 6) as f32 * 76., 62. + (i / 6) as f32 * 108. - (1. - ramp(t, start, start + 0.09)) * 22., 64., 96.).opacity(a)
    });
    board
        .child(s.pill(20., 18., tr!("Steam is open"), true))
        .child(s.at(440., 22.).child(s.text(12., theme::text_muted(), tr!("Collection"))))
        .children(covers)
        .child(s.toast(
            Icon::Check,
            theme::success(),
            trn!(n, "{n} game from Steam joined the Collection", "{n} games from Steam joined the Collection"),
            fade(t, 0.52, 0.58, 0.88, 0.94),
            false,
        ))
}

fn stores(board: Div, s: &Sc) -> Div {
    let t = s.t;
    let tabs = s.at(20., 18.).flex().gap(s.p(4.)).p(s.p(3.)).rounded_full().bg(theme::white_a(0.06)).children(
        ["Epic Games", "GOG", "Amazon", "Steam"].into_iter().enumerate().map(|(i, name)| {
            div().px(s.p(11.)).py(s.p(4.)).rounded_full().when(i == 0, |d| d.bg(theme::white_a(0.14))).child(s.text(
                11.5,
                if i == 0 { theme::text_primary() } else { theme::text_muted() },
                name,
            ))
        }),
    );
    let logged = t > 0.45;
    let button = if logged {
        div()
            .flex()
            .items_center()
            .justify_center()
            .gap(s.p(6.))
            .h(s.p(32.))
            .rounded_full()
            .bg(theme::rgb_a(95, 211, 155, 0.13))
            .border_1()
            .border_color(theme::rgb_a(95, 211, 155, 0.35))
            .child(s.ic(Icon::Check, 14., theme::success()))
            .child(s.text(13., theme::success(), tr!("Logged in")).font_weight(theme::W_BOLD))
    } else {
        s.button(tr!("Log in with Epic Games"), true).h(s.p(32.))
    };
    let card = s
        .panel(110., 64., 300., 118.)
        .rounded(s.p(16.))
        .child(
            s.rect(18., 18., 34., 34.)
                .rounded(s.p(10.))
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::rgb_a(165, 220, 235, 0.14))
                .child(s.ic(Icon::Storefront, 19., theme::accent())),
        )
        .child(s.at(64., 16.).child(s.heading(16., "Epic Games")))
        .child(s.at(64., 40.).child(s.text(12., theme::text_muted(), tr!("Log in to see your library"))))
        .child(div().absolute().left(s.p(18.)).right(s.p(18.)).top(s.p(72.)).child(button));
    let covers = (0..7).map(|i| {
        let a = fade(t, 0.52 + i as f32 * 0.012, 0.6 + i as f32 * 0.012, 0.9, 0.97);
        s.cover(i + 3, 48. + i as f32 * 62., 206. + (1. - a) * 12., 52., 78.).opacity(a)
    });
    let (cx, cy) = if t < 0.3 {
        let x = ramp(t, 0.06, 0.3);
        (lerp(430., 252., x), lerp(280., 146., x))
    } else {
        let x = ramp(t, 0.62, 0.74);
        (lerp(252., 480., x), lerp(146., 300., x))
    };
    let press = fade(t, 0.33, 0.36, 0.38, 0.41);
    board
        .child(tabs)
        .child(card)
        .child(s.at(48., 184.).opacity(fade(t, 0.5, 0.58, 0.9, 0.97)).child(s.text(12., theme::text_muted(), tr!("Your Epic library"))))
        .children(covers)
        .child(s.cursor(cx, cy, press, fade(t, 0.02, 0.08, 0.66, 0.74)))
}

fn emulators(board: Div, s: &Sc) -> Div {
    let t = s.t;
    let rows = [("RetroArch", "Flatpak"), ("PCSX2", "~/Applications"), ("Dolphin", "/usr/bin"), ("PPSSPP", "Flatpak")]
        .into_iter()
        .enumerate()
        .map(|(i, (name, place))| {
            let a = fade(t, 0.02 + i as f32 * 0.05, 0.08 + i as f32 * 0.05, 0.92, 0.98);
            let found = t > 0.2 + i as f32 * 0.07;
            div()
                .absolute()
                .left(s.p(20.))
                .right(s.p(20.))
                .top(s.p(68. + i as f32 * 40.))
                .h(s.p(34.))
                .opacity(a)
                .flex()
                .items_center()
                .gap(s.p(10.))
                .px(s.p(12.))
                .rounded(s.p(10.))
                .bg(theme::white_a(0.045))
                .border_1()
                .border_color(theme::white_a(0.07))
                .child(s.ic(Icon::SportsEsports, 17., warm()))
                .child(s.text(13.5, theme::text_primary(), name).font_weight(theme::W_BOLD))
                .child(s.text(11.5, theme::text_dim(), place))
                .child(div().ml_auto().child(if found { s.check(20., 1.).into_any_element() } else { s.spinner(16.).into_any_element() }))
        });
    board
        .child(s.at(20., 16.).child(s.heading(16., tr!("Detect emulators"))))
        .child(s.at(20., 40.).child(s.text(11.5, theme::text_dim(), tr!("Looking in PATH, Flatpak and ~/Applications"))))
        .children(rows)
        .child(s.at(20., 240.).child(s.text(12., theme::text_muted(), tr!("Scanning ~/ROMs for games"))))
        .child(s.bar(20., 262., 480., 8., ramp(t, 0.46, 0.8)).opacity(1. - ramp(t, 0.94, 0.99)))
        .child(
            s.at(20., 282.)
                .flex()
                .items_center()
                .gap(s.p(6.))
                .opacity(fade(t, 0.8, 0.84, 0.94, 0.99))
                .child(s.ic(Icon::Check, 15., theme::success()))
                .child(s.text(13., theme::success(), tr!("142 ROMs from 6 systems")).font_weight(theme::W_BOLD)),
        )
}

fn proton(board: Div, s: &Sc) -> Div {
    let t = s.t;
    let row = |y: f32, name: &'static str, size: &'static str, dim: bool| {
        s.panel(20., y, 480., 44.)
            .opacity(if dim { 0.55 } else { 1. })
            .child(s.at(14., 6.).child(s.heading(14., name)))
            .child(s.at(14., 25.).child(s.text(11.5, theme::text_dim(), size)))
    };
    let right = if t < 0.19 {
        s.at(392., 7.).child(s.button(tr!("Download"), true))
    } else if t < 0.62 {
        s.bar(316., 18., 150., 8., ramp(t, 0.2, 0.6))
    } else {
        s.at(300., 10.)
            .flex()
            .items_center()
            .gap(s.p(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(s.p(5.))
                    .child(s.ic(Icon::Check, 14., theme::success()))
                    .child(s.text(12.5, theme::success(), tr!("Installed")).font_weight(theme::W_BOLD)),
            )
            .child(
                div()
                    .px(s.p(10.))
                    .py(s.p(3.))
                    .rounded_full()
                    .bg(theme::rgb_a(95, 211, 155, 0.16))
                    .border_1()
                    .border_color(theme::rgb_a(95, 211, 155, 0.4))
                    .opacity(fade(t, 0.64, 0.68, 0.95, 0.99))
                    .child(s.text(11.5, theme::success(), tr!("Default")).font_weight(theme::W_BOLD)),
            )
    };
    let tabs = s.at(20., 46.).flex().gap(s.p(4.)).p(s.p(3.)).rounded_full().bg(theme::white_a(0.06)).children(
        ["GE-Proton", "Proton-CachyOS", "Wine-GE"].into_iter().enumerate().map(|(i, name)| {
            div().px(s.p(11.)).py(s.p(4.)).rounded_full().when(i == 0, |d| d.bg(theme::white_a(0.14))).child(s.text(
                11.5,
                if i == 0 { theme::text_primary() } else { theme::text_muted() },
                name,
            ))
        }),
    );
    let ghost = |y: f32| s.at(410., y + 7.).child(s.button(tr!("Download"), false));
    let (cx, cy) = if t < 0.2 {
        let x = ramp(t, 0.03, 0.13);
        (lerp(430., 440., x), lerp(300., 104., x))
    } else {
        let x = ramp(t, 0.3, 0.4);
        (lerp(440., 500., x), lerp(104., 310., x))
    };
    board
        .child(s.at(20., 16.).child(s.heading(16., tr!("Wine Manager"))))
        .child(tabs)
        .child(row(86., "GE-Proton10-25", "512 MB", false).child(right))
        .child(row(138., "GE-Proton10-24", "509 MB", true).child(ghost(0.).left(s.p(390.))))
        .child(row(190., "GE-Proton9-27", "498 MB", true).child(ghost(0.).left(s.p(390.))))
        .child(s.toast(Icon::Check, theme::success(), tr!("GE-Proton10-25 is now the default"), fade(t, 0.68, 0.73, 0.92, 0.98), false))
        .child(s.cursor(cx, cy, fade(t, 0.14, 0.16, 0.17, 0.19), fade(t, 0.01, 0.05, 0.32, 0.4)))
}

fn screenshots(board: Div, s: &Sc) -> Div {
    let t = s.t;
    let src = s.a.stage.clone().or_else(|| s.a.covers.first().cloned().flatten());
    // The picture flies from the whole screen into the timeline.
    let x = ramp(t, 0.38, 0.6);
    let (sx, sy, sw, sh) = (lerp(10., 262., x), lerp(8., 254., x), lerp(500., 72., x), lerp(304., 44., x));
    let snap = s
        .picture(src.clone(), sx, sy, sw, sh)
        .rounded(s.p(lerp(16., 5., x)))
        .border(s.p(lerp(5., 2., x)))
        .border_color(white())
        .opacity(fade(t, 0.34, 0.37, 0.93, 0.98));
    let strip = s
        .panel(18., 238., 330., 66.)
        .bg(theme::rgb_a(10, 13, 18, 0.85))
        .opacity(fade(t, 0.36, 0.42, 0.93, 0.98))
        .child(s.at(10., 5.).child(s.text(10.5, theme::text_muted(), tr!("Timeline"))))
        .children((0..3).map(|i| s.cover(i + 1, 10. + i as f32 * 78., 20., 72., 40.).rounded(s.p(5.))));
    board
        .child(s.picture(src, 0., 0., W, H))
        .child(div().absolute().inset_0().bg(linear_gradient(
            180.,
            linear_color_stop(theme::black_a(0.0), 0.5),
            linear_color_stop(theme::black_a(0.55), 1.),
        )))
        .child(s.pill(18., 16., format!("{} \u{b7} 0:42:10", s.a.title(0)), false))
        .child(s.key(448., 262., s.a.shot_key.clone(), fade(t, 0.27, 0.29, 0.32, 0.35)))
        .child(div().absolute().inset_0().bg(white()).opacity(0.85 * fade(t, 0.3, 0.32, 0.33, 0.44)))
        .child(strip)
        .child(snap)
        .child(s.toast(Icon::PhotoCamera, theme::accent(), tr!("Screenshot saved"), fade(t, 0.6, 0.65, 0.9, 0.96), true))
}

fn saves(board: Div, s: &Sc) -> Div {
    let t = s.t;
    let backed = t > 0.48;
    let new_row = ramp(t, 0.5, 0.58) * (1. - ramp(t, 0.95, 0.99));
    let hist_row = |when: SharedString, size: &'static str, tag: bool| {
        div()
            .flex()
            .items_center()
            .gap(s.p(10.))
            .h(s.p(26.))
            .px(s.p(12.))
            .rounded(s.p(8.))
            .bg(theme::white_a(0.035))
            .child(s.text(12.5, theme::text_primary(), when))
            .when(tag, |d| {
                d.child(div().px(s.p(7.)).rounded_full().bg(theme::rgb_a(95, 211, 155, 0.16)).child(s.text(
                    10.5,
                    theme::success(),
                    tr!("new"),
                )))
            })
            .child(div().ml_auto().child(s.text(11.5, theme::text_dim(), size)))
    };
    let circle = s
        .rect(228., 86., 64., 64.)
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(0x0d1218))
        .border(s.p(3.))
        .border_color(if backed { theme::success().into() } else { theme::white_a(0.12) })
        .child(if backed { s.ic(Icon::Check, 30., theme::success()) } else { s.ic(Icon::CloudSync, 26., theme::accent()) });
    board
        .child(
            s.panel(20., 16., 480., 56.)
                .child(s.cover(0, 10., 6., 30., 44.).rounded(s.p(5.)))
                .child(s.at(52., 9.).child(s.heading(14., s.a.title(0))))
                .child(s.at(52., 31.).child(s.text(12., theme::text_muted(), tr!("Session ended \u{b7} 1h 12min")))),
        )
        .child(circle)
        .child(s.bar(200., 158., 120., 5., ramp(t, 0.1, 0.46)).opacity(1. - ramp(t, 0.46, 0.5)))
        .child(div().absolute().left_0().right_0().top(s.p(166.)).flex().justify_center().child(if backed {
            s.text(13., theme::success(), tr!("Backed up with Ludusavi")).font_weight(theme::W_BOLD)
        } else {
            s.text(13., theme::text_muted(), tr!("Backing up the save\u{2026}"))
        }))
        .child(
            s.at(20., 196.)
                .w(s.p(480.))
                .flex()
                .flex_col()
                .gap(s.p(4.))
                .child(s.text(11., theme::text_dim(), tr!("Backups")))
                .child(hist_row(tr!("Today").into(), "2.1 MB", true).h(s.p(26. * new_row)).opacity(new_row).overflow_hidden())
                .child(hist_row(tr!("Yesterday").into(), "2.1 MB", false))
                .child(hist_row(tr!("Last week").into(), "2.0 MB", false)),
        )
}

fn tray(board: Div, s: &Sc) -> Div {
    let t = s.t;
    let gone = ramp(t, 0.18, 0.34) * (1. - ramp(t, 0.96, 1.));
    let (wx, wy, ww, wh) = (lerp(110., 470., gone), lerp(58., 6., gone), lerp(300., 14., gone), lerp(190., 10., gone));
    let window = s
        .picture(s.a.stage.clone().or_else(|| s.a.covers.first().cloned().flatten()), wx, wy, ww, wh)
        .rounded(s.p(10.))
        .border_1()
        .border_color(theme::white_a(0.12))
        .opacity(1. - ramp(t, 0.28, 0.34) + ramp(t, 0.96, 1.))
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(s.p(20. * (1. - gone)))
                .bg(theme::black_a(0.55))
                .flex()
                .justify_end()
                .gap(s.p(6.))
                .px(s.p(8.))
                .items_center()
                .child(div().size(s.p(9.)).rounded_full().bg(theme::white_a(0.25)))
                .child(div().size(s.p(9.)).rounded_full().bg(rgb(0xf07f86))),
        );
    let glow = fade(t, 0.34, 0.38, 0.44, 0.48);
    let menu = s.menu(
        340.,
        30.,
        160.,
        vec![
            s.menu_row(None, tr!("Open Heronite"), false, None),
            div().h(px(1.)).my(s.p(3.)).bg(theme::white_a(0.08)),
            s.menu_row(None, s.a.title(0), false, None),
            s.menu_row(None, s.a.title(1), false, None),
            div().h(px(1.)).my(s.p(3.)).bg(theme::white_a(0.08)),
            s.menu_row(None, tr!("Quit"), false, None),
        ],
        fade(t, 0.48, 0.51, 0.64, 0.68),
    );
    let note = s
        .at(274., 238.)
        .w(s.p(230.))
        .opacity(fade(t, 0.68, 0.73, 0.92, 0.98))
        .p(s.p(10.))
        .rounded(s.p(12.))
        .bg(theme::rgb_a(22, 27, 35, 0.97))
        .border_1()
        .border_color(theme::white_a(0.13))
        .flex()
        .flex_col()
        .gap(s.p(3.))
        .child(s.heading(12.5, "Heronite").text_color(theme::accent()))
        .child(s.text(12., rgb(0xc9d2de), tr!("{title} is ready to play.", title = s.a.title(0))).whitespace_normal());
    let (cx, cy) = if t < 0.2 {
        let x = ramp(t, 0.02, 0.12);
        (lerp(300., 397., x), lerp(280., 62., x))
    } else if t < 0.5 {
        let x = ramp(t, 0.2, 0.28);
        (lerp(397., 470., x), lerp(62., 14., x))
    } else {
        let x = ramp(t, 0.5, 0.6);
        (lerp(470., 480., x), lerp(14., 130., x))
    };
    let press = fade(t, 0.13, 0.15, 0.16, 0.18).max(fade(t, 0.43, 0.45, 0.46, 0.49));
    board
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(s.p(26.))
                .flex()
                .items_center()
                .justify_between()
                .px(s.p(12.))
                .bg(theme::black_a(0.55))
                .child(s.text(11., theme::text_muted(), "1  2  3"))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(s.p(10.))
                        .child(
                            div()
                                .size(s.p(16.))
                                .rounded(s.p(4.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(theme::accent())
                                .shadow(vec![BoxShadow {
                                    color: theme::accent().into(),
                                    offset: point(px(0.), px(0.)),
                                    blur_radius: s.p(14. * glow),
                                    spread_radius: px(0.),
                                    inset: false,
                                }])
                                .child(s.text(10., theme::on_accent(), "H").font_weight(theme::W_BOLD)),
                        )
                        .child(s.text(11., theme::text_muted(), "21:40")),
                ),
        )
        .child(window)
        .child(menu)
        .child(note)
        .child(s.cursor(cx, cy, press, fade(t, 0.01, 0.04, 0.55, 0.6)))
}

fn palette(board: Div, s: &Sc) -> Div {
    let t = s.t;
    let title = s.a.title(0);
    let query: String = title.chars().filter(|c| c.is_alphanumeric()).take(3).collect::<String>().to_lowercase();
    let typed = query.chars().take(((t - 0.2) / 0.05).floor().clamp(0., 3.) as usize).collect::<String>();
    let caret = (t * 13.).floor() as i32 % 2 == 0;
    let results = fade(t, 0.32, 0.36, 0.93, 0.99);
    let enter = fade(t, 0.68, 0.7, 0.72, 0.76);
    let result = |ic: Option<Icon>, cover: bool, label: SharedString, hint: SharedString, on: bool| {
        div()
            .flex()
            .items_center()
            .gap(s.p(10.))
            .h(s.p(42.))
            .px(s.p(10.))
            .rounded(s.p(10.))
            .when(on, |d| d.bg(theme::rgb_a(165, 220, 235, 0.12 + 0.2 * enter)).border_1().border_color(theme::rgb_a(165, 220, 235, 0.3)))
            .when(cover, |d| d.child(s.cover(0, 0., 0., 26., 36.).relative().left_0().top_0().rounded(s.p(4.))))
            .children(ic.map(|i| div().w(s.p(26.)).flex().justify_center().child(s.ic(i, 16., violet()))))
            .child(s.text(13.5, theme::text_primary(), label))
            .child(div().ml_auto().child(s.text(11.5, theme::text_dim(), hint)))
    };
    let section = |name: &'static str| div().px(s.p(10.)).pt(s.p(6.)).pb(s.p(4.)).child(s.text(10.5, theme::text_dim(), name));
    let pal = s
        .at(60., 28. - (1. - ramp(t, 0.12, 0.16)) * 6.)
        .w(s.p(400.))
        .opacity(fade(t, 0.12, 0.16, 0.94, 0.99))
        .p(s.p(8.))
        .rounded(s.p(16.))
        .bg(theme::rgb_a(20, 25, 33, 0.97))
        .border_1()
        .border_color(theme::white_a(0.14))
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap(s.p(9.))
                .h(s.p(40.))
                .px(s.p(12.))
                .rounded(s.p(10.))
                .bg(theme::white_a(0.05))
                .child(s.ic(Icon::FaSearch, 13., theme::text_dim()))
                .child(s.text(15., theme::text_primary(), typed))
                .child(div().w(px(2.)).h(s.p(18.)).bg(theme::accent()).opacity(if caret { 1. } else { 0. })),
        )
        .child(
            div()
                .opacity(results)
                .flex()
                .flex_col()
                .child(section(tr!("Games")))
                .child(result(None, true, title.clone(), tr!("Play").into(), true))
                .child(section(tr!("Actions")))
                .child(result(Some(Icon::Settings), false, tr!("{title}: game settings", title = title.clone()).into(), "".into(), false))
                .child(result(
                    Some(Icon::FolderOpen),
                    false,
                    tr!("{title}: install folder", title = title.clone()).into(),
                    "".into(),
                    false,
                )),
        );
    let keys = s
        .at(196., 266.)
        .flex()
        .gap(s.p(8.))
        .opacity(fade(t, 0.01, 0.04, 0.13, 0.17))
        .child(s.key(0., 0., "Ctrl", fade(t, 0.06, 0.07, 0.09, 0.1)).relative())
        .child(s.key(0., 0., "K", fade(t, 0.06, 0.07, 0.09, 0.1)).relative());
    board.children((0..6).map(|i| s.cover(i, 24. + i as f32 * 82., 70., 72., 108.).opacity(0.18))).child(keys).child(pal).child(s.toast(
        Icon::PlayArrow,
        theme::accent(),
        tr!("Starting {title}\u{2026}", title = title.clone()),
        fade(t, 0.74, 0.78, 0.92, 0.98),
        false,
    ))
}

fn statuses(board: Div, s: &Sc) -> Div {
    let t = s.t;
    let names = [tr!("Playing"), tr!("Beaten"), tr!("Plan to Play"), tr!("On Hold"), tr!("Abandoned")];
    let m1 = fade(t, 0.17, 0.2, 0.5, 0.53);
    let m2 = fade(t, 0.31, 0.34, 0.5, 0.53);
    let hl = theme::white_a(0.08);
    let menu1 = s.menu(
        188.,
        96.,
        170.,
        vec![
            s.menu_row(Some(Icon::PlayArrow), tr!("Play"), false, None),
            s.menu_row(Some(Icon::FavoriteBorder), tr!("Favourite"), false, None),
            s.menu_row(Some(Icon::Flag), tr!("Status"), true, (t > 0.28).then_some(hl)),
            s.menu_row(Some(Icon::Star), tr!("Rate"), true, None),
            s.menu_row(Some(Icon::Settings), tr!("Settings"), false, None),
        ],
        m1,
    );
    let menu2 = s.menu(
        364.,
        150.,
        136.,
        names
            .iter()
            .enumerate()
            .map(|(i, n)| s.menu_row(None, *n, false, (i == 1 && t > 0.41).then(|| theme::rgb_a(95, 211, 155, 0.2).into())))
            .collect(),
        m2,
    );
    let badge = fade(t, 0.56, 0.6, 0.94, 0.99);
    let stars = ramp(t, 0.62, 0.76) * (1. - ramp(t, 0.95, 0.99));
    let star_row = |c: Hsla, ic: Icon| div().flex().gap(s.p(2.)).children((0..5).map(move |_| s.ic(ic, 20., c)));
    let (cx, cy) = if t < 0.27 {
        let x = ramp(t, 0.02, 0.12);
        (lerp(420., 130., x), lerp(290., 116., x))
    } else if t < 0.4 {
        let x = ramp(t, 0.27, 0.31);
        (lerp(130., 236., x), lerp(116., 176., x))
    } else if t < 0.56 {
        let x = ramp(t, 0.4, 0.44);
        (lerp(236., 404., x), lerp(176., 208., x))
    } else {
        let x = ramp(t, 0.56, 0.68);
        (lerp(404., 480., x), lerp(208., 300., x))
    };
    let press = fade(t, 0.12, 0.13, 0.15, 0.17).max(fade(t, 0.45, 0.46, 0.48, 0.5));
    board
        .child(s.cover(0, 60., 36., 120., 180.).rounded(s.p(10.)))
        .child(
            s.at(68., 184.)
                .opacity(badge)
                .flex()
                .items_center()
                .gap(s.p(6.))
                .px(s.p(10.))
                .py(s.p(3.))
                .rounded_full()
                .bg(theme::rgb_a(10, 13, 18, 0.85))
                .border_1()
                .border_color(theme::rgb_a(95, 211, 155, 0.45))
                .child(div().size(s.p(7.)).rounded_full().bg(theme::success()))
                .child(s.text(11.5, theme::success(), names[1]).font_weight(theme::W_BOLD)),
        )
        .child(s.at(62., 228.).child(star_row(rgb(0x4a5462).into(), Icon::StarBorder)))
        .child(s.at(62., 228.).w(s.p(110. * 0.8 * stars)).overflow_hidden().child(star_row(warm().into(), Icon::Star)))
        .child(menu1)
        .child(menu2)
        .child(s.cursor(cx, cy, press, fade(t, 0.01, 0.04, 0.6, 0.68)))
}

fn appearance(board: Div, s: &Sc) -> Div {
    let t = s.t;
    // The background darkens, the panels turn solid in three colours, the
    // covers turn square; a small settings panel shows what changes.
    let dark = 0.25 + 0.45 * fade(t, 0.06, 0.18, 0.88, 0.96);
    let solid = fade(t, 0.3, 0.34, 0.88, 0.94);
    let tint = [rgb(0x1d2a44), rgb(0x2d2346), rgb(0x173a35)][if t < 0.42 {
        0
    } else if t < 0.52 {
        1
    } else {
        2
    }];
    let square = fade(t, 0.62, 0.7, 0.88, 0.96);
    let ch = lerp(78., 52., square);
    let surface = |x: f32, y: f32, w: f32, h: f32| {
        s.rect(x, y, w, h)
            .rounded(s.p(12.))
            .overflow_hidden()
            .border_1()
            .border_color(theme::white_a(0.12))
            .child(div().absolute().inset_0().bg(theme::white_a(0.07)).opacity(1. - solid))
            .child(div().absolute().inset_0().bg(tint).opacity(solid))
    };
    let setting = |label: SharedString, value: SharedString, on: bool| {
        div()
            .flex()
            .flex_col()
            .gap(s.p(2.))
            .px(s.p(10.))
            .py(s.p(7.))
            .rounded(s.p(8.))
            .when(on, |d| d.bg(theme::rgb_a(165, 220, 235, 0.14)).border_1().border_color(theme::rgb_a(165, 220, 235, 0.35)))
            .child(s.text(10.5, theme::text_dim(), label))
            .child(s.text(12., if on { theme::accent() } else { theme::text_primary() }, value).font_weight(theme::W_BOLD))
    };
    let skeleton = |y: f32, w: f32| s.rect(14., y, w, 8.).rounded_full().bg(theme::white_a(0.12));
    let panels: SharedString = if solid > 0.5 { tr!("Solid").into() } else { tr!("Transparent").into() };
    let aspect: SharedString = if square > 0.5 { heronite_i18n::tr("Square (1:1)").into() } else { "Steam (2:3)".into() };
    board
        .child(s.picture(s.a.stage.clone().or_else(|| s.a.covers.first().cloned().flatten()), 0., 0., W, H))
        .child(div().absolute().inset_0().bg(black()).opacity(dark))
        .child(
            surface(14., 14., 150., 292.).children(
                (0..6).map(|i| s.cover(i, 14. + (i % 2) as f32 * 68., 14. + (i / 2) as f32 * (ch + 12.), 60., ch).rounded(s.p(6.))),
            ),
        )
        .child(
            surface(176., 40., 184., 210.)
                .child(s.cover(0, 14., 14., 52., 78.).rounded(s.p(6.)))
                .child(s.at(76., 16.).w(s.p(96.)).child(s.heading(13., s.a.title(0)).whitespace_normal()))
                .child(skeleton(104., 150.))
                .child(skeleton(120., 130.))
                .child(skeleton(136., 140.))
                .child(
                    s.rect(14., 166., 156., 28.).rounded_full().bg(white()).child(
                        div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(s.text(12., rgb(0x0a0e13), tr!("Play")).font_weight(theme::W_BOLD)),
                    ),
                ),
        )
        .child(
            s.rect(372., 40., 134., 210.)
                .rounded(s.p(12.))
                .bg(theme::rgb_a(18, 22, 28, 0.94))
                .border_1()
                .border_color(theme::white_a(0.14))
                .p(s.p(8.))
                .flex()
                .flex_col()
                .gap(s.p(6.))
                .child(div().px(s.p(2.)).child(s.text(11., theme::text_muted(), tr!("Appearance"))))
                .child(setting(
                    tr!("Background").into(),
                    tr!("Darkening: {value}%", value = (dark * 100.).round()).into(),
                    t > 0.04 && t < 0.29,
                ))
                .child(setting(tr!("Panels").into(), panels, t > 0.29 && t < 0.61))
                .child(setting(tr!("Cover aspect").into(), aspect, t > 0.61 && t < 0.87)),
        )
}

fn stats(board: Div, s: &Sc) -> Div {
    let t = s.t;
    let hours = (18. * ramp(t, 0.05, 0.35)).round() as u32;
    let heights = [42., 26., 88., 18., 30., 46., 34., 24., 6., 55., 62., 20., 38., 70.];
    let bars = heights.iter().enumerate().map(|(i, h)| {
        let g = ramp(t, 0.08 + i as f32 * 0.012, 0.22 + i as f32 * 0.012) * (1. - ramp(t, 0.95, 0.99));
        let last = i == heights.len() - 1;
        div().flex_1().h(s.p(160. * h / 100. * g)).rounded_t(s.p(5.)).bg(if last { warm() } else { theme::accent() }).opacity(if last {
            1.
        } else {
            0.85
        })
    });
    let slide = ramp(t, 0.46, 0.58) * (1. - ramp(t, 0.95, 0.99));
    let year = chrono::Datelike::year(&chrono::Local::now());
    let wrapped = s
        .rect(336. + (1. - slide) * 220., 28., 164., 262.)
        .opacity(slide)
        .p(s.p(16.))
        .rounded(s.p(18.))
        .bg(linear_gradient(160., linear_color_stop(rgb(0x6b4ae0), 0.), linear_color_stop(rgb(0xe7a35b), 1.)))
        .flex()
        .flex_col()
        .child(s.text(10.5, theme::white_a(0.8), tr!("Year in review")))
        .child(div().mt(s.p(10.)).child(s.heading(21., tr!("Your {year} in games", year)).whitespace_normal()))
        .child(div().mt(s.p(8.)).child(s.text(12., theme::white_a(0.85), tr!("Seven stories about how you played.")).whitespace_normal()))
        .child(
            div().mt_auto().flex().child(
                div()
                    .flex()
                    .items_center()
                    .gap(s.p(5.))
                    .px(s.p(12.))
                    .py(s.p(6.))
                    .rounded_full()
                    .bg(theme::white_a(0.92))
                    .child(s.ic(Icon::PlayArrow, 13., rgb(0x2a1840)))
                    .child(s.text(12., rgb(0x2a1840), tr!("Watch")).font_weight(theme::W_BOLD)),
            ),
        );
    board
        .child(s.at(26., 20.).child(s.text(12., theme::text_muted(), tr!("This month"))))
        // GPUI has no baseline alignment: both lines end on the same bottom.
        .child(
            s.at(24., 36.)
                .h(s.p(46.))
                .flex()
                .items_end()
                .gap(s.p(6.))
                .child(s.heading(44., format!("{hours}h")).line_height(s.p(46.)))
                .child(s.text(22., theme::text_muted(), "43min").line_height(s.p(22.)).pb(s.p(5.))),
        )
        .child(s.at(26., 94.).child(s.text(12.5, theme::text_muted(), tr!("9 games \u{b7} a 6-day streak"))))
        .child(s.rect(26., 130., 290., 160.).flex().items_end().gap(s.p(5.)).children(bars))
        .child(wrapped)
}

fn console(board: Div, s: &Sc) -> Div {
    let t = s.t;
    // The focus moves right twice, then A starts the game.
    let focus = ramp(t, 0.16, 0.22) + ramp(t, 0.36, 0.42);
    let press = fade(t, 0.7, 0.72, 0.76, 0.8);
    let shown = (focus.round() as usize).min(2);
    let hour = chrono::Timelike::hour(&chrono::Local::now());
    let greeting = if hour < 12 {
        tr!("Good morning")
    } else if hour < 18 {
        tr!("Good afternoon")
    } else {
        tr!("Good evening")
    };
    let pad = |label: &'static str, c: Rgba, glow: f32| {
        div()
            .size(s.p(20. * (1. + 0.25 * glow)))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(c)
            .child(s.text(11., rgb(0x05140c), label).font_weight(theme::W_BOLD))
    };
    board
        .bg(linear_gradient(180., linear_color_stop(rgb(0x1b2638), 0.), linear_color_stop(rgb(0x06080b), 0.75)))
        .child(s.at(28., 20.).child(s.heading(28., greeting)))
        .child(s.at(28., 58.).child(s.text(12.5, theme::text_muted(), tr!("Continue playing"))))
        .children((0..5).map(|i| s.cover(i, 28. + i as f32 * 98., 90., 84., 126.).rounded(s.p(10.))))
        .child(
            s.rect(24. + focus * 98. + 4. * press, 86. + 4. * press, 92. - 8. * press, 134. - 8. * press)
                .rounded(s.p(13.))
                .border(s.p(3.))
                .border_color(theme::accent())
                .shadow(vec![BoxShadow {
                    color: theme::rgb_a(165, 220, 235, 0.55).into(),
                    offset: point(px(0.), px(0.)),
                    blur_radius: s.p(24.),
                    spread_radius: px(0.),
                    inset: false,
                }]),
        )
        .child(s.at(28., 232.).child(s.heading(15., s.a.title(shown))))
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(s.p(40.))
                .flex()
                .items_center()
                .gap(s.p(22.))
                .px(s.p(28.))
                .bg(theme::black_a(0.4))
                .child(div().flex().items_center().gap(s.p(6.)).child(s.text(12., theme::text_muted(), tr!("\u{2190} \u{2192} Navigate"))))
                .child(div().flex().items_center().gap(s.p(6.)).child(pad("A", rgb(0x5fd39b), press)).child(s.text(
                    12.,
                    theme::text_muted(),
                    tr!("Play"),
                )))
                .child(div().flex().items_center().gap(s.p(6.)).child(pad("B", rgb(0xf07f86), 0.)).child(s.text(
                    12.,
                    theme::text_muted(),
                    tr!("Back"),
                ))),
        )
        .child(
            s.toast(
                Icon::PlayArrow,
                theme::accent(),
                tr!("Starting {title}\u{2026}", title = s.a.title(2)),
                fade(t, 0.76, 0.8, 0.92, 0.98),
                false,
            )
            .bottom(s.p(52.)),
        )
}
