//! The app's global keyboard shortcuts seen together, so every shortcut
//! field (Overlay, Screenshots, Replay, Quick launcher and the Controller
//! table) refuses a key another one already has, whichever is set first.

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

/// Why `pressed` cannot be `own`'s shortcut: another one has it.
pub(super) fn taken_by(d: &SettingsPage, own: Shortcut, pressed: &str, cx: &App) -> Option<String> {
    let key = pressed.replace("CommandOrControl", "Ctrl");
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
    use super::{clash, normalized, Shortcut};

    fn on() -> Vec<(Shortcut, String)> {
        vec![
            (Shortcut::Overlay, "CommandOrControl+Shift+G".into()),
            (Shortcut::Screenshot, "F10".into()),
            (Shortcut::Replay, "Ctrl+Alt+R".into()),
            (Shortcut::QuickLaunch, "Ctrl+Alt+Space".into()),
        ]
    }

    #[test]
    fn quick_launch_refuses_the_others() {
        assert_eq!(clash(Shortcut::QuickLaunch, "CommandOrControl+Shift+G", &on()), Some(Shortcut::Overlay));
        assert_eq!(clash(Shortcut::QuickLaunch, "F10", &on()), Some(Shortcut::Screenshot));
        assert_eq!(clash(Shortcut::QuickLaunch, "CommandOrControl+Alt+R", &on()), Some(Shortcut::Replay));
    }

    #[test]
    fn the_others_refuse_quick_launch() {
        // The quick launcher's comes as shown ("Ctrl"), the field's as saved.
        for own in [Shortcut::Overlay, Shortcut::Screenshot, Shortcut::Replay] {
            assert_eq!(clash(own, "CommandOrControl+Alt+Space", &on()), Some(Shortcut::QuickLaunch), "{own:?}");
        }
    }

    #[test]
    fn its_own_shortcut_is_no_clash() {
        assert_eq!(clash(Shortcut::QuickLaunch, "CommandOrControl+Alt+Space", &on()), None);
        assert_eq!(clash(Shortcut::Overlay, "CommandOrControl+Shift+G", &on()), None);
    }

    #[test]
    fn shortcuts_that_are_off_take_nothing() {
        let only_overlay = vec![(Shortcut::Overlay, "CommandOrControl+Shift+G".to_string())];
        assert_eq!(clash(Shortcut::Overlay, "F10", &only_overlay), None);
        assert_eq!(clash(Shortcut::QuickLaunch, "F10", &only_overlay), None);
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
