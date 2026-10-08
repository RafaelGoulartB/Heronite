//! Heroic / MUI form controls used by Collection dialogs. Values come from
//! Heroic's CSS (`_buttons.scss`, `ToggleSwitch`, `SelectField`,
//! `TextInputField`) and MUI defaults (Tabs, Slider) as rendered in Heronite;
//! see `docs/reference/dialogs/*.png`.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// MUI default primary (Tabs indicator, Slider, focused outlines).
pub fn mui_primary() -> Rgba {
    rgb(0x1976d2)
}

/// Heroic `ToggleSwitch`: 20px checkbox, 3px accent border, radius 4,
/// checked = accent fill with ✓; label bold + accent when checked.
pub fn checkbox(id: impl Into<ElementId>, checked: bool, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.))
        .cursor_pointer()
        .child(
            div()
                .ml(px(13.))
                .size(px(20.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .border(px(3.))
                .border_color(theme::accent())
                .when(checked, |d| d.bg(theme::accent()))
                .when(checked, |d| d.child(icon(Icon::Check, px(16.), theme::background()))),
        )
        .child(
            div()
                .text_size(px(16.))
                .text_color(if checked { theme::accent() } else { rgb(0xcaf3fd) })
                .when(checked, |d| d.font_weight(theme::W_BOLD))
                .child(label.into()),
        )
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ButtonKind {
    /// `.button.is-primary` (`--brand-primary`).
    Primary,
    /// `.button.is-secondary`.
    Secondary,
    /// `.button.is-tertiary`.
    Tertiary,
    /// Outlined text button used in Collection dialogs (CHANGE / SEARCH WEB).
    Outline,
    /// Plain text button (RESET).
    Text,
}

/// Heroic `.button`: uppercase, 16px/500, padding .75em, radius 4.
pub fn button(id: impl Into<ElementId>, kind: ButtonKind, label: impl Into<SharedString>) -> Stateful<Div> {
    let label: SharedString = label.into();
    let base = div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .px(px(10.))
        .py(px(7.))
        .rounded(px(4.))
        .cursor_pointer()
        .text_size(px(13.33))
        .font_weight(theme::W_500)
        .child(label.to_uppercase());
    match kind {
        ButtonKind::Primary => base.bg(rgb(0x1de8f5)).text_color(rgb(0x161616)).hover(|s| s.bg(rgb(0x7fffff))),
        ButtonKind::Secondary => base.bg(rgb(0xa5dceb)).text_color(rgb(0x161616)).hover(|s| s.bg(theme::accent_overlay())),
        ButtonKind::Tertiary => base.bg(rgb(0x51595a)).text_color(rgb(0xffffff)).hover(|s| s.bg(theme::background())),
        ButtonKind::Outline => base
            .border_1()
            .border_color(theme::white_a(0.7))
            .text_color(rgb(0xcaf3fd))
            .hover(|s| s.bg(theme::white_a(0.08))),
        ButtonKind::Text => base.text_color(rgb(0xcaf3fd)).hover(|s| s.bg(theme::white_a(0.08))),
    }
}

/// MUI `Tabs` with scroll buttons: 14px/500 uppercase labels, active tab in
/// MUI primary with a 2px indicator; bottom border `rgba(255,255,255,.08)`.
pub fn tabs<T: Copy + PartialEq + 'static>(
    id: &str,
    items: &[(T, &'static str)],
    active: T,
    on_select: impl Fn(T, &mut Window, &mut App) + 'static,
) -> Div {
    let on_select = std::rc::Rc::new(on_select);
    let mut row = div().flex().items_end().gap(px(4.)).px(px(16.));
    for (i, (value, label)) in items.iter().enumerate() {
        let is_active = *value == active;
        let v = *value;
        let cb = on_select.clone();
        row = row.child(
            div()
                .id(SharedString::from(format!("{id}-tab-{i}")))
                .flex()
                .flex_col()
                .items_center()
                .cursor_pointer()
                .on_click(move |_, w, cx| cb(v, w, cx))
                .child(
                    div()
                        .px(px(12.))
                        .py(px(10.))
                        .text_size(px(14.))
                        .font_weight(theme::W_500)
                        .text_color(if is_active { rgb(0x90caf9).into() } else { theme::white_a(0.7) })
                        .child(label.to_uppercase()),
                )
                .child(div().h(px(2.)).w_full().when(is_active, |d| d.bg(mui_primary()))),
        );
    }
    div()
        .flex()
        .items_center()
        .mx(px(-16.))
        .border_b_1()
        .border_color(theme::white_a(0.08))
        .child(div().px(px(8.)).child(icon(Icon::ChevronLeft, px(18.), theme::white_a(0.5))))
        .child(row.flex_1())
        .child(div().px(px(8.)).child(icon(Icon::ChevronRight, px(18.), theme::white_a(0.5))))
}

/// Section heading inside dialogs (`h4`, 16px/600).
pub fn section_title(text: impl Into<SharedString>) -> Div {
    div().text_size(px(16.)).font_weight(theme::W_BOLD).text_color(rgb(0xcaf3fd)).child(text.into())
}

/// Secondary paragraph text (`--text-secondary` is #fff on nord-dark, 13.33px).
pub fn help_text(text: impl Into<SharedString>) -> Div {
    div().text_size(px(13.33)).text_color(white()).child(text.into())
}

/// Read-only select look (`.selectFieldWrapper`): 40px, `--input-background`,
/// radius 4, shadow, chevron at right. Clicking it opens a menu the caller
/// renders (see `ui::dropdown::menu_panel`).
pub fn select_box(id: impl Into<ElementId>, value: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex()
        .items_center()
        .h(px(40.))
        .w_full()
        .pl(px(12.))
        .pr(px(44.))
        .rounded(px(4.))
        .bg(theme::navbar_background())
        .shadow(vec![BoxShadow {
            color: theme::black_a(0.25),
            offset: point(px(0.), px(4.)),
            blur_radius: px(4.),
            spread_radius: px(0.),
            inset: false,
        }])
        .cursor_pointer()
        .text_size(px(16.))
        .text_color(rgb(0xa5edfd))
        .child(value.into())
        .child(
            div()
                .absolute()
                .right(px(0.))
                .top(px(8.))
                .bottom(px(8.))
                .w(px(40.))
                .flex()
                .items_center()
                .justify_center()
                .border_l_1()
                .border_color(theme::white_a(0.6))
                .child(icon(Icon::ExpandMore, px(18.), rgb(0xa5edfd))),
        )
}
