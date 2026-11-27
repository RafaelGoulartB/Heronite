//! The row of icons along the top of Spotlight, as on a console's home: the
//! focused icon larger, with a ring a little way out and its name under it;
//! the icons before it stay in sight (up to [`LEAD`] of them) and the row
//! slides once the focus passes them. Every icon's place and size, and its
//! name's, come from one number, the row's position (the focused index,
//! gliding on a spring), so a move and a held direction change smoothly.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{Badges, TileShape, TileSize};
use heronite_i18n::tr;

use super::super::super::super::CollectionView;
use super::{Look, SIDE};
use crate::console::kit::{self, u};
use crate::console::look;
use crate::console::pointer::{self, Clickable};
use crate::console::Pad;
use crate::icons::Icon;
use crate::theme;

/// Side of an icon and of the focused one (square, medium covers), design px.
const ICON: f32 = 120.;
const ICON_ON: f32 = 148.;
/// Between two icons, design px.
const GAP: f32 = 16.;
/// Room above the icons (for the focus ring and its glow), design px.
const PAD: f32 = 12.;
/// The name beside the focused icon: its size, the space before it and its widest, design px.
const NAME: f32 = 17.;
const NAME_GAP: f32 = 14.;
const NAME_MAX: f32 = 560.;
/// Icons kept in sight before the focused one.
const LEAD: f32 = 2.;
/// The focus ring: how far out from the icon and how thick, design px.
const RING_GAP: f32 = 5.;
const RING: f32 = 2.5;

/// Width and height of an icon and of the focused one by the theme's cover
/// size and shape (a portrait cover keeps the app's cover proportions, so
/// nothing of it is cut).
pub(super) fn icon_sizes(cover_aspect: f32) -> ((f32, f32), (f32, f32)) {
    let t = look::theme();
    let k = match t.tiles.size {
        TileSize::Small => 0.85,
        TileSize::Medium => 1.,
        TileSize::Large => 1.2,
        TileSize::Huge => 1.4,
    };
    let dims = |side: f32| match t.tiles.shape {
        TileShape::Square => (side, side),
        TileShape::Portrait => (side * 0.8, side * 0.8 * cover_aspect),
        TileShape::Landscape => (side * 1.6, side * 1.6 * 9. / 16.),
    };
    (dims(ICON * k), dims(ICON_ON * k))
}

/// Height of the row: the focused icon (its name beside it, under the
/// smaller ones) and the room around it, design px.
pub(super) fn row_height(on: (f32, f32)) -> f32 {
    PAD + on.1 + PAD
}

/// Where an icon sits and how large, design px from the focused icon's place.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Place {
    x: f32,
    w: f32,
    h: f32,
    opacity: f32,
}

/// The icon `d` places from the focus (fractional while the row glides),
/// from the focused one's place: the focused one at 0 at full size, the
/// others [`GAP`] apart; those more than `lead` places before it fade as
/// they leave on the left. Between two places every icon is between their
/// sizes, so neighbours never overlap.
fn place(d: f32, off: (f32, f32), on: (f32, f32), lead: f32) -> Place {
    let k = d.abs().min(1.);
    let (w, h) = (on.0 + (off.0 - on.0) * k, on.1 + (off.1 - on.1) * k);
    let x = if d >= 1. {
        on.0 + GAP + (d - 1.) * (off.0 + GAP)
    } else if d >= 0. {
        d * (on.0 + GAP)
    } else {
        d * (off.0 + GAP)
    };
    let opacity = if d < -lead { (1. + d + lead).clamp(0., 1.).powf(1.5) } else { 1. };
    Place { x, w, h, opacity }
}

/// How many icons show before the focused one for the row at `pos` (up to
/// [`LEAD`]): the row only starts to slide once the focus passes them.
fn lead(pos: f32) -> f32 {
    pos.clamp(0., LEAD)
}

/// The "All games" icon: the grid of the library on a soft tile.
fn all_games(w: f32, h: f32, k: f32) -> Div {
    div()
        .size_full()
        .rounded(u(look::tile_radius()))
        .flex()
        .items_center()
        .justify_center()
        .bg(kit::ink(0.12))
        .border_1()
        .border_color(kit::ink(0.12))
        .child(crate::ui::icon(Icon::GridView, px(w.min(h) * k * 0.36), theme::text_primary()))
}

