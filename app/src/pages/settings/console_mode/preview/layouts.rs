//! The content of a picture, by the first tab's layout. Positions are in
//! canvas units (640 x 360); the console's own screens are the model
//! (`collection/console/layouts/`), drawn small and simplified.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{
    FactStyle, GridPanel, Headings, HeroAlign, HeroTitle, Layout, Size3, SpotActions, TileShape, TileTitles, WheelItems,
};
use heronite_i18n::tr;

use super::{Pic, CANVAS_H, CANVAS_W};
use crate::console::theme_rows;
use crate::theme::W_BOLD;

/// The first tab's layout drawn over the background.
pub(super) fn content(p: &mut Pic) -> Div {
    let layout = p.t.tabs.first().map(|t| t.layout).unwrap_or_default();
    let layer = div().absolute().inset_0();
    match layout {
        Layout::Shelves => shelves(p, layer),
        Layout::Spotlight => spotlight(p, layer),
        Layout::Cinema => cinema(p, layer),
        Layout::Dashboard => dashboard(p, layer),
        Layout::Grid => grid(p, layer),
        Layout::Wheel => wheel(p, layer),
        Layout::Carousel => carousel(p, layer),
        Layout::Xbox => xbox(p, layer),
    }
}

impl Pic<'_, '_> {
    /// `child` placed at `x`, `y`.
    fn at(&self, x: f32, y: f32, child: impl IntoElement) -> Div {
        div().absolute().left(self.u(x)).top(self.u(y)).child(child)
    }

    /// The focused game's facts: a line of text bars or chips, as the theme says.
    fn facts(&self) -> Div {
        let mut row = div().flex().items_center().gap(self.u(5.));
        match self.t.hero.fact_style {
            FactStyle::Line => {
                for (n, w) in [34., 46., 28.].into_iter().enumerate() {
                    if n > 0 {
                        row = row.child(div().size(self.u(2.)).rounded_full().bg(self.muted));
                    }
                    row = row.child(self.line(w, 4., 0.6));
                }
            }
            FactStyle::Chips => {
                for w in [26., 22., 30.] {
                    row = row.child(
                        div()
                            .px(self.u(5.))
                            .py(self.u(3.))
                            .rounded(self.u(6.))
                            .border(self.u(0.8))
                            .border_color(Rgba { a: 0.35, ..self.text })
                            .child(self.line(w, 3., 0.7)),
                    );
                }
            }
        }
        row
    }

    /// The focused game's name (or its logo, drawn as a heavier name),
    /// facts and description, as the hero options say.
    fn hero(&self, w: f32, big: f32) -> Div {
        let h = &self.t.hero;
        let size = if h.title == HeroTitle::Logo { big * 1.3 } else { big };
        let mut col = div().flex().flex_col().gap(self.u(6.)).child(self.game_name(0, size, w));
        if h.facts {
            col = col.child(self.facts());
        }
        if h.overview {
            col =
                col.child(div().flex().flex_col().gap(self.u(3.)).child(self.line(w * 0.9, 3.5, 0.4)).child(self.line(w * 0.7, 3.5, 0.4)));
        }
        col
    }

    /// The title of shelf `n` of the theme (the ones switched on).
    fn shelf_heading(&self, n: usize) -> Div {
        let spec = self.t.shelves.iter().filter(|s| s.on).nth(n);
        let name = spec.map(|s| theme_rows::shelf_title(s, |k| k.to_string())).unwrap_or_default();
        let name = if self.t.interface.headings == Headings::Spaced { name.to_uppercase() } else { name };
        self.label(name, 8.5, self.text).font_weight(W_BOLD)
    }
}

fn shelves(p: &mut Pic, layer: Div) -> Div {
    let top = p.top() + 10.;
    let shelf_y = top
        + match p.t.hero.size {
            Size3::Small => 84.,
            Size3::Medium => 108.,
            Size3::Large => 132.,
        };
    let h = 74. * p.size();
    let hero = p.hero(250., 18.);
    let stats = p.t.hero.stats.then(|| {
        let mut col = div().flex().flex_col().items_end().gap(p.u(6.));
        for w in [30., 38., 26.] {
            col = col.child(p.line(w, 5., 0.7));
        }
        p.at(CANVAS_W - 70., top + 8., col)
    });
    let first = p.tile_row(0, h, 8., 9, true);
    let second = p.tile_row(4, h, 8., 9, false);
    let dim = if p.t.dim_others { 0.45 } else { 1. };
    layer
        .child(p.scrim_left(0.6))
        .child(p.scrim_bottom(0.5))
        .child(p.at(26., top, hero))
        .children(stats)
        .child(p.at(26., shelf_y, div().flex().flex_col().gap(p.u(6.)).child(p.shelf_heading(0)).child(first)))
        .child(p.at(26., shelf_y + h + 32., div().flex().flex_col().gap(p.u(6.)).opacity(dim).child(p.shelf_heading(1)).child(second)))
}

