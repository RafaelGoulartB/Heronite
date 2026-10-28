//! Carousel: one large cover in the middle, its neighbours smaller, darker
//! and spaced out to each side, all over a glossy floor (each cover's
//! reflection); the focused game's name, facts and place in the list under
//! it. ◀ ▶ turn it and LT / RT jump to the previous / next letter (the
//! letter shows over the covers for a moment); ↑ moves to a row of filter
//! pills (All, Installed, Favourites, Playing) and ↓ comes back, and with a
//! single tab LB / RB change the filter from anywhere. The covers fan out
//! from the middle when the carousel first shows and when the filter
//! changes. GPUI has no 3D transforms, so a cover's pose is its size,
//! offset, opacity and shade, each cover on a spring of its own distance
//! from the middle.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{HeroTitle, TabSpec, TileShape, TileSize};
use heronite_data::{CollectionGame, Library};
use heronite_i18n::{n_, tr};
use heronite_images::{Variant, REFLECTION_PART};

use super::super::super::CollectionView;
use super::super::hero::{fitted_title, HeroSize};
use super::super::sources::{by_title, keys, ListKey};
use super::grid::{jump_letter, letter_of};
use crate::console::kit::{self, u};
use crate::console::look;
use crate::console::motion;
use crate::console::{hint, Btn, Hint, Pad};
use crate::icons::Icon;
use crate::theme;

const FILTERS: [&str; 4] = [n_!("All"), n_!("Installed"), n_!("Favourites"), n_!("Playing")];
/// Index of "Playing" in [`FILTERS`].
const PLAYING: usize = 3;
/// Covers drawn on each side of the middle one.
const REACH: usize = 5;
/// How the covers move when the focus does: a touch under critical damping.
const TURN: SpringConfig = SpringConfig::new(280., 31., 1.);

/// Scale, opacity and shade (a dim over the art) of a cover 0 to 5 places
/// from the middle.
const SCALE: [f32; 6] = [1., 0.8, 0.66, 0.56, 0.48, 0.42];
const OPACITY: [f32; 6] = [1., 0.94, 0.74, 0.48, 0.2, 0.];
const SHADE: [f32; 6] = [0., 0.3, 0.44, 0.54, 0.6, 0.64];
/// Space between two covers, in widths of the middle one.
const GAP: f32 = 0.08;
/// Offset of each place from the middle, in widths of the middle cover:
/// every cover [`GAP`] from the next, whatever their sizes.
const OFFSET: [f32; 6] = {
    let mut o = [0.; 6];
    let mut i = 1;
    while i < 6 {
        o[i] = o[i - 1] + (SCALE[i - 1] + SCALE[i]) / 2. + GAP;
        i += 1;
    }
    o
};
/// Covers shrink towards this point of their height: the far ones' bottoms
/// rise a little, like a floor seen in perspective.
const ANCHOR: f32 = 0.78;
/// Opacity of the reflections, and the part of one (of the cover's height)
/// above the name: the name sits over its faded tail.
const FLOOR: f32 = 0.3;
const FLOOR_ROOM: f32 = 0.14;
/// Design px the filters take above the covers.
const PILLS_H: f32 = 58.;
/// Heights of the lines under the covers (the name, or the logo, the facts
/// and the place in the list) and the space between them, design px.
const NAME_H: f32 = 40.;
const LOGO_H: f32 = 72.;
const FACTS_H: f32 = 26.;
const PLACE_H: f32 = 30.;
const INFO_GAP: f32 = 10.;
/// Room under the info, design px.
const INFO_PAD: f32 = 16.;
/// The fan out of the covers, and how long a jump's letter shows.
const FAN_MS: u64 = 640;
const LETTER_MS: u64 = 950;
/// Width of the line that shows the place in the list, design px.
const TRACK: f32 = 180.;

#[derive(Default)]
pub(crate) struct Nav {
    /// Index in [`FILTERS`].
    filter: usize,
    index: usize,
    /// The focus is on the row of pills.
    pills: bool,
    /// The side the last move came from (-1 left, 1 right): the name slides
    /// in from it.
    from: f32,
    /// LT / RT jumps so far (each shows its letter).
    jumps: u64,
    /// When the covers fanned out last (`None`: they fan out on the next frame).
    shown: Option<Instant>,
    /// Counts the fans (part of their ids, so each one plays).
    fans: u64,
}

impl Nav {
    /// A new filter, the focus on `index`; the covers fan out again.
    fn set_filter(&mut self, filter: usize, index: usize) {
        self.filter = filter;
        self.index = index;
        self.from = 0.;
        self.shown = None;
        self.fans += 1;
    }
}

