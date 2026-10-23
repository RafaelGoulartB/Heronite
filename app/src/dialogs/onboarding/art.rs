//! The pictures of "Getting started", drawn so they never need games: until
//! the library has art, the Collection preview is the bare layout (neutral
//! cover slots, bars for the text, a glow of the accent for the art), and
//! empty covers or a landscape stand in elsewhere. The Collection preview
//! reads the current appearance, so it changes with every choice; the
//! overlay preview is the panel the Guide button opens.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::appearance::{BackgroundSource, InterfaceMode};
use heronite_i18n::tr;

use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// Up to this many covers and titles of the most played games.
#[derive(Clone, Default)]
pub struct Art {
    /// `None` while loading or missing.
    pub covers: Vec<Option<ImageSource>>,
    pub titles: Vec<SharedString>,
    /// Background art of the most played game.
    pub stage: Option<ImageSource>,
}

impl Art {
    pub fn has_games(&self) -> bool {
        !self.titles.is_empty()
    }

    fn cover(&self, i: usize) -> Option<ImageSource> {
        self.covers.get(i).cloned().flatten()
    }
}

/// Lengths on a board `k` times its design size.
#[derive(Clone, Copy)]
struct Board {
    k: f32,
}

impl Board {
    fn p(self, v: f32) -> Pixels {
        px(v * self.k)
    }

    fn at(self, x: f32, y: f32) -> Div {
        div().absolute().left(self.p(x)).top(self.p(y))
    }

    fn rect(self, x: f32, y: f32, w: f32, h: f32) -> Div {
        self.at(x, y).w(self.p(w)).h(self.p(h))
    }

    fn text(self, size: f32, color: impl Into<Hsla>, s: impl Into<SharedString>) -> Div {
        div().text_size(self.p(size)).text_color(color.into()).whitespace_nowrap().overflow_hidden().text_ellipsis().child(s.into())
    }
}

/// A game in the overlay preview while there is no art: a sky with a sun
/// over three ridges.
fn landscape() -> Div {
    let (top, mid, low, sun) = (rgb(0x0f2030), rgb(0x1d4a5b), rgb(0x7cc6c0), rgb(0xe7fbff));
    let ridges: [(Rgba, &[(f32, f32)]); 3] = [
        (rgb(0x163646), &[(0., 0.5), (0.14, 0.36), (0.3, 0.52), (0.45, 0.3), (0.6, 0.48), (0.76, 0.34), (1., 0.46)]),
        (rgb(0x0f2733), &[(0., 0.66), (0.2, 0.76), (0.38, 0.6), (0.56, 0.74), (0.72, 0.62), (0.88, 0.75), (1., 0.66)]),
        (rgb(0x08141b), &[(0., 0.84), (0.3, 0.9), (0.62, 0.8), (1., 0.88)]),
    ];
    div()
        .absolute()
        .inset_0()
        .overflow_hidden()
        .bg(linear_gradient(180., linear_color_stop(top, 0.), linear_color_stop(mid, 0.45)))
        .child(div().absolute().left_0().right_0().bottom_0().h(relative(0.6)).bg(linear_gradient(
            180.,
            linear_color_stop(theme::black_a(0.), 0.),
            linear_color_stop(low, 1.),
        )))
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let at = |(x, y): (f32, f32)| bounds.origin + point(bounds.size.width * x, bounds.size.height * y);
                    let centre = at((0.28, 0.3));
                    for (radius, alpha) in [(0.2, 0.12), (0.15, 0.2), (0.11, 1.)] {
                        let r = f32::from(bounds.size.height) * radius;
                        let mut path = PathBuilder::fill();
                        for s in 0..48 {
                            let a = s as f32 / 48. * std::f32::consts::TAU;
                            let p = centre + point(px(r * a.cos()), px(r * a.sin()));
                            if s == 0 {
                                path.move_to(p);
                            } else {
                                path.line_to(p);
                            }
                        }
                        path.close();
                        if let Ok(path) = path.build() {
                            window.paint_path(path, Hsla { a: alpha, ..sun.into() });
                        }
                    }
                    for (color, line) in ridges {
                        let mut path = PathBuilder::fill();
                        path.move_to(at((0., 1.)));
                        for p in line {
                            path.line_to(at(*p));
                        }
                        path.line_to(at((1., 1.)));
                        path.close();
                        if let Ok(path) = path.build() {
                            window.paint_path(path, color);
                        }
                    }
                },
            )
            .absolute()
            .inset_0(),
        )
}

