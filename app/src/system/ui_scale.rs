//! The interface scale: the one picked in Settings → Appearance (or with
//! Ctrl + / Ctrl − / Ctrl 0), else what the system asks apps to use. It sits
//! on top of the screen's own scale (a 4K TV at 300%) and reaches every window
//! through `gpui-zoom`, so the whole interface, text and images grow or shrink
//! and stay sharp.
//!
//! The system's, first found:
//! - `GDK_DPI_SCALE`, the text scale GTK apps take (`0.5` is half);
//! - `QT_SCALE_FACTOR`, the scale Qt apps take;
//! - GNOME's "Large Text" (`text-scaling-factor`, which GNOME-based desktops
//!   share), on Wayland only: on X11 it is already part of `Xft.dpi`, which
//!   GPUI reads as the screen's scale;
//! - Windows' "Text size" (Accessibility), 100 to 225%.
//!
//! Chromium, so Heroic, scales its whole interface by the desktop's text
//! scale too.

use std::sync::OnceLock;

use gpui_kit::*;
use heronite_data::appearance::{interface_scale_step, Appearance, INTERFACE_SCALE_RANGE};
use heronite_data::AppPaths;
use heronite_i18n::tr;

/// Where the system's scale comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Nothing asks for a scale: 100%.
    Default,
    GdkDpiScale,
    QtScaleFactor,
    GnomeLargeText,
    // Read on Windows only; the label is shared.
    #[cfg_attr(not(windows), allow(dead_code))]
    WindowsTextSize,
}

/// The scale the system asks for, % (on a step of the setting's range).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SystemScale {
    pub percent: f32,
    pub source: Source,
}

impl SystemScale {
    const DEFAULT: SystemScale = SystemScale { percent: 100., source: Source::Default };

    /// What the Appearance page says it follows.
    pub fn label(self) -> String {
        let percent = self.percent.round();
        match self.source {
            Source::Default => tr!("Nothing on this system asks for another size: {percent}%.", percent = percent),
            Source::GdkDpiScale => tr!("{percent}%, from GDK_DPI_SCALE.", percent = percent),
            Source::QtScaleFactor => tr!("{percent}%, from QT_SCALE_FACTOR.", percent = percent),
            Source::GnomeLargeText => tr!("{percent}%, from the desktop's text size.", percent = percent),
            Source::WindowsTextSize => tr!("{percent}%, from Windows' text size.", percent = percent),
        }
    }
}

/// A factor (`0.5`) as a scale on the setting's steps; `None` when it is not
/// a positive number.
fn from_factor(factor: f32, source: Source) -> Option<SystemScale> {
    (factor.is_finite() && factor > 0.).then(|| SystemScale { percent: interface_scale_step(factor * 100.), source })
}

/// A factor in an environment variable (`GDK_DPI_SCALE=0.5`).
fn env_factor(name: &str, source: Source) -> Option<SystemScale> {
    let v = std::env::var(name).ok()?;
    from_factor(v.trim().parse().ok()?, source)
}

/// `gsettings`' `1.25` (GNOME writes doubles as plain numbers).
fn parse_gsettings_double(out: &str) -> Option<f32> {
    out.trim().trim_start_matches("double ").parse().ok()
}

#[cfg(target_os = "linux")]
fn desktop_text_scale() -> Option<SystemScale> {
    // GPUI draws on Wayland whenever a compositor is named.
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()) || std::env::var_os("WAYLAND_SOCKET").is_some();
    if !wayland {
        return None;
    }
    let out = std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.interface", "text-scaling-factor"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    from_factor(parse_gsettings_double(&String::from_utf8_lossy(&out.stdout))?, Source::GnomeLargeText)
}

#[cfg(windows)]
fn desktop_text_scale() -> Option<SystemScale> {
    use heronite_platform::registry::{dword, Hive};
    let percent = dword(Hive::CurrentUser, r"Software\Microsoft\Accessibility", "TextScaleFactor")?;
    from_factor(percent as f32 / 100., Source::WindowsTextSize)
}

#[cfg(not(any(target_os = "linux", windows)))]
fn desktop_text_scale() -> Option<SystemScale> {
    None
}

