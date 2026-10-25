//! The row of icons along the top of Spotlight, as on a console's home: the
//! focused icon at the left edge, larger, its name beside it, the next
//! icons after the name and the ones before it sliding off to the left.
//! Every icon's place and size, and its name's, come from one number, the
//! row's position (the focused index, gliding on a spring), so a move, a held
//! direction and the room for a longer or shorter name all change smoothly
//! together.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{Badges, TileShape, TileSize};
use heronite_i18n::tr;

use super::super::super::super::CollectionView;
use super::{Look, SIDE};
use crate::console::kit::{self, u};
use crate::console::look;
use crate::icons::Icon;
use crate::theme;

/// Side of an icon and of the focused one (square, medium covers), design px.
const ICON: f32 = 92.;
const ICON_ON: f32 = 130.;
/// Between two icons, design px.
const GAP: f32 = 14.;
/// Room above the icons (for the focus ring and its glow), design px.
pub(super) const PAD: f32 = 12.;
/// The name beside the focused icon: its size, the space on each side of it
/// and its widest (a longer one goes on two lines), design px.
const NAME: f32 = 19.;
const NAME_PAD: f32 = 20.;
const NAME_MAX: f32 = 250.;
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

/// Height of the row (the focused icon and the room above and below it), design px.
pub(super) fn row_height(on: (f32, f32)) -> f32 {
    on.1 + 2. * PAD
}

/// Where an icon sits and how large, design px from the focused icon's place.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Place {
    x: f32,
    w: f32,
    h: f32,
    opacity: f32,
}

/// The icon `d` places from the focus (fractional while the row glides):
/// the focused one at 0 at full size, the next one after the name's `room`,
/// the others [`GAP`] apart; the ones before the focus slide off to the left
/// and fade. Between two places every icon is between their sizes, so
/// neighbours never overlap.
fn place(d: f32, off: (f32, f32), on: (f32, f32), room: f32) -> Place {
    let k = d.abs().min(1.);
    let (w, h) = (on.0 + (off.0 - on.0) * k, on.1 + (off.1 - on.1) * k);
    let next = on.0 + room;
    let x = if d >= 1. {
        next + (d - 1.) * (off.0 + GAP)
    } else if d >= 0. {
        next * d
    } else {
        d * (off.0 + GAP)
    };
    let opacity = if d < 0. { (1. + d).clamp(0., 1.).powf(1.5) } else { 1. };
    Place { x, w, h, opacity }
}

/// Room between the focused icon and the next one for a name `name_w` wide.
pub(super) fn name_room(name_w: f32) -> f32 {
    (name_w + 2. * NAME_PAD).max(GAP)
}

