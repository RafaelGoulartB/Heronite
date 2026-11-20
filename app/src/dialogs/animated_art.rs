//! Animated backgrounds in the image pickers (Covers' Background slot, Game
//! settings → Search images for a background): the mark on a picture that
//! moves, and, once one is picked while Settings → Appearance → "Animated
//! backgrounds" is off, the line that says it will show still, with "Turn on".

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::AppPaths;
use heronite_i18n::tr;
use heronite_images::ImageRef;

use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// Whether backgrounds that move play (else they show their first frame).
pub fn on() -> bool {
    theme::appearance().animated_backgrounds
}

/// Whether a background picture moves (a video, an animated GIF or PNG),
/// looked at off the UI thread once per picture: the background picked
/// before, shown in a picker, says it will show still too.
#[derive(Default)]
pub struct Moves {
    known: Option<(String, bool)>,
    asking: Option<String>,
}

impl Moves {
    /// Whether the picture of `chain` moves; `false` until known (the view
    /// is drawn again then). `field` is where the view keeps this.
    pub fn of<V: 'static>(&mut self, chain: &[ImageRef], cx: &mut Context<V>, field: fn(&mut V) -> &mut Moves) -> bool {
        let id = format!("{:?}", chain.first());
        if let Some((known, moves)) = &self.known {
            if *known == id {
                return *moves;
            }
        }
        if self.asking.as_deref() != Some(id.as_str()) {
            self.asking = Some(id.clone());
            let chain = chain.to_vec();
            cx.spawn(async move |this, cx| {
                let moves = cx.background_spawn(async move { heronite_images::animation::moving_file(&chain).is_some() }).await;
                this.update(cx, |this, cx| {
                    let m = field(this);
                    m.asking = None;
                    m.known = Some((id, moves));
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        false
    }
}

/// Turns "Animated backgrounds" on, as its switch in Settings does.
pub fn turn_on(app: &AppPaths, cx: &mut App) {
    set(app, true, cx);
}

/// Turns "Animated backgrounds" on or off and saves it, as its switch in
/// Settings does.
pub fn set(app: &AppPaths, on: bool, cx: &mut App) {
    let mut a = theme::appearance();
    a.animated_backgrounds = on;
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

/// The line under a background picker's pictures while the picture picked
/// moves (`picked_animated`) and Animated backgrounds is off: it will show
/// still, with "Turn on". `None` otherwise: the marks on the pictures say
/// enough.
pub fn hint<V: 'static>(
    id: &str,
    picked_animated: bool,
    cx: &mut Context<V>,
    turn_on: impl Fn(&mut V, &mut Context<V>) + 'static,
) -> Option<Div> {
    if !picked_animated || on() {
        return None;
    }
    Some(
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(10.))
            .py(px(6.))
            .rounded(px(theme::R_INPUT))
            .bg(theme::ink_a(0.04))
            .child(icon(Icon::Animation, px(16.), theme::accent()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(12.))
                    .line_height(px(16.))
                    .text_color(theme::text_muted())
                    .child(tr!("This background moves, but Animated backgrounds is off: it will show still.")),
            )
            .child(
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
            ),
    )
}
