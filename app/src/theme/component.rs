//! Colours of the gpui-component widgets.

use gpui_kit::{hsla, rgb};

use super::{background, navbar_background, white_a, FONT_BODY};

/// Colours of the gpui-component widgets we reuse (inputs, sliders, popovers)
/// so they look like Heroic's controls on nord-dark.
pub fn apply_component_theme(cx: &mut gpui_kit::App) {
    use gpui_kit::component::theme::{Theme, ThemeMode};
    Theme::change(ThemeMode::Dark, None, cx);
    let t = Theme::global_mut(cx);
    t.font_family = FONT_BODY.into();
    let c = &mut t.colors;
    c.background = background().into();
    c.foreground = rgb(0xcaf3fd).into();
    c.input = white_a(0.18);
    c.ring = rgb(0x1976d2).into();
    c.caret = white_a(0.9);
    c.selection = hsla(210. / 360., 0.8, 0.5, 0.35);
    c.primary = rgb(0x1976d2).into();
    c.slider_bar = rgb(0x1976d2).into();
    c.slider_thumb = rgb(0x1976d2).into();
    c.popover = navbar_background().into();
    c.popover_foreground = rgb(0xcaf3fd).into();
    c.border = white_a(0.12);
    c.muted_foreground = white_a(0.38);
}