impl CollectionView {
    /// The row: `found`'s icons and "All games" after them, at `look`'s
    /// position, the name of the focused one (`index`) under it.
    /// `to_row`: the presses that bring the focus to the row (a click on an
    /// icon sends them, then ← → to it).
    pub(super) fn spotlight_row(&mut self, found: &[SharedString], index: usize, look: &Look, to_row: &[Pad], window: &Window) -> Div {
        let k = kit::scale();
        let dpr = window.scale_factor();
        let (off, on) = (look.off, look.on);
        let lead = lead(look.pos);
        let anchor = SIDE + lead * (off.0 + GAP);
        let view = f32::from(crate::ui::content_size(window).width) / k;
        let theme = look::theme();
        let radius = look::tile_radius();
        let ring_colour: Hsla = theme::console_focus().into();
        let mut row = div().relative().flex_none().w_full().h(u(row_height(on)));
        let mut names = Vec::new();
        for i in 0..=found.len() {
            let d = i as f32 - look.pos;
            let p = place(d, off, on, lead);
            let x = anchor + p.x;
            if p.opacity <= 0.01 || x > view || x + p.w < 0. {
                continue;
            }
            let near = (1. - d.abs()).clamp(0., 1.);
            let face = match found.get(i) {
                Some(key) => {
                    // Above the covers of other screens still queued (a grid's
                    // are 50): the row on screen loads first, the focus nearest.
                    let priority = if i == index { 200 } else { 150 - 10 * (d.abs() as i32).min(8) };
                    // Requested once at the focused size; smaller, it is scaled.
                    let src = self.console_shaped_art(key, theme.tiles.shape, on.0, on.1, priority, window).0;
                    let g = self.lib.as_ref().and_then(|l| l.get(key));
                    let grey = theme.tiles.grey_uninstalled && g.is_some_and(|g| !g.installed);
                    let running = g.and_then(|g| {
                        if self.preparing.contains_key(&g.app_name) {
                            Some(tr!("Preparing"))
                        } else {
                            self.playing.contains_key(&g.app_name).then(|| tr!("Playing"))
                        }
                    });
                    let title = g.map_or(String::new(), |g| g.title.clone());
                    let categories = self.console_categories(key);
                    // The copies take the top-right, or go beside "Playing" when a category style has it.
                    let corner_taken =
                        categories.as_ref().is_some_and(|(style, _)| crate::collection::category_badges::takes_top_right(*style));
                    let copies = Some(self.console_copy_count(key)).filter(|n| *n > 1);
                    let categories = categories.and_then(|(style, list)| {
                        crate::collection::category_badges::overlay(
                            style,
                            &list,
                            super::super::super::tile::category_area(p.w, k, dpr),
                            false,
                        )
                    });
                    // The picture carries its own corners: a clip is square.
                    div()
                        .relative()
                        .size_full()
                        .rounded(u(radius))
                        .bg(kit::ink(0.08))
                        .child(match src {
                            Some(s) => {
                                img(s).size_full().rounded(u(radius)).object_fit(ObjectFit::Cover).grayscale(grey).into_any_element()
                            }
                            // No picture: the name in the tile.
                            None => div()
                                .size_full()
                                .flex()
                                .items_end()
                                .p(u(8.))
                                .text_size(u(12.))
                                .line_height(u(15.))
                                .font_weight(theme::W_BOLD)
                                .text_color(theme::text_primary())
                                .overflow_hidden()
                                .child(div().line_clamp(4).child(title))
                                .into_any_element(),
                        })
                        .children(categories)
                        .when_some(running.filter(|_| theme.tiles.badges != Badges::Off), |d, b| {
                            d.child(div().absolute().top(u(6.)).left(u(6.)).child(kit::badge(b, theme::console_btn_a())))
                        })
                        .when_some(copies, |d, n| {
                            let badge = super::super::super::tile::copies_badge(n).absolute().top(u(6.));
                            d.child(match (corner_taken, running.is_some() && theme.tiles.badges != Badges::Off) {
                                (false, _) => badge.right(u(6.)),
                                // Under "Playing", on the left.
                                (true, true) => badge.top(u(34.)).left(u(6.)),
                                (true, false) => badge.left(u(6.)),
                            })
                        })
                }
                None => all_games(p.w, p.h, k),
            };
            if d.abs() < 1. {
                names.push(name_label(self.spotlight_name(found, i), x, p, d, look));
            }
            let ring = near * look.ring;
            let mut pads = to_row.to_vec();
            pads.extend(pointer::along(index, i, Pad::Left, Pad::Right));
            let pads = if to_row.is_empty() { pointer::focus_or_activate(pads) } else { pads };
            let tile = div()
                .on_pads(pads)
                .absolute()
                .left(u(x))
                .top(u(PAD))
                .w(u(p.w))
                .h(u(p.h))
                .opacity(p.opacity)
                .child(
                    div()
                        .size_full()
                        .rounded(u(radius))
                        .overflow_hidden()
                        .shadow(kit::shadow(0.35 + 0.25 * near, 6. + 8. * near, 14. + 18. * near))
                        .child(face),
                )
                .when(ring > 0.01, |d| {
                    // A thin ring a little way out, and a soft glow of the same colour.
                    d.child(
                        div()
                            .absolute()
                            .top(-u(RING_GAP))
                            .left(-u(RING_GAP))
                            .right(-u(RING_GAP))
                            .bottom(-u(RING_GAP))
                            .rounded(u(radius + RING_GAP))
                            .border(u(RING))
                            .border_color(Hsla { a: ring, ..ring_colour })
                            .shadow(vec![BoxShadow {
                                color: Hsla { a: 0.22 * ring, ..ring_colour },
                                offset: point(px(0.), px(0.)),
                                blur_radius: u(18.),
                                spread_radius: px(0.),
                                inset: false,
                            }]),
                    )
                });
            row = row.child(tile);
        }
        row.children(names)
    }

