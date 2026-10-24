//! The cover tile every console layout draws, styled by the theme's
//! `tiles` options: shape (portrait cover, wide art, square crop), corners,
//! how the focus shows (ring, glow, lift, underline), badges, the name under
//! it and greyed uninstalled games.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{Badges, FocusStyle, TileShape, TileTitles};
use heronite_i18n::tr;
use heronite_images::Variant;

use super::super::data::parse_day;
use super::super::focus_left::STAR;
use super::super::CollectionView;
use crate::console::kit::{self, u};
use crate::console::look;
use crate::console::motion;
use crate::icons::Icon;
use crate::theme;

/// How much the focused cover grows (each side, share of its size).
const LIFT: f32 = 0.045;

/// Height of the store strip above the art, and of the name bar under it (design px).
const HEADER_H: f32 = 22.;
const BAR_H: f32 = 28.;

/// Games added within this many days wear the "New" badge.
const NEW_DAYS: i64 = 14;

/// Room a tile needs under it for its name (design px), per the theme. The
/// name bar is part of the tile, so it needs none.
pub(crate) fn label_room() -> f32 {
    if matches!(look::theme().tiles.titles, TileTitles::Never | TileTitles::Bar) {
        0.
    } else {
        26.
    }
}

/// Whether a game added on `added` ("YYYY-MM-DD...") is still new.
fn is_new(added: &str) -> bool {
    let Some(day) = parse_day(added) else { return false };
    let age = chrono::Local::now().date_naive().signed_duration_since(day).num_days();
    (0..=NEW_DAYS).contains(&age)
}

/// `e` with its top and/or bottom corners rounded by `r`.
fn round<E: Styled>(e: E, top: bool, bottom: bool, r: Pixels) -> E {
    let e = if top { e.rounded_tl(r).rounded_tr(r) } else { e };
    if bottom {
        e.rounded_bl(r).rounded_br(r)
    } else {
        e
    }
}

impl CollectionView {
    /// Design size of a cover tile with the theme's shape and size.
    pub(super) fn console_tile_size(&self) -> (f32, f32) {
        let t = look::theme();
        look::tile_size(t.tiles.shape, t.tiles.size, self.cover_aspect)
    }

    /// Wide art of `key` cropped to `w`×`h` design px (its cover when it has
    /// none or it fails to load).
    pub(super) fn console_wide_art(&mut self, key: &SharedString, w: f32, h: f32, priority: i32, window: &Window) -> Option<ImageSource> {
        let s = kit::scale() * window.scale_factor();
        let (pw, ph) = ((w * s).round() as u32, (h * s).round() as u32);
        let id: SharedString = format!("cw:{key}:{pw}x{ph}").into();
        if self.stage_chains.get(key).is_some_and(|c| !c.is_empty()) && !self.images.is_failed(&id) {
            let chains = &self.stage_chains;
            return self
                .images
                .get_or_request(&id, || chains.get(key).cloned().unwrap_or_default(), pw, ph, Variant::Cover, priority)
                .map(ImageSource::Render);
        }
        self.cover(key, w * kit::scale(), h * kit::scale(), window.scale_factor(), priority)
    }

    /// A cover tile of the theme's shape. `load`: it is on screen, so its
    /// image is requested (off screen it is an empty box of the same size).
    pub(super) fn console_tile(&mut self, key: &SharedString, w: f32, h: f32, focused: bool, load: bool, window: &Window) -> Div {
        self.console_tile_shaped(key, look::theme().tiles.shape, w, h, focused, load, window)
    }