fn cinema(p: &mut Pic, layer: Div) -> Div {
    let h = 82. * p.size();
    let row_y = p.bottom() - h - if p.t.tiles.titles == TileTitles::Always { 22. } else { 10. };
    let centred = p.t.hero.align == HeroAlign::Center;
    let hero = p.hero(300., 22.);
    let hero = if centred {
        div().absolute().left_0().right_0().top(p.u(p.top() + 24.)).flex().justify_center().child(hero.items_center())
    } else {
        p.at(26., p.top() + 30., hero)
    };
    let row = p.tile_row(0, h, 10., 8, true);
    layer.child(p.scrim_left(0.55)).child(p.scrim_bottom(0.65)).child(hero).child(p.at(26., row_y, row))
}

fn spotlight(p: &mut Pic, layer: Div) -> Div {
    let top = p.top() + 4.;
    let big = 54. * p.s;
    let small = 38. * p.s;
    let shape = Some(if p.t.tiles.shape == TileShape::Portrait { TileShape::Square } else { p.t.tiles.shape });
    let mut icons = div().flex().items_center().gap(p.u(9.));
    icons = icons.child(p.tile(0, big, shape, true));
    icons = icons.child(div().w(p.u(96.)).child(p.game_name(0, 10., 96.)));
    for i in 1..8 {
        icons = icons.child(p.tile(i, small, shape, false));
    }
    let mut buttons = div().flex().items_center().gap(p.u(7.)).child(
        div()
            .w(p.u(104. * p.s))
            .h(p.u(24. * p.s))
            .flex()
            .items_center()
            .justify_center()
            .rounded(p.u(12.))
            .bg(p.text)
            .child(p.label(tr!("Play"), 9., p.bg).font_weight(W_BOLD)),
    );
    let more = if p.t.layouts.spotlight_actions == SpotActions::Full { 3 } else { 1 };
    for _ in 0..more {
        buttons = buttons.child(div().size(p.u(24. * p.s)).rounded_full().bg(Rgba { a: 0.18, ..p.text }));
    }
    let game = div().flex().flex_col().gap(p.u(10.)).child(p.game_name(0, 24., 260.)).child(buttons).child(p.line(70., 4., 0.5));
    let cards = p.t.layouts.spotlight_cards.then(|| {
        let mut row = div().flex().gap(p.u(10.));
        for _ in 0..3 {
            row = row.child(
                div()
                    .w(p.u(160.))
                    .h(p.u(70.))
                    .p(p.u(9.))
                    .flex()
                    .flex_col()
                    .gap(p.u(5.))
                    .rounded(p.u(10.))
                    .bg(p.panel)
                    .child(p.line(50., 4., 0.8))
                    .child(p.line(80., 4., 0.45)),
            );
        }
        p.at(26., CANVAS_H - 44., row)
    });
    layer.child(p.scrim_left(0.55)).child(p.scrim_bottom(0.55)).child(p.at(26., top, icons)).child(p.at(26., 176., game)).children(cards)
}