/// `table` between its entries for a (fractional) distance `a` from the middle.
fn at(table: &[f32; 6], a: f32) -> f32 {
    let a = a.clamp(0., 5.);
    let i = (a.floor() as usize).min(4);
    table[i] + (table[i + 1] - table[i]) * (a - i as f32)
}

/// The pose of a cover `d` places from the middle.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Pose {
    scale: f32,
    /// In widths of the middle cover, with the side of `d`.
    offset: f32,
    opacity: f32,
    shade: f32,
}

fn pose(d: f32) -> Pose {
    let a = d.abs();
    Pose { scale: at(&SCALE, a), offset: at(&OFFSET, a) * d.signum(), opacity: at(&OPACITY, a), shade: at(&SHADE, a) }
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
    w = w.min(max_w).min(max_h / ratio).max(1.);
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

/// The row of filters: the current one on a soft fill, on the focus pill
/// while the row has the focus; the row steps back while the covers have it.
fn filter_pills(tab: usize, filter: usize, focused: bool) -> AnyElement {
    let row = div().flex_none().flex().justify_center().gap(u(6.)).pt(u(6.)).children(FILTERS.iter().enumerate().map(|(j, label)| {
        let (on, current) = (focused && j == filter, j == filter);
        kit::row_base(on)
            .min_h(u(36.))
            .px(u(18.))
            .when(!on && current, |d| d.bg(kit::ink(0.14)).text_color(kit::strong()))
            .when(!current, |d| d.text_color(theme::text_muted()))
            .child(div().text_size(u(15.)).font_weight(if current { theme::W_BOLD } else { theme::W_REGULAR }).child(tr(label)))
    }));
    motion::spring(SharedString::from(format!("cr-pills-{tab}")), row, motion::SOFT, focused, |d, k| d.opacity(0.62 + 0.38 * k))
}

/// The place in the list: the letter, a short line with a mark that glides
/// along it, "12 of 219".
fn place(tab: usize, letter: char, index: usize, n: usize) -> Div {
    let k = kit::scale();
    let mark = (TRACK / n as f32).clamp(12., 44.);
    let at = if n > 1 { index as f32 / (n - 1) as f32 } else { 0. };
    let line = div().relative().flex_none().w(u(TRACK)).h(u(4.)).rounded_full().bg(kit::ink(0.14)).child(turn(
        SharedString::from(format!("cr-place-{tab}")),
        div().absolute().top_0().h_full().w(u(mark)).rounded_full().bg(kit::strong()),
        at,
        move |el, at| el.left(px((TRACK - mark) * k * at)),
    ));
    // Both sides as wide, so the line stays in the middle.
    let side = |w: f32| div().flex_none().w(u(w)).whitespace_nowrap();
    div()
        .flex()
        .items_center()
        .gap(u(14.))
        .text_size(u(13.5))
        .child(side(96.).flex().justify_end().font_weight(theme::W_BOLD).text_color(theme::text_muted()).child(letter.to_string()))
        .child(line)
        .child(side(96.).text_color(theme::text_dim()).child(tr!("{index} of {total}", index = index + 1, total = n)))
}

/// The letter of a jump, large over the covers; it fades away.
fn letter_flash(id: SharedString, letter: char) -> AnyElement {
    let card = div()
        .flex()
        .items_center()
        .justify_center()
        .size(u(128.))
        .rounded(u(30.))
        .bg(kit::shade(0.55))
        .border_1()
        .border_color(kit::ink(0.12))
        .shadow(kit::shadow(0.4, 12., 40.))
        .font_family(kit::heading_font())
        .font_weight(theme::W_BOLD)
        .text_size(u(68.))
        .text_color(kit::strong())
        .child(letter.to_string());
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .child(card)
        .with_animation(id, Animation::new(Duration::from_millis(LETTER_MS)), |d, t| {
            // In quickly, a moment still, then out.
            let o = if t < 0.12 { t / 0.12 } else { 1. - ((t - 0.12) / 0.88).powi(3) };
            d.opacity(o)
        })
        .into_any_element()
}

impl CollectionView {
    /// The games of the tab that pass pill `filter`, sorted A–Z.
    fn carousel_keys(&self, spec: &TabSpec, filter: usize) -> Arc<[SharedString]> {
        let order = |games: &[Arc<CollectionGame>], lib: &Library| self.carousel_order(games, lib, filter);
        if filter == PLAYING {
            // Also reads the games being played now, which the cache does not follow.
            let Some(lib) = self.lib.as_ref() else { return Arc::default() };
            return order(&self.console_games(&spec.source), lib).into();
        }
        self.console_list(ListKey::Carousel { source: spec.source.clone(), filter }, order)
    }

    fn carousel_order(&self, games: &[Arc<CollectionGame>], lib: &Library, filter: usize) -> Vec<SharedString> {
        let playing = |g: &CollectionGame| {
            self.playing.contains_key(&g.app_name)
                || lib.statuses.iter().any(|s| s.id == g.status_id && matches!(s.slug.as_str(), "playing" | "endless"))
        };
        let picked: Vec<&CollectionGame> = games
            .iter()
            .map(|g| g.as_ref())
            .filter(|g| match filter {
                1 => g.installed,
                2 => g.favourite,
                PLAYING => playing(g),
                _ => true,
            })
            .collect();
        keys(by_title(picked))
    }

    /// The letter of `key`'s title (`#` for digits and symbols).
    fn carousel_letter(&self, key: &str) -> char {
        self.lib.as_ref().and_then(|l| l.get(key)).map_or('#', |g| letter_of(&g.title))
    }

    /// The reflection of `key`'s cover (its wide art when `wide`), `pw`×`ph`
    /// physical px like the cover it is under.
    fn carousel_floor(&mut self, key: &SharedString, wide: bool, pw: u32, ph: u32, priority: i32) -> Option<ImageSource> {
        let id: SharedString = format!("cr:{key}:{pw}x{ph}").into();
        let wide = wide && self.stage_chains.get(key).is_some_and(|c| !c.is_empty());
        let chains = if wide { &self.stage_chains } else { &self.cover_chains };
        self.images
            .get_or_request(&id, || chains.get(key).cloned().unwrap_or_default(), pw, ph, Variant::Reflection, priority)
            .map(ImageSource::Render)
    }

    pub(super) fn render_carousel(&mut self, tab: usize, spec: &TabSpec, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = look::theme();
        let filter = self.tab_nav(tab).carousel.filter.min(FILTERS.len() - 1);
        let all = self.carousel_keys(spec, filter);
        let n = all.len();
        let nav = &mut self.tab_nav(tab).carousel;
        nav.index = nav.index.min(n.saturating_sub(1));
        // Nothing to turn: the filters keep the focus.
        if n == 0 {
            nav.pills = true;
        }
        let shown = *nav.shown.get_or_insert_with(Instant::now);
        let (index, pills_focus, from, jumps, fans) = (nav.index, nav.pills, nav.from, nav.jumps, nav.fans);
        let focused = all.get(index).cloned();
        self.console_focus_game(focused.clone(), cx);

        let pills = filter_pills(tab, filter, pills_focus);
        let Some(focused) = focused else {
            return div()
                .flex()
                .flex_col()
                .size_full()
                .child(pills)
                .child(div().flex_1().flex().items_center().justify_center().child(kit::empty(Icon::GridView, tr!("No games match"), "")))
                .into_any_element();
        };

        let k = kit::scale();
        let viewport = window.viewport_size();
        let (vw, dpr) = (f32::from(viewport.width), window.scale_factor());
        let info_h = if theme.hero.title == HeroTitle::Logo { LOGO_H } else { NAME_H }
            + if theme.hero.facts { FACTS_H + INFO_GAP } else { 0. }
            + INFO_GAP
            + PLACE_H
            + INFO_PAD;
        let room = (super::body_height(window, self.hints_visible()) - PILLS_H - info_h) / (1. + FLOOR_ROOM);
        let (cw, ch) = cover_size(theme.tiles.shape, theme.tiles.size, self.cover_aspect, vw / k * 0.42, room);
        let (cwp, chp) = (cw * k, ch * k);
        let (pw, ph) = ((cwp * dpr).round() as u32, (chp * dpr).round() as u32);
        let landscape = theme.tiles.shape == TileShape::Landscape;
        let radius = look::tile_radius() * k;
        let grey = theme.tiles.grey_uninstalled;
        let reflect = theme.layouts.carousel_reflect;
        let fan = motion::on() && !motion::reduced() && shown.elapsed() < Duration::from_millis(FAN_MS);
        let centre = vw / 2.;

        // Far covers first, so the near ones paint over them.
        let mut order: Vec<usize> = (index.saturating_sub(REACH)..=(index + REACH).min(n - 1)).collect();
        order.sort_by_key(|i| std::cmp::Reverse(i.abs_diff(index)));
        let mut stage = div().relative().flex_none().w_full().h(px(chp * (1. + FLOOR_ROOM)));
        for i in order {
            let key = all[i].clone();
            let Some(g) = self.lib.as_ref().and_then(|l| l.get(&key)).cloned() else { continue };
            let near = i.abs_diff(index);
            let priority = if near == 0 { 200 } else { 90 - 10 * near as i32 };
            let src =
                if landscape { self.console_wide_art(&key, cw, ch, priority, window) } else { self.cover(&key, cwp, chp, dpr, priority) };
            let floor = (reflect && near <= 3).then(|| self.carousel_floor(&key, landscape, pw, ph, priority - 30)).flatten();
            let ring = i == index && !pills_focus;
            let grey = grey && !g.installed;
            let title = g.title.clone();
            let d = i as f32 - index as f32;
            let cover = turn(SharedString::from(format!("cr-{tab}-{key}")), div().absolute(), d, move |el, d| {
                let p = pose(d);
                let (w, h) = (cwp * p.scale, chp * p.scale);
                let top = (chp - h) * ANCHOR;
                let near = (1. - d.abs()).clamp(0., 1.);
                let face = div()
                    .relative()
                    .size_full()
                    .rounded(px(radius))
                    .overflow_hidden()
                    .bg(kit::ink(0.08))
                    .shadow(kit::shadow(0.4 + 0.2 * near, 10. + 12. * near, 24. + 24. * near))
                    .child(match src {
                        Some(s) => img(s).size_full().object_fit(ObjectFit::Cover).grayscale(grey).into_any_element(),
                        None => div()
                            .size_full()
                            .flex()
                            .items_end()
                            .p(px(12. * k * p.scale))
                            .text_size(px(16. * k * p.scale))
                            .font_weight(theme::W_BOLD)
                            .text_color(theme::text_primary())
                            .child(title)
                            .into_any_element(),
                    })
                    .when(p.shade > 0.005, |f| f.child(div().absolute().inset_0().bg(kit::dim(p.shade))));
                // A thin ring a little way out from the cover.
                let gap = 6. * k;
                let ring_colour: Hsla = theme::console_focus().into();
                el.left(px(centre - w / 2. + p.offset * cwp))
                    .top(px(top))
                    .w(px(w))
                    .h(px(h))
                    .opacity(p.opacity)
                    .when_some(floor, |el, f| {
                        el.child(
                            img(f)
                                .absolute()
                                .left_0()
                                .top(px(h + 8. * k))
                                .w(px(w))
                                .h(px(h * REFLECTION_PART))
                                .rounded(px(radius))
                                .opacity(FLOOR * (1. - p.shade)),
                        )
                    })
                    .child(face)
                    .when(ring && near > 0.01, |el| {
                        el.child(
                            div()
                                .absolute()
                                .top(px(-gap))
                                .left(px(-gap))
                                .right(px(-gap))
                                .bottom(px(-gap))
                                .rounded(px(radius + gap))
                                .border(px(2.5 * k))
                                .border_color(Hsla { a: near, ..ring_colour }),
                        )
                    })
            });
            // The fan: each cover slides out from behind the middle one.
            let cover = if fan {
                let shift = pose(d).offset * cwp;
                div()
                    .absolute()
                    .inset_0()
                    .child(cover)
                    .with_animation(
                        SharedString::from(format!("cr-fan-{tab}-{fans}-{key}")),
                        Animation::new(Duration::from_millis(FAN_MS)).with_easing(ease_out_quint()),
                        move |el, t| el.left(px(-shift * (1. - t))).opacity(t),
                    )
                    .into_any_element()
            } else {
                cover
            };
            stage = stage.child(cover);
        }
        let letter = self.carousel_letter(&focused);
        if jumps > 0 && motion::on() {
            let flash = letter_flash(SharedString::from(format!("cr-letter-{tab}-{jumps}")), letter);
            stage = stage.child(div().absolute().top_0().left_0().right_0().h(px(chp)).child(flash));
        }

        let g = self.lib.as_ref().and_then(|l| l.get(&focused)).cloned();
        let info = g.map(|g| {
            let logo = theme.hero.title == HeroTitle::Logo;
            let name = if logo {
                self.console_name(&g, HeroSize { title: 32., logo: (420., LOGO_H), max_w: 880., overview: 0 }, window)
            } else {
                let size = fitted_title(32., g.title.chars().count());
                kit::title(g.title.clone(), size).max_w(u(880.)).truncate().into_any_element()
            };
            // Every line keeps its height whatever the game (a long name
            // shrinks, a status pill is taller than the text), so the
            // covers above never move when the focus does.
            let row = |h: f32| div().flex_none().h(u(h)).flex().items_center().justify_center();
            let info = div()
                .relative()
                .flex()
                .flex_col()
                .items_center()
                .gap(u(INFO_GAP))
                .child(row(if logo { LOGO_H } else { NAME_H }).child(name))
                .when(theme.hero.facts, |d| d.child(row(FACTS_H).child(self.console_fact_line(&g, Self::console_facts(&g), 15.))))
                .child(row(PLACE_H).child(place(tab, letter, index, n)));
            // From the side the focus came from, never from blank (held
            // directions keep it readable).
            motion::enter_from(SharedString::from(format!("cr-info-{}", g.key)), info, 280, 18. * from, 0., 0.3)
        });
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(pills)
            .child(div().flex_1().min_h_0().flex().flex_col().justify_center().pb(u(INFO_PAD)).child(stage).children(info))
            .into_any_element()
    }

    pub(super) fn carousel_pad(&mut self, tab: usize, spec: &TabSpec, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let theme = look::theme();
        let filter = self.tab_nav(tab).carousel.filter.min(FILTERS.len() - 1);
        let all = self.carousel_keys(spec, filter);
        let n = all.len();
        let (i, pills) = {
            let nav = &self.tab_nav(tab).carousel;
            (nav.index.min(n.saturating_sub(1)), nav.pills)
        };
        // A new filter: the focused game stays when it passes it.
        let next_filter = match pad {
            Pad::Left | Pad::Right if pills => Some((filter as isize + pad.dir()).clamp(0, FILTERS.len() as isize - 1) as usize),
            // With a single tab the bumpers have nothing else to do.
            Pad::LB | Pad::RB if theme.tabs.len() < 2 => Some(super::step(filter, FILTERS.len(), pad.dir(), true)),
            _ => None,
        };
        if let Some(next) = next_filter {
            if next != filter {
                let keep = all.get(i).cloned();
                let at = keep.and_then(|k| self.carousel_keys(spec, next).iter().position(|x| *x == k)).unwrap_or(0);
                self.tab_nav(tab).carousel.set_filter(next, at);
            }
            return true;
        }
        if pills {
            return match pad {
                Pad::Down | Pad::A | Pad::B => {
                    self.tab_nav(tab).carousel.pills = false;
                    true
                }
                _ => false,
            };
        }
        let letters: Vec<char> =
            if matches!(pad, Pad::LT | Pad::RT) { all.iter().map(|k| self.carousel_letter(k)).collect() } else { Vec::new() };
        let nav = &mut self.tab_nav(tab).carousel;
        let to = match pad {
            Pad::Left | Pad::Right => super::step(i, n, pad.dir(), theme.nav.wrap),
            Pad::LT | Pad::RT => {
                let j = jump_letter(&letters, i, pad == Pad::RT);
                if j != i {
                    nav.jumps += 1;
                }
                j
            }
            Pad::Up => {
                nav.pills = true;
                return true;
            }
            Pad::A | Pad::X => {
                if let Some(k) = all.get(i) {
                    self.console_activate(k.clone(), pad, window, cx);
                }
                return true;
            }
            Pad::Y => {
                self.open_console_filters(window, cx);
                return true;
            }
            _ => return false,
        };
        if to != i {
            nav.from = if to > i { 1. } else { -1. };
            nav.index = to;
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
        h.insert(h.len() - self.console_tab_hints().len(), hint(Btn::LT, tr!("Letter")));
        if look::theme().tabs.len() < 2 {
            h.insert(h.len() - 1, hint(Btn::Bumpers, tr!("Filter")));
        }
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
        assert_eq!(pose(0.), Pose { scale: 1., offset: 0., opacity: 1., shade: 0. });
        let (p1, p2) = (pose(1.), pose(2.));
        assert!(p2.scale < p1.scale && p2.opacity < p1.opacity && p2.offset > p1.offset && p2.shade > p1.shade);
        assert_eq!(pose(-2.), Pose { offset: -p2.offset, ..p2 });
        // Halfway between two places is halfway between their poses.
        assert!((pose(0.5).scale - (SCALE[0] + SCALE[1]) / 2.).abs() < 1e-6);
        assert_eq!(pose(9.).opacity, 0.);
    }

    #[test]
    fn covers_keep_their_gap_while_they_turn() {
        // Two neighbours anywhere along the way: the same gap between them.
        for step in 0..40 {
            let d = step as f32 / 10.;
            let (a, b) = (pose(d), pose(d + 1.));
            let gap = (b.offset - b.scale / 2.) - (a.offset + a.scale / 2.);
            assert!((gap - GAP).abs() < 1e-4, "{d}: {gap}");
        }
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
