//! Wheel: a vertical list of the tab's games on the left, EmulationStation
//! style. The focused row stays centred and the others fade and bend away
//! with their distance; the rows are placed from the list's scroll position,
//! so a [`Glide`] moves them all smoothly. ◀ ▶ switch the filter in the
//! header (All games, each platform, Favourites). On the right: the focused
//! game's art with its logo over it, its facts and overview.

use std::collections::BTreeMap;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{TabSpec, WheelItems};
use heronite_data::organize::{release_year, Organize};
use heronite_data::CollectionGame;
use heronite_i18n::{tr, trn};

use super::super::super::CollectionView;
use super::super::backdrop::Scrim;
use super::super::hero::clip;
use super::super::sources::{by_title, keys};
use super::grid::fact_cell;
use crate::console::kit::{self, u};
use crate::console::look;
use crate::console::motion::{self, Axis, Glide, Goal};
use crate::console::{hint, Btn, Hint, Pad};
use crate::icons::Icon;
use crate::theme;

pub(super) const SCRIM: Scrim = Scrim::Full;

const SIDE: f32 = 46.;
/// Distance between two rows, design px.
const ROW: f32 = 62.;
/// Height of a row's pill (the rest is the gap).
const ROW_H: f32 = 54.;
/// Height of the header with the filter.
const HEAD: f32 = 46.;
/// Rows drawn on each side of the centre (further ones are invisible).
const REACH: isize = 7;
/// LT / RT jump this many games.
const JUMP: usize = 10;
/// Moves longer than this many rows jump instead of gliding past every cover.
const GLIDE_MAX: usize = 12;

pub(crate) struct Nav {
    /// Index in the filters (see [`CollectionView::wheel_data`]).
    filter: usize,
    index: usize,
    scroll: Glide,
    /// The filter changed: back to the top at once.
    restart: bool,
}

impl Default for Nav {
    fn default() -> Self {
        Self { filter: 0, index: 0, scroll: Glide::new(Axis::Y), restart: false }
    }
}

#[derive(Clone, PartialEq)]
enum Filter {
    All,
    Platform(String),
    Favourites,
}

/// What the wheel shows now: the filters, the active one and its games.
struct Data {
    filters: Vec<(Filter, SharedString, usize)>,
    /// Index of the active filter.
    active: usize,
    keys: Vec<SharedString>,
}

/// Opacity of a row `d` rows from the centre.
fn fade(d: f32) -> f32 {
    (1. - d.abs() * 0.16).max(0.)
}

/// How far (design px) a row `d` rows from the centre bends to the right.
fn bend(d: f32) -> f32 {
    d * d * 2.4
}