    pub(super) fn console_tile_shaped(
        &mut self,
        key: &SharedString,
        shape: TileShape,
        w: f32,
        h: f32,
        focused: bool,
        load: bool,
        window: &Window,
    ) -> Div {
        if !load && !focused {
            return div().flex_none().w(u(w)).h(u(h));
        }
        let theme = look::theme();
        let priority = if focused { 200 } else { 50 };
        let (has_header, has_bar) = (theme.tiles.header, theme.tiles.titles == TileTitles::Bar);
        // The strip and the bar are part of the tile: the art takes what is left.
        let art_h = h - if has_header { HEADER_H } else { 0. } - if has_bar { BAR_H } else { 0. };
        let src = match shape {
            TileShape::Landscape => self.console_wide_art(key, w, art_h, priority, window),
            _ => self.cover(key, w * kit::scale(), art_h * kit::scale(), window.scale_factor(), priority),
        };
        let landscape_art = shape == TileShape::Landscape && theme.tiles.titles == TileTitles::Never;
        let logo = landscape_art.then(|| self.console_logo(key, w * 0.62, art_h * 0.36, true, window)).flatten();
        let game = self.lib.as_ref().and_then(|l| l.get(key)).cloned();
        let title = game.as_ref().map_or(String::new(), |g| g.title.clone());
        let missing = game.as_ref().is_some_and(|g| !g.installed);
        let running = game.as_ref().and_then(|g| {
            if self.preparing.contains_key(&g.app_name) {
                Some(tr!("Preparing"))
            } else {
                self.playing.contains_key(&g.app_name).then(|| tr!("Playing"))
            }
        });
        let all_badges = theme.tiles.badges == Badges::All;
        let style = theme.tiles.focus;
        let (wpx, hpx) = (u(w), u(h));
        let art = TileArt {
            // Over the art only: without one the tile shows the name itself.
            wide_title: (landscape_art && logo.is_none() && src.is_some()).then(|| title.clone()),
            src,
            logo,
            grey: theme.tiles.grey_uninstalled && missing,
            badge: if theme.tiles.badges == Badges::Off { None } else { running },
            favourite: all_badges && game.as_ref().is_some_and(|g| g.favourite),
            new: all_badges && game.as_ref().and_then(|g| self.game_extras.added.get(&g.key)).is_some_and(|d| is_new(d)),
            title: title.clone(),
            top_r: !has_header,
            bottom_r: !has_bar,
            width: wpx,
            art_h,
        };
        let store = has_header.then(|| game.as_ref().map_or(String::new(), |g| g.source_label.to_uppercase()));
        let name_under = match theme.tiles.titles {
            TileTitles::Always => true,
            TileTitles::Focused => focused,
            TileTitles::Never | TileTitles::Bar => false,
        };
        let radius = look::tile_radius();
        let face = move |t: f32| {
            let inner = u((radius - 3.).max(1.));
            // The strip and the bar grow with the tile.
            let scale = 1. + 2. * focus_grow(style) * t;
            let header = store.clone().map(|name| store_strip(name, scale, inner));
            let bar = has_bar.then(|| name_bar(&art.title, missing, scale, inner));
            focus_chrome(div().absolute(), style, t, wpx, hpx)
                .bg(kit::ink(0.07))
                .flex()
                .flex_col()
                .children(header)
                .child(art.view(inner))
                .children(bar)
        };
        let underline = (style == FocusStyle::Underline && focused)
            .then(|| div().absolute().left(wpx * 0.18).right(wpx * 0.18).bottom(-u(9.)).h(u(4.)).rounded_full().bg(theme::console_focus()));
        let label = name_under.then(|| {
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(hpx + u(if focused { 12. } else { 8. }))
                .truncate()
                .text_size(u(14.))
                .when(focused, |d| d.font_weight(theme::W_BOLD).text_color(theme::text_primary()))
                .when(!focused, |d| d.text_color(theme::text_muted()))
                .child(title)
        });
        // A game in a row and the same one in another never share an animation.
        let id = (key.clone(), shape as usize);
        div()
            .flex_none()
            .relative()
            .w(wpx)
            .h(hpx)
            .child(motion::spring(id, div().absolute().inset_0(), motion::LIFT, focused && style != FocusStyle::Underline, move |d, t| {
                d.child(face(t))
            }))
            .children(underline)
            .children(label)
    }
}

