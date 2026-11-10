//! Which Linux backend the session calls for, from its environment.

use crate::model::{Backend, Unsupported};

/// The backend for a session whose environment `var` reads, or why there is
/// none. Pure, so every desktop is tested anywhere.
pub fn detect(var: impl Fn(&str) -> Option<String>) -> Result<Backend, Unsupported> {
    let set = |name: &str| var(name).is_some_and(|v| !v.is_empty());
    let desktop = var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let names: Vec<String> = desktop.split(':').map(|d| d.trim().to_ascii_lowercase()).filter(|d| !d.is_empty()).collect();
    let is = |want: &[&str]| names.iter().any(|n| want.contains(&n.as_str()));

    // gamescope owns the outputs (Steam Deck's game mode, a gamescope session).
    if set("GAMESCOPE_WAYLAND_DISPLAY") || is(&["gamescope"]) {
        return Err(Unsupported::GamescopeSession);
    }
    if set("HYPRLAND_INSTANCE_SIGNATURE") || is(&["hyprland"]) {
        return Ok(Backend::Hyprland);
    }
    if is(&["kde"]) || var("KDE_FULL_SESSION").as_deref() == Some("true") {
        return Ok(Backend::Kde);
    }
    // Ubuntu says `ubuntu:GNOME`, Pop!_OS `pop:GNOME`.
    if is(&["gnome", "gnome-classic", "gnome-flashback"]) {
        return Ok(Backend::Gnome);
    }
    // niri speaks wlr-output-management too.
    if set("SWAYSOCK") || is(&["sway", "river", "labwc", "wayfire", "hikari", "dwl", "qtile", "wlroots", "niri"]) {
        return Ok(Backend::Wlroots);
    }
    Err(Unsupported::UnknownDesktop(desktop))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string())
    }

    #[test]
    fn picks_the_backend_of_each_desktop() {
        assert_eq!(detect(env(&[("XDG_CURRENT_DESKTOP", "KDE")])), Ok(Backend::Kde));
        assert_eq!(detect(env(&[("XDG_CURRENT_DESKTOP", "ubuntu:GNOME")])), Ok(Backend::Gnome));
        assert_eq!(detect(env(&[("XDG_CURRENT_DESKTOP", "Hyprland")])), Ok(Backend::Hyprland));
        assert_eq!(detect(env(&[("HYPRLAND_INSTANCE_SIGNATURE", "abc_123")])), Ok(Backend::Hyprland));
        assert_eq!(detect(env(&[("XDG_CURRENT_DESKTOP", "sway")])), Ok(Backend::Wlroots));
        assert_eq!(detect(env(&[("SWAYSOCK", "/run/user/1000/sway-ipc.sock")])), Ok(Backend::Wlroots));
        assert_eq!(detect(env(&[("KDE_FULL_SESSION", "true")])), Ok(Backend::Kde));
    }

    #[test]
    fn a_gamescope_session_wins_over_the_desktop_below_it() {
        let e = [("XDG_CURRENT_DESKTOP", "KDE"), ("GAMESCOPE_WAYLAND_DISPLAY", "gamescope-0")];
        assert_eq!(detect(env(&e)), Err(Unsupported::GamescopeSession));
        assert_eq!(detect(env(&[("XDG_CURRENT_DESKTOP", "gamescope")])), Err(Unsupported::GamescopeSession));
    }

    #[test]
    fn says_which_desktop_it_does_not_know() {
        assert_eq!(detect(env(&[("XDG_CURRENT_DESKTOP", "XFCE")])), Err(Unsupported::UnknownDesktop("XFCE".into())));
        assert_eq!(detect(env(&[])), Err(Unsupported::UnknownDesktop(String::new())));
        // An empty variable is not set.
        assert_eq!(detect(env(&[("SWAYSOCK", ""), ("XDG_CURRENT_DESKTOP", "LXQt")])), Err(Unsupported::UnknownDesktop("LXQt".into())));
    }
}
