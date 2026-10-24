//! The cover tile every console layout draws, styled by the theme's
//! `tiles` options: shape (portrait cover, wide art, square crop), corners,
//! how the focus shows (ring, glow, lift, underline), badges, the name under
//! it and greyed uninstalled games.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{Badges, FocusStyle, TileShape, TileTitles};
use heronite_i18n::tr;
use heronite_images::Variant;

use super::super::CollectionView;
use crate::console::kit::{self, u};
use crate::console::look;
use crate::console::motion;
use crate::icons::Icon;
use crate::theme;

/// How much the focused cover grows (each side, share of its size).
const LIFT: f32 = 0.045;

/// Room a tile needs under it for its name (design px), per the theme.
pub(crate) fn label_room() -> f32 {
    if look::theme().tiles.titles == TileTitles::Never {
        0.
    } else {
        26.
    }
}

impl CollectionView {
    /// Design size of a cover tile with the theme's shape and size.
    pub(super) fn console_tile_size(&self) -> (f32, f32) {
        let t = look::theme();
        look::tile_size(t.tiles.shape, t.tiles.size, self.cover_aspect)
    }

    /// Wide art of `key` cropped to `w`×`h` design px (its cover when it has none).
    pub(super) fn console_wide_art(&mut self, key: &SharedString, w: f32, h: f32, priority: i32, window: &Window) -> Option<ImageSource> {
        let s = kit::scale() * window.scale_factor();
        let (pw, ph) = ((w * s).round() as u32, (h * s).round() as u32);
        if self.stage_chains.get(key).is_some_and(|c| !c.is_empty()) {
            let id: SharedString = format!("cw:{key}:{pw}x{ph}").into();
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
        let src = match shape {
            TileShape::Landscape => self.console_wide_art(key, w, h, priority, window),
            _ => self.cover(key, w * kit::scale(), h * kit::scale(), window.scale_factor(), priority),
        };
        let logo = (shape == TileShape::Landscape && theme.tiles.titles == TileTitles::Never)
            .then(|| self.console_logo(key, w * 0.62, h * 0.36, window))
            .flatten();
        let game = self.lib.as_ref().and_then(|l| l.get(key)).cloned();
        let title = game.as_ref().map_or(String::new(), |g| g.title.clone());
        let grey = theme.tiles.grey_uninstalled && game.as_ref().is_some_and(|g| !g.installed);
        let running = game.as_ref().and_then(|g| {
            if self.preparing.contains_key(&g.app_name) {
                Some(tr!("Preparing"))
            } else {
                self.playing.contains_key(&g.app_name).then(|| tr!("Playing"))
            }
        });
        let badge = if theme.tiles.badges == Badges::Off { None } else { running };
        let favourite = theme.tiles.badges == Badges::All && game.as_ref().is_some_and(|g| g.favourite);
        let name_under = match theme.tiles.titles {
            TileTitles::Always => true,
            TileTitles::Focused => focused,
            TileTitles::Never => false,
        };
        let style = theme.tiles.focus;
        let radius = look::tile_radius();
        let ring_colour: Hsla = theme::console_focus().into();
        let wide_title =
            (shape == TileShape::Landscape && theme.tiles.titles == TileTitles::Never && logo.is_none()).then(|| title.clone());
        let (wpx, hpx) = (u(w), u(h));
        let face = move |t: f32| {
            let grow = match style {
                FocusStyle::Lift => 0.07,
                FocusStyle::Underline => 0.,
                _ => LIFT,
            };
            let (gx, gy) = (wpx * (grow * t), hpx * (grow * t));
            let rise = if style == FocusStyle::Lift { u(10.) * t } else { px(0.) };
            let t1 = t.clamp(0., 1.);
            let mut d =
                div().absolute().top(-gy - rise).bottom(-gy + rise).left(-gx).right(-gx).rounded(u(radius)).bg(theme::white_a(0.07));
            d = match style {
                FocusStyle::Ring => d
                    .border_3()
                    .border_color(Hsla { a: t1, ..ring_colour })
                    .when(t > 0.02, |d| d.shadow(kit::shadow(0.6 * t1, 4. + 8. * t, 10. + 20. * t))),
                FocusStyle::Glow => d.border_2().border_color(Hsla { a: t1, ..ring_colour }).when(t > 0.02, |d| {
                    d.shadow(vec![BoxShadow {
                        color: Hsla { a: 0.55 * t1, ..ring_colour },
                        offset: point(px(0.), px(0.)),
                        blur_radius: u(30. * t1),
                        spread_radius: u(2. * t1),
                        inset: false,
                    }])
                }),
                FocusStyle::Lift => d.when(t > 0.02, |d| d.shadow(kit::shadow(0.75 * t1, 10. + 16. * t, 18. + 26. * t))),
                FocusStyle::Underline => d,
            };
            let inner = u((radius - 3.).max(1.));
            d.child(match src.clone() {
                Some(s) => img(s).size_full().object_fit(ObjectFit::Cover).rounded(inner).grayscale(grey).into_any_element(),
                None => div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .p(u(12.))
                    .overflow_hidden()
                    .rounded(inner)
                    .bg(linear_gradient(180., linear_color_stop(theme::white_a(0.02), 0.), linear_color_stop(theme::white_a(0.09), 1.)))
                    .child(
                        div()
                            .w_full()
                            .text_size(u(15.))
                            .line_height(u(19.))
                            .font_weight(theme::W_BOLD)
                            .text_color(theme::text_primary())
                            .line_clamp(4)
                            .text_ellipsis()
                            .child(title.clone()),
                    )
                    .into_any_element(),
            })
            .children(logo.clone().map(|l| {
                div()
                    .absolute()
                    .left(u(12.))
                    .bottom(u(10.))
                    .w(wpx * 0.62)
                    .h(hpx * 0.36)
                    .child(img(l).size_full().object_fit(ObjectFit::Contain))
            }))
            .children(wide_title.clone().map(|name| {
                div()
                    .absolute()
                    .inset_0()
                    .rounded(inner)
                    .bg(linear_gradient(0., linear_color_stop(theme::black_a(0.72), 0.), linear_color_stop(theme::black_a(0.), 0.55)))
                    .flex()
                    .items_end()
                    .p(u(10.))
                    .child(div().w_full().truncate().text_size(u(14.)).font_weight(theme::W_BOLD).text_color(white()).child(name))
            }))
            .when_some(badge, |d, b| d.child(div().absolute().top(u(8.)).left(u(8.)).child(kit::badge(b, theme::console_btn_a()))))
            .when(favourite, |d| {
                d.child(
                    div()
                        .absolute()
                        .top(u(8.))
                        .right(u(8.))
                        .size(u(24.))
                        .rounded_full()
                        .bg(theme::black_a(0.7))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(crate::ui::icon(Icon::Favorite, u(14.), rgb(0xf4d35e))),
                )
            })
        };
        let id = SharedString::from(format!("tile-{key}-{}", shape as u8));
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
                .when(focused, |d| d.font_weight(theme::W_BOLD).text_color(white()))
                .when(!focused, |d| d.text_color(theme::text_muted()))
                .child(title_of(&self.lib, key))
        });
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

fn title_of(lib: &Option<std::sync::Arc<heronite_data::Library>>, key: &SharedString) -> String {
    lib.as_ref().and_then(|l| l.get(key)).map_or(String::new(), |g| g.title.clone())
}