/// Where a cover goes while there are no games: a neutral block.
pub fn slot(w: f32, h: f32, radius: f32) -> Div {
    div()
        .flex_none()
        .w(px(w))
        .h(px(h))
        .rounded(px(radius))
        .bg(linear_gradient(180., linear_color_stop(theme::ink_a(0.1), 0.), linear_color_stop(theme::ink_a(0.04), 1.)))
        .border_1()
        .border_color(theme::ink_a(0.06))
}

/// A cover of the user's games, else a [`slot`] while there is none.
pub fn cover(art: &Art, i: usize, w: f32, h: f32, radius: f32) -> Div {
    if !art.has_games() {
        return slot(w, h, radius);
    }
    let hue = (i as f32 * 0.137 + 0.55) % 1.;
    let d = div().relative().flex_none().w(px(w)).h(px(h)).rounded(px(radius)).bg(hsla(hue, 0.3, 0.28, 1.));
    match art.cover(i) {
        Some(src) => d.child(img(src).absolute().inset_0().size_full().rounded(px(radius)).object_fit(ObjectFit::Cover)),
        None => d,
    }
}

/// Where a cover will go: a dashed outline with a plus.
pub fn ghost(w: f32, h: f32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(px(w))
        .h(px(h))
        .rounded(px(10.))
        .border_2()
        .border_dashed()
        .border_color(theme::ink_a(0.14))
        .bg(theme::ink_a(0.03))
        .child(icon(Icon::Add, px(20.), theme::ink_a(0.22)))
}

fn lift() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: theme::shadow_a(0.5),
        offset: point(px(0.), px(10.)),
        blur_radius: px(24.),
        spread_radius: px(0.),
        inset: false,
    }]
}

/// Three covers side by side, the middle one raised (three empty ones
/// without games).
pub fn fan(art: &Art, h: f32) -> Div {
    let (w, ch) = (76., 112.);
    let top = (h - ch) / 2.;
    let slot = |n: usize| {
        let el = if art.has_games() { cover(art, [1, 0, 2][n], w, ch, 9.) } else { ghost(w, ch) };
        div().absolute().left(px(n as f32 * 82.)).top(px(top + if n == 1 { -8. } else { 6. })).shadow(lift()).rounded(px(9.)).child(el)
    };
    div()
        .absolute()
        .inset_0()
        .flex()
        .justify_center()
        .child(div().relative().w(px(76. + 2. * 82.)).h(px(h)).child(slot(0)).child(slot(2)).child(slot(1)))
}

/// The Collection as the current appearance draws it, `w` wide on a 600x380
/// board: the art (or a landscape) behind the Focus list and the selected
/// game, or, with `grid`, the covers across it and the game in a drawer.
pub fn collection_preview(art: &Art, w: f32, grid: bool) -> Div {
    let b = Board { k: w / 600. };
    let a = theme::appearance();
    let solid = a.interface_mode == InterfaceMode::Solid;
    let (r, g, bl) = theme::panel_rgb();
    let panel = if solid { theme::rgba_hsla(r, g, bl, 0.98) } else { theme::preview_glass() };
    let backdrop = if a.background == BackgroundSource::Solid {
        div().absolute().inset_0().bg(theme::rgba_hsla(r, g, bl, 1.))
    } else {
        div()
            .absolute()
            .inset_0()
            .bg(theme::page_bg())
            .child(stage(art).opacity(a.collection_opacity / 100.))
            // The Collection rolls the art's highlights off (fades it on the
            // light theme); a flat dim does it here.
            .child(div().absolute().inset_0().bg(theme::stage_ink_a(if theme::is_light() { 0.6 } else { 0.4 })))
            .child(div().absolute().inset_0().bg(linear_gradient(
                90.,
                linear_color_stop(theme::stage_ink_a(0.85), 0.),
                linear_color_stop(theme::stage_ink_a(0.2), 0.7),
            )))
    };
    let board = div()
        .relative()
        .w(px(w))
        .h(b.p(380.))
        .overflow_hidden()
        .rounded(px(16.))
        .border_1()
        .border_color(theme::ink_a(0.08))
        .child(backdrop);
    if grid {
        grid_layout(board, art, b, panel)
    } else {
        focus_layout(board, art, b, panel)
    }
}