impl CollectionView {
    fn organize_for_wheel(&self) -> Organize<'_> {
        Organize { extras: Some(&self.game_extras), ..Default::default() }
    }

    /// The filters over the tab's games: All, each platform present (most
    /// games first; one that holds every game adds nothing) and Favourites,
    /// then the games of filter `want` sorted A–Z.
    fn wheel_data(&self, spec: &TabSpec, want: usize) -> Data {
        let games = self.console_games(&spec.source);
        let org = self.organize_for_wheel();
        let mut platforms: BTreeMap<String, usize> = BTreeMap::new();
        for g in &games {
            *platforms.entry(org.platform(g)).or_default() += 1;
        }
        let mut platforms: Vec<(String, usize)> = platforms.into_iter().filter(|(_, n)| *n < games.len()).collect();
        platforms.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let favourites = games.iter().filter(|g| g.favourite).count();
        let mut filters = vec![(Filter::All, SharedString::from(tr!("All games")), games.len())];
        filters.extend(platforms.into_iter().map(|(p, n)| (Filter::Platform(p.clone()), SharedString::from(p), n)));
        if favourites > 0 && favourites < games.len() {
            filters.push((Filter::Favourites, tr!("Favourites").into(), favourites));
        }
        let active = want.min(filters.len() - 1);
        let picked: Vec<&CollectionGame> = games
            .iter()
            .map(|g| g.as_ref())
            .filter(|g| match &filters[active].0 {
                Filter::All => true,
                Filter::Platform(p) => org.platform(g) == *p,
                Filter::Favourites => g.favourite,
            })
            .collect();
        Data { keys: keys(by_title(picked)), filters, active }
    }

    /// The small tag at the end of a row: the emulated system, else the store.
    fn wheel_tag(&self, g: &CollectionGame) -> String {
        if self.game_extras.systems.contains_key(&g.key) {
            self.organize_for_wheel().platform(g)
        } else {
            g.source_label.to_string()
        }
    }

    pub(super) fn render_wheel(&mut self, tab: usize, spec: &TabSpec, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.lib.is_none() {
            return kit::empty(Icon::GridView, tr!("Loading your library\u{2026}"), "").into_any_element();
        }
        let want = self.tab_nav(tab).wheel.filter;
        let data = self.wheel_data(spec, want);
        let n = data.keys.len();
        if n == 0 {
            return kit::empty(Icon::GridView, tr!("No games yet"), tr!("Add a game from the menu (\u{2630}).")).into_any_element();
        }
        let nav = &mut self.tab_nav(tab).wheel;
        nav.filter = data.active;
        nav.index = nav.index.min(n - 1);
        if std::mem::take(&mut nav.restart) {
            nav.scroll.reset();
        }
        let (index, glide) = (nav.index, nav.scroll.clone());
        let focused = data.keys[index].clone();
        self.console_focus_game(Some(focused.clone()), cx);

        let scale = kit::scale();
        let (vw, vh) = (f32::from(window.viewport_size().width) / scale, f32::from(window.viewport_size().height) / scale);
        let list_w = (vw * 0.37).round().min(470.);
        let (_, label, count) = &data.filters[data.active];
        let head = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(u(12.))
            .h(u(HEAD))
            .px(u(SIDE))
            .child(crate::ui::icon(Icon::ChevronLeft, u(22.), theme::text_muted()))
            .child(kit::title(label.clone(), 22.))
            .child(crate::ui::icon(Icon::ChevronRight, u(22.), theme::text_muted()))
            .child(div().text_size(u(15.)).text_color(theme::text_muted()).child(trn!(*count, "{n} game", "{n} games")));

        let handle = glide.handle().clone();
        let row_px = f32::from(u(ROW));
        // Before the first layout the list's height is a guess.
        let view_px = match f32::from(handle.bounds().size.height) {
            h if h > 1. => h,
            _ => (vh - 76. - HEAD) * scale,
        };
        let pos = -f32::from(handle.offset().y) / row_px;
        let mid = (view_px - row_px) / 2.;
        let centre = pos.round() as isize;
        let (first, last) = ((centre - REACH).max(0) as usize, (centre + REACH).clamp(0, n as isize - 1) as usize);
        // Tall enough to scroll the last game to the middle.
        let mut rows = div().relative().w_full().h(px(n as f32 * row_px + view_px));
        for i in first..=last {
            rows = rows.children(self.wheel_row(&data.keys[i], i, i as f32 - pos, i == index, list_w, mid, window));
        }
        let list =
            div().id(("console-wheel", tab)).flex_1().min_h_0().overflow_y_scroll().track_scroll(&handle).child(rows).child(glide.driver());

        let art_l = SIDE + list_w + 40.;
        let panel = self.lib.as_ref().and_then(|l| l.get(&focused)).cloned().map(|g| {
            let panel_w = vw - art_l - SIDE;
            self.wheel_panel(&g, panel_w, vh, window)
        });
        div()
            .flex()
            .size_full()
            .child(div().flex_none().flex().flex_col().w(u(SIDE + list_w + 40.)).h_full().child(head).child(list))
            .child(div().flex_1().min_w_0().h_full().pt(u(HEAD + 6.)).pr(u(SIDE)).children(panel))
            .into_any_element()
    }

    /// One row of the list, `d` rows from the centre.
    fn wheel_row(&mut self, key: &SharedString, i: usize, d: f32, focused: bool, list_w: f32, mid: f32, window: &Window) -> Option<Div> {
        let alpha = fade(d);
        if alpha <= 0.01 && !focused {
            return None;
        }
        let g = self.lib.as_ref()?.get(key)?.clone();
        let tag = self.wheel_tag(&g);
        let logo =
            (look::theme().layouts.wheel_items == WheelItems::Logo).then(|| self.console_logo(key, 200., 34., focused, window)).flatten();
        let what = match logo {
            Some(src) => div()
                .flex_1()
                .min_w_0()
                .h(u(34.))
                .flex()
                .items_center()
                .child(div().w(u(200.)).h(u(34.)).child(img(src).size_full().object_fit(ObjectFit::Contain)))
                .into_any_element(),
            None => {
                let cover = self.cover(key, 30. * kit::scale(), 42. * kit::scale(), window.scale_factor(), if focused { 120 } else { 40 });
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(u(14.))
                    .child(
                        div()
                            .flex_none()
                            .w(u(30.))
                            .h(u(42.))
                            .rounded(u(4.))
                            .overflow_hidden()
                            .bg(kit::ink(0.1))
                            .children(cover.map(|s| img(s).size_full().object_fit(ObjectFit::Cover))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(u(19.))
                            .when(focused, |d| d.font_weight(theme::W_BOLD))
                            .child(g.title.clone()),
                    )
                    .into_any_element()
            }
        };
        let pill = kit::row_base(focused).h(u(ROW_H)).gap(u(14.)).child(what).child(
            div()
                .flex_none()
                .text_size(u(12.))
                .text_color(if focused {
                    if kit::light() {
                        Hsla { a: 0.65, ..theme::console_on_focus().into() }
                    } else {
                        theme::black_a(0.55)
                    }
                } else {
                    theme::text_dim().into()
                })
                .child(tag),
        );
        Some(
            div()
                .absolute()
                .top(px(mid + i as f32 * f32::from(u(ROW))))
                .left(u(SIDE - 22. + bend(d)))
                .w(u(list_w))
                .h(u(ROW))
                .flex()
                .items_center()
                .opacity(alpha)
                .child(div().w_full().child(pill)),
        )
    }

    /// The focused game on the right: art in a rounded frame with its logo
    /// over it, six facts and the overview.
    fn wheel_panel(&mut self, g: &CollectionGame, w: f32, vh: f32, window: &Window) -> AnyElement {
        let theme = look::theme();
        let key = SharedString::from(g.key.clone());
        let art_h = (w * 9. / 16.).min(vh - 76. - HEAD - 230.).max(120.).round();
        let art = self.console_wide_art(&key, w, art_h, 200, window);
        let logo = self.console_logo(&key, w * 0.55, art_h * 0.34, true, window);
        let no_logo = logo.is_none();
        let dash = || "\u{2014}".to_string();
        let status = self
            .lib
            .as_ref()
            .and_then(|l| l.statuses.iter().find(|s| s.id == g.status_id))
            .map(|s| (heronite_data::status::status_label(&s.name).to_string(), theme::status_color(&s.slug)));
        let value = |text: String| div().truncate().text_size(u(15.)).text_color(theme::text_primary()).child(text);
        let row = || div().flex().gap(u(18.));
        let facts = div()
            .flex()
            .flex_col()
            .gap(u(12.))
            .mt(u(16.))
            .child(
                row()
                    .child(fact_cell(tr!("Release year"), value(release_year(g).map_or_else(dash, |y| y.to_string()))))
                    .child(fact_cell(tr!("Genre"), value(g.details.genres.first().cloned().unwrap_or_else(dash))))
                    .child(fact_cell(tr!("Rating"), value(g.details.critic_score.map_or_else(dash, |s| format!("{s:.0}"))))),
            )
            .child(
                row()
                    .child(fact_cell(
                        tr!("Playtime"),
                        value(if g.playtime_minutes > 0 {
                            heronite_data::collection::format_playtime_minutes(g.playtime_minutes)
                        } else {
                            dash()
                        }),
                    ))
                    .child(fact_cell(
                        tr!("Status"),
                        match status {
                            Some((label, colour)) => value(label).text_color(kit::tint(colour)),
                            None => value(dash()),
                        },
                    ))
                    .child(fact_cell(tr!("Developer"), value(g.details.developers.first().cloned().unwrap_or_else(dash)))),
            );
        let overview = theme.hero.overview.then(|| self.console_overview(&g.key)).flatten();
        let frame = div()
            .relative()
            .flex_none()
            .w(u(w))
            .h(u(art_h))
            .rounded(u(16.))
            .overflow_hidden()
            .border_1()
            .border_color(kit::ink(0.08))
            .bg(kit::ink(0.07))
            .shadow(kit::shadow(0.55, 18., 44.))
            .children(art.map(|s| img(s).size_full().object_fit(ObjectFit::Cover)))
            .child(div().absolute().inset_0().bg(linear_gradient(
                0.,
                linear_color_stop(theme::black_a(0.55), 0.),
                linear_color_stop(theme::black_a(0.), 0.45),
            )))
            .children(logo.map(|l| {
                div()
                    .absolute()
                    .left(u(20.))
                    .bottom(u(16.))
                    .w(u(w * 0.55))
                    .h(u(art_h * 0.34))
                    .child(img(l).size_full().object_fit(ObjectFit::Contain))
            }));
        let panel = div()
            .relative()
            .w(u(w))
            .flex()
            .flex_col()
            .child(frame)
            .when(no_logo, |d| d.child(kit::title(g.title.clone(), 26.).mt(u(14.))))
            .child(facts)
            .children(overview.map(|o| {
                motion::fade_in(
                    SharedString::from(format!("wheel-about-{}", g.key)),
                    kit::body_text(clip(&o, 240)).mt(u(10.)).line_clamp(4),
                    260,
                )
            }));
        motion::enter_from(SharedString::from(format!("wheel-panel-{}", g.key)), panel, 280, 0., 10., 0.3)
    }

    pub(super) fn wheel_pad(&mut self, tab: usize, spec: &TabSpec, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let want = self.tab_nav(tab).wheel.filter;
        let data = self.wheel_data(spec, want);
        let n = data.keys.len();
        if n == 0 {
            return false;
        }
        let nav = &mut self.tab_nav(tab).wheel;
        let i = nav.index.min(n - 1);
        let to = match pad {
            Pad::Left | Pad::Right => {
                let delta = if pad == Pad::Right { 1 } else { -1 };
                nav.filter = super::step(data.active, data.filters.len(), delta, true);
                nav.index = 0;
                nav.restart = true;
                return true;
            }
            Pad::Up => super::step(i, n, -1, true),
            Pad::Down => super::step(i, n, 1, true),
            Pad::LT => i.saturating_sub(JUMP),
            Pad::RT => (i + JUMP).min(n - 1),
            Pad::A | Pad::X => {
                self.console_activate(data.keys[i].clone(), pad, window, cx);
                return true;
            }
            Pad::Y => {
                self.console_search(window, cx);
                return true;
            }
            _ => return false,
        };
        nav.index = to;
        let goal = Goal::Offset(to as f32 * f32::from(u(ROW)));
        if to.abs_diff(i) > GLIDE_MAX {
            nav.scroll.jump(goal);
        } else {
            nav.scroll.go(goal);
        }
        true
    }

    pub(super) fn wheel_hints(&self, _tab: usize, _spec: &TabSpec) -> Vec<Hint> {
        let mut h = self.console_cover_hints();
        h.insert(0, hint(Btn::LeftRight, tr!("Filter")));
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` above brings gpui's own `test` macro.
    use core::prelude::v1::test;

    #[test]
    fn rows_fade_and_bend_with_distance() {
        assert_eq!(fade(0.), 1.);
        assert!(fade(2.) < fade(1.));
        assert_eq!(fade(-3.), fade(3.));
        assert_eq!(fade(7.), 0.);
        assert_eq!(bend(0.), 0.);
        assert_eq!(bend(-2.), bend(2.));
    }
}
