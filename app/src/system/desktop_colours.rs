//! The desktop's colours, for "From the desktop": its accent and its
//! windows' background. The accent comes from the XDG settings portal
//! (GNOME 47+, KDE Plasma 6, COSMIC), else KDE's `kdeglobals` or GNOME's
//! `accent-color` setting; the windows' background from KDE's colour scheme,
//! or libadwaita's on GNOME. Other desktops give what the portal knows.
//! Read on a thread of its own (one per process), again each time the portal
//! says they changed.

use std::sync::{mpsc, Mutex};
use std::time::Duration;

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use heronite_data::appearance::{is_light_colour, Rgb};

use crate::theme::DesktopColours;

/// libadwaita's window background, light and dark.
const ADWAITA_LIGHT: Rgb = (250, 250, 251);
const ADWAITA_DARK: Rgb = (34, 34, 38);
/// Breeze's accent, KDE's default.
const BREEZE_ACCENT: Rgb = (61, 174, 233);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Desktop {
    Kde,
    Gnome,
    Other,
}

/// The desktop named by `XDG_CURRENT_DESKTOP` (`ubuntu:GNOME`, `KDE`...).
fn desktop_of(current: &str) -> Desktop {
    let has = |name: &str| current.split(':').any(|d| d.eq_ignore_ascii_case(name));
    if has("KDE") {
        Desktop::Kde
    } else if has("GNOME") {
        Desktop::Gnome
    } else {
        Desktop::Other
    }
}

/// `key` in `[section]` of an INI file such as `kdeglobals`.
fn ini_value<'a>(text: &'a str, section: &str, key: &str) -> Option<&'a str> {
    let mut inside = false;
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            inside = name == section;
        } else if inside {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim() == key {
                    return Some(v.trim());
                }
            }
        }
    }
    None
}