fn carousel(p: &mut Pic, layer: Div) -> Div {
    let h = (150. * p.size()).min(200.);
    let shape = p.t.tiles.shape;
    let w = h * p.aspect(shape);
    let top = p.top() + 18.;
    let gap = 16.;
    let reflect = p.t.layouts.carousel_reflect;
    let mut out = layer.child(p.scrim_bottom(0.45));
    // From the far ones in, so the focused cover paints last.
    for d in [3_i32, 2, 1, 0] {
        for side in [-1_i32, 1] {
            if d == 0 && side == 1 {
                continue;
            }
            let f = [1., 0.8, 0.66, 0.56][d as usize];
            let (ch, cw) = (h * f, w * f);
            let mut x = CANVAS_W / 2. - cw / 2.;
            if d > 0 {
                let mut off = w / 2. + gap;
                for j in 1..d {
                    off += w * [1., 0.8, 0.66, 0.56][j as usize] + gap;
                }
                off += cw / 2.;
                x = CANVAS_W / 2. + side as f32 * off - cw / 2.;
            }
            // The far ones rise a little.
            let y = top + (h - ch) * 0.35;
            let i = (side * d).rem_euclid(12) as usize;
            let mut cover = p.tile(i, ch, None, d == 0);
            if d > 0 {
                cover = cover.child(div().absolute().inset_0().rounded(p.u(p.radius(ch))).bg(p.shade(0.18 * d as f32)));
            }
            out = out.child(p.at(x, y, cover));
            if reflect {
                let r = p.radius(ch);
                out = out.child(p.at(
                    x,
                    y + ch + 4.,
                    div().w(p.u(cw)).h(p.u(ch * 0.32)).rounded(p.u(r)).bg(linear_gradient(
                        180.,
                        linear_color_stop(Rgba { a: if d == 0 { 0.16 } else { 0.08 }, ..p.text }, 0.),
                        linear_color_stop(Rgba { a: 0., ..p.text }, 1.),
                    )),
                ));
            }
        }
    }
    let under = div()
        .absolute()
        .left_0()
        .right_0()
        .top(p.u(top + h + if reflect { h * 0.32 + 10. } else { 16. }))
        .flex()
        .flex_col()
        .items_center()
        .gap(p.u(6.))
        .child(p.game_name(0, 13., 260.))
        .child(
            div()
                .relative()
                .w(p.u(110.))
                .h(p.u(1.5))
                .bg(Rgba { a: 0.25, ..p.text })
                .child(div().absolute().left(p.u(30.)).top(p.u(-1.)).w(p.u(14.)).h(p.u(3.5)).rounded(p.u(2.)).bg(p.text)),
        );
    out.child(under)
}

fn grid(p: &mut Pic, layer: Div) -> Div {
    let panel = p.t.layouts.grid_panel == GridPanel::Right;
    let mut top = p.top() + 8.;
    let h = 72. * p.size();
    let titled = !panel;
    let mut out = layer.child(p.cover_all().bg(p.shade(0.45)));
    if titled {
        out = out.child(p.at(26., top, div().flex().items_center().gap(p.u(10.)).child(p.game_name(0, 13., 200.)).child(p.facts())));
        top += 26.;
    }
    let right = if panel { 200. } else { 26. };
    let w = h * p.aspect(p.t.tiles.shape);
    let cols = (((CANVAS_W - 26. - right) + 10.) / (w + 10.)).floor().max(1.) as usize;
    let mut rows = div().flex().flex_col().gap(p.u(12.));
    for r in 0..3 {
        let mut row = div().flex().gap(p.u(10.));
        for c in 0..cols {
            let i = r * cols + c;
            row = row.child(p.tile(i, h, None, i == 0));
        }
        rows = rows.child(row);
    }
    out = out.child(p.at(26., top, rows));
    if panel {
        let art = p.picture_box(0, true, 160., 90., 8.);
        let side = div()
            .w(p.u(174.))
            .h(p.u(CANVAS_H - top - 30.))
            .p(p.u(7.))
            .flex()
            .flex_col()
            .gap(p.u(7.))
            .rounded(p.u(12.))
            .bg(p.panel)
            .child(art)
            .child(p.game_name(0, 11., 160.))
            .child(p.line(120., 4., 0.5))
            .child(p.line(90., 4., 0.5));
        out = out.child(p.at(CANVAS_W - 190., top, side));
    }
    out
}

fn wheel(p: &mut Pic, layer: Div) -> Div {
    let logos = p.t.layouts.wheel_items == WheelItems::Logo;
    let mut col = div().flex().flex_col().gap(p.u(9.));
    for j in 0..7_usize {
        let focused = j == 3;
        let i = (j + 9) % 12;
        let size = if focused { 15. } else { 10. };
        let name = match p.title(i) {
            Some(t) => {
                let base = p.label(t, size, if focused { p.text } else { p.muted }).max_w(p.u(210.));
                if focused || logos {
                    base.font_family(p.head_font).font_weight(W_BOLD)
                } else {
                    base
                }
            }
            None => p.line(if focused { 150. } else { 110. }, size * 0.6, if focused { 0.9 } else { 0.45 }),
        };
        let fade = 1. - (j as f32 - 3.).abs() * 0.22;
        col = col.child(div().opacity(fade).child(name));
    }
    let cover = p.tile(0, 150. * p.s, Some(TileShape::Portrait), false);
    let facts = div().flex().flex_col().gap(p.u(6.)).child(cover).child(p.facts());
    layer.child(p.scrim_left(0.75)).child(p.at(30., p.top() + 22., col)).child(p.at(CANVAS_W - 200., p.top() + 14., facts))
}