/// What the art of a tile shows.
#[derive(Clone)]
struct TileArt {
    src: Option<ImageSource>,
    logo: Option<ImageSource>,
    /// The name over the art (a wide tile without a logo).
    wide_title: Option<String>,
    grey: bool,
    /// "Preparing" / "Playing".
    badge: Option<&'static str>,
    favourite: bool,
    new: bool,
    title: String,
    /// Round the art's top / bottom corners (no strip / bar on that side).
    top_r: bool,
    bottom_r: bool,
    width: Pixels,
    /// Design px.
    art_h: f32,
}

impl TileArt {
    /// The art with its badges, its corners `inner` rounded.
    fn view(&self, inner: Pixels) -> Div {
        let (top_r, bottom_r) = (self.top_r, self.bottom_r);
        let picture = match self.src.clone() {
            Some(s) => {
                round(img(s).size_full().object_fit(ObjectFit::Cover).grayscale(self.grey), top_r, bottom_r, inner).into_any_element()
            }
            // Without a picture the tile shows the name itself.
            None => round(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .p(u(12.))
                    .overflow_hidden()
                    .bg(linear_gradient(180., linear_color_stop(kit::ink(0.02), 0.), linear_color_stop(kit::ink(0.09), 1.)))
                    .child(
                        div()
                            .w_full()
                            .text_size(u(15.))
                            .line_height(u(19.))
                            .font_weight(theme::W_BOLD)
                            .text_color(theme::text_primary())
                            .line_clamp(4)
                            .text_ellipsis()
                            .child(self.title.clone()),
                    ),
                top_r,
                bottom_r,
                inner,
            )
            .into_any_element(),
        };
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(picture)
            .children(self.logo.clone().map(|l| {
                div()
                    .absolute()
                    .left(u(12.))
                    .bottom(u(10.))
                    .w(self.width * 0.62)
                    .h(u(self.art_h * 0.36))
                    .child(img(l).size_full().object_fit(ObjectFit::Contain))
            }))
            .children(self.wide_title.clone().map(|name| {
                round(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(linear_gradient(0., linear_color_stop(theme::black_a(0.72), 0.), linear_color_stop(theme::black_a(0.), 0.55)))
                        .flex()
                        .items_end()
                        .p(u(10.))
                        .child(div().w_full().truncate().text_size(u(14.)).font_weight(theme::W_BOLD).text_color(white()).child(name)),
                    top_r,
                    bottom_r,
                    inner,
                )
            }))
            .when_some(self.badge, |d, b| d.child(div().absolute().top(u(8.)).left(u(8.)).child(kit::badge(b, theme::console_btn_a()))))
            .when(self.new, |d| d.child(new_badge(if top_r { inner } else { px(0.) })))
            .when(self.favourite, |d| d.child(favourite_badge(self.new)))
    }
}

/// "New", flush with the art's top-right corner (`corner` rounds it).
fn new_badge(corner: Pixels) -> Div {
    div()
        .absolute()
        .top_0()
        .right_0()
        .px(u(8.))
        .py(u(3.))
        .rounded_tr(corner)
        .rounded_bl(u(6.))
        .bg(rgb(0xd6249f))
        .text_size(u(11.))
        .font_weight(theme::W_BOLD)
        .text_color(white())
        .child(tr!("New"))
}

/// The heart on a favourite, under the "New" badge when there is one.
fn favourite_badge(under_new: bool) -> Div {
    div()
        .absolute()
        .top(u(if under_new { 30. } else { 8. }))
        .right(u(8.))
        .size(u(24.))
        .rounded_full()
        .bg(theme::black_a(0.7))
        .flex()
        .items_center()
        .justify_center()
        .child(crate::ui::icon(Icon::Favorite, u(14.), rgb(STAR)))
}

/// A strip across the tile `h` design px high (before it grows by `scale`).
fn strip(h: f32, scale: f32) -> Div {
    div().flex_none().w_full().h(u(h) * scale).flex().items_center().justify_center().bg(theme::console_sidebar())
}

