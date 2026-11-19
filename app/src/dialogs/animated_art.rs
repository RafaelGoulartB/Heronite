//! Animated backgrounds in the image pickers (Covers' Background slot, Game
//! settings → Search images for a background): the mark on a picture that
//! moves, and the line that says backgrounds can move, with "Turn on" while
//! Settings → Appearance → "Animated backgrounds" is off.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::AppPaths;
use heronite_i18n::tr;

use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// Whether backgrounds that move play (else they show their first frame).
pub fn on() -> bool {
    theme::appearance().animated_backgrounds
}

/// Turns "Animated backgrounds" on, as its switch in Settings does.
pub fn turn_on(app: &AppPaths, cx: &mut App) {
    let mut a = theme::appearance();
    a.animated_backgrounds = true;
    theme::set_appearance(a);
    let (a, app) = (theme::appearance(), app.clone());
    cx.background_spawn(async move {
        if let Err(err) = a.save(&app) {
            log::warn!("saving appearance.json: {err:#}");
        }
    })
    .detach();
    cx.refresh_windows();
}

/// "Animated" over the corner of a picture that moves.
pub fn badge() -> Div {
    div()
        .absolute()
        .top(px(6.))
        .left(px(6.))
        .flex()
        .items_center()
        .gap(px(4.))
        .px(px(6.))
        .py(px(2.))
        .rounded_full()
        .bg(gpui_kit::black().opacity(0.62))
        .text_size(px(11.))
        .text_color(gpui_kit::white())
        .child(icon(Icon::Animation, px(12.), gpui_kit::white()))
        .child(tr!("Animated"))
}

/// A picture with [`badge`] over it when `animated`.
pub fn marked(picture: impl IntoElement, animated: bool) -> Div {
    div().relative().child(picture).when(animated, |d| d.child(badge()))
}

/// The line over a background picker's pictures: what an animated one does
/// and, while they are off, "Turn on". `any` says some of the pictures
/// listed move.
pub fn hint<V: 'static>(id: &str, any: bool, cx: &mut Context<V>, turn_on: impl Fn(&mut V, &mut Context<V>) + 'static) -> Div {
    let on = on();
    let text = match (on, any) {
        (true, _) => tr!("Pictures marked Animated move behind the Collection. A video file of your own can too."),
        (false, true) => tr!(
            "Backgrounds can move: those marked Animated, or a video file of your own. Animated backgrounds are off, so they show still."
        ),
        (false, false) => {
            tr!("Backgrounds can move: pick a video file of your own, or an animated one when there is one. Animated backgrounds are off.")
        }
    };
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(10.))
        .py(px(6.))
        .rounded(px(theme::R_INPUT))
        .bg(theme::ink_a(0.04))
        .child(icon(Icon::Animation, px(16.), theme::accent()))
        .child(div().flex_1().min_w_0().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_muted()).child(text))
        .when(!on, |d| {
            d.child(
                div()
                    .id(SharedString::from(format!("{id}-turn-on")))
                    .flex_none()
                    .px(px(10.))
                    .py(px(4.))
                    .rounded_full()
                    .border_1()
                    .border_color(theme::accent_tint_border())
                    .bg(theme::accent_tint())
                    .text_size(px(12.))
                    .text_color(theme::text_primary())
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::ink_a(0.08)))
                    .child(tr!("Turn on"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        turn_on(this, cx);
                        cx.notify();
                    })),
            )
        })
}