fn dashboard(p: &mut Pic, layer: Div) -> Div {
    let top = p.top() + 6.;
    let l = &p.t.layouts.dashboard;
    let (week, downloads, random, journal, shortcuts) = (l.week, l.downloads, l.random, l.journal, l.shortcuts);
    let big = p
        .picture_box(0, true, 330., 168., 10.)
        .child(div().absolute().left(p.u(12.)).bottom(p.u(10.)).child(p.game_name(0, 15., 260.)))
        .child(
            div()
                .absolute()
                .top(p.u(-3.))
                .left(p.u(-3.))
                .w(p.u(336.))
                .h(p.u(174.))
                .rounded(p.u(13.))
                .border(p.u(1.6).max(px(1.)))
                .border_color(p.accent),
        );
    let widget = |p: &Pic, w: f32, h: f32| {
        div().w(p.u(w)).h(p.u(h)).p(p.u(9.)).flex().flex_col().gap(p.u(6.)).rounded(p.u(10.)).bg(p.panel).child(p.line(w * 0.4, 4., 0.8))
    };
    let mut side = div().flex().flex_col().gap(p.u(10.));
    let mut covers = div().flex().gap(p.u(10.));
    for i in 1..3 {
        covers = covers.child(p.picture_box(i, true, 120., 79., 8.));
    }
    side = side.child(covers);
    if week {
        let mut bars = div().flex().items_end().gap(p.u(5.));
        for (n, hgt) in [10., 22., 14., 30., 6., 18., 26.].into_iter().enumerate() {
            bars = bars.child(div().w(p.u(9.)).h(p.u(hgt)).rounded(p.u(2.)).bg(if n == 3 { p.accent } else { Rgba { a: 0.35, ..p.text } }));
        }
        side = side.child(widget(p, 250., 79.).child(bars));
    }
    let mut bottom = div().flex().gap(p.u(10.));
    for (on, w) in [(downloads, 140.), (random, 140.), (journal, 140.), (shortcuts, 160.)] {
        if on {
            bottom = bottom.child(widget(p, w, 74.).child(p.line(w * 0.6, 4., 0.45)).child(p.line(w * 0.5, 4., 0.45)));
        }
    }
    layer.child(p.cover_all().bg(p.shade(0.4))).child(p.at(26., top, big)).child(p.at(370., top, side)).child(p.at(26., top + 184., bottom))
}

fn xbox(p: &mut Pic, layer: Div) -> Div {
    let top = p.top() + 30.;
    let h = 112. * p.s;
    let mut row = div().flex().items_start().gap(p.u(9.));
    row = row.child(p.tile(0, h, Some(TileShape::Landscape), true));
    for i in 1..6 {
        row = row.child(p.tile(i, h, Some(TileShape::Square), false));
    }
    let groups = p.t.layouts.xbox_groups.then(|| {
        let mut col = div().flex().flex_col().gap(p.u(9.));
        for r in 0..2 {
            let mut line = div().flex().gap(p.u(9.));
            for c in 0..4 {
                line = line.child(
                    div()
                        .w(p.u(126.))
                        .h(p.u(32.))
                        .px(p.u(9.))
                        .flex()
                        .items_center()
                        .gap(p.u(6.))
                        .rounded(p.u(8.))
                        .bg(p.panel)
                        .child(div().size(p.u(12.)).rounded(p.u(3.)).bg(Rgba { a: 0.4, ..p.text }))
                        .child(p.line(40. + ((r * 4 + c) % 3) as f32 * 14., 4., 0.6)),
                );
            }
            col = col.child(line);
        }
        p.at(26., top + h + 30., col)
    });
    layer.child(p.scrim_bottom(0.65)).child(p.at(26., top - 22., p.game_name(0, 12., 260.))).child(p.at(26., top, row)).children(groups)
}
