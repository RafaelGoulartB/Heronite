//! A keyboard shortcut field: a focusable box showing the accelerator; the
//! next key combination pressed in it replaces it.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::theme;

/// Electron-style accelerator from a key press (`acceleratorFromKey`), or
/// `None` for a modifier alone. The Super (logo) key is `Super`.
pub fn accelerator_from_key(ev: &KeyDownEvent) -> Option<String> {
    let k = ev.keystroke.key.as_str();
    if matches!(k, "control" | "ctrl" | "alt" | "shift" | "meta" | "super" | "platform" | "cmd" | "function") {
        return None;
    }
    let key = match k {
        "space" => "Space".to_string(),
        "up" => "Up".into(),
        "down" => "Down".into(),
        "left" => "Left".into(),
        "right" => "Right".into(),
        "escape" => "Esc".into(),
        "print" | "printscreen" => "PrintScreen".into(),
        k if k.chars().count() == 1 => k.to_uppercase(),
        k => {
            let mut c = k.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
    };
    let m = &ev.keystroke.modifiers;
    let mut parts = Vec::new();
    if m.control {
        parts.push("CommandOrControl".to_string());
    }
    if m.platform {
        parts.push("Super".into());
    }
    if m.alt {
        parts.push("Alt".into());
    }
    if m.shift {
        parts.push("Shift".into());
    }
    parts.push(key);
    Some(parts.join("+"))
}

/// Read-only input look with the accelerator text (`input_box` needs an
/// `InputState`; this one is a plain focusable box).
pub fn hotkey_box(base: Stateful<Div>, text: String, focused: bool) -> Stateful<Div> {
    base.flex()
        .items_center()
        .w_full()
        .h(px(theme::INPUT_H))
        .px(px(14.))
        .rounded(px(theme::R_INPUT))
        .bg(theme::input_fill())
        .border_1()
        .border_color(if focused { theme::input_border_focus() } else { theme::input_border() })
        .font_family(theme::font_body())
        .text_size(px(14.))
        .text_color(theme::text_primary())
        .child(text)
}
