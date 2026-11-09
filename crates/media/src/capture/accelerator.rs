//! Electron accelerators and their Hyprland / XDG portal / Windows
//! `RegisterHotKey` translations, plus the `hyprctl binds -j` entries and Lua
//! expressions that install them.

use serde::Deserialize;

use super::BIND_DESCRIPTION;

/// A parsed Electron accelerator (`CommandOrControl+Shift+F10`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accelerator {
    /// Modifier names as written, in order.
    pub modifiers: Vec<String>,
    /// The key as written (`F10`, `Esc`, `PrintScreen`, `a`).
    pub key: String,
}

/// Parses an Electron-style accelerator. Returns `None` when there is no key
/// or it contains characters that cannot be a key name.
pub fn parse_accelerator(accelerator: &str) -> Option<Accelerator> {
    let mut parts: Vec<&str> = accelerator.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    let key = parts.pop()?;
    if !key.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b' ' | b'-')) {
        return None;
    }
    Some(Accelerator { modifiers: parts.into_iter().map(str::to_owned).collect(), key: key.to_owned() })
}

/// Hyprland modifier masks (`modmask` in `hyprctl binds -j`).
pub const MOD_SHIFT: u32 = 1;
pub const MOD_CTRL: u32 = 4;
pub const MOD_ALT: u32 = 8;
pub const MOD_SUPER: u32 = 64;

/// How a shortcut is expressed for Hyprland.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyprlandBinding {
    /// `CTRL + SHIFT + F10`, the string given to `hl.bind` / `hl.unbind`.
    pub keys: String,
    /// The key alone (`F10`).
    pub key: String,
    pub modmask: u32,
}

fn hypr_modifier(name: &str) -> Option<(&'static str, u32)> {
    Some(match name.to_ascii_lowercase().as_str() {
        "commandorcontrol" | "control" | "ctrl" => ("CTRL", MOD_CTRL),
        "command" | "super" | "meta" => ("SUPER", MOD_SUPER),
        "alt" | "option" => ("ALT", MOD_ALT),
        "shift" => ("SHIFT", MOD_SHIFT),
        _ => return None,
    })
}

/// `toHyprlandAccelerator`.
pub fn to_hyprland_binding(accelerator: &str) -> Option<HyprlandBinding> {
    let acc = parse_accelerator(accelerator)?;
    let mut names = Vec::new();
    let mut modmask = 0;
    for m in &acc.modifiers {
        let (name, mask) = hypr_modifier(m)?;
        names.push(name);
        modmask |= mask;
    }
    let key = match acc.key.as_str() {
        "Esc" => "Escape",
        "Up" => "UP",
        "Down" => "DOWN",
        "Left" => "LEFT",
        "Right" => "RIGHT",
        "PrintScreen" => "Print",
        other => other,
    };
    names.push(key);
    Some(HyprlandBinding { keys: names.join(" + "), key: key.to_owned(), modmask })
}

/// Trigger string for the XDG GlobalShortcuts portal
/// (`CTRL+SHIFT+F10`; modifiers `SHIFT CTRL ALT LOGO`, key = XKB keysym name).
pub fn to_portal_trigger(accelerator: &str) -> Option<String> {
    let acc = parse_accelerator(accelerator)?;
    let mut out: Vec<String> = Vec::new();
    for m in &acc.modifiers {
        out.push(
            match m.to_ascii_lowercase().as_str() {
                "commandorcontrol" | "control" | "ctrl" => "CTRL",
                "command" | "super" | "meta" => "LOGO",
                "alt" | "option" => "ALT",
                "shift" => "SHIFT",
                _ => return None,
            }
            .to_owned(),
        );
    }
    out.push(keysym_name(&acc.key));
    Some(out.join("+"))
}

