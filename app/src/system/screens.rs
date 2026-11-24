//! The screen the main window opens on (Settings → General → Window), kept
//! in `defaultSettings` under [`KEY_OPEN_ON_SCREEN`]. Empty, or a screen
//! that is not connected, keeps the window where it was left
//! (`system::hub::window`).

use std::rc::Rc;

use gpui_kit::*;
use heronite_data::game_config::{load_global, Settings};
use heronite_data::DataPaths;
use heronite_i18n::tr;
use serde_json::Value;

/// Heronite's: the key ([`key`]) of the screen picked for the main window.
pub const KEY_OPEN_ON_SCREEN: &str = "heroniteOpenOnScreen";

/// A screen the window can be put on, for the Settings menus.
struct Choice {
    /// What is saved under [`KEY_OPEN_ON_SCREEN`].
    key: String,
    /// "Screen 1 (main)", "Screen 2".
    label: String,
}

/// Whether the app can choose the window's screen. Windows places a new
/// window where it is told; a Wayland compositor decides for itself, and X11
/// has not been tried.
fn placeable() -> bool {
    cfg!(windows)
}

/// A screen's key: its uuid, which Windows derives from the monitor's device
/// name (`\\.\DISPLAY2`), so it lasts across restarts where a `DisplayId`
/// (the monitor's handle) does not.
pub fn key(display: &dyn PlatformDisplay) -> Option<String> {
    display.uuid().ok().map(|u| u.to_string())
}

/// The connected screens, left to right then top to bottom (the order the
/// labels number them in).
fn ordered(cx: &App) -> Vec<Rc<dyn PlatformDisplay>> {
    let mut displays = cx.displays();
    displays.sort_by(|a, b| {
        let (a, b) = (a.bounds().origin, b.bounds().origin);
        f32::from(a.x).total_cmp(&f32::from(b.x)).then(f32::from(a.y).total_cmp(&f32::from(b.y)))
    });
    displays
}

/// The Settings menu of [`KEY_OPEN_ON_SCREEN`]: "Where it was left", the
/// screens, and the saved screen while it is not connected (picking another
/// is the only way to change it, so it stays visible).
pub struct Menu {
    pub names: Vec<SharedString>,
    pub selected: usize,
    keys: Vec<String>,
}

impl Menu {
    /// The menu for the saved key `saved`; `None` when there is no choice.
    pub fn new(cx: &App, saved: &str) -> Option<Self> {
        let choices = choices(cx)?;
        let mut names: Vec<SharedString> = vec![tr!("Where it was left").into()];
        let mut keys = vec![String::new()];
        for c in choices {
            names.push(c.label.into());
            keys.push(c.key);
        }
        if !saved.is_empty() && !keys.iter().any(|k| k == saved) {
            names.push(tr!("A screen that is not connected").into());
            keys.push(saved.to_owned());
        }
        let selected = keys.iter().position(|k| k == saved).unwrap_or(0);
        Some(Self { names, selected, keys })
    }

    /// The key to save for entry `i`.
    pub fn key(&self, i: usize) -> String {
        self.keys.get(i).cloned().unwrap_or_default()
    }
}

/// The screens to offer, or `None` when there is no choice to make (one
/// screen, or a system that does not let the app place its window).
fn choices(cx: &App) -> Option<Vec<Choice>> {
    if !placeable() {
        return None;
    }
    let displays = ordered(cx);
    if displays.len() < 2 {
        return None;
    }
    let primary = cx.primary_display().map(|d| d.id());
    let list: Vec<Choice> = displays
        .iter()
        .enumerate()
        .filter_map(|(i, d)| {
            // GPUI's sizes are logical (scaled by Windows and the interface
            // zoom), not the resolution people know: the number and "main"
            // tell the screens apart instead.
            let n = i + 1;
            let label = if Some(d.id()) == primary { tr!("Screen {n} (main)", n = n) } else { tr!("Screen {n}", n = n) };
            Some(Choice { key: key(d.as_ref())?, label: label.to_string() })
        })
        .collect();
    (list.len() >= 2).then_some(list)
}

/// The screen saved in `settings` (`""` when none was picked).
fn saved_key(settings: &Settings) -> &str {
    settings.get(KEY_OPEN_ON_SCREEN).and_then(Value::as_str).unwrap_or_default()
}

/// The screen picked in Settings (`""` when none), read from `config.json`.
pub fn load(data_paths: &DataPaths) -> String {
    let home = dirs::home_dir().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default();
    saved_key(&load_global(data_paths, &home)).to_string()
}

/// The connected screen saved under `key`, if the app can place its window
/// there.
pub fn find(cx: &App, saved: &str) -> Option<Rc<dyn PlatformDisplay>> {
    if !placeable() || saved.is_empty() {
        return None;
    }
    cx.displays().into_iter().find(|d| key(d.as_ref()).as_deref() == Some(saved))
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's own `test` macro would shadow `#[test]`.
    use super::saved_key;
    use serde_json::json;

    #[test]
    fn saved_key_is_empty_unless_a_screen_was_picked() {
        let read = |v: serde_json::Value| saved_key(v.as_object().unwrap()).to_string();
        assert_eq!(read(json!({})), "");
        assert_eq!(read(json!({ "heroniteOpenOnScreen": "" })), "");
        assert_eq!(read(json!({ "heroniteOpenOnScreen": false })), "");
        assert_eq!(read(json!({ "heroniteOpenOnScreen": "6f1c" })), "6f1c");
    }
}
