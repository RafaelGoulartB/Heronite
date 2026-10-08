//! Heroic's `Dialog` (`components/UI/Dialog`): a modal card on
//! `--modal-background`, radius 10, 1px `--modal-border`, opening with
//! opacity + translateY(50px→0) over 500ms, over a `--modal-backdrop` veil.
//! Header: title (`--text-xl` ≈ 23px) with a close button top-right.
//! Content padding: 0 32px 24px; header padding: 24px 16px 0 32px.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

pub struct DialogProps {
    pub id: SharedString,
    pub title: SharedString,
    /// Card width in px (Heroic: `max-width: min(700px, 85vw)`, Collection
    /// dialogs override it, e.g. settings 840px).
    pub width: f32,
    /// Optional fixed content height (scrolls inside).
    pub height: Option<f32>,
}

/// Wraps `body` (already padded by the caller via [`dialog_content`]) and
/// `footer` into the modal. `on_close` runs for the X button and backdrop
/// clicks. Render it as the last child of the root so it paints on top.
pub fn dialog(
    props: DialogProps,
    body: AnyElement,
    footer: Option<AnyElement>,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let on_close = std::rc::Rc::new(on_close);
    let close_btn = {
        let on_close = on_close.clone();
        div()
            .id(SharedString::from(format!("{}-close", props.id)))
            .absolute()
            .top(px(14.))
            .right(px(20.))
            .p(px(4.))
            .rounded(px(8.))
            .cursor_pointer()
            .border_1()
            .border_color(theme::white_a(0.6))
            .hover(|s| s.bg(theme::white_a(0.08)))
            .on_click(move |_, w, cx| on_close(w, cx))
            .child(icon(Icon::Close, px(16.), rgb(0xcaf3fd)))
    };
    let mut card = div()
        .id(props.id.clone())
        .occlude()
        .relative()
        .flex()
        .flex_col()
        .w(px(props.width))
        .max_h(relative(0.95))
        .rounded(px(10.))
        .border_1()
        .border_color(theme::background())
        .bg(theme::background())
        .text_color(rgb(0xcaf3fd))
        .font_family(theme::FONT_BODY)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .pt(px(24. + 24.))
                .pr(px(16.))
                .pl(px(32.))
                .pb(px(24.))
                .font_family(theme::FONT_HEADING)
                .text_size(px(23.04))
                .font_weight(theme::W_BOLD)
                .text_color(rgb(0xcaf3fd))
                .child(props.title.clone()),
        )
        .child(close_btn);
    let content = div()
        .id(SharedString::from(format!("{}-content", props.id)))
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px(px(32.))
        .pb(px(24.))
        .child(body);
    card = card.child(match props.height {
        Some(h) => content.h(px(h)).flex_none(),
        None => content,
    });
    if let Some(footer) = footer {
        card = card.child(div().flex().justify_end().gap(px(16.)).px(px(32.)).pb(px(24.)).child(footer));
    }
    let backdrop_close = on_close.clone();
    deferred(
        div()
            .id(SharedString::from(format!("{}-backdrop", props.id)))
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            // `--modal-backdrop` = `--background-darker-80` (#191c22 at 50%).
            .bg(theme::rgb_a(25, 28, 34, 0.5))
            .on_mouse_down(MouseButton::Left, move |_, w, cx| backdrop_close(w, cx))
            .child(card.with_animation(
                SharedString::from(format!("{}-open", props.id)),
                Animation::new(Duration::from_millis(500)).with_easing(ease_out_quint()),
                |el, t| el.opacity(t).mt(px(50. * (1. - t))),
            )),
    )
    .with_priority(30)
    .into_any_element()
}
