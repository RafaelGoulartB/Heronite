//! The content of a picture, by the layout of its tab. Positions are in
//! canvas units (640 x 360); the console's own screens are the model
//! (`collection/console/layouts/`), drawn small and simplified.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{
    FactStyle, GridPanel, Headings, HeroTitle, Layout, Size3, SpotActions, TileShape, TileSize, TileTitles, WheelItems,
};
use heronite_i18n::tr;

use super::tile::Dress;
use super::{Pic, CANVAS_H};
use crate::console::kit::spaced_caps;
use crate::console::theme_rows;
use crate::icons::Icon;
use crate::theme::{W_BOLD, W_REGULAR};

/// The layout of the tab shown, drawn over the background.
pub(super) fn content(p: &mut Pic) -> Div {
    let layout = p.t.tabs.get(p.tab).map(|t| t.layout).unwrap_or_default();
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
    pub(super) fn at(&self, x: f32, y: f32, child: impl IntoElement) -> Div {
        div().absolute().left(self.u(x)).top(self.u(y)).child(child)
    }

    /// The focused game's facts: a line of text bars or chips, as the theme says.
    pub(super) fn facts(&self) -> Div {
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
    /// facts and, where the layout shows it, the description, as the hero
    /// options say.
    fn hero(&self, w: f32, big: f32, overview: bool) -> Div {
        let h = &self.t.hero;
        let size = if h.title == HeroTitle::Logo { big * 1.3 } else { big };
        let mut col = div().flex().flex_col().gap(self.u(6.)).child(self.game_name(0, size, w));
        if h.facts {
            col = col.child(self.facts());
        }
        if h.overview && overview {
            col =
                col.child(div().flex().flex_col().gap(self.u(3.)).child(self.line(w * 0.9, 3.5, 0.4)).child(self.line(w * 0.7, 3.5, 0.4)));
        }
        col
    }

    /// The title of shelf `n` of the theme (the ones switched on).
    fn shelf_heading(&self, n: usize) -> Div {
        let spec = self.t.shelves.iter().filter(|s| s.on).nth(n);
        let name = spec.map(|s| theme_rows::shelf_title(s, |k| k.to_string())).unwrap_or_default();
        // Spaced capitals are lighter and a little smaller, as in the console.
        if self.t.interface.headings == Headings::Spaced {
            self.label(spaced_caps(&name), 8.5 * 0.95, self.text).font_weight(W_REGULAR)
        } else {
            self.label(name, 8.5, self.text).font_weight(W_BOLD)
        }
    }

    /// The stats panel at the right edge: time played, last played, size,
    /// score and genre as an icon and a line each.
    fn stats(&self) -> Div {
        let mut col = div()
            .flex()
            .flex_col()
            .gap(self.u(6.))
            .px(self.u(8.))
            .py(self.u(7.))
            .rounded_l(self.u(7.))
            .bg(Rgba { a: 0.72, ..self.sidebar });
        for (ic, w) in [(Icon::AccessTime, 24.), (Icon::History, 30.), (Icon::Star, 12.), (Icon::Label, 26.)] {
            col = col.child(
                div().flex().items_center().gap(self.u(5.)).child(crate::ui::icon(ic, self.u(7.), self.muted)).child(self.line(w, 4., 0.7)),
            );
        }
        col
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
    // The names under the covers take room of their own.
    let names = if matches!(p.t.tiles.titles, TileTitles::Always | TileTitles::Focused) { 11. } else { 0. };
    let hero = p.hero(250., 18., true);
    let stats = p.t.hero.stats.then(|| div().absolute().right_0().top(p.u(top + 8.)).child(p.stats()));
    let first = p.tile_row(0, h, 8., 9, true);
    let second = p.tile_row(4, h, 8., 9, false);
    let dim = if p.t.dim_others { 0.45 } else { 1. };
    layer
        .child(p.scrim_left(0.6))
        .child(p.scrim_bottom(0.5))
        .child(p.at(26., top, hero))
        .children(stats)
        .child(p.at(26., shelf_y, div().flex().flex_col().gap(p.u(6.)).child(p.shelf_heading(0)).child(first)))
        .child(p.at(
            26.,
            shelf_y + h + 32. + names,
            div().flex().flex_col().gap(p.u(6.)).opacity(dim).child(p.shelf_heading(1)).child(second),
        ))
}

/// Cinema (the console's `layouts/cinema.rs`): the game's name large at the
/// left with its facts and genres under it, its achievements on the right,
/// one row of large covers along the bottom.
fn cinema(p: &mut Pic, layer: Div) -> Div {
    let h = 104. * p.size().min(1.15);
    let names = if p.t.tiles.titles == TileTitles::Always { 12. } else { 0. };
    let row_y = p.bottom() - h - names - 4.;
    let gap = p.u(3.);
    // The facts: small labels over values, between thin rules.
    let rule = || div().flex_none().w(px(1.)).h(p.u(9.)).bg(Rgba { a: 0.25, ..p.text });
    let mut facts = div().flex().items_center().gap(p.u(6.));
    for (i, w) in [14., 16., 18., 12., 14.].into_iter().enumerate() {
        if i > 0 {
            facts = facts.child(rule());
        }
        facts = facts.child(div().flex().flex_col().items_center().gap(gap).child(p.line(w * 0.7, 2., 0.4)).child(p.line(w, 3.5, 0.9)));
    }
    let genres = div().flex().gap(p.u(5.)).child(p.line(12., 2.5, 0.45)).child(p.line(14., 2.5, 0.45)).child(p.line(10., 2.5, 0.45));
    let block = div()
        .absolute()
        .left(p.u(26.))
        .max_w(p.u(250.))
        .bottom(p.u(CANVAS_H - row_y + 16.))
        .flex()
        .flex_col()
        .items_start()
        .gap(p.u(7.))
        .child(p.game_name(0, 22., 250.))
        .child(facts)
        .child(genres);
    // The achievements, level with the facts.
    let ach = div()
        .absolute()
        .right(p.u(30.))
        .bottom(p.u(CANVAS_H - row_y + 30.))
        .flex()
        .items_center()
        .gap(p.u(5.))
        .child(div().size(p.u(11.)).rounded_full().bg(Rgba { a: 0.25, ..p.accent }))
        .child(div().flex().flex_col().gap(gap).child(p.line(26., 2., 0.45)).child(p.line(18., 3.5, 0.9)).child(
            div().w(p.u(56.)).h(p.u(1.5)).rounded_full().bg(Rgba { a: 0.2, ..p.text }).child(div().w(p.u(14.)).h_full().bg(p.accent)),
        ));
    let row = p.tile_row(0, h, 8., 9, true);
    layer.child(p.scrim_left(0.6)).child(p.scrim_bottom(0.75)).child(block).child(ach).child(p.at(26., row_y, row))
}

/// Spotlight (the console's `layouts/spotlight/`): a row of square icons
/// under the tab pills, the focused one larger and ringed with its name
/// beside it; under it the logo (or the name), the pill buttons, the facts,
/// the genre chips and a strip of screenshots fading into the dark.
fn spotlight(p: &mut Pic, layer: Div) -> Div {
    const SIDE: f32 = 21.;
    let top = p.top() - 10.;
    // The icons keep the cover's shape: a portrait one is taller than a
    // square one, a wide one shorter; the size choice scales them.
    let (shape, k) = match p.t.tiles.shape {
        TileShape::Portrait => (TileShape::Portrait, 1.15),
        TileShape::Landscape => (TileShape::Landscape, 0.8),
        TileShape::Square => (TileShape::Square, 1.),
    };
    let size = match p.t.tiles.size {
        TileSize::Small => 0.85,
        TileSize::Medium => 1.,
        TileSize::Large => 1.15,
        TileSize::Huge => 1.3,
    };
    let k = k * size * p.s;
    let (big, small) = ((69. * k).min(104.), (54. * k).min(82.));
    let shape = Some(shape);
    let mut others = div().flex().items_start().gap(p.u(7.5));
    for i in 1..12 {
        others = others.child(p.tile_as(Dress::SPOT, i, small, shape, false));
    }
    let name = p.title(0).map(|t| p.label(t, 8., p.text).max_w(p.u(150.)));
    let icons = div()
        .flex()
        .items_start()
        .gap(p.u(7.5))
        .child(p.tile_as(Dress::SPOT, 0, big, shape, true))
        .child(div().flex().flex_col().gap(p.u(4.)).child(others).children(name));
    let below = top + big;

    // The logo (a heavier name: the picture has no logos) or the name, its
    // foot where the console puts it.
    let logo = p.t.hero.title == HeroTitle::Logo;
    let name = p.game_name(0, if logo { 25. } else { 22. }, 330.).when(logo, |d| d.font_weight(W_BOLD));
    let title = p.at(SIDE, below + 34., div().h(p.u(40.)).flex().items_end().child(name));

    // Play, Options (Status with every action) and the heart, as pills.
    let pill = |p: &Pic, w: Option<f32>, ic: Icon, text: Option<SharedString>| {
        let h = 18. * p.s;
        let fg = p.text;
        div()
            .flex_none()
            .h(p.u(h))
            .when_some(w, |d, w| d.w(p.u(w * p.s)))
            .when(w.is_none(), |d| d.w(p.u(h)))
            .flex()
            .items_center()
            .justify_center()
            .gap(p.u(3.))
            .rounded_full()
            .bg(p.shade(0.42))
            .border(p.u(0.6))
            .border_color(Rgba { a: 0.16, ..p.text })
            .child(crate::ui::icon(ic, p.u(7. * p.s), fg))
            .children(text.map(|t| p.label(t, 7., fg)))
    };
    let mut buttons = div()
        .flex()
        .items_center()
        .gap(p.u(5.5))
        .child(pill(p, Some(86.), Icon::PlayArrow, Some(tr!("Play").into())))
        .child(pill(p, Some(60.), Icon::MoreHoriz, Some(tr!("Options").into())));
    if p.t.layouts.spotlight_actions == SpotActions::Full {
        buttons = buttons.child(pill(p, Some(58.), Icon::Flag, Some(tr!("Status").into())));
    }
    buttons = buttons.child(pill(p, None, Icon::FavoriteBorder, None));

    // The facts: a small caption over each value, thin rules between.
    let mut facts = div().flex().items_center().gap(p.u(6.));
    for (n, (cap, w)) in
        [(tr!("Playtime"), 24.), (tr!("Last played"), 30.), (tr!("Platform"), 26.), (tr!("Released"), 16.)].into_iter().enumerate()
    {
        if n > 0 {
            facts = facts.child(div().w(p.u(0.6)).h(p.u(14.)).bg(Rgba { a: 0.2, ..p.text }));
        }
        facts =
            facts.child(div().flex().flex_col().gap(p.u(2.5)).child(p.label(cap.to_uppercase(), 4.2, p.muted)).child(p.line(w, 4.5, 0.85)));
    }
    let mut chips = div().flex().gap(p.u(3.));
    for w in [16., 26.] {
        chips = chips.child(div().px(p.u(4.5)).py(p.u(2.5)).rounded_full().bg(Rgba { a: 0.14, ..p.text }).child(p.line(w, 3., 0.7)));
    }
    let rest = div().flex().flex_col().child(buttons).child(div().mt(p.u(9.)).child(facts)).child(div().mt(p.u(6.)).child(chips));
    let strip_y = below + 149.;
    let mut out = layer.child(p.scrim_left(0.55)).child(p.at(SIDE, top, icons)).child(title).child(p.at(SIDE, below + 80., rest));

    // Under the head: the strip of screenshots (the store's or the user's),
    // else the cards, else a glimpse of the first section of the page.
    let sections = &p.t.layouts.spotlight_sections;
    let page = sections.about || sections.achievements || sections.details || sections.related;
    if sections.media || sections.screenshots {
        let mut strip = div().flex().gap(p.u(7.5));
        for i in 0..5 {
            strip = strip.child(p.picture_box(i + 3, true, 135., 76., 5.));
        }
        out = out.child(p.at(SIDE, strip_y, strip));
    } else if p.t.layouts.spotlight_cards {
        let mut row = div().flex().gap(p.u(8.));
        for _ in 0..3 {
            row = row.child(
                div()
                    .w(p.u(150.))
                    .h(p.u(62.))
                    .p(p.u(8.))
                    .flex()
                    .flex_col()
                    .gap(p.u(5.))
                    .rounded(p.u(8.))
                    .bg(p.panel)
                    .child(p.line(40., 4., 0.8))
                    .child(p.line(70., 4., 0.45)),
            );
        }
        out = out.child(p.at(SIDE, strip_y, row));
    } else if page {
        let peek =
            div().flex().flex_col().gap(p.u(6.)).child(p.line(46., 5., 0.8)).child(p.line(300., 3.5, 0.4)).child(p.line(240., 3.5, 0.4));
        out = out.child(p.at(SIDE, strip_y + 6., peek));
    }
    // The bottom fades into the dark, under the hints.
    let fade = div().absolute().left_0().right_0().bottom_0().h(p.u(CANVAS_H - strip_y - 8.)).bg(linear_gradient(
        180.,
        linear_color_stop(Rgba { a: 0., ..p.bg }, 0.),
        linear_color_stop(Rgba { a: 0.97, ..p.bg }, 1.),
    ));
    out.child(fade)
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
            let mut x = p.width() / 2. - cw / 2.;
            if d > 0 {
                let mut off = w / 2. + gap;
                for j in 1..d {
                    off += w * [1., 0.8, 0.66, 0.56][j as usize] + gap;
                }
                off += cw / 2.;
                x = p.width() / 2. + side as f32 * off - cw / 2.;
            }
            // The far ones rise a little.
            let y = top + (h - ch) * 0.35;
            let i = (side * d).rem_euclid(12) as usize;
            let mut cover = p.tile_as(Dress::GREY, i, ch, None, d == 0);
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
    let cols = (((p.width() - 26. - right) + 10.) / (w + 10.)).floor().max(1.) as usize;
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
        out = out.child(p.at(p.width() - 190., top, side));
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
    let cover = p.tile_as(Dress::PLAIN, 0, 150. * p.s, Some(TileShape::Portrait), false);
    let facts = div().flex().flex_col().gap(p.u(6.)).child(cover).child(p.facts());
    layer.child(p.scrim_left(0.75)).child(p.at(30., p.top() + 22., col)).child(p.at(p.width() - 200., p.top() + 14., facts))
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
    // The squares follow the size choice; the shelves under them keep their room.
    let room = p.bottom() - top - if p.t.layouts.xbox_groups { 30. + 73. } else { 8. };
    let h = (112. * p.size()).min(room.max(60.));
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