/// The system's scale, read once per process (the variables cannot change
/// under a running app; a new text size shows at the next start).
pub fn system() -> SystemScale {
    static SYSTEM: OnceLock<SystemScale> = OnceLock::new();
    *SYSTEM.get_or_init(|| {
        let found = env_factor("GDK_DPI_SCALE", Source::GdkDpiScale)
            .or_else(|| env_factor("QT_SCALE_FACTOR", Source::QtScaleFactor))
            .or_else(desktop_text_scale)
            // 100% is no request: the next source may still have one.
            .filter(|s| s.percent != 100.);
        let scale = found.unwrap_or(SystemScale::DEFAULT);
        log::info!("system interface scale: {}% ({:?})", scale.percent, scale.source);
        scale
    })
}

/// The scale `a` asks for, %: its own, else the system's.
pub fn percent_of(a: &Appearance) -> f32 {
    a.interface_scale.unwrap_or_else(|| system().percent)
}

/// Sets the scale of the saved appearance before the first window opens.
pub fn init() {
    gpui_zoom::set_zoom(percent_of(&crate::theme::appearance()) / 100.);
}

/// Applies the current appearance's scale to every window, once the update
/// that changed it is over (a window cannot be updated from inside its own
/// update).
pub fn sync(cx: &mut App) {
    if !gpui_zoom::set_zoom(percent_of(&crate::theme::appearance()) / 100.) {
        return;
    }
    cx.defer(|cx| {
        for window in cx.windows() {
            window.update(cx, |_, window, cx| window.bounds_changed(cx)).ok();
        }
    });
}

/// Picks `percent` (`None`: follow the system), applies it everywhere and
/// saves it off the UI thread.
pub fn set(percent: Option<f32>, app: &AppPaths, cx: &mut App) {
    let mut look = crate::theme::appearance();
    look.interface_scale = percent;
    crate::theme::set_appearance(look);
    sync(cx);
    let (look, app) = (crate::theme::appearance(), app.clone());
    cx.background_spawn(async move {
        if let Err(err) = look.save(&app) {
            log::warn!("saving appearance.json: {err:#}");
        }
    })
    .detach();
}

/// The next scale up (`up`) or down from `percent` on a 10% grid: 75 → 80 or 70.
pub fn stepped(percent: f32, up: bool) -> f32 {
    let tens = percent / 10.;
    let next = if up { (tens + 1e-3).floor() + 1. } else { (tens - 1e-3).ceil() - 1. };
    (next * 10.).clamp(INTERFACE_SCALE_RANGE.0, INTERFACE_SCALE_RANGE.1)
}

/// Ctrl + / Ctrl = (zoom in), Ctrl − (zoom out), Ctrl 0 (back to the
/// system's): the scale it asks for, `Some(None)` for the system's; `None`
/// when `ks` is not one of them.
pub fn shortcut(ks: &Keystroke) -> Option<Option<f32>> {
    let m = &ks.modifiers;
    if !(m.control || m.platform) || m.alt {
        return None;
    }
    let current = percent_of(&crate::theme::appearance());
    // `+` is Shift + `=` on most layouts, and a key of its own on the keypad.
    match (ks.key.as_str(), ks.key_char.as_deref()) {
        ("=" | "+", _) | (_, Some("+")) => Some(Some(stepped(current, true))),
        ("-", _) if !m.shift => Some(Some(stepped(current, false))),
        ("0", _) if !m.shift => Some(None),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's own `test` macro would shadow `#[test]`.
    use super::{from_factor, parse_gsettings_double, stepped, Source};

    #[test]
    fn factors_become_steps_of_the_setting() {
        assert_eq!(from_factor(0.5, Source::GdkDpiScale).map(|s| s.percent), Some(50.));
        assert_eq!(from_factor(1.25, Source::GnomeLargeText).map(|s| s.percent), Some(125.));
        assert_eq!(from_factor(0.33, Source::QtScaleFactor).map(|s| s.percent), Some(50.));
        assert_eq!(from_factor(4., Source::QtScaleFactor).map(|s| s.percent), Some(200.));
        assert_eq!(from_factor(0., Source::GdkDpiScale), None);
        assert_eq!(from_factor(f32::NAN, Source::GdkDpiScale), None);
        assert_eq!(parse_gsettings_double("1.25\n"), Some(1.25));
        assert_eq!(parse_gsettings_double("double 1.5"), Some(1.5));
        assert_eq!(parse_gsettings_double("nope"), None);
    }

    #[test]
    fn steps_go_by_tens_within_the_range() {
        assert_eq!(stepped(100., true), 110.);
        assert_eq!(stepped(100., false), 90.);
        assert_eq!(stepped(75., true), 80.);
        assert_eq!(stepped(75., false), 70.);
        assert_eq!(stepped(200., true), 200.);
        assert_eq!(stepped(50., false), 50.);
    }
}
