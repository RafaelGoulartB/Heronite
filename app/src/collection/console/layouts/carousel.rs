//! Carousel: one large cover in the middle with its neighbours smaller and
//! fainter to each side, the focused game's name (or logo), facts and
//! "n of N" under it. ◀ ▶ turn it; ↑ moves to a row of filter pills (All,
//! Installed, Favourites, Playing) and ↓ comes back. GPUI has no 3D
//! transforms, so a cover's pose is its size, offset and opacity, each cover
//! on a spring of its own distance from the middle.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{TabSpec, TileShape, TileSize};
use heronite_data::CollectionGame;
use heronite_i18n::{n_, tr};

use super::super::super::CollectionView;
use super::super::hero::HeroSize;
use super::super::sources::{by_title, keys};
use crate::console::kit::{self, u};
use crate::console::look;
use crate::console::motion;
use crate::console::{hint, Btn, Hint, Pad};
use crate::icons::Icon;
use crate::theme;

const FILTERS: [&str; 4] = [n_!("All"), n_!("Installed"), n_!("Favourites"), n_!("Playing")];
/// LT / RT jump this many games.
const JUMP: usize = 10;
/// Covers drawn on each side of the middle one.
const REACH: usize = 5;
/// How the covers move when the focus does: a touch under critical damping.
const TURN: SpringConfig = SpringConfig::new(280., 31., 1.);

/// Scale, offset (in cover widths) and opacity of a cover 0 to 5 places
/// from the middle.
const SCALE: [f32; 6] = [1., 0.76, 0.6, 0.48, 0.4, 0.34];
const OFFSET: [f32; 6] = [0., 0.88, 1.52, 2.0, 2.38, 2.7];
const OPACITY: [f32; 6] = [1., 0.9, 0.62, 0.36, 0.14, 0.];

#[derive(Default)]
pub(crate) struct Nav {
    /// Index in [`FILTERS`].
    filter: usize,
    index: usize,
    /// The focus is on the row of pills.
    pills: bool,
}

/// `table` between its entries for a (fractional) distance `a` from the middle.
fn at(table: &[f32; 6], a: f32) -> f32 {
    let a = a.clamp(0., 5.);
    let i = (a.floor() as usize).min(4);
    table[i] + (table[i + 1] - table[i]) * (a - i as f32)
}

/// The pose (scale, offset in cover widths, opacity) of a cover `d` places
/// from the middle; the offset has the side of `d`.
fn pose(d: f32) -> (f32, f32, f32) {
    let a = d.abs();
    (at(&SCALE, a), at(&OFFSET, a) * if d < 0. { -1. } else { 1. }, at(&OPACITY, a))
}

/// Size of the middle cover, design px: the theme's size (and shape), within
/// `max_h` high and `max_w` wide.
fn cover_size(shape: TileShape, size: TileSize, aspect: f32, max_w: f32, max_h: f32) -> (f32, f32) {
    let base = match size {
        TileSize::Small => 190.,
        TileSize::Medium => 230.,
        TileSize::Large => 270.,
        TileSize::Huge => 300.,
    };
    let (mut w, ratio): (f32, f32) = match shape {
        TileShape::Portrait => (base, aspect),
        TileShape::Square => (base, 1.),
        TileShape::Landscape => (base * 1.7, 9. / 16.),
    };
    w = w.min(max_w).min(max_h / ratio);
    (w, w * ratio)
}

/// `build(el, d)` with `d` springing towards `target` (a new element starts
/// there); at once without animations.
fn turn<E: IntoElement + 'static>(id: impl Into<ElementId>, el: E, target: f32, build: impl FnOnce(E, f32) -> E + 'static) -> AnyElement {
    if !motion::on() {
        return build(el, target).into_any_element();
    }
    el.with_spring(id, SpringAnimation::new(TURN).to(target).with_epsilon(0.003), build).into_any_element()
}

impl CollectionView {
    /// The games of the tab that pass pill `filter`, sorted A–Z.
    fn carousel_keys(&self, spec: &TabSpec, filter: usize) -> Vec<SharedString> {
        let Some(lib) = self.lib.as_ref() else { return Vec::new() };
        let playing = |g: &CollectionGame| {
            self.playing.contains_key(&g.app_name)
                || lib.statuses.iter().any(|s| s.id == g.status_id && matches!(s.slug.as_str(), "playing" | "endless"))
        };
        let games = self.console_games(spec.source.as_str());
        let picked: Vec<&CollectionGame> = games
            .iter()
            .map(|g| g.as_ref())
            .filter(|g| match filter {
                1 => g.installed,
                2 => g.favourite,
                3 => playing(g),
                _ => true,
            })
            .collect();
        keys(by_title(picked))
    }