/// The selected game's art, else a glow of the accent.
fn stage(art: &Art) -> Div {
    match art.stage.clone() {
        Some(src) => div().absolute().inset_0().child(img(src).size_full().object_fit(ObjectFit::Cover)),
        None if art.has_games() => div().absolute().inset_0(),
        None => div().absolute().inset_0().bg(linear_gradient(
            150.,
            linear_color_stop(theme::accent_alpha(0.32), 0.),
            linear_color_stop(theme::rgba_hsla(110, 90, 255, 0.12), 0.75),
        )),
    }
}

/// The game's title, or bars where it goes while there are no games.
fn title_text(art: &Art, b: Board, size: f32, w: f32) -> Div {
    match art.titles.first() {
        Some(t) => b.text(size, theme::text_primary(), t.clone()).font_family(theme::font_heading()).font_weight(theme::W_BOLD),
        None => div()
            .flex()
            .flex_col()
            .gap(b.p(size * 0.3))
            .child(div().w(b.p(w * 0.7)).h(b.p(size * 0.6)).rounded_full().bg(theme::ink_a(0.18)))
            .child(div().w(b.p(w * 0.4)).h(b.p(size * 0.3)).rounded_full().bg(theme::ink_a(0.1))),
    }
}

fn search_pill(b: Board, x: f32, y: f32, w: f32) -> Div {
    b.rect(x, y, w, 24.)
        .rounded_full()
        .bg(theme::ink_a(0.07))
        .flex()
        .items_center()
        .gap(b.p(6.))
        .px(b.p(9.))
        .child(icon(Icon::FaSearch, b.p(10.), theme::text_dim()))
        .child(b.text(10., theme::text_dim(), tr!("Search games")))
}

/// A cover of the list at `(x, y)`; the first one is the selected game.
fn list_cover(art: &Art, b: Board, i: usize, x: f32, y: f32) -> Div {
    let (cw, ch) = (52., 78.);
    let radius = theme::appearance().cover_corners.radius();
    // Few games: the grid repeats them rather than leaving holes.
    let n = if art.has_games() { i % art.titles.len() } else { i };
    let c = cover(art, n, cw * b.k, ch * b.k, radius * b.k);
    let cell = b.rect(x, y, cw, ch);
    if i == 0 {
        cell.rounded(b.p(radius + 2.)).border_2().border_color(theme::accent()).child(c)
    } else {
        cell.child(c)
    }
}

/// Play, the status and the facts of the selected game, `w` wide.
fn game_side(b: Board, panel: Hsla, w: f32) -> Div {
    let fact = |label: &'static str, value: SharedString| {
        div()
            .flex()
            .justify_between()
            .gap(b.p(8.))
            .child(b.text(10.5, theme::text_dim(), label))
            .child(b.text(10.5, theme::text_primary(), value).font_weight(theme::W_BOLD))
    };
    div()
        .w(b.p(w))
        .flex()
        .flex_col()
        .gap(b.p(10.))
        .child(
            div()
                .h(b.p(30.))
                .rounded_full()
                .bg(theme::accent())
                .flex()
                .items_center()
                .justify_center()
                .gap(b.p(5.))
                .child(icon(Icon::PlayArrow, b.p(14.), theme::on_accent()))
                .child(b.text(12., theme::on_accent(), tr!("Play")).font_weight(theme::W_BOLD)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(b.p(6.))
                .px(b.p(10.))
                .py(b.p(5.))
                .rounded_full()
                .bg(panel)
                .w(b.p(96.))
                .child(div().size(b.p(7.)).rounded_full().bg(theme::accent()))
                .child(b.text(11., theme::text_primary(), tr!("Playing"))),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(b.p(6.))
                .p(b.p(10.))
                .rounded(b.p(10.))
                .bg(panel)
                .border_1()
                .border_color(theme::ink_a(if theme::high_contrast() { 0.35 } else { 0.08 }))
                .child(fact(tr!("Rating"), "★★★★☆".into()))
                .child(fact(tr!("Save backup"), tr!("Yesterday").into()))
                .child(fact(tr!("On Linux"), "Platinum".into())),
        )
}