/// Electron key name -> XKB keysym name.
fn keysym_name(key: &str) -> String {
    let mapped = match key.to_ascii_lowercase().as_str() {
        "esc" | "escape" => "Escape",
        "printscreen" | "print" => "Print",
        "up" => "Up",
        "down" => "Down",
        "left" => "Left",
        "right" => "Right",
        "space" => "space",
        "plus" => "plus",
        "enter" | "return" => "Return",
        "backspace" => "BackSpace",
        "pageup" => "Page_Up",
        "pagedown" => "Page_Down",
        "capslock" => "Caps_Lock",
        "scrolllock" => "Scroll_Lock",
        "numlock" => "Num_Lock",
        "tab" => "Tab",
        "delete" => "Delete",
        "insert" => "Insert",
        "home" => "Home",
        "end" => "End",
        _ => "",
    };
    if !mapped.is_empty() {
        return mapped.to_owned();
    }
    // F1..F24 keep their case; single letters are lowercase keysyms.
    if key.len() == 1 && key.as_bytes()[0].is_ascii_alphabetic() {
        return key.to_ascii_lowercase();
    }
    let b = key.as_bytes();
    if b.len() >= 2 && (b[0] == b'f' || b[0] == b'F') && b[1..].iter().all(u8::is_ascii_digit) {
        return format!("F{}", &key[1..]);
    }
    key.to_owned()
}

// ---------------------------------------------------------------------------
// Windows `RegisterHotKey`

/// `RegisterHotKey` modifier flags (`MOD_*` of `winuser.h`).
pub const WIN_MOD_ALT: u32 = 0x0001;
pub const WIN_MOD_CONTROL: u32 = 0x0002;
pub const WIN_MOD_SHIFT: u32 = 0x0004;
pub const WIN_MOD_WIN: u32 = 0x0008;
/// Holding the keys down does not repeat the hotkey.
pub const WIN_MOD_NOREPEAT: u32 = 0x4000;

/// How a shortcut is given to Windows' `RegisterHotKey`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsHotkey {
    /// `MOD_ALT | MOD_CONTROL | MOD_SHIFT | MOD_WIN` (without `MOD_NOREPEAT`).
    pub modifiers: u32,
    /// The virtual-key code (`VK_F10`).
    pub vk: u32,
}

/// The `RegisterHotKey` modifiers and virtual key of an accelerator; `None`
/// for a modifier or key Windows has no hotkey for.
pub fn to_windows_hotkey(accelerator: &str) -> Option<WindowsHotkey> {
    let acc = parse_accelerator(accelerator)?;
    let mut modifiers = 0;
    for m in &acc.modifiers {
        modifiers |= match m.to_ascii_lowercase().as_str() {
            "commandorcontrol" | "control" | "ctrl" => WIN_MOD_CONTROL,
            "command" | "super" | "meta" | "win" | "windows" => WIN_MOD_WIN,
            "alt" | "option" => WIN_MOD_ALT,
            "shift" => WIN_MOD_SHIFT,
            _ => return None,
        };
    }
    Some(WindowsHotkey { modifiers, vk: virtual_key(&acc.key)? })
}

