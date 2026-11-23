//! A keyboard shortcut field: a focusable box showing the accelerator; the
//! next key combination pressed in it replaces it.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use crate::theme;

/// What a key pressed in a shortcut field does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyPress {
    /// The new shortcut.
    Set(String),
    /// Backspace or Delete alone, in a field that can be empty.
    Clear,
    /// Esc or Tab alone: the keyboard leaves the field, which keeps its value.
    Leave,
    /// A key that is a shortcut only with Ctrl, Alt or the logo key (a
    /// letter, Enter…, alone or with Shift): it would take the key, or its
    /// capital, from every program. The accelerator it would be.
    NeedsModifier(String),
    /// A modifier alone, or a key no global shortcut can have.
    Ignore,
}

/// Reads a key pressed in a shortcut field (all of them share this, so they
/// take and refuse the same keys). `clearable`: Backspace or Delete empties it.
pub fn hotkey_press(ks: &Keystroke, clearable: bool) -> HotkeyPress {
    let m = &ks.modifiers;
    let plain = !(m.control || m.alt || m.shift || m.platform);
    match ks.key.as_str() {
        "escape" if plain => return HotkeyPress::Leave,
        // Shift Tab moves back: still not a shortcut.
        "tab" if !(m.control || m.alt || m.platform) => return HotkeyPress::Leave,
        "backspace" | "delete" if plain && clearable => return HotkeyPress::Clear,
        _ => {}
    }
    // Shift alone still types (a capital, a symbol) or edits (Shift Enter, a
    // selection with Shift and an arrow): it is no modifier for this.
    let types = !(m.control || m.alt || m.platform);
    match accelerator_from_keystroke(ks) {
        None => HotkeyPress::Ignore,
        Some(a) if types && !works_alone(a.rsplit('+').next().unwrap_or(&a)) => HotkeyPress::NeedsModifier(a),
        Some(a) => HotkeyPress::Set(a),
    }
}

/// Whether a key without Ctrl, Alt or the logo key (alone or with Shift) can
/// be a global shortcut: one no program needs typed (F1–F24, Print Screen,
/// Pause, Scroll Lock), so taking it everywhere costs nothing.
pub fn works_alone(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    let f_key = k.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=24).contains(&n));
    f_key || matches!(k.as_str(), "printscreen" | "pause" | "scrolllock")
}

/// What a shortcut field says when a key needs a modifier.
pub fn needs_modifier_text(key: &str) -> String {
    let logo = if cfg!(windows) { "Win" } else { "Super" };
    tr!("{key} needs Ctrl, Alt or {logo} with it; only the F keys, Print Screen and Pause work alone or with just Shift.", key, logo)
}

/// Electron-style accelerator from a key press (`acceleratorFromKey`), or
/// `None` for a modifier alone and for keys no global shortcut can have.
/// The Super (logo) key is `Super`.
fn accelerator_from_keystroke(ks: &Keystroke) -> Option<String> {
    let k = ks.key.as_str();
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
        // `+` separates the parts of an accelerator.
        "+" => "Plus".into(),
        "-" => "Minus".into(),
        k if k.chars().count() == 1 => k.to_uppercase(),
        k => {
            let mut c = k.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
    };
    let m = &ks.modifiers;
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
    // Keys no global shortcut can have (punctuation) change nothing.
    Some(parts.join("+")).filter(|a| heronite_media::capture::is_bindable(a))
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

#[cfg(test)]
mod tests {
    use gpui_kit::Keystroke;

    use super::{hotkey_press, works_alone, HotkeyPress};

    fn press(keys: &str, clearable: bool) -> HotkeyPress {
        hotkey_press(&Keystroke::parse(keys).unwrap(), clearable)
    }

    #[test]
    fn keystrokes_become_shortcuts_or_field_actions() {
        use HotkeyPress::*;
        let set = |s: &str| Set(s.into());
        let needs = |s: &str| NeedsModifier(s.into());
        for (keys, clearable, want) in [
            // Modifiers make a shortcut.
            ("ctrl-shift-g", false, set("CommandOrControl+Shift+G")),
            ("alt-space", false, set("Alt+Space")),
            ("shift-f2", false, set("Shift+F2")),
            ("ctrl-escape", false, set("CommandOrControl+Esc")),
            ("ctrl-shift-a", false, set("CommandOrControl+Shift+A")),
            // Plain keys, and Shift with keys that type, need a modifier.
            ("a", false, needs("A")),
            ("space", false, needs("Space")),
            ("enter", false, needs("Enter")),
            ("backspace", false, needs("Backspace")),
            ("shift-a", false, needs("Shift+A")),
            ("shift-enter", false, needs("Shift+Enter")),
            ("shift-space", false, needs("Shift+Space")),
            ("shift-up", false, needs("Shift+Up")),
            ("shift-backspace", true, needs("Shift+Backspace")),
            // F keys work alone.
            ("f10", false, set("F10")),
            ("f24", false, set("F24")),
            // Esc and Tab leave the field.
            ("escape", true, Leave),
            ("tab", false, Leave),
            ("shift-tab", false, Leave),
            // Backspace clears only where the field can be empty.
            ("backspace", true, Clear),
            ("delete", true, Clear),
            ("ctrl-backspace", true, set("CommandOrControl+Backspace")),
            // Modifiers alone change nothing.
            ("shift", false, Ignore),
            ("control", false, Ignore),
        ] {
            assert_eq!(press(keys, clearable), want, "{keys} (clearable: {clearable})");
        }
    }

    #[test]
    fn f_keys_and_print_screen_work_alone() {
        for key in ["PrintScreen", "Pause", "F1", "F24"] {
            assert!(works_alone(key), "{key}");
        }
        for key in ["F", "F25", "Fx"] {
            assert!(!works_alone(key), "{key}");
        }
    }
}