/// The Focus view: the list in a panel on the left, the game beside it.
fn focus_layout(board: Div, art: &Art, b: Board, panel: Hsla) -> Div {
    let radius = theme::appearance().cover_corners.radius();
    board
        .child(
            b.rect(10., 10., 196., 360.)
                .rounded(b.p(12.))
                .bg(panel)
                .child(search_pill(b, 10., 10., 176.))
                .children((0..9).map(|i| list_cover(art, b, i, 10. + (i % 3) as f32 * 60., 46. + (i / 3) as f32 * 86.))),
        )
        .child(b.at(228., 34.).w(b.p(350.)).child(title_text(art, b, 28., 350.)))
        .child(b.at(228., 76.).child(cover(art, 0, 112. * b.k, 168. * b.k, radius * b.k)))
        .child(b.at(354., 76.).child(game_side(b, panel, 200.)))
}

/// The Grid view: covers across the window, the game in a drawer on the right.
fn grid_layout(board: Div, art: &Art, b: Board, panel: Hsla) -> Div {
    let radius = theme::appearance().cover_corners.radius();
    let drawer = b
        .rect(392., 10., 198., 360.)
        .rounded(b.p(14.))
        .overflow_hidden()
        .bg(panel)
        .border_1()
        .border_color(theme::ink_a(0.1))
        .child(b.rect(0., 0., 198., 74.).rounded_t(b.p(14.)).overflow_hidden().child(stage(art)))
        .child(b.at(12., 46.).child(cover(art, 0, 46. * b.k, 69. * b.k, radius * b.k)))
        .child(b.at(64., 84.).w(b.p(124.)).child(title_text(art, b, 14., 124.)))
        .child(b.at(12., 126.).child(game_side(b, panel, 174.)));
    board
        .child(search_pill(b, 10., 10., 200.))
        .children((0..24).map(|i| list_cover(art, b, i, 10. + (i % 6) as f32 * 62., 46. + (i / 6) as f32 * 88.)))
        .child(drawer)
}

/// The panel the controller's Guide button opens over a game, on a 600x340
/// board `w` wide; `left`: the side of the screen it opens on.
pub fn overlay_preview(art: &Art, w: f32, left: bool) -> Div {
    // The overlay is dark whatever the theme.
    theme::drawn_as(theme::Scheme::Dark, || overlay_board(art, w, left))
}