/// Electron key name -> Windows virtual-key code (`VK_*`, layout independent
/// for every key but the letters and digits, which are their ASCII codes).
fn virtual_key(key: &str) -> Option<u32> {
    let lower = key.to_ascii_lowercase();
    let b = lower.as_bytes();
    if b.len() == 1 && (b[0].is_ascii_alphabetic() || b[0].is_ascii_digit()) {
        return Some(u32::from(b[0].to_ascii_uppercase()));
    }
    // F1..F24: VK_F1 = 0x70.
    if b.len() >= 2 && b[0] == b'f' && b[1..].iter().all(u8::is_ascii_digit) {
        let n: u32 = lower[1..].parse().ok()?;
        return (1..=24).contains(&n).then(|| 0x70 + n - 1);
    }
    // num0..num9: VK_NUMPAD0 = 0x60.
    if let Some(d) = lower.strip_prefix("num").filter(|d| d.len() == 1 && d.as_bytes()[0].is_ascii_digit()) {
        return Some(0x60 + u32::from(d.as_bytes()[0] - b'0'));
    }
    Some(match lower.as_str() {
        "space" => 0x20,
        "esc" | "escape" => 0x1B,
        "enter" | "return" => 0x0D,
        "tab" => 0x09,
        "backspace" => 0x08,
        "delete" | "del" => 0x2E,
        "insert" | "ins" => 0x2D,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "printscreen" | "print" => 0x2C,
        "pause" => 0x13,
        "capslock" => 0x14,
        "numlock" => 0x90,
        "scrolllock" => 0x91,
        "plus" => 0xBB,
        "-" | "minus" => 0xBD,
        "numdec" => 0x6E,
        "numadd" => 0x6B,
        "numsub" => 0x6D,
        "nummult" => 0x6A,
        "numdiv" => 0x6F,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Hyprland bind inspection (`hyprctl binds -j`)

/// One entry of `hyprctl binds -j`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HyprlandBind {
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub modmask: Option<u32>,
    #[serde(default)]
    pub description: Option<String>,
}

impl HyprlandBind {
    /// Same key and modifiers (key compared case-insensitively).
    pub fn is_same_trigger(&self, binding: &HyprlandBinding) -> bool {
        self.key.as_deref().map(str::to_uppercase).as_deref() == Some(binding.key.to_uppercase().as_str())
            && self.modmask == Some(binding.modmask)
    }

    /// One of ours (left behind by a crashed session) with this trigger.
    pub fn is_ours(&self, binding: &HyprlandBinding) -> bool {
        self.is_ours_as(binding, BIND_DESCRIPTION)
    }

    /// [`is_ours`](Self::is_ours) for the bind described as `description`.
    pub fn is_ours_as(&self, binding: &HyprlandBinding, description: &str) -> bool {
        self.description.as_deref() == Some(description) && self.is_same_trigger(binding)
    }
}

/// The Lua expression that installs the temporary screenshot bind (the game
/// gets the keys too).
pub fn hyprland_bind_expression(binding: &HyprlandBinding, pid: u32) -> String {
    hyprland_bind_expression_as(binding, &format!("kill -USR2 {pid}"), BIND_DESCRIPTION, false)
}

/// A temporary bind that runs `command`. `consume`: the focused window (the
/// game) does not get the keys.
pub fn hyprland_bind_expression_as(binding: &HyprlandBinding, command: &str, description: &str, consume: bool) -> String {
    let q = |s: &str| serde_json::to_string(s).expect("string");
    format!(
        "hl.bind({}, hl.dsp.exec_cmd({}), {{ non_consuming = {}, dont_inhibit = true, allow_input_capture = true, description = {} }})",
        q(&binding.keys),
        q(command),
        !consume,
        q(description),
    )
}

/// The Lua expression that removes a bind.
pub fn hyprland_unbind_expression(keys: &str) -> String {
    format!("hl.unbind({})", serde_json::to_string(keys).expect("string"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_accelerators() {
        assert_eq!(
            parse_accelerator("CommandOrControl + Shift+F10"),
            Some(Accelerator { modifiers: vec!["CommandOrControl".into(), "Shift".into()], key: "F10".into() })
        );
        assert_eq!(parse_accelerator("F12").unwrap().modifiers.len(), 0);
        assert!(parse_accelerator("").is_none());
        assert!(parse_accelerator("Ctrl+").map(|a| a.key) == Some("Ctrl".into()));
        assert!(parse_accelerator("Ctrl+?").is_none());
    }

    #[test]
    fn hyprland_translation() {
        let b = to_hyprland_binding("F12").unwrap();
        assert_eq!((b.keys.as_str(), b.key.as_str(), b.modmask), ("F12", "F12", 0));
        let b = to_hyprland_binding("CommandOrControl+Shift+PrintScreen").unwrap();
        assert_eq!(b.keys, "CTRL + SHIFT + Print");
        assert_eq!(b.key, "Print");
        assert_eq!(b.modmask, MOD_CTRL | MOD_SHIFT);
        let b = to_hyprland_binding("Super+Alt+Esc").unwrap();
        assert_eq!(b.keys, "SUPER + ALT + Escape");
        assert_eq!(b.modmask, MOD_SUPER | MOD_ALT);
        assert_eq!(to_hyprland_binding("Meta+Up").unwrap().keys, "SUPER + UP");
        assert_eq!(to_hyprland_binding("alt+Left").unwrap().keys, "ALT + LEFT");
        assert_eq!(to_hyprland_binding("Option+Down").unwrap().keys, "ALT + DOWN");
        assert_eq!(to_hyprland_binding("Command+Right").unwrap().modmask, MOD_SUPER);
        assert_eq!(to_hyprland_binding("Control+A").unwrap().keys, "CTRL + A");
        // Unknown modifier / bad key.
        assert!(to_hyprland_binding("Hyper+F1").is_none());
        assert!(to_hyprland_binding("Ctrl+F1+?").is_none());
        assert!(to_hyprland_binding("").is_none());
    }

    #[test]
    fn portal_translation() {
        assert_eq!(to_portal_trigger("F12").unwrap(), "F12");
        assert_eq!(to_portal_trigger("CommandOrControl+Shift+PrintScreen").unwrap(), "CTRL+SHIFT+Print");
        assert_eq!(to_portal_trigger("Super+Alt+Esc").unwrap(), "LOGO+ALT+Escape");
        assert_eq!(to_portal_trigger("Ctrl+A").unwrap(), "CTRL+a");
        assert_eq!(to_portal_trigger("Ctrl+5").unwrap(), "CTRL+5");
        assert_eq!(to_portal_trigger("Alt+PageUp").unwrap(), "ALT+Page_Up");
        assert_eq!(to_portal_trigger("Ctrl+Space").unwrap(), "CTRL+space");
        assert_eq!(to_portal_trigger("f9").unwrap(), "F9");
        assert!(to_portal_trigger("Hyper+F1").is_none());
    }

    #[test]
    fn windows_hotkeys() {
        let hk = |a: &str| to_windows_hotkey(a).map(|h| (h.modifiers, h.vk));
        assert_eq!(hk("F10"), Some((0, 0x79)));
        assert_eq!(hk("f1"), Some((0, 0x70)));
        assert_eq!(hk("F24"), Some((0, 0x87)));
        assert_eq!(hk("CommandOrControl+Shift+PrintScreen"), Some((WIN_MOD_CONTROL | WIN_MOD_SHIFT, 0x2C)));
        assert_eq!(hk("Super+Alt+Esc"), Some((WIN_MOD_WIN | WIN_MOD_ALT, 0x1B)));
        assert_eq!(hk("Ctrl+a"), Some((WIN_MOD_CONTROL, u32::from(b'A'))));
        assert_eq!(hk("Alt+5"), Some((WIN_MOD_ALT, u32::from(b'5'))));
        assert_eq!(hk("Shift+Space"), Some((WIN_MOD_SHIFT, 0x20)));
        assert_eq!(hk("Alt+PageUp"), Some((WIN_MOD_ALT, 0x21)));
        assert_eq!(hk("Option+Pagedown"), Some((WIN_MOD_ALT, 0x22)));
        assert_eq!(hk("Meta+Up"), Some((WIN_MOD_WIN, 0x26)));
        assert_eq!(hk("Control+num7"), Some((WIN_MOD_CONTROL, 0x67)));
        assert_eq!(hk("Ctrl+Plus"), Some((WIN_MOD_CONTROL, 0xBB)));
        assert_eq!(hk("Command+Right"), Some((WIN_MOD_WIN, 0x27)));
        // The settings' console choices all map.
        for a in ["F12", "Shift+F12", "CommandOrControl+F12", "Alt+F12", "CommandOrControl+Shift+S"] {
            assert!(hk(a).is_some(), "{a}");
        }
        // No such key, modifier, or F number.
        assert!(hk("F25").is_none());
        assert!(hk("F0").is_none());
        assert!(hk("Hyper+F1").is_none());
        assert!(hk("Ctrl+Bogus").is_none());
        assert!(hk("Ctrl+?").is_none());
        assert!(hk("").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn windows_hotkey_constants_match_the_sdk() {
        use windows::Win32::UI::Input::KeyboardAndMouse as k;
        assert_eq!(WIN_MOD_ALT, k::MOD_ALT.0);
        assert_eq!(WIN_MOD_CONTROL, k::MOD_CONTROL.0);
        assert_eq!(WIN_MOD_SHIFT, k::MOD_SHIFT.0);
        assert_eq!(WIN_MOD_WIN, k::MOD_WIN.0);
        assert_eq!(WIN_MOD_NOREPEAT, k::MOD_NOREPEAT.0);
        let vk = |a: &str| to_windows_hotkey(a).unwrap().vk;
        for (name, code) in [
            ("F10", k::VK_F10),
            ("F24", k::VK_F24),
            ("Esc", k::VK_ESCAPE),
            ("PrintScreen", k::VK_SNAPSHOT),
            ("PageUp", k::VK_PRIOR),
            ("PageDown", k::VK_NEXT),
            ("Insert", k::VK_INSERT),
            ("Delete", k::VK_DELETE),
            ("Home", k::VK_HOME),
            ("End", k::VK_END),
            ("Backspace", k::VK_BACK),
            ("Return", k::VK_RETURN),
            ("Pause", k::VK_PAUSE),
            ("CapsLock", k::VK_CAPITAL),
            ("NumLock", k::VK_NUMLOCK),
            ("ScrollLock", k::VK_SCROLL),
            ("Plus", k::VK_OEM_PLUS),
            ("Minus", k::VK_OEM_MINUS),
            ("num0", k::VK_NUMPAD0),
            ("num9", k::VK_NUMPAD9),
            ("numadd", k::VK_ADD),
            ("numsub", k::VK_SUBTRACT),
            ("nummult", k::VK_MULTIPLY),
            ("numdiv", k::VK_DIVIDE),
            ("numdec", k::VK_DECIMAL),
            ("Left", k::VK_LEFT),
            ("Down", k::VK_DOWN),
        ] {
            assert_eq!(vk(name), u32::from(code.0), "{name}");
        }
    }

    #[test]
    fn hyprland_expressions() {
        let b = to_hyprland_binding("Ctrl+F10").unwrap();
        assert_eq!(
            hyprland_bind_expression(&b, 4242),
            r#"hl.bind("CTRL + F10", hl.dsp.exec_cmd("kill -USR2 4242"), { non_consuming = true, dont_inhibit = true, allow_input_capture = true, description = "Heronite game screenshot" })"#
        );
        assert_eq!(hyprland_unbind_expression("CTRL + F10"), r#"hl.unbind("CTRL + F10")"#);
        let b = to_hyprland_binding("Super+G").unwrap();
        assert_eq!(
            hyprland_bind_expression_as(&b, "kill -USR1 7", "Heronite overlay", true),
            r#"hl.bind("SUPER + G", hl.dsp.exec_cmd("kill -USR1 7"), { non_consuming = false, dont_inhibit = true, allow_input_capture = true, description = "Heronite overlay" })"#
        );
    }

    #[test]
    fn bind_matching() {
        let b = to_hyprland_binding("Ctrl+f10").unwrap();
        let bind: HyprlandBind =
            serde_json::from_str(r#"{"key":"F10","modmask":4,"description":"Heronite game screenshot","dispatcher":"exec"}"#).unwrap();
        assert!(bind.is_same_trigger(&b) && bind.is_ours(&b));
        let other: HyprlandBind = serde_json::from_str(r#"{"key":"F10","modmask":4,"description":""}"#).unwrap();
        assert!(other.is_same_trigger(&b) && !other.is_ours(&b));
        let diff: HyprlandBind = serde_json::from_str(r#"{"key":"F10","modmask":5}"#).unwrap();
        assert!(!diff.is_same_trigger(&b));
        let missing: HyprlandBind = serde_json::from_str("{}").unwrap();
        assert!(!missing.is_same_trigger(&b));
    }
}
