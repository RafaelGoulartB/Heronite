//! Glass modal and drawer. The Collection stays visible behind a dark veil;
//! the panel is a glass surface (see `ui::glass`) with a round close button.
//!
//! * [`dialog`]: centred modal, radius 26, title Rubik 22, padding 28.
//! * [`drawer`]: right-hand sheet inset 10px, radius 24, with header and
//!   footer slots (game settings).

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{close_button, dialog_title};
use crate::ui::{glass, icon};

pub struct DialogProps {
    pub id: SharedString,
    pub title: SharedString,
    /// Card width in px (Collection dialogs override it, e.g. settings 840px).
    pub width: f32,
    /// Optional fixed content height (scrolls inside).
    pub height: Option<f32>,
}

/// Wraps `body` and `footer` into the modal. `on_close` runs for the X button
/// and backdrop clicks. Render it as the last child of the root so it paints
/// on top.
pub fn dialog(
    props: DialogProps,
    body: AnyElement,
    footer: Option<AnyElement>,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    build(props, None, body, footer, None, on_close)
}

/// [`dialog`] whose body shows a draggable scrollbar while it overflows.
/// The caller keeps `scroll` between frames; its size is only known after a
/// first layout, so the caller draws once more when `scroll.bounds()` is
/// still empty.
pub fn dialog_scrolled(
    props: DialogProps,
    scroll: &ScrollHandle,
    body: AnyElement,
    footer: Option<AnyElement>,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    build(props, Some(scroll), body, footer, None, on_close)
}

/// [`dialog_scrolled`] with a round back arrow before the title: a dialog
/// showing a step of another one (`on_back` returns to it).
pub fn dialog_scrolled_with_back(
    props: DialogProps,
    scroll: &ScrollHandle,
    body: AnyElement,
    footer: Option<AnyElement>,
    on_back: impl Fn(&mut Window, &mut App) + 'static,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    build(props, Some(scroll), body, footer, Some(Rc::new(on_back)), on_close)
}

fn build(
    props: DialogProps,
    scroll: Option<&ScrollHandle>,
    body: AnyElement,
    footer: Option<AnyElement>,
    on_back: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let on_close = Rc::new(on_close);
    let close = {
        let on_close = on_close.clone();
        close_button(SharedString::from(format!("{}-close", props.id))).on_click(move |_, w, cx| on_close(w, cx))
    };
    let header = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .pl(px(28.))
        .pr(px(20.))
        .pt(px(22.))
        .pb(px(14.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .min_w_0()
                .children(on_back.map(|back| {
                    div()
                        .id(SharedString::from(format!("{}-back", props.id)))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(px(32.))
                        .rounded_full()
                        .bg(theme::ink_a(0.08))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::ink_a(0.14)))
                        .on_click(move |_, w, cx| back(w, cx))
                        .child(icon(Icon::ChevronLeft, px(18.), theme::text_primary()))
                }))
                .child(div().min_w_0().child(dialog_title(props.title.clone()))),
        )
        .child(close);
    let content = div()
        .id(SharedString::from(format!("{}-content", props.id)))
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px(px(28.))
        .pb(px(24.))
        .child(body);
    let content = match props.height {
        Some(h) => content.h(px(h)).flex_none(),
        None => content,
    };
    let content = match scroll {
        Some(scroll) => {
            // Only one dialog is open at a time: one id for its scrollbar.
            let bar = crate::ui::scrollbar_draggable(
                "dialog-scrollbar",
                scroll.offset().y,
                scroll.max_offset().y,
                scroll.bounds().size.height,
                {
                    let scroll = scroll.clone();
                    move |distance, window, _| {
                        scroll.set_offset(point(px(0.), px(-distance)));
                        window.refresh();
                    }
                },
                None,
            );
            div()
                .relative()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(content.track_scroll(scroll))
                .children(bar.map(|b| b.right(px(8.))))
                .into_any_element()
        }
        None => content.into_any_element(),
    };
    let mut card = glass::glass_panel(theme::R_SHEET)
        .id(props.id.clone())
        .occlude()
        .flex()
        .flex_col()
        .w(px(props.width))
        .max_h(relative(0.95))
        .text_color(theme::text_primary())
        .font_family(theme::font_body())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(header)
        .child(content);
    if let Some(footer) = footer {
        card = card.child(div().flex().flex_none().justify_end().items_center().gap(px(10.)).px(px(28.)).pb(px(24.)).child(footer));
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
            .bg(theme::glass_backdrop_veil())
            .on_mouse_down(MouseButton::Left, move |_, w, cx| backdrop_close(w, cx))
            .child(if theme::animations() {
                card.with_animation(
                    SharedString::from(format!("{}-open", props.id)),
                    Animation::new(Duration::from_millis(220)).with_easing(ease_out_quint()),
                    |el, t| el.opacity(t).mt(px(18. * (1. - t))),
                )
                .into_any_element()
            } else {
                card.into_any_element()
            }),
    )
    .with_priority(30)
    .into_any_element()
}

pub struct DrawerProps {
    pub id: SharedString,
    /// Drawer width in px (game settings: 560).
    pub width: f32,
}

/// Right-hand glass drawer (inset 10px, radius 24) over a dark veil. `header`
/// and `footer` are fixed slots (build the header with `controls::kicker`,
/// `dialog_title`, `close_button`...); `body` scrolls and gets padding 16/20.
/// A veil click or `on_close` closes it. Render it last in the root.
pub fn drawer(
    props: DrawerProps,
    header: AnyElement,
    body: AnyElement,
    footer: Option<AnyElement>,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let on_close = std::rc::Rc::new(on_close);
    let mut panel = glass::glass_panel(theme::R_DRAWER)
        .id(props.id.clone())
        .occlude()
        .absolute()
        .top(px(10.))
        .right(px(10.))
        .bottom(px(10.))
        .w(px(props.width))
        .max_w(relative(0.96))
        .flex()
        .flex_col()
        .text_color(theme::text_primary())
        .font_family(theme::font_body())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(div().flex_none().child(header))
        .child(
            div()
                .id(SharedString::from(format!("{}-body", props.id)))
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(20.))
                .py(px(16.))
                // `HERONITE_DEBUG_DIALOG_SCROLL=<px>`: the body starts scrolled (captures).
                .child(div().when_some(crate::env::parse::<f32>("HERONITE_DEBUG_DIALOG_SCROLL"), |d, y| d.mt(px(-y))).child(body)),
        );
    if let Some(footer) = footer {
        panel = panel.child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_between()
                .px(px(20.))
                .py(px(14.))
                .border_t_1()
                .border_color(theme::glass_border())
                .child(footer),
        );
    }
    let backdrop_close = on_close.clone();
    deferred(
        div()
            .id(SharedString::from(format!("{}-backdrop", props.id)))
            .absolute()
            .inset_0()
            .occlude()
            .bg(theme::glass_backdrop_veil())
            .on_mouse_down(MouseButton::Left, move |_, w, cx| backdrop_close(w, cx))
            .child(if theme::animations() {
                panel
                    .with_animation(
                        SharedString::from(format!("{}-open", props.id)),
                        Animation::new(Duration::from_millis(240)).with_easing(ease_out_quint()),
                        |el, t| el.opacity(t).right(px(10. - 36. * (1. - t))),
                    )
                    .into_any_element()
            } else {
                panel.into_any_element()
            }),
    )
    .with_priority(30)
    .into_any_element()
}
