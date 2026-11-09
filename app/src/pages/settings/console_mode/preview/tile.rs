//! The covers of a picture: the shape, size, corners and focus of the theme's
//! tiles, with what each layout of the console draws on them.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{Badges, FocusStyle, TileShape, TileTitles};
use heronite_i18n::tr;

use super::{round_sides, Pic};
use crate::icons::Icon;
use crate::theme;

/// Height of the store strip above the art and of the name bar under it,
/// canvas units (the console's 22 and 28 design px, half).
const HEADER_H: f32 = 10.;
const BAR_H: f32 = 12.;
/// The favourite heart's colour.
const HEART: u32 = 0xf5cd5b;

/// What a layout's covers draw besides the art: the shared tile of the
/// console (Shelves, Cinema, Grid, Xbox) draws all of it, the Carousel and
/// Spotlight have their own covers with fewer extras.
#[derive(Clone, Copy)]
pub(super) struct Dress {
    /// The name under the cover or in a bar (`tiles.titles`).
    pub titles: bool,
    /// The store strip above the cover (`tiles.header`).
    pub header: bool,
    /// "Playing" (`tiles.badges`).
    pub badges: bool,
    /// The favourite heart, when the badges are all of them.
    pub hearts: bool,
    /// Uninstalled games in greys (`tiles.grey_uninstalled`).
    pub grey: bool,
}

impl Dress {
    pub const SHARED: Dress = Dress { titles: true, header: true, badges: true, hearts: true, grey: true };
    /// Carousel: greys only.
    pub const GREY: Dress = Dress { titles: false, header: false, badges: false, hearts: false, grey: true };
    /// Spotlight: greys and "Playing".
    pub const SPOT: Dress = Dress { titles: false, header: false, badges: true, hearts: false, grey: true };
    pub const PLAIN: Dress = Dress { titles: false, header: false, badges: false, hearts: false, grey: false };
}

/// Store names the strips show, by game.
const STORES: [&str; 4] = ["STEAM", "EPIC GAMES", "GOG", "STEAM"];

impl Pic<'_, '_> {
    /// A cover of game `i`, `h` units tall in the theme's shape (or `shape`),
    /// with its focus and, as the theme says, its name.
    pub fn tile(&mut self, i: usize, h: f32, shape: Option<TileShape>, focused: bool) -> Div {
        self.tile_as(Dress::SHARED, i, h, shape, focused)
    }

    /// [`Self::tile`] with the extras of `dress`.
    pub(super) fn tile_as(&mut self, dress: Dress, i: usize, h: f32, shape: Option<TileShape>, focused: bool) -> Div {
        let t = self.t;
        let shape = shape.unwrap_or(t.tiles.shape);
        let w = h * self.aspect(shape);
        let r = self.radius(h);
        // The strip and the bar are part of the tile: the art takes what is left.
        let header = dress.header && t.tiles.header;
        let bar = dress.titles && t.tiles.titles == TileTitles::Bar;
        let (head_h, bar_h) = (if header { HEADER_H } else { 0. }, if bar { BAR_H } else { 0. });
        // A couple of the others stand for games that are not installed.
        let grey = dress.grey && t.tiles.grey_uninstalled && !focused && i % 4 == 2;
        let art = self.picture_part(i, shape == TileShape::Landscape, w, h - head_h - bar_h, r, (!header, !bar), grey);
        let strip = |text: Div, top: bool| {
            let height = if top { head_h } else { bar_h };
            round_sides(
                div().flex_none().w(self.u(w)).h(self.u(height)).px(self.u(3.)).flex().items_center().justify_center().bg(self.sidebar),
                top,
                !top,
                self.u(r),
            )
            .child(text.max_w(self.u(w - 6.)))
        };
        let mut cover = div().flex_none().relative().flex().flex_col().w(self.u(w)).h(self.u(h));
        if header {
            cover = cover.child(strip(self.label(STORES[i % STORES.len()], 5., self.text).font_weight(theme::W_BOLD), true));
        }
        cover = cover.child(art);
        if bar {
            let name = match self.title(i) {
                Some(n) => self.label(n, 6., self.text).font_weight(theme::W_BOLD),
                None => self.line(w * 0.6, 3.5, 0.7),
            };
            cover = cover.child(strip(name, false));
        }
        if dress.badges && t.tiles.badges != Badges::Off {
            if i % 5 == 1 {
                cover = cover.child(
                    div()
                        .absolute()
                        .top(self.u(head_h + 3.))
                        .left(self.u(3.))
                        .px(self.u(3.5))
                        .py(self.u(1.2))
                        .rounded_full()
                        .bg(theme::console_btn_a())
                        .child(self.label(tr!("Playing"), 5., self.on_fill(theme::console_btn_a())).font_weight(theme::W_BOLD)),
                );
            }
            if dress.hearts && t.tiles.badges == Badges::All && i % 4 == 3 {
                cover = cover.child(
                    div()
                        .absolute()
                        .top(self.u(head_h + 3.))
                        .right(self.u(3.))
                        .size(self.u(9.))
                        .rounded_full()
                        .bg(theme::black_a(0.7))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(crate::ui::icon(Icon::Favorite, self.u(5.5), rgb(HEART))),
                );
            }
        }
        let mut frame = div().flex_none().relative().child(cover);
        if focused {
            let a = self.accent;
            frame = match t.tiles.focus {
                FocusStyle::Ring => frame.child(
                    div()
                        .absolute()
                        .top(self.u(-3.5))
                        .left(self.u(-3.5))
                        .w(self.u(w + 7.))
                        .h(self.u(h + 7.))
                        .rounded(self.u(r + 3.5))
                        .border(self.u(1.6).max(px(1.)))
                        .border_color(a),
                ),
                FocusStyle::Glow => frame.rounded(self.u(r)).shadow(vec![BoxShadow {
                    color: Hsla { a: 0.7, ..Hsla::from(a) },
                    offset: point(px(0.), px(0.)),
                    blur_radius: self.u(16.),
                    spread_radius: self.u(1.),
                    inset: false,
                }]),
                FocusStyle::Lift => frame.mt(self.u(-7.)).mb(self.u(7.)).rounded(self.u(r)).shadow(vec![BoxShadow {
                    color: theme::black_a(0.6),
                    offset: point(px(0.), self.u(6.)),
                    blur_radius: self.u(12.),
                    spread_radius: px(0.),
                    inset: false,
                }]),
                FocusStyle::Underline => frame.child(
                    div().absolute().left(self.u(w * 0.2)).bottom(self.u(-6.)).w(self.u(w * 0.6)).h(self.u(2.5)).rounded(self.u(2.)).bg(a),
                ),
            };
        }
        let named = dress.titles && matches!((t.tiles.titles, focused), (TileTitles::Always, _) | (TileTitles::Focused, true));
        let mut col = div().flex_none().flex().flex_col().gap(self.u(5.)).w(self.u(w)).child(frame);
        if named {
            col = col.child(match self.title(i) {
                Some(n) => self.label(n, 8., if focused { self.text } else { self.muted }),
                None => self.line(w * 0.7, 4., 0.6),
            });
        }
        col
    }

    /// A row of covers from `first`, the first one focused, starting `x`
    /// units in; it runs off the right edge like the console's.
    pub fn tile_row(&mut self, first: usize, h: f32, gap: f32, count: usize, focus: bool) -> Div {
        let mut row = div().flex().items_start().gap(self.u(gap));
        for j in 0..count {
            row = row.child(self.tile(first + j, h, None, focus && j == 0));
        }
        row
    }
}