    pub(super) fn render_carousel(&mut self, tab: usize, spec: &TabSpec, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = look::theme();
        let nav = &self.tab_nav(tab).carousel;
        let (filter, pills_focus) = (nav.filter.min(FILTERS.len() - 1), nav.pills);
        let all = self.carousel_keys(spec, filter);
        let n = all.len();
        let nav = &mut self.tab_nav(tab).carousel;
        nav.index = nav.index.min(n.saturating_sub(1));
        let index = nav.index;
        let focused = all.get(index).cloned();
        self.console_focus_game(focused.clone(), cx);

        let pills =
            div().flex_none().flex().justify_center().gap(u(10.)).pt(u(4.)).children(FILTERS.iter().enumerate().map(|(j, label)| {
                let (on, current) = (pills_focus && j == filter, j == filter);
                kit::row_base(on)
                    .min_h(u(38.))
                    .px(u(20.))
                    .when(!on, |d| d.bg(kit::ink(if current { 0.16 } else { 0.07 })))
                    .when(current && !on, |d| d.text_color(kit::strong()))
                    .child(div().text_size(u(15.)).font_weight(theme::W_BOLD).child(heronite_i18n::tr(label)))
            }));
        let Some(focused) = focused else {
            return div()
                .flex()
                .flex_col()
                .size_full()
                .child(pills)
                .child(kit::empty(Icon::GridView, tr!("No games match"), ""))
                .into_any_element();
        };

        let k = kit::scale();
        let (vw, vh) = (f32::from(window.viewport_size().width), f32::from(window.viewport_size().height));
        // Room for the pills above and the name, facts and count below.
        let (cw, ch) = cover_size(theme.tiles.shape, theme.tiles.size, self.cover_aspect, vw / k * 0.42, vh / k - 76. - 60. - 52. - 140.);
        let (cwp, chp) = (cw * k, ch * k);
        let landscape = theme.tiles.shape == TileShape::Landscape;
        let radius = look::tile_radius() * k;
        let grey = theme.tiles.grey_uninstalled;
        let reflect = theme.layouts.carousel_reflect;

        // Far covers first, so the near ones paint over them.
        let mut order: Vec<usize> = (index.saturating_sub(REACH)..=(index + REACH).min(n - 1)).collect();
        order.sort_by_key(|i| std::cmp::Reverse(i.abs_diff(index)));
        let mut stage = div().relative().flex_none().w_full().h(px(chp) + u(22.));
        for i in order {
            let key = all[i].clone();
            let Some(g) = self.lib.as_ref().and_then(|l| l.get(&key)).cloned() else { continue };
            let src = if landscape {
                self.console_wide_art(&key, cw, ch, if i == index { 200 } else { 60 }, window)
            } else {
                self.cover(&key, cwp, chp, window.scale_factor(), if i == index { 200 } else { 60 })
            };
            let ring = i == index && !pills_focus;
            let grey = grey && !g.installed;
            let title = g.title.clone();
            let d = i as f32 - index as f32;
            let centre = vw / 2.;
            stage = stage.child(turn(SharedString::from(format!("cr-{tab}-{key}")), div().absolute(), d, move |el, d| {
                let (s, off, op) = pose(d);
                let (w, h) = (cwp * s, chp * s);
                // Scaled around a point at 60% of the height.
                let top = chp * 0.6 - h * 0.6;
                let near = (1. - d.abs()).clamp(0., 1.);
                let ring_colour: Hsla = theme::console_focus().into();
                let face = div()
                    .size_full()
                    .rounded(px(radius))
                    .overflow_hidden()
                    .bg(kit::ink(0.08))
                    .shadow(kit::shadow(0.55, 14., 30.))
                    .child(match src {
                        Some(s) => img(s).size_full().object_fit(ObjectFit::Cover).grayscale(grey).into_any_element(),
                        None => div()
                            .size_full()
                            .flex()
                            .items_end()
                            .p(px(10. * k))
                            .text_size(px(15. * k * s))
                            .font_weight(theme::W_BOLD)
                            .text_color(theme::text_primary())
                            .child(title)
                            .into_any_element(),
                    })
                    .when(ring, |d| {
                        d.child(div().absolute().inset_0().rounded(px(radius)).border_3().border_color(Hsla { a: near, ..ring_colour }))
                    });
                el.left(px(centre - w / 2. + off * cwp)).top(px(top)).w(px(w)).h(px(h)).opacity(op).child(face).when(reflect, |el| {
                    // A faded floor under the cover (images cannot be flipped).
                    el.child(
                        div().absolute().left_0().right_0().top(px(h + 6. * k)).h(px(h * 0.22)).rounded(px(radius)).bg(linear_gradient(
                            180.,
                            linear_color_stop(kit::ink(0.16), 0.),
                            linear_color_stop(kit::ink(0.), 1.),
                        )),
                    )
                })
            }));
        }

        let g = self.lib.as_ref().and_then(|l| l.get(&focused)).cloned();
        let info = g.map(|g| {
            let size = HeroSize { title: 30., logo: (420., 62.), max_w: 760., overview: 0 };
            let name = self.console_name(&g, size, window);
            let info = div()
                .relative()
                .flex()
                .flex_col()
                .items_center()
                .gap(u(8.))
                .child(name)
                .when(theme.hero.facts, |d| d.child(self.console_fact_line(&g, Self::console_facts(&g), 15.)))
                .child(div().text_size(u(13.5)).text_color(theme::text_dim()).child(tr!(
                    "{index} of {total}",
                    index = index + 1,
                    total = n
                )));
            motion::enter_from(SharedString::from(format!("cr-info-{}", g.key)), info, 240, 0., 8., 0.3)
        });
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(pills)
            .child(div().flex_1().min_h_0().flex().flex_col().justify_center().child(stage).children(info))
            .into_any_element()
    }