    /// The name shown under icon `i` of the row ("All games" past the games).
    pub(super) fn spotlight_name(&self, found: &[SharedString], i: usize) -> String {
        match found.get(i) {
            Some(key) => self.lib.as_ref().and_then(|l| l.get(key)).map_or(String::new(), |g| g.title.clone()),
            None => tr!("All games").to_string(),
        }
    }
}

/// The name of the icon `p` at `x` (`d` places from the focus) just right of
/// it, on the line of its bottom edge: the focused icon reaches lower than
/// the others, so the name reads under the next ones. It rides along with
/// its icon and is fully there only at the focus: the name of the icon
/// arriving fades in as the one leaving fades out.
fn name_label(name: String, x: f32, p: Place, d: f32, look: &Look) -> Div {
    let fade = (1. - d.abs()).clamp(0., 1.).powi(3);
    div()
        .absolute()
        .left(u(x + p.w + NAME_GAP))
        .top(u(PAD + p.h - NAME * 1.3 + 1.))
        .max_w(u(NAME_MAX))
        .text_size(u(NAME))
        .line_height(u(NAME * 1.3))
        .font_weight(theme::W_BOLD)
        .text_color(kit::strong())
        .whitespace_nowrap()
        .truncate()
        // A step back while the focus is below the row.
        .opacity(fade * (0.6 + 0.4 * look.ring))
        .child(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` above brings gpui's own `test` macro.
    use core::prelude::v1::test;

    const OFF: (f32, f32) = (80., 80.);
    const ON: (f32, f32) = (110., 110.);

    #[test]
    fn the_focus_is_full_size_and_its_neighbours_a_gap_away() {
        assert_eq!(place(0., OFF, ON, 2.), Place { x: 0., w: 110., h: 110., opacity: 1. });
        let next = place(1., OFF, ON, 2.);
        assert_eq!((next.x, next.w), (110. + GAP, 80.));
        assert_eq!(place(2., OFF, ON, 2.).x, 110. + GAP + 80. + GAP);
        assert_eq!(place(-1., OFF, ON, 2.).x, -(80. + GAP));
        assert_eq!(place(-2., OFF, ON, 2.).opacity, 1.);
        assert_eq!(place(-3., OFF, ON, 2.).opacity, 0.);
    }

    #[test]
    fn icons_never_overlap_while_the_row_glides() {
        for step in -30..40 {
            let d = step as f32 / 10.;
            let (a, b) = (place(d, OFF, ON, 2.), place(d + 1., OFF, ON, 2.));
            let gap = b.x - (a.x + a.w);
            assert!(gap >= GAP - 1e-3, "{d}: {gap}");
        }
    }

    #[test]
    fn the_row_slides_once_the_focus_passes_the_lead() {
        assert_eq!(lead(0.), 0.);
        assert_eq!(lead(1.5), 1.5);
        assert_eq!(lead(7.), LEAD);
    }
}