fn overlay_board(art: &Art, w: f32, left: bool) -> Div {
    let b = Board { k: w / 600. };
    let game = match art.stage.clone() {
        Some(src) => div().absolute().inset_0().child(img(src).size_full().object_fit(ObjectFit::Cover)),
        None => landscape(),
    };
    let title: SharedString = art.titles.first().cloned().unwrap_or_else(|| tr!("Your game").into());
    let keys =
        |ks: &[&'static str]| {
            div().ml_auto().flex().gap(b.p(3.)).children(ks.iter().map(|k| {
                div().px(b.p(5.)).py(b.p(1.)).rounded(b.p(4.)).bg(theme::white_a(0.1)).child(b.text(9.5, theme::text_muted(), *k))
            }))
        };
    let row = |ic: Icon, label: &'static str, focused: bool| {
        div()
            .flex()
            .items_center()
            .gap(b.p(8.))
            .h(b.p(30.))
            .px(b.p(9.))
            .rounded(b.p(9.))
            .bg(if focused { theme::accent_alpha(0.16).into() } else { theme::white_a(0.04) })
            .when(focused, |d| d.border_1().border_color(theme::accent_alpha(0.5)))
            .child(icon(ic, b.p(14.), theme::text_muted()))
            .child(b.text(11.5, theme::text_primary(), label))
    };
    let tab = |ic: Icon, on: bool| {
        div()
            .flex_1()
            .h(b.p(26.))
            .rounded(b.p(7.))
            .flex()
            .items_center()
            .justify_center()
            .when(on, |d| d.bg(theme::white_a(0.1)))
            .child(icon(ic, b.p(13.), if on { theme::text_primary() } else { theme::text_dim() }))
    };
    let px_x = if left { 14. } else { 600. - 14. - 270. };
    let panel = b
        .rect(px_x, 14., 270., 312.)
        .rounded(b.p(16.))
        .bg(theme::rgba_hsla(18, 22, 30, 0.93))
        .border_1()
        .border_color(theme::white_a(0.1))
        .shadow(lift())
        .p(b.p(12.))
        .flex()
        .flex_col()
        .gap(b.p(8.))
        .child(
            div().flex().items_center().gap(b.p(8.)).child(cover(art, 0, 24. * b.k, 34. * b.k, 4. * b.k)).child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(b.text(12.5, theme::text_primary(), title).font_family(theme::font_heading()).font_weight(theme::W_BOLD))
                    .child(b.text(10.5, theme::text_dim(), tr!("Playing for 0:42 · 60 FPS"))),
            ),
        )
        .child(
            div()
                .flex()
                .gap(b.p(3.))
                .p(b.p(3.))
                .rounded(b.p(9.))
                .bg(theme::black_a(0.3))
                .child(tab(Icon::Home, true))
                .child(tab(Icon::PhotoCamera, false))
                .child(tab(Icon::Notes, false))
                .child(tab(Icon::VolumeUp, false))
                .child(tab(Icon::SportsEsports, false)),
        )
        .child(row(Icon::PhotoCamera, tr!("Take a screenshot"), true).child(keys(&["Guide", "RB"])))
        .child(row(Icon::Bookmark, tr!("Mark a moment"), false).child(keys(&["Guide", "Y"])))
        .child(
            row(Icon::VolumeUp, tr!("Volume"), false).child(
                div()
                    .ml_auto()
                    .w(b.p(70.))
                    .h(b.p(4.))
                    .rounded_full()
                    .bg(theme::white_a(0.15))
                    .child(div().h_full().w(relative(0.64)).rounded_full().bg(theme::accent())),
            ),
        )
        .child(row(Icon::Tune, tr!("FPS limit"), false).child(keys(&["60"])))
        .child(row(Icon::Home, tr!("Back to Heronite"), false));
    let pad_x = if left { 600. - 14. - 150. } else { 18. };
    div()
        .relative()
        .w(px(w))
        .h(b.p(340.))
        .overflow_hidden()
        .rounded(px(16.))
        .border_1()
        .border_color(theme::white_a(0.08))
        .child(game)
        .child(div().absolute().inset_0().bg(theme::black_a(0.4)))
        .child(b.rect(18., 16., 120., 6.).rounded_full().bg(theme::white_a(0.75)))
        .child(b.rect(18., 26., 72., 6.).rounded_full().bg(rgb(0x7ec7ff)))
        .child(panel)
        .child(
            b.at(pad_x, 262.).flex().items_center().gap(b.p(8.)).child(icon(Icon::SportsEsports, b.p(48.), theme::white_a(0.8))).child(
                div()
                    .px(b.p(8.))
                    .py(b.p(3.))
                    .rounded_full()
                    .bg(theme::black_a(0.55))
                    .border_1()
                    .border_color(theme::accent_alpha(0.6))
                    .child(b.text(11., theme::text_primary(), tr!("Guide opens it"))),
            ),
        )
}

/// A ring `size` wide filled clockwise from the top to `fraction` (0..=1).
pub fn ring(fraction: f32, size: f32, label: impl Into<SharedString>) -> Div {
    let fraction = fraction.clamp(0., 1.);
    let accent = theme::accent();
    let track = theme::ink_a(0.09);
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .flex()
        .items_center()
        .justify_center()
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let stroke = 5.;
                    let c = bounds.center();
                    let r = (f32::from(bounds.size.width).min(f32::from(bounds.size.height)) - stroke) / 2.;
                    let arc = |from: f32, to: f32, color: Hsla, window: &mut Window| {
                        let steps = ((to - from) * 96.).ceil().max(1.) as usize;
                        let mut path = PathBuilder::stroke(px(stroke));
                        for s in 0..=steps {
                            let t = from + (to - from) * s as f32 / steps as f32;
                            let a = t * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                            let p = c + point(px(r * a.cos()), px(r * a.sin()));
                            if s == 0 {
                                path.move_to(p);
                            } else {
                                path.line_to(p);
                            }
                        }
                        if let Ok(path) = path.build() {
                            window.paint_path(path, color);
                        }
                    };
                    arc(0., 1., track, window);
                    if fraction > 0. {
                        arc(0., fraction, accent.into(), window);
                    }
                },
            )
            .absolute()
            .inset_0(),
        )
        .child(div().font_family(theme::font_heading()).font_weight(theme::W_BOLD).text_size(px(14.)).child(label.into()))
}