    pub(super) fn carousel_pad(&mut self, tab: usize, spec: &TabSpec, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let wrap = look::theme().nav.wrap;
        let filter = self.tab_nav(tab).carousel.filter.min(FILTERS.len() - 1);
        let all = self.carousel_keys(spec, filter);
        let n = all.len();
        let nav = &mut self.tab_nav(tab).carousel;
        if nav.pills {
            let next = match pad {
                Pad::Left => filter.saturating_sub(1),
                Pad::Right => (filter + 1).min(FILTERS.len() - 1),
                Pad::Down | Pad::A | Pad::B => {
                    nav.pills = false;
                    return true;
                }
                _ => return false,
            };
            if next != filter {
                nav.filter = next;
                nav.index = 0;
            }
            return true;
        }
        let i = nav.index.min(n.saturating_sub(1));
        match pad {
            Pad::Left | Pad::Right => nav.index = super::step(i, n, pad.dir(), wrap),
            Pad::LT => nav.index = i.saturating_sub(JUMP),
            Pad::RT => nav.index = (i + JUMP).min(n.saturating_sub(1)),
            Pad::Up => nav.pills = true,
            Pad::A | Pad::X => {
                if let Some(k) = all.get(i) {
                    self.console_activate(k.clone(), pad, window, cx);
                }
            }
            Pad::Y => self.open_console_filters(window, cx),
            _ => return false,
        }
        true
    }

    pub(super) fn carousel_hints(&self, tab: usize, _spec: &TabSpec) -> Vec<Hint> {
        if self.console.tab_navs.get(tab).is_some_and(|n| n.carousel.pills) {
            let mut h = vec![hint(Btn::LeftRight, tr!("Filter")), hint(Btn::B, tr!("Back"))];
            h.extend(self.console_tab_hints());
            return h;
        }
        let mut h = self.console_cover_hints();
        h.insert(0, hint(Btn::LeftRight, tr!("Move")));
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` above brings gpui's own `test` macro.
    use core::prelude::v1::test;

    #[test]
    fn poses_shrink_fade_and_mirror() {
        assert_eq!(pose(0.), (1., 0., 1.));
        let (s1, o1, p1) = pose(1.);
        let (s2, o2, p2) = pose(2.);
        assert!(s2 < s1 && p2 < p1 && o2 > o1);
        assert_eq!(pose(-2.), (s2, -o2, p2));
        // Halfway between two places is halfway between their poses.
        assert!((pose(0.5).0 - (SCALE[0] + SCALE[1]) / 2.).abs() < 1e-6);
        assert_eq!(pose(9.).2, 0.);
    }

    #[test]
    fn the_cover_fits_its_room() {
        let (w, h) = cover_size(TileShape::Portrait, TileSize::Huge, 1.5, 600., 300.);
        assert!((h - 300.).abs() < 1e-3 && (w - 200.).abs() < 1e-3);
        let (w, h) = cover_size(TileShape::Landscape, TileSize::Small, 1.5, 200., 900.);
        assert_eq!(w, 200.);
        assert!((h - 200. * 9. / 16.).abs() < 1e-3);
        let (w, _) = cover_size(TileShape::Portrait, TileSize::Medium, 1.5, 900., 900.);
        assert_eq!(w, 230.);
    }
}
