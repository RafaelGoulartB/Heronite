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
    /// A key that is a shortcut only with a modifier (a letter, Enter…): it
    /// would take the key from every program. The accelerator it would be.
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
    match accelerator_from_keystroke(ks) {
        None => HotkeyPress::Ignore,
        Some(a) if plain && !works_alone(&a) => HotkeyPress::NeedsModifier(a),
        Some(a) => HotkeyPress::Set(a),
    }
}

/// Whether a key without Ctrl, Alt, Shift or the logo key can be a global
/// shortcut: one no program needs typed (F1–F24, Print Screen, Pause, Scroll
/// Lock), so taking it everywhere costs nothing.
pub fn works_alone(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    let f_key = k.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=24).contains(&n));
    f_key || matches!(k.as_str(), "printscreen" | "pause" | "scrolllock")
}

/// What a shortcut field says when a key needs a modifier.
pub fn needs_modifier_text(key: &str) -> String {
    let logo = if cfg!(windows) { "Win" } else { "Super" };
    tr!("{key} needs Ctrl, Alt, Shift or {logo} with it; only the F keys, Print Screen and Pause work alone.", key, logo)
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
    fn modifiers_make_a_shortcut() {
        assert_eq!(press("ctrl-shift-g", false), HotkeyPress::Set("CommandOrControl+Shift+G".into()));
        assert_eq!(press("alt-space", false), HotkeyPress::Set("Alt+Space".into()));
        assert_eq!(press("shift-f2", false), HotkeyPress::Set("Shift+F2".into()));
        assert_eq!(press("ctrl-escape", false), HotkeyPress::Set("CommandOrControl+Esc".into()));
    }

    #[test]
    fn plain_keys_need_a_modifier() {
        assert_eq!(press("a", false), HotkeyPress::NeedsModifier("A".into()));
        assert_eq!(press("space", false), HotkeyPress::NeedsModifier("Space".into()));
        assert_eq!(press("enter", false), HotkeyPress::NeedsModifier("Enter".into()));
        assert_eq!(press("backspace", false), HotkeyPress::NeedsModifier("Backspace".into()));
    }

    #[test]
    fn f_keys_and_print_screen_work_alone() {
        assert_eq!(press("f10", false), HotkeyPress::Set("F10".into()));
        assert_eq!(press("f24", false), HotkeyPress::Set("F24".into()));
        assert!(works_alone("PrintScreen"));
        assert!(works_alone("Pause"));
        assert!(!works_alone("F"));
        assert!(!works_alone("F25"));
        assert!(!works_alone("Fx"));
    }

    #[test]
    fn esc_and_tab_leave_the_field() {
        assert_eq!(press("escape", true), HotkeyPress::Leave);
        assert_eq!(press("tab", false), HotkeyPress::Leave);
        assert_eq!(press("shift-tab", false), HotkeyPress::Leave);
    }

    #[test]
    fn backspace_clears_only_where_the_field_can_be_empty() {
        assert_eq!(press("backspace", true), HotkeyPress::Clear);
        assert_eq!(press("delete", true), HotkeyPress::Clear);
        assert_eq!(press("ctrl-backspace", true), HotkeyPress::Set("CommandOrControl+Backspace".into()));
    }

    #[test]
    fn modifiers_alone_change_nothing() {
        assert_eq!(press("shift", false), HotkeyPress::Ignore);
        assert_eq!(press("control", false), HotkeyPress::Ignore);
    }
}
