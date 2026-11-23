//! The app's global keyboard shortcuts seen together, so every shortcut
//! field (Overlay, Screenshots, Replay, Quick launcher and the Controller
//! table) refuses a key another one already has, whichever is set first,
//! and a key Heronite itself answers (Ctrl K, the command palette).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::SettingsPage;

/// A global keyboard shortcut of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Shortcut {
    Overlay,
    Screenshot,
    Replay,
    QuickLaunch,
}

/// The shortcuts that are on, each with its accelerator.
fn in_use(d: &SettingsPage, cx: &App) -> Vec<(Shortcut, String)> {
    let o = &d.overlay.prefs;
    let mut on = Vec::new();
    if o.enabled && o.keyboard_shortcut {
        on.push((Shortcut::Overlay, o.keyboard_accelerator.clone()));
    }
    if d.settings.screenshots.capture_enabled {
        on.push((Shortcut::Screenshot, super::screenshots::accelerator(d)));
    }
    if let Some(a) = super::replay::accelerator(d, cx) {
        on.push((Shortcut::Replay, a));
    }
    if let Some(a) = super::quick_launch::shown(cx) {
        on.push((Shortcut::QuickLaunch, a));
    }
    on
}

/// An accelerator in one spelling: the modifiers by what they are, in one
/// order, and the key, all lower case (`CommandOrControl+Shift+G` and
/// `shift+ctrl+g` are the same keys).
fn normalized(accelerator: &str) -> String {
    let mut parts: Vec<String> = accelerator.split('+').map(|p| p.trim().to_ascii_lowercase()).filter(|p| !p.is_empty()).collect();
    let Some(key) = parts.pop() else { return String::new() };
    let mut mods: Vec<&str> = parts
        .iter()
        .map(|m| match m.as_str() {
            "commandorcontrol" | "control" | "ctrl" => "ctrl",
            "command" | "super" | "meta" | "win" | "windows" => "super",
            "option" => "alt",
            m => m,
        })
        .collect();
    mods.sort_unstable();
    mods.dedup();
    let key = if key == "escape" { "esc".to_string() } else { key };
    mods.into_iter().chain([key.as_str()]).collect::<Vec<_>>().join("+")
}

/// The other shortcut among `on` that `pressed` already is, if any.
fn clash(own: Shortcut, pressed: &str, on: &[(Shortcut, String)]) -> Option<Shortcut> {
    let pressed = normalized(pressed);
    on.iter().find(|(s, a)| *s != own && normalized(a) == pressed).map(|(s, _)| *s)
}

/// Whether `pressed` is Ctrl K, which opens the command palette in the main
/// window: a global shortcut takes the keys before any window sees them, so
/// the palette would never open again.
fn opens_the_palette(pressed: &str) -> bool {
    normalized(pressed) == "ctrl+k"
}

/// Why `pressed` cannot be `own`'s shortcut: another one has it, or it is
/// Heronite's own Ctrl K.
pub(super) fn taken_by(d: &SettingsPage, own: Shortcut, pressed: &str, cx: &App) -> Option<String> {
    let key = pressed.replace("CommandOrControl", "Ctrl");
    if opens_the_palette(pressed) {
        return Some(tr!("{key} already opens the command palette in Heronite.", key));
    }
    clash(own, pressed, &in_use(d, cx)).map(|other| match other {
        Shortcut::Overlay => tr!("{key} already opens the overlay.", key),
        Shortcut::Screenshot => tr!("{key} already takes a screenshot.", key),
        Shortcut::Replay => tr!("{key} already saves the replay.", key),
        Shortcut::QuickLaunch => tr!("{key} already opens the quick launcher.", key),
    })
}

impl SettingsPage {
    /// Esc or Tab in a shortcut field: the keyboard moves on (to the next
    /// field when there is a tab order, else back to the section list) and
    /// the field keeps its shortcut.
    pub(super) fn leave_hotkey_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let before = window.focused(cx);
        window.focus_next(cx);
        if window.focused(cx) == before {
            window.focus(&self.nav_focus, cx);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{clash, normalized, opens_the_palette, Shortcut};

    fn on() -> Vec<(Shortcut, String)> {
        vec![
            (Shortcut::Overlay, "CommandOrControl+Shift+G".into()),
            (Shortcut::Screenshot, "F10".into()),
            (Shortcut::Replay, "Ctrl+Alt+R".into()),
            (Shortcut::QuickLaunch, "Ctrl+Alt+Space".into()),
        ]
    }

    #[test]
    fn a_shortcut_that_is_on_cannot_be_taken_twice() {
        use Shortcut::*;
        let only_overlay = vec![(Overlay, "CommandOrControl+Shift+G".to_string())];
        for (own, pressed, taken, want) in [
            // The quick launcher refuses the others.
            (QuickLaunch, "CommandOrControl+Shift+G", on(), Some(Overlay)),
            (QuickLaunch, "F10", on(), Some(Screenshot)),
            (QuickLaunch, "CommandOrControl+Alt+R", on(), Some(Replay)),
            // The others refuse it: its own comes as shown ("Ctrl"), the field's as saved.
            (Overlay, "CommandOrControl+Alt+Space", on(), Some(QuickLaunch)),
            (Screenshot, "CommandOrControl+Alt+Space", on(), Some(QuickLaunch)),
            (Replay, "CommandOrControl+Alt+Space", on(), Some(QuickLaunch)),
            // Its own shortcut is no clash.
            (QuickLaunch, "CommandOrControl+Alt+Space", on(), None),
            (Overlay, "CommandOrControl+Shift+G", on(), None),
            // Shortcuts that are off take nothing.
            (Overlay, "F10", only_overlay.clone(), None),
            (QuickLaunch, "F10", only_overlay.clone(), None),
        ] {
            assert_eq!(clash(own, pressed, &taken), want, "{own:?} {pressed}");
        }
    }

    #[test]
    fn ctrl_k_is_the_command_palettes() {
        // As the fields save it, and in other spellings.
        for pressed in ["CommandOrControl+K", "Ctrl+K", "control+k"] {
            assert!(opens_the_palette(pressed), "{pressed}");
        }
        for pressed in ["CommandOrControl+Shift+K", "Alt+K", "Ctrl+Alt+K", "CommandOrControl+Alt+Space", "K"] {
            assert!(!opens_the_palette(pressed), "{pressed}");
        }
    }

    #[test]
    fn spelling_and_order_do_not_matter() {
        assert_eq!(normalized("CommandOrControl+Shift+G"), normalized("shift+ctrl+g"));
        assert_eq!(normalized("Super+G"), normalized("Win+g"));
        assert_eq!(normalized("Ctrl+Esc"), normalized("Control+Escape"));
        assert_ne!(normalized("Ctrl+G"), normalized("Ctrl+Shift+G"));
        assert_ne!(normalized("Alt+F1"), normalized("Alt+F10"));
    }
}