/// KDE's `r,g,b` (or `r,g,b,a`).
fn kde_colour(v: &str) -> Option<Rgb> {
    let mut parts = v.split(',').map(|p| p.trim().parse::<u8>().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

/// The colours of a `kdeglobals`: the accent (the selection's colour when
/// it follows the scheme), and the windows' background on its side.
fn kde(text: &str) -> DesktopColours {
    let colour = |section: &str, key: &str| ini_value(text, section, key).and_then(kde_colour);
    let accent = colour("General", "AccentColor").or_else(|| colour("Colors:Selection", "BackgroundNormal")).unwrap_or(BREEZE_ACCENT);
    let window = colour("Colors:Window", "BackgroundNormal");
    let light = window.filter(|c| is_light_colour(*c));
    DesktopColours { accent: Some(accent), window_dark: window.filter(|_| light.is_none()), window_light: light }
}

/// GNOME's named accent (`gsettings`' `'blue'`) in libadwaita's colours.
fn gnome_named(name: &str) -> Option<Rgb> {
    Some(match name.trim().trim_matches('\'') {
        "blue" => (0x35, 0x84, 0xe4),
        "teal" => (0x21, 0x90, 0xa4),
        "green" => (0x3a, 0x94, 0x4a),
        "yellow" => (0xc8, 0x88, 0x00),
        "orange" => (0xed, 0x5b, 0x00),
        "red" => (0xe6, 0x2d, 0x42),
        "pink" => (0xd5, 0x61, 0x99),
        "purple" => (0x91, 0x41, 0xac),
        "slate" => (0x6f, 0x83, 0x96),
        _ => return None,
    })
}

fn gnome_accent() -> Option<Rgb> {
    let out = std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.interface", "accent-color"]).output().ok()?;
    gnome_named(&String::from_utf8_lossy(&out.stdout))
}

/// The desktop's colours now; `portal` is the portal's accent, which wins.
fn read(portal: Option<Rgb>) -> DesktopColours {
    let desktop = desktop_of(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default());
    let mut c = match desktop {
        Desktop::Kde => kde(&dirs::config_dir().and_then(|d| std::fs::read_to_string(d.join("kdeglobals")).ok()).unwrap_or_default()),
        Desktop::Gnome => DesktopColours { accent: None, window_dark: Some(ADWAITA_DARK), window_light: Some(ADWAITA_LIGHT) },
        Desktop::Other => DesktopColours::default(),
    };
    c.accent = portal.or(c.accent).or_else(|| (desktop == Desktop::Gnome).then(gnome_accent).flatten());
    c
}

/// The portal's accent; its channels out of 0..=1 mean the desktop has none.
fn portal_rgb((r, g, b): (f64, f64, f64)) -> Option<Rgb> {
    let ch = |v: f64| (0.0..=1.0).contains(&v).then(|| (v * 255.).round() as u8);
    Some((ch(r)?, ch(g)?, ch(b)?))
}

#[cfg(target_os = "linux")]
async fn portal_accent(settings: &ashpd::desktop::settings::Settings) -> Option<Rgb> {
    let c = settings.accent_color().await.ok()?;
    portal_rgb((c.red(), c.green(), c.blue()))
}

/// The one watcher thread: the latest reading and who wants the next ones.
struct Watcher {
    latest: Option<DesktopColours>,
    subscribers: Vec<UnboundedSender<DesktopColours>>,
}

static WATCHER: Mutex<Option<Watcher>> = Mutex::new(None);

/// Hands a reading to every subscriber still listening.
fn publish(c: DesktopColours) {
    let mut guard = WATCHER.lock().unwrap();
    let Some(w) = guard.as_mut() else { return };
    w.latest = Some(c);
    w.subscribers.retain(|tx| tx.unbounded_send(c).is_ok());
}

/// Reads the desktop's colours on a thread of its own, then again each time
/// the portal says the appearance changed, sending each new reading. The
/// first one is also returned when it comes within `wait`, so a look that
/// uses them starts in them. One thread for the whole process: the window
/// (and with it each subscriber) comes and goes from the tray.
pub fn watch(wait: Duration) -> (Option<DesktopColours>, UnboundedReceiver<DesktopColours>) {
    let (tx, rx) = unbounded();
    {
        let mut guard = WATCHER.lock().unwrap();
        if let Some(w) = guard.as_mut() {
            if let Some(c) = w.latest {
                let _ = tx.unbounded_send(c);
            }
            w.subscribers.push(tx);
            return (w.latest, rx);
        }
        *guard = Some(Watcher { latest: None, subscribers: vec![tx] });
    }
    let (first_tx, first_rx) = mpsc::sync_channel(1);
    let started = std::thread::Builder::new()
        .name("desktop-colours".into())
        .spawn(move || {
            futures::executor::block_on(async move {
                #[cfg(target_os = "linux")]
                {
                    use ashpd::desktop::settings::Settings;
                    use futures::StreamExt;
                    let settings = Settings::new().await.ok();
                    let mut last = read(match &settings {
                        Some(s) => portal_accent(s).await,
                        None => None,
                    });
                    let _ = first_tx.send(last);
                    publish(last);
                    let Some(settings) = settings else { return };
                    let Ok(mut changes) = settings.receive_setting_changed().await else { return };
                    while let Some(s) = changes.next().await {
                        let ns = s.namespace();
                        if ns != "org.freedesktop.appearance"
                            && !ns.starts_with("org.kde.kdeglobals")
                            && ns != "org.gnome.desktop.interface"
                        {
                            continue;
                        }
                        let now = read(portal_accent(&settings).await);
                        if now != last {
                            last = now;
                            publish(now);
                        }
                    }
                }
                #[cfg(not(target_os = "linux"))]
                {
                    let now = read(None);
                    let _ = first_tx.send(now);
                    publish(now);
                }
            })
        })
        .is_ok();
    let first = (started && !wait.is_zero()).then(|| first_rx.recv_timeout(wait).ok()).flatten();
    (first, rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktops_are_told_apart() {
        assert_eq!(desktop_of("KDE"), Desktop::Kde);
        assert_eq!(desktop_of("ubuntu:GNOME"), Desktop::Gnome);
        assert_eq!(desktop_of("Hyprland"), Desktop::Other);
        assert_eq!(desktop_of(""), Desktop::Other);
    }

    #[test]
    fn kdeglobals_give_the_accent_and_the_window() {
        let dark = "[General]\nAccentColor=233,100,40\n\n[Colors:Window]\nBackgroundNormal=32,35,38\n";
        assert_eq!(kde(dark), DesktopColours { accent: Some((233, 100, 40)), window_dark: Some((32, 35, 38)), window_light: None });
        // The accent follows the scheme: the selection's colour.
        let light = "[Colors:Selection]\nBackgroundNormal=61,174,233,255\n[Colors:Window]\nBackgroundNormal=239,240,241\n";
        assert_eq!(kde(light), DesktopColours { accent: Some((61, 174, 233)), window_dark: None, window_light: Some((239, 240, 241)) });
        // A key of another section does not count.
        assert_eq!(kde("[Colors:View]\nBackgroundNormal=1,2,3\n").window_dark, None);
        assert_eq!(kde("").accent, Some(BREEZE_ACCENT));
    }

    #[test]
    fn gnome_and_portal_accents() {
        assert_eq!(gnome_named("'teal'\n"), Some((0x21, 0x90, 0xa4)));
        assert_eq!(gnome_named("'nope'"), None);
        assert_eq!(portal_rgb((1., 0.5, 0.)), Some((255, 128, 0)));
        assert_eq!(portal_rgb((-1., -1., -1.)), None);
    }
}
