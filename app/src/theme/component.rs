//! Colours of the gpui-component widgets.

use gpui_kit::{hsla, rgb};

use super::{background, custom_palette, ink_a, is_light, navbar_background, text_primary};

/// Colours of the gpui-component widgets we reuse (inputs, sliders, popovers)
/// so they look like Heroic's controls on nord-dark, or like the light
/// theme's. Run again when the theme changes (`CollectionView::sync_look`).
pub fn apply_component_theme(cx: &mut gpui_kit::App) {
    use gpui_kit::component::theme::{Theme, ThemeMode};
    let light = is_light();
    Theme::change(if light { ThemeMode::Light } else { ThemeMode::Dark }, None, cx);
    let t = Theme::global_mut(cx);
    t.font_family = super::font_body().into();
    let c = &mut t.colors;
    let foreground = if light || custom_palette().is_some() { text_primary().into() } else { rgb(0xcaf3fd).into() };
    c.background = background().into();
    c.foreground = foreground;
    c.input = ink_a(0.18);
    c.ring = rgb(0x1976d2).into();
    c.caret = ink_a(0.9);
    c.selection = hsla(210. / 360., 0.8, 0.5, 0.35);
    c.primary = rgb(0x1976d2).into();
    c.slider_bar = rgb(0x1976d2).into();
    c.slider_thumb = rgb(0x1976d2).into();
    c.popover = navbar_background().into();
    c.popover_foreground = foreground;
    c.border = ink_a(0.12);
    c.muted_foreground = ink_a(0.38);
}