/// How wide `text` is in the name's type, at most [`NAME_MAX`], design px.
pub(super) fn name_width(text: &str, window: &Window) -> f32 {
    if text.is_empty() {
        return 0.;
    }
    let family = SharedString::from(look::fonts().1);
    let run = TextRun {
        len: text.len(),
        font: Font { weight: theme::W_BOLD, ..font(family) },
        color: theme::text_primary().into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window.text_system().shape_line(SharedString::from(text.to_string()), u(NAME), &[run], None);
    // A pixel more for the rounding of the laid-out text.
    (f32::from(line.width) / kit::scale() + 1.).min(NAME_MAX)
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
    /// position, the name of the focused one (`index`) beside it.
    pub(super) fn spotlight_row(&mut self, found: &[SharedString], index: usize, look: &Look, window: &Window) -> Div {
        let k = kit::scale();
        let dpr = window.scale_factor();
        let (off, on) = (look.off, look.on);
        let room = name_room(look.name_w);
        let view = f32::from(window.viewport_size().width) / k;
        let theme = look::theme();
        let landscape = theme.tiles.shape == TileShape::Landscape;
        let radius = look::tile_radius();
        let ring_colour: Hsla = theme::console_focus().into();
        let mut row = div().relative().flex_none().w_full().h(u(row_height(on)));
        let mut names = Vec::new();
        for i in 0..=found.len() {
            let d = i as f32 - look.pos;
            let p = place(d, off, on, room);
            let x = SIDE + p.x;
            if p.opacity <= 0.01 || x > view {
                continue;
            }
            let near = (1. - d.abs()).clamp(0., 1.);
            let face = match found.get(i) {
                Some(key) => {
                    let priority = if i == index { 200 } else { 90 - 10 * (d.abs() as i32).min(8) };
                    // Requested once at the focused size; smaller, it is scaled.
                    let src = if landscape {
                        self.console_wide_art(key, on.0, on.1, priority, window)
                    } else {
                        self.cover(key, on.0 * k, on.1 * k, dpr, priority)
                    };
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
                        .when_some(running.filter(|_| theme.tiles.badges != Badges::Off), |d, b| {
                            d.child(div().absolute().top(u(6.)).left(u(6.)).child(kit::badge(b, theme::console_btn_a())))
                        })
                }
                None => all_games(p.w, p.h, k),
            };
            if d.abs() < 1. {
                let name = self.spotlight_name(found, i);
                let width = if i == index { look.name_target } else { name_width(&name, window) };
                // Never over the next icon while the room for it opens or closes.
                let next = place(d + 1., off, on, room);
                let clip = (next.x - p.x - p.w - 2. * NAME_PAD).max(0.);
                names.push(name_label(name, width, clip, p, d, off, look));
            }
            let ring = near * look.ring;
            let tile = div()
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

    /// The name shown beside icon `i` of the row ("All games" past the games).
    pub(super) fn spotlight_name(&self, found: &[SharedString], i: usize) -> String {
        match found.get(i) {
            Some(key) => self.lib.as_ref().and_then(|l| l.get(key)).map_or(String::new(), |g| g.title.clone()),
            None => tr!("All games").to_string(),
        }
    }
}

/// The name of an icon `p` (`d` places from the focus) beside it, in line
/// with the middle of the other icons. It rides along with its icon and is
/// fully there only at the focus: the name of the icon arriving fades in as
/// the one leaving fades out. It shows `clip` design px of its `width`, the
/// room up to the next icon.
fn name_label(name: String, width: f32, clip: f32, p: Place, d: f32, off: (f32, f32), look: &Look) -> Div {
    let fade = (1. - d.abs()).clamp(0., 1.).powi(2);
    div()
        .absolute()
        .left(u(SIDE + p.x + p.w + NAME_PAD))
        .top(u(PAD))
        .h(u(off.1))
        .w(u(clip + 1.))
        .overflow_hidden()
        .flex()
        .items_center()
        // A step back while the focus is below the row.
        .opacity(fade * (0.6 + 0.4 * look.ring))
        .child(
            div()
                .flex_none()
                .w(u(width + 1.))
                .text_size(u(NAME))
                .line_height(u(NAME * 1.22))
                .font_weight(theme::W_BOLD)
                .text_color(kit::strong())
                .line_clamp(2)
                .child(name),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` above brings gpui's own `test` macro.
    use core::prelude::v1::test;

    const OFF: (f32, f32) = (80., 80.);
    const ON: (f32, f32) = (110., 110.);

    #[test]
    fn the_focus_is_at_the_edge_and_full_size() {
        assert_eq!(place(0., OFF, ON, 200.), Place { x: 0., w: 110., h: 110., opacity: 1. });
        let next = place(1., OFF, ON, 200.);
        assert_eq!((next.x, next.w), (310., 80.));
        assert_eq!(place(2., OFF, ON, 200.).x, 310. + 80. + GAP);
        assert_eq!(place(-1., OFF, ON, 200.).opacity, 0.);
    }

    #[test]
    fn icons_never_overlap_while_the_row_glides() {
        for room in [GAP, 120., NAME_MAX + 2. * NAME_PAD] {
            for step in -30..40 {
                let d = step as f32 / 10.;
                let (a, b) = (place(d, OFF, ON, room), place(d + 1., OFF, ON, room));
                let gap = b.x - (a.x + a.w);
                assert!(gap >= GAP.min(room) - 1e-3, "{room} {d}: {gap}");
            }
        }
    }

    #[test]
    fn the_name_room_has_a_floor() {
        assert_eq!(name_room(0.), 2. * NAME_PAD);
        assert_eq!(name_room(100.), 100. + 2. * NAME_PAD);
    }
}