/// The store above the art (`Tiles::header`).
fn store_strip(name: String, scale: f32, inner: Pixels) -> Div {
    round(strip(HEADER_H, scale), true, false, inner)
        .px(u(6.))
        .text_size(u(11.))
        .font_weight(theme::W_BOLD)
        .text_color(theme::text_primary())
        .child(div().truncate().child(name))
}

/// The name under the art (`TileTitles::Bar`); a download mark when the game is not installed.
fn name_bar(title: &str, missing: bool, scale: f32, inner: Pixels) -> Div {
    round(strip(BAR_H, scale), false, true, inner)
        .relative()
        .px(u(if missing { 24. } else { 8. }))
        .text_size(u(13.))
        .font_weight(theme::W_BOLD)
        .text_color(theme::text_primary())
        .child(div().truncate().child(title.to_string()))
        .when(missing, |d| d.child(div().absolute().right(u(7.)).child(crate::ui::icon(Icon::Download, u(14.), theme::text_muted()))))
}

/// How much a focused tile grows (each side, share of its size) in `style`.
fn focus_grow(style: FocusStyle) -> f32 {
    match style {
        FocusStyle::Lift => 0.07,
        FocusStyle::Underline => 0.,
        _ => LIFT,
    }
}

/// The theme's focus mark on `d` at `t` (0 = idle, 1 = focused; more while
/// it springs past): `d` grows out of its `w`×`h` box and gets the ring,
/// the glow or the lift's shadow. `d` must be `absolute`.
fn focus_chrome(d: Div, style: FocusStyle, t: f32, w: Pixels, h: Pixels) -> Div {
    let ring: Hsla = theme::console_focus().into();
    let grow = focus_grow(style);
    let (gx, gy) = (w * (grow * t), h * (grow * t));
    let rise = if style == FocusStyle::Lift { u(10.) * t } else { px(0.) };
    let t1 = t.clamp(0., 1.);
    let d = d.top(-gy - rise).bottom(-gy + rise).left(-gx).right(-gx).rounded(u(look::tile_radius()));
    match style {
        FocusStyle::Ring => d
            .border_3()
            .border_color(Hsla { a: t1, ..ring })
            .when(t > 0.02, |d| d.shadow(kit::shadow(0.6 * t1, 4. + 8. * t, 10. + 20. * t))),
        FocusStyle::Glow => d.border_2().border_color(Hsla { a: t1, ..ring }).when(t > 0.02, |d| {
            d.shadow(vec![BoxShadow {
                color: Hsla { a: 0.55 * t1, ..ring },
                offset: point(px(0.), px(0.)),
                blur_radius: u(30. * t1),
                spread_radius: u(2. * t1),
                inset: false,
            }])
        }),
        FocusStyle::Lift => d.when(t > 0.02, |d| d.shadow(kit::shadow(0.75 * t1, 10. + 16. * t, 18. + 26. * t))),
        FocusStyle::Underline => d,
    }
}

/// A tile body that grows and gets the theme's focus mark when focused.
/// Goes inside a box of `w`×`h` design px (it fills it).
pub(super) fn focus_frame(id: impl Into<ElementId>, focused: bool, w: f32, h: f32, content: Div) -> AnyElement {
    let style = look::theme().tiles.focus;
    let (wpx, hpx) = (u(w), u(h));
    motion::spring(id, div().absolute().inset_0(), motion::LIFT, focused, move |d, t| {
        let bar = (style == FocusStyle::Underline).then(|| {
            div()
                .absolute()
                .left(wpx * 0.18)
                .right(wpx * 0.18)
                .bottom(u(8.))
                .h(u(4.))
                .rounded_full()
                .bg(theme::console_focus())
                .opacity(t.clamp(0., 1.))
        });
        focus_chrome(d, style, t, wpx, hpx).child(content).children(bar)
    })
}
