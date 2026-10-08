//! Local glass styling of the palette, following docs/agents/glass-common.md.
//! TODO(glass foundation): switch to `theme`/`ui` helpers (glass_panel, kbd,
//! caption) once they are committed and delete what they replace.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::theme;

/// Panel fill when there is no blurred art behind it.
pub fn panel_fill() -> Rgba {
    theme::rgb_a(22, 27, 36, 0.94)
}

pub fn text() -> Rgba {
    rgb(0xe8edf4)
}

pub fn muted() -> Rgba {
    rgb(0x8c97a8)
}

pub fn icon_idle() -> Rgba {
    rgb(0x9aa6b6)
}

/// Selected row: accent tint.
pub fn selected_fill() -> Rgba {
    theme::rgb_a(165, 220, 235, 0.16)
}

/// Keyboard hint chip (`kbd`): 11px mono, radius 5.
pub fn kbd(text: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(5.))
        .bg(theme::white_a(0.08))
        .border_1()
        .border_color(theme::white_a(0.12))
        .font_family("monospace")
        .text_size(px(11.))
        .text_color(rgb(0xc3cbd8))
        .child(text.into())
}
