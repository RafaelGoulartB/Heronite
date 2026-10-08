//! `CollectionDropdown`: a header tool button whose menu opens on hover and
//! closes 180ms after the pointer leaves (button + menu count as one area).

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::theme;

/// Menu panel (`.dropdown.expanded`): `--background`, radius .5em, padding
/// .35rem, `4px 5px 5px 7px rgba(0,0,0,.2)` shadow.
pub fn menu_panel(min_w: f32) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.8))
        .min_w(px(min_w))
        .p(px(5.6))
        .rounded(px(8.))
        .bg(theme::background())
        .shadow(vec![BoxShadow {
            color: theme::black_a(0.2),
            offset: point(px(4.), px(5.)),
            blur_radius: px(5.),
            spread_radius: px(7.),
            inset: false,
        }])
}

/// One menu row: `--text-sm` (13.33px), padding .5em/.75em, radius .25em,
/// hover `rgba(255,255,255,.12)` + white, active item in `--accent`.
pub fn menu_item(id: impl Into<ElementId>, active: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(10.))
        .w_full()
        .py(px(6.67))
        .px(px(10.))
        .rounded(px(3.33))
        .text_size(px(13.33))
        .cursor_pointer()
        .text_color(if active { theme::accent() } else { rgb(0xcaf3fd) })
        .hover(|s| s.bg(theme::white_a(0.12)).text_color(white()))
}

pub fn menu_separator() -> Div {
    div().h(px(1.)).my(px(2.)).bg(theme::white_a(0.1))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Left,
    Right,
}

/// Wraps `button` and, when `open`, the `menu` anchored below it.
pub fn dropdown(
    id: impl Into<ElementId>,
    button: impl IntoElement,
    menu: Option<AnyElement>,
    align: Align,
    on_hover: impl Fn(&bool, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let mut container = div().id(id).relative().flex_none().on_hover(on_hover).child(button);
    if let Some(menu) = menu {
        let positioned = div().absolute().top(px(32.)).pt(px(4.)).occlude();
        let positioned = match align {
            Align::Left => positioned.left_0(),
            Align::Right => positioned.right_0(),
        };
        container = container.child(deferred(positioned.child(menu)).with_priority(10));
    }
    container
}
