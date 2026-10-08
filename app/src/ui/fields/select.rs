//! `SelectField`: label, 40px select and its menu.

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::input::input_surface;
use super::text::field_label;
use crate::icons::Icon;
use crate::ui::dropdown::{menu_item, menu_panel};
use crate::theme;
use crate::ui::icon;

/// `SelectField`: label, 40px select and, when `open`, its menu of `options`
/// `(label, selected)`. `on_toggle` opens/closes it, `on_pick(index)` picks.
pub fn select_field<V: 'static>(
    id: &str,
    label: Option<&str>,
    value: impl Into<SharedString>,
    disabled: bool,
    open: bool,
    options: Vec<(SharedString, bool)>,
    cx: &mut Context<V>,
    on_toggle: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    on_pick: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) + 'static,
) -> Div {
    let select = input_surface()
        .id(SharedString::from(format!("{id}-select")))
        .relative()
        .flex()
        .items_center()
        .h(px(theme::INPUT_H))
        .w_full()
        .pl(px(14.))
        .pr(px(40.))
        .when(open && !disabled, |d| d.border_color(theme::input_border_focus()))
        .when(disabled, |d| d.opacity(0.5).cursor_not_allowed())
        .when(!disabled, |d| d.cursor_pointer().on_click(cx.listener(move |this, _, w, cx| on_toggle(this, w, cx))))
        .child(div().min_w_0().truncate().child(value.into()))
        .child(div().absolute().right(px(14.)).top_0().bottom_0().flex().items_center().child(icon(Icon::ExpandMore, px(18.), theme::text_muted())));
    let mut wrap = div()
        .relative()
        .flex()
        .flex_col()
        .children(label.map(|l| field_label(l.to_string())))
        .child(select);
    if open && !disabled {
        let on_pick = Rc::new(on_pick);
        let mut list = div()
            .id(SharedString::from(format!("{id}-list")))
            .flex()
            .flex_col()
            .gap(px(2.))
            .max_h(px(280.))
            .overflow_y_scroll();
        for (i, (text, selected)) in options.into_iter().enumerate() {
            let on_pick = on_pick.clone();
            list = list.child(
                menu_item(SharedString::from(format!("{id}-opt-{i}")), selected)
                    .child(div().min_w_0().truncate().child(text))
                    .on_click(cx.listener(move |this, _, w, cx| on_pick(this, i, w, cx))),
            );
        }
        let menu = menu_panel(100.).w_full().child(list);
        let top = if label.is_some() { 22. + 44. } else { 44. };
        wrap = wrap.child(deferred(div().absolute().top(px(top)).left_0().right_0().occlude().child(menu)).with_priority(40));
    }
    wrap
}
